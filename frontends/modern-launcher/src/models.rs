use anyhow::{Context, Result, ensure};
use bytemuck::{Pod, Zeroable};
use core_model::{Region, System};
use glam::{Mat3, Mat4, Vec3};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModelKind {
    WesternBox,
    JapaneseBoxProxy,
    GbaBox,
    GbaJapaneseBox,
    UsCartridge,
    JpPalCartridge,
    GbaCartridge,
}

impl ModelKind {
    pub const fn is_box(self) -> bool {
        matches!(
            self,
            Self::WesternBox | Self::JapaneseBoxProxy | Self::GbaBox | Self::GbaJapaneseBox
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MaterialSlot {
    BoxFront,
    BoxBack,
    BoxSpine,
    BoxEdges,
    CartridgeLabel,
    Shell,
    Hardware,
    RearLabel,
    Pcb,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

#[derive(Clone, Debug)]
pub struct Primitive {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub slot: MaterialSlot,
    pub color: [f32; 4],
    pub roughness: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Bounds {
    pub min: Vec3,
    pub max: Vec3,
}

impl Bounds {
    pub fn size(self) -> Vec3 {
        self.max - self.min
    }
}

#[derive(Clone, Debug)]
pub struct Model {
    pub primitives: Vec<Primitive>,
    pub bounds: Bounds,
}

impl Model {
    /// Coordinates are metres, +Y up, +Z front.
    pub fn load(kind: ModelKind) -> Result<Self> {
        let bytes: &[u8] = match kind {
            ModelKind::WesternBox => {
                include_bytes!("../../../vendor/models/snes-western-box.glb")
            }
            ModelKind::JapaneseBoxProxy => {
                include_bytes!("../../../vendor/models/snes-japanese-box.glb")
            }
            ModelKind::GbaBox => {
                include_bytes!("../../../vendor/models/gba-western-box.glb")
            }
            ModelKind::GbaJapaneseBox => {
                include_bytes!("../../../vendor/models/gba-japanese-box.glb")
            }
            ModelKind::UsCartridge => include_bytes!("../../../vendor/models/snes-us-low.glb"),
            ModelKind::JpPalCartridge => {
                include_bytes!("../../../vendor/models/snes-jp-pal-low.glb")
            }
            ModelKind::GbaCartridge => {
                include_bytes!("../../../vendor/models/gba-cartridge-low.glb")
            }
        };

        Self::from_glb(bytes)
    }

    fn from_glb(bytes: &[u8]) -> Result<Self> {
        let gltf = gltf::Gltf::from_slice(bytes).context("invalid embedded glTF model")?;

        let blob = gltf
            .blob
            .as_deref()
            .context("model has no embedded buffer")?;

        ensure!(
            gltf.buffers().len() == 1,
            "expected one embedded model buffer"
        );

        let scene = gltf.default_scene().context("model has no default scene")?;
        let mut primitives = Vec::new();

        for node in scene.nodes() {
            read_node(node, Mat4::IDENTITY, blob, &mut primitives)?;
        }

        ensure!(!primitives.is_empty(), "model contains no geometry");

        let mut bounds = Bounds {
            min: Vec3::splat(f32::INFINITY),
            max: Vec3::splat(f32::NEG_INFINITY),
        };

        for vertex in primitives.iter().flat_map(|p| &p.vertices) {
            let position = Vec3::from_array(vertex.position);
            bounds.min = bounds.min.min(position);
            bounds.max = bounds.max.max(position);
        }

        Ok(Self { primitives, bounds })
    }
}

fn read_node(
    node: gltf::Node<'_>,
    parent: Mat4,
    blob: &[u8],
    output: &mut Vec<Primitive>,
) -> Result<()> {
    let transform = parent * Mat4::from_cols_array_2d(&node.transform().matrix());

    ensure!(
        transform.is_finite() && transform.determinant().abs() > 1e-12,
        "invalid model transform"
    );

    let normal_transform = Mat3::from_mat4(transform).inverse().transpose();

    if let Some(mesh) = node.mesh() {
        for primitive in mesh.primitives() {
            ensure!(
                primitive.mode() == gltf::mesh::Mode::Triangles,
                "only triangle model primitives are supported"
            );

            let reader = primitive.reader(|buffer| match buffer.source() {
                gltf::buffer::Source::Bin => Some(blob),
                gltf::buffer::Source::Uri(_) => None,
            });

            let positions: Vec<_> = reader
                .read_positions()
                .context("missing model positions")?
                .collect();

            let normals: Vec<_> = reader
                .read_normals()
                .context("missing model normals")?
                .collect();

            let uvs: Vec<_> = reader
                .read_tex_coords(0)
                .context("missing model UVs")?
                .into_f32()
                .collect();

            ensure!(
                positions.len() == normals.len() && positions.len() == uvs.len(),
                "model vertex attribute count mismatch"
            );

            let vertices: Vec<_> = positions
                .into_iter()
                .zip(normals)
                .zip(uvs)
                .map(|((p, n), uv)| Vertex {
                    position: transform.transform_point3(Vec3::from_array(p)).to_array(),
                    normal: (normal_transform * Vec3::from_array(n))
                        .normalize_or_zero()
                        .to_array(),
                    uv,
                })
                .collect();

            ensure!(
                vertices.iter().all(|v| v
                    .position
                    .iter()
                    .chain(&v.normal)
                    .chain(&v.uv)
                    .all(|c| c.is_finite())),
                "non-finite model vertex"
            );

            let mut indices: Vec<_> = reader
                .read_indices()
                .context("missing model indices")?
                .into_u32()
                .collect();

            ensure!(indices.len().is_multiple_of(3), "incomplete model triangle");

            ensure!(
                indices.iter().all(|&i| (i as usize) < vertices.len()),
                "model index out of bounds"
            );

            if transform.determinant() < 0.0 {
                for triangle in indices.as_chunks_mut::<3>().0 {
                    triangle.swap(1, 2);
                }
            }

            let material = primitive.material();

            let name = material
                .name()
                .context("model material has no semantic name")?;

            let slot =
                material_slot(name).with_context(|| format!("unmapped model material {name}"))?;
            let pbr = material.pbr_metallic_roughness();

            output.push(Primitive {
                vertices,
                indices,
                slot,
                color: pbr.base_color_factor(),
                roughness: pbr.roughness_factor(),
            });
        }
    }

    for child in node.children() {
        read_node(child, transform, blob, output)?;
    }

    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", try_from = "String")]
pub enum ReleaseMarket {
    NorthAmerica,
    Japan,
    Europe,
    Unknown,
}

impl ReleaseMarket {
    pub const ALL: [Self; 4] = [Self::NorthAmerica, Self::Japan, Self::Europe, Self::Unknown];

    pub fn region(self) -> Option<Region> {
        match self {
            Self::NorthAmerica => Some(Region::NorthAmerica),
            Self::Japan => Some(Region::Japan),
            Self::Europe => Some(Region::Europe),
            Self::Unknown => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::NorthAmerica => "North America",
            Self::Japan => "Japan",
            Self::Europe => "Europe",
            Self::Unknown => "Unknown release",
        }
    }
}

impl From<Option<Region>> for ReleaseMarket {
    fn from(region: Option<Region>) -> Self {
        match region {
            Some(Region::Japan) => Self::Japan,
            Some(Region::NorthAmerica | Region::Canada) => Self::NorthAmerica,
            Some(
                Region::Europe
                | Region::Scandinavia
                | Region::Finland
                | Region::Denmark
                | Region::France
                | Region::Netherlands
                | Region::Spain
                | Region::Germany
                | Region::Italy
                | Region::Australia,
            ) => Self::Europe,
            _ => Self::Unknown,
        }
    }
}

impl TryFrom<String> for ReleaseMarket {
    type Error = String;

    fn try_from(name: String) -> Result<Self, Self::Error> {
        match name.to_ascii_lowercase().as_str() {
            "north_america" => Ok(Self::NorthAmerica),
            "japan" => Ok(Self::Japan),
            "europe" => Ok(Self::Europe),
            "unknown" => Ok(Self::Unknown),
            _ => Err(format!("unsupported display market {name:?}")),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ArtworkMapping {
    /// [left, top, width, height] in normalized source-image coordinates.
    pub crop: [f32; 4],
    pub quarter_turns_clockwise: u8,
}

impl Default for ArtworkMapping {
    fn default() -> Self {
        Self {
            crop: [0.0, 0.0, 1.0, 1.0],
            quarter_turns_clockwise: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ArtworkMappings {
    pub front: ArtworkMapping,
    pub back: ArtworkMapping,
    pub spine: ArtworkMapping,
    pub edges: ArtworkMapping,
    pub label: ArtworkMapping,
    pub label_top: ArtworkMapping,
}

#[derive(Clone, Debug)]
pub struct PresentationProfile {
    pub system: System,
    pub market: ReleaseMarket,
    pub box_model: ModelKind,
    pub cartridge_model: Option<ModelKind>,
    pub box_dimensions_mm: [f32; 3],
    pub cartridge_dimensions_mm: Option<[f32; 3]>,
    /// Top fold occupies V in 0..fold_v. V=0 is the top of an unfolded scan.
    pub label_fold_v: f32,
    pub front_axis: [f32; 3],
    pub up_axis: [f32; 3],
    pub box_spine_axis: [f32; 3],
    pub box_spine_up_axis: [f32; 3],
    pub box_art_rotation_quarter_turns: u8,
    pub artwork: ArtworkMappings,
    /// Model coordinates in metres, before model_to_display.
    pub box_centre_m: [f32; 3],
    pub cartridge_centre_m: [f32; 3],
    pub cartridge_label_m: Option<[f32; 3]>,
}

impl PresentationProfile {
    pub fn box_size_m(&self) -> Vec3 {
        Vec3::from_array(self.box_dimensions_mm) * 0.001
    }

    pub fn cartridge_label_anchor(&self) -> Option<Vec3> {
        self.cartridge_label_m.map(Vec3::from_array)
    }

    pub fn artwork_mapping(&self, slot: MaterialSlot) -> ArtworkMapping {
        let mut mapping = match slot {
            MaterialSlot::BoxFront => self.artwork.front,
            MaterialSlot::BoxBack => self.artwork.back,
            MaterialSlot::BoxSpine => self.artwork.spine,
            MaterialSlot::BoxEdges => self.artwork.edges,
            MaterialSlot::CartridgeLabel => self.artwork.label,
            _ => return ArtworkMapping::default(),
        };

        if matches!(
            slot,
            MaterialSlot::BoxFront
                | MaterialSlot::BoxBack
                | MaterialSlot::BoxSpine
                | MaterialSlot::BoxEdges
        ) {
            mapping.quarter_turns_clockwise =
                (mapping.quarter_turns_clockwise + self.box_art_rotation_quarter_turns) % 4;
        }

        mapping
    }

    pub fn model_to_display(&self, kind: ModelKind) -> Mat4 {
        let front = Vec3::from_array(self.front_axis);
        let up = Vec3::from_array(self.up_axis);
        let basis = Mat3::from_cols(up.cross(front), up, front).transpose();

        let anchor = Vec3::from_array(if kind.is_box() {
            self.box_centre_m
        } else {
            self.cartridge_centre_m
        });

        Mat4::from_mat3(basis) * Mat4::from_translation(-anchor)
    }
}

#[derive(Clone, Debug)]
pub struct Profiles {
    pub profiles: Vec<PresentationProfile>,
}

impl Profiles {
    pub fn for_system_market(&self, system: System, market: ReleaseMarket) -> &PresentationProfile {
        self.profiles
            .iter()
            .find(|profile| profile.system == system && profile.market == market)
            .expect("validated profiles contain every system and release market")
    }

    pub fn validate(&self) -> Result<()> {
        for system in System::ALL {
            for market in ReleaseMarket::ALL {
                ensure!(
                    self.profiles
                        .iter()
                        .filter(|p| p.system == system && p.market == market)
                        .count()
                        == 1,
                    "system/market profile must be unique and present"
                );
            }
        }

        for profile in &self.profiles {
            ensure!(
                profile.box_model.is_box(),
                "packaging profile references a cartridge mesh"
            );

            validate_dimensions(&profile.box_dimensions_mm)?;

            ensure!(
                profile.cartridge_model.is_some() == profile.cartridge_dimensions_mm.is_some()
                    && profile.cartridge_model.is_some() == profile.cartridge_label_m.is_some(),
                "cartridge model, dimensions and label anchor must be specified together"
            );

            ensure!(
                profile.cartridge_model.is_none_or(|kind| !kind.is_box()),
                "cartridge profile references a packaging mesh"
            );

            if let Some(dimensions) = &profile.cartridge_dimensions_mm {
                validate_dimensions(dimensions)?;
            }

            for (front, up) in [
                (profile.front_axis, profile.up_axis),
                (profile.box_spine_axis, profile.box_spine_up_axis),
            ] {
                let front = Vec3::from_array(front);
                let up = Vec3::from_array(up);

                ensure!(
                    front.is_finite()
                        && up.is_finite()
                        && (front.length_squared() - 1.0).abs() < 1e-4
                        && (up.length_squared() - 1.0).abs() < 1e-4
                        && front.dot(up).abs() < 1e-4,
                    "profile axes must be orthonormal unit vectors"
                );
            }

            for anchor in [
                Some(profile.box_centre_m),
                Some(profile.cartridge_centre_m),
                profile.cartridge_label_m,
            ]
            .into_iter()
            .flatten()
            {
                ensure!(
                    anchor.iter().all(|v| v.is_finite()),
                    "non-finite model anchor"
                );
            }

            ensure!(
                (0.0..1.0).contains(&profile.label_fold_v),
                "invalid fold coordinate"
            );

            ensure!(
                profile.box_art_rotation_quarter_turns < 4,
                "box artwork rotation must be 0..3 quarter turns"
            );

            let artwork = &profile.artwork;

            for mapping in [
                artwork.front,
                artwork.back,
                artwork.spine,
                artwork.edges,
                artwork.label,
                artwork.label_top,
            ] {
                let [x, y, w, h] = mapping.crop;

                ensure!(
                    mapping.crop.iter().all(|v| v.is_finite())
                        && x >= 0.0
                        && y >= 0.0
                        && w > 0.0
                        && h > 0.0
                        && x + w <= 1.00001
                        && y + h <= 1.00001,
                    "artwork crop must be a nonempty normalized image rectangle"
                );

                ensure!(
                    mapping.quarter_turns_clockwise < 4,
                    "artwork rotation must be 0..3 quarter turns"
                );
            }
        }

        Ok(())
    }
}

fn validate_dimensions(dimensions: &[f32; 3]) -> Result<()> {
    ensure!(
        dimensions.iter().all(|v| v.is_finite() && *v > 0.0),
        "invalid physical dimensions"
    );

    Ok(())
}

fn material_slot(name: &str) -> Option<MaterialSlot> {
    Some(match name {
        "Western_Box_Front" => MaterialSlot::BoxFront,
        "Western_Box_Back" => MaterialSlot::BoxBack,
        "Western_Box_Spine" => MaterialSlot::BoxSpine,
        "Western_Box_Edges" => MaterialSlot::BoxEdges,
        "PAL_Label" | "NTSC_Label" | "GBA_Label" => MaterialSlot::CartridgeLabel,
        "PAL_Purchased_Shell" | "NTSC_Purchased_Shell" | "GBA_Shell" => MaterialSlot::Shell,
        "PAL_Purchased_Hardware" | "NTSC_Purchased_Hardware" | "GBA_Hardware" | "GBA_Chip" => {
            MaterialSlot::Hardware
        }
        "PAL_Purchased_Rear_Label" => MaterialSlot::RearLabel,
        "PAL_Purchased_PCB" | "GBA_PCB" => MaterialSlot::Pcb,
        _ => return None,
    })
}

impl PresentationProfile {
    fn new(
        system: System,
        market: ReleaseMarket,
        box_model: ModelKind,
        cartridge_model: Option<ModelKind>,
    ) -> Self {
        let (box_dimensions_mm, box_spine_axis, box_spine_up_axis) = match box_model {
            ModelKind::WesternBox => ([179.0, 128.0, 31.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]),
            ModelKind::JapaneseBoxProxy => {
                ([105.0, 190.0, 30.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0])
            }
            ModelKind::GbaBox => ([125.0, 125.0, 25.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
            ModelKind::GbaJapaneseBox => ([138.0, 88.0, 22.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]),
            _ => unreachable!("packaging must use a box model"),
        };

        let (cartridge_dimensions_mm, cartridge_centre_m, cartridge_label_m, label_fold_v) =
            match cartridge_model {
                Some(ModelKind::UsCartridge) => (
                    Some([135.4, 87.5, 19.9]),
                    [0.0; 3],
                    Some([-2.3034569e-11, 0.02290777, 0.010021304]),
                    7.0 / 44.0,
                ),
                Some(ModelKind::JpPalCartridge) => (
                    Some([127.0, 86.0, 20.0]),
                    [0.0; 3],
                    Some([-3.152644e-7, 0.019813074, 0.0096242]),
                    0.0,
                ),
                Some(ModelKind::GbaCartridge) => (
                    Some([57.000015, 32.958633, 6.226722]),
                    [0.0, 1.1758879e-5, -8.28295e-6],
                    Some([2.7939677e-9, -0.0015970371, 0.0018489606]),
                    0.0,
                ),
                None => (None, [0.0; 3], None, 0.0),
                _ => unreachable!("cartridge must use a cartridge model"),
            };

        Self {
            system,
            market,
            box_model,
            cartridge_model,
            box_dimensions_mm,
            cartridge_dimensions_mm,
            label_fold_v,
            box_spine_axis,
            box_spine_up_axis,
            front_axis: [0.0, 0.0, 1.0],
            up_axis: [0.0, 1.0, 0.0],
            box_art_rotation_quarter_turns: 0,
            artwork: ArtworkMappings::default(),
            box_centre_m: [0.0; 3],
            cartridge_centre_m,
            cartridge_label_m,
        }
    }
}

pub fn profiles() -> Result<Profiles> {
    use ModelKind::*;
    use ReleaseMarket::*;

    let result = Profiles {
        profiles: [
            (System::Snes, NorthAmerica, WesternBox, Some(UsCartridge)),
            (System::Snes, Japan, JapaneseBoxProxy, Some(JpPalCartridge)),
            (System::Snes, Europe, WesternBox, Some(JpPalCartridge)),
            (System::Snes, Unknown, WesternBox, None),
            (System::Gba, NorthAmerica, GbaBox, Some(GbaCartridge)),
            (System::Gba, Europe, GbaBox, Some(GbaCartridge)),
            (System::Gba, Japan, GbaJapaneseBox, Some(GbaCartridge)),
            (System::Gba, Unknown, GbaBox, Some(GbaCartridge)),
        ]
        .into_iter()
        .map(|(system, market, box_model, cartridge_model)| {
            PresentationProfile::new(system, market, box_model, cartridge_model)
        })
        .collect(),
    };

    result.validate()?;
    Ok(result)
}
