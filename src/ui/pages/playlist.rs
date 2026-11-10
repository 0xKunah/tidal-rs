//! Playlist page: header, play and shuffle, track table.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{Icon, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::model::{ContextRef, Playlist, PlaylistId, Track};
use crate::sort::{SortKey, TrackSort, sorted_indices};
use crate::ui::state::{AppState, COVER_LARGE, COVER_SMALL, Remote};
use crate::ui::track_table::{PageHeader, PlayAs, TableOptions, track_table};
use crate::ui::{dialogs, format, tokens, widgets};

/// Large header shared by playlist and album pages.
pub fn page_header(
	cover: Option<std::path::PathBuf>,
	kind: SharedString,
	title: SharedString,
	meta: SharedString,
) -> Div {
	h_flex()
		.items_end()
		.gap(px(24.))
		.px(px(32.))
		.pt(px(28.))
		.pb(px(22.))
		.bg(linear_gradient(
			180.,
			linear_color_stop(rgba(0xbe123c38), 0.),
			linear_color_stop(rgba(0x09090b00), 1.),
		))
		.child(div().shadow_lg().child(widgets::cover(cover, 168., 10.)))
		.child(
			v_flex()
				.min_w_0()
				.child(
					div()
						.text_size(px(11.))
						.font_weight(FontWeight::SEMIBOLD)
						.text_color(tokens::fg_muted())
						.child(kind.to_uppercase()),
				)
				.child(
					div()
						.mt(px(6.))
						.mb(px(8.))
						.text_size(px(44.))
						.line_height(px(48.))
						.font_weight(FontWeight::BOLD)
						.truncate()
						.child(title),
				)
				.child(div().text_color(tokens::fg_muted()).child(meta)),
		)
}

/// Round red play button and the Shuffle button.
pub fn play_actions(
	state: &Entity<AppState>,
	kind: ContextRef,
	tracks: Vec<Track>,
	cx: &App,
) -> Div {
	let view = state.read(cx).player.view();
	let is_current = view.context.as_ref() == Some(&kind);
	let playing_here = is_current && view.playing;
	let shuffle_on = is_current && view.shuffle;

	let play_state = state.clone();
	let play_kind = kind.clone();
	let play_tracks = tracks.clone();
	let shuffle_state = state.clone();
	h_flex()
		.gap(px(10.))
		.px(px(32.))
		.pb(px(16.))
		.child(
			div()
				.id("page-play")
				.size(px(46.))
				.rounded_full()
				.bg(tokens::red())
				.flex()
				.items_center()
				.justify_center()
				.cursor_pointer()
				.shadow_md()
				.hover(|style| style.opacity(0.9))
				.child(widgets::icon(
					if playing_here {
						IconName::Pause
					} else {
						IconName::Play
					},
					20.,
					tokens::fg(),
				))
				.on_click(move |_, _, cx| {
					let kind = play_kind.clone();
					let tracks = play_tracks.clone();
					play_state.update(cx, |state, cx| {
						if is_current {
							state.dispatch(crate::player::Command::TogglePlay, cx);
						} else {
							state.play_list(kind, tracks, None, None, cx);
						}
					});
				}),
		)
		.child(
			h_flex()
				.id("page-shuffle")
				.h(px(36.))
				.px(px(14.))
				.gap(px(8.))
				.rounded(px(8.))
				.border_1()
				.cursor_pointer()
				.font_weight(FontWeight::MEDIUM)
				.map(|button| {
					if shuffle_on {
						button
							.border_color(rgba(0xdc262673))
							.bg(tokens::red_tint())
							.text_color(tokens::red())
					} else {
						button
							.border_color(tokens::border())
							.hover(|style| style.bg(tokens::muted()))
					}
				})
				.child(widgets::icon(
					IconName::Shuffle,
					14.,
					if shuffle_on {
						tokens::red()
					} else {
						tokens::fg()
					},
				))
				.child(rust_i18n::t!("page.shuffle"))
				.on_click(move |_, _, cx| {
					let kind = kind.clone();
					let tracks = tracks.clone();
					shuffle_state.update(cx, |state, cx| {
						state.play_list(kind, tracks, None, Some(true), cx)
					});
				}),
		)
}

pub fn message(text: impl Into<SharedString>) -> Div {
	div()
		.p(px(32.))
		.text_color(tokens::fg_muted())
		.child(text.into())
}

fn sort_label(key: SortKey) -> SharedString {
	match key {
		SortKey::Custom => rust_i18n::t!("sort.custom"),
		SortKey::Title => rust_i18n::t!("sort.title"),
		SortKey::Artist => rust_i18n::t!("sort.artist"),
		SortKey::Album => rust_i18n::t!("sort.album"),
		SortKey::Added => rust_i18n::t!("sort.added"),
		SortKey::Duration => rust_i18n::t!("sort.duration"),
	}
	.into()
}

/// "Sort" dropdown; picking the current key again flips the direction.
fn sort_menu(state: &Entity<AppState>, id: &PlaylistId, sort: TrackSort) -> impl IntoElement {
	let state = state.clone();
	let id = id.clone();
	let direction = match (sort.key, sort.descending) {
		(SortKey::Custom, _) => IconName::ArrowUpDown,
		(_, true) => IconName::ArrowDown,
		(_, false) => IconName::ArrowUp,
	};
	Button::new("playlist-sort")
		.icon(Icon::new(direction))
		.label(sort_label(sort.key))
		.ghost()
		.dropdown_menu(move |mut menu, _, _| {
			for key in [
				SortKey::Custom,
				SortKey::Title,
				SortKey::Artist,
				SortKey::Album,
				SortKey::Added,
				SortKey::Duration,
			] {
				let state = state.clone();
				let id = id.clone();
				menu = menu.item(
					PopupMenuItem::new(sort_label(key))
						.checked(sort.key == key)
						.on_click(move |_, _, cx| {
							let id = id.clone();
							state.update(cx, |state, cx| state.set_sort(id, key, cx));
						}),
				);
			}
			menu
		})
}

/// "..." button with Rename and Delete, for playlists the user owns.
fn more_menu(state: &Entity<AppState>, playlist: &Playlist) -> impl IntoElement {
	let state = state.clone();
	let playlist = playlist.clone();
	Button::new("playlist-more")
		.icon(Icon::new(IconName::Ellipsis))
		.ghost()
		.dropdown_menu(move |menu, _, _| {
			let rename_state = state.clone();
			let delete_state = state.clone();
			let rename = playlist.clone();
			let delete = playlist.clone();
			menu.item(
				PopupMenuItem::new(rust_i18n::t!("page.rename"))
					.icon(Icon::new(IconName::Pencil))
					.on_click(move |_, window, cx| {
						let state = rename_state.clone();
						let id = rename.id.clone();
						dialogs::ask_name(
							rust_i18n::t!("page.rename").into(),
							rename.title.clone(),
							rust_i18n::t!("page.save").into(),
							move |title, cx| {
								state
									.update(cx, |state, _| state.rename_playlist(id.clone(), title))
							},
							window,
							cx,
						);
					}),
			)
			.item(
				PopupMenuItem::new(rust_i18n::t!("page.delete"))
					.icon(Icon::new(IconName::Trash))
					.on_click(move |_, window, cx| {
						let state = delete_state.clone();
						let id = delete.id.clone();
						dialogs::confirm_delete(
							delete.title.clone(),
							move |cx| {
								state.update(cx, |state, cx| state.delete_playlist(id.clone(), cx))
							},
							window,
							cx,
						);
					}),
			)
		})
}

pub fn playlist_page(state: &Entity<AppState>, id: &PlaylistId, cx: &App) -> AnyElement {
	let app = state.read(cx);
	let Some(playlist) = app.playlist(id).cloned() else {
		return message(rust_i18n::t!("page.loading")).into_any_element();
	};
	let cover = app
		.cover(playlist.cover.as_deref(), COVER_LARGE)
		.or_else(|| app.cover(playlist.cover.as_deref(), COVER_SMALL));
	let meta: SharedString = format!(
		"{}, {}",
		rust_i18n::t!("page.track_count", count = playlist.track_count),
		format::total(playlist.duration)
	)
	.into();
	let kind = ContextRef::Playlist {
		id: playlist.id.clone(),
		title: playlist.title.clone(),
	};
	let header = page_header(
		cover.clone(),
		rust_i18n::t!("page.playlist").into(),
		playlist.title.clone().into(),
		meta.clone(),
	);
	let owned = playlist.owned.then(|| playlist.id.clone());
	let sort = app.sort_of(id);
	let Some(Remote::Ready(tracks)) = app.playlist_tracks.get(id) else {
		let status = match app.playlist_tracks.get(id) {
			Some(Remote::Failed(_)) => rust_i18n::t!("page.load_failed"),
			_ => rust_i18n::t!("page.loading"),
		};
		return v_flex()
			.size_full()
			.child(header)
			.child(message(status))
			.into_any_element();
	};
	let order = sorted_indices(tracks, sort);
	let tracks: Vec<Track> = order.iter().map(|&ix| tracks[ix].clone()).collect();
	let page: PageHeader = {
		let state = state.clone();
		let kind = kind.clone();
		let tracks = tracks.clone();
		let playlist = playlist.clone();
		Rc::new(move |_, cx: &mut App| {
			v_flex()
				.w_full()
				.child(page_header(
					cover.clone(),
					rust_i18n::t!("page.playlist").into(),
					playlist.title.clone().into(),
					meta.clone(),
				))
				.child(
					play_actions(&state, kind.clone(), tracks.clone(), cx)
						.child(sort_menu(&state, &playlist.id, sort))
						.when(playlist.owned, |actions| {
							actions.child(more_menu(&state, &playlist))
						}),
				)
				.into_any_element()
		})
	};
	v_flex()
		.size_full()
		.child(track_table(
			TableOptions {
				id: "playlist-tracks",
				play_as: PlayAs::List(kind),
				show_album: true,
				show_added: true,
				owned_playlist: owned,
				source_index: Some(Rc::new(order)),
				reorderable: playlist.owned && sort.key == SortKey::Custom,
				sort: Some((playlist.id.clone(), sort)),
			},
			tracks,
			state,
			Some(page),
			cx,
		))
		.into_any_element()
}
