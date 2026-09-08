use std::collections::BTreeSet;

use crate::render::vulkanic::error::{GalError, GalResult};
use crate::render::vulkanic::handles::{Handle, HandleKind};
use crate::render::vulkanic::resources::{
    AccessFlags, BackendApi, BlendMode, PipelineStageFlags, ResourceBinding, ResourceBindingDesc,
    ResourceBindingKind, ResourceLayoutDesc, ResourceSetDesc, ShaderCodeFormat, ShaderModuleDesc,
    ShaderStage,
};

use super::cloud_contract::{CloudBlend, CloudPassContract};
use super::distant_horizons_contract::{DistantHorizonsPassContract, DistantHorizonsPassKind};
use super::entity_contract::{EntityPassContract, EntitySourceDrawSemantics, EntitySourceOutput};
use super::hand_contract::{HandPassContract, HandSourceOutput};
use super::lowering::{
    DistantHorizonsFragmentOutput, FullscreenSourceFragmentOutput, FullscreenSourceRasterPrimitive,
    LoweredCloudSourcePair, LoweredDistantHorizonsSourcePair, LoweredEntitySourcePair,
    LoweredFullscreenSourcePair, LoweredHandSourcePair, LoweredShadowSourcePair,
    LoweredTerrainSourcePair, LoweredTexturedMaterialSourcePair,
    LoweredTranslucentTerrainSourcePair, LoweredWeatherSourcePair,
    TerrainSourceOpaqueResourceBindingPlan, TerrainSourceOpaqueResourceKind,
    TerrainSourceUniformField,
};
use super::material_contract::{
    pack_textured_material_source_primitives, TexturedMaterialPassContract,
    TexturedMaterialSourcePrimitive, TEXTURED_MATERIAL_SOURCE_VERTEX_BYTES,
};
use super::source_uniforms::{TerrainSourceUniformFrame, TerrainSourceUniformRequirements};
use super::terrain_contract::{
    TerrainMaterialClass, TerrainPassContract, TerrainPassOutput, TerrainPassRequiredResource,
    TerrainTranslucentBlend, TerrainTranslucentRasterState,
};
use super::terrain_source_resources::{
    TerrainSourceOwnedResourceSet, TerrainSourceResourceAvailabilitySet, TerrainSourceResourceRole,
};
use super::weather_contract::{WeatherBlend, WeatherPassContract};

/// Fixed std430 source-terrain vertex record declared by the Rust source
/// lowerer. Frontend staging may use this semantic ABI, but it is not a Java
/// vertex layout or a native backend format.
pub(crate) const TERRAIN_SOURCE_VERTEX_BYTES: usize = 8 * 4 * std::mem::size_of::<f32>();
/// One source terrain instance carries a copied semantic model transform and
/// color modulation. This is deliberately independent of a Java vertex
/// layout or backend state, but preserves the per-instance appearance already
/// present in the shared world-mesh semantic request.
pub(crate) const TERRAIN_SOURCE_INSTANCE_BYTES: usize = 20 * std::mem::size_of::<f32>();
/// Fixed std430 record copied from the DH CPU mesh stream and expanded by the
/// Rust world frontend. This is intentionally distinct from the near-terrain
/// source record: DH has pre-resolved vertex color/light/material data rather
/// than atlas-backed Minecraft terrain vertices.
pub(crate) const DISTANT_HORIZONS_SOURCE_VERTEX_BYTES: usize = 32;
/// Private Rust source-stream record for a DH range with complete atlas
/// provenance. It extends the regular DH semantic record with one exact
/// atlas rectangle and repeated tile coordinates; it is not a Java/DH GL
/// layout or backend-native vertex format.
pub(crate) const DISTANT_HORIZONS_EXACT_ATLAS_SOURCE_VERTEX_BYTES: usize = 56;
/// The per-column std140 semantic frame block consumed by the lowered DH
/// source preamble. It carries a Rust-owned column origin and LOD controls;
/// source-declared `dh*` matrices live in the separately typed scalar block.
pub(crate) const DISTANT_HORIZONS_SOURCE_COLUMN_FRAME_BYTES: usize = 128;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ProgramIdentity(String);

impl ProgramIdentity {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShaderStageKind {
    Vertex,
    Fragment,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShaderStageSource {
    pub stage: ShaderStageKind,
    pub label: String,
    pub source: String,
    pub entry_point: String,
}

impl ShaderStageSource {
    /// Converts owned shader-pack text into an explicit GAL shader-module
    /// description. This selects only the portable coordinate convention
    /// required by the target API; backend compilation and native objects
    /// remain private to their respective backends.
    pub fn shader_module_descriptor(&self, api: BackendApi) -> ShaderModuleDesc {
        let stage = match self.stage {
            ShaderStageKind::Vertex => ShaderStage::Vertex,
            ShaderStageKind::Fragment => ShaderStage::Fragment,
        };
        ShaderModuleDesc {
            label: self.label.clone(),
            stage,
            code_format: ShaderCodeFormat::Glsl,
            code: shader_stage_code_for_backend(api, &self.source),
            entry_point: self.entry_point.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerrainMaterialProgram {
    pub identity: ProgramIdentity,
    pub vertex: ShaderStageSource,
    pub fragment: ShaderStageSource,
    /// Backend-neutral semantic resources required in addition to the mesh
    /// material set. Pipeline construction turns these into ordinary GAL
    /// resource layouts; program descriptions never contain backend handles.
    pub required_resources: Vec<TerrainProgramResource>,
}

/// Actual source text after Rust's bounded source lowering, paired with the
/// pack-declared semantic sampler roles it needs. It may represent a normal
/// terrain or shadow-material source pair; its identity and named outputs keep
/// those uses distinct. This is intentionally not a `TerrainMaterialProgram`:
/// no existing renderer path can compile or draw it until a later source-mesh
/// and semantic resource-set slice is complete.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoweredTerrainSourceProgram {
    pub identity: ProgramIdentity,
    /// Semantic material pass admitted by this program. Shadow-only programs
    /// intentionally carry no material kind.
    pub material_kind: Option<TerrainMaterialProgramKind>,
    /// Exact shader-pack source generation which produced this program.
    pub shader_pack_generation: u64,
    pub vertex: ShaderStageSource,
    pub fragment: ShaderStageSource,
    /// Fixed semantic bindings inserted by the source lowerer. They describe
    /// Rust-owned data, never a Java vertex layout or native backend object.
    pub execution_interface: TerrainSourceExecutionInterface,
    /// Typed scalar input requirements derived from the selected source.
    /// Unresolved entries intentionally keep this preparation artifact out of
    /// execution; this is never an untyped uniform payload.
    pub scalar_uniform_requirements: TerrainSourceUniformRequirements,
    pub opaque_resource_bindings: TerrainSourceOpaqueResourceBindingPlan,
    pub required_resources: Vec<TerrainProgramResource>,
    /// Exact named terrain outputs retained from the source contract. Shadow
    /// programs intentionally leave this absent because their outputs belong
    /// to a separate shadow pass schema.
    terrain_outputs: Option<Vec<TerrainPassOutput>>,
    /// Source-declared shader-pack color slot for each named terrain output.
    /// These slots are source metadata used to resolve Rust-owned semantic
    /// targets; they are never GAL attachment indices or backend bindings.
    terrain_output_color_slots: Option<Vec<(TerrainPassOutput, u32)>>,
    /// Explicit source-derived raster semantics for the separate translucent
    /// stage. Normal terrain and shadow programs deliberately leave this
    /// empty instead of borrowing a renderer default.
    translucent_raster_state: Option<TerrainTranslucentRasterState>,
}

/// Prepared selected-source entity program. This owns source text and typed
/// semantic requirements only; it cannot compile a backend pipeline, bind a
/// native texture, or select a producer route. Keeping it distinct from
/// terrain prevents local entity textures and `entity.properties` IDs from
/// being mistaken for terrain-atlas semantics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoweredEntitySourceProgram {
    pub identity: ProgramIdentity,
    pub shader_pack_generation: u64,
    /// Exact source generation for the Rust-owned entity ID map.
    pub entity_id_generation: u64,
    /// The source-pack entity map remains Rust-owned with the prepared
    /// program. Frame preparation resolves copied canonical identities here;
    /// Java never transports a shader-pack numeric entity ID.
    entity_contract: EntityPassContract,
    pub vertex: ShaderStageSource,
    pub fragment: ShaderStageSource,
    pub execution_interface: TerrainSourceExecutionInterface,
    pub scalar_uniform_requirements: TerrainSourceUniformRequirements,
    pub opaque_resource_bindings: TerrainSourceOpaqueResourceBindingPlan,
    named_output_color_slots: Vec<(TerrainPassOutput, u32)>,
}

/// Prepared selected-source first-person hand/item program. It has the same
/// owned indexed-stream ABI as an entity program, but retains a distinct hand
/// contract so a later writer cannot accidentally use world-camera transforms
/// or the entity pass depth domain.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoweredHandSourceProgram {
    pub identity: ProgramIdentity,
    pub shader_pack_generation: u64,
    hand_contract: HandPassContract,
    pub vertex: ShaderStageSource,
    pub fragment: ShaderStageSource,
    pub execution_interface: TerrainSourceExecutionInterface,
    pub scalar_uniform_requirements: TerrainSourceUniformRequirements,
    pub opaque_resource_bindings: TerrainSourceOpaqueResourceBindingPlan,
    named_output_color_slots: Vec<(TerrainPassOutput, u32)>,
}

/// Shared typed contract for Rust-owned indexed streams with a local material
/// texture. Entity and hand passes intentionally retain distinct source
/// contracts and pass ownership, but their frontend resource preparation uses
/// the same fixed ABI and semantic resource-set rules.
///
/// This is not a backend abstraction: it exposes only source-derived shader
/// metadata and typed GAL descriptions. The frontend still owns cache keys,
/// uploads, and pass scheduling.
pub trait LocalTexturedSourceProgram {
    fn identity(&self) -> &ProgramIdentity;
    fn shader_pack_generation(&self) -> u64;
    fn execution_interface(&self) -> &TerrainSourceExecutionInterface;
    fn shader_module_descriptors_with_alpha_cutoff(
        &self,
        api: BackendApi,
        alpha_cutoff: Option<f32>,
    ) -> [ShaderModuleDesc; 2];
    fn execution_resource_layouts(&self) -> GalResult<TerrainSourceExecutionLayouts>;
    fn require_semantic_resources(
        &self,
        availability: &TerrainSourceResourceAvailabilitySet,
    ) -> GalResult<()>;
    fn pack_resource_set_desc(
        &self,
        label: String,
        layout: Handle,
        resources: &TerrainSourceOwnedResourceSet,
    ) -> GalResult<ResourceSetDesc>;
}

/// Source-derived program preparation for generic textured world material.
/// It retains the selected source's named outputs and semantic resource plan,
/// but has no executable vertex-stream/pipeline allocation yet. That keeps
/// the legacy post-final overlay from being mistaken for shader-pack
/// participation while a dedicated Rust-owned material writer is staged.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoweredTexturedMaterialSourceProgram {
    pub identity: ProgramIdentity,
    pub shader_pack_generation: u64,
    pub vertex: ShaderStageSource,
    pub fragment: ShaderStageSource,
    /// Fixed source-material stream contract. It has no terrain-only lanes
    /// and no per-instance transform table because its vertices are explicit
    /// camera-relative semantic positions.
    pub execution_interface: TexturedMaterialSourceExecutionInterface,
    pub scalar_uniform_requirements: TerrainSourceUniformRequirements,
    pub opaque_resource_bindings: TerrainSourceOpaqueResourceBindingPlan,
    named_output_color_slots: Vec<(TerrainPassOutput, u32)>,
}

/// Prepared selected-source weather program. It owns no target, pipeline, or
/// route; the runtime must later provide a dedicated named weather pass.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoweredWeatherSourceProgram {
    pub identity: ProgramIdentity,
    pub shader_pack_generation: u64,
    pub vertex: ShaderStageSource,
    pub fragment: ShaderStageSource,
    pub execution_interface: TexturedMaterialSourceExecutionInterface,
    pub scalar_uniform_requirements: TerrainSourceUniformRequirements,
    pub opaque_resource_bindings: TerrainSourceOpaqueResourceBindingPlan,
    pub lit_color_output_slot: u8,
    pub alpha_discard_threshold_bits: u32,
    pub blend: WeatherBlend,
}

/// Prepared selected-source vanilla-cloud program. It owns no target,
/// pipeline, or route; a future cloud writer must provide an explicit named
/// pass and fully semantic resource availability.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoweredCloudSourceProgram {
    pub identity: ProgramIdentity,
    pub shader_pack_generation: u64,
    pub vertex: ShaderStageSource,
    pub fragment: ShaderStageSource,
    pub execution_interface: TexturedMaterialSourceExecutionInterface,
    pub scalar_uniform_requirements: TerrainSourceUniformRequirements,
    pub opaque_resource_bindings: TerrainSourceOpaqueResourceBindingPlan,
    pub blend: CloudBlend,
    named_output_color_slots: Vec<(TerrainPassOutput, u32)>,
}

/// Backend-neutral layouts required to execute a lowered source-terrain
/// program. These are descriptions only: callers still need to allocate
/// Rust-owned resources, create GAL layouts/sets, and prove source-plan
/// readiness before any render route can use them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerrainSourceExecutionLayouts {
    /// Fixed Rust-owned streams and scalar blocks inserted by the source
    /// lowerer. This is always descriptor set zero.
    pub source_data: ResourceLayoutDesc,
    /// Selected-pack sampler resources, mapped from declared semantic roles.
    /// This is always descriptor set one.
    pub pack_resources: ResourceLayoutDesc,
}

/// Backend-neutral layouts for the compact source-material stream. Set zero
/// contains only Rust-owned vertex/transforms/scalar data; set one contains
/// the selected pack's pass-local semantic resources.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TexturedMaterialSourceExecutionLayouts {
    pub source_data: ResourceLayoutDesc,
    pub pack_resources: ResourceLayoutDesc,
}

/// Fixed stream ABI for a source-derived generic textured-material pass.
/// This explicitly models the data `gbuffers_textured` may consume and keeps
/// it distinct from terrain's material/entity/tangent stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TexturedMaterialSourceExecutionInterface {
    pub vertex_stream: TerrainSourceFixedBinding,
    pub vertex_stride: u32,
    pub vertex_fields: [TerrainSourceVertexField; 4],
    pub legacy_transforms: TerrainSourceFixedBinding,
    pub scalar_uniforms: Option<TerrainSourceFixedBinding>,
    pub legacy_transform_bytes: u32,
    pub scalar_uniform_bytes: u32,
    pub scalar_uniform_fields: Vec<TerrainSourceUniformField>,
}

/// Source-derived executable preparation for the distinct Distant Horizons
/// terrain stage. The program is deliberately separate from
/// [`LoweredTerrainSourceProgram`]: DH uses a copied 32-byte column stream,
/// one column-frame block per draw, and `dh*` transform semantics. Keeping
/// the ABI distinct prevents the near-terrain selected-source route from
/// interpreting a DH asset as an atlas-backed mesh.
///
/// This remains a preparation artifact. It owns no native object, creates no
/// target, and cannot select a gameplay route.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoweredDistantHorizonsSourceProgram {
    pub identity: ProgramIdentity,
    pub shader_pack_generation: u64,
    /// The semantic DH material phase selected from source. It prevents a
    /// later pass owner from applying opaque depth/blend behavior to a
    /// separately lowered `dh_water` program.
    pub pass_kind: DistantHorizonsPassKind,
    /// Source-declared blend semantics for the translucent phase only. This
    /// is intentionally not an API pipeline flag.
    pub translucent_blend: Option<TerrainTranslucentBlend>,
    pub vertex: ShaderStageSource,
    pub fragment: ShaderStageSource,
    pub execution_interface: DistantHorizonsSourceExecutionInterface,
    pub scalar_uniform_requirements: TerrainSourceUniformRequirements,
    pub opaque_resource_bindings: TerrainSourceOpaqueResourceBindingPlan,
    pub required_resources: Vec<TerrainProgramResource>,
}

/// Source-derived DH program variant for a range whose copied semantic
/// provenance resolves one exact Minecraft atlas sprite per quad. It keeps
/// the selected pack's transform, material-category, lighting, and fragment
/// logic intact; the adapter supplies only the source `gl_Color` input from
/// Rust-owned atlas data.
///
/// This is deliberately distinct from the reduced-color stream ABI. It owns
/// no target or native resource and remains a shader-pack preparation
/// artifact, while the world frontend owns its private vertex stream and
/// atlas resource lifetime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoweredDistantHorizonsExactAtlasSourceProgram {
    pub source: LoweredDistantHorizonsSourceProgram,
}

impl LoweredDistantHorizonsExactAtlasSourceProgram {
    pub fn shader_module_descriptors(&self, api: BackendApi) -> [ShaderModuleDesc; 2] {
        self.source.shader_module_descriptors(api)
    }
}

/// Derives the exact-atlas variant of an already admitted DH source program.
/// The selected source remains authoritative for all shader-pack logic. This
/// adapter changes only the private Rust vertex/material input convention so
/// a provenance-resolved atlas tile can become `gl_Color` before the source
/// vertex and fragment stages execute.
pub fn prepare_lowered_distant_horizons_exact_atlas_source_program(
    program: &LoweredDistantHorizonsSourceProgram,
) -> GalResult<LoweredDistantHorizonsExactAtlasSourceProgram> {
    if program.pass_kind != DistantHorizonsPassKind::Opaque || program.translucent_blend.is_some() {
        return Err(GalError::invalid_argument(
            "exact-atlas Distant Horizons source adapter requires an opaque source program",
        ));
    }
    if program.vertex.source.contains("vulkanic_source_dh_atlas_")
        || program
            .fragment
            .source
            .contains("vulkanic_source_dh_atlas_")
    {
        return Err(GalError::invalid_argument(
            "Distant Horizons source program already contains an exact-atlas adapter interface",
        ));
    }
    if !program
        .vertex
        .source
        .contains("vulkanic_source_vertex_color")
        || !program
            .vertex
            .source
            .contains("vulkanic_source_dh_material_id")
    {
        return Err(GalError::unsupported_feature(
            "Distant Horizons source program does not expose the required semantic color/material interface",
        ));
    }

    let varying_locations = exact_atlas_distant_horizons_varying_locations(
        &program.vertex.source,
        &program.fragment.source,
    )?;
    let vertex = inject_exact_atlas_distant_horizons_vertex_adapter(
        &program.vertex.source,
        varying_locations,
    )?;
    let fragment = inject_exact_atlas_distant_horizons_fragment_adapter(
        &program.fragment.source,
        varying_locations,
    )?;
    let mut source = program.clone();
    source.identity = ProgramIdentity::new(format!("{}:exact-atlas", program.identity.as_str()));
    source.vertex = ShaderStageSource {
        stage: ShaderStageKind::Vertex,
        label: format!("{}:exact-atlas-adapter", program.vertex.label),
        source: vertex,
        entry_point: "main".to_string(),
    };
    source.fragment = ShaderStageSource {
        stage: ShaderStageKind::Fragment,
        label: format!("{}:exact-atlas-adapter", program.fragment.label),
        source: fragment,
        entry_point: "main".to_string(),
    };
    source.execution_interface.vertex_stride =
        DISTANT_HORIZONS_EXACT_ATLAS_SOURCE_VERTEX_BYTES as u32;
    source.execution_interface.material_identity_contract =
        DistantHorizonsMaterialIdentityContract::AtlasBacked;
    source.execution_interface.validate()?;
    Ok(LoweredDistantHorizonsExactAtlasSourceProgram { source })
}

#[derive(Clone, Copy, Debug)]
struct ExactAtlasDistantHorizonsVaryingLocations {
    tile_uv: u32,
    atlas_rect: u32,
    tint_and_material: u32,
}

/// Allocates three locations outside the selected source interface. The
/// adapter must not reserve a high arbitrary location: Vulkan counts declared
/// interface components against the device limit even when lower locations are
/// unused.
fn exact_atlas_distant_horizons_varying_locations(
    vertex: &str,
    fragment: &str,
) -> GalResult<ExactAtlasDistantHorizonsVaryingLocations> {
    let mut occupied = std::collections::BTreeSet::new();
    for source in [vertex, fragment] {
        for line in source.lines() {
            let Some(location_start) = line.find("layout(location = ") else {
                continue;
            };
            let value = &line[location_start + "layout(location = ".len()..];
            let Some(location_end) = value.find(')') else {
                return Err(GalError::invalid_argument(
                    "Distant Horizons source has an unterminated explicit varying location",
                ));
            };
            let location = value[..location_end].trim().parse::<u32>().map_err(|_| {
                GalError::invalid_argument(
                    "Distant Horizons source has a non-numeric explicit varying location",
                )
            })?;
            occupied.insert(location);
        }
    }
    for first in 0..=u32::MAX - 2 {
        if !occupied.contains(&first)
            && !occupied.contains(&(first + 1))
            && !occupied.contains(&(first + 2))
        {
            return Ok(ExactAtlasDistantHorizonsVaryingLocations {
                tile_uv: first,
                atlas_rect: first + 1,
                tint_and_material: first + 2,
            });
        }
    }
    Err(GalError::unsupported_feature(
        "Distant Horizons source has no three-location interval for the exact-atlas adapter",
    ))
}

fn inject_exact_atlas_distant_horizons_vertex_adapter(
    source: &str,
    locations: ExactAtlasDistantHorizonsVaryingLocations,
) -> GalResult<String> {
    let start = source.find("struct VulkanicDistantHorizonsVertex {").ok_or_else(|| {
        GalError::unsupported_feature(
            "Distant Horizons source vertex adapter could not find the semantic vertex-color definition",
        )
    })?;
    let end_marker = "#define vulkanic_source_ftransform() (dhProjection * vulkanic_source_model_view * vulkanic_source_position)";
    let end = source[start..]
        .find(end_marker)
        .map(|offset| start + offset + end_marker.len())
        .ok_or_else(|| {
            GalError::unsupported_feature(
                "Distant Horizons source vertex adapter could not find the semantic transform definition",
            )
        })?;
    let preamble = r#"struct VulkanicDistantHorizonsExactAtlasVertex {
    float local_x;
    float local_y;
    float local_z;
    float micro_x;
    float micro_y;
    float micro_z;
    float tile_u;
    float tile_v;
    float atlas_u0;
    float atlas_v0;
    float atlas_u1;
    float atlas_v1;
    uint color_rgba;
    uint light_normal_tint_material;
};
layout(set = 0, binding = 0, std430) readonly buffer VulkanicDistantHorizonsExactAtlasVertices {
    VulkanicDistantHorizonsExactAtlasVertex vulkanic_source_dh_vertices[];
};
layout(set = 0, binding = 1, std140) uniform VulkanicDistantHorizonsColumnFrame {
    mat4 vulkanic_source_dh_unused_combined_matrix;
    vec4 vulkanic_source_dh_column_origin_and_world_y;
    vec4 vulkanic_source_dh_model_offset_and_reserved;
    vec4 vulkanic_source_dh_clip_micro_noise_earth;
    uvec4 vulkanic_source_dh_flags_and_noise;
};
#define VULKANIC_SOURCE_DH_ATLAS_TILE_LOCATION __VULKANIC_SOURCE_DH_ATLAS_TILE_LOCATION__
#define VULKANIC_SOURCE_DH_ATLAS_RECT_LOCATION __VULKANIC_SOURCE_DH_ATLAS_RECT_LOCATION__
#define VULKANIC_SOURCE_DH_ATLAS_TINT_LOCATION __VULKANIC_SOURCE_DH_ATLAS_TINT_LOCATION__
layout(location = VULKANIC_SOURCE_DH_ATLAS_TILE_LOCATION) out vec2 vulkanic_source_dh_atlas_tile_uv;
layout(location = VULKANIC_SOURCE_DH_ATLAS_RECT_LOCATION) flat out vec4 vulkanic_source_dh_atlas_rect;
layout(location = VULKANIC_SOURCE_DH_ATLAS_TINT_LOCATION) flat out uint vulkanic_source_dh_atlas_tint_and_material;
#define vulkanic_source_dh_vertex vulkanic_source_dh_vertices[gl_VertexIndex]
vec3 vulkanic_source_dh_normal(uint normal) {
    if (normal == 0u) return vec3(0.0, -1.0, 0.0);
    if (normal == 1u) return vec3(0.0, 1.0, 0.0);
    if (normal == 2u) return vec3(0.0, 0.0, -1.0);
    if (normal == 3u) return vec3(0.0, 0.0, 1.0);
    if (normal == 4u) return vec3(-1.0, 0.0, 0.0);
    return vec3(1.0, 0.0, 0.0);
}
vec4 vulkanic_source_dh_position() {
    return vec4(
        vec3(vulkanic_source_dh_vertex.local_x, vulkanic_source_dh_vertex.local_y, vulkanic_source_dh_vertex.local_z)
            // Match Iris's DHTerrainTransformer: compact DH micro offsets
            // perturb the horizontal quad edges only.
            + vec3(vulkanic_source_dh_vertex.micro_x, 0.0, vulkanic_source_dh_vertex.micro_z)
            // The column origin already carries the dimension's minimum Y.
            // Keep worldYOffset as source-pack context, never as a second
            // geometry translation.
            + vulkanic_source_dh_model_offset_and_reserved.xyz,
        1.0
    );
}
vec4 vulkanic_source_dh_vertex_color() {
    return vec4(
        float(vulkanic_source_dh_vertex.color_rgba & 0xffu),
        float((vulkanic_source_dh_vertex.color_rgba >> 8u) & 0xffu),
        float((vulkanic_source_dh_vertex.color_rgba >> 16u) & 0xffu),
        float((vulkanic_source_dh_vertex.color_rgba >> 24u) & 0xffu)
    ) / 255.0;
}
vec2 vulkanic_source_dh_packed_lightmap_coordinates() {
    return (vec2(
        // Iris's DHTerrainTransformer expands the packed byte as
        // `(blockLight, skyLight)`: upper nibble first, lower nibble second.
        // The copied DH semantic record retains the original order as
        // `(skyLight, blockLight)`, so preserve the source lightmap contract
        // here instead of treating its byte layout as a texture coordinate.
        float((vulkanic_source_dh_vertex.light_normal_tint_material >> 8u) & 0xffu),
        float(vulkanic_source_dh_vertex.light_normal_tint_material & 0xffu)
    ) + vec2(0.5)) / 16.0;
}
#define vulkanic_source_texture_matrix (mat4[2](mat4(1.0), mat4(1.0)))
#define vulkanic_source_lightmap_uv vec4(vulkanic_source_dh_packed_lightmap_coordinates(), 0.0, 1.0)
#define vulkanic_source_position vulkanic_source_dh_position()
#define vulkanic_source_vertex_color vulkanic_source_dh_vertex_color()
#define vulkanic_source_normal vulkanic_source_dh_normal((vulkanic_source_dh_vertex.light_normal_tint_material >> 16u) & 0xffu)
#define vulkanic_source_dh_material_id int((vulkanic_source_dh_vertex.light_normal_tint_material >> 25u) & 0x0fu)
#define vulkanic_source_model_view dhModelView
#define vulkanic_source_normal_matrix transpose(inverse(mat3(vulkanic_source_model_view)))
#define vulkanic_source_ftransform() (dhProjection * vulkanic_source_model_view * vulkanic_source_position)"#;
    let preamble = preamble
        .replace(
            "__VULKANIC_SOURCE_DH_ATLAS_TILE_LOCATION__",
            &locations.tile_uv.to_string(),
        )
        .replace(
            "__VULKANIC_SOURCE_DH_ATLAS_RECT_LOCATION__",
            &locations.atlas_rect.to_string(),
        )
        .replace(
            "__VULKANIC_SOURCE_DH_ATLAS_TINT_LOCATION__",
            &locations.tint_and_material.to_string(),
        );
    let mut lowered = String::with_capacity(source.len() + preamble.len());
    lowered.push_str(&source[..start]);
    lowered.push_str(&preamble);
    lowered.push_str(&source[end..]);
    let closing = main_function_closing_brace(&lowered).ok_or_else(|| {
        GalError::invalid_argument(
            "Distant Horizons source vertex adapter requires a brace-balanced void main body",
        )
    })?;
    lowered.insert_str(
        closing,
        r#"
    vulkanic_source_dh_atlas_tile_uv = vec2(vulkanic_source_dh_vertex.tile_u, vulkanic_source_dh_vertex.tile_v);
    vulkanic_source_dh_atlas_rect = vec4(vulkanic_source_dh_vertex.atlas_u0, vulkanic_source_dh_vertex.atlas_v0, vulkanic_source_dh_vertex.atlas_u1, vulkanic_source_dh_vertex.atlas_v1);
    vulkanic_source_dh_atlas_tint_and_material = vulkanic_source_dh_vertex.light_normal_tint_material >> 24u;
"#,
    );
    Ok(lowered)
}

fn inject_exact_atlas_distant_horizons_fragment_adapter(
    source: &str,
    locations: ExactAtlasDistantHorizonsVaryingLocations,
) -> GalResult<String> {
    if !source.contains("void main()") {
        return Err(GalError::invalid_argument(
            "Distant Horizons source fragment adapter requires a void main function",
        ));
    }
    let declarations = r#"
layout(set = 2, binding = 0) uniform texture2D vulkanic_source_dh_atlas_texture;
layout(set = 2, binding = 1) uniform sampler vulkanic_source_dh_atlas_sampler;
#define VULKANIC_SOURCE_DH_ATLAS_TILE_LOCATION __VULKANIC_SOURCE_DH_ATLAS_TILE_LOCATION__
#define VULKANIC_SOURCE_DH_ATLAS_RECT_LOCATION __VULKANIC_SOURCE_DH_ATLAS_RECT_LOCATION__
#define VULKANIC_SOURCE_DH_ATLAS_TINT_LOCATION __VULKANIC_SOURCE_DH_ATLAS_TINT_LOCATION__
layout(location = VULKANIC_SOURCE_DH_ATLAS_TILE_LOCATION) in vec2 vulkanic_source_dh_atlas_tile_uv;
layout(location = VULKANIC_SOURCE_DH_ATLAS_RECT_LOCATION) flat in vec4 vulkanic_source_dh_atlas_rect;
layout(location = VULKANIC_SOURCE_DH_ATLAS_TINT_LOCATION) flat in uint vulkanic_source_dh_atlas_tint_and_material;
vec4 vulkanic_source_dh_atlas_color() {
    vec2 extent = vec2(textureSize(sampler2D(vulkanic_source_dh_atlas_texture, vulkanic_source_dh_atlas_sampler), 0));
    vec2 texel = vec2(0.5) / extent;
    vec2 sprite_min = vulkanic_source_dh_atlas_rect.xy + texel;
    vec2 sprite_max = vulkanic_source_dh_atlas_rect.zw - texel;
    vec2 atlas_uv = mix(sprite_min, max(sprite_min, sprite_max), fract(vulkanic_source_dh_atlas_tile_uv));
    vec4 color = texture(sampler2D(vulkanic_source_dh_atlas_texture, vulkanic_source_dh_atlas_sampler), atlas_uv);
    // DH's reduced vertex color is still a semantic material input after an
    // exact sprite replaces the reduced-color-only route. It carries the
    // source material's face color for ordinary blocks as well as biome tint
    // for tinted blocks. Applying it only to the tint bit loses that source
    // factor for materials such as redstone ore and terracotta.
    color.rgb *= glColor.rgb;
    color.a *= glColor.a;
    return color;
}
"#;
    // The lowered source declares pack varyings, including `glColor`, between
    // its version line and main. Insert our extra interface after that source
    // interface so the adapter can consume the selected program's color
    // semantic without assuming a declaration order.
    let declarations = declarations
        .replace(
            "__VULKANIC_SOURCE_DH_ATLAS_TILE_LOCATION__",
            &locations.tile_uv.to_string(),
        )
        .replace(
            "__VULKANIC_SOURCE_DH_ATLAS_RECT_LOCATION__",
            &locations.atlas_rect.to_string(),
        )
        .replace(
            "__VULKANIC_SOURCE_DH_ATLAS_TINT_LOCATION__",
            &locations.tint_and_material.to_string(),
        );
    let mut lowered = insert_before_main(source, &declarations)?;
    let anchor = "vec4 color = vec4(glColor.rgb, 1.0);";
    if !lowered.contains(anchor) {
        return Err(GalError::unsupported_feature(
            "Distant Horizons source fragment adapter could not locate the source color initialization",
        ));
    }
    lowered = lowered.replacen(anchor, "vec4 color = vulkanic_source_dh_atlas_color();", 1);
    apply_exact_atlas_distant_horizons_fragment_probe(
        &mut lowered,
        std::env::var("MATTMC_RUST_SELECTED_SOURCE_FRAGMENT_PROBE")
            .ok()
            .as_deref(),
    )?;
    Ok(lowered)
}

/// The regular selected-source fragment probe operates on the near-terrain
/// `tex`/`texCoord` interface. The exact-atlas DH adapter has a deliberately
/// different interface, so its probe must be injected here, after this
/// adapter has supplied the atlas color. Keeping it local prevents a capture
/// receipt from claiming that a near-terrain-only probe observed DH output.
fn apply_exact_atlas_distant_horizons_fragment_probe(
    source: &mut String,
    mode: Option<&str>,
) -> GalResult<()> {
    let Some(mode) = mode.map(str::trim).filter(|mode| !mode.is_empty()) else {
        return Ok(());
    };
    let (label, probe) = match mode {
        "atlas" => (
            "atlas",
            "out_distant_horizons_lit_color = color;\n    return;",
        ),
        "atlas-uv" => (
            "atlas-uv",
            "out_distant_horizons_lit_color = vec4(fract(vulkanic_source_dh_atlas_tile_uv), 0.0, 1.0);\n    return;",
        ),
        "atlas-alpha" => (
            "atlas-alpha",
            "out_distant_horizons_lit_color = vec4(vec3(color.a), 1.0);\n    return;",
        ),
        // These checkpoints preserve the real selected-source program and
        // its Rust-owned target/resources while locating whether the first
        // visible loss is before or inside the pack lighting function.
        // They remain unavailable unless a graphics-audit capture opts in.
        "pre-lighting" => (
            "pre-lighting",
            "out_distant_horizons_lit_color = color;\n    return;",
        ),
        "lightmap" => (
            "lightmap",
            "out_distant_horizons_lit_color = vec4(lmCoord, 0.0, 1.0);\n    return;",
        ),
        // Other modes are defined by the near-terrain source interface and
        // intentionally leave the DH program unmodified. This lets a single
        // capture use a terrain-only probe without inventing DH semantics.
        _ => return Ok(()),
    };
    let anchor = "vec4 color = vulkanic_source_dh_atlas_color();";
    if !source.contains(anchor) {
        return Err(GalError::invalid_argument(format!(
            "exact-atlas Distant Horizons {label} probe could not locate the adapter color initialization"
        )));
    }
    let insertion = match label {
        "pre-lighting" => {
            let anchor = "DoLighting(color, shadowMult,";
            source.find(anchor).ok_or_else(|| {
                GalError::invalid_argument(
                    "exact-atlas Distant Horizons pre-lighting probe could not locate the source lighting call",
                )
            })?
        }
        _ => source.find(anchor).expect("checked source anchor") + anchor.len(),
    };
    source.insert_str(
        insertion,
        &format!("\n    // selected-source diagnostic probe: {label}\n    {probe}\n"),
    );
    Ok(())
}

fn main_function_closing_brace(source: &str) -> Option<usize> {
    let main = source.find("void main()")?;
    let open = source[main..].find('{')? + main;
    let mut depth = 0usize;
    for (offset, character) in source[open..].char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(open + offset);
                }
            }
            _ => {}
        }
    }
    None
}

fn insert_before_main(source: &str, declarations: &str) -> GalResult<String> {
    let main = source.find("void main()").ok_or_else(|| {
        GalError::invalid_argument("lowered shader source has no void main function")
    })?;
    let mut output = String::with_capacity(source.len() + declarations.len());
    output.push_str(&source[..main]);
    output.push_str(declarations);
    output.push_str(&source[main..]);
    Ok(output)
}

/// Source-derived preparation for a pass-local fullscreen stage. This covers
/// deferred/composite consumers without relabeling them as terrain meshes or
/// inheriting an Iris/Java fullscreen draw. It owns no target, pipeline,
/// descriptor set, or route selection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoweredFullscreenSourceProgram {
    pub identity: ProgramIdentity,
    /// Pack-relative source stage identity retained for diagnostics and
    /// source-to-runtime correlation. It is semantic shader-pack metadata,
    /// not an attachment, native program, or backend handle.
    pub source_stage_path: String,
    pub shader_pack_generation: u64,
    /// Explicit Rust-owned geometry contract for this source stage. Backends
    /// receive only its draw count, never Java/Iris buffers or state.
    pub raster_primitive: FullscreenSourceRasterPrimitive,
    pub vertex: ShaderStageSource,
    pub fragment: ShaderStageSource,
    pub execution_interface: FullscreenSourceExecutionInterface,
    pub scalar_uniform_requirements: TerrainSourceUniformRequirements,
    pub opaque_resource_bindings: TerrainSourceOpaqueResourceBindingPlan,
    pub outputs: Vec<FullscreenSourceFragmentOutput>,
    /// A source fullscreen pass cannot sample and write the same semantic
    /// color resource in one draw. These requirements force a future Rust
    /// executor to allocate an explicit previous/current pair instead of
    /// accidentally binding one image for both uses.
    pub feedback_requirements: Vec<FullscreenSourceFeedbackRequirement>,
    /// Program-local source directives requesting mip generation for a
    /// semantic pack color before this pass samples it. The source language
    /// keeps these directives in the fragment stage; they are not global
    /// attachment metadata and cannot be guessed from texture usage alone.
    pub mipmap_requirements: Vec<FullscreenSourceMipmapRequirement>,
}

/// One source-declared sampled/output alias that needs a Rust-owned feedback
/// pair. The source location and binding are retained only for diagnostic
/// correlation; allocation and native image identity remain runtime-private.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FullscreenSourceFeedbackRequirement {
    pub role: TerrainSourceResourceRole,
    pub sampled_binding: u32,
    pub output_location: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FullscreenSourceMipmapRequirement {
    pub role: TerrainSourceResourceRole,
    pub sampled_binding: u32,
}

/// Fixed backend-neutral source ABI for a fullscreen pass. A Rust-owned
/// procedural triangle supplies position, primary UV, and secondary UV from
/// the draw vertex index, so no Java/Iris stream or backend-specific vertex
/// declaration participates. The second coordinate retains the semantic role
/// of legacy texture-coordinate set one. Set zero owns only semantic texture
/// transforms and scalar source uniforms; sampled resources remain in the
/// separate pack set.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FullscreenSourceExecutionInterface {
    pub texture_transforms: TerrainSourceFixedBinding,
    pub texture_transform_bytes: u32,
    pub scalar_uniforms: Option<TerrainSourceFixedBinding>,
    pub scalar_uniform_bytes: u32,
    pub scalar_uniform_fields: Vec<TerrainSourceUniformField>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FullscreenSourceExecutionLayouts {
    pub source_data: ResourceLayoutDesc,
    pub pack_resources: ResourceLayoutDesc,
}

impl FullscreenSourceExecutionInterface {
    const TEXTURE_TRANSFORMS: TerrainSourceFixedBinding = TerrainSourceFixedBinding {
        set: 0,
        binding: 0,
        kind: TerrainSourceBindingKind::UniformBuffer,
    };
    const SCALAR_UNIFORMS: TerrainSourceFixedBinding = TerrainSourceFixedBinding {
        set: 0,
        binding: 2,
        kind: TerrainSourceBindingKind::UniformBuffer,
    };

    fn from_lowered_pair(lowered: &LoweredFullscreenSourcePair) -> Self {
        let contract = lowered.uniform_contract();
        Self {
            texture_transforms: Self::TEXTURE_TRANSFORMS,
            texture_transform_bytes: 2 * 16 * std::mem::size_of::<f32>() as u32,
            scalar_uniforms: (!contract.fields().is_empty()).then_some(Self::SCALAR_UNIFORMS),
            scalar_uniform_bytes: contract.std140_size(),
            scalar_uniform_fields: contract.fields().to_vec(),
        }
    }

    pub fn validate(&self) -> GalResult<()> {
        if self.texture_transforms != Self::TEXTURE_TRANSFORMS
            || self.texture_transform_bytes != 128
        {
            return Err(GalError::invalid_argument(
                "fullscreen source texture transforms must use fixed set 0 binding 0 with two std140 mat4 values",
            ));
        }
        validate_source_scalar_uniform_block(
            self.scalar_uniforms,
            self.scalar_uniform_bytes,
            &self.scalar_uniform_fields,
            Self::SCALAR_UNIFORMS,
            "fullscreen source",
        )
    }
}

impl LoweredFullscreenSourceProgram {
    pub fn shader_module_descriptors(&self, api: BackendApi) -> [ShaderModuleDesc; 2] {
        [
            self.vertex.shader_module_descriptor(api),
            self.fragment.shader_module_descriptor(api),
        ]
    }

    pub fn pack_scalar_uniforms(&self, frame: &TerrainSourceUniformFrame) -> GalResult<Vec<u8>> {
        let bytes = frame.pack_std140(&self.scalar_uniform_requirements)?;
        if bytes.len() != self.execution_interface.scalar_uniform_bytes as usize {
            return Err(GalError::invalid_argument(format!(
                "fullscreen source scalar uniform pack is {} bytes but program ABI requires {}",
                bytes.len(),
                self.execution_interface.scalar_uniform_bytes
            )));
        }
        Ok(bytes)
    }

    pub fn pack_texture_transforms(
        &self,
        transforms: &TerrainSourceTextureTransforms,
    ) -> GalResult<Vec<u8>> {
        self.execution_interface.validate()?;
        transforms.validate()?;
        let mut bytes =
            Vec::with_capacity(self.execution_interface.texture_transform_bytes as usize);
        for value in transforms
            .atlas_texture_matrix
            .iter()
            .chain(transforms.lightmap_texture_matrix.iter())
        {
            bytes.extend_from_slice(&value.to_ne_bytes());
        }
        if bytes.len() != self.execution_interface.texture_transform_bytes as usize {
            return Err(GalError::invalid_argument(
                "fullscreen source texture transform pack does not match its fixed ABI size",
            ));
        }
        Ok(bytes)
    }

    pub fn execution_resource_layouts(&self) -> GalResult<FullscreenSourceExecutionLayouts> {
        self.execution_interface.validate()?;
        let mut source_data_bindings = vec![resource_binding_descriptor(
            self.execution_interface.texture_transforms,
            true,
        )];
        if let Some(scalar_uniforms) = self.execution_interface.scalar_uniforms {
            source_data_bindings.push(resource_binding_descriptor(scalar_uniforms, true));
        }
        source_data_bindings.sort_by_key(|binding| binding.binding);
        validate_unique_layout_bindings("fullscreen source data", &source_data_bindings)?;
        Ok(FullscreenSourceExecutionLayouts {
            source_data: ResourceLayoutDesc {
                label: format!("{}:source-data", self.identity.as_str()),
                bindings: source_data_bindings,
            },
            pack_resources: source_pack_resource_layout(
                format!("{}:pack-resources", self.identity.as_str()),
                &self.opaque_resource_bindings,
            )?,
        })
    }

    pub fn require_semantic_resources(
        &self,
        availability: &TerrainSourceResourceAvailabilitySet,
    ) -> GalResult<()> {
        if availability.shader_pack_generation() != self.shader_pack_generation {
            return Err(GalError::invalid_argument(format!(
                "fullscreen source program generation {} does not match resource availability generation {}",
                self.shader_pack_generation,
                availability.shader_pack_generation()
            )));
        }
        for binding in self.opaque_resource_bindings.bindings() {
            if availability.resource_for(binding.role()).is_none() {
                return Err(GalError::invalid_argument(format!(
                    "fullscreen source resource '{}' is unavailable for semantic role '{}'",
                    binding.resource_name(),
                    binding.role().semantic_name()
                )));
            }
        }
        Ok(())
    }

    /// Builds the deterministic set-one GAL description for the semantic
    /// fullscreen source-resource plan. The caller owns the resources and
    /// pass lifetime; this only maps already validated semantic roles into a
    /// backend-neutral GAL descriptor set.
    pub fn pack_resource_set_desc(
        &self,
        label: impl Into<String>,
        layout: Handle,
        resources: &TerrainSourceOwnedResourceSet,
    ) -> GalResult<ResourceSetDesc> {
        if layout.kind() != Some(HandleKind::ResourceLayout) {
            return Err(GalError::invalid_argument(
                "fullscreen source pack resources require a GAL resource-layout handle",
            ));
        }
        self.execution_resource_layouts()?;
        self.require_semantic_resources(resources.availability())?;
        let mut bindings = Vec::with_capacity(self.opaque_resource_bindings.bindings().len());
        for source_binding in self.opaque_resource_bindings.bindings() {
            let (resource, kind, access) = match source_binding.kind() {
                TerrainSourceOpaqueResourceKind::CombinedTextureSampler => (
                    resources
                        .combined_sampler_for(source_binding.role())
                        .ok_or_else(|| {
                            GalError::invalid_argument(format!(
                                "fullscreen source resource '{}' has no owned sampler for semantic role '{}'",
                                source_binding.resource_name(),
                                source_binding.role().semantic_name()
                            ))
                        })?,
                    ResourceBindingKind::CombinedTextureSampler,
                    AccessFlags::READ,
                ),
                TerrainSourceOpaqueResourceKind::StorageImage => (
                    resources
                        .storage_texture_for(source_binding.role())
                        .ok_or_else(|| {
                            GalError::invalid_argument(format!(
                                "fullscreen source storage resource '{}' has no owned texture view for semantic role '{}'",
                                source_binding.resource_name(),
                                source_binding.role().semantic_name()
                            ))
                        })?,
                    ResourceBindingKind::StorageTexture,
                    source_storage_access(source_binding.qualifiers())?,
                ),
            };
            bindings.push(ResourceBinding {
                binding: source_binding.binding(),
                array_index: 0,
                resource,
                kind,
                access,
                dynamic_offsets: Vec::new(),
                buffer_range: None,
            });
        }
        Ok(ResourceSetDesc {
            label: label.into(),
            layout,
            bindings,
        })
    }
}

/// Fixed Rust-owned descriptor ABI inserted by the DH source lowerer.
/// Binding numbers describe owned semantic resources only; they do not map to
/// Java, Iris, OpenGL, or Vulkan objects.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DistantHorizonsSourceExecutionInterface {
    pub vertex_stream: TerrainSourceFixedBinding,
    pub vertex_stride: u32,
    /// The texture identity carried by the source vertex stream. This is
    /// semantic shader-pack data, never a backend texture binding.
    pub material_identity_contract: DistantHorizonsMaterialIdentityContract,
    pub column_frame: TerrainSourceFixedBinding,
    pub column_frame_bytes: u32,
    pub scalar_uniforms: Option<TerrainSourceFixedBinding>,
    pub scalar_uniform_bytes: u32,
    pub scalar_uniform_fields: Vec<TerrainSourceUniformField>,
}

/// Describes whether a Distant Horizons source stream can identify the exact
/// Minecraft material sampled by a terrain fragment. A reduced vertex color
/// and material category are deliberately insufficient: they cannot stand in
/// for atlas UVs and an atlas-backed material identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DistantHorizonsMaterialIdentityContract {
    ReducedColorMaterialCategory,
    AtlasBacked,
}

/// Backend-neutral layouts for a lowered DH source program. Descriptor set
/// zero contains only fixed DH geometry/frame/scalar semantics; descriptor
/// set one contains source-declared semantic sampler/image roles.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DistantHorizonsSourceExecutionLayouts {
    pub source_data: ResourceLayoutDesc,
    pub pack_resources: ResourceLayoutDesc,
}

impl DistantHorizonsSourceExecutionInterface {
    const VERTEX_STREAM: TerrainSourceFixedBinding = TerrainSourceFixedBinding {
        set: 0,
        binding: 0,
        kind: TerrainSourceBindingKind::StorageBuffer,
    };
    const COLUMN_FRAME: TerrainSourceFixedBinding = TerrainSourceFixedBinding {
        set: 0,
        binding: 1,
        kind: TerrainSourceBindingKind::UniformBuffer,
    };
    const SCALAR_UNIFORMS: TerrainSourceFixedBinding = TerrainSourceFixedBinding {
        set: 0,
        binding: 2,
        kind: TerrainSourceBindingKind::UniformBuffer,
    };

    fn from_lowered_pair(lowered: &LoweredDistantHorizonsSourcePair) -> Self {
        let contract = lowered.uniform_contract();
        Self {
            vertex_stream: Self::VERTEX_STREAM,
            vertex_stride: DISTANT_HORIZONS_SOURCE_VERTEX_BYTES as u32,
            material_identity_contract:
                DistantHorizonsMaterialIdentityContract::ReducedColorMaterialCategory,
            column_frame: Self::COLUMN_FRAME,
            column_frame_bytes: DISTANT_HORIZONS_SOURCE_COLUMN_FRAME_BYTES as u32,
            scalar_uniforms: (!contract.fields().is_empty()).then_some(Self::SCALAR_UNIFORMS),
            scalar_uniform_bytes: contract.std140_size(),
            scalar_uniform_fields: contract.fields().to_vec(),
        }
    }

    pub fn validate(&self) -> GalResult<()> {
        if self.vertex_stream != Self::VERTEX_STREAM
            || !matches!(
                self.vertex_stride as usize,
                DISTANT_HORIZONS_SOURCE_VERTEX_BYTES
                    | DISTANT_HORIZONS_EXACT_ATLAS_SOURCE_VERTEX_BYTES
            )
        {
            return Err(GalError::invalid_argument(format!(
                "Distant Horizons source vertex stream must use fixed set 0 binding 0 with {}-byte reduced-color or {}-byte exact-atlas records",
                DISTANT_HORIZONS_SOURCE_VERTEX_BYTES,
                DISTANT_HORIZONS_EXACT_ATLAS_SOURCE_VERTEX_BYTES,
            )));
        }
        if self.column_frame != Self::COLUMN_FRAME
            || self.column_frame_bytes != DISTANT_HORIZONS_SOURCE_COLUMN_FRAME_BYTES as u32
        {
            return Err(GalError::invalid_argument(format!(
                "Distant Horizons source column frame must use fixed set 0 binding 1 with {} bytes",
                DISTANT_HORIZONS_SOURCE_COLUMN_FRAME_BYTES
            )));
        }
        validate_source_scalar_uniform_block(
            self.scalar_uniforms,
            self.scalar_uniform_bytes,
            &self.scalar_uniform_fields,
            Self::SCALAR_UNIFORMS,
            "Distant Horizons source",
        )
    }

    pub fn has_exact_material_texture_identity(&self) -> bool {
        matches!(
            self.material_identity_contract,
            DistantHorizonsMaterialIdentityContract::AtlasBacked
        )
    }
}

impl LoweredDistantHorizonsSourceProgram {
    /// A selected shader-pack DH program may own a frame only when its copied
    /// semantic stream can identify the exact material texture it samples.
    /// Declaring an atlas sampler alone is not sufficient; the vertex stream
    /// must also carry atlas-addressable material identity.
    pub fn has_exact_material_texture_identity(&self) -> bool {
        self.execution_interface
            .has_exact_material_texture_identity()
            && self
                .opaque_resource_bindings
                .bindings()
                .iter()
                .any(|binding| matches!(binding.role(), TerrainSourceResourceRole::MaterialAtlas))
    }

    pub fn shader_module_descriptors(&self, api: BackendApi) -> [ShaderModuleDesc; 2] {
        [
            self.vertex.shader_module_descriptor(api),
            self.fragment.shader_module_descriptor(api),
        ]
    }

    pub fn pack_scalar_uniforms(&self, frame: &TerrainSourceUniformFrame) -> GalResult<Vec<u8>> {
        let bytes = frame.pack_std140(&self.scalar_uniform_requirements)?;
        if bytes.len() != self.execution_interface.scalar_uniform_bytes as usize {
            return Err(GalError::invalid_argument(format!(
                "Distant Horizons source scalar uniform pack is {} bytes but program ABI requires {}",
                bytes.len(), self.execution_interface.scalar_uniform_bytes
            )));
        }
        Ok(bytes)
    }

    pub fn execution_resource_layouts(&self) -> GalResult<DistantHorizonsSourceExecutionLayouts> {
        self.execution_interface.validate()?;
        let mut source_data_bindings = vec![
            resource_binding_descriptor(self.execution_interface.vertex_stream, false),
            resource_binding_descriptor(self.execution_interface.column_frame, true),
        ];
        if let Some(scalar_uniforms) = self.execution_interface.scalar_uniforms {
            source_data_bindings.push(resource_binding_descriptor(scalar_uniforms, true));
        }
        source_data_bindings.sort_by_key(|binding| binding.binding);
        validate_unique_layout_bindings("Distant Horizons source data", &source_data_bindings)?;
        Ok(DistantHorizonsSourceExecutionLayouts {
            source_data: ResourceLayoutDesc {
                label: format!("{}:source-data", self.identity.as_str()),
                bindings: source_data_bindings,
            },
            pack_resources: source_pack_resource_layout(
                format!("{}:pack-resources", self.identity.as_str()),
                &self.opaque_resource_bindings,
            )?,
        })
    }

    /// Requires every source-declared sampler/image to be available as a
    /// Rust-owned semantic resource from the same shader-pack generation.
    /// This admits neither legacy DH textures nor Iris-managed bindings.
    pub fn require_semantic_resources(
        &self,
        availability: &TerrainSourceResourceAvailabilitySet,
    ) -> GalResult<()> {
        if availability.shader_pack_generation() != self.shader_pack_generation {
            return Err(GalError::invalid_argument(format!(
                "Distant Horizons source program generation {} does not match resource availability generation {}",
                self.shader_pack_generation,
                availability.shader_pack_generation()
            )));
        }
        for binding in self.opaque_resource_bindings.bindings() {
            if availability.resource_for(binding.role()).is_none() {
                return Err(GalError::invalid_argument(format!(
                    "Distant Horizons source resource '{}' is unavailable for semantic role '{}'",
                    binding.resource_name(),
                    binding.role().semantic_name()
                )));
            }
        }
        Ok(())
    }

    /// Builds the deterministic source pack-resource set for the DH program.
    /// The resource set contains only owned GAL handles after their semantic
    /// role and generation were checked above.
    pub fn pack_resource_set_desc(
        &self,
        label: impl Into<String>,
        layout: Handle,
        resources: &TerrainSourceOwnedResourceSet,
    ) -> GalResult<ResourceSetDesc> {
        if layout.kind() != Some(HandleKind::ResourceLayout) {
            return Err(GalError::invalid_argument(
                "Distant Horizons source pack resources require a GAL resource-layout handle",
            ));
        }
        self.execution_resource_layouts()?;
        self.require_semantic_resources(resources.availability())?;
        let mut bindings = Vec::with_capacity(self.opaque_resource_bindings.bindings().len());
        for source_binding in self.opaque_resource_bindings.bindings() {
            let (resource, kind, access) = match source_binding.kind() {
                TerrainSourceOpaqueResourceKind::CombinedTextureSampler => (
                    resources
                        .combined_sampler_for(source_binding.role())
                        .ok_or_else(|| {
                            GalError::invalid_argument(format!(
                                "Distant Horizons source resource '{}' has no owned sampler for semantic role '{}'",
                                source_binding.resource_name(),
                                source_binding.role().semantic_name()
                            ))
                        })?,
                    ResourceBindingKind::CombinedTextureSampler,
                    AccessFlags::READ,
                ),
                TerrainSourceOpaqueResourceKind::StorageImage => (
                    resources
                        .storage_texture_for(source_binding.role())
                        .ok_or_else(|| {
                            GalError::invalid_argument(format!(
                                "Distant Horizons source resource '{}' has no owned storage texture view for semantic role '{}'",
                                source_binding.resource_name(),
                                source_binding.role().semantic_name()
                            ))
                        })?,
                    ResourceBindingKind::StorageTexture,
                    source_storage_access(source_binding.qualifiers())?,
                ),
            };
            bindings.push(ResourceBinding {
                binding: source_binding.binding(),
                array_index: 0,
                resource,
                kind,
                access,
                dynamic_offsets: Vec::new(),
                buffer_range: None,
            });
        }
        Ok(ResourceSetDesc {
            label: label.into(),
            layout,
            bindings,
        })
    }
}

impl LoweredTerrainSourceProgram {
    /// Returns the source-derived terrain output schema when this is a normal
    /// terrain program. Consumers use the semantic names to build Rust-owned
    /// targets; attachment locations and native handles stay elsewhere.
    pub fn terrain_outputs(&self) -> Option<&[TerrainPassOutput]> {
        self.terrain_outputs.as_deref()
    }

    /// Retains the selected source pass's named-output to shader-pack-color
    /// mapping. Normal terrain consumers resolve this through the source
    /// target manifest before constructing an explicit GAL target.
    pub fn terrain_output_color_slots(&self) -> Option<&[(TerrainPassOutput, u32)]> {
        self.terrain_output_color_slots.as_deref()
    }

    /// Produces explicit GAL shader descriptions from the retained, lowered
    /// source pair. Creating these descriptions does not allocate shader
    /// modules, make a pipeline, bind resources, or select a render route.
    pub fn shader_module_descriptors(&self, api: BackendApi) -> [ShaderModuleDesc; 2] {
        [
            self.vertex.shader_module_descriptor(api),
            self.fragment.shader_module_descriptor(api),
        ]
    }

    /// Returns the source-declared raster semantics only for the independent
    /// translucent stage. Callers must translate this to explicit GAL state;
    /// it never exposes a legacy renderer state object.
    pub fn translucent_raster_state(&self) -> Option<TerrainTranslucentRasterState> {
        self.translucent_raster_state
    }

    /// Maps the source's semantic translucent blend rule to the explicit GAL
    /// pipeline model. This mapping is shared by both Rust backends; neither
    /// OpenGL blend state nor Vulkan blend factors leak out of the backends.
    pub fn translucent_blend_mode(&self) -> Option<BlendMode> {
        self.translucent_raster_state
            .map(|state| match state.blend {
                TerrainTranslucentBlend::SourceAlphaOver => BlendMode::Alpha,
            })
    }

    /// Packs only the named source semantics admitted by this prepared
    /// program. It cannot accept arbitrary bytes or backend state. The
    /// world frontend couples the returned bytes to the explicit GAL
    /// buffer/resource-set lifecycle in the selected Rust submission.
    pub fn pack_scalar_uniforms(&self, frame: &TerrainSourceUniformFrame) -> GalResult<Vec<u8>> {
        let bytes = frame.pack_std140(&self.scalar_uniform_requirements)?;
        if bytes.len() != self.execution_interface.scalar_uniform_bytes as usize {
            return Err(GalError::invalid_argument(format!(
                "terrain source scalar uniform pack is {} bytes but program ABI requires {}",
                bytes.len(),
                self.execution_interface.scalar_uniform_bytes
            )));
        }
        Ok(bytes)
    }

    /// Packs the two fixed legacy texture transforms consumed by the lowered
    /// source preamble. They remain an explicit semantic frame input instead
    /// of a borrowed Java/Iris uniform block or a guessed identity matrix.
    pub fn pack_legacy_texture_transforms(
        &self,
        transforms: &TerrainSourceTextureTransforms,
    ) -> GalResult<Vec<u8>> {
        self.execution_interface.validate()?;
        transforms.validate()?;
        let mut bytes =
            Vec::with_capacity(self.execution_interface.legacy_transform_bytes as usize);
        for value in transforms
            .atlas_texture_matrix
            .iter()
            .chain(transforms.lightmap_texture_matrix.iter())
        {
            bytes.extend_from_slice(&value.to_ne_bytes());
        }
        if bytes.len() != self.execution_interface.legacy_transform_bytes as usize {
            return Err(GalError::invalid_argument(
                "terrain source legacy texture transform pack does not match its fixed ABI size",
            ));
        }
        Ok(bytes)
    }

    /// Derives the closed GAL layout contract from the fixed source ABI and
    /// the selected pack's semantic resource plan. It never accepts native
    /// objects, Java renderer state, or arbitrary binding declarations.
    pub fn execution_resource_layouts(&self) -> GalResult<TerrainSourceExecutionLayouts> {
        self.execution_interface.validate()?;

        let mut source_data_bindings = vec![
            // Material vertices share the completion-gated frame stream with
            // transforms and scalar data. The explicit per-draw byte range
            // is selected through this dynamic offset.
            resource_binding_descriptor(self.execution_interface.vertex_stream, false),
            resource_binding_descriptor(self.execution_interface.legacy_transforms, true),
            resource_binding_descriptor(self.execution_interface.instance_stream, true),
        ];
        if let Some(scalar_uniforms) = self.execution_interface.scalar_uniforms {
            source_data_bindings.push(resource_binding_descriptor(scalar_uniforms, true));
        }
        source_data_bindings.sort_by_key(|binding| binding.binding);
        validate_unique_layout_bindings("terrain source data", &source_data_bindings)?;

        let mut pack_resource_bindings =
            Vec::with_capacity(self.opaque_resource_bindings.bindings().len());
        for source_binding in self.opaque_resource_bindings.bindings() {
            let kind = match source_binding.kind() {
                TerrainSourceOpaqueResourceKind::CombinedTextureSampler => {
                    ResourceBindingKind::CombinedTextureSampler
                }
                TerrainSourceOpaqueResourceKind::StorageImage => {
                    ResourceBindingKind::StorageTexture
                }
            };
            pack_resource_bindings.push(ResourceBindingDesc {
                binding: source_binding.binding(),
                kind,
                stages: PipelineStageFlags::DRAW,
                array_count: 1,
                optional: false,
                dynamic_offset_count: 0,
            });
        }
        pack_resource_bindings.sort_by_key(|binding| binding.binding);
        validate_unique_layout_bindings("terrain source pack resources", &pack_resource_bindings)?;

        Ok(TerrainSourceExecutionLayouts {
            source_data: ResourceLayoutDesc {
                label: format!("{}:source-data", self.identity.as_str()),
                bindings: source_data_bindings,
            },
            pack_resources: ResourceLayoutDesc {
                label: format!("{}:pack-resources", self.identity.as_str()),
                bindings: pack_resource_bindings,
            },
        })
    }

    /// Ensures every active source sampler role has a Rust-owned semantic
    /// resource in the same shader-pack generation. This remains independent
    /// of native handles and does not create a GAL resource set.
    pub fn require_semantic_resources(
        &self,
        availability: &TerrainSourceResourceAvailabilitySet,
    ) -> GalResult<()> {
        if availability.shader_pack_generation() != self.shader_pack_generation {
            return Err(GalError::invalid_argument(format!(
                "terrain source program generation {} does not match resource availability generation {}",
                self.shader_pack_generation,
                availability.shader_pack_generation()
            )));
        }
        for binding in self.opaque_resource_bindings.bindings() {
            if availability.resource_for(binding.role()).is_none() {
                return Err(GalError::invalid_argument(format!(
                    "terrain source resource '{}' is unavailable for semantic role '{}'",
                    binding.resource_name(),
                    binding.role().semantic_name()
                )));
            }
        }
        Ok(())
    }

    /// Builds the deterministic set-one GAL description for the semantic
    /// source-resource plan. The caller owns all semantic resource lifetimes;
    /// sampled and writable image bindings remain distinct in the resulting
    /// GAL set.
    pub fn pack_resource_set_desc(
        &self,
        label: impl Into<String>,
        layout: Handle,
        resources: &TerrainSourceOwnedResourceSet,
    ) -> GalResult<ResourceSetDesc> {
        if layout.kind() != Some(HandleKind::ResourceLayout) {
            return Err(GalError::invalid_argument(
                "terrain source pack resources require a GAL resource-layout handle",
            ));
        }
        self.execution_resource_layouts()?;
        self.require_semantic_resources(resources.availability())?;
        let mut bindings = Vec::with_capacity(self.opaque_resource_bindings.bindings().len());
        for source_binding in self.opaque_resource_bindings.bindings() {
            let (resource, kind, access) = match source_binding.kind() {
                TerrainSourceOpaqueResourceKind::CombinedTextureSampler => (
                    resources
                        .combined_sampler_for(source_binding.role())
                        .ok_or_else(|| {
                            GalError::invalid_argument(format!(
                                "terrain source resource '{}' has no owned sampler for semantic role '{}'",
                                source_binding.resource_name(),
                                source_binding.role().semantic_name()
                            ))
                        })?,
                    ResourceBindingKind::CombinedTextureSampler,
                    AccessFlags::READ,
                ),
                TerrainSourceOpaqueResourceKind::StorageImage => (
                    resources
                        .storage_texture_for(source_binding.role())
                        .ok_or_else(|| {
                            GalError::invalid_argument(format!(
                                "terrain source resource '{}' has no owned storage texture view for semantic role '{}'",
                                source_binding.resource_name(),
                                source_binding.role().semantic_name()
                            ))
                        })?,
                    ResourceBindingKind::StorageTexture,
                    source_storage_access(source_binding.qualifiers())?,
                ),
            };
            bindings.push(ResourceBinding {
                binding: source_binding.binding(),
                array_index: 0,
                resource,
                kind,
                access,
                dynamic_offsets: Vec::new(),
                buffer_range: None,
            });
        }
        Ok(ResourceSetDesc {
            label: label.into(),
            layout,
            bindings,
        })
    }
}

impl LoweredEntitySourceProgram {
    /// Converts retained entity source into explicit GAL shader descriptions.
    /// Backend compilation remains private to the backend and this does not
    /// allocate a pipeline or choose a rendering route.
    pub fn shader_module_descriptors(&self, api: BackendApi) -> [ShaderModuleDesc; 2] {
        [
            self.vertex.shader_module_descriptor(api),
            self.fragment.shader_module_descriptor(api),
        ]
    }

    /// Creates the source-derived entity stages for one explicit material
    /// alpha contract.  This is a semantic pipeline specialization, not a
    /// backend alpha-test state: Java's cutout policy is represented by the
    /// copied material mode and Rust supplies the corresponding shader hook.
    pub fn shader_module_descriptors_with_alpha_cutoff(
        &self,
        api: BackendApi,
        alpha_cutoff: Option<f32>,
    ) -> [ShaderModuleDesc; 2] {
        let fragment_source = match alpha_cutoff {
            Some(cutoff) => {
                debug_assert!(cutoff.is_finite() && cutoff >= 0.0);
                let declaration =
                    format!("#define VULKANIC_SOURCE_ENTITY_ALPHA_CUTOFF {cutoff:.8}\n");
                self.fragment.source.replacen(
                    "#version 450\n",
                    &format!("#version 450\n{declaration}"),
                    1,
                )
            }
            None => self.fragment.source.clone(),
        };
        [
            self.vertex.shader_module_descriptor(api),
            ShaderModuleDesc {
                label: self.fragment.label.clone(),
                stage: ShaderStage::Fragment,
                code_format: ShaderCodeFormat::Glsl,
                code: shader_stage_code_for_backend(api, &fragment_source),
                entry_point: self.fragment.entry_point.clone(),
            },
        ]
    }

    /// Packs only source-declared entity uniforms through the typed semantic
    /// catalog. The caller must supply resolved `entityId` and `entityColor`;
    /// arbitrary bytes and Java/Iris uniform state are rejected by design.
    pub fn pack_scalar_uniforms(&self, frame: &TerrainSourceUniformFrame) -> GalResult<Vec<u8>> {
        let bytes = frame.pack_std140(&self.scalar_uniform_requirements)?;
        if bytes.len() != self.execution_interface.scalar_uniform_bytes as usize {
            return Err(GalError::invalid_argument(format!(
                "entity source scalar uniform pack is {} bytes but program ABI requires {}",
                bytes.len(),
                self.execution_interface.scalar_uniform_bytes
            )));
        }
        Ok(bytes)
    }

    /// Packs the fixed source texture/lightmap transform block used by the
    /// owned entity stream. Entity-local UVs use the identity atlas lane;
    /// packed light coordinates retain the same explicit Minecraft semantic
    /// conversion as terrain. No Java/Iris uniform block is borrowed.
    pub fn pack_legacy_texture_transforms(
        &self,
        transforms: &TerrainSourceTextureTransforms,
    ) -> GalResult<Vec<u8>> {
        self.execution_interface.validate()?;
        transforms.validate()?;
        let mut bytes =
            Vec::with_capacity(self.execution_interface.legacy_transform_bytes as usize);
        for value in transforms
            .atlas_texture_matrix
            .iter()
            .chain(transforms.lightmap_texture_matrix.iter())
        {
            bytes.extend_from_slice(&value.to_ne_bytes());
        }
        if bytes.len() != self.execution_interface.legacy_transform_bytes as usize {
            return Err(GalError::invalid_argument(
                "entity source legacy texture transform pack does not match its fixed ABI size",
            ));
        }
        Ok(bytes)
    }

    /// Derives the closed GAL layout contract for one Rust-owned entity
    /// writer. It uses the same fixed source stream ABI as terrain, but its
    /// set-one roles remain entity-local (not terrain-atlas aliases).
    pub fn execution_resource_layouts(&self) -> GalResult<TerrainSourceExecutionLayouts> {
        self.execution_interface.validate()?;
        let mut source_data_bindings = vec![
            resource_binding_descriptor(self.execution_interface.vertex_stream, false),
            resource_binding_descriptor(self.execution_interface.legacy_transforms, true),
            resource_binding_descriptor(self.execution_interface.instance_stream, true),
        ];
        if let Some(scalar_uniforms) = self.execution_interface.scalar_uniforms {
            source_data_bindings.push(resource_binding_descriptor(scalar_uniforms, true));
        }
        source_data_bindings.sort_by_key(|binding| binding.binding);
        validate_unique_layout_bindings("entity source data", &source_data_bindings)?;

        let mut pack_resource_bindings =
            Vec::with_capacity(self.opaque_resource_bindings.bindings().len());
        for source_binding in self.opaque_resource_bindings.bindings() {
            let kind = match source_binding.kind() {
                TerrainSourceOpaqueResourceKind::CombinedTextureSampler => {
                    ResourceBindingKind::CombinedTextureSampler
                }
                TerrainSourceOpaqueResourceKind::StorageImage => {
                    ResourceBindingKind::StorageTexture
                }
            };
            pack_resource_bindings.push(ResourceBindingDesc {
                binding: source_binding.binding(),
                kind,
                stages: PipelineStageFlags::DRAW,
                array_count: 1,
                optional: false,
                dynamic_offset_count: 0,
            });
        }
        pack_resource_bindings.sort_by_key(|binding| binding.binding);
        validate_unique_layout_bindings("entity source pack resources", &pack_resource_bindings)?;

        Ok(TerrainSourceExecutionLayouts {
            source_data: ResourceLayoutDesc {
                label: format!("{}:source-data", self.identity.as_str()),
                bindings: source_data_bindings,
            },
            pack_resources: ResourceLayoutDesc {
                label: format!("{}:pack-resources", self.identity.as_str()),
                bindings: pack_resource_bindings,
            },
        })
    }

    /// Validates that every selected-pack entity resource is owned by Rust
    /// for the same source generation. This is semantic availability only;
    /// no descriptor set or backend object is created here.
    pub fn require_semantic_resources(
        &self,
        availability: &TerrainSourceResourceAvailabilitySet,
    ) -> GalResult<()> {
        if availability.shader_pack_generation() != self.shader_pack_generation {
            return Err(GalError::invalid_argument(format!(
                "entity source program generation {} does not match resource availability generation {}",
                self.shader_pack_generation,
                availability.shader_pack_generation()
            )));
        }
        for binding in self.opaque_resource_bindings.bindings() {
            if availability.resource_for(binding.role()).is_none() {
                return Err(GalError::invalid_argument(format!(
                    "entity source resource '{}' is unavailable for semantic role '{}'",
                    binding.resource_name(),
                    binding.role().semantic_name()
                )));
            }
        }
        Ok(())
    }

    /// Builds the explicit semantic set-one description for a future owned
    /// entity pass. The caller must provide a local material resource from
    /// Rust's cache; Java/Iris bindings cannot enter this boundary.
    pub fn pack_resource_set_desc(
        &self,
        label: impl Into<String>,
        layout: Handle,
        resources: &TerrainSourceOwnedResourceSet,
    ) -> GalResult<ResourceSetDesc> {
        if layout.kind() != Some(HandleKind::ResourceLayout) {
            return Err(GalError::invalid_argument(
                "entity source pack resources require a GAL resource-layout handle",
            ));
        }
        self.execution_resource_layouts()?;
        self.require_semantic_resources(resources.availability())?;
        let mut bindings = Vec::with_capacity(self.opaque_resource_bindings.bindings().len());
        for source_binding in self.opaque_resource_bindings.bindings() {
            let (resource, kind, access) = match source_binding.kind() {
                TerrainSourceOpaqueResourceKind::CombinedTextureSampler => (
                    resources
                        .combined_sampler_for(source_binding.role())
                        .ok_or_else(|| {
                            GalError::invalid_argument(format!(
                                "entity source resource '{}' has no owned sampler for semantic role '{}'",
                                source_binding.resource_name(),
                                source_binding.role().semantic_name()
                            ))
                        })?,
                    ResourceBindingKind::CombinedTextureSampler,
                    AccessFlags::READ,
                ),
                TerrainSourceOpaqueResourceKind::StorageImage => (
                    resources
                        .storage_texture_for(source_binding.role())
                        .ok_or_else(|| {
                            GalError::invalid_argument(format!(
                                "entity source resource '{}' has no owned storage texture view for semantic role '{}'",
                                source_binding.resource_name(),
                                source_binding.role().semantic_name()
                            ))
                        })?,
                    ResourceBindingKind::StorageTexture,
                    source_storage_access(source_binding.qualifiers())?,
                ),
            };
            bindings.push(ResourceBinding {
                binding: source_binding.binding(),
                array_index: 0,
                resource,
                kind,
                access,
                dynamic_offsets: Vec::new(),
                buffer_range: None,
            });
        }
        Ok(ResourceSetDesc {
            label: label.into(),
            layout,
            bindings,
        })
    }

    /// Resolves one copied semantic entity identity through the immutable
    /// source generation retained by this program. This is CPU-only semantic
    /// preparation, not a backend binding or uniform-location lookup.
    pub fn resolve_draw_semantics(
        &self,
        entity_identity: &str,
        entity_color_argb: u32,
    ) -> GalResult<EntitySourceDrawSemantics> {
        let semantics = self
            .entity_contract
            .resolve_draw_semantics(entity_identity, entity_color_argb)?;
        if semantics.entity_id_generation != self.entity_id_generation {
            return Err(GalError::invalid_argument(
                "entity source program resolved a stale entity-id generation",
            ));
        }
        Ok(semantics)
    }

    /// Named source outputs retain their shader-pack slot metadata without
    /// exposing attachment indices or backend framebuffer state.
    pub fn named_output_color_slots(&self) -> &[(TerrainPassOutput, u32)] {
        &self.named_output_color_slots
    }
}

impl LocalTexturedSourceProgram for LoweredEntitySourceProgram {
    fn identity(&self) -> &ProgramIdentity {
        &self.identity
    }

    fn shader_pack_generation(&self) -> u64 {
        self.shader_pack_generation
    }

    fn execution_interface(&self) -> &TerrainSourceExecutionInterface {
        &self.execution_interface
    }

    fn shader_module_descriptors_with_alpha_cutoff(
        &self,
        api: BackendApi,
        alpha_cutoff: Option<f32>,
    ) -> [ShaderModuleDesc; 2] {
        LoweredEntitySourceProgram::shader_module_descriptors_with_alpha_cutoff(
            self,
            api,
            alpha_cutoff,
        )
    }

    fn execution_resource_layouts(&self) -> GalResult<TerrainSourceExecutionLayouts> {
        LoweredEntitySourceProgram::execution_resource_layouts(self)
    }

    fn require_semantic_resources(
        &self,
        availability: &TerrainSourceResourceAvailabilitySet,
    ) -> GalResult<()> {
        LoweredEntitySourceProgram::require_semantic_resources(self, availability)
    }

    fn pack_resource_set_desc(
        &self,
        label: String,
        layout: Handle,
        resources: &TerrainSourceOwnedResourceSet,
    ) -> GalResult<ResourceSetDesc> {
        LoweredEntitySourceProgram::pack_resource_set_desc(self, label, layout, resources)
    }
}

impl LoweredHandSourceProgram {
    pub fn shader_module_descriptors(&self, api: BackendApi) -> [ShaderModuleDesc; 2] {
        [
            self.vertex.shader_module_descriptor(api),
            self.fragment.shader_module_descriptor(api),
        ]
    }

    /// The first-person writer has the same explicit cutout contract as an
    /// entity writer, but the projection/depth domain remains hand-specific.
    pub fn shader_module_descriptors_with_alpha_cutoff(
        &self,
        api: BackendApi,
        alpha_cutoff: Option<f32>,
    ) -> [ShaderModuleDesc; 2] {
        let fragment_source = match alpha_cutoff {
            Some(cutoff) => {
                debug_assert!(cutoff.is_finite() && cutoff >= 0.0);
                let declaration =
                    format!("#define VULKANIC_SOURCE_ENTITY_ALPHA_CUTOFF {cutoff:.8}\n");
                self.fragment.source.replacen(
                    "#version 450\n",
                    &format!("#version 450\n{declaration}"),
                    1,
                )
            }
            None => self.fragment.source.clone(),
        };
        [
            self.vertex.shader_module_descriptor(api),
            ShaderModuleDesc {
                label: self.fragment.label.clone(),
                stage: ShaderStage::Fragment,
                code_format: ShaderCodeFormat::Glsl,
                code: shader_stage_code_for_backend(api, &fragment_source),
                entry_point: self.fragment.entry_point.clone(),
            },
        ]
    }

    pub fn pack_scalar_uniforms(&self, frame: &TerrainSourceUniformFrame) -> GalResult<Vec<u8>> {
        let bytes = frame.pack_std140(&self.scalar_uniform_requirements)?;
        if bytes.len() != self.execution_interface.scalar_uniform_bytes as usize {
            return Err(GalError::invalid_argument(format!(
                "hand source scalar uniform pack is {} bytes but program ABI requires {}",
                bytes.len(),
                self.execution_interface.scalar_uniform_bytes
            )));
        }
        Ok(bytes)
    }

    pub fn pack_legacy_texture_transforms(
        &self,
        transforms: &TerrainSourceTextureTransforms,
    ) -> GalResult<Vec<u8>> {
        self.execution_interface.validate()?;
        transforms.validate()?;
        let mut bytes =
            Vec::with_capacity(self.execution_interface.legacy_transform_bytes as usize);
        for value in transforms
            .atlas_texture_matrix
            .iter()
            .chain(transforms.lightmap_texture_matrix.iter())
        {
            bytes.extend_from_slice(&value.to_ne_bytes());
        }
        if bytes.len() != self.execution_interface.legacy_transform_bytes as usize {
            return Err(GalError::invalid_argument(
                "hand source legacy texture transform pack does not match its fixed ABI size",
            ));
        }
        Ok(bytes)
    }

    pub fn execution_resource_layouts(&self) -> GalResult<TerrainSourceExecutionLayouts> {
        self.execution_interface.validate()?;
        let mut source_data_bindings = vec![
            resource_binding_descriptor(self.execution_interface.vertex_stream, false),
            resource_binding_descriptor(self.execution_interface.legacy_transforms, true),
            resource_binding_descriptor(self.execution_interface.instance_stream, true),
        ];
        if let Some(scalar_uniforms) = self.execution_interface.scalar_uniforms {
            source_data_bindings.push(resource_binding_descriptor(scalar_uniforms, true));
        }
        source_data_bindings.sort_by_key(|binding| binding.binding);
        validate_unique_layout_bindings("hand source data", &source_data_bindings)?;

        let mut pack_resource_bindings =
            Vec::with_capacity(self.opaque_resource_bindings.bindings().len());
        for source_binding in self.opaque_resource_bindings.bindings() {
            let kind = match source_binding.kind() {
                TerrainSourceOpaqueResourceKind::CombinedTextureSampler => {
                    ResourceBindingKind::CombinedTextureSampler
                }
                TerrainSourceOpaqueResourceKind::StorageImage => {
                    ResourceBindingKind::StorageTexture
                }
            };
            pack_resource_bindings.push(ResourceBindingDesc {
                binding: source_binding.binding(),
                kind,
                stages: PipelineStageFlags::DRAW,
                array_count: 1,
                optional: false,
                dynamic_offset_count: 0,
            });
        }
        pack_resource_bindings.sort_by_key(|binding| binding.binding);
        validate_unique_layout_bindings("hand source pack resources", &pack_resource_bindings)?;
        Ok(TerrainSourceExecutionLayouts {
            source_data: ResourceLayoutDesc {
                label: format!("{}:source-data", self.identity.as_str()),
                bindings: source_data_bindings,
            },
            pack_resources: ResourceLayoutDesc {
                label: format!("{}:pack-resources", self.identity.as_str()),
                bindings: pack_resource_bindings,
            },
        })
    }

    pub fn require_semantic_resources(
        &self,
        availability: &TerrainSourceResourceAvailabilitySet,
    ) -> GalResult<()> {
        if availability.shader_pack_generation() != self.shader_pack_generation {
            return Err(GalError::invalid_argument(format!(
                "hand source program generation {} does not match resource availability generation {}",
                self.shader_pack_generation,
                availability.shader_pack_generation()
            )));
        }
        for binding in self.opaque_resource_bindings.bindings() {
            if availability.resource_for(binding.role()).is_none() {
                return Err(GalError::invalid_argument(format!(
                    "hand source resource '{}' is unavailable for semantic role '{}'",
                    binding.resource_name(),
                    binding.role().semantic_name()
                )));
            }
        }
        Ok(())
    }

    pub fn pack_resource_set_desc(
        &self,
        label: impl Into<String>,
        layout: Handle,
        resources: &TerrainSourceOwnedResourceSet,
    ) -> GalResult<ResourceSetDesc> {
        if layout.kind() != Some(HandleKind::ResourceLayout) {
            return Err(GalError::invalid_argument(
                "hand source pack resources require a GAL resource-layout handle",
            ));
        }
        self.execution_resource_layouts()?;
        self.require_semantic_resources(resources.availability())?;
        let mut bindings = Vec::with_capacity(self.opaque_resource_bindings.bindings().len());
        for source_binding in self.opaque_resource_bindings.bindings() {
            let (resource, kind, access) = match source_binding.kind() {
                TerrainSourceOpaqueResourceKind::CombinedTextureSampler => (
                    resources.combined_sampler_for(source_binding.role()).ok_or_else(|| {
                        GalError::invalid_argument(format!(
                            "hand source resource '{}' has no owned sampler for semantic role '{}'",
                            source_binding.resource_name(),
                            source_binding.role().semantic_name()
                        ))
                    })?,
                    ResourceBindingKind::CombinedTextureSampler,
                    AccessFlags::READ,
                ),
                TerrainSourceOpaqueResourceKind::StorageImage => (
                    resources.storage_texture_for(source_binding.role()).ok_or_else(|| {
                        GalError::invalid_argument(format!(
                            "hand source resource '{}' has no owned storage texture view for semantic role '{}'",
                            source_binding.resource_name(),
                            source_binding.role().semantic_name()
                        ))
                    })?,
                    ResourceBindingKind::StorageTexture,
                    source_storage_access(source_binding.qualifiers())?,
                ),
            };
            bindings.push(ResourceBinding {
                binding: source_binding.binding(),
                array_index: 0,
                resource,
                kind,
                access,
                dynamic_offsets: Vec::new(),
                buffer_range: None,
            });
        }
        Ok(ResourceSetDesc {
            label: label.into(),
            layout,
            bindings,
        })
    }

    pub fn named_output_color_slots(&self) -> &[(TerrainPassOutput, u32)] {
        &self.named_output_color_slots
    }

    pub fn contract(&self) -> &HandPassContract {
        &self.hand_contract
    }
}

impl LocalTexturedSourceProgram for LoweredHandSourceProgram {
    fn identity(&self) -> &ProgramIdentity {
        &self.identity
    }

    fn shader_pack_generation(&self) -> u64 {
        self.shader_pack_generation
    }

    fn execution_interface(&self) -> &TerrainSourceExecutionInterface {
        &self.execution_interface
    }

    fn shader_module_descriptors_with_alpha_cutoff(
        &self,
        api: BackendApi,
        alpha_cutoff: Option<f32>,
    ) -> [ShaderModuleDesc; 2] {
        LoweredHandSourceProgram::shader_module_descriptors_with_alpha_cutoff(
            self,
            api,
            alpha_cutoff,
        )
    }

    fn execution_resource_layouts(&self) -> GalResult<TerrainSourceExecutionLayouts> {
        LoweredHandSourceProgram::execution_resource_layouts(self)
    }

    fn require_semantic_resources(
        &self,
        availability: &TerrainSourceResourceAvailabilitySet,
    ) -> GalResult<()> {
        LoweredHandSourceProgram::require_semantic_resources(self, availability)
    }

    fn pack_resource_set_desc(
        &self,
        label: String,
        layout: Handle,
        resources: &TerrainSourceOwnedResourceSet,
    ) -> GalResult<ResourceSetDesc> {
        LoweredHandSourceProgram::pack_resource_set_desc(self, label, layout, resources)
    }
}

impl LoweredTexturedMaterialSourceProgram {
    /// Exact named shader-pack outputs produced by this source stage. These
    /// remain semantic target roles; neither a GLSL output location nor a
    /// backend attachment index escapes program preparation.
    pub fn named_output_color_slots(&self) -> &[(TerrainPassOutput, u32)] {
        &self.named_output_color_slots
    }

    pub fn shader_module_descriptors(&self, api: BackendApi) -> [ShaderModuleDesc; 2] {
        [
            self.vertex.shader_module_descriptor(api),
            self.fragment.shader_module_descriptor(api),
        ]
    }

    /// Builds the bounded `gbuffers_textured` source stream. The selected
    /// program declares one semantic base-color sampler; the frontend binds
    /// either the copied terrain atlas or one Rust-owned local material asset
    /// for a compatible batch. No backend object or Java texture leaks into
    /// that selection.
    pub fn pack_material_primitives(
        &self,
        primitives: &[TexturedMaterialSourcePrimitive],
    ) -> GalResult<Vec<u8>> {
        self.execution_interface.validate()?;
        if !self
            .opaque_resource_bindings
            .bindings()
            .iter()
            .any(|binding| binding.role() == TerrainSourceResourceRole::MaterialAtlas)
        {
            return Err(GalError::unsupported_feature(
                "textured material source program has no declared base-color sampler",
            ));
        }
        pack_textured_material_source_primitives(primitives)
    }

    pub fn pack_scalar_uniforms(&self, frame: &TerrainSourceUniformFrame) -> GalResult<Vec<u8>> {
        self.execution_interface.validate()?;
        let bytes = frame.pack_std140(&self.scalar_uniform_requirements)?;
        if bytes.len() != self.execution_interface.scalar_uniform_bytes as usize {
            return Err(GalError::invalid_argument(format!(
                "textured material source scalar uniform pack is {} bytes but program ABI requires {}",
                bytes.len(), self.execution_interface.scalar_uniform_bytes
            )));
        }
        Ok(bytes)
    }

    pub fn pack_legacy_texture_transforms(
        &self,
        transforms: &TerrainSourceTextureTransforms,
    ) -> GalResult<Vec<u8>> {
        self.execution_interface.validate()?;
        transforms.validate()?;
        let mut bytes =
            Vec::with_capacity(self.execution_interface.legacy_transform_bytes as usize);
        for value in transforms
            .atlas_texture_matrix
            .iter()
            .chain(transforms.lightmap_texture_matrix.iter())
        {
            bytes.extend_from_slice(&value.to_ne_bytes());
        }
        if bytes.len() != self.execution_interface.legacy_transform_bytes as usize {
            return Err(GalError::invalid_argument(
                "textured material source texture transforms do not match their fixed ABI size",
            ));
        }
        Ok(bytes)
    }

    pub fn execution_resource_layouts(&self) -> GalResult<TexturedMaterialSourceExecutionLayouts> {
        self.execution_interface.validate()?;
        let mut source_data_bindings = vec![
            resource_binding_descriptor(self.execution_interface.vertex_stream, true),
            resource_binding_descriptor(self.execution_interface.legacy_transforms, true),
        ];
        if let Some(scalar_uniforms) = self.execution_interface.scalar_uniforms {
            source_data_bindings.push(resource_binding_descriptor(scalar_uniforms, true));
        }
        source_data_bindings.sort_by_key(|binding| binding.binding);
        validate_unique_layout_bindings("textured material source data", &source_data_bindings)?;
        Ok(TexturedMaterialSourceExecutionLayouts {
            source_data: ResourceLayoutDesc {
                label: format!("{}:source-data", self.identity.as_str()),
                bindings: source_data_bindings,
            },
            pack_resources: source_pack_resource_layout(
                format!("{}:pack-resources", self.identity.as_str()),
                &self.opaque_resource_bindings,
            )?,
        })
    }

    pub fn require_semantic_resources(
        &self,
        availability: &TerrainSourceResourceAvailabilitySet,
    ) -> GalResult<()> {
        if availability.shader_pack_generation() != self.shader_pack_generation {
            return Err(GalError::invalid_argument(format!(
                "textured material source generation {} does not match resource generation {}",
                self.shader_pack_generation,
                availability.shader_pack_generation()
            )));
        }
        for binding in self.opaque_resource_bindings.bindings() {
            if availability.resource_for(binding.role()).is_none() {
                return Err(GalError::invalid_argument(format!(
                    "textured material source resource '{}' is unavailable for semantic role '{}'",
                    binding.resource_name(),
                    binding.role().semantic_name()
                )));
            }
        }
        Ok(())
    }

    /// Materializes the selected source's set-one semantic resource plan for
    /// the compact textured-material writer. Set zero remains a distinct
    /// Rust-owned quad stream; this method deliberately shares only the
    /// backend-neutral resource-role contract with terrain.
    pub fn pack_resource_set_desc(
        &self,
        label: impl Into<String>,
        layout: Handle,
        resources: &TerrainSourceOwnedResourceSet,
    ) -> GalResult<ResourceSetDesc> {
        if layout.kind() != Some(HandleKind::ResourceLayout) {
            return Err(GalError::invalid_argument(
                "textured material source pack resources require a GAL resource-layout handle",
            ));
        }
        self.execution_resource_layouts()?;
        self.require_semantic_resources(resources.availability())?;
        let mut bindings = Vec::with_capacity(self.opaque_resource_bindings.bindings().len());
        for source_binding in self.opaque_resource_bindings.bindings() {
            let (resource, kind, access) = match source_binding.kind() {
                TerrainSourceOpaqueResourceKind::CombinedTextureSampler => (
                    resources
                        .combined_sampler_for(source_binding.role())
                        .ok_or_else(|| {
                            GalError::invalid_argument(format!(
                                "textured material source resource '{}' has no owned sampler for semantic role '{}'",
                                source_binding.resource_name(),
                                source_binding.role().semantic_name()
                            ))
                        })?,
                    ResourceBindingKind::CombinedTextureSampler,
                    AccessFlags::READ,
                ),
                TerrainSourceOpaqueResourceKind::StorageImage => (
                    resources
                        .storage_texture_for(source_binding.role())
                        .ok_or_else(|| {
                            GalError::invalid_argument(format!(
                                "textured material source resource '{}' has no owned storage texture view for semantic role '{}'",
                                source_binding.resource_name(),
                                source_binding.role().semantic_name()
                            ))
                        })?,
                    ResourceBindingKind::StorageTexture,
                    source_storage_access(source_binding.qualifiers())?,
                ),
            };
            bindings.push(ResourceBinding {
                binding: source_binding.binding(),
                array_index: 0,
                resource,
                kind,
                access,
                dynamic_offsets: Vec::new(),
                buffer_range: None,
            });
        }
        Ok(ResourceSetDesc {
            label: label.into(),
            layout,
            bindings,
        })
    }
}

impl LoweredWeatherSourceProgram {
    /// Adapts the weather program to the shared compact world-material stream.
    /// The adapter preserves the weather program identity, shader source,
    /// named output, and semantic bindings; it merely avoids a second copy of
    /// the Rust-owned quad transport and resource-set machinery.
    pub fn material_stream_program(&self) -> LoweredTexturedMaterialSourceProgram {
        LoweredTexturedMaterialSourceProgram {
            identity: self.identity.clone(),
            shader_pack_generation: self.shader_pack_generation,
            vertex: self.vertex.clone(),
            fragment: self.fragment.clone(),
            execution_interface: self.execution_interface.clone(),
            scalar_uniform_requirements: self.scalar_uniform_requirements.clone(),
            opaque_resource_bindings: self.opaque_resource_bindings.clone(),
            named_output_color_slots: vec![(
                TerrainPassOutput::LitTerrainColor,
                u32::from(self.lit_color_output_slot),
            )],
        }
    }

    pub fn shader_module_descriptors(&self, api: BackendApi) -> [ShaderModuleDesc; 2] {
        [
            self.vertex.shader_module_descriptor(api),
            self.fragment.shader_module_descriptor(api),
        ]
    }

    pub fn alpha_discard_threshold(&self) -> f32 {
        f32::from_bits(self.alpha_discard_threshold_bits)
    }

    pub fn execution_resource_layouts(&self) -> GalResult<TexturedMaterialSourceExecutionLayouts> {
        self.execution_interface.validate()?;
        let mut source_data_bindings = vec![
            resource_binding_descriptor(self.execution_interface.vertex_stream, true),
            resource_binding_descriptor(self.execution_interface.legacy_transforms, true),
        ];
        if let Some(scalar_uniforms) = self.execution_interface.scalar_uniforms {
            source_data_bindings.push(resource_binding_descriptor(scalar_uniforms, true));
        }
        source_data_bindings.sort_by_key(|binding| binding.binding);
        validate_unique_layout_bindings("weather source data", &source_data_bindings)?;
        Ok(TexturedMaterialSourceExecutionLayouts {
            source_data: ResourceLayoutDesc {
                label: format!("{}:source-data", self.identity.as_str()),
                bindings: source_data_bindings,
            },
            pack_resources: source_pack_resource_layout(
                format!("{}:pack-resources", self.identity.as_str()),
                &self.opaque_resource_bindings,
            )?,
        })
    }

    pub fn require_semantic_resources(
        &self,
        availability: &TerrainSourceResourceAvailabilitySet,
    ) -> GalResult<()> {
        if availability.shader_pack_generation() != self.shader_pack_generation {
            return Err(GalError::invalid_argument(format!(
                "weather source generation {} does not match resource generation {}",
                self.shader_pack_generation,
                availability.shader_pack_generation()
            )));
        }
        for binding in self.opaque_resource_bindings.bindings() {
            if availability.resource_for(binding.role()).is_none() {
                return Err(GalError::invalid_argument(format!(
                    "weather source resource '{}' is unavailable for semantic role '{}'",
                    binding.resource_name(),
                    binding.role().semantic_name()
                )));
            }
        }
        Ok(())
    }
}

impl LoweredCloudSourceProgram {
    /// Adapts the cloud stage to the shared compact material stream while
    /// preserving cloud's separate source identity and named outputs.
    pub fn material_stream_program(&self) -> LoweredTexturedMaterialSourceProgram {
        LoweredTexturedMaterialSourceProgram {
            identity: self.identity.clone(),
            shader_pack_generation: self.shader_pack_generation,
            vertex: self.vertex.clone(),
            fragment: self.fragment.clone(),
            execution_interface: self.execution_interface.clone(),
            scalar_uniform_requirements: self.scalar_uniform_requirements.clone(),
            opaque_resource_bindings: self.opaque_resource_bindings.clone(),
            named_output_color_slots: self.named_output_color_slots.clone(),
        }
    }

    pub fn shader_module_descriptors(&self, api: BackendApi) -> [ShaderModuleDesc; 2] {
        [
            self.vertex.shader_module_descriptor(api),
            self.fragment.shader_module_descriptor(api),
        ]
    }

    pub fn named_output_color_slots(&self) -> &[(TerrainPassOutput, u32)] {
        &self.named_output_color_slots
    }

    pub fn execution_resource_layouts(&self) -> GalResult<TexturedMaterialSourceExecutionLayouts> {
        self.execution_interface.validate()?;
        let mut source_data_bindings = vec![
            resource_binding_descriptor(self.execution_interface.vertex_stream, true),
            resource_binding_descriptor(self.execution_interface.legacy_transforms, true),
        ];
        if let Some(scalar_uniforms) = self.execution_interface.scalar_uniforms {
            source_data_bindings.push(resource_binding_descriptor(scalar_uniforms, true));
        }
        source_data_bindings.sort_by_key(|binding| binding.binding);
        validate_unique_layout_bindings("cloud source data", &source_data_bindings)?;
        Ok(TexturedMaterialSourceExecutionLayouts {
            source_data: ResourceLayoutDesc {
                label: format!("{}:source-data", self.identity.as_str()),
                bindings: source_data_bindings,
            },
            pack_resources: source_pack_resource_layout(
                format!("{}:pack-resources", self.identity.as_str()),
                &self.opaque_resource_bindings,
            )?,
        })
    }

    pub fn require_semantic_resources(
        &self,
        availability: &TerrainSourceResourceAvailabilitySet,
    ) -> GalResult<()> {
        if availability.shader_pack_generation() != self.shader_pack_generation {
            return Err(GalError::invalid_argument(format!(
                "cloud source generation {} does not match resource generation {}",
                self.shader_pack_generation,
                availability.shader_pack_generation()
            )));
        }
        for binding in self.opaque_resource_bindings.bindings() {
            if availability.resource_for(binding.role()).is_none() {
                return Err(GalError::invalid_argument(format!(
                    "cloud source resource '{}' is unavailable for semantic role '{}'",
                    binding.resource_name(),
                    binding.role().semantic_name()
                )));
            }
        }
        Ok(())
    }
}

fn source_storage_access(qualifiers: &str) -> GalResult<AccessFlags> {
    let words = qualifiers.split_whitespace().collect::<Vec<_>>();
    let readonly = words.contains(&"readonly");
    let writeonly = words.contains(&"writeonly");
    if readonly && writeonly {
        return Err(GalError::invalid_argument(
            "terrain source storage image cannot be both readonly and writeonly",
        ));
    }
    Ok(if readonly {
        AccessFlags::READ
    } else if writeonly {
        AccessFlags::WRITE
    } else {
        AccessFlags(AccessFlags::READ.0 | AccessFlags::WRITE.0)
    })
}

/// Exact semantic values for the two fixed legacy texture coordinates used by
/// the lowered terrain source. The first transforms atlas UVs; the second
/// transforms packed lightmap coordinates. Neither is a native uniform
/// location, renderer object, or a license to borrow shader-pack state.
#[derive(Clone, Debug, PartialEq)]
pub struct TerrainSourceTextureTransforms {
    pub atlas_texture_matrix: [f32; 16],
    pub lightmap_texture_matrix: [f32; 16],
}

impl TerrainSourceTextureTransforms {
    /// Canonical Minecraft terrain texture transforms, expressed as owned
    /// semantic values. Atlas coordinates are already normalized atlas UVs;
    /// lightmap coordinates are the original UV2 integer values and require
    /// the vanilla 1/256 scale plus 1/32 texel-center offset. This matches
    /// the documented `LightTexture` replacement transform, without reading
    /// an Iris uniform, texture unit, or mutable GL matrix.
    pub fn canonical_minecraft_terrain() -> Self {
        Self {
            atlas_texture_matrix: [
                1.0, 0.0, 0.0, 0.0, // column 0
                0.0, 1.0, 0.0, 0.0, // column 1
                0.0, 0.0, 1.0, 0.0, // column 2
                0.0, 0.0, 0.0, 1.0, // column 3
            ],
            lightmap_texture_matrix: [
                1.0 / 256.0,
                0.0,
                0.0,
                0.0,
                0.0,
                1.0 / 256.0,
                0.0,
                0.0,
                0.0,
                0.0,
                1.0 / 256.0,
                0.0,
                1.0 / 32.0,
                1.0 / 32.0,
                1.0 / 32.0,
                1.0,
            ],
        }
    }

    pub fn validate(&self) -> GalResult<()> {
        for (name, matrix) in [
            ("atlas", &self.atlas_texture_matrix),
            ("lightmap", &self.lightmap_texture_matrix),
        ] {
            if matrix.iter().any(|value| !value.is_finite()) {
                return Err(GalError::invalid_argument(format!(
                    "terrain source {name} texture matrix contains a non-finite value"
                )));
            }
        }
        Ok(())
    }
}

fn resource_binding_descriptor(
    binding: TerrainSourceFixedBinding,
    dynamic: bool,
) -> ResourceBindingDesc {
    ResourceBindingDesc {
        binding: binding.binding,
        kind: match binding.kind {
            TerrainSourceBindingKind::StorageBuffer => ResourceBindingKind::StorageBuffer,
            TerrainSourceBindingKind::UniformBuffer => ResourceBindingKind::UniformBuffer,
        },
        stages: PipelineStageFlags::DRAW,
        array_count: 1,
        optional: false,
        dynamic_offset_count: u32::from(dynamic),
    }
}

fn source_pack_resource_layout(
    label: String,
    opaque_resource_bindings: &TerrainSourceOpaqueResourceBindingPlan,
) -> GalResult<ResourceLayoutDesc> {
    let mut bindings = Vec::with_capacity(opaque_resource_bindings.bindings().len());
    for source_binding in opaque_resource_bindings.bindings() {
        let kind = match source_binding.kind() {
            TerrainSourceOpaqueResourceKind::CombinedTextureSampler => {
                ResourceBindingKind::CombinedTextureSampler
            }
            TerrainSourceOpaqueResourceKind::StorageImage => ResourceBindingKind::StorageTexture,
        };
        bindings.push(ResourceBindingDesc {
            binding: source_binding.binding(),
            kind,
            stages: PipelineStageFlags::DRAW,
            array_count: 1,
            optional: false,
            dynamic_offset_count: 0,
        });
    }
    bindings.sort_by_key(|binding| binding.binding);
    validate_unique_layout_bindings(&label, &bindings)?;
    Ok(ResourceLayoutDesc { label, bindings })
}

fn validate_source_scalar_uniform_block(
    scalar_uniforms: Option<TerrainSourceFixedBinding>,
    scalar_uniform_bytes: u32,
    scalar_uniform_fields: &[TerrainSourceUniformField],
    expected_binding: TerrainSourceFixedBinding,
    source_label: &str,
) -> GalResult<()> {
    match (scalar_uniforms, scalar_uniform_fields.is_empty()) {
        (None, true) if scalar_uniform_bytes == 0 => return Ok(()),
        (Some(binding), false) if binding == expected_binding => {}
        (None, false) => {
            return Err(GalError::invalid_argument(format!(
                "{source_label} scalar fields require fixed set 0 binding {}",
                expected_binding.binding
            )));
        }
        (Some(_), true) => {
            return Err(GalError::invalid_argument(format!(
                "{source_label} scalar binding is present without scalar fields"
            )));
        }
        (Some(_), false) => {
            return Err(GalError::invalid_argument(format!(
                "{source_label} scalar block must use fixed set 0 binding {} uniform-buffer ABI",
                expected_binding.binding
            )));
        }
        (None, true) => {
            return Err(GalError::invalid_argument(format!(
                "{source_label} empty scalar block has non-zero byte size"
            )));
        }
    }

    let mut previous_end = 0_u32;
    let mut previous_name = "";
    for field in scalar_uniform_fields {
        if field.name() <= previous_name {
            return Err(GalError::invalid_argument(format!(
                "{source_label} scalar fields must be strictly name-sorted"
            )));
        }
        if field.offset() < previous_end || field.offset() % 4 != 0 {
            return Err(GalError::invalid_argument(format!(
                "{source_label} scalar field '{}' has overlapping or unaligned std140 offset {}",
                field.name(),
                field.offset()
            )));
        }
        let end = field.offset().checked_add(field.size()).ok_or_else(|| {
            GalError::invalid_argument(format!("{source_label} scalar field range overflows u32"))
        })?;
        if field.array_length() == 1 && field.array_stride() != 0 {
            return Err(GalError::invalid_argument(format!(
                "{source_label} scalar field '{}' is not an array but has an array stride",
                field.name()
            )));
        }
        if field.array_length() > 1 && (field.array_stride() == 0 || field.array_stride() % 16 != 0)
        {
            return Err(GalError::invalid_argument(format!(
                "{source_label} scalar array '{}' has invalid std140 stride {}",
                field.name(),
                field.array_stride()
            )));
        }
        previous_end = end;
        previous_name = field.name();
    }
    if scalar_uniform_bytes == 0
        || scalar_uniform_bytes % 16 != 0
        || previous_end > scalar_uniform_bytes
    {
        return Err(GalError::invalid_argument(format!(
            "{source_label} scalar block size is not a valid std140 envelope"
        )));
    }
    Ok(())
}

fn validate_unique_layout_bindings(label: &str, bindings: &[ResourceBindingDesc]) -> GalResult<()> {
    if bindings
        .windows(2)
        .any(|pair| pair[0].binding == pair[1].binding)
    {
        return Err(GalError::invalid_argument(format!(
            "{label} contains duplicate binding numbers"
        )));
    }
    Ok(())
}

/// Backend-neutral storage kind required by one fixed source-lowering
/// binding. Resource-layout construction maps these semantic requirements to
/// ordinary GAL bindings in a later, explicit pipeline slice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerrainSourceBindingKind {
    StorageBuffer,
    UniformBuffer,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerrainSourceFixedBinding {
    pub set: u32,
    pub binding: u32,
    pub kind: TerrainSourceBindingKind,
}

/// One fixed `vec4` lane of the source-lowering terrain vertex stream. These
/// names match Rust-owned lowered GLSL fields, not legacy Java attributes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerrainSourceVertexField {
    pub name: &'static str,
    pub offset: u32,
    pub component_count: u32,
}

/// Source-derived requirements common to both lowered terrain stages. These
/// values match the lowered GLSL preamble and make its future GAL layout
/// explicit without allocating buffers or deciding routing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerrainSourceExecutionInterface {
    pub vertex_stream: TerrainSourceFixedBinding,
    pub vertex_stride: u32,
    pub vertex_fields: [TerrainSourceVertexField; 8],
    pub legacy_transforms: TerrainSourceFixedBinding,
    pub scalar_uniforms: Option<TerrainSourceFixedBinding>,
    pub instance_stream: TerrainSourceFixedBinding,
    pub instance_stride: u32,
    pub legacy_transform_bytes: u32,
    pub scalar_uniform_bytes: u32,
    pub scalar_uniform_fields: Vec<TerrainSourceUniformField>,
}

impl TerrainSourceExecutionInterface {
    const VERTEX_STREAM: TerrainSourceFixedBinding = TerrainSourceFixedBinding {
        set: 0,
        binding: 0,
        kind: TerrainSourceBindingKind::StorageBuffer,
    };
    const LEGACY_TRANSFORMS: TerrainSourceFixedBinding = TerrainSourceFixedBinding {
        set: 0,
        binding: 1,
        kind: TerrainSourceBindingKind::UniformBuffer,
    };
    const SCALAR_UNIFORMS: TerrainSourceFixedBinding = TerrainSourceFixedBinding {
        set: 0,
        binding: 2,
        kind: TerrainSourceBindingKind::UniformBuffer,
    };
    const INSTANCE_STREAM: TerrainSourceFixedBinding = TerrainSourceFixedBinding {
        set: 0,
        binding: 3,
        kind: TerrainSourceBindingKind::StorageBuffer,
    };

    fn from_uniform_contract(contract: &super::lowering::TerrainSourceUniformContract) -> Self {
        Self {
            vertex_stream: Self::VERTEX_STREAM,
            vertex_stride: TERRAIN_SOURCE_VERTEX_BYTES as u32,
            vertex_fields: [
                TerrainSourceVertexField {
                    name: "position",
                    offset: 0,
                    component_count: 4,
                },
                TerrainSourceVertexField {
                    name: "color",
                    offset: 16,
                    component_count: 4,
                },
                TerrainSourceVertexField {
                    name: "normal_light",
                    offset: 32,
                    component_count: 4,
                },
                TerrainSourceVertexField {
                    name: "atlas_uv_lightmap",
                    offset: 48,
                    component_count: 4,
                },
                TerrainSourceVertexField {
                    name: "entity",
                    offset: 64,
                    component_count: 4,
                },
                TerrainSourceVertexField {
                    name: "mid_tex_coord",
                    offset: 80,
                    component_count: 4,
                },
                TerrainSourceVertexField {
                    name: "tangent",
                    offset: 96,
                    component_count: 4,
                },
                TerrainSourceVertexField {
                    name: "mid_block",
                    offset: 112,
                    component_count: 4,
                },
            ],
            legacy_transforms: Self::LEGACY_TRANSFORMS,
            scalar_uniforms: (!contract.fields().is_empty()).then_some(Self::SCALAR_UNIFORMS),
            instance_stream: Self::INSTANCE_STREAM,
            instance_stride: TERRAIN_SOURCE_INSTANCE_BYTES as u32,
            // Two std140 mat4 texture transforms.
            legacy_transform_bytes: 2 * 16 * std::mem::size_of::<f32>() as u32,
            scalar_uniform_bytes: contract.std140_size(),
            scalar_uniform_fields: contract.fields().to_vec(),
        }
    }

    fn from_lowered_pair(lowered: &LoweredTerrainSourcePair) -> Self {
        Self::from_uniform_contract(lowered.uniform_contract())
    }

    fn from_lowered_entity_pair(lowered: &LoweredEntitySourcePair) -> Self {
        Self::from_uniform_contract(lowered.uniform_contract())
    }

    fn from_lowered_hand_pair(lowered: &LoweredHandSourcePair) -> Self {
        Self::from_uniform_contract(lowered.uniform_contract())
    }

    fn from_lowered_translucent_pair(lowered: &LoweredTranslucentTerrainSourcePair) -> Self {
        Self::from_uniform_contract(lowered.uniform_contract())
    }

    fn from_lowered_shadow_pair(lowered: &LoweredShadowSourcePair) -> Self {
        Self::from_uniform_contract(lowered.uniform_contract())
    }

    /// Rejects an internally inconsistent prepared-source interface before a
    /// later runtime slice can turn it into GAL layouts and uploads. The
    /// source lowerer owns this ABI; callers must not be able to reinterpret
    /// it as a legacy renderer vertex format or arbitrary binding scheme.
    pub fn validate(&self) -> GalResult<()> {
        const EXPECTED_FIELDS: [(&str, u32); 8] = [
            ("position", 0),
            ("color", 16),
            ("normal_light", 32),
            ("atlas_uv_lightmap", 48),
            ("entity", 64),
            ("mid_tex_coord", 80),
            ("tangent", 96),
            ("mid_block", 112),
        ];

        if self.vertex_stream != Self::VERTEX_STREAM {
            return Err(GalError::invalid_argument(
                "terrain source vertex stream must use fixed set 0 binding 0 storage-buffer ABI",
            ));
        }
        if self.vertex_stride != TERRAIN_SOURCE_VERTEX_BYTES as u32 {
            return Err(GalError::invalid_argument(format!(
                "terrain source vertex stride {} does not match the fixed {}-byte ABI",
                self.vertex_stride, TERRAIN_SOURCE_VERTEX_BYTES
            )));
        }
        for (field, (expected_name, expected_offset)) in
            self.vertex_fields.iter().zip(EXPECTED_FIELDS)
        {
            if field.name != expected_name
                || field.offset != expected_offset
                || field.component_count != 4
            {
                return Err(GalError::invalid_argument(format!(
                    "terrain source vertex field '{}' is not the fixed {} vec4 lane at offset {}",
                    field.name, expected_name, expected_offset
                )));
            }
        }
        if self.legacy_transforms != Self::LEGACY_TRANSFORMS || self.legacy_transform_bytes != 128 {
            return Err(GalError::invalid_argument(
                "terrain source legacy transforms must use fixed set 0 binding 1 with two std140 mat4 values",
            ));
        }
        if self.instance_stream != Self::INSTANCE_STREAM
            || self.instance_stride != TERRAIN_SOURCE_INSTANCE_BYTES as u32
        {
            return Err(GalError::invalid_argument(format!(
                "terrain source instance stream must use fixed set 0 binding 3 with {}-byte transform/color records",
                TERRAIN_SOURCE_INSTANCE_BYTES
            )));
        }

        match (self.scalar_uniforms, self.scalar_uniform_fields.is_empty()) {
            (None, true) if self.scalar_uniform_bytes == 0 => return Ok(()),
            (Some(binding), false) if binding == Self::SCALAR_UNIFORMS => {}
            (None, false) => {
                return Err(GalError::invalid_argument(
                    "terrain source scalar fields require fixed set 0 binding 2",
                ));
            }
            (Some(_), true) => {
                return Err(GalError::invalid_argument(
                    "terrain source scalar binding is present without scalar fields",
                ));
            }
            (Some(_), false) => {
                return Err(GalError::invalid_argument(
                    "terrain source scalar block must use fixed set 0 binding 2 uniform-buffer ABI",
                ));
            }
            (None, true) => {
                return Err(GalError::invalid_argument(
                    "empty terrain source scalar block has non-zero byte size",
                ));
            }
        }

        let mut previous_end = 0_u32;
        let mut previous_name = "";
        for field in &self.scalar_uniform_fields {
            if field.name() <= previous_name {
                return Err(GalError::invalid_argument(
                    "terrain source scalar fields must be strictly name-sorted",
                ));
            }
            if field.offset() < previous_end || field.offset() % 4 != 0 {
                return Err(GalError::invalid_argument(format!(
                    "terrain source scalar field '{}' has overlapping or unaligned std140 offset {}",
                    field.name(),
                    field.offset()
                )));
            }
            let end = field.offset().checked_add(field.size()).ok_or_else(|| {
                GalError::invalid_argument("terrain source scalar field range overflows u32")
            })?;
            if field.array_length() == 1 && field.array_stride() != 0 {
                return Err(GalError::invalid_argument(format!(
                    "terrain source scalar field '{}' is not an array but has an array stride",
                    field.name()
                )));
            }
            if field.array_length() > 1
                && (field.array_stride() == 0 || field.array_stride() % 16 != 0)
            {
                return Err(GalError::invalid_argument(format!(
                    "terrain source scalar array '{}' has invalid std140 stride {}",
                    field.name(),
                    field.array_stride()
                )));
            }
            previous_end = end;
            previous_name = field.name();
        }
        if self.scalar_uniform_bytes == 0
            || self.scalar_uniform_bytes % 16 != 0
            || previous_end > self.scalar_uniform_bytes
        {
            return Err(GalError::invalid_argument(
                "terrain source scalar block size is not a valid std140 envelope",
            ));
        }
        Ok(())
    }
}

impl TexturedMaterialSourceExecutionInterface {
    const VERTEX_STREAM: TerrainSourceFixedBinding = TerrainSourceFixedBinding {
        set: 0,
        binding: 0,
        kind: TerrainSourceBindingKind::StorageBuffer,
    };
    const LEGACY_TRANSFORMS: TerrainSourceFixedBinding = TerrainSourceFixedBinding {
        set: 0,
        binding: 1,
        kind: TerrainSourceBindingKind::UniformBuffer,
    };
    const SCALAR_UNIFORMS: TerrainSourceFixedBinding = TerrainSourceFixedBinding {
        set: 0,
        binding: 2,
        kind: TerrainSourceBindingKind::UniformBuffer,
    };

    fn from_uniform_contract(contract: &super::lowering::TerrainSourceUniformContract) -> Self {
        Self {
            vertex_stream: Self::VERTEX_STREAM,
            vertex_stride: TEXTURED_MATERIAL_SOURCE_VERTEX_BYTES as u32,
            vertex_fields: [
                TerrainSourceVertexField {
                    name: "position",
                    offset: 0,
                    component_count: 4,
                },
                TerrainSourceVertexField {
                    name: "color",
                    offset: 16,
                    component_count: 4,
                },
                TerrainSourceVertexField {
                    name: "normal_light",
                    offset: 32,
                    component_count: 4,
                },
                TerrainSourceVertexField {
                    name: "texture_uv_lightmap",
                    offset: 48,
                    component_count: 4,
                },
            ],
            legacy_transforms: Self::LEGACY_TRANSFORMS,
            scalar_uniforms: (!contract.fields().is_empty()).then_some(Self::SCALAR_UNIFORMS),
            legacy_transform_bytes: 2 * 16 * std::mem::size_of::<f32>() as u32,
            scalar_uniform_bytes: contract.std140_size(),
            scalar_uniform_fields: contract.fields().to_vec(),
        }
    }

    fn from_lowered_pair(lowered: &LoweredTexturedMaterialSourcePair) -> Self {
        Self::from_uniform_contract(lowered.uniform_contract())
    }

    pub fn validate(&self) -> GalResult<()> {
        const EXPECTED_FIELDS: [(&str, u32); 4] = [
            ("position", 0),
            ("color", 16),
            ("normal_light", 32),
            ("texture_uv_lightmap", 48),
        ];
        if self.vertex_stream != Self::VERTEX_STREAM
            || self.vertex_stride != TEXTURED_MATERIAL_SOURCE_VERTEX_BYTES as u32
        {
            return Err(GalError::invalid_argument(
                "textured material source must use its fixed set 0 binding 0 64-byte storage stream",
            ));
        }
        for (field, (name, offset)) in self.vertex_fields.iter().zip(EXPECTED_FIELDS) {
            if field.name != name || field.offset != offset || field.component_count != 4 {
                return Err(GalError::invalid_argument(format!(
                    "textured material source field '{}' is not the fixed {} vec4 lane at offset {}",
                    field.name, name, offset
                )));
            }
        }
        if self.legacy_transforms != Self::LEGACY_TRANSFORMS || self.legacy_transform_bytes != 128 {
            return Err(GalError::invalid_argument(
                "textured material source transforms must use fixed set 0 binding 1 with two std140 mat4 values",
            ));
        }
        validate_source_scalar_uniform_block(
            self.scalar_uniforms,
            self.scalar_uniform_bytes,
            &self.scalar_uniform_fields,
            Self::SCALAR_UNIFORMS,
            "textured material source",
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerrainProgramResource {
    ColoredVoxelLightVolume,
}

impl TerrainMaterialProgram {
    /// Converts a Rust-owned built-in terrain program into explicit GAL
    /// shader modules. This is shared by ordinary mesh materials and DH LOD
    /// materials so neither frontend has to reproduce backend dialect
    /// selection or module descriptions.
    pub fn shader_module_descriptors(&self, api: BackendApi) -> [ShaderModuleDesc; 2] {
        [
            self.vertex.shader_module_descriptor(api),
            self.fragment.shader_module_descriptor(api),
        ]
    }

    pub fn requires(&self, resource: TerrainProgramResource) -> bool {
        self.required_resources.contains(&resource)
    }
}

/// Forms an owned source-program preparation artifact only after the pair has
/// completed backend-neutral dialect lowering and its semantic sampler plan
/// is proven to belong to the same lowered source. The result is deliberately
/// kept separate from executable terrain programs until its source vertex
/// stream, scalar uniforms, and semantic resource sets are all supplied.
pub fn prepare_lowered_terrain_source_program(
    contract: &TerrainPassContract,
    lowered: &LoweredTerrainSourcePair,
    opaque_resource_bindings: &TerrainSourceOpaqueResourceBindingPlan,
    kind: TerrainMaterialProgramKind,
) -> GalResult<LoweredTerrainSourceProgram> {
    let material_class = match kind {
        TerrainMaterialProgramKind::Opaque => TerrainMaterialClass::Opaque,
        TerrainMaterialProgramKind::Cutout => TerrainMaterialClass::Cutout,
        TerrainMaterialProgramKind::Translucent => {
            return Err(GalError::unsupported_feature(
                "lowered terrain source preparation supports only opaque and cutout materials",
            ));
        }
    };
    if !contract.material_classes.contains(&material_class) {
        return Err(GalError::unsupported_feature(format!(
            "terrain source contract does not admit {:?} material preparation",
            kind
        )));
    }
    lowered.require_backend_neutral_lowering()?;
    lowered.require_matching_opaque_resource_bindings(opaque_resource_bindings)?;
    let suffix = match kind {
        TerrainMaterialProgramKind::Opaque => "opaque",
        TerrainMaterialProgramKind::Cutout => "cutout",
        TerrainMaterialProgramKind::Translucent => unreachable!(),
    };
    let requires_colored_voxel_light = contract
        .required_resources
        .contains(&TerrainPassRequiredResource::ColoredVoxelLightVolume);
    let execution_interface = TerrainSourceExecutionInterface::from_lowered_pair(lowered);
    execution_interface.validate()?;
    let scalar_uniform_requirements =
        TerrainSourceUniformRequirements::from_contract(lowered.uniform_contract())?;
    scalar_uniform_requirements.require_fully_semantic()?;
    let terrain_outputs = contract.outputs.iter().copied().collect::<Vec<_>>();
    let terrain_output_color_slots = terrain_outputs
        .iter()
        .copied()
        .map(|output| {
            contract
                .output_color_slot(output)
                .map(|slot| (output, slot))
                .ok_or_else(|| {
                    GalError::invalid_argument(format!(
                        "selected terrain source contract has no shader-pack color slot for '{}'",
                        output.semantic_name()
                    ))
                })
        })
        .collect::<GalResult<Vec<_>>>()?;
    let program = LoweredTerrainSourceProgram {
        identity: ProgramIdentity::new(format!(
            "vulkanic:shader-pack/{}/terrain_{}_source_gen{}",
            contract.pack_name.to_ascii_lowercase(),
            suffix,
            contract.generation
        )),
        material_kind: Some(kind),
        shader_pack_generation: contract.generation,
        vertex: ShaderStageSource {
            stage: ShaderStageKind::Vertex,
            label: format!("{}:lowered-vertex", lowered.vertex().entry_path()),
            source: lowered.vertex().source().to_string(),
            entry_point: "main".to_string(),
        },
        fragment: ShaderStageSource {
            stage: ShaderStageKind::Fragment,
            label: format!("{}:lowered-fragment", lowered.fragment().entry_path()),
            source: lowered.fragment().source().to_string(),
            entry_point: "main".to_string(),
        },
        execution_interface,
        scalar_uniform_requirements,
        opaque_resource_bindings: opaque_resource_bindings.clone(),
        required_resources: if requires_colored_voxel_light {
            vec![TerrainProgramResource::ColoredVoxelLightVolume]
        } else {
            Vec::new()
        },
        terrain_outputs: Some(terrain_outputs),
        terrain_output_color_slots: Some(terrain_output_color_slots),
        translucent_raster_state: None,
    };
    program.execution_resource_layouts()?;
    Ok(program)
}

/// Prepares one selected ordinary-entity source program after its source
/// contract, Rust-owned entity-ID generation, lowered shader pair, and local
/// material resource plan agree. It intentionally stops before pipeline,
/// target, geometry stream, and draw construction; no compatibility route can
/// mistake this preparation artifact for executed entity work.
pub fn prepare_lowered_entity_source_program(
    contract: &EntityPassContract,
    lowered: &LoweredEntitySourcePair,
    opaque_resource_bindings: &TerrainSourceOpaqueResourceBindingPlan,
) -> GalResult<LoweredEntitySourceProgram> {
    if contract.generation == 0 || contract.entity_id_generation() == 0 {
        return Err(GalError::invalid_argument(
            "entity source preparation requires non-zero shader and entity-id generations",
        ));
    }
    if contract.outputs.is_empty() || contract.outputs.len() != contract.output_color_slots.len() {
        return Err(GalError::invalid_argument(
            "entity source outputs and shader-pack slots must be non-empty and aligned",
        ));
    }
    lowered.require_backend_neutral_lowering()?;
    lowered.require_matching_opaque_resource_bindings(opaque_resource_bindings)?;
    let execution_interface = TerrainSourceExecutionInterface::from_lowered_entity_pair(lowered);
    execution_interface.validate()?;
    let scalar_uniform_requirements =
        TerrainSourceUniformRequirements::from_contract(lowered.uniform_contract())?;
    scalar_uniform_requirements.require_fully_semantic()?;
    let named_output_color_slots = contract
        .outputs
        .iter()
        .copied()
        .zip(contract.output_color_slots.iter().copied())
        .map(|(output, slot)| (entity_output_to_terrain_output(output), slot))
        .collect::<Vec<_>>();
    let program = LoweredEntitySourceProgram {
        identity: ProgramIdentity::new(format!(
            "vulkanic:shader-pack/{}/entity_source_gen{}",
            contract.pack_name.to_ascii_lowercase(),
            contract.generation
        )),
        shader_pack_generation: contract.generation,
        entity_id_generation: contract.entity_id_generation(),
        entity_contract: contract.clone(),
        vertex: ShaderStageSource {
            stage: ShaderStageKind::Vertex,
            label: format!("{}:lowered-vertex", lowered.vertex().entry_path()),
            source: lowered.vertex().source().to_string(),
            entry_point: "main".to_string(),
        },
        fragment: ShaderStageSource {
            stage: ShaderStageKind::Fragment,
            label: format!("{}:lowered-fragment", lowered.fragment().entry_path()),
            source: lowered.fragment().source().to_string(),
            entry_point: "main".to_string(),
        },
        execution_interface,
        scalar_uniform_requirements,
        opaque_resource_bindings: opaque_resource_bindings.clone(),
        named_output_color_slots,
    };
    program.execution_resource_layouts()?;
    Ok(program)
}

fn entity_output_to_terrain_output(output: EntitySourceOutput) -> TerrainPassOutput {
    match output {
        EntitySourceOutput::LitColor => TerrainPassOutput::LitTerrainColor,
        EntitySourceOutput::MaterialAuxiliary => TerrainPassOutput::MaterialAuxiliary,
        EntitySourceOutput::ViewSpaceNormal => TerrainPassOutput::ViewSpaceNormal,
    }
}

/// Prepares a selected first-person source program while keeping it separate
/// from the entity writer. Preparation proves that source code, semantic
/// resources, and the fixed owned stream ABI agree; it intentionally does
/// not allocate a pipeline, select a route, or create the hand pass.
pub fn prepare_lowered_hand_source_program(
    contract: &HandPassContract,
    lowered: &LoweredHandSourcePair,
    opaque_resource_bindings: &TerrainSourceOpaqueResourceBindingPlan,
) -> GalResult<LoweredHandSourceProgram> {
    if contract.generation == 0 {
        return Err(GalError::invalid_argument(
            "hand source preparation requires a non-zero shader-pack generation",
        ));
    }
    if contract.outputs.is_empty() || contract.outputs.len() != contract.output_color_slots.len() {
        return Err(GalError::invalid_argument(
            "hand source outputs and shader-pack slots must be non-empty and aligned",
        ));
    }
    lowered.require_backend_neutral_lowering()?;
    lowered.require_matching_opaque_resource_bindings(opaque_resource_bindings)?;
    let execution_interface = TerrainSourceExecutionInterface::from_lowered_hand_pair(lowered);
    execution_interface.validate()?;
    let scalar_uniform_requirements =
        TerrainSourceUniformRequirements::from_contract(lowered.uniform_contract())?;
    scalar_uniform_requirements.require_fully_semantic()?;
    let named_output_color_slots = contract
        .outputs
        .iter()
        .copied()
        .zip(contract.output_color_slots.iter().copied())
        .map(|(output, slot)| (hand_output_to_terrain_output(output), slot))
        .collect::<Vec<_>>();
    let program = LoweredHandSourceProgram {
        identity: ProgramIdentity::new(format!(
            "vulkanic:shader-pack/{}/hand_source_gen{}",
            contract.pack_name.to_ascii_lowercase(),
            contract.generation
        )),
        shader_pack_generation: contract.generation,
        hand_contract: contract.clone(),
        vertex: ShaderStageSource {
            stage: ShaderStageKind::Vertex,
            label: format!("{}:lowered-vertex", lowered.vertex().entry_path()),
            source: lowered.vertex().source().to_string(),
            entry_point: "main".to_string(),
        },
        fragment: ShaderStageSource {
            stage: ShaderStageKind::Fragment,
            label: format!("{}:lowered-fragment", lowered.fragment().entry_path()),
            source: lowered.fragment().source().to_string(),
            entry_point: "main".to_string(),
        },
        execution_interface,
        scalar_uniform_requirements,
        opaque_resource_bindings: opaque_resource_bindings.clone(),
        named_output_color_slots,
    };
    program.execution_resource_layouts()?;
    Ok(program)
}

fn hand_output_to_terrain_output(output: HandSourceOutput) -> TerrainPassOutput {
    match output {
        HandSourceOutput::LitColor => TerrainPassOutput::LitTerrainColor,
        HandSourceOutput::MaterialAuxiliary => TerrainPassOutput::MaterialAuxiliary,
        HandSourceOutput::ViewSpaceNormal => TerrainPassOutput::ViewSpaceNormal,
    }
}

/// Prepares one source-derived `gbuffers_textured` program only after its
/// shader-pack generation, source lowering, scalar uniforms, and pass-local
/// semantic resource plan agree. This intentionally stops before pipeline and
/// stream construction: the Rust-owned material stream and named-target pass
/// are constructed by the world frontend rather than reusing the final-output
/// overlay path.
pub fn prepare_lowered_textured_material_source_program(
    contract: &TexturedMaterialPassContract,
    lowered: &LoweredTexturedMaterialSourcePair,
    opaque_resource_bindings: &TerrainSourceOpaqueResourceBindingPlan,
) -> GalResult<LoweredTexturedMaterialSourceProgram> {
    if contract.generation == 0 {
        return Err(GalError::invalid_argument(
            "textured material source preparation requires a non-zero shader-pack generation",
        ));
    }
    if contract.outputs.len() != contract.output_color_slots.len() || contract.outputs.is_empty() {
        return Err(GalError::invalid_argument(
            "textured material source outputs and shader-pack slots must be non-empty and aligned",
        ));
    }
    lowered.require_backend_neutral_lowering()?;
    lowered.require_matching_opaque_resource_bindings(opaque_resource_bindings)?;
    let execution_interface = TexturedMaterialSourceExecutionInterface::from_lowered_pair(lowered);
    execution_interface.validate()?;
    let scalar_uniform_requirements =
        TerrainSourceUniformRequirements::from_contract(lowered.uniform_contract())?;
    scalar_uniform_requirements.require_fully_semantic()?;
    let named_output_color_slots = contract
        .outputs
        .iter()
        .copied()
        .zip(contract.output_color_slots.iter().copied())
        .map(|(output, slot)| (output.terrain_output(), slot))
        .collect::<Vec<_>>();
    if named_output_color_slots
        .iter()
        .map(|(output, _)| output)
        .collect::<BTreeSet<_>>()
        .len()
        != named_output_color_slots.len()
    {
        return Err(GalError::invalid_argument(
            "textured material source maps multiple outputs to one semantic named target",
        ));
    }
    let program = LoweredTexturedMaterialSourceProgram {
        identity: ProgramIdentity::new(format!(
            "vulkanic:shader-pack/{}/textured_material_source_gen{}",
            contract.pack_name.to_ascii_lowercase(),
            contract.generation
        )),
        shader_pack_generation: contract.generation,
        vertex: ShaderStageSource {
            stage: ShaderStageKind::Vertex,
            label: format!("{}:lowered-vertex", lowered.vertex().entry_path()),
            source: lowered.vertex().source().to_string(),
            entry_point: "main".to_string(),
        },
        fragment: ShaderStageSource {
            stage: ShaderStageKind::Fragment,
            label: format!("{}:lowered-fragment", lowered.fragment().entry_path()),
            source: lowered.fragment().source().to_string(),
            entry_point: "main".to_string(),
        },
        execution_interface,
        scalar_uniform_requirements,
        opaque_resource_bindings: opaque_resource_bindings.clone(),
        named_output_color_slots,
    };
    program.execution_resource_layouts()?;
    Ok(program)
}

/// Prepares the independently selected weather source. This deliberately
/// stops before target creation and draw recording, so preparation cannot
/// select a mixed Java/Rust weather route.
pub fn prepare_lowered_weather_source_program(
    contract: &WeatherPassContract,
    lowered: &LoweredWeatherSourcePair,
    opaque_resource_bindings: &TerrainSourceOpaqueResourceBindingPlan,
) -> GalResult<LoweredWeatherSourceProgram> {
    if contract.generation == 0 || contract.lit_color_output_slot != 0 {
        return Err(GalError::invalid_argument(
            "weather source preparation requires a non-zero generation and named lit-color slot zero",
        ));
    }
    lowered.require_backend_neutral_lowering()?;
    lowered.require_matching_opaque_resource_bindings(opaque_resource_bindings)?;
    let execution_interface =
        TexturedMaterialSourceExecutionInterface::from_uniform_contract(lowered.uniform_contract());
    execution_interface.validate()?;
    let scalar_uniform_requirements =
        TerrainSourceUniformRequirements::from_contract(lowered.uniform_contract())?;
    scalar_uniform_requirements.require_fully_semantic()?;
    let program = LoweredWeatherSourceProgram {
        identity: ProgramIdentity::new(format!(
            "vulkanic:shader-pack/{}/weather_source_gen{}",
            contract.pack_name.to_ascii_lowercase(),
            contract.generation
        )),
        shader_pack_generation: contract.generation,
        vertex: ShaderStageSource {
            stage: ShaderStageKind::Vertex,
            label: format!("{}:lowered-vertex", lowered.vertex().entry_path()),
            source: lowered.vertex().source().to_string(),
            entry_point: "main".to_string(),
        },
        fragment: ShaderStageSource {
            stage: ShaderStageKind::Fragment,
            label: format!("{}:lowered-fragment", lowered.fragment().entry_path()),
            source: lowered.fragment().source().to_string(),
            entry_point: "main".to_string(),
        },
        execution_interface,
        scalar_uniform_requirements,
        opaque_resource_bindings: opaque_resource_bindings.clone(),
        lit_color_output_slot: contract.lit_color_output_slot,
        alpha_discard_threshold_bits: contract.alpha_discard_threshold_bits,
        blend: contract.blend,
    };
    program.execution_resource_layouts()?;
    Ok(program)
}

/// Prepares the selected vanilla-cloud source without creating a target,
/// pipeline, or route. The dedicated Rust-owned cloud writer remains a later
/// transaction that must prove its resources and named targets explicitly.
pub fn prepare_lowered_cloud_source_program(
    contract: &CloudPassContract,
    lowered: &LoweredCloudSourcePair,
    opaque_resource_bindings: &TerrainSourceOpaqueResourceBindingPlan,
) -> GalResult<LoweredCloudSourceProgram> {
    if contract.generation == 0 || contract.outputs.is_empty() {
        return Err(GalError::invalid_argument(
            "cloud source preparation requires a non-zero generation and named outputs",
        ));
    }
    if contract.outputs.len() != 3 || contract.outputs.len() != contract.output_color_slots.len() {
        return Err(GalError::unsupported_feature(
            "cloud source preparation supports exactly lit, material, and translucency outputs",
        ));
    }
    lowered.require_backend_neutral_lowering()?;
    lowered.require_matching_opaque_resource_bindings(opaque_resource_bindings)?;
    let execution_interface =
        TexturedMaterialSourceExecutionInterface::from_uniform_contract(lowered.uniform_contract());
    execution_interface.validate()?;
    let scalar_uniform_requirements =
        TerrainSourceUniformRequirements::from_contract(lowered.uniform_contract())?;
    scalar_uniform_requirements.require_fully_semantic()?;
    let named_output_color_slots = contract
        .outputs
        .iter()
        .copied()
        .zip(contract.output_color_slots.iter().copied())
        .map(|(output, slot)| (output.terrain_output(), slot))
        .collect::<Vec<_>>();
    if named_output_color_slots
        .iter()
        .map(|(output, _)| output)
        .collect::<BTreeSet<_>>()
        .len()
        != named_output_color_slots.len()
    {
        return Err(GalError::invalid_argument(
            "cloud source maps multiple outputs to one semantic named target",
        ));
    }
    let program = LoweredCloudSourceProgram {
        identity: ProgramIdentity::new(format!(
            "vulkanic:shader-pack/{}/cloud_source_gen{}",
            contract.pack_name.to_ascii_lowercase(),
            contract.generation
        )),
        shader_pack_generation: contract.generation,
        vertex: ShaderStageSource {
            stage: ShaderStageKind::Vertex,
            label: format!("{}:lowered-vertex", lowered.vertex().entry_path()),
            source: lowered.vertex().source().to_string(),
            entry_point: "main".to_string(),
        },
        fragment: ShaderStageSource {
            stage: ShaderStageKind::Fragment,
            label: format!("{}:lowered-fragment", lowered.fragment().entry_path()),
            source: lowered.fragment().source().to_string(),
            entry_point: "main".to_string(),
        },
        execution_interface,
        scalar_uniform_requirements,
        opaque_resource_bindings: opaque_resource_bindings.clone(),
        blend: contract.blend,
        named_output_color_slots,
    };
    program.execution_resource_layouts()?;
    Ok(program)
}

/// Prepares the distinct translucent terrain source program without merging it
/// into the normal opaque/cutout G-buffer contract. This remains preparation
/// only: the caller must later provide an explicit target, depth/history, and
/// blend pass before it can execute.
pub fn prepare_lowered_translucent_terrain_source_program(
    contract: &TerrainPassContract,
    lowered: &LoweredTranslucentTerrainSourcePair,
    opaque_resource_bindings: &TerrainSourceOpaqueResourceBindingPlan,
) -> GalResult<LoweredTerrainSourceProgram> {
    if contract.pass_kind != super::terrain_contract::TerrainSourcePassKind::Translucent
        || !contract
            .material_classes
            .contains(&TerrainMaterialClass::Translucent)
    {
        return Err(GalError::unsupported_feature(
            "translucent source preparation requires a translucent terrain contract",
        ));
    }
    let translucent_raster_state = contract.translucent_raster_state.ok_or_else(|| {
        GalError::unsupported_feature(
            "selected translucent terrain source has no explicit alpha/blend raster contract",
        )
    })?;
    // Parsing a `gbuffers_water` stage is not enough to admit it. The source
    // contract must have modeled every feature it selected before a later
    // executor may even receive this preparation artifact.
    contract.require_selected_subset()?;
    lowered.require_backend_neutral_lowering()?;
    lowered.require_matching_opaque_resource_bindings(opaque_resource_bindings)?;
    let execution_interface =
        TerrainSourceExecutionInterface::from_lowered_translucent_pair(lowered);
    execution_interface.validate()?;
    let scalar_uniform_requirements =
        TerrainSourceUniformRequirements::from_contract(lowered.uniform_contract())?;
    scalar_uniform_requirements.require_fully_semantic()?;
    let terrain_outputs = contract.outputs.iter().copied().collect::<Vec<_>>();
    let terrain_output_color_slots = terrain_outputs
        .iter()
        .copied()
        .map(|output| {
            contract
                .output_color_slot(output)
                .map(|slot| (output, slot))
                .ok_or_else(|| {
                    GalError::invalid_argument(format!(
                    "selected translucent source contract has no shader-pack color slot for '{}'",
                    output.semantic_name()
                ))
                })
        })
        .collect::<GalResult<Vec<_>>>()?;
    let program = LoweredTerrainSourceProgram {
        identity: ProgramIdentity::new(format!(
            "vulkanic:shader-pack/{}/terrain_translucent_source_gen{}",
            contract.pack_name.to_ascii_lowercase(),
            contract.generation
        )),
        material_kind: Some(TerrainMaterialProgramKind::Translucent),
        shader_pack_generation: contract.generation,
        vertex: ShaderStageSource {
            stage: ShaderStageKind::Vertex,
            label: format!(
                "{}:lowered-translucent-vertex",
                lowered.vertex().entry_path()
            ),
            source: lowered.vertex().source().to_string(),
            entry_point: "main".to_string(),
        },
        fragment: ShaderStageSource {
            stage: ShaderStageKind::Fragment,
            label: format!(
                "{}:lowered-translucent-fragment",
                lowered.fragment().entry_path()
            ),
            source: lowered.fragment().source().to_string(),
            entry_point: "main".to_string(),
        },
        execution_interface,
        scalar_uniform_requirements,
        opaque_resource_bindings: opaque_resource_bindings.clone(),
        required_resources: Vec::new(),
        terrain_outputs: Some(terrain_outputs),
        terrain_output_color_slots: Some(terrain_output_color_slots),
        translucent_raster_state: Some(translucent_raster_state),
    };
    program.execution_resource_layouts()?;
    Ok(program)
}

/// Forms the owned preparation artifact for the selected Distant Horizons
/// source stage. Unlike near terrain, this requires the exact DH column-stream
/// ABI and exposes only the source's own named distant color output. It is
/// deliberately not a route selector or a compatibility path for Iris.
pub fn prepare_lowered_distant_horizons_source_program(
    contract: &DistantHorizonsPassContract,
    lowered: &LoweredDistantHorizonsSourcePair,
    opaque_resource_bindings: &TerrainSourceOpaqueResourceBindingPlan,
) -> GalResult<LoweredDistantHorizonsSourceProgram> {
    if contract.pack_name.trim().is_empty() || contract.generation == 0 {
        return Err(GalError::invalid_argument(
            "Distant Horizons source program requires a non-empty pack name and non-zero generation",
        ));
    }
    if lowered.fragment().outputs() != [DistantHorizonsFragmentOutput::LitColor] {
        return Err(GalError::unsupported_feature(
            "Distant Horizons source program requires exactly one named lit-color output",
        ));
    }
    match (contract.pass_kind, contract.translucent_blend) {
        (DistantHorizonsPassKind::Opaque, None)
        | (DistantHorizonsPassKind::Translucent, Some(TerrainTranslucentBlend::SourceAlphaOver)) => {
        }
        (DistantHorizonsPassKind::Opaque, Some(_)) => {
            return Err(GalError::invalid_argument(
                "opaque Distant Horizons source program must not carry translucent blend semantics",
            ));
        }
        (DistantHorizonsPassKind::Translucent, None) => {
            return Err(GalError::invalid_argument(
                "translucent Distant Horizons source program requires explicit source blend semantics",
            ));
        }
    }
    lowered.require_backend_neutral_lowering()?;
    lowered.require_matching_opaque_resource_bindings(opaque_resource_bindings)?;
    let execution_interface = DistantHorizonsSourceExecutionInterface::from_lowered_pair(lowered);
    execution_interface.validate()?;
    let scalar_uniform_requirements =
        TerrainSourceUniformRequirements::from_contract(lowered.uniform_contract())?;
    scalar_uniform_requirements.require_fully_semantic()?;
    let program = LoweredDistantHorizonsSourceProgram {
        identity: ProgramIdentity::new(format!(
            "vulkanic:shader-pack/{}/distant_horizons_{}_source_gen{}",
            contract.pack_name.to_ascii_lowercase(),
            match contract.pass_kind {
                DistantHorizonsPassKind::Opaque => "opaque",
                DistantHorizonsPassKind::Translucent => "translucent",
            },
            contract.generation
        )),
        shader_pack_generation: contract.generation,
        pass_kind: contract.pass_kind,
        translucent_blend: contract.translucent_blend,
        vertex: ShaderStageSource {
            stage: ShaderStageKind::Vertex,
            label: format!(
                "{}:lowered-distant-horizons-vertex",
                lowered.vertex().entry_path()
            ),
            source: lowered.vertex().source().to_string(),
            entry_point: "main".to_string(),
        },
        fragment: ShaderStageSource {
            stage: ShaderStageKind::Fragment,
            label: format!(
                "{}:lowered-distant-horizons-fragment",
                lowered.fragment().entry_path()
            ),
            source: lowered.fragment().source().to_string(),
            entry_point: "main".to_string(),
        },
        execution_interface,
        scalar_uniform_requirements,
        opaque_resource_bindings: opaque_resource_bindings.clone(),
        required_resources: if contract
            .required_resources
            .contains(&TerrainPassRequiredResource::ColoredVoxelLightVolume)
        {
            vec![TerrainProgramResource::ColoredVoxelLightVolume]
        } else {
            Vec::new()
        },
    };
    program.execution_resource_layouts()?;
    Ok(program)
}

/// Creates a preparation artifact for one source-defined fullscreen consumer.
/// The caller must later supply named output attachments, owned fullscreen
/// geometry, complete source uniforms, semantic sampler resources, and pass
/// ordering. This function performs none of those actions and cannot admit a
/// live route on its own.
pub fn prepare_lowered_fullscreen_source_program(
    pack_name: &str,
    shader_pack_generation: u64,
    stage_path: &str,
    lowered: &LoweredFullscreenSourcePair,
    opaque_resource_bindings: &TerrainSourceOpaqueResourceBindingPlan,
) -> GalResult<LoweredFullscreenSourceProgram> {
    if pack_name.trim().is_empty() || shader_pack_generation == 0 || stage_path.trim().is_empty() {
        return Err(GalError::invalid_argument(
            "fullscreen source program requires pack name, generation, and stage path",
        ));
    }
    lowered.require_backend_neutral_lowering()?;
    lowered.require_matching_opaque_resource_bindings(opaque_resource_bindings)?;
    let execution_interface = FullscreenSourceExecutionInterface::from_lowered_pair(lowered);
    execution_interface.validate()?;
    let scalar_uniform_requirements =
        TerrainSourceUniformRequirements::from_contract(lowered.uniform_contract())?;
    scalar_uniform_requirements.require_fully_semantic()?;
    let outputs = lowered.fragment().outputs().to_vec();
    if outputs.is_empty() {
        return Err(GalError::invalid_argument(
            "fullscreen source program requires at least one named semantic output",
        ));
    }
    let mut feedback_requirements = Vec::new();
    for output in &outputs {
        if !matches!(output.role(), TerrainSourceResourceRole::ShaderPackColor(_)) {
            return Err(GalError::invalid_argument(format!(
                "fullscreen source output '{}' is not a semantic shader-pack color resource",
                output.semantic_name()
            )));
        }
        for binding in opaque_resource_bindings.bindings() {
            if binding.kind() == TerrainSourceOpaqueResourceKind::CombinedTextureSampler
                && binding.role() == output.role()
            {
                feedback_requirements.push(FullscreenSourceFeedbackRequirement {
                    role: output.role(),
                    sampled_binding: binding.binding(),
                    output_location: output.source_location(),
                });
            }
        }
    }
    feedback_requirements.sort_by(|left, right| {
        left.output_location
            .cmp(&right.output_location)
            .then_with(|| left.sampled_binding.cmp(&right.sampled_binding))
            .then_with(|| left.role.cmp(&right.role))
    });
    feedback_requirements.dedup();
    let mipmap_requirements = derive_fullscreen_mipmap_requirements(
        lowered.fragment().source(),
        opaque_resource_bindings,
    )?;
    let program = LoweredFullscreenSourceProgram {
        identity: ProgramIdentity::new(format!(
            "vulkanic:shader-pack/{}/{}-source-gen{}",
            pack_name.to_ascii_lowercase(),
            stage_path
                .trim_end_matches(".fsh")
                .trim_end_matches(".glsl")
                .replace('/', "-"),
            shader_pack_generation
        )),
        source_stage_path: stage_path.to_string(),
        shader_pack_generation,
        raster_primitive: lowered.raster_primitive(),
        vertex: ShaderStageSource {
            stage: ShaderStageKind::Vertex,
            label: format!(
                "{}:lowered-fullscreen-vertex",
                lowered.vertex().entry_path()
            ),
            source: lowered.vertex().source().to_string(),
            entry_point: "main".to_string(),
        },
        fragment: ShaderStageSource {
            stage: ShaderStageKind::Fragment,
            label: format!(
                "{}:lowered-fullscreen-fragment",
                lowered.fragment().entry_path()
            ),
            source: lowered.fragment().source().to_string(),
            entry_point: "main".to_string(),
        },
        execution_interface,
        scalar_uniform_requirements,
        opaque_resource_bindings: opaque_resource_bindings.clone(),
        outputs,
        feedback_requirements,
        mipmap_requirements,
    };
    program.execution_resource_layouts()?;
    Ok(program)
}

fn derive_fullscreen_mipmap_requirements(
    fragment_source: &str,
    bindings: &TerrainSourceOpaqueResourceBindingPlan,
) -> GalResult<Vec<FullscreenSourceMipmapRequirement>> {
    let mut requested = std::collections::BTreeMap::<String, bool>::new();
    for line in fragment_source.lines() {
        let Some(fragment) = line.find("const bool ").map(|index| &line[index..]) else {
            continue;
        };
        let Some((name, value)) = fragment
            .strip_prefix("const bool ")
            .and_then(|rest| rest.split_once('='))
        else {
            continue;
        };
        let name = name.trim();
        let Some(resource_name) = name.strip_suffix("MipmapEnabled") else {
            continue;
        };
        let value = value
            .split_once(';')
            .map(|(value, _)| value)
            .unwrap_or(value)
            .trim();
        let enabled = match value {
            "true" => true,
            "false" => false,
            _ => {
                return Err(GalError::invalid_argument(format!(
                    "fullscreen source mip directive '{name}' must be true or false"
                )))
            }
        };
        requested.insert(resource_name.to_string(), enabled);
    }
    let mut requirements = Vec::new();
    for (resource_name, enabled) in requested {
        if !enabled {
            continue;
        }
        let binding = bindings
            .bindings()
            .iter()
            .find(|binding| binding.resource_name() == resource_name)
            .ok_or_else(|| GalError::unsupported_feature(format!(
                "fullscreen source requests mipmaps for '{resource_name}', which has no active semantic resource binding"
            )))?;
        if binding.kind() != TerrainSourceOpaqueResourceKind::CombinedTextureSampler
            || !matches!(
                binding.role(),
                TerrainSourceResourceRole::ShaderPackColor(_)
            )
        {
            return Err(GalError::unsupported_feature(format!(
                "fullscreen source requests mipmaps for '{resource_name}', which is not a semantic shader-pack color sampler"
            )));
        }
        requirements.push(FullscreenSourceMipmapRequirement {
            role: binding.role(),
            sampled_binding: binding.binding(),
        });
    }
    requirements.sort_by(|left, right| {
        left.sampled_binding
            .cmp(&right.sampled_binding)
            .then_with(|| left.role.cmp(&right.role))
    });
    requirements.dedup();
    Ok(requirements)
}

/// Forms an owned source-shadow program preparation artifact from one exact
/// scoped shadow pair. The program retains shadow-specific output names and
/// transform semantics; it is not a normal terrain material program and it
/// cannot select a route, allocate a shadow target, or issue a draw.
pub fn prepare_lowered_shadow_source_program(
    pack_name: &str,
    shader_pack_generation: u64,
    lowered: &LoweredShadowSourcePair,
    opaque_resource_bindings: &TerrainSourceOpaqueResourceBindingPlan,
) -> GalResult<LoweredTerrainSourceProgram> {
    if pack_name.trim().is_empty() || shader_pack_generation == 0 {
        return Err(GalError::invalid_argument(
            "source shadow program requires a non-empty pack name and non-zero generation",
        ));
    }
    lowered.require_backend_neutral_lowering()?;
    lowered.require_matching_opaque_resource_bindings(opaque_resource_bindings)?;
    let execution_interface = TerrainSourceExecutionInterface::from_lowered_shadow_pair(lowered);
    execution_interface.validate()?;
    let scalar_uniform_requirements =
        TerrainSourceUniformRequirements::from_contract(lowered.uniform_contract())?;
    scalar_uniform_requirements.require_fully_semantic()?;
    let program = LoweredTerrainSourceProgram {
        identity: ProgramIdentity::new(format!(
            "vulkanic:shader-pack/{}/shadow_source_gen{}",
            pack_name.to_ascii_lowercase(),
            shader_pack_generation
        )),
        material_kind: None,
        shader_pack_generation,
        vertex: ShaderStageSource {
            stage: ShaderStageKind::Vertex,
            label: format!("{}:lowered-shadow-vertex", lowered.vertex().entry_path()),
            source: lowered.vertex().source().to_string(),
            entry_point: "main".to_string(),
        },
        fragment: ShaderStageSource {
            stage: ShaderStageKind::Fragment,
            label: format!(
                "{}:lowered-shadow-fragment",
                lowered.fragment().entry_path()
            ),
            source: lowered.fragment().source().to_string(),
            entry_point: "main".to_string(),
        },
        execution_interface,
        scalar_uniform_requirements,
        opaque_resource_bindings: opaque_resource_bindings.clone(),
        required_resources: Vec::new(),
        terrain_outputs: None,
        terrain_output_color_slots: None,
        translucent_raster_state: None,
    };
    program.execution_resource_layouts()?;
    Ok(program)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompositeProgram {
    pub identity: ProgramIdentity,
    pub vertex: ShaderStageSource,
    pub fragment: ShaderStageSource,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerrainMaterialProgramKind {
    Opaque,
    Cutout,
    Translucent,
}

impl TerrainMaterialProgramKind {
    fn identity(self) -> ProgramIdentity {
        match self {
            Self::Opaque => ProgramIdentity::new("vulkanic:builtin/terrain_opaque_v1"),
            Self::Cutout => ProgramIdentity::new("vulkanic:builtin/terrain_cutout_v1"),
            Self::Translucent => ProgramIdentity::new("vulkanic:builtin/terrain_translucent_v1"),
        }
    }

    fn label_suffix(self) -> &'static str {
        match self {
            Self::Opaque => "opaque",
            Self::Cutout => "cutout",
            Self::Translucent => "translucent",
        }
    }
}

pub fn minimal_terrain_solid_program() -> TerrainMaterialProgram {
    minimal_terrain_material_program(TerrainMaterialProgramKind::Opaque)
}

pub fn minimal_terrain_cutout_program() -> TerrainMaterialProgram {
    minimal_terrain_material_program(TerrainMaterialProgramKind::Cutout)
}

/// Rust-owned solid-color entity-outline mask program. It deliberately uses
/// the ordinary mesh/instance storage ABI so the eventual mask writer can
/// share copied geometry without importing a Java renderer or native handle.
pub fn minimal_entity_outline_program() -> TerrainMaterialProgram {
    TerrainMaterialProgram {
        identity: ProgramIdentity::new("vulkanic:builtin/entity_outline_mask_v1"),
        vertex: ShaderStageSource {
            stage: ShaderStageKind::Vertex,
            label: "minimal-entity-outline.vertex".to_string(),
            source: MINIMAL_ENTITY_OUTLINE_VERTEX.to_string(),
            entry_point: "main".to_string(),
        },
        fragment: ShaderStageSource {
            stage: ShaderStageKind::Fragment,
            label: "minimal-entity-outline.fragment".to_string(),
            source: MINIMAL_ENTITY_OUTLINE_FRAGMENT.to_string(),
            entry_point: "main".to_string(),
        },
        required_resources: Vec::new(),
    }
}

/// Rust-owned fragment-only optical mask program.  The vertex ABI remains the
/// copied world-mesh instance stream, while the fragment writes zero alpha so
/// the optical target preserves its copied color and changes only stencil.
pub fn minimal_optical_stencil_write_program() -> TerrainMaterialProgram {
    let mut program = minimal_direct_terrain_cutout_program();
    program.identity = ProgramIdentity::new("vulkanic:builtin/optical_stencil_write_v1");
    program.fragment = ShaderStageSource {
        stage: ShaderStageKind::Fragment,
        label: "minimal-optical-stencil-write.fragment".to_string(),
        source: MINIMAL_OPTICAL_STENCIL_WRITE_FRAGMENT.to_string(),
        entry_point: "main".to_string(),
    };
    program
}

/// First Rust-owned material program for Distant Horizons' opaque CPU LOD
/// stream. It is intentionally separate from Minecraft terrain: DH carries
/// pre-resolved vertex color and packed light, not atlas UVs or sprite
/// identities. The eventual frontend supplies its four explicit bindings as
/// Rust-owned GAL resources; this description contains no Java/DH GPU state.
pub fn minimal_distant_horizons_lod_opaque_program() -> TerrainMaterialProgram {
    TerrainMaterialProgram {
        identity: ProgramIdentity::new("vulkanic:builtin/distant_horizons_lod_opaque_v1"),
        vertex: ShaderStageSource {
            stage: ShaderStageKind::Vertex,
            label: "minimal-distant-horizons-lod-opaque.vertex".to_string(),
            source: MINIMAL_DISTANT_HORIZONS_LOD_OPAQUE_VERTEX.to_string(),
            entry_point: "main".to_string(),
        },
        fragment: ShaderStageSource {
            stage: ShaderStageKind::Fragment,
            label: "minimal-distant-horizons-lod-opaque.fragment".to_string(),
            source: MINIMAL_DISTANT_HORIZONS_LOD_OPAQUE_FRAGMENT.to_string(),
            entry_point: "main".to_string(),
        },
        required_resources: Vec::new(),
    }
}

/// Rust-owned alpha-blended material program for Distant Horizons' non-water
/// transparent CPU LOD streams. It shares the copied DH vertex and semantic
/// lightmap contract with the opaque program, but writes one composited color
/// attachment and deliberately does not participate in the G-buffer or shadow
/// pass. Water is intentionally resolved by the separate Rust water-surface
/// pass, whose depth/cull/blend policy is not conflated with this generic lane.
pub fn minimal_distant_horizons_lod_transparent_program() -> TerrainMaterialProgram {
    TerrainMaterialProgram {
        identity: ProgramIdentity::new("vulkanic:builtin/distant_horizons_lod_transparent_v1"),
        vertex: ShaderStageSource {
            stage: ShaderStageKind::Vertex,
            label: "minimal-distant-horizons-lod-transparent.vertex".to_string(),
            source: MINIMAL_DISTANT_HORIZONS_LOD_OPAQUE_VERTEX.to_string(),
            entry_point: "main".to_string(),
        },
        fragment: ShaderStageSource {
            stage: ShaderStageKind::Fragment,
            label: "minimal-distant-horizons-lod-transparent.fragment".to_string(),
            source: MINIMAL_DISTANT_HORIZONS_LOD_TRANSPARENT_FRAGMENT.to_string(),
            entry_point: "main".to_string(),
        },
        required_resources: Vec::new(),
    }
}

/// Exact-atlas opaque DH material program. This is deliberately distinct from
/// the reduced color stream: it is admitted only for immutable column
/// segments whose copied face provenance resolves every quad to the owned
/// Minecraft terrain atlas.
pub fn minimal_distant_horizons_lod_exact_atlas_opaque_program() -> TerrainMaterialProgram {
    TerrainMaterialProgram {
        identity: ProgramIdentity::new(
            "vulkanic:builtin/distant_horizons_lod_exact_atlas_opaque_v2",
        ),
        vertex: ShaderStageSource {
            stage: ShaderStageKind::Vertex,
            label: "minimal-distant-horizons-lod-exact-atlas-opaque.vertex".to_string(),
            source: MINIMAL_DISTANT_HORIZONS_LOD_EXACT_ATLAS_OPAQUE_VERTEX.to_string(),
            entry_point: "main".to_string(),
        },
        fragment: ShaderStageSource {
            stage: ShaderStageKind::Fragment,
            label: "minimal-distant-horizons-lod-exact-atlas-opaque.fragment".to_string(),
            source: MINIMAL_DISTANT_HORIZONS_LOD_EXACT_ATLAS_OPAQUE_FRAGMENT.to_string(),
            entry_point: "main".to_string(),
        },
        required_resources: Vec::new(),
    }
}

/// Exact-atlas DH writer for a source-derived frame. The selected DH program
/// writes one named primary color target; this private Rust writer therefore
/// retains that exact output schema while resolving immutable face ranges to
/// an owned Minecraft atlas. It does not reinterpret DH as an ordinary
/// terrain G-buffer writer.
pub fn minimal_distant_horizons_lod_exact_atlas_source_program() -> TerrainMaterialProgram {
    TerrainMaterialProgram {
        identity: ProgramIdentity::new(
            "vulkanic:builtin/distant_horizons_lod_exact_atlas_source_v1",
        ),
        vertex: ShaderStageSource {
            stage: ShaderStageKind::Vertex,
            label: "minimal-distant-horizons-lod-exact-atlas-source.vertex".to_string(),
            source: MINIMAL_DISTANT_HORIZONS_LOD_EXACT_ATLAS_OPAQUE_VERTEX.to_string(),
            entry_point: "main".to_string(),
        },
        fragment: ShaderStageSource {
            stage: ShaderStageKind::Fragment,
            label: "minimal-distant-horizons-lod-exact-atlas-source.fragment".to_string(),
            source: MINIMAL_DISTANT_HORIZONS_LOD_EXACT_ATLAS_SOURCE_FRAGMENT.to_string(),
            entry_point: "main".to_string(),
        },
        required_resources: Vec::new(),
    }
}

/// The complete two-set binding contract for the private DH opaque program.
/// Set zero changes with a visible LOD segment; set one changes only with a
/// complete Rust-owned vanilla-lightmap generation. The descriptions are
/// backend-neutral and can be consumed unchanged by the Rust Vulkan and
/// OpenGL implementations.
pub fn distant_horizons_lod_opaque_resource_layouts(label: &str) -> [ResourceLayoutDesc; 2] {
    [
        ResourceLayoutDesc {
            label: format!("{label}.geometry-and-frame"),
            bindings: vec![
                ResourceBindingDesc {
                    binding: 0,
                    kind: ResourceBindingKind::StorageBuffer,
                    stages: PipelineStageFlags::DRAW,
                    array_count: 1,
                    optional: false,
                    dynamic_offset_count: 0,
                },
                ResourceBindingDesc {
                    binding: 1,
                    kind: ResourceBindingKind::UniformBuffer,
                    stages: PipelineStageFlags::DRAW,
                    array_count: 1,
                    optional: false,
                    dynamic_offset_count: 0,
                },
            ],
        },
        ResourceLayoutDesc {
            label: format!("{label}.lightmap"),
            bindings: vec![
                ResourceBindingDesc {
                    binding: 0,
                    kind: ResourceBindingKind::SampledTexture,
                    stages: PipelineStageFlags::DRAW,
                    array_count: 1,
                    optional: false,
                    dynamic_offset_count: 0,
                },
                ResourceBindingDesc {
                    binding: 1,
                    kind: ResourceBindingKind::Sampler,
                    stages: PipelineStageFlags::DRAW,
                    array_count: 1,
                    optional: false,
                    dynamic_offset_count: 0,
                },
            ],
        },
    ]
}

/// Three-set contract for a DH segment with fully resolved per-face Minecraft
/// atlas provenance. The atlas and lightmap are both Rust-owned semantic
/// resources; neither is a borrowed Java/OpenGL binding.
pub fn distant_horizons_lod_exact_atlas_resource_layouts(label: &str) -> [ResourceLayoutDesc; 2] {
    let [geometry_and_frame, _lightmap] = distant_horizons_lod_opaque_resource_layouts(label);
    [
        geometry_and_frame,
        ResourceLayoutDesc {
            label: format!("{label}.terrain-atlas-and-lightmap"),
            bindings: vec![
                ResourceBindingDesc {
                    binding: 0,
                    kind: ResourceBindingKind::SampledTexture,
                    stages: PipelineStageFlags::DRAW,
                    array_count: 1,
                    optional: false,
                    dynamic_offset_count: 0,
                },
                ResourceBindingDesc {
                    binding: 1,
                    kind: ResourceBindingKind::Sampler,
                    stages: PipelineStageFlags::DRAW,
                    array_count: 1,
                    optional: false,
                    dynamic_offset_count: 0,
                },
                ResourceBindingDesc {
                    binding: 2,
                    kind: ResourceBindingKind::SampledTexture,
                    stages: PipelineStageFlags::DRAW,
                    array_count: 1,
                    optional: false,
                    dynamic_offset_count: 0,
                },
                ResourceBindingDesc {
                    binding: 3,
                    kind: ResourceBindingKind::Sampler,
                    stages: PipelineStageFlags::DRAW,
                    array_count: 1,
                    optional: false,
                    dynamic_offset_count: 0,
                },
            ],
        },
    ]
}

/// Additional set used only by the source-derived exact-atlas DH adapter.
/// The selected pack's declared resources remain in its ordinary set one;
/// this set supplies the copied Minecraft atlas that turns resolved tile UVs
/// into the source program's semantic vertex color.
pub fn distant_horizons_exact_atlas_source_resource_layout(label: &str) -> ResourceLayoutDesc {
    ResourceLayoutDesc {
        label: format!("{label}.exact-atlas"),
        bindings: vec![
            ResourceBindingDesc {
                binding: 0,
                kind: ResourceBindingKind::SampledTexture,
                stages: PipelineStageFlags::DRAW,
                array_count: 1,
                optional: false,
                dynamic_offset_count: 0,
            },
            ResourceBindingDesc {
                binding: 1,
                kind: ResourceBindingKind::Sampler,
                stages: PipelineStageFlags::DRAW,
                array_count: 1,
                optional: false,
                dynamic_offset_count: 0,
            },
        ],
    }
}

pub fn minimal_terrain_translucent_program() -> TerrainMaterialProgram {
    minimal_terrain_material_program(TerrainMaterialProgramKind::Translucent)
}

pub fn minimal_direct_terrain_solid_program() -> TerrainMaterialProgram {
    minimal_direct_terrain_material_program(TerrainMaterialProgramKind::Opaque)
}

pub fn minimal_direct_terrain_cutout_program() -> TerrainMaterialProgram {
    minimal_direct_terrain_material_program(TerrainMaterialProgramKind::Cutout)
}

/// Direct forward counterpart of the deferred translucent terrain program.
/// Vanilla's non-source route has one color attachment, so it must never bind
/// the multi-output G-buffer fragment merely because the material is water or
/// another translucent block.
pub fn minimal_direct_terrain_translucent_program() -> TerrainMaterialProgram {
    minimal_direct_terrain_material_program(TerrainMaterialProgramKind::Translucent)
}

/// Blended model texture with the vanilla item/entity alpha-discard contract.
/// This is not Sodium translucent terrain: model sampling uses implicit LOD,
/// and alpha below 0.1 must discard before any depth/stencil write.
pub fn minimal_direct_model_translucent_cutout_program() -> TerrainMaterialProgram {
    let mut program = minimal_direct_terrain_translucent_program();
    program.identity = ProgramIdentity::new("vulkanic:builtin/direct_model_translucent_cutout_v1");
    program.vertex.label = "direct-model-translucent-cutout.vertex".to_string();
    program.vertex.source = program.vertex.source.replacen(
        "#version 450\n", "#version 450\n#define VULKANIC_MODEL_TRANSLUCENT_CUTOUT 1\n", 1);
    program.fragment.label = "direct-model-translucent-cutout.fragment".to_string();
    program.fragment.source = program.fragment.source.replacen(
        "#version 450\n", "#version 450\n#define VULKANIC_MODEL_TRANSLUCENT_CUTOUT 1\n", 1);
    program
}

/// Private standard model foil program. Requires an explicit per-instance
/// foil buffer at set0/binding4 and original UVs. The explicit frontend owns
/// transport, binding and lifetime validation. Never accepts decal foil.
pub const STANDARD_ITEM_FOIL_PROGRAM_ID: &str = "vulkanic:builtin/direct_standard_item_foil_v1";

pub fn minimal_direct_standard_item_foil_program() -> TerrainMaterialProgram {
    let mut program = minimal_direct_terrain_cutout_program();
    program.identity = ProgramIdentity::new(STANDARD_ITEM_FOIL_PROGRAM_ID);
    program.vertex.label = "direct-standard-item-foil.vertex".to_string();
    program.vertex.source = MINIMAL_TERRAIN_MATERIAL_VERTEX.replacen(
        "#version 450\n", "#version 450\n#define VULKANIC_STANDARD_ITEM_FOIL 1\n", 1);
    program.fragment.label = "direct-standard-item-foil.fragment".to_string();
    program.fragment.source = STANDARD_ITEM_FOIL_FRAGMENT.to_string();
    program
}

pub fn minimal_terrain_material_program(
    kind: TerrainMaterialProgramKind,
) -> TerrainMaterialProgram {
    TerrainMaterialProgram {
        identity: kind.identity(),
        vertex: ShaderStageSource {
            stage: ShaderStageKind::Vertex,
            label: format!("minimal-terrain-{}.vertex", kind.label_suffix()),
            source: MINIMAL_TERRAIN_MATERIAL_VERTEX.to_string(),
            entry_point: "main".to_string(),
        },
        fragment: ShaderStageSource {
            stage: ShaderStageKind::Fragment,
            label: format!("minimal-terrain-{}.fragment", kind.label_suffix()),
            // Frozen Sodium compiles `USE_FRAGMENT_DISCARD` exclusively for
            // the cutout pass.  Fabulous opaque terrain shares this deferred
            // program, so it must not accidentally inherit cutout's
            // front-face/alpha rejection before deferred lighting.  The
            // semantic pass kind selects the source-equivalent define here;
            // no backend state or Java renderer policy is involved.
            source: minimal_deferred_terrain_fragment_source(kind),
            entry_point: "main".to_string(),
        },
        required_resources: Vec::new(),
    }
}

/// Builds the first executable Rust-owned lowering for the normal terrain
/// contract. It is deliberately admitted only for a fully supported source
/// profile; callers must not use it as a fallback for a richer selected pack.
pub fn complementary_terrain_subset_program(
    contract: &TerrainPassContract,
    kind: TerrainMaterialProgramKind,
) -> GalResult<TerrainMaterialProgram> {
    complementary_terrain_subset_program_with_resources(contract, kind, None, 0)
}

/// Selects the source-derived lowering only after every semantic resource
/// required by the selected profile has a complete, matching generation.
pub fn complementary_terrain_subset_program_with_resources(
    contract: &TerrainPassContract,
    kind: TerrainMaterialProgramKind,
    voxel_light_volume: Option<&VoxelLightVolumeReadiness>,
    frame_counter: u64,
) -> GalResult<TerrainMaterialProgram> {
    contract.require_selected_subset_with_resources(voxel_light_volume, frame_counter)?;
    if !matches!(
        kind,
        TerrainMaterialProgramKind::Opaque | TerrainMaterialProgramKind::Cutout
    ) {
        return Err(
            crate::render::vulkanic::error::GalError::unsupported_feature(
                "Complementary terrain subset only supports opaque and cutout materials",
            ),
        );
    }
    let requires_colored_voxel_light = contract
        .required_resources
        .contains(&super::terrain_contract::TerrainPassRequiredResource::ColoredVoxelLightVolume);
    Ok(TerrainMaterialProgram {
        identity: ProgramIdentity::new(format!(
            "vulkanic:shader-pack/{}/terrain_{}_subset_v1",
            contract.pack_name.to_ascii_lowercase(),
            kind.label_suffix()
        )),
        vertex: ShaderStageSource {
            stage: ShaderStageKind::Vertex,
            label: format!("terrain-contract-{}.vertex", kind.label_suffix()),
            source: MINIMAL_TERRAIN_MATERIAL_VERTEX.to_string(),
            entry_point: "main".to_string(),
        },
        fragment: ShaderStageSource {
            stage: ShaderStageKind::Fragment,
            label: format!("terrain-contract-{}.fragment", kind.label_suffix()),
            source: complementary_terrain_subset_fragment_source(requires_colored_voxel_light),
            entry_point: "main".to_string(),
        },
        required_resources: if requires_colored_voxel_light {
            vec![TerrainProgramResource::ColoredVoxelLightVolume]
        } else {
            Vec::new()
        },
    })
}

/// Keeps the shader interface exactly aligned with the semantic resource
/// contract. A profile without `ColoredVoxelLighting` must not declare an
/// unbound set-1 D3 interface merely because another admitted profile uses it.
fn complementary_terrain_subset_fragment_source(requires_colored_voxel_light: bool) -> String {
    if requires_colored_voxel_light {
        COMPLEMENTARY_TERRAIN_SUBSET_FRAGMENT.replacen(
            "#version 450\n",
            "#version 450\n#define VULKANIC_TERRAIN_COLORED_VOXEL_LIGHT 1\n",
            1,
        )
    } else {
        COMPLEMENTARY_TERRAIN_SUBSET_FRAGMENT.to_string()
    }
}

pub fn minimal_direct_terrain_material_program(
    kind: TerrainMaterialProgramKind,
) -> TerrainMaterialProgram {
    TerrainMaterialProgram {
        identity: ProgramIdentity::new(match kind {
            TerrainMaterialProgramKind::Opaque => "vulkanic:builtin/direct_terrain_opaque_v1",
            TerrainMaterialProgramKind::Cutout => "vulkanic:builtin/direct_terrain_cutout_v1",
            TerrainMaterialProgramKind::Translucent => {
                "vulkanic:builtin/direct_terrain_translucent_v1"
            }
        }),
        vertex: ShaderStageSource {
            stage: ShaderStageKind::Vertex,
            label: format!("minimal-direct-terrain-{}.vertex", kind.label_suffix()),
            source: minimal_direct_terrain_vertex_source(),
            entry_point: "main".to_string(),
        },
        fragment: ShaderStageSource {
            stage: ShaderStageKind::Fragment,
            label: format!("minimal-direct-terrain-{}.fragment", kind.label_suffix()),
            // Frozen Sodium compiles `USE_FRAGMENT_DISCARD` only for the
            // cutout terrain pass.  In particular, translucent terrain must
            // retain both faces and its source alpha for the later Fabulous
            // composition; applying the cutout discard rule there changes
            // glass and fluid coverage before the compositor ever sees it.
            source: minimal_direct_terrain_fragment_source(kind),
            entry_point: "main".to_string(),
        },
        required_resources: Vec::new(),
    }
}

fn minimal_direct_terrain_fragment_source(kind: TerrainMaterialProgramKind) -> String {
    terrain_fragment_source_with_pass_define(MINIMAL_TERRAIN_MATERIAL_FRAGMENT_DIRECT, kind)
}

fn minimal_deferred_terrain_fragment_source(kind: TerrainMaterialProgramKind) -> String {
    terrain_fragment_source_with_pass_define(MINIMAL_TERRAIN_MATERIAL_FRAGMENT, kind)
}

fn terrain_fragment_source_with_pass_define(
    source: &str,
    kind: TerrainMaterialProgramKind,
) -> String {
    let discard_define = terrain_fragment_discard_define(kind);
    source.replacen("#version 450\n", &format!("#version 450\n{discard_define}"), 1)
}

fn terrain_fragment_discard_define(kind: TerrainMaterialProgramKind) -> &'static str {
    matches!(kind, TerrainMaterialProgramKind::Cutout)
        .then_some("#define VULKANIC_TERRAIN_FRAGMENT_DISCARD 1\n")
        .unwrap_or_default()
}

/// Frozen's active OpenGL Sodium terrain program samples the 16×16 lightmap at each
/// vertex from its packed `a_LightAndData.xy / 256` coordinate, then lets
/// normal varying interpolation carry the lit vertex colour across the
/// triangle. Sodium's CPU encoder writes each packed 0..15 light level as a
/// byte coordinate `level * 16`, and its OpenGL block-layer vertex shader divides that byte by
/// 256 before sampling it with LightTexture's linear sampler. The semantic
/// level/15 representation therefore converts to `level * 15 / 16`, not a
/// texel-centre coordinate.
/// Both direct and deferred builtin terrain use this exact vertex contract.
fn minimal_direct_terrain_vertex_source() -> String {
    MINIMAL_TERRAIN_MATERIAL_VERTEX.to_string()
}

pub fn minimal_g_buffer_composite_program() -> CompositeProgram {
    minimal_deferred_lighting_program()
}

pub fn minimal_deferred_lighting_program() -> CompositeProgram {
    CompositeProgram {
        identity: ProgramIdentity::new("vulkanic:builtin/deferred_lighting_v1"),
        vertex: ShaderStageSource {
            stage: ShaderStageKind::Vertex,
            label: "minimal-deferred-lighting.vertex".to_string(),
            source: MINIMAL_FULLSCREEN_VERTEX.to_string(),
            entry_point: "main".to_string(),
        },
        fragment: ShaderStageSource {
            stage: ShaderStageKind::Fragment,
            label: "minimal-deferred-lighting.fragment".to_string(),
            source: MINIMAL_DEFERRED_LIGHTING_FRAGMENT.to_string(),
            entry_point: "main".to_string(),
        },
    }
}

pub fn minimal_composite_color_grade_program() -> CompositeProgram {
    CompositeProgram {
        identity: ProgramIdentity::new("vulkanic:builtin/composite_color_grade_v1"),
        vertex: ShaderStageSource {
            stage: ShaderStageKind::Vertex,
            label: "minimal-composite-color-grade.vertex".to_string(),
            source: MINIMAL_FULLSCREEN_VERTEX.to_string(),
            entry_point: "main".to_string(),
        },
        fragment: ShaderStageSource {
            stage: ShaderStageKind::Fragment,
            label: "minimal-composite-color-grade.fragment".to_string(),
            source: MINIMAL_COMPOSITE_COLOR_GRADE_FRAGMENT.to_string(),
            entry_point: "main".to_string(),
        },
    }
}

pub fn minimal_composite_depth_fog_program() -> CompositeProgram {
    CompositeProgram {
        identity: ProgramIdentity::new("vulkanic:builtin/composite_depth_fog_v1"),
        vertex: ShaderStageSource {
            stage: ShaderStageKind::Vertex,
            label: "minimal-composite-depth-fog.vertex".to_string(),
            source: MINIMAL_FULLSCREEN_VERTEX.to_string(),
            entry_point: "main".to_string(),
        },
        fragment: ShaderStageSource {
            stage: ShaderStageKind::Fragment,
            label: "minimal-composite-depth-fog.fragment".to_string(),
            source: MINIMAL_COMPOSITE_DEPTH_FOG_FRAGMENT.to_string(),
            entry_point: "main".to_string(),
        },
    }
}

pub fn minimal_final_copy_program() -> CompositeProgram {
    CompositeProgram {
        identity: ProgramIdentity::new("vulkanic:builtin/final_copy_v1"),
        vertex: ShaderStageSource {
            stage: ShaderStageKind::Vertex,
            label: "minimal-final-copy.vertex".to_string(),
            source: MINIMAL_FULLSCREEN_VERTEX.to_string(),
            entry_point: "main".to_string(),
        },
        fragment: ShaderStageSource {
            stage: ShaderStageKind::Fragment,
            label: "minimal-final-copy.fragment".to_string(),
            source: MINIMAL_FINAL_COPY_FRAGMENT.to_string(),
            entry_point: "main".to_string(),
        },
    }
}

pub fn minimal_shadow_depth_program() -> TerrainMaterialProgram {
    TerrainMaterialProgram {
        identity: ProgramIdentity::new("vulkanic:builtin/shadow_depth_v1"),
        vertex: ShaderStageSource {
            stage: ShaderStageKind::Vertex,
            label: "minimal-shadow-depth.vertex".to_string(),
            source: MINIMAL_SHADOW_DEPTH_VERTEX.to_string(),
            entry_point: "main".to_string(),
        },
        fragment: ShaderStageSource {
            stage: ShaderStageKind::Fragment,
            label: "minimal-shadow-depth.fragment".to_string(),
            source: MINIMAL_SHADOW_DEPTH_FRAGMENT.to_string(),
            entry_point: "main".to_string(),
        },
        required_resources: Vec::new(),
    }
}

pub fn shader_stage_code_for_backend(api: BackendApi, source: &str) -> Vec<u8> {
    if api != BackendApi::Vulkan {
        return source.as_bytes().to_vec();
    }
    // A Vulkan framebuffer's row direction and sampled-image coordinates
    // differ from the pass graph's top-left image convention. Every
    // fullscreen transfer compensates exactly once; otherwise forward work
    // inserted after a transfer acquires a different vertical parity from
    // G-buffer content.
    let prefix = "#version 450\n#define VULKANIC_GAL_ZERO_TO_ONE_CLIP_DEPTH 1\n#define VULKANIC_GAL_FLIP_FULLSCREEN_UV_Y 1\n";
    source.replacen("#version 450\n", &prefix, 1).into_bytes()
}

#[cfg(test)]
mod shader_stage_code_tests {
    use super::*;

    #[test]
    fn vulkan_fullscreen_transfers_correct_framebuffer_to_texture_row_origin() {
        let source = "#version 450\n#ifdef VULKANIC_GAL_FULLSCREEN_UV_TOP_ORIGIN\n#endif\n";
        let lowered = String::from_utf8(shader_stage_code_for_backend(BackendApi::Vulkan, source))
            .expect("Vulkan shader source must remain UTF-8");

        assert!(lowered.contains("#define VULKANIC_GAL_ZERO_TO_ONE_CLIP_DEPTH 1"));
        assert!(lowered.contains("#define VULKANIC_GAL_FLIP_FULLSCREEN_UV_Y 1"));
        assert!(!lowered.contains("#define VULKANIC_GAL_FULLSCREEN_UV_TOP_ORIGIN 1"));
    }

    #[test]
    fn vulkan_fullscreen_transfers_correct_the_framebuffer_to_texture_row_origin() {
        let source = "#version 450\n#ifdef VULKANIC_GAL_FLIP_FULLSCREEN_UV_Y\n#endif\n";
        let lowered = String::from_utf8(shader_stage_code_for_backend(BackendApi::Vulkan, source))
            .expect("Vulkan shader source must remain UTF-8");

        assert!(lowered.contains("#define VULKANIC_GAL_FLIP_FULLSCREEN_UV_Y 1"));
    }
}

pub const MINIMAL_TERRAIN_MATERIAL_VERTEX: &str = r#"#version 450
layout(set = 1, binding = 0) uniform texture2D LightmapTexture;
layout(set = 1, binding = 1) uniform sampler LightmapSampler;
struct MeshVertex {
    vec4 position_uv;
    vec4 color_uv;
    vec4 normal_light;
    vec4 extra_data;
    vec4 shader_data;
};
layout(set = 0, binding = 0, std430) readonly buffer WorldMeshVertices {
    MeshVertex vertices[];
};
struct MeshInstance {
    mat4 model;
    vec4 color;
    vec4 material;
    vec4 animation_region;
    vec4 animation_next_region;
    vec4 overlay_color;
};
layout(set = 0, binding = 1, std430) readonly buffer WorldMeshInstances {
    mat4 view;
    mat4 projection;
    mat4 light_view_projection;
    vec4 shadow_params;
    vec4 fog_color_and_environmental_start;
    vec4 fog_ranges;
    MeshInstance instances[];
};
layout(location = 0) out vec2 v_uv;
layout(location = 1) out vec4 v_color;
layout(location = 2) flat out vec4 v_material;
layout(location = 3) out vec3 v_normal;
layout(location = 4) out vec2 v_light;
layout(location = 5) out vec3 v_world_position;
layout(location = 6) flat out vec4 v_animation_region;
layout(location = 7) flat out vec4 v_animation_next_region;
layout(location = 8) flat out vec4 v_overlay_color;
// Frozen's terrain shader interpolates distances and resolves the non-linear
// clamped fog function in the fragment shader.  Never interpolate the final
// fog factor: that changes the ramp across large (especially translucent)
// terrain triangles.
layout(location = 9) out vec2 v_fog_distances;
layout(location = 10) flat out vec4 v_fog_color_and_environmental_start;
layout(location = 11) flat out vec4 v_fog_ranges;
layout(location = 12) flat out uint v_terrain_material_bits;
#ifdef VULKANIC_STANDARD_ITEM_FOIL
// Two affine rows (xy basis, z translation) and RGB strength. Padding is
// explicit; no reuse of terrain animation fields or packed vertex alpha.
struct ItemFoilInstance { vec4 uv_row0; vec4 uv_row1; vec4 parameters; };
layout(set = 0, binding = 4, std430) readonly buffer ItemFoilInstances {
    ItemFoilInstance foil_instances[];
};
layout(location = 13) flat out float v_foil_strength;
#endif
void main() {
    MeshVertex vertex = vertices[gl_VertexIndex];
    MeshInstance instance = instances[gl_InstanceIndex];
    vec4 world = instance.model * vec4(vertex.position_uv.xyz, 1.0);
    vec4 clip = projection * view * world;
#ifdef VULKANIC_GAL_ZERO_TO_ONE_CLIP_DEPTH
    clip.z = clip.z * 0.5 + clip.w * 0.5;
#endif
    gl_Position = clip;
    // A copied world-mesh vertex carries both its local sprite coordinate and
    // its resolved terrain-atlas coordinate.  The submitted semantic section
    // declares which coordinate space its owned texture uses; do not make a
    // backend infer that from a native texture object.  This keeps standalone
    // water sprites local while atlas-backed terrain (including glass) samples
    // its resolved atlas region.
    // Independent semantic flags, encoded exactly in the Rust-owned ABI lane.
    uint material_semantics = uint(instance.material.w);
    v_uv = (material_semantics & 1u) != 0u
        ? vertex.shader_data.xy
        : vec2(vertex.position_uv.w, vertex.color_uv.w);
    // Frozen OpenGL Sodium preserves UV2 byte coordinates (including smooth
    // lighting's fractional levels) and divides by 256 in chunk_vertex.glsl.
    // The copied vertex lane is byte/240; rescale without nibble truncation.
    // LightTexture is linear,
    // so this deliberately samples at `level / 16`, including the boundary
    // interpolation between neighbouring lightmap texels. Do not replace it
    // with texel-centre coordinates: that is a different lighting contract.
    // The resulting lit vertex color is then interpolated across the triangle.
#ifdef VULKANIC_STANDARD_ITEM_FOIL
    ItemFoilInstance foil = foil_instances[gl_InstanceIndex];
    vec3 original_uv = vec3(vertex.position_uv.w, vertex.color_uv.w, 1.0);
    v_uv = vec2(dot(foil.uv_row0.xyz, original_uv), dot(foil.uv_row1.xyz, original_uv));
    v_foil_strength = foil.parameters.x;
    // Frozen glint consumes ColorModulator, not baked tint or lightmap.
    v_color = instance.color;
#else
    vec2 light_uv = clamp(vertex.extra_data.xy, vec2(0.0), vec2(255.0 / 240.0)) * (15.0 / 16.0);
    // Frozen's standalone baked blocks use core/terrain.vsh, not Sodium's
    // chunk shader. Its half-texel offset samples the lightmap texel centres.
    if ((material_semantics & 8u) != 0u) {
        light_uv += vec2(0.5 / 16.0);
    }
    vec4 light_color = texture(sampler2D(LightmapTexture, LightmapSampler), light_uv);
    if ((material_semantics & 16u) != 0u) {
        ivec2 light_texel = ivec2(round(clamp(vertex.extra_data.xy, vec2(0.0), vec2(1.0)) * 15.0));
        light_color = texelFetch(sampler2D(LightmapTexture, LightmapSampler), light_texel, 0);
    }
    v_color = vec4(vertex.color_uv.rgb, vertex.normal_light.w) * instance.color
        * light_color;
    if ((material_semantics & 2u) != 0u) {
        // Frozen's entity.vsh applies minecraft_mix_light before the copied
        // UV2 lightmap. ModelPart normals are semantic mesh data, so Rust
        // owns this calculation rather than borrowing Java's Lighting UBO.
        const vec3 LIGHT0_DIRECTION = normalize(vec3(0.2, 1.0, -0.7));
        const vec3 LIGHT1_DIRECTION = normalize(vec3(-0.2, 1.0, 0.7));
        const vec3 NETHER_LIGHT1_DIRECTION = normalize(vec3(-0.2, -1.0, 0.7));
        // Mesh normals are model-local, just like positions. Frozen's baked
        // encoder transforms them by the pose normal matrix before lighting;
        // the separate frame view matrix is not part of that pose operation.
        vec3 normal = normalize(transpose(inverse(mat3(instance.model)))
            * vec3(vertex.normal_light.yz, vertex.extra_data.z));
#ifdef VULKANIC_MODEL_TRANSLUCENT_CUTOUT
        // Frozen BakedModelEncoder transforms then NormI8.pack truncates
        // each component toward zero before the vertex attribute is read.
        // Preserve that quantization in Rust's shader, not in Java extraction.
        // Do not normalize again: the packed vector is not exactly unit length.
        normal = trunc(clamp(normal, vec3(-1.0), vec3(1.0)) * 127.0) / 127.0;
#endif
        vec2 light = max(vec2(0.0), vec2(
            dot(LIGHT0_DIRECTION, normal),
            dot((material_semantics & 4u) != 0u ? NETHER_LIGHT1_DIRECTION : LIGHT1_DIRECTION, normal)
        ));
        float diffuse = min(1.0, (light.x + light.y) * 0.6 + 0.4);
        v_color.rgb *= diffuse;
    }
#endif
    v_material = instance.material;
    v_animation_region = instance.animation_region;
    v_animation_next_region = instance.animation_next_region;
    v_overlay_color = instance.overlay_color;
    v_normal = normalize(vec3(vertex.normal_light.yz, vertex.extra_data.z));
    v_light = clamp(vertex.extra_data.xy, vec2(0.0), vec2(1.0));
    v_terrain_material_bits = uint(clamp(vertex.extra_data.w, 0.0, 255.0));
    // Frozen's terrain vertex shader resolves both fog distances from
    // `Position + ModelOffset` before applying ModelViewMat.  `world` is this
    // explicit camera-relative semantic position.  Euclidean distance would
    // survive a view rotation, but Sodium's cylindrical distance would not;
    // using `view * world` here makes camera pitch alter the fog ramp.
    vec3 fog_position = world.xyz;
    v_fog_distances = vec2(
        length(fog_position),
        max(length(fog_position.xz), abs(fog_position.y))
    );
    v_fog_color_and_environmental_start = fog_color_and_environmental_start;
    v_fog_ranges = fog_ranges;
    float shadow_range = max(shadow_params.w, 1.0);
    v_world_position = world.xyz / shadow_range * 0.5 + 0.5;
}
"#;

/// Solid-color vertex contract for the Rust-owned entity-outline mask. It
/// reuses only the copied mesh vertex/instance ABI; terrain texture/material
/// semantics are intentionally absent from this pass.
pub const MINIMAL_ENTITY_OUTLINE_VERTEX: &str = r#"#version 450
struct MeshVertex {
    vec4 position_uv;
    vec4 color_uv;
    vec4 normal_light;
    vec4 extra_data;
    vec4 shader_data;
};
layout(set = 0, binding = 0, std430) readonly buffer WorldMeshVertices {
    MeshVertex vertices[];
};
struct MeshInstance {
    mat4 model;
    vec4 color;
    vec4 material;
    vec4 animation_region;
    vec4 animation_next_region;
    vec4 overlay_color;
};
layout(set = 0, binding = 1, std430) readonly buffer WorldMeshInstances {
    mat4 view;
    mat4 projection;
    mat4 light_view_projection;
    vec4 shadow_params;
    MeshInstance instances[];
};
layout(location = 0) out vec4 v_outline_color;
void main() {
    MeshVertex vertex = vertices[gl_VertexIndex];
    MeshInstance instance = instances[gl_InstanceIndex];
    vec4 world = instance.model * vec4(vertex.position_uv.xyz, 1.0);
    vec4 clip = projection * view * world;
#ifdef VULKANIC_GAL_ZERO_TO_ONE_CLIP_DEPTH
    clip.z = clip.z * 0.5 + clip.w * 0.5;
#endif
    gl_Position = clip;
    v_outline_color = instance.color;
}
"#;

/// Solid-color fragment contract for the entity-outline mask. Alpha is kept
/// from the semantic outline color so the bundled Sobel pass can detect edges.
pub const MINIMAL_ENTITY_OUTLINE_FRAGMENT: &str = r#"#version 450
layout(location = 0) in vec4 v_outline_color;
layout(location = 0) out vec4 out_outline_color;
void main() {
    out_outline_color = v_outline_color;
}
"#;

/// Fragment contract for an optical stencil-write draw.  Zero alpha with
/// alpha blending leaves the copied optical color untouched while the
/// explicit GAL stencil replace operation records the aperture value.
pub const MINIMAL_OPTICAL_STENCIL_WRITE_FRAGMENT: &str = r#"#version 450
layout(location = 0) out vec4 out_optical_mask;
void main() {
    out_optical_mask = vec4(0.0);
}
"#;

/// Vulkan-native fullscreen contracts for the bundled entity-outline chain.
/// They keep the vanilla effect's semantics while making descriptor bindings
/// explicit and avoiding a Java post-chain or implicit sampler lookup.
pub const MINIMAL_ENTITY_OUTLINE_FULLSCREEN_VERTEX: &str = r#"#version 450
const vec2 positions[3] = vec2[3](vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
layout(location = 0) out vec2 v_uv;
void main() {
    vec2 position = positions[gl_VertexIndex];
    gl_Position = vec4(position, 0.0, 1.0);
    v_uv = position * 0.5 + 0.5;
#ifdef VULKANIC_GAL_FLIP_FULLSCREEN_UV_Y
    v_uv.y = 1.0 - v_uv.y;
#endif
}
"#;

pub const MINIMAL_ENTITY_OUTLINE_SOBEL_FRAGMENT: &str = r#"#version 450
layout(set = 0, binding = 0) uniform texture2D InTexture;
layout(set = 0, binding = 1) uniform sampler InSampler;
layout(location = 0) in vec2 v_uv;
layout(location = 0) out vec4 out_color;
void main() {
    vec2 one_texel = 1.0 / vec2(textureSize(sampler2D(InTexture, InSampler), 0));
    vec4 center = texture(sampler2D(InTexture, InSampler), v_uv);
    vec4 left = texture(sampler2D(InTexture, InSampler), v_uv - vec2(one_texel.x, 0.0));
    vec4 right = texture(sampler2D(InTexture, InSampler), v_uv + vec2(one_texel.x, 0.0));
    vec4 up = texture(sampler2D(InTexture, InSampler), v_uv - vec2(0.0, one_texel.y));
    vec4 down = texture(sampler2D(InTexture, InSampler), v_uv + vec2(0.0, one_texel.y));
    float total = clamp(abs(center.a - left.a) + abs(center.a - right.a) + abs(center.a - up.a) + abs(center.a - down.a), 0.0, 1.0);
    vec3 color = center.rgb * center.a + left.rgb * left.a + right.rgb * right.a + up.rgb * up.a + down.rgb * down.a;
    out_color = vec4(color * 0.2, total);
}
"#;

pub const MINIMAL_ENTITY_OUTLINE_BLUR_FRAGMENT: &str = r#"#version 450
layout(set = 0, binding = 0) uniform texture2D InTexture;
layout(set = 0, binding = 1) uniform sampler InSampler;
layout(set = 0, binding = 2, std140) uniform BlurConfig { vec2 BlurDir; float Radius; };
layout(location = 0) in vec2 v_uv;
layout(location = 0) out vec4 out_color;
void main() {
    vec2 one_texel = 1.0 / vec2(textureSize(sampler2D(InTexture, InSampler), 0));
    vec2 step_value = one_texel * BlurDir;
    float radius = max(Radius, 0.0);
    vec4 blurred = vec4(0.0);
    for (float a = -radius + 0.5; a <= radius; a += 2.0) {
        blurred += texture(sampler2D(InTexture, InSampler), v_uv + step_value * a);
    }
    blurred += texture(sampler2D(InTexture, InSampler), v_uv + step_value * radius) * 0.5;
    out_color = blurred / (radius + 0.5);
}
"#;

pub const MINIMAL_ENTITY_OUTLINE_BLIT_FRAGMENT: &str = r#"#version 450
layout(set = 0, binding = 0) uniform texture2D InTexture;
layout(set = 0, binding = 1) uniform sampler InSampler;
layout(set = 0, binding = 2, std140) uniform BlitConfig { vec4 ColorModulate; };
layout(location = 0) in vec2 v_uv;
layout(location = 0) out vec4 out_color;
void main() {
    out_color = texture(sampler2D(InTexture, InSampler), v_uv) * ColorModulate;
}
"#;

/// Rust-owned DH vertex contract. The semantic record is exactly the private
/// 32-byte expansion produced by `world_primitive_frontend::lod`, not DH's
/// legacy GL vertex format. `LightmapTexture` and `LightmapSampler` are a
/// separately bound Rust-owned 16x16 semantic vanilla lightmap.
pub const MINIMAL_DISTANT_HORIZONS_LOD_OPAQUE_VERTEX: &str = r#"#version 450
struct DistantHorizonsLodVertex {
    float local_x;
    float local_y;
    float local_z;
    float micro_x;
    float micro_y;
    float micro_z;
    uint color_rgba;
    uint light_material_normal;
};
layout(set = 0, binding = 0, std430) readonly buffer DistantHorizonsLodVertices {
    DistantHorizonsLodVertex vertices[];
};
layout(set = 0, binding = 1, std140) uniform DistantHorizonsLodFrame {
    mat4 combined_matrix;
    vec4 column_origin_and_world_y;
    vec4 model_offset_and_reserved;
    vec4 clip_micro_noise_earth;
    uvec4 flags_and_noise;
};
layout(location = 0) out vec4 v_color;
layout(location = 1) flat out uvec2 v_light;
layout(location = 2) flat out uint v_material;
layout(location = 3) flat out uint v_normal;
layout(location = 4) out vec3 v_world_position;

vec3 dh_normal(uint normal) {
    if (normal == 0u) return vec3(0.0, -1.0, 0.0);
    if (normal == 1u) return vec3(0.0, 1.0, 0.0);
    if (normal == 2u) return vec3(0.0, 0.0, -1.0);
    if (normal == 3u) return vec3(0.0, 0.0, 1.0);
    if (normal == 4u) return vec3(-1.0, 0.0, 0.0);
    return vec3(1.0, 0.0, 0.0);
}

void main() {
    DistantHorizonsLodVertex vertex = vertices[gl_VertexIndex];
    // DH's compact stream stores a third micro field, but its terrain
    // transformer applies edge perturbation in the horizontal plane only.
    // Treating micro_y as height moves otherwise flat water geometry away
    // from the source terrain surface.
    vec3 local = vec3(vertex.local_x, vertex.local_y, vertex.local_z)
        + vec3(vertex.micro_x, 0.0, vertex.micro_z);
    vec3 world = local + column_origin_and_world_y.xyz;
    vec4 clip = combined_matrix * vec4(
        local + model_offset_and_reserved.xyz,
        1.0
    );
#ifdef VULKANIC_GAL_ZERO_TO_ONE_CLIP_DEPTH
    clip.z = clip.z * 0.5 + clip.w * 0.5;
#endif
    gl_Position = clip;
    v_color = vec4(
        float(vertex.color_rgba & 0xffu),
        float((vertex.color_rgba >> 8u) & 0xffu),
        float((vertex.color_rgba >> 16u) & 0xffu),
        float((vertex.color_rgba >> 24u) & 0xffu)
    ) / 255.0;
    v_light = uvec2(
        vertex.light_material_normal & 0xffu,
        (vertex.light_material_normal >> 8u) & 0xffu
    );
    v_material = (vertex.light_material_normal >> 16u) & 0xffu;
    v_normal = (vertex.light_material_normal >> 24u) & 0xffu;
    v_world_position = world;
}
"#;

/// Private Rust vertex stream for the exact-atlas DH subset. Its 56-byte
/// layout is owned by the world frontend and is not DH's legacy GL format.
pub const MINIMAL_DISTANT_HORIZONS_LOD_EXACT_ATLAS_OPAQUE_VERTEX: &str = r#"#version 450
struct DistantHorizonsLodExactAtlasVertex {
    float local_x;
    float local_y;
    float local_z;
    float micro_x;
    float micro_y;
    float micro_z;
    float tile_u;
    float tile_v;
    float atlas_u0;
    float atlas_v0;
    float atlas_u1;
    float atlas_v1;
    uint color_rgba;
    uint light_normal_pad;
};
layout(set = 0, binding = 0, std430) readonly buffer DistantHorizonsLodExactAtlasVertices {
    DistantHorizonsLodExactAtlasVertex vertices[];
};
layout(set = 0, binding = 1, std140) uniform DistantHorizonsLodFrame {
    mat4 combined_matrix;
    vec4 column_origin_and_world_y;
    vec4 model_offset_and_reserved;
    vec4 clip_micro_noise_earth;
    uvec4 flags_and_noise;
};
layout(location = 0) out vec2 v_tile_uv;
layout(location = 1) flat out vec4 v_atlas_rect;
layout(location = 2) out vec4 v_color;
layout(location = 3) flat out uvec2 v_light;
layout(location = 4) flat out uint v_normal;
layout(location = 5) out vec3 v_world_position;
layout(location = 6) flat out uint v_material_flags;

void main() {
    DistantHorizonsLodExactAtlasVertex vertex = vertices[gl_VertexIndex];
    // Keep the exact-atlas stream on the same DH X/Z-only micro-offset
    // contract as the reduced-color stream.
    vec3 local = vec3(vertex.local_x, vertex.local_y, vertex.local_z)
        + vec3(vertex.micro_x, 0.0, vertex.micro_z);
    vec3 world = local + column_origin_and_world_y.xyz;
    vec4 clip = combined_matrix * vec4(
        local + model_offset_and_reserved.xyz,
        1.0
    );
#ifdef VULKANIC_GAL_ZERO_TO_ONE_CLIP_DEPTH
    clip.z = clip.z * 0.5 + clip.w * 0.5;
#endif
    gl_Position = clip;
    v_tile_uv = vec2(vertex.tile_u, vertex.tile_v);
    v_atlas_rect = vec4(vertex.atlas_u0, vertex.atlas_v0, vertex.atlas_u1, vertex.atlas_v1);
    v_color = vec4(
        float(vertex.color_rgba & 0xffu),
        float((vertex.color_rgba >> 8u) & 0xffu),
        float((vertex.color_rgba >> 16u) & 0xffu),
        float((vertex.color_rgba >> 24u) & 0xffu)
    ) / 255.0;
    v_light = uvec2(
        vertex.light_normal_pad & 0xffu,
        (vertex.light_normal_pad >> 8u) & 0xffu
    );
    v_normal = (vertex.light_normal_pad >> 16u) & 0xffu;
    v_material_flags = (vertex.light_normal_pad >> 24u) & 0xffu;
    v_world_position = world;
}
"#;

pub const MINIMAL_DISTANT_HORIZONS_LOD_OPAQUE_FRAGMENT: &str = r#"#version 450
layout(set = 1, binding = 0) uniform texture2D LightmapTexture;
layout(set = 1, binding = 1) uniform sampler LightmapSampler;
layout(location = 0) in vec4 v_color;
layout(location = 1) flat in uvec2 v_light;
layout(location = 2) flat in uint v_material;
layout(location = 3) flat in uint v_normal;
layout(location = 4) in vec3 v_world_position;
layout(location = 0) out vec4 out_terrain_lit_color;
layout(location = 1) out vec4 out_terrain_view_space_normal;
layout(location = 2) out vec4 out_terrain_material_auxiliary;
layout(location = 3) out vec4 out_world_position;

vec3 dh_normal(uint normal) {
    if (normal == 0u) return vec3(0.0, -1.0, 0.0);
    if (normal == 1u) return vec3(0.0, 1.0, 0.0);
    if (normal == 2u) return vec3(0.0, 0.0, -1.0);
    if (normal == 3u) return vec3(0.0, 0.0, 1.0);
    if (normal == 4u) return vec3(-1.0, 0.0, 0.0);
    return vec3(1.0, 0.0, 0.0);
}

void main() {
    vec2 light_uv = (vec2(v_light) + vec2(0.5)) / 16.0;
    vec3 light_color = texture(sampler2D(LightmapTexture, LightmapSampler), light_uv).rgb;
    vec4 color = vec4(v_color.rgb * light_color, v_color.a);
    vec3 normal = dh_normal(v_normal);
    out_terrain_lit_color = color;
    out_terrain_view_space_normal = vec4(normal * 0.5 + 0.5, color.a);
    out_terrain_material_auxiliary = vec4(
        float(v_material) / 255.0,
        float(v_light.x) / 15.0,
        float(v_light.y) / 15.0,
        color.a
    );
    // The existing minimal deferred path stores a bounded world-space
    // encoding. LOD admission is not enabled until its exact frame range is
    // supplied by the future LOD pass/resource contract.
    out_world_position = vec4(clamp(v_world_position / 1024.0 * 0.5 + 0.5, 0.0, 1.0), color.a);
}
"#;

pub const MINIMAL_DISTANT_HORIZONS_LOD_EXACT_ATLAS_OPAQUE_FRAGMENT: &str = r#"#version 450
layout(set = 1, binding = 0) uniform texture2D TerrainAtlasColor;
layout(set = 1, binding = 1) uniform sampler TerrainAtlasSampler;
layout(set = 1, binding = 2) uniform texture2D LightmapTexture;
layout(set = 1, binding = 3) uniform sampler LightmapSampler;
layout(location = 0) in vec2 v_tile_uv;
layout(location = 1) flat in vec4 v_atlas_rect;
layout(location = 2) in vec4 v_color;
layout(location = 3) flat in uvec2 v_light;
layout(location = 4) flat in uint v_normal;
layout(location = 5) in vec3 v_world_position;
layout(location = 6) flat in uint v_material_flags;
layout(location = 0) out vec4 out_terrain_lit_color;
layout(location = 1) out vec4 out_terrain_view_space_normal;
layout(location = 2) out vec4 out_terrain_material_auxiliary;
layout(location = 3) out vec4 out_world_position;

vec3 dh_normal(uint normal) {
    if (normal == 0u) return vec3(0.0, -1.0, 0.0);
    if (normal == 1u) return vec3(0.0, 1.0, 0.0);
    if (normal == 2u) return vec3(0.0, 0.0, -1.0);
    if (normal == 3u) return vec3(0.0, 0.0, 1.0);
    if (normal == 4u) return vec3(-1.0, 0.0, 0.0);
    return vec3(1.0, 0.0, 0.0);
}

void main() {
    vec2 atlas_extent = vec2(textureSize(sampler2D(TerrainAtlasColor, TerrainAtlasSampler), 0));
    vec2 texel = vec2(0.5) / atlas_extent;
    vec2 sprite_min = v_atlas_rect.xy + texel;
    vec2 sprite_max = v_atlas_rect.zw - texel;
    // DH can merge one semantic material across many blocks. Repeat inside
    // that named sprite instead of letting a large face stretch one tile or
    // escape into a neighbour in the global Minecraft atlas.
    vec2 atlas_uv = mix(sprite_min, max(sprite_min, sprite_max), fract(v_tile_uv));
    vec4 atlas_color = texture(sampler2D(TerrainAtlasColor, TerrainAtlasSampler), atlas_uv);
    vec2 light_uv = (vec2(v_light) + vec2(0.5)) / 16.0;
    vec3 light_color = texture(sampler2D(LightmapTexture, LightmapSampler), light_uv).rgb;
    vec3 semantic_tint = (v_material_flags & 1u) != 0u ? v_color.rgb : vec3(1.0);
    vec4 color = vec4(atlas_color.rgb * semantic_tint * light_color, atlas_color.a * v_color.a);
    if (color.a < 0.1) discard;
    vec3 normal = dh_normal(v_normal);
    out_terrain_lit_color = color;
    out_terrain_view_space_normal = vec4(normal * 0.5 + 0.5, color.a);
    out_terrain_material_auxiliary = vec4(0.0, float(v_light.x) / 15.0, float(v_light.y) / 15.0, color.a);
    out_world_position = vec4(clamp(v_world_position / 1024.0 * 0.5 + 0.5, 0.0, 1.0), color.a);
}
"#;

/// The provenance-resolved exact-atlas source writer owns DH's one declared
/// lit-color output. Its target schema intentionally matches the selected
/// `dh_terrain` contract rather than ordinary terrain's G-buffer outputs.
pub const MINIMAL_DISTANT_HORIZONS_LOD_EXACT_ATLAS_SOURCE_FRAGMENT: &str = r#"#version 450
layout(set = 1, binding = 0) uniform texture2D TerrainAtlasColor;
layout(set = 1, binding = 1) uniform sampler TerrainAtlasSampler;
layout(set = 1, binding = 2) uniform texture2D LightmapTexture;
layout(set = 1, binding = 3) uniform sampler LightmapSampler;
layout(location = 0) in vec2 v_tile_uv;
layout(location = 1) flat in vec4 v_atlas_rect;
layout(location = 2) in vec4 v_color;
layout(location = 3) flat in uvec2 v_light;
layout(location = 4) flat in uint v_normal;
layout(location = 5) in vec3 v_world_position;
layout(location = 6) flat in uint v_material_flags;
layout(location = 0) out vec4 out_source_primary;

void main() {
    vec2 atlas_extent = vec2(textureSize(sampler2D(TerrainAtlasColor, TerrainAtlasSampler), 0));
    vec2 texel = vec2(0.5) / atlas_extent;
    vec2 sprite_min = v_atlas_rect.xy + texel;
    vec2 sprite_max = v_atlas_rect.zw - texel;
    vec2 atlas_uv = mix(sprite_min, max(sprite_min, sprite_max), fract(v_tile_uv));
    vec4 atlas_color = texture(sampler2D(TerrainAtlasColor, TerrainAtlasSampler), atlas_uv);
    vec2 light_uv = (vec2(v_light) + vec2(0.5)) / 16.0;
    vec3 light_color = texture(sampler2D(LightmapTexture, LightmapSampler), light_uv).rgb;
    vec3 semantic_tint = (v_material_flags & 1u) != 0u ? v_color.rgb : vec3(1.0);
    vec4 color = vec4(atlas_color.rgb * semantic_tint * light_color, atlas_color.a * v_color.a);
    if (color.a < 0.1) discard;
    out_source_primary = color;
}
"#;

/// The non-water transparent pass is composited after deferred lighting. It
/// keeps DH's source-visible alpha and lightmap result intact, with blend and
/// depth-write policy declared by the backend-neutral graphics pipeline.
pub const MINIMAL_DISTANT_HORIZONS_LOD_TRANSPARENT_FRAGMENT: &str = r#"#version 450
layout(set = 1, binding = 0) uniform texture2D LightmapTexture;
layout(set = 1, binding = 1) uniform sampler LightmapSampler;
layout(location = 0) in vec4 v_color;
layout(location = 1) flat in uvec2 v_light;
layout(location = 0) out vec4 out_color;

void main() {
    vec2 light_uv = (vec2(v_light) + vec2(0.5)) / 16.0;
    vec3 light_color = texture(sampler2D(LightmapTexture, LightmapSampler), light_uv).rgb;
    out_color = vec4(v_color.rgb * light_color, v_color.a);
}
"#;

pub const MINIMAL_TERRAIN_MATERIAL_FRAGMENT: &str = r#"#version 450
layout(set = 0, binding = 2) uniform texture2D Tex0;
layout(set = 0, binding = 3) uniform sampler Samp0;
layout(location = 0) in vec2 v_uv;
layout(location = 1) in vec4 v_color;
layout(location = 2) flat in vec4 v_material;
layout(location = 3) in vec3 v_normal;
layout(location = 4) in vec2 v_light;
layout(location = 5) in vec3 v_world_position;
layout(location = 6) flat in vec4 v_animation_region;
layout(location = 7) flat in vec4 v_animation_next_region;
layout(location = 8) flat in vec4 v_overlay_color;
layout(location = 9) in vec2 v_fog_distances;
layout(location = 10) flat in vec4 v_fog_color_and_environmental_start;
layout(location = 11) flat in vec4 v_fog_ranges;
layout(location = 12) flat in uint v_terrain_material_bits;
// These locations are implementation details. The shader-pack contract names
// the values terrain_lit_color, terrain_view_space_normal, and
// terrain_material_auxiliary.
layout(location = 0) out vec4 out_terrain_lit_color;
layout(location = 1) out vec4 out_terrain_view_space_normal;
layout(location = 2) out vec4 out_terrain_material_auxiliary;
layout(location = 3) out vec4 out_world_position;
// Frozen Java OpenGL reported GL_MAX_TEXTURE_LOD_BIAS = 15 for the paired
// baseline device. Sodium applies its negative counterpart only to materials
// whose compact material byte disables mipmaps. Preserve derivative-selected
// sampling; `textureLod(..., 0.0)` would incorrectly force the base mip.
const float FROZEN_MAX_TEXTURE_LOD_BIAS = 15.0;
// Keep the copied Frozen fog behavior intact even for disabled or degenerate
// ranges. In particular, `end <= start` is a hard transition, not a disabled
// fog range; only its explicit sentinel disables fog.
const float FROZEN_FOG_DISABLED_SENTINEL_THRESHOLD = 1.0e12;
bool frozen_fog_range_disabled(float start, float end) {
    return max(abs(start), abs(end)) >= FROZEN_FOG_DISABLED_SENTINEL_THRESHOLD;
}
bool frozen_fog_distance_invalid(float distance) {
    return distance != distance || abs(distance) >= FROZEN_FOG_DISABLED_SENTINEL_THRESHOLD;
}
float frozen_linear_fog_value(float distance, float start, float end) {
    if (frozen_fog_distance_invalid(distance) || frozen_fog_range_disabled(start, end)) {
        return 0.0;
    }
    if (end <= start) {
        return distance > start ? 1.0 : 0.0;
    }
    if (distance <= start) {
        return 0.0;
    }
    if (distance >= end) {
        return 1.0;
    }
    return clamp((distance - start) / (end - start), 0.0, 1.0);
}
void main() {
    vec2 sample_uv = v_animation_region.xy + v_uv * v_animation_region.zw;
    // Keep the deferred Fabulous material path on the same compact Sodium
    // material contract as the direct path.  These bits are semantic mesh
    // data, not a backend sampler workaround: they control whether a sprite
    // may use its mip chain and which alpha/cull rule its fragment receives.
    bool use_mipmaps = (v_terrain_material_bits & 1u) != 0u;
    vec4 color = use_mipmaps
        ? texture(sampler2D(Tex0, Samp0), sample_uv)
        : texture(sampler2D(Tex0, Samp0), sample_uv, -FROZEN_MAX_TEXTURE_LOD_BIAS);
    if (v_material.y > 0.5) {
        vec2 next_uv = v_animation_next_region.xy + v_uv * v_animation_next_region.zw;
        vec4 next_color = texture(sampler2D(Tex0, Samp0), next_uv);
        color = mix(color, next_color, clamp(v_material.z, 0.0, 1.0));
    }
    color *= v_color;
    if (v_overlay_color.a > 0.0) {
        color.rgb = mix(color.rgb, v_overlay_color.rgb, v_overlay_color.a);
    }
    uint alpha_cutoff_class = (v_terrain_material_bits >> 1u) & 3u;
    float alpha_cutoff = float[4](0.0, 0.1, 0.1, 1.0)[alpha_cutoff_class];
#ifdef VULKANIC_TERRAIN_FRAGMENT_DISCARD
    // Match Sodium: pass culling is explicit pipeline state, never a
    // material-dependent fragment-side back-face discard.
    if (color.a < max(v_material.x, alpha_cutoff)) {
        discard;
    }
#endif
    float environmental_fog = frozen_linear_fog_value(
        v_fog_distances.x,
        v_fog_color_and_environmental_start.w,
        v_fog_ranges.x
    );
    float render_distance_fog = frozen_linear_fog_value(
        v_fog_distances.y,
        v_fog_ranges.y,
        v_fog_ranges.z
    );
    vec3 n = normalize(v_normal) * 0.5 + 0.5;
    out_terrain_lit_color = color;
    // Keep Fog's copied per-vertex distances semantic through deferred work.
    // The normal alpha is otherwise unused by this admitted deferred path.
    out_terrain_view_space_normal = vec4(n, max(environmental_fog, render_distance_fog));
    // This deferred route only receives opaque/cutout terrain after its
    // material-specific discard. Sodium's compact vertex alpha may be baked
    // AO, so it is not an existence bit.  Carry explicit coverage separately
    // instead of allowing dark AO vertices to disappear in deferred lighting.
    out_terrain_material_auxiliary = vec4(v_material.x, v_light.x, v_light.y, 1.0);
    out_world_position = vec4(v_world_position, 1.0);
}
"#;

/// Rust-owned lowering of the stable normal-terrain expressions shared by the
/// audited Complementary source: atlas sample, alpha discard, tint/AO, light
/// map contribution, directional shade, and named G-buffer outputs. Richer
/// branches are admitted only after their semantic inputs have implementations.
pub const COMPLEMENTARY_TERRAIN_SUBSET_FRAGMENT: &str = r#"#version 450
layout(set = 0, binding = 2) uniform texture2D TerrainAtlasColor;
layout(set = 0, binding = 3) uniform sampler TerrainAtlasSampler;
#ifdef VULKANIC_TERRAIN_COLORED_VOXEL_LIGHT
// Rust-owned semantic ColoredVoxelLighting resources. These use a second
// ordinary GAL resource set rather than any Iris or backend-private state.
// The matching layout is created by TerrainVoxelLightSamplingResources.
layout(set = 1, binding = 0) uniform utexture3D TerrainVoxelOccupancy;
layout(set = 1, binding = 1) uniform texture3D TerrainColoredVoxelLight;
layout(set = 1, binding = 2) uniform sampler TerrainVoxelLightSampler;
layout(set = 1, binding = 3, std140) uniform TerrainVoxelLightMapping {
    vec4 scene_to_volume_offset_and_normal_offset;
    vec4 scene_to_volume_scale;
    vec4 inverse_extent;
    ivec4 valid_world_min;
    ivec4 valid_world_max_exclusive;
    ivec4 camera_cell;
};
#endif
layout(location = 0) in vec2 v_uv;
layout(location = 1) in vec4 v_color;
layout(location = 2) flat in vec4 v_material;
layout(location = 3) in vec3 v_normal;
layout(location = 4) in vec2 v_light;
layout(location = 5) in vec3 v_world_position;
layout(location = 6) flat in vec4 v_animation_region;
layout(location = 7) flat in vec4 v_animation_next_region;
layout(location = 0) out vec4 out_terrain_lit_color;
layout(location = 1) out vec4 out_terrain_view_space_normal;
layout(location = 2) out vec4 out_terrain_material_auxiliary;
layout(location = 3) out vec4 out_world_position;

float complementary_block_light(float value) {
    float steep = pow(value * value, 4.0) * 3.8;
    float calm = value * 1.8;
    return pow(steep + calm, 2.25);
}

#ifdef VULKANIC_TERRAIN_COLORED_VOXEL_LIGHT
vec3 terrain_voxel_coordinate(vec3 world_position, vec3 surface_normal) {
    vec3 shifted_world = world_position
        + surface_normal * scene_to_volume_offset_and_normal_offset.w;
    return (shifted_world - vec3(camera_cell.xyz)
        + scene_to_volume_offset_and_normal_offset.xyz)
        * scene_to_volume_scale.xyz
        * inverse_extent.xyz;
}
#endif

void main() {
    vec2 sample_uv = v_animation_region.xy + v_uv * v_animation_region.zw;
    vec4 sampled_atlas_color = texture(sampler2D(TerrainAtlasColor, TerrainAtlasSampler), sample_uv);
    if (v_material.y > 0.5) {
        vec2 next_uv = v_animation_next_region.xy + v_uv * v_animation_next_region.zw;
        sampled_atlas_color = mix(
            sampled_atlas_color,
            texture(sampler2D(TerrainAtlasColor, TerrainAtlasSampler), next_uv),
            clamp(v_material.z, 0.0, 1.0)
        );
    }
    // gbuffers_terrain: if (color.a <= 0.00001) discard;
    if (sampled_atlas_color.a <= 0.00001) discard;

    // gbuffers_terrain: color.rgb *= glColor.rgb;
    vec3 tint_result = sampled_atlas_color.rgb * v_color.rgb;
    float raw_ao = clamp(v_color.a, 0.0, 1.0);
    vec3 normal = normalize(v_normal);
    float directional_shade = 0.75 + 0.25 * max(normal.y, 0.0);
    float block_light = complementary_block_light(clamp(v_light.x, 0.0, 1.0));
    float sky_light = clamp(v_light.y, 0.0, 1.0);
    vec3 scene_lighting = vec3(0.18 + 0.82 * sky_light);
#ifdef VULKANIC_TERRAIN_COLORED_VOXEL_LIGHT
    vec3 voxel_coordinate = terrain_voxel_coordinate(v_world_position, normal);
    bool voxel_coordinate_in_bounds = all(greaterThanEqual(voxel_coordinate, vec3(0.0)))
        && all(lessThanEqual(voxel_coordinate, vec3(1.0)));
    // The occupancy lookup keeps the material field in the semantic contract
    // and makes a missing/incorrect integer D3 binding visible rather than
    // silently using only the flood-fill texture.
    uint voxel_occupancy = voxel_coordinate_in_bounds
        ? texture(usampler3D(TerrainVoxelOccupancy, TerrainVoxelLightSampler), voxel_coordinate).r
        : 0u;
    vec3 colored_voxel_light = voxel_coordinate_in_bounds && voxel_occupancy != 0u
        ? texture(sampler3D(TerrainColoredVoxelLight, TerrainVoxelLightSampler), voxel_coordinate).rgb
        : vec3(0.0);
#else
    vec3 colored_voxel_light = vec3(0.0);
#endif
    vec3 final_diffuse = sqrt(max(
        vec3(raw_ao * directional_shade * directional_shade) *
        (vec3(block_light) + scene_lighting * scene_lighting + colored_voxel_light),
        vec3(0.0)
    ));
    vec4 terrain_lit_color = vec4(tint_result * final_diffuse, sampled_atlas_color.a);
    // gbuffers_terrain writes this only when its reflection profile is on.
    // The output is still pass-local and does not require implementing the
    // later deferred reflection consumer.
    float sky_light_factor = pow(max(sky_light - 0.7, 0.0) * 3.33333, 2.0);

    out_terrain_lit_color = terrain_lit_color;
    out_terrain_material_auxiliary = vec4(0.0, 0.0, sky_light_factor, 1.0);
    out_terrain_view_space_normal = vec4(normal, 1.0);
    out_world_position = vec4(v_world_position, terrain_lit_color.a);
}
"#;

const STANDARD_ITEM_FOIL_FRAGMENT: &str = r#"#version 450
layout(set = 0, binding = 2) uniform texture2D Tex0;
layout(set = 0, binding = 3) uniform sampler Samp0;
layout(location = 0) in vec2 v_uv;
layout(location = 1) in vec4 v_color;
layout(location = 9) in vec2 v_fog_distances;
layout(location = 10) flat in vec4 v_fog_color_and_environmental_start;
layout(location = 11) flat in vec4 v_fog_ranges;
layout(location = 13) flat in float v_foil_strength;
layout(location = 0) out vec4 out_color;
float foil_fog_value(float distance, float start, float end) {
    // Frozen OpenGL fog.glsl sentinel/degenerate-range behavior.
    if (distance != distance || abs(distance) >= 1.0e12
        || max(abs(start), abs(end)) >= 1.0e12) return 0.0;
    if (end <= start) return distance > start ? 1.0 : 0.0;
    if (distance <= start) return 0.0;
    if (distance >= end) return 1.0;
    return clamp((distance - start) / (end - start), 0.0, 1.0);
}
void main() {
    vec4 color = texture(sampler2D(Tex0, Samp0), v_uv) * v_color;
    if (color.a < 0.1) discard;
    float fog = max(
        foil_fog_value(v_fog_distances.x, v_fog_color_and_environmental_start.w, v_fog_ranges.x),
        foil_fog_value(v_fog_distances.y, v_fog_ranges.y, v_fog_ranges.z));
    // Foil fades to black; ordinary fog-color mixing is incorrect. Neither
    // strength nor fog changes texture alpha or discard coverage.
    out_color = vec4(color.rgb * ((1.0 - fog) * v_foil_strength), color.a);
}
"#;

pub const MINIMAL_TERRAIN_MATERIAL_FRAGMENT_DIRECT: &str = r#"#version 450
layout(set = 0, binding = 2) uniform texture2D Tex0;
layout(set = 0, binding = 3) uniform sampler Samp0;
layout(location = 0) in vec2 v_uv;
layout(location = 1) in vec4 v_color;
layout(location = 2) flat in vec4 v_material;
layout(location = 6) flat in vec4 v_animation_region;
layout(location = 7) flat in vec4 v_animation_next_region;
layout(location = 8) flat in vec4 v_overlay_color;
layout(location = 9) in vec2 v_fog_distances;
layout(location = 10) flat in vec4 v_fog_color_and_environmental_start;
layout(location = 11) flat in vec4 v_fog_ranges;
layout(location = 12) flat in uint v_terrain_material_bits;
layout(location = 0) out vec4 out_color;
// Frozen Java OpenGL reported GL_MAX_TEXTURE_LOD_BIAS = 15 for the paired
// baseline device. Keep Sodium's derivative-selected negative-bias sample;
// forcing level zero is not equivalent for distant terrain.
const float FROZEN_MAX_TEXTURE_LOD_BIAS = 15.0;
const float FROZEN_FOG_DISABLED_SENTINEL_THRESHOLD = 1.0e12;
bool frozen_fog_range_disabled(float start, float end) {
    return max(abs(start), abs(end)) >= FROZEN_FOG_DISABLED_SENTINEL_THRESHOLD;
}
bool frozen_fog_distance_invalid(float distance) {
    return distance != distance || abs(distance) >= FROZEN_FOG_DISABLED_SENTINEL_THRESHOLD;
}
float frozen_linear_fog_value(float distance, float start, float end) {
    if (frozen_fog_distance_invalid(distance) || frozen_fog_range_disabled(start, end)) {
        return 0.0;
    }
    if (end <= start) {
        return distance > start ? 1.0 : 0.0;
    }
    if (distance <= start) {
        return 0.0;
    }
    if (distance >= end) {
        return 1.0;
    }
    return clamp((distance - start) / (end - start), 0.0, 1.0);
}
void main() {
    vec2 sample_uv = v_animation_region.xy + v_uv * v_animation_region.zw;
    // Frozen Sodium's compact material byte controls both this exact choice
    // and alpha cutoff. A non-mipped material asks for a negative LOD bias
    // while retaining derivative-selected mip sampling.
#ifdef VULKANIC_MODEL_TRANSLUCENT_CUTOUT
    vec4 color = texture(sampler2D(Tex0, Samp0), sample_uv);
#else
    bool use_mipmaps = (v_terrain_material_bits & 1u) != 0u;
    vec4 color = use_mipmaps
        ? texture(sampler2D(Tex0, Samp0), sample_uv)
        : texture(sampler2D(Tex0, Samp0), sample_uv, -FROZEN_MAX_TEXTURE_LOD_BIAS);
#endif
    if (v_material.y > 0.5) {
        vec2 next_uv = v_animation_next_region.xy + v_uv * v_animation_next_region.zw;
        vec4 next_color = texture(sampler2D(Tex0, Samp0), next_uv);
        color = mix(color, next_color, clamp(v_material.z, 0.0, 1.0));
    }
    color *= v_color;
    if (v_overlay_color.a > 0.0) {
        color.rgb = mix(color.rgb, v_overlay_color.rgb, v_overlay_color.a);
    }
#ifdef VULKANIC_MODEL_TRANSLUCENT_CUTOUT
    if (color.a < 0.1) discard;
#endif
    uint alpha_cutoff_class = (v_terrain_material_bits >> 1u) & 3u;
    float alpha_cutoff = float[4](0.0, 0.1, 0.1, 1.0)[alpha_cutoff_class];
#ifdef VULKANIC_TERRAIN_FRAGMENT_DISCARD
    // Frozen Sodium's `block_layer_*.fsh` applies only the compact material
    // alpha cutoff here. Cull state belongs to the explicit pass pipeline;
    // a fragment-side back-face discard would remove valid cutout geometry
    // (for example foliage planes) that the baseline still draws.
    if (alpha_cutoff > 0.0 && color.a < alpha_cutoff) {
        discard;
    }
#endif
    float fog_value = max(
        frozen_linear_fog_value(v_fog_distances.x, v_fog_color_and_environmental_start.w, v_fog_ranges.x),
        frozen_linear_fog_value(v_fog_distances.y, v_fog_ranges.y, v_fog_ranges.z)
    );
    float fog_alpha = clamp(fog_value * v_fog_ranges.w, 0.0, 1.0);
    color.rgb = mix(color.rgb, v_fog_color_and_environmental_start.rgb, fog_alpha);
    out_color = color;
}
"#;

pub const MINIMAL_G_BUFFER_COMPOSITE_VERTEX: &str = MINIMAL_FULLSCREEN_VERTEX;

pub const MINIMAL_FULLSCREEN_VERTEX: &str = r#"#version 450
layout(location = 0) out vec2 v_uv;
void main() {
    vec2 positions[3] = vec2[](
        vec2(-1.0, -1.0),
        vec2( 3.0, -1.0),
        vec2(-1.0,  3.0)
    );
    vec2 uvs[3] = vec2[](
        vec2(0.0, 0.0),
        vec2(2.0, 0.0),
        vec2(0.0, 2.0)
    );
    gl_Position = vec4(positions[gl_VertexIndex], 0.0, 1.0);
    v_uv = uvs[gl_VertexIndex];
#ifdef VULKANIC_GAL_FLIP_FULLSCREEN_UV_Y
    v_uv.y = 1.0 - v_uv.y;
#endif
}
"#;

pub const MINIMAL_G_BUFFER_COMPOSITE_FRAGMENT: &str = MINIMAL_DEFERRED_LIGHTING_FRAGMENT;

pub const MINIMAL_DEFERRED_LIGHTING_FRAGMENT: &str = r#"#version 450
layout(set = 0, binding = 0) uniform texture2D AlbedoTex;
layout(set = 0, binding = 1) uniform texture2D NormalTex;
layout(set = 0, binding = 2) uniform texture2D MaterialLightTex;
layout(set = 0, binding = 3) uniform texture2D WorldPositionTex;
layout(set = 0, binding = 4) uniform texture2D ShadowDepthTex;
layout(set = 0, binding = 5) uniform sampler Samp0;
layout(set = 0, binding = 6, std430) readonly buffer ShaderCompositeUniforms {
    mat4 light_view_projection;
    vec4 shadow_params;
    vec4 color_grade_params;
    mat4 projection_inverse;
    vec4 fog_color_and_environmental_start;
    vec4 fog_ranges;
};
layout(location = 0) in vec2 v_uv;
layout(location = 0) out vec4 out_color;
void main() {
    vec4 albedo = texture(sampler2D(AlbedoTex, Samp0), v_uv);
    vec4 material_light = texture(sampler2D(MaterialLightTex, Samp0), v_uv);
    if (material_light.a < 0.5) {
        out_color = vec4(albedo.rgb, 0.0);
        return;
    }
    vec4 packed_world = texture(sampler2D(WorldPositionTex, Samp0), v_uv);
    float shadow_range = max(shadow_params.w, 1.0);
    vec3 world_position = (packed_world.xyz * 2.0 - 1.0) * shadow_range;
    vec4 light_clip = light_view_projection * vec4(world_position, 1.0);
    vec3 light_ndc = light_clip.xyz / max(abs(light_clip.w), 0.0001);
    vec2 shadow_uv = light_ndc.xy * 0.5 + 0.5;
    float shadow_factor = 1.0;
    if (shadow_params.x > 0.5
            && shadow_uv.x >= 0.0 && shadow_uv.x <= 1.0
            && shadow_uv.y >= 0.0 && shadow_uv.y <= 1.0) {
        float shadow_depth = texture(sampler2D(ShadowDepthTex, Samp0), shadow_uv).r;
        float compare_depth = light_ndc.z * 0.5 + 0.5;
        float bias = shadow_params.y;
        shadow_factor = compare_depth - bias > shadow_depth ? shadow_params.z : 1.0;
    }
    // Terrain material passes write terrain_lit_color, not raw base color.
    // Their vertex color has already applied Minecraft's per-face shade and
    // packed block/sky-light factor. Relighting it here darkens horizontal
    // faces a second time and makes correct atlas regions look like unrelated
    // dark materials. Keep this pass limited to the explicit shadow dependency
    // until a source-derived deferred-lighting contract replaces it.
    out_color = vec4(albedo.rgb * shadow_factor, albedo.a);
}
"#;

pub const MINIMAL_COMPOSITE_COLOR_GRADE_FRAGMENT: &str = r#"#version 450
layout(set = 0, binding = 0) uniform texture2D Tex0;
layout(set = 0, binding = 5) uniform sampler Samp0;
layout(set = 0, binding = 6, std430) readonly buffer ShaderCompositeUniforms {
    mat4 light_view_projection;
    vec4 shadow_params;
    vec4 color_grade_params;
    mat4 projection_inverse;
    vec4 fog_color_and_environmental_start;
    vec4 fog_ranges;
};
layout(location = 0) in vec2 v_uv;
layout(location = 0) out vec4 out_color;
void main() {
    vec4 color = texture(sampler2D(Tex0, Samp0), v_uv);
    if (color.a < 0.5) {
        out_color = color;
        return;
    }
    float exposure = color_grade_params.x;
    vec3 lift = vec3(color_grade_params.y);
    vec3 graded = pow(max(color.rgb * exposure + lift, vec3(0.0)), vec3(color_grade_params.z));
    out_color = vec4(graded, color.a);
}
"#;

pub const MINIMAL_COMPOSITE_DEPTH_FOG_FRAGMENT: &str = r#"#version 450
layout(set = 0, binding = 0) uniform texture2D Tex0;
layout(set = 0, binding = 1) uniform texture2D NormalTex;
layout(set = 0, binding = 4) uniform texture2D MainDepthTex;
layout(set = 0, binding = 5) uniform sampler Samp0;
layout(set = 0, binding = 6, std430) readonly buffer ShaderCompositeUniforms {
    mat4 light_view_projection;
    vec4 shadow_params;
    vec4 color_grade_params;
    mat4 projection_inverse;
    vec4 fog_color_and_environmental_start;
    vec4 fog_ranges;
};
layout(location = 0) in vec2 v_uv;
layout(location = 0) out vec4 out_color;
void main() {
    vec4 color = texture(sampler2D(Tex0, Samp0), v_uv);
    float device_depth = texture(sampler2D(MainDepthTex, Samp0), v_uv).r;
    // `v_uv` deliberately addresses Vulkan attachments with the opposite row
    // origin from the fullscreen raster position.  It is therefore correct
    // for sampling, but not for rebuilding the game's view-space ray: that
    // reconstruction must use the unflipped clip-space Y coordinate.
    vec2 reconstruction_uv = v_uv;
#ifdef VULKANIC_GAL_FLIP_FULLSCREEN_UV_Y
    reconstruction_uv.y = 1.0 - reconstruction_uv.y;
#endif
    // Deferred work preserves Frozen's interpolated vertex fog factor in the
    // normal alpha channel. Do not reconstruct a different distance from
    // depth: perspective reconstruction changes the fog ramp over terrain
    // and translucent triangles.
    float fog_factor = texture(sampler2D(NormalTex, Samp0), v_uv).a;
    // `color.a` is not a fog-opacity channel. Sodium's compact terrain
    // vertices use it for separate ambient occlusion, and the deferred
    // G-buffer must preserve that semantic for lighting. Frozen resolves fog
    // with the copied fog-color alpha instead.
    float fog_alpha = clamp(fog_factor * fog_ranges.w, 0.0, 1.0);
    out_color = vec4(mix(color.rgb, fog_color_and_environmental_start.rgb, fog_alpha), color.a);
}
"#;

pub const MINIMAL_FINAL_COPY_FRAGMENT: &str = r#"#version 450
layout(set = 0, binding = 0) uniform texture2D Tex0;
layout(set = 0, binding = 5) uniform sampler Samp0;
layout(set = 0, binding = 6, std430) readonly buffer ShaderCompositeUniforms {
    mat4 light_view_projection;
    vec4 shadow_params;
    vec4 color_grade_params;
    mat4 projection_inverse;
    vec4 fog_color_and_environmental_start;
    vec4 fog_ranges;
};
layout(location = 0) in vec2 v_uv;
layout(location = 0) out vec4 out_color;
void main() {
    vec4 color = texture(sampler2D(Tex0, Samp0), v_uv);
    out_color = vec4(color.rgb + vec3(shadow_params.x * 0.0), color.a);
}
"#;

pub const MINIMAL_SHADOW_DEPTH_VERTEX: &str = r#"#version 450
struct MeshVertex {
    vec4 position_uv;
    vec4 color_uv;
    vec4 normal_light;
    vec4 extra_data;
    vec4 shader_data;
};
layout(set = 0, binding = 0, std430) readonly buffer WorldMeshVertices {
    MeshVertex vertices[];
};
struct MeshInstance {
    mat4 model;
    vec4 color;
    vec4 material;
    vec4 animation_region;
    vec4 animation_next_region;
    vec4 overlay_color;
};
layout(set = 0, binding = 1, std430) readonly buffer WorldMeshInstances {
    mat4 view;
    mat4 projection;
    mat4 light_view_projection;
    vec4 shadow_params;
    MeshInstance instances[];
};
layout(location = 0) out vec2 v_uv;
layout(location = 1) out vec4 v_color;
layout(location = 2) flat out vec4 v_material;
layout(location = 6) flat out vec4 v_animation_region;
layout(location = 7) flat out vec4 v_animation_next_region;
void main() {
    MeshVertex vertex = vertices[gl_VertexIndex];
    MeshInstance instance = instances[gl_InstanceIndex];
    vec4 clip = light_view_projection * instance.model * vec4(vertex.position_uv.xyz, 1.0);
#ifdef VULKANIC_GAL_ZERO_TO_ONE_CLIP_DEPTH
    clip.z = clip.z * 0.5 + clip.w * 0.5;
#endif
    gl_Position = clip;
    v_uv = vec2(vertex.position_uv.w, vertex.color_uv.w);
    v_color = vec4(vertex.color_uv.rgb * vertex.normal_light.x, vertex.normal_light.w) * instance.color;
    v_material = instance.material;
    v_animation_region = instance.animation_region;
    v_animation_next_region = instance.animation_next_region;
}
"#;

pub const MINIMAL_SHADOW_DEPTH_FRAGMENT: &str = r#"#version 450
layout(set = 0, binding = 2) uniform texture2D Tex0;
layout(set = 0, binding = 3) uniform sampler Samp0;
layout(location = 0) in vec2 v_uv;
layout(location = 1) in vec4 v_color;
layout(location = 2) flat in vec4 v_material;
layout(location = 6) flat in vec4 v_animation_region;
layout(location = 7) flat in vec4 v_animation_next_region;
// The shadow target carries explicit color attachments as part of the
// backend-neutral pass contract. Keep those writes explicit even though the
// depth result is the semantic output consumed by lighting.
layout(location = 0) out vec4 out_shadow_color;
layout(location = 1) out vec4 out_light_shaft;
void main() {
    vec2 sample_uv = v_animation_region.xy + v_uv * v_animation_region.zw;
    vec4 color = texture(sampler2D(Tex0, Samp0), sample_uv);
    if (v_material.y > 0.5) {
        vec2 next_uv = v_animation_next_region.xy + v_uv * v_animation_next_region.zw;
        vec4 next_color = texture(sampler2D(Tex0, Samp0), next_uv);
        color = mix(color, next_color, clamp(v_material.z, 0.0, 1.0));
    }
    color *= v_color;
    if (v_material.x > 0.0 && color.a < v_material.x) {
        discard;
    }
    out_shadow_color = vec4(0.0);
    out_light_shaft = vec4(0.0);
    gl_FragDepth = gl_FragCoord.z;
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::vulkanic::handles::{Handle, HandleKind};
    use crate::render::vulkanic::shader_pack::cloud_contract::{
        derive_cloud_pass_contract, lower_cloud_source_pair,
    };
    use crate::render::vulkanic::shader_pack::distant_horizons_contract::{
        derive_distant_horizons_opaque_contract, derive_distant_horizons_translucent_contract,
        DistantHorizonsPassKind,
    };
    use crate::render::vulkanic::shader_pack::entity_contract::{
        bind_entity_source_resources, derive_entity_contract, lower_entity_source_pair,
    };
    use crate::render::vulkanic::shader_pack::lowering::{
        lower_distant_horizons_source_pair, lower_fullscreen_source_pair, lower_shadow_source_pair,
        lower_terrain_source_pair, lower_translucent_terrain_source_pair,
    };
    use crate::render::vulkanic::shader_pack::preprocess::{
        complete_bundled_pack_source_for_test, preprocess_distant_horizons_sources,
        preprocess_source_stage_pair, preprocess_terrain_sources,
    };
    use crate::render::vulkanic::shader_pack::source::{ShaderPackSource, ShaderSourceFile};
    use crate::render::vulkanic::shader_pack::terrain_contract::{
        derive_complementary_terrain_contract, derive_complementary_translucent_terrain_contract,
        TerrainProgramScope, TerrainSourceStage, TerrainSourceStages, TerrainTranslucentBlend,
    };
    use crate::render::vulkanic::shader_pack::terrain_source_resources::{
        TerrainSourceOwnedResource, TerrainSourceOwnedResourceSet,
        TerrainSourceResourceAvailability, TerrainSourceResourceAvailabilitySet,
        TerrainSourceResourceBindings, TerrainSourceResourceRole,
        TerrainSourceSampledResourceShape, TERRAIN_RESOURCE_BINDINGS_PATH,
    };

    #[test]
    fn entity_outline_shader_contract_is_solid_color_and_preserves_alpha() {
        assert!(MINIMAL_ENTITY_OUTLINE_VERTEX.contains("WorldMeshVertices"));
        assert!(MINIMAL_ENTITY_OUTLINE_VERTEX.contains("v_outline_color = instance.color"));
        assert!(MINIMAL_ENTITY_OUTLINE_FRAGMENT.contains("out_outline_color = v_outline_color"));
        assert!(!MINIMAL_ENTITY_OUTLINE_FRAGMENT.contains("sampler2D"));
    }

    #[test]
    fn entity_outline_fullscreen_contracts_use_explicit_vulkan_bindings() {
        assert!(MINIMAL_ENTITY_OUTLINE_FULLSCREEN_VERTEX.contains("gl_VertexIndex"));
        assert!(MINIMAL_ENTITY_OUTLINE_SOBEL_FRAGMENT.contains("binding = 0"));
        assert!(MINIMAL_ENTITY_OUTLINE_SOBEL_FRAGMENT.contains("textureSize"));
        assert!(MINIMAL_ENTITY_OUTLINE_BLUR_FRAGMENT.contains("binding = 2"));
        assert!(MINIMAL_ENTITY_OUTLINE_BLIT_FRAGMENT.contains("ColorModulate"));
        for source in [
            MINIMAL_ENTITY_OUTLINE_FULLSCREEN_VERTEX,
            MINIMAL_ENTITY_OUTLINE_SOBEL_FRAGMENT,
            MINIMAL_ENTITY_OUTLINE_BLUR_FRAGMENT,
            MINIMAL_ENTITY_OUTLINE_BLIT_FRAGMENT,
        ] {
            assert!(source.starts_with("#version 450"));
        }
    }

    #[test]
    fn minimal_distant_horizons_streams_keep_compact_micro_y_non_positional() {
        for source in [
            MINIMAL_DISTANT_HORIZONS_LOD_OPAQUE_VERTEX,
            MINIMAL_DISTANT_HORIZONS_LOD_EXACT_ATLAS_OPAQUE_VERTEX,
        ] {
            assert!(source.contains("vec3(vertex.micro_x, 0.0, vertex.micro_z)"));
            assert!(
                !source.contains("vec3(vertex.micro_x, vertex.micro_y, vertex.micro_z)"),
                "DH micro_y is payload metadata, not a terrain height offset"
            );
        }
    }

    #[test]
    fn distant_horizons_streams_do_not_apply_the_dimension_world_y_offset_twice() {
        for source in [
            MINIMAL_DISTANT_HORIZONS_LOD_OPAQUE_VERTEX,
            MINIMAL_DISTANT_HORIZONS_LOD_EXACT_ATLAS_OPAQUE_VERTEX,
        ] {
            assert!(
                !source.contains("+ vec3(0.0, column_origin_and_world_y.w, 0.0)"),
                "the column origin already contains DH's min-world-Y"
            );
        }
    }

    fn paired_source() -> ShaderPackSource {
        ShaderPackSource::new(
            "lowered-source-test",
            9,
            vec![
                ShaderSourceFile::new(
                    "gbuffers_terrain.vsh",
                    "#version 130\nout vec2 texCoord;\nout vec4 glColor;\nout float smoothnessD;\nout float materialMask;\nout float skyLightFactor;\nuniform sampler2D tex;\nvoid main() { texCoord = gl_MultiTexCoord0.xy; glColor = vec4(1.0); smoothnessD = 0.0; materialMask = 0.0; skyLightFactor = 1.0; gl_Position = ftransform(); }",
                ),
                ShaderSourceFile::new(
                    "gbuffers_terrain.fsh",
                    "#version 130\nin vec2 texCoord;\nin vec4 glColor;\nin float smoothnessD;\nin float materialMask;\nin float skyLightFactor;\nuniform sampler2D tex;\nvoid DoLighting() {}\n/* DRAWBUFFERS:06 */\nvoid main() { vec4 color = texture2D(tex, texCoord); if (color.a <= 0.00001) discard; color.rgb *= glColor.rgb; DoLighting(); gl_FragData[0] = color; gl_FragData[1] = vec4(smoothnessD, materialMask, skyLightFactor, 1.0); }",
                ),
                ShaderSourceFile::new("lib/common.glsl", "#define TEST 1\n"),
                ShaderSourceFile::new("shaders.properties", ""),
                ShaderSourceFile::new("block.properties", ""),
                ShaderSourceFile::new(TERRAIN_RESOURCE_BINDINGS_PATH, "tex=material_atlas\n"),
            ],
        )
        .unwrap()
    }

    fn paired_entity_source() -> ShaderPackSource {
        ShaderPackSource::new(
            "lowered-entity-source-test",
            11,
            vec![
                ShaderSourceFile::new(
                    "world0/gbuffers_entities.vsh",
                    "#version 130\nvoid main() { vec4 p = gl_Vertex; vec2 light = GetLightMapCoordinates(); vec3 normal = gl_Normal; vec4 color = gl_Color; gl_Position = ftransform(); }",
                ),
                ShaderSourceFile::new(
                    "world0/gbuffers_entities.fsh",
                    "#version 130\nuniform sampler2D tex;\nuniform int entityId;\nuniform vec4 entityColor;\nvoid DoLighting() {}\nvoid main() { vec4 color = texture2D(tex, texCoord); color *= glColor; color.rgb = mix(color.rgb, entityColor.rgb, entityColor.a); DoLighting(); gl_FragData[0] = color; gl_FragData[1] = color; /* DRAWBUFFERS:06 */ }",
                ),
                ShaderSourceFile::new("entity.properties", "entity.50076=boat\n"),
                ShaderSourceFile::new(TERRAIN_RESOURCE_BINDINGS_PATH, "tex=material_atlas\n"),
            ],
        )
        .unwrap()
    }

    fn paired_shadow_source() -> ShaderPackSource {
        ShaderPackSource::new(
            "lowered-shadow-source-test",
            10,
            vec![
                ShaderSourceFile::new(
                    "shadow.vsh",
                    "#version 130\nout vec2 texCoord;\nvoid main() { texCoord = gl_MultiTexCoord0.xy; gl_Position = ftransform(); }",
                ),
                ShaderSourceFile::new(
                    "shadow.fsh",
                    "#version 130\nin vec2 texCoord;\nuniform sampler2D tex;\nvoid main() { gl_FragData[0] = texture2D(tex, texCoord); }",
                ),
                ShaderSourceFile::new(TERRAIN_RESOURCE_BINDINGS_PATH, "tex=material_atlas\n"),
            ],
        )
        .unwrap()
    }

    fn paired_translucent_source() -> ShaderPackSource {
        ShaderPackSource::new(
            "lowered-translucent-source-test",
            12,
            vec![
                ShaderSourceFile::new(
                    "gbuffers_water.vsh",
                    "#version 130\nout vec2 texCoord;\nout vec4 glColor;\nvoid main() { texCoord = gl_MultiTexCoord0.xy; glColor = vec4(1.0); gl_Position = ftransform(); }",
                ),
                ShaderSourceFile::new(
                    "gbuffers_water.fsh",
                    "#version 130\nin vec2 texCoord;\nin vec4 glColor;\nuniform sampler2D tex;\nvoid DoLighting() {}\nvoid DoFog(inout vec3 color, inout float sky, float distance, vec3 player, float up, float sun, float dither) {}\n/* DRAWBUFFERS:03 */\nvoid main() { vec4 colorP = texture2D(tex, texCoord); vec4 color = colorP * vec4(glColor.rgb, 1.0); vec3 viewPos = vec3(0.0); vec3 playerPos = vec3(0.0); float lViewPos = 0.0; float VdotU = 0.0; float VdotS = 0.0; float dither = 0.0; vec4 translucentMult = vec4(1.0); DoLighting(); float sky = 0.0; DoFog(color.rgb, sky, lViewPos, playerPos, VdotU, VdotS, dither); gl_FragData[0] = color; gl_FragData[1] = vec4(1.0 - translucentMult.rgb, translucentMult.a); }",
                ),
                ShaderSourceFile::new("lib/common.glsl", "#define WATER_REFLECT_QUALITY -1\n"),
                ShaderSourceFile::new(
                    "shaders.properties",
                    "alphaTest.gbuffers_water=GREATER 0.0001\nblend.gbuffers_water=SRC_ALPHA ONE_MINUS_SRC_ALPHA ONE ONE_MINUS_SRC_ALPHA\n",
                ),
                ShaderSourceFile::new("block.properties", "block.32000=minecraft:water\n"),
                ShaderSourceFile::new(TERRAIN_RESOURCE_BINDINGS_PATH, "tex=material_atlas\n"),
            ],
        )
        .unwrap()
    }

    fn paired_cloud_source() -> ShaderPackSource {
        ShaderPackSource::new(
            "lowered-cloud-source-test",
            13,
            vec![
                ShaderSourceFile::new(
                    "world0/gbuffers_clouds.vsh",
                    "#version 130\nout vec2 texCoord;\nout vec4 glColor;\nvoid main() { texCoord = (gl_TextureMatrix[0] * gl_MultiTexCoord0).xy; glColor = gl_Color; gl_Position = gbufferModelView * gl_Vertex; }",
                ),
                ShaderSourceFile::new(
                    "world0/gbuffers_clouds.fsh",
                    "#version 130\nin vec2 texCoord;\nin vec4 glColor;\nuniform sampler2D tex;\n/* DRAWBUFFERS:063 */\nvoid main() { vec4 color = texture2D(tex, texCoord) * glColor; gl_FragData[0] = color; gl_FragData[1] = vec4(0.0); gl_FragData[2] = vec4(1.0); }",
                ),
                ShaderSourceFile::new(TERRAIN_RESOURCE_BINDINGS_PATH, "tex=material_atlas\n"),
            ],
        )
        .unwrap()
    }

    #[test]
    fn minimal_deferred_pass_does_not_relight_already_lit_terrain_color() {
        // The terrain material contract names attachment zero terrain_lit_color:
        // Java/Sodium has already supplied its directional shade and packed
        // light through the vertex color. Applying another synthetic normal
        // term here made valid atlas regions appear as the wrong dark block.
        assert!(MINIMAL_DEFERRED_LIGHTING_FRAGMENT
            .contains("out_color = vec4(albedo.rgb * shadow_factor, albedo.a);"));
        assert!(!MINIMAL_DEFERRED_LIGHTING_FRAGMENT.contains("float face ="));
        assert!(!MINIMAL_DEFERRED_LIGHTING_FRAGMENT.contains("float light ="));
    }

    #[test]
    fn direct_terrain_vertex_has_an_explicit_modelpart_diffuse_branch() {
        assert!(MINIMAL_TERRAIN_MATERIAL_VERTEX.contains("(material_semantics & 2u) != 0u"));
        assert!(MINIMAL_TERRAIN_MATERIAL_VERTEX.contains("LIGHT0_DIRECTION"));
        assert!(MINIMAL_TERRAIN_MATERIAL_VERTEX.contains("NETHER_LIGHT1_DIRECTION"));
        assert!(MINIMAL_TERRAIN_MATERIAL_VERTEX.contains("v_color.rgb *= diffuse"));
        assert!(MINIMAL_TERRAIN_MATERIAL_VERTEX.contains("(material_semantics & 1u) != 0u"));
        assert!(MINIMAL_TERRAIN_MATERIAL_VERTEX.contains("transpose(inverse(mat3(instance.model)))"));
    }

    #[test]
    fn vanilla_fog_uses_sodium_cylindrical_distance_once_after_deferred_lighting() {
        // Frozen Java OpenGL's Sodium shader resolves its fog factor from the
        // interpolated spherical/cylindrical vertex distances before the
        // model-view transform. The deferred Rust route transports that
        // immutable factor through the explicit G-buffer and applies it once
        // after deferred lighting.
        assert!(MINIMAL_TERRAIN_MATERIAL_VERTEX
            .contains("vec3 fog_position = world.xyz;"));
        assert!(MINIMAL_TERRAIN_MATERIAL_VERTEX.contains("v_fog_distances = vec2("));
        assert!(MINIMAL_TERRAIN_MATERIAL_VERTEX.contains(
            "max(length(fog_position.xz), abs(fog_position.y))"
        ));
        assert!(!MINIMAL_TERRAIN_MATERIAL_VERTEX.contains("view_position"));
        assert!(!MINIMAL_TERRAIN_MATERIAL_FRAGMENT.contains("v_fog_factor"));
        assert!(MINIMAL_TERRAIN_MATERIAL_FRAGMENT_DIRECT.contains(
            "frozen_linear_fog_value(v_fog_distances.x, v_fog_color_and_environmental_start.w, v_fog_ranges.x)"
        ));
        assert!(MINIMAL_TERRAIN_MATERIAL_FRAGMENT_DIRECT.contains(
            "frozen_linear_fog_value(v_fog_distances.y, v_fog_ranges.y, v_fog_ranges.z)"
        ));
        assert!(MINIMAL_TERRAIN_MATERIAL_FRAGMENT_DIRECT.contains(
            "clamp(fog_value * v_fog_ranges.w, 0.0, 1.0)"
        ));
        assert!(!MINIMAL_TERRAIN_MATERIAL_FRAGMENT_DIRECT.contains("v_fog_factor"));
        for source in [
            MINIMAL_TERRAIN_MATERIAL_FRAGMENT,
            MINIMAL_TERRAIN_MATERIAL_FRAGMENT_DIRECT,
        ] {
            assert!(source.contains("const float FROZEN_FOG_DISABLED_SENTINEL_THRESHOLD = 1.0e12;"));
            assert!(source.contains("distance != distance"));
            assert!(source.contains("return distance > start ? 1.0 : 0.0;"));
            assert!(source.contains("frozen_fog_range_disabled(start, end)"));
        }
        assert!(MINIMAL_TERRAIN_MATERIAL_FRAGMENT
            .contains("out_terrain_view_space_normal = vec4(n, max(environmental_fog, render_distance_fog));"));
        assert!(MINIMAL_COMPOSITE_DEPTH_FOG_FRAGMENT
            .contains("uniform texture2D NormalTex"));
        assert!(MINIMAL_COMPOSITE_DEPTH_FOG_FRAGMENT
            .contains("texture(sampler2D(NormalTex, Samp0), v_uv).a"));
        assert!(MINIMAL_COMPOSITE_DEPTH_FOG_FRAGMENT
            .contains("clamp(fog_factor * fog_ranges.w, 0.0, 1.0)"));
        assert!(!MINIMAL_COMPOSITE_DEPTH_FOG_FRAGMENT
            .contains("fog_factor * color.a"));
    }

    #[test]
    fn direct_terrain_preserves_the_explicit_atlas_or_local_uv_semantic() {
        // Atlas coordinates are independent of diffuse and lightmap semantics.
        assert!(MINIMAL_TERRAIN_MATERIAL_VERTEX
            .contains("(material_semantics & 1u) != 0u\n        ? vertex.shader_data.xy"));
        assert!(MINIMAL_TERRAIN_MATERIAL_VERTEX
            .contains(": vec2(vertex.position_uv.w, vertex.color_uv.w);"));
        assert!(!MINIMAL_TERRAIN_MATERIAL_VERTEX
            .contains("instance.material.w > 0.5"));
    }

    #[test]
    fn model_lightmap_contracts_do_not_reuse_sodium_boundary_sampling() {
        let source = minimal_direct_terrain_vertex_source();
        assert!(source.contains("(material_semantics & 8u) != 0u"));
        assert!(source.contains("light_uv += vec2(0.5 / 16.0)"));
        assert!(source.contains("(material_semantics & 16u) != 0u"));
        assert!(source.contains("texelFetch(sampler2D(LightmapTexture, LightmapSampler), light_texel, 0)"));
    }

    #[test]
    fn builtin_material_programs_sample_the_explicit_vanilla_lightmap() {
        for source in [MINIMAL_TERRAIN_MATERIAL_VERTEX] {
            assert!(
                source.contains("layout(set = 1, binding = 0) uniform texture2D LightmapTexture")
            );
            assert!(source.contains("layout(set = 1, binding = 1) uniform sampler LightmapSampler"));
            assert!(source.contains(
                "clamp(vertex.extra_data.xy, vec2(0.0), vec2(255.0 / 240.0)) * (15.0 / 16.0);"
            ));
            assert!(source.contains("texture(sampler2D(LightmapTexture, LightmapSampler), light_uv)"));
        }
        assert!(!MINIMAL_TERRAIN_MATERIAL_FRAGMENT.contains("LightmapTexture"));
    }

    #[test]
    fn direct_vanilla_terrain_matches_frozen_sodium_lightmap_vertex_sampling() {
        let vertex = minimal_direct_terrain_vertex_source();
        // Frozen OpenGL Sodium `block_layer_opaque.vsh` samples its linear
        // LightTexture directly at `a_LightAndData.xy / 256`. The semantic
        // stream holds each full byte as byte / 240, preserving fractional
        // levels as well as the usual level * 16 coordinates.
        // Sampling here—not
        // the fragment—
        // also preserves Frozen's interpolation of already-lit vertex color.
        assert!(vertex.contains("layout(set = 1, binding = 0) uniform texture2D LightmapTexture"));
        assert!(vertex.contains(
            "clamp(vertex.extra_data.xy, vec2(0.0), vec2(255.0 / 240.0)) * (15.0 / 16.0);"
        ));
        assert!(vertex.contains("v_color = vec4(vertex.color_uv.rgb, vertex.normal_light.w) * instance.color"));
        assert!(vertex.contains("texture(sampler2D(LightmapTexture, LightmapSampler), light_uv)"));
        // Frozen carries the blend alpha independently in `a_Color`; it never
        // multiplies the AO-baked RGB vertex color by that alpha or derives it
        // from packed light.
        assert!(vertex.contains("vec4(vertex.color_uv.rgb, vertex.normal_light.w)"));
        assert!(!vertex.contains("vertex.color_uv.rgb * vertex.normal_light.w"));
        assert!(!MINIMAL_TERRAIN_MATERIAL_FRAGMENT_DIRECT.contains("LightmapTexture"));
        assert!(!MINIMAL_TERRAIN_MATERIAL_FRAGMENT_DIRECT.contains("v_light"));
    }

    #[test]
    fn builtin_terrain_paths_preserve_sodium_mip_and_cutout_material_bits() {
        assert!(MINIMAL_TERRAIN_MATERIAL_VERTEX
            .contains("v_terrain_material_bits = uint(clamp(vertex.extra_data.w, 0.0, 255.0));"));
        for fragment in [
            MINIMAL_TERRAIN_MATERIAL_FRAGMENT,
            MINIMAL_TERRAIN_MATERIAL_FRAGMENT_DIRECT,
        ] {
            assert!(fragment.contains("bool use_mipmaps = (v_terrain_material_bits & 1u) != 0u;"));
            assert!(fragment.contains("const float FROZEN_MAX_TEXTURE_LOD_BIAS = 15.0;"));
            assert!(fragment.contains(
                "texture(sampler2D(Tex0, Samp0), sample_uv, -FROZEN_MAX_TEXTURE_LOD_BIAS)"
            ));
            assert!(!fragment.contains("textureLod(sampler2D(Tex0, Samp0), sample_uv, 0.0)"));
            assert!(fragment.contains("uint alpha_cutoff_class = (v_terrain_material_bits >> 1u) & 3u;"));
            // Frozen Sodium leaves face selection to the raster pipeline and
            // performs no fragment-side `gl_FrontFacing` discard.
            assert!(!fragment.contains("gl_FrontFacing"));
        }
    }

    #[test]
    fn deferred_opaque_coverage_does_not_reinterpret_compact_vertex_alpha_as_visibility() {
        // Sodium stores baked AO in compact vertex alpha for the normal
        // terrain stream. The forward translucent route needs that alpha;
        // the deferred opaque/cutout route instead has a binary post-discard
        // coverage contract for its lighting pass.
        assert!(MINIMAL_TERRAIN_MATERIAL_FRAGMENT.contains(
            "out_terrain_material_auxiliary = vec4(v_material.x, v_light.x, v_light.y, 1.0);"
        ));
        assert!(MINIMAL_TERRAIN_MATERIAL_FRAGMENT
            .contains("out_world_position = vec4(v_world_position, 1.0);"));
        assert!(MINIMAL_DEFERRED_LIGHTING_FRAGMENT.contains("if (material_light.a < 0.5)"));
    }

    #[test]
    fn direct_translucent_terrain_uses_the_single_target_forward_contract() {
        let program = minimal_direct_terrain_translucent_program();
        assert_eq!(
            ProgramIdentity::new("vulkanic:builtin/direct_terrain_translucent_v1"),
            program.identity
        );
        assert_eq!(
            MINIMAL_TERRAIN_MATERIAL_FRAGMENT_DIRECT,
            program.fragment.source
        );
        assert!(program
            .fragment
            .source
            .contains("layout(location = 0) out vec4 out_color;"));
        assert!(!program.fragment.source.contains("out_terrain_lit_color"));
    }

    #[test]
    fn direct_terrain_fragment_discard_is_cutout_only_like_frozen_sodium() {
        let cutout = minimal_direct_terrain_cutout_program();
        let opaque = minimal_direct_terrain_solid_program();
        let translucent = minimal_direct_terrain_translucent_program();

        assert!(MINIMAL_TERRAIN_MATERIAL_FRAGMENT_DIRECT
            .contains("#ifdef VULKANIC_TERRAIN_FRAGMENT_DISCARD"));
        assert!(cutout
            .fragment
            .source
            .starts_with("#version 450\n#define VULKANIC_TERRAIN_FRAGMENT_DISCARD 1\n"));
        assert!(!opaque
            .fragment
            .source
            .contains("#define VULKANIC_TERRAIN_FRAGMENT_DISCARD"));
        assert!(!translucent
            .fragment
            .source
            .contains("#define VULKANIC_TERRAIN_FRAGMENT_DISCARD"));
    }

    #[test]
    fn deferred_fabulous_terrain_discard_is_cutout_only_like_frozen_sodium() {
        // Fabulous routes opaque/cutout terrain through the deferred
        // material shader. Keep its pass-local discard admission identical
        // to Frozen's `USE_FRAGMENT_DISCARD` policy, rather than allowing
        // the shared source body to make opaque terrain disappear.
        let cutout = minimal_terrain_cutout_program();
        let opaque = minimal_terrain_solid_program();
        let translucent = minimal_terrain_material_program(TerrainMaterialProgramKind::Translucent);

        assert!(MINIMAL_TERRAIN_MATERIAL_FRAGMENT
            .contains("#ifdef VULKANIC_TERRAIN_FRAGMENT_DISCARD"));
        assert!(cutout
            .fragment
            .source
            .starts_with("#version 450\n#define VULKANIC_TERRAIN_FRAGMENT_DISCARD 1\n"));
        assert!(!opaque
            .fragment
            .source
            .contains("#define VULKANIC_TERRAIN_FRAGMENT_DISCARD"));
        assert!(!translucent
            .fragment
            .source
            .contains("#define VULKANIC_TERRAIN_FRAGMENT_DISCARD"));
    }

    #[test]
    fn lowered_source_program_preserves_real_stages_and_semantic_sampler_plan() {
        let source = paired_source();
        let contract = derive_complementary_terrain_contract(&source).unwrap();
        let artifacts =
            preprocess_terrain_sources(&source, &contract.source_stages().unwrap()).unwrap();
        let lowered = lower_terrain_source_pair(&artifacts.vertex, &artifacts.fragment).unwrap();
        let declarations = TerrainSourceResourceBindings::from_source(&source).unwrap();
        let bindings = lowered
            .opaque_resource_contract()
            .bind_semantic_roles(&declarations)
            .unwrap();

        let program = prepare_lowered_terrain_source_program(
            &contract,
            &lowered,
            &bindings,
            TerrainMaterialProgramKind::Opaque,
        )
        .unwrap();

        assert_eq!(
            "vulkanic:shader-pack/lowered-source-test/terrain_opaque_source_gen9",
            program.identity.as_str()
        );
        assert_eq!(9, program.shader_pack_generation);
        assert_eq!(
            Some(TerrainMaterialProgramKind::Opaque),
            program.material_kind
        );
        assert_eq!(lowered.vertex().source(), program.vertex.source);
        assert_eq!(lowered.fragment().source(), program.fragment.source);
        assert_eq!(
            Some(
                [
                    TerrainPassOutput::LitTerrainColor,
                    TerrainPassOutput::MaterialAuxiliary,
                ]
                .as_slice()
            ),
            program.terrain_outputs(),
            "prepared source terrain programs must retain the named source output schema"
        );
        assert_eq!(
            Some(
                [
                    (TerrainPassOutput::LitTerrainColor, 0),
                    (TerrainPassOutput::MaterialAuxiliary, 6),
                ]
                .as_slice()
            ),
            program.terrain_output_color_slots(),
            "prepared source terrain programs must retain source-declared color-target semantics"
        );
        assert_eq!(1, program.opaque_resource_bindings.bindings().len());
        assert_eq!(
            TerrainSourceResourceRole::MaterialAtlas,
            program.opaque_resource_bindings.bindings()[0].role()
        );
        assert_eq!(
            crate::render::vulkanic::shader_pack::lowering::TerrainSourceOpaqueResourceKind::CombinedTextureSampler,
            program.opaque_resource_bindings.bindings()[0].kind()
        );
        let vulkan_modules = program.shader_module_descriptors(BackendApi::Vulkan);
        assert_eq!(ShaderStage::Vertex, vulkan_modules[0].stage);
        assert_eq!(ShaderStage::Fragment, vulkan_modules[1].stage);
        assert_eq!(ShaderCodeFormat::Glsl, vulkan_modules[0].code_format);
        assert!(std::str::from_utf8(&vulkan_modules[0].code)
            .unwrap()
            .contains("VULKANIC_GAL_ZERO_TO_ONE_CLIP_DEPTH"));
        let opengl_modules = program.shader_module_descriptors(BackendApi::OpenGl);
        let opengl_vertex = std::str::from_utf8(&opengl_modules[0].code).unwrap();
        assert!(
            opengl_vertex.contains("VULKANIC_GAL_ZERO_TO_ONE_CLIP_DEPTH"),
            "the backend-neutral source retains its conditional clip-depth finalizer"
        );
        assert!(
            !opengl_vertex.contains("#define VULKANIC_GAL_ZERO_TO_ONE_CLIP_DEPTH 1"),
            "only Vulkan enables the backend-neutral clip-depth finalizer"
        );
        assert_eq!(program.vertex.label, vulkan_modules[0].label);
        assert_eq!(program.fragment.label, vulkan_modules[1].label);
        assert_eq!(
            TerrainSourceFixedBinding {
                set: 0,
                binding: 0,
                kind: TerrainSourceBindingKind::StorageBuffer,
            },
            program.execution_interface.vertex_stream
        );
        assert_eq!(
            TERRAIN_SOURCE_VERTEX_BYTES as u32,
            program.execution_interface.vertex_stride
        );
        assert_eq!(
            [
                "position",
                "color",
                "normal_light",
                "atlas_uv_lightmap",
                "entity",
                "mid_tex_coord",
                "tangent",
                "mid_block",
            ],
            program
                .execution_interface
                .vertex_fields
                .map(|field| field.name)
        );
        assert_eq!(
            TerrainSourceFixedBinding {
                set: 0,
                binding: 1,
                kind: TerrainSourceBindingKind::UniformBuffer,
            },
            program.execution_interface.legacy_transforms
        );
        assert_eq!(128, program.execution_interface.legacy_transform_bytes);
        assert_eq!(
            TerrainSourceFixedBinding {
                set: 0,
                binding: 3,
                kind: TerrainSourceBindingKind::StorageBuffer,
            },
            program.execution_interface.instance_stream
        );
        assert_eq!(
            TERRAIN_SOURCE_INSTANCE_BYTES as u32,
            program.execution_interface.instance_stride
        );
        assert_eq!(
            Some(TerrainSourceFixedBinding {
                set: 0,
                binding: 2,
                kind: TerrainSourceBindingKind::UniformBuffer,
            }),
            program.execution_interface.scalar_uniforms
        );
        assert_eq!(128, program.execution_interface.scalar_uniform_bytes);
        assert_eq!(
            program.execution_interface.scalar_uniform_bytes,
            lowered.uniform_contract().std140_size()
        );
        assert_eq!(
            program.execution_interface.scalar_uniform_fields,
            lowered.uniform_contract().fields()
        );
        assert_eq!(
            program
                .execution_interface
                .scalar_uniform_fields
                .iter()
                .map(|field| field.name())
                .collect::<Vec<_>>(),
            ["gbufferModelView", "gbufferProjection"]
        );
        assert!(program
            .pack_scalar_uniforms(&TerrainSourceUniformFrame::default())
            .unwrap_err()
            .to_string()
            .contains("view matrix"));
        assert_eq!(
            128,
            program
                .pack_scalar_uniforms(&TerrainSourceUniformFrame {
                    view_matrix: Some([1.0; 16]),
                    projection_matrix: Some([2.0; 16]),
                    ..TerrainSourceUniformFrame::default()
                })
                .unwrap()
                .len()
        );
        let legacy_texture_transforms = program
            .pack_legacy_texture_transforms(&TerrainSourceTextureTransforms {
                atlas_texture_matrix: [1.0; 16],
                lightmap_texture_matrix: [2.0; 16],
            })
            .unwrap();
        assert_eq!(128, legacy_texture_transforms.len());
        assert_eq!(
            1.0,
            f32::from_ne_bytes(legacy_texture_transforms[0..4].try_into().unwrap())
        );
        assert_eq!(
            2.0,
            f32::from_ne_bytes(legacy_texture_transforms[64..68].try_into().unwrap())
        );
        assert!(TerrainSourceTextureTransforms {
            atlas_texture_matrix: [f32::NAN; 16],
            lightmap_texture_matrix: [1.0; 16],
        }
        .validate()
        .unwrap_err()
        .to_string()
        .contains("atlas texture matrix"));
        let canonical_transforms = TerrainSourceTextureTransforms::canonical_minecraft_terrain();
        canonical_transforms.validate().unwrap();
        assert_eq!(1.0, canonical_transforms.atlas_texture_matrix[0]);
        assert_eq!(1.0, canonical_transforms.atlas_texture_matrix[15]);
        assert_eq!(1.0 / 256.0, canonical_transforms.lightmap_texture_matrix[0]);
        assert_eq!(1.0 / 256.0, canonical_transforms.lightmap_texture_matrix[5]);
        assert_eq!(1.0 / 32.0, canonical_transforms.lightmap_texture_matrix[12]);
        assert_eq!(1.0 / 32.0, canonical_transforms.lightmap_texture_matrix[13]);
        let layouts = program.execution_resource_layouts().unwrap();
        assert_eq!(
            vec![
                (0, ResourceBindingKind::StorageBuffer),
                (1, ResourceBindingKind::UniformBuffer),
                (2, ResourceBindingKind::UniformBuffer),
                (3, ResourceBindingKind::StorageBuffer),
            ],
            layouts
                .source_data
                .bindings
                .iter()
                .map(|binding| (binding.binding, binding.kind))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            vec![(0, 0), (1, 1), (2, 1), (3, 1)],
            layouts
                .source_data
                .bindings
                .iter()
                .map(|binding| (binding.binding, binding.dynamic_offset_count))
                .collect::<Vec<_>>()
        );
        assert!(layouts.source_data.bindings.iter().all(|binding| {
            binding.stages == PipelineStageFlags::DRAW
                && binding.array_count == 1
                && !binding.optional
        }));
        assert_eq!(
            vec![(
                program.opaque_resource_bindings.bindings()[0].binding(),
                ResourceBindingKind::CombinedTextureSampler,
            )],
            layouts
                .pack_resources
                .bindings
                .iter()
                .map(|binding| (binding.binding, binding.kind))
                .collect::<Vec<_>>()
        );
        let missing_resources = TerrainSourceResourceAvailabilitySet::new(9, 4, []).unwrap();
        assert!(program
            .require_semantic_resources(&missing_resources)
            .unwrap_err()
            .to_string()
            .contains("material_atlas"));
        let atlas_resources = TerrainSourceResourceAvailabilitySet::new(
            9,
            4,
            [TerrainSourceResourceAvailability {
                role: TerrainSourceResourceRole::MaterialAtlas,
                shape: TerrainSourceSampledResourceShape::Texture2d,
                resource_generation: 11,
            }],
        )
        .unwrap();
        program
            .require_semantic_resources(&atlas_resources)
            .unwrap();
        let source_resources = TerrainSourceOwnedResourceSet::new(
            atlas_resources,
            [TerrainSourceOwnedResource {
                role: TerrainSourceResourceRole::MaterialAtlas,
                combined_sampler: Handle::new(HandleKind::CombinedTextureSampler, 3, 1).unwrap(),
            }],
        )
        .unwrap();
        let packed_resource_set = program
            .pack_resource_set_desc(
                "lowered-source-test.pack-resources",
                Handle::new(HandleKind::ResourceLayout, 5, 1).unwrap(),
                &source_resources,
            )
            .unwrap();
        assert_eq!(1, packed_resource_set.bindings.len());
        assert_eq!(
            program.opaque_resource_bindings.bindings()[0].binding(),
            packed_resource_set.bindings[0].binding
        );
        assert_eq!(
            ResourceBindingKind::CombinedTextureSampler,
            packed_resource_set.bindings[0].kind
        );
        assert!(program
            .pack_resource_set_desc(
                "wrong-layout",
                Handle::new(HandleKind::Sampler, 5, 1).unwrap(),
                &source_resources,
            )
            .unwrap_err()
            .to_string()
            .contains("resource-layout handle"));
        let stale_pack_resources = TerrainSourceResourceAvailabilitySet::new(
            8,
            4,
            [TerrainSourceResourceAvailability {
                role: TerrainSourceResourceRole::MaterialAtlas,
                shape: TerrainSourceSampledResourceShape::Texture2d,
                resource_generation: 11,
            }],
        )
        .unwrap();
        assert!(program
            .require_semantic_resources(&stale_pack_resources)
            .unwrap_err()
            .to_string()
            .contains("does not match resource availability"));
        assert!(program.required_resources.is_empty());
        assert!(prepare_lowered_terrain_source_program(
            &contract,
            &lowered,
            &bindings,
            TerrainMaterialProgramKind::Translucent,
        )
        .is_err());
    }

    #[test]
    fn lowered_entity_program_requires_typed_identity_color_and_local_texture_contract() {
        let source = paired_entity_source();
        let contract = derive_entity_contract(&source, TerrainProgramScope::Overworld).unwrap();
        let lowered = lower_entity_source_pair(&source, &contract).unwrap();
        let declarations = TerrainSourceResourceBindings::from_source(&source).unwrap();
        let bindings = bind_entity_source_resources(&lowered, &declarations).unwrap();

        let program =
            prepare_lowered_entity_source_program(&contract, &lowered, &bindings).unwrap();
        assert_eq!(
            "vulkanic:shader-pack/lowered-entity-source-test/entity_source_gen11",
            program.identity.as_str()
        );
        assert_eq!(11, program.shader_pack_generation);
        assert_eq!(11, program.entity_id_generation);
        assert_eq!(
            [
                (TerrainPassOutput::LitTerrainColor, 0),
                (TerrainPassOutput::MaterialAuxiliary, 6),
            ]
            .as_slice(),
            program.named_output_color_slots()
        );
        assert_eq!(
            Some(TerrainSourceResourceRole::MaterialTexture),
            program.opaque_resource_bindings.role_for("tex")
        );
        let cutout_descriptors =
            program.shader_module_descriptors_with_alpha_cutoff(BackendApi::Vulkan, Some(0.1));
        let cutout =
            String::from_utf8(cutout_descriptors.into_iter().nth(1).unwrap().code).unwrap();
        assert!(cutout.contains("#define VULKANIC_SOURCE_ENTITY_ALPHA_CUTOFF 0.10000000"));
        let opaque_descriptors =
            program.shader_module_descriptors_with_alpha_cutoff(BackendApi::Vulkan, None);
        let opaque =
            String::from_utf8(opaque_descriptors.into_iter().nth(1).unwrap().code).unwrap();
        assert!(opaque.contains("#define VULKANIC_SOURCE_ENTITY_ALPHA_CUTOFF -1.0"));
        let layouts = program.execution_resource_layouts().unwrap();
        assert_eq!(
            vec![0, 1, 2, 3],
            layouts
                .source_data
                .bindings
                .iter()
                .map(|binding| binding.binding)
                .collect::<Vec<_>>(),
            "entity source data must use only the fixed Rust-owned stream and uniform bindings"
        );
        assert_eq!(
            vec![0],
            layouts
                .pack_resources
                .bindings
                .iter()
                .map(|binding| binding.binding)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            ResourceBindingKind::CombinedTextureSampler,
            layouts.pack_resources.bindings[0].kind
        );
        let uniforms = program
            .pack_scalar_uniforms(&TerrainSourceUniformFrame {
                entity_id: Some(50_076),
                entity_color: Some([0.25, 0.5, 0.75, 1.0]),
                view_matrix: Some([1.0; 16]),
                projection_matrix: Some([1.0; 16]),
                ..TerrainSourceUniformFrame::default()
            })
            .unwrap();
        assert_eq!(
            program.execution_interface.scalar_uniform_bytes as usize,
            uniforms.len()
        );
        let transforms = program
            .pack_legacy_texture_transforms(
                &TerrainSourceTextureTransforms::canonical_minecraft_terrain(),
            )
            .unwrap();
        assert_eq!(
            program.execution_interface.legacy_transform_bytes as usize,
            transforms.len(),
            "entity local UVs and packed light coordinates must use the fixed owned source transform ABI"
        );
        assert!(program
            .pack_scalar_uniforms(&TerrainSourceUniformFrame {
                entity_id: Some(50_076),
                view_matrix: Some([1.0; 16]),
                projection_matrix: Some([1.0; 16]),
                ..TerrainSourceUniformFrame::default()
            })
            .unwrap_err()
            .to_string()
            .contains("current rendered entity color"));
    }

    #[test]
    fn lowered_translucent_source_program_has_a_separate_output_contract() {
        let source = paired_translucent_source();
        let contract = derive_complementary_translucent_terrain_contract(&source).unwrap();
        let artifacts =
            preprocess_terrain_sources(&source, &contract.source_stages().unwrap()).unwrap();
        let lowered =
            lower_translucent_terrain_source_pair(&artifacts.vertex, &artifacts.fragment).unwrap();
        let declarations = TerrainSourceResourceBindings::from_source(&source).unwrap();
        let bindings = lowered
            .opaque_resource_contract()
            .bind_semantic_roles(&declarations)
            .unwrap();

        let program =
            prepare_lowered_translucent_terrain_source_program(&contract, &lowered, &bindings)
                .unwrap();

        assert_eq!(
            "vulkanic:shader-pack/lowered-translucent-source-test/terrain_translucent_source_gen12",
            program.identity.as_str()
        );
        assert_eq!(
            Some(TerrainMaterialProgramKind::Translucent),
            program.material_kind
        );
        assert_eq!(
            Some(
                [
                    TerrainPassOutput::LitTerrainColor,
                    TerrainPassOutput::TranslucencyAuxiliary,
                ]
                .as_slice()
            ),
            program.terrain_outputs()
        );
        assert_eq!(
            Some(
                [
                    (TerrainPassOutput::LitTerrainColor, 0),
                    (TerrainPassOutput::TranslucencyAuxiliary, 3),
                ]
                .as_slice()
            ),
            program.terrain_output_color_slots()
        );
        let raster = program
            .translucent_raster_state
            .expect("the prepared translucent program must retain source raster semantics");
        assert_eq!(TerrainTranslucentBlend::SourceAlphaOver, raster.blend);
        assert!((raster.alpha_test.greater_than() - 0.0001).abs() < f32::EPSILON);
        assert_eq!(Some(BlendMode::Alpha), program.translucent_blend_mode());
    }

    #[test]
    fn translucent_source_program_rejects_missing_explicit_raster_contract() {
        let mut files = paired_translucent_source().files();
        files.retain(|file| file.path != "shaders.properties");
        files.push(ShaderSourceFile::new("shaders.properties", ""));
        let source = ShaderPackSource::new("missing-translucent-raster", 13, files).unwrap();
        let contract = derive_complementary_translucent_terrain_contract(&source).unwrap();
        let artifacts =
            preprocess_terrain_sources(&source, &contract.source_stages().unwrap()).unwrap();
        let lowered =
            lower_translucent_terrain_source_pair(&artifacts.vertex, &artifacts.fragment).unwrap();
        let declarations = TerrainSourceResourceBindings::from_source(&source).unwrap();
        let bindings = lowered
            .opaque_resource_contract()
            .bind_semantic_roles(&declarations)
            .unwrap();

        let error =
            prepare_lowered_translucent_terrain_source_program(&contract, &lowered, &bindings)
                .unwrap_err();
        assert!(error
            .to_string()
            .contains("no explicit alpha/blend raster contract"));
    }

    #[test]
    fn lowered_fullscreen_program_owns_semantic_outputs_without_terrain_mesh_state() {
        let source = ShaderPackSource::new(
            "fullscreen-program",
            12,
            vec![
                ShaderSourceFile::new(
                    "world0/deferred1.vsh",
                    "#version 130\nout vec2 uv;\nvoid main() { uv = (gl_TextureMatrix[0] * gl_MultiTexCoord0).xy; gl_Position = ftransform(); }",
                ),
                ShaderSourceFile::new(
                    "world0/deferred1.fsh",
                    "#version 130\nin vec2 uv;\nuniform sampler2D tex;\n/* DRAWBUFFERS:0 */\nvoid main() { gl_FragData[0] = texture2D(tex, uv); }",
                ),
                ShaderSourceFile::new(
                    TERRAIN_RESOURCE_BINDINGS_PATH,
                    "tex=material_atlas\ncolortex0=shader_pack_color:primary\n",
                ),
            ],
        )
        .unwrap();
        let stages = TerrainSourceStages {
            vertex: TerrainSourceStage {
                path: "world0/deferred1.vsh".to_string(),
                defines: Default::default(),
            },
            fragment: TerrainSourceStage {
                path: "world0/deferred1.fsh".to_string(),
                defines: Default::default(),
            },
        };
        let artifacts = preprocess_source_stage_pair(&source, &stages).unwrap();
        let declarations = TerrainSourceResourceBindings::from_source(&source).unwrap();
        let lowered =
            lower_fullscreen_source_pair(&artifacts.vertex, &artifacts.fragment, &declarations)
                .unwrap();
        let bindings = lowered
            .opaque_resource_contract()
            .bind_semantic_roles(&declarations)
            .unwrap();
        let program = prepare_lowered_fullscreen_source_program(
            source.name(),
            source.generation(),
            "world0/deferred1.fsh",
            &lowered,
            &bindings,
        )
        .unwrap();
        assert_eq!(
            "vulkanic:shader-pack/fullscreen-program/world0-deferred1-source-gen12",
            program.identity.as_str()
        );
        assert_eq!(
            TerrainSourceResourceRole::ShaderPackColor("primary".to_string()),
            program.outputs[0].role()
        );
        assert!(program.feedback_requirements.is_empty());
        let layouts = program.execution_resource_layouts().unwrap();
        assert_eq!(
            vec![0],
            layouts
                .source_data
                .bindings
                .iter()
                .map(|binding| binding.binding)
                .collect::<Vec<_>>()
        );
        assert!(program
            .vertex
            .source
            .contains("VulkanicSourceFullscreenFrame"));
        assert!(!program
            .vertex
            .source
            .contains("VulkanicSourceTerrainVertices"));
        let resources = TerrainSourceOwnedResourceSet::new(
            TerrainSourceResourceAvailabilitySet::new(
                source.generation(),
                4,
                [TerrainSourceResourceAvailability {
                    role: TerrainSourceResourceRole::MaterialAtlas,
                    shape: TerrainSourceSampledResourceShape::Texture2d,
                    resource_generation: 9,
                }],
            )
            .unwrap(),
            [TerrainSourceOwnedResource {
                role: TerrainSourceResourceRole::MaterialAtlas,
                combined_sampler: Handle::new(HandleKind::CombinedTextureSampler, 3, 1).unwrap(),
            }],
        )
        .unwrap();
        let set = program
            .pack_resource_set_desc(
                "lowered-fullscreen-test.pack-resources",
                Handle::new(HandleKind::ResourceLayout, 4, 1).unwrap(),
                &resources,
            )
            .unwrap();
        assert_eq!(1, set.bindings.len());
        assert_eq!(
            ResourceBindingKind::CombinedTextureSampler,
            set.bindings[0].kind
        );
        assert!(program
            .pack_resource_set_desc(
                "lowered-fullscreen-test.wrong-layout",
                Handle::new(HandleKind::Sampler, 4, 1).unwrap(),
                &resources,
            )
            .unwrap_err()
            .to_string()
            .contains("resource-layout handle"));
    }

    #[test]
    fn lowered_fullscreen_program_requires_ping_pong_for_sampled_output_role() {
        let source = ShaderPackSource::new(
            "fullscreen-feedback",
            13,
            vec![
                ShaderSourceFile::new(
                    "world0/deferred1.vsh",
                    "#version 130\nout vec2 uv;\nvoid main() { uv = gl_MultiTexCoord0.xy; gl_Position = ftransform(); }",
                ),
                ShaderSourceFile::new(
                    "world0/deferred1.fsh",
                    "#version 130\nin vec2 uv;\nuniform sampler2D colortex0;\n/* DRAWBUFFERS:0 */\nvoid main() { gl_FragData[0] = texture2D(colortex0, uv); }",
                ),
                ShaderSourceFile::new(
                    TERRAIN_RESOURCE_BINDINGS_PATH,
                    "colortex0=shader_pack_color:primary\n",
                ),
            ],
        )
        .unwrap();
        let stages = TerrainSourceStages {
            vertex: TerrainSourceStage {
                path: "world0/deferred1.vsh".to_string(),
                defines: Default::default(),
            },
            fragment: TerrainSourceStage {
                path: "world0/deferred1.fsh".to_string(),
                defines: Default::default(),
            },
        };
        let artifacts = preprocess_source_stage_pair(&source, &stages).unwrap();
        let declarations = TerrainSourceResourceBindings::from_source(&source).unwrap();
        let lowered =
            lower_fullscreen_source_pair(&artifacts.vertex, &artifacts.fragment, &declarations)
                .unwrap();
        let bindings = lowered
            .opaque_resource_contract()
            .bind_semantic_roles(&declarations)
            .unwrap();
        let program = prepare_lowered_fullscreen_source_program(
            source.name(),
            source.generation(),
            "world0/deferred1.fsh",
            &lowered,
            &bindings,
        )
        .unwrap();

        assert_eq!(
            vec![FullscreenSourceFeedbackRequirement {
                role: TerrainSourceResourceRole::ShaderPackColor("primary".to_string()),
                sampled_binding: 0,
                output_location: 0,
            }],
            program.feedback_requirements
        );
    }

    #[test]
    fn lowered_fullscreen_program_derives_mipmap_requirement_from_active_semantic_sampler() {
        let source = ShaderPackSource::new(
            "fullscreen-mipmap",
            14,
            vec![
                ShaderSourceFile::new(
                    "world0/deferred1.vsh",
                    "#version 130\nout vec2 uv;\nvoid main() { uv = gl_MultiTexCoord0.xy; gl_Position = ftransform(); }",
                ),
                ShaderSourceFile::new(
                    "world0/deferred1.fsh",
                    "#version 130\nin vec2 uv;\nuniform sampler2D colortex0;\n/* DRAWBUFFERS:0 */\n/* const bool colortex0MipmapEnabled = true; */\nvoid main() { gl_FragData[0] = texture2D(colortex0, uv); }",
                ),
                ShaderSourceFile::new(
                    TERRAIN_RESOURCE_BINDINGS_PATH,
                    "colortex0=shader_pack_color:primary\n",
                ),
            ],
        )
        .unwrap();
        let stages = TerrainSourceStages {
            vertex: TerrainSourceStage {
                path: "world0/deferred1.vsh".to_string(),
                defines: Default::default(),
            },
            fragment: TerrainSourceStage {
                path: "world0/deferred1.fsh".to_string(),
                defines: Default::default(),
            },
        };
        let artifacts = preprocess_source_stage_pair(&source, &stages).unwrap();
        let declarations = TerrainSourceResourceBindings::from_source(&source).unwrap();
        let lowered =
            lower_fullscreen_source_pair(&artifacts.vertex, &artifacts.fragment, &declarations)
                .unwrap();
        let bindings = lowered
            .opaque_resource_contract()
            .bind_semantic_roles(&declarations)
            .unwrap();
        let program = prepare_lowered_fullscreen_source_program(
            source.name(),
            source.generation(),
            "world0/deferred1.fsh",
            &lowered,
            &bindings,
        )
        .unwrap();

        assert_eq!(
            vec![FullscreenSourceMipmapRequirement {
                role: TerrainSourceResourceRole::ShaderPackColor("primary".to_string()),
                sampled_binding: 0,
            }],
            program.mipmap_requirements
        );
    }

    #[test]
    fn lowered_fullscreen_program_rejects_mipmap_directive_without_active_sampler() {
        let source = ShaderPackSource::new(
            "fullscreen-mipmap-unbound",
            15,
            vec![
                ShaderSourceFile::new(
                    "world0/deferred1.vsh",
                    "#version 130\nout vec2 uv;\nvoid main() { uv = gl_MultiTexCoord0.xy; gl_Position = ftransform(); }",
                ),
                ShaderSourceFile::new(
                    "world0/deferred1.fsh",
                    "#version 130\nin vec2 uv;\n/* DRAWBUFFERS:0 */\n/* const bool colortex0MipmapEnabled = true; */\nvoid main() { gl_FragData[0] = vec4(1.0); }",
                ),
                ShaderSourceFile::new(
                    TERRAIN_RESOURCE_BINDINGS_PATH,
                    "colortex0=shader_pack_color:primary\n",
                ),
            ],
        )
        .unwrap();
        let stages = TerrainSourceStages {
            vertex: TerrainSourceStage {
                path: "world0/deferred1.vsh".to_string(),
                defines: Default::default(),
            },
            fragment: TerrainSourceStage {
                path: "world0/deferred1.fsh".to_string(),
                defines: Default::default(),
            },
        };
        let artifacts = preprocess_source_stage_pair(&source, &stages).unwrap();
        let declarations = TerrainSourceResourceBindings::from_source(&source).unwrap();
        let lowered =
            lower_fullscreen_source_pair(&artifacts.vertex, &artifacts.fragment, &declarations)
                .unwrap();
        let bindings = lowered
            .opaque_resource_contract()
            .bind_semantic_roles(&declarations)
            .unwrap();
        let error = prepare_lowered_fullscreen_source_program(
            source.name(),
            source.generation(),
            "world0/deferred1.fsh",
            &lowered,
            &bindings,
        )
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("no active semantic resource binding"));
    }

    #[test]
    fn exact_atlas_distant_horizons_source_writer_preserves_owned_material_inputs() {
        let program = minimal_distant_horizons_lod_exact_atlas_source_program();

        assert_eq!(
            "vulkanic:builtin/distant_horizons_lod_exact_atlas_source_v1",
            program.identity.as_str()
        );
        assert!(program.vertex.source.contains("v_tile_uv"));
        assert!(program.fragment.source.contains("TerrainAtlasColor"));
        assert!(program.fragment.source.contains("LightmapTexture"));
        assert!(program.fragment.source.contains("out_source_primary"));
        assert!(program.fragment.source.contains("discard"));
    }

    #[test]
    fn lowered_distant_horizons_program_uses_its_own_column_stream_contract() {
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
        let program =
            prepare_lowered_distant_horizons_source_program(&contract, &lowered, &bindings)
                .unwrap();

        assert_eq!(
            "vulkanic:shader-pack/complementaryhungloified-complete-test/distant_horizons_opaque_source_gen91",
            program.identity.as_str()
        );
        assert_eq!(91, program.shader_pack_generation);
        assert_eq!(
            DISTANT_HORIZONS_SOURCE_VERTEX_BYTES as u32,
            program.execution_interface.vertex_stride
        );
        assert_eq!(
            DistantHorizonsMaterialIdentityContract::ReducedColorMaterialCategory,
            program.execution_interface.material_identity_contract,
            "the bundled Complementary DH source consumes DH color/category semantics, not atlas-backed texels"
        );
        assert_eq!(
            DISTANT_HORIZONS_SOURCE_COLUMN_FRAME_BYTES as u32,
            program.execution_interface.column_frame_bytes
        );
        assert_eq!(
            TerrainSourceFixedBinding {
                set: 0,
                binding: 0,
                kind: TerrainSourceBindingKind::StorageBuffer,
            },
            program.execution_interface.vertex_stream
        );
        assert_eq!(
            TerrainSourceFixedBinding {
                set: 0,
                binding: 1,
                kind: TerrainSourceBindingKind::UniformBuffer,
            },
            program.execution_interface.column_frame
        );
        assert!(program
            .vertex
            .source
            .contains("VulkanicDistantHorizonsVertices"));
        assert!(program
            .vertex
            .source
            .contains("vulkanic_source_dh_vertex.light_material_normal >> 16u"));
        assert!(!program
            .vertex
            .source
            .contains("VulkanicDistantHorizonsMaterialIds"));
        assert!(program.vertex.source.contains("dhModelView"));
        assert!(program
            .fragment
            .source
            .contains("out_distant_horizons_lit_color"));
        assert!(!program.fragment.source.contains("out_terrain_lit_color"));
        let layouts = program.execution_resource_layouts().unwrap();
        assert_eq!(
            vec![0, 1, 2],
            layouts
                .source_data
                .bindings
                .iter()
                .map(|binding| binding.binding)
                .collect::<Vec<_>>()
        );
        assert!(!layouts.pack_resources.bindings.is_empty());

        let mut near_terrain_abi = program.clone();
        near_terrain_abi.execution_interface.vertex_stride = TERRAIN_SOURCE_VERTEX_BYTES as u32;
        assert!(near_terrain_abi
            .execution_resource_layouts()
            .unwrap_err()
            .to_string()
            .contains("Distant Horizons source vertex stream"));
    }

    #[test]
    fn exact_atlas_distant_horizons_adapter_preserves_the_selected_source_contract() {
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
        let program =
            prepare_lowered_distant_horizons_source_program(&contract, &lowered, &bindings)
                .unwrap();

        let exact = prepare_lowered_distant_horizons_exact_atlas_source_program(&program)
            .expect("the selected opaque DH source exposes the semantic adapter anchors");

        assert!(exact.source.identity.as_str().ends_with(":exact-atlas"));
        assert_eq!(
            DISTANT_HORIZONS_EXACT_ATLAS_SOURCE_VERTEX_BYTES as u32,
            exact.source.execution_interface.vertex_stride
        );
        assert_eq!(
            DistantHorizonsMaterialIdentityContract::AtlasBacked,
            exact.source.execution_interface.material_identity_contract
        );
        assert!(exact
            .source
            .vertex
            .source
            .contains("VulkanicDistantHorizonsExactAtlasVertex"));
        assert!(exact
            .source
            .vertex
            .source
            .contains("vulkanic_source_dh_atlas_tint_and_material"));
        let lightmap = exact
            .source
            .vertex
            .source
            .split("vec2 vulkanic_source_dh_packed_lightmap_coordinates()")
            .nth(1)
            .expect("the exact-atlas adapter must define its DH lightmap conversion")
            .split("#define vulkanic_source_texture_matrix")
            .next()
            .expect(
                "the exact-atlas adapter lightmap conversion must terminate before its aliases",
            );
        let block_offset = lightmap
            .find("light_normal_tint_material >> 8u")
            .expect("the DH block-light nibble must be emitted");
        let sky_offset = lightmap
            .find("light_normal_tint_material & 0xffu")
            .expect("the DH sky-light nibble must be emitted");
        assert!(
            block_offset < sky_offset,
            "Iris DH lightmap coordinates are (block, sky), not the copied byte order (sky, block)"
        );
        assert!(
            exact.source.vertex.source.contains(
                "vec3(vulkanic_source_dh_vertex.micro_x, 0.0, vulkanic_source_dh_vertex.micro_z)"
            ),
            "the exact-atlas DH adapter must retain Iris's X/Z-only micro-offset semantics"
        );
        assert!(exact
            .source
            .fragment
            .source
            .contains("vulkanic_source_dh_atlas_color"));
        assert!(exact
            .source
            .fragment
            .source
            .contains("color.rgb *= glColor.rgb;"));
        assert!(!exact
            .source
            .fragment
            .source
            .contains("if ((vulkanic_source_dh_atlas_tint_and_material & 1u) != 0u)"));
        assert!(exact
            .source
            .fragment
            .source
            .contains("layout(set = 2, binding = 0) uniform texture2D"));
        assert!(
            exact.source.fragment.source.contains("DoLighting"),
            "the adapter must retain selected source lighting rather than substitute minimal DH lighting"
        );
        assert!(!exact.source.fragment.source.contains("TerrainAtlasColor"));
    }

    #[test]
    fn exact_atlas_distant_horizons_probe_observes_the_adapter_color() {
        let mut fragment = r#"
void main() {
    vec4 color = vulkanic_source_dh_atlas_color();
    out_distant_horizons_lit_color = color;
}
"#
        .to_string();

        apply_exact_atlas_distant_horizons_fragment_probe(&mut fragment, Some("atlas"))
            .expect("the exact-atlas probe must use the DH adapter color anchor");

        assert!(fragment.contains("selected-source diagnostic probe: atlas"));
        assert!(fragment.contains("out_distant_horizons_lit_color = color;\n    return;"));
        assert_eq!(
            1,
            fragment
                .matches("vec4 color = vulkanic_source_dh_atlas_color();")
                .count(),
            "the probe must preserve the exact-atlas color initialization"
        );
    }

    #[test]
    fn exact_atlas_distant_horizons_probes_preserve_source_lighting_boundaries() {
        let source = r#"
void main() {
    vec2 lmCoord = vec2(0.5);
    vec4 color = vulkanic_source_dh_atlas_color();
    DoLighting(color, shadowMult, playerPos);
}
"#;
        let mut pre_lighting = source.to_string();
        apply_exact_atlas_distant_horizons_fragment_probe(&mut pre_lighting, Some("pre-lighting"))
            .expect("the pre-lighting probe must locate the source lighting boundary");
        assert!(pre_lighting.contains(
            "selected-source diagnostic probe: pre-lighting\n    out_distant_horizons_lit_color = color;\n    return;\nDoLighting"
        ));

        let mut lightmap = source.to_string();
        apply_exact_atlas_distant_horizons_fragment_probe(&mut lightmap, Some("lightmap"))
            .expect("the lightmap probe must use the adapter color anchor");
        assert!(lightmap.contains("out_distant_horizons_lit_color = vec4(lmCoord, 0.0, 1.0);"));
    }

    #[test]
    fn lowered_dh_water_program_retains_source_alpha_phase_and_dh_column_abi() {
        let complete = complete_bundled_pack_source_for_test();
        let source = ShaderPackSource::new(
            "complete-dh-water-program",
            92,
            complete
                .files()
                .into_iter()
                .chain(std::iter::once(ShaderSourceFile::new(
                    crate::render::vulkanic::shader_pack::source::RUNTIME_OPTIONS_PATH,
                    "DISTANT_HORIZONS=1\nSHADOW_QUALITY=-1\nFXAA_DEFINE=-1\nCOLORED_LIGHTING=0\nENTITY_SHADOWS_DEFINE=-1\nPLAYER_SHADOW=-1\nRAIN_PUDDLES=0\n",
                )))
                .collect(),
        )
        .unwrap();
        let contract =
            derive_distant_horizons_translucent_contract(&source, TerrainProgramScope::Overworld)
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
        let program =
            prepare_lowered_distant_horizons_source_program(&contract, &lowered, &bindings)
                .unwrap();

        assert_eq!(DistantHorizonsPassKind::Translucent, program.pass_kind);
        assert_eq!(
            Some(TerrainTranslucentBlend::SourceAlphaOver),
            program.translucent_blend
        );
        assert!(program
            .identity
            .as_str()
            .contains("distant_horizons_translucent_source_gen92"));
        assert_eq!(
            DISTANT_HORIZONS_SOURCE_VERTEX_BYTES as u32,
            program.execution_interface.vertex_stride
        );
        assert!(program.fragment.source.contains("depthtex1"));
        assert!(program
            .fragment
            .source
            .contains("out_distant_horizons_lit_color"));
        program.execution_resource_layouts().unwrap();
    }

    #[test]
    fn lowered_shadow_source_program_keeps_its_shadow_identity_and_outputs() {
        let source = paired_shadow_source();
        let vertex = crate::render::vulkanic::shader_pack::preprocess::preprocess_artifact(
            crate::render::vulkanic::shader_pack::preprocess::PreprocessInput {
                source: &source,
                entry: "shadow.vsh",
                defines: &[],
            },
        )
        .unwrap();
        let fragment = crate::render::vulkanic::shader_pack::preprocess::preprocess_artifact(
            crate::render::vulkanic::shader_pack::preprocess::PreprocessInput {
                source: &source,
                entry: "shadow.fsh",
                defines: &[],
            },
        )
        .unwrap();
        let lowered = lower_shadow_source_pair(&vertex, &fragment).unwrap();
        let declarations = TerrainSourceResourceBindings::from_source(&source).unwrap();
        let bindings = lowered
            .opaque_resource_contract()
            .bind_semantic_roles(&declarations)
            .unwrap();

        let program = prepare_lowered_shadow_source_program(
            source.name(),
            source.generation(),
            &lowered,
            &bindings,
        )
        .unwrap();

        assert_eq!(
            "vulkanic:shader-pack/lowered-shadow-source-test/shadow_source_gen10",
            program.identity.as_str()
        );
        assert!(program.vertex.source.contains("shadowModelView"));
        assert!(program.vertex.source.contains("shadowProjection"));
        assert!(program.fragment.source.contains("out_shadow_color"));
        assert!(!program.fragment.source.contains("out_terrain_lit_color"));
        assert_eq!(
            vec!["shadowModelView", "shadowProjection"],
            program
                .execution_interface
                .scalar_uniform_fields
                .iter()
                .map(|field| field.name())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            128,
            program
                .pack_scalar_uniforms(&TerrainSourceUniformFrame {
                    shadow_model_view: Some([1.0; 16]),
                    shadow_projection: Some([2.0; 16]),
                    ..TerrainSourceUniformFrame::default()
                })
                .unwrap()
                .len()
        );
        assert_eq!(
            TerrainSourceResourceRole::MaterialAtlas,
            program.opaque_resource_bindings.bindings()[0].role()
        );
        assert!(program.required_resources.is_empty());
    }

    #[test]
    fn lowered_cloud_source_program_retains_its_named_output_schema() {
        let source = paired_cloud_source();
        let contract = derive_cloud_pass_contract(&source, TerrainProgramScope::Overworld).unwrap();
        let lowered = lower_cloud_source_pair(&source, &contract).unwrap();
        let declarations = TerrainSourceResourceBindings::from_source(&source).unwrap();
        let bindings = lowered
            .opaque_resource_contract()
            .bind_semantic_roles(&declarations)
            .unwrap();
        let program = prepare_lowered_cloud_source_program(&contract, &lowered, &bindings).unwrap();

        assert_eq!(
            "vulkanic:shader-pack/lowered-cloud-source-test/cloud_source_gen13",
            program.identity.as_str()
        );
        assert!(program.vertex.source.contains("vulkanic_source_position"));
        assert!(program.fragment.source.contains("out_cloud_lit_color"));
        assert!(program
            .fragment
            .source
            .contains("out_cloud_material_auxiliary"));
        assert!(program
            .fragment
            .source
            .contains("out_cloud_translucency_auxiliary"));
        assert_eq!(
            vec![
                (TerrainPassOutput::LitTerrainColor, 0),
                (TerrainPassOutput::MaterialAuxiliary, 6),
                (TerrainPassOutput::TranslucencyAuxiliary, 3),
            ],
            program.named_output_color_slots()
        );
        program.execution_resource_layouts().unwrap();
    }

    #[test]
    fn selected_scoped_shadow_source_prepares_without_becoming_an_executable_pass() {
        let source =
            crate::render::vulkanic::shader_pack::preprocess::complete_bundled_pack_source_for_test(
            );
        let stages = crate::render::vulkanic::shader_pack::terrain_contract::shadow_source_stages_for_scope(
            &source,
            crate::render::vulkanic::shader_pack::terrain_contract::TerrainProgramScope::Overworld,
        )
        .unwrap();
        let artifacts = preprocess_terrain_sources(&source, &stages).unwrap();
        let lowered = lower_shadow_source_pair(&artifacts.vertex, &artifacts.fragment).unwrap();
        let declarations = TerrainSourceResourceBindings::from_source(&source).unwrap();
        let bindings = lowered
            .opaque_resource_contract()
            .bind_semantic_roles(&declarations)
            .unwrap();

        let program = prepare_lowered_shadow_source_program(
            source.name(),
            source.generation(),
            &lowered,
            &bindings,
        )
        .unwrap();

        assert!(program.fragment.source.contains("out_shadow_color"));
        assert!(program
            .fragment
            .source
            .contains("out_shadow_light_shaft_color"));
        assert!(!program.fragment.source.contains("out_terrain_lit_color"));
        let layouts = program.execution_resource_layouts().unwrap();
        assert!(layouts
            .pack_resources
            .bindings
            .iter()
            .all(|binding| binding.kind != ResourceBindingKind::StorageTexture));
        assert!(program
            .opaque_resource_bindings
            .bindings()
            .iter()
            .all(|source| source.resource_name() != "voxel_img"));
    }

    #[test]
    fn source_storage_access_preserves_glsl_read_write_qualifiers() {
        assert_eq!(
            AccessFlags::READ,
            source_storage_access("readonly").unwrap()
        );
        assert_eq!(
            AccessFlags::WRITE,
            source_storage_access("writeonly coherent").unwrap()
        );
        assert_eq!(
            AccessFlags(AccessFlags::READ.0 | AccessFlags::WRITE.0),
            source_storage_access("coherent restrict").unwrap()
        );
        assert!(source_storage_access("readonly writeonly")
            .unwrap_err()
            .to_string()
            .contains("both readonly and writeonly"));
    }

    #[test]
    fn prepared_source_interface_rejects_mutated_fixed_abi_fields() {
        let source = paired_source();
        let contract = derive_complementary_terrain_contract(&source).unwrap();
        let artifacts =
            preprocess_terrain_sources(&source, &contract.source_stages().unwrap()).unwrap();
        let lowered = lower_terrain_source_pair(&artifacts.vertex, &artifacts.fragment).unwrap();
        let declarations = TerrainSourceResourceBindings::from_source(&source).unwrap();
        let bindings = lowered
            .opaque_resource_contract()
            .bind_semantic_roles(&declarations)
            .unwrap();
        let program = prepare_lowered_terrain_source_program(
            &contract,
            &lowered,
            &bindings,
            TerrainMaterialProgramKind::Opaque,
        )
        .unwrap();

        program.execution_interface.validate().unwrap();

        let mut wrong_stride = program.execution_interface.clone();
        wrong_stride.vertex_stride = 96;
        assert!(wrong_stride
            .validate()
            .unwrap_err()
            .to_string()
            .contains("128-byte ABI"));

        let mut wrong_lane = program.execution_interface.clone();
        wrong_lane.vertex_fields[3].offset = 64;
        assert!(wrong_lane
            .validate()
            .unwrap_err()
            .to_string()
            .contains("atlas_uv_lightmap"));

        let mut missing_scalar_binding = program.execution_interface.clone();
        missing_scalar_binding.scalar_uniforms = None;
        assert!(missing_scalar_binding
            .validate()
            .unwrap_err()
            .to_string()
            .contains("scalar fields require fixed set 0 binding 2"));

        let mut wrong_instances = program.execution_interface.clone();
        wrong_instances.instance_stride = 32;
        assert!(wrong_instances
            .validate()
            .unwrap_err()
            .to_string()
            .contains("binding 3"));
    }

    #[test]
    fn prepared_textured_material_interface_rejects_mutated_fixed_abi_fields() {
        let source = complete_bundled_pack_source_for_test();
        let contract = super::super::material_contract::derive_textured_material_contract(
            &source,
            super::super::terrain_contract::TerrainProgramScope::Overworld,
        )
        .unwrap();
        let lowered = super::super::material_contract::lower_textured_material_source_pair(
            &source, &contract,
        )
        .unwrap();
        let declarations = TerrainSourceResourceBindings::from_source(&source).unwrap();
        let bindings = lowered
            .opaque_resource_contract()
            .bind_semantic_roles(&declarations)
            .unwrap();
        let program =
            prepare_lowered_textured_material_source_program(&contract, &lowered, &bindings)
                .unwrap();

        program.execution_interface.validate().unwrap();

        let mut wrong_stride = program.execution_interface.clone();
        wrong_stride.vertex_stride = 48;
        assert!(wrong_stride
            .validate()
            .unwrap_err()
            .to_string()
            .contains("64-byte storage stream"));

        let mut wrong_lane = program.execution_interface.clone();
        wrong_lane.vertex_fields[3].offset = 44;
        assert!(wrong_lane
            .validate()
            .unwrap_err()
            .to_string()
            .contains("texture_uv_lightmap"));

        let mut missing_scalar_binding = program.execution_interface.clone();
        missing_scalar_binding.scalar_uniforms = None;
        assert!(missing_scalar_binding
            .validate()
            .unwrap_err()
            .to_string()
            .contains("scalar fields require fixed set 0 binding 2"));

        let mut invalid_scalar_envelope = program.execution_interface.clone();
        invalid_scalar_envelope.scalar_uniform_bytes = 0;
        assert!(invalid_scalar_envelope
            .validate()
            .unwrap_err()
            .to_string()
            .contains("not a valid std140 envelope"));

        let local_texture_primitive =
            super::super::material_contract::stage_textured_material_primitive(
                [
                    [-1.0, -1.0, 2.0],
                    [1.0, -1.0, 2.0],
                    [1.0, 1.0, 2.0],
                    [-1.0, 1.0, 2.0],
                ],
                [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
                0xffff_ffff,
                0,
                super::super::material_contract::TexturedMaterialTextureCoordinates::LocalTexture,
                super::super::material_contract::TexturedMaterialWinding::CounterClockwise,
            )
            .unwrap();
        assert_eq!(
            super::super::material_contract::TEXTURED_MATERIAL_SOURCE_VERTEX_BYTES * 4,
            program
                .pack_material_primitives(&[local_texture_primitive])
                .unwrap()
                .len(),
            "the source ABI accepts local UVs; the frontend selects the owned semantic texture binding"
        );
    }
}
use super::voxel_light_volume::VoxelLightVolumeReadiness;
