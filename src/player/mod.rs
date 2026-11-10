//! Pure playback state machine: commands in, effects out. No I/O, no GPUI.

pub mod context;
pub mod history;

use std::collections::{HashSet, VecDeque};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::model::{ContextRef, Origin, QueueItem, Track, TrackId};
use context::ListContext;
use history::{History, Played, Source};

/// Previous restarts the track instead of going back once it played longer than this.
const RESTART_THRESHOLD: Duration = Duration::from_secs(3);
/// A track counts as played (recently played list) after this much listening.
const PLAYED_THRESHOLD: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Repeat {
	#[default]
	Off,
	All,
	One,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
	/// `start` None = first track (random with shuffle); `shuffle` Some sets the global shuffle first.
	PlayList {
		kind: ContextRef,
		tracks: Vec<Track>,
		start: Option<usize>,
		shuffle: Option<bool>,
	},
	PlaySingle(QueueItem),
	AddToQueue(QueueItem),
	RemoveFromQueue(usize),
	MoveInQueue {
		from: usize,
		to: usize,
	},
	ClearQueue,
	MoveUpcoming {
		from: usize,
		to: usize,
	},
	Next,
	Previous,
	TogglePlay,
	Seek(Duration),
	SetShuffle(bool),
	CycleRepeat,
	SetVolume(f32),
	/// Mutes, or restores the volume from before muting.
	ToggleMute,
	Progress(Duration),
	TrackEnded,
	TrackUnavailable,
	SuggestionsLoaded {
		seed: TrackId,
		tracks: Vec<Track>,
	},
}

#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
	Load {
		track: Track,
		start_at: Duration,
		autoplay: bool,
	},
	Pause,
	Resume,
	Seek(Duration),
	Volume(f32),
	FetchSuggestions {
		seed: Track,
	},
	Played(QueueItem),
	Stop,
}

#[derive(Clone, Debug, PartialEq)]
pub enum NextLabel {
	Context { kind: ContextRef, shuffled: bool },
	Suggestions { based_on: String },
	Nothing,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlayerView {
	pub now: Option<QueueItem>,
	pub playing: bool,
	pub position: Duration,
	pub user_queue: Vec<QueueItem>,
	pub next_label: NextLabel,
	pub upcoming: Vec<Track>,
	pub shuffle: bool,
	pub repeat: Repeat,
	pub volume: f32,
	pub context: Option<ContextRef>,
}

fn new_rng() -> fastrand::Rng {
	fastrand::Rng::new()
}

/// Suggestion candidates from TIDAL's track radio.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Suggestions {
	/// Last requested seed; only results for it are accepted.
	seed: Option<Track>,
	candidates: Vec<Track>,
	/// Title of the seed the current candidates came from.
	based_on: String,
	/// Next presses received while no candidate was available.
	#[serde(skip)]
	pending_skips: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Player {
	now: Option<Played>,
	user_queue: VecDeque<QueueItem>,
	context: Option<ListContext>,
	history: History,
	shuffle: bool,
	repeat: Repeat,
	volume: f32,
	position: Duration,
	/// Always false after a restore: the app restarts paused.
	#[serde(skip)]
	playing: bool,
	#[serde(skip, default = "new_rng")]
	rng: fastrand::Rng,
	suggestions: Suggestions,
	/// Tracks played this session; never suggested again.
	#[serde(skip)]
	played_ids: HashSet<TrackId>,
	/// Whether the current play already emitted `Effect::Played`.
	counted: bool,
	/// Volume to restore when unmuting; Some while muted.
	#[serde(default)]
	unmuted_volume: Option<f32>,
}

impl Player {
	pub fn new(seed: u64) -> Self {
		Self {
			now: None,
			user_queue: VecDeque::new(),
			context: None,
			history: History::default(),
			shuffle: false,
			repeat: Repeat::Off,
			volume: 1.0,
			position: Duration::ZERO,
			playing: false,
			rng: fastrand::Rng::with_seed(seed),
			suggestions: Suggestions::default(),
			played_ids: HashSet::new(),
			counted: false,
			unmuted_volume: None,
		}
	}

	pub fn handle(&mut self, cmd: Command) -> Vec<Effect> {
		match cmd {
			Command::PlayList {
				kind,
				tracks,
				start,
				shuffle,
			} => self.play_list(kind, tracks, start, shuffle),
			Command::PlaySingle(item) => {
				self.context = None;
				self.history.clear_forward();
				self.reset_suggestions();
				self.start(Played {
					item,
					source: Source::Single,
				})
			}
			Command::AddToQueue(item) => {
				self.user_queue.push_back(item);
				vec![]
			}
			Command::RemoveFromQueue(index) => {
				self.user_queue.remove(index);
				vec![]
			}
			Command::MoveInQueue { from, to } => {
				if from < self.user_queue.len()
					&& to < self.user_queue.len()
					&& let Some(item) = self.user_queue.remove(from)
				{
					self.user_queue.insert(to, item);
				}
				vec![]
			}
			Command::ClearQueue => {
				self.user_queue.clear();
				vec![]
			}
			Command::MoveUpcoming { from, to } => {
				if self.list_continues() {
					if let Some(context) = &mut self.context {
						context.move_upcoming(from, to);
					}
				} else {
					let candidates = &mut self.suggestions.candidates;
					if from < candidates.len() && to < candidates.len() {
						let track = candidates.remove(from);
						candidates.insert(to, track);
					}
				}
				vec![]
			}
			Command::Next => self.next(true),
			Command::Previous => self.previous(),
			Command::TogglePlay => {
				if self.now.is_none() {
					return vec![];
				}
				self.playing = !self.playing;
				vec![if self.playing {
					Effect::Resume
				} else {
					Effect::Pause
				}]
			}
			Command::Seek(position) => {
				self.position = position;
				vec![Effect::Seek(position)]
			}
			Command::SetShuffle(on) => {
				self.shuffle = on;
				if let Some(context) = &mut self.context {
					context.set_shuffle(on, &mut self.rng);
					if let Some(now) = &mut self.now
						&& matches!(now.source, Source::Context { .. })
					{
						now.source = Source::Context {
							cursor: context.cursor(),
						};
					}
				}
				vec![]
			}
			Command::CycleRepeat => {
				self.repeat = match self.repeat {
					Repeat::Off => Repeat::All,
					Repeat::All => Repeat::One,
					Repeat::One => Repeat::Off,
				};
				vec![]
			}
			Command::SetVolume(volume) => {
				self.volume = volume.clamp(0.0, 1.0);
				self.unmuted_volume = None;
				vec![Effect::Volume(self.volume)]
			}
			Command::ToggleMute => {
				match self.unmuted_volume.take() {
					Some(previous) => self.volume = previous,
					None => {
						self.unmuted_volume = Some(self.volume);
						self.volume = 0.0;
					}
				}
				vec![Effect::Volume(self.volume)]
			}
			Command::Progress(position) => {
				self.position = position;
				if position >= PLAYED_THRESHOLD {
					self.count_played()
				} else {
					vec![]
				}
			}
			Command::TrackEnded => {
				let mut effects = self.count_played();
				if self.repeat == Repeat::One
					&& let Some(now) = self.now.clone()
				{
					effects.extend(self.start_without_history(now));
					return effects;
				}
				effects.extend(self.next(true));
				effects
			}
			Command::TrackUnavailable => self.next(false),
			Command::SuggestionsLoaded { seed, tracks } => self.suggestions_loaded(seed, tracks),
		}
	}

	fn play_list(
		&mut self,
		kind: ContextRef,
		tracks: Vec<Track>,
		start: Option<usize>,
		shuffle: Option<bool>,
	) -> Vec<Effect> {
		if let Some(on) = shuffle {
			self.shuffle = on;
		}
		let playable: Vec<usize> = (0..tracks.len()).filter(|&i| tracks[i].available).collect();
		if playable.is_empty() {
			return vec![];
		}
		let start = match start {
			Some(index) => index,
			None if self.shuffle => playable[self.rng.usize(..playable.len())],
			None => playable[0],
		};
		let origin = kind.origin();
		let mut context = ListContext::new(kind, tracks, start, self.shuffle, &mut self.rng);
		if !context.current().available {
			context.advance(false, self.shuffle, &mut self.rng);
		}
		let played = Played {
			item: QueueItem {
				track: context.current().clone(),
				origin,
			},
			source: Source::Context {
				cursor: context.cursor(),
			},
		};
		self.context = Some(context);
		self.history.clear_forward();
		self.reset_suggestions();
		self.start(played)
	}

	fn reset_suggestions(&mut self) {
		self.suggestions = Suggestions::default();
	}

	/// Emits `Played` once per play.
	fn count_played(&mut self) -> Vec<Effect> {
		match &self.now {
			Some(now) if !self.counted => {
				self.counted = true;
				vec![Effect::Played(now.item.clone())]
			}
			_ => vec![],
		}
	}

	/// True when the list context still has a track after the current one.
	fn list_continues(&self) -> bool {
		let wrap = self.repeat == Repeat::All;
		self.context
			.as_ref()
			.is_some_and(|context| context.has_next(wrap))
	}

	fn eligible_candidates(&self) -> impl Iterator<Item = &Track> {
		self.suggestions
			.candidates
			.iter()
			.filter(|track| track.available && !self.played_ids.contains(&track.id))
	}

	fn suggestions_loaded(&mut self, seed: TrackId, mut tracks: Vec<Track>) -> Vec<Effect> {
		let Some(expected) = &self.suggestions.seed else {
			return vec![];
		};
		if expected.id != seed {
			return vec![];
		}
		if self.shuffle {
			self.rng.shuffle(&mut tracks);
		}
		self.suggestions.based_on = expected.title.clone();
		self.suggestions.candidates = tracks;
		let mut effects = vec![];
		while self.suggestions.pending_skips > 0 {
			let step = self.next_suggestion();
			if step.is_empty() {
				break;
			}
			self.suggestions.pending_skips -= 1;
			effects.extend(step);
		}
		effects
	}

	/// Starts the first eligible candidate, or records a pending skip when there is none.
	fn next_suggestion(&mut self) -> Vec<Effect> {
		let next = self.eligible_candidates().next().cloned();
		match next {
			Some(track) => {
				self.suggestions
					.candidates
					.retain(|candidate| candidate.id != track.id);
				self.context = None;
				self.start(Played {
					item: QueueItem {
						track,
						origin: Origin::Other,
					},
					source: Source::Suggestion,
				})
			}
			None => {
				if self.suggestions.seed.is_some() {
					self.suggestions.pending_skips += 1;
				}
				vec![]
			}
		}
	}

	/// Makes `played` current, recording the previous current track in history.
	fn start(&mut self, played: Played) -> Vec<Effect> {
		if let Some(previous) = self.now.take() {
			self.history.push_back(previous);
		}
		self.start_without_history(played)
	}

	fn start_without_history(&mut self, played: Played) -> Vec<Effect> {
		if let (Source::Context { cursor }, Some(context)) = (played.source, &mut self.context) {
			context.set_cursor(cursor);
		}
		let track = played.item.track.clone();
		let reseed = matches!(played.source, Source::Single | Source::Suggestion);
		self.played_ids.insert(track.id);
		self.now = Some(played);
		self.position = Duration::ZERO;
		self.playing = true;
		self.counted = false;
		let mut effects = vec![Effect::Load {
			track: track.clone(),
			start_at: Duration::ZERO,
			autoplay: true,
		}];
		if reseed || (self.user_queue.is_empty() && !self.list_continues()) {
			self.suggestions.seed = Some(track.clone());
			effects.push(Effect::FetchSuggestions { seed: track });
		}
		effects
	}

	/// `record` false drops the current track from history (it could not be played).
	fn next(&mut self, record: bool) -> Vec<Effect> {
		if !record {
			self.now = None;
		}
		if let Some(current) = self.now.clone()
			&& let Some(forward) = self.history.go_forward(current)
		{
			self.now = None;
			return self.start_without_history(forward);
		}
		if let Some(item) = self.user_queue.pop_front() {
			return self.start(Played {
				item,
				source: Source::User,
			});
		}
		let wrap = self.repeat == Repeat::All;
		if let Some(context) = &mut self.context
			&& let Some(track) = context.advance(wrap, self.shuffle, &mut self.rng).cloned()
		{
			let played = Played {
				item: QueueItem {
					track,
					origin: context.kind.origin(),
				},
				source: Source::Context {
					cursor: context.cursor(),
				},
			};
			return self.start(played);
		}
		self.next_suggestion()
	}

	fn previous(&mut self) -> Vec<Effect> {
		let Some(current) = self.now.clone() else {
			return vec![];
		};
		if self.position > RESTART_THRESHOLD {
			self.position = Duration::ZERO;
			return vec![Effect::Seek(Duration::ZERO)];
		}
		match self.history.go_back(current) {
			Some(previous) => self.start_without_history(previous),
			None => {
				self.position = Duration::ZERO;
				vec![Effect::Seek(Duration::ZERO)]
			}
		}
	}

	/// The track that will play when the current one ends, if already known
	/// (used to fetch it in advance). None with repeat one: the same track replays.
	pub fn predicted_next(&self) -> Option<Track> {
		if self.repeat == Repeat::One {
			return None;
		}
		if let Some(forward) = self.history.peek_forward() {
			return Some(forward.item.track.clone());
		}
		if let Some(item) = self.user_queue.front() {
			return Some(item.track.clone());
		}
		if let Some(context) = &self.context
			&& let Some(track) = context.peek_next(self.repeat == Repeat::All, self.shuffle)
		{
			return Some(track.clone());
		}
		self.eligible_candidates().next().cloned()
	}

	pub fn view(&self) -> PlayerView {
		let candidates: Vec<Track> = self.eligible_candidates().cloned().collect();
		let (next_label, upcoming) = match &self.context {
			Some(context) if self.list_continues() || candidates.is_empty() => (
				NextLabel::Context {
					kind: context.kind.clone(),
					shuffled: self.shuffle,
				},
				context.upcoming().into_iter().cloned().collect(),
			),
			_ if !candidates.is_empty() => (
				NextLabel::Suggestions {
					based_on: self.suggestions.based_on.clone(),
				},
				candidates,
			),
			_ => (NextLabel::Nothing, vec![]),
		};
		PlayerView {
			now: self.now.as_ref().map(|played| played.item.clone()),
			playing: self.playing,
			position: self.position,
			user_queue: self.user_queue.iter().cloned().collect(),
			next_label,
			upcoming,
			shuffle: self.shuffle,
			repeat: self.repeat,
			volume: self.volume,
			context: self.context.as_ref().map(|context| context.kind.clone()),
		}
	}

	/// After a restore: set the volume and load the current track paused at the saved position.
	pub fn resume_effects(&self) -> Vec<Effect> {
		match &self.now {
			Some(played) => vec![
				Effect::Volume(self.volume),
				Effect::Load {
					track: played.item.track.clone(),
					start_at: self.position,
					autoplay: false,
				},
			],
			None => vec![],
		}
	}
}

#[cfg(test)]
mod tests {
	use std::time::Duration;

	use super::*;
	use crate::model::{ContextRef, Origin, QueueItem, TrackId};
	use crate::test_support::{list, t};

	fn loads(fx: &[Effect]) -> Vec<u64> {
		fx.iter()
			.filter_map(|e| match e {
				Effect::Load { track, .. } => Some(track.id.0),
				_ => None,
			})
			.collect()
	}

	fn now(p: &Player) -> u64 {
		p.view().now.expect("something is playing").track.id.0
	}

	fn play(p: &mut Player, ids: &[u64], start: usize) -> Vec<Effect> {
		p.handle(Command::PlayList {
			kind: ContextRef::Recent,
			tracks: list(ids),
			start: Some(start),
			shuffle: None,
		})
	}

	fn other(id: u64) -> QueueItem {
		QueueItem {
			track: t(id),
			origin: Origin::Other,
		}
	}

	#[test]
	fn play_list_loads_start_track_and_autoplays() {
		let mut p = Player::new(1);
		let fx = p.handle(Command::PlayList {
			kind: ContextRef::Recent,
			tracks: list(&[1, 2, 3]),
			start: Some(1),
			shuffle: None,
		});
		assert!(
			matches!(&fx[0], Effect::Load { track, autoplay: true, start_at }
			if track.id == TrackId(2) && *start_at == Duration::ZERO)
		);
	}

	#[test]
	fn add_to_queue_is_fifo_and_before_context() {
		let mut p = Player::new(1);
		play(&mut p, &[1, 2, 3], 0);
		p.handle(Command::AddToQueue(other(10)));
		p.handle(Command::AddToQueue(other(11)));
		let mut seen = vec![];
		for _ in 0..3 {
			p.handle(Command::Next);
			seen.push(now(&p));
		}
		assert_eq!(seen, vec![10, 11, 2]);
	}

	#[test]
	fn five_rapid_nexts_land_five_tracks_further() {
		let mut p = Player::new(1);
		play(&mut p, &(1..=10).collect::<Vec<_>>(), 0);
		let mut all = vec![];
		for _ in 0..5 {
			all.extend(loads(&p.handle(Command::Next)));
		}
		assert_eq!(all, vec![2, 3, 4, 5, 6]);
		assert_eq!(now(&p), 6);
	}

	#[test]
	fn previous_after_three_seconds_restarts() {
		let mut p = Player::new(1);
		play(&mut p, &[1, 2], 1);
		p.handle(Command::Progress(Duration::from_secs(4)));
		assert_eq!(
			p.handle(Command::Previous),
			vec![Effect::Seek(Duration::ZERO)]
		);
		assert_eq!(now(&p), 2);
	}

	#[test]
	fn previous_goes_back_then_next_walks_forward() {
		let mut p = Player::new(1);
		play(&mut p, &[1, 2, 3, 4], 0);
		p.handle(Command::Next);
		p.handle(Command::Next);
		p.handle(Command::Progress(Duration::from_secs(1)));
		p.handle(Command::Previous);
		assert_eq!(now(&p), 2);
		p.handle(Command::Next);
		assert_eq!(now(&p), 3);
		p.handle(Command::Next);
		assert_eq!(now(&p), 4);
	}

	#[test]
	fn repeat_one_replays_current_on_end() {
		let mut p = Player::new(1);
		play(&mut p, &[1, 2], 0);
		p.handle(Command::CycleRepeat);
		p.handle(Command::CycleRepeat);
		assert_eq!(p.view().repeat, Repeat::One);
		assert_eq!(loads(&p.handle(Command::TrackEnded)), vec![1]);
	}

	#[test]
	fn repeat_all_wraps_the_list() {
		let mut p = Player::new(1);
		play(&mut p, &[1, 2], 1);
		p.handle(Command::CycleRepeat);
		assert_eq!(loads(&p.handle(Command::TrackEnded)), vec![1]);
	}

	#[test]
	fn toggle_play_pauses_and_resumes() {
		let mut p = Player::new(1);
		play(&mut p, &[1], 0);
		assert_eq!(p.handle(Command::TogglePlay), vec![Effect::Pause]);
		assert_eq!(p.handle(Command::TogglePlay), vec![Effect::Resume]);
	}

	#[test]
	fn unavailable_current_is_skipped() {
		let mut p = Player::new(1);
		play(&mut p, &[1, 2], 0);
		assert_eq!(loads(&p.handle(Command::TrackUnavailable)), vec![2]);
	}

	#[test]
	fn shuffle_button_sets_shuffle_and_plays_a_list_track() {
		let mut p = Player::new(1);
		p.handle(Command::PlayList {
			kind: ContextRef::Recent,
			tracks: list(&[1, 2, 3, 4, 5]),
			start: None,
			shuffle: Some(true),
		});
		let v = p.view();
		assert!(v.shuffle && (1..=5).contains(&now(&p)) && v.upcoming.len() == 4);
		assert_eq!(
			v.next_label,
			NextLabel::Context {
				kind: ContextRef::Recent,
				shuffled: true
			}
		);
	}

	fn played(fx: &[Effect]) -> bool {
		fx.iter().any(|e| matches!(e, Effect::Played(_)))
	}

	fn upcoming_ids(p: &Player) -> Vec<u64> {
		p.view().upcoming.iter().map(|t| t.id.0).collect()
	}

	#[test]
	fn last_list_track_prefetches_and_end_continues_with_suggestions() {
		let mut p = Player::new(1);
		let fx = p.handle(Command::PlayList {
			kind: ContextRef::Recent,
			tracks: list(&[1, 2]),
			start: Some(1),
			shuffle: None,
		});
		assert!(
			fx.iter()
				.any(|e| matches!(e, Effect::FetchSuggestions { seed } if seed.id == TrackId(2)))
		);
		p.handle(Command::SuggestionsLoaded {
			seed: TrackId(2),
			tracks: list(&[20, 21]),
		});
		let v = p.view();
		assert_eq!(
			v.next_label,
			NextLabel::Suggestions {
				based_on: "T2".into()
			}
		);
		assert_eq!(upcoming_ids(&p), vec![20, 21]);
		assert_eq!(loads(&p.handle(Command::TrackEnded)), vec![20]);
	}

	#[test]
	fn play_single_prefetches_and_continues_when_shuffle_off() {
		let mut p = Player::new(1);
		let fx = p.handle(Command::PlaySingle(other(1)));
		assert!(
			fx.iter()
				.any(|e| matches!(e, Effect::FetchSuggestions { seed } if seed.id == TrackId(1)))
		);
		p.handle(Command::SuggestionsLoaded {
			seed: TrackId(1),
			tracks: list(&[1, 30]),
		});
		assert_eq!(loads(&p.handle(Command::TrackEnded)), vec![30]);
	}

	#[test]
	fn shuffle_on_picks_among_candidates() {
		let mut p = Player::new(1);
		p.handle(Command::SetShuffle(true));
		p.handle(Command::PlaySingle(other(1)));
		p.handle(Command::SuggestionsLoaded {
			seed: TrackId(1),
			tracks: list(&[20, 21, 22, 23]),
		});
		let next = loads(&p.handle(Command::Next))[0];
		assert!([20, 21, 22, 23].contains(&next));
	}

	#[test]
	fn next_before_suggestions_arrive_is_kept() {
		let mut p = Player::new(1);
		p.handle(Command::PlaySingle(other(1)));
		assert!(loads(&p.handle(Command::Next)).is_empty());
		assert!(loads(&p.handle(Command::Next)).is_empty());
		p.handle(Command::SuggestionsLoaded {
			seed: TrackId(1),
			tracks: list(&[20, 21, 22]),
		});
		assert_eq!(now(&p), 21);
	}

	#[test]
	fn stale_suggestions_are_ignored() {
		let mut p = Player::new(1);
		p.handle(Command::PlaySingle(other(1)));
		p.handle(Command::SuggestionsLoaded {
			seed: TrackId(1),
			tracks: list(&[20, 21, 22]),
		});
		p.handle(Command::Next);
		p.handle(Command::SuggestionsLoaded {
			seed: TrackId(1),
			tracks: list(&[90]),
		});
		assert_eq!(upcoming_ids(&p), vec![21, 22]);
	}

	#[test]
	fn played_is_emitted_once_at_thirty_seconds() {
		let mut p = Player::new(1);
		play(&mut p, &[1, 2], 0);
		assert!(!played(
			&p.handle(Command::Progress(Duration::from_secs(29)))
		));
		let fx = p.handle(Command::Progress(Duration::from_secs(30)));
		assert!(
			fx.iter()
				.any(|e| matches!(e, Effect::Played(i) if i.track.id == TrackId(1)))
		);
		assert!(!played(
			&p.handle(Command::Progress(Duration::from_secs(31)))
		));
	}

	#[test]
	fn short_track_counts_on_end() {
		let mut p = Player::new(1);
		let mut short = t(1);
		short.duration = Duration::from_secs(20);
		p.handle(Command::PlaySingle(QueueItem {
			track: short,
			origin: Origin::Other,
		}));
		assert!(played(&p.handle(Command::TrackEnded)));
	}

	#[test]
	fn mute_restores_the_previous_volume() {
		let mut p = Player::new(1);
		p.handle(Command::SetVolume(0.2));
		assert_eq!(p.handle(Command::ToggleMute), vec![Effect::Volume(0.0)]);
		assert_eq!(p.handle(Command::ToggleMute), vec![Effect::Volume(0.2)]);
	}

	#[test]
	fn moving_the_volume_while_muted_unmutes() {
		let mut p = Player::new(1);
		p.handle(Command::SetVolume(0.2));
		p.handle(Command::ToggleMute);
		p.handle(Command::SetVolume(0.5));
		assert_eq!(p.handle(Command::ToggleMute), vec![Effect::Volume(0.0)]);
		assert_eq!(p.handle(Command::ToggleMute), vec![Effect::Volume(0.5)]);
	}

	fn predicted(p: &Player) -> Option<u64> {
		p.predicted_next().map(|track| track.id.0)
	}

	#[test]
	fn predicts_the_user_queue_first() {
		let mut p = Player::new(1);
		play(&mut p, &[1, 2], 0);
		p.handle(Command::AddToQueue(other(10)));
		assert_eq!(predicted(&p), Some(10));
	}

	#[test]
	fn predicts_the_next_list_track() {
		let mut p = Player::new(1);
		play(&mut p, &[1, 2, 3], 0);
		assert_eq!(predicted(&p), Some(2));
	}

	#[test]
	fn predicts_a_suggestion_after_the_list() {
		let mut p = Player::new(1);
		play(&mut p, &[1, 2], 1);
		assert_eq!(predicted(&p), None);
		p.handle(Command::SuggestionsLoaded {
			seed: TrackId(2),
			tracks: list(&[20, 21]),
		});
		assert_eq!(predicted(&p), Some(20));
	}

	#[test]
	fn predicts_the_track_walked_back_from() {
		let mut p = Player::new(1);
		play(&mut p, &[1, 2, 3], 0);
		p.handle(Command::Next);
		p.handle(Command::Previous);
		assert_eq!(now(&p), 1);
		assert_eq!(predicted(&p), Some(2));
	}

	#[test]
	fn repeat_one_predicts_nothing_to_fetch() {
		let mut p = Player::new(1);
		play(&mut p, &[1, 2], 0);
		p.handle(Command::CycleRepeat);
		p.handle(Command::CycleRepeat);
		assert_eq!(predicted(&p), None);
	}

	#[test]
	fn user_queue_edits() {
		let mut p = Player::new(1);
		play(&mut p, &[1], 0);
		for id in [10, 11, 12] {
			p.handle(Command::AddToQueue(other(id)));
		}
		p.handle(Command::MoveInQueue { from: 2, to: 0 });
		p.handle(Command::RemoveFromQueue(1));
		assert_eq!(
			p.view()
				.user_queue
				.iter()
				.map(|i| i.track.id.0)
				.collect::<Vec<_>>(),
			vec![12, 11]
		);
		p.handle(Command::ClearQueue);
		assert!(p.view().user_queue.is_empty());
	}
}
