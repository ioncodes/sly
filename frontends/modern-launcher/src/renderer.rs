use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result, anyhow};
use bytemuck::{Pod, Zeroable};
use glam::Mat4;
use wgpu::util::DeviceExt;
use winit::window::Window;

use crate::artwork_cache::{ArtworkKey, CoverAccent, DecodedArtwork, DecodedImage};
use crate::models::{self, MaterialSlot, Model, ModelKind, Vertex};
use crate::ui::{UiFrame, UiRenderer};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const MAX_DRAWS: usize = 384;
const MAX_ARTWORK_RECORDS: usize = 128;
const OBJECT_STRIDE: usize = 256;

macro_rules! fullscreen_shader {
    ($path:literal) => {
        concat!(
            include_str!("../shaders/fullscreen.wgsl"),
            include_str!($path)
        )
    };
}

#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
struct Globals {
    view_projection: [[f32; 4]; 4],
    viewport: [f32; 4],
    effects: [f32; 4],
    theme_tint: [f32; 4],
}

#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
struct ObjectUniform {
    model: [[f32; 4]; 4],
    tint: [f32; 4],
    material: [f32; 4],
    label: [f32; 4],
    artwork_crop: [f32; 4],
    top_crop: [f32; 4],
    artwork_params: [f32; 4],
    edge_bounds_min: [f32; 4],
    edge_bounds_size: [f32; 4],
}

#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
struct IrisUniform {
    size_center: [f32; 4],
    radius_fade: [f32; 4],
}

pub struct DrawObject {
    pub kind: ModelKind,
    pub system: core_model::System,
    pub market: models::ReleaseMarket,
    pub shell_finish: Option<crate::materials::ShellFinish>,
    pub transform: Mat4,
    pub artwork: Option<ArtworkKey>,
    pub tint: [f32; 4],
    /// Draw checker placeholders instead of any scans.
    pub placeholder: bool,
    /// Normalized viewport scissor: x, y, width, height.
    pub clip: Option<[f32; 4]>,
}

pub struct GamePresentation {
    pub rect: [f32; 4],
    pub opacity: f32,
}

pub struct Scene {
    pub view_projection: Mat4,
    pub objects: Vec<DrawObject>,
    pub contours: bool,
    pub background_phase: f32,
    /// Linear RGB accent from the selected cover.
    pub theme_tint: [f32; 3],
    pub lighting: bool,
    pub visible: bool,
    pub game: Option<GamePresentation>,
    /// Opening radius as a fraction of the distance to the farthest corner.
    pub iris_radius: f32,
    pub fade: f32,
    /// Projected label center in normalized viewport coordinates, +Y down.
    pub iris_center: [f32; 2],
}

impl Default for Scene {
    fn default() -> Self {
        Self {
            view_projection: Mat4::IDENTITY,
            objects: Vec::new(),
            contours: true,
            background_phase: 0.0,
            theme_tint: CoverAccent::default().linear,
            lighting: true,
            visible: true,
            game: None,
            iris_radius: 1.1,
            fade: 0.0,
            iris_center: [0.5, 0.5],
        }
    }
}

struct Mesh {
    vertex: wgpu::Buffer,
    index: wgpu::Buffer,
    count: u32,
    slot: MaterialSlot,
    color: [f32; 4],
    roughness: f32,
    bounds: models::Bounds,
}

impl Mesh {
    fn upload(device: &wgpu::Device, p: models::Primitive, bounds: models::Bounds) -> Self {
        Mesh {
            vertex: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("shared mesh vertices"),
                contents: bytemuck::cast_slice(&p.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            index: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("shared mesh indices"),
                contents: bytemuck::cast_slice(&p.indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
            count: p.indices.len() as u32,
            slot: p.slot,
            color: p.color,
            roughness: p.roughness,
            bounds,
        }
    }
}

struct Texture {
    texture: wgpu::Texture,
    binding: wgpu::BindGroup,
    bytes: usize,
}

struct ArtSet {
    roles: HashMap<MaterialSlot, Texture>,
    label_top: Option<Texture>,
    border_average: Option<[f32; 3]>,
    used: u64,
    bytes: usize,
}

pub struct Renderer {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    ui: UiRenderer,
    surface: Option<wgpu::Surface<'static>>,
    window: Option<Arc<Window>>,
    needs_present: bool,
    acquisition_attempt: u64,
    last_presented_attempt: u64,
    last_present_failure: Option<&'static str>,
    last_acquire_ms: f64,
    surface_config: Option<wgpu::SurfaceConfiguration>,
    // Rendered offscreen first, then copied to the surface by the present pass.
    target: wgpu::Texture,
    multisampled: Option<wgpu::TextureView>,
    sample_count: u32,
    depth: wgpu::TextureView,
    size: [u32; 2],
    globals: wgpu::Buffer,
    globals_binding: wgpu::BindGroup,
    objects: wgpu::Buffer,
    object_binding: wgpu::BindGroup,
    object_staging: Vec<u8>,
    iris: wgpu::Buffer,
    iris_binding: wgpu::BindGroup,
    game_uniform: wgpu::Buffer,
    game_binding: wgpu::BindGroup,
    game_pipeline: wgpu::RenderPipeline,
    mesh_pipeline: wgpu::RenderPipeline,
    background_pipeline: wgpu::RenderPipeline,
    iris_pipeline: wgpu::RenderPipeline,
    present_pipeline: Option<wgpu::RenderPipeline>,
    present_binding: wgpu::BindGroup,
    texture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    artwork_sampler: wgpu::Sampler,
    game_sampler: wgpu::Sampler,
    neutral: Texture,
    models: HashMap<ModelKind, Vec<Mesh>>,
    profiles: models::Profiles,
    artwork: HashMap<ArtworkKey, ArtSet>,
    art_budget: usize,
    art_bytes: usize,
    frame_number: u64,
    pub last_draws: usize,
    pub last_upload_bytes: usize,
    game_texture: Option<(Texture, [u32; 2])>,
}

impl Renderer {
    pub async fn new(
        window: Option<Arc<Window>>,
        size: [u32; 2],
        art_budget: usize,
    ) -> Result<Self> {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = wgpu::Backends::VULKAN | wgpu::Backends::METAL | wgpu::Backends::DX12;
        let instance = wgpu::Instance::new(desc);

        let surface = window
            .as_ref()
            .map(|window| instance.create_surface(window.clone()))
            .transpose()?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: surface.as_ref(),
                power_preference: wgpu::PowerPreference::LowPower,
                ..Default::default()
            })
            .await
            .context("No Vulkan, Metal, or DX12 adapter is available")?;

        let adapter_info = adapter.get_info();

        let artwork_anisotropy = if adapter
            .get_downlevel_capabilities()
            .flags
            .contains(wgpu::DownlevelFlags::ANISOTROPIC_FILTERING)
        {
            16
        } else {
            1
        };

        tracing::info!(
            ?adapter_info,
            limits = ?adapter.limits(),
            artwork_anisotropy,
            "graphics adapter"
        );

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("Modern Launcher device"),
                required_limits: wgpu::Limits::downlevel_defaults()
                    .using_resolution(adapter.limits()),
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                ..Default::default()
            })
            .await?;

        let surface_config = surface
            .as_ref()
            .map(|surface| {
                let capabilities = surface.get_capabilities(&adapter);

                tracing::info!(
                    backend = ?adapter_info.backend,
                    present_modes = ?capabilities.present_modes,
                    formats = ?capabilities.formats,
                    alpha_modes = ?capabilities.alpha_modes,
                    "surface capabilities"
                );

                let mut config = surface
                    .get_default_config(&adapter, size[0], size[1])
                    .ok_or_else(|| anyhow!("Surface has no supported presentation format"))?;

                config.present_mode = preferred_present_mode(&capabilities.present_modes);

                if let Ok(mode) = std::env::var("SLY_PRESENT_MODE") {
                    let requested = match mode.as_str() {
                        "mailbox" => Some(wgpu::PresentMode::Mailbox),
                        "fifo" => Some(wgpu::PresentMode::Fifo),
                        "immediate" => Some(wgpu::PresentMode::Immediate),
                        _ => None,
                    };

                    match requested.filter(|mode| capabilities.present_modes.contains(mode)) {
                        Some(mode) => config.present_mode = mode,
                        None => tracing::warn!(
                            %mode,
                            "Unsupported diagnostic present mode; retaining supported default"
                        ),
                    }
                }

                tracing::info!(present_mode = ?config.present_mode, "surface presentation");
                config.desired_maximum_frame_latency = 2;
                surface.configure(&device, &config);
                Ok::<_, anyhow::Error>(config)
            })
            .transpose()?;

        let uniform_layout = |label, dynamic| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(label),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: dynamic,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            })
        };

        let globals_layout = uniform_layout("scene globals", false);
        let objects_layout = uniform_layout("object uniforms", true);
        let iris_layout = uniform_layout("iris uniform", false);
        let game_layout = uniform_layout("game opacity", false);

        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sRGB art"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let buffer = |label, size| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };

        let game_uniform = buffer("game opacity", 16);
        let globals = buffer("scene", std::mem::size_of::<Globals>() as u64);
        let objects = buffer("visible objects", (MAX_DRAWS * OBJECT_STRIDE) as u64);
        let iris = buffer("iris", std::mem::size_of::<IrisUniform>() as u64);

        let binding = |label, layout, buffer: &wgpu::Buffer, size| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer,
                        offset: 0,
                        size,
                    }),
                }],
            })
        };

        let globals_binding = binding("scene", &globals_layout, &globals, None);

        let object_binding = binding(
            "object",
            &objects_layout,
            &objects,
            wgpu::BufferSize::new(std::mem::size_of::<ObjectUniform>() as u64),
        );

        let iris_binding = binding("iris", &iris_layout, &iris, None);
        let game_binding = binding("game opacity", &game_layout, &game_uniform, None);

        let shader = |label, source: &'static str| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            })
        };

        let scene_shader = shader("lit models", include_str!("../shaders/scene.wgsl"));

        let bg_shader = shader(
            "static contours",
            fullscreen_shader!("../shaders/background.wgsl"),
        );

        let iris_shader = shader("circular iris", fullscreen_shader!("../shaders/iris.wgsl"));

        let sample_count = if [FORMAT, wgpu::TextureFormat::Depth24Plus]
            .into_iter()
            .all(|format| {
                adapter
                    .get_texture_format_features(format)
                    .flags
                    .contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X4)
            }) {
            4
        } else {
            1
        };

        tracing::info!(sample_count, "3D scene antialiasing");

        let mesh_pipeline = pipeline(
            &device,
            "models",
            &scene_shader,
            &[
                &globals_layout,
                &objects_layout,
                &texture_layout,
                &texture_layout,
            ],
            true,
            PipelineTarget {
                format: FORMAT,
                blend: true,
                sample_count,
            },
        );

        let background_pipeline = pipeline(
            &device,
            "contours",
            &bg_shader,
            &[&globals_layout],
            false,
            PipelineTarget {
                format: FORMAT,
                blend: false,
                sample_count,
            },
        );

        let iris_pipeline = pipeline(
            &device,
            "iris",
            &iris_shader,
            &[&iris_layout],
            false,
            PipelineTarget {
                format: FORMAT,
                blend: true,
                sample_count: 1,
            },
        );

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("art with mipmaps"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        let artwork_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("printed art edges"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            // Covers are seen almost edge-on; anisotropic filtering keeps them sharp.
            anisotropy_clamp: artwork_anisotropy,
            ..Default::default()
        });

        let game_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("framebuffer nearest pixels"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let game_shader = shader(
            "core framebuffer",
            fullscreen_shader!("../shaders/game.wgsl"),
        );

        let game_pipeline = pipeline(
            &device,
            "core framebuffer",
            &game_shader,
            &[&texture_layout, &game_layout],
            false,
            PipelineTarget {
                format: FORMAT,
                blend: true,
                sample_count: 1,
            },
        );

        let target = target_texture(&device, size);
        let depth = depth_texture(&device, size, sample_count);
        let multisampled = multisampled_texture(&device, size, sample_count);

        let present_binding = texture_binding(
            &device,
            &texture_layout,
            &sampler,
            &target.create_view(&Default::default()),
        );

        let present_pipeline = surface_config.as_ref().map(|config| {
            let shader = shader("present", fullscreen_shader!("../shaders/present.wgsl"));

            pipeline(
                &device,
                "present",
                &shader,
                &[&texture_layout],
                false,
                PipelineTarget {
                    format: config.format,
                    blend: false,
                    sample_count: 1,
                },
            )
        });

        let neutral = upload_texture(
            &device,
            &queue,
            &texture_layout,
            &sampler,
            &solid_image([255, 255, 255, 255]),
        );

        let profiles = models::profiles()?;
        let mut models = HashMap::new();

        for kind in [
            ModelKind::WesternBox,
            ModelKind::JapaneseBoxProxy,
            ModelKind::GbaBox,
            ModelKind::GbaJapaneseBox,
            ModelKind::GbaCartridge,
            ModelKind::UsCartridge,
            ModelKind::JpPalCartridge,
        ] {
            let model = Model::load(kind)?;

            let meshes = model
                .primitives
                .into_iter()
                .map(|p| Mesh::upload(&device, p, model.bounds))
                .collect();

            models.insert(kind, meshes);
        }

        let ui = UiRenderer::new(&device, &queue, FORMAT);

        Ok(Self {
            device,
            queue,
            ui,
            surface,
            window,
            needs_present: false,
            acquisition_attempt: 0,
            last_presented_attempt: 0,
            last_present_failure: None,
            last_acquire_ms: 0.0,
            surface_config,
            target,
            multisampled,
            sample_count,
            depth,
            size,
            globals,
            globals_binding,
            objects,
            object_binding,
            object_staging: vec![0; MAX_DRAWS * OBJECT_STRIDE],
            iris,
            iris_binding,
            game_uniform,
            game_binding,
            game_pipeline,
            mesh_pipeline,
            background_pipeline,
            iris_pipeline,
            present_pipeline,
            present_binding,
            texture_layout,
            sampler,
            artwork_sampler,
            game_sampler,
            neutral,
            models,
            profiles,
            artwork: HashMap::new(),
            art_budget,
            art_bytes: 0,
            frame_number: 0,
            last_draws: 0,
            last_upload_bytes: 0,
            game_texture: None,
        })
    }

    pub fn needs_present(&self) -> bool {
        self.needs_present
    }

    pub fn size(&self) -> [u32; 2] {
        self.size
    }

    pub fn has_artwork(&self, key: &ArtworkKey) -> bool {
        self.artwork.contains_key(key)
    }

    pub fn set_art_budget(&mut self, bytes: usize) {
        self.art_budget = bytes;
        while self.art_bytes > self.art_budget && self.evict_oldest_artwork() {}
    }

    fn evict_oldest_artwork(&mut self) -> bool {
        let Some(oldest) = self
            .artwork
            .iter()
            .min_by_key(|(_, set)| set.used)
            .map(|(key, _)| key.clone())
        else {
            return false;
        };

        let removed = self
            .artwork
            .remove(&oldest)
            .expect("resident artwork exists");

        self.art_bytes -= removed.bytes;
        true
    }

    pub fn resize(&mut self, size: [u32; 2]) {
        if size.contains(&0) || size == self.size {
            return;
        }

        self.size = size;
        self.target = target_texture(&self.device, size);
        self.depth = depth_texture(&self.device, size, self.sample_count);
        self.multisampled = multisampled_texture(&self.device, size, self.sample_count);

        self.present_binding = texture_binding(
            &self.device,
            &self.texture_layout,
            &self.sampler,
            &self.target.create_view(&Default::default()),
        );

        if let (Some(surface), Some(config)) = (&self.surface, &mut self.surface_config) {
            config.width = size[0];
            config.height = size[1];
            surface.configure(&self.device, config);
        }
    }

    /// Returns false if the artwork exceeds the GPU budget.
    pub fn upload_artwork(&mut self, key: ArtworkKey, artwork: &DecodedArtwork) -> Result<bool> {
        if self.artwork.contains_key(&key) {
            return Ok(true);
        }

        let images = [
            (MaterialSlot::BoxFront, artwork.front.as_ref()),
            (MaterialSlot::BoxBack, artwork.back.as_ref()),
            (MaterialSlot::BoxSpine, artwork.spine.as_ref()),
            (MaterialSlot::CartridgeLabel, artwork.label.as_ref()),
        ];

        let limit = self.device.limits().max_texture_dimension_2d;
        let mut estimate = 0usize;

        for image in images
            .iter()
            .filter_map(|(_, image)| *image)
            .chain(artwork.label_top.as_ref())
        {
            estimate = estimate
                .checked_add(validate_image(image, limit)?)
                .context("artwork byte count overflow")?;
        }

        if estimate > self.art_budget {
            return Ok(false);
        }

        // Cap the record count too, since empty releases still take a slot.
        while self.art_bytes + estimate > self.art_budget
            || self.artwork.len() >= MAX_ARTWORK_RECORDS
        {
            if !self.evict_oldest_artwork() {
                break;
            }
        }

        let upload = |image: &DecodedImage| {
            upload_texture(
                &self.device,
                &self.queue,
                &self.texture_layout,
                &self.artwork_sampler,
                image,
            )
        };

        let roles: HashMap<_, _> = images
            .into_iter()
            .filter_map(|(slot, image)| image.map(|image| (slot, upload(image))))
            .collect();

        let label_top = artwork.label_top.as_ref().map(upload);
        let bytes = roles.values().map(|texture| texture.bytes).sum::<usize>()
            + label_top.as_ref().map_or(0, |texture| texture.bytes);
        self.art_bytes += bytes;
        self.last_upload_bytes = bytes;

        self.artwork.insert(
            key,
            ArtSet {
                roles,
                label_top,
                border_average: artwork
                    .front
                    .as_ref()
                    .or(artwork.back.as_ref())
                    .map(|image| image.border_average),
                used: self.frame_number,
                bytes,
            },
        );

        Ok(true)
    }

    pub fn update_game(&mut self, width: u32, height: u32, stride: u32, bytes: &[u8]) {
        if self
            .game_texture
            .as_ref()
            .is_none_or(|(_, size)| *size != [width, height])
        {
            let texture = texture_2d(
                &self.device,
                "borrowed core framebuffer upload",
                [width, height],
                FORMAT,
                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                1,
                1,
            );

            let binding = texture_binding(
                &self.device,
                &self.texture_layout,
                &self.game_sampler,
                &texture.create_view(&Default::default()),
            );

            self.game_texture = Some((
                Texture {
                    texture,
                    binding,
                    bytes: width as usize * height as usize * 4,
                },
                [width, height],
            ));
        }

        let (texture, _) = self
            .game_texture
            .as_ref()
            .expect("frame texture initialized");

        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
    }

    pub fn render(&mut self, scene: &Scene, frame: &UiFrame) -> Result<()> {
        let render_started = Instant::now();

        self.ui.prepare(
            &self.device,
            &self.queue,
            frame,
            self.size,
            frame.scale_factor,
        )?;

        tracing::trace!(
            elapsed_ms = render_started.elapsed().as_secs_f64() * 1000.0,
            "UI prepared"
        );

        let encode_started = Instant::now();
        self.frame_number += 1;

        let globals = Globals {
            view_projection: scene.view_projection.to_cols_array_2d(),
            viewport: [
                self.size[0] as f32,
                self.size[1] as f32,
                frame.scale_factor,
                0.0,
            ],
            effects: [
                f32::from(scene.contours),
                f32::from(scene.lighting),
                scene.background_phase.sin(),
                scene.background_phase.cos(),
            ],
            theme_tint: [
                scene.theme_tint[0],
                scene.theme_tint[1],
                scene.theme_tint[2],
                1.0,
            ],
        };

        self.queue
            .write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));

        let iris = IrisUniform {
            size_center: [
                self.size[0] as f32,
                self.size[1] as f32,
                scene.iris_center[0],
                scene.iris_center[1],
            ],
            radius_fade: [scene.iris_radius, scene.fade, 0.0, 0.0],
        };

        self.queue
            .write_buffer(&self.iris, 0, bytemuck::bytes_of(&iris));

        for object in &scene.objects {
            if let Some(key) = &object.artwork
                && let Some(set) = self.artwork.get_mut(key)
            {
                set.used = self.frame_number;
            }
        }

        let mut draws = Vec::with_capacity(128);

        if scene.visible {
            for object in &scene.objects {
                if let Some(meshes) = self.models.get(&object.kind) {
                    let profile = self
                        .profiles
                        .for_system_market(object.system, object.market);

                    let transform = object.transform * profile.model_to_display(object.kind);
                    let top_mapping = profile.artwork.label_top;
                    let fold_v = profile.label_fold_v;

                    let artwork = object
                        .artwork
                        .as_ref()
                        .filter(|_| !object.placeholder)
                        .and_then(|key| self.artwork.get(key));

                    for (mesh_index, mesh) in meshes.iter().enumerate() {
                        anyhow::ensure!(
                            draws.len() < MAX_DRAWS,
                            "Visible scene exceeds draw budget"
                        );

                        let texture = artwork.and_then(|set| set.roles.get(&mesh.slot));
                        let is_label = mesh.slot == MaterialSlot::CartridgeLabel;

                        let mut top = if is_label {
                            artwork.and_then(|set| set.label_top.as_ref())
                        } else {
                            None
                        };

                        let mut mapping = profile.artwork_mapping(mesh.slot);
                        let mut secondary_mapping = top_mapping;
                        let mut primary = texture;
                        // Missing edges and spines extend the front and back scans. A real spine
                        // scan wins.
                        let mut edge_mode = 0;

                        if texture.is_none()
                            && matches!(mesh.slot, MaterialSlot::BoxEdges | MaterialSlot::BoxSpine)
                        {
                            primary =
                                artwork.and_then(|set| set.roles.get(&MaterialSlot::BoxFront));
                            top = artwork.and_then(|set| set.roles.get(&MaterialSlot::BoxBack));
                            edge_mode = u8::from(primary.is_some()) + 2 * u8::from(top.is_some());
                            mapping = profile.artwork_mapping(MaterialSlot::BoxFront);
                            secondary_mapping = profile.artwork_mapping(MaterialSlot::BoxBack);
                        }

                        let mut tint = neutral_color(mesh.slot, mesh.color);

                        let finish = object
                            .shell_finish
                            .filter(|_| mesh.slot == MaterialSlot::Shell);

                        if let Some(finish) = finish {
                            tint[..3].copy_from_slice(&finish.color);
                        }

                        if texture.is_none()
                            && mesh.slot == MaterialSlot::BoxBack
                            && let Some(average) = artwork.and_then(|set| set.border_average)
                        {
                            let luminance = crate::color::luminance(average);

                            for (channel, sampled) in tint[..3].iter_mut().zip(average) {
                                *channel =
                                    (sampled * 0.8 + luminance * 0.2) * 0.92 + *channel * 0.08;
                            }
                        }

                        for (value, scale) in tint.iter_mut().zip(object.tint) {
                            *value *= scale;
                        }

                        let uniform = ObjectUniform {
                            model: transform.to_cols_array_2d(),
                            tint,
                            material: [
                                f32::from(texture.is_some()),
                                finish.map_or(mesh.roughness, |finish| finish.roughness),
                                f32::from(
                                    texture.is_none()
                                        && (matches!(
                                            mesh.slot,
                                            MaterialSlot::BoxFront
                                                | MaterialSlot::BoxBack
                                                | MaterialSlot::CartridgeLabel
                                        ) || (edge_mode == 0
                                            && matches!(
                                                mesh.slot,
                                                MaterialSlot::BoxSpine | MaterialSlot::BoxEdges
                                            ))),
                                ),
                                f32::from(edge_mode),
                            ],
                            label: [f32::from(is_label), fold_v, f32::from(top.is_some()), 0.0],
                            artwork_crop: mapping.crop,
                            top_crop: secondary_mapping.crop,
                            artwork_params: [
                                f32::from(mapping.quarter_turns_clockwise),
                                f32::from(secondary_mapping.quarter_turns_clockwise),
                                if finish.is_some()
                                    || (mesh.slot == MaterialSlot::Shell
                                        && object.system == core_model::System::Gba)
                                {
                                    2.0
                                } else {
                                    f32::from(mesh.slot == MaterialSlot::Shell)
                                },
                                f32::from(matches!(
                                    mesh.slot,
                                    MaterialSlot::BoxFront
                                        | MaterialSlot::BoxBack
                                        | MaterialSlot::BoxSpine
                                        | MaterialSlot::BoxEdges
                                )),
                            ],
                            edge_bounds_min: mesh.bounds.min.extend(0.0).to_array(),
                            edge_bounds_size: mesh.bounds.size().extend(0.0).to_array(),
                        };

                        let start = draws.len() * OBJECT_STRIDE;
                        self.object_staging[start..start + std::mem::size_of::<ObjectUniform>()]
                            .copy_from_slice(bytemuck::bytes_of(&uniform));
                        draws.push((object.kind, mesh_index, primary, top, object.clip));
                    }
                }
            }
        }

        self.last_draws = draws.len();

        if !draws.is_empty() {
            self.queue.write_buffer(
                &self.objects,
                0,
                &self.object_staging[..draws.len() * OBJECT_STRIDE],
            );
        }

        let view = self.target.create_view(&Default::default());
        let scene_view = self.multisampled.as_ref().unwrap_or(&view);
        let resolve_view = self.multisampled.as_ref().map(|_| &view);

        if let Some(game) = &scene.game {
            self.queue.write_buffer(
                &self.game_uniform,
                0,
                bytemuck::cast_slice(&[game.opacity.clamp(0.0, 1.0), 0.0, 0.0, 0.0]),
            );
        }

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Sly frame"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("background"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    resolve_target: if draws.is_empty() { resolve_view } else { None },
                    ..color_attachment(scene_view, wgpu::LoadOp::Clear(wgpu::Color::BLACK))
                })],
                ..Default::default()
            });

            if scene.visible {
                pass.set_pipeline(&self.background_pipeline);
                pass.set_bind_group(0, &self.globals_binding, &[]);
                pass.draw(0..3, 0..1);
            }
        }

        if !draws.is_empty() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("visible models"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    resolve_target: resolve_view,
                    ..color_attachment(scene_view, wgpu::LoadOp::Load)
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });

            pass.set_pipeline(&self.mesh_pipeline);
            pass.set_bind_group(0, &self.globals_binding, &[]);

            for (i, (kind, index, texture, top, clip)) in draws.iter().enumerate() {
                let Some([x, y, width, height]) = scissor(*clip, self.size) else {
                    continue;
                };

                pass.set_scissor_rect(x, y, width, height);
                let mesh = &self.models[kind][*index];
                pass.set_bind_group(1, &self.object_binding, &[(i * OBJECT_STRIDE) as u32]);
                pass.set_bind_group(2, &texture.unwrap_or(&self.neutral).binding, &[]);
                pass.set_bind_group(3, &top.unwrap_or(&self.neutral).binding, &[]);
                pass.set_vertex_buffer(0, mesh.vertex.slice(..));
                pass.set_index_buffer(mesh.index.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.count, 0, 0..1);
            }
        }

        if let (Some(game), Some((texture, _))) = (&scene.game, &self.game_texture)
            && let Some([x, y, width, height]) = game_viewport(game.rect, self.size)
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("game framebuffer"),
                color_attachments: &[Some(color_attachment(&view, wgpu::LoadOp::Load))],
                ..Default::default()
            });

            pass.set_viewport(x as f32, y as f32, width as f32, height as f32, 0.0, 1.0);
            pass.set_scissor_rect(x, y, width, height);
            pass.set_pipeline(&self.game_pipeline);
            pass.set_bind_group(0, &texture.binding, &[]);
            pass.set_bind_group(1, &self.game_binding, &[]);
            pass.draw(0..3, 0..1);
        }

        if scene.iris_radius < 1.0 || scene.fade > 0.0 {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("transition"),
                color_attachments: &[Some(color_attachment(&view, wgpu::LoadOp::Load))],
                ..Default::default()
            });

            pass.set_pipeline(&self.iris_pipeline);
            pass.set_bind_group(0, &self.iris_binding, &[]);
            pass.draw(0..3, 0..1);
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("interface"),
                color_attachments: &[Some(color_attachment(&view, wgpu::LoadOp::Load))],
                ..Default::default()
            });

            self.ui.render(&mut pass)?;
        }

        tracing::trace!(
            elapsed_ms = encode_started.elapsed().as_secs_f64() * 1000.0,
            "scene encoded"
        );

        let surface_frame = self.acquire_frame()?;
        self.needs_present = self.surface.is_some() && surface_frame.is_none();

        if let (Some(frame), Some(pipeline)) = (&surface_frame, &self.present_pipeline) {
            let view = frame.texture.create_view(&Default::default());

            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("present"),
                color_attachments: &[Some(color_attachment(
                    &view,
                    wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                ))],
                ..Default::default()
            });

            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &self.present_binding, &[]);
            pass.draw(0..3, 0..1);
        }

        let finish_started = Instant::now();
        let commands = encoder.finish();

        tracing::trace!(
            elapsed_ms = finish_started.elapsed().as_secs_f64() * 1000.0,
            "encoder finished"
        );

        let submit_started = Instant::now();
        self.queue.submit([commands]);

        tracing::trace!(
            elapsed_ms = submit_started.elapsed().as_secs_f64() * 1000.0,
            "queue submitted"
        );

        if let Some(frame) = surface_frame {
            let present_started = Instant::now();

            if let Some(window) = &self.window {
                window.pre_present_notify();
            }

            self.queue.present(frame);

            tracing::trace!(
                elapsed_ms = present_started.elapsed().as_secs_f64() * 1000.0,
                "frame presented"
            );

            self.last_presented_attempt = self.acquisition_attempt;
        }

        self.ui.end_frame();

        tracing::trace!(
            elapsed_ms = render_started.elapsed().as_secs_f64() * 1000.0,
            "render completed"
        );

        Ok(())
    }

    fn acquire_frame(&mut self) -> Result<Option<wgpu::SurfaceTexture>> {
        let Some(surface) = &self.surface else {
            return Ok(None);
        };

        let acquired = Instant::now();
        let result = surface.get_current_texture();
        self.acquisition_attempt = self.acquisition_attempt.wrapping_add(1);
        self.last_acquire_ms = acquired.elapsed().as_secs_f64() * 1000.0;

        let outcome = match result {
            wgpu::CurrentSurfaceTexture::Success(frame) => {
                tracing::trace!(
                    elapsed_ms = self.last_acquire_ms,
                    outcome = "success",
                    "surface acquisition"
                );

                return Ok(Some(frame));
            }
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                tracing::debug!(
                    elapsed_ms = self.last_acquire_ms,
                    outcome = "suboptimal",
                    "surface acquisition"
                );

                return Ok(Some(frame));
            }
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                if let Some(config) = &self.surface_config {
                    surface.configure(&self.device, config);
                }

                "lost or outdated"
            }
            wgpu::CurrentSurfaceTexture::Timeout => "timeout",
            wgpu::CurrentSurfaceTexture::Occluded => "occluded",
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err(anyhow!("Surface validation failed"));
            }
        };

        self.last_present_failure = Some(outcome);

        tracing::debug!(
            elapsed_ms = self.last_acquire_ms,
            outcome,
            "surface acquisition"
        );

        Ok(None)
    }
}

fn preferred_present_mode(supported: &[wgpu::PresentMode]) -> wgpu::PresentMode {
    // Mailbox, then Immediate, for low latency at our own 60 Hz pacing. Fifo always works.
    [wgpu::PresentMode::Mailbox, wgpu::PresentMode::Immediate]
        .into_iter()
        .find(|mode| supported.contains(mode))
        .unwrap_or(wgpu::PresentMode::Fifo)
}

fn color_attachment(
    view: &wgpu::TextureView,
    load: wgpu::LoadOp<wgpu::Color>,
) -> wgpu::RenderPassColorAttachment<'_> {
    wgpu::RenderPassColorAttachment {
        view,
        depth_slice: None,
        resolve_target: None,
        ops: wgpu::Operations {
            load,
            store: wgpu::StoreOp::Store,
        },
    }
}

fn texture_2d(
    device: &wgpu::Device,
    label: &str,
    [width, height]: [u32; 2],
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
    sample_count: u32,
    mip_level_count: u32,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count,
        sample_count,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

fn target_texture(device: &wgpu::Device, size: [u32; 2]) -> wgpu::Texture {
    texture_2d(
        device,
        "compositor",
        size,
        FORMAT,
        wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        1,
        1,
    )
}

fn multisampled_texture(
    device: &wgpu::Device,
    size: [u32; 2],
    sample_count: u32,
) -> Option<wgpu::TextureView> {
    (sample_count > 1).then(|| {
        texture_2d(
            device,
            "antialiased scene color",
            size,
            FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
            sample_count,
            1,
        )
        .create_view(&Default::default())
    })
}

fn depth_texture(device: &wgpu::Device, size: [u32; 2], sample_count: u32) -> wgpu::TextureView {
    texture_2d(
        device,
        "scene depth",
        size,
        wgpu::TextureFormat::Depth24Plus,
        wgpu::TextureUsages::RENDER_ATTACHMENT,
        sample_count,
        1,
    )
    .create_view(&Default::default())
}

fn texture_binding(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    view: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("texture"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

fn validate_image(image: &DecodedImage, limit: u32) -> Result<usize> {
    anyhow::ensure!(
        image.width > 0 && image.height > 0 && image.width <= limit && image.height <= limit,
        "artwork dimensions exceed device limits"
    );

    let count = 32 - image.width.max(image.height).leading_zeros();

    anyhow::ensure!(
        image.mip_levels.len() + 1 == count as usize,
        "artwork must contain a complete worker-generated mip pyramid"
    );

    let (mut width, mut height) = (image.width, image.height);
    let mut bytes = 0usize;

    for (w, h, rgba) in image.levels() {
        anyhow::ensure!(
            w == width && h == height,
            "artwork mip dimensions are out of order"
        );

        let expected: usize = (u64::from(w) * u64::from(h) * 4)
            .try_into()
            .context("artwork level exceeds addressable memory")?;

        anyhow::ensure!(
            rgba.len() == expected,
            "artwork mip byte count does not match dimensions"
        );

        bytes += rgba.len();
        width = (width / 2).max(1);
        height = (height / 2).max(1);
    }

    Ok(bytes)
}

fn solid_image(rgba: [u8; 4]) -> DecodedImage {
    DecodedImage {
        width: 1,
        height: 1,
        rgba: rgba.to_vec(),
        border_average: [1.0; 3],
        mip_levels: Vec::new(),
    }
}

fn upload_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    image: &DecodedImage,
) -> Texture {
    let texture = texture_2d(
        device,
        "worker-filtered artwork sRGB",
        [image.width, image.height],
        FORMAT,
        wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
        1,
        image.mip_levels.len() as u32 + 1,
    );

    for (level, (width, height, pixels)) in image.levels().enumerate() {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
    }

    let binding = texture_binding(
        device,
        layout,
        sampler,
        &texture.create_view(&Default::default()),
    );

    Texture {
        texture,
        binding,
        bytes: image.byte_len(),
    }
}

fn neutral_color(slot: MaterialSlot, authored: [f32; 4]) -> [f32; 4] {
    match slot {
        MaterialSlot::BoxFront => [0.64, 0.62, 0.67, 1.0],
        MaterialSlot::BoxBack => [0.52, 0.50, 0.55, 1.0],
        MaterialSlot::BoxSpine | MaterialSlot::BoxEdges => [0.64, 0.62, 0.67, 1.0],
        MaterialSlot::CartridgeLabel => [0.67, 0.65, 0.58, 1.0],
        // Darken the authored plastic, which washes out under the preview light.
        MaterialSlot::Shell => [
            authored[0] * 0.82,
            authored[1] * 0.82,
            authored[2] * 0.82,
            authored[3],
        ],
        _ => authored,
    }
}

fn game_viewport(rect: [f32; 4], [width, height]: [u32; 2]) -> Option<[u32; 4]> {
    if rect.iter().any(|value| !value.is_finite()) || rect[2] <= 0.0 || rect[3] <= 0.0 {
        return None;
    }

    // Round to whole pixels so integer scaling doesn't gain a pixel.
    let left = (rect[0].clamp(0.0, 1.0) * width as f32).round() as u32;
    let top = (rect[1].clamp(0.0, 1.0) * height as f32).round() as u32;
    let right = ((rect[0] + rect[2]).clamp(0.0, 1.0) * width as f32).round() as u32;
    let bottom = ((rect[1] + rect[3]).clamp(0.0, 1.0) * height as f32).round() as u32;

    (right > left && bottom > top).then_some([
        left,
        top,
        right.saturating_sub(left),
        bottom.saturating_sub(top),
    ])
}

fn scissor(clip: Option<[f32; 4]>, [width, height]: [u32; 2]) -> Option<[u32; 4]> {
    let Some([x, y, w, h]) = clip else {
        return Some([0, 0, width, height]);
    };

    if [x, y, w, h].iter().any(|value| !value.is_finite()) || w <= 0.0 || h <= 0.0 {
        return None;
    }

    let left = (x.clamp(0.0, 1.0) * width as f32).floor() as u32;
    let top = (y.clamp(0.0, 1.0) * height as f32).floor() as u32;
    let right = ((x + w).clamp(0.0, 1.0) * width as f32).ceil() as u32;
    let bottom = ((y + h).clamp(0.0, 1.0) * height as f32).ceil() as u32;

    (right > left && bottom > top).then_some([
        left,
        top,
        right.saturating_sub(left),
        bottom.saturating_sub(top),
    ])
}

struct PipelineTarget {
    format: wgpu::TextureFormat,
    blend: bool,
    sample_count: u32,
}

fn pipeline(
    device: &wgpu::Device,
    label: &str,
    shader: &wgpu::ShaderModule,
    layouts: &[&wgpu::BindGroupLayout],
    mesh: bool,
    target: PipelineTarget,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &layouts.iter().copied().map(Some).collect::<Vec<_>>(),
        immediate_size: 0,
    });

    let attributes = wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x2];

    let vertex = [Some(wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Vertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &attributes,
    })];

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: if mesh { &vertex } else { &[] },
        },
        primitive: wgpu::PrimitiveState {
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: mesh.then_some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth24Plus,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: target.sample_count,
            ..Default::default()
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target.format,
                blend: target.blend.then_some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}
