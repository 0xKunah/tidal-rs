//! Track lyrics: synced (LRC) and plain text, plus a per-track cache.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use crate::model::TrackId;

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Lyrics {
	/// Timed lines, sorted by time.
	pub synced: Vec<(Duration, String)>,
	pub plain: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LyricsState {
	/// Requested (or about to be) and not answered yet.
	Loading,
	Missing,
	Ready(Lyrics),
}

/// Parses `[mm:ss]` / `[mm:ss.xx]` timestamps. Metadata tags and garbage lines are ignored.
pub fn parse_lrc(text: &str) -> Vec<(Duration, String)> {
	let mut lines = Vec::new();
	for raw in text.lines() {
		let mut rest = raw.trim_end_matches('\r');
		let mut times = Vec::new();
		while let Some(after) = rest.strip_prefix('[') {
			let Some(end) = after.find(']') else { break };
			match parse_timestamp(&after[..end]) {
				Some(time) => times.push(time),
				None => {
					times.clear();
					break;
				}
			}
			rest = &after[end + 1..];
		}
		let text = rest.trim();
		for time in times {
			lines.push((time, text.to_string()));
		}
	}
	lines.sort_by_key(|(time, _)| *time);
	lines
}

fn parse_timestamp(tag: &str) -> Option<Duration> {
	let (minutes, seconds) = tag.split_once(':')?;
	let minutes: u64 = minutes.parse().ok()?;
	let (whole, fraction) = match seconds.split_once('.') {
		Some((whole, fraction)) => (whole, fraction),
		None => (seconds, ""),
	};
	if whole.is_empty() || !whole.bytes().all(|b| b.is_ascii_digit()) {
		return None;
	}
	if fraction.len() > 3 || !fraction.bytes().all(|b| b.is_ascii_digit()) {
		return None;
	}
	let whole: u64 = whole.parse().ok()?;
	let millis: u64 = if fraction.is_empty() {
		0
	} else {
		format!("{fraction:0<3}").parse().ok()?
	};
	Some(Duration::from_millis(
		(minutes * 60 + whole) * 1000 + millis,
	))
}

impl Lyrics {
	/// None when there is neither text nor timed lines.
	pub fn from_parts(plain: Option<String>, lrc: Option<&str>) -> Option<Lyrics> {
		let synced = lrc.map(parse_lrc).unwrap_or_default();
		let plain = plain.filter(|text| !text.trim().is_empty());
		if synced.is_empty() && plain.is_none() {
			return None;
		}
		Some(Lyrics { synced, plain })
	}

	/// Last timed line whose time is <= `position`.
	pub fn active_index(&self, position: Duration) -> Option<usize> {
		self.synced
			.partition_point(|(time, _)| *time <= position)
			.checked_sub(1)
	}
}

/// Never seeks past the end of the track.
pub fn clamp_seek(position: Duration, duration: Duration) -> Duration {
	position.min(duration)
}

/// Lyrics by track id, so a late answer never lands on another track.
#[derive(Default)]
pub struct LyricsCache {
	entries: HashMap<TrackId, Option<Lyrics>>,
	requested: HashSet<TrackId>,
	failures: HashMap<TrackId, u8>,
}

/// A failed request (network, token) is asked again this many times.
const MAX_RETRIES: u8 = 2;

impl LyricsCache {
	pub fn new() -> Self {
		Self::default()
	}

	/// True the first time a track is asked about.
	pub fn should_request(&mut self, id: TrackId) -> bool {
		self.requested.insert(id)
	}

	pub fn set(&mut self, id: TrackId, lyrics: Option<Lyrics>) {
		self.requested.insert(id);
		self.entries.insert(id, lyrics);
	}

	/// Records a failed request. True when it should be sent again; after
	/// `MAX_RETRIES` the track is given up on (`Missing`).
	pub fn failed(&mut self, id: TrackId) -> bool {
		let count = self.failures.entry(id).or_default();
		if *count < MAX_RETRIES {
			*count += 1;
			return true;
		}
		self.set(id, None);
		false
	}

	pub fn get(&self, id: TrackId) -> LyricsState {
		match self.entries.get(&id) {
			None => LyricsState::Loading,
			Some(None) => LyricsState::Missing,
			Some(Some(lyrics)) => LyricsState::Ready(lyrics.clone()),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn ms(value: u64) -> Duration {
		Duration::from_millis(value)
	}

	#[test]
	fn parse_lrc_reads_minutes_seconds_centiseconds() {
		assert_eq!(
			parse_lrc("[01:02.50] hello"),
			vec![(ms(62_500), "hello".to_string())]
		);
	}

	#[test]
	fn parse_lrc_expands_multiple_timestamps() {
		assert_eq!(
			parse_lrc("[00:10.00][01:30.00] chorus"),
			vec![
				(ms(10_000), "chorus".to_string()),
				(ms(90_000), "chorus".to_string())
			]
		);
	}

	#[test]
	fn parse_lrc_ignores_metadata_and_garbage() {
		let text = "[ar:X]\n[offset:0]\nno tag\n[xx:yy] bad\n[00:01.00] ok";
		assert_eq!(parse_lrc(text), vec![(ms(1000), "ok".to_string())]);
	}

	#[test]
	fn parse_lrc_handles_crlf_and_empty_text() {
		assert_eq!(
			parse_lrc("[00:01.00] a\r\n[00:05.00]\r\n"),
			vec![(ms(1000), "a".to_string()), (ms(5000), String::new())]
		);
	}

	#[test]
	fn active_index_before_first_is_none_then_tracks_position() {
		let lyrics = Lyrics {
			synced: vec![(ms(1000), "a".into()), (ms(5000), "b".into())],
			plain: None,
		};
		assert_eq!(lyrics.active_index(ms(0)), None);
		assert_eq!(lyrics.active_index(ms(1000)), Some(0));
		assert_eq!(lyrics.active_index(ms(4900)), Some(0));
		assert_eq!(lyrics.active_index(ms(5000)), Some(1));
		assert_eq!(lyrics.active_index(ms(999_000)), Some(1));
	}

	#[test]
	fn from_parts_none_when_empty() {
		assert_eq!(Lyrics::from_parts(None, None), None);
		assert_eq!(Lyrics::from_parts(Some("  ".into()), Some("")), None);
		let lyrics = Lyrics::from_parts(Some("x".into()), None).unwrap();
		assert_eq!(lyrics.plain.as_deref(), Some("x"));
		assert!(lyrics.synced.is_empty());
	}

	#[test]
	fn clamp_seek_never_exceeds_duration() {
		assert_eq!(
			clamp_seek(Duration::from_secs(500), Duration::from_secs(200)),
			Duration::from_secs(200)
		);
		assert_eq!(
			clamp_seek(Duration::from_secs(10), Duration::from_secs(200)),
			Duration::from_secs(10)
		);
	}

	#[test]
	fn failed_requests_are_retried_a_few_times_before_giving_up() {
		let mut cache = LyricsCache::new();
		assert!(cache.should_request(TrackId(1)));
		assert!(cache.failed(TrackId(1)));
		assert!(cache.failed(TrackId(1)));
		assert_eq!(cache.get(TrackId(1)), LyricsState::Loading);
		assert!(!cache.failed(TrackId(1)));
		assert_eq!(cache.get(TrackId(1)), LyricsState::Missing);
	}

	#[test]
	fn cache_requests_each_track_once_and_keys_by_id() {
		let mut cache = LyricsCache::new();
		assert!(cache.should_request(TrackId(1)));
		assert!(!cache.should_request(TrackId(1)));
		assert_eq!(cache.get(TrackId(1)), LyricsState::Loading);
		cache.set(TrackId(1), None);
		assert_eq!(cache.get(TrackId(1)), LyricsState::Missing);
		assert_eq!(cache.get(TrackId(2)), LyricsState::Loading);
	}
}
