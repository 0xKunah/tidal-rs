//! Device authorization ("TV") login, token refresh and refresh token storage.

use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};

use crate::tidal::config::ClientCreds;
use crate::tidal::{Endpoints, TidalError, TidalResult};

const SCOPE: &str = "r_usr w_usr w_sub";
const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
const KEYRING_SERVICE: &str = "tidal-rs";
const KEYRING_USER: &str = "refresh-token";
const PKCE_LOGIN: &str = "https://login.tidal.com/authorize";
/// TIDAL sends the browser here after login (an "Oops" page); the user pastes its address.
const PKCE_REDIRECT: &str = "https://tidal.com/android/login/auth";
const PKCE_SCOPE: &str = "r_usr+w_usr+w_sub";

/// Secrets of one browser (PKCE) login attempt.
#[derive(Clone, Debug, PartialEq)]
pub struct PkceLogin {
	pub verifier: String,
	pub unique_key: String,
}

impl PkceLogin {
	pub fn new() -> Self {
		let mut bytes = [0u8; 40];
		getrandom::fill(&mut bytes).expect("system random source");
		Self {
			verifier: URL_SAFE_NO_PAD.encode(&bytes[..32]),
			unique_key: bytes[32..]
				.iter()
				.map(|byte| format!("{byte:02x}"))
				.collect(),
		}
	}
}

impl Default for PkceLogin {
	fn default() -> Self {
		Self::new()
	}
}

/// S256 code challenge: base64url (no padding) of SHA-256(verifier).
pub fn pkce_challenge(verifier: &str) -> String {
	URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Reads `code` from the address of the page TIDAL redirected to.
pub fn code_from_redirect(address: &str) -> TidalResult<String> {
	let invalid = || TidalError::Config(rust_i18n::t!("login.bad_redirect").to_string());
	let url = url::Url::parse(address.trim()).map_err(|_| invalid())?;
	url.query_pairs()
		.find(|(key, _)| key == "code")
		.map(|(_, value)| value.into_owned())
		.filter(|code| !code.is_empty())
		.ok_or_else(invalid)
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeviceCode {
	pub device_code: String,
	pub user_code: String,
	/// Page where the user confirms; already contains the code when TIDAL provides it.
	pub verification_uri: String,
	pub interval: Duration,
	pub expires_in: Duration,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tokens {
	pub access: String,
	pub refresh: String,
	pub expires_at: SystemTime,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceAuthResponse {
	device_code: String,
	user_code: String,
	verification_uri: String,
	verification_uri_complete: Option<String>,
	expires_in: u64,
	interval: u64,
}

#[derive(Deserialize)]
struct TokenResponse {
	access_token: String,
	refresh_token: Option<String>,
	expires_in: u64,
}

#[derive(Deserialize)]
struct ErrorResponse {
	error: Option<String>,
}

pub struct Auth {
	http: reqwest::Client,
	endpoints: Endpoints,
	creds: ClientCreds,
}

impl Auth {
	pub fn new(endpoints: Endpoints, creds: ClientCreds) -> Self {
		Self {
			http: reqwest::Client::new(),
			endpoints,
			creds,
		}
	}

	/// True when `client.toml` has the browser (PKCE) client, which unlocks FLAC.
	pub fn is_pkce(&self) -> bool {
		self.creds.client_id_pkce.is_some()
	}

	/// The TIDAL login page to open in the browser.
	pub fn pkce_login_url(&self, login: &PkceLogin) -> TidalResult<String> {
		let client_id = self
			.creds
			.client_id_pkce
			.as_deref()
			.ok_or_else(|| TidalError::Config("client_id_pkce".into()))?;
		let challenge = pkce_challenge(&login.verifier);
		let url = url::Url::parse_with_params(
			PKCE_LOGIN,
			&[
				("response_type", "code"),
				("redirect_uri", PKCE_REDIRECT),
				("client_id", client_id),
				("lang", "EN"),
				("appMode", "android"),
				("client_unique_key", login.unique_key.as_str()),
				("code_challenge", challenge.as_str()),
				("code_challenge_method", "S256"),
				("restrict_signup", "true"),
			],
		)
		.map_err(|error| TidalError::Config(error.to_string()))?;
		Ok(url.into())
	}

	/// Trades the code from the pasted redirect address for tokens.
	pub async fn exchange_pkce(&self, redirect: &str, login: &PkceLogin) -> TidalResult<Tokens> {
		let code = code_from_redirect(redirect)?;
		let client_id = self
			.creds
			.client_id_pkce
			.as_deref()
			.ok_or_else(|| TidalError::Config("client_id_pkce".into()))?;
		let response = self
			.http
			.post(self.url("oauth2/token")?)
			.form(&[
				("code", code.as_str()),
				("client_id", client_id),
				("grant_type", "authorization_code"),
				("redirect_uri", PKCE_REDIRECT),
				("scope", PKCE_SCOPE),
				("code_verifier", login.verifier.as_str()),
				("client_unique_key", login.unique_key.as_str()),
			])
			.send()
			.await?;
		if !response.status().is_success() {
			return Err(api_error(response).await);
		}
		tokens(response.json().await?, None)
	}

	/// Client used for refreshing: the PKCE one when configured.
	fn refresh_client(&self) -> (&str, &str) {
		match (&self.creds.client_id_pkce, &self.creds.client_secret_pkce) {
			(Some(id), Some(secret)) => (id, secret),
			_ => (&self.creds.client_id, &self.creds.client_secret),
		}
	}

	fn url(&self, path: &str) -> TidalResult<url::Url> {
		self.endpoints
			.auth
			.join(path)
			.map_err(|error| TidalError::Config(error.to_string()))
	}

	pub async fn start_device(&self) -> TidalResult<DeviceCode> {
		let response = self
			.http
			.post(self.url("oauth2/device_authorization")?)
			.form(&[
				("client_id", self.creds.client_id.as_str()),
				("scope", SCOPE),
			])
			.send()
			.await?;
		if !response.status().is_success() {
			return Err(api_error(response).await);
		}
		let body: DeviceAuthResponse = response.json().await?;
		Ok(DeviceCode {
			device_code: body.device_code,
			user_code: body.user_code,
			verification_uri: body
				.verification_uri_complete
				.unwrap_or(body.verification_uri),
			interval: Duration::from_secs(body.interval),
			expires_in: Duration::from_secs(body.expires_in),
		})
	}

	/// Polls until the user confirmed the code on link.tidal.com.
	pub async fn poll_device(&self, code: &DeviceCode) -> TidalResult<Tokens> {
		let deadline = Instant::now() + code.expires_in;
		loop {
			let response = self
				.http
				.post(self.url("oauth2/token")?)
				.form(&[
					("client_id", self.creds.client_id.as_str()),
					("client_secret", self.creds.client_secret.as_str()),
					("device_code", code.device_code.as_str()),
					("grant_type", DEVICE_GRANT),
					("scope", SCOPE),
				])
				.send()
				.await?;
			let status = response.status();
			if status.is_success() {
				return tokens(response.json().await?, None);
			}
			let text = response.text().await.unwrap_or_default();
			let error = serde_json::from_str::<ErrorResponse>(&text)
				.ok()
				.and_then(|body| body.error);
			match error.as_deref() {
				Some("authorization_pending") | Some("slow_down") => {}
				Some("expired_token") => return Err(TidalError::LoginExpired),
				_ => {
					return Err(TidalError::Api {
						status: status.as_u16(),
						body: text,
					});
				}
			}
			if Instant::now() >= deadline {
				return Err(TidalError::LoginExpired);
			}
			tokio::time::sleep(code.interval).await;
		}
	}

	/// New access token; keeps `refresh` when TIDAL does not rotate it.
	pub async fn refresh(&self, refresh: &str) -> TidalResult<Tokens> {
		let (client_id, client_secret) = self.refresh_client();
		let response = self
			.http
			.post(self.url("oauth2/token")?)
			.form(&[
				("client_id", client_id),
				("client_secret", client_secret),
				("refresh_token", refresh),
				("grant_type", "refresh_token"),
				("scope", SCOPE),
			])
			.send()
			.await?;
		match response.status().as_u16() {
			200..=299 => tokens(response.json().await?, Some(refresh)),
			400 | 401 => Err(TidalError::Unauthorized),
			_ => Err(api_error(response).await),
		}
	}
}

fn tokens(body: TokenResponse, previous_refresh: Option<&str>) -> TidalResult<Tokens> {
	let refresh = body
		.refresh_token
		.or_else(|| previous_refresh.map(str::to_string))
		.ok_or_else(|| TidalError::Decode("token response without refresh_token".into()))?;
	Ok(Tokens {
		access: body.access_token,
		refresh,
		expires_at: SystemTime::now() + Duration::from_secs(body.expires_in),
	})
}

pub(crate) async fn api_error(response: reqwest::Response) -> TidalError {
	let status = response.status().as_u16();
	let body = response.text().await.unwrap_or_default();
	TidalError::Api { status, body }
}

/// Where the refresh token lives between runs.
pub trait TokenStore: Send + Sync {
	fn load(&self) -> Option<String>;
	fn save(&self, refresh: &str);
	fn clear(&self);
}

/// Windows Credential Manager.
pub struct KeyringStore;

impl KeyringStore {
	fn entry() -> Option<keyring::Entry> {
		keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
			.inspect_err(|error| eprintln!("credential store unavailable: {error}"))
			.ok()
	}
}

impl TokenStore for KeyringStore {
	fn load(&self) -> Option<String> {
		Self::entry()?.get_password().ok()
	}

	fn save(&self, refresh: &str) {
		if let Some(entry) = Self::entry()
			&& let Err(error) = entry.set_password(refresh)
		{
			eprintln!("could not save the refresh token: {error}");
		}
	}

	fn clear(&self) {
		if let Some(entry) = Self::entry() {
			let _ = entry.delete_credential();
		}
	}
}

#[derive(Default)]
pub struct MemoryStore(Mutex<Option<String>>);

impl TokenStore for MemoryStore {
	fn load(&self) -> Option<String> {
		self.0.lock().ok()?.clone()
	}

	fn save(&self, refresh: &str) {
		if let Ok(mut slot) = self.0.lock() {
			*slot = Some(refresh.to_string());
		}
	}

	fn clear(&self) {
		if let Ok(mut slot) = self.0.lock() {
			*slot = None;
		}
	}
}

#[cfg(test)]
mod tests {
	use std::time::Duration;

	use wiremock::matchers::{body_string_contains, method, path};
	use wiremock::{Mock, MockServer, ResponseTemplate};

	use super::*;
	use crate::tidal::Endpoints;
	use crate::tidal::config::ClientCreds;

	fn auth(server: &MockServer) -> Auth {
		let base = url::Url::parse(&format!("{}/v1/", server.uri())).unwrap();
		Auth::new(
			Endpoints {
				auth: base.clone(),
				api: base,
			},
			ClientCreds {
				client_id: "cid".into(),
				client_secret: "secret".into(),
				client_id_pkce: None,
				client_secret_pkce: None,
			},
		)
	}

	fn pkce_auth(server: &MockServer) -> Auth {
		let base = url::Url::parse(&format!("{}/v1/", server.uri())).unwrap();
		Auth::new(
			Endpoints {
				auth: base.clone(),
				api: base,
			},
			ClientCreds {
				client_id: "cid".into(),
				client_secret: "secret".into(),
				client_id_pkce: Some("pkce-id".into()),
				client_secret_pkce: Some("pkce-secret".into()),
			},
		)
	}

	fn pkce() -> PkceLogin {
		PkceLogin {
			verifier: "tidal-rs-test-verifier-0123456789abcdefghijk".into(),
			unique_key: "1a2b3c4d5e6f7a8b".into(),
		}
	}

	fn code() -> DeviceCode {
		DeviceCode {
			device_code: "d".into(),
			user_code: "ABCD".into(),
			verification_uri: "link.tidal.com/ABCD".into(),
			interval: Duration::ZERO,
			expires_in: Duration::from_secs(300),
		}
	}

	#[tokio::test]
	async fn device_flow_polls_until_authorized() {
		let server = MockServer::start().await;
		Mock::given(method("POST"))
			.and(path("/v1/oauth2/device_authorization"))
			.respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
				"deviceCode": "d", "userCode": "ABCD",
				"verificationUri": "link.tidal.com", "verificationUriComplete": "link.tidal.com/ABCD",
				"expiresIn": 300, "interval": 0
			})))
			.mount(&server)
			.await;
		Mock::given(method("POST"))
			.and(path("/v1/oauth2/token"))
			.and(body_string_contains("device_code"))
			.respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
				"status": 400, "error": "authorization_pending"
			})))
			.up_to_n_times(1)
			.mount(&server)
			.await;
		Mock::given(method("POST"))
			.and(path("/v1/oauth2/token"))
			.respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
				"access_token": "a", "refresh_token": "r", "expires_in": 3600
			})))
			.mount(&server)
			.await;
		let auth = auth(&server);
		let code = auth.start_device().await.unwrap();
		assert_eq!(code.user_code, "ABCD");
		let tokens = auth.poll_device(&code).await.unwrap();
		assert_eq!(tokens.access, "a");
		assert_eq!(tokens.refresh, "r");
	}

	#[tokio::test]
	async fn device_flow_fails_when_expired() {
		let server = MockServer::start().await;
		Mock::given(method("POST"))
			.and(path("/v1/oauth2/token"))
			.respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
				"status": 400, "error": "expired_token"
			})))
			.mount(&server)
			.await;
		assert_eq!(
			auth(&server).poll_device(&code()).await,
			Err(TidalError::LoginExpired)
		);
	}

	#[tokio::test]
	async fn refresh_keeps_old_refresh_token_when_absent() {
		let server = MockServer::start().await;
		Mock::given(method("POST"))
			.and(path("/v1/oauth2/token"))
			.and(body_string_contains("grant_type=refresh_token"))
			.respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
				"access_token": "a2", "expires_in": 3600
			})))
			.mount(&server)
			.await;
		let tokens = auth(&server).refresh("r-old").await.unwrap();
		assert_eq!(tokens.access, "a2");
		assert_eq!(tokens.refresh, "r-old");
	}

	#[test]
	fn pkce_challenge_is_base64url_sha256() {
		// Expected value computed with Python's hashlib + base64.urlsafe_b64encode.
		assert_eq!(
			pkce_challenge("tidal-rs-test-verifier-0123456789abcdefghijk"),
			"f4W-ukombir2GK-LHggrxyY2af49mLbVx3MMC2Dk4Jo"
		);
	}

	#[test]
	fn new_pkce_login_has_a_long_verifier() {
		let login = PkceLogin::new();
		assert!(login.verifier.len() >= 43);
		assert_eq!(login.unique_key.len(), 16);
	}

	#[tokio::test]
	async fn login_url_carries_the_pkce_parameters() {
		let server = MockServer::start().await;
		let url = url::Url::parse(&pkce_auth(&server).pkce_login_url(&pkce()).unwrap()).unwrap();
		let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
		assert_eq!(url.host_str(), Some("login.tidal.com"));
		assert_eq!(query["client_id"], "pkce-id");
		assert_eq!(query["response_type"], "code");
		assert_eq!(
			query["redirect_uri"],
			"https://tidal.com/android/login/auth"
		);
		assert_eq!(query["code_challenge_method"], "S256");
		assert_eq!(
			query["code_challenge"],
			"f4W-ukombir2GK-LHggrxyY2af49mLbVx3MMC2Dk4Jo"
		);
		assert_eq!(query["client_unique_key"], "1a2b3c4d5e6f7a8b");
	}

	#[test]
	fn code_is_read_from_the_redirect_address() {
		assert_eq!(
			code_from_redirect(" https://tidal.com/android/login/auth?code=abc123&state=x "),
			Ok("abc123".to_string())
		);
		assert!(code_from_redirect("https://tidal.com/android/login/auth").is_err());
		assert!(code_from_redirect("not a url").is_err());
	}

	#[tokio::test]
	async fn pkce_exchange_posts_code_and_verifier() {
		let server = MockServer::start().await;
		Mock::given(method("POST"))
			.and(path("/v1/oauth2/token"))
			.and(body_string_contains("grant_type=authorization_code"))
			.and(body_string_contains("code=abc123"))
			.and(body_string_contains("code_verifier=tidal-rs-test-verifier"))
			.and(body_string_contains("client_id=pkce-id"))
			.respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
				"access_token": "a", "refresh_token": "r", "expires_in": 3600
			})))
			.expect(1)
			.mount(&server)
			.await;
		let tokens = pkce_auth(&server)
			.exchange_pkce("https://tidal.com/android/login/auth?code=abc123", &pkce())
			.await
			.unwrap();
		assert_eq!(tokens.refresh, "r");
	}

	#[tokio::test]
	async fn refresh_uses_the_pkce_client_when_configured() {
		let server = MockServer::start().await;
		Mock::given(method("POST"))
			.and(path("/v1/oauth2/token"))
			.and(body_string_contains("client_id=pkce-id"))
			.and(body_string_contains("client_secret=pkce-secret"))
			.respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
				"access_token": "a2", "expires_in": 3600
			})))
			.expect(1)
			.mount(&server)
			.await;
		assert!(pkce_auth(&server).is_pkce());
		assert_eq!(pkce_auth(&server).refresh("r").await.unwrap().access, "a2");
	}

	#[test]
	fn memory_store_roundtrip() {
		let store = MemoryStore::default();
		store.save("r");
		assert_eq!(store.load().as_deref(), Some("r"));
		store.clear();
		assert_eq!(store.load(), None);
	}
}
