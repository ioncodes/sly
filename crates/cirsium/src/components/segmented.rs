use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    AlignItems, AnyElement, App, BorderStyle, Bounds, Corners, DefiniteLength, Display, Edges,
    Element, ElementId, FlexDirection, GlobalElementId, InspectorElementId, LayoutId, Pixels,
    SharedString, Size, Style, Window, div, point, px, quad, size, svg,
};

use crate::theme::Theme;

type SelectHandler = Rc<dyn Fn(&str, &mut Window, &mut App) + 'static>;

const PILL_RADIUS: f32 = 4.;
const TRACK_RADIUS: f32 = 6.;
const TRACK_PAD: f32 = 2.;
const SEGMENT_GAP: f32 = 2.;
const EASE: f32 = 0.35;
const SETTLE: f32 = 0.5;

struct Segment {
    key: SharedString,
    icon: SharedString,
    label: SharedString,
    enabled: bool,
}

pub struct Segmented {
    id: SharedString,
    options: Vec<Segment>,
    selected: SharedString,
    on_select: Option<SelectHandler>,
}

impl Segmented {
    pub fn new(id: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            options: Vec::new(),
            selected: SharedString::default(),
            on_select: None,
        }
    }

    pub fn option(
        mut self,
        key: impl Into<SharedString>,
        icon: impl Into<SharedString>,
        label: impl Into<SharedString>,
    ) -> Self {
        self.options.push(Segment {
            key: key.into(),
            icon: icon.into(),
            label: label.into(),
            enabled: true,
        });
        self
    }

    pub fn option_label(
        self,
        key: impl Into<SharedString>,
        label: impl Into<SharedString>,
    ) -> Self {
        self.option(key, "", label)
    }

    pub fn option_disabled(
        mut self,
        key: impl Into<SharedString>,
        label: impl Into<SharedString>,
    ) -> Self {
        self.options.push(Segment {
            key: key.into(),
            icon: SharedString::default(),
            label: label.into(),
            enabled: false,
        });
        self
    }

    pub fn selected(mut self, key: impl Into<SharedString>) -> Self {
        self.selected = key.into();
        self
    }

    pub fn on_select(mut self, handler: impl Fn(&str, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Rc::new(handler));
        self
    }

    fn button(&self, theme: &Theme, option: &Segment, active: bool) -> AnyElement {
        let icon_color = if !option.enabled {
            theme.text_dim
        } else if active {
            theme.on_accent
        } else {
            theme.text_secondary
        };

        let mut button = div()
            .id(SharedString::from(format!("{}-{}", self.id, option.key)))
            .flex()
            .items_center()
            .justify_center()
            .gap_1()
            .px_2()
            .py(px(1.))
            .rounded_sm()
            .text_xs()
            .when(!option.icon.is_empty(), |b| {
                b.child(
                    svg()
                        .path(option.icon.clone())
                        .size(px(12.))
                        .text_color(icon_color),
                )
            })
            .child(option.label.clone());

        if !option.enabled {
            button = button.text_color(theme.text_dim);
        } else if active {
            button = button.cursor_pointer().text_color(theme.on_accent);
        } else {
            let hover = theme.bg_header;
            button = button
                .cursor_pointer()
                .text_color(theme.text_secondary)
                .hover(move |s| s.bg(hover));
        }

        if option.enabled
            && let Some(handler) = self.on_select.clone()
        {
            let key = option.key.clone();
            button = button.on_click(move |_, window, cx| {
                cx.stop_propagation();
                handler(key.as_ref(), window, cx);
            });
        }

        button.into_any_element()
    }
}

pub struct SegmentedLayout {
    children: Vec<AnyElement>,
    child_ids: Vec<LayoutId>,
    active: Option<usize>,
}

pub struct SegmentedPrepaint {
    track: Bounds<Pixels>,
    pill: Option<Bounds<Pixels>>,
}

#[derive(Clone, Copy, Default)]
struct Persist {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    init: bool,
}

impl Element for Segmented {
    type RequestLayoutState = SegmentedLayout;
    type PrepaintState = SegmentedPrepaint;

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
        let theme = Theme::global(cx);
        let options = std::mem::take(&mut self.options);
        let current = options.iter().position(|o| o.key == self.selected);
        let active = options
            .iter()
            .enumerate()
            .cycle()
            .skip(current.unwrap_or(0))
            .take(options.len())
            .find(|(_, option)| option.enabled)
            .map(|(index, _)| index);

        if let Some(index) = active
            && self.selected != options[index].key
        {
            self.selected = options[index].key.clone();
            if let Some(handler) = self.on_select.clone() {
                let key = self.selected.clone();
                window.defer(cx, move |window, cx| handler(key.as_ref(), window, cx));
            }
        }

        let mut children: Vec<AnyElement> = Vec::with_capacity(options.len());
        let mut child_ids: Vec<LayoutId> = Vec::with_capacity(options.len());
        for (i, option) in options.iter().enumerate() {
            let mut child = self.button(&theme, option, active == Some(i));
            child_ids.push(child.request_layout(window, cx));
            children.push(child);
        }

        let style = Style {
            display: Display::Flex,
            flex_direction: FlexDirection::Row,
            align_items: Some(AlignItems::Center),
            gap: Size {
                width: DefiniteLength::from(px(SEGMENT_GAP)),
                height: DefiniteLength::from(px(0.)),
            },
            padding: Edges::all(DefiniteLength::from(px(TRACK_PAD))),
            ..Default::default()
        };

        let layout_id = window.request_layout(style, child_ids.iter().copied(), cx);
        (
            layout_id,
            SegmentedLayout {
                children,
                child_ids,
                active,
            },
        )
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        for child in &mut request_layout.children {
            child.prepaint(window, cx);
        }
        let pill = request_layout
            .active
            .and_then(|i| request_layout.child_ids.get(i))
            .map(|id| window.layout_bounds(*id));
        SegmentedPrepaint {
            track: bounds,
            pill,
        }
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let theme = Theme::global(cx);

        window.paint_quad(quad(
            prepaint.track,
            Corners::all(px(TRACK_RADIUS)),
            theme.bg_panel_alt,
            Edges::all(px(1.)),
            theme.border_subtle,
            BorderStyle::Solid,
        ));

        let global_id = id.expect("Segmented must have an id");
        let accent = theme.accent;
        let track_origin = prepaint.track.origin;

        if let Some(target) = prepaint.pill {
            let tx = f32::from(target.origin.x - track_origin.x);
            let ty = f32::from(target.origin.y - track_origin.y);
            let tw = f32::from(target.size.width);
            let th = f32::from(target.size.height);

            window.with_element_state::<Persist, _>(global_id, |persist, window| {
                let mut p = persist.unwrap_or_default();
                if !p.init {
                    p = Persist {
                        x: tx,
                        y: ty,
                        w: tw,
                        h: th,
                        init: true,
                    };
                } else {
                    let mut moving = false;

                    for (cur, tgt) in [
                        (&mut p.x, tx),
                        (&mut p.y, ty),
                        (&mut p.w, tw),
                        (&mut p.h, th),
                    ] {
                        if (tgt - *cur).abs() > SETTLE {
                            *cur += (tgt - *cur) * EASE;
                            moving = true;
                        } else {
                            *cur = tgt;
                        }
                    }

                    if moving {
                        window.request_animation_frame();
                    }
                }

                let pill = Bounds {
                    origin: point(track_origin.x + px(p.x), track_origin.y + px(p.y)),
                    size: size(px(p.w), px(p.h)),
                };
                window.paint_quad(quad(
                    pill,
                    Corners::all(px(PILL_RADIUS)),
                    accent,
                    Edges::default(),
                    gpui::transparent_black(),
                    BorderStyle::default(),
                ));
                ((), p)
            });
        } else {
            window.with_element_state::<Persist, _>(global_id, |_, _| ((), Persist::default()));
        }

        for child in &mut request_layout.children {
            child.paint(window, cx);
        }
    }
}

impl IntoElement for Segmented {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}
