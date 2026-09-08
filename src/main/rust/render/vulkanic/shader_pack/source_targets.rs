//! Source-derived declarations for named shader-pack color targets.
//!
//! Shader packs express these through legacy `colortexN` declarations. Those
//! names are parsed only at this source boundary, then resolved through the
//! pack's semantic resource table. The result intentionally contains neither
//! attachment indices nor backend handles.

use std::collections::BTreeMap;

use xxhash_rust::xxh32::xxh32;

use crate::render::vulkanic::commands::{
    AttachmentLoadOp, AttachmentStoreOp, ClearColor, CommandOp, PassAttachment, ResourceBarrier,
    TextureImageCopyRegion, TextureOrigin3d, TextureUsageState,
};
use crate::render::vulkanic::error::{GalError, GalResult, StatusCode};
use crate::render::vulkanic::gal::VulkanicGal;
use crate::render::vulkanic::handles::Handle;
use crate::render::vulkanic::resources::{
    CombinedTextureSamplerDesc, Extent3d, QueueClass, RenderPassDesc, RenderTargetDesc,
    SamplerAddressMode, SamplerDesc, SamplerFilter, TextureDesc, TextureDimension, TextureFormat,
    TextureSubresourceRange, TextureUsage, TextureViewDesc,
};

use super::lowering::{
    FullscreenSourceFragmentOutput, TerrainSourceOpaqueResourceBindingPlan,
    TerrainSourceOpaqueResourceKind,
};
use super::programs::LoweredFullscreenSourceProgram;
use super::source::ShaderPackSource;
use super::terrain_contract::TerrainPassOutput;
use super::terrain_source_resources::{
    TerrainSourceOwnedResource, TerrainSourceOwnedResourceSet, TerrainSourceResourceAvailability,
    TerrainSourceResourceAvailabilitySet, TerrainSourceResourceBindings, TerrainSourceResourceRole,
};

pub const PIPELINE_SETTINGS_PATH: &str = "lib/pipelineSettings.glsl";
const MAX_SOURCE_COLOR_TARGETS: u32 = 8;

/// Pack-level color format before a backend capability decision. This keeps
/// a source declaration such as `R11F_G11F_B10F` explicit even while the
/// current GAL has not yet exposed that color format.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ShaderPackColorFormat {
    R11fG11fB10f,
    R32f,
    Rgb16f,
    Rgba8,
    R8,
    Rgba8Snorm,
    Rgba16f,
}

impl ShaderPackColorFormat {
    fn parse(value: &str) -> GalResult<Self> {
        match value {
            "R11F_G11F_B10F" => Ok(Self::R11fG11fB10f),
            "R32F" => Ok(Self::R32f),
            "RGB16F" => Ok(Self::Rgb16f),
            "RGBA8" => Ok(Self::Rgba8),
            "R8" => Ok(Self::R8),
            "RGBA8_SNORM" => Ok(Self::Rgba8Snorm),
            "RGBA16F" => Ok(Self::Rgba16f),
            _ => Err(GalError::unsupported_feature(format!(
                "unsupported shader-pack color target format '{value}'"
            ))),
        }
    }

    /// The exact generic GAL format declared by this source target. Native
    /// device/driver image-format support is checked when the Rust-owned
    /// target cache stages resources; there is no optional schema fallback.
    pub const fn gal_schema_color_format(self) -> TextureFormat {
        match self {
            Self::R11fG11fB10f => TextureFormat::R11fG11fB10f,
            Self::R32f => TextureFormat::R32Float,
            Self::Rgb16f => TextureFormat::Rgb16Float,
            Self::Rgba8 => TextureFormat::Rgba8Unorm,
            Self::R8 => TextureFormat::R8Unorm,
            Self::Rgba8Snorm => TextureFormat::Rgba8Snorm,
            Self::Rgba16f => TextureFormat::Rgba16Float,
        }
    }

    /// Physical Rust-owned storage formats that retain this source-level
    /// color contract. RGB16F has no Vulkan color-attachment guarantee on
    /// common devices, while RGBA16F retains its sampled RGB precision. The
    /// source contract continues to identify the target as RGB16F; callers
    /// only observe the selected backend-neutral storage format.
    const fn compatible_storage_formats(self) -> &'static [TextureFormat] {
        match self {
            Self::Rgb16f => &[TextureFormat::Rgb16Float, TextureFormat::Rgba16Float],
            Self::R11fG11fB10f => &[TextureFormat::R11fG11fB10f],
            Self::R32f => &[TextureFormat::R32Float],
            Self::Rgba8 => &[TextureFormat::Rgba8Unorm],
            Self::R8 => &[TextureFormat::R8Unorm],
            Self::Rgba8Snorm => &[TextureFormat::Rgba8Snorm],
            Self::Rgba16f => &[TextureFormat::Rgba16Float],
        }
    }

    fn accepts_storage_format(self, format: TextureFormat) -> bool {
        match self {
            Self::Rgb16f => matches!(
                format,
                TextureFormat::Rgb16Float | TextureFormat::Rgba16Float
            ),
            _ => self.gal_schema_color_format() == format,
        }
    }
}

/// One named color target as declared by a source generation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShaderPackColorTargetDecl {
    /// Source-declared output slot. This stays source-level metadata: runtime
    /// scheduling resolves it to a Rust-owned attachment, never an Iris or
    /// OpenGL draw-buffer identity.
    pub source_slot: u32,
    pub role: TerrainSourceResourceRole,
    pub format: ShaderPackColorFormat,
    pub clear_each_frame: bool,
    /// Optional literal `colortexNClearColor` supplied by the selected source
    /// generation. Bits preserve exact source values while keeping manifest
    /// equality independent of floating-point comparison rules.
    pub clear_color_bits: Option<[u32; 4]>,
}

impl ShaderPackColorTargetDecl {
    pub fn name(&self) -> &str {
        self.role
            .shader_pack_color_name()
            .expect("source target declarations always use ShaderPackColor roles")
    }

    pub const fn source_slot(&self) -> u32 {
        self.source_slot
    }

    pub const fn gal_schema_color_format(&self) -> TextureFormat {
        self.format.gal_schema_color_format()
    }

    fn accepts_storage_format(&self, format: TextureFormat) -> bool {
        self.format.accepts_storage_format(format)
    }
}

/// Immutable source-generation target metadata. It is a preparation artifact
/// only: target allocation, feedback ping-pong, pass execution, and route
/// selection remain separate Rust runtime responsibilities.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShaderPackColorTargetManifest {
    pack_name: String,
    generation: u64,
    targets: BTreeMap<String, ShaderPackColorTargetDecl>,
}

/// A source-derived identity for one private set of shader-pack color images.
/// It deliberately uses semantic generations and extents rather than a
/// frame target, attachment number, native image, or backend handle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ShaderPackColorTargetIdentity {
    pub world_generation: u64,
    pub shader_pack_generation: u64,
    pub extent: Extent3d,
    /// Sorted semantic names which require previous/current images because a
    /// source fullscreen stage reads and writes the same named color target.
    pub feedback_target_names: Vec<String>,
    /// Sorted semantic names whose active source program explicitly requests
    /// a complete mip chain. Allocation alone never declares those mips
    /// initialized; a later source pass must generate them before sampling.
    pub mipmapped_target_names: Vec<String>,
}

impl ShaderPackColorTargetIdentity {
    pub(crate) fn new(
        world_generation: u64,
        shader_pack_generation: u64,
        extent: Extent3d,
        feedback_target_names: impl IntoIterator<Item = String>,
        mipmapped_target_names: impl IntoIterator<Item = String>,
    ) -> GalResult<Self> {
        let mut feedback_target_names = feedback_target_names.into_iter().collect::<Vec<_>>();
        feedback_target_names.sort();
        feedback_target_names.dedup();
        let mut mipmapped_target_names = mipmapped_target_names.into_iter().collect::<Vec<_>>();
        mipmapped_target_names.sort();
        mipmapped_target_names.dedup();
        let identity = Self {
            world_generation,
            shader_pack_generation,
            extent,
            feedback_target_names,
            mipmapped_target_names,
        };
        identity.validate()?;
        Ok(identity)
    }

    fn validate(&self) -> GalResult<()> {
        if self.world_generation == 0 || self.shader_pack_generation == 0 {
            return Err(GalError::invalid_argument(
                "shader-pack color target identity requires non-zero world and shader-pack generations",
            ));
        }
        if self.extent.width == 0 || self.extent.height == 0 || self.extent.depth != 1 {
            return Err(GalError::invalid_argument(
                "shader-pack color targets require a non-zero two-dimensional extent",
            ));
        }
        if self
            .feedback_target_names
            .iter()
            .chain(self.mipmapped_target_names.iter())
            .any(|name| {
                name.is_empty()
                    || !name.bytes().all(|byte| {
                        byte == b'_' || byte.is_ascii_lowercase() || byte.is_ascii_digit()
                    })
            })
        {
            return Err(GalError::invalid_argument(
                "shader-pack color target feedback/mipmap names must be normalized semantic identifiers",
            ));
        }
        Ok(())
    }
}

/// Rust-owned handles for one semantic color target. A feedback target has a
/// distinct previous image; non-feedback targets intentionally own only one
/// image so the runtime cannot accidentally add temporal copies to unrelated
/// source stages.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ShaderPackColorTarget {
    pub source_slot: u32,
    pub format: TextureFormat,
    pub clear_each_frame: bool,
    pub clear_color_bits: Option<[u32; 4]>,
    pub mip_levels: u32,
    pub current_texture: Handle,
    /// A sampled view may cover the full source-requested mip chain. Render
    /// targets always use this explicit base-mip attachment view instead.
    pub current_attachment_view: Handle,
    pub current_view: Handle,
    pub previous_texture: Option<Handle>,
    pub previous_attachment_view: Option<Handle>,
    pub previous_view: Option<Handle>,
}

/// A complete private source-generation color target set. The map key is a
/// semantic resource name, never `colortexN` or a backend attachment index.
#[derive(Clone, Debug)]
pub(crate) struct ShaderPackColorTargets {
    pub identity: ShaderPackColorTargetIdentity,
    targets: BTreeMap<String, ShaderPackColorTarget>,
}

/// One resolved, Rust-owned color attachment for a source fullscreen pass.
/// `source_slot` is the semantic destination recovered from `DRAWBUFFERS`.
/// The caller orders attachments by the lowered GLSL output location, while
/// this record keeps semantic target identity separate from that ordinal. The
/// texture and view are opaque GAL handles; native attachment state stays in
/// the backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FullscreenSourceColorAttachment {
    pub source_slot: u32,
    pub role: TerrainSourceResourceRole,
    pub texture: Handle,
    pub view: Handle,
    pub format: TextureFormat,
    pub clear_each_frame: bool,
    /// Source-declared clear value, if any. A primary target without one uses
    /// the dynamic semantic fog color supplied for the source frame.
    pub clear_color_bits: Option<[u32; 4]>,
}

/// One Rust-owned target selected for a named terrain fragment output. The
/// source slot retains pack-level meaning while `output` keeps the lowered
/// terrain shader's compact output ordering explicit. Neither is a native
/// attachment index or a backend object.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TerrainSourceColorAttachment {
    pub output: TerrainPassOutput,
    pub source_slot: u32,
    pub role: TerrainSourceResourceRole,
    pub texture: Handle,
    pub view: Handle,
    pub format: TextureFormat,
    pub clear_each_frame: bool,
    pub clear_color_bits: Option<[u32; 4]>,
}

/// Per-binding source-color sampling requirements. The plan is intentionally
/// independent of any source stage type: terrain, Distant Horizons, and
/// fullscreen programs all sample the same named color resources through this
/// semantic policy. A missing entry means current-image, base-mip sampling.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ShaderPackColorSamplingPlan {
    bindings: BTreeMap<(TerrainSourceResourceRole, u32), SourceColorBinding>,
}

impl ShaderPackColorSamplingPlan {
    pub(crate) fn from_fullscreen(program: &LoweredFullscreenSourceProgram) -> Self {
        let mut bindings = BTreeMap::new();
        for binding in program.opaque_resource_bindings.bindings() {
            if !matches!(
                binding.kind(),
                TerrainSourceOpaqueResourceKind::CombinedTextureSampler
            ) || !matches!(
                binding.role(),
                TerrainSourceResourceRole::ShaderPackColor(_)
            ) {
                continue;
            }
            bindings.insert(
                (binding.role(), binding.binding()),
                SourceColorBinding {
                    feedback: program.feedback_requirements.iter().any(|requirement| {
                        requirement.role == binding.role()
                            && requirement.sampled_binding == binding.binding()
                    }),
                    mipmapped: program.mipmap_requirements.iter().any(|requirement| {
                        requirement.role == binding.role()
                            && requirement.sampled_binding == binding.binding()
                    }),
                },
            );
        }
        Self { bindings }
    }

    fn binding_for(&self, role: TerrainSourceResourceRole, binding: u32) -> SourceColorBinding {
        self.bindings
            .get(&(role, binding))
            .copied()
            .unwrap_or_default()
    }

    fn validate_for(
        &self,
        opaque_bindings: &TerrainSourceOpaqueResourceBindingPlan,
    ) -> GalResult<()> {
        for ((role, binding), _) in &self.bindings {
            let matches_source_binding = opaque_bindings.bindings().iter().any(|candidate| {
                candidate.binding() == *binding
                    && candidate.role() == *role
                    && candidate.kind() == TerrainSourceOpaqueResourceKind::CombinedTextureSampler
            });
            if !matches_source_binding {
                return Err(GalError::invalid_argument(format!(
                    "source-color sampling policy references absent combined sampler '{}' at binding {}",
                    role.semantic_name(),
                    binding
                )));
            }
        }
        Ok(())
    }
}

/// Rust-owned sampler bindings for the shader-pack color subset of one
/// source stage. These are deliberately separate from target allocation:
/// targets have a source-generation lifetime, while bindings are
/// program-local and may choose the previous feedback image or mip sampling.
/// No Java/Iris sampler, texture unit, framebuffer, or native handle crosses
/// this boundary.
#[derive(Debug)]
pub(crate) struct ShaderPackSourceColorResources {
    resources: TerrainSourceOwnedResourceSet,
    combined_samplers: Vec<Handle>,
    samplers: Vec<Handle>,
    sampled_views: BTreeMap<TerrainSourceResourceRole, Handle>,
}

impl ShaderPackSourceColorResources {
    pub(crate) fn resources(&self) -> &TerrainSourceOwnedResourceSet {
        &self.resources
    }

    /// Diagnostic correlation only: the selected GAL view is still opaque to
    /// callers and backend-native identity never leaves the runtime.
    pub(crate) fn sampled_view_for(&self, role: TerrainSourceResourceRole) -> Option<Handle> {
        self.sampled_views.get(&role).copied()
    }

    pub(crate) fn destroy(self, gal: &mut VulkanicGal) {
        for handle in self.combined_samplers.into_iter().rev() {
            let _ = gal.destroy(handle);
        }
        for handle in self.samplers.into_iter().rev() {
            let _ = gal.destroy(handle);
        }
    }
}

/// Stable semantic identity for one program-local table of Rust-owned named
/// color samplers. It deliberately contains target generations and lowered
/// binding facts, never a texture handle, texture unit, descriptor, or
/// backend-native identity.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ShaderPackSourceColorResourceKey {
    world_generation: u64,
    shader_pack_generation: u64,
    extent: (u32, u32, u32),
    feedback_target_names: Vec<String>,
    mipmapped_target_names: Vec<String>,
    bindings: Vec<(TerrainSourceResourceRole, u32, bool, bool)>,
}

impl ShaderPackSourceColorResourceKey {
    fn new(
        shader_pack_generation: u64,
        opaque_bindings: &TerrainSourceOpaqueResourceBindingPlan,
        sampling: &ShaderPackColorSamplingPlan,
        targets: &ShaderPackColorTargets,
    ) -> GalResult<Self> {
        if shader_pack_generation != targets.identity.shader_pack_generation {
            return Err(GalError::invalid_argument(format!(
                "source program generation {} does not match color target generation {}",
                shader_pack_generation, targets.identity.shader_pack_generation
            )));
        }
        sampling.validate_for(opaque_bindings)?;
        let mut bindings = opaque_bindings
            .bindings()
            .iter()
            .filter(|binding| {
                binding.kind() == TerrainSourceOpaqueResourceKind::CombinedTextureSampler
                    && matches!(
                        binding.role(),
                        TerrainSourceResourceRole::ShaderPackColor(_)
                    )
            })
            .map(|binding| {
                let policy = sampling.binding_for(binding.role(), binding.binding());
                (
                    binding.role(),
                    binding.binding(),
                    policy.feedback,
                    policy.mipmapped,
                )
            })
            .collect::<Vec<_>>();
        bindings.sort();
        Ok(Self {
            world_generation: targets.identity.world_generation,
            shader_pack_generation,
            extent: (
                targets.identity.extent.width,
                targets.identity.extent.height,
                targets.identity.extent.depth,
            ),
            feedback_target_names: targets.identity.feedback_target_names.clone(),
            mipmapped_target_names: targets.identity.mipmapped_target_names.clone(),
            bindings,
        })
    }

    fn matches_targets(&self, targets: &ShaderPackColorTargets) -> bool {
        self.world_generation == targets.identity.world_generation
            && self.shader_pack_generation == targets.identity.shader_pack_generation
            && self.extent
                == (
                    targets.identity.extent.width,
                    targets.identity.extent.height,
                    targets.identity.extent.depth,
                )
            && self.feedback_target_names == targets.identity.feedback_target_names
            && self.mipmapped_target_names == targets.identity.mipmapped_target_names
    }
}

/// Two-phase owner for program-local named-color samplers. Source targets and
/// the combined samplers that reference them must advance together: a source
/// submission either confirms both or discards both. This cache has no route
/// authority and does not expose a backend object outside the GAL.
#[derive(Debug, Default)]
pub(crate) struct ShaderPackSourceColorResourceCache {
    active: BTreeMap<ShaderPackSourceColorResourceKey, ShaderPackSourceColorResources>,
    pending: BTreeMap<ShaderPackSourceColorResourceKey, ShaderPackSourceColorResources>,
}

impl ShaderPackSourceColorResourceCache {
    /// Stages an owned source-color subset and returns a cloned semantic table
    /// suitable for merging into a one-frame source resource snapshot. The
    /// resource handles remain owned by this cache until matching confirmation
    /// or discard; callers never own target lifetime by cloning the table.
    pub(crate) fn stage(
        &mut self,
        gal: &mut VulkanicGal,
        shader_pack_generation: u64,
        opaque_bindings: &TerrainSourceOpaqueResourceBindingPlan,
        sampling: &ShaderPackColorSamplingPlan,
        targets: &ShaderPackColorTargets,
    ) -> GalResult<TerrainSourceOwnedResourceSet> {
        let key = ShaderPackSourceColorResourceKey::new(
            shader_pack_generation,
            opaque_bindings,
            sampling,
            targets,
        )?;
        if let Some(resources) = self.pending.get(&key).or_else(|| self.active.get(&key)) {
            return Ok(resources.resources().clone());
        }
        if self
            .pending
            .keys()
            .any(|pending| !pending.matches_targets(targets))
        {
            return Err(GalError::backend(
                "shader-pack source-color replacement is awaiting confirmation for a different target generation",
            ));
        }
        let resources = prepare_source_color_resources(
            gal,
            shader_pack_generation,
            opaque_bindings,
            sampling,
            targets,
        )?;
        let snapshot = resources.resources().clone();
        self.pending.insert(key, resources);
        Ok(snapshot)
    }

    /// Promotes all program-local bindings prepared for the target generation
    /// used by a successful combined source submission. Older target
    /// generations retire through the GAL before their images are replaced.
    pub(crate) fn confirm_submission(&mut self, gal: &mut VulkanicGal) -> GalResult<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let target_key = self
            .pending
            .keys()
            .next()
            .expect("non-empty pending source-color cache")
            .clone();
        if self
            .pending
            .keys()
            .any(|pending| !pending.matches_targets_key(&target_key))
        {
            return Err(GalError::backend(
                "shader-pack source-color cache cannot confirm mixed target generations",
            ));
        }
        let stale = self
            .active
            .keys()
            .filter(|active| !active.matches_targets_key(&target_key))
            .cloned()
            .collect::<Vec<_>>();
        for key in stale {
            if let Some(resources) = self.active.remove(&key) {
                resources.destroy(gal);
            }
        }
        for (key, resources) in std::mem::take(&mut self.pending) {
            if let Some(previous) = self.active.insert(key, resources) {
                previous.destroy(gal);
            }
        }
        Ok(())
    }

    /// Discards only bindings staged for a failed combined source submission.
    /// Confirmed bindings remain valid for their still-active target generation.
    pub(crate) fn discard_submission(&mut self, gal: &mut VulkanicGal) {
        for (_, resources) in std::mem::take(&mut self.pending) {
            resources.destroy(gal);
        }
    }

    pub(crate) fn destroy(&mut self, gal: &mut VulkanicGal) {
        self.discard_submission(gal);
        for (_, resources) in std::mem::take(&mut self.active) {
            resources.destroy(gal);
        }
    }

    #[cfg(test)]
    fn active_len(&self) -> usize {
        self.active.len()
    }

    #[cfg(test)]
    fn pending_len(&self) -> usize {
        self.pending.len()
    }
}

impl ShaderPackSourceColorResourceKey {
    fn matches_targets_key(&self, other: &Self) -> bool {
        self.world_generation == other.world_generation
            && self.shader_pack_generation == other.shader_pack_generation
            && self.extent == other.extent
            && self.feedback_target_names == other.feedback_target_names
            && self.mipmapped_target_names == other.mipmapped_target_names
    }
}

/// Resolves only source-declared shader-pack color samplers into owned GAL
/// resources. Other semantic roles (material atlas, depth, custom textures,
/// storage images, and so on) are deliberately left for their own complete
/// owned resource paths; the caller merges those subsets and rejects missing
/// roles before pass creation.
///
/// The sampling policy follows the portable pack-color protocol: render
/// targets are linearly sampled with clamp-to-edge addressing. A source-local
/// `*MipmapEnabled` directive upgrades only that role to linear mip sampling,
/// after target staging has allocated the requested mip chain. This derives
/// from source semantics, not from a borrowed renderer sampler object.
pub(crate) fn prepare_source_color_resources(
    gal: &mut VulkanicGal,
    shader_pack_generation: u64,
    opaque_bindings: &TerrainSourceOpaqueResourceBindingPlan,
    sampling: &ShaderPackColorSamplingPlan,
    targets: &ShaderPackColorTargets,
) -> GalResult<ShaderPackSourceColorResources> {
    if shader_pack_generation != targets.identity.shader_pack_generation {
        return Err(GalError::invalid_argument(format!(
            "source program generation {} does not match color target generation {}",
            shader_pack_generation, targets.identity.shader_pack_generation
        )));
    }
    sampling.validate_for(opaque_bindings)?;

    let mut requirements = BTreeMap::<TerrainSourceResourceRole, SourceColorBinding>::new();
    for binding in opaque_bindings.bindings() {
        if !matches!(
            binding.kind(),
            TerrainSourceOpaqueResourceKind::CombinedTextureSampler
        ) {
            // Storage images belong to an independently owned semantic
            // subset. This helper has no authority to create or bind them.
            continue;
        }
        let role = binding.role();
        if !matches!(role, TerrainSourceResourceRole::ShaderPackColor(_)) {
            // Depth, material, shadow, volume, and copied pack assets are
            // prepared by their own owners, then merged by the fullscreen
            // source pass contract. Rejecting them here made mixed semantic
            // passes impossible to admit without a borrowed renderer state.
            continue;
        }
        let candidate = sampling.binding_for(role.clone(), binding.binding());
        if let Some(existing) = requirements.insert(role.clone(), candidate) {
            if existing != candidate {
                return Err(GalError::unsupported_feature(format!(
                    "fullscreen source aliases semantic color '{}' with incompatible feedback or mipmap behavior",
                    role.semantic_name()
                )));
            }
        }
    }

    let mut created = Vec::new();
    let result = (|| -> GalResult<ShaderPackSourceColorResources> {
        let mut availability = Vec::with_capacity(requirements.len());
        let mut resources = Vec::with_capacity(requirements.len());
        let mut combined_samplers = Vec::with_capacity(requirements.len());
        let mut samplers = Vec::with_capacity(requirements.len());
        let mut sampled_views = BTreeMap::new();
        for (role, binding) in requirements {
            let name = role.shader_pack_color_name().expect("validated color role");
            let target = targets.target(name).ok_or_else(|| {
                GalError::invalid_argument(format!(
                    "fullscreen source semantic color target '{name}' is unavailable"
                ))
            })?;
            let view = if binding.feedback {
                target.previous_view.ok_or_else(|| {
                    GalError::invalid_argument(format!(
                        "fullscreen source target '{name}' samples its own output but has no previous feedback view"
                    ))
                })?
            } else {
                target.current_view
            };
            if binding.mipmapped && target.mip_levels < 2 {
                return Err(GalError::invalid_argument(format!(
                    "fullscreen source target '{name}' requests mip sampling but has no staged mip chain"
                )));
            }
            let sampler = gal.create_sampler(
                ShaderPackColorSamplingPolicy {
                    mipmapped: binding.mipmapped,
                }
                .descriptor(&format!(
                    "shader-pack-color.world{}-pack{}.{}.{}-sampler",
                    targets.identity.world_generation,
                    targets.identity.shader_pack_generation,
                    name,
                    if binding.feedback {
                        "previous"
                    } else {
                        "current"
                    }
                )),
            )?;
            created.push(sampler);
            samplers.push(sampler);
            let combined = gal.create_combined_texture_sampler(CombinedTextureSamplerDesc {
                label: format!(
                    "shader-pack-color.world{}-pack{}.{}.{}-combined",
                    targets.identity.world_generation,
                    targets.identity.shader_pack_generation,
                    name,
                    if binding.feedback {
                        "previous"
                    } else {
                        "current"
                    }
                ),
                texture_view: view,
                sampler,
            })?;
            created.push(combined);
            combined_samplers.push(combined);
            availability.push(TerrainSourceResourceAvailability {
                role: role.clone(),
                shape: role.expected_sampled_resource_shape(),
                resource_generation: color_resource_generation(&targets.identity, name, binding),
            });
            resources.push(TerrainSourceOwnedResource {
                role: role.clone(),
                combined_sampler: combined,
            });
            sampled_views.insert(role, view);
        }
        Ok(ShaderPackSourceColorResources {
            resources: TerrainSourceOwnedResourceSet::new(
                TerrainSourceResourceAvailabilitySet::new(
                    targets.identity.shader_pack_generation,
                    targets.identity.world_generation,
                    availability,
                )?,
                resources,
            )?,
            combined_samplers,
            samplers,
            sampled_views,
        })
    })();
    if result.is_err() {
        for handle in created.into_iter().rev() {
            let _ = gal.destroy(handle);
        }
    }
    result
}

/// Fullscreen stages derive feedback and mip policy from their own lowered
/// source. Keep this focused wrapper so existing fullscreen preparation stays
/// compact while terrain and DH use the generic allocator above.
pub(crate) fn prepare_fullscreen_source_color_resources(
    gal: &mut VulkanicGal,
    program: &LoweredFullscreenSourceProgram,
    targets: &ShaderPackColorTargets,
) -> GalResult<ShaderPackSourceColorResources> {
    prepare_source_color_resources(
        gal,
        program.shader_pack_generation,
        &program.opaque_resource_bindings,
        &ShaderPackColorSamplingPlan::from_fullscreen(program),
        targets,
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SourceColorBinding {
    feedback: bool,
    mipmapped: bool,
}

impl Default for SourceColorBinding {
    fn default() -> Self {
        Self {
            feedback: false,
            mipmapped: false,
        }
    }
}

/// Resolves the output side of a lowered fullscreen stage through the pack
/// manifest and the Rust-owned color target generation. This is the missing
/// semantic bridge between source `DRAWBUFFERS` locations and a future GAL
/// render target: locations are checked against the manifest once, while the
/// executor receives only named roles and opaque GAL views.
///
/// The helper intentionally does not create a pass, pipeline, framebuffer, or
/// command. In particular, it cannot select a shader-pack route by itself.
pub(crate) fn resolve_fullscreen_source_color_attachments(
    program: &LoweredFullscreenSourceProgram,
    manifest: &ShaderPackColorTargetManifest,
    targets: &ShaderPackColorTargets,
) -> GalResult<Vec<FullscreenSourceColorAttachment>> {
    if program.shader_pack_generation != manifest.generation()
        || program.shader_pack_generation != targets.identity.shader_pack_generation
    {
        return Err(GalError::invalid_argument(
            "fullscreen source program, color manifest, and staged color targets must share one shader-pack generation",
        ));
    }

    let mut attachments = Vec::with_capacity(program.outputs.len());
    let mut locations = std::collections::BTreeSet::new();
    let mut roles = std::collections::BTreeSet::new();
    for output in &program.outputs {
        let location = output.source_location();
        let role = output.role();
        if !locations.insert(location) {
            return Err(GalError::invalid_argument(format!(
                "fullscreen source program '{}' writes source location {location} more than once",
                program.identity.as_str()
            )));
        }
        if !roles.insert(role.clone()) {
            return Err(GalError::invalid_argument(format!(
                "fullscreen source program '{}' writes semantic color '{}' more than once",
                program.identity.as_str(),
                role.semantic_name()
            )));
        }
        let name = role.shader_pack_color_name().ok_or_else(|| {
            GalError::invalid_argument(format!(
                "fullscreen source output '{}' is not a shader-pack color target",
                output.semantic_name()
            ))
        })?;
        let source_slot = output.source_slot();
        let declaration = manifest.target_for_source_slot(source_slot).ok_or_else(|| {
            GalError::invalid_argument(format!(
                "fullscreen source output location {location} maps to missing shader-pack color slot {source_slot}"
            ))
        })?;
        if declaration.role != role {
            return Err(GalError::invalid_argument(format!(
                "fullscreen source output location {location} maps to semantic color '{}' but program writes '{}'",
                declaration.name(),
                role.semantic_name()
            )));
        }
        let target = targets.target(name).ok_or_else(|| {
            GalError::invalid_argument(format!(
                "fullscreen source output semantic color '{name}' has no staged Rust-owned target"
            ))
        })?;
        let expected_format = declaration.gal_schema_color_format();
        if !declaration.accepts_storage_format(target.format) {
            return Err(GalError::invalid_argument(format!(
                "fullscreen source output semantic color '{name}' staged storage format {:?} is incompatible with manifest {:?}",
                target.format, expected_format
            )));
        }
        attachments.push(FullscreenSourceColorAttachment {
            source_slot,
            role,
            texture: target.current_texture,
            view: target.current_attachment_view,
            format: target.format,
            clear_each_frame: target.clear_each_frame,
            clear_color_bits: target.clear_color_bits,
        });
    }
    attachments.sort_by_key(|attachment| {
        program
            .outputs
            .iter()
            .find(|output| output.role() == attachment.role)
            .map(FullscreenSourceFragmentOutput::source_location)
            .expect("resolved fullscreen attachment must originate from program output")
    });
    if attachments.is_empty() {
        return Err(GalError::invalid_argument(
            "fullscreen source program has no resolved color attachments",
        ));
    }
    Ok(attachments)
}

/// Resolves the source-derived normal-terrain output schema against one
/// staged Rust-owned shader-pack color generation. The caller supplies the
/// mapping retained by the lowered program rather than raw `DRAWBUFFERS`
/// text, which makes missing, duplicate, or stale outputs fail before a GAL
/// pass can be constructed.
pub(crate) fn resolve_terrain_source_color_attachments(
    output_color_slots: &[(TerrainPassOutput, u32)],
    manifest: &ShaderPackColorTargetManifest,
    targets: &ShaderPackColorTargets,
) -> GalResult<Vec<TerrainSourceColorAttachment>> {
    if manifest.generation() != targets.identity.shader_pack_generation {
        return Err(GalError::invalid_argument(
            "terrain source color manifest and staged targets must share one shader-pack generation",
        ));
    }
    if output_color_slots.is_empty() {
        return Err(GalError::invalid_argument(
            "terrain source program has no named color outputs to resolve",
        ));
    }
    let mut outputs = std::collections::BTreeSet::new();
    let mut source_slots = std::collections::BTreeSet::new();
    let mut roles = std::collections::BTreeSet::new();
    let mut attachments = Vec::with_capacity(output_color_slots.len());
    for &(output, source_slot) in output_color_slots {
        if !outputs.insert(output) {
            return Err(GalError::invalid_argument(format!(
                "terrain source program maps '{}' more than once",
                output.semantic_name()
            )));
        }
        if !source_slots.insert(source_slot) {
            return Err(GalError::invalid_argument(format!(
                "terrain source program maps more than one semantic output to shader-pack color slot {source_slot}"
            )));
        }
        let declaration = manifest
            .target_for_source_slot(source_slot)
            .ok_or_else(|| {
                GalError::invalid_argument(format!(
                "terrain source output '{}' maps to missing shader-pack color slot {source_slot}",
                output.semantic_name()
            ))
            })?;
        let name = declaration.name();
        if !roles.insert(declaration.role.clone()) {
            return Err(GalError::invalid_argument(format!(
                "terrain source output '{}' aliases semantic shader-pack color '{name}'",
                output.semantic_name()
            )));
        }
        let target = targets.target(name).ok_or_else(|| {
            GalError::invalid_argument(format!(
                "terrain source output '{}' has no staged Rust-owned target for semantic color '{name}'",
                output.semantic_name()
            ))
        })?;
        let format = declaration.gal_schema_color_format();
        if !declaration.accepts_storage_format(target.format) {
            return Err(GalError::invalid_argument(format!(
                "terrain source output '{}' staged storage format {:?} is incompatible with source manifest {:?}",
                output.semantic_name(),
                target.format,
                format
            )));
        }
        attachments.push(TerrainSourceColorAttachment {
            output,
            source_slot,
            role: declaration.role.clone(),
            texture: target.current_texture,
            view: target.current_attachment_view,
            format: target.format,
            clear_each_frame: target.clear_each_frame,
            clear_color_bits: target.clear_color_bits,
        });
    }
    attachments.sort_by_key(|attachment| attachment.output);
    Ok(attachments)
}

/// Returns all staged semantic colors in exact source-slot order. A future
/// fullscreen pass uses this to construct its complete backend-neutral target
/// and pipeline color-format vector, including source slots the particular
/// shader leaves unwritten. No slot is synthesized from a backend default.
pub(crate) fn source_color_attachments_by_slot(
    manifest: &ShaderPackColorTargetManifest,
    targets: &ShaderPackColorTargets,
) -> GalResult<Vec<FullscreenSourceColorAttachment>> {
    if manifest.generation() != targets.identity.shader_pack_generation {
        return Err(GalError::invalid_argument(
            "shader-pack color manifest and staged color targets must share one generation",
        ));
    }
    let mut attachments = Vec::with_capacity(MAX_SOURCE_COLOR_TARGETS as usize);
    for slot in 0..MAX_SOURCE_COLOR_TARGETS {
        let declaration = manifest.target_for_source_slot(slot).ok_or_else(|| {
            GalError::invalid_argument(format!(
                "shader-pack color manifest lacks source slot {slot}"
            ))
        })?;
        let name = declaration.name();
        let target = targets.target(name).ok_or_else(|| {
            GalError::invalid_argument(format!(
                "shader-pack color source slot {slot} ('{name}') has no staged Rust-owned target"
            ))
        })?;
        let format = declaration.gal_schema_color_format();
        if !declaration.accepts_storage_format(target.format) {
            return Err(GalError::invalid_argument(format!(
                "shader-pack color source slot {slot} ('{name}') staged storage format {:?} is incompatible with manifest {:?}",
                target.format, format
            )));
        }
        attachments.push(FullscreenSourceColorAttachment {
            source_slot: slot,
            role: declaration.role.clone(),
            texture: target.current_texture,
            view: target.current_attachment_view,
            format: target.format,
            clear_each_frame: target.clear_each_frame,
            clear_color_bits: target.clear_color_bits,
        });
    }
    Ok(attachments)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ShaderPackColorSamplingPolicy {
    mipmapped: bool,
}

impl ShaderPackColorSamplingPolicy {
    fn descriptor(self, label: &str) -> SamplerDesc {
        SamplerDesc {
            label: label.to_string(),
            min_filter: SamplerFilter::Linear,
            mag_filter: SamplerFilter::Linear,
            mip_filter: if self.mipmapped {
                SamplerFilter::Linear
            } else {
                SamplerFilter::Nearest
            },
            address_u: SamplerAddressMode::ClampToEdge,
            address_v: SamplerAddressMode::ClampToEdge,
            address_w: SamplerAddressMode::ClampToEdge,
            comparison: None,
        }
    }
}

fn color_resource_generation(
    identity: &ShaderPackColorTargetIdentity,
    name: &str,
    binding: SourceColorBinding,
) -> u64 {
    let identity_text = format!(
        "{}:{}:{}x{}:{}:{}:{}:{}:{}",
        identity.world_generation,
        identity.shader_pack_generation,
        identity.extent.width,
        identity.extent.height,
        name,
        binding.feedback,
        binding.mipmapped,
        identity.feedback_target_names.join(","),
        identity.mipmapped_target_names.join(","),
    );
    u64::from(xxh32(identity_text.as_bytes(), 0)).max(1)
}

impl ShaderPackColorTargets {
    pub(crate) fn target(&self, name: &str) -> Option<ShaderPackColorTarget> {
        self.targets.get(name).copied()
    }

    pub(crate) fn targets(&self) -> impl Iterator<Item = (&str, ShaderPackColorTarget)> {
        self.targets
            .iter()
            .map(|(name, target)| (name.as_str(), *target))
    }

    /// Semantic roles backed by this complete Rust-owned target generation.
    /// These are source-graph outputs, not a promise that any particular
    /// stage has already created sampler bindings for them. Callers use this
    /// only when validating complete graph residency; program-local bindings
    /// continue to be staged and checked separately at execution time.
    pub(crate) fn declared_roles(&self) -> impl Iterator<Item = TerrainSourceResourceRole> + '_ {
        self.targets
            .keys()
            .cloned()
            .map(TerrainSourceResourceRole::ShaderPackColor)
    }

    fn create(
        gal: &mut VulkanicGal,
        identity: ShaderPackColorTargetIdentity,
        manifest: &ShaderPackColorTargetManifest,
    ) -> GalResult<Self> {
        identity.validate()?;
        if manifest.generation() != identity.shader_pack_generation {
            return Err(GalError::invalid_argument(format!(
                "shader-pack color target manifest generation {} does not match identity generation {}",
                manifest.generation(),
                identity.shader_pack_generation
            )));
        }
        manifest.require_gal_schema_formats()?;
        for feedback_name in &identity.feedback_target_names {
            if manifest.target(feedback_name).is_none() {
                return Err(GalError::invalid_argument(format!(
                    "source feedback target '{feedback_name}' is absent from the shader-pack color manifest"
                )));
            }
        }
        for mipmapped_name in &identity.mipmapped_target_names {
            if manifest.target(mipmapped_name).is_none() {
                return Err(GalError::invalid_argument(format!(
                    "source mipmapped target '{mipmapped_name}' is absent from the shader-pack color manifest"
                )));
            }
        }

        let mut created = Vec::new();
        let result = (|| -> GalResult<Self> {
            let mut targets = BTreeMap::new();
            for declaration in manifest.targets() {
                let name = declaration.name();
                let _ = declaration.gal_schema_color_format();
                let mip_levels = if identity
                    .mipmapped_target_names
                    .binary_search_by(|candidate| candidate.as_str().cmp(name))
                    .is_ok()
                {
                    full_mip_levels(identity.extent)
                } else {
                    1
                };
                let (format, current_texture) = create_compatible_color_texture(
                    gal,
                    &identity,
                    name,
                    declaration.format,
                    mip_levels,
                    "current",
                    &mut created,
                )?;
                let current_view = create_color_view(
                    gal,
                    &identity,
                    name,
                    format,
                    mip_levels,
                    "current",
                    current_texture,
                    &mut created,
                )?;
                let current_attachment_view = if mip_levels > 1 {
                    create_color_view(
                        gal,
                        &identity,
                        name,
                        format,
                        1,
                        "current-attachment",
                        current_texture,
                        &mut created,
                    )?
                } else {
                    current_view
                };
                let (previous_texture, previous_attachment_view, previous_view) = if identity
                    .feedback_target_names
                    .binary_search_by(|candidate| candidate.as_str().cmp(name))
                    .is_ok()
                {
                    let (previous_format, texture) = create_compatible_color_texture(
                        gal,
                        &identity,
                        name,
                        declaration.format,
                        mip_levels,
                        "previous",
                        &mut created,
                    )?;
                    if previous_format != format {
                        return Err(GalError::backend(format!(
                            "shader-pack color target '{name}' selected inconsistent current {:?} and previous {:?} storage formats",
                            format, previous_format
                        )));
                    }
                    let view = create_color_view(
                        gal,
                        &identity,
                        name,
                        format,
                        mip_levels,
                        "previous",
                        texture,
                        &mut created,
                    )?;
                    let attachment_view = if mip_levels > 1 {
                        Some(create_color_view(
                            gal,
                            &identity,
                            name,
                            format,
                            1,
                            "previous-attachment",
                            texture,
                            &mut created,
                        )?)
                    } else {
                        None
                    };
                    (Some(texture), attachment_view, Some(view))
                } else {
                    (None, None, None)
                };
                if targets
                    .insert(
                        name.to_string(),
                        ShaderPackColorTarget {
                            source_slot: declaration.source_slot,
                            format,
                            clear_each_frame: declaration.clear_each_frame,
                            clear_color_bits: declaration.clear_color_bits,
                            mip_levels,
                            current_texture,
                            current_attachment_view,
                            current_view,
                            previous_texture,
                            previous_attachment_view,
                            previous_view,
                        },
                    )
                    .is_some()
                {
                    return Err(GalError::invalid_argument(format!(
                        "shader-pack color manifest repeats semantic target '{name}'"
                    )));
                }
            }
            Ok(Self { identity, targets })
        })();
        if result.is_err() {
            for handle in created.into_iter().rev() {
                let _ = gal.destroy(handle);
            }
        }
        result
    }

    fn destroy(self, gal: &mut VulkanicGal) {
        for target in self.targets.into_values() {
            for handle in [
                target.previous_attachment_view,
                target.previous_view,
                (target.current_attachment_view != target.current_view)
                    .then_some(target.current_attachment_view),
                target.current_view.into(),
                target.previous_texture,
                target.current_texture.into(),
            ]
            .into_iter()
            .flatten()
            {
                let _ = gal.destroy(handle);
            }
        }
    }
}

fn create_compatible_color_texture(
    gal: &mut VulkanicGal,
    identity: &ShaderPackColorTargetIdentity,
    name: &str,
    declared_format: ShaderPackColorFormat,
    mip_levels: u32,
    phase: &str,
    created: &mut Vec<Handle>,
) -> GalResult<(TextureFormat, Handle)> {
    let candidates = declared_format.compatible_storage_formats();
    for (index, format) in candidates.iter().copied().enumerate() {
        match create_color_texture(gal, identity, name, format, mip_levels, phase, created) {
            Ok(texture) => return Ok((format, texture)),
            Err(error)
                if error.code == StatusCode::UnsupportedFeature && index + 1 < candidates.len() =>
            {
                continue;
            }
            Err(error) => return Err(error),
        }
    }
    Err(GalError::unsupported_feature(format!(
        "shader-pack color target '{name}' has no supported Rust-owned storage format for {declared_format:?}"
    )))
}

fn create_color_texture(
    gal: &mut VulkanicGal,
    identity: &ShaderPackColorTargetIdentity,
    name: &str,
    format: TextureFormat,
    mip_levels: u32,
    phase: &str,
    created: &mut Vec<Handle>,
) -> GalResult<Handle> {
    let handle = gal.create_texture(TextureDesc {
        label: format!(
            "shader-pack-color.world{}-pack{}.{}.{}.texture",
            identity.world_generation, identity.shader_pack_generation, name, phase
        ),
        dimension: TextureDimension::D2,
        format,
        extent: identity.extent,
        mip_levels,
        array_layers: 1,
        usages: vec![
            TextureUsage::ColorAttachment,
            TextureUsage::Sampled,
            TextureUsage::TransferSrc,
            TextureUsage::TransferDst,
        ],
    })?;
    created.push(handle);
    Ok(handle)
}

fn create_color_view(
    gal: &mut VulkanicGal,
    identity: &ShaderPackColorTargetIdentity,
    name: &str,
    format: TextureFormat,
    mip_levels: u32,
    phase: &str,
    texture: Handle,
    created: &mut Vec<Handle>,
) -> GalResult<Handle> {
    let handle = gal.create_texture_view(TextureViewDesc {
        label: format!(
            "shader-pack-color.world{}-pack{}.{}.{}.view",
            identity.world_generation, identity.shader_pack_generation, name, phase
        ),
        texture,
        format,
        base_mip: 0,
        mip_count: mip_levels,
        base_layer: 0,
        layer_count: 1,
    })?;
    created.push(handle);
    Ok(handle)
}

fn full_mip_levels(extent: Extent3d) -> u32 {
    let largest_dimension = extent.width.max(extent.height);
    u32::BITS - largest_dimension.leading_zeros()
}

/// Submit-confirmed semantic state for one private shader-pack color target.
/// This contains no image views or native identities: it answers only whether
/// the current and optional previous images have meaningful source-owned
/// contents for the target generation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ShaderPackColorFrameTargetState {
    current_initialized: bool,
    previous_initialized: bool,
    mipmaps_initialized: bool,
    previous_mipmaps_initialized: bool,
    /// Distinguishes an earlier writer in this exact source schedule from a
    /// value merely retained by a confirmed older frame. A self-feedback pass
    /// must snapshot the former before it overwrites the current image, while
    /// the latter remains valid temporal history.
    current_written_this_frame: bool,
}

/// Semantic values needed for the source-pack's one-time full clear. Fog is
/// copied gameplay/environment data; the remaining defaults follow the
/// portable pack target protocol (slot one is depth history, all other
/// non-primary targets begin transparent black). No Iris framebuffer, target,
/// or render-state object participates in this contract.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ShaderPackColorBootstrapClearValues {
    pub fog_color: ClearColor,
}

#[derive(Debug)]
struct ShaderPackColorBootstrapTarget {
    name: String,
    previous: bool,
    texture: Handle,
    view: Handle,
    render_target: Handle,
    render_pass: Handle,
    clear_color: ClearColor,
}

/// One bounded, Rust-owned initialization transaction for all current and
/// feedback sides of a named source color generation. The transient passes
/// exist only to perform explicit clears through GAL; target images retain
/// their usual generation lifetime in `ShaderPackColorTargetCache`.
#[derive(Debug)]
pub(crate) struct ShaderPackColorBootstrapPlan {
    targets: Vec<ShaderPackColorBootstrapTarget>,
}

/// The last successfully submitted color-history state for one exact source
/// target identity. A replacement extent, world, or pack generation starts
/// uninitialized rather than inheriting images across an incompatible route.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ShaderPackColorFrameState {
    identity: ShaderPackColorTargetIdentity,
    targets: BTreeMap<String, ShaderPackColorFrameTargetState>,
}

/// One in-progress source-frame color schedule. It is intentionally detached
/// from submission confirmation: a rejected command list cannot make a color
/// target or feedback image semantically valid.
#[derive(Clone, Debug)]
pub(crate) struct ShaderPackColorFramePlan {
    identity: ShaderPackColorTargetIdentity,
    targets: BTreeMap<String, ShaderPackColorFrameTargetState>,
}

impl ShaderPackColorFramePlan {
    pub(crate) fn begin(
        targets: &ShaderPackColorTargets,
        prior: Option<&ShaderPackColorFrameState>,
    ) -> GalResult<Self> {
        if let Some(prior) = prior {
            if prior.identity != targets.identity {
                return Err(GalError::invalid_argument(
                    "shader-pack color-frame state belongs to a different target generation",
                ));
            }
        }
        let mut frame_targets = BTreeMap::new();
        for (name, target) in targets.targets() {
            let prior_state = prior
                .and_then(|state| state.targets.get(name).copied())
                .unwrap_or_default();
            frame_targets.insert(
                name.to_string(),
                ShaderPackColorFrameTargetState {
                    current_initialized: prior_state.current_initialized,
                    previous_initialized: if target.previous_view.is_some() {
                        prior_state.previous_initialized
                    } else {
                        false
                    },
                    mipmaps_initialized: if target.mip_levels > 1 {
                        prior_state.mipmaps_initialized
                    } else {
                        false
                    },
                    previous_mipmaps_initialized: if target.previous_view.is_some()
                        && target.mip_levels > 1
                    {
                        prior_state.previous_mipmaps_initialized
                    } else {
                        false
                    },
                    current_written_this_frame: false,
                },
            );
        }
        Ok(Self {
            identity: targets.identity.clone(),
            targets: frame_targets,
        })
    }

    /// A source-generation bootstrap clears every current/feedback image
    /// exactly once. Later frames preserve the submit-confirmed target state
    /// and let their declared passes apply the per-target clear/load policy.
    /// A partially initialized generation is never safe to guess about.
    pub(crate) fn requires_initial_clear(&self) -> GalResult<bool> {
        let initialized = self
            .targets
            .values()
            .map(|state| state.current_initialized)
            .collect::<Vec<_>>();
        if initialized.iter().all(|initialized| !initialized) {
            if self.targets.values().any(|state| {
                state.previous_initialized
                    || state.mipmaps_initialized
                    || state.previous_mipmaps_initialized
            }) {
                return Err(GalError::invalid_argument(
                    "shader-pack color generation has inconsistent partial initialization",
                ));
            }
            return Ok(true);
        }
        if initialized.iter().all(|initialized| *initialized) {
            return Ok(false);
        }
        Err(GalError::invalid_argument(
            "shader-pack color generation has partially initialized current targets",
        ))
    }

    /// Creates the initial full-clear commands for a fresh source target
    /// generation. This is intentionally explicit rather than treating an
    /// allocation as initialized data. Callers record, submit, destroy the
    /// transient plan, then confirm this frame only when that submission
    /// succeeds.
    pub(crate) fn stage_full_clear(
        &self,
        gal: &mut VulkanicGal,
        targets: &ShaderPackColorTargets,
        values: ShaderPackColorBootstrapClearValues,
    ) -> GalResult<ShaderPackColorBootstrapPlan> {
        self.require_targets(targets)?;
        if self.targets.values().any(|state| {
            state.current_initialized
                || state.previous_initialized
                || state.mipmaps_initialized
                || state.previous_mipmaps_initialized
        }) {
            return Err(GalError::invalid_argument(
                "shader-pack color full clear is only valid for an uninitialized target generation",
            ));
        }
        ShaderPackColorBootstrapPlan::stage(gal, targets, values)
    }

    /// Records the full clear and makes the initialized state visible only in
    /// this in-progress plan. `confirm_frame_submission` remains the sole
    /// way to expose it to another frame.
    pub(crate) fn append_full_clear(
        &mut self,
        bootstrap: &ShaderPackColorBootstrapPlan,
        operations: &mut Vec<CommandOp>,
    ) -> GalResult<()> {
        if self.targets.values().any(|state| {
            state.current_initialized
                || state.previous_initialized
                || state.mipmaps_initialized
                || state.previous_mipmaps_initialized
        }) {
            return Err(GalError::invalid_argument(
                "shader-pack color full clear cannot overwrite an already initialized frame plan",
            ));
        }
        bootstrap.append(operations);
        for state in self.targets.values_mut() {
            state.current_initialized = true;
            state.mipmaps_initialized = false;
            state.previous_mipmaps_initialized = false;
            state.current_written_this_frame = true;
            // A target without a previous image leaves this false. The
            // bootstrap plan contains a previous-side clear exactly for
            // declared feedback targets.
        }
        for target in &bootstrap.targets {
            if target.previous {
                self.targets
                    .get_mut(&target.name)
                    .expect("bootstrap target name derives from frame target")
                    .previous_initialized = true;
            }
        }
        Ok(())
    }

    /// Returns the prior states for a complete source render target in source
    /// slot order. The caller still owns pass load/store selection, while this
    /// plan guarantees the state originates from semantic frame history.
    pub(crate) fn attachment_states(
        &self,
        attachments: &[FullscreenSourceColorAttachment],
    ) -> GalResult<Vec<TextureUsageState>> {
        attachments
            .iter()
            .map(|attachment| {
                let name = attachment.role.shader_pack_color_name().ok_or_else(|| {
                    GalError::invalid_argument(
                        "shader-pack color frame attachment is not a named color role",
                    )
                })?;
                let state = self.targets.get(name).ok_or_else(|| {
                    GalError::invalid_argument(format!(
                        "shader-pack color frame has no target state for '{name}'"
                    ))
                })?;
                Ok(if state.current_initialized {
                    TextureUsageState::ShaderRead
                } else {
                    TextureUsageState::Undefined
                })
            })
            .collect()
    }

    /// A source program may sample the current image only after an earlier
    /// pass in this exact frame, or a confirmed previous frame, initialized it.
    /// Feedback samples have their own prior image and never alias current.
    pub(crate) fn require_sample(
        &self,
        role: &TerrainSourceResourceRole,
        feedback: bool,
    ) -> GalResult<()> {
        self.require_sample_with_mips(role, feedback, false)
    }

    /// Mipmapped sampling is a separate semantic dependency: allocating a
    /// chain does not initialize its descendants. A source frame must record
    /// an explicit `GenerateMipmaps` operation after the most recent write.
    pub(crate) fn require_sample_with_mips(
        &self,
        role: &TerrainSourceResourceRole,
        feedback: bool,
        mipmapped: bool,
    ) -> GalResult<()> {
        let name = role.shader_pack_color_name().ok_or_else(|| {
            GalError::invalid_argument(
                "shader-pack color sample is not a named semantic color role",
            )
        })?;
        let state = self.targets.get(name).ok_or_else(|| {
            GalError::invalid_argument(format!(
                "shader-pack color frame has no target state for sampled '{name}'"
            ))
        })?;
        let initialized = if feedback {
            state.previous_initialized
        } else {
            state.current_initialized
        };
        if initialized {
            if mipmapped
                && !(if feedback {
                    state.previous_mipmaps_initialized
                } else {
                    state.mipmaps_initialized
                })
            {
                return Err(GalError::invalid_argument(format!(
                    "shader-pack color target '{name}' is sampled with mips before an explicit source-owned mip generation"
                )));
            }
            return Ok(());
        }
        Err(GalError::invalid_argument(format!(
            "shader-pack color target '{name}' is sampled as {} before a confirmed source-owned initialization",
            if feedback { "feedback history" } else { "current-frame data" }
        )))
    }

    /// Records semantic availability after one pass. Every cleared attachment
    /// is initialized by its clear operation; every declared output is
    /// initialized by the fragment program. Other non-clearing attachments
    /// retain exactly their prior status.
    pub(crate) fn record_pass(
        &mut self,
        attachments: &[FullscreenSourceColorAttachment],
        outputs: &[FullscreenSourceColorAttachment],
    ) -> GalResult<()> {
        for attachment in attachments {
            if attachment.clear_each_frame {
                let state = self.target_state_mut(&attachment.role)?;
                state.current_initialized = true;
                state.mipmaps_initialized = false;
                state.current_written_this_frame = true;
            }
        }
        for output in outputs {
            let state = self.target_state_mut(&output.role)?;
            state.current_initialized = true;
            state.mipmaps_initialized = false;
            state.current_written_this_frame = true;
        }
        Ok(())
    }

    /// Records outputs from an earlier non-fullscreen source pass such as
    /// terrain opaque/cutout. It has the same submit-confirmed ownership as
    /// fullscreen outputs and accepts only declared named color roles.
    pub(crate) fn record_external_outputs(
        &mut self,
        outputs: &[TerrainSourceResourceRole],
    ) -> GalResult<()> {
        for role in outputs {
            let state = self.target_state_mut(role)?;
            state.current_initialized = true;
            state.mipmaps_initialized = false;
            state.current_written_this_frame = true;
        }
        Ok(())
    }

    /// Makes same-frame writes available to a later self-feedback source
    /// stage. The source contract identifies feedback when a stage samples and
    /// writes one named color role; Vulkan and OpenGL cannot bind one image for
    /// both, so the scheduler snapshots only a value actually written earlier
    /// in this frame. Unwritten roles keep their confirmed previous-frame
    /// history for temporal stages.
    pub(crate) fn append_same_frame_feedback_snapshots(
        &mut self,
        targets: &ShaderPackColorTargets,
        roles: &[TerrainSourceResourceRole],
        operations: &mut Vec<CommandOp>,
    ) -> GalResult<Vec<TerrainSourceResourceRole>> {
        self.require_targets(targets)?;
        let mut names = roles
            .iter()
            .map(|role| {
                role.shader_pack_color_name()
                    .map(str::to_string)
                    .ok_or_else(|| {
                        GalError::invalid_argument(
                            "shader-pack feedback snapshot role is not a named color target",
                        )
                    })
            })
            .collect::<GalResult<Vec<_>>>()?;
        names.sort();
        names.dedup();

        let mut copied = Vec::new();
        for name in names {
            let target = targets.target(&name).ok_or_else(|| {
                GalError::invalid_argument(format!(
                    "shader-pack feedback snapshot target '{name}' is unavailable"
                ))
            })?;
            let previous_texture = target.previous_texture.ok_or_else(|| {
                GalError::invalid_argument(format!(
                    "shader-pack source stage declared self-feedback for '{name}' without a staged feedback image"
                ))
            })?;
            let state = self.targets.get(&name).ok_or_else(|| {
                GalError::invalid_argument(format!(
                    "shader-pack color frame has no feedback state for '{name}'"
                ))
            })?;
            if !state.current_written_this_frame {
                continue;
            }
            operations.push(CommandOp::Barrier(texture_barrier(
                target.current_texture,
                TextureUsageState::ShaderRead,
                TextureUsageState::TransferSrc,
            )));
            operations.push(CommandOp::Barrier(texture_barrier(
                previous_texture,
                if state.previous_initialized {
                    TextureUsageState::ShaderRead
                } else {
                    TextureUsageState::Undefined
                },
                TextureUsageState::TransferDst,
            )));
            operations.push(CommandOp::CopyTexture(TextureImageCopyRegion {
                row_order: crate::render::vulkanic::commands::TextureRowOrder::Preserve,
                src_texture: target.current_texture,
                src_mip: 0,
                src_layer: 0,
                src_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
                dst_texture: previous_texture,
                dst_mip: 0,
                dst_layer: 0,
                dst_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
                extent: self.identity.extent,
            }));
            operations.push(CommandOp::Barrier(texture_barrier(
                previous_texture,
                TextureUsageState::TransferDst,
                TextureUsageState::ShaderRead,
            )));
            operations.push(CommandOp::Barrier(texture_barrier(
                target.current_texture,
                TextureUsageState::TransferSrc,
                TextureUsageState::ShaderRead,
            )));
            let state = self.targets.get_mut(&name).expect("validated target state");
            state.previous_initialized = true;
            state.previous_mipmaps_initialized = false;
            copied.push(TerrainSourceResourceRole::ShaderPackColor(name));
        }
        Ok(copied)
    }

    /// Generates the complete descendant chain for source-declared mipmapped
    /// colors. It is deliberately a named semantic operation rather than an
    /// implicit sampler side effect, so Vulkan and OpenGL lower the same GAL
    /// transition and later passes can reject stale descendants precisely.
    pub(crate) fn append_mipmaps(
        &mut self,
        targets: &ShaderPackColorTargets,
        roles: &[TerrainSourceResourceRole],
        operations: &mut Vec<CommandOp>,
    ) -> GalResult<()> {
        self.append_mipmaps_for_side(targets, roles, false, operations)
    }

    /// Generates mip descendants for feedback images after bootstrap or a
    /// current-to-previous copy. Feedback and current sides never alias.
    pub(crate) fn append_feedback_mipmaps(
        &mut self,
        targets: &ShaderPackColorTargets,
        roles: &[TerrainSourceResourceRole],
        operations: &mut Vec<CommandOp>,
    ) -> GalResult<()> {
        self.append_mipmaps_for_side(targets, roles, true, operations)
    }

    fn append_mipmaps_for_side(
        &mut self,
        targets: &ShaderPackColorTargets,
        roles: &[TerrainSourceResourceRole],
        previous: bool,
        operations: &mut Vec<CommandOp>,
    ) -> GalResult<()> {
        self.require_targets(targets)?;
        let mut names = roles
            .iter()
            .map(|role| {
                role.shader_pack_color_name()
                    .map(str::to_string)
                    .ok_or_else(|| {
                        GalError::invalid_argument(
                            "shader-pack mip generation role is not a named color target",
                        )
                    })
            })
            .collect::<GalResult<Vec<_>>>()?;
        names.sort();
        names.dedup();
        for name in names {
            let target = targets.target(&name).ok_or_else(|| {
                GalError::invalid_argument(format!(
                    "shader-pack mip generation target '{name}' is unavailable"
                ))
            })?;
            if target.mip_levels < 2 {
                return Err(GalError::invalid_argument(format!(
                    "shader-pack color target '{name}' requested mip generation without a staged mip chain"
                )));
            }
            let state = self.targets.get(&name).ok_or_else(|| {
                GalError::invalid_argument(format!(
                    "shader-pack color frame has no target state for mip generation '{name}'"
                ))
            })?;
            if !(if previous {
                state.previous_initialized
            } else {
                state.current_initialized
            }) {
                return Err(GalError::invalid_argument(format!(
                    "shader-pack color target '{name}' cannot generate {} mips before level zero is initialized",
                    if previous { "feedback" } else { "current" }
                )));
            }
            let texture = if previous {
                target.previous_texture.ok_or_else(|| {
                    GalError::invalid_argument(format!(
                        "shader-pack color target '{name}' has no feedback image for mip generation"
                    ))
                })?
            } else {
                target.current_texture
            };
            let mips_initialized = if previous {
                state.previous_mipmaps_initialized
            } else {
                state.mipmaps_initialized
            };
            let all_mips = TextureSubresourceRange {
                base_mip: 0,
                mip_count: target.mip_levels,
                base_layer: 0,
                layer_count: 1,
            };
            let base_mip = TextureSubresourceRange {
                base_mip: 0,
                mip_count: 1,
                base_layer: 0,
                layer_count: 1,
            };
            let descendants = TextureSubresourceRange {
                base_mip: 1,
                mip_count: target.mip_levels - 1,
                base_layer: 0,
                layer_count: 1,
            };
            operations.push(CommandOp::Barrier(ResourceBarrier {
                resource: texture,
                subresources: Some(base_mip),
                before: TextureUsageState::ShaderRead,
                after: TextureUsageState::TransferSrc,
                src_queue: QueueClass::Graphics,
                dst_queue: QueueClass::Graphics,
            }));
            operations.push(CommandOp::Barrier(ResourceBarrier {
                resource: texture,
                subresources: Some(descendants),
                before: if mips_initialized {
                    TextureUsageState::ShaderRead
                } else {
                    TextureUsageState::Undefined
                },
                after: TextureUsageState::TransferDst,
                src_queue: QueueClass::Graphics,
                dst_queue: QueueClass::Graphics,
            }));
            operations.push(CommandOp::GenerateMipmaps {
                texture,
                subresources: all_mips,
            });
            operations.push(CommandOp::Barrier(ResourceBarrier {
                resource: texture,
                subresources: Some(all_mips),
                before: TextureUsageState::TransferSrc,
                after: TextureUsageState::ShaderRead,
                src_queue: QueueClass::Graphics,
                dst_queue: QueueClass::Graphics,
            }));
            let state = self.targets.get_mut(&name).expect("validated target state");
            if previous {
                state.previous_mipmaps_initialized = true;
            } else {
                state.mipmaps_initialized = true;
            }
        }
        Ok(())
    }

    /// Appends the source-defined current-to-previous copies for feedback
    /// targets. The copy happens only after the complete source schedule has
    /// written a target, and callers must confirm this frame after the whole
    /// submission succeeds before its history can be sampled next frame.
    pub(crate) fn append_feedback_copies(
        &mut self,
        targets: &ShaderPackColorTargets,
        operations: &mut Vec<CommandOp>,
    ) -> GalResult<()> {
        self.require_targets(targets)?;
        for (name, target) in targets.targets() {
            let Some(previous_texture) = target.previous_texture else {
                continue;
            };
            let state = self.targets.get(name).expect("validated target state");
            if !state.current_initialized {
                return Err(GalError::invalid_argument(format!(
                    "shader-pack feedback target '{name}' was not initialized by the complete source frame"
                )));
            }
            operations.push(CommandOp::Barrier(texture_barrier(
                target.current_texture,
                TextureUsageState::ShaderRead,
                TextureUsageState::TransferSrc,
            )));
            operations.push(CommandOp::Barrier(texture_barrier(
                previous_texture,
                if state.previous_initialized {
                    TextureUsageState::ShaderRead
                } else {
                    TextureUsageState::Undefined
                },
                TextureUsageState::TransferDst,
            )));
            operations.push(CommandOp::CopyTexture(TextureImageCopyRegion {
                row_order: crate::render::vulkanic::commands::TextureRowOrder::Preserve,
                src_texture: target.current_texture,
                src_mip: 0,
                src_layer: 0,
                src_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
                dst_texture: previous_texture,
                dst_mip: 0,
                dst_layer: 0,
                dst_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
                extent: self.identity.extent,
            }));
            operations.push(CommandOp::Barrier(texture_barrier(
                previous_texture,
                TextureUsageState::TransferDst,
                TextureUsageState::ShaderRead,
            )));
            operations.push(CommandOp::Barrier(texture_barrier(
                target.current_texture,
                TextureUsageState::TransferSrc,
                TextureUsageState::ShaderRead,
            )));
            self.targets
                .get_mut(name)
                .expect("validated target state")
                .previous_initialized = true;
            self.targets
                .get_mut(name)
                .expect("validated target state")
                .previous_mipmaps_initialized = false;
        }
        Ok(())
    }

    pub(crate) fn into_confirmed_state(self) -> ShaderPackColorFrameState {
        ShaderPackColorFrameState {
            identity: self.identity,
            targets: self.targets,
        }
    }

    fn target_state_mut(
        &mut self,
        role: &TerrainSourceResourceRole,
    ) -> GalResult<&mut ShaderPackColorFrameTargetState> {
        let name = role.shader_pack_color_name().ok_or_else(|| {
            GalError::invalid_argument(
                "shader-pack color frame output is not a named semantic color role",
            )
        })?;
        self.targets.get_mut(name).ok_or_else(|| {
            GalError::invalid_argument(format!(
                "shader-pack color frame has no target state for output '{name}'"
            ))
        })
    }

    fn require_targets(&self, targets: &ShaderPackColorTargets) -> GalResult<()> {
        if self.identity != targets.identity {
            return Err(GalError::invalid_argument(
                "shader-pack color-frame plan belongs to a different target generation",
            ));
        }
        Ok(())
    }
}

impl ShaderPackColorBootstrapPlan {
    fn stage(
        gal: &mut VulkanicGal,
        targets: &ShaderPackColorTargets,
        values: ShaderPackColorBootstrapClearValues,
    ) -> GalResult<Self> {
        let mut created = Vec::new();
        let result = (|| -> GalResult<Self> {
            let mut bootstrap_targets = Vec::new();
            for (name, target) in targets.targets() {
                let clear_color = source_color_clear_color(
                    target.source_slot,
                    target.clear_color_bits,
                    values.fog_color,
                );
                let mut append_target =
                    |previous: bool, texture: Handle, view: Handle| -> GalResult<()> {
                        let side = if previous { "previous" } else { "current" };
                        let label = format!(
                            "shader-pack-color-bootstrap.world{}-pack{}.{}.{}",
                            targets.identity.world_generation,
                            targets.identity.shader_pack_generation,
                            name,
                            side,
                        );
                        let render_target = gal.create_render_target(RenderTargetDesc {
                            label: format!("{label}.target"),
                            color_views: vec![view],
                            depth_stencil_view: None,
                            extent: targets.identity.extent,
                        })?;
                        created.push(render_target);
                        let render_pass = gal.create_render_pass(RenderPassDesc {
                            label: format!("{label}.pass"),
                            target: render_target,
                            color_formats: vec![target.format],
                            depth_format: None,
                        })?;
                        created.push(render_pass);
                        bootstrap_targets.push(ShaderPackColorBootstrapTarget {
                            name: name.to_string(),
                            previous,
                            texture,
                            view,
                            render_target,
                            render_pass,
                            clear_color,
                        });
                        Ok(())
                    };
                append_target(
                    false,
                    target.current_texture,
                    target.current_attachment_view,
                )?;
                if let (Some(texture), Some(view)) = (target.previous_texture, target.previous_view)
                {
                    append_target(
                        true,
                        texture,
                        target.previous_attachment_view.unwrap_or(view),
                    )?;
                }
            }
            Ok(Self {
                targets: bootstrap_targets,
            })
        })();
        if result.is_err() {
            for handle in created.into_iter().rev() {
                let _ = gal.destroy(handle);
            }
        }
        result
    }

    fn append(&self, operations: &mut Vec<CommandOp>) {
        for target in &self.targets {
            operations.push(CommandOp::Barrier(texture_barrier(
                target.texture,
                TextureUsageState::Undefined,
                TextureUsageState::ColorAttachment,
            )));
            operations.push(CommandOp::BeginPass {
                pass: target.render_pass,
                target: target.render_target,
                colors: vec![PassAttachment {
                    view: target.view,
                    load_op: AttachmentLoadOp::Clear,
                    store_op: AttachmentStoreOp::Store,
                    clear_color: Some(target.clear_color),
                }],
                depth_stencil: None,
            });
            operations.push(CommandOp::EndPass);
            operations.push(CommandOp::Barrier(texture_barrier(
                target.texture,
                TextureUsageState::ColorAttachment,
                TextureUsageState::ShaderRead,
            )));
        }
    }

    pub(crate) fn destroy(self, gal: &mut VulkanicGal) {
        for target in self.targets.into_iter().rev() {
            for handle in [target.render_pass, target.render_target] {
                let _ = gal.destroy(handle);
            }
        }
    }
}

pub(crate) fn source_color_clear_color(
    source_slot: u32,
    clear_color_bits: Option<[u32; 4]>,
    fog_color: ClearColor,
) -> ClearColor {
    // Capture-only target/readback isolation. This is intentionally scoped to
    // the named source primary target and cannot alter normal route selection
    // or material execution.
    if source_slot == 0
        && matches!(
            std::env::var("MATTMC_RUST_SELECTED_SOURCE_CLEAR_PROBE")
                .ok()
                .as_deref()
                .map(str::trim),
            Some("primary-red")
        )
    {
        return ClearColor {
            r: 1.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        };
    }
    if let Some([r, g, b, a]) = clear_color_bits {
        return ClearColor {
            r: f32::from_bits(r),
            g: f32::from_bits(g),
            b: f32::from_bits(b),
            a: f32::from_bits(a),
        };
    }
    match source_slot {
        // The portable Iris/OptiFine target contract starts main color at the
        // semantic fog color and gives depth history a known far-depth value.
        0 => ClearColor {
            r: fog_color.r,
            g: fog_color.g,
            b: fog_color.b,
            a: 1.0,
        },
        1 => ClearColor {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        },
        _ => ClearColor {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        },
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

/// A two-phase private target cache. The caller must confirm the combined
/// submission that used a pending replacement before it becomes active;
/// rejected submissions discard only their newly created targets.
#[derive(Debug, Default)]
pub(crate) struct ShaderPackColorTargetCache {
    active: Option<ShaderPackColorTargets>,
    pending: Option<ShaderPackColorTargets>,
    confirmed_frame: Option<ShaderPackColorFrameState>,
}

impl ShaderPackColorTargetCache {
    pub(crate) fn stage(
        &mut self,
        gal: &mut VulkanicGal,
        identity: ShaderPackColorTargetIdentity,
        manifest: &ShaderPackColorTargetManifest,
    ) -> GalResult<ShaderPackColorTargets> {
        if let Some(pending) = &self.pending {
            if pending.identity != identity {
                return Err(GalError::backend(
                    "shader-pack color target replacement is awaiting submission confirmation",
                ));
            }
            return Ok(pending.clone());
        }
        if let Some(active) = &self.active {
            if active.identity == identity {
                return Ok(active.clone());
            }
        }
        let targets = ShaderPackColorTargets::create(gal, identity, manifest)?;
        self.pending = Some(targets.clone());
        Ok(targets)
    }

    pub(crate) fn confirm_submission(&mut self, gal: &mut VulkanicGal) {
        let Some(replacement) = self.pending.take() else {
            return;
        };
        if let Some(previous) = self.active.replace(replacement) {
            previous.destroy(gal);
        }
        // A replacement owns different images, so even a semantically similar
        // source target set must not inherit history across its generation.
        self.confirmed_frame = None;
    }

    /// Starts an exact target-generation source frame. The returned plan is
    /// private semantic scheduling state; it cannot make a target valid until
    /// `confirm_frame_submission` follows a successful combined submission.
    pub(crate) fn begin_frame(
        &self,
        targets: &ShaderPackColorTargets,
    ) -> GalResult<ShaderPackColorFramePlan> {
        let prior = self
            .confirmed_frame
            .as_ref()
            .filter(|state| state.identity == targets.identity);
        ShaderPackColorFramePlan::begin(targets, prior)
    }

    /// Atomically advances target allocation and semantic color history after
    /// the exact combined source submission succeeds. The caller must discard
    /// the frame plan on failure; no rejected draw can seed feedback history.
    pub(crate) fn confirm_frame_submission(
        &mut self,
        gal: &mut VulkanicGal,
        frame: ShaderPackColorFramePlan,
    ) -> GalResult<()> {
        let frame_state = frame.into_confirmed_state();
        let matches_active = self
            .active
            .as_ref()
            .is_some_and(|active| active.identity == frame_state.identity);
        let matches_pending = self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.identity == frame_state.identity);
        if !matches_active && !matches_pending {
            return Err(GalError::invalid_argument(
                "shader-pack color frame confirmation has no matching active or pending target generation",
            ));
        }
        if matches_pending {
            self.confirm_submission(gal);
        }
        self.confirmed_frame = Some(frame_state);
        Ok(())
    }

    pub(crate) fn discard_submission(&mut self, gal: &mut VulkanicGal) {
        if let Some(pending) = self.pending.take() {
            pending.destroy(gal);
        }
    }

    pub(crate) fn destroy(&mut self, gal: &mut VulkanicGal) {
        self.discard_submission(gal);
        self.confirmed_frame = None;
        if let Some(active) = self.active.take() {
            active.destroy(gal);
        }
    }
}

impl ShaderPackColorTargetManifest {
    pub fn from_source(
        source: &ShaderPackSource,
        bindings: &TerrainSourceResourceBindings,
    ) -> GalResult<Self> {
        let settings = source.get(PIPELINE_SETTINGS_PATH).ok_or_else(|| {
            GalError::unsupported_feature(format!(
                "shader-pack source is missing required color-target declaration file {PIPELINE_SETTINGS_PATH}"
            ))
        })?;
        let declarations = parse_pipeline_settings(settings)?;
        let mut targets = BTreeMap::new();
        for slot in 0..MAX_SOURCE_COLOR_TARGETS {
            let role = bindings.shader_pack_color_output_for_slot(slot)?;
            let name = role
                .shader_pack_color_name()
                .expect("shader_pack_color_output_for_slot returns a color role")
                .to_string();
            // `colortex` directives use the portable OptiFine/Iris protocol:
            // absent properties have a defined RGBA8/clear/no-mipmap default.
            // This is source-language semantics, not an Iris render-target
            // query or a backend substitution.
            let target = declarations
                .get(&slot)
                .copied()
                .unwrap_or_else(SourceColorSlotDecl::protocol_default);
            if targets
                .insert(
                    name.clone(),
                    ShaderPackColorTargetDecl {
                        source_slot: slot,
                        role,
                        format: target.format,
                        clear_each_frame: target.clear_each_frame,
                        clear_color_bits: target.clear_color_bits,
                    },
                )
                .is_some()
            {
                return Err(GalError::invalid_argument(format!(
                    "shader-pack source maps more than one color slot to semantic target '{name}'"
                )));
            }
        }
        Ok(Self {
            pack_name: source.name().to_string(),
            generation: source.generation(),
            targets,
        })
    }

    pub fn pack_name(&self) -> &str {
        &self.pack_name
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }

    pub fn target(&self, name: &str) -> Option<&ShaderPackColorTargetDecl> {
        self.targets.get(name)
    }

    /// Resolves a source output slot through the pack's semantic target
    /// declaration. A source executor uses this before creating a Rust-owned
    /// render target, so sparse or mismatched `DRAWBUFFERS` locations fail at
    /// source scheduling rather than being guessed by either backend.
    pub fn target_for_source_slot(&self, source_slot: u32) -> Option<&ShaderPackColorTargetDecl> {
        self.targets
            .values()
            .find(|target| target.source_slot == source_slot)
    }

    pub fn targets(&self) -> impl Iterator<Item = &ShaderPackColorTargetDecl> {
        self.targets.values()
    }

    /// Rejects a candidate source plan until every color target it needs has
    /// an exact generic GAL format. This deliberately names the semantic
    /// target, never an underlying image, attachment, or API handle. Native
    /// support remains a separate cache-staging check, before route admission.
    pub fn require_gal_schema_formats(&self) -> GalResult<()> {
        // Every parsed source format has an exact schema mapping. Native
        // image support is checked during Rust-owned target staging.
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
struct SourceColorSlotDecl {
    format: ShaderPackColorFormat,
    clear_each_frame: bool,
    clear_color_bits: Option<[u32; 4]>,
}

impl SourceColorSlotDecl {
    const fn protocol_default() -> Self {
        Self {
            format: ShaderPackColorFormat::Rgba8,
            clear_each_frame: true,
            clear_color_bits: None,
        }
    }
}

#[derive(Default)]
struct PartialSourceColorSlotDecl {
    format: Option<ShaderPackColorFormat>,
    clear_each_frame: Option<bool>,
    clear_color_bits: Option<[u32; 4]>,
}

fn parse_pipeline_settings(contents: &str) -> GalResult<BTreeMap<u32, SourceColorSlotDecl>> {
    let mut slots = BTreeMap::<u32, PartialSourceColorSlotDecl>::new();
    for (line_number, raw_line) in contents.lines().enumerate() {
        // Shader-pack `const` directives are intentionally discovered from
        // the raw source, including `/* ... */` blocks. Complementary uses
        // that documented directive channel for its colortex formats.
        let line = directive_fragment(raw_line);
        if line.is_empty() {
            continue;
        }
        if let Some((slot, format)) = parse_const_assignment(line, "int", "Format")? {
            let format = ShaderPackColorFormat::parse(&format)?;
            let declaration = slots.entry(slot).or_default();
            if declaration.format.replace(format).is_some() {
                return Err(GalError::invalid_argument(format!(
                    "shader-pack color target colortex{slot} declares its format more than once at line {}",
                    line_number + 1
                )));
            }
            continue;
        }
        if let Some((slot, value)) = parse_const_assignment(line, "bool", "Clear")? {
            let declaration = slots.entry(slot).or_default();
            if declaration
                .clear_each_frame
                .replace(parse_bool(&value)?)
                .is_some()
            {
                return Err(GalError::invalid_argument(format!(
                    "shader-pack color target colortex{slot} declares its clear policy more than once at line {}",
                    line_number + 1
                )));
            }
            continue;
        }
        if let Some((slot, value)) = parse_const_assignment(line, "vec4", "ClearColor")? {
            let declaration = slots.entry(slot).or_default();
            if declaration
                .clear_color_bits
                .replace(parse_clear_color_bits(&value)?)
                .is_some()
            {
                return Err(GalError::invalid_argument(format!(
                    "shader-pack color target colortex{slot} declares its clear color more than once at line {}",
                    line_number + 1
                )));
            }
            continue;
        }
    }
    let mut complete = BTreeMap::new();
    for (slot, declaration) in slots {
        if slot >= MAX_SOURCE_COLOR_TARGETS {
            return Err(GalError::unsupported_feature(format!(
                "shader-pack color target colortex{slot} exceeds the bounded source target range"
            )));
        }
        let defaults = SourceColorSlotDecl::protocol_default();
        let format = declaration.format.unwrap_or(defaults.format);
        let clear_each_frame = declaration
            .clear_each_frame
            .unwrap_or(defaults.clear_each_frame);
        let clear_color_bits = declaration.clear_color_bits.or(defaults.clear_color_bits);
        complete.insert(
            slot,
            SourceColorSlotDecl {
                format,
                clear_each_frame,
                clear_color_bits,
            },
        );
    }
    Ok(complete)
}

fn parse_const_assignment(
    line: &str,
    value_type: &str,
    suffix: &str,
) -> GalResult<Option<(u32, String)>> {
    let Some(rest) = line.strip_prefix(&format!("const {value_type} ")) else {
        return Ok(None);
    };
    let Some((name, value)) = rest.split_once('=') else {
        return Err(GalError::invalid_argument(
            "shader-pack color target declaration is missing '='",
        ));
    };
    let name = name.trim();
    let Some(slot) = name
        .strip_prefix("colortex")
        .and_then(|name| name.strip_suffix(suffix))
    else {
        return Ok(None);
    };
    let slot = slot.parse::<u32>().map_err(|_| {
        GalError::invalid_argument(format!(
            "shader-pack color target declaration '{name}' has an invalid slot"
        ))
    })?;
    let value = value
        .split_once(';')
        .map(|(value, _)| value)
        .unwrap_or(value)
        .trim();
    if value.is_empty() {
        return Err(GalError::invalid_argument(format!(
            "shader-pack color target declaration '{name}' has an empty value"
        )));
    }
    Ok(Some((slot, value.to_string())))
}

fn parse_bool(value: &str) -> GalResult<bool> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(GalError::invalid_argument(format!(
            "shader-pack color target boolean must be true or false, found '{value}'"
        ))),
    }
}

/// Parses the bounded literal subset of the shader-pack clear-color
/// directive. Source expressions, macros, and constructors with non-literal
/// values are intentionally rejected until the reusable constant evaluator
/// can represent them; silently substituting a protocol default would make a
/// selected source generation semantically dishonest.
fn parse_clear_color_bits(value: &str) -> GalResult<[u32; 4]> {
    let value = value.trim();
    let Some(arguments) = value
        .strip_prefix("vec4(")
        .and_then(|arguments| arguments.strip_suffix(')'))
    else {
        return Err(GalError::unsupported_feature(format!(
            "shader-pack color clear value '{value}' must be a literal vec4(...) constructor"
        )));
    };
    let components = arguments.split(',').map(str::trim).collect::<Vec<_>>();
    let components = match components.as_slice() {
        [component] => [*component, *component, *component, *component],
        [r, g, b, a] => [*r, *g, *b, *a],
        _ => {
            return Err(GalError::invalid_argument(format!(
                "shader-pack color clear value '{value}' must contain one or four literal components"
            )));
        }
    };
    let mut bits = [0; 4];
    for (index, component) in components.into_iter().enumerate() {
        let component = component
            .strip_suffix(['f', 'F'])
            .unwrap_or(component)
            .trim();
        let parsed = component.parse::<f32>().map_err(|_| {
            GalError::invalid_argument(format!(
                "shader-pack color clear component '{component}' is not a finite literal"
            ))
        })?;
        if !parsed.is_finite() {
            return Err(GalError::invalid_argument(format!(
                "shader-pack color clear component '{component}' must be finite"
            )));
        }
        bits[index] = parsed.to_bits();
    }
    Ok(bits)
}

fn directive_fragment(line: &str) -> &str {
    line.find("const ")
        .map(|index| line[index..].trim())
        .unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::vulkanic::backends::mock::MockBackend;
    use crate::render::vulkanic::commands::{CommandList, CommandListDesc, SubmissionBatch};
    use crate::render::vulkanic::gal::VulkanicGal;
    use crate::render::vulkanic::shader_pack::lowering::lower_fullscreen_source_pair;
    use crate::render::vulkanic::shader_pack::preprocess::preprocess_source_stage_pair;
    use crate::render::vulkanic::shader_pack::programs::prepare_lowered_fullscreen_source_program;
    use crate::render::vulkanic::shader_pack::source::ShaderSourceFile;
    use crate::render::vulkanic::shader_pack::terrain_contract::{
        TerrainSourceStage, TerrainSourceStages,
    };
    use crate::render::vulkanic::shader_pack::terrain_source_resources::{
        TerrainSourceResourceBindings, TERRAIN_RESOURCE_BINDINGS_PATH,
    };

    fn bindings() -> &'static str {
        concat!(
            "colortex0=shader_pack_color:primary\n",
            "colortex1=shader_pack_color:previous_depth\n",
            "colortex2=shader_pack_color:temporal_aa\n",
            "colortex3=shader_pack_color:translucent_final\n",
            "colortex4=shader_pack_color:volumetric_factor\n",
            "colortex5=shader_pack_color:normal_scene\n",
            "colortex6=shader_pack_color:material_auxiliary\n",
            "colortex7=shader_pack_color:temporal_reflection\n",
        )
    }

    fn settings() -> &'static str {
        concat!(
            "/* exact source declarations */\n",
            "const int colortex0Format = R11F_G11F_B10F;\n",
            "const int colortex1Format = R32F;\n",
            "const int colortex2Format = RGB16F;\n",
            "const int colortex3Format = RGBA8;\n",
            "const int colortex4Format = R8;\n",
            "const int colortex5Format = RGBA8_SNORM;\n",
            "const int colortex6Format = RGBA8;\n",
            "const int colortex7Format = RGBA16F;\n",
            "const bool colortex0Clear = true;\n",
            "const bool colortex1Clear = false;\n",
            "const bool colortex2Clear = false;\n",
            "const bool colortex3Clear = true;\n",
            "const bool colortex4Clear = false;\n",
            "const bool colortex5Clear = false;\n",
            "const bool colortex6Clear = true;\n",
            "const bool colortex7Clear = false;\n",
        )
    }

    fn source(settings: &str) -> ShaderPackSource {
        ShaderPackSource::new(
            "source-targets",
            7,
            vec![
                ShaderSourceFile::new(PIPELINE_SETTINGS_PATH, settings),
                ShaderSourceFile::new(TERRAIN_RESOURCE_BINDINGS_PATH, bindings()),
            ],
        )
        .unwrap()
    }

    fn gal() -> VulkanicGal {
        VulkanicGal::new_with_backend(Box::new(MockBackend::default()), false)
    }

    fn fullscreen_program(source: &ShaderPackSource) -> LoweredFullscreenSourceProgram {
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
        let artifacts = preprocess_source_stage_pair(source, &stages).unwrap();
        let bindings = TerrainSourceResourceBindings::from_source(source).unwrap();
        let lowered =
            lower_fullscreen_source_pair(&artifacts.vertex, &artifacts.fragment, &bindings)
                .unwrap();
        let opaque_bindings = lowered
            .opaque_resource_contract()
            .bind_semantic_roles(&bindings)
            .unwrap();
        prepare_lowered_fullscreen_source_program(
            source.name(),
            source.generation(),
            "world0/deferred1.fsh",
            &lowered,
            &opaque_bindings,
        )
        .unwrap()
    }

    #[test]
    fn manifest_keeps_named_target_semantics_and_exact_source_formats() {
        let source = source(settings());
        let bindings = TerrainSourceResourceBindings::from_source(&source).unwrap();
        let manifest = ShaderPackColorTargetManifest::from_source(&source, &bindings).unwrap();
        let primary = manifest.target("primary").unwrap();
        assert_eq!(0, primary.source_slot());
        assert_eq!(ShaderPackColorFormat::R11fG11fB10f, primary.format);
        assert!(primary.clear_each_frame);
        assert_eq!(
            TextureFormat::R11fG11fB10f,
            primary.gal_schema_color_format()
        );
        assert_eq!(
            TextureFormat::Rgba16Float,
            manifest
                .target("temporal_reflection")
                .unwrap()
                .gal_schema_color_format()
        );
        assert_eq!(8, manifest.targets().count());
        assert_eq!(
            "material_auxiliary",
            manifest.target_for_source_slot(6).unwrap().name()
        );
        assert!(manifest.require_gal_schema_formats().is_ok());
    }

    #[test]
    fn manifest_uses_portable_protocol_defaults_for_missing_properties() {
        let incomplete = settings().replace("const bool colortex6Clear = true;\n", "");
        let incomplete_source = source(&incomplete);
        let bindings = TerrainSourceResourceBindings::from_source(&incomplete_source).unwrap();
        let manifest =
            ShaderPackColorTargetManifest::from_source(&incomplete_source, &bindings).unwrap();
        assert!(
            manifest
                .target("material_auxiliary")
                .unwrap()
                .clear_each_frame
        );

        let no_format = settings().replace("const int colortex6Format = RGBA8;\n", "");
        let no_format_source = source(&no_format);
        let bindings = TerrainSourceResourceBindings::from_source(&no_format_source).unwrap();
        assert_eq!(
            ShaderPackColorFormat::Rgba8,
            ShaderPackColorTargetManifest::from_source(&no_format_source, &bindings)
                .unwrap()
                .target("material_auxiliary")
                .unwrap()
                .format
        );
    }

    #[test]
    fn manifest_preserves_literal_source_clear_colors_without_backend_identity() {
        let with_clear_color = format!(
            "{}const vec4 colortex4ClearColor = vec4(0.125, 0.25f, 0.5, 1.0);\n",
            settings()
        );
        let source = source(&with_clear_color);
        let bindings = TerrainSourceResourceBindings::from_source(&source).unwrap();
        let manifest = ShaderPackColorTargetManifest::from_source(&source, &bindings).unwrap();
        assert_eq!(
            Some([
                0.125f32.to_bits(),
                0.25f32.to_bits(),
                0.5f32.to_bits(),
                1.0f32.to_bits(),
            ]),
            manifest
                .target("volumetric_factor")
                .unwrap()
                .clear_color_bits
        );

        let identity = ShaderPackColorTargetIdentity::new(
            41,
            source.generation(),
            Extent3d {
                width: 320,
                height: 180,
                depth: 1,
            },
            std::iter::empty::<String>(),
            std::iter::empty::<String>(),
        )
        .unwrap();
        let mut gal = gal();
        let mut cache = ShaderPackColorTargetCache::default();
        let targets = cache.stage(&mut gal, identity, &manifest).unwrap();
        let mut frame = cache.begin_frame(&targets).unwrap();
        let bootstrap = frame
            .stage_full_clear(
                &mut gal,
                &targets,
                ShaderPackColorBootstrapClearValues {
                    fog_color: ClearColor {
                        r: 0.7,
                        g: 0.6,
                        b: 0.5,
                        a: 1.0,
                    },
                },
            )
            .unwrap();
        let mut operations = Vec::new();
        frame
            .append_full_clear(&bootstrap, &mut operations)
            .unwrap();
        assert!(operations.iter().any(|operation| matches!(
            operation,
            CommandOp::BeginPass { colors, .. }
                if colors.first().and_then(|attachment| attachment.clear_color)
                    == Some(ClearColor { r: 0.125, g: 0.25, b: 0.5, a: 1.0 })
        )));
        bootstrap.destroy(&mut gal);
        cache.destroy(&mut gal);
    }

    #[test]
    fn manifest_rejects_nonliteral_or_duplicate_source_clear_colors() {
        let nonliteral = format!(
            "{}const vec4 colortex4ClearColor = vec4(fogColor.rgb, 1.0);\n",
            settings()
        );
        let nonliteral_source = source(&nonliteral);
        let bindings = TerrainSourceResourceBindings::from_source(&nonliteral_source).unwrap();
        assert!(
            ShaderPackColorTargetManifest::from_source(&nonliteral_source, &bindings)
                .unwrap_err()
                .to_string()
                .contains("shader-pack color clear value")
        );

        let duplicate = format!(
            "{}const vec4 colortex4ClearColor = vec4(0.0);\nconst vec4 colortex4ClearColor = vec4(1.0);\n",
            settings()
        );
        let duplicate_source = source(&duplicate);
        let bindings = TerrainSourceResourceBindings::from_source(&duplicate_source).unwrap();
        assert!(
            ShaderPackColorTargetManifest::from_source(&duplicate_source, &bindings)
                .unwrap_err()
                .to_string()
                .contains("clear color more than once")
        );
    }

    #[test]
    fn manifest_rejects_duplicated_or_unknown_format_declarations() {
        let duplicate = format!("{}const int colortex0Format = RGBA8;\n", settings());
        let duplicate_source = source(&duplicate);
        let bindings = TerrainSourceResourceBindings::from_source(&duplicate_source).unwrap();
        assert!(
            ShaderPackColorTargetManifest::from_source(&duplicate_source, &bindings)
                .unwrap_err()
                .to_string()
                .contains("format more than once")
        );

        let unknown = settings().replace("RGBA16F", "RGB10_A2");
        let unknown_source = source(&unknown);
        let bindings = TerrainSourceResourceBindings::from_source(&unknown_source).unwrap();
        assert!(
            ShaderPackColorTargetManifest::from_source(&unknown_source, &bindings)
                .unwrap_err()
                .to_string()
                .contains("unsupported shader-pack color target format")
        );
    }

    #[test]
    fn private_targets_use_exact_formats_and_feedback_pairs_without_route_selection() {
        let source = source(settings());
        let bindings = TerrainSourceResourceBindings::from_source(&source).unwrap();
        let manifest = ShaderPackColorTargetManifest::from_source(&source, &bindings).unwrap();
        let identity = ShaderPackColorTargetIdentity::new(
            41,
            source.generation(),
            Extent3d {
                width: 320,
                height: 180,
                depth: 1,
            },
            ["primary".to_string()],
            std::iter::empty::<String>(),
        )
        .unwrap();
        let mut gal = gal();
        let mut cache = ShaderPackColorTargetCache::default();

        let targets = cache.stage(&mut gal, identity.clone(), &manifest).unwrap();
        assert_eq!(18, gal.metrics().resource_creates);
        let primary = targets.target("primary").unwrap();
        assert_eq!(TextureFormat::R11fG11fB10f, primary.format);
        assert!(primary.previous_texture.is_some());
        assert!(primary.previous_view.is_some());
        let material_auxiliary = targets.target("material_auxiliary").unwrap();
        assert_eq!(TextureFormat::Rgba8Unorm, material_auxiliary.format);
        assert!(material_auxiliary.previous_texture.is_none());
        assert!(material_auxiliary.previous_view.is_none());
        assert_eq!(8, targets.targets().count());

        cache.confirm_submission(&mut gal);
        let reused = cache.stage(&mut gal, identity, &manifest).unwrap();
        assert_eq!(primary, reused.target("primary").unwrap());
        assert_eq!(18, gal.metrics().resource_creates);

        cache.destroy(&mut gal);
        assert_eq!(18, gal.metrics().resource_destroys);
    }

    #[test]
    fn color_frame_history_advances_only_after_confirmation_and_copies_feedback() {
        let source = source(settings());
        let bindings = TerrainSourceResourceBindings::from_source(&source).unwrap();
        let manifest = ShaderPackColorTargetManifest::from_source(&source, &bindings).unwrap();
        let identity = ShaderPackColorTargetIdentity::new(
            41,
            source.generation(),
            Extent3d {
                width: 320,
                height: 180,
                depth: 1,
            },
            ["primary".to_string()],
            std::iter::empty::<String>(),
        )
        .unwrap();
        let mut gal = gal();
        let mut cache = ShaderPackColorTargetCache::default();
        let targets = cache.stage(&mut gal, identity, &manifest).unwrap();
        let primary = targets.target("primary").unwrap();
        let attachment = FullscreenSourceColorAttachment {
            source_slot: 0,
            role: TerrainSourceResourceRole::ShaderPackColor("primary".to_string()),
            texture: primary.current_texture,
            view: primary.current_view,
            format: primary.format,
            clear_each_frame: primary.clear_each_frame,
            clear_color_bits: primary.clear_color_bits,
        };

        let mut first = cache.begin_frame(&targets).unwrap();
        assert_eq!(
            vec![TextureUsageState::Undefined],
            first
                .attachment_states(std::slice::from_ref(&attachment))
                .unwrap()
        );
        assert!(first.require_sample(&attachment.role, true).is_err());
        first
            .record_pass(
                std::slice::from_ref(&attachment),
                std::slice::from_ref(&attachment),
            )
            .unwrap();
        let mut operations = Vec::new();
        first
            .append_feedback_copies(&targets, &mut operations)
            .unwrap();
        assert!(operations.iter().any(|operation| matches!(
            operation,
            CommandOp::CopyTexture(copy)
                if copy.src_texture == primary.current_texture
                    && copy.dst_texture == primary.previous_texture.unwrap()
        )));

        // This is the only state transition that a successful combined
        // submission may make visible to the next source frame.
        cache
            .confirm_frame_submission(&mut gal, first)
            .expect("a matching staged generation must accept confirmed history");
        let second = cache.begin_frame(&targets).unwrap();
        assert_eq!(
            vec![TextureUsageState::ShaderRead],
            second
                .attachment_states(std::slice::from_ref(&attachment))
                .unwrap()
        );
        second
            .require_sample(&attachment.role, true)
            .expect("only the confirmed feedback copy may seed the next frame");

        // An abandoned frame never modifies the confirmed state.
        let abandoned = cache.begin_frame(&targets).unwrap();
        drop(abandoned);
        let third = cache.begin_frame(&targets).unwrap();
        third
            .require_sample(&attachment.role, true)
            .expect("discarding a plan must leave the prior confirmed history intact");
        cache.destroy(&mut gal);
    }

    #[test]
    fn same_frame_feedback_snapshots_an_earlier_writer_before_self_feedback() {
        let source = source(settings());
        let bindings = TerrainSourceResourceBindings::from_source(&source).unwrap();
        let manifest = ShaderPackColorTargetManifest::from_source(&source, &bindings).unwrap();
        let identity = ShaderPackColorTargetIdentity::new(
            41,
            source.generation(),
            Extent3d {
                width: 320,
                height: 180,
                depth: 1,
            },
            ["primary".to_string()],
            std::iter::empty::<String>(),
        )
        .unwrap();
        let mut gal = gal();
        let mut cache = ShaderPackColorTargetCache::default();
        let targets = cache.stage(&mut gal, identity, &manifest).unwrap();
        let primary = targets.target("primary").unwrap();
        let role = TerrainSourceResourceRole::ShaderPackColor("primary".to_string());
        let mut frame = cache.begin_frame(&targets).unwrap();
        frame
            .record_external_outputs(std::slice::from_ref(&role))
            .unwrap();

        let mut operations = Vec::new();
        assert_eq!(
            vec![role.clone()],
            frame
                .append_same_frame_feedback_snapshots(
                    &targets,
                    std::slice::from_ref(&role),
                    &mut operations,
                )
                .unwrap()
        );
        assert!(operations.iter().any(|operation| matches!(
            operation,
            CommandOp::CopyTexture(copy)
                if copy.src_texture == primary.current_texture
                    && copy.dst_texture == primary.previous_texture.unwrap()
        )));
        frame
            .require_sample(&role, true)
            .expect("the copied same-frame image must be available to the self-feedback sampler");

        let mut no_writer = cache.begin_frame(&targets).unwrap();
        let no_writer_ops = no_writer
            .append_same_frame_feedback_snapshots(
                &targets,
                std::slice::from_ref(&role),
                &mut operations,
            )
            .unwrap();
        assert!(
            no_writer_ops.is_empty(),
            "an unwritten role must retain prior-frame history rather than overwrite it"
        );
        cache.destroy(&mut gal);
    }

    #[test]
    fn full_clear_bootstraps_current_and_feedback_images_before_source_execution() {
        let source = source(settings());
        let bindings = TerrainSourceResourceBindings::from_source(&source).unwrap();
        let manifest = ShaderPackColorTargetManifest::from_source(&source, &bindings).unwrap();
        let identity = ShaderPackColorTargetIdentity::new(
            41,
            source.generation(),
            Extent3d {
                width: 320,
                height: 180,
                depth: 1,
            },
            ["primary".to_string()],
            std::iter::empty::<String>(),
        )
        .unwrap();
        let mut gal = gal();
        let mut cache = ShaderPackColorTargetCache::default();
        let targets = cache.stage(&mut gal, identity, &manifest).unwrap();
        let mut frame = cache.begin_frame(&targets).unwrap();
        let bootstrap = frame
            .stage_full_clear(
                &mut gal,
                &targets,
                ShaderPackColorBootstrapClearValues {
                    fog_color: ClearColor {
                        r: 0.2,
                        g: 0.3,
                        b: 0.4,
                        a: 0.0,
                    },
                },
            )
            .unwrap();
        let mut operations = Vec::new();
        frame
            .append_full_clear(&bootstrap, &mut operations)
            .unwrap();
        let clears = operations
            .iter()
            .filter_map(|operation| match operation {
                CommandOp::BeginPass { colors, .. } => colors.first(),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            9,
            clears.len(),
            "eight current targets plus primary feedback"
        );
        assert!(clears.iter().all(|attachment| {
            attachment.load_op == AttachmentLoadOp::Clear
                && attachment.store_op == AttachmentStoreOp::Store
        }));
        assert!(clears.iter().any(|attachment| {
            attachment.clear_color
                == Some(ClearColor {
                    r: 0.2,
                    g: 0.3,
                    b: 0.4,
                    a: 1.0,
                })
        }));
        assert!(clears.iter().any(|attachment| {
            attachment.clear_color
                == Some(ClearColor {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 1.0,
                })
        }));
        gal.submit(SubmissionBatch {
            label: "shader-pack-color-bootstrap".to_string(),
            command_lists: vec![CommandList::from(CommandListDesc {
                label: "shader-pack-color-bootstrap.commands".to_string(),
                operations,
            })],
        })
        .expect("the source bootstrap clear must be a valid backend-neutral submission");
        bootstrap.destroy(&mut gal);
        cache
            .confirm_frame_submission(&mut gal, frame)
            .expect("only the submitted full clear may seed source color history");
        let next = cache.begin_frame(&targets).unwrap();
        next.require_sample(
            &TerrainSourceResourceRole::ShaderPackColor("primary".to_string()),
            true,
        )
        .expect("the feedback side must be initialized by the submitted full clear");
        cache.destroy(&mut gal);
    }

    #[test]
    fn source_color_bootstrap_is_required_only_for_a_fresh_generation() {
        let source = source(settings());
        let bindings = TerrainSourceResourceBindings::from_source(&source).unwrap();
        let manifest = ShaderPackColorTargetManifest::from_source(&source, &bindings).unwrap();
        let identity = ShaderPackColorTargetIdentity::new(
            41,
            source.generation(),
            Extent3d {
                width: 320,
                height: 180,
                depth: 1,
            },
            std::iter::empty::<String>(),
            std::iter::empty::<String>(),
        )
        .unwrap();
        let mut gal = gal();
        let mut cache = ShaderPackColorTargetCache::default();
        let targets = cache.stage(&mut gal, identity, &manifest).unwrap();

        let mut first = cache.begin_frame(&targets).unwrap();
        assert!(first.requires_initial_clear().unwrap());
        let bootstrap = first
            .stage_full_clear(
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
            )
            .unwrap();
        let mut operations = Vec::new();
        first
            .append_full_clear(&bootstrap, &mut operations)
            .unwrap();
        bootstrap.destroy(&mut gal);
        cache.confirm_frame_submission(&mut gal, first).unwrap();

        let second = cache.begin_frame(&targets).unwrap();
        assert!(!second.requires_initial_clear().unwrap());
        assert!(second
            .stage_full_clear(
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
            )
            .unwrap_err()
            .to_string()
            .contains("only valid for an uninitialized target generation"));
        cache.destroy(&mut gal);
    }

    #[test]
    fn mipmapped_source_sampling_requires_and_records_explicit_generation() {
        let source = source(settings());
        let bindings = TerrainSourceResourceBindings::from_source(&source).unwrap();
        let manifest = ShaderPackColorTargetManifest::from_source(&source, &bindings).unwrap();
        let identity = ShaderPackColorTargetIdentity::new(
            41,
            source.generation(),
            Extent3d {
                width: 320,
                height: 180,
                depth: 1,
            },
            std::iter::empty::<String>(),
            ["primary".to_string()],
        )
        .unwrap();
        let mut gal = gal();
        let mut cache = ShaderPackColorTargetCache::default();
        let targets = cache.stage(&mut gal, identity, &manifest).unwrap();
        let mut frame = cache.begin_frame(&targets).unwrap();
        let bootstrap = frame
            .stage_full_clear(
                &mut gal,
                &targets,
                ShaderPackColorBootstrapClearValues {
                    fog_color: ClearColor {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 1.0,
                    },
                },
            )
            .unwrap();
        let primary = TerrainSourceResourceRole::ShaderPackColor("primary".to_string());
        let mut operations = Vec::new();
        frame
            .append_full_clear(&bootstrap, &mut operations)
            .unwrap();
        assert!(frame
            .require_sample_with_mips(&primary, false, true)
            .is_err());
        frame
            .append_mipmaps(&targets, std::slice::from_ref(&primary), &mut operations)
            .unwrap();
        frame
            .require_sample_with_mips(&primary, false, true)
            .expect("mipmapped sampling must become valid only after GAL mip generation");
        assert!(operations.iter().any(|operation| matches!(
            operation,
            CommandOp::GenerateMipmaps { texture, subresources }
                if *texture == targets.target("primary").unwrap().current_texture
                    && subresources.mip_count > 1
        )));
        gal.submit(SubmissionBatch {
            label: "shader-pack-color-mips".to_string(),
            command_lists: vec![CommandList::from(CommandListDesc {
                label: "shader-pack-color-mips.commands".to_string(),
                operations,
            })],
        })
        .expect("the explicit source mip transaction must validate");
        bootstrap.destroy(&mut gal);
        cache.confirm_frame_submission(&mut gal, frame).unwrap();
        cache.destroy(&mut gal);
    }

    #[test]
    fn private_targets_allocate_source_requested_mip_chains_without_claiming_execution() {
        let source = source(settings());
        let bindings = TerrainSourceResourceBindings::from_source(&source).unwrap();
        let manifest = ShaderPackColorTargetManifest::from_source(&source, &bindings).unwrap();
        let identity = ShaderPackColorTargetIdentity::new(
            41,
            source.generation(),
            Extent3d {
                width: 320,
                height: 180,
                depth: 1,
            },
            ["primary".to_string()],
            ["primary".to_string(), "material_auxiliary".to_string()],
        )
        .unwrap();
        let mut gal = gal();
        let mut cache = ShaderPackColorTargetCache::default();

        let targets = cache.stage(&mut gal, identity.clone(), &manifest).unwrap();
        assert_eq!(9, targets.target("primary").unwrap().mip_levels);
        assert_eq!(9, targets.target("material_auxiliary").unwrap().mip_levels);
        assert_eq!(1, targets.target("previous_depth").unwrap().mip_levels);

        cache.confirm_submission(&mut gal);
        let without_mips = ShaderPackColorTargetIdentity::new(
            41,
            source.generation(),
            identity.extent,
            ["primary".to_string()],
            std::iter::empty::<String>(),
        )
        .unwrap();
        let replacement = cache.stage(&mut gal, without_mips, &manifest).unwrap();
        assert_eq!(1, replacement.target("primary").unwrap().mip_levels);
        cache.discard_submission(&mut gal);
        cache.destroy(&mut gal);
    }

    #[test]
    fn fullscreen_color_resources_bind_feedback_and_mips_from_source_semantics() {
        let source = ShaderPackSource::new(
            "fullscreen-source-targets",
            7,
            vec![
                ShaderSourceFile::new(PIPELINE_SETTINGS_PATH, settings()),
                ShaderSourceFile::new(TERRAIN_RESOURCE_BINDINGS_PATH, bindings()),
                ShaderSourceFile::new(
                    "world0/deferred1.vsh",
                    "#version 130\nout vec2 uv;\nvoid main() { uv = gl_MultiTexCoord0.xy; gl_Position = ftransform(); }",
                ),
                ShaderSourceFile::new(
                    "world0/deferred1.fsh",
                    "#version 130\n/* DRAWBUFFERS:0 */\nin vec2 uv;\nuniform sampler2D colortex0;\n/* const bool colortex0MipmapEnabled = true; */\nvoid main() { gl_FragData[0] = texture2D(colortex0, uv); }",
                ),
            ],
        )
        .unwrap();
        let bindings = TerrainSourceResourceBindings::from_source(&source).unwrap();
        let manifest = ShaderPackColorTargetManifest::from_source(&source, &bindings).unwrap();
        let identity = ShaderPackColorTargetIdentity::new(
            41,
            source.generation(),
            Extent3d {
                width: 320,
                height: 180,
                depth: 1,
            },
            ["primary".to_string()],
            ["primary".to_string()],
        )
        .unwrap();
        let mut gal = gal();
        let mut cache = ShaderPackColorTargetCache::default();
        let targets = cache.stage(&mut gal, identity, &manifest).unwrap();
        let program = fullscreen_program(&source);
        let resources = prepare_fullscreen_source_color_resources(&mut gal, &program, &targets)
            .expect("source-declared feedback/mips must prepare owned color resources");
        let primary = targets.target("primary").unwrap();
        assert_eq!(9, primary.mip_levels);
        assert_eq!(
            primary.previous_view,
            resources.sampled_view_for(TerrainSourceResourceRole::ShaderPackColor(
                "primary".to_string()
            ))
        );
        assert!(resources
            .resources()
            .combined_sampler_for(TerrainSourceResourceRole::ShaderPackColor(
                "primary".to_string()
            ))
            .is_some());
        let outputs = resolve_fullscreen_source_color_attachments(&program, &manifest, &targets)
            .expect("the lowered output must resolve through the same semantic color target");
        assert_eq!(1, outputs.len());
        assert_eq!(0, outputs[0].source_slot);
        assert_eq!(
            TerrainSourceResourceRole::ShaderPackColor("primary".to_string()),
            outputs[0].role
        );
        assert_eq!(primary.current_attachment_view, outputs[0].view);
        let all_slots = source_color_attachments_by_slot(&manifest, &targets)
            .expect("all manifest color slots must resolve without backend defaults");
        assert_eq!(8, all_slots.len());
        assert_eq!(0, all_slots[0].source_slot);
        assert_eq!(6, all_slots[6].source_slot);
        assert_eq!(
            TerrainSourceResourceRole::ShaderPackColor("material_auxiliary".to_string()),
            all_slots[6].role
        );
        resources.destroy(&mut gal);
        cache.destroy(&mut gal);
    }

    #[test]
    fn generic_source_color_resources_bind_current_targets_without_fullscreen_policy() {
        let source = ShaderPackSource::new(
            "generic-source-targets",
            7,
            vec![
                ShaderSourceFile::new(PIPELINE_SETTINGS_PATH, settings()),
                ShaderSourceFile::new(TERRAIN_RESOURCE_BINDINGS_PATH, bindings()),
                ShaderSourceFile::new(
                    "world0/deferred1.vsh",
                    "#version 130\nout vec2 uv;\nvoid main() { uv = gl_MultiTexCoord0.xy; gl_Position = ftransform(); }",
                ),
                ShaderSourceFile::new(
                    "world0/deferred1.fsh",
                    "#version 130\n/* DRAWBUFFERS:0 */\nin vec2 uv;\nuniform sampler2D colortex0;\n/* const bool colortex0MipmapEnabled = true; */\nvoid main() { gl_FragData[0] = texture2D(colortex0, uv); }",
                ),
            ],
        )
        .unwrap();
        let bindings = TerrainSourceResourceBindings::from_source(&source).unwrap();
        let manifest = ShaderPackColorTargetManifest::from_source(&source, &bindings).unwrap();
        let identity = ShaderPackColorTargetIdentity::new(
            41,
            source.generation(),
            Extent3d {
                width: 320,
                height: 180,
                depth: 1,
            },
            ["primary".to_string()],
            ["primary".to_string()],
        )
        .unwrap();
        let mut gal = gal();
        let mut cache = ShaderPackColorTargetCache::default();
        let targets = cache.stage(&mut gal, identity, &manifest).unwrap();
        let program = fullscreen_program(&source);

        let resources = prepare_source_color_resources(
            &mut gal,
            program.shader_pack_generation,
            &program.opaque_resource_bindings,
            &ShaderPackColorSamplingPlan::default(),
            &targets,
        )
        .expect("terrain/DH-style source sampling must allocate current named targets");
        let primary = targets.target("primary").unwrap();
        assert_eq!(
            Some(primary.current_view),
            resources.sampled_view_for(TerrainSourceResourceRole::ShaderPackColor(
                "primary".to_string()
            ))
        );
        resources.destroy(&mut gal);
        cache.destroy(&mut gal);
    }

    #[test]
    fn generic_source_color_policy_rejects_unbound_sampler_identity() {
        let source = ShaderPackSource::new(
            "generic-source-policy-rejection",
            7,
            vec![
                ShaderSourceFile::new(PIPELINE_SETTINGS_PATH, settings()),
                ShaderSourceFile::new(TERRAIN_RESOURCE_BINDINGS_PATH, bindings()),
                ShaderSourceFile::new(
                    "world0/deferred1.vsh",
                    "#version 130\nout vec2 uv;\nvoid main() { uv = gl_MultiTexCoord0.xy; gl_Position = ftransform(); }",
                ),
                ShaderSourceFile::new(
                    "world0/deferred1.fsh",
                    "#version 130\n/* DRAWBUFFERS:0 */\nin vec2 uv;\nuniform sampler2D colortex0;\nvoid main() { gl_FragData[0] = texture2D(colortex0, uv); }",
                ),
            ],
        )
        .unwrap();
        let bindings = TerrainSourceResourceBindings::from_source(&source).unwrap();
        let manifest = ShaderPackColorTargetManifest::from_source(&source, &bindings).unwrap();
        let identity = ShaderPackColorTargetIdentity::new(
            41,
            source.generation(),
            Extent3d {
                width: 320,
                height: 180,
                depth: 1,
            },
            std::iter::empty::<String>(),
            std::iter::empty::<String>(),
        )
        .unwrap();
        let mut gal = gal();
        let mut cache = ShaderPackColorTargetCache::default();
        let targets = cache.stage(&mut gal, identity, &manifest).unwrap();
        let program = fullscreen_program(&source);
        let invalid_policy = ShaderPackColorSamplingPlan {
            bindings: BTreeMap::from([(
                (
                    TerrainSourceResourceRole::ShaderPackColor("primary".to_string()),
                    99,
                ),
                SourceColorBinding {
                    feedback: true,
                    mipmapped: false,
                },
            )]),
        };
        assert!(prepare_source_color_resources(
            &mut gal,
            program.shader_pack_generation,
            &program.opaque_resource_bindings,
            &invalid_policy,
            &targets,
        )
        .unwrap_err()
        .to_string()
        .contains("references absent combined sampler"));
        cache.destroy(&mut gal);
    }

    #[test]
    fn source_color_sampler_cache_follows_target_confirmation_and_discard() {
        let source = ShaderPackSource::new(
            "source-color-cache-transaction",
            7,
            vec![
                ShaderSourceFile::new(PIPELINE_SETTINGS_PATH, settings()),
                ShaderSourceFile::new(TERRAIN_RESOURCE_BINDINGS_PATH, bindings()),
                ShaderSourceFile::new(
                    "world0/deferred1.vsh",
                    "#version 130\nout vec2 uv;\nvoid main() { uv = gl_MultiTexCoord0.xy; gl_Position = ftransform(); }",
                ),
                ShaderSourceFile::new(
                    "world0/deferred1.fsh",
                    "#version 130\n/* DRAWBUFFERS:0 */\nin vec2 uv;\nuniform sampler2D colortex0;\nvoid main() { gl_FragData[0] = texture2D(colortex0, uv); }",
                ),
            ],
        )
        .unwrap();
        let bindings = TerrainSourceResourceBindings::from_source(&source).unwrap();
        let manifest = ShaderPackColorTargetManifest::from_source(&source, &bindings).unwrap();
        let extent = Extent3d {
            width: 320,
            height: 180,
            depth: 1,
        };
        let program = fullscreen_program(&source);
        let mut gal = gal();
        let mut target_cache = ShaderPackColorTargetCache::default();
        let mut resource_cache = ShaderPackSourceColorResourceCache::default();

        let first_identity = ShaderPackColorTargetIdentity::new(
            41,
            source.generation(),
            extent,
            std::iter::empty::<String>(),
            std::iter::empty::<String>(),
        )
        .unwrap();
        let first_targets = target_cache
            .stage(&mut gal, first_identity.clone(), &manifest)
            .unwrap();
        resource_cache
            .stage(
                &mut gal,
                program.shader_pack_generation,
                &program.opaque_resource_bindings,
                &ShaderPackColorSamplingPlan::default(),
                &first_targets,
            )
            .unwrap();
        assert_eq!(0, resource_cache.active_len());
        assert_eq!(1, resource_cache.pending_len());
        resource_cache.discard_submission(&mut gal);
        target_cache.discard_submission(&mut gal);
        assert_eq!(0, resource_cache.active_len());
        assert_eq!(0, resource_cache.pending_len());

        let first_targets = target_cache
            .stage(&mut gal, first_identity, &manifest)
            .unwrap();
        resource_cache
            .stage(
                &mut gal,
                program.shader_pack_generation,
                &program.opaque_resource_bindings,
                &ShaderPackColorSamplingPlan::default(),
                &first_targets,
            )
            .unwrap();
        resource_cache.confirm_submission(&mut gal).unwrap();
        target_cache.confirm_submission(&mut gal);
        assert_eq!(1, resource_cache.active_len());

        let replacement_targets = target_cache
            .stage(
                &mut gal,
                ShaderPackColorTargetIdentity::new(
                    42,
                    source.generation(),
                    extent,
                    std::iter::empty::<String>(),
                    std::iter::empty::<String>(),
                )
                .unwrap(),
                &manifest,
            )
            .unwrap();
        resource_cache
            .stage(
                &mut gal,
                program.shader_pack_generation,
                &program.opaque_resource_bindings,
                &ShaderPackColorSamplingPlan::default(),
                &replacement_targets,
            )
            .unwrap();
        resource_cache.confirm_submission(&mut gal).unwrap();
        target_cache.confirm_submission(&mut gal);
        assert_eq!(1, resource_cache.active_len());
        assert_eq!(0, resource_cache.pending_len());

        resource_cache.destroy(&mut gal);
        target_cache.destroy(&mut gal);
    }

    #[test]
    fn terrain_outputs_resolve_named_pack_targets_without_attachment_indices() {
        let source = source(settings());
        let bindings = TerrainSourceResourceBindings::from_source(&source).unwrap();
        let manifest = ShaderPackColorTargetManifest::from_source(&source, &bindings).unwrap();
        let identity = ShaderPackColorTargetIdentity::new(
            41,
            source.generation(),
            Extent3d {
                width: 320,
                height: 180,
                depth: 1,
            },
            std::iter::empty::<String>(),
            std::iter::empty::<String>(),
        )
        .unwrap();
        let mut gal = gal();
        let mut cache = ShaderPackColorTargetCache::default();
        let targets = cache.stage(&mut gal, identity, &manifest).unwrap();

        let attachments = resolve_terrain_source_color_attachments(
            &[
                (TerrainPassOutput::LitTerrainColor, 0),
                (TerrainPassOutput::MaterialAuxiliary, 6),
            ],
            &manifest,
            &targets,
        )
        .expect("named terrain outputs must resolve through staged pack targets");
        assert_eq!(2, attachments.len());
        assert_eq!(TerrainPassOutput::LitTerrainColor, attachments[0].output);
        assert_eq!(0, attachments[0].source_slot);
        assert_eq!(
            TerrainSourceResourceRole::ShaderPackColor("primary".to_string()),
            attachments[0].role
        );
        assert_eq!(
            targets.target("primary").unwrap().current_attachment_view,
            attachments[0].view
        );
        assert_eq!(TerrainPassOutput::MaterialAuxiliary, attachments[1].output);
        assert_eq!(6, attachments[1].source_slot);
        assert_eq!(
            TerrainSourceResourceRole::ShaderPackColor("material_auxiliary".to_string()),
            attachments[1].role
        );
        assert_eq!(
            targets
                .target("material_auxiliary")
                .unwrap()
                .current_attachment_view,
            attachments[1].view
        );

        let duplicate = resolve_terrain_source_color_attachments(
            &[
                (TerrainPassOutput::LitTerrainColor, 0),
                (TerrainPassOutput::MaterialAuxiliary, 0),
            ],
            &manifest,
            &targets,
        )
        .unwrap_err();
        assert!(duplicate
            .to_string()
            .contains("more than one semantic output"));
        cache.destroy(&mut gal);
    }

    #[test]
    fn fullscreen_output_resolution_rejects_a_manifest_program_semantic_mismatch() {
        let source = ShaderPackSource::new(
            "fullscreen-source-target-output-mismatch",
            7,
            vec![
                ShaderSourceFile::new(PIPELINE_SETTINGS_PATH, settings()),
                ShaderSourceFile::new(TERRAIN_RESOURCE_BINDINGS_PATH, bindings()),
                ShaderSourceFile::new(
                    "world0/deferred1.vsh",
                    "#version 130\nout vec2 uv;\nvoid main() { uv = gl_MultiTexCoord0.xy; gl_Position = ftransform(); }",
                ),
                ShaderSourceFile::new(
                    "world0/deferred1.fsh",
                    "#version 130\n/* DRAWBUFFERS:0 */\nin vec2 uv;\nuniform sampler2D colortex0;\nvoid main() { gl_FragData[0] = texture2D(colortex0, uv); }",
                ),
            ],
        )
        .unwrap();
        let bindings = TerrainSourceResourceBindings::from_source(&source).unwrap();
        let mut manifest = ShaderPackColorTargetManifest::from_source(&source, &bindings).unwrap();
        let identity = ShaderPackColorTargetIdentity::new(
            41,
            source.generation(),
            Extent3d {
                width: 320,
                height: 180,
                depth: 1,
            },
            std::iter::empty::<String>(),
            std::iter::empty::<String>(),
        )
        .unwrap();
        let mut gal = gal();
        let mut cache = ShaderPackColorTargetCache::default();
        let targets = cache.stage(&mut gal, identity, &manifest).unwrap();
        manifest.targets.get_mut("primary").unwrap().role =
            TerrainSourceResourceRole::ShaderPackColor("previous_depth".to_string());
        let error = resolve_fullscreen_source_color_attachments(
            &fullscreen_program(&source),
            &manifest,
            &targets,
        )
        .unwrap_err();
        assert!(error.to_string().contains("maps to semantic color"));
        cache.destroy(&mut gal);
    }

    #[test]
    fn fullscreen_color_resources_reject_mip_sampling_without_a_staged_chain() {
        let source = ShaderPackSource::new(
            "fullscreen-source-targets-no-mips",
            7,
            vec![
                ShaderSourceFile::new(PIPELINE_SETTINGS_PATH, settings()),
                ShaderSourceFile::new(TERRAIN_RESOURCE_BINDINGS_PATH, bindings()),
                ShaderSourceFile::new(
                    "world0/deferred1.vsh",
                    "#version 130\nout vec2 uv;\nvoid main() { uv = gl_MultiTexCoord0.xy; gl_Position = ftransform(); }",
                ),
                ShaderSourceFile::new(
                    "world0/deferred1.fsh",
                    "#version 130\n/* DRAWBUFFERS:0 */\nin vec2 uv;\nuniform sampler2D colortex0;\n/* const bool colortex0MipmapEnabled = true; */\nvoid main() { gl_FragData[0] = texture2D(colortex0, uv); }",
                ),
            ],
        )
        .unwrap();
        let bindings = TerrainSourceResourceBindings::from_source(&source).unwrap();
        let manifest = ShaderPackColorTargetManifest::from_source(&source, &bindings).unwrap();
        let identity = ShaderPackColorTargetIdentity::new(
            41,
            source.generation(),
            Extent3d {
                width: 320,
                height: 180,
                depth: 1,
            },
            ["primary".to_string()],
            std::iter::empty::<String>(),
        )
        .unwrap();
        let mut gal = gal();
        let mut cache = ShaderPackColorTargetCache::default();
        let targets = cache.stage(&mut gal, identity, &manifest).unwrap();
        let error = prepare_fullscreen_source_color_resources(
            &mut gal,
            &fullscreen_program(&source),
            &targets,
        )
        .unwrap_err();
        assert!(error.to_string().contains("has no staged mip chain"));
        cache.destroy(&mut gal);
    }

    #[test]
    fn feedback_target_must_be_declared_before_any_private_resource_is_created() {
        let source = source(settings());
        let bindings = TerrainSourceResourceBindings::from_source(&source).unwrap();
        let manifest = ShaderPackColorTargetManifest::from_source(&source, &bindings).unwrap();
        let identity = ShaderPackColorTargetIdentity::new(
            41,
            source.generation(),
            Extent3d {
                width: 320,
                height: 180,
                depth: 1,
            },
            ["absent_target".to_string()],
            std::iter::empty::<String>(),
        )
        .unwrap();
        let mut gal = gal();
        let mut cache = ShaderPackColorTargetCache::default();

        assert!(cache.stage(&mut gal, identity, &manifest).is_err());
        assert_eq!(0, gal.metrics().resource_creates);
    }
}
