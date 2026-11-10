//! "Recently played": tracks heard outside the user's playlists.

use std::rc::Rc;

use gpui_kit::component::v_flex;
use gpui_kit::*;

use crate::model::ContextRef;
use crate::ui::pages::playlist::message;
use crate::ui::state::AppState;
use crate::ui::track_table::{PageHeader, PlayAs, TableOptions, track_table};

pub fn recent_page(state: &Entity<AppState>, cx: &App) -> AnyElement {
	let tracks = state.read(cx).recent.tracks();
	let title = div()
		.px(px(32.))
		.pt(px(28.))
		.pb(px(18.))
		.text_size(px(32.))
		.font_weight(FontWeight::BOLD)
		.child(rust_i18n::t!("recent.title"));
	if tracks.is_empty() {
		return v_flex()
			.child(title)
			.child(message(rust_i18n::t!("recent.empty")))
			.into_any_element();
	}
	let page: PageHeader = Rc::new(|_, _| {
		div()
			.px(px(32.))
			.pt(px(28.))
			.pb(px(18.))
			.text_size(px(32.))
			.font_weight(FontWeight::BOLD)
			.child(rust_i18n::t!("recent.title"))
			.into_any_element()
	});
	v_flex()
		.size_full()
		.child(track_table(
			TableOptions {
				id: "recent-tracks",
				play_as: PlayAs::List(ContextRef::Recent),
				show_album: true,
				show_added: false,
				owned_playlist: None,
				source_index: None,
				reorderable: false,
				sort: None,
			},
			tracks,
			state,
			Some(page),
			cx,
		))
		.into_any_element()
}
