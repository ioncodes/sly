use anyhow::Result;
use bytemuck::{Pod, Zeroable};
use glyphon::{
    Attrs, Buffer, Cache, Color, ContentType, CustomGlyph, Family, FontSystem, Metrics,
    RasterizedCustomGlyph, Resolution, Shaping, SwashCache, TextArea, TextAtlas, TextBounds,
    TextRenderer, Viewport, Weight, Wrap,
};
use wgpu::util::DeviceExt;

use crate::color::srgb8_to_linear;
use crate::icons::StatusIcon;
use crate::prompts::PromptIcon;

const MAX_RASTER_BYTES: usize = 16 * 1024 * 1024;

pub fn valid_scale(scale: f32) -> f32 {
    if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FontFace {
    #[default]
    Sans,
    Semibold,
    Mono,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug)]
pub struct UiText {
    pub position: [f32; 2],
    pub size: f32,
    pub color: [u8; 4],
    pub text: String,
    pub max_width: f32,
    pub max_height: f32,
    pub line_height: f32,
    pub font: FontFace,
    pub align: TextAlign,
    pub vertically_centered: bool,
    /// Place below an earlier text, with a gap in logical pixels.
    pub after_text: Option<(usize, f32)>,
    /// Cut off at this y; only whole lines are drawn.
    pub clip_bottom: Option<f32>,
}

#[derive(Clone, Copy, Debug)]
pub struct UiRect {
    pub rect: [f32; 4],
    pub color: [u8; 4],
    pub radius: f32,
    /// Shrink to the bottom of these texts (half-open range).
    pub text_range: Option<[usize; 2]>,
    pub bottom_padding: f32,
    /// Vertically center the plate and its texts on this y.
    pub vertical_center: Option<f32>,
}

#[derive(Clone, Copy)]
pub struct UiMark([usize; 3]);

pub struct UiFragment {
    start: UiMark,
    end: UiMark,
}

struct BottomRightRow {
    fragments: Vec<UiFragment>,
    right: f32,
    bottom: f32,
    gap: f32,
}

/// UI for one frame, in logical pixels. Rectangles draw below text.
pub struct UiFrame {
    pub rects: Vec<UiRect>,
    pub texts: Vec<UiText>,
    icons: Vec<CustomGlyph>,
    bottom_right_rows: Vec<BottomRightRow>,
    pub logical_size: [f32; 2],
    pub scale_factor: f32,
    pub text_scale: f32,
}

impl Default for UiFrame {
    fn default() -> Self {
        Self::new([1, 1], 1.0)
    }
}

impl UiFrame {
    pub fn new(size: [u32; 2], scale: f32) -> Self {
        let scale_factor = valid_scale(scale);

        Self {
            rects: Vec::new(),
            texts: Vec::new(),
            icons: Vec::new(),
            bottom_right_rows: Vec::new(),
            logical_size: [size[0] as f32 / scale_factor, size[1] as f32 / scale_factor],
            scale_factor,
            text_scale: 1.0,
        }
    }

    pub fn mark(&self) -> UiMark {
        UiMark([self.rects.len(), self.texts.len(), self.icons.len()])
    }

    pub fn fragment(&self, start: UiMark) -> UiFragment {
        UiFragment {
            start,
            end: self.mark(),
        }
    }

    pub fn bottom_right_row(
        &mut self,
        fragments: Vec<UiFragment>,
        right: f32,
        bottom: f32,
        gap: f32,
    ) {
        self.bottom_right_rows.push(BottomRightRow {
            fragments,
            right,
            bottom,
            gap,
        });
    }

    pub fn rect(&mut self, rect: [f32; 4], color: [u8; 4], radius: f32) {
        self.rects.push(UiRect {
            rect,
            color,
            radius,
            text_range: None,
            bottom_padding: 0.0,
            vertical_center: None,
        });
    }

    pub fn text_backplate(
        &mut self,
        rect: [f32; 4],
        color: [u8; 4],
        radius: f32,
        text_range: [usize; 2],
        padding: f32,
    ) -> &mut UiRect {
        self.rects.push(UiRect {
            rect,
            color,
            radius,
            text_range: Some(text_range),
            bottom_padding: padding,
            vertical_center: None,
        });

        self.rects
            .last_mut()
            .expect("the backplate was just appended")
    }

    pub fn icon(&mut self, icon: PromptIcon, x: f32, y: f32, size: f32) {
        let [left, top, right, bottom] = icon.raster().bounds;
        // Size the icon in whole physical pixels, as glyphon rasterizes it.
        let scale = self.scale_factor;
        let width = ((right - left) * size * scale).ceil() / scale;
        let height = ((bottom - top) * size * scale).ceil() / scale;

        self.icons.push(CustomGlyph {
            id: icon.id(),
            left: ((x + (size - width) * 0.5) * scale).round() / scale,
            top: ((y + (size - height) * 0.5) * scale).round() / scale,
            width,
            height,
            color: None,
            snap_to_physical_pixel: true,
            metadata: 0,
        });
    }

    pub fn status_icon(&mut self, icon: StatusIcon, x: f32, y: f32, size: f32, color: [u8; 4]) {
        let scale = self.scale_factor;
        let side = (size * scale).round() / scale;
        let [r, g, b, a] = color;

        self.icons.push(CustomGlyph {
            id: icon.id(),
            left: (x * scale).round() / scale,
            top: (y * scale).round() / scale,
            width: side,
            height: side,
            color: Some(Color::rgba(r, g, b, a)),
            snap_to_physical_pixel: true,
            metadata: 0,
        });
    }

    pub fn text(
        &mut self,
        x: f32,
        y: f32,
        max_width: f32,
        size: f32,
        text: impl Into<String>,
        color: [u8; 4],
    ) -> &mut UiText {
        let size = size * self.text_scale;

        self.texts.push(UiText {
            position: [x, y],
            size,
            color,
            text: text.into(),
            max_width,
            max_height: size * 1.3,
            line_height: size * 1.3,
            font: FontFace::Sans,
            align: TextAlign::Left,
            vertically_centered: false,
            after_text: None,
            clip_bottom: None,
        });

        self.texts
            .last_mut()
            .expect("the text command was just appended")
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct RectInstance {
    rect: [f32; 4],
    color: [f32; 4],
    parameters: [f32; 4],
}

struct TextKey {
    text: String,
    size: f32,
    width: f32,
    height: f32,
    line_height: f32,
    font: FontFace,
    align: TextAlign,
}

impl TextKey {
    fn matches(&self, text: &UiText) -> bool {
        self.text == text.text
            && self.size == text.size
            && self.width == text.max_width
            && self.height == text.max_height
            && self.line_height == text.line_height
            && self.font == text.font
            && self.align == text.align
    }
}

struct CachedText {
    key: TextKey,
    buffer: Buffer,
    height: f32,
    ink: Option<(f32, Option<[f32; 4]>)>,
}

impl CachedText {
    fn new(fonts: &mut FontSystem, text: &UiText) -> Self {
        let metrics = Metrics::new(text.size.max(1.0), text.line_height.max(1.0));
        let mut buffer = Buffer::new(fonts, metrics);

        buffer.set_size(
            Some(text.max_width.max(1.0)),
            Some(text.max_height.max(1.0)),
        );
        buffer.set_wrap(Wrap::WordOrGlyph);

        let attrs = Attrs::new()
            .family(Family::Name(if text.font == FontFace::Mono {
                "IBM Plex Mono"
            } else {
                "IBM Plex Sans"
            }))
            .weight(if text.font == FontFace::Semibold {
                Weight::SEMIBOLD
            } else {
                Weight::NORMAL
            });

        let alignment = match text.align {
            TextAlign::Left => glyphon::cosmic_text::Align::Left,
            TextAlign::Center => glyphon::cosmic_text::Align::Center,
            TextAlign::Right => glyphon::cosmic_text::Align::Right,
        };

        buffer.set_text(&text.text, &attrs, Shaping::Advanced, Some(alignment));

        buffer.shape_until_scroll(fonts, false);

        let height = buffer
            .layout_runs()
            .last()
            .map_or(0.0, |run| run.line_top + run.line_height)
            .min(text.max_height);

        Self {
            key: TextKey {
                text: text.text.clone(),
                size: text.size,
                width: text.max_width,
                height: text.max_height,
                line_height: text.line_height,
                font: text.font,
                align: text.align,
            },
            buffer,
            height,
            ink: None,
        }
    }

    fn ink_bounds(
        &mut self,
        fonts: &mut FontSystem,
        swash: &mut SwashCache,
        scale: f32,
    ) -> Option<[f32; 4]> {
        if let Some((_, bounds)) = self.ink.filter(|(cached_scale, _)| *cached_scale == scale) {
            return bounds;
        }

        let mut bounds = None;

        for run in self.buffer.layout_runs() {
            for glyph in run.glyphs {
                let physical = glyph.physical((0.0, 0.0), scale);

                let Some(image) = swash.get_image(fonts, physical.cache_key) else {
                    continue;
                };

                let placement = image.placement;

                if placement.width == 0 || placement.height == 0 {
                    continue;
                }

                let x = (physical.x + placement.left) as f32 / scale;
                let y = ((run.line_y * scale).round() as i32 + physical.y - placement.top) as f32
                    / scale;

                include_bounds(
                    &mut bounds,
                    [
                        x,
                        y,
                        x + placement.width as f32 / scale,
                        y + placement.height as f32 / scale,
                    ],
                );
            }
        }

        self.ink = Some((scale, bounds));
        bounds
    }
}

pub struct UiRenderer {
    fonts: FontSystem,
    swash: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    text_renderer: TextRenderer,
    text_cache: Vec<CachedText>,
    previous_texts: Vec<Option<CachedText>>,
    raster_entries: usize,
    text_positions: Vec<[f32; 3]>,
    rect_positions: Vec<[f32; 4]>,
    icons: Vec<CustomGlyph>,
    icon_buffer: Buffer,
    rect_pipeline: wgpu::RenderPipeline,
    screen: wgpu::Buffer,
    screen_group: wgpu::BindGroup,
    rect_buffer: wgpu::Buffer,
    rect_capacity: usize,
    rect_count: u32,
    instances: Vec<RectInstance>,
}

impl UiRenderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let (mut fonts, warnings) = crate::fonts::configure();

        for warning in warnings {
            tracing::warn!("{warning}");
        }

        let cache = Cache::new(device);
        let viewport = Viewport::new(device, &cache);
        let mut atlas = TextAtlas::new(device, queue, &cache, format);
        let text_renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);

        let screen = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("UI viewport"),
            contents: bytemuck::cast_slice(&[1.0_f32, 1.0, 1.0, 0.0]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("UI viewport layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let screen_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("UI viewport group"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: screen.as_entire_binding(),
            }],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("UI rectangles"),
            source: wgpu::ShaderSource::Wgsl(include_str!("ui.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("UI rectangle layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let rect_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("UI rectangles"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<RectInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![0=>Float32x4,1=>Float32x4,2=>Float32x4],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });

        let rect_capacity = 32;
        let rect_buffer = rect_buffer(device, rect_capacity);
        let icon_buffer = Buffer::new(&mut fonts, Metrics::new(16.0, 20.0));

        Self {
            fonts,
            swash: SwashCache::new(),
            viewport,
            atlas,
            text_renderer,
            text_cache: Vec::new(),
            previous_texts: Vec::new(),
            raster_entries: 0,
            text_positions: Vec::new(),
            rect_positions: Vec::new(),
            icons: Vec::new(),
            icon_buffer,
            rect_pipeline,
            screen,
            screen_group,
            rect_buffer,
            rect_capacity,
            rect_count: 0,
            instances: Vec::new(),
        }
    }

    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &UiFrame,
        size: [u32; 2],
        scale_factor: f32,
    ) -> Result<()> {
        let scale = scale_factor.max(0.01);

        self.viewport.update(
            queue,
            Resolution {
                width: size[0],
                height: size[1],
            },
        );

        queue.write_buffer(
            &self.screen,
            0,
            bytemuck::cast_slice(&[size[0] as f32, size[1] as f32, scale, 0.0]),
        );

        // Reuse shaped buffers by content, not position.
        self.previous_texts.clear();
        self.previous_texts
            .extend(self.text_cache.drain(..).map(Some));

        for (index, text) in frame.texts.iter().enumerate() {
            let matches = |cached: &Option<CachedText>| {
                cached
                    .as_ref()
                    .is_some_and(|cached| cached.key.matches(text))
            };

            let reused = match self.previous_texts.get_mut(index) {
                Some(cached) if matches(cached) => cached.take(),
                _ => self
                    .previous_texts
                    .iter_mut()
                    .find(|cached| matches(cached))
                    .and_then(Option::take),
            };

            self.text_cache
                .push(reused.unwrap_or_else(|| CachedText::new(&mut self.fonts, text)));
        }

        self.text_positions.clear();

        for (index, text) in frame.texts.iter().enumerate() {
            let [x, mut y] = text.position;

            if let Some((previous, gap)) = text.after_text.filter(|(previous, _)| *previous < index)
            {
                y = self.text_positions[previous][2] + self.text_cache[previous].height + gap;
            }

            let draw_y = y + if text.vertically_centered {
                (text.max_height - self.text_cache[index].height).max(0.0) * 0.5
            } else {
                0.0
            };

            self.text_positions.push([x, y, draw_y]);
        }

        self.rect_positions.clear();
        self.rect_positions
            .extend(frame.rects.iter().map(|rect| rect.rect));
        self.icons.clear();
        self.icons.extend_from_slice(&frame.icons);
        self.layout_bottom_right_rows(frame, scale);
        self.instances.clear();

        for (rect, &position) in frame.rects.iter().zip(&self.rect_positions) {
            if rect.rect[2] <= 0.0 || rect.rect[3] <= 0.0 || rect.color[3] == 0 {
                continue;
            }

            let mut bounds = position;

            if let Some([start, end]) = rect.text_range {
                let bottom = (start..end.min(frame.texts.len()))
                    .filter_map(|index| {
                        let text = &frame.texts[index];
                        let [_, y, draw_y] = self.text_positions[index];
                        visible_text_bottom(text, y)
                            .map(|limit| (draw_y + self.text_cache[index].height).min(limit))
                    })
                    .reduce(f32::max);

                let Some(bottom) = bottom else {
                    continue;
                };

                bounds[3] = (bottom + rect.bottom_padding - bounds[1]).clamp(0.0, bounds[3]);

                if let Some(center) = rect.vertical_center {
                    let y = (center - bounds[3] * 0.5)
                        .clamp(rect.rect[1], rect.rect[1] + rect.rect[3] - bounds[3]);
                    let offset = y - bounds[1];

                    for position in &mut self.text_positions[start..end.min(frame.texts.len())] {
                        position[1] += offset;
                        position[2] += offset;
                    }

                    bounds[1] = y;
                }
            }

            let [red, green, blue, alpha] = rect.color;

            self.instances.push(RectInstance {
                rect: bounds,
                color: [
                    srgb8_to_linear(red),
                    srgb8_to_linear(green),
                    srgb8_to_linear(blue),
                    f32::from(alpha) / 255.0,
                ],
                parameters: [
                    rect.radius.max(0.0).min(bounds[2].min(bounds[3]) * 0.5),
                    0.0,
                    0.0,
                    0.0,
                ],
            });
        }

        if self.instances.len() > self.rect_capacity {
            self.rect_capacity = self.instances.len().next_power_of_two();
            self.rect_buffer = rect_buffer(device, self.rect_capacity);
        }

        self.rect_count = self.instances.len() as u32;

        if !self.instances.is_empty() {
            queue.write_buffer(&self.rect_buffer, 0, bytemuck::cast_slice(&self.instances));
        }

        let areas = self
            .text_cache
            .iter()
            .zip(&frame.texts)
            .zip(&self.text_positions)
            .filter_map(|((cached, text), &[x, y, draw_y])| {
                let [r, g, b, a] = text.color;
                let bottom = visible_text_bottom(text, y)?;

                Some(TextArea {
                    buffer: &cached.buffer,
                    left: x * scale,
                    top: draw_y * scale,
                    scale,
                    bounds: TextBounds {
                        left: (x * scale).floor().max(0.0) as i32,
                        top: (y * scale).floor().max(0.0) as i32,
                        right: ((x + text.max_width) * scale).ceil().min(size[0] as f32) as i32,
                        bottom: (bottom * scale).ceil().min(size[1] as f32) as i32,
                    },
                    default_color: Color::rgba(r, g, b, a),
                    custom_glyphs: &[],
                })
            });

        let icons = TextArea {
            buffer: &self.icon_buffer,
            left: 0.0,
            top: 0.0,
            scale,
            bounds: TextBounds {
                left: 0,
                top: 0,
                right: size[0] as i32,
                bottom: size[1] as i32,
            },
            default_color: Color::rgb(255, 255, 255),
            custom_glyphs: &self.icons,
        };

        self.text_renderer.prepare_with_custom(
            device,
            queue,
            &mut self.fonts,
            &mut self.atlas,
            &self.viewport,
            areas.chain(std::iter::once(icons)),
            &mut self.swash,
            |request| {
                if let Some(icon) = StatusIcon::from_id(request.id) {
                    return Some(RasterizedCustomGlyph {
                        data: icon.rasterize(request.width, request.height)?,
                        content_type: ContentType::Mask,
                    });
                }

                let source = &PromptIcon::from_id(request.id)?.raster().image;

                let raster = image::imageops::resize(
                    source,
                    u32::from(request.width),
                    u32::from(request.height),
                    image::imageops::FilterType::Lanczos3,
                );

                Some(RasterizedCustomGlyph {
                    data: raster.into_raw(),
                    content_type: ContentType::Color,
                })
            },
        )?;

        // Bound swash's CPU raster cache; recount only when it grows.
        if self.swash.image_cache.len() != self.raster_entries {
            let raster_bytes: usize = self
                .swash
                .image_cache
                .values()
                .flatten()
                .map(|image| image.data.len())
                .sum();

            if raster_bytes > MAX_RASTER_BYTES {
                self.swash.image_cache.clear();
            }

            self.raster_entries = self.swash.image_cache.len();
        }

        Ok(())
    }

    fn layout_bottom_right_rows(&mut self, frame: &UiFrame, scale: f32) {
        for row in &frame.bottom_right_rows {
            let measured: Vec<_> = row
                .fragments
                .iter()
                .map(|fragment| {
                    let UiMark([r0, t0, i0]) = fragment.start;
                    let UiMark([r1, t1, i1]) = fragment.end;
                    let mut bounds = None;

                    for (rect, &[x, y, width, height]) in
                        frame.rects[r0..r1].iter().zip(&self.rect_positions[r0..r1])
                    {
                        if rect.color[3] > 0 && width > 0.0 && height > 0.0 {
                            include_bounds(&mut bounds, [x, y, x + width, y + height]);
                        }
                    }

                    for index in t0..t1 {
                        if frame.texts[index].color[3] == 0 {
                            continue;
                        }

                        if let Some([left, top, right, bottom]) = self.text_cache[index].ink_bounds(
                            &mut self.fonts,
                            &mut self.swash,
                            scale,
                        ) {
                            let [x, _, y] = self.text_positions[index];
                            include_bounds(&mut bounds, [x + left, y + top, x + right, y + bottom]);
                        }
                    }

                    for icon in &self.icons[i0..i1] {
                        include_bounds(
                            &mut bounds,
                            [
                                icon.left,
                                icon.top,
                                icon.left + icon.width,
                                icon.top + icon.height,
                            ],
                        );
                    }

                    bounds
                })
                .collect();

            let visible = measured.iter().flatten().count();

            let width = measured
                .iter()
                .flatten()
                .map(|[left, _, right, _]| right - left)
                .sum::<f32>()
                + visible.saturating_sub(1) as f32 * row.gap;

            let bottom = measured
                .iter()
                .flatten()
                .map(|bounds| bounds[3])
                .reduce(f32::max)
                .unwrap_or(0.0);

            let dy = row.bottom - bottom;
            let mut x = row.right - width;

            for (fragment, bounds) in row.fragments.iter().zip(measured) {
                let Some([left, _, right, _]) = bounds else {
                    continue;
                };

                let dx = x - left;
                let UiMark([r0, t0, i0]) = fragment.start;
                let UiMark([r1, t1, i1]) = fragment.end;

                for rect in &mut self.rect_positions[r0..r1] {
                    rect[0] += dx;
                    rect[1] += dy;
                }

                for position in &mut self.text_positions[t0..t1] {
                    position[0] += dx;
                    position[1] += dy;
                    position[2] += dy;
                }

                for icon in &mut self.icons[i0..i1] {
                    icon.left += dx;
                    icon.top += dy;
                }

                x += right - left + row.gap;
            }
        }
    }

    pub fn render(&self, pass: &mut wgpu::RenderPass<'_>) -> Result<()> {
        if self.rect_count > 0 {
            pass.set_pipeline(&self.rect_pipeline);
            pass.set_bind_group(0, &self.screen_group, &[]);
            pass.set_vertex_buffer(0, self.rect_buffer.slice(..));
            pass.draw(0..6, 0..self.rect_count);
        }

        self.text_renderer
            .render(&self.atlas, &self.viewport, pass)?;
        Ok(())
    }

    pub fn end_frame(&mut self) {
        self.atlas.trim();
    }
}

fn include_bounds(bounds: &mut Option<[f32; 4]>, [left, top, right, bottom]: [f32; 4]) {
    *bounds = Some(match *bounds {
        Some([x0, y0, x1, y1]) => [x0.min(left), y0.min(top), x1.max(right), y1.max(bottom)],
        None => [left, top, right, bottom],
    });
}

fn rect_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("UI rectangle instances"),
        size: (capacity * std::mem::size_of::<RectInstance>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn visible_text_bottom(text: &UiText, y: f32) -> Option<f32> {
    let bottom = y + text.max_height;

    if let Some(limit) = text.clip_bottom {
        let lines = ((limit.min(bottom) - y + 0.001) / text.line_height)
            .floor()
            .max(0.0);

        (lines >= 1.0).then_some(y + lines * text.line_height)
    } else {
        Some(bottom)
    }
}
