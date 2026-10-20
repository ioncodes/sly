use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Result;
use core_model::{Region, System};
use glam::camera::rh::proj::directx::orthographic;
use glam::camera::rh::view::look_at_mat4;
use glam::{Mat3, Mat4, Quat, Vec3};

use crate::animation::{Animation, PRESENTATION, Phase, smoothstep};
use crate::artwork_cache::{ArtworkCache, ArtworkKey, CoverAccent};
use crate::color::{luminance, srgb_to_linear, srgb8_to_linear};
use crate::file_browser::{self, DirectoryBrowser, DirectoryListing, MAX_DIRECTORY_ENTRIES};
use crate::icons::StatusIcon;
use crate::input::{Action, ActiveInput, Input, InputContext};
use crate::library::{Discovery, LibraryEntry};
use crate::models::{self, Profiles, ReleaseMarket};
use crate::prompts::PromptIcon;
use crate::renderer::{DrawObject, GamePresentation, Renderer, Scene};
use crate::session::{GameSession, LoadedGame, SessionLoader};
use crate::settings::{GameButton, Quality, Settings, ViewMode};
use crate::sound::{Cue, UiSounds};
use crate::ui::{FontFace, TextAlign, UiFrame, valid_scale};

mod search_ui;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Menu {
    Search,
    Consoles,
    Appearance,
    Settings,
    Folders,
    Controls,
    Pause,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SettingsRow {
    View,
    Quality,
    ReduceMotion,
    TextSize,
    IntegerScaling,
    Volume,
    Fullscreen,
    DisplayRelease,
    AddFolder,
    Rescan,
    Controls,
    GameAppearance,
    ArtworkKey,
    ArtworkAccount,
    SaveAndReturn,
}

impl SettingsRow {
    const ALL: [Self; 15] = [
        Self::View,
        Self::Quality,
        Self::ReduceMotion,
        Self::TextSize,
        Self::IntegerScaling,
        Self::Volume,
        Self::Fullscreen,
        Self::DisplayRelease,
        Self::AddFolder,
        Self::Rescan,
        Self::Controls,
        Self::GameAppearance,
        Self::ArtworkKey,
        Self::ArtworkAccount,
        Self::SaveAndReturn,
    ];

    fn index(self) -> usize {
        self as usize
    }

    fn label(self) -> &'static str {
        match self {
            Self::View => "View",
            Self::Quality => "Quality",
            Self::ReduceMotion => "Reduce motion",
            Self::TextSize => "Text size",
            Self::IntegerScaling => "Integer gameplay scaling",
            Self::Volume => "Volume",
            Self::Fullscreen => "Fullscreen",
            Self::DisplayRelease => "Display release",
            Self::AddFolder => "Add library folder",
            Self::Rescan => "Rescan library",
            Self::Controls => "Controls",
            Self::GameAppearance => "Game appearance",
            Self::ArtworkKey => "Artwork key",
            Self::ArtworkAccount => "Get artwork key",
            Self::SaveAndReturn => "Save and return",
        }
    }
}

const STATUS_DURATION: Duration = Duration::from_secs(8);

pub struct App {
    pub settings: Settings,
    pub editing_artwork_key: bool,
    artwork_key_draft: String,
    pub input: Input,
    pub animation: Animation,
    pub entries: Vec<LibraryEntry>,
    other_consoles: HashMap<System, Vec<LibraryEntry>>,
    console_selections: HashMap<System, String>,
    alphabet: Vec<String>,
    pub selected: usize,
    pub status: String,
    pub menu: Option<Menu>,
    menu_focus: usize,
    menu_motion: MenuMotion,
    search: Option<crate::search::SearchState>,
    search_preedit: String,
    search_key_motion: [MenuMotion; 2],
    sounds: UiSounds,
    browser: Option<DirectoryBrowser>,
    folder_listing: Option<DirectoryListing>,
    remapping: bool,
    status_until: Instant,
    status_visible: bool,
    status_severity: Severity,
    pub contours: bool,
    pub lighting: bool,
    pub session: Option<GameSession>,
    settings_path: PathBuf,
    discovery: Discovery,
    loader: SessionLoader,
    loaded: Option<LoadedGame>,
    artwork: ArtworkCache,
    generation: u64,
    prepared_artwork_generation: Option<u64>,
    artwork_refresh_at: Instant,
    resident_artwork: HashMap<usize, ArtworkKey>,
    theme_srgb: [f32; 3],
    theme_target: [f32; 3],
    profiles: Profiles,
    flipped: bool,
    game_size: Option<([u32; 2], f32)>,
    frame_sequence: Option<u64>,
    pub needs_redraw: bool,
    startup: Instant,
}

impl App {
    pub fn new(settings: Settings, settings_path: PathBuf) -> Result<Self> {
        settings::select_path(&settings_path).map_err(anyhow::Error::msg)?;
        let discovery = Discovery::start(settings.rom_directories.clone())?;
        let input = Input::new(&settings);
        let animation = Animation::new(0, settings.layout, settings.reduced_motion);
        let artwork = ArtworkCache::new(settings.quality.texture_budget_bytes(), 2)?;
        let profiles = models::profiles()?;

        Ok(Self {
            artwork_key_draft: settings::api_key().map_err(anyhow::Error::msg)?,
            editing_artwork_key: false,
            settings,
            input,
            animation,
            entries: Vec::new(),
            other_consoles: HashMap::new(),
            console_selections: HashMap::new(),
            alphabet: Vec::new(),
            selected: 0,
            status: String::new(),
            menu: None,
            menu_focus: 0,
            menu_motion: MenuMotion::default(),
            search: None,
            search_preedit: String::new(),
            search_key_motion: Default::default(),
            sounds: UiSounds::default(),
            browser: None,
            folder_listing: None,
            remapping: false,
            status_until: Instant::now() + STATUS_DURATION,
            status_visible: false,
            status_severity: Severity::Warning,
            contours: true,
            lighting: true,
            session: None,
            settings_path,
            discovery,
            loader: SessionLoader::new()?,
            loaded: None,
            artwork,
            generation: 0,
            prepared_artwork_generation: None,
            artwork_refresh_at: Instant::now(),
            resident_artwork: HashMap::new(),
            theme_srgb: CoverAccent::default().srgb,
            theme_target: CoverAccent::default().srgb,
            profiles,
            flipped: false,
            game_size: None,
            frame_sequence: None,
            needs_redraw: true,
            startup: Instant::now(),
        })
    }

    pub fn context(&self) -> InputContext {
        if self.menu == Some(Menu::Search) {
            InputContext::Search
        } else if self.menu.is_some() {
            InputContext::Settings
        } else if let Some(session) = &self.session {
            if session.paused() {
                InputContext::Paused
            } else {
                InputContext::Playing
            }
        } else if matches!(self.animation.phase, Phase::Launching | Phase::Returning) {
            InputContext::Transition
        } else {
            InputContext::Browsing
        }
    }

    pub fn action(&mut self, action: Action) {
        tracing::debug!(?action, context=?self.context(), "launcher action");
        self.needs_redraw = true;

        if self.menu_action(action) {
            return;
        }

        match action {
            Action::Search if self.context() == InputContext::Browsing => self.open_search(),
            Action::Consoles if self.context() == InputContext::Browsing => {
                let consoles = self.available_consoles();

                if !consoles.is_empty() {
                    let focus = consoles
                        .iter()
                        .position(|(system, _)| Some(*system) == self.active_console())
                        .unwrap_or(0);

                    self.status.clear();
                    self.open_menu(Menu::Consoles, focus);
                }
            }
            Action::Console(delta) if self.context() == InputContext::Browsing => {
                if self.cycle_console(delta) {
                    self.play_sound(Cue::View);
                }
            }
            Action::Navigate(delta)
                if self.context() == InputContext::Browsing && !self.entries.is_empty() =>
            {
                self.select(
                    self.selected
                        .saturating_add_signed(delta as isize)
                        .min(self.entries.len() - 1),
                );
            }
            Action::Alphabet(delta)
                if self.context() == InputContext::Browsing && !self.alphabet.is_empty() =>
            {
                let initial = self
                    .entries
                    .get(self.selected)
                    .map(LibraryEntry::initial)
                    .unwrap_or_default();

                let current = self
                    .alphabet
                    .iter()
                    .position(|letter| *letter == initial)
                    .unwrap_or(0);

                let next = &self.alphabet[cycle(current, delta, self.alphabet.len())];

                if let Some(index) = self
                    .entries
                    .iter()
                    .position(|entry| entry.initial() == *next)
                {
                    self.select(index);
                }
            }
            Action::Flip if self.context() == InputContext::Browsing => {
                self.flipped = !self.flipped;
                self.animation.flip(self.flipped);
                self.play_sound(Cue::Flip);
            }
            Action::CycleView if self.context() == InputContext::Browsing => {
                self.settings.layout = self.settings.layout.next();
                self.animation.set_view(self.settings.layout);
                self.play_sound(Cue::View);
            }
            Action::Launch if self.context() == InputContext::Browsing => {
                if let Some(entry) = self.entries.get(self.selected) {
                    self.loader
                        .request((entry.clone(), self.settings.data_directory.clone()));
                    self.loaded = None;
                    self.status.clear();
                    self.invalidate_artwork();
                    self.animation.begin_launch();
                    self.clear_inputs();
                }
            }
            Action::Back => {
                if self.animation.phase == Phase::Launching {
                    self.sounds.stop();
                    self.loader.cancel();
                    self.loaded = None;
                    self.animation.cancel_launch();
                    self.status.clear();
                    self.clear_inputs();
                    self.play_sound(Cue::Back);
                } else if self.session.is_some() {
                    self.return_to_library();
                    self.play_sound(Cue::Back);
                }
            }
            Action::Pause => {
                if let Some(session) = &mut self.session {
                    session.set_paused(true);
                    self.open_menu(Menu::Pause, 0);
                } else {
                    self.clear_inputs();
                }
            }
            Action::QuickSave => self.quick_save(),
            Action::QuickLoad => self.quick_load(),
            Action::Button(button, pressed) => {
                if let Some(session) = &mut self.session {
                    session.set_button(0, button, pressed);
                }
            }
            Action::Settings => {
                if let Some(session) = &mut self.session {
                    session.set_paused(true);
                }

                self.open_menu(Menu::Settings, 0);
            }
            Action::Fullscreen => {
                self.settings.fullscreen = !self.settings.fullscreen;
            }
            _ => {}
        }
    }

    fn play_sound(&mut self, cue: Cue) {
        self.sounds.play(cue, self.settings.volume, Instant::now());
    }

    fn open_menu(&mut self, menu: Menu, focus: usize) {
        self.menu = Some(menu);
        self.menu_focus = focus;
        self.clear_inputs();
        self.play_sound(Cue::Select);
    }

    pub fn rescan(&mut self) {
        match Discovery::start(self.settings.rom_directories.clone()) {
            Ok(discovery) => {
                self.discovery = discovery;
                self.status.clear();
                self.needs_redraw = true;
            }
            Err(error) => self.notice(Severity::Error, error.to_string()),
        }
    }

    pub fn select(&mut self, index: usize) {
        if index >= self.entries.len() || index == self.selected {
            return;
        }

        self.selected = index;
        self.flipped = false;
        self.animation.select(index);
        self.invalidate_artwork();
        self.status.clear();
        self.needs_redraw = true;
        self.play_sound(Cue::Navigate);
    }

    fn active_console(&self) -> Option<System> {
        self.entries.first().map(|entry| entry.system)
    }

    fn available_consoles(&self) -> Vec<(System, usize)> {
        System::ALL
            .into_iter()
            .filter_map(|system| {
                let count = if self.active_console() == Some(system) {
                    self.entries.len()
                } else {
                    self.other_consoles.get(&system).map_or(0, Vec::len)
                };

                (count > 0).then_some((system, count))
            })
            .collect()
    }

    fn remember_console_selection(&mut self) {
        if let Some(entry) = self.entries.get(self.selected) {
            self.console_selections
                .insert(entry.system, entry.sha1.clone());
        }
    }

    fn replace_library(&mut self, entries: Vec<LibraryEntry>) {
        let previous_console = self.active_console();
        self.remember_console_selection();
        self.entries.clear();
        self.other_consoles.clear();

        for entry in entries {
            self.other_consoles
                .entry(entry.system)
                .or_default()
                .push(entry);
        }

        let available = self.available_consoles();

        let system = previous_console
            .filter(|system| available.iter().any(|(candidate, _)| candidate == system))
            .or_else(|| available.first().map(|(system, _)| *system));

        if let Some(system) = system {
            self.entries = self.other_consoles.remove(&system).unwrap_or_default();
        }

        self.reset_console_shelf();

        if self.menu == Some(Menu::Search) {
            self.refresh_search();
        }

        if self.menu == Some(Menu::Consoles) {
            self.menu_focus = self.menu_focus.min(available.len().saturating_sub(1));

            if available.is_empty() {
                self.menu = None;
            }
        }
    }

    fn activate_console(&mut self, system: System) -> bool {
        if self.active_console() == Some(system)
            || self.session.is_some()
            || self.animation.phase != Phase::Browsing
        {
            return false;
        }

        let Some(incoming) = self
            .other_consoles
            .remove(&system)
            .filter(|entries| !entries.is_empty())
        else {
            return false;
        };

        self.remember_console_selection();

        if let Some(previous) = self.active_console() {
            self.other_consoles
                .insert(previous, std::mem::take(&mut self.entries));
        }

        self.entries = incoming;
        self.reset_console_shelf();
        self.status.clear();
        self.clear_inputs();
        true
    }

    fn cycle_console(&mut self, delta: i32) -> bool {
        let consoles = self.available_consoles();

        let Some(current) = consoles
            .iter()
            .position(|(system, _)| Some(*system) == self.active_console())
        else {
            return false;
        };

        self.activate_console(consoles[cycle(current, delta, consoles.len())].0)
    }

    fn reset_console_shelf(&mut self) {
        self.selected = self
            .active_console()
            .and_then(|system| self.console_selections.get(&system))
            .and_then(|sha1| self.entries.iter().position(|entry| &entry.sha1 == sha1))
            .unwrap_or(0);

        self.alphabet = self
            .entries
            .iter()
            .map(LibraryEntry::initial)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();

        if self.animation.phase == Phase::Browsing {
            let ambient_phase = self.animation.ambient_phase();

            self.animation = Animation::new(
                self.selected,
                self.settings.layout,
                self.settings.reduced_motion,
            );

            self.animation.set_ambient_phase(ambient_phase);
        } else {
            self.animation.select(self.selected);
        }

        self.flipped = false;
        self.resident_artwork.clear();
        self.prepared_artwork_generation = None;
        self.theme_target = CoverAccent::default().srgb;
        self.needs_redraw = true;
    }

    pub fn invalidate_artwork(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.artwork.set_generation(self.generation);
        self.needs_redraw = true;
    }

    pub fn clear_inputs(&mut self) {
        self.input.clear();

        if let Some(session) = &mut self.session {
            session.clear_inputs();
        }
    }

    pub fn focus(&mut self, focused: bool) {
        self.input.set_focused(focused);

        if !focused {
            self.sounds.stop();

            if let Some(session) = &mut self.session {
                session.set_paused(true);
                self.menu = Some(Menu::Pause);
                self.menu_focus = 0;
            }

            self.clear_inputs();
        }

        self.needs_redraw = true;
    }

    fn commit_artwork_key(&mut self, key: &str) {
        if let Err(error) = settings::set_api_key(key) {
            self.notice(Severity::Error, error);
            return;
        }

        self.artwork_key_draft = key.trim().to_owned();
        self.editing_artwork_key = false;

        self.invalidate_artwork();
        self.artwork.retry_failed();
        self.prepared_artwork_generation = None;
        self.artwork_refresh_at = Instant::now();
    }

    fn finish_artwork_key_edit(&mut self, save: bool) {
        if save {
            let draft = self.artwork_key_draft.clone();
            self.commit_artwork_key(&draft);
        } else {
            self.editing_artwork_key = false;
            self.artwork_key_draft = settings::api_key().unwrap_or_default();
        }
    }

    pub fn artwork_key_input(
        &mut self,
        event: &winit::event::KeyEvent,
        modifiers: winit::keyboard::ModifiersState,
    ) {
        use winit::keyboard::{KeyCode, PhysicalKey};

        if event.state != winit::event::ElementState::Pressed {
            return;
        }

        match event.physical_key {
            PhysicalKey::Code(KeyCode::Enter) => self.finish_artwork_key_edit(true),
            PhysicalKey::Code(KeyCode::Escape) => self.finish_artwork_key_edit(false),
            PhysicalKey::Code(KeyCode::Backspace) => {
                self.artwork_key_draft.pop();
            }
            PhysicalKey::Code(KeyCode::KeyA)
                if modifiers.control_key() || modifiers.super_key() =>
            {
                self.artwork_key_draft.clear()
            }
            PhysicalKey::Code(KeyCode::KeyV)
                if modifiers.control_key() || modifiers.super_key() =>
            {
                match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text()) {
                    Ok(text) => match settings::normalize_key(&text) {
                        Ok(text) => self.artwork_key_draft.push_str(&text),
                        Err(error) => self.notice(Severity::Error, error),
                    },
                    Err(_) => self.notice(Severity::Error, "Cannot read clipboard".into()),
                }
            }
            _ if !modifiers.control_key() && !modifiers.super_key() => {
                if let Some(text) = &event.text {
                    self.artwork_key_draft
                        .extend(text.chars().filter(|c| c.is_ascii() && !c.is_control()));
                }
            }
            _ => {}
        }

        self.needs_redraw = true;
    }

    pub fn save_settings(&mut self) -> bool {
        if let Err(error) = self.settings.save(&self.settings_path) {
            self.notice(Severity::Error, error.to_string());
            return false;
        }

        true
    }

    pub fn close(&mut self) {
        self.sounds.stop();

        if let Some(session) = &self.session
            && let Err(error) = session.persist()
        {
            eprintln!("Cannot persist session: {error}");
        }

        self.save_settings();
    }

    pub fn update(&mut self, dt: f32, now: Instant, renderer: &mut Renderer) {
        self.sounds.update(self.settings.volume);
        renderer.last_upload_bytes = 0;
        let was_animating = self.animation.is_active();
        self.animation.advance(dt);

        if let Some(browser) = &mut self.browser
            && let Some(result) = browser.poll()
        {
            match result {
                Ok(mut listing) => {
                    listing.entries.retain(|entry| {
                        entry.is_directory || core_loader::system_for_path(&entry.path).is_ok()
                    });

                    if listing.truncated {
                        self.notice(
                            Severity::Warning,
                            format!(
                                "Directory listing is limited to {MAX_DIRECTORY_ENTRIES} entries. \
                             Choose a smaller directory."
                            ),
                        );
                    } else if !listing.warnings.is_empty() {
                        self.notice(Severity::Warning, listing.warnings.join("; "));
                    }

                    self.folder_listing = Some(listing);
                    self.menu_focus = 0;
                }
                Err(error) => self.notice(Severity::Error, error),
            }

            self.needs_redraw = true;
        }

        if let Some((device, key, button)) = self.input.take_remap() {
            let bindings = match device {
                ActiveInput::Gamepad => &mut self.settings.controls.game_gamepad,
                ActiveInput::Keyboard => &mut self.settings.controls.game_keyboard,
            };

            bindings.insert(key, button);
            self.input.configure(&self.settings);
            self.remapping = false;

            if self.save_settings() {
                self.status.clear();
                self.play_sound(Cue::Select);
            }
        }

        let actions = self.input.poll(
            self.startup.elapsed().as_secs_f64(),
            self.context(),
            self.settings.layout,
        );

        for action in actions {
            self.action(action);
        }

        self.advance_menu_motion(dt);

        if let Some(result) = self.discovery.poll() {
            self.invalidate_artwork();
            self.replace_library(result.entries);
            self.notice(Severity::Warning, result.warnings.join("\n"));
        }

        if let Some(result) = self.loader.poll() {
            match result {
                Ok(loaded) => {
                    self.loaded = Some(loaded);

                    if self.animation.phase == Phase::Launching {
                        self.play_sound(Cue::Launch);
                    }
                }
                Err(error) => {
                    self.notice(Severity::Error, error);
                    self.sounds.stop();
                    self.animation.cancel_launch();
                    self.loaded = None;
                    self.clear_inputs();
                }
            }

            self.needs_redraw = true;
        }

        if self.animation.launch_ready()
            && let Some(loaded) = self.loaded.take()
        {
            self.notice(Severity::Warning, loaded.warnings.join("\n"));
            self.session = Some(loaded.activate(self.settings.volume));
            self.frame_sequence = None;
            self.animation.enter_playing();
            self.clear_inputs();
        }

        if let Some(session) = &mut self.session {
            self.needs_redraw |= session.tick(now);
            let warnings = session.take_warnings();

            if let Some(frame) = session.frame()
                && self.frame_sequence != Some(frame.sequence)
            {
                renderer.update_game(frame.width, frame.height, frame.stride, frame.bytes);
                self.game_size = Some(([frame.width, frame.height], frame.display_aspect));
                self.frame_sequence = Some(frame.sequence);
            }

            if !warnings.is_empty() {
                self.notice(Severity::Warning, warnings.join("\n"));
            }
        }

        if (self.session.is_none() || self.menu == Some(Menu::Appearance))
            && self.animation.phase != Phase::Launching
            && (self.menu.is_none() || self.menu == Some(Menu::Appearance))
            && (self.prepared_artwork_generation != Some(self.generation)
                || self.artwork.has_pending()
                || now >= self.artwork_refresh_at)
        {
            let uploaded = self.prepare_artwork(renderer);
            self.prepared_artwork_generation = Some(self.generation);
            self.artwork_refresh_at = now + Duration::from_millis(if uploaded { 16 } else { 1000 });
        }

        let blend = if self.settings.reduced_motion {
            1.0
        } else {
            1.0 - (-dt.clamp(0.0, 0.25) * 7.0).exp()
        };

        for (current, target) in self.theme_srgb.iter_mut().zip(self.theme_target) {
            if (*current - target).abs() > 0.0005 {
                *current += (target - *current) * blend;
                self.needs_redraw = true;
            } else {
                *current = target;
            }
        }

        self.needs_redraw |= was_animating || self.animation.is_active();
        self.needs_redraw |= self.menu.is_none() && self.animation.ambient_active();
        let status_visible = self.status_visible(now);
        self.needs_redraw |= status_visible != self.status_visible;
        self.status_visible = status_visible;
    }

    fn status_visible(&self, now: Instant) -> bool {
        !self.status.is_empty() && now < self.status_until
    }

    fn prepare_artwork(&mut self, renderer: &mut Renderer) -> bool {
        self.artwork
            .set_budget(self.settings.quality.texture_budget_bytes());
        renderer.set_art_budget(self.settings.quality.texture_budget_bytes());

        for completion in self.artwork.poll() {
            if let Err(error) = completion.result {
                tracing::warn!(%error,key=?completion.key,"artwork");
            }

            self.needs_redraw = true;
        }

        let neighbors = self.settings.quality.resident_neighbors() + 2;

        let mut indices: Vec<_> = (self.selected.saturating_sub(neighbors)
            ..(self.selected + neighbors + 1).min(self.entries.len()))
            .collect();

        indices.sort_by_key(|index| index.abs_diff(self.selected));
        let mut uploaded = 0;
        let tier = self.settings.quality.artwork_size();

        for index in indices {
            let entry = &self.entries[index];
            let region = self.display_region(entry);
            let key = self.artwork.request(self.generation, entry, region, tier);

            if index == self.selected {
                let preview_key = self.entry_art_key(index);

                let accent = self.artwork.get(&key).map(|art| art.accent).or_else(|| {
                    // Keep the old tint while a quality change loads the new tier.
                    self.artwork.get(&preview_key).map(|art| art.accent)
                });

                self.theme_target = accent.unwrap_or_default().srgb;
            }

            if !renderer.has_artwork(&key)
                && uploaded < 1
                && let Some(art) = self.artwork.get(&key)
            {
                if let Err(error) = renderer.upload_artwork(key.clone(), art.as_ref()) {
                    tracing::warn!(%error,"artwork upload");
                }

                uploaded += 1;
                self.needs_redraw = true;
            }
        }

        self.reconcile_local_artwork(|key| renderer.has_artwork(key));
        uploaded > 0
    }

    fn reconcile_local_artwork(&mut self, resident: impl Fn(&ArtworkKey) -> bool) {
        let radius = self.settings.quality.resident_neighbors() + 2;
        let focus = self.animation.selection().round().max(0.0) as usize;
        let mut indices = BTreeSet::new();

        for center in [self.selected, focus] {
            indices.extend(
                center.saturating_sub(radius)
                    ..center.saturating_add(radius + 1).min(self.entries.len()),
            );
        }

        let desired: HashMap<_, _> = indices
            .into_iter()
            .map(|index| (index, self.requested_art_key(index)))
            .collect();

        self.resident_artwork.retain(|index, key| {
            desired
                .get(index)
                .is_some_and(|wanted| key.same_release(wanted))
                && resident(key)
        });

        for (index, key) in desired {
            if resident(&key) {
                self.resident_artwork.insert(index, key);
            }
        }
    }

    pub fn now_seconds(&self) -> f64 {
        self.startup.elapsed().as_secs_f64()
    }

    pub fn theme_accent(&self) -> [u8; 3] {
        self.theme_srgb
            .map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8)
    }

    pub fn entry_finish(&self, entry: &LibraryEntry) -> &'static crate::materials::CartridgeFinish {
        self.settings
            .cartridge_finish(&entry.game_id())
            .and_then(|id| crate::materials::find_cartridge_finish(entry.system, id))
            .unwrap_or(&crate::materials::cartridge_finishes(entry.system)[0])
    }

    fn display_region(&self, entry: &LibraryEntry) -> Option<Region> {
        self.settings
            .release_overrides
            .get(&entry.sha1)
            .map_or(entry.region, |market| market.region())
    }

    fn market(&self, entry: &LibraryEntry) -> ReleaseMarket {
        self.display_region(entry).into()
    }

    fn entry_art_key(&self, index: usize) -> ArtworkKey {
        let requested = self.requested_art_key(index);

        self.resident_artwork
            .get(&index)
            .filter(|key| key.same_release(&requested))
            .cloned()
            .unwrap_or(requested)
    }

    fn requested_art_key(&self, index: usize) -> ArtworkKey {
        ArtworkKey::new(
            &self.entries[index],
            self.display_region(&self.entries[index]),
            self.settings.quality.artwork_size(),
        )
    }

    pub fn scene_with_scale(&self, size: [u32; 2], scale: f32) -> Scene {
        let scale = valid_scale(scale);
        let logical_size = size.map(|value| value as f32 / scale);
        let aspect = size[0] as f32 / size[1].max(1) as f32;
        let height = 0.5_f32.max(0.75 / aspect);
        let width = height * aspect;
        let transition = self.animation.transition();
        let appearance = self.menu == Some(Menu::Appearance);

        let appearance_preview = appearance
            .then(|| self.appearance_preview_rect(logical_size))
            .flatten();

        let mut scene = Scene {
            view_projection: orthographic(
                -width / 2.0,
                width / 2.0,
                -height / 2.0,
                height / 2.0,
                0.01,
                10.0,
            ) * look_at_mat4(Vec3::new(0.0, 0.0, 2.0), Vec3::ZERO, Vec3::Y),
            contours: self.contours && !appearance,
            lighting: self.lighting,
            background_phase: self.animation.ambient_phase(),
            theme_tint: self.theme_srgb.map(srgb_to_linear),
            visible: appearance || self.session.is_none(),
            iris_radius: if appearance {
                1.1
            } else {
                1.0 - transition.iris_closed
            },
            fade: if !appearance && self.animation.phase == Phase::Playing {
                transition.blackout_opacity
            } else {
                0.0
            },
            ..Default::default()
        };

        if !appearance && self.animation.phase == Phase::Playing {
            scene.iris_radius = 1.1;

            if let Some((dimensions, aspect)) = self.game_size {
                scene.game = Some(GamePresentation {
                    rect: game_rect(size, dimensions, aspect, self.settings.integer_scaling),
                    opacity: 1.0,
                });
            }

            return scene;
        }

        if self.menu.is_some()
            && self.menu != Some(Menu::Search)
            && (!appearance || appearance_preview.is_none())
        {
            scene.objects.clear();
            return scene;
        }

        let weights = self.animation.view_weights();
        let ambient = self.animation.ambient_phase();

        let floating = if self.animation.ambient_enabled() {
            1.0
        } else {
            0.0
        };

        let float_height = PRESENTATION.float_height;
        let float_rotation = PRESENTATION.float_rotation;
        let compact = compact_library(logical_size, self.settings.text_scale);
        let viewport_top = self.library_scene_top(logical_size) / logical_size[1];
        let composition_top = if compact { 0.20 } else { viewport_top };
        let row_center = height * (0.5 - (composition_top + 0.90) * 0.5);
        let ribbon_center = height * (0.5 - (composition_top + 0.69) * 0.5);
        let neighbors = self.settings.quality.resident_neighbors();
        let focus = self.animation.selection();
        let center = focus.round().max(0.0) as usize;

        let shelf = if appearance {
            0..0
        } else {
            center.saturating_sub(neighbors)..(center + neighbors + 1).min(self.entries.len())
        };

        for index in shelf {
            let entry = &self.entries[index];
            let market = self.market(entry);
            let profile = self.profiles.for_system_market(entry.system, market);
            let offset = index as f32 - focus;
            let approach = (1.0 - offset.abs() * 2.0).clamp(0.0, 1.0);
            let clearance = smoothstep(approach / 0.2);
            let emphasis = smoothstep((approach - 0.2) / 0.8);
            let pullout = clearance * 0.32;
            let lateral = pullout * (1.0 - emphasis);

            let row = Vec3::new(
                -width * 0.1775 + offset * 0.054 + lateral * RESTING_ROW_TILT.tan(),
                row_center,
                pullout - offset.abs() * 0.004,
            );

            let column = Vec3::new(
                -width * 0.2175 + emphasis * width * 0.04,
                row_center - offset * 0.052 - lateral * RESTING_COLUMN_TILT.tan(),
                pullout - offset.abs() * 0.004,
            );

            let ribbon = Vec3::new(
                offset * 0.054 + lateral * RESTING_ROW_TILT.tan(),
                ribbon_center,
                pullout - offset.abs() * 0.004,
            );

            let resting_wave = (ambient * 2.0 - index as f32 * 0.68).sin() * float_height;
            let bob = resting_wave * (1.0 - emphasis) + ambient.sin() * float_height * emphasis;

            let position = row * weights[0]
                + column * weights[1]
                + ribbon * weights[2]
                + Vec3::Y * (bob * floating);

            let idle = resting_box_rotation(profile, weights[1]);
            let turn = self.animation.flip_angle_for(index);
            let face = selected_box_rotation(turn, ambient, float_rotation * floating);
            let rotation = idle.slerp(face, emphasis);
            let desired_scale = (1.48 + emphasis * 0.34) * (1.0 - weights[2] * 0.10);
            let dimensions = profile.box_size_m();

            let available_height = height
                * (0.90 - weights[2] * 0.21 - composition_top - 48.0 / logical_size[1])
                - float_height * 2.0;

            let scale = box_display_scale(dimensions, rotation, desired_scale, available_height);
            let opacity = (neighbors as f32 - offset.abs()).clamp(0.0, 1.0);

            scene.objects.push(DrawObject {
                system: entry.system,
                shell_finish: None,
                market,
                placeholder: entry.system == System::Snes && market == ReleaseMarket::Unknown,
                kind: profile.box_model,
                transform: Mat4::from_scale_rotation_translation(
                    Vec3::splat(scale),
                    rotation,
                    position,
                ),
                artwork: Some(self.entry_art_key(index)),
                tint: [1.0, 1.0, 1.0, opacity],
                clip: Some([
                    0.0,
                    viewport_top,
                    0.645 + weights[2] * 0.355,
                    (0.90 - weights[2] * 0.21 - viewport_top).max(0.0),
                ]),
            });
        }

        if let Some(entry) = self.entries.get(self.selected) {
            let market = self.market(entry);
            let profile = self.profiles.for_system_market(entry.system, market);
            let proxy_geometry = profile.cartridge_model.is_none();
            let placeholder = entry.system == System::Snes && proxy_geometry;

            let kind = profile
                .cartridge_model
                .unwrap_or(models::ModelKind::UsCartridge);

            // Without a cartridge model, borrow the US shell. Unknown SNES releases get the dummy
            // label; other consoles keep their scanned label.
            let cartridge_market = if proxy_geometry {
                ReleaseMarket::NorthAmerica
            } else {
                market
            };

            let display_profile = self
                .profiles
                .for_system_market(entry.system, cartridge_market);

            let row_layout = self.library_layout(logical_size, ViewMode::Row);
            let cart_center = if compact {
                height * 0.065
            } else {
                height * (0.5 - (viewport_top + row_layout.title_top / logical_size[1]) * 0.5)
            };
            let cart_x = if compact {
                width * 0.315
            } else {
                width
                    * ((row_layout.title_x + row_layout.title_width * 0.5) / logical_size[0] - 0.5)
            };
            let row = Vec3::new(cart_x, cart_center, 0.28);

            let ribbon_details_center = if compact {
                -height * 0.10
            } else {
                let layout = self.library_layout(logical_size, ViewMode::Ribbon);
                height * (0.5 - layout.ribbon_details_center() / logical_size[1])
            };

            let ribbon = Vec3::new(-width * 0.26, ribbon_details_center, 0.28);
            let idle = row * (1.0 - weights[2])
                + ribbon * weights[2]
                + Vec3::Y * ((ambient + 0.8).sin() * float_height * 1.5 * floating);
            let mut position = idle.lerp(
                Vec3::new(0.0, height * 0.09, 0.65),
                transition.cartridge_expansion,
            );

            let mut scale = (1.65 - weights[2] * 0.50)
                * cartridge_display_scale(display_profile)
                * (1.0 + transition.cartridge_expansion * 1.5);

            let rotation = Quat::from_rotation_x(-0.16 + ambient.sin() * float_rotation * floating)
                * Quat::from_rotation_y(
                    (-0.25 + (ambient.cos() - 1.0) * float_rotation * floating)
                        * (1.0 - transition.cartridge_expansion),
                );

            if let Some([left, top, preview_width, preview_height]) = appearance_preview {
                if let Some(dimensions) = &display_profile.cartridge_dimensions_mm {
                    let bounds = rotated_size(Vec3::from_array(*dimensions) * 0.001, rotation);
                    scale = (preview_width * 0.82 / logical_size[0] * width / bounds.x)
                        .min(preview_height * 0.75 / logical_size[1] * height / bounds.y);
                }

                position = Vec3::new(
                    ((left + preview_width * 0.5) / logical_size[0] - 0.5) * width,
                    (0.5 - (top + preview_height * 0.5) / logical_size[1]) * height,
                    0.28,
                );
            }

            scene.objects.push(DrawObject {
                system: entry.system,
                shell_finish: self.entry_finish(entry).shell,
                market: cartridge_market,
                placeholder,
                kind,
                transform: Mat4::from_scale_rotation_translation(
                    Vec3::splat(scale),
                    rotation,
                    position,
                ),
                artwork: Some(self.entry_art_key(self.selected)),
                tint: [1.0; 4],
                clip: None,
            });
        }

        if compact && !appearance {
            let top = self.library_scene_top(logical_size);
            let bottom = self.library_detail_top(logical_size) - 12.0;
            let available = bottom - top;
            let expansion = transition.cartridge_expansion;

            if available < 60.0 && expansion == 0.0 {
                scene.objects.clear();
            } else {
                let rest_scale = (available.max(60.0) / logical_size[1] / 0.72).min(1.0);
                let display_scale = rest_scale + (1.0 - rest_scale) * expansion;
                let center = (top + bottom) * 0.5 / logical_size[1];
                let shift = height * (0.5 - center) * (1.0 - expansion);
                scene.view_projection *= Mat4::from_translation(Vec3::new(0.0, shift, 0.0))
                    * Mat4::from_scale(Vec3::splat(display_scale));
                let clip_top = (top / logical_size[1]) * (1.0 - expansion);
                let clip_bottom = (bottom / logical_size[1]) * (1.0 - expansion) + expansion;

                for object in &mut scene.objects {
                    let (left, width) = object.clip.map_or((0.0, 1.0), |clip| {
                        (
                            0.5 + (clip[0] - 0.5) * display_scale,
                            clip[2] * display_scale,
                        )
                    });

                    object.clip = Some([left, clip_top, width, (clip_bottom - clip_top).max(0.0)]);
                }
            }
        }

        if let Some(cartridge) = scene.objects.iter().find(|object| !object.kind.is_box()) {
            let profile = self
                .profiles
                .for_system_market(cartridge.system, cartridge.market);

            if let Some(label) = profile.cartridge_label_anchor() {
                let label = (scene.view_projection
                    * cartridge.transform
                    * profile.model_to_display(cartridge.kind))
                .project_point3(label);

                scene.iris_center = [label.x * 0.5 + 0.5, 0.5 - label.y * 0.5];
            }
        }

        for object in &mut scene.objects {
            if let Some(clip) = object.clip
                && object.kind.is_box()
            {
                let profile = self
                    .profiles
                    .for_system_market(object.system, object.market);

                let dimensions = profile.box_size_m();

                object.tint[3] *= box_viewport_opacity(
                    scene.view_projection
                        * object.transform
                        * profile.model_to_display(object.kind),
                    dimensions,
                    clip,
                );
            }
        }

        scene
            .objects
            .sort_by(|a, b| a.transform.w_axis.z.total_cmp(&b.transform.w_axis.z));

        scene
    }

    fn library_layout(&self, logical_size: [f32; 2], view: ViewMode) -> LibraryLayout {
        let mut layout = LibraryLayout::new(logical_size, self.settings.text_scale, view);
        let (hints, prompt_scale) = self.footer_hints(logical_size, &layout);

        let icon_height = hints
            .iter()
            .map(|(control, _)| control_ink_height(control, self.input.active))
            .fold(0.0_f32, f32::max);

        layout.footer_controls_top = logical_size[1] - layout.margin - icon_height * prompt_scale;

        if view != ViewMode::Ribbon || compact_library(logical_size, self.settings.text_scale) {
            return layout;
        }

        let [width, height] = logical_size;
        let world_height = 0.5_f32.max(0.75 / (width / height));
        let pixels_per_metre = height / world_height;
        let available_height = world_height * (0.69 - layout.scene_top / height - 48.0 / height)
            - PRESENTATION.float_height * 2.0;

        let system = self
            .entries
            .get(self.selected)
            .map_or(System::Snes, |entry| entry.system);

        let mut half_height = 0.0_f32;
        let mut cartridge_half_width = 0.0_f32;

        // Size the details lane for the largest package on this console, so it never moves between
        // selections.
        for profile in self
            .profiles
            .profiles
            .iter()
            .filter(|profile| profile.system == system)
        {
            let dimensions = profile.box_size_m();

            for (rotation, desired_scale) in [
                (selected_box_rotation(0.0, 0.0, 0.0), 1.82),
                (resting_box_rotation(profile, 0.0), 1.48),
            ] {
                let scale =
                    box_display_scale(dimensions, rotation, desired_scale * 0.9, available_height);
                half_height = half_height.max(rotated_size(dimensions, rotation).y * scale * 0.5);
            }

            if let Some(dimensions) = &profile.cartridge_dimensions_mm {
                let rotation = Quat::from_rotation_x(-0.16) * Quat::from_rotation_y(-0.25);

                let half_width = rotated_size(Vec3::from_array(*dimensions) * 0.001, rotation).x
                    * 1.15
                    * cartridge_display_scale(profile)
                    * pixels_per_metre
                    * 0.5;

                cartridge_half_width = cartridge_half_width.max(half_width);
            }
        }

        layout.ribbon_lane_top =
            (layout.scene_top + height * 0.69) * 0.5 + half_height * pixels_per_metre;
        layout.title_top = layout.ribbon_lane_top + layout.info_padding;

        if cartridge_half_width > 0.0 {
            let outer_margin = width * 0.24 - cartridge_half_width;
            layout.title_width =
                (width - outer_margin - layout.info_padding - layout.title_x).max(1.0);
        }

        layout
    }

    pub fn library_detail_top(&self, logical_size: [f32; 2]) -> f32 {
        let layout = self.library_layout(logical_size, self.settings.layout);
        layout.title_top - layout.info_padding
    }

    pub fn library_scene_top(&self, logical_size: [f32; 2]) -> f32 {
        LibraryLayout::new(logical_size, self.settings.text_scale, self.settings.layout).scene_top
    }

    pub fn ui(&self, size: [u32; 2], scale: f32) -> UiFrame {
        let mut ui = UiFrame::new(size, scale);
        ui.text_scale = self.settings.text_scale;
        let [width, height] = ui.logical_size;
        let text_scale = ui.text_scale;
        let ink = UI_INK;
        let muted = UI_MUTED;

        if self.animation.phase == Phase::Launching {
            if self.animation.launch_ready() {
                ui.text(
                    24.0,
                    height * 0.4,
                    width - 48.0,
                    15.0,
                    "Back / Escape to cancel",
                    [192, 192, 184, 255],
                )
                .align = TextAlign::Center;
            }

            return ui;
        }

        if self.animation.phase == Phase::Returning {
            return ui;
        }

        if let Some(menu) = self.menu {
            if menu == Menu::Search {
                self.search_ui(&mut ui);
            } else {
                self.menu_ui(&mut ui, menu);
            }

            return ui;
        }

        let compact = compact_library(ui.logical_size, text_scale);
        let layout = self.library_layout(ui.logical_size, self.settings.layout);
        let theme = UiTheme::new(self.theme_accent());

        if self.session.is_some() {
            if self.status_visible(Instant::now()) {
                let message_height = notice_height(text_scale);
                notice_ui(
                    &mut ui,
                    [
                        16.0,
                        height - message_height - 16.0,
                        width - 32.0,
                        message_height,
                    ],
                    &self.status,
                    self.status_severity,
                );
            }

            return ui;
        }

        let console = self
            .entries
            .get(self.selected)
            .map_or("Sly", |entry| entry.system.name());

        capsule_ui(&mut ui, layout.margin, layout.margin, console, theme);

        if self.entries.is_empty() && self.discovery.finished() {
            let title_y = (height * if compact { 0.23 } else { 0.34 }).max(24.0);
            let title_size = if compact { 20.0 } else { 30.0 };
            let title_height = title_size * text_scale * if compact { 2.6 } else { 1.3 };

            let title = ui.text(
                24.0,
                title_y,
                width - 48.0,
                title_size,
                "Your library starts here",
                ink,
            );

            title.align = TextAlign::Center;
            title.max_height = title_height;

            let instruction = ui.text(
                24.0,
                title_y + title_height + 12.0,
                width - 48.0,
                16.0,
                "Open Settings to choose a ROM folder.",
                muted,
            );

            instruction.align = TextAlign::Center;
            instruction.max_height = 16.0 * text_scale * 2.6;
        } else if !self.entries.is_empty() {
            let entry = &self.entries[self.selected];
            self.alphabet_ui(&mut ui);
            let notice_visible = self.status_visible(Instant::now());
            let centered_ribbon =
                !compact && self.settings.layout == ViewMode::Ribbon && !notice_visible;

            let card_bottom = if notice_visible {
                layout.footer_top - notice_height(text_scale) - 20.0
            } else if centered_ribbon {
                layout.footer_controls_top
            } else {
                layout.footer_top - 8.0
            };

            let title_index = ui.texts.len();

            let title = ui.text(
                layout.title_x,
                layout.title_top,
                layout.title_width,
                layout.title_size,
                &entry.title,
                ink,
            );

            title.font = FontFace::Semibold;
            title.max_height = layout.title_height;
            title.clip_bottom = Some(card_bottom - layout.info_padding);

            let region = self
                .display_region(entry)
                .map(|region| region.to_string())
                .unwrap_or_else(|| "Unknown release".into());

            let release = if let Some(timing) = entry.tv_system {
                format!("{region} / {}", format!("{timing:?}").to_uppercase())
            } else {
                region
            };

            let region_index = ui.texts.len();

            let region = ui.text(
                layout.title_x,
                layout.title_top + layout.title_height + 8.0,
                layout.title_width,
                12.0,
                release,
                muted,
            );

            region.after_text = Some((title_index, 8.0));
            region.clip_bottom = Some(card_bottom - layout.info_padding);

            if height >= 500.0 {
                self.metadata_ui(
                    &mut ui,
                    entry,
                    [
                        layout.title_x,
                        layout.title_top + layout.title_height + 32.0,
                        layout.title_width,
                        card_bottom - layout.info_padding,
                    ],
                    region_index,
                );
            }

            let padding = layout.info_padding;

            let plate = ui.text_backplate(
                [
                    layout.title_x - padding,
                    layout.title_top - padding,
                    layout.title_width + padding * 2.0,
                    card_bottom - layout.title_top + padding,
                ],
                theme.info,
                14.0,
                [title_index, ui.texts.len()],
                padding,
            );

            if centered_ribbon {
                plate.vertical_center = Some(layout.ribbon_details_center());
            }
        }

        self.library_footer(&mut ui, &layout, theme);
        ui
    }

    fn library_footer(&self, ui: &mut UiFrame, layout: &LibraryLayout, theme: UiTheme) {
        let [width, height] = ui.logical_size;
        let scale = ui.text_scale;

        capsule_ui(
            ui,
            layout.margin,
            layout.footer_top,
            &self.library_count(),
            theme,
        );

        let (hints, prompt_scale) = self.footer_hints(ui.logical_size, layout);
        ui.text_scale = prompt_scale;
        let mut fragments = Vec::with_capacity(hints.len());

        for (control, label) in hints {
            let start = ui.mark();

            prompt_ui(
                ui,
                0.0,
                0.0,
                layout.footer_height,
                &control,
                label,
                self.input.active,
            );

            fragments.push(ui.fragment(start));
        }

        ui.bottom_right_row(
            fragments,
            width - layout.margin,
            height - layout.margin,
            14.0 * prompt_scale,
        );

        ui.text_scale = scale;

        if self.status_visible(Instant::now()) {
            let message_height = notice_height(scale);
            notice_ui(
                ui,
                [
                    layout.margin,
                    layout.footer_top - message_height - 12.0,
                    width - layout.margin * 2.0,
                    message_height,
                ],
                &self.status,
                self.status_severity,
            );
        }
    }

    fn library_count(&self) -> String {
        format!(
            "{} / {} Games",
            if self.entries.is_empty() {
                0
            } else {
                self.selected + 1
            },
            self.entries.len()
        )
    }

    fn footer_hints(
        &self,
        logical_size: [f32; 2],
        layout: &LibraryLayout,
    ) -> (Vec<(String, &'static str)>, f32) {
        let width = logical_size[0];
        let scale = self.settings.text_scale;
        let count_width = capsule_width(&self.library_count(), scale);
        let available = width - layout.margin * 2.0 - count_width - 16.0;
        let compact = compact_library(logical_size, scale);
        let mut hints = self.input.control_hints(compact);

        let mut total = hints
            .iter()
            .map(|(control, label)| prompt_width(control, label, scale, self.input.active))
            .sum::<f32>();

        if total > available {
            hints
                .retain(|(_, label)| matches!(*label, "Play" | "Settings" | "Consoles" | "Search"));

            total = hints
                .iter()
                .map(|(control, label)| prompt_width(control, label, scale, self.input.active))
                .sum();
        }

        let prompt_scale = scale * (available / total.max(1.0)).min(1.0);
        (hints, prompt_scale)
    }

    fn metadata_ui(
        &self,
        ui: &mut UiFrame,
        entry: &LibraryEntry,
        [x, y, width, bottom]: [f32; 4],
        after: usize,
    ) {
        let (summary, maker) = metadata_lines(&entry.meta);
        let mut previous = after;
        let mut gap = 14.0;

        if !summary.is_empty() {
            previous = ui.texts.len();
            let summary = ui.text(x, y, width, 17.0, summary, UI_INK);
            summary.max_height = summary.line_height * 2.0;
            summary.after_text = Some((after, 14.0));
            summary.clip_bottom = Some(bottom);
            gap = 9.0;
        }

        if let Some(maker) = maker {
            let maker = ui.text(x, y + 32.0, width, 17.0, maker, UI_MUTED);
            maker.max_height = maker.line_height * 3.0;
            maker.after_text = Some((previous, gap));
            maker.clip_bottom = Some(bottom);
        }
    }

    fn alphabet_ui(&self, ui: &mut UiFrame) {
        if self.alphabet.is_empty() {
            return;
        }

        let [width, _height] = ui.logical_size;
        let scale = ui.text_scale;
        let theme = UiTheme::new(self.theme_accent());

        let margin = self
            .library_layout(ui.logical_size, self.settings.layout)
            .margin;

        let console_width = self
            .entries
            .get(self.selected)
            .map_or(96.0, |entry| capsule_width(entry.system.name(), scale));

        let available = width - 2.0 * margin - console_width - 28.0;
        let cell = 24.0 * scale;
        let cross = capsule_height(scale) - 12.0;
        let visible = ((available / cell).floor().max(1.0) as usize).min(self.alphabet.len());

        let active = self
            .entries
            .get(self.selected)
            .map(LibraryEntry::initial)
            .unwrap_or_default();

        let selected = self
            .alphabet
            .iter()
            .position(|letter| *letter == active)
            .unwrap_or(0);

        let start = selected
            .saturating_sub(visible / 2)
            .min(self.alphabet.len().saturating_sub(visible));

        let length = visible as f32 * cell;

        let [x, y, w, h] = [
            ((width - length - 12.0) * 0.5).max(margin + console_width + 16.0),
            margin,
            length + 12.0,
            cross + 12.0,
        ];

        ui.rect([x, y, w, h], theme.soft, 12.0);

        for (offset, initial) in self.alphabet.iter().skip(start).take(visible).enumerate() {
            let [cx, cy, cw, ch] = [x + 6.0 + offset as f32 * cell, y + 6.0, cell, cross];
            let selected = *initial == active;

            if selected {
                ui.rect([cx + 1.0, cy + 1.0, cw - 2.0, ch - 2.0], theme.accent, 6.0);
            }

            let text = ui.text(
                cx,
                cy,
                cw,
                12.0,
                initial,
                if selected {
                    [255, 255, 255, 255]
                } else {
                    UI_MUTED
                },
            );

            text.align = TextAlign::Center;
            text.max_height = ch;
            text.vertically_centered = true;
            text.font = FontFace::Semibold;
        }
    }

    pub fn advance_menu_motion(&mut self, dt: f32) {
        self.needs_redraw |=
            self.menu_motion
                .advance(self.menu, self.menu_focus, dt, self.settings.reduced_motion);

        self.advance_search_motion(dt);
    }

    pub fn appearance_preview_rect(&self, logical_size: [f32; 2]) -> Option<[f32; 4]> {
        MenuLayout::new(
            logical_size,
            self.settings.text_scale,
            true,
            self.status_visible(Instant::now()),
        )
        .with_cartridge_preview(logical_size, self.settings.text_scale)
        .preview
    }

    fn menu_ui(&self, ui: &mut UiFrame, menu: Menu) {
        let [width, height] = ui.logical_size;
        let ink = UI_INK;
        let muted = UI_MUTED;
        let theme = UiTheme::new(self.theme_accent());

        let error = self
            .input
            .gamepad_error
            .as_deref()
            .filter(|_| menu == Menu::Controls);

        let (status, severity) = match error {
            Some(error) => (error, Severity::Error),
            None if self.status_visible(Instant::now()) => (&*self.status, self.status_severity),
            None => ("", self.status_severity),
        };

        let mut layout = MenuLayout::new(
            ui.logical_size,
            ui.text_scale,
            matches!(menu, Menu::Folders | Menu::Consoles | Menu::Appearance),
            !status.is_empty(),
        );

        if menu == Menu::Appearance {
            layout = layout.with_cartridge_preview(ui.logical_size, ui.text_scale);
        }

        if layout.preview.is_none() {
            ui.rect([0.0, 0.0, width, height], theme.background, 0.0);
        }

        let heading = match menu {
            Menu::Search => "Search games",
            Menu::Consoles => "Consoles",
            Menu::Appearance => "Appearance",
            Menu::Settings => "Settings",
            Menu::Folders => "Folders",
            Menu::Controls => "Controls",
            Menu::Pause => "Paused",
        };

        ui.text(
            layout.left,
            layout.top,
            layout.width * 0.75,
            layout.heading_size,
            heading,
            ink,
        )
        .font = FontFace::Semibold;

        let rows = self.menu_rows();

        let visible = ((layout.body_bottom - layout.body_top) / layout.row_height)
            .floor()
            .max(1.0) as usize;

        let start = self.menu_focus.saturating_sub(visible.saturating_sub(1));

        ui.text(
            layout.left + layout.width * 0.75,
            layout.top + 6.0,
            layout.width * 0.25,
            11.0,
            format!(
                "{}{} / {}{}",
                if start > 0 { "↑ " } else { "" },
                if rows.is_empty() {
                    0
                } else {
                    self.menu_focus + 1
                },
                rows.len(),
                if start + visible < rows.len() {
                    " ↓"
                } else {
                    ""
                }
            ),
            muted,
        )
        .align = TextAlign::Right;

        if layout.subtitle_height > 0.0 {
            let path = if menu == Menu::Consoles {
                self.input.console_shortcuts()
            } else if menu == Menu::Appearance {
                self.entries
                    .get(self.selected)
                    .map_or_else(String::new, |entry| {
                        format!("{} / {}", entry.system.name(), entry.title)
                    })
            } else {
                self.folder_listing
                    .as_ref()
                    .map(|l| {
                        if file_browser::is_drive_list(&l.path) {
                            "Drives".into()
                        } else {
                            l.path.display().to_string()
                        }
                    })
                    .unwrap_or_else(|| "Choose Home to start again".into())
            };

            ui.text(
                layout.left,
                layout.body_top - layout.subtitle_height - 6.0,
                layout.width,
                13.0,
                path,
                muted,
            );
        }

        let visual_row = (self.menu_motion.position(menu, self.menu_focus) - start as f32)
            .clamp(0.0, visible.saturating_sub(1) as f32);
        let highlight_y = layout.body_top + visual_row * layout.row_height + 3.0;

        ui.rect(
            [
                layout.left - 12.0,
                highlight_y,
                layout.width + 24.0,
                layout.row_height - 6.0,
            ],
            theme.soft,
            8.0,
        );

        ui.rect(
            [
                layout.left - 12.0,
                highlight_y + 8.0,
                3.0,
                layout.row_height - 22.0,
            ],
            theme.accent,
            1.5,
        );

        let row_label =
            |ui: &mut UiFrame, label: &str, [x, y, width, height]: [f32; 4], selected| {
                let label = ui.text(
                    x,
                    y,
                    width,
                    17.0,
                    label,
                    if selected { theme.accent } else { ink },
                );

                label.max_height = height;
                label.vertically_centered = true;

                if selected {
                    label.font = FontFace::Semibold;
                }
            };

        for (index, (label, value)) in rows.iter().enumerate().skip(start).take(visible) {
            let y = layout.body_top + (index - start) as f32 * layout.row_height;
            let selected = index == self.menu_focus;

            if value.is_empty() {
                row_label(
                    ui,
                    label,
                    [layout.left, y, layout.width, layout.row_height],
                    selected,
                );

                continue;
            }

            if layout.stacked {
                let label_index = ui.texts.len();
                let label_y =
                    y + (layout.row_height - layout.label_height - layout.value_height - 6.0) * 0.5;

                row_label(
                    ui,
                    label,
                    [layout.left, label_y, layout.width, layout.label_height],
                    selected,
                );

                let active_binding = GameButton::ALL
                    .get(index)
                    .filter(|_| menu == Menu::Controls)
                    .map(|button| {
                        let (keys, pads) = self.settings.controls.game_bindings(*button);

                        match self.input.active {
                            ActiveInput::Keyboard => format!("Keyboard: {}", binding_names(&keys)),
                            ActiveInput::Gamepad => format!("Controller: {}", binding_names(&pads)),
                        }
                    });

                let value = ui.text(
                    layout.left,
                    y + layout.label_height + 3.0,
                    layout.width,
                    15.0,
                    active_binding.as_deref().unwrap_or(value),
                    muted,
                );

                value.max_height = layout.value_height;
                value.after_text = Some((label_index, 6.0));
            } else {
                row_label(
                    ui,
                    label,
                    [layout.left, y, layout.width * 0.56, layout.row_height],
                    selected,
                );

                let value = ui.text(
                    layout.left + layout.width * 0.57,
                    y,
                    layout.width * 0.43,
                    15.0,
                    value,
                    muted,
                );

                value.align = TextAlign::Right;
                value.max_height = layout.row_height;
                value.vertically_centered = true;
            }
        }

        if self.remapping {
            ui.text(
                layout.left,
                layout.footer_top,
                layout.width,
                11.0,
                "Press a new key or button.\nEscape / East: cancel",
                muted,
            )
            .max_height = layout.prompt_height;
        } else {
            let gamepad = self.input.active == ActiveInput::Gamepad;
            let choose_console = menu == Menu::Consoles;

            let hints = if gamepad {
                [
                    ("D-pad", if choose_console { "Choose" } else { "Adjust" }),
                    ("South", "Select"),
                    ("East", "Back"),
                ]
            } else {
                [
                    (
                        if choose_console {
                            "Vertical arrows"
                        } else {
                            "Arrows"
                        },
                        if choose_console { "Choose" } else { "Adjust" },
                    ),
                    ("Enter", "Select"),
                    ("Escape", "Back"),
                ]
            };

            let start = usize::from(
                hints
                    .iter()
                    .map(|(control, label)| {
                        prompt_width(control, label, ui.text_scale, self.input.active)
                    })
                    .sum::<f32>()
                    > layout.width,
            );

            let mut x = layout.left;

            for &(control, label) in &hints[start..] {
                prompt_ui(
                    ui,
                    x,
                    layout.footer_top,
                    layout.prompt_height,
                    control,
                    label,
                    self.input.active,
                );

                x += prompt_width(control, label, ui.text_scale, self.input.active);
            }
        }

        if !status.is_empty() {
            notice_ui(
                ui,
                [
                    layout.left,
                    layout.footer_top + layout.prompt_height + 12.0,
                    layout.width,
                    layout.status_height,
                ],
                status,
                severity,
            );
        }
    }

    fn menu_rows(&self) -> Vec<(String, String)> {
        match self.menu {
            Some(Menu::Search) => Vec::new(),
            Some(Menu::Appearance) => vec![
                (
                    "Cartridge finish".into(),
                    self.entries
                        .get(self.selected)
                        .map_or("Default", |entry| self.entry_finish(entry).name)
                        .into(),
                ),
                ("Reset to default".into(), String::new()),
                ("Return".into(), String::new()),
            ],
            Some(Menu::Consoles) => self
                .available_consoles()
                .into_iter()
                .map(|(system, count)| {
                    (
                        system.name().into(),
                        format!(
                            "{count} game{}{}",
                            if count == 1 { "" } else { "s" },
                            if Some(system) == self.active_console() {
                                " · Current"
                            } else {
                                ""
                            }
                        ),
                    )
                })
                .collect(),
            Some(Menu::Settings) => SettingsRow::ALL
                .into_iter()
                .map(|row| (row.label().into(), self.settings_value(row)))
                .collect(),
            Some(Menu::Folders) => {
                let mut rows = vec![
                    ("Home folder".into(), String::new()),
                    (
                        "Use this folder".into(),
                        if self
                            .folder_listing
                            .as_ref()
                            .is_some_and(|l| !file_browser::is_drive_list(&l.path))
                        {
                            String::new()
                        } else {
                            "Unavailable".into()
                        },
                    ),
                ];

                if let Some(listing) = &self.folder_listing {
                    for entry in &listing.entries {
                        rows.push((
                            entry.name.clone(),
                            if entry.is_directory { "Folder" } else { "ROM" }.into(),
                        ));
                    }
                }

                rows
            }
            Some(Menu::Controls) => {
                let mut rows: Vec<_> = GameButton::ALL
                    .iter()
                    .map(|button| {
                        let (keys, pads) = self.settings.controls.game_bindings(*button);

                        let value = format!(
                            "Keys: {} / Pad: {}",
                            binding_names(&keys),
                            binding_names(&pads)
                        );

                        (format!("{button:?}"), value)
                    })
                    .collect();

                rows.push((
                    "Reset controls to defaults".into(),
                    "Keyboard and controller".into(),
                ));

                rows
            }
            Some(Menu::Pause) => [
                "Resume",
                "Quick save",
                "Quick load",
                "Settings",
                "Return to library",
            ]
            .iter()
            .map(|s| (s.to_string(), String::new()))
            .collect(),
            None => Vec::new(),
        }
    }

    fn settings_value(&self, row: SettingsRow) -> String {
        let percent = |value: f32| format!("{}%", (value * 100.0).round());
        let entry = self.entries.get(self.selected);

        match row {
            SettingsRow::ArtworkKey if self.editing_artwork_key => {
                format!("{}_", self.artwork_key_draft)
            }
            SettingsRow::ArtworkKey if self.artwork_key_draft.is_empty() => "Anonymous".into(),
            SettingsRow::ArtworkKey => "*".repeat(self.artwork_key_draft.len().min(32)),
            SettingsRow::ArtworkAccount => String::new(),
            SettingsRow::View => format!("{:?}", self.settings.layout),
            SettingsRow::Quality => format!("{:?}", self.settings.quality),
            SettingsRow::ReduceMotion => on_off(self.settings.reduced_motion),
            SettingsRow::TextSize => percent(self.settings.text_scale),
            SettingsRow::IntegerScaling => on_off(self.settings.integer_scaling),
            SettingsRow::Volume => percent(self.settings.volume),
            SettingsRow::Fullscreen => on_off(self.settings.fullscreen),
            SettingsRow::DisplayRelease => entry
                .and_then(|entry| self.settings.release_overrides.get(&entry.sha1))
                .map_or("ROM metadata", |market| market.label())
                .into(),
            SettingsRow::AddFolder => {
                format!("{} configured", self.settings.rom_directories.len())
            }
            SettingsRow::Controls => {
                let count = self.input.connected_gamepads();
                format!("{count} controller{}", if count == 1 { "" } else { "s" })
            }
            SettingsRow::GameAppearance => entry
                .map_or("Choose a game", |entry| self.entry_finish(entry).name)
                .into(),
            SettingsRow::Rescan | SettingsRow::SaveAndReturn => String::new(),
        }
    }

    fn settings_row(&self) -> Option<SettingsRow> {
        (self.menu == Some(Menu::Settings))
            .then(|| SettingsRow::ALL.get(self.menu_focus).copied())
            .flatten()
    }

    fn open_settings_at(&mut self, row: SettingsRow) {
        self.menu = Some(Menu::Settings);
        self.menu_focus = row.index();
    }

    fn close_settings(&mut self) {
        self.save_settings();
        self.menu = self.session.is_some().then_some(Menu::Pause);
        self.menu_focus = 0;
    }

    fn resume(&mut self) {
        self.menu = None;

        if let Some(session) = &mut self.session {
            session.set_paused(false);
        }
    }

    fn add_rom_directory(&mut self, path: PathBuf) {
        if !self.settings.rom_directories.contains(&path) {
            self.settings.rom_directories.push(path);
        }

        self.rescan();
        self.open_settings_at(SettingsRow::AddFolder);
        self.save_settings();
    }

    fn menu_action(&mut self, action: Action) -> bool {
        if self.editing_artwork_key {
            match action {
                Action::Launch => self.finish_artwork_key_edit(true),
                Action::Back => self.finish_artwork_key_edit(false),
                _ => {}
            }

            self.needs_redraw = true;
            return true;
        }
        if self.menu.is_none() {
            return false;
        }

        if self.menu == Some(Menu::Search) {
            self.search_action(action);
            return true;
        }

        if self.remapping {
            if action == Action::Back {
                self.input.cancel_remap();
                self.remapping = false;
                self.play_sound(Cue::Back);
            }

            return true;
        }

        let rows = self.menu_rows();
        let count = rows.len();
        let previous_menu = self.menu;
        let previous_focus = self.menu_focus;
        let previous_value = rows.get(self.menu_focus).map(|row| row.1.as_str());

        match action {
            Action::Console(delta) if self.menu == Some(Menu::Consoles) => {
                if self.cycle_console(delta) {
                    self.menu = None;
                    self.menu_focus = 0;
                }
            }
            Action::Navigate(delta) => {
                if count > 0 {
                    self.menu_focus = cycle(self.menu_focus, delta, count);
                }
            }
            Action::Adjust(delta) => self.adjust_setting(delta),
            Action::Launch => match self.menu {
                Some(Menu::Search) => unreachable!("search actions are handled above"),
                Some(Menu::Appearance) => match self.menu_focus {
                    0 => self.adjust_cartridge_finish(1),
                    1 => self.set_cartridge_finish(None),
                    2 => self.open_settings_at(SettingsRow::GameAppearance),
                    _ => {}
                },
                Some(Menu::Consoles) => {
                    if let Some((system, _)) =
                        self.available_consoles().get(self.menu_focus).copied()
                    {
                        self.activate_console(system);
                        self.menu = None;
                        self.menu_focus = 0;
                        self.clear_inputs();
                    }
                }
                Some(Menu::Settings) => match self.settings_row() {
                    Some(SettingsRow::ArtworkKey) => {
                        self.clear_inputs();
                        self.editing_artwork_key = true;
                    }
                    Some(SettingsRow::ArtworkAccount) => {
                        if let Err(error) = open::that(settings::ACCOUNT_URL) {
                            self.notice(Severity::Error, error.to_string());
                        }
                    }
                    Some(SettingsRow::AddFolder) => {
                        let path = DirectoryBrowser::default_start(
                            self.settings.rom_directories.first().map(|path| {
                                if path.is_file() {
                                    path.parent().unwrap_or(path)
                                } else {
                                    path.as_path()
                                }
                            }),
                        );

                        match DirectoryBrowser::new(path) {
                            Ok(browser) => {
                                self.browser = Some(browser);
                                self.folder_listing = None;
                                self.menu = Some(Menu::Folders);
                                self.menu_focus = 0;
                            }
                            Err(error) => self.notice(Severity::Error, error.to_string()),
                        }
                    }
                    Some(SettingsRow::Rescan) => self.rescan(),
                    Some(SettingsRow::Controls) => {
                        self.menu = Some(Menu::Controls);
                        self.menu_focus = 0;
                    }
                    Some(SettingsRow::GameAppearance) => {
                        if !self.entries.is_empty() {
                            self.menu = Some(Menu::Appearance);
                            self.menu_focus = 0;
                            self.status.clear();
                        }
                    }
                    Some(SettingsRow::SaveAndReturn) => self.close_settings(),
                    _ => self.adjust_setting(1),
                },
                Some(Menu::Folders) => {
                    if self.menu_focus == 0 {
                        if let Some(browser) = &mut self.browser {
                            browser.navigate(DirectoryBrowser::default_start(None));
                            self.folder_listing = None;
                        }
                    } else if let Some(listing) = &self.folder_listing {
                        if self.menu_focus == 1 {
                            if !file_browser::is_drive_list(&listing.path) {
                                self.add_rom_directory(listing.path.clone());
                            }
                        } else if let Some(entry) = listing.entries.get(self.menu_focus - 2) {
                            if !entry.is_directory {
                                self.add_rom_directory(entry.path.clone());
                            } else if let Some(browser) = &mut self.browser {
                                browser.navigate(entry.path.clone());
                                self.folder_listing = None;
                                self.menu_focus = 0;
                            }
                        }
                    }
                }
                Some(Menu::Controls) => {
                    if let Some(button) = GameButton::ALL.get(self.menu_focus) {
                        self.input.begin_remap(*button);
                        self.remapping = true;
                    } else {
                        self.settings.controls = crate::settings::Controls::default();
                        self.input.configure(&self.settings);

                        if self.save_settings() {
                            self.status.clear();
                        }
                    }
                }
                Some(Menu::Pause) => match self.menu_focus {
                    0 => self.resume(),
                    1 => self.quick_save(),
                    2 => self.quick_load(),
                    3 => {
                        self.menu = Some(Menu::Settings);
                        self.menu_focus = 0;
                    }
                    4 => {
                        self.menu = None;
                        self.return_to_library();
                    }
                    _ => {}
                },
                None => {}
            },
            Action::Consoles | Action::Pause if self.menu == Some(Menu::Consoles) => {
                self.menu = None;
                self.menu_focus = 0;
                self.clear_inputs();
            }
            Action::Pause if self.menu == Some(Menu::Pause) => self.resume(),
            Action::Back | Action::Settings => match self.menu {
                Some(Menu::Consoles) => {
                    self.menu = None;
                    self.menu_focus = 0;
                    self.clear_inputs();
                }
                Some(Menu::Controls | Menu::Folders | Menu::Appearance) => {
                    self.open_settings_at(match self.menu {
                        Some(Menu::Folders) => SettingsRow::AddFolder,
                        Some(Menu::Appearance) => SettingsRow::GameAppearance,
                        _ => SettingsRow::Controls,
                    });

                    self.browser = None;
                }
                Some(Menu::Pause) => self.resume(),
                _ => self.close_settings(),
            },
            Action::QuickSave | Action::QuickLoad => return false,
            _ => {}
        }

        let cue = match action {
            Action::Navigate(_) if self.menu_focus != previous_focus => Some(Cue::Navigate),
            Action::Adjust(_)
                if self
                    .menu_rows()
                    .get(self.menu_focus)
                    .map(|row| row.1.as_str())
                    != previous_value =>
            {
                Some(Cue::Adjust)
            }
            Action::Launch => Some(Cue::Select),
            Action::Console(_) if self.menu != previous_menu => Some(Cue::View),
            Action::Back | Action::Settings | Action::Pause | Action::Consoles
                if self.menu != previous_menu =>
            {
                Some(Cue::Back)
            }
            _ => None,
        };

        if let Some(cue) = cue {
            self.play_sound(cue);
        }

        self.needs_redraw = true;
        true
    }

    fn adjust_setting(&mut self, delta: i32) {
        if self.menu == Some(Menu::Appearance) {
            if self.menu_focus == 0 {
                self.adjust_cartridge_finish(delta);
            }

            return;
        }

        let Some(row) = self.settings_row() else {
            return;
        };

        let settings = &mut self.settings;

        match row {
            SettingsRow::View => {
                settings.layout = cycle_value(&ViewMode::ALL, settings.layout, delta);
                self.animation.set_view(settings.layout);
            }
            SettingsRow::Quality => {
                settings.quality = cycle_value(&Quality::ALL, settings.quality, delta);
                self.invalidate_artwork();
            }
            SettingsRow::ReduceMotion => {
                settings.reduced_motion = !settings.reduced_motion;
                self.animation.set_reduced_motion(settings.reduced_motion);
            }
            SettingsRow::TextSize => {
                settings.text_scale = (settings.text_scale + delta as f32 * 0.1).clamp(0.75, 2.0);
            }
            SettingsRow::IntegerScaling => settings.integer_scaling = !settings.integer_scaling,
            SettingsRow::Volume => {
                settings.volume = (settings.volume + delta as f32 * 0.05).clamp(0.0, 1.0);

                if let Some(session) = &mut self.session {
                    session.set_volume(settings.volume);
                }
            }
            SettingsRow::Fullscreen => settings.fullscreen = !settings.fullscreen,
            SettingsRow::DisplayRelease => {
                let Some(entry) = self.entries.get(self.selected) else {
                    return;
                };

                // None uses the ROM's own region.
                let choices = [None]
                    .into_iter()
                    .chain(ReleaseMarket::ALL.map(Some))
                    .collect::<Vec<_>>();

                let current = settings.release_overrides.get(&entry.sha1).copied();

                match cycle_value(&choices, current, delta) {
                    Some(market) => settings
                        .release_overrides
                        .insert(entry.sha1.clone(), market),
                    None => settings.release_overrides.remove(&entry.sha1),
                };

                self.invalidate_artwork();
            }
            _ => {}
        }
    }

    fn adjust_cartridge_finish(&mut self, delta: i32) {
        let Some(entry) = self.entries.get(self.selected) else {
            return;
        };

        let choices = crate::materials::cartridge_finishes(entry.system);
        let current = self.entry_finish(entry).id;

        let index = choices
            .iter()
            .position(|finish| finish.id == current)
            .unwrap_or(0);

        let next = &choices[cycle(index, delta, choices.len())];

        if next.id != current {
            self.set_cartridge_finish((next.id != "default").then_some(next.id));
        }
    }

    fn set_cartridge_finish(&mut self, finish: Option<&str>) {
        let Some(entry) = self.entries.get(self.selected) else {
            return;
        };

        self.settings.set_cartridge_finish(&entry.game_id(), finish);

        if self.save_settings() {
            self.status.clear();
        }

        self.needs_redraw = true;
    }

    fn notice(&mut self, severity: Severity, message: String) {
        self.status_severity = severity;
        self.status = message;
        self.status_until = Instant::now() + STATUS_DURATION;
        self.needs_redraw = true;
    }

    fn quick_save(&mut self) {
        if let Some(session) = &self.session {
            let result = session.quick_save();

            self.notice(
                Severity::Error,
                match result {
                    Ok(()) => String::new(),
                    Err(error) => format!("Save failed: {error}"),
                },
            );
        }
    }

    fn quick_load(&mut self) {
        if let Some(session) = &mut self.session {
            let result = session.quick_load();
            self.frame_sequence = None;

            match result {
                Ok(true) => self.status.clear(),
                Ok(false) => self.notice(Severity::Warning, "No quick state saved yet".into()),
                Err(error) => self.notice(Severity::Error, format!("Load failed: {error}")),
            }
        }
    }

    fn return_to_library(&mut self) {
        if let Some(mut session) = self.session.take() {
            session.clear_inputs();

            match session.persist() {
                Ok(_) => self.status.clear(),
                Err(error) => self.notice(Severity::Error, format!("Save failed: {error}")),
            }

            self.animation.return_to_library();
            self.game_size = None;
            self.frame_sequence = None;
            self.clear_inputs();
        }
    }
}

fn on_off(value: bool) -> String {
    if value { "On" } else { "Off" }.into()
}

fn binding_names(bindings: &[String]) -> String {
    if bindings.is_empty() {
        return "Unbound".into();
    }

    bindings
        .iter()
        .map(|key| key.strip_prefix("Key").unwrap_or(key))
        .collect::<Vec<_>>()
        .join(", ")
}

fn cycle(index: usize, delta: i32, len: usize) -> usize {
    (index as i64 + i64::from(delta)).rem_euclid(len as i64) as usize
}

fn cycle_value<T: Copy + PartialEq>(choices: &[T], current: T, delta: i32) -> T {
    let index = choices
        .iter()
        .position(|choice| *choice == current)
        .unwrap_or(0);

    choices[cycle(index, delta, choices.len())]
}

const UI_INK: [u8; 4] = [43, 39, 49, 255];
const UI_MUTED: [u8; 4] = [94, 89, 101, 255];

#[derive(Clone, Copy)]
struct UiTheme {
    accent: [u8; 4],
    soft: [u8; 4],
    info: [u8; 4],
    background: [u8; 4],
}

impl UiTheme {
    fn new(rgb: [u8; 3]) -> Self {
        // Darken the accent until white text on it meets AA contrast.
        let mut accent = rgb;
        let luminance = |color: [u8; 3]| luminance(color.map(srgb8_to_linear));

        let tint = |amount: f32| {
            let color =
                rgb.map(|channel| (250.0 + (f32::from(channel) - 250.0) * amount).round() as u8);
            [color[0], color[1], color[2], 255]
        };

        let soft = tint(0.18);
        let max_luminance =
            ((luminance([soft[0], soft[1], soft[2]]) + 0.05) / 4.5 - 0.05).min(0.17);

        while luminance(accent) > max_luminance {
            accent = accent.map(|channel| (f32::from(channel) * 0.96) as u8);
        }

        Self {
            accent: [accent[0], accent[1], accent[2], 255],
            soft,
            info: tint(0.14),
            background: tint(0.035),
        }
    }
}

fn capsule_height(scale: f32) -> f32 {
    (24.0 * scale + 12.0).max(36.0)
}

fn capsule_width(label: &str, scale: f32) -> f32 {
    (label.chars().count() as f32 * 7.0 * scale + 28.0).max(96.0)
}

fn capsule_ui(ui: &mut UiFrame, x: f32, y: f32, label: &str, theme: UiTheme) {
    let width = capsule_width(label, ui.text_scale);
    let height = capsule_height(ui.text_scale);
    ui.rect([x, y, width, height], theme.soft, 12.0);
    let text = ui.text(x + 14.0, y, width - 28.0, 13.0, label, theme.accent);
    text.font = FontFace::Semibold;
    text.max_height = height;
    text.vertically_centered = true;
    text.align = TextAlign::Center;
}

fn control_parts(control: &str) -> impl Iterator<Item = &str> {
    match control {
        "Arrows" => "ArrowLeft / ArrowRight",
        "Vertical arrows" => "ArrowUp / ArrowDown",
        _ => control,
    }
    .split(" / ")
}

fn control_width(control: &str, scale: f32, device: ActiveInput) -> f32 {
    control_parts(control)
        .map(|part| {
            if PromptIcon::for_control(part, device).is_some() {
                28.0
            } else {
                (keycap_label(part).chars().count() as f32 * 6.0 + 14.0).max(26.0)
            }
        })
        .sum::<f32>()
        * scale
}

fn keycap_label(control: &str) -> &str {
    match control {
        "Slash" => "/",
        "Backspace" => "Bksp",
        "Delete" => "Del",
        _ => control,
    }
}

fn control_ink_height(control: &str, device: ActiveInput) -> f32 {
    control_parts(control)
        .map(|part| PromptIcon::for_control(part, device).map_or(22.0, PromptIcon::ink_height))
        .fold(0.0, f32::max)
}

fn prompt_width(control: &str, label: &str, scale: f32, device: ActiveInput) -> f32 {
    control_width(control, scale, device) + (label.chars().count() as f32 * 7.0 + 14.0) * scale
}

fn prompt_ui(
    ui: &mut UiFrame,
    x: f32,
    y: f32,
    height: f32,
    control: &str,
    label: &str,
    device: ActiveInput,
) {
    let scale = ui.text_scale;
    let width = control_width(control, scale, device);
    let size = 28.0 * scale;
    let top = y + (height - size) * 0.5;
    let mut part_x = x;

    for part in control_parts(control) {
        let part_width = control_width(part, scale, device);

        if let Some(icon) = PromptIcon::for_control(part, device) {
            ui.icon(icon, part_x, top, size);
        } else {
            ui.rect(
                [part_x, top + 3.0 * scale, part_width, size - 6.0 * scale],
                UI_INK,
                4.0 * scale,
            );

            let key = ui.text(
                part_x + 3.0 * scale,
                top + 3.0 * scale,
                part_width - 6.0 * scale,
                10.0,
                keycap_label(part),
                [255; 4],
            );

            key.align = TextAlign::Center;
            key.vertically_centered = true;
            key.max_height = size - 6.0 * scale;
            key.font = FontFace::Semibold;
        }

        part_x += part_width;
    }

    let text = ui.text(
        x + width + 4.0 * scale,
        y,
        prompt_width(control, label, scale, device) - width - 6.0 * scale,
        14.0,
        label,
        UI_INK,
    );

    text.max_height = height;
    text.vertically_centered = true;
}

fn metadata_lines(meta: &game_database::GameMeta) -> (String, Option<String>) {
    let clean = |value: &Option<String>| {
        value
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };

    let players = meta.players_label().map(|players| {
        format!(
            "{players} {}",
            if meta.max_users == Some(1) {
                "player"
            } else {
                "players"
            }
        )
    });

    let summary = [meta.released_label(), players, clean(&meta.genre)]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");

    let maker = match (clean(&meta.developer), clean(&meta.publisher)) {
        (Some(developer), Some(publisher)) if developer.eq_ignore_ascii_case(&publisher) => {
            Some(developer)
        }
        (Some(developer), Some(publisher)) => Some(format!(
            "Developed by {developer} · Published by {publisher}"
        )),
        (Some(developer), None) => Some(format!("Developed by {developer}")),
        (None, Some(publisher)) => Some(format!("Published by {publisher}")),
        (None, None) => None,
    };

    (summary, maker)
}

fn compact_library([width, height]: [f32; 2], text_scale: f32) -> bool {
    width < 1100.0 * text_scale.sqrt() || height < 560.0 * text_scale.sqrt()
}

struct LibraryLayout {
    margin: f32,
    scene_top: f32,
    title_x: f32,
    title_width: f32,
    title_top: f32,
    title_size: f32,
    title_height: f32,
    info_padding: f32,
    footer_top: f32,
    footer_height: f32,
    footer_controls_top: f32,
    ribbon_lane_top: f32,
}

impl LibraryLayout {
    fn new([width, height]: [f32; 2], scale: f32, view: ViewMode) -> Self {
        let compact = compact_library([width, height], scale);
        let small = width < 640.0;
        let margin = if small { 16.0 } else { 24.0 };
        let scene_top = margin + capsule_height(scale) + 16.0;
        let info_padding = if small { 12.0 } else { 18.0 };

        let title_size = if small {
            17.0
        } else if compact {
            20.0
        } else if view == ViewMode::Ribbon {
            22.0
        } else {
            28.0
        };

        let lines = if small {
            3.0
        } else if compact || view == ViewMode::Ribbon {
            2.0
        } else {
            3.0
        };

        let title_height = title_size * scale * 1.3 * lines;
        let footer_height = capsule_height(scale);
        let footer_top = height - footer_height - margin;
        let ribbon_lane_top = height * 0.69;

        let title_top = if compact {
            footer_top
                - title_height
                - 15.6 * scale
                - 16.0
                - info_padding
                - if height >= 500.0 {
                    44.2 * scale + 23.0
                } else {
                    0.0
                }
        } else if view == ViewMode::Ribbon {
            ribbon_lane_top + info_padding
        } else {
            height * 0.51
        };

        let (title_x, title_width) = if compact {
            (
                margin + info_padding,
                width - margin * 2.0 - info_padding * 2.0,
            )
        } else if view == ViewMode::Ribbon {
            (width * 0.45, width - margin - width * 0.45 - info_padding)
        } else {
            (width * 0.685, width - margin - width * 0.685 - info_padding)
        };

        Self {
            margin,
            scene_top,
            title_x,
            title_width,
            title_top,
            title_size,
            title_height,
            info_padding,
            footer_top,
            footer_height,
            footer_controls_top: height - margin - 22.0 * scale,
            ribbon_lane_top,
        }
    }

    fn ribbon_details_center(&self) -> f32 {
        (self.ribbon_lane_top + self.footer_controls_top) * 0.5
    }
}

fn notice_height(text_scale: f32) -> f32 {
    14.0 * 1.3 * text_scale * 3.0 + 24.0
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Severity {
    Warning,
    Error,
}

impl Severity {
    fn colors(self) -> ([u8; 4], [u8; 4], [u8; 4]) {
        match self {
            Self::Warning => ([250, 196, 30, 120], [196, 136, 0, 255], [110, 72, 0, 255]),
            Self::Error => ([220, 60, 50, 64], [204, 52, 44, 255], [132, 30, 26, 255]),
        }
    }

    fn icon(self) -> StatusIcon {
        match self {
            Self::Warning => StatusIcon::Warning,
            Self::Error => StatusIcon::Error,
        }
    }
}

fn notice_ui(ui: &mut UiFrame, [x, y, width, height]: [f32; 4], message: &str, severity: Severity) {
    let (wash, icon_color, foreground) = severity.colors();
    ui.rect([x, y, width, height], [252, 250, 247, 200], 12.0);
    ui.rect([x, y, width, height], wash, 12.0);

    let icon = 20.0 * ui.text_scale;
    let icon_x = x + 16.0;
    ui.status_icon(
        severity.icon(),
        icon_x,
        y + ((height - icon) * 0.5),
        icon,
        icon_color,
    );

    let text_x = icon_x + icon + 12.0;
    let text = ui.text(
        text_x,
        y + 12.0,
        x + width - text_x - 16.0,
        14.0,
        message,
        foreground,
    );
    text.max_height = height - 24.0;
    text.vertically_centered = true;
}

struct MenuLayout {
    left: f32,
    top: f32,
    width: f32,
    heading_size: f32,
    body_top: f32,
    body_bottom: f32,
    row_height: f32,
    label_height: f32,
    value_height: f32,
    stacked: bool,
    subtitle_height: f32,
    footer_top: f32,
    prompt_height: f32,
    status_height: f32,
    preview: Option<[f32; 4]>,
}

#[derive(Default)]
struct MenuMotion {
    menu: Option<Menu>,
    focus: usize,
    position: f32,
    velocity: f32,
}

impl MenuMotion {
    fn advance(&mut self, menu: Option<Menu>, focus: usize, dt: f32, reduced_motion: bool) -> bool {
        let target = focus as f32;

        if menu != self.menu || reduced_motion || self.focus.abs_diff(focus) > 3 {
            self.menu = menu;
            self.focus = focus;
            self.position = target;
            self.velocity = 0.0;
            return false;
        }

        self.focus = focus;

        if menu.is_none() || (self.position - target).abs() < 0.001 && self.velocity.abs() < 0.005 {
            self.position = target;
            self.velocity = 0.0;
            return false;
        }

        // Closed-form underdamped spring, independent of frame rate.
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        let damping = 18.0;
        let frequency = 22.0;
        let offset = self.position - target;
        let sine = (frequency * dt).sin();
        let cosine = (frequency * dt).cos();
        let decay = (-damping * dt).exp();
        let combined = (self.velocity + damping * offset) / frequency;
        self.position = target + decay * (offset * cosine + combined * sine);

        self.velocity = decay
            * (self.velocity * cosine
                - (damping * self.velocity + (damping * damping + frequency * frequency) * offset)
                    / frequency
                    * sine);

        true
    }

    fn position(&self, menu: Menu, focus: usize) -> f32 {
        if self.menu == Some(menu) && self.focus == focus {
            self.position
        } else {
            focus as f32
        }
    }
}

impl MenuLayout {
    fn with_cartridge_preview(mut self, [width, height]: [f32; 2], text_scale: f32) -> Self {
        if width >= 1000.0 * text_scale.sqrt() && height >= 560.0 * text_scale.sqrt() {
            self.left = 48.0;
            self.width = width * 0.48 - self.left;
            self.stacked = true;
            self.label_height = 17.0 * 1.3 * text_scale;
            self.value_height = 15.0 * 2.6 * text_scale;
            self.row_height = self.label_height + self.value_height + 16.0;
            let x = width * 0.54;

            self.preview = Some([
                x,
                self.body_top,
                width - x - 24.0,
                self.body_bottom - self.body_top,
            ]);
        }

        self
    }

    fn new([width, height]: [f32; 2], text_scale: f32, subtitle: bool, status: bool) -> Self {
        let compact = width < 720.0 * text_scale || height < 500.0 * text_scale;
        let top = if compact { 16.0 } else { 36.0 };

        let left = if compact {
            24.0
        } else {
            ((width - 880.0) * 0.5).max(48.0)
        };

        let content_width = (width - 2.0 * left).max(1.0);
        let heading_size = if compact { 24.0 } else { 32.0 };
        let stacked = content_width < 640.0 * text_scale;
        let label_height = 17.0 * 1.3 * text_scale * if stacked { 2.0 } else { 1.0 };
        let value_height = 15.0 * 1.3 * text_scale;

        let row_height = if stacked {
            label_height + value_height + 16.0
        } else {
            44.0 * text_scale
        };

        let prompt_height = 11.0 * 1.3 * text_scale * 2.0;
        let status_height = if status {
            notice_height(text_scale)
        } else {
            0.0
        };
        let footer_top =
            height - top - prompt_height - status_height - if status { 12.0 } else { 0.0 };
        let body_bottom = footer_top - 12.0;
        let mut body_top = top + heading_size * 1.3 * text_scale + 16.0;
        let desired_subtitle_height = 13.0 * 1.3 * text_scale;

        let subtitle_height =
            if subtitle && body_bottom - body_top >= row_height + desired_subtitle_height + 6.0 {
                desired_subtitle_height
            } else {
                0.0
            };

        if subtitle_height > 0.0 {
            body_top += subtitle_height + 6.0;
        }

        Self {
            left,
            top,
            width: content_width,
            heading_size,
            body_top,
            body_bottom,
            row_height,
            label_height,
            value_height,
            stacked,
            subtitle_height,
            footer_top,
            prompt_height,
            status_height,
            preview: None,
        }
    }
}

const RESTING_ROW_TILT: f32 = -0.20;
const RESTING_COLUMN_TILT: f32 = 0.18;

fn resting_box_rotation(profile: &models::PresentationProfile, column: f32) -> Quat {
    let display = profile.model_to_display(profile.box_model);
    let spine = display.transform_vector3(Vec3::from_array(profile.box_spine_axis));
    let up = display.transform_vector3(Vec3::from_array(profile.box_spine_up_axis));
    let rotation = Quat::from_mat3(&Mat3::from_cols(up.cross(spine), up, spine).transpose());
    let row = Quat::from_rotation_x(-0.018) * Quat::from_rotation_y(RESTING_ROW_TILT) * rotation;

    let column_pose = Quat::from_rotation_x(RESTING_COLUMN_TILT)
        * Quat::from_rotation_z(-std::f32::consts::FRAC_PI_2)
        * rotation;

    row.slerp(column_pose, column)
}

fn selected_box_rotation(flip: f32, phase: f32, motion: f32) -> Quat {
    Quat::from_rotation_x(-0.04 + phase.sin() * motion * 0.5)
        * Quat::from_rotation_y(-0.085 + flip + (phase.cos() - 1.0) * motion * 0.35)
        * Quat::from_rotation_z(-0.025)
}

fn rotated_size(dimensions: Vec3, rotation: Quat) -> Vec3 {
    (rotation * Vec3::X).abs() * dimensions.x
        + (rotation * Vec3::Y).abs() * dimensions.y
        + (rotation * Vec3::Z).abs() * dimensions.z
}

fn box_display_scale(dimensions: Vec3, rotation: Quat, desired: f32, available_height: f32) -> f32 {
    desired.min((available_height / rotated_size(dimensions, rotation).y).max(0.2))
}

/// Scales small cartridges up to the on-screen width of a SNES cartridge.
fn cartridge_display_scale(profile: &models::PresentationProfile) -> f32 {
    profile
        .cartridge_dimensions_mm
        .as_ref()
        .map_or(1.0, |dimensions| 135.4 / dimensions[0])
}

fn box_viewport_opacity(projected: Mat4, dimensions: Vec3, clip: [f32; 4]) -> f32 {
    let mut lower = glam::Vec2::splat(f32::INFINITY);
    let mut upper = glam::Vec2::splat(f32::NEG_INFINITY);

    for x in [-0.5, 0.5] {
        for y in [-0.5, 0.5] {
            for z in [-0.5, 0.5] {
                let point = projected.project_point3(dimensions * Vec3::new(x, y, z));
                let uv = glam::Vec2::new(point.x * 0.5 + 0.5, 0.5 - point.y * 0.5);
                lower = lower.min(uv);
                upper = upper.max(uv);
            }
        }
    }

    let minimum = glam::Vec2::new(clip[0], clip[1]);
    let maximum = minimum + glam::Vec2::new(clip[2], clip[3]);
    let visible = (upper.min(maximum) - lower.max(minimum)).max(glam::Vec2::ZERO);
    let coverage = visible / (upper - lower).max(glam::Vec2::splat(0.00001));
    (coverage.x * coverage.y).clamp(0.0, 1.0)
}

pub fn game_rect(available: [u32; 2], size: [u32; 2], aspect: f32, integer: bool) -> [f32; 4] {
    let [width, height] = available.map(|v| v as f32);
    let native_width = size[1] as f32 * aspect;
    let fit = (width / native_width).min(height / size[1] as f32);

    let scale = if integer && fit >= 1.0 {
        fit.floor()
    } else {
        fit
    };

    let w = native_width * scale / width;
    let h = size[1] as f32 * scale / height;
    [(1.0 - w) * 0.5, (1.0 - h) * 0.5, w, h]
}
