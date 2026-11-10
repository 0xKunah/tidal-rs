//! Search results: tracks, artists, albums.

use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;

use crate::model::Route;
use crate::ui::pages::artist::album_grid;
use crate::ui::pages::playlist::message;
use crate::ui::state::{AppState, COVER_SMALL};
use crate::ui::track_table::{PlayAs, TableOptions, track_list};
use crate::ui::{tokens, widgets};

fn section_title(text: impl Into<SharedString>) -> Div {
	div()
		.mt(px(28.))
		.mb(px(12.))
		.text_size(px(18.))
		.font_weight(FontWeight::BOLD)
		.child(text.into())
}

pub fn search_page(state: &Entity<AppState>, cx: &App) -> AnyElement {
	let app = state.read(cx);
	if app.search.query().is_empty() {
		return message(rust_i18n::t!("search.placeholder")).into_any_element();
	}
	let Some(results) = app.search.results().cloned() else {
		return message(rust_i18n::t!("page.loading")).into_any_element();
	};
	if results.tracks.is_empty() && results.artists.is_empty() && results.albums.is_empty() {
		return message(rust_i18n::t!("search.no_results")).into_any_element();
	}
	let mut page = v_flex().px(px(32.)).pt(px(8.)).pb(px(32.));
	if !results.tracks.is_empty() {
		page = page
			.child(section_title(rust_i18n::t!("search.tracks")))
			.child(track_list(
				TableOptions {
					id: "search-tracks",
					play_as: PlayAs::Single,
					show_album: true,
					show_added: false,
					owned_playlist: None,
					source_index: None,
					reorderable: false,
					sort: None,
				},
				results.tracks.clone(),
				state,
				cx,
			));
	}
	if !results.artists.is_empty() {
		let artists =
			h_flex()
				.flex_wrap()
				.gap(px(16.))
				.children(results.artists.iter().enumerate().map(|(ix, artist)| {
					let picture = app.cover(artist.picture.as_deref(), COVER_SMALL);
					let state = state.clone();
					let id = artist.id;
					v_flex()
						.id(("search-artist", ix))
						.w(px(140.))
						.p(px(8.))
						.gap(px(8.))
						.items_center()
						.rounded(px(10.))
						.cursor_pointer()
						.hover(|style| style.bg(tokens::muted()))
						.child(
							div()
								.rounded_full()
								.overflow_hidden()
								.child(widgets::cover(picture, 116., 58.)),
						)
						.child(
							div()
								.w_full()
								.truncate()
								.text_center()
								.font_weight(FontWeight::MEDIUM)
								.child(artist.name.clone()),
						)
						.on_click(move |_, _, cx| {
							state.update(cx, |state, cx| state.navigate(Route::Artist(id), cx))
						})
				}));
		page = page
			.child(section_title(rust_i18n::t!("search.artists")))
			.child(artists);
	}
	if !results.albums.is_empty() {
		page = page
			.child(section_title(rust_i18n::t!("search.albums")))
			.child(album_grid("search-albums", &results.albums, state, cx));
	}
	div()
		.id("search-page")
		.size_full()
		.overflow_y_scrollbar()
		.child(page)
		.into_any_element()
}
