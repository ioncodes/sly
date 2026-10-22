use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use cirsium::assets::icon;
use cirsium::components::{
    CheckboxSize, MenuItem, PanelChrome, Segmented, StatusView, TextInput, TextInputEvent,
    checkbox, combobox_menu, context_menu, field_button, field_text_button, icon_button, popover,
    status_bar, title_bar,
};
use cirsium::widgets::{
    AudioScopeView, AudioScopeWidget, AudioScopesView, BackgroundLayerView, BitmapView,
    BitmapWidget, BreakpointKind, BreakpointsWidget, CallStackView, CallStackWidget, CartridgeView,
    CartridgeWidget, ChannelScopesWidget, ControlsView, ControlsWidget, CpuWidget,
    DisassemblyWidget, FramebufferView, FramebufferWidget, GridHover, GridPanel, LogWidget,
    MemoryWidget, MmioView, MmioWidget, PaletteView, PaletteWidget, RowProvider, SavestatesWidget,
    ScriptBuiltinsWidget, ScriptConsoleWidget, ScriptEditorWidget, SpritesView, SpritesWidget,
    StructuredPanel, StructuredView, TablePanel, TableView, TilemapWidget, TilesetView,
    TilesetWidget, ViewMode,
};

use cirsium::{
    ACCENT_PRESETS, ChangeTracker, GUEST_COLORS, Highlight, Theme, ThemeMode, accent_at, set_theme,
};

use core_model::{Button, ExecutionState};
use debugger_model::{
    BreakpointMark, CpuId, CpuView, DebugCore, PaletteSelection, RunOutcome, VideoResource,
    VideoResourceId, VideoResourceKind, VideoRevision,
};

use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, ClipboardItem, Context, Entity, FocusHandle, KeyDownEvent,
    KeyUpEvent, MouseDownEvent, Pixels, Point, RenderImage, ScrollHandle, SharedString, Size, Task,
    UniformListScrollHandle, Window, div, px, rgb,
};
use web_time::Instant;
use workspace_model::{PanelDescriptor, PanelKind, RefreshPolicy, WorkspaceIcon};

use crate::breakpoints::Breakpoints;
use crate::disasm::Disasm;
use crate::logging::{LogBuffer, Logs};
use crate::memory::{self, Memory};
use crate::script::Script;

mod actions;
mod input;
mod panels;
mod preferences;
mod refresh;
mod render;
mod runtime;

use panels::{DebuggerPanelFactory, PanelFrames};

struct ProcessorPane {
    id: CpuId,
    title: SharedString,
    space: Arc<str>,
    cpu_view: Arc<cirsium::widgets::CpuView>,
    disasm: Disasm,
    call_stack: Arc<CallStackView>,
    reg_changes: ChangeTracker<Arc<str>>,
}

#[derive(Default)]
struct GridState {
    resource: Option<VideoResourceId>,
    selected: HashMap<VideoResourceId, usize>,
    hovered: Option<GridHover>,
    pinned: Option<GridHover>,
    viewport: Size<f32>,
    show_window_overlays: bool,
}

impl GridState {
    fn sync(&mut self, resources: &[VideoResource], kind: VideoResourceKind) {
        let current = resources
            .iter()
            .position(|r| r.kind == kind && Some(&r.id) == self.resource.as_ref());
        let next = resources
            .iter()
            .cycle()
            .skip(current.unwrap_or(0))
            .take(resources.len())
            .find(|r| r.kind == kind && r.enabled)
            .map(|r| r.id.clone());

        if next != self.resource {
            self.resource = next;
            self.hovered = None;
            self.pinned = None;
        }
    }

    fn index(&self, resources: &[VideoResource]) -> usize {
        resources
            .iter()
            .position(|r| Some(&r.id) == self.resource.as_ref())
            .unwrap_or(0)
    }

    fn selected(&self) -> Option<usize> {
        self.resource
            .as_ref()
            .and_then(|id| self.selected.get(id))
            .copied()
    }

    fn toggle_selected(&mut self, index: usize, pin: bool) {
        let Some(resource) = self.resource.clone() else {
            return;
        };

        if self.selected.get(&resource) == Some(&index) {
            self.selected.remove(&resource);
            self.pinned = None;
        } else {
            self.selected.insert(resource, index);
            // Selecting a cell can keep its hover popup open where it was clicked.
            self.pinned = self.hovered.filter(|hover| pin && hover.index == index);
        }
    }

    fn set_hovered(&mut self, hovered: Option<GridHover>) -> bool {
        let changed = self.hovered != hovered;
        if changed {
            self.hovered = hovered;
        }
        changed
    }

    fn set_viewport(&mut self, viewport: Size<f32>) -> bool {
        let changed = (self.viewport.width - viewport.width).abs() > 0.5
            || (self.viewport.height - viewport.height).abs() > 0.5;

        if changed {
            self.viewport = viewport;
            // The pin is anchored in window space, so it would drift away from its cell.
            self.pinned = None;
        }

        changed
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct VideoCacheKey {
    resource: VideoResourceId,
    revision: VideoRevision,
    palette: Option<PaletteSelection>,
    palette_revision: Option<VideoRevision>,
}

impl VideoCacheKey {
    fn read(
        video: &dyn debugger_model::VideoDebug,
        resource: &VideoResourceId,
        palette: Option<PaletteSelection>,
    ) -> Option<Self> {
        let palette_revision = match &palette {
            Some(selection) => Some(video.revision(&selection.resource)?),
            None => None,
        };

        Some(Self {
            resource: resource.clone(),
            revision: video.revision(resource)?,
            palette,
            palette_revision,
        })
    }
}

pub(crate) struct Debugger {
    target: Box<dyn DebugCore>,
    session: storage::Session<storage::PlatformStore>,
    saves_ready: bool,
    active_slot: storage::Slot,
    savestates_view: Arc<cirsium::widgets::SavestatesView>,
    cartridge: Arc<CartridgeView>,
    mmio_view: Arc<MmioView>,
    mmio_scroll: UniformListScrollHandle,
    palette_view: Arc<PaletteView>,
    palette_selected: Option<(usize, usize)>,
    structured_views: HashMap<SharedString, Arc<StructuredView>>,
    panel_scrolls: HashMap<String, ScrollHandle>,
    table_views: HashMap<SharedString, Arc<TableView>>,
    table_selection: HashMap<SharedString, usize>,
    scopes_views: HashMap<SharedString, AudioScopesView>,
    scope_samples: Vec<f32>,
    scope_view: AudioScopeView,
    audio_samples: Vec<i16>,
    tileset: Option<TilesetView>,
    video_resources: Vec<VideoResource>,
    palette_ui: GridState,
    bitmap: Option<BitmapView>,
    bitmap_ui: GridState,
    bitmap_key: Option<VideoCacheKey>,
    tileset_ui: GridState,
    tilemap: Option<BackgroundLayerView>,
    tilemap_ui: GridState,
    sprites: Option<SpritesView>,
    sprites_ui: GridState,
    framebuffer: Option<FramebufferView>,
    palette_key: Option<VideoCacheKey>,
    tileset_key: Option<VideoCacheKey>,
    tilemap_key: Option<VideoCacheKey>,
    tilemap_overlay_key: Option<VideoCacheKey>,
    sprites_key: Option<VideoCacheKey>,
    expanded: HashMap<SharedString, HashSet<SharedString>>,
    registry: cirsium::PanelRegistry,
    step_targets: Vec<cirsium::widgets::StepTargetView>,
    panel_frames: PanelFrames,
    status: StatusView,
    panes: Vec<ProcessorPane>,
    memory: Memory,
    breakpoints: Breakpoints,
    logs: Logs,
    show_log: bool,
    jump_input: Entity<TextInput>,
    bp_input: Entity<TextInput>,
    log_filter_input: Entity<TextInput>,
    script: Script,
    hook_marks: Vec<BreakpointMark>,
    script_editor: Entity<cirsium::components::CodeEditor>,
    script_file_name: SharedString,
    #[cfg(target_family = "wasm")]
    script_external: bool,
    #[cfg(not(target_family = "wasm"))]
    script_path: Option<std::path::PathBuf>,
    builtin_filter_input: Entity<TextInput>,
    builtin_filter: SharedString,
    collapsed: HashSet<SharedString>,
    settings: Option<Point<Pixels>>,
    artwork_key_input: Entity<TextInput>,
    art_query: Option<crate::cover::ArtQuery>,
    live_refresh: bool,
    grid_inline_details: bool,
    grid_hover_details: bool,
    grid_pin_popup: bool,
    grid_fill_width: HashSet<String>,
    workspace: SharedString,
    execution: ExecutionState,
    game_focus: FocusHandle,
    pressed_buttons: Vec<Button>,
    run_task: Option<Task<()>>,
    audio: crate::audio_out::AudioOut,
    stats: crate::stats::RunStats,
    perf: Vec<SharedString>,
    emu_clock: Instant,
    sram_flushed: Instant,
    accent: usize,
    #[cfg(target_family = "wasm")]
    web_tick_armed: bool,
}

impl Drop for Debugger {
    fn drop(&mut self) {
        self.persist_save_ram();
    }
}

const DEFAULT_FRAME_INTERVAL: Duration = Duration::from_millis(16);
const MAX_CATCHUP_FRAMES: u32 = 3;
const SAVE_RAM_FLUSH: Duration = Duration::from_secs(60);
#[cfg(not(target_family = "wasm"))]
const FRAME_PACE_YIELD: Duration = Duration::from_millis(1);

#[derive(Clone, Copy, PartialEq, Eq)]
enum CpuRefresh {
    Highlight,
    Baseline,
}

#[derive(Default)]
struct GenericRefresh {
    tables: Vec<SharedString>,
    structured: Vec<(SharedString, Option<usize>)>,
    scopes: Vec<SharedString>,
}

impl Debugger {
    pub(crate) fn new(
        target: Box<dyn DebugCore>,
        game: storage::GameId,
        log_buffer: LogBuffer,
        cx: &mut Context<Self>,
    ) -> Self {
        let panes: Vec<ProcessorPane> = target
            .cpus()
            .into_iter()
            .map(|desc| {
                let mut pane = ProcessorPane {
                    disasm: Disasm::new(&desc),
                    id: desc.id,
                    title: desc.title.into(),
                    space: desc.space,
                    cpu_view: Arc::default(),
                    call_stack: Arc::default(),
                    reg_changes: ChangeTracker::default(),
                };
                Self::refresh_pane_view(target.as_ref(), &mut pane, CpuRefresh::Baseline);
                pane
            })
            .collect();
        assert!(
            !panes.is_empty(),
            "a DebugCore must expose at least one processor"
        );

        let jump_input = cx.new(|cx| TextInput::new(cx, "go to...").hex_only(true).max_len(8));
        cx.subscribe(&jump_input, Self::on_jump_event).detach();

        let bp_input = cx.new(|cx| TextInput::new(cx, "address...").hex_only(true).max_len(8));
        cx.subscribe(&bp_input, Self::on_bp_event).detach();

        let log_filter_input = cx.new(|cx| TextInput::new(cx, "filter messages..."));
        cx.subscribe(&log_filter_input, Self::on_log_filter_event)
            .detach();

        let script_editor = cx.new(|cx| {
            let mut editor = cirsium::components::CodeEditor::new(
                cx,
                crate::script::default_script(game.system),
            );
            editor.set_completions(crate::script::api_completions());
            editor
        });
        cx.subscribe(&script_editor, Self::on_script_editor_event)
            .detach();

        let builtin_filter_input = cx.new(|cx| TextInput::new(cx, "filter builtins..."));
        cx.subscribe(&builtin_filter_input, Self::on_builtin_filter_event)
            .detach();

        let mut breakpoints = Breakpoints::default();
        breakpoints.set_cpu_titles(
            target
                .cpus()
                .into_iter()
                .map(|c| (c.id, SharedString::from(c.title)))
                .collect(),
        );

        let step_targets: Vec<cirsium::widgets::StepTargetView> = target
            .step_targets()
            .into_iter()
            .map(|t| cirsium::widgets::StepTargetView {
                id: SharedString::from(t.id),
                label: SharedString::from(t.label),
            })
            .collect();
        let manifest = panels::prepare_manifest(target.manifest());
        let workspace = manifest
            .workspaces
            .first()
            .map(|w| SharedString::from(w.id.clone()))
            .unwrap_or_default();

        let audio = crate::audio_out::AudioOut::new(target.audio_spec());
        let panel_frames = PanelFrames::default();
        let mut panel_factory = DebuggerPanelFactory::new(panel_frames.clone());
        let registry = cirsium::PanelRegistry::new(manifest, &mut panel_factory, cx)
            .expect("invalid component manifest");
        let panel_scrolls = registry
            .manifest()
            .panels
            .iter()
            .map(|panel| (panel.id.to_string(), ScrollHandle::new()))
            .collect();
        let mut status: StatusView = target.session_info().into();
        status.state = target.caption("paused").into();

        let mut debugger = Self {
            cartridge: Arc::new(target.cartridge().clone().into()),
            mmio_view: Arc::new(target.mmio_view().into()),
            mmio_scroll: UniformListScrollHandle::new(),
            palette_view: Arc::new(PaletteView::default()),
            palette_selected: None,
            structured_views: HashMap::new(),
            panel_scrolls,
            table_views: HashMap::new(),
            table_selection: HashMap::new(),
            scopes_views: HashMap::new(),
            scope_samples: Vec::new(),
            scope_view: AudioScopeView::default(),
            audio_samples: Vec::new(),
            tileset: None,
            video_resources: Vec::new(),
            palette_ui: GridState::default(),
            bitmap: None,
            bitmap_ui: GridState::default(),
            bitmap_key: None,
            tileset_ui: GridState::default(),
            tilemap: None,
            tilemap_ui: GridState {
                show_window_overlays: true,
                ..GridState::default()
            },
            sprites: None,
            sprites_ui: GridState::default(),
            framebuffer: target.software_frame().map(Into::into),
            palette_key: None,
            tileset_key: None,
            tilemap_key: None,
            tilemap_overlay_key: None,
            sprites_key: None,
            expanded: HashMap::new(),
            registry,
            step_targets,
            panel_frames,
            status,
            memory: Memory::new(target.as_ref()),
            breakpoints,
            logs: Logs::new(log_buffer),
            show_log: false,
            jump_input,
            bp_input,
            log_filter_input,
            script: Script::new(target.as_ref()),
            hook_marks: Vec::new(),
            script_editor,
            script_file_name: storage::SCRIPT_FILE.into(),
            #[cfg(target_family = "wasm")]
            script_external: false,
            #[cfg(not(target_family = "wasm"))]
            script_path: None,
            builtin_filter_input,
            builtin_filter: SharedString::default(),
            target,
            session: storage::Session::new(storage::PlatformStore::default(), game),
            saves_ready: false,
            active_slot: storage::Slot::Quick,
            savestates_view: Arc::default(),
            panes,
            collapsed: HashSet::from([SharedString::from(LogWidget::ID)]),
            settings: None,
            artwork_key_input: cx.new(|cx| TextInput::new(cx, "Anonymous").secret()),
            art_query: None,
            live_refresh: true,
            grid_inline_details: true,
            grid_hover_details: false,
            grid_pin_popup: false,
            grid_fill_width: HashSet::new(),
            workspace,
            execution: ExecutionState::Paused,
            game_focus: cx.focus_handle(),
            pressed_buttons: Vec::new(),
            run_task: None,
            audio,
            stats: crate::stats::RunStats::default(),
            perf: Vec::new(),
            emu_clock: Instant::now(),
            sram_flushed: Instant::now(),
            accent: 0,
            #[cfg(target_family = "wasm")]
            web_tick_armed: false,
        };

        debugger.load_preferences(cx);
        debugger.refresh_panes(CpuRefresh::Baseline, false, cx);
        debugger
    }

    fn scroll_for(&self, id: &str) -> ScrollHandle {
        self.panel_scrolls.get(id).cloned().unwrap_or_default()
    }

    pub(crate) fn set_cover_art(&mut self, cover: Arc<str>, cx: &mut Context<Self>) {
        self.target.set_cover_art(cover);
        self.cartridge = Arc::new(self.target.cartridge().clone().into());
        cx.notify();
    }

    pub(crate) fn resolve_cover_art(
        &mut self,
        art: crate::cover::ArtQuery,
        cx: &mut Context<Self>,
    ) {
        self.art_query = Some(art.clone());
        let client = cx.http_client();

        cx.spawn(async move |this, cx| {
            if let Some(cover) = crate::cover::resolve(client, &art).await {
                let _ = this.update(cx, |this, cx| this.set_cover_art(cover, cx));
            }
        })
        .detach();
    }
}
