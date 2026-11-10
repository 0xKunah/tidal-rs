//! Domain types shared by every layer.

use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TrackId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AlbumId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ArtistId(pub u64);

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PlaylistId(pub String);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArtistRef {
	pub id: ArtistId,
	pub name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AlbumRef {
	pub id: AlbumId,
	pub title: String,
	/// TIDAL image uuid.
	pub cover: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Track {
	pub id: TrackId,
	pub title: String,
	pub artists: Vec<ArtistRef>,
	pub album: AlbumRef,
	pub duration: Duration,
	/// False when TIDAL does not allow streaming it.
	pub available: bool,
	/// Date it was added to the playlist it was loaded from (ISO 8601), if any.
	#[serde(default)]
	pub added: Option<String>,
}

impl Track {
	/// Artist names joined with ", ".
	pub fn artist_names(&self) -> String {
		self.artists
			.iter()
			.map(|artist| artist.name.as_str())
			.collect::<Vec<_>>()
			.join(", ")
	}
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Album {
	pub id: AlbumId,
	pub title: String,
	pub artists: Vec<ArtistRef>,
	pub cover: Option<String>,
	pub year: Option<u16>,
	pub track_count: u32,
	pub duration: Duration,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Artist {
	pub id: ArtistId,
	pub name: String,
	pub picture: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Playlist {
	pub id: PlaylistId,
	pub title: String,
	pub cover: Option<String>,
	pub track_count: u32,
	pub duration: Duration,
	/// True when the user created it (editable).
	pub owned: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArtistPage {
	pub artist: Artist,
	pub top_tracks: Vec<Track>,
	pub albums: Vec<Album>,
	pub singles: Vec<Album>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchResults {
	pub tracks: Vec<Track>,
	pub artists: Vec<Artist>,
	pub albums: Vec<Album>,
}

/// The list a track was started from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ContextRef {
	Playlist { id: PlaylistId, title: String },
	Album { id: AlbumId, title: String },
	ArtistTop { id: ArtistId, name: String },
	Recent,
}

impl ContextRef {
	pub fn origin(&self) -> Origin {
		match self {
			ContextRef::Playlist { id, .. } => Origin::Playlist(id.clone()),
			_ => Origin::Other,
		}
	}
}

/// Where a queued track came from; drives "Recently played".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Origin {
	Playlist(PlaylistId),
	Other,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueueItem {
	pub track: Track,
	pub origin: Origin,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Route {
	Playlist(PlaylistId),
	Album(AlbumId),
	Artist(ArtistId),
	Search,
	Recent,
	Settings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioQuality {
	HiRes,
	Lossless,
	High,
}

impl AudioQuality {
	/// Value of the `audioquality` API parameter.
	pub fn api(self) -> &'static str {
		match self {
			AudioQuality::HiRes => "HI_RES_LOSSLESS",
			AudioQuality::Lossless => "LOSSLESS",
			AudioQuality::High => "HIGH",
		}
	}
}

pub fn cover_url(uuid: &str, size: u32) -> String {
	format!(
		"https://resources.tidal.com/images/{}/{size}x{size}.jpg",
		uuid.replace('-', "/")
	)
}

pub fn track_share_url(id: TrackId) -> String {
	format!("https://tidal.com/browse/track/{}", id.0)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn cover_url_uses_slashes_and_size() {
		assert_eq!(
			cover_url("ab-cd", 320),
			"https://resources.tidal.com/images/ab/cd/320x320.jpg"
		);
	}

	#[test]
	fn playlist_context_has_playlist_origin() {
		let id = PlaylistId("p".into());
		let kind = ContextRef::Playlist {
			id: id.clone(),
			title: "P".into(),
		};
		assert_eq!(kind.origin(), Origin::Playlist(id));
		assert_eq!(ContextRef::Recent.origin(), Origin::Other);
	}

	#[test]
	fn share_url() {
		assert_eq!(
			track_share_url(TrackId(5)),
			"https://tidal.com/browse/track/5"
		);
	}
}
