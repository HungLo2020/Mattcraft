//! Rust-owned typed expansion of Distant Horizons' semantic CPU LOD stream.
//!
//! This module deliberately does not know about DH VBOs, VAOs, shaders, or
//! framebuffers. It converts the immutable semantic records copied over FFI
//! into explicit quad geometry that a later Rust-owned LOD material pass can
//! consume without reinterpreting a Java/OpenGL vertex layout.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    GalError, GalResult, WORLD_LOD_LAYER_OPAQUE, WORLD_LOD_LAYER_TRANSPARENT_SIDE,
    WORLD_LOD_LAYER_TRANSPARENT_UP, WORLD_LOD_LAYER_TRANSPARENT_WATER_UP,
    WORLD_LOD_MAX_NORMAL_INDEX, WorldLodColumnAsset, WorldLodColumnInstanceRequest,
    WorldLodColumnMaterialProvenance, WorldLodFaceMaterial, WorldLodRenderFrame, WorldLodSegment,
    WorldLodVertex, selected_source_raster_probe_cull_mode,
    selected_source_raster_probe_front_face, validate_world_lod_column_asset,
};
use crate::render::vulkanic::CullMode;
use crate::render::vulkanic::commands::{
    AttachmentLoadOp, AttachmentStoreOp, CommandOp, PassAttachment, ResourceBarrier,
    TextureImageCopyRegion, TextureOrigin3d, TextureUsageState,
};
use crate::render::vulkanic::gal::VulkanicGal;
use crate::render::vulkanic::handles::Handle;
use crate::render::vulkanic::resources::{
    AccessFlags, BlendMode, BufferDesc, BufferUsage, CombinedTextureSamplerDesc, CompareOp,
    Extent3d, FrontFace, GraphicsPipelineDesc, IndexType, MemoryDomain, PipelineLayoutDesc,
    PrimitiveTopology, QueueClass, RenderPassDesc, RenderTargetDesc, ResourceBinding,
    ResourceBindingKind, ResourceSetDesc, SamplerAddressMode, SamplerDesc, SamplerFilter,
    TextureDesc, TextureDimension, TextureFormat, TextureUsage, TextureViewDesc,
};
use crate::render::vulkanic::shader_pack::distant_horizons_contract::DistantHorizonsPassKind;
use crate::render::vulkanic::shader_pack::lightmap::VanillaLightmapBinding;
use crate::render::vulkanic::shader_pack::programs::{
    LoweredDistantHorizonsExactAtlasSourceProgram, LoweredDistantHorizonsSourceProgram,
    distant_horizons_exact_atlas_source_resource_layout,
    distant_horizons_lod_exact_atlas_resource_layouts,
    distant_horizons_lod_opaque_resource_layouts,
    minimal_distant_horizons_lod_exact_atlas_opaque_program,
    minimal_distant_horizons_lod_opaque_program, minimal_distant_horizons_lod_transparent_program,
};
use crate::render::vulkanic::shader_pack::source_targets::{
    ShaderPackColorTargets, TerrainSourceColorAttachment, source_color_clear_color,
};
use crate::render::vulkanic::shader_pack::source_uniforms::TerrainSourceUniformFrame;
use crate::render::vulkanic::shader_pack::terrain_contract::TerrainPassOutput;
use crate::render::vulkanic::shader_pack::terrain_source_resources::{
    TerrainSourceOwnedResource, TerrainSourceOwnedResourceSet, TerrainSourceResourceAvailability,
    TerrainSourceResourceAvailabilitySet, TerrainSourceResourceRole,
    TerrainSourceSampledResourceShape,
};

const MICRO_OFFSET_SCALE: f32 = 0.01;

/// Distant Horizons preserves the source OpenGL quad order in both its
/// reduced-color and provenance-resolved exact-atlas streams. The Rust Vulkan
/// whole-frame target uses the standard Vulkan viewport orientation, so that
/// source order remains counter-clockwise at rasterization.
const WORLD_LOD_SOURCE_FRONT_FACE: FrontFace = FrontFace::CounterClockwise;

/// Private Rust shader-input layout for expanded DH columns. This is not the
/// FFI record layout and intentionally has no OpenGL/Vulkan vertex-format
/// meaning. Backends receive it only after a later LOD material pass defines
/// an explicit pipeline interface.
pub(crate) const WORLD_LOD_GPU_VERTEX_LAYOUT_V1: u32 = 1;
pub(crate) const WORLD_LOD_GPU_VERTEX_BYTES: usize = 32;
/// Private Rust-owned exact-atlas DH vertex ABI. Unlike the legacy DH stream,
/// this carries copied atlas UVs and has no Java/OpenGL layout meaning.
pub(crate) const WORLD_LOD_TEXTURED_GPU_VERTEX_LAYOUT_V2: u32 = 2;
pub(crate) const WORLD_LOD_TEXTURED_GPU_VERTEX_BYTES: usize = 56;
pub(crate) const WORLD_LOD_TERRAIN_ATLAS_IDENTITY: &str = "minecraft:textures/atlas/blocks.png";

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct WorldLodDrawUniform {
    pub combined_matrix: [f32; 16],
    pub column_origin_and_world_y: [f32; 4],
    /// Distant Horizons keeps its per-column geometry local and supplies the
    /// camera-relative column origin independently of its raw model-view
    /// matrix. The copied column origin already contains the dimension's
    /// minimum Y, so `world_y_offset` remains scalar source-pack context and
    /// must not be applied a second time to clip-space placement.
    pub model_offset_and_reserved: [f32; 4],
    pub clip_micro_noise_earth: [f32; 4],
    pub flags_and_noise: [u32; 4],
}

impl WorldLodDrawUniform {
    pub(crate) fn from_semantics(
        frame: &WorldLodRenderFrame,
        draw: WorldLodGpuDraw,
    ) -> GalResult<Self> {
        Self::from_semantics_with_camera(frame, draw, [0.0; 3])
    }

    pub(crate) fn from_semantics_with_camera(
        frame: &WorldLodRenderFrame,
        draw: WorldLodGpuDraw,
        camera_world_position: [f32; 3],
    ) -> GalResult<Self> {
        if !frame.enabled {
            return Err(GalError::invalid_argument(
                "world LOD draw uniform requires an enabled semantic render frame",
            ));
        }
        if frame.flags & !0x1f != 0
            || frame.micro_offset <= 0.0
            || frame.clip_distance < 0.0
            || frame
                .combined_matrix
                .iter()
                .chain([
                    &frame.clip_distance,
                    &frame.micro_offset,
                    &frame.noise_intensity,
                    &frame.earth_radius,
                ])
                .any(|value| !value.is_finite())
            || camera_world_position.iter().any(|value| !value.is_finite())
        {
            return Err(GalError::invalid_argument(
                "world LOD draw uniform received invalid frame semantics",
            ));
        }
        Ok(Self {
            combined_matrix: frame.combined_matrix,
            column_origin_and_world_y: [
                draw.origin[0] as f32,
                draw.origin[1] as f32,
                draw.origin[2] as f32,
                frame.world_y_offset as f32,
            ],
            model_offset_and_reserved: [
                draw.origin[0] as f32 - camera_world_position[0],
                draw.origin[1] as f32 - camera_world_position[1],
                draw.origin[2] as f32 - camera_world_position[2],
                0.0,
            ],
            clip_micro_noise_earth: [
                frame.clip_distance,
                frame.micro_offset,
                frame.noise_intensity,
                frame.earth_radius,
            ],
            flags_and_noise: [
                frame.flags,
                frame.noise_steps,
                frame.noise_dropoff as u32,
                0,
            ],
        })
    }

    /// Fixed backend-neutral buffer layout for the first Rust-owned LOD
    /// material pass. The layout consists of one matrix, three vec4-aligned
    /// float blocks, and one uvec4 block, so both Vulkan std140 and the OpenGL
    /// compatibility backend can bind the same owned bytes without decoding
    /// any DH or Java renderer state.
    pub(crate) fn pack_std140(self) -> [u8; 128] {
        let mut bytes = [0u8; 128];
        let mut offset = 0usize;
        for value in self
            .combined_matrix
            .iter()
            .chain(self.column_origin_and_world_y.iter())
            .chain(self.model_offset_and_reserved.iter())
            .chain(self.clip_micro_noise_earth.iter())
        {
            bytes[offset..offset + std::mem::size_of::<f32>()]
                .copy_from_slice(&value.to_ne_bytes());
            offset += std::mem::size_of::<f32>();
        }
        for value in self.flags_and_noise {
            bytes[offset..offset + std::mem::size_of::<u32>()]
                .copy_from_slice(&value.to_ne_bytes());
            offset += std::mem::size_of::<u32>();
        }
        debug_assert_eq!(offset, bytes.len());
        bytes
    }
}

/// Distant Horizons' public block-material classification. It deliberately
/// remains a small semantic category: source-specific texture and shader-pack
/// policy belong to a later Rust-owned LOD material resolver.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum WorldLodMaterialCategory {
    Unknown,
    Leaves,
    Stone,
    Wood,
    Metal,
    Dirt,
    Lava,
    Deepslate,
    Snow,
    Sand,
    Terracotta,
    NetherStone,
    Water,
    Grass,
    Air,
    Illuminated,
}

impl TryFrom<u8> for WorldLodMaterialCategory {
    type Error = GalError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Unknown),
            1 => Ok(Self::Leaves),
            2 => Ok(Self::Stone),
            3 => Ok(Self::Wood),
            4 => Ok(Self::Metal),
            5 => Ok(Self::Dirt),
            6 => Ok(Self::Lava),
            7 => Ok(Self::Deepslate),
            8 => Ok(Self::Snow),
            9 => Ok(Self::Sand),
            10 => Ok(Self::Terracotta),
            11 => Ok(Self::NetherStone),
            12 => Ok(Self::Water),
            13 => Ok(Self::Grass),
            14 => Ok(Self::Air),
            15 => Ok(Self::Illuminated),
            _ => Err(GalError::invalid_argument(format!(
                "unknown Distant Horizons material category {value}"
            ))),
        }
    }
}

/// The Rust-owned material pass class is explicit in the semantic planner.
/// Transparent work keeps the exact legacy-visible order and remains separate
/// from opaque work; neither backend infers blending policy from a layer id.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorldLodPassClass {
    Opaque,
    Transparent,
    WaterSurface,
}

/// A named material contract for the first Rust-owned DH pass. DH emits
/// pre-resolved terrain color rather than Minecraft atlas UVs, so this cannot
/// share the atlas/material bindings used by static chunk terrain. The
/// lightmap is still a semantic Rust-owned resource, not a Java texture.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WorldLodMaterialContract {
    pub pass: WorldLodPassClass,
    pub vertex_layout_version: u32,
    pub requires_vanilla_lightmap: bool,
    pub uses_vertex_color: bool,
    pub uses_face_normal: bool,
    pub uses_material_category: bool,
}

impl WorldLodMaterialContract {
    pub(crate) const OPAQUE: Self = Self {
        pass: WorldLodPassClass::Opaque,
        vertex_layout_version: WORLD_LOD_GPU_VERTEX_LAYOUT_V1,
        requires_vanilla_lightmap: true,
        uses_vertex_color: true,
        uses_face_normal: true,
        uses_material_category: true,
    };

    pub(crate) const TRANSPARENT: Self = Self {
        pass: WorldLodPassClass::Transparent,
        vertex_layout_version: WORLD_LOD_GPU_VERTEX_LAYOUT_V1,
        requires_vanilla_lightmap: true,
        uses_vertex_color: true,
        uses_face_normal: true,
        uses_material_category: true,
    };

    pub(crate) const WATER_SURFACE: Self = Self {
        pass: WorldLodPassClass::WaterSurface,
        vertex_layout_version: WORLD_LOD_GPU_VERTEX_LAYOUT_V1,
        requires_vanilla_lightmap: true,
        uses_vertex_color: true,
        uses_face_normal: true,
        uses_material_category: true,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorldLodAdmissionError {
    UnknownLayer(u32),
    WaterLayerRequiresWaterPath(u32),
}

impl std::fmt::Display for WorldLodAdmissionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownLayer(layer) => {
                write!(formatter, "unknown Distant Horizons layer {layer}")
            }
            Self::WaterLayerRequiresWaterPath(layer) => write!(
                formatter,
                "Distant Horizons water layer {layer} must use the explicit water-surface admission path"
            ),
        }
    }
}

impl std::error::Error for WorldLodAdmissionError {}

/// A fully resolved, backend-neutral opaque LOD draw. It preserves the
/// copied semantic material categories for a later Rust material resolver,
/// while all GPU handles remain private to the frontend.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WorldLodOpaqueDraw {
    pub draw: WorldLodGpuDraw,
    pub uniforms: WorldLodDrawUniform,
    pub pass: WorldLodPassClass,
    pub material_contract: WorldLodMaterialContract,
}

/// A resolved non-water transparent draw. `order` is copied from the actual
/// DH visible-list traversal and is retained until the private transparent
/// pass consumes it; no backend-side resorting is permitted.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WorldLodTransparentDraw {
    pub draw: WorldLodGpuDraw,
    pub uniforms: WorldLodDrawUniform,
    pub pass: WorldLodPassClass,
    pub material_contract: WorldLodMaterialContract,
}

/// A resolved DH water-surface draw. DH emits this as its own stream because
/// it must preserve the material's depth/cull policy independently of general
/// transparent detail geometry.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WorldLodWaterDraw {
    pub draw: WorldLodGpuDraw,
    pub uniforms: WorldLodDrawUniform,
    pub pass: WorldLodPassClass,
    pub material_contract: WorldLodMaterialContract,
}

/// Complete semantic classification of the DH work visible in one frame. The
/// planner owns no rendering policy beyond the source layer contract: the
/// caller receives one admitted stream for every visible segment.
#[derive(Clone, Debug, Default)]
pub(crate) struct WorldLodFramePlan {
    pub opaque_draws: Vec<WorldLodOpaqueDraw>,
    pub transparent_draws: Vec<WorldLodTransparentDraw>,
    pub water_draws: Vec<WorldLodWaterDraw>,
}

pub(crate) fn plan_world_lod_frame(
    frame: &WorldLodRenderFrame,
    draws: &[WorldLodGpuDraw],
) -> GalResult<WorldLodFramePlan> {
    plan_world_lod_frame_with_camera(frame, draws, [0.0; 3])
}

/// Resolves visible DH draws using the copied semantic camera position. This
/// is intentionally a frontend transform input, not a backend or FFI detail.
pub(crate) fn plan_world_lod_frame_with_camera(
    frame: &WorldLodRenderFrame,
    draws: &[WorldLodGpuDraw],
    camera_world_position: [f32; 3],
) -> GalResult<WorldLodFramePlan> {
    let mut plan = WorldLodFramePlan {
        opaque_draws: Vec::with_capacity(draws.len()),
        transparent_draws: Vec::new(),
        water_draws: Vec::new(),
    };
    for &draw in draws {
        match draw.layer {
            WORLD_LOD_LAYER_OPAQUE => plan.opaque_draws.push(admit_world_lod_draw_with_camera(
                frame,
                draw,
                camera_world_position,
            )?),
            WORLD_LOD_LAYER_TRANSPARENT_SIDE | WORLD_LOD_LAYER_TRANSPARENT_UP => {
                plan.transparent_draws
                    .push(admit_world_lod_transparent_draw_with_camera(
                        frame,
                        draw,
                        camera_world_position,
                    )?);
            }
            WORLD_LOD_LAYER_TRANSPARENT_WATER_UP => {
                plan.water_draws
                    .push(admit_world_lod_water_draw_with_camera(
                        frame,
                        draw,
                        camera_world_position,
                    )?);
            }
            layer => {
                return Err(GalError::invalid_argument(
                    WorldLodAdmissionError::UnknownLayer(layer).to_string(),
                ));
            }
        }
    }
    plan.transparent_draws.sort_by_key(|draw| {
        (
            draw.draw.order,
            draw.draw.layer,
            draw.draw.column_key,
            draw.draw.segment_index,
        )
    });
    plan.water_draws.sort_by_key(|draw| {
        (
            draw.draw.order,
            draw.draw.layer,
            draw.draw.column_key,
            draw.draw.segment_index,
        )
    });
    Ok(plan)
}

pub(crate) fn admit_world_lod_water_draw_with_camera(
    frame: &WorldLodRenderFrame,
    draw: WorldLodGpuDraw,
    camera_world_position: [f32; 3],
) -> GalResult<WorldLodWaterDraw> {
    if draw.layer != WORLD_LOD_LAYER_TRANSPARENT_WATER_UP {
        return Err(GalError::invalid_argument(
            "Distant Horizons water-surface draw received a non-water layer",
        ));
    }
    let uniforms =
        WorldLodDrawUniform::from_semantics_with_camera(frame, draw, camera_world_position)?;
    Ok(WorldLodWaterDraw {
        draw,
        uniforms,
        pass: WorldLodPassClass::WaterSurface,
        material_contract: WorldLodMaterialContract::WATER_SURFACE,
    })
}

pub(crate) fn admit_world_lod_draw(
    frame: &WorldLodRenderFrame,
    draw: WorldLodGpuDraw,
) -> GalResult<WorldLodOpaqueDraw> {
    admit_world_lod_draw_with_camera(frame, draw, [0.0; 3])
}

pub(crate) fn admit_world_lod_draw_with_camera(
    frame: &WorldLodRenderFrame,
    draw: WorldLodGpuDraw,
    camera_world_position: [f32; 3],
) -> GalResult<WorldLodOpaqueDraw> {
    let pass = match draw.layer {
        WORLD_LOD_LAYER_OPAQUE => WorldLodPassClass::Opaque,
        WORLD_LOD_LAYER_TRANSPARENT_SIDE
        | WORLD_LOD_LAYER_TRANSPARENT_UP
        | WORLD_LOD_LAYER_TRANSPARENT_WATER_UP => {
            return Err(GalError::unsupported_feature(
                "Distant Horizons transparent draw must use the explicit transparent admission path",
            ));
        }
        layer => {
            return Err(GalError::invalid_argument(
                WorldLodAdmissionError::UnknownLayer(layer).to_string(),
            ));
        }
    };
    // Frame validation happens before draw resolution. Preserve that division:
    // admission only classifies the semantic layer and never hides malformed
    // render-frame state behind an "unsupported" result.
    let uniforms =
        WorldLodDrawUniform::from_semantics_with_camera(frame, draw, camera_world_position)?;
    Ok(WorldLodOpaqueDraw {
        draw,
        uniforms,
        pass,
        material_contract: WorldLodMaterialContract::OPAQUE,
    })
}

pub(crate) fn admit_world_lod_transparent_draw(
    frame: &WorldLodRenderFrame,
    draw: WorldLodGpuDraw,
) -> GalResult<WorldLodTransparentDraw> {
    admit_world_lod_transparent_draw_with_camera(frame, draw, [0.0; 3])
}

pub(crate) fn admit_world_lod_transparent_draw_with_camera(
    frame: &WorldLodRenderFrame,
    draw: WorldLodGpuDraw,
    camera_world_position: [f32; 3],
) -> GalResult<WorldLodTransparentDraw> {
    match draw.layer {
        WORLD_LOD_LAYER_TRANSPARENT_SIDE | WORLD_LOD_LAYER_TRANSPARENT_UP => {}
        WORLD_LOD_LAYER_TRANSPARENT_WATER_UP => {
            return Err(GalError::unsupported_feature(
                WorldLodAdmissionError::WaterLayerRequiresWaterPath(draw.layer).to_string(),
            ));
        }
        WORLD_LOD_LAYER_OPAQUE => {
            return Err(GalError::invalid_argument(
                "Distant Horizons opaque draw must use the opaque admission path",
            ));
        }
        layer => {
            return Err(GalError::invalid_argument(
                WorldLodAdmissionError::UnknownLayer(layer).to_string(),
            ));
        }
    }
    let uniforms =
        WorldLodDrawUniform::from_semantics_with_camera(frame, draw, camera_world_position)?;
    Ok(WorldLodTransparentDraw {
        draw,
        uniforms,
        pass: WorldLodPassClass::Transparent,
        material_contract: WorldLodMaterialContract::TRANSPARENT,
    })
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct WorldLodDrawResourceKey {
    column_key: u64,
    column_generation: u64,
    segment_index: u32,
}

impl WorldLodDrawResourceKey {
    fn from_draw(draw: WorldLodGpuDraw) -> Self {
        Self {
            column_key: draw.column_key,
            column_generation: draw.column_generation,
            // A column can legitimately contain several segments in the same
            // layer, so the buffer handle is included in the private cache
            // key's identity through this ordinal-like stable handle slot.
            // The visible semantic request has already resolved it against
            // the exact generation before reaching this pass owner.
            segment_index: draw.segment_index,
        }
    }
}

struct WorldLodPipelineResources {
    vertex_shader: Handle,
    fragment_shader: Handle,
    geometry_and_frame_layout: Handle,
    lightmap_layout: Handle,
    pipeline_layout: Handle,
    pipeline: Handle,
}

impl WorldLodPipelineResources {
    fn destroy(self, gal: &mut VulkanicGal) {
        for handle in [
            self.pipeline,
            self.pipeline_layout,
            self.lightmap_layout,
            self.geometry_and_frame_layout,
            self.fragment_shader,
            self.vertex_shader,
        ] {
            let _ = gal.destroy(handle);
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct WorldLodDrawResources {
    uniform_buffer: Handle,
    resource_set: Handle,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct WorldLodLightmapResourceKey {
    world_generation: u64,
    lightmap_generation: u64,
    /// A semantic generation can be staged again while the surrounding frame
    /// graph is rebuilt. Keep the set-one binding tied to the exact private
    /// GAL residency, rather than accidentally reusing a set that still
    /// references its predecessor's view.
    texture_view: Handle,
    sampler: Handle,
}

impl From<VanillaLightmapBinding> for WorldLodLightmapResourceKey {
    fn from(binding: VanillaLightmapBinding) -> Self {
        Self {
            world_generation: binding.world_generation,
            lightmap_generation: binding.lightmap_generation,
            texture_view: binding.texture_view,
            sampler: binding.sampler,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct WorldLodLightmapResources {
    texture_view: Handle,
    sampler: Handle,
    resource_set: Handle,
}

impl WorldLodDrawResources {
    fn destroy(self, gal: &mut VulkanicGal) {
        let _ = gal.destroy(self.resource_set);
        let _ = gal.destroy(self.uniform_buffer);
    }
}

/// Private resource owner shared by the opaque and non-water transparent DH
/// material passes. It owns only set-zero resources because the shared
/// semantic lightmap's generation-keyed set-one binding is supplied by the
/// later frame executor. The pass class remains explicit so compatible
/// semantic streams share no accidental blend/depth policy.
struct WorldLodPassResources {
    pass: WorldLodPassClass,
    pipeline: Option<WorldLodPipelineResources>,
    draws: BTreeMap<WorldLodDrawResourceKey, WorldLodDrawResources>,
    lightmaps: BTreeMap<WorldLodLightmapResourceKey, WorldLodLightmapResources>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct WorldLodPreparedDraw {
    pub pipeline: Handle,
    pub pipeline_layout: Handle,
    pub geometry_resource_set: Handle,
    pub lightmap_resource_set: Handle,
    pub index_buffer: Handle,
    pub index_type: IndexType,
    pub index_count: u32,
}

impl WorldLodPassResources {
    fn new(pass: WorldLodPassClass) -> Self {
        Self {
            pass,
            pipeline: None,
            draws: BTreeMap::new(),
            lightmaps: BTreeMap::new(),
        }
    }

    fn stage_draw(
        &mut self,
        gal: &mut VulkanicGal,
        draw: WorldLodGpuDraw,
        uniforms: WorldLodDrawUniform,
        material_contract: WorldLodMaterialContract,
        lightmap: VanillaLightmapBinding,
        ops: &mut Vec<CommandOp>,
    ) -> GalResult<WorldLodPreparedDraw> {
        if material_contract.pass != self.pass
            || material_contract.vertex_layout_version != WORLD_LOD_GPU_VERTEX_LAYOUT_V1
            || draw.index_count == 0
            || draw.index_count % 3 != 0
        {
            return Err(GalError::invalid_argument(
                "world LOD material pass received an invalid admitted draw",
            ));
        }
        self.ensure_pipeline(gal)?;
        let key = WorldLodDrawResourceKey::from_draw(draw);
        if !self.draws.contains_key(&key) {
            let pipeline = self
                .pipeline
                .as_ref()
                .expect("world LOD pipeline exists after successful initialization");
            let uniform_buffer = gal.create_buffer(BufferDesc {
                label: format!(
                    "world-lod-column{}-gen{}-segment{}.frame",
                    key.column_key, key.column_generation, key.segment_index
                ),
                size: 128,
                memory: MemoryDomain::Upload,
                usages: vec![BufferUsage::Uniform, BufferUsage::HostWrite],
            })?;
            let resource_set = match gal.create_resource_set(ResourceSetDesc {
                label: format!(
                    "world-lod-column{}-gen{}-segment{}.geometry-and-frame-set",
                    key.column_key, key.column_generation, key.segment_index
                ),
                layout: pipeline.geometry_and_frame_layout,
                bindings: vec![
                    ResourceBinding {
                        binding: 0,
                        array_index: 0,
                        resource: draw.vertex_buffer,
                        kind: ResourceBindingKind::StorageBuffer,
                        access: AccessFlags::READ,
                        dynamic_offsets: Vec::new(),
                        buffer_range: None,
                    },
                    ResourceBinding {
                        binding: 1,
                        array_index: 0,
                        resource: uniform_buffer,
                        kind: ResourceBindingKind::UniformBuffer,
                        access: AccessFlags::READ,
                        dynamic_offsets: Vec::new(),
                        buffer_range: None,
                    },
                ],
            }) {
                Ok(set) => set,
                Err(error) => {
                    let _ = gal.destroy(uniform_buffer);
                    return Err(error);
                }
            };
            self.draws.insert(
                key,
                WorldLodDrawResources {
                    uniform_buffer,
                    resource_set,
                },
            );
        }
        let resources = self
            .draws
            .get(&key)
            .copied()
            .expect("world LOD material resource entry exists after creation");
        let lightmap_resource_set = match self.ensure_lightmap_resource_set(gal, lightmap) {
            Ok(resource_set) => resource_set,
            Err(error) => return Err(error),
        };
        ops.extend([
            CommandOp::Barrier(buffer_barrier(
                resources.uniform_buffer,
                TextureUsageState::ShaderRead,
                TextureUsageState::TransferDst,
            )),
            CommandOp::HostWriteBuffer {
                buffer: resources.uniform_buffer,
                offset: 0,
                data: uniforms.pack_std140().to_vec(),
            },
            CommandOp::Barrier(buffer_barrier(
                resources.uniform_buffer,
                TextureUsageState::TransferDst,
                TextureUsageState::ShaderRead,
            )),
        ]);
        let pipeline = self
            .pipeline
            .as_ref()
            .expect("world LOD pipeline remains alive while draw resources are live");
        Ok(WorldLodPreparedDraw {
            pipeline: pipeline.pipeline,
            pipeline_layout: pipeline.pipeline_layout,
            geometry_resource_set: resources.resource_set,
            lightmap_resource_set,
            index_buffer: draw.index_buffer,
            index_type: draw.index_type,
            index_count: draw.index_count,
        })
    }

    pub(crate) fn destroy(&mut self, gal: &mut VulkanicGal) {
        for (_, draw) in std::mem::take(&mut self.draws) {
            draw.destroy(gal);
        }
        self.clear_lightmap_bindings(gal);
        if let Some(pipeline) = self.pipeline.take() {
            pipeline.destroy(gal);
        }
    }

    /// A lightmap residency may be replaced after the combined frame that
    /// first uses its successor has been submitted. Retire only the stale
    /// set-one bindings before the runtime releases the superseded texture
    /// view; geometry and pipeline resources remain generation-independent.
    pub(crate) fn retain_lightmap_binding(
        &mut self,
        gal: &mut VulkanicGal,
        binding: VanillaLightmapBinding,
    ) {
        let retained = WorldLodLightmapResourceKey::from(binding);
        let stale = self
            .lightmaps
            .keys()
            .filter(|key| **key != retained)
            .copied()
            .collect::<Vec<_>>();
        for key in stale {
            if let Some(resources) = self.lightmaps.remove(&key) {
                let _ = gal.destroy(resources.resource_set);
            }
        }
    }

    /// Releases every private set-one binding before the owning shader
    /// runtime tears down its lightmap residency. The runtime owns the image
    /// lifetime; this cache owns only consumers of its view.
    pub(crate) fn clear_lightmap_bindings(&mut self, gal: &mut VulkanicGal) {
        for (_, resources) in std::mem::take(&mut self.lightmaps) {
            let _ = gal.destroy(resources.resource_set);
        }
    }

    /// Drops private set-zero bindings that no longer match the immutable
    /// column asset map. Geometry residency owns the buffers themselves; this
    /// cache owns only resource sets/uniform buffers that reference them, so
    /// both layers retire at the same generation boundary.
    pub(crate) fn reconcile_assets(
        &mut self,
        gal: &mut VulkanicGal,
        assets: &BTreeMap<u64, WorldLodGpuColumnAsset>,
    ) {
        let stale = self
            .draws
            .keys()
            .filter(|key| {
                assets.get(&key.column_key).is_none_or(|asset| {
                    asset.column_generation != key.column_generation
                        || asset.segments.get(key.segment_index as usize).is_none()
                })
            })
            .copied()
            .collect::<Vec<_>>();
        for key in stale {
            if let Some(resources) = self.draws.remove(&key) {
                resources.destroy(gal);
            }
        }
    }

    fn ensure_pipeline(&mut self, gal: &mut VulkanicGal) -> GalResult<()> {
        if self.pipeline.is_some() {
            return Ok(());
        }
        let (label, program, blend, cull_mode, depth_compare, depth_write, color_formats) =
            match self.pass {
                WorldLodPassClass::Opaque => (
                    "world-lod-opaque",
                    minimal_distant_horizons_lod_opaque_program(),
                    BlendMode::Disabled,
                    CullMode::Back,
                    CompareOp::LessOrEqual,
                    true,
                    vec![TextureFormat::Rgba8Unorm; 4],
                ),
                WorldLodPassClass::Transparent => (
                    "world-lod-transparent",
                    minimal_distant_horizons_lod_transparent_program(),
                    BlendMode::Alpha,
                    CullMode::Back,
                    CompareOp::LessOrEqual,
                    false,
                    vec![TextureFormat::Rgba8Unorm],
                ),
                WorldLodPassClass::WaterSurface => (
                    "world-lod-water-surface",
                    minimal_distant_horizons_lod_transparent_program(),
                    BlendMode::Alpha,
                    CullMode::None,
                    CompareOp::Always,
                    true,
                    vec![TextureFormat::Rgba8Unorm],
                ),
            };
        let [geometry_and_frame_desc, lightmap_desc] =
            distant_horizons_lod_opaque_resource_layouts(label);
        let mut created = Vec::new();
        let result = (|| -> GalResult<WorldLodPipelineResources> {
            let geometry_and_frame_layout = gal.create_resource_layout(geometry_and_frame_desc)?;
            created.push(geometry_and_frame_layout);
            let lightmap_layout = gal.create_resource_layout(lightmap_desc)?;
            created.push(lightmap_layout);
            let pipeline_layout = gal.create_pipeline_layout(PipelineLayoutDesc {
                label: format!("{label}.pipeline-layout"),
                resource_layouts: vec![geometry_and_frame_layout, lightmap_layout],
            })?;
            created.push(pipeline_layout);
            let [vertex_desc, fragment_desc] =
                program.shader_module_descriptors(gal.capabilities().api);
            let vertex_shader = gal.create_shader_module(vertex_desc)?;
            created.push(vertex_shader);
            let fragment_shader = gal.create_shader_module(fragment_desc)?;
            created.push(fragment_shader);
            let pipeline = gal.create_graphics_pipeline(GraphicsPipelineDesc {
                label: format!("{label}.pipeline"),
                layout: pipeline_layout,
                vertex_shader,
                fragment_shader,
                topology: PrimitiveTopology::Triangles,
                cull_mode,
                front_face: FrontFace::CounterClockwise,
                provoking_vertex: crate::render::vulkanic::resources::ProvokingVertex::Last,
                raster_y_direction: crate::render::vulkanic::resources::RasterYDirection::Up,
                blend,
                depth_compare: Some(depth_compare),
                depth_write,
                depth_bias: None,
                color_formats,
                depth_format: Some(TextureFormat::Depth32Float),
                stencil: None,
            })?;
            created.push(pipeline);
            Ok(WorldLodPipelineResources {
                vertex_shader,
                fragment_shader,
                geometry_and_frame_layout,
                lightmap_layout,
                pipeline_layout,
                pipeline,
            })
        })();
        if result.is_err() {
            for handle in created.into_iter().rev() {
                let _ = gal.destroy(handle);
            }
        }
        self.pipeline = Some(result?);
        Ok(())
    }

    fn ensure_lightmap_resource_set(
        &mut self,
        gal: &mut VulkanicGal,
        binding: VanillaLightmapBinding,
    ) -> GalResult<Handle> {
        if binding.world_generation == 0 || binding.lightmap_generation == 0 {
            return Err(GalError::invalid_argument(
                "world LOD material pass requires a complete vanilla lightmap generation",
            ));
        }
        let key = WorldLodLightmapResourceKey::from(binding);
        if let Some(resources) = self.lightmaps.get(&key) {
            if resources.texture_view != binding.texture_view
                || resources.sampler != binding.sampler
            {
                return Err(GalError::invalid_argument(
                    "world LOD lightmap generation resolved to different Rust-owned resources",
                ));
            }
            return Ok(resources.resource_set);
        }
        let pipeline = self
            .pipeline
            .as_ref()
            .expect("world LOD pipeline exists before lightmap binding creation");
        let resource_set = gal.create_resource_set(ResourceSetDesc {
            label: format!(
                "world-lod-lightmap.world{}-gen{}.resource-set",
                key.world_generation, key.lightmap_generation
            ),
            layout: pipeline.lightmap_layout,
            bindings: vec![
                ResourceBinding {
                    binding: 0,
                    array_index: 0,
                    resource: binding.texture_view,
                    kind: ResourceBindingKind::SampledTexture,
                    access: AccessFlags::READ,
                    dynamic_offsets: Vec::new(),
                    buffer_range: None,
                },
                ResourceBinding {
                    binding: 1,
                    array_index: 0,
                    resource: binding.sampler,
                    kind: ResourceBindingKind::Sampler,
                    access: AccessFlags::READ,
                    dynamic_offsets: Vec::new(),
                    buffer_range: None,
                },
            ],
        })?;
        self.lightmaps.insert(
            key,
            WorldLodLightmapResources {
                texture_view: binding.texture_view,
                sampler: binding.sampler,
                resource_set,
            },
        );
        Ok(resource_set)
    }
}

/// Compatibility wrapper for the first Rust-owned DH opaque material pass.
/// The shared internal owner keeps opaque and transparent resource policy
/// physically separate while avoiding producer-specific duplicate plumbing.
pub(crate) struct WorldLodOpaquePassResources {
    inner: WorldLodPassResources,
}

impl Default for WorldLodOpaquePassResources {
    fn default() -> Self {
        Self {
            inner: WorldLodPassResources::new(WorldLodPassClass::Opaque),
        }
    }
}

impl WorldLodOpaquePassResources {
    pub(crate) fn stage_draw(
        &mut self,
        gal: &mut VulkanicGal,
        draw: WorldLodOpaqueDraw,
        lightmap: VanillaLightmapBinding,
        ops: &mut Vec<CommandOp>,
    ) -> GalResult<WorldLodPreparedDraw> {
        if draw.pass != WorldLodPassClass::Opaque
            || draw.material_contract != WorldLodMaterialContract::OPAQUE
        {
            return Err(GalError::invalid_argument(
                "world LOD opaque pass received a non-opaque admitted draw",
            ));
        }
        self.inner.stage_draw(
            gal,
            draw.draw,
            draw.uniforms,
            draw.material_contract,
            lightmap,
            ops,
        )
    }

    pub(crate) fn destroy(&mut self, gal: &mut VulkanicGal) {
        self.inner.destroy(gal);
    }

    pub(crate) fn retain_lightmap_binding(
        &mut self,
        gal: &mut VulkanicGal,
        binding: VanillaLightmapBinding,
    ) {
        self.inner.retain_lightmap_binding(gal, binding);
    }

    pub(crate) fn clear_lightmap_bindings(&mut self, gal: &mut VulkanicGal) {
        self.inner.clear_lightmap_bindings(gal);
    }

    pub(crate) fn reconcile_assets(
        &mut self,
        gal: &mut VulkanicGal,
        assets: &BTreeMap<u64, WorldLodGpuColumnAsset>,
    ) {
        self.inner.reconcile_assets(gal, assets);
    }
}

/// Rust-owned non-water transparent DH pass resources. The public type makes
/// its separate blend/depth execution policy explicit; water is intentionally
/// not admitted here.
pub(crate) struct WorldLodTransparentPassResources {
    inner: WorldLodPassResources,
}

impl Default for WorldLodTransparentPassResources {
    fn default() -> Self {
        Self {
            inner: WorldLodPassResources::new(WorldLodPassClass::Transparent),
        }
    }
}

impl WorldLodTransparentPassResources {
    pub(crate) fn stage_draw(
        &mut self,
        gal: &mut VulkanicGal,
        draw: WorldLodTransparentDraw,
        lightmap: VanillaLightmapBinding,
        ops: &mut Vec<CommandOp>,
    ) -> GalResult<WorldLodPreparedDraw> {
        if draw.pass != WorldLodPassClass::Transparent
            || draw.material_contract != WorldLodMaterialContract::TRANSPARENT
        {
            return Err(GalError::invalid_argument(
                "world LOD transparent pass received a non-transparent admitted draw",
            ));
        }
        self.inner.stage_draw(
            gal,
            draw.draw,
            draw.uniforms,
            draw.material_contract,
            lightmap,
            ops,
        )
    }

    pub(crate) fn destroy(&mut self, gal: &mut VulkanicGal) {
        self.inner.destroy(gal);
    }

    pub(crate) fn retain_lightmap_binding(
        &mut self,
        gal: &mut VulkanicGal,
        binding: VanillaLightmapBinding,
    ) {
        self.inner.retain_lightmap_binding(gal, binding);
    }

    pub(crate) fn clear_lightmap_bindings(&mut self, gal: &mut VulkanicGal) {
        self.inner.clear_lightmap_bindings(gal);
    }

    pub(crate) fn reconcile_assets(
        &mut self,
        gal: &mut VulkanicGal,
        assets: &BTreeMap<u64, WorldLodGpuColumnAsset>,
    ) {
        self.inner.reconcile_assets(gal, assets);
    }
}

/// Rust-owned DH water-surface pass. It shares the immutable copied geometry
/// and lightmap ownership with other LOD materials, while retaining DH's
/// water-specific cull/depth/blend policy in its own private pipeline.
pub(crate) struct WorldLodWaterPassResources {
    inner: WorldLodPassResources,
}

impl Default for WorldLodWaterPassResources {
    fn default() -> Self {
        Self {
            inner: WorldLodPassResources::new(WorldLodPassClass::WaterSurface),
        }
    }
}

impl WorldLodWaterPassResources {
    pub(crate) fn stage_draw(
        &mut self,
        gal: &mut VulkanicGal,
        draw: WorldLodWaterDraw,
        lightmap: VanillaLightmapBinding,
        ops: &mut Vec<CommandOp>,
    ) -> GalResult<WorldLodPreparedDraw> {
        if draw.pass != WorldLodPassClass::WaterSurface
            || draw.material_contract != WorldLodMaterialContract::WATER_SURFACE
        {
            return Err(GalError::invalid_argument(
                "world LOD water pass received a non-water admitted draw",
            ));
        }
        self.inner.stage_draw(
            gal,
            draw.draw,
            draw.uniforms,
            draw.material_contract,
            lightmap,
            ops,
        )
    }

    pub(crate) fn destroy(&mut self, gal: &mut VulkanicGal) {
        self.inner.destroy(gal);
    }

    pub(crate) fn retain_lightmap_binding(
        &mut self,
        gal: &mut VulkanicGal,
        binding: VanillaLightmapBinding,
    ) {
        self.inner.retain_lightmap_binding(gal, binding);
    }

    pub(crate) fn clear_lightmap_bindings(&mut self, gal: &mut VulkanicGal) {
        self.inner.clear_lightmap_bindings(gal);
    }

    pub(crate) fn reconcile_assets(
        &mut self,
        gal: &mut VulkanicGal,
        assets: &BTreeMap<u64, WorldLodGpuColumnAsset>,
    ) {
        self.inner.reconcile_assets(gal, assets);
    }
}

/// A generation-bound owned terrain-atlas binding. The frontend derives this
/// from its resource-pack snapshot; Java/DH GL texture state never enters the
/// LOD pass.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WorldLodTerrainAtlasBinding {
    pub mesh_generation: u64,
    pub texture_view: Handle,
    pub sampler: Handle,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct WorldLodExactAtlasPreparedDraw {
    pub pipeline: Handle,
    pub pipeline_layout: Handle,
    pub geometry_resource_set: Handle,
    pub atlas_and_lightmap_resource_set: Handle,
    pub index_buffer: Handle,
    pub index_type: IndexType,
    pub index_count: u32,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct WorldLodExactAtlasBindingKey {
    mesh_generation: u64,
    atlas_view: Handle,
    atlas_sampler: Handle,
    lightmap: WorldLodLightmapResourceKey,
}

struct WorldLodExactAtlasPipelineResources {
    vertex_shader: Handle,
    fragment_shader: Handle,
    geometry_and_frame_layout: Handle,
    atlas_and_lightmap_layout: Handle,
    pipeline_layout: Handle,
    pipeline: Handle,
}

impl WorldLodExactAtlasPipelineResources {
    fn destroy(self, gal: &mut VulkanicGal) {
        for handle in [
            self.pipeline,
            self.pipeline_layout,
            self.atlas_and_lightmap_layout,
            self.geometry_and_frame_layout,
            self.fragment_shader,
            self.vertex_shader,
        ] {
            let _ = gal.destroy(handle);
        }
    }
}

/// Private exact-atlas opaque DH pass. It is intentionally unable to receive
/// incomplete source ranges: callers construct it only from
/// `WorldLodTexturedGpuDraw`, whose source ordinal has already been paired to
/// complete copied material provenance.
#[derive(Default)]
pub(crate) struct WorldLodExactAtlasOpaquePassResources {
    pipeline: Option<WorldLodExactAtlasPipelineResources>,
    draws: BTreeMap<WorldLodDrawResourceKey, WorldLodDrawResources>,
    material_sets: BTreeMap<WorldLodExactAtlasBindingKey, Handle>,
}

impl WorldLodExactAtlasOpaquePassResources {
    pub(crate) fn stage_draw(
        &mut self,
        gal: &mut VulkanicGal,
        draw: WorldLodTexturedGpuDraw,
        uniforms: WorldLodDrawUniform,
        atlas: WorldLodTerrainAtlasBinding,
        lightmap: VanillaLightmapBinding,
        ops: &mut Vec<CommandOp>,
    ) -> GalResult<WorldLodExactAtlasPreparedDraw> {
        if draw.layer != WORLD_LOD_LAYER_OPAQUE {
            return Err(GalError::invalid_argument(
                "world LOD exact-atlas pass accepts opaque segments only",
            ));
        }
        if atlas.mesh_generation == 0
            || lightmap.world_generation == 0
            || lightmap.lightmap_generation == 0
        {
            return Err(GalError::invalid_argument(
                "world LOD exact-atlas pass requires complete atlas and lightmap generations",
            ));
        }
        self.ensure_pipeline(gal)?;
        let key = WorldLodDrawResourceKey {
            column_key: draw.column_key,
            column_generation: draw.column_generation,
            segment_index: draw.source_segment_index,
        };
        if !self.draws.contains_key(&key) {
            let pipeline = self.pipeline.as_ref().expect("exact-atlas pipeline exists");
            let uniform_buffer = gal.create_buffer(BufferDesc {
                label: format!(
                    "world-lod-exact-atlas-column{}-gen{}-segment{}.frame",
                    key.column_key, key.column_generation, key.segment_index
                ),
                size: 128,
                memory: MemoryDomain::Upload,
                usages: vec![BufferUsage::Uniform, BufferUsage::HostWrite],
            })?;
            let resource_set = match gal.create_resource_set(ResourceSetDesc {
                label: format!(
                    "world-lod-exact-atlas-column{}-gen{}-segment{}.geometry-and-frame-set",
                    key.column_key, key.column_generation, key.segment_index
                ),
                layout: pipeline.geometry_and_frame_layout,
                bindings: vec![
                    ResourceBinding {
                        binding: 0,
                        array_index: 0,
                        resource: draw.vertex_buffer,
                        kind: ResourceBindingKind::StorageBuffer,
                        access: AccessFlags::READ,
                        dynamic_offsets: Vec::new(),
                        buffer_range: None,
                    },
                    ResourceBinding {
                        binding: 1,
                        array_index: 0,
                        resource: uniform_buffer,
                        kind: ResourceBindingKind::UniformBuffer,
                        access: AccessFlags::READ,
                        dynamic_offsets: Vec::new(),
                        buffer_range: None,
                    },
                ],
            }) {
                Ok(set) => set,
                Err(error) => {
                    let _ = gal.destroy(uniform_buffer);
                    return Err(error);
                }
            };
            self.draws.insert(
                key,
                WorldLodDrawResources {
                    uniform_buffer,
                    resource_set,
                },
            );
        }
        let resources = self
            .draws
            .get(&key)
            .copied()
            .expect("exact-atlas draw resources exist");
        let material_set = self.ensure_material_set(gal, atlas, lightmap)?;
        ops.extend([
            CommandOp::Barrier(buffer_barrier(
                resources.uniform_buffer,
                TextureUsageState::ShaderRead,
                TextureUsageState::TransferDst,
            )),
            CommandOp::HostWriteBuffer {
                buffer: resources.uniform_buffer,
                offset: 0,
                data: uniforms.pack_std140().to_vec(),
            },
            CommandOp::Barrier(buffer_barrier(
                resources.uniform_buffer,
                TextureUsageState::TransferDst,
                TextureUsageState::ShaderRead,
            )),
        ]);
        let pipeline = self
            .pipeline
            .as_ref()
            .expect("exact-atlas pipeline remains alive");
        Ok(WorldLodExactAtlasPreparedDraw {
            pipeline: pipeline.pipeline,
            pipeline_layout: pipeline.pipeline_layout,
            geometry_resource_set: resources.resource_set,
            atlas_and_lightmap_resource_set: material_set,
            index_buffer: draw.index_buffer,
            index_type: draw.index_type,
            index_count: draw.index_count,
        })
    }

    pub(crate) fn reconcile_assets(
        &mut self,
        gal: &mut VulkanicGal,
        assets: &BTreeMap<u64, WorldLodTexturedGpuColumnAsset>,
    ) {
        let stale = self
            .draws
            .keys()
            .filter(|key| {
                assets.get(&key.column_key).is_none_or(|asset| {
                    asset.column_generation != key.column_generation
                        || !asset
                            .segments
                            .iter()
                            .any(|segment| segment.source_segment_index == key.segment_index)
                })
            })
            .copied()
            .collect::<Vec<_>>();
        for key in stale {
            if let Some(resources) = self.draws.remove(&key) {
                resources.destroy(gal);
            }
        }
    }

    pub(crate) fn retain_bindings(
        &mut self,
        gal: &mut VulkanicGal,
        atlas: WorldLodTerrainAtlasBinding,
        lightmap: VanillaLightmapBinding,
    ) {
        let retained = WorldLodExactAtlasBindingKey {
            mesh_generation: atlas.mesh_generation,
            atlas_view: atlas.texture_view,
            atlas_sampler: atlas.sampler,
            lightmap: lightmap.into(),
        };
        let stale = self
            .material_sets
            .keys()
            .filter(|key| **key != retained)
            .copied()
            .collect::<Vec<_>>();
        for key in stale {
            if let Some(set) = self.material_sets.remove(&key) {
                let _ = gal.destroy(set);
            }
        }
    }

    pub(crate) fn clear_bindings(&mut self, gal: &mut VulkanicGal) {
        for (_, set) in std::mem::take(&mut self.material_sets) {
            let _ = gal.destroy(set);
        }
    }

    pub(crate) fn destroy(&mut self, gal: &mut VulkanicGal) {
        for (_, resources) in std::mem::take(&mut self.draws) {
            resources.destroy(gal);
        }
        self.clear_bindings(gal);
        if let Some(pipeline) = self.pipeline.take() {
            pipeline.destroy(gal);
        }
    }

    fn ensure_pipeline(&mut self, gal: &mut VulkanicGal) -> GalResult<()> {
        if self.pipeline.is_some() {
            return Ok(());
        }
        let label = "world-lod-exact-atlas-opaque";
        let [geometry_and_frame_desc, atlas_and_lightmap_desc] =
            distant_horizons_lod_exact_atlas_resource_layouts(label);
        let mut created = Vec::new();
        let result = (|| -> GalResult<WorldLodExactAtlasPipelineResources> {
            let geometry_and_frame_layout = gal.create_resource_layout(geometry_and_frame_desc)?;
            created.push(geometry_and_frame_layout);
            let atlas_and_lightmap_layout = gal.create_resource_layout(atlas_and_lightmap_desc)?;
            created.push(atlas_and_lightmap_layout);
            let pipeline_layout = gal.create_pipeline_layout(PipelineLayoutDesc {
                label: format!("{label}.pipeline-layout"),
                resource_layouts: vec![geometry_and_frame_layout, atlas_and_lightmap_layout],
            })?;
            created.push(pipeline_layout);
            let [vertex_desc, fragment_desc] =
                minimal_distant_horizons_lod_exact_atlas_opaque_program()
                    .shader_module_descriptors(gal.capabilities().api);
            let vertex_shader = gal.create_shader_module(vertex_desc)?;
            created.push(vertex_shader);
            let fragment_shader = gal.create_shader_module(fragment_desc)?;
            created.push(fragment_shader);
            let pipeline = gal.create_graphics_pipeline(GraphicsPipelineDesc {
                label: format!("{label}.pipeline"),
                layout: pipeline_layout,
                vertex_shader,
                fragment_shader,
                topology: PrimitiveTopology::Triangles,
                cull_mode: CullMode::Back,
                front_face: FrontFace::CounterClockwise,
                provoking_vertex: crate::render::vulkanic::resources::ProvokingVertex::Last,
                raster_y_direction: crate::render::vulkanic::resources::RasterYDirection::Up,
                blend: BlendMode::Disabled,
                depth_compare: Some(CompareOp::LessOrEqual),
                depth_write: true,
                depth_bias: None,
                color_formats: vec![TextureFormat::Rgba8Unorm; 4],
                depth_format: Some(TextureFormat::Depth32Float),
                stencil: None,
            })?;
            created.push(pipeline);
            Ok(WorldLodExactAtlasPipelineResources {
                vertex_shader,
                fragment_shader,
                geometry_and_frame_layout,
                atlas_and_lightmap_layout,
                pipeline_layout,
                pipeline,
            })
        })();
        if result.is_err() {
            for handle in created.into_iter().rev() {
                let _ = gal.destroy(handle);
            }
        }
        self.pipeline = Some(result?);
        Ok(())
    }

    fn ensure_material_set(
        &mut self,
        gal: &mut VulkanicGal,
        atlas: WorldLodTerrainAtlasBinding,
        lightmap: VanillaLightmapBinding,
    ) -> GalResult<Handle> {
        let key = WorldLodExactAtlasBindingKey {
            mesh_generation: atlas.mesh_generation,
            atlas_view: atlas.texture_view,
            atlas_sampler: atlas.sampler,
            lightmap: lightmap.into(),
        };
        if let Some(&set) = self.material_sets.get(&key) {
            return Ok(set);
        }
        let pipeline = self
            .pipeline
            .as_ref()
            .expect("exact-atlas pipeline exists before material set creation");
        let set = gal.create_resource_set(ResourceSetDesc {
            label: format!(
                "world-lod-exact-atlas.mesh{}-lightmap-world{}-gen{}.resource-set",
                key.mesh_generation,
                key.lightmap.world_generation,
                key.lightmap.lightmap_generation
            ),
            layout: pipeline.atlas_and_lightmap_layout,
            bindings: vec![
                ResourceBinding {
                    binding: 0,
                    array_index: 0,
                    resource: atlas.texture_view,
                    kind: ResourceBindingKind::SampledTexture,
                    access: AccessFlags::READ,
                    dynamic_offsets: Vec::new(),
                    buffer_range: None,
                },
                ResourceBinding {
                    binding: 1,
                    array_index: 0,
                    resource: atlas.sampler,
                    kind: ResourceBindingKind::Sampler,
                    access: AccessFlags::READ,
                    dynamic_offsets: Vec::new(),
                    buffer_range: None,
                },
                ResourceBinding {
                    binding: 2,
                    array_index: 0,
                    resource: lightmap.texture_view,
                    kind: ResourceBindingKind::SampledTexture,
                    access: AccessFlags::READ,
                    dynamic_offsets: Vec::new(),
                    buffer_range: None,
                },
                ResourceBinding {
                    binding: 3,
                    array_index: 0,
                    resource: lightmap.sampler,
                    kind: ResourceBindingKind::Sampler,
                    access: AccessFlags::READ,
                    dynamic_offsets: Vec::new(),
                    buffer_range: None,
                },
            ],
        })?;
        self.material_sets.insert(key, set);
        Ok(set)
    }
}

/// Source-frame counterpart of the regular exact-atlas DH pass. It shares
/// immutable copied geometry and Rust-owned atlas/lightmap semantics, then
/// writes the named terrain color, normal, and material outputs required by
/// later selected-source shader-pack stages. The selected DH program still
/// owns unresolved reduced-color ranges; this owner never guesses a texture
/// from a DH material category.
#[derive(Default)]
pub(crate) struct WorldLodExactAtlasSourcePassResources {
    pipelines:
        BTreeMap<WorldLodExactAtlasSourcePipelineKey, WorldLodExactAtlasSourcePipelineResources>,
    draws: BTreeMap<WorldLodExactAtlasSourceDrawKey, WorldLodExactAtlasSourceDrawResources>,
    material_sets: BTreeMap<WorldLodExactAtlasSourceBindingKey, Handle>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct WorldLodExactAtlasSourceDrawKey {
    pipeline: WorldLodExactAtlasSourcePipelineKey,
    draw: WorldLodDrawResourceKey,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct WorldLodExactAtlasSourceBindingKey {
    pipeline: WorldLodExactAtlasSourcePipelineKey,
    mesh_generation: u64,
    atlas_view: Handle,
    atlas_sampler: Handle,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct WorldLodExactAtlasSourcePipelineKey {
    identity: String,
    shader_pack_generation: u64,
    primary_format: TextureFormat,
    pack_resources_layout: Handle,
    front_face: FrontFace,
}

struct WorldLodExactAtlasSourcePipelineResources {
    vertex_shader: Handle,
    fragment_shader: Handle,
    source_data_layout: Handle,
    pack_resources_layout: Handle,
    exact_atlas_layout: Handle,
    pipeline_layout: Handle,
    pipeline: Handle,
}

impl WorldLodExactAtlasSourcePipelineResources {
    fn destroy(self, gal: &mut VulkanicGal) {
        for handle in [
            self.pipeline,
            self.pipeline_layout,
            self.exact_atlas_layout,
            self.source_data_layout,
            self.fragment_shader,
            self.vertex_shader,
        ] {
            let _ = gal.destroy(handle);
        }
    }
}

struct WorldLodExactAtlasSourceDrawResources {
    column_frame_buffer: Handle,
    scalar_uniform_buffer: Option<Handle>,
    source_data_set: Handle,
}

impl WorldLodExactAtlasSourceDrawResources {
    fn destroy(self, gal: &mut VulkanicGal) {
        let _ = gal.destroy(self.source_data_set);
        if let Some(buffer) = self.scalar_uniform_buffer {
            let _ = gal.destroy(buffer);
        }
        let _ = gal.destroy(self.column_frame_buffer);
    }
}

impl WorldLodExactAtlasSourcePassResources {
    pub(crate) fn stage_draw(
        &mut self,
        gal: &mut VulkanicGal,
        program: &LoweredDistantHorizonsExactAtlasSourceProgram,
        color_attachment: &TerrainSourceColorAttachment,
        pack_resources_layout: Handle,
        draw: WorldLodTexturedGpuDraw,
        uniforms: WorldLodDrawUniform,
        atlas: WorldLodTerrainAtlasBinding,
        source_uniforms: &TerrainSourceUniformFrame,
        ops: &mut Vec<CommandOp>,
    ) -> GalResult<WorldLodPreparedSourceDraw> {
        if draw.layer != WORLD_LOD_LAYER_OPAQUE {
            return Err(GalError::invalid_argument(
                "world LOD exact-atlas source pass accepts opaque segments only",
            ));
        }
        if atlas.mesh_generation == 0 {
            return Err(GalError::invalid_argument(
                "world LOD exact-atlas source pass requires a complete atlas generation",
            ));
        }
        let pipeline_key =
            exact_atlas_source_pipeline_key(program, color_attachment, pack_resources_layout)?;
        self.ensure_pipeline(gal, program, &pipeline_key)?;
        let scalar_uniforms = program.source.pack_scalar_uniforms(source_uniforms)?;
        let draw_key = WorldLodExactAtlasSourceDrawKey {
            pipeline: pipeline_key.clone(),
            draw: WorldLodDrawResourceKey {
                column_key: draw.column_key,
                column_generation: draw.column_generation,
                segment_index: draw.source_segment_index,
            },
        };
        if !self.draws.contains_key(&draw_key) {
            let pipeline = self
                .pipelines
                .get(&pipeline_key)
                .expect("exact-atlas source pipeline exists");
            let column_frame_buffer = gal.create_buffer(BufferDesc {
                label: format!(
                    "world-lod-exact-atlas-source-column{}-gen{}-segment{}.frame",
                    draw_key.draw.column_key,
                    draw_key.draw.column_generation,
                    draw_key.draw.segment_index
                ),
                size: 128,
                memory: MemoryDomain::Upload,
                usages: vec![BufferUsage::Uniform, BufferUsage::HostWrite],
            })?;
            let scalar_uniform_buffer =
                if program.source.execution_interface.scalar_uniforms.is_some() {
                    Some(gal.create_buffer(BufferDesc {
                        label: format!(
                            "world-lod-exact-atlas-source-column{}-gen{}-segment{}.scalars",
                            draw_key.draw.column_key,
                            draw_key.draw.column_generation,
                            draw_key.draw.segment_index
                        ),
                        size: u64::from(program.source.execution_interface.scalar_uniform_bytes),
                        memory: MemoryDomain::Upload,
                        usages: vec![BufferUsage::Uniform, BufferUsage::HostWrite],
                    })?)
                } else {
                    None
                };
            let mut bindings = vec![
                ResourceBinding {
                    binding: 0,
                    array_index: 0,
                    resource: draw.vertex_buffer,
                    kind: ResourceBindingKind::StorageBuffer,
                    access: AccessFlags::READ,
                    dynamic_offsets: Vec::new(),
                    buffer_range: None,
                },
                ResourceBinding {
                    binding: 1,
                    array_index: 0,
                    resource: column_frame_buffer,
                    kind: ResourceBindingKind::UniformBuffer,
                    access: AccessFlags::READ,
                    dynamic_offsets: vec![0],
                    buffer_range: Some(u64::from(
                        program.source.execution_interface.column_frame_bytes,
                    )),
                },
            ];
            if let (Some(binding), Some(buffer)) = (
                program.source.execution_interface.scalar_uniforms,
                scalar_uniform_buffer,
            ) {
                bindings.push(ResourceBinding {
                    binding: binding.binding,
                    array_index: 0,
                    resource: buffer,
                    kind: ResourceBindingKind::UniformBuffer,
                    access: AccessFlags::READ,
                    dynamic_offsets: vec![0],
                    buffer_range: Some(u64::from(
                        program.source.execution_interface.scalar_uniform_bytes,
                    )),
                });
            }
            bindings.sort_by_key(|binding| binding.binding);
            let resource_set = match gal.create_resource_set(ResourceSetDesc {
                label: format!(
                    "world-lod-exact-atlas-source-column{}-gen{}-segment{}.geometry-and-frame-set",
                    draw_key.draw.column_key,
                    draw_key.draw.column_generation,
                    draw_key.draw.segment_index
                ),
                layout: pipeline.source_data_layout,
                bindings,
            }) {
                Ok(set) => set,
                Err(error) => {
                    if let Some(buffer) = scalar_uniform_buffer {
                        let _ = gal.destroy(buffer);
                    }
                    let _ = gal.destroy(column_frame_buffer);
                    return Err(error);
                }
            };
            self.draws.insert(
                draw_key.clone(),
                WorldLodExactAtlasSourceDrawResources {
                    column_frame_buffer,
                    scalar_uniform_buffer,
                    source_data_set: resource_set,
                },
            );
        }
        let (column_frame_buffer, scalar_uniform_buffer, source_data_set) = self
            .draws
            .get(&draw_key)
            .map(|resources| {
                (
                    resources.column_frame_buffer,
                    resources.scalar_uniform_buffer,
                    resources.source_data_set,
                )
            })
            .expect("exact-atlas source draw resources exist");
        let atlas_set = self.ensure_material_set(gal, &pipeline_key, atlas)?;
        append_source_uniform_upload(ops, column_frame_buffer, uniforms.pack_std140().to_vec());
        if let Some(buffer) = scalar_uniform_buffer {
            append_source_uniform_upload(ops, buffer, scalar_uniforms);
        }
        let pipeline = self
            .pipelines
            .get(&pipeline_key)
            .expect("exact-atlas source pipeline remains alive");
        Ok(WorldLodPreparedSourceDraw {
            pipeline: pipeline.pipeline,
            pipeline_layout: pipeline.pipeline_layout,
            source_data_set,
            source_data_dynamic_offsets: [0, 0],
            source_data_dynamic_offset_count: if program
                .source
                .execution_interface
                .scalar_uniforms
                .is_some()
            {
                2
            } else {
                1
            },
            pack_resources_layout: pipeline.pack_resources_layout,
            source_extra_resource_set: Some(atlas_set),
            index_buffer: draw.index_buffer,
            index_type: draw.index_type,
            index_count: draw.index_count,
        })
    }

    pub(crate) fn reconcile_assets(
        &mut self,
        gal: &mut VulkanicGal,
        assets: &BTreeMap<u64, WorldLodTexturedGpuColumnAsset>,
    ) {
        let stale =
            self.draws
                .keys()
                .filter(|key| {
                    assets.get(&key.draw.column_key).is_none_or(|asset| {
                        asset.column_generation != key.draw.column_generation
                            || !asset.segments.iter().any(|segment| {
                                segment.source_segment_index == key.draw.segment_index
                            })
                    })
                })
                .cloned()
                .collect::<Vec<_>>();
        for key in stale {
            if let Some(resources) = self.draws.remove(&key) {
                resources.destroy(gal);
            }
        }
    }

    pub(crate) fn retain_bindings(
        &mut self,
        gal: &mut VulkanicGal,
        atlas: WorldLodTerrainAtlasBinding,
    ) {
        let stale = self
            .material_sets
            .keys()
            .filter(|key| {
                key.mesh_generation != atlas.mesh_generation
                    || key.atlas_view != atlas.texture_view
                    || key.atlas_sampler != atlas.sampler
            })
            .cloned()
            .collect::<Vec<_>>();
        for key in stale {
            if let Some(set) = self.material_sets.remove(&key) {
                let _ = gal.destroy(set);
            }
        }
    }

    pub(crate) fn clear_bindings(&mut self, gal: &mut VulkanicGal) {
        for (_, set) in std::mem::take(&mut self.material_sets) {
            let _ = gal.destroy(set);
        }
    }

    pub(crate) fn destroy(&mut self, gal: &mut VulkanicGal) {
        for (_, resources) in std::mem::take(&mut self.draws) {
            resources.destroy(gal);
        }
        self.clear_bindings(gal);
        for (_, pipeline) in std::mem::take(&mut self.pipelines) {
            pipeline.destroy(gal);
        }
    }

    fn ensure_pipeline(
        &mut self,
        gal: &mut VulkanicGal,
        program: &LoweredDistantHorizonsExactAtlasSourceProgram,
        pipeline_key: &WorldLodExactAtlasSourcePipelineKey,
    ) -> GalResult<()> {
        if self.pipelines.contains_key(&pipeline_key) {
            return Ok(());
        }
        let label = format!(
            "world-lod-exact-atlas-source-{}",
            pipeline_key.identity.replace(':', "-")
        );
        let layouts = program.source.execution_resource_layouts()?;
        let mut created = Vec::new();
        let result = (|| -> GalResult<WorldLodExactAtlasSourcePipelineResources> {
            let source_data_layout = gal.create_resource_layout(layouts.source_data)?;
            created.push(source_data_layout);
            let exact_atlas_layout = gal.create_resource_layout(
                distant_horizons_exact_atlas_source_resource_layout(&label),
            )?;
            created.push(exact_atlas_layout);
            let pipeline_layout = gal.create_pipeline_layout(PipelineLayoutDesc {
                label: format!("{label}.pipeline-layout"),
                resource_layouts: vec![
                    source_data_layout,
                    pipeline_key.pack_resources_layout,
                    exact_atlas_layout,
                ],
            })?;
            created.push(pipeline_layout);
            let [vertex_desc, fragment_desc] =
                program.shader_module_descriptors(gal.capabilities().api);
            let vertex_shader = gal.create_shader_module(vertex_desc)?;
            created.push(vertex_shader);
            let fragment_shader = gal.create_shader_module(fragment_desc)?;
            created.push(fragment_shader);
            let pipeline = gal.create_graphics_pipeline(GraphicsPipelineDesc {
                label: format!("{label}.pipeline"),
                layout: pipeline_layout,
                vertex_shader,
                fragment_shader,
                topology: PrimitiveTopology::Triangles,
                cull_mode: CullMode::Back,
                front_face: pipeline_key.front_face,
                provoking_vertex: crate::render::vulkanic::resources::ProvokingVertex::Last,
                raster_y_direction: crate::render::vulkanic::resources::RasterYDirection::Up,
                blend: BlendMode::Disabled,
                depth_compare: Some(CompareOp::LessOrEqual),
                depth_write: true,
                depth_bias: None,
                color_formats: vec![pipeline_key.primary_format],
                depth_format: Some(TextureFormat::Depth32Float),
                stencil: None,
            })?;
            created.push(pipeline);
            Ok(WorldLodExactAtlasSourcePipelineResources {
                vertex_shader,
                fragment_shader,
                source_data_layout,
                pack_resources_layout: pipeline_key.pack_resources_layout,
                exact_atlas_layout,
                pipeline_layout,
                pipeline,
            })
        })();
        if result.is_err() {
            for handle in created.into_iter().rev() {
                let _ = gal.destroy(handle);
            }
        }
        self.pipelines.insert(pipeline_key.clone(), result?);
        Ok(())
    }

    fn ensure_material_set(
        &mut self,
        gal: &mut VulkanicGal,
        pipeline_key: &WorldLodExactAtlasSourcePipelineKey,
        atlas: WorldLodTerrainAtlasBinding,
    ) -> GalResult<Handle> {
        let key = WorldLodExactAtlasSourceBindingKey {
            pipeline: pipeline_key.clone(),
            mesh_generation: atlas.mesh_generation,
            atlas_view: atlas.texture_view,
            atlas_sampler: atlas.sampler,
        };
        if let Some(&set) = self.material_sets.get(&key) {
            return Ok(set);
        }
        let pipeline = self
            .pipelines
            .get(&pipeline_key)
            .expect("exact-atlas source pipeline exists before material set creation");
        let set = gal.create_resource_set(ResourceSetDesc {
            label: format!(
                "world-lod-exact-atlas-source.mesh{}.resource-set",
                key.mesh_generation,
            ),
            layout: pipeline.exact_atlas_layout,
            bindings: vec![
                ResourceBinding {
                    binding: 0,
                    array_index: 0,
                    resource: atlas.texture_view,
                    kind: ResourceBindingKind::SampledTexture,
                    access: AccessFlags::READ,
                    dynamic_offsets: Vec::new(),
                    buffer_range: None,
                },
                ResourceBinding {
                    binding: 1,
                    array_index: 0,
                    resource: atlas.sampler,
                    kind: ResourceBindingKind::Sampler,
                    access: AccessFlags::READ,
                    dynamic_offsets: Vec::new(),
                    buffer_range: None,
                },
            ],
        })?;
        self.material_sets.insert(key, set);
        Ok(set)
    }
}

/// Program identity for one Rust-owned source-derived DH pipeline. The color
/// target format is semantic pass data; native image or framebuffer identity
/// deliberately does not participate in this cache key.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct WorldLodSourceProgramKey {
    identity: String,
    shader_pack_generation: u64,
    color_format: TextureFormat,
    cull_mode: u32,
    front_face: FrontFace,
}

impl WorldLodSourceProgramKey {
    fn from_program(
        program: &LoweredDistantHorizonsSourceProgram,
        color_format: TextureFormat,
    ) -> GalResult<Self> {
        Ok(Self {
            identity: program.identity.as_str().to_string(),
            shader_pack_generation: program.shader_pack_generation,
            color_format,
            // This probe is diagnostic-only. It is deliberately part of the
            // pipeline identity so a cached normal pipeline cannot make an
            // experiment silently ineffective.
            cull_mode: selected_source_raster_probe_cull_mode()? as u32,
            front_face: selected_source_raster_probe_front_face(WORLD_LOD_SOURCE_FRONT_FACE)?,
        })
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct WorldLodSourceDrawKey {
    program: WorldLodSourceProgramKey,
    draw: WorldLodDrawResourceKey,
}

/// Stable, semantic set-one identity for one source-derived DH program. The
/// key intentionally excludes opaque GAL/native resource handles: resource
/// generations prove compatibility while each backend keeps physical binding
/// identity private.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct WorldLodSourcePackKey {
    program: WorldLodSourceProgramKey,
    world_generation: u64,
    resource_generations: Vec<(TerrainSourceResourceRole, u64)>,
}

struct WorldLodSourcePipelineResources {
    vertex_shader: Handle,
    fragment_shader: Handle,
    source_data_layout: Handle,
    pack_resources_layout: Handle,
    pipeline_layout: Handle,
    pipeline: Handle,
}

impl WorldLodSourcePipelineResources {
    fn destroy(self, gal: &mut VulkanicGal) {
        for handle in [
            self.pipeline,
            self.pipeline_layout,
            self.pack_resources_layout,
            self.source_data_layout,
            self.fragment_shader,
            self.vertex_shader,
        ] {
            let _ = gal.destroy(handle);
        }
    }
}

struct WorldLodSourceDrawResources {
    column_frame_buffer: Handle,
    scalar_uniform_buffer: Option<Handle>,
    source_data_set: Handle,
}

impl WorldLodSourceDrawResources {
    fn destroy(self, gal: &mut VulkanicGal) {
        let _ = gal.destroy(self.source_data_set);
        if let Some(buffer) = self.scalar_uniform_buffer {
            let _ = gal.destroy(buffer);
        }
        let _ = gal.destroy(self.column_frame_buffer);
    }
}

/// One explicit draw record for the source-derived DH terrain pass. Set zero
/// is fully Rust-owned here; the caller must separately construct set one
/// from the lowered program's semantic pack-resource plan. That separation
/// keeps source texture/material policy out of column geometry ownership.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WorldLodPreparedSourceDraw {
    pub pipeline: Handle,
    pub pipeline_layout: Handle,
    pub source_data_set: Handle,
    pub source_data_dynamic_offsets: [u32; 2],
    pub source_data_dynamic_offset_count: u8,
    pub pack_resources_layout: Handle,
    /// An exact-atlas source range retains one additional Rust-owned atlas
    /// binding after the selected shader-pack resources. Ordinary reduced DH
    /// draws leave this absent.
    pub source_extra_resource_set: Option<Handle>,
    pub index_buffer: Handle,
    pub index_type: IndexType,
    pub index_count: u32,
}

/// One private render target pairing source-declared terrain outputs with
/// DH's separately owned depth stream. The key is restricted to Rust GAL
/// handles and semantic generations; neither Java/Iris framebuffer state nor
/// a backend-native image identity participates.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct WorldLodSourceTargetKey {
    world_generation: u64,
    shader_pack_generation: u64,
    width: u32,
    height: u32,
    color_attachments: Vec<WorldLodSourceColorAttachmentKey>,
    /// The target owns this exact depth attachment. A source-depth cache can
    /// replace its texture/view while retaining the same world, pack, and
    /// extent, so omitting it would let DH draw into an old attachment while
    /// downstream source stages sample the replacement.
    distant_depth_view: Handle,
}

#[derive(Clone, Copy, Debug)]
struct WorldLodSourceTargetResources {
    target: Handle,
    pass: Handle,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct WorldLodSourceColorAttachmentKey {
    output: TerrainPassOutput,
    source_slot: u32,
    texture: Handle,
    view: Handle,
    format: TextureFormat,
}

/// Explicit pass-local handles for the source-derived DH opaque stage. This
/// record does not own a route decision or draw list; the eventual executor
/// must bind the matching source program and consume the target in its one
/// combined frame submission.
#[derive(Clone, Debug)]
pub(crate) struct WorldLodPreparedSourceTarget {
    pub target: Handle,
    pub pass: Handle,
    pub color_attachments: Vec<TerrainSourceColorAttachment>,
    pub primary_color_texture: Handle,
    pub primary_color_view: Handle,
    pub primary_color_format: TextureFormat,
    pub primary_color_clears_each_frame: bool,
    pub distant_depth_texture: Handle,
    pub distant_depth_view: Handle,
}

fn primary_source_color_attachment(
    color_targets: &ShaderPackColorTargets,
) -> GalResult<TerrainSourceColorAttachment> {
    let target = color_targets.target("primary").ok_or_else(|| {
        GalError::invalid_argument(
            "Distant Horizons source contract has no semantic primary color target",
        )
    })?;
    Ok(TerrainSourceColorAttachment {
        output: TerrainPassOutput::LitTerrainColor,
        source_slot: target.source_slot,
        role: TerrainSourceResourceRole::ShaderPackColor("primary".to_string()),
        texture: target.current_texture,
        view: target.current_attachment_view,
        format: target.format,
        clear_each_frame: target.clear_each_frame,
        clear_color_bits: target.clear_color_bits,
    })
}

fn exact_atlas_source_pipeline_key(
    program: &LoweredDistantHorizonsExactAtlasSourceProgram,
    attachment: &TerrainSourceColorAttachment,
    pack_resources_layout: Handle,
) -> GalResult<WorldLodExactAtlasSourcePipelineKey> {
    if attachment.output != TerrainPassOutput::LitTerrainColor {
        return Err(GalError::invalid_argument(
            "world LOD exact-atlas source pass requires the named DH primary color attachment",
        ));
    }
    Ok(WorldLodExactAtlasSourcePipelineKey {
        identity: program.source.identity.as_str().to_string(),
        shader_pack_generation: program.source.shader_pack_generation,
        primary_format: attachment.format,
        pack_resources_layout,
        front_face: selected_source_raster_probe_front_face(WORLD_LOD_SOURCE_FRONT_FACE)?,
    })
}

/// Private Rust owner for the source-derived DH column ABI. It is deliberately
/// not a route selector and owns no render target or pack-resource set: those
/// belong to the shader-pack runtime's named target/resource transaction.
///
/// The owner is nevertheless production-grade in the narrow sense that its
/// pipelines, buffers, resource layouts, and update barriers are explicit
/// VulkanicGAL objects reusable by both Rust backends.
#[derive(Default)]
pub(crate) struct WorldLodSourcePassResources {
    pipelines: BTreeMap<WorldLodSourceProgramKey, WorldLodSourcePipelineResources>,
    draws: BTreeMap<WorldLodSourceDrawKey, WorldLodSourceDrawResources>,
    pack_resources: BTreeMap<WorldLodSourcePackKey, Handle>,
    targets: BTreeMap<WorldLodSourceTargetKey, WorldLodSourceTargetResources>,
}

impl WorldLodSourcePassResources {
    /// Stages the shared terrain-compatible render target used by source-derived
    /// DH phases. Named pack colors remain runtime-owned; this owner joins
    /// their terrain output schema to DH's distinct depth attachment.
    pub(crate) fn stage_target(
        &mut self,
        gal: &mut VulkanicGal,
        color_targets: &ShaderPackColorTargets,
        depth_targets: WorldLodSourceTargets,
    ) -> GalResult<WorldLodPreparedSourceTarget> {
        if color_targets.identity.world_generation != depth_targets.identity.world_generation
            || color_targets.identity.shader_pack_generation
                != depth_targets.identity.shader_pack_generation
            || color_targets.identity.extent != depth_targets.identity.extent
        {
            return Err(GalError::invalid_argument(
                "Distant Horizons source color and depth target generations do not match",
            ));
        }
        let primary = primary_source_color_attachment(color_targets)?;
        self.stage_target_attachments(gal, vec![primary.clone()], primary, depth_targets)
    }

    fn stage_target_attachments(
        &mut self,
        gal: &mut VulkanicGal,
        color_attachments: Vec<TerrainSourceColorAttachment>,
        primary: TerrainSourceColorAttachment,
        depth_targets: WorldLodSourceTargets,
    ) -> GalResult<WorldLodPreparedSourceTarget> {
        if color_attachments.is_empty() {
            return Err(GalError::invalid_argument(
                "Distant Horizons source target requires at least one named color attachment",
            ));
        }
        let key = WorldLodSourceTargetKey {
            world_generation: depth_targets.identity.world_generation,
            shader_pack_generation: depth_targets.identity.shader_pack_generation,
            width: depth_targets.identity.extent.width,
            height: depth_targets.identity.extent.height,
            color_attachments: color_attachments
                .iter()
                .map(|attachment| WorldLodSourceColorAttachmentKey {
                    output: attachment.output,
                    source_slot: attachment.source_slot,
                    texture: attachment.texture,
                    view: attachment.view,
                    format: attachment.format,
                })
                .collect(),
            distant_depth_view: depth_targets.distant_depth_view,
        };
        if !self.targets.contains_key(&key) {
            let stale = self
                .targets
                .keys()
                .filter(|existing| {
                    existing.world_generation != key.world_generation
                        || existing.shader_pack_generation != key.shader_pack_generation
                        || existing.width != key.width
                        || existing.height != key.height
                        || existing.distant_depth_view != key.distant_depth_view
                })
                .cloned()
                .collect::<Vec<_>>();
            for stale_key in stale {
                if let Some(resources) = self.targets.remove(&stale_key) {
                    let _ = gal.destroy(resources.pass);
                    let _ = gal.destroy(resources.target);
                }
            }
            let target = gal.create_render_target(RenderTargetDesc {
                label: format!(
                    "world-lod-source.world{}-pack{}.opaque-target",
                    key.world_generation, key.shader_pack_generation
                ),
                color_views: color_attachments
                    .iter()
                    .map(|attachment| attachment.view)
                    .collect(),
                depth_stencil_view: Some(depth_targets.distant_depth_view),
                extent: depth_targets.identity.extent,
            })?;
            let pass = match gal.create_render_pass(RenderPassDesc {
                label: format!(
                    "world-lod-source.world{}-pack{}.opaque-pass",
                    key.world_generation, key.shader_pack_generation
                ),
                target,
                color_formats: color_attachments
                    .iter()
                    .map(|attachment| attachment.format)
                    .collect(),
                depth_format: Some(TextureFormat::Depth32Float),
            }) {
                Ok(pass) => pass,
                Err(error) => {
                    let _ = gal.destroy(target);
                    return Err(error);
                }
            };
            self.targets
                .insert(key.clone(), WorldLodSourceTargetResources { target, pass });
        }
        let resources = self
            .targets
            .get(&key)
            .expect("DH source target exists after successful staging");
        Ok(WorldLodPreparedSourceTarget {
            target: resources.target,
            pass: resources.pass,
            color_attachments,
            primary_color_texture: primary.texture,
            primary_color_view: primary.view,
            primary_color_format: primary.format,
            primary_color_clears_each_frame: primary.clear_each_frame,
            distant_depth_texture: depth_targets.distant_depth_texture,
            distant_depth_view: depth_targets.distant_depth_view,
        })
    }

    /// Records one source-derived DH draw using only prepared Rust-owned
    /// target and descriptor sets. The caller owns phase ordering and chooses
    /// whether the depth image remains attached for an immediate snapshot or
    /// transitions back to shader-read after a late translucent pass.
    pub(crate) fn append_draw(
        target: &WorldLodPreparedSourceTarget,
        draw: WorldLodPreparedSourceDraw,
        pack_resources: Handle,
        fog_color: crate::render::vulkanic::commands::ClearColor,
        // The combined source-frame scheduler owns the one initial clear of
        // a shared named color target. DH may be the first writer, or it may
        // follow ordinary terrain, so this cannot be inferred from the DH
        // target alone.
        clear_primary_color: bool,
        color_before: TextureUsageState,
        depth_before: TextureUsageState,
        depth_after: Option<TextureUsageState>,
        ops: &mut Vec<CommandOp>,
    ) -> GalResult<()> {
        if pack_resources.kind() != Some(crate::render::vulkanic::handles::HandleKind::ResourceSet)
        {
            return Err(GalError::invalid_argument(
                "Distant Horizons source opaque draw requires a GAL pack resource-set handle",
            ));
        }
        if draw.source_data_dynamic_offset_count > 2 {
            return Err(GalError::invalid_argument(
                "Distant Horizons source opaque draw has an invalid set-zero dynamic-offset count",
            ));
        }
        if color_before == TextureUsageState::ColorAttachment
            || depth_before == TextureUsageState::DepthStencilAttachment
        {
            return Err(GalError::invalid_argument(
                "Distant Horizons source opaque draw cannot begin while a target attachment is already in a pass",
            ));
        }
        if color_before == TextureUsageState::Undefined && !clear_primary_color {
            return Err(GalError::invalid_argument(
                "Distant Horizons source primary color is undefined but the combined source frame did not schedule its initial clear",
            ));
        }
        if clear_primary_color && !target.primary_color_clears_each_frame {
            return Err(GalError::invalid_argument(
                "Distant Horizons source primary color clear conflicts with the shader-pack target declaration",
            ));
        }
        if clear_primary_color && color_before != TextureUsageState::ShaderRead {
            return Err(GalError::invalid_argument(
                "Distant Horizons source primary color can clear only from the source-frame shader-read boundary",
            ));
        }
        for attachment in &target.color_attachments {
            ops.push(CommandOp::Barrier(texture_barrier(
                attachment.texture,
                color_before,
                TextureUsageState::ColorAttachment,
            )));
        }
        ops.push(CommandOp::Barrier(texture_barrier(
            target.distant_depth_texture,
            depth_before,
            TextureUsageState::DepthStencilAttachment,
        )));
        ops.push(CommandOp::BeginPass {
            pass: target.pass,
            target: target.target,
            colors: target
                .color_attachments
                .iter()
                .map(|attachment| PassAttachment {
                    view: attachment.view,
                    load_op: if clear_primary_color
                        && attachment.output == TerrainPassOutput::LitTerrainColor
                    {
                        AttachmentLoadOp::Clear
                    } else {
                        AttachmentLoadOp::Load
                    },
                    store_op: AttachmentStoreOp::Store,
                    clear_color: (clear_primary_color
                        && attachment.output == TerrainPassOutput::LitTerrainColor)
                        .then(|| {
                            source_color_clear_color(
                                attachment.source_slot,
                                attachment.clear_color_bits,
                                fog_color,
                            )
                        }),
                })
                .collect(),
            depth_stencil: Some(PassAttachment {
                view: target.distant_depth_view,
                load_op: if depth_before == TextureUsageState::Undefined {
                    AttachmentLoadOp::Clear
                } else {
                    AttachmentLoadOp::Load
                },
                store_op: AttachmentStoreOp::Store,
                clear_color: None,
            }),
        });
        ops.push(CommandOp::BindGraphicsPipeline(draw.pipeline));
        ops.push(CommandOp::BindResourceSet {
            pipeline_layout: draw.pipeline_layout,
            set_index: 0,
            set: draw.source_data_set,
            dynamic_offsets: draw.source_data_dynamic_offsets
                [..usize::from(draw.source_data_dynamic_offset_count)]
                .iter()
                .copied()
                .map(u64::from)
                .collect(),
        });
        ops.push(CommandOp::BindResourceSet {
            pipeline_layout: draw.pipeline_layout,
            set_index: 1,
            set: pack_resources,
            dynamic_offsets: Vec::new(),
        });
        if let Some(extra_resources) = draw.source_extra_resource_set {
            ops.push(CommandOp::BindResourceSet {
                pipeline_layout: draw.pipeline_layout,
                set_index: 2,
                set: extra_resources,
                dynamic_offsets: Vec::new(),
            });
        }
        ops.push(CommandOp::SetIndexBuffer {
            buffer: draw.index_buffer,
            offset: 0,
            index_type: draw.index_type,
        });
        ops.push(CommandOp::DrawIndexed {
            indices: draw.index_count,
            instances: 1,
        });
        ops.push(CommandOp::EndPass);
        if let Some(depth_after) = depth_after {
            ops.push(CommandOp::Barrier(texture_barrier(
                target.distant_depth_texture,
                TextureUsageState::DepthStencilAttachment,
                depth_after,
            )));
        }
        // Downstream source-derived passes consume the same named target as a
        // semantic sampled resource. Make that dependency explicit instead
        // of leaving the target in an attachment state for a backend to infer.
        for attachment in &target.color_attachments {
            ops.push(CommandOp::Barrier(texture_barrier(
                attachment.texture,
                TextureUsageState::ColorAttachment,
                TextureUsageState::ShaderRead,
            )));
        }
        Ok(())
    }

    pub(crate) fn stage_draw(
        &mut self,
        gal: &mut VulkanicGal,
        program: &LoweredDistantHorizonsSourceProgram,
        color_format: TextureFormat,
        draw: WorldLodGpuDraw,
        column_frame: WorldLodDrawUniform,
        source_uniforms: &TerrainSourceUniformFrame,
        ops: &mut Vec<CommandOp>,
    ) -> GalResult<WorldLodPreparedSourceDraw> {
        program.execution_interface.validate()?;
        if draw.index_count == 0 || draw.index_count % 3 != 0 {
            return Err(GalError::invalid_argument(
                "source-derived Distant Horizons draw requires triangle-aligned indices",
            ));
        }
        let key = WorldLodSourceProgramKey::from_program(program, color_format)?;
        self.ensure_pipeline(gal, program, &key)?;
        let scalar_uniforms = program.pack_scalar_uniforms(source_uniforms)?;
        if program.execution_interface.scalar_uniforms.is_none() && !scalar_uniforms.is_empty() {
            return Err(GalError::invalid_argument(
                "source-derived Distant Horizons program has no scalar binding but received scalar bytes",
            ));
        }
        let draw_key = WorldLodSourceDrawKey {
            program: key.clone(),
            draw: WorldLodDrawResourceKey::from_draw(draw),
        };
        let created_draw_resources = !self.draws.contains_key(&draw_key);
        if created_draw_resources {
            let pipeline = self
                .pipelines
                .get(&key)
                .expect("source DH pipeline exists after successful initialization");
            let mut created = Vec::new();
            let result = (|| -> GalResult<WorldLodSourceDrawResources> {
                let column_frame_buffer = gal.create_buffer(BufferDesc {
                    label: format!(
                        "world-lod-source-column{}-gen{}-segment{}.frame",
                        draw.column_key, draw.column_generation, draw.segment_index
                    ),
                    size: u64::from(program.execution_interface.column_frame_bytes),
                    memory: MemoryDomain::Upload,
                    usages: vec![BufferUsage::Uniform, BufferUsage::HostWrite],
                })?;
                created.push(column_frame_buffer);
                let scalar_uniform_buffer = if program.execution_interface.scalar_uniforms.is_some()
                {
                    let buffer = gal.create_buffer(BufferDesc {
                        label: format!(
                            "world-lod-source-column{}-gen{}-segment{}.scalars",
                            draw.column_key, draw.column_generation, draw.segment_index
                        ),
                        size: u64::from(program.execution_interface.scalar_uniform_bytes),
                        memory: MemoryDomain::Upload,
                        usages: vec![BufferUsage::Uniform, BufferUsage::HostWrite],
                    })?;
                    created.push(buffer);
                    Some(buffer)
                } else {
                    None
                };
                let mut bindings = vec![
                    ResourceBinding {
                        binding: program.execution_interface.vertex_stream.binding,
                        array_index: 0,
                        resource: draw.vertex_buffer,
                        kind: ResourceBindingKind::StorageBuffer,
                        access: AccessFlags::READ,
                        dynamic_offsets: Vec::new(),
                        buffer_range: None,
                    },
                    ResourceBinding {
                        binding: program.execution_interface.column_frame.binding,
                        array_index: 0,
                        resource: column_frame_buffer,
                        kind: ResourceBindingKind::UniformBuffer,
                        access: AccessFlags::READ,
                        dynamic_offsets: vec![0],
                        buffer_range: Some(u64::from(
                            program.execution_interface.column_frame_bytes,
                        )),
                    },
                ];
                if let (Some(binding), Some(buffer)) = (
                    program.execution_interface.scalar_uniforms,
                    scalar_uniform_buffer,
                ) {
                    bindings.push(ResourceBinding {
                        binding: binding.binding,
                        array_index: 0,
                        resource: buffer,
                        kind: ResourceBindingKind::UniformBuffer,
                        access: AccessFlags::READ,
                        dynamic_offsets: vec![0],
                        buffer_range: Some(u64::from(
                            program.execution_interface.scalar_uniform_bytes,
                        )),
                    });
                }
                bindings.sort_by_key(|binding| binding.binding);
                let source_data_set = gal.create_resource_set(ResourceSetDesc {
                    label: format!(
                        "world-lod-source-column{}-gen{}-segment{}.set-zero",
                        draw.column_key, draw.column_generation, draw.segment_index
                    ),
                    layout: pipeline.source_data_layout,
                    bindings,
                })?;
                created.push(source_data_set);
                Ok(WorldLodSourceDrawResources {
                    column_frame_buffer,
                    scalar_uniform_buffer,
                    source_data_set,
                })
            })();
            if result.is_err() {
                for handle in created.into_iter().rev() {
                    let _ = gal.destroy(handle);
                }
            }
            self.draws.insert(draw_key.clone(), result?);
        }
        let resources = self
            .draws
            .get(&draw_key)
            .expect("source DH draw resources exist after successful staging");
        append_source_uniform_upload(
            ops,
            resources.column_frame_buffer,
            column_frame.pack_std140().to_vec(),
        );
        if let Some(buffer) = resources.scalar_uniform_buffer {
            append_source_uniform_upload(ops, buffer, scalar_uniforms);
        }
        let pipeline = self
            .pipelines
            .get(&key)
            .expect("source DH pipeline remains alive while draw resources are live");
        Ok(WorldLodPreparedSourceDraw {
            pipeline: pipeline.pipeline,
            pipeline_layout: pipeline.pipeline_layout,
            source_data_set: resources.source_data_set,
            source_data_dynamic_offsets: [0, 0],
            source_data_dynamic_offset_count: if program
                .execution_interface
                .scalar_uniforms
                .is_some()
            {
                2
            } else {
                1
            },
            pack_resources_layout: pipeline.pack_resources_layout,
            source_extra_resource_set: None,
            index_buffer: draw.index_buffer,
            index_type: draw.index_type,
            index_count: draw.index_count,
        })
    }

    /// Materializes the source program's named semantic sampler/storage set
    /// after set-zero data preparation. This remains separate from draw
    /// recording and target selection, so incomplete pack resources reject
    /// before the DH route can issue any command.
    pub(crate) fn stage_pack_resources(
        &mut self,
        gal: &mut VulkanicGal,
        program: &LoweredDistantHorizonsSourceProgram,
        color_format: TextureFormat,
        resources: &TerrainSourceOwnedResourceSet,
    ) -> GalResult<Handle> {
        program.require_semantic_resources(resources.availability())?;
        let world_generation = resources.availability().world_generation();
        if world_generation == 0 {
            return Err(GalError::invalid_argument(
                "Distant Horizons source pack resources require a non-zero world generation",
            ));
        }
        let program_key = WorldLodSourceProgramKey::from_program(program, color_format)?;
        self.ensure_pipeline(gal, program, &program_key)?;
        let key = WorldLodSourcePackKey {
            program: program_key.clone(),
            world_generation,
            resource_generations: resources.generation_signature(),
        };
        if let Some(set) = self.pack_resources.get(&key) {
            return Ok(*set);
        }
        let stale = self
            .pack_resources
            .keys()
            .filter(|existing| {
                existing.program == key.program
                    && existing.world_generation == key.world_generation
                    && existing.resource_generations != key.resource_generations
            })
            .cloned()
            .collect::<Vec<_>>();
        for stale_key in stale {
            if let Some(set) = self.pack_resources.remove(&stale_key) {
                let _ = gal.destroy(set);
            }
        }
        let layout = self
            .pipelines
            .get(&program_key)
            .expect("source DH pipeline exists after successful initialization")
            .pack_resources_layout;
        let set = gal.create_resource_set(program.pack_resource_set_desc(
            format!(
                "world-lod-source-{}-pack{}-world{}.set-one",
                program_key.identity.replace(':', "-"),
                program_key.shader_pack_generation,
                world_generation,
            ),
            layout,
            resources,
        )?)?;
        self.pack_resources.insert(key, set);
        Ok(set)
    }

    /// Returns the layout owned by the normal source pipeline for semantic
    /// pack resources. Source-derived variants must share this exact layout
    /// handle with the set-one resource set, rather than recreating an
    /// equivalent layout with a distinct GAL identity.
    pub(crate) fn pack_resources_layout(
        &mut self,
        gal: &mut VulkanicGal,
        program: &LoweredDistantHorizonsSourceProgram,
        color_format: TextureFormat,
    ) -> GalResult<Handle> {
        let key = WorldLodSourceProgramKey::from_program(program, color_format)?;
        self.ensure_pipeline(gal, program, &key)?;
        Ok(self
            .pipelines
            .get(&key)
            .expect("source DH pipeline exists after successful initialization")
            .pack_resources_layout)
    }

    pub(crate) fn reconcile_assets(
        &mut self,
        gal: &mut VulkanicGal,
        assets: &BTreeMap<u64, WorldLodGpuColumnAsset>,
    ) {
        let stale = self
            .draws
            .keys()
            .filter(|key| {
                assets.get(&key.draw.column_key).is_none_or(|asset| {
                    asset.column_generation != key.draw.column_generation
                        || asset
                            .segments
                            .get(key.draw.segment_index as usize)
                            .is_none()
                })
            })
            .cloned()
            .collect::<Vec<_>>();
        for key in stale {
            if let Some(resources) = self.draws.remove(&key) {
                resources.destroy(gal);
            }
        }
    }

    pub(crate) fn destroy(&mut self, gal: &mut VulkanicGal) {
        for (_, resources) in std::mem::take(&mut self.draws) {
            resources.destroy(gal);
        }
        for (_, set) in std::mem::take(&mut self.pack_resources) {
            let _ = gal.destroy(set);
        }
        for (_, resources) in std::mem::take(&mut self.targets) {
            let _ = gal.destroy(resources.pass);
            let _ = gal.destroy(resources.target);
        }
        for (_, resources) in std::mem::take(&mut self.pipelines) {
            resources.destroy(gal);
        }
    }

    fn ensure_pipeline(
        &mut self,
        gal: &mut VulkanicGal,
        program: &LoweredDistantHorizonsSourceProgram,
        key: &WorldLodSourceProgramKey,
    ) -> GalResult<()> {
        if self.pipelines.contains_key(key) {
            return Ok(());
        }
        let layouts = program.execution_resource_layouts()?;
        let label = format!(
            "world-lod-source-{}-gen{}",
            key.identity.replace(':', "-"),
            key.shader_pack_generation
        );
        let mut created = Vec::new();
        let result = (|| -> GalResult<WorldLodSourcePipelineResources> {
            let source_data_layout = gal.create_resource_layout(layouts.source_data)?;
            created.push(source_data_layout);
            let pack_resources_layout = gal.create_resource_layout(layouts.pack_resources)?;
            created.push(pack_resources_layout);
            let pipeline_layout = gal.create_pipeline_layout(PipelineLayoutDesc {
                label: format!("{label}.pipeline-layout"),
                resource_layouts: vec![source_data_layout, pack_resources_layout],
            })?;
            created.push(pipeline_layout);
            let [vertex_desc, fragment_desc] =
                program.shader_module_descriptors(gal.capabilities().api);
            let vertex_shader = gal.create_shader_module(vertex_desc)?;
            created.push(vertex_shader);
            let fragment_shader = gal.create_shader_module(fragment_desc)?;
            created.push(fragment_shader);
            let (blend, depth_write) = match (program.pass_kind, program.translucent_blend) {
                (DistantHorizonsPassKind::Opaque, None) => (BlendMode::Disabled, true),
                (DistantHorizonsPassKind::Translucent, Some(_)) => (BlendMode::Alpha, false),
                (DistantHorizonsPassKind::Opaque, Some(_)) => {
                    return Err(GalError::invalid_argument(
                        "opaque Distant Horizons source pipeline cannot carry translucent blend semantics",
                    ));
                }
                (DistantHorizonsPassKind::Translucent, None) => {
                    return Err(GalError::invalid_argument(
                        "translucent Distant Horizons source pipeline requires explicit source blend semantics",
                    ));
                }
            };
            let pipeline = gal.create_graphics_pipeline(GraphicsPipelineDesc {
                label: format!("{label}.pipeline"),
                layout: pipeline_layout,
                vertex_shader,
                fragment_shader,
                topology: PrimitiveTopology::Triangles,
                cull_mode: match key.cull_mode {
                    value if value == CullMode::None as u32 => CullMode::None,
                    value if value == CullMode::Front as u32 => CullMode::Front,
                    value if value == CullMode::Back as u32 => CullMode::Back,
                    _ => {
                        unreachable!("source DH pipeline cache key only permits a valid cull mode")
                    }
                },
                front_face: key.front_face,
                provoking_vertex: crate::render::vulkanic::resources::ProvokingVertex::Last,
                raster_y_direction: crate::render::vulkanic::resources::RasterYDirection::Up,
                blend,
                depth_compare: Some(CompareOp::LessOrEqual),
                depth_write,
                depth_bias: None,
                color_formats: vec![key.color_format],
                depth_format: Some(TextureFormat::Depth32Float),
                stencil: None,
            })?;
            created.push(pipeline);
            Ok(WorldLodSourcePipelineResources {
                vertex_shader,
                fragment_shader,
                source_data_layout,
                pack_resources_layout,
                pipeline_layout,
                pipeline,
            })
        })();
        if result.is_err() {
            for handle in created.into_iter().rev() {
                let _ = gal.destroy(handle);
            }
        }
        self.pipelines.insert(key.clone(), result?);
        Ok(())
    }
}

fn append_source_uniform_upload(ops: &mut Vec<CommandOp>, buffer: Handle, data: Vec<u8>) {
    ops.push(CommandOp::Barrier(buffer_barrier(
        buffer,
        TextureUsageState::ShaderRead,
        TextureUsageState::TransferDst,
    )));
    ops.push(CommandOp::HostWriteBuffer {
        buffer,
        offset: 0,
        data,
    });
    ops.push(CommandOp::Barrier(buffer_barrier(
        buffer,
        TextureUsageState::TransferDst,
        TextureUsageState::ShaderRead,
    )));
}

/// Semantic identity for the Rust-owned distant-depth target consumed by a
/// future source-derived Distant Horizons stage. It deliberately contains no
/// frame target, native image, GL framebuffer, or backend handle: selected DH
/// source writes the shader pack's shared primary color target, while later
/// pack stages sample its distinct distant depth.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WorldLodSourceTargetIdentity {
    pub world_generation: u64,
    pub shader_pack_generation: u64,
    pub extent: Extent3d,
}

impl WorldLodSourceTargetIdentity {
    pub(crate) fn validate(self) -> GalResult<()> {
        if self.world_generation == 0 || self.shader_pack_generation == 0 {
            return Err(GalError::invalid_argument(
                "Distant Horizons source target requires non-zero world and shader-pack generations",
            ));
        }
        if self.extent.width == 0 || self.extent.height == 0 || self.extent.depth != 1 {
            return Err(GalError::invalid_argument(
                "Distant Horizons source target requires a non-zero 2D extent",
            ));
        }
        Ok(())
    }
}

/// Backend-neutral handles owned wholly by Rust for one source-derived DH
/// depth generation. The source program and shared pack-color target are not
/// created or selected here.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WorldLodSourceTargets {
    pub identity: WorldLodSourceTargetIdentity,
    /// Monotonic Rust-owned allocation generation. This distinguishes a
    /// replacement depth image from an earlier allocation with the same
    /// world, pack, and extent without exposing a native image identity.
    resource_generation: u64,
    pub distant_depth_texture: Handle,
    pub distant_depth_view: Handle,
    distant_depth_sampler: Handle,
    distant_depth_combined_sampler: Handle,
    /// A depth-only pass used to establish the far-depth stream on a frame
    /// with no visible DH opaque ranges. It is an explicit semantic clear,
    /// never an alias of near-terrain depth.
    distant_depth_clear_target: Handle,
    distant_depth_clear_pass: Handle,
    /// A distinct snapshot of the opaque DH depth stream. Shader-pack stages
    /// may read it after the opaque pass while later work writes or samples
    /// the live target, so it must never alias `distant_depth_texture`.
    pub distant_depth_before_translucency_texture: Handle,
    pub distant_depth_before_translucency_view: Handle,
    distant_depth_before_translucency_sampler: Handle,
    distant_depth_before_translucency_combined_sampler: Handle,
}

impl WorldLodSourceTargets {
    fn create(
        gal: &mut VulkanicGal,
        identity: WorldLodSourceTargetIdentity,
        resource_generation: u64,
    ) -> GalResult<Self> {
        identity.validate()?;
        if resource_generation == 0 {
            return Err(GalError::invalid_argument(
                "Distant Horizons source targets require a non-zero allocation generation",
            ));
        }
        let label = format!(
            "world-lod-source.world{}-pack{}-{}x{}",
            identity.world_generation,
            identity.shader_pack_generation,
            identity.extent.width,
            identity.extent.height,
        );
        let mut created = Vec::new();
        let result = (|| -> GalResult<Self> {
            let distant_depth_texture = gal.create_texture(TextureDesc {
                label: format!("{label}.distant-depth.texture"),
                dimension: TextureDimension::D2,
                format: TextureFormat::Depth32Float,
                extent: identity.extent,
                mip_levels: 1,
                array_layers: 1,
                usages: vec![
                    TextureUsage::DepthStencilAttachment,
                    TextureUsage::Sampled,
                    TextureUsage::TransferSrc,
                ],
            })?;
            created.push(distant_depth_texture);
            let distant_depth_view = gal.create_texture_view(TextureViewDesc {
                label: format!("{label}.distant-depth.view"),
                texture: distant_depth_texture,
                format: TextureFormat::Depth32Float,
                base_mip: 0,
                mip_count: 1,
                base_layer: 0,
                layer_count: 1,
            })?;
            created.push(distant_depth_view);
            let distant_depth_sampler = gal.create_sampler(SamplerDesc {
                label: format!("{label}.distant-depth.sampler"),
                min_filter: SamplerFilter::Nearest,
                mag_filter: SamplerFilter::Nearest,
                mip_filter: SamplerFilter::Nearest,
                address_u: SamplerAddressMode::ClampToEdge,
                address_v: SamplerAddressMode::ClampToEdge,
                address_w: SamplerAddressMode::ClampToEdge,
                comparison: None,
            })?;
            created.push(distant_depth_sampler);
            let distant_depth_combined_sampler =
                gal.create_combined_texture_sampler(CombinedTextureSamplerDesc {
                    label: format!("{label}.distant-depth.combined-sampler"),
                    texture_view: distant_depth_view,
                    sampler: distant_depth_sampler,
                })?;
            created.push(distant_depth_combined_sampler);
            let distant_depth_clear_target = gal.create_render_target(RenderTargetDesc {
                label: format!("{label}.distant-depth.clear-target"),
                color_views: Vec::new(),
                depth_stencil_view: Some(distant_depth_view),
                extent: identity.extent,
            })?;
            created.push(distant_depth_clear_target);
            let distant_depth_clear_pass = match gal.create_render_pass(RenderPassDesc {
                label: format!("{label}.distant-depth.clear-pass"),
                target: distant_depth_clear_target,
                color_formats: Vec::new(),
                depth_format: Some(TextureFormat::Depth32Float),
            }) {
                Ok(pass) => pass,
                Err(error) => return Err(error),
            };
            created.push(distant_depth_clear_pass);
            let distant_depth_before_translucency_texture = gal.create_texture(TextureDesc {
                label: format!("{label}.distant-depth-before-translucency.texture"),
                dimension: TextureDimension::D2,
                format: TextureFormat::Depth32Float,
                extent: identity.extent,
                mip_levels: 1,
                array_layers: 1,
                usages: vec![TextureUsage::Sampled, TextureUsage::TransferDst],
            })?;
            created.push(distant_depth_before_translucency_texture);
            let distant_depth_before_translucency_view =
                gal.create_texture_view(TextureViewDesc {
                    label: format!("{label}.distant-depth-before-translucency.view"),
                    texture: distant_depth_before_translucency_texture,
                    format: TextureFormat::Depth32Float,
                    base_mip: 0,
                    mip_count: 1,
                    base_layer: 0,
                    layer_count: 1,
                })?;
            created.push(distant_depth_before_translucency_view);
            let distant_depth_before_translucency_sampler = gal.create_sampler(SamplerDesc {
                label: format!("{label}.distant-depth-before-translucency.sampler"),
                min_filter: SamplerFilter::Nearest,
                mag_filter: SamplerFilter::Nearest,
                mip_filter: SamplerFilter::Nearest,
                address_u: SamplerAddressMode::ClampToEdge,
                address_v: SamplerAddressMode::ClampToEdge,
                address_w: SamplerAddressMode::ClampToEdge,
                comparison: None,
            })?;
            created.push(distant_depth_before_translucency_sampler);
            let distant_depth_before_translucency_combined_sampler = gal
                .create_combined_texture_sampler(CombinedTextureSamplerDesc {
                    label: format!("{label}.distant-depth-before-translucency.combined-sampler"),
                    texture_view: distant_depth_before_translucency_view,
                    sampler: distant_depth_before_translucency_sampler,
                })?;
            created.push(distant_depth_before_translucency_combined_sampler);
            Ok(Self {
                identity,
                resource_generation,
                distant_depth_texture,
                distant_depth_view,
                distant_depth_sampler,
                distant_depth_combined_sampler,
                distant_depth_clear_target,
                distant_depth_clear_pass,
                distant_depth_before_translucency_texture,
                distant_depth_before_translucency_view,
                distant_depth_before_translucency_sampler,
                distant_depth_before_translucency_combined_sampler,
            })
        })();
        if result.is_err() {
            for handle in created.into_iter().rev() {
                let _ = gal.destroy(handle);
            }
        }
        result
    }

    fn destroy(self, gal: &mut VulkanicGal) {
        for handle in [
            self.distant_depth_before_translucency_combined_sampler,
            self.distant_depth_before_translucency_sampler,
            self.distant_depth_before_translucency_view,
            self.distant_depth_before_translucency_texture,
            self.distant_depth_clear_pass,
            self.distant_depth_clear_target,
            self.distant_depth_combined_sampler,
            self.distant_depth_sampler,
            self.distant_depth_view,
            self.distant_depth_texture,
        ] {
            let _ = gal.destroy(handle);
        }
    }

    /// Returns the two independently sampled DH depth streams as one
    /// generation-coherent semantic source-resource subset. This is still
    /// target ownership only: it selects no shader program and creates no
    /// render pass. Later source stages must merge this exact set with the
    /// pack and world resource sets before their own source declarations are
    /// admitted.
    pub(crate) fn semantic_resources(&self) -> GalResult<TerrainSourceOwnedResourceSet> {
        let generation = self.resource_generation;
        let availability = TerrainSourceResourceAvailabilitySet::new(
            self.identity.shader_pack_generation,
            self.identity.world_generation,
            [
                TerrainSourceResourceAvailability {
                    role: TerrainSourceResourceRole::DistantHorizonsOpaqueDepth,
                    shape: TerrainSourceSampledResourceShape::Texture2d,
                    resource_generation: generation,
                },
                TerrainSourceResourceAvailability {
                    role: TerrainSourceResourceRole::DistantHorizonsDepthBeforeTranslucency,
                    shape: TerrainSourceSampledResourceShape::Texture2d,
                    resource_generation: generation,
                },
            ],
        )?;
        TerrainSourceOwnedResourceSet::new(
            availability,
            [
                TerrainSourceOwnedResource {
                    role: TerrainSourceResourceRole::DistantHorizonsOpaqueDepth,
                    combined_sampler: self.distant_depth_combined_sampler,
                },
                TerrainSourceOwnedResource {
                    role: TerrainSourceResourceRole::DistantHorizonsDepthBeforeTranslucency,
                    combined_sampler: self.distant_depth_before_translucency_combined_sampler,
                },
            ],
        )
    }

    /// Appends the semantic opaque-depth snapshot boundary used by later DH
    /// source stages. The live depth target is expected to have completed its
    /// opaque attachment writes; both images finish shader-readable. No
    /// source program, native target, or route selection is implied here.
    pub(crate) fn append_opaque_depth_snapshot(&self, ops: &mut Vec<CommandOp>) {
        let extent = self.identity.extent;
        ops.push(CommandOp::Barrier(texture_barrier(
            self.distant_depth_texture,
            TextureUsageState::DepthStencilAttachment,
            TextureUsageState::TransferSrc,
        )));
        ops.push(CommandOp::Barrier(texture_barrier(
            self.distant_depth_before_translucency_texture,
            TextureUsageState::Undefined,
            TextureUsageState::TransferDst,
        )));
        ops.push(CommandOp::CopyTexture(TextureImageCopyRegion {
            row_order: crate::render::vulkanic::commands::TextureRowOrder::Preserve,
            src_texture: self.distant_depth_texture,
            src_mip: 0,
            src_layer: 0,
            src_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
            dst_texture: self.distant_depth_before_translucency_texture,
            dst_mip: 0,
            dst_layer: 0,
            dst_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
            extent,
        }));
        ops.push(CommandOp::Barrier(texture_barrier(
            self.distant_depth_texture,
            TextureUsageState::TransferSrc,
            TextureUsageState::ShaderRead,
        )));
        ops.push(CommandOp::Barrier(texture_barrier(
            self.distant_depth_before_translucency_texture,
            TextureUsageState::TransferDst,
            TextureUsageState::ShaderRead,
        )));
    }

    /// Establishes a valid far opaque-depth stream when no visible Distant
    /// Horizons opaque range is available for a source-preparation frame.
    /// The later shader-pack stages still receive their own cleared far-depth
    /// images, not the near-terrain depth attachment and not an undefined
    /// placeholder. Both sampled streams finish in `ShaderRead`.
    pub(crate) fn append_empty_opaque_depth_snapshot(
        &self,
        prior_usage: TextureUsageState,
        ops: &mut Vec<CommandOp>,
    ) -> GalResult<()> {
        if !matches!(
            prior_usage,
            TextureUsageState::Undefined | TextureUsageState::ShaderRead
        ) {
            return Err(GalError::invalid_argument(
                "empty Distant Horizons source depth must begin undefined or shader-readable",
            ));
        }
        ops.push(CommandOp::Barrier(texture_barrier(
            self.distant_depth_texture,
            prior_usage,
            TextureUsageState::DepthStencilAttachment,
        )));
        ops.push(CommandOp::BeginPass {
            pass: self.distant_depth_clear_pass,
            target: self.distant_depth_clear_target,
            colors: Vec::new(),
            depth_stencil: Some(PassAttachment {
                view: self.distant_depth_view,
                load_op: AttachmentLoadOp::Clear,
                store_op: AttachmentStoreOp::Store,
                clear_color: None,
            }),
        });
        ops.push(CommandOp::EndPass);
        ops.push(CommandOp::Barrier(texture_barrier(
            self.distant_depth_texture,
            TextureUsageState::DepthStencilAttachment,
            TextureUsageState::TransferSrc,
        )));
        ops.push(CommandOp::Barrier(texture_barrier(
            self.distant_depth_before_translucency_texture,
            prior_usage,
            TextureUsageState::TransferDst,
        )));
        ops.push(CommandOp::CopyTexture(TextureImageCopyRegion {
            row_order: crate::render::vulkanic::commands::TextureRowOrder::Preserve,
            src_texture: self.distant_depth_texture,
            src_mip: 0,
            src_layer: 0,
            src_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
            dst_texture: self.distant_depth_before_translucency_texture,
            dst_mip: 0,
            dst_layer: 0,
            dst_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
            extent: self.identity.extent,
        }));
        ops.push(CommandOp::Barrier(texture_barrier(
            self.distant_depth_texture,
            TextureUsageState::TransferSrc,
            TextureUsageState::ShaderRead,
        )));
        ops.push(CommandOp::Barrier(texture_barrier(
            self.distant_depth_before_translucency_texture,
            TextureUsageState::TransferDst,
            TextureUsageState::ShaderRead,
        )));
        Ok(())
    }
}

/// Two-phase target cache. A replacement becomes active only after the exact
/// submission using it succeeds; failed source-plan preparation cannot leak a
/// newly created target into a later world or shader-pack generation.
#[derive(Default)]
pub(crate) struct WorldLodSourceTargetCache {
    active: Option<WorldLodSourceTargets>,
    pending: Option<WorldLodSourceTargets>,
    next_resource_generation: u64,
}

impl WorldLodSourceTargetCache {
    pub(crate) fn stage(
        &mut self,
        gal: &mut VulkanicGal,
        identity: WorldLodSourceTargetIdentity,
    ) -> GalResult<WorldLodSourceTargets> {
        identity.validate()?;
        if let Some(pending) = self.pending {
            if pending.identity != identity {
                return Err(GalError::backend(
                    "Distant Horizons source target replacement is awaiting submission confirmation",
                ));
            }
            return Ok(pending);
        }
        if let Some(active) = self.active {
            if active.identity == identity {
                return Ok(active);
            }
        }
        let targets = self.create_targets(gal, identity)?;
        self.pending = Some(targets);
        Ok(targets)
    }

    /// Stages the same owned source depth family while reporting the only
    /// valid usage that can precede a depth clear. A pending replacement is
    /// deliberately rejected here: one admission frame must own one clear
    /// and one later confirmation, rather than recording duplicate clears
    /// against the same unconfirmed targets.
    pub(crate) fn stage_for_empty_depth_snapshot(
        &mut self,
        gal: &mut VulkanicGal,
        identity: WorldLodSourceTargetIdentity,
    ) -> GalResult<(WorldLodSourceTargets, TextureUsageState, bool)> {
        identity.validate()?;
        if self.pending.is_some() {
            return Err(GalError::backend(
                "Distant Horizons source depth targets are already awaiting one admission submission",
            ));
        }
        if let Some(active) = self.active {
            if active.identity == identity {
                return Ok((active, TextureUsageState::ShaderRead, false));
            }
        }
        let targets = self.create_targets(gal, identity)?;
        self.pending = Some(targets);
        Ok((targets, TextureUsageState::Undefined, true))
    }

    pub(crate) fn confirm_submission(&mut self, gal: &mut VulkanicGal) {
        let Some(replacement) = self.pending.take() else {
            return;
        };
        if let Some(previous) = self.active.replace(replacement) {
            previous.destroy(gal);
        }
    }

    pub(crate) fn discard_submission(&mut self, gal: &mut VulkanicGal) {
        if let Some(pending) = self.pending.take() {
            pending.destroy(gal);
        }
    }

    /// Returns the confirmed semantic far-depth inputs only when they belong
    /// to this exact source frame compatibility identity. Callers receive no
    /// backend object identity and cannot reuse a stale world, pack, or extent.
    pub(crate) fn active_semantic_resources(
        &self,
        identity: WorldLodSourceTargetIdentity,
    ) -> GalResult<Option<TerrainSourceOwnedResourceSet>> {
        identity.validate()?;
        self.active
            .filter(|targets| targets.identity == identity)
            .map(|targets| targets.semantic_resources())
            .transpose()
    }

    pub(crate) fn destroy(&mut self, gal: &mut VulkanicGal) {
        self.discard_submission(gal);
        if let Some(active) = self.active.take() {
            active.destroy(gal);
        }
    }

    fn create_targets(
        &mut self,
        gal: &mut VulkanicGal,
        identity: WorldLodSourceTargetIdentity,
    ) -> GalResult<WorldLodSourceTargets> {
        self.next_resource_generation =
            self.next_resource_generation
                .checked_add(1)
                .ok_or_else(|| {
                    GalError::backend("Distant Horizons source target generation overflow")
                })?;
        WorldLodSourceTargets::create(gal, identity, self.next_resource_generation)
    }

    #[cfg(test)]
    pub(crate) fn active_identity(&self) -> Option<WorldLodSourceTargetIdentity> {
        self.active.map(|targets| targets.identity)
    }
}

/// DH emits one direction code for every generated quad. Retaining it as a
/// semantic face is required for source-independent directional shading and
/// for later shader-pack material lowering.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorldLodFaceNormal {
    Down,
    Up,
    North,
    South,
    West,
    East,
}

impl TryFrom<u8> for WorldLodFaceNormal {
    type Error = GalError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Down),
            1 => Ok(Self::Up),
            2 => Ok(Self::North),
            3 => Ok(Self::South),
            4 => Ok(Self::West),
            5 => Ok(Self::East),
            _ => Err(GalError::invalid_argument(format!(
                "unknown Distant Horizons face normal {value}"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct WorldLodExpandedVertex {
    /// Column-local position before the asset origin is applied.
    pub local_position: [f32; 3],
    /// DH's signed micro offset, decoded from its semantic packed metadata.
    pub micro_offset: [f32; 3],
    pub color_rgba: [f32; 4],
    pub sky_light: u8,
    pub block_light: u8,
    pub material: WorldLodMaterialCategory,
    pub normal: WorldLodFaceNormal,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct WorldLodExpandedSegment {
    pub layer: u32,
    pub vertices: Vec<WorldLodExpandedVertex>,
    /// The exact quad expansion used by DH's shared element buffer:
    /// `[a, b, c, c, d, a]` for each four-vertex quad.
    pub indices: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct WorldLodExpandedColumnAsset {
    pub column_key: u64,
    pub column_generation: u64,
    pub origin: [i32; 3],
    pub segments: Vec<WorldLodExpandedSegment>,
}

/// One fully resolved textured LOD vertex. It is a Rust frontend semantic
/// artifact, not a producer ABI or backend vertex format. The atlas rectangle
/// comes only from a generation-bound face material record, never from a
/// guessed block category or the pre-resolved DH color. `tile_uv` is allowed
/// to exceed one for a coalesced DH face so the private shader can repeat the
/// named sprite without sampling neighbouring atlas tiles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct WorldLodTexturedVertex {
    pub local_position: [f32; 3],
    pub micro_offset: [f32; 3],
    pub color_rgba: [f32; 4],
    pub tinted: bool,
    pub sky_light: u8,
    pub block_light: u8,
    /// DH's source-defined coarse material category. Exact-atlas rendering
    /// still needs this for shader-pack material branches after it resolves a
    /// concrete sprite; atlas identity alone cannot replace it.
    pub material: WorldLodMaterialCategory,
    pub normal: WorldLodFaceNormal,
    pub tile_uv: [f32; 2],
    pub atlas_rect: [f32; 4],
}

/// A single DH reduced quad that can be rendered with an exact source atlas
/// region. The owner can batch these only with compatible atlas/material
/// resources; this structure deliberately contains no native resource ID.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct WorldLodTexturedQuad {
    pub quad_index: u32,
    pub material_id: u32,
    pub face: u32,
    pub face_layer: u32,
    pub tinted: bool,
    pub atlas_identity: String,
    pub sprite_identity: String,
    pub vertices: [WorldLodTexturedVertex; 4],
}

/// Returns the unwrapped semantic tile coverage of an exact-atlas quad. This
/// is diagnostic-facing data: a merged LOD face must retain its repeat span
/// instead of being reduced to one global-atlas UV rectangle.
pub(crate) fn world_lod_textured_quad_tile_span(quad: &WorldLodTexturedQuad) -> [f32; 2] {
    let minimum = quad
        .vertices
        .iter()
        .fold([f32::INFINITY; 2], |minimum, vertex| {
            [
                minimum[0].min(vertex.tile_uv[0]),
                minimum[1].min(vertex.tile_uv[1]),
            ]
        });
    let maximum = quad
        .vertices
        .iter()
        .fold([f32::NEG_INFINITY; 2], |maximum, vertex| {
            [
                maximum[0].max(vertex.tile_uv[0]),
                maximum[1].max(vertex.tile_uv[1]),
            ]
        });
    [maximum[0] - minimum[0], maximum[1] - minimum[1]]
}

/// Why a reduced DH quad cannot participate in an exact-atlas route. These
/// reasons are intentionally semantic so callers can diagnose the producer
/// without conflating them with pipeline or backend availability.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorldLodTexturedQuadUnavailableReason {
    MaterialUnavailable,
    MaterialMixed,
    VariantUnavailable,
    VariantMixed,
    InconsistentFace,
    MissingFaceMaterial,
}

impl WorldLodTexturedQuadUnavailableReason {
    /// Stable diagnostic category for a quad deliberately retained on the
    /// reduced-color path. This is semantic planner evidence, not backend
    /// policy or a shader fallback selector.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::MaterialUnavailable => "material-unavailable",
            Self::MaterialMixed => "material-mixed",
            Self::VariantUnavailable => "variant-unavailable",
            Self::VariantMixed => "variant-mixed",
            Self::InconsistentFace => "inconsistent-face",
            Self::MissingFaceMaterial => "missing-face-material",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WorldLodTexturedQuadUnavailable {
    pub quad_index: u32,
    /// Builder-local semantic material ID and DH face are retained solely to
    /// explain why a reduced quad could not enter the exact-atlas stream.
    /// They never select a replacement material.
    pub material_id: u32,
    pub face: u32,
    pub reason: WorldLodTexturedQuadUnavailableReason,
}

/// Exact-atlas planning result for one already copied DH segment. A future
/// textured route must require `unavailable.is_empty()` for a segment it
/// admits; it may not silently substitute the old color-only material for an
/// incomplete quad.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct WorldLodTexturedSegmentPlan {
    pub layer: u32,
    /// Source quad cardinality stays explicit so a partial exact-atlas plan
    /// can construct a complementary coarse index stream without guessing
    /// from the resolved subset.
    pub source_quad_count: u32,
    pub quads: Vec<WorldLodTexturedQuad>,
    pub unavailable: Vec<WorldLodTexturedQuadUnavailable>,
}

/// Exact-atlas planning result for one immutable DH column generation. This
/// binds the provenance sidecar to copied geometry before any future texture
/// resource or draw is considered, so reloads cannot combine old atlas UVs
/// with a new reduced-column payload.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct WorldLodTexturedColumnPlan {
    pub column_key: u64,
    pub column_generation: u64,
    pub segments: Vec<WorldLodTexturedSegmentPlan>,
}

/// One exact-atlas segment ready for a later private GPU residency. The
/// source ordinal is preserved so it can replace only its matching legacy
/// color-only segment; no duplicate same-frame geometry is implied here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorldLodTexturedGpuSegment {
    pub source_segment_index: u32,
    pub layer: u32,
    pub vertex_layout_version: u32,
    pub vertex_bytes: Vec<u8>,
    pub index_type: IndexType,
    pub index_bytes: Vec<u8>,
    /// For a mixed source segment, the legacy DH vertex stream consumes this
    /// complementary index list. Known quads must not be drawn once by the
    /// coarse pass and again by the exact-atlas pass.
    pub unresolved_index_bytes: Option<Vec<u8>>,
}

/// A partial exact-atlas asset is deliberate. Known quads are packed into the
/// atlas stream while unknown quads retain the existing reduced-color stream.
/// `unavailable_source_segments` records exactly those mixed source segments;
/// it prevents the caller from suppressing their coarse geometry wholesale.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct WorldLodTexturedGpuColumnAsset {
    pub column_key: u64,
    pub column_generation: u64,
    pub segments: Vec<WorldLodTexturedGpuSegment>,
    pub unavailable_source_segments: Vec<u32>,
}

/// Immutable, owned bytes ready for a future Rust LOD GPU asset. Keeping this
/// separate from the expanded records gives the eventual asset cache a stable
/// payload without retaining DH's CPU layout or any Java-owned memory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorldLodGpuSegment {
    pub layer: u32,
    pub vertex_layout_version: u32,
    pub vertex_bytes: Vec<u8>,
    pub index_type: IndexType,
    pub index_bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorldLodGpuColumnAsset {
    pub column_key: u64,
    pub column_generation: u64,
    pub origin: [i32; 3],
    pub segments: Vec<WorldLodGpuSegment>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct WorldLodGpuSegmentResources {
    pub vertex_buffer: Handle,
    pub index_buffer: Handle,
}

#[derive(Clone, Debug)]
pub(crate) struct WorldLodGpuColumnResources {
    pub column_generation: u64,
    pub segments: Vec<WorldLodGpuSegmentResources>,
}

/// One generation-checked LOD draw range ready for a later Rust-owned
/// material pass. This is deliberately an internal frontend record: it keeps
/// the stable semantic column identity alongside private GAL buffers without
/// exposing either native backend state or the original DH vertex layout.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WorldLodGpuDraw {
    pub column_key: u64,
    pub column_generation: u64,
    pub origin: [i32; 3],
    pub layer: u32,
    pub segment_index: u32,
    /// Stable visible-list order copied from DH semantic extraction. It is
    /// irrelevant to opaque batching but required to preserve transparent
    /// ordering before the backend receives any draw operations.
    pub order: u32,
    pub vertex_buffer: Handle,
    pub index_buffer: Handle,
    pub index_type: IndexType,
    pub index_count: u32,
}

impl WorldLodGpuColumnResources {
    fn destroy(self, gal: &mut VulkanicGal) {
        for segment in self.segments.into_iter().rev() {
            let _ = gal.destroy(segment.index_buffer);
            let _ = gal.destroy(segment.vertex_buffer);
        }
    }
}

/// Private residency for immutable LOD geometry. It deliberately exposes no
/// pipeline, material, or draw operation: a completed material/pass contract
/// must select those separately. Uploads are staged into the caller's combined
/// frame submission and commit only once that submission is accepted.
#[derive(Default)]
pub(crate) struct WorldLodGpuResidency {
    active: BTreeMap<u64, WorldLodGpuColumnResources>,
    pending: Option<BTreeMap<u64, WorldLodGpuColumnResources>>,
}

impl WorldLodGpuResidency {
    pub(crate) fn stage_visible_uploads(
        &mut self,
        gal: &mut VulkanicGal,
        assets: &BTreeMap<u64, WorldLodGpuColumnAsset>,
        instances: &[WorldLodColumnInstanceRequest],
        ops: &mut Vec<CommandOp>,
    ) -> GalResult<()> {
        if self.pending.is_some() {
            return Err(GalError::backend(
                "world LOD GPU upload transaction is already awaiting submission confirmation",
            ));
        }
        let mut requested = BTreeSet::new();
        for instance in instances {
            let asset = assets.get(&instance.column_key).ok_or_else(|| {
                GalError::invalid_argument(format!(
                    "world LOD GPU upload references unknown column {}",
                    instance.column_key
                ))
            })?;
            if asset.column_generation != instance.column_generation {
                return Err(GalError::invalid_argument(
                    "world LOD GPU upload instance generation differs from cached payload",
                ));
            }
            let segment = asset
                .segments
                .get(instance.segment_index as usize)
                .ok_or_else(|| {
                    GalError::invalid_argument(format!(
                        "world LOD GPU upload references missing segment {}",
                        instance.segment_index
                    ))
                })?;
            if segment.layer != instance.layer {
                return Err(GalError::invalid_argument(
                    "world LOD GPU upload instance layer differs from cached payload",
                ));
            }
            requested.insert(instance.column_key);
        }

        let mut created = BTreeMap::new();
        let mut staged_ops = Vec::new();
        let result =
            (|| -> GalResult<()> {
                for column_key in requested {
                    let asset = assets
                        .get(&column_key)
                        .expect("requested columns are validated against the asset map");
                    if self.active.get(&column_key).is_some_and(|resources| {
                        resources.column_generation == asset.column_generation
                    }) {
                        continue;
                    }
                    let resources = create_column_resources(gal, asset)?;
                    staged_ops.extend(upload_ops(asset, &resources));
                    created.insert(column_key, resources);
                }
                Ok(())
            })();
        if let Err(error) = result {
            for (_, resources) in created {
                resources.destroy(gal);
            }
            return Err(error);
        }
        if !created.is_empty() {
            ops.append(&mut staged_ops);
            self.pending = Some(created);
        }
        Ok(())
    }

    /// Resolves the exact visible ranges for the current combined submission.
    /// Newly created resources are intentionally visible here before
    /// `confirm_submission`: their host writes and transfer barriers precede
    /// the eventual draw in that same submission. If the submission fails,
    /// `discard_submission` destroys those resources instead of letting them
    /// escape into the active cache.
    pub(crate) fn resolve_visible_draws(
        &self,
        assets: &BTreeMap<u64, WorldLodGpuColumnAsset>,
        instances: &[WorldLodColumnInstanceRequest],
    ) -> GalResult<Vec<WorldLodGpuDraw>> {
        let mut draws = Vec::with_capacity(instances.len());
        for instance in instances {
            let asset = assets.get(&instance.column_key).ok_or_else(|| {
                GalError::invalid_argument(format!(
                    "world LOD draw references unknown column {}",
                    instance.column_key
                ))
            })?;
            if asset.column_generation != instance.column_generation {
                return Err(GalError::invalid_argument(
                    "world LOD draw instance generation differs from cached payload",
                ));
            }
            let segment = asset
                .segments
                .get(instance.segment_index as usize)
                .ok_or_else(|| {
                    GalError::invalid_argument(format!(
                        "world LOD draw references missing segment {}",
                        instance.segment_index
                    ))
                })?;
            if segment.layer != instance.layer {
                return Err(GalError::invalid_argument(
                    "world LOD draw instance layer differs from cached payload",
                ));
            }
            if segment.vertex_layout_version != WORLD_LOD_GPU_VERTEX_LAYOUT_V1
                || segment.vertex_bytes.len() % WORLD_LOD_GPU_VERTEX_BYTES != 0
            {
                return Err(GalError::invalid_argument(
                    "world LOD draw references an unsupported GPU vertex payload",
                ));
            }
            let index_stride = match segment.index_type {
                IndexType::U16 => std::mem::size_of::<u16>(),
                IndexType::U32 => std::mem::size_of::<u32>(),
            };
            if segment.index_bytes.len() % index_stride != 0 {
                return Err(GalError::invalid_argument(
                    "world LOD draw index payload is not aligned to its explicit index type",
                ));
            }
            let index_count = u32::try_from(segment.index_bytes.len() / index_stride)
                .map_err(|_| GalError::invalid_argument("world LOD index count exceeds u32"))?;
            if index_count == 0 || index_count % 3 != 0 {
                return Err(GalError::invalid_argument(
                    "world LOD draw requires a non-empty triangle-aligned index range",
                ));
            }
            let resources =
                self.resources_for_submission(instance.column_key, instance.column_generation)?;
            let resources = resources
                .segments
                .get(instance.segment_index as usize)
                .ok_or_else(|| {
                    GalError::backend(format!(
                        "world LOD GPU resources are missing segment {}",
                        instance.segment_index
                    ))
                })?;
            draws.push(WorldLodGpuDraw {
                column_key: instance.column_key,
                column_generation: instance.column_generation,
                origin: asset.origin,
                layer: instance.layer,
                segment_index: instance.segment_index,
                order: instance.order,
                vertex_buffer: resources.vertex_buffer,
                index_buffer: resources.index_buffer,
                index_type: segment.index_type,
                index_count,
            });
        }
        Ok(draws)
    }

    pub(crate) fn confirm_submission(&mut self, gal: &mut VulkanicGal) -> GalResult<()> {
        let Some(created) = self.pending.take() else {
            return Ok(());
        };
        for (column_key, resources) in created {
            if let Some(previous) = self.active.insert(column_key, resources) {
                previous.destroy(gal);
            }
        }
        Ok(())
    }

    pub(crate) fn discard_submission(&mut self, gal: &mut VulkanicGal) {
        if let Some(created) = self.pending.take() {
            for (_, resources) in created {
                resources.destroy(gal);
            }
        }
    }

    pub(crate) fn reconcile_assets(
        &mut self,
        gal: &mut VulkanicGal,
        assets: &BTreeMap<u64, WorldLodGpuColumnAsset>,
    ) {
        self.discard_submission(gal);
        let stale = self
            .active
            .iter()
            .filter_map(|(&column_key, resources)| {
                assets
                    .get(&column_key)
                    .is_none_or(|asset| asset.column_generation != resources.column_generation)
                    .then_some(column_key)
            })
            .collect::<Vec<_>>();
        for column_key in stale {
            if let Some(resources) = self.active.remove(&column_key) {
                resources.destroy(gal);
            }
        }
    }

    pub(crate) fn destroy(&mut self, gal: &mut VulkanicGal) {
        self.discard_submission(gal);
        for (_, resources) in std::mem::take(&mut self.active) {
            resources.destroy(gal);
        }
    }

    #[cfg(test)]
    pub(crate) fn active_generation(&self, column_key: u64) -> Option<u64> {
        self.active
            .get(&column_key)
            .map(|resources| resources.column_generation)
    }

    fn resources_for_submission(
        &self,
        column_key: u64,
        column_generation: u64,
    ) -> GalResult<&WorldLodGpuColumnResources> {
        let resources = self
            .pending
            .as_ref()
            .and_then(|pending| pending.get(&column_key))
            .or_else(|| self.active.get(&column_key))
            .ok_or_else(|| {
                GalError::invalid_argument(format!(
                    "world LOD draw column {column_key} has not been staged for GPU submission",
                ))
            })?;
        if resources.column_generation != column_generation {
            return Err(GalError::invalid_argument(format!(
                "world LOD draw column {column_key} GPU generation {} does not match instance generation {column_generation}",
                resources.column_generation
            )));
        }
        Ok(resources)
    }
}

#[derive(Clone, Copy, Debug)]
struct WorldLodTexturedGpuSegmentResources {
    vertex_buffer: Handle,
    index_buffer: Handle,
    /// Indexes into the matching reduced-color source vertex stream for
    /// source quads which have no exact atlas provenance. Keeping this beside
    /// the exact-atlas asset lets the caller partition a partial segment
    /// without duplicating the resolved quads in the coarse pass.
    unresolved_index_buffer: Option<Handle>,
}

#[derive(Clone, Debug)]
struct WorldLodTexturedGpuColumnResources {
    column_generation: u64,
    segments: BTreeMap<u32, WorldLodTexturedGpuSegmentResources>,
}

impl WorldLodTexturedGpuColumnResources {
    fn destroy(self, gal: &mut VulkanicGal) {
        for (_, segment) in self.segments.into_iter().rev() {
            if let Some(index_buffer) = segment.unresolved_index_buffer {
                let _ = gal.destroy(index_buffer);
            }
            let _ = gal.destroy(segment.index_buffer);
            let _ = gal.destroy(segment.vertex_buffer);
        }
    }
}

/// An exact-atlas draw replaces one source segment, never a whole column.
/// The source segment ordinal is retained until final command construction so
/// incomplete semantic provenance cannot accidentally duplicate or suppress
/// a neighboring legacy DH segment.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WorldLodTexturedGpuDraw {
    pub column_key: u64,
    pub column_generation: u64,
    pub origin: [i32; 3],
    pub layer: u32,
    pub source_segment_index: u32,
    pub order: u32,
    pub vertex_buffer: Handle,
    pub index_buffer: Handle,
    pub index_type: IndexType,
    pub index_count: u32,
}

/// Private residency for the complete exact-atlas subset of a DH column. It
/// intentionally shares no buffers with the legacy stream: the two private
/// vertex contracts differ, and route selection happens per source segment.
#[derive(Default)]
pub(crate) struct WorldLodTexturedGpuResidency {
    active: BTreeMap<u64, WorldLodTexturedGpuColumnResources>,
    pending: Option<BTreeMap<u64, WorldLodTexturedGpuColumnResources>>,
}

impl WorldLodTexturedGpuResidency {
    pub(crate) fn stage_visible_uploads(
        &mut self,
        gal: &mut VulkanicGal,
        assets: &BTreeMap<u64, WorldLodTexturedGpuColumnAsset>,
        instances: &[WorldLodColumnInstanceRequest],
        ops: &mut Vec<CommandOp>,
    ) -> GalResult<()> {
        if self.pending.is_some() {
            return Err(GalError::backend(
                "world LOD exact-atlas GPU upload transaction is already awaiting submission confirmation",
            ));
        }
        let requested = instances
            .iter()
            .filter_map(|instance| {
                let asset = assets.get(&instance.column_key)?;
                (asset.column_generation == instance.column_generation
                    && asset.segments.iter().any(|segment| {
                        segment.source_segment_index == instance.segment_index
                            && segment.layer == instance.layer
                    }))
                .then_some(instance.column_key)
            })
            .collect::<BTreeSet<_>>();
        let mut created = BTreeMap::new();
        let mut staged_ops = Vec::new();
        let result =
            (|| -> GalResult<()> {
                for column_key in requested {
                    let asset = assets
                        .get(&column_key)
                        .expect("requested exact-atlas columns come from the asset map");
                    if self.active.get(&column_key).is_some_and(|resources| {
                        resources.column_generation == asset.column_generation
                    }) {
                        continue;
                    }
                    let resources = create_textured_column_resources(gal, asset)?;
                    staged_ops.extend(textured_upload_ops(asset, &resources));
                    created.insert(column_key, resources);
                }
                Ok(())
            })();
        if let Err(error) = result {
            for (_, resources) in created {
                resources.destroy(gal);
            }
            return Err(error);
        }
        if !created.is_empty() {
            ops.append(&mut staged_ops);
            self.pending = Some(created);
        }
        Ok(())
    }

    /// Returns `Ok(None)` for an incomplete source segment. That is a route
    /// decision, not a resource error: the caller must keep exactly one
    /// legacy-color draw for that segment.
    pub(crate) fn resolve_visible_draw(
        &self,
        assets: &BTreeMap<u64, WorldLodTexturedGpuColumnAsset>,
        instance: &WorldLodColumnInstanceRequest,
        origin: [i32; 3],
    ) -> GalResult<Option<WorldLodTexturedGpuDraw>> {
        let Some(asset) = assets.get(&instance.column_key) else {
            return Ok(None);
        };
        if asset.column_generation != instance.column_generation {
            return Ok(None);
        }
        let Some(segment) = asset.segments.iter().find(|segment| {
            segment.source_segment_index == instance.segment_index
                && segment.layer == instance.layer
        }) else {
            return Ok(None);
        };
        if segment.vertex_layout_version != WORLD_LOD_TEXTURED_GPU_VERTEX_LAYOUT_V2
            || segment.vertex_bytes.is_empty()
            || segment.vertex_bytes.len() % WORLD_LOD_TEXTURED_GPU_VERTEX_BYTES != 0
        {
            return Err(GalError::invalid_argument(
                "world LOD exact-atlas draw references an unsupported vertex payload",
            ));
        }
        let index_stride = match segment.index_type {
            IndexType::U16 => std::mem::size_of::<u16>(),
            IndexType::U32 => std::mem::size_of::<u32>(),
        };
        if segment.index_bytes.len() % index_stride != 0 {
            return Err(GalError::invalid_argument(
                "world LOD exact-atlas index payload is not aligned to its explicit index type",
            ));
        }
        let index_count =
            u32::try_from(segment.index_bytes.len() / index_stride).map_err(|_| {
                GalError::invalid_argument("world LOD exact-atlas index count exceeds u32")
            })?;
        if index_count == 0 || index_count % 3 != 0 {
            return Err(GalError::invalid_argument(
                "world LOD exact-atlas draw requires a non-empty triangle-aligned index range",
            ));
        }
        let resources =
            self.resources_for_submission(instance.column_key, instance.column_generation)?;
        let resources = resources
            .segments
            .get(&instance.segment_index)
            .ok_or_else(|| {
                GalError::backend(format!(
                    "world LOD exact-atlas GPU resources are missing source segment {}",
                    instance.segment_index
                ))
            })?;
        Ok(Some(WorldLodTexturedGpuDraw {
            column_key: instance.column_key,
            column_generation: instance.column_generation,
            origin,
            layer: instance.layer,
            source_segment_index: instance.segment_index,
            order: instance.order,
            vertex_buffer: resources.vertex_buffer,
            index_buffer: resources.index_buffer,
            index_type: segment.index_type,
            index_count,
        }))
    }

    /// Resolves the complementary reduced-color range for a source segment
    /// which also has one or more exact-atlas quads. The caller supplies the
    /// normal LOD draw so the vertex stream and per-draw semantics remain
    /// identical; this helper replaces only its explicit index range.
    pub(crate) fn resolve_visible_unresolved_draw(
        &self,
        assets: &BTreeMap<u64, WorldLodTexturedGpuColumnAsset>,
        instance: &WorldLodColumnInstanceRequest,
        source_draw: WorldLodGpuDraw,
    ) -> GalResult<Option<WorldLodGpuDraw>> {
        if source_draw.column_key != instance.column_key
            || source_draw.column_generation != instance.column_generation
            || source_draw.layer != instance.layer
            || source_draw.segment_index != instance.segment_index
        {
            return Err(GalError::invalid_argument(
                "world LOD unresolved range does not match its visible source draw",
            ));
        }
        let Some(asset) = assets.get(&instance.column_key) else {
            return Ok(None);
        };
        if asset.column_generation != instance.column_generation {
            return Ok(None);
        }
        let Some(segment) = asset.segments.iter().find(|segment| {
            segment.source_segment_index == instance.segment_index
                && segment.layer == instance.layer
        }) else {
            return Ok(None);
        };
        let Some(unresolved_index_bytes) = segment.unresolved_index_bytes.as_ref() else {
            return Ok(None);
        };
        if unresolved_index_bytes.is_empty()
            || unresolved_index_bytes.len() % std::mem::size_of::<u32>() != 0
        {
            return Err(GalError::invalid_argument(
                "world LOD unresolved range has an invalid explicit u32 index payload",
            ));
        }
        let index_count = u32::try_from(unresolved_index_bytes.len() / std::mem::size_of::<u32>())
            .map_err(|_| {
                GalError::invalid_argument("world LOD unresolved index count exceeds u32")
            })?;
        if index_count % 3 != 0 {
            return Err(GalError::invalid_argument(
                "world LOD unresolved range is not triangle aligned",
            ));
        }
        let resources =
            self.resources_for_submission(instance.column_key, instance.column_generation)?;
        let resources = resources
            .segments
            .get(&instance.segment_index)
            .ok_or_else(|| {
                GalError::backend(format!(
                    "world LOD exact-atlas GPU resources are missing source segment {}",
                    instance.segment_index
                ))
            })?;
        let index_buffer = resources.unresolved_index_buffer.ok_or_else(|| {
            GalError::backend(
                "world LOD unresolved index buffer was not created for a partial exact-atlas segment",
            )
        })?;
        Ok(Some(WorldLodGpuDraw {
            index_buffer,
            index_type: IndexType::U32,
            index_count,
            ..source_draw
        }))
    }

    pub(crate) fn confirm_submission(&mut self, gal: &mut VulkanicGal) -> GalResult<()> {
        let Some(created) = self.pending.take() else {
            return Ok(());
        };
        for (column_key, resources) in created {
            if let Some(previous) = self.active.insert(column_key, resources) {
                previous.destroy(gal);
            }
        }
        Ok(())
    }

    pub(crate) fn discard_submission(&mut self, gal: &mut VulkanicGal) {
        if let Some(created) = self.pending.take() {
            for (_, resources) in created {
                resources.destroy(gal);
            }
        }
    }

    pub(crate) fn reconcile_assets(
        &mut self,
        gal: &mut VulkanicGal,
        assets: &BTreeMap<u64, WorldLodTexturedGpuColumnAsset>,
    ) {
        self.discard_submission(gal);
        let stale = self
            .active
            .iter()
            .filter_map(|(&key, resources)| {
                assets
                    .get(&key)
                    .is_none_or(|asset| asset.column_generation != resources.column_generation)
                    .then_some(key)
            })
            .collect::<Vec<_>>();
        for key in stale {
            if let Some(resources) = self.active.remove(&key) {
                resources.destroy(gal);
            }
        }
    }

    pub(crate) fn destroy(&mut self, gal: &mut VulkanicGal) {
        self.discard_submission(gal);
        for (_, resources) in std::mem::take(&mut self.active) {
            resources.destroy(gal);
        }
    }

    fn resources_for_submission(
        &self,
        column_key: u64,
        column_generation: u64,
    ) -> GalResult<&WorldLodTexturedGpuColumnResources> {
        let resources = self
            .pending
            .as_ref()
            .and_then(|pending| pending.get(&column_key))
            .or_else(|| self.active.get(&column_key))
            .ok_or_else(|| {
                GalError::invalid_argument(format!(
                "world LOD exact-atlas column {column_key} has not been staged for GPU submission"
            ))
            })?;
        if resources.column_generation != column_generation {
            return Err(GalError::invalid_argument(format!(
                "world LOD exact-atlas GPU generation {} does not match instance generation {column_generation}",
                resources.column_generation
            )));
        }
        Ok(resources)
    }
}

pub(crate) fn expand_world_lod_column_asset(
    asset: &WorldLodColumnAsset,
) -> GalResult<WorldLodExpandedColumnAsset> {
    validate_world_lod_column_asset(asset)?;
    let mut segments = Vec::with_capacity(asset.segments.len());
    for segment in &asset.segments {
        segments.push(expand_segment(segment)?);
    }
    Ok(WorldLodExpandedColumnAsset {
        column_key: asset.column_key,
        column_generation: asset.column_generation,
        origin: asset.origin,
        segments,
    })
}

/// Resolves the exact atlas UVs for one compact DH segment. The segment's
/// one-ID-per-quad sidecar and face material table must belong to the same
/// immutable column generation; that pairing is validated before this helper
/// is called. This function deliberately does not derive a texture from the
/// vertex material category, because that category is only DH shading data.
pub(crate) fn plan_world_lod_textured_segment(
    segment: &WorldLodSegment,
    quad_material_ids: &[u32],
    face_materials: &[WorldLodFaceMaterial],
) -> GalResult<WorldLodTexturedSegmentPlan> {
    let variant_states = vec![super::WORLD_LOD_VARIANT_EXACT; quad_material_ids.len()];
    let variant_positions = vec![0; quad_material_ids.len()];
    plan_world_lod_textured_segment_with_variants(
        segment,
        quad_material_ids,
        &variant_states,
        &variant_positions,
        face_materials,
    )
}

pub(crate) fn plan_world_lod_textured_segment_with_variants(
    segment: &WorldLodSegment,
    quad_material_ids: &[u32],
    quad_variant_states: &[u8],
    quad_variant_positions: &[u64],
    face_materials: &[WorldLodFaceMaterial],
) -> GalResult<WorldLodTexturedSegmentPlan> {
    let face_material_index = world_lod_face_material_index(face_materials);
    plan_world_lod_textured_segment_with_material_index(
        segment,
        quad_material_ids,
        quad_variant_states,
        quad_variant_positions,
        &face_material_index,
    )
}

/// Indexes immutable Java-copied face provenance once per column generation.
/// A real DH column has tens of thousands of reduced quads and thousands of
/// face records; repeatedly scanning the latter for every quad made asset
/// publication quadratic without adding any rendering semantics.
fn world_lod_face_material_index(
    face_materials: &[WorldLodFaceMaterial],
) -> BTreeMap<(u32, u32, u64), Vec<&WorldLodFaceMaterial>> {
    let mut index = BTreeMap::new();
    for material in face_materials {
        index
            .entry((
                material.material_id,
                material.face,
                material.variant_position,
            ))
            .or_insert_with(Vec::new)
            .push(material);
    }
    for layers in index.values_mut() {
        layers.sort_by_key(|material| material.face_layer);
    }
    index
}

fn plan_world_lod_textured_segment_with_material_index(
    segment: &WorldLodSegment,
    quad_material_ids: &[u32],
    quad_variant_states: &[u8],
    quad_variant_positions: &[u64],
    face_materials: &BTreeMap<(u32, u32, u64), Vec<&WorldLodFaceMaterial>>,
) -> GalResult<WorldLodTexturedSegmentPlan> {
    if segment.vertices.len() % 4 != 0 {
        return Err(GalError::invalid_argument(
            "world LOD textured planning requires quad-aligned segment vertices",
        ));
    }
    if quad_material_ids.len() != segment.vertices.len() / 4 {
        return Err(GalError::invalid_argument(format!(
            "world LOD textured planning has {} material IDs for {} quads",
            quad_material_ids.len(),
            segment.vertices.len() / 4
        )));
    }
    if quad_variant_states.len() != quad_material_ids.len()
        || quad_variant_positions.len() != quad_material_ids.len()
    {
        return Err(GalError::invalid_argument(
            "world LOD textured planning requires one variant record per quad",
        ));
    }
    let mut plan = WorldLodTexturedSegmentPlan {
        layer: segment.layer,
        source_quad_count: u32::try_from(quad_material_ids.len())
            .map_err(|_| GalError::invalid_argument("world LOD textured quad count exceeds u32"))?,
        quads: Vec::with_capacity(quad_material_ids.len()),
        unavailable: Vec::new(),
    };
    for (quad_index, ((vertices, &material_id), (&variant_state, &variant_position))) in segment
        .vertices
        .chunks_exact(4)
        .zip(quad_material_ids)
        .zip(quad_variant_states.iter().zip(quad_variant_positions))
        .enumerate()
    {
        let quad_index = u32::try_from(quad_index)
            .map_err(|_| GalError::invalid_argument("world LOD quad index exceeds u32"))?;
        let face = u32::from(vertices[0].normal_index);
        let unavailable_reason = if material_id == super::WORLD_LOD_MATERIAL_UNAVAILABLE {
            Some(WorldLodTexturedQuadUnavailableReason::MaterialUnavailable)
        } else if material_id == super::WORLD_LOD_MATERIAL_MIXED {
            Some(WorldLodTexturedQuadUnavailableReason::MaterialMixed)
        } else if variant_state == super::WORLD_LOD_VARIANT_UNAVAILABLE {
            Some(WorldLodTexturedQuadUnavailableReason::VariantUnavailable)
        } else if variant_state == super::WORLD_LOD_VARIANT_MIXED {
            Some(WorldLodTexturedQuadUnavailableReason::VariantMixed)
        } else if vertices
            .iter()
            .any(|vertex| vertex.normal_index != vertices[0].normal_index)
        {
            Some(WorldLodTexturedQuadUnavailableReason::InconsistentFace)
        } else {
            None
        };
        if let Some(reason) = unavailable_reason {
            plan.unavailable.push(WorldLodTexturedQuadUnavailable {
                quad_index,
                material_id,
                face,
                reason,
            });
            continue;
        }
        // Position-specific material records carry a biome tint or a weighted
        // model selection. A stable position-zero record remains a safe
        // semantic fallback only when no exact override was copied; genuinely
        // weighted models have no such base record and still reject.
        let Some(materials) = face_materials
            .get(&(material_id, face, variant_position))
            .or_else(|| face_materials.get(&(material_id, face, 0)))
        else {
            plan.unavailable.push(WorldLodTexturedQuadUnavailable {
                quad_index,
                material_id,
                face,
                reason: WorldLodTexturedQuadUnavailableReason::MissingFaceMaterial,
            });
            continue;
        };
        let expanded = [
            expand_vertex(&vertices[0])?,
            expand_vertex(&vertices[1])?,
            expand_vertex(&vertices[2])?,
            expand_vertex(&vertices[3])?,
        ];
        for material in materials {
            let tile_uv = world_lod_face_tile_coordinates(&expanded, face, material)?;
            let vertices = expanded.map(|vertex| WorldLodTexturedVertex {
                local_position: vertex.local_position,
                micro_offset: vertex.micro_offset,
                color_rgba: if material.tinted {
                    [
                        material.tint_rgb[0],
                        material.tint_rgb[1],
                        material.tint_rgb[2],
                        1.0,
                    ]
                } else {
                    vertex.color_rgba
                },
                tinted: material.tinted,
                sky_light: vertex.sky_light,
                block_light: vertex.block_light,
                material: vertex.material,
                normal: vertex.normal,
                tile_uv: [0.0, 0.0],
                atlas_rect: material.atlas_uv,
            });
            let mut vertices = vertices;
            for (serialized_vertex_index, vertex) in vertices.iter_mut().enumerate() {
                // The serialised order determines the unwrapped tile coordinate;
                // the copied material transform preserves model UV rotation.
                vertex.tile_uv =
                    world_lod_face_sprite_tile_uv(material, tile_uv[serialized_vertex_index]);
            }
            plan.quads.push(WorldLodTexturedQuad {
                quad_index,
                material_id,
                face,
                face_layer: material.face_layer,
                tinted: material.tinted,
                atlas_identity: material.atlas_identity.clone(),
                sprite_identity: material.sprite_identity.clone(),
                vertices,
            });
        }
    }
    Ok(plan)
}

/// Binds each per-quad provenance sidecar to its exact copied column. The
/// result retains incomplete quads as explicit unavailability records, so a
/// later pass can never infer a sprite from a DH material category or a table
/// ordinal when reduced geometry lost that information.
pub(crate) fn plan_world_lod_textured_column(
    asset: &WorldLodColumnAsset,
    provenance: &WorldLodColumnMaterialProvenance,
) -> GalResult<WorldLodTexturedColumnPlan> {
    validate_world_lod_column_asset(asset)?;
    if provenance.column_key != asset.column_key
        || provenance.column_generation != asset.column_generation
    {
        return Err(GalError::invalid_argument(
            "world LOD textured provenance key or generation differs from its column asset",
        ));
    }
    if provenance.segments.len() != asset.segments.len() {
        return Err(GalError::invalid_argument(format!(
            "world LOD textured provenance has {} segments for {} column segments",
            provenance.segments.len(),
            asset.segments.len()
        )));
    }

    let mut segments = Vec::with_capacity(asset.segments.len());
    let face_material_index = world_lod_face_material_index(&provenance.face_materials);
    for (segment_index, segment) in asset.segments.iter().enumerate() {
        let expected_index = u32::try_from(segment_index)
            .map_err(|_| GalError::invalid_argument("world LOD segment index exceeds u32"))?;
        let provenance_segment = provenance.segments.get(segment_index).ok_or_else(|| {
            GalError::invalid_argument("world LOD textured provenance is incomplete")
        })?;
        if provenance_segment.segment_index != expected_index
            || provenance_segment.layer != segment.layer
        {
            return Err(GalError::invalid_argument(format!(
                "world LOD textured provenance segment {segment_index} does not match its geometry layer/order",
            )));
        }
        segments.push(plan_world_lod_textured_segment_with_material_index(
            segment,
            &provenance_segment.quad_material_ids,
            &provenance_segment.quad_variant_states,
            &provenance_segment.quad_variant_positions,
            &face_material_index,
        )?);
    }
    Ok(WorldLodTexturedColumnPlan {
        column_key: asset.column_key,
        column_generation: asset.column_generation,
        segments,
    })
}

/// Converts every resolved, block-atlas-backed DH quad into an owned GPU
/// payload. A source segment can be partial: the later draw planner retains
/// its reduced-color draw for unknown quads, then overlays its resolved atlas
/// subset with the same depth contract. This keeps one source identity per
/// textured quad without assigning an arbitrary sprite to its neighbours.
pub(crate) fn pack_world_lod_textured_column_asset(
    plan: &WorldLodTexturedColumnPlan,
) -> GalResult<WorldLodTexturedGpuColumnAsset> {
    let mut segments = Vec::with_capacity(plan.segments.len());
    let mut unavailable_source_segments = Vec::new();
    for (source_segment_index, segment) in plan.segments.iter().enumerate() {
        let source_segment_index = u32::try_from(source_segment_index).map_err(|_| {
            GalError::invalid_argument("world LOD textured source segment index exceeds u32")
        })?;
        if segment
            .quads
            .iter()
            .any(|quad| quad.atlas_identity != WORLD_LOD_TERRAIN_ATLAS_IDENTITY)
        {
            unavailable_source_segments.push(source_segment_index);
            continue;
        }
        if !segment.unavailable.is_empty() {
            unavailable_source_segments.push(source_segment_index);
        }
        if segment.quads.is_empty() {
            continue;
        }

        validate_world_lod_textured_segment_coverage(segment)?;

        let vertex_count = segment.quads.len().checked_mul(4).ok_or_else(|| {
            GalError::invalid_argument("world LOD textured vertex count overflows usize")
        })?;
        let index_count = segment.quads.len().checked_mul(6).ok_or_else(|| {
            GalError::invalid_argument("world LOD textured index count overflows usize")
        })?;
        let mut vertex_bytes = Vec::with_capacity(
            vertex_count
                .checked_mul(WORLD_LOD_TEXTURED_GPU_VERTEX_BYTES)
                .ok_or_else(|| {
                    GalError::invalid_argument("world LOD textured vertex byte count overflows")
                })?,
        );
        let mut index_bytes = Vec::with_capacity(
            index_count
                .checked_mul(std::mem::size_of::<u32>())
                .ok_or_else(|| {
                    GalError::invalid_argument("world LOD textured index byte count overflows")
                })?,
        );
        for quad in &segment.quads {
            let base = u32::try_from(vertex_bytes.len() / WORLD_LOD_TEXTURED_GPU_VERTEX_BYTES)
                .map_err(|_| {
                    GalError::invalid_argument("world LOD textured vertex index exceeds u32")
                })?;
            for vertex in quad.vertices {
                write_world_lod_textured_vertex(&mut vertex_bytes, vertex);
            }
            for index in [base, base + 1, base + 2, base + 2, base + 3, base] {
                index_bytes.extend_from_slice(&index.to_le_bytes());
            }
        }
        debug_assert_eq!(
            vertex_count * WORLD_LOD_TEXTURED_GPU_VERTEX_BYTES,
            vertex_bytes.len()
        );
        let unresolved_index_bytes = (!segment.unavailable.is_empty())
            .then(|| world_lod_unresolved_quad_indices(&segment.unavailable))
            .transpose()?;
        segments.push(WorldLodTexturedGpuSegment {
            source_segment_index,
            layer: segment.layer,
            vertex_layout_version: WORLD_LOD_TEXTURED_GPU_VERTEX_LAYOUT_V2,
            vertex_bytes,
            index_type: IndexType::U32,
            index_bytes,
            unresolved_index_bytes,
        });
    }
    Ok(WorldLodTexturedGpuColumnAsset {
        column_key: plan.column_key,
        column_generation: plan.column_generation,
        segments,
        unavailable_source_segments,
    })
}

fn validate_world_lod_textured_segment_coverage(
    segment: &WorldLodTexturedSegmentPlan,
) -> GalResult<()> {
    if segment.source_quad_count == 0 {
        return Err(GalError::invalid_argument(
            "world LOD textured segment has no source quads",
        ));
    }
    let mut covered = vec![false; segment.source_quad_count as usize];
    let mut layers = BTreeSet::new();
    for quad in &segment.quads {
        let slot = covered.get_mut(quad.quad_index as usize).ok_or_else(|| {
            GalError::invalid_argument(
                "world LOD exact-atlas quad index exceeds its source segment",
            )
        })?;
        if !layers.insert((quad.quad_index, quad.face_layer)) {
            return Err(GalError::invalid_argument(
                "world LOD exact-atlas source quad layer is duplicated",
            ));
        }
        *slot = true;
    }
    for unavailable in &segment.unavailable {
        let slot = covered
            .get_mut(unavailable.quad_index as usize)
            .ok_or_else(|| {
                GalError::invalid_argument(
                    "world LOD unresolved quad index exceeds its source segment",
                )
            })?;
        if std::mem::replace(slot, true) {
            return Err(GalError::invalid_argument(
                "world LOD exact-atlas and unresolved streams overlap",
            ));
        }
    }
    if covered.iter().any(|covered| !covered) {
        return Err(GalError::invalid_argument(
            "world LOD textured segment leaves source quads uncovered",
        ));
    }
    Ok(())
}

fn world_lod_unresolved_quad_indices(
    unavailable: &[WorldLodTexturedQuadUnavailable],
) -> GalResult<Vec<u8>> {
    let index_count = unavailable.len().checked_mul(6).ok_or_else(|| {
        GalError::invalid_argument("world LOD unresolved index count overflows usize")
    })?;
    let mut bytes = Vec::with_capacity(
        index_count
            .checked_mul(std::mem::size_of::<u32>())
            .ok_or_else(|| {
                GalError::invalid_argument("world LOD unresolved index byte count overflows usize")
            })?,
    );
    for unavailable in unavailable {
        let base = unavailable.quad_index.checked_mul(4).ok_or_else(|| {
            GalError::invalid_argument("world LOD unresolved vertex index overflows u32")
        })?;
        for index in [base, base + 1, base + 2, base + 2, base + 3, base] {
            bytes.extend_from_slice(&index.to_le_bytes());
        }
    }
    Ok(bytes)
}

fn write_world_lod_textured_vertex(bytes: &mut Vec<u8>, vertex: WorldLodTexturedVertex) {
    for value in vertex.local_position {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for value in vertex.micro_offset {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for value in vertex.tile_uv {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for value in vertex.atlas_rect {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for value in vertex.color_rgba {
        bytes.push((value * 255.0).round().clamp(0.0, 255.0) as u8);
    }
    bytes.extend_from_slice(&[
        vertex.sky_light,
        vertex.block_light,
        face_normal_id(vertex.normal),
        // Keep the private exact-atlas stream compact while retaining both
        // source semantics. DH has sixteen material categories, so one bit
        // for exact face tint plus four category bits fits in this byte.
        (material_category_id(vertex.material) << 1) | u8::from(vertex.tinted),
    ]);
    debug_assert_eq!(0, bytes.len() % WORLD_LOD_TEXTURED_GPU_VERTEX_BYTES);
}

fn world_lod_face_tile_coordinates(
    vertices: &[WorldLodExpandedVertex; 4],
    face: u32,
    material: &WorldLodFaceMaterial,
) -> GalResult<[[f32; 2]; 4]> {
    let face = WorldLodFaceNormal::try_from(
        u8::try_from(face).map_err(|_| GalError::invalid_argument("world LOD face exceeds u8"))?,
    )?;
    let mut coordinates = vertices.map(|vertex| match face {
        WorldLodFaceNormal::Down => [vertex.local_position[0], vertex.local_position[2]],
        WorldLodFaceNormal::Up => [-vertex.local_position[0], vertex.local_position[2]],
        WorldLodFaceNormal::North => [vertex.local_position[0], vertex.local_position[1]],
        WorldLodFaceNormal::South => [-vertex.local_position[0], vertex.local_position[1]],
        WorldLodFaceNormal::West => [vertex.local_position[2], vertex.local_position[1]],
        WorldLodFaceNormal::East => [vertex.local_position[2], -vertex.local_position[1]],
    });
    let minimum = coordinates
        .iter()
        .fold([f32::INFINITY; 2], |minimum, value| {
            [minimum[0].min(value[0]), minimum[1].min(value[1])]
        });
    for coordinate in &mut coordinates {
        coordinate[0] -= minimum[0];
        coordinate[1] -= minimum[1];
    }
    if !coordinates.into_iter().flatten().all(f32::is_finite) {
        return Err(GalError::invalid_argument(
            "world LOD exact-atlas quad has non-finite tile coordinates",
        ));
    }
    // Verify the material rotation/mirroring is a square transform before the
    // values reach the private shader. The ABI already rejects duplicates;
    // this makes malformed manual Rust construction fail deterministically.
    for corner in 0..4 {
        let _ = world_lod_face_material_corner(material, corner);
    }
    Ok(coordinates)
}

fn world_lod_face_sprite_tile_uv(
    material: &WorldLodFaceMaterial,
    canonical_tile_uv: [f32; 2],
) -> [f32; 2] {
    let origin = world_lod_face_material_corner(material, 0);
    let u_axis = world_lod_face_material_corner(material, 3);
    let v_axis = world_lod_face_material_corner(material, 1);
    [
        origin[0]
            + canonical_tile_uv[0] * (u_axis[0] - origin[0])
            + canonical_tile_uv[1] * (v_axis[0] - origin[0]),
        origin[1]
            + canonical_tile_uv[0] * (u_axis[1] - origin[1])
            + canonical_tile_uv[1] * (v_axis[1] - origin[1]),
    ]
}

fn world_lod_face_material_corner(
    material: &WorldLodFaceMaterial,
    corner_index: usize,
) -> [f32; 2] {
    debug_assert!(corner_index < 4);
    let corner = (material.uv_corner_order >> (corner_index * 2)) & 0x3;
    [
        if corner & 0x1 == 0 { 0.0 } else { 1.0 },
        if corner & 0x2 == 0 { 0.0 } else { 1.0 },
    ]
}

/// Maps the compact DH vertex order back to the canonical face-corner order
/// used by the copied Java model-material provenance. DH serializes a quad
/// using `LodQuadBuilder.DIRECTION_VERTEX_IBO_QUAD`; its order is deliberately
/// direction-specific to preserve outward winding. Treating the serialized
/// index as a canonical UV corner rotates or mirrors atlas sprites on four
/// faces even though the semantic sprite identity and atlas region are right.
fn world_lod_serialized_vertex_canonical_corner(
    face: u32,
    serialized_vertex_index: usize,
) -> GalResult<usize> {
    if serialized_vertex_index >= 4 {
        return Err(GalError::invalid_argument(
            "world LOD quad vertex index exceeds the four-vertex DH quad contract",
        ));
    }
    let order = match WorldLodFaceNormal::try_from(
        u8::try_from(face).map_err(|_| GalError::invalid_argument("world LOD face exceeds u8"))?,
    )? {
        // DOWN and UP serialize `[10, 11, 01, 00]` in their canonical face
        // axes, while NORTH and SOUTH already serialize canonical corners.
        WorldLodFaceNormal::Down | WorldLodFaceNormal::Up => [3, 2, 1, 0],
        WorldLodFaceNormal::North | WorldLodFaceNormal::South => [0, 1, 2, 3],
        // WEST and EAST serialize `[00, 10, 11, 01]` in their local Z/Y axes.
        WorldLodFaceNormal::West | WorldLodFaceNormal::East => [0, 3, 2, 1],
    };
    Ok(order[serialized_vertex_index])
}

/// Converts typed semantic LOD geometry into a fixed, backend-neutral binary
/// payload. Vertex data is little-endian and contains, in order:
/// position `f32x3`, micro offset `f32x3`, unpremultiplied RGBA8, and four
/// semantic bytes for sky light, block light, material category, and face.
/// Indices are explicitly u32; this avoids treating DH's shared index buffer
/// as a native resource or relying on backend-default index typing.
pub(crate) fn pack_world_lod_gpu_column_asset(
    asset: &WorldLodExpandedColumnAsset,
) -> GalResult<WorldLodGpuColumnAsset> {
    let mut segments = Vec::with_capacity(asset.segments.len());
    for segment in &asset.segments {
        let vertex_capacity = segment
            .vertices
            .len()
            .checked_mul(WORLD_LOD_GPU_VERTEX_BYTES)
            .ok_or_else(|| GalError::invalid_argument("world LOD GPU vertex payload overflows"))?;
        let index_capacity = segment
            .indices
            .len()
            .checked_mul(std::mem::size_of::<u32>())
            .ok_or_else(|| GalError::invalid_argument("world LOD GPU index payload overflows"))?;
        let mut vertex_bytes = Vec::with_capacity(vertex_capacity);
        for vertex in &segment.vertices {
            write_vertex(&mut vertex_bytes, vertex);
        }
        let mut index_bytes = Vec::with_capacity(index_capacity);
        for &index in &segment.indices {
            if index as usize >= segment.vertices.len() {
                return Err(GalError::invalid_argument(format!(
                    "world LOD GPU segment index {index} exceeds {} vertices",
                    segment.vertices.len()
                )));
            }
            index_bytes.extend_from_slice(&index.to_le_bytes());
        }
        segments.push(WorldLodGpuSegment {
            layer: segment.layer,
            vertex_layout_version: WORLD_LOD_GPU_VERTEX_LAYOUT_V1,
            vertex_bytes,
            index_type: IndexType::U32,
            index_bytes,
        });
    }
    Ok(WorldLodGpuColumnAsset {
        column_key: asset.column_key,
        column_generation: asset.column_generation,
        origin: asset.origin,
        segments,
    })
}

pub(crate) fn validate_expanded_instance(
    column: &WorldLodExpandedColumnAsset,
    segment_index: u32,
    layer: u32,
) -> GalResult<()> {
    let segment = column.segments.get(segment_index as usize).ok_or_else(|| {
        GalError::invalid_argument(format!(
            "expanded world LOD column {} is missing segment {segment_index}",
            column.column_key
        ))
    })?;
    if segment.layer != layer {
        return Err(GalError::invalid_argument(
            "expanded world LOD segment layer differs from visible instance layer",
        ));
    }
    Ok(())
}

fn expand_segment(segment: &WorldLodSegment) -> GalResult<WorldLodExpandedSegment> {
    let mut vertices = Vec::with_capacity(segment.vertices.len());
    let mut indices = Vec::with_capacity(segment.vertices.len() / 4 * 6);
    for (vertex_index, vertex) in segment.vertices.iter().enumerate() {
        vertices.push(expand_vertex(vertex)?);
        if vertex_index % 4 == 3 {
            let base = u32::try_from(vertex_index - 3).map_err(|_| {
                GalError::invalid_argument("world LOD vertex index exceeds u32 range")
            })?;
            indices.extend_from_slice(&[base, base + 1, base + 2, base + 2, base + 3, base]);
        }
    }
    Ok(WorldLodExpandedSegment {
        layer: segment.layer,
        vertices,
        indices,
    })
}

fn expand_vertex(vertex: &WorldLodVertex) -> GalResult<WorldLodExpandedVertex> {
    if vertex.normal_index > WORLD_LOD_MAX_NORMAL_INDEX {
        return Err(GalError::invalid_argument(format!(
            "world LOD normal index {} is outside the six DH face directions",
            vertex.normal_index
        )));
    }
    let light = vertex.packed_light_and_micro_offset;
    let micro = (light >> 8) as u8;
    Ok(WorldLodExpandedVertex {
        local_position: vertex.local_position.map(f32::from),
        micro_offset: [
            decode_micro_axis(micro & 0b11),
            decode_micro_axis((micro >> 2) & 0b11),
            decode_micro_axis((micro >> 4) & 0b11),
        ],
        color_rgba: vertex.color_rgba.map(|channel| channel as f32 / 255.0),
        sky_light: (light & 0x0f) as u8,
        block_light: ((light >> 4) & 0x0f) as u8,
        material: WorldLodMaterialCategory::try_from(vertex.material_id)?,
        normal: WorldLodFaceNormal::try_from(vertex.normal_index)?,
    })
}

fn decode_micro_axis(bits: u8) -> f32 {
    // This intentionally matches DH's GLSL: a negative flag wins when both
    // bits are set, which is how its CPU builder represents a negative offset.
    if bits & 0b10 != 0 {
        -MICRO_OFFSET_SCALE
    } else if bits & 0b01 != 0 {
        MICRO_OFFSET_SCALE
    } else {
        0.0
    }
}

fn write_vertex(bytes: &mut Vec<u8>, vertex: &WorldLodExpandedVertex) {
    for component in vertex.local_position {
        bytes.extend_from_slice(&component.to_le_bytes());
    }
    for component in vertex.micro_offset {
        bytes.extend_from_slice(&component.to_le_bytes());
    }
    for component in vertex.color_rgba {
        bytes.push((component * 255.0).round().clamp(0.0, 255.0) as u8);
    }
    bytes.extend_from_slice(&[
        vertex.sky_light,
        vertex.block_light,
        material_category_id(vertex.material),
        face_normal_id(vertex.normal),
    ]);
    debug_assert_eq!(0, bytes.len() % WORLD_LOD_GPU_VERTEX_BYTES);
}

fn material_category_id(value: WorldLodMaterialCategory) -> u8 {
    match value {
        WorldLodMaterialCategory::Unknown => 0,
        WorldLodMaterialCategory::Leaves => 1,
        WorldLodMaterialCategory::Stone => 2,
        WorldLodMaterialCategory::Wood => 3,
        WorldLodMaterialCategory::Metal => 4,
        WorldLodMaterialCategory::Dirt => 5,
        WorldLodMaterialCategory::Lava => 6,
        WorldLodMaterialCategory::Deepslate => 7,
        WorldLodMaterialCategory::Snow => 8,
        WorldLodMaterialCategory::Sand => 9,
        WorldLodMaterialCategory::Terracotta => 10,
        WorldLodMaterialCategory::NetherStone => 11,
        WorldLodMaterialCategory::Water => 12,
        WorldLodMaterialCategory::Grass => 13,
        WorldLodMaterialCategory::Air => 14,
        WorldLodMaterialCategory::Illuminated => 15,
    }
}

fn face_normal_id(value: WorldLodFaceNormal) -> u8 {
    match value {
        WorldLodFaceNormal::Down => 0,
        WorldLodFaceNormal::Up => 1,
        WorldLodFaceNormal::North => 2,
        WorldLodFaceNormal::South => 3,
        WorldLodFaceNormal::West => 4,
        WorldLodFaceNormal::East => 5,
    }
}

fn create_column_resources(
    gal: &mut VulkanicGal,
    asset: &WorldLodGpuColumnAsset,
) -> GalResult<WorldLodGpuColumnResources> {
    let mut segments = Vec::with_capacity(asset.segments.len());
    let result = (|| -> GalResult<()> {
        for (segment_index, segment) in asset.segments.iter().enumerate() {
            let label = format!(
                "world-lod-column{}-gen{}-segment{segment_index}",
                asset.column_key, asset.column_generation
            );
            let vertex_buffer = gal.create_buffer(BufferDesc {
                label: format!("{label}.vertices"),
                size: segment.vertex_bytes.len() as u64,
                memory: MemoryDomain::Upload,
                usages: vec![
                    BufferUsage::Vertex,
                    BufferUsage::Storage,
                    BufferUsage::HostWrite,
                ],
            })?;
            let index_buffer = match gal.create_buffer(BufferDesc {
                label: format!("{label}.indices"),
                size: segment.index_bytes.len() as u64,
                memory: MemoryDomain::Upload,
                usages: vec![BufferUsage::Index, BufferUsage::HostWrite],
            }) {
                Ok(buffer) => buffer,
                Err(error) => {
                    let _ = gal.destroy(vertex_buffer);
                    return Err(error);
                }
            };
            segments.push(WorldLodGpuSegmentResources {
                vertex_buffer,
                index_buffer,
            });
        }
        Ok(())
    })();
    if let Err(error) = result {
        for segment in segments.into_iter().rev() {
            let _ = gal.destroy(segment.index_buffer);
            let _ = gal.destroy(segment.vertex_buffer);
        }
        return Err(error);
    }
    Ok(WorldLodGpuColumnResources {
        column_generation: asset.column_generation,
        segments,
    })
}

fn upload_ops(
    asset: &WorldLodGpuColumnAsset,
    resources: &WorldLodGpuColumnResources,
) -> Vec<CommandOp> {
    debug_assert_eq!(asset.segments.len(), resources.segments.len());
    let mut ops = Vec::with_capacity(asset.segments.len() * 4);
    for (segment, resources) in asset.segments.iter().zip(&resources.segments) {
        ops.push(CommandOp::HostWriteBuffer {
            buffer: resources.vertex_buffer,
            offset: 0,
            data: segment.vertex_bytes.clone(),
        });
        ops.push(CommandOp::Barrier(buffer_barrier(
            resources.vertex_buffer,
            TextureUsageState::TransferDst,
            TextureUsageState::ShaderRead,
        )));
        ops.push(CommandOp::HostWriteBuffer {
            buffer: resources.index_buffer,
            offset: 0,
            data: segment.index_bytes.clone(),
        });
        ops.push(CommandOp::Barrier(buffer_barrier(
            resources.index_buffer,
            TextureUsageState::TransferDst,
            TextureUsageState::IndexRead,
        )));
    }
    ops
}

fn create_textured_column_resources(
    gal: &mut VulkanicGal,
    asset: &WorldLodTexturedGpuColumnAsset,
) -> GalResult<WorldLodTexturedGpuColumnResources> {
    let mut segments = BTreeMap::new();
    let result = (|| -> GalResult<()> {
        for segment in &asset.segments {
            let label = format!(
                "world-lod-exact-atlas-column{}-gen{}-source-segment{}",
                asset.column_key, asset.column_generation, segment.source_segment_index
            );
            let vertex_buffer = gal.create_buffer(BufferDesc {
                label: format!("{label}.vertices"),
                size: segment.vertex_bytes.len() as u64,
                memory: MemoryDomain::Upload,
                usages: vec![
                    BufferUsage::Vertex,
                    BufferUsage::Storage,
                    BufferUsage::HostWrite,
                ],
            })?;
            let index_buffer = match gal.create_buffer(BufferDesc {
                label: format!("{label}.indices"),
                size: segment.index_bytes.len() as u64,
                memory: MemoryDomain::Upload,
                usages: vec![BufferUsage::Index, BufferUsage::HostWrite],
            }) {
                Ok(buffer) => buffer,
                Err(error) => {
                    let _ = gal.destroy(vertex_buffer);
                    return Err(error);
                }
            };
            let unresolved_index_buffer = match segment.unresolved_index_bytes.as_ref() {
                Some(bytes) => match gal.create_buffer(BufferDesc {
                    label: format!("{label}.unresolved-indices"),
                    size: bytes.len() as u64,
                    memory: MemoryDomain::Upload,
                    usages: vec![BufferUsage::Index, BufferUsage::HostWrite],
                }) {
                    Ok(buffer) => Some(buffer),
                    Err(error) => {
                        let _ = gal.destroy(index_buffer);
                        let _ = gal.destroy(vertex_buffer);
                        return Err(error);
                    }
                },
                None => None,
            };
            segments.insert(
                segment.source_segment_index,
                WorldLodTexturedGpuSegmentResources {
                    vertex_buffer,
                    index_buffer,
                    unresolved_index_buffer,
                },
            );
        }
        Ok(())
    })();
    if let Err(error) = result {
        for (_, segment) in segments.into_iter().rev() {
            if let Some(index_buffer) = segment.unresolved_index_buffer {
                let _ = gal.destroy(index_buffer);
            }
            let _ = gal.destroy(segment.index_buffer);
            let _ = gal.destroy(segment.vertex_buffer);
        }
        return Err(error);
    }
    Ok(WorldLodTexturedGpuColumnResources {
        column_generation: asset.column_generation,
        segments,
    })
}

fn textured_upload_ops(
    asset: &WorldLodTexturedGpuColumnAsset,
    resources: &WorldLodTexturedGpuColumnResources,
) -> Vec<CommandOp> {
    let mut ops = Vec::with_capacity(asset.segments.len() * 4);
    for segment in &asset.segments {
        let resources = resources
            .segments
            .get(&segment.source_segment_index)
            .expect("exact-atlas resources are created for every packed source segment");
        ops.push(CommandOp::HostWriteBuffer {
            buffer: resources.vertex_buffer,
            offset: 0,
            data: segment.vertex_bytes.clone(),
        });
        ops.push(CommandOp::Barrier(buffer_barrier(
            resources.vertex_buffer,
            TextureUsageState::TransferDst,
            TextureUsageState::ShaderRead,
        )));
        ops.push(CommandOp::HostWriteBuffer {
            buffer: resources.index_buffer,
            offset: 0,
            data: segment.index_bytes.clone(),
        });
        ops.push(CommandOp::Barrier(buffer_barrier(
            resources.index_buffer,
            TextureUsageState::TransferDst,
            TextureUsageState::IndexRead,
        )));
        if let (Some(bytes), Some(index_buffer)) = (
            segment.unresolved_index_bytes.as_ref(),
            resources.unresolved_index_buffer,
        ) {
            ops.push(CommandOp::HostWriteBuffer {
                buffer: index_buffer,
                offset: 0,
                data: bytes.clone(),
            });
            ops.push(CommandOp::Barrier(buffer_barrier(
                index_buffer,
                TextureUsageState::TransferDst,
                TextureUsageState::IndexRead,
            )));
        }
    }
    ops
}

fn buffer_barrier(
    buffer: Handle,
    before: TextureUsageState,
    after: TextureUsageState,
) -> ResourceBarrier {
    ResourceBarrier {
        resource: buffer,
        subresources: None,
        before,
        after,
        src_queue: QueueClass::Graphics,
        dst_queue: QueueClass::Graphics,
    }
}

fn texture_barrier(
    texture: Handle,
    before: TextureUsageState,
    after: TextureUsageState,
) -> ResourceBarrier {
    ResourceBarrier {
        resource: texture,
        subresources: None,
        before,
        after,
        src_queue: QueueClass::Graphics,
        dst_queue: QueueClass::Graphics,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::vulkanic::backends::{
        mock::MockBackend, presentation_capabilities, vulkan_capabilities,
    };
    use crate::render::vulkanic::handles::HandleKind;
    use crate::render::vulkanic::resources::{
        Extent3d, SamplerAddressMode, SamplerDesc, SamplerFilter, TextureDesc, TextureDimension,
        TextureUsage, TextureViewDesc,
    };
    use crate::render::vulkanic::shader_pack::distant_horizons_contract::derive_distant_horizons_opaque_contract;
    use crate::render::vulkanic::shader_pack::fullscreen::FullscreenSourcePassFrame;
    use crate::render::vulkanic::shader_pack::lowering::lower_distant_horizons_source_pair;
    use crate::render::vulkanic::shader_pack::lowering::{
        TerrainSourceOpaqueResourceBindingPlan, TerrainSourceOpaqueResourceKind,
    };
    use crate::render::vulkanic::shader_pack::preprocess::{
        complete_bundled_pack_source_for_test, preprocess_distant_horizons_sources,
    };
    use crate::render::vulkanic::shader_pack::programs::{
        TerrainSourceTextureTransforms, prepare_lowered_distant_horizons_source_program,
    };
    use crate::render::vulkanic::shader_pack::runtime::ShaderPackRuntimeExecutor;
    use crate::render::vulkanic::shader_pack::source_targets::ShaderPackColorBootstrapClearValues;
    use crate::render::vulkanic::shader_pack::terrain_contract::TerrainProgramScope;
    use crate::render::vulkanic::shader_pack::terrain_source_resources::{
        TerrainSourceOwnedResource, TerrainSourceOwnedResourceSet,
        TerrainSourceOwnedStorageResource, TerrainSourceResourceAvailability,
        TerrainSourceResourceAvailabilitySet, TerrainSourceResourceBindings,
        TerrainSourceResourceRole, TerrainSourceSampledResourceShape,
    };
    use crate::render::vulkanic::world_primitive_frontend::{
        WORLD_LOD_MATERIAL_MIXED, WORLD_LOD_MATERIAL_UNAVAILABLE, WORLD_LOD_VERTEX_LAYOUT_V1,
        WorldLodColumnAsset, WorldLodColumnMaterialProvenance, WorldLodFaceMaterial,
        WorldLodMaterialIdentity, WorldLodRenderFrame, WorldLodSegment,
        WorldLodSegmentMaterialProvenance, WorldLodVertex,
    };
    use crate::render::vulkanic::{CommandList, CommandListDesc, SubmissionBatch};

    fn asset() -> WorldLodColumnAsset {
        WorldLodColumnAsset {
            column_key: 7,
            column_generation: 3,
            vertex_layout_version: WORLD_LOD_VERTEX_LAYOUT_V1,
            origin: [-128, 64, 256],
            segments: vec![WorldLodSegment {
                layer: WORLD_LOD_LAYER_OPAQUE,
                vertices: vec![
                    WorldLodVertex {
                        local_position: [1, 2, 3],
                        packed_light_and_micro_offset: 0x0132,
                        color_rgba: [64, 128, 255, 192],
                        material_id: 4,
                        normal_index: 1,
                    },
                    WorldLodVertex {
                        local_position: [4, 2, 3],
                        packed_light_and_micro_offset: 0x0c21,
                        color_rgba: [64, 128, 255, 192],
                        material_id: 4,
                        normal_index: 1,
                    },
                    WorldLodVertex {
                        local_position: [4, 5, 3],
                        packed_light_and_micro_offset: 0x300f,
                        color_rgba: [64, 128, 255, 192],
                        material_id: 4,
                        normal_index: 1,
                    },
                    WorldLodVertex {
                        local_position: [1, 5, 3],
                        packed_light_and_micro_offset: 0x0000,
                        color_rgba: [64, 128, 255, 192],
                        material_id: 4,
                        normal_index: 1,
                    },
                ],
            }],
        }
    }

    fn single_block_face_asset() -> WorldLodColumnAsset {
        let mut asset = asset();
        let vertices = &mut asset.segments[0].vertices;
        vertices[0].local_position = [1, 2, 3];
        vertices[1].local_position = [1, 2, 4];
        vertices[2].local_position = [2, 2, 4];
        vertices[3].local_position = [2, 2, 3];
        asset
    }

    fn lowered_source_program() -> LoweredDistantHorizonsSourceProgram {
        let source = complete_bundled_pack_source_for_test();
        let contract =
            derive_distant_horizons_opaque_contract(&source, TerrainProgramScope::Overworld)
                .unwrap();
        let artifacts =
            preprocess_distant_horizons_sources(&source, &contract.source_stages).unwrap();
        let lowered =
            lower_distant_horizons_source_pair(&artifacts.vertex, &artifacts.fragment).unwrap();
        let declarations = TerrainSourceResourceBindings::from_source(&source).unwrap();
        let bindings = lowered
            .opaque_resource_contract()
            .bind_semantic_roles(&declarations)
            .unwrap();
        prepare_lowered_distant_horizons_source_program(&contract, &lowered, &bindings).unwrap()
    }

    /// Builds minimal Rust-owned physical resources for every active semantic
    /// role in a lowered program. The fixture proves descriptor/pipeline
    /// ownership and synchronization only; it deliberately supplies no Java,
    /// Iris, or backend-native resources and does not stand in for runtime
    /// texture/volume generation.
    fn owned_resources_for_programs(
        gal: &mut VulkanicGal,
        bindings: &[&TerrainSourceOpaqueResourceBindingPlan],
        shader_pack_generation: u64,
        world_generation: u64,
    ) -> (TerrainSourceOwnedResourceSet, Vec<Handle>) {
        let mut seen = BTreeSet::new();
        let mut available = Vec::new();
        let mut sampled = Vec::new();
        let mut storage = Vec::new();
        let mut handles = Vec::new();
        for plan in bindings {
            for binding in plan.bindings() {
                let role = binding.role();
                // Named shader-pack colors and DH depth snapshots are owned
                // by their explicit target caches. This test helper supplies
                // only external semantic resources; duplicating either cache
                // role would make the binding conflict that production must
                // reject.
                if matches!(
                    role,
                    TerrainSourceResourceRole::ShaderPackColor(_)
                        | TerrainSourceResourceRole::DistantHorizonsOpaqueDepth
                        | TerrainSourceResourceRole::DistantHorizonsDepthBeforeTranslucency
                ) {
                    continue;
                }
                if !seen.insert(role.clone()) {
                    continue;
                }
                let shape = role.expected_sampled_resource_shape();
                let (dimension, format, extent) = match shape {
                    TerrainSourceSampledResourceShape::Texture2d => (
                        TextureDimension::D2,
                        TextureFormat::Rgba8Unorm,
                        Extent3d {
                            width: 1,
                            height: 1,
                            depth: 1,
                        },
                    ),
                    TerrainSourceSampledResourceShape::UnsignedTexture2d => (
                        TextureDimension::D2,
                        TextureFormat::R8Uint,
                        Extent3d {
                            width: 1,
                            height: 1,
                            depth: 1,
                        },
                    ),
                    TerrainSourceSampledResourceShape::DepthCompareTexture2d => (
                        TextureDimension::D2,
                        TextureFormat::Depth32Float,
                        Extent3d {
                            width: 1,
                            height: 1,
                            depth: 1,
                        },
                    ),
                    TerrainSourceSampledResourceShape::UnsignedTexture3d => (
                        TextureDimension::D3,
                        TextureFormat::R8Uint,
                        Extent3d {
                            width: 1,
                            height: 1,
                            depth: 1,
                        },
                    ),
                    TerrainSourceSampledResourceShape::FloatTexture3d => (
                        TextureDimension::D3,
                        TextureFormat::Rgba16Float,
                        Extent3d {
                            width: 1,
                            height: 1,
                            depth: 1,
                        },
                    ),
                };
                let storage_only = binding.kind() == TerrainSourceOpaqueResourceKind::StorageImage;
                let mut usages = vec![TextureUsage::Sampled];
                if storage_only {
                    usages.push(TextureUsage::Storage);
                }
                let texture = gal
                    .create_texture(TextureDesc {
                        label: format!("test.dh-source.{}.texture", role.diagnostic_name()),
                        dimension,
                        format,
                        extent,
                        mip_levels: 1,
                        array_layers: 1,
                        usages,
                    })
                    .unwrap();
                let view = gal
                    .create_texture_view(TextureViewDesc {
                        label: format!("test.dh-source.{}.view", role.diagnostic_name()),
                        texture,
                        format,
                        base_mip: 0,
                        mip_count: 1,
                        base_layer: 0,
                        layer_count: 1,
                    })
                    .unwrap();
                available.push(TerrainSourceResourceAvailability {
                    role: role.clone(),
                    shape,
                    resource_generation: 1,
                });
                if storage_only {
                    storage.push(TerrainSourceOwnedStorageResource {
                        role,
                        texture_view: view,
                    });
                    handles.extend([view, texture]);
                    continue;
                }
                let sampler = gal
                    .create_sampler(SamplerDesc {
                        label: format!("test.dh-source.{}.sampler", role.diagnostic_name()),
                        min_filter: SamplerFilter::Nearest,
                        mag_filter: SamplerFilter::Nearest,
                        mip_filter: SamplerFilter::Nearest,
                        address_u: SamplerAddressMode::ClampToEdge,
                        address_v: SamplerAddressMode::ClampToEdge,
                        address_w: SamplerAddressMode::ClampToEdge,
                        comparison: (shape
                            == TerrainSourceSampledResourceShape::DepthCompareTexture2d)
                            .then_some(CompareOp::LessOrEqual),
                    })
                    .unwrap();
                let combined = gal
                    .create_combined_texture_sampler(CombinedTextureSamplerDesc {
                        label: format!("test.dh-source.{}.combined", role.diagnostic_name()),
                        texture_view: view,
                        sampler,
                    })
                    .unwrap();
                sampled.push(TerrainSourceOwnedResource {
                    role,
                    combined_sampler: combined,
                });
                handles.extend([combined, sampler, view, texture]);
            }
        }
        let resources = TerrainSourceOwnedResourceSet::with_storage_resources(
            TerrainSourceResourceAvailabilitySet::new(
                shader_pack_generation,
                world_generation,
                available,
            )
            .unwrap(),
            sampled,
            storage,
        )
        .unwrap();
        (resources, handles)
    }

    fn distant_source_uniforms() -> TerrainSourceUniformFrame {
        let identity = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        TerrainSourceUniformFrame {
            frame_counter: Some(1),
            render_stage: Some(0),
            frame_modulo_eight: Some(1.0),
            world_time: Some(1),
            world_day: Some(1),
            moon_phase: Some(0),
            frame_time_seconds: Some(0.016),
            frame_time_counter: Some(1.0),
            frame_time_smooth: Some(0.016),
            aspect_ratio: Some(1.5),
            blindness: Some(0.0),
            darkness_factor: Some(0.0),
            max_blindness_darkness: Some(0.0),
            sun_angle: Some(0.25),
            celestial_is_moon: Some(0),
            celestial_alpha: Some(1.0),
            celestial_sun_path_rotation: Some(0.0),
            rain_strength: Some(0.0),
            rain_factor: Some(0.0),
            thunder_strength: Some(0.0),
            sky_darken: Some(0.0),
            camera_world_position: Some([0.0, 64.0, 0.0]),
            camera_world_position_int: Some([0, 64, 0]),
            camera_world_position_fract: Some([0.0, 0.0, 0.0]),
            previous_camera_world_position: Some([0.0, 64.0, 0.0]),
            camera_velocity: Some(0.0),
            view_matrix: Some(identity),
            view_matrix_inverse: Some(identity),
            projection_matrix: Some(identity),
            projection_matrix_inverse: Some(identity),
            previous_view_matrix: Some(identity),
            previous_projection_matrix: Some(identity),
            shadow_model_view: Some(identity),
            shadow_model_view_inverse: Some(identity),
            shadow_projection: Some(identity),
            shadow_projection_inverse: Some(identity),
            distant_model_view: Some(identity),
            distant_projection: Some(identity),
            distant_projection_inverse: Some(identity),
            viewport_width: Some(96.0),
            viewport_height: Some(64.0),
            near_plane: Some(0.05),
            eye_submersion: Some(0),
            screen_brightness: Some(1.0),
            darkness_light_factor: Some(0.0),
            night_vision: Some(0.0),
            eye_brightness: Some([240, 240]),
            eye_brightness_m: Some(1.0),
            eye_brightness_m2: Some(1.0),
            fog_color: Some([0.5, 0.6, 0.7]),
            legacy_fog_parameter_color: Some([0.5, 0.6, 0.7, 1.0]),
            legacy_fog_environmental_start: Some(0.0),
            legacy_fog_environmental_end: Some(128.0),
            biome_precipitation: Some(0),
            biome_resource_location: Some("minecraft:plains".to_string()),
            biome_dry: Some(0.0),
            biome_snowy: Some(0.0),
            biome_nether_wastes: Some(0.0),
            biome_crimson_forest: Some(0.0),
            biome_warped_forest: Some(0.0),
            biome_basalt_deltas: Some(0.0),
            biome_soul_valley: Some(0.0),
            biome_pale_garden: Some(0.0),
            biome_rainy: Some(0.0),
            wetness: Some(0.0),
            sky_color: Some([0.5, 0.6, 0.7]),
            material_atlas_size: Some([256, 256]),
            far_plane: Some(128.0),
            distant_horizons_render_distance: Some(256),
            relative_eye_position: Some([0.0, 0.0, 0.0]),
            entity_id: Some(-1),
            entity_color: None,
            current_rendered_item_id: Some(-1),
            block_entity_id: Some(-1),
            held_item_id_main: Some(0),
            held_item_id_off_hand: Some(0),
            held_block_light_main: Some(0),
            held_block_light_off_hand: Some(0),
        }
    }

    fn lightmap_binding(gal: &mut VulkanicGal) -> (VanillaLightmapBinding, [Handle; 3]) {
        let texture = gal
            .create_texture(TextureDesc {
                label: "test.world-lod-lightmap.texture".to_string(),
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba8Unorm,
                extent: Extent3d {
                    width: 16,
                    height: 16,
                    depth: 1,
                },
                mip_levels: 1,
                array_layers: 1,
                usages: vec![TextureUsage::Sampled],
            })
            .unwrap();
        let sampler = gal
            .create_sampler(SamplerDesc {
                label: "test.world-lod-lightmap.sampler".to_string(),
                min_filter: SamplerFilter::Nearest,
                mag_filter: SamplerFilter::Nearest,
                mip_filter: SamplerFilter::Nearest,
                address_u: SamplerAddressMode::ClampToEdge,
                address_v: SamplerAddressMode::ClampToEdge,
                address_w: SamplerAddressMode::ClampToEdge,
                comparison: None,
            })
            .unwrap();
        let texture_view = gal
            .create_texture_view(TextureViewDesc {
                label: "test.world-lod-lightmap.view".to_string(),
                texture,
                format: TextureFormat::Rgba8Unorm,
                base_mip: 0,
                mip_count: 1,
                base_layer: 0,
                layer_count: 1,
            })
            .unwrap();
        (
            VanillaLightmapBinding {
                world_generation: 17,
                lightmap_generation: 9,
                texture_view,
                sampler,
            },
            [texture_view, sampler, texture],
        )
    }

    #[test]
    fn expands_dh_quad_layout_without_backend_vertex_assumptions() {
        let expanded = expand_world_lod_column_asset(&asset()).unwrap();
        let segment = &expanded.segments[0];
        assert_eq!(vec![0, 1, 2, 2, 3, 0], segment.indices);
        assert_eq!([-128, 64, 256], expanded.origin);
        assert_eq!([0.01, 0.0, 0.0], segment.vertices[0].micro_offset);
        assert_eq!([0.0, -0.01, 0.0], segment.vertices[1].micro_offset);
        assert_eq!([0.0, 0.0, -0.01], segment.vertices[2].micro_offset);
        assert_eq!(2, segment.vertices[0].sky_light);
        assert_eq!(3, segment.vertices[0].block_light);
        assert_eq!(
            WorldLodMaterialCategory::Metal,
            segment.vertices[0].material
        );
        assert_eq!(WorldLodFaceNormal::Up, segment.vertices[0].normal);
    }

    #[test]
    fn source_pipeline_uses_the_dh_quad_raster_convention() {
        // QuadElementBuffer emits 0,1,2,2,3,0. With the Rust Vulkan
        // whole-frame viewport convention, that DH source order is CCW.
        assert_eq!(FrontFace::CounterClockwise, WORLD_LOD_SOURCE_FRONT_FACE);
    }

    #[test]
    fn exact_atlas_plan_preserves_face_uv_orientation_without_guessing() {
        let asset = single_block_face_asset();
        let segment = &asset.segments[0];
        let material = WorldLodFaceMaterial {
            material_id: 1,
            face: 1,
            face_layer: 0,
            tinted: false,
            tint_rgb: [1.0, 1.0, 1.0],
            atlas_identity: "minecraft:blocks".to_string(),
            sprite_identity: "minecraft:block/grass_block_top".to_string(),
            atlas_uv: [0.25, 0.5, 0.375, 0.625],
            uv_corner_order: 0x78,
            variant_position: 0,
        };

        let plan = plan_world_lod_textured_segment(segment, &[1], &[material.clone()]).unwrap();

        assert!(plan.unavailable.is_empty());
        assert_eq!(1, plan.quads.len());
        assert_eq!(
            [[1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.0, 0.0]],
            plan.quads[0].vertices.map(|vertex| vertex.tile_uv)
        );
        assert_eq!(
            "minecraft:block/grass_block_top",
            plan.quads[0].sprite_identity
        );

        let rotated = WorldLodFaceMaterial {
            uv_corner_order: 0x2d,
            ..material
        };
        let plan = plan_world_lod_textured_segment(segment, &[1], &[rotated]).unwrap();
        assert_eq!(
            [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]],
            plan.quads[0].vertices.map(|vertex| vertex.tile_uv)
        );
    }

    #[test]
    fn exact_atlas_plan_preserves_coplanar_base_and_cutout_overlay_layers() {
        let asset = single_block_face_asset();
        let segment = &asset.segments[0];
        let base = WorldLodFaceMaterial {
            material_id: 1,
            face: 1,
            face_layer: 0,
            tinted: false,
            tint_rgb: [1.0, 1.0, 1.0],
            atlas_identity: "minecraft:textures/atlas/blocks.png".to_string(),
            sprite_identity: "minecraft:block/grass_block_side".to_string(),
            atlas_uv: [0.25, 0.5, 0.375, 0.625],
            uv_corner_order: 0x78,
            variant_position: 0,
        };
        let overlay = WorldLodFaceMaterial {
            face_layer: 1,
            tinted: true,
            tint_rgb: [0.25, 0.75, 0.25],
            sprite_identity: "minecraft:block/grass_block_side_overlay".to_string(),
            ..base.clone()
        };

        let plan = plan_world_lod_textured_segment(segment, &[1], &[base, overlay]).unwrap();

        assert!(plan.unavailable.is_empty());
        assert_eq!(2, plan.quads.len());
        assert_eq!(0, plan.quads[0].face_layer);
        assert!(!plan.quads[0].tinted);
        assert_eq!(
            "minecraft:block/grass_block_side",
            plan.quads[0].sprite_identity
        );
        assert_eq!(1, plan.quads[1].face_layer);
        assert!(plan.quads[1].tinted);
        assert_eq!(
            "minecraft:block/grass_block_side_overlay",
            plan.quads[1].sprite_identity
        );
        let packed = pack_world_lod_textured_column_asset(&WorldLodTexturedColumnPlan {
            column_key: asset.column_key,
            column_generation: asset.column_generation,
            segments: vec![plan],
        })
        .unwrap();
        assert_eq!(
            12,
            packed.segments[0].index_bytes.len() / std::mem::size_of::<u32>()
        );
        assert!(packed.unavailable_source_segments.is_empty());
    }

    #[test]
    fn exact_atlas_uvs_follow_dh_serialized_vertex_order_for_every_face() {
        let expectations = [
            (0, [3, 2, 1, 0]),
            (1, [3, 2, 1, 0]),
            (2, [0, 1, 2, 3]),
            (3, [0, 1, 2, 3]),
            (4, [0, 3, 2, 1]),
            (5, [0, 3, 2, 1]),
        ];
        for (face, expected) in expectations {
            let actual = std::array::from_fn(|vertex| {
                world_lod_serialized_vertex_canonical_corner(face, vertex).unwrap()
            });
            assert_eq!(expected, actual, "face {face}");
        }
        assert!(world_lod_serialized_vertex_canonical_corner(6, 0).is_err());
        assert!(world_lod_serialized_vertex_canonical_corner(0, 4).is_err());
    }

    #[test]
    fn exact_atlas_plan_rejects_incomplete_or_ambiguous_quad_provenance() {
        let asset = single_block_face_asset();
        let segment = &asset.segments[0];
        let material = WorldLodFaceMaterial {
            material_id: 1,
            face: 1,
            face_layer: 0,
            tinted: false,
            tint_rgb: [1.0, 1.0, 1.0],
            atlas_identity: "minecraft:blocks".to_string(),
            sprite_identity: "minecraft:block/grass_block_top".to_string(),
            atlas_uv: [0.25, 0.5, 0.375, 0.625],
            uv_corner_order: 0x78,
            variant_position: 0,
        };

        let unavailable = plan_world_lod_textured_segment(
            segment,
            &[WORLD_LOD_MATERIAL_UNAVAILABLE],
            &[material.clone()],
        )
        .unwrap();
        assert_eq!(
            vec![WorldLodTexturedQuadUnavailable {
                quad_index: 0,
                material_id: WORLD_LOD_MATERIAL_UNAVAILABLE,
                face: 1,
                reason: WorldLodTexturedQuadUnavailableReason::MaterialUnavailable,
            }],
            unavailable.unavailable
        );

        let mixed = plan_world_lod_textured_segment(
            segment,
            &[WORLD_LOD_MATERIAL_MIXED],
            &[material.clone()],
        )
        .unwrap();
        assert_eq!(
            WorldLodTexturedQuadUnavailableReason::MaterialMixed,
            mixed.unavailable[0].reason
        );

        let missing = plan_world_lod_textured_segment(segment, &[1], &[]).unwrap();
        assert_eq!(
            WorldLodTexturedQuadUnavailableReason::MissingFaceMaterial,
            missing.unavailable[0].reason
        );

        let mut inconsistent = segment.clone();
        inconsistent.vertices[3].normal_index = 0;
        let inconsistent =
            plan_world_lod_textured_segment(&inconsistent, &[1], &[material]).unwrap();
        assert_eq!(
            WorldLodTexturedQuadUnavailableReason::InconsistentFace,
            inconsistent.unavailable[0].reason
        );
        assert_eq!(
            "material-unavailable",
            WorldLodTexturedQuadUnavailableReason::MaterialUnavailable.as_str()
        );
        assert_eq!(
            "missing-face-material",
            WorldLodTexturedQuadUnavailableReason::MissingFaceMaterial.as_str()
        );
    }

    #[test]
    fn exact_atlas_plan_requires_the_weighted_model_position_that_selected_its_sprite() {
        let asset = single_block_face_asset();
        let segment = &asset.segments[0];
        let weighted = WorldLodFaceMaterial {
            material_id: 1,
            face: 1,
            face_layer: 0,
            tinted: false,
            tint_rgb: [1.0, 1.0, 1.0],
            atlas_identity: "minecraft:textures/atlas/blocks.png".to_string(),
            sprite_identity: "minecraft:block/oak_leaves".to_string(),
            atlas_uv: [0.125, 0.25, 0.1875, 0.3125],
            uv_corner_order: 0x78,
            variant_position: 0x1f2e_3d4c_5b6a_7988,
        };
        let exact = plan_world_lod_textured_segment_with_variants(
            segment,
            &[1],
            &[crate::render::vulkanic::world_primitive_frontend::WORLD_LOD_VARIANT_EXACT],
            &[weighted.variant_position],
            &[weighted.clone()],
        )
        .unwrap();
        assert_eq!(1, exact.quads.len());
        assert!(exact.unavailable.is_empty());

        let wrong_position = plan_world_lod_textured_segment_with_variants(
            segment,
            &[1],
            &[crate::render::vulkanic::world_primitive_frontend::WORLD_LOD_VARIANT_EXACT],
            &[weighted.variant_position.wrapping_add(1)],
            &[weighted],
        )
        .unwrap();
        assert_eq!(
            WorldLodTexturedQuadUnavailableReason::MissingFaceMaterial,
            wrong_position.unavailable[0].reason
        );

        let mixed = plan_world_lod_textured_segment_with_variants(
            segment,
            &[1],
            &[crate::render::vulkanic::world_primitive_frontend::WORLD_LOD_VARIANT_MIXED],
            &[0],
            &[],
        )
        .unwrap();
        assert_eq!(
            WorldLodTexturedQuadUnavailableReason::VariantMixed,
            mixed.unavailable[0].reason
        );
    }

    #[test]
    fn exact_atlas_plan_uses_neutral_tint_only_when_no_positioned_tint_exists() {
        let asset = single_block_face_asset();
        let segment = &asset.segments[0];
        let base = WorldLodFaceMaterial {
            material_id: 1,
            face: 1,
            face_layer: 0,
            tinted: true,
            tint_rgb: [1.0, 1.0, 1.0],
            atlas_identity: "minecraft:textures/atlas/blocks.png".to_string(),
            sprite_identity: "minecraft:block/grass_block_side_overlay".to_string(),
            atlas_uv: [0.25, 0.5, 0.375, 0.625],
            uv_corner_order: 0x78,
            variant_position: 0,
        };
        let positioned = WorldLodFaceMaterial {
            tint_rgb: [0.2, 0.7, 0.3],
            variant_position: 0x91_0000_6200_0228,
            ..base.clone()
        };

        let exact = plan_world_lod_textured_segment_with_variants(
            segment,
            &[1],
            &[crate::render::vulkanic::world_primitive_frontend::WORLD_LOD_VARIANT_EXACT],
            &[positioned.variant_position],
            &[base.clone(), positioned],
        )
        .unwrap();
        assert_eq!([0.2, 0.7, 0.3, 1.0], exact.quads[0].vertices[0].color_rgba);

        let merged = plan_world_lod_textured_segment_with_variants(
            segment,
            &[1],
            &[crate::render::vulkanic::world_primitive_frontend::WORLD_LOD_VARIANT_EXACT],
            &[0x91_0000_6200_0229],
            &[base],
        )
        .unwrap();
        assert_eq!([1.0, 1.0, 1.0, 1.0], merged.quads[0].vertices[0].color_rgba);
        assert!(merged.unavailable.is_empty());
    }

    #[test]
    fn exact_atlas_column_plan_is_generation_and_segment_order_bound() {
        let asset = single_block_face_asset();
        let provenance = WorldLodColumnMaterialProvenance {
            column_key: asset.column_key,
            column_generation: asset.column_generation,
            identities: vec![WorldLodMaterialIdentity {
                block_state_identity: "minecraft:grass_block[snowy=false]".to_string(),
                biome_identity: "minecraft:plains".to_string(),
            }],
            segments: vec![WorldLodSegmentMaterialProvenance {
                layer: WORLD_LOD_LAYER_OPAQUE,
                segment_index: 0,
                quad_material_ids: vec![1],
                quad_variant_states: vec![
                    crate::render::vulkanic::world_primitive_frontend::WORLD_LOD_VARIANT_EXACT,
                ],
                quad_variant_positions: vec![0],
            }],
            face_materials: vec![WorldLodFaceMaterial {
                material_id: 1,
                face: 1,
                face_layer: 0,
                tinted: false,
                tint_rgb: [1.0, 1.0, 1.0],
                atlas_identity: "minecraft:textures/atlas/blocks.png".to_string(),
                sprite_identity: "minecraft:block/grass_block_top".to_string(),
                atlas_uv: [0.25, 0.5, 0.375, 0.625],
                uv_corner_order: 0x78,
                variant_position: 0,
            }],
        };

        let plan = plan_world_lod_textured_column(&asset, &provenance).unwrap();
        assert_eq!(asset.column_key, plan.column_key);
        assert_eq!(asset.column_generation, plan.column_generation);
        assert_eq!(1, plan.segments.len());
        assert_eq!(1, plan.segments[0].quads.len());

        let packed = pack_world_lod_textured_column_asset(&plan).unwrap();
        assert_eq!(plan.column_key, packed.column_key);
        assert_eq!(plan.column_generation, packed.column_generation);
        assert_eq!(1, packed.segments.len());
        assert!(packed.unavailable_source_segments.is_empty());
        let segment = &packed.segments[0];
        assert_eq!(0, segment.source_segment_index);
        assert_eq!(WORLD_LOD_LAYER_OPAQUE, segment.layer);
        assert_eq!(
            WORLD_LOD_TEXTURED_GPU_VERTEX_LAYOUT_V2,
            segment.vertex_layout_version
        );
        assert_eq!(IndexType::U32, segment.index_type);
        assert_eq!(
            4 * WORLD_LOD_TEXTURED_GPU_VERTEX_BYTES,
            segment.vertex_bytes.len()
        );
        assert_eq!(6 * std::mem::size_of::<u32>(), segment.index_bytes.len());
        assert_eq!(1.0f32.to_le_bytes(), segment.vertex_bytes[0..4]);
        assert_eq!(1.0f32.to_le_bytes(), segment.vertex_bytes[24..28]);
        assert_eq!(0.25f32.to_le_bytes(), segment.vertex_bytes[32..36]);
        assert_eq!([64, 128, 255, 192], segment.vertex_bytes[48..52]);
        assert_eq!(
            material_category_id(WorldLodMaterialCategory::Metal) << 1,
            segment.vertex_bytes[55],
            "the exact-atlas source stream preserves the DH material category beside its tint bit"
        );

        let mut wrong_atlas = plan.clone();
        wrong_atlas.segments[0].quads[0].atlas_identity =
            "minecraft:textures/atlas/items.png".to_string();
        let wrong_atlas = pack_world_lod_textured_column_asset(&wrong_atlas).unwrap();
        assert!(wrong_atlas.segments.is_empty());
        assert_eq!(vec![0], wrong_atlas.unavailable_source_segments);

        let partial = WorldLodTexturedColumnPlan {
            column_key: plan.column_key,
            column_generation: plan.column_generation,
            segments: vec![WorldLodTexturedSegmentPlan {
                layer: WORLD_LOD_LAYER_OPAQUE,
                source_quad_count: 2,
                quads: vec![plan.segments[0].quads[0].clone()],
                unavailable: vec![WorldLodTexturedQuadUnavailable {
                    quad_index: 1,
                    material_id: 1,
                    face: 1,
                    reason: WorldLodTexturedQuadUnavailableReason::MaterialUnavailable,
                }],
            }],
        };
        let partial = pack_world_lod_textured_column_asset(&partial).unwrap();
        assert_eq!(
            1,
            partial.segments.len(),
            "known quads retain their exact atlas payload"
        );
        assert_eq!(
            vec![0],
            partial.unavailable_source_segments,
            "the source segment retains a complementary unresolved range"
        );
        assert_eq!(
            Some(6 * std::mem::size_of::<u32>()),
            partial.segments[0]
                .unresolved_index_bytes
                .as_ref()
                .map(Vec::len),
            "only the unavailable quad remains in the coarse index stream"
        );
        assert_eq!(
            Some(4u32.to_le_bytes()),
            partial.segments[0]
                .unresolved_index_bytes
                .as_ref()
                .map(|bytes| bytes[0..4].try_into().unwrap()),
            "the complementary range starts at the unresolved source quad"
        );

        let wrong_generation = WorldLodColumnMaterialProvenance {
            column_generation: asset.column_generation + 1,
            ..provenance.clone()
        };
        assert!(
            plan_world_lod_textured_column(&asset, &wrong_generation)
                .unwrap_err()
                .to_string()
                .contains("generation")
        );

        let wrong_segment = WorldLodColumnMaterialProvenance {
            segments: vec![WorldLodSegmentMaterialProvenance {
                segment_index: 1,
                ..provenance.segments[0].clone()
            }],
            ..provenance
        };
        assert!(
            plan_world_lod_textured_column(&asset, &wrong_segment)
                .unwrap_err()
                .to_string()
                .contains("layer/order")
        );
    }

    #[test]
    fn exact_atlas_plan_preserves_merged_dh_tile_repetition_without_atlas_bleed() {
        let asset = asset();
        let segment = &asset.segments[0];
        let material = WorldLodFaceMaterial {
            material_id: 1,
            face: 1,
            face_layer: 0,
            tinted: false,
            tint_rgb: [1.0, 1.0, 1.0],
            atlas_identity: "minecraft:textures/atlas/blocks.png".to_string(),
            sprite_identity: "minecraft:block/grass_block_top".to_string(),
            atlas_uv: [0.25, 0.5, 0.375, 0.625],
            uv_corner_order: 0x78,
            variant_position: 0,
        };

        let plan = plan_world_lod_textured_segment(segment, &[1], &[material.clone()]).unwrap();
        assert!(plan.unavailable.is_empty());
        assert_eq!(1, plan.quads.len());
        assert_eq!(
            [[3.0, 0.0], [0.0, 0.0], [0.0, 0.0], [3.0, 0.0]],
            plan.quads[0].vertices.map(|vertex| vertex.tile_uv),
            "the material shader receives unwrapped repeat coordinates rather than one stretched atlas UV range"
        );
        assert_eq!(
            [3.0, 0.0],
            world_lod_textured_quad_tile_span(&plan.quads[0])
        );
        assert!(
            plan.quads[0]
                .vertices
                .iter()
                .all(|vertex| vertex.atlas_rect == material.atlas_uv)
        );
    }

    #[test]
    fn lightmap_binding_key_distinguishes_rebuilt_residencies() {
        let first = VanillaLightmapBinding {
            world_generation: 17,
            lightmap_generation: 9,
            texture_view: Handle::new(HandleKind::TextureView, 4, 1).unwrap(),
            sampler: Handle::new(HandleKind::Sampler, 5, 1).unwrap(),
        };
        let replacement = VanillaLightmapBinding {
            texture_view: Handle::new(HandleKind::TextureView, 6, 1).unwrap(),
            sampler: Handle::new(HandleKind::Sampler, 7, 1).unwrap(),
            ..first
        };

        assert_ne!(
            WorldLodLightmapResourceKey::from(first),
            WorldLodLightmapResourceKey::from(replacement)
        );
    }

    #[test]
    fn rejects_unknown_semantic_face_normal() {
        let mut invalid = asset();
        invalid.segments[0].vertices[0].normal_index = 6;
        let error = expand_world_lod_column_asset(&invalid).unwrap_err();
        assert!(error.to_string().contains("face normal"));
    }

    #[test]
    fn packs_owned_gpu_bytes_without_reusing_dh_vertex_layout() {
        let expanded = expand_world_lod_column_asset(&asset()).unwrap();
        let packed = pack_world_lod_gpu_column_asset(&expanded).unwrap();
        let segment = &packed.segments[0];
        assert_eq!(
            WORLD_LOD_GPU_VERTEX_LAYOUT_V1,
            segment.vertex_layout_version
        );
        assert_eq!(IndexType::U32, segment.index_type);
        assert_eq!(4 * WORLD_LOD_GPU_VERTEX_BYTES, segment.vertex_bytes.len());
        assert_eq!(6 * std::mem::size_of::<u32>(), segment.index_bytes.len());
        assert_eq!(1.0f32.to_le_bytes(), segment.vertex_bytes[0..4]);
        assert_eq!(0.01f32.to_le_bytes(), segment.vertex_bytes[12..16]);
        assert_eq!([64, 128, 255, 192], segment.vertex_bytes[24..28]);
        assert_eq!([2, 3, 4, 1], segment.vertex_bytes[28..32]);
        assert_eq!(
            [0u32, 1, 2, 2, 3, 0]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
            segment.index_bytes
        );
    }

    #[test]
    fn rejects_out_of_range_expanded_index_before_gpu_asset_creation() {
        let mut expanded = expand_world_lod_column_asset(&asset()).unwrap();
        expanded.segments[0].indices[0] = 4;
        let error = pack_world_lod_gpu_column_asset(&expanded).unwrap_err();
        assert!(error.to_string().contains("exceeds 4 vertices"));
    }

    #[test]
    fn resolves_pending_uploads_as_generation_checked_draw_ranges() {
        let expanded = expand_world_lod_column_asset(&asset()).unwrap();
        let packed = pack_world_lod_gpu_column_asset(&expanded).unwrap();
        let assets = BTreeMap::from([(packed.column_key, packed)]);
        let instance = WorldLodColumnInstanceRequest {
            column_key: 7,
            column_generation: 3,
            layer: WORLD_LOD_LAYER_OPAQUE,
            segment_index: 0,
            order: 9,
        };
        let mut gal = VulkanicGal::new_with_backend(
            Box::new(MockBackend::with_capabilities(presentation_capabilities(
                vulkan_capabilities(),
            ))),
            false,
        );
        let mut residency = WorldLodGpuResidency::default();
        let mut ops = Vec::new();
        residency
            .stage_visible_uploads(&mut gal, &assets, &[instance], &mut ops)
            .unwrap();
        let draws = residency
            .resolve_visible_draws(&assets, &[instance])
            .unwrap();
        assert_eq!(1, draws.len());
        assert_eq!(7, draws[0].column_key);
        assert_eq!(3, draws[0].column_generation);
        assert_eq!([-128, 64, 256], draws[0].origin);
        assert_eq!(WORLD_LOD_LAYER_OPAQUE, draws[0].layer);
        assert_eq!(IndexType::U32, draws[0].index_type);
        assert_eq!(6, draws[0].index_count);
        assert!(!draws[0].vertex_buffer.is_null());
        assert!(!draws[0].index_buffer.is_null());

        let stale = WorldLodColumnInstanceRequest {
            column_generation: 4,
            ..instance
        };
        assert!(residency.resolve_visible_draws(&assets, &[stale]).is_err());
        residency.discard_submission(&mut gal);
    }

    #[test]
    fn draw_uniform_preserves_resolved_frame_and_column_origin_without_legacy_state() {
        let expanded = expand_world_lod_column_asset(&asset()).unwrap();
        let packed = pack_world_lod_gpu_column_asset(&expanded).unwrap();
        let assets = BTreeMap::from([(packed.column_key, packed)]);
        let instance = WorldLodColumnInstanceRequest {
            column_key: 7,
            column_generation: 3,
            layer: WORLD_LOD_LAYER_OPAQUE,
            segment_index: 0,
            order: 0,
        };
        let mut gal = VulkanicGal::new_with_backend(
            Box::new(MockBackend::with_capabilities(presentation_capabilities(
                vulkan_capabilities(),
            ))),
            false,
        );
        let mut residency = WorldLodGpuResidency::default();
        let mut ops = Vec::new();
        residency
            .stage_visible_uploads(&mut gal, &assets, &[instance], &mut ops)
            .unwrap();
        let draw = residency
            .resolve_visible_draws(&assets, &[instance])
            .unwrap()[0];
        let frame = WorldLodRenderFrame {
            enabled: true,
            flags: 0b1011,
            world_y_offset: -64,
            combined_matrix: [
                1.0, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 3.0, 0.0, 4.0, 5.0, 6.0, 1.0,
            ],
            clip_distance: 24.0,
            micro_offset: 0.01,
            noise_intensity: 0.25,
            earth_radius: 6_371_000.0,
            noise_steps: 4,
            noise_dropoff: 96,
            ..WorldLodRenderFrame::default()
        };
        let uniform = WorldLodDrawUniform::from_semantics(&frame, draw).unwrap();
        assert_eq!(
            [-128.0, 64.0, 256.0, -64.0],
            uniform.column_origin_and_world_y
        );
        assert_eq!(
            [-128.0, 64.0, 256.0, 0.0],
            uniform.model_offset_and_reserved
        );
        assert_eq!(
            [24.0, 0.01, 0.25, 6_371_000.0],
            uniform.clip_micro_noise_earth
        );
        assert_eq!([0b1011, 4, 96, 0], uniform.flags_and_noise);
        let packed = uniform.pack_std140();
        assert_eq!(128, packed.len());
        assert_eq!(1.0f32.to_ne_bytes(), packed[0..4]);
        assert_eq!((-128.0f32).to_ne_bytes(), packed[64..68]);
        assert_eq!(0b1011u32.to_ne_bytes(), packed[112..116]);
        residency.discard_submission(&mut gal);
    }

    #[test]
    fn column_uniform_keeps_world_origin_separate_from_dh_camera_relative_offset() {
        let frame = WorldLodRenderFrame {
            enabled: true,
            flags: 0,
            combined_matrix: [1.0; 16],
            clip_distance: 1.0,
            micro_offset: 0.01,
            earth_radius: 1.0,
            ..WorldLodRenderFrame::default()
        };
        let draw = WorldLodGpuDraw {
            column_key: 1,
            column_generation: 1,
            origin: [160, 72, -544],
            layer: WORLD_LOD_LAYER_OPAQUE,
            segment_index: 0,
            order: 0,
            vertex_buffer: Handle::from_raw(1),
            index_buffer: Handle::from_raw(2),
            index_type: IndexType::U16,
            index_count: 3,
        };
        let uniform =
            WorldLodDrawUniform::from_semantics_with_camera(&frame, draw, [150.5, 64.0, -530.25])
                .unwrap();
        assert_eq!(
            [160.0, 72.0, -544.0, 0.0],
            uniform.column_origin_and_world_y
        );
        assert_eq!([9.5, 8.0, -13.75, 0.0], uniform.model_offset_and_reserved);
    }

    #[test]
    fn source_pass_stages_owned_column_data_without_selecting_pack_resources() {
        let expanded = expand_world_lod_column_asset(&asset()).unwrap();
        let packed = pack_world_lod_gpu_column_asset(&expanded).unwrap();
        let assets = BTreeMap::from([(packed.column_key, packed)]);
        let instance = WorldLodColumnInstanceRequest {
            column_key: 7,
            column_generation: 3,
            layer: WORLD_LOD_LAYER_OPAQUE,
            segment_index: 0,
            order: 0,
        };
        let mut gal = VulkanicGal::new_with_backend(
            Box::new(MockBackend::with_capabilities(presentation_capabilities(
                vulkan_capabilities(),
            ))),
            false,
        );
        let mut residency = WorldLodGpuResidency::default();
        let mut upload_ops = Vec::new();
        residency
            .stage_visible_uploads(&mut gal, &assets, &[instance], &mut upload_ops)
            .unwrap();
        let draw = residency
            .resolve_visible_draws(&assets, &[instance])
            .unwrap()[0];
        let frame = WorldLodRenderFrame {
            enabled: true,
            combined_matrix: [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
            model_view_matrix: [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
            projection_matrix: [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
            projection_inverse_matrix: [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
            clip_distance: 24.0,
            micro_offset: MICRO_OFFSET_SCALE,
            noise_intensity: 0.25,
            earth_radius: 6_371_000.0,
            noise_steps: 4,
            noise_dropoff: 96,
            ..WorldLodRenderFrame::default()
        };
        let program = lowered_source_program();
        let mut source_resources = WorldLodSourcePassResources::default();
        let mut ops = Vec::new();
        let prepared = source_resources
            .stage_draw(
                &mut gal,
                &program,
                TextureFormat::Rgba8Unorm,
                draw,
                WorldLodDrawUniform::from_semantics(&frame, draw).unwrap(),
                &distant_source_uniforms(),
                &mut ops,
            )
            .unwrap();

        assert_eq!(Some(HandleKind::GraphicsPipeline), prepared.pipeline.kind());
        assert_eq!(
            Some(HandleKind::PipelineLayout),
            prepared.pipeline_layout.kind()
        );
        assert_eq!(
            Some(HandleKind::ResourceSet),
            prepared.source_data_set.kind()
        );
        assert_eq!(
            Some(HandleKind::ResourceLayout),
            prepared.pack_resources_layout.kind()
        );
        assert_eq!([0, 0], prepared.source_data_dynamic_offsets);
        assert_eq!(2, prepared.source_data_dynamic_offset_count);
        assert_eq!(draw.index_buffer, prepared.index_buffer);
        assert_eq!(draw.index_type, prepared.index_type);
        assert_eq!(draw.index_count, prepared.index_count);
        assert_eq!(6, ops.len());
        assert_eq!(
            2,
            ops.iter()
                .filter(|op| matches!(op, CommandOp::HostWriteBuffer { .. }))
                .count()
        );
        assert!(
            ops.iter()
                .all(|op| !format!("{op:?}").contains("material-id"))
        );

        let mut cached_ops = Vec::new();
        let cached = source_resources
            .stage_draw(
                &mut gal,
                &program,
                TextureFormat::Rgba8Unorm,
                draw,
                WorldLodDrawUniform::from_semantics(&frame, draw).unwrap(),
                &distant_source_uniforms(),
                &mut cached_ops,
            )
            .unwrap();
        assert_eq!(prepared.pipeline, cached.pipeline);
        assert_eq!(prepared.source_data_set, cached.source_data_set);
        assert_eq!(
            prepared.source_data_dynamic_offsets,
            cached.source_data_dynamic_offsets
        );
        assert_eq!(
            prepared.source_data_dynamic_offset_count,
            cached.source_data_dynamic_offset_count
        );
        assert_eq!(6, cached_ops.len());

        source_resources.destroy(&mut gal);
        residency.discard_submission(&mut gal);
    }

    #[test]
    fn source_pass_rejects_incomplete_semantic_pack_resources_before_set_one_creation() {
        let mut gal = VulkanicGal::new_with_backend(
            Box::new(MockBackend::with_capabilities(presentation_capabilities(
                vulkan_capabilities(),
            ))),
            false,
        );
        let program = lowered_source_program();
        let empty = TerrainSourceOwnedResourceSet::new(
            TerrainSourceResourceAvailabilitySet::new(program.shader_pack_generation, 17, [])
                .unwrap(),
            [],
        )
        .unwrap();
        let mut source_resources = WorldLodSourcePassResources::default();

        let error = source_resources
            .stage_pack_resources(&mut gal, &program, TextureFormat::Rgba8Unorm, &empty)
            .unwrap_err();
        assert!(error.to_string().contains("unavailable"));
        assert!(source_resources.pack_resources.is_empty());
        assert!(source_resources.pipelines.is_empty());
    }

    #[test]
    fn opaque_layer_admission_preserves_uniforms_and_requires_explicit_transparent_path() {
        let expanded = expand_world_lod_column_asset(&asset()).unwrap();
        let packed = pack_world_lod_gpu_column_asset(&expanded).unwrap();
        let assets = BTreeMap::from([(packed.column_key, packed)]);
        let instance = WorldLodColumnInstanceRequest {
            column_key: 7,
            column_generation: 3,
            layer: WORLD_LOD_LAYER_OPAQUE,
            segment_index: 0,
            order: 0,
        };
        let mut gal = VulkanicGal::new_with_backend(
            Box::new(MockBackend::with_capabilities(presentation_capabilities(
                vulkan_capabilities(),
            ))),
            false,
        );
        let mut residency = WorldLodGpuResidency::default();
        let mut ops = Vec::new();
        residency
            .stage_visible_uploads(&mut gal, &assets, &[instance], &mut ops)
            .unwrap();
        let draw = residency
            .resolve_visible_draws(&assets, &[instance])
            .unwrap()[0];
        let frame = WorldLodRenderFrame {
            enabled: true,
            combined_matrix: [1.0; 16],
            micro_offset: MICRO_OFFSET_SCALE,
            ..WorldLodRenderFrame::default()
        };
        let admitted = admit_world_lod_draw(&frame, draw).unwrap();
        assert_eq!(WorldLodPassClass::Opaque, admitted.pass);
        assert_eq!(draw.column_key, admitted.draw.column_key);
        assert_eq!(
            draw.origin[0] as f32,
            admitted.uniforms.column_origin_and_world_y[0]
        );
        assert_eq!(WorldLodMaterialContract::OPAQUE, admitted.material_contract);
        assert!(admitted.material_contract.requires_vanilla_lightmap);
        assert!(admitted.material_contract.uses_vertex_color);
        let water_error = admit_world_lod_transparent_draw(
            &frame,
            WorldLodGpuDraw {
                layer: WORLD_LOD_LAYER_TRANSPARENT_WATER_UP,
                ..draw
            },
        )
        .unwrap_err();
        assert!(
            water_error
                .to_string()
                .contains("explicit water-surface admission path")
        );

        let transparent = WorldLodGpuDraw {
            layer: WORLD_LOD_LAYER_TRANSPARENT_SIDE,
            ..draw
        };
        let error = admit_world_lod_draw(&frame, transparent).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("explicit transparent admission path")
        );

        let unknown = WorldLodGpuDraw { layer: 99, ..draw };
        let error = admit_world_lod_draw(&frame, unknown).unwrap_err();
        assert!(error.to_string().contains("unknown Distant Horizons layer"));
        residency.discard_submission(&mut gal);
    }

    #[test]
    fn frame_plan_preserves_transparent_and_water_draws_in_visible_list_order() {
        let expanded = expand_world_lod_column_asset(&asset()).unwrap();
        let packed = pack_world_lod_gpu_column_asset(&expanded).unwrap();
        let assets = BTreeMap::from([(packed.column_key, packed)]);
        let instance = WorldLodColumnInstanceRequest {
            column_key: 7,
            column_generation: 3,
            layer: WORLD_LOD_LAYER_OPAQUE,
            segment_index: 0,
            order: 0,
        };
        let mut gal = VulkanicGal::new_with_backend(
            Box::new(MockBackend::with_capabilities(presentation_capabilities(
                vulkan_capabilities(),
            ))),
            false,
        );
        let mut residency = WorldLodGpuResidency::default();
        let mut ops = Vec::new();
        residency
            .stage_visible_uploads(&mut gal, &assets, &[instance], &mut ops)
            .unwrap();
        let draw = residency
            .resolve_visible_draws(&assets, &[instance])
            .unwrap()[0];
        let frame = WorldLodRenderFrame {
            enabled: true,
            combined_matrix: [1.0; 16],
            micro_offset: MICRO_OFFSET_SCALE,
            ..WorldLodRenderFrame::default()
        };
        let transparent_later = WorldLodGpuDraw {
            layer: WORLD_LOD_LAYER_TRANSPARENT_UP,
            order: 9,
            ..draw
        };
        let transparent_earlier = WorldLodGpuDraw {
            layer: WORLD_LOD_LAYER_TRANSPARENT_SIDE,
            order: 3,
            ..draw
        };
        let water = WorldLodGpuDraw {
            layer: WORLD_LOD_LAYER_TRANSPARENT_WATER_UP,
            order: 11,
            ..draw
        };
        let plan = plan_world_lod_frame(
            &frame,
            &[draw, transparent_later, water, transparent_earlier],
        )
        .unwrap();
        assert_eq!(1, plan.opaque_draws.len());
        assert_eq!(2, plan.transparent_draws.len());
        assert_eq!(1, plan.water_draws.len());
        assert_eq!(
            WORLD_LOD_LAYER_TRANSPARENT_WATER_UP,
            plan.water_draws[0].draw.layer
        );
        assert_eq!(11, plan.water_draws[0].draw.order);
        assert_eq!(WorldLodPassClass::WaterSurface, plan.water_draws[0].pass);
        assert_eq!(
            WorldLodMaterialContract::WATER_SURFACE,
            plan.water_draws[0].material_contract
        );
        assert_eq!(3, plan.transparent_draws[0].draw.order);
        assert_eq!(
            WORLD_LOD_LAYER_TRANSPARENT_SIDE,
            plan.transparent_draws[0].draw.layer
        );
        assert_eq!(
            WorldLodPassClass::Transparent,
            plan.transparent_draws[0].pass
        );
        assert_eq!(
            WorldLodMaterialContract::TRANSPARENT,
            plan.transparent_draws[0].material_contract
        );
        assert_eq!(9, plan.transparent_draws[1].draw.order);
        residency.discard_submission(&mut gal);
    }

    #[test]
    fn opaque_pass_resources_reuse_generation_keyed_bindings_and_only_write_frame_uniforms() {
        let expanded = expand_world_lod_column_asset(&asset()).unwrap();
        let packed = pack_world_lod_gpu_column_asset(&expanded).unwrap();
        let assets = BTreeMap::from([(packed.column_key, packed)]);
        let instance = WorldLodColumnInstanceRequest {
            column_key: 7,
            column_generation: 3,
            layer: WORLD_LOD_LAYER_OPAQUE,
            segment_index: 0,
            order: 0,
        };
        let mut gal = VulkanicGal::new_with_backend(
            Box::new(MockBackend::with_capabilities(presentation_capabilities(
                vulkan_capabilities(),
            ))),
            false,
        );
        let mut residency = WorldLodGpuResidency::default();
        let mut upload_ops = Vec::new();
        residency
            .stage_visible_uploads(&mut gal, &assets, &[instance], &mut upload_ops)
            .unwrap();
        let draw = residency
            .resolve_visible_draws(&assets, &[instance])
            .unwrap()[0];
        let frame = WorldLodRenderFrame {
            enabled: true,
            combined_matrix: [1.0; 16],
            micro_offset: MICRO_OFFSET_SCALE,
            ..WorldLodRenderFrame::default()
        };
        let admitted = admit_world_lod_draw(&frame, draw).unwrap();
        let mut pass_resources = WorldLodOpaquePassResources::default();
        let (lightmap, lightmap_handles) = lightmap_binding(&mut gal);

        let creates_before = gal.metrics().resource_creates;
        let mut first_ops = Vec::new();
        let first = pass_resources
            .stage_draw(&mut gal, admitted, lightmap, &mut first_ops)
            .unwrap();
        let creates_after_first = gal.metrics().resource_creates;
        assert!(creates_after_first > creates_before);
        assert_eq!(3, first_ops.len());
        assert!(matches!(first_ops[0], CommandOp::Barrier(_)));
        assert!(matches!(
            &first_ops[1],
            CommandOp::HostWriteBuffer { data, .. } if data.len() == 128
        ));
        assert!(matches!(first_ops[2], CommandOp::Barrier(_)));

        let mut second_ops = Vec::new();
        let second = pass_resources
            .stage_draw(&mut gal, admitted, lightmap, &mut second_ops)
            .unwrap();
        assert_eq!(creates_after_first, gal.metrics().resource_creates);
        assert_eq!(first.pipeline, second.pipeline);
        assert_eq!(first.pipeline_layout, second.pipeline_layout);
        assert_eq!(first.geometry_resource_set, second.geometry_resource_set);
        assert_eq!(first.lightmap_resource_set, second.lightmap_resource_set);
        assert_eq!(first.index_buffer, second.index_buffer);
        assert_eq!(IndexType::U32, first.index_type);
        assert_eq!(6, first.index_count);
        assert_eq!(3, second_ops.len());
        assert!(matches!(
            &second_ops[1],
            CommandOp::HostWriteBuffer { data, .. } if data.len() == 128
        ));

        pass_resources.reconcile_assets(&mut gal, &BTreeMap::new());
        assert!(pass_resources.inner.draws.is_empty());
        assert_eq!(
            1,
            pass_resources.inner.lightmaps.len(),
            "lightmap ownership follows the shader/runtime generation, not one column asset"
        );
        pass_resources.clear_lightmap_bindings(&mut gal);
        assert!(pass_resources.inner.lightmaps.is_empty());
        for handle in lightmap_handles {
            gal.destroy(handle).unwrap();
        }
        pass_resources.destroy(&mut gal);
        residency.discard_submission(&mut gal);
    }

    #[test]
    fn transparent_pass_uses_the_shared_semantic_stream_with_distinct_alpha_policy() {
        let expanded = expand_world_lod_column_asset(&asset()).unwrap();
        let packed = pack_world_lod_gpu_column_asset(&expanded).unwrap();
        let assets = BTreeMap::from([(packed.column_key, packed)]);
        let instance = WorldLodColumnInstanceRequest {
            column_key: 7,
            column_generation: 3,
            layer: WORLD_LOD_LAYER_OPAQUE,
            segment_index: 0,
            order: 4,
        };
        let mut gal = VulkanicGal::new_with_backend(
            Box::new(MockBackend::with_capabilities(presentation_capabilities(
                vulkan_capabilities(),
            ))),
            false,
        );
        let mut residency = WorldLodGpuResidency::default();
        let mut upload_ops = Vec::new();
        residency
            .stage_visible_uploads(&mut gal, &assets, &[instance], &mut upload_ops)
            .unwrap();
        let draw = residency
            .resolve_visible_draws(&assets, &[instance])
            .unwrap()[0];
        let frame = WorldLodRenderFrame {
            enabled: true,
            combined_matrix: [1.0; 16],
            micro_offset: MICRO_OFFSET_SCALE,
            ..WorldLodRenderFrame::default()
        };
        let admitted = admit_world_lod_transparent_draw(
            &frame,
            WorldLodGpuDraw {
                layer: WORLD_LOD_LAYER_TRANSPARENT_SIDE,
                ..draw
            },
        )
        .unwrap();
        assert_eq!(WorldLodPassClass::Transparent, admitted.pass);
        assert_eq!(
            WorldLodMaterialContract::TRANSPARENT,
            admitted.material_contract
        );
        let (lightmap, lightmap_handles) = lightmap_binding(&mut gal);
        let mut pass_resources = WorldLodTransparentPassResources::default();
        let mut ops = Vec::new();
        let prepared = pass_resources
            .stage_draw(&mut gal, admitted, lightmap, &mut ops)
            .unwrap();
        assert!(!prepared.pipeline.is_null());
        assert!(!prepared.geometry_resource_set.is_null());
        assert_eq!(3, ops.len());
        assert!(matches!(
            &ops[1],
            CommandOp::HostWriteBuffer { data, .. } if data.len() == 128
        ));
        assert_eq!(1, pass_resources.inner.draws.len());
        assert_eq!(1, pass_resources.inner.lightmaps.len());
        pass_resources.destroy(&mut gal);
        for handle in lightmap_handles {
            gal.destroy(handle).unwrap();
        }
        residency.discard_submission(&mut gal);
    }

    #[test]
    fn source_target_cache_owns_a_generation_bound_distant_depth_target() {
        let mut gal = VulkanicGal::new_with_backend(
            Box::new(MockBackend::with_capabilities(presentation_capabilities(
                vulkan_capabilities(),
            ))),
            false,
        );
        let first_identity = WorldLodSourceTargetIdentity {
            world_generation: 17,
            shader_pack_generation: 91,
            extent: Extent3d {
                width: 320,
                height: 180,
                depth: 1,
            },
        };
        let mut cache = WorldLodSourceTargetCache::default();
        let creates_before = gal.metrics().resource_creates;
        let first = cache.stage(&mut gal, first_identity).unwrap();
        assert!(!first.distant_depth_texture.is_null());
        assert!(!first.distant_depth_view.is_null());
        assert!(!first.distant_depth_before_translucency_texture.is_null());
        assert!(!first.distant_depth_before_translucency_view.is_null());
        assert_ne!(
            first.distant_depth_texture,
            first.distant_depth_before_translucency_texture
        );
        assert_eq!(
            10,
            gal.metrics().resource_creates - creates_before,
            "one source generation owns two depth textures/views/samplers, two combined samplers, and its explicit depth-clear target/pass"
        );
        cache.confirm_submission(&mut gal);
        assert_eq!(Some(first_identity), cache.active_identity());

        let reused = cache.stage(&mut gal, first_identity).unwrap();
        assert_eq!(first.distant_depth_texture, reused.distant_depth_texture);
        assert_eq!(first.distant_depth_view, reused.distant_depth_view);
        assert_eq!(
            first.distant_depth_before_translucency_texture,
            reused.distant_depth_before_translucency_texture
        );
        assert_eq!(
            10,
            gal.metrics().resource_creates - creates_before,
            "reusing the active source generation must not recreate any depth resources"
        );

        let resized_identity = WorldLodSourceTargetIdentity {
            extent: Extent3d {
                width: 640,
                height: 360,
                depth: 1,
            },
            ..first_identity
        };
        let replacement = cache.stage(&mut gal, resized_identity).unwrap();
        assert_ne!(
            first.distant_depth_texture,
            replacement.distant_depth_texture
        );
        assert_eq!(Some(first_identity), cache.active_identity());
        cache.discard_submission(&mut gal);
        assert_eq!(Some(first_identity), cache.active_identity());

        let replacement = cache.stage(&mut gal, resized_identity).unwrap();
        cache.confirm_submission(&mut gal);
        assert_eq!(Some(resized_identity), cache.active_identity());
        assert_ne!(first.distant_depth_view, replacement.distant_depth_view);
        cache.destroy(&mut gal);
        assert_eq!(None, cache.active_identity());
    }

    #[test]
    fn source_target_snapshot_keeps_live_and_pre_translucency_depth_distinct() {
        let mut gal = VulkanicGal::new_with_backend(
            Box::new(MockBackend::with_capabilities(presentation_capabilities(
                vulkan_capabilities(),
            ))),
            false,
        );
        let identity = WorldLodSourceTargetIdentity {
            world_generation: 17,
            shader_pack_generation: 91,
            extent: Extent3d {
                width: 320,
                height: 180,
                depth: 1,
            },
        };
        let mut cache = WorldLodSourceTargetCache::default();
        let targets = cache.stage(&mut gal, identity).unwrap();
        let mut ops = Vec::new();
        targets.append_opaque_depth_snapshot(&mut ops);

        assert!(matches!(
            ops.as_slice(),
            [
                CommandOp::Barrier(ResourceBarrier {
                    resource: live_before,
                    before: TextureUsageState::DepthStencilAttachment,
                    after: TextureUsageState::TransferSrc,
                    ..
                }),
                CommandOp::Barrier(ResourceBarrier {
                    resource: snapshot_before,
                    before: TextureUsageState::Undefined,
                    after: TextureUsageState::TransferDst,
                    ..
                }),
                CommandOp::CopyTexture(TextureImageCopyRegion {
                    row_order: crate::render::vulkanic::commands::TextureRowOrder::Preserve,
                    src_texture,
                    dst_texture,
                    extent: Extent3d { width: 320, height: 180, depth: 1 },
                    ..
                }),
                CommandOp::Barrier(ResourceBarrier {
                    resource: live_after,
                    before: TextureUsageState::TransferSrc,
                    after: TextureUsageState::ShaderRead,
                    ..
                }),
                CommandOp::Barrier(ResourceBarrier {
                    resource: snapshot_after,
                    before: TextureUsageState::TransferDst,
                    after: TextureUsageState::ShaderRead,
                    ..
                }),
            ] if *live_before == targets.distant_depth_texture
                && *snapshot_before == targets.distant_depth_before_translucency_texture
                && *src_texture == targets.distant_depth_texture
                && *dst_texture == targets.distant_depth_before_translucency_texture
                && *live_after == targets.distant_depth_texture
                && *snapshot_after == targets.distant_depth_before_translucency_texture
        ));
        cache.discard_submission(&mut gal);
    }

    #[test]
    fn source_targets_expose_distinct_generation_bound_depth_resources() {
        let mut gal = VulkanicGal::new_with_backend(
            Box::new(MockBackend::with_capabilities(presentation_capabilities(
                vulkan_capabilities(),
            ))),
            false,
        );
        let identity = WorldLodSourceTargetIdentity {
            world_generation: 17,
            shader_pack_generation: 91,
            extent: Extent3d {
                width: 320,
                height: 180,
                depth: 1,
            },
        };
        let mut cache = WorldLodSourceTargetCache::default();
        let targets = cache.stage(&mut gal, identity).unwrap();
        let resources = targets.semantic_resources().unwrap();
        assert_eq!(2, resources.len());
        assert_eq!(
            Some(targets.distant_depth_combined_sampler),
            resources.combined_sampler_for(TerrainSourceResourceRole::DistantHorizonsOpaqueDepth)
        );
        assert_eq!(
            Some(targets.distant_depth_before_translucency_combined_sampler),
            resources.combined_sampler_for(
                TerrainSourceResourceRole::DistantHorizonsDepthBeforeTranslucency
            )
        );
        assert_eq!(91, resources.availability().shader_pack_generation());
        assert_eq!(17, resources.availability().world_generation());
        let opaque_generation = resources
            .availability()
            .resource_for(TerrainSourceResourceRole::DistantHorizonsOpaqueDepth)
            .expect("opaque depth must be available")
            .resource_generation;
        let snapshot_generation = resources
            .availability()
            .resource_for(TerrainSourceResourceRole::DistantHorizonsDepthBeforeTranslucency)
            .expect("pre-translucency depth must be available")
            .resource_generation;
        assert_ne!(0, opaque_generation);
        assert_eq!(opaque_generation, snapshot_generation);
        cache.discard_submission(&mut gal);
    }

    #[test]
    fn source_opaque_target_pairs_pack_primary_color_with_distinct_dh_depth() {
        let source = complete_bundled_pack_source_for_test();
        let mut gal = VulkanicGal::new_with_backend(
            Box::new(MockBackend::with_capabilities(presentation_capabilities(
                vulkan_capabilities(),
            ))),
            false,
        );
        let extent = Extent3d {
            width: 96,
            height: 64,
            depth: 1,
        };
        let mut runtime = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(1).unwrap();
        runtime.observe_distant_horizons_source_candidate_for_scope(
            &source,
            TerrainProgramScope::Overworld,
        );
        let colors = runtime
            .stage_source_color_targets(&mut gal, 17, extent)
            .unwrap()
            .expect("bundled source declares a semantic primary color target");
        let primary = colors.target("primary").unwrap();
        let identity = WorldLodSourceTargetIdentity {
            world_generation: 17,
            shader_pack_generation: source.generation(),
            extent,
        };
        let mut depth_cache = WorldLodSourceTargetCache::default();
        let depth = depth_cache.stage(&mut gal, identity).unwrap();
        let mut pass_resources = WorldLodSourcePassResources::default();
        let prepared = pass_resources
            .stage_target(&mut gal, &colors, depth)
            .unwrap();
        assert_eq!(Some(HandleKind::RenderTarget), prepared.target.kind());
        assert_eq!(Some(HandleKind::RenderPass), prepared.pass.kind());
        assert_eq!(primary.current_texture, prepared.primary_color_texture);
        assert_eq!(primary.current_attachment_view, prepared.primary_color_view);
        assert_eq!(depth.distant_depth_texture, prepared.distant_depth_texture);
        assert_eq!(depth.distant_depth_view, prepared.distant_depth_view);
        let cached = pass_resources
            .stage_target(&mut gal, &colors, depth)
            .unwrap();
        assert_eq!(prepared.target, cached.target);
        assert_eq!(prepared.pass, cached.pass);

        // The selected DH source contract has one primary color output. Both
        // reduced and atlas-resolved segments must share this target schema;
        // treating atlas ranges as ordinary terrain G-buffer writers would
        // make the source plan semantically inconsistent.
        assert_eq!(1, prepared.color_attachments.len());
        assert_eq!(
            TerrainPassOutput::LitTerrainColor,
            prepared.color_attachments[0].output
        );

        let operations = vec![
            CommandOp::Barrier(texture_barrier(
                prepared.primary_color_texture,
                TextureUsageState::Undefined,
                TextureUsageState::ColorAttachment,
            )),
            CommandOp::Barrier(texture_barrier(
                prepared.distant_depth_texture,
                TextureUsageState::Undefined,
                TextureUsageState::DepthStencilAttachment,
            )),
            CommandOp::BeginPass {
                pass: prepared.pass,
                target: prepared.target,
                colors: vec![PassAttachment {
                    view: prepared.primary_color_view,
                    load_op: AttachmentLoadOp::Clear,
                    store_op: AttachmentStoreOp::Store,
                    clear_color: Some(crate::render::vulkanic::commands::ClearColor {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 1.0,
                    }),
                }],
                depth_stencil: Some(PassAttachment {
                    view: prepared.distant_depth_view,
                    load_op: AttachmentLoadOp::Clear,
                    store_op: AttachmentStoreOp::Store,
                    clear_color: None,
                }),
            },
            CommandOp::EndPass,
        ];
        gal.submit(SubmissionBatch {
            label: "world-lod-source.opaque-target".to_string(),
            command_lists: vec![CommandList::from(CommandListDesc {
                label: "world-lod-source.opaque-target.commands".to_string(),
                operations,
            })],
        })
        .expect("the explicit DH source target/pass must validate without backend state");

        // The cache may replace a depth stream after a resource-generation
        // boundary without changing world, pack, or extent. The source target
        // must be rebuilt for the replacement; otherwise its attachment and
        // the `dhDepthTex` semantic resource diverge.
        let replacement_depth = WorldLodSourceTargets::create(&mut gal, identity, 2).unwrap();
        let replacement = pass_resources
            .stage_target(&mut gal, &colors, replacement_depth)
            .unwrap();
        assert_ne!(prepared.target, replacement.target);
        assert_ne!(prepared.pass, replacement.pass);
        assert_eq!(
            replacement_depth.distant_depth_texture,
            replacement.distant_depth_texture
        );
        assert_eq!(
            replacement_depth.distant_depth_view,
            replacement.distant_depth_view
        );

        pass_resources.destroy(&mut gal);
        replacement_depth.destroy(&mut gal);
        depth_cache.discard_submission(&mut gal);
        runtime.discard_source_color_targets_submission(&mut gal);
        runtime.destroy(&mut gal).unwrap();
    }

    #[test]
    fn source_opaque_and_depth_transaction_executes_after_pack_semantic_bootstrap() {
        let source = complete_bundled_pack_source_for_test();
        let program = lowered_source_program();
        let mut gal = VulkanicGal::new_with_backend(
            Box::new(MockBackend::with_capabilities(presentation_capabilities(
                vulkan_capabilities(),
            ))),
            false,
        );
        let extent = Extent3d {
            width: 96,
            height: 64,
            depth: 1,
        };
        let mut runtime = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(1).unwrap();
        runtime.observe_distant_horizons_source_candidate_for_scope(
            &source,
            TerrainProgramScope::Overworld,
        );
        let colors = runtime
            .stage_source_color_targets(&mut gal, 17, extent)
            .unwrap()
            .expect("bundled source retains named color targets");
        let mut depth_cache = WorldLodSourceTargetCache::default();
        let depth = depth_cache
            .stage(
                &mut gal,
                WorldLodSourceTargetIdentity {
                    world_generation: 17,
                    shader_pack_generation: source.generation(),
                    extent,
                },
            )
            .unwrap();
        let consumer_programs = runtime
            .prepared_lowered_distant_horizons_depth_consumers()
            .expect("the bundled source retains complete DH depth consumers");
        let mut binding_plans = vec![&program.opaque_resource_bindings];
        binding_plans.extend(
            consumer_programs
                .iter()
                .map(|consumer| &consumer.opaque_resource_bindings),
        );
        let (resources, resource_handles) = owned_resources_for_programs(
            &mut gal,
            &binding_plans,
            program.shader_pack_generation,
            17,
        );
        let depth_resources = depth.semantic_resources().unwrap();
        let consumer_plans = runtime
            .stage_distant_horizons_depth_consumer_execution_plans(
                &mut gal,
                &colors,
                &[resources.clone(), depth_resources],
                extent,
            )
            .expect("complete semantic resources must stage every declared DH depth consumer");
        assert!(
            !consumer_plans.is_empty(),
            "the bundled source must retain at least one explicit DH depth consumer"
        );

        let expanded = expand_world_lod_column_asset(&asset()).unwrap();
        let packed = pack_world_lod_gpu_column_asset(&expanded).unwrap();
        let assets = BTreeMap::from([(packed.column_key, packed)]);
        let instance = WorldLodColumnInstanceRequest {
            column_key: 7,
            column_generation: 3,
            layer: WORLD_LOD_LAYER_OPAQUE,
            segment_index: 0,
            order: 0,
        };
        let mut residency = WorldLodGpuResidency::default();
        let mut ops = Vec::new();
        let mut source_color_transaction = runtime
            .begin_source_color_transaction(
                &mut gal,
                &colors,
                ShaderPackColorBootstrapClearValues {
                    fog_color: crate::render::vulkanic::commands::ClearColor {
                        r: 0.1,
                        g: 0.2,
                        b: 0.3,
                        a: 1.0,
                    },
                },
                &mut ops,
            )
            .unwrap();
        residency
            .stage_visible_uploads(&mut gal, &assets, &[instance], &mut ops)
            .unwrap();
        let draw = residency
            .resolve_visible_draws(&assets, &[instance])
            .unwrap()[0];
        let frame = WorldLodRenderFrame {
            enabled: true,
            combined_matrix: [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
            model_view_matrix: [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
            projection_matrix: [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
            projection_inverse_matrix: [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
            clip_distance: 24.0,
            micro_offset: MICRO_OFFSET_SCALE,
            noise_intensity: 0.25,
            earth_radius: 6_371_000.0,
            noise_steps: 4,
            noise_dropoff: 96,
            ..WorldLodRenderFrame::default()
        };
        let mut source_resources = WorldLodSourcePassResources::default();
        let opaque_target = source_resources
            .stage_target(&mut gal, &colors, depth)
            .unwrap();
        let source_draw = source_resources
            .stage_draw(
                &mut gal,
                &program,
                opaque_target.primary_color_format,
                draw,
                WorldLodDrawUniform::from_semantics(&frame, draw).unwrap(),
                &distant_source_uniforms(),
                &mut ops,
            )
            .unwrap();
        let pack_set = source_resources
            .stage_pack_resources(
                &mut gal,
                &program,
                opaque_target.primary_color_format,
                &resources,
            )
            .unwrap();
        WorldLodSourcePassResources::append_draw(
            &opaque_target,
            source_draw,
            pack_set,
            crate::render::vulkanic::commands::ClearColor {
                r: 0.1,
                g: 0.2,
                b: 0.3,
                a: 1.0,
            },
            true,
            TextureUsageState::ShaderRead,
            TextureUsageState::Undefined,
            None,
            &mut ops,
        )
        .unwrap();
        depth.append_opaque_depth_snapshot(&mut ops);
        // A normal terrain writer may already have populated the shared
        // pack color target. A later DH draw must load that named image, not
        // infer another clear from DH's local target declaration.
        WorldLodSourcePassResources::append_draw(
            &opaque_target,
            source_draw,
            pack_set,
            crate::render::vulkanic::commands::ClearColor {
                r: 0.1,
                g: 0.2,
                b: 0.3,
                a: 1.0,
            },
            false,
            TextureUsageState::ShaderRead,
            TextureUsageState::ShaderRead,
            None,
            &mut ops,
        )
        .unwrap();
        depth.append_opaque_depth_snapshot(&mut ops);
        assert!(
            ops.iter()
                .any(|op| matches!(op, CommandOp::DrawIndexed { .. }))
        );
        assert!(ops.iter().any(|op| matches!(
            op,
            CommandOp::BeginPass { colors, .. }
                if colors.first().is_some_and(|attachment| {
                    attachment.load_op == AttachmentLoadOp::Clear
                        && attachment.clear_color
                            == Some(crate::render::vulkanic::commands::ClearColor {
                                r: 0.1,
                                g: 0.2,
                                b: 0.3,
                                a: 1.0,
                            })
                })
        )));
        let shared_primary_load_ops = ops
            .iter()
            .filter_map(|op| match op {
                CommandOp::BeginPass { target, colors, .. } if *target == opaque_target.target => {
                    colors.first().map(|attachment| attachment.load_op)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            vec![AttachmentLoadOp::Clear, AttachmentLoadOp::Load],
            shared_primary_load_ops,
            "the source-frame scheduler must clear the shared named target once, then preserve the prior producer output",
        );
        assert!(ops.iter().any(|op| matches!(op, CommandOp::CopyTexture(_))));
        source_color_transaction
            .record_external_outputs(&[TerrainSourceResourceRole::ShaderPackColor(
                "primary".to_string(),
            )])
            .unwrap();
        for (consumer, plan) in consumer_programs.iter().zip(consumer_plans.iter()) {
            source_color_transaction
                .append_fullscreen_consumer(
                    plan,
                    consumer,
                    FullscreenSourcePassFrame {
                        texture_transforms: consumer
                            .pack_texture_transforms(
                                &TerrainSourceTextureTransforms::canonical_minecraft_terrain(),
                            )
                            .unwrap(),
                        scalar_uniforms: consumer
                            .pack_scalar_uniforms(&distant_source_uniforms())
                            .unwrap(),
                        texture_transform_before: TextureUsageState::Undefined,
                        scalar_uniform_before: consumer
                            .execution_interface
                            .scalar_uniforms
                            .map(|_| TextureUsageState::Undefined),
                        clear_values: ShaderPackColorBootstrapClearValues {
                            fog_color: crate::render::vulkanic::commands::ClearColor {
                                r: 0.0,
                                g: 0.0,
                                b: 0.0,
                                a: 1.0,
                            },
                        },
                        color_attachment_before: Vec::new(),
                    },
                    &mut ops,
                )
                .unwrap();
        }
        source_color_transaction.finish(&mut ops).unwrap();
        assert!(
            ops.iter().any(|op| matches!(
                op,
                CommandOp::Draw {
                    vertices: 3,
                    instances: 1
                }
            )),
            "the complete source-derived DH transaction must execute its declared fullscreen consumers after bootstrap"
        );
        gal.submit(SubmissionBatch {
            label: "world-lod-source.complete-opaque".to_string(),
            command_lists: vec![CommandList::from(CommandListDesc {
                label: "world-lod-source.complete-opaque.commands".to_string(),
                operations: ops,
            })],
        })
        .expect("DH source opaque draw and depth snapshot must validate as one submission");
        source_color_transaction
            .confirm(&mut runtime, &mut gal)
            .expect(
                "the combined DH source submission must be the only history confirmation point",
            );
        let reused_colors = runtime
            .stage_source_color_targets(&mut gal, 17, extent)
            .unwrap()
            .expect("the confirmed source target generation must become reusable");
        assert_eq!(
            colors.target("primary"),
            reused_colors.target("primary"),
            "a confirmed source-color transaction must promote its first target generation instead of restaging it"
        );

        source_resources.destroy(&mut gal);
        residency.discard_submission(&mut gal);
        for plan in consumer_plans {
            plan.destroy(&mut gal);
        }
        for handle in resource_handles {
            let _ = gal.destroy(handle);
        }
        depth_cache.discard_submission(&mut gal);
        runtime.discard_source_color_targets_submission(&mut gal);
        runtime.destroy(&mut gal).unwrap();
    }

    #[test]
    fn source_target_snapshot_is_a_valid_depth_to_sampled_gal_transition() {
        let mut gal = VulkanicGal::new_with_backend(
            Box::new(MockBackend::with_capabilities(presentation_capabilities(
                vulkan_capabilities(),
            ))),
            false,
        );
        let identity = WorldLodSourceTargetIdentity {
            world_generation: 17,
            shader_pack_generation: 91,
            extent: Extent3d {
                width: 64,
                height: 32,
                depth: 1,
            },
        };
        let mut cache = WorldLodSourceTargetCache::default();
        let targets = cache.stage(&mut gal, identity).unwrap();
        let mut ops = vec![CommandOp::Barrier(texture_barrier(
            targets.distant_depth_texture,
            TextureUsageState::Undefined,
            TextureUsageState::DepthStencilAttachment,
        ))];
        targets.append_opaque_depth_snapshot(&mut ops);

        gal.submit(SubmissionBatch {
            label: "world-lod-source.snapshot".to_string(),
            command_lists: vec![CommandList::from(CommandListDesc {
                label: "world-lod-source.snapshot.commands".to_string(),
                operations: ops,
            })],
        })
        .expect("the opaque DH depth snapshot must validate as one GAL transaction");
        cache.confirm_submission(&mut gal);
        assert_eq!(Some(identity), cache.active_identity());
    }

    #[test]
    fn empty_source_depth_snapshot_clears_and_reuses_the_distinct_dh_stream() {
        let mut gal = VulkanicGal::new_with_backend(
            Box::new(MockBackend::with_capabilities(presentation_capabilities(
                vulkan_capabilities(),
            ))),
            false,
        );
        let identity = WorldLodSourceTargetIdentity {
            world_generation: 23,
            shader_pack_generation: 47,
            extent: Extent3d {
                width: 64,
                height: 32,
                depth: 1,
            },
        };
        let mut cache = WorldLodSourceTargetCache::default();
        let (targets, initial_usage, created) = cache
            .stage_for_empty_depth_snapshot(&mut gal, identity)
            .unwrap();
        assert_eq!(TextureUsageState::Undefined, initial_usage);
        assert!(created);
        let mut ops = Vec::new();
        targets
            .append_empty_opaque_depth_snapshot(initial_usage, &mut ops)
            .unwrap();
        assert!(ops.iter().any(|op| matches!(
            op,
            CommandOp::BeginPass { colors, .. } if colors.is_empty()
        )));
        assert!(
            !ops.iter()
                .any(|op| matches!(op, CommandOp::Draw { .. } | CommandOp::DrawIndexed { .. }))
        );
        gal.submit(SubmissionBatch {
            label: "world-lod-source.empty-depth-initial".to_string(),
            command_lists: vec![CommandList::from(CommandListDesc {
                label: "world-lod-source.empty-depth-initial.commands".to_string(),
                operations: ops,
            })],
        })
        .expect("an empty DH source frame must establish its depth streams without geometry");
        cache.confirm_submission(&mut gal);
        let active_resources = cache
            .active_semantic_resources(identity)
            .unwrap()
            .expect("the confirmed empty snapshot must remain available to a later source frame");
        assert!(
            active_resources
                .availability()
                .resource_for(TerrainSourceResourceRole::DistantHorizonsOpaqueDepth)
                .is_some()
        );
        assert!(
            active_resources
                .availability()
                .resource_for(TerrainSourceResourceRole::DistantHorizonsDepthBeforeTranslucency)
                .is_some()
        );

        let (reused, reuse_usage, recreated) = cache
            .stage_for_empty_depth_snapshot(&mut gal, identity)
            .unwrap();
        assert_eq!(TextureUsageState::ShaderRead, reuse_usage);
        assert!(!recreated);
        let mut reuse_ops = Vec::new();
        reused
            .append_empty_opaque_depth_snapshot(reuse_usage, &mut reuse_ops)
            .unwrap();
        gal.submit(SubmissionBatch {
            label: "world-lod-source.empty-depth-reuse".to_string(),
            command_lists: vec![CommandList::from(CommandListDesc {
                label: "world-lod-source.empty-depth-reuse.commands".to_string(),
                operations: reuse_ops,
            })],
        })
        .expect("a confirmed DH source depth stream must be reusable from shader-read state");
        let resources = reused.semantic_resources().unwrap();
        assert!(resources.availability().resources().any(|resource| {
            resource.role == TerrainSourceResourceRole::DistantHorizonsOpaqueDepth
        }));
        assert!(resources.availability().resources().any(|resource| {
            resource.role == TerrainSourceResourceRole::DistantHorizonsDepthBeforeTranslucency
        }));
    }

    #[test]
    fn source_target_identity_rejects_zero_generations_and_non_2d_extent() {
        let invalid_generation = WorldLodSourceTargetIdentity {
            world_generation: 0,
            shader_pack_generation: 1,
            extent: Extent3d {
                width: 1,
                height: 1,
                depth: 1,
            },
        };
        assert!(invalid_generation.validate().is_err());
        let invalid_extent = WorldLodSourceTargetIdentity {
            world_generation: 1,
            shader_pack_generation: 1,
            extent: Extent3d {
                width: 1,
                height: 1,
                depth: 2,
            },
        };
        assert!(invalid_extent.validate().is_err());
    }
}
