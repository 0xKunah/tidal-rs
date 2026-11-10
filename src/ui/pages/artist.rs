//! Artist page: popular tracks, albums, singles and EPs. Nothing else.

use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;

use crate::model::{Album, ArtistId, ContextRef, Route};
use crate::ui::pages::playlist::{message, page_header, play_actions};
use crate::ui::state::{AppState, COVER_LARGE, COVER_SMALL, Remote};
use crate::ui::track_table::{PlayAs, TableOptions, track_list};
use crate::ui::{tokens, widgets};

const CARD_WIDTH: f32 = 168.;

fn section_title(text: impl Into<SharedString>) -> Div {
	div()
		.mt(px(28.))
		.mb(px(12.))
		.text_size(px(18.))
		.font_weight(FontWeight::BOLD)
		.child(text.into())
}

/// Grid of album cards; a click opens the album.
pub fn album_grid(
	id: &'static str,
	albums: &[Album],
	state: &Entity<AppState>,
	cx: &App,
) -> impl IntoElement {
	let app = state.read(cx);
	h_flex()
		.flex_wrap()
		.gap(px(16.))
		.children(albums.iter().enumerate().map(|(ix, album)| {
			let cover = app.cover(album.cover.as_deref(), COVER_SMALL);
			let state = state.clone();
			let album_id = album.id;
			v_flex()
				.id((id, ix))
				.w(px(CARD_WIDTH))
				.p(px(8.))
				.gap(px(8.))
				.rounded(px(10.))
				.cursor_pointer()
				.hover(|style| style.bg(tokens::muted()))
				.child(widgets::cover(cover, CARD_WIDTH - 16., 8.))
				.child(
					div()
						.truncate()
						.font_weight(FontWeight::MEDIUM)
						.child(album.title.clone()),
				)
				.child(
					div()
						.text_size(px(12.))
						.text_color(tokens::fg_muted())
						.child(album.year.map(|year| year.to_string()).unwrap_or_default()),
				)
				.on_click(move |_, _, cx| {
					state.update(cx, |state, cx| state.navigate(Route::Album(album_id), cx))
				})
		}))
}

pub fn artist_page(state: &Entity<AppState>, id: ArtistId, cx: &App) -> AnyElement {
	let app = state.read(cx);
	let page = match app.artists.get(&id) {
		Some(Remote::Ready(page)) => page.clone(),
		Some(Remote::Failed(_)) => {
			return message(rust_i18n::t!("page.load_failed")).into_any_element();
		}
		_ => return message(rust_i18n::t!("page.loading")).into_any_element(),
	};
	let picture = app
		.cover(page.artist.picture.as_deref(), COVER_LARGE)
		.or_else(|| app.cover(page.artist.picture.as_deref(), COVER_SMALL));
	let kind = ContextRef::ArtistTop {
		id,
		name: page.artist.name.clone(),
	};
	let mut body = v_flex().px(px(32.)).pb(px(32.));
	if !page.top_tracks.is_empty() {
		body = body
			.child(section_title(rust_i18n::t!("page.popular")))
			.child(track_list(
				TableOptions {
					id: "artist-top",
					play_as: PlayAs::List(kind.clone()),
					show_album: true,
					show_added: false,
					owned_playlist: None,
					source_index: None,
					reorderable: false,
					sort: None,
				},
				page.top_tracks.clone(),
				state,
				cx,
			));
	}
	if !page.albums.is_empty() {
		body = body
			.child(section_title(rust_i18n::t!("page.albums")))
			.child(album_grid("artist-albums", &page.albums, state, cx));
	}
	if !page.singles.is_empty() {
		body = body
			.child(section_title(rust_i18n::t!("page.singles")))
			.child(album_grid("artist-singles", &page.singles, state, cx));
	}
	v_flex()
		.id("artist-page")
		.size_full()
		.overflow_y_scrollbar()
		.child(page_header(
			picture,
			rust_i18n::t!("page.artist").into(),
			page.artist.name.clone().into(),
			SharedString::default(),
		))
		.child(play_actions(state, kind, page.top_tracks.clone(), cx))
		.child(body)
		.into_any_element()
}
