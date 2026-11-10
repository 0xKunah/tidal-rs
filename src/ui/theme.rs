use std::borrow::Cow;

use gpui_kit::component::scroll::ScrollbarMode;
use gpui_kit::component::{Theme, ThemeMode, ThemeRegistry};
use gpui_kit::*;

const THEME_JSON: &str = include_str!("../../assets/themes/tidal-rs.json");
const THEME_NAME: &str = "tidal-rs";
pub const FONT_FAMILY: &str = "Inter";

/// Registers the bundled Inter fonts and applies the tidal-rs dark theme.
pub fn install(cx: &mut App) {
	let fonts: Vec<Cow<'static, [u8]>> = vec![
		Cow::Borrowed(include_bytes!("../../assets/fonts/Inter-Regular.ttf")),
		Cow::Borrowed(include_bytes!("../../assets/fonts/Inter-Medium.ttf")),
		Cow::Borrowed(include_bytes!("../../assets/fonts/Inter-SemiBold.ttf")),
		Cow::Borrowed(include_bytes!("../../assets/fonts/Inter-Bold.ttf")),
	];
	if let Err(error) = cx.text_system().add_fonts(fonts) {
		eprintln!("failed to load Inter fonts: {error}");
	}

	let registry = ThemeRegistry::global_mut(cx);
	registry
		.load_themes_from_str(THEME_JSON)
		.expect("bundled theme must parse");
	let config = registry
		.themes()
		.get(THEME_NAME)
		.cloned()
		.expect("bundled theme must be registered");

	Theme::update(cx, |theme| {
		theme.dark_theme = config;
		theme.font_family = FONT_FAMILY.into();
	});
	Theme::change(ThemeMode::Dark, None, cx);
	// Long lists need a visible scrollbar, not one that only shows while scrolling.
	Theme::set_scrollbar_mode(ScrollbarMode::Always, cx);
}

#[cfg(test)]
mod tests {
	#[test]
	fn theme_file_is_dark_with_red_primary() {
		let v: serde_json::Value =
			serde_json::from_str(include_str!("../../assets/themes/tidal-rs.json")).unwrap();
		let theme = &v["themes"][0];
		assert_eq!(theme["mode"], "dark");
		assert_eq!(theme["name"], "tidal-rs");
		assert_eq!(theme["colors"]["primary.background"], "#dc2626");
		assert_eq!(theme["colors"]["background"], "#09090b");
	}
}
