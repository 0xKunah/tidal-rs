//! The app's single source of truth on the UI thread. It owns the pure `Player`
//! and executes its effects through the backend (network) and audio threads.
//! Views read it and call its methods; they never talk to those threads.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui_kit::*;

use crate::audio::engine::{AudioCmd, AudioEvent, AudioHandle};
use crate::backend::{Request, Response};
use crate::controller::{
	ErrorPolicy, Generations, LoadDebouncer, PendingLoad, audio_error_policy, playback_error_policy,
};
use crate::i18n::{self, Lang};
use crate::model::AudioQuality;
use crate::model::TrackId;
use crate::model::{
	Album, AlbumId, ArtistId, ArtistPage, ContextRef, Playlist, PlaylistId, QueueItem, Route, Track,
};
use crate::player::{Command, Effect, Player};
use crate::playlist_index::PlaylistIndex;
use crate::recent::RecentList;
use crate::search::SearchState;
use crate::sort::{SortKey, TrackSort};
use crate::store::{Settings, Snapshot};
use crate::tidal::TidalError;
use crate::tidal::auth::DeviceCode;
use crate::tidal::manifest::{StreamInfo, StreamSource};

pub const COVER_SMALL: u32 = 160;

/// What was last pushed to the Windows media controls.
#[cfg(windows)]
#[derive(Default)]
struct MediaSync {
	initialized: bool,
	track: Option<crate::model::TrackId>,
	playing: bool,
	last_state: Option<Instant>,
}
pub const COVER_LARGE: u32 = 640;
const TOAST_DURATION: Duration = Duration::from_secs(3);
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(300);
/// Session changes are written this long after the last one.
const SAVE_DEBOUNCE: Duration = Duration::from_secs(1);
/// Playback position alone is saved at most this often.
const POSITION_SAVE_INTERVAL: Duration = Duration::from_secs(5);

/// Remote data as seen by a page.
#[derive(Clone, Debug, PartialEq)]
pub enum Remote<T> {
	Loading,
	Ready(T),
	Failed(String),
}

impl<T> Remote<T> {
	pub fn ready(&self) -> Option<&T> {
		match self {
			Remote::Ready(value) => Some(value),
			_ => None,
		}
	}
}

fn remote<T>(result: Result<T, TidalError>) -> Remote<T> {
	match result {
		Ok(value) => Remote::Ready(value),
		Err(error) => Remote::Failed(error.to_string()),
	}
}

#[derive(Clone, Debug, PartialEq)]
pub enum LoginState {
	Connecting,
	Code(DeviceCode),
	/// Browser login: open `url`, then paste the redirect address.
	Pkce {
		url: String,
		error: Option<String>,
	},
	MissingCredentials(PathBuf),
	Failed(String),
	LoggedIn {
		name: String,
	},
}

#[derive(Clone, Debug)]
pub struct Toast {
	pub id: u64,
	pub text: SharedString,
}

pub struct AppState {
	pub player: Player,
	pub recent: RecentList,
	pub settings: Settings,
	pub route: Option<Route>,
	pub last_playlist: Option<PlaylistId>,
	pub queue_panel_open: bool,
	pub playlist_sorts: HashMap<PlaylistId, TrackSort>,
	pub playlists: Remote<Vec<Playlist>>,
	pub playlist_tracks: HashMap<PlaylistId, Remote<Vec<Track>>>,
	pub albums: HashMap<AlbumId, Remote<(Album, Vec<Track>)>>,
	pub artists: HashMap<ArtistId, Remote<ArtistPage>>,
	pub index: PlaylistIndex,
	pub login: LoginState,
	/// Quality label of the stream currently loaded.
	pub badge: Option<SharedString>,
	pub toasts: Vec<Toast>,
	/// Last single-clicked track; `Q` queues it.
	pub selected: Option<QueueItem>,
	pub search: SearchState,
	/// Scroll state of the virtualized tables, by table id.
	list_states: std::cell::RefCell<HashMap<&'static str, ListState>>,
	search_timer: Option<Task<()>>,
	/// Where the session is saved; None in demo mode.
	state_path: Option<PathBuf>,
	save_timer: Option<Task<()>>,
	last_save: Instant,
	/// The startup route is validated once, when the playlists first arrive.
	startup_route_pending: bool,
	/// Generation whose audio actually started.
	started_generation: Option<u64>,
	/// Track whose stream info was requested ahead of time.
	prefetch_requested: Option<TrackId>,
	/// Stream info of the track being downloaded ahead of time.
	prefetched: Option<(TrackId, AudioQuality, StreamInfo)>,
	/// Login page of the browser (PKCE) flow in progress.
	pkce_url: Option<String>,
	/// The stream sent to the audio engine was DASH (Hi-Res).
	loaded_segmented: bool,
	/// Hi-Res failed to decode once: use Lossless for the rest of the session.
	pub hires_fallback: bool,
	#[cfg(windows)]
	media: Option<crate::os::windows::MediaControls>,
	#[cfg(windows)]
	media_sync: MediaSync,
	covers: HashMap<(String, u32), PathBuf>,
	requested_covers: HashSet<(String, u32)>,
	debouncer: LoadDebouncer,
	load_timer: Option<Task<()>>,
	generations: Generations,
	current_load: Option<PendingLoad>,
	attempt: u32,
	retry_timer: Option<Task<()>>,
	next_toast: u64,
	backend: async_channel::Sender<Request>,
	audio: AudioHandle,
	_tasks: Vec<Task<()>>,
}

impl AppState {
	pub fn new(
		snapshot: Snapshot,
		state_path: Option<PathBuf>,
		backend: async_channel::Sender<Request>,
		responses: async_channel::Receiver<Response>,
		audio: AudioHandle,
		audio_events: async_channel::Receiver<AudioEvent>,
		cx: &mut Context<Self>,
	) -> Self {
		let response_task = cx.spawn(async move |this, cx| {
			while let Ok(response) = responses.recv().await {
				if this
					.update(cx, |state, cx| state.on_response(response, cx))
					.is_err()
				{
					return;
				}
			}
		});
		let audio_task = cx.spawn(async move |this, cx| {
			while let Ok(event) = audio_events.recv().await {
				if this
					.update(cx, |state, cx| state.on_audio(event, cx))
					.is_err()
				{
					return;
				}
			}
		});
		let mut state = Self {
			player: snapshot.player,
			recent: snapshot.recent,
			settings: snapshot.settings,
			route: snapshot.route,
			last_playlist: snapshot.last_playlist,
			queue_panel_open: snapshot.queue_panel_open,
			playlist_sorts: snapshot.playlist_sorts,
			playlists: Remote::Loading,
			playlist_tracks: HashMap::new(),
			albums: HashMap::new(),
			artists: HashMap::new(),
			index: PlaylistIndex::default(),
			login: LoginState::Connecting,
			badge: None,
			toasts: Vec::new(),
			selected: None,
			search: SearchState::default(),
			list_states: Default::default(),
			search_timer: None,
			state_path,
			save_timer: None,
			last_save: Instant::now(),
			startup_route_pending: true,
			started_generation: None,
			prefetch_requested: None,
			prefetched: None,
			pkce_url: None,
			loaded_segmented: false,
			hires_fallback: false,
			#[cfg(windows)]
			media: None,
			#[cfg(windows)]
			media_sync: MediaSync::default(),
			covers: HashMap::new(),
			requested_covers: HashSet::new(),
			debouncer: LoadDebouncer::new(LoadDebouncer::DELAY),
			load_timer: None,
			generations: Generations::default(),
			current_load: None,
			attempt: 0,
			retry_timer: None,
			next_toast: 0,
			backend,
			audio,
			_tasks: vec![response_task, audio_task],
		};
		cx.on_app_quit(|state, _| {
			state.save_now();
			async {}
		})
		.detach();
		let resume = state.player.resume_effects();
		state.run(resume, cx);
		if let Some(now) = state.player.view().now {
			state.want_cover(now.track.album.cover.as_deref(), COVER_SMALL);
		}
		state
	}

	fn send(&self, request: Request) {
		let _ = self.backend.try_send(request);
	}

	pub fn is_logged_in(&self) -> bool {
		matches!(self.login, LoginState::LoggedIn { .. })
	}

	// ---- player ----

	pub fn dispatch(&mut self, cmd: Command, cx: &mut Context<Self>) {
		let position_only = matches!(cmd, Command::Progress(_));
		let effects = self.player.handle(cmd);
		self.run(effects, cx);
		if !position_only || self.last_save.elapsed() >= POSITION_SAVE_INTERVAL {
			self.mark_dirty(cx);
		}
		#[cfg(windows)]
		self.sync_media();
		self.maybe_prefetch();
		cx.notify();
	}

	/// Once the current track plays, fetches the next one ahead so it starts without a gap.
	fn maybe_prefetch(&mut self) {
		if self.started_generation != Some(self.generations.current()) {
			return;
		}
		let Some(next) = self.player.predicted_next() else {
			return;
		};
		if self.prefetch_requested == Some(next.id) {
			return;
		}
		self.prefetch_requested = Some(next.id);
		self.send(Request::Prefetch {
			track: next.id,
			quality: self.effective_quality(),
		});
	}

	// ---- Windows media controls ----

	/// Registers the window with the Windows media controls (media keys, overlay, Stream Deck).
	#[cfg(windows)]
	pub fn attach_media(&mut self, hwnd: isize, cx: &mut Context<Self>) {
		let (tx, rx) = async_channel::unbounded();
		match crate::os::windows::MediaControls::attach(hwnd, tx) {
			Ok(media) => self.media = Some(media),
			Err(error) => {
				eprintln!("media controls unavailable: {error}");
				return;
			}
		}
		cx.spawn(async move |this, cx| {
			while let Ok(button) = rx.recv().await {
				if this
					.update(cx, |state, cx| state.on_media_button(button, cx))
					.is_err()
				{
					return;
				}
			}
		})
		.detach();
		self.media_sync = MediaSync::default();
		self.sync_media();
	}

	#[cfg(windows)]
	fn on_media_button(&mut self, button: crate::os::windows::MediaButton, cx: &mut Context<Self>) {
		use crate::os::windows::MediaButton;
		let playing = self.player.view().playing;
		let command = match button {
			MediaButton::Play if !playing => Command::TogglePlay,
			MediaButton::Pause if playing => Command::TogglePlay,
			MediaButton::Next => Command::Next,
			MediaButton::Previous => Command::Previous,
			_ => return,
		};
		self.dispatch(command, cx);
	}

	/// Pushes track changes immediately and timeline updates at most once a second.
	#[cfg(windows)]
	fn sync_media(&mut self) {
		use crate::os::windows::NowPlaying;
		let Some(media) = &self.media else {
			return;
		};
		let view = self.player.view();
		let now = view.now.as_ref().map(|item| NowPlaying {
			title: item.track.title.clone(),
			artist: item.track.artist_names(),
			album: item.track.album.title.clone(),
			cover_url: item
				.track
				.album
				.cover
				.as_deref()
				.map(|uuid| crate::model::cover_url(uuid, COVER_LARGE)),
			playing: view.playing,
			position: view.position,
			duration: item.track.duration,
		});
		let track = view.now.as_ref().map(|item| item.track.id);
		let sync = &mut self.media_sync;
		if !sync.initialized || sync.track != track {
			let _ = media.set_track(now.as_ref());
			sync.track = track;
			sync.initialized = true;
			sync.last_state = None;
		}
		if let Some(now) = &now {
			let due = sync
				.last_state
				.is_none_or(|at| at.elapsed() >= Duration::from_secs(1));
			if due || sync.playing != now.playing {
				let _ = media.set_state(now);
				sync.playing = now.playing;
				sync.last_state = Some(Instant::now());
			}
		}
	}

	// ---- persistence ----

	/// Schedules a save shortly after the latest change.
	pub fn mark_dirty(&mut self, cx: &mut Context<Self>) {
		let Some(path) = self.state_path.clone() else {
			return;
		};
		self.save_timer = Some(cx.spawn(async move |this, cx| {
			cx.background_executor().timer(SAVE_DEBOUNCE).await;
			let Ok(snapshot) = this.update(cx, |state, _| {
				state.last_save = Instant::now();
				state.snapshot()
			}) else {
				return;
			};
			cx.background_executor()
				.spawn(async move {
					if let Err(error) = crate::store::save(&path, &snapshot) {
						eprintln!("could not save the session: {error}");
					}
				})
				.await;
		}));
	}

	/// Writes the session synchronously (on quit).
	pub fn save_now(&mut self) {
		if let Some(path) = &self.state_path
			&& let Err(error) = crate::store::save(path, &self.snapshot())
		{
			eprintln!("could not save the session: {error}");
		}
	}

	/// Starts a list. Playlists become the "last played playlist".
	pub fn play_list(
		&mut self,
		kind: ContextRef,
		tracks: Vec<Track>,
		start: Option<usize>,
		shuffle: Option<bool>,
		cx: &mut Context<Self>,
	) {
		if let ContextRef::Playlist { id, .. } = &kind {
			self.last_playlist = Some(id.clone());
		}
		self.dispatch(
			Command::PlayList {
				kind,
				tracks,
				start,
				shuffle,
			},
			cx,
		);
	}

	pub fn play_single(&mut self, item: QueueItem, cx: &mut Context<Self>) {
		self.dispatch(Command::PlaySingle(item), cx);
	}

	fn run(&mut self, effects: Vec<Effect>, cx: &mut Context<Self>) {
		for effect in effects {
			match effect {
				Effect::Load {
					track,
					start_at,
					autoplay,
				} => {
					self.audio.send(AudioCmd::Stop);
					self.badge = None;
					self.want_cover(track.album.cover.as_deref(), COVER_SMALL);
					self.debouncer.request(
						PendingLoad {
							track,
							start_at,
							autoplay,
						},
						Instant::now(),
					);
					self.schedule_load(cx);
				}
				Effect::Pause => self.audio.send(AudioCmd::Pause),
				Effect::Resume => self.audio.send(AudioCmd::Play),
				Effect::Seek(position) => self.audio.send(AudioCmd::Seek(position)),
				Effect::Volume(volume) => self.audio.send(AudioCmd::Volume(volume)),
				Effect::FetchSuggestions { seed } => self.send(Request::Suggestions(seed)),
				Effect::Played(item) => {
					self.recent.record(&item);
					self.mark_dirty(cx);
				}
				Effect::Stop => self.audio.send(AudioCmd::Stop),
			}
		}
	}

	fn schedule_load(&mut self, cx: &mut Context<Self>) {
		let Some(deadline) = self.debouncer.deadline() else {
			return;
		};
		let wait = deadline.saturating_duration_since(Instant::now());
		self.load_timer = Some(cx.spawn(async move |this, cx| {
			cx.background_executor().timer(wait).await;
			let _ = this.update(cx, |state, cx| state.fire_load(cx));
		}));
	}

	fn fire_load(&mut self, cx: &mut Context<Self>) {
		match self.debouncer.take_due(Instant::now()) {
			Some(load) => {
				let generation = self.generations.advance();
				self.attempt = 0;
				self.retry_timer = None;
				let quality = self.effective_quality();
				match self.prefetched.take() {
					Some((track, prefetched_quality, info))
						if track == load.track.id && prefetched_quality == quality =>
					{
						self.loaded_segmented =
							matches!(info.source, StreamSource::Segmented { .. });
						self.audio.send(AudioCmd::Load {
							generation,
							key: load.track.id.0,
							info,
							start_at: load.start_at,
							autoplay: self.player.view().playing,
						});
					}
					_ => self.request_stream(generation, &load),
				}
				self.current_load = Some(load);
			}
			None => self.schedule_load(cx),
		}
	}

	fn request_stream(&self, generation: u64, load: &PendingLoad) {
		self.send(Request::Stream {
			generation,
			track: load.track.id,
			quality: self.effective_quality(),
		});
	}

	/// The setting, capped at Lossless once Hi-Res proved undecodable.
	pub fn effective_quality(&self) -> AudioQuality {
		match self.settings.quality {
			AudioQuality::HiRes if self.hires_fallback => AudioQuality::Lossless,
			quality => quality,
		}
	}

	/// Applies an error policy for the current load: skip the track or retry it.
	fn on_playback_error(&mut self, policy: ErrorPolicy, cx: &mut Context<Self>) {
		match policy {
			ErrorPolicy::Skip => {
				self.toast(rust_i18n::t!("toast.unavailable"), cx);
				self.dispatch(Command::TrackUnavailable, cx);
			}
			ErrorPolicy::Retry { after } => {
				if self.attempt == 0 {
					self.toast(rust_i18n::t!("toast.network"), cx);
				}
				self.attempt += 1;
				let generation = self.generations.current();
				let position = self.player.view().position;
				self.retry_timer = Some(cx.spawn(async move |this, cx| {
					cx.background_executor().timer(after).await;
					let _ = this.update(cx, |state, _| {
						if !state.generations.is_current(generation) {
							return;
						}
						if let Some(load) = &mut state.current_load {
							load.start_at = position;
							let load = load.clone();
							state.request_stream(generation, &load);
						}
					});
				}));
			}
		}
	}

	fn on_audio(&mut self, event: AudioEvent, cx: &mut Context<Self>) {
		match event {
			AudioEvent::Started { generation, badge }
				if self.generations.is_current(generation) =>
			{
				self.started_generation = Some(generation);
				self.badge = Some(badge.into());
				self.attempt = 0;
				cx.notify();
			}
			AudioEvent::Position {
				generation,
				position,
			} if self.generations.is_current(generation) => {
				self.dispatch(Command::Progress(position), cx);
			}
			AudioEvent::Ended { generation } if self.generations.is_current(generation) => {
				self.dispatch(Command::TrackEnded, cx);
			}
			AudioEvent::Failed { generation, error } if self.generations.is_current(generation) => {
				if self.loaded_segmented
					&& matches!(error, crate::audio::engine::AudioError::Decode(_))
					&& !self.hires_fallback
				{
					self.hires_fallback = true;
					self.toast(rust_i18n::t!("settings.quality_max_unsupported"), cx);
					if let Some(load) = self.current_load.clone() {
						self.request_stream(generation, &load);
					}
					return;
				}
				let policy = audio_error_policy(&error, self.attempt);
				self.on_playback_error(policy, cx);
			}
			_ => {}
		}
	}

	// ---- backend ----

	fn on_response(&mut self, response: Response, cx: &mut Context<Self>) {
		match response {
			Response::Playlists(result) => {
				if let Ok(playlists) = &result {
					for playlist in playlists.clone() {
						self.want_cover(playlist.cover.as_deref(), COVER_SMALL);
					}
				}
				self.playlists = remote(result);
				if (self.startup_route_pending || self.route.is_none())
					&& let Some(playlists) = self.playlists.ready()
				{
					self.startup_route_pending = false;
					self.route = crate::store::initial_route(
						self.route.as_ref(),
						self.last_playlist.as_ref(),
						playlists,
					);
					if let Some(route) = self.route.clone() {
						self.navigate(route, cx);
					}
				}
			}
			Response::Playlist(id, result) => {
				if let Ok(tracks) = &result {
					self.index.set(id.clone(), tracks);
					self.want_track_covers(&tracks.clone());
				}
				self.playlist_tracks.insert(id, remote(result));
			}
			Response::Album(id, result) => {
				if let Ok((album, tracks)) = &result {
					self.want_cover(album.cover.clone().as_deref(), COVER_LARGE);
					self.want_track_covers(&tracks.clone());
				}
				self.albums.insert(id, remote(result));
			}
			Response::Artist(id, result) => {
				if let Ok(page) = &result {
					let page = page.clone();
					self.want_cover(page.artist.picture.as_deref(), COVER_LARGE);
					self.want_track_covers(&page.top_tracks);
					for album in page.albums.iter().chain(&page.singles) {
						self.want_cover(album.cover.as_deref(), COVER_SMALL);
					}
				}
				self.artists.insert(id, remote(result));
			}
			Response::Search(query, Ok(results)) => {
				self.want_track_covers(&results.tracks);
				for album in &results.albums {
					self.want_cover(album.cover.clone().as_deref(), COVER_SMALL);
				}
				for artist in &results.artists {
					self.want_cover(artist.picture.clone().as_deref(), COVER_SMALL);
				}
				self.search.apply(&query, results);
			}
			Response::Search(_, Err(_)) => {}
			Response::Suggestions(seed, Ok(tracks)) => {
				self.want_track_covers(&tracks);
				self.dispatch(Command::SuggestionsLoaded { seed, tracks }, cx);
			}
			Response::Suggestions(_, Err(_)) => {}
			Response::Stream { generation, result } => {
				if !self.generations.is_current(generation) {
					return;
				}
				match result {
					Ok(info) => {
						self.loaded_segmented = matches!(
							info.source,
							crate::tidal::manifest::StreamSource::Segmented { .. }
						);
						if let Some(load) = &self.current_load {
							self.audio.send(AudioCmd::Load {
								generation,
								key: load.track.id.0,
								info,
								start_at: load.start_at,
								autoplay: self.player.view().playing,
							});
						}
					}
					Err(error) => {
						let policy = playback_error_policy(&error, self.attempt);
						self.on_playback_error(policy, cx);
					}
				}
			}
			Response::Prefetch {
				track,
				quality,
				result,
			} => {
				if self.prefetch_requested == Some(track)
					&& let Ok(info) = result
				{
					self.audio.send(AudioCmd::Prefetch {
						key: track.0,
						info: info.clone(),
					});
					self.prefetched = Some((track, quality, info));
				}
			}
			Response::PlaylistEdited { result, .. } => {
				if result.is_err() {
					self.toast(rust_i18n::t!("toast.edit_failed"), cx);
				}
			}
			Response::Cover { uuid, size, path } => {
				self.covers.insert((uuid, size), path);
			}
			Response::PlaylistIndex(index) => self.index = index,
			Response::LoginCode(code) => self.login = LoginState::Code(code),
			Response::PkceUrl(url) => {
				self.pkce_url = Some(url.clone());
				self.login = LoginState::Pkce { url, error: None };
			}
			Response::LoggedIn { name } => {
				self.login = LoginState::LoggedIn { name };
				// A load requested before the connection was ready was dropped: retry it.
				let generation = self.generations.current();
				if self.started_generation != Some(generation)
					&& let Some(load) = self.current_load.clone()
				{
					self.request_stream(generation, &load);
				}
			}
			Response::NeedsLogin => {
				self.login = LoginState::Connecting;
				self.send(Request::StartLogin);
			}
			Response::MissingCredentials(path) => self.login = LoginState::MissingCredentials(path),
			Response::LoginFailed(error) => {
				self.login = match self.pkce_url.clone() {
					// Stay on the paste step so a wrong address can be corrected.
					Some(url) if !self.is_logged_in() => LoginState::Pkce {
						url,
						error: Some(error),
					},
					_ => LoginState::Failed(error),
				}
			}
		}
		cx.notify();
	}

	/// Sends the pasted "Oops" page address to finish the browser login.
	pub fn complete_pkce(&mut self, address: String, cx: &mut Context<Self>) {
		if let LoginState::Pkce { error, .. } = &mut self.login {
			*error = None;
		}
		self.send(Request::CompletePkce(address));
		cx.notify();
	}

	pub fn retry_connection(&mut self, cx: &mut Context<Self>) {
		self.login = LoginState::Connecting;
		self.send(Request::Reconnect);
		cx.notify();
	}

	pub fn logout(&mut self, cx: &mut Context<Self>) {
		self.send(Request::Logout);
		self.login = LoginState::Connecting;
		cx.notify();
	}

	// ---- navigation ----

	pub fn navigate(&mut self, route: Route, cx: &mut Context<Self>) {
		match &route {
			Route::Playlist(id) => {
				self.playlist_tracks
					.entry(id.clone())
					.or_insert(Remote::Loading);
				self.send(Request::Playlist(id.clone()));
			}
			Route::Album(id) => {
				if !matches!(self.albums.get(id), Some(Remote::Ready(_))) {
					self.albums.insert(*id, Remote::Loading);
					self.send(Request::Album(*id));
				}
			}
			Route::Artist(id) => {
				if !matches!(self.artists.get(id), Some(Remote::Ready(_))) {
					self.artists.insert(*id, Remote::Loading);
					self.send(Request::Artist(*id));
				}
			}
			Route::Recent => {
				for track in self.recent.tracks() {
					self.want_cover(track.album.cover.as_deref(), COVER_SMALL);
				}
			}
			Route::Search | Route::Settings => {}
		}
		if self.route.as_ref() != Some(&route) {
			for list in self.list_states.borrow().values() {
				list.scroll_to(ListOffset {
					item_ix: 0,
					offset_in_item: px(0.),
				});
			}
		}
		self.route = Some(route);
		self.mark_dirty(cx);
		cx.notify();
	}

	/// List state of a virtualized table with `count` items; reset when the count changes.
	pub fn list_state(&self, table: &'static str, count: usize) -> ListState {
		let mut states = self.list_states.borrow_mut();
		let state = states
			.entry(table)
			.or_insert_with(|| ListState::new(count, ListAlignment::Top, px(600.)));
		if state.item_count() != count {
			state.reset(count);
		}
		state.clone()
	}

	pub fn playlist(&self, id: &PlaylistId) -> Option<&Playlist> {
		self.playlists
			.ready()
			.and_then(|playlists| playlists.iter().find(|playlist| &playlist.id == id))
	}

	// ---- search and settings ----

	/// Typing in the search box: opens the search page, requests after a short pause.
	pub fn set_search_query(&mut self, query: &str, cx: &mut Context<Self>) {
		if !self.search.set_query(query) {
			cx.notify();
			return;
		}
		if self.route != Some(Route::Search) {
			self.navigate(Route::Search, cx);
		}
		let query = self.search.query().to_string();
		self.search_timer = Some(cx.spawn(async move |this, cx| {
			cx.background_executor().timer(SEARCH_DEBOUNCE).await;
			let _ = this.update(cx, |state, _| {
				if state.search.query() == query {
					state.send(Request::Search(query));
				}
			});
		}));
		cx.notify();
	}

	pub fn set_language(&mut self, language: Option<Lang>, cx: &mut Context<Self>) {
		self.settings.language = language;
		let system = i18n::system_locale();
		i18n::apply(i18n::resolve(language, system.as_deref()));
		cx.refresh_windows();
		self.mark_dirty(cx);
		cx.notify();
	}

	pub fn set_quality(&mut self, quality: AudioQuality, cx: &mut Context<Self>) {
		self.settings.quality = quality;
		self.mark_dirty(cx);
		cx.notify();
	}

	pub fn account_name(&self) -> Option<&str> {
		match &self.login {
			LoginState::LoggedIn { name } => Some(name),
			_ => None,
		}
	}

	/// Picks a sort for a playlist (same key flips the direction) and remembers it.
	pub fn set_sort(&mut self, id: PlaylistId, key: SortKey, cx: &mut Context<Self>) {
		let sort = self.sort_of(&id).select(key);
		if sort == TrackSort::default() {
			self.playlist_sorts.remove(&id);
		} else {
			self.playlist_sorts.insert(id, sort);
		}
		self.mark_dirty(cx);
		cx.notify();
	}

	pub fn sort_of(&self, id: &PlaylistId) -> TrackSort {
		self.playlist_sorts.get(id).copied().unwrap_or_default()
	}

	// ---- playlist edits ----

	pub fn add_to_playlist(
		&mut self,
		id: PlaylistId,
		title: &str,
		track: crate::model::TrackId,
		cx: &mut Context<Self>,
	) {
		self.index.add(id.clone(), track);
		self.send(Request::AddToPlaylist { id, track });
		self.toast(rust_i18n::t!("toast.added_to_playlist", name = title), cx);
	}

	pub fn create_playlist(
		&mut self,
		title: String,
		add: Option<crate::model::TrackId>,
		cx: &mut Context<Self>,
	) {
		if add.is_some() {
			self.toast(
				rust_i18n::t!("toast.added_to_playlist", name = title.clone()),
				cx,
			);
		}
		self.send(Request::CreatePlaylist { title, add });
	}

	pub fn rename_playlist(&mut self, id: PlaylistId, title: String) {
		self.send(Request::RenamePlaylist { id, title });
	}

	pub fn delete_playlist(&mut self, id: PlaylistId, cx: &mut Context<Self>) {
		self.index.remove_playlist(&id);
		if self.route == Some(Route::Playlist(id.clone())) {
			self.route = None;
		}
		self.send(Request::DeletePlaylist(id));
		cx.notify();
	}

	pub fn remove_from_playlist(&mut self, id: PlaylistId, index: usize, cx: &mut Context<Self>) {
		if let Some(Remote::Ready(tracks)) = self.playlist_tracks.get_mut(&id)
			&& index < tracks.len()
		{
			tracks.remove(index);
		}
		self.send(Request::RemoveFromPlaylist { id, index });
		cx.notify();
	}

	pub fn move_playlist_item(
		&mut self,
		id: PlaylistId,
		from: usize,
		to: usize,
		cx: &mut Context<Self>,
	) {
		if let Some(Remote::Ready(tracks)) = self.playlist_tracks.get_mut(&id)
			&& from < tracks.len()
			&& to < tracks.len()
		{
			let track = tracks.remove(from);
			tracks.insert(to, track);
		}
		self.send(Request::MovePlaylistItem { id, from, to });
		cx.notify();
	}

	// ---- covers ----

	pub fn cover(&self, uuid: Option<&str>, size: u32) -> Option<PathBuf> {
		self.covers.get(&(uuid?.to_string(), size)).cloned()
	}

	fn want_cover(&mut self, uuid: Option<&str>, size: u32) {
		let Some(uuid) = uuid else {
			return;
		};
		let key = (uuid.to_string(), size);
		if self.requested_covers.insert(key) {
			self.send(Request::Cover {
				uuid: uuid.to_string(),
				size,
			});
		}
	}

	fn want_track_covers(&mut self, tracks: &[Track]) {
		for track in tracks {
			self.want_cover(track.album.cover.as_deref(), COVER_SMALL);
		}
	}

	// ---- toasts ----

	pub fn toast(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
		let text: SharedString = text.into();
		if self.toasts.iter().any(|toast| toast.text == text) {
			return;
		}
		self.next_toast += 1;
		let id = self.next_toast;
		self.toasts.push(Toast { id, text });
		self.toasts.truncate(3);
		cx.spawn(async move |this, cx| {
			cx.background_executor().timer(TOAST_DURATION).await;
			let _ = this.update(cx, |state, cx| {
				state.toasts.retain(|toast| toast.id != id);
				cx.notify();
			});
		})
		.detach();
		cx.notify();
	}

	/// Snapshot for persistence.
	pub fn snapshot(&self) -> Snapshot {
		Snapshot {
			version: Snapshot::VERSION,
			player: self.player.clone(),
			recent: self.recent.clone(),
			route: self.route.clone(),
			last_playlist: self.last_playlist.clone(),
			settings: self.settings.clone(),
			queue_panel_open: self.queue_panel_open,
			playlist_sorts: self.playlist_sorts.clone(),
		}
	}
}
