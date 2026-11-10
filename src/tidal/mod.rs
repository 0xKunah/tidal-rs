//! TIDAL private API (v1). Everything that talks to TIDAL lives here.

pub mod auth;
pub mod config;
pub mod dto;
pub mod http;
pub mod manifest;

use std::future::Future;

use url::Url;

use crate::model::{
	Album, AlbumId, ArtistId, ArtistPage, AudioQuality, Playlist, PlaylistId, SearchResults, Track,
	TrackId,
};
use manifest::StreamInfo;

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum TidalError {
	#[error("not authorized")]
	Unauthorized,
	#[error("not found")]
	NotFound,
	#[error("not available for streaming")]
	Unavailable,
	#[error("the login code expired")]
	LoginExpired,
	#[error("network error: {0}")]
	Network(String),
	#[error("TIDAL returned {status}: {body}")]
	Api { status: u16, body: String },
	#[error("unexpected response: {0}")]
	Decode(String),
	#[error("{0}")]
	Config(String),
}

impl From<reqwest::Error> for TidalError {
	fn from(error: reqwest::Error) -> Self {
		if error.is_decode() {
			TidalError::Decode(error.to_string())
		} else {
			TidalError::Network(error.to_string())
		}
	}
}

/// Base URLs; tests point them at a mock server.
#[derive(Clone, Debug)]
pub struct Endpoints {
	pub auth: Url,
	pub api: Url,
}

impl Default for Endpoints {
	fn default() -> Self {
		Self {
			auth: Url::parse("https://auth.tidal.com/v1/").expect("static url"),
			api: Url::parse("https://api.tidal.com/v1/").expect("static url"),
		}
	}
}

pub type TidalResult<T> = Result<T, TidalError>;

/// Everything the app needs from TIDAL.
pub trait TidalApi: Send + Sync + 'static {
	/// Playlists the user created and the ones they favorited.
	fn playlists(&self) -> impl Future<Output = TidalResult<Vec<Playlist>>> + Send;
	fn playlist_tracks(
		&self,
		id: &PlaylistId,
	) -> impl Future<Output = TidalResult<Vec<Track>>> + Send;
	fn album(&self, id: AlbumId) -> impl Future<Output = TidalResult<(Album, Vec<Track>)>> + Send;
	fn artist(&self, id: ArtistId) -> impl Future<Output = TidalResult<ArtistPage>> + Send;
	fn search(&self, query: &str) -> impl Future<Output = TidalResult<SearchResults>> + Send;
	fn track_radio(&self, id: TrackId) -> impl Future<Output = TidalResult<Vec<Track>>> + Send;
	fn stream(
		&self,
		id: TrackId,
		quality: AudioQuality,
	) -> impl Future<Output = TidalResult<StreamInfo>> + Send;
	fn create_playlist(&self, title: &str) -> impl Future<Output = TidalResult<Playlist>> + Send;
	fn rename_playlist(
		&self,
		id: &PlaylistId,
		title: &str,
	) -> impl Future<Output = TidalResult<()>> + Send;
	fn delete_playlist(&self, id: &PlaylistId) -> impl Future<Output = TidalResult<()>> + Send;
	fn add_to_playlist(
		&self,
		id: &PlaylistId,
		track: TrackId,
	) -> impl Future<Output = TidalResult<()>> + Send;
	fn remove_from_playlist(
		&self,
		id: &PlaylistId,
		index: usize,
	) -> impl Future<Output = TidalResult<()>> + Send;
	fn move_in_playlist(
		&self,
		id: &PlaylistId,
		from: usize,
		to: usize,
	) -> impl Future<Output = TidalResult<()>> + Send;
}
