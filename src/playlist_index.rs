//! Which track is in which playlist, for the "already in it" marker.

use std::collections::{HashMap, HashSet};

use crate::model::{PlaylistId, Track, TrackId};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaylistIndex(HashMap<PlaylistId, HashSet<TrackId>>);

impl PlaylistIndex {
	pub fn set(&mut self, id: PlaylistId, tracks: &[Track]) {
		self.0
			.insert(id, tracks.iter().map(|track| track.id).collect());
	}

	pub fn contains(&self, id: &PlaylistId, track: TrackId) -> bool {
		self.0.get(id).is_some_and(|tracks| tracks.contains(&track))
	}

	pub fn add(&mut self, id: PlaylistId, track: TrackId) {
		self.0.entry(id).or_default().insert(track);
	}

	pub fn remove_playlist(&mut self, id: &PlaylistId) {
		self.0.remove(id);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::model::{PlaylistId, TrackId};
	use crate::test_support::list;

	#[test]
	fn playlist_index_tracks_membership() {
		let p1 = PlaylistId("p1".into());
		let mut index = PlaylistIndex::default();
		index.set(p1.clone(), &list(&[1, 2]));
		assert!(index.contains(&p1, TrackId(2)));
		assert!(!index.contains(&p1, TrackId(3)));
		index.add(p1.clone(), TrackId(3));
		assert!(index.contains(&p1, TrackId(3)));
		index.remove_playlist(&p1);
		assert!(!index.contains(&p1, TrackId(1)));
	}
}
