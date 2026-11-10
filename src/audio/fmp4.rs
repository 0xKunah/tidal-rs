//! Turns TIDAL's DASH FLAC (fragmented MP4) into a plain FLAC stream, segment by
//! segment, so playback starts with the first segment instead of the whole file.

/// Iterates the boxes of `data`: (type, payload). Stops at the first malformed box.
fn boxes(data: &[u8]) -> impl Iterator<Item = (&[u8], &[u8])> {
	let mut rest = data;
	std::iter::from_fn(move || {
		if rest.len() < 8 {
			return None;
		}
		let size = u32::from_be_bytes(rest[..4].try_into().ok()?) as usize;
		let kind = &rest[4..8];
		let (header, size) = match size {
			0 => (8, rest.len()),
			1 => {
				let large = u64::from_be_bytes(rest.get(8..16)?.try_into().ok()?) as usize;
				(16, large)
			}
			size => (8, size),
		};
		if size < header || size > rest.len() {
			rest = &[];
			return None;
		}
		let payload = &rest[header..size];
		rest = &rest[size..];
		Some((kind, payload))
	})
}

fn child<'a>(data: &'a [u8], kind: &[u8]) -> Option<&'a [u8]> {
	boxes(data)
		.find(|(found, _)| *found == kind)
		.map(|(_, payload)| payload)
}

/// `fLaC` marker + metadata blocks, read from the init segment's `dfLa` box.
pub fn flac_header(init: &[u8]) -> Option<Vec<u8>> {
	let moov = child(init, b"moov")?;
	let stsd = boxes(moov)
		.filter(|(kind, _)| *kind == b"trak")
		.find_map(|(_, trak)| {
			let stbl = child(child(child(trak, b"mdia")?, b"minf")?, b"stbl")?;
			child(stbl, b"stsd")
		})?;
	// stsd: version/flags (4) + entry count (4), then sample entries.
	let entry = child(stsd.get(8..)?, b"fLaC")?;
	// Audio sample entry fields take 28 bytes before the child boxes.
	let dfla = child(entry.get(28..)?, b"dfLa")?;
	// dfLa: version/flags (4), then FLAC metadata blocks.
	let mut header = b"fLaC".to_vec();
	header.extend_from_slice(dfla.get(4..)?);
	Some(header)
}

/// Raw FLAC frames of a media segment (`mdat` payloads).
pub fn mdat_payloads(segment: &[u8]) -> Vec<&[u8]> {
	boxes(segment)
		.filter(|(kind, _)| *kind == b"mdat")
		.map(|(_, payload)| payload)
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::test_support::{init_segment, mp4_box, streaminfo};

	#[test]
	fn flac_header_comes_from_the_dfla_box() {
		let header = flac_header(&init_segment()).unwrap();
		assert_eq!(&header[..4], b"fLaC");
		assert_eq!(&header[4..], streaminfo().as_slice());
	}

	#[test]
	fn init_without_flac_is_rejected() {
		let init = mp4_box(b"moov", &mp4_box(b"mvhd", &[0; 100]));
		assert!(flac_header(&init).is_none());
	}

	#[test]
	fn media_payload_is_the_mdat_content() {
		let segment = [
			mp4_box(b"styp", b"msdh"),
			mp4_box(b"moof", &[1, 2, 3]),
			mp4_box(b"mdat", &[0xff, 0xf8, 9, 9]),
		]
		.concat();
		assert_eq!(mdat_payloads(&segment), vec![&[0xff, 0xf8, 9, 9][..]]);
	}

	#[test]
	fn truncated_boxes_stop_parsing_without_panicking() {
		let mut segment = mp4_box(b"mdat", &[1, 2, 3, 4]);
		segment.truncate(10);
		assert!(mdat_payloads(&segment).is_empty());
	}
}
