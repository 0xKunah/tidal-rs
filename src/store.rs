//! Session snapshot persisted across restarts.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::i18n::Lang;
use crate::model::{AudioQuality, Playlist, PlaylistId, Route};
use crate::player::Player;
use crate::recent::RecentList;
use crate::sort::TrackSort;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
	/// None follows the Windows language.
	pub language: Option<Lang>,
	pub quality: AudioQuality,
}

impl Default for Settings {
	fn default() -> Self {
		Self {
			language: None,
			quality: AudioQuality::HiRes,
		}
	}
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
	pub version: u32,
	pub player: Player,
	pub recent: RecentList,
	pub route: Option<Route>,
	pub last_playlist: Option<PlaylistId>,
	pub settings: Settings,
	pub queue_panel_open: bool,
	/// Display order chosen per playlist.
	#[serde(default)]
	pub playlist_sorts: HashMap<PlaylistId, TrackSort>,
}

impl Snapshot {
	pub const VERSION: u32 = 1;
}

impl Default for Snapshot {
	fn default() -> Self {
		Self {
			version: Self::VERSION,
			player: Player::new(fastrand::u64(..)),
			recent: RecentList::default(),
			route: None,
			last_playlist: None,
			settings: Settings::default(),
			queue_panel_open: true,
			playlist_sorts: HashMap::new(),
		}
	}
}

/// Missing, corrupt or other-version files give the default snapshot.
pub fn load(path: &Path) -> Snapshot {
	let Ok(text) = std::fs::read_to_string(path) else {
		return Snapshot::default();
	};
	let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
		return Snapshot::default();
	};
	if value.get("version").and_then(|v| v.as_u64()) != Some(u64::from(Snapshot::VERSION)) {
		return Snapshot::default();
	}
	serde_json::from_value(value).unwrap_or_default()
}

/// Writes `path.tmp` then renames it over `path`.
pub fn save(path: &Path, snap: &Snapshot) -> io::Result<()> {
	if let Some(dir) = path.parent() {
		std::fs::create_dir_all(dir)?;
	}
	let tmp = path.with_extension("json.tmp");
	let json = serde_json::to_vec(snap).map_err(io::Error::other)?;
	std::fs::write(&tmp, json)?;
	std::fs::rename(&tmp, path)
}

/// The app data folder: `tidal-rs` under the roaming AppData folder.
pub fn app_dir() -> PathBuf {
	directories::BaseDirs::new()
		.map(|dirs| dirs.config_dir().to_path_buf())
		.unwrap_or_else(std::env::temp_dir)
		.join("tidal-rs")
}

pub fn state_path() -> PathBuf {
	app_dir().join("state.json")
}

/// Start page: the saved content page, else the last played playlist, else the first playlist.
pub fn initial_route(
	saved: Option<&Route>,
	last_playlist: Option<&PlaylistId>,
	playlists: &[Playlist],
) -> Option<Route> {
	let exists = |id: &PlaylistId| playlists.iter().any(|playlist| &playlist.id == id);
	match saved {
		Some(Route::Playlist(id)) if exists(id) => return Some(Route::Playlist(id.clone())),
		Some(route @ (Route::Album(_) | Route::Artist(_) | Route::Recent)) => {
			return Some(route.clone());
		}
		_ => {}
	}
	if let Some(id) = last_playlist
		&& exists(id)
	{
		return Some(Route::Playlist(id.clone()));
	}
	playlists
		.first()
		.map(|playlist| Route::Playlist(playlist.id.clone()))
}

#[cfg(test)]
mod tests {
	use std::time::Duration;

	use super::*;
	use crate::model::{AlbumId, ContextRef, Origin, Playlist, PlaylistId, QueueItem, TrackId};
	use crate::player::{Command, Effect, Player};
	use crate::test_support::{list, t};

	fn temp_path(name: &str) -> PathBuf {
		let dir = std::env::temp_dir().join(format!("tidal-rs-test-{name}-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		dir.join("state.json")
	}

	fn assert_default(snap: &Snapshot) {
		assert_eq!(snap.version, Snapshot::VERSION);
		assert!(snap.player.view().now.is_none());
		assert!(snap.recent.tracks().is_empty());
		assert_eq!(snap.route, None);
		assert_eq!(snap.settings, Settings::default());
		assert!(snap.queue_panel_open);
	}

	fn playlist(id: &str) -> Playlist {
		Playlist {
			id: PlaylistId(id.into()),
			title: id.into(),
			cover: None,
			track_count: 1,
			duration: Duration::from_secs(60),
			owned: true,
		}
	}

	fn playing_snapshot() -> Snapshot {
		let mut player = Player::new(1);
		player.handle(Command::PlayList {
			kind: ContextRef::Recent,
			tracks: list(&[1, 2, 3]),
			start: Some(1),
			shuffle: None,
		});
		player.handle(Command::Progress(Duration::from_secs(42)));
		let mut snap = Snapshot {
			player,
			route: Some(Route::Album(AlbumId(5))),
			..Snapshot::default()
		};
		snap.recent.record(&QueueItem {
			track: t(9),
			origin: Origin::Other,
		});
		snap
	}

	#[test]
	fn snapshot_roundtrip() {
		let path = temp_path("roundtrip");
		let snap = playing_snapshot();
		save(&path, &snap).unwrap();
		let loaded = load(&path);
		let (a, b) = (snap.player.view(), loaded.player.view());
		assert_eq!(a.now, b.now);
		assert_eq!(a.position, b.position);
		assert_eq!(a.upcoming, b.upcoming);
		assert_eq!(loaded.recent.tracks(), snap.recent.tracks());
		assert_eq!(loaded.route, snap.route);
	}

	#[test]
	fn playlist_sorts_survive_a_restart() {
		use crate::sort::{SortKey, TrackSort};
		let path = temp_path("sorts");
		let mut snap = Snapshot::default();
		let sort = TrackSort {
			key: SortKey::Added,
			descending: true,
		};
		snap.playlist_sorts.insert(PlaylistId("a".into()), sort);
		save(&path, &snap).unwrap();
		assert_eq!(
			load(&path).playlist_sorts.get(&PlaylistId("a".into())),
			Some(&sort)
		);
	}

	#[test]
	fn missing_corrupt_or_other_version_gives_default() {
		let path = temp_path("fallback");
		assert_default(&load(&path));
		std::fs::write(&path, "not json").unwrap();
		assert_default(&load(&path));
		let mut value = serde_json::to_value(playing_snapshot()).unwrap();
		value["version"] = serde_json::json!(999);
		std::fs::write(&path, value.to_string()).unwrap();
		assert_default(&load(&path));
	}

	#[test]
	fn save_leaves_no_tmp_file() {
		let path = temp_path("atomic");
		save(&path, &Snapshot::default()).unwrap();
		assert!(path.exists());
		assert!(!path.with_extension("json.tmp").exists());
	}

	#[test]
	fn restored_player_is_paused_and_resumes_paused() {
		let path = temp_path("paused");
		save(&path, &playing_snapshot()).unwrap();
		let snap = load(&path);
		assert!(!snap.player.view().playing);
		assert!(snap.player.resume_effects().iter().any(|e| matches!(e,
			Effect::Load { track, start_at, autoplay: false }
				if track.id == TrackId(2) && *start_at == Duration::from_secs(42))));
	}

	#[test]
	fn initial_route_rules() {
		let a = playlist("a");
		let b = playlist("b");
		let all = vec![a.clone(), b.clone()];
		assert_eq!(
			initial_route(Some(&Route::Album(AlbumId(5))), None, &all),
			Some(Route::Album(AlbumId(5)))
		);
		assert_eq!(
			initial_route(
				Some(&Route::Playlist(PlaylistId("gone".into()))),
				Some(&b.id),
				&all
			),
			Some(Route::Playlist(b.id.clone()))
		);
		assert_eq!(
			initial_route(Some(&Route::Search), None, &all),
			Some(Route::Playlist(a.id.clone()))
		);
		assert_eq!(initial_route(None, None, &[]), None);
	}
}
