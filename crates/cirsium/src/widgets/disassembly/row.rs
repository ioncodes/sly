use super::*;
use crate::diff::{Highlight, highlight_color, highlight_fill};
use gpui::{ColorExt, StyledText};

#[derive(Clone, Default)]
pub(super) struct RowStates {
    pub(super) current: Option<usize>,
    pub(super) pipeline: Option<usize>,
    pub(super) current_note: Option<SharedString>,
    pub(super) selected: Option<usize>,
    pub(super) breakpoints: Rc<HashMap<usize, BreakpointKind>>,
}

impl RowStates {
    pub(super) fn is_current(&self, ix: usize) -> bool {
        self.current == Some(ix)
    }

    pub(super) fn is_selected(&self, ix: usize) -> bool {
        self.selected == Some(ix)
    }

    pub(super) fn breakpoint(&self, ix: usize) -> Option<BreakpointKind> {
        self.breakpoints.get(&ix).copied()
    }

    pub(super) fn marker(&self, ix: usize) -> Option<Highlight> {
        if self.is_current(ix) {
            Some(Highlight::Execution)
        } else if self.pipeline == Some(ix) {
            Some(Highlight::Pipeline)
        } else {
            None
        }
    }

    pub(super) fn marker_color(&self, theme: &Theme, ix: usize) -> Option<Hsla> {
        self.marker(ix).map(|marker| highlight_color(theme, marker))
    }

    pub(super) fn background(&self, theme: &Theme, ix: usize) -> Option<Hsla> {
        if let Some(marker) = self.marker(ix) {
            return highlight_fill(theme, marker).map(|(fill, _)| fill);
        }

        if let Some(kind) = self.breakpoint(ix) {
            return Some(breakpoint_color(theme, kind).opacity(0.20));
        }

        self.is_selected(ix).then_some(theme.bg_panel_alt)
    }

    pub(super) fn note<'a>(&'a self, ix: usize, line: &'a DisasmLine) -> &'a str {
        match &self.current_note {
            Some(note) if self.is_current(ix) => note.as_ref(),
            _ => line.note.as_ref(),
        }
    }
}

pub(super) struct DisasmRowOptions {
    pub(super) arrows_width: f32,
    pub(super) show_bytes: bool,
    pub(super) mnemonic_width: f32,
    pub(super) rows: RowStates,
    pub(super) on_select: Option<SelectCallback>,
    pub(super) on_context: Option<ContextCallback>,
}

pub(super) fn disasm_row(
    theme: &Theme,
    ix: usize,
    line: &DisasmLine,
    arrows: &GutterCell,
    options: &DisasmRowOptions,
) -> Stateful<Div> {
    let breakpoint = options.rows.breakpoint(ix);
    let background = options.rows.background(theme, ix);
    let note = SharedString::from(options.rows.note(ix, line));

    let mut gutter = div()
        .w(px(13.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center();
    if let Some(kind) = breakpoint {
        gutter = gutter.child(
            div()
                .size(px(8.))
                .rounded_full()
                .bg(breakpoint_color(theme, kind)),
        );
    } else if let Some(color) = options.rows.marker_color(theme, ix) {
        gutter = gutter.child(
            svg()
                .path(icon::CHEVRON_RIGHT)
                .size(px(11.))
                .text_color(color),
        );
    }

    let mut row = div()
        .id(("disasm-row", ix))
        .flex()
        .items_center()
        .gap_2()
        .w_full()
        .h(px(ROW_HEIGHT))
        .px_1()
        .rounded_md()
        .when_some(background, |d, bg| d.bg(bg))
        .child(gutter);

    if options.arrows_width > 0. {
        let draws = arrows.clone();
        row = row.child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| paint_arrows(window, bounds, &draws),
            )
            .flex_none()
            .w(px(options.arrows_width))
            .h(px(ROW_HEIGHT)),
        );
    }

    row = row.child(
        div()
            .font_family(theme.mono_font)
            .text_sm()
            .min_w(px(58.))
            .flex_none()
            .text_color(theme.code_address)
            .child(SharedString::from(line.address.clone())),
    );

    if options.show_bytes {
        row = row.child(
            div()
                .font_family(theme.mono_font)
                .text_sm()
                .w(px(BYTES_WIDTH))
                .flex_none()
                .text_color(theme.text_dim)
                .child(SharedString::from(line.bytes.clone())),
        );
    }

    row = row.child(self::instruction_text(
        theme,
        &line.tokens,
        options.mnemonic_width,
    ));

    if !note.is_empty() {
        row = row.child(
            div()
                .ml_auto()
                .pl_4()
                .flex_none()
                .font_family(theme.mono_font)
                .text_sm()
                .text_color(theme.code_comment)
                .child(note),
        );
    }

    if background.is_none() {
        let hover_bg = theme.bg_header;
        row = row.hover(move |s| s.bg(hover_bg));
    }
    if let Some(select) = options.on_select.clone() {
        row = row
            .cursor_pointer()
            .on_click(move |_, window, cx| select(ix, window, cx));
    }
    if let Some(context) = options.on_context.clone() {
        row = row.on_mouse_down(MouseButton::Right, move |event, window, cx| {
            context(ix, event.position, window, cx)
        });
    }

    row
}

pub(super) fn instruction_text(theme: &Theme, tokens: &[DisasmToken], mnemonic_min: f32) -> Div {
    let mut text = div()
        .flex()
        .items_center()
        .font_family(theme.mono_font)
        .text_sm();
    if let Some(mnemonic) = tokens.first() {
        text = text.child(
            div()
                .min_w(px(mnemonic_min))
                .pr_2()
                .flex_none()
                .text_color(super::model::token_color(theme, mnemonic.kind))
                .child(SharedString::from(mnemonic.text.clone())),
        );
    }

    let mut operands = String::new();
    let mut highlights = Vec::with_capacity(tokens.len().saturating_sub(1));

    for token in tokens.iter().skip(1).filter(|token| !token.text.is_empty()) {
        let start = operands.len();
        operands.push_str(&token.text);

        let color = super::model::token_color(theme, token.kind);
        highlights.push((start..operands.len(), color.into()));
    }

    if !operands.is_empty() {
        text = text.child(
            div()
                .flex_none()
                .child(StyledText::new(operands).with_highlights(highlights)),
        );
    }

    text
}
