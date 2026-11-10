//! Left column: search, "Recently played", playlists, settings.

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::model::{ContextRef, Route};
use crate::ui::state::{AppState, COVER_SMALL, Remote};
use crate::ui::{tokens, widgets};

pub const WIDTH: f32 = 248.;

fn nav_item(
	id: &'static str,
	name: IconName,
	label: SharedString,
	active: bool,
	on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
	h_flex()
		.id(id)
		.h(px(34.))
		.px(px(10.))
		.gap(px(10.))
		.rounded(px(8.))
		.cursor_pointer()
		.font_weight(FontWeight::MEDIUM)
		.text_color(if active {
			tokens::fg()
		} else {
			tokens::fg_muted()
		})
		.when(active, |item| item.bg(tokens::muted()))
		.hover(|style| style.bg(tokens::muted()).text_color(tokens::fg()))
		.child(widgets::icon(
			name,
			16.,
			if active {
				tokens::fg()
			} else {
				tokens::fg_muted()
			},
		))
		.child(label)
		.on_click(on_click)
}

pub fn sidebar(
	state: &Entity<AppState>,
	search: &Entity<InputState>,
	cx: &App,
) -> impl IntoElement {
	let app = state.read(cx);
	let route = app.route.clone();
	let playing_playlist = match app.player.view().context {
		Some(ContextRef::Playlist { id, .. }) => Some(id),
		_ => None,
	};
	let playlists = match &app.playlists {
		Remote::Ready(playlists) => playlists.clone(),
		_ => Vec::new(),
	};
	let rows = playlists.into_iter().enumerate().map(|(ix, playlist)| {
		let active = route == Some(Route::Playlist(playlist.id.clone()));
		let playing = playing_playlist.as_ref() == Some(&playlist.id);
		let cover = app.cover(playlist.cover.as_deref(), COVER_SMALL);
		let id = playlist.id.clone();
		let state = state.clone();
		h_flex()
			.id(("playlist", ix))
			.h(px(48.))
			.px(px(8.))
			.gap(px(10.))
			.rounded(px(8.))
			.cursor_pointer()
			.when(active, |row| row.bg(tokens::muted()))
			.hover(|style| style.bg(tokens::muted()))
			.child(widgets::cover(cover, 34., 6.))
			.child(
				v_flex()
					.flex_1()
					.min_w_0()
					.child(div().truncate().child(playlist.title.clone()))
					.child(
						div()
							.text_size(px(11.))
							.text_color(tokens::fg_dim())
							.mt(px(2.))
							.child(rust_i18n::t!(
								"page.track_count",
								count = playlist.track_count
							)),
					),
			)
			.when(playing, |row| {
				row.child(widgets::icon(IconName::AudioLines, 14., tokens::red()))
			})
			.on_click(move |_, _, cx| {
				state.update(cx, |state, cx| {
					state.navigate(Route::Playlist(id.clone()), cx)
				})
			})
	});

	let recent_state = state.clone();
	let create_state = state.clone();
	let settings_state = state.clone();
	v_flex()
		.w(px(WIDTH))
		.h_full()
		.flex_none()
		.bg(tokens::panel())
		.border_r_1()
		.border_color(tokens::border())
		.px(px(10.))
		.py(px(12.))
		.child(
			Input::new(search)
				.prefix(widgets::icon(IconName::Search, 14., tokens::fg_dim()))
				.suffix(widgets::kbd("Ctrl K"))
				.cleanable(true),
		)
		.child(div().mt(px(10.)).child(nav_item(
			"nav-recent",
			IconName::Clock,
			rust_i18n::t!("sidebar.recent").into(),
			route == Some(Route::Recent),
			move |_, _, cx| recent_state.update(cx, |state, cx| state.navigate(Route::Recent, cx)),
		)))
		.child(
			h_flex()
				.justify_between()
				.px(px(10.))
				.pt(px(18.))
				.pb(px(8.))
				.child(widgets::section_label(rust_i18n::t!("sidebar.playlists")))
				.child(widgets::icon_button(
					"new-playlist",
					IconName::Plus,
					14.,
					false,
					move |_, window, cx| {
						let state = create_state.clone();
						crate::ui::dialogs::ask_name(
							rust_i18n::t!("sidebar.new_playlist").into(),
							String::new(),
							rust_i18n::t!("page.create").into(),
							move |title, cx| {
								state.update(cx, |state, cx| state.create_playlist(title, None, cx))
							},
							window,
							cx,
						);
					},
				)),
		)
		.child(
			v_flex()
				.id("playlists")
				.flex_1()
				.min_h_0()
				.overflow_y_scrollbar()
				.children(rows),
		)
		.child(
			div()
				.pt(px(8.))
				.border_t_1()
				.border_color(tokens::border())
				.child(nav_item(
					"nav-settings",
					IconName::Settings,
					rust_i18n::t!("sidebar.settings").into(),
					route == Some(Route::Settings),
					move |_, _, cx| {
						settings_state.update(cx, |state, cx| state.navigate(Route::Settings, cx))
					},
				)),
		)
}
