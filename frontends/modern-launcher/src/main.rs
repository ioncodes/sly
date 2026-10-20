mod animation;
mod app;
mod artwork_cache;
mod color;
mod file_browser;
mod fonts;
mod icons;
mod input;
mod library;
mod materials;
mod models;
mod prompts;
mod renderer;
mod search;
mod session;
mod settings;
mod sound;
mod ui;
mod worker;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::{ElementState, Ime, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::window::{Fullscreen, Window, WindowId};

use crate::app::App;
use crate::input::InputContext;
use crate::renderer::Renderer;
use crate::settings::Settings;

#[derive(Parser)]
#[command(about = "Sly Launcher, a modern take on emulation frontends")]
struct Cli {
    /// ROM files or directories. Overrides saved library roots.
    roms: Vec<PathBuf>,
    #[arg(long)]
    settings: Option<PathBuf>,
    #[arg(long, default_value_t = 1280)]
    width: u32,
    #[arg(long, default_value_t = 720)]
    height: u32,
    #[arg(long)]
    mute: bool,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "modern_launcher=info,wgpu=warn".into()),
        )
        .init();

    let cli = Cli::parse();

    anyhow::ensure!(
        (320..=7680).contains(&cli.width) && (240..=4320).contains(&cli.height),
        "Window size must be between 320x240 and 7680x4320"
    );

    let path = cli
        .settings
        .clone()
        .unwrap_or_else(::settings::default_path);

    let mut settings = Settings::load(&path).context("Cannot read launcher settings")?;

    if !cli.roms.is_empty() {
        settings.rom_directories = cli.roms;
    }

    if cli.mute {
        settings.volume = 0.0;
    }

    let app = App::new(settings, path)?;

    let mut runtime = Runtime {
        app,
        window: None,
        renderer: None,
        last_update: Instant::now(),
        next_draw: Instant::now(),
        focused: true,
        occluded: false,
        size: [cli.width, cli.height],
        failure: None,
        text_input: TextInput::default(),
    };

    let event_loop = EventLoop::new()?;
    event_loop.run_app(&mut runtime)?;

    if let Some(error) = runtime.failure {
        return Err(error);
    }

    Ok(())
}

struct Runtime {
    app: App,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    last_update: Instant,
    next_draw: Instant,
    focused: bool,
    occluded: bool,
    size: [u32; 2],
    failure: Option<anyhow::Error>,
    text_input: TextInput,
}

#[derive(Default)]
struct TextInput {
    allowed: bool,
    composing: bool,
    modifiers: ModifiersState,
    candidate_rect: Option<[f32; 4]>,
    opening_key: Option<PhysicalKey>,
    cancelled_composition: bool,
}

impl TextInput {
    fn cancel_composition(&mut self) {
        self.composing = false;
        self.cancelled_composition = true;
    }

    fn cancel_hidden_preedit(&mut self, has_preedit: bool) -> bool {
        if self.composing && !has_preedit {
            self.cancel_composition();
            true
        } else {
            false
        }
    }

    fn accepts_ime(&mut self, event: &Ime) -> bool {
        match event {
            Ime::Enabled => {
                // Commits from a cancelled composition are ignored until IME restarts.
                self.cancelled_composition = false;
            }
            Ime::Disabled => self.composing = false,
            Ime::Preedit(text, _) if !self.cancelled_composition => {
                self.composing = !text.is_empty();
            }
            Ime::Commit(_) if !self.cancelled_composition => self.composing = false,
            _ => return false,
        }

        true
    }

    fn keyboard_text<'a>(
        &self,
        text: Option<&'a str>,
        state: ElementState,
        synthetic: bool,
        search_was_open: bool,
        search_is_open: bool,
    ) -> Option<&'a str> {
        // Key events only type when no IME composition is active, and the key that opened search
        // never types. Ctrl+Alt still allows AltGr.
        (self.allowed
            && !self.composing
            && search_was_open
            && search_is_open
            && !synthetic
            && state == ElementState::Pressed
            && !self.modifiers.super_key()
            && (!self.modifiers.control_key() || self.modifiers.alt_key()))
        .then_some(text)
        .flatten()
    }
}

impl Runtime {
    fn sync_text_input(&mut self) {
        let Some(window) = &self.window else {
            return;
        };

        let allowed = self.focused && self.app.context() == InputContext::Search;

        if allowed != self.text_input.allowed {
            window.set_ime_allowed(allowed);
            self.text_input.allowed = allowed;
            self.text_input.composing = false;
            self.text_input.candidate_rect = None;

            if !allowed {
                self.text_input.opening_key = None;
                self.text_input.cancel_composition();
            }

            self.app.search_preedit("");
        }

        if allowed {
            if self
                .text_input
                .cancel_hidden_preedit(self.app.search_has_preedit())
            {
                // A controller edit removed the preedit, so cancel the native composition too.
                window.set_ime_allowed(false);
                window.set_ime_allowed(true);
            }

            let size = window.inner_size();

            let rect = self
                .app
                .search_input_rect([size.width, size.height], window.scale_factor() as f32);

            if rect != self.text_input.candidate_rect {
                if let Some([x, y, width, height]) = rect {
                    window.set_ime_cursor_area(
                        LogicalPosition::new(x, y),
                        LogicalSize::new(width, height),
                    );
                }

                self.text_input.candidate_rect = rect;
            }
        }
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: anyhow::Error) {
        eprintln!("Launcher error: {error:#}");
        self.failure = Some(error);
        event_loop.exit();
    }

    fn redraw(&mut self) -> Result<()> {
        let Some(window) = &self.window else {
            return Ok(());
        };

        if self.occluded || window.inner_size().width == 0 || window.inner_size().height == 0 {
            return Ok(());
        }

        let renderer = self.renderer.as_mut().context("Renderer unavailable")?;
        self.app.needs_redraw = false;
        let scale = window.scale_factor() as f32;
        let ui = self.app.ui(renderer.size(), scale);
        let scene = self.app.scene_with_scale(renderer.size(), scale);

        tracing::trace!(
            elapsed = self.app.animation.elapsed,
            phase = ?self.app.animation.phase,
            selection = self.app.animation.selection(),
            "render frame"
        );

        renderer.render(&scene, &ui)?;
        self.app.needs_redraw |= renderer.needs_present();
        self.next_draw = Instant::now() + Duration::from_secs_f64(1.0 / 60.0);

        if window.fullscreen().is_some() != self.app.settings.fullscreen {
            window.set_fullscreen(
                self.app
                    .settings
                    .fullscreen
                    .then_some(Fullscreen::Borderless(None)),
            );
        }

        Ok(())
    }
}

impl ApplicationHandler for Runtime {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let result = (|| -> Result<()> {
            let window = Arc::new(
                event_loop.create_window(
                    Window::default_attributes()
                        .with_title("Sly Launcher")
                        .with_inner_size(LogicalSize::new(self.size[0], self.size[1]))
                        .with_min_inner_size(LogicalSize::new(480, 360)),
                )?,
            );

            let size = window.inner_size();

            let renderer = pollster::block_on(Renderer::new(
                Some(window.clone()),
                [size.width, size.height],
                self.app.settings.quality.texture_budget_bytes(),
            ))?;

            self.renderer = Some(renderer);
            self.window = Some(window);
            self.last_update = Instant::now();
            Ok(())
        })();

        if let Err(error) = result {
            self.fail(event_loop, error);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if self.window.is_none() {
            return;
        }

        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize([size.width, size.height]);
                }

                self.app.needs_redraw = true;
            }
            WindowEvent::Focused(focused) => {
                self.focused = focused && !self.occluded;

                if !self.focused {
                    self.text_input.modifiers = ModifiersState::empty();
                }

                self.app.focus(self.focused);
            }
            WindowEvent::Occluded(occluded) => {
                self.occluded = occluded;

                self.focused = !occluded
                    && self
                        .window
                        .as_ref()
                        .is_some_and(|window| window.has_focus());

                self.app.focus(self.focused);
                self.app.needs_redraw = true;
            }
            WindowEvent::ScaleFactorChanged { .. } => self.app.needs_redraw = true,
            WindowEvent::ModifiersChanged(modifiers) => {
                self.text_input.modifiers = modifiers.state();
            }
            WindowEvent::KeyboardInput {
                event,
                is_synthetic,
                ..
            } if self.focused && !is_synthetic => {
                if self.app.editing_artwork_key {
                    self.app
                        .artwork_key_input(&event, self.text_input.modifiers);
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                    return;
                }
                let context = self.app.context();
                let opening_key = self.text_input.opening_key == Some(event.physical_key);

                if opening_key && event.state == ElementState::Released {
                    self.text_input.opening_key = None;
                }

                if self.text_input.composing && context == InputContext::Search {
                    // Escape during a composition cancels it instead of closing search.
                    if event.state == ElementState::Pressed
                        && event.physical_key == PhysicalKey::Code(KeyCode::Escape)
                        && let Some(window) = &self.window
                    {
                        window.set_ime_allowed(false);
                        window.set_ime_allowed(true);
                        self.text_input.cancel_composition();
                        self.app.search_preedit("");
                    }
                } else {
                    let actions = self.app.input.keyboard(
                        &event,
                        context,
                        self.app.settings.layout,
                        self.app.now_seconds(),
                    );

                    for action in actions {
                        self.app.action(action);
                    }

                    if context != InputContext::Search && self.app.context() == InputContext::Search
                    {
                        self.text_input.opening_key = Some(event.physical_key);
                    }

                    if !opening_key
                        && let Some(text) = self.text_input.keyboard_text(
                            event.text.as_deref(),
                            event.state,
                            is_synthetic,
                            context == InputContext::Search,
                            self.app.context() == InputContext::Search,
                        )
                    {
                        self.app.search_text(text);
                    }
                }
            }
            WindowEvent::Ime(ime)
                if self.text_input.allowed
                    && self.app.context() == InputContext::Search
                    && self.text_input.accepts_ime(&ime) =>
            {
                match ime {
                    Ime::Preedit(text, _) => {
                        if self.text_input.composing {
                            for action in self.app.input.clear() {
                                self.app.action(action);
                            }
                        }

                        self.app.search_preedit(&text);
                    }
                    Ime::Commit(text) => {
                        self.app.search_preedit("");
                        self.app.search_text(&text);
                    }
                    Ime::Disabled => {
                        self.app.search_preedit("");
                    }
                    Ime::Enabled => {}
                }
            }
            WindowEvent::RedrawRequested => {
                if let Err(error) = self.redraw() {
                    self.fail(event_loop, error);
                }
            }
            _ => {}
        }

        self.sync_text_input();
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();

        tracing::trace!(
            dt = now.duration_since(self.last_update).as_secs_f32(),
            weights = ?self.app.animation.view_weights(),
            dirty = self.app.needs_redraw,
            "native tick"
        );

        if let Some(renderer) = &mut self.renderer {
            self.app.update(
                now.duration_since(self.last_update).as_secs_f32(),
                now,
                renderer,
            );

            self.last_update = now;
        }

        if !self.focused && self.app.session.as_ref().is_some_and(|s| !s.paused()) {
            self.app.focus(false);
        }

        // Controller actions may open or close search during update().
        self.sync_text_input();

        if !self.occluded
            && self.app.needs_redraw
            && now >= self.next_draw
            && let Some(window) = &self.window
        {
            window.request_redraw();
            // Redraws may be deferred while hidden; don't spin on a missed deadline.
            self.next_draw = now + Duration::from_secs_f64(1.0 / 60.0);
        }

        let due = self
            .app
            .session
            .as_ref()
            .and_then(session::GameSession::next_frame_due);

        let mut next = now + Duration::from_millis(16);

        if let Some(due) = due {
            next = next.min(due);
        }

        if self.app.needs_redraw {
            next = next.min(self.next_draw);
        }

        event_loop.set_control_flow(ControlFlow::WaitUntil(
            next.max(now + Duration::from_millis(1)),
        ));
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.app.close();
    }
}
