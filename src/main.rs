#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;

use gpui_kit::component::TitleBar;
use gpui_kit::*;
use tidal_rs::audio::engine::spawn_audio;
use tidal_rs::backend::{BackendConfig, cover_dir, spawn_backend};
use tidal_rs::i18n;
use tidal_rs::store::{self, Snapshot};
use tidal_rs::tidal::Endpoints;
use tidal_rs::tidal::auth::KeyringStore;
use tidal_rs::tidal::config::client_creds_path;
use tidal_rs::ui::state::AppState;
use tidal_rs::ui::{app::AppView, assets::AppAssets, theme};

fn main() {
	// Before any window exists, so the media session carries this id.
	#[cfg(windows)]
	if let Err(error) = tidal_rs::os::windows::set_app_id(tidal_rs::os::windows::APP_ID) {
		eprintln!("could not set the app id: {error}");
	}
	// Dev aid: TIDAL_RS_DEMO=1 uses canned data instead of TIDAL and never touches the saved session.
	let demo = std::env::var_os("TIDAL_RS_DEMO").is_some();
	let state_path = (!demo).then(store::state_path);
	let snapshot = match &state_path {
		Some(path) => store::load(path),
		None => Snapshot::default(),
	};
	let backend = if demo {
		tidal_rs::demo::spawn_demo_backend()
	} else {
		spawn_backend(BackendConfig {
			endpoints: Endpoints::default(),
			store: Arc::new(KeyringStore),
			creds_path: client_creds_path(),
			cover_dir: cover_dir(),
		})
	};
	let (audio_events_tx, audio_events) = async_channel::unbounded();
	let audio = spawn_audio(audio_events_tx, backend.runtime.clone());
	let system = i18n::system_locale();
	i18n::apply(i18n::resolve(snapshot.settings.language, system.as_deref()));

	gpui_kit::application()
		.with_assets(AppAssets)
		.run(move |cx| {
			gpui_kit::init(cx);
			theme::install(cx);
			cx.bind_keys(tidal_rs::ui::app::key_bindings());
			let state = cx.new(|cx| {
				AppState::new(
					snapshot,
					state_path,
					backend.requests.clone(),
					backend.responses.clone(),
					audio,
					audio_events,
					cx,
				)
			});
			let bounds = Bounds::centered(None, size(px(1320.), px(820.)), cx);
			// Dev aid: TIDAL_RS_BACKGROUND=1 opens the window without stealing focus.
			let background = std::env::var_os("TIDAL_RS_BACKGROUND").is_some();
			let options = WindowOptions {
				window_bounds: Some(WindowBounds::Windowed(bounds)),
				window_min_size: Some(size(px(1000.), px(640.))),
				focus: !background,
				..TitleBar::window_options()
			};
			if demo {
				tidal_rs::demo::run_scenario(state.clone(), cx);
			}
			gpui_kit::open_window(options, cx, |window, cx| {
				cx.new(|cx| AppView::new(state, window, cx))
			})
			.expect("failed to open the main window");
			if !background {
				cx.activate(true);
			}
			// Closing the window quits; the session is saved by the quit hook.
			cx.on_window_closed(|cx, _| cx.quit()).detach();
		});
}
