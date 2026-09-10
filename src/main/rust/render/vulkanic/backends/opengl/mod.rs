mod context;
mod lowering;
#[cfg(test)]
pub(in crate::render::vulkanic) mod renderdoc;
mod resources;
mod trace;

use std::sync::{Mutex, MutexGuard};

#[cfg(test)]
use glow::HasContext;

use super::{
    graphics_backend_lock, opengl_capabilities, presentation_capabilities, Backend,
    BackendCreateDesc, BackendRuntimeMetrics, BackendToken, CompletedHostRead,
};
use crate::render::vulkanic::commands::ValidatedSubmissionBatch;
use crate::render::vulkanic::error::{GalError, GalResult};
use crate::render::vulkanic::frame::{
    AcquiredFrame, FrameAcquireDesc, FrameAcquireStatus, FrameId, FramePresentStatus,
    FrameRenderTargetId, FrameResizeDesc, FrameResizeResult, FrameSurfaceDesc, PresentFrameDesc,
    PresentedFrame,
};
use crate::render::vulkanic::handles::{Handle, HandleKind};
use crate::render::vulkanic::resources::BackendCapabilities;
use crate::render::vulkanic::sync::SubmissionId;

use self::context::{ExistingOpenGlContextDesc, OpenGlContext};
use self::lowering::OpenGlLowerer;
#[cfg(test)]
use self::lowering::OpenGlSyncStats;
#[cfg(test)]
use self::lowering::StateCacheSnapshot;
use self::resources::OpenGlObjects;

pub(in crate::render::vulkanic) struct OpenGlBackend {
    context: OpenGlContext,
    objects: OpenGlObjects,
    lowerer: Mutex<OpenGlLowerer>,
    presentation: Option<OpenGlPresentationState>,
    _global_lock: MutexGuard<'static, ()>,
}

struct OpenGlPresentationState {
    desc: FrameSurfaceDesc,
    next_frame: u64,
    acquired: Vec<FrameId>,
}

#[allow(dead_code)]
impl OpenGlBackend {
    pub(in crate::render::vulkanic) fn new(label: &str) -> GalResult<Self> {
        let _zone = trace::Zone::new("opengl.backend.create");
        let global_lock = graphics_backend_lock()
            .lock()
            .map_err(|_| GalError::backend("OpenGL backend global lock poisoned"))?;
        let context = OpenGlContext::new(label)?;
        let objects = OpenGlObjects::new(context.gl().clone());
        Ok(Self {
            lowerer: Mutex::new(OpenGlLowerer::new(context.gl().clone(), context.provoking_vertex, context.clip_control)),
            context,
            objects,
            presentation: None,
            _global_lock: global_lock,
        })
    }

    pub(in crate::render::vulkanic::backends) fn from_existing_context(
        desc: ExistingOpenGlContextDesc,
    ) -> GalResult<Self> {
        let _zone = trace::Zone::new("opengl.backend.borrow-existing-context");
        let global_lock = graphics_backend_lock()
            .lock()
            .map_err(|_| GalError::backend("OpenGL backend global lock poisoned"))?;
        let context = OpenGlContext::from_existing_context(desc)?;
        let objects = OpenGlObjects::new(context.gl().clone());
        Ok(Self {
            lowerer: Mutex::new(OpenGlLowerer::new(context.gl().clone(), context.provoking_vertex, context.clip_control)),
            context,
            objects,
            presentation: Some(OpenGlPresentationState {
                desc: FrameSurfaceDesc {
                    label: "unconfigured-existing-opengl-surface".to_string(),
                    extent: crate::render::vulkanic::resources::Extent3d {
                        width: 1,
                        height: 1,
                        depth: 1,
                    },
                    color_format: crate::render::vulkanic::resources::TextureFormat::Rgba8Unorm,
                    present_mode: crate::render::vulkanic::frame::PresentMode::Fifo,
                    max_frames_in_flight: 1,
                },
                next_frame: 0,
                acquired: Vec::new(),
            }),
            _global_lock: global_lock,
        })
    }

    pub(in crate::render::vulkanic::backends) fn borrowed_minecraft_context(
        label: &str,
        stable_window_id: u64,
    ) -> GalResult<Self> {
        Self::from_existing_context(ExistingOpenGlContextDesc {
            label: label.to_string(),
            stable_window_id,
            render_thread: std::thread::current().id(),
        })
    }

    pub(super) fn completed_host_reads_snapshot(&self) -> Vec<self::lowering::CompletedHostRead> {
        self.lowerer
            .lock()
            .map(|lowerer| lowerer.completed_host_reads_snapshot().to_vec())
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(super) fn completed_host_reads_for_test(&self) -> Vec<self::lowering::CompletedHostRead> {
        self.completed_host_reads_snapshot()
    }

    #[cfg(test)]
    pub(super) fn gl_errors_for_test(&self) -> Vec<String> {
        self.lowerer
            .lock()
            .map(|lowerer| lowerer.gl_errors_for_test().to_vec())
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(super) fn state_cache_for_test(&self) -> StateCacheSnapshot {
        self.lowerer
            .lock()
            .map(|lowerer| lowerer.state_cache_for_test())
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(super) fn sync_stats_for_test(&self) -> OpenGlSyncStats {
        self.lowerer
            .lock()
            .map(|lowerer| lowerer.sync_stats_for_test())
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(super) fn texture_mip_range_for_test(&self, handle: Handle) -> GalResult<(i32, i32)> {
        self.context.make_current()?;
        let _state_guard = self.context.borrowed_state_guard();
        let texture = self.objects.texture(handle)?;
        let target = self::resources::texture_target(texture.dimension);
        unsafe {
            self.context
                .gl()
                .bind_texture(target, Some(texture.texture));
            Ok((
                self.context
                    .gl()
                    .get_tex_parameter_i32(target, glow::TEXTURE_BASE_LEVEL),
                self.context
                    .gl()
                    .get_tex_parameter_i32(target, glow::TEXTURE_MAX_LEVEL),
            ))
        }
    }
}

impl Backend for OpenGlBackend {
    fn capabilities(&self) -> BackendCapabilities {
        let mut capabilities = opengl_capabilities();
        let (max_texture_extent_2d, max_texture_extent_3d) = self.context.texture_extent_limits();
        capabilities.limits.max_texture_extent_2d = capabilities
            .limits
            .max_texture_extent_2d
            .min(max_texture_extent_2d);
        capabilities.limits.max_texture_extent_3d = capabilities
            .limits
            .max_texture_extent_3d
            .min(max_texture_extent_3d);
        capabilities.features.texture_3d &= max_texture_extent_3d != 0;
        capabilities.features.storage_textures = self.context.supports_storage_textures();
        capabilities.features.compute =
            self.context.supports_compute_shaders() && capabilities.features.storage_textures;
        if capabilities.features.compute {
            // OpenGL 4.3 / GLES 3.1 guarantee this minimum dispatch extent.
            // Per-axis device queries can remain backend-private if a future
            // semantic workload needs to approach that bound.
            capabilities.limits.max_dispatch_groups_per_axis = 65_535;
        }
        if self.presentation.is_some() {
            presentation_capabilities(capabilities)
        } else {
            capabilities
        }
    }

    fn create(&mut self, handle: Handle, desc: BackendCreateDesc<'_>) -> GalResult<BackendToken> {
        let _zone = trace::Zone::new("opengl.backend.resource.create");
        self.context.make_current()?;
        let _state_guard = self.context.borrowed_state_guard();
        self.objects.create(handle, desc)
    }

    fn destroy(&mut self, handle: Handle, kind: HandleKind, token: BackendToken) -> GalResult<()> {
        let _zone = trace::Zone::new("opengl.backend.resource.destroy");
        self.context.make_current()?;
        let _state_guard = self.context.borrowed_state_guard();
        self.objects.destroy(handle, kind, token)
    }

    fn encode_passes(&mut self, batch: &ValidatedSubmissionBatch) -> GalResult<()> {
        let _zone = trace::Zone::new("opengl.backend.lowering.encode");
        self.context.make_current()?;
        let _state_guard = self.context.borrowed_state_guard();
        self.lowerer
            .lock()
            .map_err(|_| GalError::backend("OpenGL lowerer lock poisoned"))?
            .encode(batch)
    }

    fn submit(&mut self, id: SubmissionId, _batch: &ValidatedSubmissionBatch) -> GalResult<()> {
        let _zone = trace::Zone::new("opengl.backend.submit");
        trace::message(&format!("gal.submission backend=opengl id={}", id.0));
        self.context.make_current()?;
        let _state_guard = self.context.borrowed_state_guard_with_images(
            self.objects.submission_uses_storage_textures(_batch),
        );
        let mut lowerer = self
            .lowerer
            .lock()
            .map_err(|_| GalError::backend("OpenGL lowerer lock poisoned"))?;
        lowerer.reset_state_cache();
        lowerer.submit(id, &mut self.objects)
    }

    fn completed_submission(&self) -> SubmissionId {
        self.lowerer
            .lock()
            .map(|mut lowerer| lowerer.poll_completed_submission())
            .unwrap_or(SubmissionId(0))
    }

    fn retire(&mut self, completed: SubmissionId) -> GalResult<()> {
        let _zone = trace::Zone::new("opengl.backend.retire");
        self.context.make_current()?;
        let _state_guard = self.context.borrowed_state_guard();
        self.lowerer
            .lock()
            .map_err(|_| GalError::backend("OpenGL lowerer lock poisoned"))?
            .retire(completed)
    }

    fn completed_host_reads(&self) -> Vec<CompletedHostRead> {
        self.completed_host_reads_snapshot()
            .into_iter()
            .map(|read| CompletedHostRead {
                submission: read.submission,
                buffer: read.buffer,
                offset: read.offset,
                bytes: read.bytes,
            })
            .collect()
    }

    fn runtime_metrics(&self) -> BackendRuntimeMetrics {
        let sync = self
            .lowerer
            .lock()
            .map(|lowerer| lowerer.sync_stats_snapshot())
            .unwrap_or_default();
        BackendRuntimeMetrics {
            command_batches: sync.command_batches,
            command_lists: sync.command_lists,
            command_ops: sync.command_ops,
            gl_calls: sync.gl_calls,
            gl_flushes: sync.flushes as u64,
            gl_finishes: sync.finishes as u64,
            gl_fences_inserted: sync.fences_inserted as u64,
            gl_fences_polled: sync.fences_polled as u64,
            gl_fences_waited: sync.fences_waited as u64,
            gl_fences_deleted: sync.fences_deleted as u64,
            ..BackendRuntimeMetrics::default()
        }
    }

    fn configure_frame_surface(&mut self, desc: &FrameSurfaceDesc) -> GalResult<()> {
        let Some(presentation) = &mut self.presentation else {
            return Err(GalError::unsupported_feature(
                "OpenGL presentation requires an explicit borrowed Minecraft context",
            ));
        };
        self.context.make_current()?;
        let _state_guard = self.context.borrowed_state_guard();
        presentation.desc = desc.clone();
        Ok(())
    }

    fn acquire_frame(&mut self, desc: &FrameAcquireDesc) -> GalResult<AcquiredFrame> {
        let Some(presentation) = &mut self.presentation else {
            return Err(GalError::unsupported_feature(
                "OpenGL presentation requires an explicit borrowed Minecraft context",
            ));
        };
        self.context.make_current()?;
        let _state_guard = self.context.borrowed_state_guard();
        presentation.next_frame += 1;
        let frame = FrameId(presentation.next_frame);
        if desc.expected_extent.width == 0 || desc.expected_extent.height == 0 {
            return Ok(AcquiredFrame {
                frame,
                correlation_id: desc.correlation_id,
                status: FrameAcquireStatus::Minimized,
                render_target: FrameRenderTargetId(0),
                extent: desc.expected_extent,
                color_format: presentation.desc.color_format,
            });
        }
        let framebuffer = self.context.current_draw_framebuffer();
        presentation.acquired.push(frame);
        self.objects
            .set_frame_target_framebuffer(frame.0, framebuffer);
        trace::message(&format!(
            "gal.frame.acquire backend=opengl correlation={} frame={}",
            desc.correlation_id.0, frame.0
        ));
        Ok(AcquiredFrame {
            frame,
            correlation_id: desc.correlation_id,
            status: if desc.expected_extent == presentation.desc.extent {
                FrameAcquireStatus::Ready
            } else {
                FrameAcquireStatus::Suboptimal
            },
            render_target: FrameRenderTargetId(frame.0),
            extent: presentation.desc.extent,
            color_format: presentation.desc.color_format,
        })
    }

    fn resize_frame_surface(&mut self, desc: &FrameResizeDesc) -> GalResult<FrameResizeResult> {
        let Some(presentation) = &mut self.presentation else {
            return Err(GalError::unsupported_feature(
                "OpenGL presentation requires an explicit borrowed Minecraft context",
            ));
        };
        self.context.make_current()?;
        let _state_guard = self.context.borrowed_state_guard();
        presentation.desc.extent = desc.extent;
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
        let Some(presentation) = &mut self.presentation else {
            return Err(GalError::unsupported_feature(
                "OpenGL presentation requires an explicit borrowed Minecraft context",
            ));
        };
        if !presentation.acquired.contains(&desc.frame) {
            return Err(GalError::submission(
                crate::render::vulkanic::StatusCode::InvalidArgument,
                "OpenGL frame was not acquired before present",
            ));
        }
        self.context.make_current()?;
        let _state_guard = self.context.borrowed_state_guard();
        let completed_submission = self
            .lowerer
            .lock()
            .map_err(|_| GalError::backend("OpenGL lowerer lock poisoned"))?
            .poll_completed_submission();
        trace::message(&format!(
            "gal.frame.present backend=opengl correlation={} frame={} submission={}",
            desc.correlation_id.0, desc.frame.0, desc.wait_for.0
        ));
        Ok(PresentedFrame {
            frame: desc.frame,
            correlation_id: desc.correlation_id,
            render_target: FrameRenderTargetId(desc.frame.0),
            status: FramePresentStatus::Presented,
            completed_submission,
        })
    }

    fn shutdown_frame_surface(&mut self) -> GalResult<()> {
        if let Some(presentation) = &mut self.presentation {
            presentation.acquired.clear();
        }
        Ok(())
    }

    #[cfg(test)]
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    #[cfg(test)]
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl Drop for OpenGlBackend {
    fn drop(&mut self) {
        let _zone = trace::Zone::new("opengl.backend.drop");
        let _ = self.context.make_current();
        let _state_guard = self.context.borrowed_state_guard();
        if let Ok(mut lowerer) = self.lowerer.lock() {
            lowerer.delete_all_fences();
        }
        self.objects.destroy_all();
    }
}

#[cfg(test)]
pub(in crate::render::vulkanic::backends) mod conformance;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::vulkanic::commands::{
        CommandListDesc, CommandOp, ResourceBarrier, SubmissionBatch, TextureUsageState,
    };
    use crate::render::vulkanic::gal::VulkanicGal;
    use crate::render::vulkanic::resources::{BufferDesc, BufferUsage, MemoryDomain, QueueClass};

    #[test]
    fn opengl_backend_can_bootstrap_or_reports_environment_gap() {
        match OpenGlBackend::new("MattMC OpenGL bootstrap") {
            Ok(mut backend) => {
                let capabilities = backend.capabilities();
                let (max_texture_extent_2d, max_texture_extent_3d) =
                    backend.context.texture_extent_limits();
                assert!(
                    capabilities.limits.max_texture_extent_2d <= max_texture_extent_2d,
                    "OpenGL capability report exceeded the active context's 2D limit"
                );
                assert!(
                    capabilities.limits.max_texture_extent_3d <= max_texture_extent_3d
                        || !capabilities.features.texture_3d,
                    "OpenGL capability report exceeded the active context's 3D limit"
                );
                backend.retire(SubmissionId(0)).unwrap();
            }
            Err(error) => {
                let text = error.to_string();
                assert!(
                    text.contains("OpenGL") || text.contains("EGL") || text.contains("GL"),
                    "unexpected OpenGL bootstrap failure: {text}"
                );
            }
        }
    }

    #[test]
    fn opengl_existing_context_contract_rejects_wrong_render_thread() {
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || sender.send(std::thread::current().id()).unwrap())
            .join()
            .unwrap();
        let other_thread = receiver.recv().unwrap();
        let result = OpenGlBackend::from_existing_context(ExistingOpenGlContextDesc {
            label: "borrowed-context-test".to_string(),
            stable_window_id: 42,
            render_thread: other_thread,
        });
        let error = match result {
            Ok(_) => panic!("wrong-thread borrowed context must be rejected"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains("render thread"),
            "unexpected wrong-thread error: {error}"
        );
    }

    #[test]
    fn opengl_existing_context_contract_accepts_same_render_thread() {
        let owner = match OpenGlContext::new("borrowed-context-owner") {
            Ok(owner) => owner,
            Err(error) => {
                assert!(
                    error.to_string().contains("OpenGL")
                        || error.to_string().contains("EGL")
                        || error.to_string().contains("GLX"),
                    "unexpected OpenGL bootstrap failure: {error}"
                );
                return;
            }
        };
        owner
            .make_current()
            .expect("isolated owner context should be current for borrowed-context test");
        let backend = match OpenGlBackend::from_existing_context(ExistingOpenGlContextDesc {
            label: "borrowed-context-same-thread".to_string(),
            stable_window_id: 84,
            render_thread: std::thread::current().id(),
        }) {
            Ok(backend) => backend,
            Err(error) => {
                assert!(
                    error.to_string().contains("OpenGL")
                        || error.to_string().contains("GL")
                        || error.to_string().contains("context"),
                    "unexpected same-thread borrowed-context failure: {error}"
                );
                return;
            }
        };
        let mut gal = VulkanicGal::new_with_backend(Box::new(backend), false);
        let surface = crate::render::vulkanic::frame::FrameSurfaceDesc {
            label: "borrowed-opengl-window".to_string(),
            extent: crate::render::vulkanic::resources::Extent3d {
                width: 64,
                height: 48,
                depth: 1,
            },
            color_format: crate::render::vulkanic::resources::TextureFormat::Rgba8Unorm,
            present_mode: crate::render::vulkanic::frame::PresentMode::Fifo,
            max_frames_in_flight: 2,
        };
        gal.configure_frame_surface(surface)
            .expect("borrowed OpenGL frame surface should configure");
        let acquired = gal
            .acquire_frame(crate::render::vulkanic::frame::FrameAcquireDesc {
                correlation_id: crate::render::vulkanic::frame::FrameCorrelationId(1),
                expected_extent: crate::render::vulkanic::resources::Extent3d {
                    width: 64,
                    height: 48,
                    depth: 1,
                },
            })
            .expect("borrowed OpenGL frame should acquire");
        assert_eq!(
            acquired.status,
            crate::render::vulkanic::frame::FrameAcquireStatus::Ready
        );
        gal.present_frame(crate::render::vulkanic::frame::PresentFrameDesc {
            frame: acquired.frame,
            correlation_id: acquired.correlation_id,
            wait_for: SubmissionId(0),
        })
        .expect("borrowed OpenGL frame should present");
        gal.shutdown_frame_surface()
            .expect("borrowed OpenGL shutdown should not own the external context");
        drop(gal);
        drop(owner);
    }

    #[test]
    fn opengl_backend_consumes_owned_copy_submission() {
        let backend = match OpenGlBackend::new("MattMC OpenGL copy") {
            Ok(backend) => backend,
            Err(error) => {
                assert!(
                    error.to_string().contains("OpenGL") || error.to_string().contains("EGL"),
                    "unexpected OpenGL bootstrap failure: {error}"
                );
                return;
            }
        };
        let mut gal = VulkanicGal::new_with_backend(Box::new(backend), false);
        let src = gal
            .create_buffer(BufferDesc {
                label: "copy-src".to_string(),
                size: 16,
                memory: MemoryDomain::Upload,
                usages: vec![BufferUsage::TransferSrc, BufferUsage::HostWrite],
            })
            .unwrap();
        let dst = gal
            .create_buffer(BufferDesc {
                label: "copy-dst".to_string(),
                size: 16,
                memory: MemoryDomain::Readback,
                usages: vec![BufferUsage::TransferDst, BufferUsage::HostRead],
            })
            .unwrap();
        let list = gal
            .create_command_list(CommandListDesc {
                label: "copy".to_string(),
                operations: vec![
                    CommandOp::HostWriteBuffer {
                        buffer: src,
                        offset: 0,
                        data: vec![7; 16],
                    },
                    buffer_barrier(src),
                    CommandOp::CopyBuffer { src, dst, size: 16 },
                    buffer_barrier(dst),
                    CommandOp::HostReadBuffer {
                        buffer: dst,
                        offset: 0,
                        size: 16,
                    },
                ],
            })
            .unwrap();
        let token = gal
            .submit(SubmissionBatch {
                label: "copy-submit".to_string(),
                command_lists: vec![list],
            })
            .unwrap();
        gal.retire_through_for_test(token.submission).unwrap();
        let reads = gal
            .opengl_backend()
            .unwrap()
            .completed_host_reads_for_test();
        assert_eq!(reads.last().unwrap().bytes, vec![7; 16]);
    }

    fn buffer_barrier(resource: Handle) -> CommandOp {
        CommandOp::Barrier(ResourceBarrier {
            resource,
            subresources: None,
            before: TextureUsageState::TransferDst,
            after: TextureUsageState::TransferSrc,
            src_queue: QueueClass::Graphics,
            dst_queue: QueueClass::Graphics,
        })
    }
}
