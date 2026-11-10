//! Back/forward history of played tracks.

use serde::{Deserialize, Serialize};

use crate::model::QueueItem;

/// Where the current track came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
	/// The list context; `cursor` is its position in the play order.
	Context { cursor: usize },
	/// The user queue ("Added by you").
	User,
	/// A suggestion.
	Suggestion,
	/// A single track started on its own.
	Single,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Played {
	pub item: QueueItem,
	pub source: Source,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct History {
	back: Vec<Played>,
	forward: Vec<Played>,
}

impl History {
	const LIMIT: usize = 200;

	/// Records the track being left by a normal advance; the forward stack is unchanged.
	pub fn push_back(&mut self, played: Played) {
		self.back.push(played);
		if self.back.len() > Self::LIMIT {
			self.back.remove(0);
		}
	}

	/// Moving back: `current` goes to the forward stack, the previous track is returned.
	pub fn go_back(&mut self, current: Played) -> Option<Played> {
		let previous = self.back.pop()?;
		self.forward.push(current);
		Some(previous)
	}

	/// Moving forward through tracks left by `go_back`.
	pub fn go_forward(&mut self, current: Played) -> Option<Played> {
		let next = self.forward.pop()?;
		self.push_back(current);
		Some(next)
	}

	/// The track `go_forward` would return.
	pub fn peek_forward(&self) -> Option<&Played> {
		self.forward.last()
	}

	pub fn clear_forward(&mut self) {
		self.forward.clear();
	}
}
