use gpui::prelude::*;
use gpui::{App, ClickEvent, FontWeight, SharedString, Window, div, px, svg};

use super::ClickHandler;
use super::header_options::{HeaderOptions, OptionsBuilder};
use crate::assets::icon;
use crate::theme::Theme;

pub const HEADER_HEIGHT: f32 = 36.;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Sizing {
    #[default]
    Fixed,
    Grow,
    Scroll,
}

#[derive(IntoElement)]
pub struct Panel {
    id: SharedString,
    title: SharedString,
    meta: Option<SharedString>,
    glyph: Option<SharedString>,
    collapsed: bool,
    sizing: Sizing,
    on_toggle: Option<ClickHandler>,
    header_action: Option<gpui::AnyElement>,
    header_options: Option<OptionsBuilder>,
    body: Option<gpui::AnyElement>,
}

impl Panel {
    pub fn new(id: impl Into<SharedString>, title: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            meta: None,
            glyph: None,
            collapsed: false,
            sizing: Sizing::Fixed,
            on_toggle: None,
            header_action: None,
            header_options: None,
            body: None,
        }
    }

    pub fn meta(mut self, meta: impl Into<SharedString>) -> Self {
        self.meta = Some(meta.into());
        self
    }

    pub fn meta_opt(mut self, meta: Option<SharedString>) -> Self {
        self.meta = meta;
        self
    }

    pub fn glyph(mut self, icon_path: impl Into<SharedString>) -> Self {
        self.glyph = Some(icon_path.into());
        self
    }

    pub fn glyph_opt(mut self, icon_path: Option<SharedString>) -> Self {
        self.glyph = icon_path;
        self
    }

    pub fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }

    pub fn grow(mut self, grow: bool) -> Self {
        self.sizing = if grow { Sizing::Grow } else { Sizing::Fixed };
        self
    }

    pub fn scrollable(mut self, scrollable: bool) -> Self {
        self.sizing = if scrollable {
            Sizing::Scroll
        } else {
            Sizing::Fixed
        };
        self
    }

    pub fn on_toggle(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_toggle = Some(Box::new(handler));
        self
    }

    pub fn on_toggle_opt(mut self, handler: Option<ClickHandler>) -> Self {
        self.on_toggle = handler;
        self
    }

    pub fn header_action(mut self, action: impl IntoElement) -> Self {
        self.header_action = Some(action.into_any_element());
        self
    }

    pub fn header_options(
        mut self,
        build: impl Fn(bool, &Theme) -> gpui::AnyElement + 'static,
    ) -> Self {
        self.header_options = Some(Box::new(build));
        self
    }

    pub fn child(mut self, body: impl IntoElement) -> Self {
        self.body = Some(body.into_any_element());
        self
    }
}

pub struct PanelChrome {
    id: SharedString,
    title: SharedString,
    glyph: Option<SharedString>,
    collapsed: bool,
    on_toggle: Option<ClickHandler>,
}

impl PanelChrome {
    pub fn new(id: impl Into<SharedString>, title: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            glyph: None,
            collapsed: false,
            on_toggle: None,
        }
    }

    pub fn glyph_opt(mut self, icon_path: Option<SharedString>) -> Self {
        self.glyph = icon_path;
        self
    }

    pub fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }

    pub fn id(&self) -> &SharedString {
        &self.id
    }

    pub fn on_toggle(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_toggle = Some(Box::new(handler));
        self
    }

    pub fn panel(self) -> Panel {
        Panel::new(self.id, self.title)
            .collapsed(self.collapsed)
            .on_toggle_opt(self.on_toggle)
            .glyph_opt(self.glyph)
    }
}

impl RenderOnce for Panel {
    fn render(mut self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::global(cx);
        let t = &*theme;
        let chevron = if self.collapsed {
            icon::CHEVRON_RIGHT
        } else {
            icon::CHEVRON_DOWN
        };

        let responsive = self.header_options.is_some();
        let mut left = div()
            .flex()
            .flex_1()
            .items_center()
            .gap_2()
            .min_w(px(0.))
            .overflow_hidden()
            .when(responsive, |left| left.flex_none());
        if let Some(glyph) = self.glyph.clone() {
            left = left.child(
                svg()
                    .flex_none()
                    .path(glyph)
                    .size(px(12.))
                    .text_color(t.text_muted),
            );
        }

        left = left.child(
            div()
                .flex_none()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.text_secondary)
                .whitespace_nowrap()
                .child(self.title.to_uppercase()),
        );

        if let Some(meta) = self.meta.clone() {
            left = left.child(
                div()
                    .flex_none()
                    .font_family(t.mono_font)
                    .text_xs()
                    .text_color(t.text_muted)
                    .whitespace_nowrap()
                    .child(meta),
            );
        }
        let mut right = div().flex().flex_none().items_center().gap_2();
        if !responsive && let Some(action) = self.header_action.take() {
            right = right.child(action);
        }
        right = right.child(svg().path(chevron).size(px(12.)).text_color(t.text_muted));

        let hover_bg = t.bg_panel_alt;
        let mut header = div()
            .id(self.id.clone())
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .h(px(HEADER_HEIGHT))
            .px_3()
            .bg(t.bg_header)
            .border_b_1()
            .border_color(t.border_subtle)
            .hover(move |s| s.bg(hover_bg))
            .child(left);
        if let Some(build) = self.header_options {
            header = header.gap_2().child(HeaderOptions {
                id: format!("{}-options", self.id).into(),
                primary: self.header_action,
                build,
            });
        }
        header = header.child(right);
        if let Some(toggle) = self.on_toggle {
            header = header.on_click(toggle);
        }

        let fill = self.sizing == Sizing::Grow && !self.collapsed;
        let bounded = self.sizing != Sizing::Fixed && !self.collapsed;

        let mut card = div()
            .flex()
            .flex_col()
            .bg(t.bg_panel)
            .border_1()
            .border_color(t.border_subtle)
            .overflow_hidden()
            .when(fill, |c| c.flex_1())
            .when(bounded, |c| c.min_h(px(0.)))
            .child(header);

        if !self.collapsed
            && let Some(body) = self.body
        {
            card = card.child(
                div()
                    .flex()
                    .flex_col()
                    .p_2()
                    .when(bounded, |b| b.flex_1().min_h(px(0.)))
                    .child(body),
            );
        }

        card
    }
}
