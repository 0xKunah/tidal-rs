//! Shell-side helpers for executing player effects: load debouncing, generations,
//! and what to do when a stream fails.

use std::time::{Duration, Instant};

use crate::audio::engine::AudioError;
use crate::model::Track;
use crate::tidal::TidalError;

#[derive(Clone, Debug, PartialEq)]
pub struct PendingLoad {
	pub track: Track,
	pub start_at: Duration,
	pub autoplay: bool,
}

/// A lone request runs at once; requests arriving in a burst (within `delay` of the
/// previous one) are merged and only the last runs, so rapid skips never download
/// the tracks skipped over.
#[derive(Debug)]
pub struct LoadDebouncer {
	delay: Duration,
	pending: Option<(Instant, PendingLoad)>,
	last_fired: Option<Instant>,
}

impl LoadDebouncer {
	pub const DELAY: Duration = Duration::from_millis(200);

	pub fn new(delay: Duration) -> Self {
		Self {
			delay,
			pending: None,
			last_fired: None,
		}
	}

	pub fn request(&mut self, load: PendingLoad, now: Instant) {
		let idle = self.pending.is_none()
			&& self
				.last_fired
				.is_none_or(|fired| now.saturating_duration_since(fired) >= self.delay);
		let deadline = if idle { now } else { now + self.delay };
		self.pending = Some((deadline, load));
	}

	pub fn take_due(&mut self, now: Instant) -> Option<PendingLoad> {
		match &self.pending {
			Some((deadline, _)) if *deadline <= now => {
				self.last_fired = Some(now);
				self.pending.take().map(|(_, load)| load)
			}
			_ => None,
		}
	}

	pub fn deadline(&self) -> Option<Instant> {
		self.pending.as_ref().map(|(deadline, _)| *deadline)
	}
}

/// Tags each load; events and responses from older loads are dropped.
#[derive(Debug, Default)]
pub struct Generations(u64);

impl Generations {
	pub fn advance(&mut self) -> u64 {
		self.0 += 1;
		self.0
	}

	pub fn current(&self) -> u64 {
		self.0
	}

	pub fn is_current(&self, generation: u64) -> bool {
		self.0 == generation
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorPolicy {
	/// The track cannot be played: move on.
	Skip,
	/// Transient: stay on the track and try again.
	Retry { after: Duration },
}

fn retry(attempt: u32) -> ErrorPolicy {
	let after = match attempt {
		0 => Duration::from_secs(2),
		1 => Duration::from_secs(5),
		_ => Duration::from_secs(10),
	};
	ErrorPolicy::Retry { after }
}

/// Only errors that say the track itself is unplayable skip it; everything else
/// (network, server, session) retries, so an outage never skips through the queue.
pub fn playback_error_policy(error: &TidalError, attempt: u32) -> ErrorPolicy {
	match error {
		TidalError::NotFound | TidalError::Unavailable | TidalError::Decode(_) => ErrorPolicy::Skip,
		TidalError::Api { status, .. } if (400..500).contains(status) && *status != 429 => {
			ErrorPolicy::Skip
		}
		_ => retry(attempt),
	}
}

pub fn audio_error_policy(error: &AudioError, attempt: u32) -> ErrorPolicy {
	match error {
		AudioError::Decode(_) => ErrorPolicy::Skip,
		AudioError::Network(_) | AudioError::Device(_) => retry(attempt),
	}
}

#[cfg(test)]
mod tests {
	use std::time::{Duration, Instant};

	use super::*;
	use crate::audio::engine::AudioError;
	use crate::test_support::t;
	use crate::tidal::TidalError;

	fn load(id: u64) -> PendingLoad {
		PendingLoad {
			track: t(id),
			start_at: Duration::ZERO,
			autoplay: true,
		}
	}

	#[test]
	fn debouncer_only_the_last_request_fires() {
		let t0 = Instant::now();
		let mut debouncer = LoadDebouncer::new(LoadDebouncer::DELAY);
		debouncer.request(load(1), t0);
		debouncer.request(load(2), t0 + Duration::from_millis(50));
		assert_eq!(debouncer.take_due(t0 + Duration::from_millis(100)), None);
		assert_eq!(
			debouncer.take_due(t0 + Duration::from_millis(260)),
			Some(load(2))
		);
		assert_eq!(debouncer.take_due(t0 + Duration::from_millis(500)), None);
	}

	#[test]
	fn debouncer_fires_at_once_when_idle() {
		let t0 = Instant::now();
		let mut debouncer = LoadDebouncer::new(LoadDebouncer::DELAY);
		debouncer.request(load(1), t0);
		assert_eq!(debouncer.deadline(), Some(t0));
		assert_eq!(debouncer.take_due(t0), Some(load(1)));
	}

	#[test]
	fn requests_right_after_a_fire_are_debounced() {
		let t0 = Instant::now();
		let mut debouncer = LoadDebouncer::new(LoadDebouncer::DELAY);
		debouncer.request(load(1), t0);
		assert_eq!(debouncer.take_due(t0), Some(load(1)));
		debouncer.request(load(2), t0 + Duration::from_millis(50));
		debouncer.request(load(3), t0 + Duration::from_millis(80));
		assert_eq!(debouncer.take_due(t0 + Duration::from_millis(200)), None);
		assert_eq!(
			debouncer.take_due(t0 + Duration::from_millis(280)),
			Some(load(3))
		);
	}

	#[test]
	fn generations_reject_stale() {
		let mut generations = Generations::default();
		let first = generations.advance();
		let second = generations.advance();
		assert!(!generations.is_current(first));
		assert!(generations.is_current(second));
	}

	#[test]
	fn network_error_retries_unavailable_skips() {
		let network = TidalError::Network("down".into());
		assert_eq!(
			playback_error_policy(&network, 0),
			ErrorPolicy::Retry {
				after: Duration::from_secs(2)
			}
		);
		assert_eq!(
			playback_error_policy(&network, 1),
			ErrorPolicy::Retry {
				after: Duration::from_secs(5)
			}
		);
		assert_eq!(
			playback_error_policy(&network, 5),
			ErrorPolicy::Retry {
				after: Duration::from_secs(10)
			}
		);
		assert_eq!(
			playback_error_policy(&TidalError::Unavailable, 0),
			ErrorPolicy::Skip
		);
		assert_eq!(
			playback_error_policy(&TidalError::NotFound, 0),
			ErrorPolicy::Skip
		);
		assert!(matches!(
			playback_error_policy(&TidalError::Unauthorized, 0),
			ErrorPolicy::Retry { .. }
		));
	}

	#[test]
	fn audio_network_errors_retry_decode_errors_skip() {
		assert!(matches!(
			audio_error_policy(&AudioError::Network("x".into()), 0),
			ErrorPolicy::Retry { .. }
		));
		assert_eq!(
			audio_error_policy(&AudioError::Decode("x".into()), 0),
			ErrorPolicy::Skip
		);
	}
}
