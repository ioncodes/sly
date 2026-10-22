use std::collections::VecDeque;
use std::ops::Range;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Bounds, ClipboardItem, Context, CursorStyle, EntityInputHandler, EventEmitter,
    FocusHandle, Focusable, KeyBinding, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    Pixels, Point, ScrollHandle, SharedString, UTF16Selection, Window, actions, div, point, px,
};

use scripting::{LuaToken, tokenize_lua};

use self::complete::Suggestions;
use self::element::EditorElement;
use crate::components::line_geometry::LineGeometry;
use crate::components::{text_offsets, vertical_scroll_area};
use crate::theme::Theme;

mod complete;
mod element;
mod input;

pub use complete::Completion;

const CONTEXT: &str = "CodeEditor";
const INDENT: &str = "  ";
const HISTORY_LIMIT: usize = 256;

actions!(
    code_editor,
    [
        Backspace,
        Delete,
        MoveLeft,
        MoveRight,
        MoveUp,
        MoveDown,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        SelectAll,
        MoveHome,
        MoveEnd,
        SelectHome,
        SelectEnd,
        Newline,
        Indent,
        Dismiss,
        Paste,
        Cut,
        Copy,
        Undo,
        Redo,
        Save,
        SaveAs,
        Run,
    ]
);

pub fn register_code_editor_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, Some(CONTEXT)),
        KeyBinding::new("delete", Delete, Some(CONTEXT)),
        KeyBinding::new("left", MoveLeft, Some(CONTEXT)),
        KeyBinding::new("right", MoveRight, Some(CONTEXT)),
        KeyBinding::new("up", MoveUp, Some(CONTEXT)),
        KeyBinding::new("down", MoveDown, Some(CONTEXT)),
        KeyBinding::new("shift-left", SelectLeft, Some(CONTEXT)),
        KeyBinding::new("shift-right", SelectRight, Some(CONTEXT)),
        KeyBinding::new("shift-up", SelectUp, Some(CONTEXT)),
        KeyBinding::new("shift-down", SelectDown, Some(CONTEXT)),
        KeyBinding::new("cmd-a", SelectAll, Some(CONTEXT)),
        KeyBinding::new("ctrl-a", SelectAll, Some(CONTEXT)),
        KeyBinding::new("cmd-c", Copy, Some(CONTEXT)),
        KeyBinding::new("ctrl-c", Copy, Some(CONTEXT)),
        KeyBinding::new("cmd-x", Cut, Some(CONTEXT)),
        KeyBinding::new("ctrl-x", Cut, Some(CONTEXT)),
        KeyBinding::new("cmd-v", Paste, Some(CONTEXT)),
        KeyBinding::new("ctrl-v", Paste, Some(CONTEXT)),
        KeyBinding::new("cmd-z", Undo, Some(CONTEXT)),
        KeyBinding::new("ctrl-z", Undo, Some(CONTEXT)),
        KeyBinding::new("cmd-shift-z", Redo, Some(CONTEXT)),
        KeyBinding::new("ctrl-shift-z", Redo, Some(CONTEXT)),
        KeyBinding::new("ctrl-y", Redo, Some(CONTEXT)),
        KeyBinding::new("cmd-s", Save, Some(CONTEXT)),
        KeyBinding::new("ctrl-s", Save, Some(CONTEXT)),
        KeyBinding::new("cmd-shift-s", SaveAs, Some(CONTEXT)),
        KeyBinding::new("ctrl-shift-s", SaveAs, Some(CONTEXT)),
        KeyBinding::new("home", MoveHome, Some(CONTEXT)),
        KeyBinding::new("end", MoveEnd, Some(CONTEXT)),
        KeyBinding::new("shift-home", SelectHome, Some(CONTEXT)),
        KeyBinding::new("shift-end", SelectEnd, Some(CONTEXT)),
        KeyBinding::new("enter", Newline, Some(CONTEXT)),
        KeyBinding::new("tab", Indent, Some(CONTEXT)),
        KeyBinding::new("escape", Dismiss, Some(CONTEXT)),
        KeyBinding::new("cmd-enter", Run, Some(CONTEXT)),
        KeyBinding::new("ctrl-enter", Run, Some(CONTEXT)),
    ]);
}

pub enum CodeEditorEvent {
    Changed,
    Save,
    SaveAs,
    Run,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct EditState {
    text: String,
    selected_range: Range<usize>,
    selection_reversed: bool,
}

#[derive(Default)]
struct EditHistory {
    undo: VecDeque<EditState>,
    redo: VecDeque<EditState>,
}

impl EditHistory {
    fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    fn record(&mut self, state: EditState) {
        Self::push(&mut self.undo, state);
        self.redo.clear();
    }

    fn undo(&mut self, current: EditState) -> Option<EditState> {
        let previous = self.undo.pop_back()?;
        Self::push(&mut self.redo, current);
        Some(previous)
    }

    fn redo(&mut self, current: EditState) -> Option<EditState> {
        let next = self.redo.pop_back()?;
        Self::push(&mut self.undo, current);
        Some(next)
    }

    fn push(stack: &mut VecDeque<EditState>, state: EditState) {
        if stack.back() == Some(&state) {
            return;
        }
        if stack.len() == HISTORY_LIMIT {
            stack.pop_front();
        }
        stack.push_back(state);
    }
}

pub struct CodeEditor {
    focus_handle: FocusHandle,
    text: String,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    tokens: Vec<LuaToken>,
    line_starts: Vec<usize>,
    error_line: Option<usize>,
    scroll: ScrollHandle,
    is_selecting: bool,
    reveal_cursor: bool,
    placeholder: SharedString,
    completions: Vec<Completion>,
    suggestions: Option<Suggestions>,
    hover: Option<(Point<Pixels>, Completion)>,
    layout: element::Layout,
    history: EditHistory,
}

impl CodeEditor {
    pub fn new(cx: &mut Context<Self>, text: impl Into<String>) -> Self {
        let text = text.into();
        let mut editor = Self {
            focus_handle: cx.focus_handle(),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            tokens: Vec::new(),
            line_starts: Vec::new(),
            error_line: None,
            scroll: ScrollHandle::new(),
            is_selecting: false,
            reveal_cursor: false,
            placeholder: "-- write Lua here".into(),
            completions: Vec::new(),
            suggestions: None,
            hover: None,
            layout: element::Layout::default(),
            history: EditHistory::default(),
            text,
        };
        editor.reparse();
        editor
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn set_text(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.text = text.into();
        self.selected_range = 0..0;
        self.selection_reversed = false;
        self.marked_range = None;
        self.suggestions = None;
        self.history.clear();
        self.reparse();
        cx.emit(CodeEditorEvent::Changed);
        cx.notify();
    }

    pub fn set_error_line(&mut self, line: Option<usize>, cx: &mut Context<Self>) {
        if self.error_line != line {
            self.error_line = line;
            cx.notify();
        }
    }

    pub fn insert(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle, cx);
        self.replace_text_in_range(None, text, window, cx);
        self.close_suggestions();
    }

    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    fn reparse(&mut self) {
        self.tokens = tokenize_lua(&self.text);

        self.line_starts.clear();
        self.line_starts.push(0);
        self.line_starts.extend(
            self.text
                .bytes()
                .enumerate()
                .filter(|(_, byte)| *byte == b'\n')
                .map(|(ix, _)| ix + 1),
        );
    }

    fn edit_state(&self) -> EditState {
        EditState {
            text: self.text.clone(),
            selected_range: self.selected_range.clone(),
            selection_reversed: self.selection_reversed,
        }
    }

    fn record_edit(&mut self) {
        self.history.record(self.edit_state());
    }

    fn replace_text_with_history(&mut self, range: Range<usize>, new_text: &str) {
        if self.text[range.clone()] != *new_text {
            self.record_edit();
        }
        self.text.replace_range(range, new_text);
    }

    fn restore_edit(&mut self, state: EditState, cx: &mut Context<Self>) {
        self.text = state.text;
        self.selected_range = state.selected_range;
        self.selection_reversed = state.selection_reversed;
        self.marked_range = None;
        self.reveal_cursor = true;
        self.close_suggestions();
        self.reparse();
        cx.emit(CodeEditorEvent::Changed);
        cx.notify();
    }

    fn line_at(&self, offset: usize) -> usize {
        self.line_starts
            .partition_point(|start| *start <= offset)
            .saturating_sub(1)
    }

    fn line_range(&self, line: usize) -> Range<usize> {
        let start = self
            .line_starts
            .get(line)
            .copied()
            .unwrap_or(self.text.len());
        let end = self
            .line_starts
            .get(line + 1)
            .map(|next| next - 1)
            .unwrap_or(self.text.len());
        start..end.max(start)
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        self.reveal_cursor = true;
        self.close_suggestions();
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }

        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }

        self.reveal_cursor = true;
        cx.notify();
    }

    fn vertical(&self, offset: usize, delta: isize) -> usize {
        let line = self.line_at(offset);
        let column = offset - self.line_starts[line];
        let target = line.saturating_add_signed(delta).min(self.line_count() - 1);

        let range = self.line_range(target);
        let mut candidate = (range.start + column).min(range.end);
        while candidate > range.start && !self.text.is_char_boundary(candidate) {
            candidate -= 1;
        }
        candidate
    }

    fn left(&mut self, _: &MoveLeft, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx);
        }
    }

    fn right(&mut self, _: &MoveRight, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.end, cx);
        }
    }

    fn up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(open) = self.suggestions.as_mut() {
            open.move_selection(-1);
            cx.notify();
            return;
        }
        self.move_to(self.vertical(self.cursor_offset(), -1), cx);
    }

    fn down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(open) = self.suggestions.as_mut() {
            open.move_selection(1);
            cx.notify();
            return;
        }
        self.move_to(self.vertical(self.cursor_offset(), 1), cx);
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.vertical(self.cursor_offset(), -1), cx);
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.vertical(self.cursor_offset(), 1), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.text.len(), cx);
    }

    fn home(&mut self, _: &MoveHome, _: &mut Window, cx: &mut Context<Self>) {
        let start = self.line_home(self.cursor_offset());
        self.move_to(start, cx);
    }

    fn end(&mut self, _: &MoveEnd, _: &mut Window, cx: &mut Context<Self>) {
        let end = self.line_range(self.line_at(self.cursor_offset())).end;
        self.move_to(end, cx);
    }

    fn select_home(&mut self, _: &SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        let start = self.line_home(self.cursor_offset());
        self.select_to(start, cx);
    }

    fn select_end(&mut self, _: &SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        let end = self.line_range(self.line_at(self.cursor_offset())).end;
        self.select_to(end, cx);
    }

    fn line_home(&self, offset: usize) -> usize {
        let range = self.line_range(self.line_at(offset));
        let indent = self.text[range.clone()]
            .find(|c: char| !c.is_whitespace())
            .map(|ix| range.start + ix)
            .unwrap_or(range.end);

        if offset == indent {
            range.start
        } else {
            indent
        }
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let cursor = self.cursor_offset();
            let start = self.line_range(self.line_at(cursor)).start;
            let target = if cursor >= start + INDENT.len()
                && self.text[start..cursor].ends_with(INDENT)
                && self.text[start..cursor].trim().is_empty()
            {
                cursor - INDENT.len()
            } else {
                self.previous_boundary(cursor)
            };
            self.select_to(target, cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        if self.close_suggestions() {
            cx.notify();
        }
    }

    fn newline(&mut self, _: &Newline, window: &mut Window, cx: &mut Context<Self>) {
        if self.accept_suggestion(cx) {
            return;
        }

        let range = self.line_range(self.line_at(self.selected_range.start));
        let indent: String = self.text[range]
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();

        self.replace_text_in_range(None, &format!("\n{indent}"), window, cx);
    }

    fn indent(&mut self, _: &Indent, window: &mut Window, cx: &mut Context<Self>) {
        if self.accept_suggestion(cx) {
            return;
        }
        self.replace_text_in_range(None, INDENT, window, cx);
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.text[self.selected_range.clone()].to_string(),
            ));
        }
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.text[self.selected_range.clone()].to_string(),
            ));
            self.replace_text_in_range(None, "", window, cx);
        }
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_text_in_range(None, &text, window, cx);
        }
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        let current = self.edit_state();
        if let Some(previous) = self.history.undo(current) {
            self.restore_edit(previous, cx);
        }
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        let current = self.edit_state();
        if let Some(next) = self.history.redo(current) {
            self.restore_edit(next, cx);
        }
    }

    fn save(&mut self, _: &Save, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(CodeEditorEvent::Save);
    }

    fn save_as(&mut self, _: &SaveAs, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(CodeEditorEvent::SaveAs);
    }

    fn run(&mut self, _: &Run, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(CodeEditorEvent::Run);
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        let offset = self.offset_for_position(event.position);

        match event.click_count {
            2 => {
                let word = self.word_at(offset);
                self.move_to(word.start, cx);
                self.select_to(word.end, cx);
                return;
            }
            count if count >= 3 => {
                let line = self.line_range(self.line_at(offset));
                self.move_to(line.start, cx);
                self.select_to(line.end, cx);
                return;
            }
            _ => {}
        }

        self.is_selecting = true;
        if event.modifiers.shift {
            self.select_to(offset, cx);
        } else {
            self.move_to(offset, cx);
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            let offset = self.offset_for_position(event.position);
            self.select_to(offset, cx);
            return;
        }

        self.update_hover(event.position, cx);
    }

    fn update_hover(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(bounds) = self.layout.bounds else {
            return;
        };

        let inside = bounds.contains(&position);
        let hovered = inside
            .then(|| self.documented_at(self.offset_for_position(position)))
            .flatten()
            .map(|item| {
                let local = position - bounds.origin;
                (point(local.x, local.y), item)
            });

        let changed = match (&self.hover, &hovered) {
            (None, None) => false,
            (Some((_, a)), Some((_, b))) => a.label != b.label,
            _ => true,
        };

        if changed {
            self.hover = hovered;
            cx.notify();
        }
    }

    fn offset_for_position(&self, position: Point<Pixels>) -> usize {
        let Some(bounds) = self.layout.bounds else {
            return self.cursor_offset();
        };

        let local = position - bounds.origin;
        let row = (f32::from(local.y) / f32::from(self.layout.line_height).max(1.)).floor();
        let line = (row.max(0.) as usize).min(self.line_count().saturating_sub(1));

        let range = self.line_range(line);
        let x = local.x - self.layout.gutter;
        match self.layout.lines.get(&line) {
            Some(shaped) => {
                let index = shaped.index_at(point(x, px(0.)), self.layout.line_height);
                range.start + index.min(range.len())
            }
            None => range.start,
        }
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        text_offsets::offset_to_utf16(&self.text, offset)
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        text_offsets::range_to_utf16(&self.text, range)
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        text_offsets::range_from_utf16(&self.text, range)
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        text_offsets::previous_boundary(&self.text, offset)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        text_offsets::next_boundary(&self.text, offset)
    }
}

impl EventEmitter<CodeEditorEvent> for CodeEditor {}

impl Focusable for CodeEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for CodeEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::global(cx);
        let t = &*theme;
        let focused = self.focus_handle.is_focused(window);

        let overlay = self
            .suggestion_popup(t, cx)
            .or_else(|| self.hover_popup(t))
            .into_iter();

        let editor = div()
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::indent))
            .on_action(cx.listener(Self::dismiss))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::save_as))
            .on_action(cx.listener(Self::run))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .cursor(CursorStyle::IBeam)
            .id("code-editor")
            .flex_1()
            .min_h(px(0.))
            .w_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .bg(t.bg_panel_alt)
            .border_1()
            .border_color(if focused {
                t.border_strong
            } else {
                t.border_subtle
            })
            .rounded_md()
            .font_family(t.mono_font)
            .text_size(px(12.))
            .text_color(t.text_primary)
            .child(EditorElement {
                editor: cx.entity(),
            });

        let editor = vertical_scroll_area(editor, self.scroll.clone(), t).flex_1();

        div()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .child(editor)
            .children(overlay)
    }
}

const POPUP_ROW: f32 = 18.;
const POPUP_PAD: f32 = 8.;

impl CodeEditor {
    fn popup_frame(&self, anchor: Point<Pixels>, height: Pixels) -> (Pixels, Pixels) {
        let offset = self.scroll.offset();
        let line_height = self.layout.line_height.max(px(1.));
        let viewport = self.scroll.bounds().size.height;

        let below = anchor.y + offset.y + line_height;
        let above = anchor.y + offset.y - height;
        let top = if viewport > px(0.) && below + height > viewport && above > px(0.) {
            above
        } else {
            below
        };

        ((anchor.x + offset.x).max(px(0.)), top.max(px(0.)))
    }

    fn popup_card(&self, t: &Theme, left: Pixels, top: Pixels) -> gpui::Div {
        div()
            .absolute()
            .left(left)
            .top(top)
            .flex()
            .flex_col()
            .p_1()
            .bg(t.bg_header)
            .border_1()
            .border_color(t.border_strong)
            .rounded_md()
            .shadow_md()
            .font_family(t.mono_font)
            .text_xs()
    }

    fn suggestion_popup(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let open = self.suggestions.as_ref()?;
        let height = px(open.matches.len() as f32 * POPUP_ROW + POPUP_PAD);
        let (left, top) = self.popup_frame(self.layout.caret, height);

        let rows = open.matches.iter().enumerate().map(|(ix, item)| {
            let selected = ix == open.selected;
            let entity = cx.entity();

            div()
                .id(SharedString::from(format!("completion-{ix}")))
                .flex()
                .items_center()
                .gap_2()
                .h(px(POPUP_ROW))
                .px_1()
                .rounded_sm()
                .when(selected, |row| row.bg(t.accent_soft))
                .cursor_pointer()
                .on_click(move |_, _, cx| {
                    entity.update(cx, |editor, cx| {
                        if let Some(open) = editor.suggestions.as_mut() {
                            open.selected = ix;
                        }
                        editor.accept_suggestion(cx);
                    });
                })
                .child(
                    div()
                        .text_color(if selected { t.accent } else { t.text_primary })
                        .child(item.label.clone()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_color(t.text_dim)
                        .truncate()
                        .child(item.detail.clone()),
                )
        });

        Some(
            self.popup_card(t, left, top)
                .w(px(360.))
                .children(rows)
                .into_any_element(),
        )
    }

    fn hover_popup(&self, t: &Theme) -> Option<AnyElement> {
        let (anchor, item) = self.hover.as_ref()?;
        let height = px(2. * POPUP_ROW + POPUP_PAD);
        let (left, top) = self.popup_frame(*anchor, height);

        Some(
            self.popup_card(t, left, top)
                .max_w(px(420.))
                .child(div().text_color(t.accent).child(item.detail.clone()))
                .child(
                    div()
                        .pt(px(2.))
                        .font_family(t.ui_font)
                        .text_color(t.text_secondary)
                        .child(item.doc.clone()),
                )
                .into_any_element(),
        )
    }
}
