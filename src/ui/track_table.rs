//! The track table used by playlist, album, artist, search and recently played pages.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::h_flex;
use gpui_kit::component::menu::ContextMenuExt as _;
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::model::{ContextRef, Origin, PlaylistId, QueueItem, Track};
use crate::sort::{SortKey, TrackSort};
use crate::ui::queue_panel::DragPreview;
use crate::ui::state::{AppState, COVER_SMALL};
use crate::ui::track_menu::{MenuTarget, track_menu};
use crate::ui::{format, tokens, widgets};

pub const ROW_HEIGHT: f32 = 52.;

/// What double clicking a row plays.
#[derive(Clone)]
pub enum PlayAs {
	/// The whole list, starting at the row.
	List(ContextRef),
	/// Only that track; suggestions follow.
	Single,
}

#[derive(Clone)]
pub struct TableOptions {
	pub id: &'static str,
	pub play_as: PlayAs,
	pub show_album: bool,
	pub show_added: bool,
	/// Rows of a playlist the user owns get "Remove from this playlist".
	pub owned_playlist: Option<PlaylistId>,
	/// Position of each displayed row in the playlist when the view is sorted.
	pub source_index: Option<Rc<Vec<usize>>>,
	/// Rows can be dragged to reorder the playlist (owned and unsorted).
	pub reorderable: bool,
	/// Playlist pages: clicking a column title sorts by it.
	pub sort: Option<(PlaylistId, TrackSort)>,
}

impl TableOptions {
	fn origin(&self) -> Origin {
		match &self.play_as {
			PlayAs::List(kind) => kind.origin(),
			PlayAs::Single => Origin::Other,
		}
	}
}

/// Page content shown above the column titles; it scrolls away with the rows.
pub type PageHeader = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

/// Column titles. On playlists they sort the view when clicked.
fn header(options: &TableOptions, inset: f32, state: &Entity<AppState>) -> Div {
	let sortable = |key: SortKey, content: Div| -> AnyElement {
		let Some((playlist, sort)) = options.sort.clone() else {
			return content.into_any_element();
		};
		let active = sort.key == key && key != SortKey::Custom;
		let state = state.clone();
		let arrow = if sort.descending {
			IconName::ArrowDown
		} else {
			IconName::ArrowUp
		};
		content
			.id(("sort-column", key as usize))
			.cursor_pointer()
			.hover(|style| style.text_color(tokens::fg()))
			.when(active, |label| {
				label
					.text_color(tokens::fg())
					.child(widgets::icon(arrow, 11., tokens::red()))
			})
			.on_click(move |_, _, cx| {
				let playlist = playlist.clone();
				state.update(cx, |state, cx| state.set_sort(playlist, key, cx));
			})
			.into_any_element()
	};
	let label = |text: SharedString| {
		h_flex()
			.gap(px(4.))
			.text_size(px(11.))
			.font_weight(FontWeight::MEDIUM)
			.text_color(tokens::fg_dim())
			.child(text.to_uppercase())
	};
	let row = h_flex()
		.w_full()
		.h(px(32.))
		.px(px(10.))
		.border_b_1()
		.border_color(tokens::border())
		.child(
			div()
				.w(px(36.))
				.child(sortable(SortKey::Custom, label("#".into()))),
		)
		.child(div().flex_1().min_w_0().child(sortable(
			SortKey::Title,
			label(rust_i18n::t!("table.title").into()),
		)))
		.when(options.show_album, |row| {
			row.child(div().w(px(260.)).px(px(12.)).child(sortable(
				SortKey::Album,
				label(rust_i18n::t!("table.album").into()),
			)))
		})
		.when(options.show_added, |row| {
			row.child(div().w(px(110.)).child(sortable(
				SortKey::Added,
				label(rust_i18n::t!("table.added").into()),
			)))
		})
		.child(
			div().w(px(56.)).flex().justify_end().child(sortable(
				SortKey::Duration,
				h_flex()
					.gap(px(4.))
					.child(widgets::icon(IconName::Clock, 13., tokens::fg_dim())),
			)),
		);
	div().w_full().px(px(inset)).mb(px(6.)).child(row)
}

/// Builds the row at `ix` with its play handler and context menu.
fn interactive_row(
	ix: usize,
	tracks: &Rc<Vec<Track>>,
	options: &TableOptions,
	playing_id: Option<crate::model::TrackId>,
	state: &Entity<AppState>,
	cx: &App,
) -> impl IntoElement + use<> {
	let track = &tracks[ix];
	let playing = Some(track.id) == playing_id;
	let app = state.read(cx);
	let cover = app.cover(track.album.cover.as_deref(), COVER_SMALL);
	let selected = app
		.selected
		.as_ref()
		.is_some_and(|item| item.track.id == track.id);
	let origin = options.origin();
	// Plays this row: the list from here, or the single track.
	let play_now = {
		let state = state.clone();
		let tracks = tracks.clone();
		let play_as = options.play_as.clone();
		Rc::new(move |cx: &mut App| {
			let tracks = tracks.clone();
			let play_as = play_as.clone();
			state.update(cx, |state, cx| match play_as {
				PlayAs::List(kind) => {
					state.play_list(kind, tracks.as_ref().clone(), Some(ix), None, cx)
				}
				PlayAs::Single => state.play_single(
					QueueItem {
						track: tracks[ix].clone(),
						origin: Origin::Other,
					},
					cx,
				),
			});
		})
	};
	let on_click = {
		let state = state.clone();
		let tracks = tracks.clone();
		let origin = origin.clone();
		let play_now = play_now.clone();
		move |event: &ClickEvent, _: &mut Window, cx: &mut App| {
			if event.click_count() >= 2 {
				play_now(cx);
				return;
			}
			let item = QueueItem {
				track: tracks[ix].clone(),
				origin: origin.clone(),
			};
			state.update(cx, |state, cx| {
				state.selected = Some(item);
				cx.notify();
			});
		}
	};
	let on_play_cell = move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
		cx.stop_propagation();
		play_now(cx);
	};
	let target = MenuTarget {
		item: QueueItem {
			track: track.clone(),
			origin,
		},
		owned_row: options.owned_playlist.clone().map(|id| {
			let position = options
				.source_index
				.as_ref()
				.and_then(|source| source.get(ix).copied())
				.unwrap_or(ix);
			(id, position)
		}),
		queue_index: None,
	};
	let menu_state = state.clone();
	let row = row(
		(options.id, ix),
		ix,
		track,
		playing,
		cover,
		options.show_album,
		options.show_added,
		on_play_cell,
	)
	.when(selected, |row| row.bg(tokens::muted()))
	.on_click(on_click);
	let row = match options
		.owned_playlist
		.as_ref()
		.filter(|_| options.reorderable)
	{
		Some(playlist) => {
			let drop_state = state.clone();
			let playlist = playlist.clone();
			let dragged = DraggedTrackRow {
				playlist: playlist.clone(),
				from: ix,
				title: track.title.clone().into(),
			};
			row.on_drag(dragged, |dragged, _, _, cx| {
				cx.new(|_| DragPreview(dragged.title.clone()))
			})
			.drag_over::<DraggedTrackRow>(|style, _, _, _| style.bg(tokens::accent()))
			.on_drop(move |dragged: &DraggedTrackRow, _, cx| {
				if dragged.playlist != playlist || dragged.from == ix {
					return;
				}
				let (id, from) = (playlist.clone(), dragged.from);
				drop_state.update(cx, |state, cx| state.move_playlist_item(id, from, ix, cx));
			})
		}
		None => row,
	};
	row.context_menu(move |menu, window, cx| track_menu(menu, &target, &menu_state, window, cx))
}

/// A playlist row being dragged to a new position.
#[derive(Clone)]
pub struct DraggedTrackRow {
	pub playlist: PlaylistId,
	pub from: usize,
	pub title: SharedString,
}

/// Virtualized table filling the remaining height; for long lists.
/// Virtualized table filling the remaining height. `page` (cover, title, buttons)
/// is the first item, so it scrolls away with the rows.
pub fn track_table(
	options: TableOptions,
	tracks: Vec<Track>,
	state: &Entity<AppState>,
	page: Option<PageHeader>,
	cx: &App,
) -> impl IntoElement {
	let playing_id = state.read(cx).player.view().now.map(|item| item.track.id);
	let tracks = Rc::new(tracks);
	let lead = usize::from(page.is_some()) + 1;
	let list_state = state.read(cx).list_state(options.id, tracks.len() + lead);
	let state = state.clone();
	let list_options = options.clone();
	let list = list(list_state.clone(), move |ix, window, cx| {
		if let Some(page) = page.as_ref().filter(|_| ix == 0) {
			return div().w_full().child(page(window, cx)).into_any_element();
		}
		if ix + 1 == lead {
			return header(&list_options, 24., &state).into_any_element();
		}
		let row = ix - lead;
		div()
			.w_full()
			.px(px(24.))
			.when(row + 1 == tracks.len(), |last| last.pb(px(24.)))
			.child(interactive_row(
				row,
				&tracks,
				&list_options,
				playing_id,
				&state,
				cx,
			))
			.into_any_element()
	});
	div()
		.relative()
		.flex_1()
		.min_h_0()
		.child(list.size_full())
		.child(
			div()
				.absolute()
				.top_0()
				.right_0()
				.bottom_0()
				.w(Scrollbar::width())
				.child(Scrollbar::vertical(&list_state).viewport_from_layout()),
		)
}

/// Plain table for short lists inside a scrolling page.
pub fn track_list(
	options: TableOptions,
	tracks: Vec<Track>,
	state: &Entity<AppState>,
	cx: &App,
) -> Div {
	let playing_id = state.read(cx).player.view().now.map(|item| item.track.id);
	let tracks = Rc::new(tracks);
	div()
		.flex()
		.flex_col()
		.child(header(&options, 0., state))
		.children(
			(0..tracks.len())
				.map(|ix| interactive_row(ix, &tracks, &options, playing_id, state, cx)),
		)
}

#[allow(clippy::too_many_arguments)]
pub fn row(
	id: impl Into<ElementId>,
	ix: usize,
	track: &Track,
	playing: bool,
	cover: Option<std::path::PathBuf>,
	show_album: bool,
	show_added: bool,
	on_play: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
	let group: SharedString = "track-row".into();
	let title_color = if playing { tokens::red() } else { tokens::fg() };
	let number = div()
		.id(("play-cell", ix))
		.cursor_pointer()
		.on_click(on_play)
		.w(px(36.))
		.flex_none()
		.relative()
		.text_color(tokens::fg_muted())
		.child(if playing {
			widgets::icon(IconName::AudioLines, 15., tokens::red()).into_any_element()
		} else {
			div()
				.group_hover(group.clone(), |style| style.invisible())
				.child((ix + 1).to_string())
				.into_any_element()
		})
		.when(!playing, |number| {
			number.child(
				div()
					.absolute()
					.top_0()
					.left_0()
					.invisible()
					.group_hover(group.clone(), |style| style.visible())
					.child(widgets::icon(IconName::Play, 14., tokens::fg())),
			)
		});
	h_flex()
		.id(id)
		.group(group)
		.w_full()
		.h(px(ROW_HEIGHT))
		.px(px(10.))
		.rounded(px(8.))
		.text_color(tokens::fg_muted())
		.hover(|style| style.bg(tokens::muted()))
		.child(number)
		.child(
			h_flex()
				.flex_1()
				.min_w_0()
				.gap(px(12.))
				.child(widgets::cover(cover, 36., 4.))
				.child(
					div()
						.flex()
						.flex_col()
						.min_w_0()
						.child(
							div()
								.truncate()
								.text_color(title_color)
								.font_weight(FontWeight::MEDIUM)
								.child(track.title.clone()),
						)
						.child(
							div()
								.truncate()
								.text_size(px(12.))
								.mt(px(2.))
								.child(track.artist_names()),
						),
				),
		)
		.when(show_album, |row| {
			row.child(
				div()
					.w(px(260.))
					.px(px(12.))
					.truncate()
					.child(track.album.title.clone()),
			)
		})
		.when(show_added, |row| {
			row.child(
				div().w(px(110.)).child(
					track
						.added
						.as_deref()
						.map(format::added_date)
						.unwrap_or_default(),
				),
			)
		})
		.child(
			div()
				.w(px(56.))
				.flex()
				.justify_end()
				.child(format::clock(track.duration)),
		)
}
