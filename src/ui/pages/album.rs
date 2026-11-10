//! Album page: header, play and shuffle, track table.

use std::rc::Rc;

use gpui_kit::component::v_flex;
use gpui_kit::*;

use crate::model::{AlbumId, ContextRef};
use crate::ui::format;
use crate::ui::pages::playlist::{message, page_header, play_actions};
use crate::ui::state::{AppState, COVER_LARGE, COVER_SMALL, Remote};
use crate::ui::track_table::{PageHeader, PlayAs, TableOptions, track_table};

pub fn album_page(state: &Entity<AppState>, id: AlbumId, cx: &App) -> AnyElement {
	let app = state.read(cx);
	let (album, tracks) = match app.albums.get(&id) {
		Some(Remote::Ready((album, tracks))) => (album.clone(), tracks.clone()),
		Some(Remote::Failed(_)) => {
			return message(rust_i18n::t!("page.load_failed")).into_any_element();
		}
		_ => return message(rust_i18n::t!("page.loading")).into_any_element(),
	};
	let cover = app
		.cover(album.cover.as_deref(), COVER_LARGE)
		.or_else(|| app.cover(album.cover.as_deref(), COVER_SMALL));
	let artists = album
		.artists
		.iter()
		.map(|artist| artist.name.as_str())
		.collect::<Vec<_>>()
		.join(", ");
	let mut meta = vec![artists];
	if let Some(year) = album.year {
		meta.push(year.to_string());
	}
	meta.push(rust_i18n::t!("page.track_count", count = album.track_count).to_string());
	meta.push(format::total(album.duration));
	let kind = ContextRef::Album {
		id,
		title: album.title.clone(),
	};
	let page: PageHeader = {
		let state = state.clone();
		let kind = kind.clone();
		let tracks = tracks.clone();
		let title: SharedString = album.title.clone().into();
		let meta: SharedString = meta.join(", ").into();
		Rc::new(move |_, cx: &mut App| {
			v_flex()
				.w_full()
				.child(page_header(
					cover.clone(),
					rust_i18n::t!("page.album").into(),
					title.clone(),
					meta.clone(),
				))
				.child(play_actions(&state, kind.clone(), tracks.clone(), cx))
				.into_any_element()
		})
	};
	v_flex()
		.size_full()
		.child(track_table(
			TableOptions {
				id: "album-tracks",
				play_as: PlayAs::List(kind),
				show_album: false,
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
