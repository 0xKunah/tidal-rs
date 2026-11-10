//! Diagnostic: replays the app's TIDAL calls with the saved login and prints
//! outcomes (counts, errors, timings). Never prints tokens.
//!
//! cargo run --example diagnose

use std::sync::Arc;
use std::time::Instant;

use tidal_rs::model::AudioQuality;
use tidal_rs::tidal::auth::{Auth, KeyringStore, TokenStore};
use tidal_rs::tidal::config::{client_creds_path, load_client_creds};
use tidal_rs::tidal::http::HttpTidal;
use tidal_rs::tidal::{Endpoints, TidalApi};

#[tokio::main]
async fn main() {
	let creds = load_client_creds(&client_creds_path()).expect("client.toml");
	let store: Arc<dyn TokenStore> = Arc::new(KeyringStore);
	let refresh = store
		.load()
		.expect("no saved login: log in with the app first");
	let auth = Auth::new(Endpoints::default(), creds.clone());
	let tokens = auth.refresh(&refresh).await.expect("refresh");
	store.save(&tokens.refresh);
	let api = HttpTidal::connect(
		Endpoints::default(),
		Auth::new(Endpoints::default(), creds),
		tokens,
		store,
	)
	.await
	.expect("connect");
	println!("connected, user {}", api.user_id());

	let started = Instant::now();
	match api.playlists().await {
		Ok(playlists) => {
			println!(
				"playlists: {} ({} owned) in {:?}",
				playlists.len(),
				playlists.iter().filter(|p| p.owned).count(),
				started.elapsed()
			);
			if let Some(first) = playlists.first() {
				let started = Instant::now();
				match api.playlist_tracks(&first.id).await {
					Ok(tracks) => println!(
						"first playlist: {} tracks (announced {}) in {:?}",
						tracks.len(),
						first.track_count,
						started.elapsed()
					),
					Err(error) => println!("playlist_tracks error: {error}"),
				}
			}
		}
		Err(error) => println!("playlists error: {error}"),
	}

	if let Ok(results) = api.search("nightcall").await
		&& let Some(track) = results.tracks.first()
		&& let Ok(info) = api.stream(track.id, AudioQuality::Lossless).await
	{
		use std::sync::atomic::AtomicBool;
		use tidal_rs::audio::buffer::stream_buffer;
		use tidal_rs::audio::download::download;
		use tidal_rs::audio::engine::build_decoder;
		let (writer, reader) = stream_buffer(None);
		let started = Instant::now();
		let source = info.source.clone();
		let download_codec = info.codec.clone();
		let remuxed = tidal_rs::audio::download::is_remuxed(&info.source, &info.codec);
		let client = reqwest::Client::new();
		let task = tokio::spawn(async move {
			download(
				&client,
				&source,
				&download_codec,
				writer,
				Arc::new(AtomicBool::new(false)),
			)
			.await;
			started.elapsed()
		});
		let codec = info.codec.clone();
		let ready = tokio::task::spawn_blocking(move || {
			let decoder = build_decoder(reader, Some(&codec), remuxed);
			let ready = started.elapsed();
			if let Ok(decoder) = decoder {
				use rodio::Source as _;
				let rate = decoder.sample_rate().get() as usize;
				let channels = decoder.channels().get() as usize;
				let samples = decoder.take(rate * channels * 5).count();
				println!(
					"decoded {:.1} s of audio ({rate} Hz, {channels} ch) after {:?}",
					samples as f32 / (rate * channels) as f32,
					started.elapsed()
				);
				return (true, ready);
			}
			(false, ready)
		})
		.await
		.unwrap();
		let downloaded = task.await.unwrap();
		println!(
			"decoder ready={} after {:?}; full download after {:?}",
			ready.0, ready.1, downloaded
		);
	}

	match api.search("nightcall").await {
		Ok(results) => {
			if let Some(track) = results.tracks.first() {
				for quality in [AudioQuality::HiRes, AudioQuality::Lossless] {
					let started = Instant::now();
					match api.stream(track.id, quality).await {
						Ok(info) => println!(
							"stream {quality:?}: {:?} {} {:?} {:?}bit {:?}Hz gain {:?}dB peak {:?} in {:?}",
							info.quality,
							match info.source {
								tidal_rs::tidal::manifest::StreamSource::Progressive { .. } => {
									"progressive".to_string()
								}
								tidal_rs::tidal::manifest::StreamSource::Segmented {
									ref segments,
									..
								} => format!("dash, {} segments", segments.len()),
							},
							info.codec,
							info.bit_depth,
							info.sample_rate,
							info.replay_gain,
							info.peak,
							started.elapsed()
						),
						Err(error) => println!("stream {quality:?} error: {error}"),
					}
				}
			}
		}
		Err(error) => println!("search error: {error}"),
	}
}
