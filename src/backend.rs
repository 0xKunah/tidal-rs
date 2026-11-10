//! The network thread: a tokio runtime answering UI requests with TIDAL calls.
//! The UI never awaits it; responses come back on a channel.

use std::path::PathBuf;
use std::sync::Arc;

use crate::model::{
	Album, AlbumId, ArtistId, ArtistPage, AudioQuality, Playlist, PlaylistId, SearchResults, Track,
	TrackId, cover_url,
};
use crate::playlist_index::PlaylistIndex;
use crate::tidal::auth::{Auth, DeviceCode, PkceLogin, TokenStore, Tokens};
use crate::tidal::config::load_client_creds;
use crate::tidal::http::HttpTidal;
use crate::tidal::manifest::StreamInfo;
use crate::tidal::{Endpoints, TidalApi, TidalError};

type Result<T> = std::result::Result<T, TidalError>;

#[derive(Clone, Debug, PartialEq)]
pub enum Request {
	Playlists,
	Playlist(PlaylistId),
	Album(AlbumId),
	Artist(ArtistId),
	Search(String),
	Suggestions(Track),
	Stream {
		generation: u64,
		track: TrackId,
		quality: AudioQuality,
	},
	/// Stream info for the track expected to play next.
	Prefetch {
		track: TrackId,
		quality: AudioQuality,
	},
	CreatePlaylist {
		title: String,
		add: Option<TrackId>,
	},
	RenamePlaylist {
		id: PlaylistId,
		title: String,
	},
	DeletePlaylist(PlaylistId),
	AddToPlaylist {
		id: PlaylistId,
		track: TrackId,
	},
	RemoveFromPlaylist {
		id: PlaylistId,
		index: usize,
	},
	MovePlaylistItem {
		id: PlaylistId,
		from: usize,
		to: usize,
	},
	Cover {
		uuid: String,
		size: u32,
	},
	StartLogin,
	/// Address of the page TIDAL redirected to after the browser (PKCE) login.
	CompletePkce(String),
	/// Retries the startup connection (e.g. after being offline).
	Reconnect,
	Logout,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Response {
	Playlists(Result<Vec<Playlist>>),
	Playlist(PlaylistId, Result<Vec<Track>>),
	Album(AlbumId, Result<(Album, Vec<Track>)>),
	Artist(ArtistId, Result<ArtistPage>),
	Search(String, Result<SearchResults>),
	Suggestions(TrackId, Result<Vec<Track>>),
	Stream {
		generation: u64,
		result: Result<StreamInfo>,
	},
	Prefetch {
		track: TrackId,
		quality: AudioQuality,
		result: Result<StreamInfo>,
	},
	PlaylistEdited {
		id: Option<PlaylistId>,
		result: Result<()>,
	},
	Cover {
		uuid: String,
		size: u32,
		path: PathBuf,
	},
	PlaylistIndex(PlaylistIndex),
	LoginCode(DeviceCode),
	/// Browser (PKCE) login: the page to open.
	PkceUrl(String),
	LoggedIn {
		name: String,
	},
	NeedsLogin,
	/// `client.toml` is missing or invalid; carries its path.
	MissingCredentials(PathBuf),
	LoginFailed(String),
}

/// Answers the requests that map to TIDAL API calls.
pub async fn handle<A: TidalApi>(api: &A, request: Request) -> Vec<Response> {
	match request {
		Request::Playlists => vec![Response::Playlists(api.playlists().await)],
		Request::Playlist(id) => {
			let tracks = api.playlist_tracks(&id).await;
			vec![Response::Playlist(id, tracks)]
		}
		Request::Album(id) => vec![Response::Album(id, api.album(id).await)],
		Request::Artist(id) => vec![Response::Artist(id, api.artist(id).await)],
		Request::Search(query) => {
			let results = api.search(&query).await;
			vec![Response::Search(query, results)]
		}
		Request::Suggestions(seed) => {
			vec![Response::Suggestions(
				seed.id,
				api.track_radio(seed.id).await,
			)]
		}
		Request::Stream {
			generation,
			track,
			quality,
		} => vec![Response::Stream {
			generation,
			result: api.stream(track, quality).await,
		}],
		Request::Prefetch { track, quality } => vec![Response::Prefetch {
			track,
			quality,
			result: api.stream(track, quality).await,
		}],
		Request::CreatePlaylist { title, add } => match api.create_playlist(&title).await {
			Ok(playlist) => {
				let result = match add {
					Some(track) => api.add_to_playlist(&playlist.id, track).await,
					None => Ok(()),
				};
				edited(api, Some(playlist.id), result).await
			}
			Err(error) => vec![Response::PlaylistEdited {
				id: None,
				result: Err(error),
			}],
		},
		Request::RenamePlaylist { id, title } => {
			let result = api.rename_playlist(&id, &title).await;
			edited(api, Some(id), result).await
		}
		Request::DeletePlaylist(id) => {
			let result = api.delete_playlist(&id).await;
			let mut out = vec![Response::PlaylistEdited {
				id: Some(id),
				result: result.clone(),
			}];
			if result.is_ok() {
				out.push(Response::Playlists(api.playlists().await));
			}
			out
		}
		Request::AddToPlaylist { id, track } => {
			let result = api.add_to_playlist(&id, track).await;
			edited(api, Some(id), result).await
		}
		Request::RemoveFromPlaylist { id, index } => {
			let result = api.remove_from_playlist(&id, index).await;
			edited(api, Some(id), result).await
		}
		Request::MovePlaylistItem { id, from, to } => {
			let result = api.move_in_playlist(&id, from, to).await;
			edited(api, Some(id), result).await
		}
		Request::Cover { .. }
		| Request::StartLogin
		| Request::CompletePkce(_)
		| Request::Reconnect
		| Request::Logout => vec![],
	}
}

/// After an edit: the outcome, then the refreshed playlist and playlist list.
async fn edited<A: TidalApi>(api: &A, id: Option<PlaylistId>, result: Result<()>) -> Vec<Response> {
	let ok = result.is_ok();
	let mut out = vec![Response::PlaylistEdited {
		id: id.clone(),
		result,
	}];
	if ok {
		if let Some(id) = id {
			let tracks = api.playlist_tracks(&id).await;
			out.push(Response::Playlist(id, tracks));
		}
		out.push(Response::Playlists(api.playlists().await));
	}
	out
}

pub struct Backend {
	pub requests: async_channel::Sender<Request>,
	pub responses: async_channel::Receiver<Response>,
	pub runtime: tokio::runtime::Handle,
}

pub struct BackendConfig {
	pub endpoints: Endpoints,
	pub store: Arc<dyn TokenStore>,
	pub creds_path: PathBuf,
	pub cover_dir: PathBuf,
}

/// Starts the runtime thread and connects with the saved refresh token if any.
pub fn spawn_backend(config: BackendConfig) -> Backend {
	let runtime = tokio::runtime::Builder::new_multi_thread()
		.enable_all()
		.thread_name("backend")
		.build()
		.expect("failed to start the network runtime");
	let handle = runtime.handle().clone();
	let (request_tx, request_rx) = async_channel::unbounded();
	let (response_tx, response_rx) = async_channel::unbounded();
	let shared = Arc::new(Shared {
		config,
		pending_pkce: std::sync::Mutex::new(None),
		out: response_tx,
		api: tokio::sync::RwLock::new(None),
		http: reqwest::Client::new(),
	});
	std::thread::Builder::new()
		.name("backend".into())
		.spawn(move || runtime.block_on(run(shared, request_rx)))
		.expect("failed to start the network thread");
	Backend {
		requests: request_tx,
		responses: response_rx,
		runtime: handle,
	}
}

struct Shared {
	config: BackendConfig,
	/// Secrets of the browser login in progress.
	pending_pkce: std::sync::Mutex<Option<PkceLogin>>,
	out: async_channel::Sender<Response>,
	api: tokio::sync::RwLock<Option<Arc<HttpTidal>>>,
	http: reqwest::Client,
}

impl Shared {
	fn send(&self, response: Response) {
		let _ = self.out.try_send(response);
	}

	/// Reads `client.toml` on every attempt so "Try again" picks up a newly created file.
	fn auth(&self) -> Option<Auth> {
		match load_client_creds(&self.config.creds_path) {
			Ok(creds) => Some(Auth::new(self.config.endpoints.clone(), creds)),
			Err(_) => {
				self.send(Response::MissingCredentials(self.config.creds_path.clone()));
				None
			}
		}
	}
}

async fn run(shared: Arc<Shared>, requests: async_channel::Receiver<Request>) {
	tokio::spawn(startup(shared.clone()));
	while let Ok(request) = requests.recv().await {
		let shared = shared.clone();
		match request {
			Request::StartLogin => {
				tokio::spawn(login(shared));
			}
			Request::CompletePkce(address) => {
				tokio::spawn(complete_pkce(shared, address));
			}
			Request::Reconnect => {
				tokio::spawn(startup(shared));
			}
			Request::Logout => {
				shared.config.store.clear();
				*shared.api.write().await = None;
				shared.send(Response::NeedsLogin);
			}
			Request::Cover { uuid, size } => {
				tokio::spawn(cover(shared, uuid, size));
			}
			other => {
				let api = shared.api.read().await.clone();
				if let Some(api) = api {
					tokio::spawn(async move {
						for response in handle(api.as_ref(), other).await {
							shared.send(response);
						}
					});
				}
			}
		}
	}
}

async fn startup(shared: Arc<Shared>) {
	let Some(auth) = shared.auth() else {
		return;
	};
	let Some(refresh) = shared.config.store.load() else {
		shared.send(Response::NeedsLogin);
		return;
	};
	match auth.refresh(&refresh).await {
		Ok(tokens) => {
			shared.config.store.save(&tokens.refresh);
			connect(&shared, auth, tokens).await;
		}
		Err(TidalError::Unauthorized) => shared.send(Response::NeedsLogin),
		Err(error) => shared.send(Response::LoginFailed(error.to_string())),
	}
}

async fn login(shared: Arc<Shared>) {
	let Some(auth) = shared.auth() else {
		return;
	};
	if auth.is_pkce() {
		let pending = PkceLogin::new();
		match auth.pkce_login_url(&pending) {
			Ok(url) => {
				if let Ok(mut slot) = shared.pending_pkce.lock() {
					*slot = Some(pending);
				}
				shared.send(Response::PkceUrl(url));
			}
			Err(error) => shared.send(Response::LoginFailed(error.to_string())),
		}
		return;
	}
	let code = match auth.start_device().await {
		Ok(code) => code,
		Err(error) => {
			shared.send(Response::LoginFailed(error.to_string()));
			return;
		}
	};
	shared.send(Response::LoginCode(code.clone()));
	match auth.poll_device(&code).await {
		Ok(tokens) => {
			shared.config.store.save(&tokens.refresh);
			connect(&shared, auth, tokens).await;
		}
		Err(error) => shared.send(Response::LoginFailed(error.to_string())),
	}
}

async fn complete_pkce(shared: Arc<Shared>, address: String) {
	let Some(auth) = shared.auth() else {
		return;
	};
	let pending = shared
		.pending_pkce
		.lock()
		.ok()
		.and_then(|slot| slot.clone());
	let Some(pending) = pending else {
		shared.send(Response::LoginFailed(
			rust_i18n::t!("login.bad_redirect").to_string(),
		));
		return;
	};
	match auth.exchange_pkce(&address, &pending).await {
		Ok(tokens) => {
			shared.config.store.save(&tokens.refresh);
			connect(&shared, auth, tokens).await;
		}
		Err(error) => shared.send(Response::LoginFailed(error.to_string())),
	}
}

async fn connect(shared: &Arc<Shared>, auth: Auth, tokens: Tokens) {
	let endpoints = shared.config.endpoints.clone();
	let api = match HttpTidal::connect(endpoints, auth, tokens, shared.config.store.clone()).await {
		Ok(api) => Arc::new(api),
		Err(error) => {
			shared.send(Response::LoginFailed(error.to_string()));
			return;
		}
	};
	let name = api.display_name().await;
	*shared.api.write().await = Some(api.clone());
	shared.send(Response::LoggedIn { name });
	let playlists = api.playlists().await;
	shared.send(Response::Playlists(playlists.clone()));
	if let Ok(playlists) = playlists {
		let mut index = PlaylistIndex::default();
		for playlist in playlists.iter().filter(|playlist| playlist.owned) {
			if let Ok(tracks) = api.playlist_tracks(&playlist.id).await {
				index.set(playlist.id.clone(), &tracks);
			}
		}
		shared.send(Response::PlaylistIndex(index));
	}
}

async fn cover(shared: Arc<Shared>, uuid: String, size: u32) {
	let path = shared.config.cover_dir.join(format!("{uuid}_{size}.jpg"));
	if !path.exists() {
		let Ok(response) = shared.http.get(cover_url(&uuid, size)).send().await else {
			return;
		};
		if !response.status().is_success() {
			return;
		}
		let Ok(bytes) = response.bytes().await else {
			return;
		};
		if tokio::fs::create_dir_all(&shared.config.cover_dir)
			.await
			.is_err()
		{
			return;
		}
		let tmp = path.with_extension("part");
		if tokio::fs::write(&tmp, &bytes).await.is_err()
			|| tokio::fs::rename(&tmp, &path).await.is_err()
		{
			return;
		}
	}
	shared.send(Response::Cover { uuid, size, path });
}

/// `%LOCALAPPDATA%\tidal-rs\covers`.
pub fn cover_dir() -> PathBuf {
	directories::BaseDirs::new()
		.map(|dirs| dirs.data_local_dir().to_path_buf())
		.unwrap_or_else(std::env::temp_dir)
		.join("tidal-rs")
		.join("covers")
}

#[cfg(test)]
mod tests {
	use std::sync::Mutex;
	use std::time::Duration;

	use super::*;
	use crate::model::{Album, AlbumId, ArtistId, ArtistPage, Playlist, PlaylistId, SearchResults};
	use crate::test_support::list;
	use crate::tidal::TidalResult;
	use crate::tidal::manifest::StreamInfo;

	#[derive(Default)]
	struct MockApi {
		calls: Mutex<Vec<String>>,
	}

	impl MockApi {
		fn record(&self, call: String) {
			self.calls.lock().unwrap().push(call);
		}

		fn calls(&self) -> Vec<String> {
			self.calls.lock().unwrap().clone()
		}
	}

	fn playlist(id: &str) -> Playlist {
		Playlist {
			id: PlaylistId(id.into()),
			title: id.into(),
			cover: None,
			track_count: 0,
			duration: Duration::ZERO,
			owned: true,
		}
	}

	impl TidalApi for MockApi {
		async fn playlists(&self) -> TidalResult<Vec<Playlist>> {
			self.record("playlists".into());
			Ok(vec![playlist("p1")])
		}
		async fn playlist_tracks(&self, id: &PlaylistId) -> TidalResult<Vec<Track>> {
			self.record(format!("playlist_tracks {}", id.0));
			Ok(list(&[1, 2]))
		}
		async fn album(&self, _id: AlbumId) -> TidalResult<(Album, Vec<Track>)> {
			Err(TidalError::NotFound)
		}
		async fn artist(&self, _id: ArtistId) -> TidalResult<ArtistPage> {
			Err(TidalError::NotFound)
		}
		async fn search(&self, query: &str) -> TidalResult<SearchResults> {
			self.record(format!("search {query}"));
			Ok(SearchResults {
				tracks: list(&[1]),
				..SearchResults::default()
			})
		}
		async fn track_radio(&self, _id: TrackId) -> TidalResult<Vec<Track>> {
			Ok(list(&[5, 6]))
		}
		async fn stream(&self, _id: TrackId, _quality: AudioQuality) -> TidalResult<StreamInfo> {
			Err(TidalError::Unavailable)
		}
		async fn create_playlist(&self, title: &str) -> TidalResult<Playlist> {
			self.record(format!("create {title}"));
			Ok(playlist("new"))
		}
		async fn rename_playlist(&self, _id: &PlaylistId, _title: &str) -> TidalResult<()> {
			Ok(())
		}
		async fn delete_playlist(&self, _id: &PlaylistId) -> TidalResult<()> {
			Ok(())
		}
		async fn add_to_playlist(&self, id: &PlaylistId, track: TrackId) -> TidalResult<()> {
			self.record(format!("add {} {}", id.0, track.0));
			Ok(())
		}
		async fn remove_from_playlist(&self, _id: &PlaylistId, _index: usize) -> TidalResult<()> {
			Ok(())
		}
		async fn move_in_playlist(
			&self,
			_id: &PlaylistId,
			_from: usize,
			_to: usize,
		) -> TidalResult<()> {
			Ok(())
		}
	}

	#[tokio::test]
	async fn handle_routes_search_with_its_query() {
		let api = MockApi::default();
		let responses = handle(&api, Request::Search("a".into())).await;
		assert!(
			matches!(&responses[..], [Response::Search(query, Ok(results))]
			if query == "a" && results.tracks.len() == 1)
		);
	}

	#[tokio::test]
	async fn create_playlist_with_track_adds_it() {
		let api = MockApi::default();
		let responses = handle(
			&api,
			Request::CreatePlaylist {
				title: "Roadtrip".into(),
				add: Some(TrackId(9)),
			},
		)
		.await;
		assert_eq!(
			api.calls()[..2],
			["create Roadtrip".to_string(), "add new 9".to_string()]
		);
		assert!(responses.iter().any(|r| matches!(r,
			Response::PlaylistEdited { id: Some(id), result: Ok(()) } if id.0 == "new")));
		assert!(
			responses
				.iter()
				.any(|r| matches!(r, Response::Playlists(Ok(_))))
		);
	}

	#[tokio::test]
	async fn stream_response_keeps_its_generation() {
		let api = MockApi::default();
		let responses = handle(
			&api,
			Request::Stream {
				generation: 7,
				track: TrackId(1),
				quality: AudioQuality::Lossless,
			},
		)
		.await;
		assert!(matches!(
			&responses[..],
			[Response::Stream {
				generation: 7,
				result: Err(TidalError::Unavailable)
			}]
		));
	}
}
