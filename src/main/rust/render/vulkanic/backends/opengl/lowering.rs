use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

use glow::HasContext;

use super::resources::{
    opengl_resource_binding_point, texture_format, texture_target, topology, OpenGlObjects,
    ResourceSetObject,
};
use super::trace;
use crate::render::vulkanic::commands::{
    AttachmentLoadOp, BufferImageCopyRegion, CommandOp, ResourceBarrier, TextureImageCopyRegion,
    TextureUsageState, ValidatedSubmissionBatch,
};
use crate::render::vulkanic::error::{GalError, GalResult};
use crate::render::vulkanic::handles::Handle;
use crate::render::vulkanic::resources::{
    BlendMode, CompareOp, CullMode, FrontFace, ResourceBindingKind, TextureDimension, TextureFormat,
};
use crate::render::vulkanic::sync::SubmissionId;

static GL_DRAW_TRACE_LIMIT: OnceLock<usize> = OnceLock::new();
static GL_DRAW_TRACE_COUNT: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::render::vulkanic) struct CompletedHostRead {
    pub(in crate::render::vulkanic) submission: SubmissionId,
    pub(in crate::render::vulkanic) buffer: Handle,
    pub(in crate::render::vulkanic) offset: u64,
    pub(in crate::render::vulkanic) bytes: Vec<u8>,
}

pub(super) struct OpenGlLowerer {
    gl: Rc<glow::Context>,
    provoking_vertex: super::context::ProvokingVertexFn,
    clip_control: super::context::ClipControlFn,
    pending: VecDeque<ValidatedSubmissionBatch>,
    submitted: VecDeque<PendingFence>,
    completed: SubmissionId,
    completed_host_reads: Vec<CompletedHostRead>,
    gl_errors: Vec<String>,
    cache: StateCache,
    sync_stats: OpenGlSyncStats,
}

impl OpenGlLowerer {
    pub(super) fn new(gl: Rc<glow::Context>, provoking_vertex: super::context::ProvokingVertexFn, clip_control: super::context::ClipControlFn) -> Self {
        Self {
            gl,
            provoking_vertex,
            clip_control,
            pending: VecDeque::new(),
            submitted: VecDeque::new(),
            completed: SubmissionId(0),
            completed_host_reads: Vec::new(),
            gl_errors: Vec::new(),
            cache: StateCache::default(),
            sync_stats: OpenGlSyncStats::default(),
        }
    }

    pub(super) fn encode(&mut self, batch: &ValidatedSubmissionBatch) -> GalResult<()> {
        let _zone = trace::Zone::new("opengl.lowering.own-validated-batch");
        self.pending.push_back(batch.clone());
        Ok(())
    }

    pub(super) fn submit(
        &mut self,
        id: SubmissionId,
        objects: &mut OpenGlObjects,
    ) -> GalResult<()> {
        let _zone = trace::Zone::new("opengl.lowering.execute-submission");
        let Some(batch) = self.pending.pop_front() else {
            return Err(GalError::backend(
                "OpenGL submit called without encoded commands",
            ));
        };
        self.sync_stats.command_batches = self.sync_stats.command_batches.saturating_add(1);
        self.sync_stats.command_lists = self
            .sync_stats
            .command_lists
            .saturating_add(batch.command_lists.len() as u64);
        self.sync_stats.command_ops = self.sync_stats.command_ops.saturating_add(
            batch
                .command_lists
                .iter()
                .map(|list| list.operations.len() as u64)
                .sum::<u64>(),
        );
        let mut state = ExecutionState::default();
        for list in &batch.command_lists {
            for op in &list.operations {
                self.execute_op(id, objects, &mut state, op)?;
                self.check_errors(&format!("command in {}", list.label))?;
            }
        }
        unsafe {
            let _zone = trace::Zone::new("opengl.backend.fence-insert");
            self.record_gl_call();
            let fence = self
                .gl
                .fence_sync(glow::SYNC_GPU_COMMANDS_COMPLETE, 0)
                .map_err(|error| {
                    GalError::backend(format!("failed to insert OpenGL fence: {error}"))
                })?;
            self.sync_stats.fences_inserted += 1;
            self.submitted.push_back(PendingFence {
                submission: id,
                fence,
            });
        }
        self.poll_fences(false);
        Ok(())
    }

    pub(super) fn reset_state_cache(&mut self) {
        self.cache = StateCache::default();
    }

    pub(super) fn retire(&mut self, completed: SubmissionId) -> GalResult<()> {
        let _zone = trace::Zone::new("opengl.backend.retirement");
        if completed > self.completed {
            self.wait_through(completed)?;
        }
        Ok(())
    }

    pub(super) fn poll_completed_submission(&mut self) -> SubmissionId {
        self.poll_fences(false);
        self.completed
    }

    pub(super) fn delete_all_fences(&mut self) {
        unsafe {
            while let Some(pending) = self.submitted.pop_front() {
                self.record_gl_call();
                self.gl.delete_sync(pending.fence);
                self.sync_stats.fences_deleted += 1;
            }
        }
    }

    pub(super) fn sync_stats_snapshot(&self) -> OpenGlSyncStats {
        self.sync_stats
    }

    #[cfg(test)]
    pub(super) fn sync_stats_for_test(&self) -> OpenGlSyncStats {
        self.sync_stats_snapshot()
    }

    pub(super) fn completed_host_reads_snapshot(&self) -> &[CompletedHostRead] {
        &self.completed_host_reads
    }

    #[cfg(test)]
    pub(super) fn gl_errors_for_test(&self) -> &[String] {
        &self.gl_errors
    }

    #[cfg(test)]
    pub(super) fn state_cache_for_test(&self) -> StateCacheSnapshot {
        StateCacheSnapshot {
            program_binds: self.cache.program_binds,
            vao_binds: self.cache.vao_binds,
            framebuffer_binds: self.cache.framebuffer_binds,
            texture_binds: self.cache.texture_binds,
            sampler_binds: self.cache.sampler_binds,
            state_changes: self.cache.state_changes,
        }
    }

    fn poll_fences(&mut self, flush_commands: bool) {
        let _zone = trace::Zone::new("opengl.backend.fence-poll");
        let flags = if flush_commands {
            glow::SYNC_FLUSH_COMMANDS_BIT
        } else {
            0
        };
        unsafe {
            while let Some(pending) = self.submitted.front().copied() {
                self.sync_stats.fences_polled += 1;
                self.record_gl_call();
                let status = self.gl.client_wait_sync(pending.fence, flags, 0);
                if status == glow::ALREADY_SIGNALED || status == glow::CONDITION_SATISFIED {
                    let pending = self.submitted.pop_front().expect("front fence exists");
                    self.record_gl_call();
                    self.gl.delete_sync(pending.fence);
                    self.sync_stats.fences_deleted += 1;
                    self.completed = self.completed.max(pending.submission);
                } else {
                    break;
                }
            }
        }
    }

    fn wait_through(&mut self, completed: SubmissionId) -> GalResult<()> {
        let _zone = trace::Zone::new("opengl.backend.fence-wait");
        unsafe {
            while let Some(pending) = self.submitted.front().copied() {
                if pending.submission > completed {
                    break;
                }
                self.sync_stats.fences_waited += 1;
                loop {
                    self.record_gl_call();
                    let status = self.gl.client_wait_sync(
                        pending.fence,
                        glow::SYNC_FLUSH_COMMANDS_BIT,
                        1_000_000,
                    );
                    if status == glow::ALREADY_SIGNALED || status == glow::CONDITION_SATISFIED {
                        let pending = self.submitted.pop_front().expect("front fence exists");
                        self.record_gl_call();
                        self.gl.delete_sync(pending.fence);
                        self.sync_stats.fences_deleted += 1;
                        self.completed = self.completed.max(pending.submission);
                        break;
                    }
                    if status == glow::WAIT_FAILED {
                        return Err(GalError::backend("OpenGL fence wait failed"));
                    }
                }
            }
        }
        Ok(())
    }

    fn record_gl_call(&mut self) {
        self.sync_stats.gl_calls = self.sync_stats.gl_calls.saturating_add(1);
    }

    fn execute_op(
        &mut self,
        id: SubmissionId,
        objects: &mut OpenGlObjects,
        state: &mut ExecutionState,
        op: &CommandOp,
    ) -> GalResult<()> {
        match op {
            CommandOp::HostWriteBuffer {
                buffer,
                offset,
                data,
            } => self.host_write(objects, *buffer, *offset, data),
            CommandOp::CopyBuffer { src, dst, size } => {
                let _zone = trace::Zone::new("opengl.lowering.copy-buffer");
                let src_gl_buffer = objects.buffer(*src)?.buffer;
                let dst_object = objects.buffer(*dst)?;
                unsafe {
                    self.gl
                        .bind_buffer(glow::COPY_READ_BUFFER, Some(src_gl_buffer));
                    self.gl
                        .bind_buffer(glow::COPY_WRITE_BUFFER, Some(dst_object.buffer));
                    self.gl.copy_buffer_sub_data(
                        glow::COPY_READ_BUFFER,
                        glow::COPY_WRITE_BUFFER,
                        0,
                        0,
                        i32::try_from(*size)
                            .map_err(|_| GalError::backend("copy size exceeds i32"))?,
                    );
                }
                Ok(())
            }
            CommandOp::CopyBufferToTexture(region) => self.copy_buffer_to_texture(objects, region),
            CommandOp::CopyTextureToBuffer(region) => self.copy_texture_to_buffer(objects, region),
            CommandOp::CopyTexture(region) => self.copy_texture(objects, region),
            CommandOp::GenerateMipmaps {
                texture,
                subresources,
            } => self.generate_mipmaps(objects, *texture, *subresources),
            CommandOp::HostReadBuffer {
                buffer,
                offset,
                size,
            } => {
                let _zone = trace::Zone::new("opengl.backend.host-readback");
                let buffer_object = objects.buffer(*buffer)?;
                let start = usize::try_from(*offset)
                    .map_err(|_| GalError::backend("read offset exceeds usize"))?;
                let end = start
                    .checked_add(
                        usize::try_from(*size)
                            .map_err(|_| GalError::backend("read size exceeds usize"))?,
                    )
                    .ok_or_else(|| GalError::backend("read range overflows"))?;
                if end > buffer_object.size as usize {
                    return Err(GalError::backend("host read exceeds buffer size"));
                }
                let mut bytes = vec![0u8; end - start];
                // The CPU upload shadow is not authoritative after shader
                // writes or GPU copies. HostReadBuffer explicitly requests
                // GPU completion/readback; GL's get operation waits for the
                // preceding commands instead of manufacturing stale results.
                unsafe {
                    self.gl.bind_buffer(glow::COPY_READ_BUFFER, Some(buffer_object.buffer));
                    self.gl.get_buffer_sub_data(glow::COPY_READ_BUFFER,
                        i32::try_from(*offset).map_err(|_| GalError::backend("OpenGL read offset exceeds i32"))?,
                        &mut bytes);
                }
                self.completed_host_reads.push(CompletedHostRead {
                    submission: id,
                    buffer: *buffer,
                    offset: *offset,
                    bytes,
                });
                Ok(())
            }
            CommandOp::BeginPass {
                pass,
                target,
                colors,
                depth_stencil,
            } => {
                let _zone = trace::Zone::new("opengl.lowering.begin-pass");
                let pass_object = objects.render_pass(*pass)?;
                let target_object = objects.pass_target(*target)?;
                if pass_object.target != *target {
                    return Err(GalError::backend(
                        "OpenGL render pass target mismatch during lowering",
                    ));
                }
                self.bind_framebuffer(target_object.framebuffer);
                unsafe {
                    if colors.is_empty() {
                        self.gl.draw_buffer(glow::NONE);
                    } else if target_object.framebuffer.is_some() {
                        let draw_buffers = (0..colors.len())
                            .map(|index| glow::COLOR_ATTACHMENT0 + index as u32)
                            .collect::<Vec<_>>();
                        self.gl.draw_buffers(&draw_buffers);
                        self.gl.color_mask(true, true, true, true);
                    } else {
                        self.gl.draw_buffer(glow::BACK);
                        self.gl.color_mask(true, true, true, true);
                    }
                    self.gl.viewport(
                        0,
                        0,
                        i32::try_from(target_object.extent.width)
                            .map_err(|_| GalError::backend("viewport width exceeds i32"))?,
                        i32::try_from(target_object.extent.height)
                            .map_err(|_| GalError::backend("viewport height exceeds i32"))?,
                    );
                    self.gl.scissor(
                        0,
                        0,
                        i32::try_from(target_object.extent.width)
                            .map_err(|_| GalError::backend("scissor width exceeds i32"))?,
                        i32::try_from(target_object.extent.height)
                            .map_err(|_| GalError::backend("scissor height exceeds i32"))?,
                    );
                    self.gl.enable(glow::SCISSOR_TEST);
                    let mut mask = 0;
                    let framebuffer_color_clear_supported = target_object.framebuffer.is_some();
                    for (index, color) in colors.iter().enumerate() {
                        if color.load_op == AttachmentLoadOp::Clear {
                            let clear = color.clear_color.unwrap_or(
                                crate::render::vulkanic::commands::ClearColor {
                                    r: 0.0,
                                    g: 0.0,
                                    b: 0.0,
                                    a: 0.0,
                                },
                            );
                            if framebuffer_color_clear_supported {
                                self.gl.clear_buffer_f32_slice(
                                    glow::COLOR,
                                    u32::try_from(index).map_err(|_| {
                                        GalError::backend("color attachment index exceeds u32")
                                    })?,
                                    &[clear.r, clear.g, clear.b, clear.a],
                                );
                            } else {
                                self.gl.clear_color(clear.r, clear.g, clear.b, clear.a);
                                mask |= glow::COLOR_BUFFER_BIT;
                            }
                        }
                    }
                    if let Some(depth) = depth_stencil {
                        if depth.load_op == AttachmentLoadOp::Clear {
                            self.gl.depth_mask(true);
                            self.gl.clear_depth_f32(1.0);
                            mask |= glow::DEPTH_BUFFER_BIT;
                            if pass_object.depth_format == Some(TextureFormat::Depth24Stencil8) {
                                self.gl.stencil_mask(0xff);
                                self.gl.clear_stencil(0);
                                mask |= glow::STENCIL_BUFFER_BIT;
                            }
                        }
                    }
                    if mask != 0 {
                        self.gl.clear(mask);
                    }
                }
                state.in_pass = true;
                state.target = Some(*target);
                Ok(())
            }
            CommandOp::EndPass => {
                let _zone = trace::Zone::new("opengl.lowering.end-pass");
                unsafe {
                    self.gl.disable(glow::SCISSOR_TEST);
                }
                state.in_pass = false;
                state.pipeline = None;
                state.pipeline_layout = None;
                state.index_buffer = None;
                state.bound_sets.clear();
                Ok(())
            }
            CommandOp::BindGraphicsPipeline(handle) => {
                let _zone = trace::Zone::new("opengl.lowering.bind-graphics-pipeline");
                let pipeline = objects.graphics_pipeline(*handle)?;
                self.trace_draw_state("before-bind-graphics-pipeline", state);
                self.bind_program(Some(pipeline.program));
                self.bind_vao(Some(pipeline.vao));
                unsafe {
                    (self.provoking_vertex)(match pipeline.provoking_vertex {
                        crate::render::vulkanic::resources::ProvokingVertex::First => glow::FIRST_VERTEX_CONVENTION,
                        crate::render::vulkanic::resources::ProvokingVertex::Last => glow::LAST_VERTEX_CONVENTION,
                    });
                    if self.cache.raster_y_direction != Some(pipeline.raster_y_direction) {
                        (self.clip_control)(match pipeline.raster_y_direction {
                            crate::render::vulkanic::resources::RasterYDirection::Up => glow::LOWER_LEFT,
                            crate::render::vulkanic::resources::RasterYDirection::Down => glow::UPPER_LEFT,
                        }, glow::NEGATIVE_ONE_TO_ONE);
                        self.cache.raster_y_direction = Some(pipeline.raster_y_direction);
                    }
                }
                self.apply_fixed_state(
                    pipeline.cull_mode,
                    pipeline.front_face,
                    pipeline.blend,
                    pipeline.depth_compare,
                    pipeline.depth_write,
                    pipeline.depth_bias,
                    pipeline.stencil,
                );
                state.pipeline = Some(*handle);
                state.compute_pipeline = None;
                state.pipeline_layout = Some(pipeline.layout);
                state.topology = topology(pipeline.topology);
                self.trace_draw_state("after-bind-graphics-pipeline", state);
                Ok(())
            }
            CommandOp::BindComputePipeline(handle) => {
                let _zone = trace::Zone::new("opengl.lowering.bind-compute-pipeline");
                if state.in_pass {
                    return Err(GalError::backend(
                        "OpenGL compute pipeline cannot bind inside a render pass",
                    ));
                }
                let pipeline = objects.compute_pipeline(*handle)?;
                self.bind_program(Some(pipeline.program));
                self.bind_vao(None);
                state.pipeline = None;
                state.compute_pipeline = Some(*handle);
                state.pipeline_layout = Some(pipeline.layout);
                state.index_buffer = None;
                state.bound_sets.clear();
                Ok(())
            }
            CommandOp::BindResourceSet {
                pipeline_layout,
                set_index,
                set,
                dynamic_offsets,
            } => {
                let _zone = trace::Zone::new("opengl.lowering.bind-resource-set");
                let pipeline_layout_object = objects.pipeline_layout(*pipeline_layout)?;
                let Some(expected_layout) = pipeline_layout_object
                    .resource_layouts
                    .get(*set_index as usize)
                else {
                    return Err(GalError::backend("OpenGL resource set index out of range"));
                };
                let set_object = objects.resource_set(*set)?;
                if *expected_layout != set_object.layout {
                    return Err(GalError::backend(
                        "OpenGL resource set layout mismatch during lowering",
                    ));
                }
                state.bound_sets.insert(*set_index, *set);
                self.validate_sampled_texture_view_compatibility(objects, state)?;
                self.bind_resource_set(objects, *set_index, set_object, dynamic_offsets)?;
                Ok(())
            }
            CommandOp::SetIndexBuffer {
                buffer,
                offset,
                index_type,
            } => {
                let _zone = trace::Zone::new("opengl.lowering.bind-index-buffer");
                let buffer_object = objects.buffer(*buffer)?;
                unsafe {
                    self.gl
                        .bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(buffer_object.buffer));
                }
                state.index_buffer = Some((*buffer, *offset, *index_type));
                Ok(())
            }
            CommandOp::DrawIndexed { indices, instances } => {
                let _zone = trace::Zone::new("opengl.lowering.draw-indexed");
                let Some((_, offset, index_type)) = state.index_buffer else {
                    return Err(GalError::backend("indexed draw missing index buffer"));
                };
                self.trace_draw_state("before-draw-indexed", state);
                unsafe {
                    self.gl.draw_elements_instanced(
                        state.topology,
                        i32::try_from(*indices)
                            .map_err(|_| GalError::backend("index count exceeds i32"))?,
                        gl_index_type(index_type),
                        i32::try_from(offset)
                            .map_err(|_| GalError::backend("index offset exceeds i32"))?,
                        i32::try_from(*instances)
                            .map_err(|_| GalError::backend("instance count exceeds i32"))?,
                    );
                }
                self.trace_draw_state("after-draw-indexed", state);
                Ok(())
            }
            CommandOp::Draw {
                vertices,
                instances,
            } => {
                let _zone = trace::Zone::new("opengl.lowering.draw");
                unsafe {
                    self.gl.draw_arrays_instanced(
                        state.topology,
                        0,
                        i32::try_from(*vertices)
                            .map_err(|_| GalError::backend("vertex count exceeds i32"))?,
                        i32::try_from(*instances)
                            .map_err(|_| GalError::backend("instance count exceeds i32"))?,
                    );
                }
                Ok(())
            }
            CommandOp::Dispatch {
                groups_x,
                groups_y,
                groups_z,
            } => {
                let _zone = trace::Zone::new("opengl.lowering.dispatch");
                if state.compute_pipeline.is_none() || state.in_pass {
                    return Err(GalError::backend(
                        "OpenGL dispatch requires an active compute pipeline outside a render pass",
                    ));
                }
                unsafe {
                    self.record_gl_call();
                    self.gl.dispatch_compute(*groups_x, *groups_y, *groups_z);
                }
                Ok(())
            }
            CommandOp::Barrier(barrier) => self.apply_resource_barrier(barrier),
            CommandOp::DispatchIndirect { .. }
            | CommandOp::DrawIndirect { .. }
            | CommandOp::CopyFrameTargetToTexture { .. }
            | CommandOp::CopyTextureToFrameTarget { .. }
            | CommandOp::Present { .. }
            | CommandOp::SetVertexBuffer { .. } => Err(GalError::backend(format!(
                "OpenGL backend does not support command in isolated path: {op:?}"
            ))),
        }
    }

    fn host_write(
        &mut self,
        objects: &mut OpenGlObjects,
        buffer: Handle,
        offset: u64,
        data: &[u8],
    ) -> GalResult<()> {
        let _zone = trace::Zone::new("opengl.lowering.host-write");
        let buffer_object = objects.buffer_mut(buffer)?;
        let start =
            usize::try_from(offset).map_err(|_| GalError::backend("write offset exceeds usize"))?;
        let end = start
            .checked_add(data.len())
            .ok_or_else(|| GalError::backend("write range overflows"))?;
        if end > buffer_object.size as usize {
            return Err(GalError::backend("host write exceeds buffer size"));
        }
        unsafe {
            self.gl
                .bind_buffer(glow::COPY_WRITE_BUFFER, Some(buffer_object.buffer));
            self.gl.buffer_sub_data_u8_slice(
                glow::COPY_WRITE_BUFFER,
                i32::try_from(offset).map_err(|_| GalError::backend("write offset exceeds i32"))?,
                data,
            );
        }
        Ok(())
    }

    fn copy_buffer_to_texture(
        &mut self,
        objects: &OpenGlObjects,
        region: &BufferImageCopyRegion,
    ) -> GalResult<()> {
        let _zone = trace::Zone::new("opengl.lowering.copy-buffer-to-texture");
        let source = objects.buffer(region.buffer)?;
        let texture = objects.texture(region.texture)?;
        let format = texture_format(texture.format)?;
        let offset = u32::try_from(region.buffer_offset)
            .map_err(|_| GalError::backend("pixel-unpack buffer offset exceeds u32"))?;
        let row_length = pixel_unpack_row_length(region.bytes_per_row, format.bytes_per_pixel)?;
        let image_height = i32::try_from(region.rows_per_image)
            .map_err(|_| GalError::backend("pixel-unpack image height exceeds i32"))?;
        let mip = i32::try_from(region.texture_mip)
            .map_err(|_| GalError::backend("texture mip exceeds i32"))?;
        let x = i32::try_from(region.texture_origin.x)
            .map_err(|_| GalError::backend("texture origin x exceeds i32"))?;
        let y = gl_y_for_copy_region(texture, region)?;
        let z = i32::try_from(region.texture_origin.z)
            .map_err(|_| GalError::backend("texture origin z exceeds i32"))?;
        let width = i32::try_from(region.extent.width)
            .map_err(|_| GalError::backend("texture width exceeds i32"))?;
        let height = i32::try_from(region.extent.height)
            .map_err(|_| GalError::backend("texture height exceeds i32"))?;
        let depth = i32::try_from(region.extent.depth)
            .map_err(|_| GalError::backend("texture depth exceeds i32"))?;
        unsafe {
            // Consume the explicit GPU buffer, including shader-written data.
            // Pixel-store strides preserve the existing source row order and
            // padding without a CPU shadow or a second packed upload copy.
            let prior = std::num::NonZeroU32::new(
                self.gl.get_parameter_i32(glow::PIXEL_UNPACK_BUFFER_BINDING) as u32
            ).map(glow::NativeBuffer);
            self.gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, Some(source.buffer));
            self.gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            self.gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, row_length);
            self.gl.pixel_store_i32(glow::UNPACK_IMAGE_HEIGHT, image_height);
            self.gl.pixel_store_i32(glow::UNPACK_SKIP_ROWS, 0);
            self.gl.pixel_store_i32(glow::UNPACK_SKIP_PIXELS, 0);
            self.gl.pixel_store_i32(glow::UNPACK_SKIP_IMAGES, 0);
            let target = texture_target(texture.dimension);
            self.gl.bind_texture(target, Some(texture.texture));
            match texture.dimension {
                TextureDimension::D2 => self.gl.tex_sub_image_2d(
                    target, mip, x, y, width, height, format.external, format.ty,
                    glow::PixelUnpackData::BufferOffset(offset)),
                TextureDimension::D3 => self.gl.tex_sub_image_3d(
                    target, mip, x, y, z, width, height, depth, format.external, format.ty,
                    glow::PixelUnpackData::BufferOffset(offset)),
                _ => unreachable!("GAL validated texture dimension"),
            }
            self.gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, prior);
        }
        Ok(())
    }

    fn copy_texture_to_buffer(
        &mut self,
        objects: &mut OpenGlObjects,
        region: &BufferImageCopyRegion,
    ) -> GalResult<()> {
        let _zone = trace::Zone::new("opengl.lowering.copy-texture-to-buffer");
        let texture = objects.texture(region.texture)?;
        let format = texture_format(texture.format)?;
        let width = usize::try_from(region.extent.width)
            .map_err(|_| GalError::backend("readback width exceeds usize"))?;
        let height = usize::try_from(region.extent.height)
            .map_err(|_| GalError::backend("readback height exceeds usize"))?;
        let depth = usize::try_from(region.extent.depth)
            .map_err(|_| GalError::backend("readback depth exceeds usize"))?;
        let row_bytes = width
            .checked_mul(format.bytes_per_pixel as usize)
            .ok_or_else(|| GalError::backend("readback row size overflows"))?;
        let slice_bytes = row_bytes
            .checked_mul(height)
            .ok_or_else(|| GalError::backend("readback slice size overflows"))?;
        let mut pixels = vec![0; slice_bytes * depth];
        let read_fbo = unsafe { self.gl.create_framebuffer() }.map_err(|error| {
            GalError::backend(format!("failed to create readback FBO: {error}"))
        })?;
        let gl_y = gl_y_for_copy_region(texture, region)?;
        unsafe {
            self.gl
                .bind_framebuffer(glow::READ_FRAMEBUFFER, Some(read_fbo));
            let attachment = if texture.format == TextureFormat::Depth32Float {
                glow::DEPTH_ATTACHMENT
            } else {
                glow::COLOR_ATTACHMENT0
            };
            if texture.format == TextureFormat::Depth32Float {
                self.gl.read_buffer(glow::NONE);
            } else {
                self.gl.read_buffer(glow::COLOR_ATTACHMENT0);
            }
            self.gl.pixel_store_i32(glow::PACK_ALIGNMENT, 1);
            self.gl.pixel_store_i32(glow::PACK_ROW_LENGTH, 0);
            self.gl.pixel_store_i32(glow::PACK_SKIP_ROWS, 0);
            self.gl.pixel_store_i32(glow::PACK_SKIP_PIXELS, 0);
            self.gl.pixel_store_i32(glow::PACK_IMAGE_HEIGHT, 0);
            for slice in 0..depth {
                match texture.dimension {
                    crate::render::vulkanic::resources::TextureDimension::D2 => {
                        self.gl.framebuffer_texture_2d(
                            glow::READ_FRAMEBUFFER,
                            attachment,
                            glow::TEXTURE_2D,
                            Some(texture.texture),
                            i32::try_from(region.texture_mip)
                                .map_err(|_| GalError::backend("texture mip exceeds i32"))?,
                        );
                    }
                    crate::render::vulkanic::resources::TextureDimension::D3 => {
                        self.gl.framebuffer_texture_3d(
                            glow::READ_FRAMEBUFFER,
                            attachment,
                            glow::TEXTURE_3D,
                            Some(texture.texture),
                            i32::try_from(region.texture_mip)
                                .map_err(|_| GalError::backend("texture mip exceeds i32"))?,
                            i32::try_from(region.texture_origin.z)
                                .and_then(|origin| i32::try_from(slice).map(|value| origin + value))
                                .map_err(|_| {
                                    GalError::backend("texture depth layer exceeds i32")
                                })?,
                        );
                    }
                    _ => unreachable!("GAL validated texture dimension"),
                }
                let start = slice * slice_bytes;
                self.gl.read_pixels(
                    i32::try_from(region.texture_origin.x)
                        .map_err(|_| GalError::backend("texture origin x exceeds i32"))?,
                    gl_y,
                    i32::try_from(region.extent.width)
                        .map_err(|_| GalError::backend("texture width exceeds i32"))?,
                    i32::try_from(region.extent.height)
                        .map_err(|_| GalError::backend("texture height exceeds i32"))?,
                    format.external,
                    format.ty,
                    glow::PixelPackData::Slice(Some(&mut pixels[start..start + slice_bytes])),
                );
            }
            self.gl.bind_framebuffer(glow::READ_FRAMEBUFFER, None);
            self.gl.delete_framebuffer(read_fbo);
        }
        if texture.dimension != TextureDimension::D3 {
            for slice in 0..depth {
                flip_rows_in_place(
                    &mut pixels[slice * slice_bytes..(slice + 1) * slice_bytes],
                    row_bytes,
                    height,
                );
            }
        }
        let target = objects.buffer_mut(region.buffer)?;
        let dst_start = usize::try_from(region.buffer_offset)
            .map_err(|_| GalError::backend("readback buffer offset exceeds usize"))?;
        let bytes_per_row = usize::try_from(region.bytes_per_row)
            .map_err(|_| GalError::backend("readback bytes_per_row exceeds usize"))?;
        let rows_per_image = usize::try_from(region.rows_per_image)
            .map_err(|_| GalError::backend("readback rows_per_image exceeds usize"))?;
        unsafe {
            self.gl
                .bind_buffer(glow::COPY_WRITE_BUFFER, Some(target.buffer));
            if bytes_per_row == row_bytes && rows_per_image == height {
                self.gl.buffer_sub_data_u8_slice(glow::COPY_WRITE_BUFFER,
                    i32::try_from(dst_start).map_err(|_| GalError::backend("readback buffer offset exceeds i32"))?,
                    &pixels);
            } else {
                // Update only the declared texel bytes. Padding and adjacent
                // GPU-written data must not be replaced with an upload cache.
                for slice in 0..depth {
                    for row in 0..height {
                        let src = slice * slice_bytes + row * row_bytes;
                        let dst = dst_start + (slice * rows_per_image + row) * bytes_per_row;
                        self.gl.buffer_sub_data_u8_slice(glow::COPY_WRITE_BUFFER,
                            i32::try_from(dst).map_err(|_| GalError::backend("readback row offset exceeds i32"))?,
                            &pixels[src..src + row_bytes]);
                    }
                }
            }
        }
        Ok(())
    }

    fn copy_texture(
        &mut self,
        objects: &OpenGlObjects,
        region: &TextureImageCopyRegion,
    ) -> GalResult<()> {
        let _zone = trace::Zone::new("opengl.lowering.copy-texture");
        if region.row_order != crate::render::vulkanic::commands::TextureRowOrder::Preserve {
            return Err(GalError::unsupported_feature(
                "OpenGL explicit texture row reversal is not implemented",
            ));
        }
        let source = objects.texture(region.src_texture)?;
        let destination = objects.texture(region.dst_texture)?;
        let src_y = gl_y_for_texture_copy(
            source,
            region.src_mip,
            region.src_origin.y,
            region.extent.height,
        )?;
        let dst_y = gl_y_for_texture_copy(
            destination,
            region.dst_mip,
            region.dst_origin.y,
            region.extent.height,
        )?;
        unsafe {
            self.gl.copy_image_sub_data(
                source.texture,
                texture_target(source.dimension),
                i32::try_from(region.src_mip)
                    .map_err(|_| GalError::backend("source texture mip exceeds i32"))?,
                i32::try_from(region.src_origin.x)
                    .map_err(|_| GalError::backend("source texture origin x exceeds i32"))?,
                src_y,
                i32::try_from(region.src_origin.z)
                    .map_err(|_| GalError::backend("source texture origin z exceeds i32"))?,
                destination.texture,
                texture_target(destination.dimension),
                i32::try_from(region.dst_mip)
                    .map_err(|_| GalError::backend("destination texture mip exceeds i32"))?,
                i32::try_from(region.dst_origin.x)
                    .map_err(|_| GalError::backend("destination texture origin x exceeds i32"))?,
                dst_y,
                i32::try_from(region.dst_origin.z)
                    .map_err(|_| GalError::backend("destination texture origin z exceeds i32"))?,
                i32::try_from(region.extent.width)
                    .map_err(|_| GalError::backend("texture copy width exceeds i32"))?,
                i32::try_from(region.extent.height)
                    .map_err(|_| GalError::backend("texture copy height exceeds i32"))?,
                i32::try_from(region.extent.depth)
                    .map_err(|_| GalError::backend("texture copy depth exceeds i32"))?,
            );
        }
        Ok(())
    }

    fn generate_mipmaps(
        &mut self,
        objects: &mut OpenGlObjects,
        texture: Handle,
        subresources: crate::render::vulkanic::resources::TextureSubresourceRange,
    ) -> GalResult<()> {
        let _zone = trace::Zone::new("opengl.lowering.generate-mipmaps");
        let texture = objects.texture(texture)?;
        if subresources.base_mip != 0
            || subresources.mip_count != texture.mip_levels
            || subresources.base_layer != 0
            || subresources.layer_count != texture.array_layers
        {
            return Err(GalError::backend(
                "OpenGL mip generation requires the validated complete texture mip and layer range",
            ));
        }
        unsafe {
            self.gl
                .bind_texture(texture_target(texture.dimension), Some(texture.texture));
            self.gl.generate_mipmap(texture_target(texture.dimension));
        }
        // This native operation binds a texture outside descriptor reconstruction.
        // Make following explicit resource-set binds re-establish all texture state.
        self.cache.textures.clear();
        Ok(())
    }

    fn bind_resource_set(
        &mut self,
        objects: &OpenGlObjects,
        set_index: u32,
        set: &ResourceSetObject,
        dynamic_offsets: &[u64],
    ) -> GalResult<()> {
        let mut sampled_texture_units = Vec::new();
        let mut sampler_bindings = Vec::new();
        let mut dynamic_offset_index = 0usize;
        for binding in &set.bindings {
            let binding_point = opengl_resource_binding_point(set_index, binding.binding)?;
            match binding.kind {
                ResourceBindingKind::SampledTexture => {
                    let view = objects.texture_view(binding.resource)?;
                    let texture = objects.texture(view.texture)?;
                    let unit = binding_point;
                    sampled_texture_units.push(unit);
                    self.bind_sampled_texture_view(
                        unit,
                        texture_target(texture.dimension),
                        texture.texture,
                        view.base_mip,
                        view.mip_count,
                    );
                    self.bind_sampler_unit(unit, None);
                }
                ResourceBindingKind::Sampler => {
                    sampler_bindings.push(binding.resource);
                }
                ResourceBindingKind::CombinedTextureSampler => {
                    let combined = objects.combined_texture_sampler(binding.resource)?;
                    let view = objects.texture_view(combined.texture_view)?;
                    let texture = objects.texture(view.texture)?;
                    let sampler = objects.sampler(combined.sampler)?;
                    let unit = binding_point;
                    self.bind_sampled_texture_view(
                        unit,
                        texture_target(texture.dimension),
                        texture.texture,
                        view.base_mip,
                        view.mip_count,
                    );
                    self.bind_sampler_unit(unit, Some(sampler.sampler));
                }
                ResourceBindingKind::UniformBuffer | ResourceBindingKind::StorageBuffer => {
                    let buffer = objects.buffer(binding.resource)?;
                    let target = if binding.kind == ResourceBindingKind::UniformBuffer {
                        glow::UNIFORM_BUFFER
                    } else {
                        glow::SHADER_STORAGE_BUFFER
                    };
                    let offset = if binding.dynamic_offsets.is_empty() {
                        0
                    } else if dynamic_offsets.is_empty() {
                        binding.dynamic_offsets[0]
                    } else {
                        let Some(offset) = dynamic_offsets.get(dynamic_offset_index).copied()
                        else {
                            return Err(GalError::backend(
                                "missing dynamic offset for OpenGL resource binding",
                            ));
                        };
                        dynamic_offset_index += binding.dynamic_offsets.len();
                        offset
                    };
                    unsafe {
                        let range = binding
                            .buffer_range
                            .unwrap_or_else(|| buffer.size.saturating_sub(offset));
                        self.gl.bind_buffer_range(
                            target,
                            binding_point,
                            Some(buffer.buffer),
                            i32::try_from(offset)
                                .map_err(|_| GalError::backend("dynamic offset exceeds i32"))?,
                            i32::try_from(range)
                                .map_err(|_| GalError::backend("dynamic range exceeds i32"))?,
                        );
                    }
                }
                ResourceBindingKind::StorageTexture => {
                    let view = objects.texture_view(binding.resource)?;
                    let texture = objects.texture(view.texture)?;
                    self.bind_image_unit(
                        binding_point,
                        texture.texture,
                        view.base_mip,
                        texture.dimension == TextureDimension::D3,
                        view.base_layer,
                        storage_texture_access(binding.access)?,
                        texture_format(view.format)?.internal as u32,
                    )?;
                }
            }
        }
        for (index, sampler) in sampler_bindings.iter().copied().enumerate() {
            let sampler = objects.sampler(sampler)?;
            if let Some(unit) = sampled_texture_units.get(index).copied() {
                self.bind_sampler_unit(unit, Some(sampler.sampler));
            }
        }
        if sampler_bindings.len() == 1 {
            let sampler = objects.sampler(sampler_bindings[0])?;
            for unit in sampled_texture_units {
                self.bind_sampler_unit(unit, Some(sampler.sampler));
            }
        }
        Ok(())
    }

    fn validate_sampled_texture_view_compatibility(
        &self,
        objects: &OpenGlObjects,
        state: &ExecutionState,
    ) -> GalResult<()> {
        let mut views = BTreeMap::new();
        for set_handle in state.bound_sets.values().copied() {
            let set = objects.resource_set(set_handle)?;
            for binding in &set.bindings {
                if !matches!(
                    binding.kind,
                    ResourceBindingKind::SampledTexture
                        | ResourceBindingKind::CombinedTextureSampler
                ) {
                    continue;
                }
                let view_handle = match binding.kind {
                    ResourceBindingKind::SampledTexture => binding.resource,
                    ResourceBindingKind::CombinedTextureSampler => {
                        objects
                            .combined_texture_sampler(binding.resource)?
                            .texture_view
                    }
                    _ => unreachable!(),
                };
                let view = objects.texture_view(view_handle)?;
                record_sampled_texture_view_range(
                    &mut views,
                    view.texture,
                    (
                        view.base_mip,
                        view.mip_count,
                        view.base_layer,
                        view.layer_count,
                    ),
                )?;
            }
        }
        Ok(())
    }

    fn apply_fixed_state(
        &mut self,
        cull_mode: CullMode,
        front_face: FrontFace,
        blend: BlendMode,
        depth_compare: Option<CompareOp>,
        depth_write: bool,
        depth_bias: Option<crate::render::vulkanic::resources::DepthBias>,
        stencil: Option<crate::render::vulkanic::resources::StencilState>,
    ) {
        unsafe {
            let front_face_ccw = gl_front_face_is_counter_clockwise(front_face);
            if self.cache.front_face_ccw != Some(front_face_ccw) {
                self.record_gl_call();
                self.gl
                    .front_face(if front_face_ccw { glow::CCW } else { glow::CW });
                self.cache.front_face_ccw = Some(front_face_ccw);
                self.cache.state_changes += 1;
            }
            if self.cache.cull != Some(cull_mode) {
                match cull_mode {
                    CullMode::None => self.gl.disable(glow::CULL_FACE),
                    CullMode::Front => {
                        self.gl.enable(glow::CULL_FACE);
                        self.gl.cull_face(glow::FRONT);
                    }
                    CullMode::Back => {
                        self.gl.enable(glow::CULL_FACE);
                        self.gl.cull_face(glow::BACK);
                    }
                }
                self.cache.cull = Some(cull_mode);
                self.cache.state_changes += 1;
            }
            if self.cache.blend != Some(blend) {
                let blend_state = opengl_blend_state(blend);
                if blend_state.enabled {
                    self.gl.enable(glow::BLEND);
                } else {
                    self.gl.disable(glow::BLEND);
                }
                self.gl
                    .blend_equation_separate(blend_state.color_op, blend_state.alpha_op);
                if let Some(factors) = blend_state.factors {
                    self.gl.blend_func_separate(
                        factors.src_color,
                        factors.dst_color,
                        factors.src_alpha,
                        factors.dst_alpha,
                    );
                }
                self.cache.blend = Some(blend);
                self.cache.state_changes += 1;
            }
            if self.cache.depth_compare != depth_compare || self.cache.depth_write != depth_write {
                if let Some(compare) = depth_compare {
                    self.gl.enable(glow::DEPTH_TEST);
                    self.gl.depth_func(compare_op(compare));
                    self.gl.depth_mask(depth_write);
                } else {
                    self.gl.disable(glow::DEPTH_TEST);
                    self.gl.depth_mask(false);
                }
                self.cache.depth_compare = depth_compare;
                self.cache.depth_write = depth_write;
                self.cache.state_changes += 1;
            }
            if self.cache.depth_bias != depth_bias {
                if let Some(bias) = depth_bias {
                    self.gl.enable(glow::POLYGON_OFFSET_FILL);
                    self.gl
                        .polygon_offset(bias.slope_factor, bias.constant_factor);
                } else {
                    self.gl.disable(glow::POLYGON_OFFSET_FILL);
                }
                self.cache.depth_bias = depth_bias;
                self.cache.state_changes += 1;
            }
            if let Some(stencil) = stencil {
                self.gl.enable(glow::STENCIL_TEST);
                apply_stencil_face(&self.gl, glow::FRONT, stencil.front);
                apply_stencil_face(&self.gl, glow::BACK, stencil.back);
            } else {
                self.gl.disable(glow::STENCIL_TEST);
            }
        }
    }

    fn bind_program(&mut self, program: Option<glow::Program>) {
        if self.cache.program == program {
            return;
        }
        unsafe {
            self.gl.use_program(program);
        }
        self.cache.program = program;
        self.cache.program_binds += 1;
    }

    fn trace_draw_state(&self, phase: &str, state: &ExecutionState) {
        let limit = *GL_DRAW_TRACE_LIMIT.get_or_init(|| {
            std::env::var("MATTMC_RUST_GAL_TRACE_GL_DRAWS")
                .ok()
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0)
        });
        if limit == 0 {
            return;
        }
        let index = GL_DRAW_TRACE_COUNT.fetch_add(1, Ordering::Relaxed);
        if index >= limit {
            return;
        }
        unsafe {
            let current_program = self.gl.get_parameter_i32(glow::CURRENT_PROGRAM);
            let program_label = if current_program > 0 {
                self.gl
                    .get_object_label(glow::PROGRAM, current_program as u32)
            } else {
                String::new()
            };
            eprintln!(
                "MATTMC_RUST_GAL_GL_DRAW_STATE phase={} current_program={} program_label={} draw_fbo={} read_fbo={} draw_buffer0={} vertex_array={} array_buffer={} element_array_buffer={} active_texture={} depth_test={} depth_write={} blend={} cull_face={} scissor={} pipeline_bound={} index_buffer_bound={}",
                phase,
                current_program,
                program_label,
                self.gl.get_parameter_i32(glow::DRAW_FRAMEBUFFER_BINDING),
                self.gl.get_parameter_i32(glow::READ_FRAMEBUFFER_BINDING),
                self.gl.get_parameter_i32(glow::DRAW_BUFFER0),
                self.gl.get_parameter_i32(glow::VERTEX_ARRAY_BINDING),
                self.gl.get_parameter_i32(glow::ARRAY_BUFFER_BINDING),
                self.gl.get_parameter_i32(glow::ELEMENT_ARRAY_BUFFER_BINDING),
                self.gl.get_parameter_i32(glow::ACTIVE_TEXTURE),
                self.gl.is_enabled(glow::DEPTH_TEST),
                self.gl.get_parameter_bool(glow::DEPTH_WRITEMASK),
                self.gl.is_enabled(glow::BLEND),
                self.gl.is_enabled(glow::CULL_FACE),
                self.gl.is_enabled(glow::SCISSOR_TEST),
                state.pipeline.is_some(),
                state.index_buffer.is_some(),
            );
        }
    }

    fn bind_vao(&mut self, vao: Option<glow::VertexArray>) {
        if self.cache.vao == vao {
            return;
        }
        unsafe {
            self.gl.bind_vertex_array(vao);
        }
        self.cache.vao = vao;
        self.cache.vao_binds += 1;
    }

    fn bind_framebuffer(&mut self, framebuffer: Option<glow::Framebuffer>) {
        if self.cache.framebuffer == framebuffer {
            return;
        }
        unsafe {
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, framebuffer);
        }
        self.cache.framebuffer = framebuffer;
        self.cache.framebuffer_binds += 1;
    }

    fn bind_texture_unit(&mut self, unit: u32, target: u32, texture: Option<glow::Texture>) {
        if self.cache.textures.get(&unit).copied() == Some((target, texture)) {
            return;
        }
        unsafe {
            self.gl.active_texture(glow::TEXTURE0 + unit);
            self.gl.bind_texture(target, texture);
        }
        self.cache.textures.insert(unit, (target, texture));
        self.cache.texture_binds += 1;
    }

    fn bind_sampled_texture_view(
        &mut self,
        unit: u32,
        target: u32,
        texture: glow::Texture,
        base_mip: u32,
        mip_count: u32,
    ) {
        self.bind_texture_unit(unit, target, Some(texture));
        unsafe {
            // OpenGL texture parameters are texture-object state, unlike a
            // Vulkan image view. The compatibility backend reconstructs this
            // private detail per binding and rejects simultaneous incompatible
            // views above so one draw cannot silently sample the wrong mip.
            self.gl.active_texture(glow::TEXTURE0 + unit);
            self.gl
                .tex_parameter_i32(target, glow::TEXTURE_BASE_LEVEL, base_mip as i32);
            self.gl.tex_parameter_i32(
                target,
                glow::TEXTURE_MAX_LEVEL,
                base_mip.saturating_add(mip_count.saturating_sub(1)) as i32,
            );
        }
    }

    fn bind_sampler_unit(&mut self, unit: u32, sampler: Option<glow::Sampler>) {
        if self.cache.samplers.get(&unit).copied().flatten() == sampler {
            return;
        }
        unsafe {
            self.gl.bind_sampler(unit, sampler);
        }
        self.cache.samplers.insert(unit, sampler);
        self.cache.sampler_binds += 1;
    }

    fn bind_image_unit(
        &mut self,
        unit: u32,
        texture: glow::Texture,
        mip: u32,
        layered: bool,
        layer: u32,
        access: u32,
        format: u32,
    ) -> GalResult<()> {
        let level =
            i32::try_from(mip).map_err(|_| GalError::backend("OpenGL image mip exceeds i32"))?;
        let layer = i32::try_from(layer)
            .map_err(|_| GalError::backend("OpenGL image layer exceeds i32"))?;
        let state = ImageUnitBinding {
            texture,
            level,
            layered,
            layer,
            access,
            format,
        };
        if self.cache.image_units.get(&unit).copied() == Some(state) {
            return Ok(());
        }
        unsafe {
            self.gl
                .bind_image_texture(unit, Some(texture), level, layered, layer, access, format);
        }
        self.cache.image_units.insert(unit, state);
        self.cache.image_binds += 1;
        Ok(())
    }

    fn check_errors(&mut self, context: &str) -> GalResult<()> {
        let mut saw_error = false;
        loop {
            let error = unsafe { self.gl.get_error() };
            if error == glow::NO_ERROR {
                break;
            }
            saw_error = true;
            self.gl_errors
                .push(format!("OpenGL error 0x{error:04x} after {context}"));
        }
        if saw_error
            && std::env::var("MATTMC_OPENGL_STRICT")
                .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
                .unwrap_or(false)
        {
            return Err(GalError::backend(format!(
                "OpenGL strict error scan failed after {context}"
            )));
        }
        Ok(())
    }

    fn apply_resource_barrier(&mut self, barrier: &ResourceBarrier) -> GalResult<()> {
        let bits = gl_memory_barrier_bits(barrier.before, barrier.after);
        if bits != 0 {
            unsafe {
                self.record_gl_call();
                self.gl.memory_barrier(bits);
            }
        }
        Ok(())
    }
}

fn gl_front_face_is_counter_clockwise(front_face: FrontFace) -> bool {
    matches!(front_face, FrontFace::CounterClockwise)
}

fn record_sampled_texture_view_range(
    views: &mut BTreeMap<Handle, (u32, u32, u32, u32)>,
    texture: Handle,
    range: (u32, u32, u32, u32),
) -> GalResult<()> {
    if let Some(previous) = views.insert(texture, range) {
        if previous != range {
            return Err(GalError::backend(
                "OpenGL lowering cannot bind incompatible mip/layer views of one texture in a single draw",
            ));
        }
    }
    Ok(())
}

#[derive(Default)]
struct ExecutionState {
    in_pass: bool,
    target: Option<Handle>,
    pipeline: Option<Handle>,
    compute_pipeline: Option<Handle>,
    pipeline_layout: Option<Handle>,
    index_buffer: Option<(Handle, u64, crate::render::vulkanic::resources::IndexType)>,
    topology: u32,
    bound_sets: BTreeMap<u32, Handle>,
}

#[derive(Default)]
struct StateCache {
    raster_y_direction: Option<crate::render::vulkanic::resources::RasterYDirection>,
    program: Option<glow::Program>,
    vao: Option<glow::VertexArray>,
    framebuffer: Option<glow::Framebuffer>,
    front_face_ccw: Option<bool>,
    cull: Option<CullMode>,
    blend: Option<BlendMode>,
    depth_compare: Option<CompareOp>,
    depth_write: bool,
    depth_bias: Option<crate::render::vulkanic::resources::DepthBias>,
    textures: BTreeMap<u32, (u32, Option<glow::Texture>)>,
    samplers: BTreeMap<u32, Option<glow::Sampler>>,
    image_units: BTreeMap<u32, ImageUnitBinding>,
    program_binds: usize,
    vao_binds: usize,
    framebuffer_binds: usize,
    texture_binds: usize,
    sampler_binds: usize,
    image_binds: usize,
    state_changes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ImageUnitBinding {
    texture: glow::Texture,
    level: i32,
    layered: bool,
    layer: i32,
    access: u32,
    format: u32,
}

fn storage_texture_access(
    access: crate::render::vulkanic::resources::AccessFlags,
) -> GalResult<u32> {
    match (access.reads(), access.writes()) {
        (true, true) => Ok(glow::READ_WRITE),
        (true, false) => Ok(glow::READ_ONLY),
        (false, true) => Ok(glow::WRITE_ONLY),
        (false, false) => Err(GalError::backend(
            "OpenGL storage texture binding requires read or write access",
        )),
    }
}

#[derive(Copy, Clone)]
struct PendingFence {
    submission: SubmissionId,
    fence: glow::Fence,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::render::vulkanic) struct OpenGlSyncStats {
    pub(in crate::render::vulkanic) command_batches: u64,
    pub(in crate::render::vulkanic) command_lists: u64,
    pub(in crate::render::vulkanic) command_ops: u64,
    pub(in crate::render::vulkanic) gl_calls: u64,
    pub(in crate::render::vulkanic) flushes: usize,
    pub(in crate::render::vulkanic) finishes: usize,
    pub(in crate::render::vulkanic) fences_inserted: usize,
    pub(in crate::render::vulkanic) fences_polled: usize,
    pub(in crate::render::vulkanic) fences_waited: usize,
    pub(in crate::render::vulkanic) fences_deleted: usize,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::render::vulkanic) struct StateCacheSnapshot {
    pub(in crate::render::vulkanic) program_binds: usize,
    pub(in crate::render::vulkanic) vao_binds: usize,
    pub(in crate::render::vulkanic) framebuffer_binds: usize,
    pub(in crate::render::vulkanic) texture_binds: usize,
    pub(in crate::render::vulkanic) sampler_binds: usize,
    pub(in crate::render::vulkanic) state_changes: usize,
}

pub(super) fn compare_op(compare: CompareOp) -> u32 {
    match compare {
        CompareOp::Always => glow::ALWAYS,
        CompareOp::Less => glow::LESS,
        CompareOp::LessOrEqual => glow::LEQUAL,
        CompareOp::Equal => glow::EQUAL,
        CompareOp::Greater => glow::GREATER,
    }
}

fn stencil_op(operation: crate::render::vulkanic::resources::StencilOp) -> u32 {
    match operation {
        crate::render::vulkanic::resources::StencilOp::Keep => glow::KEEP,
        crate::render::vulkanic::resources::StencilOp::Replace => glow::REPLACE,
    }
}

unsafe fn apply_stencil_face(
    gl: &glow::Context,
    face: u32,
    state: crate::render::vulkanic::resources::StencilFaceState,
) {
    gl.stencil_func_separate(
        face,
        compare_op(state.compare),
        state.reference as i32,
        state.read_mask,
    );
    gl.stencil_op_separate(
        face,
        stencil_op(state.fail_op),
        stencil_op(state.depth_fail_op),
        stencil_op(state.pass_op),
    );
    gl.stencil_mask_separate(face, state.write_mask);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OpenGlBlendFactors {
    src_color: u32,
    dst_color: u32,
    src_alpha: u32,
    dst_alpha: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OpenGlBlendState {
    enabled: bool,
    color_op: u32,
    alpha_op: u32,
    factors: Option<OpenGlBlendFactors>,
}

fn opengl_blend_state(blend: BlendMode) -> OpenGlBlendState {
    let factors = match blend {
        BlendMode::Disabled => None,
        BlendMode::TerrainTranslucent => Some(OpenGlBlendFactors {
            src_color: glow::SRC_ALPHA,
            dst_color: glow::ONE_MINUS_SRC_ALPHA,
            src_alpha: glow::ONE,
            dst_alpha: glow::ONE_MINUS_SRC_ALPHA,
        }),
        BlendMode::Alpha => Some(OpenGlBlendFactors {
            src_color: glow::SRC_ALPHA,
            dst_color: glow::ONE_MINUS_SRC_ALPHA,
            src_alpha: glow::ONE,
            dst_alpha: glow::ONE_MINUS_SRC_ALPHA,
        }),
        BlendMode::AlphaPreserveAlpha => Some(OpenGlBlendFactors {
            src_color: glow::SRC_ALPHA,
            dst_color: glow::ONE_MINUS_SRC_ALPHA,
            src_alpha: glow::ZERO,
            dst_alpha: glow::ONE,
        }),
        BlendMode::Premultiplied => Some(OpenGlBlendFactors {
            src_color: glow::ONE,
            dst_color: glow::ONE_MINUS_SRC_ALPHA,
            src_alpha: glow::ONE,
            dst_alpha: glow::ONE_MINUS_SRC_ALPHA,
        }),
        BlendMode::Additive => Some(OpenGlBlendFactors {
            src_color: glow::ONE,
            dst_color: glow::ONE,
            src_alpha: glow::ONE,
            dst_alpha: glow::ONE,
        }),
        BlendMode::Invert => Some(OpenGlBlendFactors {
            src_color: glow::ONE_MINUS_DST_COLOR,
            dst_color: glow::ONE_MINUS_SRC_COLOR,
            src_alpha: glow::ONE,
            dst_alpha: glow::ZERO,
        }),
        BlendMode::Multiply => Some(OpenGlBlendFactors {
            src_color: glow::DST_COLOR,
            dst_color: glow::ZERO,
            src_alpha: glow::ONE,
            dst_alpha: glow::ZERO,
        }),
        BlendMode::Overlay => Some(OpenGlBlendFactors {
            src_color: glow::SRC_ALPHA,
            dst_color: glow::ONE,
            src_alpha: glow::ONE,
            dst_alpha: glow::ZERO,
        }),
        BlendMode::Glint => Some(OpenGlBlendFactors {
            src_color: glow::SRC_COLOR,
            dst_color: glow::ONE,
            src_alpha: glow::ZERO,
            dst_alpha: glow::ONE,
        }),
        BlendMode::Vignette => Some(OpenGlBlendFactors {
            src_color: glow::ZERO,
            dst_color: glow::ONE_MINUS_SRC_COLOR,
            src_alpha: glow::ONE,
            dst_alpha: glow::ZERO,
        }),
    };
    OpenGlBlendState {
        enabled: factors.is_some(),
        color_op: glow::FUNC_ADD,
        alpha_op: glow::FUNC_ADD,
        factors,
    }
}

fn gl_index_type(index_type: crate::render::vulkanic::resources::IndexType) -> u32 {
    match index_type {
        crate::render::vulkanic::resources::IndexType::U16 => glow::UNSIGNED_SHORT,
        crate::render::vulkanic::resources::IndexType::U32 => glow::UNSIGNED_INT,
    }
}

fn gl_memory_barrier_bits(before: TextureUsageState, after: TextureUsageState) -> u32 {
    match (before, after) {
        (TextureUsageState::ShaderWrite, TextureUsageState::ShaderRead) => {
            glow::SHADER_IMAGE_ACCESS_BARRIER_BIT | glow::TEXTURE_FETCH_BARRIER_BIT
                | glow::SHADER_STORAGE_BARRIER_BIT | glow::UNIFORM_BARRIER_BIT | glow::BUFFER_UPDATE_BARRIER_BIT
        }
        (TextureUsageState::ShaderWrite, TextureUsageState::ShaderStorageRead)
        | (TextureUsageState::ShaderStorageRead, TextureUsageState::ShaderStorageRead) => {
            glow::SHADER_IMAGE_ACCESS_BARRIER_BIT | glow::SHADER_STORAGE_BARRIER_BIT
        }
        (TextureUsageState::ShaderWrite, TextureUsageState::ShaderWrite) => {
            glow::SHADER_IMAGE_ACCESS_BARRIER_BIT | glow::SHADER_STORAGE_BARRIER_BIT
        }
        (TextureUsageState::ShaderWrite, TextureUsageState::TransferSrc) => {
            glow::SHADER_IMAGE_ACCESS_BARRIER_BIT
                | glow::TEXTURE_UPDATE_BARRIER_BIT
                | glow::FRAMEBUFFER_BARRIER_BIT
                | glow::BUFFER_UPDATE_BARRIER_BIT
        }
        (TextureUsageState::TransferDst, TextureUsageState::ShaderWrite) => {
            glow::TEXTURE_UPDATE_BARRIER_BIT | glow::SHADER_IMAGE_ACCESS_BARRIER_BIT
                | glow::SHADER_STORAGE_BARRIER_BIT | glow::BUFFER_UPDATE_BARRIER_BIT
        }
        (TextureUsageState::TransferDst, TextureUsageState::ShaderRead) => {
            glow::TEXTURE_FETCH_BARRIER_BIT | glow::SHADER_STORAGE_BARRIER_BIT
        }
        (TextureUsageState::TransferDst, TextureUsageState::ShaderStorageRead) => {
            glow::SHADER_IMAGE_ACCESS_BARRIER_BIT | glow::SHADER_STORAGE_BARRIER_BIT
        }
        (TextureUsageState::TransferDst, TextureUsageState::IndexRead) => {
            glow::ELEMENT_ARRAY_BARRIER_BIT
        }
        (TextureUsageState::TransferDst, TextureUsageState::TransferSrc)
        | (TextureUsageState::TransferSrc, TextureUsageState::TransferDst) => {
            glow::BUFFER_UPDATE_BARRIER_BIT | glow::TEXTURE_UPDATE_BARRIER_BIT
        }
        (TextureUsageState::ColorAttachment, TextureUsageState::ShaderRead)
        | (TextureUsageState::DepthStencilAttachment, TextureUsageState::ShaderRead) => {
            glow::FRAMEBUFFER_BARRIER_BIT | glow::TEXTURE_FETCH_BARRIER_BIT
        }
        _ => 0,
    }
}

fn flip_rows_in_place(bytes: &mut [u8], row_bytes: usize, rows: usize) {
    for y in 0..rows / 2 {
        let top = y * row_bytes;
        let bottom = (rows - 1 - y) * row_bytes;
        for x in 0..row_bytes {
            bytes.swap(top + x, bottom + x);
        }
    }
}

fn gl_y_for_copy_region(
    texture: &super::resources::TextureObject,
    region: &BufferImageCopyRegion,
) -> GalResult<i32> {
    if texture.dimension == TextureDimension::D3 {
        // A 3D texture is a semantic volume, not a framebuffer. Its Y axis
        // must agree with Vulkan image coordinates and shader imageLoad/store;
        // screen-space top-left normalization applies only to D2 copies.
        return i32::try_from(region.texture_origin.y)
            .map_err(|_| GalError::backend("3D texture origin y exceeds i32"));
    }
    let texture_height = i64::from((texture.extent.height >> region.texture_mip).max(1));
    let origin_y = i64::from(region.texture_origin.y);
    let copy_height = i64::from(region.extent.height);
    let end_y = origin_y
        .checked_add(copy_height)
        .ok_or_else(|| GalError::backend("top-left texture region overflows"))?;
    if end_y > texture_height {
        return Err(GalError::backend(
            "top-left texture region is outside GL image bounds",
        ));
    }
    let gl_y = texture_height - end_y;
    i32::try_from(gl_y).map_err(|_| GalError::backend("translated GL y exceeds i32"))
}

fn gl_y_for_texture_copy(
    texture: &super::resources::TextureObject,
    mip: u32,
    origin_y: u32,
    height: u32,
) -> GalResult<i32> {
    gl_y_for_texture_copy_values(
        texture.dimension,
        texture.extent.height,
        mip,
        origin_y,
        height,
    )
}

fn gl_y_for_texture_copy_values(
    dimension: TextureDimension,
    base_height: u32,
    mip: u32,
    origin_y: u32,
    height: u32,
) -> GalResult<i32> {
    if dimension == TextureDimension::D3 {
        return i32::try_from(origin_y)
            .map_err(|_| GalError::backend("3D texture origin y exceeds i32"));
    }
    let mip_height = i64::from((base_height >> mip).max(1));
    let end_y = i64::from(origin_y)
        .checked_add(i64::from(height))
        .ok_or_else(|| GalError::backend("top-left texture copy overflows"))?;
    if end_y > mip_height {
        return Err(GalError::backend(
            "top-left texture copy is outside GL image bounds",
        ));
    }
    let gl_y = mip_height - end_y;
    i32::try_from(gl_y).map_err(|_| GalError::backend("translated GL y exceeds i32"))
}

/// Both D2 and D3 copies keep source row order and express padding through
/// pixel-store stride; gl_y_for_copy_region remains the coordinate conversion.
fn pixel_unpack_row_length(bytes_per_row: u32, bytes_per_pixel: u32) -> GalResult<i32> {
    if bytes_per_row == 0 || bytes_per_pixel == 0 || bytes_per_row % bytes_per_pixel != 0 {
        return Err(GalError::backend("pixel-unpack pitch must contain complete pixels"));
    }
    i32::try_from(bytes_per_row / bytes_per_pixel)
        .map_err(|_| GalError::backend("pixel-unpack row length exceeds i32"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::vulkanic::resources::StencilOp;

    #[test]
    fn explicit_front_face_selects_the_matching_opengl_convention() {
        assert!(gl_front_face_is_counter_clockwise(
            FrontFace::CounterClockwise
        ));
        assert!(!gl_front_face_is_counter_clockwise(FrontFace::Clockwise));
    }

    #[test]
    fn explicit_stencil_compare_and_operations_lower_without_java_state() {
        assert_eq!(glow::GREATER, compare_op(CompareOp::Greater));
        assert_eq!(glow::EQUAL, compare_op(CompareOp::Equal));
        assert_eq!(glow::KEEP, stencil_op(StencilOp::Keep));
        assert_eq!(glow::REPLACE, stencil_op(StencilOp::Replace));
    }

    #[test]
    fn overlay_blend_lowers_to_source_alpha_additive_equation() {
        let state = opengl_blend_state(BlendMode::Overlay);
        assert!(state.enabled);
        assert_eq!(glow::FUNC_ADD, state.color_op);
        assert_eq!(glow::FUNC_ADD, state.alpha_op);
        assert_eq!(
            Some(OpenGlBlendFactors {
                src_color: glow::SRC_ALPHA,
                dst_color: glow::ONE,
                src_alpha: glow::ONE,
                dst_alpha: glow::ZERO,
            }),
            state.factors
        );
    }

    #[test]
    fn pixel_unpack_pitch_preserves_declared_padding_and_rejects_partial_pixels() {
        assert_eq!(5, pixel_unpack_row_length(20, 4).unwrap());
        assert_eq!(3, pixel_unpack_row_length(24, 8).unwrap());
        assert!(pixel_unpack_row_length(17, 4).is_err());
        assert!(pixel_unpack_row_length(0, 4).is_err());
        assert!(pixel_unpack_row_length(4, 0).is_err());
        assert!(pixel_unpack_row_length(u32::MAX, 1).is_err());
    }

    #[test]
    fn texture_copy_preserves_top_left_coordinates_across_mips() {
        assert_eq!(
            20,
            gl_y_for_texture_copy_values(TextureDimension::D2, 64, 1, 5, 7).unwrap()
        );
        assert_eq!(
            5,
            gl_y_for_texture_copy_values(TextureDimension::D3, 64, 1, 5, 7).unwrap()
        );
        assert!(gl_y_for_texture_copy_values(TextureDimension::D2, 16, 0, 15, 2).is_err());
    }

    #[test]
    fn glint_blend_matches_frozen_rgb_addition_and_preserves_destination_alpha() {
        let state=opengl_blend_state(BlendMode::Glint);
        assert!(state.enabled);
        assert_eq!(glow::FUNC_ADD,state.color_op);
        assert_eq!(glow::FUNC_ADD,state.alpha_op);
        assert_eq!(Some(OpenGlBlendFactors {
            src_color:glow::SRC_COLOR,dst_color:glow::ONE,
            src_alpha:glow::ZERO,dst_alpha:glow::ONE,
        }),state.factors);
    }

    #[test]
    fn multiply_blend_lowers_to_single_source_times_destination() {
        let state = opengl_blend_state(BlendMode::Multiply);
        assert!(state.enabled);
        assert_eq!(glow::FUNC_ADD, state.color_op);
        assert_eq!(glow::FUNC_ADD, state.alpha_op);
        assert_eq!(
            Some(OpenGlBlendFactors {
                src_color: glow::DST_COLOR,
                dst_color: glow::ZERO,
                src_alpha: glow::ONE,
                dst_alpha: glow::ZERO,
            }),
            state.factors
        );
    }

    #[test]
    fn premultiplied_blend_lowers_to_one_times_destination_alpha() {
        let state = opengl_blend_state(BlendMode::Premultiplied);
        assert!(state.enabled);
        assert_eq!(glow::FUNC_ADD, state.color_op);
        assert_eq!(glow::FUNC_ADD, state.alpha_op);
        assert_eq!(
            Some(OpenGlBlendFactors {
                src_color: glow::ONE,
                dst_color: glow::ONE_MINUS_SRC_ALPHA,
                src_alpha: glow::ONE,
                dst_alpha: glow::ONE_MINUS_SRC_ALPHA,
            }),
            state.factors
        );
    }

    #[test]
    fn sampled_texture_views_reject_conflicting_mip_ranges() {
        let texture = Handle::from_raw(0x100);
        let mut views = BTreeMap::new();
        record_sampled_texture_view_range(&mut views, texture, (1, 1, 0, 1)).unwrap();
        record_sampled_texture_view_range(&mut views, texture, (1, 1, 0, 1)).unwrap();
        let error = record_sampled_texture_view_range(&mut views, texture, (0, 2, 0, 1))
            .expect_err("one OpenGL draw cannot safely bind two incompatible mip views");
        assert!(error.message.contains("incompatible mip/layer views"));
    }
}
