use gpui::prelude::*;
use gpui::{
    Anchor, AnyElement, App, AvailableSpace, Bounds, Element, ElementId, Entity, FocusHandle,
    GlobalElementId, InspectorElementId, LayoutId, Pixels, SharedString, Size, Style, Window, div,
    point, px, size,
};

use super::menu::overlay;
use super::{CONTROL_HEIGHT, field_button};
use crate::assets::icon;
use crate::theme::Theme;

const GAP: f32 = 8.;
const MAX_CONTENT: Size<AvailableSpace> = Size {
    width: AvailableSpace::MaxContent,
    height: AvailableSpace::MaxContent,
};

pub(super) type OptionsBuilder = Box<dyn Fn(bool, &Theme) -> AnyElement>;

pub(super) struct HeaderOptions {
    pub id: SharedString,
    pub primary: Option<AnyElement>,
    pub build: OptionsBuilder,
}

pub(super) struct MenuState {
    open: bool,
    focus: FocusHandle,
    previous_focus: Option<FocusHandle>,
}

impl HeaderOptions {
    fn primary_width(&mut self, window: &mut Window, cx: &mut App) -> Pixels {
        self.primary.as_mut().map_or(px(0.), |primary| {
            primary.layout_as_root(MAX_CONTENT, window, cx).width + px(GAP)
        })
    }

    fn prepaint_row(
        &mut self,
        mut options: AnyElement,
        options_size: Size<Pixels>,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Vec<AnyElement> {
        let options_x = bounds.right() - options_size.width;
        let mut elements = Vec::with_capacity(2);
        if let Some(mut primary) = self.primary.take() {
            let primary_size = primary.layout_as_root(MAX_CONTENT, window, cx);
            primary.prepaint_at(
                point(
                    options_x - px(GAP) - primary_size.width,
                    bounds.center().y - primary_size.height / 2.,
                ),
                window,
                cx,
            );
            elements.push(primary);
        }
        options.prepaint_at(
            point(options_x, bounds.center().y - options_size.height / 2.),
            window,
            cx,
        );
        elements.push(options);
        elements
    }

    fn close(state: &Entity<MenuState>, window: &mut Window, cx: &mut App) {
        state.update(cx, |state, cx| {
            if state.open {
                state.open = false;
                if state.focus.contains_focused(window, cx) {
                    window.focus(&state.focus, cx);
                }
                cx.notify();
            }
        });
    }
}

impl IntoElement for HeaderOptions {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for HeaderOptions {
    type RequestLayoutState = Entity<MenuState>;
    type PrepaintState = Vec<AnyElement>;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone().into())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let state = window.use_keyed_state(self.id.clone(), cx, |_, cx| MenuState {
            open: false,
            focus: cx.focus_handle().tab_index(0),
            previous_focus: None,
        });
        let primary_width = self.primary_width(window, cx);
        let style = Style {
            flex_grow: 1.,
            flex_shrink: 1.,
            flex_basis: px(0.).into(),
            min_size: size((primary_width + px(26.)).into(), px(CONTROL_HEIGHT).into()),
            size: size(gpui::auto(), px(CONTROL_HEIGHT).into()),
            ..Default::default()
        };
        (window.request_layout(style, [], cx), state)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let theme = Theme::global(cx);
        let mut inline = (self.build)(false, &theme);
        let inline_size = inline.layout_as_root(MAX_CONTENT, window, cx);
        let primary_width = self.primary_width(window, cx);
        if primary_width + inline_size.width <= bounds.size.width {
            if state.read(cx).open {
                Self::close(state, window, cx);
                if state.read(cx).focus.is_focused(window) {
                    if let Some(previous) = state.read(cx).previous_focus.clone() {
                        window.focus(&previous, cx);
                    } else {
                        window.blur(cx);
                    }
                }
            }
            return self.prepaint_row(inline, inline_size, bounds, window, cx);
        }

        let focus = state.read(cx).focus.clone();
        let toggle_state = state.clone();
        let key_state = state.clone();
        let mut button = field_button(
            &theme,
            "header-options-button",
            icon::SETTINGS,
            move |_, window, cx| {
                cx.stop_propagation();
                toggle_state.update(cx, |state, cx| {
                    if !state.open {
                        state.previous_focus = window.focused(cx);
                    }
                    state.open = !state.open;
                    window.focus(&state.focus, cx);
                    cx.notify();
                });
            },
        )
        .track_focus(&focus)
        .on_key_down(move |event, window, cx| {
            match event.keystroke.key.as_str() {
                "escape" => Self::close(&key_state, window, cx),
                "enter" | "space" => {
                    key_state.update(cx, |state, cx| {
                        state.open = !state.open;
                        cx.notify();
                    });
                }
                "tab" => {
                    if event.keystroke.modifiers.shift {
                        window.focus_prev(cx);
                    } else {
                        window.focus_next(cx);
                    }
                }
                _ => return,
            }
            cx.stop_propagation();
        });

        if state.read(cx).open {
            let dismiss_state = state.clone();
            let trigger_bounds = Bounds::new(
                point(bounds.right() - px(26.), bounds.origin.y),
                size(px(26.), bounds.size.height),
            );
            let content = div()
                .id("header-options-content")
                .p_2()
                .min_w(px(160.))
                .on_click(|_, _, cx| cx.stop_propagation())
                .child((self.build)(true, &theme));
            button = button.child(overlay(
                &theme,
                point(bounds.right(), bounds.bottom() + px(4.)),
                Anchor::TopRight,
                "header-options-menu",
                content,
                move |event, window, cx| {
                    if !trigger_bounds.contains(&event.position) {
                        Self::close(&dismiss_state, window, cx);
                    }
                },
            ));
        }

        let mut button = button.into_any_element();
        let button_size = button.layout_as_root(MAX_CONTENT, window, cx);
        self.prepaint_row(button, button_size, bounds, window, cx)
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _state: &mut Self::RequestLayoutState,
        elements: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        for element in elements {
            element.paint(window, cx);
        }
    }
}
