//! Stream manifests returned by `playbackinfopostpaywall`.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use quick_xml::events::{BytesStart, Event};
use serde::{Deserialize, Serialize};

use crate::model::AudioQuality;
use crate::tidal::{TidalError, TidalResult};

const BTS_MIME: &str = "application/vnd.tidal.bts";
const DASH_MIME: &str = "application/dash+xml";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BtsManifest {
	#[serde(default)]
	codecs: String,
	#[serde(default)]
	encryption_type: Option<String>,
	urls: Vec<String>,
}

fn codec(name: &str) -> Codec {
	let lower = name.to_ascii_lowercase();
	if lower.contains("flac") {
		Codec::Flac
	} else if lower.starts_with("mp4a") || lower.contains("aac") {
		Codec::Aac
	} else {
		Codec::Other(name.to_string())
	}
}

/// Decodes the base64 `manifest` of a playback info response.
pub fn decode_manifest(mime: &str, manifest_b64: &str) -> TidalResult<(StreamSource, Codec)> {
	let bytes = STANDARD
		.decode(manifest_b64.trim())
		.map_err(|error| TidalError::Decode(format!("manifest base64: {error}")))?;
	let text = String::from_utf8(bytes)
		.map_err(|error| TidalError::Decode(format!("manifest: {error}")))?;
	match mime {
		BTS_MIME => {
			let bts: BtsManifest = serde_json::from_str(&text)
				.map_err(|error| TidalError::Decode(format!("bts manifest: {error}")))?;
			if bts
				.encryption_type
				.as_deref()
				.is_some_and(|kind| kind != "NONE")
			{
				return Err(TidalError::Unavailable);
			}
			let url = bts
				.urls
				.into_iter()
				.next()
				.ok_or_else(|| TidalError::Decode("bts manifest without url".into()))?;
			Ok((StreamSource::Progressive { url }, codec(&bts.codecs)))
		}
		DASH_MIME => parse_mpd(&text),
		other => Err(TidalError::Decode(format!("unknown manifest type {other}"))),
	}
}

fn attribute(element: &BytesStart, name: &str) -> TidalResult<Option<String>> {
	let found = element
		.try_get_attribute(name)
		.map_err(|error| TidalError::Decode(format!("mpd attribute: {error}")))?;
	found
		.map(|value| {
			value
				.normalized_value(quick_xml::XmlVersion::Implicit1_0)
				.map(|value| value.into_owned())
				.map_err(|error| TidalError::Decode(format!("mpd attribute: {error}")))
		})
		.transpose()
}

/// Reads the first representation's `SegmentTemplate` and expands its `SegmentTimeline`.
fn parse_mpd(xml: &str) -> TidalResult<(StreamSource, Codec)> {
	let mut reader = quick_xml::Reader::from_str(xml);
	let mut codecs = String::new();
	let mut init = None;
	let mut media = None;
	let mut start_number: u64 = 1;
	let mut count: u64 = 0;
	loop {
		let event = reader
			.read_event()
			.map_err(|error| TidalError::Decode(format!("mpd: {error}")))?;
		match event {
			Event::Start(element) | Event::Empty(element) => match element.local_name().as_ref() {
				"Representation" if codecs.is_empty() => {
					codecs = attribute(&element, "codecs")?.unwrap_or_default();
				}
				"SegmentTemplate" if init.is_none() => {
					init = attribute(&element, "initialization")?;
					media = attribute(&element, "media")?;
					start_number = attribute(&element, "startNumber")?
						.and_then(|value| value.parse().ok())
						.unwrap_or(1);
				}
				"S" => {
					let repeat: i64 = attribute(&element, "r")?
						.and_then(|value| value.parse().ok())
						.unwrap_or(0);
					count += 1 + repeat.max(0) as u64;
				}
				_ => {}
			},
			Event::Eof => break,
			_ => {}
		}
	}
	let (Some(init), Some(media)) = (init, media) else {
		return Err(TidalError::Decode("mpd without SegmentTemplate".into()));
	};
	let segments = (start_number..start_number + count)
		.map(|number| media.replace("$Number$", &number.to_string()))
		.collect();
	Ok((StreamSource::Segmented { init, segments }, codec(&codecs)))
}

/// Short label for the player bar, e.g. `LOSSLESS` or `HI-RES 24/96`.
pub fn quality_badge(info: &StreamInfo) -> String {
	match info.quality {
		AudioQuality::Lossless => "LOSSLESS".into(),
		AudioQuality::High => "HIGH".into(),
		AudioQuality::HiRes => {
			let depth = info.bit_depth.unwrap_or(24);
			let rate = info.sample_rate.unwrap_or(96_000);
			let khz = if rate.is_multiple_of(1000) {
				(rate / 1000).to_string()
			} else {
				format!("{:.1}", f64::from(rate) / 1000.0)
			};
			format!("HI-RES {depth}/{khz}")
		}
	}
}

/// Maps the `audioQuality` field of a playback info response.
pub fn quality_from_api(value: &str) -> AudioQuality {
	match value {
		"HI_RES_LOSSLESS" | "HI_RES" => AudioQuality::HiRes,
		"LOSSLESS" => AudioQuality::Lossless,
		_ => AudioQuality::High,
	}
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Codec {
	Flac,
	Aac,
	Other(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StreamSource {
	/// One file over HTTP.
	Progressive { url: String },
	/// DASH: an init segment then media segments, concatenated in order.
	Segmented { init: String, segments: Vec<String> },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StreamInfo {
	pub source: StreamSource,
	pub codec: Codec,
	pub quality: AudioQuality,
	pub bit_depth: Option<u8>,
	pub sample_rate: Option<u32>,
	/// TIDAL loudness normalization for the track, in dB.
	#[serde(default)]
	pub replay_gain: Option<f32>,
	/// Track peak amplitude (1.0 = full scale).
	#[serde(default)]
	pub peak: Option<f32>,
}

/// Linear gain for TIDAL loudness normalization, limited so the peak never clips.
pub fn normalization_gain(replay_gain: Option<f32>, peak: Option<f32>) -> f32 {
	let Some(db) = replay_gain else {
		return 1.0;
	};
	let gain = 10f32.powf(db / 20.0);
	match peak {
		Some(peak) if peak > 0.0 => gain.min(1.0 / peak),
		_ => gain,
	}
}

#[cfg(test)]
mod tests {
	use base64::Engine;
	use base64::engine::general_purpose::STANDARD;

	use super::*;
	use crate::tidal::TidalError;

	fn b64(text: &str) -> String {
		STANDARD.encode(text)
	}

	fn info(quality: AudioQuality, bit_depth: u8, sample_rate: u32) -> StreamInfo {
		StreamInfo {
			source: StreamSource::Progressive { url: "u".into() },
			codec: Codec::Flac,
			quality,
			bit_depth: Some(bit_depth),
			sample_rate: Some(sample_rate),
			replay_gain: None,
			peak: None,
		}
	}

	#[test]
	fn bts_manifest_gives_progressive_flac() {
		let manifest = b64(
			r#"{"mimeType":"audio/flac","codecs":"flac","encryptionType":"NONE","urls":["https://x/y.flac"]}"#,
		);
		assert_eq!(
			decode_manifest("application/vnd.tidal.bts", &manifest),
			Ok((
				StreamSource::Progressive {
					url: "https://x/y.flac".into()
				},
				Codec::Flac
			))
		);
	}

	#[test]
	fn encrypted_manifest_is_unavailable() {
		let manifest = b64(
			r#"{"mimeType":"audio/flac","codecs":"flac","encryptionType":"OLD_AES","urls":["https://x/y.flac"]}"#,
		);
		assert_eq!(
			decode_manifest("application/vnd.tidal.bts", &manifest),
			Err(TidalError::Unavailable)
		);
	}

	#[test]
	fn dash_manifest_expands_segment_timeline() {
		let manifest = b64(include_str!("../../tests/fixtures/tidal/manifest.mpd"));
		let (source, codec) = decode_manifest("application/dash+xml", &manifest).unwrap();
		assert_eq!(codec, Codec::Flac);
		assert_eq!(
			source,
			StreamSource::Segmented {
				init: "https://x/init.mp4?a=1&b=2".into(),
				segments: (1..=4)
					.map(|n| format!("https://x/{n}.mp4?a=1&b=2"))
					.collect(),
			}
		);
	}

	#[test]
	fn normalization_applies_replay_gain() {
		assert!((normalization_gain(Some(-6.0206), None) - 0.5).abs() < 0.001);
		assert_eq!(normalization_gain(None, None), 1.0);
	}

	#[test]
	fn normalization_never_clips_the_peak() {
		let gain = normalization_gain(Some(6.0), Some(0.9));
		assert!((gain - 1.0 / 0.9).abs() < 0.001);
	}

	#[test]
	fn badges() {
		assert_eq!(
			quality_badge(&info(AudioQuality::Lossless, 16, 44100)),
			"LOSSLESS"
		);
		assert_eq!(
			quality_badge(&info(AudioQuality::HiRes, 24, 96000)),
			"HI-RES 24/96"
		);
		assert_eq!(
			quality_badge(&info(AudioQuality::HiRes, 24, 44100)),
			"HI-RES 24/44.1"
		);
		assert_eq!(quality_badge(&info(AudioQuality::High, 16, 44100)), "HIGH");
	}
}
