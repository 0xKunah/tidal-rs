//! Full-page lyrics: timed lines that follow playback, or the cover when there are none.

use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::lyrics::{Lyrics, LyricsState};
use crate::ui::state::{AppState, COVER_LARGE, COVER_SMALL};
use crate::ui::{tokens, widgets};

const COVER_SIZE: f32 = 380.;
/// Cover shown to the left of the lyrics.
const SIDE_COVER_SIZE: f32 = 340.;
/// Empty space above and below the lines so the first and last can be centred.
const EDGE_SPACE: f32 = 260.;
/// Share of the remaining distance covered each frame while easing to the active line.
const EASE: f32 = 0.12;

pub fn lyrics_view(state: &Entity<AppState>, scroll: &ScrollHandle, cx: &App) -> AnyElement {
	let app = state.read(cx);
	let view = app.player.view();
	let Some(now) = view.now.clone() else {
		return div().size_full().into_any_element();
	};
	let ready = match app.lyrics_state(now.track.id) {
		LyricsState::Ready(lyrics) => Some(lyrics),
		LyricsState::Loading | LyricsState::Missing => None,
	};
	let Some(lyrics) = ready else {
		let cover = app
			.cover(now.track.album.cover.as_deref(), COVER_LARGE)
			.or_else(|| app.cover(now.track.album.cover.as_deref(), COVER_SMALL));
		return h_flex()
			.size_full()
			.items_center()
			.justify_center()
			.child(widgets::cover(cover, COVER_SIZE, 12.))
			.into_any_element();
	};
	let body = if lyrics.synced.is_empty() {
		plain(&lyrics, scroll)
	} else {
		synced(state, &lyrics, view.position, scroll)
	};
	let cover = app
		.cover(now.track.album.cover.as_deref(), COVER_LARGE)
		.or_else(|| app.cover(now.track.album.cover.as_deref(), COVER_SMALL));
	h_flex()
		.size_full()
		.child(
			div()
				.h_full()
				.flex_none()
				.flex()
				.items_center()
				.justify_center()
				.px(px(48.))
				.child(widgets::cover(cover, SIDE_COVER_SIZE, 12.)),
		)
		.child(div().flex_1().h_full().min_w_0().child(body))
		.into_any_element()
}

fn plain(lyrics: &Lyrics, scroll: &ScrollHandle) -> AnyElement {
	let text = lyrics.plain.clone().unwrap_or_default();
	div()
		.id("lyrics-plain")
		.size_full()
		.overflow_y_scroll()
		.track_scroll(scroll)
		.child(
			v_flex()
				.w_full()
				.items_center()
				.py(px(EDGE_SPACE / 2.))
				.child(
					div()
						.w_full()
						.max_w(px(900.))
						.px(px(32.))
						.text_size(px(26.))
						.font_weight(FontWeight::BOLD)
						.text_color(tokens::fg_dim())
						.child(text),
				),
		)
		.into_any_element()
}

fn synced(
	state: &Entity<AppState>,
	lyrics: &Lyrics,
	position: std::time::Duration,
	scroll: &ScrollHandle,
) -> AnyElement {
	let active = lyrics.active_index(position);
	let lines = lyrics
		.synced
		.iter()
		.enumerate()
		.map(|(index, (time, text))| {
			let time = *time;
			let seek_state = state.clone();
			div()
				.id(("lyric-line", index))
				.flex_none()
				.w_full()
				.max_w(px(900.))
				.px(px(32.))
				.py(px(10.))
				.text_size(px(30.))
				.font_weight(FontWeight::BOLD)
				.cursor_pointer()
				.text_color(if Some(index) == active {
					tokens::fg()
				} else {
					tokens::fg_dim()
				})
				.when(text.is_empty(), |line| line.h(px(40.)))
				.child(text.clone())
				.on_click(move |_, _, cx| {
					seek_state.update(cx, |state, cx| state.seek_to(time, cx));
				})
		});

	let follow = scroll.clone();
	// Child 0 is the top spacer, so line `i` is child `i + 1`.
	let target = active.map(|index| index + 1);
	div()
		.size_full()
		.relative()
		.child(
			div()
				.id("lyrics-synced")
				.size_full()
				.flex()
				.flex_col()
				.items_center()
				.overflow_y_scroll()
				.track_scroll(scroll)
				// Lines must be direct children: the scroll handle indexes them.
				.child(div().h(px(EDGE_SPACE)).flex_none())
				.children(lines)
				.child(div().h(px(EDGE_SPACE)).flex_none()),
		)
		.child(
			// Eases the scroll offset toward the active line, one step per frame.
			canvas(
				|_, _, _| (),
				move |_, _, window, _| {
					let Some(line) = target.and_then(|ix| follow.bounds_for_item(ix)) else {
						return;
					};
					let delta = follow.bounds().center().y - line.center().y;
					if delta.abs() < px(0.5) {
						return;
					}
					let offset = follow.offset();
					follow.set_offset(point(offset.x, offset.y + delta * EASE));
					window.request_animation_frame();
				},
			)
			.absolute()
			.size_0(),
		)
		.into_any_element()
}
