//! The track context menu, identical everywhere a track is shown.

use gpui_kit::assets::IconName;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::{Icon, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::model::{PlaylistId, QueueItem, Route};
use crate::player::Command;
use crate::ui::state::AppState;
use crate::ui::{dialogs, tokens};

/// The track a menu acts on, and where it is shown.
#[derive(Clone)]
pub struct MenuTarget {
	pub item: QueueItem,
	/// Set on rows of a playlist the user owns: (playlist, row index).
	pub owned_row: Option<(PlaylistId, usize)>,
	/// Set on rows of the queue's "Added by you" section.
	pub queue_index: Option<usize>,
}

fn with_hint(label: SharedString, hint: &'static str) -> impl Fn(&mut Window, &mut App) -> Div {
	move |_, _| {
		h_flex()
			.w_full()
			.justify_between()
			.gap(px(16.))
			.child(label.clone())
			.child(
				div()
					.text_size(px(11.))
					.text_color(tokens::fg_dim())
					.child(hint),
			)
	}
}

pub fn track_menu(
	menu: PopupMenu,
	target: &MenuTarget,
	state: &Entity<AppState>,
	window: &mut Window,
	cx: &mut Context<PopupMenu>,
) -> PopupMenu {
	let item = target.item.clone();
	let track = item.track.clone();
	let app = state.read(cx);
	let owned: Vec<(PlaylistId, String, bool)> = app
		.playlists
		.ready()
		.map(|playlists| {
			playlists
				.iter()
				.filter(|playlist| playlist.owned)
				.map(|playlist| {
					(
						playlist.id.clone(),
						playlist.title.clone(),
						app.index.contains(&playlist.id, track.id),
					)
				})
				.collect()
		})
		.unwrap_or_default();

	let queue_state = state.clone();
	let queue_item = item.clone();
	let mut menu = menu.min_w(px(236.)).item(
		PopupMenuItem::element(with_hint(rust_i18n::t!("menu.add_to_queue").into(), "Q"))
			.icon(Icon::new(IconName::ListPlus))
			.on_click(move |_, _, cx| {
				let item = queue_item.clone();
				queue_state.update(cx, |state, cx| {
					state.dispatch(Command::AddToQueue(item), cx)
				});
			}),
	);

	let playlist_state = state.clone();
	let playlist_track = track.clone();
	menu = menu.submenu_with_icon(
		Some(Icon::new(IconName::Plus)),
		rust_i18n::t!("menu.add_to_playlist"),
		window,
		cx,
		move |submenu, _, _| {
			let picker_state = playlist_state.clone();
			let picker_track = playlist_track.id;
			let mut submenu = submenu
				.min_w(px(224.))
				.max_h(px(360.))
				.scrollable(true)
				.item(
					PopupMenuItem::new(rust_i18n::t!("menu.filter_playlists"))
						.icon(Icon::new(IconName::Search))
						.on_click(move |_, window, cx| {
							dialogs::pick_playlist(picker_state.clone(), picker_track, window, cx)
						}),
				)
				.separator();
			for (id, title, already) in owned.clone() {
				let state = playlist_state.clone();
				let track = playlist_track.clone();
				let label: SharedString = title.clone().into();
				submenu = submenu.item(
					PopupMenuItem::element(move |_, _| {
						h_flex()
							.w_full()
							.justify_between()
							.gap(px(16.))
							.child(div().truncate().child(label.clone()))
							.when(already, |row| {
								row.child(
									div()
										.text_size(px(11.))
										.text_color(tokens::fg_dim())
										.child(rust_i18n::t!("menu.already_in")),
								)
							})
					})
					.disabled(already)
					.on_click(move |_, _, cx| {
						let (id, title, track) = (id.clone(), title.clone(), track.clone());
						state.update(cx, |state, cx| {
							state.add_to_playlist(id, &title, track.id, cx)
						});
					}),
				);
			}
			let state = playlist_state.clone();
			let track = playlist_track.clone();
			submenu.separator().item(
				PopupMenuItem::new(rust_i18n::t!("menu.new_playlist"))
					.icon(Icon::new(IconName::Plus))
					.on_click(move |_, window, cx| {
						let state = state.clone();
						let track_id = track.id;
						dialogs::ask_name(
							rust_i18n::t!("menu.new_playlist").into(),
							String::new(),
							rust_i18n::t!("page.create").into(),
							move |title, cx| {
								state.update(cx, |state, cx| {
									state.create_playlist(title, Some(track_id), cx)
								})
							},
							window,
							cx,
						);
					}),
			)
		},
	);

	menu = menu.separator();
	let album_state = state.clone();
	let album_id = track.album.id;
	menu = menu.item(
		PopupMenuItem::new(rust_i18n::t!("menu.go_to_album"))
			.icon(Icon::new(IconName::Disc))
			.on_click(move |_, _, cx| {
				album_state.update(cx, |state, cx| state.navigate(Route::Album(album_id), cx))
			}),
	);
	if track.artists.len() > 1 {
		let artists = track.artists.clone();
		let artist_state = state.clone();
		menu = menu.submenu_with_icon(
			Some(Icon::new(IconName::User)),
			rust_i18n::t!("menu.go_to_artist"),
			window,
			cx,
			move |mut submenu, _, _| {
				for artist in artists.clone() {
					let state = artist_state.clone();
					submenu = submenu.item(PopupMenuItem::new(artist.name.clone()).on_click(
						move |_, _, cx| {
							state.update(cx, |state, cx| {
								state.navigate(Route::Artist(artist.id), cx)
							})
						},
					));
				}
				submenu
			},
		);
	} else if let Some(artist) = track.artists.first().cloned() {
		let artist_state = state.clone();
		menu = menu.item(
			PopupMenuItem::new(rust_i18n::t!("menu.go_to_artist"))
				.icon(Icon::new(IconName::User))
				.on_click(move |_, _, cx| {
					artist_state
						.update(cx, |state, cx| state.navigate(Route::Artist(artist.id), cx))
				}),
		);
	}

	menu = menu.separator();
	let link_state = state.clone();
	let link_track = track.id;
	menu = menu.item(
		PopupMenuItem::new(rust_i18n::t!("menu.copy_link"))
			.icon(Icon::new(IconName::Link))
			.on_click(move |_, _, cx| {
				cx.write_to_clipboard(ClipboardItem::new_string(crate::model::track_share_url(
					link_track,
				)));
				link_state.update(cx, |state, cx| {
					state.toast(rust_i18n::t!("toast.link_copied"), cx)
				});
			}),
	);

	if let Some((id, index)) = target.owned_row.clone() {
		let remove_state = state.clone();
		menu = menu.separator().item(
			PopupMenuItem::new(rust_i18n::t!("menu.remove_from_playlist"))
				.icon(Icon::new(IconName::Trash))
				.on_click(move |_, _, cx| {
					let id = id.clone();
					remove_state.update(cx, |state, cx| state.remove_from_playlist(id, index, cx))
				}),
		);
	}
	if let Some(index) = target.queue_index {
		let queue_state = state.clone();
		menu = menu.separator().item(
			PopupMenuItem::new(rust_i18n::t!("menu.remove_from_queue"))
				.icon(Icon::new(IconName::X))
				.on_click(move |_, _, cx| {
					queue_state.update(cx, |state, cx| {
						state.dispatch(Command::RemoveFromQueue(index), cx)
					})
				}),
		);
	}
	menu
}
