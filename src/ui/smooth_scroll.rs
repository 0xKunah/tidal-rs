//! Smooth mouse wheel scrolling for every scrollable area.
//!
//! A wheel notch arrives as one big jump. This captures it before any scroll view sees it,
//! then replays it as small pixel steps over a few frames so scrolling eases out.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::*;

/// Time constant of the ease out: the remaining distance shrinks by 1/e every `TAU`.
const TAU: Duration = Duration::from_millis(90);
/// Below this many pixels left, the rest is applied at once.
const STOP_BELOW: f32 = 0.5;

/// Splits `pending` into the part to scroll this frame and what is left.
fn step(pending: f32, dt: Duration) -> (f32, f32) {
	let fraction = 1. - (-dt.as_secs_f32() / TAU.as_secs_f32()).exp();
	let now = pending * fraction;
	if (pending - now).abs() < STOP_BELOW {
		(pending, 0.)
	} else {
		(now, pending - now)
	}
}

#[derive(Default)]
struct Inner {
	pending: Point<f32>,
	position: Point<Pixels>,
	modifiers: Modifiers,
	running: bool,
	/// True while a replayed step is dispatched, so the listener lets it through.
	replaying: bool,
	last_frame: Option<Instant>,
}

#[derive(Clone, Default)]
pub struct SmoothScroll(Rc<RefCell<Inner>>);

impl SmoothScroll {
	/// Zero size element that installs the wheel listener; put it anywhere in the root view.
	pub fn listener(&self) -> impl IntoElement + use<> {
		let this = self.clone();
		canvas(
			|_, _, _| (),
			move |_, _, window, _| {
				let this = this.clone();
				window.on_mouse_event(
					move |event: &ScrollEvent, phase, window: &mut Window, cx: &mut App| {
						if phase == DispatchPhase::Capture {
							this.capture(event, window, cx);
						}
					},
				);
			},
		)
		.absolute()
		.size_0()
	}

	fn capture(&self, event: &ScrollEvent, window: &mut Window, cx: &mut App) {
		let ScrollDelta::Lines(_) = event.delta else {
			return;
		};
		if self.0.borrow().replaying || event.source != ScrollSource::Pointer {
			return;
		}
		let delta = event.delta.pixel_delta(window.line_height());
		{
			let mut inner = self.0.borrow_mut();
			let (x, y) = (f32::from(delta.x), f32::from(delta.y));
			// A reversal drops what was still queued the other way.
			if inner.pending.x * x < 0. {
				inner.pending.x = 0.;
			}
			if inner.pending.y * y < 0. {
				inner.pending.y = 0.;
			}
			inner.pending.x += x;
			inner.pending.y += y;
			inner.position = event.position;
			inner.modifiers = event.modifiers;
			if inner.running {
				cx.stop_propagation();
				return;
			}
			inner.running = true;
			inner.last_frame = Some(Instant::now());
		}
		cx.stop_propagation();
		self.schedule(window);
	}

	fn schedule(&self, window: &mut Window) {
		let this = self.clone();
		window.on_next_frame(move |window, cx| this.tick(window, cx));
	}

	fn tick(&self, window: &mut Window, cx: &mut App) {
		let (event, done) = {
			let mut inner = self.0.borrow_mut();
			let now = Instant::now();
			let dt = inner
				.last_frame
				.replace(now)
				.map_or(Duration::from_millis(16), |last| now - last)
				.min(Duration::from_millis(50));
			let (sx, rx) = step(inner.pending.x, dt);
			let (sy, ry) = step(inner.pending.y, dt);
			inner.pending = point(rx, ry);
			let done = rx == 0. && ry == 0.;
			if done {
				inner.running = false;
				inner.last_frame = None;
			}
			let event = ScrollEvent {
				position: inner.position,
				delta: ScrollDelta::Pixels(point(px(sx), px(sy))),
				modifiers: inner.modifiers,
				touch_phase: TouchPhase::Moved,
				source: ScrollSource::Pointer,
			};
			(event, done)
		};
		self.0.borrow_mut().replaying = true;
		window.dispatch_event(PlatformInput::Scroll(event), cx);
		self.0.borrow_mut().replaying = false;
		if !done {
			self.schedule(window);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[::core::prelude::v1::test]
	fn steps_ease_out_and_sum_to_the_whole_distance() {
		let (mut pending, mut total) = (100., 0.);
		let mut frames = 0;
		while pending != 0. {
			let (now, left) = step(pending, Duration::from_millis(16));
			assert!(now > 0.);
			total += now;
			pending = left;
			frames += 1;
			assert!(frames < 200);
		}
		assert!((total - 100.).abs() < 0.001);
		assert!(frames > 5, "must spread over several frames");
	}

	#[::core::prelude::v1::test]
	fn negative_distance_scrolls_back() {
		let (now, left) = step(-60., Duration::from_millis(16));
		assert!(now < 0. && left < 0. && now + left == -60.);
	}

	#[::core::prelude::v1::test]
	fn tiny_remainder_is_applied_at_once() {
		assert_eq!(step(0.3, Duration::from_millis(16)), (0.3, 0.));
	}
}
