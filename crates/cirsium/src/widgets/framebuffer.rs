use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    App, ClickEvent, DevicePixels, ImageFiltering, ObjectFit, SurfaceSource, Window, div, px, size,
    surface,
};
use gpui_wgpu::{WgpuContextHandle, wgpu};

use crate::assets::icon;
use crate::components::{ClickHandler, Panel, empty_state};
use crate::theme::Theme;

#[derive(Clone)]
pub struct FramebufferView {
    pub width: u32,
    pub height: u32,
    state: Rc<RefCell<FramebufferState>>,
}

struct FramebufferState {
    pixels: Vec<u8>,
    dirty: bool,
    gpu: Option<FramebufferTexture>,
}

struct FramebufferTexture {
    context: WgpuContextHandle,
    texture: Arc<wgpu::Texture>,
}

impl From<core_model::SoftwareFrame<'_>> for FramebufferView {
    fn from(fb: core_model::SoftwareFrame<'_>) -> Self {
        let pixels = if fb.is_valid() {
            fb.bytes().to_vec()
        } else {
            Vec::new()
        };

        Self {
            width: fb.width,
            height: fb.height,
            state: Rc::new(RefCell::new(FramebufferState {
                pixels,
                dirty: true,
                gpu: None,
            })),
        }
    }
}

impl FramebufferView {
    pub fn update(&mut self, fb: core_model::SoftwareFrame<'_>) -> bool {
        if self.width != fb.width || self.height != fb.height || !fb.is_valid() {
            return false;
        }

        let mut state = self.state.borrow_mut();
        let pixels = fb.bytes();
        if state.pixels != pixels {
            state.pixels.clear();
            state.pixels.extend_from_slice(pixels);
            state.dirty = true;
        }

        true
    }

    fn upload(&self, window: &Window) -> Option<SurfaceSource> {
        let mut state = self.state.borrow_mut();

        let Some(context) = WgpuContextHandle::from_window(window).filter(|c| !c.device_lost())
        else {
            state.gpu = None;
            return None;
        };

        if state.pixels.is_empty() {
            return None;
        }

        let FramebufferState { pixels, dirty, gpu } = &mut *state;
        let gpu = match gpu {
            Some(gpu) if gpu.context.is_same_device(&context) => gpu,
            slot => {
                *dirty = true;
                slot.insert(self.create_texture(&context))
            }
        };

        if std::mem::take(dirty) {
            context.queue().write_texture(
                gpu.texture.as_image_copy(),
                pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.width * 4),
                    rows_per_image: Some(self.height),
                },
                gpu.texture.size(),
            );
        }

        Some(SurfaceSource::Texture {
            texture: gpu.texture.clone(),
            size: size(
                DevicePixels(self.width as i32),
                DevicePixels(self.height as i32),
            ),
        })
    }

    fn create_texture(&self, context: &WgpuContextHandle) -> FramebufferTexture {
        let texture = context.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("sly-framebuffer"),
            size: wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        FramebufferTexture {
            context: context.clone(),
            texture: Arc::new(texture),
        }
    }
}

#[derive(IntoElement)]
pub struct FramebufferWidget {
    view: Option<FramebufferView>,
    collapsed: bool,
    on_toggle: Option<ClickHandler>,
}

impl FramebufferWidget {
    pub const ID: &'static str = debugger_model::panel::FRAMEBUFFER;

    pub fn new(view: Option<FramebufferView>) -> Self {
        Self {
            view,
            collapsed: false,
            on_toggle: None,
        }
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
}

impl RenderOnce for FramebufferWidget {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::global(cx);
        let t = &*theme;

        let body = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .items_center()
            .justify_center()
            .overflow_hidden();
        let body = match self.view.as_ref().and_then(|view| view.upload(window)) {
            Some(texture) => body.child(
                surface(texture)
                    .size_full()
                    .object_fit(ObjectFit::Contain)
                    .image_filtering(ImageFiltering::Nearest),
            ),
            None => body.child(empty_state(t, "No frame")),
        };

        let mut panel = Panel::new(Self::ID, "Screen")
            .glyph(icon::MONITOR)
            .grow(true)
            .collapsed(self.collapsed)
            .on_toggle_opt(self.on_toggle);
        if let Some(view) = &self.view {
            panel = panel.meta(format!("{}×{}", view.width, view.height));
        }

        panel.child(body)
    }
}
