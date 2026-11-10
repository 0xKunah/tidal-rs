//! Streams a track over HTTP into a `BufferWriter`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::audio::buffer::BufferWriter;
use crate::audio::fmp4;
use crate::tidal::manifest::{Codec, StreamSource};

/// True when the stream is DASH FLAC, which is remuxed to plain FLAC while downloading.
pub fn is_remuxed(source: &StreamSource, codec: &Codec) -> bool {
	matches!(source, StreamSource::Segmented { .. }) && *codec == Codec::Flac
}

/// Downloads `source` into `writer`, stopping early when `cancel` is set.
/// Errors are reported through the writer so the reader sees them.
pub async fn download(
	client: &reqwest::Client,
	source: &StreamSource,
	codec: &Codec,
	writer: BufferWriter,
	cancel: Arc<AtomicBool>,
) {
	if let StreamSource::Segmented { init, segments } = source
		&& *codec == Codec::Flac
	{
		match remux_flac(client, init, segments, &writer, &cancel).await {
			Ok(()) if !cancel.load(Ordering::Relaxed) => writer.finish(),
			Ok(()) => writer.fail("cancelled".into()),
			Err(error) => writer.fail(error),
		}
		return;
	}
	let urls: Vec<&str> = match source {
		StreamSource::Progressive { url } => vec![url.as_str()],
		StreamSource::Segmented { init, segments } => std::iter::once(init.as_str())
			.chain(segments.iter().map(String::as_str))
			.collect(),
	};
	let single = urls.len() == 1;
	if !single && let Some(len) = total_length(client, source).await {
		// Known size up front lets decoding start before the last segment arrives.
		writer.set_len(len);
	}
	for url in urls {
		if let Err(error) = fetch(client, url, &writer, &cancel, single).await {
			writer.fail(error);
			return;
		}
		if cancel.load(Ordering::Relaxed) {
			writer.fail("cancelled".into());
			return;
		}
	}
	writer.finish();
}

/// Writes a plain FLAC stream: the header from the init segment, then each
/// segment's frames as soon as it arrives.
async fn remux_flac(
	client: &reqwest::Client,
	init: &str,
	segments: &[String],
	writer: &BufferWriter,
	cancel: &AtomicBool,
) -> Result<(), String> {
	let init = get_bytes(client, init).await?;
	let header = fmp4::flac_header(&init).ok_or("DASH init segment without FLAC header")?;
	writer.push(&header);
	for url in segments {
		if cancel.load(Ordering::Relaxed) {
			return Ok(());
		}
		let segment = get_bytes(client, url).await?;
		for frames in fmp4::mdat_payloads(&segment) {
			writer.push(frames);
		}
	}
	Ok(())
}

async fn get_bytes(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
	let response = client
		.get(url)
		.send()
		.await
		.map_err(|error| error.to_string())?;
	if !response.status().is_success() {
		return Err(format!("HTTP {}", response.status()));
	}
	response
		.bytes()
		.await
		.map(|bytes| bytes.to_vec())
		.map_err(|error| error.to_string())
}

/// Sum of every part's `Content-Length` (HEAD requests in parallel); None if any is unknown.
pub async fn total_length(client: &reqwest::Client, source: &StreamSource) -> Option<u64> {
	let urls: Vec<String> = match source {
		StreamSource::Progressive { url } => vec![url.clone()],
		StreamSource::Segmented { init, segments } => std::iter::once(init.clone())
			.chain(segments.iter().cloned())
			.collect(),
	};
	let mut heads = tokio::task::JoinSet::new();
	for url in urls {
		let client = client.clone();
		heads.spawn(async move {
			let response = client.head(&url).send().await.ok()?;
			if !response.status().is_success() {
				return None;
			}
			response
				.headers()
				.get(reqwest::header::CONTENT_LENGTH)?
				.to_str()
				.ok()?
				.parse::<u64>()
				.ok()
		});
	}
	let mut total = 0;
	while let Some(result) = heads.join_next().await {
		total += result.ok()??;
	}
	Some(total)
}

async fn fetch(
	client: &reqwest::Client,
	url: &str,
	writer: &BufferWriter,
	cancel: &AtomicBool,
	set_len: bool,
) -> Result<(), String> {
	let mut response = client
		.get(url)
		.send()
		.await
		.map_err(|error| error.to_string())?;
	if !response.status().is_success() {
		return Err(format!("HTTP {}", response.status()));
	}
	if set_len && let Some(len) = response.content_length() {
		writer.set_len(len);
	}
	while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
		if cancel.load(Ordering::Relaxed) {
			return Ok(());
		}
		writer.push(&chunk);
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use std::io::Read;
	use std::sync::Arc;
	use std::sync::atomic::AtomicBool;

	use wiremock::matchers::{method, path};
	use wiremock::{Mock, MockServer, ResponseTemplate};

	use super::*;
	use crate::audio::buffer::stream_buffer;
	use crate::tidal::manifest::{Codec, StreamSource};

	#[tokio::test]
	async fn download_fills_the_buffer() {
		let server = MockServer::start().await;
		let body: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
		Mock::given(method("GET"))
			.and(path("/track.flac"))
			.respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone()))
			.mount(&server)
			.await;
		let (writer, mut reader) = stream_buffer(None);
		let source = StreamSource::Progressive {
			url: format!("{}/track.flac", server.uri()),
		};
		download(
			&reqwest::Client::new(),
			&source,
			&Codec::Other("test".into()),
			writer,
			Arc::new(AtomicBool::new(false)),
		)
		.await;
		assert_eq!(reader.wait_len(), Some(10_000));
		let all = tokio::task::spawn_blocking(move || {
			let mut all = vec![];
			reader.read_to_end(&mut all).unwrap();
			all
		})
		.await
		.unwrap();
		assert_eq!(all, body);
	}

	#[tokio::test]
	async fn download_concatenates_init_and_segments() {
		let server = MockServer::start().await;
		let parts: Vec<(&str, Vec<u8>)> = vec![
			("/init.mp4", vec![1, 1]),
			("/1.mp4", vec![2, 2, 2]),
			("/2.mp4", vec![3]),
			("/3.mp4", vec![4, 4]),
		];
		for (at, body) in &parts {
			Mock::given(path(*at))
				.respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone()))
				.mount(&server)
				.await;
		}
		let (writer, mut reader) = stream_buffer(None);
		let source = StreamSource::Segmented {
			init: format!("{}/init.mp4", server.uri()),
			segments: (1..=3)
				.map(|n| format!("{}/{n}.mp4", server.uri()))
				.collect(),
		};
		download(
			&reqwest::Client::new(),
			&source,
			&Codec::Other("test".into()),
			writer,
			Arc::new(AtomicBool::new(false)),
		)
		.await;
		assert_eq!(reader.wait_len(), Some(8));
		let all = tokio::task::spawn_blocking(move || {
			let mut all = vec![];
			reader.read_to_end(&mut all).unwrap();
			all
		})
		.await
		.unwrap();
		assert_eq!(all, vec![1, 1, 2, 2, 2, 3, 4, 4]);
	}

	#[tokio::test]
	async fn segmented_total_length_comes_from_head_requests() {
		let server = MockServer::start().await;
		for (at, body) in [("/init.mp4", vec![0u8; 5]), ("/1.mp4", vec![0u8; 7])] {
			Mock::given(path(at))
				.respond_with(ResponseTemplate::new(200).set_body_bytes(body))
				.mount(&server)
				.await;
		}
		let source = StreamSource::Segmented {
			init: format!("{}/init.mp4", server.uri()),
			segments: vec![format!("{}/1.mp4", server.uri())],
		};
		assert_eq!(
			total_length(&reqwest::Client::new(), &source).await,
			Some(12)
		);
	}

	#[tokio::test]
	async fn dash_flac_is_remuxed_to_plain_flac() {
		use crate::test_support::{init_segment, mp4_box, streaminfo};
		let server = MockServer::start().await;
		let segment =
			|frames: &[u8]| [mp4_box(b"moof", &[0; 12]), mp4_box(b"mdat", frames)].concat();
		for (at, body) in [
			("/init.mp4", init_segment()),
			("/1.mp4", segment(&[0xff, 0xf8, 1])),
			("/2.mp4", segment(&[0xff, 0xf8, 2])),
		] {
			Mock::given(path(at))
				.respond_with(ResponseTemplate::new(200).set_body_bytes(body))
				.mount(&server)
				.await;
		}
		let (writer, mut reader) = stream_buffer(None);
		let source = StreamSource::Segmented {
			init: format!("{}/init.mp4", server.uri()),
			segments: (1..=2)
				.map(|n| format!("{}/{n}.mp4", server.uri()))
				.collect(),
		};
		download(
			&reqwest::Client::new(),
			&source,
			&Codec::Flac,
			writer,
			Arc::new(AtomicBool::new(false)),
		)
		.await;
		let all = tokio::task::spawn_blocking(move || {
			let mut all = vec![];
			reader.read_to_end(&mut all).unwrap();
			all
		})
		.await
		.unwrap();
		let expected = [
			b"fLaC".to_vec(),
			streaminfo(),
			vec![0xff, 0xf8, 1, 0xff, 0xf8, 2],
		]
		.concat();
		assert_eq!(all, expected);
	}

	#[tokio::test]
	async fn http_error_fails_the_reader() {
		let server = MockServer::start().await;
		Mock::given(method("GET"))
			.respond_with(ResponseTemplate::new(403))
			.mount(&server)
			.await;
		let (writer, mut reader) = stream_buffer(None);
		let source = StreamSource::Progressive {
			url: format!("{}/x", server.uri()),
		};
		download(
			&reqwest::Client::new(),
			&source,
			&Codec::Other("test".into()),
			writer,
			Arc::new(AtomicBool::new(false)),
		)
		.await;
		let mut byte = [0u8; 1];
		assert!(reader.read(&mut byte).is_err());
	}
}
