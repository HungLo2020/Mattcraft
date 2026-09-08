use crate::render::vulkanic::commands::{
    AttachmentLoadOp, AttachmentStoreOp, ClearColor, CommandOp, PassAttachment, ResourceBarrier,
    TextureImageCopyRegion, TextureOrigin3d, TextureUsageState,
};
use crate::render::vulkanic::error::{GalError, GalResult};
use crate::render::vulkanic::gal::VulkanicGal;
use crate::render::vulkanic::handles::Handle;
use crate::render::vulkanic::resources::{
    CombinedTextureSamplerDesc, CompareOp, Extent3d, IndexType, QueueClass, SamplerAddressMode,
    SamplerDesc, SamplerFilter,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use super::super::world_primitive_frontend::{
    WORLD_STRATUM_ENTITY_MESH, WORLD_STRATUM_TERRAIN,
};
use super::assets::{ShaderPackAssets, TerrainShaderPackAssetBindings};
use super::cloud_contract::{
    derive_cloud_pass_contract, lower_cloud_source_pair, CloudFaceDisposition, CloudPassContract,
};
use super::distant_horizons_contract::{
    derive_distant_horizons_opaque_contract, derive_distant_horizons_translucent_contract,
    DistantHorizonsPassContract,
};
use super::entity_contract::{
    bind_entity_source_resources, derive_entity_contract, lower_entity_source_pair,
    EntityPassContract, LoweredEntitySourcePair,
};
use super::fullscreen::{FullscreenSourceExecutionPlan, FullscreenSourcePassFrame};
use super::fullscreen_contract::{
    derive_fullscreen_source_chain, derive_sky_source_stage, derive_sky_textured_source_stage,
    FullscreenSourceStage, FullscreenSourceStageKind,
};
use super::hand_contract::{
    bind_hand_source_resources, derive_hand_contract, lower_hand_source_pair, HandPassContract,
    LoweredHandSourcePair,
};
use super::interface::{analyze_terrain_vertex_interface, TerrainVertexInterface};
use super::lightmap::{
    VanillaLightmapBinding, VanillaLightmapCache, VanillaLightmapCacheUpdate, VanillaLightmapFrame,
    VanillaLightmapResidency,
};
use super::lowering::{
    lower_distant_horizons_source_pair, lower_fullscreen_source_pair,
    lower_fullscreen_source_pair_with_raster_primitive,
    lower_shadow_source_pair_with_owned_storage, lower_terrain_source_pair,
    lower_translucent_terrain_source_pair, FullscreenSourceRasterPrimitive, LoweredCloudSourcePair,
    LoweredDistantHorizonsSourcePair, LoweredShadowSourcePair, LoweredTerrainSourcePair,
    LoweredTranslucentTerrainSourcePair, LoweredWeatherSourcePair, ShadowFragmentOutput,
    TerrainSourceLoweringSummary, TerrainSourceOpaqueResourceBindingPlan,
};
use super::material_contract::{
    derive_textured_material_contract, lower_textured_material_source_pair,
    LoweredTexturedMaterialSourcePair, TexturedMaterialPassContract,
};
use super::pass_graph::{AttachmentRole, PassIdentity};
use super::preprocess::{
    preprocess_distant_horizons_fullscreen_stage_pair, preprocess_distant_horizons_sources,
    preprocess_source_stage_pair, preprocess_terrain_sources, PreprocessedTerrainSourceSummary,
};
use super::programs::{
    complementary_terrain_subset_program_with_resources, prepare_lowered_cloud_source_program,
    prepare_lowered_distant_horizons_source_program, prepare_lowered_entity_source_program,
    prepare_lowered_fullscreen_source_program, prepare_lowered_hand_source_program,
    prepare_lowered_shadow_source_program, prepare_lowered_terrain_source_program,
    prepare_lowered_textured_material_source_program,
    prepare_lowered_translucent_terrain_source_program, prepare_lowered_weather_source_program,
    LoweredCloudSourceProgram, LoweredDistantHorizonsSourceProgram, LoweredEntitySourceProgram,
    LoweredFullscreenSourceProgram, LoweredHandSourceProgram, LoweredTerrainSourceProgram,
    LoweredTexturedMaterialSourceProgram, LoweredWeatherSourceProgram, TerrainMaterialProgram,
    TerrainMaterialProgramKind, TerrainProgramResource,
};
use super::resources::ShaderPackRuntimePlan;
use super::source::ShaderPackSource;
use super::source_assets::TerrainSourceAssetResources;
use super::source_targets::{
    resolve_terrain_source_color_attachments, source_color_clear_color,
    ShaderPackColorBootstrapClearValues, ShaderPackColorBootstrapPlan, ShaderPackColorFramePlan,
    ShaderPackColorSamplingPlan, ShaderPackColorTargetCache, ShaderPackColorTargetIdentity,
    ShaderPackColorTargetManifest, ShaderPackColorTargets, ShaderPackSourceColorResourceCache,
    TerrainSourceColorAttachment,
};
use super::source_uniforms::{
    TerrainSourceUniformRequirementSummary, TerrainSourceUniformRequirements,
};
#[cfg(test)]
use super::terrain_contract::TerrainPassOutput;
use super::terrain_contract::{
    derive_complementary_translucent_terrain_contract_for_scope, shadow_source_stages_for_scope,
    TerrainPassContract, TerrainPassRequiredResource, TerrainProgramScope,
};
use super::terrain_source_resources::{
    TerrainSourceOwnedResourceSet, TerrainSourceResourceBindings, TerrainSourceResourceRole,
};
use super::terrain_voxelization::{
    PuddleOccupancyDescriptor, TerrainColoredLightDiagnosticState, TerrainColoredLightRuntime,
    TerrainOccupancyRuntime, TerrainPuddleDiagnosticState, TerrainPuddleRuntime,
    TerrainVoxelLightSamplingBinding,
};
use super::voxel_emission_table::VoxelEmissionTable;
use super::voxel_light_volume::{
    VoxelLightVolumeDescriptor, VoxelLightVolumeIdentity, VoxelLightVolumeMapping,
    VoxelLightVolumeViewDirection,
};
use super::voxel_material_map::VoxelMaterialMap;
use super::weather_contract::{
    derive_weather_pass_contract, lower_weather_source_pair, WeatherPassContract,
};
use crate::render::vulkanic::world_primitive_frontend::TerrainVoxelSourceMesh;

pub(crate) const TERRAIN_RUNTIME_COMPOSITE_UNIFORM_BYTES: u64 =
    16 * 4 + 4 * 4 + 4 * 4 + 16 * 4 + 4 * 4 + 4 * 4;

fn shader_pack_color_name_from_role(role: &str) -> GalResult<String> {
    role.strip_prefix("shader_pack_color:")
        .map(str::to_string)
        .ok_or_else(|| {
            GalError::invalid_argument(format!(
                "fullscreen resource role '{role}' is not a named shader-pack color target"
            ))
        })
}

/// One source-generation color transaction for one combined Rust submission.
///
/// This owns only source-color lifecycle semantics: bootstrap clears, explicit
/// mip prerequisites, feedback copies, and submit-confirmed history. World
/// frontends supply semantic terrain or DH draws separately, so neither route
/// can reinterpret attachment state, borrow Iris targets, or advance history
/// after a rejected submission.
pub(crate) struct ShaderPackSourceColorFrameTransaction {
    targets: ShaderPackColorTargets,
    frame: ShaderPackColorFramePlan,
    bootstrap: Option<ShaderPackColorBootstrapPlan>,
    finalized: bool,
}

impl ShaderPackSourceColorFrameTransaction {
    fn begin(
        gal: &mut VulkanicGal,
        targets: &ShaderPackColorTargets,
        frame: ShaderPackColorFramePlan,
        clear_values: ShaderPackColorBootstrapClearValues,
        operations: &mut Vec<CommandOp>,
    ) -> GalResult<Self> {
        let bootstrap = if frame.requires_initial_clear()? {
            Some(frame.stage_full_clear(gal, targets, clear_values)?)
        } else {
            None
        };
        let mut transaction = Self {
            targets: targets.clone(),
            frame,
            bootstrap,
            finalized: false,
        };
        if let Some(bootstrap) = transaction.bootstrap.as_ref() {
            transaction.frame.append_full_clear(bootstrap, operations)?;
        }
        let mipmapped_roles = transaction
            .targets
            .identity
            .mipmapped_target_names
            .iter()
            .cloned()
            .map(TerrainSourceResourceRole::ShaderPackColor)
            .collect::<Vec<_>>();
        transaction
            .frame
            .append_mipmaps(&transaction.targets, &mipmapped_roles, operations)?;
        let feedback_mipmapped_roles = mipmapped_roles
            .iter()
            .filter(|role| transaction.is_feedback_role(role))
            .cloned()
            .collect::<Vec<_>>();
        transaction.frame.append_feedback_mipmaps(
            &transaction.targets,
            &feedback_mipmapped_roles,
            operations,
        )?;
        Ok(transaction)
    }

    /// Records Rust-owned terrain/DH source outputs whose draw was staged by
    /// a dedicated semantic frontend. The scheduler validates only named
    /// color ownership and never accepts arbitrary native attachment handles.
    pub(crate) fn record_external_outputs(
        &mut self,
        outputs: &[TerrainSourceResourceRole],
    ) -> GalResult<()> {
        self.require_open()?;
        self.frame.record_external_outputs(outputs)
    }

    /// Appends one lowered fullscreen source consumer after establishing its
    /// exact current/feedback mip prerequisites from the source contract.
    pub(crate) fn append_fullscreen_consumer(
        &mut self,
        plan: &FullscreenSourceExecutionPlan,
        program: &LoweredFullscreenSourceProgram,
        frame: FullscreenSourcePassFrame,
        operations: &mut Vec<CommandOp>,
    ) -> GalResult<()> {
        self.require_open()?;
        let current_mip_roles = program
            .mipmap_requirements
            .iter()
            .filter(|requirement| {
                !program.feedback_requirements.iter().any(|feedback| {
                    feedback.role == requirement.role
                        && feedback.sampled_binding == requirement.sampled_binding
                })
            })
            .map(|requirement| requirement.role.clone())
            .collect::<Vec<_>>();
        self.frame
            .append_mipmaps(&self.targets, &current_mip_roles, operations)?;
        let feedback_roles = program
            .feedback_requirements
            .iter()
            .map(|requirement| requirement.role.clone())
            .collect::<Vec<_>>();
        let copied_feedback_roles = self.frame.append_same_frame_feedback_snapshots(
            &self.targets,
            &feedback_roles,
            operations,
        )?;
        let copied_feedback_mip_roles = copied_feedback_roles
            .into_iter()
            .filter(|role| {
                program.mipmap_requirements.iter().any(|requirement| {
                    requirement.role == *role
                        && program.feedback_requirements.iter().any(|feedback| {
                            feedback.role == requirement.role
                                && feedback.sampled_binding == requirement.sampled_binding
                        })
                })
            })
            .collect::<Vec<_>>();
        self.frame.append_feedback_mipmaps(
            &self.targets,
            &copied_feedback_mip_roles,
            operations,
        )?;
        plan.append_draw_with_color_frame(program, &mut self.frame, frame, operations)
    }

    /// Finishes the source side of the combined submission. Confirmation is
    /// intentionally separate and consumes this transaction only after GAL
    /// accepts the same command list.
    pub(crate) fn finish(&mut self, operations: &mut Vec<CommandOp>) -> GalResult<()> {
        self.require_open()?;
        self.frame
            .append_feedback_copies(&self.targets, operations)?;
        self.finalized = true;
        Ok(())
    }

    pub(crate) fn confirm(
        self,
        runtime: &mut ShaderPackRuntimeExecutor,
        gal: &mut VulkanicGal,
    ) -> GalResult<()> {
        if !self.finalized {
            self.discard(runtime, gal);
            return Err(GalError::invalid_argument(
                "shader-pack source color transaction must finish before submission confirmation",
            ));
        }
        let result = runtime.confirm_source_color_transaction_submission(gal, self.frame);
        if let Some(bootstrap) = self.bootstrap {
            bootstrap.destroy(gal);
        }
        if result.is_err() {
            runtime.discard_source_color_targets_submission(gal);
        }
        result
    }

    pub(crate) fn discard(self, runtime: &mut ShaderPackRuntimeExecutor, gal: &mut VulkanicGal) {
        if let Some(bootstrap) = self.bootstrap {
            bootstrap.destroy(gal);
        }
        runtime.discard_source_color_targets_submission(gal);
    }

    fn require_open(&self) -> GalResult<()> {
        if self.finalized {
            return Err(GalError::invalid_argument(
                "shader-pack source color transaction is already finalized",
            ));
        }
        Ok(())
    }

    fn is_feedback_role(&self, role: &TerrainSourceResourceRole) -> bool {
        role.shader_pack_color_name().is_some_and(|name| {
            self.targets
                .identity
                .feedback_target_names
                .binary_search_by(|candidate| candidate.as_str().cmp(name))
                .is_ok()
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerrainMaterialPassMode {
    Opaque,
    Cutout,
    Translucent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TerrainGraphIsolation {
    Full,
    TerrainOnly,
    GBufferNoShadow,
    TerrainPlusShadow,
    FullDrawsSkipped,
}

impl TerrainGraphIsolation {
    fn from_env() -> Self {
        match std::env::var("MATTMC_RUST_SHADER_GRAPH_ISOLATION")
            .unwrap_or_default()
            .trim()
        {
            "terrain-only" => Self::TerrainOnly,
            "terrain-plus-gbuffer-no-shadow" | "gbuffer-no-shadow" => Self::GBufferNoShadow,
            "terrain-plus-shadow" => Self::TerrainPlusShadow,
            "full-draws-skipped" => Self::FullDrawsSkipped,
            _ => Self::Full,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct TerrainMeshDraw {
    /// Optional source-specific shadow binding. Built-in draws leave this
    /// unset and reuse their terrain mesh binding for the minimal shadow pass.
    /// A lowered source shadow pair supplies its own program, scalar uniforms,
    /// and semantic set-one resources without borrowing terrain-pass state.
    pub shadow: Option<TerrainShadowDraw>,
    pub pipeline: Handle,
    pub pipeline_layout: Handle,
    pub resource_set: Handle,
    /// Dynamic offsets required by the semantic mesh resource set. Fixture
    /// meshes use one streamed-instance offset; source-derived terrain sets
    /// may own static bindings and therefore require none.
    pub resource_set_dynamic_offsets: Vec<u64>,
    /// Optional shader-pack-owned semantic resources. This is distinct from
    /// the mesh/material set and intentionally carries only GAL handles; the
    /// frontend never sees backend state or shader-pack internals.
    pub shader_resource_set: Option<TerrainShaderResourceSet>,
    pub index_buffer: Handle,
    pub index_offset: u64,
    pub index_type: IndexType,
    pub index_count: u32,
    pub instance_count: u32,
    /// Semantic producer stratum.  The terrain graph uses this to keep
    /// translucent entity meshes out of the deferred capture when Fabulous
    /// owns their named `item_entity` attachment.
    pub stratum: u32,
    pub material_mode: TerrainMaterialPassMode,
    /// The pass contract makes shadow participation explicit. A draw with an
    /// unavailable shadow program is skipped only when its semantic material
    /// route declares that omission up front; missing shadow bindings for
    /// ordinary terrain remain a hard error.
    pub shadow_participation: TerrainShadowParticipation,
}

/// A direct world-material batch which must be composed after deferred terrain
/// lighting. Unlike terrain mesh draws this record does not participate in
/// G-buffer or shadow production; it keeps the already-decoded semantic
/// material resource binding and depth policy explicit while the runtime owns
/// the graph placement.
#[derive(Clone, Debug)]
pub(crate) struct TerrainForwardMaterialDraw {
    pub pipeline: Handle,
    pub pipeline_layout: Handle,
    pub resource_set: Handle,
    /// Optional Rust-owned semantic resource set declared by this direct
    /// material pipeline (currently vanilla weather's copied lightmap).
    /// It is never a Java or backend-native handle.
    pub shader_resource_set: Option<TerrainShaderResourceSet>,
    pub index_buffer: Handle,
    pub index_offset: u64,
    pub index_type: IndexType,
    pub index_count: u32,
    pub instance_count: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerrainShadowParticipation {
    Required,
    Unavailable,
}

#[derive(Clone, Debug)]
pub(crate) struct TerrainShadowDraw {
    pub pipeline: Handle,
    pub pipeline_layout: Handle,
    pub resource_set: Handle,
    pub resource_set_dynamic_offsets: Vec<u64>,
    pub shader_resource_set: Option<TerrainShaderResourceSet>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TerrainShaderResourceSet {
    pub set_index: u32,
    pub set: Handle,
}

/// A generation-coherent ordinary-terrain lightmap descriptor. The descriptor
/// belongs to the same Rust residency generation as its sampled view, so the
/// runtime retires it before replacing that view.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct VanillaLightmapResourceSet {
    pub world_generation: u64,
    pub lightmap_generation: u64,
    pub set: Handle,
}

/// One draw in the source-derived generic textured-material stage. It is
/// intentionally separate from [`TerrainMeshDraw`]: material vertices are a
/// compact explicit source stream, not indexed terrain sections, and this
/// writer must never be selected by terrain material-mode filtering.
#[derive(Clone, Debug)]
pub(crate) struct TexturedMaterialSourceDraw {
    pub pipeline: Handle,
    pub pipeline_layout: Handle,
    pub resource_set: Handle,
    pub resource_set_dynamic_offsets: Vec<u64>,
    pub shader_resource_set: Option<TerrainShaderResourceSet>,
    /// The source vertex preamble expands each compact quad to two triangles
    /// from `gl_VertexIndex`. Keeping this a direct draw avoids inventing an
    /// indexed terrain buffer or a backend-specific index upload for generic
    /// source-material primitives.
    pub vertices: u32,
}

/// One indexed/instanced draw in the source-derived entity stage. It cannot
/// be confused with terrain or generic textured-material work: entity-local
/// texture and Rust-resolved identity are both baked into its owning set-one
/// resource contract before this record is created.
#[derive(Clone, Debug)]
pub(crate) struct EntitySourceDraw {
    pub pipeline: Handle,
    pub pipeline_layout: Handle,
    pub resource_set: Handle,
    pub resource_set_dynamic_offsets: Vec<u64>,
    pub shader_resource_set: TerrainShaderResourceSet,
    pub index_buffer: Handle,
    pub index_offset: u64,
    pub index_type: IndexType,
    pub index_count: u32,
    pub instance_count: u32,
}

/// A complete Rust-owned semantic binding for a terrain program requirement.
/// The contained handles are GAL resources visible only inside Rust frontend
/// and runtime code; Java and the shader-pack contract never see them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TerrainShaderProgramBinding {
    pub resource: TerrainProgramResource,
    pub resource_layout: Handle,
    pub resource_set: TerrainShaderResourceSet,
    pub resource_generation: u64,
}

/// An indivisible built-in candidate-subset terrain program and its matching
/// semantic shader resource set. It exercises resource-generation coherence
/// in focused tests, but is not lowered shader-pack source execution. The
/// latter remains unavailable until its source vertex and semantic resource
/// interfaces are assembled into explicit GAL layouts.
#[derive(Clone, Debug)]
pub(crate) struct TerrainSourceProgramCandidate {
    pub program: TerrainMaterialProgram,
    pub binding: Option<TerrainShaderProgramBinding>,
}

/// Fully source-derived, backend-neutral inputs needed to create an owned
/// colored-light runtime for one world/resource generation. It is deliberately
/// preparation data only: constructing it cannot allocate a native resource,
/// select a shader program, or alter route ownership.
#[derive(Clone, Debug)]
pub(crate) struct TerrainColoredLightPreparation {
    pub descriptor: VoxelLightVolumeDescriptor,
    pub materials: VoxelMaterialMap,
    pub emission: VoxelEmissionTable,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TerrainTranslucentCaptureTargets {
    pub extent: Extent3d,
    pub color_texture: Handle,
    pub color_view: Handle,
    pub depth_texture: Handle,
    pub depth_view: Handle,
    pub target: Handle,
    pub pass: Handle,
}

#[derive(Clone, Copy)]
pub(crate) struct TerrainRuntimeTargets {
    pub shadow_depth_texture: Handle,
    pub shadow_depth_view: Handle,
    pub shadow_color_texture: Handle,
    pub shadow_color_view: Handle,
    pub shadow_light_shaft_texture: Handle,
    pub shadow_light_shaft_view: Handle,
    pub shadow_target: Handle,
    pub shadow_pass: Handle,
    pub albedo_texture: Handle,
    pub albedo_view: Handle,
    pub normal_texture: Handle,
    pub normal_view: Handle,
    pub material_light_texture: Handle,
    pub material_light_view: Handle,
    pub world_position_texture: Handle,
    pub world_position_view: Handle,
    pub depth_texture: Handle,
    pub depth_view: Handle,
    /// Rust-owned depth snapshots retained for source-derived terrain passes.
    /// The plan controls their validity; these handles alone do not admit a
    /// selected source route or make uninitialized depth observable.
    pub depth_history: TerrainDepthHistoryTargets,
    pub target: Handle,
    pub g_buffer_pass: Handle,
    pub deferred_lit_texture: Handle,
    pub deferred_lit_view: Handle,
    pub deferred_lit_target: Handle,
    pub deferred_lighting_pass: Handle,
    pub deferred_lighting_pipeline: Handle,
    pub deferred_lighting_resource_set: Handle,
    pub translucent_target: Handle,
    pub translucent_pass: Handle,
    /// Optional isolated color/depth capture for translucent terrain. The normal
    /// deferred path still writes `deferred_lit`; this target preserves only
    /// the translucent draw stream for a later explicit Fabulous handoff.
    pub translucent_capture: Option<TerrainTranslucentCaptureTargets>,
    pub translucent_capture_initialized: bool,
    pub composite0_texture: Handle,
    pub composite0_view: Handle,
    pub composite0_target: Handle,
    pub composite0_pass: Handle,
    pub composite0_pipeline: Handle,
    pub composite0_resource_set: Handle,
    pub composite1_texture: Handle,
    pub composite1_view: Handle,
    pub composite1_target: Handle,
    pub composite1_pass: Handle,
    pub composite1_pipeline: Handle,
    pub composite1_resource_set: Handle,
    pub final_pass: Handle,
    pub final_pipeline: Handle,
    pub final_resource_set: Handle,
    pub screen_pipeline_layout: Handle,
    pub composite_uniform_buffer: Handle,
    pub shadow_targets_initialized: bool,
}

/// The source-derived shadow writer's complete Rust-owned target contract.
/// It is intentionally smaller than the whole terrain graph: normal terrain
/// and Distant Horizons source passes need only these explicit shadow
/// attachments before they write pack-named color targets. No Iris state,
/// attachment-number convention, or backend object escapes through it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TerrainSourceShadowPassTargets {
    pub shadow_depth_texture: Handle,
    pub shadow_depth_view: Handle,
    pub shadow_color_texture: Handle,
    pub shadow_color_view: Handle,
    pub shadow_light_shaft_texture: Handle,
    pub shadow_light_shaft_view: Handle,
    pub shadow_target: Handle,
    pub shadow_pass: Handle,
    pub initialized: bool,
}

impl From<TerrainRuntimeTargets> for TerrainSourceShadowPassTargets {
    fn from(targets: TerrainRuntimeTargets) -> Self {
        Self {
            shadow_depth_texture: targets.shadow_depth_texture,
            shadow_depth_view: targets.shadow_depth_view,
            shadow_color_texture: targets.shadow_color_texture,
            shadow_color_view: targets.shadow_color_view,
            shadow_light_shaft_texture: targets.shadow_light_shaft_texture,
            shadow_light_shaft_view: targets.shadow_light_shaft_view,
            shadow_target: targets.shadow_target,
            shadow_pass: targets.shadow_pass,
            initialized: targets.shadow_targets_initialized,
        }
    }
}

/// The bounded normal-terrain portion of a selected source frame. Color
/// attachments come from the pack's named Rust-owned target generation,
/// while depth remains an explicit Rust-owned world attachment. This is not
/// a general G-buffer schema and deliberately carries no Iris or backend
/// state.
#[derive(Clone, Debug)]
pub(crate) struct TerrainSourceColorPassTargets {
    /// The source-stage ordering contract for this writer. Bootstrap terrain
    /// initializes pack-declared attachments and the main depth image;
    /// translucency subsequently loads both. This is semantic pass ordering,
    /// not an attachment-slot or backend-state convention.
    pub phase: TerrainSourceColorPassPhase,
    pub color_attachments: Vec<TerrainSourceColorAttachment>,
    /// Per-frame semantic clear inputs. The named attachment retains the
    /// source declaration; this supplies only dynamic fog for its portable
    /// primary-color default.
    pub clear_values: ShaderPackColorBootstrapClearValues,
    pub depth_texture: Handle,
    pub depth_view: Handle,
    pub target: Handle,
    pub pass: Handle,
}

/// The two world-material writers currently admitted by the source-derived
/// terrain graph. Keeping their load/store semantics explicit prevents a
/// future translucent pass from clearing opaque/cutout output or rewriting
/// depth as if it were the first terrain writer.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum TerrainSourceColorPassPhase {
    Bootstrap,
    /// A source-defined sky initializer has already written the pack's named
    /// primary color. Opaque/cutout terrain must load that attachment while
    /// preserving the normal bootstrap behavior for every other target and
    /// for depth.
    BootstrapAfterSky,
    /// The distinct source-derived `gbuffers_textured` writer runs after
    /// opaque/cutout terrain against the same named color/depth generation.
    /// It always loads existing attachments: generic material cannot
    /// bootstrap, clear, or silently replace terrain output.
    TexturedMaterial,
    /// `gbuffers_weather` is a distinct source-defined alpha-over writer.
    /// It shares named targets with terrain but never bootstraps or clears.
    Weather,
    /// `gbuffers_clouds` is a separate source-defined alpha-over writer.
    /// It shares the named terrain targets but has its own source contract,
    /// so cloud work cannot be relabelled as weather or generic material.
    Clouds,
    /// `gbuffers_entities` consumes an entity-local material texture and
    /// Rust-resolved entity identity. It loads existing named targets and
    /// never bootstraps or clears terrain output.
    Entities,
    /// `gbuffers_hand` is a distinct first-person writer. It preserves the
    /// Rust-owned named color generation but clears the private depth domain
    /// before rendering hands/items, so world-depth occlusion cannot leak
    /// into first-person composition. This is a semantic pass rule rather
    /// than an Iris phase or backend state reconstruction.
    Hands,
    /// The first terrain writer is translucent. It must initialize named
    /// color/depth attachments before alpha-over drawing rather than loading
    /// an image that has not been written by a bootstrap pass.
    TranslucentFirst,
    Translucent,
}

impl TerrainSourceColorPassPhase {
    /// Derives the writer ordering from the lowered source program rather
    /// than a caller-selected material label. The only program carrying the
    /// explicit source alpha/blend contract is the separate translucent
    /// stage; normal terrain remains the bootstrap writer.
    pub(crate) fn for_program(program: &LoweredTerrainSourceProgram) -> Self {
        if program.translucent_raster_state().is_some() {
            Self::Translucent
        } else {
            Self::Bootstrap
        }
    }

    pub(crate) fn compatible_with_program(self, program: &LoweredTerrainSourceProgram) -> bool {
        matches!(
            (self, Self::for_program(program)),
            (Self::Bootstrap, Self::Bootstrap)
                | (Self::BootstrapAfterSky, Self::Bootstrap)
                | (Self::Translucent, Self::Translucent)
                | (Self::TranslucentFirst, Self::Translucent)
        )
    }

    fn accepts_material(self, material_mode: TerrainMaterialPassMode) -> bool {
        match self {
            Self::Bootstrap | Self::BootstrapAfterSky => matches!(
                material_mode,
                TerrainMaterialPassMode::Opaque | TerrainMaterialPassMode::Cutout
            ),
            // `gbuffers_textured` has its own semantic stream and draw type.
            // Never let the terrain writer accidentally record a terrain mesh
            // into this load-only pass merely because both are world material.
            Self::TexturedMaterial
            | Self::Weather
            | Self::Clouds
            | Self::Entities
            | Self::Hands => false,
            Self::Translucent => material_mode == TerrainMaterialPassMode::Translucent,
            Self::TranslucentFirst => material_mode == TerrainMaterialPassMode::Translucent,
        }
    }

    fn depth_before(self) -> TextureUsageState {
        match self {
            Self::Bootstrap | Self::BootstrapAfterSky => TextureUsageState::Undefined,
            Self::TexturedMaterial | Self::Weather | Self::Clouds | Self::Entities => {
                TextureUsageState::ShaderRead
            }
            // Hands own a fresh private depth attachment.  Its first use is
            // the explicit clear/load pass, so the semantic predecessor is
            // Undefined rather than the shared world-depth read state.
            Self::Hands => TextureUsageState::Undefined,
            Self::Translucent => TextureUsageState::ShaderRead,
            Self::TranslucentFirst => TextureUsageState::Undefined,
        }
    }

    fn color_load_op(self, attachment: &TerrainSourceColorAttachment) -> AttachmentLoadOp {
        match self {
            Self::Bootstrap if attachment.clear_each_frame => AttachmentLoadOp::Clear,
            Self::BootstrapAfterSky
                if attachment.role.shader_pack_color_name() == Some("primary") =>
            {
                AttachmentLoadOp::Load
            }
            Self::BootstrapAfterSky if attachment.clear_each_frame => AttachmentLoadOp::Clear,
            Self::Bootstrap
            | Self::BootstrapAfterSky
            | Self::TexturedMaterial
            | Self::Weather
            | Self::Clouds
            | Self::Entities
            | Self::Hands
            | Self::Translucent => AttachmentLoadOp::Load,
            Self::TranslucentFirst if attachment.clear_each_frame => AttachmentLoadOp::Clear,
            Self::TranslucentFirst => AttachmentLoadOp::Load,
        }
    }

    fn depth_load_op(self) -> AttachmentLoadOp {
        match self {
            Self::Bootstrap | Self::BootstrapAfterSky => AttachmentLoadOp::Clear,
            Self::TexturedMaterial
            | Self::Weather
            | Self::Clouds
            | Self::Entities
            | Self::Translucent => AttachmentLoadOp::Load,
            Self::TranslucentFirst => AttachmentLoadOp::Clear,
            // First-person depth intentionally begins at the backend-neutral
            // clear value (1.0) after all world material writers. Color still
            // loads because hands compose over the completed world image.
            Self::Hands => AttachmentLoadOp::Clear,
        }
    }
}

/// The three semantic images involved in a main-depth snapshot transaction.
/// It is intentionally independent of render targets, attachment slots, and
/// native image identity so both backends consume the same GAL copy contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TerrainDepthHistoryTargets {
    pub main_depth_texture: Handle,
    pub before_translucency_texture: Handle,
    pub previous_texture: Handle,
}

/// Per-frame depth-history copy plan supplied by the world frontend. The
/// frontend advances validity only after the combined frame submission is
/// accepted, so this declaration never turns an aborted frame into history.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TerrainDepthHistoryPlan {
    /// Whether the prior `before_translucency` snapshot may be copied into
    /// `previous` before replacing it with this frame's opaque/cutout depth.
    pub prior_before_translucency_valid: bool,
    /// The prior state of the destination snapshot. A valid destination is
    /// transitioned from shader-read; an invalid one starts undefined.
    pub prior_previous_valid: bool,
    pub extent: Extent3d,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TerrainRuntimeFrame {
    pub frame_target: Handle,
    pub color_attachment: Handle,
    pub background_color: ClearColor,
    /// A Rust-owned semantic background writer has initialized the complete
    /// G-buffer before opaque terrain. The terrain graph must load rather
    /// than clear it; this is explicit pass ordering, never backend state.
    pub g_buffer_background_initialized: bool,
    pub uniforms: TerrainCompositeUniforms,
    /// Optional private source-depth history. Normal world material output is
    /// unchanged when no source stage declares a need for these snapshots.
    pub depth_history: Option<TerrainDepthHistoryPlan>,
    /// Shadow attachments are persistent graph resources just like the
    /// deferred screen targets; only their first use starts from Undefined.
    pub shadow_targets_initialized: bool,
    /// Composite attachments persist across frames. The first frame starts
    /// from Undefined; later frames begin from ShaderRead after the prior
    /// composite chain's final transition.
    pub screen_targets_initialized: bool,
    /// The isolated translucent capture has a valid ShaderRead layout after
    /// its first completed graph write.
    pub translucent_capture_initialized: bool,
    /// When true, translucent entity meshes are lowered into Fabulous's
    /// external `item_entity` attachment by the enclosing frontend and must
    /// not also be rasterized into either deferred transparency destination.
    pub translucent_entity_external: bool,
    /// When true, translucent terrain is retained only in the explicit
    /// Fabulous `translucent` attachment.  The enclosing handoff composes
    /// that attachment over the copied opaque main image; writing it into the
    /// normal deferred image as well would blend every pane twice.
    pub translucent_terrain_external: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TerrainCompositeUniforms {
    pub light_view_projection: [f32; 16],
    pub shadow_params: [f32; 4],
    pub color_grade_params: [f32; 4],
    /// Inverse of the copied game projection. The depth composite uses this
    /// only to reconstruct camera-relative distance from the explicit main
    /// depth attachment; no backend depth/state query is involved.
    pub projection_inverse: [f32; 16],
    /// Copied vanilla fog color plus environmental spherical-fog start.
    pub fog_color_and_environmental_start: [f32; 4],
    /// Environmental end, render-distance cylindrical start/end, and copied
    /// vanilla fog-color alpha for direct Sodium-compatible fog.
    pub fog_ranges: [f32; 4],
}

/// Owned-source discovery is deliberately separate from the executable plan.
/// A discovered contract proves only that Rust has parsed semantic pack input;
/// it cannot route any draw through an incomplete selected-source pipeline.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum TerrainSourceCandidateState {
    Unavailable,
    Disabled {
        generation: u64,
        pack_name: String,
    },
    Discovered {
        generation: u64,
        pack_name: String,
        requires_colored_voxel_light: bool,
        contract: TerrainPassContract,
        /// The scoped `gbuffers_textured` contract is retained independently
        /// from terrain. It is the future Rust-owned writer for generic
        /// material quads; discovery never authorizes the existing final
        /// overlay path to stand in for shader-pack participation.
        textured_material_contract: Option<TexturedMaterialPassContract>,
        textured_material_contract_error: Option<String>,
        textured_material_lowered_pair: Option<LoweredTexturedMaterialSourcePair>,
        /// `gbuffers_textured` owns a pass-local sampler/image contract. It
        /// must never borrow normal terrain bindings merely because the two
        /// source stages share a pack generation.
        textured_material_source_resource_binding_count: Option<u32>,
        textured_material_source_resource_binding_error: Option<String>,
        textured_material_source_resource_bindings: Option<TerrainSourceOpaqueResourceBindingPlan>,
        /// Entity source discovery is intentionally independent from generic
        /// material staging. The lowered contract is consumed by the
        /// Rust-owned entity mesh stream and pass writer, which validate every
        /// requested semantic input and optional vertex attribute at frame
        /// admission time.
        entity_contract: Option<EntityPassContract>,
        entity_contract_error: Option<String>,
        entity_lowered_pair: Option<LoweredEntitySourcePair>,
        entity_source_resource_binding_count: Option<u32>,
        entity_source_resource_binding_error: Option<String>,
        entity_source_resource_bindings: Option<TerrainSourceOpaqueResourceBindingPlan>,
        /// First-person source discovery is deliberately separate from both
        /// world entities and the generic textured-material stage. Its own
        /// projection and cleared-depth domain are supplied by the dedicated
        /// Rust-owned hand writer before frame admission.
        hand_contract: Option<HandPassContract>,
        hand_contract_error: Option<String>,
        hand_lowered_pair: Option<LoweredHandSourcePair>,
        hand_source_resource_binding_count: Option<u32>,
        hand_source_resource_binding_error: Option<String>,
        hand_source_resource_bindings: Option<TerrainSourceOpaqueResourceBindingPlan>,
        /// Weather is a separate selected-source stage. It reuses compact
        /// semantic material vertices but retains its own source output and
        /// resource requirements; discovery alone cannot enable a route.
        weather_contract: Option<WeatherPassContract>,
        weather_contract_error: Option<String>,
        weather_lowered_pair: Option<LoweredWeatherSourcePair>,
        weather_source_resource_binding_count: Option<u32>,
        weather_source_resource_binding_error: Option<String>,
        weather_source_resource_bindings: Option<TerrainSourceOpaqueResourceBindingPlan>,
        /// Cloud source preparation is admitted only when the owned cloud
        /// writer declares compatible targets and frame resources.
        cloud_contract: Option<CloudPassContract>,
        cloud_contract_error: Option<String>,
        cloud_lowered_pair: Option<LoweredCloudSourcePair>,
        cloud_source_resource_binding_count: Option<u32>,
        cloud_source_resource_binding_error: Option<String>,
        cloud_source_resource_bindings: Option<TerrainSourceOpaqueResourceBindingPlan>,
        /// The pack's translucent terrain stage is discovered independently
        /// from normal terrain. It has a separate source contract and cannot
        /// be silently treated as an opaque/cutout variant during admission.
        translucent_contract: Option<TerrainPassContract>,
        translucent_contract_error: Option<String>,
        translucent_source_summary: Option<PreprocessedTerrainSourceSummary>,
        translucent_source_preprocess_error: Option<String>,
        translucent_source_lowering_error: Option<String>,
        translucent_lowered_pair: Option<LoweredTranslucentTerrainSourcePair>,
        /// The translucent source has its own pass-local sampler contract.
        /// Retaining it separately prevents `gbuffers_water` from inheriting
        /// normal-terrain resources with incompatible semantics.
        translucent_source_resource_binding_count: Option<u32>,
        translucent_source_resource_binding_error: Option<String>,
        translucent_source_resource_bindings: Option<TerrainSourceOpaqueResourceBindingPlan>,
        /// Both terrain stages are source-expanded and fingerprinted before a
        /// source candidate can progress beyond discovery. This is provenance
        /// only; it is not a compiled program or render-route decision.
        source_summary: Option<PreprocessedTerrainSourceSummary>,
        source_preprocess_error: Option<String>,
        /// The semantically scoped shadow-stage pair is expanded separately
        /// from normal terrain and consumed by the Rust-owned shadow-color
        /// pass when its explicit target/resources are available.
        source_shadow_summary: Option<PreprocessedTerrainSourceSummary>,
        source_shadow_preprocess_error: Option<String>,
        /// Bounded diagnostic from lowering the scoped shadow fragment's
        /// named outputs. It cannot create a program or a shadow attachment.
        source_shadow_output_count: Option<u32>,
        source_shadow_lowering_error: Option<String>,
        /// The exact lowered shadow pair retained with the source generation.
        /// Execution still requires the pass to declare compatible attachments
        /// and a complete source shadow resource contract.
        source_lowered_shadow_pair: Option<LoweredShadowSourcePair>,
        /// Active resource roles for the separately lowered shadow source
        /// pair. These remain distinct from the normal terrain plan because
        /// binding numbers are pass-local, while shared pack assets may be
        /// prepared from their semantic union.
        source_shadow_resource_binding_count: Option<u32>,
        source_shadow_resource_binding_error: Option<String>,
        source_shadow_resource_bindings: Option<TerrainSourceOpaqueResourceBindingPlan>,
        /// Source-derived, backend-neutral vertex requirements observed after
        /// preprocessing the paired vertex stage. This is a strict future
        /// lowering prerequisite, never a Java/Iris vertex layout.
        source_vertex_interface: Option<TerrainVertexInterface>,
        /// Compact proof that both source stages reached the private lowering
        /// contract. It is diagnostics only and never admits execution.
        source_lowering_summary: Option<TerrainSourceLoweringSummary>,
        source_lowering_error: Option<String>,
        /// Bounded source-derived summary of scalar inputs which are (or are
        /// not) backed by named Rust gameplay semantics. It is diagnostic
        /// provenance only and cannot admit a source execution route.
        source_uniform_requirement_summary: Option<TerrainSourceUniformRequirementSummary>,
        source_uniform_requirement_error: Option<String>,
        /// The validated lowered source pair is retained privately so later
        /// preparation uses exactly the source that discovery inspected.
        /// Retaining it does not compile, bind, or execute a program.
        source_lowered_pair: Option<LoweredTerrainSourcePair>,
        /// Every lowered opaque resource needs a pack-declared semantic role
        /// before it can be assembled into a Rust-owned resource layout.
        /// This is still discovery provenance, never a GAL resource set.
        source_resource_binding_count: Option<u32>,
        source_resource_binding_error: Option<String>,
        /// Stable source-name to semantic-role mapping retained for a future
        /// Rust-owned resource-set builder. It carries no native handles,
        /// texture units, or backend descriptors.
        source_resource_bindings: Option<TerrainSourceOpaqueResourceBindingPlan>,
        /// Source-derived terrain PNG declarations. They are retained only as
        /// semantic pack paths until the runtime validates matching Rust-owned
        /// binary assets and creates explicit GAL resources.
        source_asset_binding_count: Option<u32>,
        source_asset_binding_error: Option<String>,
        source_asset_bindings: Option<TerrainShaderPackAssetBindings>,
        /// Source-derived named color declarations shared by any later
        /// fullscreen consumers. The manifest is semantic pack metadata;
        /// target allocation remains private and cannot select this route.
        source_color_target_count: Option<u32>,
        source_color_target_error: Option<String>,
        source_color_target_gal_schema_error: Option<String>,
        source_color_targets: Option<ShaderPackColorTargetManifest>,
        /// Optional source-defined world-sky initializer. It is deliberately
        /// separate from the deferred/composite chain because it writes the
        /// named primary color before terrain rather than consuming terrain
        /// output after it.
        pre_terrain_sky_preparation: Option<FullscreenSourceStagePreparation>,
        /// Optional source-defined vanilla celestial writer. It remains
        /// separate from the sky disc because it consumes an owned local
        /// texture and emits real quad geometry for the sun/moon path.
        pre_terrain_celestial_preparation: Option<FullscreenSourceStagePreparation>,
        /// The source pack's complete scoped fullscreen chain. It belongs to
        /// the normal world contract rather than Distant Horizons: vanilla
        /// terrain can require the same deferred/composite/final stages even
        /// when no DH geometry is visible. Retaining this independently keeps
        /// eventual shared target allocation from silently deriving feedback
        /// history from only the current DH depth-consumer subset.
        post_terrain_preparation: Vec<FullscreenSourceStagePreparation>,
        post_terrain_preparation_error: Option<String>,
        voxel_materials: Option<VoxelMaterialMap>,
        voxel_emission: Option<VoxelEmissionTable>,
    },
    Rejected {
        generation: u64,
        pack_name: String,
        reason: String,
    },
}

/// Discovery and lowering provenance for the distinct Distant Horizons
/// source stage. This deliberately does not share the near-terrain candidate:
/// DH has its own copied column stream, transform contract, output targets,
/// and eventual composite dependency. Retaining it here establishes
/// generation-coherent source ownership without selecting a live DH route.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum DistantHorizonsSourceCandidateState {
    Unavailable,
    Disabled {
        generation: u64,
        pack_name: String,
    },
    Discovered {
        generation: u64,
        pack_name: String,
        contract: DistantHorizonsPassContract,
        /// Discovery for the pack's separate DH translucent entry. It is
        /// retained only to give range admission an exact reason; this state
        /// never lowers, binds, or routes `dh_water` until its owned late
        /// target and full resource contract are implemented.
        translucent_contract: DistantHorizonsTranslucentSourceCandidate,
        /// Prepared provenance for the separately selected DH translucent
        /// pair. Retaining it does not create a pipeline or route; it only
        /// guarantees a future late-pass owner consumes the exact source that
        /// discovery validated.
        translucent_source_summary: Option<PreprocessedTerrainSourceSummary>,
        translucent_source_preprocess_error: Option<String>,
        translucent_source_lowering_error: Option<String>,
        translucent_source_lowered_pair: Option<LoweredDistantHorizonsSourcePair>,
        translucent_source_resource_binding_error: Option<String>,
        translucent_source_resource_bindings: Option<TerrainSourceOpaqueResourceBindingPlan>,
        source_summary: Option<PreprocessedTerrainSourceSummary>,
        source_preprocess_error: Option<String>,
        source_lowering_error: Option<String>,
        source_lowered_pair: Option<LoweredDistantHorizonsSourcePair>,
        source_uniform_requirement_summary: Option<TerrainSourceUniformRequirementSummary>,
        source_uniform_requirement_error: Option<String>,
        source_resource_binding_count: Option<u32>,
        source_resource_binding_error: Option<String>,
        source_resource_bindings: Option<TerrainSourceOpaqueResourceBindingPlan>,
        /// Source-derived format, clear, and mip declarations for named pack
        /// colors. These remain runtime preparation data: no target is
        /// allocated and no source route is selected here.
        source_color_target_count: Option<u32>,
        source_color_target_error: Option<String>,
        /// A generic GAL-format diagnostic. Native backend/device support is
        /// checked later while staging Rust-owned target resources; Rust must
        /// never replace an unavailable source format with a nearby format.
        source_color_target_gal_schema_error: Option<String>,
        source_color_targets: Option<ShaderPackColorTargetManifest>,
        /// Every later source stage which actually samples the DH depth
        /// stream. These are paired preprocessor artifacts only: they do not
        /// compile through the terrain-mesh lowerer and cannot make DH live.
        /// Retaining them prevents a future executor from silently skipping a
        /// source-declared depth consumer.
        depth_consumer_preparation: Vec<DistantHorizonsDepthConsumerPreparation>,
        /// The complete post-terrain chain expanded with the same explicit
        /// Distant Horizons source mode as the DH writer. This is separate
        /// from the normal-world chain because a pack may branch on
        /// `DISTANT_HORIZONS` while consuming the same named color targets.
        post_terrain_preparation: Vec<FullscreenSourceStagePreparation>,
        post_terrain_preparation_error: Option<String>,
    },
    Rejected {
        generation: u64,
        pack_name: String,
        reason: String,
    },
}

/// Bounded discovery provenance for `dh_water`. The opaque DH route must stay
/// independently usable when a pack omits this stage, while a visible water
/// range must never be silently treated as opaque if the stage is malformed
/// or still lacks an owned executor.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum DistantHorizonsTranslucentSourceCandidate {
    Unavailable,
    Discovered(DistantHorizonsPassContract),
    Rejected(String),
}

/// Bounded source provenance for one later shader-pack stage that samples DH
/// depth. The source pair stays independent of a frontend or backend so the
/// eventual fullscreen executor can lower it through explicit named resources
/// rather than borrowing Iris pass state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DistantHorizonsDepthConsumerPreparation {
    pub stage_path: String,
    pub reads_opaque_depth: bool,
    pub reads_depth_before_translucency: bool,
    pub source_summary: Option<PreprocessedTerrainSourceSummary>,
    pub source_preprocess_error: Option<String>,
    /// Fullscreen lowering must complete before this consumer can be compiled
    /// into an owned pass. Discovery retains its specific failure rather than
    /// pretending that preprocessing alone establishes executable readiness.
    pub source_lowering_error: Option<String>,
    /// Manifest-derived semantic color targets written by this stage. Raw
    /// `DRAWBUFFERS` locations are intentionally not retained here.
    pub source_output_roles: Vec<String>,
    /// Source program preparation additionally requires a fully semantic
    /// scalar-uniform contract. Its result still has no pipeline, targets, or
    /// route effect, but exposes the first concrete missing runtime input.
    pub source_program_preparation_error: Option<String>,
    pub source_program_identity: Option<String>,
    /// Owned source-derived program retained only after preprocessing,
    /// semantic resource binding, and scalar-uniform validation all succeed.
    /// This prevents a future executor from re-parsing another pack generation
    /// or silently skipping a source-declared DH depth consumer.
    pub source_program: Option<LoweredFullscreenSourceProgram>,
    /// Source-declared output/input aliases that require a distinct
    /// previous/current attachment pair before this consumer can execute.
    pub source_feedback_roles: Vec<String>,
    /// Source-local `colortexNMipmapEnabled` directives resolved through
    /// active semantic sampler bindings. Allocation is diagnostic preparation
    /// only until a later owned fullscreen pass generates the requested mips.
    pub source_mipmap_roles: Vec<String>,
    /// Full source-declared sampler/image plan for this consumer. It remains
    /// diagnostic preparation until a generic fullscreen executor owns every
    /// named semantic resource and output attachment.
    pub source_resource_binding_count: Option<u32>,
    pub source_resource_binding_error: Option<String>,
}

/// Bounded reusable preparation evidence for one selected shader-pack
/// fullscreen stage. It intentionally does not know whether a stage belongs
/// to normal terrain or Distant Horizons: both feed the same source-owned
/// named-color graph after their geometry passes. The data contains source
/// identities and lowered Rust programs only, never Iris state or backend
/// objects.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FullscreenSourceStagePreparation {
    pub stage_path: String,
    pub kind: FullscreenSourceStageKind,
    pub source_summary: Option<PreprocessedTerrainSourceSummary>,
    pub source_preprocess_error: Option<String>,
    pub source_lowering_error: Option<String>,
    pub source_output_roles: Vec<String>,
    pub source_program_preparation_error: Option<String>,
    pub source_program_identity: Option<String>,
    pub source_program: Option<LoweredFullscreenSourceProgram>,
    pub source_feedback_roles: Vec<String>,
    pub source_mipmap_roles: Vec<String>,
    pub source_resource_binding_count: Option<u32>,
    pub source_resource_binding_error: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FullscreenSourceMode {
    NormalWorld,
    DistantHorizons,
}

fn prepare_fullscreen_source_stage(
    source: &ShaderPackSource,
    stage: &FullscreenSourceStage,
    mode: FullscreenSourceMode,
) -> FullscreenSourceStagePreparation {
    let artifacts = match mode {
        FullscreenSourceMode::NormalWorld => {
            preprocess_source_stage_pair(source, &stage.source_stages)
        }
        FullscreenSourceMode::DistantHorizons => {
            preprocess_distant_horizons_fullscreen_stage_pair(source, &stage.source_stages)
        }
    };
    match artifacts {
        Ok(artifacts) => {
            let lowering: GalResult<_> = (|| -> GalResult<_> {
                let declarations = TerrainSourceResourceBindings::from_source(source)?;
                lower_fullscreen_source_pair_with_raster_primitive(
                    &artifacts.vertex,
                    &artifacts.fragment,
                    &declarations,
                    fullscreen_stage_raster_primitive(stage.kind),
                )
            })();
            let resource_bindings = lowering.as_ref().ok().map(|lowered| {
                TerrainSourceResourceBindings::from_source(source).and_then(|declarations| {
                    lowered
                        .opaque_resource_contract()
                        .bind_semantic_roles(&declarations)
                })
            });
            let program_preparation = match (lowering.as_ref(), resource_bindings.as_ref()) {
                (Ok(lowered), Some(Ok(bindings))) => prepare_lowered_fullscreen_source_program(
                    source.name(),
                    source.generation(),
                    &stage.stage_path,
                    lowered,
                    bindings,
                ),
                (Err(error), _) => Err(GalError::unsupported_feature(format!(
                    "fullscreen source lowering for '{}' failed: {error}",
                    stage.stage_path
                ))),
                (_, Some(Err(error))) => Err(GalError::unsupported_feature(format!(
                    "fullscreen source semantic resource binding for '{}' failed: {error}",
                    stage.stage_path
                ))),
                (_, None) => Err(GalError::unsupported_feature(format!(
                    "fullscreen source semantic resource binding for '{}' was not prepared",
                    stage.stage_path
                ))),
            };
            FullscreenSourceStagePreparation {
                stage_path: stage.stage_path.clone(),
                kind: stage.kind,
                source_summary: Some(artifacts.summary()),
                source_preprocess_error: None,
                source_lowering_error: lowering.as_ref().err().map(ToString::to_string),
                source_output_roles: lowering
                    .as_ref()
                    .ok()
                    .map(|lowered| {
                        lowered
                            .fragment()
                            .outputs()
                            .iter()
                            .map(|output| output.role().semantic_name().to_string())
                            .collect()
                    })
                    .unwrap_or_default(),
                source_program_preparation_error: program_preparation
                    .as_ref()
                    .err()
                    .map(ToString::to_string),
                source_program_identity: program_preparation
                    .as_ref()
                    .ok()
                    .map(|program| program.identity.as_str().to_string()),
                source_program: program_preparation.as_ref().ok().cloned(),
                source_feedback_roles: program_preparation
                    .as_ref()
                    .ok()
                    .map(|program| {
                        program
                            .feedback_requirements
                            .iter()
                            .map(|requirement| requirement.role.diagnostic_name())
                            .collect()
                    })
                    .unwrap_or_default(),
                source_mipmap_roles: program_preparation
                    .as_ref()
                    .ok()
                    .map(|program| {
                        program
                            .mipmap_requirements
                            .iter()
                            .map(|requirement| requirement.role.diagnostic_name())
                            .collect()
                    })
                    .unwrap_or_default(),
                source_resource_binding_count: resource_bindings
                    .as_ref()
                    .and_then(|result| result.as_ref().ok())
                    .map(|plan| plan.bindings().len() as u32),
                source_resource_binding_error: resource_bindings
                    .as_ref()
                    .and_then(|result| result.as_ref().err())
                    .map(ToString::to_string),
            }
        }
        Err(error) => FullscreenSourceStagePreparation {
            stage_path: stage.stage_path.clone(),
            kind: stage.kind,
            source_summary: None,
            source_preprocess_error: Some(error.to_string()),
            source_lowering_error: None,
            source_output_roles: Vec::new(),
            source_program_preparation_error: None,
            source_program_identity: None,
            source_program: None,
            source_feedback_roles: Vec::new(),
            source_mipmap_roles: Vec::new(),
            source_resource_binding_count: None,
            source_resource_binding_error: None,
        },
    }
}

fn fullscreen_stage_raster_primitive(
    kind: FullscreenSourceStageKind,
) -> FullscreenSourceRasterPrimitive {
    match kind {
        // `gbuffers_skybasic` reconstructs its camera ray from the actual
        // sky-disc depth field. Keep that geometry source-owned instead of
        // approximating it with a fullscreen triangle.
        // The semantic source sky initializer is a background writer. A
        // fullscreen triangle avoids coupling its coverage to the vanilla
        // disc's camera-space radius while preserving the source fragment
        // shader's exact ray reconstruction and later terrain occlusion.
        FullscreenSourceStageKind::Sky => FullscreenSourceRasterPrimitive::FullscreenTriangle,
        FullscreenSourceStageKind::SkyTextured => {
            FullscreenSourceRasterPrimitive::VanillaCelestialQuad
        }
        FullscreenSourceStageKind::Deferred { .. }
        | FullscreenSourceStageKind::Composite { .. }
        | FullscreenSourceStageKind::Final => FullscreenSourceRasterPrimitive::FullscreenTriangle,
    }
}

#[derive(Debug)]
pub(crate) struct ShaderPackRuntimeExecutor {
    plan: ShaderPackRuntimePlan,
    source_candidate: TerrainSourceCandidateState,
    /// Discovery expands a whole pack and lowers several independent source
    /// families. Keep that work generation-and-scope keyed: source discovery
    /// is immutable until either input changes and must not recur on every
    /// render frame while the selected route is preparing its owned resources.
    source_candidate_scope: Option<TerrainProgramScope>,
    distant_horizons_source_candidate: DistantHorizonsSourceCandidateState,
    /// The DH source pair is discovered independently, but has the same
    /// immutable-input rule as ordinary terrain discovery.
    distant_horizons_source_candidate_scope: Option<TerrainProgramScope>,
    /// Generation-coherent copied vanilla lightmap semantics. This owns only
    /// Rust bytes at this stage; the later sampled-image owner will consume
    /// this cache rather than Java's legacy lightmap texture.
    vanilla_lightmap: VanillaLightmapCache,
    /// Last confirmed Rust-owned sampled lightmap. It remains distinct from a
    /// staged replacement until the exact combined submission has succeeded.
    vanilla_lightmap_residency: Option<VanillaLightmapResidency>,
    pending_vanilla_lightmap_residency: Option<VanillaLightmapResidency>,
    /// Private preparation state for a future selected-source terrain pass.
    /// It owns Rust D3 resources and copied mesh semantics only; it cannot
    /// select source programs or bind terrain material resources.
    terrain_occupancy: Option<TerrainOccupancyRuntime>,
    /// Full private colored-light preparation. It stays unavailable outside
    /// explicit test installation and cannot select a source terrain program.
    terrain_colored_light: Option<TerrainColoredLightRuntime>,
    /// Private source-derived puddle occupancy field. It is a semantic
    /// unsigned image reconstructed from copied translucent terrain, never an
    /// Iris image or an implicit backend object. Resource preparation alone
    /// cannot admit the selected source route.
    terrain_puddle: Option<TerrainPuddleRuntime>,
    /// Decoded pack PNGs referenced by the lowered source binding plan.
    /// These are private Rust-owned GAL resources only. Creating them cannot
    /// select the source program or alter the internal fixture execution.
    source_asset_resources: Option<TerrainSourceAssetResources>,
    /// Rust-owned semantic wrappers around copied material atlases. The world
    /// frontend supplies only private GAL view/sampler pairs; source-plan
    /// lifetime and combined-sampler retirement stay here. Keeping roles
    /// distinct prevents albedo, specular, and future normal atlases from
    /// aliasing one another merely because their extents happen to match.
    source_material_texture_resources:
        BTreeMap<TerrainSourceResourceRole, TerrainSourceMaterialTextureResources>,
    /// Private semantic wrappers around the Rust-owned shadow-depth target.
    /// They are generation-bound compare samplers, not a source program
    /// binding and not evidence that selected-source execution is admitted.
    source_shadow_depth_resources: Option<TerrainSourceShadowDepthResources>,
    /// Private semantic wrappers around the two Rust-owned shadow color
    /// attachments. They expose only the declared source roles and remain
    /// preparation resources until a matching source shadow pass is admitted.
    source_shadow_color_resources: Option<TerrainSourceShadowColorResources>,
    /// Rust-owned semantic wrappers around the current and confirmed main
    /// depth snapshots. These are preparation resources only; incomplete
    /// history remains absent instead of being silently aliased to live depth.
    source_main_depth_resources: Option<TerrainSourceMainDepthResources>,
    /// Exact named source color targets prepared privately for a later
    /// fullscreen executor. Staging these images cannot select a source
    /// route: confirmation remains tied to an eventual combined submission.
    source_color_targets: ShaderPackColorTargetCache,
    /// Program-local combined samplers for the exact named source-color
    /// target generation. Their pending/confirmed lifecycle is coupled to
    /// `source_color_targets`, so discarded target staging cannot leave a
    /// resource set that points at discarded views.
    source_color_resources: ShaderPackSourceColorResourceCache,
}

/// Rust-internal handoff from the world texture cache to the shader runtime.
/// It has no Java, OpenGL, Vulkan, or native-handle representation.
#[derive(Clone, Debug)]
pub(crate) struct TerrainSourceMaterialTextureInput {
    pub role: TerrainSourceResourceRole,
    pub shader_pack_generation: u64,
    pub world_generation: u64,
    pub mesh_asset_generation: u64,
    pub texture_view: Handle,
    pub sampler: Handle,
}

/// Rust-internal handoff from the owned terrain runtime targets to source
/// resource preparation. This is intentionally a GAL view identity only:
/// neither backend objects nor shader-pack-specific binding slots escape.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TerrainSourceShadowDepthInput {
    pub shader_pack_generation: u64,
    pub world_generation: u64,
    pub shader_graph_generation: u64,
    pub shadow_depth_view: Handle,
}

/// Rust-internal handoff from a future owned shadow-color target. The input
/// carries only GAL resource identities and generation semantics; it has no
/// Java, Iris, OpenGL, Vulkan, or native-handle representation.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TerrainSourceShadowColorInput {
    pub shader_pack_generation: u64,
    pub world_generation: u64,
    pub shader_graph_generation: u64,
    pub shadow_color_view: Handle,
    pub shadow_color_secondary_view: Handle,
    pub sampler: Handle,
}

/// Rust-internal handoff from the owned G-buffer to source-resource
/// preparation. Optional snapshots are present only after their own combined
/// frame submission was confirmed by the frontend.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TerrainSourceMainDepthInput {
    pub shader_pack_generation: u64,
    pub world_generation: u64,
    pub shader_graph_generation: u64,
    pub main_depth_view: Handle,
    pub before_translucency_view: Option<Handle>,
    pub previous_view: Option<Handle>,
    pub sampler: Handle,
}

#[derive(Debug)]
struct TerrainSourceMaterialTextureResources {
    role: TerrainSourceResourceRole,
    shader_pack_generation: u64,
    world_generation: u64,
    mesh_asset_generation: u64,
    combined_sampler: Handle,
}

impl TerrainSourceMaterialTextureResources {
    fn compatible_with(&self, input: &TerrainSourceMaterialTextureInput) -> bool {
        self.role == input.role
            && self.shader_pack_generation == input.shader_pack_generation
            && self.world_generation == input.world_generation
            && self.mesh_asset_generation == input.mesh_asset_generation
    }

    fn semantic_resource_set(&self) -> GalResult<TerrainSourceOwnedResourceSet> {
        let availability = super::terrain_source_resources::TerrainSourceResourceAvailabilitySet::new(
            self.shader_pack_generation,
            self.world_generation,
            [super::terrain_source_resources::TerrainSourceResourceAvailability {
                role: self.role.clone(),
                shape: super::terrain_source_resources::TerrainSourceSampledResourceShape::Texture2d,
                resource_generation: self.shader_pack_generation,
            }],
        )?;
        TerrainSourceOwnedResourceSet::new(
            availability,
            [
                super::terrain_source_resources::TerrainSourceOwnedResource {
                    role: self.role.clone(),
                    combined_sampler: self.combined_sampler,
                },
            ],
        )
    }
}

#[derive(Debug)]
struct TerrainSourceShadowDepthResources {
    shader_pack_generation: u64,
    world_generation: u64,
    shader_graph_generation: u64,
    primary_sampler: Handle,
    secondary_sampler: Handle,
    raw_sampler: Option<Handle>,
    primary_combined_sampler: Handle,
    secondary_combined_sampler: Handle,
    raw_combined_sampler: Option<Handle>,
}

#[derive(Debug)]
struct TerrainSourceShadowColorResources {
    shader_pack_generation: u64,
    world_generation: u64,
    shader_graph_generation: u64,
    shadow_color_view: Handle,
    shadow_color_secondary_view: Handle,
    sampler: Handle,
    combined_samplers: BTreeMap<TerrainSourceResourceRole, Handle>,
}

#[derive(Debug)]
struct TerrainSourceMainDepthResources {
    shader_pack_generation: u64,
    world_generation: u64,
    shader_graph_generation: u64,
    main_depth_view: Handle,
    before_translucency_view: Option<Handle>,
    previous_view: Option<Handle>,
    sampler: Handle,
    combined_samplers: BTreeMap<TerrainSourceResourceRole, Handle>,
}

impl TerrainSourceShadowDepthResources {
    fn compatible_with(&self, input: TerrainSourceShadowDepthInput) -> bool {
        self.shader_pack_generation == input.shader_pack_generation
            && self.world_generation == input.world_generation
            && self.shader_graph_generation == input.shader_graph_generation
    }

    fn semantic_resource_set(&self) -> GalResult<TerrainSourceOwnedResourceSet> {
        use super::terrain_source_resources::{
            TerrainSourceResourceAvailability, TerrainSourceResourceAvailabilitySet,
            TerrainSourceSampledResourceShape,
        };

        let availability = TerrainSourceResourceAvailabilitySet::new(
            self.shader_pack_generation,
            self.world_generation,
            [
                TerrainSourceResourceAvailability {
                    role: TerrainSourceResourceRole::ShadowDepthPrimary,
                    shape: TerrainSourceSampledResourceShape::DepthCompareTexture2d,
                    resource_generation: self.shader_graph_generation,
                },
                TerrainSourceResourceAvailability {
                    role: TerrainSourceResourceRole::ShadowDepthSecondary,
                    shape: TerrainSourceSampledResourceShape::DepthCompareTexture2d,
                    resource_generation: self.shader_graph_generation,
                },
            ]
            .into_iter()
            .chain(self.raw_combined_sampler.map(|_| {
                TerrainSourceResourceAvailability {
                    role: TerrainSourceResourceRole::ShadowDepthRaw,
                    shape: TerrainSourceSampledResourceShape::Texture2d,
                    resource_generation: self.shader_graph_generation,
                }
            })),
        )?;
        TerrainSourceOwnedResourceSet::new(
            availability,
            [
                super::terrain_source_resources::TerrainSourceOwnedResource {
                    role: TerrainSourceResourceRole::ShadowDepthPrimary,
                    combined_sampler: self.primary_combined_sampler,
                },
                super::terrain_source_resources::TerrainSourceOwnedResource {
                    role: TerrainSourceResourceRole::ShadowDepthSecondary,
                    combined_sampler: self.secondary_combined_sampler,
                },
            ]
            .into_iter()
            .chain(self.raw_combined_sampler.map(|combined_sampler| {
                super::terrain_source_resources::TerrainSourceOwnedResource {
                    role: TerrainSourceResourceRole::ShadowDepthRaw,
                    combined_sampler,
                }
            })),
        )
    }

    fn destroy(self, gal: &mut VulkanicGal) -> GalResult<()> {
        for handle in [
            self.raw_combined_sampler,
            Some(self.secondary_combined_sampler),
            Some(self.primary_combined_sampler),
            self.raw_sampler,
            Some(self.secondary_sampler),
            Some(self.primary_sampler),
        ]
        .into_iter()
        .flatten()
        {
            gal.destroy(handle)?;
        }
        Ok(())
    }
}

impl TerrainSourceShadowColorResources {
    fn compatible_with(&self, input: TerrainSourceShadowColorInput) -> bool {
        self.shader_pack_generation == input.shader_pack_generation
            && self.world_generation == input.world_generation
            && self.shader_graph_generation == input.shader_graph_generation
            && self.shadow_color_view == input.shadow_color_view
            && self.shadow_color_secondary_view == input.shadow_color_secondary_view
            && self.sampler == input.sampler
    }

    fn semantic_resource_set(&self) -> GalResult<TerrainSourceOwnedResourceSet> {
        use super::terrain_source_resources::{
            TerrainSourceResourceAvailability, TerrainSourceResourceAvailabilitySet,
            TerrainSourceSampledResourceShape,
        };

        let availability = TerrainSourceResourceAvailabilitySet::new(
            self.shader_pack_generation,
            self.world_generation,
            self.combined_samplers
                .keys()
                .cloned()
                .map(|role| TerrainSourceResourceAvailability {
                    role,
                    shape: TerrainSourceSampledResourceShape::Texture2d,
                    resource_generation: self.shader_graph_generation,
                }),
        )?;
        TerrainSourceOwnedResourceSet::new(
            availability,
            self.combined_samplers
                .iter()
                .map(|(role, &combined_sampler)| {
                    super::terrain_source_resources::TerrainSourceOwnedResource {
                        role: role.clone(),
                        combined_sampler,
                    }
                }),
        )
    }

    fn destroy(self, gal: &mut VulkanicGal) -> GalResult<()> {
        for (_, combined_sampler) in self.combined_samplers.into_iter().rev() {
            gal.destroy(combined_sampler)?;
        }
        Ok(())
    }
}

impl TerrainSourceMainDepthResources {
    fn compatible_with(&self, input: TerrainSourceMainDepthInput) -> bool {
        self.shader_pack_generation == input.shader_pack_generation
            && self.world_generation == input.world_generation
            && self.shader_graph_generation == input.shader_graph_generation
            && self.main_depth_view == input.main_depth_view
            && self.before_translucency_view == input.before_translucency_view
            && self.previous_view == input.previous_view
            && self.sampler == input.sampler
    }

    fn base_compatible_with(&self, input: TerrainSourceMainDepthInput) -> bool {
        self.shader_pack_generation == input.shader_pack_generation
            && self.world_generation == input.world_generation
            && self.shader_graph_generation == input.shader_graph_generation
            && self.main_depth_view == input.main_depth_view
            && self.sampler == input.sampler
    }

    fn semantic_resource_set(&self) -> GalResult<TerrainSourceOwnedResourceSet> {
        use super::terrain_source_resources::{
            TerrainSourceResourceAvailability, TerrainSourceResourceAvailabilitySet,
            TerrainSourceSampledResourceShape,
        };

        let availability = TerrainSourceResourceAvailabilitySet::new(
            self.shader_pack_generation,
            self.world_generation,
            self.combined_samplers
                .keys()
                .cloned()
                .map(|role| TerrainSourceResourceAvailability {
                    role,
                    shape: TerrainSourceSampledResourceShape::Texture2d,
                    resource_generation: self.shader_graph_generation,
                }),
        )?;
        TerrainSourceOwnedResourceSet::new(
            availability,
            self.combined_samplers
                .iter()
                .map(|(role, &combined_sampler)| {
                    super::terrain_source_resources::TerrainSourceOwnedResource {
                        role: role.clone(),
                        combined_sampler,
                    }
                }),
        )
    }

    fn destroy(self, gal: &mut VulkanicGal) -> GalResult<()> {
        for (_, handle) in self.combined_samplers.into_iter().rev() {
            gal.destroy(handle)?;
        }
        Ok(())
    }
}

impl ShaderPackRuntimeExecutor {
    pub(crate) fn terrain_material_multipass_v1(generation: u64) -> GalResult<Self> {
        // The ordinary Rust-owned graph is self-contained. Selected-source
        // discovery is an explicit later step because parsing and lowering a
        // pack is meaningful only when a real semantic terrain input exists.
        Self::from_fixture_plan(ShaderPackRuntimePlan::terrain_material_multipass_v1(
            generation,
        )?)
    }

    /// Installs an owned source generation only for contract discovery and
    /// diagnostics. The executable plan remains the internal Rust fixture;
    /// callers cannot accidentally claim selected-source execution merely by
    /// loading pack text.
    pub(crate) fn terrain_material_fixture_from_source(
        source: &ShaderPackSource,
    ) -> GalResult<Self> {
        let generation = source.generation();
        let mut plan = ShaderPackRuntimePlan::terrain_material_multipass_v1(generation)?;
        plan.terrain_contract =
            Some(ShaderPackRuntimePlan::discover_terrain_contract_from_source(generation, source)?);
        Self::from_fixture_plan(plan)
    }

    fn from_fixture_plan(plan: ShaderPackRuntimePlan) -> GalResult<Self> {
        write_contract_diagnostic(&plan);
        Ok(Self {
            plan,
            source_candidate: TerrainSourceCandidateState::Unavailable,
            source_candidate_scope: None,
            distant_horizons_source_candidate: DistantHorizonsSourceCandidateState::Unavailable,
            distant_horizons_source_candidate_scope: None,
            vanilla_lightmap: VanillaLightmapCache::default(),
            vanilla_lightmap_residency: None,
            pending_vanilla_lightmap_residency: None,
            terrain_occupancy: None,
            terrain_colored_light: None,
            terrain_puddle: None,
            source_asset_resources: None,
            source_material_texture_resources: BTreeMap::new(),
            source_shadow_depth_resources: None,
            source_shadow_color_resources: None,
            source_main_depth_resources: None,
            source_color_targets: ShaderPackColorTargetCache::default(),
            source_color_resources: ShaderPackSourceColorResourceCache::default(),
        })
    }

    pub(crate) fn plan(&self) -> &ShaderPackRuntimePlan {
        &self.plan
    }

    pub(crate) fn generation(&self) -> u64 {
        self.plan.generation
    }

    /// Accepts only copied semantic lightmap inputs. It cannot observe or
    /// retain a Java texture, sampler, image view, or renderer state.
    pub(crate) fn observe_vanilla_lightmap(
        &mut self,
        world_generation: u64,
        frame: Option<VanillaLightmapFrame>,
    ) -> GalResult<Option<VanillaLightmapCacheUpdate>> {
        if world_generation == 0 {
            return Err(GalError::invalid_argument(
                "vanilla lightmap observation requires a non-zero world generation",
            ));
        }
        match frame {
            Some(frame) => self
                .vanilla_lightmap
                .update(world_generation, frame)
                .map(Some),
            None => {
                if self.vanilla_lightmap.world_generation() != 0
                    && self.vanilla_lightmap.world_generation() != world_generation
                {
                    self.vanilla_lightmap.clear();
                }
                Ok(None)
            }
        }
    }

    pub(crate) fn vanilla_lightmap_cache(&self) -> &VanillaLightmapCache {
        &self.vanilla_lightmap
    }

    /// Stages a fresh Rust-owned lightmap image into the caller's existing
    /// combined frame submission. The resource is intentionally unavailable
    /// to selected-source assembly until `confirm_vanilla_lightmap_submission`
    /// observes that submission's success.
    pub(crate) fn stage_vanilla_lightmap_residency(
        &mut self,
        gal: &mut VulkanicGal,
        ops: &mut Vec<CommandOp>,
    ) -> GalResult<bool> {
        if self.vanilla_lightmap.world_generation() == 0 {
            return Ok(false);
        }
        if self
            .vanilla_lightmap_residency
            .as_ref()
            .is_some_and(|resources| resources.is_compatible_with(&self.vanilla_lightmap))
        {
            return Ok(false);
        }
        if let Some(pending) = self.pending_vanilla_lightmap_residency.as_ref() {
            // Several semantic consumers can request the same lightmap while
            // one combined frame is being assembled. The pending binding is
            // valid for that exact submission, so a compatible request is a
            // no-op; only a changed generation would need another transaction.
            if pending.is_compatible_with(&self.vanilla_lightmap) {
                return Ok(false);
            }
            return Err(GalError::invalid_argument(
                "vanilla lightmap replacement conflicts with a different pending combined submission",
            ));
        }
        let replacement = VanillaLightmapResidency::create(gal, &self.vanilla_lightmap)?;
        if let Err(error) = replacement.append_upload(&self.vanilla_lightmap, ops) {
            let _ = replacement.destroy(gal);
            return Err(error);
        }
        self.pending_vanilla_lightmap_residency = Some(replacement);
        Ok(true)
    }

    pub(crate) fn has_pending_vanilla_lightmap_submission(&self) -> bool {
        self.pending_vanilla_lightmap_residency.is_some()
    }

    pub(crate) fn confirm_vanilla_lightmap_submission(
        &mut self,
        gal: &mut VulkanicGal,
    ) -> GalResult<()> {
        let Some(replacement) = self.pending_vanilla_lightmap_residency.take() else {
            return Ok(());
        };
        if let Some(previous) = self.vanilla_lightmap_residency.replace(replacement) {
            previous.destroy(gal)?;
        }
        Ok(())
    }

    pub(crate) fn discard_vanilla_lightmap_submission(&mut self, gal: &mut VulkanicGal) {
        if let Some(pending) = self.pending_vanilla_lightmap_residency.take() {
            let _ = pending.destroy(gal);
        }
    }

    pub(crate) fn candidate_vanilla_lightmap_resource_set(
        &self,
        allow_pending: bool,
    ) -> GalResult<Option<TerrainSourceOwnedResourceSet>> {
        if !self.candidate_source_requires_resource(TerrainSourceResourceRole::Lightmap) {
            return Ok(None);
        }
        let resources = if allow_pending {
            self.pending_vanilla_lightmap_residency
                .as_ref()
                .or(self.vanilla_lightmap_residency.as_ref())
        } else {
            self.vanilla_lightmap_residency.as_ref()
        };
        resources
            .map(|resources| {
                resources
                    .semantic_resource_set(self.expected_shader_pack_generation_for_resources())
            })
            .transpose()
    }

    /// Returns only a Rust-owned generation-coherent lightmap binding for
    /// built-in material passes. Unlike the source-candidate accessor above,
    /// this has no shader-pack admission policy: the caller still has to own
    /// a complete pass contract before it can bind or draw with this resource.
    pub(crate) fn vanilla_lightmap_binding(
        &self,
        allow_pending: bool,
    ) -> Option<VanillaLightmapBinding> {
        let resources = if allow_pending {
            self.pending_vanilla_lightmap_residency
                .as_ref()
                .or(self.vanilla_lightmap_residency.as_ref())
        } else {
            self.vanilla_lightmap_residency.as_ref()
        };
        resources.map(VanillaLightmapResidency::binding)
    }

    /// Resolves a caller-declared descriptor layout against the active (or
    /// pending combined-submission) Rust-owned lightmap generation. The
    /// returned set is retained by that residency, not by the caller.
    pub(crate) fn vanilla_lightmap_resource_set(
        &mut self,
        gal: &mut VulkanicGal,
        layout: Handle,
        allow_pending: bool,
    ) -> GalResult<Option<VanillaLightmapResourceSet>> {
        let resources = if allow_pending {
            self.pending_vanilla_lightmap_residency
                .as_mut()
                .or(self.vanilla_lightmap_residency.as_mut())
        } else {
            self.vanilla_lightmap_residency.as_mut()
        };
        resources
            .map(|resources| {
                Ok(VanillaLightmapResourceSet {
                    world_generation: resources.binding().world_generation,
                    lightmap_generation: resources.binding().lightmap_generation,
                    set: resources.resource_set_for_layout(gal, layout)?,
                })
            })
            .transpose()
    }

    pub(crate) fn observe_source_candidate(&mut self, source: &ShaderPackSource) {
        self.observe_source_candidate_for_scope(source, TerrainProgramScope::Default);
    }

    fn source_candidate_matches(
        &self,
        source: &ShaderPackSource,
        scope: TerrainProgramScope,
    ) -> bool {
        if self.source_candidate_scope != Some(scope) {
            return false;
        }
        match &self.source_candidate {
            TerrainSourceCandidateState::Unavailable => false,
            TerrainSourceCandidateState::Disabled {
                generation,
                pack_name,
            }
            | TerrainSourceCandidateState::Rejected {
                generation,
                pack_name,
                ..
            }
            | TerrainSourceCandidateState::Discovered {
                generation,
                pack_name,
                ..
            } => *generation == source.generation() && pack_name == source.name(),
        }
    }

    fn distant_horizons_source_candidate_matches(
        &self,
        source: &ShaderPackSource,
        scope: TerrainProgramScope,
    ) -> bool {
        if self.distant_horizons_source_candidate_scope != Some(scope) {
            return false;
        }
        match &self.distant_horizons_source_candidate {
            DistantHorizonsSourceCandidateState::Unavailable => false,
            DistantHorizonsSourceCandidateState::Disabled {
                generation,
                pack_name,
            }
            | DistantHorizonsSourceCandidateState::Rejected {
                generation,
                pack_name,
                ..
            }
            | DistantHorizonsSourceCandidateState::Discovered {
                generation,
                pack_name,
                ..
            } => *generation == source.generation() && pack_name == source.name(),
        }
    }

    /// Discovers the pack's distinct Distant Horizons pair without borrowing
    /// DH or Iris runtime state. This is separate from normal terrain source
    /// discovery and is intentionally preparation-only: no targets, pipeline,
    /// resource set, command, or route decision is created here.
    pub(crate) fn observe_distant_horizons_source_candidate_for_scope(
        &mut self,
        source: &ShaderPackSource,
        scope: TerrainProgramScope,
    ) {
        if self.distant_horizons_source_candidate_matches(source, scope) {
            return;
        }
        if source.is_empty() {
            self.distant_horizons_source_candidate =
                DistantHorizonsSourceCandidateState::Disabled {
                    generation: source.generation(),
                    pack_name: source.name().to_string(),
                };
            self.distant_horizons_source_candidate_scope = Some(scope);
            return;
        }

        self.distant_horizons_source_candidate = match derive_distant_horizons_opaque_contract(
            source, scope,
        ) {
            Ok(contract) => {
                let translucent_contract = if scope
                    .distant_horizons_translucent_entry_candidates()
                    .iter()
                    .all(|path| source.get(path).is_none())
                {
                    DistantHorizonsTranslucentSourceCandidate::Unavailable
                } else {
                    match derive_distant_horizons_translucent_contract(source, scope) {
                        Ok(contract) => {
                            DistantHorizonsTranslucentSourceCandidate::Discovered(contract)
                        }
                        Err(error) => {
                            DistantHorizonsTranslucentSourceCandidate::Rejected(error.to_string())
                        }
                    }
                };
                let (
                    translucent_source_summary,
                    translucent_source_preprocess_error,
                    translucent_source_lowering_error,
                    translucent_source_lowered_pair,
                    translucent_source_resource_binding_error,
                    translucent_source_resource_bindings,
                ) = match &translucent_contract {
                    DistantHorizonsTranslucentSourceCandidate::Discovered(contract) => {
                        match preprocess_distant_horizons_sources(source, &contract.source_stages) {
                            Ok(artifacts) => {
                                match lower_distant_horizons_source_pair(
                                    &artifacts.vertex,
                                    &artifacts.fragment,
                                ) {
                                    Ok(lowered) => {
                                        let resource_bindings: GalResult<_> =
                                            (|| -> GalResult<_> {
                                                let bindings =
                                                    TerrainSourceResourceBindings::from_source(
                                                        source,
                                                    )?;
                                                lowered
                                                    .opaque_resource_contract()
                                                    .bind_semantic_roles(&bindings)
                                            })();
                                        (
                                            Some(artifacts.summary()),
                                            None,
                                            None,
                                            Some(lowered),
                                            resource_bindings
                                                .as_ref()
                                                .err()
                                                .map(ToString::to_string),
                                            resource_bindings.ok(),
                                        )
                                    }
                                    Err(error) => (
                                        Some(artifacts.summary()),
                                        None,
                                        Some(error.to_string()),
                                        None,
                                        None,
                                        None,
                                    ),
                                }
                            }
                            Err(error) => (None, Some(error.to_string()), None, None, None, None),
                        }
                    }
                    DistantHorizonsTranslucentSourceCandidate::Unavailable
                    | DistantHorizonsTranslucentSourceCandidate::Rejected(_) => {
                        (None, None, None, None, None, None)
                    }
                };
                let (
                    source_summary,
                    source_preprocess_error,
                    source_lowering_error,
                    source_lowered_pair,
                    source_uniform_requirement_summary,
                    source_uniform_requirement_error,
                    source_resource_binding_count,
                    source_resource_binding_error,
                    source_resource_bindings,
                ) = match preprocess_distant_horizons_sources(source, &contract.source_stages) {
                    Ok(artifacts) => match lower_distant_horizons_source_pair(
                        &artifacts.vertex,
                        &artifacts.fragment,
                    ) {
                        Ok(lowered) => {
                            let uniform_requirements =
                                TerrainSourceUniformRequirements::from_contract(
                                    lowered.uniform_contract(),
                                );
                            let resource_bindings: GalResult<_> = (|| -> GalResult<_> {
                                let bindings = TerrainSourceResourceBindings::from_source(source)?;
                                lowered
                                    .opaque_resource_contract()
                                    .bind_semantic_roles(&bindings)
                            })();
                            (
                                Some(artifacts.summary()),
                                None,
                                None,
                                Some(lowered),
                                uniform_requirements
                                    .as_ref()
                                    .ok()
                                    .map(TerrainSourceUniformRequirements::summary),
                                uniform_requirements.as_ref().err().map(ToString::to_string),
                                resource_bindings
                                    .as_ref()
                                    .ok()
                                    .map(|plan| plan.bindings().len() as u32),
                                resource_bindings.as_ref().err().map(ToString::to_string),
                                resource_bindings.ok(),
                            )
                        }
                        Err(error) => (
                            Some(artifacts.summary()),
                            None,
                            Some(error.to_string()),
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                        ),
                    },
                    Err(error) => (
                        None,
                        Some(error.to_string()),
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                    ),
                };
                let source_color_targets: GalResult<_> = (|| -> GalResult<_> {
                    let bindings = TerrainSourceResourceBindings::from_source(source)?;
                    ShaderPackColorTargetManifest::from_source(source, &bindings)
                })();
                let source_color_target_count = source_color_targets
                    .as_ref()
                    .ok()
                    .map(|targets| targets.targets().count() as u32);
                let source_color_target_error =
                    source_color_targets.as_ref().err().map(ToString::to_string);
                let source_color_target_gal_schema_error = source_color_targets
                    .as_ref()
                    .ok()
                    .and_then(|targets| targets.require_gal_schema_formats().err())
                    .map(|error| error.to_string());
                let depth_consumer_preparation = contract
                        .distant_depth_consumers
                        .iter()
                        .map(|consumer| {
                            match preprocess_distant_horizons_fullscreen_stage_pair(
                                source,
                                &consumer.source_stages,
                            ) {
                                Ok(artifacts) => {
                                    let lowering: GalResult<_> = (|| -> GalResult<_> {
                                        let declarations =
                                            TerrainSourceResourceBindings::from_source(source)?;
                                        lower_fullscreen_source_pair(
                                            &artifacts.vertex,
                                            &artifacts.fragment,
                                            &declarations,
                                        )
                                    })();
                                    let resource_bindings = lowering.as_ref().ok().map(|lowered| {
                                        TerrainSourceResourceBindings::from_source(source).and_then(
                                            |declarations| {
                                                lowered
                                                    .opaque_resource_contract()
                                                    .bind_semantic_roles(&declarations)
                                            },
                                        )
                                    });
                                    let program_preparation = match (
                                        lowering.as_ref(),
                                        resource_bindings.as_ref(),
                                    ) {
                                        (Ok(lowered), Some(Ok(bindings))) => {
                                            prepare_lowered_fullscreen_source_program(
                                                source.name(),
                                                source.generation(),
                                                &consumer.stage_path,
                                                lowered,
                                                bindings,
                                            )
                                        }
                                        (Err(error), _) => Err(GalError::unsupported_feature(
                                            format!(
                                                "fullscreen source lowering for '{}' failed: {error}",
                                                consumer.stage_path
                                            ),
                                        )),
                                        (_, Some(Err(error))) => Err(GalError::unsupported_feature(
                                            format!(
                                                "fullscreen source semantic resource binding for '{}' failed: {error}",
                                                consumer.stage_path
                                            ),
                                        )),
                                        (_, None) => Err(GalError::unsupported_feature(
                                            format!(
                                                "fullscreen source semantic resource binding for '{}' was not prepared",
                                                consumer.stage_path
                                            ),
                                        )),
                                    };
                                    DistantHorizonsDepthConsumerPreparation {
                                        stage_path: consumer.stage_path.clone(),
                                        reads_opaque_depth: consumer.reads_opaque_depth,
                                        reads_depth_before_translucency: consumer
                                            .reads_depth_before_translucency,
                                        source_summary: Some(artifacts.summary()),
                                        source_preprocess_error: None,
                                        source_lowering_error: lowering
                                            .as_ref()
                                            .err()
                                            .map(ToString::to_string),
                                        source_output_roles: lowering
                                            .as_ref()
                                            .ok()
                                            .map(|lowered| {
                                                lowered
                                                    .fragment()
                                                    .outputs()
                                                    .iter()
                                                    .map(|output| {
                                                        output
                                                            .role()
                                                            .semantic_name()
                                                            .to_string()
                                                    })
                                                    .collect()
                                            })
                                            .unwrap_or_default(),
                                        source_program_preparation_error: program_preparation
                                            .as_ref()
                                            .err()
                                            .map(ToString::to_string),
                                        source_program_identity: program_preparation
                                            .as_ref()
                                            .ok()
                                            .map(|program| program.identity.as_str().to_string()),
                                        source_program: program_preparation.as_ref().ok().cloned(),
                                        source_feedback_roles: program_preparation
                                            .as_ref()
                                            .ok()
                                            .map(|program| {
                                                program
                                                    .feedback_requirements
                                                    .iter()
                                                    .map(|requirement| {
                                                        requirement.role.diagnostic_name()
                                                    })
                                                    .collect()
                                            })
                                            .unwrap_or_default(),
                                        source_mipmap_roles: program_preparation
                                            .as_ref()
                                            .ok()
                                            .map(|program| {
                                                program
                                                    .mipmap_requirements
                                                    .iter()
                                                    .map(|requirement| {
                                                        requirement.role.diagnostic_name()
                                                    })
                                                    .collect()
                                            })
                                            .unwrap_or_default(),
                                        source_resource_binding_count: resource_bindings
                                            .as_ref()
                                            .and_then(|result| result.as_ref().ok())
                                            .map(|plan| plan.bindings().len() as u32),
                                        source_resource_binding_error: resource_bindings
                                            .as_ref()
                                            .and_then(|result| result.as_ref().err())
                                            .map(|error| error.to_string()),
                                    }
                                }
                                Err(error) => DistantHorizonsDepthConsumerPreparation {
                                    stage_path: consumer.stage_path.clone(),
                                    reads_opaque_depth: consumer.reads_opaque_depth,
                                    reads_depth_before_translucency: consumer
                                        .reads_depth_before_translucency,
                                    source_summary: None,
                                    source_preprocess_error: Some(error.to_string()),
                                    source_lowering_error: None,
                                    source_output_roles: Vec::new(),
                                    source_program_preparation_error: None,
                                    source_program_identity: None,
                                    source_program: None,
                                    source_feedback_roles: Vec::new(),
                                    source_mipmap_roles: Vec::new(),
                                    source_resource_binding_count: None,
                                    source_resource_binding_error: None,
                                },
                            }
                        })
                        .collect();
                let (post_terrain_preparation, post_terrain_preparation_error) =
                    match derive_fullscreen_source_chain(source, scope) {
                        Ok(stages) => (
                            stages
                                .iter()
                                .map(|stage| {
                                    prepare_fullscreen_source_stage(
                                        source,
                                        stage,
                                        FullscreenSourceMode::DistantHorizons,
                                    )
                                })
                                .collect(),
                            None,
                        ),
                        Err(error) => (Vec::new(), Some(error.to_string())),
                    };
                DistantHorizonsSourceCandidateState::Discovered {
                    generation: source.generation(),
                    pack_name: source.name().to_string(),
                    contract,
                    translucent_contract,
                    translucent_source_summary,
                    translucent_source_preprocess_error,
                    translucent_source_lowering_error,
                    translucent_source_lowered_pair,
                    translucent_source_resource_binding_error,
                    translucent_source_resource_bindings,
                    source_summary,
                    source_preprocess_error,
                    source_lowering_error,
                    source_lowered_pair,
                    source_uniform_requirement_summary,
                    source_uniform_requirement_error,
                    source_resource_binding_count,
                    source_resource_binding_error,
                    source_resource_bindings,
                    source_color_target_count,
                    source_color_target_error,
                    source_color_target_gal_schema_error,
                    source_color_targets: source_color_targets.ok(),
                    depth_consumer_preparation,
                    post_terrain_preparation,
                    post_terrain_preparation_error,
                }
            }
            Err(error) => DistantHorizonsSourceCandidateState::Rejected {
                generation: source.generation(),
                pack_name: source.name().to_string(),
                reason: error.to_string(),
            },
        };
        self.distant_horizons_source_candidate_scope = Some(scope);
    }

    pub(crate) fn distant_horizons_source_candidate(&self) -> &DistantHorizonsSourceCandidateState {
        &self.distant_horizons_source_candidate
    }

    /// Returns only the separately discovered DH translucent source state.
    /// Callers may use it to explain why a semantically translucent LOD range
    /// was not admitted, but cannot use it as a route or backend capability.
    pub(crate) fn distant_horizons_translucent_source_candidate(
        &self,
    ) -> Option<&DistantHorizonsTranslucentSourceCandidate> {
        match &self.distant_horizons_source_candidate {
            DistantHorizonsSourceCandidateState::Discovered {
                translucent_contract,
                ..
            } => Some(translucent_contract),
            DistantHorizonsSourceCandidateState::Unavailable
            | DistantHorizonsSourceCandidateState::Disabled { .. }
            | DistantHorizonsSourceCandidateState::Rejected { .. } => None,
        }
    }

    /// Returns the one pack-wide named color manifest shared by ordinary
    /// terrain and any retained Distant Horizons fullscreen consumers. Both
    /// discovery paths parse the same source generation, so disagreement is
    /// a hard source-contract error rather than an opportunity to allocate
    /// separate, silently divergent target sets.
    fn source_color_target_manifest(&self) -> GalResult<Option<&ShaderPackColorTargetManifest>> {
        let terrain = match &self.source_candidate {
            TerrainSourceCandidateState::Discovered {
                source_color_targets,
                ..
            } => source_color_targets.as_ref(),
            TerrainSourceCandidateState::Unavailable
            | TerrainSourceCandidateState::Disabled { .. }
            | TerrainSourceCandidateState::Rejected { .. } => None,
        };
        let distant_horizons = match &self.distant_horizons_source_candidate {
            DistantHorizonsSourceCandidateState::Discovered {
                source_color_targets,
                ..
            } => source_color_targets.as_ref(),
            DistantHorizonsSourceCandidateState::Unavailable
            | DistantHorizonsSourceCandidateState::Disabled { .. }
            | DistantHorizonsSourceCandidateState::Rejected { .. } => None,
        };
        match (terrain, distant_horizons) {
            (Some(terrain), Some(distant_horizons)) if terrain != distant_horizons => {
                Err(GalError::invalid_argument(
                    "ordinary terrain and Distant Horizons source discovery disagree on the shader-pack color target manifest",
                ))
            }
            (Some(manifest), _) | (_, Some(manifest)) => Ok(Some(manifest)),
            (None, None) => Ok(None),
        }
    }

    /// Resolves the feedback and mip history required by every scoped
    /// fullscreen stage in the normal world source contract. This is kept
    /// separate from the existing DH depth-consumer preparation because that
    /// subset is not a valid proxy for the complete vanilla/DH composite
    /// chain. A missing or partially lowered stage keeps complete target
    /// staging unavailable rather than allocating a target generation with
    /// insufficient history images.
    fn complete_source_color_target_requirements(&self) -> GalResult<(Vec<String>, Vec<String>)> {
        let (preparation, preparation_error) = match &self.source_candidate {
            TerrainSourceCandidateState::Discovered {
                post_terrain_preparation,
                post_terrain_preparation_error,
                ..
            } => (
                post_terrain_preparation.as_slice(),
                post_terrain_preparation_error.as_deref(),
            ),
            TerrainSourceCandidateState::Unavailable
            | TerrainSourceCandidateState::Disabled { .. }
            | TerrainSourceCandidateState::Rejected { .. } => return Ok((Vec::new(), Vec::new())),
        };
        if let Some(error) = preparation_error {
            return Err(GalError::unsupported_feature(format!(
                "complete shader-pack fullscreen chain cannot stage named color targets: {error}"
            )));
        }
        if preparation.is_empty() {
            return Err(GalError::unsupported_feature(
                "complete shader-pack fullscreen chain has no retained stages",
            ));
        }
        let mut feedback_names = Vec::new();
        let mut mipmapped_names = Vec::new();
        for stage in preparation {
            if let Some(error) = stage
                .source_preprocess_error
                .as_deref()
                .or(stage.source_lowering_error.as_deref())
                .or(stage.source_program_preparation_error.as_deref())
                .or(stage.source_resource_binding_error.as_deref())
            {
                return Err(GalError::unsupported_feature(format!(
                    "complete shader-pack fullscreen stage '{}' is not prepared: {error}",
                    stage.stage_path
                )));
            }
            if stage.source_program.is_none() {
                return Err(GalError::unsupported_feature(format!(
                    "complete shader-pack fullscreen stage '{}' has no retained lowered program",
                    stage.stage_path
                )));
            }
            feedback_names.extend(
                stage
                    .source_feedback_roles
                    .iter()
                    .map(|role| shader_pack_color_name_from_role(role))
                    .collect::<GalResult<Vec<_>>>()?,
            );
            mipmapped_names.extend(
                stage
                    .source_mipmap_roles
                    .iter()
                    .map(|role| shader_pack_color_name_from_role(role))
                    .collect::<GalResult<Vec<_>>>()?,
            );
        }
        Ok((feedback_names, mipmapped_names))
    }

    /// Stages exact Rust-owned named pack-color images shared by ordinary
    /// terrain and Distant Horizons source consumers. This is source-resource
    /// preparation only: it creates no pipeline, pass, descriptor set, draw,
    /// route, or presenter, and callers must confirm/discard it with their
    /// future combined submission outcome.
    pub(crate) fn stage_source_color_targets(
        &mut self,
        gal: &mut VulkanicGal,
        world_generation: u64,
        extent: crate::render::vulkanic::resources::Extent3d,
    ) -> GalResult<Option<ShaderPackColorTargets>> {
        let manifest = match self.source_color_target_manifest()? {
            Some(manifest) => manifest.clone(),
            None => return Ok(None),
        };
        let generation = manifest.generation();
        let (feedback_names, mipmapped_names) = match &self.distant_horizons_source_candidate {
            DistantHorizonsSourceCandidateState::Discovered {
                depth_consumer_preparation,
                ..
            } => {
                let feedback_names = depth_consumer_preparation
                    .iter()
                    .flat_map(|consumer| consumer.source_feedback_roles.iter())
                    .map(|role| {
                        role.strip_prefix("shader_pack_color:")
                            .map(str::to_string)
                            .ok_or_else(|| {
                                GalError::invalid_argument(format!(
                                    "fullscreen feedback role '{role}' is not a named shader-pack color target"
                                ))
                            })
                    })
                    .collect::<GalResult<Vec<_>>>()?;
                let mipmapped_names = depth_consumer_preparation
                    .iter()
                    .flat_map(|consumer| consumer.source_mipmap_roles.iter())
                    .map(|role| {
                        role.strip_prefix("shader_pack_color:")
                            .map(str::to_string)
                            .ok_or_else(|| {
                                GalError::invalid_argument(format!(
                                    "fullscreen mipmap role '{role}' is not a named shader-pack color target"
                                ))
                            })
                    })
                    .collect::<GalResult<Vec<_>>>()?;
                (feedback_names, mipmapped_names)
            }
            DistantHorizonsSourceCandidateState::Unavailable
            | DistantHorizonsSourceCandidateState::Disabled { .. }
            | DistantHorizonsSourceCandidateState::Rejected { .. } => (Vec::new(), Vec::new()),
        };
        let identity = ShaderPackColorTargetIdentity::new(
            world_generation,
            generation,
            extent,
            feedback_names,
            mipmapped_names,
        )?;
        self.source_color_targets
            .stage(gal, identity, &manifest)
            .map(Some)
    }

    /// Stages the one complete named-color target generation required by the
    /// full scoped source chain. This remains private preparation: it does
    /// not record a fullscreen draw, select a route, or present a frame.
    /// Future normal vanilla terrain and DH execution must use this entry
    /// point together, rather than deriving feedback/mips from whichever
    /// geometry family happened to be visible first.
    pub(crate) fn stage_complete_source_color_targets(
        &mut self,
        gal: &mut VulkanicGal,
        world_generation: u64,
        extent: crate::render::vulkanic::resources::Extent3d,
    ) -> GalResult<Option<ShaderPackColorTargets>> {
        let manifest = match self.source_color_target_manifest()? {
            Some(manifest) => manifest.clone(),
            None => return Ok(None),
        };
        let (feedback_names, mipmapped_names) = self.complete_source_color_target_requirements()?;
        let identity = ShaderPackColorTargetIdentity::new(
            world_generation,
            manifest.generation(),
            extent,
            feedback_names,
            mipmapped_names,
        )?;
        self.source_color_targets
            .stage(gal, identity, &manifest)
            .map(Some)
    }

    /// Resolves one lowered normal-terrain program's named outputs through
    /// the selected pack manifest and staged Rust-owned target generation.
    /// The result is preparation data for the world frontend, which owns the
    /// explicit depth target, render pass, source transaction, and combined
    /// submission. Keeping that split prevents either ordinary terrain or DH
    /// from treating legacy source slots as backend attachments.
    pub(crate) fn resolve_terrain_source_color_outputs(
        &self,
        program: &LoweredTerrainSourceProgram,
        targets: &ShaderPackColorTargets,
    ) -> GalResult<Vec<TerrainSourceColorAttachment>> {
        let manifest = self.source_color_target_manifest()?.ok_or_else(|| {
            GalError::invalid_argument(
                "lowered terrain source program has no selected shader-pack color target manifest",
            )
        })?;
        if program.shader_pack_generation != manifest.generation()
            || program.shader_pack_generation != targets.identity.shader_pack_generation
        {
            return Err(GalError::invalid_argument(
                "lowered terrain source program, color manifest, and staged targets must share one shader-pack generation",
            ));
        }
        let output_color_slots = program.terrain_output_color_slots().ok_or_else(|| {
            GalError::invalid_argument(
                "shadow source programs cannot resolve normal-terrain shader-pack color outputs",
            )
        })?;
        resolve_terrain_source_color_attachments(output_color_slots, manifest, targets)
    }

    /// Resolves the distinct `gbuffers_textured` source writer through the
    /// same Rust-owned named-color generation as terrain. The program keeps
    /// its own output schema and sampler plan; this helper deliberately does
    /// not treat generic material as a terrain mesh or reuse terrain bindings.
    pub(crate) fn resolve_textured_material_source_color_outputs(
        &self,
        program: &LoweredTexturedMaterialSourceProgram,
        targets: &ShaderPackColorTargets,
    ) -> GalResult<Vec<TerrainSourceColorAttachment>> {
        let manifest = self.source_color_target_manifest()?.ok_or_else(|| {
            GalError::invalid_argument(
                "lowered textured material source program has no selected shader-pack color target manifest",
            )
        })?;
        if program.shader_pack_generation != manifest.generation()
            || program.shader_pack_generation != targets.identity.shader_pack_generation
        {
            return Err(GalError::invalid_argument(
                "textured material source program, color manifest, and staged targets must share one shader-pack generation",
            ));
        }
        resolve_terrain_source_color_attachments(
            program.named_output_color_slots(),
            manifest,
            targets,
        )
    }

    /// Resolves the distinct `gbuffers_entities` writer through the same
    /// Rust-owned named-color generation as terrain. Entity source programs
    /// retain their local-material contract and can never select legacy
    /// draw-buffer slots or Java/Iris targets directly.
    pub(crate) fn resolve_entity_source_color_outputs(
        &self,
        program: &LoweredEntitySourceProgram,
        targets: &ShaderPackColorTargets,
    ) -> GalResult<Vec<TerrainSourceColorAttachment>> {
        let manifest = self.source_color_target_manifest()?.ok_or_else(|| {
            GalError::invalid_argument(
                "lowered entity source program has no selected shader-pack color target manifest",
            )
        })?;
        if program.shader_pack_generation != manifest.generation()
            || program.shader_pack_generation != targets.identity.shader_pack_generation
        {
            return Err(GalError::invalid_argument(
                "entity source program, color manifest, and staged targets must share one shader-pack generation",
            ));
        }
        resolve_terrain_source_color_attachments(
            program.named_output_color_slots(),
            manifest,
            targets,
        )
    }

    /// Resolves the separately lowered `gbuffers_hand` source through the
    /// exact Rust-owned named-color generation. The caller must still supply
    /// its explicit first-person depth boundary; this helper cannot reuse a
    /// Java/Iris target or select a hand route.
    pub(crate) fn resolve_hand_source_color_outputs(
        &self,
        program: &LoweredHandSourceProgram,
        targets: &ShaderPackColorTargets,
    ) -> GalResult<Vec<TerrainSourceColorAttachment>> {
        let manifest = self.source_color_target_manifest()?.ok_or_else(|| {
            GalError::invalid_argument(
                "lowered hand source program has no selected shader-pack color target manifest",
            )
        })?;
        if program.shader_pack_generation != manifest.generation()
            || program.shader_pack_generation != targets.identity.shader_pack_generation
        {
            return Err(GalError::invalid_argument(
                "hand source program, color manifest, and staged targets must share one shader-pack generation",
            ));
        }
        resolve_terrain_source_color_attachments(
            program.named_output_color_slots(),
            manifest,
            targets,
        )
    }

    /// Stages the named shader-pack color sampler subset for one ordinary
    /// terrain program. Normal terrain has no feedback or mip policy of its
    /// own; those concerns are explicit only on lowered fullscreen stages.
    /// The returned table is semantic and caller-owned only as a clone of the
    /// cache's handles, suitable for merging into the exact frame snapshot.
    pub(crate) fn stage_terrain_source_color_resources(
        &mut self,
        gal: &mut VulkanicGal,
        program: &LoweredTerrainSourceProgram,
        targets: &ShaderPackColorTargets,
    ) -> GalResult<TerrainSourceOwnedResourceSet> {
        self.source_color_resources.stage(
            gal,
            program.shader_pack_generation,
            &program.opaque_resource_bindings,
            &ShaderPackColorSamplingPlan::default(),
            targets,
        )
    }

    /// Stages only the named color sampler subset required by the prepared
    /// textured-material source program. The cache owns the generated GAL
    /// sets and retires them with the source generation; this returns no Java
    /// or backend handle and cannot select a rendering route.
    pub(crate) fn stage_textured_material_source_color_resources(
        &mut self,
        gal: &mut VulkanicGal,
        program: &LoweredTexturedMaterialSourceProgram,
        targets: &ShaderPackColorTargets,
    ) -> GalResult<TerrainSourceOwnedResourceSet> {
        self.source_color_resources.stage(
            gal,
            program.shader_pack_generation,
            &program.opaque_resource_bindings,
            &ShaderPackColorSamplingPlan::default(),
            targets,
        )
    }

    /// Stages only the named color sampler subset declared by the distinct
    /// entity source contract. The caller adds its local material texture
    /// separately; this cache never aliases it to a terrain atlas resource.
    pub(crate) fn stage_terrain_source_color_resources_for_entity(
        &mut self,
        gal: &mut VulkanicGal,
        program: &LoweredEntitySourceProgram,
        targets: &ShaderPackColorTargets,
    ) -> GalResult<TerrainSourceOwnedResourceSet> {
        self.source_color_resources.stage(
            gal,
            program.shader_pack_generation,
            &program.opaque_resource_bindings,
            &ShaderPackColorSamplingPlan::default(),
            targets,
        )
    }

    /// Stages only the named shader-pack samplers declared by the hand
    /// source. A later first-person writer adds its Rust-owned material
    /// texture and stream separately, so this cannot inherit entity or Java
    /// renderer resources.
    pub(crate) fn stage_terrain_source_color_resources_for_hand(
        &mut self,
        gal: &mut VulkanicGal,
        program: &LoweredHandSourceProgram,
        targets: &ShaderPackColorTargets,
    ) -> GalResult<TerrainSourceOwnedResourceSet> {
        self.source_color_resources.stage(
            gal,
            program.shader_pack_generation,
            &program.opaque_resource_bindings,
            &ShaderPackColorSamplingPlan::default(),
            targets,
        )
    }

    /// Equivalent named-color resource preparation for the distinct Distant
    /// Horizons source ABI. DH stays semantically separate at geometry/depth
    /// level but samples the same Rust-owned pack color generation.
    pub(crate) fn stage_distant_horizons_source_color_resources(
        &mut self,
        gal: &mut VulkanicGal,
        program: &LoweredDistantHorizonsSourceProgram,
        targets: &ShaderPackColorTargets,
    ) -> GalResult<TerrainSourceOwnedResourceSet> {
        self.source_color_resources.stage(
            gal,
            program.shader_pack_generation,
            &program.opaque_resource_bindings,
            &ShaderPackColorSamplingPlan::default(),
            targets,
        )
    }

    pub(crate) fn confirm_source_color_targets_submission(&mut self, gal: &mut VulkanicGal) {
        self.source_color_targets.confirm_submission(gal);
    }

    /// Promotes program-local named-color sampler tables only after the same
    /// combined source submission succeeds. Call this before target promotion
    /// so wrappers referencing an old target generation retire before those
    /// target images/views are retired.
    pub(crate) fn confirm_source_color_resources_submission(
        &mut self,
        gal: &mut VulkanicGal,
    ) -> GalResult<()> {
        self.source_color_resources.confirm_submission(gal)
    }

    /// Begins a private semantic color schedule for an exact staged source
    /// target generation. Both ordinary terrain and Distant Horizons use this
    /// same owner; it neither selects a route nor borrows an Iris target.
    pub(crate) fn begin_source_color_frame(
        &self,
        targets: &ShaderPackColorTargets,
    ) -> GalResult<ShaderPackColorFramePlan> {
        self.source_color_targets.begin_frame(targets)
    }

    /// Starts the reusable named-color portion of one combined source frame.
    /// It is intentionally route-neutral: ordinary terrain and Distant
    /// Horizons may both append their own semantic draws, but neither owns
    /// color-history validity or native render-target state.
    pub(crate) fn begin_source_color_transaction(
        &self,
        gal: &mut VulkanicGal,
        targets: &ShaderPackColorTargets,
        clear_values: ShaderPackColorBootstrapClearValues,
        operations: &mut Vec<CommandOp>,
    ) -> GalResult<ShaderPackSourceColorFrameTransaction> {
        ShaderPackSourceColorFrameTransaction::begin(
            gal,
            targets,
            self.begin_source_color_frame(targets)?,
            clear_values,
            operations,
        )
    }

    /// Advances named color/history validity only after the combined source
    /// submission is accepted. A failed frame must call the existing discard
    /// path and cannot become feedback input for either terrain family.
    pub(crate) fn confirm_source_color_frame_submission(
        &mut self,
        gal: &mut VulkanicGal,
        frame: ShaderPackColorFramePlan,
    ) -> GalResult<()> {
        self.source_color_targets
            .confirm_frame_submission(gal, frame)
    }

    /// Confirms both halves of one accepted source-color submission. Frame
    /// history becomes visible first; only then may an initial staged target
    /// generation become the active reusable generation. A failed history
    /// confirmation leaves target staging discardable rather than retaining a
    /// generation whose semantic contents were never accepted.
    fn confirm_source_color_transaction_submission(
        &mut self,
        gal: &mut VulkanicGal,
        frame: ShaderPackColorFramePlan,
    ) -> GalResult<()> {
        self.confirm_source_color_frame_submission(gal, frame)?;
        // Retire sampler wrappers before source-color target replacement can
        // retire the images/views they reference.
        self.confirm_source_color_resources_submission(gal)?;
        self.confirm_source_color_targets_submission(gal);
        Ok(())
    }

    pub(crate) fn discard_source_color_targets_submission(&mut self, gal: &mut VulkanicGal) {
        self.source_color_resources.discard_submission(gal);
        self.source_color_targets.discard_submission(gal);
    }

    /// Prepares exactly the DH source stages retained during discovery. The
    /// caller still needs a Rust-owned distant-depth target, a selected shared
    /// pack-color target, and every source-derived depth consumer before this
    /// artifact can ever be executed.
    pub(crate) fn prepared_lowered_distant_horizons_source_program(
        &self,
    ) -> GalResult<Option<LoweredDistantHorizonsSourceProgram>> {
        match &self.distant_horizons_source_candidate {
            DistantHorizonsSourceCandidateState::Unavailable
            | DistantHorizonsSourceCandidateState::Disabled { .. }
            | DistantHorizonsSourceCandidateState::Rejected { .. } => Ok(None),
            DistantHorizonsSourceCandidateState::Discovered {
                contract,
                source_summary: Some(_),
                source_preprocess_error: None,
                source_lowering_error: None,
                source_lowered_pair: Some(lowered),
                source_uniform_requirement_summary: Some(summary),
                source_uniform_requirement_error: None,
                source_resource_binding_count: Some(_),
                source_resource_binding_error: None,
                source_resource_bindings: Some(bindings),
                ..
            } if summary.field_count == summary.resolved_field_count => {
                prepare_lowered_distant_horizons_source_program(contract, lowered, bindings)
                    .map(Some)
            }
            DistantHorizonsSourceCandidateState::Discovered {
                source_preprocess_error,
                source_lowering_error,
                source_uniform_requirement_summary,
                source_uniform_requirement_error,
                source_resource_binding_error,
                ..
            } => {
                let uniform_status = source_uniform_requirement_error.clone().or_else(|| {
                    source_uniform_requirement_summary
                        .as_ref()
                        .and_then(|summary| {
                            (summary.field_count != summary.resolved_field_count).then(|| {
                                format!(
                                    "unresolved DH scalar source uniforms: {}",
                                    summary.unresolved_field_names.join(", ")
                                )
                            })
                        })
                });
                Err(GalError::unsupported_feature(format!(
                    "Distant Horizons source has no complete paired-stage source contract: {}",
                    source_preprocess_error
                        .as_deref()
                        .or(source_lowering_error.as_deref())
                        .or(uniform_status.as_deref())
                        .or(source_resource_binding_error.as_deref())
                        .unwrap_or("missing paired lowering/resource provenance")
                )))
            }
        }
    }

    /// Returns the separately lowered `dh_water` program only when its own
    /// source pair and semantic resource bindings are complete. This is a
    /// preparation artifact: callers still need a dedicated late source pass
    /// with the owned depth-history resource required by the contract.
    pub(crate) fn prepared_lowered_distant_horizons_translucent_source_program(
        &self,
    ) -> GalResult<Option<LoweredDistantHorizonsSourceProgram>> {
        match &self.distant_horizons_source_candidate {
            DistantHorizonsSourceCandidateState::Unavailable
            | DistantHorizonsSourceCandidateState::Disabled { .. }
            | DistantHorizonsSourceCandidateState::Rejected { .. } => Ok(None),
            DistantHorizonsSourceCandidateState::Discovered {
                translucent_contract: DistantHorizonsTranslucentSourceCandidate::Unavailable,
                ..
            } => Ok(None),
            DistantHorizonsSourceCandidateState::Discovered {
                translucent_contract: DistantHorizonsTranslucentSourceCandidate::Rejected(reason),
                ..
            } => Err(GalError::unsupported_feature(format!(
                "Distant Horizons translucent source contract was rejected: {reason}"
            ))),
            DistantHorizonsSourceCandidateState::Discovered {
                translucent_contract: DistantHorizonsTranslucentSourceCandidate::Discovered(
                    contract,
                ),
                translucent_source_summary: Some(_),
                translucent_source_preprocess_error: None,
                translucent_source_lowering_error: None,
                translucent_source_lowered_pair: Some(lowered),
                translucent_source_resource_binding_error: None,
                translucent_source_resource_bindings: Some(bindings),
                ..
            } => prepare_lowered_distant_horizons_source_program(contract, lowered, bindings)
                .map(Some),
            DistantHorizonsSourceCandidateState::Discovered {
                translucent_source_preprocess_error,
                translucent_source_lowering_error,
                translucent_source_resource_binding_error,
                ..
            } => Err(GalError::unsupported_feature(format!(
                "Distant Horizons translucent source has no complete paired-stage source contract: {}",
                translucent_source_preprocess_error
                    .as_deref()
                    .or(translucent_source_lowering_error.as_deref())
                    .or(translucent_source_resource_binding_error.as_deref())
                    .unwrap_or("missing retained lowering/resource provenance")
            ))),
        }
    }

    /// Returns the complete owned source-derived fullscreen stages that
    /// consume the Distant Horizons depth stream. This remains preparation
    /// data only: callers still need to stage every named target, create the
    /// explicit fullscreen passes, and confirm one combined submission before
    /// a DH shader-pack route can be selected.
    ///
    /// A partial consumer list is never returned. A source-declared consumer
    /// without a fully lowered semantic program is an admission error, not an
    /// excuse to omit its depth dependency.
    pub(crate) fn prepared_lowered_distant_horizons_depth_consumers(
        &self,
    ) -> GalResult<Vec<&LoweredFullscreenSourceProgram>> {
        match &self.distant_horizons_source_candidate {
            DistantHorizonsSourceCandidateState::Unavailable
            | DistantHorizonsSourceCandidateState::Disabled { .. }
            | DistantHorizonsSourceCandidateState::Rejected { .. } => Ok(Vec::new()),
            DistantHorizonsSourceCandidateState::Discovered {
                contract,
                depth_consumer_preparation,
                ..
            } => {
                if contract.distant_depth_consumers.len() != depth_consumer_preparation.len() {
                    return Err(GalError::backend(
                        "Distant Horizons depth-consumer preparation no longer matches its source contract",
                    ));
                }
                depth_consumer_preparation
                    .iter()
                    .zip(contract.distant_depth_consumers.iter())
                    .map(|(prepared, declared)| {
                        if prepared.stage_path != declared.stage_path {
                            return Err(GalError::backend(format!(
                                "Distant Horizons depth-consumer preparation '{}' does not match declared stage '{}'",
                                prepared.stage_path, declared.stage_path
                            )));
                        }
                        prepared.source_program.as_ref().ok_or_else(|| {
                            let reason = prepared
                                .source_preprocess_error
                                .as_deref()
                                .or(prepared.source_lowering_error.as_deref())
                                .or(prepared.source_program_preparation_error.as_deref())
                                .or(prepared.source_resource_binding_error.as_deref())
                                .unwrap_or("missing retained owned fullscreen program");
                            GalError::unsupported_feature(format!(
                                "Distant Horizons depth consumer '{}' is not fully prepared: {reason}",
                                prepared.stage_path
                            ))
                        })
                    })
                    .collect()
            }
        }
    }

    /// Returns the complete source-derived fullscreen chain compiled in
    /// Distant Horizons mode. This includes stages that do not sample the DH
    /// depth directly: later composite/final stages must preserve the same
    /// source configuration as the first deferred consumer.
    pub(crate) fn prepared_lowered_distant_horizons_post_terrain_fullscreen_programs(
        &self,
    ) -> GalResult<Vec<&LoweredFullscreenSourceProgram>> {
        match &self.distant_horizons_source_candidate {
            DistantHorizonsSourceCandidateState::Unavailable
            | DistantHorizonsSourceCandidateState::Disabled { .. }
            | DistantHorizonsSourceCandidateState::Rejected { .. } => Ok(Vec::new()),
            DistantHorizonsSourceCandidateState::Discovered {
                post_terrain_preparation,
                post_terrain_preparation_error,
                ..
            } => {
                if let Some(error) = post_terrain_preparation_error {
                    return Err(GalError::unsupported_feature(format!(
                        "complete Distant Horizons shader-pack fullscreen chain could not be derived: {error}"
                    )));
                }
                post_terrain_preparation
                    .iter()
                    .map(|prepared| {
                        prepared.source_program.as_ref().ok_or_else(|| {
                            let reason = prepared
                                .source_preprocess_error
                                .as_deref()
                                .or(prepared.source_lowering_error.as_deref())
                                .or(prepared.source_program_preparation_error.as_deref())
                                .or(prepared.source_resource_binding_error.as_deref())
                                .unwrap_or("missing retained owned fullscreen program");
                            GalError::unsupported_feature(format!(
                                "Distant Horizons fullscreen source stage '{}' is not fully prepared: {reason}",
                                prepared.stage_path
                            ))
                        })
                    })
                    .collect()
            }
        }
    }

    /// Returns the optional source-derived sky initializer retained for the
    /// selected normal-world contract. This stage runs after named-color
    /// bootstrap and before terrain/DH writers, so it never borrows a Java
    /// sky buffer or pretends to be a deferred/composite consumer.
    pub(crate) fn prepared_lowered_pre_terrain_sky_program(
        &self,
    ) -> GalResult<Option<&LoweredFullscreenSourceProgram>> {
        match &self.source_candidate {
            TerrainSourceCandidateState::Unavailable
            | TerrainSourceCandidateState::Disabled { .. }
            | TerrainSourceCandidateState::Rejected { .. } => Ok(None),
            TerrainSourceCandidateState::Discovered {
                pre_terrain_sky_preparation,
                ..
            } => match pre_terrain_sky_preparation {
                None => Ok(None),
                Some(prepared) => prepared.source_program.as_ref().map(Some).ok_or_else(|| {
                    let reason = prepared
                        .source_preprocess_error
                        .as_deref()
                        .or(prepared.source_lowering_error.as_deref())
                        .or(prepared.source_program_preparation_error.as_deref())
                        .or(prepared.source_resource_binding_error.as_deref())
                        .unwrap_or("missing retained owned sky initializer");
                    GalError::unsupported_feature(format!(
                        "source sky initializer '{}' is not fully prepared: {reason}",
                        prepared.stage_path
                    ))
                }),
            },
        }
    }

    /// Returns the optional source-defined celestial writer retained for the
    /// normal-world contract. Discovery/lowering is kept private until the
    /// frontend supplies a complete owned texture/resource execution plan.
    pub(crate) fn prepared_lowered_pre_terrain_celestial_program(
        &self,
    ) -> GalResult<Option<&LoweredFullscreenSourceProgram>> {
        match &self.source_candidate {
            TerrainSourceCandidateState::Unavailable
            | TerrainSourceCandidateState::Disabled { .. }
            | TerrainSourceCandidateState::Rejected { .. } => Ok(None),
            TerrainSourceCandidateState::Discovered {
                pre_terrain_celestial_preparation,
                ..
            } => match pre_terrain_celestial_preparation {
                None => Ok(None),
                Some(prepared) => prepared.source_program.as_ref().map(Some).ok_or_else(|| {
                    let reason = prepared
                        .source_preprocess_error
                        .as_deref()
                        .or(prepared.source_lowering_error.as_deref())
                        .or(prepared.source_program_preparation_error.as_deref())
                        .or(prepared.source_resource_binding_error.as_deref())
                        .unwrap_or("missing retained owned celestial source writer");
                    GalError::unsupported_feature(format!(
                        "source celestial stage '{}' is not fully prepared: {reason}",
                        prepared.stage_path
                    ))
                }),
            },
        }
    }

    /// Stages the optional source sky initializer against the same named
    /// color generation that terrain and Distant Horizons will later write.
    /// A missing sky stage is a supported pack choice; a declared but
    /// unprepared stage is rejected by `prepared_lowered_pre_terrain_sky_program`.
    pub(crate) fn stage_pre_terrain_sky_execution_plan(
        &self,
        gal: &mut VulkanicGal,
        targets: &ShaderPackColorTargets,
        external_inputs: &[TerrainSourceOwnedResourceSet],
        extent: Extent3d,
    ) -> GalResult<Option<FullscreenSourceExecutionPlan>> {
        let Some(program) = self.prepared_lowered_pre_terrain_sky_program()? else {
            return Ok(None);
        };
        let manifest = self.source_color_target_manifest()?.ok_or_else(|| {
            GalError::unsupported_feature(
                "source sky initializer requires a selected semantic color-target manifest",
            )
        })?;
        if manifest.generation() != targets.identity.shader_pack_generation {
            return Err(GalError::invalid_argument(
                "source sky initializer and named color targets have different shader-pack generations",
            ));
        }
        FullscreenSourceExecutionPlan::stage(
            gal,
            program,
            manifest,
            targets,
            external_inputs.iter().cloned(),
            extent,
        )
        .map(Some)
    }

    /// Stages the optional source-defined textured celestial writer against
    /// the same owned named-color generation as the sky initializer. The
    /// caller supplies one fully semantic resource snapshot per celestial
    /// draw, so this never inherits Java/Iris texture bindings.
    pub(crate) fn stage_pre_terrain_celestial_execution_plan(
        &self,
        gal: &mut VulkanicGal,
        targets: &ShaderPackColorTargets,
        external_inputs: &[TerrainSourceOwnedResourceSet],
        extent: Extent3d,
    ) -> GalResult<Option<FullscreenSourceExecutionPlan>> {
        let Some(program) = self.prepared_lowered_pre_terrain_celestial_program()? else {
            return Ok(None);
        };
        let manifest = self.source_color_target_manifest()?.ok_or_else(|| {
            GalError::unsupported_feature(
                "source celestial writer requires a selected semantic color-target manifest",
            )
        })?;
        if manifest.generation() != targets.identity.shader_pack_generation {
            return Err(GalError::invalid_argument(
                "source celestial writer and named color targets have different shader-pack generations",
            ));
        }
        FullscreenSourceExecutionPlan::stage(
            gal,
            program,
            manifest,
            targets,
            external_inputs.iter().cloned(),
            extent,
        )
        .map(Some)
    }

    /// Returns the complete scoped fullscreen source chain retained during
    /// discovery. Unlike the depth-consumer accessor, this includes stages
    /// that do not sample Distant Horizons depth. It is deliberately strict:
    /// the first preprocessing, lowering, or semantic-resource failure keeps
    /// the entire eventual source route unavailable instead of allowing a
    /// partial composite chain to look complete.
    pub(crate) fn prepared_lowered_post_terrain_fullscreen_programs(
        &self,
    ) -> GalResult<Vec<&LoweredFullscreenSourceProgram>> {
        match &self.source_candidate {
            TerrainSourceCandidateState::Unavailable
            | TerrainSourceCandidateState::Disabled { .. }
            | TerrainSourceCandidateState::Rejected { .. } => Ok(Vec::new()),
            TerrainSourceCandidateState::Discovered {
                post_terrain_preparation,
                post_terrain_preparation_error,
                ..
            } => {
                if let Some(error) = post_terrain_preparation_error {
                    return Err(GalError::unsupported_feature(format!(
                        "complete shader-pack fullscreen chain could not be derived: {error}"
                    )));
                }
                post_terrain_preparation
                    .iter()
                    .map(|prepared| {
                        prepared.source_program.as_ref().ok_or_else(|| {
                            let reason = prepared
                                .source_preprocess_error
                                .as_deref()
                                .or(prepared.source_lowering_error.as_deref())
                                .or(prepared.source_program_preparation_error.as_deref())
                                .or(prepared.source_resource_binding_error.as_deref())
                                .unwrap_or("missing retained owned fullscreen program");
                            GalError::unsupported_feature(format!(
                                "fullscreen source stage '{}' is not fully prepared: {reason}",
                                prepared.stage_path
                            ))
                        })
                    })
                    .collect()
            }
        }
    }

    /// Stages complete GAL execution owners for the retained post-DH source
    /// consumers. This remains private preparation: the caller owns exact
    /// frame uniform payloads, resource-state sequencing, combined
    /// submission confirmation, and route selection. Staging rejects before
    /// any draw when even one semantic resource is absent.
    pub(crate) fn stage_distant_horizons_depth_consumer_execution_plans(
        &self,
        gal: &mut VulkanicGal,
        targets: &ShaderPackColorTargets,
        external_inputs: &[TerrainSourceOwnedResourceSet],
        extent: crate::render::vulkanic::resources::Extent3d,
    ) -> GalResult<Vec<FullscreenSourceExecutionPlan>> {
        let manifest = match &self.distant_horizons_source_candidate {
            DistantHorizonsSourceCandidateState::Discovered {
                source_color_targets: Some(manifest),
                ..
            } => manifest,
            DistantHorizonsSourceCandidateState::Discovered { .. } => {
                return Err(GalError::unsupported_feature(
                    "Distant Horizons source consumers have no complete semantic color-target manifest",
                ));
            }
            DistantHorizonsSourceCandidateState::Unavailable
            | DistantHorizonsSourceCandidateState::Disabled { .. }
            | DistantHorizonsSourceCandidateState::Rejected { .. } => return Ok(Vec::new()),
        };
        let programs = self.prepared_lowered_distant_horizons_depth_consumers()?;
        let mut plans = Vec::with_capacity(programs.len());
        for program in programs {
            match FullscreenSourceExecutionPlan::stage(
                gal,
                program,
                manifest,
                targets,
                external_inputs.iter().cloned(),
                extent,
            ) {
                Ok(plan) => plans.push(plan),
                Err(error) => {
                    for plan in plans.into_iter().rev() {
                        plan.destroy(gal);
                    }
                    return Err(error);
                }
            }
        }
        Ok(plans)
    }

    /// Stages the complete Distant Horizons-mode fullscreen chain against the
    /// shared named-color target generation. The target model stays common;
    /// only source-derived control flow differs from normal-world execution.
    pub(crate) fn stage_distant_horizons_complete_post_terrain_execution_plans(
        &self,
        gal: &mut VulkanicGal,
        targets: &ShaderPackColorTargets,
        external_inputs: &[TerrainSourceOwnedResourceSet],
        extent: crate::render::vulkanic::resources::Extent3d,
    ) -> GalResult<Vec<FullscreenSourceExecutionPlan>> {
        let manifest = self.source_color_target_manifest()?.ok_or_else(|| {
            GalError::unsupported_feature(
                "complete Distant Horizons fullscreen source execution requires a selected semantic color-target manifest",
            )
        })?;
        if manifest.generation() != targets.identity.shader_pack_generation {
            return Err(GalError::invalid_argument(
                "Distant Horizons fullscreen source programs and named color targets have different shader-pack generations",
            ));
        }
        let programs = self.prepared_lowered_distant_horizons_post_terrain_fullscreen_programs()?;
        if programs.is_empty() {
            return Err(GalError::unsupported_feature(
                "complete Distant Horizons fullscreen source execution requires at least one retained stage",
            ));
        }
        let mut plans = Vec::with_capacity(programs.len());
        for program in programs {
            match FullscreenSourceExecutionPlan::stage(
                gal,
                program,
                manifest,
                targets,
                external_inputs.iter().cloned(),
                extent,
            ) {
                Ok(plan) => plans.push(plan),
                Err(error) => {
                    for plan in plans.into_iter().rev() {
                        plan.destroy(gal);
                    }
                    return Err(error);
                }
            }
        }
        Ok(plans)
    }

    /// Stages every fullscreen pass in the complete scoped world chain using
    /// one already-complete named-color generation. This deliberately has no
    /// special knowledge of normal terrain or Distant Horizons: the caller
    /// supplies the union of generation-coherent semantic inputs accumulated
    /// by those frontends, and every stage receives the same explicit source
    /// target contract. The world frontend schedules all writers, this chain,
    /// and final output in one Rust-owned submission.
    pub(crate) fn stage_complete_post_terrain_execution_plans(
        &self,
        gal: &mut VulkanicGal,
        targets: &ShaderPackColorTargets,
        external_inputs: &[TerrainSourceOwnedResourceSet],
        extent: crate::render::vulkanic::resources::Extent3d,
    ) -> GalResult<Vec<FullscreenSourceExecutionPlan>> {
        let manifest = self.source_color_target_manifest()?.ok_or_else(|| {
            GalError::unsupported_feature(
                "complete fullscreen source execution requires a selected semantic color-target manifest",
            )
        })?;
        if manifest.generation() != targets.identity.shader_pack_generation {
            return Err(GalError::invalid_argument(
                "complete fullscreen source programs and named color targets have different shader-pack generations",
            ));
        }
        let programs = self.prepared_lowered_post_terrain_fullscreen_programs()?;
        if programs.is_empty() {
            return Err(GalError::unsupported_feature(
                "complete fullscreen source execution requires at least one retained stage",
            ));
        }
        let mut plans = Vec::with_capacity(programs.len());
        for program in programs {
            match FullscreenSourceExecutionPlan::stage(
                gal,
                program,
                manifest,
                targets,
                external_inputs.iter().cloned(),
                extent,
            ) {
                Ok(plan) => plans.push(plan),
                Err(error) => {
                    for plan in plans.into_iter().rev() {
                        plan.destroy(gal);
                    }
                    return Err(error);
                }
            }
        }
        Ok(plans)
    }

    /// Re-discovers only semantic source metadata for the supplied world
    /// scope. It is not a route-selection method and cannot bind or execute
    /// an Iris/OpenGL program.
    pub(crate) fn observe_source_candidate_for_scope(
        &mut self,
        source: &ShaderPackSource,
        scope: TerrainProgramScope,
    ) {
        if self.source_candidate_matches(source, scope) {
            return;
        }
        if source.is_empty() {
            self.source_candidate = TerrainSourceCandidateState::Disabled {
                generation: source.generation(),
                pack_name: source.name().to_string(),
            };
            self.source_candidate_scope = Some(scope);
            return;
        }
        let candidate = match ShaderPackRuntimePlan::discover_terrain_contract_from_source_for_scope(
            source.generation(),
            source,
            scope,
        ) {
            Ok(contract) => {
                let (
                    textured_material_contract,
                    textured_material_contract_error,
                    textured_material_lowered_pair,
                    textured_material_source_resource_binding_count,
                    textured_material_source_resource_binding_error,
                    textured_material_source_resource_bindings,
                ) = match derive_textured_material_contract(source, scope) {
                    Ok(material_contract) => {
                        match lower_textured_material_source_pair(source, &material_contract) {
                            Ok(lowered_pair) => {
                                let resource_bindings: GalResult<_> = (|| -> GalResult<_> {
                                    let bindings =
                                        TerrainSourceResourceBindings::from_source(source)?;
                                    lowered_pair
                                        .opaque_resource_contract()
                                        .bind_semantic_roles(&bindings)
                                })(
                                );
                                match resource_bindings {
                                    Ok(bindings) => {
                                        let count = bindings.bindings().len() as u32;
                                        (
                                            Some(material_contract),
                                            None,
                                            Some(lowered_pair),
                                            Some(count),
                                            None,
                                            Some(bindings),
                                        )
                                    }
                                    Err(error) => (
                                        Some(material_contract),
                                        None,
                                        Some(lowered_pair),
                                        None,
                                        Some(error.to_string()),
                                        None,
                                    ),
                                }
                            }
                            Err(error) => (
                                Some(material_contract),
                                Some(error.to_string()),
                                None,
                                None,
                                None,
                                None,
                            ),
                        }
                    }
                    Err(error) => (None, Some(error.to_string()), None, None, None, None),
                };
                let (
                    entity_contract,
                    entity_contract_error,
                    entity_lowered_pair,
                    entity_source_resource_binding_count,
                    entity_source_resource_binding_error,
                    entity_source_resource_bindings,
                ) = match derive_entity_contract(source, scope) {
                    Ok(entity_contract) => match lower_entity_source_pair(source, &entity_contract)
                    {
                        Ok(lowered_pair) => {
                            let resource_bindings: GalResult<_> = (|| -> GalResult<_> {
                                let bindings = TerrainSourceResourceBindings::from_source(source)?;
                                bind_entity_source_resources(&lowered_pair, &bindings)
                            })();
                            match resource_bindings {
                                Ok(bindings) => {
                                    let count = bindings.bindings().len() as u32;
                                    (
                                        Some(entity_contract),
                                        None,
                                        Some(lowered_pair),
                                        Some(count),
                                        None,
                                        Some(bindings),
                                    )
                                }
                                Err(error) => (
                                    Some(entity_contract),
                                    None,
                                    Some(lowered_pair),
                                    None,
                                    Some(error.to_string()),
                                    None,
                                ),
                            }
                        }
                        Err(error) => (
                            Some(entity_contract),
                            Some(error.to_string()),
                            None,
                            None,
                            None,
                            None,
                        ),
                    },
                    Err(error) => (None, Some(error.to_string()), None, None, None, None),
                };
                let (
                    hand_contract,
                    hand_contract_error,
                    hand_lowered_pair,
                    hand_source_resource_binding_count,
                    hand_source_resource_binding_error,
                    hand_source_resource_bindings,
                ) = match derive_hand_contract(source, scope) {
                    Ok(hand_contract) => match lower_hand_source_pair(source, &hand_contract) {
                        Ok(lowered_pair) => {
                            let resource_bindings: GalResult<_> = (|| -> GalResult<_> {
                                let bindings = TerrainSourceResourceBindings::from_source(source)?;
                                bind_hand_source_resources(&lowered_pair, &bindings)
                            })();
                            match resource_bindings {
                                Ok(bindings) => {
                                    let count = bindings.bindings().len() as u32;
                                    (
                                        Some(hand_contract),
                                        None,
                                        Some(lowered_pair),
                                        Some(count),
                                        None,
                                        Some(bindings),
                                    )
                                }
                                Err(error) => (
                                    Some(hand_contract),
                                    None,
                                    Some(lowered_pair),
                                    None,
                                    Some(error.to_string()),
                                    None,
                                ),
                            }
                        }
                        Err(error) => (
                            Some(hand_contract),
                            Some(error.to_string()),
                            None,
                            None,
                            None,
                            None,
                        ),
                    },
                    Err(error) => (None, Some(error.to_string()), None, None, None, None),
                };
                let (translucent_contract, translucent_contract_error) =
                    match derive_complementary_translucent_terrain_contract_for_scope(source, scope)
                    {
                        Ok(contract) => (Some(contract), None),
                        Err(error) => (None, Some(error.to_string())),
                    };
                let (
                    weather_contract,
                    weather_contract_error,
                    weather_lowered_pair,
                    weather_source_resource_binding_count,
                    weather_source_resource_binding_error,
                    weather_source_resource_bindings,
                ) = match derive_weather_pass_contract(source, scope) {
                    Ok(weather_contract) => {
                        match lower_weather_source_pair(source, &weather_contract) {
                            Ok(lowered_pair) => {
                                let resource_bindings: GalResult<_> = (|| -> GalResult<_> {
                                    let bindings =
                                        TerrainSourceResourceBindings::from_source(source)?;
                                    lowered_pair
                                        .opaque_resource_contract()
                                        .bind_semantic_roles(&bindings)
                                })(
                                );
                                match resource_bindings {
                                    Ok(bindings) => {
                                        let count = bindings.bindings().len() as u32;
                                        (
                                            Some(weather_contract),
                                            None,
                                            Some(lowered_pair),
                                            Some(count),
                                            None,
                                            Some(bindings),
                                        )
                                    }
                                    Err(error) => (
                                        Some(weather_contract),
                                        None,
                                        Some(lowered_pair),
                                        None,
                                        Some(error.to_string()),
                                        None,
                                    ),
                                }
                            }
                            Err(error) => (
                                Some(weather_contract),
                                Some(error.to_string()),
                                None,
                                None,
                                None,
                                None,
                            ),
                        }
                    }
                    Err(error) => (None, Some(error.to_string()), None, None, None, None),
                };
                let (
                    cloud_contract,
                    cloud_contract_error,
                    cloud_lowered_pair,
                    cloud_source_resource_binding_count,
                    cloud_source_resource_binding_error,
                    cloud_source_resource_bindings,
                ) = match derive_cloud_pass_contract(source, scope) {
                    Ok(cloud_contract)
                        if cloud_contract.face_disposition
                            == CloudFaceDisposition::SuppressVanillaFaces =>
                    {
                        (Some(cloud_contract), None, None, None, None, None)
                    }
                    Ok(cloud_contract) => match lower_cloud_source_pair(source, &cloud_contract) {
                        Ok(lowered_pair) => {
                            let resource_bindings: GalResult<_> = (|| -> GalResult<_> {
                                let bindings = TerrainSourceResourceBindings::from_source(source)?;
                                lowered_pair
                                    .opaque_resource_contract()
                                    .bind_semantic_roles(&bindings)
                            })();
                            match resource_bindings {
                                Ok(bindings) => {
                                    let count = bindings.bindings().len() as u32;
                                    (
                                        Some(cloud_contract),
                                        None,
                                        Some(lowered_pair),
                                        Some(count),
                                        None,
                                        Some(bindings),
                                    )
                                }
                                Err(error) => (
                                    Some(cloud_contract),
                                    None,
                                    Some(lowered_pair),
                                    None,
                                    Some(error.to_string()),
                                    None,
                                ),
                            }
                        }
                        Err(error) => (
                            Some(cloud_contract),
                            Some(error.to_string()),
                            None,
                            None,
                            None,
                            None,
                        ),
                    },
                    Err(error) => (None, Some(error.to_string()), None, None, None, None),
                };
                let (
                    translucent_source_summary,
                    translucent_source_preprocess_error,
                    translucent_source_lowering_error,
                    translucent_lowered_pair,
                    translucent_source_resource_binding_count,
                    translucent_source_resource_binding_error,
                    translucent_source_resource_bindings,
                ) = match translucent_contract.as_ref() {
                    Some(contract) => match contract
                        .source_stages()
                        .and_then(|stages| preprocess_terrain_sources(source, &stages))
                    {
                        Ok(artifacts) => match lower_translucent_terrain_source_pair(
                            &artifacts.vertex,
                            &artifacts.fragment,
                        ) {
                            Ok(lowered) => {
                                let resource_bindings: GalResult<_> = (|| -> GalResult<_> {
                                    let bindings =
                                        TerrainSourceResourceBindings::from_source(source)?;
                                    let plan = lowered
                                        .opaque_resource_contract()
                                        .bind_semantic_roles(&bindings)?;
                                    bindings.require_contract_roles(contract)?;
                                    Ok(plan)
                                })(
                                );
                                (
                                    Some(artifacts.summary()),
                                    None,
                                    None,
                                    Some(lowered),
                                    resource_bindings
                                        .as_ref()
                                        .ok()
                                        .map(|plan| plan.bindings().len() as u32),
                                    resource_bindings
                                        .as_ref()
                                        .err()
                                        .map(|error| error.to_string()),
                                    resource_bindings.ok(),
                                )
                            }
                            Err(error) => (
                                Some(artifacts.summary()),
                                None,
                                Some(error.to_string()),
                                None,
                                None,
                                None,
                                None,
                            ),
                        },
                        Err(error) => (None, Some(error.to_string()), None, None, None, None, None),
                    },
                    None => (
                        None,
                        None,
                        translucent_contract_error.clone(),
                        None,
                        None,
                        None,
                        None,
                    ),
                };
                let (
                    source_summary,
                    source_preprocess_error,
                    source_vertex_interface,
                    source_lowering_summary,
                    source_lowering_error,
                    source_lowered_pair,
                    source_uniform_requirement_summary,
                    source_uniform_requirement_error,
                    source_resource_binding_count,
                    source_resource_binding_error,
                    source_resource_bindings,
                ) = match contract
                    .source_stages()
                    .and_then(|stages| preprocess_terrain_sources(source, &stages))
                {
                    Ok(artifacts) => {
                        match lower_terrain_source_pair(&artifacts.vertex, &artifacts.fragment) {
                            Ok(lowered) => {
                                let uniform_requirements =
                                    TerrainSourceUniformRequirements::from_contract(
                                        lowered.uniform_contract(),
                                    );
                                let resource_bindings: GalResult<_> = (|| -> GalResult<_> {
                                    let bindings =
                                        TerrainSourceResourceBindings::from_source(source)?;
                                    let plan = lowered
                                        .opaque_resource_contract()
                                        .bind_semantic_roles(&bindings)?;
                                    bindings.require_contract_roles(&contract)?;
                                    Ok(plan)
                                })(
                                );
                                (
                                    Some(artifacts.summary()),
                                    None,
                                    Some(analyze_terrain_vertex_interface(&artifacts.vertex)),
                                    Some(lowered.summary()),
                                    None,
                                    Some(lowered),
                                    uniform_requirements
                                        .as_ref()
                                        .ok()
                                        .map(TerrainSourceUniformRequirements::summary),
                                    uniform_requirements
                                        .as_ref()
                                        .err()
                                        .map(|error| error.to_string()),
                                    resource_bindings
                                        .as_ref()
                                        .ok()
                                        .map(|plan| plan.bindings().len() as u32),
                                    resource_bindings
                                        .as_ref()
                                        .err()
                                        .map(|error| error.to_string()),
                                    resource_bindings.ok(),
                                )
                            }
                            Err(error) => (
                                Some(artifacts.summary()),
                                None,
                                Some(analyze_terrain_vertex_interface(&artifacts.vertex)),
                                None,
                                Some(error.to_string()),
                                None,
                                None,
                                None,
                                None,
                                None,
                                None,
                            ),
                        }
                    }
                    Err(error) => (
                        None,
                        Some(error.to_string()),
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                    ),
                };
                let requires_colored_voxel_light = contract
                    .required_resources
                    .contains(&TerrainPassRequiredResource::ColoredVoxelLightVolume);
                let (
                    source_shadow_summary,
                    source_shadow_preprocess_error,
                    source_shadow_output_count,
                    source_shadow_lowering_error,
                    source_lowered_shadow_pair,
                    source_shadow_resource_binding_count,
                    source_shadow_resource_binding_error,
                    source_shadow_resource_bindings,
                ) = match shadow_source_stages_for_scope(source, scope)
                    .and_then(|stages| preprocess_terrain_sources(source, &stages))
                {
                    Ok(artifacts) => {
                        let storage_roles = TerrainSourceResourceBindings::from_source(source);
                        match storage_roles.and_then(|storage_roles| {
                            lower_shadow_source_pair_with_owned_storage(
                                &artifacts.vertex,
                                &artifacts.fragment,
                                &storage_roles,
                            )
                        }) {
                            Ok(lowered) => {
                                let resource_bindings: GalResult<_> = (|| -> GalResult<_> {
                                    let declarations =
                                        TerrainSourceResourceBindings::from_source(source)?;
                                    lowered
                                        .opaque_resource_contract()
                                        .bind_semantic_roles(&declarations)
                                })(
                                );
                                (
                                    Some(artifacts.summary()),
                                    None,
                                    Some(lowered.fragment().outputs().len() as u32),
                                    None,
                                    Some(lowered),
                                    resource_bindings
                                        .as_ref()
                                        .ok()
                                        .map(|plan| plan.bindings().len() as u32),
                                    resource_bindings
                                        .as_ref()
                                        .err()
                                        .map(|error| error.to_string()),
                                    resource_bindings.ok(),
                                )
                            }
                            Err(error) => (
                                Some(artifacts.summary()),
                                None,
                                None,
                                Some(error.to_string()),
                                None,
                                None,
                                None,
                                None,
                            ),
                        }
                    }
                    Err(error) => (
                        None,
                        Some(error.to_string()),
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                    ),
                };
                let source_asset_bindings = TerrainShaderPackAssetBindings::from_source(source);
                let source_asset_binding_count = source_asset_bindings
                    .as_ref()
                    .ok()
                    .map(|bindings| bindings.samplers().count() as u32);
                let source_asset_binding_error = source_asset_bindings
                    .as_ref()
                    .err()
                    .map(ToString::to_string);
                let source_color_targets: GalResult<_> = (|| -> GalResult<_> {
                    let bindings = TerrainSourceResourceBindings::from_source(source)?;
                    ShaderPackColorTargetManifest::from_source(source, &bindings)
                })();
                let source_color_target_count = source_color_targets
                    .as_ref()
                    .ok()
                    .map(|targets| targets.targets().count() as u32);
                let source_color_target_error =
                    source_color_targets.as_ref().err().map(ToString::to_string);
                let source_color_target_gal_schema_error = source_color_targets
                    .as_ref()
                    .ok()
                    .and_then(|targets| targets.require_gal_schema_formats().err())
                    .map(|error| error.to_string());
                let (post_terrain_preparation, post_terrain_preparation_error) =
                    match derive_fullscreen_source_chain(source, scope) {
                        Ok(stages) => (
                            stages
                                .iter()
                                .map(|stage| {
                                    prepare_fullscreen_source_stage(
                                        source,
                                        stage,
                                        FullscreenSourceMode::NormalWorld,
                                    )
                                })
                                .collect(),
                            None,
                        ),
                        Err(error) => (Vec::new(), Some(error.to_string())),
                    };
                let pre_terrain_sky_preparation = match derive_sky_source_stage(source, scope) {
                    Ok(Some(stage)) => Some(prepare_fullscreen_source_stage(
                        source,
                        &stage,
                        FullscreenSourceMode::NormalWorld,
                    )),
                    Ok(None) => None,
                    Err(error) => Some(FullscreenSourceStagePreparation {
                        stage_path: format!("{scope:?}/gbuffers_skybasic.fsh"),
                        kind: FullscreenSourceStageKind::Sky,
                        source_summary: None,
                        source_preprocess_error: Some(error.to_string()),
                        source_lowering_error: None,
                        source_output_roles: Vec::new(),
                        source_program_preparation_error: None,
                        source_program_identity: None,
                        source_program: None,
                        source_feedback_roles: Vec::new(),
                        source_mipmap_roles: Vec::new(),
                        source_resource_binding_count: None,
                        source_resource_binding_error: None,
                    }),
                };
                let pre_terrain_celestial_preparation =
                    match derive_sky_textured_source_stage(source, scope) {
                        Ok(Some(stage)) => Some(prepare_fullscreen_source_stage(
                            source,
                            &stage,
                            FullscreenSourceMode::NormalWorld,
                        )),
                        Ok(None) => None,
                        Err(error) => Some(FullscreenSourceStagePreparation {
                            stage_path: format!("{scope:?}/gbuffers_skytextured.fsh"),
                            kind: FullscreenSourceStageKind::SkyTextured,
                            source_summary: None,
                            source_preprocess_error: Some(error.to_string()),
                            source_lowering_error: None,
                            source_output_roles: Vec::new(),
                            source_program_preparation_error: None,
                            source_program_identity: None,
                            source_program: None,
                            source_feedback_roles: Vec::new(),
                            source_mipmap_roles: Vec::new(),
                            source_resource_binding_count: None,
                            source_resource_binding_error: None,
                        }),
                    };
                let (voxel_materials, voxel_emission) = if requires_colored_voxel_light {
                    match (
                        VoxelMaterialMap::derive(source, &contract),
                        VoxelEmissionTable::derive(source, &contract),
                    ) {
                        (Ok(materials), Ok(emission)) => (Some(materials), Some(emission)),
                        (Err(error), _) | (_, Err(error)) => {
                            self.source_candidate = TerrainSourceCandidateState::Rejected {
                                generation: source.generation(),
                                pack_name: source.name().to_string(),
                                reason: format!(
                                    "selected source colored voxel-light semantics are unsupported: {error}"
                                ),
                            };
                            return;
                        }
                    }
                } else {
                    (None, None)
                };
                TerrainSourceCandidateState::Discovered {
                    generation: source.generation(),
                    pack_name: source.name().to_string(),
                    requires_colored_voxel_light,
                    contract,
                    textured_material_contract,
                    textured_material_contract_error,
                    textured_material_lowered_pair,
                    textured_material_source_resource_binding_count,
                    textured_material_source_resource_binding_error,
                    textured_material_source_resource_bindings,
                    entity_contract,
                    entity_contract_error,
                    entity_lowered_pair,
                    entity_source_resource_binding_count,
                    entity_source_resource_binding_error,
                    entity_source_resource_bindings,
                    hand_contract,
                    hand_contract_error,
                    hand_lowered_pair,
                    hand_source_resource_binding_count,
                    hand_source_resource_binding_error,
                    hand_source_resource_bindings,
                    weather_contract,
                    weather_contract_error,
                    weather_lowered_pair,
                    weather_source_resource_binding_count,
                    weather_source_resource_binding_error,
                    weather_source_resource_bindings,
                    cloud_contract,
                    cloud_contract_error,
                    cloud_lowered_pair,
                    cloud_source_resource_binding_count,
                    cloud_source_resource_binding_error,
                    cloud_source_resource_bindings,
                    translucent_contract,
                    translucent_contract_error,
                    translucent_source_summary,
                    translucent_source_preprocess_error,
                    translucent_source_lowering_error,
                    translucent_lowered_pair,
                    translucent_source_resource_binding_count,
                    translucent_source_resource_binding_error,
                    translucent_source_resource_bindings,
                    source_summary,
                    source_preprocess_error,
                    source_shadow_summary,
                    source_shadow_preprocess_error,
                    source_shadow_output_count,
                    source_shadow_lowering_error,
                    source_lowered_shadow_pair,
                    source_shadow_resource_binding_count,
                    source_shadow_resource_binding_error,
                    source_shadow_resource_bindings,
                    source_vertex_interface,
                    source_lowering_summary,
                    source_lowering_error,
                    source_lowered_pair,
                    source_uniform_requirement_summary,
                    source_uniform_requirement_error,
                    source_resource_binding_count,
                    source_resource_binding_error,
                    source_resource_bindings,
                    source_asset_binding_count,
                    source_asset_binding_error,
                    source_asset_bindings: source_asset_bindings.ok(),
                    source_color_target_count,
                    source_color_target_error,
                    source_color_target_gal_schema_error,
                    source_color_targets: source_color_targets.ok(),
                    pre_terrain_sky_preparation,
                    pre_terrain_celestial_preparation,
                    post_terrain_preparation,
                    post_terrain_preparation_error,
                    voxel_materials,
                    voxel_emission,
                }
            }
            Err(error) => TerrainSourceCandidateState::Rejected {
                generation: source.generation(),
                pack_name: source.name().to_string(),
                reason: error.to_string(),
            },
        };
        self.source_candidate = candidate;
        self.source_candidate_scope = Some(scope);
    }

    pub(crate) fn source_candidate(&self) -> &TerrainSourceCandidateState {
        &self.source_candidate
    }

    /// Returns only the source-derived mapping from canonical Minecraft raw
    /// block-state IDs to pack material IDs. This is immutable semantic data,
    /// not an Iris material map, shader binding, or backend object.
    pub(crate) fn candidate_runtime_block_state_material_ids(&self) -> Option<&BTreeMap<i32, i32>> {
        match &self.source_candidate {
            TerrainSourceCandidateState::Discovered { contract, .. } => {
                contract.runtime_block_state_material_ids.as_ref()
            }
            TerrainSourceCandidateState::Unavailable
            | TerrainSourceCandidateState::Disabled { .. }
            | TerrainSourceCandidateState::Rejected { .. } => None,
        }
    }

    /// Resolves a copied producer semantic identity through the active source
    /// contract. This keeps pack rule matching in the shader-pack runtime
    /// rather than making a world producer infer material policy from a
    /// texture, atlas region, or reduced material category.
    pub(crate) fn candidate_material_id_for_block_state_identity(
        &self,
        identity: &str,
    ) -> Option<GalResult<i32>> {
        match &self.source_candidate {
            TerrainSourceCandidateState::Discovered { contract, .. } => {
                Some(contract.material_id_for_block_state_identity(identity))
            }
            TerrainSourceCandidateState::Unavailable
            | TerrainSourceCandidateState::Disabled { .. }
            | TerrainSourceCandidateState::Rejected { .. } => None,
        }
    }

    pub(crate) fn has_candidate_material_contract(&self) -> bool {
        matches!(
            self.source_candidate,
            TerrainSourceCandidateState::Discovered { .. }
        )
    }

    /// Bounded provenance for the generic textured-material source writer.
    /// The lowered pair is retained only after its scoped source has passed
    /// backend-neutral lowering; the world frontend later binds its named
    /// targets and appends the writer to the combined source transaction.
    pub(crate) fn candidate_textured_material_source_diagnostic(
        &self,
    ) -> (&'static str, Option<&str>) {
        match &self.source_candidate {
            TerrainSourceCandidateState::Discovered {
                textured_material_contract,
                textured_material_contract_error,
                textured_material_lowered_pair,
                textured_material_source_resource_binding_error,
                textured_material_source_resource_bindings,
                ..
            } => {
                if textured_material_lowered_pair.is_some()
                    && textured_material_source_resource_bindings.is_some()
                {
                    ("prepared", None)
                } else if textured_material_lowered_pair.is_some() {
                    (
                        "resource-rejected",
                        textured_material_source_resource_binding_error.as_deref(),
                    )
                } else if textured_material_contract.is_some() {
                    (
                        "lowering-rejected",
                        textured_material_contract_error.as_deref(),
                    )
                } else {
                    ("unavailable", textured_material_contract_error.as_deref())
                }
            }
            TerrainSourceCandidateState::Unavailable => ("unavailable", None),
            TerrainSourceCandidateState::Disabled { .. } => ("disabled", None),
            TerrainSourceCandidateState::Rejected { reason, .. } => ("rejected", Some(reason)),
        }
    }

    /// Bounded provenance for the Rust-owned entity writer. This reports only
    /// source-program/resource readiness; whole-frame route selection remains
    /// the frontend's exact-frame decision.
    pub(crate) fn candidate_entity_source_diagnostic(&self) -> (&'static str, Option<&str>) {
        match &self.source_candidate {
            TerrainSourceCandidateState::Discovered {
                entity_contract: Some(_),
                entity_lowered_pair: Some(_),
                entity_source_resource_bindings: Some(_),
                ..
            } => ("prepared", None),
            TerrainSourceCandidateState::Discovered {
                entity_contract: Some(_),
                entity_lowered_pair: Some(_),
                entity_source_resource_binding_error,
                ..
            } => (
                "resource-rejected",
                entity_source_resource_binding_error.as_deref(),
            ),
            TerrainSourceCandidateState::Discovered {
                entity_contract: Some(_),
                entity_contract_error,
                ..
            } => ("lowering-rejected", entity_contract_error.as_deref()),
            TerrainSourceCandidateState::Discovered {
                entity_contract_error,
                ..
            } => ("unavailable", entity_contract_error.as_deref()),
            TerrainSourceCandidateState::Unavailable => ("unavailable", None),
            TerrainSourceCandidateState::Disabled { .. } => ("disabled", None),
            TerrainSourceCandidateState::Rejected { reason, .. } => ("rejected", Some(reason)),
        }
    }

    /// Bounded provenance for the distinct first-person hand stage. Contract
    /// preparation and route selection remain separate, but the hand writer
    /// is now a Rust-owned executable source family when its copied projection
    /// and depth-clear semantics are present.
    pub(crate) fn candidate_hand_source_diagnostic(&self) -> (&'static str, Option<&str>) {
        match &self.source_candidate {
            TerrainSourceCandidateState::Discovered {
                hand_contract: Some(_),
                hand_lowered_pair: Some(_),
                hand_source_resource_bindings: Some(_),
                ..
            } => ("prepared", None),
            TerrainSourceCandidateState::Discovered {
                hand_contract: Some(_),
                hand_lowered_pair: Some(_),
                hand_source_resource_binding_error,
                ..
            } => (
                "resource-rejected",
                hand_source_resource_binding_error.as_deref(),
            ),
            TerrainSourceCandidateState::Discovered {
                hand_contract: Some(_),
                hand_contract_error,
                ..
            } => ("lowering-rejected", hand_contract_error.as_deref()),
            TerrainSourceCandidateState::Discovered {
                hand_contract_error,
                ..
            } => ("unavailable", hand_contract_error.as_deref()),
            TerrainSourceCandidateState::Unavailable => ("unavailable", None),
            TerrainSourceCandidateState::Disabled { .. } => ("disabled", None),
            TerrainSourceCandidateState::Rejected { reason, .. } => ("rejected", Some(reason)),
        }
    }

    /// Source-only diagnostic for the independent weather stage. It does not
    /// authorize a producer route, target, or presentation path.
    pub(crate) fn candidate_weather_source_diagnostic(&self) -> (&'static str, Option<&str>) {
        match &self.source_candidate {
            TerrainSourceCandidateState::Discovered {
                weather_contract,
                weather_contract_error,
                weather_lowered_pair,
                weather_source_resource_binding_error,
                weather_source_resource_bindings,
                ..
            } => {
                if weather_lowered_pair.is_some() && weather_source_resource_bindings.is_some() {
                    ("prepared", None)
                } else if weather_lowered_pair.is_some() {
                    (
                        "resource-rejected",
                        weather_source_resource_binding_error.as_deref(),
                    )
                } else if weather_contract.is_some() {
                    ("lowering-rejected", weather_contract_error.as_deref())
                } else {
                    ("unavailable", weather_contract_error.as_deref())
                }
            }
            TerrainSourceCandidateState::Unavailable => ("unavailable", None),
            TerrainSourceCandidateState::Disabled { .. } => ("disabled", None),
            TerrainSourceCandidateState::Rejected { reason, .. } => ("rejected", Some(reason)),
        }
    }

    /// Source-only preparation state for vanilla clouds. A `prepared` value
    /// means the selected source pair and semantic resource plan agree; it
    /// never authorizes cloud routing or a writer by itself.
    pub(crate) fn candidate_cloud_source_diagnostic(&self) -> (&'static str, Option<&str>) {
        match &self.source_candidate {
            TerrainSourceCandidateState::Discovered {
                cloud_contract:
                    Some(CloudPassContract {
                        face_disposition: CloudFaceDisposition::SuppressVanillaFaces,
                        ..
                    }),
                ..
            } => ("suppressed", None),
            TerrainSourceCandidateState::Discovered {
                cloud_lowered_pair,
                cloud_source_resource_binding_error,
                cloud_source_resource_bindings,
                ..
            } => {
                if cloud_lowered_pair.is_some() && cloud_source_resource_bindings.is_some() {
                    ("prepared", None)
                } else if cloud_lowered_pair.is_some() {
                    (
                        "resource-rejected",
                        cloud_source_resource_binding_error.as_deref(),
                    )
                } else {
                    match &self.source_candidate {
                        TerrainSourceCandidateState::Discovered {
                            cloud_contract: Some(_),
                            cloud_contract_error,
                            ..
                        } => ("lowering-rejected", cloud_contract_error.as_deref()),
                        TerrainSourceCandidateState::Discovered {
                            cloud_contract_error,
                            ..
                        } => ("unavailable", cloud_contract_error.as_deref()),
                        _ => unreachable!("matched discovered cloud candidate"),
                    }
                }
            }
            TerrainSourceCandidateState::Unavailable => ("unavailable", None),
            TerrainSourceCandidateState::Disabled { .. } => ("disabled", None),
            TerrainSourceCandidateState::Rejected { reason, .. } => ("rejected", Some(reason)),
        }
    }

    /// Prepares the independently discovered vanilla-cloud source from one
    /// exact pack generation. The prepared program is consumed by the
    /// Rust-owned cloud target writer during exact-frame assembly; preparation
    /// itself never mutates route admission or allocates backend resources.
    pub(crate) fn prepared_lowered_cloud_source_program(
        &self,
    ) -> GalResult<Option<LoweredCloudSourceProgram>> {
        match &self.source_candidate {
            TerrainSourceCandidateState::Unavailable
            | TerrainSourceCandidateState::Disabled { .. }
            | TerrainSourceCandidateState::Rejected { .. } => Ok(None),
            TerrainSourceCandidateState::Discovered {
                cloud_contract:
                    Some(CloudPassContract {
                        face_disposition: CloudFaceDisposition::SuppressVanillaFaces,
                        ..
                    }),
                ..
            } => Ok(None),
            TerrainSourceCandidateState::Discovered {
                cloud_contract: Some(contract),
                cloud_contract_error: None,
                cloud_lowered_pair: Some(lowered),
                cloud_source_resource_binding_error: None,
                cloud_source_resource_bindings: Some(bindings),
                ..
            } => prepare_lowered_cloud_source_program(contract, lowered, bindings).map(Some),
            TerrainSourceCandidateState::Discovered {
                cloud_contract_error,
                cloud_source_resource_binding_error,
                ..
            } => Err(GalError::unsupported_feature(format!(
                "selected cloud source has no complete paired-stage resource contract: {}",
                cloud_contract_error
                    .as_deref()
                    .or(cloud_source_resource_binding_error.as_deref())
                    .unwrap_or("missing contract, lowering, or semantic resource bindings")
            ))),
        }
    }

    /// Source-derived policy for vanilla cloud faces. A suppressed result is
    /// an admitted shader-pack behavior, not an unavailable renderer path.
    pub(crate) fn suppresses_vanilla_cloud_faces(&self) -> bool {
        matches!(
            &self.source_candidate,
            TerrainSourceCandidateState::Discovered {
                cloud_contract: Some(CloudPassContract {
                    face_disposition: CloudFaceDisposition::SuppressVanillaFaces,
                    ..
                }),
                ..
            }
        )
    }

    /// Prepares the selected weather source program from one coherent pack
    /// generation. This has no draw/route effect until a dedicated weather
    /// target writer is staged by the combined source-frame transaction.
    pub(crate) fn prepared_lowered_weather_source_program(
        &self,
    ) -> GalResult<Option<LoweredWeatherSourceProgram>> {
        match &self.source_candidate {
            TerrainSourceCandidateState::Unavailable
            | TerrainSourceCandidateState::Disabled { .. }
            | TerrainSourceCandidateState::Rejected { .. } => Ok(None),
            TerrainSourceCandidateState::Discovered {
                weather_contract: Some(contract),
                weather_contract_error: None,
                weather_lowered_pair: Some(lowered),
                weather_source_resource_binding_error: None,
                weather_source_resource_bindings: Some(bindings),
                ..
            } => prepare_lowered_weather_source_program(contract, lowered, bindings).map(Some),
            TerrainSourceCandidateState::Discovered {
                weather_contract_error,
                weather_source_resource_binding_error,
                ..
            } => Err(GalError::unsupported_feature(format!(
                "selected weather source has no complete paired-stage resource contract: {}",
                weather_contract_error
                    .as_deref()
                    .or(weather_source_resource_binding_error.as_deref())
                    .unwrap_or("missing contract, lowering, or semantic resource bindings")
            ))),
        }
    }

    /// Prepares the independently discovered generic textured-material stage
    /// from one exact source generation. This is deliberately not route
    /// admission: a selected frame still needs a Rust-owned material stream,
    /// named target writer, and per-frame resource coherence before any draw
    /// can execute.
    pub(crate) fn prepared_lowered_textured_material_source_program(
        &self,
    ) -> GalResult<Option<LoweredTexturedMaterialSourceProgram>> {
        match &self.source_candidate {
            TerrainSourceCandidateState::Unavailable
            | TerrainSourceCandidateState::Disabled { .. }
            | TerrainSourceCandidateState::Rejected { .. } => Ok(None),
            TerrainSourceCandidateState::Discovered {
                textured_material_contract: Some(contract),
                textured_material_contract_error: None,
                textured_material_lowered_pair: Some(lowered),
                textured_material_source_resource_binding_error: None,
                textured_material_source_resource_bindings: Some(bindings),
                ..
            } => prepare_lowered_textured_material_source_program(contract, lowered, bindings)
                .map(Some),
            TerrainSourceCandidateState::Discovered {
                textured_material_contract_error,
                textured_material_source_resource_binding_error,
                ..
            } => Err(GalError::unsupported_feature(format!(
                "selected textured material source has no complete paired-stage resource contract: {}",
                textured_material_contract_error
                    .as_deref()
                    .or(textured_material_source_resource_binding_error.as_deref())
                    .unwrap_or("missing contract, lowering, or semantic resource bindings")
            ))),
        }
    }

    /// Bounded provenance for source-route admission diagnostics. This is
    /// intentionally semantic-only: it exposes neither shader objects nor
    /// backend state, and lets callers distinguish an absent candidate from a
    /// source contract that simply does not require the colored-light volume.
    pub(crate) fn source_candidate_admission_diagnostic(
        &self,
    ) -> (&'static str, Option<&str>, Option<bool>) {
        match &self.source_candidate {
            TerrainSourceCandidateState::Unavailable => ("unavailable", None, None),
            TerrainSourceCandidateState::Disabled { .. } => ("disabled", None, None),
            TerrainSourceCandidateState::Rejected { reason, .. } => {
                ("rejected", Some(reason.as_str()), None)
            }
            TerrainSourceCandidateState::Discovered {
                requires_colored_voxel_light,
                ..
            } => ("discovered", None, Some(*requires_colored_voxel_light)),
        }
    }

    /// Bounded source-only status for the distinct translucent terrain stage.
    /// This is intentionally separate from the normal terrain candidate: a
    /// discovered water/glass source never authorizes a lowerer, resource set,
    /// or draw until its own contract is complete.
    pub(crate) fn source_candidate_translucent_diagnostic(
        &self,
    ) -> (bool, Option<&str>, Option<usize>) {
        match &self.source_candidate {
            TerrainSourceCandidateState::Discovered {
                translucent_contract,
                translucent_contract_error,
                translucent_source_preprocess_error,
                translucent_source_lowering_error,
                translucent_source_resource_binding_error,
                ..
            } => (
                translucent_contract.is_some(),
                translucent_source_preprocess_error
                    .as_deref()
                    .or(translucent_source_lowering_error.as_deref())
                    .or(translucent_source_resource_binding_error.as_deref())
                    .or(translucent_contract_error.as_deref()),
                translucent_contract
                    .as_ref()
                    .map(|contract| contract.unsupported.len()),
            ),
            TerrainSourceCandidateState::Unavailable
            | TerrainSourceCandidateState::Disabled { .. }
            | TerrainSourceCandidateState::Rejected { .. } => (false, None, None),
        }
    }

    /// Bounded semantic sampler/storage provenance for the distinct
    /// translucent source stage. This is capture-only evidence: the names
    /// are pack source identifiers paired with Rust-owned semantic roles,
    /// never backend bindings or native resources.
    pub(crate) fn source_candidate_translucent_resource_diagnostic(
        &self,
    ) -> (Option<u32>, Option<&str>, Vec<(String, String)>) {
        match &self.source_candidate {
            TerrainSourceCandidateState::Discovered {
                translucent_source_resource_binding_count,
                translucent_source_resource_binding_error,
                translucent_source_resource_bindings,
                ..
            } => {
                let bindings = translucent_source_resource_bindings
                    .as_ref()
                    .map(|plan| {
                        plan.bindings()
                            .iter()
                            .take(32)
                            .map(|binding| {
                                (
                                    binding.resource_name().to_string(),
                                    binding.role().diagnostic_name(),
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                (
                    *translucent_source_resource_binding_count,
                    translucent_source_resource_binding_error.as_deref(),
                    bindings,
                )
            }
            TerrainSourceCandidateState::Unavailable
            | TerrainSourceCandidateState::Disabled { .. }
            | TerrainSourceCandidateState::Rejected { .. } => (None, None, Vec::new()),
        }
    }

    /// Returns the validated, source-derived semantic resource plan for the
    /// current candidate. This is preparation metadata only: it contains no
    /// resource handles and cannot select a route or issue a draw.
    pub(crate) fn source_resource_binding_plan(
        &self,
    ) -> Option<&TerrainSourceOpaqueResourceBindingPlan> {
        match &self.source_candidate {
            TerrainSourceCandidateState::Discovered {
                source_resource_bindings,
                ..
            } => source_resource_bindings.as_ref(),
            TerrainSourceCandidateState::Unavailable
            | TerrainSourceCandidateState::Disabled { .. }
            | TerrainSourceCandidateState::Rejected { .. } => None,
        }
    }

    /// Returns every pass-local source plan that can share copied semantic
    /// resources in one Rust-owned frame. The plans remain deliberately
    /// separate at descriptor-set creation: terrain, shadow, Distant Horizons,
    /// and later fullscreen consumers all keep their own binding layouts.
    /// This union is only for generation/coherence and missing-resource
    /// validation, never a hidden cross-pass resource set.
    fn source_resource_binding_plans(&self) -> Vec<&TerrainSourceOpaqueResourceBindingPlan> {
        self.source_resource_binding_plans_for_frame(true)
    }

    /// A selected source frame only requires Distant Horizons bindings when
    /// that exact frame contains DH ranges. Discovery keeps the DH plans for
    /// validation and later execution, but it must not make ordinary vanilla
    /// terrain wait for a far-depth resource that no producer requested.
    fn source_resource_binding_plans_for_frame(
        &self,
        includes_distant_horizons: bool,
    ) -> Vec<&TerrainSourceOpaqueResourceBindingPlan> {
        let mut plans = match &self.source_candidate {
            TerrainSourceCandidateState::Discovered {
                source_resource_bindings,
                source_shadow_resource_bindings,
                translucent_source_resource_bindings,
                ..
            } => source_resource_bindings
                .iter()
                .chain(source_shadow_resource_bindings.iter())
                .chain(translucent_source_resource_bindings.iter())
                .collect(),
            TerrainSourceCandidateState::Unavailable
            | TerrainSourceCandidateState::Disabled { .. }
            | TerrainSourceCandidateState::Rejected { .. } => Vec::new(),
        };
        if includes_distant_horizons {
            if let DistantHorizonsSourceCandidateState::Discovered {
                source_resource_bindings,
                depth_consumer_preparation,
                ..
            } = &self.distant_horizons_source_candidate
            {
                plans.extend(source_resource_bindings.iter());
                plans.extend(
                    depth_consumer_preparation
                        .iter()
                        .filter_map(|consumer| consumer.source_program.as_ref())
                        .map(|program| &program.opaque_resource_bindings),
                );
            }
        }
        plans
    }

    fn source_required_resource_roles(&self) -> BTreeSet<TerrainSourceResourceRole> {
        self.source_required_resource_roles_for_frame(true)
    }

    fn source_required_resource_roles_for_frame(
        &self,
        includes_distant_horizons: bool,
    ) -> BTreeSet<TerrainSourceResourceRole> {
        let mut roles = self
            .source_resource_binding_plans_for_frame(includes_distant_horizons)
            .into_iter()
            .flat_map(|bindings| bindings.bindings().iter().map(|binding| binding.role()))
            .collect::<BTreeSet<_>>();
        // A source shadow storage declaration is removed from the lowered
        // GLSL only after a named Rust semantic producer takes responsibility
        // for its update. Retain that role in completeness checks so the
        // producer cannot disappear merely because the rewritten program no
        // longer binds the legacy image directly.
        if let TerrainSourceCandidateState::Discovered {
            source_lowered_shadow_pair: Some(shadow),
            ..
        } = &self.source_candidate
        {
            roles.extend(shadow.owned_storage_roles().iter().cloned());
        }
        roles
    }

    /// Returns source-derived PNG sampler paths for diagnostics and a future
    /// Rust-owned resource preparer. This has no native resource identity and
    /// cannot select a shader or rendering route.
    pub(crate) fn source_asset_binding_plan(&self) -> Option<&TerrainShaderPackAssetBindings> {
        match &self.source_candidate {
            TerrainSourceCandidateState::Discovered {
                source_asset_bindings,
                ..
            } => source_asset_bindings.as_ref(),
            TerrainSourceCandidateState::Unavailable
            | TerrainSourceCandidateState::Disabled { .. }
            | TerrainSourceCandidateState::Rejected { .. } => None,
        }
    }

    /// Creates generation-coherent copied PNG resources for the active,
    /// lowered source terrain plan. This is a private preparation operation:
    /// it deliberately does not construct a program resource set, bind a
    /// pipeline, or make source-selected terrain executable.
    pub(crate) fn ensure_candidate_source_asset_resources(
        &mut self,
        gal: &mut VulkanicGal,
        assets: &ShaderPackAssets,
    ) -> GalResult<bool> {
        let (generation, pack_name, asset_bindings) = match &self.source_candidate {
            TerrainSourceCandidateState::Discovered {
                generation,
                pack_name,
                source_asset_bindings: Some(asset_bindings),
                ..
            } => (*generation, pack_name.as_str(), asset_bindings),
            TerrainSourceCandidateState::Unavailable
            | TerrainSourceCandidateState::Disabled { .. }
            | TerrainSourceCandidateState::Rejected { .. }
            | TerrainSourceCandidateState::Discovered { .. } => return Ok(false),
        };
        if assets.generation() != generation || assets.pack_name() != pack_name {
            return Err(GalError::invalid_argument(format!(
                "shader-pack asset resources '{}'/{} do not match source candidate '{}'/{}",
                assets.pack_name(),
                assets.generation(),
                pack_name,
                generation
            )));
        }
        if self
            .source_asset_resources
            .as_ref()
            .is_some_and(|resources| {
                resources.generation() == generation && resources.pack_name() == pack_name
            })
        {
            return Ok(false);
        }
        let resource_binding_plans = self.source_resource_binding_plans();
        if resource_binding_plans.is_empty() {
            return Ok(false);
        }
        let replacement = TerrainSourceAssetResources::create(
            gal,
            assets,
            asset_bindings,
            &resource_binding_plans,
        )?;
        if let Some(previous) = self.source_asset_resources.replace(replacement) {
            previous.destroy(gal)?;
        }
        Ok(true)
    }

    /// Discards copied PNG preparation when its matching source/assets no
    /// longer form a complete generation. This does not affect the fixture
    /// plan or source-candidate discovery state.
    pub(crate) fn clear_candidate_source_asset_resources(
        &mut self,
        gal: &mut VulkanicGal,
    ) -> GalResult<()> {
        self.discard_vanilla_lightmap_submission(gal);
        if let Some(resources) = self.vanilla_lightmap_residency.take() {
            resources.destroy(gal)?;
        }
        self.clear_candidate_source_shadow_depth_resources(gal)?;
        self.clear_candidate_source_shadow_color_resources(gal)?;
        self.clear_candidate_source_material_texture_resources(gal)?;
        if let Some(previous) = self.source_asset_resources.take() {
            previous.destroy(gal)?;
        }
        Ok(())
    }

    pub(crate) fn has_candidate_source_asset_resources(&self) -> bool {
        self.source_asset_resources.is_some()
    }

    /// Builds the active copied-PNG semantic subset for a concrete world
    /// generation. It is intentionally incomplete when atlas, lightmap,
    /// shadow, or voxel roles have not yet been supplied by other Rust-owned
    /// runtime components.
    pub(crate) fn candidate_source_asset_resource_set(
        &self,
        world_generation: u64,
    ) -> GalResult<Option<TerrainSourceOwnedResourceSet>> {
        let Some(resources) = self.source_asset_resources.as_ref() else {
            return Ok(None);
        };
        let binding_plans = self.source_resource_binding_plans();
        if binding_plans.is_empty() {
            return Ok(None);
        }
        Ok(Some(resources.declared_semantic_resources(
            &binding_plans,
            world_generation,
        )?))
    }

    /// Reports whether the lowered candidate references a semantic resource
    /// role. This keeps world-resource preparation driven by source lowering,
    /// not by pack filenames or backend-specific binding slots.
    pub(crate) fn candidate_source_requires_resource(
        &self,
        role: TerrainSourceResourceRole,
    ) -> bool {
        self.source_required_resource_roles().contains(&role)
    }

    /// Exact-frame source completeness. DH-only inputs participate only once
    /// the frame has real selected-route DH work; the DH program itself still
    /// validates its complete resource plan before any far draw is staged.
    pub(crate) fn candidate_source_missing_resource_roles_for_frame(
        &self,
        prepared: Option<&TerrainSourceOwnedResourceSet>,
        includes_distant_horizons: bool,
    ) -> Vec<TerrainSourceResourceRole> {
        self.candidate_source_missing_resource_roles_for_frame_with_declared_outputs(
            prepared,
            includes_distant_horizons,
            [],
        )
    }

    /// The complete source graph can own an image before an individual
    /// program-local sampler wrapper exists for it. `declared_outputs` covers
    /// exactly those Rust-owned graph targets; it does not authorize a draw,
    /// relax a program's binding validation, or substitute an external
    /// resource. Every consuming program still creates and validates its
    /// precise sampler/resource set when the combined source frame executes.
    pub(crate) fn candidate_source_missing_resource_roles_for_frame_with_declared_outputs(
        &self,
        prepared: Option<&TerrainSourceOwnedResourceSet>,
        includes_distant_horizons: bool,
        declared_outputs: impl IntoIterator<Item = TerrainSourceResourceRole>,
    ) -> Vec<TerrainSourceResourceRole> {
        let declared_outputs = declared_outputs.into_iter().collect::<BTreeSet<_>>();
        self.source_required_resource_roles_for_frame(includes_distant_horizons)
            .into_iter()
            .filter(|role| {
                !declared_outputs.contains(role)
                    && prepared
                        .and_then(|set| set.availability().resource_for(role.clone()))
                        .is_none()
            })
            .collect()
    }

    /// Returns the parity-correct owned voxel subset only after the matching
    /// source candidate and D3 generation are fully confirmed. This is not a
    /// source-program binding and cannot alter route selection.
    pub(crate) fn candidate_colored_light_resource_set(
        &self,
        frame_counter: u64,
    ) -> GalResult<Option<TerrainSourceOwnedResourceSet>> {
        let source_generation = match &self.source_candidate {
            TerrainSourceCandidateState::Discovered { generation, .. } => *generation,
            TerrainSourceCandidateState::Unavailable
            | TerrainSourceCandidateState::Disabled { .. }
            | TerrainSourceCandidateState::Rejected { .. } => return Ok(None),
        };
        let Some(runtime) = self.terrain_colored_light.as_ref() else {
            return Ok(None);
        };
        if runtime.descriptor().shader_pack_generation != source_generation {
            return Err(GalError::invalid_argument(
                "colored voxel-light resources do not match the discovered shader-pack generation",
            ));
        }
        if !runtime.is_ready_for_frame(frame_counter) {
            return Ok(None);
        }
        runtime
            .semantic_resource_set_for_frame(frame_counter)
            .map(Some)
    }

    /// Returns resources which are ordered for the current combined
    /// submission but intentionally not yet confirmed. Callers must append
    /// the producing operations before the terrain draw and either confirm
    /// or discard the same runtime transaction with that submission.
    pub(crate) fn candidate_colored_light_resource_set_for_pending_submission(
        &self,
        frame_counter: u64,
    ) -> GalResult<Option<TerrainSourceOwnedResourceSet>> {
        let source_generation = match &self.source_candidate {
            TerrainSourceCandidateState::Discovered { generation, .. } => *generation,
            TerrainSourceCandidateState::Unavailable
            | TerrainSourceCandidateState::Disabled { .. }
            | TerrainSourceCandidateState::Rejected { .. } => return Ok(None),
        };
        let Some(runtime) = self.terrain_colored_light.as_ref() else {
            return Ok(None);
        };
        if runtime.descriptor().shader_pack_generation != source_generation {
            return Err(GalError::invalid_argument(
                "colored voxel-light resources do not match the discovered shader-pack generation",
            ));
        }
        if !runtime.has_pending_submission() {
            return Ok(None);
        }
        // The first source-owned occupancy frame legitimately uploads the
        // derived emission/tint tables and mapping before it can dispatch or
        // sample flood-fill output. That incomplete transaction is not an
        // asset failure and must not tear down material/color preparation;
        // the normal Rust graph confirms it, then a later exact frame may
        // expose the pending read-after-write resource set.
        if !runtime.pending_sampling_ready_for_frame(frame_counter) {
            return Ok(None);
        }
        runtime
            .semantic_resource_set_for_pending_submission(frame_counter)
            .map(Some)
    }

    /// Returns the source-derived puddle field only after the exact owned
    /// upload has completed. The result is semantic resource metadata; it
    /// neither binds a program nor changes source-route admission.
    pub(crate) fn candidate_puddle_resource_set(
        &self,
    ) -> GalResult<Option<TerrainSourceOwnedResourceSet>> {
        let source_generation = match &self.source_candidate {
            TerrainSourceCandidateState::Discovered { generation, .. } => *generation,
            TerrainSourceCandidateState::Unavailable
            | TerrainSourceCandidateState::Disabled { .. }
            | TerrainSourceCandidateState::Rejected { .. } => return Ok(None),
        };
        if !self.candidate_source_requires_resource(TerrainSourceResourceRole::PuddleOccupancy) {
            return Ok(None);
        }
        let Some(runtime) = self.terrain_puddle.as_ref() else {
            return Ok(None);
        };
        if runtime.descriptor().shader_pack_generation != source_generation {
            return Err(GalError::invalid_argument(
                "puddle occupancy resources do not match the discovered shader-pack generation",
            ));
        }
        if !runtime.is_ready() {
            return Ok(None);
        }
        runtime.semantic_resource_set().map(Some)
    }

    /// Same-submission resource table for the exact ordered puddle upload.
    /// This is valid only while the enclosing world submission remains
    /// pending; confirmation or discard follows that single transaction.
    pub(crate) fn candidate_puddle_resource_set_for_pending_submission(
        &self,
    ) -> GalResult<Option<TerrainSourceOwnedResourceSet>> {
        let source_generation = match &self.source_candidate {
            TerrainSourceCandidateState::Discovered { generation, .. } => *generation,
            TerrainSourceCandidateState::Unavailable
            | TerrainSourceCandidateState::Disabled { .. }
            | TerrainSourceCandidateState::Rejected { .. } => return Ok(None),
        };
        if !self.candidate_source_requires_resource(TerrainSourceResourceRole::PuddleOccupancy) {
            return Ok(None);
        }
        let Some(runtime) = self.terrain_puddle.as_ref() else {
            return Ok(None);
        };
        if runtime.descriptor().shader_pack_generation != source_generation {
            return Err(GalError::invalid_argument(
                "pending puddle occupancy resources do not match the discovered shader-pack generation",
            ));
        }
        if !runtime.has_pending_submission() {
            return Ok(None);
        }
        runtime
            .semantic_resource_set_for_pending_submission()
            .map(Some)
    }

    pub(crate) fn candidate_puddle_diagnostic_state(&self) -> Option<TerrainPuddleDiagnosticState> {
        self.terrain_puddle
            .as_ref()
            .map(TerrainPuddleRuntime::diagnostic_state)
    }

    /// Creates a source-runtime-owned semantic material texture wrapper. The
    /// supplied view and sampler already belong to Rust's world texture cache;
    /// this method owns only the role-specific combined source-resource object.
    pub(crate) fn ensure_candidate_source_material_texture_resources(
        &mut self,
        gal: &mut VulkanicGal,
        input: TerrainSourceMaterialTextureInput,
    ) -> GalResult<Option<TerrainSourceOwnedResourceSet>> {
        if !matches!(
            input.role,
            TerrainSourceResourceRole::MaterialTexture
                | TerrainSourceResourceRole::MaterialAtlas
                | TerrainSourceResourceRole::MaterialNormalMap
                | TerrainSourceResourceRole::MaterialSpecularMap
        ) {
            return Err(GalError::invalid_argument(
                "source material texture wrapper received an unsupported semantic role",
            ));
        }
        if !self.candidate_source_requires_resource(input.role.clone()) {
            self.clear_candidate_source_material_texture_role(gal, &input.role)?;
            return Ok(None);
        }
        if input.shader_pack_generation == 0
            || input.world_generation == 0
            || input.mesh_asset_generation == 0
        {
            return Err(GalError::invalid_argument(
                "source material texture requires non-zero shader-pack, world, and mesh generations",
            ));
        }
        if input.shader_pack_generation != self.expected_shader_pack_generation_for_resources() {
            return Err(GalError::invalid_argument(
                "source material texture shader-pack generation does not match the discovered candidate",
            ));
        }
        if let Some(resources) = self.source_material_texture_resources.get(&input.role) {
            if resources.compatible_with(&input) {
                return resources.semantic_resource_set().map(Some);
            }
        }
        let combined_sampler = gal.create_combined_texture_sampler(CombinedTextureSamplerDesc {
            label: format!(
                "shader-pack.source-material-{}.pack{}.world{}.mesh{}",
                input.role.semantic_name(),
                input.shader_pack_generation,
                input.world_generation,
                input.mesh_asset_generation
            ),
            texture_view: input.texture_view,
            sampler: input.sampler,
        })?;
        let replacement = TerrainSourceMaterialTextureResources {
            role: input.role.clone(),
            shader_pack_generation: input.shader_pack_generation,
            world_generation: input.world_generation,
            mesh_asset_generation: input.mesh_asset_generation,
            combined_sampler,
        };
        if let Some(previous) = self
            .source_material_texture_resources
            .insert(input.role.clone(), replacement)
        {
            gal.destroy(previous.combined_sampler)?;
        }
        self.source_material_texture_resources
            .get(&input.role)
            .expect("material texture wrapper was inserted")
            .semantic_resource_set()
            .map(Some)
    }

    /// Reports whether replacing this semantic wrapper would invalidate
    /// caller-owned program resource sets. The runtime intentionally does
    /// not destroy those sets itself because terrain and DH own distinct
    /// frontend caches; callers must retire their consumers before asking the
    /// runtime to replace the combined sampler.
    pub(crate) fn candidate_source_material_texture_will_replace(
        &self,
        input: &TerrainSourceMaterialTextureInput,
    ) -> bool {
        self.source_material_texture_resources
            .get(&input.role)
            .is_some_and(|resources| !resources.compatible_with(input))
    }

    pub(crate) fn clear_candidate_source_material_texture_role(
        &mut self,
        gal: &mut VulkanicGal,
        role: &TerrainSourceResourceRole,
    ) -> GalResult<()> {
        if let Some(previous) = self.source_material_texture_resources.remove(role) {
            gal.destroy(previous.combined_sampler)?;
        }
        Ok(())
    }

    pub(crate) fn clear_candidate_source_material_texture_resources(
        &mut self,
        gal: &mut VulkanicGal,
    ) -> GalResult<()> {
        let resources = std::mem::take(&mut self.source_material_texture_resources);
        for (_, resource) in resources {
            gal.destroy(resource.combined_sampler)?;
        }
        Ok(())
    }

    /// Creates private comparison samplers for the two source semantic shadow
    /// roles. The source route remains unavailable: this owns only a
    /// generation-coherent GAL resource subset for diagnostics and later
    /// explicit source-program assembly.
    pub(crate) fn ensure_candidate_source_shadow_depth_resources(
        &mut self,
        gal: &mut VulkanicGal,
        input: TerrainSourceShadowDepthInput,
    ) -> GalResult<Option<TerrainSourceOwnedResourceSet>> {
        let requires_primary =
            self.candidate_source_requires_resource(TerrainSourceResourceRole::ShadowDepthPrimary);
        let requires_secondary = self
            .candidate_source_requires_resource(TerrainSourceResourceRole::ShadowDepthSecondary);
        let requires_raw =
            self.candidate_source_requires_resource(TerrainSourceResourceRole::ShadowDepthRaw);
        if !requires_primary && !requires_secondary && !requires_raw {
            self.clear_candidate_source_shadow_depth_resources(gal)?;
            return Ok(None);
        }
        if input.shader_pack_generation == 0
            || input.world_generation == 0
            || input.shader_graph_generation == 0
        {
            return Err(GalError::invalid_argument(
                "source shadow depth requires non-zero shader-pack, world, and shader-graph generations",
            ));
        }
        if input.shader_pack_generation != self.expected_shader_pack_generation_for_resources() {
            return Err(GalError::invalid_argument(
                "source shadow depth shader-pack generation does not match the discovered candidate",
            ));
        }
        if self
            .source_shadow_depth_resources
            .as_ref()
            .is_some_and(|resources| resources.compatible_with(input))
        {
            return self
                .source_shadow_depth_resources
                .as_ref()
                .map(TerrainSourceShadowDepthResources::semantic_resource_set)
                .transpose();
        }

        let label_prefix = format!(
            "shader-pack.source-shadow-depth.pack{}.world{}.graph{}",
            input.shader_pack_generation, input.world_generation, input.shader_graph_generation
        );
        let compare_desc = |label: String| SamplerDesc {
            label,
            min_filter: SamplerFilter::Nearest,
            mag_filter: SamplerFilter::Nearest,
            mip_filter: SamplerFilter::Nearest,
            address_u: SamplerAddressMode::ClampToEdge,
            address_v: SamplerAddressMode::ClampToEdge,
            address_w: SamplerAddressMode::ClampToEdge,
            comparison: Some(CompareOp::LessOrEqual),
        };
        let mut created = Vec::new();
        let result = (|| -> GalResult<TerrainSourceShadowDepthResources> {
            let primary_sampler =
                gal.create_sampler(compare_desc(format!("{label_prefix}.primary")))?;
            created.push(primary_sampler);
            let secondary_sampler =
                gal.create_sampler(compare_desc(format!("{label_prefix}.secondary")))?;
            created.push(secondary_sampler);
            let raw_sampler = if requires_raw {
                let sampler = gal.create_sampler(SamplerDesc {
                    label: format!("{label_prefix}.raw"),
                    min_filter: SamplerFilter::Nearest,
                    mag_filter: SamplerFilter::Nearest,
                    mip_filter: SamplerFilter::Nearest,
                    address_u: SamplerAddressMode::ClampToEdge,
                    address_v: SamplerAddressMode::ClampToEdge,
                    address_w: SamplerAddressMode::ClampToEdge,
                    comparison: None,
                })?;
                created.push(sampler);
                Some(sampler)
            } else {
                None
            };
            let primary_combined_sampler =
                gal.create_combined_texture_sampler(CombinedTextureSamplerDesc {
                    label: format!("{label_prefix}.primary.combined"),
                    texture_view: input.shadow_depth_view,
                    sampler: primary_sampler,
                })?;
            created.push(primary_combined_sampler);
            let secondary_combined_sampler =
                gal.create_combined_texture_sampler(CombinedTextureSamplerDesc {
                    label: format!("{label_prefix}.secondary.combined"),
                    texture_view: input.shadow_depth_view,
                    sampler: secondary_sampler,
                })?;
            created.push(secondary_combined_sampler);
            let raw_combined_sampler = if let Some(raw_sampler) = raw_sampler {
                let combined = gal.create_combined_texture_sampler(CombinedTextureSamplerDesc {
                    label: format!("{label_prefix}.raw.combined"),
                    texture_view: input.shadow_depth_view,
                    sampler: raw_sampler,
                })?;
                created.push(combined);
                Some(combined)
            } else {
                None
            };
            Ok(TerrainSourceShadowDepthResources {
                shader_pack_generation: input.shader_pack_generation,
                world_generation: input.world_generation,
                shader_graph_generation: input.shader_graph_generation,
                primary_sampler,
                secondary_sampler,
                raw_sampler,
                primary_combined_sampler,
                secondary_combined_sampler,
                raw_combined_sampler,
            })
        })();
        let replacement = match result {
            Ok(resources) => resources,
            Err(error) => {
                for handle in created.into_iter().rev() {
                    let _ = gal.destroy(handle);
                }
                return Err(error);
            }
        };
        if let Some(previous) = self.source_shadow_depth_resources.replace(replacement) {
            previous.destroy(gal)?;
        }
        self.source_shadow_depth_resources
            .as_ref()
            .map(TerrainSourceShadowDepthResources::semantic_resource_set)
            .transpose()
    }

    pub(crate) fn clear_candidate_source_shadow_depth_resources(
        &mut self,
        gal: &mut VulkanicGal,
    ) -> GalResult<()> {
        if let Some(previous) = self.source_shadow_depth_resources.take() {
            previous.destroy(gal)?;
        }
        Ok(())
    }

    /// Creates semantic combined samplers for the declared Rust-owned source
    /// shadow-color attachments. This method neither invents an attachment nor
    /// selects source execution: callers provide the matching graph views.
    pub(crate) fn ensure_candidate_source_shadow_color_resources(
        &mut self,
        gal: &mut VulkanicGal,
        input: TerrainSourceShadowColorInput,
    ) -> GalResult<Option<TerrainSourceOwnedResourceSet>> {
        let required = [
            TerrainSourceResourceRole::ShadowColor,
            TerrainSourceResourceRole::ShadowColorSecondary,
        ]
        .into_iter()
        .filter(|role| self.candidate_source_requires_resource(role.clone()))
        .collect::<Vec<_>>();
        if required.is_empty() {
            self.clear_candidate_source_shadow_color_resources(gal)?;
            return Ok(None);
        }
        if input.shader_pack_generation == 0
            || input.world_generation == 0
            || input.shader_graph_generation == 0
        {
            return Err(GalError::invalid_argument(
                "source shadow color requires non-zero shader-pack, world, and shader-graph generations",
            ));
        }
        if input.shader_pack_generation != self.expected_shader_pack_generation_for_resources() {
            return Err(GalError::invalid_argument(
                "source shadow color shader-pack generation does not match the discovered candidate",
            ));
        }
        if self
            .source_shadow_color_resources
            .as_ref()
            .is_some_and(|resources| resources.compatible_with(input))
        {
            return self
                .source_shadow_color_resources
                .as_ref()
                .map(TerrainSourceShadowColorResources::semantic_resource_set)
                .transpose();
        }
        let mut created = Vec::new();
        let replacement = (|| -> GalResult<TerrainSourceShadowColorResources> {
            let mut combined_samplers = BTreeMap::new();
            for role in required {
                let texture_view = match role {
                    TerrainSourceResourceRole::ShadowColor => input.shadow_color_view,
                    TerrainSourceResourceRole::ShadowColorSecondary => {
                        input.shadow_color_secondary_view
                    }
                    _ => unreachable!("shadow-color requirements are bounded above"),
                };
                let combined_sampler =
                    gal.create_combined_texture_sampler(CombinedTextureSamplerDesc {
                        label: format!(
                            "shader-pack.source-{}.pack{}.world{}.graph{}",
                            role.semantic_name(),
                            input.shader_pack_generation,
                            input.world_generation,
                            input.shader_graph_generation
                        ),
                        texture_view,
                        sampler: input.sampler,
                    })?;
                created.push(combined_sampler);
                combined_samplers.insert(role, combined_sampler);
            }
            Ok(TerrainSourceShadowColorResources {
                shader_pack_generation: input.shader_pack_generation,
                world_generation: input.world_generation,
                shader_graph_generation: input.shader_graph_generation,
                shadow_color_view: input.shadow_color_view,
                shadow_color_secondary_view: input.shadow_color_secondary_view,
                sampler: input.sampler,
                combined_samplers,
            })
        })();
        let replacement = match replacement {
            Ok(resources) => resources,
            Err(error) => {
                for handle in created.into_iter().rev() {
                    let _ = gal.destroy(handle);
                }
                return Err(error);
            }
        };
        if let Some(previous) = self.source_shadow_color_resources.replace(replacement) {
            previous.destroy(gal)?;
        }
        self.source_shadow_color_resources
            .as_ref()
            .map(TerrainSourceShadowColorResources::semantic_resource_set)
            .transpose()
    }

    pub(crate) fn clear_candidate_source_shadow_color_resources(
        &mut self,
        gal: &mut VulkanicGal,
    ) -> GalResult<()> {
        if let Some(previous) = self.source_shadow_color_resources.take() {
            previous.destroy(gal)?;
        }
        Ok(())
    }

    /// Binds only confirmed Rust-owned main-depth semantics declared by the
    /// discovered source. A missing temporal snapshot remains absent from the
    /// returned set, allowing the source completeness check to reject the
    /// frame instead of aliasing it to live depth.
    pub(crate) fn ensure_candidate_source_main_depth_resources(
        &mut self,
        gal: &mut VulkanicGal,
        input: TerrainSourceMainDepthInput,
    ) -> GalResult<Option<TerrainSourceOwnedResourceSet>> {
        let mut required = [
            TerrainSourceResourceRole::MainDepth,
            TerrainSourceResourceRole::MainDepthBeforeTranslucency,
            TerrainSourceResourceRole::MainDepthPrevious,
        ]
        .into_iter()
        .filter(|role| self.candidate_source_requires_resource(role.clone()))
        .collect::<Vec<_>>();
        // A complete selected-source plan may discover depth consumption in a
        // retained fullscreen stage after the terrain candidate was observed.
        // The caller supplies the exact Rust-owned G-buffer view here, so keep
        // the current main-depth role available for that same frame rather
        // than allowing the later fullscreen admission to see a stale subset.
        if !required.contains(&TerrainSourceResourceRole::MainDepth) {
            required.push(TerrainSourceResourceRole::MainDepth);
        }
        // The selected-frame planner may discover a temporal sampler while
        // the immutable candidate snapshot is still carrying the prior
        // frame's role set.  An explicitly supplied Rust-owned view is safe
        // to wrap now; final completeness still rejects any undeclared or
        // missing role before execution.
        for (role, view) in [
            (
                TerrainSourceResourceRole::MainDepthBeforeTranslucency,
                input.before_translucency_view,
            ),
            (
                TerrainSourceResourceRole::MainDepthPrevious,
                input.previous_view,
            ),
        ] {
            if view.is_some() && !required.contains(&role) {
                required.push(role);
            }
        }
        if required.is_empty() {
            self.clear_candidate_source_main_depth_resources(gal)?;
            return Ok(None);
        }
        if input.shader_pack_generation == 0
            || input.world_generation == 0
            || input.shader_graph_generation == 0
        {
            return Err(GalError::invalid_argument(
                "source main depth requires non-zero shader-pack, world, and shader-graph generations",
            ));
        }
        if input.shader_pack_generation != self.expected_shader_pack_generation_for_resources() {
            return Err(GalError::invalid_argument(
                "source main depth shader-pack generation does not match the discovered candidate",
            ));
        }
        if self
            .source_main_depth_resources
            .as_ref()
            .is_some_and(|resources| resources.compatible_with(input))
        {
            return self
                .source_main_depth_resources
                .as_ref()
                .map(TerrainSourceMainDepthResources::semantic_resource_set)
                .transpose();
        }

        // A private source frame may make a new post-terrain snapshot
        // available after its base resource assembly already retained
        // `main_depth`. Extend that same generation in place so the old
        // combined sampler stays valid for the exact-frame snapshot. A
        // changed view for an already-owned role is not safe to replace here:
        // callers must retire the prior frame/resource generation first.
        if self
            .source_main_depth_resources
            .as_ref()
            .is_some_and(|resources| resources.base_compatible_with(input))
        {
            let resources = self
                .source_main_depth_resources
                .as_mut()
                .expect("main-depth resource checked before in-place extension");
            for (role, existing_view, requested_view) in [
                (
                    TerrainSourceResourceRole::MainDepthBeforeTranslucency,
                    resources.before_translucency_view,
                    input.before_translucency_view,
                ),
                (
                    TerrainSourceResourceRole::MainDepthPrevious,
                    resources.previous_view,
                    input.previous_view,
                ),
            ] {
                if resources.combined_samplers.contains_key(&role)
                    && requested_view.is_some()
                    && existing_view != requested_view
                {
                    return Err(GalError::invalid_argument(format!(
                        "source main-depth role '{}' cannot replace a view in place for the same generation",
                        role.semantic_name(),
                    )));
                }
            }
            for (role, view) in [
                (
                    TerrainSourceResourceRole::MainDepthBeforeTranslucency,
                    input.before_translucency_view,
                ),
                (
                    TerrainSourceResourceRole::MainDepthPrevious,
                    input.previous_view,
                ),
            ] {
                if !required.contains(&role) || resources.combined_samplers.contains_key(&role) {
                    continue;
                }
                let Some(view) = view else {
                    continue;
                };
                let combined_sampler =
                    gal.create_combined_texture_sampler(CombinedTextureSamplerDesc {
                        label: format!(
                            "shader-pack.source-{}.pack{}.world{}.graph{}",
                            role.semantic_name(),
                            input.shader_pack_generation,
                            input.world_generation,
                            input.shader_graph_generation,
                        ),
                        texture_view: view,
                        sampler: input.sampler,
                    })?;
                resources.combined_samplers.insert(role, combined_sampler);
            }
            if input.before_translucency_view.is_some() {
                resources.before_translucency_view = input.before_translucency_view;
            }
            if input.previous_view.is_some() {
                resources.previous_view = input.previous_view;
            }
            return resources.semantic_resource_set().map(Some);
        }

        let candidates = [
            (
                TerrainSourceResourceRole::MainDepth,
                Some(input.main_depth_view),
            ),
            (
                TerrainSourceResourceRole::MainDepthBeforeTranslucency,
                input.before_translucency_view,
            ),
            (
                TerrainSourceResourceRole::MainDepthPrevious,
                input.previous_view,
            ),
        ];
        let mut created = Vec::new();
        let result = (|| -> GalResult<TerrainSourceMainDepthResources> {
            let mut combined_samplers = BTreeMap::new();
            for (role, view) in candidates {
                if !required.contains(&role) {
                    continue;
                }
                let Some(view) = view else {
                    continue;
                };
                let combined_sampler =
                    gal.create_combined_texture_sampler(CombinedTextureSamplerDesc {
                        label: format!(
                            "shader-pack.source-{}.pack{}.world{}.graph{}",
                            role.semantic_name(),
                            input.shader_pack_generation,
                            input.world_generation,
                            input.shader_graph_generation,
                        ),
                        texture_view: view,
                        sampler: input.sampler,
                    })?;
                created.push(combined_sampler);
                combined_samplers.insert(role, combined_sampler);
            }
            Ok(TerrainSourceMainDepthResources {
                shader_pack_generation: input.shader_pack_generation,
                world_generation: input.world_generation,
                shader_graph_generation: input.shader_graph_generation,
                main_depth_view: input.main_depth_view,
                before_translucency_view: input.before_translucency_view,
                previous_view: input.previous_view,
                sampler: input.sampler,
                combined_samplers,
            })
        })();
        let replacement = match result {
            Ok(resources) => resources,
            Err(error) => {
                for handle in created.into_iter().rev() {
                    let _ = gal.destroy(handle);
                }
                return Err(error);
            }
        };
        if let Some(previous) = self.source_main_depth_resources.replace(replacement) {
            previous.destroy(gal)?;
        }
        self.source_main_depth_resources
            .as_ref()
            .map(TerrainSourceMainDepthResources::semantic_resource_set)
            .transpose()
    }

    pub(crate) fn clear_candidate_source_main_depth_resources(
        &mut self,
        gal: &mut VulkanicGal,
    ) -> GalResult<()> {
        if let Some(previous) = self.source_main_depth_resources.take() {
            previous.destroy(gal)?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn candidate_source_material_atlas_identity(&self) -> Option<(Handle, u64)> {
        self.source_material_texture_resources
            .get(&TerrainSourceResourceRole::MaterialAtlas)
            .map(|resources| (resources.combined_sampler, resources.mesh_asset_generation))
    }

    #[cfg(test)]
    pub(crate) fn candidate_source_material_texture_identity(
        &self,
        role: TerrainSourceResourceRole,
    ) -> Option<(Handle, u64)> {
        self.source_material_texture_resources
            .get(&role)
            .map(|resources| (resources.combined_sampler, resources.mesh_asset_generation))
    }

    #[cfg(test)]
    pub(crate) fn candidate_source_asset_resource_count(&self) -> Option<usize> {
        self.source_asset_resources
            .as_ref()
            .map(TerrainSourceAssetResources::len)
    }

    /// Returns an owned binding only when the currently discovered source
    /// requires it and the exact source generation has a complete matching
    /// colored-light volume. This does not select a source program; pipeline
    /// composition remains the frontend's explicit next step.
    pub(crate) fn candidate_shader_binding(
        &self,
        frame_counter: u64,
    ) -> GalResult<Option<TerrainShaderProgramBinding>> {
        let (source_generation, requires_colored_voxel_light) = match &self.source_candidate {
            TerrainSourceCandidateState::Discovered {
                generation,
                requires_colored_voxel_light,
                ..
            } => (*generation, *requires_colored_voxel_light),
            TerrainSourceCandidateState::Unavailable
            | TerrainSourceCandidateState::Disabled { .. }
            | TerrainSourceCandidateState::Rejected { .. } => return Ok(None),
        };
        if !requires_colored_voxel_light {
            return Ok(None);
        }
        let colored_light = self.terrain_colored_light.as_ref().ok_or_else(|| {
            GalError::invalid_argument(
                "selected terrain source requires a complete owned colored voxel-light volume",
            )
        })?;
        if colored_light.descriptor().shader_pack_generation != source_generation {
            return Err(GalError::invalid_argument(
                "selected terrain source and colored voxel-light volume generations differ",
            ));
        }
        let TerrainVoxelLightSamplingBinding {
            resource_layout,
            resource_set,
            resource_generation,
            ..
        } = colored_light.sampling_binding(frame_counter)?;
        Ok(Some(TerrainShaderProgramBinding {
            resource: TerrainProgramResource::ColoredVoxelLightVolume,
            resource_layout,
            resource_set: TerrainShaderResourceSet {
                set_index: 1,
                set: resource_set,
            },
            resource_generation,
        }))
    }

    /// Prepares the discovered ordinary-entity source only after its entity
    /// contract, lowered pair, and local-material semantic bindings agree.
    /// This is deliberately preparation-only: it cannot compile shaders,
    /// stage a Rust target, allocate a resource set, select a route, or issue
    /// a draw.
    pub(crate) fn prepared_lowered_entity_source_program(
        &self,
    ) -> GalResult<Option<LoweredEntitySourceProgram>> {
        match &self.source_candidate {
            TerrainSourceCandidateState::Unavailable
            | TerrainSourceCandidateState::Disabled { .. }
            | TerrainSourceCandidateState::Rejected { .. } => Ok(None),
            TerrainSourceCandidateState::Discovered {
                entity_contract: Some(contract),
                entity_contract_error: None,
                entity_lowered_pair: Some(lowered),
                entity_source_resource_binding_count: Some(_),
                entity_source_resource_binding_error: None,
                entity_source_resource_bindings: Some(bindings),
                ..
            } => prepare_lowered_entity_source_program(contract, lowered, bindings).map(Some),
            TerrainSourceCandidateState::Discovered {
                entity_contract_error,
                entity_source_resource_binding_error,
                ..
            } => Err(GalError::unsupported_feature(format!(
                "selected entity source preparation is incomplete: {}; {}",
                entity_contract_error
                    .as_deref()
                    .unwrap_or("missing entity source contract or lowered pair"),
                entity_source_resource_binding_error
                    .as_deref()
                    .unwrap_or("missing entity source resource bindings")
            ))),
        }
    }

    /// Prepares the discovered first-person source only after its hand
    /// contract, lowered pair, and owned semantic resource bindings agree.
    /// It remains route-inactive until the dedicated hand writer supplies the
    /// copied first-person projection and depth-clear contract.
    pub(crate) fn prepared_lowered_hand_source_program(
        &self,
    ) -> GalResult<Option<LoweredHandSourceProgram>> {
        match &self.source_candidate {
            TerrainSourceCandidateState::Unavailable
            | TerrainSourceCandidateState::Disabled { .. }
            | TerrainSourceCandidateState::Rejected { .. } => Ok(None),
            TerrainSourceCandidateState::Discovered {
                hand_contract: Some(contract),
                hand_contract_error: None,
                hand_lowered_pair: Some(lowered),
                hand_source_resource_binding_count: Some(_),
                hand_source_resource_binding_error: None,
                hand_source_resource_bindings: Some(bindings),
                ..
            } => prepare_lowered_hand_source_program(contract, lowered, bindings).map(Some),
            TerrainSourceCandidateState::Discovered {
                hand_contract_error,
                hand_source_resource_binding_error,
                ..
            } => Err(GalError::unsupported_feature(format!(
                "selected hand source preparation is incomplete: {}; {}",
                hand_contract_error
                    .as_deref()
                    .unwrap_or("missing hand source contract or lowered pair"),
                hand_source_resource_binding_error
                    .as_deref()
                    .unwrap_or("missing hand source resource bindings")
            ))),
        }
    }

    /// Prepares the exact paired source stages and semantic sampler plan
    /// retained during discovery. This cannot compile a backend program,
    /// allocate a resource layout, select a route, or issue a draw.
    pub(crate) fn prepared_lowered_terrain_source_program(
        &self,
        kind: TerrainMaterialProgramKind,
    ) -> GalResult<Option<LoweredTerrainSourceProgram>> {
        match &self.source_candidate {
            TerrainSourceCandidateState::Unavailable
            | TerrainSourceCandidateState::Disabled { .. }
            | TerrainSourceCandidateState::Rejected { .. } => Ok(None),
            TerrainSourceCandidateState::Discovered {
                contract,
                source_summary: Some(_),
                source_preprocess_error: None,
                source_lowering_summary: Some(_),
                source_lowering_error: None,
                source_uniform_requirement_summary: Some(summary),
                source_uniform_requirement_error: None,
                source_lowered_pair: Some(lowered),
                source_resource_binding_count: Some(_),
                source_resource_binding_error: None,
                source_resource_bindings: Some(bindings),
                ..
            } if summary.field_count == summary.resolved_field_count => {
                prepare_lowered_terrain_source_program(contract, lowered, bindings, kind).map(Some)
            }
            TerrainSourceCandidateState::Discovered {
                contract,
                source_preprocess_error,
                source_lowering_error,
                source_uniform_requirement_summary,
                source_uniform_requirement_error,
                source_resource_binding_error,
                ..
            } => {
                let contract_status = contract
                    .require_selected_subset()
                    .err()
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "otherwise source-contract supported".to_string());
                let uniform_status = source_uniform_requirement_error.clone().or_else(|| {
                    source_uniform_requirement_summary
                        .as_ref()
                        .and_then(|summary| {
                            (summary.field_count != summary.resolved_field_count).then(|| {
                                format!(
                                    "unresolved scalar source uniforms: {}",
                                    summary.unresolved_field_names.join(", ")
                                )
                            })
                        })
                });
                Err(GalError::unsupported_feature(format!(
                    "selected terrain source has no complete paired-stage source contract: {}; source contract: {contract_status}",
                    source_preprocess_error
                        .as_deref()
                        .or(source_lowering_error.as_deref())
                        .or(uniform_status.as_deref())
                        .or(source_resource_binding_error.as_deref())
                        .unwrap_or("missing paired lowering/resource provenance")
                )))
            }
        }
    }

    /// Prepares the independently lowered translucent source stage. Target
    /// allocation and pass construction remain frame-scoped: the caller must
    /// provide the current color history, depth, and blend semantics before
    /// admitting the pass.
    pub(crate) fn prepared_lowered_translucent_terrain_source_program(
        &self,
    ) -> GalResult<Option<LoweredTerrainSourceProgram>> {
        match &self.source_candidate {
            TerrainSourceCandidateState::Unavailable
            | TerrainSourceCandidateState::Disabled { .. }
            | TerrainSourceCandidateState::Rejected { .. } => Ok(None),
            TerrainSourceCandidateState::Discovered {
                translucent_contract: Some(contract),
                translucent_source_summary: Some(_),
                translucent_source_preprocess_error: None,
                translucent_source_lowering_error: None,
                translucent_lowered_pair: Some(lowered),
                translucent_source_resource_binding_count: Some(_),
                translucent_source_resource_binding_error: None,
                translucent_source_resource_bindings: Some(bindings),
                ..
            } => prepare_lowered_translucent_terrain_source_program(contract, lowered, bindings)
                .map(Some),
            TerrainSourceCandidateState::Discovered {
                translucent_contract_error,
                translucent_source_preprocess_error,
                translucent_source_lowering_error,
                translucent_source_resource_binding_error,
                ..
            } => Err(GalError::unsupported_feature(format!(
                "selected terrain source has no complete translucent source contract: {}",
                translucent_source_preprocess_error
                    .as_deref()
                    .or(translucent_source_lowering_error.as_deref())
                    .or(translucent_source_resource_binding_error.as_deref())
                    .or(translucent_contract_error.as_deref())
                    .unwrap_or("missing translucent source lowering/resource provenance")
            ))),
        }
    }

    /// Prepares the exact scoped shadow stages retained during discovery.
    /// This still cannot select source execution: callers must separately
    /// provide a matching shadow attachment/output contract and resource set.
    pub(crate) fn prepared_lowered_shadow_source_program(
        &self,
    ) -> GalResult<Option<LoweredTerrainSourceProgram>> {
        match &self.source_candidate {
            TerrainSourceCandidateState::Unavailable
            | TerrainSourceCandidateState::Disabled { .. }
            | TerrainSourceCandidateState::Rejected { .. } => Ok(None),
            TerrainSourceCandidateState::Discovered {
                generation,
                pack_name,
                source_shadow_summary: Some(_),
                source_shadow_preprocess_error: None,
                source_shadow_lowering_error: None,
                source_lowered_shadow_pair: Some(lowered),
                source_shadow_resource_binding_error: None,
                source_shadow_resource_bindings: Some(bindings),
                ..
            } => {
                let expected_outputs = [
                    ShadowFragmentOutput::ShadowColor,
                    ShadowFragmentOutput::LightShaftColor,
                ];
                if lowered.fragment().outputs() != expected_outputs {
                    return Err(GalError::unsupported_feature(
                        "selected terrain source shadow output contract is not shadow-color plus light-shaft-color",
                    ));
                }
                prepare_lowered_shadow_source_program(pack_name, *generation, lowered, bindings)
                    .map(Some)
            }
            TerrainSourceCandidateState::Discovered {
                source_shadow_preprocess_error,
                source_shadow_lowering_error,
                source_shadow_resource_binding_error,
                ..
            } => Err(GalError::unsupported_feature(format!(
                "selected terrain source has no complete scoped shadow program: {}",
                source_shadow_preprocess_error
                    .as_deref()
                    .or(source_shadow_lowering_error.as_deref())
                    .or(source_shadow_resource_binding_error.as_deref())
                    .unwrap_or("missing shadow lowering/resource provenance")
            ))),
        }
    }

    /// Builds the internal terrain fixture after source discovery. This
    /// exists only for focused tests of resource-generation coherence; it
    /// neither prepares nor executes the selected shader-pack source.
    pub(crate) fn candidate_fixture_terrain_program(
        &self,
        kind: TerrainMaterialProgramKind,
        frame_counter: u64,
    ) -> GalResult<Option<TerrainSourceProgramCandidate>> {
        let (contract, source_vertex_interface) = match &self.source_candidate {
            TerrainSourceCandidateState::Unavailable
            | TerrainSourceCandidateState::Disabled { .. }
            | TerrainSourceCandidateState::Rejected { .. } => return Ok(None),
            TerrainSourceCandidateState::Discovered {
                contract,
                source_summary: Some(_),
                source_preprocess_error: None,
                source_vertex_interface: Some(interface),
                source_lowering_summary: Some(_),
                source_lowering_error: None,
                source_uniform_requirement_summary: Some(summary),
                source_uniform_requirement_error: None,
                source_lowered_pair: Some(_),
                source_resource_binding_count: Some(_),
                source_resource_binding_error: None,
                source_resource_bindings: Some(_),
                ..
            } if summary.field_count == summary.resolved_field_count => (contract, interface),
            TerrainSourceCandidateState::Discovered {
                contract,
                source_preprocess_error,
                source_lowering_error,
                source_uniform_requirement_summary,
                source_uniform_requirement_error,
                source_resource_binding_error,
                ..
            } => {
                let contract_status = contract
                    .require_selected_subset()
                    .err()
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "otherwise source-contract supported".to_string());
                let uniform_status = source_uniform_requirement_error.clone().or_else(|| {
                    source_uniform_requirement_summary
                        .as_ref()
                        .and_then(|summary| {
                            (summary.field_count != summary.resolved_field_count).then(|| {
                                format!(
                                    "unresolved scalar source uniforms: {}",
                                    summary.unresolved_field_names.join(", ")
                                )
                            })
                        })
                });
                return Err(GalError::unsupported_feature(format!(
                    "selected terrain source has no complete paired-stage source contract: {}; source contract: {contract_status}",
                    source_preprocess_error
                        .as_deref()
                        .or(source_lowering_error.as_deref())
                        .or(uniform_status.as_deref())
                        .or(source_resource_binding_error.as_deref())
                        .unwrap_or("missing paired lowering/resource provenance")
                )));
            }
        };
        let binding = self.candidate_shader_binding(frame_counter)?;
        let readiness = if contract
            .required_resources
            .contains(&TerrainPassRequiredResource::ColoredVoxelLightVolume)
        {
            Some(
                self.terrain_colored_light
                    .as_ref()
                    .ok_or_else(|| {
                        GalError::invalid_argument(
                            "selected terrain source requires a complete owned colored voxel-light volume",
                        )
                    })?
                    .readiness()?,
            )
        } else {
            None
        };
        let program = complementary_terrain_subset_program_with_resources(
            contract,
            kind,
            readiness.as_ref(),
            frame_counter,
        )?;
        // Keep source admission honest if the source contract becomes
        // lowerable before the shared mesh ABI grows the required semantics.
        // The check remains after contract admission so today's explicit
        // `UnloweredTerrainSource` status is retained as the primary reason.
        source_vertex_interface.require_current_world_mesh_support()?;
        if program.requires(TerrainProgramResource::ColoredVoxelLightVolume) != binding.is_some() {
            return Err(GalError::invalid_argument(
                "selected terrain source program/resource binding requirements disagree",
            ));
        }
        Ok(Some(TerrainSourceProgramCandidate { program, binding }))
    }

    /// Prepares the complete semantic identity for an owned colored-light
    /// runtime. The source contract owns extent/format/material/emission
    /// policy; the caller supplies only copied world/resource/camera facts.
    pub(crate) fn candidate_colored_light_preparation(
        &self,
        world_generation: u64,
        resource_generation: u64,
        camera_world_position: [f32; 3],
    ) -> GalResult<Option<TerrainColoredLightPreparation>> {
        let (generation, pack_name, contract, requires_colored_voxel_light, materials, emission) =
            match &self.source_candidate {
                TerrainSourceCandidateState::Unavailable
                | TerrainSourceCandidateState::Disabled { .. }
                | TerrainSourceCandidateState::Rejected { .. } => return Ok(None),
                TerrainSourceCandidateState::Discovered {
                    generation,
                    pack_name,
                    contract,
                    requires_colored_voxel_light,
                    voxel_materials,
                    voxel_emission,
                    ..
                } => (
                    *generation,
                    pack_name,
                    contract,
                    *requires_colored_voxel_light,
                    voxel_materials,
                    voxel_emission,
                ),
            };
        if !requires_colored_voxel_light {
            return Ok(None);
        }
        if generation != contract.generation || world_generation == 0 || resource_generation == 0 {
            return Err(GalError::invalid_argument(
                "colored voxel-light preparation requires matching non-zero source, world, and resource generations",
            ));
        }
        let requirements = contract.voxel_light_volume_requirements.ok_or_else(|| {
            GalError::invalid_argument(
                "selected terrain source requires ColoredVoxelLighting without a volume descriptor",
            )
        })?;
        let materials = materials.as_ref().ok_or_else(|| {
            GalError::invalid_argument("selected terrain source has no derived voxel material map")
        })?;
        let emission = emission.as_ref().ok_or_else(|| {
            GalError::invalid_argument(
                "selected terrain source has no derived colored-light emission table",
            )
        })?;
        let mut camera_cell = [0_i32; 3];
        let mut camera_fraction = [0.0_f32; 3];
        for axis in 0..3 {
            let coordinate = camera_world_position[axis];
            if !coordinate.is_finite() {
                return Err(GalError::invalid_argument(
                    "colored voxel-light camera position must be finite",
                ));
            }
            let cell = coordinate.floor();
            if cell < i32::MIN as f32 || cell > i32::MAX as f32 {
                return Err(GalError::invalid_argument(
                    "colored voxel-light camera cell is outside i32 range",
                ));
            }
            camera_cell[axis] = cell as i32;
            camera_fraction[axis] = coordinate - cell;
        }
        let descriptor = VoxelLightVolumeDescriptor {
            identity: VoxelLightVolumeIdentity::new(format!(
                "shader-pack:{}/colored-voxel-light",
                pack_name.to_ascii_lowercase()
            ))?,
            shader_pack_generation: generation,
            world_generation,
            resource_generation,
            extent: requirements.extent,
            requirements,
            mapping: VoxelLightVolumeMapping::complementary(
                requirements.extent,
                camera_cell,
                camera_fraction,
            )?,
        };
        descriptor.validate()?;
        Ok(Some(TerrainColoredLightPreparation {
            descriptor,
            materials: materials.clone(),
            emission: emission.clone(),
        }))
    }

    pub(crate) fn install_private_terrain_occupancy(
        &mut self,
        gal: &mut VulkanicGal,
        descriptor: VoxelLightVolumeDescriptor,
        materials: VoxelMaterialMap,
    ) -> GalResult<()> {
        if descriptor.shader_pack_generation != self.expected_shader_pack_generation_for_resources()
        {
            return Err(GalError::invalid_argument(
                "private terrain occupancy generation must match its shader runtime",
            ));
        }
        let replacement = TerrainOccupancyRuntime::create(gal, descriptor, materials)?;
        if let Some(previous) = self.terrain_colored_light.take() {
            previous.destroy(gal)?;
        }
        if let Some(previous) = self.terrain_occupancy.replace(replacement) {
            previous.destroy(gal)?;
        }
        Ok(())
    }

    pub(crate) fn install_private_terrain_colored_light(
        &mut self,
        gal: &mut VulkanicGal,
        descriptor: VoxelLightVolumeDescriptor,
        materials: VoxelMaterialMap,
        emission: VoxelEmissionTable,
    ) -> GalResult<()> {
        if descriptor.shader_pack_generation != self.expected_shader_pack_generation_for_resources()
        {
            return Err(GalError::invalid_argument(
                "private colored-light generation must match its shader runtime",
            ));
        }
        let replacement = TerrainColoredLightRuntime::create(gal, descriptor, materials, emission)?;
        if let Some(previous) = self.terrain_occupancy.take() {
            previous.destroy(gal)?;
        }
        if let Some(previous) = self.terrain_colored_light.replace(replacement) {
            previous.destroy(gal)?;
        }
        Ok(())
    }

    /// Installs a complete source-derived colored-light generation exactly
    /// once. A changed source/world/resource descriptor replaces the previous
    /// owned runtime atomically; an identical descriptor keeps persistent D3
    /// resources alive for bounded per-frame updates.
    pub(crate) fn ensure_candidate_colored_light_runtime(
        &mut self,
        gal: &mut VulkanicGal,
        preparation: TerrainColoredLightPreparation,
    ) -> GalResult<bool> {
        if self.terrain_colored_light.as_ref().is_some_and(|runtime| {
            runtime
                .descriptor()
                .resource_compatible_with(&preparation.descriptor)
        }) {
            return Ok(false);
        }
        self.install_private_terrain_colored_light(
            gal,
            preparation.descriptor,
            preparation.materials,
            preparation.emission,
        )?;
        Ok(true)
    }

    /// Installs or reuses the private source-derived puddle field. Its stable
    /// resource identity excludes camera fraction and shadow transform, which
    /// are content updates to the same owned image; changing any generation
    /// replaces the image atomically.
    pub(crate) fn ensure_candidate_puddle_runtime(
        &mut self,
        gal: &mut VulkanicGal,
        descriptor: PuddleOccupancyDescriptor,
    ) -> GalResult<bool> {
        if !self.candidate_source_requires_resource(TerrainSourceResourceRole::PuddleOccupancy) {
            self.clear_candidate_puddle_runtime(gal)?;
            return Ok(false);
        }
        if descriptor.shader_pack_generation != self.expected_shader_pack_generation_for_resources()
        {
            return Err(GalError::invalid_argument(
                "private puddle occupancy generation must match its shader runtime",
            ));
        }
        if self
            .terrain_puddle
            .as_ref()
            .is_some_and(|runtime| runtime.resource_compatible_with(descriptor))
        {
            return Ok(false);
        }
        let replacement = TerrainPuddleRuntime::create(gal, descriptor)?;
        if let Some(previous) = self.terrain_puddle.replace(replacement) {
            previous.destroy(gal)?;
        }
        Ok(true)
    }

    pub(crate) fn clear_candidate_puddle_runtime(
        &mut self,
        gal: &mut VulkanicGal,
    ) -> GalResult<()> {
        if let Some(previous) = self.terrain_puddle.take() {
            previous.destroy(gal)?;
        }
        Ok(())
    }

    /// Removes only a source-candidate preparation runtime. This does not
    /// select or deselect any terrain program; callers use it when the
    /// semantic source contract or world-volume mapping is no longer valid.
    pub(crate) fn clear_candidate_colored_light_runtime(
        &mut self,
        gal: &mut VulkanicGal,
    ) -> GalResult<()> {
        if let Some(previous) = self.terrain_colored_light.take() {
            previous.destroy(gal)?;
        }
        Ok(())
    }

    pub(crate) fn expected_shader_pack_generation_for_resources(&self) -> u64 {
        match &self.source_candidate {
            TerrainSourceCandidateState::Discovered { generation, .. } => *generation,
            _ => self.plan.generation,
        }
    }

    pub(crate) fn has_private_terrain_occupancy(&self) -> bool {
        self.terrain_occupancy.is_some()
            || self.terrain_colored_light.is_some()
            || self.terrain_puddle.is_some()
    }

    /// Bounded admission diagnostics for the Rust-owned colored-light
    /// runtime. These values expose no resource handles or backend state and
    /// do not affect source-route selection.
    pub(crate) fn candidate_colored_light_diagnostic_state(
        &self,
        frame_counter: u64,
    ) -> Option<TerrainColoredLightDiagnosticState> {
        self.terrain_colored_light
            .as_ref()
            .map(|runtime| runtime.diagnostic_state(frame_counter))
    }

    #[cfg(test)]
    pub(crate) fn private_terrain_occupancy_mesh_count(&self) -> Option<usize> {
        self.terrain_occupancy
            .as_ref()
            .map(TerrainOccupancyRuntime::mesh_snapshot_count)
            .or_else(|| {
                self.terrain_colored_light
                    .as_ref()
                    .map(TerrainColoredLightRuntime::mesh_snapshot_count)
            })
    }

    pub(crate) fn private_terrain_occupancy_descriptor(
        &self,
    ) -> Option<&VoxelLightVolumeDescriptor> {
        self.terrain_occupancy
            .as_ref()
            .map(TerrainOccupancyRuntime::descriptor)
            .or_else(|| {
                self.terrain_colored_light
                    .as_ref()
                    .map(TerrainColoredLightRuntime::descriptor)
            })
    }

    #[cfg(test)]
    pub(crate) fn private_terrain_occupancy_mapping(&self) -> Option<VoxelLightVolumeMapping> {
        self.private_terrain_occupancy_descriptor()
            .map(|descriptor| descriptor.mapping)
    }

    #[cfg(test)]
    pub(crate) fn private_terrain_colored_light_ready(&self, frame_counter: u64) -> Option<bool> {
        self.terrain_colored_light
            .as_ref()
            .map(|runtime| runtime.is_ready_for_frame(frame_counter))
    }

    pub(crate) fn append_private_terrain_occupancy(
        &mut self,
        frame_counter: u64,
        mapping: Option<VoxelLightVolumeMapping>,
        view_direction: Option<VoxelLightVolumeViewDirection>,
        puddle_descriptor: Option<PuddleOccupancyDescriptor>,
        meshes: impl IntoIterator<Item = TerrainVoxelSourceMesh>,
        operations: &mut Vec<CommandOp>,
    ) -> GalResult<()> {
        let meshes = meshes.into_iter().collect::<Vec<_>>();
        if let Some(colored_light) = self.terrain_colored_light.as_mut() {
            let mapping = mapping.ok_or_else(|| {
                GalError::invalid_argument(
                    "colored voxel-light preparation requires a semantic volume mapping",
                )
            })?;
            colored_light.append_terrain_source_snapshot_for_mapping(
                frame_counter,
                mapping,
                view_direction,
                meshes.iter().cloned(),
                operations,
            )?;
        } else if let Some(occupancy) = self.terrain_occupancy.as_mut() {
            let mapping = mapping.ok_or_else(|| {
                GalError::invalid_argument(
                    "terrain occupancy preparation requires a semantic volume mapping",
                )
            })?;
            occupancy.append_terrain_source_snapshot_for_mapping(
                mapping,
                meshes.iter().cloned(),
                operations,
            )?;
        }
        if let Some(puddle) = self.terrain_puddle.as_mut() {
            let descriptor = puddle_descriptor.ok_or_else(|| {
                GalError::invalid_argument(
                    "puddle occupancy preparation requires a semantic shadow-scene descriptor",
                )
            })?;
            puddle.append_terrain_source_snapshot(descriptor, meshes, operations)?;
        }
        Ok(())
    }

    pub(crate) fn confirm_private_terrain_occupancy_submission(&mut self) -> GalResult<()> {
        if let Some(colored_light) = self.terrain_colored_light.as_mut() {
            if colored_light.has_pending_submission() {
                colored_light.confirm_submission()?;
            }
        } else if let Some(occupancy) = self.terrain_occupancy.as_mut() {
            if occupancy.has_pending_submission() {
                occupancy.confirm_submission()?;
            }
        }
        if let Some(puddle) = self.terrain_puddle.as_mut() {
            if puddle.has_pending_submission() {
                puddle.confirm_submission()?;
            }
        }
        Ok(())
    }

    pub(crate) fn has_pending_private_terrain_occupancy_submission(&self) -> bool {
        self.terrain_colored_light
            .as_ref()
            .is_some_and(TerrainColoredLightRuntime::has_pending_submission)
            || self
                .terrain_occupancy
                .as_ref()
                .is_some_and(TerrainOccupancyRuntime::has_pending_submission)
            || self
                .terrain_puddle
                .as_ref()
                .is_some_and(TerrainPuddleRuntime::has_pending_submission)
    }

    pub(crate) fn discard_private_terrain_occupancy_submission(&mut self) {
        if let Some(colored_light) = self.terrain_colored_light.as_mut() {
            colored_light.discard_submission();
        } else if let Some(occupancy) = self.terrain_occupancy.as_mut() {
            occupancy.discard_submission();
        }
        if let Some(puddle) = self.terrain_puddle.as_mut() {
            puddle.discard_submission();
        }
    }

    pub(crate) fn destroy(mut self, gal: &mut VulkanicGal) -> GalResult<()> {
        self.discard_vanilla_lightmap_submission(gal);
        if let Some(resources) = self.vanilla_lightmap_residency.take() {
            resources.destroy(gal)?;
        }
        self.vanilla_lightmap.clear();
        if let Some(resources) = self.source_shadow_color_resources.take() {
            resources.destroy(gal)?;
        }
        if let Some(resources) = self.source_main_depth_resources.take() {
            resources.destroy(gal)?;
        }
        if let Some(resources) = self.source_shadow_depth_resources.take() {
            resources.destroy(gal)?;
        }
        self.clear_candidate_source_material_texture_resources(gal)?;
        self.source_color_resources.destroy(gal);
        self.source_color_targets.destroy(gal);
        if let Some(resources) = self.source_asset_resources.take() {
            resources.destroy(gal)?;
        }
        if let Some(occupancy) = self.terrain_occupancy.take() {
            occupancy.destroy(gal)?;
        }
        if let Some(colored_light) = self.terrain_colored_light.take() {
            colored_light.destroy(gal)?;
        }
        if let Some(puddle) = self.terrain_puddle.take() {
            puddle.destroy(gal)?;
        }
        Ok(())
    }

    pub(crate) fn append_terrain_material_graph(
        &self,
        ops: &mut Vec<CommandOp>,
        targets: TerrainRuntimeTargets,
        frame: TerrainRuntimeFrame,
        draws: &[TerrainMeshDraw],
        forward_material_draws: &[TerrainForwardMaterialDraw],
    ) -> GalResult<()> {
        self.validate_terrain_material_graph()?;
        if draws.is_empty() {
            return Ok(());
        }
        let mut targets = targets;
        targets.translucent_capture_initialized = frame.translucent_capture_initialized;
        let isolation = TerrainGraphIsolation::from_env();
        let effective_draws_storage;
        let effective_draws = if isolation == TerrainGraphIsolation::FullDrawsSkipped {
            effective_draws_storage = Vec::new();
            effective_draws_storage.as_slice()
        } else {
            draws
        };

        if matches!(
            isolation,
            TerrainGraphIsolation::Full
                | TerrainGraphIsolation::TerrainPlusShadow
                | TerrainGraphIsolation::FullDrawsSkipped
        ) {
            self.append_shadow_depth_pass(
                ops,
                targets.into(),
                effective_draws,
                frame.shadow_targets_initialized,
            )?;
        } else if isolation == TerrainGraphIsolation::GBufferNoShadow {
            // The deferred graph still samples the explicit shadow-depth
            // attachment even when this diagnostic graph excludes shadow
            // geometry.  Initialize it through the same owned pass contract
            // with an empty draw list; leaving it Undefined would violate
            // Vulkan's sampled-image layout requirement.
            self.append_shadow_depth_pass(
                ops,
                targets.into(),
                &[],
                frame.shadow_targets_initialized,
            )?;
        }
        self.append_g_buffer_passes(
            ops,
            targets,
            frame.background_color,
            effective_draws,
            isolation == TerrainGraphIsolation::FullDrawsSkipped,
            frame.screen_targets_initialized,
            frame.g_buffer_background_initialized,
        )?;
        if let Some(history) = frame.depth_history {
            Self::append_main_depth_history(ops, targets.depth_history, history)?;
        }
        if matches!(
            isolation,
            TerrainGraphIsolation::Full
                | TerrainGraphIsolation::GBufferNoShadow
                | TerrainGraphIsolation::FullDrawsSkipped
        ) {
            self.append_deferred_and_composites(
                ops,
                targets,
                frame,
                effective_draws,
                forward_material_draws,
            )?;
        }
        Ok(())
    }

    /// Retains explicit main-depth snapshots at the boundary immediately
    /// after opaque/cutout G-buffer work and before the translucent pass.
    /// This is a semantic texture-to-texture operation: GAL validates the
    /// transfer usages and both backends lower the same command privately.
    pub(crate) fn append_main_depth_history(
        ops: &mut Vec<CommandOp>,
        targets: TerrainDepthHistoryTargets,
        history: TerrainDepthHistoryPlan,
    ) -> GalResult<()> {
        if matches!(
            std::env::var("MATTMC_RUST_SOURCE_DEPTH_TRACE").as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE")
        ) {
            eprintln!(
                "[MattMC source-depth-trace] history-copy main_texture=0x{:016x} before_texture=0x{:016x} previous_texture=0x{:016x}",
                targets.main_depth_texture.raw(),
                targets.before_translucency_texture.raw(),
                targets.previous_texture.raw(),
            );
        }
        if history.extent.width == 0 || history.extent.height == 0 || history.extent.depth != 1 {
            return Err(GalError::invalid_argument(
                "main depth history requires a non-zero 2D extent",
            ));
        }
        let full_copy = |src_texture, dst_texture| TextureImageCopyRegion {
            row_order: crate::render::vulkanic::commands::TextureRowOrder::Preserve,
            src_texture,
            src_mip: 0,
            src_layer: 0,
            src_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
            dst_texture,
            dst_mip: 0,
            dst_layer: 0,
            dst_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
            extent: history.extent,
        };

        if history.prior_before_translucency_valid {
            ops.push(CommandOp::Barrier(texture_barrier(
                targets.before_translucency_texture,
                TextureUsageState::ShaderRead,
                TextureUsageState::TransferSrc,
            )));
            ops.push(CommandOp::Barrier(texture_barrier(
                targets.previous_texture,
                if history.prior_previous_valid {
                    TextureUsageState::ShaderRead
                } else {
                    TextureUsageState::Undefined
                },
                TextureUsageState::TransferDst,
            )));
            ops.push(CommandOp::CopyTexture(full_copy(
                targets.before_translucency_texture,
                targets.previous_texture,
            )));
            ops.push(CommandOp::Barrier(texture_barrier(
                targets.previous_texture,
                TextureUsageState::TransferDst,
                TextureUsageState::ShaderRead,
            )));
            ops.push(CommandOp::Barrier(texture_barrier(
                targets.before_translucency_texture,
                TextureUsageState::TransferSrc,
                TextureUsageState::ShaderRead,
            )));
        }

        ops.push(CommandOp::Barrier(texture_barrier(
            targets.main_depth_texture,
            TextureUsageState::ShaderRead,
            TextureUsageState::TransferSrc,
        )));
        ops.push(CommandOp::Barrier(texture_barrier(
            targets.before_translucency_texture,
            if history.prior_before_translucency_valid {
                TextureUsageState::ShaderRead
            } else {
                TextureUsageState::Undefined
            },
            TextureUsageState::TransferDst,
        )));
        ops.push(CommandOp::CopyTexture(full_copy(
            targets.main_depth_texture,
            targets.before_translucency_texture,
        )));
        ops.push(CommandOp::Barrier(texture_barrier(
            targets.before_translucency_texture,
            TextureUsageState::TransferDst,
            TextureUsageState::ShaderRead,
        )));
        ops.push(CommandOp::Barrier(texture_barrier(
            targets.main_depth_texture,
            TextureUsageState::TransferSrc,
            TextureUsageState::ShaderRead,
        )));
        Ok(())
    }

    /// Exercises one validated source-derived G-buffer transaction without
    /// selecting a gameplay route or claiming the incomplete source shadow
    /// and composite passes. This is intentionally test-only: production
    /// callers must enter through the complete runtime graph.
    #[cfg(test)]
    pub(crate) fn append_terrain_g_buffer_for_test(
        &self,
        ops: &mut Vec<CommandOp>,
        targets: TerrainRuntimeTargets,
        background_color: ClearColor,
        draws: &[TerrainMeshDraw],
    ) -> GalResult<()> {
        self.validate_terrain_material_graph()?;
        self.append_g_buffer_passes(ops, targets, background_color, draws, false, false, false)
    }

    /// Appends one normal-terrain source pass to its named shader-pack color
    /// targets. This is private scheduling infrastructure only: the caller
    /// must still own complete source-graph admission, source-color history,
    /// shadow/DH/fullscreen ordering, and the eventual one-presenter route.
    /// It intentionally has no fallback or route-selection behavior.
    pub(crate) fn append_terrain_source_color_pass(
        &self,
        ops: &mut Vec<CommandOp>,
        targets: &TerrainSourceColorPassTargets,
        draws: &[TerrainMeshDraw],
    ) -> GalResult<()> {
        if matches!(
            std::env::var("MATTMC_RUST_SOURCE_DEPTH_TRACE").as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE")
        ) {
            eprintln!(
                "[MattMC source-depth-trace] source-pass phase={:?} depth_texture=0x{:016x} depth_view=0x{:016x} draws={}",
                targets.phase,
                targets.depth_texture.raw(),
                targets.depth_view.raw(),
                draws.len(),
            );
        }
        if matches!(
            std::env::var("MATTMC_RUST_SOURCE_DEPTH_TRACE").as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE")
        ) {
            eprintln!(
                "[MattMC source-depth-trace] terrain depth_texture=0x{:016x} depth_view=0x{:016x} phase={:?}",
                targets.depth_texture.raw(),
                targets.depth_view.raw(),
                targets.phase,
            );
        }
        if targets.color_attachments.is_empty() {
            return Err(GalError::invalid_argument(
                "source terrain color pass requires at least one named color attachment",
            ));
        }
        let mut seen_slots = std::collections::BTreeSet::new();
        let mut seen_outputs = std::collections::BTreeSet::new();
        for attachment in &targets.color_attachments {
            if !seen_slots.insert(attachment.source_slot) || !seen_outputs.insert(attachment.output)
            {
                return Err(GalError::invalid_argument(
                    "source terrain color pass has duplicate named output attachments",
                ));
            }
            ops.push(CommandOp::Barrier(texture_barrier(
                attachment.texture,
                TextureUsageState::ShaderRead,
                TextureUsageState::ColorAttachment,
            )));
        }
        ops.push(CommandOp::Barrier(texture_barrier(
            targets.depth_texture,
            targets.phase.depth_before(),
            TextureUsageState::DepthStencilAttachment,
        )));
        ops.push(CommandOp::BeginPass {
            pass: targets.pass,
            target: targets.target,
            colors: targets
                .color_attachments
                .iter()
                .map(|attachment| PassAttachment {
                    view: attachment.view,
                    load_op: targets.phase.color_load_op(attachment),
                    store_op: AttachmentStoreOp::Store,
                    clear_color: matches!(
                        targets.phase.color_load_op(attachment),
                        AttachmentLoadOp::Clear
                    )
                    .then(|| {
                        source_color_clear_color(
                            attachment.source_slot,
                            attachment.clear_color_bits,
                            targets.clear_values.fog_color,
                        )
                    }),
                })
                .collect(),
            depth_stencil: Some(PassAttachment {
                view: targets.depth_view,
                load_op: targets.phase.depth_load_op(),
                store_op: AttachmentStoreOp::Store,
                clear_color: None,
            }),
        });
        let accepted_draws = draws
            .iter()
            .filter(|draw| targets.phase.accepts_material(draw.material_mode))
            .collect::<Vec<_>>();
        if matches!(
            std::env::var("MATTMC_RUST_SOURCE_DEPTH_TRACE").as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE")
        ) {
            eprintln!(
                "[MattMC source-depth-trace] source-pass phase={:?} accepted_draws={} attachments={:?}",
                targets.phase,
                accepted_draws.len(),
                targets
                    .color_attachments
                    .iter()
                    .map(|attachment| (
                        attachment.role.shader_pack_color_name(),
                        attachment.source_slot,
                    ))
                    .collect::<Vec<_>>(),
            );
        }
        let mut draw_state = IndexedDrawState::default();
        for draw in accepted_draws {
            append_indexed_draw(
                ops,
                &mut draw_state,
                draw.pipeline,
                draw.pipeline_layout,
                draw.resource_set,
                &draw.resource_set_dynamic_offsets,
                draw.shader_resource_set,
                draw.index_buffer,
                draw.index_offset,
                draw.index_type,
                draw.index_count,
                draw.instance_count,
            );
        }
        ops.push(CommandOp::EndPass);
        for attachment in &targets.color_attachments {
            ops.push(CommandOp::Barrier(texture_barrier(
                attachment.texture,
                TextureUsageState::ColorAttachment,
                TextureUsageState::ShaderRead,
            )));
        }
        ops.push(CommandOp::Barrier(texture_barrier(
            targets.depth_texture,
            TextureUsageState::DepthStencilAttachment,
            TextureUsageState::ShaderRead,
        )));
        Ok(())
    }

    /// Appends the Rust-owned `gbuffers_textured` writer against the selected
    /// pack's named targets. It is intentionally load-only and does not own
    /// source-color transaction completion, route selection, or presentation.
    /// Those remain with the combined source-frame coordinator.
    pub(crate) fn append_textured_material_source_color_pass(
        &self,
        ops: &mut Vec<CommandOp>,
        targets: &TerrainSourceColorPassTargets,
        draws: &[TexturedMaterialSourceDraw],
    ) -> GalResult<()> {
        self.append_source_material_color_pass(
            ops,
            targets,
            draws,
            TerrainSourceColorPassPhase::TexturedMaterial,
            "textured material",
        )
    }

    /// Appends the separate source-derived weather writer. It has no target,
    /// presentation, or transaction ownership outside this combined frame;
    /// the phase merely preserves its distinct alpha-over source contract.
    pub(crate) fn append_weather_source_color_pass(
        &self,
        ops: &mut Vec<CommandOp>,
        targets: &TerrainSourceColorPassTargets,
        draws: &[TexturedMaterialSourceDraw],
    ) -> GalResult<()> {
        self.append_source_material_color_pass(
            ops,
            targets,
            draws,
            TerrainSourceColorPassPhase::Weather,
            "weather",
        )
    }

    /// Appends the source-derived cloud writer against the same Rust-owned
    /// named targets. The caller owns route selection and transaction
    /// completion; this only records the explicit load-only pass.
    pub(crate) fn append_cloud_source_color_pass(
        &self,
        ops: &mut Vec<CommandOp>,
        targets: &TerrainSourceColorPassTargets,
        draws: &[TexturedMaterialSourceDraw],
    ) -> GalResult<()> {
        self.append_source_material_color_pass(
            ops,
            targets,
            draws,
            TerrainSourceColorPassPhase::Clouds,
            "clouds",
        )
    }

    /// Appends one Rust-owned indexed entity writer against the selected
    /// shader-pack targets. The pass is intentionally load-only and has no
    /// route, target, transaction, or presentation ownership outside the
    /// combined source-frame coordinator.
    pub(crate) fn append_entity_source_color_pass(
        &self,
        ops: &mut Vec<CommandOp>,
        targets: &TerrainSourceColorPassTargets,
        draws: &[EntitySourceDraw],
    ) -> GalResult<()> {
        self.append_indexed_source_color_pass(
            ops,
            targets,
            draws,
            TerrainSourceColorPassPhase::Entities,
            "entity",
        )
    }

    /// Records the separate Rust-owned `gbuffers_hand` pass. It shares the
    /// explicit indexed source stream with entity meshes, but its pass phase
    /// clears a fresh depth domain while loading the completed world colors.
    /// It has no route or presentation ownership outside the source-frame
    /// coordinator.
    pub(crate) fn append_hand_source_color_pass(
        &self,
        ops: &mut Vec<CommandOp>,
        targets: &TerrainSourceColorPassTargets,
        draws: &[EntitySourceDraw],
    ) -> GalResult<()> {
        self.append_indexed_source_color_pass(
            ops,
            targets,
            draws,
            TerrainSourceColorPassPhase::Hands,
            "hand",
        )
    }

    fn append_indexed_source_color_pass(
        &self,
        ops: &mut Vec<CommandOp>,
        targets: &TerrainSourceColorPassTargets,
        draws: &[EntitySourceDraw],
        expected_phase: TerrainSourceColorPassPhase,
        writer: &str,
    ) -> GalResult<()> {
        if matches!(
            std::env::var("MATTMC_RUST_SOURCE_DEPTH_TRACE").as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE")
        ) {
            eprintln!(
                "[MattMC source-depth-trace] indexed-pass writer={} phase={:?} depth_texture=0x{:016x} depth_view=0x{:016x} draws={}",
                writer, targets.phase, targets.depth_texture.raw(), targets.depth_view.raw(), draws.len(),
            );
        }
        if targets.phase != expected_phase {
            return Err(GalError::invalid_argument(format!(
                "{writer} source draw requires its explicit source pass phase",
            )));
        }
        if targets.color_attachments.is_empty() {
            return Err(GalError::invalid_argument(format!(
                "{writer} source pass requires at least one named color attachment",
            )));
        }
        let mut seen_slots = std::collections::BTreeSet::new();
        let mut seen_outputs = std::collections::BTreeSet::new();
        for attachment in &targets.color_attachments {
            if !seen_slots.insert(attachment.source_slot) || !seen_outputs.insert(attachment.output)
            {
                return Err(GalError::invalid_argument(format!(
                    "{writer} source pass has duplicate named output attachments",
                )));
            }
            ops.push(CommandOp::Barrier(texture_barrier(
                attachment.texture,
                TextureUsageState::ShaderRead,
                TextureUsageState::ColorAttachment,
            )));
        }
        ops.push(CommandOp::Barrier(texture_barrier(
            targets.depth_texture,
            targets.phase.depth_before(),
            TextureUsageState::DepthStencilAttachment,
        )));
        ops.push(CommandOp::BeginPass {
            pass: targets.pass,
            target: targets.target,
            colors: targets
                .color_attachments
                .iter()
                .map(|attachment| PassAttachment {
                    view: attachment.view,
                    load_op: targets.phase.color_load_op(attachment),
                    store_op: AttachmentStoreOp::Store,
                    clear_color: matches!(
                        targets.phase.color_load_op(attachment),
                        AttachmentLoadOp::Clear
                    )
                    .then(|| {
                        source_color_clear_color(
                            attachment.source_slot,
                            attachment.clear_color_bits,
                            targets.clear_values.fog_color,
                        )
                    }),
                })
                .collect(),
            depth_stencil: Some(PassAttachment {
                view: targets.depth_view,
                load_op: targets.phase.depth_load_op(),
                store_op: AttachmentStoreOp::Store,
                clear_color: None,
            }),
        });
        let mut draw_state = IndexedDrawState::default();
        for draw in draws {
            append_indexed_draw(
                ops,
                &mut draw_state,
                draw.pipeline,
                draw.pipeline_layout,
                draw.resource_set,
                &draw.resource_set_dynamic_offsets,
                Some(draw.shader_resource_set),
                draw.index_buffer,
                draw.index_offset,
                draw.index_type,
                draw.index_count,
                draw.instance_count,
            );
        }
        ops.push(CommandOp::EndPass);
        for attachment in &targets.color_attachments {
            ops.push(CommandOp::Barrier(texture_barrier(
                attachment.texture,
                TextureUsageState::ColorAttachment,
                TextureUsageState::ShaderRead,
            )));
        }
        ops.push(CommandOp::Barrier(texture_barrier(
            targets.depth_texture,
            TextureUsageState::DepthStencilAttachment,
            TextureUsageState::ShaderRead,
        )));
        Ok(())
    }

    fn append_source_material_color_pass(
        &self,
        ops: &mut Vec<CommandOp>,
        targets: &TerrainSourceColorPassTargets,
        draws: &[TexturedMaterialSourceDraw],
        expected_phase: TerrainSourceColorPassPhase,
        writer: &str,
    ) -> GalResult<()> {
        if matches!(
            std::env::var("MATTMC_RUST_SOURCE_DEPTH_TRACE").as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE")
        ) {
            eprintln!(
                "[MattMC source-depth-trace] material-pass phase={:?} depth_texture=0x{:016x} depth_view=0x{:016x} draws={}",
                targets.phase, targets.depth_texture.raw(), targets.depth_view.raw(), draws.len(),
            );
        }
        if targets.phase != expected_phase {
            return Err(GalError::invalid_argument(format!(
                "{writer} source draw requires its explicit source pass phase"
            )));
        }
        if targets.color_attachments.is_empty() {
            return Err(GalError::invalid_argument(format!(
                "{writer} source pass requires at least one named color attachment",
            )));
        }
        let mut seen_slots = std::collections::BTreeSet::new();
        let mut seen_outputs = std::collections::BTreeSet::new();
        for attachment in &targets.color_attachments {
            if !seen_slots.insert(attachment.source_slot) || !seen_outputs.insert(attachment.output)
            {
                return Err(GalError::invalid_argument(format!(
                    "{writer} source pass has duplicate named output attachments",
                )));
            }
            ops.push(CommandOp::Barrier(texture_barrier(
                attachment.texture,
                TextureUsageState::ShaderRead,
                TextureUsageState::ColorAttachment,
            )));
        }
        ops.push(CommandOp::Barrier(texture_barrier(
            targets.depth_texture,
            TextureUsageState::ShaderRead,
            TextureUsageState::DepthStencilAttachment,
        )));
        ops.push(CommandOp::BeginPass {
            pass: targets.pass,
            target: targets.target,
            colors: targets
                .color_attachments
                .iter()
                .map(|attachment| PassAttachment {
                    view: attachment.view,
                    load_op: AttachmentLoadOp::Load,
                    store_op: AttachmentStoreOp::Store,
                    clear_color: None,
                })
                .collect(),
            depth_stencil: Some(PassAttachment {
                view: targets.depth_view,
                load_op: AttachmentLoadOp::Load,
                store_op: AttachmentStoreOp::Store,
                clear_color: None,
            }),
        });
        let mut draw_state = DirectDrawState::default();
        for draw in draws {
            append_direct_draw(
                ops,
                &mut draw_state,
                draw.pipeline,
                draw.pipeline_layout,
                draw.resource_set,
                &draw.resource_set_dynamic_offsets,
                draw.shader_resource_set,
                draw.vertices,
            );
        }
        ops.push(CommandOp::EndPass);
        for attachment in &targets.color_attachments {
            ops.push(CommandOp::Barrier(texture_barrier(
                attachment.texture,
                TextureUsageState::ColorAttachment,
                TextureUsageState::ShaderRead,
            )));
        }
        ops.push(CommandOp::Barrier(texture_barrier(
            targets.depth_texture,
            TextureUsageState::DepthStencilAttachment,
            TextureUsageState::ShaderRead,
        )));
        Ok(())
    }

    /// Records the lowered source shadow programs into their explicit owned
    /// shadow attachments. The caller still owns source-frame ordering,
    /// color-history completion, fullscreen consumers, and route admission.
    /// This reusable boundary is shared by ordinary terrain and later DH
    /// source work without inheriting the rest of the fixture pass graph.
    pub(crate) fn append_terrain_source_shadow_pass(
        &self,
        ops: &mut Vec<CommandOp>,
        targets: TerrainSourceShadowPassTargets,
        draws: &[TerrainMeshDraw],
    ) -> GalResult<()> {
        self.append_shadow_depth_pass(ops, targets, draws, targets.initialized)
    }

    fn validate_terrain_material_graph(&self) -> GalResult<()> {
        let passes = self
            .plan
            .graph
            .passes()
            .iter()
            .map(|pass| pass.identity.as_str())
            .collect::<Vec<_>>();
        let expected = [
            "vulkanic:pass/shadow_depth",
            "vulkanic:pass/terrain_opaque",
            "vulkanic:pass/terrain_cutout",
            "vulkanic:pass/deferred_lighting",
            "vulkanic:pass/terrain_translucent",
            "vulkanic:pass/composite_0",
            "vulkanic:pass/composite_1",
            "vulkanic:pass/final_output",
        ];
        if passes != expected {
            return Err(GalError::invalid_argument(format!(
                "terrain material runtime graph has unexpected pass order: {:?}",
                passes
            )));
        }
        Ok(())
    }

    fn append_shadow_depth_pass(
        &self,
        ops: &mut Vec<CommandOp>,
        targets: TerrainSourceShadowPassTargets,
        draws: &[TerrainMeshDraw],
        targets_initialized: bool,
    ) -> GalResult<()> {
        let pass = self.pass_identity(AttachmentRole::ShadowDepth)?;
        let attachment_before = if targets_initialized {
            TextureUsageState::ShaderRead
        } else {
            TextureUsageState::Undefined
        };
        ops.push(CommandOp::Barrier(texture_barrier(
            targets.shadow_depth_texture,
            attachment_before,
            TextureUsageState::DepthStencilAttachment,
        )));
        for texture in [
            targets.shadow_color_texture,
            targets.shadow_light_shaft_texture,
        ] {
            ops.push(CommandOp::Barrier(texture_barrier(
                texture,
                attachment_before,
                TextureUsageState::ColorAttachment,
            )));
        }
        ops.push(CommandOp::BeginPass {
            pass: targets.shadow_pass,
            target: targets.shadow_target,
            colors: vec![
                PassAttachment {
                    view: targets.shadow_color_view,
                    load_op: AttachmentLoadOp::Clear,
                    store_op: AttachmentStoreOp::Store,
                    clear_color: Some(ClearColor {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 0.0,
                    }),
                },
                PassAttachment {
                    view: targets.shadow_light_shaft_view,
                    load_op: AttachmentLoadOp::Clear,
                    store_op: AttachmentStoreOp::Store,
                    clear_color: Some(ClearColor {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 0.0,
                    }),
                },
            ],
            depth_stencil: Some(PassAttachment {
                view: targets.shadow_depth_view,
                load_op: AttachmentLoadOp::Clear,
                store_op: AttachmentStoreOp::Store,
                clear_color: None,
            }),
        });
        let mut draw_state = IndexedDrawState::default();
        for draw in draws.iter().filter(|draw| {
            draw.material_mode != TerrainMaterialPassMode::Translucent
                && draw.shadow_participation == TerrainShadowParticipation::Required
        }) {
            let shadow = draw.shadow.as_ref().ok_or_else(|| {
                GalError::backend(format!(
                    "{} mesh draw missing shadow pipeline",
                    pass.as_str()
                ))
            })?;
            append_indexed_draw(
                ops,
                &mut draw_state,
                shadow.pipeline,
                shadow.pipeline_layout,
                shadow.resource_set,
                &shadow.resource_set_dynamic_offsets,
                shadow.shader_resource_set,
                draw.index_buffer,
                draw.index_offset,
                draw.index_type,
                draw.index_count,
                draw.instance_count,
            );
        }
        ops.push(CommandOp::EndPass);
        ops.push(CommandOp::Barrier(texture_barrier(
            targets.shadow_depth_texture,
            TextureUsageState::DepthStencilAttachment,
            TextureUsageState::ShaderRead,
        )));
        for texture in [
            targets.shadow_color_texture,
            targets.shadow_light_shaft_texture,
        ] {
            ops.push(CommandOp::Barrier(texture_barrier(
                texture,
                TextureUsageState::ColorAttachment,
                TextureUsageState::ShaderRead,
            )));
        }
        Ok(())
    }

    fn append_g_buffer_passes(
        &self,
        ops: &mut Vec<CommandOp>,
        targets: TerrainRuntimeTargets,
        background_color: ClearColor,
        draws: &[TerrainMeshDraw],
        force_empty_clear: bool,
        targets_initialized: bool,
        background_initialized: bool,
    ) -> GalResult<()> {
        let attachment_before = if targets_initialized || background_initialized {
            TextureUsageState::ShaderRead
        } else {
            TextureUsageState::Undefined
        };
        for texture in [
            targets.albedo_texture,
            targets.normal_texture,
            targets.material_light_texture,
            targets.world_position_texture,
        ] {
            ops.push(CommandOp::Barrier(texture_barrier(
                texture,
                attachment_before,
                TextureUsageState::ColorAttachment,
            )));
        }
        ops.push(CommandOp::Barrier(texture_barrier(
            targets.depth_texture,
            attachment_before,
            TextureUsageState::DepthStencilAttachment,
        )));

        let mut wrote_g_buffer = false;
        for mode in [
            TerrainMaterialPassMode::Opaque,
            TerrainMaterialPassMode::Cutout,
        ] {
            let has_mode_draws = draws.iter().any(|draw| draw.material_mode == mode);
            if !has_mode_draws && !force_empty_clear {
                continue;
            }
            let load_op = if wrote_g_buffer || background_initialized {
                AttachmentLoadOp::Load
            } else {
                AttachmentLoadOp::Clear
            };
            ops.push(CommandOp::BeginPass {
                pass: targets.g_buffer_pass,
                target: targets.target,
                colors: vec![
                    PassAttachment {
                        view: targets.albedo_view,
                        load_op,
                        store_op: AttachmentStoreOp::Store,
                        clear_color: Some(background_color),
                    },
                    PassAttachment {
                        view: targets.normal_view,
                        load_op,
                        store_op: AttachmentStoreOp::Store,
                        clear_color: Some(ClearColor {
                            r: 0.5,
                            g: 0.5,
                            b: 1.0,
                            a: 1.0,
                        }),
                    },
                    PassAttachment {
                        view: targets.material_light_view,
                        load_op,
                        store_op: AttachmentStoreOp::Store,
                        clear_color: Some(ClearColor {
                            r: 0.0,
                            g: 1.0,
                            b: 1.0,
                            a: 0.0,
                        }),
                    },
                    PassAttachment {
                        view: targets.world_position_view,
                        load_op,
                        store_op: AttachmentStoreOp::Store,
                        clear_color: Some(ClearColor {
                            r: 0.5,
                            g: 0.5,
                            b: 0.5,
                            a: 0.0,
                        }),
                    },
                ],
                depth_stencil: Some(PassAttachment {
                    view: targets.depth_view,
                    load_op,
                    store_op: AttachmentStoreOp::Store,
                    clear_color: None,
                }),
            });
            let mut draw_state = IndexedDrawState::default();
            for draw in draws.iter().filter(|draw| draw.material_mode == mode) {
                append_indexed_draw(
                    ops,
                    &mut draw_state,
                    draw.pipeline,
                    draw.pipeline_layout,
                    draw.resource_set,
                    &draw.resource_set_dynamic_offsets,
                    draw.shader_resource_set,
                    draw.index_buffer,
                    draw.index_offset,
                    draw.index_type,
                    draw.index_count,
                    draw.instance_count,
                );
            }
            ops.push(CommandOp::EndPass);
            wrote_g_buffer = true;
        }

        for texture in [
            targets.albedo_texture,
            targets.normal_texture,
            targets.material_light_texture,
            targets.world_position_texture,
        ] {
            ops.push(CommandOp::Barrier(texture_barrier(
                texture,
                TextureUsageState::ColorAttachment,
                TextureUsageState::ShaderRead,
            )));
        }
        ops.push(CommandOp::Barrier(texture_barrier(
            targets.depth_texture,
            TextureUsageState::DepthStencilAttachment,
            TextureUsageState::ShaderRead,
        )));
        Ok(())
    }

    fn append_deferred_and_composites(
        &self,
        ops: &mut Vec<CommandOp>,
        targets: TerrainRuntimeTargets,
        frame: TerrainRuntimeFrame,
        draws: &[TerrainMeshDraw],
        forward_material_draws: &[TerrainForwardMaterialDraw],
    ) -> GalResult<()> {
        let screen_texture_before = screen_texture_before(frame.screen_targets_initialized);
        ops.push(CommandOp::Barrier(buffer_barrier(
            targets.composite_uniform_buffer,
            TextureUsageState::ShaderRead,
            TextureUsageState::TransferDst,
        )));
        ops.push(CommandOp::HostWriteBuffer {
            buffer: targets.composite_uniform_buffer,
            offset: 0,
            data: frame.uniforms.pack(),
        });
        ops.push(CommandOp::Barrier(buffer_barrier(
            targets.composite_uniform_buffer,
            TextureUsageState::TransferDst,
            TextureUsageState::ShaderRead,
        )));

        self.append_screen_pass(
            ops,
            targets.deferred_lit_texture,
            targets.deferred_lighting_pass,
            targets.deferred_lit_target,
            targets.deferred_lit_view,
            targets.deferred_lighting_pipeline,
            targets.deferred_lighting_resource_set,
            targets.screen_pipeline_layout,
            transparent_clear(frame.background_color),
            screen_texture_before,
        );
        self.append_translucent_pass(
            ops,
            targets,
            draws,
            frame.translucent_entity_external,
            frame.translucent_terrain_external,
        )?;
        self.append_forward_material_pass(ops, targets, forward_material_draws)?;
        self.append_screen_pass(
            ops,
            targets.composite0_texture,
            targets.composite0_pass,
            targets.composite0_target,
            targets.composite0_view,
            targets.composite0_pipeline,
            targets.composite0_resource_set,
            targets.screen_pipeline_layout,
            transparent_clear(frame.background_color),
            screen_texture_before,
        );
        self.append_screen_pass(
            ops,
            targets.composite1_texture,
            targets.composite1_pass,
            targets.composite1_target,
            targets.composite1_view,
            targets.composite1_pipeline,
            targets.composite1_resource_set,
            targets.screen_pipeline_layout,
            transparent_clear(frame.background_color),
            screen_texture_before,
        );

        ops.push(CommandOp::BeginPass {
            pass: targets.final_pass,
            target: frame.frame_target,
            colors: vec![PassAttachment {
                view: frame.color_attachment,
                load_op: AttachmentLoadOp::Load,
                store_op: AttachmentStoreOp::Store,
                clear_color: None,
            }],
            // Final fullscreen stages sample the main depth texture through
            // their resource set; they do not perform depth testing. Keeping
            // it attached here creates an illegal sampled+depth-attachment
            // feedback layout on Vulkan.
            depth_stencil: None,
        });
        ops.push(CommandOp::BindGraphicsPipeline(targets.final_pipeline));
        ops.push(CommandOp::BindResourceSet {
            pipeline_layout: targets.screen_pipeline_layout,
            set_index: 0,
            set: targets.final_resource_set,
            dynamic_offsets: Vec::new(),
        });
        ops.push(CommandOp::Draw {
            vertices: 3,
            instances: 1,
        });
        ops.push(CommandOp::EndPass);
        Ok(())
    }

    fn append_translucent_pass(
        &self,
        ops: &mut Vec<CommandOp>,
        targets: TerrainRuntimeTargets,
        draws: &[TerrainMeshDraw],
        translucent_entity_external: bool,
        translucent_terrain_external: bool,
    ) -> GalResult<()> {
        if !draws.iter().any(|draw| {
            draw.material_mode == TerrainMaterialPassMode::Translucent
                && !Self::translucent_draw_is_external(
                    draw,
                    translucent_entity_external,
                    false,
                )
        }) {
            return Ok(());
        }
        if let Some(capture) = targets.translucent_capture {
            ops.push(CommandOp::Barrier(texture_barrier(
                capture.color_texture,
                if targets.translucent_capture_initialized {
                    TextureUsageState::ShaderRead
                } else {
                    TextureUsageState::Undefined
                },
                TextureUsageState::ColorAttachment,
            )));
            ops.push(CommandOp::Barrier(texture_barrier(
                targets.depth_texture,
                TextureUsageState::ShaderRead,
                TextureUsageState::TransferSrc,
            )));
            ops.push(CommandOp::Barrier(texture_barrier(
                capture.depth_texture,
                if targets.translucent_capture_initialized { TextureUsageState::ShaderRead }
                else { TextureUsageState::Undefined },
                TextureUsageState::TransferDst,
            )));
            ops.push(CommandOp::CopyTexture(TextureImageCopyRegion {
                row_order: crate::render::vulkanic::commands::TextureRowOrder::Preserve,
                src_texture: targets.depth_texture, src_mip: 0, src_layer: 0,
                src_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
                dst_texture: capture.depth_texture, dst_mip: 0, dst_layer: 0,
                dst_origin: TextureOrigin3d { x: 0, y: 0, z: 0 }, extent: capture.extent,
            }));
            ops.push(CommandOp::Barrier(texture_barrier(
                targets.depth_texture, TextureUsageState::TransferSrc, TextureUsageState::ShaderRead,
            )));
            ops.push(CommandOp::Barrier(texture_barrier(
                capture.depth_texture, TextureUsageState::TransferDst, TextureUsageState::DepthStencilAttachment,
            )));
            ops.push(CommandOp::BeginPass {
                pass: capture.pass,
                target: capture.target,
                colors: vec![PassAttachment {
                    view: capture.color_view,
                    load_op: AttachmentLoadOp::Clear,
                    store_op: AttachmentStoreOp::Store,
                    clear_color: Some(ClearColor {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 0.0,
                    }),
                }],
                depth_stencil: Some(PassAttachment {
                    view: capture.depth_view,
                    load_op: AttachmentLoadOp::Load,
                    store_op: AttachmentStoreOp::Store,
                    clear_color: None,
                }),
            });
            let mut capture_draw_state = IndexedDrawState::default();
            for draw in draws.iter().filter(|draw| {
                draw.material_mode == TerrainMaterialPassMode::Translucent
                    && !Self::translucent_draw_is_external(
                        draw,
                        translucent_entity_external,
                        false,
                    )
            }) {
                append_indexed_draw(
                    ops,
                    &mut capture_draw_state,
                    draw.pipeline,
                    draw.pipeline_layout,
                    draw.resource_set,
                    &draw.resource_set_dynamic_offsets,
                    draw.shader_resource_set,
                    draw.index_buffer,
                    draw.index_offset,
                    draw.index_type,
                    draw.index_count,
                    draw.instance_count,
                );
            }
            ops.push(CommandOp::EndPass);
            ops.push(CommandOp::Barrier(texture_barrier(
                capture.color_texture,
                TextureUsageState::ColorAttachment,
                TextureUsageState::ShaderRead,
            )));
            ops.push(CommandOp::Barrier(texture_barrier(
                capture.depth_texture,
                TextureUsageState::DepthStencilAttachment,
                TextureUsageState::ShaderRead,
            )));
        }
        // The Fabulous handoff samples the dedicated capture and performs the
        // one alpha composition itself.  Do not open an empty deferred pass
        // after all translucent work was routed there: more importantly, do
        // not let a future draw accidentally reintroduce that second writer.
        if !draws.iter().any(|draw| {
            draw.material_mode == TerrainMaterialPassMode::Translucent
                && !Self::translucent_draw_is_external(
                    draw,
                    translucent_entity_external,
                    translucent_terrain_external,
                )
        }) {
            return Ok(());
        }
        ops.push(CommandOp::Barrier(texture_barrier(
            targets.deferred_lit_texture,
            TextureUsageState::ShaderRead,
            TextureUsageState::ColorAttachment,
        )));
        ops.push(CommandOp::Barrier(texture_barrier(
            targets.depth_texture,
            TextureUsageState::ShaderRead,
            TextureUsageState::DepthStencilAttachment,
        )));
        ops.push(CommandOp::BeginPass {
            pass: targets.translucent_pass,
            target: targets.translucent_target,
            colors: vec![PassAttachment {
                view: targets.deferred_lit_view,
                load_op: AttachmentLoadOp::Load,
                store_op: AttachmentStoreOp::Store,
                clear_color: None,
            }],
            depth_stencil: Some(PassAttachment {
                view: targets.depth_view,
                load_op: AttachmentLoadOp::Load,
                store_op: AttachmentStoreOp::Store,
                clear_color: None,
            }),
        });
        let mut draw_state = IndexedDrawState::default();
        for draw in draws.iter().filter(|draw| {
            draw.material_mode == TerrainMaterialPassMode::Translucent
                && !Self::translucent_draw_is_external(
                    draw,
                    translucent_entity_external,
                    translucent_terrain_external,
                )
        }) {
            append_indexed_draw(
                ops,
                &mut draw_state,
                draw.pipeline,
                draw.pipeline_layout,
                draw.resource_set,
                &draw.resource_set_dynamic_offsets,
                draw.shader_resource_set,
                draw.index_buffer,
                draw.index_offset,
                draw.index_type,
                draw.index_count,
                draw.instance_count,
            );
        }
        ops.push(CommandOp::EndPass);
        ops.push(CommandOp::Barrier(texture_barrier(
            targets.deferred_lit_texture,
            TextureUsageState::ColorAttachment,
            TextureUsageState::ShaderRead,
        )));
        ops.push(CommandOp::Barrier(texture_barrier(
            targets.depth_texture,
            TextureUsageState::DepthStencilAttachment,
            TextureUsageState::ShaderRead,
        )));
        Ok(())
    }

/// Returns whether this translucent draw is owned by the explicit Fabulous
/// attachment graph rather than the normal deferred color target.  The
/// capture pass deliberately keeps terrain available to Fabulous while entity
/// meshes use their separate `item_entity` role; the final deferred pass must
/// omit both external families so neither is composited twice.
    fn translucent_draw_is_external(
    draw: &TerrainMeshDraw,
    translucent_entity_external: bool,
    translucent_terrain_external: bool,
) -> bool {
    (translucent_entity_external && draw.stratum == WORLD_STRATUM_ENTITY_MESH)
        || (translucent_terrain_external && draw.stratum == WORLD_STRATUM_TERRAIN)
}

    /// Direct semantic material quads use their own texture/pipeline contract,
    /// but must be written after deferred terrain lighting and before the
    /// composite chain. Recording them against the acquired target earlier in
    /// the frame lets the final composite overwrite their color.
    fn append_forward_material_pass(
        &self,
        ops: &mut Vec<CommandOp>,
        targets: TerrainRuntimeTargets,
        draws: &[TerrainForwardMaterialDraw],
    ) -> GalResult<()> {
        if draws.is_empty() {
            return Ok(());
        }
        ops.push(CommandOp::Barrier(texture_barrier(
            targets.deferred_lit_texture,
            TextureUsageState::ShaderRead,
            TextureUsageState::ColorAttachment,
        )));
        ops.push(CommandOp::Barrier(texture_barrier(
            targets.depth_texture,
            TextureUsageState::ShaderRead,
            TextureUsageState::DepthStencilAttachment,
        )));
        ops.push(CommandOp::BeginPass {
            pass: targets.translucent_pass,
            target: targets.translucent_target,
            colors: vec![PassAttachment {
                view: targets.deferred_lit_view,
                load_op: AttachmentLoadOp::Load,
                store_op: AttachmentStoreOp::Store,
                clear_color: None,
            }],
            depth_stencil: Some(PassAttachment {
                view: targets.depth_view,
                load_op: AttachmentLoadOp::Load,
                store_op: AttachmentStoreOp::Store,
                clear_color: None,
            }),
        });
        let mut draw_state = IndexedDrawState::default();
        for draw in draws {
            append_indexed_draw(
                ops,
                &mut draw_state,
                draw.pipeline,
                draw.pipeline_layout,
                draw.resource_set,
                &[],
                draw.shader_resource_set,
                draw.index_buffer,
                draw.index_offset,
                draw.index_type,
                draw.index_count,
                draw.instance_count,
            );
        }
        ops.push(CommandOp::EndPass);
        ops.push(CommandOp::Barrier(texture_barrier(
            targets.deferred_lit_texture,
            TextureUsageState::ColorAttachment,
            TextureUsageState::ShaderRead,
        )));
        ops.push(CommandOp::Barrier(texture_barrier(
            targets.depth_texture,
            TextureUsageState::DepthStencilAttachment,
            TextureUsageState::ShaderRead,
        )));
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn append_screen_pass(
        &self,
        ops: &mut Vec<CommandOp>,
        texture: Handle,
        pass: Handle,
        target: Handle,
        color_view: Handle,
        pipeline: Handle,
        resource_set: Handle,
        pipeline_layout: Handle,
        clear_color: ClearColor,
        texture_before: TextureUsageState,
    ) {
        ops.push(CommandOp::Barrier(texture_barrier(
            texture,
            texture_before,
            TextureUsageState::ColorAttachment,
        )));
        ops.push(CommandOp::BeginPass {
            pass,
            target,
            colors: vec![PassAttachment {
                view: color_view,
                load_op: AttachmentLoadOp::Clear,
                store_op: AttachmentStoreOp::Store,
                clear_color: Some(clear_color),
            }],
            depth_stencil: None,
        });
        ops.push(CommandOp::BindGraphicsPipeline(pipeline));
        ops.push(CommandOp::BindResourceSet {
            pipeline_layout,
            set_index: 0,
            set: resource_set,
            dynamic_offsets: Vec::new(),
        });
        ops.push(CommandOp::Draw {
            vertices: 3,
            instances: 1,
        });
        ops.push(CommandOp::EndPass);
        ops.push(CommandOp::Barrier(texture_barrier(
            texture,
            TextureUsageState::ColorAttachment,
            TextureUsageState::ShaderRead,
        )));
    }

    fn pass_identity(&self, role: AttachmentRole) -> GalResult<&PassIdentity> {
        self.plan
            .graph
            .passes()
            .iter()
            .find(|pass| pass.depth == Some(role) || pass.colors.contains(&role))
            .map(|pass| &pass.identity)
            .ok_or_else(|| {
                GalError::invalid_argument(format!("shader runtime graph is missing {role:?} pass"))
            })
    }
}

fn screen_texture_before(initialized: bool) -> TextureUsageState {
    if initialized {
        TextureUsageState::ShaderRead
    } else {
        TextureUsageState::Undefined
    }
}

fn write_contract_diagnostic(plan: &ShaderPackRuntimePlan) {
    let Some(dir) = std::env::var_os("MATTMC_TERRAIN_PASS_CONTRACT_DIAGNOSTIC_DIR") else {
        return;
    };
    let dir = Path::new(&dir);
    if fs::create_dir_all(dir).is_err() {
        return;
    }
    let _ = fs::write(
        dir.join(format!(
            "terrain-pass-contract-generation-{}.json",
            plan.generation
        )),
        format!("{}\n", plan.terrain_contract_diagnostic_json()),
    );
}

impl TerrainCompositeUniforms {
    pub(crate) fn pack(self) -> Vec<u8> {
        let mut out = Vec::with_capacity(TERRAIN_RUNTIME_COMPOSITE_UNIFORM_BYTES as usize);
        for value in self.light_view_projection {
            push_f32(&mut out, value);
        }
        for value in self.shadow_params {
            push_f32(&mut out, value);
        }
        for value in self.color_grade_params {
            push_f32(&mut out, value);
        }
        for value in self.projection_inverse {
            push_f32(&mut out, value);
        }
        for value in self.fog_color_and_environmental_start {
            push_f32(&mut out, value);
        }
        for value in self.fog_ranges {
            push_f32(&mut out, value);
        }
        out
    }
}

#[derive(Default)]
struct IndexedDrawState {
    pipeline: Option<Handle>,
    resource_set: Option<(Handle, u32, Handle, Vec<u64>)>,
    shader_resource_set: Option<(Handle, u32, Handle)>,
    index_buffer: Option<(Handle, u64, IndexType)>,
}

/// Shared explicit binding cache for direct source-material draws. It stays
/// separate from indexed terrain state so a future source writer cannot leave
/// an index-buffer assumption attached to its pass.
#[derive(Default)]
struct DirectDrawState {
    pipeline: Option<Handle>,
    resource_set: Option<(Handle, u32, Handle, Vec<u64>)>,
    shader_resource_set: Option<(Handle, u32, Handle)>,
}

#[allow(clippy::too_many_arguments)]
fn append_direct_draw(
    ops: &mut Vec<CommandOp>,
    state: &mut DirectDrawState,
    pipeline: Handle,
    pipeline_layout: Handle,
    resource_set: Handle,
    resource_set_dynamic_offsets: &[u64],
    shader_resource_set: Option<TerrainShaderResourceSet>,
    vertices: u32,
) {
    if state.pipeline != Some(pipeline) {
        ops.push(CommandOp::BindGraphicsPipeline(pipeline));
        state.pipeline = Some(pipeline);
        state.resource_set = None;
        state.shader_resource_set = None;
    }
    let resource_set_binding = (
        pipeline_layout,
        0,
        resource_set,
        resource_set_dynamic_offsets.to_vec(),
    );
    if state.resource_set.as_ref() != Some(&resource_set_binding) {
        ops.push(CommandOp::BindResourceSet {
            pipeline_layout,
            set_index: 0,
            set: resource_set,
            dynamic_offsets: resource_set_dynamic_offsets.to_vec(),
        });
        state.resource_set = Some(resource_set_binding);
    }
    let shader_resource_set_binding =
        shader_resource_set.map(|binding| (pipeline_layout, binding.set_index, binding.set));
    if state.shader_resource_set != shader_resource_set_binding {
        if let Some((pipeline_layout, set_index, set)) = shader_resource_set_binding {
            ops.push(CommandOp::BindResourceSet {
                pipeline_layout,
                set_index,
                set,
                dynamic_offsets: Vec::new(),
            });
        }
        state.shader_resource_set = shader_resource_set_binding;
    }
    ops.push(CommandOp::Draw {
        vertices,
        instances: 1,
    });
}

#[allow(clippy::too_many_arguments)]
fn append_indexed_draw(
    ops: &mut Vec<CommandOp>,
    state: &mut IndexedDrawState,
    pipeline: Handle,
    pipeline_layout: Handle,
    resource_set: Handle,
    resource_set_dynamic_offsets: &[u64],
    shader_resource_set: Option<TerrainShaderResourceSet>,
    index_buffer: Handle,
    index_offset: u64,
    index_type: IndexType,
    index_count: u32,
    instance_count: u32,
) {
    if state.pipeline != Some(pipeline) {
        ops.push(CommandOp::BindGraphicsPipeline(pipeline));
        state.pipeline = Some(pipeline);
        state.resource_set = None;
        state.shader_resource_set = None;
    }
    let resource_set_binding = (
        pipeline_layout,
        0,
        resource_set,
        resource_set_dynamic_offsets.to_vec(),
    );
    if state.resource_set.as_ref() != Some(&resource_set_binding) {
        ops.push(CommandOp::BindResourceSet {
            pipeline_layout,
            set_index: 0,
            set: resource_set,
            dynamic_offsets: resource_set_dynamic_offsets.to_vec(),
        });
        state.resource_set = Some(resource_set_binding);
    }
    let shader_resource_set_binding =
        shader_resource_set.map(|binding| (pipeline_layout, binding.set_index, binding.set));
    if state.shader_resource_set != shader_resource_set_binding {
        if let Some((pipeline_layout, set_index, set)) = shader_resource_set_binding {
            ops.push(CommandOp::BindResourceSet {
                pipeline_layout,
                set_index,
                set,
                dynamic_offsets: Vec::new(),
            });
        }
        state.shader_resource_set = shader_resource_set_binding;
    }
    let index_binding = (index_buffer, index_offset, index_type);
    if state.index_buffer != Some(index_binding) {
        ops.push(CommandOp::SetIndexBuffer {
            buffer: index_buffer,
            offset: index_offset,
            index_type,
        });
        state.index_buffer = Some(index_binding);
    }
    ops.push(CommandOp::DrawIndexed {
        indices: index_count,
        instances: instance_count,
    });
}

fn buffer_barrier(
    resource: Handle,
    before: TextureUsageState,
    after: TextureUsageState,
) -> ResourceBarrier {
    ResourceBarrier {
        resource,
        subresources: None,
        before,
        after,
        src_queue: QueueClass::Graphics,
        dst_queue: QueueClass::Graphics,
    }
}

fn texture_barrier(
    resource: Handle,
    before: TextureUsageState,
    after: TextureUsageState,
) -> ResourceBarrier {
    ResourceBarrier {
        resource,
        subresources: None,
        before,
        after,
        src_queue: QueueClass::Graphics,
        dst_queue: QueueClass::Graphics,
    }
}

fn transparent_clear(color: ClearColor) -> ClearColor {
    ClearColor {
        r: color.r,
        g: color.g,
        b: color.b,
        a: 0.0,
    }
}

fn push_f32(out: &mut Vec<u8>, value: f32) {
    out.extend_from_slice(&value.to_ne_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::vulkanic::backends::{
        mock::MockBackend, presentation_capabilities, vulkan_capabilities,
    };
    use crate::render::vulkanic::commands::{CommandList, CommandListDesc, SubmissionBatch};
    use crate::render::vulkanic::handles::HandleKind;
    use crate::render::vulkanic::resources::{
        Extent3d, TextureDesc, TextureDimension, TextureFormat, TextureUsage, TextureViewDesc,
    };
    use crate::render::vulkanic::shader_pack::assets::{
        ShaderPackAssetFile, ShaderPackAssetUpdate,
    };
    use crate::render::vulkanic::shader_pack::preprocess::complete_bundled_pack_source_for_test;
    use crate::render::vulkanic::shader_pack::source::ShaderSourceFile;
    use crate::render::vulkanic::shader_pack::terrain_source_resources::{
        TerrainSourceOwnedResourceSet, TerrainSourceResourceAvailabilitySet,
        TerrainSourceResourceRole, TERRAIN_RESOURCE_BINDINGS_PATH,
    };
    use std::path::PathBuf;

    fn gal() -> VulkanicGal {
        VulkanicGal::new_with_backend(
            Box::new(MockBackend::with_capabilities(presentation_capabilities(
                vulkan_capabilities(),
            ))),
            false,
        )
    }

    #[test]
    fn persistent_screen_targets_use_shader_read_after_first_frame() {
        assert_eq!(TextureUsageState::Undefined, screen_texture_before(false));
        assert_eq!(TextureUsageState::ShaderRead, screen_texture_before(true));
    }

    #[test]
    fn fabulous_translucency_handoff_excludes_each_external_family_from_deferred_blending() {
        let draw = |stratum| TerrainMeshDraw {
            shadow: None,
            pipeline: Handle::NULL,
            pipeline_layout: Handle::NULL,
            resource_set: Handle::NULL,
            resource_set_dynamic_offsets: Vec::new(),
            shader_resource_set: None,
            index_buffer: Handle::NULL,
            index_offset: 0,
            index_type: IndexType::U32,
            index_count: 3,
            instance_count: 1,
            stratum,
            material_mode: TerrainMaterialPassMode::Translucent,
            shadow_participation: TerrainShadowParticipation::Unavailable,
        };
        let terrain = draw(WORLD_STRATUM_TERRAIN);
        let entity = draw(WORLD_STRATUM_ENTITY_MESH);

        assert!(ShaderPackRuntimeExecutor::translucent_draw_is_external(
            &terrain, false, true,
        ));
        assert!(ShaderPackRuntimeExecutor::translucent_draw_is_external(
            &entity, true, false,
        ));
        assert!(!ShaderPackRuntimeExecutor::translucent_draw_is_external(
            &terrain, false, false,
        ));
        assert!(!ShaderPackRuntimeExecutor::translucent_draw_is_external(
            &entity, false, true,
        ));
    }

    fn copied_png_assets_for(source: &ShaderPackSource) -> ShaderPackAssets {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../resources/shaders/ComplementaryHungLoIfied/shaders");
        let bindings = TerrainShaderPackAssetBindings::from_source(source).unwrap();
        let mut files = Vec::new();
        for (_, path) in bindings.samplers() {
            files.push(ShaderPackAssetFile::new(
                path,
                fs::read(root.join(path)).unwrap(),
            ));
            let sidecar = format!("{path}.mcmeta");
            if root.join(&sidecar).is_file() {
                files.push(ShaderPackAssetFile::new(
                    &sidecar,
                    fs::read(root.join(&sidecar)).unwrap(),
                ));
            }
        }
        ShaderPackAssets::new(ShaderPackAssetUpdate {
            pack_name: source.name().to_string(),
            generation: source.generation(),
            files,
        })
        .unwrap()
    }
    use crate::render::vulkanic::shader_pack::terrain_contract::bundled_complementary_hung_loified_source;

    #[test]
    fn runtime_executor_owns_expected_gameplay_pass_order() {
        let executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(9).unwrap();
        let passes = executor
            .plan()
            .graph
            .passes()
            .iter()
            .map(|pass| pass.identity.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            passes,
            vec![
                "vulkanic:pass/shadow_depth",
                "vulkanic:pass/terrain_opaque",
                "vulkanic:pass/terrain_cutout",
                "vulkanic:pass/deferred_lighting",
                "vulkanic:pass/terrain_translucent",
                "vulkanic:pass/composite_0",
                "vulkanic:pass/composite_1",
                "vulkanic:pass/final_output",
            ]
        );
        assert!(executor.validate_terrain_material_graph().is_ok());
    }

    #[test]
    fn runtime_owns_only_generation_coherent_vanilla_lightmap_bytes() {
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(9).unwrap();
        let frame = VanillaLightmapFrame {
            generation: 4,
            inputs: super::super::lightmap::VanillaLightmapInputs {
                ambient_light_factor: 0.0,
                sky_factor: 1.0,
                block_factor: 1.5,
                night_vision_factor: 0.0,
                darkness_scale: 0.0,
                darken_world_factor: 0.0,
                brightness_factor: 0.0,
                sky_light_color: [1.0; 3],
                ambient_color: [1.0; 3],
            },
        };
        assert_eq!(
            Some(VanillaLightmapCacheUpdate::Replaced),
            executor.observe_vanilla_lightmap(3, Some(frame)).unwrap()
        );
        assert_eq!(3, executor.vanilla_lightmap_cache().world_generation());
        assert_eq!(4, executor.vanilla_lightmap_cache().lightmap_generation());
        assert_eq!(
            16 * 16 * 4,
            executor.vanilla_lightmap_cache().rgba8().len(),
            "the runtime stores copied semantic image bytes, not a Java texture object"
        );
        assert_eq!(
            Some(VanillaLightmapCacheUpdate::Unchanged),
            executor.observe_vanilla_lightmap(3, Some(frame)).unwrap()
        );
        assert!(executor.observe_vanilla_lightmap(0, Some(frame)).is_err());
    }

    #[test]
    fn runtime_stages_and_confirms_lightmap_residency_only_with_submission_success() {
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(9).unwrap();
        let frame = VanillaLightmapFrame {
            generation: 4,
            inputs: super::super::lightmap::VanillaLightmapInputs {
                ambient_light_factor: 0.0,
                sky_factor: 1.0,
                block_factor: 1.5,
                night_vision_factor: 0.0,
                darkness_scale: 0.0,
                darken_world_factor: 0.0,
                brightness_factor: 0.0,
                sky_light_color: [1.0; 3],
                ambient_color: [1.0; 3],
            },
        };
        executor.observe_vanilla_lightmap(3, Some(frame)).unwrap();
        let mut gal = gal();
        let mut ops = Vec::new();
        assert!(executor
            .stage_vanilla_lightmap_residency(&mut gal, &mut ops)
            .unwrap());
        assert!(executor.has_pending_vanilla_lightmap_submission());
        assert!(executor.vanilla_lightmap_residency.is_none());
        assert!(executor.vanilla_lightmap_binding(false).is_none());
        let pending_binding = executor.vanilla_lightmap_binding(true).unwrap();
        assert_eq!(3, pending_binding.world_generation);
        assert_eq!(4, pending_binding.lightmap_generation);
        assert!(!pending_binding.texture_view.is_null());
        assert!(!pending_binding.sampler.is_null());
        let mut same_submission_ops = Vec::new();
        assert!(
            !executor
                .stage_vanilla_lightmap_residency(&mut gal, &mut same_submission_ops)
                .unwrap(),
            "a second consumer in the same combined submission reuses the pending residency"
        );
        assert!(same_submission_ops.is_empty());
        gal.submit(SubmissionBatch {
            label: "test.runtime-lightmap".to_string(),
            command_lists: vec![CommandList::from(CommandListDesc {
                label: "test.runtime-lightmap.commands".to_string(),
                operations: ops,
            })],
        })
        .unwrap();
        executor
            .confirm_vanilla_lightmap_submission(&mut gal)
            .unwrap();
        assert!(!executor.has_pending_vanilla_lightmap_submission());
        assert!(executor.vanilla_lightmap_residency.is_some());
        assert_eq!(
            Some(pending_binding),
            executor.vanilla_lightmap_binding(false),
            "confirmation preserves the same Rust-owned semantic lightmap binding"
        );

        let mut duplicate_ops = Vec::new();
        assert!(!executor
            .stage_vanilla_lightmap_residency(&mut gal, &mut duplicate_ops)
            .unwrap());
        assert!(duplicate_ops.is_empty());

        executor
            .observe_vanilla_lightmap(
                3,
                Some(VanillaLightmapFrame {
                    generation: 5,
                    ..frame
                }),
            )
            .unwrap();
        let mut replacement_ops = Vec::new();
        assert!(executor
            .stage_vanilla_lightmap_residency(&mut gal, &mut replacement_ops)
            .unwrap());
        executor.discard_vanilla_lightmap_submission(&mut gal);
        assert!(!executor.has_pending_vanilla_lightmap_submission());
        assert!(executor.vanilla_lightmap_residency.is_some());
    }

    #[test]
    fn lightmap_consumer_sets_retire_before_a_replaced_residency_view() {
        use crate::render::vulkanic::resources::{
            PipelineStageFlags, ResourceBindingDesc, ResourceBindingKind, ResourceLayoutDesc,
        };

        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(9).unwrap();
        let frame = VanillaLightmapFrame {
            generation: 1,
            inputs: super::super::lightmap::VanillaLightmapInputs {
                ambient_light_factor: 0.0,
                sky_factor: 1.0,
                block_factor: 1.0,
                night_vision_factor: 0.0,
                darkness_scale: 0.0,
                darken_world_factor: 0.0,
                brightness_factor: 0.0,
                sky_light_color: [1.0; 3],
                ambient_color: [1.0; 3],
            },
        };
        executor.observe_vanilla_lightmap(1, Some(frame)).unwrap();
        let mut gal = gal();
        let layout = gal
            .create_resource_layout(ResourceLayoutDesc {
                label: "test.lightmap.consumer.layout".to_owned(),
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
            })
            .unwrap();
        let mut ops = Vec::new();
        executor
            .stage_vanilla_lightmap_residency(&mut gal, &mut ops)
            .unwrap();
        let first = executor
            .vanilla_lightmap_resource_set(&mut gal, layout, true)
            .unwrap()
            .unwrap();
        assert_eq!(1, first.lightmap_generation);
        gal.submit(SubmissionBatch {
            label: "test.lightmap.consumer".to_owned(),
            command_lists: vec![CommandList::from(CommandListDesc {
                label: "test.lightmap.consumer.commands".to_owned(),
                operations: ops,
            })],
        })
        .unwrap();
        executor
            .confirm_vanilla_lightmap_submission(&mut gal)
            .unwrap();
        executor
            .observe_vanilla_lightmap(
                1,
                Some(VanillaLightmapFrame {
                    generation: 2,
                    ..frame
                }),
            )
            .unwrap();
        let mut replacement_ops = Vec::new();
        executor
            .stage_vanilla_lightmap_residency(&mut gal, &mut replacement_ops)
            .unwrap();
        executor
            .vanilla_lightmap_resource_set(&mut gal, layout, true)
            .unwrap();
        gal.submit(SubmissionBatch {
            label: "test.lightmap.consumer.replace".to_owned(),
            command_lists: vec![CommandList::from(CommandListDesc {
                label: "test.lightmap.consumer.replace.commands".to_owned(),
                operations: replacement_ops,
            })],
        })
        .unwrap();
        executor
            .confirm_vanilla_lightmap_submission(&mut gal)
            .unwrap();
    }

    #[test]
    fn owned_source_generation_is_discovered_without_admitting_source_execution() {
        let source = bundled_complementary_hung_loified_source(11).unwrap();
        let executor =
            ShaderPackRuntimeExecutor::terrain_material_fixture_from_source(&source).unwrap();
        let contract = executor.plan().terrain_contract.as_ref().unwrap();
        assert_eq!(source.generation(), contract.generation);
        assert_eq!(source.name(), contract.pack_name);
        assert_eq!(
            "vulkanic:builtin/terrain_opaque_v1",
            executor.plan().programs.terrain_opaque.identity.as_str()
        );
        assert!(executor
            .plan()
            .terrain_contract_diagnostic_json()
            .contains("\"selected_source_plan_prepared\":false"));
    }

    #[test]
    fn owned_source_candidate_is_observed_without_replacing_the_fixture_plan() {
        let source = bundled_complementary_hung_loified_source(13).unwrap();
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();
        let fixture_program = executor.plan().programs.terrain_opaque.identity.clone();

        executor.observe_source_candidate(&source);

        let candidate = executor.source_candidate();
        assert!(
            matches!(
                candidate,
                TerrainSourceCandidateState::Discovered {
                    generation: 13,
                    requires_colored_voxel_light: true,
                    ..
                }
            ),
            "{candidate:?}"
        );
        assert!(executor
            .candidate_shader_binding(0)
            .unwrap_err()
            .to_string()
            .contains("complete owned colored voxel-light volume"));
        assert!(matches!(
            candidate,
            TerrainSourceCandidateState::Discovered {
                source_shadow_summary: None,
                source_shadow_preprocess_error: Some(error),
                source_shadow_output_count: None,
                source_shadow_lowering_error: None,
                ..
            } if error.contains("missing shadow source for Default")
        ));
        assert_eq!(
            fixture_program,
            executor.plan().programs.terrain_opaque.identity
        );
        assert_eq!(
            Some("lib/textures/noise.png"),
            executor
                .source_asset_binding_plan()
                .and_then(|bindings| bindings.sampler_path("noisetex"))
        );
        let (translucent_discovered, translucent_reason, unsupported_count) =
            executor.source_candidate_translucent_diagnostic();
        assert!(!translucent_discovered);
        assert!(unsupported_count.is_none());
        assert!(translucent_reason
            .expect("the curated normal-only fixture must identify its missing translucent stage")
            .contains("missing translucent terrain fragment source"));
    }

    #[test]
    fn material_texture_wrapper_rejects_a_non_material_semantic_role_before_handle_use() {
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();
        let mut gal = gal();
        let error = executor
            .ensure_candidate_source_material_texture_resources(
                &mut gal,
                TerrainSourceMaterialTextureInput {
                    role: TerrainSourceResourceRole::Lightmap,
                    shader_pack_generation: 1,
                    world_generation: 1,
                    mesh_asset_generation: 1,
                    texture_view: Handle::NULL,
                    sampler: Handle::NULL,
                },
            )
            .unwrap_err();
        assert!(error.to_string().contains("unsupported semantic role"));
    }

    #[test]
    fn active_normal_map_binding_owns_a_distinct_semantic_material_wrapper() {
        let source = ShaderPackSource::new(
            "normal-map-resource",
            29,
            vec![
                ShaderSourceFile::new(
                    "gbuffers_terrain.vsh",
                    "#version 130\nout vec2 texCoord;\nout vec4 glColor;\nout float smoothnessD;\nout float materialMask;\nout float skyLightFactor;\nuniform sampler2D tex;\nvoid main() { texCoord = gl_MultiTexCoord0.xy; glColor = vec4(1.0); smoothnessD = 0.0; materialMask = 0.0; skyLightFactor = 1.0; gl_Position = ftransform(); }",
                ),
                ShaderSourceFile::new(
                    "gbuffers_terrain.fsh",
                    "#version 130\nin vec2 texCoord;\nin vec4 glColor;\nin float smoothnessD;\nin float materialMask;\nin float skyLightFactor;\nuniform sampler2D tex;\nuniform sampler2D normals;\nvoid DoLighting() {}\n/* DRAWBUFFERS:06 */\nvoid main() { vec4 color = texture2D(tex, texCoord); color.rgb += texture2D(normals, texCoord).rgb * 0.0; if (color.a <= 0.00001) discard; color.rgb *= glColor.rgb; DoLighting(); gl_FragData[0] = color; gl_FragData[1] = vec4(smoothnessD, materialMask, skyLightFactor, 1.0); }",
                ),
                ShaderSourceFile::new(
                    TERRAIN_RESOURCE_BINDINGS_PATH,
                    "tex=material_atlas\nnormals=material_normal_map\n",
                ),
                ShaderSourceFile::new("lib/common.glsl", "#define TEST 1\n"),
                ShaderSourceFile::new("shaders.properties", ""),
                ShaderSourceFile::new("block.properties", ""),
            ],
        )
        .unwrap();
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();
        let mut gal = gal();
        executor.observe_source_candidate(&source);
        assert!(executor
            .candidate_source_requires_resource(TerrainSourceResourceRole::MaterialNormalMap));

        let texture = gal
            .create_texture(TextureDesc {
                label: "normal-map-resource.texture".to_string(),
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba8Unorm,
                extent: Extent3d {
                    width: 4,
                    height: 4,
                    depth: 1,
                },
                mip_levels: 1,
                array_layers: 1,
                usages: vec![TextureUsage::Sampled, TextureUsage::TransferDst],
            })
            .unwrap();
        let view = gal
            .create_texture_view(TextureViewDesc {
                label: "normal-map-resource.view".to_string(),
                texture,
                format: TextureFormat::Rgba8Unorm,
                base_mip: 0,
                mip_count: 1,
                base_layer: 0,
                layer_count: 1,
            })
            .unwrap();
        let sampler = gal
            .create_sampler(SamplerDesc {
                label: "normal-map-resource.sampler".to_string(),
                min_filter: SamplerFilter::Nearest,
                mag_filter: SamplerFilter::Nearest,
                mip_filter: SamplerFilter::Nearest,
                address_u: SamplerAddressMode::ClampToEdge,
                address_v: SamplerAddressMode::ClampToEdge,
                address_w: SamplerAddressMode::ClampToEdge,
                comparison: None,
            })
            .unwrap();
        let resources = executor
            .ensure_candidate_source_material_texture_resources(
                &mut gal,
                TerrainSourceMaterialTextureInput {
                    role: TerrainSourceResourceRole::MaterialNormalMap,
                    shader_pack_generation: source.generation(),
                    world_generation: 3,
                    mesh_asset_generation: 5,
                    texture_view: view,
                    sampler,
                },
            )
            .unwrap()
            .expect("the active normal-map source role must receive a wrapper");
        assert!(resources
            .combined_sampler_for(TerrainSourceResourceRole::MaterialNormalMap)
            .is_some());
        executor
            .clear_candidate_source_material_texture_resources(&mut gal)
            .unwrap();
        gal.destroy(sampler).unwrap();
        gal.destroy(view).unwrap();
        gal.destroy(texture).unwrap();
    }

    #[test]
    fn complete_source_discovery_records_paired_lowering_provenance_without_admission() {
        let source = complete_bundled_pack_source_for_test();
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();

        executor.observe_source_candidate_for_scope(&source, TerrainProgramScope::Overworld);

        let contract = match executor.source_candidate() {
            TerrainSourceCandidateState::Discovered { contract, .. } => contract,
            candidate => panic!("expected a discovered puddle source candidate, got {candidate:?}"),
        };
        assert!(
            contract.require_selected_subset().is_ok(),
            "the owned puddle resource must replace only the obsolete feature gate"
        );

        assert_eq!(
            ("prepared", None),
            executor.candidate_textured_material_source_diagnostic(),
            "the bundled scoped gbuffers_textured pair must be retained before a future named-target writer may be implemented"
        );
        assert_eq!(
            ("prepared", None),
            executor.candidate_entity_source_diagnostic(),
            "gbuffers_entities readiness must reflect the owned entity stream and named writer without selecting a route by itself"
        );
        assert_eq!(
            ("prepared", None),
            executor.candidate_hand_source_diagnostic(),
            "gbuffers_hand must be source-lowered and resource-resolved without accidentally selecting the Java/Iris hand route"
        );
        let hand_program = executor
            .prepared_lowered_hand_source_program()
            .unwrap()
            .expect("the complete bundled hand contract must prepare without selecting a route");
        assert!(hand_program.identity.as_str().contains("hand_source_gen"));
        assert_eq!(
            vec![
                (TerrainPassOutput::LitTerrainColor, 0),
                (TerrainPassOutput::MaterialAuxiliary, 6),
                (TerrainPassOutput::ViewSpaceNormal, 5),
            ],
            hand_program.named_output_color_slots(),
            "gbuffers_hand DRAWBUFFERS:06 must retain named source output meanings"
        );
        let mut gal = gal();
        let hand_targets = executor
            .stage_source_color_targets(
                &mut gal,
                73,
                Extent3d {
                    width: 320,
                    height: 180,
                    depth: 1,
                },
            )
            .unwrap()
            .expect("the bundled hand contract requires Rust-owned named color targets");
        let hand_outputs = executor
            .resolve_hand_source_color_outputs(&hand_program, &hand_targets)
            .unwrap();
        let hand_color_resources = executor
            .stage_terrain_source_color_resources_for_hand(&mut gal, &hand_program, &hand_targets)
            .unwrap();
        assert!(
            hand_color_resources
                .combined_sampler_for(TerrainSourceResourceRole::MaterialTexture)
                .is_none(),
            "hand named-color staging must not manufacture a local material texture; the future writer owns that separate Rust resource"
        );
        assert_eq!(
            hand_program.named_output_color_slots(),
            hand_outputs
                .iter()
                .map(|attachment| (attachment.output, attachment.source_slot))
                .collect::<Vec<_>>(),
            "hand output slots must resolve through named Rust-owned targets rather than Iris attachments"
        );
        assert_eq!(
            AttachmentLoadOp::Load,
            TerrainSourceColorPassPhase::Hands.color_load_op(&hand_outputs[0]),
            "first-person output must compose over the completed world color"
        );
        assert_eq!(
            AttachmentLoadOp::Clear,
            TerrainSourceColorPassPhase::Hands.depth_load_op(),
            "first-person output must begin a fresh Rust-owned depth domain"
        );
        assert_eq!(
            TextureUsageState::Undefined,
            TerrainSourceColorPassPhase::Hands.depth_before(),
            "the private first-person depth image must transition explicitly from its initial state"
        );
        let entity_program = executor
            .prepared_lowered_entity_source_program()
            .unwrap()
            .expect("the complete bundled entity contract must prepare without admitting a draw");
        assert!(entity_program
            .identity
            .as_str()
            .contains("entity_source_gen"));
        assert_eq!(
            Some(TerrainSourceResourceRole::MaterialTexture),
            entity_program.opaque_resource_bindings.role_for("tex"),
            "the selected entity pass must retain its local-material sampler role rather than terrain atlas semantics"
        );
        assert_eq!(
            ("prepared", None),
            executor.candidate_weather_source_diagnostic(),
            "the scoped weather stage must remain independently prepared without enabling a draw"
        );
        let (cloud_state, cloud_reason) = executor.candidate_cloud_source_diagnostic();
        assert_eq!("suppressed", cloud_state);
        assert!(
            cloud_reason.is_none(),
            "the bundled inactive cloud branch must report its source-declared suppression without inventing a failed writer: {cloud_reason:?}"
        );
        let textured_program = executor
            .prepared_lowered_textured_material_source_program()
            .unwrap()
            .expect("the complete bundled textured contract must prepare without admitting a draw");
        assert_eq!(
            vec![
                (TerrainPassOutput::LitTerrainColor, 0),
                (TerrainPassOutput::MaterialAuxiliary, 6),
                (TerrainPassOutput::TranslucencyAuxiliary, 3),
            ],
            textured_program.named_output_color_slots(),
            "gbuffers_textured DRAWBUFFERS:063 must retain its material and translucency meanings"
        );
        let weather_program = executor
            .prepared_lowered_weather_source_program()
            .unwrap()
            .expect("complete weather contract must prepare without admitting a draw");
        assert!(weather_program
            .identity
            .as_str()
            .contains("weather_source_gen"));
        assert_eq!(0, weather_program.lit_color_output_slot);
        assert_eq!(0.1, weather_program.alpha_discard_threshold());
        assert_eq!(
            super::super::weather_contract::WeatherBlend::SourceAlphaOver,
            weather_program.blend
        );

        assert!(
            matches!(
                executor.source_candidate(),
                TerrainSourceCandidateState::Discovered {
                    textured_material_contract: Some(material_contract),
                    textured_material_contract_error: None,
                    textured_material_lowered_pair: Some(_),
                    textured_material_source_resource_binding_count: Some(count),
                    textured_material_source_resource_binding_error: None,
                    textured_material_source_resource_bindings: Some(_),
                    entity_contract: Some(entity_contract),
                    entity_contract_error: None,
                    entity_lowered_pair: Some(_),
                    entity_source_resource_binding_count: Some(entity_count),
                    entity_source_resource_binding_error: None,
                    entity_source_resource_bindings: Some(_),
                    weather_contract: Some(weather_contract),
                    weather_contract_error: None,
                    weather_lowered_pair: Some(_),
                    weather_source_resource_binding_count: Some(weather_count),
                    weather_source_resource_binding_error: None,
                    weather_source_resource_bindings: Some(_),
                    source_summary: Some(_),
                    source_preprocess_error: None,
                    source_shadow_summary: Some(shadow_summary),
                    source_shadow_preprocess_error: None,
                    source_shadow_output_count: Some(2),
                    source_shadow_lowering_error: None,
                    source_lowering_summary: Some(summary),
                    source_lowering_error: None,
                    source_resource_binding_count: Some(8),
                    source_resource_binding_error: None,
                    ..
                } if summary.varying_count > 0
                    && summary.opaque_resource_count > summary.active_opaque_resource_count
                    && summary.active_opaque_resource_count > 0
                    && *count > 0
                    && *weather_count > 0
                    && *entity_count > 0
                    && material_contract.scope == TerrainProgramScope::Overworld
                    && entity_contract.scope == TerrainProgramScope::Overworld
                    && weather_contract.scope == TerrainProgramScope::Overworld
                    && shadow_summary.vertex_entry == "world0/shadow.vsh"
                    && shadow_summary.fragment_entry == "world0/shadow.fsh"
            ),
            "{:#?}",
            executor.source_candidate()
        );
        let entity_base_color_role = match executor.source_candidate() {
            TerrainSourceCandidateState::Discovered {
                entity_source_resource_bindings: Some(bindings),
                ..
            } => bindings.role_for("tex"),
            candidate => panic!("expected discovered entity binding plan, got {candidate:?}"),
        };
        assert_eq!(
            Some(TerrainSourceResourceRole::MaterialTexture),
            entity_base_color_role,
            "gbuffers_entities must retain its local material sampler semantics instead of inheriting terrain atlas ownership"
        );
        assert_eq!(
            executor.plan().programs.terrain_opaque.identity.as_str(),
            "vulkanic:builtin/terrain_opaque_v1"
        );
        let shadow = executor
            .prepared_lowered_shadow_source_program()
            .unwrap()
            .expect("complete scoped shadow source must remain available for a later owned pass");
        assert!(shadow.identity.as_str().contains("shadow_source_gen"));
        assert!(shadow.fragment.source.contains("out_shadow_color"));
        assert!(!shadow.fragment.source.contains("out_terrain_lit_color"));
        assert!(!shadow.vertex.source.contains("puddle_img"));
        assert!(!shadow.vertex.source.contains("imageStore"));
    }

    #[test]
    fn selected_vanilla_cloud_style_prepares_without_admitting_a_cloud_route() {
        let source = complete_bundled_pack_source_for_test();
        let mut files = source.files();
        files.push(ShaderSourceFile::new(
            crate::render::vulkanic::shader_pack::source::RUNTIME_OPTIONS_PATH,
            "CLOUD_STYLE_DEFINE=50\n",
        ));
        let source =
            ShaderPackSource::new("bundled-vanilla-cloud-style", source.generation(), files)
                .unwrap();
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();

        executor.observe_source_candidate_for_scope(&source, TerrainProgramScope::Overworld);

        assert_eq!(
            ("prepared", None),
            executor.candidate_cloud_source_diagnostic(),
            "a selected vanilla cloud branch must have paired source lowering and semantic resource bindings before any writer is staged"
        );
        let cloud = executor
            .prepared_lowered_cloud_source_program()
            .unwrap()
            .expect("prepared cloud source must not require route admission");
        assert!(cloud.identity.as_str().contains("cloud_source_gen"));
        assert_eq!(
            vec![
                (TerrainPassOutput::LitTerrainColor, 0),
                (TerrainPassOutput::MaterialAuxiliary, 6),
                (TerrainPassOutput::TranslucencyAuxiliary, 3),
            ],
            cloud.named_output_color_slots()
        );
    }

    #[test]
    fn entity_source_color_outputs_resolve_through_named_targets() {
        let source = complete_bundled_pack_source_for_test();
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();
        executor.observe_source_candidate_for_scope(&source, TerrainProgramScope::Overworld);
        let program = executor
            .prepared_lowered_entity_source_program()
            .unwrap()
            .expect("the complete source fixture must prepare the entity contract");
        let mut gal = gal();
        let targets = executor
            .stage_source_color_targets(
                &mut gal,
                73,
                Extent3d {
                    width: 320,
                    height: 180,
                    depth: 1,
                },
            )
            .unwrap()
            .expect("selected source must stage Rust-owned named targets");
        let outputs = executor
            .resolve_entity_source_color_outputs(&program, &targets)
            .unwrap();
        assert_eq!(
            vec![
                (TerrainPassOutput::LitTerrainColor, 0),
                (TerrainPassOutput::MaterialAuxiliary, 6),
                (TerrainPassOutput::ViewSpaceNormal, 5),
            ],
            outputs
                .iter()
                .map(|output| (output.output, output.source_slot))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            targets.target("primary").unwrap().current_attachment_view,
            outputs[0].view
        );
        executor.discard_source_color_targets_submission(&mut gal);
        executor.destroy(&mut gal).unwrap();
    }

    #[test]
    fn active_puddle_shadow_writer_requires_the_owned_semantic_resource() {
        let complete = complete_bundled_pack_source_for_test();
        let mut files = complete.files();
        files.push(ShaderSourceFile::new(
            crate::render::vulkanic::shader_pack::source::RUNTIME_OPTIONS_PATH,
            "RAIN_PUDDLES=1\nDETAIL_QUALITY=3\n",
        ));
        let source = ShaderPackSource::new("complete-puddles", 92, files).unwrap();
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();

        executor.observe_source_candidate_for_scope(&source, TerrainProgramScope::Overworld);

        assert!(
            matches!(
                executor.source_candidate(),
                TerrainSourceCandidateState::Discovered {
                    source_shadow_preprocess_error: None,
                    source_shadow_lowering_error: None,
                    source_lowered_shadow_pair: Some(shadow),
                    ..
                } if shadow.owned_storage_roles() == [TerrainSourceResourceRole::PuddleOccupancy]
            ),
            "{:#?}",
            executor.source_candidate()
        );
        assert!(executor
            .candidate_source_requires_resource(TerrainSourceResourceRole::PuddleOccupancy,));
        let shadow = executor
            .prepared_lowered_shadow_source_program()
            .unwrap()
            .expect("the active puddle source shadow pair must remain privately prepared");
        assert!(!shadow.vertex.source.contains("puddle_img"));
        assert!(!shadow.vertex.source.contains("imageStore"));
    }

    #[test]
    fn source_shadow_depth_resources_are_generation_bound_compare_samplers() {
        let source = complete_bundled_pack_source_for_test();
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();
        let mut gal = gal();
        executor.observe_source_candidate_for_scope(&source, TerrainProgramScope::Overworld);

        let texture = gal
            .create_texture(TextureDesc {
                label: "source-shadow-depth-test.texture".to_string(),
                dimension: TextureDimension::D2,
                format: TextureFormat::Depth32Float,
                extent: Extent3d {
                    width: 16,
                    height: 16,
                    depth: 1,
                },
                mip_levels: 1,
                array_layers: 1,
                usages: vec![TextureUsage::DepthStencilAttachment, TextureUsage::Sampled],
            })
            .unwrap();
        let view = gal
            .create_texture_view(TextureViewDesc {
                label: "source-shadow-depth-test.view".to_string(),
                texture,
                format: TextureFormat::Depth32Float,
                base_mip: 0,
                mip_count: 1,
                base_layer: 0,
                layer_count: 1,
            })
            .unwrap();
        let input = TerrainSourceShadowDepthInput {
            shader_pack_generation: source.generation(),
            world_generation: 4,
            shader_graph_generation: 9,
            shadow_depth_view: view,
        };

        let first = executor
            .ensure_candidate_source_shadow_depth_resources(&mut gal, input)
            .unwrap()
            .expect("source declares two shadow compare samplers");
        let primary = first
            .combined_sampler_for(TerrainSourceResourceRole::ShadowDepthPrimary)
            .unwrap();
        let secondary = first
            .combined_sampler_for(TerrainSourceResourceRole::ShadowDepthSecondary)
            .unwrap();
        assert_ne!(primary, secondary);
        assert_eq!(2, first.len());
        assert_eq!(
            Some(TerrainSourceResourceRole::ShadowDepthPrimary.expected_sampled_resource_shape()),
            first
                .availability()
                .resource_for(TerrainSourceResourceRole::ShadowDepthPrimary)
                .map(|resource| resource.shape)
        );

        let cached = executor
            .ensure_candidate_source_shadow_depth_resources(&mut gal, input)
            .unwrap()
            .unwrap();
        assert_eq!(
            Some(primary),
            cached.combined_sampler_for(TerrainSourceResourceRole::ShadowDepthPrimary)
        );

        let replaced = executor
            .ensure_candidate_source_shadow_depth_resources(
                &mut gal,
                TerrainSourceShadowDepthInput {
                    shader_graph_generation: 10,
                    ..input
                },
            )
            .unwrap()
            .unwrap();
        assert_ne!(
            Some(primary),
            replaced.combined_sampler_for(TerrainSourceResourceRole::ShadowDepthPrimary)
        );
        executor
            .clear_candidate_source_shadow_depth_resources(&mut gal)
            .unwrap();
        gal.destroy(view).unwrap();
        gal.destroy(texture).unwrap();
    }

    #[test]
    fn source_shadow_color_requires_an_owned_color_view_and_retires_generation_bound_wrappers() {
        let source = complete_bundled_pack_source_for_test();
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();
        let mut gal = gal();
        executor.observe_source_candidate_for_scope(&source, TerrainProgramScope::Overworld);

        let texture = gal
            .create_texture(TextureDesc {
                label: "source-shadow-color-test.texture".to_string(),
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba8Unorm,
                extent: Extent3d {
                    width: 16,
                    height: 16,
                    depth: 1,
                },
                mip_levels: 1,
                array_layers: 1,
                usages: vec![
                    TextureUsage::ColorAttachment,
                    TextureUsage::Sampled,
                    TextureUsage::TransferSrc,
                ],
            })
            .unwrap();
        let view = gal
            .create_texture_view(TextureViewDesc {
                label: "source-shadow-color-test.view".to_string(),
                texture,
                format: TextureFormat::Rgba8Unorm,
                base_mip: 0,
                mip_count: 1,
                base_layer: 0,
                layer_count: 1,
            })
            .unwrap();
        let sampler = gal
            .create_sampler(SamplerDesc {
                label: "source-shadow-color-test.sampler".to_string(),
                min_filter: SamplerFilter::Nearest,
                mag_filter: SamplerFilter::Nearest,
                mip_filter: SamplerFilter::Nearest,
                address_u: SamplerAddressMode::ClampToEdge,
                address_v: SamplerAddressMode::ClampToEdge,
                address_w: SamplerAddressMode::ClampToEdge,
                comparison: None,
            })
            .unwrap();
        let input = TerrainSourceShadowColorInput {
            shader_pack_generation: source.generation(),
            world_generation: 4,
            shader_graph_generation: 9,
            shadow_color_view: view,
            shadow_color_secondary_view: view,
            sampler,
        };

        let first = executor
            .ensure_candidate_source_shadow_color_resources(&mut gal, input)
            .unwrap()
            .expect("source declares shadowcolor0 as a color resource");
        let combined = first
            .combined_sampler_for(TerrainSourceResourceRole::ShadowColor)
            .unwrap();
        assert_eq!(1, first.len());
        assert_eq!(
            Some(TerrainSourceResourceRole::ShadowColor.expected_sampled_resource_shape()),
            first
                .availability()
                .resource_for(TerrainSourceResourceRole::ShadowColor)
                .map(|resource| resource.shape)
        );
        assert_eq!(
            Some(combined),
            executor
                .ensure_candidate_source_shadow_color_resources(&mut gal, input)
                .unwrap()
                .unwrap()
                .combined_sampler_for(TerrainSourceResourceRole::ShadowColor)
        );

        let replaced = executor
            .ensure_candidate_source_shadow_color_resources(
                &mut gal,
                TerrainSourceShadowColorInput {
                    shader_graph_generation: 10,
                    ..input
                },
            )
            .unwrap()
            .unwrap();
        assert_ne!(
            Some(combined),
            replaced.combined_sampler_for(TerrainSourceResourceRole::ShadowColor)
        );
        executor
            .clear_candidate_source_shadow_color_resources(&mut gal)
            .unwrap();
        gal.destroy(sampler).unwrap();
        gal.destroy(view).unwrap();
        gal.destroy(texture).unwrap();
    }

    #[test]
    fn source_shadow_color_resource_set_preserves_two_declared_color_roles() {
        let primary_view = Handle::new(HandleKind::TextureView, 91, 1).unwrap();
        let secondary_view = Handle::new(HandleKind::TextureView, 92, 1).unwrap();
        let sampler = Handle::new(HandleKind::Sampler, 93, 1).unwrap();
        let primary_combined = Handle::new(HandleKind::CombinedTextureSampler, 94, 1).unwrap();
        let secondary_combined = Handle::new(HandleKind::CombinedTextureSampler, 95, 1).unwrap();
        let resources = TerrainSourceShadowColorResources {
            shader_pack_generation: 7,
            world_generation: 8,
            shader_graph_generation: 9,
            shadow_color_view: primary_view,
            shadow_color_secondary_view: secondary_view,
            sampler,
            combined_samplers: BTreeMap::from([
                (TerrainSourceResourceRole::ShadowColor, primary_combined),
                (
                    TerrainSourceResourceRole::ShadowColorSecondary,
                    secondary_combined,
                ),
            ]),
        };

        let set = resources.semantic_resource_set().unwrap();
        assert_eq!(2, set.len());
        assert_eq!(
            Some(primary_combined),
            set.combined_sampler_for(TerrainSourceResourceRole::ShadowColor)
        );
        assert_eq!(
            Some(secondary_combined),
            set.combined_sampler_for(TerrainSourceResourceRole::ShadowColorSecondary)
        );
        assert_eq!(
            Some(TerrainSourceResourceRole::ShadowColorSecondary.expected_sampled_resource_shape()),
            set.availability()
                .resource_for(TerrainSourceResourceRole::ShadowColorSecondary)
                .map(|resource| resource.shape)
        );
    }

    #[test]
    fn source_shadow_color_wrapper_binds_both_source_declared_roles() {
        let source = ShaderPackSource::new(
            "two-shadow-color-source",
            41,
            vec![
                ShaderSourceFile::new(
                    "gbuffers_terrain.vsh",
                    "#version 130\nout vec2 texCoord;\nout vec4 glColor;\nout float smoothnessD;\nout float materialMask;\nout float skyLightFactor;\nvoid main() { texCoord = gl_MultiTexCoord0.xy; glColor = vec4(1.0); smoothnessD = 0.0; materialMask = 0.0; skyLightFactor = 1.0; gl_Position = ftransform(); }",
                ),
                ShaderSourceFile::new(
                    "gbuffers_terrain.fsh",
                    "#version 130\nin vec2 texCoord;\nin vec4 glColor;\nin float smoothnessD;\nin float materialMask;\nin float skyLightFactor;\nuniform sampler2D tex;\nuniform sampler2D shadowcolor0;\nuniform sampler2D shadowcolor1;\nuniform sampler2DShadow shadowtex0;\nvoid DoLighting() {}\n/* DRAWBUFFERS:06 */\nvoid main() { vec4 color = texture2D(tex, texCoord); color.rgb += (texture2D(shadowcolor0, texCoord).rgb + texture2D(shadowcolor1, texCoord).rgb) * 0.0; if (color.a <= 0.00001) discard; color.rgb *= glColor.rgb; DoLighting(); gl_FragData[0] = color; gl_FragData[1] = vec4(smoothnessD, materialMask, skyLightFactor, 1.0); }",
                ),
                ShaderSourceFile::new(
                    TERRAIN_RESOURCE_BINDINGS_PATH,
                    "tex=material_atlas\nshadowcolor0=shadow_color\nshadowcolor1=shadow_color_secondary\nshadowtex0=shadow_depth_primary\n",
                ),
                ShaderSourceFile::new("lib/common.glsl", "#define TEST 1\n"),
                ShaderSourceFile::new("shaders.properties", ""),
                ShaderSourceFile::new("block.properties", ""),
            ],
        )
        .unwrap();
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();
        executor.observe_source_candidate(&source);
        assert!(
            executor.candidate_source_requires_resource(TerrainSourceResourceRole::ShadowColor),
            "source discovery did not retain the primary shadow-color binding: {:?}",
            executor.source_candidate()
        );
        assert!(
            executor.candidate_source_requires_resource(
                TerrainSourceResourceRole::ShadowColorSecondary
            ),
            "source discovery did not retain the secondary shadow-color binding: {:?}",
            executor.source_candidate()
        );

        let mut gal = gal();
        let create_view = |gal: &mut VulkanicGal, label: &str| {
            let texture = gal
                .create_texture(TextureDesc {
                    label: format!("{label}.texture"),
                    dimension: TextureDimension::D2,
                    format: TextureFormat::Rgba8Unorm,
                    extent: Extent3d {
                        width: 4,
                        height: 4,
                        depth: 1,
                    },
                    mip_levels: 1,
                    array_layers: 1,
                    usages: vec![TextureUsage::ColorAttachment, TextureUsage::Sampled],
                })
                .unwrap();
            let view = gal
                .create_texture_view(TextureViewDesc {
                    label: format!("{label}.view"),
                    texture,
                    format: TextureFormat::Rgba8Unorm,
                    base_mip: 0,
                    mip_count: 1,
                    base_layer: 0,
                    layer_count: 1,
                })
                .unwrap();
            (texture, view)
        };
        let (primary_texture, primary_view) = create_view(&mut gal, "source-shadow-primary");
        let (secondary_texture, secondary_view) = create_view(&mut gal, "source-shadow-secondary");
        let sampler = gal
            .create_sampler(SamplerDesc {
                label: "source-shadow-color-pair.sampler".to_string(),
                min_filter: SamplerFilter::Nearest,
                mag_filter: SamplerFilter::Nearest,
                mip_filter: SamplerFilter::Nearest,
                address_u: SamplerAddressMode::ClampToEdge,
                address_v: SamplerAddressMode::ClampToEdge,
                address_w: SamplerAddressMode::ClampToEdge,
                comparison: None,
            })
            .unwrap();
        let set = executor
            .ensure_candidate_source_shadow_color_resources(
                &mut gal,
                TerrainSourceShadowColorInput {
                    shader_pack_generation: source.generation(),
                    world_generation: 4,
                    shader_graph_generation: 9,
                    shadow_color_view: primary_view,
                    shadow_color_secondary_view: secondary_view,
                    sampler,
                },
            )
            .unwrap()
            .expect("both declared semantic shadow color resources must be prepared");
        assert_eq!(2, set.len());
        assert_ne!(
            set.combined_sampler_for(TerrainSourceResourceRole::ShadowColor),
            set.combined_sampler_for(TerrainSourceResourceRole::ShadowColorSecondary),
        );

        executor
            .clear_candidate_source_shadow_color_resources(&mut gal)
            .unwrap();
        gal.destroy(sampler).unwrap();
        for (view, texture) in [
            (secondary_view, secondary_texture),
            (primary_view, primary_texture),
        ] {
            gal.destroy(view).unwrap();
            gal.destroy(texture).unwrap();
        }
    }

    #[test]
    fn source_discovery_is_cached_by_pack_generation_and_world_scope() {
        let source = complete_bundled_pack_source_for_test();
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();

        executor.observe_source_candidate_for_scope(&source, TerrainProgramScope::Overworld);
        executor.observe_distant_horizons_source_candidate_for_scope(
            &source,
            TerrainProgramScope::Overworld,
        );
        let terrain = executor.source_candidate.clone();
        let distant_horizons = executor.distant_horizons_source_candidate.clone();
        assert!(executor.source_candidate_matches(&source, TerrainProgramScope::Overworld));
        assert!(executor
            .distant_horizons_source_candidate_matches(&source, TerrainProgramScope::Overworld));

        // A repeated render-frame observation must retain the already lowered
        // semantic candidates rather than re-expanding the configured pack.
        executor.observe_source_candidate_for_scope(&source, TerrainProgramScope::Overworld);
        executor.observe_distant_horizons_source_candidate_for_scope(
            &source,
            TerrainProgramScope::Overworld,
        );
        assert_eq!(terrain, executor.source_candidate);
        assert_eq!(distant_horizons, executor.distant_horizons_source_candidate);

        // Scope remains part of the cache key: a dimension transition must
        // rediscover semantic source paths even with the same pack generation.
        assert!(!executor.source_candidate_matches(&source, TerrainProgramScope::Nether));
        assert!(!executor
            .distant_horizons_source_candidate_matches(&source, TerrainProgramScope::Nether));
    }

    #[test]
    fn source_main_depth_exposes_only_confirmed_snapshot_roles() {
        let source = complete_bundled_pack_source_for_test();
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();
        executor.observe_source_candidate_for_scope(&source, TerrainProgramScope::Overworld);
        executor.observe_distant_horizons_source_candidate_for_scope(
            &source,
            TerrainProgramScope::Overworld,
        );
        assert!(executor.candidate_source_requires_resource(TerrainSourceResourceRole::MainDepth));
        assert!(executor.candidate_source_requires_resource(
            TerrainSourceResourceRole::MainDepthBeforeTranslucency
        ));

        let mut gal = gal();
        let create_depth_view = |gal: &mut VulkanicGal, label: &str| {
            let texture = gal
                .create_texture(TextureDesc {
                    label: format!("{label}.texture"),
                    dimension: TextureDimension::D2,
                    format: TextureFormat::Depth32Float,
                    extent: Extent3d {
                        width: 16,
                        height: 16,
                        depth: 1,
                    },
                    mip_levels: 1,
                    array_layers: 1,
                    usages: vec![
                        TextureUsage::Sampled,
                        TextureUsage::TransferSrc,
                        TextureUsage::TransferDst,
                    ],
                })
                .unwrap();
            let view = gal
                .create_texture_view(TextureViewDesc {
                    label: format!("{label}.view"),
                    texture,
                    format: TextureFormat::Depth32Float,
                    base_mip: 0,
                    mip_count: 1,
                    base_layer: 0,
                    layer_count: 1,
                })
                .unwrap();
            (texture, view)
        };
        let (main_texture, main_view) = create_depth_view(&mut gal, "source-main-depth.main");
        let (before_texture, before_view) = create_depth_view(&mut gal, "source-main-depth.before");
        let (replacement_texture, replacement_view) =
            create_depth_view(&mut gal, "source-main-depth.replacement");
        let sampler = gal
            .create_sampler(SamplerDesc {
                label: "source-main-depth.sampler".to_string(),
                min_filter: SamplerFilter::Nearest,
                mag_filter: SamplerFilter::Nearest,
                mip_filter: SamplerFilter::Nearest,
                address_u: SamplerAddressMode::ClampToEdge,
                address_v: SamplerAddressMode::ClampToEdge,
                address_w: SamplerAddressMode::ClampToEdge,
                comparison: None,
            })
            .unwrap();
        let base_input = TerrainSourceMainDepthInput {
            shader_pack_generation: source.generation(),
            world_generation: 4,
            shader_graph_generation: 9,
            main_depth_view: main_view,
            before_translucency_view: None,
            previous_view: None,
            sampler,
        };
        let first = executor
            .ensure_candidate_source_main_depth_resources(&mut gal, base_input)
            .unwrap()
            .expect("source declares a current main depth role");
        assert!(first
            .combined_sampler_for(TerrainSourceResourceRole::MainDepth)
            .is_some());
        assert!(first
            .combined_sampler_for(TerrainSourceResourceRole::MainDepthBeforeTranslucency)
            .is_none());
        assert!(first
            .combined_sampler_for(TerrainSourceResourceRole::MainDepthPrevious)
            .is_none());

        let confirmed = executor
            .ensure_candidate_source_main_depth_resources(
                &mut gal,
                TerrainSourceMainDepthInput {
                    before_translucency_view: Some(before_view),
                    previous_view: None,
                    ..base_input
                },
            )
            .unwrap()
            .expect("confirmed depth snapshots must become separate semantic roles");
        assert_eq!(2, confirmed.len());
        for role in [
            TerrainSourceResourceRole::MainDepth,
            TerrainSourceResourceRole::MainDepthBeforeTranslucency,
        ] {
            assert!(confirmed.combined_sampler_for(role).is_some());
        }

        let replacement_error = executor
            .ensure_candidate_source_main_depth_resources(
                &mut gal,
                TerrainSourceMainDepthInput {
                    before_translucency_view: Some(replacement_view),
                    previous_view: None,
                    ..base_input
                },
            )
            .expect_err("an established temporal source-depth role must not retarget in place");
        assert!(replacement_error
            .to_string()
            .contains("cannot replace a view in place"));

        executor
            .clear_candidate_source_main_depth_resources(&mut gal)
            .unwrap();
        gal.destroy(sampler).unwrap();
        for (view, texture) in [
            (replacement_view, replacement_texture),
            (before_view, before_texture),
            (main_view, main_texture),
        ] {
            gal.destroy(view).unwrap();
            gal.destroy(texture).unwrap();
        }
    }

    #[test]
    fn copied_source_assets_are_generation_coherent_private_runtime_preparation() {
        let source = complete_bundled_pack_source_for_test();
        let assets = copied_png_assets_for(&source);
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();
        let mut gal = gal();

        executor.observe_source_candidate_for_scope(&source, TerrainProgramScope::Overworld);
        assert!(executor
            .ensure_candidate_source_asset_resources(&mut gal, &assets)
            .unwrap());
        assert!(executor
            .candidate_source_asset_resource_count()
            .is_some_and(|count| count > 0));
        assert!(
            executor
                .source_asset_resources
                .as_ref()
                .and_then(|resources| resources.combined_sampler_for("gaux4"))
                .is_some(),
            "shadow-only gaux4 must be retained from the separately lowered shadow plan"
        );
        assert!(!executor
            .ensure_candidate_source_asset_resources(&mut gal, &assets)
            .unwrap());

        let mismatched = ShaderPackAssets::new(ShaderPackAssetUpdate {
            pack_name: "different-pack".to_string(),
            generation: source.generation(),
            files: Vec::new(),
        })
        .unwrap();
        assert!(executor
            .ensure_candidate_source_asset_resources(&mut gal, &mismatched)
            .unwrap_err()
            .to_string()
            .contains("do not match source candidate"));

        executor
            .clear_candidate_source_asset_resources(&mut gal)
            .unwrap();
        assert_eq!(None, executor.candidate_source_asset_resource_count());
        executor.destroy(&mut gal).unwrap();
        assert!(gal.metrics().resource_destroys > 0);
    }

    #[test]
    fn complete_source_distinguishes_active_terrain_resources_from_global_declarations() {
        let source = complete_bundled_pack_source_for_test();
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();

        executor.observe_source_candidate_for_scope(&source, TerrainProgramScope::Overworld);

        let TerrainSourceCandidateState::Discovered {
            source_lowered_pair: Some(lowered),
            source_lowering_summary: Some(summary),
            source_uniform_requirement_summary: Some(uniform_summary),
            source_resource_binding_count: Some(8),
            source_resource_binding_error: None,
            ..
        } = executor.source_candidate()
        else {
            panic!("expected complete source to remain a diagnostic-only candidate");
        };
        assert_eq!(
            vec![
                "atlasSize",
                "cameraPosition",
                "cameraPositionFract",
                "darknessLightFactor",
                "far",
                "fogColor",
                "frameCounter",
                "frameTimeCounter",
                "framemod8",
                "gbufferModelView",
                "gbufferModelViewInverse",
                "gbufferProjection",
                "gbufferProjectionInverse",
                "heldBlockLightValue",
                "heldBlockLightValue2",
                "heldItemId",
                "heldItemId2",
                "inBasaltDeltas",
                "inCrimsonForest",
                "inDry",
                "inNetherWastes",
                "inSnowy",
                "inSoulValley",
                "inWarpedForest",
                "isEyeInWater",
                "moonPhase",
                "nightVision",
                "rainFactor",
                "relativeEyePosition",
                "screenBrightness",
                "shadowModelView",
                "shadowModelViewInverse",
                "shadowProjection",
                "shadowProjectionInverse",
                "skyColor",
                "sunAngle",
                "viewHeight",
                "viewWidth",
                "worldDay",
                "worldTime",
            ],
            lowered
                .uniform_contract()
                .fields()
                .iter()
                .map(|field| field.name())
                .collect::<Vec<_>>(),
            "the source-derived environment contract must not be silently narrowed"
        );
        assert_eq!(27, summary.opaque_resource_count);
        assert_eq!(8, summary.active_opaque_resource_count);
        assert_eq!(40, uniform_summary.field_count);
        assert_eq!(40, uniform_summary.resolved_field_count);
        assert!(uniform_summary.unresolved_field_names.is_empty());
        assert_eq!(
            vec![
                "floodfill_sampler",
                "floodfill_sampler_copy",
                "noisetex",
                "shadowcolor0",
                "shadowtex0",
                "shadowtex1",
                "specular",
                "tex",
            ],
            lowered
                .opaque_resource_contract()
                .active_resources()
                .map(|resource| resource.name())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            8,
            executor
                .source_resource_binding_plan()
                .unwrap()
                .bindings()
                .len()
        );
        assert!(matches!(
            executor.source_candidate(),
            TerrainSourceCandidateState::Discovered {
                source_shadow_resource_binding_count: Some(count),
                source_shadow_resource_binding_error: None,
                source_shadow_resource_bindings: Some(_),
                ..
            } if *count > 0
        ));
        assert!(executor
            .prepared_lowered_terrain_source_program(TerrainMaterialProgramKind::Opaque)
            .unwrap()
            .is_some());
    }

    #[test]
    fn declared_source_resources_progress_through_private_candidate_preparation() {
        let source = ShaderPackSource::new(
            "declared-resource-source",
            19,
            vec![
                ShaderSourceFile::new(
                    "program/gbuffers_terrain.glsl",
                    "#version 130\nuniform sampler2D tex;\nvoid DoLighting() {}\n/* DRAWBUFFERS:06 */\nvoid main() { vec4 color = texture2D(tex, texCoord); if (color.a <= 0.00001) discard; color.rgb *= glColor.rgb; DoLighting(); gl_FragData[0] = color; gl_FragData[1] = vec4(smoothnessD, materialMask, skyLightFactor, 1.0); }",
                ),
                ShaderSourceFile::new(
                    TERRAIN_RESOURCE_BINDINGS_PATH,
                    "tex=material_atlas\n",
                ),
                ShaderSourceFile::new("lib/common.glsl", "#define SHADOW_QUALITY 2\n"),
                ShaderSourceFile::new("shaders.properties", ""),
                ShaderSourceFile::new("block.properties", ""),
            ],
        )
        .unwrap();
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();
        executor.observe_source_candidate(&source);

        let candidate = executor.source_candidate();
        assert!(
            matches!(
                candidate,
                TerrainSourceCandidateState::Discovered {
                    source_lowering_summary: Some(_),
                    source_lowering_error: None,
                    source_resource_binding_count: Some(1),
                    source_resource_binding_error: None,
                    ..
                }
            ),
            "{candidate:?}"
        );
        assert!(executor
            .candidate_fixture_terrain_program(TerrainMaterialProgramKind::Opaque, 0)
            .unwrap()
            .is_some());
        let preparation_error = executor
            .prepared_lowered_terrain_source_program(TerrainMaterialProgramKind::Opaque)
            .unwrap_err()
            .to_string();
        assert!(preparation_error.contains("compatibility_fragment_outputs"));
        let bindings = executor.source_resource_binding_plan().unwrap().bindings();
        assert_eq!(1, bindings.len());
        assert_eq!("tex", bindings[0].resource_name());
        assert_eq!(TerrainSourceResourceRole::MaterialAtlas, bindings[0].role());
        assert_eq!(
            crate::render::vulkanic::shader_pack::lowering::TerrainSourceOpaqueResourceKind::CombinedTextureSampler,
            bindings[0].kind()
        );
        assert_eq!(0, bindings[0].binding());
        assert_eq!(
            "vulkanic:builtin/terrain_opaque_v1",
            executor.plan().programs.terrain_opaque.identity.as_str()
        );
    }

    #[test]
    fn fully_lowered_source_is_retained_for_private_preparation_only() {
        let source = ShaderPackSource::new(
            "retained-lowered-source",
            31,
            vec![
                ShaderSourceFile::new(
                    "gbuffers_terrain.vsh",
                    "#version 130\nout vec2 texCoord;\nout vec4 glColor;\nout float smoothnessD;\nout float materialMask;\nout float skyLightFactor;\nuniform sampler2D tex;\nvoid main() { texCoord = gl_MultiTexCoord0.xy; glColor = vec4(1.0); smoothnessD = 0.0; materialMask = 0.0; skyLightFactor = 1.0; gl_Position = ftransform(); }",
                ),
                ShaderSourceFile::new(
                    "gbuffers_terrain.fsh",
                    "#version 130\nin vec2 texCoord;\nin vec4 glColor;\nin float smoothnessD;\nin float materialMask;\nin float skyLightFactor;\nuniform sampler2D tex;\nvoid DoLighting() {}\n/* DRAWBUFFERS:06 */\nvoid main() { vec4 color = texture2D(tex, texCoord); if (color.a <= 0.00001) discard; color.rgb *= glColor.rgb; DoLighting(); gl_FragData[0] = color; gl_FragData[1] = vec4(smoothnessD, materialMask, skyLightFactor, 1.0); }",
                ),
                ShaderSourceFile::new(TERRAIN_RESOURCE_BINDINGS_PATH, "tex=material_atlas\n"),
                ShaderSourceFile::new("lib/common.glsl", "#define TEST 1\n"),
                ShaderSourceFile::new("shaders.properties", ""),
                ShaderSourceFile::new("block.properties", ""),
            ],
        )
        .unwrap();
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();
        executor.observe_source_candidate(&source);

        assert!(matches!(
            executor.source_candidate(),
            TerrainSourceCandidateState::Discovered {
                source_uniform_requirement_summary: Some(summary),
                source_uniform_requirement_error: None,
                ..
            } if summary.field_count == 2
                && summary.resolved_field_count == 2
                && summary.unresolved_field_names.is_empty()
        ));

        let prepared = executor
            .prepared_lowered_terrain_source_program(TerrainMaterialProgramKind::Opaque)
            .unwrap()
            .expect("complete paired source must be retained privately");
        assert_eq!(
            "vulkanic:shader-pack/retained-lowered-source/terrain_opaque_source_gen31",
            prepared.identity.as_str()
        );
        assert!(prepared.vertex.source.contains("#version 450"));
        assert!(prepared.fragment.source.contains("#version 450"));
        assert_eq!(1, prepared.opaque_resource_bindings.bindings().len());
        assert_eq!(
            crate::render::vulkanic::shader_pack::lowering::TerrainSourceOpaqueResourceKind::CombinedTextureSampler,
            prepared.opaque_resource_bindings.bindings()[0].kind()
        );
        assert_eq!(
            "vulkanic:builtin/terrain_opaque_v1",
            executor.plan().programs.terrain_opaque.identity.as_str(),
            "source preparation must not replace the executable fixture plan"
        );
    }

    #[test]
    fn scoped_complementary_candidate_uses_world_entry_pair_not_shared_include_body() {
        let source = complete_bundled_pack_source_for_test();
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();

        executor.observe_source_candidate_for_scope(&source, TerrainProgramScope::Overworld);

        assert!(matches!(
            executor.source_candidate(),
            TerrainSourceCandidateState::Discovered {
                source_summary: Some(summary),
                source_preprocess_error: None,
                source_lowering_summary: Some(_),
                source_lowering_error: None,
                source_shadow_summary: Some(shadow_summary),
                source_shadow_preprocess_error: None,
                source_shadow_lowering_error: None,
                ..
            } if summary.vertex_entry == "world0/gbuffers_terrain.vsh"
                && summary.fragment_entry == "world0/gbuffers_terrain.fsh"
                && shadow_summary.vertex_entry == "world0/shadow.vsh"
                && shadow_summary.fragment_entry == "world0/shadow.fsh"
        ));
        assert!(executor
            .prepared_lowered_terrain_source_program(TerrainMaterialProgramKind::Opaque)
            .unwrap()
            .is_some());
        assert!(executor
            .prepared_lowered_shadow_source_program()
            .unwrap()
            .is_some());
    }

    #[test]
    fn scoped_distant_horizons_candidate_retains_its_own_source_pair_without_selecting_a_route() {
        let source = complete_bundled_pack_source_for_test();
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();
        let fixture_program = executor.plan().programs.terrain_opaque.identity.clone();

        executor.observe_source_candidate_for_scope(&source, TerrainProgramScope::Overworld);
        executor.observe_distant_horizons_source_candidate_for_scope(
            &source,
            TerrainProgramScope::Overworld,
        );

        assert!(
            matches!(
                executor.distant_horizons_source_candidate(),
                DistantHorizonsSourceCandidateState::Discovered {
                    generation,
                    source_summary: Some(summary),
                    source_preprocess_error: None,
                    source_lowering_error: None,
                    source_uniform_requirement_summary: Some(uniforms),
                    source_uniform_requirement_error: None,
                    source_resource_binding_count: Some(_),
                    source_resource_binding_error: None,
                    source_color_target_count: Some(8),
                    source_color_target_error: None,
                    source_color_target_gal_schema_error: None,
                    source_color_targets: Some(targets),
                    depth_consumer_preparation,
                    ..
                } if *generation == source.generation()
                    && summary.vertex_entry == "world0/dh_terrain.vsh"
                    && summary.fragment_entry == "world0/dh_terrain.fsh"
                    && uniforms.field_count == uniforms.resolved_field_count
                    && targets.target("primary").is_some()
                    && depth_consumer_preparation.iter().any(|consumer| {
                        consumer.stage_path == "world0/deferred1.fsh"
                            && consumer.reads_opaque_depth
                            && consumer.source_preprocess_error.is_none()
                            && consumer.source_lowering_error.is_none()
                            && consumer.source_resource_binding_error.is_none()
                            && !consumer.source_output_roles.is_empty()
                            && consumer.source_program_preparation_error.is_none()
                            && consumer.source_program_identity.as_deref().is_some_and(|identity|
                                identity.contains("world0-deferred1-source-gen91")
                            )
                            && consumer.source_program.as_ref().is_some_and(|program|
                                program.identity.as_str().contains("world0-deferred1-source-gen91")
                            )
                            && consumer
                                .source_feedback_roles
                                .iter()
                                .any(|role| role == "shader_pack_color:primary")
                            && consumer.source_summary.as_ref().is_some_and(|summary| {
                                summary.vertex_entry == "world0/deferred1.vsh"
                                    && summary.fragment_entry == "world0/deferred1.fsh"
                            })
                    })
                    && depth_consumer_preparation.iter().any(|consumer| {
                        consumer.stage_path == "world0/composite.fsh"
                            && consumer.reads_opaque_depth
                            && consumer.reads_depth_before_translucency
                            && consumer.source_preprocess_error.is_none()
                    })
            ),
            "{:#?}",
            executor.distant_horizons_source_candidate()
        );

        let prepared = executor
            .prepared_lowered_distant_horizons_source_program()
            .unwrap()
            .expect("complete DH pair must remain available for Rust-owned target preparation");
        assert_eq!(
            "vulkanic:shader-pack/complementaryhungloified-complete-test/distant_horizons_opaque_source_gen91",
            prepared.identity.as_str()
        );
        assert_eq!(32, prepared.execution_interface.vertex_stride);
        assert_eq!(128, prepared.execution_interface.column_frame_bytes);
        assert!(prepared
            .vertex
            .source
            .contains("VulkanicDistantHorizonsVertices"));
        assert!(prepared
            .fragment
            .source
            .contains("out_distant_horizons_lit_color"));
        let depth_consumers = executor
            .prepared_lowered_distant_horizons_depth_consumers()
            .unwrap();
        assert_eq!(5, depth_consumers.len());
        assert!(depth_consumers.iter().any(|program| {
            program
                .identity
                .as_str()
                .contains("world0-deferred1-source-gen91")
        }));
        assert!(depth_consumers.iter().any(|program| {
            program
                .identity
                .as_str()
                .contains("world0-composite-source-gen91")
        }));
        let complete_chain = executor
            .prepared_lowered_post_terrain_fullscreen_programs()
            .expect("every scoped fullscreen stage must either retain a lowered program or expose its first precise preparation failure");
        assert_eq!(
            8,
            complete_chain.len(),
            "the bundled Overworld pack has one complete retained post-terrain source chain",
        );
        assert_eq!(
            "vulkanic:shader-pack/complementaryhungloified-complete-test/world0-final-source-gen91",
            complete_chain.last().unwrap().identity.as_str(),
        );
        assert_eq!(
            fixture_program,
            executor.plan().programs.terrain_opaque.identity,
            "DH preparation must not replace the active near-terrain fixture or select a DH route",
        );
    }

    #[test]
    fn dh_water_discovery_never_reuses_the_opaque_dh_source_pair() {
        let complete = complete_bundled_pack_source_for_test();
        let source = ShaderPackSource::new(
            "complete-dh-without-water",
            92,
            complete
                .files()
                .into_iter()
                .filter(|file| !file.path.contains("dh_water"))
                .collect(),
        )
        .unwrap();
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();

        executor.observe_distant_horizons_source_candidate_for_scope(
            &source,
            TerrainProgramScope::Overworld,
        );

        assert!(matches!(
            executor.distant_horizons_source_candidate(),
            DistantHorizonsSourceCandidateState::Discovered {
                contract,
                translucent_contract: DistantHorizonsTranslucentSourceCandidate::Unavailable,
                ..
            } if contract.program_path == "world0/dh_terrain.fsh"
        ));
    }

    #[test]
    fn dh_water_runtime_preparation_retains_a_distinct_lowered_program() {
        let complete = complete_bundled_pack_source_for_test();
        let source = ShaderPackSource::new(
            "complete-dh-water-runtime",
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
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();

        executor.observe_distant_horizons_source_candidate_for_scope(
            &source,
            TerrainProgramScope::Overworld,
        );

        let program = executor
            .prepared_lowered_distant_horizons_translucent_source_program()
            .unwrap()
            .expect("selected dh_water source must retain its prepared Rust program");
        assert_eq!(
            crate::render::vulkanic::shader_pack::distant_horizons_contract::DistantHorizonsPassKind::Translucent,
            program.pass_kind
        );
        assert_eq!(
            Some(
                crate::render::vulkanic::shader_pack::terrain_contract::TerrainTranslucentBlend::SourceAlphaOver
            ),
            program.translucent_blend
        );
        assert!(program
            .identity
            .as_str()
            .contains("distant_horizons_translucent_source_gen92"));
        assert!(program.fragment.source.contains("depthtex1"));
    }

    #[test]
    fn distant_horizons_source_target_preparation_is_private_and_submission_confirmed() {
        let source = complete_bundled_pack_source_for_test();
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();
        let fixture_program = executor.plan().programs.terrain_opaque.identity.clone();
        executor.observe_distant_horizons_source_candidate_for_scope(
            &source,
            TerrainProgramScope::Overworld,
        );
        let mut gal = gal();
        let extent = Extent3d {
            width: 320,
            height: 180,
            depth: 1,
        };

        let targets = executor
            .stage_source_color_targets(&mut gal, 73, extent)
            .unwrap()
            .expect("a discovered DH source candidate must stage named private targets");
        assert_eq!(8, targets.targets().count());
        let primary = targets.target("primary").unwrap();
        assert_eq!(TextureFormat::R11fG11fB10f, primary.format);
        assert!(primary.previous_view.is_some());
        assert_eq!(9, primary.mip_levels);
        assert_eq!(
            fixture_program,
            executor.plan().programs.terrain_opaque.identity,
            "staging source images must not select a source terrain/DH route",
        );
        assert!(!executor.has_pending_vanilla_lightmap_submission());

        executor.confirm_source_color_targets_submission(&mut gal);
        let reused = executor
            .stage_source_color_targets(&mut gal, 73, extent)
            .unwrap()
            .unwrap();
        assert_eq!(primary, reused.target("primary").unwrap());
        executor.destroy(&mut gal).unwrap();
    }

    #[test]
    fn complete_source_color_target_identity_uses_every_scoped_fullscreen_stage() {
        let source = complete_bundled_pack_source_for_test();
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();
        executor.observe_source_candidate_for_scope(&source, TerrainProgramScope::Overworld);
        executor.observe_distant_horizons_source_candidate_for_scope(
            &source,
            TerrainProgramScope::Overworld,
        );
        let expected = match executor.source_candidate() {
            TerrainSourceCandidateState::Discovered {
                post_terrain_preparation,
                post_terrain_preparation_error: None,
                ..
            } => post_terrain_preparation,
            candidate => panic!("expected complete normal source discovery, got {candidate:#?}"),
        };
        assert_eq!(8, expected.len());
        let mut feedback = expected
            .iter()
            .flat_map(|stage| stage.source_feedback_roles.iter())
            .map(|role| shader_pack_color_name_from_role(role).unwrap())
            .collect::<Vec<_>>();
        feedback.sort();
        feedback.dedup();
        let mut mipmapped = expected
            .iter()
            .flat_map(|stage| stage.source_mipmap_roles.iter())
            .map(|role| shader_pack_color_name_from_role(role).unwrap())
            .collect::<Vec<_>>();
        mipmapped.sort();
        mipmapped.dedup();

        let mut gal = gal();
        let targets = executor
            .stage_complete_source_color_targets(
                &mut gal,
                73,
                Extent3d {
                    width: 320,
                    height: 180,
                    depth: 1,
                },
            )
            .unwrap()
            .expect("complete source discovery must stage one full-chain target generation");
        assert_eq!(feedback, targets.identity.feedback_target_names);
        assert_eq!(mipmapped, targets.identity.mipmapped_target_names);
        assert!(
            !targets.identity.feedback_target_names.is_empty(),
            "the complete chain must retain explicit feedback history rather than silently using the DH subset"
        );
        executor.discard_source_color_targets_submission(&mut gal);
        executor.destroy(&mut gal).unwrap();
    }

    #[test]
    fn bundled_overworld_sky_initializer_lowers_as_an_owned_source_stage() {
        let source = complete_bundled_pack_source_for_test();
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();
        executor.observe_source_candidate_for_scope(&source, TerrainProgramScope::Overworld);

        let sky = executor
            .prepared_lowered_pre_terrain_sky_program()
            .expect("a declared sky stage must either lower or expose a precise error")
            .expect("Complementary's overworld scope declares gbuffers_skybasic");
        assert_eq!("world0/gbuffers_skybasic.fsh", sky.source_stage_path);
        assert_eq!(
            FullscreenSourceRasterPrimitive::FullscreenTriangle,
            sky.raster_primitive,
            "the semantic sky initializer must cover the complete background target"
        );
        assert!(sky
            .vertex
            .source
            .contains("vulkanic_source_fullscreen_transform"));
        assert!(sky
            .vertex
            .source
            .contains("vulkanic_source_fullscreen_vertex_color"));
        assert!(sky
            .fragment
            .source
            .contains("out_vulkanic_source_color_primary"));
    }

    #[test]
    fn bundled_overworld_celestial_stage_lowers_with_owned_quad_semantics() {
        let source = complete_bundled_pack_source_for_test();
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();
        executor.observe_source_candidate_for_scope(&source, TerrainProgramScope::Overworld);

        let celestial = executor
            .prepared_lowered_pre_terrain_celestial_program()
            .expect("a declared celestial stage must either lower or expose a precise error")
            .expect("Complementary's overworld scope declares gbuffers_skytextured");
        assert_eq!(
            "world0/gbuffers_skytextured.fsh",
            celestial.source_stage_path
        );
        assert_eq!(
            FullscreenSourceRasterPrimitive::VanillaCelestialQuad,
            celestial.raster_primitive
        );
        assert!(celestial
            .vertex
            .source
            .contains("vulkanic_source_fullscreen_celestial_position"));
        assert!(celestial
            .vertex
            .source
            .contains("uniform VulkanicSourceTerrainUniforms"));
        for declaration in [
            "float sunAngle;",
            "int moonPhase;",
            "int vulkanic_source_celestial_is_moon;",
            "float vulkanic_source_celestial_alpha;",
            "float vulkanic_source_celestial_sun_path_rotation;",
        ] {
            assert!(
                celestial.vertex.source.contains(declaration),
                "owned celestial vertex GLSL is missing {declaration}"
            );
        }
        for name in [
            "vulkanic_source_celestial_is_moon",
            "vulkanic_source_celestial_alpha",
            "vulkanic_source_celestial_sun_path_rotation",
        ] {
            assert!(
                celestial
                    .execution_interface
                    .scalar_uniform_fields
                    .iter()
                    .any(|field| field.name() == name),
                "owned celestial program is missing injected semantic field {name}"
            );
            assert!(
                celestial.vertex.source.contains(name),
                "owned celestial vertex GLSL is missing injected semantic declaration {name}"
            );
        }
    }

    #[test]
    fn complete_source_color_target_staging_rejects_an_incomplete_world_chain() {
        let complete = complete_bundled_pack_source_for_test();
        let source = ShaderPackSource::new(
            "complete-pack-without-world-final",
            complete.generation(),
            complete
                .files()
                .into_iter()
                .filter(|file| file.path != "world0/final.vsh" && file.path != "world0/final.fsh")
                .collect(),
        )
        .unwrap();
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();
        executor.observe_source_candidate_for_scope(&source, TerrainProgramScope::Overworld);
        let mut gal = gal();
        let error = executor
            .stage_complete_source_color_targets(
                &mut gal,
                73,
                Extent3d {
                    width: 320,
                    height: 180,
                    depth: 1,
                },
            )
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("complete shader-pack fullscreen chain"),
            "incomplete source discovery must not allocate partial full-chain targets: {error}"
        );
        executor.destroy(&mut gal).unwrap();
    }

    #[test]
    fn complete_fullscreen_staging_rejects_missing_semantic_inputs_without_route_selection() {
        let source = complete_bundled_pack_source_for_test();
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();
        let fixture_program = executor.plan().programs.terrain_opaque.identity.clone();
        executor.observe_source_candidate_for_scope(&source, TerrainProgramScope::Overworld);
        let mut gal = gal();
        let extent = Extent3d {
            width: 320,
            height: 180,
            depth: 1,
        };
        let targets = executor
            .stage_complete_source_color_targets(&mut gal, 73, extent)
            .unwrap()
            .expect("complete source discovery must stage a private target generation");
        let error = executor
            .stage_complete_post_terrain_execution_plans(&mut gal, &targets, &[], extent)
            .unwrap_err();
        assert!(
            error.to_string().contains("semantic")
                || error.to_string().contains("resource"),
            "the complete chain must reject a missing source input rather than selecting a partial route: {error}"
        );
        assert_eq!(
            fixture_program,
            executor.plan().programs.terrain_opaque.identity,
            "private fullscreen staging must not replace the active fixture program or select gameplay execution"
        );
        executor.discard_source_color_targets_submission(&mut gal);
        executor.destroy(&mut gal).unwrap();
    }

    #[test]
    fn normal_terrain_and_distant_horizons_resolve_the_same_named_color_generation() {
        let source = complete_bundled_pack_source_for_test();
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();
        executor.observe_source_candidate_for_scope(&source, TerrainProgramScope::Overworld);
        executor.observe_distant_horizons_source_candidate_for_scope(
            &source,
            TerrainProgramScope::Overworld,
        );
        let program = executor
            .prepared_lowered_terrain_source_program(TerrainMaterialProgramKind::Opaque)
            .unwrap()
            .expect("normal terrain source must prepare before named target resolution");
        let mut gal = gal();
        let targets = executor
            .stage_source_color_targets(
                &mut gal,
                73,
                Extent3d {
                    width: 320,
                    height: 180,
                    depth: 1,
                },
            )
            .unwrap()
            .expect("shared source discovery must stage one named color generation");
        let attachments = executor
            .resolve_terrain_source_color_outputs(&program, &targets)
            .expect("normal terrain outputs must resolve through the shared target generation");
        assert_eq!(3, attachments.len());
        assert_eq!(
            "primary",
            attachments[0].role.shader_pack_color_name().unwrap()
        );
        assert_eq!(
            "material_auxiliary",
            attachments[1].role.shader_pack_color_name().unwrap()
        );
        assert_eq!(
            "normal_scene",
            attachments[2].role.shader_pack_color_name().unwrap()
        );
        assert_eq!(
            targets.target("primary").unwrap().current_attachment_view,
            attachments[0].view
        );
        assert_eq!(
            targets
                .target("material_auxiliary")
                .unwrap()
                .current_attachment_view,
            attachments[1].view
        );
        assert_eq!(
            targets
                .target("normal_scene")
                .unwrap()
                .current_attachment_view,
            attachments[2].view
        );
        executor.discard_source_color_targets_submission(&mut gal);
        executor.destroy(&mut gal).unwrap();
    }

    #[test]
    fn normal_terrain_source_pass_uses_named_color_targets_and_explicit_depth() {
        let source = complete_bundled_pack_source_for_test();
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();
        executor.observe_source_candidate_for_scope(&source, TerrainProgramScope::Overworld);
        let program = executor
            .prepared_lowered_terrain_source_program(TerrainMaterialProgramKind::Opaque)
            .unwrap()
            .expect("normal terrain source must prepare before pass construction");
        let mut gal = gal();
        let extent = Extent3d {
            width: 320,
            height: 180,
            depth: 1,
        };
        let targets = executor
            .stage_source_color_targets(&mut gal, 73, extent)
            .unwrap()
            .expect("selected source must stage named color targets");
        let color_attachments = executor
            .resolve_terrain_source_color_outputs(&program, &targets)
            .unwrap();
        let depth_texture = gal
            .create_texture(crate::render::vulkanic::resources::TextureDesc {
                label: "normal-terrain-source-depth".to_string(),
                dimension: crate::render::vulkanic::resources::TextureDimension::D2,
                format: crate::render::vulkanic::resources::TextureFormat::Depth32Float,
                extent,
                mip_levels: 1,
                array_layers: 1,
                usages: vec![
                    crate::render::vulkanic::resources::TextureUsage::DepthStencilAttachment,
                    crate::render::vulkanic::resources::TextureUsage::Sampled,
                ],
            })
            .unwrap();
        let depth_view = gal
            .create_texture_view(crate::render::vulkanic::resources::TextureViewDesc {
                label: "normal-terrain-source-depth.view".to_string(),
                texture: depth_texture,
                format: crate::render::vulkanic::resources::TextureFormat::Depth32Float,
                base_mip: 0,
                mip_count: 1,
                base_layer: 0,
                layer_count: 1,
            })
            .unwrap();
        let target = gal
            .create_render_target(crate::render::vulkanic::resources::RenderTargetDesc {
                label: "normal-terrain-source-target".to_string(),
                color_views: color_attachments
                    .iter()
                    .map(|attachment| attachment.view)
                    .collect(),
                depth_stencil_view: Some(depth_view),
                extent,
            })
            .unwrap();
        let pass = gal
            .create_render_pass(crate::render::vulkanic::resources::RenderPassDesc {
                label: "normal-terrain-source-pass".to_string(),
                target,
                color_formats: color_attachments
                    .iter()
                    .map(|attachment| attachment.format)
                    .collect(),
                depth_format: Some(crate::render::vulkanic::resources::TextureFormat::Depth32Float),
            })
            .unwrap();
        let mut operations = Vec::new();
        let transaction = executor
            .begin_source_color_transaction(
                &mut gal,
                &targets,
                ShaderPackColorBootstrapClearValues {
                    fog_color: ClearColor {
                        r: 0.1,
                        g: 0.2,
                        b: 0.3,
                        a: 1.0,
                    },
                },
                &mut operations,
            )
            .unwrap();
        executor
            .append_terrain_source_color_pass(
                &mut operations,
                &TerrainSourceColorPassTargets {
                    phase: TerrainSourceColorPassPhase::Bootstrap,
                    color_attachments: color_attachments.clone(),
                    clear_values: ShaderPackColorBootstrapClearValues {
                        fog_color: ClearColor {
                            r: 0.1,
                            g: 0.2,
                            b: 0.3,
                            a: 1.0,
                        },
                    },
                    depth_texture,
                    depth_view,
                    target,
                    pass,
                },
                &[],
            )
            .unwrap();
        executor
            .append_terrain_source_color_pass(
                &mut operations,
                &TerrainSourceColorPassTargets {
                    phase: TerrainSourceColorPassPhase::Translucent,
                    color_attachments: color_attachments.clone(),
                    clear_values: ShaderPackColorBootstrapClearValues {
                        fog_color: ClearColor {
                            r: 0.1,
                            g: 0.2,
                            b: 0.3,
                            a: 1.0,
                        },
                    },
                    depth_texture,
                    depth_view,
                    target,
                    pass,
                },
                &[],
            )
            .unwrap();
        let material_begin = operations.len();
        executor
            .append_textured_material_source_color_pass(
                &mut operations,
                &TerrainSourceColorPassTargets {
                    phase: TerrainSourceColorPassPhase::TexturedMaterial,
                    color_attachments: color_attachments.clone(),
                    clear_values: ShaderPackColorBootstrapClearValues {
                        fog_color: ClearColor {
                            r: 0.1,
                            g: 0.2,
                            b: 0.3,
                            a: 1.0,
                        },
                    },
                    depth_texture,
                    depth_view,
                    target,
                    pass,
                },
                &[],
            )
            .unwrap();
        let material_operations = &operations[material_begin..];
        assert!(material_operations.iter().any(|operation| matches!(
            operation,
            CommandOp::BeginPass { colors, depth_stencil, .. }
                if colors.iter().all(|color| color.load_op == AttachmentLoadOp::Load)
                    && depth_stencil.as_ref().is_some_and(|depth| depth.load_op == AttachmentLoadOp::Load)
        )));
        assert!(material_operations.iter().all(|operation| !matches!(
            operation,
            CommandOp::BeginPass { colors, .. }
                if colors.iter().any(|color| color.load_op == AttachmentLoadOp::Clear)
        )));
        let cloud_begin = operations.len();
        executor
            .append_cloud_source_color_pass(
                &mut operations,
                &TerrainSourceColorPassTargets {
                    phase: TerrainSourceColorPassPhase::Clouds,
                    color_attachments: color_attachments.clone(),
                    clear_values: ShaderPackColorBootstrapClearValues {
                        fog_color: ClearColor {
                            r: 0.1,
                            g: 0.2,
                            b: 0.3,
                            a: 1.0,
                        },
                    },
                    depth_texture,
                    depth_view,
                    target,
                    pass,
                },
                &[],
            )
            .unwrap();
        let cloud_operations = &operations[cloud_begin..];
        assert!(cloud_operations.iter().any(|operation| matches!(
            operation,
            CommandOp::BeginPass { colors, depth_stencil, .. }
                if colors.iter().all(|color| color.load_op == AttachmentLoadOp::Load)
                    && depth_stencil.as_ref().is_some_and(|depth| depth.load_op == AttachmentLoadOp::Load)
        )));
        assert!(cloud_operations.iter().all(|operation| !matches!(
            operation,
            CommandOp::BeginPass { colors, .. }
                if colors.iter().any(|color| color.load_op == AttachmentLoadOp::Clear)
        )));
        assert!(operations.iter().any(|operation| matches!(
            operation,
            CommandOp::BeginPass { target: pass_target, .. } if *pass_target == target
        )));
        assert!(operations.iter().any(|operation| matches!(
            operation,
            CommandOp::Barrier(barrier)
                if barrier.resource == color_attachments[0].texture
                    && barrier.before == TextureUsageState::ShaderRead
                    && barrier.after == TextureUsageState::ColorAttachment
        )));
        assert!(
            operations.iter().any(|operation| matches!(
                operation,
                CommandOp::BeginPass {
                    colors,
                    depth_stencil: Some(depth),
                    ..
                } if colors.iter().all(|attachment| attachment.load_op == AttachmentLoadOp::Load)
                    && depth.load_op == AttachmentLoadOp::Load
            )),
            "the translucent writer must preserve opaque/cutout color and depth"
        );
        assert!(
            operations.iter().any(|operation| matches!(
                operation,
                CommandOp::Barrier(barrier)
                    if barrier.resource == depth_texture
                        && barrier.before == TextureUsageState::ShaderRead
                        && barrier.after == TextureUsageState::DepthStencilAttachment
            )),
            "the translucent writer must explicitly reacquire depth after the normal terrain pass"
        );
        let entity_begin = operations.len();
        let entity_targets = TerrainSourceColorPassTargets {
            phase: TerrainSourceColorPassPhase::Entities,
            color_attachments: color_attachments.clone(),
            clear_values: ShaderPackColorBootstrapClearValues {
                fog_color: ClearColor {
                    r: 0.1,
                    g: 0.2,
                    b: 0.3,
                    a: 1.0,
                },
            },
            depth_texture,
            depth_view,
            target,
            pass,
        };
        executor
            .append_entity_source_color_pass(&mut operations, &entity_targets, &[])
            .unwrap();
        let entity_operations = &operations[entity_begin..];
        assert!(entity_operations.iter().any(|operation| matches!(
            operation,
            CommandOp::BeginPass { colors, depth_stencil, .. }
                if colors.iter().all(|color| color.load_op == AttachmentLoadOp::Load)
                    && depth_stencil.as_ref().is_some_and(|depth| depth.load_op == AttachmentLoadOp::Load)
        )));
        assert!(entity_operations.iter().all(|operation| !matches!(
            operation,
            CommandOp::BeginPass { colors, .. }
                if colors.iter().any(|color| color.load_op == AttachmentLoadOp::Clear)
        )));
        let hand_targets = TerrainSourceColorPassTargets {
            phase: TerrainSourceColorPassPhase::Hands,
            ..entity_targets.clone()
        };
        let hand_begin = operations.len();
        executor
            .append_hand_source_color_pass(&mut operations, &hand_targets, &[])
            .unwrap();
        let hand_operations = &operations[hand_begin..];
        assert!(hand_operations.iter().any(|operation| matches!(
            operation,
            CommandOp::BeginPass { colors, depth_stencil, .. }
                if colors.iter().all(|color| color.load_op == AttachmentLoadOp::Load)
                    && depth_stencil.as_ref().is_some_and(|depth| depth.load_op == AttachmentLoadOp::Clear)
        )), "the hand writer must preserve completed world color but clear its own depth domain");
        assert!(
            hand_operations.iter().any(|operation| matches!(
                operation,
                CommandOp::Barrier(barrier)
                    if barrier.resource == depth_texture
                        && barrier.before == TextureUsageState::Undefined
                        && barrier.after == TextureUsageState::DepthStencilAttachment
            )),
            "the hand writer must explicitly transition its fresh depth domain"
        );
        assert!(executor
            .append_hand_source_color_pass(&mut Vec::new(), &entity_targets, &[])
            .is_err());
        let wrong_phase = TerrainSourceColorPassTargets {
            phase: TerrainSourceColorPassPhase::TexturedMaterial,
            ..entity_targets
        };
        assert!(executor
            .append_entity_source_color_pass(&mut Vec::new(), &wrong_phase, &[])
            .is_err());
        gal.destroy(pass).unwrap();
        gal.destroy(target).unwrap();
        gal.destroy(depth_view).unwrap();
        gal.destroy(depth_texture).unwrap();
        transaction.discard(&mut executor, &mut gal);
        executor.destroy(&mut gal).unwrap();
    }

    #[test]
    fn first_translucent_source_writer_initializes_attachments() {
        assert_eq!(
            TextureUsageState::Undefined,
            TerrainSourceColorPassPhase::TranslucentFirst.depth_before()
        );
        assert_eq!(
            AttachmentLoadOp::Clear,
            TerrainSourceColorPassPhase::TranslucentFirst.depth_load_op()
        );
    }

    #[test]
    fn textured_material_source_resolves_its_own_named_color_outputs() {
        let source = complete_bundled_pack_source_for_test();
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();
        executor.observe_source_candidate_for_scope(&source, TerrainProgramScope::Overworld);
        let program = executor
            .prepared_lowered_textured_material_source_program()
            .unwrap()
            .expect("complete selected source must retain the textured material program");
        let mut gal = gal();
        let targets = executor
            .stage_source_color_targets(
                &mut gal,
                73,
                Extent3d {
                    width: 320,
                    height: 180,
                    depth: 1,
                },
            )
            .unwrap()
            .expect("selected source must stage named color targets");
        let resolved = executor
            .resolve_textured_material_source_color_outputs(&program, &targets)
            .unwrap();

        assert_eq!(3, resolved.len());
        assert_eq!(TerrainPassOutput::LitTerrainColor, resolved[0].output);
        assert_eq!(0, resolved[0].source_slot);
        assert_eq!(TerrainPassOutput::MaterialAuxiliary, resolved[1].output);
        assert_eq!(6, resolved[1].source_slot);
        assert_eq!(TerrainPassOutput::TranslucencyAuxiliary, resolved[2].output);
        assert_eq!(3, resolved[2].source_slot);

        executor.destroy(&mut gal).unwrap();
    }

    #[test]
    fn source_color_transaction_bootstraps_once_then_reuses_confirmed_targets() {
        let source = complete_bundled_pack_source_for_test();
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();
        executor.observe_source_candidate_for_scope(&source, TerrainProgramScope::Overworld);
        let mut gal = gal();
        let extent = Extent3d {
            width: 320,
            height: 180,
            depth: 1,
        };
        let targets = executor
            .stage_source_color_targets(&mut gal, 73, extent)
            .unwrap()
            .expect("selected source must stage named color targets");
        let clear_values = ShaderPackColorBootstrapClearValues {
            fog_color: ClearColor {
                r: 0.1,
                g: 0.2,
                b: 0.3,
                a: 1.0,
            },
        };
        let mut first_ops = Vec::new();
        let mut first = executor
            .begin_source_color_transaction(&mut gal, &targets, clear_values, &mut first_ops)
            .unwrap();
        assert!(first_ops
            .iter()
            .any(|operation| matches!(operation, CommandOp::BeginPass { .. })));
        first
            .record_external_outputs(&[TerrainSourceResourceRole::ShaderPackColor(
                "primary".to_string(),
            )])
            .unwrap();
        first.finish(&mut first_ops).unwrap();
        let token = gal
            .submit(SubmissionBatch {
                label: "source-color-bootstrap-once".to_string(),
                command_lists: vec![CommandList::from(CommandListDesc {
                    label: "source-color-bootstrap-once.commands".to_string(),
                    operations: first_ops,
                })],
            })
            .unwrap();
        first.confirm(&mut executor, &mut gal).unwrap();
        gal.retire_through_for_test(token.submission).unwrap();

        let mut second_ops = Vec::new();
        let second = executor
            .begin_source_color_transaction(&mut gal, &targets, clear_values, &mut second_ops)
            .unwrap();
        assert!(
            second_ops.is_empty(),
            "a confirmed source target generation must not repeat its bootstrap clears"
        );
        second.discard(&mut executor, &mut gal);
        executor.destroy(&mut gal).unwrap();
    }

    #[test]
    fn discarded_source_color_transaction_retires_only_pending_target_generation() {
        let source = complete_bundled_pack_source_for_test();
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();
        executor.observe_source_candidate_for_scope(&source, TerrainProgramScope::Overworld);
        let mut gal = gal();
        let extent = Extent3d {
            width: 320,
            height: 180,
            depth: 1,
        };
        let first = executor
            .stage_source_color_targets(&mut gal, 73, extent)
            .unwrap()
            .expect("ordinary source discovery must stage private named colors");
        let first_primary = first.target("primary").unwrap();
        let mut operations = Vec::new();
        executor
            .begin_source_color_transaction(
                &mut gal,
                &first,
                ShaderPackColorBootstrapClearValues {
                    fog_color: ClearColor {
                        r: 0.1,
                        g: 0.2,
                        b: 0.3,
                        a: 1.0,
                    },
                },
                &mut operations,
            )
            .unwrap()
            .discard(&mut executor, &mut gal);
        assert!(
            !operations.is_empty(),
            "discarded source preparation must have owned real bootstrap work before it is retired"
        );
        let restaged = executor
            .stage_source_color_targets(&mut gal, 73, extent)
            .unwrap()
            .expect("discarding an unsubmitted transaction must allow a clean restage");
        assert_ne!(
            first_primary.current_texture,
            restaged.target("primary").unwrap().current_texture,
            "unsubmitted source targets must never become the active generation"
        );
        executor.discard_source_color_targets_submission(&mut gal);
        executor.destroy(&mut gal).unwrap();
    }

    #[test]
    fn ordinary_and_distant_source_share_one_semantic_color_target_cache() {
        let source = complete_bundled_pack_source_for_test();
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();
        executor.observe_source_candidate_for_scope(&source, TerrainProgramScope::Overworld);
        executor.observe_distant_horizons_source_candidate_for_scope(
            &source,
            TerrainProgramScope::Overworld,
        );
        let mut gal = gal();
        let extent = Extent3d {
            width: 320,
            height: 180,
            depth: 1,
        };

        let ordinary = executor
            .stage_source_color_targets(&mut gal, 73, extent)
            .unwrap()
            .expect("ordinary source discovery must stage its named color targets");
        assert_eq!(8, ordinary.targets().count());
        assert!(ordinary.target("primary").is_some());
        executor.confirm_source_color_targets_submission(&mut gal);

        let shared = executor
            .stage_source_color_targets(&mut gal, 73, extent)
            .unwrap()
            .expect("DH source must reuse the matching pack-wide color targets");
        assert_eq!(ordinary.identity, shared.identity);
        assert_eq!(
            ordinary.target("primary"),
            shared.target("primary"),
            "ordinary terrain and DH must never allocate divergent primary targets"
        );
        executor.destroy(&mut gal).unwrap();
    }

    #[test]
    fn dh_fullscreen_pass_preparation_rejects_missing_external_semantics_before_execution() {
        let source = complete_bundled_pack_source_for_test();
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();
        executor.observe_distant_horizons_source_candidate_for_scope(
            &source,
            TerrainProgramScope::Overworld,
        );
        let mut gal = gal();
        let targets = executor
            .stage_source_color_targets(
                &mut gal,
                73,
                Extent3d {
                    width: 320,
                    height: 180,
                    depth: 1,
                },
            )
            .unwrap()
            .expect("a complete DH candidate must stage private named source targets");

        let error = executor
            .stage_distant_horizons_depth_consumer_execution_plans(
                &mut gal,
                &targets,
                &[TerrainSourceOwnedResourceSet::new(
                    TerrainSourceResourceAvailabilitySet::new(source.generation(), 73, []).unwrap(),
                    [],
                )
                .unwrap()],
                Extent3d {
                    width: 320,
                    height: 180,
                    depth: 1,
                },
            )
            .unwrap_err();
        assert!(
            error.to_string().contains("unavailable for semantic role"),
            "{error}"
        );
        executor.destroy(&mut gal).unwrap();
    }

    #[test]
    fn vanilla_source_completeness_excludes_discovered_distant_horizons_roles() {
        let source = complete_bundled_pack_source_for_test();
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();
        executor.observe_source_candidate_for_scope(&source, TerrainProgramScope::Overworld);
        executor.observe_distant_horizons_source_candidate_for_scope(
            &source,
            TerrainProgramScope::Overworld,
        );

        let vanilla_roles = executor.source_required_resource_roles_for_frame(false);
        let distant_roles = executor.source_required_resource_roles_for_frame(true);
        assert!(
            vanilla_roles.is_subset(&distant_roles),
            "admitting DH may add requirements but cannot drop ordinary terrain roles"
        );
        let normal_plans = executor.source_resource_binding_plans_for_frame(false);
        let all_plans = executor.source_resource_binding_plans_for_frame(true);
        assert!(
            normal_plans.len() <= all_plans.len(),
            "a vanilla-only frame must never acquire an additional source plan from discovered DH state"
        );
    }

    #[test]
    fn dh_source_and_fullscreen_consumers_participate_in_shared_resource_completeness() {
        let source = complete_bundled_pack_source_for_test();
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();
        executor.observe_distant_horizons_source_candidate_for_scope(
            &source,
            TerrainProgramScope::Overworld,
        );
        let mut expected_roles = executor
            .prepared_lowered_distant_horizons_source_program()
            .unwrap()
            .expect("bundled source exposes one DH draw program")
            .opaque_resource_bindings
            .bindings()
            .iter()
            .map(|binding| binding.role())
            .collect::<Vec<_>>();
        expected_roles.extend(
            executor
                .prepared_lowered_distant_horizons_depth_consumers()
                .unwrap()
                .into_iter()
                .flat_map(|program| {
                    program
                        .opaque_resource_bindings
                        .bindings()
                        .iter()
                        .map(|binding| binding.role())
                }),
        );
        expected_roles.sort();
        expected_roles.dedup();
        assert!(!expected_roles.is_empty());
        for role in expected_roles {
            assert!(
                executor.candidate_source_requires_resource(role.clone()),
                "shared source resource completeness omitted {}",
                role.semantic_name(),
            );
        }
    }

    #[test]
    fn rejected_distant_horizons_source_target_preparation_retires_only_pending_images() {
        let source = complete_bundled_pack_source_for_test();
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();
        executor.observe_distant_horizons_source_candidate_for_scope(
            &source,
            TerrainProgramScope::Overworld,
        );
        let mut gal = gal();
        let extent = Extent3d {
            width: 320,
            height: 180,
            depth: 1,
        };
        let resource_creates_before = gal.metrics().resource_creates;
        let resource_destroys_before = gal.metrics().resource_destroys;

        let staged = executor
            .stage_source_color_targets(&mut gal, 73, extent)
            .unwrap()
            .unwrap();
        let staged_resource_count = staged
            .targets()
            .map(|(_, target)| {
                2 + u64::from(target.current_attachment_view != target.current_view)
                    + u64::from(target.previous_view.is_some()) * 2
                    + u64::from(target.previous_attachment_view.is_some())
            })
            .sum::<u64>();
        assert_eq!(
            resource_creates_before + staged_resource_count,
            gal.metrics().resource_creates
        );
        executor.discard_source_color_targets_submission(&mut gal);
        assert_eq!(
            resource_destroys_before + staged_resource_count,
            gal.metrics().resource_destroys
        );

        executor
            .stage_source_color_targets(&mut gal, 73, extent)
            .unwrap()
            .unwrap();
        assert_eq!(
            resource_creates_before + staged_resource_count * 2,
            gal.metrics().resource_creates
        );
        executor.destroy(&mut gal).unwrap();
    }

    #[test]
    fn unresolved_source_scalar_uniforms_block_all_candidate_preparation() {
        let source = ShaderPackSource::new(
            "unresolved-scalar-source",
            32,
            vec![
                ShaderSourceFile::new(
                    "gbuffers_terrain.vsh",
                    "#version 130\nuniform float packSpecificValue;\nout vec2 texCoord;\nout vec4 glColor;\nout float smoothnessD;\nout float materialMask;\nout float skyLightFactor;\nuniform sampler2D tex;\nvoid main() { texCoord = gl_MultiTexCoord0.xy; glColor = vec4(1.0); smoothnessD = 0.0; materialMask = 0.0; skyLightFactor = 1.0; gl_Position = ftransform() + vec4(packSpecificValue); }",
                ),
                ShaderSourceFile::new(
                    "gbuffers_terrain.fsh",
                    "#version 130\nin vec2 texCoord;\nin vec4 glColor;\nin float smoothnessD;\nin float materialMask;\nin float skyLightFactor;\nuniform sampler2D tex;\nvoid DoLighting() {}\n/* DRAWBUFFERS:06 */\nvoid main() { vec4 color = texture2D(tex, texCoord); if (color.a <= 0.00001) discard; color.rgb *= glColor.rgb; DoLighting(); gl_FragData[0] = color; gl_FragData[1] = vec4(smoothnessD, materialMask, skyLightFactor, 1.0); }",
                ),
                ShaderSourceFile::new(TERRAIN_RESOURCE_BINDINGS_PATH, "tex=material_atlas\n"),
                ShaderSourceFile::new("lib/common.glsl", "#define TEST 1\n"),
                ShaderSourceFile::new("shaders.properties", ""),
                ShaderSourceFile::new("block.properties", ""),
            ],
        )
        .unwrap();
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();
        executor.observe_source_candidate(&source);

        assert!(matches!(
            executor.source_candidate(),
            TerrainSourceCandidateState::Discovered {
                source_uniform_requirement_summary: Some(summary),
                source_uniform_requirement_error: None,
                ..
            } if summary.field_count == 3
                && summary.resolved_field_count == 2
                && summary.unresolved_field_names == vec!["packSpecificValue".to_string()]
        ));
        for error in [
            executor
                .prepared_lowered_terrain_source_program(TerrainMaterialProgramKind::Opaque)
                .unwrap_err()
                .to_string(),
            executor
                .candidate_fixture_terrain_program(TerrainMaterialProgramKind::Opaque, 0)
                .unwrap_err()
                .to_string(),
        ] {
            assert!(
                error.contains("unresolved scalar source uniforms"),
                "{error}"
            );
            assert!(error.contains("packSpecificValue"), "{error}");
        }
    }

    #[test]
    fn explicit_empty_source_is_observed_as_disabled_not_rejected() {
        let source = ShaderPackSource::new("disabled", 17, Vec::new()).unwrap();
        let mut runtime = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(3).unwrap();

        runtime.observe_source_candidate(&source);

        assert!(matches!(
            runtime.source_candidate(),
            TerrainSourceCandidateState::Disabled { generation: 17, pack_name }
                if pack_name == "disabled"
        ));
        assert_eq!(
            "vulkanic:builtin/terrain_opaque_v1",
            runtime.plan().programs.terrain_opaque.identity.as_str()
        );
        assert_eq!(None, runtime.candidate_shader_binding(0).unwrap());
    }

    #[test]
    fn discovered_source_candidate_without_pack_version_uses_owned_target_version() {
        let source = ShaderPackSource::new(
            "bounded-source-candidate",
            23,
            vec![
                ShaderSourceFile::new(
                    "program/gbuffers_terrain.glsl",
                    "void DoLighting() {}\n/* DRAWBUFFERS:06 */\nvoid main() { vec4 color = texture2D(tex, texCoord); if (color.a <= 0.00001) discard; color.rgb *= glColor.rgb; DoLighting(); gl_FragData[0] = color; gl_FragData[1] = vec4(smoothnessD, materialMask, skyLightFactor, 1.0); }",
                ),
                ShaderSourceFile::new(TERRAIN_RESOURCE_BINDINGS_PATH, "tex=material_atlas\n"),
                ShaderSourceFile::new("lib/common.glsl", "#define TEST 1\n"),
                ShaderSourceFile::new("shaders.properties", ""),
                ShaderSourceFile::new("block.properties", ""),
            ],
        )
        .unwrap();
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();
        executor.observe_source_candidate(&source);

        let program = executor
            .candidate_fixture_terrain_program(TerrainMaterialProgramKind::Opaque, 0)
            .unwrap()
            .expect("the owned lowering supplies GLSL 450 when the pack leaves version to Iris");
        assert!(program.program.vertex.source.starts_with("#version 450\n"));
        assert!(program
            .program
            .fragment
            .source
            .starts_with("#version 450\n"));
        assert!(matches!(
            executor.source_candidate(),
            TerrainSourceCandidateState::Discovered {
                source_summary: Some(summary),
                source_preprocess_error: None,
                source_lowering_summary: Some(_),
                source_lowering_error: None,
                ..
            } if summary.vertex_entry == "program/gbuffers_terrain.glsl"
                && summary.fragment_entry == "program/gbuffers_terrain.glsl"
        ));
    }

    #[test]
    fn fragment_only_source_candidate_cannot_prepare_a_selected_program() {
        let source = ShaderPackSource::new(
            "fragment-only",
            24,
            vec![
                ShaderSourceFile::new(
                    "gbuffers_terrain.fsh",
                    "void DoLighting() {}\n/* DRAWBUFFERS:06 */\nvoid main() { vec4 color = texture2D(tex, texCoord); if (color.a <= 0.00001) discard; color.rgb *= glColor.rgb; DoLighting(); gl_FragData[0] = color; gl_FragData[1] = vec4(smoothnessD, materialMask, skyLightFactor, 1.0); }",
                ),
                ShaderSourceFile::new("lib/common.glsl", "#define TEST 1\n"),
                ShaderSourceFile::new("shaders.properties", ""),
                ShaderSourceFile::new("block.properties", ""),
            ],
        )
        .unwrap();
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();
        executor.observe_source_candidate(&source);

        assert!(matches!(
            executor.source_candidate(),
            TerrainSourceCandidateState::Discovered {
                source_summary: None,
                source_preprocess_error: Some(error),
                ..
            } if error.contains("missing shader source gbuffers_terrain.vsh")
        ));
        assert!(executor
            .candidate_fixture_terrain_program(TerrainMaterialProgramKind::Opaque, 0)
            .unwrap_err()
            .to_string()
            .contains("no complete paired-stage source contract"));
    }

    #[test]
    fn colored_source_candidate_prepares_owned_volume_identity_from_semantics() {
        let source = bundled_complementary_hung_loified_source(29).unwrap();
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();
        executor.observe_source_candidate(&source);

        let preparation = executor
            .candidate_colored_light_preparation(41, 53, [-1.25, 64.5, 7.75])
            .unwrap()
            .expect("Complementary source should prepare ColoredVoxelLighting semantics");

        assert_eq!(29, preparation.descriptor.shader_pack_generation);
        assert_eq!(41, preparation.descriptor.world_generation);
        assert_eq!(53, preparation.descriptor.resource_generation);
        assert_eq!([-2, 64, 7], preparation.descriptor.mapping.camera_cell);
        assert_eq!(
            [0.75, 0.5, 0.75],
            preparation.descriptor.mapping.camera_fraction
        );
        assert_eq!(29, preparation.materials.shader_pack_generation());
        assert_eq!(29, preparation.emission.shader_pack_generation());
        assert!(executor
            .candidate_colored_light_preparation(0, 53, [0.0; 3])
            .is_err());
    }

    #[test]
    fn prepared_colored_light_generation_installs_once_and_reuses_matching_descriptor() {
        let source = bundled_complementary_hung_loified_source(31).unwrap();
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();
        executor.observe_source_candidate(&source);
        let preparation = executor
            .candidate_colored_light_preparation(43, 59, [0.25, 64.5, 0.75])
            .unwrap()
            .unwrap();
        let mut gal = gal();

        assert!(executor
            .ensure_candidate_colored_light_runtime(&mut gal, preparation.clone())
            .unwrap());
        assert!(!executor
            .ensure_candidate_colored_light_runtime(&mut gal, preparation)
            .unwrap());
        let fractional = executor
            .candidate_colored_light_preparation(43, 59, [0.75, 64.5, 0.25])
            .unwrap()
            .unwrap();
        assert!(!executor
            .ensure_candidate_colored_light_runtime(&mut gal, fractional)
            .unwrap());
        assert!(executor.has_private_terrain_occupancy());
        assert_eq!(
            31,
            executor
                .private_terrain_occupancy_descriptor()
                .unwrap()
                .shader_pack_generation
        );
    }

    #[test]
    fn incomplete_owned_source_is_rejected_without_changing_fixture_execution() {
        let source = ShaderPackSource::new(
            "incomplete",
            14,
            vec![ShaderSourceFile::new(
                "program/gbuffers_terrain.glsl",
                "void main() {}",
            )],
        )
        .unwrap();
        let mut executor = ShaderPackRuntimeExecutor::terrain_material_multipass_v1(7).unwrap();
        let fixture_program = executor.plan().programs.terrain_opaque.identity.clone();

        executor.observe_source_candidate(&source);

        assert!(matches!(
            executor.source_candidate(),
            TerrainSourceCandidateState::Rejected {
                generation: 14,
                pack_name,
                ..
            } if pack_name == "incomplete"
        ));
        assert_eq!(
            fixture_program,
            executor.plan().programs.terrain_opaque.identity
        );
    }

    #[test]
    fn composite_uniform_block_layout_is_stable() {
        let uniforms = TerrainCompositeUniforms {
            light_view_projection: [1.0; 16],
            shadow_params: [2.0; 4],
            color_grade_params: [3.0; 4],
            projection_inverse: [4.0; 16],
            fog_color_and_environmental_start: [5.0; 4],
            fog_ranges: [6.0; 4],
        };
        assert_eq!(
            TERRAIN_RUNTIME_COMPOSITE_UNIFORM_BYTES as usize,
            uniforms.pack().len()
        );
    }

    #[test]
    fn indexed_draw_emission_skips_redundant_state_binds_inside_pass() {
        let pipeline = test_handle(HandleKind::GraphicsPipeline, 1);
        let other_pipeline = test_handle(HandleKind::GraphicsPipeline, 2);
        let layout = test_handle(HandleKind::PipelineLayout, 3);
        let set = test_handle(HandleKind::ResourceSet, 4);
        let other_set = test_handle(HandleKind::ResourceSet, 5);
        let index_buffer = test_handle(HandleKind::Buffer, 6);
        let mut ops = Vec::new();
        let mut state = IndexedDrawState::default();

        append_indexed_draw(
            &mut ops,
            &mut state,
            pipeline,
            layout,
            set,
            &[0],
            None,
            index_buffer,
            0,
            IndexType::U32,
            6,
            1,
        );
        append_indexed_draw(
            &mut ops,
            &mut state,
            pipeline,
            layout,
            set,
            &[0],
            None,
            index_buffer,
            0,
            IndexType::U32,
            6,
            2,
        );
        append_indexed_draw(
            &mut ops,
            &mut state,
            pipeline,
            layout,
            other_set,
            &[0],
            None,
            index_buffer,
            0,
            IndexType::U32,
            6,
            3,
        );
        append_indexed_draw(
            &mut ops,
            &mut state,
            pipeline,
            layout,
            other_set,
            &[0],
            None,
            index_buffer,
            12,
            IndexType::U32,
            6,
            4,
        );
        append_indexed_draw(
            &mut ops,
            &mut state,
            other_pipeline,
            layout,
            other_set,
            &[0],
            None,
            index_buffer,
            12,
            IndexType::U32,
            6,
            5,
        );

        let pipeline_binds = ops
            .iter()
            .filter(|op| matches!(op, CommandOp::BindGraphicsPipeline(_)))
            .count();
        let resource_set_binds = ops
            .iter()
            .filter(|op| matches!(op, CommandOp::BindResourceSet { .. }))
            .count();
        let index_binds = ops
            .iter()
            .filter(|op| matches!(op, CommandOp::SetIndexBuffer { .. }))
            .count();
        let draws = ops
            .iter()
            .filter(|op| matches!(op, CommandOp::DrawIndexed { .. }))
            .count();

        assert_eq!(2, pipeline_binds);
        assert_eq!(3, resource_set_binds);
        assert_eq!(2, index_binds);
        assert_eq!(5, draws);
    }

    #[test]
    fn indexed_draw_emission_distinguishes_static_and_dynamic_set_bindings() {
        let pipeline = test_handle(HandleKind::GraphicsPipeline, 1);
        let layout = test_handle(HandleKind::PipelineLayout, 2);
        let set = test_handle(HandleKind::ResourceSet, 3);
        let index_buffer = test_handle(HandleKind::Buffer, 4);
        let mut ops = Vec::new();
        let mut state = IndexedDrawState::default();

        append_indexed_draw(
            &mut ops,
            &mut state,
            pipeline,
            layout,
            set,
            &[],
            None,
            index_buffer,
            0,
            IndexType::U32,
            6,
            1,
        );
        append_indexed_draw(
            &mut ops,
            &mut state,
            pipeline,
            layout,
            set,
            &[32],
            None,
            index_buffer,
            0,
            IndexType::U32,
            6,
            1,
        );
        append_indexed_draw(
            &mut ops,
            &mut state,
            pipeline,
            layout,
            set,
            &[32],
            None,
            index_buffer,
            0,
            IndexType::U32,
            6,
            1,
        );

        let dynamic_offsets = ops
            .iter()
            .filter_map(|op| match op {
                CommandOp::BindResourceSet {
                    set_index: 0,
                    dynamic_offsets,
                    ..
                } => Some(dynamic_offsets.as_slice()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(vec![&[][..], &[32][..]], dynamic_offsets);
    }

    #[test]
    fn indexed_draw_emission_binds_optional_shader_resources_by_semantic_set() {
        let pipeline = test_handle(HandleKind::GraphicsPipeline, 1);
        let layout = test_handle(HandleKind::PipelineLayout, 2);
        let mesh_set = test_handle(HandleKind::ResourceSet, 3);
        let shader_set = test_handle(HandleKind::ResourceSet, 4);
        let replacement_shader_set = test_handle(HandleKind::ResourceSet, 5);
        let index_buffer = test_handle(HandleKind::Buffer, 6);
        let shader_binding = TerrainShaderResourceSet {
            set_index: 1,
            set: shader_set,
        };
        let mut ops = Vec::new();
        let mut state = IndexedDrawState::default();
        for binding in [
            Some(shader_binding),
            Some(shader_binding),
            None,
            Some(TerrainShaderResourceSet {
                set_index: 1,
                set: replacement_shader_set,
            }),
        ] {
            append_indexed_draw(
                &mut ops,
                &mut state,
                pipeline,
                layout,
                mesh_set,
                &[0],
                binding,
                index_buffer,
                0,
                IndexType::U32,
                6,
                1,
            );
        }
        let shader_binds = ops
            .iter()
            .filter(|op| matches!(op, CommandOp::BindResourceSet { set_index: 1, .. }))
            .count();
        assert_eq!(2, shader_binds);
        assert!(ops.iter().any(|op| {
            matches!(op, CommandOp::BindResourceSet { set_index: 1, set, .. } if *set == replacement_shader_set)
        }));
    }

    fn test_handle(kind: HandleKind, index: u32) -> Handle {
        Handle::new(kind, index, 1).unwrap()
    }

    #[test]
    fn textured_material_direct_draw_expands_compact_quads_without_an_index_buffer() {
        let pipeline = test_handle(HandleKind::GraphicsPipeline, 100);
        let layout = test_handle(HandleKind::PipelineLayout, 101);
        let source_set = test_handle(HandleKind::ResourceSet, 102);
        let pack_set = test_handle(HandleKind::ResourceSet, 103);
        let mut operations = Vec::new();
        let mut state = DirectDrawState::default();
        append_direct_draw(
            &mut operations,
            &mut state,
            pipeline,
            layout,
            source_set,
            &[64, 128, 256],
            Some(TerrainShaderResourceSet {
                set_index: 1,
                set: pack_set,
            }),
            12,
        );
        assert!(operations.iter().any(|operation| {
            matches!(
                operation,
                CommandOp::Draw {
                    vertices: 12,
                    instances: 1
                }
            )
        }));
        assert!(!operations.iter().any(|operation| matches!(
            operation,
            CommandOp::SetIndexBuffer { .. } | CommandOp::DrawIndexed { .. }
        )));
        assert!(operations.iter().any(|operation| {
            matches!(operation, CommandOp::BindResourceSet { set_index: 1, set, .. } if *set == pack_set)
        }));
    }

    #[test]
    fn main_depth_history_first_frame_copies_only_current_depth() {
        let targets = TerrainDepthHistoryTargets {
            main_depth_texture: test_handle(HandleKind::Texture, 1),
            before_translucency_texture: test_handle(HandleKind::Texture, 2),
            previous_texture: test_handle(HandleKind::Texture, 3),
        };
        let mut ops = Vec::new();
        ShaderPackRuntimeExecutor::append_main_depth_history(
            &mut ops,
            targets,
            TerrainDepthHistoryPlan {
                prior_before_translucency_valid: false,
                prior_previous_valid: false,
                extent: Extent3d {
                    width: 320,
                    height: 180,
                    depth: 1,
                },
            },
        )
        .unwrap();

        let copies = ops
            .iter()
            .filter_map(|op| match op {
                CommandOp::CopyTexture(copy) => Some(copy),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(1, copies.len());
        assert_eq!(targets.main_depth_texture, copies[0].src_texture);
        assert_eq!(targets.before_translucency_texture, copies[0].dst_texture);
        assert!(ops.iter().all(|op| !matches!(
            op,
            CommandOp::CopyTexture(copy) if copy.dst_texture == targets.previous_texture
        )));
    }

    #[test]
    fn main_depth_history_rotates_confirmed_snapshot_before_replacement() {
        let targets = TerrainDepthHistoryTargets {
            main_depth_texture: test_handle(HandleKind::Texture, 1),
            before_translucency_texture: test_handle(HandleKind::Texture, 2),
            previous_texture: test_handle(HandleKind::Texture, 3),
        };
        let mut ops = Vec::new();
        ShaderPackRuntimeExecutor::append_main_depth_history(
            &mut ops,
            targets,
            TerrainDepthHistoryPlan {
                prior_before_translucency_valid: true,
                prior_previous_valid: true,
                extent: Extent3d {
                    width: 320,
                    height: 180,
                    depth: 1,
                },
            },
        )
        .unwrap();

        let copies = ops
            .iter()
            .filter_map(|op| match op {
                CommandOp::CopyTexture(copy) => Some((copy.src_texture, copy.dst_texture)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            vec![
                (
                    targets.before_translucency_texture,
                    targets.previous_texture,
                ),
                (
                    targets.main_depth_texture,
                    targets.before_translucency_texture,
                ),
            ],
            copies
        );
        assert!(ops.iter().any(|op| matches!(
            op,
            CommandOp::Barrier(barrier)
                if barrier.resource == targets.previous_texture
                    && barrier.before == TextureUsageState::ShaderRead
                    && barrier.after == TextureUsageState::TransferDst
        )));
    }

    #[test]
    fn main_depth_history_rejects_non_2d_extent() {
        let targets = TerrainDepthHistoryTargets {
            main_depth_texture: test_handle(HandleKind::Texture, 1),
            before_translucency_texture: test_handle(HandleKind::Texture, 2),
            previous_texture: test_handle(HandleKind::Texture, 3),
        };
        let error = ShaderPackRuntimeExecutor::append_main_depth_history(
            &mut Vec::new(),
            targets,
            TerrainDepthHistoryPlan {
                prior_before_translucency_valid: false,
                prior_previous_valid: false,
                extent: Extent3d {
                    width: 1,
                    height: 1,
                    depth: 2,
                },
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("non-zero 2D extent"));
    }
}
