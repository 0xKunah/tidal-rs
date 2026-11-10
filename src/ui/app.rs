//! Root view: title bar, sidebar, page, queue panel, player bar, toasts, keyboard.

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::slider::{SliderEvent, SliderState};
use gpui_kit::component::{TitleBar, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::model::Route;
use crate::player::Command;
use crate::ui::pages::{album, artist, login, playlist, recent, search, settings};
use crate::ui::player_bar::PlayerSliders;
use crate::ui::state::AppState;
use crate::ui::{player_bar, queue_panel, sidebar, tokens, widgets};

gpui_kit::actions!(tidal_rs, [TogglePlay, FocusSearch, QueueSelected]);

/// Space, Ctrl K and Q. Space and Q only apply outside text inputs.
pub fn key_bindings() -> Vec<KeyBinding> {
	vec![
		KeyBinding::new("space", TogglePlay, Some("AppView")),
		KeyBinding::new("ctrl-k", FocusSearch, None),
		KeyBinding::new("q", QueueSelected, Some("AppView")),
	]
}

pub struct AppView {
	state: Entity<AppState>,
	search: Entity<InputState>,
	/// Paste field of the browser (PKCE) login.
	pkce_input: Entity<InputState>,
	sliders: PlayerSliders,
	/// Locale the input placeholders were last set in.
	placeholder_locale: String,
	focus: FocusHandle,
	_subscriptions: Vec<Subscription>,
}

impl AppView {
	pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
		let search =
			cx.new(|cx| InputState::new(window, cx).placeholder(rust_i18n::t!("sidebar.search")));
		let volume_value = state.read(cx).player.view().volume;
		let progress = cx.new(|_| SliderState::new().min(0.).max(1.).step(0.001));
		let volume = cx.new(|_| {
			SliderState::new()
				.min(0.)
				.max(1.)
				.step(0.01)
				.default_value(volume_value)
		});
		let seek_state = state.clone();
		let search_state = state.clone();
		let pkce_input = cx.new(|cx| {
			InputState::new(window, cx).placeholder(rust_i18n::t!("login.paste_placeholder"))
		});
		let pkce_state = state.clone();
		let volume_state = state.clone();
		let subscriptions = vec![
			cx.subscribe_in(
				&pkce_input,
				window,
				move |_, input, event: &InputEvent, _, cx| {
					if matches!(event, InputEvent::PressEnter { .. }) {
						let address = input.read(cx).value().to_string();
						pkce_state.update(cx, |state, cx| state.complete_pkce(address, cx));
					}
				},
			),
			cx.subscribe_in(
				&search,
				window,
				move |_, input, event: &InputEvent, _, cx| {
					if matches!(event, InputEvent::Change | InputEvent::PressEnter { .. }) {
						let query = input.read(cx).value().to_string();
						search_state.update(cx, |state, cx| state.set_search_query(&query, cx));
					}
				},
			),
			cx.observe_in(&state, window, |this, _, window, cx| {
				this.sync_sliders(window, cx);
				this.sync_placeholders(window, cx);
				cx.notify();
			}),
			cx.subscribe_in(
				&progress,
				window,
				move |this, _, event: &SliderEvent, _, cx| match event {
					SliderEvent::Change(value) => {
						this.sliders.seeking = Some(value.start());
						cx.notify();
					}
					SliderEvent::Release(value) => {
						this.sliders.seeking = None;
						let fraction = value.start();
						seek_state.update(cx, |state, cx| {
							if let Some(now) = state.player.view().now {
								let position = now.track.duration.mul_f32(fraction);
								state.dispatch(Command::Seek(position), cx);
							}
						});
					}
				},
			),
			cx.subscribe_in(&volume, window, move |_, _, event: &SliderEvent, _, cx| {
				let (SliderEvent::Change(value) | SliderEvent::Release(value)) = event;
				let value = value.start();
				volume_state.update(cx, |state, cx| {
					state.dispatch(Command::SetVolume(value), cx)
				});
			}),
		];
		let focus = cx.focus_handle();
		window.focus(&focus, cx);
		#[cfg(windows)]
		{
			use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
			if let Ok(handle) = window.window_handle()
				&& let RawWindowHandle::Win32(win32) = handle.as_raw()
			{
				let hwnd = win32.hwnd.get();
				state.update(cx, |state, cx| state.attach_media(hwnd, cx));
			}
		}
		Self {
			state,
			search,
			pkce_input: pkce_input.clone(),
			placeholder_locale: rust_i18n::locale().to_string(),
			sliders: PlayerSliders {
				progress,
				volume,
				seeking: None,
			},
			focus,
			_subscriptions: subscriptions,
		}
	}

	/// Re-translates the input placeholders after a language change.
	fn sync_placeholders(&mut self, window: &mut Window, cx: &mut Context<Self>) {
		let locale = rust_i18n::locale().to_string();
		if locale == self.placeholder_locale {
			return;
		}
		self.placeholder_locale = locale;
		self.search.update(cx, |input, cx| {
			input.set_placeholder(rust_i18n::t!("sidebar.search"), window, cx)
		});
		self.pkce_input.update(cx, |input, cx| {
			input.set_placeholder(rust_i18n::t!("login.paste_placeholder"), window, cx)
		});
	}

	/// Mirrors playback position and volume into the sliders, except while dragging.
	fn sync_sliders(&mut self, window: &mut Window, cx: &mut Context<Self>) {
		let view = self.state.read(cx).player.view();
		if self.sliders.seeking.is_none() {
			let duration = view
				.now
				.as_ref()
				.map(|item| item.track.duration.as_secs_f32())
				.unwrap_or(0.);
			let fraction = if duration > 0. {
				(view.position.as_secs_f32() / duration).clamp(0., 1.)
			} else {
				0.
			};
			self.sliders
				.progress
				.update(cx, |slider, cx| slider.set_value(fraction, window, cx));
		}
		let volume = view.volume;
		self.sliders.volume.update(cx, |slider, cx| {
			if (slider.value().start() - volume).abs() > f32::EPSILON {
				slider.set_value(volume, window, cx);
			}
		});
	}

	fn toggle_play(&mut self, _: &TogglePlay, _: &mut Window, cx: &mut Context<Self>) {
		self.state
			.update(cx, |state, cx| state.dispatch(Command::TogglePlay, cx));
	}

	fn focus_search(&mut self, _: &FocusSearch, window: &mut Window, cx: &mut Context<Self>) {
		self.search.update(cx, |input, cx| input.focus(window, cx));
	}

	fn queue_selected(&mut self, _: &QueueSelected, _: &mut Window, cx: &mut Context<Self>) {
		self.state.update(cx, |state, cx| {
			if let Some(item) = state.selected.clone() {
				state.dispatch(Command::AddToQueue(item), cx);
			}
		});
	}

	fn page(&self, cx: &App) -> AnyElement {
		let route = self.state.read(cx).route.clone();
		match route {
			Some(Route::Playlist(id)) => playlist::playlist_page(&self.state, &id, cx),
			Some(Route::Album(id)) => album::album_page(&self.state, id, cx),
			Some(Route::Artist(id)) => artist::artist_page(&self.state, id, cx),
			Some(Route::Search) => search::search_page(&self.state, cx),
			Some(Route::Recent) => recent::recent_page(&self.state, cx),
			Some(Route::Settings) => settings::settings_page(&self.state, cx),
			None if self
				.state
				.read(cx)
				.playlists
				.ready()
				.is_some_and(|playlists| playlists.is_empty()) =>
			{
				self.empty_state()
			}
			None => playlist::message(rust_i18n::t!("page.loading")).into_any_element(),
		}
	}

	/// No playlist yet: invite to create one.
	fn empty_state(&self) -> AnyElement {
		let state = self.state.clone();
		v_flex()
			.size_full()
			.items_center()
			.justify_center()
			.gap(px(16.))
			.child(
				div()
					.text_size(px(20.))
					.font_weight(FontWeight::BOLD)
					.child(rust_i18n::t!("page.empty_title")),
			)
			.child(
				div()
					.id("empty-create")
					.h(px(36.))
					.px(px(16.))
					.flex()
					.items_center()
					.rounded(px(8.))
					.bg(tokens::red())
					.cursor_pointer()
					.font_weight(FontWeight::MEDIUM)
					.hover(|style| style.opacity(0.9))
					.child(rust_i18n::t!("page.empty_action"))
					.on_click(move |_, window, cx| {
						let state = state.clone();
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
					}),
			)
			.into_any_element()
	}

	fn toasts(&self, cx: &App) -> impl IntoElement {
		let toasts = self.state.read(cx).toasts.clone();
		v_flex()
			.absolute()
			.right(px(20.))
			.bottom(px(96.))
			.gap(px(8.))
			.children(toasts.into_iter().map(|toast| {
				div()
					.px(px(14.))
					.py(px(10.))
					.rounded(px(8.))
					.bg(rgb(0x0f0f11))
					.border_1()
					.border_color(tokens::border())
					.shadow_lg()
					.child(toast.text)
			}))
	}
}

impl Render for AppView {
	fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
		let app = self.state.read(cx);
		let logged_in = app.is_logged_in();
		let queue_open = app.queue_panel_open;
		let toggle_state = self.state.clone();
		let title = TitleBar::new().child(
			h_flex()
				.w_full()
				.justify_between()
				.pr(px(8.))
				.child(
					h_flex()
						.gap_2()
						.text_xs()
						.text_color(tokens::fg_dim())
						.child(div().size_3p5().rounded_sm().bg(tokens::red()))
						.child(rust_i18n::t!("app.name")),
				)
				.when(logged_in, |bar| {
					bar.child(
						widgets::icon_button(
							"title-queue-toggle",
							IconName::ListMusic,
							14.,
							queue_open,
							move |_, _, cx| {
								toggle_state.update(cx, |state, cx| {
									state.queue_panel_open = !state.queue_panel_open;
									cx.notify();
								})
							},
						)
						// The title bar is a drag area: block it so Windows delivers the click here.
						.occlude(),
					)
				}),
		);
		let focus = self.focus.clone();
		let refocus = move |_: &MouseDownEvent, window: &mut Window, cx: &mut App| {
			window.focus(&focus, cx);
		};
		let body = if logged_in {
			v_flex()
				.flex_1()
				.min_h_0()
				.child(
					h_flex()
						.flex_1()
						.min_h_0()
						.items_start()
						.child(sidebar::sidebar(&self.state, &self.search, cx))
						.child(
							div()
								.flex_1()
								.h_full()
								.min_w_0()
								.on_mouse_down(MouseButton::Left, refocus.clone())
								.child(self.page(cx)),
						)
						.when(queue_open, |row| {
							row.child(queue_panel::queue_panel(&self.state, cx))
						}),
				)
				.child(
					div()
						.on_mouse_down(MouseButton::Left, refocus)
						.child(player_bar::player_bar(&self.state, &self.sliders, cx)),
				)
				.into_any_element()
		} else {
			div()
				.flex_1()
				.child(login::login_page(&self.state, &self.pkce_input, cx))
				.into_any_element()
		};
		v_flex()
			.size_full()
			.relative()
			.key_context("AppView")
			.track_focus(&self.focus)
			.on_action(cx.listener(Self::toggle_play))
			.on_action(cx.listener(Self::focus_search))
			.on_action(cx.listener(Self::queue_selected))
			.bg(rgb(0x09090b))
			.text_color(tokens::fg())
			.text_size(px(13.))
			.child(title)
			.child(body)
			.child(self.toasts(cx))
	}
}
