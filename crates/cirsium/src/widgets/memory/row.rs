use gpui::prelude::*;
use gpui::{
    BorderStyle, Bounds, CursorStyle, Div, Edges, FontWeight, HitboxBehavior, Hsla, MouseButton,
    MouseDownEvent, MouseMoveEvent, Pixels, Point, SharedString, TextAlign, TextRun, canvas, div,
    point, px, quad, size,
};

use crate::diff::{Highlight, highlight_fill};
use crate::theme::Theme;
use crate::widgets::{BreakpointKind, breakpoint_colors};

use super::{
    ByteDownCallback, ByteDragCallback, ContextCallback, MemoryCell, MemoryRow, ascii_glyph,
};

const ROW_HEIGHT: f32 = 20.;
const ADDR_WIDTH: f32 = 76.;
const HEX_CELL_WIDTH: f32 = 21.;
const ASCII_CELL_WIDTH: f32 = 9.;
const GROUP_GAP: f32 = 8.;
const HEX_ASCII_GAP: f32 = 14.;

#[allow(clippy::too_many_arguments)]
pub(super) fn mem_row(
    theme: &Theme,
    row: Option<&MemoryRow>,
    above: Option<&MemoryRow>,
    below: Option<&MemoryRow>,
    bytes_per_row: usize,
    selection: Option<(u32, u32)>,
    breakpoints: &[(u32, u32, BreakpointKind)],
    on_byte_down: Option<ByteDownCallback>,
    on_byte_drag: Option<ByteDragCallback>,
    on_context: Option<ContextCallback>,
) -> Div {
    let Some(row) = row else {
        return div().h(px(ROW_HEIGHT));
    };

    let in_selection = |addr: u32| selection.is_some_and(|(lo, hi)| addr >= lo && addr <= hi);
    let breakpoint_at = |addr: u32| {
        breakpoints
            .iter()
            .find(|(lo, hi, _)| addr >= *lo && addr <= *hi)
            .map(|&(_, _, kind)| kind)
    };

    let address = div()
        .font_family(theme.mono_font)
        .text_sm()
        .w(px(ADDR_WIDTH))
        .flex_none()
        .text_color(theme.code_address)
        .child(row.label.clone());

    let half = bytes_per_row / 2;
    let last = row.cells.len().saturating_sub(1);
    let fills: Vec<Fill> = row
        .cells
        .iter()
        .enumerate()
        .map(|(i, cell)| self::cell_fill(cell, in_selection(row.addr + i as u32)))
        .collect();

    let mut hex = Vec::with_capacity(row.cells.len());
    let mut ascii = Vec::with_capacity(row.cells.len());

    let stride = bytes_per_row as u32;
    for (i, cell) in row.cells.iter().enumerate() {
        let addr = row.addr + i as u32;
        let colors = self::fill_colors(theme, fills[i]);
        let dashed = matches!(fills[i], Fill::Breakpoint(_));
        let text = match colors {
            Some((_, border)) if dashed => border,
            _ => theme.text_primary,
        };

        let hex_left = i == 0 || i == half || fills[i - 1] != fills[i];
        let hex_right = i == last || i + 1 == half || fills[i + 1] != fills[i];
        let ascii_left = i == 0 || fills[i - 1] != fills[i];
        let ascii_right = i == last || fills[i + 1] != fills[i];

        let (top, bottom) = match fills[i] {
            Fill::Selection => (
                !in_selection(addr.wrapping_sub(stride)),
                !in_selection(addr + stride),
            ),
            Fill::Breakpoint(kind) => (
                in_selection(addr.wrapping_sub(stride))
                    || breakpoint_at(addr.wrapping_sub(stride)) != Some(kind),
                in_selection(addr.wrapping_add(stride))
                    || breakpoint_at(addr.wrapping_add(stride)) != Some(kind),
            ),
            Fill::Highlight(_) => {
                let same = |r: Option<&MemoryRow>| {
                    r.and_then(|r| {
                        r.cells
                            .get(i)
                            .map(|c| self::cell_fill(c, in_selection(r.addr + i as u32)))
                    }) == Some(fills[i])
                };
                (!same(above), !same(below))
            }
            Fill::None => (true, true),
        };

        let style = if dashed {
            BorderStyle::Dashed
        } else {
            BorderStyle::Solid
        };

        let fill = |left: bool, right: bool| {
            colors.map(|(background, border)| CellFill {
                background,
                border,
                style,
                edges: Edges {
                    left: self::edge(left),
                    right: self::edge(right),
                    top: self::edge(top),
                    bottom: self::edge(bottom),
                },
            })
        };

        hex.push(ByteCell {
            text: format!("{:02X}", cell.value).into(),
            color: text,
            fill: fill(hex_left, hex_right),
        });

        ascii.push(ByteCell {
            text: ascii_glyph(cell.value).to_string().into(),
            color: text,
            fill: fill(ascii_left, ascii_right),
        });
    }

    let callbacks = StripCallbacks {
        on_byte_down,
        on_byte_drag,
        on_context,
    };

    div()
        .flex()
        .items_center()
        .gap_2()
        .h(px(ROW_HEIGHT))
        .px_1()
        .child(address)
        .child(self::byte_strip(
            theme,
            row.addr,
            hex,
            HEX_CELL_WIDTH,
            Some(half),
            callbacks.clone(),
        ))
        .child(div().w(px(HEX_ASCII_GAP)).flex_none())
        .child(self::byte_strip(
            theme,
            row.addr,
            ascii,
            ASCII_CELL_WIDTH,
            None,
            callbacks,
        ))
}

pub(super) fn column_header(theme: &Theme, bytes_per_row: usize) -> Div {
    let mut hex = div().flex().items_center().flex_none();
    let mut ascii = div().flex().items_center().flex_none();
    for col in 0..bytes_per_row {
        if col == bytes_per_row / 2 {
            hex = hex.child(div().w(px(GROUP_GAP)).flex_none());
        }

        hex = hex.child(
            self::cell_box(HEX_CELL_WIDTH)
                .font_family(theme.mono_font)
                .text_xs()
                .font_weight(FontWeight::BOLD)
                .text_color(theme.text_muted)
                .child(format!("{col:02X}")),
        );
        ascii = ascii.child(
            self::cell_box(ASCII_CELL_WIDTH)
                .font_family(theme.mono_font)
                .text_xs()
                .font_weight(FontWeight::BOLD)
                .text_color(theme.text_dim)
                .child(format!("{col:X}")),
        );
    }

    div()
        .flex()
        .items_center()
        .gap_2()
        .h(px(ROW_HEIGHT))
        .px_1()
        .border_b_1()
        .border_color(theme.border_subtle)
        .child(
            div()
                .w(px(ADDR_WIDTH))
                .flex_none()
                .text_xs()
                .font_weight(FontWeight::BOLD)
                .text_color(theme.text_muted)
                .child("ADDRESS"),
        )
        .child(hex)
        .child(div().w(px(HEX_ASCII_GAP)).flex_none())
        .child(ascii)
}

fn cell_box(width: f32) -> Div {
    div()
        .w(px(width))
        .h(px(ROW_HEIGHT))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Fill {
    None,
    Highlight(Highlight),
    Selection,
    Breakpoint(BreakpointKind),
}

fn cell_fill(cell: &MemoryCell, selected: bool) -> Fill {
    if selected {
        return Fill::Selection;
    }

    if let Some(kind) = cell.breakpoint {
        return Fill::Breakpoint(kind);
    }

    match cell.highlight {
        Highlight::None => Fill::None,
        Highlight::Changed => Fill::Highlight(Highlight::Written),
        highlight => Fill::Highlight(highlight),
    }
}

fn fill_colors(theme: &Theme, fill: Fill) -> Option<(Hsla, Hsla)> {
    match fill {
        Fill::None => None,
        Fill::Highlight(highlight) => highlight_fill(theme, highlight),
        Fill::Selection => Some((theme.selection, theme.selection_border)),
        Fill::Breakpoint(kind) => Some(breakpoint_colors(theme, kind)),
    }
}

struct ByteCell {
    text: SharedString,
    color: Hsla,
    fill: Option<CellFill>,
}

#[derive(Clone, Copy)]
struct CellFill {
    background: Hsla,
    border: Hsla,
    style: BorderStyle,
    edges: Edges<Pixels>,
}

#[derive(Clone)]
struct StripCallbacks {
    on_byte_down: Option<ByteDownCallback>,
    on_byte_drag: Option<ByteDragCallback>,
    on_context: Option<ContextCallback>,
}

#[derive(Clone, Copy)]
struct StripLayout {
    cell_width: f32,
    group_break: Option<usize>,
    len: usize,
}

impl StripLayout {
    fn cell_x(self, index: usize) -> f32 {
        let gap = if self.group_break.is_some_and(|half| index >= half) {
            GROUP_GAP
        } else {
            0.
        };

        (index as f32 * self.cell_width) + gap
    }

    fn cell_at(self, x: f32) -> Option<usize> {
        (0..self.len).find(|&index| {
            let left = self.cell_x(index);
            x >= left && x < left + self.cell_width
        })
    }
}

fn edge(visible: bool) -> Pixels {
    px(if visible { 1. } else { 0. })
}

fn byte_strip(
    theme: &Theme,
    base: u32,
    cells: Vec<ByteCell>,
    cell_width: f32,
    group_break: Option<usize>,
    callbacks: StripCallbacks,
) -> Div {
    let layout = StripLayout {
        cell_width,
        group_break,
        len: cells.len(),
    };

    div()
        .flex()
        .flex_none()
        .w(px(layout.cell_x(layout.len)))
        .h(px(ROW_HEIGHT))
        .font_family(theme.mono_font)
        .text_sm()
        .child(
            canvas(
                move |bounds, window, _| {
                    let style = window.text_style();
                    let font = style.font();
                    let font_size = style.font_size.to_pixels(window.rem_size());
                    let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);

                    let cells = cells
                        .into_iter()
                        .enumerate()
                        .map(|(index, cell)| {
                            let origin = bounds.origin + point(px(layout.cell_x(index)), px(0.));
                            let bounds = Bounds::new(origin, size(px(cell_width), px(ROW_HEIGHT)));

                            let run = TextRun {
                                len: cell.text.len(),
                                font: font.clone(),
                                color: cell.color,
                                background_color: None,
                                underline: None,
                                strikethrough: None,
                                letter_spacing: None,
                            };
                            let line =
                                window
                                    .text_system()
                                    .shape_line(cell.text, font_size, &[run]);

                            (cell.fill, bounds, line)
                        })
                        .collect::<Vec<_>>();

                    (hitbox, window.line_height(), cells)
                },
                move |strip, (hitbox, line_height, cells), window, cx| {
                    for (fill, bounds, line) in cells {
                        let mut edges = Edges::default();
                        if let Some(fill) = fill {
                            window.paint_quad(quad(
                                bounds,
                                px(0.),
                                fill.background,
                                fill.edges,
                                fill.border,
                                fill.style,
                            ));
                            edges = fill.edges;
                        }

                        let origin = bounds.origin
                            + point(
                                (bounds.size.width - line.width + edges.left - edges.right) / 2.,
                                (bounds.size.height - line_height + edges.top - edges.bottom) / 2.,
                            );
                        let _ = line.paint(origin, line_height, TextAlign::Left, None, window, cx);
                    }

                    window.set_cursor_style(CursorStyle::PointingHand, &hitbox);

                    let addr_at = move |position: Point<Pixels>| {
                        layout
                            .cell_at(f32::from(position.x - strip.origin.x))
                            .map(|index| base + index as u32)
                    };

                    if let Some(down) = callbacks.on_byte_down {
                        let hitbox = hitbox.clone();
                        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                            if phase.bubble()
                                && event.button == MouseButton::Left
                                && hitbox.is_hovered(window)
                                && let Some(addr) = addr_at(event.position)
                            {
                                down(addr, event.modifiers.shift, window, cx);
                            }
                        });
                    }

                    if let Some(context) = callbacks.on_context {
                        let hitbox = hitbox.clone();
                        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                            if phase.bubble()
                                && event.button == MouseButton::Right
                                && hitbox.is_hovered(window)
                                && let Some(addr) = addr_at(event.position)
                            {
                                context(addr, event.position, window, cx);
                            }
                        });
                    }

                    if let Some(drag) = callbacks.on_byte_drag {
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                            if phase.bubble()
                                && event.dragging()
                                && hitbox.is_hovered(window)
                                && let Some(addr) = addr_at(event.position)
                            {
                                drag(addr, window, cx);
                            }
                        });
                    }
                },
            )
            .size_full(),
        )
}
