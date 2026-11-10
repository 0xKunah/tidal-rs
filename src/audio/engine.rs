//! The audio thread: owns the output device and the rodio player.
//! Every command and event carries a generation number so stale loads are ignored.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use crate::audio::buffer::{StreamReader, stream_buffer};
use crate::audio::download::{download, is_remuxed};
use crate::tidal::manifest::{Codec, StreamInfo, normalization_gain, quality_badge};

const TICK: Duration = Duration::from_millis(100);

#[derive(Clone, Debug, PartialEq)]
pub enum AudioCmd {
	Load {
		generation: u64,
		/// Identifies the track; a matching prefetch is reused.
		key: u64,
		info: StreamInfo,
		start_at: Duration,
		autoplay: bool,
	},
	/// Starts downloading the track expected to play next.
	Prefetch {
		key: u64,
		info: StreamInfo,
	},
	Play,
	Pause,
	Seek(Duration),
	Volume(f32),
	Stop,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AudioError {
	Network(String),
	Decode(String),
	Device(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum AudioEvent {
	Started { generation: u64, badge: String },
	Position { generation: u64, position: Duration },
	Ended { generation: u64 },
	Failed { generation: u64, error: AudioError },
}

enum Message {
	Cmd(AudioCmd),
	Ready {
		generation: u64,
		result: Result<rodio::Decoder<StreamReader>, AudioError>,
		start_at: Duration,
		autoplay: bool,
		badge: String,
		gain: f32,
	},
}

#[derive(Clone)]
pub struct AudioHandle {
	tx: mpsc::Sender<Message>,
}

impl AudioHandle {
	pub fn send(&self, cmd: AudioCmd) {
		let _ = self.tx.send(Message::Cmd(cmd));
	}
}

/// Builds a decoder over a stream that may still be downloading.
/// Remuxed DASH FLAC starts at once but is not seekable until rebuilt with
/// `seekable_decoder` once the download has finished.
pub fn build_decoder(
	reader: StreamReader,
	codec: Option<&Codec>,
	remuxed: bool,
) -> Result<rodio::Decoder<StreamReader>, AudioError> {
	if remuxed {
		return rodio::Decoder::builder()
			.with_data(reader)
			.with_seekable(false)
			.with_hint("flac")
			.build()
			.map_err(|error| AudioError::Decode(error.to_string()));
	}
	let len = reader
		.wait_len()
		.ok_or_else(|| AudioError::Network("download failed before starting".into()))?;
	let mut builder = rodio::Decoder::builder()
		.with_data(reader)
		.with_byte_len(len)
		.with_seekable(true);
	match codec {
		Some(Codec::Flac) => builder = builder.with_hint("flac"),
		Some(Codec::Aac) => builder = builder.with_hint("m4a"),
		_ => {}
	}
	builder
		.build()
		.map_err(|error| AudioError::Decode(error.to_string()))
}

/// Seekable FLAC decoder over a fully downloaded remuxed stream.
fn seekable_decoder(
	reader: StreamReader,
	len: u64,
) -> Result<rodio::Decoder<StreamReader>, AudioError> {
	rodio::Decoder::builder()
		.with_data(reader)
		.with_byte_len(len)
		.with_seekable(true)
		.with_hint("flac")
		.build()
		.map_err(|error| AudioError::Decode(error.to_string()))
}

pub fn spawn_audio(
	events: async_channel::Sender<AudioEvent>,
	rt: tokio::runtime::Handle,
) -> AudioHandle {
	let (tx, rx) = mpsc::channel();
	let handle = AudioHandle { tx: tx.clone() };
	std::thread::Builder::new()
		.name("audio".into())
		.spawn(move || AudioThread::new(events, rt, tx).run(rx))
		.expect("failed to start the audio thread");
	handle
}

struct AudioThread {
	events: async_channel::Sender<AudioEvent>,
	rt: tokio::runtime::Handle,
	tx: mpsc::Sender<Message>,
	http: reqwest::Client,
	sink: Option<rodio::MixerDeviceSink>,
	player: Option<rodio::Player>,
	generation: u64,
	cancel: Arc<AtomicBool>,
	loaded: bool,
	ended: bool,
	volume: f32,
	/// Loudness normalization of the current track (linear).
	gain: f32,
	/// Handle on the current track's bytes, to rebuild its decoder.
	source: Option<StreamReader>,
	/// The current decoder can seek (false for remuxed DASH FLAC until rebuilt).
	seekable: bool,
	/// Seek waiting for the download to finish.
	pending_seek: Option<Duration>,
	/// Download of the track expected to play next.
	prefetched: Option<Prefetched>,
}

struct Prefetched {
	key: u64,
	reader: StreamReader,
	cancel: Arc<AtomicBool>,
}

impl AudioThread {
	fn new(
		events: async_channel::Sender<AudioEvent>,
		rt: tokio::runtime::Handle,
		tx: mpsc::Sender<Message>,
	) -> Self {
		Self {
			events,
			rt,
			tx,
			http: reqwest::Client::new(),
			sink: None,
			player: None,
			generation: 0,
			cancel: Arc::new(AtomicBool::new(false)),
			loaded: false,
			ended: false,
			volume: 1.0,
			gain: 1.0,
			source: None,
			seekable: true,
			pending_seek: None,
			prefetched: None,
		}
	}

	fn emit(&self, event: AudioEvent) {
		let _ = self.events.send_blocking(event);
	}

	/// Opens the default output device on first use.
	fn player(&mut self) -> Result<&rodio::Player, AudioError> {
		if self.player.is_none() {
			let mut sink = rodio::DeviceSinkBuilder::open_default_sink()
				.map_err(|error| AudioError::Device(error.to_string()))?;
			sink.log_on_drop(false);
			let player = rodio::Player::connect_new(sink.mixer());
			player.set_volume(self.volume * self.gain);
			self.sink = Some(sink);
			self.player = Some(player);
		}
		Ok(self.player.as_ref().expect("player just created"))
	}

	fn run(mut self, rx: mpsc::Receiver<Message>) {
		loop {
			match rx.recv_timeout(TICK) {
				Ok(Message::Cmd(cmd)) => self.command(cmd),
				Ok(Message::Ready {
					generation,
					result,
					start_at,
					autoplay,
					badge,
					gain,
				}) => self.ready(generation, result, start_at, autoplay, badge, gain),
				Err(RecvTimeoutError::Timeout) => self.tick(),
				Err(RecvTimeoutError::Disconnected) => return,
			}
		}
	}

	fn command(&mut self, cmd: AudioCmd) {
		match cmd {
			AudioCmd::Load {
				generation,
				key,
				info,
				start_at,
				autoplay,
			} => self.load(generation, key, info, start_at, autoplay),
			AudioCmd::Prefetch { key, info } => {
				if self.prefetched.as_ref().is_some_and(|p| p.key == key) {
					return;
				}
				if let Some(old) = self.prefetched.take() {
					old.cancel.store(true, Ordering::Relaxed);
				}
				let (reader, cancel) = self.start_download(&info);
				self.prefetched = Some(Prefetched {
					key,
					reader,
					cancel,
				});
			}
			AudioCmd::Play => {
				if let Some(player) = &self.player {
					player.play();
				}
			}
			AudioCmd::Pause => {
				if let Some(player) = &self.player {
					player.pause();
				}
			}
			AudioCmd::Seek(position) => {
				if !self.loaded {
					return;
				}
				if self.seekable {
					if let Some(player) = &self.player {
						let _ = player.try_seek(position);
					}
				} else {
					self.pending_seek = Some(position);
					self.apply_pending_seek();
				}
			}
			AudioCmd::Volume(volume) => {
				self.volume = volume;
				if let Some(player) = &self.player {
					player.set_volume(volume * self.gain);
				}
			}
			AudioCmd::Stop => self.stop(),
		}
	}

	fn stop(&mut self) {
		self.cancel.store(true, Ordering::Relaxed);
		if let Some(player) = &self.player {
			player.clear();
		}
		self.loaded = false;
	}

	/// Starts downloading `info` into a new buffer.
	fn start_download(&self, info: &StreamInfo) -> (StreamReader, Arc<AtomicBool>) {
		let cancel = Arc::new(AtomicBool::new(false));
		let (writer, reader) = stream_buffer(None);
		let client = self.http.clone();
		let source = info.source.clone();
		let codec = info.codec.clone();
		let flag = cancel.clone();
		self.rt
			.spawn(async move { download(&client, &source, &codec, writer, flag).await });
		(reader, cancel)
	}

	fn load(
		&mut self,
		generation: u64,
		key: u64,
		info: StreamInfo,
		start_at: Duration,
		autoplay: bool,
	) {
		self.stop();
		self.generation = generation;
		self.ended = false;
		let (reader, cancel) = match self.prefetched.take() {
			Some(prefetched) if prefetched.key == key => (prefetched.reader, prefetched.cancel),
			other => {
				self.prefetched = other;
				self.start_download(&info)
			}
		};
		self.cancel = cancel;
		let remuxed = is_remuxed(&info.source, &info.codec);
		// A fully prefetched remuxed track can be seekable from the start.
		let finished = reader.finished_len().filter(|_| remuxed);
		self.source = Some(reader.reopen());
		self.pending_seek = None;
		self.seekable = !remuxed || finished.is_some();
		let tx = self.tx.clone();
		let badge = quality_badge(&info);
		let gain = normalization_gain(info.replay_gain, info.peak);
		let codec = info.codec.clone();
		std::thread::spawn(move || {
			let result = match finished {
				Some(len) => seekable_decoder(reader, len),
				None => build_decoder(reader, Some(&codec), remuxed),
			};
			let _ = tx.send(Message::Ready {
				generation,
				result,
				start_at,
				autoplay,
				badge,
				gain,
			});
		});
	}

	fn ready(
		&mut self,
		generation: u64,
		result: Result<rodio::Decoder<StreamReader>, AudioError>,
		start_at: Duration,
		autoplay: bool,
		badge: String,
		gain: f32,
	) {
		if generation != self.generation {
			return;
		}
		let decoder = match result {
			Ok(decoder) => decoder,
			Err(error) => {
				self.emit(AudioEvent::Failed { generation, error });
				return;
			}
		};
		let volume = self.volume * gain;
		let seekable = self.seekable;
		let player = match self.player() {
			Ok(player) => player,
			Err(error) => {
				self.emit(AudioEvent::Failed { generation, error });
				return;
			}
		};
		player.pause();
		player.set_volume(volume);
		player.append(decoder);
		if autoplay {
			player.play();
		}
		if !start_at.is_zero() {
			if seekable {
				let _ = player.try_seek(start_at);
			} else {
				self.pending_seek = Some(start_at);
			}
		}
		self.gain = gain;
		self.loaded = true;
		self.emit(AudioEvent::Started { generation, badge });
	}

	/// Rebuilds a seekable decoder once the download has finished, then seeks.
	fn apply_pending_seek(&mut self) {
		let Some(position) = self.pending_seek else {
			return;
		};
		let Some(source) = &self.source else {
			return;
		};
		let Some(len) = source.finished_len() else {
			return;
		};
		let Ok(decoder) = seekable_decoder(source.reopen(), len) else {
			self.pending_seek = None;
			return;
		};
		let volume = self.volume * self.gain;
		let Some(player) = &self.player else {
			return;
		};
		let playing = !player.is_paused();
		player.clear();
		player.set_volume(volume);
		player.append(decoder);
		let _ = player.try_seek(position);
		if playing {
			player.play();
		}
		self.seekable = true;
		self.pending_seek = None;
	}

	fn tick(&mut self) {
		self.apply_pending_seek();
		let Some(player) = &self.player else {
			return;
		};
		if !self.loaded {
			return;
		}
		let generation = self.generation;
		if player.empty() {
			if !self.ended {
				self.ended = true;
				self.loaded = false;
				self.emit(AudioEvent::Ended { generation });
			}
			return;
		}
		if !player.is_paused() {
			let position = player.get_pos();
			self.emit(AudioEvent::Position {
				generation,
				position,
			});
		}
	}
}

#[cfg(test)]
mod tests {
	use rodio::Source;

	use super::*;
	use crate::audio::buffer::stream_buffer;

	/// 16-bit PCM mono WAV with `samples` samples of silence.
	fn wav_bytes(samples: u32, rate: u32) -> Vec<u8> {
		let data_len = samples * 2;
		let mut bytes = Vec::new();
		bytes.extend_from_slice(b"RIFF");
		bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
		bytes.extend_from_slice(b"WAVEfmt ");
		bytes.extend_from_slice(&16u32.to_le_bytes());
		bytes.extend_from_slice(&1u16.to_le_bytes());
		bytes.extend_from_slice(&1u16.to_le_bytes());
		bytes.extend_from_slice(&rate.to_le_bytes());
		bytes.extend_from_slice(&(rate * 2).to_le_bytes());
		bytes.extend_from_slice(&2u16.to_le_bytes());
		bytes.extend_from_slice(&16u16.to_le_bytes());
		bytes.extend_from_slice(b"data");
		bytes.extend_from_slice(&data_len.to_le_bytes());
		bytes.resize(bytes.len() + data_len as usize, 0);
		bytes
	}

	#[test]
	fn decoder_reads_from_stream_reader() {
		let wav = wav_bytes(44_100, 44_100);
		let (writer, reader) = stream_buffer(Some(wav.len() as u64));
		writer.push(&wav);
		writer.finish();
		let decoder = build_decoder(reader, None, false).unwrap();
		assert_eq!(decoder.sample_rate().get(), 44_100);
		assert_eq!(decoder.count(), 44_100);
	}
}
