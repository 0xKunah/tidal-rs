//! Login screen: device code flow, missing credentials, errors.

use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::ui::state::{AppState, LoginState};
use crate::ui::tokens;

fn button(
	id: &'static str,
	label: SharedString,
	primary: bool,
	on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
	div()
		.id(id)
		.h(px(36.))
		.px(px(16.))
		.flex()
		.items_center()
		.justify_center()
		.rounded(px(8.))
		.cursor_pointer()
		.font_weight(FontWeight::MEDIUM)
		.map(|button| {
			if primary {
				button
					.bg(tokens::red())
					.text_color(tokens::fg())
					.hover(|style| style.opacity(0.9))
			} else {
				button
					.border_1()
					.border_color(tokens::border())
					.hover(|style| style.bg(tokens::muted()))
			}
		})
		.child(label)
		.on_click(on_click)
}

fn muted_text(text: impl Into<SharedString>) -> Div {
	div()
		.text_color(tokens::fg_muted())
		.text_center()
		.child(text.into())
}

pub fn login_page(
	state: &Entity<AppState>,
	pkce_input: &Entity<InputState>,
	cx: &App,
) -> impl IntoElement {
	let login = state.read(cx).login.clone();
	let retry_state = state.clone();
	let retry = move || {
		let state = retry_state.clone();
		button(
			"retry",
			rust_i18n::t!("login.retry").into(),
			false,
			move |_, _, cx| state.update(cx, |state, cx| state.retry_connection(cx)),
		)
	};
	let body = match login {
		LoginState::Connecting | LoginState::LoggedIn { .. } => {
			v_flex().child(muted_text(rust_i18n::t!("page.loading")))
		}
		LoginState::Code(code) => {
			let link = code.verification_uri.clone();
			let link = if link.starts_with("http") {
				link
			} else {
				format!("https://{link}")
			};
			let user_code = code.user_code.clone();
			v_flex()
				.items_center()
				.gap(px(18.))
				.child(muted_text(rust_i18n::t!("login.instructions")))
				.child(
					div()
						.px(px(24.))
						.py(px(14.))
						.rounded(px(10.))
						.bg(tokens::muted())
						.border_1()
						.border_color(tokens::border())
						.text_size(px(32.))
						.font_weight(FontWeight::BOLD)
						.child(code.user_code.clone()),
				)
				.child(
					gpui_kit::component::h_flex()
						.gap(px(10.))
						.child(button(
							"open-link",
							rust_i18n::t!("login.open").into(),
							true,
							move |_, _, _| {
								let _ = open::that(&link);
							},
						))
						.child(button(
							"copy-code",
							rust_i18n::t!("login.copy").into(),
							false,
							move |_, _, cx| {
								cx.write_to_clipboard(ClipboardItem::new_string(user_code.clone()))
							},
						)),
				)
				.child(
					div()
						.text_size(px(12.))
						.text_color(tokens::fg_dim())
						.child(rust_i18n::t!("login.waiting")),
				)
		}
		LoginState::Pkce { url, error } => {
			let step =
				|text: SharedString| div().w_full().text_color(tokens::fg_muted()).child(text);
			let connect_state = state.clone();
			let connect_input = pkce_input.clone();
			v_flex()
				.w(px(520.))
				.gap(px(14.))
				.child(step(rust_i18n::t!("login.pkce_step1").into()))
				.child(button(
					"open-pkce",
					rust_i18n::t!("login.open_pkce").into(),
					true,
					move |_, _, _| {
						let _ = open::that(&url);
					},
				))
				.child(step(rust_i18n::t!("login.pkce_step2").into()))
				.child(step(rust_i18n::t!("login.pkce_step3").into()))
				.child(
					h_flex()
						.w_full()
						.gap(px(10.))
						.child(div().flex_1().child(Input::new(pkce_input)))
						.child(button(
							"pkce-connect",
							rust_i18n::t!("login.connect").into(),
							false,
							move |_, _, cx| {
								let address = connect_input.read(cx).value().to_string();
								connect_state
									.update(cx, |state, cx| state.complete_pkce(address, cx));
							},
						)),
				)
				.children(error.map(|error| {
					div()
						.text_size(px(12.))
						.text_color(tokens::red())
						.child(error)
				}))
		}
		LoginState::MissingCredentials(path) => v_flex()
			.items_center()
			.gap(px(16.))
			.child(muted_text(rust_i18n::t!(
				"login.missing_client",
				path = path.display().to_string()
			)))
			.child(retry()),
		LoginState::Failed(error) => v_flex()
			.items_center()
			.gap(px(16.))
			.child(muted_text(rust_i18n::t!("login.failed", error = error)))
			.child(retry()),
	};
	v_flex()
		.size_full()
		.items_center()
		.justify_center()
		.gap(px(24.))
		.child(
			div()
				.text_size(px(24.))
				.font_weight(FontWeight::BOLD)
				.child(rust_i18n::t!("login.title")),
		)
		.child(body.max_w(px(520.)))
}
