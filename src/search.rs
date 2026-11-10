//! Search query and results; results for an outdated query are dropped.

use crate::model::SearchResults;

#[derive(Clone, Debug, Default)]
pub struct SearchState {
	query: String,
	results: Option<SearchResults>,
}

impl SearchState {
	/// Returns true when a request should be sent (non-empty and changed).
	pub fn set_query(&mut self, query: &str) -> bool {
		let query = query.trim();
		if query == self.query {
			return false;
		}
		self.query = query.to_string();
		if query.is_empty() {
			self.results = None;
			return false;
		}
		true
	}

	pub fn apply(&mut self, query: &str, results: SearchResults) {
		if query == self.query {
			self.results = Some(results);
		}
	}

	pub fn query(&self) -> &str {
		&self.query
	}

	pub fn results(&self) -> Option<&SearchResults> {
		self.results.as_ref()
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::model::{SearchResults, TrackId};
	use crate::test_support::list;

	fn results_with_track(id: u64) -> SearchResults {
		SearchResults {
			tracks: list(&[id]),
			..SearchResults::default()
		}
	}

	#[test]
	fn stale_search_results_are_ignored() {
		let mut s = SearchState::default();
		s.set_query("nig");
		s.set_query("nightcall");
		s.apply("nig", results_with_track(1));
		assert!(s.results().is_none());
		s.apply("nightcall", results_with_track(2));
		assert_eq!(s.results().unwrap().tracks[0].id, TrackId(2));
	}

	#[test]
	fn empty_or_unchanged_query_sends_nothing() {
		let mut s = SearchState::default();
		assert!(!s.set_query(""));
		assert!(s.set_query("a"));
		assert!(!s.set_query("a"));
		assert!(!s.set_query("  a "));
	}

	#[test]
	fn clearing_the_query_clears_results() {
		let mut s = SearchState::default();
		s.set_query("a");
		s.apply("a", results_with_track(1));
		s.set_query("");
		assert!(s.results().is_none());
		assert_eq!(s.query(), "");
	}
}
