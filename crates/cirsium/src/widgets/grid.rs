use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    Anchor, AnchoredPositionMode, App, Bounds, ClickEvent, Div, Hsla, ImageFiltering,
    MouseMoveEvent, Pixels, Point, RenderImage, SharedString, Size, Window, anchored, canvas,
    deferred, div, fill, img, point, px, size,
};

use crate::components::{
    CheckboxSize, ClickHandler, FOOTER_ROW_H, Panel, Segmented, checkbox, empty_state,
    footer_placeholder,
};
use crate::theme::{Theme, with_alpha};

mod image_grid;

pub(super) type SelectCallback = Rc<dyn Fn(usize, &mut Window, &mut App) + 'static>;
pub(super) type HoverCallback = Rc<dyn Fn(Option<GridHover>, &mut Window, &mut App) + 'static>;
pub(super) type ViewportCallback = Rc<dyn Fn(Size<f32>, &mut Window, &mut App) + 'static>;
type ToggleCallback = Rc<dyn Fn(&mut Window, &mut App) + 'static>;

type DetailFn = Box<dyn Fn(&Theme, usize) -> Div + 'static>;

const DETAILS_WIDTH: f32 = 256.;
const DETAILS_BREAKPOINT: f32 = 640.;
const DETAILS_GAP: f32 = 12.;

#[derive(Clone, Copy, PartialEq)]
pub struct GridHover {
    pub index: usize,
    pub position: Point<Pixels>,
}

pub use debugger_model::VideoResource;

pub(super) fn build_atlas<'a>(
    cols: usize,
    rows: usize,
    cell_px: u32,
    cells: impl Iterator<Item = (&'a [[u8; 4]], u32, u32)>,
) -> Arc<RenderImage> {
    let mut atlas = image::RgbaImage::new(cols as u32 * cell_px, rows as u32 * cell_px);

    let mut row = vec![0u8; (cell_px * 4) as usize];
    for (i, (pixels, width, height)) in cells.enumerate() {
        let x = (i % cols) as u32 * cell_px + (cell_px - width) / 2;
        let y = (i / cols) as u32 * cell_px + (cell_px - height) / 2;
        self::blit(&mut atlas, &mut row, x, y, pixels, width, height);
    }

    Arc::new(RenderImage::new(vec![image::Frame::new(atlas)]))
}

pub(in crate::widgets) fn build_image(
    pixels: &[[u8; 4]],
    width: u32,
    height: u32,
) -> Arc<RenderImage> {
    let mut image = image::RgbaImage::new(width, height);
    self::bgra_into(pixels, &mut image);
    Arc::new(RenderImage::new(vec![image::Frame::new(image)]))
}

fn bgra_into(pixels: &[[u8; 4]], out: &mut [u8]) {
    for (dst, &src) in out.as_chunks_mut::<4>().0.iter_mut().zip(pixels) {
        *dst = self::bgra(src);
    }
}

pub(in crate::widgets) fn bgra([r, g, b, a]: [u8; 4]) -> [u8; 4] {
    [b, g, r, a]
}

fn blit(
    atlas: &mut image::RgbaImage,
    row: &mut [u8],
    x0: u32,
    y0: u32,
    pixels: &[[u8; 4]],
    width: u32,
    height: u32,
) {
    let atlas_width = atlas.width() as usize;
    let data: &mut [u8] = atlas;
    let (x0, y0) = (x0 as usize, y0 as usize);
    let (width, height) = (width as usize, height as usize);
    let row = &mut row[..width * 4];

    for sy in 0..height {
        for sx in 0..width {
            let px = self::bgra(pixels[sy * width + sx]);
            row[sx * 4..sx * 4 + 4].copy_from_slice(&px);
        }

        let o = ((y0 + sy) * atlas_width + x0) * 4;
        data[o..o + row.len()].copy_from_slice(row);
    }
}

#[derive(Clone, Copy)]
pub struct GridOverlay {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone)]
pub struct GridWindowOverlay {
    pub bands: Vec<GridOverlay>,
    pub effective: Vec<GridOverlay>,
    pub label_anchor: Option<GridOverlay>,
    pub label: SharedString,
    pub slot: usize,
}

impl From<debugger_model::TilemapViewport> for GridOverlay {
    fn from(viewport: debugger_model::TilemapViewport) -> Self {
        Self {
            x: viewport.x,
            y: viewport.y,
            width: viewport.width,
            height: viewport.height,
        }
    }
}

impl GridOverlay {
    fn spans(start: u32, len: u32, max: u32) -> impl Iterator<Item = (u32, u32)> {
        let start = start % max;
        let len = len.min(max);
        let first = (start, len.min(max - start));
        let wrapped = (len > max - start).then(|| (0, len - (max - start)));
        std::iter::once(first).chain(wrapped)
    }

    fn rects(self, map_w: u32, map_h: u32, scale: f32) -> Vec<Bounds<Pixels>> {
        let mut rects = Vec::with_capacity(4);
        for (y, h) in Self::spans(self.y, self.height, map_h) {
            for (x, w) in Self::spans(self.x, self.width, map_w) {
                rects.push(Bounds {
                    origin: point(px(x as f32 * scale), px(y as f32 * scale)),
                    size: size(px(w as f32 * scale), px(h as f32 * scale)),
                });
            }
        }
        rects
    }
}

pub struct GridContent {
    pub image: Option<Arc<RenderImage>>,
    pub cols: usize,
    pub rows: usize,
    pub tile_px: u32,
    pub count: usize,
    pub empty_message: &'static str,
    pub overlay: Option<GridOverlay>,
    pub footprint: Option<debugger_model::TilemapViewport>,
    pub window_overlays: Vec<GridWindowOverlay>,
}

#[derive(IntoElement)]
pub struct GridPanel {
    id: &'static str,
    title: SharedString,
    glyph: &'static str,
    meta: Option<SharedString>,
    content: Option<GridContent>,
    detail: Option<DetailFn>,
    detail_rows: usize,
    side_details_width: Option<f32>,
    placeholder: &'static str,
    layers: Vec<VideoResource>,
    layer: usize,
    selected: Option<usize>,
    hovered: Option<GridHover>,
    pinned: Option<GridHover>,
    inline_details: bool,
    hover_details: bool,
    viewport: Size<f32>,
    collapsed: bool,
    on_toggle: Option<ClickHandler>,
    on_select: Option<SelectCallback>,
    on_hover: Option<HoverCallback>,
    on_layer_select: Option<SelectCallback>,
    on_viewport: Option<ViewportCallback>,
    window_overlay_toggle: bool,
    show_window_overlays: bool,
    on_toggle_window_overlays: Option<ToggleCallback>,
    fill_width: bool,
    on_toggle_fill_width: Option<ToggleCallback>,
}

impl GridPanel {
    pub fn new(
        id: &'static str,
        title: &'static str,
        glyph: &'static str,
        content: Option<GridContent>,
    ) -> Self {
        Self {
            id,
            title: title.into(),
            glyph,
            meta: None,
            content,
            detail: None,
            detail_rows: 1,
            side_details_width: None,
            placeholder: "",
            layers: Vec::new(),
            layer: 0,
            selected: None,
            hovered: None,
            pinned: None,
            inline_details: true,
            hover_details: false,
            viewport: size(0., 0.),
            collapsed: false,
            on_toggle: None,
            on_select: None,
            on_hover: None,
            on_layer_select: None,
            on_viewport: None,
            window_overlay_toggle: false,
            show_window_overlays: true,
            on_toggle_window_overlays: None,
            fill_width: false,
            on_toggle_fill_width: None,
        }
    }

    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = title.into();
        self
    }

    pub fn meta(mut self, meta: impl Into<SharedString>) -> Self {
        self.meta = Some(meta.into());
        self
    }

    pub fn detail(mut self, detail: impl Fn(&Theme, usize) -> Div + 'static) -> Self {
        self.detail = Some(Box::new(detail));
        self
    }

    pub fn detail_rows(mut self, rows: usize) -> Self {
        self.detail_rows = rows;
        self
    }

    pub fn side_details(mut self, width: f32) -> Self {
        self.side_details_width = Some(width);
        self
    }

    pub fn placeholder(mut self, placeholder: &'static str) -> Self {
        self.placeholder = placeholder;
        self
    }

    pub fn layers(mut self, layers: Vec<VideoResource>) -> Self {
        self.layers = layers;
        self
    }

    pub fn layer(mut self, layer: usize) -> Self {
        self.layer = layer;
        self
    }

    pub fn selected(mut self, selected: Option<usize>) -> Self {
        self.selected = selected;
        self
    }

    pub fn hovered(mut self, hovered: Option<GridHover>) -> Self {
        self.hovered = hovered;
        self
    }

    /// Keeps the hover popup open at `pinned.position` while `pinned.index` stays selected.
    pub fn pinned(mut self, pinned: Option<GridHover>) -> Self {
        self.pinned = pinned;
        self
    }

    /// Shows the selected cell's details beside or below the grid.
    pub fn inline_details(mut self, enabled: bool) -> Self {
        self.inline_details = enabled;
        self
    }

    /// Shows the hovered cell's details in its popup.
    pub fn hover_details(mut self, enabled: bool) -> Self {
        self.hover_details = enabled;
        self
    }

    pub fn viewport(mut self, viewport: Size<f32>) -> Self {
        self.viewport = viewport;
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

    pub fn on_hover(
        mut self,
        handler: impl Fn(Option<GridHover>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_hover = Some(Rc::new(handler));
        self
    }

    pub fn on_layer_select(
        mut self,
        handler: impl Fn(usize, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_layer_select = Some(Rc::new(handler));
        self
    }

    pub fn on_viewport(
        mut self,
        handler: impl Fn(Size<f32>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_viewport = Some(Rc::new(handler));
        self
    }

    pub fn window_overlays_visible(mut self, visible: bool) -> Self {
        self.show_window_overlays = visible;
        self
    }

    /// Sizes cells to the full width and scrolls vertically instead of fitting the whole grid.
    pub fn fill_width(mut self, enabled: bool) -> Self {
        self.fill_width = enabled;
        self
    }

    pub fn on_toggle_fill_width(
        mut self,
        handler: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_toggle_fill_width = Some(Rc::new(handler));
        self
    }

    pub fn window_overlay_toggle(mut self, enabled: bool) -> Self {
        self.window_overlay_toggle = enabled;
        self
    }

    pub fn on_toggle_window_overlays(
        mut self,
        handler: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_toggle_window_overlays = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for GridPanel {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::global(cx);
        let t = &*theme;

        // Measure the whole body so moving the inspector cannot change the breakpoint.
        let side_details = self.inline_details
            && (self.side_details_width.is_some() || self.viewport.width >= DETAILS_BREAKPOINT);
        let detail_width = self.side_details_width.unwrap_or(DETAILS_WIDTH);
        let mut body = div()
            .relative()
            .flex()
            .when(!side_details, |body| body.flex_col())
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            .gap(px(DETAILS_GAP))
            .child(self::measure(self.on_viewport));

        let mut selectors = div().flex().items_center().gap_2();
        // A lone resource has nothing to switch to.
        if self.layers.len() > 1 {
            selectors = selectors.child(self::picker(
                format!("{}-layer", self.id),
                &self.layers,
                self.layer,
                self.on_layer_select,
            ));
        }

        let mut options = Vec::new();
        if self.window_overlay_toggle
            && let Some(handler) = self.on_toggle_window_overlays.clone()
        {
            options.push((
                "grid-window-overlays",
                self.show_window_overlays,
                "Overlays",
                handler,
            ));
        }
        if let Some(handler) = self.on_toggle_fill_width.clone()
            && self.content.is_some()
        {
            options.push(("grid-fill-width", self.fill_width, "Fill width", handler));
        }

        match &self.content {
            None => {
                body = body.child(
                    div()
                        .flex()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .child(empty_state(t, "Layer disabled by the current mode")),
                );
            }
            Some(content) => {
                let detail_height = FOOTER_ROW_H * (self.detail_rows as f32);
                let detail_height = if self.viewport.height > 0. {
                    detail_height.min(self.viewport.height * 0.45)
                } else {
                    detail_height
                };
                let grid_viewport = if !self.inline_details {
                    self.viewport
                } else if side_details {
                    size(
                        (self.viewport.width - detail_width - DETAILS_GAP).max(0.),
                        self.viewport.height,
                    )
                } else {
                    size(
                        self.viewport.width,
                        (self.viewport.height - detail_height - DETAILS_GAP).max(0.),
                    )
                };
                body = body.child(image_grid::cell_grid(
                    t,
                    self.id,
                    content,
                    image_grid::Interaction {
                        selected: self.selected,
                        hovered: self.hovered,
                        pinned: self.pinned,
                        detail: self.detail.as_ref().filter(|_| self.hover_details),
                    },
                    grid_viewport,
                    window.viewport_size(),
                    self.on_select,
                    self.on_hover,
                    self.show_window_overlays,
                    self.fill_width,
                ));

                if self.inline_details {
                    let selected = self.selected.filter(|&index| index < content.count);
                    let slot = div()
                        .id(format!("{}-details", self.id))
                        .flex_none()
                        .flex()
                        .flex_col()
                        .min_w(px(0.))
                        .min_h(px(0.))
                        .when(side_details, |slot| {
                            slot.w(px(detail_width))
                                .h_full()
                                .border_l_1()
                                .border_color(t.border_subtle)
                                .pl_2()
                        })
                        .when(!side_details, |slot| slot.h(px(detail_height)))
                        .overflow_y_scroll();
                    body = body.child(match (selected, &self.detail) {
                        (Some(index), Some(detail)) => slot.child(detail(t, index).flex_none()),
                        _ => slot.child(footer_placeholder(t, self.placeholder).flex_none()),
                    });
                }
            }
        }

        let mut panel = Panel::new(self.id, self.title)
            .glyph(self.glyph)
            .grow(true)
            .collapsed(self.collapsed)
            .on_toggle_opt(self.on_toggle)
            .header_action(selectors);
        if !options.is_empty() {
            panel = panel.header_options(move |in_menu, theme| {
                div()
                    .flex()
                    .gap_2()
                    .whitespace_nowrap()
                    .when(in_menu, |row| row.flex_col())
                    .when(!in_menu, |row| row.items_center())
                    .children(options.iter().map(|(id, checked, label, handler)| {
                        let click_handler = handler.clone();
                        let key_handler = handler.clone();
                        checkbox(
                            theme,
                            id,
                            *checked,
                            *label,
                            CheckboxSize::Small,
                            move |_, window, cx| {
                                cx.stop_propagation();
                                click_handler(window, cx);
                            },
                        )
                        .focusable()
                        .tab_index(0)
                        .on_key_down(move |event, window, cx| {
                            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                cx.stop_propagation();
                                key_handler(window, cx);
                            }
                        })
                    }))
                    .into_any_element()
            });
        }
        if let Some(meta) = self.meta {
            panel = panel.meta(meta);
        }

        panel.child(body)
    }
}

pub(super) fn picker(
    id: impl Into<SharedString>,
    layers: &[VideoResource],
    selected: usize,
    handler: Option<SelectCallback>,
) -> Div {
    let id = id.into();
    let group_of = |index: usize| {
        let name = &layers[index].name;
        layers
            .iter()
            .position(|layer| &layer.name == name)
            .unwrap_or(index)
    };
    let members = |group: usize| {
        (0..layers.len()).filter(move |&index| layers[index].name == layers[group].name)
    };
    let segmented = |id: SharedString, key: usize| {
        let mut segmented = Segmented::new(id).selected(key.to_string());
        if let Some(handler) = handler.clone() {
            let enabled_members: Vec<_> = layers
                .iter()
                .enumerate()
                .map(|(index, layer)| {
                    if layer.enabled {
                        Some(index)
                    } else {
                        layers
                            .iter()
                            .position(|candidate| candidate.name == layer.name && candidate.enabled)
                    }
                })
                .collect();
            segmented = segmented.on_select(move |key, window, cx| {
                if let Ok(index) = key.parse::<usize>()
                    && let Some(&Some(index)) = enabled_members.get(index)
                {
                    handler(index, window, cx);
                }
            });
        }
        segmented
    };

    let current = (selected < layers.len()).then(|| group_of(selected));
    let mut groups = segmented(id.clone(), current.unwrap_or(selected));
    for group in (0..layers.len()).filter(|&index| group_of(index) == index) {
        let label = layers[group].name.clone();
        groups = if members(group).any(|index| layers[index].enabled) {
            groups.option_label(group.to_string(), label)
        } else {
            groups.option_disabled(group.to_string(), label)
        };
    }

    let mut row = div().flex().items_center().gap_2();
    if let Some(group) = current.filter(|&group| members(group).count() > 1) {
        let mut variants = segmented(format!("{id}-variant").into(), selected);
        for index in members(group) {
            let layer = &layers[index];
            let label = layer.variant.as_ref().unwrap_or(&layer.name).clone();
            variants = if layer.enabled {
                variants.option_label(index.to_string(), label)
            } else {
                variants.option_disabled(index.to_string(), label)
            };
        }
        row = row.child(variants);
    }

    row.child(groups)
}

fn measure(on_viewport: Option<ViewportCallback>) -> impl IntoElement {
    canvas(
        move |bounds, window, cx| {
            if let Some(cb) = &on_viewport {
                cb(
                    size(f32::from(bounds.size.width), f32::from(bounds.size.height)),
                    window,
                    cx,
                );
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_full()
}
