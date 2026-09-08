use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::sync::Arc;

use ash::vk;
use ash::vk::Handle as _;

use super::device::VulkanContext;
use super::resources::VulkanObjects;
use super::trace;
use crate::render::vulkanic::commands::{
    AttachmentLoadOp, AttachmentStoreOp, BufferImageCopyRegion, CommandOp, TextureImageCopyRegion,
    TextureRowOrder, TextureUsageState, ValidatedSubmissionBatch,
};
use crate::render::vulkanic::error::{GalError, GalResult};
use crate::render::vulkanic::handles::{Handle, HandleKind};
use crate::render::vulkanic::metrics::elapsed_nanos_u64;
use crate::render::vulkanic::sync::SubmissionId;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct VulkanLoweringMetrics {
    pub(super) command_buffer_alloc_nanos: u64,
    pub(super) command_buffer_begin_nanos: u64,
    pub(super) command_recording_nanos: u64,
    pub(super) command_buffer_end_nanos: u64,
    pub(super) queue_submit_nanos: u64,
    pub(super) timeline_poll_nanos: u64,
    pub(super) timeline_wait_nanos: u64,
    pub(super) device_wait_idle_nanos: u64,
    // Cumulative ownership events, not native VkCommandBuffer destruction:
    // retirement returns buffers to the bounded recycle pool and the pool is
    // destroyed only when the lowerer shuts down.
    pub(super) command_buffers_allocated: u64,
    pub(super) command_buffers_freed: u64,
    pub(super) wait_count: u64,
    pub(super) device_wait_idle_count: u64,
    pub(super) gpu_timestamp_status: u64,
    pub(super) gpu_shadow_depth_nanos: u64,
    pub(super) gpu_terrain_opaque_nanos: u64,
    pub(super) gpu_terrain_cutout_nanos: u64,
    pub(super) gpu_deferred_lighting_nanos: u64,
    pub(super) gpu_composite0_nanos: u64,
    pub(super) gpu_composite1_nanos: u64,
    pub(super) gpu_final_output_nanos: u64,
    pub(super) gpu_frame_total_nanos: u64,
}

pub(super) struct SubmissionLowerer {
    context: Arc<VulkanContext>,
    pending: VecDeque<EncodedSubmission>,
    in_flight: VecDeque<InFlightSubmission>,
    completed: SubmissionId,
    completed_host_reads: Vec<CompletedHostRead>,
    metrics: VulkanLoweringMetrics,
    timestamp_pool: Option<vk::QueryPool>,
    next_timestamp_set: u32,
    live_command_buffers: HashSet<vk::CommandBuffer>,
    recycled_command_buffers: Vec<vk::CommandBuffer>,
}

const GPU_TIMESTAMP_SET_COUNT: u32 = 8;
const GPU_TIMESTAMP_QUERIES_PER_SET: u32 = 16;
// Whole-frame submissions are independent of swapchain acquire/present
// slots, so bound their native command-buffer window explicitly.
const MAX_IN_FLIGHT_SUBMISSIONS: usize = 8;
// Host readbacks are diagnostic payloads. Retain only a bounded recent window
// so full-frame attachment captures cannot grow process memory linearly.
const MAX_COMPLETED_HOST_READ_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GpuTimestampQuery {
    FrameStart = 0,
    ShadowDepthStart = 1,
    ShadowDepthEnd = 2,
    TerrainOpaqueStart = 3,
    TerrainOpaqueEnd = 4,
    TerrainCutoutStart = 5,
    TerrainCutoutEnd = 6,
    DeferredLightingStart = 7,
    DeferredLightingEnd = 8,
    Composite0Start = 9,
    Composite0End = 10,
    Composite1Start = 11,
    Composite1End = 12,
    FinalOutputStart = 13,
    FinalOutputEnd = 14,
    FrameEnd = 15,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct GpuTimestampSet {
    base_query: u32,
    active: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct GpuTimestampResult {
    status: u64,
    shadow_depth_nanos: u64,
    terrain_opaque_nanos: u64,
    terrain_cutout_nanos: u64,
    deferred_lighting_nanos: u64,
    composite0_nanos: u64,
    composite1_nanos: u64,
    final_output_nanos: u64,
    frame_total_nanos: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TimestampPassKind {
    ShadowDepth,
    TerrainOpaque,
    TerrainCutout,
    DeferredLighting,
    Composite0,
    Composite1,
    FinalOutput,
}

impl TimestampPassKind {
    fn start_query(self) -> GpuTimestampQuery {
        match self {
            Self::ShadowDepth => GpuTimestampQuery::ShadowDepthStart,
            Self::TerrainOpaque => GpuTimestampQuery::TerrainOpaqueStart,
            Self::TerrainCutout => GpuTimestampQuery::TerrainCutoutStart,
            Self::DeferredLighting => GpuTimestampQuery::DeferredLightingStart,
            Self::Composite0 => GpuTimestampQuery::Composite0Start,
            Self::Composite1 => GpuTimestampQuery::Composite1Start,
            Self::FinalOutput => GpuTimestampQuery::FinalOutputStart,
        }
    }

    fn end_query(self) -> GpuTimestampQuery {
        match self {
            Self::ShadowDepth => GpuTimestampQuery::ShadowDepthEnd,
            Self::TerrainOpaque => GpuTimestampQuery::TerrainOpaqueEnd,
            Self::TerrainCutout => GpuTimestampQuery::TerrainCutoutEnd,
            Self::DeferredLighting => GpuTimestampQuery::DeferredLightingEnd,
            Self::Composite0 => GpuTimestampQuery::Composite0End,
            Self::Composite1 => GpuTimestampQuery::Composite1End,
            Self::FinalOutput => GpuTimestampQuery::FinalOutputEnd,
        }
    }
}

impl SubmissionLowerer {
    pub(super) fn new(context: Arc<VulkanContext>) -> Self {
        let timestamp_pool = if gpu_timestamps_enabled() {
            create_timestamp_pool(&context).ok()
        } else {
            None
        };
        Self {
            context,
            pending: VecDeque::new(),
            in_flight: VecDeque::new(),
            completed: SubmissionId(0),
            completed_host_reads: Vec::new(),
            metrics: VulkanLoweringMetrics::default(),
            timestamp_pool,
            next_timestamp_set: 0,
            live_command_buffers: HashSet::new(),
            recycled_command_buffers: Vec::new(),
        }
    }

    pub(super) fn encode(
        &mut self,
        objects: &VulkanObjects,
        batch: &ValidatedSubmissionBatch,
    ) -> GalResult<()> {
        stdout_trace(&format!(
            "vulkan.encode.batch label={} lists={}",
            sanitize_label(&batch.label),
            batch.command_lists.len()
        ));
        let alloc_started = std::time::Instant::now();
        let timestamp_set = self.allocate_timestamp_set();
        self.metrics.command_buffer_alloc_nanos = self
            .metrics
            .command_buffer_alloc_nanos
            .saturating_add(elapsed_nanos_u64(alloc_started));
        let _zone = trace::Zone::new("vulkan.lowering.command-recording");
        let mut state = EncodingState {
            timestamp_set,
            ..EncodingState::default()
        };
        let mut command_buffers = Vec::with_capacity(batch.command_lists.len().max(1));
        let result: GalResult<()> = (|| {
            let count = batch.command_lists.len().max(1);
            for index in 0..count {
                // Primary command buffers do not inherit recording state from
                // one another.  Source partitioning promises complete pass
                // segments per list; enforce that contract here instead of
                // allowing a later list to rely on a render pass or pipeline
                // binding recorded in an earlier command buffer.
                if state.in_pass || state.frame_present.is_some() {
                    return Err(GalError::backend(
                        "command-list partition crossed a live render pass",
                    ));
                }
                state.graphics_pipeline = None;
                state.compute_pipeline = None;
                state.pipeline_layout = None;
                let command_buffer = self.allocate_command_buffer()?;
                if !self.live_command_buffers.insert(command_buffer) {
                    return Err(GalError::backend(
                        "Vulkan allocated a duplicate live command buffer",
                    ));
                }
                command_buffers.push(command_buffer);
                self.metrics.command_buffers_allocated += 1;
                let begin_info = vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
                let begin_started = std::time::Instant::now();
                unsafe {
                    self.context
                        .device
                        .begin_command_buffer(command_buffer, &begin_info)
                }
                .map_err(|error| {
                    GalError::backend(format!("failed to begin Vulkan command buffer: {error:?}"))
                })?;
                self.metrics.command_buffer_begin_nanos = self
                    .metrics
                    .command_buffer_begin_nanos
                    .saturating_add(elapsed_nanos_u64(begin_started));
                let recording_started = std::time::Instant::now();
                unsafe {
                    self.context.begin_label(
                        command_buffer,
                        &format!("gal.batch.{}", sanitize_label(&batch.label)),
                    )
                };
                if index == 0 {
                    if let (Some(pool), true) = (self.timestamp_pool, timestamp_set.active) {
                        unsafe {
                            self.context.device.cmd_reset_query_pool(
                                command_buffer,
                                pool,
                                timestamp_set.base_query,
                                GPU_TIMESTAMP_QUERIES_PER_SET,
                            );
                            self.write_timestamp(
                                command_buffer,
                                &state,
                                GpuTimestampQuery::FrameStart,
                                vk::PipelineStageFlags::TOP_OF_PIPE,
                            );
                        }
                    }
                }
                if let Some(list) = batch.command_lists.get(index) {
                    stdout_trace(&format!(
                        "vulkan.encode.list label={} ops={}",
                        sanitize_label(&list.label),
                        list.operations.len()
                    ));
                    unsafe {
                        self.context.begin_label(
                            command_buffer,
                            &format!("gal.command-list.{}", sanitize_label(&list.label)),
                        )
                    };
                    for op in &list.operations {
                        stdout_trace(&format!(
                            "vulkan.encode.begin cb=0x{:016x} op={}",
                            command_buffer.as_raw(),
                            command_op_kind(op)
                        ));
                        self.encode_op(objects, command_buffer, &mut state, op)?;
                        stdout_trace(&format!(
                            "vulkan.encode.end cb=0x{:016x} op={}",
                            command_buffer.as_raw(),
                            command_op_kind(op)
                        ));
                    }
                    unsafe { self.context.end_label(command_buffer) };
                }
                if state.in_pass || state.frame_present.is_some() {
                    return Err(GalError::backend(
                        "command-list partition ended with a live render pass",
                    ));
                }
                if index + 1 == count {
                    self.transition_pending_frame_targets_to_present(command_buffer, &mut state);
                    if timestamp_set.active {
                        unsafe {
                            self.write_timestamp(
                                command_buffer,
                                &state,
                                GpuTimestampQuery::FrameEnd,
                                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                            )
                        };
                    }
                }
                unsafe { self.context.end_label(command_buffer) };
                self.metrics.command_recording_nanos = self
                    .metrics
                    .command_recording_nanos
                    .saturating_add(elapsed_nanos_u64(recording_started));
                let end_started = std::time::Instant::now();
                unsafe { self.context.device.end_command_buffer(command_buffer) }.map_err(
                    |error| {
                        GalError::backend(format!("failed to end Vulkan command buffer: {error:?}"))
                    },
                )?;
                self.metrics.command_buffer_end_nanos = self
                    .metrics
                    .command_buffer_end_nanos
                    .saturating_add(elapsed_nanos_u64(end_started));
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.free_command_buffers(&command_buffers)?;
            self.metrics.command_buffers_freed += command_buffers.len() as u64;
            return Err(error);
        }
        self.pending.push_back(EncodedSubmission {
            command_buffers,
            host_reads: state.host_reads,
            timestamp_set,
        });
        Ok(())
    }

    pub(super) fn submit(&mut self, id: SubmissionId) -> GalResult<()> {
        if self.in_flight.len() >= MAX_IN_FLIGHT_SUBMISSIONS {
            let oldest = self
                .in_flight
                .front()
                .map(|submission| submission.id)
                .expect("in-flight window is non-empty at its limit");
            // Wait only for the oldest timeline value. This preserves
            // explicit asynchronous ownership without allowing unbounded
            // command buffers or descriptor-backed resources to accumulate.
            self.retire(oldest)?;
        }
        let Some(encoded) = self.pending.pop_front() else {
            return Err(GalError::backend(
                "Vulkan submit called without encoded commands",
            ));
        };
        let _zone = trace::Zone::new("vulkan.backend.queue-submit");
        let command_buffer_infos = encoded
            .command_buffers
            .iter()
            .map(|command_buffer| {
                vk::CommandBufferSubmitInfo::default().command_buffer(*command_buffer)
            })
            .collect::<Vec<_>>();
        let signal_info = [vk::SemaphoreSubmitInfo::default()
            .semaphore(self.context.timeline)
            .value(id.0)
            .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)];
        let submit = vk::SubmitInfo2::default()
            .command_buffer_infos(&command_buffer_infos)
            .signal_semaphore_infos(&signal_info);
        let queue_submit_started = std::time::Instant::now();
        let submit_result = unsafe {
            self.context
                .device
                .queue_submit2(self.context.queue, &[submit], vk::Fence::null())
        };
        self.metrics.queue_submit_nanos = self
            .metrics
            .queue_submit_nanos
            .saturating_add(elapsed_nanos_u64(queue_submit_started));
        if let Err(error) = submit_result {
            // queue_submit2 did not take ownership of these command buffers.
            // Release them immediately so a failed submission cannot leak
            // live ownership or exhaust the bounded command-buffer pool.
            let failure_bytes = format!("Vulkan queue submit failed: {error:?}").into_bytes();
            for request in &encoded.host_reads {
                self.completed_host_reads.push(CompletedHostRead {
                    submission: id,
                    buffer: request.buffer,
                    offset: request.offset,
                    bytes: failure_bytes.clone(),
                });
            }
            let command_buffer_count = encoded.command_buffers.len() as u64;
            self.free_command_buffers(&encoded.command_buffers)?;
            self.metrics.command_buffers_freed = self
                .metrics
                .command_buffers_freed
                .saturating_add(command_buffer_count);
            return Err(GalError::backend(
                String::from_utf8_lossy(&failure_bytes).into_owned(),
            ));
        }
        self.in_flight.push_back(InFlightSubmission {
            id,
            command_buffers: encoded.command_buffers,
            host_reads: encoded.host_reads,
            timestamp_set: encoded.timestamp_set,
        });
        debug_assert!(
            self.in_flight.len() <= MAX_IN_FLIGHT_SUBMISSIONS,
            "Vulkan lowerer exceeded its bounded in-flight submission window"
        );
        if std::env::var_os("MATTMC_TRACE_SUBMISSIONS").is_some() {
            println!(
                "vulkan.submission.ownership id={} pending={} in_flight={} live_command_buffers={} allocated={} freed={}",
                id.0,
                self.pending.len(),
                self.in_flight.len(),
                self.live_command_buffers.len(),
                self.metrics.command_buffers_allocated,
                self.metrics.command_buffers_freed,
            );
        }
        Ok(())
    }

    pub(super) fn completed_submission(&mut self) -> SubmissionId {
        let poll_started = std::time::Instant::now();
        if let Ok(value) = unsafe {
            self.context
                .device
                .get_semaphore_counter_value(self.context.timeline)
        } {
            self.completed = SubmissionId(value);
        }
        self.metrics.timeline_poll_nanos = self
            .metrics
            .timeline_poll_nanos
            .saturating_add(elapsed_nanos_u64(poll_started));
        while let Some(front) = self.in_flight.front() {
            if front.id > self.completed {
                break;
            }
            let complete = self.in_flight.pop_front().expect("front existed");
            self.complete_host_reads(&complete);
            self.complete_gpu_timestamps(&complete);
            self.free_command_buffers(&complete.command_buffers)
                .expect("retiring a Vulkan submission must own its command buffers");
            self.metrics.command_buffers_freed += complete.command_buffers.len() as u64;
        }
        self.completed
    }

    pub(super) fn retire(&mut self, completed: SubmissionId) -> GalResult<()> {
        while let Some(front) = self.in_flight.front() {
            if front.id > completed {
                break;
            }
            let _zone = trace::Zone::new("vulkan.backend.wait-timeline");
            let wait_started = std::time::Instant::now();
            let context = &self.context;
            let complete = wait_then_pop_front(&mut self.in_flight,
                |entry| wait_timeline(context, entry.id))?.expect("front existed");
            self.metrics.timeline_wait_nanos = self
                .metrics
                .timeline_wait_nanos
                .saturating_add(elapsed_nanos_u64(wait_started));
            self.metrics.wait_count += 1;
            self.free_command_buffers(&complete.command_buffers)?;
            self.metrics.command_buffers_freed += complete.command_buffers.len() as u64;
            self.complete_host_reads(&complete);
            self.complete_gpu_timestamps(&complete);
            self.completed = complete.id;
        }
        Ok(())
    }

    /// Wait for and retire every submission currently owned by this lowerer.
    ///
    /// Presentation is the ownership boundary for the native frame surface.
    /// Resource-update submissions may be issued after the frontend's frame
    /// token, so retiring only that token can leave newer command buffers and
    /// descriptor-backed resources alive indefinitely.  Drain the explicit
    /// in-flight queue at that boundary; this does not use device-idle and
    /// preserves timeline-ordered synchronization.
    pub(super) fn retire_all_in_flight(&mut self) -> GalResult<()> {
        let Some(latest) = self.in_flight.back().map(|submission| submission.id) else {
            return Ok(());
        };
        self.retire(latest)
    }

    pub(super) fn completed_host_reads_snapshot(&self) -> &[CompletedHostRead] {
        &self.completed_host_reads
    }

    pub(super) fn metrics(&self) -> VulkanLoweringMetrics {
        self.metrics
    }

    pub(super) fn wait_idle_and_clear(&mut self) {
        let wait_started = std::time::Instant::now();
        let _ = self.context.wait_idle();
        self.metrics.device_wait_idle_nanos = self
            .metrics
            .device_wait_idle_nanos
            .saturating_add(elapsed_nanos_u64(wait_started));
        self.metrics.device_wait_idle_count += 1;
        let pending = self.pending.drain(..).collect::<Vec<_>>();
        for encoded in pending {
            self.free_command_buffers(&encoded.command_buffers)
                .expect("clearing pending Vulkan commands must own their command buffers");
            self.metrics.command_buffers_freed += encoded.command_buffers.len() as u64;
        }
        let in_flight = self.in_flight.drain(..).collect::<Vec<_>>();
        for complete in in_flight {
            self.free_command_buffers(&complete.command_buffers)
                .expect("clearing in-flight Vulkan commands must own their command buffers");
            self.metrics.command_buffers_freed += complete.command_buffers.len() as u64;
        }
        if !self.recycled_command_buffers.is_empty() {
            unsafe {
                self.context.device.free_command_buffers(
                    self.context.command_pool,
                    &self.recycled_command_buffers,
                );
            }
            self.recycled_command_buffers.clear();
        }
    }

    fn allocate_timestamp_set(&mut self) -> GpuTimestampSet {
        if self.timestamp_pool.is_none()
            || self.context.timestamp_valid_bits == 0
            || self.context.timestamp_period <= 0.0
        {
            return GpuTimestampSet::default();
        }
        for _ in 0..GPU_TIMESTAMP_SET_COUNT {
            let set_index = self.next_timestamp_set % GPU_TIMESTAMP_SET_COUNT;
            self.next_timestamp_set = self.next_timestamp_set.wrapping_add(1);
            let base_query = set_index * GPU_TIMESTAMP_QUERIES_PER_SET;
            if !self.timestamp_set_in_use(base_query) {
                return GpuTimestampSet {
                    base_query,
                    active: true,
                };
            }
        }
        GpuTimestampSet::default()
    }

    fn timestamp_set_in_use(&self, base_query: u32) -> bool {
        self.pending.iter().any(|pending| {
            pending.timestamp_set.active && pending.timestamp_set.base_query == base_query
        }) || self.in_flight.iter().any(|in_flight| {
            in_flight.timestamp_set.active && in_flight.timestamp_set.base_query == base_query
        })
    }

    unsafe fn write_timestamp(
        &self,
        command_buffer: vk::CommandBuffer,
        state: &EncodingState,
        query: GpuTimestampQuery,
        stage: vk::PipelineStageFlags,
    ) {
        let Some(pool) = self.timestamp_pool else {
            return;
        };
        if !state.timestamp_set.active {
            return;
        }
        unsafe {
            self.context.device.cmd_write_timestamp(
                command_buffer,
                stage,
                pool,
                state.timestamp_set.base_query + query as u32,
            );
        }
    }

    unsafe fn switch_timestamp_pass(
        &self,
        command_buffer: vk::CommandBuffer,
        state: &mut EncodingState,
        next: Option<TimestampPassKind>,
    ) {
        if state.current_timestamp_pass == next {
            return;
        }
        if let Some(current) = state.current_timestamp_pass {
            unsafe {
                self.write_timestamp(
                    command_buffer,
                    state,
                    current.end_query(),
                    vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                );
            }
        }
        state.current_timestamp_pass = next;
        if let Some(next) = state.current_timestamp_pass {
            unsafe {
                self.write_timestamp(
                    command_buffer,
                    state,
                    next.start_query(),
                    vk::PipelineStageFlags::TOP_OF_PIPE,
                );
            }
        }
    }

    fn allocate_command_buffer(&mut self) -> GalResult<vk::CommandBuffer> {
        if let Some(command_buffer) = self.recycled_command_buffers.pop() {
            unsafe {
                self.context
                    .device
                    .reset_command_buffer(command_buffer, vk::CommandBufferResetFlags::empty())
            }
            .map_err(|error| {
                GalError::backend(format!(
                    "failed to reset recycled Vulkan command buffer: {error:?}"
                ))
            })?;
            return Ok(command_buffer);
        }
        let allocate_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(self.context.command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        unsafe { self.context.device.allocate_command_buffers(&allocate_info) }
            .map_err(|error| {
                GalError::backend(format!(
                    "failed to allocate Vulkan command buffer: {error:?}"
                ))
            })?
            .into_iter()
            .next()
            .ok_or_else(|| GalError::backend("Vulkan returned no command buffer"))
    }

    fn free_command_buffers(&mut self, command_buffers: &[vk::CommandBuffer]) -> GalResult<()> {
        let requested = command_buffers.iter().copied().collect::<HashSet<_>>();
        if requested.len() != command_buffers.len()
            || requested
                .iter()
                .any(|command_buffer| !self.live_command_buffers.contains(command_buffer))
        {
            return Err(GalError::backend(
                "Vulkan command buffer retired more than once or was never owned",
            ));
        }
        for command_buffer in command_buffers {
            if !self.live_command_buffers.remove(command_buffer) {
                return Err(GalError::backend(
                    "Vulkan command buffer retired more than once or was never owned",
                ));
            }
        }
        for command_buffer in command_buffers {
            unsafe {
                self.context
                    .device
                    .reset_command_buffer(*command_buffer, vk::CommandBufferResetFlags::empty())
            }
            .map_err(|error| {
                GalError::backend(format!(
                    "failed to reset retired Vulkan command buffer: {error:?}"
                ))
            })?;
            self.recycled_command_buffers.push(*command_buffer);
        }
        Ok(())
    }

    fn encode_op(
        &self,
        objects: &VulkanObjects,
        command_buffer: vk::CommandBuffer,
        state: &mut EncodingState,
        op: &CommandOp,
    ) -> GalResult<()> {
        unsafe {
            match op {
                CommandOp::BeginPass {
                    pass,
                    target,
                    colors,
                    depth_stencil,
                } => {
                    stdout_trace(&format!(
                        "vulkan.begin-pass target=0x{:016x} colors={} depth={}",
                        target.raw(),
                        colors.len(),
                        depth_stencil.is_some()
                    ));
                    let pass_object = objects.render_pass(*pass)?;
                    let timestamp_pass = timestamp_pass_kind(&pass_object.label);
                    if let Some(pass_kind) = timestamp_pass {
                        self.write_timestamp(
                            command_buffer,
                            state,
                            pass_kind.start_query(),
                            vk::PipelineStageFlags::TOP_OF_PIPE,
                        );
                    }
                    state.current_timestamp_pass = timestamp_pass;
                    self.context
                        .begin_label(command_buffer, &format!("gal.pass.0x{:016x}", pass.raw()));
                    if pass_object.target != *target {
                        return Err(GalError::backend(
                            "render pass target mismatch during lowering",
                        ));
                    }
                    let (color_attachments, depth_attachment, extent, frame_present) = if target
                        .kind()
                        == Some(HandleKind::FrameTarget)
                    {
                        let frame = objects.frame_target(*target)?;
                        let clear_frame = !state.frame_target_touched;
                        let old_layout = state
                            .frame_target_layouts
                            .get(target)
                            .copied()
                            .unwrap_or(frame.image_layout);
                        let range = vk::ImageSubresourceRange {
                            aspect_mask: vk::ImageAspectFlags::COLOR,
                            base_mip_level: 0,
                            level_count: 1,
                            base_array_layer: 0,
                            layer_count: 1,
                        };
                        let to_attachment = vk::ImageMemoryBarrier2::default()
                            .src_stage_mask(frame_layout_stage_mask(old_layout))
                            .src_access_mask(frame_layout_access_mask(old_layout))
                            .dst_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                            .dst_access_mask(
                                vk::AccessFlags2::COLOR_ATTACHMENT_READ
                                    | vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
                            )
                            .old_layout(old_layout)
                            .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                            .image(frame.image)
                            .subresource_range(range);
                        self.context.device.cmd_pipeline_barrier2(
                            command_buffer,
                            &vk::DependencyInfo::default()
                                .image_memory_barriers(std::slice::from_ref(&to_attachment)),
                        );
                        stdout_trace("vulkan.begin-pass.frame-barrier");
                        state
                            .frame_target_layouts
                            .insert(*target, vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
                        let clear = colors
                            .first()
                            .and_then(|attachment| attachment.clear_color)
                            .map(|color| [color.r, color.g, color.b, color.a])
                            .unwrap_or([0.08, 0.31, 0.74, 1.0]);
                        let load = if clear_frame {
                            vk::AttachmentLoadOp::CLEAR
                        } else if colors.is_empty() {
                            vk::AttachmentLoadOp::LOAD
                        } else {
                            load_op(colors[0].load_op)
                        };
                        trace::message(&format!(
                                "gal.frame.target.begin backend=vulkan frame={} image={} view=0x{:016x} extent={}x{} layout={} load={} clear={:.3},{:.3},{:.3},{:.3}",
                                frame.frame_id,
                                frame.image_index,
                                frame.image_view.as_raw(),
                                frame.extent.width,
                                frame.extent.height,
                                old_layout.as_raw(),
                                load.as_raw(),
                                clear[0],
                                clear[1],
                                clear[2],
                                clear[3]
                            ));
                        state.frame_target_touched = true;
                        let depth_attachment = depth_stencil
                            .as_ref()
                            .map(|attachment| {
                                let view = objects.texture_view(attachment.view)?;
                                Ok(vk::RenderingAttachmentInfo::default()
                                    .image_view(view.view)
                                    .image_layout(vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL)
                                    .load_op(load_op(attachment.load_op))
                                    .store_op(store_op(attachment.store_op))
                                    .clear_value(vk::ClearValue {
                                        depth_stencil: vk::ClearDepthStencilValue {
                                            depth: 1.0,
                                            stencil: 0,
                                        },
                                    }))
                            })
                            .transpose()?;
                        (
                            vec![vk::RenderingAttachmentInfo::default()
                                .image_view(frame.image_view)
                                .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                                .load_op(load)
                                .store_op(if colors.is_empty() {
                                    vk::AttachmentStoreOp::STORE
                                } else {
                                    store_op(colors[0].store_op)
                                })
                                .clear_value(vk::ClearValue {
                                    color: vk::ClearColorValue { float32: clear },
                                })],
                            depth_attachment,
                            frame.extent,
                            Some(FramePresentTransition {
                                target: *target,
                                image: frame.image,
                                image_index: frame.image_index,
                                frame_id: frame.frame_id,
                                range,
                            }),
                        )
                    } else {
                        let target_object = objects.render_target(*target)?;
                        let color_attachments = colors
                            .iter()
                            .map(|attachment| {
                                let view = objects.texture_view(attachment.view)?;
                                Ok(vk::RenderingAttachmentInfo::default()
                                    .image_view(view.view)
                                    .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                                    .load_op(load_op(attachment.load_op))
                                    .store_op(store_op(attachment.store_op))
                                    .clear_value(vk::ClearValue {
                                        color: vk::ClearColorValue {
                                            float32: attachment
                                                .clear_color
                                                .map(|color| [color.r, color.g, color.b, color.a])
                                                .unwrap_or([0.0, 0.0, 0.0, 0.0]),
                                        },
                                    }))
                            })
                            .collect::<GalResult<Vec<_>>>()?;
                        let depth_attachment = depth_stencil
                            .as_ref()
                            .map(|attachment| {
                                let view = objects.texture_view(attachment.view)?;
                                Ok(vk::RenderingAttachmentInfo::default()
                                    .image_view(view.view)
                                    .image_layout(vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL)
                                    .load_op(load_op(attachment.load_op))
                                    .store_op(store_op(attachment.store_op))
                                    .clear_value(vk::ClearValue {
                                        depth_stencil: vk::ClearDepthStencilValue {
                                            depth: 1.0,
                                            stencil: 0,
                                        },
                                    }))
                            })
                            .transpose()?;
                        (
                            color_attachments,
                            depth_attachment,
                            target_object.extent,
                            None,
                        )
                    };
                    let mut rendering = vk::RenderingInfo::default()
                        .render_area(vk::Rect2D {
                            offset: vk::Offset2D { x: 0, y: 0 },
                            extent: vk::Extent2D {
                                width: extent.width,
                                height: extent.height,
                            },
                        })
                        .layer_count(1)
                        .color_attachments(&color_attachments);
                    if let Some(depth_attachment) = depth_attachment.as_ref() {
                        rendering = rendering.depth_attachment(depth_attachment);
                    }
                    self.context
                        .device
                        .cmd_begin_rendering(command_buffer, &rendering);
                    stdout_trace("vulkan.begin-pass.cmd-begin-rendering");
                    let viewport = vk::Viewport {
                        x: 0.0,
                        y: extent.height as f32,
                        width: extent.width as f32,
                        height: -(extent.height as f32),
                        min_depth: 0.0,
                        max_depth: 1.0,
                    };
                    let scissor = vk::Rect2D {
                        offset: vk::Offset2D { x: 0, y: 0 },
                        extent: vk::Extent2D {
                            width: extent.width,
                            height: extent.height,
                        },
                    };
                    self.context
                        .device
                        .cmd_set_viewport(command_buffer, 0, &[viewport]);
                    self.context
                        .device
                        .cmd_set_scissor(command_buffer, 0, &[scissor]);
                    state.in_pass = true;
                    state.frame_present = frame_present;
                }
                CommandOp::EndPass => {
                    self.context.device.cmd_end_rendering(command_buffer);
                    if let Some(pass_kind) = state.current_timestamp_pass {
                        self.write_timestamp(
                            command_buffer,
                            state,
                            pass_kind.end_query(),
                            vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                        );
                    }
                    if let Some(present) = state.frame_present.take() {
                        state.pending_frame_presents.insert(present.target, present);
                    }
                    self.context.end_label(command_buffer);
                    state.in_pass = false;
                    state.graphics_pipeline = None;
                    state.compute_pipeline = None;
                    state.pipeline_layout = None;
                    state.current_timestamp_pass = None;
                }
                CommandOp::BindGraphicsPipeline(handle) => {
                    let pipeline = objects.graphics_pipeline(*handle)?;
                    self.context.device.cmd_bind_pipeline(
                        command_buffer,
                        vk::PipelineBindPoint::GRAPHICS,
                        pipeline.pipeline.pipeline,
                    );
                    if let Some(pipeline_timestamp_pass) = timestamp_pipeline_kind(&pipeline.label)
                    {
                        self.switch_timestamp_pass(
                            command_buffer,
                            state,
                            Some(pipeline_timestamp_pass),
                        );
                    }
                    state.graphics_pipeline = Some(*handle);
                    state.compute_pipeline = None;
                    state.pipeline_layout = Some(pipeline.layout);
                }
                CommandOp::BindComputePipeline(handle) => {
                    let pipeline = objects.compute_pipeline(*handle)?;
                    self.context.device.cmd_bind_pipeline(
                        command_buffer,
                        vk::PipelineBindPoint::COMPUTE,
                        pipeline.pipeline,
                    );
                    state.compute_pipeline = Some(*handle);
                    state.graphics_pipeline = None;
                    state.pipeline_layout = Some(pipeline.layout);
                }
                CommandOp::BindResourceSet {
                    pipeline_layout,
                    set_index,
                    set,
                    dynamic_offsets,
                } => {
                    if std::env::var_os("MATTMC_TRACE_ENTITY_TEXTURE_OPS").is_some() {
                        eprintln!(
                            "vulkan.bind-resource-set layout={:?} set_index={} set={:?} dynamic_offsets={:?}",
                            pipeline_layout,
                            set_index,
                            set,
                            dynamic_offsets,
                        );
                    }
                    let layout = objects.pipeline_layout(*pipeline_layout)?;
                    let set = objects.resource_set(*set)?;
                    let bind_dynamic_offsets = if dynamic_offsets.is_empty() {
                        Cow::Borrowed(set.dynamic_offsets.as_slice())
                    } else {
                        Cow::Owned(
                            dynamic_offsets
                                .iter()
                                .copied()
                                .map(|offset| {
                                    u32::try_from(offset).map_err(|_| {
                                        GalError::backend("dynamic descriptor offset exceeds u32")
                                    })
                                })
                                .collect::<GalResult<Vec<_>>>()?,
                        )
                    };
                    let bind_point = if state.graphics_pipeline.is_some() {
                        vk::PipelineBindPoint::GRAPHICS
                    } else {
                        vk::PipelineBindPoint::COMPUTE
                    };
                    self.context.device.cmd_bind_descriptor_sets(
                        command_buffer,
                        bind_point,
                        layout.layout,
                        *set_index,
                        &[set.set],
                        bind_dynamic_offsets.as_ref(),
                    );
                }
                CommandOp::SetVertexBuffer {
                    slot,
                    buffer,
                    offset,
                } => {
                    let buffer = objects.buffer(*buffer)?;
                    self.context.device.cmd_bind_vertex_buffers(
                        command_buffer,
                        *slot,
                        &[buffer.buffer],
                        &[*offset],
                    );
                }
                CommandOp::SetIndexBuffer {
                    buffer,
                    offset,
                    index_type,
                } => {
                    let buffer = objects.buffer(*buffer)?;
                    self.context.device.cmd_bind_index_buffer(
                        command_buffer,
                        buffer.buffer,
                        *offset,
                        vk_index_type(*index_type),
                    );
                }
                CommandOp::Draw {
                    vertices,
                    instances,
                } => {
                    self.context
                        .device
                        .cmd_draw(command_buffer, *vertices, *instances, 0, 0);
                }
                CommandOp::DrawIndexed { indices, instances } => {
                    self.context.device.cmd_draw_indexed(
                        command_buffer,
                        *indices,
                        *instances,
                        0,
                        0,
                        0,
                    );
                }
                CommandOp::DrawIndirect {
                    buffer,
                    offset,
                    draw_count,
                } => {
                    let buffer = objects.buffer(*buffer)?;
                    self.context.device.cmd_draw_indirect(
                        command_buffer,
                        buffer.buffer,
                        *offset,
                        *draw_count,
                        std::mem::size_of::<vk::DrawIndirectCommand>() as u32,
                    );
                }
                CommandOp::Dispatch {
                    groups_x,
                    groups_y,
                    groups_z,
                } => {
                    self.context.device.cmd_dispatch(
                        command_buffer,
                        *groups_x,
                        *groups_y,
                        *groups_z,
                    );
                }
                CommandOp::DispatchIndirect { buffer, offset } => {
                    let buffer = objects.buffer(*buffer)?;
                    self.context.device.cmd_dispatch_indirect(
                        command_buffer,
                        buffer.buffer,
                        *offset,
                    );
                }
                CommandOp::CopyBuffer { src, dst, size } => {
                    let _zone = trace::Zone::new("vulkan.lowering.copy-buffer");
                    let src = objects.buffer(*src)?;
                    let dst = objects.buffer(*dst)?;
                    let region = vk::BufferCopy {
                        src_offset: 0,
                        dst_offset: 0,
                        size: *size,
                    };
                    self.context.device.cmd_copy_buffer(
                        command_buffer,
                        src.buffer,
                        dst.buffer,
                        &[region],
                    );
                }
                CommandOp::CopyBufferToTexture(region) => {
                    let _zone = trace::Zone::new("vulkan.lowering.copy-buffer-to-texture");
                    let buffer = objects.buffer(region.buffer)?;
                    let texture = objects.texture(region.texture)?;
                    if std::env::var_os("MATTMC_TRACE_ENTITY_TEXTURE_OPS").is_some() {
                        eprintln!(
                            "vulkan.copy-buffer-to-texture buffer={:?} texture={:?} label={} offset={} row={} rows={} extent={}x{}x{}",
                            region.buffer,
                            region.texture,
                            texture.label,
                            region.buffer_offset,
                            region.bytes_per_row,
                            region.rows_per_image,
                            region.extent.width,
                            region.extent.height,
                            region.extent.depth,
                        );
                    }
                    let copy =
                        buffer_image_copy(region, texture.aspect, texture.copy_bytes_per_texel);
                    self.context.device.cmd_copy_buffer_to_image(
                        command_buffer,
                        buffer.buffer,
                        texture.image,
                        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        &[copy],
                    );
                }
                CommandOp::CopyTextureToBuffer(region) => {
                    let _zone = trace::Zone::new("vulkan.lowering.copy-texture-to-buffer");
                    let buffer = objects.buffer(region.buffer)?;
                    let texture = objects.texture(region.texture)?;
                    let copy =
                        buffer_image_copy(region, texture.aspect, texture.copy_bytes_per_texel);
                    self.context.device.cmd_copy_image_to_buffer(
                        command_buffer,
                        texture.image,
                        vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                        buffer.buffer,
                        &[copy],
                    );
                }
                CommandOp::CopyTexture(region) => {
                    let _zone = trace::Zone::new("vulkan.lowering.copy-texture");
                    let source = objects.texture(region.src_texture)?;
                    let destination = objects.texture(region.dst_texture)?;
                    if region.row_order == TextureRowOrder::Reverse {
                        let source_features = self.context.instance
                            .get_physical_device_format_properties(self.context.physical_device, source.format)
                            .optimal_tiling_features;
                        let destination_features = self.context.instance
                            .get_physical_device_format_properties(self.context.physical_device, destination.format)
                            .optimal_tiling_features;
                        validate_row_reversal_format_features(source_features, destination_features)?;
                        let blit = texture_row_reversal_blit(region, source.aspect, destination.aspect)?;
                        self.context.device.cmd_blit_image(
                            command_buffer, source.image, vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                            destination.image, vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                            &[blit], vk::Filter::NEAREST,
                        );
                    } else {
                        let copy = texture_image_copy(region, source.aspect, destination.aspect);
                        self.context.device.cmd_copy_image(
                        command_buffer,
                        source.image,
                        vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                        destination.image,
                        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        &[copy],
                        );
                    }
                }
                CommandOp::CopyFrameTargetToTexture { src, dst, extent } => {
                    let _zone = trace::Zone::new("vulkan.lowering.copy-frame-target");
                    if state.in_pass {
                        return Err(GalError::backend(
                            "frame-target sampling copy cannot be encoded inside a render pass",
                        ));
                    }
                    if !state.transfer_dst_textures.contains(dst) {
                        return Err(GalError::backend(
                            "frame-target sampling copy requires an explicit TransferDst barrier",
                        ));
                    }
                    let frame = objects.frame_target(*src)?;
                    let destination = objects.texture(*dst)?;
                    let old_layout = state
                        .frame_target_layouts
                        .get(src)
                        .copied()
                        .unwrap_or(frame.image_layout);
                    let source_range = vk::ImageSubresourceRange {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        base_mip_level: 0,
                        level_count: 1,
                        base_array_layer: 0,
                        layer_count: 1,
                    };
                    let source_barrier = vk::ImageMemoryBarrier2::default()
                        .src_stage_mask(frame_layout_stage_mask(old_layout))
                        .src_access_mask(frame_layout_access_mask(old_layout))
                        .dst_stage_mask(vk::PipelineStageFlags2::COPY)
                        .dst_access_mask(vk::AccessFlags2::TRANSFER_READ)
                        .old_layout(old_layout)
                        .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                        .image(frame.image)
                        .subresource_range(source_range);
                    self.context.device.cmd_pipeline_barrier2(
                        command_buffer,
                        &vk::DependencyInfo::default()
                            .image_memory_barriers(std::slice::from_ref(&source_barrier)),
                    );
                    let copy = vk::ImageCopy {
                        src_subresource: vk::ImageSubresourceLayers {
                            aspect_mask: vk::ImageAspectFlags::COLOR,
                            mip_level: 0,
                            base_array_layer: 0,
                            layer_count: 1,
                        },
                        src_offset: vk::Offset3D::default(),
                        dst_subresource: vk::ImageSubresourceLayers {
                            aspect_mask: destination.aspect,
                            mip_level: 0,
                            base_array_layer: 0,
                            layer_count: 1,
                        },
                        dst_offset: vk::Offset3D::default(),
                        extent: vk::Extent3D {
                            width: extent.width,
                            height: extent.height,
                            depth: extent.depth,
                        },
                    };
                    self.context.device.cmd_copy_image(
                        command_buffer,
                        frame.image,
                        vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                        destination.image,
                        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        &[copy],
                    );
                    state
                        .frame_target_layouts
                        .insert(*src, vk::ImageLayout::TRANSFER_SRC_OPTIMAL);
                }
                CommandOp::CopyTextureToFrameTarget { src, dst, extent } => {
                    let _zone = trace::Zone::new("vulkan.lowering.copy-to-frame-target");
                    if state.in_pass {
                        return Err(GalError::backend(
                            "frame-target presentation copy cannot be encoded inside a render pass",
                        ));
                    }
                    // The GAL validator requires the source texture to carry
                    // TransferSrc usage; Vulkan lowering accepts the explicit
                    // source layout established by the preceding barrier.
                    let source = objects.texture(*src)?;
                    let frame = objects.frame_target(*dst)?;
                    let old_layout = state
                        .frame_target_layouts
                        .get(dst)
                        .copied()
                        .unwrap_or(frame.image_layout);
                    let range = vk::ImageSubresourceRange {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        base_mip_level: 0,
                        level_count: 1,
                        base_array_layer: 0,
                        layer_count: 1,
                    };
                    let to_transfer = vk::ImageMemoryBarrier2::default()
                        .src_stage_mask(frame_layout_stage_mask(old_layout))
                        .src_access_mask(frame_layout_access_mask(old_layout))
                        .dst_stage_mask(vk::PipelineStageFlags2::COPY)
                        .dst_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                        .old_layout(old_layout)
                        .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                        .image(frame.image)
                        .subresource_range(range);
                    self.context.device.cmd_pipeline_barrier2(
                        command_buffer,
                        &vk::DependencyInfo::default()
                            .image_memory_barriers(std::slice::from_ref(&to_transfer)),
                    );
                    let copy = vk::ImageCopy {
                        src_subresource: vk::ImageSubresourceLayers {
                            aspect_mask: source.aspect,
                            mip_level: 0,
                            base_array_layer: 0,
                            layer_count: 1,
                        },
                        src_offset: vk::Offset3D::default(),
                        dst_subresource: vk::ImageSubresourceLayers {
                            aspect_mask: vk::ImageAspectFlags::COLOR,
                            mip_level: 0,
                            base_array_layer: 0,
                            layer_count: 1,
                        },
                        dst_offset: vk::Offset3D::default(),
                        extent: vk::Extent3D {
                            width: extent.width,
                            height: extent.height,
                            depth: extent.depth,
                        },
                    };
                    self.context.device.cmd_copy_image(
                        command_buffer,
                        source.image,
                        vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                        frame.image,
                        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        &[copy],
                    );
                    state
                        .frame_target_layouts
                        .insert(*dst, vk::ImageLayout::TRANSFER_DST_OPTIMAL);
                }
                CommandOp::GenerateMipmaps {
                    texture,
                    subresources,
                } => {
                    let _zone = trace::Zone::new("vulkan.lowering.generate-mipmaps");
                    let texture = objects.texture(*texture)?;
                    let mip_end = subresources
                        .base_mip
                        .checked_add(subresources.mip_count)
                        .ok_or_else(|| GalError::backend("mip generation range overflows"))?;
                    for source_mip in subresources.base_mip..mip_end - 1 {
                        let destination_mip = source_mip + 1;
                        let source_extent = vk::Offset3D {
                            x: i32::try_from((texture.extent.width >> source_mip).max(1))
                                .map_err(|_| GalError::backend("source mip width exceeds i32"))?,
                            y: i32::try_from((texture.extent.height >> source_mip).max(1))
                                .map_err(|_| GalError::backend("source mip height exceeds i32"))?,
                            z: i32::try_from((texture.extent.depth >> source_mip).max(1))
                                .map_err(|_| GalError::backend("source mip depth exceeds i32"))?,
                        };
                        let destination_extent = vk::Offset3D {
                            x: i32::try_from((texture.extent.width >> destination_mip).max(1))
                                .map_err(|_| {
                                    GalError::backend("destination mip width exceeds i32")
                                })?,
                            y: i32::try_from((texture.extent.height >> destination_mip).max(1))
                                .map_err(|_| {
                                    GalError::backend("destination mip height exceeds i32")
                                })?,
                            z: i32::try_from((texture.extent.depth >> destination_mip).max(1))
                                .map_err(|_| {
                                    GalError::backend("destination mip depth exceeds i32")
                                })?,
                        };
                        let blit = vk::ImageBlit::default()
                            .src_subresource(vk::ImageSubresourceLayers {
                                aspect_mask: texture.aspect,
                                mip_level: source_mip,
                                base_array_layer: subresources.base_layer,
                                layer_count: subresources.layer_count,
                            })
                            .src_offsets([vk::Offset3D::default(), source_extent])
                            .dst_subresource(vk::ImageSubresourceLayers {
                                aspect_mask: texture.aspect,
                                mip_level: destination_mip,
                                base_array_layer: subresources.base_layer,
                                layer_count: subresources.layer_count,
                            })
                            .dst_offsets([vk::Offset3D::default(), destination_extent]);
                        self.context.device.cmd_blit_image(
                            command_buffer,
                            texture.image,
                            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                            texture.image,
                            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                            &[blit],
                            vk::Filter::LINEAR,
                        );
                        let image_barrier = vk::ImageMemoryBarrier2::default()
                            .src_stage_mask(stage_mask(TextureUsageState::TransferDst))
                            .src_access_mask(access_mask(TextureUsageState::TransferDst))
                            .dst_stage_mask(stage_mask(TextureUsageState::TransferSrc))
                            .dst_access_mask(access_mask(TextureUsageState::TransferSrc))
                            .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                            .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                            .image(texture.image)
                            .subresource_range(vk::ImageSubresourceRange {
                                aspect_mask: texture.aspect,
                                base_mip_level: destination_mip,
                                level_count: 1,
                                base_array_layer: subresources.base_layer,
                                layer_count: subresources.layer_count,
                            });
                        let dependency = vk::DependencyInfo::default()
                            .image_memory_barriers(std::slice::from_ref(&image_barrier));
                        self.context
                            .device
                            .cmd_pipeline_barrier2(command_buffer, &dependency);
                    }
                }
                CommandOp::Barrier(barrier) => {
                    let _zone = trace::Zone::new("vulkan.lowering.barrier");
                    if barrier.resource.kind()
                        == Some(crate::render::vulkanic::handles::HandleKind::Texture)
                        || barrier.resource.kind()
                            == Some(crate::render::vulkanic::handles::HandleKind::TextureView)
                    {
                        let (texture_handle, view_range) = if barrier.resource.kind()
                            == Some(crate::render::vulkanic::handles::HandleKind::TextureView)
                        {
                            let view = objects.texture_view(barrier.resource)?;
                            (
                                view.texture,
                                crate::render::vulkanic::resources::TextureSubresourceRange {
                                    base_mip: view.base_mip,
                                    mip_count: view.mip_levels,
                                    base_layer: view.base_layer,
                                    layer_count: view.array_layers,
                                },
                            )
                        } else {
                            let texture = objects.texture(barrier.resource)?;
                            (
                                barrier.resource,
                                crate::render::vulkanic::resources::TextureSubresourceRange {
                                    base_mip: 0,
                                    mip_count: texture.mip_levels,
                                    base_layer: 0,
                                    layer_count: texture.array_layers,
                                },
                            )
                        };
                        let texture = objects.texture(texture_handle)?;
                        let range = barrier.subresources.unwrap_or(view_range);
                        if std::env::var_os("MATTMC_TRACE_SUBMISSIONS").is_some()
                            && texture.label.contains("source-final-output")
                        {
                            stdout_trace(&format!(
                                "vulkan.barrier source-final-output resource=0x{:016x} texture=0x{:016x} label={} before={:?} after={:?} mip={}..{} layer={}..{}",
                                barrier.resource.raw(),
                                texture_handle.raw(),
                                sanitize_label(&texture.label),
                                barrier.before,
                                barrier.after,
                                range.base_mip,
                                range.mip_count,
                                range.base_layer,
                                range.layer_count,
                            ));
                        }
                        let image_barrier = vk::ImageMemoryBarrier2::default()
                            .src_stage_mask(stage_mask(barrier.before))
                            .src_access_mask(image_access_mask(barrier.before))
                            .dst_stage_mask(stage_mask(barrier.after))
                            .dst_access_mask(image_access_mask(barrier.after))
                            .old_layout(image_layout_for_aspect(barrier.before, texture.aspect))
                            .new_layout(image_layout_for_aspect(barrier.after, texture.aspect))
                            .image(texture.image)
                            .subresource_range(vk::ImageSubresourceRange {
                                aspect_mask: texture.aspect,
                                base_mip_level: range.base_mip,
                                level_count: range.mip_count,
                                base_array_layer: range.base_layer,
                                layer_count: range.layer_count,
                            });
                        let dependency = vk::DependencyInfo::default()
                            .image_memory_barriers(std::slice::from_ref(&image_barrier));
                        self.context
                            .device
                            .cmd_pipeline_barrier2(command_buffer, &dependency);
                        if barrier.after == TextureUsageState::TransferDst {
                            state.transfer_dst_textures.insert(barrier.resource);
                        } else {
                            state.transfer_dst_textures.remove(&barrier.resource);
                        }
                    } else if barrier.resource.kind()
                        == Some(crate::render::vulkanic::handles::HandleKind::Buffer)
                    {
                        let buffer = objects.buffer(barrier.resource)?;
                        let buffer_barrier = vk::BufferMemoryBarrier2::default()
                            .src_stage_mask(stage_mask(barrier.before))
                            .src_access_mask(access_mask(barrier.before))
                            .dst_stage_mask(stage_mask(barrier.after))
                            .dst_access_mask(access_mask(barrier.after))
                            .buffer(buffer.buffer)
                            .offset(0)
                            .size(buffer.size);
                        let dependency = vk::DependencyInfo::default()
                            .buffer_memory_barriers(std::slice::from_ref(&buffer_barrier));
                        self.context
                            .device
                            .cmd_pipeline_barrier2(command_buffer, &dependency);
                    }
                }
                CommandOp::HostWriteBuffer {
                    buffer,
                    offset,
                    data,
                } => {
                    let _zone = trace::Zone::new("vulkan.lowering.host-write");
                    let buffer = objects.buffer(*buffer)?;
                    if !data.is_empty()
                        && *offset % 4 == 0
                        && data.len() % 4 == 0
                        && data.len() <= 65_536
                    {
                        self.context.device.cmd_update_buffer(
                            command_buffer,
                            buffer.buffer,
                            *offset,
                            data,
                        );
                        // `cmd_update_buffer` is a transfer operation, not a
                        // host write.  Describe that actual dependency before
                        // the staging buffer is consumed by a copy-to-image.
                        // Using HOST/HOST_WRITE here leaves the transfer write
                        // unsynchronized on implementations that do not make
                        // same-queue command ordering a memory dependency,
                        // which manifests as black sampled textures.
                        let transfer_write = vk::BufferMemoryBarrier2::default()
                            .src_stage_mask(vk::PipelineStageFlags2::TRANSFER)
                            .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                            .dst_stage_mask(vk::PipelineStageFlags2::TRANSFER)
                            .dst_access_mask(vk::AccessFlags2::TRANSFER_READ)
                            .buffer(buffer.buffer)
                            .offset(*offset)
                            .size(data.len() as u64);
                        self.context.device.cmd_pipeline_barrier2(
                            command_buffer,
                            &vk::DependencyInfo::default()
                                .buffer_memory_barriers(std::slice::from_ref(&transfer_write)),
                        );
                    } else {
                        let memory_offset =
                            buffer.memory_offset.checked_add(*offset).ok_or_else(|| {
                                GalError::backend("Vulkan host-write buffer offset overflow")
                            })?;
                        self.context
                            .write_mapped_memory(buffer.memory, memory_offset, data)?;
                        // Mapped writes happen on the host while the command
                        // buffer is being recorded; they are not transfer
                        // commands. Establish host-write availability before
                        // the semantic TransferDst -> consumer barrier that
                        // follows this operation. Without this dependency,
                        // uniform/storage uploads could remain invisible to a
                        // later shader read on a non-coherent execution path.
                        let host_write =
                            mapped_host_write_dependency(buffer.buffer, *offset, data.len() as u64);
                        self.context.device.cmd_pipeline_barrier2(
                            command_buffer,
                            &vk::DependencyInfo::default()
                                .buffer_memory_barriers(std::slice::from_ref(&host_write)),
                        );
                    }
                }
                CommandOp::HostReadBuffer {
                    buffer,
                    offset,
                    size,
                } => {
                    let _zone = trace::Zone::new("vulkan.lowering.host-read-schedule");
                    let buffer_object = objects.buffer(*buffer)?;
                    state.host_reads.push(HostReadRequest {
                        buffer: *buffer,
                        memory: buffer_object.memory,
                        memory_offset: buffer_object.memory_offset,
                        offset: *offset,
                        size: *size,
                    });
                }
                CommandOp::Present { .. } => {
                    return Err(GalError::unsupported_feature(
                        "presentation commands are lowered by the frame coordinator",
                    ));
                }
            }
        }
        Ok(())
    }

    fn transition_pending_frame_targets_to_present(
        &self,
        command_buffer: vk::CommandBuffer,
        state: &mut EncodingState,
    ) {
        for (target, present) in std::mem::take(&mut state.pending_frame_presents) {
            let old_layout = state
                .frame_target_layouts
                .get(&target)
                .copied()
                .unwrap_or(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
            let to_present = vk::ImageMemoryBarrier2::default()
                .src_stage_mask(frame_layout_stage_mask(old_layout))
                .src_access_mask(frame_layout_access_mask(old_layout))
                // Presentation is an external queue operation; there is no
                // in-queue destination access to synchronize here.
                .dst_stage_mask(vk::PipelineStageFlags2::NONE)
                .old_layout(old_layout)
                .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
                .image(present.image)
                .subresource_range(present.range);
            unsafe {
                self.context.device.cmd_pipeline_barrier2(
                    command_buffer,
                    &vk::DependencyInfo::default()
                        .image_memory_barriers(std::slice::from_ref(&to_present)),
                );
            }
            state
                .frame_target_layouts
                .insert(target, vk::ImageLayout::PRESENT_SRC_KHR);
            trace::message(&format!(
                "gal.frame.target.present-ready backend=vulkan frame={} image={}",
                present.frame_id, present.image_index
            ));
        }
    }

    fn complete_host_reads(&mut self, complete: &InFlightSubmission) {
        let _zone = trace::Zone::new("vulkan.backend.host-readback");
        for request in &complete.host_reads {
            let result = request
                .memory_offset
                .checked_add(request.offset)
                .ok_or_else(|| GalError::backend("Vulkan host-read buffer offset overflow"))
                .and_then(|offset| {
                    self.context
                        .read_mapped_memory(request.memory, offset, request.size)
                });
            match result {
                Ok(bytes) => self.completed_host_reads.push(CompletedHostRead {
                    submission: complete.id,
                    buffer: request.buffer,
                    offset: request.offset,
                    bytes,
                }),
                Err(error) => self.completed_host_reads.push(CompletedHostRead {
                    submission: complete.id,
                    buffer: request.buffer,
                    offset: request.offset,
                    bytes: error.to_string().into_bytes(),
                }),
            }
        }
        self.prune_completed_host_reads(complete.id);
    }

    fn prune_completed_host_reads(&mut self, newest_submission: SubmissionId) {
        let mut bytes = 0usize;
        let mut keep_from = self.completed_host_reads.len();
        for (index, read) in self.completed_host_reads.iter().enumerate().rev() {
            // Keep every read from the newest submission so the caller can
            // correlate all attachments belonging to one presented frame.
            if read.submission != newest_submission && bytes >= MAX_COMPLETED_HOST_READ_BYTES {
                break;
            }
            bytes = bytes.saturating_add(read.bytes.len());
            keep_from = index;
        }
        if keep_from > 0 {
            self.completed_host_reads.drain(..keep_from);
        }
    }

    fn complete_gpu_timestamps(&mut self, complete: &InFlightSubmission) {
        let Some(pool) = self.timestamp_pool else {
            self.apply_gpu_timestamp_result(GpuTimestampResult::default());
            return;
        };
        if !complete.timestamp_set.active {
            self.apply_gpu_timestamp_result(GpuTimestampResult::default());
            return;
        }
        let mut values = [0_u64; GPU_TIMESTAMP_QUERIES_PER_SET as usize];
        let mut ready = [false; GPU_TIMESTAMP_QUERIES_PER_SET as usize];
        for query in 0..GPU_TIMESTAMP_QUERIES_PER_SET {
            let mut value = [0_u64; 1];
            let result = unsafe {
                self.context.device.get_query_pool_results(
                    pool,
                    complete.timestamp_set.base_query + query,
                    &mut value,
                    vk::QueryResultFlags::TYPE_64,
                )
            };
            match result {
                Ok(()) => {
                    values[query as usize] = value[0];
                    ready[query as usize] = true;
                }
                Err(vk::Result::NOT_READY) => {}
                Err(_) => {
                    self.apply_gpu_timestamp_result(GpuTimestampResult::default());
                    return;
                }
            }
        }
        self.apply_gpu_timestamp_result(decode_gpu_timestamp_result(
            &values,
            &ready,
            self.context.timestamp_period,
        ));
    }

    fn apply_gpu_timestamp_result(&mut self, result: GpuTimestampResult) {
        self.metrics.gpu_timestamp_status = result.status;
        self.metrics.gpu_shadow_depth_nanos = result.shadow_depth_nanos;
        self.metrics.gpu_terrain_opaque_nanos = result.terrain_opaque_nanos;
        self.metrics.gpu_terrain_cutout_nanos = result.terrain_cutout_nanos;
        self.metrics.gpu_deferred_lighting_nanos = result.deferred_lighting_nanos;
        self.metrics.gpu_composite0_nanos = result.composite0_nanos;
        self.metrics.gpu_composite1_nanos = result.composite1_nanos;
        self.metrics.gpu_final_output_nanos = result.final_output_nanos;
        self.metrics.gpu_frame_total_nanos = result.frame_total_nanos;
    }
}

fn sanitize_label(label: &str) -> String {
    label
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

impl Drop for SubmissionLowerer {
    fn drop(&mut self) {
        self.wait_idle_and_clear();
        if let Some(pool) = self.timestamp_pool.take() {
            unsafe { self.context.device.destroy_query_pool(pool, None) };
        }
    }
}

fn create_timestamp_pool(context: &Arc<VulkanContext>) -> GalResult<vk::QueryPool> {
    if context.timestamp_valid_bits == 0 || context.timestamp_period <= 0.0 {
        return Err(GalError::backend(
            "Vulkan queue family does not expose timestamp queries",
        ));
    }
    let info = vk::QueryPoolCreateInfo::default()
        .query_type(vk::QueryType::TIMESTAMP)
        .query_count(GPU_TIMESTAMP_SET_COUNT * GPU_TIMESTAMP_QUERIES_PER_SET);
    unsafe { context.device.create_query_pool(&info, None) }.map_err(|error| {
        GalError::backend(format!(
            "failed to create Vulkan timestamp query pool: {error:?}"
        ))
    })
}

fn gpu_timestamps_enabled() -> bool {
    matches!(
        std::env::var("MATTMC_RUST_VULKAN_GPU_TIMESTAMPS")
            .ok()
            .as_deref(),
        Some("1" | "true" | "TRUE" | "yes" | "on")
    )
}

fn timestamp_pass_kind(label: &str) -> Option<TimestampPassKind> {
    let label = label.trim();
    if label.contains("shadow_depth") || label.contains("shadow-pass") {
        Some(TimestampPassKind::ShadowDepth)
    } else if label.contains("terrain_opaque") {
        Some(TimestampPassKind::TerrainOpaque)
    } else if label.contains("terrain_cutout") {
        Some(TimestampPassKind::TerrainCutout)
    } else if label.contains("deferred_lighting") || label.contains("deferred-lighting-pass") {
        Some(TimestampPassKind::DeferredLighting)
    } else if label.contains("composite_0") || label.contains("composite-0-pass") {
        Some(TimestampPassKind::Composite0)
    } else if label.contains("composite_1") || label.contains("composite-1-pass") {
        Some(TimestampPassKind::Composite1)
    } else if label.contains("final_output") || label.contains("final-output-pass") {
        Some(TimestampPassKind::FinalOutput)
    } else {
        None
    }
}

fn timestamp_pipeline_kind(label: &str) -> Option<TimestampPassKind> {
    let label = label.trim();
    if label.contains("shadow_depth") || label.contains("shadow-pipeline") {
        Some(TimestampPassKind::ShadowDepth)
    } else if label.contains("terrain_opaque")
        || (label.contains("world-mesh-gbuffer") && label.contains("-mode1-"))
    {
        Some(TimestampPassKind::TerrainOpaque)
    } else if label.contains("terrain_cutout")
        || (label.contains("world-mesh-gbuffer") && label.contains("-mode2-"))
    {
        Some(TimestampPassKind::TerrainCutout)
    } else if label.contains("deferred_lighting") || label.contains("deferred-lighting.pipeline") {
        Some(TimestampPassKind::DeferredLighting)
    } else if label.contains("composite_0") || label.contains("composite-0.pipeline") {
        Some(TimestampPassKind::Composite0)
    } else if label.contains("composite_1") || label.contains("composite-1.pipeline") {
        Some(TimestampPassKind::Composite1)
    } else if label.contains("final_output") || label.contains("final-output.pipeline") {
        Some(TimestampPassKind::FinalOutput)
    } else {
        None
    }
}

fn decode_gpu_timestamp_result(
    values: &[u64],
    ready: &[bool],
    timestamp_period: f32,
) -> GpuTimestampResult {
    fn delta(values: &[u64], start: GpuTimestampQuery, end: GpuTimestampQuery, period: f32) -> u64 {
        let Some(start) = values.get(start as usize).copied() else {
            return 0;
        };
        let Some(end) = values.get(end as usize).copied() else {
            return 0;
        };
        if end <= start {
            return 0;
        }
        ((end - start) as f64 * f64::from(period)).min(u64::MAX as f64) as u64
    }
    fn pair_ready(ready: &[bool], start: GpuTimestampQuery, end: GpuTimestampQuery) -> bool {
        ready.get(start as usize).copied().unwrap_or(false)
            && ready.get(end as usize).copied().unwrap_or(false)
    }
    fn ready_delta(
        values: &[u64],
        ready: &[bool],
        start: GpuTimestampQuery,
        end: GpuTimestampQuery,
        period: f32,
    ) -> u64 {
        if pair_ready(ready, start, end) {
            delta(values, start, end, period)
        } else {
            0
        }
    }

    let frame_total = ready_delta(
        values,
        ready,
        GpuTimestampQuery::FrameStart,
        GpuTimestampQuery::FrameEnd,
        timestamp_period,
    );
    let result = GpuTimestampResult {
        status: u64::from(frame_total > 0),
        shadow_depth_nanos: ready_delta(
            values,
            ready,
            GpuTimestampQuery::ShadowDepthStart,
            GpuTimestampQuery::ShadowDepthEnd,
            timestamp_period,
        ),
        terrain_opaque_nanos: ready_delta(
            values,
            ready,
            GpuTimestampQuery::TerrainOpaqueStart,
            GpuTimestampQuery::TerrainOpaqueEnd,
            timestamp_period,
        ),
        terrain_cutout_nanos: ready_delta(
            values,
            ready,
            GpuTimestampQuery::TerrainCutoutStart,
            GpuTimestampQuery::TerrainCutoutEnd,
            timestamp_period,
        ),
        deferred_lighting_nanos: ready_delta(
            values,
            ready,
            GpuTimestampQuery::DeferredLightingStart,
            GpuTimestampQuery::DeferredLightingEnd,
            timestamp_period,
        ),
        composite0_nanos: ready_delta(
            values,
            ready,
            GpuTimestampQuery::Composite0Start,
            GpuTimestampQuery::Composite0End,
            timestamp_period,
        ),
        composite1_nanos: ready_delta(
            values,
            ready,
            GpuTimestampQuery::Composite1Start,
            GpuTimestampQuery::Composite1End,
            timestamp_period,
        ),
        final_output_nanos: ready_delta(
            values,
            ready,
            GpuTimestampQuery::FinalOutputStart,
            GpuTimestampQuery::FinalOutputEnd,
            timestamp_period,
        ),
        frame_total_nanos: frame_total,
    };
    result
}

#[cfg(test)]
mod timestamp_tests {
    use super::*;

    #[test]
    fn retirement_wait_failure_preserves_in_flight_ownership_and_order() {
        let mut pending = VecDeque::from([(SubmissionId(3), vec![7, 8]), (SubmissionId(4), vec![9])]);
        let before = pending.clone();
        assert!(wait_then_pop_front(&mut pending, |entry| {
            assert_eq!(entry.0, SubmissionId(3));
            Err(GalError::backend("injected timeline timeout"))
        }).is_err());
        assert_eq!(pending, before);
        assert_eq!(wait_then_pop_front(&mut pending, |entry| {
            assert_eq!(entry.0, SubmissionId(3));
            Ok(())
        }).unwrap(), Some(before[0].clone()));
        assert_eq!(pending, VecDeque::from([before[1].clone()]));
    }

    #[test]
    fn buffer_image_copy_preserves_texel_row_length_for_r8_and_depth_extent() {
        let copy = buffer_image_copy(
            &BufferImageCopyRegion {
                buffer: crate::render::vulkanic::handles::Handle::new(
                    crate::render::vulkanic::handles::HandleKind::Buffer,
                    1,
                    1,
                )
                .unwrap(),
                buffer_offset: 12,
                bytes_per_row: 7,
                rows_per_image: 5,
                texture: crate::render::vulkanic::handles::Handle::new(
                    crate::render::vulkanic::handles::HandleKind::Texture,
                    1,
                    1,
                )
                .unwrap(),
                texture_mip: 0,
                texture_layer: 0,
                texture_origin: crate::render::vulkanic::commands::TextureOrigin3d {
                    x: 1,
                    y: 2,
                    z: 3,
                },
                extent: crate::render::vulkanic::resources::Extent3d {
                    width: 7,
                    height: 4,
                    depth: 2,
                },
            },
            vk::ImageAspectFlags::COLOR,
            1,
        );
        assert_eq!(7, copy.buffer_row_length);
        assert_eq!(5, copy.buffer_image_height);
        assert_eq!(3, copy.image_offset.z);
        assert_eq!(2, copy.image_extent.depth);
    }

    #[test]
    fn row_reversal_checks_format_support_and_only_reverses_destination_y() {
        use crate::render::vulkanic::commands::TextureOrigin3d;
        use crate::render::vulkanic::resources::Extent3d;
        let src = vk::FormatFeatureFlags::BLIT_SRC;
        let dst = vk::FormatFeatureFlags::BLIT_DST;
        validate_row_reversal_format_features(src, dst).unwrap();
        assert!(validate_row_reversal_format_features(vk::FormatFeatureFlags::empty(), dst).is_err());
        assert!(validate_row_reversal_format_features(src, vk::FormatFeatureFlags::empty()).is_err());
        let mut region = TextureImageCopyRegion {
            row_order: TextureRowOrder::Reverse,
            src_texture: Handle::new(HandleKind::Texture, 1, 1).unwrap(),
            src_mip: 1, src_layer: 2, src_origin: TextureOrigin3d { x: 3, y: 5, z: 0 },
            dst_texture: Handle::new(HandleKind::Texture, 2, 1).unwrap(),
            dst_mip: 2, dst_layer: 3, dst_origin: TextureOrigin3d { x: 7, y: 11, z: 0 },
            extent: Extent3d { width: 13, height: 17, depth: 1 },
        };
        let blit = texture_row_reversal_blit(&region, vk::ImageAspectFlags::COLOR, vk::ImageAspectFlags::COLOR).unwrap();
        let coordinates = |offsets: [vk::Offset3D; 2]| offsets.map(|p| (p.x, p.y, p.z));
        assert_eq!([(3, 5, 0), (16, 22, 1)], coordinates(blit.src_offsets));
        assert_eq!([(7, 28, 0), (20, 11, 1)], coordinates(blit.dst_offsets));
        assert_eq!((1, 2), (blit.src_subresource.mip_level, blit.src_subresource.base_array_layer));
        assert_eq!((2, 3), (blit.dst_subresource.mip_level, blit.dst_subresource.base_array_layer));
        let combined = vk::ImageAspectFlags::DEPTH | vk::ImageAspectFlags::STENCIL;
        assert!(texture_row_reversal_blit(&region, combined, combined).is_err());
        region.dst_origin.y = i32::MAX as u32;
        assert!(texture_row_reversal_blit(&region, vk::ImageAspectFlags::COLOR, vk::ImageAspectFlags::COLOR).is_err());
    }

    #[test]
    fn texture_image_copy_preserves_independent_depth_subresources() {
        use crate::render::vulkanic::commands::{TextureImageCopyRegion, TextureOrigin3d};
        use crate::render::vulkanic::handles::{Handle, HandleKind};
        use crate::render::vulkanic::resources::Extent3d;

        let copy = texture_image_copy(
            &TextureImageCopyRegion {
                row_order: crate::render::vulkanic::commands::TextureRowOrder::Preserve,
                src_texture: Handle::new(HandleKind::Texture, 1, 1).unwrap(),
                src_mip: 1,
                src_layer: 0,
                src_origin: TextureOrigin3d { x: 3, y: 5, z: 0 },
                dst_texture: Handle::new(HandleKind::Texture, 2, 1).unwrap(),
                dst_mip: 2,
                dst_layer: 0,
                dst_origin: TextureOrigin3d { x: 7, y: 11, z: 0 },
                extent: Extent3d {
                    width: 13,
                    height: 17,
                    depth: 1,
                },
            },
            vk::ImageAspectFlags::DEPTH,
            vk::ImageAspectFlags::DEPTH,
        );
        assert_eq!(1, copy.src_subresource.mip_level);
        assert_eq!(2, copy.dst_subresource.mip_level);
        assert_eq!(3, copy.src_offset.x);
        assert_eq!(11, copy.dst_offset.y);
        assert_eq!(17, copy.extent.height);
        assert!(copy
            .src_subresource
            .aspect_mask
            .contains(vk::ImageAspectFlags::DEPTH));
    }

    #[test]
    fn depth_stencil_layouts_match_combined_aspect_ranges() {
        let combined = vk::ImageAspectFlags::DEPTH | vk::ImageAspectFlags::STENCIL;
        assert!(
            image_layout_for_aspect(TextureUsageState::DepthStencilAttachment, combined)
                == vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL
        );
        assert!(
            image_layout_for_aspect(TextureUsageState::ShaderRead, combined)
                == vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL
        );
        assert!(
            image_layout_for_aspect(
                TextureUsageState::DepthStencilAttachment,
                vk::ImageAspectFlags::DEPTH
            ) == vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL
        );
        assert!(
            image_layout_for_aspect(TextureUsageState::ShaderRead, vk::ImageAspectFlags::DEPTH)
                == vk::ImageLayout::DEPTH_READ_ONLY_OPTIMAL
        );
    }

    #[test]
    fn timestamp_pass_labels_classify_shader_graph_passes() {
        assert_eq!(
            Some(TimestampPassKind::ShadowDepth),
            timestamp_pass_kind("vulkanic:pass/shadow_depth")
        );
        assert_eq!(
            Some(TimestampPassKind::TerrainOpaque),
            timestamp_pass_kind("vulkanic:pass/terrain_opaque")
        );
        assert_eq!(
            Some(TimestampPassKind::FinalOutput),
            timestamp_pass_kind("vulkanic:pass/final_output")
        );
        assert_eq!(None, timestamp_pass_kind("minecraft.world.clear"));
    }

    #[test]
    fn timestamp_labels_classify_actual_runtime_resource_names() {
        assert_eq!(
            Some(TimestampPassKind::ShadowDepth),
            timestamp_pass_kind("world-gbuffer.shadow-pass")
        );
        assert_eq!(
            Some(TimestampPassKind::DeferredLighting),
            timestamp_pass_kind("world-gbuffer.deferred-lighting-pass")
        );
        assert_eq!(
            Some(TimestampPassKind::Composite0),
            timestamp_pipeline_kind("world-gbuffer.composite-0.pipeline")
        );
        assert_eq!(
            Some(TimestampPassKind::TerrainOpaque),
            timestamp_pipeline_kind("world-mesh-gbuffer-stratum4-sand-gen7-section0-texture3-mode1-depth2-cull1.pipeline")
        );
        assert_eq!(
            Some(TimestampPassKind::TerrainCutout),
            timestamp_pipeline_kind("world-mesh-gbuffer-stratum4-leaves-gen7-section0-texture9-mode2-depth2-cull1.pipeline")
        );
    }

    #[test]
    fn gpu_timestamp_decoder_reports_nanos_without_waiting_policy() {
        let mut values = [0_u64; GPU_TIMESTAMP_QUERIES_PER_SET as usize];
        let mut ready = [false; GPU_TIMESTAMP_QUERIES_PER_SET as usize];
        values[GpuTimestampQuery::FrameStart as usize] = 10;
        values[GpuTimestampQuery::ShadowDepthStart as usize] = 12;
        values[GpuTimestampQuery::ShadowDepthEnd as usize] = 18;
        values[GpuTimestampQuery::FrameEnd as usize] = 30;
        ready[GpuTimestampQuery::FrameStart as usize] = true;
        ready[GpuTimestampQuery::ShadowDepthStart as usize] = true;
        ready[GpuTimestampQuery::ShadowDepthEnd as usize] = true;
        ready[GpuTimestampQuery::FrameEnd as usize] = true;
        let result = decode_gpu_timestamp_result(&values, &ready, 2.0);
        assert_eq!(1, result.status);
        assert_eq!(12, result.shadow_depth_nanos);
        assert_eq!(40, result.frame_total_nanos);
        assert_eq!(0, result.terrain_opaque_nanos);
    }

    #[test]
    fn gpu_timestamp_decoder_reports_unavailable_for_unwritten_first_frame() {
        let values = [0_u64; GPU_TIMESTAMP_QUERIES_PER_SET as usize];
        let ready = [false; GPU_TIMESTAMP_QUERIES_PER_SET as usize];
        let result = decode_gpu_timestamp_result(&values, &ready, 1.0);
        assert_eq!(0, result.status);
        assert_eq!(0, result.frame_total_nanos);
    }

    #[test]
    fn gpu_timestamp_decoder_keeps_written_pass_when_other_queries_are_unavailable() {
        let mut values = [0_u64; GPU_TIMESTAMP_QUERIES_PER_SET as usize];
        let mut ready = [false; GPU_TIMESTAMP_QUERIES_PER_SET as usize];
        for query in [
            GpuTimestampQuery::FrameStart,
            GpuTimestampQuery::FrameEnd,
            GpuTimestampQuery::TerrainOpaqueStart,
            GpuTimestampQuery::TerrainOpaqueEnd,
        ] {
            ready[query as usize] = true;
        }
        values[GpuTimestampQuery::FrameStart as usize] = 100;
        values[GpuTimestampQuery::TerrainOpaqueStart as usize] = 110;
        values[GpuTimestampQuery::TerrainOpaqueEnd as usize] = 160;
        values[GpuTimestampQuery::FrameEnd as usize] = 200;
        let result = decode_gpu_timestamp_result(&values, &ready, 1.5);
        assert_eq!(1, result.status);
        assert_eq!(150, result.frame_total_nanos);
        assert_eq!(75, result.terrain_opaque_nanos);
        assert_eq!(0, result.terrain_cutout_nanos);
    }

    #[test]
    fn gpu_timestamp_decoder_requires_both_pass_endpoints() {
        let mut values = [0_u64; GPU_TIMESTAMP_QUERIES_PER_SET as usize];
        let mut ready = [false; GPU_TIMESTAMP_QUERIES_PER_SET as usize];
        ready[GpuTimestampQuery::FrameStart as usize] = true;
        ready[GpuTimestampQuery::FrameEnd as usize] = true;
        ready[GpuTimestampQuery::TerrainCutoutStart as usize] = true;
        values[GpuTimestampQuery::FrameStart as usize] = 3;
        values[GpuTimestampQuery::FrameEnd as usize] = 9;
        values[GpuTimestampQuery::TerrainCutoutStart as usize] = 4;
        values[GpuTimestampQuery::TerrainCutoutEnd as usize] = 8;
        let result = decode_gpu_timestamp_result(&values, &ready, 10.0);
        assert_eq!(1, result.status);
        assert_eq!(60, result.frame_total_nanos);
        assert_eq!(0, result.terrain_cutout_nanos);
    }

    #[test]
    fn frame_target_transfer_layouts_use_matching_pipeline_dependencies() {
        assert!(
            vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT
                == frame_layout_stage_mask(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
        );
        assert!(
            (vk::AccessFlags2::COLOR_ATTACHMENT_READ | vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
                == frame_layout_access_mask(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
        );
        assert!(
            vk::PipelineStageFlags2::COPY
                == frame_layout_stage_mask(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
        );
        assert!(
            vk::AccessFlags2::TRANSFER_READ
                == frame_layout_access_mask(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
        );
        assert!(
            vk::PipelineStageFlags2::NONE
                == frame_layout_stage_mask(vk::ImageLayout::PRESENT_SRC_KHR)
        );
        assert!(
            vk::AccessFlags2::empty() == frame_layout_access_mask(vk::ImageLayout::PRESENT_SRC_KHR)
        );
    }

    #[test]
    fn shader_read_barrier_covers_graphics_uniform_loads() {
        let stages = stage_mask(TextureUsageState::ShaderRead);
        assert!(stages.contains(vk::PipelineStageFlags2::VERTEX_SHADER));
        assert!(stages.contains(vk::PipelineStageFlags2::FRAGMENT_SHADER));
        let access = access_mask(TextureUsageState::ShaderRead);
        assert!(access.contains(vk::AccessFlags2::SHADER_SAMPLED_READ));
        assert!(access.contains(vk::AccessFlags2::UNIFORM_READ));
    }

    #[test]
    fn image_shader_read_excludes_uniform_buffer_access_without_weakening_buffer_barriers() {
        assert_eq!(image_access_mask(TextureUsageState::ShaderRead).as_raw(), vk::AccessFlags2::SHADER_SAMPLED_READ.as_raw());
        assert!(access_mask(TextureUsageState::ShaderRead).contains(vk::AccessFlags2::UNIFORM_READ));
        assert_eq!(image_access_mask(TextureUsageState::TransferDst).as_raw(), vk::AccessFlags2::TRANSFER_WRITE.as_raw());
        assert_eq!(image_access_mask(TextureUsageState::ShaderStorageRead).as_raw(), vk::AccessFlags2::SHADER_STORAGE_READ.as_raw());
    }

    #[test]
    fn mapped_host_writes_publish_to_the_transfer_consumer_stage() {
        let barrier = mapped_host_write_dependency(vk::Buffer::null(), 12, 48);
        assert!(vk::PipelineStageFlags2::HOST == barrier.src_stage_mask);
        assert!(vk::AccessFlags2::HOST_WRITE == barrier.src_access_mask);
        assert!(vk::PipelineStageFlags2::TRANSFER == barrier.dst_stage_mask);
        assert!(vk::AccessFlags2::TRANSFER_WRITE == barrier.dst_access_mask);
        assert_eq!(12, barrier.offset);
        assert_eq!(48, barrier.size);
    }
}

#[derive(Default)]
struct EncodingState {
    in_pass: bool,
    graphics_pipeline: Option<crate::render::vulkanic::handles::Handle>,
    compute_pipeline: Option<crate::render::vulkanic::handles::Handle>,
    pipeline_layout: Option<crate::render::vulkanic::handles::Handle>,
    frame_target_touched: bool,
    frame_present: Option<FramePresentTransition>,
    frame_target_layouts: BTreeMap<Handle, vk::ImageLayout>,
    transfer_dst_textures: BTreeSet<Handle>,
    pending_frame_presents: BTreeMap<Handle, FramePresentTransition>,
    host_reads: Vec<HostReadRequest>,
    timestamp_set: GpuTimestampSet,
    current_timestamp_pass: Option<TimestampPassKind>,
}

struct FramePresentTransition {
    target: Handle,
    image: vk::Image,
    image_index: u32,
    frame_id: u64,
    range: vk::ImageSubresourceRange,
}

struct EncodedSubmission {
    command_buffers: Vec<vk::CommandBuffer>,
    host_reads: Vec<HostReadRequest>,
    timestamp_set: GpuTimestampSet,
}

struct InFlightSubmission {
    id: SubmissionId,
    command_buffers: Vec<vk::CommandBuffer>,
    host_reads: Vec<HostReadRequest>,
    timestamp_set: GpuTimestampSet,
}

#[derive(Clone, Debug)]
struct HostReadRequest {
    buffer: Handle,
    memory: vk::DeviceMemory,
    memory_offset: u64,
    offset: u64,
    size: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::render::vulkanic) struct CompletedHostRead {
    pub(super) submission: SubmissionId,
    pub(super) buffer: Handle,
    pub(super) offset: u64,
    pub(super) bytes: Vec<u8>,
}

fn load_op(op: AttachmentLoadOp) -> vk::AttachmentLoadOp {
    match op {
        AttachmentLoadOp::Load => vk::AttachmentLoadOp::LOAD,
        AttachmentLoadOp::Clear => vk::AttachmentLoadOp::CLEAR,
        AttachmentLoadOp::DontCare => vk::AttachmentLoadOp::DONT_CARE,
    }
}

fn store_op(op: AttachmentStoreOp) -> vk::AttachmentStoreOp {
    match op {
        AttachmentStoreOp::Store => vk::AttachmentStoreOp::STORE,
        AttachmentStoreOp::DontCare => vk::AttachmentStoreOp::DONT_CARE,
    }
}

pub(super) fn image_layout(state: TextureUsageState) -> vk::ImageLayout {
    match state {
        TextureUsageState::Undefined => vk::ImageLayout::UNDEFINED,
        TextureUsageState::ShaderRead => vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
        TextureUsageState::ShaderWrite => vk::ImageLayout::GENERAL,
        TextureUsageState::ColorAttachment => vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
        TextureUsageState::DepthStencilAttachment => vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL,
        TextureUsageState::TransferSrc => vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
        TextureUsageState::TransferDst => vk::ImageLayout::TRANSFER_DST_OPTIMAL,
        TextureUsageState::Present => vk::ImageLayout::PRESENT_SRC_KHR,
        TextureUsageState::IndexRead => vk::ImageLayout::UNDEFINED,
        TextureUsageState::ShaderStorageRead => vk::ImageLayout::GENERAL,
    }
}

/// Depth/stencil images need the combined Vulkan layouts whenever their
/// stencil aspect is part of the subresource range.  Using the depth-only
/// layouts with a D24S8 image is rejected by validation even when the GAL
/// semantic usage is simply `DepthStencilAttachment`.
pub(super) fn image_layout_for_aspect(
    state: TextureUsageState,
    aspect: vk::ImageAspectFlags,
) -> vk::ImageLayout {
    let has_stencil = aspect.contains(vk::ImageAspectFlags::STENCIL);
    match (state, has_stencil) {
        (TextureUsageState::DepthStencilAttachment, true) => {
            vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL
        }
        (TextureUsageState::ShaderRead, true) => vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL,
        (TextureUsageState::ShaderRead, false) if aspect.contains(vk::ImageAspectFlags::DEPTH) => {
            vk::ImageLayout::DEPTH_READ_ONLY_OPTIMAL
        }
        _ => image_layout(state),
    }
}

fn frame_layout_stage_mask(layout: vk::ImageLayout) -> vk::PipelineStageFlags2 {
    match layout {
        // PRESENT_SRC_KHR is owned by the presentation engine rather than a
        // shader/transfer stage in this queue.  NONE is the explicit
        // synchronization2 representation for that external dependency.
        vk::ImageLayout::PRESENT_SRC_KHR => vk::PipelineStageFlags2::NONE,
        vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL => {
            vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT
        }
        vk::ImageLayout::TRANSFER_SRC_OPTIMAL => vk::PipelineStageFlags2::COPY,
        _ => vk::PipelineStageFlags2::TOP_OF_PIPE,
    }
}

fn frame_layout_access_mask(layout: vk::ImageLayout) -> vk::AccessFlags2 {
    match layout {
        vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL => {
            vk::AccessFlags2::COLOR_ATTACHMENT_READ | vk::AccessFlags2::COLOR_ATTACHMENT_WRITE
        }
        vk::ImageLayout::TRANSFER_SRC_OPTIMAL => vk::AccessFlags2::TRANSFER_READ,
        _ => vk::AccessFlags2::empty(),
    }
}

fn mapped_host_write_dependency(
    buffer: vk::Buffer,
    offset: u64,
    size: u64,
) -> vk::BufferMemoryBarrier2<'static> {
    vk::BufferMemoryBarrier2::default()
        .src_stage_mask(vk::PipelineStageFlags2::HOST)
        .src_access_mask(vk::AccessFlags2::HOST_WRITE)
        .dst_stage_mask(vk::PipelineStageFlags2::TRANSFER)
        .dst_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
        .buffer(buffer)
        .offset(offset)
        .size(size)
}

pub(super) fn stage_mask(state: TextureUsageState) -> vk::PipelineStageFlags2 {
    match state {
        TextureUsageState::Undefined => vk::PipelineStageFlags2::TOP_OF_PIPE,
        TextureUsageState::ShaderRead
        | TextureUsageState::ShaderWrite
        | TextureUsageState::ShaderStorageRead => {
            vk::PipelineStageFlags2::COMPUTE_SHADER
                | vk::PipelineStageFlags2::VERTEX_SHADER
                | vk::PipelineStageFlags2::FRAGMENT_SHADER
        }
        TextureUsageState::ColorAttachment => vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
        TextureUsageState::DepthStencilAttachment => {
            vk::PipelineStageFlags2::EARLY_FRAGMENT_TESTS
                | vk::PipelineStageFlags2::LATE_FRAGMENT_TESTS
        }
        TextureUsageState::TransferSrc | TextureUsageState::TransferDst => {
            vk::PipelineStageFlags2::TRANSFER
        }
        TextureUsageState::IndexRead => vk::PipelineStageFlags2::INDEX_INPUT,
        TextureUsageState::Present => vk::PipelineStageFlags2::NONE,
    }
}

fn image_access_mask(state: TextureUsageState) -> vk::AccessFlags2 {
    // GAL's resource type is explicit. A sampled image can never be a uniform
    // buffer, even though both resources currently share ShaderRead state.
    if state == TextureUsageState::ShaderRead {
        vk::AccessFlags2::SHADER_SAMPLED_READ
    } else {
        access_mask(state)
    }
}

pub(super) fn access_mask(state: TextureUsageState) -> vk::AccessFlags2 {
    match state {
        TextureUsageState::Undefined | TextureUsageState::Present => vk::AccessFlags2::empty(),
        // `ShaderRead` is the backend-neutral read state used for both sampled
        // textures and descriptor-backed uniform buffers.  A barrier that
        // names only sampled reads does not make a preceding update visible to
        // a graphics UBO load, which can leave a draw observing stale frame
        // parameters. Image barriers use image_access_mask instead, because
        // their explicit resource type excludes uniform-buffer accesses.
        TextureUsageState::ShaderRead => {
            vk::AccessFlags2::SHADER_SAMPLED_READ | vk::AccessFlags2::UNIFORM_READ
        }
        TextureUsageState::ShaderWrite => vk::AccessFlags2::SHADER_STORAGE_WRITE,
        TextureUsageState::ShaderStorageRead => vk::AccessFlags2::SHADER_STORAGE_READ,
        TextureUsageState::ColorAttachment => {
            vk::AccessFlags2::COLOR_ATTACHMENT_READ | vk::AccessFlags2::COLOR_ATTACHMENT_WRITE
        }
        TextureUsageState::DepthStencilAttachment => {
            vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_READ
                | vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_WRITE
        }
        TextureUsageState::TransferSrc => vk::AccessFlags2::TRANSFER_READ,
        TextureUsageState::TransferDst => vk::AccessFlags2::TRANSFER_WRITE,
        TextureUsageState::IndexRead => vk::AccessFlags2::INDEX_READ,
    }
}

fn vk_index_type(index_type: crate::render::vulkanic::resources::IndexType) -> vk::IndexType {
    match index_type {
        crate::render::vulkanic::resources::IndexType::U16 => vk::IndexType::UINT16,
        crate::render::vulkanic::resources::IndexType::U32 => vk::IndexType::UINT32,
    }
}

fn buffer_image_copy(
    region: &BufferImageCopyRegion,
    aspect_mask: vk::ImageAspectFlags,
    bytes_per_texel: u32,
) -> vk::BufferImageCopy {
    vk::BufferImageCopy::default()
        .buffer_offset(region.buffer_offset)
        .buffer_row_length(region.bytes_per_row / bytes_per_texel)
        .buffer_image_height(region.rows_per_image)
        .image_subresource(vk::ImageSubresourceLayers {
            aspect_mask,
            mip_level: region.texture_mip,
            base_array_layer: region.texture_layer,
            layer_count: 1,
        })
        .image_offset(vk::Offset3D {
            x: region.texture_origin.x as i32,
            y: region.texture_origin.y as i32,
            z: region.texture_origin.z as i32,
        })
        .image_extent(vk::Extent3D {
            width: region.extent.width,
            height: region.extent.height,
            depth: region.extent.depth,
        })
}

fn validate_row_reversal_format_features(
    source: vk::FormatFeatureFlags,
    destination: vk::FormatFeatureFlags,
) -> GalResult<()> {
    if !source.contains(vk::FormatFeatureFlags::BLIT_SRC)
        || !destination.contains(vk::FormatFeatureFlags::BLIT_DST)
    {
        return Err(GalError::unsupported_feature(
            "texture row reversal requires Vulkan format blit source/destination support",
        ));
    }
    Ok(())
}

fn texture_row_reversal_blit(
    region: &TextureImageCopyRegion,
    src_aspect: vk::ImageAspectFlags,
    dst_aspect: vk::ImageAspectFlags,
) -> GalResult<vk::ImageBlit> {
    // A blit region addresses exactly one aspect; combined depth/stencil needs
    // separate regions and is deliberately not admitted here.
    if src_aspect != dst_aspect || src_aspect.as_raw().count_ones() != 1 {
        return Err(GalError::unsupported_feature(
            "texture row reversal requires one matching image aspect",
        ));
    }
    let offset = |origin: crate::render::vulkanic::commands::TextureOrigin3d,
                  end: bool| -> GalResult<vk::Offset3D> {
        let component = |start: u32, size: u32| -> GalResult<i32> {
            let value = start.checked_add(if end { size } else { 0 })
                .ok_or_else(|| GalError::backend("row reversal offset overflows"))?;
            i32::try_from(value).map_err(|_| GalError::backend("row reversal offset exceeds i32"))
        };
        Ok(vk::Offset3D {
            x: component(origin.x, region.extent.width)?,
            y: component(origin.y, region.extent.height)?,
            z: component(origin.z, region.extent.depth)?,
        })
    };
    let src_offsets = [offset(region.src_origin, false)?, offset(region.src_origin, true)?];
    let mut dst_offsets = [offset(region.dst_origin, false)?, offset(region.dst_origin, true)?];
    let y = dst_offsets[0].y;
    dst_offsets[0].y = dst_offsets[1].y;
    dst_offsets[1].y = y;
    Ok(vk::ImageBlit::default()
        .src_subresource(vk::ImageSubresourceLayers {
            aspect_mask: src_aspect, mip_level: region.src_mip,
            base_array_layer: region.src_layer, layer_count: 1,
        })
        .src_offsets(src_offsets)
        .dst_subresource(vk::ImageSubresourceLayers {
            aspect_mask: dst_aspect, mip_level: region.dst_mip,
            base_array_layer: region.dst_layer, layer_count: 1,
        })
        .dst_offsets(dst_offsets))
}

fn texture_image_copy(
    region: &TextureImageCopyRegion,
    src_aspect_mask: vk::ImageAspectFlags,
    dst_aspect_mask: vk::ImageAspectFlags,
) -> vk::ImageCopy {
    vk::ImageCopy::default()
        .src_subresource(vk::ImageSubresourceLayers {
            aspect_mask: src_aspect_mask,
            mip_level: region.src_mip,
            base_array_layer: region.src_layer,
            layer_count: 1,
        })
        .src_offset(vk::Offset3D {
            x: region.src_origin.x as i32,
            y: region.src_origin.y as i32,
            z: region.src_origin.z as i32,
        })
        .dst_subresource(vk::ImageSubresourceLayers {
            aspect_mask: dst_aspect_mask,
            mip_level: region.dst_mip,
            base_array_layer: region.dst_layer,
            layer_count: 1,
        })
        .dst_offset(vk::Offset3D {
            x: region.dst_origin.x as i32,
            y: region.dst_origin.y as i32,
            z: region.dst_origin.z as i32,
        })
        .extent(vk::Extent3D {
            width: region.extent.width,
            height: region.extent.height,
            depth: region.extent.depth,
        })
}

/// A failed wait must retain all submission-owned commands, reads and queries.
fn wait_then_pop_front<T>(queue: &mut VecDeque<T>, wait: impl FnOnce(&T) -> GalResult<()>) -> GalResult<Option<T>> {
    if let Some(front) = queue.front() {
        wait(front)?;
    }
    Ok(queue.pop_front())
}

fn wait_timeline(context: &VulkanContext, id: SubmissionId) -> GalResult<()> {
    let semaphores = [context.timeline];
    let values = [id.0];
    let wait_info = vk::SemaphoreWaitInfo::default()
        .semaphores(&semaphores)
        .values(&values);
    unsafe { context.device.wait_semaphores(&wait_info, 30_000_000_000) }
        .map_err(|error| GalError::backend(format!("Vulkan timeline wait failed: {error:?}")))
}

fn stdout_trace(message: &str) {
    if std::env::var_os("MATTMC_TRACE_SUBMISSIONS").is_some() {
        println!("{message}");
    }
}

fn command_op_kind(op: &CommandOp) -> &'static str {
    match op {
        CommandOp::BeginPass { .. } => "BeginPass",
        CommandOp::BindGraphicsPipeline(_) => "BindGraphicsPipeline",
        CommandOp::BindComputePipeline(_) => "BindComputePipeline",
        CommandOp::BindResourceSet { .. } => "BindResourceSet",
        CommandOp::SetVertexBuffer { .. } => "SetVertexBuffer",
        CommandOp::SetIndexBuffer { .. } => "SetIndexBuffer",
        CommandOp::Draw { .. } => "Draw",
        CommandOp::DrawIndexed { .. } => "DrawIndexed",
        CommandOp::DrawIndirect { .. } => "DrawIndirect",
        CommandOp::Dispatch { .. } => "Dispatch",
        CommandOp::DispatchIndirect { .. } => "DispatchIndirect",
        CommandOp::CopyBuffer { .. } => "CopyBuffer",
        CommandOp::CopyBufferToTexture(_) => "CopyBufferToTexture",
        CommandOp::CopyTextureToBuffer(_) => "CopyTextureToBuffer",
        CommandOp::CopyTexture(_) => "CopyTexture",
        CommandOp::CopyFrameTargetToTexture { .. } => "CopyFrameTargetToTexture",
        CommandOp::CopyTextureToFrameTarget { .. } => "CopyTextureToFrameTarget",
        CommandOp::GenerateMipmaps { .. } => "GenerateMipmaps",
        CommandOp::HostWriteBuffer { .. } => "HostWriteBuffer",
        CommandOp::HostReadBuffer { .. } => "HostReadBuffer",
        CommandOp::Present { .. } => "Present",
        CommandOp::Barrier(_) => "Barrier",
        CommandOp::EndPass => "EndPass",
    }
}
