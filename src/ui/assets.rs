use std::borrow::Cow;

use gpui_kit::*;

gpui_kit::assets::icon_assets!(
	pub ExtraIcons,
	[
		ArrowDown, ArrowUp, ArrowUpDown, AudioLines, Clock, Disc, Ellipsis, GripVertical, House, Link, ListMusic, ListPlus, LogOut,
		Pause, Pencil, Play, Plus, Repeat, Repeat1, Search, Settings, Shuffle, SkipBack,
		SkipForward, Trash, User, Volume2, VolumeX, X
	]
);

/// Component icons plus the extra Lucide icons the app uses.
pub struct AppAssets;

impl AssetSource for AppAssets {
	fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
		if let Some(bytes) = ExtraIcons.load(path)? {
			return Ok(Some(bytes));
		}
		gpui_kit::assets::Assets.load(path)
	}

	fn list(&self, path: &str) -> Result<Vec<SharedString>> {
		let mut paths = gpui_kit::assets::Assets.list(path)?;
		paths.extend(ExtraIcons.list(path)?);
		paths.sort();
		paths.dedup();
		Ok(paths)
	}
}
