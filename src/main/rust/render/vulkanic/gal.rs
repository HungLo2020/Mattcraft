use std::collections::{BTreeMap, BTreeSet};

use super::backends::{
    Backend, BackendCreateDesc, BackendRuntimeMetrics, BackendToken, CompletedHostRead,
};
use super::commands::{
    AttachmentLoadOp, AttachmentStoreOp, BufferImageCopyRegion, CommandList, CommandListDesc,
    CommandOp, ResourceBarrier, SubmissionBatch, TextureImageCopyRegion, TextureOrigin3d,
    ValidatedSubmissionBatch, TextureUsageState,
};
use super::error::{GalError, GalResult, StatusCode};
use super::frame::{
    AcquiredFrame, FrameAcquireDesc, FrameId, FrameResizeDesc, FrameResizeResult, FrameSurfaceDesc,
    PresentFrameDesc, PresentedFrame,
};
use super::handles::{Handle, HandleKind, MAX_GENERATION};
use super::metrics::{elapsed_nanos_u64, Metrics, WholeFrameProfile};
use super::resources::*;
use super::sync::{RetirementQueue, SubmissionId, SyncToken};

/// Hard ceiling for one explicit handle arena. Frontend-specific residency
/// limits remain tighter, but the GAL itself must also reject hostile/direct
/// callers before a resource storm can grow a slot vector without bound.
pub(crate) const MAX_ARENA_SLOTS: usize = 1_048_576;
/// Explicit upper bound for swapchain frames retained concurrently.  The
/// frontend normally uses two; keeping a small GAL-wide ceiling prevents a
/// direct FFI caller from turning frame-slot configuration into unbounded
/// synchronization/resource allocation.
pub(crate) const MAX_FRAMES_IN_FLIGHT: u32 = 8;
/// Maximum width/height accepted for an explicit frame surface.  This keeps
/// malformed FFI requests from forcing an unbounded swapchain allocation.
pub(crate) const MAX_FRAME_SURFACE_AXIS: u32 = 16_384;

#[derive(Clone, Debug)]
struct Slot<T> {
    generation: u32,
    last_destroyed_generation: Option<u32>,
    value: Option<T>,
}

#[derive(Clone, Debug)]
struct Arena<T> {
    kind: HandleKind,
    slots: Vec<Slot<T>>,
}

impl<T> Arena<T> {
    #[allow(dead_code)]
    fn new(kind: HandleKind) -> Self {
        Self {
            kind,
            slots: Vec::new(),
        }
    }

    fn next_handle(&self) -> GalResult<Handle> {
        for (index, slot) in self.slots.iter().enumerate() {
            if slot.value.is_none() {
                let index = u32::try_from(index).map_err(|_| {
                    GalError::handle(
                        StatusCode::GenerationExhausted,
                        "handle index space exhausted",
                    )
                })?;
                return Handle::new(self.kind, index, slot.generation);
            }
        }
        if self.slots.len() >= MAX_ARENA_SLOTS {
            return Err(GalError::handle(
                StatusCode::GenerationExhausted,
                format!("{} handle arena slots exhausted", self.kind as u8),
            ));
        }
        let index = u32::try_from(self.slots.len()).map_err(|_| {
            GalError::handle(
                StatusCode::GenerationExhausted,
                "handle index space exhausted",
            )
        })?;
        Handle::new(self.kind, index, 1)
    }

    fn insert_at(&mut self, handle: Handle, value: T) -> GalResult<Handle> {
        let (index, generation) = handle.require_kind(self.kind)?;
        if index == self.slots.len() {
            if self.slots.len() >= MAX_ARENA_SLOTS {
                return Err(GalError::handle(
                    StatusCode::GenerationExhausted,
                    format!("{} handle arena slots exhausted", self.kind as u8),
                ));
            }
            self.slots.push(Slot {
                generation,
                last_destroyed_generation: None,
                value: Some(value),
            });
            return Ok(handle);
        }
        let slot = self.slots.get_mut(index).ok_or_else(|| {
            GalError::handle(StatusCode::StaleHandle, "handle slot does not exist")
        })?;
        if slot.generation != generation || slot.value.is_some() {
            return Err(GalError::handle(
                StatusCode::StaleHandle,
                "handle slot is not available for insertion",
            ));
        }
        slot.last_destroyed_generation = None;
        slot.value = Some(value);
        Ok(handle)
    }

    fn get(&self, handle: Handle) -> GalResult<&T> {
        let (index, generation) = handle.require_kind(self.kind)?;
        let Some(slot) = self.slots.get(index) else {
            return Err(GalError::handle(
                StatusCode::StaleHandle,
                "handle slot does not exist",
            ));
        };
        if slot.generation != generation {
            return Err(GalError::handle(
                StatusCode::StaleHandle,
                format!(
                    "stale handle generation kind={:?} index={} requested_generation={} live_generation={}",
                    self.kind, index, generation, slot.generation
                ),
            ));
        }
        slot.value.as_ref().ok_or_else(|| {
            if slot.last_destroyed_generation == Some(generation) {
                GalError::handle(StatusCode::DoubleDestroy, "resource was already destroyed")
            } else {
                GalError::handle(StatusCode::StaleHandle, "resource is not live")
            }
        })
    }

    fn remove(&mut self, handle: Handle) -> GalResult<T> {
        let (index, generation) = handle.require_kind(self.kind)?;
        let Some(slot) = self.slots.get_mut(index) else {
            return Err(GalError::handle(
                StatusCode::StaleHandle,
                "handle slot does not exist",
            ));
        };
        if slot.generation != generation {
            if slot.last_destroyed_generation == Some(generation) {
                return Err(GalError::handle(
                    StatusCode::DoubleDestroy,
                    "resource was already destroyed",
                ));
            }
            return Err(GalError::handle(
                StatusCode::StaleHandle,
                format!(
                    "stale handle generation kind={:?} index={} requested_generation={} live_generation={}",
                    self.kind, index, generation, slot.generation
                ),
            ));
        }
        let value = slot.value.take().ok_or_else(|| {
            GalError::handle(StatusCode::DoubleDestroy, "resource was already destroyed")
        })?;
        if slot.generation == MAX_GENERATION {
            slot.value = Some(value);
            return Err(GalError::handle(
                StatusCode::GenerationExhausted,
                "resource generation exhausted",
            ));
        }
        slot.last_destroyed_generation = Some(slot.generation);
        slot.generation += 1;
        Ok(value)
    }

    #[cfg(test)]
    fn force_generation(&mut self, handle: Handle, generation: u32) {
        let index = handle.index() as usize;
        self.slots[index].generation = generation;
    }
}

#[derive(Clone, Debug)]
struct ResourceRecord<T> {
    desc: T,
    token: BackendToken,
    last_submission: Option<SubmissionId>,
}

#[derive(Clone, Copy, Debug)]
struct PendingDestroy {
    kind: HandleKind,
    token: BackendToken,
}

#[derive(Clone, Debug)]
pub(super) struct TextureViewInfo {
    pub(super) texture: Handle,
    pub(super) format: TextureFormat,
    pub(super) extent: Extent3d,
    pub(super) range: TextureSubresourceRange,
    pub(super) usages: Vec<TextureUsage>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AccessMode {
    Read,
    Write,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AccessFamily {
    Vertex,
    Index,
    Uniform,
    Storage,
    Sampled,
    Transfer,
    Attachment,
    Host,
    Present,
    Indirect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AccessTarget {
    Buffer {
        handle: Handle,
        offset: u64,
        size: u64,
    },
    Texture {
        texture: Handle,
        range: TextureSubresourceRange,
    },
    FrameTarget {
        handle: Handle,
    },
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum AccessResourceKey {
    Buffer(Handle),
    Texture(Handle),
    FrameTarget(Handle),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AccessEvent {
    target: AccessTarget,
    mode: AccessMode,
    family: AccessFamily,
    attachment_load_op: Option<AttachmentLoadOp>,
    attachment_store_op: Option<AttachmentStoreOp>,
}

#[derive(Default)]
struct AccessTracker {
    resources: BTreeMap<AccessResourceKey, AccessBucket>,
}

#[derive(Default)]
struct AccessBucket {
    reads: Vec<AccessEvent>,
    writes: Vec<AccessEvent>,
}

impl AccessTracker {
    fn push_read(&mut self, event: AccessEvent) {
        let bucket = self
            .resources
            .entry(event.target.resource_key())
            .or_default();
        if !bucket.reads.contains(&event) {
            bucket.reads.push(event);
        }
    }

    fn push_write(&mut self, event: AccessEvent) {
        self.resources
            .entry(event.target.resource_key())
            .or_default()
            .writes
            .push(event);
    }

    fn bucket(&self, target: AccessTarget) -> Option<&AccessBucket> {
        self.resources.get(&target.resource_key())
    }

    fn retain_non_overlapping(&mut self, target: AccessTarget) {
        let key = target.resource_key();
        if let Some(bucket) = self.resources.get_mut(&key) {
            bucket
                .reads
                .retain(|access| !targets_overlap(access.target, target));
            bucket
                .writes
                .retain(|access| !targets_overlap(access.target, target));
            if bucket.reads.is_empty() && bucket.writes.is_empty() {
                self.resources.remove(&key);
            }
        }
    }

    fn active_read_entries(&self) -> usize {
        self.resources
            .values()
            .map(|bucket| bucket.reads.len())
            .sum()
    }

    fn active_write_entries(&self) -> usize {
        self.resources
            .values()
            .map(|bucket| bucket.writes.len())
            .sum()
    }
}

#[derive(Default)]
pub(super) struct CommandNormalizationStats {
    pub(super) ops_before: u64,
    pub(super) ops_after: u64,
    pub(super) pipeline_binds_removed: u64,
    pub(super) resource_set_binds_removed: u64,
    pub(super) vertex_buffer_binds_removed: u64,
    pub(super) index_buffer_binds_removed: u64,
}

#[derive(Default)]
struct CommandStateTracker {
    graphics_pipeline: Option<Handle>,
    compute_pipeline: Option<Handle>,
    resource_sets: BTreeMap<(Handle, u32), (Handle, Vec<u64>)>,
    vertex_buffers: BTreeMap<u32, (Handle, u64)>,
    index_buffer: Option<(Handle, u64, IndexType)>,
}

impl CommandStateTracker {
    fn invalidate(&mut self) {
        self.graphics_pipeline = None;
        self.compute_pipeline = None;
        self.resource_sets.clear();
        self.vertex_buffers.clear();
        self.index_buffer = None;
    }
}

pub struct VulkanicGal {
    backend: Box<dyn Backend>,
    buffers: Arena<ResourceRecord<BufferDesc>>,
    textures: Arena<ResourceRecord<TextureDesc>>,
    texture_views: Arena<ResourceRecord<TextureViewDesc>>,
    samplers: Arena<ResourceRecord<SamplerDesc>>,
    combined_texture_samplers: Arena<ResourceRecord<CombinedTextureSamplerDesc>>,
    shaders: Arena<ResourceRecord<ShaderModuleDesc>>,
    resource_layouts: Arena<ResourceRecord<ResourceLayoutDesc>>,
    resource_sets: Arena<ResourceRecord<ResourceSetDesc>>,
    pipeline_layouts: Arena<ResourceRecord<PipelineLayoutDesc>>,
    graphics_pipelines: Arena<ResourceRecord<GraphicsPipelineDesc>>,
    compute_pipelines: Arena<ResourceRecord<ComputePipelineDesc>>,
    render_targets: Arena<ResourceRecord<RenderTargetDesc>>,
    frame_targets: Arena<ResourceRecord<FrameTargetDesc>>,
    /// Rust-owned depth attachments reserved for acquired frame targets.
    /// They remain private until frame-pass construction supplies them as an
    /// explicit depth attachment.
    frame_target_depth: BTreeMap<Handle, (Handle, Handle)>,
    /// Acquired-frame depth becomes sampleable only after a Rust-owned world
    /// pass has actually written it.  Allocation alone must not admit it to
    /// post-effect inputs.
    frame_target_depth_populated: BTreeSet<Handle>,
    frame_target_depth_pending: BTreeSet<Handle>,
    render_passes: Arena<ResourceRecord<RenderPassDesc>>,
    dependencies: BTreeMap<Handle, BTreeSet<Handle>>,
    reverse_dependencies: BTreeMap<Handle, BTreeSet<Handle>>,
    pending_destroys: BTreeMap<Handle, PendingDestroy>,
    retirement: RetirementQueue,
    next_submission: u64,
    latest_accepted_submission: SubmissionId,
    completed_submission: SubmissionId,
    metrics: Metrics,
}

fn submission_trace(message: &str) {
    if std::env::var_os("MATTMC_TRACE_WHOLE_FRAME").is_some() {
        eprintln!("{message}");
    }
}

impl VulkanicGal {
    #[allow(dead_code)]
    pub(in crate::render::vulkanic) fn new_with_backend(
        backend: Box<dyn Backend>,
        tracy_enabled: bool,
    ) -> Self {
        Self {
            backend,
            buffers: Arena::new(HandleKind::Buffer),
            textures: Arena::new(HandleKind::Texture),
            texture_views: Arena::new(HandleKind::TextureView),
            samplers: Arena::new(HandleKind::Sampler),
            combined_texture_samplers: Arena::new(HandleKind::CombinedTextureSampler),
            shaders: Arena::new(HandleKind::ShaderModule),
            resource_layouts: Arena::new(HandleKind::ResourceLayout),
            resource_sets: Arena::new(HandleKind::ResourceSet),
            pipeline_layouts: Arena::new(HandleKind::PipelineLayout),
            graphics_pipelines: Arena::new(HandleKind::GraphicsPipeline),
            compute_pipelines: Arena::new(HandleKind::ComputePipeline),
            render_targets: Arena::new(HandleKind::RenderTarget),
            frame_targets: Arena::new(HandleKind::FrameTarget),
            frame_target_depth: BTreeMap::new(),
            frame_target_depth_populated: BTreeSet::new(),
            frame_target_depth_pending: BTreeSet::new(),
            render_passes: Arena::new(HandleKind::RenderPass),
            dependencies: BTreeMap::new(),
            reverse_dependencies: BTreeMap::new(),
            pending_destroys: BTreeMap::new(),
            retirement: RetirementQueue::new(),
            next_submission: 1,
            latest_accepted_submission: SubmissionId(0),
            completed_submission: SubmissionId(0),
            metrics: Metrics::new(tracy_enabled),
        }
    }

    pub fn metrics(&self) -> &Metrics {
        &self.metrics
    }

    pub(in crate::render::vulkanic) fn backend_runtime_metrics(
        &self,
    ) -> super::backends::BackendRuntimeMetrics {
        self.backend.runtime_metrics()
    }

    pub fn capabilities(&self) -> BackendCapabilities {
        self.backend.capabilities()
    }

    pub fn configure_frame_surface(&mut self, desc: FrameSurfaceDesc) -> GalResult<()> {
        if desc.extent.width == 0 || desc.extent.height == 0 || desc.extent.depth == 0 {
            return self.validation_error(GalError::resource(
                StatusCode::InvalidArgument,
                "frame surface extent must be non-zero",
            ));
        }
        if desc.extent.width > MAX_FRAME_SURFACE_AXIS
            || desc.extent.height > MAX_FRAME_SURFACE_AXIS
            || desc.extent.depth != 1
        {
            return self.validation_error(GalError::resource(
                StatusCode::InvalidArgument,
                format!(
                    "frame surface extent exceeds the explicit {}x{}x1 bound",
                    MAX_FRAME_SURFACE_AXIS, MAX_FRAME_SURFACE_AXIS
                ),
            ));
        }
        if desc.max_frames_in_flight == 0 {
            return self.validation_error(GalError::resource(
                StatusCode::InvalidArgument,
                "frame surface must allow at least one frame in flight",
            ));
        }
        if desc.max_frames_in_flight > MAX_FRAMES_IN_FLIGHT {
            return self.validation_error(GalError::resource(
                StatusCode::InvalidArgument,
                format!(
                    "frame surface exceeds the explicit {}-frame in-flight bound",
                    MAX_FRAMES_IN_FLIGHT
                ),
            ));
        }
        let capabilities = self.capabilities();
        if !capabilities.supports(BackendFeature::Presentation) {
            return self.unsupported(format!(
                "backend '{}' was not created with presentation support",
                capabilities.name
            ));
        }
        self.backend.configure_frame_surface(&desc)
    }

    pub fn acquire_frame(&mut self, desc: FrameAcquireDesc) -> GalResult<AcquiredFrame> {
        if desc.expected_extent.depth == 0 {
            return self.validation_error(GalError::resource(
                StatusCode::InvalidArgument,
                "frame acquire extent depth must be non-zero",
            ));
        }
        let capabilities = self.capabilities();
        if !capabilities.supports(BackendFeature::Presentation) {
            return self.unsupported(format!(
                "backend '{}' was not created with presentation support",
                capabilities.name
            ));
        }
        self.backend.acquire_frame(&desc)
    }

    pub fn resize_frame_surface(&mut self, desc: FrameResizeDesc) -> GalResult<FrameResizeResult> {
        if desc.extent.depth == 0 {
            return self.validation_error(GalError::resource(
                StatusCode::InvalidArgument,
                "frame resize extent depth must be non-zero",
            ));
        }
        let capabilities = self.capabilities();
        if !capabilities.supports(BackendFeature::Presentation) {
            return self.unsupported(format!(
                "backend '{}' was not created with presentation support",
                capabilities.name
            ));
        }
        self.backend.resize_frame_surface(&desc)
    }

    pub fn present_frame(&mut self, desc: PresentFrameDesc) -> GalResult<PresentedFrame> {
        let capabilities = self.capabilities();
        if !capabilities.supports(BackendFeature::Presentation) {
            return self.unsupported(format!(
                "backend '{}' was not created with presentation support",
                capabilities.name
            ));
        }
        let presented = self.backend.present_frame(&desc)?;
        if presented.completed_submission > self.completed_submission {
            self.completed_submission = presented.completed_submission;
        }
        Ok(presented)
    }

    pub fn cancel_frame(&mut self, frame: FrameId) -> GalResult<()> {
        let capabilities = self.capabilities();
        if !capabilities.supports(BackendFeature::Presentation) {
            return self.unsupported(format!(
                "backend '{}' was not created with presentation support",
                capabilities.name
            ));
        }
        self.backend.cancel_frame(frame)
    }

    pub fn shutdown_frame_surface(&mut self) -> GalResult<()> {
        self.backend.shutdown_frame_surface()
    }

    pub fn create_frame_target(&mut self, desc: FrameTargetDesc) -> GalResult<Handle> {
        if desc.extent.width == 0 || desc.extent.height == 0 || desc.extent.depth == 0 {
            return self.validation_error(GalError::resource(
                StatusCode::InvalidArgument,
                "frame target extent must be non-empty",
            ));
        }
        let capabilities = self.capabilities();
        if !capabilities.supports(BackendFeature::Presentation) {
            return self.unsupported(format!(
                "backend '{}' was not created with presentation support",
                capabilities.name
            ));
        }
        let handle = self.frame_targets.next_handle()?;
        let depth_texture = self.create_texture(TextureDesc {
            label: format!("{}.depth", desc.label),
            dimension: TextureDimension::D2,
            format: TextureFormat::Depth32Float,
            extent: desc.extent,
            mip_levels: 1,
            array_layers: 1,
            usages: vec![TextureUsage::DepthStencilAttachment, TextureUsage::Sampled, TextureUsage::TransferDst],
        })?;
        let depth_view = match self.create_texture_view(TextureViewDesc {
            label: format!("{}.depth-view", desc.label),
            texture: depth_texture,
            format: TextureFormat::Depth32Float,
            base_mip: 0,
            mip_count: 1,
            base_layer: 0,
            layer_count: 1,
        }) {
            Ok(view) => view,
            Err(error) => {
                let _ = self.destroy(depth_texture);
                return Err(error);
            }
        };
        let token = match self
            .backend
            .create(handle, BackendCreateDesc::FrameTarget(&desc))
        {
            Ok(token) => token,
            Err(error) => {
                let _ = self.destroy(depth_view);
                let _ = self.destroy(depth_texture);
                return Err(error);
            }
        };
        self.frame_target_depth
            .insert(handle, (depth_texture, depth_view));
        self.metrics.resource_creates += 1;
        let result = self.frame_targets.insert_at(
            handle,
            ResourceRecord {
                desc,
                token,
                last_submission: None,
            },
        );
        if let Err(error) = result {
            self.frame_target_depth.remove(&handle);
            let _ = self.destroy(depth_view);
            let _ = self.destroy(depth_texture);
            let _ = self.backend.destroy(handle, HandleKind::FrameTarget, token);
            return Err(error);
        }
        Ok(handle)
    }

    pub fn create_buffer(&mut self, desc: BufferDesc) -> GalResult<Handle> {
        if desc.size == 0 || desc.usages.is_empty() {
            return self.validation_error(GalError::resource(
                StatusCode::InvalidArgument,
                "buffer size and usages must be non-empty",
            ));
        }
        let capabilities = self.capabilities();
        if desc.size > capabilities.limits.max_buffer_size {
            return self.unsupported(format!(
                "buffer '{}' size {} exceeds backend '{}' limit {}",
                desc.label, desc.size, capabilities.name, capabilities.limits.max_buffer_size
            ));
        }
        if desc.usages.contains(&BufferUsage::Storage)
            && !capabilities.supports(BackendFeature::StorageBuffers)
        {
            return self.unsupported(format!(
                "backend '{}' does not support storage buffers",
                capabilities.name
            ));
        }
        let handle = self.buffers.next_handle()?;
        let token = self
            .backend
            .create(handle, BackendCreateDesc::Buffer(&desc))?;
        self.metrics.resource_creates += 1;
        self.buffers.insert_at(
            handle,
            ResourceRecord {
                desc,
                token,
                last_submission: None,
            },
        )
    }

    pub(in crate::render::vulkanic) fn frame_target_color_format(
        &self,
        handle: Handle,
    ) -> GalResult<ColorFormat> {
        Ok(self.frame_targets.get(handle)?.desc.color_format)
    }

    /// Returns only the backend-neutral identity and compatibility facts for
    /// an acquired frame target. Frontends may cache pass resources by this
    /// descriptor, but never receive a backend image, view, framebuffer, or
    /// presentation object.
    pub(in crate::render::vulkanic) fn frame_target_desc(
        &self,
        handle: Handle,
    ) -> GalResult<FrameTargetDesc> {
        Ok(self.frame_targets.get(handle)?.desc.clone())
    }

    pub(in crate::render::vulkanic) fn pass_target_color_format(
        &self,
        handle: Handle,
    ) -> GalResult<ColorFormat> {
        match handle.kind() {
            Some(HandleKind::FrameTarget) => self.frame_target_color_format(handle),
            Some(HandleKind::RenderTarget) => {
                let target = self.render_targets.get(handle)?;
                let Some(view) = target.desc.color_views.first().copied() else {
                    return Err(GalError::resource(
                        StatusCode::InvalidArgument,
                        "render target must have a color attachment for world primitives",
                    ));
                };
                self.texture_view_info(view).map(|info| info.format)
            }
            _ => Err(GalError::resource(
                StatusCode::WrongHandleType,
                "pass target must be a render target or frame target",
            )),
        }
    }

    /// Returns the semantic extent of a render or acquired frame target.
    /// Frontends use this to size private intermediate resources without
    /// reaching into backend framebuffers or native images.
    pub(in crate::render::vulkanic) fn pass_target_extent(
        &self,
        handle: Handle,
    ) -> GalResult<Extent3d> {
        match handle.kind() {
            Some(HandleKind::FrameTarget) => Ok(self.frame_targets.get(handle)?.desc.extent),
            Some(HandleKind::RenderTarget) => Ok(self.render_targets.get(handle)?.desc.extent),
            _ => Err(GalError::resource(
                StatusCode::WrongHandleType,
                "pass target must be a render target or frame target",
            )),
        }
    }

    /// Resolves the color attachment texture behind an owned render target.
    /// This is an explicit GAL resource identity, never a backend/native
    /// handle, and is intentionally unavailable for acquired frame targets.
    pub(in crate::render::vulkanic) fn pass_target_color_texture(
        &self,
        handle: Handle,
    ) -> GalResult<Handle> {
        match handle.kind() {
            Some(HandleKind::RenderTarget) => {
                let target = self.render_targets.get(handle)?;
                let view = target.desc.color_views.first().copied().ok_or_else(|| {
                    GalError::resource(
                        StatusCode::InvalidArgument,
                        "render target must have a color attachment",
                    )
                })?;
                Ok(self.texture_view_info(view)?.texture)
            }
            Some(HandleKind::FrameTarget) => Err(GalError::unsupported_feature(
                "acquired frame targets are copied through the explicit frame-target operation",
            )),
            _ => Err(GalError::resource(
                StatusCode::WrongHandleType,
                "pass target must be a render target or frame target",
            )),
        }
    }

    pub(in crate::render::vulkanic) fn pass_target_color_attachment(
        &self,
        handle: Handle,
    ) -> GalResult<Handle> {
        match handle.kind() {
            Some(HandleKind::FrameTarget) => {
                self.frame_targets.get(handle)?;
                Ok(handle)
            }
            Some(HandleKind::RenderTarget) => {
                let target = self.render_targets.get(handle)?;
                target.desc.color_views.first().copied().ok_or_else(|| {
                    GalError::resource(
                        StatusCode::InvalidArgument,
                        "render target must have a color attachment for world primitives",
                    )
                })
            }
            _ => Err(GalError::resource(
                StatusCode::WrongHandleType,
                "pass target must be a render target or frame target",
            )),
        }
    }

    pub(in crate::render::vulkanic) fn pass_target_depth_attachment(
        &self,
        handle: Handle,
    ) -> GalResult<Option<(Handle, Handle)>> {
        match handle.kind() {
            Some(HandleKind::FrameTarget) => {
                self.frame_targets.get(handle)?;
                if self.frame_target_depth_populated.contains(&handle)
                    || self.frame_target_depth_pending.contains(&handle)
                {
                    Ok(self.frame_target_depth.get(&handle).copied())
                } else {
                    // Allocation alone must not expose an unpopulated depth
                    // image to post-effects.
                    Ok(None)
                }
            }
            Some(HandleKind::RenderTarget) => {
                let target = self.render_targets.get(handle)?;
                target
                    .desc
                    .depth_stencil_view
                    .map(|view| {
                        self.texture_view_info(view)
                            .map(|info| (info.texture, view))
                    })
                    .transpose()
            }
            _ => Err(GalError::resource(
                StatusCode::WrongHandleType,
                "pass target must be a render target or frame target",
            )),
        }
    }

    pub(in crate::render::vulkanic) fn frame_target_owned_depth_attachment(
        &self,
        handle: Handle,
    ) -> GalResult<(Handle, Handle)> {
        if handle.kind() != Some(HandleKind::FrameTarget) {
            return Err(GalError::resource(
                StatusCode::WrongHandleType,
                "owned frame depth attachment requires a frame target",
            ));
        }
        self.frame_targets.get(handle)?;
        self.frame_target_depth
            .get(&handle)
            .copied()
            .ok_or_else(|| {
                GalError::resource(
                    StatusCode::StaleHandle,
                    "frame target depth attachment is unavailable",
                )
            })
    }

    pub(in crate::render::vulkanic) fn begin_frame_target_depth_write(
        &mut self,
        handle: Handle,
    ) -> GalResult<()> {
        if handle.kind() != Some(HandleKind::FrameTarget) {
            return Err(GalError::resource(
                StatusCode::WrongHandleType,
                "populated frame depth requires a frame target",
            ));
        }
        self.frame_targets.get(handle)?;
        if !self.frame_target_depth.contains_key(&handle) {
            return Err(GalError::resource(
                StatusCode::StaleHandle,
                "frame target depth attachment is unavailable",
            ));
        }
        self.frame_target_depth_pending.insert(handle);
        Ok(())
    }

    pub(in crate::render::vulkanic) fn commit_frame_target_depth_write(
        &mut self,
        handle: Handle,
    ) -> GalResult<()> {
        self.frame_targets.get(handle)?;
        self.frame_target_depth_pending.remove(&handle);
        self.frame_target_depth_populated.insert(handle);
        Ok(())
    }

    pub(in crate::render::vulkanic) fn rollback_frame_target_depth_write(
        &mut self,
        handle: Handle,
    ) {
        self.frame_target_depth_pending.remove(&handle);
    }

    pub fn create_texture(&mut self, desc: TextureDesc) -> GalResult<Handle> {
        if desc.extent.width == 0
            || desc.extent.height == 0
            || desc.extent.depth == 0
            || desc.mip_levels == 0
            || desc.array_layers == 0
            || desc.usages.is_empty()
        {
            return self.validation_error(GalError::resource(
                StatusCode::InvalidArgument,
                "texture extent, levels, layers, and usages must be non-empty",
            ));
        }
        let capabilities = self.capabilities();
        match desc.dimension {
            TextureDimension::D2 => {
                if desc.extent.depth != 1 {
                    return self.validation_error(GalError::resource(
                        StatusCode::InvalidArgument,
                        "D2 textures require depth 1",
                    ));
                }
            }
            TextureDimension::D3 => {
                if desc.array_layers != 1 {
                    return self.validation_error(GalError::resource(
                        StatusCode::InvalidArgument,
                        "D3 textures cannot have array layers",
                    ));
                }
                if !capabilities.supports(BackendFeature::Texture3d) {
                    return self.unsupported(format!(
                        "backend '{}' does not support D3 textures",
                        capabilities.name
                    ));
                }
                if desc.extent.width > capabilities.limits.max_texture_extent_3d
                    || desc.extent.height > capabilities.limits.max_texture_extent_3d
                    || desc.extent.depth > capabilities.limits.max_texture_extent_3d
                {
                    return self.unsupported(format!(
                        "texture '{}' extent {}x{}x{} exceeds backend '{}' 3D extent limit {}",
                        desc.label,
                        desc.extent.width,
                        desc.extent.height,
                        desc.extent.depth,
                        capabilities.name,
                        capabilities.limits.max_texture_extent_3d
                    ));
                }
            }
            _ => return self.unsupported("only D2 and D3 texture dimensions are modeled"),
        }
        let max_mips_for_extent = max_texture_mip_levels(desc.extent);
        if desc.mip_levels > max_mips_for_extent {
            return self.validation_error(GalError::resource(
                StatusCode::InvalidArgument,
                format!(
                    "texture '{}' requests {} mips for {}x{}x{} extent (maximum {})",
                    desc.label,
                    desc.mip_levels,
                    desc.extent.width,
                    desc.extent.height,
                    desc.extent.depth,
                    max_mips_for_extent
                ),
            ));
        }
        if desc.format == TextureFormat::R8Uint
            && (desc.usages.contains(&TextureUsage::ColorAttachment)
                || desc.usages.contains(&TextureUsage::DepthStencilAttachment)
                || desc.usages.contains(&TextureUsage::Present))
        {
            return self.validation_error(GalError::resource(
                StatusCode::InvalidArgument,
                "R8Uint is a sampled/storage/transfer format, not a render target format",
            ));
        }
        if is_depth_format(desc.format)
            && (desc.usages.contains(&TextureUsage::ColorAttachment)
                || desc.usages.contains(&TextureUsage::Storage)
                || desc.usages.contains(&TextureUsage::Present))
        {
            return self.validation_error(GalError::resource(
                StatusCode::InvalidArgument,
                "depth formats cannot be used as color, storage, or present textures",
            ));
        }
        if desc.dimension == TextureDimension::D2
            && (desc.extent.width > capabilities.limits.max_texture_extent_2d
                || desc.extent.height > capabilities.limits.max_texture_extent_2d)
        {
            return self.unsupported(format!(
                "texture '{}' extent {}x{} exceeds backend '{}' 2D extent limit {}",
                desc.label,
                desc.extent.width,
                desc.extent.height,
                capabilities.name,
                capabilities.limits.max_texture_extent_2d
            ));
        }
        if desc.mip_levels > capabilities.limits.max_texture_mip_levels {
            return self.unsupported(format!(
                "texture '{}' mip count {} exceeds backend '{}' limit {}",
                desc.label,
                desc.mip_levels,
                capabilities.name,
                capabilities.limits.max_texture_mip_levels
            ));
        }
        if desc.mip_levels > 1 && !capabilities.supports(BackendFeature::TextureMipLevels) {
            return self.unsupported(format!(
                "backend '{}' does not support multi-mip textures",
                capabilities.name
            ));
        }
        if desc.array_layers > capabilities.limits.max_texture_array_layers {
            return self.unsupported(format!(
                "texture '{}' layer count {} exceeds backend '{}' limit {}",
                desc.label,
                desc.array_layers,
                capabilities.name,
                capabilities.limits.max_texture_array_layers
            ));
        }
        if desc.array_layers > 1 && !capabilities.supports(BackendFeature::TextureArrayLayers) {
            return self.unsupported(format!(
                "backend '{}' does not support texture arrays",
                capabilities.name
            ));
        }
        if desc.usages.contains(&TextureUsage::Storage)
            && !capabilities.supports(BackendFeature::StorageTextures)
        {
            return self.unsupported(format!(
                "backend '{}' does not support storage textures",
                capabilities.name
            ));
        }
        if desc.usages.contains(&TextureUsage::Present)
            && !capabilities.supports(BackendFeature::Presentation)
        {
            return self.unsupported(format!(
                "backend '{}' does not support presentation textures in the isolated path",
                capabilities.name
            ));
        }
        if desc.dimension == TextureDimension::D3
            && (desc.usages.contains(&TextureUsage::ColorAttachment)
                || desc.usages.contains(&TextureUsage::DepthStencilAttachment)
                || desc.usages.contains(&TextureUsage::Present))
        {
            return self.unsupported(
                "D3 textures are modeled for sampled, storage, and transfer use, not render targets",
            );
        }
        if desc.dimension == TextureDimension::D3 {
            for usage in desc.usages.iter().copied() {
                if !capabilities.supports_texture_3d_usage(desc.format, usage) {
                    return self.unsupported(format!(
                        "backend '{}' does not support D3 texture format {:?} with usage {:?}",
                        capabilities.name, desc.format, usage
                    ));
                }
            }
        }
        let handle = self.textures.next_handle()?;
        let token = self
            .backend
            .create(handle, BackendCreateDesc::Texture(&desc))?;
        self.metrics.resource_creates += 1;
        self.textures.insert_at(
            handle,
            ResourceRecord {
                desc,
                token,
                last_submission: None,
            },
        )
    }

    pub fn create_texture_view(&mut self, desc: TextureViewDesc) -> GalResult<Handle> {
        let (texture_dimension, texture_mip_levels, texture_array_layers, texture_format) = {
            let texture = self.textures.get(desc.texture)?;
            (
                texture.desc.dimension,
                texture.desc.mip_levels,
                texture.desc.array_layers,
                texture.desc.format,
            )
        };
        let Some(mip_end) = desc.base_mip.checked_add(desc.mip_count) else {
            return self.validation_error(GalError::resource(
                StatusCode::InvalidArgument,
                "texture view mip range overflows",
            ));
        };
        let Some(layer_end) = desc.base_layer.checked_add(desc.layer_count) else {
            return self.validation_error(GalError::resource(
                StatusCode::InvalidArgument,
                "texture view layer range overflows",
            ));
        };
        if desc.mip_count == 0
            || desc.layer_count == 0
            || mip_end > texture_mip_levels
            || layer_end > texture_array_layers
        {
            return self.validation_error(GalError::resource(
                StatusCode::InvalidArgument,
                "texture view range is outside the texture",
            ));
        }
        if desc.format != texture_format {
            return self.validation_error(GalError::resource(
                StatusCode::InvalidArgument,
                "texture view format reinterpretation is not modeled",
            ));
        }
        if texture_dimension == TextureDimension::D3
            && (desc.base_layer != 0 || desc.layer_count != 1)
        {
            return self.validation_error(GalError::resource(
                StatusCode::InvalidArgument,
                "D3 texture views must address the single 3D image layer",
            ));
        }
        let handle = self.texture_views.next_handle()?;
        let token = self
            .backend
            .create(handle, BackendCreateDesc::TextureView(&desc))?;
        self.add_dependency(desc.texture, handle);
        self.metrics.resource_creates += 1;
        self.texture_views.insert_at(
            handle,
            ResourceRecord {
                desc,
                token,
                last_submission: None,
            },
        )
    }

    pub fn create_sampler(&mut self, desc: SamplerDesc) -> GalResult<Handle> {
        let handle = self.samplers.next_handle()?;
        let token = self
            .backend
            .create(handle, BackendCreateDesc::Sampler(&desc))?;
        self.metrics.resource_creates += 1;
        self.samplers.insert_at(
            handle,
            ResourceRecord {
                desc,
                token,
                last_submission: None,
            },
        )
    }

    /// Creates a logical texture-view/sampler pairing. This is separate from
    /// either child resource so a source shader can require one combined
    /// sampling binding without exposing backend descriptor mechanics.
    pub fn create_combined_texture_sampler(
        &mut self,
        desc: CombinedTextureSamplerDesc,
    ) -> GalResult<Handle> {
        let view = self.texture_view_info(desc.texture_view)?;
        let sampler = self.samplers.get(desc.sampler)?;
        if matches!(
            std::env::var("MATTMC_RUST_SOURCE_DEPTH_TRACE").as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE")
        ) && desc.label.contains("source-main_depth")
        {
            eprintln!(
                "[MattMC source-depth-trace] combined label={} view=0x{:016x} texture=0x{:016x} sampler=0x{:016x}",
                desc.label,
                desc.texture_view.raw(),
                view.texture.raw(),
                desc.sampler.raw(),
            );
        }
        if !view.usages.contains(&TextureUsage::Sampled) {
            return self.validation_error(GalError::resource(
                StatusCode::InvalidArgument,
                "combined texture sampler requires sampled texture usage",
            ));
        }
        if view.format == TextureFormat::R8Uint
            && (sampler.desc.min_filter != SamplerFilter::Nearest
                || sampler.desc.mag_filter != SamplerFilter::Nearest
                || sampler.desc.mip_filter != SamplerFilter::Nearest)
        {
            return self.validation_error(GalError::resource(
                StatusCode::InvalidArgument,
                "integer combined texture samplers require nearest min, mag, and mip filters",
            ));
        }
        if sampler.desc.comparison.is_some() && !is_depth_format(view.format) {
            return self.validation_error(GalError::resource(
                StatusCode::InvalidArgument,
                "comparison combined texture samplers require a depth texture view",
            ));
        }
        let handle = self.combined_texture_samplers.next_handle()?;
        let token = self
            .backend
            .create(handle, BackendCreateDesc::CombinedTextureSampler(&desc))?;
        self.add_dependency(desc.texture_view, handle);
        self.add_dependency(desc.sampler, handle);
        self.metrics.resource_creates += 1;
        self.combined_texture_samplers.insert_at(
            handle,
            ResourceRecord {
                desc,
                token,
                last_submission: None,
            },
        )
    }

    pub fn create_shader_module(&mut self, desc: ShaderModuleDesc) -> GalResult<Handle> {
        if desc.code.is_empty() || desc.entry_point.trim().is_empty() {
            return self.validation_error(GalError::resource(
                StatusCode::InvalidArgument,
                "shader code and entry point must be non-empty",
            ));
        }
        let handle = self.shaders.next_handle()?;
        let token = self
            .backend
            .create(handle, BackendCreateDesc::ShaderModule(&desc))?;
        self.metrics.resource_creates += 1;
        self.shaders.insert_at(
            handle,
            ResourceRecord {
                desc,
                token,
                last_submission: None,
            },
        )
    }

    pub fn create_resource_layout(&mut self, desc: ResourceLayoutDesc) -> GalResult<Handle> {
        let capabilities = self.capabilities();
        if desc.bindings.len() > capabilities.limits.max_resource_layout_bindings as usize {
            return self.unsupported(format!(
                "resource layout '{}' binding count {} exceeds backend '{}' limit {}",
                desc.label,
                desc.bindings.len(),
                capabilities.name,
                capabilities.limits.max_resource_layout_bindings
            ));
        }
        let mut seen = BTreeSet::new();
        for binding in &desc.bindings {
            if !seen.insert(binding.binding) {
                return self.validation_error(GalError::resource(
                    StatusCode::InvalidArgument,
                    format!("duplicate resource binding {}", binding.binding),
                ));
            }
            if binding.array_count == 0 {
                return self.validation_error(GalError::resource(
                    StatusCode::InvalidArgument,
                    format!("binding {} array count must be non-zero", binding.binding),
                ));
            }
            if binding.array_count > 1 && !capabilities.supports(BackendFeature::DescriptorArrays) {
                return self.unsupported(format!(
                    "backend '{}' does not support descriptor arrays",
                    capabilities.name
                ));
            }
            if binding.array_count > capabilities.limits.max_binding_array_count {
                return self.unsupported(format!(
                    "binding {} array count {} exceeds backend '{}' limit {}",
                    binding.binding,
                    binding.array_count,
                    capabilities.name,
                    capabilities.limits.max_binding_array_count
                ));
            }
            if binding.optional && !capabilities.supports(BackendFeature::OptionalBindings) {
                return self.unsupported(format!(
                    "backend '{}' does not support optional bindings",
                    capabilities.name
                ));
            }
            if binding.stages == PipelineStageFlags::NONE {
                return self.validation_error(GalError::resource(
                    StatusCode::InvalidArgument,
                    format!("binding {} must declare shader stages", binding.binding),
                ));
            }
            if binding.dynamic_offset_count > 0
                && !matches!(
                    binding.kind,
                    ResourceBindingKind::UniformBuffer | ResourceBindingKind::StorageBuffer
                )
            {
                return self.validation_error(GalError::resource(
                    StatusCode::InvalidArgument,
                    format!(
                        "binding {} dynamic offsets are only valid for buffer bindings",
                        binding.binding
                    ),
                ));
            }
            if binding.dynamic_offset_count > 0
                && !capabilities.supports(BackendFeature::DynamicBufferOffsets)
            {
                return self.unsupported(format!(
                    "backend '{}' does not support dynamic buffer offsets",
                    capabilities.name
                ));
            }
            if binding.dynamic_offset_count > capabilities.limits.max_dynamic_offsets_per_binding {
                return self.unsupported(format!(
                    "binding {} dynamic offset count {} exceeds backend '{}' limit {}",
                    binding.binding,
                    binding.dynamic_offset_count,
                    capabilities.name,
                    capabilities.limits.max_dynamic_offsets_per_binding
                ));
            }
            if binding.kind == ResourceBindingKind::StorageTexture
                && !capabilities.supports(BackendFeature::StorageTextures)
            {
                return self.unsupported(format!(
                    "backend '{}' does not support storage texture bindings",
                    capabilities.name
                ));
            }
        }
        let handle = self.resource_layouts.next_handle()?;
        let token = self
            .backend
            .create(handle, BackendCreateDesc::ResourceLayout(&desc))?;
        self.metrics.resource_creates += 1;
        self.resource_layouts.insert_at(
            handle,
            ResourceRecord {
                desc,
                token,
                last_submission: None,
            },
        )
    }

    pub fn create_resource_set(&mut self, desc: ResourceSetDesc) -> GalResult<Handle> {
        let layout_bindings = self
            .resource_layouts
            .get(desc.layout)?
            .desc
            .bindings
            .clone();
        let mut seen = BTreeSet::new();
        let mut populated = BTreeSet::new();
        for binding in &desc.bindings {
            if !seen.insert((binding.binding, binding.array_index)) {
                return self.validation_error(GalError::resource(
                    StatusCode::InvalidArgument,
                    format!(
                        "duplicate resource set binding {}[{}]",
                        binding.binding, binding.array_index
                    ),
                ));
            }
            let Some(expected) = layout_bindings
                .iter()
                .find(|item| item.binding == binding.binding)
            else {
                return self.validation_error(GalError::resource(
                    StatusCode::InvalidArgument,
                    format!(
                        "binding {} is not declared by resource layout",
                        binding.binding
                    ),
                ));
            };
            if binding.array_index >= expected.array_count {
                return self.validation_error(GalError::resource(
                    StatusCode::InvalidArgument,
                    format!(
                        "binding {} array index {} is outside declared count {}",
                        binding.binding, binding.array_index, expected.array_count
                    ),
                ));
            }
            if expected.kind != binding.kind {
                return self.validation_error(GalError::resource(
                    StatusCode::InvalidArgument,
                    format!("binding {} kind mismatch", binding.binding),
                ));
            }
            if binding.dynamic_offsets.len() != expected.dynamic_offset_count as usize {
                return self.validation_error(GalError::resource(
                    StatusCode::InvalidArgument,
                    format!("binding {} dynamic offset count mismatch", binding.binding),
                ));
            }
            self.validate_binding_resource(binding)?;
            self.validate_resource_binding_buffer_range(binding, &binding.dynamic_offsets)?;
            if !binding.access.reads() && !binding.access.writes() {
                return self.validation_error(GalError::resource(
                    StatusCode::InvalidArgument,
                    "resource binding must declare read or write access",
                ));
            }
            populated.insert((binding.binding, binding.array_index));
        }
        for expected in &layout_bindings {
            if expected.optional {
                continue;
            }
            for array_index in 0..expected.array_count {
                if !populated.contains(&(expected.binding, array_index)) {
                    return self.validation_error(GalError::resource(
                        StatusCode::InvalidArgument,
                        format!(
                            "required binding {}[{}] is missing from resource set",
                            expected.binding, array_index
                        ),
                    ));
                }
            }
        }
        self.validate_integer_sampled_texture_sampler_pairing(&desc.bindings)?;
        let handle = self.resource_sets.next_handle()?;
        let token = self
            .backend
            .create(handle, BackendCreateDesc::ResourceSet(&desc))?;
        self.add_dependency(desc.layout, handle);
        for binding in &desc.bindings {
            self.add_dependency(binding.resource, handle);
        }
        self.metrics.resource_creates += 1;
        self.resource_sets.insert_at(
            handle,
            ResourceRecord {
                desc,
                token,
                last_submission: None,
            },
        )
    }

    pub fn create_pipeline_layout(&mut self, desc: PipelineLayoutDesc) -> GalResult<Handle> {
        for layout in &desc.resource_layouts {
            self.resource_layouts.get(*layout)?;
        }
        let handle = self.pipeline_layouts.next_handle()?;
        let token = self
            .backend
            .create(handle, BackendCreateDesc::PipelineLayout(&desc))?;
        for layout in &desc.resource_layouts {
            self.add_dependency(*layout, handle);
        }
        self.metrics.resource_creates += 1;
        self.pipeline_layouts.insert_at(
            handle,
            ResourceRecord {
                desc,
                token,
                last_submission: None,
            },
        )
    }

    pub fn create_graphics_pipeline(&mut self, desc: GraphicsPipelineDesc) -> GalResult<Handle> {
        if matches!(
            std::env::var("MATTMC_RUST_PIPELINE_DEPTH_TRACE").as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE")
        ) && desc.label.to_ascii_lowercase().contains("terrain")
        {
            eprintln!(
                "[MattMC pipeline-depth-trace] label={} depth_format={:?} depth_compare={:?} depth_write={} colors={:?}",
                desc.label, desc.depth_format, desc.depth_compare, desc.depth_write, desc.color_formats
            );
        }
        self.pipeline_layouts.get(desc.layout)?;
        self.require_shader_stage(desc.vertex_shader, ShaderStage::Vertex)?;
        self.require_shader_stage(desc.fragment_shader, ShaderStage::Fragment)?;
        if desc.color_formats.is_empty() && desc.depth_format.is_none() {
            return self.validation_error(GalError::resource(
                StatusCode::InvalidArgument,
                "graphics pipeline needs at least one color or depth format",
            ));
        }
        if let Some(depth_bias) = desc.depth_bias {
            if !depth_bias.is_finite() {
                return self.validation_error(GalError::resource(
                    StatusCode::InvalidArgument,
                    "graphics pipeline depth bias factors must be finite",
                ));
            }
            if desc.depth_format.is_none() || desc.depth_compare.is_none() {
                return self.validation_error(GalError::resource(
                    StatusCode::InvalidArgument,
                    "graphics pipeline depth bias requires an enabled depth attachment and test",
                ));
            }
        }
        if let Some(stencil) = desc.stencil {
            if desc.depth_format != Some(TextureFormat::Depth24Stencil8) {
                return self.validation_error(GalError::resource(
                    StatusCode::InvalidArgument,
                    "stencil state requires a Depth24Stencil8 attachment",
                ));
            }
            if stencil.front.read_mask == 0 && stencil.back.read_mask == 0 {
                return self.validation_error(GalError::resource(
                    StatusCode::InvalidArgument,
                    "stencil state requires a non-zero read mask",
                ));
            }
        }
        let capabilities = self.capabilities();
        if desc.color_formats.len() > capabilities.limits.max_color_attachments as usize {
            return self.unsupported(format!(
                "graphics pipeline '{}' color attachment count {} exceeds backend '{}' limit {}",
                desc.label,
                desc.color_formats.len(),
                capabilities.name,
                capabilities.limits.max_color_attachments
            ));
        }
        if desc.color_formats.len() > 1
            && !capabilities.supports(BackendFeature::MultipleColorAttachments)
        {
            return self.unsupported(format!(
                "backend '{}' does not support multiple color attachments",
                capabilities.name
            ));
        }
        if desc.color_formats.is_empty() && !capabilities.supports(BackendFeature::DepthOnlyPass) {
            return self.unsupported(format!(
                "backend '{}' does not support depth-only graphics passes",
                capabilities.name
            ));
        }
        if desc.blend != BlendMode::Disabled && !capabilities.supports(BackendFeature::BlendedPass)
        {
            return self.unsupported(format!(
                "backend '{}' does not support blended graphics passes",
                capabilities.name
            ));
        }
        let handle = self.graphics_pipelines.next_handle()?;
        let token = self
            .backend
            .create(handle, BackendCreateDesc::GraphicsPipeline(&desc))?;
        self.add_dependency(desc.layout, handle);
        self.add_dependency(desc.vertex_shader, handle);
        self.add_dependency(desc.fragment_shader, handle);
        self.metrics.resource_creates += 1;
        self.graphics_pipelines.insert_at(
            handle,
            ResourceRecord {
                desc,
                token,
                last_submission: None,
            },
        )
    }

    pub fn create_compute_pipeline(&mut self, desc: ComputePipelineDesc) -> GalResult<Handle> {
        let capabilities = self.capabilities();
        if !capabilities.supports(BackendFeature::Compute) {
            return self.unsupported(format!(
                "backend '{}' does not support compute pipelines",
                capabilities.name
            ));
        }
        self.pipeline_layouts.get(desc.layout)?;
        self.require_shader_stage(desc.shader, ShaderStage::Compute)?;
        let handle = self.compute_pipelines.next_handle()?;
        let token = self
            .backend
            .create(handle, BackendCreateDesc::ComputePipeline(&desc))?;
        self.add_dependency(desc.layout, handle);
        self.add_dependency(desc.shader, handle);
        self.metrics.resource_creates += 1;
        self.compute_pipelines.insert_at(
            handle,
            ResourceRecord {
                desc,
                token,
                last_submission: None,
            },
        )
    }

    pub fn create_render_target(&mut self, desc: RenderTargetDesc) -> GalResult<Handle> {
        if desc.color_views.is_empty() && desc.depth_stencil_view.is_none() {
            return self.validation_error(GalError::resource(
                StatusCode::InvalidArgument,
                "render target requires at least one attachment",
            ));
        }
        if desc.extent.width == 0 || desc.extent.height == 0 || desc.extent.depth == 0 {
            return self.validation_error(GalError::resource(
                StatusCode::InvalidArgument,
                "render target extent must be non-empty",
            ));
        }
        let capabilities = self.capabilities();
        if desc.color_views.len() > capabilities.limits.max_color_attachments as usize {
            return self.unsupported(format!(
                "render target '{}' color attachment count {} exceeds backend '{}' limit {}",
                desc.label,
                desc.color_views.len(),
                capabilities.name,
                capabilities.limits.max_color_attachments
            ));
        }
        if desc.color_views.len() > 1
            && !capabilities.supports(BackendFeature::MultipleColorAttachments)
        {
            return self.unsupported(format!(
                "backend '{}' does not support multiple color attachments",
                capabilities.name
            ));
        }
        if desc.color_views.is_empty() && !capabilities.supports(BackendFeature::DepthOnlyPass) {
            return self.unsupported(format!(
                "backend '{}' does not support depth-only render targets",
                capabilities.name
            ));
        }
        for view in &desc.color_views {
            let info = self.texture_view_info(*view)?;
            if is_depth_stencil_format(info.format) {
                return self.validation_error(GalError::resource(
                    StatusCode::InvalidArgument,
                    "color attachment view uses a depth/stencil format",
                ));
            }
            if !info.usages.contains(&TextureUsage::ColorAttachment) {
                return self.validation_error(GalError::resource(
                    StatusCode::InvalidArgument,
                    "color attachment texture lacks color attachment usage",
                ));
            }
            if info.extent != desc.extent {
                return self.validation_error(GalError::resource(
                    StatusCode::InvalidArgument,
                    "color attachment extent does not match render target",
                ));
            }
        }
        if let Some(view) = desc.depth_stencil_view {
            let info = self.texture_view_info(view)?;
            if !is_depth_stencil_format(info.format) {
                return self.validation_error(GalError::resource(
                    StatusCode::InvalidArgument,
                    "depth attachment view does not use a depth/stencil format",
                ));
            }
            if !info.usages.contains(&TextureUsage::DepthStencilAttachment) {
                return self.validation_error(GalError::resource(
                    StatusCode::InvalidArgument,
                    "depth attachment texture lacks depth/stencil attachment usage",
                ));
            }
            if info.extent != desc.extent {
                return self.validation_error(GalError::resource(
                    StatusCode::InvalidArgument,
                    "depth attachment extent does not match render target",
                ));
            }
        }
        let handle = self.render_targets.next_handle()?;
        let token = self
            .backend
            .create(handle, BackendCreateDesc::RenderTarget(&desc))?;
        for view in &desc.color_views {
            self.add_dependency(*view, handle);
        }
        if let Some(view) = desc.depth_stencil_view {
            self.add_dependency(view, handle);
        }
        self.metrics.resource_creates += 1;
        self.render_targets.insert_at(
            handle,
            ResourceRecord {
                desc,
                token,
                last_submission: None,
            },
        )
    }

    pub fn create_render_pass(&mut self, desc: RenderPassDesc) -> GalResult<Handle> {
        let (target_color_formats, target_depth_format) = {
            match desc.target.kind() {
                Some(HandleKind::RenderTarget) => {
                    let target = self.render_targets.get(desc.target)?;
                    let color_views = target.desc.color_views.clone();
                    let depth_view = target.desc.depth_stencil_view;
                    let color_formats = color_views
                        .iter()
                        .map(|view| self.texture_view_info(*view).map(|info| info.format))
                        .collect::<GalResult<Vec<_>>>()?;
                    let depth_format = depth_view
                        .map(|view| self.texture_view_info(view).map(|info| info.format))
                        .transpose()?;
                    (color_formats, depth_format)
                }
                Some(HandleKind::FrameTarget) => {
                    let target = self.frame_targets.get(desc.target)?;
                    (vec![target.desc.color_format], desc.depth_format)
                }
                _ => {
                    return self.validation_error(GalError::resource(
                        StatusCode::WrongHandleType,
                        "render pass target must be a render target or frame target",
                    ))
                }
            }
        };
        if desc.color_formats != target_color_formats {
            return self.validation_error(GalError::resource(
                StatusCode::InvalidArgument,
                "render pass color formats do not match target",
            ));
        }
        // A target may own a depth image while an individual pass deliberately
        // omits it (for example a fullscreen post-process that samples the
        // completed depth texture).  Dynamic rendering permits this subset;
        // requiring an attachment merely because the target has one would
        // force illegal sampled/attachment feedback on Vulkan.
        if desc.depth_format != target_depth_format
            && !(desc.depth_format.is_none() && target_depth_format.is_some())
        {
            return self.validation_error(GalError::resource(
                StatusCode::InvalidArgument,
                "render pass depth format does not match target",
            ));
        }
        let handle = self.render_passes.next_handle()?;
        let token = self
            .backend
            .create(handle, BackendCreateDesc::RenderPass(&desc))?;
        self.add_dependency(desc.target, handle);
        self.metrics.resource_creates += 1;
        self.render_passes.insert_at(
            handle,
            ResourceRecord {
                desc,
                token,
                last_submission: None,
            },
        )
    }

    pub fn destroy(&mut self, handle: Handle) -> GalResult<()> {
        self.ensure_no_dependents(handle)?;
        let owned_frame_depth = if handle.kind() == Some(HandleKind::FrameTarget) {
            self.frame_target_depth_populated.remove(&handle);
            self.frame_target_depth_pending.remove(&handle);
            self.frame_target_depth.remove(&handle)
        } else {
            None
        };
        let pending = match handle.kind() {
            Some(HandleKind::Buffer) => self.remove_record(handle, HandleKind::Buffer)?,
            Some(HandleKind::Texture) => self.remove_record(handle, HandleKind::Texture)?,
            Some(HandleKind::TextureView) => self.remove_record(handle, HandleKind::TextureView)?,
            Some(HandleKind::Sampler) => self.remove_record(handle, HandleKind::Sampler)?,
            Some(HandleKind::CombinedTextureSampler) => {
                self.remove_record(handle, HandleKind::CombinedTextureSampler)?
            }
            Some(HandleKind::ShaderModule) => {
                self.remove_record(handle, HandleKind::ShaderModule)?
            }
            Some(HandleKind::ResourceLayout) => {
                self.remove_record(handle, HandleKind::ResourceLayout)?
            }
            Some(HandleKind::ResourceSet) => self.remove_record(handle, HandleKind::ResourceSet)?,
            Some(HandleKind::PipelineLayout) => {
                self.remove_record(handle, HandleKind::PipelineLayout)?
            }
            Some(HandleKind::GraphicsPipeline) => {
                self.remove_record(handle, HandleKind::GraphicsPipeline)?
            }
            Some(HandleKind::ComputePipeline) => {
                self.remove_record(handle, HandleKind::ComputePipeline)?
            }
            Some(HandleKind::RenderTarget) => {
                self.remove_record(handle, HandleKind::RenderTarget)?
            }
            Some(HandleKind::FrameTarget) => self.remove_record(handle, HandleKind::FrameTarget)?,
            Some(HandleKind::RenderPass) => self.remove_record(handle, HandleKind::RenderPass)?,
            None => {
                return Err(GalError::handle(
                    StatusCode::WrongHandleType,
                    "unknown handle kind",
                ))
            }
        };
        if let Some((depth_texture, depth_view)) = owned_frame_depth {
            // The view owns the dependency edge to the texture, so retire it
            // first. Both remain private Rust GAL resources.
            let _ = self.destroy(depth_view);
            let _ = self.destroy(depth_texture);
        }
        self.remove_reverse_edges(handle);
        if let Some(last_submission) = pending.1 {
            if last_submission > self.completed_submission {
                self.retirement.defer(handle, last_submission);
                self.pending_destroys.insert(handle, pending.0);
                self.metrics.deferred_retires += 1;
                return Ok(());
            }
        }
        self.backend
            .destroy(handle, pending.0.kind, pending.0.token)?;
        self.metrics.resource_destroys += 1;
        Ok(())
    }

    pub fn create_command_list(&mut self, desc: CommandListDesc) -> GalResult<CommandList> {
        let capabilities = self.capabilities();
        if desc.operations.len() > capabilities.limits.max_commands_per_list as usize {
            return self.unsupported(format!(
                "command list '{}' operation count {} exceeds backend '{}' limit {}",
                desc.label,
                desc.operations.len(),
                capabilities.name,
                capabilities.limits.max_commands_per_list
            ));
        }
        self.validate_command_ops(&desc.label, &desc.operations)?;
        Ok(desc.into())
    }

    pub fn submit(&mut self, batch: SubmissionBatch) -> GalResult<SyncToken> {
        self.submit_inner(batch, None)
    }

    pub fn submit_profiled(
        &mut self,
        batch: SubmissionBatch,
        profile: &mut WholeFrameProfile,
    ) -> GalResult<SyncToken> {
        self.submit_inner(batch, Some(profile))
    }

    fn submit_inner(
        &mut self,
        mut batch: SubmissionBatch,
        mut profile: Option<&mut WholeFrameProfile>,
    ) -> GalResult<SyncToken> {
        let submit_started = std::time::Instant::now();
        let creates_before = self.metrics.resource_creates;
        let destroys_before = self.metrics.resource_destroys;
        let backend_metrics_before = self.backend.runtime_metrics();
        if batch.command_lists.is_empty() {
            return self.validation_error(GalError::submission(
                StatusCode::InvalidArgument,
                "submission batch must contain command lists",
            ));
        }
        let capabilities = self.capabilities();
        if batch.command_lists.len() > capabilities.limits.max_command_lists_per_submission as usize
        {
            return self.unsupported(format!(
                "submission '{}' command list count {} exceeds backend '{}' limit {}",
                batch.label,
                batch.command_lists.len(),
                capabilities.name,
                capabilities.limits.max_command_lists_per_submission
            ));
        }
        for list in &batch.command_lists {
            if list.operations.len() > capabilities.limits.max_commands_per_list as usize {
                return self.unsupported(format!(
                    "command list '{}' operation count {} exceeds backend '{}' limit {}",
                    list.label,
                    list.operations.len(),
                    capabilities.name,
                    capabilities.limits.max_commands_per_list
                ));
            }
            let validate_ops_started = std::time::Instant::now();
            self.validate_command_ops(&list.label, &list.operations)?;
            if let Some(profile) = profile.as_deref_mut() {
                profile.gal_validate_ops_nanos = profile
                    .gal_validate_ops_nanos
                    .saturating_add(elapsed_nanos_u64(validate_ops_started));
            }
        }
        let normalization = normalize_submission_batch(&mut batch);
        if let Some(profile) = profile.as_deref_mut() {
            profile.gal_command_ops_before_normalize = profile
                .gal_command_ops_before_normalize
                .saturating_add(normalization.ops_before);
            profile.gal_command_ops_after_normalize = profile
                .gal_command_ops_after_normalize
                .saturating_add(normalization.ops_after);
            profile.gal_redundant_pipeline_binds_removed = profile
                .gal_redundant_pipeline_binds_removed
                .saturating_add(normalization.pipeline_binds_removed);
            profile.gal_redundant_resource_set_binds_removed = profile
                .gal_redundant_resource_set_binds_removed
                .saturating_add(normalization.resource_set_binds_removed);
            profile.gal_redundant_vertex_buffer_binds_removed = profile
                .gal_redundant_vertex_buffer_binds_removed
                .saturating_add(normalization.vertex_buffer_binds_removed);
            profile.gal_redundant_index_buffer_binds_removed = profile
                .gal_redundant_index_buffer_binds_removed
                .saturating_add(normalization.index_buffer_binds_removed);
        }
        let referenced = referenced_handles(&batch);
        let validate_handles_started = std::time::Instant::now();
        for handle in &referenced {
            self.validate_any_resource(*handle)?;
        }
        if let Some(profile) = profile.as_deref_mut() {
            profile.gal_validate_handles_nanos = profile
                .gal_validate_handles_nanos
                .saturating_add(elapsed_nanos_u64(validate_handles_started));
        }
        let hazards_started = std::time::Instant::now();
        self.validate_submission_hazards(&batch, profile.as_deref_mut())?;
        if let Some(profile) = profile.as_deref_mut() {
            profile.gal_hazard_analysis_nanos = profile
                .gal_hazard_analysis_nanos
                .saturating_add(elapsed_nanos_u64(hazards_started));
            add_command_profile(profile, &batch);
        }
        let validated = ValidatedSubmissionBatch::from(batch);
        let id = SubmissionId(self.next_submission);
        self.next_submission += 1;
        submission_trace(&format!(
            "gal.submit.encode.begin id={} label={}",
            id.0, validated.label
        ));
        let backend_encode_started = std::time::Instant::now();
        self.backend.encode_passes(&validated)?;
        submission_trace(&format!("gal.submit.encode.end id={}", id.0));
        if let Some(profile) = profile.as_deref_mut() {
            profile.backend_encode_nanos = profile
                .backend_encode_nanos
                .saturating_add(elapsed_nanos_u64(backend_encode_started));
        }
        let backend_submit_started = std::time::Instant::now();
        submission_trace(&format!("gal.submit.queue.begin id={}", id.0));
        self.backend.submit(id, &validated)?;
        self.latest_accepted_submission = id;
        submission_trace(&format!("gal.submit.queue.end id={}", id.0));
        if let Some(profile) = profile.as_deref_mut() {
            profile.backend_submit_nanos = profile
                .backend_submit_nanos
                .saturating_add(elapsed_nanos_u64(backend_submit_started));
        }
        self.metrics.submissions += 1;
        for handle in referenced {
            self.mark_in_flight(handle, id)?;
        }
        if let Some(profile) = profile.as_deref_mut() {
            profile.gal_submit_total_nanos = profile
                .gal_submit_total_nanos
                .saturating_add(elapsed_nanos_u64(submit_started));
            profile.resource_creates_delta =
                self.metrics.resource_creates.saturating_sub(creates_before);
            profile.resource_destroys_delta = self
                .metrics
                .resource_destroys
                .saturating_sub(destroys_before);
            let backend_metrics_after = self.backend.runtime_metrics();
            add_backend_metric_deltas(profile, backend_metrics_before, backend_metrics_after);
        }
        Ok(SyncToken { submission: id })
    }

    pub fn retire_completed(&mut self) -> GalResult<Vec<Handle>> {
        self.poll_completed();
        self.backend.retire(self.completed_submission)?;
        let mut retired = Vec::new();
        for entry in self.retirement.drain_completed(self.completed_submission) {
            if let Some(pending) = self.pending_destroys.remove(&entry.handle) {
                self.backend
                    .destroy(entry.handle, pending.kind, pending.token)?;
                self.metrics.resource_destroys += 1;
                retired.push(entry.handle);
            }
        }
        Ok(retired)
    }

    pub(in crate::render::vulkanic) fn poll_completed(&mut self) -> SubmissionId {
        let completed = self.backend.completed_submission();
        if completed > self.completed_submission {
            self.completed_submission = completed;
        }
        self.completed_submission
    }

    pub(in crate::render::vulkanic) fn latest_submission_id(&self) -> SubmissionId {
        self.latest_accepted_submission
    }

    pub(in crate::render::vulkanic) fn render_pass_last_submission(
        &self, pass: Handle,
    ) -> GalResult<Option<SubmissionId>> {
        Ok(self.render_passes.get(pass)?.last_submission)
    }

    /// Exposes the id that the immediately following submission will receive.
    /// Frontends use this to protect transient stream ranges until that
    /// submission completes; taking this value must be followed by one submit.
    pub(in crate::render::vulkanic) fn next_submission_id(&self) -> SubmissionId {
        SubmissionId(self.next_submission)
    }

    pub(in crate::render::vulkanic) fn retire_through(
        &mut self,
        id: SubmissionId,
    ) -> GalResult<Vec<Handle>> {
        if id > self.latest_submission_id() {
            return Err(GalError::invalid_argument("cannot wait for an unsubmitted completion id"));
        }
        self.backend.retire(id)?;
        if id > self.completed_submission {
            self.completed_submission = id;
        }
        let mut retired = Vec::new();
        for entry in self.retirement.drain_completed(self.completed_submission) {
            if let Some(pending) = self.pending_destroys.remove(&entry.handle) {
                self.backend
                    .destroy(entry.handle, pending.kind, pending.token)?;
                self.metrics.resource_destroys += 1;
                retired.push(entry.handle);
            }
        }
        Ok(retired)
    }

    pub(in crate::render::vulkanic) fn completed_host_reads(&self) -> Vec<CompletedHostRead> {
        self.backend.completed_host_reads()
    }

    fn validation_error<T>(&mut self, error: GalError) -> GalResult<T> {
        self.metrics.validation_failures += 1;
        Err(error)
    }

    fn unsupported<T>(&mut self, message: impl Into<String>) -> GalResult<T> {
        self.validation_error(GalError::unsupported_feature(message))
    }

    fn add_dependency(&mut self, resource: Handle, dependent: Handle) {
        self.dependencies
            .entry(resource)
            .or_default()
            .insert(dependent);
        self.reverse_dependencies
            .entry(dependent)
            .or_default()
            .insert(resource);
    }

    fn remove_reverse_edges(&mut self, dependent: Handle) {
        if let Some(resources) = self.reverse_dependencies.remove(&dependent) {
            for resource in resources {
                if let Some(dependents) = self.dependencies.get_mut(&resource) {
                    dependents.remove(&dependent);
                    if dependents.is_empty() {
                        self.dependencies.remove(&resource);
                    }
                }
            }
        }
    }

    fn ensure_no_dependents(&mut self, handle: Handle) -> GalResult<()> {
        if let Some(dependents) = self.dependencies.get(&handle) {
            if !dependents.is_empty() {
                let label = self.dependency_debug_label(handle);
                let dependents = dependents
                    .iter()
                    .map(|dependent| {
                        format!(
                            "0x{:016x} ({})",
                            dependent.raw(),
                            self.dependency_debug_label(*dependent)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                return self.validation_error(GalError::resource(
                    StatusCode::DependencyViolation,
                    format!(
                        "resource 0x{:016x} ({label}) has live dependents [{dependents}]",
                        handle.raw()
                    ),
                ));
            }
        }
        Ok(())
    }

    fn dependency_debug_label(&self, handle: Handle) -> &str {
        match handle.kind() {
            Some(HandleKind::TextureView) => self
                .texture_views
                .get(handle)
                .map(|record| record.desc.label.as_str())
                .unwrap_or("<stale-texture-view>"),
            Some(HandleKind::ResourceSet) => self
                .resource_sets
                .get(handle)
                .map(|record| record.desc.label.as_str())
                .unwrap_or("<stale-resource-set>"),
            _ => "<unlabeled-resource>",
        }
    }

    fn validate_binding_resource(&mut self, binding: &ResourceBinding) -> GalResult<()> {
        match binding.kind {
            ResourceBindingKind::UniformBuffer => {
                let record = self.buffers.get(binding.resource)?;
                if !record.desc.usages.contains(&BufferUsage::Uniform) {
                    return self.validation_error(GalError::resource(
                        StatusCode::InvalidArgument,
                        "uniform buffer binding requires uniform buffer usage",
                    ));
                }
                if binding.access.writes() {
                    return self.validation_error(GalError::resource(
                        StatusCode::InvalidArgument,
                        "uniform buffer binding cannot declare write access",
                    ));
                }
            }
            ResourceBindingKind::StorageBuffer => {
                let record = self.buffers.get(binding.resource)?;
                if !record.desc.usages.contains(&BufferUsage::Storage) {
                    return self.validation_error(GalError::resource(
                        StatusCode::InvalidArgument,
                        "storage buffer binding requires storage buffer usage",
                    ));
                }
            }
            ResourceBindingKind::SampledTexture => {
                let info = self.texture_view_info(binding.resource)?;
                if !info.usages.contains(&TextureUsage::Sampled) {
                    return self.validation_error(GalError::resource(
                        StatusCode::InvalidArgument,
                        "sampled texture binding requires sampled texture usage",
                    ));
                }
                if binding.access.writes() {
                    return self.validation_error(GalError::resource(
                        StatusCode::InvalidArgument,
                        "sampled texture binding cannot declare write access",
                    ));
                }
            }
            ResourceBindingKind::CombinedTextureSampler => {
                let pair = self.combined_texture_samplers.get(binding.resource)?;
                let view = self.texture_view_info(pair.desc.texture_view)?;
                if !view.usages.contains(&TextureUsage::Sampled) {
                    return self.validation_error(GalError::resource(
                        StatusCode::InvalidArgument,
                        "combined texture sampler requires sampled texture usage",
                    ));
                }
                if binding.access.writes() {
                    return self.validation_error(GalError::resource(
                        StatusCode::InvalidArgument,
                        "combined texture sampler binding cannot declare write access",
                    ));
                }
            }
            ResourceBindingKind::StorageTexture => {
                let info = self.texture_view_info(binding.resource)?;
                if !info.usages.contains(&TextureUsage::Storage) {
                    return self.validation_error(GalError::resource(
                        StatusCode::InvalidArgument,
                        "storage texture binding requires storage texture usage",
                    ));
                }
            }
            ResourceBindingKind::Sampler => {
                self.samplers.get(binding.resource)?;
                if binding.access.writes() {
                    return self.validation_error(GalError::resource(
                        StatusCode::InvalidArgument,
                        "sampler binding cannot declare write access",
                    ));
                }
            }
        }
        Ok(())
    }

    fn validate_integer_sampled_texture_sampler_pairing(
        &mut self,
        bindings: &[ResourceBinding],
    ) -> GalResult<()> {
        // The current semantic resource-set model exposes samplers separately
        // from sampled textures. Both native backends bind those sampler
        // objects to the sampled set, so an integer texture cannot safely
        // coexist with linear sampler state in that set. Keep the restriction
        // here until the ABI gains an explicit texture-to-sampler association.
        let has_integer_sampled_texture = bindings.iter().any(|binding| {
            binding.kind == ResourceBindingKind::SampledTexture
                && self
                    .texture_view_info(binding.resource)
                    .map(|info| info.format == TextureFormat::R8Uint)
                    .unwrap_or(false)
        });
        if !has_integer_sampled_texture {
            return Ok(());
        }
        for binding in bindings {
            if binding.kind != ResourceBindingKind::Sampler {
                continue;
            }
            let sampler = self.samplers.get(binding.resource)?;
            if sampler.desc.min_filter != SamplerFilter::Nearest
                || sampler.desc.mag_filter != SamplerFilter::Nearest
                || sampler.desc.mip_filter != SamplerFilter::Nearest
            {
                return self.validation_error(GalError::resource(
                    StatusCode::InvalidArgument,
                    "integer sampled textures require nearest min, mag, and mip sampler filters in the same resource set",
                ));
            }
        }
        Ok(())
    }

    fn validate_resource_binding_buffer_range(
        &mut self,
        binding: &ResourceBinding,
        offsets: &[u64],
    ) -> GalResult<()> {
        if binding.buffer_range.is_some()
            && !matches!(
                binding.kind,
                ResourceBindingKind::UniformBuffer | ResourceBindingKind::StorageBuffer
            )
        {
            return self.validation_error(GalError::resource(
                StatusCode::InvalidArgument,
                format!(
                    "binding {} buffer range is only valid for buffer bindings",
                    binding.binding
                ),
            ));
        }
        if !matches!(
            binding.kind,
            ResourceBindingKind::UniformBuffer | ResourceBindingKind::StorageBuffer
        ) {
            return Ok(());
        }
        let record = self.buffers.get(binding.resource)?;
        let range = binding.buffer_range.unwrap_or_else(|| {
            let max_default_offset = binding.dynamic_offsets.iter().copied().max().unwrap_or(0);
            record.desc.size.saturating_sub(max_default_offset)
        });
        if range == 0 {
            return self.validation_error(GalError::resource(
                StatusCode::InvalidArgument,
                format!("binding {} buffer range must be non-zero", binding.binding),
            ));
        }
        for offset in offsets {
            let Some(end) = offset.checked_add(range) else {
                return self.validation_error(GalError::resource(
                    StatusCode::InvalidArgument,
                    format!("binding {} dynamic buffer range overflows", binding.binding),
                ));
            };
            if end > record.desc.size {
                return self.validation_error(GalError::resource(
                    StatusCode::InvalidArgument,
                    format!(
                        "binding {} dynamic buffer range is outside the buffer",
                        binding.binding
                    ),
                ));
            }
        }
        Ok(())
    }

    fn require_shader_stage(&self, handle: Handle, stage: ShaderStage) -> GalResult<()> {
        let shader = self.shaders.get(handle)?;
        if shader.desc.stage != stage {
            return Err(GalError::resource(
                StatusCode::InvalidArgument,
                format!(
                    "shader stage mismatch: expected {stage:?}, got {:?}",
                    shader.desc.stage
                ),
            ));
        }
        Ok(())
    }

    fn validate_command_ops(&mut self, label: &str, ops: &[CommandOp]) -> GalResult<()> {
        let capabilities = self.capabilities();
        let mut in_pass = false;
        let mut active_pass = None;
        let mut graphics_pipeline = None;
        let mut compute_pipeline = None;
        let mut active_pipeline_layout = None;
        let mut index_buffer = None;
        for (op_index, op) in ops.iter().enumerate() {
            match op {
                CommandOp::BeginPass {
                    pass,
                    target,
                    colors,
                    depth_stencil,
                } => {
                    if in_pass {
                        return self.validation_error(GalError::command(
                            StatusCode::InvalidArgument,
                            "nested passes are invalid",
                        ));
                    }
                    let pass_target = self.render_pass_desc(*pass)?.target;
                    let frame_target = match target.kind() {
                        Some(HandleKind::FrameTarget) => {
                            self.frame_targets.get(*target)?;
                            true
                        }
                        Some(HandleKind::RenderTarget) => false,
                        _ => {
                            return self.validation_error(GalError::command(
                                StatusCode::WrongHandleType,
                                "pass target must be a render target or frame target",
                            ))
                        }
                    };
                    let (expected_colors, expected_depth) = if frame_target {
                        let pass_desc = self.render_pass_desc(*pass)?;
                        let expected_colors = if pass_desc.color_formats.is_empty() {
                            Vec::new()
                        } else {
                            vec![*target]
                        };
                        let pass_depth = pass_desc.depth_format;
                        if let Some(format) = pass_depth {
                            let attachment = depth_stencil.as_ref().ok_or_else(|| {
                                GalError::command(
                                    StatusCode::InvalidArgument,
                                    format!(
                                        "frame-target pass '{}' with depth format requires a depth attachment",
                                        pass_desc.label
                                    ),
                                )
                            })?;
                            let info = self.texture_view_info(attachment.view)?;
                            if info.format != format {
                                return self.validation_error(GalError::command(
                                    StatusCode::InvalidArgument,
                                    "frame-target pass depth attachment format does not match render pass",
                                ));
                            }
                            (expected_colors, Some(attachment.view))
                        } else {
                            (expected_colors, None)
                        }
                    } else {
                        let target_record = self.render_targets.get(*target)?;
                        (
                            target_record.desc.color_views.clone(),
                            target_record.desc.depth_stencil_view,
                        )
                    };
                    if pass_target != *target {
                        return self.validation_error(GalError::command(
                            StatusCode::InvalidArgument,
                            "pass target does not match command target",
                        ));
                    }
                    if colors.len() != expected_colors.len() {
                        return self.validation_error(GalError::command(
                            StatusCode::InvalidArgument,
                            "pass color attachment count does not match target",
                        ));
                    }
                    for (index, color) in colors.iter().enumerate() {
                        if color.view != expected_colors[index] {
                            return self.validation_error(GalError::command(
                                StatusCode::InvalidArgument,
                                "pass color attachment view does not match target",
                            ));
                        }
                    }
                    if depth_stencil.is_some() && expected_depth.is_none() {
                        return self.validation_error(GalError::command(
                            StatusCode::InvalidArgument,
                            "pass depth attachment presence does not match target",
                        ));
                    }
                    if let (Some(depth), Some(expected_depth)) = (depth_stencil, expected_depth) {
                        if depth.view != expected_depth {
                            return self.validation_error(GalError::command(
                                StatusCode::InvalidArgument,
                                "pass depth attachment view does not match target",
                            ));
                        }
                    }
                    in_pass = true;
                    active_pass = Some(*pass);
                }
                CommandOp::BindGraphicsPipeline(handle) => {
                    let layout = self.graphics_pipelines.get(*handle)?.desc.layout;
                    if let Some(pass) = active_pass {
                        self.validate_pipeline_pass_compatibility(*handle, pass)?;
                    }
                    graphics_pipeline = Some(*handle);
                    compute_pipeline = None;
                    active_pipeline_layout = Some(layout);
                }
                CommandOp::BindComputePipeline(handle) => {
                    if in_pass {
                        return self.validation_error(GalError::command(
                            StatusCode::InvalidArgument,
                            "compute pipeline cannot be bound inside a render pass",
                        ));
                    }
                    let layout = self.compute_pipelines.get(*handle)?.desc.layout;
                    compute_pipeline = Some(*handle);
                    graphics_pipeline = None;
                    active_pipeline_layout = Some(layout);
                }
                CommandOp::BindResourceSet {
                    pipeline_layout,
                    set_index,
                    set,
                    dynamic_offsets,
                } => {
                    if active_pipeline_layout != Some(*pipeline_layout) {
                        return self.validation_error(GalError::command(
                            StatusCode::InvalidArgument,
                            "resource set pipeline layout does not match active pipeline",
                        ));
                    }
                    let layout = self.pipeline_layouts.get(*pipeline_layout)?;
                    let set_record = self.resource_sets.get(*set)?;
                    let Some(expected_layout) =
                        layout.desc.resource_layouts.get(*set_index as usize)
                    else {
                        return self.validation_error(GalError::command(
                            StatusCode::InvalidArgument,
                            "resource set index is outside pipeline layout",
                        ));
                    };
                    if *expected_layout != set_record.desc.layout {
                        return self.validation_error(GalError::command(
                            StatusCode::InvalidArgument,
                            "resource set layout is not compatible with pipeline layout",
                        ));
                    }
                    let set_bindings = set_record.desc.bindings.clone();
                    let expected_dynamic_offsets: usize = set_bindings
                        .iter()
                        .map(|binding| binding.dynamic_offsets.len())
                        .sum();
                    if !dynamic_offsets.is_empty()
                        && dynamic_offsets.len() != expected_dynamic_offsets
                    {
                        return self.validation_error(GalError::command(
                            StatusCode::InvalidArgument,
                            format!(
                                "resource set bind dynamic offset count {} does not match expected {}",
                                dynamic_offsets.len(),
                                expected_dynamic_offsets
                            ),
                        ));
                    }
                    let mut offset_index = 0usize;
                    for binding in &set_bindings {
                        let count = binding.dynamic_offsets.len();
                        if count == 0 {
                            continue;
                        }
                        let offsets = if dynamic_offsets.is_empty() {
                            binding.dynamic_offsets.as_slice()
                        } else {
                            let end = offset_index + count;
                            let slice = &dynamic_offsets[offset_index..end];
                            offset_index = end;
                            slice
                        };
                        self.validate_resource_binding_buffer_range(binding, offsets)?;
                    }
                }
                CommandOp::SetVertexBuffer { buffer, .. } => {
                    let record = self.buffers.get(*buffer)?;
                    if !record.desc.usages.contains(&BufferUsage::Vertex) {
                        return self.validation_error(GalError::command(
                            StatusCode::InvalidArgument,
                            "vertex buffer binding requires vertex buffer usage",
                        ));
                    }
                }
                CommandOp::SetIndexBuffer {
                    buffer,
                    offset,
                    index_type,
                } => {
                    let record = self.buffers.get(*buffer)?;
                    if !record.desc.usages.contains(&BufferUsage::Index) {
                        return self.validation_error(GalError::command(
                            StatusCode::InvalidArgument,
                            "index buffer binding requires index buffer usage",
                        ));
                    }
                    let index_size = index_type_size(*index_type);
                    if *offset % index_size != 0 {
                        return self.validation_error(GalError::command(
                            StatusCode::InvalidArgument,
                            "index buffer offset must be aligned to index type size",
                        ));
                    }
                    if *offset >= record.desc.size {
                        return self.validation_error(GalError::command(
                            StatusCode::InvalidArgument,
                            "index buffer offset is outside the buffer",
                        ));
                    }
                    index_buffer = Some((*buffer, *offset, *index_type));
                }
                CommandOp::Draw {
                    vertices,
                    instances,
                } => {
                    if !in_pass || graphics_pipeline.is_none() || *vertices == 0 || *instances == 0
                    {
                        return self.validation_error(GalError::command(
                            StatusCode::InvalidArgument,
                            "draw requires active pass, graphics pipeline, and non-zero counts",
                        ));
                    }
                }
                CommandOp::DrawIndexed { indices, instances } => {
                    if !in_pass || graphics_pipeline.is_none() || *indices == 0 || *instances == 0 {
                        return self.validation_error(GalError::command(StatusCode::InvalidArgument, "indexed draw requires active pass, pipeline, index buffer, and non-zero counts"));
                    }
                    let Some((buffer, offset, index_type)) = index_buffer else {
                        return self.validation_error(GalError::command(
                            StatusCode::InvalidArgument,
                            "indexed draw requires active index buffer",
                        ));
                    };
                    let bytes = u64::from(*indices)
                        .checked_mul(index_type_size(index_type))
                        .ok_or_else(|| {
                            GalError::command(
                                StatusCode::InvalidArgument,
                                "indexed draw byte range overflows",
                            )
                        })?;
                    self.validate_buffer_range(buffer, offset, bytes, BufferUsage::Index)?;
                }
                CommandOp::DrawIndirect {
                    buffer,
                    offset,
                    draw_count,
                } => {
                    if !capabilities.supports(BackendFeature::IndirectDraw) {
                        return self.unsupported(format!(
                            "backend '{}' does not support indirect draw commands",
                            capabilities.name
                        ));
                    }
                    if !in_pass || graphics_pipeline.is_none() || *draw_count == 0 {
                        return self.validation_error(GalError::command(
                            StatusCode::InvalidArgument,
                            "indirect draw requires active pass, graphics pipeline, and non-zero draw count",
                        ));
                    }
                    if *draw_count > capabilities.limits.max_draw_count {
                        return self.unsupported(format!(
                            "indirect draw count {} exceeds backend '{}' limit {}",
                            draw_count, capabilities.name, capabilities.limits.max_draw_count
                        ));
                    }
                    self.validate_buffer_range(*buffer, *offset, 1, BufferUsage::Indirect)?;
                }
                CommandOp::Dispatch {
                    groups_x,
                    groups_y,
                    groups_z,
                } => {
                    if !capabilities.supports(BackendFeature::Compute) {
                        return self.unsupported(format!(
                            "backend '{}' does not support dispatch commands",
                            capabilities.name
                        ));
                    }
                    if in_pass
                        || compute_pipeline.is_none()
                        || *groups_x == 0
                        || *groups_y == 0
                        || *groups_z == 0
                    {
                        return self.validation_error(GalError::command(StatusCode::InvalidArgument, "dispatch requires compute pipeline outside render pass and non-zero groups"));
                    }
                    if *groups_x > capabilities.limits.max_dispatch_groups_per_axis
                        || *groups_y > capabilities.limits.max_dispatch_groups_per_axis
                        || *groups_z > capabilities.limits.max_dispatch_groups_per_axis
                    {
                        return self.unsupported(format!(
                            "dispatch group count exceeds backend '{}' limit {}",
                            capabilities.name, capabilities.limits.max_dispatch_groups_per_axis
                        ));
                    }
                }
                CommandOp::DispatchIndirect { buffer, offset } => {
                    if !capabilities.supports(BackendFeature::IndirectDispatch) {
                        return self.unsupported(format!(
                            "backend '{}' does not support indirect dispatch commands",
                            capabilities.name
                        ));
                    }
                    if in_pass || compute_pipeline.is_none() {
                        return self.validation_error(GalError::command(
                            StatusCode::InvalidArgument,
                            "indirect dispatch requires compute pipeline outside render pass",
                        ));
                    }
                    self.validate_buffer_range(*buffer, *offset, 1, BufferUsage::Indirect)?;
                }
                CommandOp::CopyBuffer { src, dst, size } => {
                    if in_pass {
                        return self.validation_error(GalError::command(
                            StatusCode::InvalidArgument,
                            "buffer copy requires no active render pass",
                        ));
                    }
                    let src_record = self.buffers.get(*src)?;
                    let src_ok = src_record.desc.usages.contains(&BufferUsage::TransferSrc);
                    let dst_record = self.buffers.get(*dst)?;
                    let dst_ok = dst_record.desc.usages.contains(&BufferUsage::TransferDst);
                    if src == dst || *size == 0 {
                        return self.validation_error(GalError::command(
                            StatusCode::InvalidArgument,
                            "buffer copy requires distinct buffers and non-zero size",
                        ));
                    }
                    if !src_ok || !dst_ok {
                        return self.validation_error(GalError::command(
                            StatusCode::InvalidArgument,
                            "buffer copy requires transfer source and destination usages",
                        ));
                    }
                }
                CommandOp::CopyBufferToTexture(region) => {
                    if in_pass {
                        return self.validation_error(GalError::command(
                            StatusCode::InvalidArgument,
                            "buffer-to-texture copy requires no active render pass",
                        ));
                    }
                    if (region.texture_mip > 0 || region.texture_layer > 0)
                        && !capabilities.supports(BackendFeature::TextureSubresourceCopies)
                    {
                        return self.unsupported(format!(
                            "backend '{}' does not support texture subresource copies",
                            capabilities.name
                        ));
                    }
                    self.validate_buffer_texture_copy_region(
                        region,
                        BufferUsage::TransferSrc,
                        TextureUsage::TransferDst,
                    )?;
                }
                CommandOp::CopyTextureToBuffer(region) => {
                    if in_pass {
                        return self.validation_error(GalError::command(
                            StatusCode::InvalidArgument,
                            "texture-to-buffer copy requires no active render pass",
                        ));
                    }
                    if (region.texture_mip > 0 || region.texture_layer > 0)
                        && !capabilities.supports(BackendFeature::TextureSubresourceCopies)
                    {
                        return self.unsupported(format!(
                            "backend '{}' does not support texture subresource copies",
                            capabilities.name
                        ));
                    }
                    self.validate_buffer_texture_copy_region(
                        region,
                        BufferUsage::TransferDst,
                        TextureUsage::TransferSrc,
                    )?;
                }
                CommandOp::CopyTexture(region) => {
                    if in_pass {
                        return self.validation_error(GalError::command(
                            StatusCode::InvalidArgument,
                            "texture copy requires no active render pass",
                        ));
                    }
                    self.validate_texture_copy_region(region)?;
                }
                CommandOp::CopyFrameTargetToTexture { src, dst, extent } => {
                    if in_pass {
                        return self.validation_error(GalError::command(
                            StatusCode::InvalidArgument,
                            "frame-target copy requires no active render pass",
                        ));
                    }
                    self.validate_frame_target_copy(*src, *dst, *extent)?;
                }
                CommandOp::CopyTextureToFrameTarget { src, dst, extent } => {
                    if in_pass {
                        return self.validation_error(GalError::command(
                            StatusCode::InvalidArgument,
                            "texture-to-frame-target copy requires no active render pass",
                        ));
                    }
                    self.validate_texture_to_frame_target_copy(*src, *dst, *extent)?;
                }
                CommandOp::GenerateMipmaps {
                    texture,
                    subresources,
                } => {
                    if in_pass {
                        return self.validation_error(GalError::command(
                            StatusCode::InvalidArgument,
                            "mip generation requires no active render pass",
                        ));
                    }
                    if !capabilities.supports(BackendFeature::TextureMipLevels) {
                        return self.unsupported(format!(
                            "backend '{}' does not support texture mip generation",
                            capabilities.name
                        ));
                    }
                    let texture_record = self.textures.get(*texture)?;
                    if !texture_record
                        .desc
                        .usages
                        .contains(&TextureUsage::TransferSrc)
                        || !texture_record
                            .desc
                            .usages
                            .contains(&TextureUsage::TransferDst)
                    {
                        return self.validation_error(GalError::command(
                            StatusCode::InvalidArgument,
                            "mip generation requires transfer source and destination texture usages",
                        ));
                    }
                    if is_depth_format(texture_record.desc.format)
                        || texture_record.desc.format == TextureFormat::R8Uint
                    {
                        return self.unsupported(
                            "mip generation requires a non-depth, non-integer color texture",
                        );
                    }
                    if subresources.mip_count < 2 {
                        return self.validation_error(GalError::command(
                            StatusCode::InvalidArgument,
                            "mip generation requires a range containing a source mip and at least one destination mip",
                        ));
                    }
                    self.validate_texture_subresource_range(*texture, *subresources)?;
                }
                CommandOp::HostWriteBuffer {
                    buffer,
                    offset,
                    data,
                } => {
                    if in_pass {
                        return self.validation_error(GalError::command(
                            StatusCode::InvalidArgument,
                            "host writes require no active render pass",
                        ));
                    }
                    if data.is_empty() {
                        return self.validation_error(GalError::command(
                            StatusCode::InvalidArgument,
                            format!(
                                "command list '{label}' operation {op_index} host write payload must be non-zero"
                            ),
                        ));
                    }
                    if !capabilities.supports(BackendFeature::HostBufferAccess) {
                        return self.unsupported(format!(
                            "backend '{}' does not support host buffer writes",
                            capabilities.name
                        ));
                    }
                    self.validate_buffer_range(
                        *buffer,
                        *offset,
                        data.len() as u64,
                        BufferUsage::HostWrite,
                    )?;
                    let record = self.buffers.get(*buffer)?;
                    if record.desc.memory != MemoryDomain::Upload {
                        return self.validation_error(GalError::command(
                            StatusCode::InvalidArgument,
                            "host writes require upload memory",
                        ));
                    }
                }
                CommandOp::HostReadBuffer {
                    buffer,
                    offset,
                    size,
                } => {
                    if in_pass {
                        return self.validation_error(GalError::command(
                            StatusCode::InvalidArgument,
                            "host reads require no active render pass",
                        ));
                    }
                    if *size == 0 {
                        return self.validation_error(GalError::command(
                            StatusCode::InvalidArgument,
                            format!(
                                "command list '{label}' operation {op_index} host read size must be non-zero"
                            ),
                        ));
                    }
                    if !capabilities.supports(BackendFeature::HostBufferAccess) {
                        return self.unsupported(format!(
                            "backend '{}' does not support host buffer reads",
                            capabilities.name
                        ));
                    }
                    self.validate_buffer_range(*buffer, *offset, *size, BufferUsage::HostRead)?;
                    let record = self.buffers.get(*buffer)?;
                    if record.desc.memory != MemoryDomain::Readback {
                        return self.validation_error(GalError::command(
                            StatusCode::InvalidArgument,
                            "host reads require readback memory",
                        ));
                    }
                }
                CommandOp::Present {
                    texture,
                    subresources,
                } => {
                    if !capabilities.supports(BackendFeature::Presentation) {
                        return self.unsupported(format!(
                            "backend '{}' does not support presentation commands in the isolated path",
                            capabilities.name
                        ));
                    }
                    let record = self.textures.get(*texture)?;
                    if !record.desc.usages.contains(&TextureUsage::Present) {
                        return self.validation_error(GalError::command(
                            StatusCode::InvalidArgument,
                            "presentation requires present texture usage",
                        ));
                    }
                    self.validate_texture_subresource_range(*texture, *subresources)?;
                }
                CommandOp::Barrier(barrier) => {
                    if in_pass {
                        return self.validation_error(GalError::command(
                            StatusCode::InvalidArgument,
                            "resource barriers require no active render pass",
                        ));
                    }
                    self.validate_any_resource(barrier.resource)?;
                    if let Some(range) = barrier.subresources {
                        match barrier.resource.kind() {
                            Some(HandleKind::Texture) => {
                                self.validate_texture_subresource_range(barrier.resource, range)?;
                            }
                            Some(HandleKind::TextureView) => {
                                let info = self.texture_view_info(barrier.resource)?;
                                self.validate_texture_subresource_range(info.texture, range)?;
                                if !texture_range_contains(info.range, range) {
                                    return self.validation_error(GalError::command(
                                        StatusCode::InvalidArgument,
                                        "barrier subresource range is outside the texture view",
                                    ));
                                }
                            }
                            _ => {
                                return self.validation_error(GalError::command(
                                    StatusCode::InvalidArgument,
                                    "barrier subresource ranges are only valid for textures",
                                ));
                            }
                        }
                    }
                    // A write-after-write/read dependency is meaningful even
                    // when the resource keeps its layout and semantic usage.
                    // Read-only same-usage barriers still describe no work.
                    if barrier.before == barrier.after && !matches!(barrier.before,
                        TextureUsageState::ShaderWrite | TextureUsageState::ColorAttachment
                        | TextureUsageState::DepthStencilAttachment | TextureUsageState::TransferDst)
                    {
                        return self.validation_error(GalError::command(
                            StatusCode::InvalidArgument,
                            "barrier must declare a semantic state change or write dependency",
                        ));
                    }
                    if barrier.src_queue == QueueClass::Present
                        || barrier.dst_queue == QueueClass::Present
                    {
                        let texture = match barrier.resource.kind() {
                            Some(HandleKind::Texture) => self.textures.get(barrier.resource)?,
                            Some(HandleKind::TextureView) => {
                                let info = self.texture_view_info(barrier.resource)?;
                                self.textures.get(info.texture)?
                            }
                            _ => {
                                return self.validation_error(GalError::command(
                                    StatusCode::InvalidArgument,
                                    "presentation queue ownership requires a texture or texture view",
                                ));
                            }
                        };
                        if !texture.desc.usages.contains(&TextureUsage::Present) {
                            return self.validation_error(GalError::command(
                                StatusCode::InvalidArgument,
                                "queue ownership transfer involving presentation requires present texture usage",
                            ));
                        }
                    }
                }
                CommandOp::EndPass => {
                    if !in_pass {
                        return self.validation_error(GalError::command(
                            StatusCode::InvalidArgument,
                            "EndPass without BeginPass",
                        ));
                    }
                    in_pass = false;
                    active_pass = None;
                    graphics_pipeline = None;
                    active_pipeline_layout = None;
                    index_buffer = None;
                }
            }
        }
        if in_pass {
            return self.validation_error(GalError::command(
                StatusCode::InvalidArgument,
                "command list ended inside a pass",
            ));
        }
        Ok(())
    }

    fn validate_pipeline_pass_compatibility(
        &mut self,
        pipeline: Handle,
        pass: Handle,
    ) -> GalResult<()> {
        let compatible = {
            let pipeline_record = self.graphics_pipelines.get(pipeline)?;
            let pass_record = self.render_passes.get(pass)?;
            pipeline_record.desc.color_formats == pass_record.desc.color_formats
                && pipeline_record.desc.depth_format == pass_record.desc.depth_format
        };
        if !compatible {
            if std::env::var_os("MATTMC_TRACE_PIPELINE_PASS_COMPAT").is_some() {
                let pipeline_record = self.graphics_pipelines.get(pipeline)?;
                let pass_record = self.render_passes.get(pass)?;
                eprintln!(
                    "vulkan.pipeline-pass-mismatch pipeline={:?} label={} colors={:?} depth={:?} pass={:?} label={} colors={:?} depth={:?}",
                    pipeline,
                    pipeline_record.desc.label,
                    pipeline_record.desc.color_formats,
                    pipeline_record.desc.depth_format,
                    pass,
                    pass_record.desc.label,
                    pass_record.desc.color_formats,
                    pass_record.desc.depth_format,
                );
            }
            return self.validation_error(GalError::command(
                StatusCode::InvalidArgument,
                "graphics pipeline attachment formats are not compatible with render pass",
            ));
        }
        Ok(())
    }

    fn validate_any_resource(&self, handle: Handle) -> GalResult<()> {
        match handle.kind() {
            Some(HandleKind::Buffer) => self.buffers.get(handle).map(|_| ()),
            Some(HandleKind::Texture) => self.textures.get(handle).map(|_| ()),
            Some(HandleKind::TextureView) => self.texture_views.get(handle).map(|_| ()),
            Some(HandleKind::Sampler) => self.samplers.get(handle).map(|_| ()),
            Some(HandleKind::CombinedTextureSampler) => {
                self.combined_texture_samplers.get(handle).map(|_| ())
            }
            Some(HandleKind::ShaderModule) => self.shaders.get(handle).map(|_| ()),
            Some(HandleKind::ResourceLayout) => self.resource_layouts.get(handle).map(|_| ()),
            Some(HandleKind::ResourceSet) => self.resource_sets.get(handle).map(|_| ()),
            Some(HandleKind::PipelineLayout) => self.pipeline_layouts.get(handle).map(|_| ()),
            Some(HandleKind::GraphicsPipeline) => self.graphics_pipelines.get(handle).map(|_| ()),
            Some(HandleKind::ComputePipeline) => self.compute_pipelines.get(handle).map(|_| ()),
            Some(HandleKind::RenderTarget) => self.render_targets.get(handle).map(|_| ()),
            Some(HandleKind::FrameTarget) => self.frame_targets.get(handle).map(|_| ()),
            Some(HandleKind::RenderPass) => self.render_passes.get(handle).map(|_| ()),
            None => Err(GalError::handle(
                StatusCode::WrongHandleType,
                "unknown handle kind",
            )),
        }
    }

    fn render_pass_desc(&self, pass: Handle) -> GalResult<&RenderPassDesc> {
        self.render_passes.get(pass).map(|record| &record.desc)
    }

    fn validate_submission_hazards(
        &mut self,
        batch: &SubmissionBatch,
        mut profile: Option<&mut WholeFrameProfile>,
    ) -> GalResult<()> {
        let mut accesses = AccessTracker::default();
        for list in &batch.command_lists {
            for op in &list.operations {
                match op {
                    CommandOp::BeginPass {
                        colors,
                        depth_stencil,
                        ..
                    } => {
                        let mut pass_attachment_targets = Vec::new();
                        for color in colors {
                            let target =
                                match color.view.kind() {
                                    Some(HandleKind::FrameTarget) => {
                                        self.frame_targets.get(color.view)?;
                                        AccessTarget::FrameTarget { handle: color.view }
                                    }
                                    Some(HandleKind::TextureView) => {
                                        self.texture_view_access_target(color.view)?
                                    }
                                    _ => return self.validation_error(GalError::submission(
                                        StatusCode::WrongHandleType,
                                        "color attachment must be a texture view or frame target",
                                    )),
                                };
                            if pass_attachment_targets
                                .iter()
                                .any(|previous| targets_overlap(*previous, target))
                            {
                                return self.validation_error(GalError::submission(
                                    StatusCode::InvalidArgument,
                                    "overlapping attachments in the same pass are invalid",
                                ));
                            }
                            pass_attachment_targets.push(target);
                            let event = AccessEvent {
                                target,
                                mode: AccessMode::Write,
                                family: AccessFamily::Attachment,
                                attachment_load_op: Some(color.load_op),
                                attachment_store_op: Some(color.store_op),
                            };
                            self.record_access(&mut accesses, event, profile.as_deref_mut())?;
                        }
                        if let Some(depth) = depth_stencil {
                            let target = self.texture_view_access_target(depth.view)?;
                            if pass_attachment_targets
                                .iter()
                                .any(|previous| targets_overlap(*previous, target))
                            {
                                return self.validation_error(GalError::submission(
                                    StatusCode::InvalidArgument,
                                    "overlapping attachments in the same pass are invalid",
                                ));
                            }
                            let event = AccessEvent {
                                target,
                                mode: AccessMode::Write,
                                family: AccessFamily::Attachment,
                                attachment_load_op: Some(depth.load_op),
                                attachment_store_op: Some(depth.store_op),
                            };
                            self.record_access(&mut accesses, event, profile.as_deref_mut())?;
                        }
                    }
                    CommandOp::BindResourceSet { set, dynamic_offsets, .. } => {
                        let binding_count = self.resource_sets.get(*set)?.desc.bindings.len();
                        let mut offset_index = 0;
                        for index in 0..binding_count {
                            let count = self.resource_sets.get(*set)?.desc.bindings[index].dynamic_offsets.len();
                            // Keep normal binding validation allocation-free;
                            // each declared range contributes one access event.
                            for slot in 0..count.max(1) {
                                let event = {
                                    let binding = &self.resource_sets.get(*set)?.desc.bindings[index];
                                    let offset = if count == 0 { 0 }
                                        else if dynamic_offsets.is_empty() { binding.dynamic_offsets[slot] }
                                        else { dynamic_offsets[offset_index + slot] };
                                    self.resource_binding_access(binding, offset)?
                                };
                                self.record_access(&mut accesses, event, profile.as_deref_mut())?;
                            }
                            offset_index += count;
                        }
                    }
                    CommandOp::SetVertexBuffer { buffer, offset, .. } => {
                        let target = self.buffer_access_target(*buffer, *offset, None)?;
                        self.record_access(
                            &mut accesses,
                            AccessEvent {
                                target,
                                mode: AccessMode::Read,
                                family: AccessFamily::Vertex,
                                attachment_load_op: None,
                                attachment_store_op: None,
                            },
                            profile.as_deref_mut(),
                        )?;
                    }
                    CommandOp::SetIndexBuffer { buffer, offset, .. } => {
                        let target = self.buffer_access_target(*buffer, *offset, None)?;
                        self.record_access(
                            &mut accesses,
                            AccessEvent {
                                target,
                                mode: AccessMode::Read,
                                family: AccessFamily::Index,
                                attachment_load_op: None,
                                attachment_store_op: None,
                            },
                            profile.as_deref_mut(),
                        )?;
                    }
                    CommandOp::DrawIndirect {
                        buffer,
                        offset,
                        draw_count: _,
                    }
                    | CommandOp::DispatchIndirect { buffer, offset } => {
                        let target = self.buffer_access_target(*buffer, *offset, None)?;
                        self.record_access(
                            &mut accesses,
                            AccessEvent {
                                target,
                                mode: AccessMode::Read,
                                family: AccessFamily::Indirect,
                                attachment_load_op: None,
                                attachment_store_op: None,
                            },
                            profile.as_deref_mut(),
                        )?;
                    }
                    CommandOp::CopyBuffer { src, dst, size } => {
                        let src_target = self.buffer_access_target(*src, 0, Some(*size))?;
                        let dst_target = self.buffer_access_target(*dst, 0, Some(*size))?;
                        self.record_access(
                            &mut accesses,
                            AccessEvent {
                                target: src_target,
                                mode: AccessMode::Read,
                                family: AccessFamily::Transfer,
                                attachment_load_op: None,
                                attachment_store_op: None,
                            },
                            profile.as_deref_mut(),
                        )?;
                        self.record_access(
                            &mut accesses,
                            AccessEvent {
                                target: dst_target,
                                mode: AccessMode::Write,
                                family: AccessFamily::Transfer,
                                attachment_load_op: None,
                                attachment_store_op: None,
                            },
                            profile.as_deref_mut(),
                        )?;
                    }
                    CommandOp::CopyBufferToTexture(region) => {
                        let buffer_target = self.buffer_access_target(
                            region.buffer,
                            region.buffer_offset,
                            Some(self.buffer_texture_copy_size(region)?),
                        )?;
                        self.record_access(
                            &mut accesses,
                            AccessEvent {
                                target: buffer_target,
                                mode: AccessMode::Read,
                                family: AccessFamily::Transfer,
                                attachment_load_op: None,
                                attachment_store_op: None,
                            },
                            profile.as_deref_mut(),
                        )?;
                        self.record_access(
                            &mut accesses,
                            AccessEvent {
                                target: self.texture_copy_target(region)?,
                                mode: AccessMode::Write,
                                family: AccessFamily::Transfer,
                                attachment_load_op: None,
                                attachment_store_op: None,
                            },
                            profile.as_deref_mut(),
                        )?;
                    }
                    CommandOp::CopyTextureToBuffer(region) => {
                        self.record_access(
                            &mut accesses,
                            AccessEvent {
                                target: self.texture_copy_target(region)?,
                                mode: AccessMode::Read,
                                family: AccessFamily::Transfer,
                                attachment_load_op: None,
                                attachment_store_op: None,
                            },
                            profile.as_deref_mut(),
                        )?;
                        let buffer_target = self.buffer_access_target(
                            region.buffer,
                            region.buffer_offset,
                            Some(self.buffer_texture_copy_size(region)?),
                        )?;
                        self.record_access(
                            &mut accesses,
                            AccessEvent {
                                target: buffer_target,
                                mode: AccessMode::Write,
                                family: AccessFamily::Transfer,
                                attachment_load_op: None,
                                attachment_store_op: None,
                            },
                            profile.as_deref_mut(),
                        )?;
                    }
                    CommandOp::CopyTexture(region) => {
                        self.record_access(
                            &mut accesses,
                            AccessEvent {
                                target: self.texture_image_copy_target(
                                    region.src_texture,
                                    region.src_mip,
                                    region.src_layer,
                                )?,
                                mode: AccessMode::Read,
                                family: AccessFamily::Transfer,
                                attachment_load_op: None,
                                attachment_store_op: None,
                            },
                            profile.as_deref_mut(),
                        )?;
                        self.record_access(
                            &mut accesses,
                            AccessEvent {
                                target: self.texture_image_copy_target(
                                    region.dst_texture,
                                    region.dst_mip,
                                    region.dst_layer,
                                )?,
                                mode: AccessMode::Write,
                                family: AccessFamily::Transfer,
                                attachment_load_op: None,
                                attachment_store_op: None,
                            },
                            profile.as_deref_mut(),
                        )?;
                    }
                    CommandOp::CopyFrameTargetToTexture { src, dst, .. } => {
                        // An acquired frame target has no separately addressable
                        // image handle on the explicit GAL surface. The copy
                        // operation itself is therefore the ownership/state
                        // transition from the preceding stored attachment to a
                        // transfer read; requiring a synthetic barrier on the
                        // opaque FrameTarget handle would violate that boundary.
                        accesses.retain_non_overlapping(AccessTarget::FrameTarget { handle: *src });
                        self.record_access(
                            &mut accesses,
                            AccessEvent {
                                target: AccessTarget::FrameTarget { handle: *src },
                                mode: AccessMode::Read,
                                family: AccessFamily::Transfer,
                                attachment_load_op: None,
                                attachment_store_op: None,
                            },
                            profile.as_deref_mut(),
                        )?;
                        // The explicit copy has completed the transfer read;
                        // the acquired target may be attached again later in
                        // this same atomic submission without a backend handle
                        // transition being exposed to the frontend.
                        accesses.retain_non_overlapping(AccessTarget::FrameTarget { handle: *src });
                        self.record_access(
                            &mut accesses,
                            AccessEvent {
                                target: self.texture_image_copy_target(*dst, 0, 0)?,
                                mode: AccessMode::Write,
                                family: AccessFamily::Transfer,
                                attachment_load_op: None,
                                attachment_store_op: None,
                            },
                            profile.as_deref_mut(),
                        )?;
                    }
                    CommandOp::CopyTextureToFrameTarget { src, dst, .. } => {
                        accesses.retain_non_overlapping(AccessTarget::FrameTarget { handle: *dst });
                        self.record_access(
                            &mut accesses,
                            AccessEvent {
                                target: self.texture_image_copy_target(*src, 0, 0)?,
                                mode: AccessMode::Read,
                                family: AccessFamily::Transfer,
                                attachment_load_op: None,
                                attachment_store_op: None,
                            },
                            profile.as_deref_mut(),
                        )?;
                        self.record_access(
                            &mut accesses,
                            AccessEvent {
                                target: AccessTarget::FrameTarget { handle: *dst },
                                mode: AccessMode::Write,
                                family: AccessFamily::Transfer,
                                attachment_load_op: None,
                                attachment_store_op: None,
                            },
                            profile.as_deref_mut(),
                        )?;
                        // A texture-to-frame copy is a complete explicit
                        // presentation write. Later GUI/world overlay passes
                        // may legally attach that same acquired target in the
                        // combined submission, with Vulkan lowering providing
                        // the TransferDst -> ColorAttachment transition.
                        // Keep this symmetric with CopyFrameTargetToTexture:
                        // no opaque frame-image state leaks into callers, but
                        // the completed transfer cannot look like an
                        // overlapping write hazard.
                        accesses.retain_non_overlapping(AccessTarget::FrameTarget { handle: *dst });
                    }
                    CommandOp::GenerateMipmaps {
                        texture,
                        subresources,
                    } => {
                        self.record_access(
                            &mut accesses,
                            AccessEvent {
                                target: AccessTarget::Texture {
                                    texture: *texture,
                                    range: *subresources,
                                },
                                mode: AccessMode::Write,
                                family: AccessFamily::Transfer,
                                attachment_load_op: None,
                                attachment_store_op: None,
                            },
                            profile.as_deref_mut(),
                        )?;
                    }
                    CommandOp::HostWriteBuffer {
                        buffer,
                        offset,
                        data,
                    } => {
                        let target =
                            self.buffer_access_target(*buffer, *offset, Some(data.len() as u64))?;
                        self.record_access(
                            &mut accesses,
                            AccessEvent {
                                target,
                                mode: AccessMode::Write,
                                family: AccessFamily::Host,
                                attachment_load_op: None,
                                attachment_store_op: None,
                            },
                            profile.as_deref_mut(),
                        )?;
                    }
                    CommandOp::HostReadBuffer {
                        buffer,
                        offset,
                        size,
                    } => {
                        let target = self.buffer_access_target(*buffer, *offset, Some(*size))?;
                        self.record_access(
                            &mut accesses,
                            AccessEvent {
                                target,
                                mode: AccessMode::Read,
                                family: AccessFamily::Host,
                                attachment_load_op: None,
                                attachment_store_op: None,
                            },
                            profile.as_deref_mut(),
                        )?;
                    }
                    CommandOp::Present {
                        texture,
                        subresources,
                    } => {
                        self.record_access(
                            &mut accesses,
                            AccessEvent {
                                target: AccessTarget::Texture {
                                    texture: *texture,
                                    range: *subresources,
                                },
                                mode: AccessMode::Read,
                                family: AccessFamily::Present,
                                attachment_load_op: None,
                                attachment_store_op: None,
                            },
                            profile.as_deref_mut(),
                        )?;
                    }
                    CommandOp::Barrier(barrier) => {
                        let barrier_target = self.barrier_target(barrier)?;
                        accesses.retain_non_overlapping(barrier_target);
                        if let Some(profile) = profile.as_deref_mut() {
                            profile.gal_hazard_barriers_applied =
                                profile.gal_hazard_barriers_applied.saturating_add(1);
                        }
                    }
                    CommandOp::BindGraphicsPipeline(_)
                    | CommandOp::BindComputePipeline(_)
                    | CommandOp::Draw { .. }
                    | CommandOp::DrawIndexed { .. }
                    | CommandOp::Dispatch { .. }
                    | CommandOp::EndPass => {}
                }
            }
        }
        if let Some(profile) = profile.as_deref_mut() {
            profile.gal_hazard_active_read_entries = profile
                .gal_hazard_active_read_entries
                .saturating_add(accesses.active_read_entries() as u64);
            profile.gal_hazard_active_write_entries = profile
                .gal_hazard_active_write_entries
                .saturating_add(accesses.active_write_entries() as u64);
        }
        Ok(())
    }

    fn resource_binding_access(&self, binding: &ResourceBinding, offset: u64) -> GalResult<AccessEvent> {
        let mode = if binding.access.writes() {
            AccessMode::Write
        } else {
            AccessMode::Read
        };
        let buffer_size = || -> GalResult<u64> {
            let size = self.buffers.get(binding.resource)?.desc.size;
            Ok(binding.buffer_range.unwrap_or_else(|| {
                let max_default_offset = binding.dynamic_offsets.iter().copied().max().unwrap_or(0);
                size.saturating_sub(max_default_offset)
            }))
        };
        match binding.kind {
            ResourceBindingKind::UniformBuffer => Ok(AccessEvent {
                target: self.buffer_access_target(binding.resource, offset, Some(buffer_size()?))?,
                mode: AccessMode::Read,
                family: AccessFamily::Uniform,
                attachment_load_op: None,
                attachment_store_op: None,
            }),
            ResourceBindingKind::StorageBuffer => Ok(AccessEvent {
                target: self.buffer_access_target(binding.resource, offset, Some(buffer_size()?))?,
                mode,
                family: AccessFamily::Storage,
                attachment_load_op: None,
                attachment_store_op: None,
            }),
            ResourceBindingKind::SampledTexture => Ok(AccessEvent {
                target: self.texture_view_access_target(binding.resource)?,
                mode: AccessMode::Read,
                family: AccessFamily::Sampled,
                attachment_load_op: None,
                attachment_store_op: None,
            }),
            ResourceBindingKind::CombinedTextureSampler => {
                let pair = self.combined_texture_samplers.get(binding.resource)?;
                Ok(AccessEvent {
                    target: self.texture_view_access_target(pair.desc.texture_view)?,
                    mode: AccessMode::Read,
                    family: AccessFamily::Sampled,
                    attachment_load_op: None,
                    attachment_store_op: None,
                })
            }
            ResourceBindingKind::StorageTexture => Ok(AccessEvent {
                target: self.texture_view_access_target(binding.resource)?,
                mode,
                family: AccessFamily::Storage,
                attachment_load_op: None,
                attachment_store_op: None,
            }),
            ResourceBindingKind::Sampler => Ok(AccessEvent {
                target: AccessTarget::Buffer {
                    handle: binding.resource,
                    offset: 0,
                    size: 0,
                },
                mode: AccessMode::Read,
                family: AccessFamily::Sampled,
                attachment_load_op: None,
                attachment_store_op: None,
            }),
        }
    }

    fn record_access(
        &mut self,
        accesses: &mut AccessTracker,
        event: AccessEvent,
        mut profile: Option<&mut WholeFrameProfile>,
    ) -> GalResult<()> {
        if event.target.is_zero_sized_sampler_marker() {
            return Ok(());
        }
        if let Some(profile) = profile.as_deref_mut() {
            match event.mode {
                AccessMode::Read => {
                    profile.gal_hazard_read_events =
                        profile.gal_hazard_read_events.saturating_add(1);
                }
                AccessMode::Write => {
                    profile.gal_hazard_write_events =
                        profile.gal_hazard_write_events.saturating_add(1);
                }
            }
        }
        match event.mode {
            AccessMode::Read => {
                if let Some(bucket) = accesses.bucket(event.target) {
                    for previous in bucket.writes.iter().copied() {
                        if let Some(profile) = profile.as_deref_mut() {
                            profile.gal_hazard_candidates_examined =
                                profile.gal_hazard_candidates_examined.saturating_add(1);
                        }
                        if targets_overlap(previous.target, event.target) {
                            if let Some(profile) = profile.as_deref_mut() {
                                profile.gal_hazard_conflicts =
                                    profile.gal_hazard_conflicts.saturating_add(1);
                            }
                            return self.validation_error(GalError::submission(
                                StatusCode::InvalidArgument,
                                format!(
                                    "overlapping {:?} {:?} access on {:?} conflicts with prior {:?} {:?} access",
                                    event.family, event.mode, event.target, previous.family, previous.mode
                                ),
                            ));
                        }
                    }
                }
                accesses.push_read(event);
            }
            AccessMode::Write => {
                if let Some(bucket) = accesses.bucket(event.target) {
                    for previous in bucket.writes.iter().copied() {
                        if let Some(profile) = profile.as_deref_mut() {
                            profile.gal_hazard_candidates_examined =
                                profile.gal_hazard_candidates_examined.saturating_add(1);
                        }
                        if targets_overlap(previous.target, event.target) {
                            if previous.family == AccessFamily::Attachment
                                && event.family == AccessFamily::Attachment
                            {
                                if event.attachment_load_op == Some(AttachmentLoadOp::Load)
                                    && previous.attachment_store_op
                                        != Some(AttachmentStoreOp::Store)
                                {
                                    if let Some(profile) = profile.as_deref_mut() {
                                        profile.gal_hazard_conflicts =
                                            profile.gal_hazard_conflicts.saturating_add(1);
                                    }
                                    return self.validation_error(GalError::submission(
                                        StatusCode::InvalidArgument,
                                        "attachment load depends on a prior pass that did not store",
                                    ));
                                }
                                continue;
                            }
                            if let Some(profile) = profile.as_deref_mut() {
                                profile.gal_hazard_conflicts =
                                    profile.gal_hazard_conflicts.saturating_add(1);
                            }
                            return self.validation_error(GalError::submission(
                                StatusCode::InvalidArgument,
                                format!(
                                "overlapping {:?} {:?} access on {:?} conflicts with prior {:?} {:?} access",
                                    event.family, event.mode, event.target, previous.family, previous.mode
                                ),
                            ));
                        }
                    }
                    for previous in bucket.reads.iter().copied() {
                        if let Some(profile) = profile.as_deref_mut() {
                            profile.gal_hazard_candidates_examined =
                                profile.gal_hazard_candidates_examined.saturating_add(1);
                        }
                        if targets_overlap(previous.target, event.target) {
                            if let Some(profile) = profile.as_deref_mut() {
                                profile.gal_hazard_conflicts =
                                    profile.gal_hazard_conflicts.saturating_add(1);
                            }
                            return self.validation_error(GalError::submission(
                                StatusCode::InvalidArgument,
                                format!(
                                    "overlapping {:?} {:?} access on {:?} conflicts with prior {:?} {:?} access",
                                    event.family, event.mode, event.target, previous.family, previous.mode
                                ),
                            ));
                        }
                    }
                }
                accesses.push_write(event);
            }
        }
        Ok(())
    }

    fn buffer_access_target(
        &self,
        buffer: Handle,
        offset: u64,
        size: Option<u64>,
    ) -> GalResult<AccessTarget> {
        let record = self.buffers.get(buffer)?;
        let size = size.unwrap_or_else(|| record.desc.size.saturating_sub(offset));
        Ok(AccessTarget::Buffer {
            handle: buffer,
            offset,
            size,
        })
    }

    fn barrier_target(&self, barrier: &ResourceBarrier) -> GalResult<AccessTarget> {
        match barrier.resource.kind() {
            Some(HandleKind::Buffer) => self.buffer_access_target(barrier.resource, 0, None),
            Some(HandleKind::Texture) => {
                let record = self.textures.get(barrier.resource)?;
                Ok(AccessTarget::Texture {
                    texture: barrier.resource,
                    range: barrier.subresources.unwrap_or(TextureSubresourceRange {
                        base_mip: 0,
                        mip_count: record.desc.mip_levels,
                        base_layer: 0,
                        layer_count: record.desc.array_layers,
                    }),
                })
            }
            Some(HandleKind::TextureView) => {
                let AccessTarget::Texture { texture, range } =
                    self.texture_view_access_target(barrier.resource)?
                else {
                    unreachable!("texture view access target is always a texture")
                };
                Ok(AccessTarget::Texture {
                    texture,
                    range: barrier.subresources.unwrap_or(range),
                })
            }
            _ => Err(GalError::command(
                StatusCode::InvalidArgument,
                "barrier resource must be a buffer, texture, or texture view",
            )),
        }
    }

    pub(super) fn texture_view_info(&self, view: Handle) -> GalResult<TextureViewInfo> {
        let view_record = self.texture_views.get(view)?;
        let texture_record = self.textures.get(view_record.desc.texture)?;
        Ok(TextureViewInfo {
            texture: view_record.desc.texture,
            format: view_record.desc.format,
            extent: texture_record.desc.extent,
            range: TextureSubresourceRange {
                base_mip: view_record.desc.base_mip,
                mip_count: view_record.desc.mip_count,
                base_layer: view_record.desc.base_layer,
                layer_count: view_record.desc.layer_count,
            },
            usages: texture_record.desc.usages.clone(),
        })
    }

    fn texture_view_access_target(&self, view: Handle) -> GalResult<AccessTarget> {
        let view_record = self.texture_views.get(view)?;
        Ok(AccessTarget::Texture {
            texture: view_record.desc.texture,
            range: TextureSubresourceRange {
                base_mip: view_record.desc.base_mip,
                mip_count: view_record.desc.mip_count,
                base_layer: view_record.desc.base_layer,
                layer_count: view_record.desc.layer_count,
            },
        })
    }

    fn validate_buffer_range(
        &mut self,
        buffer: Handle,
        offset: u64,
        size: u64,
        usage: BufferUsage,
    ) -> GalResult<()> {
        let record = self.buffers.get(buffer)?;
        let has_usage = record.desc.usages.contains(&usage);
        let buffer_size = record.desc.size;
        if size == 0 {
            return self.validation_error(GalError::command(
                StatusCode::InvalidArgument,
                "buffer range size must be non-zero",
            ));
        }
        let Some(end) = offset.checked_add(size) else {
            return self.validation_error(GalError::command(
                StatusCode::InvalidArgument,
                "buffer range overflows",
            ));
        };
        if end > buffer_size {
            return self.validation_error(GalError::command(
                StatusCode::InvalidArgument,
                "buffer range is outside the buffer",
            ));
        }
        if !has_usage {
            return self.validation_error(GalError::command(
                StatusCode::InvalidArgument,
                format!("buffer range requires {usage:?} usage"),
            ));
        }
        Ok(())
    }

    fn validate_buffer_texture_copy_region(
        &mut self,
        region: &BufferImageCopyRegion,
        buffer_usage: BufferUsage,
        texture_usage: TextureUsage,
    ) -> GalResult<()> {
        let size = self.buffer_texture_copy_size(region)?;
        self.validate_buffer_range(region.buffer, region.buffer_offset, size, buffer_usage)?;
        let (texture_dimension, texture_format, texture_usages, texture_extent) = {
            let texture = self.textures.get(region.texture)?;
            (
                texture.desc.dimension,
                texture.desc.format,
                texture.desc.usages.clone(),
                texture.desc.extent,
            )
        };
        if !texture_usages.contains(&texture_usage) {
            return self.validation_error(GalError::command(
                StatusCode::InvalidArgument,
                format!("texture copy requires {texture_usage:?} usage"),
            ));
        }
        if region.extent.width == 0 || region.extent.height == 0 || region.extent.depth == 0 {
            return self.validation_error(GalError::command(
                StatusCode::InvalidArgument,
                "texture copy extent must be non-zero",
            ));
        }
        let Some(bytes_per_texel) = texture_format.copy_bytes_per_texel() else {
            return self.unsupported(format!(
                "texture format {texture_format:?} does not support buffer texture copies"
            ));
        };
        let Some(minimum_row_bytes) = region.extent.width.checked_mul(bytes_per_texel) else {
            return self.validation_error(GalError::command(
                StatusCode::InvalidArgument,
                "texture copy row byte count overflows",
            ));
        };
        if region.buffer_offset % 4 != 0
            || region.bytes_per_row < minimum_row_bytes
            || region.rows_per_image < region.extent.height
            || region.bytes_per_row % bytes_per_texel != 0
        {
            return self.validation_error(GalError::command(
                StatusCode::InvalidArgument,
                "texture copy row layout is malformed",
            ));
        }
        self.validate_texture_subresource_range(
            region.texture,
            TextureSubresourceRange {
                base_mip: region.texture_mip,
                mip_count: 1,
                base_layer: region.texture_layer,
                layer_count: 1,
            },
        )?;
        if texture_dimension == TextureDimension::D3 && region.texture_layer != 0 {
            return self.validation_error(GalError::command(
                StatusCode::InvalidArgument,
                "D3 texture copies must use texture layer 0",
            ));
        }
        if texture_dimension == TextureDimension::D2
            && (region.texture_origin.z != 0 || region.extent.depth != 1)
        {
            return self.validation_error(GalError::command(
                StatusCode::InvalidArgument,
                "D2 texture copies must address one depth slice at z 0",
            ));
        }
        let mip_extent = texture_mip_extent(texture_extent, region.texture_mip);
        let Some(x_end) = region.texture_origin.x.checked_add(region.extent.width) else {
            return self.validation_error(GalError::command(
                StatusCode::InvalidArgument,
                "texture copy x range overflows",
            ));
        };
        let Some(y_end) = region.texture_origin.y.checked_add(region.extent.height) else {
            return self.validation_error(GalError::command(
                StatusCode::InvalidArgument,
                "texture copy y range overflows",
            ));
        };
        let Some(z_end) = region.texture_origin.z.checked_add(region.extent.depth) else {
            return self.validation_error(GalError::command(
                StatusCode::InvalidArgument,
                "texture copy z range overflows",
            ));
        };
        if x_end > mip_extent.width || y_end > mip_extent.height || z_end > mip_extent.depth {
            return self.validation_error(GalError::command(
                StatusCode::InvalidArgument,
                "texture copy region is outside texture extent",
            ));
        }
        Ok(())
    }

    fn buffer_texture_copy_size(&self, region: &BufferImageCopyRegion) -> GalResult<u64> {
        if region.extent.width == 0 || region.extent.height == 0 || region.extent.depth == 0 {
            return Err(GalError::command(
                StatusCode::InvalidArgument,
                "texture copy extent must be non-zero",
            ));
        }
        let bytes_per_row = u64::from(region.bytes_per_row);
        let rows_per_image = u64::from(region.rows_per_image);
        let layers = u64::from(region.extent.depth);
        rows_per_image
            .checked_mul(layers.saturating_sub(1))
            .and_then(|rows_before_last| {
                rows_before_last.checked_add(u64::from(region.extent.height))
            })
            .and_then(|rows| rows.checked_mul(bytes_per_row))
            .ok_or_else(|| {
                GalError::command(
                    StatusCode::InvalidArgument,
                    "texture copy buffer size overflows",
                )
            })
    }

    fn validate_texture_copy_region(&mut self, region: &TextureImageCopyRegion) -> GalResult<()> {
        if region.row_order == super::commands::TextureRowOrder::Reverse
            && !self.capabilities().supports(BackendFeature::TextureRowReversal)
        {
            return self.validation_error(GalError::unsupported_feature(
                "backend does not support explicit texture row reversal",
            ));
        }
        if region.src_texture == region.dst_texture
            || region.extent.width == 0
            || region.extent.height == 0
            || region.extent.depth == 0
        {
            return self.validation_error(GalError::command(
                StatusCode::InvalidArgument,
                "texture copy requires distinct textures and a non-zero extent",
            ));
        }
        let (src_dimension, src_format, src_usages, src_extent) = {
            let record = self.textures.get(region.src_texture)?;
            (
                record.desc.dimension,
                record.desc.format,
                record.desc.usages.clone(),
                record.desc.extent,
            )
        };
        let (dst_dimension, dst_format, dst_usages, dst_extent) = {
            let record = self.textures.get(region.dst_texture)?;
            (
                record.desc.dimension,
                record.desc.format,
                record.desc.usages.clone(),
                record.desc.extent,
            )
        };
        if !src_usages.contains(&TextureUsage::TransferSrc)
            || !dst_usages.contains(&TextureUsage::TransferDst)
        {
            return self.validation_error(GalError::command(
                StatusCode::InvalidArgument,
                "texture copy requires transfer source and destination usages",
            ));
        }
        if src_dimension != dst_dimension || src_format != dst_format {
            return self.validation_error(GalError::command(
                StatusCode::InvalidArgument,
                "texture copy requires matching dimensions and formats",
            ));
        }
        if region.row_order == super::commands::TextureRowOrder::Reverse
            && src_dimension != TextureDimension::D2
        {
            return self.validation_error(GalError::unsupported_feature(
                "texture row reversal currently requires D2 textures",
            ));
        }
        self.validate_texture_copy_box(
            region.src_texture,
            region.src_mip,
            region.src_layer,
            region.src_origin,
            region.extent,
            src_dimension,
            src_extent,
        )?;
        self.validate_texture_copy_box(
            region.dst_texture,
            region.dst_mip,
            region.dst_layer,
            region.dst_origin,
            region.extent,
            dst_dimension,
            dst_extent,
        )
    }

    fn validate_texture_copy_box(
        &mut self,
        texture: Handle,
        mip: u32,
        layer: u32,
        origin: TextureOrigin3d,
        extent: Extent3d,
        dimension: TextureDimension,
        base_extent: Extent3d,
    ) -> GalResult<()> {
        self.validate_texture_subresource_range(
            texture,
            TextureSubresourceRange {
                base_mip: mip,
                mip_count: 1,
                base_layer: layer,
                layer_count: 1,
            },
        )?;
        if dimension == TextureDimension::D3 && layer != 0 {
            return self.validation_error(GalError::command(
                StatusCode::InvalidArgument,
                "D3 texture copies must use layer 0",
            ));
        }
        if dimension == TextureDimension::D2 && (origin.z != 0 || extent.depth != 1) {
            return self.validation_error(GalError::command(
                StatusCode::InvalidArgument,
                "D2 texture copies must address one depth slice at z 0",
            ));
        }
        let mip_extent = texture_mip_extent(base_extent, mip);
        let valid = origin
            .x
            .checked_add(extent.width)
            .is_some_and(|end| end <= mip_extent.width)
            && origin
                .y
                .checked_add(extent.height)
                .is_some_and(|end| end <= mip_extent.height)
            && origin
                .z
                .checked_add(extent.depth)
                .is_some_and(|end| end <= mip_extent.depth);
        if !valid {
            return self.validation_error(GalError::command(
                StatusCode::InvalidArgument,
                "texture copy region is outside texture extent",
            ));
        }
        Ok(())
    }

    fn validate_frame_target_copy(
        &mut self,
        src: Handle,
        dst: Handle,
        extent: Extent3d,
    ) -> GalResult<()> {
        if src == dst || extent.width == 0 || extent.height == 0 || extent.depth == 0 {
            return self.validation_error(GalError::command(
                StatusCode::InvalidArgument,
                "frame-target copy requires distinct handles and a non-zero extent",
            ));
        }
        let source = self.frame_targets.get(src)?.desc.clone();
        let destination = self.textures.get(dst)?.desc.clone();
        if !destination.usages.contains(&TextureUsage::TransferDst) {
            return self.validation_error(GalError::command(
                StatusCode::InvalidArgument,
                "frame-target copy destination requires transfer-destination usage",
            ));
        }
        if destination.format != source.color_format
            || destination.dimension != TextureDimension::D2
            || extent.width > source.extent.width
            || extent.height > source.extent.height
            || extent.depth != 1
            || extent.width > destination.extent.width
            || extent.height > destination.extent.height
        {
            return self.validation_error(GalError::command(
                StatusCode::InvalidArgument,
                "frame-target copy requires matching 2D format and bounded extent",
            ));
        }
        Ok(())
    }

    fn validate_texture_to_frame_target_copy(
        &mut self,
        src: Handle,
        dst: Handle,
        extent: Extent3d,
    ) -> GalResult<()> {
        if src == dst || extent.width == 0 || extent.height == 0 || extent.depth == 0 {
            return self.validation_error(GalError::command(
                StatusCode::InvalidArgument,
                "texture-to-frame-target copy requires distinct handles and a non-zero extent",
            ));
        }
        let source = self.textures.get(src)?.desc.clone();
        let destination = self.frame_targets.get(dst)?.desc.clone();
        if !source.usages.contains(&TextureUsage::TransferSrc) {
            return self.validation_error(GalError::command(
                StatusCode::InvalidArgument,
                "texture-to-frame-target copy source requires transfer-source usage",
            ));
        }
        if source.format != destination.color_format
            || source.dimension != TextureDimension::D2
            || extent.width > source.extent.width
            || extent.height > source.extent.height
            || extent.depth != 1
            || extent.width > destination.extent.width
            || extent.height > destination.extent.height
        {
            return self.validation_error(GalError::command(
                StatusCode::InvalidArgument,
                "texture-to-frame-target copy requires matching 2D format and bounded extent",
            ));
        }
        Ok(())
    }

    fn texture_copy_target(&self, region: &BufferImageCopyRegion) -> GalResult<AccessTarget> {
        self.texture_image_copy_target(region.texture, region.texture_mip, region.texture_layer)
    }

    fn texture_image_copy_target(
        &self,
        texture: Handle,
        mip: u32,
        layer: u32,
    ) -> GalResult<AccessTarget> {
        self.textures.get(texture)?;
        Ok(AccessTarget::Texture {
            texture,
            range: TextureSubresourceRange {
                base_mip: mip,
                mip_count: 1,
                base_layer: layer,
                layer_count: 1,
            },
        })
    }

    fn validate_texture_subresource_range(
        &mut self,
        texture: Handle,
        range: TextureSubresourceRange,
    ) -> GalResult<()> {
        let record = self.textures.get(texture)?;
        let dimension = record.desc.dimension;
        let mip_levels = record.desc.mip_levels;
        let array_layers = record.desc.array_layers;
        let Some(mip_end) = range.base_mip.checked_add(range.mip_count) else {
            return self.validation_error(GalError::command(
                StatusCode::InvalidArgument,
                "texture subresource mip range overflows",
            ));
        };
        let Some(layer_end) = range.base_layer.checked_add(range.layer_count) else {
            return self.validation_error(GalError::command(
                StatusCode::InvalidArgument,
                "texture subresource layer range overflows",
            ));
        };
        if range.mip_count == 0
            || range.layer_count == 0
            || mip_end > mip_levels
            || layer_end > array_layers
        {
            return self.validation_error(GalError::command(
                StatusCode::InvalidArgument,
                "texture subresource range is outside the texture",
            ));
        }
        if dimension == TextureDimension::D3 && (range.base_layer != 0 || range.layer_count != 1) {
            return self.validation_error(GalError::command(
                StatusCode::InvalidArgument,
                "D3 texture subresources have exactly one image layer",
            ));
        }
        Ok(())
    }

    fn mark_in_flight(&mut self, handle: Handle, id: SubmissionId) -> GalResult<()> {
        // A command references resource sets and pipelines, while their
        // descriptor/layout dependency edges own the sampler, image view,
        // texture, and shader handles they contain.  Retire the complete
        // dependency closure with the submission; marking only the directly
        // encoded set allowed a replaced sampler to be destroyed while its
        // descriptor set was still executing on Vulkan.
        let dependencies = self
            .reverse_dependencies
            .get(&handle)
            .cloned()
            .unwrap_or_default();
        for dependency in dependencies {
            self.mark_in_flight(dependency, id)?;
        }
        match handle.kind() {
            Some(HandleKind::Buffer) => {
                self.buffers.get_mut_record(handle)?.last_submission = Some(id)
            }
            Some(HandleKind::Texture) => {
                self.textures.get_mut_record(handle)?.last_submission = Some(id)
            }
            Some(HandleKind::TextureView) => {
                self.texture_views.get_mut_record(handle)?.last_submission = Some(id)
            }
            Some(HandleKind::Sampler) => {
                self.samplers.get_mut_record(handle)?.last_submission = Some(id)
            }
            Some(HandleKind::CombinedTextureSampler) => {
                let pair = self.combined_texture_samplers.get(handle)?.desc.clone();
                self.combined_texture_samplers
                    .get_mut_record(handle)?
                    .last_submission = Some(id);
                self.texture_views
                    .get_mut_record(pair.texture_view)?
                    .last_submission = Some(id);
                self.samplers.get_mut_record(pair.sampler)?.last_submission = Some(id);
            }
            Some(HandleKind::ShaderModule) => {
                self.shaders.get_mut_record(handle)?.last_submission = Some(id)
            }
            Some(HandleKind::ResourceLayout) => {
                self.resource_layouts
                    .get_mut_record(handle)?
                    .last_submission = Some(id)
            }
            Some(HandleKind::ResourceSet) => {
                self.resource_sets.get_mut_record(handle)?.last_submission = Some(id)
            }
            Some(HandleKind::PipelineLayout) => {
                self.pipeline_layouts
                    .get_mut_record(handle)?
                    .last_submission = Some(id)
            }
            Some(HandleKind::GraphicsPipeline) => {
                self.graphics_pipelines
                    .get_mut_record(handle)?
                    .last_submission = Some(id)
            }
            Some(HandleKind::ComputePipeline) => {
                self.compute_pipelines
                    .get_mut_record(handle)?
                    .last_submission = Some(id)
            }
            Some(HandleKind::RenderTarget) => {
                self.render_targets.get_mut_record(handle)?.last_submission = Some(id)
            }
            Some(HandleKind::FrameTarget) => {
                self.frame_targets.get_mut_record(handle)?.last_submission = Some(id)
            }
            Some(HandleKind::RenderPass) => {
                self.render_passes.get_mut_record(handle)?.last_submission = Some(id)
            }
            None => {
                return Err(GalError::handle(
                    StatusCode::WrongHandleType,
                    "unknown handle kind",
                ))
            }
        }
        Ok(())
    }

    fn remove_record(
        &mut self,
        handle: Handle,
        kind: HandleKind,
    ) -> GalResult<(PendingDestroy, Option<SubmissionId>)> {
        macro_rules! remove_from {
            ($arena:expr) => {{
                let record = $arena.remove(handle)?;
                (
                    PendingDestroy {
                        kind,
                        token: record.token,
                    },
                    record.last_submission,
                )
            }};
        }
        Ok(match kind {
            HandleKind::Buffer => remove_from!(self.buffers),
            HandleKind::Texture => remove_from!(self.textures),
            HandleKind::TextureView => remove_from!(self.texture_views),
            HandleKind::Sampler => remove_from!(self.samplers),
            HandleKind::CombinedTextureSampler => remove_from!(self.combined_texture_samplers),
            HandleKind::ShaderModule => remove_from!(self.shaders),
            HandleKind::ResourceLayout => remove_from!(self.resource_layouts),
            HandleKind::ResourceSet => remove_from!(self.resource_sets),
            HandleKind::PipelineLayout => remove_from!(self.pipeline_layouts),
            HandleKind::GraphicsPipeline => remove_from!(self.graphics_pipelines),
            HandleKind::ComputePipeline => remove_from!(self.compute_pipelines),
            HandleKind::RenderTarget => remove_from!(self.render_targets),
            HandleKind::FrameTarget => remove_from!(self.frame_targets),
            HandleKind::RenderPass => remove_from!(self.render_passes),
        })
    }

    #[cfg(test)]
    pub(super) fn force_buffer_generation_for_test(&mut self, handle: Handle, generation: u32) {
        self.buffers.force_generation(handle, generation);
    }

    #[cfg(test)]
    pub(super) fn mock_backend(&self) -> Option<&super::backends::mock::MockBackend> {
        self.backend.as_any().downcast_ref()
    }

    #[cfg(test)]
    pub(super) fn sampler_descriptor_for_test(&self, handle: Handle) -> GalResult<&SamplerDesc> {
        Ok(&self.samplers.get(handle)?.desc)
    }

    #[cfg(test)]
    pub(super) fn resource_set_descriptor_for_test(&self, handle: Handle) -> GalResult<&ResourceSetDesc> {
        Ok(&self.resource_sets.get(handle)?.desc)
    }

    #[cfg(test)]
    pub(super) fn graphics_pipeline_descriptor_for_test(&self, handle: Handle) -> GalResult<&GraphicsPipelineDesc> {
        Ok(&self.graphics_pipelines.get(handle)?.desc)
    }

    #[cfg(test)]
    pub(super) fn mock_backend_mut(&mut self) -> Option<&mut super::backends::mock::MockBackend> {
        self.backend.as_any_mut().downcast_mut()
    }

    #[cfg(test)]
    pub(super) fn vulkan_backend(&self) -> Option<&super::backends::vulkan::VulkanBackend> {
        self.backend.as_any().downcast_ref()
    }

    #[cfg(test)]
    pub(super) fn vulkan_backend_mut(
        &mut self,
    ) -> Option<&mut super::backends::vulkan::VulkanBackend> {
        self.backend.as_any_mut().downcast_mut()
    }

    #[cfg(test)]
    pub(super) fn opengl_backend(&self) -> Option<&super::backends::opengl::OpenGlBackend> {
        self.backend.as_any().downcast_ref()
    }

    #[cfg(test)]
    pub(super) fn retire_through_for_test(&mut self, id: SubmissionId) -> GalResult<Vec<Handle>> {
        self.retire_through(id)
    }
}

trait ArenaRecordExt<T> {
    fn get_mut_record(&mut self, handle: Handle) -> GalResult<&mut ResourceRecord<T>>;
}

impl<T> ArenaRecordExt<T> for Arena<ResourceRecord<T>> {
    fn get_mut_record(&mut self, handle: Handle) -> GalResult<&mut ResourceRecord<T>> {
        let (index, generation) = handle.require_kind(self.kind)?;
        let Some(slot) = self.slots.get_mut(index) else {
            return Err(GalError::handle(
                StatusCode::StaleHandle,
                "handle slot does not exist",
            ));
        };
        if slot.generation != generation {
            return Err(GalError::handle(
                StatusCode::StaleHandle,
                "stale handle generation",
            ));
        }
        slot.value
            .as_mut()
            .ok_or_else(|| GalError::handle(StatusCode::StaleHandle, "resource is not live"))
    }
}

fn referenced_handles(batch: &SubmissionBatch) -> BTreeSet<Handle> {
    let mut handles = BTreeSet::new();
    for list in &batch.command_lists {
        for op in &list.operations {
            match op {
                CommandOp::BeginPass {
                    pass,
                    target,
                    colors,
                    depth_stencil,
                } => {
                    handles.insert(*pass);
                    handles.insert(*target);
                    for color in colors {
                        handles.insert(color.view);
                    }
                    if let Some(depth) = depth_stencil {
                        handles.insert(depth.view);
                    }
                }
                CommandOp::BindGraphicsPipeline(handle)
                | CommandOp::BindComputePipeline(handle)
                | CommandOp::DrawIndirect { buffer: handle, .. }
                | CommandOp::DispatchIndirect { buffer: handle, .. }
                | CommandOp::SetIndexBuffer { buffer: handle, .. }
                | CommandOp::HostWriteBuffer { buffer: handle, .. }
                | CommandOp::HostReadBuffer { buffer: handle, .. }
                | CommandOp::Barrier(super::commands::ResourceBarrier {
                    resource: handle, ..
                }) => {
                    handles.insert(*handle);
                }
                CommandOp::CopyBufferToTexture(region) | CommandOp::CopyTextureToBuffer(region) => {
                    handles.insert(region.buffer);
                    handles.insert(region.texture);
                }
                CommandOp::CopyTexture(region) => {
                    handles.insert(region.src_texture);
                    handles.insert(region.dst_texture);
                }
                CommandOp::CopyFrameTargetToTexture { src, dst, .. } => {
                    handles.insert(*src);
                    handles.insert(*dst);
                }
                CommandOp::CopyTextureToFrameTarget { src, dst, .. } => {
                    handles.insert(*src);
                    handles.insert(*dst);
                }
                CommandOp::GenerateMipmaps { texture, .. } => {
                    handles.insert(*texture);
                }
                CommandOp::Present {
                    texture: handle, ..
                } => {
                    handles.insert(*handle);
                }
                CommandOp::BindResourceSet {
                    pipeline_layout,
                    set,
                    ..
                } => {
                    handles.insert(*pipeline_layout);
                    handles.insert(*set);
                }
                CommandOp::SetVertexBuffer { buffer, .. } => {
                    handles.insert(*buffer);
                }
                CommandOp::CopyBuffer { src, dst, .. } => {
                    handles.insert(*src);
                    handles.insert(*dst);
                }
                CommandOp::Draw { .. }
                | CommandOp::DrawIndexed { .. }
                | CommandOp::Dispatch { .. }
                | CommandOp::EndPass => {}
            }
        }
    }
    handles
}

pub(super) fn normalize_submission_batch(batch: &mut SubmissionBatch) -> CommandNormalizationStats {
    let mut stats = CommandNormalizationStats::default();
    for list in &mut batch.command_lists {
        stats.ops_before = stats
            .ops_before
            .saturating_add(list.operations.len() as u64);
        let original = std::mem::take(&mut list.operations);
        let mut normalized = Vec::with_capacity(original.len());
        let mut state = CommandStateTracker::default();
        for op in original {
            let keep = match &op {
                CommandOp::BeginPass { .. } | CommandOp::EndPass | CommandOp::Barrier(_) => {
                    state.invalidate();
                    true
                }
                CommandOp::BindGraphicsPipeline(handle) => {
                    if state.graphics_pipeline == Some(*handle) {
                        stats.pipeline_binds_removed =
                            stats.pipeline_binds_removed.saturating_add(1);
                        false
                    } else {
                        state.graphics_pipeline = Some(*handle);
                        state.compute_pipeline = None;
                        true
                    }
                }
                CommandOp::BindComputePipeline(handle) => {
                    if state.compute_pipeline == Some(*handle) {
                        stats.pipeline_binds_removed =
                            stats.pipeline_binds_removed.saturating_add(1);
                        false
                    } else {
                        state.compute_pipeline = Some(*handle);
                        state.graphics_pipeline = None;
                        true
                    }
                }
                CommandOp::BindResourceSet {
                    pipeline_layout,
                    set_index,
                    set,
                    dynamic_offsets,
                } => {
                    let key = (*pipeline_layout, *set_index);
                    let value = (*set, dynamic_offsets.clone());
                    if state.resource_sets.get(&key) == Some(&value) {
                        stats.resource_set_binds_removed =
                            stats.resource_set_binds_removed.saturating_add(1);
                        false
                    } else {
                        state.resource_sets.insert(key, value);
                        true
                    }
                }
                CommandOp::SetVertexBuffer {
                    slot,
                    buffer,
                    offset,
                } => {
                    let value = (*buffer, *offset);
                    if state.vertex_buffers.get(slot).copied() == Some(value) {
                        stats.vertex_buffer_binds_removed =
                            stats.vertex_buffer_binds_removed.saturating_add(1);
                        false
                    } else {
                        state.vertex_buffers.insert(*slot, value);
                        true
                    }
                }
                CommandOp::SetIndexBuffer {
                    buffer,
                    offset,
                    index_type,
                } => {
                    let value = (*buffer, *offset, *index_type);
                    if state.index_buffer == Some(value) {
                        stats.index_buffer_binds_removed =
                            stats.index_buffer_binds_removed.saturating_add(1);
                        false
                    } else {
                        state.index_buffer = Some(value);
                        true
                    }
                }
                CommandOp::CopyBuffer { .. }
                | CommandOp::CopyBufferToTexture(_)
                | CommandOp::CopyTextureToBuffer(_)
                | CommandOp::CopyTexture(_)
                | CommandOp::CopyFrameTargetToTexture { .. }
                | CommandOp::CopyTextureToFrameTarget { .. }
                | CommandOp::GenerateMipmaps { .. }
                | CommandOp::HostWriteBuffer { .. }
                | CommandOp::HostReadBuffer { .. }
                | CommandOp::Present { .. } => {
                    state.invalidate();
                    true
                }
                CommandOp::Draw { .. }
                | CommandOp::DrawIndexed { .. }
                | CommandOp::DrawIndirect { .. }
                | CommandOp::Dispatch { .. }
                | CommandOp::DispatchIndirect { .. } => true,
            };
            if keep {
                normalized.push(op);
            }
        }
        stats.ops_after = stats.ops_after.saturating_add(normalized.len() as u64);
        list.operations = normalized;
    }
    stats
}

fn add_command_profile(profile: &mut WholeFrameProfile, batch: &SubmissionBatch) {
    for list in &batch.command_lists {
        for op in &list.operations {
            match op {
                CommandOp::BeginPass { .. } => profile.pass_count += 1,
                CommandOp::BindGraphicsPipeline(_) | CommandOp::BindComputePipeline(_) => {
                    profile.pipeline_binds += 1;
                }
                CommandOp::BindResourceSet { .. } => profile.resource_set_binds += 1,
                CommandOp::Draw { .. } => profile.draw_ops += 1,
                CommandOp::DrawIndexed { .. } => profile.draw_indexed_ops += 1,
                CommandOp::HostWriteBuffer { data, .. } => {
                    profile.host_write_ops += 1;
                    profile.host_write_bytes =
                        profile.host_write_bytes.saturating_add(data.len() as u64);
                }
                CommandOp::Barrier(_) => profile.barrier_ops += 1,
                CommandOp::EndPass
                | CommandOp::SetVertexBuffer { .. }
                | CommandOp::SetIndexBuffer { .. }
                | CommandOp::DrawIndirect { .. }
                | CommandOp::Dispatch { .. }
                | CommandOp::DispatchIndirect { .. }
                | CommandOp::CopyBuffer { .. }
                | CommandOp::CopyBufferToTexture(_)
                | CommandOp::CopyTextureToBuffer(_)
                | CommandOp::CopyTexture(_)
                | CommandOp::CopyFrameTargetToTexture { .. }
                | CommandOp::CopyTextureToFrameTarget { .. }
                | CommandOp::GenerateMipmaps { .. }
                | CommandOp::HostReadBuffer { .. }
                | CommandOp::Present { .. } => {}
            }
        }
    }
}

fn add_backend_metric_deltas(
    profile: &mut WholeFrameProfile,
    before: BackendRuntimeMetrics,
    after: BackendRuntimeMetrics,
) {
    profile.vulkan_command_buffer_alloc_nanos = after
        .vulkan_command_buffer_alloc_nanos
        .saturating_sub(before.vulkan_command_buffer_alloc_nanos);
    profile.vulkan_command_buffer_begin_nanos = after
        .vulkan_command_buffer_begin_nanos
        .saturating_sub(before.vulkan_command_buffer_begin_nanos);
    profile.vulkan_command_recording_nanos = after
        .vulkan_command_recording_nanos
        .saturating_sub(before.vulkan_command_recording_nanos);
    profile.vulkan_command_buffer_end_nanos = after
        .vulkan_command_buffer_end_nanos
        .saturating_sub(before.vulkan_command_buffer_end_nanos);
    profile.vulkan_queue_submit_nanos = after
        .vulkan_queue_submit_nanos
        .saturating_sub(before.vulkan_queue_submit_nanos);
    profile.vulkan_timeline_poll_nanos = after
        .vulkan_timeline_poll_nanos
        .saturating_sub(before.vulkan_timeline_poll_nanos);
    profile.vulkan_timeline_wait_nanos = after
        .vulkan_timeline_wait_nanos
        .saturating_sub(before.vulkan_timeline_wait_nanos);
    profile.vulkan_device_wait_idle_nanos = after
        .vulkan_device_wait_idle_nanos
        .saturating_sub(before.vulkan_device_wait_idle_nanos);
    profile.vulkan_command_buffers_allocated = after
        .vulkan_command_buffers_allocated
        .saturating_sub(before.vulkan_command_buffers_allocated);
    profile.vulkan_command_buffers_freed = after
        .vulkan_command_buffers_freed
        .saturating_sub(before.vulkan_command_buffers_freed);
    profile.vulkan_wait_count = after
        .vulkan_wait_count
        .saturating_sub(before.vulkan_wait_count);
    profile.vulkan_device_wait_idle_count = after
        .vulkan_device_wait_idle_count
        .saturating_sub(before.vulkan_device_wait_idle_count);
    profile.vulkan_acquire_nanos = after
        .vulkan_acquire_nanos
        .saturating_sub(before.vulkan_acquire_nanos);
    profile.vulkan_present_nanos = after
        .vulkan_present_nanos
        .saturating_sub(before.vulkan_present_nanos);
    profile.vulkan_present_wait_nanos = after
        .vulkan_present_wait_nanos
        .saturating_sub(before.vulkan_present_wait_nanos);
    profile.vulkan_present_mode = after.vulkan_present_mode;
    profile.vulkan_requested_present_mode = after.vulkan_requested_present_mode;
    profile.vulkan_supported_present_modes = after.vulkan_supported_present_modes;
    profile.vulkan_present_mode_fallback_reason = after.vulkan_present_mode_fallback_reason;
    profile.vulkan_acquired_image_index = after.vulkan_acquired_image_index;
    profile.vulkan_swapchain_generation = after.vulkan_swapchain_generation;
    profile.vulkan_swapchain_image_count = after.vulkan_swapchain_image_count;
    profile.vulkan_surface_min_image_count = after.vulkan_surface_min_image_count;
    profile.vulkan_surface_max_image_count = after.vulkan_surface_max_image_count;
    profile.vulkan_configured_frames_in_flight = after.vulkan_configured_frames_in_flight;
    profile.vulkan_images_in_flight = after.vulkan_images_in_flight;
    profile.vulkan_available_frame_slots = after.vulkan_available_frame_slots;
    profile.gpu_timestamp_status = after.gpu_timestamp_status;
    profile.gpu_shadow_depth_nanos = after.gpu_shadow_depth_nanos;
    profile.gpu_terrain_opaque_nanos = after.gpu_terrain_opaque_nanos;
    profile.gpu_terrain_cutout_nanos = after.gpu_terrain_cutout_nanos;
    profile.gpu_deferred_lighting_nanos = after.gpu_deferred_lighting_nanos;
    profile.gpu_composite0_nanos = after.gpu_composite0_nanos;
    profile.gpu_composite1_nanos = after.gpu_composite1_nanos;
    profile.gpu_final_output_nanos = after.gpu_final_output_nanos;
    profile.gpu_frame_total_nanos = after.gpu_frame_total_nanos;
}

fn is_depth_stencil_format(format: TextureFormat) -> bool {
    matches!(
        format,
        TextureFormat::Depth24Stencil8 | TextureFormat::Depth32Float
    )
}

impl AccessTarget {
    fn is_zero_sized_sampler_marker(self) -> bool {
        matches!(self, AccessTarget::Buffer { size: 0, .. })
    }

    fn resource_key(self) -> AccessResourceKey {
        match self {
            AccessTarget::Buffer { handle, .. } => AccessResourceKey::Buffer(handle),
            AccessTarget::Texture { texture, .. } => AccessResourceKey::Texture(texture),
            AccessTarget::FrameTarget { handle } => AccessResourceKey::FrameTarget(handle),
        }
    }
}

fn index_type_size(index_type: super::resources::IndexType) -> u64 {
    match index_type {
        super::resources::IndexType::U16 => 2,
        super::resources::IndexType::U32 => 4,
    }
}

fn texture_mip_extent(base: Extent3d, mip: u32) -> Extent3d {
    Extent3d {
        width: (base.width >> mip).max(1),
        height: (base.height >> mip).max(1),
        depth: (base.depth >> mip).max(1),
    }
}

fn max_texture_mip_levels(extent: Extent3d) -> u32 {
    let largest_dimension = extent.width.max(extent.height).max(extent.depth);
    u32::BITS - largest_dimension.leading_zeros()
}

fn is_depth_format(format: TextureFormat) -> bool {
    matches!(
        format,
        TextureFormat::Depth24Stencil8 | TextureFormat::Depth32Float
    )
}

fn targets_overlap(left: AccessTarget, right: AccessTarget) -> bool {
    match (left, right) {
        (
            AccessTarget::Buffer {
                handle: left_handle,
                offset: left_offset,
                size: left_size,
            },
            AccessTarget::Buffer {
                handle: right_handle,
                offset: right_offset,
                size: right_size,
            },
        ) => {
            if left_handle != right_handle || left_size == 0 || right_size == 0 {
                return false;
            }
            ranges_overlap(left_offset, left_size, right_offset, right_size)
        }
        (
            AccessTarget::Texture {
                texture: left_texture,
                range: left_range,
            },
            AccessTarget::Texture {
                texture: right_texture,
                range: right_range,
            },
        ) => left_texture == right_texture && texture_ranges_overlap(left_range, right_range),
        (
            AccessTarget::FrameTarget {
                handle: left_handle,
            },
            AccessTarget::FrameTarget {
                handle: right_handle,
            },
        ) => left_handle == right_handle,
        _ => false,
    }
}

fn ranges_overlap(left_offset: u64, left_size: u64, right_offset: u64, right_size: u64) -> bool {
    let left_end = left_offset.saturating_add(left_size);
    let right_end = right_offset.saturating_add(right_size);
    left_offset < right_end && right_offset < left_end
}

fn texture_ranges_overlap(left: TextureSubresourceRange, right: TextureSubresourceRange) -> bool {
    ranges_overlap(
        left.base_mip as u64,
        left.mip_count as u64,
        right.base_mip as u64,
        right.mip_count as u64,
    ) && ranges_overlap(
        left.base_layer as u64,
        left.layer_count as u64,
        right.base_layer as u64,
        right.layer_count as u64,
    )
}

fn texture_range_contains(outer: TextureSubresourceRange, inner: TextureSubresourceRange) -> bool {
    let outer_mip_end = outer.base_mip.saturating_add(outer.mip_count);
    let inner_mip_end = inner.base_mip.saturating_add(inner.mip_count);
    let outer_layer_end = outer.base_layer.saturating_add(outer.layer_count);
    let inner_layer_end = inner.base_layer.saturating_add(inner.layer_count);
    outer.base_mip <= inner.base_mip
        && inner_mip_end <= outer_mip_end
        && outer.base_layer <= inner.base_layer
        && inner_layer_end <= outer_layer_end
}

#[cfg(test)]
mod arena_tests {
    use super::*;

    #[test]
    fn arena_rejects_growth_at_the_explicit_slot_bound() {
        let mut arena = Arena::<()>::new(HandleKind::Buffer);
        arena.slots.resize_with(MAX_ARENA_SLOTS, || Slot {
            generation: 1,
            last_destroyed_generation: None,
            value: Some(()),
        });
        let handle = Handle::new(HandleKind::Buffer, MAX_ARENA_SLOTS as u32, 1).unwrap();
        let error = arena.insert_at(handle, ()).unwrap_err();
        assert_eq!(StatusCode::GenerationExhausted, error.code);
    }
}
