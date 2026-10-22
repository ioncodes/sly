use std::collections::BTreeMap;

use super::*;

/// Side length the hover preview aims for.
const PREVIEW_PX: f32 = 192.;
const PREVIEW_PADDING: f32 = 4.;
const PREVIEW_BORDER: f32 = 1.;
const PREVIEW_GAP: f32 = 12.;
/// Outer size of the preview box, including its padding and border.
const PREVIEW_EXTENT: f32 = PREVIEW_PX + (2. * (PREVIEW_PADDING + PREVIEW_BORDER));
/// Distance the preview keeps from window edges.
const PREVIEW_MARGIN: f32 = 6.;
/// Pixels shown around the cursor when every cell is a single pixel (bitmaps).
const LOUPE_SPAN: u32 = 16;
/// Horizontal padding around the grid (`px_1` on both sides).
const GRID_PADDING_X: f32 = 8.;
/// Width of the details listed under the preview.
const POPUP_DETAILS_WIDTH: f32 = 224.;

/// Pointer state and popup content for a grid.
pub(super) struct Interaction<'a> {
    pub selected: Option<usize>,
    pub hovered: Option<GridHover>,
    pub pinned: Option<GridHover>,
    /// Details shown in the popup, when enabled.
    pub detail: Option<&'a DetailFn>,
}

fn cell_size(viewport: Size<f32>, cols: usize, rows: usize, tile_px: u32, fill_width: bool) -> f32 {
    if viewport.width < 1. || viewport.height < 1. {
        return (tile_px * 2) as f32;
    }

    let fit = if fill_width {
        (viewport.width - GRID_PADDING_X) / cols as f32
    } else {
        (viewport.width / cols as f32).min(viewport.height / rows as f32)
    };
    if tile_px == 1 {
        fit.max(0.1)
    } else {
        fit.floor().max(4.)
    }
}

#[expect(clippy::too_many_arguments)]
pub(super) fn cell_grid(
    theme: &Theme,
    id: &'static str,
    content: &GridContent,
    interaction: Interaction,
    viewport: Size<f32>,
    window_size: Size<Pixels>,
    on_select: Option<SelectCallback>,
    on_hover: Option<HoverCallback>,
    show_window_overlays: bool,
    fill_width: bool,
) -> Div {
    let container = div()
        .relative()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .px_1()
        .items_center()
        .when(!fill_width, |container| container.justify_center())
        .overflow_hidden();

    let Some(image) = content.image.clone() else {
        return container.child(empty_state(theme, content.empty_message));
    };

    let cell = self::cell_size(
        viewport,
        content.cols,
        content.rows,
        content.tile_px,
        fill_width,
    );
    let grid = render(
        theme,
        GridSpec {
            id: format!("{id}-grid").into(),
            image,
            cols: content.cols,
            rows: content.rows,
            tile_px: content.tile_px,
            count: content.count,
            cell,
            window_size,
            overlay: content.overlay,
            footprint: content.footprint,
            window_overlays: if show_window_overlays {
                content.window_overlays.clone()
            } else {
                Vec::new()
            },
        },
        interaction,
        on_select,
        on_hover,
    );

    if fill_width {
        container.child(
            div()
                .id(SharedString::from(format!("{id}-scroll")))
                .flex()
                .flex_col()
                .items_center()
                .flex_1()
                .w_full()
                .min_h(px(0.))
                .overflow_y_scroll()
                .child(grid),
        )
    } else {
        container.child(grid)
    }
}

struct GridSpec {
    id: SharedString,
    image: Arc<RenderImage>,
    cols: usize,
    rows: usize,
    tile_px: u32,
    count: usize,
    cell: f32,
    window_size: Size<Pixels>,
    overlay: Option<GridOverlay>,
    footprint: Option<debugger_model::TilemapViewport>,
    window_overlays: Vec<GridWindowOverlay>,
}

fn render(
    theme: &Theme,
    spec: GridSpec,
    interaction: Interaction,
    on_select: Option<SelectCallback>,
    on_hover: Option<HoverCallback>,
) -> impl IntoElement {
    let GridSpec {
        id,
        image,
        cols,
        rows,
        tile_px,
        count,
        cell,
        window_size,
        overlay,
        footprint,
        window_overlays,
    } = spec;
    let Interaction {
        selected,
        hovered,
        pinned,
        detail,
    } = interaction;
    // A pin only lasts as long as the selection it was made with.
    let pinned = pinned.filter(|pin| pin.index < count && selected == Some(pin.index));

    let bounds = Rc::new(Cell::new(Bounds::<Pixels>::default()));
    let hit = {
        let bounds = bounds.clone();
        move |position: Point<Pixels>| -> Option<usize> {
            let bounds = bounds.get();
            if !bounds.contains(&position) {
                return None;
            }

            let col = (f32::from(position.x - bounds.origin.x) / cell) as usize;
            let row = (f32::from(position.y - bounds.origin.y) / cell) as usize;
            let index = row.min(rows - 1) * cols + col.min(cols - 1);
            (index < count).then_some(index)
        }
    };

    let track_bounds = canvas(
        {
            let bounds = bounds.clone();
            move |grid_bounds, _, _| bounds.set(grid_bounds)
        },
        |_, _, _, _| {},
    )
    .absolute()
    .inset_0();

    let outline = |bounds: Bounds<Pixels>, color: Hsla| {
        div()
            .absolute()
            .left(bounds.origin.x)
            .top(bounds.origin.y)
            .w(bounds.size.width)
            .h(bounds.size.height)
            .border_1()
            .border_color(color)
    };

    let ring = |index: usize, color: Hsla| {
        outline(
            Bounds {
                origin: point(
                    px((index % cols) as f32 * cell),
                    px((index / cols) as f32 * cell),
                ),
                size: size(px(cell), px(cell)),
            },
            color,
        )
    };

    let mut grid = div()
        .id(id)
        .relative()
        .flex_none()
        .w(px(cell * cols as f32))
        .h(px(cell * rows as f32))
        .child(
            img(image.clone())
                .size_full()
                .image_filtering(ImageFiltering::Nearest),
        )
        .child(track_bounds);

    if let Some(overlay) = overlay {
        let (map_w, map_h) = (cols as u32 * tile_px, rows as u32 * tile_px);
        for bounds in overlay.rects(map_w, map_h, cell / tile_px as f32) {
            grid = grid.child(
                outline(bounds, with_alpha(theme.accent, 0.7)).bg(with_alpha(theme.accent, 0.12)),
            );
        }
    }

    if let Some(viewport) = footprint {
        let color = theme.accent;
        let scale = cell / tile_px as f32;
        grid = grid.child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let [a, b, tx, c, d, ty] =
                        viewport.transform.unwrap_or([1., 0., 0., 0., 1., 0.]);
                    let (x, y, w, h) = (
                        viewport.x as f32,
                        viewport.y as f32,
                        viewport.width as f32,
                        viewport.height as f32,
                    );

                    let mut path = gpui::PathBuilder::stroke(px(1.));
                    for (i, (x, y)) in [(x, y), (x + w, y), (x + w, y + h), (x, y + h), (x, y)]
                        .into_iter()
                        .enumerate()
                    {
                        let p = point(
                            bounds.origin.x + px((a * x + b * y + tx) * scale),
                            bounds.origin.y + px((c * x + d * y + ty) * scale),
                        );

                        if i == 0 {
                            path.move_to(p);
                        } else {
                            path.line_to(p);
                        }
                    }

                    if let Ok(path) = path.build() {
                        window.paint_path(path, color);
                    }
                },
            )
            .absolute()
            .size_full(),
        );
    }

    let (map_w, map_h) = (cols as u32 * tile_px, rows as u32 * tile_px);
    let scale = cell / tile_px as f32;
    let window_overlays: Arc<[GridWindowOverlay]> = window_overlays.into();
    if !window_overlays.is_empty() {
        let painted = window_overlays.clone();
        let colors = [theme.warning, theme.info, theme.success, theme.accent];
        grid = grid.child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    self::paint_window_overlays(
                        window, bounds, &painted, map_w, map_h, scale, colors,
                    );
                },
            )
            .absolute()
            .inset_0(),
        );

        for overlay in window_overlays.iter() {
            let Some(anchor) = overlay.label_anchor else {
                continue;
            };
            let Some(bounds) = anchor
                .rects(map_w, map_h, scale)
                .into_iter()
                .max_by_key(|bounds| u32::from(bounds.size.width))
            else {
                continue;
            };
            let color = slot_color(&colors, overlay.slot);
            let label_width = px((f32::from(bounds.size.width) - 4.).max(0.));
            grid = grid.child(
                div()
                    .absolute()
                    .left(bounds.origin.x + px(2.))
                    .top(bounds.origin.y + px(2.))
                    .flex()
                    .items_start()
                    .w(label_width)
                    .h(px(15.))
                    .overflow_hidden()
                    .child(
                        div()
                            .flex_none()
                            .px_1()
                            .rounded_sm()
                            .bg(with_alpha(theme.bg_root, 0.82))
                            .text_xs()
                            .whitespace_nowrap()
                            .text_color(color)
                            .child(overlay.label.clone()),
                    ),
            );
        }
    }

    if let Some(hover) = hovered.filter(|h| h.index < count && selected != Some(h.index)) {
        grid = grid.child(ring(hover.index, theme.accent_hover));
    }
    let popup = |index: usize| {
        self::popup(
            theme,
            self::preview(theme, &image, index, cols, rows, tile_px, cell),
            detail.map(|detail| detail(theme, index).w(px(POPUP_DETAILS_WIDTH))),
        )
    };
    let place = |popup: Div, cursor: Point<Pixels>, priority: usize| {
        let (anchor, position) = preview_position(cursor, window_size);
        deferred(
            anchored()
                .position_mode(AnchoredPositionMode::Window)
                .anchor(anchor)
                .position(position)
                .snap_to_window_with_margin(px(PREVIEW_MARGIN))
                .child(popup),
        )
        .priority(priority)
    };
    if let Some(hover) = hovered.filter(|h| h.index < count)
        && pinned.is_none_or(|pin| pin.index != hover.index)
        && let Some(popup) = popup(hover.index)
    {
        grid = grid.child(place(popup, hover.position, 2));
    }
    if let Some(pin) = pinned
        && let Some(popup) = popup(pin.index)
    {
        // Occlude so the pointer can rest on the pinned popup without hovering cells beneath it.
        grid = grid.child(place(popup.occlude(), pin.position, 3));
    }
    if let Some(index) = selected.filter(|&i| i < count) {
        grid = grid.child(ring(index, theme.accent));
    }

    if let Some(handler) = on_select {
        let hit = hit.clone();
        grid = grid
            .cursor_pointer()
            .on_click(move |event: &ClickEvent, window, cx| {
                cx.stop_propagation();
                if let Some(index) = hit(event.position()) {
                    handler(index, window, cx);
                }
            });
    }

    if let Some(handler) = on_hover {
        let moved = handler.clone();
        grid = grid
            .on_mouse_move(move |event: &MouseMoveEvent, window, cx| {
                let hover = hit(event.position).map(|index| GridHover {
                    index,
                    position: event.position,
                });
                moved(hover, window, cx);
            })
            .on_hover(move |hovered, window, cx| {
                if !hovered {
                    handler(None, window, cx);
                }
            });
    }

    grid
}

/// Keep the preview beside the cursor, flipping away from nearby window edges.
fn preview_position(cursor: Point<Pixels>, window: Size<Pixels>) -> (Anchor, Point<Pixels>) {
    let extent = px(PREVIEW_EXTENT);
    let gap = px(PREVIEW_GAP);
    let margin = px(PREVIEW_MARGIN);
    let left = (cursor.x + gap + extent) > (window.width - margin);
    let above = (cursor.y + gap + extent) > (window.height - margin);
    let anchor = match (left, above) {
        (false, false) => Anchor::TopLeft,
        (true, false) => Anchor::TopRight,
        (false, true) => Anchor::BottomLeft,
        (true, true) => Anchor::BottomRight,
    };
    let position = point(
        cursor.x + if left { -gap } else { gap },
        cursor.y + if above { -gap } else { gap },
    );
    (anchor, position)
}

fn popup(theme: &Theme, preview: Option<Div>, details: Option<Div>) -> Option<Div> {
    if preview.is_none() && details.is_none() {
        return None;
    }

    Some(
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap_1()
            .p(px(PREVIEW_PADDING))
            .bg(theme.bg_panel)
            .border_1()
            .border_color(theme.border_strong)
            .rounded_md()
            .shadow_lg()
            .children(preview)
            .children(details),
    )
}

/// Enlarged copy of the hovered cell, or a loupe around the hovered pixel for bitmaps.
fn preview(
    theme: &Theme,
    image: &Arc<RenderImage>,
    index: usize,
    cols: usize,
    rows: usize,
    tile_px: u32,
    cell: f32,
) -> Option<Div> {
    let (map_w, map_h) = (cols as u32 * tile_px, rows as u32 * tile_px);
    let (col, row) = ((index % cols) as u32, (index / cols) as u32);
    let (x, y, span) = if tile_px == 1 {
        let span = LOUPE_SPAN.min(map_w).min(map_h);
        let clamp = |center: u32, max: u32| center.saturating_sub(span / 2).min(max - span);
        (clamp(col, map_w), clamp(row, map_h), span)
    } else {
        (col * tile_px, row * tile_px, tile_px)
    };

    let zoom = (PREVIEW_PX / span as f32).floor().max(1.);
    let side = span as f32 * zoom;
    if side <= cell {
        return None;
    }

    let mut view = div().relative().size(px(side)).overflow_hidden().child(
        img(image.clone())
            .absolute()
            .left(px(-(x as f32 * zoom)))
            .top(px(-(y as f32 * zoom)))
            .w(px(map_w as f32 * zoom))
            .h(px(map_h as f32 * zoom))
            .image_filtering(ImageFiltering::Nearest),
    );
    if tile_px == 1 {
        view = view.child(
            div()
                .absolute()
                .left(px((col - x) as f32 * zoom))
                .top(px((row - y) as f32 * zoom))
                .size(px(zoom))
                .border_1()
                .border_color(theme.accent),
        );
    }

    Some(view)
}

fn slot_color(colors: &[Hsla], slot: usize) -> Hsla {
    colors[slot.min(colors.len() - 1)]
}

#[expect(clippy::too_many_arguments)]
fn paint_window_overlays(
    window: &mut Window,
    canvas_bounds: Bounds<Pixels>,
    overlays: &[GridWindowOverlay],
    map_w: u32,
    map_h: u32,
    scale: f32,
    colors: [Hsla; 4],
) {
    let offset = |bounds: Bounds<Pixels>| Bounds {
        origin: point(
            canvas_bounds.origin.x + bounds.origin.x,
            canvas_bounds.origin.y + bounds.origin.y,
        ),
        size: bounds.size,
    };

    for overlay in overlays {
        let mut shade = slot_color(&colors, overlay.slot);
        shade.lightness = 0.04;
        shade.alpha = 0.80;

        for rect in &overlay.effective {
            for bounds in rect.rects(map_w, map_h, scale) {
                window.paint_quad(fill(offset(bounds), shade));
            }
        }
    }

    for overlay in overlays.iter().rev() {
        let color = slot_color(&colors, overlay.slot);
        let mut edges = BTreeMap::<(bool, u32), Vec<(u32, i32)>>::new();
        for band in &overlay.bands {
            if band.width == 0 || band.height == 0 {
                continue;
            }

            for (y, height) in GridOverlay::spans(band.y, band.height, map_h) {
                for (x, width) in GridOverlay::spans(band.x, band.width, map_w) {
                    for (horizontal, position, start, end, sign) in [
                        (true, y, x, x + width, 1),
                        (true, y + height, x, x + width, -1),
                        (false, x, y, y + height, 1),
                        (false, x + width, y, y + height, -1),
                    ] {
                        edges
                            .entry((horizontal, position))
                            .or_default()
                            .extend([(start, sign), (end, -sign)]);
                    }
                }
            }
        }
        for ((horizontal, position), mut events) in edges {
            events.sort_unstable_by_key(|&(position, _)| position);

            let mut count = 0;
            let mut previous = 0;
            for (end, delta) in events {
                if count != 0 && end > previous {
                    let along = (previous as f32 * scale).round();
                    let length = (end as f32 * scale).round() - along;
                    let limit = if horizontal { map_h } else { map_w };
                    let across = (position as f32 * scale)
                        .round()
                        .min((limit as f32 * scale - 1.).max(0.));
                    let (x, y, width, height) = if horizontal {
                        (along, across, length, 1.)
                    } else {
                        (across, along, 1., length)
                    };
                    window.paint_quad(fill(
                        offset(Bounds {
                            origin: point(px(x), px(y)),
                            size: size(px(width), px(height)),
                        }),
                        with_alpha(color, 0.9),
                    ));
                }

                count += delta;
                previous = end;
            }
        }
    }
}
