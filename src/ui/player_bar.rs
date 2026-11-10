//! Bottom bar: now playing, transport controls, progress, quality, queue, volume.

use gpui_kit::assets::IconName;
use gpui_kit::component::menu::ContextMenuExt as _;
use gpui_kit::component::slider::{Slider, SliderState};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::player::{Command, Repeat};
use crate::ui::state::{AppState, COVER_SMALL};
use crate::ui::track_menu::{MenuTarget, track_menu};
use crate::ui::{format, tokens, widgets};

pub const HEIGHT: f32 = 80.;

/// Slider entities owned by the root view.
pub struct PlayerSliders {
	pub progress: Entity<SliderState>,
	pub volume: Entity<SliderState>,
	/// Position shown while the user drags the progress slider.
	pub seeking: Option<f32>,
}

fn command_button(
	state: &Entity<AppState>,
	id: &'static str,
	name: IconName,
	active: bool,
	command: Command,
) -> impl IntoElement {
	let state = state.clone();
	let button = widgets::icon_button(id, name, 18., active, move |_, _, cx| {
		let command = command.clone();
		state.update(cx, |state, cx| state.dispatch(command, cx));
	});
	// A dot under active toggles, like the mockup.
	div().relative().child(button).when(active, |wrapper| {
		wrapper.child(
			div()
				.absolute()
				.bottom(px(-2.))
				.left(px(14.))
				.size(px(4.))
				.rounded_full()
				.bg(tokens::red()),
		)
	})
}

pub fn player_bar(state: &Entity<AppState>, sliders: &PlayerSliders, cx: &App) -> impl IntoElement {
	let app = state.read(cx);
	let view = app.player.view();
	let now = view.now.clone();
	let duration = now
		.as_ref()
		.map(|item| item.track.duration)
		.unwrap_or_default();
	let position = match sliders.seeking {
		Some(fraction) => duration.mul_f32(fraction),
		None => view.position,
	};

	let now_playing = h_flex().min_w_0().children(now.map(|item| {
		let cover = app.cover(item.track.album.cover.as_deref(), COVER_SMALL);
		let target = MenuTarget {
			item: item.clone(),
			owned_row: None,
			queue_index: None,
		};
		let menu_state = state.clone();
		h_flex()
			.id("now-playing")
			.gap(px(12.))
			.min_w_0()
			.p(px(6.))
			.rounded(px(10.))
			.hover(|style| style.bg(tokens::muted()))
			.context_menu(move |menu, window, cx| {
				track_menu(menu, &target, &menu_state, window, cx)
			})
			.child(widgets::cover(cover, 52., 6.))
			.child(
				v_flex()
					.min_w_0()
					.child(
						div()
							.truncate()
							.font_weight(FontWeight::SEMIBOLD)
							.child(item.track.title.clone()),
					)
					.child(
						div()
							.truncate()
							.mt(px(3.))
							.text_size(px(12.))
							.text_color(tokens::fg_muted())
							.child(item.track.artist_names()),
					),
			)
	}));

	let play_state = state.clone();
	let play = div()
		.id("play")
		.size(px(36.))
		.rounded_full()
		.bg(tokens::fg())
		.flex()
		.items_center()
		.justify_center()
		.cursor_pointer()
		.hover(|style| style.opacity(0.9))
		.child(widgets::icon(
			if view.playing {
				IconName::Pause
			} else {
				IconName::Play
			},
			16.,
			rgb(0x09090b).into(),
		))
		.on_click(move |_, _, cx| {
			play_state.update(cx, |state, cx| state.dispatch(Command::TogglePlay, cx))
		});

	let repeat_icon = if view.repeat == Repeat::One {
		IconName::Repeat1
	} else {
		IconName::Repeat
	};
	let controls = v_flex()
		.items_center()
		.gap(px(4.))
		.child(
			h_flex()
				.gap(px(16.))
				.items_center()
				.child(command_button(
					state,
					"shuffle",
					IconName::Shuffle,
					view.shuffle,
					Command::SetShuffle(!view.shuffle),
				))
				.child(command_button(
					state,
					"previous",
					IconName::SkipBack,
					false,
					Command::Previous,
				))
				.child(play)
				.child(command_button(
					state,
					"next",
					IconName::SkipForward,
					false,
					Command::Next,
				))
				.child(command_button(
					state,
					"repeat",
					repeat_icon,
					view.repeat != Repeat::Off,
					Command::CycleRepeat,
				)),
		)
		.child(
			h_flex()
				.w_full()
				.max_w(px(520.))
				.gap(px(10.))
				.text_size(px(11.))
				.text_color(tokens::fg_dim())
				.child(
					div()
						.w(px(40.))
						.flex()
						.justify_end()
						.child(format::clock(position)),
				)
				.child(div().flex_1().child(Slider::new(&sliders.progress)))
				.child(div().w(px(40.)).child(format::clock(duration))),
		);

	let toggle_state = state.clone();
	let queue_open = app.queue_panel_open;
	let mute_state = state.clone();
	let volume = view.volume;
	h_flex()
		.h(px(HEIGHT))
		.w_full()
		.flex_none()
		.px(px(16.))
		.bg(tokens::panel())
		.border_t_1()
		.border_color(tokens::border())
		.child(div().flex_1().min_w_0().child(now_playing))
		.child(div().flex_1().child(controls))
		.child(
			h_flex()
				.flex_1()
				.justify_end()
				.gap(px(12.))
				.children(app.badge.clone().map(|badge| {
					div()
						.px(px(7.))
						.py(px(3.))
						.rounded(px(5.))
						.border_1()
						.border_color(tokens::border())
						.text_size(px(10.))
						.font_weight(FontWeight::SEMIBOLD)
						.text_color(tokens::fg_muted())
						.child(badge)
				}))
				.child(widgets::icon_button(
					"queue-toggle",
					IconName::ListMusic,
					18.,
					queue_open,
					move |_, _, cx| {
						toggle_state.update(cx, |state, cx| {
							state.queue_panel_open = !state.queue_panel_open;
							cx.notify();
						})
					},
				))
				.child(widgets::icon_button(
					"mute",
					if volume == 0. {
						IconName::VolumeX
					} else {
						IconName::Volume2
					},
					18.,
					false,
					move |_, _, cx| {
						mute_state.update(cx, |state, cx| {
							state.dispatch(Command::ToggleMute, cx);
						})
					},
				))
				.child(div().w(px(96.)).child(Slider::new(&sliders.volume))),
		)
}
