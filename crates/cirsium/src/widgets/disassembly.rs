use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    App, Bounds, ClickEvent, Div, Hsla, MouseButton, MouseMoveEvent, PathBuilder, Pixels, Point,
    SharedString, Size, Stateful, UniformListScrollHandle, Window, canvas, div, point, px, size,
    svg, uniform_list,
};

use super::breakpoints::{BreakpointKind, breakpoint_color};
use crate::assets::icon;
use crate::components::{ClickHandler, Panel, Segmented, uniform_vertical_scroll_area};
use crate::theme::Theme;

mod arrows;
mod graph;
mod graph_paint;
mod graph_view;
mod linear;
mod model;
mod row;

use arrows::{GutterCell, arrow_gutters, paint_arrows};
use graph::{EdgeKind, GraphEdge, GraphLayout};
pub use model::*;

const ROW_HEIGHT: f32 = 20.;
const MIN_LIST_HEIGHT: f32 = 120.;
const BYTES_WIDTH: f32 = 96.;
const GRAPH_TOP_PAD: f32 = 16.;
const GRAPH_BOTTOM_PAD: f32 = 28.;
const LINEAR_MNEMONIC_W: f32 = 42.;
const GRAPH_MNEMONIC_W: f32 = 36.;
const MNEMONIC_PAD: f32 = 24.;

type SelectCallback = Rc<dyn Fn(usize, &mut Window, &mut App) + 'static>;
type ContextCallback = Rc<dyn Fn(usize, Point<Pixels>, &mut Window, &mut App) + 'static>;
type ViewCallback = Rc<dyn Fn(ViewMode, &mut Window, &mut App) + 'static>;
type PanCallback = Rc<dyn Fn(Point<Pixels>, &mut Window, &mut App) + 'static>;
type ViewportCallback = Rc<dyn Fn(Size<f32>, &mut Window, &mut App) + 'static>;
type EdgeHoverCallback = Rc<dyn Fn(Option<usize>, &mut Window, &mut App) + 'static>;
type JumpCallback = Rc<dyn Fn(Point<Pixels>, Point<Pixels>, &mut Window, &mut App) + 'static>;

fn addr_index(lines: &[DisasmLine]) -> HashMap<u32, usize> {
    let mut index = HashMap::with_capacity(lines.len());
    for (i, line) in lines.iter().enumerate() {
        index.entry(line.addr).or_insert(i);
    }
    index
}

#[derive(Clone, Default)]
pub struct DisassemblyView {
    pub lines: Vec<DisasmLine>,
    pub show_bytes: bool,
}

#[derive(Default)]
pub struct DisassemblyLayoutCache {
    glyphs: GlyphMetrics,
    gutters: Option<GutterEntry>,
    graph: Option<GraphEntry>,
    mnemonic: Option<(Arc<DisassemblyView>, f32)>,
}

#[derive(Clone, Copy, Default, PartialEq)]
struct GlyphMetrics {
    code: f32,
    note: f32,
}

impl GlyphMetrics {
    fn measure(window: &Window, font: &'static str) -> Self {
        let text = window.text_system();
        let font_id = text.resolve_font(&gpui::font(font));
        let rem = window.rem_size();
        let advance = |scale: f32| {
            text.advance(font_id, rem * scale, 'm')
                .map_or(f32::from(rem) * scale * 0.6, |advance| {
                    f32::from(advance.width)
                })
        };

        Self {
            code: advance(0.875),
            note: advance(0.75),
        }
    }
}

struct GutterEntry {
    lines: Arc<DisassemblyView>,
    selected: Option<usize>,
    cells: Rc<(Vec<GutterCell>, f32)>,
}

struct GraphEntry {
    lines: Arc<DisassemblyView>,
    layout: Rc<GraphLayout>,
}

impl DisassemblyLayoutCache {
    fn set_glyphs(&mut self, glyphs: GlyphMetrics) {
        if self.glyphs != glyphs {
            self.glyphs = glyphs;
            self.graph = None;
            self.mnemonic = None;
        }
    }

    fn gutters(
        &mut self,
        view: &Arc<DisassemblyView>,
        selected: Option<usize>,
    ) -> Rc<(Vec<GutterCell>, f32)> {
        if let Some(entry) = &self.gutters
            && Arc::ptr_eq(&entry.lines, view)
            && entry.selected == selected
        {
            return entry.cells.clone();
        }

        let cells = Rc::new(arrow_gutters(&view.lines, selected));
        self.gutters = Some(GutterEntry {
            lines: view.clone(),
            selected,
            cells: cells.clone(),
        });

        cells
    }

    fn graph(&mut self, view: &Arc<DisassemblyView>) -> Rc<GraphLayout> {
        if let Some(entry) = &self.graph
            && Arc::ptr_eq(&entry.lines, view)
        {
            return entry.layout.clone();
        }

        let mnemonic_w = self.mnemonic_width(view).max(GRAPH_MNEMONIC_W);
        let layout = Rc::new(graph::build(&view.lines, mnemonic_w, self.glyphs));
        self.graph = Some(GraphEntry {
            lines: view.clone(),
            layout: layout.clone(),
        });
        layout
    }

    fn mnemonic_width(&mut self, view: &Arc<DisassemblyView>) -> f32 {
        if let Some((lines, width)) = &self.mnemonic
            && Arc::ptr_eq(lines, view)
        {
            return *width;
        }

        let width = view
            .lines
            .iter()
            .filter_map(|line| line.tokens.first())
            .filter(|token| token.kind != TokenKind::Plain)
            .map(|token| token.text.chars().count() as f32 * self.glyphs.code + MNEMONIC_PAD)
            .fold(0., f32::max);
        self.mnemonic = Some((view.clone(), width));

        width
    }
}

#[derive(IntoElement)]
pub struct DisassemblyWidget {
    view: Arc<DisassemblyView>,
    scroll: UniformListScrollHandle,
    id: SharedString,
    title: SharedString,
    collapsed: bool,
    rows: row::RowStates,
    layout_cache: Rc<RefCell<DisassemblyLayoutCache>>,
    on_toggle: Option<ClickHandler>,
    on_select: Option<SelectCallback>,
    on_context: Option<ContextCallback>,
    view_mode: ViewMode,
    graph_offset: Point<Pixels>,
    on_set_view: Option<ViewCallback>,
    decode: Option<Segmented>,
    on_pan_begin: Option<PanCallback>,
    on_pan_drag: Option<PanCallback>,
    follow: bool,
    graph_viewport: Size<f32>,
    on_viewport: Option<ViewportCallback>,
    hovered_edge: Option<usize>,
    on_edge_hover: Option<EdgeHoverCallback>,
    on_jump: Option<JumpCallback>,
}

impl DisassemblyWidget {
    pub const ID: &'static str = "disassembly";

    pub fn new(view: Arc<DisassemblyView>, scroll: UniformListScrollHandle) -> Self {
        Self {
            view,
            scroll,
            id: Self::ID.into(),
            title: "Disassembly".into(),
            collapsed: false,
            rows: row::RowStates::default(),
            layout_cache: Rc::default(),
            on_toggle: None,
            on_select: None,
            on_context: None,
            view_mode: ViewMode::Linear,
            graph_offset: Point::default(),
            on_set_view: None,
            decode: None,
            on_pan_begin: None,
            on_pan_drag: None,
            follow: false,
            graph_viewport: size(0., 0.),
            on_viewport: None,
            hovered_edge: None,
            on_edge_hover: None,
            on_jump: None,
        }
    }

    pub fn decode_modes(
        mut self,
        modes: &[debugger_model::DecodeMode],
        selected: Option<&str>,
        handler: impl Fn(Option<&str>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.decode = (!modes.is_empty()).then(|| {
            let mut selector =
                Segmented::new(format!("{}-decode", self.id)).option_label("live", "Live");
            for mode in modes {
                selector = selector.option_label(format!("mode:{}", mode.id), mode.label.clone());
            }
            selector
                .selected(selected.map_or_else(|| "live".to_string(), |id| format!("mode:{id}")))
                .on_select(move |key, window, cx| {
                    handler(key.strip_prefix("mode:"), window, cx);
                })
        });
        self
    }

    pub fn current(mut self, current: Option<usize>) -> Self {
        self.rows.current = current;
        self
    }

    pub fn pipeline(mut self, pipeline: Option<usize>) -> Self {
        self.rows.pipeline = pipeline;
        self
    }

    pub fn current_note(mut self, note: Option<SharedString>) -> Self {
        self.rows.current_note = note;
        self
    }

    pub fn selected(mut self, selected: Option<usize>) -> Self {
        self.rows.selected = selected;
        self
    }

    pub fn breakpoints(mut self, breakpoints: Rc<HashMap<usize, BreakpointKind>>) -> Self {
        self.rows.breakpoints = breakpoints;
        self
    }

    pub fn layout_cache(mut self, cache: Rc<RefCell<DisassemblyLayoutCache>>) -> Self {
        self.layout_cache = cache;
        self
    }

    pub fn id(mut self, id: impl Into<SharedString>) -> Self {
        self.id = id.into();
        self
    }

    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = title.into();
        self
    }

    pub fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }

    pub fn on_toggle(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_toggle = Some(Box::new(handler));
        self
    }

    pub fn on_select(mut self, handler: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Rc::new(handler));
        self
    }

    pub fn on_context(
        mut self,
        handler: impl Fn(usize, Point<Pixels>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_context = Some(Rc::new(handler));
        self
    }

    pub fn view_mode(mut self, mode: ViewMode) -> Self {
        self.view_mode = mode;
        self
    }

    pub fn graph_offset(mut self, offset: Point<Pixels>) -> Self {
        self.graph_offset = offset;
        self
    }

    pub fn on_set_view(
        mut self,
        handler: impl Fn(ViewMode, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_set_view = Some(Rc::new(handler));
        self
    }

    pub fn on_pan_begin(
        mut self,
        handler: impl Fn(Point<Pixels>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_pan_begin = Some(Rc::new(handler));
        self
    }

    pub fn on_pan_drag(
        mut self,
        handler: impl Fn(Point<Pixels>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_pan_drag = Some(Rc::new(handler));
        self
    }

    pub fn follow(mut self, follow: bool) -> Self {
        self.follow = follow;
        self
    }

    pub fn graph_viewport(mut self, viewport: Size<f32>) -> Self {
        self.graph_viewport = viewport;
        self
    }

    pub fn on_viewport(
        mut self,
        handler: impl Fn(Size<f32>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_viewport = Some(Rc::new(handler));
        self
    }

    pub fn hovered_edge(mut self, edge: Option<usize>) -> Self {
        self.hovered_edge = edge;
        self
    }

    pub fn on_edge_hover(
        mut self,
        handler: impl Fn(Option<usize>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_edge_hover = Some(Rc::new(handler));
        self
    }

    pub fn on_jump(
        mut self,
        handler: impl Fn(Point<Pixels>, Point<Pixels>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_jump = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for DisassemblyWidget {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let DisassemblyWidget {
            view,
            scroll,
            id,
            title,
            collapsed,
            rows,
            layout_cache,
            view_mode,
            graph_offset,
            on_toggle,
            on_select,
            on_context,
            on_set_view,
            decode,
            on_pan_begin,
            on_pan_drag,
            follow,
            graph_viewport,
            on_viewport,
            hovered_edge,
            on_edge_hover,
            on_jump,
        } = self;

        let current_addr = rows
            .current
            .and_then(|ix| view.lines.get(ix))
            .map(|l| SharedString::from(l.address.clone()));

        let selector = Segmented::new(SharedString::from(format!("{id}-view")))
            .option("linear", icon::LIST, "Linear")
            .option("graph", icon::GRAPH, "Graph")
            .selected(view_mode.key())
            .on_select(move |key, window, cx| {
                if let Some(cb) = &on_set_view {
                    cb(ViewMode::from_key(key), window, cx);
                }
            });

        let theme = Theme::global(cx);
        let t = &*theme;

        layout_cache
            .borrow_mut()
            .set_glyphs(GlyphMetrics::measure(window, t.mono_font));

        let body = match view_mode {
            ViewMode::Linear => {
                let list = self::linear::render_linear(
                    view,
                    scroll.clone(),
                    id.clone(),
                    rows,
                    layout_cache,
                    on_select,
                    on_context,
                );
                uniform_vertical_scroll_area(list, &scroll, t)
                    .flex_1()
                    .into_any_element()
            }
            ViewMode::Graph => self::graph_view::render_graph(
                &theme,
                view,
                id.clone(),
                rows,
                layout_cache,
                graph_offset,
                follow,
                graph_viewport,
                hovered_edge,
                on_select,
                on_context,
                on_pan_begin,
                on_pan_drag,
                on_viewport,
                on_edge_hover,
                on_jump,
            )
            .into_any_element(),
        };

        let mut actions = div().flex().items_center().gap_1();
        if let Some(decode) = decode {
            actions = actions.child(decode);
        }

        actions = actions.child(selector);

        let mut panel = Panel::new(id, title)
            .glyph(icon::CODE)
            .collapsed(collapsed)
            .grow(true)
            .on_toggle_opt(on_toggle)
            .header_action(actions)
            .child(body);

        if let Some(addr) = current_addr {
            panel = panel.meta(addr);
        }

        panel
    }
}
