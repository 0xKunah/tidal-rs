//! Builders shared by unit tests.

use std::time::Duration;

use crate::model::{AlbumId, AlbumRef, ArtistId, ArtistRef, Track, TrackId};

/// Track `id` titled "T{id}", artist "A", album 1, 180 s, available.
pub fn t(id: u64) -> Track {
	Track {
		id: TrackId(id),
		title: format!("T{id}"),
		artists: vec![ArtistRef {
			id: ArtistId(1),
			name: "A".into(),
		}],
		album: AlbumRef {
			id: AlbumId(1),
			title: "Album".into(),
			cover: None,
		},
		duration: Duration::from_secs(180),
		available: true,
		added: None,
	}
}

pub fn list(ids: &[u64]) -> Vec<Track> {
	ids.iter().map(|&id| t(id)).collect()
}

pub fn ids(tracks: &[&Track]) -> Vec<u64> {
	tracks.iter().map(|track| track.id.0).collect()
}

pub fn rng() -> fastrand::Rng {
	fastrand::Rng::with_seed(7)
}

// ---- fragmented MP4 (DASH FLAC) builders ----

pub fn mp4_box(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
	let mut out = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
	out.extend_from_slice(kind);
	out.extend_from_slice(payload);
	out
}

/// STREAMINFO metadata block (last block), 34 bytes of body.
pub fn streaminfo() -> Vec<u8> {
	let mut block = vec![0x80, 0x00, 0x00, 0x22];
	block.extend((0..34u8).collect::<Vec<_>>());
	block
}

/// Minimal fMP4 init segment: ftyp + moov/trak/mdia/minf/stbl/stsd/fLaC/dfLa.
pub fn init_segment() -> Vec<u8> {
	let mut dfla = vec![0, 0, 0, 0];
	dfla.extend(streaminfo());
	let mut entry = vec![0u8; 28];
	entry.extend(mp4_box(b"dfLa", &dfla));
	let mut stsd = vec![0, 0, 0, 0, 0, 0, 0, 1];
	stsd.extend(mp4_box(b"fLaC", &entry));
	let stbl = mp4_box(b"stbl", &mp4_box(b"stsd", &stsd));
	let minf = mp4_box(b"minf", &stbl);
	let mdia = mp4_box(b"mdia", &[mp4_box(b"mdhd", &[0; 24]), minf].concat());
	let trak = mp4_box(b"trak", &[mp4_box(b"tkhd", &[0; 84]), mdia].concat());
	let moov = mp4_box(b"moov", &[mp4_box(b"mvhd", &[0; 100]), trak].concat());
	[mp4_box(b"ftyp", b"iso6mp41"), moov].concat()
}
