// This module is intentionally private to render::vulkanic. Rust backend
// implementations must stay behind the Vulkanic frontend boundary, matching the
// Java rule that non-Vulkanic code cannot import net.vulkanic.backends.*.
pub(super) mod opengl;
pub(super) mod vulkan;

#[cfg(test)]
mod conformance_matrix;

use std::sync::{Mutex, OnceLock};

use super::commands::ValidatedSubmissionBatch;
use super::error::{GalError, GalResult};
use super::frame::{
    AcquiredFrame, FrameAcquireDesc, FrameId, FrameResizeDesc, FrameResizeResult, FrameSurfaceDesc,
    PresentFrameDesc, PresentedFrame,
};
use super::handles::{Handle, HandleKind};
use super::resources::{
    BackendApi, BackendCapabilities, BackendFeatureFlags, BackendLimits, BufferDesc,
    CombinedTextureSamplerDesc, ComputePipelineDesc, FrameTargetDesc, GraphicsPipelineDesc,
    PipelineLayoutDesc, RenderPassDesc, RenderTargetDesc, ResourceLayoutDesc, ResourceSetDesc,
    SamplerDesc, ShaderModuleDesc, TextureDesc, TextureViewDesc,
};
use super::sync::SubmissionId;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct BackendToken(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::render::vulkanic) enum BackendKind {
    Vulkan,
    OpenGl,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::render::vulkanic) struct CompletedHostRead {
    pub(in crate::render::vulkanic) submission: SubmissionId,
    pub(in crate::render::vulkanic) buffer: Handle,
    pub(in crate::render::vulkanic) offset: u64,
    pub(in crate::render::vulkanic) bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::render::vulkanic) struct BackendRuntimeMetrics {
    pub(in crate::render::vulkanic) command_batches: u64,
    pub(in crate::render::vulkanic) command_lists: u64,
    pub(in crate::render::vulkanic) command_ops: u64,
    pub(in crate::render::vulkanic) gl_calls: u64,
    pub(in crate::render::vulkanic) gl_flushes: u64,
    pub(in crate::render::vulkanic) gl_finishes: u64,
    pub(in crate::render::vulkanic) gl_fences_inserted: u64,
    pub(in crate::render::vulkanic) gl_fences_polled: u64,
    pub(in crate::render::vulkanic) gl_fences_waited: u64,
    pub(in crate::render::vulkanic) gl_fences_deleted: u64,
    pub(in crate::render::vulkanic) vulkan_command_buffer_alloc_nanos: u64,
    pub(in crate::render::vulkanic) vulkan_command_buffer_begin_nanos: u64,
    pub(in crate::render::vulkanic) vulkan_command_recording_nanos: u64,
    pub(in crate::render::vulkanic) vulkan_command_buffer_end_nanos: u64,
    pub(in crate::render::vulkanic) vulkan_queue_submit_nanos: u64,
    pub(in crate::render::vulkanic) vulkan_timeline_poll_nanos: u64,
    pub(in crate::render::vulkanic) vulkan_timeline_wait_nanos: u64,
    pub(in crate::render::vulkanic) vulkan_device_wait_idle_nanos: u64,
    pub(in crate::render::vulkanic) vulkan_acquire_nanos: u64,
    pub(in crate::render::vulkanic) vulkan_present_nanos: u64,
    pub(in crate::render::vulkanic) vulkan_present_wait_nanos: u64,
    pub(in crate::render::vulkanic) vulkan_command_buffers_allocated: u64,
    pub(in crate::render::vulkanic) vulkan_command_buffers_freed: u64,
    pub(in crate::render::vulkanic) vulkan_wait_count: u64,
    pub(in crate::render::vulkanic) vulkan_device_wait_idle_count: u64,
    pub(in crate::render::vulkanic) vulkan_present_mode: u64,
    pub(in crate::render::vulkanic) vulkan_requested_present_mode: u64,
    pub(in crate::render::vulkanic) vulkan_supported_present_modes: u64,
    pub(in crate::render::vulkanic) vulkan_present_mode_fallback_reason: u64,
    pub(in crate::render::vulkanic) vulkan_acquired_image_index: u64,
    pub(in crate::render::vulkanic) vulkan_swapchain_generation: u64,
    pub(in crate::render::vulkanic) vulkan_swapchain_image_count: u64,
    pub(in crate::render::vulkanic) vulkan_surface_min_image_count: u64,
    pub(in crate::render::vulkanic) vulkan_surface_max_image_count: u64,
    pub(in crate::render::vulkanic) vulkan_configured_frames_in_flight: u64,
    pub(in crate::render::vulkanic) vulkan_images_in_flight: u64,
    pub(in crate::render::vulkanic) vulkan_available_frame_slots: u64,
    pub(in crate::render::vulkanic) gpu_timestamp_status: u64,
    pub(in crate::render::vulkanic) gpu_shadow_depth_nanos: u64,
    pub(in crate::render::vulkanic) gpu_terrain_opaque_nanos: u64,
    pub(in crate::render::vulkanic) gpu_terrain_cutout_nanos: u64,
    pub(in crate::render::vulkanic) gpu_deferred_lighting_nanos: u64,
    pub(in crate::render::vulkanic) gpu_composite0_nanos: u64,
    pub(in crate::render::vulkanic) gpu_composite1_nanos: u64,
    pub(in crate::render::vulkanic) gpu_final_output_nanos: u64,
    pub(in crate::render::vulkanic) gpu_frame_total_nanos: u64,
}

pub(super) fn graphics_backend_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub(in crate::render::vulkanic) fn create_backend(
    kind: BackendKind,
    label: &str,
) -> GalResult<Box<dyn Backend>> {
    match kind {
        BackendKind::Vulkan => Ok(Box::new(vulkan::VulkanBackend::new(label)?)),
        BackendKind::OpenGl => Ok(Box::new(opengl::OpenGlBackend::new(label)?)),
    }
}

pub(in crate::render::vulkanic) fn create_borrowed_opengl_backend(
    label: &str,
    stable_window_id: u64,
) -> GalResult<Box<dyn Backend>> {
    Ok(Box::new(opengl::OpenGlBackend::borrowed_minecraft_context(
        label,
        stable_window_id,
    )?))
}

pub(in crate::render::vulkanic) fn create_native_windowed_vulkan_backend(
    label: &str,
    platform: u32,
    stable_window_id: u64,
    native_display: u64,
    native_window: u64,
    surface_desc: FrameSurfaceDesc,
) -> GalResult<Box<dyn Backend>> {
    Ok(Box::new(vulkan::VulkanBackend::new_native_windowed(
        label,
        platform,
        stable_window_id,
        native_display,
        native_window,
        surface_desc,
    )?))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum BackendCreateDesc<'a> {
    Buffer(&'a BufferDesc),
    Texture(&'a TextureDesc),
    TextureView(&'a TextureViewDesc),
    Sampler(&'a SamplerDesc),
    CombinedTextureSampler(&'a CombinedTextureSamplerDesc),
    ShaderModule(&'a ShaderModuleDesc),
    ResourceLayout(&'a ResourceLayoutDesc),
    ResourceSet(&'a ResourceSetDesc),
    PipelineLayout(&'a PipelineLayoutDesc),
    GraphicsPipeline(&'a GraphicsPipelineDesc),
    ComputePipeline(&'a ComputePipelineDesc),
    RenderTarget(&'a RenderTargetDesc),
    FrameTarget(&'a FrameTargetDesc),
    RenderPass(&'a RenderPassDesc),
}

pub(super) trait Backend {
    fn capabilities(&self) -> BackendCapabilities;
    fn create(&mut self, handle: Handle, desc: BackendCreateDesc<'_>) -> GalResult<BackendToken>;
    fn destroy(&mut self, handle: Handle, kind: HandleKind, token: BackendToken) -> GalResult<()>;
    fn encode_passes(&mut self, batch: &ValidatedSubmissionBatch) -> GalResult<()>;
    fn submit(&mut self, id: SubmissionId, batch: &ValidatedSubmissionBatch) -> GalResult<()>;
    fn completed_submission(&self) -> SubmissionId;
    fn retire(&mut self, completed: SubmissionId) -> GalResult<()>;
    fn completed_host_reads(&self) -> Vec<CompletedHostRead> {
        Vec::new()
    }
    fn runtime_metrics(&self) -> BackendRuntimeMetrics {
        BackendRuntimeMetrics::default()
    }
    fn configure_frame_surface(&mut self, _desc: &FrameSurfaceDesc) -> GalResult<()> {
        Err(GalError::unsupported_feature(
            "backend was not created with a presentation surface",
        ))
    }
    fn acquire_frame(&mut self, _desc: &FrameAcquireDesc) -> GalResult<AcquiredFrame> {
        Err(GalError::unsupported_feature(
            "backend was not created with a presentation surface",
        ))
    }
    fn resize_frame_surface(&mut self, _desc: &FrameResizeDesc) -> GalResult<FrameResizeResult> {
        Err(GalError::unsupported_feature(
            "backend was not created with a presentation surface",
        ))
    }
    fn present_frame(&mut self, _desc: &PresentFrameDesc) -> GalResult<PresentedFrame> {
        Err(GalError::unsupported_feature(
            "backend was not created with a presentation surface",
        ))
    }
    fn cancel_frame(&mut self, _frame: FrameId) -> GalResult<()> {
        Err(GalError::unsupported_feature(
            "backend does not support cancelling an acquired presentation frame",
        ))
    }
    fn shutdown_frame_surface(&mut self) -> GalResult<()> {
        Ok(())
    }

    #[cfg(test)]
    fn as_any(&self) -> &dyn std::any::Any;

    #[cfg(test)]
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}

pub(super) fn vulkan_capabilities() -> BackendCapabilities {
    BackendCapabilities {
        api: BackendApi::Vulkan,
        name: "Rust Vulkan",
        features: BackendFeatureFlags {
            graphics: true,
            compute: true,
            descriptor_arrays: true,
            optional_bindings: true,
            dynamic_buffer_offsets: true,
            uniform_buffers: true,
            storage_buffers: true,
            storage_textures: true,
            indirect_draw: true,
            indirect_dispatch: true,
            multiple_color_attachments: true,
            depth_only_pass: true,
            blended_pass: true,
            texture_subresource_copies: true,
            texture_mip_levels: true,
            texture_array_layers: true,
            host_buffer_access: true,
            presentation: false,
            renderdoc_capture: true,
            tracy_zones: true,
            texture_3d: true,
            texture_row_reversal: true,
        },
        limits: BackendLimits {
            max_buffer_size: 256 * 1024 * 1024,
            max_texture_extent_2d: 8192,
            max_texture_extent_3d: 2048,
            max_texture_mip_levels: 13,
            max_texture_array_layers: 64,
            max_resource_layout_bindings: 32,
            max_binding_array_count: 16,
            max_color_attachments: 4,
            max_dynamic_offsets_per_binding: 8,
            max_command_lists_per_submission: 64,
            max_commands_per_list: 16_384,
            max_draw_count: 4096,
            max_dispatch_groups_per_axis: 65_535,
        },
    }
}

pub(super) fn presentation_capabilities(
    mut capabilities: BackendCapabilities,
) -> BackendCapabilities {
    capabilities.features.presentation = true;
    capabilities
}

pub(super) fn opengl_capabilities() -> BackendCapabilities {
    BackendCapabilities {
        api: BackendApi::OpenGl,
        name: "Rust OpenGL",
        features: BackendFeatureFlags {
            graphics: true,
            compute: false,
            descriptor_arrays: true,
            optional_bindings: true,
            dynamic_buffer_offsets: true,
            uniform_buffers: true,
            storage_buffers: true,
            storage_textures: false,
            indirect_draw: false,
            indirect_dispatch: false,
            multiple_color_attachments: true,
            depth_only_pass: true,
            blended_pass: true,
            texture_subresource_copies: true,
            texture_mip_levels: true,
            texture_array_layers: false,
            host_buffer_access: true,
            presentation: false,
            renderdoc_capture: true,
            tracy_zones: true,
            texture_3d: true,
            texture_row_reversal: false,
        },
        limits: BackendLimits {
            max_buffer_size: 64 * 1024 * 1024,
            max_texture_extent_2d: 4096,
            // The isolated GL path supports D3 allocation, explicit mips, box
            // upload/readback, and sampled binding. Storage images remain
            // independently capability-gated by the current context.
            max_texture_extent_3d: 2048,
            max_texture_mip_levels: 13,
            max_texture_array_layers: 1,
            max_resource_layout_bindings: 16,
            max_binding_array_count: 8,
            max_color_attachments: 4,
            max_dynamic_offsets_per_binding: 4,
            max_command_lists_per_submission: 32,
            max_commands_per_list: 8192,
            max_draw_count: 1,
            max_dispatch_groups_per_axis: 0,
        },
    }
}

#[cfg(test)]
pub(super) mod mock {
    use std::collections::{BTreeMap, VecDeque};

    use super::*;
    use crate::render::vulkanic::error::GalError;
    use crate::render::vulkanic::frame::{
        FrameAcquireStatus, FrameId, FramePresentStatus, FrameRenderTargetId,
    };

    #[derive(Default)]
    pub(in crate::render::vulkanic) struct MockBackend {
        pub(in crate::render::vulkanic) creates: Vec<(Handle, HandleKind)>,
        pub(in crate::render::vulkanic) destroys: Vec<(Handle, HandleKind)>,
        pub(in crate::render::vulkanic) submissions: Vec<SubmissionId>,
        pub(in crate::render::vulkanic) encoded_batches: usize,
        pub(in crate::render::vulkanic) completed: SubmissionId,
        pub(in crate::render::vulkanic) fail_next_create: bool,
        pub(in crate::render::vulkanic) fail_next_submit: bool,
        pub(in crate::render::vulkanic) fail_retire_at: Option<SubmissionId>,
        pub(in crate::render::vulkanic) retire_requests: Vec<SubmissionId>,
        pub(in crate::render::vulkanic) capabilities: Option<BackendCapabilities>,
        pub(in crate::render::vulkanic) live: BTreeMap<Handle, BackendToken>,
        next_token: u64,
        pub(in crate::render::vulkanic) submitted_labels: VecDeque<String>,
        pub(in crate::render::vulkanic) frame_surface: Option<FrameSurfaceDesc>,
        pub(in crate::render::vulkanic) acquired_frames: Vec<FrameId>,
        pub(in crate::render::vulkanic) presented_frames: Vec<FrameId>,
        pub(in crate::render::vulkanic) next_frame: u64,
        pub(in crate::render::vulkanic) minimized: bool,
    }

    impl MockBackend {
        pub(in crate::render::vulkanic) fn fail_next_create(&mut self) {
            self.fail_next_create = true;
        }

        #[cfg(test)]
        pub(in crate::render::vulkanic) fn fail_next_submit(&mut self) {
            self.fail_next_submit = true;
        }

        pub(in crate::render::vulkanic) fn complete_through(&mut self, id: SubmissionId) {
            self.completed = id;
        }

        pub(in crate::render::vulkanic) fn with_capabilities(
            capabilities: BackendCapabilities,
        ) -> Self {
            Self {
                capabilities: Some(capabilities),
                ..Self::default()
            }
        }
    }

    impl Backend for MockBackend {
        fn capabilities(&self) -> BackendCapabilities {
            self.capabilities.unwrap_or_else(vulkan_capabilities)
        }

        fn create(
            &mut self,
            handle: Handle,
            desc: BackendCreateDesc<'_>,
        ) -> GalResult<BackendToken> {
            if self.fail_next_create {
                self.fail_next_create = false;
                return Err(GalError::backend("mock create failure"));
            }
            let kind = match desc {
                BackendCreateDesc::Buffer(_) => HandleKind::Buffer,
                BackendCreateDesc::Texture(_) => HandleKind::Texture,
                BackendCreateDesc::TextureView(_) => HandleKind::TextureView,
                BackendCreateDesc::Sampler(_) => HandleKind::Sampler,
                BackendCreateDesc::CombinedTextureSampler(_) => HandleKind::CombinedTextureSampler,
                BackendCreateDesc::ShaderModule(_) => HandleKind::ShaderModule,
                BackendCreateDesc::ResourceLayout(_) => HandleKind::ResourceLayout,
                BackendCreateDesc::ResourceSet(_) => HandleKind::ResourceSet,
                BackendCreateDesc::PipelineLayout(_) => HandleKind::PipelineLayout,
                BackendCreateDesc::GraphicsPipeline(_) => HandleKind::GraphicsPipeline,
                BackendCreateDesc::ComputePipeline(_) => HandleKind::ComputePipeline,
                BackendCreateDesc::RenderTarget(_) => HandleKind::RenderTarget,
                BackendCreateDesc::FrameTarget(_) => HandleKind::FrameTarget,
                BackendCreateDesc::RenderPass(_) => HandleKind::RenderPass,
            };
            self.next_token += 1;
            let token = BackendToken(self.next_token);
            self.live.insert(handle, token);
            self.creates.push((handle, kind));
            Ok(token)
        }

        fn destroy(
            &mut self,
            handle: Handle,
            kind: HandleKind,
            token: BackendToken,
        ) -> GalResult<()> {
            if self.live.remove(&handle) != Some(token) {
                return Err(GalError::backend("mock destroy for unknown token"));
            }
            self.destroys.push((handle, kind));
            Ok(())
        }

        fn encode_passes(&mut self, batch: &ValidatedSubmissionBatch) -> GalResult<()> {
            self.encoded_batches += batch.command_lists.len();
            Ok(())
        }

        fn submit(&mut self, id: SubmissionId, batch: &ValidatedSubmissionBatch) -> GalResult<()> {
            if self.fail_next_submit {
                self.fail_next_submit = false;
                return Err(GalError::backend("mock submit failure"));
            }
            self.submissions.push(id);
            self.submitted_labels.push_back(batch.label.clone());
            Ok(())
        }

        fn completed_submission(&self) -> SubmissionId {
            self.completed
        }

        fn retire(&mut self, completed: SubmissionId) -> GalResult<()> {
            self.retire_requests.push(completed);
            if self.fail_retire_at == Some(completed) {
                self.fail_retire_at = None;
                return Err(GalError::backend("mock completion wait failure"));
            }
            Ok(())
        }

        fn configure_frame_surface(&mut self, desc: &FrameSurfaceDesc) -> GalResult<()> {
            self.frame_surface = Some(desc.clone());
            Ok(())
        }

        fn acquire_frame(&mut self, desc: &FrameAcquireDesc) -> GalResult<AcquiredFrame> {
            let surface = self.frame_surface.as_ref().ok_or_else(|| {
                GalError::unsupported_feature("mock backend has no configured frame surface")
            })?;
            self.next_frame += 1;
            let frame = FrameId(self.next_frame);
            if self.minimized || desc.expected_extent.width == 0 || desc.expected_extent.height == 0
            {
                return Ok(AcquiredFrame {
                    frame,
                    correlation_id: desc.correlation_id,
                    status: FrameAcquireStatus::Minimized,
                    render_target: FrameRenderTargetId(0),
                    extent: desc.expected_extent,
                    color_format: surface.color_format,
                });
            }
            self.acquired_frames.push(frame);
            Ok(AcquiredFrame {
                frame,
                correlation_id: desc.correlation_id,
                status: if desc.expected_extent == surface.extent {
                    FrameAcquireStatus::Ready
                } else {
                    FrameAcquireStatus::Suboptimal
                },
                render_target: FrameRenderTargetId(frame.0),
                extent: surface.extent,
                color_format: surface.color_format,
            })
        }

        fn resize_frame_surface(&mut self, desc: &FrameResizeDesc) -> GalResult<FrameResizeResult> {
            let surface = self.frame_surface.as_mut().ok_or_else(|| {
                GalError::unsupported_feature("mock backend has no configured frame surface")
            })?;
            surface.extent = desc.extent;
            Ok(FrameResizeResult {
                status: if desc.extent.width == 0 || desc.extent.height == 0 {
                    FrameAcquireStatus::Minimized
                } else {
                    FrameAcquireStatus::Resized
                },
                extent: desc.extent,
            })
        }

        fn present_frame(&mut self, desc: &PresentFrameDesc) -> GalResult<PresentedFrame> {
            if !self.acquired_frames.contains(&desc.frame) {
                return Err(GalError::submission(
                    crate::render::vulkanic::StatusCode::InvalidArgument,
                    "presented frame was not acquired",
                ));
            }
            self.presented_frames.push(desc.frame);
            Ok(PresentedFrame {
                frame: desc.frame,
                correlation_id: desc.correlation_id,
                render_target: FrameRenderTargetId(desc.frame.0),
                status: FramePresentStatus::Presented,
                completed_submission: desc.wait_for,
            })
        }

        fn cancel_frame(&mut self, frame: FrameId) -> GalResult<()> {
            let Some(index) = self
                .acquired_frames
                .iter()
                .position(|candidate| *candidate == frame)
            else {
                return Err(GalError::submission(
                    crate::render::vulkanic::StatusCode::InvalidArgument,
                    "cancelled frame was not acquired",
                ));
            };
            self.acquired_frames.remove(index);
            Ok(())
        }

        fn shutdown_frame_surface(&mut self) -> GalResult<()> {
            self.frame_surface = None;
            Ok(())
        }

        fn as_any(&self) -> &dyn std::any::Any {
            self
        }

        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }
}
