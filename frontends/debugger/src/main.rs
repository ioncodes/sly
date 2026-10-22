#![cfg_attr(target_family = "wasm", no_main)]

mod app;
mod audio_out;
mod breakpoints;
mod cover;
mod disasm;
mod logging;
mod memory;
mod root;
mod save;
mod script;
mod stats;
#[cfg(target_family = "wasm")]
mod web_dnd;

use std::sync::Arc;

use cirsium::components::{register_code_editor_keys, register_text_input_keys};
use cirsium::{Assets, Theme, ThemeMode, set_theme};
use gpui::prelude::*;
use gpui::{App, Bounds, WindowBounds, WindowOptions, px, size};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;

use debugger_model::DebugCore;
use logging::{LogBuffer, LogLayer};
use root::Root;

fn init_logging() -> LogBuffer {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    let logs = LogBuffer::new();

    let registry = tracing_subscriber::registry()
        .with(filter)
        .with(LogLayer::new(logs.clone()));

    #[cfg(not(target_family = "wasm"))]
    {
        use tracing_subscriber::util::SubscriberInitExt;
        registry
            .with(tracing_subscriber::fmt::layer().without_time())
            .init();
    }

    #[cfg(target_family = "wasm")]
    {
        let _ = tracing::subscriber::set_global_default(registry);
    }

    logs
}

pub(crate) type LoadedRom = (Box<dyn DebugCore>, cover::ArtQuery, storage::GameId);

pub(crate) fn load_rom(
    rom: Vec<u8>,
    file_name: String,
) -> Result<LoadedRom, Box<dyn std::error::Error>> {
    let system = core_loader::system_for_path(&file_name)?;
    let core = core_loader::load_debug(system, rom, file_name.clone())?;
    let meta = core
        .cartridge_info()
        .ok_or("loaded core did not provide cartridge metadata")?;
    let name = game_database::lookup(system, &meta.rom)
        .name
        .unwrap_or_else(|| {
            std::path::Path::new(&file_name)
                .file_stem()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or(file_name)
        });

    let art = cover::ArtQuery {
        game: meta.game_id(),
        region: meta.region,
        fallback: artwork::libretro::cover_url(system, &name).map(Into::into),
    };

    Ok((core, art, meta.game_id()))
}

#[cfg(not(target_family = "wasm"))]
fn file_name_of(path: &std::path::Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

fn run(initial: Option<(Box<dyn DebugCore>, cover::ArtQuery, storage::GameId)>, logs: LogBuffer) {
    #[cfg(target_family = "wasm")]
    let app = gpui_platform::single_threaded_web();
    #[cfg(not(target_family = "wasm"))]
    let app = gpui_platform::application();

    app.with_assets(Assets).run(move |cx: &mut App| {
        if let Err(err) = Assets.load_fonts(cx) {
            tracing::error!("font load failed: {err}");
        }

        cx.set_quit_mode(gpui::QuitMode::LastWindowClosed);
        register_text_input_keys(cx);
        register_code_editor_keys(cx);

        #[cfg(not(target_family = "wasm"))]
        cx.set_http_client(Arc::new(cover::ArtAuthClient(Arc::new(
            reqwest_client::ReqwestClient::user_agent("debugger").unwrap(),
        ))));

        #[cfg(target_family = "wasm")]
        cx.set_http_client(Arc::new(cover::ArtAuthClient));

        set_theme(cx, Theme::new(ThemeMode::Dark));

        let bounds = Bounds::centered(None, size(px(1600.), px(960.)), cx);
        let _window = cx
            .open_window(window_options(bounds), |_window, cx| {
                cx.new(|cx| Root::new(logs, initial, cx))
            })
            .unwrap();
        cx.activate(true);

        #[cfg(target_family = "wasm")]
        web_dnd::install(_window, cx);
    });
}

fn window_options(bounds: Bounds<gpui::Pixels>) -> WindowOptions {
    let options = WindowOptions::new().window_bounds(Some(WindowBounds::Windowed(bounds)));

    #[cfg(not(target_family = "wasm"))]
    let options = options
        .titlebar(Some(gpui::TitlebarOptions {
            title: Some("Sly Debugger".into()),
            appears_transparent: true,
            traffic_light_position: Some(cirsium::components::traffic_light_position()),
        }))
        .window_decorations(Some(gpui::WindowDecorations::Client));

    options
}

// ---------------------------------------------------------------------------
// Native entrypoint
// ---------------------------------------------------------------------------

#[cfg(not(target_family = "wasm"))]
fn main() {
    let logs = init_logging();

    let initial = match std::env::args().nth(1) {
        Some(path) => match load_rom_path(&path) {
            Ok(target) => Some(target),
            Err(err) => {
                eprintln!("failed to load {path}: {err}");
                std::process::exit(1);
            }
        },
        None => None,
    };

    run(initial, logs);
}

#[cfg(not(target_family = "wasm"))]
fn load_rom_path(path: &str) -> Result<LoadedRom, Box<dyn std::error::Error>> {
    let rom = std::fs::read(path)?;
    load_rom(rom, file_name_of(std::path::Path::new(path)))
}

// ---------------------------------------------------------------------------
// Web entrypoint
// ---------------------------------------------------------------------------

#[cfg(target_family = "wasm")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    gpui_platform::web_init();
    let logs = init_logging();
    run(None, logs);
}
