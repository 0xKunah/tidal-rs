//! Small modal dialogs: playlist name, delete confirmation.

use std::rc::Rc;

use gpui_kit::component::dialog::AlertDialog;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::model::{PlaylistId, TrackId};
use crate::ui::state::AppState;
use crate::ui::tokens;

fn dialog_button(
	id: &'static str,
	label: SharedString,
	primary: bool,
	on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
	let base = div()
		.id(id)
		.h(px(34.))
		.px(px(14.))
		.flex()
		.items_center()
		.rounded(px(8.))
		.cursor_pointer()
		.font_weight(FontWeight::MEDIUM)
		.child(label)
		.on_click(on_click);
	if primary {
		base.bg(tokens::red()).hover(|style| style.opacity(0.9))
	} else {
		base.border_1()
			.border_color(tokens::border())
			.hover(|style| style.bg(tokens::muted()))
	}
}

/// Asks for a name; `on_submit` receives the trimmed, non-empty value.
pub fn ask_name(
	title: SharedString,
	initial: String,
	ok_label: SharedString,
	on_submit: impl Fn(String, &mut App) + 'static,
	window: &mut Window,
	cx: &mut App,
) {
	let input = cx.new(|cx| {
		InputState::new(window, cx)
			.placeholder(rust_i18n::t!("page.name"))
			.default_value(initial)
	});
	input.update(cx, |input, cx| input.focus(window, cx));
	let on_submit = Rc::new(on_submit);
	let submit = {
		let input = input.clone();
		let on_submit = on_submit.clone();
		Rc::new(move |window: &mut Window, cx: &mut App| {
			let value = input.read(cx).value().trim().to_string();
			if value.is_empty() {
				return;
			}
			on_submit(value, cx);
			window.close_dialog(cx);
		})
	};
	window.open_dialog(cx, move |dialog, _, _| {
		let ok = submit.clone();
		let enter = submit.clone();
		dialog
			.w(px(420.))
			.title(title.clone())
			.child(Input::new(&input))
			.on_ok(move |_, window, cx| {
				enter(window, cx);
				false
			})
			.footer(
				h_flex()
					.justify_end()
					.gap(px(8.))
					.child(dialog_button(
						"dialog-cancel",
						rust_i18n::t!("page.cancel").into(),
						false,
						|_, window, cx| window.close_dialog(cx),
					))
					.child(dialog_button(
						"dialog-ok",
						ok_label.clone(),
						true,
						move |_, window, cx| ok(window, cx),
					)),
			)
	});
}

/// Picks one of the user's playlists for `track`, with a filter field.
pub fn pick_playlist(state: Entity<AppState>, track: TrackId, window: &mut Window, cx: &mut App) {
	let input = cx
		.new(|cx| InputState::new(window, cx).placeholder(rust_i18n::t!("menu.filter_playlists")));
	input.update(cx, |input, cx| input.focus(window, cx));
	window.open_dialog(cx, move |dialog, _, cx| {
		let query = input.read(cx).value().to_lowercase();
		let app = state.read(cx);
		let rows: Vec<(PlaylistId, String, bool)> = app
			.playlists
			.ready()
			.map(|playlists| {
				playlists
					.iter()
					.filter(|playlist| playlist.owned)
					.filter(|playlist| playlist.title.to_lowercase().contains(&query))
					.map(|playlist| {
						(
							playlist.id.clone(),
							playlist.title.clone(),
							app.index.contains(&playlist.id, track),
						)
					})
					.collect()
			})
			.unwrap_or_default();
		let list = v_flex()
			.id("pick-playlist-list")
			.max_h(px(360.))
			.overflow_y_scrollbar()
			.children(
				rows.into_iter()
					.enumerate()
					.map(|(ix, (id, title, already))| {
						let state = state.clone();
						let label = title.clone();
						h_flex()
							.id(("pick-playlist", ix))
							.h(px(36.))
							.px(px(10.))
							.justify_between()
							.rounded(px(6.))
							.when(!already, |row| {
								row.cursor_pointer()
									.hover(|style| style.bg(tokens::muted()))
									.on_click(move |_, window, cx| {
										let (id, title) = (id.clone(), title.clone());
										state.update(cx, |state, cx| {
											state.add_to_playlist(id, &title, track, cx)
										});
										window.close_dialog(cx);
									})
							})
							.child(div().truncate().child(label))
							.when(already, |row| {
								row.text_color(tokens::fg_dim()).child(
									div()
										.text_size(px(11.))
										.child(rust_i18n::t!("menu.already_in")),
								)
							})
					}),
			);
		dialog
			.w(px(420.))
			.title(rust_i18n::t!("menu.add_to_playlist").to_string())
			.child(Input::new(&input))
			.child(div().mt(px(8.)).child(list))
	});
}

/// Asks before deleting; `on_confirm` runs only on confirmation.
pub fn confirm_delete(
	name: String,
	on_confirm: impl Fn(&mut App) + 'static,
	window: &mut Window,
	cx: &mut App,
) {
	let on_confirm = Rc::new(on_confirm);
	window.open_alert_dialog(cx, move |dialog: AlertDialog, _, _| {
		let on_confirm = on_confirm.clone();
		dialog
			.title(rust_i18n::t!("page.delete_confirm", name = name.clone()).to_string())
			.ok_text(rust_i18n::t!("page.delete").to_string())
			.cancel_text(rust_i18n::t!("page.cancel").to_string())
			.show_cancel(true)
			.on_ok(move |_, _, cx| {
				on_confirm(cx);
				true
			})
	});
}
