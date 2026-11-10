//! TIDAL v1 JSON shapes and their conversion to the domain model.

use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use crate::model::{
	Album, AlbumId, AlbumRef, Artist, ArtistId, ArtistRef, Playlist, PlaylistId, Track, TrackId,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Page<T> {
	#[serde(default = "Vec::new")]
	pub items: Vec<T>,
	#[serde(default)]
	pub total_number_of_items: Option<usize>,
}

impl<T> Default for Page<T> {
	fn default() -> Self {
		Self {
			items: Vec::new(),
			total_number_of_items: None,
		}
	}
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDto {
	pub user_id: u64,
	pub country_code: String,
}

#[derive(Deserialize)]
pub struct ArtistRefDto {
	pub id: u64,
	pub name: String,
}

impl From<ArtistRefDto> for ArtistRef {
	fn from(dto: ArtistRefDto) -> Self {
		ArtistRef {
			id: ArtistId(dto.id),
			name: dto.name,
		}
	}
}

#[derive(Deserialize)]
pub struct AlbumRefDto {
	pub id: u64,
	pub title: String,
	#[serde(default)]
	pub cover: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackDto {
	pub id: u64,
	pub title: String,
	#[serde(default)]
	pub version: Option<String>,
	pub duration: u64,
	#[serde(default)]
	pub allow_streaming: Option<bool>,
	#[serde(default)]
	pub stream_ready: Option<bool>,
	#[serde(default)]
	pub artists: Vec<ArtistRefDto>,
	pub album: AlbumRefDto,
	#[serde(default)]
	pub date_added: Option<String>,
}

impl From<TrackDto> for Track {
	fn from(dto: TrackDto) -> Self {
		let title = match dto.version.filter(|version| !version.is_empty()) {
			Some(version) => format!("{} ({version})", dto.title),
			None => dto.title,
		};
		Track {
			id: TrackId(dto.id),
			title,
			artists: dto.artists.into_iter().map(Into::into).collect(),
			album: AlbumRef {
				id: AlbumId(dto.album.id),
				title: dto.album.title,
				cover: dto.album.cover,
			},
			duration: Duration::from_secs(dto.duration),
			available: dto.allow_streaming.unwrap_or(true) && dto.stream_ready.unwrap_or(true),
			added: dto.date_added,
		}
	}
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumDto {
	pub id: u64,
	pub title: String,
	#[serde(default)]
	pub duration: Option<u64>,
	#[serde(default)]
	pub number_of_tracks: Option<u32>,
	#[serde(default)]
	pub release_date: Option<String>,
	#[serde(default)]
	pub cover: Option<String>,
	#[serde(default)]
	pub artists: Vec<ArtistRefDto>,
}

impl From<AlbumDto> for Album {
	fn from(dto: AlbumDto) -> Self {
		Album {
			id: AlbumId(dto.id),
			title: dto.title,
			artists: dto.artists.into_iter().map(Into::into).collect(),
			cover: dto.cover,
			year: dto
				.release_date
				.as_deref()
				.and_then(|date| date.get(..4))
				.and_then(|year| year.parse().ok()),
			track_count: dto.number_of_tracks.unwrap_or(0),
			duration: Duration::from_secs(dto.duration.unwrap_or(0)),
		}
	}
}

#[derive(Deserialize)]
pub struct ArtistDto {
	pub id: u64,
	pub name: String,
	#[serde(default)]
	pub picture: Option<String>,
}

impl From<ArtistDto> for Artist {
	fn from(dto: ArtistDto) -> Self {
		Artist {
			id: ArtistId(dto.id),
			name: dto.name,
			picture: dto.picture,
		}
	}
}

#[derive(Deserialize)]
pub struct CreatorDto {
	#[serde(default)]
	pub id: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistDto {
	pub uuid: String,
	pub title: String,
	#[serde(default)]
	pub number_of_tracks: Option<u32>,
	#[serde(default)]
	pub duration: Option<u64>,
	#[serde(default)]
	pub image: Option<String>,
	#[serde(default)]
	pub square_image: Option<String>,
	#[serde(default)]
	pub creator: Option<CreatorDto>,
}

impl PlaylistDto {
	pub fn into_playlist(self, user_id: u64) -> Playlist {
		Playlist {
			owned: self.creator.and_then(|creator| creator.id) == Some(user_id),
			id: PlaylistId(self.uuid),
			title: self.title,
			cover: self.square_image.or(self.image),
			track_count: self.number_of_tracks.unwrap_or(0),
			duration: Duration::from_secs(self.duration.unwrap_or(0)),
		}
	}
}

/// `playlistsAndFavoritePlaylists` items wrap the playlist; tolerate bare playlists too.
#[derive(Deserialize)]
#[serde(untagged)]
pub enum FavoritePlaylistDto {
	Wrapped { playlist: PlaylistDto },
	Plain(PlaylistDto),
}

impl FavoritePlaylistDto {
	pub fn into_playlist(self, user_id: u64) -> Playlist {
		match self {
			FavoritePlaylistDto::Wrapped { playlist } | FavoritePlaylistDto::Plain(playlist) => {
				playlist.into_playlist(user_id)
			}
		}
	}
}

/// Playlist items can be tracks or videos; only tracks are kept.
#[derive(Deserialize)]
pub struct PlaylistItemDto {
	#[serde(rename = "type", default)]
	pub kind: Option<String>,
	pub item: Value,
}

impl PlaylistItemDto {
	pub fn into_track(self) -> Option<Track> {
		if self.kind.as_deref().is_some_and(|kind| kind != "track") {
			return None;
		}
		serde_json::from_value::<TrackDto>(self.item)
			.ok()
			.map(Into::into)
	}
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct SearchDto {
	pub tracks: Page<TrackDto>,
	pub artists: Page<ArtistDto>,
	pub albums: Page<AlbumDto>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackInfoDto {
	pub audio_quality: String,
	#[serde(default)]
	pub bit_depth: Option<u8>,
	#[serde(default)]
	pub sample_rate: Option<u32>,
	pub manifest_mime_type: String,
	pub manifest: String,
	#[serde(default)]
	pub track_replay_gain: Option<f32>,
	#[serde(default)]
	pub track_peak_amplitude: Option<f32>,
}
