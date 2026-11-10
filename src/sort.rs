//! Display order of a playlist's tracks.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

use crate::model::Track;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortKey {
	/// The playlist's own order.
	#[default]
	Custom,
	Title,
	Artist,
	Album,
	Added,
	Duration,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackSort {
	pub key: SortKey,
	pub descending: bool,
}

impl TrackSort {
	/// Picking the current key flips the direction; another key starts in its natural
	/// direction (newest first for dates).
	pub fn select(self, key: SortKey) -> Self {
		if key == SortKey::Custom {
			return Self::default();
		}
		if key == self.key {
			return Self {
				key,
				descending: !self.descending,
			};
		}
		Self {
			key,
			descending: key == SortKey::Added,
		}
	}
}

fn text_order(a: &str, b: &str) -> Ordering {
	a.to_lowercase().cmp(&b.to_lowercase())
}

/// Indices into `tracks` in display order. Stable: ties keep the playlist order.
/// Tracks without a date stay last in both directions.
pub fn sorted_indices(tracks: &[Track], sort: TrackSort) -> Vec<usize> {
	let mut indices: Vec<usize> = (0..tracks.len()).collect();
	let directed = |ordering: Ordering| {
		if sort.descending {
			ordering.reverse()
		} else {
			ordering
		}
	};
	indices.sort_by(|&a, &b| {
		let (a, b) = (&tracks[a], &tracks[b]);
		match sort.key {
			SortKey::Custom => Ordering::Equal,
			SortKey::Title => directed(text_order(&a.title, &b.title)),
			SortKey::Artist => directed(text_order(&a.artist_names(), &b.artist_names())),
			SortKey::Album => directed(text_order(&a.album.title, &b.album.title)),
			SortKey::Duration => directed(a.duration.cmp(&b.duration)),
			SortKey::Added => match (&a.added, &b.added) {
				(Some(a), Some(b)) => directed(a.cmp(b)),
				(Some(_), None) => Ordering::Less,
				(None, Some(_)) => Ordering::Greater,
				(None, None) => Ordering::Equal,
			},
		}
	});
	indices
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::test_support::t;

	fn track(id: u64, title: &str, artist: &str, added: Option<&str>, seconds: u64) -> Track {
		let mut track = t(id);
		track.title = title.into();
		track.artists[0].name = artist.into();
		track.album.title = format!("Album {}", 10 - id);
		track.added = added.map(str::to_string);
		track.duration = std::time::Duration::from_secs(seconds);
		track
	}

	fn tracks() -> Vec<Track> {
		vec![
			track(1, "beta", "Zed", Some("2026-03-01T00:00:00"), 200),
			track(2, "Alpha", "Moby", None, 100),
			track(3, "gamma", "abba", Some("2026-05-01T00:00:00"), 300),
		]
	}

	fn order(sort: TrackSort) -> Vec<u64> {
		let tracks = tracks();
		sorted_indices(&tracks, sort)
			.into_iter()
			.map(|ix| tracks[ix].id.0)
			.collect()
	}

	fn by(key: SortKey, descending: bool) -> TrackSort {
		TrackSort { key, descending }
	}

	#[test]
	fn custom_keeps_the_playlist_order() {
		assert_eq!(order(TrackSort::default()), vec![1, 2, 3]);
	}

	#[test]
	fn text_sorts_ignore_case() {
		assert_eq!(order(by(SortKey::Title, false)), vec![2, 1, 3]);
		assert_eq!(order(by(SortKey::Artist, false)), vec![3, 2, 1]);
	}

	#[test]
	fn descending_reverses() {
		assert_eq!(order(by(SortKey::Duration, true)), vec![3, 1, 2]);
		assert_eq!(order(by(SortKey::Album, false)), vec![3, 2, 1]);
	}

	#[test]
	fn missing_dates_go_last_either_way() {
		assert_eq!(order(by(SortKey::Added, true)), vec![3, 1, 2]);
		assert_eq!(order(by(SortKey::Added, false)), vec![1, 3, 2]);
	}

	#[test]
	fn choosing_the_same_key_flips_and_a_new_key_resets() {
		let sort = TrackSort::default().select(SortKey::Title);
		assert_eq!(sort, by(SortKey::Title, false));
		assert_eq!(sort.select(SortKey::Title), by(SortKey::Title, true));
		assert_eq!(sort.select(SortKey::Added), by(SortKey::Added, true));
		assert_eq!(sort.select(SortKey::Custom), TrackSort::default());
	}
}
