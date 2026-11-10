//! Small building blocks shared by every view.

use std::path::PathBuf;

use gpui_kit::assets::IconName;
use gpui_kit::component::{Icon, h_flex};
use gpui_kit::*;

use crate::ui::tokens;

pub fn icon(name: IconName, size: f32, color: Hsla) -> Icon {
	Icon::new(name).size(px(size)).text_color(color)
}

/// Cover image, or a neutral placeholder while it downloads.
pub fn cover(path: Option<PathBuf>, size: f32, radius: f32) -> AnyElement {
	match path {
		Some(path) => img(path)
			.size(px(size))
			.flex_none()
			.rounded(px(radius))
			.object_fit(ObjectFit::Cover)
			.into_any_element(),
		None => div()
			.size(px(size))
			.flex_none()
			.rounded(px(radius))
			.bg(tokens::accent())
			.flex()
			.items_center()
			.justify_center()
			.child(icon(
				IconName::Disc,
				(size * 0.4).min(48.),
				tokens::fg_dim(),
			))
			.into_any_element(),
	}
}

/// Square ghost button with an icon; red when `active`.
pub fn icon_button(
	id: impl Into<ElementId>,
	name: IconName,
	size: f32,
	active: bool,
	on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
	let color = if active {
		tokens::red()
	} else {
		tokens::fg_muted()
	};
	div()
		.id(id)
		.size(px(size + 14.))
		.flex_none()
		.flex()
		.items_center()
		.justify_center()
		.rounded(px(6.))
		.cursor_pointer()
		.text_color(color)
		.hover(|style| style.bg(tokens::muted()).text_color(tokens::fg()))
		.child(icon(name, size, color))
		.on_click(on_click)
}

/// Small uppercase section label.
pub fn section_label(text: impl Into<SharedString>) -> Div {
	h_flex()
		.text_size(px(11.))
		.font_weight(FontWeight::SEMIBOLD)
		.text_color(tokens::fg_dim())
		.child(text.into().to_uppercase())
}

/// Rounded keyboard hint, e.g. `Ctrl K`.
pub fn kbd(text: &'static str) -> Div {
	div()
		.px(px(5.))
		.py(px(1.))
		.rounded(px(4.))
		.border_1()
		.border_color(tokens::border())
		.bg(tokens::muted())
		.text_size(px(10.))
		.font_weight(FontWeight::MEDIUM)
		.text_color(tokens::fg_muted())
		.child(text)
}
