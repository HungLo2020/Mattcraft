//! Private semantic frontend for depth-aware world text.
//!
//! This module intentionally knows nothing about Java font renderers, atlas
//! objects, OpenGL texture units, or Vulkan descriptors. It owns copied raw
//! atlas pixels and validates the immutable glyph stream before a later pass
//! writer turns it into explicit GAL resources and commands.

use std::collections::{BTreeMap, BTreeSet};

use super::super::commands::{
    AttachmentLoadOp, AttachmentStoreOp, CommandOp, PassAttachment, TextureOrigin3d,
    TextureUsageState,
};
use super::super::error::{GalError, GalResult, StatusCode};
use super::super::gal::VulkanicGal;
use super::super::handles::Handle;
use super::super::resources::{
    AccessFlags, BlendMode, BufferDesc, BufferUsage, ColorFormat, CompareOp, DepthBias, Extent3d,
    FrontFace, GraphicsPipelineDesc, MemoryDomain, PipelineLayoutDesc, PipelineStageFlags,
    PrimitiveTopology, QueueClass, RasterYDirection, ResourceBinding, ResourceBindingDesc, ResourceBindingKind,
    ResourceLayoutDesc, ResourceSetDesc, SamplerAddressMode, SamplerDesc, SamplerFilter,
    ShaderCodeFormat, ShaderModuleDesc, ShaderStage, TextureDesc, TextureDimension, TextureFormat,
    TextureUsage, TextureViewDesc,
};
use super::super::shader_pack::programs::shader_stage_code_for_backend;
use super::super::{BufferImageCopyRegion, CullMode};

pub(crate) const WORLD_TEXT_IMAGE_ALPHA8: u32 = 1;
pub(crate) const WORLD_TEXT_IMAGE_RGBA8: u32 = 2;
pub(crate) const WORLD_TEXT_DEPTH_SEE_THROUGH: u32 = 1;
pub(crate) const WORLD_TEXT_DEPTH_NORMAL: u32 = 2;
pub(crate) const WORLD_TEXT_DEPTH_POLYGON_OFFSET: u32 = 3;
const MAX_WORLD_TEXT_IMAGE_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const MAX_WORLD_TEXT_IMAGES: usize = 4_096;
pub(crate) const MAX_WORLD_TEXT_IMAGE_BYTES_TOTAL: usize = 64 * 1024 * 1024;
// Each entry owns an upload buffer, texture, descriptors, shaders, and three
// pipelines. Keep revision/format churn from turning the semantic frontend
// into an unbounded GPU-object cache when a caller rotates atlas identities.
const MAX_WORLD_TEXT_RESOURCES: usize = 8_192;
const MAX_WORLD_TEXT_QUADS_PER_FRAME: usize = 65_536;
const MAX_WORLD_TEXT_QUADS_PER_BATCH: usize = 256;
const WORLD_TEXT_HEADER_BYTES: usize = 144;
const WORLD_TEXT_QUAD_BYTES: usize = 176;
const WORLD_TEXT_UNIFORM_BYTES: u64 =
    (WORLD_TEXT_HEADER_BYTES + MAX_WORLD_TEXT_QUADS_PER_BATCH * WORLD_TEXT_QUAD_BYTES) as u64;

const WORLD_TEXT_VERTEX_SHADER: &[u8] = br#"#version 450
struct TextQuad {
    mat4 model_view;
    vec4 p0;
    vec4 p1;
    vec4 p2;
    vec4 p3;
    vec4 uv01;
    vec4 uv23;
    vec4 color;
};
layout(set = 0, binding = 0, std140) uniform WorldTextBatch {
    mat4 projection;
    mat4 view;
    vec4 texture_mode;
    TextQuad quads[256];
};
layout(location = 0) out vec2 v_uv;
layout(location = 1) out vec4 v_color;
layout(location = 2) flat out float v_colored;
const vec2 corners[6] = vec2[6](
    vec2(0.0, 0.0), vec2(0.0, 1.0), vec2(1.0, 1.0),
    vec2(1.0, 1.0), vec2(1.0, 0.0), vec2(0.0, 0.0)
);
void main() {
    TextQuad quad = quads[gl_InstanceIndex];
    vec2 corner = corners[gl_VertexIndex];
    vec3 top = mix(quad.p0.xyz, quad.p3.xyz, corner.x);
    vec3 bottom = mix(quad.p1.xyz, quad.p2.xyz, corner.x);
    vec3 position = mix(top, bottom, corner.y);
    // Semantic glyph corners are TL, BL, BR, TR. The packed pairs preserve
    // that producer order, so reconstruct the horizontal top/bottom edges
    // explicitly instead of treating the first pair as one edge.
    // The copied glyph positions and atlas coordinates are both ordered
    // left-to-right (TL, BL, BR, TR). Preserve that semantic orientation:
    // swapping the U edges mirrors every name tag in the final world frame.
    vec2 uv_top = mix(quad.uv01.xy, quad.uv23.zw, corner.x);
    vec2 uv_bottom = mix(quad.uv01.zw, quad.uv23.xy, corner.x);
    vec4 clip = projection * view * quad.model_view * vec4(position, 1.0);
#ifdef VULKANIC_GAL_ZERO_TO_ONE_CLIP_DEPTH
    clip.z = clip.z * 0.5 + clip.w * 0.5;
#endif
    gl_Position = clip;
    v_uv = mix(uv_top, uv_bottom, corner.y);
    v_color = quad.color;
    v_colored = texture_mode.x;
}
"#;

const WORLD_TEXT_FRAGMENT_SHADER: &[u8] = br#"#version 450
layout(set = 0, binding = 1) uniform texture2D TextAtlas;
layout(set = 0, binding = 2) uniform sampler TextSampler;
layout(location = 0) in vec2 v_uv;
layout(location = 1) in vec4 v_color;
layout(location = 2) flat in float v_colored;
layout(location = 0) out vec4 out_color;
void main() {
    vec4 sampled = texture(sampler2D(TextAtlas, TextSampler), v_uv);
    vec4 glyph = v_colored > 0.5 ? sampled : vec4(1.0, 1.0, 1.0, sampled.r);
    vec4 color = glyph * v_color;
    if (color.a <= (1.0 / 255.0)) discard;
    out_color = color;
}
"#;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorldTextImageFormat {
    Alpha8,
    Rgba8,
}

impl TryFrom<u32> for WorldTextImageFormat {
    type Error = GalError;

    fn try_from(value: u32) -> GalResult<Self> {
        match value {
            WORLD_TEXT_IMAGE_ALPHA8 => Ok(Self::Alpha8),
            WORLD_TEXT_IMAGE_RGBA8 => Ok(Self::Rgba8),
            _ => Err(GalError::ffi(
                StatusCode::UnknownEnum,
                format!("unknown world text image format {value}"),
            )),
        }
    }
}

impl WorldTextImageFormat {
    fn bytes_per_texel(self) -> usize {
        match self {
            Self::Alpha8 => 1,
            Self::Rgba8 => 4,
        }
    }

    fn texture_format(self) -> TextureFormat {
        match self {
            Self::Alpha8 => TextureFormat::R8Unorm,
            Self::Rgba8 => TextureFormat::Rgba8Unorm,
        }
    }

    fn shader_mode(self) -> f32 {
        match self {
            Self::Alpha8 => 0.0,
            Self::Rgba8 => 1.0,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct WorldTextImageAsset {
    pub asset_id: u64,
    pub atlas_generation: u64,
    pub atlas_revision: u64,
    pub format: WorldTextImageFormat,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

#[derive(Clone, Debug)]
pub(crate) struct WorldTextQuadRequest {
    pub asset_id: u64,
    pub atlas_generation: u64,
    pub atlas_revision: u64,
    pub colored: bool,
    pub depth_policy: u32,
    pub packed_light: u32,
    pub distance_to_camera_sq: f64,
    pub model_view_matrix: [f32; 16],
    /// Top-left, bottom-left, bottom-right, top-right local glyph positions.
    pub positions: [[f32; 3]; 4],
    pub uvs: [[f32; 2]; 4],
    pub color_argb: u32,
    pub block_entity_id: i32,
}

#[derive(Clone, Debug)]
pub(crate) struct WorldTextFrame {
    pub quads: Vec<WorldTextQuadRequest>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorldTextBatch {
    pub asset_id: u64,
    pub atlas_generation: u64,
    pub atlas_revision: u64,
    pub colored: bool,
    pub depth_policy: u32,
    /// Kept in the batch key so distinct semantic light values cannot be
    /// coalesced before the text shader consumes the lightmap contract.
    pub packed_light: u32,
    pub block_entity_id: i32,
    pub start: usize,
    pub count: usize,
}

/// Bounded semantic execution evidence for one text submission. This is
/// derived from copied glyph data before lowering and contains no backend
/// object or native presentation state.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct WorldTextFrameStats {
    pub quad_count: u64,
    pub batch_count: u64,
    pub draw_count: u64,
    pub clip_xy_visible_quad_count: u64,
    pub first_ndc_bounds: Option<[f32; 4]>,
    pub first_ndc_corners: Option<[[f32; 2]; 4]>,
    /// First semantic quads in producer order. This is bounded diagnostic
    /// evidence for verifying text-plane handedness without retaining frame
    /// vertex streams or backend state.
    pub ndc_bounds_sample: Vec<[f32; 4]>,
}

#[derive(Default)]
pub(crate) struct WorldTextFrontend {
    asset_generation: u64,
    images: BTreeMap<u64, WorldTextImageAsset>,
    resources: BTreeMap<WorldTextResourceKey, WorldTextResources>,
    /// Multiple producers can append text into one command batch. Pending
    /// atlas uploads therefore live for the whole batch, not one append call.
    upload_submission_active: bool,
    /// Exact resource keys whose upload operations were appended to the
    /// currently active command batch. Confirmation must not commit unrelated
    /// resources staged by another discarded producer/path.
    pending_upload_keys: BTreeSet<WorldTextResourceKey>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct WorldTextResourceKey {
    raster_y_direction: RasterYDirection,
    asset_id: u64,
    atlas_generation: u64,
    atlas_revision: u64,
    color_format: ColorFormat,
}

struct WorldTextResources {
    upload_buffer: Handle,
    uniform_buffer: Handle,
    texture: Handle,
    sampler: Handle,
    vertex_shader: Handle,
    fragment_shader: Handle,
    texture_view: Handle,
    resource_layout: Handle,
    resource_set: Handle,
    pipeline_layout: Handle,
    pipeline_depth_disabled: Handle,
    pipeline_depth_test_no_write: Handle,
    pipeline_depth_test_polygon_offset: Handle,
    /// Set only when the command stream containing the initial texture upload
    /// is admitted for execution. Source-candidate planning may construct the
    /// resource and then discard its commands, so construction alone is not
    /// evidence that the image reached Vulkan.
    uploaded: bool,
    upload_pending: bool,
}

impl WorldTextResources {
    fn handles_in_destroy_order(&self) -> [Handle; 13] {
        [
            self.pipeline_depth_test_polygon_offset,
            self.pipeline_depth_test_no_write,
            self.pipeline_depth_disabled,
            self.pipeline_layout,
            self.resource_set,
            self.resource_layout,
            self.texture_view,
            self.fragment_shader,
            self.vertex_shader,
            self.sampler,
            self.texture,
            self.uniform_buffer,
            self.upload_buffer,
        ]
    }
}

impl WorldTextFrontend {
    /// Commits atlas residency only after the command stream containing its
    /// upload has been accepted by the GAL. Resource construction or command
    /// planning alone must never suppress a required first-use upload.
    pub(crate) fn confirm_submission(&mut self) {
        for key in std::mem::take(&mut self.pending_upload_keys) {
            if let Some(resources) = self.resources.get_mut(&key) {
                resources.uploaded = true;
                resources.upload_pending = false;
            }
        }
        self.upload_submission_active = false;
    }

    pub(crate) fn cancel_submission(&mut self) {
        for key in std::mem::take(&mut self.pending_upload_keys) {
            if let Some(resources) = self.resources.get_mut(&key) {
                if !resources.uploaded {
                    resources.upload_pending = false;
                }
            }
        }
        self.upload_submission_active = false;
    }

    /// Starts an explicit upload transaction for one owning frame producer.
    /// Multiple appenders within that producer share pending keys, while a
    /// previous producer's abandoned markers are discarded before any new
    /// command vector is assembled.
    pub(crate) fn begin_submission(&mut self) {
        if self.upload_submission_active || !self.pending_upload_keys.is_empty() {
            self.cancel_submission();
        } else {
            for resources in self.resources.values_mut() {
                if !resources.uploaded {
                    resources.upload_pending = false;
                }
            }
        }
        self.pending_upload_keys.clear();
        self.upload_submission_active = true;
    }

    /// Atomically installs copied, caller-independent font atlas images.
    pub(crate) fn apply_image_update(
        &mut self,
        generation: u64,
        assets: Vec<WorldTextImageAsset>,
    ) -> GalResult<()> {
        if generation == 0 || generation <= self.asset_generation {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                format!(
                    "stale world text image generation {generation}; current generation is {}",
                    self.asset_generation
                ),
            ));
        }
        if assets.len() > MAX_WORLD_TEXT_IMAGES {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                format!(
                    "world text image asset count {} exceeds {}",
                    assets.len(),
                    MAX_WORLD_TEXT_IMAGES
                ),
            ));
        }
        let mut next = BTreeMap::new();
        let mut total_bytes = 0usize;
        for asset in assets {
            validate_image_asset(&asset)?;
            total_bytes = total_bytes.checked_add(asset.pixels.len()).ok_or_else(|| {
                GalError::ffi(
                    StatusCode::InvalidArgument,
                    "world text image aggregate size overflows",
                )
            })?;
            if total_bytes > MAX_WORLD_TEXT_IMAGE_BYTES_TOTAL {
                return Err(GalError::ffi(
                    StatusCode::InvalidArgument,
                    format!(
                        "world text image assets use {} bytes; maximum is {}",
                        total_bytes, MAX_WORLD_TEXT_IMAGE_BYTES_TOTAL
                    ),
                ));
            }
            if next.insert(asset.asset_id, asset).is_some() {
                return Err(GalError::ffi(
                    StatusCode::InvalidArgument,
                    "duplicate world text image asset id",
                ));
            }
        }
        self.images = next;
        self.asset_generation = generation;
        Ok(())
    }

    pub(crate) fn reset(&mut self, gal: &mut VulkanicGal) {
        self.destroy_resources(gal);
        self.images.clear();
        self.asset_generation = 0;
        self.upload_submission_active = false;
        self.pending_upload_keys.clear();
    }

    /// Validates every semantic glyph and preserves Java's declared order.
    /// Only consecutive compatible quads may share a later draw, so normal and
    /// see-through text never reorder around overlapping world content.
    pub(crate) fn prepare_frame(&self, frame: &WorldTextFrame) -> GalResult<Vec<WorldTextBatch>> {
        if frame.quads.len() > MAX_WORLD_TEXT_QUADS_PER_FRAME {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                format!(
                    "world text frame has {} quads, exceeding {}",
                    frame.quads.len(),
                    MAX_WORLD_TEXT_QUADS_PER_FRAME
                ),
            ));
        }
        let mut batches = Vec::<WorldTextBatch>::new();
        for (index, quad) in frame.quads.iter().enumerate() {
            validate_quad(quad, self.images.get(&quad.asset_id))?;
            let can_extend = batches.last().is_some_and(|batch| {
                batch.asset_id == quad.asset_id
                    && batch.atlas_generation == quad.atlas_generation
                    && batch.atlas_revision == quad.atlas_revision
                    && batch.colored == quad.colored
                    && batch.depth_policy == quad.depth_policy
                    && batch.packed_light == quad.packed_light
                    && batch.block_entity_id == quad.block_entity_id
                    && batch.start + batch.count == index
            });
            if can_extend {
                batches.last_mut().expect("checked above").count += 1;
            } else {
                batches.push(WorldTextBatch {
                    asset_id: quad.asset_id,
                    atlas_generation: quad.atlas_generation,
                    atlas_revision: quad.atlas_revision,
                    colored: quad.colored,
                    depth_policy: quad.depth_policy,
                    packed_light: quad.packed_light,
                    block_entity_id: quad.block_entity_id,
                    start: index,
                    count: 1,
                });
            }
        }
        Ok(batches)
    }

    /// Writes all world text through the caller's already-owned frame pass.
    /// Image upload ops are appended to that same combined frame submission;
    /// no Java font buffer or secondary presenter participates.
    pub(crate) fn append_frame_ops(
        &mut self,
        gal: &mut VulkanicGal,
        frame_target: Handle,
        pass: Handle,
        color_attachment: Handle,
        depth_texture: Handle,
        depth_view: Handle,
        depth_before: TextureUsageState,
        color_format: ColorFormat,
        raster_y_direction: RasterYDirection,
        view_matrix: [f32; 16],
        projection_matrix: [f32; 16],
        quads: &[WorldTextQuadRequest],
        ops: &mut Vec<CommandOp>,
        _mark_uploaded: bool,
    ) -> GalResult<WorldTextFrameStats> {
        if quads.is_empty() {
            return Ok(WorldTextFrameStats::default());
        }
        // Reset abandoned markers once, at the start of a combined batch.
        // Subsequent appenders in that batch observe the pending marker and
        // do not duplicate the same atlas upload.
        if !self.upload_submission_active {
            self.begin_submission();
        }
        self.prune_stale_resources(gal);
        let batches = self.prepare_frame(&WorldTextFrame {
            quads: quads.to_vec(),
        })?;
        let mut stats = WorldTextFrameStats {
            quad_count: quads.len() as u64,
            batch_count: batches.len() as u64,
            ..WorldTextFrameStats::default()
        };
        for quad in quads {
            let corners = quad_ndc_corners(view_matrix, projection_matrix, quad);
            let bounds = corners.map(ndc_corners_bounds);
            if stats.first_ndc_bounds.is_none() {
                stats.first_ndc_bounds = bounds;
                stats.first_ndc_corners = corners;
            }
            if let Some(bounds) = bounds.filter(|_| stats.ndc_bounds_sample.len() < 6) {
                stats.ndc_bounds_sample.push(bounds);
            }
            if bounds.is_some_and(ndc_bounds_intersect_viewport) {
                stats.clip_xy_visible_quad_count += 1;
            }
        }
        let mut depth_transitioned = false;
        for batch in batches {
            let image = self
                .images
                .get(&batch.asset_id)
                .ok_or_else(|| {
                    GalError::backend("world text image vanished after semantic validation")
                })?
                .clone();
            let key = WorldTextResourceKey {
                raster_y_direction,
                asset_id: batch.asset_id,
                atlas_generation: batch.atlas_generation,
                atlas_revision: batch.atlas_revision,
                color_format,
            };
            let upload = if self
                .resources
                .get(&key)
                .is_some_and(|resource| resource.uploaded || resource.upload_pending)
            {
                None
            } else if let Some(resources) = self.resources.get_mut(&key) {
                resources.upload_pending = true;
                Some((None, upload_ops_for_resources(resources, &image)?))
            } else {
                if self.resources.len() >= MAX_WORLD_TEXT_RESOURCES {
                    return Err(GalError::unsupported_feature(format!(
                        "world text resource cache exceeds bounded limit {}",
                        MAX_WORLD_TEXT_RESOURCES
                    )));
                }
                let (resources, upload_ops) = self.create_resources(gal, key, &image)?;
                Some((Some(resources), upload_ops))
            };
            if let Some((resources, upload_ops)) = upload {
                if let Some(mut resources) = resources {
                    resources.upload_pending = true;
                    self.resources.insert(key, resources);
                }
                self.pending_upload_keys.insert(key);
                ops.extend(upload_ops);
            }
            // Upload residency is committed only by `confirm_submission` after
            // the enclosing GAL submission succeeds.  Marking it here while
            // merely constructing command operations can leave a resource
            // permanently treated as resident when a later outer-stage error
            // discards those operations, causing a subsequent Vulkan draw to
            // sample an image that is still in UNDEFINED layout.
            let resources = self
                .resources
                .get(&key)
                .ok_or_else(|| GalError::backend("world text resources missing after creation"))?;
            for chunk in
                quads[batch.start..batch.start + batch.count].chunks(MAX_WORLD_TEXT_QUADS_PER_BATCH)
            {
                let uniforms =
                    packed_uniforms(view_matrix, projection_matrix, image.format, chunk)?;
                ops.push(CommandOp::Barrier(buffer_barrier(
                    resources.uniform_buffer,
                    TextureUsageState::ShaderRead,
                    TextureUsageState::TransferDst,
                )));
                ops.push(CommandOp::HostWriteBuffer {
                    buffer: resources.uniform_buffer,
                    offset: 0,
                    data: uniforms,
                });
                ops.push(CommandOp::Barrier(buffer_barrier(
                    resources.uniform_buffer,
                    TextureUsageState::TransferDst,
                    TextureUsageState::ShaderRead,
                )));
                if !depth_transitioned && depth_before != TextureUsageState::DepthStencilAttachment
                {
                    // The complete terrain graph leaves its geometry depth
                    // readable by its composite stages. This text pass uses
                    // the same resource as a read-only depth attachment, so
                    // make the cross-pass dependency explicit before binding
                    // it again. Direct primitive frames already leave their
                    // compatibility depth attachment in this state.
                    ops.push(CommandOp::Barrier(texture_barrier(
                        depth_texture,
                        depth_before,
                        TextureUsageState::DepthStencilAttachment,
                    )));
                    depth_transitioned = true;
                }
                ops.push(CommandOp::BeginPass {
                    pass,
                    target: frame_target,
                    colors: vec![PassAttachment {
                        view: color_attachment,
                        load_op: AttachmentLoadOp::Load,
                        store_op: AttachmentStoreOp::Store,
                        clear_color: None,
                    }],
                    depth_stencil: Some(PassAttachment {
                        view: depth_view,
                        load_op: AttachmentLoadOp::Load,
                        store_op: AttachmentStoreOp::Store,
                        clear_color: None,
                    }),
                });
                let pipeline = match batch.depth_policy {
                    WORLD_TEXT_DEPTH_SEE_THROUGH => resources.pipeline_depth_disabled,
                    WORLD_TEXT_DEPTH_NORMAL => resources.pipeline_depth_test_no_write,
                    WORLD_TEXT_DEPTH_POLYGON_OFFSET => resources.pipeline_depth_test_polygon_offset,
                    _ => unreachable!("validated world text depth policy"),
                };
                ops.push(CommandOp::BindGraphicsPipeline(pipeline));
                ops.push(CommandOp::BindResourceSet {
                    pipeline_layout: resources.pipeline_layout,
                    set_index: 0,
                    set: resources.resource_set,
                    dynamic_offsets: Vec::new(),
                });
                ops.push(CommandOp::Draw {
                    vertices: 6,
                    instances: chunk.len() as u32,
                });
                stats.draw_count += 1;
                ops.push(CommandOp::EndPass);
            }
        }
        // The marker is retained in the call signature for the caller's
        // transaction semantics;
        // residency is always committed by `confirm_submission` after GAL
        // accepts the complete enclosing submission.
        Ok(stats)
    }

    fn create_resources(
        &self,
        gal: &mut VulkanicGal,
        key: WorldTextResourceKey,
        image: &WorldTextImageAsset,
    ) -> GalResult<(WorldTextResources, Vec<CommandOp>)> {
        let label = format!(
            "world-text-asset{}-atlas{}-rev{}",
            key.asset_id, key.atlas_generation, key.atlas_revision
        );
        let mut created = Vec::new();
        let result = (|| -> GalResult<WorldTextResources> {
            let upload_buffer = gal.create_buffer(BufferDesc {
                label: format!("{label}.upload"),
                size: image.pixels.len() as u64,
                memory: MemoryDomain::Upload,
                usages: vec![
                    BufferUsage::TransferSrc,
                    BufferUsage::TransferDst,
                    BufferUsage::HostWrite,
                ],
            })?;
            created.push(upload_buffer);
            let uniform_buffer = gal.create_buffer(BufferDesc {
                label: format!("{label}.uniform"),
                size: WORLD_TEXT_UNIFORM_BYTES,
                memory: MemoryDomain::Upload,
                usages: vec![
                    BufferUsage::Uniform,
                    BufferUsage::TransferDst,
                    BufferUsage::HostWrite,
                ],
            })?;
            created.push(uniform_buffer);
            let texture = gal.create_texture(TextureDesc {
                label: format!("{label}.texture"),
                dimension: TextureDimension::D2,
                format: image.format.texture_format(),
                extent: Extent3d {
                    width: image.width,
                    height: image.height,
                    depth: 1,
                },
                mip_levels: 1,
                array_layers: 1,
                usages: vec![TextureUsage::Sampled, TextureUsage::TransferDst],
            })?;
            created.push(texture);
            let sampler = gal.create_sampler(SamplerDesc {
                label: format!("{label}.sampler"),
                min_filter: SamplerFilter::Nearest,
                mag_filter: SamplerFilter::Nearest,
                mip_filter: SamplerFilter::Nearest,
                address_u: SamplerAddressMode::ClampToEdge,
                address_v: SamplerAddressMode::ClampToEdge,
                address_w: SamplerAddressMode::ClampToEdge,
                comparison: None,
            })?;
            created.push(sampler);
            let vertex_shader = gal.create_shader_module(ShaderModuleDesc {
                label: format!("{label}.vertex"),
                stage: ShaderStage::Vertex,
                code_format: ShaderCodeFormat::Glsl,
                code: shader_stage_code_for_backend(
                    gal.capabilities().api,
                    std::str::from_utf8(WORLD_TEXT_VERTEX_SHADER)
                        .expect("world text shader is UTF-8"),
                ),
                entry_point: "main".to_string(),
            })?;
            created.push(vertex_shader);
            let fragment_shader = gal.create_shader_module(ShaderModuleDesc {
                label: format!("{label}.fragment"),
                stage: ShaderStage::Fragment,
                code_format: ShaderCodeFormat::Glsl,
                code: shader_stage_code_for_backend(
                    gal.capabilities().api,
                    std::str::from_utf8(WORLD_TEXT_FRAGMENT_SHADER)
                        .expect("world text shader is UTF-8"),
                ),
                entry_point: "main".to_string(),
            })?;
            created.push(fragment_shader);
            let texture_view = gal.create_texture_view(TextureViewDesc {
                label: format!("{label}.view"),
                texture,
                format: image.format.texture_format(),
                base_mip: 0,
                mip_count: 1,
                base_layer: 0,
                layer_count: 1,
            })?;
            created.push(texture_view);
            let resource_layout = gal.create_resource_layout(ResourceLayoutDesc {
                label: format!("{label}.layout"),
                bindings: vec![
                    ResourceBindingDesc {
                        binding: 0,
                        kind: ResourceBindingKind::UniformBuffer,
                        stages: PipelineStageFlags::DRAW,
                        array_count: 1,
                        optional: false,
                        dynamic_offset_count: 0,
                    },
                    ResourceBindingDesc {
                        binding: 1,
                        kind: ResourceBindingKind::SampledTexture,
                        stages: PipelineStageFlags::DRAW,
                        array_count: 1,
                        optional: false,
                        dynamic_offset_count: 0,
                    },
                    ResourceBindingDesc {
                        binding: 2,
                        kind: ResourceBindingKind::Sampler,
                        stages: PipelineStageFlags::DRAW,
                        array_count: 1,
                        optional: false,
                        dynamic_offset_count: 0,
                    },
                ],
            })?;
            created.push(resource_layout);
            let resource_set = gal.create_resource_set(ResourceSetDesc {
                label: format!("{label}.set"),
                layout: resource_layout,
                bindings: vec![
                    ResourceBinding {
                        binding: 0,
                        array_index: 0,
                        resource: uniform_buffer,
                        kind: ResourceBindingKind::UniformBuffer,
                        access: AccessFlags::READ,
                        dynamic_offsets: Vec::new(),
                        buffer_range: None,
                    },
                    ResourceBinding {
                        binding: 1,
                        array_index: 0,
                        resource: texture_view,
                        kind: ResourceBindingKind::SampledTexture,
                        access: AccessFlags::READ,
                        dynamic_offsets: Vec::new(),
                        buffer_range: None,
                    },
                    ResourceBinding {
                        binding: 2,
                        array_index: 0,
                        resource: sampler,
                        kind: ResourceBindingKind::Sampler,
                        access: AccessFlags::READ,
                        dynamic_offsets: Vec::new(),
                        buffer_range: None,
                    },
                ],
            })?;
            created.push(resource_set);
            let pipeline_layout = gal.create_pipeline_layout(PipelineLayoutDesc {
                label: format!("{label}.pipeline-layout"),
                resource_layouts: vec![resource_layout],
            })?;
            created.push(pipeline_layout);
            let mut build_pipeline = |depth_compare, depth_bias, label_suffix: &str| {
                gal.create_graphics_pipeline(GraphicsPipelineDesc {
                    label: format!("{label}.pipeline.{label_suffix}"),
                    layout: pipeline_layout,
                    vertex_shader,
                    fragment_shader,
                    topology: PrimitiveTopology::Triangles,
                    cull_mode: CullMode::None,
                    front_face: FrontFace::CounterClockwise,
                    provoking_vertex: crate::render::vulkanic::resources::ProvokingVertex::Last,
                    raster_y_direction: key.raster_y_direction,
                    blend: BlendMode::Alpha,
                    depth_compare,
                    depth_write: false,
                    depth_bias,
                    color_formats: vec![key.color_format],
                    depth_format: Some(TextureFormat::Depth32Float),
                    stencil: None,
                })
            };
            let pipeline_depth_disabled = build_pipeline(None, None, "see-through")?;
            created.push(pipeline_depth_disabled);
            let pipeline_depth_test_no_write =
                build_pipeline(Some(CompareOp::LessOrEqual), None, "normal")?;
            created.push(pipeline_depth_test_no_write);
            // Minecraft declares `withDepthBias(-1.0, -10.0)` for polygon-offset
            // text: slope first, then constant. The semantic contract stores the
            // two terms by meaning rather than by backend argument order.
            let pipeline_depth_test_polygon_offset = build_pipeline(
                Some(CompareOp::LessOrEqual),
                Some(DepthBias::new(-10.0, -1.0)),
                "polygon-offset",
            )?;
            created.push(pipeline_depth_test_polygon_offset);
            Ok(WorldTextResources {
                upload_buffer,
                uniform_buffer,
                texture,
                sampler,
                vertex_shader,
                fragment_shader,
                texture_view,
                resource_layout,
                resource_set,
                pipeline_layout,
                pipeline_depth_disabled,
                pipeline_depth_test_no_write,
                pipeline_depth_test_polygon_offset,
                uploaded: false,
                upload_pending: false,
            })
        })();
        match result {
            Ok(resources) => {
                let upload_ops = vec![
                    CommandOp::HostWriteBuffer {
                        buffer: resources.upload_buffer,
                        offset: 0,
                        data: image.pixels.clone(),
                    },
                    CommandOp::Barrier(buffer_barrier(
                        resources.upload_buffer,
                        TextureUsageState::TransferDst,
                        TextureUsageState::TransferSrc,
                    )),
                    CommandOp::Barrier(texture_barrier(
                        resources.texture,
                        TextureUsageState::Undefined,
                        TextureUsageState::TransferDst,
                    )),
                    CommandOp::CopyBufferToTexture(BufferImageCopyRegion {
                        buffer: resources.upload_buffer,
                        buffer_offset: 0,
                        bytes_per_row: image
                            .width
                            .checked_mul(image.format.bytes_per_texel() as u32)
                            .ok_or_else(|| {
                                GalError::invalid_argument("world text row pitch overflows")
                            })?,
                        rows_per_image: image.height,
                        texture: resources.texture,
                        texture_mip: 0,
                        texture_layer: 0,
                        texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
                        extent: Extent3d {
                            width: image.width,
                            height: image.height,
                            depth: 1,
                        },
                    }),
                    CommandOp::Barrier(texture_barrier(
                        resources.texture,
                        TextureUsageState::TransferDst,
                        TextureUsageState::ShaderRead,
                    )),
                ];
                Ok((resources, upload_ops))
            }
            Err(error) => {
                for handle in created.into_iter().rev() {
                    let _ = gal.destroy(handle);
                }
                Err(error)
            }
        }
    }

    fn destroy_resources(&mut self, gal: &mut VulkanicGal) {
        for (_, resources) in std::mem::take(&mut self.resources) {
            for handle in resources.handles_in_destroy_order() {
                let _ = gal.destroy(handle);
            }
        }
    }

    fn prune_stale_resources(&mut self, gal: &mut VulkanicGal) {
        let stale = self
            .resources
            .keys()
            .filter(|key| {
                !self.images.get(&key.asset_id).is_some_and(|image| {
                    image.atlas_generation == key.atlas_generation
                        && image.atlas_revision == key.atlas_revision
                })
            })
            .copied()
            .collect::<Vec<_>>();
        for key in stale {
            if let Some(resources) = self.resources.remove(&key) {
                for handle in resources.handles_in_destroy_order() {
                    let _ = gal.destroy(handle);
                }
            }
        }
    }
}

fn upload_ops_for_resources(
    resources: &WorldTextResources,
    image: &WorldTextImageAsset,
) -> GalResult<Vec<CommandOp>> {
    let bytes_per_row = image
        .width
        .checked_mul(image.format.bytes_per_texel() as u32)
        .ok_or_else(|| GalError::invalid_argument("world text row pitch overflows"))?;
    Ok(vec![
        CommandOp::HostWriteBuffer {
            buffer: resources.upload_buffer,
            offset: 0,
            data: image.pixels.clone(),
        },
        CommandOp::Barrier(buffer_barrier(
            resources.upload_buffer,
            TextureUsageState::TransferDst,
            TextureUsageState::TransferSrc,
        )),
        CommandOp::Barrier(texture_barrier(
            resources.texture,
            TextureUsageState::Undefined,
            TextureUsageState::TransferDst,
        )),
        CommandOp::CopyBufferToTexture(BufferImageCopyRegion {
            buffer: resources.upload_buffer,
            buffer_offset: 0,
            bytes_per_row,
            rows_per_image: image.height,
            texture: resources.texture,
            texture_mip: 0,
            texture_layer: 0,
            texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
            extent: Extent3d {
                width: image.width,
                height: image.height,
                depth: 1,
            },
        }),
        CommandOp::Barrier(texture_barrier(
            resources.texture,
            TextureUsageState::TransferDst,
            TextureUsageState::ShaderRead,
        )),
    ])
}

fn packed_uniforms(
    view_matrix: [f32; 16],
    projection_matrix: [f32; 16],
    format: WorldTextImageFormat,
    quads: &[WorldTextQuadRequest],
) -> GalResult<Vec<u8>> {
    if quads.is_empty() || quads.len() > MAX_WORLD_TEXT_QUADS_PER_BATCH {
        return Err(GalError::invalid_argument(
            "world text batch has invalid quad count",
        ));
    }
    let mut out = Vec::with_capacity(WORLD_TEXT_HEADER_BYTES + quads.len() * WORLD_TEXT_QUAD_BYTES);
    for value in projection_matrix {
        push_f32(&mut out, value);
    }
    for value in view_matrix {
        push_f32(&mut out, value);
    }
    push_f32(&mut out, format.shader_mode());
    push_f32(&mut out, 0.0);
    push_f32(&mut out, 0.0);
    push_f32(&mut out, 0.0);
    for quad in quads {
        for value in quad.model_view_matrix {
            push_f32(&mut out, value);
        }
        for position in quad.positions {
            for value in position {
                push_f32(&mut out, value);
            }
            push_f32(&mut out, 1.0);
        }
        for uv in [quad.uvs[0], quad.uvs[1], quad.uvs[2], quad.uvs[3]] {
            push_f32(&mut out, uv[0]);
            push_f32(&mut out, uv[1]);
        }
        for value in argb_to_rgba(quad.color_argb) {
            push_f32(&mut out, value);
        }
    }
    Ok(out)
}

fn argb_to_rgba(argb: u32) -> [f32; 4] {
    [
        ((argb >> 16) & 0xff) as f32 / 255.0,
        ((argb >> 8) & 0xff) as f32 / 255.0,
        (argb & 0xff) as f32 / 255.0,
        ((argb >> 24) & 0xff) as f32 / 255.0,
    ]
}

fn push_f32(out: &mut Vec<u8>, value: f32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn quad_ndc_bounds(
    view_matrix: [f32; 16],
    projection_matrix: [f32; 16],
    quad: &WorldTextQuadRequest,
) -> Option<[f32; 4]> {
    quad_ndc_corners(view_matrix, projection_matrix, quad).map(ndc_corners_bounds)
}

fn quad_ndc_corners(
    view_matrix: [f32; 16],
    projection_matrix: [f32; 16],
    quad: &WorldTextQuadRequest,
) -> Option<[[f32; 2]; 4]> {
    let mut corners = [[0.0; 2]; 4];
    for (index, position) in quad.positions.into_iter().enumerate() {
        let model = transform_column_major(
            quad.model_view_matrix,
            [position[0], position[1], position[2], 1.0],
        );
        let view = transform_column_major(view_matrix, model);
        let clip = transform_column_major(projection_matrix, view);
        if !clip.into_iter().all(f32::is_finite) || clip[3] <= 1.0e-6 {
            return None;
        }
        corners[index] = [clip[0] / clip[3], clip[1] / clip[3]];
    }
    Some(corners)
}

fn ndc_corners_bounds(corners: [[f32; 2]; 4]) -> [f32; 4] {
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for [x, y] in corners {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    [min_x, min_y, max_x, max_y]
}

fn ndc_bounds_intersect_viewport(bounds: [f32; 4]) -> bool {
    bounds[0] <= 1.0 && bounds[2] >= -1.0 && bounds[1] <= 1.0 && bounds[3] >= -1.0
}

fn transform_column_major(matrix: [f32; 16], point: [f32; 4]) -> [f32; 4] {
    [
        matrix[0] * point[0] + matrix[4] * point[1] + matrix[8] * point[2] + matrix[12] * point[3],
        matrix[1] * point[0] + matrix[5] * point[1] + matrix[9] * point[2] + matrix[13] * point[3],
        matrix[2] * point[0] + matrix[6] * point[1] + matrix[10] * point[2] + matrix[14] * point[3],
        matrix[3] * point[0] + matrix[7] * point[1] + matrix[11] * point[2] + matrix[15] * point[3],
    ]
}

fn buffer_barrier(
    buffer: Handle,
    before: TextureUsageState,
    after: TextureUsageState,
) -> super::super::commands::ResourceBarrier {
    super::super::commands::ResourceBarrier {
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
) -> super::super::commands::ResourceBarrier {
    super::super::commands::ResourceBarrier {
        resource: texture,
        subresources: None,
        before,
        after,
        src_queue: QueueClass::Graphics,
        dst_queue: QueueClass::Graphics,
    }
}

fn validate_image_asset(asset: &WorldTextImageAsset) -> GalResult<()> {
    if asset.asset_id == 0 || asset.atlas_generation == 0 || asset.atlas_revision == 0 {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "world text image asset requires non-zero identity, generation, and revision",
        ));
    }
    if asset.width == 0 || asset.height == 0 {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "world text image asset has zero extent",
        ));
    }
    let expected = usize::try_from(asset.width)
        .ok()
        .and_then(|width| {
            usize::try_from(asset.height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|texels| texels.checked_mul(asset.format.bytes_per_texel()))
        .ok_or_else(|| {
            GalError::ffi(
                StatusCode::InvalidArgument,
                "world text image size overflows",
            )
        })?;
    if expected > MAX_WORLD_TEXT_IMAGE_BYTES || asset.pixels.len() != expected {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!(
                "world text image asset has {} bytes; expected {expected} bytes within {}",
                asset.pixels.len(),
                MAX_WORLD_TEXT_IMAGE_BYTES
            ),
        ));
    }
    Ok(())
}

fn validate_quad(
    quad: &WorldTextQuadRequest,
    image: Option<&WorldTextImageAsset>,
) -> GalResult<()> {
    let image = image.ok_or_else(|| {
        GalError::ffi(
            StatusCode::InvalidArgument,
            format!("unknown world text image asset {}", quad.asset_id),
        )
    })?;
    if image.atlas_generation != quad.atlas_generation
        || image.atlas_revision != quad.atlas_revision
    {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!(
                "world text quad references stale atlas asset {}",
                quad.asset_id
            ),
        ));
    }
    if quad.colored != matches!(image.format, WorldTextImageFormat::Rgba8) {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!(
                "world text quad color mode conflicts with atlas asset {}",
                quad.asset_id
            ),
        ));
    }
    if !matches!(
        quad.depth_policy,
        WORLD_TEXT_DEPTH_SEE_THROUGH | WORLD_TEXT_DEPTH_NORMAL | WORLD_TEXT_DEPTH_POLYGON_OFFSET
    ) {
        return Err(GalError::ffi(
            StatusCode::UnknownEnum,
            format!("unknown world text depth policy {}", quad.depth_policy),
        ));
    }
    if !quad.distance_to_camera_sq.is_finite() || quad.distance_to_camera_sq < 0.0 {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "world text distance must be finite and non-negative",
        ));
    }
    let finite = quad
        .model_view_matrix
        .iter()
        .chain(quad.positions.iter().flatten())
        .chain(quad.uvs.iter().flatten())
        .all(|value| value.is_finite());
    if !finite {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "world text quad contains non-finite geometry",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::vulkanic::backends::{mock::MockBackend, vulkan_capabilities};
    use crate::render::vulkanic::resources::{FrameTargetDesc, RenderPassDesc};
    use crate::render::vulkanic::{CommandList, CommandListDesc, SubmissionBatch};

    fn asset() -> WorldTextImageAsset {
        WorldTextImageAsset {
            asset_id: 7,
            atlas_generation: 3,
            atlas_revision: 4,
            format: WorldTextImageFormat::Alpha8,
            width: 2,
            height: 2,
            pixels: vec![0, 64, 128, 255],
        }
    }

    fn quad(depth_policy: u32) -> WorldTextQuadRequest {
        WorldTextQuadRequest {
            asset_id: 7,
            atlas_generation: 3,
            atlas_revision: 4,
            colored: false,
            depth_policy,
            packed_light: 0,
            block_entity_id: -1,
            distance_to_camera_sq: 4.0,
            model_view_matrix: [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
            positions: [
                [0.0, 0.0, 0.0],
                [0.0, 8.0, 0.0],
                [8.0, 8.0, 0.0],
                [8.0, 0.0, 0.0],
            ],
            uvs: [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]],
            color_argb: 0xffff_ffff,
        }
    }

    #[test]
    fn rejects_stale_or_malformed_image_generations() {
        let mut frontend = WorldTextFrontend::default();
        frontend.apply_image_update(1, vec![asset()]).unwrap();
        assert!(frontend.apply_image_update(1, vec![asset()]).is_err());
        let mut malformed = asset();
        malformed.pixels.pop();
        assert!(frontend.apply_image_update(2, vec![malformed]).is_err());
        assert_eq!(1, frontend.asset_generation);
    }

    #[test]
    fn rejects_excessive_world_text_image_count_before_copying_assets() {
        let mut frontend = WorldTextFrontend::default();
        let assets = (0..=MAX_WORLD_TEXT_IMAGES)
            .map(|index| WorldTextImageAsset {
                asset_id: index as u64 + 1,
                ..asset()
            })
            .collect();
        let error = frontend.apply_image_update(1, assets).unwrap_err();
        assert!(error.to_string().contains("image asset count"));
        assert_eq!(0, frontend.asset_generation);
    }

    #[test]
    fn rejects_excessive_world_text_image_bytes_before_installing_generation() {
        let mut frontend = WorldTextFrontend::default();
        let bytes_per_asset = 1024 * 1024;
        let assets = (0..65)
            .map(|index| WorldTextImageAsset {
                asset_id: index as u64 + 1,
                width: 1024,
                height: 1024,
                pixels: vec![0; bytes_per_asset],
                ..asset()
            })
            .collect();
        let error = frontend.apply_image_update(1, assets).unwrap_err();
        assert!(error.to_string().contains("image assets use"));
        assert_eq!(0, frontend.asset_generation);
    }

    #[test]
    fn vertex_shader_preserves_semantic_left_to_right_glyph_uvs() {
        let shader = std::str::from_utf8(WORLD_TEXT_VERTEX_SHADER).unwrap();
        assert!(shader.contains("vec2 uv_top = mix(quad.uv01.xy, quad.uv23.zw, corner.x);"));
        assert!(shader.contains("vec2 uv_bottom = mix(quad.uv01.zw, quad.uv23.xy, corner.x);"));
        assert!(!shader.contains("mix(quad.uv23.zw, quad.uv01.xy, corner.x)"));
    }

    #[test]
    fn preserves_depth_and_submission_order_when_batching() {
        let mut frontend = WorldTextFrontend::default();
        frontend.apply_image_update(1, vec![asset()]).unwrap();
        let frame = WorldTextFrame {
            quads: vec![
                quad(WORLD_TEXT_DEPTH_SEE_THROUGH),
                quad(WORLD_TEXT_DEPTH_SEE_THROUGH),
                quad(WORLD_TEXT_DEPTH_NORMAL),
                quad(WORLD_TEXT_DEPTH_POLYGON_OFFSET),
            ],
        };
        let batches = frontend.prepare_frame(&frame).unwrap();
        assert_eq!(3, batches.len());
        assert_eq!(2, batches[0].count);
        assert_eq!(WORLD_TEXT_DEPTH_NORMAL, batches[1].depth_policy);
        assert_eq!(WORLD_TEXT_DEPTH_POLYGON_OFFSET, batches[2].depth_policy);
    }

    #[test]
    fn preserves_packed_light_boundaries_when_batching() {
        let mut frontend = WorldTextFrontend::default();
        frontend.apply_image_update(1, vec![asset()]).unwrap();
        let mut lit = quad(WORLD_TEXT_DEPTH_NORMAL);
        lit.packed_light = 0x00f0_00f0;
        let mut dim = lit.clone();
        dim.packed_light = 0x0010_0010;
        let batches = frontend
            .prepare_frame(&WorldTextFrame {
                quads: vec![lit, dim],
            })
            .unwrap();
        assert_eq!(2, batches.len());
        assert_eq!(0x00f0_00f0, batches[0].packed_light);
        assert_eq!(0x0010_0010, batches[1].packed_light);
    }

    #[test]
    fn preserves_block_entity_boundaries_when_batching() {
        let mut frontend = WorldTextFrontend::default();
        frontend.apply_image_update(1, vec![asset()]).unwrap();
        let mut first = quad(WORLD_TEXT_DEPTH_NORMAL);
        first.block_entity_id = 11;
        let mut second = first.clone();
        second.block_entity_id = 12;
        let batches = frontend
            .prepare_frame(&WorldTextFrame {
                quads: vec![first, second],
            })
            .unwrap();
        assert_eq!(2, batches.len());
        assert_eq!(11, batches[0].block_entity_id);
        assert_eq!(12, batches[1].block_entity_id);
    }

    #[test]
    fn rejects_stale_atlas_references_before_draw_planning() {
        let mut frontend = WorldTextFrontend::default();
        frontend.apply_image_update(1, vec![asset()]).unwrap();
        let mut stale = quad(WORLD_TEXT_DEPTH_NORMAL);
        stale.atlas_revision = 5;
        assert!(
            frontend
                .prepare_frame(&WorldTextFrame { quads: vec![stale] })
                .is_err()
        );
    }

    #[test]
    fn packs_the_global_view_matrix_between_projection_and_text_quads() {
        let mut view = [0.0; 16];
        view[0] = 1.0;
        view[5] = 1.0;
        view[10] = 1.0;
        view[12] = 7.0;
        view[15] = 1.0;
        let mut projection = [0.0; 16];
        projection[0] = 2.0;
        projection[5] = 3.0;
        projection[10] = 4.0;
        projection[15] = 1.0;
        let packed = packed_uniforms(
            view,
            projection,
            WorldTextImageFormat::Alpha8,
            &[quad(WORLD_TEXT_DEPTH_NORMAL)],
        )
        .unwrap();
        assert_eq!(
            WORLD_TEXT_HEADER_BYTES + WORLD_TEXT_QUAD_BYTES,
            packed.len()
        );
        let f32_at =
            |offset: usize| f32::from_le_bytes(packed[offset..offset + 4].try_into().unwrap());
        assert_eq!(2.0, f32_at(0));
        assert_eq!(7.0, f32_at(64 + 12 * 4));
        assert_eq!(0.0, f32_at(128));
    }

    #[test]
    fn samples_the_copied_font_region_with_the_readable_local_u_orientation() {
        let shader = std::str::from_utf8(WORLD_TEXT_VERTEX_SHADER).unwrap();
        assert!(shader.contains("mix(quad.uv01.xy, quad.uv23.zw, corner.x)"));
        assert!(shader.contains("mix(quad.uv01.zw, quad.uv23.xy, corner.x)"));
    }

    #[test]
    fn classifies_projected_glyph_bounds_without_backend_state() {
        let identity = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        let mut visible = quad(WORLD_TEXT_DEPTH_SEE_THROUGH);
        visible.positions = [
            [-0.5, 0.5, 0.0],
            [-0.5, -0.5, 0.0],
            [0.5, -0.5, 0.0],
            [0.5, 0.5, 0.0],
        ];
        assert_eq!(
            Some([-0.5, -0.5, 0.5, 0.5]),
            quad_ndc_bounds(identity, identity, &visible),
        );
        assert!(ndc_bounds_intersect_viewport(
            quad_ndc_bounds(identity, identity, &visible).unwrap()
        ));

        visible.model_view_matrix[12] = 3.0;
        assert!(!ndc_bounds_intersect_viewport(
            quad_ndc_bounds(identity, identity, &visible).unwrap()
        ));
    }

    #[test]
    fn name_tag_projection_includes_the_copied_global_view() {
        let identity = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        let mut global_view = identity;
        global_view[12] = 8.0;
        let mut glyph = quad(WORLD_TEXT_DEPTH_NORMAL);
        glyph.positions = [
            [-0.5, 0.5, 0.0],
            [-0.5, -0.5, 0.0],
            [0.5, -0.5, 0.0],
            [0.5, 0.5, 0.0],
        ];
        assert_ne!(
            quad_ndc_bounds(identity, identity, &glyph),
            quad_ndc_bounds(global_view, identity, &glyph),
            "the semantic name-tag pose remains relative to the copied frame view",
        );
    }

    #[test]
    fn atlas_upload_residency_commits_only_on_submission_confirmation() {
        let key = WorldTextResourceKey {
            raster_y_direction: RasterYDirection::Up,
            asset_id: 7,
            atlas_generation: 2,
            atlas_revision: 3,
            color_format: ColorFormat::Bgra8Unorm,
        };
        let pending = WorldTextResources {
            upload_buffer: Handle::NULL,
            uniform_buffer: Handle::NULL,
            texture: Handle::NULL,
            sampler: Handle::NULL,
            vertex_shader: Handle::NULL,
            fragment_shader: Handle::NULL,
            texture_view: Handle::NULL,
            resource_layout: Handle::NULL,
            resource_set: Handle::NULL,
            pipeline_layout: Handle::NULL,
            pipeline_depth_disabled: Handle::NULL,
            pipeline_depth_test_no_write: Handle::NULL,
            pipeline_depth_test_polygon_offset: Handle::NULL,
            uploaded: false,
            upload_pending: true,
        };
        let mut frontend = WorldTextFrontend::default();
        frontend.resources.insert(key, pending);
        frontend.pending_upload_keys.insert(key);

        frontend.cancel_submission();
        let resources = frontend.resources.get(&key).unwrap();
        assert!(!resources.uploaded);
        assert!(!resources.upload_pending);

        frontend.resources.get_mut(&key).unwrap().upload_pending = true;
        frontend.pending_upload_keys.insert(key);
        frontend.confirm_submission();
        let resources = frontend.resources.get(&key).unwrap();
        assert!(resources.uploaded);
        assert!(!resources.upload_pending);

        // A second semantic producer may append an empty text slice after the
        // first producer has staged an upload. That append must not reopen the
        // transaction and clear the pending marker before confirmation.
        assert!(frontend.resources.get(&key).unwrap().uploaded);
        frontend.resources.get_mut(&key).unwrap().uploaded = false;
        frontend.resources.get_mut(&key).unwrap().upload_pending = true;
        frontend.upload_submission_active = true;
        let mut gal = super::super::tests::gal();
        let mut ops = Vec::new();
        frontend
            .append_frame_ops(
                &mut gal,
                Handle::NULL,
                Handle::NULL,
                Handle::NULL,
                Handle::NULL,
                Handle::NULL,
                TextureUsageState::Undefined,
                ColorFormat::Bgra8Unorm,
                RasterYDirection::Up,
                [0.0; 16],
                [0.0; 16],
                &[],
                &mut ops,
                false,
            )
            .unwrap();
        let resources = frontend.resources.get(&key).unwrap();
        assert!(resources.upload_pending);
        assert!(!resources.uploaded);
    }

    #[test]
    fn begin_submission_discards_previous_producer_markers() {
        let key = WorldTextResourceKey {
            raster_y_direction: RasterYDirection::Up,
            asset_id: 7,
            atlas_generation: 2,
            atlas_revision: 3,
            color_format: ColorFormat::Bgra8Unorm,
        };
        let pending = WorldTextResources {
            upload_buffer: Handle::NULL,
            uniform_buffer: Handle::NULL,
            texture: Handle::NULL,
            sampler: Handle::NULL,
            vertex_shader: Handle::NULL,
            fragment_shader: Handle::NULL,
            texture_view: Handle::NULL,
            resource_layout: Handle::NULL,
            resource_set: Handle::NULL,
            pipeline_layout: Handle::NULL,
            pipeline_depth_disabled: Handle::NULL,
            pipeline_depth_test_no_write: Handle::NULL,
            pipeline_depth_test_polygon_offset: Handle::NULL,
            uploaded: false,
            upload_pending: true,
        };
        let mut frontend = WorldTextFrontend::default();
        frontend.resources.insert(key, pending);
        frontend.pending_upload_keys.insert(key);
        frontend.upload_submission_active = true;

        frontend.begin_submission();

        let resources = frontend.resources.get(&key).unwrap();
        assert!(!resources.uploaded);
        assert!(!resources.upload_pending);
        assert!(frontend.upload_submission_active);
        assert!(frontend.pending_upload_keys.is_empty());
    }

    #[test]
    fn appends_owned_alpha_text_draws_with_explicit_depth_modes() {
        let mut capabilities = vulkan_capabilities();
        capabilities.features.presentation = true;
        let mut gal = VulkanicGal::new_with_backend(
            Box::new(MockBackend::with_capabilities(capabilities)),
            false,
        );
        let target = gal
            .create_frame_target(FrameTargetDesc {
                label: "world-text-test-target".to_string(),
                frame_id: 1,
                render_target: crate::render::vulkanic::frame::FrameRenderTargetId(1),
                extent: Extent3d {
                    width: 64,
                    height: 64,
                    depth: 1,
                },
                color_format: ColorFormat::Bgra8Unorm,
            })
            .unwrap();
        let depth_texture = gal
            .create_texture(TextureDesc {
                label: "world-text-test-depth".to_string(),
                dimension: TextureDimension::D2,
                format: TextureFormat::Depth32Float,
                extent: Extent3d {
                    width: 64,
                    height: 64,
                    depth: 1,
                },
                mip_levels: 1,
                array_layers: 1,
                usages: vec![TextureUsage::DepthStencilAttachment],
            })
            .unwrap();
        let depth_view = gal
            .create_texture_view(TextureViewDesc {
                label: "world-text-test-depth-view".to_string(),
                texture: depth_texture,
                format: TextureFormat::Depth32Float,
                base_mip: 0,
                mip_count: 1,
                base_layer: 0,
                layer_count: 1,
            })
            .unwrap();
        let pass = gal
            .create_render_pass(RenderPassDesc {
                label: "world-text-test-pass".to_string(),
                target,
                color_formats: vec![ColorFormat::Bgra8Unorm],
                depth_format: Some(TextureFormat::Depth32Float),
            })
            .unwrap();
        let mut frontend = WorldTextFrontend::default();
        frontend.apply_image_update(1, vec![asset()]).unwrap();
        let mut ops = Vec::new();
        frontend
            .append_frame_ops(
                &mut gal,
                target,
                pass,
                target,
                depth_texture,
                depth_view,
                TextureUsageState::DepthStencilAttachment,
                ColorFormat::Bgra8Unorm,
                RasterYDirection::Up,
                [
                    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
                ],
                [
                    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
                ],
                &[
                    quad(WORLD_TEXT_DEPTH_SEE_THROUGH),
                    quad(WORLD_TEXT_DEPTH_NORMAL),
                    quad(WORLD_TEXT_DEPTH_POLYGON_OFFSET),
                ],
                &mut ops,
                true,
            )
            .unwrap();

        assert_eq!(1, frontend.resources.len());
        assert_eq!(
            3,
            ops.iter()
                .filter(|op| matches!(
                    op,
                    CommandOp::Draw {
                        vertices: 6,
                        instances: 1
                    }
                ))
                .count()
        );
        assert_eq!(
            3,
            ops.iter()
                .filter(|op| matches!(op, CommandOp::BindGraphicsPipeline(_)))
                .count()
        );
        // `append_frame_ops` only stages residency; even the legacy
        // `mark_uploaded` argument must not commit before the GAL accepts the
        // complete enclosing submission.
        assert!(!frontend.resources.values().next().unwrap().uploaded);
        frontend.confirm_submission();
        let resources = frontend.resources.values().next().unwrap();
        assert!(
            resources.uploaded,
            "admitted text execution must commit atlas upload state"
        );
        let bound_pipelines = ops
            .iter()
            .filter_map(|op| match op {
                CommandOp::BindGraphicsPipeline(handle) => Some(*handle),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            vec![
                resources.pipeline_depth_disabled,
                resources.pipeline_depth_test_no_write,
                resources.pipeline_depth_test_polygon_offset,
            ],
            bound_pipelines
        );
        assert!(
            !ops.iter().any(|op| matches!(
                op,
                CommandOp::Barrier(barrier)
                    if barrier.resource == depth_texture
                        && barrier.before == TextureUsageState::ShaderRead
                        && barrier.after == TextureUsageState::DepthStencilAttachment
            )),
            "direct world text retains the already-attached depth state"
        );
        let up_pipelines = bound_pipelines;
        let mut down_ops = Vec::new();
        frontend.append_frame_ops(
            &mut gal, target, pass, target, depth_texture, depth_view,
            TextureUsageState::DepthStencilAttachment, ColorFormat::Bgra8Unorm,
            RasterYDirection::Down,
            super::super::matrix4_identity(), super::super::matrix4_identity(),
            &[quad(WORLD_TEXT_DEPTH_SEE_THROUGH), quad(WORLD_TEXT_DEPTH_NORMAL),
                quad(WORLD_TEXT_DEPTH_POLYGON_OFFSET)], &mut down_ops, false,
        ).unwrap();
        assert_eq!(frontend.resources.len(), 2,
            "the same atlas must not alias opposite target orientations");
        let down_pipelines = down_ops.iter().filter_map(|op| match op {
            CommandOp::BindGraphicsPipeline(handle) => Some(*handle), _ => None,
        }).collect::<Vec<_>>();
        assert_eq!(down_pipelines.len(), 3);
        for (direction, pipelines) in [(RasterYDirection::Up, up_pipelines),
            (RasterYDirection::Down, down_pipelines)] {
            for pipeline in pipelines {
                assert_eq!(gal.graphics_pipeline_descriptor_for_test(pipeline).unwrap()
                    .raster_y_direction, direction);
            }
        }
        frontend.reset(&mut gal);
        for handle in [pass, depth_view, depth_texture, target] {
            gal.destroy(handle).unwrap();
        }
        assert_eq!(gal.metrics().resource_creates, gal.metrics().resource_destroys);
    }

    #[test]
    fn transitions_shader_read_depth_before_depth_aware_text() {
        let mut capabilities = vulkan_capabilities();
        capabilities.features.presentation = true;
        let mut gal = VulkanicGal::new_with_backend(
            Box::new(MockBackend::with_capabilities(capabilities)),
            false,
        );
        let target = gal
            .create_frame_target(FrameTargetDesc {
                label: "world-text-transition-target".to_string(),
                frame_id: 1,
                render_target: crate::render::vulkanic::frame::FrameRenderTargetId(1),
                extent: Extent3d {
                    width: 64,
                    height: 64,
                    depth: 1,
                },
                color_format: ColorFormat::Bgra8Unorm,
            })
            .unwrap();
        let depth_texture = gal
            .create_texture(TextureDesc {
                label: "world-text-transition-depth".to_string(),
                dimension: TextureDimension::D2,
                format: TextureFormat::Depth32Float,
                extent: Extent3d {
                    width: 64,
                    height: 64,
                    depth: 1,
                },
                mip_levels: 1,
                array_layers: 1,
                usages: vec![TextureUsage::DepthStencilAttachment, TextureUsage::Sampled],
            })
            .unwrap();
        let depth_view = gal
            .create_texture_view(TextureViewDesc {
                label: "world-text-transition-depth-view".to_string(),
                texture: depth_texture,
                format: TextureFormat::Depth32Float,
                base_mip: 0,
                mip_count: 1,
                base_layer: 0,
                layer_count: 1,
            })
            .unwrap();
        let pass = gal
            .create_render_pass(RenderPassDesc {
                label: "world-text-transition-pass".to_string(),
                target,
                color_formats: vec![ColorFormat::Bgra8Unorm],
                depth_format: Some(TextureFormat::Depth32Float),
            })
            .unwrap();
        let mut frontend = WorldTextFrontend::default();
        frontend.apply_image_update(1, vec![asset()]).unwrap();
        let mut ops = Vec::new();
        frontend
            .append_frame_ops(
                &mut gal,
                target,
                pass,
                target,
                depth_texture,
                depth_view,
                TextureUsageState::ShaderRead,
                ColorFormat::Bgra8Unorm,
                RasterYDirection::Up,
                [
                    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
                ],
                [
                    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
                ],
                &[quad(WORLD_TEXT_DEPTH_NORMAL)],
                &mut ops,
                true,
            )
            .unwrap();

        assert!(ops.iter().any(|op| matches!(
            op,
            CommandOp::Barrier(barrier)
                if barrier.resource == depth_texture
                    && barrier.before == TextureUsageState::ShaderRead
                    && barrier.after == TextureUsageState::DepthStencilAttachment
        )));

        gal.submit(SubmissionBatch {
            label: "world-text-transition-submission".to_string(),
            command_lists: vec![CommandList::from(CommandListDesc {
                label: "world-text-transition-commands".to_string(),
                operations: {
                    let mut operations = vec![CommandOp::Barrier(texture_barrier(
                        depth_texture,
                        TextureUsageState::Undefined,
                        TextureUsageState::ShaderRead,
                    ))];
                    operations.extend(ops);
                    operations
                },
            })],
        })
        .expect("the explicit depth transition resolves the sampled-to-attachment hazard");
    }
}
