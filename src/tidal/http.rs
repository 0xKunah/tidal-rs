//! `TidalApi` over HTTP (reqwest), with transparent token refresh.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use futures::StreamExt as _;
use reqwest::{Method, RequestBuilder, Response, StatusCode};
use serde::de::DeserializeOwned;

use crate::model::{
	Album, AlbumId, ArtistId, ArtistPage, AudioQuality, Playlist, PlaylistId, SearchResults, Track,
	TrackId,
};
use crate::tidal::auth::{Auth, TokenStore, Tokens, api_error};
use crate::tidal::dto::{
	AlbumDto, ArtistDto, FavoritePlaylistDto, Page, PlaybackInfoDto, PlaylistDto, PlaylistItemDto,
	SearchDto, SessionDto, TrackDto,
};
use crate::tidal::manifest::{StreamInfo, decode_manifest, quality_from_api};
use crate::tidal::{Endpoints, TidalApi, TidalError, TidalResult};

/// TIDAL rejects larger pages ("max page size is [50]").
const PAGE_SIZE: usize = 50;
/// Pages fetched at the same time for long lists.
const PARALLEL_PAGES: usize = 6;
/// Refresh a bit before expiry so requests rarely hit a 401.
const REFRESH_MARGIN: Duration = Duration::from_secs(60);

pub struct HttpTidal {
	http: reqwest::Client,
	endpoints: Endpoints,
	auth: Auth,
	store: Arc<dyn TokenStore>,
	tokens: tokio::sync::Mutex<Tokens>,
	user_id: u64,
	country: String,
}

impl HttpTidal {
	/// Validates the tokens with `GET sessions` and learns the user id and country.
	pub async fn connect(
		endpoints: Endpoints,
		auth: Auth,
		tokens: Tokens,
		store: Arc<dyn TokenStore>,
	) -> TidalResult<Self> {
		let mut api = Self {
			http: reqwest::Client::new(),
			endpoints,
			auth,
			store,
			tokens: tokio::sync::Mutex::new(tokens),
			user_id: 0,
			country: String::new(),
		};
		let session: SessionDto = api.get("sessions", &[]).await?;
		api.user_id = session.user_id;
		api.country = session.country_code;
		Ok(api)
	}

	pub fn user_id(&self) -> u64 {
		self.user_id
	}

	/// "First Last", else the username, else a generic label; never fails.
	pub async fn display_name(&self) -> String {
		#[derive(serde::Deserialize)]
		#[serde(rename_all = "camelCase")]
		struct UserDto {
			#[serde(default)]
			username: Option<String>,
			#[serde(default)]
			first_name: Option<String>,
			#[serde(default)]
			last_name: Option<String>,
		}
		let fallback = format!("TIDAL #{}", self.user_id);
		let Ok(user) = self
			.get::<UserDto>(&format!("users/{}", self.user_id), &[])
			.await
		else {
			return fallback;
		};
		let full = [user.first_name, user.last_name]
			.into_iter()
			.flatten()
			.filter(|part| !part.is_empty())
			.collect::<Vec<_>>()
			.join(" ");
		if !full.is_empty() {
			full
		} else {
			user.username
				.filter(|name| !name.is_empty())
				.unwrap_or(fallback)
		}
	}

	fn url(&self, path: &str) -> TidalResult<url::Url> {
		self.endpoints
			.api
			.join(path)
			.map_err(|error| TidalError::Config(error.to_string()))
	}

	async fn access_token(&self) -> TidalResult<String> {
		let tokens = self.tokens.lock().await;
		if tokens.expires_at > SystemTime::now() + REFRESH_MARGIN {
			return Ok(tokens.access.clone());
		}
		let stale = tokens.access.clone();
		drop(tokens);
		self.refresh_tokens(&stale).await
	}

	/// Refreshes unless another request already did since `stale` was read.
	async fn refresh_tokens(&self, stale: &str) -> TidalResult<String> {
		let mut tokens = self.tokens.lock().await;
		if tokens.access != stale {
			return Ok(tokens.access.clone());
		}
		let fresh = self.auth.refresh(&tokens.refresh).await?;
		self.store.save(&fresh.refresh);
		*tokens = fresh;
		Ok(tokens.access.clone())
	}

	/// Sends a request built by `build` with auth and `countryCode`; on 401 refreshes once and retries.
	pub(crate) async fn send(
		&self,
		method: Method,
		path: &str,
		build: impl Fn(RequestBuilder) -> RequestBuilder,
	) -> TidalResult<Response> {
		let url = self.url(path)?;
		let mut access = self.access_token().await?;
		for attempt in 0..2 {
			let request = self
				.http
				.request(method.clone(), url.clone())
				.bearer_auth(&access)
				.query(&[("countryCode", self.country.as_str())]);
			let response = build(request).send().await?;
			match response.status() {
				StatusCode::UNAUTHORIZED if attempt == 0 => {
					access = self.refresh_tokens(&access).await?;
				}
				StatusCode::UNAUTHORIZED => return Err(TidalError::Unauthorized),
				StatusCode::NOT_FOUND => return Err(TidalError::NotFound),
				status if status.is_success() => return Ok(response),
				_ => return Err(api_error(response).await),
			}
		}
		Err(TidalError::Unauthorized)
	}

	async fn get<T: DeserializeOwned>(
		&self,
		path: &str,
		query: &[(&str, String)],
	) -> TidalResult<T> {
		let response = self
			.send(Method::GET, path, |request| request.query(query))
			.await?;
		let text = response.text().await?;
		serde_json::from_str(&text).map_err(|error| TidalError::Decode(format!("{path}: {error}")))
	}

	async fn page<T: DeserializeOwned>(
		&self,
		path: &str,
		query: &[(&str, String)],
		offset: usize,
	) -> TidalResult<Page<T>> {
		let mut paged = query.to_vec();
		paged.push(("limit", PAGE_SIZE.to_string()));
		paged.push(("offset", offset.to_string()));
		self.get(path, &paged).await
	}

	/// Reads the first page, then fetches the remaining pages in parallel (in order).
	async fn get_all<T: DeserializeOwned>(
		&self,
		path: &str,
		query: &[(&str, String)],
	) -> TidalResult<Vec<T>> {
		let first: Page<T> = self.page(path, query, 0).await?;
		let mut items = first.items;
		let total = first.total_number_of_items.unwrap_or(items.len());
		if items.is_empty() || items.len() >= total {
			return Ok(items);
		}
		let offsets = (items.len()..total).step_by(PAGE_SIZE);
		let pages: Vec<TidalResult<Page<T>>> = futures::stream::iter(offsets)
			.map(|offset| self.page(path, query, offset))
			.buffered(PARALLEL_PAGES)
			.collect()
			.await;
		for page in pages {
			items.extend(page?.items);
		}
		Ok(items)
	}

	async fn tracks(&self, path: &str, query: &[(&str, String)]) -> TidalResult<Vec<Track>> {
		Ok(self
			.get_all::<TrackDto>(path, query)
			.await?
			.into_iter()
			.map(Into::into)
			.collect())
	}

	async fn albums(&self, path: &str, query: &[(&str, String)]) -> TidalResult<Vec<Album>> {
		Ok(self
			.get_all::<AlbumDto>(path, query)
			.await?
			.into_iter()
			.map(Into::into)
			.collect())
	}
}

impl TidalApi for HttpTidal {
	async fn playlists(&self) -> TidalResult<Vec<Playlist>> {
		let path = format!("users/{}/playlistsAndFavoritePlaylists", self.user_id);
		let items: Vec<FavoritePlaylistDto> = self.get_all(&path, &[]).await?;
		Ok(items
			.into_iter()
			.map(|item| item.into_playlist(self.user_id))
			.collect())
	}

	async fn playlist_tracks(&self, id: &PlaylistId) -> TidalResult<Vec<Track>> {
		let items: Vec<PlaylistItemDto> = self
			.get_all(&format!("playlists/{}/items", id.0), &[])
			.await?;
		Ok(items
			.into_iter()
			.filter_map(PlaylistItemDto::into_track)
			.collect())
	}

	async fn album(&self, id: AlbumId) -> TidalResult<(Album, Vec<Track>)> {
		let album: AlbumDto = self.get(&format!("albums/{}", id.0), &[]).await?;
		let tracks = self.tracks(&format!("albums/{}/tracks", id.0), &[]).await?;
		Ok((album.into(), tracks))
	}

	async fn artist(&self, id: ArtistId) -> TidalResult<ArtistPage> {
		let artist: ArtistDto = self.get(&format!("artists/{}", id.0), &[]).await?;
		let top: Page<TrackDto> = self
			.get(
				&format!("artists/{}/toptracks", id.0),
				&[("limit", "10".into())],
			)
			.await?;
		let albums = self
			.albums(&format!("artists/{}/albums", id.0), &[])
			.await?;
		let singles = self
			.albums(
				&format!("artists/{}/albums", id.0),
				&[("filter", "EPSANDSINGLES".into())],
			)
			.await?;
		Ok(ArtistPage {
			artist: artist.into(),
			top_tracks: top.items.into_iter().map(Into::into).collect(),
			albums,
			singles,
		})
	}

	async fn search(&self, query: &str) -> TidalResult<SearchResults> {
		let found: SearchDto = self
			.get(
				"search",
				&[
					("query", query.to_string()),
					("types", "TRACKS,ARTISTS,ALBUMS".into()),
					("limit", "20".into()),
				],
			)
			.await?;
		Ok(SearchResults {
			tracks: found.tracks.items.into_iter().map(Into::into).collect(),
			artists: found.artists.items.into_iter().map(Into::into).collect(),
			albums: found.albums.items.into_iter().map(Into::into).collect(),
		})
	}

	async fn track_radio(&self, id: TrackId) -> TidalResult<Vec<Track>> {
		let page: Page<TrackDto> = self
			.get(&format!("tracks/{}/radio", id.0), &[("limit", "50".into())])
			.await?;
		Ok(page.items.into_iter().map(Into::into).collect())
	}

	async fn stream(&self, id: TrackId, quality: AudioQuality) -> TidalResult<StreamInfo> {
		let info: PlaybackInfoDto = self
			.get(
				&format!("tracks/{}/playbackinfopostpaywall", id.0),
				&[
					("audioquality", quality.api().to_string()),
					("playbackmode", "STREAM".into()),
					("assetpresentation", "FULL".into()),
				],
			)
			.await?;
		let (source, codec) = decode_manifest(&info.manifest_mime_type, &info.manifest)?;
		Ok(StreamInfo {
			source,
			codec,
			quality: quality_from_api(&info.audio_quality),
			bit_depth: info.bit_depth,
			sample_rate: info.sample_rate,
			replay_gain: info.track_replay_gain,
			peak: info.track_peak_amplitude,
		})
	}

	async fn create_playlist(&self, title: &str) -> TidalResult<Playlist> {
		let path = format!("users/{}/playlists", self.user_id);
		let response = self
			.send(Method::POST, &path, |request| {
				request.form(&[("title", title), ("description", "")])
			})
			.await?;
		let text = response.text().await?;
		let dto: PlaylistDto = serde_json::from_str(&text)
			.map_err(|error| TidalError::Decode(format!("{path}: {error}")))?;
		let mut playlist = dto.into_playlist(self.user_id);
		playlist.owned = true;
		Ok(playlist)
	}

	async fn rename_playlist(&self, id: &PlaylistId, title: &str) -> TidalResult<()> {
		let etag = self.etag(id).await?;
		self.send(Method::POST, &format!("playlists/{}", id.0), |request| {
			request
				.header("If-None-Match", &etag)
				.form(&[("title", title), ("description", "")])
		})
		.await?;
		Ok(())
	}

	async fn delete_playlist(&self, id: &PlaylistId) -> TidalResult<()> {
		self.send(Method::DELETE, &format!("playlists/{}", id.0), |request| {
			request
		})
		.await?;
		Ok(())
	}

	async fn add_to_playlist(&self, id: &PlaylistId, track: TrackId) -> TidalResult<()> {
		let etag = self.etag(id).await?;
		let track = track.0.to_string();
		self.send(
			Method::POST,
			&format!("playlists/{}/items", id.0),
			|request| {
				request.header("If-None-Match", &etag).form(&[
					("trackIds", track.as_str()),
					("onDupes", "FAIL"),
					("onArtifactNotFound", "FAIL"),
				])
			},
		)
		.await?;
		Ok(())
	}

	async fn remove_from_playlist(&self, id: &PlaylistId, index: usize) -> TidalResult<()> {
		let etag = self.etag(id).await?;
		self.send(
			Method::DELETE,
			&format!("playlists/{}/items/{index}", id.0),
			|request| {
				request
					.header("If-None-Match", &etag)
					.query(&[("order", "INDEX"), ("orderDirection", "ASC")])
			},
		)
		.await?;
		Ok(())
	}

	async fn move_in_playlist(&self, id: &PlaylistId, from: usize, to: usize) -> TidalResult<()> {
		let etag = self.etag(id).await?;
		let to = to.to_string();
		self.send(
			Method::POST,
			&format!("playlists/{}/items/{from}", id.0),
			|request| {
				request
					.header("If-None-Match", &etag)
					.form(&[("toIndex", to.as_str())])
			},
		)
		.await?;
		Ok(())
	}
}

impl HttpTidal {
	/// Current version tag of a playlist; edits must send it as `If-None-Match`.
	async fn etag(&self, id: &PlaylistId) -> TidalResult<String> {
		let response = self
			.send(Method::GET, &format!("playlists/{}", id.0), |request| {
				request
			})
			.await?;
		response
			.headers()
			.get("etag")
			.and_then(|value| value.to_str().ok())
			.map(str::to_string)
			.ok_or_else(|| TidalError::Decode("playlist response without ETag".into()))
	}
}

#[cfg(test)]
mod tests {
	use std::sync::Arc;
	use std::time::{Duration, SystemTime};

	use serde_json::{Value, json};
	use wiremock::matchers::{header, method, path, query_param};
	use wiremock::{Mock, MockServer, ResponseTemplate};

	use super::*;
	use crate::model::{AlbumId, PlaylistId, TrackId};
	use crate::tidal::auth::{Auth, MemoryStore, TokenStore, Tokens};
	use crate::tidal::config::ClientCreds;
	use crate::tidal::{Endpoints, TidalApi, TidalError};

	const TRACK: &str = include_str!("../../tests/fixtures/tidal/track.json");
	const ALBUM: &str = include_str!("../../tests/fixtures/tidal/album.json");
	const ARTIST: &str = include_str!("../../tests/fixtures/tidal/artist.json");
	const SESSIONS: &str = include_str!("../../tests/fixtures/tidal/sessions.json");
	const PLAYLISTS: &str = include_str!("../../tests/fixtures/tidal/playlists_and_favorites.json");

	fn track_json(id: u64) -> Value {
		let mut value: Value = serde_json::from_str(TRACK).unwrap();
		value["id"] = json!(id);
		value["title"] = json!(format!("T{id}"));
		value
	}

	fn page(items: Vec<Value>, total: usize) -> Value {
		json!({ "limit": 100, "offset": 0, "totalNumberOfItems": total, "items": items })
	}

	fn endpoints(server: &MockServer) -> Endpoints {
		let base = url::Url::parse(&format!("{}/v1/", server.uri())).unwrap();
		Endpoints {
			auth: base.clone(),
			api: base,
		}
	}

	fn tokens(access: &str) -> Tokens {
		Tokens {
			access: access.into(),
			refresh: "r1".into(),
			expires_at: SystemTime::now() + Duration::from_secs(3600),
		}
	}

	async fn connect_with(
		server: &MockServer,
		store: Arc<dyn TokenStore>,
		access: &str,
	) -> HttpTidal {
		Mock::given(method("GET"))
			.and(path("/v1/sessions"))
			.respond_with(ResponseTemplate::new(200).set_body_string(SESSIONS))
			.mount(server)
			.await;
		let creds = ClientCreds {
			client_id: "cid".into(),
			client_secret: "secret".into(),
			client_id_pkce: None,
			client_secret_pkce: None,
		};
		let endpoints = endpoints(server);
		HttpTidal::connect(
			endpoints.clone(),
			Auth::new(endpoints, creds),
			tokens(access),
			store,
		)
		.await
		.unwrap()
	}

	async fn connected(server: &MockServer) -> HttpTidal {
		connect_with(server, Arc::new(MemoryStore::default()), "a").await
	}

	async fn mount_json(server: &MockServer, at: &str, body: Value) {
		Mock::given(method("GET"))
			.and(path(at))
			.respond_with(ResponseTemplate::new(200).set_body_json(body))
			.mount(server)
			.await;
	}

	#[tokio::test]
	async fn get_retries_once_after_401_with_refreshed_token() {
		let server = MockServer::start().await;
		let store: Arc<dyn TokenStore> = Arc::new(MemoryStore::default());
		let api = connect_with(&server, store.clone(), "old").await;
		Mock::given(method("GET"))
			.and(path("/v1/users/42/playlistsAndFavoritePlaylists"))
			.and(header("authorization", "Bearer old"))
			.respond_with(ResponseTemplate::new(401))
			.mount(&server)
			.await;
		Mock::given(method("GET"))
			.and(path("/v1/users/42/playlistsAndFavoritePlaylists"))
			.and(header("authorization", "Bearer new"))
			.respond_with(ResponseTemplate::new(200).set_body_string(PLAYLISTS))
			.mount(&server)
			.await;
		Mock::given(method("POST"))
			.and(path("/v1/oauth2/token"))
			.respond_with(ResponseTemplate::new(200).set_body_json(json!({
				"access_token": "new", "refresh_token": "r2", "expires_in": 3600
			})))
			.mount(&server)
			.await;
		assert_eq!(api.playlists().await.unwrap().len(), 2);
		assert_eq!(store.load().as_deref(), Some("r2"));
	}

	#[tokio::test]
	async fn playlists_returns_owned_and_favorited() {
		let server = MockServer::start().await;
		let api = connected(&server).await;
		Mock::given(method("GET"))
			.and(path("/v1/users/42/playlistsAndFavoritePlaylists"))
			.and(query_param("countryCode", "FR"))
			.respond_with(ResponseTemplate::new(200).set_body_string(PLAYLISTS))
			.mount(&server)
			.await;
		let playlists = api.playlists().await.unwrap();
		assert_eq!(playlists[0].id, PlaylistId("aaaa-1111".into()));
		assert_eq!(playlists[0].cover.as_deref(), Some("sq-a"));
		assert_eq!(playlists[1].cover.as_deref(), Some("img-b"));
		assert_eq!(
			playlists.iter().map(|p| p.owned).collect::<Vec<_>>(),
			vec![true, false]
		);
	}

	#[tokio::test]
	async fn playlist_tracks_paginates_all_pages() {
		let server = MockServer::start().await;
		let api = connected(&server).await;
		// TIDAL rejects pages above 50 items ("Too big page, max page size is [50]").
		for (offset, count) in [(0, 50), (50, 50), (100, 50), (150, 50), (200, 50)] {
			let items = (0..count)
				.map(|i| json!({ "type": "track", "item": track_json(offset + i) }))
				.collect();
			Mock::given(method("GET"))
				.and(path("/v1/playlists/aaaa-1111/items"))
				.and(query_param("limit", "50"))
				.and(query_param("offset", offset.to_string()))
				.respond_with(ResponseTemplate::new(200).set_body_json(page(items, 250)))
				.expect(1)
				.mount(&server)
				.await;
		}
		let tracks = api
			.playlist_tracks(&PlaylistId("aaaa-1111".into()))
			.await
			.unwrap();
		assert_eq!(tracks.len(), 250);
		assert_eq!(tracks[249].id, TrackId(249));
	}

	#[tokio::test]
	async fn search_maps_tracks_artists_albums() {
		let server = MockServer::start().await;
		let api = connected(&server).await;
		Mock::given(method("GET"))
			.and(path("/v1/search"))
			.and(query_param("query", "nightcall"))
			.respond_with(ResponseTemplate::new(200).set_body_json(json!({
				"tracks": page(vec![track_json(1)], 1),
				"artists": page(vec![serde_json::from_str(ARTIST).unwrap()], 1),
				"albums": page(vec![serde_json::from_str(ALBUM).unwrap()], 1),
			})))
			.mount(&server)
			.await;
		let results = api.search("nightcall").await.unwrap();
		assert_eq!(results.tracks[0].title, "T1");
		assert_eq!(results.tracks[0].artist_names(), "Kavinsky, Lovefoxxx");
		assert_eq!(results.artists[0].name, "Kavinsky");
		assert_eq!(results.albums[0].year, Some(2013));
	}

	#[tokio::test]
	async fn track_radio_returns_tracks() {
		let server = MockServer::start().await;
		let api = connected(&server).await;
		mount_json(
			&server,
			"/v1/tracks/5/radio",
			page(vec![track_json(6), track_json(7)], 2),
		)
		.await;
		let tracks = api.track_radio(TrackId(5)).await.unwrap();
		assert_eq!(
			tracks.iter().map(|t| t.id.0).collect::<Vec<_>>(),
			vec![6, 7]
		);
	}

	#[tokio::test]
	async fn album_and_unstreamable_track() {
		let server = MockServer::start().await;
		let api = connected(&server).await;
		mount_json(
			&server,
			"/v1/albums/77646164",
			serde_json::from_str(ALBUM).unwrap(),
		)
		.await;
		let mut locked = track_json(2);
		locked["allowStreaming"] = json!(false);
		mount_json(
			&server,
			"/v1/albums/77646164/tracks",
			page(vec![track_json(1), locked], 2),
		)
		.await;
		let (album, tracks) = api.album(AlbumId(77646164)).await.unwrap();
		assert_eq!(album.title, "OutRun");
		assert_eq!(album.track_count, 13);
		assert!(tracks[0].available);
		assert!(!tracks[1].available);
	}

	#[tokio::test]
	async fn artist_page_has_top_tracks_albums_and_singles() {
		let server = MockServer::start().await;
		let api = connected(&server).await;
		mount_json(
			&server,
			"/v1/artists/3586",
			serde_json::from_str(ARTIST).unwrap(),
		)
		.await;
		mount_json(
			&server,
			"/v1/artists/3586/toptracks",
			page(vec![track_json(1)], 1),
		)
		.await;
		let album: Value = serde_json::from_str(ALBUM).unwrap();
		Mock::given(method("GET"))
			.and(path("/v1/artists/3586/albums"))
			.and(query_param("filter", "EPSANDSINGLES"))
			.respond_with(
				ResponseTemplate::new(200)
					.set_body_json(page(vec![album.clone(), album.clone()], 2)),
			)
			.mount(&server)
			.await;
		mount_json(&server, "/v1/artists/3586/albums", page(vec![album], 1)).await;
		let artist = api.artist(crate::model::ArtistId(3586)).await.unwrap();
		assert_eq!(artist.artist.picture.as_deref(), Some("pic-kav"));
		assert_eq!(artist.top_tracks.len(), 1);
		assert_eq!(artist.albums.len(), 1);
		assert_eq!(artist.singles.len(), 2);
	}

	#[tokio::test]
	async fn http_404_is_not_found() {
		let server = MockServer::start().await;
		let api = connected(&server).await;
		Mock::given(method("GET"))
			.and(path("/v1/albums/1"))
			.respond_with(ResponseTemplate::new(404))
			.mount(&server)
			.await;
		assert_eq!(
			api.album(AlbumId(1)).await.unwrap_err(),
			TidalError::NotFound
		);
	}

	async fn mount_etag(server: &MockServer) {
		Mock::given(method("GET"))
			.and(path("/v1/playlists/aaaa-1111"))
			.respond_with(
				ResponseTemplate::new(200)
					.insert_header("ETag", "\"42\"")
					.set_body_json(json!({ "uuid": "aaaa-1111", "title": "Night Drive" })),
			)
			.mount(server)
			.await;
	}

	fn pid() -> PlaylistId {
		PlaylistId("aaaa-1111".into())
	}

	#[tokio::test]
	async fn add_to_playlist_sends_etag() {
		let server = MockServer::start().await;
		let api = connected(&server).await;
		mount_etag(&server).await;
		Mock::given(method("POST"))
			.and(path("/v1/playlists/aaaa-1111/items"))
			.and(header("if-none-match", "\"42\""))
			.and(wiremock::matchers::body_string_contains("trackIds=5"))
			.respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
			.expect(1)
			.mount(&server)
			.await;
		api.add_to_playlist(&pid(), TrackId(5)).await.unwrap();
	}

	#[tokio::test]
	async fn remove_uses_index_and_etag() {
		let server = MockServer::start().await;
		let api = connected(&server).await;
		mount_etag(&server).await;
		Mock::given(method("DELETE"))
			.and(path("/v1/playlists/aaaa-1111/items/3"))
			.and(header("if-none-match", "\"42\""))
			.respond_with(ResponseTemplate::new(200))
			.expect(1)
			.mount(&server)
			.await;
		api.remove_from_playlist(&pid(), 3).await.unwrap();
	}

	#[tokio::test]
	async fn move_sends_to_index() {
		let server = MockServer::start().await;
		let api = connected(&server).await;
		mount_etag(&server).await;
		Mock::given(method("POST"))
			.and(path("/v1/playlists/aaaa-1111/items/4"))
			.and(header("if-none-match", "\"42\""))
			.and(wiremock::matchers::body_string_contains("toIndex=1"))
			.respond_with(ResponseTemplate::new(200))
			.expect(1)
			.mount(&server)
			.await;
		api.move_in_playlist(&pid(), 4, 1).await.unwrap();
	}

	#[tokio::test]
	async fn create_returns_owned_playlist() {
		let server = MockServer::start().await;
		let api = connected(&server).await;
		Mock::given(method("POST"))
			.and(path("/v1/users/42/playlists"))
			.and(wiremock::matchers::body_string_contains("title=Roadtrip"))
			.respond_with(ResponseTemplate::new(201).set_body_json(json!({
				"uuid": "cccc-3333", "title": "Roadtrip", "numberOfTracks": 0, "duration": 0
			})))
			.mount(&server)
			.await;
		let playlist = api.create_playlist("Roadtrip").await.unwrap();
		assert_eq!(playlist.id, PlaylistId("cccc-3333".into()));
		assert!(playlist.owned);
	}

	#[tokio::test]
	async fn rename_and_delete() {
		let server = MockServer::start().await;
		let api = connected(&server).await;
		mount_etag(&server).await;
		Mock::given(method("POST"))
			.and(path("/v1/playlists/aaaa-1111"))
			.and(wiremock::matchers::body_string_contains("title=Late+Drive"))
			.respond_with(ResponseTemplate::new(200))
			.expect(1)
			.mount(&server)
			.await;
		Mock::given(method("DELETE"))
			.and(path("/v1/playlists/aaaa-1111"))
			.respond_with(ResponseTemplate::new(204))
			.expect(1)
			.mount(&server)
			.await;
		api.rename_playlist(&pid(), "Late Drive").await.unwrap();
		api.delete_playlist(&pid()).await.unwrap();
	}

	#[tokio::test]
	async fn stream_sends_quality_and_full_presentation() {
		use base64::Engine;
		let server = MockServer::start().await;
		let api = connected(&server).await;
		let manifest = base64::engine::general_purpose::STANDARD.encode(
			r#"{"mimeType":"audio/flac","codecs":"flac","encryptionType":"NONE","urls":["https://x/y.flac"]}"#,
		);
		Mock::given(method("GET"))
			.and(path("/v1/tracks/7/playbackinfopostpaywall"))
			.and(query_param("audioquality", "LOSSLESS"))
			.and(query_param("playbackmode", "STREAM"))
			.and(query_param("assetpresentation", "FULL"))
			.respond_with(ResponseTemplate::new(200).set_body_json(json!({
				"trackId": 7, "audioQuality": "LOSSLESS", "bitDepth": 16, "sampleRate": 44100,
				"trackReplayGain": -9.5, "trackPeakAmplitude": 0.98,
				"manifestMimeType": "application/vnd.tidal.bts", "manifest": manifest
			})))
			.mount(&server)
			.await;
		let info = api
			.stream(TrackId(7), AudioQuality::Lossless)
			.await
			.unwrap();
		assert_eq!(info.quality, AudioQuality::Lossless);
		assert_eq!(info.bit_depth, Some(16));
		assert_eq!(info.replay_gain, Some(-9.5));
		assert_eq!(info.peak, Some(0.98));
		assert_eq!(
			info.source,
			crate::tidal::manifest::StreamSource::Progressive {
				url: "https://x/y.flac".into()
			}
		);
	}

	#[tokio::test]
	async fn display_name_prefers_full_name_then_username() {
		let server = MockServer::start().await;
		let api = connected(&server).await;
		Mock::given(method("GET"))
			.and(path("/v1/users/42"))
			.respond_with(ResponseTemplate::new(200).set_body_json(json!({
				"id": 42, "username": "nightrider", "firstName": "Alex", "lastName": "Martin"
			})))
			.up_to_n_times(1)
			.mount(&server)
			.await;
		Mock::given(method("GET"))
			.and(path("/v1/users/42"))
			.respond_with(ResponseTemplate::new(200).set_body_json(json!({
				"id": 42, "username": "nightrider", "firstName": null, "lastName": null
			})))
			.mount(&server)
			.await;
		assert_eq!(api.display_name().await, "Alex Martin");
		assert_eq!(api.display_name().await, "nightrider");
	}

	#[tokio::test]
	async fn connection_refused_is_network_error() {
		let base = url::Url::parse("http://127.0.0.1:9/v1/").unwrap();
		let endpoints = Endpoints {
			auth: base.clone(),
			api: base,
		};
		let creds = ClientCreds {
			client_id: "cid".into(),
			client_secret: "secret".into(),
			client_id_pkce: None,
			client_secret_pkce: None,
		};
		let result = HttpTidal::connect(
			endpoints.clone(),
			Auth::new(endpoints, creds),
			tokens("a"),
			Arc::new(MemoryStore::default()),
		)
		.await;
		assert!(matches!(result, Err(TidalError::Network(_))));
	}
}
