//! Settings: language, audio quality, account.

use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::i18n::Lang;
use crate::model::AudioQuality;
use crate::ui::state::AppState;
use crate::ui::tokens;

fn chip(
	id: &'static str,
	label: SharedString,
	active: bool,
	on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
	div()
		.id(id)
		.h(px(32.))
		.px(px(14.))
		.flex()
		.items_center()
		.rounded(px(8.))
		.border_1()
		.cursor_pointer()
		.font_weight(FontWeight::MEDIUM)
		.map(|chip| {
			if active {
				chip.border_color(rgba(0xdc262673))
					.bg(tokens::red_tint())
					.text_color(tokens::red())
			} else {
				chip.border_color(tokens::border())
					.hover(|style| style.bg(tokens::muted()))
			}
		})
		.child(label)
		.on_click(on_click)
}

fn row(title: impl Into<SharedString>, content: impl IntoElement) -> Div {
	v_flex()
		.gap(px(10.))
		.py(px(18.))
		.border_b_1()
		.border_color(tokens::border())
		.child(div().font_weight(FontWeight::SEMIBOLD).child(title.into()))
		.child(content)
}

pub fn settings_page(state: &Entity<AppState>, cx: &App) -> AnyElement {
	let app = state.read(cx);
	let language = app.settings.language;
	let quality = app.settings.quality;
	let account = app.account_name().unwrap_or_default().to_string();
	let hires_fallback = app.hires_fallback;

	let language_chip = |id: &'static str, label: SharedString, value: Option<Lang>| {
		let state = state.clone();
		chip(id, label, language == value, move |_, _, cx| {
			state.update(cx, |state, cx| state.set_language(value, cx))
		})
	};
	let quality_chip = |id: &'static str, label: SharedString, value: AudioQuality| {
		let state = state.clone();
		chip(id, label, quality == value, move |_, _, cx| {
			state.update(cx, |state, cx| state.set_quality(value, cx))
		})
	};
	let logout_state = state.clone();

	v_flex()
		.id("settings-page")
		.size_full()
		.overflow_y_scrollbar()
		.px(px(32.))
		.pt(px(28.))
		.max_w(px(720.))
		.child(
			div()
				.pb(px(8.))
				.text_size(px(32.))
				.font_weight(FontWeight::BOLD)
				.child(rust_i18n::t!("settings.title")),
		)
		.child(row(
			rust_i18n::t!("settings.language"),
			h_flex()
				.gap(px(8.))
				.child(language_chip(
					"lang-system",
					rust_i18n::t!("settings.language_system").into(),
					None,
				))
				.child(language_chip(
					"lang-fr",
					rust_i18n::t!("settings.language_fr").into(),
					Some(Lang::Fr),
				))
				.child(language_chip(
					"lang-en",
					rust_i18n::t!("settings.language_en").into(),
					Some(Lang::En),
				)),
		))
		.child(row(
			rust_i18n::t!("settings.quality"),
			v_flex()
				.gap(px(8.))
				.child(
					h_flex()
						.gap(px(8.))
						.child(quality_chip(
							"quality-max",
							rust_i18n::t!("settings.quality_max").into(),
							AudioQuality::HiRes,
						))
						.child(quality_chip(
							"quality-lossless",
							rust_i18n::t!("settings.quality_lossless").into(),
							AudioQuality::Lossless,
						))
						.child(quality_chip(
							"quality-high",
							rust_i18n::t!("settings.quality_high").into(),
							AudioQuality::High,
						)),
				)
				.when(hires_fallback, |column| {
					column.child(
						div()
							.text_size(px(12.))
							.text_color(tokens::fg_dim())
							.child(rust_i18n::t!("settings.quality_max_unsupported")),
					)
				}),
		))
		.child(row(
			rust_i18n::t!("settings.account"),
			h_flex()
				.gap(px(16.))
				.child(div().text_color(tokens::fg_muted()).child(account))
				.child(chip(
					"logout",
					rust_i18n::t!("settings.logout").into(),
					false,
					move |_, _, cx| logout_state.update(cx, |state, cx| state.logout(cx)),
				)),
		))
		.into_any_element()
}
