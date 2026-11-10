//! Dev aid: `TIDAL_RS_DEMO=1` runs the UI against canned data, without TIDAL.
//! Used to check layouts; nothing here plays audio.

use std::time::Duration;

use crate::backend::{Backend, Request, Response, handle};
use crate::model::{
	Album, AlbumId, AlbumRef, Artist, ArtistId, ArtistPage, ArtistRef, AudioQuality, Playlist,
	PlaylistId, SearchResults, Track, TrackId,
};
use crate::playlist_index::PlaylistIndex;
use crate::tidal::manifest::StreamInfo;
use crate::tidal::{TidalApi, TidalError, TidalResult};

const TITLES: [(&str, &str, &str); 12] = [
	("Midnight City", "M83", "Hurry Up, We're Dreaming"),
	("Nightcall", "Kavinsky", "OutRun"),
	("Resonance", "HOME", "Odyssey"),
	("Tech Noir", "Gunship", "Gunship"),
	("Genesis", "Justice", "Cross"),
	("Turbo Killer", "Carpenter Brut", "Trilogy"),
	("A Real Hero", "College", "Northern Council"),
	("Sunset", "The Midnight", "Endless Summer"),
	("Roller Mobster", "Carpenter Brut", "Trilogy"),
	("Odd Look", "Kavinsky", "OutRun"),
	("Wait", "M83", "Hurry Up, We're Dreaming"),
	("Dreams", "The Midnight", "Days of Thunder"),
];

fn track(id: u64) -> Track {
	let (title, artist, album) = TITLES[(id as usize) % TITLES.len()];
	Track {
		id: TrackId(id),
		title: title.into(),
		artists: vec![ArtistRef {
			id: ArtistId(id % 5 + 1),
			name: artist.into(),
		}],
		album: AlbumRef {
			id: AlbumId(id % 7 + 1),
			title: album.into(),
			cover: None,
		},
		duration: Duration::from_secs(180 + id * 7 % 120),
		available: true,
		added: Some("2026-03-12T10:00:00.000+0000".into()),
	}
}

fn tracks(from: u64, count: u64) -> Vec<Track> {
	(from..from + count).map(track).collect()
}

fn playlist(id: &str, title: &str, count: u32, owned: bool) -> Playlist {
	Playlist {
		id: PlaylistId(id.into()),
		title: title.into(),
		cover: None,
		track_count: count,
		duration: Duration::from_secs(u64::from(count) * 230),
		owned,
	}
}

fn album(id: u64) -> Album {
	Album {
		id: AlbumId(id),
		title: TITLES[id as usize % TITLES.len()].2.into(),
		artists: vec![ArtistRef {
			id: ArtistId(1),
			name: "Kavinsky".into(),
		}],
		cover: None,
		year: Some(2013),
		track_count: 12,
		duration: Duration::from_secs(2700),
	}
}

struct DemoApi;

impl TidalApi for DemoApi {
	async fn playlists(&self) -> TidalResult<Vec<Playlist>> {
		Ok(vec![
			playlist("night-drive", "Night Drive", 42, true),
			playlist("focus", "Focus", 118, true),
			playlist("running", "Running 170 BPM", 36, true),
			playlist("sunday", "Dimanche matin", 24, true),
			playlist("synthwave", "Synthwave Essentials", 77, false),
		])
	}
	async fn playlist_tracks(&self, id: &PlaylistId) -> TidalResult<Vec<Track>> {
		let count = match id.0.as_str() {
			"focus" => 118,
			"synthwave" => 77,
			_ => 42,
		};
		Ok(tracks(1, count))
	}
	async fn album(&self, id: AlbumId) -> TidalResult<(Album, Vec<Track>)> {
		Ok((album(id.0), tracks(id.0 * 10, 12)))
	}
	async fn artist(&self, id: ArtistId) -> TidalResult<ArtistPage> {
		Ok(ArtistPage {
			artist: Artist {
				id,
				name: TITLES[id.0 as usize].1.into(),
				picture: None,
			},
			top_tracks: tracks(100, 10),
			albums: (1..5).map(album).collect(),
			singles: (5..8).map(album).collect(),
		})
	}
	async fn search(&self, _query: &str) -> TidalResult<SearchResults> {
		Ok(SearchResults {
			tracks: tracks(200, 8),
			artists: (1..5)
				.map(|id| Artist {
					id: ArtistId(id),
					name: TITLES[id as usize].1.into(),
					picture: None,
				})
				.collect(),
			albums: (1..6).map(album).collect(),
		})
	}
	async fn track_radio(&self, id: TrackId) -> TidalResult<Vec<Track>> {
		Ok(tracks(id.0 * 3 + 300, 10))
	}
	async fn stream(&self, _id: TrackId, _quality: AudioQuality) -> TidalResult<StreamInfo> {
		Err(TidalError::Network("demo mode has no audio".into()))
	}
	async fn create_playlist(&self, title: &str) -> TidalResult<Playlist> {
		Ok(playlist("new", title, 0, true))
	}
	async fn rename_playlist(&self, _id: &PlaylistId, _title: &str) -> TidalResult<()> {
		Ok(())
	}
	async fn delete_playlist(&self, _id: &PlaylistId) -> TidalResult<()> {
		Ok(())
	}
	async fn add_to_playlist(&self, _id: &PlaylistId, _track: TrackId) -> TidalResult<()> {
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

pub fn spawn_demo_backend() -> Backend {
	let runtime = tokio::runtime::Builder::new_multi_thread()
		.enable_all()
		.build()
		.expect("demo runtime");
	let handle_rt = runtime.handle().clone();
	let (request_tx, request_rx) = async_channel::unbounded::<Request>();
	let (response_tx, response_rx) = async_channel::unbounded();
	std::thread::spawn(move || {
		runtime.block_on(async move {
			let api = DemoApi;
			let _ = response_tx.try_send(Response::LoggedIn {
				name: "Demo".into(),
			});
			for response in handle(&api, Request::Playlists).await {
				let _ = response_tx.try_send(response);
			}
			let mut index = PlaylistIndex::default();
			index.set(PlaylistId("night-drive".into()), &tracks(1, 42));
			let _ = response_tx.try_send(Response::PlaylistIndex(index));
			while let Ok(request) = request_rx.recv().await {
				for response in handle(&api, request).await {
					let _ = response_tx.try_send(response);
				}
			}
		});
	});
	Backend {
		requests: request_tx,
		responses: response_rx,
		runtime: handle_rt,
	}
}

/// Dev aid: `TIDAL_RS_DEMO_SCENARIO` opens a page (album, artist, search, settings,
/// recent) or fills the queue (queue), one second after start, for window captures.
pub fn run_scenario(state: gpui_kit::Entity<crate::ui::state::AppState>, cx: &mut gpui_kit::App) {
	use crate::model::{ContextRef, Origin, QueueItem, Route};
	use crate::player::Command;

	let Some(scenario) = std::env::var("TIDAL_RS_DEMO_SCENARIO").ok() else {
		return;
	};
	cx.spawn(async move |cx| {
		cx.background_executor().timer(Duration::from_secs(1)).await;
		state.update(cx, |state, cx| match scenario.as_str() {
			"album" => state.navigate(Route::Album(AlbumId(2)), cx),
			"artist" => state.navigate(Route::Artist(ArtistId(2)), cx),
			"search" => state.set_search_query("night", cx),
			"settings" => state.navigate(Route::Settings, cx),
			"recent" => {
				for id in 40..46 {
					state.recent.record(&QueueItem {
						track: track(id),
						origin: Origin::Other,
					});
				}
				state.navigate(Route::Recent, cx);
			}
			"queue" => {
				state.play_list(
					ContextRef::Playlist {
						id: PlaylistId("night-drive".into()),
						title: "Night Drive".into(),
					},
					tracks(1, 42),
					Some(1),
					None,
					cx,
				);
				for id in [50, 51] {
					state.dispatch(
						Command::AddToQueue(QueueItem {
							track: track(id),
							origin: Origin::Other,
						}),
						cx,
					);
				}
				state.dispatch(Command::Progress(Duration::from_secs(98)), cx);
				state.dispatch(Command::SetShuffle(true), cx);
			}
			_ => {}
		});
	})
	.detach();
}
