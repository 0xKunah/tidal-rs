//! Right column: now playing, "Added by you", then what plays next.

use gpui_kit::assets::IconName;
use gpui_kit::component::menu::ContextMenuExt as _;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::model::{ContextRef, Origin, QueueItem, Track};
use crate::player::{Command, NextLabel};
use crate::ui::state::{AppState, COVER_SMALL};
use crate::ui::track_menu::{MenuTarget, track_menu};
use crate::ui::{tokens, widgets};

pub const WIDTH: f32 = 300.;
/// Upcoming tracks rendered in the panel; the rest stays in the queue.
const UPCOMING_SHOWN: usize = 100;

/// A queue row being dragged.
#[derive(Clone)]
pub struct DraggedQueueRow {
	pub section: Section,
	pub from: usize,
	pub title: SharedString,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Section {
	AddedByYou,
	Upcoming,
}

/// Drag preview: the title in a small pill.
pub struct DragPreview(pub SharedString);

impl Render for DragPreview {
	fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
		div()
			.px(px(10.))
			.py(px(6.))
			.rounded(px(6.))
			.bg(tokens::accent())
			.text_color(tokens::fg())
			.text_size(px(12.))
			.shadow_lg()
			.child(self.0.clone())
	}
}

fn section_header(text: impl Into<SharedString>) -> Div {
	h_flex()
		.justify_between()
		.px(px(6.))
		.pt(px(14.))
		.pb(px(6.))
		.child(widgets::section_label(text))
}

fn context_name(kind: &ContextRef) -> String {
	match kind {
		ContextRef::Playlist { title, .. } | ContextRef::Album { title, .. } => title.clone(),
		ContextRef::ArtistTop { name, .. } => name.clone(),
		ContextRef::Recent => rust_i18n::t!("queue.recent_context").to_string(),
	}
}

fn next_label(label: &NextLabel) -> Option<String> {
	match label {
		NextLabel::Context { kind, shuffled } => Some(if *shuffled {
			rust_i18n::t!("queue.next_context_shuffled", name = context_name(kind)).to_string()
		} else {
			rust_i18n::t!("queue.next_context", name = context_name(kind)).to_string()
		}),
		NextLabel::Suggestions { based_on } => {
			Some(rust_i18n::t!("queue.next_suggestions", name = based_on).to_string())
		}
		NextLabel::Nothing => None,
	}
}

#[allow(clippy::too_many_arguments)]
fn queue_row(
	id: ElementId,
	track: &Track,
	cover: Option<std::path::PathBuf>,
	now: bool,
	drag: Option<DraggedQueueRow>,
	target: MenuTarget,
	state: &Entity<AppState>,
) -> impl IntoElement + use<> {
	let group: SharedString = "queue-row".into();
	let title_color = if now {
		rgb(0xfca5a5).into()
	} else {
		tokens::fg()
	};
	let row = h_flex()
		.id(id)
		.group(group.clone())
		.h(px(48.))
		.px(px(6.))
		.gap(px(10.))
		.rounded(px(8.))
		.when(now, |row| row.bg(tokens::red_tint()))
		.when(!now, |row| row.hover(|style| style.bg(tokens::muted())))
		.child(widgets::cover(cover, 32., 4.))
		.child(
			v_flex()
				.flex_1()
				.min_w_0()
				.child(
					div()
						.truncate()
						.font_weight(FontWeight::MEDIUM)
						.text_color(title_color)
						.child(track.title.clone()),
				)
				.child(
					div()
						.truncate()
						.mt(px(2.))
						.text_size(px(11.))
						.text_color(tokens::fg_dim())
						.child(track.artist_names()),
				),
		)
		.when(now, |row| {
			row.child(widgets::icon(IconName::AudioLines, 14., tokens::red()))
		})
		.when(drag.is_some(), |row| {
			row.child(
				div()
					.invisible()
					.group_hover(group.clone(), |style| style.visible())
					.child(widgets::icon(IconName::GripVertical, 14., tokens::fg_dim())),
			)
		});
	let row = match drag {
		Some(dragged) => {
			let (section, to) = (dragged.section, dragged.from);
			let drop_state = state.clone();
			row.cursor_grab()
				.on_drag(dragged, |dragged, _, _, cx| {
					cx.new(|_| DragPreview(dragged.title.clone()))
				})
				.drag_over::<DraggedQueueRow>(move |style, dragged, _, _| {
					if dragged.section == section {
						style.bg(tokens::accent())
					} else {
						style
					}
				})
				.on_drop(move |dragged: &DraggedQueueRow, _, cx| {
					if dragged.section != section || dragged.from == to {
						return;
					}
					let from = dragged.from;
					let command = match section {
						Section::AddedByYou => Command::MoveInQueue { from, to },
						Section::Upcoming => Command::MoveUpcoming { from, to },
					};
					drop_state.update(cx, |state, cx| state.dispatch(command, cx));
				})
		}
		None => row,
	};
	let menu_state = state.clone();
	row.context_menu(move |menu, window, cx| track_menu(menu, &target, &menu_state, window, cx))
}

pub fn queue_panel(state: &Entity<AppState>, cx: &App) -> impl IntoElement {
	let app = state.read(cx);
	let view = app.player.view();
	let origin = view
		.context
		.as_ref()
		.map(ContextRef::origin)
		.unwrap_or(Origin::Other);
	let cover = |track: &Track| app.cover(track.album.cover.as_deref(), COVER_SMALL);
	let close_state = state.clone();
	let clear_state = state.clone();

	let mut panel = v_flex()
		.id("queue-scroll")
		.flex_1()
		.min_h_0()
		.overflow_y_scrollbar()
		.px(px(12.))
		.pb(px(12.));

	if let Some(now) = &view.now {
		panel = panel
			.child(section_header(rust_i18n::t!("queue.now_playing")))
			.child(queue_row(
				"queue-now".into(),
				&now.track,
				cover(&now.track),
				true,
				None,
				MenuTarget {
					item: now.clone(),
					owned_row: None,
					queue_index: None,
				},
				state,
			));
	}

	if !view.user_queue.is_empty() {
		panel = panel
			.child(
				section_header(rust_i18n::t!("queue.added_by_you")).child(
					div()
						.id("queue-clear")
						.text_size(px(12.))
						.text_color(tokens::fg_muted())
						.cursor_pointer()
						.hover(|style| style.text_color(tokens::fg()))
						.child(rust_i18n::t!("queue.clear"))
						.on_click(move |_, _, cx| {
							clear_state
								.update(cx, |state, cx| state.dispatch(Command::ClearQueue, cx))
						}),
				),
			)
			.children(view.user_queue.iter().enumerate().map(|(ix, item)| {
				queue_row(
					("queue-user", ix).into(),
					&item.track,
					cover(&item.track),
					false,
					Some(DraggedQueueRow {
						section: Section::AddedByYou,
						from: ix,
						title: item.track.title.clone().into(),
					}),
					MenuTarget {
						item: item.clone(),
						owned_row: None,
						queue_index: Some(ix),
					},
					state,
				)
			}));
	}

	if let Some(label) = next_label(&view.next_label)
		&& !view.upcoming.is_empty()
	{
		let origin = match view.next_label {
			NextLabel::Suggestions { .. } => Origin::Other,
			_ => origin,
		};
		panel = panel.child(section_header(label)).children(
			view.upcoming
				.iter()
				.take(UPCOMING_SHOWN)
				.enumerate()
				.map(|(ix, track)| {
					queue_row(
						("queue-next", ix).into(),
						track,
						cover(track),
						false,
						Some(DraggedQueueRow {
							section: Section::Upcoming,
							from: ix,
							title: track.title.clone().into(),
						}),
						MenuTarget {
							item: QueueItem {
								track: track.clone(),
								origin: origin.clone(),
							},
							owned_row: None,
							queue_index: None,
						},
						state,
					)
				}),
		);
	}

	if view.now.is_none() {
		panel = panel.child(
			div()
				.p(px(6.))
				.pt(px(14.))
				.text_color(tokens::fg_dim())
				.child(rust_i18n::t!("queue.empty")),
		);
	}

	v_flex()
		.w(px(WIDTH))
		.h_full()
		.flex_none()
		.bg(tokens::panel())
		.border_l_1()
		.border_color(tokens::border())
		.child(
			h_flex()
				.justify_between()
				.px(px(16.))
				.pt(px(14.))
				.child(
					div()
						.text_size(px(14.))
						.font_weight(FontWeight::SEMIBOLD)
						.child(rust_i18n::t!("queue.title")),
				)
				.child(widgets::icon_button(
					"queue-close",
					IconName::X,
					14.,
					false,
					move |_, _, cx| {
						close_state.update(cx, |state, cx| {
							state.queue_panel_open = false;
							cx.notify();
						})
					},
				)),
		)
		.child(panel)
}
