//! Play order over a list (playlist, album, artist top tracks, recently played).

use serde::{Deserialize, Serialize};

use crate::model::{ContextRef, Track};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ListContext {
	pub kind: ContextRef,
	tracks: Vec<Track>,
	/// Permutation of indices into `tracks`; play order.
	order: Vec<usize>,
	/// Index into `order` of the current track.
	cursor: usize,
}

impl ListContext {
	/// `start` indexes `tracks`. With shuffle, the start track plays first and the rest is shuffled.
	pub fn new(
		kind: ContextRef,
		tracks: Vec<Track>,
		start: usize,
		shuffle: bool,
		rng: &mut fastrand::Rng,
	) -> Self {
		let start = start.min(tracks.len().saturating_sub(1));
		let mut context = Self {
			kind,
			order: (0..tracks.len()).collect(),
			tracks,
			cursor: start,
		};
		if shuffle {
			context.set_shuffle(true, rng);
		}
		context
	}

	pub fn tracks(&self) -> &[Track] {
		&self.tracks
	}

	pub fn current(&self) -> &Track {
		&self.tracks[self.order[self.cursor]]
	}

	fn next_cursor(&self) -> Option<usize> {
		(self.cursor + 1..self.order.len()).find(|&c| self.tracks[self.order[c]].available)
	}

	/// The track `advance` would move to, when it is known in advance
	/// (a shuffled wrap reshuffles, so it is not).
	pub fn peek_next(&self, wrap: bool, shuffle: bool) -> Option<&Track> {
		if let Some(next) = self.next_cursor() {
			return Some(&self.tracks[self.order[next]]);
		}
		if !wrap || shuffle {
			return None;
		}
		self.order
			.iter()
			.map(|&ix| &self.tracks[ix])
			.find(|track| track.available)
	}

	pub fn has_next(&self, wrap: bool) -> bool {
		self.next_cursor().is_some() || (wrap && self.tracks.iter().any(|t| t.available))
	}

	/// Moves to the next available track. At the end: `None` unless `wrap`,
	/// which restarts the list (reshuffled when `shuffle`).
	pub fn advance(
		&mut self,
		wrap: bool,
		shuffle: bool,
		rng: &mut fastrand::Rng,
	) -> Option<&Track> {
		if let Some(next) = self.next_cursor() {
			self.cursor = next;
			return Some(self.current());
		}
		if !wrap {
			return None;
		}
		if shuffle {
			rng.shuffle(&mut self.order);
		}
		let first = (0..self.order.len()).find(|&c| self.tracks[self.order[c]].available)?;
		self.cursor = first;
		Some(self.current())
	}

	/// On: current first, every other track shuffled after it.
	/// Off: list order, positioned on the current track.
	pub fn set_shuffle(&mut self, on: bool, rng: &mut fastrand::Rng) {
		if self.tracks.is_empty() {
			return;
		}
		let current = self.order[self.cursor];
		if on {
			let mut rest: Vec<usize> = (0..self.tracks.len()).filter(|&i| i != current).collect();
			rng.shuffle(&mut rest);
			self.order = std::iter::once(current).chain(rest).collect();
			self.cursor = 0;
		} else {
			self.order = (0..self.tracks.len()).collect();
			self.cursor = current;
		}
	}

	fn upcoming_cursors(&self) -> Vec<usize> {
		(self.cursor + 1..self.order.len())
			.filter(|&c| self.tracks[self.order[c]].available)
			.collect()
	}

	pub fn upcoming(&self) -> Vec<&Track> {
		self.upcoming_cursors()
			.into_iter()
			.map(|c| &self.tracks[self.order[c]])
			.collect()
	}

	/// `from` and `to` index `upcoming()`.
	pub fn move_upcoming(&mut self, from: usize, to: usize) {
		let cursors = self.upcoming_cursors();
		if from >= cursors.len() || to >= cursors.len() || from == to {
			return;
		}
		let mut values: Vec<usize> = cursors.iter().map(|&c| self.order[c]).collect();
		let moved = values.remove(from);
		values.insert(to, moved);
		for (cursor, value) in cursors.into_iter().zip(values) {
			self.order[cursor] = value;
		}
	}

	pub fn cursor(&self) -> usize {
		self.cursor
	}

	pub fn set_cursor(&mut self, cursor: usize) {
		if cursor < self.order.len() {
			self.cursor = cursor;
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::model::TrackId;
	use crate::test_support::{ids, list, rng};

	#[test]
	fn list_order_without_shuffle() {
		let c = ListContext::new(
			ContextRef::Recent,
			list(&[1, 2, 3, 4, 5]),
			2,
			false,
			&mut rng(),
		);
		assert_eq!(c.current().id, TrackId(3));
		assert_eq!(ids(&c.upcoming()), vec![4, 5]);
	}

	#[test]
	fn shuffle_keeps_current_and_permutes_the_rest() {
		let c = ListContext::new(
			ContextRef::Recent,
			list(&[1, 2, 3, 4, 5]),
			2,
			true,
			&mut rng(),
		);
		assert_eq!(c.current().id, TrackId(3));
		let mut up = ids(&c.upcoming());
		up.sort();
		assert_eq!(up, vec![1, 2, 4, 5]);
	}

	#[test]
	fn advance_skips_unavailable() {
		let mut tracks = list(&[1, 2, 3]);
		tracks[1].available = false;
		let mut c = ListContext::new(ContextRef::Recent, tracks, 0, false, &mut rng());
		assert_eq!(c.advance(false, false, &mut rng()).unwrap().id, TrackId(3));
	}

	#[test]
	fn advance_at_end_is_none_without_wrap_and_wraps_with_it() {
		let mut c = ListContext::new(ContextRef::Recent, list(&[1, 2]), 1, false, &mut rng());
		assert!(!c.has_next(false));
		assert!(c.advance(false, false, &mut rng()).is_none());
		let mut c = ListContext::new(ContextRef::Recent, list(&[1, 2]), 1, false, &mut rng());
		assert_eq!(c.advance(true, false, &mut rng()).unwrap().id, TrackId(1));
	}

	#[test]
	fn shuffle_off_resumes_list_order_after_current() {
		let mut c = ListContext::new(
			ContextRef::Recent,
			list(&[1, 2, 3, 4, 5]),
			0,
			true,
			&mut rng(),
		);
		c.advance(false, true, &mut rng());
		let cur = c.current().id.0;
		c.set_shuffle(false, &mut rng());
		assert_eq!(c.current().id.0, cur);
		assert_eq!(ids(&c.upcoming()), ((cur + 1)..=5).collect::<Vec<_>>());
	}

	#[test]
	fn move_upcoming_reorders() {
		let mut c = ListContext::new(
			ContextRef::Recent,
			list(&[1, 2, 3, 4]),
			0,
			false,
			&mut rng(),
		);
		c.move_upcoming(2, 0);
		assert_eq!(ids(&c.upcoming()), vec![4, 2, 3]);
	}
}
