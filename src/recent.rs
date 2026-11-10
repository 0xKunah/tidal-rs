//! "Recently played": tracks listened to outside the user's playlists.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::model::{Origin, QueueItem, Track};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RecentList {
	entries: VecDeque<Track>,
}

impl RecentList {
	pub const CAP: usize = 500;

	/// Records a play; plays started from a playlist are ignored.
	pub fn record(&mut self, item: &QueueItem) {
		if matches!(item.origin, Origin::Playlist(_)) {
			return;
		}
		self.entries.retain(|track| track.id != item.track.id);
		self.entries.push_front(item.track.clone());
		self.entries.truncate(Self::CAP);
	}

	/// Most recent first.
	pub fn tracks(&self) -> Vec<Track> {
		self.entries.iter().cloned().collect()
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::model::{Origin, PlaylistId, QueueItem, TrackId};
	use crate::test_support::t;

	fn other(id: u64) -> QueueItem {
		QueueItem {
			track: t(id),
			origin: Origin::Other,
		}
	}

	#[test]
	fn ignores_playlist_origin() {
		let mut r = RecentList::default();
		r.record(&QueueItem {
			track: t(1),
			origin: Origin::Playlist(PlaylistId("p".into())),
		});
		assert!(r.tracks().is_empty());
	}

	#[test]
	fn dedupes_and_moves_to_top() {
		let mut r = RecentList::default();
		for id in [1, 2, 1] {
			r.record(&other(id));
		}
		assert_eq!(
			r.tracks().iter().map(|t| t.id.0).collect::<Vec<_>>(),
			vec![1, 2]
		);
	}

	#[test]
	fn caps_at_500() {
		let mut r = RecentList::default();
		for id in 0..600 {
			r.record(&other(id));
		}
		assert_eq!(r.tracks().len(), 500);
		assert_eq!(r.tracks()[0].id, TrackId(599));
	}
}
