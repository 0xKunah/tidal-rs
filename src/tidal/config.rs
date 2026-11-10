//! Client credentials, provided by the user in `client.toml` (never in the code).

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::tidal::TidalError;

#[derive(Clone, Debug, Deserialize)]
pub struct ClientCreds {
	/// Device-code ("TV") client: limited to HIGH (AAC).
	pub client_id: String,
	pub client_secret: String,
	/// Browser (PKCE) client: required for Lossless and Hi-Res FLAC.
	#[serde(default)]
	pub client_id_pkce: Option<String>,
	#[serde(default)]
	pub client_secret_pkce: Option<String>,
}

/// `client.toml` in the app data folder.
pub fn client_creds_path() -> PathBuf {
	crate::store::app_dir().join("client.toml")
}

/// Errors carry the file path so the login screen can tell the user where to put it.
pub fn load_client_creds(path: &Path) -> Result<ClientCreds, TidalError> {
	let text = std::fs::read_to_string(path)
		.map_err(|_| TidalError::Config(path.display().to_string()))?;
	toml::from_str(&text)
		.map_err(|error| TidalError::Config(format!("{}: {error}", path.display())))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn missing_client_file_names_the_path() {
		let path = std::env::temp_dir()
			.join("tidal-rs-no-such-dir")
			.join("client.toml");
		match load_client_creds(&path) {
			Err(TidalError::Config(message)) => assert!(message.contains("client.toml")),
			other => panic!("unexpected {other:?}"),
		}
	}

	#[test]
	fn reads_id_and_secret() {
		let dir = std::env::temp_dir().join(format!("tidal-rs-creds-{}", std::process::id()));
		std::fs::create_dir_all(&dir).unwrap();
		let path = dir.join("client.toml");
		std::fs::write(&path, "client_id = \"id\"\nclient_secret = \"secret\"\n").unwrap();
		let creds = load_client_creds(&path).unwrap();
		assert_eq!(creds.client_id, "id");
		assert_eq!(creds.client_secret, "secret");
		assert_eq!(creds.client_id_pkce, None);
		std::fs::write(
			&path,
			"client_id = \"id\"\nclient_secret = \"secret\"\nclient_id_pkce = \"pid\"\nclient_secret_pkce = \"psecret\"\n",
		)
		.unwrap();
		let creds = load_client_creds(&path).unwrap();
		assert_eq!(creds.client_id_pkce.as_deref(), Some("pid"));
		assert_eq!(creds.client_secret_pkce.as_deref(), Some("psecret"));
		let _ = std::fs::remove_dir_all(&dir);
	}
}
