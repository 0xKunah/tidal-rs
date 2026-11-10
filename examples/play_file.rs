//! Manual audio check: serves a local file (or 2 s of generated silence) over a local
//! mock HTTP server and plays it through the real audio engine.
//!
//! cargo run --example play_file -- [path] [volume 0.0-1.0]

use std::time::Duration;

use tidal_rs::audio::engine::{AudioCmd, AudioEvent, spawn_audio};
use tidal_rs::model::AudioQuality;
use tidal_rs::tidal::manifest::{Codec, StreamInfo, StreamSource};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn silence_wav(seconds: u32) -> Vec<u8> {
	let rate = 44_100u32;
	let data_len = rate * seconds * 2;
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

#[tokio::main]
async fn main() {
	let mut args = std::env::args().skip(1);
	let path = args.next().filter(|arg| arg != "-");
	let volume: f32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
	let (bytes, codec) = match &path {
		Some(path) => {
			let codec = if path.ends_with(".flac") {
				Codec::Flac
			} else {
				Codec::Other("file".into())
			};
			(std::fs::read(path).expect("readable file"), codec)
		}
		None => (silence_wav(2), Codec::Other("wav".into())),
	};
	let server = MockServer::start().await;
	Mock::given(method("GET"))
		.respond_with(ResponseTemplate::new(200).set_body_bytes(bytes))
		.mount(&server)
		.await;

	let (events_tx, events_rx) = async_channel::unbounded();
	let audio = spawn_audio(events_tx, tokio::runtime::Handle::current());
	audio.send(AudioCmd::Volume(volume));
	audio.send(AudioCmd::Load {
		generation: 1,
		key: 1,
		info: StreamInfo {
			source: StreamSource::Progressive {
				url: format!("{}/file", server.uri()),
			},
			codec,
			quality: AudioQuality::Lossless,
			bit_depth: Some(16),
			sample_rate: Some(44_100),
			replay_gain: None,
			peak: None,
		},
		start_at: Duration::ZERO,
		autoplay: true,
	});
	loop {
		match tokio::time::timeout(Duration::from_secs(600), events_rx.recv()).await {
			Ok(Ok(AudioEvent::Position { position, .. })) => {
				println!("position {:.2}s", position.as_secs_f32())
			}
			Ok(Ok(event @ AudioEvent::Ended { .. })) => {
				println!("{event:?}");
				return;
			}
			Ok(Ok(event @ AudioEvent::Failed { .. })) => {
				println!("{event:?}");
				std::process::exit(1);
			}
			Ok(Ok(event)) => println!("{event:?}"),
			_ => {
				println!("timeout");
				std::process::exit(1);
			}
		}
	}
}
