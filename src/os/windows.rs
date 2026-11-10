//! Windows integration: the process AppUserModelID and the System Media
//! Transport Controls (media keys, volume overlay, Stream Deck scripts).

use std::time::Duration;

use windows::Foundation::{TimeSpan, TypedEventHandler, Uri};
use windows::Media::{
	MediaPlaybackStatus, MediaPlaybackType, SystemMediaTransportControls,
	SystemMediaTransportControlsButton, SystemMediaTransportControlsButtonPressedEventArgs,
	SystemMediaTransportControlsTimelineProperties,
};
use windows::Storage::Streams::RandomAccessStreamReference;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::WinRT::ISystemMediaTransportControlsInterop;
use windows::Win32::UI::Shell::{
	GetCurrentProcessExplicitAppUserModelID, SetCurrentProcessExplicitAppUserModelID,
};
use windows::core::{HSTRING, Ref, factory};

/// Contains "tidal" so the user's Stream Deck scripts (`-AppMatch tidal`) find the session.
pub const APP_ID: &str = "TidalRs.Player";

/// Must run before any window is created.
pub fn set_app_id(id: &str) -> windows::core::Result<()> {
	unsafe { SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(id)) }
}

pub fn app_id() -> Option<String> {
	unsafe {
		let raw = GetCurrentProcessExplicitAppUserModelID().ok()?;
		let id = raw.to_string().ok();
		CoTaskMemFree(Some(raw.0 as *const _));
		id
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaButton {
	Play,
	Pause,
	Next,
	Previous,
}

/// What the Windows media overlay shows.
#[derive(Clone, Debug, PartialEq)]
pub struct NowPlaying {
	pub title: String,
	pub artist: String,
	pub album: String,
	pub cover_url: Option<String>,
	pub playing: bool,
	pub position: Duration,
	pub duration: Duration,
}

fn ticks(duration: Duration) -> TimeSpan {
	TimeSpan {
		Duration: (duration.as_nanos() / 100) as i64,
	}
}

pub struct MediaControls {
	smtc: SystemMediaTransportControls,
	token: i64,
}

impl MediaControls {
	/// Registers the window with SMTC; button presses are sent to `buttons`.
	pub fn attach(
		hwnd: isize,
		buttons: async_channel::Sender<MediaButton>,
	) -> windows::core::Result<Self> {
		let interop =
			factory::<SystemMediaTransportControls, ISystemMediaTransportControlsInterop>()?;
		let smtc: SystemMediaTransportControls =
			unsafe { interop.GetForWindow(HWND(hwnd as *mut _)) }?;
		smtc.SetIsEnabled(true)?;
		smtc.SetIsPlayEnabled(true)?;
		smtc.SetIsPauseEnabled(true)?;
		smtc.SetIsNextEnabled(true)?;
		smtc.SetIsPreviousEnabled(true)?;
		let handler = TypedEventHandler::new(
			move |_: Ref<SystemMediaTransportControls>,
			      args: Ref<SystemMediaTransportControlsButtonPressedEventArgs>| {
				if let Some(args) = args.as_ref() {
					let button = match args.Button()? {
						SystemMediaTransportControlsButton::Play => Some(MediaButton::Play),
						SystemMediaTransportControlsButton::Pause => Some(MediaButton::Pause),
						SystemMediaTransportControlsButton::Next => Some(MediaButton::Next),
						SystemMediaTransportControlsButton::Previous => Some(MediaButton::Previous),
						_ => None,
					};
					if let Some(button) = button {
						let _ = buttons.try_send(button);
					}
				}
				Ok(())
			},
		);
		let token = smtc.ButtonPressed(&handler)?;
		Ok(Self { smtc, token })
	}

	/// Title, artist, album and cover; call when the track changes.
	pub fn set_track(&self, now: Option<&NowPlaying>) -> windows::core::Result<()> {
		let updater = self.smtc.DisplayUpdater()?;
		let Some(now) = now else {
			updater.ClearAll()?;
			updater.Update()?;
			return self.smtc.SetPlaybackStatus(MediaPlaybackStatus::Stopped);
		};
		updater.SetType(MediaPlaybackType::Music)?;
		let music = updater.MusicProperties()?;
		music.SetTitle(&HSTRING::from(now.title.as_str()))?;
		music.SetArtist(&HSTRING::from(now.artist.as_str()))?;
		music.SetAlbumTitle(&HSTRING::from(now.album.as_str()))?;
		if let Some(url) = &now.cover_url {
			let uri = Uri::CreateUri(&HSTRING::from(url.as_str()))?;
			updater.SetThumbnail(&RandomAccessStreamReference::CreateFromUri(&uri)?)?;
		}
		updater.Update()
	}

	/// Play state and timeline; cheap enough to call on every change.
	pub fn set_state(&self, now: &NowPlaying) -> windows::core::Result<()> {
		self.smtc.SetPlaybackStatus(if now.playing {
			MediaPlaybackStatus::Playing
		} else {
			MediaPlaybackStatus::Paused
		})?;
		let timeline = SystemMediaTransportControlsTimelineProperties::new()?;
		timeline.SetStartTime(ticks(Duration::ZERO))?;
		timeline.SetMinSeekTime(ticks(Duration::ZERO))?;
		timeline.SetEndTime(ticks(now.duration))?;
		timeline.SetMaxSeekTime(ticks(now.duration))?;
		timeline.SetPosition(ticks(now.position))?;
		self.smtc.UpdateTimelineProperties(&timeline)
	}
}

impl Drop for MediaControls {
	fn drop(&mut self) {
		let _ = self.smtc.RemoveButtonPressed(self.token);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn app_id_is_set_and_contains_tidal() {
		set_app_id(APP_ID).unwrap();
		assert_eq!(app_id().as_deref(), Some(APP_ID));
		assert!(APP_ID.to_lowercase().contains("tidal"));
	}
}
