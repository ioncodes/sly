use std::collections::HashMap;
use std::ops::Range;

use gpui::prelude::*;
use gpui::{
    App, Bounds, Element, ElementId, ElementInputHandler, Entity, GlobalElementId, Hsla,
    InspectorElementId, LayoutId, PaintQuad, Pixels, Point, ShapedLine, Style, TextRun, Window,
    fill, point, px, relative, size,
};

use scripting::LuaTokenKind;

use super::CodeEditor;
use crate::components::line_geometry::LineGeometry;
use crate::theme::{Theme, with_alpha};

const GUTTER_PAD: f32 = 10.;
const TEXT_PAD: f32 = 6.;
const FALLBACK_ROWS: usize = 120;

#[derive(Default)]
pub(super) struct Layout {
    pub bounds: Option<Bounds<Pixels>>,
    pub lines: HashMap<usize, ShapedLine>,
    pub line_height: Pixels,
    pub gutter: Pixels,
    pub caret: Point<Pixels>,
}

pub(super) struct EditorElement {
    pub(super) editor: Entity<CodeEditor>,
}

pub(super) struct PrepaintState {
    caret: Point<Pixels>,
    lines: Vec<(usize, ShapedLine)>,
    numbers: Vec<(usize, ShapedLine)>,
    quads: Vec<PaintQuad>,
    cursor: Option<PaintQuad>,
    gutter: Pixels,
    line_height: Pixels,
}

impl IntoElement for EditorElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for EditorElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let rows = self.editor.read(cx).line_count().max(1);
        let line_height = window.line_height();

        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = (line_height * rows as f32).into();

        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let theme = Theme::global(cx);
        let t = &*theme;
        let style = window.text_style();
        let font = style.font();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line_height = window.line_height();

        let editor = self.editor.read(cx);
        let rows = editor.line_count().max(1);
        let cursor_offset = editor.cursor_offset();
        let cursor_line = editor.line_at(cursor_offset);
        let selection = editor.selected_range.clone();
        let error_line = editor.error_line.and_then(|line| line.checked_sub(1));
        let focused = editor.focus_handle.is_focused(window);

        let viewport = editor.scroll.bounds();
        let visible = if viewport.size.height > px(0.) {
            let first = ((viewport.top() - bounds.top()) / line_height).floor();
            let last = ((viewport.bottom() - bounds.top()) / line_height).ceil();
            (first.max(0.) as usize).saturating_sub(1)..(last.max(0.) as usize + 1).min(rows)
        } else {
            0..rows.min(FALLBACK_ROWS)
        };

        let digits = rows.to_string().len().max(2);
        let gutter = px(GUTTER_PAD * 2. + f32::from(font_size) * 0.62 * digits as f32);

        let mut lines = Vec::with_capacity(visible.len());
        let mut numbers = Vec::with_capacity(visible.len());
        let mut quads = Vec::new();

        let text_x = bounds.left() + gutter;
        let row_top = |row: usize| bounds.top() + line_height * row as f32;

        let placeholder = editor.text.is_empty().then(|| editor.placeholder.clone());
        let mut token_cursor = editor
            .tokens
            .partition_point(|token| token.end <= editor.line_range(visible.start).start);

        for row in visible.clone() {
            let range = editor.line_range(row);
            let text = &editor.text[range.clone()];

            let shaped = match placeholder.as_ref().filter(|_| row == 0) {
                Some(hint) => window.text_system().shape_line(
                    hint.clone(),
                    font_size,
                    &[TextRun {
                        len: hint.len(),
                        font: font.clone(),
                        color: t.text_dim,
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                        letter_spacing: None,
                    }],
                ),
                None => window.text_system().shape_line(
                    text.to_string().into(),
                    font_size,
                    &self::runs(editor, &range, &mut token_cursor, &font, t, style.color),
                ),
            };

            let label = (row + 1).to_string();
            let number = window.text_system().shape_line(
                label.clone().into(),
                font_size,
                &[TextRun {
                    len: label.len(),
                    font: font.clone(),
                    color: if row == cursor_line {
                        t.text_secondary
                    } else {
                        t.text_dim
                    },
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                    letter_spacing: None,
                }],
            );

            let top = row_top(row);
            if Some(row) == error_line {
                quads.push(fill(
                    Bounds::new(
                        point(bounds.left(), top),
                        size(bounds.size.width, line_height),
                    ),
                    with_alpha(t.danger, 0.14),
                ));
            } else if row == cursor_line && focused && selection.is_empty() {
                quads.push(fill(
                    Bounds::new(
                        point(bounds.left(), top),
                        size(bounds.size.width, line_height),
                    ),
                    t.bg_panel,
                ));
            }

            if !selection.is_empty() && selection.start <= range.end && selection.end >= range.start
            {
                let start = selection.start.saturating_sub(range.start).min(range.len());
                let end = selection.end.saturating_sub(range.start).min(range.len());
                let trailing = selection.end > range.end;

                for mut selection_bounds in shaped
                    .platform_layout
                    .selection_bounds(start..end, line_height)
                {
                    selection_bounds.origin += point(text_x, top);
                    quads.push(fill(selection_bounds, t.selection));
                }

                if trailing {
                    quads.push(fill(
                        Bounds::new(
                            point(text_x + shaped.width, top),
                            size(font_size * 0.5, line_height),
                        ),
                        t.selection,
                    ));
                }
            }

            lines.push((row, shaped));
            numbers.push((row, number));
        }

        let caret_x = lines
            .iter()
            .find(|(ix, _)| *ix == cursor_line)
            .and_then(|(_, shaped)| {
                shaped.caret_x(cursor_offset - editor.line_starts[cursor_line], line_height)
            })
            .unwrap_or_default();
        let caret = point(gutter + caret_x, line_height * cursor_line as f32);

        let cursor = focused.then(|| {
            // Snap the left edge to a device pixel: gpui rounds quad edges independently, so an
            // unsnapped caret renders 1px or 2px wide depending on where its x lands.
            let scale = window.scale_factor();
            let left = px((f32::from(text_x + caret_x) * scale).round() / scale);
            fill(
                Bounds::new(point(left, row_top(cursor_line)), size(px(2.), line_height)),
                t.accent,
            )
        });

        if editor.reveal_cursor && viewport.size.height > px(0.) {
            let top = line_height * cursor_line as f32;
            let offset = editor.scroll.offset();
            let visible_top = -offset.y;
            let visible_bottom = visible_top + viewport.size.height - line_height;

            let target = if top < visible_top {
                Some(top)
            } else if top > visible_bottom {
                Some(top - viewport.size.height + line_height)
            } else {
                None
            };

            if let Some(target) = target {
                editor
                    .scroll
                    .set_offset(point(offset.x, -target.max(px(0.))));
            }
        }

        self.editor.update(cx, |editor, _| {
            editor.reveal_cursor = false;
        });

        PrepaintState {
            caret,
            lines,
            numbers,
            quads,
            cursor,
            gutter,
            line_height,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.editor.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.editor.clone()),
            cx,
        );

        for quad in prepaint.quads.drain(..) {
            window.paint_quad(quad);
        }

        let line_height = prepaint.line_height;
        let text_x = bounds.left() + prepaint.gutter;

        for (row, number) in prepaint.numbers.drain(..) {
            let right = bounds.left() + prepaint.gutter - px(TEXT_PAD);
            let origin = point(
                right - number.width,
                bounds.top() + line_height * row as f32,
            );
            number
                .paint(origin, line_height, gpui::TextAlign::Left, None, window, cx)
                .ok();
        }

        let mut painted = HashMap::with_capacity(prepaint.lines.len());
        for (row, line) in prepaint.lines.drain(..) {
            let origin = point(text_x, bounds.top() + line_height * row as f32);
            line.paint(origin, line_height, gpui::TextAlign::Left, None, window, cx)
                .ok();
            painted.insert(row, line);
        }

        if let Some(cursor) = prepaint.cursor.take() {
            window.paint_quad(cursor);
        }

        let gutter = prepaint.gutter;
        let caret = prepaint.caret;
        self.editor.update(cx, |editor, _| {
            editor.layout = Layout {
                bounds: Some(bounds),
                lines: painted,
                line_height,
                gutter,
                caret,
            };
        });
    }
}

fn runs(
    editor: &CodeEditor,
    line: &Range<usize>,
    token_cursor: &mut usize,
    font: &gpui::Font,
    theme: &Theme,
    plain: Hsla,
) -> Vec<TextRun> {
    let mut runs: Vec<TextRun> = Vec::new();
    let mut cursor = line.start;

    let push = |len: usize, color: Hsla, runs: &mut Vec<TextRun>| {
        if len == 0 {
            return;
        }
        runs.push(TextRun {
            len,
            font: font.clone(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
            letter_spacing: None,
        });
    };

    while *token_cursor < editor.tokens.len() && editor.tokens[*token_cursor].end <= line.start {
        *token_cursor += 1;
    }

    for token in &editor.tokens[*token_cursor..] {
        if token.start >= line.end {
            break;
        }

        let start = token.start.max(line.start);
        let end = token.end.min(line.end);
        push(start - cursor, plain, &mut runs);
        push(
            end - start,
            self::color(token.kind, theme, plain),
            &mut runs,
        );
        cursor = end;
    }

    push(line.end.saturating_sub(cursor), plain, &mut runs);
    runs
}

fn color(kind: LuaTokenKind, theme: &Theme, plain: Hsla) -> Hsla {
    match kind {
        LuaTokenKind::Keyword => theme.code_flow,
        LuaTokenKind::Builtin => theme.code_system,
        LuaTokenKind::Number => theme.code_number,
        LuaTokenKind::Str => theme.code_opcode,
        LuaTokenKind::Comment => theme.code_comment,
        LuaTokenKind::Operator => theme.code_alu,
        LuaTokenKind::Punctuation => theme.code_punctuation,
        LuaTokenKind::Ident | LuaTokenKind::Plain => plain,
    }
}
