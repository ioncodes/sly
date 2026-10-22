use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    App, ClickEvent, Div, FontWeight, SharedString, Stateful, UniformListScrollHandle, Window, div,
    px, uniform_list,
};

use crate::assets::icon;
use crate::components::{
    ClickHandler, DETAIL_INDENT, INDENT_STEP, Panel, RowCallback, chevron_slot,
    empty_state_centered, hex, label_text, row_toggle, uniform_vertical_scroll_area,
};

use crate::diff::highlight_color;
use crate::theme::Theme;

pub use debugger_model::{MmioFieldView, MmioGroup, MmioRegisterView, MmioView};

#[derive(IntoElement)]
pub struct MmioWidget {
    view: Arc<MmioView>,
    expanded: HashSet<SharedString>,
    collapsed: bool,
    priority: Option<SharedString>,
    scroll: UniformListScrollHandle,
    on_toggle: Option<ClickHandler>,
    on_toggle_register: Option<RowCallback>,
}

impl MmioWidget {
    pub const ID: &'static str = debugger_model::panel::MMIO;

    pub fn new(view: Arc<MmioView>) -> Self {
        Self {
            view,
            expanded: HashSet::new(),
            collapsed: false,
            priority: None,
            scroll: UniformListScrollHandle::new(),
            on_toggle: None,
            on_toggle_register: None,
        }
    }

    pub fn expanded(mut self, expanded: HashSet<SharedString>) -> Self {
        self.expanded = expanded;
        self
    }

    pub fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }

    pub fn priority(mut self, group: Option<SharedString>) -> Self {
        self.priority = group;
        self
    }

    pub fn scroll(mut self, scroll: UniformListScrollHandle) -> Self {
        self.scroll = scroll;
        self
    }

    pub fn on_toggle(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_toggle = Some(Box::new(handler));
        self
    }

    pub fn on_toggle_register(
        mut self,
        handler: impl Fn(SharedString, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_toggle_register = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for MmioWidget {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::global(cx);
        let t = &*theme;
        let scroll = self.scroll.clone();

        let count = self
            .view
            .groups
            .iter()
            .map(|group| group.registers.len())
            .sum::<usize>();

        let body = if count == 0 {
            empty_state_centered(t, "No registers")
        } else {
            let mut groups: Vec<_> = self.view.groups.iter().enumerate().collect();
            if let Some(priority) = &self.priority {
                groups.sort_by_key(|(_, g)| g.name.as_ref() != priority.as_ref());
            }

            let mut rows = Vec::with_capacity(count + groups.len());
            for (group_ix, group) in groups {
                rows.push(MmioRow::Group(group_ix));

                for (register_ix, reg) in group.registers.iter().enumerate() {
                    let is_expanded = self.expanded.contains(reg.id.as_ref());
                    rows.push(MmioRow::Register(group_ix, register_ix, is_expanded));

                    if is_expanded {
                        self::append_fields(
                            &mut rows,
                            group_ix,
                            register_ix,
                            &reg.fields,
                            &mut Vec::new(),
                        );
                    }
                }
            }

            let view = self.view;
            let on_toggle = self.on_toggle_register;

            let list = uniform_list("mmio-list", rows.len(), move |range, _, cx| {
                let theme = Theme::global(cx);

                range
                    .map(|ix| match &rows[ix] {
                        MmioRow::Group(group) => {
                            self::group_header(&theme, view.groups[*group].name.clone().into())
                                .into_any_element()
                        }
                        MmioRow::Register(group, register, expanded) => self::register_row(
                            &theme,
                            &view.groups[*group].registers[*register],
                            *expanded,
                            on_toggle.clone(),
                        )
                        .into_any_element(),
                        MmioRow::Field(group, register, path) => {
                            let (last, parents) = path.split_last().unwrap();

                            let mut fields = &view.groups[*group].registers[*register].fields;
                            for &index in parents {
                                fields = &fields[index].children;
                            }

                            self::field_row(&theme, &fields[*last], parents.len())
                                .into_any_element()
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .track_scroll(&scroll)
            .flex_1()
            .min_h(px(0.));

            uniform_vertical_scroll_area(list, &scroll, t).flex_1()
        };

        Panel::new(Self::ID, "MMIO")
            .glyph(icon::LIST)
            .grow(true)
            .collapsed(self.collapsed)
            .meta(format!("{count}"))
            .on_toggle_opt(self.on_toggle)
            .child(body)
    }
}

const ROW_HEIGHT: f32 = 22.;

enum MmioRow {
    Group(usize),
    Register(usize, usize, bool),
    Field(usize, usize, Vec<usize>),
}

fn append_fields(
    rows: &mut Vec<MmioRow>,
    group: usize,
    register: usize,
    fields: &[MmioFieldView],
    path: &mut Vec<usize>,
) {
    for (index, field) in fields.iter().enumerate() {
        path.push(index);
        rows.push(MmioRow::Field(group, register, path.clone()));
        append_fields(rows, group, register, &field.children, path);
        path.pop();
    }
}

fn group_header(theme: &Theme, name: SharedString) -> Div {
    div()
        .flex()
        .h(px(ROW_HEIGHT))
        .items_center()
        .px_1()
        .py(px(2.))
        .bg(theme.bg_panel_alt)
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.text_muted)
        .child(name.to_uppercase())
}

fn register_row(
    theme: &Theme,
    reg: &MmioRegisterView,
    expanded: bool,
    on_toggle: Option<RowCallback>,
) -> Stateful<Div> {
    let expandable = !reg.fields.is_empty();
    let chevron = chevron_slot(theme, expandable, expanded);

    let name = div()
        .text_xs()
        .text_color(theme.text_secondary)
        .child(SharedString::from(reg.name.clone()));

    let addr = div()
        .font_family(theme.mono_font)
        .text_xs()
        .text_color(theme.text_dim)
        .child(SharedString::from(reg.addr.clone()));

    let value = div()
        .font_family(theme.mono_font)
        .text_xs()
        .text_color(highlight_color(theme, reg.highlight))
        .child(hex(reg.value, reg.bits));

    let mut row = div()
        .id(reg.id.clone())
        .flex()
        .h(px(ROW_HEIGHT))
        .items_center()
        .gap_2()
        .px_1()
        .py(px(3.))
        .child(chevron)
        .child(name)
        .child(addr)
        .child(div().flex_1())
        .child(value);

    if expandable && let Some(handler) = on_toggle {
        row = row_toggle(row, theme, reg.id.clone().into(), handler);
    }

    row
}

fn field_row(theme: &Theme, field: &MmioFieldView, depth: usize) -> Div {
    let indent = px(DETAIL_INDENT + depth as f32 * INDENT_STEP);

    let bit_label = div()
        .font_family(theme.mono_font)
        .text_xs()
        .text_color(theme.text_dim)
        .w(px(44.))
        .flex_none()
        .child(SharedString::from(field.bit_label.clone()));

    let name = label_text(theme, field.name.clone());

    let value_color = match (field.is_bool, field.value != 0) {
        (true, true) => theme.accent,
        (true, false) => theme.text_muted,
        (false, _) => theme.text_secondary,
    };
    let value = div()
        .font_family(theme.mono_font)
        .text_xs()
        .text_color(value_color)
        .child(SharedString::from(field.display.clone()));

    div()
        .flex()
        .h(px(ROW_HEIGHT))
        .items_center()
        .gap_2()
        .pl(indent)
        .pr_1()
        .py(px(2.))
        .child(bit_label)
        .child(name)
        .child(div().flex_1())
        .child(value)
}
