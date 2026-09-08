//! Bounded source lowering from legacy terrain fragment syntax to explicit
//! shader-pack semantics.
//!
//! This is intentionally a source transform, not an Iris compatibility layer:
//! it owns text copied from the selected pack and emits GLSL 450 with named
//! terrain outputs. Remaining compatibility syntax is reported by dialect
//! preflight and keeps execution unavailable until a complete lowering exists.

use std::collections::{BTreeMap, BTreeSet};

use crate::render::vulkanic::error::{GalError, GalResult};

use super::dialect::{analyze_glsl_text, GlslDialectReport};
use super::preprocess::PreprocessedShaderSource;
use super::terrain_contract::parse_draw_buffers_slots;
use super::terrain_source_resources::{TerrainSourceResourceBindings, TerrainSourceResourceRole};

/// Raster primitive semantics supplied by a Rust-owned procedural source
/// stage. This selects only owned geometry; it never borrows an Iris vertex
/// stream or backend state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FullscreenSourceRasterPrimitive {
    /// The standard three-vertex coverage triangle for deferred, composite,
    /// and final source stages.
    FullscreenTriangle,
    /// Vanilla's top sky disc expanded from its triangle fan into eight owned
    /// wedges. This preserves the source program's real geometric depth field
    /// for `gl_FragCoord.z` ray reconstruction.
    VanillaSkyDisc,
    /// Vanilla's sun/moon quad geometry, reconstructed from copied sky
    /// semantics and source-pack configuration. It owns both positions and
    /// UVs; no SkyRenderer buffer, Iris vertex format, or native state is
    /// borrowed by the selected source route.
    VanillaCelestialQuad,
}

impl FullscreenSourceRasterPrimitive {
    pub const fn vertex_count(self) -> u32 {
        match self {
            Self::FullscreenTriangle => 3,
            Self::VanillaSkyDisc => 24,
            Self::VanillaCelestialQuad => 6,
        }
    }
}

/// Named terrain outputs recovered from the audited `DRAWBUFFERS:06` source
/// contract. The identifiers are semantic; only a later pass description maps
/// them to a concrete attachment set.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerrainFragmentOutput {
    LitColor,
    MaterialAuxiliary,
    ViewSpaceNormal,
}

/// Named outputs of the source pack's distinct translucent terrain stage.
/// The source locations are retained only while rewriting legacy GLSL; later
/// pass construction consumes the semantic names and never raw DRAWBUFFERS
/// indices.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TranslucentTerrainFragmentOutput {
    LitColor,
    TranslucencyAuxiliary,
    MaterialAuxiliary,
}

/// Named outputs of `gbuffers_textured`. These have the same GLSL output
/// locations as the selected pack's textured pass, but they are not terrain
/// normals: the final lane carries the pass's translucency auxiliary data.
/// Keeping this separate from both terrain and water prevents a future
/// source-material writer from binding an otherwise compatible target with
/// the wrong semantic interpretation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TexturedMaterialFragmentOutput {
    LitColor,
    MaterialAuxiliary,
    TranslucencyAuxiliary,
}

/// The weather stage writes a single lit scene-color output. It is distinct
/// from terrain, water, and generic textured material outputs so later pass
/// scheduling cannot reinterpret its source slot as a G-buffer attachment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WeatherFragmentOutput {
    LitColor,
}

/// Named outputs of the source pack's distinct vanilla cloud stage. Clouds
/// share `DRAWBUFFERS:063` with generic textured material, but keep their own
/// names so a future writer cannot bind them as terrain or an overlay.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloudFragmentOutput {
    LitColor,
    MaterialAuxiliary,
    TranslucencyAuxiliary,
}

impl TexturedMaterialFragmentOutput {
    fn legacy_index(self) -> u32 {
        match self {
            Self::LitColor => 0,
            Self::MaterialAuxiliary => 1,
            Self::TranslucencyAuxiliary => 2,
        }
    }

    fn semantic_name(self) -> &'static str {
        match self {
            Self::LitColor => "out_textured_material_lit_color",
            Self::MaterialAuxiliary => "out_textured_material_auxiliary",
            Self::TranslucencyAuxiliary => "out_textured_material_translucency_auxiliary",
        }
    }
}

impl WeatherFragmentOutput {
    fn legacy_index(self) -> u32 {
        match self {
            Self::LitColor => 0,
        }
    }

    fn semantic_name(self) -> &'static str {
        match self {
            Self::LitColor => "out_weather_lit_color",
        }
    }
}

impl CloudFragmentOutput {
    fn legacy_index(self) -> u32 {
        match self {
            Self::LitColor => 0,
            Self::MaterialAuxiliary => 1,
            Self::TranslucencyAuxiliary => 2,
        }
    }

    fn semantic_name(self) -> &'static str {
        match self {
            Self::LitColor => "out_cloud_lit_color",
            Self::MaterialAuxiliary => "out_cloud_material_auxiliary",
            Self::TranslucencyAuxiliary => "out_cloud_translucency_auxiliary",
        }
    }
}

impl TranslucentTerrainFragmentOutput {
    fn legacy_index(self) -> u32 {
        match self {
            Self::LitColor => 0,
            Self::TranslucencyAuxiliary => 1,
            Self::MaterialAuxiliary => 2,
        }
    }

    fn semantic_name(self) -> &'static str {
        match self {
            Self::LitColor => "out_terrain_lit_color",
            Self::TranslucencyAuxiliary => "out_terrain_translucency_auxiliary",
            Self::MaterialAuxiliary => "out_terrain_material_auxiliary",
        }
    }
}

impl TerrainFragmentOutput {
    fn legacy_index(self) -> u32 {
        match self {
            Self::LitColor => 0,
            Self::MaterialAuxiliary => 1,
            Self::ViewSpaceNormal => 2,
        }
    }

    fn semantic_name(self) -> &'static str {
        match self {
            Self::LitColor => "out_terrain_lit_color",
            Self::MaterialAuxiliary => "out_terrain_material_auxiliary",
            Self::ViewSpaceNormal => "out_terrain_view_space_normal",
        }
    }
}

/// Named outputs from a source-derived shadow fragment. These are distinct
/// from terrain G-buffer outputs: a later shadow pass maps them to owned
/// shadow attachments rather than reusing a terrain attachment by index.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShadowFragmentOutput {
    ShadowColor,
    LightShaftColor,
}

/// The Distant Horizons terrain source writes one lit color target. Its depth
/// is the explicit depth attachment of the DH pass, not a second fragment
/// output. Keeping this distinct from ordinary terrain's G-buffer outputs
/// prevents source lowering from silently changing pack composition rules.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DistantHorizonsFragmentOutput {
    LitColor,
}

impl DistantHorizonsFragmentOutput {
    fn legacy_index(self) -> u32 {
        match self {
            Self::LitColor => 0,
        }
    }

    fn semantic_name(self) -> &'static str {
        match self {
            Self::LitColor => "out_distant_horizons_lit_color",
        }
    }
}

impl ShadowFragmentOutput {
    fn legacy_index(self) -> u32 {
        match self {
            Self::ShadowColor => 0,
            Self::LightShaftColor => 1,
        }
    }

    fn semantic_name(self) -> &'static str {
        match self {
            Self::ShadowColor => "out_shadow_color",
            Self::LightShaftColor => "out_shadow_light_shaft_color",
        }
    }
}

/// Owned intermediate source from one lowering step. It does not indicate
/// executable readiness and has no backend-specific binding information.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoweredTerrainFragmentSource {
    entry_path: String,
    source: String,
    outputs: Vec<TerrainFragmentOutput>,
    remaining_dialect: GlslDialectReport,
}

/// Owned source for the separate translucent terrain fragment stage. It is a
/// preparation artifact only; it carries no framebuffer, blend state, or
/// backend object.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoweredTranslucentTerrainFragmentSource {
    entry_path: String,
    source: String,
    outputs: Vec<TranslucentTerrainFragmentOutput>,
    remaining_dialect: GlslDialectReport,
}

/// Owned fragment source for the generic `gbuffers_textured` pass. This is
/// source preparation only; the named target writer remains a separate,
/// explicitly admitted runtime slice.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoweredTexturedMaterialFragmentSource {
    entry_path: String,
    source: String,
    outputs: Vec<TexturedMaterialFragmentOutput>,
    remaining_dialect: GlslDialectReport,
}

/// Owned source for the selected weather fragment. It remains backend-neutral
/// source preparation: pass targets, blending, and execution are separate
/// runtime responsibilities.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoweredWeatherFragmentSource {
    entry_path: String,
    source: String,
    outputs: Vec<WeatherFragmentOutput>,
    remaining_dialect: GlslDialectReport,
}

/// Owned fragment source for the selected vanilla cloud stage. It owns no
/// target or pipeline and remains distinct from generic material semantics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoweredCloudFragmentSource {
    entry_path: String,
    source: String,
    outputs: Vec<CloudFragmentOutput>,
    remaining_dialect: GlslDialectReport,
}

/// Owned shadow fragment source after the bounded output lowering step. It
/// has no pipeline, attachment, or backend binding; those require a later
/// complete source shadow-pass contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoweredShadowFragmentSource {
    entry_path: String,
    source: String,
    outputs: Vec<ShadowFragmentOutput>,
    remaining_dialect: GlslDialectReport,
}

/// Owned DH fragment source after its legacy output has been given a named
/// semantic meaning. It is intentionally not a terrain G-buffer program and
/// remains unexecutable until the DH color/depth consumer is fully owned.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoweredDistantHorizonsFragmentSource {
    entry_path: String,
    source: String,
    outputs: Vec<DistantHorizonsFragmentOutput>,
    remaining_dialect: GlslDialectReport,
}

/// One manifest-derived semantic target written by a source fullscreen stage.
/// `source_location` is retained only to lower the source text; scheduling and
/// attachment ownership use `role`, never a raw legacy draw-buffer number.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FullscreenSourceFragmentOutput {
    /// GLSL `gl_FragData` ordinal, preserved as the explicit output location
    /// of the lowered shader.
    source_location: u32,
    /// Semantic shader-pack color destination recovered from the source
    /// `DRAWBUFFERS` declaration. This is deliberately distinct from the GLSL
    /// output location: `gl_FragData[0]` may target `colortex3`.
    source_slot: u32,
    role: TerrainSourceResourceRole,
    semantic_name: String,
}

impl FullscreenSourceFragmentOutput {
    pub fn source_location(&self) -> u32 {
        self.source_location
    }

    pub fn source_slot(&self) -> u32 {
        self.source_slot
    }

    pub fn role(&self) -> TerrainSourceResourceRole {
        self.role.clone()
    }

    pub fn semantic_name(&self) -> &str {
        &self.semantic_name
    }
}

/// Owned fullscreen fragment source after legacy outputs have been mapped to
/// pack-declared semantic color roles. It is intentionally distinct from the
/// terrain-mesh fragment types: a fullscreen stage must not inherit a mesh
/// vertex ABI merely because it is a later shader-pack consumer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoweredFullscreenSourceFragment {
    entry_path: String,
    source: String,
    outputs: Vec<FullscreenSourceFragmentOutput>,
    remaining_dialect: GlslDialectReport,
}

/// Rust-owned source for a standard fullscreen semantic triangle/quad input.
/// The only fixed stream is position plus UV; this source carries no Java,
/// Iris, OpenGL, or backend object identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoweredFullscreenSourceVertex {
    entry_path: String,
    source: String,
    remaining_dialect: GlslDialectReport,
}

/// Owned GLSL 450 vertex source after legacy terrain names have been mapped to
/// the future explicit source-vertex stream. The stream binding is deliberately
/// private preparation: no current render route allocates or binds it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoweredTerrainVertexSource {
    entry_path: String,
    source: String,
    remaining_dialect: GlslDialectReport,
}

/// Deterministic scalar/vector/matrix uniform layout shared by the two source
/// stages. Samplers and images remain distinct named resources for a later
/// lowering step.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerrainSourceUniformContract {
    declarations: Vec<String>,
    fields: Vec<TerrainSourceUniformField>,
    std140_size: u32,
}

impl TerrainSourceUniformContract {
    pub fn declarations(&self) -> &[String] {
        &self.declarations
    }

    /// Deterministic std140 layout for the source-declared scalar/vector/
    /// matrix uniforms. The layout is semantic source preparation only; no
    /// Java or backend state participates in its construction.
    pub fn fields(&self) -> &[TerrainSourceUniformField] {
        &self.fields
    }

    pub fn std140_size(&self) -> u32 {
        self.std140_size
    }
}

/// Bounded GLSL value categories supported in the selected terrain source
/// scalar block. Opaque uniforms are represented separately by the semantic
/// resource binding plan and never appear here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerrainSourceUniformType {
    Float,
    Int,
    Uint,
    Bool,
    Vec2,
    Vec3,
    Vec4,
    IVec2,
    IVec3,
    IVec4,
    UVec2,
    UVec3,
    UVec4,
    Mat2,
    Mat3,
    Mat4,
}

impl TerrainSourceUniformType {
    fn from_glsl(type_name: &str) -> GalResult<Self> {
        match type_name {
            "float" => Ok(Self::Float),
            "int" => Ok(Self::Int),
            "uint" => Ok(Self::Uint),
            "bool" => Ok(Self::Bool),
            "vec2" => Ok(Self::Vec2),
            "vec3" => Ok(Self::Vec3),
            "vec4" => Ok(Self::Vec4),
            "ivec2" => Ok(Self::IVec2),
            "ivec3" => Ok(Self::IVec3),
            "ivec4" => Ok(Self::IVec4),
            "uvec2" => Ok(Self::UVec2),
            "uvec3" => Ok(Self::UVec3),
            "uvec4" => Ok(Self::UVec4),
            "mat2" => Ok(Self::Mat2),
            "mat3" => Ok(Self::Mat3),
            "mat4" => Ok(Self::Mat4),
            _ => Err(GalError::unsupported_feature(format!(
                "terrain source scalar uniform type '{type_name}' has no std140 semantic layout"
            ))),
        }
    }

    fn std140_alignment(self) -> u32 {
        match self {
            Self::Float | Self::Int | Self::Uint | Self::Bool => 4,
            Self::Vec2 | Self::IVec2 | Self::UVec2 => 8,
            Self::Vec3
            | Self::Vec4
            | Self::IVec3
            | Self::IVec4
            | Self::UVec3
            | Self::UVec4
            | Self::Mat2
            | Self::Mat3
            | Self::Mat4 => 16,
        }
    }

    fn std140_size(self) -> u32 {
        match self {
            Self::Float | Self::Int | Self::Uint | Self::Bool => 4,
            Self::Vec2 | Self::IVec2 | Self::UVec2 => 8,
            // A standalone vec3 has 16-byte alignment but occupies three
            // components. std140 permits a following scalar to use the
            // fourth component; arrays still round their stride to 16 below.
            // Treating vec3 as a 16-byte payload shifts every later scalar
            // field from the first vec3 onward.
            Self::Vec3 | Self::IVec3 | Self::UVec3 => 12,
            Self::Vec4 | Self::IVec4 | Self::UVec4 => 16,
            Self::Mat2 => 32,
            Self::Mat3 => 48,
            Self::Mat4 => 64,
        }
    }
}

/// One source scalar uniform after deterministic std140 layout. Array values
/// carry their required 16-byte rounded stride; scalar values use a zero
/// stride to avoid pretending they are arrays.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerrainSourceUniformField {
    name: String,
    ty: TerrainSourceUniformType,
    array_length: u32,
    offset: u32,
    size: u32,
    array_stride: u32,
}

impl TerrainSourceUniformField {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn ty(&self) -> TerrainSourceUniformType {
        self.ty
    }

    pub fn array_length(&self) -> u32 {
        self.array_length
    }

    pub fn offset(&self) -> u32 {
        self.offset
    }

    pub fn size(&self) -> u32 {
        self.size
    }

    pub fn array_stride(&self) -> u32 {
        self.array_stride
    }
}

/// One source-derived vertex-to-fragment field. Locations are assigned from a
/// stable semantic sort, never from Java/Iris attribute state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerrainSourceVaryingField {
    name: String,
    type_name: String,
    interpolation: String,
    location: u32,
}

impl TerrainSourceVaryingField {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn type_name(&self) -> &str {
        &self.type_name
    }

    pub fn interpolation(&self) -> &str {
        &self.interpolation
    }

    pub fn location(&self) -> u32 {
        self.location
    }
}

/// Deterministic interface between the selected terrain source stages. This
/// covers only simple scalar/vector fields today; arrays, matrices, and other
/// multi-location interfaces are rejected instead of guessing a layout.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerrainSourceVaryingContract {
    fields: Vec<TerrainSourceVaryingField>,
}

/// Source-level opaque resource category. This is intentionally not a Vulkan
/// descriptor or OpenGL binding; a later runtime maps these semantic fields to
/// explicit GAL resources.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerrainSourceOpaqueResourceKind {
    /// GLSL sampler declarations combine the image view and sampler state.
    /// The eventual runtime maps this to one semantic GAL pair binding.
    CombinedTextureSampler,
    StorageImage,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerrainSourceOpaqueResource {
    name: String,
    type_name: String,
    qualifiers: String,
    kind: TerrainSourceOpaqueResourceKind,
    binding: u32,
    active: bool,
}

impl TerrainSourceOpaqueResource {
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn type_name(&self) -> &str {
        &self.type_name
    }
    pub fn qualifiers(&self) -> &str {
        &self.qualifiers
    }
    pub fn kind(&self) -> TerrainSourceOpaqueResourceKind {
        self.kind
    }

    pub fn binding(&self) -> u32 {
        self.binding
    }

    /// Whether the already-expanded vertex or fragment stage actually
    /// references this declaration. Declarations remain in the lowered
    /// source with deterministic bindings, but only active resources belong
    /// to the executable semantic resource contract.
    pub fn active(&self) -> bool {
        self.active
    }
}

/// One source resource paired with a pack-declared portable role. The binding
/// remains a lowering-local ordering value; backends will later receive only
/// a resource layout assembled from these semantic roles.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerrainSourceOpaqueResourceBinding {
    resource_name: String,
    role: TerrainSourceResourceRole,
    kind: TerrainSourceOpaqueResourceKind,
    qualifiers: String,
    binding: u32,
}

impl TerrainSourceOpaqueResourceBinding {
    pub fn resource_name(&self) -> &str {
        &self.resource_name
    }

    pub fn role(&self) -> TerrainSourceResourceRole {
        self.role.clone()
    }

    pub fn kind(&self) -> TerrainSourceOpaqueResourceKind {
        self.kind
    }

    pub fn qualifiers(&self) -> &str {
        &self.qualifiers
    }

    pub fn binding(&self) -> u32 {
        self.binding
    }
}

/// Complete portable binding plan for all opaque resources in a lowered
/// terrain pair. It deliberately cannot be constructed from source names
/// alone: every resource must have a pack-declared semantic role.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerrainSourceOpaqueResourceBindingPlan {
    bindings: Vec<TerrainSourceOpaqueResourceBinding>,
}

impl TerrainSourceOpaqueResourceBindingPlan {
    pub fn bindings(&self) -> &[TerrainSourceOpaqueResourceBinding] {
        &self.bindings
    }

    /// Resolves only an active, lowered source resource name. Raw pack
    /// declarations may include inactive samplers and must not allocate a
    /// runtime resource by themselves.
    pub fn role_for(&self, resource_name: &str) -> Option<TerrainSourceResourceRole> {
        self.bindings
            .iter()
            .find(|binding| binding.resource_name == resource_name)
            .map(TerrainSourceOpaqueResourceBinding::role)
    }

    /// Reclassifies one already-declared active sampler at a pass-specific
    /// semantic boundary. This is deliberately narrow: source names remain
    /// pack data, but a legacy pack-wide `tex=material_atlas` declaration
    /// cannot force an entity pass to borrow terrain atlas ownership.
    pub fn with_sampled_role_override(
        &self,
        resource_name: &str,
        expected_role: TerrainSourceResourceRole,
        replacement_role: TerrainSourceResourceRole,
    ) -> GalResult<Self> {
        if expected_role.expected_sampler_type() != replacement_role.expected_sampler_type()
            || expected_role.expected_sampled_resource_shape()
                != replacement_role.expected_sampled_resource_shape()
        {
            return Err(GalError::invalid_argument(format!(
                "source resource role override '{}' -> '{}' is not sampler-compatible",
                expected_role.semantic_name(),
                replacement_role.semantic_name()
            )));
        }
        let mut bindings = self.bindings.clone();
        let binding = bindings
            .iter_mut()
            .find(|binding| binding.resource_name == resource_name)
            .ok_or_else(|| {
                GalError::unsupported_feature(format!(
                    "source resource plan has no active sampler '{resource_name}' to override"
                ))
            })?;
        if binding.kind != TerrainSourceOpaqueResourceKind::CombinedTextureSampler
            || binding.role != expected_role
        {
            return Err(GalError::unsupported_feature(format!(
                "source resource '{}' is not the expected '{}' sampled role",
                resource_name,
                expected_role.semantic_name()
            )));
        }
        binding.role = replacement_role;
        Ok(Self { bindings })
    }
}

/// Paired source resource table. Bindings are deterministic source-lowering
/// identities only, not native handles or runtime admission evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerrainSourceOpaqueResourceContract {
    resources: Vec<TerrainSourceOpaqueResource>,
}

impl TerrainSourceOpaqueResourceContract {
    pub fn resources(&self) -> &[TerrainSourceOpaqueResource] {
        &self.resources
    }

    fn resource_for(&self, name: &str) -> Option<&TerrainSourceOpaqueResource> {
        self.resources.iter().find(|resource| resource.name == name)
    }

    pub fn active_resources(&self) -> impl Iterator<Item = &TerrainSourceOpaqueResource> {
        self.resources.iter().filter(|resource| resource.active)
    }

    /// Converts pack-declared source names into a closed semantic contract.
    /// Storage images remain distinct from sampled resources so later source
    /// program preparation can require an owned texture view rather than
    /// silently treating a writable image as a sampler.
    pub fn bind_semantic_roles(
        &self,
        declarations: &TerrainSourceResourceBindings,
    ) -> GalResult<TerrainSourceOpaqueResourceBindingPlan> {
        let mut bindings = Vec::with_capacity(self.resources.len());
        let mut missing_roles = Vec::new();
        for resource in self.active_resources() {
            let Some(declared_role) = declarations.role_for(&resource.name) else {
                missing_roles.push(resource.name.as_str());
                continue;
            };
            let role = match resource.kind {
                TerrainSourceOpaqueResourceKind::CombinedTextureSampler => {
                    declared_role.resolve_sampled_declaration(&resource.type_name)?
                }
                TerrainSourceOpaqueResourceKind::StorageImage => declared_role,
            };
            let expected_type = match resource.kind {
                TerrainSourceOpaqueResourceKind::CombinedTextureSampler => {
                    role.expected_sampler_type()
                }
                TerrainSourceOpaqueResourceKind::StorageImage => {
                    role.expected_storage_image_type().ok_or_else(|| {
                        GalError::unsupported_feature(format!(
                            "terrain source role '{}' cannot provide storage-image resource '{}'",
                            role.semantic_name(),
                            resource.name
                        ))
                    })?
                }
            };
            if resource.type_name != expected_type {
                return Err(GalError::invalid_argument(format!(
                    "terrain material source resource '{}' declares '{}' but role {:?} requires '{}'",
                    resource.name,
                    resource.type_name,
                    role,
                    expected_type
                )));
            }
            bindings.push(TerrainSourceOpaqueResourceBinding {
                resource_name: resource.name.clone(),
                role: role.clone(),
                kind: resource.kind,
                qualifiers: resource.qualifiers.clone(),
                binding: resource.binding,
            });
        }
        if !missing_roles.is_empty() {
            const MAX_MISSING_ROLE_DIAGNOSTICS: usize = 12;
            let omitted = missing_roles
                .len()
                .saturating_sub(MAX_MISSING_ROLE_DIAGNOSTICS);
            let listed = missing_roles
                .iter()
                .take(MAX_MISSING_ROLE_DIAGNOSTICS)
                .copied()
                .collect::<Vec<_>>()
                .join(", ");
            let suffix = (omitted != 0).then(|| format!(" (+{omitted} more)"));
            return Err(GalError::unsupported_feature(format!(
                "terrain material source resources have no declared semantic roles: {listed}{}",
                suffix.unwrap_or_default()
            )));
        }
        // The semantic declaration file is pack-wide: its entries may belong
        // to a paired shadow, composite, or disabled preprocessing branch.
        // This source pair owns only the roles it actively consumes. An
        // unrelated declaration therefore cannot allocate or bind a resource
        // through this plan, while an active declaration still requires an
        // exact semantic role above.
        Ok(TerrainSourceOpaqueResourceBindingPlan { bindings })
    }
}

impl TerrainSourceVaryingContract {
    pub fn fields(&self) -> &[TerrainSourceVaryingField] {
        &self.fields
    }

    fn location_for(&self, name: &str) -> Option<u32> {
        self.fields
            .iter()
            .find(|field| field.name == name)
            .map(|field| field.location)
    }
}

/// Coherently lowered source pair. It is private source preparation only and
/// does not create resources or admit a selected-source render route.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoweredTerrainSourcePair {
    vertex: LoweredTerrainVertexSource,
    fragment: LoweredTerrainFragmentSource,
    uniform_contract: TerrainSourceUniformContract,
    varying_contract: TerrainSourceVaryingContract,
    opaque_resource_contract: TerrainSourceOpaqueResourceContract,
}

/// Coherently lowered source pair for the selected pack's distinct
/// translucent terrain stage. Keeping it separate from normal terrain makes
/// its depth/history/blend contract an explicit later requirement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoweredTranslucentTerrainSourcePair {
    vertex: LoweredTerrainVertexSource,
    fragment: LoweredTranslucentTerrainFragmentSource,
    uniform_contract: TerrainSourceUniformContract,
    varying_contract: TerrainSourceVaryingContract,
    opaque_resource_contract: TerrainSourceOpaqueResourceContract,
}

/// Coherently lowered generic textured-material source pair. It deliberately
/// reuses the owned source vertex stream and transforms, while retaining its
/// distinct output schema so it cannot be mistaken for terrain or water.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoweredTexturedMaterialSourcePair {
    vertex: LoweredTerrainVertexSource,
    fragment: LoweredTexturedMaterialFragmentSource,
    uniform_contract: TerrainSourceUniformContract,
    varying_contract: TerrainSourceVaryingContract,
    opaque_resource_contract: TerrainSourceOpaqueResourceContract,
}

/// Coherently lowered ordinary entity source pair. It shares the owned
/// indexed semantic mesh stream with terrain, but retains its entity-specific
/// transform and output contract so an entity draw cannot be relabelled as a
/// terrain or generic textured-material draw.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoweredEntitySourcePair {
    vertex: LoweredTerrainVertexSource,
    fragment: LoweredTerrainFragmentSource,
    uniform_contract: TerrainSourceUniformContract,
    varying_contract: TerrainSourceVaryingContract,
    opaque_resource_contract: TerrainSourceOpaqueResourceContract,
}

/// Coherently lowered first-person hand/item source pair. It shares the
/// owned indexed semantic mesh stream with ordinary entities, but its copied
/// hand projection and isolated depth domain are deliberately a distinct
/// semantic contract. A caller therefore cannot relabel a hand draw as a
/// world entity merely because both source programs use legacy gbuffer
/// matrix names.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoweredHandSourcePair {
    vertex: LoweredTerrainVertexSource,
    fragment: LoweredTerrainFragmentSource,
    uniform_contract: TerrainSourceUniformContract,
    varying_contract: TerrainSourceVaryingContract,
    opaque_resource_contract: TerrainSourceOpaqueResourceContract,
}

/// Coherently lowered weather-source pair. The compact world-material stream
/// is reused, while weather retains its own named output semantics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoweredWeatherSourcePair {
    vertex: LoweredTerrainVertexSource,
    fragment: LoweredWeatherFragmentSource,
    uniform_contract: TerrainSourceUniformContract,
    varying_contract: TerrainSourceVaryingContract,
    opaque_resource_contract: TerrainSourceOpaqueResourceContract,
}

/// Coherently lowered source pair for vanilla clouds. It shares the owned
/// camera-relative material stream but no terrain/weather target contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoweredCloudSourcePair {
    vertex: LoweredTerrainVertexSource,
    fragment: LoweredCloudFragmentSource,
    uniform_contract: TerrainSourceUniformContract,
    varying_contract: TerrainSourceVaryingContract,
    opaque_resource_contract: TerrainSourceOpaqueResourceContract,
}

/// Coherently lowered source pair for a Rust-owned shadow-material pass.
///
/// This remains source preparation only. In particular, it neither allocates
/// a shadow-color attachment nor makes a selected shader-pack route
/// executable. Keeping it distinct from [`LoweredTerrainSourcePair`] prevents
/// a source shadow output from being mislabeled as a terrain G-buffer output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoweredShadowSourcePair {
    vertex: LoweredTerrainVertexSource,
    fragment: LoweredShadowFragmentSource,
    uniform_contract: TerrainSourceUniformContract,
    varying_contract: TerrainSourceVaryingContract,
    opaque_resource_contract: TerrainSourceOpaqueResourceContract,
    /// Source storage-image roles whose writes are produced by a confirmed
    /// Rust semantic runtime instead of the lowered shadow program.
    owned_storage_roles: Vec<TerrainSourceResourceRole>,
}

/// Coherently lowered source pair for the distinct Distant Horizons opaque
/// terrain stage. Its vertex transforms use the source-declared `dh*`
/// matrices rather than borrowing normal terrain or Iris state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoweredDistantHorizonsSourcePair {
    vertex: LoweredTerrainVertexSource,
    fragment: LoweredDistantHorizonsFragmentSource,
    uniform_contract: TerrainSourceUniformContract,
    varying_contract: TerrainSourceVaryingContract,
    opaque_resource_contract: TerrainSourceOpaqueResourceContract,
}

/// Coherently lowered source pair for a source-defined fullscreen stage such
/// as a deferred or composite DH-depth consumer. This remains preparation
/// only: it has no target, pipeline, resource set, command, or route effect.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoweredFullscreenSourcePair {
    vertex: LoweredFullscreenSourceVertex,
    fragment: LoweredFullscreenSourceFragment,
    uniform_contract: TerrainSourceUniformContract,
    varying_contract: TerrainSourceVaryingContract,
    opaque_resource_contract: TerrainSourceOpaqueResourceContract,
    raster_primitive: FullscreenSourceRasterPrimitive,
}

/// Bounded provenance from successful paired lowering. The expanded source is
/// intentionally not retained by discovery; these counts make lowering state
/// observable without becoming a runtime program or resource plan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerrainSourceLoweringSummary {
    pub scalar_uniform_count: u32,
    pub varying_count: u32,
    /// All source-declared opaque resources retained with deterministic
    /// lowering bindings, including declaration-only global-header entries.
    pub opaque_resource_count: u32,
    /// Opaque resources referenced by at least one expanded terrain stage and
    /// therefore requiring a semantic runtime binding before admission.
    pub active_opaque_resource_count: u32,
}

impl LoweredTerrainSourcePair {
    pub fn vertex(&self) -> &LoweredTerrainVertexSource {
        &self.vertex
    }

    pub fn fragment(&self) -> &LoweredTerrainFragmentSource {
        &self.fragment
    }

    pub fn uniform_contract(&self) -> &TerrainSourceUniformContract {
        &self.uniform_contract
    }

    pub fn varying_contract(&self) -> &TerrainSourceVaryingContract {
        &self.varying_contract
    }

    pub fn opaque_resource_contract(&self) -> &TerrainSourceOpaqueResourceContract {
        &self.opaque_resource_contract
    }

    pub fn summary(&self) -> TerrainSourceLoweringSummary {
        TerrainSourceLoweringSummary {
            scalar_uniform_count: self.uniform_contract.declarations.len() as u32,
            varying_count: self.varying_contract.fields.len() as u32,
            opaque_resource_count: self.opaque_resource_contract.resources.len() as u32,
            active_opaque_resource_count: self.opaque_resource_contract.active_resources().count()
                as u32,
        }
    }

    /// Confirms that both owned stages have completed the bounded legacy
    /// dialect lowering required before any backend compiler may see them.
    /// This does not create a program or a resource layout.
    pub fn require_backend_neutral_lowering(&self) -> GalResult<()> {
        self.vertex
            .remaining_dialect()
            .require_backend_neutral_lowering()?;
        self.fragment
            .remaining_dialect()
            .require_backend_neutral_lowering()
    }

    /// Rejects a semantic role plan produced for a different lowered source
    /// pair. Names and deterministic lowering bindings must agree exactly;
    /// backend resource identity is intentionally outside this check.
    pub fn require_matching_opaque_resource_bindings(
        &self,
        bindings: &TerrainSourceOpaqueResourceBindingPlan,
    ) -> GalResult<()> {
        let expected = self
            .opaque_resource_contract
            .active_resources()
            .collect::<Vec<_>>();
        if expected.len() != bindings.bindings().len() {
            return Err(GalError::invalid_argument(format!(
                "terrain source resource plan has {} bindings but lowered pair requires {}",
                bindings.bindings().len(),
                expected.len()
            )));
        }
        for (resource, binding) in expected.iter().zip(bindings.bindings()) {
            if resource.name() != binding.resource_name()
                || resource.kind() != binding.kind()
                || resource.qualifiers() != binding.qualifiers()
                || resource.binding() != binding.binding()
            {
                return Err(GalError::invalid_argument(format!(
                    "terrain source resource plan does not match lowered resource '{}' at binding {}",
                    resource.name(),
                    resource.binding()
                )));
            }
        }
        Ok(())
    }
}

impl LoweredTexturedMaterialSourcePair {
    pub fn vertex(&self) -> &LoweredTerrainVertexSource {
        &self.vertex
    }

    pub fn fragment(&self) -> &LoweredTexturedMaterialFragmentSource {
        &self.fragment
    }

    pub fn uniform_contract(&self) -> &TerrainSourceUniformContract {
        &self.uniform_contract
    }

    pub fn varying_contract(&self) -> &TerrainSourceVaryingContract {
        &self.varying_contract
    }

    pub fn opaque_resource_contract(&self) -> &TerrainSourceOpaqueResourceContract {
        &self.opaque_resource_contract
    }

    pub fn require_backend_neutral_lowering(&self) -> GalResult<()> {
        self.vertex
            .remaining_dialect()
            .require_backend_neutral_lowering()?;
        self.fragment
            .remaining_dialect()
            .require_backend_neutral_lowering()
    }

    pub fn require_matching_opaque_resource_bindings(
        &self,
        bindings: &TerrainSourceOpaqueResourceBindingPlan,
    ) -> GalResult<()> {
        require_matching_opaque_resource_bindings(
            "textured material source",
            &self.opaque_resource_contract,
            bindings,
        )
    }
}

impl LoweredEntitySourcePair {
    pub fn vertex(&self) -> &LoweredTerrainVertexSource {
        &self.vertex
    }

    pub fn fragment(&self) -> &LoweredTerrainFragmentSource {
        &self.fragment
    }

    pub fn uniform_contract(&self) -> &TerrainSourceUniformContract {
        &self.uniform_contract
    }

    pub fn varying_contract(&self) -> &TerrainSourceVaryingContract {
        &self.varying_contract
    }

    pub fn opaque_resource_contract(&self) -> &TerrainSourceOpaqueResourceContract {
        &self.opaque_resource_contract
    }

    pub fn require_backend_neutral_lowering(&self) -> GalResult<()> {
        self.vertex
            .remaining_dialect()
            .require_backend_neutral_lowering()?;
        self.fragment
            .remaining_dialect()
            .require_backend_neutral_lowering()
    }

    pub fn require_matching_opaque_resource_bindings(
        &self,
        bindings: &TerrainSourceOpaqueResourceBindingPlan,
    ) -> GalResult<()> {
        require_matching_opaque_resource_bindings(
            "entity source",
            &self.opaque_resource_contract,
            bindings,
        )
    }
}

impl LoweredHandSourcePair {
    pub fn vertex(&self) -> &LoweredTerrainVertexSource {
        &self.vertex
    }

    pub fn fragment(&self) -> &LoweredTerrainFragmentSource {
        &self.fragment
    }

    pub fn uniform_contract(&self) -> &TerrainSourceUniformContract {
        &self.uniform_contract
    }

    pub fn varying_contract(&self) -> &TerrainSourceVaryingContract {
        &self.varying_contract
    }

    pub fn opaque_resource_contract(&self) -> &TerrainSourceOpaqueResourceContract {
        &self.opaque_resource_contract
    }

    pub fn require_backend_neutral_lowering(&self) -> GalResult<()> {
        self.vertex
            .remaining_dialect()
            .require_backend_neutral_lowering()?;
        self.fragment
            .remaining_dialect()
            .require_backend_neutral_lowering()
    }

    pub fn require_matching_opaque_resource_bindings(
        &self,
        bindings: &TerrainSourceOpaqueResourceBindingPlan,
    ) -> GalResult<()> {
        require_matching_opaque_resource_bindings(
            "hand source",
            &self.opaque_resource_contract,
            bindings,
        )
    }
}

impl LoweredWeatherSourcePair {
    pub fn vertex(&self) -> &LoweredTerrainVertexSource {
        &self.vertex
    }

    pub fn fragment(&self) -> &LoweredWeatherFragmentSource {
        &self.fragment
    }

    pub fn uniform_contract(&self) -> &TerrainSourceUniformContract {
        &self.uniform_contract
    }

    pub fn varying_contract(&self) -> &TerrainSourceVaryingContract {
        &self.varying_contract
    }

    pub fn opaque_resource_contract(&self) -> &TerrainSourceOpaqueResourceContract {
        &self.opaque_resource_contract
    }

    pub fn require_backend_neutral_lowering(&self) -> GalResult<()> {
        self.vertex
            .remaining_dialect()
            .require_backend_neutral_lowering()?;
        self.fragment
            .remaining_dialect()
            .require_backend_neutral_lowering()
    }

    pub fn require_matching_opaque_resource_bindings(
        &self,
        bindings: &TerrainSourceOpaqueResourceBindingPlan,
    ) -> GalResult<()> {
        require_matching_opaque_resource_bindings(
            "weather source",
            &self.opaque_resource_contract,
            bindings,
        )
    }
}

impl LoweredCloudSourcePair {
    pub fn vertex(&self) -> &LoweredTerrainVertexSource {
        &self.vertex
    }

    pub fn fragment(&self) -> &LoweredCloudFragmentSource {
        &self.fragment
    }

    pub fn uniform_contract(&self) -> &TerrainSourceUniformContract {
        &self.uniform_contract
    }

    pub fn varying_contract(&self) -> &TerrainSourceVaryingContract {
        &self.varying_contract
    }

    pub fn opaque_resource_contract(&self) -> &TerrainSourceOpaqueResourceContract {
        &self.opaque_resource_contract
    }

    pub fn require_backend_neutral_lowering(&self) -> GalResult<()> {
        self.vertex
            .remaining_dialect()
            .require_backend_neutral_lowering()?;
        self.fragment
            .remaining_dialect()
            .require_backend_neutral_lowering()
    }

    pub fn require_matching_opaque_resource_bindings(
        &self,
        bindings: &TerrainSourceOpaqueResourceBindingPlan,
    ) -> GalResult<()> {
        require_matching_opaque_resource_bindings(
            "cloud source",
            &self.opaque_resource_contract,
            bindings,
        )
    }
}

impl LoweredTranslucentTerrainSourcePair {
    pub fn vertex(&self) -> &LoweredTerrainVertexSource {
        &self.vertex
    }

    pub fn fragment(&self) -> &LoweredTranslucentTerrainFragmentSource {
        &self.fragment
    }

    pub fn uniform_contract(&self) -> &TerrainSourceUniformContract {
        &self.uniform_contract
    }

    pub fn varying_contract(&self) -> &TerrainSourceVaryingContract {
        &self.varying_contract
    }

    pub fn opaque_resource_contract(&self) -> &TerrainSourceOpaqueResourceContract {
        &self.opaque_resource_contract
    }

    pub fn require_backend_neutral_lowering(&self) -> GalResult<()> {
        self.vertex
            .remaining_dialect()
            .require_backend_neutral_lowering()?;
        self.fragment
            .remaining_dialect()
            .require_backend_neutral_lowering()
    }

    pub fn require_matching_opaque_resource_bindings(
        &self,
        bindings: &TerrainSourceOpaqueResourceBindingPlan,
    ) -> GalResult<()> {
        let expected = self
            .opaque_resource_contract
            .active_resources()
            .collect::<Vec<_>>();
        if expected.len() != bindings.bindings().len() {
            return Err(GalError::invalid_argument(format!(
                "translucent terrain source resource plan has {} bindings but lowered pair requires {}",
                bindings.bindings().len(),
                expected.len()
            )));
        }
        for (resource, binding) in expected.iter().zip(bindings.bindings()) {
            if resource.name() != binding.resource_name()
                || resource.kind() != binding.kind()
                || resource.qualifiers() != binding.qualifiers()
                || resource.binding() != binding.binding()
            {
                return Err(GalError::invalid_argument(format!(
                    "translucent terrain source resource plan does not match lowered resource '{}' at binding {}",
                    resource.name(),
                    resource.binding()
                )));
            }
        }
        Ok(())
    }
}

fn require_matching_opaque_resource_bindings(
    source_label: &str,
    contract: &TerrainSourceOpaqueResourceContract,
    bindings: &TerrainSourceOpaqueResourceBindingPlan,
) -> GalResult<()> {
    let expected = contract.active_resources().collect::<Vec<_>>();
    if expected.len() != bindings.bindings().len() {
        return Err(GalError::invalid_argument(format!(
            "{source_label} resource plan has {} bindings but lowered pair requires {}",
            bindings.bindings().len(),
            expected.len()
        )));
    }
    for (resource, binding) in expected.iter().zip(bindings.bindings()) {
        if resource.name() != binding.resource_name()
            || resource.kind() != binding.kind()
            || resource.qualifiers() != binding.qualifiers()
            || resource.binding() != binding.binding()
        {
            return Err(GalError::invalid_argument(format!(
                "{source_label} resource plan does not match lowered resource '{}' at binding {}",
                resource.name(),
                resource.binding()
            )));
        }
    }
    Ok(())
}

impl LoweredShadowSourcePair {
    pub fn vertex(&self) -> &LoweredTerrainVertexSource {
        &self.vertex
    }

    pub fn fragment(&self) -> &LoweredShadowFragmentSource {
        &self.fragment
    }

    pub fn uniform_contract(&self) -> &TerrainSourceUniformContract {
        &self.uniform_contract
    }

    pub fn varying_contract(&self) -> &TerrainSourceVaryingContract {
        &self.varying_contract
    }

    pub fn opaque_resource_contract(&self) -> &TerrainSourceOpaqueResourceContract {
        &self.opaque_resource_contract
    }

    /// Semantic roles whose legacy shadow writes were explicitly replaced by
    /// an owned Rust producer. These still participate in source-resource
    /// completeness even though the rewritten GLSL no longer declares them.
    pub fn owned_storage_roles(&self) -> &[TerrainSourceResourceRole] {
        &self.owned_storage_roles
    }

    /// Confirms that both source stages have completed the bounded legacy
    /// dialect lowering needed before any future backend compiler may see
    /// them. It still does not create a program or admit source execution.
    pub fn require_backend_neutral_lowering(&self) -> GalResult<()> {
        self.vertex
            .remaining_dialect()
            .require_backend_neutral_lowering()?;
        self.fragment
            .remaining_dialect()
            .require_backend_neutral_lowering()
    }

    /// Rejects a semantic role plan produced for a different lowered shadow
    /// pair. The comparison is source-name and deterministic binding based;
    /// native resource identity remains outside the source contract.
    pub fn require_matching_opaque_resource_bindings(
        &self,
        bindings: &TerrainSourceOpaqueResourceBindingPlan,
    ) -> GalResult<()> {
        let expected = self
            .opaque_resource_contract
            .active_resources()
            .collect::<Vec<_>>();
        if expected.len() != bindings.bindings().len() {
            return Err(GalError::invalid_argument(format!(
                "shadow source resource plan has {} bindings but lowered pair requires {}",
                bindings.bindings().len(),
                expected.len()
            )));
        }
        for (resource, binding) in expected.iter().zip(bindings.bindings()) {
            if resource.name() != binding.resource_name()
                || resource.kind() != binding.kind()
                || resource.binding() != binding.binding()
            {
                return Err(GalError::invalid_argument(format!(
                    "shadow source resource plan does not match lowered resource '{}' at binding {}",
                    resource.name(),
                    resource.binding()
                )));
            }
        }
        Ok(())
    }
}

impl LoweredDistantHorizonsSourcePair {
    pub fn vertex(&self) -> &LoweredTerrainVertexSource {
        &self.vertex
    }

    pub fn fragment(&self) -> &LoweredDistantHorizonsFragmentSource {
        &self.fragment
    }

    pub fn uniform_contract(&self) -> &TerrainSourceUniformContract {
        &self.uniform_contract
    }

    pub fn varying_contract(&self) -> &TerrainSourceVaryingContract {
        &self.varying_contract
    }

    pub fn opaque_resource_contract(&self) -> &TerrainSourceOpaqueResourceContract {
        &self.opaque_resource_contract
    }

    /// DH lowering is source preparation only. This explicit check keeps a
    /// future runtime from compiling a compatibility-dialect source merely
    /// because it has the right output name.
    pub fn require_backend_neutral_lowering(&self) -> GalResult<()> {
        self.vertex
            .remaining_dialect()
            .require_backend_neutral_lowering()?;
        self.fragment
            .remaining_dialect()
            .require_backend_neutral_lowering()
    }

    /// Rejects a semantic role plan produced for another lowered source pair.
    /// The comparison is only source-name/type/qualifier/binding metadata;
    /// native resource identity stays wholly outside this source contract.
    pub fn require_matching_opaque_resource_bindings(
        &self,
        bindings: &TerrainSourceOpaqueResourceBindingPlan,
    ) -> GalResult<()> {
        let expected = self
            .opaque_resource_contract
            .active_resources()
            .collect::<Vec<_>>();
        if expected.len() != bindings.bindings().len() {
            return Err(GalError::invalid_argument(format!(
                "Distant Horizons source resource plan has {} bindings but lowered pair requires {}",
                bindings.bindings().len(),
                expected.len()
            )));
        }
        for (resource, binding) in expected.iter().zip(bindings.bindings()) {
            if resource.name() != binding.resource_name()
                || resource.kind() != binding.kind()
                || resource.qualifiers() != binding.qualifiers()
                || resource.binding() != binding.binding()
            {
                return Err(GalError::invalid_argument(format!(
                    "Distant Horizons source resource plan does not match lowered resource '{}' at binding {}",
                    resource.name(),
                    resource.binding()
                )));
            }
        }
        Ok(())
    }
}

impl LoweredFullscreenSourcePair {
    pub fn raster_primitive(&self) -> FullscreenSourceRasterPrimitive {
        self.raster_primitive
    }
    pub fn vertex(&self) -> &LoweredFullscreenSourceVertex {
        &self.vertex
    }

    pub fn fragment(&self) -> &LoweredFullscreenSourceFragment {
        &self.fragment
    }

    pub fn uniform_contract(&self) -> &TerrainSourceUniformContract {
        &self.uniform_contract
    }

    pub fn varying_contract(&self) -> &TerrainSourceVaryingContract {
        &self.varying_contract
    }

    pub fn opaque_resource_contract(&self) -> &TerrainSourceOpaqueResourceContract {
        &self.opaque_resource_contract
    }

    pub fn require_backend_neutral_lowering(&self) -> GalResult<()> {
        self.vertex
            .remaining_dialect()
            .require_backend_neutral_lowering()?;
        self.fragment
            .remaining_dialect()
            .require_backend_neutral_lowering()
    }

    pub fn require_matching_opaque_resource_bindings(
        &self,
        bindings: &TerrainSourceOpaqueResourceBindingPlan,
    ) -> GalResult<()> {
        require_matching_opaque_resource_bindings(
            "fullscreen source",
            &self.opaque_resource_contract,
            bindings,
        )
    }
}

impl LoweredTerrainVertexSource {
    pub fn entry_path(&self) -> &str {
        &self.entry_path
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn remaining_dialect(&self) -> &GlslDialectReport {
        &self.remaining_dialect
    }
}

impl LoweredTerrainFragmentSource {
    pub fn entry_path(&self) -> &str {
        &self.entry_path
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn outputs(&self) -> &[TerrainFragmentOutput] {
        &self.outputs
    }

    pub fn remaining_dialect(&self) -> &GlslDialectReport {
        &self.remaining_dialect
    }
}

impl LoweredTranslucentTerrainFragmentSource {
    pub fn entry_path(&self) -> &str {
        &self.entry_path
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn outputs(&self) -> &[TranslucentTerrainFragmentOutput] {
        &self.outputs
    }

    pub fn remaining_dialect(&self) -> &GlslDialectReport {
        &self.remaining_dialect
    }
}

impl LoweredTexturedMaterialFragmentSource {
    pub fn entry_path(&self) -> &str {
        &self.entry_path
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn outputs(&self) -> &[TexturedMaterialFragmentOutput] {
        &self.outputs
    }

    pub fn remaining_dialect(&self) -> &GlslDialectReport {
        &self.remaining_dialect
    }
}

impl LoweredWeatherFragmentSource {
    pub fn entry_path(&self) -> &str {
        &self.entry_path
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn outputs(&self) -> &[WeatherFragmentOutput] {
        &self.outputs
    }

    pub fn remaining_dialect(&self) -> &GlslDialectReport {
        &self.remaining_dialect
    }
}

impl LoweredCloudFragmentSource {
    pub fn entry_path(&self) -> &str {
        &self.entry_path
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn outputs(&self) -> &[CloudFragmentOutput] {
        &self.outputs
    }

    pub fn remaining_dialect(&self) -> &GlslDialectReport {
        &self.remaining_dialect
    }
}

impl LoweredShadowFragmentSource {
    pub fn entry_path(&self) -> &str {
        &self.entry_path
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn outputs(&self) -> &[ShadowFragmentOutput] {
        &self.outputs
    }

    pub fn remaining_dialect(&self) -> &GlslDialectReport {
        &self.remaining_dialect
    }
}

impl LoweredDistantHorizonsFragmentSource {
    pub fn entry_path(&self) -> &str {
        &self.entry_path
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn outputs(&self) -> &[DistantHorizonsFragmentOutput] {
        &self.outputs
    }

    pub fn remaining_dialect(&self) -> &GlslDialectReport {
        &self.remaining_dialect
    }
}

impl LoweredFullscreenSourceFragment {
    pub fn entry_path(&self) -> &str {
        &self.entry_path
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn outputs(&self) -> &[FullscreenSourceFragmentOutput] {
        &self.outputs
    }

    pub fn remaining_dialect(&self) -> &GlslDialectReport {
        &self.remaining_dialect
    }
}

impl LoweredFullscreenSourceVertex {
    pub fn entry_path(&self) -> &str {
        &self.entry_path
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn remaining_dialect(&self) -> &GlslDialectReport {
        &self.remaining_dialect
    }
}

/// Lowers only the two legacy fragment constructs whose mapping is fully
/// source-derived today: `texture2D` and `gl_FragData[n]`. This does not
/// attempt to guess vertex interfaces, fixed transforms, varying locations,
/// samplers, or backend descriptor bindings.
pub fn lower_terrain_fragment_surface(
    source: &PreprocessedShaderSource,
) -> GalResult<LoweredTerrainFragmentSource> {
    let uniform_contract = derive_terrain_source_uniform_contract(source, source)?;
    let varying_contract = derive_terrain_source_varying_contract(source, source)?;
    let opaque_resource_contract = derive_terrain_source_opaque_resource_contract(source, source)?;
    lower_terrain_fragment_surface_with_contracts(
        source,
        &uniform_contract,
        &varying_contract,
        &opaque_resource_contract,
    )
}

/// Lowers a legacy shadow fragment into named shadow-pass outputs. This is
/// deliberately fragment-only preparation: execution still requires a
/// separately lowered shadow vertex stage and a complete Rust-owned shadow
/// attachment/resource contract.
pub fn lower_shadow_fragment_surface(
    source: &PreprocessedShaderSource,
) -> GalResult<LoweredShadowFragmentSource> {
    let uniform_contract = derive_terrain_source_uniform_contract(source, source)?;
    let opaque_resource_contract = derive_terrain_source_opaque_resource_contract(source, source)?;
    lower_shadow_fragment_surface_with_contracts(
        source,
        &uniform_contract,
        None,
        &opaque_resource_contract,
    )
}

fn lower_shadow_fragment_surface_with_contracts(
    source: &PreprocessedShaderSource,
    uniform_contract: &TerrainSourceUniformContract,
    varying_contract: Option<&TerrainSourceVaryingContract>,
    opaque_resource_contract: &TerrainSourceOpaqueResourceContract,
) -> GalResult<LoweredShadowFragmentSource> {
    let mut lowered = upgrade_version(source.expanded_source())?;
    lowered = strip_nonopaque_uniforms(&lowered)?;
    let uses_legacy_fog = lower_legacy_fog(&mut lowered);
    for (legacy, explicit) in [
        ("texture2DLod", "textureLod"),
        ("texture3DLod", "textureLod"),
        ("textureCubeLod", "textureLod"),
        ("texture2D", "texture"),
        ("texture3D", "texture"),
        ("textureCube", "texture"),
        ("shadow2D", "vulkanic_source_shadow2D"),
    ] {
        lowered = replace_identifier(&lowered, legacy, explicit);
    }
    // A shadow fragment may declare inputs that only its paired vertex stage
    // produces. Fragment-only preparation deliberately leaves those locations
    // untouched; complete pair lowering validates and assigns them.
    if let Some(varying_contract) = varying_contract {
        lowered = replace_identifier(&lowered, "varying", "in");
        lowered = apply_varying_locations(&lowered, VaryingStorage::In, varying_contract)?;
    }
    lowered = apply_opaque_resource_bindings(&lowered, &opaque_resource_contract)?;
    let mut outputs = Vec::new();
    for output in [
        ShadowFragmentOutput::ShadowColor,
        ShadowFragmentOutput::LightShaftColor,
    ] {
        let (rewritten, occurrences) =
            replace_fragment_output(&lowered, output.legacy_index(), output.semantic_name())?;
        lowered = rewritten;
        if occurrences > 0 {
            outputs.push(output);
        }
    }
    if contains_fragment_output(&lowered)? {
        return Err(GalError::unsupported_feature(format!(
            "shadow fragment '{}' writes an unsupported gl_FragData index",
            source.entry_path()
        )));
    }
    if !outputs.contains(&ShadowFragmentOutput::ShadowColor) {
        return Err(GalError::invalid_argument(format!(
            "shadow fragment '{}' has no shadow color output to lower",
            source.entry_path()
        )));
    }
    let declarations = outputs
        .iter()
        .map(|output| {
            format!(
                "layout(location = {}) out vec4 {};\n",
                output.legacy_index(),
                output.semantic_name()
            )
        })
        .collect::<String>();
    if uses_legacy_fog {
        lowered = insert_after_version(&lowered, LEGACY_FOG_SEMANTIC_PREAMBLE)?;
    }
    lowered = insert_after_version(&lowered, &uniform_block(uniform_contract))?;
    lowered = insert_after_version(&lowered, FRAGMENT_SEMANTIC_PREAMBLE)?;
    lowered = insert_after_version(&lowered, &declarations)?;
    let remaining_dialect = analyze_glsl_text(source.entry_path(), &lowered);
    Ok(LoweredShadowFragmentSource {
        entry_path: source.entry_path().to_string(),
        source: lowered,
        outputs,
        remaining_dialect,
    })
}

/// Lowers both stages with one exact scalar uniform layout. This is still a
/// preparation artifact, but it prevents a future program from silently using
/// mismatched stage-local UBO layouts.
pub fn lower_terrain_source_pair(
    vertex: &PreprocessedShaderSource,
    fragment: &PreprocessedShaderSource,
) -> GalResult<LoweredTerrainSourcePair> {
    // The selected runtime owns colored-light volume production before every
    // terrain phase. Source writes to that owned semantic volume must be
    // externalized consistently for normal terrain as well as shadow passes.
    let owned_storage_bindings = TerrainSourceResourceBindings::default();
    let vertex = externalize_owned_semantic_storage_writes(vertex, &owned_storage_bindings)?;
    let fragment = externalize_owned_semantic_storage_writes(fragment, &owned_storage_bindings)?;
    let uniform_contract = derive_terrain_source_uniform_contract(&vertex, &fragment)?;
    let varying_contract = derive_terrain_source_varying_contract(&vertex, &fragment)?;
    let opaque_resource_contract =
        derive_terrain_source_opaque_resource_contract(&vertex, &fragment)?;
    let mut lowered_fragment = lower_terrain_fragment_surface_with_contracts(
        &fragment,
        &uniform_contract,
        &varying_contract,
        &opaque_resource_contract,
    )?;
    apply_selected_source_fragment_probe(&mut lowered_fragment)?;
    dump_selected_source_lowered_shader(
        lowered_fragment.entry_path(),
        "fragment",
        lowered_fragment.source(),
    );
    Ok(LoweredTerrainSourcePair {
        vertex: lower_source_vertex_surface_with_contracts(
            &vertex,
            &uniform_contract,
            &varying_contract,
            &opaque_resource_contract,
            SourceTransformSemantics::Terrain,
        )?,
        fragment: lowered_fragment,
        uniform_contract,
        varying_contract,
        opaque_resource_contract,
    })
}

/// Lowers the selected pack's generic textured-material stage through the
/// same explicit Rust-owned transform and sampler contracts as terrain, but
/// preserves the pass's own `DRAWBUFFERS:063` output meanings. In particular,
/// output location two is translucency auxiliary data, not a terrain normal.
pub fn lower_textured_material_source_pair(
    vertex: &PreprocessedShaderSource,
    fragment: &PreprocessedShaderSource,
) -> GalResult<LoweredTexturedMaterialSourcePair> {
    let owned_storage_bindings = TerrainSourceResourceBindings::default();
    let vertex = externalize_owned_semantic_storage_writes(vertex, &owned_storage_bindings)?;
    let fragment = externalize_owned_semantic_storage_writes(fragment, &owned_storage_bindings)?;
    let uniform_contract = derive_terrain_source_uniform_contract(&vertex, &fragment)?;
    let varying_contract = derive_terrain_source_varying_contract(&vertex, &fragment)?;
    let opaque_resource_contract =
        derive_terrain_source_opaque_resource_contract(&vertex, &fragment)?;
    Ok(LoweredTexturedMaterialSourcePair {
        vertex: lower_source_vertex_surface_with_contracts(
            &vertex,
            &uniform_contract,
            &varying_contract,
            &opaque_resource_contract,
            SourceTransformSemantics::TexturedMaterial,
        )?,
        fragment: lower_textured_material_fragment_surface_with_contracts(
            &fragment,
            &uniform_contract,
            &varying_contract,
            &opaque_resource_contract,
        )?,
        uniform_contract,
        varying_contract,
        opaque_resource_contract,
    })
}

/// Lowers a selected ordinary entity stage through the Rust-owned indexed
/// source stream. This only prepares source code and semantic layouts; a
/// caller still needs an exact entity-instance contract and named target
/// writer before selected-source execution can be admitted.
pub fn lower_entity_source_pair(
    vertex: &PreprocessedShaderSource,
    fragment: &PreprocessedShaderSource,
) -> GalResult<LoweredEntitySourcePair> {
    let owned_storage_bindings = TerrainSourceResourceBindings::default();
    let vertex = externalize_owned_semantic_storage_writes(vertex, &owned_storage_bindings)?;
    let fragment = externalize_owned_semantic_storage_writes(fragment, &owned_storage_bindings)?;
    let uniform_contract =
        derive_source_uniform_contract(&vertex, &fragment, SourceTransformSemantics::Entity)?;
    let varying_contract = derive_terrain_source_varying_contract(&vertex, &fragment)?;
    let opaque_resource_contract =
        derive_terrain_source_opaque_resource_contract(&vertex, &fragment)?;
    let mut lowered_fragment = lower_terrain_fragment_surface_with_contracts(
        &fragment,
        &uniform_contract,
        &varying_contract,
        &opaque_resource_contract,
    )?;
    install_entity_alpha_cutout_hook(&mut lowered_fragment)?;
    apply_selected_source_entity_fragment_probe(&mut lowered_fragment)?;
    Ok(LoweredEntitySourcePair {
        vertex: lower_source_vertex_surface_with_contracts(
            &vertex,
            &uniform_contract,
            &varying_contract,
            &opaque_resource_contract,
            SourceTransformSemantics::Entity,
        )?,
        fragment: lowered_fragment,
        uniform_contract,
        varying_contract,
        opaque_resource_contract,
    })
}

/// Lowers an explicitly discovered first-person hand/item stage. The
/// resulting pair is intentionally separate from entity lowering even though
/// legacy source names both transforms `gbuffer*`: later execution must bind
/// copied first-person matrices and the hand depth domain, never the world
/// camera matrices or a borrowed Iris hand pass.
pub fn lower_hand_source_pair(
    vertex: &PreprocessedShaderSource,
    fragment: &PreprocessedShaderSource,
) -> GalResult<LoweredHandSourcePair> {
    let owned_storage_bindings = TerrainSourceResourceBindings::default();
    let vertex = externalize_owned_semantic_storage_writes(vertex, &owned_storage_bindings)?;
    let fragment = externalize_owned_semantic_storage_writes(fragment, &owned_storage_bindings)?;
    let uniform_contract =
        derive_source_uniform_contract(&vertex, &fragment, SourceTransformSemantics::Hand)?;
    let varying_contract = derive_terrain_source_varying_contract(&vertex, &fragment)?;
    let opaque_resource_contract =
        derive_terrain_source_opaque_resource_contract(&vertex, &fragment)?;
    let mut lowered_fragment = lower_terrain_fragment_surface_with_contracts(
        &fragment,
        &uniform_contract,
        &varying_contract,
        &opaque_resource_contract,
    )?;
    install_entity_alpha_cutout_hook(&mut lowered_fragment)?;
    apply_selected_source_entity_fragment_probe(&mut lowered_fragment)?;
    let mut lowered_vertex = lower_source_vertex_surface_with_contracts(
        &vertex,
        &uniform_contract,
        &varying_contract,
        &opaque_resource_contract,
        SourceTransformSemantics::Hand,
    )?;
    apply_selected_source_hand_vertex_probe(&mut lowered_vertex)?;
    Ok(LoweredHandSourcePair {
        vertex: lowered_vertex,
        fragment: lowered_fragment,
        uniform_contract,
        varying_contract,
        opaque_resource_contract,
    })
}

/// Capture-only probe for the first-person hand vertex boundary. Hand meshes
/// use a distinct projection/model-view transport, so the ordinary terrain
/// probe must not silently alter them during diagnostics.
fn apply_selected_source_hand_vertex_probe(
    source: &mut LoweredTerrainVertexSource,
) -> GalResult<()> {
    let mode = std::env::var("MATTMC_RUST_SELECTED_SOURCE_HAND_VERTEX_PROBE").ok();
    apply_selected_source_vertex_position_probe_mode(
        &mut source.source,
        mode.as_deref().map(str::trim),
    )
}

/// Minecraft's entity-cutout pipeline applies its alpha test before a
/// transparent texel can update the G-buffer.  Shader packs commonly leave
/// that to the legacy render pipeline instead of spelling `discard` in
/// `gbuffers_entities`.  The Rust-owned writer has no legacy fixed-function
/// alpha-test stage, so retain one explicit, source-local hook for the
/// material-mode specialization to provide.  Translucent and opaque variants
/// leave the hook disabled.
fn install_entity_alpha_cutout_hook(fragment: &mut LoweredTerrainFragmentSource) -> GalResult<()> {
    const ANCHOR: &str = "color *= glColor;";
    const HOOK: &str = concat!(
        "color *= glColor;\n",
        "    if (color.a <= VULKANIC_SOURCE_ENTITY_ALPHA_CUTOFF) discard;"
    );
    if !fragment.source.contains(ANCHOR) {
        return Err(GalError::unsupported_feature(format!(
            "entity fragment '{}' has no color-modulation anchor for explicit cutout semantics",
            fragment.entry_path
        )));
    }
    fragment.source = fragment.source.replacen(ANCHOR, HOOK, 1);
    fragment.source = insert_after_version(
        &fragment.source,
        "#ifndef VULKANIC_SOURCE_ENTITY_ALPHA_CUTOFF\n#define VULKANIC_SOURCE_ENTITY_ALPHA_CUTOFF -1.0\n#endif\n",
    )?;
    Ok(())
}

/// Capture-only probe for the distinct local-texture entity source contract.
/// It is intentionally separate from terrain's atlas probe: entity UVs are
/// local to a Rust-owned material texture and must not inherit terrain-atlas
/// assumptions while we diagnose selected-source material coverage.
fn apply_selected_source_entity_fragment_probe(
    fragment: &mut LoweredTerrainFragmentSource,
) -> GalResult<()> {
    let mode = std::env::var("MATTMC_RUST_SELECTED_SOURCE_ENTITY_FRAGMENT_PROBE").ok();
    apply_selected_source_entity_fragment_probe_mode(fragment, mode.as_deref())
}

fn apply_selected_source_entity_fragment_probe_mode(
    fragment: &mut LoweredTerrainFragmentSource,
    mode: Option<&str>,
) -> GalResult<()> {
    let Some(mode) = mode.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    let (label, replacement) = match mode {
        "texture" => (
            "texture",
            "out_terrain_lit_color = color;\n    out_terrain_material_auxiliary = vec4(0.0);\n    return;",
        ),
        "texture-center" => (
            "texture-center",
            "out_terrain_lit_color = texture(tex, vec2(0.5, 0.5));\n    out_terrain_material_auxiliary = vec4(0.0);\n    return;",
        ),
        "texture-arm" => (
            "texture-arm",
            "out_terrain_lit_color = texture(tex, vec2(0.6875, 0.375));\n    out_terrain_material_auxiliary = vec4(0.0);\n    return;",
        ),
        "texture-arm-opaque" => (
            "texture-arm-opaque",
            "out_terrain_lit_color = vec4(texture(tex, vec2(0.6875, 0.375)).rgb, 1.0);\n    out_terrain_material_auxiliary = vec4(0.0);\n    return;",
        ),
        "uv" => (
            "uv",
            "out_terrain_lit_color = vec4(texCoord, 0.0, 1.0);\n    out_terrain_material_auxiliary = vec4(0.0);\n    return;",
        ),
        "constant-red" => (
            "constant-red",
            "out_terrain_lit_color = vec4(1.0, 0.0, 0.0, 1.0);\n    out_terrain_material_auxiliary = vec4(0.0);\n    return;",
        ),
        "lit" => return Ok(()),
        other => {
            return Err(GalError::invalid_argument(format!(
                "unknown selected-source entity fragment probe '{other}'; expected texture, texture-center, texture-arm, texture-arm-opaque, uv, constant-red, or lit"
            )));
        }
    };
    const ANCHOR: &str = "vec4 color = texture(tex, texCoord);";
    if !fragment.source.contains(ANCHOR) {
        return Err(GalError::invalid_argument(
            "selected-source entity fragment probe could not locate the local material sample",
        ));
    }
    fragment.source = fragment.source.replacen(
        ANCHOR,
        &format!("{ANCHOR}\n    {replacement} // selected-source entity diagnostic probe: {label}"),
        1,
    );
    Ok(())
}

/// Lowers the selected weather stage through the same explicit camera-relative
/// material vertex stream as other world quads. The pass's one color output is
/// retained as weather semantics rather than being treated as a terrain or
/// generic material schema.
pub fn lower_weather_source_pair(
    vertex: &PreprocessedShaderSource,
    fragment: &PreprocessedShaderSource,
) -> GalResult<LoweredWeatherSourcePair> {
    let owned_storage_bindings = TerrainSourceResourceBindings::default();
    let vertex = externalize_owned_semantic_storage_writes(vertex, &owned_storage_bindings)?;
    let fragment = externalize_owned_semantic_storage_writes(fragment, &owned_storage_bindings)?;
    let uniform_contract =
        derive_source_uniform_contract(&vertex, &fragment, SourceTransformSemantics::Weather)?;
    let varying_contract = derive_terrain_source_varying_contract(&vertex, &fragment)?;
    let opaque_resource_contract =
        derive_terrain_source_opaque_resource_contract(&vertex, &fragment)?;
    Ok(LoweredWeatherSourcePair {
        vertex: lower_source_vertex_surface_with_contracts(
            &vertex,
            &uniform_contract,
            &varying_contract,
            &opaque_resource_contract,
            SourceTransformSemantics::Weather,
        )?,
        fragment: lower_weather_fragment_surface_with_contracts(
            &fragment,
            &uniform_contract,
            &varying_contract,
            &opaque_resource_contract,
        )?,
        uniform_contract,
        varying_contract,
        opaque_resource_contract,
    })
}

/// Lowers the selected pack's vanilla cloud stage through the explicit
/// camera-relative material stream, retaining cloud's distinct semantic
/// `DRAWBUFFERS:063` outputs.
pub fn lower_cloud_source_pair(
    vertex: &PreprocessedShaderSource,
    fragment: &PreprocessedShaderSource,
) -> GalResult<LoweredCloudSourcePair> {
    let owned_storage_bindings = TerrainSourceResourceBindings::default();
    let vertex = externalize_owned_semantic_storage_writes(vertex, &owned_storage_bindings)?;
    let fragment = externalize_owned_semantic_storage_writes(fragment, &owned_storage_bindings)?;
    let uniform_contract =
        derive_source_uniform_contract(&vertex, &fragment, SourceTransformSemantics::Cloud)?;
    let varying_contract = derive_terrain_source_varying_contract(&vertex, &fragment)?;
    let opaque_resource_contract =
        derive_terrain_source_opaque_resource_contract(&vertex, &fragment)?;
    Ok(LoweredCloudSourcePair {
        vertex: lower_source_vertex_surface_with_contracts(
            &vertex,
            &uniform_contract,
            &varying_contract,
            &opaque_resource_contract,
            SourceTransformSemantics::Cloud,
        )?,
        fragment: lower_cloud_fragment_surface_with_contracts(
            &fragment,
            &uniform_contract,
            &varying_contract,
            &opaque_resource_contract,
        )?,
        uniform_contract,
        varying_contract,
        opaque_resource_contract,
    })
}

trait SelectedSourceFragmentTarget {
    fn entry_path(&self) -> &str;
    fn source_text(&self) -> &str;
    fn source_text_mut(&mut self) -> &mut String;
    fn diagnostic_auxiliary_output(&self) -> &'static str;
}

impl SelectedSourceFragmentTarget for LoweredTerrainFragmentSource {
    fn entry_path(&self) -> &str {
        &self.entry_path
    }
    fn source_text(&self) -> &str {
        &self.source
    }
    fn source_text_mut(&mut self) -> &mut String {
        &mut self.source
    }
    fn diagnostic_auxiliary_output(&self) -> &'static str {
        "out_terrain_material_auxiliary"
    }
}

impl SelectedSourceFragmentTarget for LoweredTranslucentTerrainFragmentSource {
    fn entry_path(&self) -> &str {
        &self.entry_path
    }
    fn source_text(&self) -> &str {
        &self.source
    }
    fn source_text_mut(&mut self) -> &mut String {
        &mut self.source
    }
    fn diagnostic_auxiliary_output(&self) -> &'static str {
        "out_terrain_translucency_auxiliary"
    }
}

/// Replaces the selected-source terrain color only for an explicitly opted-in
/// diagnostic capture. The probe is deliberately applied after normal source
/// lowering, so it retains the identical semantic mesh, target, resource set,
/// and backend pipeline contract while isolating the first fragment stage that
/// loses visible color. Normal execution never observes this environment key.
fn apply_selected_source_fragment_probe<T: SelectedSourceFragmentTarget>(
    fragment: &mut T,
) -> GalResult<()> {
    let mode = std::env::var("MATTMC_RUST_SELECTED_SOURCE_FRAGMENT_PROBE").ok();
    apply_selected_source_fragment_probe_mode(fragment, mode.as_deref())
}

fn apply_selected_source_fragment_probe_mode<T: SelectedSourceFragmentTarget>(
    fragment: &mut T,
    mode: Option<&str>,
) -> GalResult<()> {
    let mode = match mode {
        Some(mode) => mode,
        None => return Ok(()),
    };
    let water_only = mode.trim().ends_with("-water");
    let mode = mode.trim().strip_suffix("-water").unwrap_or(mode.trim());
    if water_only && !fragment.entry_path().to_ascii_lowercase().contains("water") {
        return Ok(());
    }
    if mode == "shadow-primary" {
        const SHADOW_RETURN: &str = "return shadowcol * (1.0 - shadow0) + shadow0;";
        if !fragment.source_text().contains(SHADOW_RETURN) {
            return Err(GalError::invalid_argument(
                "selected-source shadow-primary probe could not locate SampleShadow return",
            ));
        }
        *fragment.source_text_mut() = fragment.source_text().replacen(
            SHADOW_RETURN,
            "return vec3(shadow0); // selected-source diagnostic probe: shadow-primary",
            1,
        );
        return Ok(());
    }
    // The exact-atlas Distant Horizons adapter owns this checkpoint because
    // its source initializes color differently from normal terrain.  Accept
    // it here so a bounded DH diagnostic does not prevent the paired normal
    // terrain program from preparing; the normal fragment remains unchanged.
    if mode == "pre-lighting" {
        return Ok(());
    }
    let (label, anchor, injected) = match mode {
        "force-opaque" => (
            "force-opaque",
            "if (color.a <= 0.00001) discard;",
            "if (false) discard; // selected-source diagnostic probe: force-opaque",
        ),
        "atlas" => (
            "atlas",
            "vec4 color = texture(tex, texCoord);",
            "out_terrain_lit_color = color;\n    out_terrain_material_auxiliary = vec4(0.0);\n    return;",
        ),
        // Keeps the identical selected-source vertex, index, instance, and
        // resource-set path while exposing the interpolated atlas coordinate.
        // It is capture-only evidence for distinguishing a corrupted source
        // vertex stream from a sampler/descriptor mismatch; it never changes
        // the normal selected-source fragment path.
        "atlas-uv" => (
            "atlas-uv",
            "vec4 color = texture(tex, texCoord);",
            "out_terrain_lit_color = vec4(texCoord, 0.0, 1.0);\n    out_terrain_material_auxiliary = vec4(0.0);\n    return;",
        ),
        "atlas-alpha" => (
            "atlas-alpha",
            "vec4 color = texture(tex, texCoord);",
            "out_terrain_lit_color = vec4(vec3(color.a), 1.0);\n    out_terrain_material_auxiliary = vec4(0.0);\n    return;",
        ),
        // Diagnostic-only row-origin check. The selected source normally owns
        // its UV convention; this does not change it or make flipped sampling
        // available to production execution.
        "atlas-alpha-flipped-v" => (
            "atlas-alpha-flipped-v",
            "vec4 color = texture(tex, texCoord);",
            "vec4 vulkanic_flipped_v_color = texture(tex, vec2(texCoord.x, 1.0 - texCoord.y));\n    out_terrain_lit_color = vec4(vec3(vulkanic_flipped_v_color.a), 1.0);\n    out_terrain_material_auxiliary = vec4(0.0);\n    return;",
        ),
        "tint" => (
            "tint",
            "vec4 color = texture(tex, texCoord);",
            "out_terrain_lit_color = vec4(color.rgb * glColor.rgb, color.a);\n    out_terrain_material_auxiliary = vec4(0.0);\n    return;",
        ),
        // This capture-only probe observes the fragment varying after the
        // source vertex path, before Complementary's lighting terms consume
        // its alpha as vanilla AO. It distinguishes vertex-interface loss
        // from a later lighting calculation without changing production code.
        "vertex-color" => (
            "vertex-color",
            "vec4 color = texture(tex, texCoord);",
            "out_terrain_lit_color = glColor;\n    out_terrain_material_auxiliary = vec4(0.0);\n    return;",
        ),
        // Keeps the actual RGB varying but makes the diagnostic target opaque.
        // A difference from `vertex-color` proves an AO/alpha-lane issue
        // rather than a loss of the whole interpolated color.
        "vertex-color-opaque" => (
            "vertex-color-opaque",
            "vec4 color = texture(tex, texCoord);",
            "out_terrain_lit_color = vec4(glColor.rgb, 1.0);\n    out_terrain_material_auxiliary = vec4(0.0);\n    return;",
        ),
        "vertex-color-raw" => (
            "vertex-color-raw",
            "vec4 color = texture(tex, texCoord);",
            "out_terrain_lit_color = vec4(glColorRaw.rgb, 1.0);\n    out_terrain_material_auxiliary = vec4(0.0);\n    return;",
        ),
        "lightmap" => (
            "lightmap",
            "vec4 color = texture(tex, texCoord);",
            "out_terrain_lit_color = vec4(lmCoord.x, lmCoord.y, glColor.a, 1.0);\n    out_terrain_material_auxiliary = vec4(0.0);\n    return;",
        ),
        "scene-light" => (
            "scene-light",
            "vec3 sceneLighting = lightColorM * shadowMult + ambientColorM * ambientMult;",
            "color = vec4(sceneLighting, 1.0);\n    return;",
        ),
        "light-color" => (
            "light-color",
            "vec3 sceneLighting = lightColorM * shadowMult + ambientColorM * ambientMult;",
            "color = vec4(lightColorM, 1.0);\n    return;",
        ),
        "ambient-color" => (
            "ambient-color",
            "vec3 sceneLighting = lightColorM * shadowMult + ambientColorM * ambientMult;",
            "color = vec4(ambientColorM, 1.0);\n    return;",
        ),
        "shadow-mult" => (
            "shadow-mult",
            "vec3 sceneLighting = lightColorM * shadowMult + ambientColorM * ambientMult;",
            "color = vec4(shadowMult, 1.0);\n    return;",
        ),
        "final-diffuse" => (
            "final-diffuse",
            "color.rgb *= finalDiffuse;",
            "color = vec4(finalDiffuse, 1.0);\n    return;",
        ),
        "lighting-factors" => (
            "lighting-factors",
            "vec3 finalDiffuse = pow2(directionShade * vanillaAO) * (blockLighting + pow2(sceneLighting) + minLighting) + pow2(emission);",
            "color = vec4(\n        clamp(directionShade, 0.0, 1.0),\n        clamp(vanillaAO, 0.0, 1.0),\n        clamp(max(max(sceneLighting.r, sceneLighting.g), sceneLighting.b), 0.0, 1.0),\n        1.0\n    );\n    return;",
        ),
        // The source-derived lighting path has several independently semantic
        // inputs. These two capture-only probes identify the first zero term
        // without changing normal selected-source execution.
        "lighting-components-a" => (
            "lighting-components-a",
            "vec3 finalDiffuse = pow2(directionShade * vanillaAO) * (blockLighting + pow2(sceneLighting) + minLighting) + pow2(emission);",
            "color = vec4(\n        clamp(max(max(lightColorM.r, lightColorM.g), lightColorM.b), 0.0, 1.0),\n        clamp(max(max(ambientColorM.r, ambientColorM.g), ambientColorM.b), 0.0, 1.0),\n        clamp(max(max(shadowMult.r, shadowMult.g), shadowMult.b), 0.0, 1.0),\n        1.0\n    );\n    return;",
        ),
        "lighting-components-b" => (
            "lighting-components-b",
            "vec3 finalDiffuse = pow2(directionShade * vanillaAO) * (blockLighting + pow2(sceneLighting) + minLighting) + pow2(emission);",
            "color = vec4(\n        clamp(ambientMult, 0.0, 1.0),\n        clamp(max(max(blockLighting.r, blockLighting.g), blockLighting.b), 0.0, 1.0),\n        clamp(max(max(minLighting.r, minLighting.g), minLighting.b), 0.0, 1.0),\n        1.0\n    );\n    return;",
        ),
        "darkness-scale" => (
            "darkness-scale",
            "color.rgb *= pow2(1.0 - darknessLightFactor);",
            "color = vec4(vec3(pow2(1.0 - darknessLightFactor)), 1.0);\n    return;",
        ),
        "shadow-coordinate" => (
            "shadow-coordinate",
            "vec3 shadowPos = GetShadowPos(playerPosM);",
            "color = vec4(shadowPos, 1.0);\n    return;",
        ),
        "shadow-coordinate-centered" => (
            "shadow-coordinate-centered",
            "vec3 shadowPos = GetShadowPos(playerPosM);",
            "color = vec4(clamp(shadowPos * 0.5 + 0.5, 0.0, 1.0), 1.0);\n    return;",
        ),
        // Capture-only reconstruction probe. Complementary derives the
        // player-space position from gl_FragCoord and the semantic view/
        // projection inverses before shadow sampling. Keeping this separate
        // from the shadow-coordinate probe isolates a bad reconstruction from
        // a later shadow matrix or depth-compare failure.
        "player-position" => (
            "player-position",
            "vec3 playerPos = ViewToPlayer(viewPos);",
            "out_terrain_lit_color = vec4(clamp(playerPos / 384.0 + 0.5, 0.0, 1.0), 1.0);\n    out_terrain_material_auxiliary = vec4(0.0);\n    return;",
        ),
        // R is the raster depth received by the source fragment; G is the
        // reconstructed camera-relative height; B is reconstructed view
        // distance. This single bounded diagnostic distinguishes a depth
        // convention fault from a matrix-inverse fault without perturbing the
        // selected source route.
        "reconstruction" => (
            "reconstruction",
            "vec3 playerPos = ViewToPlayer(viewPos);",
            "out_terrain_lit_color = vec4(gl_FragCoord.z, clamp(playerPos.y / 384.0 + 0.5, 0.0, 1.0), clamp(length(viewPos) / 384.0, 0.0, 1.0), 1.0);\n    out_terrain_material_auxiliary = vec4(0.0);\n    return;",
        ),
        // Encodes the reconstructed view vector without clipping large values
        // to a single color. This is capture-only and separates a projection
        // inverse failure from the following ViewToPlayer conversion.
        "view-reconstruction-components" => (
            "view-reconstruction-components",
            "vec3 playerPos = ViewToPlayer(viewPos);",
            "out_terrain_lit_color = vec4(atan(viewPos / 32.0) / 3.14159265 + 0.5, 1.0);\n    out_terrain_material_auxiliary = vec4(0.0);\n    return;",
        ),
        // Captures the exact normalized screen input to ScreenToView. It
        // rules out a bad viewport scalar before diagnosing inverse matrices.
        "screen-reconstruction-input" => (
            "screen-reconstruction-input",
            "vec3 playerPos = ViewToPlayer(viewPos);",
            "out_terrain_lit_color = vec4(gl_FragCoord.xy / vec2(viewWidth, viewHeight), gl_FragCoord.z, 1.0);\n    out_terrain_material_auxiliary = vec4(0.0);\n    return;",
        ),
        // Separates the scalar UBO values from gl_FragCoord itself. The
        // atan encoding preserves useful evidence for zero, expected, and
        // implausibly large viewport values without changing production
        // shader behavior or source-pass semantics.
        "viewport-uniforms" => (
            "viewport-uniforms",
            "vec3 playerPos = ViewToPlayer(viewPos);",
            "out_terrain_lit_color = vec4(atan(viewWidth / 1024.0) * 0.63661977, atan(viewHeight / 1024.0) * 0.63661977, clamp(gl_FragCoord.x / 1280.0, 0.0, 1.0), 1.0);\n    out_terrain_material_auxiliary = vec4(0.0);\n    return;",
        ),
        // Capture-only scalar-block probe. The selected source reconstructs
        // view space from these exact matrices, so exposing a bounded basis
        // sample distinguishes a broken dynamic UBO binding from a later
        // shadow-coordinate or compare-sampler fault.
        "matrix-basis" => (
            "matrix-basis",
            "vec3 playerPos = ViewToPlayer(viewPos);",
            "out_terrain_lit_color = vec4(\n        clamp(abs(gbufferProjection[0][0]) * 0.25, 0.0, 1.0),\n        clamp(abs(gbufferProjection[1][1]) * 0.25, 0.0, 1.0),\n        clamp(abs(gbufferProjectionInverse[3][2]) * 0.001, 0.0, 1.0),\n        1.0\n    );\n    out_terrain_material_auxiliary = vec4(0.0);\n    return;",
        ),
        "constant-red" => (
            "constant-red",
            "vec3 sceneLighting = lightColorM * shadowMult + ambientColorM * ambientMult;",
            "out_terrain_lit_color = vec4(1.0, 0.0, 0.0, 1.0);\n    out_terrain_material_auxiliary = vec4(0.0);\n    return;",
        ),
        "shadow-compare" => (
            "shadow-compare",
            "vec3 shadowPos = GetShadowPos(playerPosM);",
            "color = vec4(vec3(vulkanic_source_shadow2D(shadowtex0, vec3(shadowPos.st, shadowPos.z)).x), 1.0);\n    return;",
        ),
        "" | "lit" => return Ok(()),
        other => {
            return Err(GalError::invalid_argument(format!(
                "unknown selected-source fragment probe '{other}'; expected force-opaque, atlas, atlas-uv, atlas-alpha, atlas-alpha-flipped-v, tint, vertex-color, vertex-color-opaque, vertex-color-raw, lightmap, scene-light, light-color, ambient-color, shadow-mult, shadow-primary, pre-lighting, final-diffuse, lighting-factors, lighting-components-a, lighting-components-b, darkness-scale, player-position, reconstruction, view-reconstruction-components, screen-reconstruction-input, viewport-uniforms, matrix-basis, shadow-coordinate, shadow-coordinate-centered, shadow-compare, constant-red, or lit"
            )));
        }
    };
    // Terrain and water source programs use different semantic names for
    // the first atlas sample (`color` versus `colorP`). Keep the probe at
    // the same source boundary for both programs; otherwise an atlas probe
    // silently exercises only opaque terrain and provides no evidence for
    // the translucent writer that owns the failing fixture.
    let (anchor, sample_variable) = if fragment.source_text().contains(anchor) {
        (anchor, "color")
    } else if anchor == "vec4 color = texture(tex, texCoord);"
        && fragment
            .source_text()
            .contains("vec4 colorP = texture(tex, texCoord);")
    {
        ("vec4 colorP = texture(tex, texCoord);", "colorP")
    } else {
        return Err(GalError::invalid_argument(
            "selected-source fragment probe could not locate the lowered atlas sample",
        ));
    };
    let injected = if sample_variable == "colorP" {
        injected.replace("color.", "colorP.")
    } else {
        injected.to_string()
    };
    let injected = injected.replace(
        "out_terrain_material_auxiliary",
        fragment.diagnostic_auxiliary_output(),
    );
    let injected = format!("{anchor}\n    {injected} // selected-source diagnostic probe: {label}");
    *fragment.source_text_mut() = fragment.source_text().replacen(anchor, &injected, 1);
    Ok(())
}

/// Applies a strictly capture-only checkpoint to the reduced Distant Horizons
/// source stream.  DH has a separate fragment interface from near terrain, so
/// sharing the near-terrain probe would either mutate the wrong program or
/// reject a valid source pair before it can be observed.
///
/// The probe is intentionally selected by a distinct environment key.  It
/// never changes the semantic stream, pass target, resources, or route; it
/// only replaces the final color write after the real DH source has run far
/// enough to establish the requested varying.  This distinguishes an invalid
/// projected stream from source lighting/composition failure without a Java
/// fallback or a production rendering workaround.
fn apply_selected_source_distant_horizons_fragment_probe(
    fragment: &mut LoweredDistantHorizonsFragmentSource,
) -> GalResult<()> {
    let Some(mode) = std::env::var("MATTMC_RUST_SELECTED_SOURCE_DH_FRAGMENT_PROBE")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };

    let (label, expression) = match mode.as_str() {
        "constant-red" => ("constant-red", "vec4(1.0, 0.0, 0.0, 1.0)"),
        "vertex-color" => ("vertex-color", "vec4(glColor.rgb, 1.0)"),
        "lightmap" => ("lightmap", "vec4(lmCoord, 0.0, 1.0)"),
        "normal" => ("normal", "vec4(normalize(normal) * 0.5 + 0.5, 1.0)"),
        "player-position" => (
            "player-position",
            "vec4(clamp(playerPos / 512.0 + 0.5, 0.0, 1.0), 1.0)",
        ),
        "lit" => return Ok(()),
        other => {
            return Err(GalError::invalid_argument(format!(
                "unknown Distant Horizons selected-source fragment probe '{other}'; expected constant-red, vertex-color, lightmap, normal, player-position, or lit"
            )));
        }
    };
    const OUTPUT: &str = "out_distant_horizons_lit_color = color;";
    if !fragment.source.contains(OUTPUT) {
        return Err(GalError::invalid_argument(
            "Distant Horizons selected-source fragment probe could not locate the final lit-color write",
        ));
    }
    fragment.source = fragment.source.replacen(
        OUTPUT,
        &format!(
            "out_distant_horizons_lit_color = {expression}; // selected-source DH diagnostic probe: {label}"
        ),
        1,
    );
    Ok(())
}

/// Lowers the pack's distinct translucent terrain pair using the same
/// semantic vertex/uniform/resource contracts as normal terrain, while
/// retaining its own named fragment-output schema. This does not construct a
/// blend pass or claim executable source-route admission.
pub fn lower_translucent_terrain_source_pair(
    vertex: &PreprocessedShaderSource,
    fragment: &PreprocessedShaderSource,
) -> GalResult<LoweredTranslucentTerrainSourcePair> {
    let owned_storage_bindings = TerrainSourceResourceBindings::default();
    let vertex = externalize_owned_semantic_storage_writes(vertex, &owned_storage_bindings)?;
    let fragment = externalize_owned_semantic_storage_writes(fragment, &owned_storage_bindings)?;
    let uniform_contract = derive_terrain_source_uniform_contract(&vertex, &fragment)?;
    let varying_contract = derive_terrain_source_varying_contract(&vertex, &fragment)?;
    let opaque_resource_contract =
        derive_terrain_source_opaque_resource_contract(&vertex, &fragment)?;
    let mut lowered_fragment = lower_translucent_terrain_fragment_surface_with_contracts(
        &fragment,
        &uniform_contract,
        &varying_contract,
        &opaque_resource_contract,
    )?;
    // Keep selected-source diagnostics symmetric with opaque terrain. The
    // translucent writer has a distinct lowering function, so it must opt in
    // explicitly or atlas/output probes silently exercise only gbuffers_terrain.
    apply_selected_source_fragment_probe(&mut lowered_fragment)?;
    dump_selected_source_lowered_shader(
        lowered_fragment.entry_path(),
        "fragment",
        lowered_fragment.source(),
    );
    Ok(LoweredTranslucentTerrainSourcePair {
        vertex: lower_source_vertex_surface_with_contracts(
            &vertex,
            &uniform_contract,
            &varying_contract,
            &opaque_resource_contract,
            SourceTransformSemantics::Terrain,
        )?,
        fragment: lowered_fragment,
        uniform_contract,
        varying_contract,
        opaque_resource_contract,
    })
}

/// Lowers the exact scoped shadow-source pair into explicit source semantics.
/// Shadow transforms deliberately remain distinct from G-buffer transforms,
/// and shadow outputs remain distinct from normal terrain outputs. This is
/// not an executable pass: a future Rust-owned shadow-color attachment and
/// full named resource plan are still required before source execution can be
/// admitted.
pub fn lower_shadow_source_pair(
    vertex: &PreprocessedShaderSource,
    fragment: &PreprocessedShaderSource,
) -> GalResult<LoweredShadowSourcePair> {
    lower_shadow_source_pair_with_owned_storage(
        vertex,
        fragment,
        &TerrainSourceResourceBindings::default(),
    )
}

/// Lowers a shadow pair after explicitly establishing which pack-declared
/// storage images have a Rust-owned semantic writer. The binding table is a
/// source-level name-to-role contract, never an OpenGL/Vulkan binding table;
/// this lets a future pack rename its image without making backend state part
/// of source lowering.
pub fn lower_shadow_source_pair_with_owned_storage(
    vertex: &PreprocessedShaderSource,
    fragment: &PreprocessedShaderSource,
    owned_storage_bindings: &TerrainSourceResourceBindings,
) -> GalResult<LoweredShadowSourcePair> {
    let owned_storage_roles =
        externalized_shadow_storage_roles(vertex, fragment, owned_storage_bindings)?;
    let vertex = externalize_owned_semantic_storage_writes(vertex, owned_storage_bindings)?;
    let fragment = externalize_owned_semantic_storage_writes(fragment, owned_storage_bindings)?;
    let uniform_contract =
        derive_source_uniform_contract(&vertex, &fragment, SourceTransformSemantics::Shadow)?;
    let varying_contract = derive_terrain_source_varying_contract(&vertex, &fragment)?;
    let opaque_resource_contract =
        derive_terrain_source_opaque_resource_contract(&vertex, &fragment)?;
    Ok(LoweredShadowSourcePair {
        vertex: lower_source_vertex_surface_with_contracts(
            &vertex,
            &uniform_contract,
            &varying_contract,
            &opaque_resource_contract,
            SourceTransformSemantics::Shadow,
        )?,
        fragment: lower_shadow_fragment_surface_with_contracts(
            &fragment,
            &uniform_contract,
            Some(&varying_contract),
            &opaque_resource_contract,
        )?,
        uniform_contract,
        varying_contract,
        opaque_resource_contract,
        owned_storage_roles,
    })
}

/// Lowers the exact DH source pair with its own semantic transforms and
/// single named color output. The returned artifact has no pipeline, target,
/// or route-selection effect; the future executor must still provide a
/// complete Rust-owned DH depth/composite contract.
pub fn lower_distant_horizons_source_pair(
    vertex: &PreprocessedShaderSource,
    fragment: &PreprocessedShaderSource,
) -> GalResult<LoweredDistantHorizonsSourcePair> {
    let uniform_contract = derive_source_uniform_contract(
        vertex,
        fragment,
        SourceTransformSemantics::DistantHorizons,
    )?;
    let varying_contract = derive_terrain_source_varying_contract(vertex, fragment)?;
    let opaque_resource_contract =
        derive_terrain_source_opaque_resource_contract(vertex, fragment)?;
    let mut lowered_fragment = lower_distant_horizons_fragment_surface_with_contracts(
        fragment,
        &uniform_contract,
        &varying_contract,
        &opaque_resource_contract,
    )?;
    apply_selected_source_distant_horizons_fragment_probe(&mut lowered_fragment)?;
    Ok(LoweredDistantHorizonsSourcePair {
        vertex: lower_source_vertex_surface_with_contracts(
            vertex,
            &uniform_contract,
            &varying_contract,
            &opaque_resource_contract,
            SourceTransformSemantics::DistantHorizons,
        )?,
        fragment: lowered_fragment,
        uniform_contract,
        varying_contract,
        opaque_resource_contract,
    })
}

/// Lowers a source-defined fullscreen stage using only a Rust-owned
/// position/UV stream, scalar source uniforms, and pack-declared semantic
/// resources. Output slots are resolved immediately to named pack-color
/// roles, so later pass scheduling never has to reason about `gl_FragData` or
/// `colortexN` identifiers.
///
/// This function intentionally returns preparation only. It cannot compile a
/// program, allocate an attachment, or cause Distant Horizons to leave its
/// explicit Java compatibility route.
pub fn lower_fullscreen_source_pair(
    vertex: &PreprocessedShaderSource,
    fragment: &PreprocessedShaderSource,
    bindings: &TerrainSourceResourceBindings,
) -> GalResult<LoweredFullscreenSourcePair> {
    lower_fullscreen_source_pair_with_raster_primitive(
        vertex,
        fragment,
        bindings,
        FullscreenSourceRasterPrimitive::FullscreenTriangle,
    )
}

/// Lowers a source stage pair with its explicit owned raster primitive.
/// Callers normally use [`lower_fullscreen_source_pair`]; source sky is the
/// currently audited non-fullscreen consumer.
pub fn lower_fullscreen_source_pair_with_raster_primitive(
    vertex: &PreprocessedShaderSource,
    fragment: &PreprocessedShaderSource,
    bindings: &TerrainSourceResourceBindings,
    raster_primitive: FullscreenSourceRasterPrimitive,
) -> GalResult<LoweredFullscreenSourcePair> {
    let uniform_contract =
        derive_fullscreen_source_uniform_contract(vertex, fragment, raster_primitive)?;
    let varying_contract = derive_terrain_source_varying_contract(vertex, fragment)?;
    let opaque_resource_contract =
        derive_terrain_source_opaque_resource_contract(vertex, fragment)?;
    Ok(LoweredFullscreenSourcePair {
        vertex: lower_fullscreen_source_vertex_with_contracts(
            vertex,
            &uniform_contract,
            &varying_contract,
            &opaque_resource_contract,
            raster_primitive,
        )?,
        fragment: lower_fullscreen_source_fragment_with_contracts(
            fragment,
            &uniform_contract,
            &varying_contract,
            &opaque_resource_contract,
            bindings,
        )?,
        uniform_contract,
        varying_contract,
        opaque_resource_contract,
        raster_primitive,
    })
}

/// The selected Rust runtime generates the semantic colored-light occupancy
/// volume before source terrain/shadow passes execute. Legacy shadow sources
/// that populate the same volume with `imageStore` therefore need their
/// storage mutation externalized during source lowering: retaining it would
/// race the Rust-owned sampled volume in the very draw that consumes it.
///
/// This is deliberately narrow. Only standalone writes to declared
/// `writeonly uimage3D` resources are externalized; any other storage-image
/// form or expression is rejected so a pack cannot silently lose unrelated
/// shader behavior.
fn externalized_shadow_storage_roles(
    vertex: &PreprocessedShaderSource,
    fragment: &PreprocessedShaderSource,
    owned_storage_bindings: &TerrainSourceResourceBindings,
) -> GalResult<Vec<TerrainSourceResourceRole>> {
    let mut roles = BTreeSet::new();
    for source in [vertex, fragment] {
        for line in source.expanded_source().lines() {
            let Some(declaration) = parse_opaque_resource_declaration(line.trim())? else {
                continue;
            };
            if declaration.kind != TerrainSourceOpaqueResourceKind::StorageImage
                || declaration.type_name != "uimage2D"
                || !declaration
                    .qualifiers
                    .split_whitespace()
                    .any(|qualifier| qualifier == "writeonly")
            {
                continue;
            }
            let Some(role) = owned_storage_bindings.role_for(&declaration.name) else {
                continue;
            };
            if role == TerrainSourceResourceRole::PuddleOccupancy {
                roles.insert(role);
            }
        }
    }
    Ok(roles.into_iter().collect())
}

fn externalize_owned_semantic_storage_writes(
    source: &PreprocessedShaderSource,
    owned_storage_roles: &TerrainSourceResourceBindings,
) -> GalResult<PreprocessedShaderSource> {
    let mut externalized = std::collections::BTreeSet::new();
    for line in source.expanded_source().lines() {
        let Some(declaration) = parse_opaque_resource_declaration(line.trim())? else {
            continue;
        };
        if declaration.kind != TerrainSourceOpaqueResourceKind::StorageImage {
            continue;
        }
        let writeonly = declaration
            .qualifiers
            .split_whitespace()
            .any(|qualifier| qualifier == "writeonly");
        let semantic_writer = match declaration.type_name.as_str() {
            // The existing colored-light runtime owns its D3 occupancy field.
            "uimage3D" => true,
            // A 2D image is externalizable only when pack configuration maps
            // this exact source name to the installed puddle semantic writer.
            "uimage2D" => {
                owned_storage_roles.role_for(&declaration.name)
                    == Some(TerrainSourceResourceRole::PuddleOccupancy)
            }
            _ => false,
        };
        if !writeonly || !semantic_writer {
            return Err(GalError::unsupported_feature(format!(
                "source storage image '{}' requires an explicit Rust semantic writer",
                declaration.name
            )));
        }
        externalized.insert(declaration.name);
    }
    if externalized.is_empty() {
        return Ok(source.clone());
    }

    let rewritten =
        rewrite_externalized_image_store_calls(source.expanded_source(), &externalized)?;
    let mut without_declarations = String::with_capacity(rewritten.len());
    for line in rewritten.lines() {
        let Some(declaration) = parse_opaque_resource_declaration(line.trim())? else {
            without_declarations.push_str(line);
            without_declarations.push('\n');
            continue;
        };
        if externalized.contains(&declaration.name) {
            without_declarations.push_str("// Rust-owned semantic storage update\n");
        } else {
            without_declarations.push_str(line);
            without_declarations.push('\n');
        }
    }
    for name in &externalized {
        if glsl_identifiers(&without_declarations).contains(name) {
            return Err(GalError::unsupported_feature(format!(
                "source storage image '{name}' has unsupported uses outside standalone imageStore calls"
            )));
        }
    }
    source.rewritten_for_lowering(without_declarations)
}

fn rewrite_externalized_image_store_calls(
    source: &str,
    externalized: &std::collections::BTreeSet<String>,
) -> GalResult<String> {
    let mut output = String::with_capacity(source.len());
    let mut cursor = 0usize;
    while let Some(relative) = source[cursor..].find("imageStore") {
        let start = cursor + relative;
        let name_end = start + "imageStore".len();
        let before_is_identifier = start > 0 && is_identifier_byte(source.as_bytes()[start - 1]);
        let after_is_identifier = source
            .as_bytes()
            .get(name_end)
            .is_some_and(|byte| is_identifier_byte(*byte));
        if before_is_identifier || after_is_identifier {
            output.push_str(&source[cursor..name_end]);
            cursor = name_end;
            continue;
        }
        let mut open = name_end;
        while source
            .as_bytes()
            .get(open)
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            open += 1;
        }
        if source.as_bytes().get(open) != Some(&b'(') {
            output.push_str(&source[cursor..name_end]);
            cursor = name_end;
            continue;
        }
        let mut argument = open + 1;
        while source
            .as_bytes()
            .get(argument)
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            argument += 1;
        }
        let argument_end = source[argument..]
            .find(|character: char| !is_identifier_byte(character as u8))
            .map(|offset| argument + offset)
            .unwrap_or(source.len());
        let target = &source[argument..argument_end];
        if target.is_empty() || !externalized.contains(target) {
            return Err(GalError::unsupported_feature(
                "shadow source imageStore target has no Rust-owned semantic writer",
            ));
        }
        let mut depth = 0u32;
        let mut end = open;
        loop {
            let byte = *source.as_bytes().get(end).ok_or_else(|| {
                GalError::invalid_argument("shadow source imageStore call is unterminated")
            })?;
            match byte {
                b'(' => depth = depth.saturating_add(1),
                b')' => {
                    depth = depth.checked_sub(1).ok_or_else(|| {
                        GalError::invalid_argument(
                            "shadow source imageStore parentheses are invalid",
                        )
                    })?;
                    if depth == 0 {
                        end += 1;
                        break;
                    }
                }
                _ => {}
            }
            end += 1;
        }
        let mut statement_end = end;
        while source
            .as_bytes()
            .get(statement_end)
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            statement_end += 1;
        }
        if source.as_bytes().get(statement_end) != Some(&b';') {
            return Err(GalError::unsupported_feature(
                "shadow source imageStore must be a standalone statement",
            ));
        }
        output.push_str(&source[cursor..start]);
        output.push_str("/* Rust-owned semantic storage update */");
        cursor = end;
    }
    output.push_str(&source[cursor..]);
    Ok(output)
}

fn lower_terrain_fragment_surface_with_contracts(
    source: &PreprocessedShaderSource,
    uniform_contract: &TerrainSourceUniformContract,
    varying_contract: &TerrainSourceVaryingContract,
    opaque_resource_contract: &TerrainSourceOpaqueResourceContract,
) -> GalResult<LoweredTerrainFragmentSource> {
    let mut lowered = upgrade_version(source.expanded_source())?;
    lowered = strip_nonopaque_uniforms(&lowered)?;
    let uses_legacy_fog = lower_legacy_fog(&mut lowered);
    for (legacy, explicit) in [
        ("texture2DLod", "textureLod"),
        ("texture3DLod", "textureLod"),
        ("textureCubeLod", "textureLod"),
        ("texture2D", "texture"),
        ("texture3D", "texture"),
        ("textureCube", "texture"),
        ("shadow2D", "vulkanic_source_shadow2D"),
    ] {
        lowered = replace_identifier(&lowered, legacy, explicit);
    }
    lowered = apply_varying_locations(&lowered, VaryingStorage::In, varying_contract)?;
    lowered = apply_opaque_resource_bindings(&lowered, opaque_resource_contract)?;
    let mut outputs = Vec::new();
    for output in [
        TerrainFragmentOutput::LitColor,
        TerrainFragmentOutput::MaterialAuxiliary,
        TerrainFragmentOutput::ViewSpaceNormal,
    ] {
        let (rewritten, occurrences) =
            replace_fragment_output(&lowered, output.legacy_index(), output.semantic_name())?;
        lowered = rewritten;
        if occurrences > 0 {
            outputs.push(output);
        }
    }
    if contains_fragment_output(&lowered)? {
        return Err(GalError::unsupported_feature(format!(
            "terrain fragment '{}' writes an unsupported gl_FragData index",
            source.entry_path()
        )));
    }
    if outputs.is_empty() {
        return Err(GalError::invalid_argument(format!(
            "terrain fragment '{}' has no named terrain output to lower",
            source.entry_path()
        )));
    }
    let declarations = outputs
        .iter()
        .map(|output| {
            format!(
                "layout(location = {}) out vec4 {};\n",
                output.legacy_index(),
                output.semantic_name()
            )
        })
        .collect::<String>();
    if uses_legacy_fog {
        lowered = insert_after_version(&lowered, LEGACY_FOG_SEMANTIC_PREAMBLE)?;
    }
    // Source terrain fragments use OpenGL's lower-left gl_FragCoord contract.
    // Keep that source convention explicit for every world-material writer;
    // Vulkan's negative viewport otherwise inverts screen-space water/fog
    // reconstruction while geometry itself remains correctly transformed.
    lowered = lower_world_material_fragment_coordinates(lowered, uniform_contract)?;
    lowered = insert_after_version(&lowered, &uniform_block(uniform_contract))?;
    lowered = insert_after_version(&lowered, FRAGMENT_SEMANTIC_PREAMBLE)?;
    lowered = insert_after_version(&lowered, &declarations)?;
    let remaining_dialect = analyze_glsl_text(source.entry_path(), &lowered);
    Ok(LoweredTerrainFragmentSource {
        entry_path: source.entry_path().to_string(),
        source: lowered,
        outputs,
        remaining_dialect,
    })
}

fn lower_textured_material_fragment_surface_with_contracts(
    source: &PreprocessedShaderSource,
    uniform_contract: &TerrainSourceUniformContract,
    varying_contract: &TerrainSourceVaryingContract,
    opaque_resource_contract: &TerrainSourceOpaqueResourceContract,
) -> GalResult<LoweredTexturedMaterialFragmentSource> {
    let mut lowered = upgrade_version(source.expanded_source())?;
    lowered = strip_nonopaque_uniforms(&lowered)?;
    let uses_legacy_fog = lower_legacy_fog(&mut lowered);
    for (legacy, explicit) in [
        ("texture2DLod", "textureLod"),
        ("texture3DLod", "textureLod"),
        ("textureCubeLod", "textureLod"),
        ("texture2D", "texture"),
        ("texture3D", "texture"),
        ("textureCube", "texture"),
        ("shadow2D", "vulkanic_source_shadow2D"),
    ] {
        lowered = replace_identifier(&lowered, legacy, explicit);
    }
    lowered = apply_varying_locations(&lowered, VaryingStorage::In, varying_contract)?;
    lowered = apply_opaque_resource_bindings(&lowered, opaque_resource_contract)?;
    let mut outputs = Vec::new();
    for output in [
        TexturedMaterialFragmentOutput::LitColor,
        TexturedMaterialFragmentOutput::MaterialAuxiliary,
        TexturedMaterialFragmentOutput::TranslucencyAuxiliary,
    ] {
        let (rewritten, occurrences) =
            replace_fragment_output(&lowered, output.legacy_index(), output.semantic_name())?;
        lowered = rewritten;
        if occurrences > 0 {
            outputs.push(output);
        }
    }
    if contains_fragment_output(&lowered)? {
        return Err(GalError::unsupported_feature(format!(
            "textured material fragment '{}' writes an unsupported gl_FragData index",
            source.entry_path()
        )));
    }
    let required_outputs = [
        TexturedMaterialFragmentOutput::LitColor,
        TexturedMaterialFragmentOutput::MaterialAuxiliary,
        TexturedMaterialFragmentOutput::TranslucencyAuxiliary,
    ];
    if required_outputs
        .iter()
        .any(|output| !outputs.contains(output))
    {
        return Err(GalError::invalid_argument(format!(
            "textured material fragment '{}' lacks one or more required named outputs",
            source.entry_path()
        )));
    }
    let declarations = outputs
        .iter()
        .map(|output| {
            format!(
                "layout(location = {}) out vec4 {};\n",
                output.legacy_index(),
                output.semantic_name()
            )
        })
        .collect::<String>();
    if uses_legacy_fog {
        lowered = insert_after_version(&lowered, LEGACY_FOG_SEMANTIC_PREAMBLE)?;
    }
    lowered = insert_after_version(&lowered, &uniform_block(uniform_contract))?;
    lowered = insert_after_version(&lowered, FRAGMENT_SEMANTIC_PREAMBLE)?;
    lowered = insert_after_version(&lowered, &declarations)?;
    let remaining_dialect = analyze_glsl_text(source.entry_path(), &lowered);
    Ok(LoweredTexturedMaterialFragmentSource {
        entry_path: source.entry_path().to_string(),
        source: lowered,
        outputs,
        remaining_dialect,
    })
}

fn lower_weather_fragment_surface_with_contracts(
    source: &PreprocessedShaderSource,
    uniform_contract: &TerrainSourceUniformContract,
    varying_contract: &TerrainSourceVaryingContract,
    opaque_resource_contract: &TerrainSourceOpaqueResourceContract,
) -> GalResult<LoweredWeatherFragmentSource> {
    let mut lowered = upgrade_version(source.expanded_source())?;
    lowered = strip_nonopaque_uniforms(&lowered)?;
    let uses_legacy_fog = lower_legacy_fog(&mut lowered);
    for (legacy, explicit) in [
        ("texture2DLod", "textureLod"),
        ("texture3DLod", "textureLod"),
        ("textureCubeLod", "textureLod"),
        ("texture2D", "texture"),
        ("texture3D", "texture"),
        ("textureCube", "texture"),
        ("shadow2D", "vulkanic_source_shadow2D"),
    ] {
        lowered = replace_identifier(&lowered, legacy, explicit);
    }
    lowered = replace_identifier(&lowered, "varying", "in");
    lowered = apply_varying_locations(&lowered, VaryingStorage::In, varying_contract)?;
    lowered = apply_opaque_resource_bindings(&lowered, opaque_resource_contract)?;
    let output = WeatherFragmentOutput::LitColor;
    let (rewritten, occurrences) =
        replace_fragment_output(&lowered, output.legacy_index(), output.semantic_name())?;
    lowered = rewritten;
    if occurrences == 0 {
        return Err(GalError::invalid_argument(format!(
            "weather fragment '{}' has no lit-color output",
            source.entry_path()
        )));
    }
    if contains_fragment_output(&lowered)? {
        return Err(GalError::unsupported_feature(format!(
            "weather fragment '{}' writes an unsupported gl_FragData index",
            source.entry_path()
        )));
    }
    if uses_legacy_fog {
        lowered = insert_after_version(&lowered, LEGACY_FOG_SEMANTIC_PREAMBLE)?;
    }
    lowered = insert_after_version(&lowered, &uniform_block(uniform_contract))?;
    lowered = insert_after_version(&lowered, FRAGMENT_SEMANTIC_PREAMBLE)?;
    lowered = insert_after_version(
        &lowered,
        "layout(location = 0) out vec4 out_weather_lit_color;\n",
    )?;
    let remaining_dialect = analyze_glsl_text(source.entry_path(), &lowered);
    Ok(LoweredWeatherFragmentSource {
        entry_path: source.entry_path().to_string(),
        source: lowered,
        outputs: vec![output],
        remaining_dialect,
    })
}

fn lower_cloud_fragment_surface_with_contracts(
    source: &PreprocessedShaderSource,
    uniform_contract: &TerrainSourceUniformContract,
    varying_contract: &TerrainSourceVaryingContract,
    opaque_resource_contract: &TerrainSourceOpaqueResourceContract,
) -> GalResult<LoweredCloudFragmentSource> {
    let mut lowered = upgrade_version(source.expanded_source())?;
    lowered = strip_nonopaque_uniforms(&lowered)?;
    let uses_legacy_fog = lower_legacy_fog(&mut lowered);
    for (legacy, explicit) in [
        ("texture2DLod", "textureLod"),
        ("texture3DLod", "textureLod"),
        ("textureCubeLod", "textureLod"),
        ("texture2D", "texture"),
        ("texture3D", "texture"),
        ("textureCube", "texture"),
        ("shadow2D", "vulkanic_source_shadow2D"),
    ] {
        lowered = replace_identifier(&lowered, legacy, explicit);
    }
    lowered = replace_identifier(&lowered, "varying", "in");
    lowered = apply_varying_locations(&lowered, VaryingStorage::In, varying_contract)?;
    lowered = apply_opaque_resource_bindings(&lowered, opaque_resource_contract)?;
    let mut outputs = Vec::new();
    for output in [
        CloudFragmentOutput::LitColor,
        CloudFragmentOutput::MaterialAuxiliary,
        CloudFragmentOutput::TranslucencyAuxiliary,
    ] {
        let (rewritten, occurrences) =
            replace_fragment_output(&lowered, output.legacy_index(), output.semantic_name())?;
        lowered = rewritten;
        if occurrences > 0 {
            outputs.push(output);
        }
    }
    if contains_fragment_output(&lowered)? {
        return Err(GalError::unsupported_feature(format!(
            "cloud fragment '{}' writes an unsupported gl_FragData index",
            source.entry_path()
        )));
    }
    let required_outputs = [
        CloudFragmentOutput::LitColor,
        CloudFragmentOutput::MaterialAuxiliary,
        CloudFragmentOutput::TranslucencyAuxiliary,
    ];
    if required_outputs
        .iter()
        .any(|output| !outputs.contains(output))
    {
        return Err(GalError::invalid_argument(format!(
            "cloud fragment '{}' lacks one or more required named outputs",
            source.entry_path()
        )));
    }
    let declarations = outputs
        .iter()
        .map(|output| {
            format!(
                "layout(location = {}) out vec4 {};\n",
                output.legacy_index(),
                output.semantic_name()
            )
        })
        .collect::<String>();
    if uses_legacy_fog {
        lowered = insert_after_version(&lowered, LEGACY_FOG_SEMANTIC_PREAMBLE)?;
    }
    lowered = insert_after_version(&lowered, &uniform_block(uniform_contract))?;
    lowered = insert_after_version(&lowered, FRAGMENT_SEMANTIC_PREAMBLE)?;
    lowered = insert_after_version(&lowered, &declarations)?;
    let remaining_dialect = analyze_glsl_text(source.entry_path(), &lowered);
    Ok(LoweredCloudFragmentSource {
        entry_path: source.entry_path().to_string(),
        source: lowered,
        outputs,
        remaining_dialect,
    })
}

fn lower_translucent_terrain_fragment_surface_with_contracts(
    source: &PreprocessedShaderSource,
    uniform_contract: &TerrainSourceUniformContract,
    varying_contract: &TerrainSourceVaryingContract,
    opaque_resource_contract: &TerrainSourceOpaqueResourceContract,
) -> GalResult<LoweredTranslucentTerrainFragmentSource> {
    let mut lowered = upgrade_version(source.expanded_source())?;
    lowered = strip_nonopaque_uniforms(&lowered)?;
    let uses_legacy_fog = lower_legacy_fog(&mut lowered);
    for (legacy, explicit) in [
        ("texture2DLod", "textureLod"),
        ("texture3DLod", "textureLod"),
        ("textureCubeLod", "textureLod"),
        ("texture2D", "texture"),
        ("texture3D", "texture"),
        ("textureCube", "texture"),
        ("shadow2D", "vulkanic_source_shadow2D"),
    ] {
        lowered = replace_identifier(&lowered, legacy, explicit);
    }
    lowered = apply_varying_locations(&lowered, VaryingStorage::In, varying_contract)?;
    lowered = apply_opaque_resource_bindings(&lowered, opaque_resource_contract)?;
    let mut outputs = Vec::new();
    for output in [
        TranslucentTerrainFragmentOutput::LitColor,
        TranslucentTerrainFragmentOutput::TranslucencyAuxiliary,
        TranslucentTerrainFragmentOutput::MaterialAuxiliary,
    ] {
        let (rewritten, occurrences) =
            replace_fragment_output(&lowered, output.legacy_index(), output.semantic_name())?;
        lowered = rewritten;
        if occurrences > 0 {
            outputs.push(output);
        }
    }
    if contains_fragment_output(&lowered)? {
        return Err(GalError::unsupported_feature(format!(
            "translucent terrain fragment '{}' writes an unsupported gl_FragData index",
            source.entry_path()
        )));
    }
    if !outputs.contains(&TranslucentTerrainFragmentOutput::LitColor)
        || !outputs.contains(&TranslucentTerrainFragmentOutput::TranslucencyAuxiliary)
    {
        return Err(GalError::invalid_argument(format!(
            "translucent terrain fragment '{}' lacks a required named color or translucency output",
            source.entry_path()
        )));
    }
    let declarations = outputs
        .iter()
        .map(|output| {
            format!(
                "layout(location = {}) out vec4 {};\n",
                output.legacy_index(),
                output.semantic_name()
            )
        })
        .collect::<String>();
    if uses_legacy_fog {
        lowered = insert_after_version(&lowered, LEGACY_FOG_SEMANTIC_PREAMBLE)?;
    }
    // The water/translucent source shares the same lower-left fragment-space
    // contract as opaque terrain. Do not let it silently diverge from the
    // semantic source convention just because it has a distinct output pass.
    lowered = lower_world_material_fragment_coordinates(lowered, uniform_contract)?;
    lowered = insert_after_version(&lowered, &uniform_block(uniform_contract))?;
    lowered = insert_after_version(&lowered, FRAGMENT_SEMANTIC_PREAMBLE)?;
    lowered = insert_after_version(&lowered, &declarations)?;
    let remaining_dialect = analyze_glsl_text(source.entry_path(), &lowered);
    Ok(LoweredTranslucentTerrainFragmentSource {
        entry_path: source.entry_path().to_string(),
        source: lowered,
        outputs,
        remaining_dialect,
    })
}

fn lower_distant_horizons_fragment_surface_with_contracts(
    source: &PreprocessedShaderSource,
    uniform_contract: &TerrainSourceUniformContract,
    varying_contract: &TerrainSourceVaryingContract,
    opaque_resource_contract: &TerrainSourceOpaqueResourceContract,
) -> GalResult<LoweredDistantHorizonsFragmentSource> {
    let mut lowered = upgrade_version(source.expanded_source())?;
    lowered = strip_nonopaque_uniforms(&lowered)?;
    let uses_legacy_fog = lower_legacy_fog(&mut lowered);
    for (legacy, explicit) in [
        ("texture2DLod", "textureLod"),
        ("texture3DLod", "textureLod"),
        ("textureCubeLod", "textureLod"),
        ("texture2D", "texture"),
        ("texture3D", "texture"),
        ("textureCube", "texture"),
        ("shadow2D", "vulkanic_source_shadow2D"),
    ] {
        lowered = replace_identifier(&lowered, legacy, explicit);
    }
    lowered = apply_varying_locations(&lowered, VaryingStorage::In, varying_contract)?;
    lowered = apply_opaque_resource_bindings(&lowered, opaque_resource_contract)?;
    let output = DistantHorizonsFragmentOutput::LitColor;
    let (rewritten, occurrences) =
        replace_fragment_output(&lowered, output.legacy_index(), output.semantic_name())?;
    lowered = rewritten;
    if contains_fragment_output(&lowered)? {
        return Err(GalError::unsupported_feature(format!(
            "Distant Horizons fragment '{}' writes an unsupported gl_FragData index",
            source.entry_path()
        )));
    }
    if occurrences == 0 {
        return Err(GalError::invalid_argument(format!(
            "Distant Horizons fragment '{}' has no lit color output to lower",
            source.entry_path()
        )));
    }
    let declaration = format!(
        "layout(location = {}) out vec4 {};\n",
        output.legacy_index(),
        output.semantic_name()
    );
    if uses_legacy_fog {
        lowered = insert_after_version(&lowered, LEGACY_FOG_SEMANTIC_PREAMBLE)?;
    }
    // Insert the coordinate helper before the source uniform block so the
    // subsequent insertion leaves `viewHeight` declared before the helper.
    lowered = lower_world_material_fragment_coordinates(lowered, uniform_contract)?;
    lowered = insert_after_version(&lowered, &uniform_block(uniform_contract))?;
    lowered = insert_after_version(&lowered, FRAGMENT_SEMANTIC_PREAMBLE)?;
    lowered = insert_after_version(&lowered, &declaration)?;
    let remaining_dialect = analyze_glsl_text(source.entry_path(), &lowered);
    Ok(LoweredDistantHorizonsFragmentSource {
        entry_path: source.entry_path().to_string(),
        source: lowered,
        outputs: vec![output],
        remaining_dialect,
    })
}

const FRAGMENT_SEMANTIC_PREAMBLE: &str = r#"#define vulkanic_source_shadow2D(source_texture, source_coordinates) vec4(texture(source_texture, source_coordinates))
"#;

/// Preserves the source pack's lower-left screen-coordinate contract for
/// world-material fragments. Vulkan's native fragment Y origin is opposite.
/// Integer source-target addresses stay native, though: those addresses name
/// the Rust-owned image storage rather than a source-space screen direction.
fn lower_world_material_fragment_coordinates(
    mut source: String,
    uniform_contract: &TerrainSourceUniformContract,
) -> GalResult<String> {
    if !glsl_identifiers(&source).contains("gl_FragCoord") {
        return Ok(source);
    }
    if !uniform_contract
        .fields()
        .iter()
        .any(|field| field.name() == "viewHeight")
    {
        return Err(GalError::unsupported_feature(
            "world-material source reads gl_FragCoord but does not declare viewHeight for explicit coordinate conversion",
        ));
    }
    source = replace_identifier(
        &source,
        "gl_FragCoord",
        "vulkanic_source_world_fragment_coord()",
    );
    let (source, source_target_sampling_preamble) =
        lower_world_material_source_target_sampling(source);
    insert_after_version(
        &source,
        &format!(
            r#"vec4 vulkanic_source_world_fragment_coord() {{
    vec4 coordinate = gl_FragCoord;
#ifdef VULKANIC_GAL_ZERO_TO_ONE_CLIP_DEPTH
    coordinate.y = viewHeight - coordinate.y;
#endif
    return coordinate;
}}
{}"#,
            source_target_sampling_preamble
        ),
    )
}

/// Source terrain fragments express screen coordinates in the OpenGL
/// lower-left domain. Rust-owned pass targets are sampled in their native
/// image domain, so source-target samplers must flip that coordinate exactly
/// once. Atlas/material samplers deliberately remain untouched.
fn lower_world_material_source_target_sampling(mut source: String) -> (String, String) {
    const SOURCE_TARGET_SAMPLERS: &[&str] = &[
        "depthtex0",
        "depthtex1",
        "depthtex2",
        "dhDepthTex",
        "dhDepthTex0",
        "dhDepthTex1",
        "gaux1",
        "gaux2",
        "gaux3",
        "gaux4",
        "colortex0",
        "colortex1",
        "colortex2",
        "colortex3",
        "colortex4",
        "colortex5",
        "colortex6",
        "colortex7",
        "colortex8",
        "colortex9",
        "colortex10",
        "colortex11",
        "colortex12",
        "colortex13",
        "colortex14",
        "colortex15",
    ];

    let mut preamble = String::from(
        r#"#ifdef VULKANIC_GAL_ZERO_TO_ONE_CLIP_DEPTH
#define vulkanic_source_world_target_uv(source_uv) vec2((source_uv).x, 1.0 - (source_uv).y)
#else
#define vulkanic_source_world_target_uv(source_uv) (source_uv)
#endif
"#,
    );
    for sampler in SOURCE_TARGET_SAMPLERS {
        let source_call = format!("texture({sampler},");
        if !source.contains(&source_call) {
            continue;
        }
        let helper = format!("vulkanic_source_sample_target_{sampler}");
        source = source.replace(&source_call, &format!("{helper}("));
        // A macro deliberately expands at the original source call site. The
        // source sampler declarations can occur after our semantic preamble,
        // while a GLSL function body would require each sampler to have been
        // declared before that body is parsed.
        preamble.push_str(&format!(
            "#define {helper}(source_uv) texture({sampler}, vulkanic_source_world_target_uv(source_uv))\n"
        ));
    }
    (source, preamble)
}

/// Lowers the audited terrain vertex compatibility names into an explicit
/// indexed source stream. This is not a generic GLSL compatibility shim: a
/// source declaring any unrecognized legacy attribute is rejected rather than
/// assigned an invented location or default value.
pub fn lower_terrain_vertex_surface(
    source: &PreprocessedShaderSource,
) -> GalResult<LoweredTerrainVertexSource> {
    let uniform_contract = derive_terrain_source_uniform_contract(source, source)?;
    let varying_contract = derive_terrain_source_varying_contract(source, source)?;
    let opaque_resource_contract = derive_terrain_source_opaque_resource_contract(source, source)?;
    lower_source_vertex_surface_with_contracts(
        source,
        &uniform_contract,
        &varying_contract,
        &opaque_resource_contract,
        SourceTransformSemantics::Terrain,
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SourceTransformSemantics {
    Terrain,
    Entity,
    Hand,
    TexturedMaterial,
    Weather,
    Cloud,
    Shadow,
    DistantHorizons,
    Fullscreen,
}

impl SourceTransformSemantics {
    fn model_view_expression(self) -> &'static str {
        match self {
            // First-person ModelPart poses are already expressed in the
            // camera's hand space; compose the copied pose before the camera
            // matrix so the hand remains in the HUD-side viewport.
            Self::Hand => "(vulkanic_source_model_transform * gbufferModelView)",
            Self::Terrain | Self::Entity => "(gbufferModelView * vulkanic_source_model_transform)",
            Self::Shadow => "(shadowModelView * vulkanic_source_model_transform)",
            Self::DistantHorizons => "(dhModelView * vulkanic_source_model_transform)",
            Self::TexturedMaterial | Self::Weather | Self::Cloud | Self::Fullscreen => {
                "gbufferModelView"
            }
        }
    }

    fn model_view_uniform(self) -> &'static str {
        match self {
            Self::Terrain => "gbufferModelView",
            Self::Entity => "gbufferModelView",
            Self::Hand => "gbufferModelView",
            Self::TexturedMaterial => "gbufferModelView",
            Self::Weather => "gbufferModelView",
            Self::Cloud => "gbufferModelView",
            Self::Shadow => "shadowModelView",
            Self::DistantHorizons => "dhModelView",
            Self::Fullscreen => "vulkanic_source_fullscreen_unused_model_view",
        }
    }

    fn projection_uniform(self) -> &'static str {
        match self {
            Self::Terrain => "gbufferProjection",
            Self::Entity => "gbufferProjection",
            Self::Hand => "gbufferProjection",
            Self::TexturedMaterial => "gbufferProjection",
            Self::Weather => "gbufferProjection",
            Self::Cloud => "gbufferProjection",
            Self::Shadow => "shadowProjection",
            Self::DistantHorizons => "dhProjection",
            Self::Fullscreen => "vulkanic_source_fullscreen_unused_projection",
        }
    }
}

fn lower_source_vertex_surface_with_contracts(
    source: &PreprocessedShaderSource,
    uniform_contract: &TerrainSourceUniformContract,
    varying_contract: &TerrainSourceVaryingContract,
    opaque_resource_contract: &TerrainSourceOpaqueResourceContract,
    transforms: SourceTransformSemantics,
) -> GalResult<LoweredTerrainVertexSource> {
    let mut lowered = upgrade_version(source.expanded_source())?;
    lowered = remove_known_legacy_attributes(&lowered)?;
    lowered = strip_nonopaque_uniforms(&lowered)?;
    let uses_legacy_fog = lower_legacy_fog(&mut lowered);
    lowered = replace_identifier(&lowered, "varying", "out");
    for (legacy, explicit) in [
        ("gl_TextureMatrix", "vulkanic_source_texture_matrix"),
        ("gl_MultiTexCoord0", "vulkanic_source_atlas_uv"),
        ("gl_MultiTexCoord1", "vulkanic_source_lightmap_uv"),
        ("gl_Color", "vulkanic_source_vertex_color"),
        ("gl_NormalMatrix", "vulkanic_source_normal_matrix"),
        ("gl_Normal", "vulkanic_source_normal"),
        ("gl_ModelViewMatrix", "vulkanic_source_model_view"),
        // Legacy source still spells the projection built-in in a few DH
        // include paths. That builtin belongs to the transform family being
        // lowered; routing it through gbufferProjection makes far geometry
        // use the near-terrain clip volume.
        ("gl_ProjectionMatrix", transforms.projection_uniform()),
        ("gl_VertexID", "gl_VertexIndex"),
        ("gl_Vertex", "vulkanic_source_position"),
        ("mc_Entity", "vulkanic_source_entity"),
        ("mc_midTexCoord", "vulkanic_source_mid_tex_coord"),
        ("at_tangent", "vulkanic_source_tangent"),
        ("at_midBlock", "vulkanic_source_mid_block"),
        ("dhMaterialId", "vulkanic_source_dh_material_id"),
        (
            "GetLightMapCoordinates",
            "vulkanic_source_dh_lightmap_coordinates",
        ),
        ("ftransform", "vulkanic_source_ftransform"),
        ("texture2D", "texture"),
    ] {
        lowered = replace_identifier(&lowered, legacy, explicit);
    }
    // The OpenGL pack reconstructs player-space geometry with
    // `gbufferModelViewInverse * gl_ModelViewMatrix * gl_Vertex`.  A Rust
    // source instance already carries the camera-relative model transform;
    // applying that legacy inverse/re-application sequence would feed a
    // camera-relative position through a second, incompatible reconstruction
    // path.  Preserve the pack's final camera projection while making the
    // semantic world-position step explicit.  This is limited to terrain and
    // entity meshes; hand/material families have distinct pose contracts.
    if matches!(
        transforms,
        SourceTransformSemantics::Terrain | SourceTransformSemantics::Entity
    ) {
        lowered = lowered.replace(
            "gbufferModelViewInverse * vulkanic_source_model_view * vulkanic_source_position",
            "vulkanic_source_model_transform * vulkanic_source_position",
        );
    }
    apply_selected_source_wave_probe(&mut lowered)?;
    apply_selected_source_taa_probe(&mut lowered)?;
    apply_selected_source_vertex_probe(&mut lowered)?;
    lowered = apply_varying_locations(&lowered, VaryingStorage::Out, varying_contract)?;
    lowered = apply_opaque_resource_bindings(&lowered, opaque_resource_contract)?;
    if uses_legacy_fog {
        lowered = insert_after_version(&lowered, LEGACY_FOG_SEMANTIC_PREAMBLE)?;
    }
    lowered = insert_after_version(&lowered, &uniform_block(uniform_contract))?;
    lowered = insert_after_version(&lowered, &vertex_semantic_preamble(transforms))?;
    // Shader-pack source is authored for OpenGL clip depth. The Vulkan backend
    // selects the explicit zero-to-one convention through a source define, so
    // apply the conversion after the pack has completed every vertex write.
    // Keeping it in lowered source leaves GAL state backend-neutral and keeps
    // the OpenGL source byte-for-byte equivalent at runtime.
    lowered = append_vulkan_clip_depth_finalizer(&lowered)?;
    apply_selected_source_vertex_probe_for_entry(&mut lowered, source.entry_path())?;
    dump_selected_source_lowered_shader(source.entry_path(), "vertex", &lowered);
    let remaining_dialect = analyze_glsl_text(source.entry_path(), &lowered);
    Ok(LoweredTerrainVertexSource {
        entry_path: source.entry_path().to_string(),
        source: lowered,
        remaining_dialect,
    })
}

fn dump_selected_source_lowered_shader(entry_path: &str, stage: &str, source: &str) {
    let Ok(dir) = std::env::var("MATTMC_RUST_SHADER_DUMP_DIR") else {
        return;
    };
    let safe_name = entry_path
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect::<String>();
    let path = std::path::Path::new(&dir).join(format!("{stage}_{safe_name}.glsl"));
    if std::fs::create_dir_all(&dir).is_ok() {
        let _ = std::fs::write(path, source);
    }
}

/// Capture-only probe for isolating invalid TAA frame-modulo semantics.  The
/// normal source route retains the pack's jitter; this replacement is never
/// admitted by route selection.
fn apply_selected_source_taa_probe(source: &mut String) -> GalResult<()> {
    let Some(mode) = std::env::var("MATTMC_RUST_SELECTED_SOURCE_TAA_PROBE")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };
    if mode != "disable" {
        return Err(GalError::invalid_argument(format!(
            "unknown selected-source TAA probe '{mode}'; expected disable"
        )));
    }
    const ASSIGNMENT: &str = "gl_Position.xy = TAAJitter(gl_Position.xy, gl_Position.w);";
    if source.contains("selected-source diagnostic probe: TAA disabled") {
        return Ok(());
    }
    if source.contains(ASSIGNMENT) {
        *source = source.replacen(
            ASSIGNMENT,
            "// selected-source diagnostic probe: TAA disabled",
            1,
        );
    }
    Ok(())
}

/// Test-only selected-source vertex probe. It modifies one semantic varying
/// assignment before source compilation so the matching fragment probe can
/// distinguish a vertex-buffer field failure from a stage-interface failure.
/// Normal source execution never observes this opt-in environment key.
fn apply_selected_source_vertex_probe(source: &mut String) -> GalResult<()> {
    let mode = std::env::var("MATTMC_RUST_SELECTED_SOURCE_VERTEX_PROBE").ok();
    if matches!(
        mode.as_deref().map(str::trim),
        Some("direct-model-transform-water") | Some("clip-quad-water")
    ) {
        return Ok(());
    }
    apply_selected_source_vertex_position_probe_mode(source, mode.as_deref())
}

fn apply_selected_source_vertex_probe_for_entry(
    source: &mut String,
    entry_path: &str,
) -> GalResult<()> {
    let mode = std::env::var("MATTMC_RUST_SELECTED_SOURCE_VERTEX_PROBE").ok();
    if matches!(
        mode.as_deref().map(str::trim),
        Some("direct-model-transform-water") | Some("clip-quad-water")
    ) && !entry_path.contains("water")
    {
        return Ok(());
    }
    apply_selected_source_vertex_position_probe_mode(source, mode.as_deref())
}

/// Capture-only position isolation. Every copied terrain quad has the stable
/// `[a,b,c,c,d,a]` source index grammar, so replacing the first quad of each
/// mesh asset by a small centered clip-space quad makes the existing indexed
/// draws cover a bounded diagnostic footprint without changing their fragment
/// program, resource sets, or attachments. Keeping both the footprint and
/// primitive count small is important: a quad per terrain primitive can turn
/// the diagnostic into a GPU-bound stress test and hide the result behind
/// presentation timeouts.
/// It distinguishes a transform/clip rejection from a color-output failure;
/// normal source execution never observes this environment key.
fn apply_selected_source_vertex_position_probe_mode(
    source: &mut String,
    mode: Option<&str>,
) -> GalResult<()> {
    match mode {
        None | Some("") => Ok(()),
        Some("constant-red") => {
            const ASSIGNMENT: &str = "glColorRaw = vulkanic_source_vertex_color;";
            if !source.contains(ASSIGNMENT) {
                return Err(GalError::invalid_argument(
                    "selected-source vertex probe could not locate glColorRaw assignment",
                ));
            }
            *source = source.replacen(
                ASSIGNMENT,
                "glColorRaw = vec4(1.0, 0.0, 0.0, 1.0); // selected-source vertex diagnostic probe: constant-red",
                1,
            );
            Ok(())
        }
        Some("clip-quad" | "clip-quad-water") => {
            let closing_brace = main_function_closing_brace(source).ok_or_else(|| {
                GalError::invalid_argument(
                    "selected-source vertex position probe requires a brace-balanced void main() body",
                )
            })?;
            let probe = r#"
#ifndef VULKANIC_SOURCE_PROBE_CORNERS_DEFINED
#define VULKANIC_SOURCE_PROBE_CORNERS_DEFINED
    const vec2 vulkanic_source_probe_corners[4] = vec2[4](
        vec2(-0.15, -0.15), vec2(0.15, -0.15), vec2(0.15, 0.15), vec2(-0.15, 0.15)
    );
#endif
    int vulkanic_source_probe_corner = gl_VertexIndex % 4;
    gl_Position = vec4(vulkanic_source_probe_corners[vulkanic_source_probe_corner], 0.0, 1.0);
"#;
            source.insert_str(closing_brace, probe);
            Ok(())
        }
        Some("raw-position") => {
            let closing_brace = main_function_closing_brace(source).ok_or_else(|| {
                GalError::invalid_argument(
                    "selected-source raw-position probe requires a brace-balanced void main() body",
                )
            })?;
            source.insert_str(
                closing_brace,
                "\n    // selected-source diagnostic probe: raw-position\n    gl_Position = vec4(vulkanic_source_position.xyz / 16.0, 1.0);\n",
            );
            Ok(())
        }
        Some("instance-translation") => {
            // Lowering may revisit an already-lowered module while rebuilding
            // the source pipeline cache.  Keep this capture-only probe
            // idempotent so a second pass cannot produce duplicate GLSL
            // declarations and turn a diagnostic run into a client crash.
            if source.contains("selected-source diagnostic probe: instance-translation") {
                return Ok(());
            }
            // Only the terrain/entity source families expose the instance
            // transform semantic.  Other source vertex modules (for example
            // gbuffers_textured) must remain untouched by this diagnostic.
            if !source.contains("vulkanic_source_model_transform") {
                return Ok(());
            }
            let closing_brace = main_function_closing_brace(source).ok_or_else(|| {
                GalError::invalid_argument(
                    "selected-source instance-translation probe requires a brace-balanced void main() body",
                )
            })?;
            let probe = r#"
    // selected-source diagnostic probe: instance-translation
    vec2 vulkanic_instance_probe_origin = vulkanic_source_model_transform[3].xy / 100.0;
    vec2 vulkanic_instance_probe_corner = vec2(
        (gl_VertexIndex & 1) != 0 ? 0.08 : -0.08,
        (gl_VertexIndex & 2) != 0 ? 0.08 : -0.08
    );
    gl_Position = vec4(vulkanic_instance_probe_origin + vulkanic_instance_probe_corner, 0.0, 1.0);
"#;
            source.insert_str(closing_brace, probe);
            Ok(())
        }
        Some("direct-transform") => {
            // This probe is meaningful only for the indexed terrain stream;
            // other source vertex families intentionally do not expose a
            // per-instance model-transform semantic.
            if !source.contains("vulkanic_source_model_transform")
                || !source.contains("gbufferProjection")
                || source.contains("shadowProjection *")
            {
                return Ok(());
            }
            let closing_brace = main_function_closing_brace(source).ok_or_else(|| {
                GalError::invalid_argument(
                    "selected-source direct-transform probe requires a brace-balanced void main() body",
                )
            })?;
            source.insert_str(
                closing_brace,
                "\n    gl_Position = gbufferProjection * vulkanic_source_model_view * vulkanic_source_position; // selected-source diagnostic probe: direct-transform\n",
            );
            Ok(())
        }
        Some("direct-model-transform") => {
            // Bypass the pack's legacy world-position reconstruction entirely
            // while retaining the explicit Rust-owned model/view/projection
            // matrices. This isolates a bad inverse/legacy expression from
            // the semantic instance and vertex streams.
            if !source.contains("vulkanic_source_model_transform")
                || !source.contains("gbufferProjection")
                || source.contains("shadowProjection *")
            {
                return Ok(());
            }
            let closing_brace = main_function_closing_brace(source).ok_or_else(|| {
                GalError::invalid_argument(
                    "selected-source direct-model-transform probe requires a brace-balanced void main() body",
                )
            })?;
            source.insert_str(
                closing_brace,
                "\n    gl_Position = gbufferProjection * gbufferModelView * vulkanic_source_model_transform * vulkanic_source_position; // selected-source diagnostic probe: direct-model-transform\n",
            );
            Ok(())
        }
        Some("direct-model-transform-water") => {
            if !source.contains("vulkanic_source_model_transform")
                || !source.contains("gbufferProjection")
            {
                return Ok(());
            }
            let closing_brace = main_function_closing_brace(source).ok_or_else(|| {
                GalError::invalid_argument(
                    "selected-source water transform probe requires a brace-balanced void main() body",
                )
            })?;
            source.insert_str(
                closing_brace,
                "\n    gl_Position = gbufferProjection * gbufferModelView * vulkanic_source_model_transform * vulkanic_source_position; // selected-source diagnostic probe: direct-model-transform-water\n",
            );
            Ok(())
        }
        Some("direct-model-transform-inline") => {
            // Replace only the source body's final projection assignment.
            // Unlike the ordinary direct-model probe (which appends after
            // the source body), this preserves source-side jitter and any
            // later semantic work while isolating the legacy position chain.
            if !source.contains("vulkanic_source_model_transform")
                || !source.contains("gbufferProjection")
                || source.contains("shadowProjection *")
            {
                return Ok(());
            }
            const LEGACY: &str =
                "gl_Position = gbufferProjection * gbufferModelView * position;";
            if source.contains(LEGACY) {
                *source = source.replacen(
                    LEGACY,
                    "gl_Position = gbufferProjection * gbufferModelView * vulkanic_source_model_transform * vulkanic_source_position; // selected-source diagnostic probe: direct-model-transform-inline",
                    1,
                );
            }
            Ok(())
        }
        Some("force-depth") => {
            let closing_brace = main_function_closing_brace(source).ok_or_else(|| {
                GalError::invalid_argument(
                    "selected-source hand depth probe requires a brace-balanced void main() body",
                )
            })?;
            source.insert_str(
                closing_brace,
                "\n    gl_Position.z = 0.0; // selected-source hand diagnostic probe: force-depth\n",
            );
            Ok(())
        }
        Some("clip-depth") => {
            if !source.contains("glColorRaw") {
                return Ok(());
            }
            let closing_brace = main_function_closing_brace(source).ok_or_else(|| {
                GalError::invalid_argument(
                    "selected-source clip-depth probe requires a brace-balanced void main() body",
                )
            })?;
            source.insert_str(
                closing_brace,
                "\n    // selected-source diagnostic probe: clip-depth\n    glColorRaw = vec4(clamp(gl_Position.z / max(abs(gl_Position.w), 0.0001), -1.0, 1.0) * 0.5 + 0.5, 0.0, 0.0, 1.0);\n",
            );
            Ok(())
        }
        Some("no-instance-transform") => {
            const NEEDLE: &str = "#define vulkanic_source_model_view (gbufferModelView * vulkanic_source_model_transform)";
            if source.contains(NEEDLE) {
                *source = source.replacen(
                    NEEDLE,
                    "#define vulkanic_source_model_view gbufferModelView // selected-source hand diagnostic probe: no-instance-transform",
                    1,
                );
            }
            Ok(())
        }
        Some("reverse-transform-order") => {
            const NEEDLE: &str = "#define vulkanic_source_model_view (gbufferModelView * vulkanic_source_model_transform)";
            if source.contains(NEEDLE) {
                *source = source.replacen(
                    NEEDLE,
                    "#define vulkanic_source_model_view (vulkanic_source_model_transform * gbufferModelView) // selected-source hand diagnostic probe: reverse-transform-order",
                    1,
                );
            }
            Ok(())
        }
        Some(other) => Err(GalError::invalid_argument(format!(
            "unknown selected-source vertex probe '{other}'; expected constant-red, clip-quad, raw-position, instance-translation, direct-transform, direct-model-transform, force-depth, clip-depth, no-instance-transform, or reverse-transform-order"
        ))),
    }
}

/// Capture-only shader-pack wave isolation. The Complementary terrain vertex
/// source mutates the reconstructed position through `DoWave`; replacing that
/// call with a no-op distinguishes pack animation semantics from the explicit
/// transform/attachment path. This is never enabled by normal execution.
fn apply_selected_source_wave_probe(source: &mut String) -> GalResult<()> {
    let Some(mode) = std::env::var("MATTMC_RUST_SELECTED_SOURCE_WAVE_PROBE")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };
    if mode != "disable" {
        return Err(GalError::invalid_argument(format!(
            "unknown selected-source wave probe '{mode}'; expected disable"
        )));
    }
    let assignment = "DoWave(position.xyz, mat);";
    if !source.contains(assignment) {
        // The selected profile may preprocess the waving branch away. In
        // that case there is no mutation to disable and the probe is a
        // semantic no-op; source admission must remain private and coherent.
        return Ok(());
    }
    *source = source.replacen(
        assignment,
        "// selected-source diagnostic probe: DoWave disabled",
        1,
    );
    Ok(())
}

fn lower_fullscreen_source_vertex_with_contracts(
    source: &PreprocessedShaderSource,
    uniform_contract: &TerrainSourceUniformContract,
    varying_contract: &TerrainSourceVaryingContract,
    opaque_resource_contract: &TerrainSourceOpaqueResourceContract,
    raster_primitive: FullscreenSourceRasterPrimitive,
) -> GalResult<LoweredFullscreenSourceVertex> {
    let mut lowered = upgrade_version(source.expanded_source())?;
    lowered = strip_nonopaque_uniforms(&lowered)?;
    let uses_legacy_fog = lower_legacy_fog(&mut lowered);
    lowered = replace_identifier(&lowered, "varying", "out");
    for (legacy, explicit) in [
        (
            "gl_TextureMatrix",
            "vulkanic_source_fullscreen_texture_matrix",
        ),
        ("gl_MultiTexCoord0", "vulkanic_source_fullscreen_uv"),
        (
            "gl_MultiTexCoord1",
            "vulkanic_source_fullscreen_secondary_uv",
        ),
        ("gl_Color", "vulkanic_source_fullscreen_vertex_color"),
        ("ftransform", "vulkanic_source_fullscreen_transform"),
    ] {
        lowered = replace_identifier(&lowered, legacy, explicit);
    }
    lowered = apply_varying_locations(&lowered, VaryingStorage::Out, varying_contract)?;
    lowered = apply_opaque_resource_bindings(&lowered, opaque_resource_contract)?;
    lowered = insert_after_version(
        &lowered,
        &fullscreen_vertex_semantic_preamble(raster_primitive),
    )?;
    if uses_legacy_fog {
        lowered = insert_after_version(&lowered, LEGACY_FOG_SEMANTIC_PREAMBLE)?;
    }
    // GLSL declarations must precede the procedural helper functions that
    // consume them. `insert_after_version` prepends each insertion, so stage
    // helpers are inserted first and the semantic scalar block last.
    lowered = insert_after_version(&lowered, &uniform_block(uniform_contract))?;
    // Fullscreen source stages are authored with the same OpenGL clip-depth
    // convention as terrain sources. Their procedural coverage must receive
    // the identical backend-selected finalization, otherwise a Vulkan stage
    // that reconstructs from gl_FragCoord sees a different depth convention.
    lowered = append_vulkan_clip_depth_finalizer(&lowered)?;
    let remaining_dialect = analyze_glsl_text(source.entry_path(), &lowered);
    Ok(LoweredFullscreenSourceVertex {
        entry_path: source.entry_path().to_string(),
        source: lowered,
        remaining_dialect,
    })
}

fn lower_fullscreen_source_fragment_with_contracts(
    source: &PreprocessedShaderSource,
    uniform_contract: &TerrainSourceUniformContract,
    varying_contract: &TerrainSourceVaryingContract,
    opaque_resource_contract: &TerrainSourceOpaqueResourceContract,
    bindings: &TerrainSourceResourceBindings,
) -> GalResult<LoweredFullscreenSourceFragment> {
    let mut lowered = upgrade_version(source.expanded_source())?;
    lowered = strip_nonopaque_uniforms(&lowered)?;
    let uses_legacy_fog = lower_legacy_fog(&mut lowered);
    for (legacy, explicit) in [
        ("texture2DLod", "textureLod"),
        ("texture3DLod", "textureLod"),
        ("textureCubeLod", "textureLod"),
        ("texture2D", "texture"),
        ("texture3D", "texture"),
        ("textureCube", "texture"),
        ("shadow2D", "vulkanic_source_shadow2D"),
    ] {
        lowered = replace_identifier(&lowered, legacy, explicit);
    }
    lowered = replace_identifier(&lowered, "varying", "in");
    lowered = apply_varying_locations(&lowered, VaryingStorage::In, varying_contract)?;
    lowered = apply_opaque_resource_bindings(&lowered, opaque_resource_contract)?;
    let draw_buffer_slots = parse_draw_buffers_slots(source.expanded_source())?;
    let mut outputs = Vec::new();
    for location in 0..8 {
        let provisional_name = format!("out_vulkanic_source_color_{location}");
        let (rewritten, occurrences) =
            replace_fragment_output(&lowered, location, &provisional_name)?;
        if occurrences == 0 {
            continue;
        }
        let source_slot = *draw_buffer_slots.get(location as usize).ok_or_else(|| {
            GalError::unsupported_feature(format!(
                "fullscreen source fragment '{}' writes gl_FragData[{location}] but DRAWBUFFERS declares only {} outputs",
                source.entry_path(),
                draw_buffer_slots.len()
            ))
        })?;
        let role = bindings.shader_pack_color_output_for_slot(source_slot)?;
        let Some(name) = role.shader_pack_color_name() else {
            unreachable!("shader_pack_color_output_for_slot returns a color role");
        };
        let semantic_name = format!("out_vulkanic_source_color_{name}");
        lowered = replace_identifier(&rewritten, &provisional_name, &semantic_name);
        outputs.push(FullscreenSourceFragmentOutput {
            source_location: location,
            source_slot,
            role,
            semantic_name,
        });
    }
    if contains_fragment_output(&lowered)? {
        return Err(GalError::unsupported_feature(format!(
            "fullscreen source fragment '{}' writes an output without a declared semantic color role",
            source.entry_path()
        )));
    }
    if outputs.is_empty() {
        return Err(GalError::invalid_argument(format!(
            "fullscreen source fragment '{}' has no semantic color output to lower",
            source.entry_path()
        )));
    }
    if outputs
        .iter()
        .enumerate()
        .any(|(expected, output)| output.source_location != expected as u32)
    {
        return Err(GalError::unsupported_feature(format!(
            "fullscreen source fragment '{}' uses sparse gl_FragData locations; compact source outputs are required",
            source.entry_path()
        )));
    }
    apply_selected_source_sky_fragment_probe(&mut lowered, source.entry_path())?;
    apply_selected_source_fullscreen_probe(&mut lowered, &outputs, source.entry_path())?;
    let declarations = outputs
        .iter()
        .map(|output| {
            format!(
                "layout(location = {}) out vec4 {};\n",
                output.source_location, output.semantic_name
            )
        })
        .collect::<String>();
    if uses_legacy_fog {
        lowered = insert_after_version(&lowered, LEGACY_FOG_SEMANTIC_PREAMBLE)?;
    }
    // Fullscreen source stages need two explicit coordinate domains. Source
    // math (fog, reconstruction, dithering) is authored around OpenGL's
    // lower-left gl_FragCoord, while source target texelFetch calls keep
    // naming Rust-owned image storage in its native address space. Depth
    // textures are the one exception: their readback row order is top-left,
    // so the depth fetches below are explicitly row-normalized.
    lowered = lower_fullscreen_fragment_coordinates(lowered, uniform_contract)?;
    // Fullscreen shader-pack stages commonly reconstruct view space from a
    // sampled depth value using the legacy OpenGL clip-depth mapping.  The
    // Vulkan depth attachment is zero-to-one, so normalize these source
    // reconstruction forms at the semantic lowering boundary while keeping
    // the OpenGL source expression intact behind the backend define.
    lowered = insert_after_version(&lowered, FRAGMENT_SEMANTIC_PREAMBLE)?;
    lowered = insert_after_version(&lowered, &uniform_block(uniform_contract))?;
    lowered = insert_after_version(&lowered, &declarations)?;
    let remaining_dialect = analyze_glsl_text(source.entry_path(), &lowered);
    Ok(LoweredFullscreenSourceFragment {
        entry_path: source.entry_path().to_string(),
        source: lowered,
        outputs,
        remaining_dialect,
    })
}

/// Preserves the source pack's lower-left fragment-space convention in
/// fullscreen stages without reinterpreting source-target image addresses.
///
/// Complementary's shared `common.glsl` derives `texelCoord` directly from
/// `gl_FragCoord` and uses it for `texelFetch`. Its fullscreen varying
/// `texCoord` is also dual-purpose: it samples Rust-owned target storage and
/// forms source-space view/reprojection coordinates. On Vulkan, the sampler
/// coordinate must remain vertically flipped while the reconstruction
/// coordinate must retain the source pack's lower-left convention. Rewrite
/// those distinct source forms explicitly instead of making either backend
/// interpretation leak into the pass graph.
fn lower_fullscreen_fragment_coordinates(
    mut source: String,
    uniform_contract: &TerrainSourceUniformContract,
) -> GalResult<String> {
    if !glsl_identifiers(&source).contains("gl_FragCoord") {
        return Ok(source);
    }
    if !uniform_contract
        .fields()
        .iter()
        .any(|field| field.name() == "viewHeight")
    {
        return Err(GalError::unsupported_feature(
            "fullscreen source reads gl_FragCoord but does not declare viewHeight for explicit coordinate conversion",
        ));
    }
    source = replace_identifier(
        &source,
        "gl_FragCoord",
        "vulkanic_source_fullscreen_fragment_coord()",
    );
    source = source.replace(
        "ivec2 texelCoord = ivec2(vulkanic_source_fullscreen_fragment_coord().xy);",
        "// Source texelCoord addresses the Rust-owned target storage, not the source-space screen direction.\n        ivec2 texelCoord = ivec2(gl_FragCoord.xy);",
    );
    for depth_texture in ["depthtex0", "depthtex1", "dhDepthTex"] {
        let native_fetch = format!("texelFetch({depth_texture}, texelCoord, 0)");
        let normalized_fetch = format!(
            "texelFetch({depth_texture}, ivec2(texelCoord.x, int(viewHeight) - 1 - texelCoord.y), 0)"
        );
        source = source.replace(&native_fetch, &normalized_fetch);
    }
    // `texCoord` intentionally retains the source sampler convention: V=0 is
    // the lower edge in the legacy OpenGL shader.  Position reconstruction is
    // a different semantic domain, however.  Convert only the screen-space
    // inputs used to rebuild view rays; otherwise deferred sky/cloud code sees
    // a vertically mirrored camera while texture sampling remains correct.
    for anchor in [
        "vec4 screenPos = vec4(texCoord,",
        "vec4 screenPosDH = vec4(texCoord,",
    ] {
        source = source.replace(
            anchor,
            &anchor.replace(
                "vec4(texCoord,",
                "vec4(vulkanic_source_fullscreen_screen_uv(texCoord),",
            ),
        );
    }
    // The fullscreen vertex semantic preamble has already converted the
    // interpolated UV into the source pack's screen domain. Re-flipping it
    // here would invert deferred reconstruction a second time while integer
    // texelFetch still addressed the correct target pixel. Preserve texCoord
    // for vec[34] reconstruction and leave sampler-specific lowering to the
    // explicit source-target sampling helpers.
    insert_after_version(
        &source,
        r#"vec4 vulkanic_source_fullscreen_fragment_coord() {
    vec4 coordinate = gl_FragCoord;
#ifdef VULKANIC_GAL_ZERO_TO_ONE_CLIP_DEPTH
    coordinate.y = viewHeight - coordinate.y;
#endif
    return coordinate;
}
vec2 vulkanic_source_fullscreen_screen_uv(vec2 image_uv) {
#ifdef VULKANIC_GAL_ZERO_TO_ONE_CLIP_DEPTH
    return vec2(image_uv.x, 1.0 - image_uv.y);
#else
    return image_uv;
#endif
}
"#,
    )
}

/// Capture-only diagnostic for the source-defined sky initializer. It keeps
/// the selected program, rasterization, and semantic UBO unchanged while
/// exposing the exact directional terms consumed by `GetSky`. This prevents
/// an apparently valid sky source from being mistaken for equivalent camera
/// reconstruction across backend coordinate conventions.
fn apply_selected_source_sky_fragment_probe(
    source: &mut String,
    entry_path: &str,
) -> GalResult<()> {
    let Some(mode) = std::env::var("MATTMC_RUST_SELECTED_SOURCE_SKY_FRAGMENT_PROBE")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };
    if !entry_path
        .replace('\\', "/")
        .ends_with("gbuffers_skybasic.fsh")
    {
        return Ok(());
    }
    let (anchor, replacement) = match mode.as_str() {
        "vectors" => (
            "color.rgb = GetSky(VdotU, VdotS, dither, true, false);",
            "color.rgb = vec3(clamp(VdotU * 0.5 + 0.5, 0.0, 1.0), clamp(VdotS * 0.5 + 0.5, 0.0, 1.0), clamp(dot(sunVec, upVec) * 0.5 + 0.5, 0.0, 1.0)); // selected-source sky diagnostic probe: vectors",
        ),
        "fragment-coordinates" => (
            "color.rgb = GetSky(VdotU, VdotS, dither, true, false);",
            "color.rgb = vec3(gl_FragCoord.xy / vec2(viewWidth, viewHeight), gl_FragCoord.z); // selected-source sky diagnostic probe: fragment-coordinates",
        ),
        other => {
            return Err(GalError::invalid_argument(format!(
                "unknown selected-source sky fragment probe '{other}'; expected vectors or fragment-coordinates"
            )));
        }
    };
    if !source.contains(anchor) {
        return Err(GalError::invalid_argument(
            "selected-source sky fragment probe could not locate the GetSky assignment",
        ));
    }
    *source = source.replacen(anchor, replacement, 1);
    Ok(())
}

/// Replaces the primary output of one lowered fullscreen source stage only
/// for an explicitly requested diagnostic capture. The probe remains at the
/// source-stage boundary: it reads the same named semantic resource and uses
/// the same procedural fullscreen geometry as the normal program.
fn apply_selected_source_fullscreen_probe(
    source: &mut String,
    outputs: &[FullscreenSourceFragmentOutput],
    entry_path: &str,
) -> GalResult<()> {
    let Some(mode) = std::env::var("MATTMC_RUST_SELECTED_SOURCE_FULLSCREEN_PROBE")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };
    if mode != "distant-horizons-depth"
        && mode != "distant-horizons-depth-routing"
        && mode != "distant-horizons-depth-coordinate"
        && mode != "distant-horizons-fog-inputs"
        && mode != "distant-horizons-fog-effect"
        && mode != "gbuffer-inputs"
        && mode != "gbuffer-primary"
        && mode != "depth-input"
        && mode != "depth-input-flipped"
        && mode != "depth-input-amplified"
        && mode != "deferred-fog-inputs"
        && mode != "composite7-without-fxaa"
    {
        return Err(GalError::invalid_argument(format!(
            "unknown selected-source fullscreen probe '{mode}'; expected distant-horizons-depth, distant-horizons-depth-routing, distant-horizons-depth-coordinate, distant-horizons-fog-inputs, distant-horizons-fog-effect, gbuffer-inputs, gbuffer-primary, depth-input, depth-input-flipped, depth-input-amplified, deferred-fog-inputs, or composite7-without-fxaa"
        )));
    }
    if mode == "composite7-without-fxaa" {
        if !entry_path
            .replace('\\', "/")
            .ends_with("world0/composite7.fsh")
        {
            return Ok(());
        }
        const FXAA_CALL: &str = "FXAA311(color);";
        if !source.contains(FXAA_CALL) {
            return Err(GalError::invalid_argument(
                "composite7 FXAA diagnostic could not locate the source FXAA call",
            ));
        }
        *source = source.replacen(
            FXAA_CALL,
            "/* selected-source fullscreen diagnostic: FXAA call suppressed */",
            1,
        );
        return Ok(());
    }
    // A selected-source pack has many fullscreen stages, including others
    // that incidentally declare these names. This probe is only meaningful
    // for Complementary's DH-aware deferred1 fog stage; all other programs
    // must lower unchanged so diagnostic capture cannot affect admission.
    if !entry_path
        .replace('\\', "/")
        .ends_with("world0/deferred1.fsh")
    {
        return Ok(());
    }
    if mode == "gbuffer-inputs" || mode == "gbuffer-primary" {
        let Some(primary) = outputs
            .iter()
            .find(|output| output.role.shader_pack_color_name() == Some("primary"))
        else {
            return Err(GalError::invalid_argument(
                "gbuffer input probe requires a primary color output",
            ));
        };
        const ASSIGNMENT: &str = "{} = vec4(color, 1.0);";
        let assignment = ASSIGNMENT.replacen("{}", &primary.semantic_name, 1);
        if !source.contains(&assignment) {
            return Err(GalError::invalid_argument(
                "gbuffer input probe could not locate the primary color assignment",
            ));
        }
        let replacement = if mode == "gbuffer-inputs" {
            format!("{} = vec4(texelFetch(colortex5, texelCoord, 0).rgb, texelFetch(colortex6, texelCoord, 0).r); // selected-source fullscreen diagnostic probe: gbuffer-inputs", primary.semantic_name)
        } else {
            format!("{} = vec4(texelFetch(colortex0, texelCoord, 0).rgb, 1.0); // selected-source fullscreen diagnostic probe: gbuffer-primary", primary.semantic_name)
        };
        *source = source.replacen(&assignment, &replacement, 1);
        return Ok(());
    }
    if mode == "depth-input" || mode == "depth-input-flipped" || mode == "depth-input-amplified" {
        let Some(primary) = outputs
            .iter()
            .find(|output| output.role.shader_pack_color_name() == Some("primary"))
        else {
            return Err(GalError::invalid_argument(
                "depth input probe requires a primary color output",
            ));
        };
        let assignment = format!("{} = vec4(color, 1.0);", primary.semantic_name);
        if !source.contains(&assignment) || !source.contains("float z0 =") {
            return Err(GalError::invalid_argument(
                "depth input probe could not locate deferred depth or primary output",
            ));
        }
        let expression = if mode == "depth-input-flipped" {
            "texelFetch(depthtex0, ivec2(texelCoord.x, int(viewHeight) - 1 - texelCoord.y), 0).r"
        } else if mode == "depth-input-amplified" {
            "fract(z0 * 1024.0)"
        } else {
            "z0"
        };
        *source = source.replacen(
            &assignment,
            &format!(
                "{} = vec4(vec3({expression}), 1.0); // selected-source fullscreen diagnostic probe: {mode}",
                primary.semantic_name
            ),
            1,
        );
        return Ok(());
    }
    if mode == "deferred-fog-inputs" {
        let Some(primary) = outputs
            .iter()
            .find(|output| output.role.shader_pack_color_name() == Some("primary"))
        else {
            return Err(GalError::invalid_argument(
                "deferred fog probe requires a primary color output",
            ));
        };
        const MAIN_DEPTH_DECLARATION: &str = "float z0 = texelFetch(depthtex0, texelCoord, 0).r;";
        if !source.contains(MAIN_DEPTH_DECLARATION) {
            return Err(GalError::invalid_argument(
                "deferred fog probe could not locate deferred depth declaration",
            ));
        }
        let assignment = format!("{} = vec4(color, 1.0);", primary.semantic_name);
        if !source.contains(&assignment) {
            return Err(GalError::invalid_argument(
                "deferred fog probe could not locate primary color assignment",
            ));
        }
        *source = source.replacen(
            MAIN_DEPTH_DECLARATION,
            "float z0 = texelFetch(depthtex0, texelCoord, 0).r; vec3 vulkanicDeferredFogInputs = vec3(z0, 0.0, 0.0);",
            1,
        );
        // Initialize the distance channel before the depth branch so a
        // far-plane sample (z0 == 1) still reports the reconstructed distance
        // instead of looking indistinguishable from an unexecuted probe.
        let view_distance = "float lViewPos = length(viewPos);";
        if !source.contains(view_distance) {
            return Err(GalError::invalid_argument(
                "deferred fog probe could not locate reconstructed view distance",
            ));
        }
        *source = source.replacen(
            view_distance,
            "float lViewPos = length(viewPos); vulkanicDeferredFogInputs = vec3(z0, clamp(lViewPos / max(far, 1.0), 0.0, 1.0), 0.0);",
            1,
        );
        // Capture the values after the ordinary-world fog path has established
        // skyFade and color; the probe does not alter the normal route.
        let fog_call = "DoFog(color.rgb, skyFade, lViewPos, playerPos, VdotU, VdotS, dither);";
        if !source.contains(fog_call) {
            return Err(GalError::invalid_argument(
                "deferred fog probe could not locate fog call",
            ));
        }
        *source = source.replacen(
            fog_call,
            "DoFog(color.rgb, skyFade, lViewPos, playerPos, VdotU, VdotS, dither); vulkanicDeferredFogInputs = vec3(z0, clamp(lViewPos / max(far, 1.0), 0.0, 1.0), clamp(skyFade, 0.0, 1.0));",
            1,
        );
        *source = source.replacen(
            &assignment,
            &format!(
                "{} = vec4(vulkanicDeferredFogInputs, 1.0); // selected-source fullscreen diagnostic probe: deferred-fog-inputs",
                primary.semantic_name
            ),
            1,
        );
        return Ok(());
    }
    if !source.contains("dhDepthTex") || !source.contains("texelCoord") {
        return Err(GalError::invalid_argument(
            "distant-horizons-depth fullscreen probe target lacks dhDepthTex or texelCoord",
        ));
    }
    let Some(primary) = outputs
        .iter()
        .find(|output| output.role.shader_pack_color_name() == Some("primary"))
    else {
        return Err(GalError::invalid_argument(
            "distant-horizons-depth fullscreen probe requires a primary color output",
        ));
    };
    match mode.as_str() {
        "distant-horizons-depth" => {
            let assignment = format!("{} = vec4(color, 1.0);", primary.semantic_name);
            if !source.contains(&assignment) {
                return Err(GalError::invalid_argument(
                    "distant-horizons-depth fullscreen probe could not locate the primary color assignment",
                ));
            }
            *source = source.replacen(
                &assignment,
                &format!(
                    "{} = vec4(vec3(texelFetch(dhDepthTex, texelCoord, 0).r), 1.0); // selected-source fullscreen diagnostic probe: distant-horizons-depth",
                    primary.semantic_name
                ),
                1,
            );
        }
        "distant-horizons-depth-routing" => {
            // `deferred1` chooses the DH path only where the main terrain
            // depth is clear and the DH depth is populated. Expose both
            // values and that exact predicate in one source-stage image.
            // This is capture-only evidence; it neither changes the normal
            // shader nor turns a depth mismatch into a rendering workaround.
            const MAIN_DEPTH_DECLARATION: &str =
                "float z0 = texelFetch(depthtex0, texelCoord, 0).r;";
            if !source.contains(MAIN_DEPTH_DECLARATION) {
                return Err(GalError::invalid_argument(
                    "distant-horizons depth-routing probe could not locate deferred1 main-depth declaration",
                ));
            }
            let assignment = format!("{} = vec4(color, 1.0);", primary.semantic_name);
            if !source.contains(&assignment) {
                return Err(GalError::invalid_argument(
                    "distant-horizons depth-routing probe could not locate the primary color assignment",
                ));
            }
            *source = source.replacen(
                MAIN_DEPTH_DECLARATION,
                "float z0 = texelFetch(depthtex0, texelCoord, 0).r; float vulkanicDhDepthProbe = texelFetch(dhDepthTex, texelCoord, 0).r;",
                1,
            );
            *source = source.replacen(
                &assignment,
                &format!(
                    "{} = vec4(z0, vulkanicDhDepthProbe, (z0 >= 1.0 && vulkanicDhDepthProbe < 1.0) ? 1.0 : 0.0, 1.0); // selected-source fullscreen diagnostic probe: distant-horizons-depth-routing",
                    primary.semantic_name
                ),
                1,
            );
        }
        "distant-horizons-depth-coordinate" => {
            // Keep this at the source-stage boundary and expose both the
            // texel selected by deferred1 and its vertically mirrored peer.
            // A DH depth image can be perfectly valid while a fullscreen
            // coordinate convention reads it upside down, so aggregate depth
            // images alone cannot distinguish these two cases.
            const MAIN_DEPTH_DECLARATION: &str =
                "float z0 = texelFetch(depthtex0, texelCoord, 0).r;";
            if !source.contains(MAIN_DEPTH_DECLARATION) {
                return Err(GalError::invalid_argument(
                    "distant-horizons depth-coordinate probe could not locate deferred1 main-depth declaration",
                ));
            }
            if !source.contains("viewHeight") {
                return Err(GalError::invalid_argument(
                    "distant-horizons depth-coordinate probe requires deferred1 viewHeight semantics",
                ));
            }
            insert_selected_source_distant_horizons_probe_state(source)?;
            replace_selected_source_primary_output(
                source,
                primary,
                "vec4(vulkanicDhDepthProbe, texelFetch(dhDepthTex, ivec2(texelCoord.x, int(viewHeight) - 1 - texelCoord.y), 0).r, texCoord.y, 1.0); // selected-source fullscreen diagnostic probe: distant-horizons-depth-coordinate",
            )?;
        }
        "distant-horizons-fog-inputs" => {
            // This replacement is deliberately inside deferred1's DH-only
            // sky branch. It exposes the exact semantic values consumed by
            // DoFog: depth, reconstructed distance relative to the pack's
            // DH render distance, and the source UV orientation. It cannot
            // turn into a production lighting or fog workaround.
            let fog_call = "DoFog(color.rgb, skyFade, lViewPos, playerPos, VdotU, VdotS, dither);";
            if !source.contains(fog_call) || !source.contains("dhRenderDistance") {
                return Err(GalError::invalid_argument(
                    "distant-horizons-fog-inputs fullscreen probe target lacks the DH fog call or dhRenderDistance",
                ));
            }
            insert_selected_source_distant_horizons_probe_state(source)?;
            replace_selected_source_distant_horizons_fog_call(
                source,
                fog_call,
                "vulkanicDhFogInputs = vec3(vulkanicDhDepthProbe, clamp(lViewPos / max(float(dhRenderDistance), 1.0), 0.0, 1.0), texCoord.y); DoFog(color.rgb, skyFade, lViewPos, playerPos, VdotU, VdotS, dither);",
            )?;
            replace_selected_source_primary_output(
                source,
                primary,
                "vec4(vulkanicDhFogInputs, 1.0); // selected-source fullscreen diagnostic probe: distant-horizons-fog-inputs",
            )?;
        }
        "distant-horizons-fog-effect" => {
            // Keep the real fog call, then encode its observable effect.
            // Logarithmic distance avoids the ordinary DH configuration
            // clamp hiding the difference between 64 and 1,024 blocks.
            let fog_call = "DoFog(color.rgb, skyFade, lViewPos, playerPos, VdotU, VdotS, dither);";
            if !source.contains(fog_call) {
                return Err(GalError::invalid_argument(
                    "distant-horizons-fog-effect fullscreen probe target lacks the DH fog call",
                ));
            }
            insert_selected_source_distant_horizons_probe_state(source)?;
            replace_selected_source_distant_horizons_fog_call(
                source,
                fog_call,
                "vec3 vulkanicFogInputColor = color.rgb; DoFog(color.rgb, skyFade, lViewPos, playerPos, VdotU, VdotS, dither); vulkanicDhFogInputs = vec3(vulkanicDhDepthProbe, clamp(log2(max(lViewPos, 1.0)) / 12.0, 0.0, 1.0), clamp(length(vulkanicFogInputColor - color.rgb), 0.0, 1.0));",
            )?;
            replace_selected_source_primary_output(
                source,
                primary,
                "vec4(vulkanicDhFogInputs, 1.0); // selected-source fullscreen diagnostic probe: distant-horizons-fog-effect",
            )?;
        }
        _ => unreachable!("validated selected-source fullscreen probe mode"),
    }
    Ok(())
}

fn insert_selected_source_distant_horizons_probe_state(source: &mut String) -> GalResult<()> {
    const MAIN_DEPTH_DECLARATION: &str = "float z0 = texelFetch(depthtex0, texelCoord, 0).r;";
    if !source.contains(MAIN_DEPTH_DECLARATION) {
        return Err(GalError::invalid_argument(
            "distant-horizons fullscreen probe could not locate deferred1 main-depth declaration",
        ));
    }
    *source = source.replacen(
        MAIN_DEPTH_DECLARATION,
        "float z0 = texelFetch(depthtex0, texelCoord, 0).r; float vulkanicDhDepthProbe = texelFetch(dhDepthTex, texelCoord, 0).r; vec3 vulkanicDhFogInputs = vec3(0.0);",
        1,
    );
    Ok(())
}

fn replace_selected_source_primary_output(
    source: &mut String,
    primary: &FullscreenSourceFragmentOutput,
    replacement: &str,
) -> GalResult<()> {
    let assignment = format!("{} = vec4(color, 1.0);", primary.semantic_name);
    if !source.contains(&assignment) {
        return Err(GalError::invalid_argument(
            "distant-horizons fullscreen probe could not locate deferred1 primary output",
        ));
    }
    *source = source.replacen(
        &assignment,
        &format!("{} = {replacement}", primary.semantic_name),
        1,
    );
    Ok(())
}

/// Replaces only `deferred1`'s DH-depth branch. The program has an earlier
/// ordinary-world fog call with the same source text; touching it would refer
/// to DH locals before they exist and turn a diagnostic into invalid shader
/// source.
fn replace_selected_source_distant_horizons_fog_call(
    source: &mut String,
    fog_call: &str,
    replacement: &str,
) -> GalResult<()> {
    const DH_DEPTH_BRANCH: &str = "if (z0DH < 1.0) { // Distant Horizons Chunks";
    let branch_start = source.find(DH_DEPTH_BRANCH).ok_or_else(|| {
        GalError::invalid_argument(
            "distant-horizons fullscreen probe could not locate deferred1's DH-depth branch",
        )
    })?;
    let call_offset = source[branch_start..].find(fog_call).ok_or_else(|| {
        GalError::invalid_argument(
            "distant-horizons fullscreen probe could not locate the DH-only fog call",
        )
    })?;
    let call_start = branch_start + call_offset;
    source.replace_range(call_start..call_start + fog_call.len(), replacement);
    Ok(())
}

fn vertex_semantic_preamble(transforms: SourceTransformSemantics) -> String {
    if transforms == SourceTransformSemantics::DistantHorizons {
        return DISTANT_HORIZONS_VERTEX_SEMANTIC_PREAMBLE.to_string();
    }
    if matches!(
        transforms,
        SourceTransformSemantics::TexturedMaterial
            | SourceTransformSemantics::Weather
            | SourceTransformSemantics::Cloud
    ) {
        return TEXTURED_MATERIAL_VERTEX_SEMANTIC_PREAMBLE.to_string();
    }
    VERTEX_SEMANTIC_PREAMBLE_TEMPLATE
        .replace("{model_view}", transforms.model_view_uniform())
        .replace("{model_view_expr}", transforms.model_view_expression())
        .replace("{projection}", transforms.projection_uniform())
}

const VERTEX_SEMANTIC_PREAMBLE_TEMPLATE: &str = r#"struct VulkanicSourceTerrainVertex {
    vec4 position;
    vec4 color;
    vec4 normal_light;
    vec4 atlas_uv_lightmap;
    vec4 entity;
    vec4 mid_tex_coord;
    vec4 tangent;
    vec4 mid_block;
};
layout(set = 0, binding = 0, std430) readonly buffer VulkanicSourceTerrainVertices {
    VulkanicSourceTerrainVertex vulkanic_source_vertices[];
};
layout(set = 0, binding = 1, std140) uniform VulkanicSourceTerrainLegacyTransforms {
    mat4 vulkanic_source_texture_matrix[2];
};
struct VulkanicSourceTerrainInstance {
    mat4 model_transform;
    vec4 color_modulation;
};
layout(set = 0, binding = 3, std430) readonly buffer VulkanicSourceTerrainInstances {
    VulkanicSourceTerrainInstance vulkanic_source_instances[];
};
#define vulkanic_source_vertex vulkanic_source_vertices[gl_VertexIndex]
#define vulkanic_source_instance vulkanic_source_instances[gl_InstanceIndex]
#define vulkanic_source_model_transform vulkanic_source_instance.model_transform
#define vulkanic_source_model_view {model_view_expr}
#define vulkanic_source_normal_matrix transpose(inverse(mat3(vulkanic_source_model_view)))
#define vulkanic_source_position vulkanic_source_vertex.position
#define vulkanic_source_vertex_color (vulkanic_source_vertex.color * vulkanic_source_instance.color_modulation)
#define vulkanic_source_normal vulkanic_source_vertex.normal_light.xyz
#define vulkanic_source_atlas_uv vec4(vulkanic_source_vertex.atlas_uv_lightmap.xy, 0.0, 1.0)
#define vulkanic_source_lightmap_uv vec4(vulkanic_source_vertex.atlas_uv_lightmap.zw, 0.0, 1.0)
#define vulkanic_source_entity vulkanic_source_vertex.entity
#define vulkanic_source_mid_tex_coord vulkanic_source_vertex.mid_tex_coord
#define vulkanic_source_tangent vulkanic_source_vertex.tangent
#define vulkanic_source_mid_block vulkanic_source_vertex.mid_block.xyz
#define vulkanic_source_ftransform() ({projection} * vulkanic_source_model_view * vulkanic_source_position)
"#;

/// Compact Rust-owned stream for selected `gbuffers_textured` programs. It
/// intentionally has no terrain material/entity/tangent lanes: the material
/// contract rejects programs requiring those inputs before this lowering can
/// be prepared. Positions are copied camera-relative semantics and therefore
/// use the selected source `gbufferModelView`/`gbufferProjection` uniforms
/// without a hidden Java pose-stack or Iris vertex-format dependency.
const TEXTURED_MATERIAL_VERTEX_SEMANTIC_PREAMBLE: &str = r#"struct VulkanicSourceTexturedMaterialVertex {
    vec4 position;
    vec4 color;
    vec4 normal_light;
    vec4 texture_uv_lightmap;
};
layout(set = 0, binding = 0, std430) readonly buffer VulkanicSourceTexturedMaterialVertices {
    VulkanicSourceTexturedMaterialVertex vulkanic_source_textured_vertices[];
};
layout(set = 0, binding = 1, std140) uniform VulkanicSourceTexturedMaterialLegacyTransforms {
    mat4 vulkanic_source_texture_matrix[2];
};
// The compact material stream stores four semantic vertices per quad. Source
// material draws expand those quads as two triangles in the owned vertex
// preamble, so no Java index buffer, backend base-vertex convention, or
// per-frame index upload is required.
const int vulkanic_source_textured_quad_indices[6] = int[6](0, 1, 2, 2, 3, 0);
#define vulkanic_source_vertex vulkanic_source_textured_vertices[((gl_VertexIndex / 6) * 4) + vulkanic_source_textured_quad_indices[gl_VertexIndex % 6]]
#define vulkanic_source_model_view gbufferModelView
#define vulkanic_source_normal_matrix transpose(inverse(mat3(vulkanic_source_model_view)))
#define vulkanic_source_position vulkanic_source_vertex.position
#define vulkanic_source_vertex_color vulkanic_source_vertex.color
#define vulkanic_source_normal vulkanic_source_vertex.normal_light.xyz
#define vulkanic_source_atlas_uv vec4(vulkanic_source_vertex.texture_uv_lightmap.xy, 0.0, 1.0)
#define vulkanic_source_lightmap_uv vec4(vulkanic_source_vertex.texture_uv_lightmap.zw, 0.0, 1.0)
#define vulkanic_source_ftransform() (gbufferProjection * vulkanic_source_model_view * vulkanic_source_position)
"#;

/// Rust-owned source interface for the copied DH CPU stream. The storage
/// layout matches `world_primitive_frontend::lod`'s 32-byte expanded vertex
/// record. The per-draw column origin comes from the owned LOD frame block;
/// source-declared `dhModelView`/`dhProjection` remain scalar semantic
/// uniforms and therefore must be supplied before a route can be admitted.
const DISTANT_HORIZONS_VERTEX_SEMANTIC_PREAMBLE: &str = r#"struct VulkanicDistantHorizonsVertex {
    float local_x;
    float local_y;
    float local_z;
    float micro_x;
    float micro_y;
    float micro_z;
    uint color_rgba;
    uint light_material_normal;
};
layout(set = 0, binding = 0, std430) readonly buffer VulkanicDistantHorizonsVertices {
    VulkanicDistantHorizonsVertex vulkanic_source_dh_vertices[];
};
layout(set = 0, binding = 1, std140) uniform VulkanicDistantHorizonsColumnFrame {
    mat4 vulkanic_source_dh_unused_combined_matrix;
    vec4 vulkanic_source_dh_column_origin_and_world_y;
    vec4 vulkanic_source_dh_model_offset_and_reserved;
    vec4 vulkanic_source_dh_clip_micro_noise_earth;
    uvec4 vulkanic_source_dh_flags_and_noise;
};
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
            // Iris's DHTerrainTransformer applies compact micro offsets only
            // on X/Z. Preserve the copied Y bits as semantic data, but do not
            // turn them into terrain height offsets in this source pass.
            + vec3(vulkanic_source_dh_vertex.micro_x, 0.0, vulkanic_source_dh_vertex.micro_z)
            // The model offset already contains the column's min-world-Y
            // relative to the camera. `worldYOffset` is source-pack scalar
            // context, not a second geometry translation.
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
        // Distant Horizons packs its source byte as `(skyLight, blockLight)`.
        // Iris's terrain transformer expands it as `(blockLight, skyLight)`
        // before the shader pack's lightmap conversion. Keep the generic
        // lowered stream aligned with the exact-atlas DH stream.
        float((vulkanic_source_dh_vertex.light_material_normal >> 8u) & 0xffu),
        float(vulkanic_source_dh_vertex.light_material_normal & 0xffu)
    ) + vec2(0.5)) / 16.0;
}
#define vulkanic_source_texture_matrix (mat4[2](mat4(1.0), mat4(1.0)))
#define vulkanic_source_lightmap_uv vec4(vulkanic_source_dh_packed_lightmap_coordinates(), 0.0, 1.0)
#define vulkanic_source_position vulkanic_source_dh_position()
#define vulkanic_source_vertex_color vulkanic_source_dh_vertex_color()
#define vulkanic_source_normal vulkanic_source_dh_normal((vulkanic_source_dh_vertex.light_material_normal >> 24u) & 0xffu)
// Complementary's dh_terrain contract consumes DH's own coarse material
// category (leaves/grass/lava/etc.), not a Minecraft atlas or a guessed block
// state. Keep that category in the copied semantic vertex stream so reduced
// mixed tiles remain representable without inventing per-quad identities.
#define vulkanic_source_dh_material_id int((vulkanic_source_dh_vertex.light_material_normal >> 16u) & 0xffu)
#define vulkanic_source_model_view dhModelView
#define vulkanic_source_normal_matrix transpose(inverse(mat3(vulkanic_source_model_view)))
#define vulkanic_source_ftransform() (dhProjection * vulkanic_source_model_view * vulkanic_source_position)
"#;

fn fullscreen_vertex_semantic_preamble(
    raster_primitive: FullscreenSourceRasterPrimitive,
) -> String {
    let geometry = match raster_primitive {
        FullscreenSourceRasterPrimitive::FullscreenTriangle => {
            r#"
vec2 vulkanic_source_fullscreen_position() {
    const vec2 positions[3] = vec2[3](
        vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0)
    );
    return positions[vulkanic_source_fullscreen_vertex_index()];
}
vec2 vulkanic_source_fullscreen_uv_coordinates() {
    const vec2 coordinates[3] = vec2[3](
        vec2(0.0, 0.0), vec2(2.0, 0.0), vec2(0.0, 2.0)
    );
    vec2 coordinate = coordinates[vulkanic_source_fullscreen_vertex_index()];
    // Legacy source `texture2D` calls use Minecraft's OpenGL texture origin.
    // Vulkan images are sampled with the opposite vertical convention, while
    // integer texelFetch keeps its native image addressing. Preserve the
    // source sampler contract here, at the owned fullscreen vertex boundary.
    #ifdef VULKANIC_GAL_ZERO_TO_ONE_CLIP_DEPTH
        coordinate.y = 1.0 - coordinate.y;
    #endif
    return coordinate;
}
#define vulkanic_source_fullscreen_transform() vec4(vulkanic_source_fullscreen_position(), 0.0, 1.0)
"#.to_string()
        }
        FullscreenSourceRasterPrimitive::VanillaSkyDisc => {
            r#"
// Expanded eight-wedge form of Minecraft's top SkyRenderer disc. The source
// program's legacy ftransform() sees the same local-space sky geometry rather
// than an approximation at synthetic fullscreen depth.
vec4 vulkanic_source_fullscreen_sky_position() {
    const vec3 outer[9] = vec3[9](
        vec3(-512.0, 16.0, 0.0),
        vec3(-362.03867, 16.0, -362.03867),
        vec3(0.0, 16.0, -512.0),
        vec3(362.03867, 16.0, -362.03867),
        vec3(512.0, 16.0, 0.0),
        vec3(362.03867, 16.0, 362.03867),
        vec3(0.0, 16.0, 512.0),
        vec3(-362.03867, 16.0, 362.03867),
        vec3(-512.0, 16.0, 0.0)
    );
    int vertex = vulkanic_source_fullscreen_vertex_index();
    int wedge = vertex / 3;
    int corner = vertex - wedge * 3;
    return corner == 0
        ? vec4(0.0, 16.0, 0.0, 1.0)
        : vec4(outer[wedge + corner - 1], 1.0);
}
vec2 vulkanic_source_fullscreen_uv_coordinates() { return vec2(0.0); }
#define vulkanic_source_fullscreen_transform() (gbufferProjection * gbufferModelView * vulkanic_source_fullscreen_sky_position())
"#.to_string()
        }
        FullscreenSourceRasterPrimitive::VanillaCelestialQuad => {
            r#"
// Matches SkyRenderer's indexed quad topology and semantic transform:
// Y(-90) * X(sunAngle) * Z(sunPathRotation) * translate * scale. The moon
// preserves vanilla's reversed position/UV ordering for its phase sheet.
int vulkanic_source_celestial_corner() {
    const int triangle_corners[6] = int[6](0, 1, 2, 0, 2, 3);
    return triangle_corners[vulkanic_source_fullscreen_vertex_index()];
}
vec3 vulkanic_source_rotate_x(vec3 value, float angle) {
    float s = sin(angle);
    float c = cos(angle);
    return vec3(value.x, c * value.y - s * value.z, s * value.y + c * value.z);
}
vec3 vulkanic_source_rotate_y(vec3 value, float angle) {
    float s = sin(angle);
    float c = cos(angle);
    return vec3(c * value.x + s * value.z, value.y, -s * value.x + c * value.z);
}
vec3 vulkanic_source_rotate_z(vec3 value, float angle) {
    float s = sin(angle);
    float c = cos(angle);
    return vec3(c * value.x - s * value.y, s * value.x + c * value.y, value.z);
}
vec4 vulkanic_source_fullscreen_celestial_position() {
    const vec2 sun_corners[4] = vec2[4](
        vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, 1.0)
    );
    const vec2 moon_corners[4] = vec2[4](
        vec2(-1.0, 1.0), vec2(1.0, 1.0), vec2(1.0, -1.0), vec2(-1.0, -1.0)
    );
    bool moon = vulkanic_source_celestial_is_moon != 0;
    vec2 corner = moon ? moon_corners[vulkanic_source_celestial_corner()]
                       : sun_corners[vulkanic_source_celestial_corner()];
    float size = moon ? 20.0 : 30.0;
    float height = moon ? -100.0 : 100.0;
    vec3 position = vec3(corner.x * size, height, corner.y * size);
    position = vulkanic_source_rotate_z(position, radians(vulkanic_source_celestial_sun_path_rotation));
    position = vulkanic_source_rotate_x(position, sunAngle * 6.28318530718);
    position = vulkanic_source_rotate_y(position, -1.57079632679);
    return vec4(position, 1.0);
}
vec2 vulkanic_source_fullscreen_uv_coordinates() {
    int corner = vulkanic_source_celestial_corner();
    bool moon = vulkanic_source_celestial_is_moon != 0;
    vec2 coordinate;
    if (moon) {
        int phase = clamp(moonPhase, 0, 7);
        float u0 = float(phase % 4) * 0.25;
        float v0 = float(phase / 4) * 0.5;
        const vec2 moon_uv[4] = vec2[4](
            vec2(1.0, 1.0), vec2(0.0, 1.0), vec2(0.0, 0.0), vec2(1.0, 0.0)
        );
        coordinate = vec2(u0, v0) + moon_uv[corner] * vec2(0.25, 0.5);
    } else {
        const vec2 sun_uv[4] = vec2[4](
            vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0)
        );
        coordinate = sun_uv[corner];
    }
#ifdef VULKANIC_GAL_ZERO_TO_ONE_CLIP_DEPTH
    coordinate.y = 1.0 - coordinate.y;
#endif
    return coordinate;
}
#define vulkanic_source_fullscreen_transform() (gbufferProjection * gbufferModelView * vulkanic_source_fullscreen_celestial_position())
"#.to_string()
        }
    };
    let vertex_color = match raster_primitive {
        // SkyRenderer builds its top disc with the extracted vanilla sky
        // color. The selected source's sky vertex uses this for its legacy
        // `gl_Color` semantic (including star discrimination), so it cannot
        // be replaced with the generic procedural white value.
        FullscreenSourceRasterPrimitive::VanillaSkyDisc => {
            "#define vulkanic_source_fullscreen_vertex_color vec4(skyColor, 1.0)"
        }
        FullscreenSourceRasterPrimitive::VanillaCelestialQuad => {
            "#define vulkanic_source_fullscreen_vertex_color vec4(1.0, 1.0, 1.0, vulkanic_source_celestial_alpha)"
        }
        FullscreenSourceRasterPrimitive::FullscreenTriangle => {
            "const vec4 vulkanic_source_fullscreen_vertex_color = vec4(1.0);"
        }
    };
    r#"// Source stages use Rust-owned procedural geometry. This avoids
// inheriting a Java/Iris vertex stream and stays compatible with the OpenGL
// backend's intentionally storage-buffer-only mesh path.
layout(set = 0, binding = 0, std140) uniform VulkanicSourceFullscreenFrame {
    mat4 vulkanic_source_fullscreen_texture_matrix[2];
};
int vulkanic_source_fullscreen_vertex_index() {
#ifdef VULKANIC_GAL_ZERO_TO_ONE_CLIP_DEPTH
    return gl_VertexIndex;
#else
    return gl_VertexID;
#endif
}
VULKANIC_SOURCE_FULLSCREEN_GEOMETRY
#define vulkanic_source_fullscreen_uv vec4(vulkanic_source_fullscreen_uv_coordinates(), 0.0, 1.0)
#define vulkanic_source_fullscreen_secondary_uv vec4(vulkanic_source_fullscreen_uv_coordinates(), 0.0, 1.0)
VULKANIC_SOURCE_FULLSCREEN_VERTEX_COLOR
"#
    .replace("VULKANIC_SOURCE_FULLSCREEN_GEOMETRY", &geometry)
    .replace("VULKANIC_SOURCE_FULLSCREEN_VERTEX_COLOR", vertex_color)
}

const LEGACY_FOG_SEMANTIC_PREAMBLE: &str = r#"struct VulkanicSourceLegacyFogParameters {
    vec4 color;
    float density;
    float start;
    float end;
    float scale;
};
VulkanicSourceLegacyFogParameters vulkanic_source_fog() {
    return VulkanicSourceLegacyFogParameters(
        vulkanic_source_fog_parameter_color,
        0.0,
        vulkanic_source_fog_environmental_start,
        vulkanic_source_fog_environmental_end,
        1.0 / (vulkanic_source_fog_environmental_end - vulkanic_source_fog_environmental_start)
    );
}
"#;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VaryingStorage {
    In,
    Out,
}

impl VaryingStorage {
    fn keyword(self) -> &'static str {
        match self {
            Self::In => "in",
            Self::Out => "out",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SourceVaryingDeclaration {
    name: String,
    type_name: String,
    interpolation: String,
}

/// Derives the exact simple field interface used by a vertex/fragment pair.
/// Missing or incompatible fragment inputs fail source preparation rather than
/// relying on compiler auto-assignment or an implicit legacy link convention.
pub fn derive_terrain_source_varying_contract(
    vertex: &PreprocessedShaderSource,
    fragment: &PreprocessedShaderSource,
) -> GalResult<TerrainSourceVaryingContract> {
    derive_simple_varying_contract(vertex.expanded_source(), fragment.expanded_source())
}

/// Shared source-level interface linking. No runtime renderer state is used.
pub(in crate::render::vulkanic) fn bind_simple_paired_varyings(vertex: &str, fragment: &str) -> GalResult<(String, String)> {
    let contract = derive_simple_varying_contract(vertex, fragment)?;
    if contract.fields.iter().any(|field| !matches!(field.type_name.as_str(),
        "float" | "vec2" | "vec3" | "vec4" | "int" | "ivec2" | "ivec3" | "ivec4"
        | "uint" | "uvec2" | "uvec3" | "uvec4")) {
        return Err(GalError::unsupported_feature("post-effect varying requires a multi-location interface contract"));
    }
    for (source, storage) in [(vertex, VaryingStorage::Out), (fragment, VaryingStorage::In)] {
        if !collect_stage_varyings(source, storage)?.is_empty()
            && source.lines().any(|line| line.trim().starts_with("layout")
                && line.contains(&format!(" {} ", storage.keyword()))) {
            return Err(GalError::unsupported_feature("post-effect mixed explicit and implicit varying locations are unavailable"));
        }
    }
    Ok((apply_varying_locations(vertex, VaryingStorage::Out, &contract)?,
        apply_varying_locations(fragment, VaryingStorage::In, &contract)?))
}

fn derive_simple_varying_contract(vertex: &str, fragment: &str) -> GalResult<TerrainSourceVaryingContract> {
    let mut vertex_outputs = BTreeMap::new();
    for field in collect_stage_varyings(vertex, VaryingStorage::Out)? {
        insert_stage_varying(&mut vertex_outputs, field, "vertex output")?;
    }
    let mut fragment_inputs = BTreeMap::new();
    for field in collect_stage_varyings(fragment, VaryingStorage::In)? {
        insert_stage_varying(&mut fragment_inputs, field, "fragment input")?;
    }

    for (name, fragment_field) in &fragment_inputs {
        let Some(vertex_field) = vertex_outputs.get(name) else {
            return Err(GalError::invalid_argument(format!(
                "terrain fragment input '{name}' has no matching vertex output"
            )));
        };
        if vertex_field.type_name != fragment_field.type_name
            || vertex_field.interpolation != fragment_field.interpolation
        {
            return Err(GalError::invalid_argument(format!(
                "terrain varying '{name}' differs between vertex output ('{} {}') and fragment input ('{} {}')",
                vertex_field.interpolation,
                vertex_field.type_name,
                fragment_field.interpolation,
                fragment_field.type_name,
            )));
        }
    }

    Ok(TerrainSourceVaryingContract {
        fields: vertex_outputs
            .into_values()
            .enumerate()
            .map(|(location, field)| TerrainSourceVaryingField {
                name: field.name,
                type_name: field.type_name,
                interpolation: field.interpolation,
                location: location as u32,
            })
            .collect(),
    })
}

fn insert_stage_varying(
    fields: &mut BTreeMap<String, SourceVaryingDeclaration>,
    field: SourceVaryingDeclaration,
    stage: &str,
) -> GalResult<()> {
    match fields.get(&field.name) {
        Some(existing) if existing != &field => Err(GalError::invalid_argument(format!(
            "terrain {stage} '{}' has incompatible repeated declarations",
            field.name
        ))),
        Some(_) => Ok(()),
        None => {
            fields.insert(field.name.clone(), field);
            Ok(())
        }
    }
}

fn collect_stage_varyings(
    source: &str,
    storage: VaryingStorage,
) -> GalResult<Vec<SourceVaryingDeclaration>> {
    let mut fields = Vec::new();
    for line in source.lines() {
        let Some(declarations) = parse_varying_declaration(line.trim(), storage)? else {
            continue;
        };
        fields.extend(declarations);
    }
    Ok(fields)
}

fn parse_varying_declaration(
    line: &str,
    storage: VaryingStorage,
) -> GalResult<Option<Vec<SourceVaryingDeclaration>>> {
    if !line.ends_with(';') || line.contains('(') || line.starts_with("layout") {
        return Ok(None);
    }
    let words = line
        .trim_end_matches(';')
        .split_whitespace()
        .collect::<Vec<_>>();
    let Some(storage_index) = words.iter().position(|word| *word == storage.keyword()) else {
        return Ok(None);
    };
    if storage_index > 1 || words.len() < storage_index + 3 {
        return Ok(None);
    }
    let interpolation = words[..storage_index].join(" ");
    if !interpolation.is_empty()
        && !matches!(interpolation.as_str(), "flat" | "smooth" | "noperspective")
    {
        return Err(GalError::unsupported_feature(format!(
            "terrain {} varying uses unsupported interpolation qualifier '{interpolation}'",
            storage.keyword()
        )));
    }
    let type_name = words[storage_index + 1];
    let names = words[storage_index + 2..].join(" ");
    if names.contains('[') || names.contains(']') {
        return Err(GalError::unsupported_feature(
            "terrain varying arrays need an explicit multi-location contract",
        ));
    }
    let mut declarations = Vec::new();
    for name in names.split(',').map(str::trim) {
        if name.is_empty() || !valid_identifier(name) {
            return Err(GalError::invalid_argument(format!(
                "terrain {} varying has invalid field name '{name}'",
                storage.keyword()
            )));
        }
        declarations.push(SourceVaryingDeclaration {
            name: name.to_string(),
            type_name: type_name.to_string(),
            interpolation: interpolation.clone(),
        });
    }
    Ok(Some(declarations))
}

fn apply_varying_locations(
    source: &str,
    storage: VaryingStorage,
    contract: &TerrainSourceVaryingContract,
) -> GalResult<String> {
    let mut output = String::with_capacity(source.len());
    for line in source.lines() {
        let Some(fields) = parse_varying_declaration(line.trim(), storage)? else {
            output.push_str(line);
            output.push('\n');
            continue;
        };
        if contract.location_for(&fields[0].name).is_none() {
            output.push_str(line);
            output.push('\n');
            continue;
        }
        for field in fields {
            let Some(location) = contract.location_for(&field.name) else {
                return Err(GalError::invalid_argument(format!(
                    "terrain {} varying '{}' is absent from the paired contract",
                    storage.keyword(),
                    field.name
                )));
            };
            if field.interpolation.is_empty() {
                output.push_str(&format!(
                    "layout(location = {location}) {} {} {};\n",
                    storage.keyword(),
                    field.type_name,
                    field.name
                ));
            } else {
                output.push_str(&format!(
                    "layout(location = {location}) {} {} {} {};\n",
                    field.interpolation,
                    storage.keyword(),
                    field.type_name,
                    field.name
                ));
            }
        }
    }
    Ok(output)
}

fn valid_identifier(name: &str) -> bool {
    name.bytes()
        .next()
        .is_some_and(|byte| byte == b'_' || byte.is_ascii_alphabetic())
        && name.bytes().all(is_identifier_byte)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SourceOpaqueResourceDeclaration {
    name: String,
    type_name: String,
    qualifiers: String,
    kind: TerrainSourceOpaqueResourceKind,
    active: bool,
}

/// Derives one deterministic table for source-declared samplers/images across
/// both stages. Explicitly unsupported declaration forms fail rather than
/// falling back to compiler-assigned bindings.
pub fn derive_terrain_source_opaque_resource_contract(
    vertex: &PreprocessedShaderSource,
    fragment: &PreprocessedShaderSource,
) -> GalResult<TerrainSourceOpaqueResourceContract> {
    let mut resources = BTreeMap::<String, SourceOpaqueResourceDeclaration>::new();
    for source in [vertex, fragment] {
        for resource in collect_opaque_resources(source.expanded_source())? {
            if let Some(existing) = resources.get(&resource.name) {
                if existing.type_name != resource.type_name
                    || existing.qualifiers != resource.qualifiers
                    || existing.kind != resource.kind
                {
                    return Err(GalError::invalid_argument(format!(
                        "terrain opaque resource '{}' has incompatible paired declarations",
                        resource.name
                    )));
                }
                let active = existing.active || resource.active;
                resources
                    .get_mut(&resource.name)
                    .expect("opaque resource entry disappeared during paired lowering")
                    .active = active;
            } else {
                resources.insert(resource.name.clone(), resource);
            }
        }
    }
    Ok(TerrainSourceOpaqueResourceContract {
        resources: resources
            .into_values()
            .enumerate()
            .map(|(binding, resource)| TerrainSourceOpaqueResource {
                name: resource.name,
                type_name: resource.type_name,
                qualifiers: resource.qualifiers,
                kind: resource.kind,
                binding: binding as u32,
                active: resource.active,
            })
            .collect(),
    })
}

fn collect_opaque_resources(source: &str) -> GalResult<Vec<SourceOpaqueResourceDeclaration>> {
    let source_without_declarations = strip_opaque_resource_declarations(source)?;
    let referenced = glsl_identifiers(&source_without_declarations);
    let mut resources = Vec::new();
    for line in source.lines() {
        let Some(mut resource) = parse_opaque_resource_declaration(line.trim())? else {
            continue;
        };
        resource.active = referenced.contains(&resource.name);
        resources.push(resource);
    }
    Ok(resources)
}

/// Removes opaque declarations before the bounded reference scan so a global
/// shader header cannot make a sampler appear required merely by declaring
/// it. Active preprocessor definitions remain in the source and therefore
/// count as uses conservatively, exactly as scalar-uniform collection does.
fn strip_opaque_resource_declarations(source: &str) -> GalResult<String> {
    let mut output = String::with_capacity(source.len());
    for line in source.lines() {
        if parse_opaque_resource_declaration(line.trim())?.is_some() {
            continue;
        }
        output.push_str(line);
        output.push('\n');
    }
    Ok(output)
}

fn parse_opaque_resource_declaration(
    line: &str,
) -> GalResult<Option<SourceOpaqueResourceDeclaration>> {
    if !line.ends_with(';') || line.contains('(') {
        return Ok(None);
    }
    if line.starts_with("layout") {
        if line.contains("sampler") || line.contains("image") {
            return Err(GalError::unsupported_feature(
                "terrain opaque resources with pre-existing layouts need an explicit source layout contract",
            ));
        }
        return Ok(None);
    }
    let words = line
        .trim_end_matches(';')
        .split_whitespace()
        .collect::<Vec<_>>();
    let Some(uniform_index) = words.iter().position(|word| *word == "uniform") else {
        return Ok(None);
    };
    if !words[..uniform_index].iter().all(|qualifier| {
        matches!(
            *qualifier,
            "readonly" | "writeonly" | "coherent" | "volatile" | "restrict"
        )
    }) {
        return Ok(None);
    }
    if words.len() != uniform_index + 3 || line.contains(',') || line.contains('[') {
        return Ok(None);
    }
    let type_name = words[uniform_index + 1];
    let kind = if type_name.contains("sampler") {
        TerrainSourceOpaqueResourceKind::CombinedTextureSampler
    } else if type_name.contains("image") {
        TerrainSourceOpaqueResourceKind::StorageImage
    } else {
        return Ok(None);
    };
    let name = words[uniform_index + 2];
    if !valid_identifier(name) {
        return Err(GalError::invalid_argument(format!(
            "terrain opaque resource has invalid name '{name}'"
        )));
    }
    let qualifiers = words[..uniform_index].join(" ");
    if !qualifiers.is_empty()
        && !qualifiers.split_whitespace().all(|qualifier| {
            matches!(
                qualifier,
                "readonly" | "writeonly" | "coherent" | "volatile" | "restrict"
            )
        })
    {
        return Err(GalError::unsupported_feature(format!(
            "terrain opaque resource '{name}' has unsupported qualifiers '{qualifiers}'"
        )));
    }
    Ok(Some(SourceOpaqueResourceDeclaration {
        name: name.to_string(),
        type_name: type_name.to_string(),
        qualifiers,
        kind,
        active: false,
    }))
}

fn apply_opaque_resource_bindings(
    source: &str,
    contract: &TerrainSourceOpaqueResourceContract,
) -> GalResult<String> {
    let mut output = String::with_capacity(source.len());
    for line in source.lines() {
        let Some(declaration) = parse_opaque_resource_declaration(line.trim())? else {
            output.push_str(line);
            output.push('\n');
            continue;
        };
        let Some(resource) = contract.resource_for(&declaration.name) else {
            return Err(GalError::invalid_argument(format!(
                "terrain opaque resource '{}' is absent from the paired contract",
                declaration.name
            )));
        };
        if declaration.qualifiers.is_empty() {
            output.push_str(&format!(
                "layout(set = 1, binding = {}) uniform {} {};\n",
                resource.binding, resource.type_name, resource.name
            ));
        } else {
            output.push_str(&format!(
                "layout(set = 1, binding = {}) {} uniform {} {};\n",
                resource.binding, resource.qualifiers, resource.type_name, resource.name
            ));
        }
    }
    Ok(output)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SourceUniformDeclaration {
    name: String,
    declaration: String,
}

/// Derives one ordered layout from both stages. Equal names must have exactly
/// equal declarations: accepting a type or array mismatch would make the
/// eventual UBO ABI ambiguous.
pub fn derive_terrain_source_uniform_contract(
    vertex: &PreprocessedShaderSource,
    fragment: &PreprocessedShaderSource,
) -> GalResult<TerrainSourceUniformContract> {
    derive_source_uniform_contract(vertex, fragment, SourceTransformSemantics::Terrain)
}

fn derive_source_uniform_contract(
    vertex: &PreprocessedShaderSource,
    fragment: &PreprocessedShaderSource,
    transforms: SourceTransformSemantics,
) -> GalResult<TerrainSourceUniformContract> {
    let mut declarations = BTreeMap::new();
    for source in [vertex, fragment] {
        for uniform in collect_nonopaque_uniforms(source.expanded_source(), transforms)? {
            match declarations.get(&uniform.name) {
                Some(existing) if existing != &uniform.declaration => {
                    return Err(GalError::invalid_argument(format!(
                        "terrain source uniform '{}' has incompatible paired declarations: '{}' versus '{}'",
                        uniform.name, existing, uniform.declaration
                    )));
                }
                Some(_) => {}
                None => {
                    declarations.insert(uniform.name, uniform.declaration);
                }
            }
        }
        for (name, declaration) in
            required_legacy_transform_uniforms(source.expanded_source(), transforms)
        {
            match declarations.get(name) {
                Some(existing) if existing != declaration => {
                    return Err(GalError::invalid_argument(format!(
                        "terrain source legacy transform '{}' must be declared as '{}' rather than '{}'",
                        name, declaration, existing
                    )));
                }
                Some(_) => {}
                None => {
                    declarations.insert(name.to_string(), declaration.to_string());
                }
            }
        }
        for (name, declaration) in required_legacy_fog_uniforms(source.expanded_source()) {
            match declarations.get(name) {
                Some(existing) if existing != declaration => {
                    return Err(GalError::invalid_argument(format!(
                        "terrain source legacy fog '{}' must be declared as '{}' rather than '{}'",
                        name, declaration, existing
                    )));
                }
                Some(_) => {}
                None => {
                    declarations.insert(name.to_string(), declaration.to_string());
                }
            }
        }
    }
    let declarations = declarations.into_values().collect::<Vec<_>>();
    let (fields, std140_size) = terrain_source_std140_layout(&declarations)?;
    Ok(TerrainSourceUniformContract {
        declarations,
        fields,
        std140_size,
    })
}

/// Fullscreen source stages usually have no camera geometry. The owned vanilla
/// sky disc is the one explicit exception: its legacy source program receives
/// a real model-view/projection transform and reconstructs rays from the
/// resulting fragment depth. Keep those two fields in the same semantic UBO
/// contract rather than sourcing them from Java/Iris state.
fn derive_fullscreen_source_uniform_contract(
    vertex: &PreprocessedShaderSource,
    fragment: &PreprocessedShaderSource,
    raster_primitive: FullscreenSourceRasterPrimitive,
) -> GalResult<TerrainSourceUniformContract> {
    let mut contract =
        derive_source_uniform_contract(vertex, fragment, SourceTransformSemantics::Fullscreen)?;
    // Fragment-coordinate source stages address an owned target in pixel
    // space. The runtime owns this viewport semantic even when the selected
    // stage reaches it through an include rather than a local declaration.
    if fragment.expanded_source().contains("gl_FragCoord") {
        ensure_fullscreen_uniform(&mut contract, "viewHeight", "float viewHeight;")?;
    }
    if !matches!(
        raster_primitive,
        FullscreenSourceRasterPrimitive::VanillaSkyDisc
            | FullscreenSourceRasterPrimitive::VanillaCelestialQuad
    ) {
        return Ok(contract);
    }
    for (name, declaration) in [
        ("gbufferModelView", "mat4 gbufferModelView;"),
        ("gbufferProjection", "mat4 gbufferProjection;"),
    ] {
        ensure_fullscreen_uniform(&mut contract, name, declaration)?;
    }
    if raster_primitive == FullscreenSourceRasterPrimitive::VanillaCelestialQuad {
        for (name, declaration) in [
            ("sunAngle", "float sunAngle;"),
            ("moonPhase", "int moonPhase;"),
            (
                "vulkanic_source_celestial_is_moon",
                "int vulkanic_source_celestial_is_moon;",
            ),
            (
                "vulkanic_source_celestial_alpha",
                "float vulkanic_source_celestial_alpha;",
            ),
            (
                "vulkanic_source_celestial_sun_path_rotation",
                "float vulkanic_source_celestial_sun_path_rotation;",
            ),
        ] {
            ensure_fullscreen_uniform(&mut contract, name, declaration)?;
        }
    }
    Ok(contract)
}

fn ensure_fullscreen_uniform(
    contract: &mut TerrainSourceUniformContract,
    name: &str,
    declaration: &str,
) -> GalResult<()> {
    let already_declared = contract
        .declarations
        .iter()
        .any(|existing| uniform_name(existing).is_ok_and(|existing_name| existing_name == name));
    if already_declared {
        return Ok(());
    }
    contract.declarations.push(declaration.to_string());
    contract.declarations.sort_by(|left, right| {
        uniform_name(left)
            .expect("validated fullscreen uniform declaration")
            .cmp(&uniform_name(right).expect("validated fullscreen uniform declaration"))
    });
    let (fields, std140_size) = terrain_source_std140_layout(&contract.declarations)?;
    contract.fields = fields;
    contract.std140_size = std140_size;
    Ok(())
}

const MAX_TERRAIN_SOURCE_UNIFORM_FIELDS: usize = 512;
const MAX_TERRAIN_SOURCE_UNIFORM_BYTES: u32 = 64 * 1024;

fn terrain_source_std140_layout(
    declarations: &[String],
) -> GalResult<(Vec<TerrainSourceUniformField>, u32)> {
    if declarations.len() > MAX_TERRAIN_SOURCE_UNIFORM_FIELDS {
        return Err(GalError::invalid_argument(format!(
            "terrain source declares {} scalar uniforms, exceeding {}",
            declarations.len(),
            MAX_TERRAIN_SOURCE_UNIFORM_FIELDS
        )));
    }
    let mut fields = Vec::with_capacity(declarations.len());
    let mut offset = 0_u32;
    for declaration in declarations {
        let (type_name, name, array_length) =
            parse_terrain_source_uniform_declaration(declaration)?;
        let ty = TerrainSourceUniformType::from_glsl(type_name)?;
        let alignment = if array_length > 1 {
            16
        } else {
            ty.std140_alignment()
        };
        offset = align_up_std140(offset, alignment)?;
        let element_size = ty.std140_size();
        let array_stride = if array_length > 1 {
            align_up_std140(element_size, 16)?
        } else {
            0
        };
        let size = if array_length > 1 {
            array_stride.checked_mul(array_length).ok_or_else(|| {
                GalError::invalid_argument("terrain source uniform array size overflows u32")
            })?
        } else {
            element_size
        };
        let end = offset.checked_add(size).ok_or_else(|| {
            GalError::invalid_argument("terrain source std140 layout size overflows u32")
        })?;
        if end > MAX_TERRAIN_SOURCE_UNIFORM_BYTES {
            return Err(GalError::invalid_argument(format!(
                "terrain source std140 scalar block exceeds {} bytes",
                MAX_TERRAIN_SOURCE_UNIFORM_BYTES
            )));
        }
        fields.push(TerrainSourceUniformField {
            name: name.to_string(),
            ty,
            array_length,
            offset,
            size,
            array_stride,
        });
        offset = end;
    }
    Ok((fields, align_up_std140(offset, 16)?))
}

fn parse_terrain_source_uniform_declaration(declaration: &str) -> GalResult<(&str, &str, u32)> {
    let declaration = declaration.trim().trim_end_matches(';');
    let mut parts = declaration.split_whitespace();
    let type_name = parts
        .next()
        .ok_or_else(|| GalError::invalid_argument("terrain uniform type is missing"))?;
    let name = parts
        .next()
        .ok_or_else(|| GalError::invalid_argument("terrain uniform name is missing"))?;
    if parts.next().is_some() {
        return Err(GalError::invalid_argument(
            "terrain uniform declaration has unsupported qualifiers or tokens",
        ));
    }
    let (name, array_length) = match name.split_once('[') {
        Some((name, suffix)) => {
            let count = suffix.strip_suffix(']').ok_or_else(|| {
                GalError::invalid_argument("terrain uniform array declaration is malformed")
            })?;
            let count = count.parse::<u32>().map_err(|_| {
                GalError::invalid_argument("terrain uniform array length is not a u32")
            })?;
            if count == 0 {
                return Err(GalError::invalid_argument(
                    "terrain uniform array length must be non-zero",
                ));
            }
            (name, count)
        }
        None => (name, 1),
    };
    if !valid_identifier(name) {
        return Err(GalError::invalid_argument(format!(
            "terrain uniform has invalid name '{name}'"
        )));
    }
    Ok((type_name, name, array_length))
}

fn align_up_std140(value: u32, alignment: u32) -> GalResult<u32> {
    debug_assert!(alignment.is_power_of_two());
    value
        .checked_add(alignment - 1)
        .map(|value| value & !(alignment - 1))
        .ok_or_else(|| GalError::invalid_argument("terrain source std140 alignment overflows u32"))
}

/// Removes scalar/vector/matrix uniforms after their paired declaration has
/// been captured. Samplers/images remain source-declared named resources for
/// a later binding contract.
fn strip_nonopaque_uniforms(source: &str) -> GalResult<String> {
    let mut output = String::with_capacity(source.len());
    for line in source.lines() {
        let trimmed = line.trim();
        let Some(declaration) = trimmed.strip_prefix("uniform ") else {
            output.push_str(line);
            output.push('\n');
            continue;
        };
        let type_name = declaration
            .split_whitespace()
            .next()
            .ok_or_else(|| GalError::invalid_argument("malformed terrain uniform declaration"))?;
        if is_opaque_uniform_type(type_name) {
            output.push_str(line);
            output.push('\n');
            continue;
        }
        validate_nonopaque_uniform_declaration(declaration)?;
    }
    Ok(output)
}

fn collect_nonopaque_uniforms(
    source: &str,
    transforms: SourceTransformSemantics,
) -> GalResult<Vec<SourceUniformDeclaration>> {
    // Expanded packs commonly include a broad global uniform header. Only a
    // source-stage reference belongs in this program's explicit ABI; merely
    // declaring a value in an inactive terrain path must not create a fake
    // semantic input requirement. Strip scalar declarations before scanning
    // so a declaration cannot count as its own use.
    let source_without_scalar_uniforms = strip_nonopaque_uniforms(source)?;
    let mut referenced = glsl_identifiers(&source_without_scalar_uniforms);
    // Vertex lowering replaces these legacy built-ins with explicit source
    // uniforms after the contract has been derived. Preserve their declared
    // semantic matrices when the original source requires the replacement.
    for (name, _) in required_legacy_transform_uniforms(source, transforms) {
        referenced.insert(name.to_string());
    }
    for (name, _) in required_legacy_fog_uniforms(source) {
        referenced.insert(name.to_string());
    }
    let mut uniforms = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim();
        let Some(declaration) = trimmed.strip_prefix("uniform ") else {
            continue;
        };
        let type_name = declaration
            .split_whitespace()
            .next()
            .ok_or_else(|| GalError::invalid_argument("malformed terrain uniform declaration"))?;
        if is_opaque_uniform_type(type_name) {
            continue;
        }
        let declaration = validate_nonopaque_uniform_declaration(declaration)?;
        let name = uniform_name(&declaration)?;
        if referenced.contains(&name) {
            uniforms.push(SourceUniformDeclaration { name, declaration });
        }
    }
    Ok(uniforms)
}

/// Legacy matrix built-ins are transformed into these named source semantics.
/// They are added to the uniform ABI even when the original GLSL relied on
/// built-ins and never declared them explicitly.
fn required_legacy_transform_uniforms(
    source: &str,
    transforms: SourceTransformSemantics,
) -> Vec<(&'static str, &'static str)> {
    if transforms == SourceTransformSemantics::Fullscreen {
        // Source fullscreen stages use their own fixed Rust-owned position/UV
        // stream and texture-matrix block. They do not inherit terrain or
        // shadow camera transforms merely because legacy GLSL spells
        // `ftransform()`.
        return Vec::new();
    }
    let referenced = glsl_identifiers(source);
    let mut requirements = Vec::with_capacity(2);
    if referenced.contains("gl_ModelViewMatrix")
        || referenced.contains("gl_NormalMatrix")
        || referenced.contains("ftransform")
    {
        requirements.push((
            transforms.model_view_uniform(),
            match transforms {
                SourceTransformSemantics::Terrain
                | SourceTransformSemantics::Entity
                | SourceTransformSemantics::Hand => "mat4 gbufferModelView;",
                SourceTransformSemantics::TexturedMaterial
                | SourceTransformSemantics::Weather
                | SourceTransformSemantics::Cloud => "mat4 gbufferModelView;",
                SourceTransformSemantics::Shadow => "mat4 shadowModelView;",
                SourceTransformSemantics::DistantHorizons => "mat4 dhModelView;",
                SourceTransformSemantics::Fullscreen => unreachable!(
                    "fullscreen source stages do not derive legacy camera transform uniforms"
                ),
            },
        ));
    }
    if referenced.contains("gl_ProjectionMatrix") || referenced.contains("ftransform") {
        requirements.push((
            transforms.projection_uniform(),
            match transforms {
                SourceTransformSemantics::Terrain
                | SourceTransformSemantics::Entity
                | SourceTransformSemantics::Hand => "mat4 gbufferProjection;",
                SourceTransformSemantics::TexturedMaterial
                | SourceTransformSemantics::Weather
                | SourceTransformSemantics::Cloud => "mat4 gbufferProjection;",
                SourceTransformSemantics::Shadow => "mat4 shadowProjection;",
                SourceTransformSemantics::DistantHorizons => "mat4 dhProjection;",
                SourceTransformSemantics::Fullscreen => unreachable!(
                    "fullscreen source stages do not derive legacy camera transform uniforms"
                ),
            },
        ));
    }
    if transforms == SourceTransformSemantics::DistantHorizons
        && referenced.contains("dhProjectionInverse")
    {
        requirements.push(("dhProjectionInverse", "mat4 dhProjectionInverse;"));
    }
    requirements
}

/// Legacy `gl_Fog` is a semantic fog record, not fixed-function backend
/// state. The lowered source receives the exact copied fog-parameter RGBA
/// and environmental range needed to construct its legacy fields.
fn required_legacy_fog_uniforms(source: &str) -> Vec<(&'static str, &'static str)> {
    if !glsl_identifiers(source).contains("gl_Fog") {
        return Vec::new();
    }
    vec![
        (
            "vulkanic_source_fog_parameter_color",
            "vec4 vulkanic_source_fog_parameter_color;",
        ),
        (
            "vulkanic_source_fog_environmental_start",
            "float vulkanic_source_fog_environmental_start;",
        ),
        (
            "vulkanic_source_fog_environmental_end",
            "float vulkanic_source_fog_environmental_end;",
        ),
    ]
}

/// Returns identifier tokens outside GLSL line/block comments. GLSL has no
/// string literals, so this bounded lexical scan is sufficient for deciding
/// whether a scalar declaration is referenced by the already-expanded source.
/// It deliberately treats identifiers in active preprocessor definitions as
/// references, which is conservative and avoids dropping macro-fed inputs.
fn glsl_identifiers(source: &str) -> BTreeSet<String> {
    let bytes = source.as_bytes();
    let mut identifiers = BTreeSet::new();
    let mut index = 0_usize;
    while index < bytes.len() {
        if bytes[index] == b'/' && index + 1 < bytes.len() {
            match bytes[index + 1] {
                b'/' => {
                    index += 2;
                    while index < bytes.len() && bytes[index] != b'\n' {
                        index += 1;
                    }
                    continue;
                }
                b'*' => {
                    index += 2;
                    while index + 1 < bytes.len()
                        && !(bytes[index] == b'*' && bytes[index + 1] == b'/')
                    {
                        index += 1;
                    }
                    index = (index + 2).min(bytes.len());
                    continue;
                }
                _ => {}
            }
        }
        if bytes[index] == b'_' || bytes[index].is_ascii_alphabetic() {
            let start = index;
            index += 1;
            while index < bytes.len()
                && (bytes[index] == b'_' || bytes[index].is_ascii_alphanumeric())
            {
                index += 1;
            }
            identifiers.insert(source[start..index].to_string());
            continue;
        }
        index += 1;
    }
    identifiers
}

fn validate_nonopaque_uniform_declaration(declaration: &str) -> GalResult<String> {
    let declaration = declaration.trim();
    if !declaration.ends_with(';') || declaration.contains('{') || declaration.contains(',') {
        return Err(GalError::unsupported_feature(
            "terrain scalar uniform declarations must be one named value ending in a semicolon",
        ));
    }
    Ok(declaration.to_string())
}

fn uniform_name(declaration: &str) -> GalResult<String> {
    let name = declaration
        .trim_end_matches(';')
        .split_whitespace()
        .last()
        .ok_or_else(|| GalError::invalid_argument("malformed terrain uniform declaration"))?
        .split('[')
        .next()
        .unwrap_or_default();
    if !valid_identifier(name) {
        return Err(GalError::invalid_argument(format!(
            "terrain uniform has invalid name '{name}'"
        )));
    }
    Ok(name.to_string())
}

fn is_opaque_uniform_type(type_name: &str) -> bool {
    type_name.contains("sampler")
        || type_name.contains("image")
        || type_name.starts_with("atomic_uint")
}

fn uniform_block(contract: &TerrainSourceUniformContract) -> String {
    if contract.declarations.is_empty() {
        return String::new();
    }
    let mut block = String::from(
        "layout(set = 0, binding = 2, std140) uniform VulkanicSourceTerrainUniforms {\n",
    );
    for declaration in &contract.declarations {
        block.push_str("    ");
        block.push_str(declaration);
        block.push('\n');
    }
    block.push_str("};\n");
    block
}

fn remove_known_legacy_attributes(source: &str) -> GalResult<String> {
    let mut output = String::with_capacity(source.len());
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(declaration) = trimmed.strip_prefix("attribute ") {
            let name = declaration
                .trim_end_matches(';')
                .split_whitespace()
                .last()
                .ok_or_else(|| GalError::invalid_argument("malformed legacy terrain attribute"))?;
            let name = name.trim_end_matches(|character| character == ';' || character == ']');
            if !matches!(
                name,
                "mc_Entity" | "mc_midTexCoord" | "at_tangent" | "at_midBlock"
            ) {
                return Err(GalError::unsupported_feature(format!(
                    "terrain vertex declares unsupported legacy attribute '{name}'"
                )));
            }
            continue;
        }
        output.push_str(line);
        output.push('\n');
    }
    Ok(output)
}

fn upgrade_version(source: &str) -> GalResult<String> {
    let lines = source.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return Err(GalError::invalid_argument("shader source is empty"));
    }
    // The owned preprocessor may inject semantic `#define`s ahead of the
    // source's root version. GLSL requires version first, so move exactly one
    // root directive to the prologue without dropping the configured defines.
    let mut output = String::from("#version 450\n");
    // Shader packs commonly leave the version to Iris's compile wrapper. The
    // copied, fully preprocessed source is still complete semantic input, so
    // Rust owns the target GLSL version when the pack did not declare one.
    let version_line = lines
        .iter()
        .position(|line| line.trim_start().starts_with("#version"));
    for (index, line) in lines.into_iter().enumerate() {
        if Some(index) == version_line {
            continue;
        }
        output.push_str(line);
        output.push('\n');
    }
    Ok(output)
}

fn insert_after_version(source: &str, declarations: &str) -> GalResult<String> {
    let Some(newline) = source.find('\n') else {
        return Err(GalError::invalid_argument(
            "lowered shader version line has no body",
        ));
    };
    let mut output = String::with_capacity(source.len() + declarations.len());
    output.push_str(&source[..newline + 1]);
    output.push_str(declarations);
    output.push_str(&source[newline + 1..]);
    Ok(output)
}

fn append_vulkan_clip_depth_finalizer(source: &str) -> GalResult<String> {
    let closing_brace = main_function_closing_brace(source).ok_or_else(|| {
        GalError::invalid_argument(
            "lowered source vertex shader has no brace-balanced void main() body for Vulkan clip-depth lowering",
        )
    })?;
    let mut output = String::with_capacity(source.len() + 160);
    output.push_str(&source[..closing_brace]);
    output.push_str("\n#ifdef VULKANIC_GAL_ZERO_TO_ONE_CLIP_DEPTH\n");
    output.push_str("    gl_Position.z = (gl_Position.z + gl_Position.w) * 0.5;\n");
    output.push_str("#endif\n");
    output.push_str(&source[closing_brace..]);
    Ok(output)
}

fn main_function_closing_brace(source: &str) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        cursor = skip_glsl_trivia(bytes, cursor)?;
        if !identifier_at(bytes, cursor, b"void") {
            cursor = cursor.saturating_add(1);
            continue;
        }
        let mut after_void = cursor + b"void".len();
        after_void = skip_glsl_trivia(bytes, after_void)?;
        if !identifier_at(bytes, after_void, b"main") {
            cursor = after_void;
            continue;
        }
        let mut signature = skip_glsl_trivia(bytes, after_void + b"main".len())?;
        if bytes.get(signature) != Some(&b'(') {
            cursor = signature;
            continue;
        }
        let mut parens = 0u32;
        loop {
            signature = skip_glsl_trivia(bytes, signature)?;
            match *bytes.get(signature)? {
                b'(' => parens = parens.checked_add(1)?,
                b')' => {
                    parens = parens.checked_sub(1)?;
                    if parens == 0 {
                        signature += 1;
                        break;
                    }
                }
                _ => {}
            }
            signature += 1;
        }
        let mut body = skip_glsl_trivia(bytes, signature)?;
        if bytes.get(body) != Some(&b'{') {
            cursor = body;
            continue;
        }
        let mut braces = 0u32;
        loop {
            body = skip_glsl_trivia(bytes, body)?;
            match *bytes.get(body)? {
                b'{' => braces = braces.checked_add(1)?,
                b'}' => {
                    braces = braces.checked_sub(1)?;
                    if braces == 0 {
                        return Some(body);
                    }
                }
                _ => {}
            }
            body += 1;
        }
    }
    None
}

fn skip_glsl_trivia(bytes: &[u8], mut cursor: usize) -> Option<usize> {
    loop {
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        if bytes.get(cursor..cursor + 2) == Some(b"//") {
            cursor += 2;
            while bytes.get(cursor).is_some_and(|byte| *byte != b'\n') {
                cursor += 1;
            }
            continue;
        }
        if bytes.get(cursor..cursor + 2) == Some(b"/*") {
            cursor += 2;
            while bytes.get(cursor..cursor + 2) != Some(b"*/") {
                bytes.get(cursor)?;
                cursor += 1;
            }
            cursor += 2;
            continue;
        }
        return Some(cursor);
    }
}

fn identifier_at(bytes: &[u8], start: usize, identifier: &[u8]) -> bool {
    let Some(end) = start.checked_add(identifier.len()) else {
        return false;
    };
    bytes.get(start..end) == Some(identifier)
        && !bytes
            .get(start.wrapping_sub(1))
            .is_some_and(is_glsl_identifier_byte)
        && !bytes.get(end).is_some_and(is_glsl_identifier_byte)
}

fn is_glsl_identifier_byte(byte: &u8) -> bool {
    byte.is_ascii_alphanumeric() || *byte == b'_'
}

fn replace_identifier(source: &str, from: &str, to: &str) -> String {
    let mut output = String::with_capacity(source.len());
    let bytes = source.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        let Some(relative) = source[cursor..].find(from) else {
            output.push_str(&source[cursor..]);
            break;
        };
        let start = cursor + relative;
        let end = start + from.len();
        let before = start == 0 || !is_identifier_byte(bytes[start - 1]);
        let after = end == bytes.len() || !is_identifier_byte(bytes[end]);
        output.push_str(&source[cursor..start]);
        if before && after {
            output.push_str(to);
        } else {
            output.push_str(from);
        }
        cursor = end;
    }
    output
}

/// Converts a source's legacy fog record to an explicit function backed by
/// named semantic uniforms. The returned flag controls whether the matching
/// source preamble must be emitted.
fn lower_legacy_fog(source: &mut String) -> bool {
    if !glsl_identifiers(source).contains("gl_Fog") {
        return false;
    }
    *source = replace_identifier(source, "gl_Fog", "vulkanic_source_fog()");
    true
}

fn replace_fragment_output(
    source: &str,
    index: u32,
    replacement: &str,
) -> GalResult<(String, u32)> {
    let needle = "gl_FragData";
    let bytes = source.as_bytes();
    let mut output = String::with_capacity(source.len());
    let mut cursor = 0;
    let mut occurrences = 0;
    while cursor < bytes.len() {
        let Some(relative) = source[cursor..].find(needle) else {
            output.push_str(&source[cursor..]);
            break;
        };
        let start = cursor + relative;
        let end = start + needle.len();
        output.push_str(&source[cursor..start]);
        if (start > 0 && is_identifier_byte(bytes[start - 1]))
            || (end < bytes.len() && is_identifier_byte(bytes[end]))
        {
            output.push_str(needle);
            cursor = end;
            continue;
        }
        let mut offset = end;
        skip_space(bytes, &mut offset);
        if bytes.get(offset) != Some(&b'[') {
            output.push_str(needle);
            cursor = end;
            continue;
        }
        offset += 1;
        skip_space(bytes, &mut offset);
        let digits_start = offset;
        while bytes.get(offset).is_some_and(u8::is_ascii_digit) {
            offset += 1;
        }
        let Some(found_index) = source[digits_start..offset].parse::<u32>().ok() else {
            return Err(GalError::unsupported_feature(
                "gl_FragData index is not a literal",
            ));
        };
        skip_space(bytes, &mut offset);
        if bytes.get(offset) != Some(&b']') {
            return Err(GalError::unsupported_feature(
                "malformed gl_FragData output index",
            ));
        }
        offset += 1;
        if found_index == index {
            output.push_str(replacement);
            occurrences += 1;
        } else {
            output.push_str(&source[start..offset]);
        }
        cursor = offset;
    }
    Ok((output, occurrences))
}

fn contains_fragment_output(source: &str) -> GalResult<bool> {
    let (_, occurrences) = replace_fragment_output(source, u32::MAX, "")?;
    Ok(occurrences > 0 || source.contains("gl_FragData"))
}

fn skip_space(bytes: &[u8], offset: &mut usize) {
    while bytes.get(*offset).is_some_and(u8::is_ascii_whitespace) {
        *offset += 1;
    }
}

fn is_identifier_byte(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphanumeric()
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, OnceLock};

    use super::*;
    use crate::render::vulkanic::shader_pack::preprocess::{preprocess_artifact, PreprocessInput};
    use crate::render::vulkanic::shader_pack::source::{ShaderPackSource, ShaderSourceFile};

    fn fullscreen_probe_test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn artifact(source: &str) -> PreprocessedShaderSource {
        let pack = ShaderPackSource::new(
            "test",
            1,
            vec![ShaderSourceFile::new("terrain.fsh", source)],
        )
        .unwrap();
        preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "terrain.fsh",
            defines: &[],
        })
        .unwrap()
    }

    #[test]
    fn lowers_legacy_fragment_outputs_to_named_semantics_without_touching_varyings() {
        let lowered = lower_terrain_fragment_surface(&artifact(
            "#version 130\nvarying vec2 uv; uniform sampler2D tex;\nvoid main() { gl_FragData[0] = texture2D(tex, uv); gl_FragData [ 1 ] = vec4(1.0); }",
        ))
        .unwrap();
        assert!(lowered.source().starts_with("#version 450\n"));
        assert!(lowered.source().contains("out_terrain_lit_color"));
        assert!(lowered.source().contains("out_terrain_material_auxiliary"));
        assert!(lowered.source().contains("texture(tex, uv)"));
        assert!(!lowered.source().contains("gl_FragData"));
        assert!(!lowered
            .remaining_dialect()
            .gaps()
            .contains(&super::super::dialect::GlslDialectGap::PreVulkanGlslVersion));
        assert!(lowered
            .remaining_dialect()
            .gaps()
            .contains(&super::super::dialect::GlslDialectGap::CompatibilityVertexAttributes));
    }

    #[test]
    fn selected_source_fragment_probe_replaces_only_the_named_color_outputs() {
        let mut lowered = lower_terrain_fragment_surface(&artifact(
            "#version 130\nuniform sampler2D tex; varying vec2 texCoord; void main() { vec4 color = texture2D(tex, texCoord); gl_FragData[0] = color; gl_FragData[1] = vec4(1.0); }",
        ))
        .unwrap();

        apply_selected_source_fragment_probe_mode(&mut lowered, Some("tint")).unwrap();

        assert!(lowered
            .source()
            .contains("out_terrain_lit_color = vec4(color.rgb * glColor.rgb, color.a);"));
        assert!(lowered
            .source()
            .contains("out_terrain_material_auxiliary = vec4(0.0);"));
        assert!(lowered
            .source()
            .contains("selected-source diagnostic probe: tint"));

        let mut vertex_color = lower_terrain_fragment_surface(&artifact(
            "#version 130\nuniform sampler2D tex; varying vec2 texCoord; varying vec4 glColor; void main() { vec4 color = texture2D(tex, texCoord); gl_FragData[0] = color; }",
        ))
        .unwrap();
        apply_selected_source_fragment_probe_mode(&mut vertex_color, Some("vertex-color")).unwrap();
        assert!(vertex_color
            .source()
            .contains("out_terrain_lit_color = glColor;"));

        let mut vertex_color_opaque = lower_terrain_fragment_surface(&artifact(
            "#version 130\nuniform sampler2D tex; varying vec2 texCoord; varying vec4 glColor; void main() { vec4 color = texture2D(tex, texCoord); gl_FragData[0] = color; }",
        ))
        .unwrap();
        apply_selected_source_fragment_probe_mode(
            &mut vertex_color_opaque,
            Some("vertex-color-opaque"),
        )
        .unwrap();
        assert!(vertex_color_opaque
            .source()
            .contains("out_terrain_lit_color = vec4(glColor.rgb, 1.0);"));

        let mut vertex_color_raw = lower_terrain_fragment_surface(&artifact(
            "#version 130\nuniform sampler2D tex; varying vec2 texCoord; varying vec4 glColorRaw; void main() { vec4 color = texture2D(tex, texCoord); gl_FragData[0] = color; }",
        ))
        .unwrap();
        apply_selected_source_fragment_probe_mode(&mut vertex_color_raw, Some("vertex-color-raw"))
            .unwrap();
        assert!(vertex_color_raw
            .source()
            .contains("out_terrain_lit_color = vec4(glColorRaw.rgb, 1.0);"));
    }

    #[test]
    fn selected_source_atlas_probe_reaches_water_style_colorp_sample() {
        let mut lowered = lower_terrain_fragment_surface(&artifact(
            "#version 130\nuniform sampler2D tex; varying vec2 texCoord; void main() { vec4 colorP = texture2D(tex, texCoord); gl_FragData[0] = colorP; gl_FragData[1] = vec4(1.0); }",
        ))
        .unwrap();
        apply_selected_source_fragment_probe_mode(&mut lowered, Some("atlas-alpha")).unwrap();
        assert!(lowered
            .source()
            .contains("out_terrain_lit_color = vec4(vec3(colorP.a), 1.0);"));
        assert!(!lowered
            .source()
            .contains("out_terrain_lit_color = vec4(vec3(color.a), 1.0);"));
    }

    #[test]
    fn selected_source_atlas_probe_uses_translucent_auxiliary_output() {
        let mut lowered = LoweredTranslucentTerrainFragmentSource {
            entry_path: "gbuffers_water.fsh".to_string(),
            source: "vec4 colorP = texture(tex, texCoord);".to_string(),
            outputs: Vec::new(),
            remaining_dialect: analyze_glsl_text("gbuffers_water.fsh", ""),
        };
        apply_selected_source_fragment_probe_mode(&mut lowered, Some("atlas-alpha")).unwrap();
        assert!(lowered
            .source
            .contains("out_terrain_translucency_auxiliary"));
        assert!(!lowered.source.contains("out_terrain_material_auxiliary"));
    }

    #[test]
    fn selected_source_water_only_probe_skips_opaque_entries() {
        let mut lowered = lower_terrain_fragment_surface(&artifact(
            "#version 130\nuniform sampler2D tex; varying vec2 texCoord; void main() { vec4 color = texture2D(tex, texCoord); gl_FragData[0] = color; }",
        ))
        .unwrap();
        apply_selected_source_fragment_probe_mode(&mut lowered, Some("constant-red-water"))
            .unwrap();
        assert!(!lowered
            .source()
            .contains("selected-source diagnostic probe"));
    }

    #[test]
    fn selected_source_vertex_probe_replaces_only_the_color_varying_assignment() {
        let mut source = "void main() { glColorRaw = vulkanic_source_vertex_color; }".to_string();
        apply_selected_source_vertex_position_probe_mode(&mut source, Some("constant-red"))
            .unwrap();
        assert!(source.contains("glColorRaw = vec4(1.0, 0.0, 0.0, 1.0);"));
        assert!(!source.contains("glColorRaw = vulkanic_source_vertex_color;"));
    }

    #[test]
    fn selected_source_direct_transform_probe_overrides_pack_position_after_main() {
        let mut source =
            "mat4 vulkanic_source_model_transform; mat4 gbufferProjection; void main() { gl_Position = ftransform(); }"
                .to_string();
        apply_selected_source_vertex_position_probe_mode(&mut source, Some("direct-transform"))
            .unwrap();
        assert!(source.contains(
            "gl_Position = gbufferProjection * vulkanic_source_model_view * vulkanic_source_position"
        ));
    }

    #[test]
    fn selected_source_instance_translation_probe_is_idempotent_and_semantic_guarded() {
        let mut source =
            "mat4 vulkanic_source_model_transform; void main() { gl_Position = vec4(0.0); }"
                .to_string();
        apply_selected_source_vertex_position_probe_mode(&mut source, Some("instance-translation"))
            .unwrap();
        let once = source.clone();
        apply_selected_source_vertex_position_probe_mode(&mut source, Some("instance-translation"))
            .unwrap();
        assert_eq!(once, source);

        let mut unrelated = "void main() { gl_Position = vec4(0.0); }".to_string();
        apply_selected_source_vertex_position_probe_mode(
            &mut unrelated,
            Some("instance-translation"),
        )
        .unwrap();
        assert_eq!(unrelated, "void main() { gl_Position = vec4(0.0); }");
    }

    #[test]
    fn selected_source_wave_probe_removes_only_the_pack_position_mutation() {
        let mut source =
            "void main() { DoWave(position.xyz, mat); gl_Position = ftransform(); }".to_string();
        std::env::set_var("MATTMC_RUST_SELECTED_SOURCE_WAVE_PROBE", "disable");
        apply_selected_source_wave_probe(&mut source).unwrap();
        std::env::remove_var("MATTMC_RUST_SELECTED_SOURCE_WAVE_PROBE");
        assert!(!source.contains("DoWave(position.xyz, mat);"));
        assert!(source.contains("gl_Position = ftransform();"));
    }

    #[test]
    fn selected_source_taa_probe_is_idempotent_and_only_replaces_jitter_assignment() {
        let mut source =
            "void main() { gl_Position.xy = TAAJitter(gl_Position.xy, gl_Position.w); }"
                .to_string();
        std::env::set_var("MATTMC_RUST_SELECTED_SOURCE_TAA_PROBE", "disable");
        apply_selected_source_taa_probe(&mut source).unwrap();
        let once = source.clone();
        apply_selected_source_taa_probe(&mut source).unwrap();
        std::env::remove_var("MATTMC_RUST_SELECTED_SOURCE_TAA_PROBE");
        assert_eq!(once, source);
        assert!(source.contains("selected-source diagnostic probe: TAA disabled"));
        assert!(!source.contains("TAAJitter(gl_Position.xy"));
    }

    #[test]
    fn selected_source_direct_model_transform_probe_uses_explicit_semantic_chain() {
        let mut source = "#version 130\nmat4 gbufferProjection;\nvec4 vulkanic_source_position;\nmat4 vulkanic_source_model_transform;\nmat4 gbufferModelView;\nvoid main() { gl_Position = ftransform(); }\n".to_string();
        apply_selected_source_vertex_position_probe_mode(
            &mut source,
            Some("direct-model-transform"),
        )
        .unwrap();
        assert!(source.contains(
            "gbufferProjection * gbufferModelView * vulkanic_source_model_transform * vulkanic_source_position"
        ));
    }

    #[test]
    fn selected_source_direct_model_transform_inline_replaces_source_assignment() {
        let mut source = "mat4 gbufferProjection; mat4 gbufferModelView; mat4 vulkanic_source_model_transform; vec4 vulkanic_source_position; void main() { vec4 position = vulkanic_source_model_transform * vulkanic_source_position; gl_Position = gbufferProjection * gbufferModelView * position; }".to_string();
        apply_selected_source_vertex_position_probe_mode(
            &mut source,
            Some("direct-model-transform-inline"),
        )
        .unwrap();
        assert!(source.contains(
            "gbufferProjection * gbufferModelView * vulkanic_source_model_transform * vulkanic_source_position"
        ));
        assert!(!source.contains("* position;"));
    }

    #[test]
    fn selected_source_direct_transform_probe_does_not_touch_shadow_projection() {
        let mut source = "mat4 gbufferProjection; mat4 shadowProjection; mat4 vulkanic_source_model_transform; void main() { gl_Position = shadowProjection * vulkanic_source_model_transform * vulkanic_source_position; }".to_string();
        let before = source.clone();
        apply_selected_source_vertex_position_probe_mode(&mut source, Some("direct-transform"))
            .unwrap();
        assert_eq!(before, source);
    }

    #[test]
    fn selected_source_fragment_lightmap_probe_preserves_the_shader_visible_inputs() {
        let mut lowered = lower_terrain_fragment_surface(&artifact(
            "#version 130\nuniform sampler2D tex; varying vec2 texCoord; varying vec2 lmCoord; varying vec4 glColor; void main() { vec4 color = texture2D(tex, texCoord); gl_FragData[0] = color; }",
        ))
        .unwrap();

        apply_selected_source_fragment_probe_mode(&mut lowered, Some("lightmap")).unwrap();

        assert!(lowered
            .source()
            .contains("out_terrain_lit_color = vec4(lmCoord.x, lmCoord.y, glColor.a, 1.0);"));
    }

    #[test]
    fn selected_source_fragment_light_probes_isolate_the_scene_components() {
        let source = "#version 130\nvec3 lightColorM; vec3 ambientColorM; vec3 shadowMult; vec3 ambientMult; void DoLighting(inout vec4 color) { vec3 sceneLighting = lightColorM * shadowMult + ambientColorM * ambientMult; color *= vec4(sceneLighting, 1.0); } void main() { vec4 color = vec4(1.0); DoLighting(color); gl_FragData[0] = color; }";
        let mut direct = lower_terrain_fragment_surface(&artifact(source)).unwrap();
        apply_selected_source_fragment_probe_mode(&mut direct, Some("light-color")).unwrap();
        assert!(direct.source().contains("color = vec4(lightColorM, 1.0);"));

        let atlas_source = "#version 130\nuniform sampler2D tex; vec2 texCoord; void main() { vec4 color = texture(tex, texCoord); if (color.a <= 0.00001) discard; gl_FragData[0] = color; }";
        let mut atlas_alpha = lower_terrain_fragment_surface(&artifact(atlas_source)).unwrap();
        apply_selected_source_fragment_probe_mode(&mut atlas_alpha, Some("atlas-alpha")).unwrap();
        assert!(atlas_alpha
            .source()
            .contains("out_terrain_lit_color = vec4(vec3(color.a), 1.0);"));

        let mut atlas_uv = lower_terrain_fragment_surface(&artifact(atlas_source)).unwrap();
        apply_selected_source_fragment_probe_mode(&mut atlas_uv, Some("atlas-uv")).unwrap();
        assert!(atlas_uv
            .source()
            .contains("out_terrain_lit_color = vec4(texCoord, 0.0, 1.0);"));

        let mut atlas_alpha_flipped =
            lower_terrain_fragment_surface(&artifact(atlas_source)).unwrap();
        apply_selected_source_fragment_probe_mode(
            &mut atlas_alpha_flipped,
            Some("atlas-alpha-flipped-v"),
        )
        .unwrap();
        assert!(atlas_alpha_flipped
            .source()
            .contains("texture(tex, vec2(texCoord.x, 1.0 - texCoord.y))"));

        let mut ambient = lower_terrain_fragment_surface(&artifact(source)).unwrap();
        apply_selected_source_fragment_probe_mode(&mut ambient, Some("ambient-color")).unwrap();
        assert!(ambient
            .source()
            .contains("color = vec4(ambientColorM, 1.0);"));

        let mut shadow = lower_terrain_fragment_surface(&artifact(source)).unwrap();
        apply_selected_source_fragment_probe_mode(&mut shadow, Some("shadow-mult")).unwrap();
        assert!(shadow.source().contains("color = vec4(shadowMult, 1.0);"));

        let final_light_source = "#version 130\nfloat darknessLightFactor; vec3 finalDiffuse; vec3 color; float pow2(float value) { return value * value; } void DoLighting() { color.rgb *= finalDiffuse; color.rgb *= pow2(1.0 - darknessLightFactor); } void main() { DoLighting(); gl_FragData[0] = vec4(color, 1.0); }";
        let mut diffuse = lower_terrain_fragment_surface(&artifact(final_light_source)).unwrap();
        apply_selected_source_fragment_probe_mode(&mut diffuse, Some("final-diffuse")).unwrap();
        assert!(diffuse
            .source()
            .contains("color = vec4(finalDiffuse, 1.0);"));

        let factor_source = "#version 130\nfloat directionShade; float vanillaAO; vec3 sceneLighting; vec3 lightColorM; vec3 ambientColorM; vec3 shadowMult; vec3 ambientMult; vec3 blockLighting; vec3 minLighting; vec3 emission; float pow2(float value) { return value * value; } void DoLighting(inout vec4 color) { vec3 finalDiffuse = pow2(directionShade * vanillaAO) * (blockLighting + pow2(sceneLighting) + minLighting) + pow2(emission); color.rgb *= finalDiffuse; } void main() { vec4 color = vec4(1.0); DoLighting(color); gl_FragData[0] = color; }";
        let mut factors = lower_terrain_fragment_surface(&artifact(factor_source)).unwrap();
        apply_selected_source_fragment_probe_mode(&mut factors, Some("lighting-factors")).unwrap();
        assert!(factors.source().contains("clamp(directionShade, 0.0, 1.0)"));
        assert!(factors.source().contains("clamp(vanillaAO, 0.0, 1.0)"));

        let mut components_a = lower_terrain_fragment_surface(&artifact(factor_source)).unwrap();
        apply_selected_source_fragment_probe_mode(&mut components_a, Some("lighting-components-a"))
            .unwrap();
        assert!(components_a
            .source()
            .contains("max(max(lightColorM.r, lightColorM.g), lightColorM.b)"));
        assert!(components_a
            .source()
            .contains("max(max(ambientColorM.r, ambientColorM.g), ambientColorM.b)"));

        let mut components_b = lower_terrain_fragment_surface(&artifact(factor_source)).unwrap();
        apply_selected_source_fragment_probe_mode(&mut components_b, Some("lighting-components-b"))
            .unwrap();
        assert!(components_b
            .source()
            .contains("clamp(ambientMult, 0.0, 1.0)"));
        assert!(components_b
            .source()
            .contains("max(max(blockLighting.r, blockLighting.g), blockLighting.b)"));

        let mut darkness = lower_terrain_fragment_surface(&artifact(final_light_source)).unwrap();
        apply_selected_source_fragment_probe_mode(&mut darkness, Some("darkness-scale")).unwrap();
        assert!(darkness
            .source()
            .contains("color = vec4(vec3(pow2(1.0 - darknessLightFactor)), 1.0);"));

        let shadow_sampling_source = "#version 130\nfloat shadow0; vec3 shadowcol; vec3 SampleShadow() { return shadowcol * (1.0 - shadow0) + shadow0; } void main() { gl_FragData[0] = vec4(SampleShadow(), 1.0); }";
        let mut primary =
            lower_terrain_fragment_surface(&artifact(shadow_sampling_source)).unwrap();
        apply_selected_source_fragment_probe_mode(&mut primary, Some("shadow-primary")).unwrap();
        assert!(primary
            .source()
            .contains("return vec3(shadow0); // selected-source diagnostic probe: shadow-primary"));

        let shadow_source = "#version 130\nuniform sampler2DShadow shadowtex0; vec3 GetShadowPos(vec3 playerPos) { return playerPos; } void DoLighting(inout vec4 color) { vec3 playerPosM = vec3(0.25); vec3 shadowPos = GetShadowPos(playerPosM); color *= vec4(shadowPos, 1.0); } void main() { vec4 color = vec4(1.0); DoLighting(color); gl_FragData[0] = color; }";
        let mut coordinates = lower_terrain_fragment_surface(&artifact(shadow_source)).unwrap();
        apply_selected_source_fragment_probe_mode(&mut coordinates, Some("shadow-coordinate"))
            .unwrap();
        assert!(coordinates
            .source()
            .contains("color = vec4(shadowPos, 1.0);"));

        let mut centered = lower_terrain_fragment_surface(&artifact(shadow_source)).unwrap();
        apply_selected_source_fragment_probe_mode(
            &mut centered,
            Some("shadow-coordinate-centered"),
        )
        .unwrap();
        assert!(centered
            .source()
            .contains("color = vec4(clamp(shadowPos * 0.5 + 0.5, 0.0, 1.0), 1.0);"));

        let player_position_source = "#version 130\nvec3 viewPos; vec3 ViewToPlayer(vec3 value) { return value; } void main() { vec3 playerPos = ViewToPlayer(viewPos); gl_FragData[0] = vec4(playerPos, 1.0); }";
        let mut player_position =
            lower_terrain_fragment_surface(&artifact(player_position_source)).unwrap();
        apply_selected_source_fragment_probe_mode(&mut player_position, Some("player-position"))
            .unwrap();
        assert!(player_position.source().contains(
            "out_terrain_lit_color = vec4(clamp(playerPos / 384.0 + 0.5, 0.0, 1.0), 1.0);"
        ));

        let mut reconstruction =
            lower_terrain_fragment_surface(&artifact(player_position_source)).unwrap();
        apply_selected_source_fragment_probe_mode(&mut reconstruction, Some("reconstruction"))
            .unwrap();
        assert!(reconstruction.source().contains(
            "out_terrain_lit_color = vec4(gl_FragCoord.z, clamp(playerPos.y / 384.0 + 0.5, 0.0, 1.0), clamp(length(viewPos) / 384.0, 0.0, 1.0), 1.0);"
        ));

        let mut viewport =
            lower_terrain_fragment_surface(&artifact(player_position_source)).unwrap();
        apply_selected_source_fragment_probe_mode(&mut viewport, Some("viewport-uniforms"))
            .unwrap();
        assert!(viewport.source().contains("atan(viewWidth / 1024.0)"));
        assert!(viewport.source().contains("atan(viewHeight / 1024.0)"));
        assert!(viewport.source().contains("gl_FragCoord.x / 1280.0"));

        let mut matrix_basis =
            lower_terrain_fragment_surface(&artifact(player_position_source)).unwrap();
        apply_selected_source_fragment_probe_mode(&mut matrix_basis, Some("matrix-basis")).unwrap();
        assert!(matrix_basis
            .source()
            .contains("gbufferProjectionInverse[3][2]"));

        let mut constant_red = lower_terrain_fragment_surface(&artifact(source)).unwrap();
        apply_selected_source_fragment_probe_mode(&mut constant_red, Some("constant-red")).unwrap();
        assert!(constant_red
            .source()
            .contains("out_terrain_lit_color = vec4(1.0, 0.0, 0.0, 1.0);"));

        let mut compare = lower_terrain_fragment_surface(&artifact(shadow_source)).unwrap();
        apply_selected_source_fragment_probe_mode(&mut compare, Some("shadow-compare")).unwrap();
        assert!(compare
            .source()
            .contains("vulkanic_source_shadow2D(shadowtex0"));
    }

    #[test]
    fn selected_source_fragment_probe_rejects_unknown_mode() {
        let mut lowered = lower_terrain_fragment_surface(&artifact(
            "#version 130\nuniform sampler2D tex; varying vec2 texCoord; void main() { vec4 color = texture2D(tex, texCoord); gl_FragData[0] = color; }",
        ))
        .unwrap();

        let error =
            apply_selected_source_fragment_probe_mode(&mut lowered, Some("wrong")).unwrap_err();
        assert!(error
            .to_string()
            .contains("unknown selected-source fragment probe 'wrong'"));
    }

    #[test]
    fn selected_source_entity_probe_observes_only_local_texture_or_uv() {
        let mut lowered = lower_terrain_fragment_surface(&artifact(
            "#version 130\nuniform sampler2D tex; varying vec2 texCoord; void main() { vec4 color = texture2D(tex, texCoord); gl_FragData[0] = color; gl_FragData[1] = color; }",
        ))
        .unwrap();
        apply_selected_source_entity_fragment_probe_mode(&mut lowered, Some("texture")).unwrap();
        assert!(lowered.source().contains("out_terrain_lit_color = color;"));
        assert!(lowered
            .source()
            .contains("selected-source entity diagnostic probe: texture"));

        let mut uv = lower_terrain_fragment_surface(&artifact(
            "#version 130\nuniform sampler2D tex; varying vec2 texCoord; void main() { vec4 color = texture2D(tex, texCoord); gl_FragData[0] = color; gl_FragData[1] = color; }",
        ))
        .unwrap();
        apply_selected_source_entity_fragment_probe_mode(&mut uv, Some("uv")).unwrap();
        assert!(uv
            .source()
            .contains("out_terrain_lit_color = vec4(texCoord, 0.0, 1.0);"));

        let error =
            apply_selected_source_entity_fragment_probe_mode(&mut uv, Some("wrong")).unwrap_err();
        assert!(error
            .to_string()
            .contains("unknown selected-source entity fragment probe 'wrong'"));
    }

    #[test]
    fn distant_horizons_only_pre_lighting_probe_leaves_normal_terrain_unmodified() {
        let mut lowered = lower_terrain_fragment_surface(&artifact(
            "#version 130\nuniform sampler2D tex; varying vec2 texCoord; void main() { vec4 color = texture2D(tex, texCoord); gl_FragData[0] = color; }",
        ))
        .unwrap();
        let before = lowered.source().to_string();

        apply_selected_source_fragment_probe_mode(&mut lowered, Some("pre-lighting")).unwrap();

        assert_eq!(lowered.source(), before);
    }

    #[test]
    fn fullscreen_depth_probe_reads_the_named_distant_depth_at_the_primary_output() {
        let _guard = fullscreen_probe_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let source = "#version 130\nuniform sampler2D dhDepthTex; ivec2 texelCoord; vec3 color; void main() { gl_FragData[0] = vec4(color, 1.0); }";
        let mut lowered = source.to_string();
        let outputs = vec![FullscreenSourceFragmentOutput {
            source_location: 0,
            source_slot: 0,
            role: TerrainSourceResourceRole::ShaderPackColor("primary".to_string()),
            semantic_name: "out_vulkanic_source_color_primary".to_string(),
        }];
        lowered = lowered.replace("gl_FragData[0]", "out_vulkanic_source_color_primary");
        std::env::set_var(
            "MATTMC_RUST_SELECTED_SOURCE_FULLSCREEN_PROBE",
            "distant-horizons-depth",
        );
        let result =
            apply_selected_source_fullscreen_probe(&mut lowered, &outputs, "world0/deferred1.fsh");
        std::env::remove_var("MATTMC_RUST_SELECTED_SOURCE_FULLSCREEN_PROBE");
        result.unwrap();
        assert!(lowered.contains(
            "out_vulkanic_source_color_primary = vec4(vec3(texelFetch(dhDepthTex, texelCoord, 0).r), 1.0);"
        ));
    }

    #[test]
    fn fullscreen_depth_routing_probe_exposes_the_exact_deferred_dh_predicate() {
        let _guard = fullscreen_probe_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut lowered = "uniform sampler2D depthtex0; uniform sampler2D dhDepthTex; ivec2 texelCoord; vec3 color; void main() { float z0 = texelFetch(depthtex0, texelCoord, 0).r; out_vulkanic_source_color_primary = vec4(color, 1.0); }".to_string();
        let outputs = vec![FullscreenSourceFragmentOutput {
            source_location: 0,
            source_slot: 0,
            role: TerrainSourceResourceRole::ShaderPackColor("primary".to_string()),
            semantic_name: "out_vulkanic_source_color_primary".to_string(),
        }];
        std::env::set_var(
            "MATTMC_RUST_SELECTED_SOURCE_FULLSCREEN_PROBE",
            "distant-horizons-depth-routing",
        );
        let result =
            apply_selected_source_fullscreen_probe(&mut lowered, &outputs, "world0/deferred1.fsh");
        std::env::remove_var("MATTMC_RUST_SELECTED_SOURCE_FULLSCREEN_PROBE");
        result.unwrap();
        assert!(lowered.contains(
            "float z0 = texelFetch(depthtex0, texelCoord, 0).r; float vulkanicDhDepthProbe = texelFetch(dhDepthTex, texelCoord, 0).r;"
        ));
        assert!(lowered.contains(
            "vec4(z0, vulkanicDhDepthProbe, (z0 >= 1.0 && vulkanicDhDepthProbe < 1.0) ? 1.0 : 0.0, 1.0);"
        ));
    }

    #[test]
    fn fullscreen_depth_coordinate_probe_exposes_current_and_mirrored_dh_texels() {
        let _guard = fullscreen_probe_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut lowered = "uniform sampler2D depthtex0; uniform sampler2D dhDepthTex; uniform float viewHeight; ivec2 texelCoord; vec3 color; void main() { float z0 = texelFetch(depthtex0, texelCoord, 0).r; out_vulkanic_source_color_primary = vec4(color, 1.0); }".to_string();
        let outputs = vec![FullscreenSourceFragmentOutput {
            source_location: 0,
            source_slot: 0,
            role: TerrainSourceResourceRole::ShaderPackColor("primary".to_string()),
            semantic_name: "out_vulkanic_source_color_primary".to_string(),
        }];
        std::env::set_var(
            "MATTMC_RUST_SELECTED_SOURCE_FULLSCREEN_PROBE",
            "distant-horizons-depth-coordinate",
        );
        let result =
            apply_selected_source_fullscreen_probe(&mut lowered, &outputs, "world0/deferred1.fsh");
        std::env::remove_var("MATTMC_RUST_SELECTED_SOURCE_FULLSCREEN_PROBE");
        result.unwrap();
        assert!(lowered.contains(
            "vec4(vulkanicDhDepthProbe, texelFetch(dhDepthTex, ivec2(texelCoord.x, int(viewHeight) - 1 - texelCoord.y), 0).r, texCoord.y, 1.0);"
        ));
        assert!(lowered.contains(
            "selected-source fullscreen diagnostic probe: distant-horizons-depth-coordinate"
        ));
    }

    #[test]
    fn fullscreen_fog_input_probe_reads_deferred_dh_values_inside_the_fog_branch() {
        let _guard = fullscreen_probe_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut lowered = "int dhRenderDistance; void main() { float z0 = texelFetch(depthtex0, texelCoord, 0).r; float z0DH = texelFetch(dhDepthTex, texelCoord, 0).r; DoFog(color.rgb, skyFade, lViewPos, playerPos, VdotU, VdotS, dither); if (z0DH < 1.0) { // Distant Horizons Chunks\nfloat lViewPos = 7.0; DoFog(color.rgb, skyFade, lViewPos, playerPos, VdotU, VdotS, dither); } out_vulkanic_source_color_primary = vec4(color, 1.0); }".to_string();
        let outputs = vec![FullscreenSourceFragmentOutput {
            source_location: 0,
            source_slot: 0,
            role: TerrainSourceResourceRole::ShaderPackColor("primary".to_string()),
            semantic_name: "out_vulkanic_source_color_primary".to_string(),
        }];
        std::env::set_var(
            "MATTMC_RUST_SELECTED_SOURCE_FULLSCREEN_PROBE",
            "distant-horizons-fog-inputs",
        );
        let result =
            apply_selected_source_fullscreen_probe(&mut lowered, &outputs, "world0/deferred1.fsh");
        std::env::remove_var("MATTMC_RUST_SELECTED_SOURCE_FULLSCREEN_PROBE");
        result.unwrap();
        assert!(lowered.contains(
            "float z0 = texelFetch(depthtex0, texelCoord, 0).r; float vulkanicDhDepthProbe = texelFetch(dhDepthTex, texelCoord, 0).r; vec3 vulkanicDhFogInputs = vec3(0.0);"
        ));
        assert!(lowered.contains(
            "vulkanicDhFogInputs = vec3(vulkanicDhDepthProbe, clamp(lViewPos / max(float(dhRenderDistance), 1.0), 0.0, 1.0), texCoord.y); DoFog("
        ));
        assert!(
            lowered.contains("out_vulkanic_source_color_primary = vec4(vulkanicDhFogInputs, 1.0);")
        );
        assert!(lowered.contains(
            "if (z0DH < 1.0) { // Distant Horizons Chunks\nfloat lViewPos = 7.0; vulkanicDhFogInputs ="
        ));
    }

    #[test]
    fn fullscreen_fog_effect_probe_keeps_the_fog_call_and_encodes_its_effect() {
        let _guard = fullscreen_probe_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut lowered = "void main() { float z0 = texelFetch(depthtex0, texelCoord, 0).r; float z0DH = texelFetch(dhDepthTex, texelCoord, 0).r; DoFog(color.rgb, skyFade, lViewPos, playerPos, VdotU, VdotS, dither); if (z0DH < 1.0) { // Distant Horizons Chunks\nfloat lViewPos = 7.0; DoFog(color.rgb, skyFade, lViewPos, playerPos, VdotU, VdotS, dither); } out_vulkanic_source_color_primary = vec4(color, 1.0); }".to_string();
        let outputs = vec![FullscreenSourceFragmentOutput {
            source_location: 0,
            source_slot: 0,
            role: TerrainSourceResourceRole::ShaderPackColor("primary".to_string()),
            semantic_name: "out_vulkanic_source_color_primary".to_string(),
        }];
        std::env::set_var(
            "MATTMC_RUST_SELECTED_SOURCE_FULLSCREEN_PROBE",
            "distant-horizons-fog-effect",
        );
        let result =
            apply_selected_source_fullscreen_probe(&mut lowered, &outputs, "world0/deferred1.fsh");
        std::env::remove_var("MATTMC_RUST_SELECTED_SOURCE_FULLSCREEN_PROBE");
        result.unwrap();
        assert!(lowered.contains(
            "vec3 vulkanicFogInputColor = color.rgb; DoFog(color.rgb, skyFade, lViewPos, playerPos, VdotU, VdotS, dither); vulkanicDhFogInputs = vec3("
        ));
        assert!(lowered.contains("log2(max(lViewPos, 1.0)) / 12.0"));
        assert!(
            lowered.contains("out_vulkanic_source_color_primary = vec4(vulkanicDhFogInputs, 1.0);")
        );
        assert!(lowered.contains(
            "if (z0DH < 1.0) { // Distant Horizons Chunks\nfloat lViewPos = 7.0; vec3 vulkanicFogInputColor ="
        ));
    }

    #[test]
    fn fullscreen_gbuffer_input_probe_exposes_normal_and_material_targets() {
        let _guard = fullscreen_probe_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut lowered =
            "void main() { out_vulkanic_source_color_primary = vec4(color, 1.0); }".to_string();
        let outputs = vec![FullscreenSourceFragmentOutput {
            source_location: 0,
            source_slot: 0,
            role: TerrainSourceResourceRole::ShaderPackColor("primary".to_string()),
            semantic_name: "out_vulkanic_source_color_primary".to_string(),
        }];
        std::env::set_var(
            "MATTMC_RUST_SELECTED_SOURCE_FULLSCREEN_PROBE",
            "gbuffer-inputs",
        );
        let result =
            apply_selected_source_fullscreen_probe(&mut lowered, &outputs, "world0/deferred1.fsh");
        std::env::remove_var("MATTMC_RUST_SELECTED_SOURCE_FULLSCREEN_PROBE");
        result.unwrap();
        assert!(lowered.contains(
            "texelFetch(colortex5, texelCoord, 0).rgb, texelFetch(colortex6, texelCoord, 0).r"
        ));
    }

    #[test]
    fn fullscreen_gbuffer_primary_probe_exposes_current_primary_target() {
        let _guard = fullscreen_probe_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut lowered =
            "void main() { out_vulkanic_source_color_primary = vec4(color, 1.0); }".to_string();
        let outputs = vec![FullscreenSourceFragmentOutput {
            source_location: 0,
            source_slot: 0,
            role: TerrainSourceResourceRole::ShaderPackColor("primary".to_string()),
            semantic_name: "out_vulkanic_source_color_primary".to_string(),
        }];
        std::env::set_var(
            "MATTMC_RUST_SELECTED_SOURCE_FULLSCREEN_PROBE",
            "gbuffer-primary",
        );
        let result =
            apply_selected_source_fullscreen_probe(&mut lowered, &outputs, "world0/deferred1.fsh");
        std::env::remove_var("MATTMC_RUST_SELECTED_SOURCE_FULLSCREEN_PROBE");
        result.unwrap();
        assert!(lowered.contains(
            "vec4(texelFetch(colortex0, texelCoord, 0).rgb, 1.0); // selected-source fullscreen diagnostic probe: gbuffer-primary"
        ));
    }

    #[test]
    fn fullscreen_depth_input_probe_exposes_deferred_depth() {
        let _guard = fullscreen_probe_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut lowered = "void main() { float z0 = texelFetch(depthtex0, texelCoord, 0).r; out_vulkanic_source_color_primary = vec4(color, 1.0); }".to_string();
        let outputs = vec![FullscreenSourceFragmentOutput {
            source_location: 0,
            source_slot: 0,
            role: TerrainSourceResourceRole::ShaderPackColor("primary".to_string()),
            semantic_name: "out_vulkanic_source_color_primary".to_string(),
        }];
        std::env::set_var(
            "MATTMC_RUST_SELECTED_SOURCE_FULLSCREEN_PROBE",
            "depth-input",
        );
        let result =
            apply_selected_source_fullscreen_probe(&mut lowered, &outputs, "world0/deferred1.fsh");
        std::env::remove_var("MATTMC_RUST_SELECTED_SOURCE_FULLSCREEN_PROBE");
        result.unwrap();
        assert!(lowered.contains(
            "vec4(vec3(z0), 1.0); // selected-source fullscreen diagnostic probe: depth-input"
        ));
    }

    #[test]
    fn fullscreen_depth_input_flipped_probe_uses_explicit_mirrored_texel() {
        let _guard = fullscreen_probe_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut lowered = "void main() { float z0 = texelFetch(depthtex0, texelCoord, 0).r; out_vulkanic_source_color_primary = vec4(color, 1.0); }".to_string();
        let outputs = vec![FullscreenSourceFragmentOutput {
            source_location: 0,
            source_slot: 0,
            role: TerrainSourceResourceRole::ShaderPackColor("primary".to_string()),
            semantic_name: "out_vulkanic_source_color_primary".to_string(),
        }];
        std::env::set_var(
            "MATTMC_RUST_SELECTED_SOURCE_FULLSCREEN_PROBE",
            "depth-input-flipped",
        );
        let result =
            apply_selected_source_fullscreen_probe(&mut lowered, &outputs, "world0/deferred1.fsh");
        std::env::remove_var("MATTMC_RUST_SELECTED_SOURCE_FULLSCREEN_PROBE");
        result.unwrap();
        assert!(lowered.contains("ivec2(texelCoord.x, int(viewHeight) - 1 - texelCoord.y)"));
    }

    #[test]
    fn fullscreen_deferred_fog_probe_exposes_depth_distance_and_sky_fade() {
        let _guard = fullscreen_probe_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut lowered = "void main() { float z0 = texelFetch(depthtex0, texelCoord, 0).r; vec4 viewPos = vec4(0.0); float lViewPos = length(viewPos); DoFog(color.rgb, skyFade, lViewPos, playerPos, VdotU, VdotS, dither); out_vulkanic_source_color_primary = vec4(color, 1.0); }".to_string();
        let outputs = vec![FullscreenSourceFragmentOutput {
            source_location: 0,
            source_slot: 0,
            role: TerrainSourceResourceRole::ShaderPackColor("primary".to_string()),
            semantic_name: "out_vulkanic_source_color_primary".to_string(),
        }];
        std::env::set_var(
            "MATTMC_RUST_SELECTED_SOURCE_FULLSCREEN_PROBE",
            "deferred-fog-inputs",
        );
        let result =
            apply_selected_source_fullscreen_probe(&mut lowered, &outputs, "world0/deferred1.fsh");
        std::env::remove_var("MATTMC_RUST_SELECTED_SOURCE_FULLSCREEN_PROBE");
        result.unwrap();
        assert!(lowered.contains("vec3 vulkanicDeferredFogInputs"));
        assert!(
            lowered.contains("selected-source fullscreen diagnostic probe: deferred-fog-inputs")
        );
    }

    #[test]
    fn fullscreen_depth_probe_leaves_unrelated_source_stages_unchanged() {
        let _guard = fullscreen_probe_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut lowered = "out_vulkanic_source_color_primary = vec4(color, 1.0);".to_string();
        let outputs = vec![FullscreenSourceFragmentOutput {
            source_location: 0,
            source_slot: 0,
            role: TerrainSourceResourceRole::ShaderPackColor("primary".to_string()),
            semantic_name: "out_vulkanic_source_color_primary".to_string(),
        }];
        std::env::set_var(
            "MATTMC_RUST_SELECTED_SOURCE_FULLSCREEN_PROBE",
            "distant-horizons-depth",
        );
        let result =
            apply_selected_source_fullscreen_probe(&mut lowered, &outputs, "world0/composite4.fsh");
        std::env::remove_var("MATTMC_RUST_SELECTED_SOURCE_FULLSCREEN_PROBE");
        result.unwrap();
        assert_eq!(
            "out_vulkanic_source_color_primary = vec4(color, 1.0);",
            lowered
        );
    }

    #[test]
    fn fullscreen_source_pair_uses_named_pack_color_outputs_and_owned_uv_stream() {
        let pack = ShaderPackSource::new(
            "fullscreen-source",
            5,
            vec![
                ShaderSourceFile::new(
                    "world0/deferred.vsh",
                    "#version 130\nout vec2 uv;\nout vec2 light_uv;\nvoid main() { uv = (gl_TextureMatrix[0] * gl_MultiTexCoord0).xy; light_uv = (gl_TextureMatrix[1] * gl_MultiTexCoord1).xy; gl_Position = ftransform(); }",
                ),
                ShaderSourceFile::new(
                    "world0/deferred.fsh",
                    "#version 130\n/* DRAWBUFFERS:0 */\nin vec2 uv;\nuniform sampler2D tex;\nvoid main() { gl_FragData[0] = texture2D(tex, uv); }",
                ),
                ShaderSourceFile::new(
                    super::super::terrain_source_resources::TERRAIN_RESOURCE_BINDINGS_PATH,
                    "tex=material_atlas\ncolortex0=shader_pack_color:primary\n",
                ),
            ],
        )
        .unwrap();
        let vertex = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "world0/deferred.vsh",
            defines: &[],
        })
        .unwrap();
        let fragment = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "world0/deferred.fsh",
            defines: &[],
        })
        .unwrap();
        let bindings = TerrainSourceResourceBindings::from_source(&pack).unwrap();
        let lowered = lower_fullscreen_source_pair(&vertex, &fragment, &bindings).unwrap();
        assert_eq!(
            TerrainSourceResourceRole::ShaderPackColor("primary".to_string()),
            lowered.fragment().outputs()[0].role()
        );
        assert!(lowered
            .vertex()
            .source()
            .contains("VulkanicSourceFullscreenFrame"));
        assert!(lowered
            .vertex()
            .source()
            .contains("vulkanic_source_fullscreen_vertex_index"));
        assert!(lowered
            .vertex()
            .source()
            .contains("vulkanic_source_fullscreen_uv_coordinates"));
        assert!(!lowered
            .vertex()
            .source()
            .contains("layout(location = 0) in vec2 vulkanic_source_fullscreen_position"));
        assert!(!lowered.vertex().source().contains("gl_MultiTexCoord1"));
        assert!(!lowered.vertex().source().contains("gbufferModelView"));
        assert!(lowered
            .fragment()
            .source()
            .contains("out_vulkanic_source_color_primary"));
        assert!(!lowered.fragment().source().contains("gl_FragData"));
        assert!(lowered
            .vertex()
            .source()
            .contains("vec4(vulkanic_source_fullscreen_position(), 0.0, 1.0)"));
        assert!(lowered
            .vertex()
            .source()
            .contains("coordinate.y = 1.0 - coordinate.y"));
        assert!(lowered
            .vertex()
            .source()
            .contains("#ifdef VULKANIC_GAL_ZERO_TO_ONE_CLIP_DEPTH"));
    }

    #[test]
    fn fullscreen_fragment_coordinates_keep_source_math_and_native_target_addresses() {
        let pack = ShaderPackSource::new(
            "fullscreen-fragment-coordinate-domains",
            5,
            vec![
                ShaderSourceFile::new(
                    "world0/deferred.vsh",
                    "#version 130\nout vec2 texCoord;\nvoid main() { texCoord = gl_MultiTexCoord0.xy; gl_Position = ftransform(); }",
                ),
                ShaderSourceFile::new(
                    "world0/deferred.fsh",
                    "#version 130\n/* DRAWBUFFERS:0 */\nin vec2 texCoord;\nuniform float viewHeight;\nuniform sampler2D colortex0;\nuniform sampler2D depthtex0;\nvoid main() { ivec2 texelCoord = ivec2(gl_FragCoord.xy); vec2 sourceScreen = gl_FragCoord.xy / vec2(1.0, viewHeight); vec4 screenPos = vec4(texCoord, 0.5, 1.0); vec4 screenPosDH = vec4(texCoord, 0.75, 1.0); vec4 reconstructed = vec4(texCoord, texelFetch(colortex0, texelCoord, 0).r + texelFetch(depthtex0, texelCoord, 0).r, 1.0); gl_FragData[0] = texelFetch(colortex0, texelCoord, 0) + vec4(sourceScreen, 0.0, 0.0) + reconstructed + screenPos + screenPosDH; }",
                ),
                ShaderSourceFile::new(
                    super::super::terrain_source_resources::TERRAIN_RESOURCE_BINDINGS_PATH,
                    "colortex0=shader_pack_color:primary\n",
                ),
            ],
        )
        .unwrap();
        let vertex = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "world0/deferred.vsh",
            defines: &[],
        })
        .unwrap();
        let fragment = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "world0/deferred.fsh",
            defines: &[],
        })
        .unwrap();
        let bindings = TerrainSourceResourceBindings::from_source(&pack).unwrap();
        let lowered = lower_fullscreen_source_pair(&vertex, &fragment, &bindings).unwrap();
        let fragment = lowered.fragment().source();

        assert!(fragment.contains("vec4 vulkanic_source_fullscreen_fragment_coord()"));
        assert!(fragment.contains("coordinate.y = viewHeight - coordinate.y;"));
        assert!(fragment.contains("vec2 vulkanic_source_fullscreen_screen_uv(vec2 image_uv)"));
        assert!(fragment.contains("ivec2 texelCoord = ivec2(gl_FragCoord.xy);"));
        assert!(fragment.contains(
            "texelFetch(depthtex0, ivec2(texelCoord.x, int(viewHeight) - 1 - texelCoord.y), 0)"
        ));
        assert!(fragment.contains("texelFetch(colortex0, texelCoord, 0)"));
        assert!(fragment.contains(
            "vec2 sourceScreen = vulkanic_source_fullscreen_fragment_coord().xy / vec2(1.0, viewHeight);"
        ));
        assert!(fragment.contains("texelFetch(colortex0, texelCoord, 0)"));
        assert!(fragment.contains(
            "vec4 screenPos = vec4(vulkanic_source_fullscreen_screen_uv(texCoord), 0.5, 1.0);"
        ));
        assert!(fragment.contains(
            "vec4 screenPosDH = vec4(vulkanic_source_fullscreen_screen_uv(texCoord), 0.75, 1.0);"
        ));
        assert!(
            fragment.find("uniform float viewHeight;")
                < fragment.find("vec4 vulkanic_source_fullscreen_fragment_coord()"),
            "the source viewport uniform must be declared before the coordinate helper"
        );
    }

    #[test]
    fn fullscreen_source_composite7_fxaa_probe_only_suppresses_the_targeted_call() {
        let _guard = fullscreen_probe_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let prior = std::env::var_os("MATTMC_RUST_SELECTED_SOURCE_FULLSCREEN_PROBE");
        std::env::set_var(
            "MATTMC_RUST_SELECTED_SOURCE_FULLSCREEN_PROBE",
            "composite7-without-fxaa",
        );

        let mut composite7 = "void main() { vec3 color = vec3(1.0); FXAA311(color); }".to_string();
        apply_selected_source_fullscreen_probe(&mut composite7, &[], "world0/composite7.fsh")
            .unwrap();
        assert!(!composite7.contains("FXAA311(color);"));
        assert!(composite7.contains("FXAA call suppressed"));

        let mut unrelated = "void main() { vec3 color = vec3(1.0); FXAA311(color); }".to_string();
        apply_selected_source_fullscreen_probe(&mut unrelated, &[], "world0/composite6.fsh")
            .unwrap();
        assert!(unrelated.contains("FXAA311(color);"));

        match prior {
            Some(value) => std::env::set_var("MATTMC_RUST_SELECTED_SOURCE_FULLSCREEN_PROBE", value),
            None => std::env::remove_var("MATTMC_RUST_SELECTED_SOURCE_FULLSCREEN_PROBE"),
        }
    }

    #[test]
    fn fullscreen_source_far_world_raster_semantics_preserve_source_sky_depth() {
        let pack = ShaderPackSource::new(
            "fullscreen-sky-source",
            5,
            vec![
                ShaderSourceFile::new(
                    "world0/gbuffers_skybasic.vsh",
                    "#version 130\nvoid main() { gl_Position = ftransform(); }",
                ),
                ShaderSourceFile::new(
                    "world0/gbuffers_skybasic.fsh",
                    "#version 130\n/* DRAWBUFFERS:0 */\nvoid main() { gl_FragData[0] = vec4(1.0); }",
                ),
                ShaderSourceFile::new(
                    super::super::terrain_source_resources::TERRAIN_RESOURCE_BINDINGS_PATH,
                    "colortex0=shader_pack_color:primary\n",
                ),
            ],
        )
        .unwrap();
        let vertex = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "world0/gbuffers_skybasic.vsh",
            defines: &[],
        })
        .unwrap();
        let fragment = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "world0/gbuffers_skybasic.fsh",
            defines: &[],
        })
        .unwrap();
        let bindings = TerrainSourceResourceBindings::from_source(&pack).unwrap();
        let lowered = lower_fullscreen_source_pair_with_raster_primitive(
            &vertex,
            &fragment,
            &bindings,
            FullscreenSourceRasterPrimitive::VanillaSkyDisc,
        )
        .unwrap();
        assert_eq!(
            lowered.raster_primitive(),
            FullscreenSourceRasterPrimitive::VanillaSkyDisc
        );
        assert!(lowered
            .uniform_contract()
            .declarations()
            .iter()
            .any(|declaration| declaration == "mat4 gbufferModelView;"));
        assert!(lowered
            .uniform_contract()
            .declarations()
            .iter()
            .any(|declaration| declaration == "mat4 gbufferProjection;"));
        let names = lowered
            .uniform_contract()
            .declarations()
            .iter()
            .map(|declaration| uniform_name(declaration).unwrap())
            .collect::<Vec<_>>();
        assert!(names.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(lowered
            .vertex()
            .source()
            .contains("Expanded eight-wedge form of Minecraft's top SkyRenderer disc"));
        assert!(lowered.vertex().source().contains(
            "gbufferProjection * gbufferModelView * vulkanic_source_fullscreen_sky_position()"
        ));
        assert!(lowered
            .vertex()
            .source()
            .contains("#define vulkanic_source_fullscreen_vertex_color vec4(skyColor, 1.0)"));
        assert!(!lowered
            .vertex()
            .source()
            .contains("const vec4 vulkanic_source_fullscreen_vertex_color = vec4(1.0);"));
        assert!(lowered.vertex().source().contains(
            "#ifdef VULKANIC_GAL_ZERO_TO_ONE_CLIP_DEPTH\n    gl_Position.z = (gl_Position.z + gl_Position.w) * 0.5;\n#endif"
        ));
    }

    #[test]
    fn fullscreen_source_lowers_legacy_fog_to_named_semantic_inputs() {
        let pack = ShaderPackSource::new(
            "fullscreen-legacy-fog",
            6,
            vec![
                ShaderSourceFile::new(
                    "world0/deferred.vsh",
                    "#version 130\nout vec2 uv;\nvoid main() { uv = gl_MultiTexCoord0.xy; gl_Position = ftransform(); }",
                ),
                ShaderSourceFile::new(
                    "world0/deferred.fsh",
                    "#version 130\n/* DRAWBUFFERS:0 */\nin vec2 uv;\nvoid main() { float fog = gl_Fog.start * gl_Fog.scale + gl_Fog.color.a; gl_FragData[0] = vec4(uv, fog, 1.0); }",
                ),
                ShaderSourceFile::new(
                    super::super::terrain_source_resources::TERRAIN_RESOURCE_BINDINGS_PATH,
                    "colortex0=shader_pack_color:primary\n",
                ),
            ],
        )
        .unwrap();
        let vertex = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "world0/deferred.vsh",
            defines: &[],
        })
        .unwrap();
        let fragment = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "world0/deferred.fsh",
            defines: &[],
        })
        .unwrap();
        let bindings = TerrainSourceResourceBindings::from_source(&pack).unwrap();
        let lowered = lower_fullscreen_source_pair(&vertex, &fragment, &bindings).unwrap();

        assert!(lowered.require_backend_neutral_lowering().is_ok());
        assert!(!lowered.fragment().source().contains("gl_Fog"));
        assert!(lowered
            .fragment()
            .source()
            .contains("VulkanicSourceLegacyFogParameters"));
        assert!(
            lowered
                .fragment()
                .source()
                .find("vulkanic_source_fog_parameter_color;")
                < lowered
                    .fragment()
                    .source()
                    .find("VulkanicSourceLegacyFogParameters")
        );
        assert!(lowered
            .fragment()
            .source()
            .contains("vulkanic_source_fog().start"));
        assert_eq!(
            vec![
                "vulkanic_source_fog_environmental_end",
                "vulkanic_source_fog_environmental_start",
                "vulkanic_source_fog_parameter_color",
            ],
            lowered
                .uniform_contract()
                .fields()
                .iter()
                .map(|field| field.name())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn fullscreen_source_pair_rejects_legacy_output_without_semantic_color_role() {
        let pack = ShaderPackSource::new(
            "fullscreen-source-unmapped-output",
            5,
            vec![
                ShaderSourceFile::new(
                    "world0/deferred.vsh",
                    "#version 130\nout vec2 uv;\nvoid main() { uv = gl_MultiTexCoord0.xy; gl_Position = ftransform(); }",
                ),
                ShaderSourceFile::new(
                    "world0/deferred.fsh",
                    "#version 130\n/* DRAWBUFFERS:01 */\nin vec2 uv;\nvoid main() { gl_FragData[0] = vec4(uv, 0.0, 1.0); gl_FragData[1] = vec4(uv, 0.0, 1.0); }",
                ),
                ShaderSourceFile::new(
                    super::super::terrain_source_resources::TERRAIN_RESOURCE_BINDINGS_PATH,
                    "colortex0=shader_pack_color:primary\n",
                ),
            ],
        )
        .unwrap();
        let vertex = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "world0/deferred.vsh",
            defines: &[],
        })
        .unwrap();
        let fragment = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "world0/deferred.fsh",
            defines: &[],
        })
        .unwrap();
        let bindings = TerrainSourceResourceBindings::from_source(&pack).unwrap();
        assert!(lower_fullscreen_source_pair(&vertex, &fragment, &bindings)
            .unwrap_err()
            .to_string()
            .contains("has no declared semantic color role"));
    }

    #[test]
    fn fullscreen_source_uses_drawbuffers_slot_not_glsl_output_location() {
        let pack = ShaderPackSource::new(
            "fullscreen-source-drawbuffers-remap",
            5,
            vec![
                ShaderSourceFile::new(
                    "world0/deferred.vsh",
                    "#version 130\nout vec2 uv;\nvoid main() { uv = gl_MultiTexCoord0.xy; gl_Position = ftransform(); }",
                ),
                ShaderSourceFile::new(
                    "world0/deferred.fsh",
                    "#version 130\n/* DRAWBUFFERS:3 */\nin vec2 uv;\nvoid main() { gl_FragData[0] = vec4(uv, 0.0, 1.0); }",
                ),
                ShaderSourceFile::new(
                    super::super::terrain_source_resources::TERRAIN_RESOURCE_BINDINGS_PATH,
                    "colortex3=shader_pack_color:translucent_final\n",
                ),
            ],
        )
        .unwrap();
        let vertex = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "world0/deferred.vsh",
            defines: &[],
        })
        .unwrap();
        let fragment = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "world0/deferred.fsh",
            defines: &[],
        })
        .unwrap();
        let bindings = TerrainSourceResourceBindings::from_source(&pack).unwrap();
        let lowered = lower_fullscreen_source_pair(&vertex, &fragment, &bindings).unwrap();
        let output = &lowered.fragment().outputs()[0];
        assert_eq!(0, output.source_location());
        assert_eq!(3, output.source_slot());
        assert_eq!(
            TerrainSourceResourceRole::ShaderPackColor("translucent_final".to_string()),
            output.role()
        );
    }

    #[test]
    fn shadow_lowering_externalizes_owned_uimage3d_writes() {
        let pack = ShaderPackSource::new(
            "test",
            1,
            vec![
                ShaderSourceFile::new(
                    "shadow.vsh",
                    "#version 130\nwriteonly uniform uimage3D voxel_img;\nuniform usampler3D voxel_sampler;\nvoid main() { imageStore(voxel_img, ivec3(0), uvec4(1)); gl_Position = gl_Vertex; }",
                ),
                ShaderSourceFile::new(
                    "shadow.fsh",
                    "#version 130\nvoid main() { gl_FragData[0] = vec4(1.0); }",
                ),
            ],
        )
        .unwrap();
        let vertex = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "shadow.vsh",
            defines: &[],
        })
        .unwrap();
        let fragment = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "shadow.fsh",
            defines: &[],
        })
        .unwrap();
        let lowered = lower_shadow_source_pair(&vertex, &fragment).unwrap();
        assert!(!lowered.vertex().source().contains("voxel_img"));
        assert!(!lowered.vertex().source().contains("imageStore"));
        assert!(lowered
            .opaque_resource_contract()
            .active_resources()
            .all(|resource| resource.kind() != TerrainSourceOpaqueResourceKind::StorageImage));
    }

    #[test]
    fn terrain_lowering_externalizes_owned_uimage3d_writes() {
        let pack = ShaderPackSource::new(
            "test",
            1,
            vec![
                ShaderSourceFile::new(
                    "world0/gbuffers_terrain.vsh",
                    "#version 130\nattribute vec4 mc_Entity;\nwriteonly uniform uimage3D voxel_img;\nvoid UpdateVoxelMap() { imageStore(voxel_img, ivec3(0), uvec4(1)); }\nvoid main() { UpdateVoxelMap(); gl_Position = gl_Vertex; }",
                ),
                ShaderSourceFile::new(
                    "world0/gbuffers_terrain.fsh",
                    "#version 130\nvoid main() { gl_FragData[0] = vec4(1.0); gl_FragData[1] = vec4(0.0); }",
                ),
            ],
        )
        .unwrap();
        let vertex = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "world0/gbuffers_terrain.vsh",
            defines: &[],
        })
        .unwrap();
        let fragment = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "world0/gbuffers_terrain.fsh",
            defines: &[],
        })
        .unwrap();

        let lowered = lower_terrain_source_pair(&vertex, &fragment).unwrap();
        assert!(!lowered.vertex().source().contains("voxel_img"));
        assert!(!lowered.vertex().source().contains("imageStore"));
    }

    #[test]
    fn shadow_lowering_refuses_to_drop_puddle_storage_without_an_owned_writer() {
        let source = artifact(
            "#version 130\nwriteonly uniform uimage2D puddle_img;\nvoid main() { imageStore(puddle_img, ivec2(0), uvec4(10)); gl_Position = gl_Vertex; }",
        );
        let error = externalize_owned_semantic_storage_writes(
            &source,
            &TerrainSourceResourceBindings::default(),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("puddle_img"));
        assert!(error.contains("explicit Rust semantic writer"));
    }

    #[test]
    fn shadow_lowering_externalizes_declared_puddle_storage_and_keeps_its_role() {
        let pack = ShaderPackSource::new(
            "test",
            1,
            vec![
                ShaderSourceFile::new(
                    "shadow.vsh",
                    "#version 130\nwriteonly uniform uimage2D puddle_img;\nvoid main() { imageStore(puddle_img, ivec2(0), uvec4(10)); gl_Position = gl_Vertex; }",
                ),
                ShaderSourceFile::new(
                    "shadow.fsh",
                    "#version 130\nvoid main() { gl_FragData[0] = vec4(1.0); }",
                ),
                ShaderSourceFile::new(
                    super::super::terrain_source_resources::TERRAIN_RESOURCE_BINDINGS_PATH,
                    "puddle_img=puddle_occupancy\n",
                ),
            ],
        )
        .unwrap();
        let vertex = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "shadow.vsh",
            defines: &[],
        })
        .unwrap();
        let fragment = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "shadow.fsh",
            defines: &[],
        })
        .unwrap();
        let bindings = TerrainSourceResourceBindings::from_source(&pack).unwrap();

        let lowered =
            lower_shadow_source_pair_with_owned_storage(&vertex, &fragment, &bindings).unwrap();
        assert!(!lowered.vertex().source().contains("puddle_img"));
        assert!(!lowered.vertex().source().contains("imageStore"));
        assert_eq!(
            &[TerrainSourceResourceRole::PuddleOccupancy],
            lowered.owned_storage_roles()
        );
    }

    #[test]
    fn shadow_lowering_rejects_unmapped_storage_write_targets() {
        let source = artifact(
            "#version 130\nwriteonly uniform uimage3D voxel_img;\nvoid main() { imageStore(other_image, ivec3(0), uvec4(1)); gl_Position = gl_Vertex; }",
        );
        let error = externalize_owned_semantic_storage_writes(
            &source,
            &TerrainSourceResourceBindings::default(),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("has no Rust-owned semantic writer"));
    }

    #[test]
    fn rejects_outputs_outside_the_named_terrain_contract() {
        let error = lower_terrain_fragment_surface(&artifact(
            "#version 130\nvoid main() { gl_FragData[4] = vec4(1.0); }",
        ))
        .unwrap_err();
        assert!(error.to_string().contains("unsupported gl_FragData index"));
    }

    #[test]
    fn lowers_known_legacy_vertex_semantics_without_assigning_java_locations() {
        let lowered = lower_terrain_vertex_surface(&artifact(
            "#version 130\nattribute vec4 mc_Entity;\nattribute vec4 mc_midTexCoord;\nattribute vec4 at_tangent;\nattribute vec3 at_midBlock;\nvarying vec2 uv;\nvoid main() { uv = (gl_TextureMatrix[0] * gl_MultiTexCoord0).xy; vec3 n = gl_NormalMatrix * gl_Normal + at_midBlock / 64.0; float id = mc_Entity.x + mc_midTexCoord.x + at_tangent.w; gl_Position = ftransform() + gl_ModelViewMatrix * gl_Vertex + gl_ProjectionMatrix * gl_Color + vec4(n, id); }",
        ))
        .unwrap();
        assert!(lowered.source().contains("VulkanicSourceTerrainVertex"));
        assert!(lowered.source().contains("VulkanicSourceTerrainInstances"));
        assert!(lowered.source().contains("vec4 color_modulation"));
        assert!(lowered.source().contains(
            "(vulkanic_source_vertex.color * vulkanic_source_instance.color_modulation)"
        ));
        assert!(lowered.source().contains("vulkanic_source_mid_tex_coord"));
        assert!(lowered.source().contains("vulkanic_source_mid_block"));
        assert!(lowered.source().contains("vulkanic_source_model_view"));
        assert!(lowered.source().contains("out vec2 uv"));
        assert!(!lowered.source().contains("attribute vec4"));
        assert!(!lowered.source().contains("gl_MultiTexCoord0"));
        assert!(!lowered.source().contains("gl_ModelViewMatrix"));
        assert!(lowered.remaining_dialect().gaps().is_empty());
    }

    #[test]
    fn terrain_lowering_replaces_legacy_camera_relative_world_reconstruction() {
        let lowered = lower_terrain_vertex_surface(&artifact(
            "#version 130\nvoid main() { vec4 position = gbufferModelViewInverse * gl_ModelViewMatrix * gl_Vertex; gl_Position = gl_ProjectionMatrix * gbufferModelView * position; }",
        ))
        .unwrap();
        assert!(!lowered.source().contains(
            "gbufferModelViewInverse * vulkanic_source_model_view * vulkanic_source_position"
        ));
        assert!(lowered
            .source()
            .contains("vulkanic_source_model_transform * vulkanic_source_position"));
    }

    #[test]
    fn textured_material_lowering_expands_quad_triangles_from_owned_vertex_indices() {
        let vertex = artifact(
            "#version 130\nvarying vec2 uv; void main() { uv = gl_MultiTexCoord0.xy; gl_Position = ftransform(); }",
        );
        let fragment = artifact(
            "#version 130\nvarying vec2 uv; void main() { gl_FragData[0] = vec4(uv, 0.0, 1.0); gl_FragData[1] = vec4(1.0); gl_FragData[2] = vec4(0.0); }",
        );
        let lowered = lower_textured_material_source_pair(&vertex, &fragment).unwrap();
        let source = lowered.vertex().source();
        assert!(source.contains("vulkanic_source_textured_quad_indices[6]"));
        assert!(source.contains("(gl_VertexIndex / 6) * 4"));
        assert!(source.contains("vulkanic_source_textured_quad_indices[gl_VertexIndex % 6]"));
        assert!(
            !source.contains("VulkanicSourceTerrainInstances"),
            "textured material must not inherit terrain instance storage"
        );
    }

    #[test]
    fn entity_lowering_installs_an_explicit_cutout_hook_after_color_modulation() {
        let vertex = artifact(
            "#version 130\nvarying vec2 texCoord; varying vec4 glColor; void main() { texCoord = gl_MultiTexCoord0.xy; glColor = gl_Color; gl_Position = ftransform(); }",
        );
        let fragment = artifact(
            "#version 130\nvarying vec2 texCoord; varying vec4 glColor; uniform sampler2D tex; void main() { vec4 color = texture2D(tex, texCoord); color *= glColor; gl_FragData[0] = color; gl_FragData[1] = color; }",
        );
        let lowered = lower_entity_source_pair(&vertex, &fragment).unwrap();
        let source = lowered.fragment().source();
        let modulation = source.find("color *= glColor;").unwrap();
        let discard = source
            .find("if (color.a <= VULKANIC_SOURCE_ENTITY_ALPHA_CUTOFF) discard;")
            .unwrap();
        assert!(modulation < discard);
        assert!(source.contains("#define VULKANIC_SOURCE_ENTITY_ALPHA_CUTOFF -1.0"));
    }

    #[test]
    fn hand_lowering_composes_pose_before_camera_model_view() {
        let vertex = artifact(
            "#version 130\nvarying vec2 texCoord; varying vec4 glColor; void main() { texCoord = gl_MultiTexCoord0.xy; glColor = gl_Color; gl_Position = ftransform(); }",
        );
        let fragment = artifact(
            "#version 130\nvarying vec2 texCoord; varying vec4 glColor; uniform sampler2D tex; void main() { vec4 color = texture2D(tex, texCoord); color *= glColor; gl_FragData[0] = color; }",
        );
        let lowered = lower_hand_source_pair(&vertex, &fragment).unwrap();
        assert!(lowered
            .vertex()
            .source()
            .contains("#define vulkanic_source_model_view (vulkanic_source_model_transform * gbufferModelView)"));
    }

    #[test]
    fn hand_lowering_rejects_unresolved_legacy_attributes_before_route_selection() {
        let vertex = artifact(
            "#version 130\nvarying vec2 texCoord; varying vec4 glColor; void main() { texCoord = gl_MultiTexCoord0.xy; glColor = gl_Color; gl_Position = ftransform(); }",
        );
        let fragment = artifact(
            "#version 130\nvarying vec2 texCoord; varying vec4 glColor; uniform sampler2D tex; void main() { vec4 color = texture2D(tex, texCoord); color *= glColor; gl_FragData[0] = color; gl_FragData[1] = color; }",
        );
        let error = lower_hand_source_pair(&vertex, &fragment)
            .unwrap()
            .require_backend_neutral_lowering()
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("compatibility_vertex_attributes"));
    }

    #[test]
    fn lowers_shadow_color_outputs_without_relabeling_them_as_terrain_gbuffer_data() {
        let lowered = lower_shadow_fragment_surface(&artifact(
            "#version 130\nuniform sampler2D tex;\nvoid main() { vec4 color = texture2D(tex, vec2(0.25)); gl_FragData[0] = color; gl_FragData[1] = vec4(color.rgb * 0.25, 1.0); }",
        ))
        .unwrap();
        assert_eq!(
            vec![
                ShadowFragmentOutput::ShadowColor,
                ShadowFragmentOutput::LightShaftColor,
            ],
            lowered.outputs()
        );
        assert!(lowered.source().contains("out_shadow_color"));
        assert!(lowered.source().contains("out_shadow_light_shaft_color"));
        assert!(!lowered.source().contains("out_terrain_lit_color"));
        assert!(!lowered.source().contains("gl_FragData"));
    }

    #[test]
    fn lowered_source_vertex_finalizes_clip_depth_after_nested_main_logic() {
        let lowered = lower_terrain_vertex_surface(&artifact(
            "#version 130\nvoid helper() { if (true) { int ignored = 0; } }\nvoid main() { if (true) { gl_Position = gl_Vertex; } /* closing braces } are trivia */ }",
        ))
        .unwrap();
        let source = lowered.source();
        let finalizer = "#ifdef VULKANIC_GAL_ZERO_TO_ONE_CLIP_DEPTH\n    gl_Position.z = (gl_Position.z + gl_Position.w) * 0.5;\n#endif";
        let finalizer_offset = source
            .find(finalizer)
            .expect("clip finalizer must be injected");
        let main_body_end = source.rfind('}').expect("lowered main has a closing brace");
        assert!(finalizer_offset < main_body_end);
        assert!(source[..finalizer_offset].contains("gl_Position = vulkanic_source_position"));
    }

    #[test]
    fn clip_depth_finalizer_rejects_a_source_without_main() {
        let error = append_vulkan_clip_depth_finalizer("#version 450\nvoid helper() {}")
            .unwrap_err()
            .to_string();
        assert!(error.contains("void main() body"));
    }

    #[test]
    fn lowers_distant_horizons_with_its_own_transform_and_color_contract() {
        let pack = ShaderPackSource::new(
            "dh-test",
            1,
            vec![
                ShaderSourceFile::new(
                    "dh_terrain.vsh",
                    "#version 130\nflat out int mat;\nout vec2 lmCoord;\nout vec4 glColor;\nuniform mat4 dhProjection;\nvoid main() { mat = 2; lmCoord = vec2(0.5); glColor = gl_Color; gl_Position = ftransform() + gl_ModelViewMatrix * vec4(0.0); }",
                ),
                ShaderSourceFile::new(
                    "dh_terrain.fsh",
                    "#version 130\nflat in int mat;\nin vec2 lmCoord;\nin vec4 glColor;\nuniform mat4 dhProjectionInverse;\nvoid main() { gl_FragData[0] = glColor * vec4(float(mat)) * (dhProjectionInverse[0][0] + lmCoord.x); }",
                ),
            ],
        )
        .unwrap();
        let vertex = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "dh_terrain.vsh",
            defines: &[],
        })
        .unwrap();
        let fragment = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "dh_terrain.fsh",
            defines: &[],
        })
        .unwrap();

        let lowered = lower_distant_horizons_source_pair(&vertex, &fragment).unwrap();
        assert_eq!(
            vec![DistantHorizonsFragmentOutput::LitColor],
            lowered.fragment().outputs()
        );
        assert!(lowered
            .fragment()
            .source()
            .contains("out_distant_horizons_lit_color"));
        assert!(!lowered
            .fragment()
            .source()
            .contains("out_terrain_lit_color"));
        assert!(lowered.vertex().source().contains("dhModelView"));
        assert!(lowered.vertex().source().contains("dhProjection"));
        assert!(
            !lowered.vertex().source().contains("gbufferProjection"),
            "DH source must not inherit the near-terrain projection"
        );
        assert!(lowered
            .vertex()
            .source()
            .contains("VulkanicDistantHorizonsVertices"));
        assert!(lowered
            .vertex()
            .source()
            .contains("vulkanic_source_dh_material_id"));
        assert!(lowered
            .vertex()
            .source()
            .contains("vulkanic_source_dh_packed_lightmap_coordinates"));
        let light_coordinates = lowered
            .vertex()
            .source()
            .find("vec2 vulkanic_source_dh_packed_lightmap_coordinates")
            .expect("the lowered DH source must declare packed light coordinates");
        let light_source = &lowered.vertex().source()[light_coordinates..];
        let block_shift = light_source
            .find("light_material_normal >> 8u")
            .expect("DH block light must occupy the first lightmap component");
        let sky_component = light_source
            .find("light_material_normal & 0xffu")
            .expect("DH sky light must occupy the second lightmap component");
        assert!(
            block_shift < sky_component,
            "the generic DH source adapter must preserve Iris's (blockLight, skyLight) order"
        );
        assert!(
            lowered.vertex().source().contains(
                "vec3(vulkanic_source_dh_vertex.micro_x, 0.0, vulkanic_source_dh_vertex.micro_z)"
            ),
            "DH terrain must match Iris's X/Z-only compact micro-offset reconstruction"
        );
        assert!(
            !lowered.vertex().source().contains(
                "vec3(vulkanic_source_dh_vertex.micro_x, vulkanic_source_dh_vertex.micro_y, vulkanic_source_dh_vertex.micro_z)"
            ),
            "DH terrain must not turn compact Y offset bits into terrain height"
        );
        assert!(lowered
            .uniform_contract()
            .fields()
            .iter()
            .any(|field| field.name() == "dhProjectionInverse"));
        assert!(!lowered.vertex().source().contains("gl_ModelViewMatrix"));
        assert!(!lowered.fragment().source().contains("gl_FragData"));
    }

    #[test]
    fn distant_horizons_fragment_probe_replaces_only_the_final_named_output() {
        let _guard = fullscreen_probe_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let source =
            "void main() { vec4 color = vec4(0.5); out_distant_horizons_lit_color = color; }";
        let mut fragment = LoweredDistantHorizonsFragmentSource {
            entry_path: "dh_terrain.fsh".to_string(),
            source: source.to_string(),
            outputs: vec![DistantHorizonsFragmentOutput::LitColor],
            remaining_dialect: analyze_glsl_text("dh_terrain.fsh", source),
        };
        std::env::set_var(
            "MATTMC_RUST_SELECTED_SOURCE_DH_FRAGMENT_PROBE",
            "constant-red",
        );
        let result = apply_selected_source_distant_horizons_fragment_probe(&mut fragment);
        std::env::remove_var("MATTMC_RUST_SELECTED_SOURCE_DH_FRAGMENT_PROBE");

        result.unwrap();
        assert!(fragment
            .source()
            .contains("out_distant_horizons_lit_color = vec4(1.0, 0.0, 0.0, 1.0)"));
        assert!(fragment.source().contains("vec4 color = vec4(0.5)"));
    }

    #[test]
    fn lowered_distant_horizons_maps_fragment_y_without_affecting_fullscreen_stages() {
        let pack = ShaderPackSource::new(
            "dh-fragment-coordinate-test",
            1,
            vec![
                ShaderSourceFile::new(
                    "dh_terrain.vsh",
                    "#version 130\nflat out int mat;\nout vec2 lmCoord;\nout vec4 glColor;\nuniform mat4 dhProjection;\nvoid main() { mat = 2; lmCoord = vec2(0.5); glColor = gl_Color; gl_Position = ftransform(); }",
                ),
                ShaderSourceFile::new(
                    "dh_terrain.fsh",
                    "#version 130\nflat in int mat;\nin vec2 lmCoord;\nin vec4 glColor;\nuniform float viewHeight;\nvoid main() { vec2 screen = gl_FragCoord.xy / vec2(1.0, viewHeight); gl_FragData[0] = glColor * vec4(screen, float(mat), 1.0); }",
                ),
            ],
        )
        .unwrap();
        let vertex = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "dh_terrain.vsh",
            defines: &[],
        })
        .unwrap();
        let fragment = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "dh_terrain.fsh",
            defines: &[],
        })
        .unwrap();

        let lowered = lower_distant_horizons_source_pair(&vertex, &fragment).unwrap();
        let fragment = lowered.fragment().source();
        assert!(fragment.contains("vec4 vulkanic_source_world_fragment_coord()"));
        assert!(fragment.contains("coordinate.y = viewHeight - coordinate.y;"));
        assert!(fragment.contains("vulkanic_source_world_fragment_coord().xy"));
        assert!(
            fragment.find("uniform float viewHeight;")
                < fragment.find("vec4 vulkanic_source_world_fragment_coord()"),
            "the source viewport uniform must be declared before the coordinate helper"
        );
    }

    #[test]
    fn lowered_distant_horizons_rejects_fragment_coordinates_without_an_explicit_extent() {
        let pack = ShaderPackSource::new(
            "dh-fragment-coordinate-missing-extent-test",
            1,
            vec![
                ShaderSourceFile::new(
                    "dh_terrain.vsh",
                    "#version 130\nflat out int mat;\nout vec2 lmCoord;\nout vec4 glColor;\nuniform mat4 dhProjection;\nvoid main() { mat = 2; lmCoord = vec2(0.5); glColor = gl_Color; gl_Position = ftransform(); }",
                ),
                ShaderSourceFile::new(
                    "dh_terrain.fsh",
                    "#version 130\nflat in int mat;\nin vec2 lmCoord;\nin vec4 glColor;\nvoid main() { gl_FragData[0] = glColor * vec4(gl_FragCoord.xy, float(mat), 1.0); }",
                ),
            ],
        )
        .unwrap();
        let vertex = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "dh_terrain.vsh",
            defines: &[],
        })
        .unwrap();
        let fragment = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "dh_terrain.fsh",
            defines: &[],
        })
        .unwrap();
        let error = lower_distant_horizons_source_pair(&vertex, &fragment)
            .unwrap_err()
            .to_string();
        assert!(error.contains("does not declare viewHeight"));
    }

    #[test]
    fn lowers_selected_complementary_dh_source_without_relabeling_it_as_near_terrain() {
        let source =
            crate::render::vulkanic::shader_pack::preprocess::complete_bundled_pack_source_for_test(
            );
        let contract = crate::render::vulkanic::shader_pack::distant_horizons_contract::derive_distant_horizons_opaque_contract(
            &source,
            crate::render::vulkanic::shader_pack::terrain_contract::TerrainProgramScope::Overworld,
        )
        .unwrap();
        let artifacts =
            crate::render::vulkanic::shader_pack::preprocess::preprocess_distant_horizons_sources(
                &source,
                &contract.source_stages,
            )
            .unwrap();
        let lowered =
            lower_distant_horizons_source_pair(&artifacts.vertex, &artifacts.fragment).unwrap();
        assert_eq!(
            vec![DistantHorizonsFragmentOutput::LitColor],
            lowered.fragment().outputs()
        );
        assert!(lowered
            .fragment()
            .source()
            .contains("out_distant_horizons_lit_color"));
        assert!(!lowered
            .fragment()
            .source()
            .contains("out_terrain_lit_color"));
        assert!(lowered.vertex().source().contains("dhModelView"));
        assert!(lowered.vertex().source().contains("dhProjection"));
        assert!(lowered
            .vertex()
            .source()
            .contains("VulkanicDistantHorizonsVertices"));
        assert!(!lowered
            .vertex()
            .source()
            .contains("VulkanicSourceTerrainVertices"));
        lowered.require_backend_neutral_lowering().unwrap();
    }

    #[test]
    fn lowers_selected_dh_water_with_the_dh_vertex_stream_not_near_terrain() {
        let complete =
            crate::render::vulkanic::shader_pack::preprocess::complete_bundled_pack_source_for_test(
            );
        let source = ShaderPackSource::new(
            "complete-dh-water-lowering",
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
        let contract = crate::render::vulkanic::shader_pack::distant_horizons_contract::derive_distant_horizons_translucent_contract(
            &source,
            crate::render::vulkanic::shader_pack::terrain_contract::TerrainProgramScope::Overworld,
        )
        .unwrap();
        let artifacts =
            crate::render::vulkanic::shader_pack::preprocess::preprocess_distant_horizons_sources(
                &source,
                &contract.source_stages,
            )
            .unwrap();

        let lowered = lower_distant_horizons_source_pair(&artifacts.vertex, &artifacts.fragment)
            .expect("the selected DH water pair must lower through the DH stream");
        assert_eq!(
            vec![DistantHorizonsFragmentOutput::LitColor],
            lowered.fragment().outputs()
        );
        assert!(lowered
            .vertex()
            .source()
            .contains("VulkanicDistantHorizonsVertices"));
        assert!(lowered.vertex().source().contains("dhProjection"));
        assert!(lowered.fragment().source().contains("depthtex1"));
        assert!(lowered
            .fragment()
            .source()
            .contains("out_distant_horizons_lit_color"));
        assert!(!lowered.fragment().source().contains("gl_FragData"));
        lowered.require_backend_neutral_lowering().unwrap();
    }

    #[test]
    fn lowers_the_selected_scoped_shadow_fragment_without_a_terrain_output_alias() {
        let source =
            crate::render::vulkanic::shader_pack::preprocess::complete_bundled_pack_source_for_test(
            );
        let stages = crate::render::vulkanic::shader_pack::terrain_contract::shadow_source_stages_for_scope(
            &source,
            crate::render::vulkanic::shader_pack::terrain_contract::TerrainProgramScope::Overworld,
        )
        .unwrap();
        let artifacts =
            crate::render::vulkanic::shader_pack::preprocess::preprocess_terrain_sources(
                &source, &stages,
            )
            .unwrap();
        let lowered = lower_shadow_fragment_surface(&artifacts.fragment).unwrap();
        assert!(lowered
            .outputs()
            .contains(&ShadowFragmentOutput::ShadowColor));
        assert!(lowered.source().contains("out_shadow_color"));
        assert!(!lowered.source().contains("out_terrain_lit_color"));
    }

    #[test]
    fn shadow_pair_uses_shadow_transforms_and_keeps_shadow_outputs_distinct() {
        let pack = ShaderPackSource::new(
            "shadow-test",
            1,
            vec![
                ShaderSourceFile::new(
                    "shadow.vsh",
                    "#version 130\nout vec2 tex_coord;\nvoid main() { tex_coord = gl_MultiTexCoord0.xy; gl_Position = ftransform(); }",
                ),
                ShaderSourceFile::new(
                    "shadow.fsh",
                    "#version 130\nin vec2 tex_coord;\nuniform sampler2D tex;\nvoid main() { gl_FragData[0] = texture2D(tex, tex_coord); }",
                ),
            ],
        )
        .unwrap();
        let vertex = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "shadow.vsh",
            defines: &[],
        })
        .unwrap();
        let fragment = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "shadow.fsh",
            defines: &[],
        })
        .unwrap();

        let lowered = lower_shadow_source_pair(&vertex, &fragment).unwrap();

        assert!(lowered.vertex().source().contains("shadowModelView"));
        assert!(lowered.vertex().source().contains("shadowProjection"));
        assert!(!lowered.vertex().source().contains("gbufferModelView"));
        assert!(lowered
            .fragment()
            .source()
            .contains("layout(location = 0) in vec2 tex_coord"));
        assert!(lowered.fragment().source().contains("out_shadow_color"));
        assert!(!lowered
            .fragment()
            .source()
            .contains("out_terrain_lit_color"));
        lowered.require_backend_neutral_lowering().unwrap();
    }

    #[test]
    fn selected_scoped_shadow_pair_preserves_shadow_transform_and_output_semantics() {
        let source =
            crate::render::vulkanic::shader_pack::preprocess::complete_bundled_pack_source_for_test(
            );
        let stages = crate::render::vulkanic::shader_pack::terrain_contract::shadow_source_stages_for_scope(
            &source,
            crate::render::vulkanic::shader_pack::terrain_contract::TerrainProgramScope::Overworld,
        )
        .unwrap();
        let artifacts =
            crate::render::vulkanic::shader_pack::preprocess::preprocess_terrain_sources(
                &source, &stages,
            )
            .unwrap();

        let lowered = lower_shadow_source_pair(&artifacts.vertex, &artifacts.fragment).unwrap();

        assert!(lowered.vertex().source().contains("shadowModelView"));
        assert!(lowered.vertex().source().contains("shadowProjection"));
        assert!(!lowered
            .vertex()
            .source()
            .contains("#define vulkanic_source_model_view (gbufferModelView"));
        assert!(lowered
            .fragment()
            .outputs()
            .contains(&ShadowFragmentOutput::ShadowColor));
        assert!(!lowered
            .fragment()
            .source()
            .contains("out_terrain_lit_color"));
        lowered.require_backend_neutral_lowering().unwrap();
    }

    #[test]
    fn rejects_unknown_legacy_vertex_attributes() {
        let error = lower_terrain_vertex_surface(&artifact(
            "#version 130\nattribute vec4 unmodeled_input;\nvoid main() {}",
        ))
        .unwrap_err();
        assert!(error.to_string().contains("unmodeled_input"));
    }

    #[test]
    fn pair_lowering_uses_one_deterministic_uniform_block_for_both_stages() {
        let pack = ShaderPackSource::new(
            "test",
            1,
            vec![
                ShaderSourceFile::new(
                    "terrain.vsh",
                    "#version 130\nuniform float time;\nuniform mat4 view;\nvoid main() { gl_Position = gl_Vertex + vec4(time + view[0][0]); }",
                ),
                ShaderSourceFile::new(
                    "terrain.fsh",
                    "#version 130\nuniform float time;\nuniform vec3 fog_color;\nvoid main() { gl_FragData[0] = vec4(time + fog_color.x); }",
                ),
            ],
        )
        .unwrap();
        let vertex = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "terrain.vsh",
            defines: &[],
        })
        .unwrap();
        let fragment = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "terrain.fsh",
            defines: &[],
        })
        .unwrap();
        let lowered = lower_terrain_source_pair(&vertex, &fragment).unwrap();
        assert_eq!(
            lowered.uniform_contract().declarations(),
            ["vec3 fog_color;", "float time;", "mat4 view;"]
        );
        assert_eq!(80, lowered.uniform_contract().std140_size());
        assert_eq!(
            vec![
                ("fog_color", TerrainSourceUniformType::Vec3, 0, 12, 0),
                ("time", TerrainSourceUniformType::Float, 12, 4, 0),
                ("view", TerrainSourceUniformType::Mat4, 16, 64, 0),
            ],
            lowered
                .uniform_contract()
                .fields()
                .iter()
                .map(|field| (
                    field.name(),
                    field.ty(),
                    field.offset(),
                    field.size(),
                    field.array_stride(),
                ))
                .collect::<Vec<_>>()
        );
        let expected = uniform_block(lowered.uniform_contract());
        assert!(lowered.vertex().source().contains(&expected));
        assert!(lowered.fragment().source().contains(&expected));
    }

    #[test]
    fn translucent_pair_lowering_keeps_its_auxiliary_output_distinct_from_normal_terrain() {
        let vertex = artifact("#version 130\nvoid main() { gl_Position = gl_Vertex; }");
        let fragment = artifact(
            "#version 130\nvoid main() { gl_FragData[0] = vec4(1.0); gl_FragData[1] = vec4(0.5); gl_FragData[2] = vec4(0.25); }",
        );
        let lowered = lower_translucent_terrain_source_pair(&vertex, &fragment).unwrap();
        assert_eq!(
            &[
                TranslucentTerrainFragmentOutput::LitColor,
                TranslucentTerrainFragmentOutput::TranslucencyAuxiliary,
                TranslucentTerrainFragmentOutput::MaterialAuxiliary,
            ],
            lowered.fragment().outputs()
        );
        assert!(lowered
            .fragment()
            .source()
            .contains("out_terrain_translucency_auxiliary"));
        assert!(!lowered.fragment().source().contains("gl_FragData[1]"));
    }

    #[test]
    fn terrain_and_translucent_fragments_preserve_lower_left_source_fragment_coordinates() {
        let vertex = artifact("#version 130\nvoid main() { gl_Position = gl_Vertex; }");
        let terrain_fragment = artifact(
            "#version 130\nuniform float viewHeight;\nvoid main() { vec2 screen = gl_FragCoord.xy / vec2(1.0, viewHeight); gl_FragData[0] = vec4(screen, 0.0, 1.0); }",
        );
        let translucent_fragment = artifact(
            "#version 130\nuniform float viewHeight;\nvoid main() { vec2 screen = gl_FragCoord.xy / vec2(1.0, viewHeight); gl_FragData[0] = vec4(screen, 0.0, 1.0); gl_FragData[1] = vec4(1.0); }",
        );

        let terrain = lower_terrain_source_pair(&vertex, &terrain_fragment).unwrap();
        let translucent =
            lower_translucent_terrain_source_pair(&vertex, &translucent_fragment).unwrap();
        for lowered in [terrain.fragment().source(), translucent.fragment().source()] {
            assert!(lowered.contains("vec4 vulkanic_source_world_fragment_coord()"));
            assert!(lowered.contains("coordinate.y = viewHeight - coordinate.y;"));
            assert!(lowered.contains("vulkanic_source_world_fragment_coord().xy"));
            assert!(
                lowered.find("uniform float viewHeight;")
                    < lowered.find("vec4 vulkanic_source_world_fragment_coord()"),
                "the source extent must be declared before the coordinate helper"
            );
        }
    }

    #[test]
    fn world_material_screen_target_sampling_uses_native_image_coordinates() {
        let source = concat!(
            "#version 130\n",
            "uniform float viewHeight;\n",
            "uniform sampler2D depthtex1;\n",
            "uniform sampler2D tex;\n",
            "void main() {\n",
            "  vec2 screen = gl_FragCoord.xy / vec2(1.0, viewHeight);\n",
            "  vec4 scene = texture2D(depthtex1, screen);\n",
            "  vec4 atlas = texture2D(tex, screen);\n",
            "  gl_FragData[0] = scene + atlas;\n",
            "}\n"
        );
        let lowered = lower_world_material_fragment_coordinates(
            replace_identifier(source, "texture2D", "texture"),
            &TerrainSourceUniformContract {
                declarations: vec!["float viewHeight;".to_string()],
                fields: vec![TerrainSourceUniformField {
                    name: "viewHeight".to_string(),
                    ty: TerrainSourceUniformType::Float,
                    array_length: 1,
                    offset: 0,
                    size: 4,
                    array_stride: 0,
                }],
                std140_size: 4,
            },
        )
        .unwrap();

        assert!(lowered.contains(
            "#define vulkanic_source_sample_target_depthtex1(source_uv) texture(depthtex1, vulkanic_source_world_target_uv(source_uv))"
        ));
        assert!(lowered.contains("scene = vulkanic_source_sample_target_depthtex1( screen);"));
        assert!(lowered.contains("vec4 atlas = texture(tex, screen);"));
        assert!(lowered.contains("vec2((source_uv).x, 1.0 - (source_uv).y)"));
    }

    #[test]
    fn terrain_fragment_coordinates_without_a_source_extent_are_rejected() {
        let vertex = artifact("#version 130\nvoid main() { gl_Position = gl_Vertex; }");
        let fragment = artifact(
            "#version 130\nvoid main() { gl_FragData[0] = vec4(gl_FragCoord.xy, 0.0, 1.0); }",
        );
        let error = lower_terrain_source_pair(&vertex, &fragment)
            .unwrap_err()
            .to_string();
        assert!(error.contains("does not declare viewHeight"));
    }

    #[test]
    fn paired_uniform_type_mismatch_is_rejected_before_lowering() {
        let pack = ShaderPackSource::new(
            "test",
            1,
            vec![
                ShaderSourceFile::new(
                    "terrain.vsh",
                    "#version 130\nuniform float shared;\nvoid main() { gl_Position = vec4(shared); }",
                ),
                ShaderSourceFile::new(
                    "terrain.fsh",
                    "#version 130\nuniform vec2 shared;\nvoid main() { gl_FragData[0] = vec4(shared, 0.0, 1.0); }",
                ),
            ],
        )
        .unwrap();
        let vertex = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "terrain.vsh",
            defines: &[],
        })
        .unwrap();
        let fragment = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "terrain.fsh",
            defines: &[],
        })
        .unwrap();
        let error = lower_terrain_source_pair(&vertex, &fragment).unwrap_err();
        assert!(error
            .to_string()
            .contains("incompatible paired declarations"));
    }

    #[test]
    fn scalar_uniform_layout_rejects_unknown_types_and_lays_out_arrays() {
        let array = artifact(
            "#version 130\nuniform vec2 weights[3];\nvoid main() { gl_Position = vec4(weights[0], 0.0, 1.0); }",
        );
        let layout = derive_terrain_source_uniform_contract(&array, &array).unwrap();
        assert_eq!(48, layout.std140_size());
        assert_eq!(1, layout.fields().len());
        let field = &layout.fields()[0];
        assert_eq!("weights", field.name());
        assert_eq!(TerrainSourceUniformType::Vec2, field.ty());
        assert_eq!(3, field.array_length());
        assert_eq!(0, field.offset());
        assert_eq!(48, field.size());
        assert_eq!(16, field.array_stride());

        let unsupported = artifact(
            "#version 130\nuniform double invalid;\nvoid main() { gl_Position = vec4(float(invalid)); }",
        );
        let error = derive_terrain_source_uniform_contract(&unsupported, &unsupported)
            .unwrap_err()
            .to_string();
        assert!(error.contains("double"));
        assert!(error.contains("std140 semantic layout"));
    }

    #[test]
    fn scalar_uniform_layout_packs_vec3_tail_scalars_without_losing_std140_array_stride() {
        let scalar_tail = artifact(
            "#version 130\nuniform vec3 a_position;\nuniform float b_exposure;\nuniform vec4 c_tint;\nvoid main() { gl_Position = vec4(a_position, 1.0) + vec4(b_exposure) + c_tint; }",
        );
        let layout = derive_terrain_source_uniform_contract(&scalar_tail, &scalar_tail).unwrap();
        assert_eq!(32, layout.std140_size());
        assert_eq!(3, layout.fields().len());
        assert_eq!("a_position", layout.fields()[0].name());
        assert_eq!(0, layout.fields()[0].offset());
        assert_eq!(12, layout.fields()[0].size());
        assert_eq!("b_exposure", layout.fields()[1].name());
        assert_eq!(12, layout.fields()[1].offset());
        assert_eq!("c_tint", layout.fields()[2].name());
        assert_eq!(16, layout.fields()[2].offset());

        let array = artifact(
            "#version 130\nuniform vec3 samples[2];\nvoid main() { gl_Position = vec4(samples[1], 1.0); }",
        );
        let array_layout = derive_terrain_source_uniform_contract(&array, &array).unwrap();
        let field = &array_layout.fields()[0];
        assert_eq!(32, array_layout.std140_size());
        assert_eq!(32, field.size());
        assert_eq!(16, field.array_stride());
    }

    #[test]
    fn uniform_contract_omits_declaration_only_values_but_keeps_lowered_legacy_matrices() {
        let source = artifact(
            "#version 130\nuniform float unused;\nuniform float active;\nuniform mat4 gbufferModelView;\nuniform mat4 gbufferProjection;\n// unused must not count as a source reference\nvoid main() { gl_Position = ftransform() + vec4(active); }",
        );
        let contract = derive_terrain_source_uniform_contract(&source, &source).unwrap();
        assert_eq!(
            [
                "float active;",
                "mat4 gbufferModelView;",
                "mat4 gbufferProjection;",
            ],
            contract.declarations()
        );
        assert_eq!(144, contract.std140_size());

        let identifiers = glsl_identifiers("// ignored\nactive /* ignored too */ gbufferModelView");
        assert!(identifiers.contains("active"));
        assert!(identifiers.contains("gbufferModelView"));
        assert!(!identifiers.contains("ignored"));
    }

    #[test]
    fn legacy_transform_lowering_rejects_conflicting_semantic_matrix_declarations() {
        let source = artifact(
            "#version 130\nuniform float gbufferModelView;\nvoid main() { gl_Position = ftransform(); }",
        );
        let error = derive_terrain_source_uniform_contract(&source, &source)
            .unwrap_err()
            .to_string();
        assert!(error.contains("gbufferModelView"));
        assert!(error.contains("legacy transform"));
    }

    #[test]
    fn post_effect_interface_linking_matches_names_not_declaration_order() {
        let (vertex, fragment) = bind_simple_paired_varyings(
            "out vec2 first;\nout vec3 second;\n",
            "in vec3 second;\nin vec2 first;\n").unwrap();
        assert!(vertex.contains("layout(location = 0) out vec2 first;"));
        assert!(fragment.contains("layout(location = 0) in vec2 first;"));
        assert!(vertex.contains("layout(location = 1) out vec3 second;"));
        assert!(fragment.contains("layout(location = 1) in vec3 second;"));
        for input in ["in vec3 first;\n", "in vec2 missing;\n", "flat in vec2 first;\n"] {
            assert!(bind_simple_paired_varyings("out vec2 first;\n", input).is_err());
        }
        assert!(bind_simple_paired_varyings("out mat4 matrix;\n", "in mat4 matrix;\n").is_err());
        assert!(bind_simple_paired_varyings("out vec2 values[2];\n", "in vec2 values[2];\n").is_err());
        assert!(bind_simple_paired_varyings("layout(location=0) out vec2 fixed;\nout vec2 other;\n",
            "in vec2 other;\n").is_err());
    }

    #[test]
    fn pair_lowering_assigns_matching_varyings_stable_locations() {
        let pack = ShaderPackSource::new(
            "test",
            1,
            vec![
                ShaderSourceFile::new(
                    "terrain.vsh",
                    "#version 130\nflat out int material;\nout vec2 atlas_uv;\nout vec3 normal;\nvoid main() { gl_Position = gl_Vertex; }",
                ),
                ShaderSourceFile::new(
                    "terrain.fsh",
                    "#version 130\nflat in int material;\nin vec2 atlas_uv;\nin vec3 normal;\nvoid main() { gl_FragData[0] = vec4(float(material) + atlas_uv.x + normal.x); }",
                ),
            ],
        )
        .unwrap();
        let vertex = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "terrain.vsh",
            defines: &[],
        })
        .unwrap();
        let fragment = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "terrain.fsh",
            defines: &[],
        })
        .unwrap();
        let lowered = lower_terrain_source_pair(&vertex, &fragment).unwrap();
        assert_eq!(
            lowered
                .varying_contract()
                .fields()
                .iter()
                .map(|field| (field.name(), field.location()))
                .collect::<Vec<_>>(),
            vec![("atlas_uv", 0), ("material", 1), ("normal", 2)]
        );
        assert!(lowered
            .vertex()
            .source()
            .contains("layout(location = 1) flat out int material;"));
        assert!(lowered
            .fragment()
            .source()
            .contains("layout(location = 1) flat in int material;"));
    }

    #[test]
    fn paired_varying_mismatch_is_rejected_before_lowering() {
        let pack = ShaderPackSource::new(
            "test",
            1,
            vec![
                ShaderSourceFile::new(
                    "terrain.vsh",
                    "#version 130\nout vec3 normal;\nvoid main() {}",
                ),
                ShaderSourceFile::new(
                    "terrain.fsh",
                    "#version 130\nin vec2 normal;\nvoid main() { gl_FragData[0] = vec4(1.0); }",
                ),
            ],
        )
        .unwrap();
        let vertex = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "terrain.vsh",
            defines: &[],
        })
        .unwrap();
        let fragment = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "terrain.fsh",
            defines: &[],
        })
        .unwrap();
        let error = lower_terrain_source_pair(&vertex, &fragment).unwrap_err();
        assert!(error.to_string().contains("differs between vertex output"));
    }

    #[test]
    fn pair_lowering_assigns_stable_bindings_to_opaque_resources() {
        let pack = ShaderPackSource::new(
            "test",
            1,
            vec![
                ShaderSourceFile::new(
                    "terrain.vsh",
                    "#version 130\nuniform sampler2D atlas;\nuniform usampler3D occupancy;\nvoid main() {}",
                ),
                ShaderSourceFile::new(
                    "terrain.fsh",
                    "#version 130\nuniform sampler2D atlas;\nvoid main() { gl_FragData[0] = texture2D(atlas, vec2(0.0)); }",
                ),
            ],
        )
        .unwrap();
        let vertex = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "terrain.vsh",
            defines: &[],
        })
        .unwrap();
        let fragment = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "terrain.fsh",
            defines: &[],
        })
        .unwrap();
        let lowered = lower_terrain_source_pair(&vertex, &fragment).unwrap();
        assert_eq!(
            lowered
                .opaque_resource_contract()
                .resources()
                .iter()
                .map(|resource| (resource.name(), resource.binding()))
                .collect::<Vec<_>>(),
            vec![("atlas", 0), ("occupancy", 1)]
        );
        assert!(lowered
            .vertex()
            .source()
            .contains("layout(set = 1, binding = 0) uniform sampler2D atlas;"));
        assert!(lowered
            .vertex()
            .source()
            .contains("layout(set = 1, binding = 1) uniform usampler3D occupancy;"));
    }

    #[test]
    fn paired_opaque_resource_type_mismatch_is_rejected_before_lowering() {
        let pack = ShaderPackSource::new(
            "test",
            1,
            vec![
                ShaderSourceFile::new(
                    "terrain.vsh",
                    "#version 130\nuniform sampler2D shared;\nvoid main() {}",
                ),
                ShaderSourceFile::new(
                    "terrain.fsh",
                    "#version 130\nuniform usampler2D shared;\nvoid main() { gl_FragData[0] = vec4(1.0); }",
                ),
            ],
        )
        .unwrap();
        let vertex = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "terrain.vsh",
            defines: &[],
        })
        .unwrap();
        let fragment = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "terrain.fsh",
            defines: &[],
        })
        .unwrap();
        let error = lower_terrain_source_pair(&vertex, &fragment).unwrap_err();
        assert!(error.to_string().contains("opaque resource 'shared'"));
    }

    #[test]
    fn opaque_resources_require_exact_pack_declared_semantic_roles() {
        let pack = ShaderPackSource::new(
            "test",
            1,
            vec![
                ShaderSourceFile::new(
                    "terrain.vsh",
                    "#version 130\nuniform sampler2D tex;\nuniform usampler3D voxel_sampler;\nuniform sampler2D unused_global;\nvoid main() {}",
                ),
                ShaderSourceFile::new(
                    "terrain.fsh",
                    "#version 130\nuniform sampler2D tex;\nuniform usampler3D voxel_sampler;\nuniform sampler2D unused_global;\nvoid main() { gl_FragData[0] = texture2D(tex, vec2(0.0)); }",
                ),
                ShaderSourceFile::new(
                    super::super::terrain_source_resources::TERRAIN_RESOURCE_BINDINGS_PATH,
                    "tex=material_atlas\nunused_global=noise\n",
                ),
            ],
        )
        .unwrap();
        let vertex = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "terrain.vsh",
            defines: &[],
        })
        .unwrap();
        let fragment = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "terrain.fsh",
            defines: &[],
        })
        .unwrap();
        let lowered = lower_terrain_source_pair(&vertex, &fragment).unwrap();
        let declarations = TerrainSourceResourceBindings::from_source(&pack).unwrap();
        let plan = lowered
            .opaque_resource_contract()
            .bind_semantic_roles(&declarations)
            .unwrap();
        assert_eq!(
            vec![("tex", TerrainSourceResourceRole::MaterialAtlas, 0)],
            plan.bindings()
                .iter()
                .map(|binding| (binding.resource_name(), binding.role(), binding.binding()))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn raw_shadow_depth_sampling_resolves_the_owned_depth_image_without_a_compare_sampler() {
        let pack = ShaderPackSource::new(
            "test",
            1,
            vec![
                ShaderSourceFile::new(
                    "terrain.vsh",
                    "#version 130\nuniform sampler2D shadowtex0;\nvoid main() {}",
                ),
                ShaderSourceFile::new(
                    "terrain.fsh",
                    "#version 130\nuniform sampler2D shadowtex0;\nvoid main() { gl_FragData[0] = texture2D(shadowtex0, vec2(0.0)); }",
                ),
                ShaderSourceFile::new(
                    super::super::terrain_source_resources::TERRAIN_RESOURCE_BINDINGS_PATH,
                    "shadowtex0=shadow_depth\n",
                ),
            ],
        )
        .unwrap();
        let vertex = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "terrain.vsh",
            defines: &[],
        })
        .unwrap();
        let fragment = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "terrain.fsh",
            defines: &[],
        })
        .unwrap();
        let lowered = lower_terrain_source_pair(&vertex, &fragment).unwrap();
        let declarations = TerrainSourceResourceBindings::from_source(&pack).unwrap();
        let plan = lowered
            .opaque_resource_contract()
            .bind_semantic_roles(&declarations)
            .unwrap();
        assert_eq!(
            TerrainSourceResourceRole::ShadowDepthRaw,
            plan.bindings()[0].role()
        );
    }

    #[test]
    fn opaque_resource_plan_ignores_unreferenced_global_sampler_declarations() {
        let pack = ShaderPackSource::new(
            "test",
            1,
            vec![
                ShaderSourceFile::new(
                    "terrain.vsh",
                    "#version 130\nuniform sampler2D tex;\nuniform sampler2D unused_global;\nvoid main() {}",
                ),
                ShaderSourceFile::new(
                    "terrain.fsh",
                    "#version 130\nuniform sampler2D tex;\nuniform sampler2D unused_global;\nvoid main() { gl_FragData[0] = texture2D(tex, vec2(0.0)); }",
                ),
                ShaderSourceFile::new(
                    super::super::terrain_source_resources::TERRAIN_RESOURCE_BINDINGS_PATH,
                    "tex=material_atlas\n",
                ),
            ],
        )
        .unwrap();
        let vertex = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "terrain.vsh",
            defines: &[],
        })
        .unwrap();
        let fragment = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "terrain.fsh",
            defines: &[],
        })
        .unwrap();
        let lowered = lower_terrain_source_pair(&vertex, &fragment).unwrap();
        let resources = lowered.opaque_resource_contract().resources();
        assert_eq!(
            vec![("tex", true), ("unused_global", false)],
            resources
                .iter()
                .map(|resource| (resource.name(), resource.active()))
                .collect::<Vec<_>>()
        );
        let declarations = TerrainSourceResourceBindings::from_source(&pack).unwrap();
        let plan = lowered
            .opaque_resource_contract()
            .bind_semantic_roles(&declarations)
            .unwrap();
        assert_eq!(1, plan.bindings().len());
        assert_eq!(
            vec!["tex"],
            plan.bindings()
                .iter()
                .map(|binding| binding.resource_name())
                .collect::<Vec<_>>()
        );
        assert!(lowered
            .fragment()
            .source()
            .contains("layout(set = 1, binding = 1) uniform sampler2D unused_global;"));
    }

    #[test]
    fn opaque_resources_reject_missing_or_unused_semantic_roles() {
        let pack = ShaderPackSource::new(
            "test",
            1,
            vec![
                ShaderSourceFile::new(
                    "terrain.vsh",
                    "#version 130\nuniform sampler2D tex;\nvoid main() {}",
                ),
                ShaderSourceFile::new(
                    "terrain.fsh",
                    "#version 130\nuniform sampler2D tex;\nvoid main() { gl_FragData[0] = texture2D(tex, vec2(0.0)); }",
                ),
            ],
        )
        .unwrap();
        let vertex = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "terrain.vsh",
            defines: &[],
        })
        .unwrap();
        let fragment = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "terrain.fsh",
            defines: &[],
        })
        .unwrap();
        let lowered = lower_terrain_source_pair(&vertex, &fragment).unwrap();
        let declarations = TerrainSourceResourceBindings::from_source(&pack).unwrap();
        assert!(lowered
            .opaque_resource_contract()
            .bind_semantic_roles(&declarations)
            .unwrap_err()
            .to_string()
            .contains("have no declared semantic roles: tex"));
    }

    #[test]
    fn opaque_resources_ignore_pack_wide_declarations_owned_by_other_source_stages() {
        let pack = ShaderPackSource::new(
            "test",
            1,
            vec![
                ShaderSourceFile::new(
                    "terrain.vsh",
                    "#version 130\nuniform sampler2D tex;\nvoid main() {}",
                ),
                ShaderSourceFile::new(
                    "terrain.fsh",
                    "#version 130\nuniform sampler2D tex;\nvoid main() { gl_FragData[0] = texture2D(tex, vec2(0.0)); }",
                ),
                ShaderSourceFile::new(
                    super::super::terrain_source_resources::TERRAIN_RESOURCE_BINDINGS_PATH,
                    "tex=material_atlas\nabsent=noise\n",
                ),
            ],
        )
        .unwrap();
        let vertex = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "terrain.vsh",
            defines: &[],
        })
        .unwrap();
        let fragment = preprocess_artifact(PreprocessInput {
            source: &pack,
            entry: "terrain.fsh",
            defines: &[],
        })
        .unwrap();
        let lowered = lower_terrain_source_pair(&vertex, &fragment).unwrap();
        let declarations = TerrainSourceResourceBindings::from_source(&pack).unwrap();
        let plan = lowered
            .opaque_resource_contract()
            .bind_semantic_roles(&declarations)
            .unwrap();
        assert_eq!(
            vec!["tex"],
            plan.bindings()
                .iter()
                .map(|binding| binding.resource_name())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn lowered_distant_horizons_positions_do_not_apply_dimension_world_y_offset_twice() {
        assert!(!DISTANT_HORIZONS_VERTEX_SEMANTIC_PREAMBLE
            .contains("+ vec3(0.0, vulkanic_source_dh_column_origin_and_world_y.w, 0.0)"));
    }
}
