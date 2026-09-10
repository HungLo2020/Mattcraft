use std::fs;
use std::mem::{align_of, size_of};
use std::path::Path;
use std::ptr::NonNull;

use super::backends::{mock::MockBackend, opengl_capabilities, vulkan_capabilities};
use super::commands::*;
use super::error::{ErrorDomain, GalError, StatusCode};
use super::ffi::*;
use super::frame::*;
use super::gal::{normalize_submission_batch, CommandNormalizationStats, VulkanicGal};
use super::handles::{Handle, HandleKind, MAX_GENERATION};
use super::metrics::{Metrics, WholeFrameProfile};
use super::resources::*;
use super::sync::SubmissionId;

fn gal() -> VulkanicGal {
    VulkanicGal::new_with_backend(Box::new(MockBackend::default()), false)
}

#[test]
fn completion_wait_rejects_unsubmitted_ids_without_fabricating_progress() {
    let mut gal = gal();
    assert!(gal.retire_through(SubmissionId(1)).is_err());
    assert_eq!(gal.poll_completed(), SubmissionId(0));
    assert!(gal.mock_backend().unwrap().retire_requests.is_empty());
    let batch = || SubmissionBatch {
        label: "completion-failure".into(),
        command_lists: vec![CommandList::from(CommandListDesc {
            label: "empty-but-valid-command-list".into(), operations: vec![],
        })],
    };
    gal.mock_backend_mut().unwrap().fail_next_submit = true;
    assert!(gal.submit(batch()).is_err());
    assert_eq!(gal.next_submission_id(), SubmissionId(2));
    assert_eq!(gal.latest_submission_id(), SubmissionId(0), "a failed attempt is not an accepted receipt");
    assert!(gal.retire_through(SubmissionId(1)).is_err());
    assert!(gal.mock_backend().unwrap().retire_requests.is_empty());
    let token = gal.submit(batch()).unwrap();
    assert_eq!(token.submission, SubmissionId(2));
    assert_eq!(gal.latest_submission_id(), token.submission);
    gal.retire_through(token.submission).unwrap();
    assert_eq!(gal.poll_completed(), token.submission);
}

#[test]
fn same_usage_write_barriers_are_dependencies_but_read_only_noops_are_rejected() {
    let mut gal = gal();
    let buffer = gal.create_buffer(BufferDesc {
        label: "same-usage-dependency".into(), size: 4, memory: MemoryDomain::Upload,
        usages: vec![BufferUsage::TransferSrc, BufferUsage::TransferDst, BufferUsage::HostWrite],
    }).unwrap();
    let barrier = |before, after| CommandOp::Barrier(ResourceBarrier {
        resource: buffer, subresources: None, before, after,
        src_queue: QueueClass::Graphics, dst_queue: QueueClass::Graphics,
    });
    let batch = |operations| SubmissionBatch {
        label: "same-usage-dependency".into(),
        command_lists: vec![CommandList::from(CommandListDesc { label: "commands".into(), operations })],
    };
    gal.submit(batch(vec![
        CommandOp::HostWriteBuffer { buffer, offset: 0, data: vec![1, 2, 3, 4] },
        barrier(TextureUsageState::TransferDst, TextureUsageState::TransferDst),
        CommandOp::HostWriteBuffer { buffer, offset: 0, data: vec![5, 6, 7, 8] },
        barrier(TextureUsageState::TransferDst, TextureUsageState::TransferSrc),
    ])).unwrap();
    assert!(gal.submit(batch(vec![barrier(TextureUsageState::TransferSrc, TextureUsageState::TransferSrc)])).is_err());
}

fn gal_with_capabilities(capabilities: BackendCapabilities) -> VulkanicGal {
    VulkanicGal::new_with_backend(
        Box::new(MockBackend::with_capabilities(capabilities)),
        false,
    )
}

fn presentation_capabilities() -> BackendCapabilities {
    let mut capabilities = vulkan_capabilities();
    capabilities.features.presentation = true;
    capabilities
}

fn frame_surface(label: &str) -> FrameSurfaceDesc {
    FrameSurfaceDesc {
        label: label.to_owned(),
        extent: Extent3d {
            width: 128,
            height: 72,
            depth: 1,
        },
        color_format: TextureFormat::Rgba8Unorm,
        present_mode: PresentMode::Fifo,
        max_frames_in_flight: 2,
    }
}

fn assert_code<T>(result: Result<T, GalError>, code: super::StatusCode) {
    let error = match result {
        Ok(_) => panic!("operation unexpectedly succeeded"),
        Err(error) => error,
    };
    assert_eq!(error.code, code, "{error}");
}

fn assert_unsupported<T>(result: Result<T, GalError>, needle: &str) {
    let error = match result {
        Ok(_) => panic!("operation unexpectedly succeeded"),
        Err(error) => error,
    };
    assert_eq!(error.code, super::StatusCode::UnsupportedFeature, "{error}");
    assert!(
        error.message.contains(needle),
        "unsupported error '{error}' did not contain '{needle}'"
    );
}

fn buffer(label: &str, usages: Vec<BufferUsage>) -> BufferDesc {
    BufferDesc {
        label: label.to_owned(),
        size: 4096,
        memory: MemoryDomain::DeviceLocal,
        usages,
    }
}

fn texture(label: &str, format: TextureFormat, usages: Vec<TextureUsage>) -> TextureDesc {
    TextureDesc {
        label: label.to_owned(),
        dimension: TextureDimension::D2,
        format,
        extent: Extent3d {
            width: 128,
            height: 128,
            depth: 1,
        },
        mip_levels: 1,
        array_layers: 1,
        usages,
    }
}

fn view(label: &str, texture: Handle, format: TextureFormat) -> TextureViewDesc {
    TextureViewDesc {
        label: label.to_owned(),
        texture,
        format,
        base_mip: 0,
        mip_count: 1,
        base_layer: 0,
        layer_count: 1,
    }
}

fn sampler(label: &str) -> SamplerDesc {
    SamplerDesc {
        label: label.to_owned(),
        min_filter: SamplerFilter::Linear,
        mag_filter: SamplerFilter::Linear,
        mip_filter: SamplerFilter::Nearest,
        address_u: SamplerAddressMode::ClampToEdge,
        address_v: SamplerAddressMode::ClampToEdge,
        address_w: SamplerAddressMode::ClampToEdge,
        comparison: None,
    }
}

fn layout_binding(
    binding: u32,
    kind: ResourceBindingKind,
    stages: PipelineStageFlags,
) -> ResourceBindingDesc {
    ResourceBindingDesc {
        binding,
        kind,
        stages,
        array_count: 1,
        optional: false,
        dynamic_offset_count: 0,
    }
}

fn resource_binding(
    binding: u32,
    resource: Handle,
    kind: ResourceBindingKind,
    access: AccessFlags,
) -> ResourceBinding {
    ResourceBinding {
        binding,
        array_index: 0,
        resource,
        kind,
        access,
        dynamic_offsets: vec![],
        buffer_range: None,
    }
}

fn shader(label: &str, stage: ShaderStage) -> ShaderModuleDesc {
    ShaderModuleDesc {
        label: label.to_owned(),
        stage,
        code_format: ShaderCodeFormat::Spirv,
        code: vec![3, 2, 35, 7],
        entry_point: "main".to_owned(),
    }
}

fn color_attachment(view: Handle) -> PassAttachment {
    PassAttachment {
        view,
        load_op: AttachmentLoadOp::Clear,
        store_op: AttachmentStoreOp::Store,
        clear_color: Some(ClearColor {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        }),
    }
}

fn simple_graphics_scene(gal: &mut VulkanicGal) -> (Handle, Handle, Handle, Handle, Handle) {
    let color_texture = gal
        .create_texture(texture(
            "color",
            TextureFormat::Rgba8Unorm,
            vec![TextureUsage::ColorAttachment, TextureUsage::Sampled],
        ))
        .unwrap();
    let color_view = gal
        .create_texture_view(view("color-view", color_texture, TextureFormat::Rgba8Unorm))
        .unwrap();
    let target = gal
        .create_render_target(RenderTargetDesc {
            label: "target".to_owned(),
            color_views: vec![color_view],
            depth_stencil_view: None,
            extent: Extent3d {
                width: 128,
                height: 128,
                depth: 1,
            },
        })
        .unwrap();
    let pass = gal
        .create_render_pass(RenderPassDesc {
            label: "main-pass".to_owned(),
            target,
            color_formats: vec![TextureFormat::Rgba8Unorm],
            depth_format: None,
        })
        .unwrap();
    let layout = gal
        .create_pipeline_layout(PipelineLayoutDesc {
            label: "empty-pipeline-layout".to_owned(),
            resource_layouts: vec![],
        })
        .unwrap();
    let vertex_shader = gal
        .create_shader_module(shader("vertex", ShaderStage::Vertex))
        .unwrap();
    let fragment_shader = gal
        .create_shader_module(shader("fragment", ShaderStage::Fragment))
        .unwrap();
    let pipeline = gal
        .create_graphics_pipeline(GraphicsPipelineDesc {
            label: "pipeline".to_owned(),
            layout,
            vertex_shader,
            fragment_shader,
            topology: PrimitiveTopology::Triangles,
            cull_mode: CullMode::Back,
            front_face: crate::render::vulkanic::resources::FrontFace::CounterClockwise,
            provoking_vertex: crate::render::vulkanic::resources::ProvokingVertex::Last,
            raster_y_direction: crate::render::vulkanic::resources::RasterYDirection::Up,
            blend: BlendMode::Disabled,
            depth_compare: None,
            depth_write: false,
            depth_bias: None,
            color_formats: vec![TextureFormat::Rgba8Unorm],
            depth_format: None,
            stencil: None,
        })
        .unwrap();
    (color_view, target, pass, layout, pipeline)
}

#[test]
fn backend_capabilities_are_queryable_and_fingerprinted_without_api_tokens() {
    let gal = gal_with_capabilities(opengl_capabilities());
    let capabilities = gal.capabilities();
    assert_eq!("Rust OpenGL", capabilities.name);
    assert!(capabilities.supports(BackendFeature::Graphics));
    assert!(capabilities.supports(BackendFeature::DescriptorArrays));
    assert!(!capabilities.supports(BackendFeature::Compute));
    assert!(!capabilities.supports(BackendFeature::StorageTextures));
    assert!(capabilities.supports(BackendFeature::Texture3d));
    assert!(capabilities.supports(BackendFeature::TextureMipLevels));
    assert!(capabilities.supports_texture_3d_usage(TextureFormat::R8Uint, TextureUsage::Sampled));
    assert!(capabilities
        .supports_texture_3d_usage(TextureFormat::Rgba16Float, TextureUsage::TransferDst));
    assert!(!capabilities.supports_texture_3d_usage(TextureFormat::R8Uint, TextureUsage::Storage));
    assert!(!capabilities
        .supports_texture_3d_usage(TextureFormat::R8Uint, TextureUsage::ColorAttachment));
    let vulkan = vulkan_capabilities();
    assert!(vulkan.supports_texture_3d_usage(TextureFormat::R8Uint, TextureUsage::Storage));
    assert!(vulkan.supports_texture_3d_usage(TextureFormat::Rgba16Float, TextureUsage::Storage));
    let fingerprint = capabilities.fingerprint_json();
    assert!(fingerprint.contains("\"name\":\"Rust OpenGL\""));
    assert!(fingerprint.contains("\"compute\":false"));
    assert!(fingerprint.contains("\"max_color_attachments\":4"));
    assert!(fingerprint.contains("\"texture_3d\":true"));
    assert!(fingerprint.contains("\"max_texture_extent_3d\":2048"));
    for forbidden in [
        concat!("glow", "::"),
        concat!("ash", "::"),
        concat!("vk", "::"),
        concat!("GL", "_"),
        concat!("VK", "_"),
    ] {
        assert!(!fingerprint.contains(forbidden));
    }
}

#[test]
fn backend_capability_rejection_is_deterministic_before_native_creation() {
    let mut gal = gal_with_capabilities(opengl_capabilities());
    assert_unsupported(
        gal.create_texture(TextureDesc {
            dimension: TextureDimension::D3,
            extent: Extent3d {
                width: 8,
                height: 8,
                depth: 8,
            },
            array_layers: 1,
            usages: vec![TextureUsage::ColorAttachment],
            ..texture(
                "d3-render-target",
                TextureFormat::Rgba8Unorm,
                vec![TextureUsage::ColorAttachment],
            )
        }),
        "not render targets",
    );
    assert_unsupported(
        gal.create_texture(TextureDesc {
            usages: vec![TextureUsage::Storage],
            ..texture(
                "storage-image",
                TextureFormat::Rgba8Unorm,
                vec![TextureUsage::Storage],
            )
        }),
        "storage textures",
    );
    assert_unsupported(
        gal.create_compute_pipeline(ComputePipelineDesc {
            label: "compute".to_owned(),
            layout: Handle::from_raw(0),
            shader: Handle::from_raw(0),
        }),
        "compute pipelines",
    );
    assert_eq!(0, gal.metrics().resource_creates);
    assert_eq!(3, gal.metrics().validation_failures);
}

#[test]
fn capability_limits_reject_descriptor_and_attachment_overflows() {
    let mut gal = gal_with_capabilities(opengl_capabilities());
    assert_unsupported(
        gal.create_resource_layout(ResourceLayoutDesc {
            label: "too-many-bindings".to_owned(),
            bindings: (0..17)
                .map(|binding| {
                    layout_binding(
                        binding,
                        ResourceBindingKind::UniformBuffer,
                        PipelineStageFlags::DRAW,
                    )
                })
                .collect(),
        }),
        "binding count",
    );
    assert_unsupported(
        gal.create_resource_layout(ResourceLayoutDesc {
            label: "too-wide-array".to_owned(),
            bindings: vec![ResourceBindingDesc {
                binding: 0,
                kind: ResourceBindingKind::UniformBuffer,
                stages: PipelineStageFlags::DRAW,
                array_count: 9,
                optional: false,
                dynamic_offset_count: 0,
            }],
        }),
        "array count",
    );
    let vertex_shader = gal
        .create_shader_module(shader("vertex", ShaderStage::Vertex))
        .unwrap();
    let fragment_shader = gal
        .create_shader_module(shader("fragment", ShaderStage::Fragment))
        .unwrap();
    let layout = gal
        .create_pipeline_layout(PipelineLayoutDesc {
            label: "empty".to_owned(),
            resource_layouts: vec![],
        })
        .unwrap();
    assert_unsupported(
        gal.create_graphics_pipeline(GraphicsPipelineDesc {
            label: "too-many-attachments".to_owned(),
            layout,
            vertex_shader,
            fragment_shader,
            topology: PrimitiveTopology::Triangles,
            cull_mode: CullMode::None,
            front_face: crate::render::vulkanic::resources::FrontFace::CounterClockwise,
            provoking_vertex: crate::render::vulkanic::resources::ProvokingVertex::Last,
            raster_y_direction: crate::render::vulkanic::resources::RasterYDirection::Up,
            blend: BlendMode::Disabled,
            depth_compare: None,
            depth_write: false,
            depth_bias: None,
            color_formats: vec![TextureFormat::Rgba8Unorm; 5],
            depth_format: None,
            stencil: None,
        }),
        "color attachment count",
    );
}

#[test]
fn unsupported_indirect_and_presentation_commands_reject_before_backend_encoding() {
    let mut gal = gal_with_capabilities(opengl_capabilities());
    let (_color_view, target, pass, _layout, pipeline) = simple_graphics_scene(&mut gal);
    let indirect = gal
        .create_buffer(buffer("indirect", vec![BufferUsage::Indirect]))
        .unwrap();
    assert_unsupported(
        gal.create_command_list(CommandListDesc {
            label: "unsupported-indirect".to_owned(),
            operations: vec![
                CommandOp::BeginPass {
                    pass,
                    target,
                    colors: vec![color_attachment(_color_view)],
                    depth_stencil: None,
                },
                CommandOp::BindGraphicsPipeline(pipeline),
                CommandOp::DrawIndirect {
                    buffer: indirect,
                    offset: 0,
                    draw_count: 1,
                },
                CommandOp::EndPass,
            ],
        }),
        "indirect draw",
    );
    let present_texture = gal
        .create_texture(texture(
            "ordinary-color",
            TextureFormat::Rgba8Unorm,
            vec![TextureUsage::ColorAttachment],
        ))
        .unwrap();
    assert_unsupported(
        gal.create_command_list(CommandListDesc {
            label: "unsupported-present".to_owned(),
            operations: vec![CommandOp::Present {
                texture: present_texture,
                subresources: TextureSubresourceRange {
                    base_mip: 0,
                    mip_count: 1,
                    base_layer: 0,
                    layer_count: 1,
                },
            }],
        }),
        "presentation commands",
    );
}

#[test]
fn frame_lifecycle_requires_presentation_capability() {
    let mut gal = gal();
    assert_unsupported(
        gal.configure_frame_surface(frame_surface("headless")),
        "presentation support",
    );
    assert_unsupported(
        gal.acquire_frame(FrameAcquireDesc {
            correlation_id: FrameCorrelationId(1),
            expected_extent: Extent3d {
                width: 128,
                height: 72,
                depth: 1,
            },
        }),
        "presentation support",
    );
}

#[test]
fn frame_surface_rejects_excessive_in_flight_slots() {
    let mut gal = gal_with_capabilities(presentation_capabilities());
    let mut surface = frame_surface("bounded-slots");
    surface.max_frames_in_flight = super::gal::MAX_FRAMES_IN_FLIGHT + 1;
    assert_code(
        gal.configure_frame_surface(surface),
        StatusCode::InvalidArgument,
    );
}

#[test]
fn frame_surface_rejects_oversized_or_non_2d_extent() {
    let mut gal = gal_with_capabilities(presentation_capabilities());
    let mut oversized = frame_surface("oversized-surface");
    oversized.extent.width = super::gal::MAX_FRAME_SURFACE_AXIS + 1;
    assert_code(
        gal.configure_frame_surface(oversized),
        StatusCode::InvalidArgument,
    );

    let mut three_dimensional = frame_surface("3d-surface");
    three_dimensional.extent.depth = 2;
    assert_code(
        gal.configure_frame_surface(three_dimensional),
        StatusCode::InvalidArgument,
    );
}

#[test]
fn frame_lifecycle_preserves_correlation_and_submission_ids() {
    let mut gal = gal_with_capabilities(presentation_capabilities());
    gal.configure_frame_surface(frame_surface("window"))
        .unwrap();
    let acquired = gal
        .acquire_frame(FrameAcquireDesc {
            correlation_id: FrameCorrelationId(77),
            expected_extent: Extent3d {
                width: 128,
                height: 72,
                depth: 1,
            },
        })
        .unwrap();
    assert_eq!(acquired.status, FrameAcquireStatus::Ready);
    assert_eq!(acquired.correlation_id, FrameCorrelationId(77));
    assert_eq!(
        acquired.render_target,
        FrameRenderTargetId(acquired.frame.0)
    );
    let presented = gal
        .present_frame(PresentFrameDesc {
            frame: acquired.frame,
            correlation_id: acquired.correlation_id,
            wait_for: SubmissionId(9),
        })
        .unwrap();
    assert_eq!(presented.status, FramePresentStatus::Presented);
    assert_eq!(presented.completed_submission, SubmissionId(9));
    assert_eq!(gal.poll_completed(), SubmissionId(9));
}

#[test]
fn frame_lifecycle_models_resize_and_minimized_windows() {
    let mut gal = gal_with_capabilities(presentation_capabilities());
    gal.configure_frame_surface(frame_surface("resize"))
        .unwrap();
    let resize = gal
        .resize_frame_surface(FrameResizeDesc {
            correlation_id: FrameCorrelationId(2),
            extent: Extent3d {
                width: 256,
                height: 144,
                depth: 1,
            },
        })
        .unwrap();
    assert_eq!(resize.status, FrameAcquireStatus::Resized);
    let acquired = gal
        .acquire_frame(FrameAcquireDesc {
            correlation_id: FrameCorrelationId(3),
            expected_extent: resize.extent,
        })
        .unwrap();
    assert_eq!(acquired.extent, resize.extent);
    let minimized = gal
        .resize_frame_surface(FrameResizeDesc {
            correlation_id: FrameCorrelationId(4),
            extent: Extent3d {
                width: 0,
                height: 0,
                depth: 1,
            },
        })
        .unwrap();
    assert_eq!(minimized.status, FrameAcquireStatus::Minimized);
    let acquired = gal
        .acquire_frame(FrameAcquireDesc {
            correlation_id: FrameCorrelationId(5),
            expected_extent: minimized.extent,
        })
        .unwrap();
    assert_eq!(acquired.status, FrameAcquireStatus::Minimized);
}

#[test]
fn frame_lifecycle_rejects_present_without_acquire() {
    let mut gal = gal_with_capabilities(presentation_capabilities());
    gal.configure_frame_surface(frame_surface("bad-present"))
        .unwrap();
    assert_code(
        gal.present_frame(PresentFrameDesc {
            frame: FrameId(99),
            correlation_id: FrameCorrelationId(6),
            wait_for: SubmissionId(1),
        }),
        super::StatusCode::InvalidArgument,
    );
}

#[test]
fn frame_lifecycle_cancel_releases_an_acquired_frame() {
    let mut gal = gal_with_capabilities(presentation_capabilities());
    gal.configure_frame_surface(frame_surface("cancel"))
        .unwrap();
    let acquired = gal
        .acquire_frame(FrameAcquireDesc {
            correlation_id: FrameCorrelationId(8),
            expected_extent: Extent3d {
                width: 128,
                height: 72,
                depth: 1,
            },
        })
        .unwrap();
    gal.cancel_frame(acquired.frame).unwrap();
    assert_code(
        gal.present_frame(PresentFrameDesc {
            frame: acquired.frame,
            correlation_id: acquired.correlation_id,
            wait_for: SubmissionId(1),
        }),
        super::StatusCode::InvalidArgument,
    );
}

#[test]
fn acquired_frame_targets_are_normal_pass_targets_without_attachment_borrows() {
    let mut gal = gal_with_capabilities(presentation_capabilities());
    gal.configure_frame_surface(frame_surface("borrowed-default-framebuffer"))
        .unwrap();
    let acquired = gal
        .acquire_frame(FrameAcquireDesc {
            correlation_id: FrameCorrelationId(101),
            expected_extent: Extent3d {
                width: 128,
                height: 72,
                depth: 1,
            },
        })
        .unwrap();
    let frame_target = gal
        .create_frame_target(FrameTargetDesc {
            label: "minecraft-default-framebuffer".to_owned(),
            frame_id: acquired.frame.0,
            render_target: acquired.render_target,
            extent: acquired.extent,
            color_format: TextureFormat::Rgba8Unorm,
        })
        .unwrap();
    assert_eq!(frame_target.kind(), Some(HandleKind::FrameTarget));
    let pass = gal
        .create_render_pass(RenderPassDesc {
            label: "gui-frame-pass".to_owned(),
            target: frame_target,
            color_formats: vec![TextureFormat::Rgba8Unorm],
            depth_format: None,
        })
        .unwrap();
    let layout = gal
        .create_pipeline_layout(PipelineLayoutDesc {
            label: "gui-frame-pipeline-layout".to_owned(),
            resource_layouts: vec![],
        })
        .unwrap();
    let vertex_shader = gal
        .create_shader_module(shader("gui-frame-vertex", ShaderStage::Vertex))
        .unwrap();
    let fragment_shader = gal
        .create_shader_module(shader("gui-frame-fragment", ShaderStage::Fragment))
        .unwrap();
    let pipeline = gal
        .create_graphics_pipeline(GraphicsPipelineDesc {
            label: "gui-frame-pipeline".to_owned(),
            layout,
            vertex_shader,
            fragment_shader,
            topology: PrimitiveTopology::Triangles,
            cull_mode: CullMode::Back,
            front_face: crate::render::vulkanic::resources::FrontFace::CounterClockwise,
            provoking_vertex: crate::render::vulkanic::resources::ProvokingVertex::Last,
            raster_y_direction: crate::render::vulkanic::resources::RasterYDirection::Up,
            blend: BlendMode::Alpha,
            depth_compare: None,
            depth_write: false,
            depth_bias: None,
            color_formats: vec![TextureFormat::Rgba8Unorm],
            depth_format: None,
            stencil: None,
        })
        .unwrap();
    let list = gal
        .create_command_list(CommandListDesc {
            label: "gui-frame-list".to_owned(),
            operations: vec![
                CommandOp::BeginPass {
                    pass,
                    target: frame_target,
                    colors: vec![PassAttachment {
                        view: frame_target,
                        load_op: AttachmentLoadOp::Clear,
                        store_op: AttachmentStoreOp::Store,
                        clear_color: Some(ClearColor {
                            r: 0.25,
                            g: 0.5,
                            b: 0.75,
                            a: 1.0,
                        }),
                    }],
                    depth_stencil: None,
                },
                CommandOp::BindGraphicsPipeline(pipeline),
                CommandOp::EndPass,
            ],
        })
        .unwrap();
    let submission = gal
        .submit(SubmissionBatch {
            label: "gui-frame-submit".to_owned(),
            command_lists: vec![list],
        })
        .unwrap();
    let presented = gal
        .present_frame(PresentFrameDesc {
            frame: acquired.frame,
            correlation_id: acquired.correlation_id,
            wait_for: submission.submission,
        })
        .unwrap();
    assert_eq!(presented.completed_submission, submission.submission);
}

#[test]
fn frame_target_descriptor_exposes_only_semantic_swapchain_identity() {
    let mut gal = gal_with_capabilities(presentation_capabilities());
    gal.configure_frame_surface(frame_surface("semantic-frame-target"))
        .unwrap();
    let acquired = gal
        .acquire_frame(FrameAcquireDesc {
            correlation_id: FrameCorrelationId(102),
            expected_extent: Extent3d {
                width: 128,
                height: 72,
                depth: 1,
            },
        })
        .unwrap();
    let frame_target = gal
        .create_frame_target(FrameTargetDesc {
            label: "semantic-frame-target".to_owned(),
            frame_id: acquired.frame.0,
            render_target: acquired.render_target,
            extent: acquired.extent,
            color_format: TextureFormat::Rgba8Unorm,
        })
        .unwrap();
    let descriptor = gal.frame_target_desc(frame_target).unwrap();
    assert_eq!(acquired.frame.0, descriptor.frame_id);
    assert_eq!(acquired.render_target, descriptor.render_target);
    assert_eq!(acquired.extent, descriptor.extent);
    assert_eq!(TextureFormat::Rgba8Unorm, descriptor.color_format);
    let (depth_texture, depth_view) = gal
        .frame_target_owned_depth_attachment(frame_target)
        .unwrap();
    assert_ne!(depth_texture, depth_view);
    assert!(gal
        .pass_target_depth_attachment(frame_target)
        .unwrap()
        .is_none());
    gal.begin_frame_target_depth_write(frame_target).unwrap();
    assert_eq!(
        Some((depth_texture, depth_view)),
        gal.pass_target_depth_attachment(frame_target).unwrap()
    );
    gal.rollback_frame_target_depth_write(frame_target);
    assert!(gal
        .pass_target_depth_attachment(frame_target)
        .unwrap()
        .is_none());
    gal.begin_frame_target_depth_write(frame_target).unwrap();
    gal.commit_frame_target_depth_write(frame_target).unwrap();
    assert_eq!(
        Some((depth_texture, depth_view)),
        gal.pass_target_depth_attachment(frame_target).unwrap()
    );
    gal.destroy(frame_target).unwrap();
    assert_code(
        gal.frame_target_desc(frame_target),
        super::StatusCode::StaleHandle,
    );
    assert_code(
        gal.frame_target_owned_depth_attachment(frame_target),
        super::StatusCode::StaleHandle,
    );
}

#[test]
fn frame_target_copy_requires_explicit_owned_destination_and_matching_extent() {
    let mut gal = gal_with_capabilities(presentation_capabilities());
    gal.configure_frame_surface(frame_surface("frame-copy-contract"))
        .unwrap();
    let acquired = gal
        .acquire_frame(FrameAcquireDesc {
            correlation_id: FrameCorrelationId(103),
            expected_extent: Extent3d {
                width: 128,
                height: 72,
                depth: 1,
            },
        })
        .unwrap();
    let frame_target = gal
        .create_frame_target(FrameTargetDesc {
            label: "frame-copy-source".to_owned(),
            frame_id: acquired.frame.0,
            render_target: acquired.render_target,
            extent: acquired.extent,
            color_format: TextureFormat::Rgba8Unorm,
        })
        .unwrap();
    let destination = gal
        .create_texture(texture(
            "frame-copy-destination",
            TextureFormat::Rgba8Unorm,
            vec![TextureUsage::TransferDst],
        ))
        .unwrap();

    gal.create_command_list(CommandListDesc {
        label: "valid-frame-copy".to_owned(),
        operations: vec![
            CommandOp::Barrier(ResourceBarrier {
                resource: destination,
                subresources: None,
                before: TextureUsageState::Undefined,
                after: TextureUsageState::TransferDst,
                src_queue: QueueClass::Graphics,
                dst_queue: QueueClass::Transfer,
            }),
            CommandOp::CopyFrameTargetToTexture {
                src: frame_target,
                dst: destination,
                extent: Extent3d {
                    width: 128,
                    height: 72,
                    depth: 1,
                },
            },
        ],
    })
    .unwrap();

    let color_only_destination = gal
        .create_texture(texture(
            "frame-copy-color-only",
            TextureFormat::Rgba8Unorm,
            vec![TextureUsage::ColorAttachment],
        ))
        .unwrap();
    assert_code(
        gal.create_command_list(CommandListDesc {
            label: "frame-copy-without-transfer-dst".to_owned(),
            operations: vec![CommandOp::CopyFrameTargetToTexture {
                src: frame_target,
                dst: color_only_destination,
                extent: Extent3d {
                    width: 64,
                    height: 36,
                    depth: 1,
                },
            }],
        }),
        super::StatusCode::InvalidArgument,
    );
    assert_code(
        gal.create_command_list(CommandListDesc {
            label: "frame-copy-outside-source".to_owned(),
            operations: vec![CommandOp::CopyFrameTargetToTexture {
                src: frame_target,
                dst: destination,
                extent: Extent3d {
                    width: 129,
                    height: 72,
                    depth: 1,
                },
            }],
        }),
        super::StatusCode::InvalidArgument,
    );
}

#[test]
fn texture_to_frame_target_copy_requires_explicit_owned_source_and_matching_extent() {
    let mut gal = gal_with_capabilities(presentation_capabilities());
    gal.configure_frame_surface(frame_surface("frame-copy-reverse-contract"))
        .unwrap();
    let acquired = gal
        .acquire_frame(FrameAcquireDesc {
            correlation_id: FrameCorrelationId(104),
            expected_extent: Extent3d {
                width: 128,
                height: 72,
                depth: 1,
            },
        })
        .unwrap();
    let frame_target = gal
        .create_frame_target(FrameTargetDesc {
            label: "frame-copy-reverse-destination".to_owned(),
            frame_id: acquired.frame.0,
            render_target: acquired.render_target,
            extent: acquired.extent,
            color_format: TextureFormat::Rgba8Unorm,
        })
        .unwrap();
    let source = gal
        .create_texture(texture(
            "frame-copy-reverse-source",
            TextureFormat::Rgba8Unorm,
            vec![TextureUsage::TransferSrc],
        ))
        .unwrap();
    gal.create_command_list(CommandListDesc {
        label: "valid-reverse-frame-copy".to_owned(),
        operations: vec![
            CommandOp::Barrier(ResourceBarrier {
                resource: source,
                subresources: None,
                before: TextureUsageState::Undefined,
                after: TextureUsageState::TransferSrc,
                src_queue: QueueClass::Graphics,
                dst_queue: QueueClass::Transfer,
            }),
            CommandOp::CopyTextureToFrameTarget {
                src: source,
                dst: frame_target,
                extent: Extent3d {
                    width: 128,
                    height: 72,
                    depth: 1,
                },
            },
        ],
    })
    .unwrap();
    let invalid_source = gal
        .create_texture(texture(
            "frame-copy-reverse-invalid-source",
            TextureFormat::Rgba8Unorm,
            vec![TextureUsage::Sampled],
        ))
        .unwrap();
    assert_code(
        gal.create_command_list(CommandListDesc {
            label: "reverse-frame-copy-without-transfer-src".to_owned(),
            operations: vec![CommandOp::CopyTextureToFrameTarget {
                src: invalid_source,
                dst: frame_target,
                extent: Extent3d {
                    width: 64,
                    height: 36,
                    depth: 1,
                },
            }],
        }),
        super::StatusCode::InvalidArgument,
    );
}

#[test]
fn large_supported_batches_validate_with_backend_limits() {
    let mut gal = gal_with_capabilities(vulkan_capabilities());
    let src = gal
        .create_buffer(BufferDesc {
            label: "large-src".to_owned(),
            size: 4096,
            memory: MemoryDomain::Upload,
            usages: vec![BufferUsage::HostWrite, BufferUsage::TransferSrc],
        })
        .unwrap();
    let dst = gal
        .create_buffer(BufferDesc {
            label: "large-dst".to_owned(),
            size: 4096,
            memory: MemoryDomain::Readback,
            usages: vec![BufferUsage::TransferDst, BufferUsage::HostRead],
        })
        .unwrap();
    let mut lists = Vec::new();
    for index in 0..8 {
        lists.push(
            gal.create_command_list(CommandListDesc {
                label: format!("large-list-{index}"),
                operations: vec![
                    CommandOp::CopyBuffer { src, dst, size: 16 },
                    CommandOp::Barrier(ResourceBarrier {
                        resource: dst,
                        subresources: None,
                        before: TextureUsageState::TransferDst,
                        after: TextureUsageState::ShaderRead,
                        src_queue: QueueClass::Graphics,
                        dst_queue: QueueClass::Graphics,
                    }),
                ],
            })
            .unwrap(),
        );
    }
    let token = gal
        .submit(SubmissionBatch {
            label: "large-submit".to_owned(),
            command_lists: lists,
        })
        .unwrap();
    assert_eq!(SubmissionId(1), token.submission);
    assert_eq!(1, gal.metrics().submissions);
}

#[test]
fn vulkan_capabilities_accept_mip_layer_copy_and_indirect_commands() {
    let mut gal = gal_with_capabilities(vulkan_capabilities());
    let upload = gal
        .create_buffer(BufferDesc {
            label: "mip-layer-upload".to_owned(),
            size: 64,
            memory: MemoryDomain::Upload,
            usages: vec![BufferUsage::TransferSrc, BufferUsage::HostWrite],
        })
        .unwrap();
    let readback = gal
        .create_buffer(BufferDesc {
            label: "mip-layer-readback".to_owned(),
            size: 64,
            memory: MemoryDomain::Readback,
            usages: vec![BufferUsage::TransferDst, BufferUsage::HostRead],
        })
        .unwrap();
    let texture = gal
        .create_texture(TextureDesc {
            label: "mip-layer-texture".to_owned(),
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            extent: Extent3d {
                width: 8,
                height: 8,
                depth: 1,
            },
            mip_levels: 2,
            array_layers: 2,
            usages: vec![TextureUsage::TransferDst, TextureUsage::TransferSrc],
        })
        .unwrap();
    let region = BufferImageCopyRegion {
        buffer: upload,
        buffer_offset: 0,
        bytes_per_row: 16,
        rows_per_image: 4,
        texture,
        texture_mip: 1,
        texture_layer: 1,
        texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
        extent: Extent3d {
            width: 4,
            height: 4,
            depth: 1,
        },
    };
    gal.create_command_list(CommandListDesc {
        label: "mip-layer-copy".to_owned(),
        operations: vec![CommandOp::CopyBufferToTexture(region)],
    })
    .unwrap();
    let region = BufferImageCopyRegion {
        buffer: readback,
        buffer_offset: 0,
        bytes_per_row: 16,
        rows_per_image: 4,
        texture,
        texture_mip: 1,
        texture_layer: 1,
        texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
        extent: Extent3d {
            width: 4,
            height: 4,
            depth: 1,
        },
    };
    gal.create_command_list(CommandListDesc {
        label: "mip-layer-readback".to_owned(),
        operations: vec![CommandOp::CopyTextureToBuffer(region)],
    })
    .unwrap();

    let indirect = gal
        .create_buffer(buffer("indirect", vec![BufferUsage::Indirect]))
        .unwrap();
    let (_color_view, target, pass, _layout, graphics) = simple_graphics_scene(&mut gal);
    assert_code(
        gal.create_command_list(CommandListDesc {
            label: "copy-inside-render-pass".to_owned(),
            operations: vec![
                CommandOp::BeginPass {
                    pass,
                    target,
                    colors: vec![color_attachment(_color_view)],
                    depth_stencil: None,
                },
                CommandOp::CopyBuffer {
                    src: upload,
                    dst: readback,
                    size: 4,
                },
                CommandOp::EndPass,
            ],
        }),
        super::StatusCode::InvalidArgument,
    );
    gal.create_command_list(CommandListDesc {
        label: "indirect-draw".to_owned(),
        operations: vec![
            CommandOp::BeginPass {
                pass,
                target,
                colors: vec![color_attachment(_color_view)],
                depth_stencil: None,
            },
            CommandOp::BindGraphicsPipeline(graphics),
            CommandOp::DrawIndirect {
                buffer: indirect,
                offset: 0,
                draw_count: 1,
            },
            CommandOp::EndPass,
        ],
    })
    .unwrap();

    let layout = gal
        .create_pipeline_layout(PipelineLayoutDesc {
            label: "compute-layout".to_owned(),
            resource_layouts: vec![],
        })
        .unwrap();
    let shader = gal
        .create_shader_module(shader("compute", ShaderStage::Compute))
        .unwrap();
    let compute = gal
        .create_compute_pipeline(ComputePipelineDesc {
            label: "compute".to_owned(),
            layout,
            shader,
        })
        .unwrap();
    gal.create_command_list(CommandListDesc {
        label: "indirect-dispatch".to_owned(),
        operations: vec![
            CommandOp::BindComputePipeline(compute),
            CommandOp::DispatchIndirect {
                buffer: indirect,
                offset: 0,
            },
        ],
    })
    .unwrap();
}

#[test]
fn opengl_capabilities_reject_storage_images_and_layered_copies() {
    let mut gal = gal_with_capabilities(opengl_capabilities());
    assert_unsupported(
        gal.create_texture(TextureDesc {
            label: "layered".to_owned(),
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            extent: Extent3d {
                width: 8,
                height: 8,
                depth: 1,
            },
            mip_levels: 1,
            array_layers: 2,
            usages: vec![TextureUsage::Sampled],
        }),
        "layer count",
    );
    assert_unsupported(
        gal.create_resource_layout(ResourceLayoutDesc {
            label: "storage-image-layout".to_owned(),
            bindings: vec![ResourceBindingDesc {
                binding: 0,
                kind: ResourceBindingKind::StorageTexture,
                stages: PipelineStageFlags::COMPUTE,
                array_count: 1,
                optional: false,
                dynamic_offset_count: 0,
            }],
        }),
        "storage texture bindings",
    );
}

#[test]
fn handles_reuse_generations_and_reject_stale_or_wrong_types() {
    let mut gal = gal();
    let first = gal
        .create_buffer(buffer("first", vec![BufferUsage::Vertex]))
        .unwrap();
    assert_eq!(first.kind(), Some(HandleKind::Buffer));
    assert_eq!(first.index(), 0);
    assert_eq!(first.generation(), 1);

    gal.destroy(first).unwrap();
    assert_code(gal.destroy(first), super::StatusCode::DoubleDestroy);

    let second = gal
        .create_buffer(buffer("second", vec![BufferUsage::Vertex]))
        .unwrap();
    assert_eq!(second.index(), first.index());
    assert_eq!(second.generation(), first.generation() + 1);

    assert_code(
        gal.create_texture_view(view("wrong-resource", second, TextureFormat::Rgba8Unorm)),
        super::StatusCode::WrongHandleType,
    );
}

#[test]
fn dependencies_block_parent_destruction_and_allow_child_cleanup() {
    let mut gal = gal();
    let texture = gal
        .create_texture(texture(
            "tex",
            TextureFormat::Rgba8Unorm,
            vec![TextureUsage::Sampled],
        ))
        .unwrap();
    let view = gal
        .create_texture_view(view("view", texture, TextureFormat::Rgba8Unorm))
        .unwrap();

    assert_code(gal.destroy(texture), super::StatusCode::DependencyViolation);
    gal.destroy(view).unwrap();
    gal.destroy(texture).unwrap();
    assert_code(gal.destroy(view), super::StatusCode::DoubleDestroy);
}

#[test]
fn resource_sets_validate_layout_binding_kind_resource_type_and_access() {
    let mut gal = gal();
    let buffer = gal
        .create_buffer(buffer("ubo", vec![BufferUsage::Uniform]))
        .unwrap();
    let sampler = gal.create_sampler(sampler("sampler")).unwrap();
    let layout = gal
        .create_resource_layout(ResourceLayoutDesc {
            label: "layout".to_owned(),
            bindings: vec![
                layout_binding(
                    0,
                    ResourceBindingKind::UniformBuffer,
                    PipelineStageFlags::DRAW,
                ),
                layout_binding(1, ResourceBindingKind::Sampler, PipelineStageFlags::DRAW),
            ],
        })
        .unwrap();

    gal.create_resource_set(ResourceSetDesc {
        label: "set".to_owned(),
        layout,
        bindings: vec![
            resource_binding(
                0,
                buffer,
                ResourceBindingKind::UniformBuffer,
                AccessFlags::READ,
            ),
            resource_binding(1, sampler, ResourceBindingKind::Sampler, AccessFlags::READ),
        ],
    })
    .unwrap();

    assert_code(
        gal.create_resource_set(ResourceSetDesc {
            label: "wrong-kind".to_owned(),
            layout,
            bindings: vec![resource_binding(
                0,
                buffer,
                ResourceBindingKind::Sampler,
                AccessFlags::READ,
            )],
        }),
        super::StatusCode::InvalidArgument,
    );
    assert_code(
        gal.create_resource_set(ResourceSetDesc {
            label: "wrong-type".to_owned(),
            layout,
            bindings: vec![resource_binding(
                1,
                buffer,
                ResourceBindingKind::Sampler,
                AccessFlags::READ,
            )],
        }),
        super::StatusCode::WrongHandleType,
    );
    assert_code(
        gal.create_resource_set(ResourceSetDesc {
            label: "no-access".to_owned(),
            layout,
            bindings: vec![resource_binding(
                0,
                buffer,
                ResourceBindingKind::UniformBuffer,
                AccessFlags::NONE,
            )],
        }),
        super::StatusCode::InvalidArgument,
    );
    assert_code(
        gal.create_resource_layout(ResourceLayoutDesc {
            label: "no-stage-layout".to_owned(),
            bindings: vec![layout_binding(
                0,
                ResourceBindingKind::UniformBuffer,
                PipelineStageFlags::NONE,
            )],
        }),
        super::StatusCode::InvalidArgument,
    );
    assert_code(
        gal.create_resource_set(ResourceSetDesc {
            label: "duplicate-set-binding".to_owned(),
            layout,
            bindings: vec![
                resource_binding(
                    0,
                    buffer,
                    ResourceBindingKind::UniformBuffer,
                    AccessFlags::READ,
                ),
                resource_binding(
                    0,
                    buffer,
                    ResourceBindingKind::UniformBuffer,
                    AccessFlags::READ,
                ),
            ],
        }),
        super::StatusCode::InvalidArgument,
    );
}

#[test]
fn resource_sets_reject_linear_sampler_state_for_integer_sampled_textures() {
    let mut gal = gal();
    let occupancy = gal
        .create_texture(texture(
            "integer-occupancy",
            TextureFormat::R8Uint,
            vec![TextureUsage::Sampled],
        ))
        .unwrap();
    let occupancy_view = gal
        .create_texture_view(view(
            "integer-occupancy-view",
            occupancy,
            TextureFormat::R8Uint,
        ))
        .unwrap();
    let layout = gal
        .create_resource_layout(ResourceLayoutDesc {
            label: "integer-sampler-layout".to_owned(),
            bindings: vec![
                layout_binding(
                    0,
                    ResourceBindingKind::SampledTexture,
                    PipelineStageFlags::DRAW,
                ),
                layout_binding(1, ResourceBindingKind::Sampler, PipelineStageFlags::DRAW),
            ],
        })
        .unwrap();
    let linear_sampler = gal
        .create_sampler(sampler("linear-integer-sampler"))
        .unwrap();

    let error = gal
        .create_resource_set(ResourceSetDesc {
            label: "integer-linear-sampler-set".to_owned(),
            layout,
            bindings: vec![
                resource_binding(
                    0,
                    occupancy_view,
                    ResourceBindingKind::SampledTexture,
                    AccessFlags::READ,
                ),
                resource_binding(
                    1,
                    linear_sampler,
                    ResourceBindingKind::Sampler,
                    AccessFlags::READ,
                ),
            ],
        })
        .expect_err("integer textures must reject linear sampler filtering");
    assert_eq!(error.code, super::StatusCode::InvalidArgument);
    assert!(error.message.contains("integer sampled textures"));

    let nearest_sampler = gal
        .create_sampler(SamplerDesc {
            label: "nearest-integer-sampler".to_owned(),
            min_filter: SamplerFilter::Nearest,
            mag_filter: SamplerFilter::Nearest,
            mip_filter: SamplerFilter::Nearest,
            address_u: SamplerAddressMode::ClampToEdge,
            address_v: SamplerAddressMode::ClampToEdge,
            address_w: SamplerAddressMode::ClampToEdge,
            comparison: None,
        })
        .unwrap();
    gal.create_resource_set(ResourceSetDesc {
        label: "integer-nearest-sampler-set".to_owned(),
        layout,
        bindings: vec![
            resource_binding(
                0,
                occupancy_view,
                ResourceBindingKind::SampledTexture,
                AccessFlags::READ,
            ),
            resource_binding(
                1,
                nearest_sampler,
                ResourceBindingKind::Sampler,
                AccessFlags::READ,
            ),
        ],
    })
    .expect("integer textures must accept nearest sampler filtering");
}

#[test]
fn combined_texture_sampler_is_a_read_only_owned_pair() {
    let mut gal = gal();
    let occupancy = gal
        .create_texture(texture(
            "combined-integer-occupancy",
            TextureFormat::R8Uint,
            vec![TextureUsage::Sampled],
        ))
        .unwrap();
    let occupancy_view = gal
        .create_texture_view(view(
            "combined-integer-occupancy-view",
            occupancy,
            TextureFormat::R8Uint,
        ))
        .unwrap();
    let linear_sampler = gal
        .create_sampler(sampler("combined-linear-integer-sampler"))
        .unwrap();
    let error = gal
        .create_combined_texture_sampler(CombinedTextureSamplerDesc {
            label: "combined-linear-integer".to_owned(),
            texture_view: occupancy_view,
            sampler: linear_sampler,
        })
        .expect_err("integer combined samplers must reject linear filtering");
    assert_eq!(error.code, super::StatusCode::InvalidArgument);
    assert!(error.message.contains("integer combined texture samplers"));

    let nearest_sampler = gal
        .create_sampler(SamplerDesc {
            label: "combined-nearest-integer-sampler".to_owned(),
            min_filter: SamplerFilter::Nearest,
            mag_filter: SamplerFilter::Nearest,
            mip_filter: SamplerFilter::Nearest,
            address_u: SamplerAddressMode::ClampToEdge,
            address_v: SamplerAddressMode::ClampToEdge,
            address_w: SamplerAddressMode::ClampToEdge,
            comparison: None,
        })
        .unwrap();
    let combined = gal
        .create_combined_texture_sampler(CombinedTextureSamplerDesc {
            label: "combined-nearest-integer".to_owned(),
            texture_view: occupancy_view,
            sampler: nearest_sampler,
        })
        .unwrap();
    let layout = gal
        .create_resource_layout(ResourceLayoutDesc {
            label: "combined-sampler-layout".to_owned(),
            bindings: vec![layout_binding(
                0,
                ResourceBindingKind::CombinedTextureSampler,
                PipelineStageFlags::DRAW,
            )],
        })
        .unwrap();
    let set = gal
        .create_resource_set(ResourceSetDesc {
            label: "combined-sampler-set".to_owned(),
            layout,
            bindings: vec![resource_binding(
                0,
                combined,
                ResourceBindingKind::CombinedTextureSampler,
                AccessFlags::READ,
            )],
        })
        .unwrap();

    assert_code(
        gal.create_resource_set(ResourceSetDesc {
            label: "combined-sampler-write-set".to_owned(),
            layout,
            bindings: vec![resource_binding(
                0,
                combined,
                ResourceBindingKind::CombinedTextureSampler,
                AccessFlags::WRITE,
            )],
        }),
        super::StatusCode::InvalidArgument,
    );
    assert_code(
        gal.destroy(occupancy_view),
        super::StatusCode::DependencyViolation,
    );
    assert_code(
        gal.destroy(nearest_sampler),
        super::StatusCode::DependencyViolation,
    );
    gal.destroy(set).unwrap();
    gal.destroy(combined).unwrap();
    gal.destroy(nearest_sampler).unwrap();
    gal.destroy(occupancy_view).unwrap();
    gal.destroy(occupancy).unwrap();
}

#[test]
fn comparison_samplers_require_depth_views_and_preserve_ffi_default_sampling() {
    let mut gal = gal();
    let color = gal
        .create_texture(texture(
            "comparison-color",
            TextureFormat::Rgba8Unorm,
            vec![TextureUsage::Sampled],
        ))
        .unwrap();
    let color_view = gal
        .create_texture_view(view(
            "comparison-color-view",
            color,
            TextureFormat::Rgba8Unorm,
        ))
        .unwrap();
    let comparison_sampler = gal
        .create_sampler(SamplerDesc {
            comparison: Some(CompareOp::LessOrEqual),
            ..sampler("comparison-sampler")
        })
        .unwrap();
    let error = gal
        .create_combined_texture_sampler(CombinedTextureSamplerDesc {
            label: "comparison-color-combined".to_owned(),
            texture_view: color_view,
            sampler: comparison_sampler,
        })
        .expect_err("comparison samplers must reject color texture views");
    assert_eq!(error.code, super::StatusCode::InvalidArgument);
    assert!(error.message.contains("depth texture view"));

    let depth = gal
        .create_texture(texture(
            "comparison-depth",
            TextureFormat::Depth32Float,
            vec![TextureUsage::Sampled, TextureUsage::DepthStencilAttachment],
        ))
        .unwrap();
    let depth_view = gal
        .create_texture_view(view(
            "comparison-depth-view",
            depth,
            TextureFormat::Depth32Float,
        ))
        .unwrap();
    let combined = gal
        .create_combined_texture_sampler(CombinedTextureSamplerDesc {
            label: "comparison-depth-combined".to_owned(),
            texture_view: depth_view,
            sampler: comparison_sampler,
        })
        .unwrap();
    gal.destroy(combined).unwrap();
    gal.destroy(depth_view).unwrap();
    gal.destroy(depth).unwrap();
    gal.destroy(color_view).unwrap();
    gal.destroy(color).unwrap();
    gal.destroy(comparison_sampler).unwrap();
}

#[test]
fn resource_sets_require_complete_arrays_and_explicit_optionality() {
    let mut gal = gal();
    let first = gal
        .create_buffer(buffer("first-ubo", vec![BufferUsage::Uniform]))
        .unwrap();
    let second = gal
        .create_buffer(buffer("second-ubo", vec![BufferUsage::Uniform]))
        .unwrap();
    let array_layout = gal
        .create_resource_layout(ResourceLayoutDesc {
            label: "array-layout".to_owned(),
            bindings: vec![ResourceBindingDesc {
                binding: 0,
                kind: ResourceBindingKind::UniformBuffer,
                stages: PipelineStageFlags::DRAW,
                array_count: 2,
                optional: false,
                dynamic_offset_count: 0,
            }],
        })
        .unwrap();

    assert_code(
        gal.create_resource_set(ResourceSetDesc {
            label: "incomplete-array".to_owned(),
            layout: array_layout,
            bindings: vec![resource_binding(
                0,
                first,
                ResourceBindingKind::UniformBuffer,
                AccessFlags::READ,
            )],
        }),
        super::StatusCode::InvalidArgument,
    );

    gal.create_resource_set(ResourceSetDesc {
        label: "complete-array".to_owned(),
        layout: array_layout,
        bindings: vec![
            resource_binding(
                0,
                first,
                ResourceBindingKind::UniformBuffer,
                AccessFlags::READ,
            ),
            ResourceBinding {
                binding: 0,
                array_index: 1,
                resource: second,
                kind: ResourceBindingKind::UniformBuffer,
                access: AccessFlags::READ,
                dynamic_offsets: vec![],
                buffer_range: None,
            },
        ],
    })
    .unwrap();

    let optional_layout = gal
        .create_resource_layout(ResourceLayoutDesc {
            label: "optional-layout".to_owned(),
            bindings: vec![ResourceBindingDesc {
                binding: 0,
                kind: ResourceBindingKind::UniformBuffer,
                stages: PipelineStageFlags::DRAW,
                array_count: 2,
                optional: true,
                dynamic_offset_count: 0,
            }],
        })
        .unwrap();
    gal.create_resource_set(ResourceSetDesc {
        label: "explicitly-partial".to_owned(),
        layout: optional_layout,
        bindings: vec![],
    })
    .unwrap();

    let dynamic_layout = gal
        .create_resource_layout(ResourceLayoutDesc {
            label: "dynamic-layout".to_owned(),
            bindings: vec![ResourceBindingDesc {
                binding: 0,
                kind: ResourceBindingKind::UniformBuffer,
                stages: PipelineStageFlags::DRAW,
                array_count: 1,
                optional: false,
                dynamic_offset_count: 1,
            }],
        })
        .unwrap();
    assert_code(
        gal.create_resource_set(ResourceSetDesc {
            label: "missing-dynamic-offset".to_owned(),
            layout: dynamic_layout,
            bindings: vec![resource_binding(
                0,
                first,
                ResourceBindingKind::UniformBuffer,
                AccessFlags::READ,
            )],
        }),
        super::StatusCode::InvalidArgument,
    );
    gal.create_resource_set(ResourceSetDesc {
        label: "dynamic-offset".to_owned(),
        layout: dynamic_layout,
        bindings: vec![ResourceBinding {
            binding: 0,
            array_index: 0,
            resource: first,
            kind: ResourceBindingKind::UniformBuffer,
            access: AccessFlags::READ,
            dynamic_offsets: vec![64],
            buffer_range: None,
        }],
    })
    .unwrap();
}

#[test]
fn pipeline_and_pass_compatibility_is_validated() {
    let mut gal = gal();
    let (color_view, target, pass, layout, pipeline) = simple_graphics_scene(&mut gal);
    let good_ops = vec![
        CommandOp::BeginPass {
            pass,
            target,
            colors: vec![color_attachment(color_view)],
            depth_stencil: None,
        },
        CommandOp::BindGraphicsPipeline(pipeline),
        CommandOp::Draw {
            vertices: 3,
            instances: 1,
        },
        CommandOp::EndPass,
    ];
    gal.create_command_list(CommandListDesc {
        label: "good".to_owned(),
        operations: good_ops,
    })
    .unwrap();

    let vertex_shader = gal
        .create_shader_module(shader("second-vertex", ShaderStage::Vertex))
        .unwrap();
    let fragment_shader = gal
        .create_shader_module(shader("second-fragment", ShaderStage::Fragment))
        .unwrap();
    let bad_pipeline = gal
        .create_graphics_pipeline(GraphicsPipelineDesc {
            label: "bad-pipeline".to_owned(),
            layout,
            vertex_shader,
            fragment_shader,
            topology: PrimitiveTopology::Triangles,
            cull_mode: CullMode::Back,
            front_face: crate::render::vulkanic::resources::FrontFace::CounterClockwise,
            provoking_vertex: crate::render::vulkanic::resources::ProvokingVertex::Last,
            raster_y_direction: crate::render::vulkanic::resources::RasterYDirection::Up,
            blend: BlendMode::Disabled,
            depth_compare: None,
            depth_write: false,
            depth_bias: None,
            color_formats: vec![TextureFormat::Bgra8Unorm],
            depth_format: None,
            stencil: None,
        })
        .unwrap();
    assert_code(
        gal.create_command_list(CommandListDesc {
            label: "bad".to_owned(),
            operations: vec![
                CommandOp::BeginPass {
                    pass,
                    target,
                    colors: vec![color_attachment(color_view)],
                    depth_stencil: None,
                },
                CommandOp::BindGraphicsPipeline(bad_pipeline),
                CommandOp::EndPass,
            ],
        }),
        super::StatusCode::InvalidArgument,
    );
}

#[test]
fn graphics_pipeline_depth_bias_requires_finite_enabled_depth_contract() {
    let mut gal = gal();
    let layout = gal
        .create_pipeline_layout(PipelineLayoutDesc {
            label: "depth-bias-layout".to_owned(),
            resource_layouts: vec![],
        })
        .unwrap();
    let vertex_shader = gal
        .create_shader_module(shader("depth-bias-vertex", ShaderStage::Vertex))
        .unwrap();
    let fragment_shader = gal
        .create_shader_module(shader("depth-bias-fragment", ShaderStage::Fragment))
        .unwrap();
    let pipeline = |label: &str, depth_compare, depth_bias, depth_format| GraphicsPipelineDesc {
        label: label.to_owned(),
        layout,
        vertex_shader,
        fragment_shader,
        topology: PrimitiveTopology::Triangles,
        cull_mode: CullMode::None,
        front_face: FrontFace::CounterClockwise,
        provoking_vertex: crate::render::vulkanic::resources::ProvokingVertex::Last,
        raster_y_direction: crate::render::vulkanic::resources::RasterYDirection::Up,
        blend: BlendMode::Disabled,
        depth_compare,
        depth_write: false,
        depth_bias,
        color_formats: vec![TextureFormat::Rgba8Unorm],
        depth_format,
        stencil: None,
    };

    assert_code(
        gal.create_graphics_pipeline(pipeline(
            "non-finite-depth-bias",
            Some(CompareOp::LessOrEqual),
            Some(DepthBias::new(f32::NAN, -1.0)),
            Some(TextureFormat::Depth32Float),
        )),
        StatusCode::InvalidArgument,
    );
    assert_code(
        gal.create_graphics_pipeline(pipeline(
            "depth-bias-without-depth-test",
            None,
            Some(DepthBias::new(-10.0, -1.0)),
            Some(TextureFormat::Depth32Float),
        )),
        StatusCode::InvalidArgument,
    );
    assert!(gal
        .create_graphics_pipeline(pipeline(
            "valid-depth-bias",
            Some(CompareOp::LessOrEqual),
            Some(DepthBias::new(-10.0, -1.0)),
            Some(TextureFormat::Depth32Float),
        ))
        .is_ok());
}

#[test]
fn render_target_pass_and_attachment_views_match_their_textures() {
    let mut gal = gal();
    let color_texture = gal
        .create_texture(texture(
            "color",
            TextureFormat::Rgba8Unorm,
            vec![TextureUsage::ColorAttachment],
        ))
        .unwrap();
    assert_code(
        gal.create_texture_view(view(
            "reinterpret",
            color_texture,
            TextureFormat::Bgra8Unorm,
        )),
        super::StatusCode::InvalidArgument,
    );
    let color_view = gal
        .create_texture_view(view("color-view", color_texture, TextureFormat::Rgba8Unorm))
        .unwrap();
    assert_code(
        gal.create_render_target(RenderTargetDesc {
            label: "wrong-extent".to_owned(),
            color_views: vec![color_view],
            depth_stencil_view: None,
            extent: Extent3d {
                width: 64,
                height: 128,
                depth: 1,
            },
        }),
        super::StatusCode::InvalidArgument,
    );
    let target = gal
        .create_render_target(RenderTargetDesc {
            label: "target".to_owned(),
            color_views: vec![color_view],
            depth_stencil_view: None,
            extent: Extent3d {
                width: 128,
                height: 128,
                depth: 1,
            },
        })
        .unwrap();
    assert_code(
        gal.create_render_pass(RenderPassDesc {
            label: "wrong-format-pass".to_owned(),
            target,
            color_formats: vec![TextureFormat::Bgra8Unorm],
            depth_format: None,
        }),
        super::StatusCode::InvalidArgument,
    );
    let pass = gal
        .create_render_pass(RenderPassDesc {
            label: "pass".to_owned(),
            target,
            color_formats: vec![TextureFormat::Rgba8Unorm],
            depth_format: None,
        })
        .unwrap();
    let other_texture = gal
        .create_texture(texture(
            "other",
            TextureFormat::Rgba8Unorm,
            vec![TextureUsage::ColorAttachment],
        ))
        .unwrap();
    let other_view = gal
        .create_texture_view(view("other-view", other_texture, TextureFormat::Rgba8Unorm))
        .unwrap();
    assert_code(
        gal.create_command_list(CommandListDesc {
            label: "wrong-attachment".to_owned(),
            operations: vec![CommandOp::BeginPass {
                pass,
                target,
                colors: vec![color_attachment(other_view)],
                depth_stencil: None,
            }],
        }),
        super::StatusCode::InvalidArgument,
    );
}

#[test]
fn attachment_and_presentation_hazards_require_semantic_separation() {
    let mut gal = gal_with_capabilities(presentation_capabilities());
    let texture = gal
        .create_texture(texture(
            "attachment",
            TextureFormat::Rgba8Unorm,
            vec![TextureUsage::ColorAttachment, TextureUsage::Present],
        ))
        .unwrap();
    let first_view = gal
        .create_texture_view(view("first-view", texture, TextureFormat::Rgba8Unorm))
        .unwrap();
    let second_view = gal
        .create_texture_view(view("second-view", texture, TextureFormat::Rgba8Unorm))
        .unwrap();
    let overlapping_target = gal
        .create_render_target(RenderTargetDesc {
            label: "overlapping-target".to_owned(),
            color_views: vec![first_view, second_view],
            depth_stencil_view: None,
            extent: Extent3d {
                width: 128,
                height: 128,
                depth: 1,
            },
        })
        .unwrap();
    let overlapping_pass = gal
        .create_render_pass(RenderPassDesc {
            label: "overlapping-pass".to_owned(),
            target: overlapping_target,
            color_formats: vec![TextureFormat::Rgba8Unorm, TextureFormat::Rgba8Unorm],
            depth_format: None,
        })
        .unwrap();
    let layout = gal
        .create_pipeline_layout(PipelineLayoutDesc {
            label: "attachment-layout".to_owned(),
            resource_layouts: vec![],
        })
        .unwrap();
    let vertex_shader = gal
        .create_shader_module(shader("attachment-vertex", ShaderStage::Vertex))
        .unwrap();
    let fragment_shader = gal
        .create_shader_module(shader("attachment-fragment", ShaderStage::Fragment))
        .unwrap();
    let overlapping_pipeline = gal
        .create_graphics_pipeline(GraphicsPipelineDesc {
            label: "overlapping-pipeline".to_owned(),
            layout,
            vertex_shader,
            fragment_shader,
            topology: PrimitiveTopology::Triangles,
            cull_mode: CullMode::Back,
            front_face: crate::render::vulkanic::resources::FrontFace::CounterClockwise,
            provoking_vertex: crate::render::vulkanic::resources::ProvokingVertex::Last,
            raster_y_direction: crate::render::vulkanic::resources::RasterYDirection::Up,
            blend: BlendMode::Disabled,
            depth_compare: None,
            depth_write: false,
            depth_bias: None,
            color_formats: vec![TextureFormat::Rgba8Unorm, TextureFormat::Rgba8Unorm],
            depth_format: None,
            stencil: None,
        })
        .unwrap();
    let overlapping_list = gal
        .create_command_list(CommandListDesc {
            label: "overlapping-attachments".to_owned(),
            operations: vec![
                CommandOp::BeginPass {
                    pass: overlapping_pass,
                    target: overlapping_target,
                    colors: vec![color_attachment(first_view), color_attachment(second_view)],
                    depth_stencil: None,
                },
                CommandOp::BindGraphicsPipeline(overlapping_pipeline),
                CommandOp::EndPass,
            ],
        })
        .unwrap();
    assert_code(
        gal.submit(SubmissionBatch {
            label: "attachment-overlap".to_owned(),
            command_lists: vec![overlapping_list],
        }),
        super::StatusCode::InvalidArgument,
    );

    let target = gal
        .create_render_target(RenderTargetDesc {
            label: "single-target".to_owned(),
            color_views: vec![first_view],
            depth_stencil_view: None,
            extent: Extent3d {
                width: 128,
                height: 128,
                depth: 1,
            },
        })
        .unwrap();
    let pass = gal
        .create_render_pass(RenderPassDesc {
            label: "single-pass".to_owned(),
            target,
            color_formats: vec![TextureFormat::Rgba8Unorm],
            depth_format: None,
        })
        .unwrap();
    let vertex_shader = gal
        .create_shader_module(shader("present-vertex", ShaderStage::Vertex))
        .unwrap();
    let fragment_shader = gal
        .create_shader_module(shader("present-fragment", ShaderStage::Fragment))
        .unwrap();
    let pipeline = gal
        .create_graphics_pipeline(GraphicsPipelineDesc {
            label: "present-pipeline".to_owned(),
            layout,
            vertex_shader,
            fragment_shader,
            topology: PrimitiveTopology::Triangles,
            cull_mode: CullMode::Back,
            front_face: crate::render::vulkanic::resources::FrontFace::CounterClockwise,
            provoking_vertex: crate::render::vulkanic::resources::ProvokingVertex::Last,
            raster_y_direction: crate::render::vulkanic::resources::RasterYDirection::Up,
            blend: BlendMode::Disabled,
            depth_compare: None,
            depth_write: false,
            depth_bias: None,
            color_formats: vec![TextureFormat::Rgba8Unorm],
            depth_format: None,
            stencil: None,
        })
        .unwrap();
    let full_range = TextureSubresourceRange {
        base_mip: 0,
        mip_count: 1,
        base_layer: 0,
        layer_count: 1,
    };
    let repeated_attachment_writes = gal
        .create_command_list(CommandListDesc {
            label: "repeated-attachment-writes".to_owned(),
            operations: vec![
                CommandOp::BeginPass {
                    pass,
                    target,
                    colors: vec![color_attachment(first_view)],
                    depth_stencil: None,
                },
                CommandOp::BindGraphicsPipeline(pipeline),
                CommandOp::EndPass,
                CommandOp::BeginPass {
                    pass,
                    target,
                    colors: vec![PassAttachment {
                        view: first_view,
                        load_op: AttachmentLoadOp::Load,
                        store_op: AttachmentStoreOp::Store,
                        clear_color: None,
                    }],
                    depth_stencil: None,
                },
                CommandOp::BindGraphicsPipeline(pipeline),
                CommandOp::EndPass,
            ],
        })
        .unwrap();
    gal.submit(SubmissionBatch {
        label: "ordered-repeated-attachment-writes".to_owned(),
        command_lists: vec![repeated_attachment_writes],
    })
    .unwrap();

    let load_after_dont_care_store = gal
        .create_command_list(CommandListDesc {
            label: "load-after-dont-care-store".to_owned(),
            operations: vec![
                CommandOp::BeginPass {
                    pass,
                    target,
                    colors: vec![PassAttachment {
                        view: first_view,
                        load_op: AttachmentLoadOp::Clear,
                        store_op: AttachmentStoreOp::DontCare,
                        clear_color: None,
                    }],
                    depth_stencil: None,
                },
                CommandOp::BindGraphicsPipeline(pipeline),
                CommandOp::EndPass,
                CommandOp::BeginPass {
                    pass,
                    target,
                    colors: vec![PassAttachment {
                        view: first_view,
                        load_op: AttachmentLoadOp::Load,
                        store_op: AttachmentStoreOp::Store,
                        clear_color: None,
                    }],
                    depth_stencil: None,
                },
                CommandOp::BindGraphicsPipeline(pipeline),
                CommandOp::EndPass,
            ],
        })
        .unwrap();
    assert_code(
        gal.submit(SubmissionBatch {
            label: "attachment-load-after-dont-care-store".to_owned(),
            command_lists: vec![load_after_dont_care_store],
        }),
        super::StatusCode::InvalidArgument,
    );

    let present_without_barrier = gal
        .create_command_list(CommandListDesc {
            label: "present-without-barrier".to_owned(),
            operations: vec![
                CommandOp::BeginPass {
                    pass,
                    target,
                    colors: vec![color_attachment(first_view)],
                    depth_stencil: None,
                },
                CommandOp::BindGraphicsPipeline(pipeline),
                CommandOp::EndPass,
                CommandOp::Present {
                    texture,
                    subresources: full_range,
                },
            ],
        })
        .unwrap();
    assert_code(
        gal.submit(SubmissionBatch {
            label: "present-hazard".to_owned(),
            command_lists: vec![present_without_barrier],
        }),
        super::StatusCode::InvalidArgument,
    );

    let present_with_barrier = gal
        .create_command_list(CommandListDesc {
            label: "present-with-barrier".to_owned(),
            operations: vec![
                CommandOp::BeginPass {
                    pass,
                    target,
                    colors: vec![color_attachment(first_view)],
                    depth_stencil: None,
                },
                CommandOp::BindGraphicsPipeline(pipeline),
                CommandOp::EndPass,
                CommandOp::Barrier(ResourceBarrier {
                    resource: texture,
                    subresources: Some(full_range),
                    before: TextureUsageState::ColorAttachment,
                    after: TextureUsageState::Present,
                    src_queue: QueueClass::Graphics,
                    dst_queue: QueueClass::Present,
                }),
                CommandOp::Present {
                    texture,
                    subresources: full_range,
                },
            ],
        })
        .unwrap();
    gal.submit(SubmissionBatch {
        label: "present-separated".to_owned(),
        command_lists: vec![present_with_barrier],
    })
    .unwrap();

    let present_with_view_barrier = gal
        .create_command_list(CommandListDesc {
            label: "present-with-view-barrier".to_owned(),
            operations: vec![
                CommandOp::Barrier(ResourceBarrier {
                    resource: first_view,
                    subresources: Some(full_range),
                    before: TextureUsageState::ColorAttachment,
                    after: TextureUsageState::Present,
                    src_queue: QueueClass::Graphics,
                    dst_queue: QueueClass::Present,
                }),
                CommandOp::Present {
                    texture,
                    subresources: full_range,
                },
            ],
        })
        .unwrap();
    gal.submit(SubmissionBatch {
        label: "present-view-separated".to_owned(),
        command_lists: vec![present_with_view_barrier],
    })
    .unwrap();
}

#[test]
fn malformed_commands_and_usage_declarations_are_rejected() {
    let mut gal = gal();
    let (color_view, target, pass, _layout, pipeline) = simple_graphics_scene(&mut gal);
    let compute_layout = gal
        .create_pipeline_layout(PipelineLayoutDesc {
            label: "compute-layout".to_owned(),
            resource_layouts: vec![],
        })
        .unwrap();
    let compute_shader = gal
        .create_shader_module(shader("compute", ShaderStage::Compute))
        .unwrap();
    let compute = gal
        .create_compute_pipeline(ComputePipelineDesc {
            label: "compute".to_owned(),
            layout: compute_layout,
            shader: compute_shader,
        })
        .unwrap();
    let src = gal
        .create_buffer(buffer("src", vec![BufferUsage::TransferSrc]))
        .unwrap();
    let not_transfer_dst = gal
        .create_buffer(buffer("not-dst", vec![BufferUsage::Vertex]))
        .unwrap();

    assert_code(
        gal.create_command_list(CommandListDesc {
            label: "draw-without-pass".to_owned(),
            operations: vec![
                CommandOp::BindGraphicsPipeline(pipeline),
                CommandOp::Draw {
                    vertices: 3,
                    instances: 1,
                },
            ],
        }),
        super::StatusCode::InvalidArgument,
    );
    assert_code(
        gal.create_command_list(CommandListDesc {
            label: "dispatch-inside-pass".to_owned(),
            operations: vec![
                CommandOp::BeginPass {
                    pass,
                    target,
                    colors: vec![color_attachment(color_view)],
                    depth_stencil: None,
                },
                CommandOp::BindComputePipeline(compute),
                CommandOp::Dispatch {
                    groups_x: 1,
                    groups_y: 1,
                    groups_z: 1,
                },
                CommandOp::EndPass,
            ],
        }),
        super::StatusCode::InvalidArgument,
    );
    assert_code(
        gal.create_command_list(CommandListDesc {
            label: "bad-barrier".to_owned(),
            operations: vec![CommandOp::Barrier(ResourceBarrier {
                resource: src,
                subresources: None,
                before: TextureUsageState::TransferSrc,
                after: TextureUsageState::TransferSrc,
                src_queue: QueueClass::Transfer,
                dst_queue: QueueClass::Transfer,
            })],
        }),
        super::StatusCode::InvalidArgument,
    );
    assert_code(
        gal.create_command_list(CommandListDesc {
            label: "self-copy".to_owned(),
            operations: vec![CommandOp::CopyBuffer {
                src,
                dst: src,
                size: 16,
            }],
        }),
        super::StatusCode::InvalidArgument,
    );
    assert_code(
        gal.create_command_list(CommandListDesc {
            label: "copy-without-dst-usage".to_owned(),
            operations: vec![CommandOp::CopyBuffer {
                src,
                dst: not_transfer_dst,
                size: 16,
            }],
        }),
        super::StatusCode::InvalidArgument,
    );
    assert_code(
        gal.create_command_list(CommandListDesc {
            label: "index-without-index-usage".to_owned(),
            operations: vec![
                CommandOp::BeginPass {
                    pass,
                    target,
                    colors: vec![color_attachment(color_view)],
                    depth_stencil: None,
                },
                CommandOp::BindGraphicsPipeline(pipeline),
                CommandOp::SetIndexBuffer {
                    buffer: not_transfer_dst,
                    offset: 0,
                    index_type: IndexType::U32,
                },
                CommandOp::EndPass,
            ],
        }),
        super::StatusCode::InvalidArgument,
    );
}

#[test]
fn indexed_draws_validate_index_type_alignment_and_range() {
    let mut gal = gal();
    let (color_view, target, pass, _layout, pipeline) = simple_graphics_scene(&mut gal);
    let index = gal
        .create_buffer(BufferDesc {
            label: "typed-index".to_owned(),
            size: 12,
            memory: MemoryDomain::DeviceLocal,
            usages: vec![BufferUsage::Index],
        })
        .unwrap();
    let valid = |offset, index_type, indices| CommandListDesc {
        label: "typed-index-draw".to_owned(),
        operations: vec![
            CommandOp::BeginPass {
                pass,
                target,
                colors: vec![color_attachment(color_view)],
                depth_stencil: None,
            },
            CommandOp::BindGraphicsPipeline(pipeline),
            CommandOp::SetIndexBuffer {
                buffer: index,
                offset,
                index_type,
            },
            CommandOp::DrawIndexed {
                indices,
                instances: 1,
            },
            CommandOp::EndPass,
        ],
    };
    gal.create_command_list(valid(8, IndexType::U16, 2))
        .unwrap();
    gal.create_command_list(valid(4, IndexType::U32, 2))
        .unwrap();
    assert_code(
        gal.create_command_list(valid(1, IndexType::U16, 2)),
        super::StatusCode::InvalidArgument,
    );
    assert_code(
        gal.create_command_list(valid(8, IndexType::U32, 2)),
        super::StatusCode::InvalidArgument,
    );
}

#[test]
fn buffer_texture_copy_commands_validate_usage_ranges_and_hazards() {
    let mut gal = gal();
    let upload = gal
        .create_buffer(buffer("texture-upload", vec![BufferUsage::TransferSrc]))
        .unwrap();
    let readback = gal
        .create_buffer(buffer("texture-readback", vec![BufferUsage::TransferDst]))
        .unwrap();
    let texture_handle = gal
        .create_texture(texture(
            "copy-texture",
            TextureFormat::Rgba8Unorm,
            vec![TextureUsage::TransferDst, TextureUsage::TransferSrc],
        ))
        .unwrap();
    let full_range = TextureSubresourceRange {
        base_mip: 0,
        mip_count: 1,
        base_layer: 0,
        layer_count: 1,
    };
    let region = BufferImageCopyRegion {
        buffer: upload,
        buffer_offset: 0,
        bytes_per_row: 16,
        rows_per_image: 4,
        texture: texture_handle,
        texture_mip: 0,
        texture_layer: 0,
        texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
        extent: Extent3d {
            width: 4,
            height: 4,
            depth: 1,
        },
    };

    let read_region = BufferImageCopyRegion {
        buffer: readback,
        ..region.clone()
    };
    gal.create_command_list(CommandListDesc {
        label: "buffer-texture-transfer-with-barrier".to_owned(),
        operations: vec![
            CommandOp::CopyBufferToTexture(region.clone()),
            CommandOp::Barrier(ResourceBarrier {
                resource: texture_handle,
                subresources: Some(full_range),
                before: TextureUsageState::TransferDst,
                after: TextureUsageState::TransferSrc,
                src_queue: QueueClass::Transfer,
                dst_queue: QueueClass::Transfer,
            }),
            CommandOp::CopyTextureToBuffer(read_region.clone()),
        ],
    })
    .unwrap();

    let no_barrier = gal
        .create_command_list(CommandListDesc {
            label: "buffer-texture-transfer-without-barrier".to_owned(),
            operations: vec![
                CommandOp::CopyBufferToTexture(region.clone()),
                CommandOp::CopyTextureToBuffer(read_region.clone()),
            ],
        })
        .unwrap();
    assert_code(
        gal.submit(SubmissionBatch {
            label: "buffer-texture-hazard".to_owned(),
            command_lists: vec![no_barrier],
        }),
        super::StatusCode::InvalidArgument,
    );

    let transfer_src_only = gal
        .create_texture(texture(
            "transfer-src-only-texture",
            TextureFormat::Rgba8Unorm,
            vec![TextureUsage::TransferSrc],
        ))
        .unwrap();
    assert_code(
        gal.create_command_list(CommandListDesc {
            label: "copy-to-texture-without-dst-usage".to_owned(),
            operations: vec![CommandOp::CopyBufferToTexture(BufferImageCopyRegion {
                texture: transfer_src_only,
                ..region.clone()
            })],
        }),
        super::StatusCode::InvalidArgument,
    );

    assert_code(
        gal.create_command_list(CommandListDesc {
            label: "copy-to-texture-bad-row-layout".to_owned(),
            operations: vec![CommandOp::CopyBufferToTexture(BufferImageCopyRegion {
                bytes_per_row: 18,
                ..region.clone()
            })],
        }),
        super::StatusCode::InvalidArgument,
    );

    assert_code(
        gal.create_command_list(CommandListDesc {
            label: "copy-to-texture-outside-extent".to_owned(),
            operations: vec![CommandOp::CopyBufferToTexture(BufferImageCopyRegion {
                texture_origin: TextureOrigin3d { x: 127, y: 0, z: 0 },
                ..region
            })],
        }),
        super::StatusCode::InvalidArgument,
    );
}

#[test]
fn texture_row_reversal_is_capability_checked_and_retains_copy_validation() {
    for supported in [false, true] {
        let mut caps = vulkan_capabilities();
        caps.features.texture_row_reversal = supported;
        let mut gal = gal_with_capabilities(caps);
        let source = gal.create_texture(texture("row-source", TextureFormat::Rgba8Unorm,
            vec![TextureUsage::TransferSrc])).unwrap();
        let destination = gal.create_texture(texture("row-destination", TextureFormat::Rgba8Unorm,
            vec![TextureUsage::TransferDst])).unwrap();
        let region = TextureImageCopyRegion {
            row_order: TextureRowOrder::Reverse,
            src_texture: source, src_mip: 0, src_layer: 0,
            src_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
            dst_texture: destination, dst_mip: 0, dst_layer: 0,
            dst_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
            extent: Extent3d { width: 2, height: 3, depth: 1 },
        };
        let list = |region| CommandListDesc { label: "reverse-rows".into(),
            operations: vec![CommandOp::CopyTexture(region)] };
        if supported {
            gal.create_command_list(list(region.clone())).unwrap();
            assert_code(gal.create_command_list(list(TextureImageCopyRegion {
                dst_texture: source, ..region.clone()
            })), StatusCode::InvalidArgument);
            assert_code(gal.create_command_list(list(TextureImageCopyRegion {
                dst_origin: TextureOrigin3d { x: 0, y: 127, z: 0 }, ..region
            })), StatusCode::InvalidArgument);
        } else {
            assert_code(gal.create_command_list(list(region.clone())), StatusCode::UnsupportedFeature);
            gal.create_command_list(list(TextureImageCopyRegion {
                row_order: TextureRowOrder::Preserve, ..region
            })).unwrap();
        }
    }
}

#[test]
fn vulkan_texture_row_reversal_copies_exact_asymmetric_color_and_depth_rows() {
    use super::backends::vulkan::VulkanBackend;
    // This is deliberately a real device test: inability to create the backend
    // must not be reported as a passing pixel comparison.
    let backend = VulkanBackend::new("explicit row reversal conformance").unwrap();
    let mut gal = VulkanicGal::new_with_backend(Box::new(backend), false);
    for format in [TextureFormat::Rgba8Unorm, TextureFormat::Depth32Float] {
        let extent = Extent3d { width: 2, height: 3, depth: 1 };
        let pixels: Vec<u8> = if format == TextureFormat::Depth32Float {
            [0.125f32, 0.25, 0.375, 0.5, 0.625, 0.875].into_iter()
                .flat_map(f32::to_le_bytes).collect()
        } else { (1u8..=24).collect() };
        let source = gal.create_texture(TextureDesc {
            label: "row-source".into(), dimension: TextureDimension::D2, format, extent,
            mip_levels: 1, array_layers: 1,
            usages: vec![TextureUsage::TransferDst, TextureUsage::TransferSrc],
        }).unwrap();
        let destination = gal.create_texture(TextureDesc {
            label: "row-destination".into(), dimension: TextureDimension::D2, format, extent,
            mip_levels: 1, array_layers: 1,
            usages: vec![TextureUsage::TransferDst, TextureUsage::TransferSrc],
        }).unwrap();
        let upload = gal.create_buffer(BufferDesc {
            label: "row-upload".into(), size: 24, memory: MemoryDomain::Upload,
            usages: vec![BufferUsage::HostWrite, BufferUsage::TransferSrc],
        }).unwrap();
        let readback = gal.create_buffer(BufferDesc {
            label: "row-readback".into(), size: 24, memory: MemoryDomain::Readback,
            usages: vec![BufferUsage::TransferDst, BufferUsage::HostRead],
        }).unwrap();
        let barrier = |resource, before, after| CommandOp::Barrier(ResourceBarrier {
            resource, subresources: None, before, after,
            src_queue: QueueClass::Graphics, dst_queue: QueueClass::Graphics,
        });
        let copy = |buffer, texture| BufferImageCopyRegion {
            buffer, buffer_offset: 0, bytes_per_row: 8, rows_per_image: 3, texture,
            texture_mip: 0, texture_layer: 0,
            texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 }, extent,
        };
        let list = gal.create_command_list(CommandListDesc {
            label: "explicit-row-reversal".into(), operations: vec![
                CommandOp::HostWriteBuffer { buffer: upload, offset: 0, data: pixels.clone() },
                barrier(upload, TextureUsageState::TransferDst, TextureUsageState::TransferSrc),
                barrier(source, TextureUsageState::Undefined, TextureUsageState::TransferDst),
                CommandOp::CopyBufferToTexture(copy(upload, source)),
                barrier(source, TextureUsageState::TransferDst, TextureUsageState::TransferSrc),
                barrier(destination, TextureUsageState::Undefined, TextureUsageState::TransferDst),
                CommandOp::CopyTexture(TextureImageCopyRegion {
                    row_order: TextureRowOrder::Reverse,
                    src_texture: source, src_mip: 0, src_layer: 0,
                    src_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
                    dst_texture: destination, dst_mip: 0, dst_layer: 0,
                    dst_origin: TextureOrigin3d { x: 0, y: 0, z: 0 }, extent,
                }),
                barrier(destination, TextureUsageState::TransferDst, TextureUsageState::TransferSrc),
                CommandOp::CopyTextureToBuffer(copy(readback, destination)),
                barrier(readback, TextureUsageState::TransferDst, TextureUsageState::ShaderRead),
                CommandOp::HostReadBuffer { buffer: readback, offset: 0, size: 24 },
            ],
        }).unwrap();
        let token = gal.submit(SubmissionBatch {
            label: "explicit-row-reversal".into(), command_lists: vec![list],
        }).unwrap();
        gal.retire_through_for_test(token.submission).unwrap();
        let reads = gal.completed_host_reads();
        let actual = &reads.iter().rev().find(|read| read.buffer == readback).unwrap().bytes;
        let expected: Vec<u8> = pixels.chunks_exact(8).rev().flatten().copied().collect();
        assert_eq!(&expected, actual, "row reversal must preserve exact texels for {format:?}");
        for handle in [source, destination, upload, readback] { gal.destroy(handle).unwrap(); }
    }
}

#[test]
fn texture_copy_validates_depth_snapshot_regions_and_transfer_hazards() {
    let mut gal = gal();
    let source = gal
        .create_texture(texture(
            "depth-copy-source",
            TextureFormat::Depth32Float,
            vec![TextureUsage::TransferSrc],
        ))
        .unwrap();
    let destination = gal
        .create_texture(texture(
            "depth-copy-destination",
            TextureFormat::Depth32Float,
            vec![TextureUsage::TransferDst],
        ))
        .unwrap();
    let region = TextureImageCopyRegion {
        row_order: crate::render::vulkanic::commands::TextureRowOrder::Preserve,
        src_texture: source,
        src_mip: 0,
        src_layer: 0,
        src_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
        dst_texture: destination,
        dst_mip: 0,
        dst_layer: 0,
        dst_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
        extent: Extent3d {
            width: 4,
            height: 4,
            depth: 1,
        },
    };
    gal.create_command_list(CommandListDesc {
        label: "depth-snapshot-copy".to_owned(),
        operations: vec![
            CommandOp::Barrier(ResourceBarrier {
                resource: source,
                subresources: None,
                before: TextureUsageState::Undefined,
                after: TextureUsageState::TransferSrc,
                src_queue: QueueClass::Graphics,
                dst_queue: QueueClass::Transfer,
            }),
            CommandOp::Barrier(ResourceBarrier {
                resource: destination,
                subresources: None,
                before: TextureUsageState::Undefined,
                after: TextureUsageState::TransferDst,
                src_queue: QueueClass::Graphics,
                dst_queue: QueueClass::Transfer,
            }),
            CommandOp::CopyTexture(region.clone()),
        ],
    })
    .unwrap();

    let rgba_destination = gal
        .create_texture(texture(
            "depth-copy-rgba-destination",
            TextureFormat::Rgba8Unorm,
            vec![TextureUsage::TransferDst],
        ))
        .unwrap();
    assert_code(
        gal.create_command_list(CommandListDesc {
            label: "depth-copy-format-mismatch".to_owned(),
            operations: vec![CommandOp::CopyTexture(TextureImageCopyRegion {
                dst_texture: rgba_destination,
                ..region.clone()
            })],
        }),
        super::StatusCode::InvalidArgument,
    );
    assert_code(
        gal.create_command_list(CommandListDesc {
            label: "depth-copy-outside-extent".to_owned(),
            operations: vec![CommandOp::CopyTexture(TextureImageCopyRegion {
                dst_origin: TextureOrigin3d { x: 127, y: 0, z: 0 },
                ..region
            })],
        }),
        super::StatusCode::InvalidArgument,
    );
}

#[test]
fn d3_texture_validation_enforces_dimension_layers_mips_and_copy_boxes() {
    let mut gal = gal_with_capabilities(vulkan_capabilities());
    let upload = gal
        .create_buffer(buffer("d3-upload", vec![BufferUsage::TransferSrc]))
        .unwrap();
    let texture = gal
        .create_texture(TextureDesc {
            label: "d3-r8".to_owned(),
            dimension: TextureDimension::D3,
            format: TextureFormat::R8Uint,
            extent: Extent3d {
                width: 4,
                height: 4,
                depth: 2,
            },
            mip_levels: 1,
            array_layers: 1,
            usages: vec![
                TextureUsage::TransferDst,
                TextureUsage::TransferSrc,
                TextureUsage::Sampled,
            ],
        })
        .unwrap();
    let region = BufferImageCopyRegion {
        buffer: upload,
        buffer_offset: 0,
        bytes_per_row: 4,
        rows_per_image: 4,
        texture,
        texture_mip: 0,
        texture_layer: 0,
        texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
        extent: Extent3d {
            width: 4,
            height: 4,
            depth: 2,
        },
    };
    gal.create_command_list(CommandListDesc {
        label: "d3-full-upload".to_owned(),
        operations: vec![CommandOp::CopyBufferToTexture(region.clone())],
    })
    .unwrap();

    assert_code(
        gal.create_texture_view(TextureViewDesc {
            label: "d3-invalid-layer-view".to_owned(),
            texture,
            format: TextureFormat::R8Uint,
            base_mip: 0,
            mip_count: 1,
            base_layer: 1,
            layer_count: 1,
        }),
        super::StatusCode::InvalidArgument,
    );
    assert_code(
        gal.create_command_list(CommandListDesc {
            label: "d3-invalid-array-layer-copy".to_owned(),
            operations: vec![CommandOp::CopyBufferToTexture(BufferImageCopyRegion {
                texture_layer: 1,
                ..region.clone()
            })],
        }),
        super::StatusCode::InvalidArgument,
    );
    assert_code(
        gal.create_command_list(CommandListDesc {
            label: "d3-invalid-partial-row".to_owned(),
            operations: vec![CommandOp::CopyBufferToTexture(BufferImageCopyRegion {
                bytes_per_row: 3,
                ..region.clone()
            })],
        }),
        super::StatusCode::InvalidArgument,
    );
    assert_code(
        gal.create_command_list(CommandListDesc {
            label: "d3-invalid-depth-box".to_owned(),
            operations: vec![CommandOp::CopyBufferToTexture(BufferImageCopyRegion {
                texture_origin: TextureOrigin3d { x: 0, y: 0, z: 1 },
                ..region
            })],
        }),
        super::StatusCode::InvalidArgument,
    );
}

#[test]
fn d3_texture_lifecycle_validates_mips_storage_transitions_and_retirement() {
    let mut gal = gal_with_capabilities(vulkan_capabilities());
    let texture = gal
        .create_texture(TextureDesc {
            label: "d3-rgba16-storage".to_owned(),
            dimension: TextureDimension::D3,
            format: TextureFormat::Rgba16Float,
            extent: Extent3d {
                width: 8,
                height: 4,
                depth: 2,
            },
            mip_levels: 2,
            array_layers: 1,
            usages: vec![
                TextureUsage::Sampled,
                TextureUsage::Storage,
                TextureUsage::TransferDst,
            ],
        })
        .unwrap();
    let full_range = TextureSubresourceRange {
        base_mip: 0,
        mip_count: 2,
        base_layer: 0,
        layer_count: 1,
    };
    let list = gal
        .create_command_list(CommandListDesc {
            label: "d3-storage-transition".to_owned(),
            operations: vec![
                CommandOp::Barrier(ResourceBarrier {
                    resource: texture,
                    subresources: Some(full_range),
                    before: TextureUsageState::Undefined,
                    after: TextureUsageState::ShaderWrite,
                    src_queue: QueueClass::Graphics,
                    dst_queue: QueueClass::Graphics,
                }),
                CommandOp::Barrier(ResourceBarrier {
                    resource: texture,
                    subresources: Some(full_range),
                    before: TextureUsageState::ShaderWrite,
                    after: TextureUsageState::ShaderRead,
                    src_queue: QueueClass::Graphics,
                    dst_queue: QueueClass::Graphics,
                }),
            ],
        })
        .unwrap();
    let submission = gal
        .submit(SubmissionBatch {
            label: "d3-storage-transition-submit".to_owned(),
            command_lists: vec![list],
        })
        .unwrap();
    gal.destroy(texture).unwrap();
    assert!(gal
        .retire_through_for_test(submission.submission)
        .unwrap()
        .contains(&texture));
    assert_code(
        gal.create_texture_view(TextureViewDesc {
            label: "stale-d3-view".to_owned(),
            texture,
            format: TextureFormat::Rgba16Float,
            base_mip: 0,
            mip_count: 1,
            base_layer: 0,
            layer_count: 1,
        }),
        super::StatusCode::StaleHandle,
    );

    assert_code(
        gal.create_texture(TextureDesc {
            label: "d3-too-many-mips".to_owned(),
            dimension: TextureDimension::D3,
            format: TextureFormat::R8Uint,
            extent: Extent3d {
                width: 4,
                height: 4,
                depth: 4,
            },
            mip_levels: 4,
            array_layers: 1,
            usages: vec![TextureUsage::Sampled],
        }),
        super::StatusCode::InvalidArgument,
    );
}

#[test]
fn mip_generation_requires_explicit_ranges_usages_and_non_integer_color_formats() {
    let mut gal = gal_with_capabilities(vulkan_capabilities());
    let texture = gal
        .create_texture(TextureDesc {
            label: "mip-generation-color".to_owned(),
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            extent: Extent3d {
                width: 16,
                height: 8,
                depth: 1,
            },
            mip_levels: 5,
            array_layers: 1,
            usages: vec![
                TextureUsage::TransferSrc,
                TextureUsage::TransferDst,
                TextureUsage::Sampled,
            ],
        })
        .unwrap();
    let base_range = TextureSubresourceRange {
        base_mip: 0,
        mip_count: 1,
        base_layer: 0,
        layer_count: 1,
    };
    let descendant_range = TextureSubresourceRange {
        base_mip: 1,
        mip_count: 4,
        base_layer: 0,
        layer_count: 1,
    };
    let full_range = TextureSubresourceRange {
        base_mip: 0,
        mip_count: 5,
        base_layer: 0,
        layer_count: 1,
    };
    let list = gal
        .create_command_list(CommandListDesc {
            label: "mip-generation-valid".to_owned(),
            operations: vec![
                CommandOp::Barrier(ResourceBarrier {
                    resource: texture,
                    subresources: Some(base_range),
                    before: TextureUsageState::Undefined,
                    after: TextureUsageState::TransferSrc,
                    src_queue: QueueClass::Graphics,
                    dst_queue: QueueClass::Graphics,
                }),
                CommandOp::Barrier(ResourceBarrier {
                    resource: texture,
                    subresources: Some(descendant_range),
                    before: TextureUsageState::Undefined,
                    after: TextureUsageState::TransferDst,
                    src_queue: QueueClass::Graphics,
                    dst_queue: QueueClass::Graphics,
                }),
                CommandOp::GenerateMipmaps {
                    texture,
                    subresources: full_range,
                },
                CommandOp::Barrier(ResourceBarrier {
                    resource: texture,
                    subresources: Some(full_range),
                    before: TextureUsageState::TransferSrc,
                    after: TextureUsageState::ShaderRead,
                    src_queue: QueueClass::Graphics,
                    dst_queue: QueueClass::Graphics,
                }),
            ],
        })
        .unwrap();
    gal.submit(SubmissionBatch {
        label: "mip-generation-valid-submit".to_owned(),
        command_lists: vec![list],
    })
    .unwrap();

    assert_code(
        gal.create_command_list(CommandListDesc {
            label: "mip-generation-one-level".to_owned(),
            operations: vec![CommandOp::GenerateMipmaps {
                texture,
                subresources: base_range,
            }],
        }),
        super::StatusCode::InvalidArgument,
    );

    let integer = gal
        .create_texture(TextureDesc {
            label: "mip-generation-integer".to_owned(),
            dimension: TextureDimension::D2,
            format: TextureFormat::R8Uint,
            extent: Extent3d {
                width: 4,
                height: 4,
                depth: 1,
            },
            mip_levels: 3,
            array_layers: 1,
            usages: vec![TextureUsage::TransferSrc, TextureUsage::TransferDst],
        })
        .unwrap();
    assert_unsupported(
        gal.create_command_list(CommandListDesc {
            label: "mip-generation-integer-command".to_owned(),
            operations: vec![CommandOp::GenerateMipmaps {
                texture: integer,
                subresources: TextureSubresourceRange {
                    base_mip: 0,
                    mip_count: 3,
                    base_layer: 0,
                    layer_count: 1,
                },
            }],
        }),
        "non-depth, non-integer",
    );
}

#[test]
fn rejected_d3_copy_does_not_poison_the_following_valid_submission() {
    let mut gal = gal_with_capabilities(vulkan_capabilities());
    let upload = gal
        .create_buffer(BufferDesc {
            label: "d3-rollback-upload".to_owned(),
            size: 32,
            memory: MemoryDomain::Upload,
            usages: vec![BufferUsage::HostWrite, BufferUsage::TransferSrc],
        })
        .unwrap();
    let texture = gal
        .create_texture(TextureDesc {
            label: "d3-rollback-volume".to_owned(),
            dimension: TextureDimension::D3,
            format: TextureFormat::R8Uint,
            extent: Extent3d {
                width: 4,
                height: 4,
                depth: 2,
            },
            mip_levels: 1,
            array_layers: 1,
            usages: vec![TextureUsage::TransferDst, TextureUsage::Sampled],
        })
        .unwrap();
    let region = BufferImageCopyRegion {
        buffer: upload,
        buffer_offset: 0,
        bytes_per_row: 4,
        rows_per_image: 4,
        texture,
        texture_mip: 0,
        texture_layer: 0,
        texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
        extent: Extent3d {
            width: 4,
            height: 4,
            depth: 2,
        },
    };
    let subresources = TextureSubresourceRange {
        base_mip: 0,
        mip_count: 1,
        base_layer: 0,
        layer_count: 1,
    };
    assert_code(
        gal.create_command_list(CommandListDesc {
            label: "d3-rejected-copy".to_owned(),
            operations: vec![CommandOp::CopyBufferToTexture(BufferImageCopyRegion {
                bytes_per_row: 3,
                ..region.clone()
            })],
        }),
        super::StatusCode::InvalidArgument,
    );

    let list = gal
        .create_command_list(CommandListDesc {
            label: "d3-valid-copy-after-rejection".to_owned(),
            operations: vec![
                CommandOp::Barrier(ResourceBarrier {
                    resource: texture,
                    subresources: Some(subresources),
                    before: TextureUsageState::Undefined,
                    after: TextureUsageState::TransferDst,
                    src_queue: QueueClass::Graphics,
                    dst_queue: QueueClass::Graphics,
                }),
                CommandOp::CopyBufferToTexture(region),
                CommandOp::Barrier(ResourceBarrier {
                    resource: texture,
                    subresources: Some(subresources),
                    before: TextureUsageState::TransferDst,
                    after: TextureUsageState::ShaderRead,
                    src_queue: QueueClass::Graphics,
                    dst_queue: QueueClass::Graphics,
                }),
            ],
        })
        .unwrap();
    let submission = gal
        .submit(SubmissionBatch {
            label: "d3-valid-copy-after-rejection-submit".to_owned(),
            command_lists: vec![list],
        })
        .unwrap();
    gal.destroy(texture).unwrap();
    gal.destroy(upload).unwrap();
    let retired = gal.retire_through_for_test(submission.submission).unwrap();
    assert!(retired.contains(&texture));
    assert!(retired.contains(&upload));
}

#[test]
fn indirect_host_and_malformed_ranges_are_validated_semantically() {
    let mut gal = gal_with_capabilities(presentation_capabilities());
    let (color_view, target, pass, _layout, pipeline) = simple_graphics_scene(&mut gal);
    let not_indirect = gal
        .create_buffer(buffer("not-indirect", vec![BufferUsage::Vertex]))
        .unwrap();
    assert_code(
        gal.create_command_list(CommandListDesc {
            label: "draw-indirect-without-usage".to_owned(),
            operations: vec![
                CommandOp::BeginPass {
                    pass,
                    target,
                    colors: vec![color_attachment(color_view)],
                    depth_stencil: None,
                },
                CommandOp::BindGraphicsPipeline(pipeline),
                CommandOp::DrawIndirect {
                    buffer: not_indirect,
                    offset: 0,
                    draw_count: 1,
                },
                CommandOp::EndPass,
            ],
        }),
        super::StatusCode::InvalidArgument,
    );

    let indirect = gal
        .create_buffer(buffer("indirect", vec![BufferUsage::Indirect]))
        .unwrap();
    gal.create_command_list(CommandListDesc {
        label: "draw-indirect".to_owned(),
        operations: vec![
            CommandOp::BeginPass {
                pass,
                target,
                colors: vec![color_attachment(color_view)],
                depth_stencil: None,
            },
            CommandOp::BindGraphicsPipeline(pipeline),
            CommandOp::DrawIndirect {
                buffer: indirect,
                offset: 0,
                draw_count: 1,
            },
            CommandOp::EndPass,
        ],
    })
    .unwrap();

    let host_wrong_memory = gal
        .create_buffer(buffer("host-wrong-memory", vec![BufferUsage::HostWrite]))
        .unwrap();
    assert_code(
        gal.create_command_list(CommandListDesc {
            label: "host-write-wrong-memory".to_owned(),
            operations: vec![CommandOp::HostWriteBuffer {
                buffer: host_wrong_memory,
                offset: 0,
                data: vec![0; 16],
            }],
        }),
        super::StatusCode::InvalidArgument,
    );

    let upload = gal
        .create_buffer(BufferDesc {
            label: "upload".to_owned(),
            size: 64,
            memory: MemoryDomain::Upload,
            usages: vec![BufferUsage::HostWrite],
        })
        .unwrap();
    assert_code(
        gal.create_command_list(CommandListDesc {
            label: "host-write-inside-render-pass".to_owned(),
            operations: vec![
                CommandOp::BeginPass {
                    pass,
                    target,
                    colors: vec![color_attachment(color_view)],
                    depth_stencil: None,
                },
                CommandOp::HostWriteBuffer {
                    buffer: upload,
                    offset: 0,
                    data: vec![0; 16],
                },
                CommandOp::EndPass,
            ],
        }),
        super::StatusCode::InvalidArgument,
    );
    assert_code(
        gal.create_command_list(CommandListDesc {
            label: "barrier-inside-render-pass".to_owned(),
            operations: vec![
                CommandOp::BeginPass {
                    pass,
                    target,
                    colors: vec![color_attachment(color_view)],
                    depth_stencil: None,
                },
                CommandOp::Barrier(ResourceBarrier {
                    resource: upload,
                    subresources: None,
                    before: TextureUsageState::TransferDst,
                    after: TextureUsageState::ShaderRead,
                    src_queue: QueueClass::Graphics,
                    dst_queue: QueueClass::Graphics,
                }),
                CommandOp::EndPass,
            ],
        }),
        super::StatusCode::InvalidArgument,
    );
    assert_code(
        gal.create_command_list(CommandListDesc {
            label: "host-write-overflow".to_owned(),
            operations: vec![CommandOp::HostWriteBuffer {
                buffer: upload,
                offset: u64::MAX,
                data: vec![0; 16],
            }],
        }),
        super::StatusCode::InvalidArgument,
    );
    assert_code(
        gal.create_command_list(CommandListDesc {
            label: "host-write-outside".to_owned(),
            operations: vec![CommandOp::HostWriteBuffer {
                buffer: upload,
                offset: 32,
                data: vec![0; 64],
            }],
        }),
        super::StatusCode::InvalidArgument,
    );

    let not_present = gal
        .create_texture(texture(
            "not-present",
            TextureFormat::Rgba8Unorm,
            vec![TextureUsage::Sampled],
        ))
        .unwrap();
    assert_code(
        gal.create_command_list(CommandListDesc {
            label: "present-without-usage".to_owned(),
            operations: vec![CommandOp::Present {
                texture: not_present,
                subresources: TextureSubresourceRange {
                    base_mip: 0,
                    mip_count: 1,
                    base_layer: 0,
                    layer_count: 1,
                },
            }],
        }),
        super::StatusCode::InvalidArgument,
    );
    let present_texture = gal
        .create_texture(texture(
            "present",
            TextureFormat::Rgba8Unorm,
            vec![TextureUsage::Present],
        ))
        .unwrap();
    assert_code(
        gal.create_command_list(CommandListDesc {
            label: "present-bad-range".to_owned(),
            operations: vec![CommandOp::Present {
                texture: present_texture,
                subresources: TextureSubresourceRange {
                    base_mip: 0,
                    mip_count: 0,
                    base_layer: 0,
                    layer_count: 1,
                },
            }],
        }),
        super::StatusCode::InvalidArgument,
    );
}

#[test]
fn storage_and_subresource_hazards_are_conservative() {
    let mut gal = gal();
    let storage = gal
        .create_buffer(BufferDesc {
            label: "storage".to_owned(),
            size: 128,
            memory: MemoryDomain::Readback,
            usages: vec![
                BufferUsage::Storage,
                BufferUsage::HostRead,
                BufferUsage::TransferDst,
            ],
        })
        .unwrap();
    let layout = gal
        .create_resource_layout(ResourceLayoutDesc {
            label: "storage-layout".to_owned(),
            bindings: vec![layout_binding(
                0,
                ResourceBindingKind::StorageBuffer,
                PipelineStageFlags::COMPUTE,
            )],
        })
        .unwrap();
    let set = gal
        .create_resource_set(ResourceSetDesc {
            label: "storage-set".to_owned(),
            layout,
            bindings: vec![resource_binding(
                0,
                storage,
                ResourceBindingKind::StorageBuffer,
                AccessFlags::WRITE,
            )],
        })
        .unwrap();
    let pipeline_layout = gal
        .create_pipeline_layout(PipelineLayoutDesc {
            label: "storage-pipeline-layout".to_owned(),
            resource_layouts: vec![layout],
        })
        .unwrap();
    let shader = gal
        .create_shader_module(shader("storage-compute", ShaderStage::Compute))
        .unwrap();
    let pipeline = gal
        .create_compute_pipeline(ComputePipelineDesc {
            label: "storage-pipeline".to_owned(),
            layout: pipeline_layout,
            shader,
        })
        .unwrap();
    let list = gal
        .create_command_list(CommandListDesc {
            label: "storage-write-then-host-read".to_owned(),
            operations: vec![
                CommandOp::BindComputePipeline(pipeline),
                CommandOp::BindResourceSet {
                    pipeline_layout,
                    set_index: 0,
                    set,
                    dynamic_offsets: Vec::new(),
                },
                CommandOp::Dispatch {
                    groups_x: 1,
                    groups_y: 1,
                    groups_z: 1,
                },
                CommandOp::HostReadBuffer {
                    buffer: storage,
                    offset: 0,
                    size: 16,
                },
            ],
        })
        .unwrap();
    assert_code(
        gal.submit(SubmissionBatch {
            label: "storage-hazard".to_owned(),
            command_lists: vec![list],
        }),
        super::StatusCode::InvalidArgument,
    );
}

#[test]
fn texture_subresource_hazards_respect_non_overlapping_ranges() {
    let mut gal = gal();
    let texture = gal
        .create_texture(TextureDesc {
            label: "storage-texture".to_owned(),
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            extent: Extent3d {
                width: 64,
                height: 64,
                depth: 1,
            },
            mip_levels: 2,
            array_layers: 1,
            usages: vec![TextureUsage::Storage],
        })
        .unwrap();
    let mip0 = gal
        .create_texture_view(TextureViewDesc {
            label: "mip0".to_owned(),
            texture,
            format: TextureFormat::Rgba8Unorm,
            base_mip: 0,
            mip_count: 1,
            base_layer: 0,
            layer_count: 1,
        })
        .unwrap();
    let mip1 = gal
        .create_texture_view(TextureViewDesc {
            label: "mip1".to_owned(),
            texture,
            format: TextureFormat::Rgba8Unorm,
            base_mip: 1,
            mip_count: 1,
            base_layer: 0,
            layer_count: 1,
        })
        .unwrap();
    let layout = gal
        .create_resource_layout(ResourceLayoutDesc {
            label: "texture-storage-layout".to_owned(),
            bindings: vec![
                layout_binding(
                    0,
                    ResourceBindingKind::StorageTexture,
                    PipelineStageFlags::COMPUTE,
                ),
                layout_binding(
                    1,
                    ResourceBindingKind::StorageTexture,
                    PipelineStageFlags::COMPUTE,
                ),
            ],
        })
        .unwrap();
    let pipeline_layout = gal
        .create_pipeline_layout(PipelineLayoutDesc {
            label: "texture-storage-pipeline-layout".to_owned(),
            resource_layouts: vec![layout],
        })
        .unwrap();
    let shader = gal
        .create_shader_module(shader("texture-compute", ShaderStage::Compute))
        .unwrap();
    let pipeline = gal
        .create_compute_pipeline(ComputePipelineDesc {
            label: "texture-compute".to_owned(),
            layout: pipeline_layout,
            shader,
        })
        .unwrap();

    let disjoint = gal
        .create_resource_set(ResourceSetDesc {
            label: "disjoint-textures".to_owned(),
            layout,
            bindings: vec![
                resource_binding(
                    0,
                    mip0,
                    ResourceBindingKind::StorageTexture,
                    AccessFlags::WRITE,
                ),
                resource_binding(
                    1,
                    mip1,
                    ResourceBindingKind::StorageTexture,
                    AccessFlags::WRITE,
                ),
            ],
        })
        .unwrap();
    let disjoint_list = gal
        .create_command_list(CommandListDesc {
            label: "disjoint".to_owned(),
            operations: vec![
                CommandOp::BindComputePipeline(pipeline),
                CommandOp::BindResourceSet {
                    pipeline_layout,
                    set_index: 0,
                    set: disjoint,
                    dynamic_offsets: Vec::new(),
                },
                CommandOp::Dispatch {
                    groups_x: 1,
                    groups_y: 1,
                    groups_z: 1,
                },
            ],
        })
        .unwrap();
    gal.submit(SubmissionBatch {
        label: "disjoint-subresources".to_owned(),
        command_lists: vec![disjoint_list],
    })
    .unwrap();

    let overlapping = gal
        .create_resource_set(ResourceSetDesc {
            label: "overlapping-textures".to_owned(),
            layout,
            bindings: vec![
                resource_binding(
                    0,
                    mip0,
                    ResourceBindingKind::StorageTexture,
                    AccessFlags::WRITE,
                ),
                resource_binding(
                    1,
                    mip0,
                    ResourceBindingKind::StorageTexture,
                    AccessFlags::WRITE,
                ),
            ],
        })
        .unwrap();
    let overlapping_list = gal
        .create_command_list(CommandListDesc {
            label: "overlapping".to_owned(),
            operations: vec![
                CommandOp::BindComputePipeline(pipeline),
                CommandOp::BindResourceSet {
                    pipeline_layout,
                    set_index: 0,
                    set: overlapping,
                    dynamic_offsets: Vec::new(),
                },
                CommandOp::Dispatch {
                    groups_x: 1,
                    groups_y: 1,
                    groups_z: 1,
                },
            ],
        })
        .unwrap();
    assert_code(
        gal.submit(SubmissionBatch {
            label: "overlapping-subresources".to_owned(),
            command_lists: vec![overlapping_list],
        }),
        super::StatusCode::InvalidArgument,
    );
}

#[test]
fn resource_set_binding_requires_the_active_pipeline_layout() {
    let mut gal = gal();
    let (color_view, target, pass, _empty_layout, empty_pipeline) = simple_graphics_scene(&mut gal);
    let uniform = gal
        .create_buffer(buffer("uniform", vec![BufferUsage::Uniform]))
        .unwrap();
    let set_layout = gal
        .create_resource_layout(ResourceLayoutDesc {
            label: "set-layout".to_owned(),
            bindings: vec![layout_binding(
                0,
                ResourceBindingKind::UniformBuffer,
                PipelineStageFlags::DRAW,
            )],
        })
        .unwrap();
    let set = gal
        .create_resource_set(ResourceSetDesc {
            label: "set".to_owned(),
            layout: set_layout,
            bindings: vec![resource_binding(
                0,
                uniform,
                ResourceBindingKind::UniformBuffer,
                AccessFlags::READ,
            )],
        })
        .unwrap();
    let pipeline_layout = gal
        .create_pipeline_layout(PipelineLayoutDesc {
            label: "set-pipeline-layout".to_owned(),
            resource_layouts: vec![set_layout],
        })
        .unwrap();

    assert_code(
        gal.create_command_list(CommandListDesc {
            label: "bind-set-without-matching-active-layout".to_owned(),
            operations: vec![
                CommandOp::BeginPass {
                    pass,
                    target,
                    colors: vec![color_attachment(color_view)],
                    depth_stencil: None,
                },
                CommandOp::BindGraphicsPipeline(empty_pipeline),
                CommandOp::BindResourceSet {
                    pipeline_layout,
                    set_index: 0,
                    set,
                    dynamic_offsets: Vec::new(),
                },
                CommandOp::EndPass,
            ],
        }),
        super::StatusCode::InvalidArgument,
    );
}

#[test]
fn command_order_submission_ids_and_deferred_retirement_are_deterministic() {
    let mut gal = gal();
    let src = gal
        .create_buffer(buffer("src", vec![BufferUsage::TransferSrc]))
        .unwrap();
    let dst = gal
        .create_buffer(buffer("dst", vec![BufferUsage::TransferDst]))
        .unwrap();
    let ops = vec![CommandOp::CopyBuffer {
        src,
        dst,
        size: 256,
    }];
    let mut caller_ops = ops.clone();
    let list = gal
        .create_command_list(CommandListDesc {
            label: "copy-list".to_owned(),
            operations: caller_ops.clone(),
        })
        .unwrap();
    caller_ops.clear();
    assert_eq!(list.operations, ops);

    let token = gal
        .submit(SubmissionBatch {
            label: "copy-batch".to_owned(),
            command_lists: vec![list],
        })
        .unwrap();
    assert_eq!(token.submission.0, 1);
    assert_eq!(gal.mock_backend().unwrap().encoded_batches, 1);
    assert_eq!(
        gal.mock_backend().unwrap().submissions,
        vec![token.submission]
    );
    assert_eq!(
        gal.mock_backend()
            .unwrap()
            .submitted_labels
            .front()
            .unwrap(),
        "copy-batch"
    );

    gal.destroy(src).unwrap();
    assert_eq!(gal.metrics().deferred_retires, 1);
    assert_eq!(gal.metrics().resource_destroys, 0);

    gal.mock_backend_mut()
        .unwrap()
        .complete_through(token.submission);
    assert_eq!(gal.retire_completed().unwrap(), vec![src]);
    assert_eq!(gal.metrics().resource_destroys, 1);
}

#[test]
fn completion_poll_reports_backend_progress_without_implicit_wait() {
    let mut gal = gal();
    let buffer = gal
        .create_buffer(BufferDesc {
            label: "completion-buffer".to_string(),
            size: 16,
            memory: MemoryDomain::Upload,
            usages: vec![BufferUsage::HostWrite],
        })
        .unwrap();
    let list = gal
        .create_command_list(CommandListDesc {
            label: "completion-list".to_string(),
            operations: vec![CommandOp::HostWriteBuffer {
                buffer,
                offset: 0,
                data: vec![1; 16],
            }],
        })
        .unwrap();
    let token = gal
        .submit(SubmissionBatch {
            label: "completion-batch".to_string(),
            command_lists: vec![list],
        })
        .unwrap();

    assert_eq!(gal.latest_submission_id(), token.submission);
    assert_eq!(gal.poll_completed(), SubmissionId(0));
    assert_eq!(
        completion_result_for(token.submission, gal.poll_completed()).is_complete,
        0
    );

    gal.mock_backend_mut()
        .unwrap()
        .complete_through(token.submission);
    assert_eq!(gal.poll_completed(), token.submission);
    assert_eq!(
        completion_result_for(token.submission, gal.poll_completed()).is_complete,
        1
    );
}

#[test]
fn malformed_submission_is_rejected_before_backend_encoding() {
    let mut gal = gal();
    let (color_view, target, pass, _layout, _pipeline) = simple_graphics_scene(&mut gal);
    let stale_view = {
        let texture = gal
            .create_texture(texture(
                "temporary",
                TextureFormat::Rgba8Unorm,
                vec![TextureUsage::ColorAttachment],
            ))
            .unwrap();
        let view = gal
            .create_texture_view(view("temporary-view", texture, TextureFormat::Rgba8Unorm))
            .unwrap();
        gal.destroy(view).unwrap();
        gal.destroy(texture).unwrap();
        view
    };
    let list = CommandList {
        label: "bad-direct-list".to_owned(),
        operations: vec![
            CommandOp::BeginPass {
                pass,
                target,
                colors: vec![color_attachment(stale_view)],
                depth_stencil: None,
            },
            CommandOp::EndPass,
        ],
    };
    assert_code(
        gal.submit(SubmissionBatch {
            label: "bad-batch".to_owned(),
            command_lists: vec![list],
        }),
        super::StatusCode::InvalidArgument,
    );
    assert_eq!(gal.mock_backend().unwrap().encoded_batches, 0);
    assert!(gal.mock_backend().unwrap().submissions.is_empty());
    assert_ne!(stale_view, color_view);
}

#[test]
fn partial_backend_create_failure_does_not_consume_handle_identity() {
    let mut gal = gal();
    gal.mock_backend_mut().unwrap().fail_next_create();
    let error = gal
        .create_buffer(buffer("fails", vec![BufferUsage::Vertex]))
        .unwrap_err();
    assert_eq!(error.domain, ErrorDomain::Backend);
    assert_eq!(gal.metrics().resource_creates, 0);

    let handle = gal
        .create_buffer(buffer("after-failure", vec![BufferUsage::Vertex]))
        .unwrap();
    assert_eq!(handle.index(), 0);
    assert_eq!(handle.generation(), 1);
}

#[test]
fn generation_exhaustion_is_explicit() {
    let mut gal = gal();
    let handle = gal
        .create_buffer(buffer("last-generation", vec![BufferUsage::Vertex]))
        .unwrap();
    let max_generation_handle =
        Handle::new(HandleKind::Buffer, handle.index(), MAX_GENERATION).unwrap();
    gal.force_buffer_generation_for_test(handle, MAX_GENERATION);
    assert_code(
        gal.destroy(max_generation_handle),
        super::StatusCode::GenerationExhausted,
    );
}

#[test]
fn metrics_and_tracy_zones_are_gated() {
    let mut disabled = Metrics::new(false);
    {
        let _zone = disabled.zone("frame");
    }
    assert_eq!(disabled.zones["frame"].count, 1);
    assert_eq!(disabled.zones["frame"].total_nanos, 0);

    let mut enabled = Metrics::new(true);
    {
        let _zone = enabled.zone("frame");
        std::thread::sleep(std::time::Duration::from_micros(20));
    }
    assert_eq!(enabled.zones["frame"].count, 1);
    assert!(enabled.zones["frame"].total_nanos > 0);
}

#[test]
fn ffi_payload_validation_rejects_malformed_inputs() {
    unsafe {
        assert_code(
            validate_buffer_create_request(std::ptr::null()),
            super::StatusCode::NullPointer,
        );

        let request = FfiBufferCreateRequest {
            header: FfiHeader {
                version: FFI_ABI_VERSION,
                byte_size: size_of::<FfiBufferCreateRequest>() as u32,
            },
            label: FfiBytes {
                ptr: std::ptr::null(),
                len: 0,
            },
            size: 64,
            memory_domain: 99,
            usage_bits: 1,
        };
        assert_code(
            validate_buffer_create_request(&request),
            super::StatusCode::UnknownEnum,
        );

        let misaligned = (&request as *const _ as usize + 1) as *const FfiBufferCreateRequest;
        if (misaligned as usize) % align_of::<FfiBufferCreateRequest>() != 0 {
            assert_code(
                validate_buffer_create_request(misaligned),
                super::StatusCode::Alignment,
            );
        }

        let huge_lists = FfiSlice::<FfiCommandListRequest> {
            ptr: NonNull::<FfiCommandListRequest>::dangling().as_ptr(),
            count: u64::MAX,
        };
        let huge_request = FfiSubmissionRequest {
            header: FfiHeader {
                version: FFI_ABI_VERSION,
                byte_size: size_of::<FfiSubmissionRequest>() as u32,
            },
            label: FfiBytes {
                ptr: std::ptr::null(),
                len: 0,
            },
            command_lists: huge_lists,
        };
        assert_code(
            validate_submission_request(&huge_request),
            super::StatusCode::LengthOverflow,
        );

        let op_bytes = [1_u8, 2, 3, 4];
        let resource_uses = [FfiResourceUse {
            resource: FfiHandle { raw: 0 },
            stage_bits: 1,
            access_bits: 1,
        }];
        let list = FfiCommandListRequest {
            header: FfiHeader {
                version: FFI_ABI_VERSION,
                byte_size: size_of::<FfiCommandListRequest>() as u32,
            },
            label: FfiBytes {
                ptr: std::ptr::null(),
                len: 0,
            },
            encoded_ops: FfiBytes {
                ptr: op_bytes.as_ptr(),
                len: op_bytes.len() as u64,
            },
            resource_uses: FfiSlice {
                ptr: resource_uses.as_ptr(),
                count: resource_uses.len() as u64,
            },
        };
        let request = FfiSubmissionRequest {
            header: FfiHeader {
                version: FFI_ABI_VERSION,
                byte_size: size_of::<FfiSubmissionRequest>() as u32,
            },
            label: FfiBytes {
                ptr: std::ptr::null(),
                len: 0,
            },
            command_lists: FfiSlice {
                ptr: &list,
                count: 1,
            },
        };
        assert_code(
            validate_submission_request(&request),
            super::StatusCode::InvalidArgument,
        );
    }
}

fn ffi_header_for<T>() -> FfiHeader {
    FfiHeader {
        version: FFI_ABI_VERSION,
        byte_size: size_of::<T>() as u32,
    }
}

fn ffi_bytes(bytes: &[u8]) -> FfiBytes {
    FfiBytes {
        ptr: bytes.as_ptr(),
        len: bytes.len() as u64,
    }
}

fn ffi_slice<T>(items: &[T]) -> FfiSlice<T> {
    FfiSlice {
        ptr: items.as_ptr(),
        count: items.len() as u64,
    }
}

fn empty_ffi_slice<T>() -> FfiSlice<T> {
    FfiSlice {
        ptr: std::ptr::null(),
        count: 0,
    }
}

fn ffi_handle(kind: HandleKind, index: u32) -> FfiHandle {
    Handle::new(kind, index, 1).unwrap().into()
}

fn test_handle(kind: HandleKind, index: u32) -> Handle {
    Handle::new(kind, index, 1).unwrap()
}

fn minimal_begin_pass() -> CommandOp {
    CommandOp::BeginPass {
        pass: test_handle(HandleKind::RenderPass, 1),
        target: test_handle(HandleKind::RenderTarget, 1),
        colors: vec![PassAttachment {
            view: test_handle(HandleKind::TextureView, 1),
            load_op: AttachmentLoadOp::Load,
            store_op: AttachmentStoreOp::Store,
            clear_color: None,
        }],
        depth_stencil: None,
    }
}

fn normalize_ops_for_test(
    operations: Vec<CommandOp>,
) -> (CommandNormalizationStats, Vec<CommandOp>) {
    let mut batch = SubmissionBatch {
        label: "normalizer-test".to_owned(),
        command_lists: vec![CommandList {
            label: "main".to_owned(),
            operations,
        }],
    };
    let stats = normalize_submission_batch(&mut batch);
    let list = batch.command_lists.pop().unwrap();
    (stats, list.operations)
}

#[test]
fn command_normalization_removes_redundant_state_binds() {
    let pipeline = test_handle(HandleKind::GraphicsPipeline, 1);
    let layout = test_handle(HandleKind::PipelineLayout, 1);
    let set = test_handle(HandleKind::ResourceSet, 1);
    let vertex = test_handle(HandleKind::Buffer, 1);
    let index = test_handle(HandleKind::Buffer, 2);

    let (stats, operations) = normalize_ops_for_test(vec![
        minimal_begin_pass(),
        CommandOp::BindGraphicsPipeline(pipeline),
        CommandOp::BindGraphicsPipeline(pipeline),
        CommandOp::BindResourceSet {
            pipeline_layout: layout,
            set_index: 0,
            set,
            dynamic_offsets: Vec::new(),
        },
        CommandOp::BindResourceSet {
            pipeline_layout: layout,
            set_index: 0,
            set,
            dynamic_offsets: Vec::new(),
        },
        CommandOp::SetVertexBuffer {
            slot: 0,
            buffer: vertex,
            offset: 64,
        },
        CommandOp::SetVertexBuffer {
            slot: 0,
            buffer: vertex,
            offset: 64,
        },
        CommandOp::SetIndexBuffer {
            buffer: index,
            offset: 0,
            index_type: IndexType::U16,
        },
        CommandOp::SetIndexBuffer {
            buffer: index,
            offset: 0,
            index_type: IndexType::U16,
        },
        CommandOp::DrawIndexed {
            indices: 6,
            instances: 1,
        },
        CommandOp::EndPass,
    ]);

    assert_eq!(stats.ops_before, 11);
    assert_eq!(stats.ops_after, 7);
    assert_eq!(stats.pipeline_binds_removed, 1);
    assert_eq!(stats.resource_set_binds_removed, 1);
    assert_eq!(stats.vertex_buffer_binds_removed, 1);
    assert_eq!(stats.index_buffer_binds_removed, 1);
    assert!(matches!(operations[1], CommandOp::BindGraphicsPipeline(_)));
    assert!(matches!(operations[2], CommandOp::BindResourceSet { .. }));
    assert!(matches!(operations[3], CommandOp::SetVertexBuffer { .. }));
    assert!(matches!(operations[4], CommandOp::SetIndexBuffer { .. }));
    assert!(matches!(operations[5], CommandOp::DrawIndexed { .. }));
}

#[test]
fn command_normalization_keeps_resource_binds_with_distinct_dynamic_offsets() {
    let layout = test_handle(HandleKind::PipelineLayout, 1);
    let set = test_handle(HandleKind::ResourceSet, 1);

    let (stats, operations) = normalize_ops_for_test(vec![
        minimal_begin_pass(),
        CommandOp::BindResourceSet {
            pipeline_layout: layout,
            set_index: 0,
            set,
            dynamic_offsets: vec![0],
        },
        CommandOp::BindResourceSet {
            pipeline_layout: layout,
            set_index: 0,
            set,
            dynamic_offsets: vec![256],
        },
        CommandOp::BindResourceSet {
            pipeline_layout: layout,
            set_index: 0,
            set,
            dynamic_offsets: vec![256],
        },
        CommandOp::EndPass,
    ]);

    assert_eq!(stats.resource_set_binds_removed, 1);
    assert_eq!(
        2,
        operations
            .iter()
            .filter(|op| matches!(op, CommandOp::BindResourceSet { .. }))
            .count()
    );
}

#[test]
fn command_normalization_respects_pass_and_barrier_boundaries() {
    let pipeline = test_handle(HandleKind::GraphicsPipeline, 1);
    let texture = test_handle(HandleKind::Texture, 1);
    let barrier = CommandOp::Barrier(ResourceBarrier {
        resource: texture,
        subresources: None,
        before: TextureUsageState::TransferDst,
        after: TextureUsageState::ShaderRead,
        src_queue: QueueClass::Graphics,
        dst_queue: QueueClass::Graphics,
    });

    let (stats, operations) = normalize_ops_for_test(vec![
        minimal_begin_pass(),
        CommandOp::BindGraphicsPipeline(pipeline),
        CommandOp::BindGraphicsPipeline(pipeline),
        barrier,
        CommandOp::BindGraphicsPipeline(pipeline),
        CommandOp::EndPass,
        minimal_begin_pass(),
        CommandOp::BindGraphicsPipeline(pipeline),
        CommandOp::EndPass,
    ]);

    assert_eq!(stats.ops_before, 9);
    assert_eq!(stats.ops_after, 8);
    assert_eq!(stats.pipeline_binds_removed, 1);
    let kept_pipeline_binds = operations
        .iter()
        .filter(|op| matches!(op, CommandOp::BindGraphicsPipeline(_)))
        .count();
    assert_eq!(kept_pipeline_binds, 3);
}

#[test]
fn command_normalization_keeps_distinct_state_changes() {
    let pipeline_a = test_handle(HandleKind::GraphicsPipeline, 1);
    let pipeline_b = test_handle(HandleKind::GraphicsPipeline, 2);
    let layout = test_handle(HandleKind::PipelineLayout, 1);
    let set_a = test_handle(HandleKind::ResourceSet, 1);
    let set_b = test_handle(HandleKind::ResourceSet, 2);
    let index = test_handle(HandleKind::Buffer, 1);

    let (stats, operations) = normalize_ops_for_test(vec![
        minimal_begin_pass(),
        CommandOp::BindGraphicsPipeline(pipeline_a),
        CommandOp::BindGraphicsPipeline(pipeline_b),
        CommandOp::BindResourceSet {
            pipeline_layout: layout,
            set_index: 0,
            set: set_a,
            dynamic_offsets: Vec::new(),
        },
        CommandOp::BindResourceSet {
            pipeline_layout: layout,
            set_index: 0,
            set: set_b,
            dynamic_offsets: Vec::new(),
        },
        CommandOp::SetIndexBuffer {
            buffer: index,
            offset: 0,
            index_type: IndexType::U16,
        },
        CommandOp::SetIndexBuffer {
            buffer: index,
            offset: 2,
            index_type: IndexType::U16,
        },
        CommandOp::SetIndexBuffer {
            buffer: index,
            offset: 2,
            index_type: IndexType::U32,
        },
        CommandOp::EndPass,
    ]);

    assert_eq!(stats.ops_before, 9);
    assert_eq!(stats.ops_after, 9);
    assert_eq!(stats.pipeline_binds_removed, 0);
    assert_eq!(stats.resource_set_binds_removed, 0);
    assert_eq!(stats.index_buffer_binds_removed, 0);
    assert_eq!(operations.len(), 9);
}

#[test]
fn hazard_tracking_ignores_unrelated_resources() {
    let mut gal = gal();
    let mut operations = Vec::new();
    for index in 0..64 {
        let handle = gal
            .create_buffer(BufferDesc {
                label: format!("unrelated-write-{index}"),
                size: 16,
                memory: MemoryDomain::Upload,
                usages: vec![BufferUsage::HostWrite],
            })
            .unwrap();
        operations.push(CommandOp::HostWriteBuffer {
            buffer: handle,
            offset: 0,
            data: vec![index as u8; 4],
        });
    }
    let list = gal
        .create_command_list(CommandListDesc {
            label: "unrelated-writes".to_owned(),
            operations,
        })
        .unwrap();
    let mut profile = WholeFrameProfile::default();
    gal.submit_profiled(
        SubmissionBatch {
            label: "unrelated-write-submit".to_owned(),
            command_lists: vec![list],
        },
        &mut profile,
    )
    .unwrap();

    assert_eq!(profile.gal_hazard_write_events, 64);
    assert_eq!(profile.gal_hazard_candidates_examined, 0);
    assert_eq!(profile.gal_hazard_active_write_entries, 64);
}

#[test]
fn hazard_tracking_still_checks_same_resource_ranges() {
    let mut gal = gal();
    let buffer = gal
        .create_buffer(BufferDesc {
            label: "same-buffer".to_owned(),
            size: 64,
            memory: MemoryDomain::Upload,
            usages: vec![BufferUsage::HostWrite],
        })
        .unwrap();
    let list = gal
        .create_command_list(CommandListDesc {
            label: "disjoint-same-buffer-writes".to_owned(),
            operations: vec![
                CommandOp::HostWriteBuffer {
                    buffer,
                    offset: 0,
                    data: vec![1; 4],
                },
                CommandOp::HostWriteBuffer {
                    buffer,
                    offset: 8,
                    data: vec![2; 4],
                },
                CommandOp::HostWriteBuffer {
                    buffer,
                    offset: 16,
                    data: vec![3; 4],
                },
                CommandOp::HostWriteBuffer {
                    buffer,
                    offset: 32,
                    data: vec![4; 4],
                },
            ],
        })
        .unwrap();
    let mut profile = WholeFrameProfile::default();
    gal.submit_profiled(
        SubmissionBatch {
            label: "disjoint-same-buffer-submit".to_owned(),
            command_lists: vec![list],
        },
        &mut profile,
    )
    .unwrap();
    assert_eq!(profile.gal_hazard_write_events, 4);
    assert_eq!(profile.gal_hazard_read_events, 0);
    assert_eq!(profile.gal_hazard_candidates_examined, 6);

    let overlapping = gal
        .create_command_list(CommandListDesc {
            label: "overlapping-same-buffer-write-read".to_owned(),
            operations: vec![
                CommandOp::HostWriteBuffer {
                    buffer,
                    offset: 0,
                    data: vec![1; 8],
                },
                CommandOp::HostWriteBuffer {
                    buffer,
                    offset: 4,
                    data: vec![2; 4],
                },
            ],
        })
        .unwrap();
    assert_code(
        gal.submit(SubmissionBatch {
            label: "overlapping-same-buffer-submit".to_owned(),
            command_lists: vec![overlapping],
        }),
        super::StatusCode::InvalidArgument,
    );
}

#[test]
fn storage_binding_hazards_use_explicit_ranges_and_effective_dynamic_offsets() {
    for overrides in [false, true] {
        for second_access in [AccessFlags::READ, AccessFlags::WRITE] {
            for (second_offset, accepted) in [(256u64, true), (128, false), (0, false)] {
                let mut gal = gal();
                let buffer = gal.create_buffer(BufferDesc { label: "ranged-storage".into(),
                    size: 1024, memory: MemoryDomain::DeviceLocal, usages: vec![BufferUsage::Storage] }).unwrap();
                let layout = gal.create_resource_layout(ResourceLayoutDesc {
                    label: "ranged-storage-layout".into(), bindings: vec![ResourceBindingDesc {
                        binding: 0, kind: ResourceBindingKind::StorageBuffer, stages: PipelineStageFlags::COMPUTE,
                        array_count: 1, optional: false, dynamic_offset_count: 1,
                    }],
                }).unwrap();
                let pipeline_layout = gal.create_pipeline_layout(PipelineLayoutDesc {
                    label: "ranged-storage-pipeline-layout".into(), resource_layouts: vec![layout],
                }).unwrap();
                let shader = gal.create_shader_module(shader("ranged-storage-shader", ShaderStage::Compute)).unwrap();
                let pipeline = gal.create_compute_pipeline(ComputePipelineDesc {
                    label: "ranged-storage-pipeline".into(), layout: pipeline_layout, shader,
                }).unwrap();
                let mut operations = vec![CommandOp::BindComputePipeline(pipeline)];
                for (offset, access) in [(0, AccessFlags::WRITE), (second_offset, second_access)] {
                    let set = gal.create_resource_set(ResourceSetDesc { label: "ranged-storage-set".into(), layout,
                        bindings: vec![ResourceBinding { binding: 0, array_index: 0, resource: buffer,
                            kind: ResourceBindingKind::StorageBuffer, access,
                            dynamic_offsets: vec![if overrides { 0 } else { offset }], buffer_range: Some(256) }],
                    }).unwrap();
                    operations.push(CommandOp::BindResourceSet { pipeline_layout, set_index: 0, set,
                        dynamic_offsets: if overrides { vec![offset] } else { vec![] } });
                    operations.push(CommandOp::Dispatch { groups_x: 1, groups_y: 1, groups_z: 1 });
                }
                let commands = gal.create_command_list(CommandListDesc { label: "ranged-storage-commands".into(), operations }).unwrap();
                let result = gal.submit(SubmissionBatch { label: "ranged-storage-submit".into(), command_lists: vec![commands] });
                assert_eq!(result.is_ok(), accepted, "offset={second_offset} overrides={overrides} access={second_access:?}: {result:?}");
                if let Err(error) = result { assert_eq!(error.domain, ErrorDomain::Submission); }
            }
        }
    }
}

fn empty_resource_batch() -> FfiResourceBatch {
    FfiResourceBatch {
        header: ffi_header_for::<FfiResourceBatch>(),
        buffers: empty_ffi_slice(),
        textures: empty_ffi_slice(),
        texture_views: empty_ffi_slice(),
        samplers: empty_ffi_slice(),
        shaders: empty_ffi_slice(),
        resource_layouts: empty_ffi_slice(),
        resource_layout_bindings: empty_ffi_slice(),
        resource_sets: empty_ffi_slice(),
        resource_set_bindings: empty_ffi_slice(),
        dynamic_offsets: empty_ffi_slice(),
        pipeline_layouts: empty_ffi_slice(),
        pipeline_layout_resource_layouts: empty_ffi_slice(),
        graphics_pipelines: empty_ffi_slice(),
        compute_pipelines: empty_ffi_slice(),
        render_targets: empty_ffi_slice(),
        render_target_color_views: empty_ffi_slice(),
        render_passes: empty_ffi_slice(),
        render_pass_color_formats: empty_ffi_slice(),
        buffer_updates: empty_ffi_slice(),
        texture_updates: empty_ffi_slice(),
        destroys: empty_ffi_slice(),
        negotiated_feature_bits: 0,
    }
}

#[test]
fn ffi_v1_keeps_private_combined_texture_sampler_bindings_unavailable() {
    assert_code(
        resource_binding_kind(ResourceBindingKind::CombinedTextureSampler as u32),
        super::StatusCode::UnknownEnum,
    );
}

#[test]
fn frozen_ffi_abi_sizes_and_capability_negotiation_are_stable() {
    assert_eq!(FFI_ABI_V1_VERSION, 1);
    assert_eq!(FFI_ABI_V4_VERSION, 4);
    assert_eq!(FFI_ABI_V5_VERSION, 5);
    assert_eq!(FFI_ABI_V6_VERSION, 6);
    assert_eq!(FFI_ABI_V7_VERSION, 7);
    assert_eq!(FFI_ABI_V8_VERSION, 8);
    assert_eq!(FFI_ABI_V9_VERSION, 9);
    assert_eq!(FFI_ABI_V10_VERSION, 10);
    assert_eq!(FFI_ABI_V11_VERSION, 11);
    assert_eq!(FFI_ABI_V12_VERSION, 12);
    assert_eq!(FFI_ABI_V13_VERSION, 13);
    assert_eq!(FFI_ABI_V14_VERSION, 14);
    assert_eq!(FFI_ABI_V15_VERSION, 15);
    assert_eq!(FFI_ABI_V16_VERSION, 16);
    assert_eq!(FFI_ABI_V17_VERSION, 17);
    assert_eq!(FFI_ABI_V18_VERSION, 18);
    assert_eq!(FFI_ABI_V19_VERSION, 19);
    assert_eq!(FFI_ABI_V20_VERSION, 20);
    assert_eq!(FFI_ABI_V21_VERSION, 21);
    assert_eq!(FFI_ABI_V22_VERSION, 22);
    assert_eq!(FFI_ABI_V23_VERSION, 23);
    assert_eq!(FFI_ABI_V24_VERSION, 24);
    assert_eq!(FFI_ABI_V25_VERSION, 25);
    assert_eq!(FFI_ABI_V26_VERSION, 26);
    assert_eq!(FFI_ABI_V27_VERSION, 27);
    assert_eq!(FFI_ABI_V28_VERSION, 28);
    assert_eq!(FFI_ABI_V29_VERSION, 29);
    assert_eq!(FFI_ABI_V30_VERSION, 30);
    assert_eq!(FFI_ABI_V31_VERSION, 31);
    assert_eq!(FFI_ABI_V32_VERSION, 32);
    assert_eq!(FFI_ABI_V33_VERSION, 33);
    assert_eq!(FFI_ABI_V34_VERSION, 34);
    assert_eq!(FFI_ABI_V35_VERSION, 35);
    assert_eq!(FFI_ABI_V36_VERSION, 36);
    assert_eq!(FFI_ABI_V37_VERSION, 37);
    assert_eq!(FFI_ABI_V38_VERSION, 38);
    assert_eq!(FFI_ABI_V39_VERSION, 39);
    assert_eq!(FFI_ABI_V40_VERSION, 40);
    assert_eq!(FFI_ABI_V41_VERSION, 41);
    assert_eq!(FFI_ABI_V42_VERSION, 42);
    assert_eq!(FFI_ABI_VERSION, 54);
    assert!(!FFI_INITIAL_PRESENTATION_SUPPORTED);
    assert_eq!(size_of::<FfiHeader>(), 8);
    assert_eq!(size_of::<FfiHandle>(), 8);
    assert_eq!(size_of::<FfiBytes>(), 16);
    assert_eq!(size_of::<FfiSlice<FfiBufferDescAbi>>(), 16);
    assert_eq!(size_of::<FfiCapabilityQueryRequest>(), 24);
    assert_eq!(size_of::<FfiStatusResult>(), 136);

    let request = FfiCapabilityQueryRequest {
        header: ffi_header_for::<FfiCapabilityQueryRequest>(),
        requested_feature_bits: FfiFeatureBits::GRAPHICS | FfiFeatureBits::HOST_BUFFER_ACCESS,
        reserved0: 0,
    };
    let result = unsafe { answer_capability_query(&request, opengl_capabilities()) }
        .expect("OpenGL should negotiate its supported shared graphics subset");
    assert_eq!(result.header.version, FFI_ABI_VERSION);
    assert_eq!(result.status, super::StatusCode::Ok as i32);
    assert_ne!(result.supported_feature_bits & FfiFeatureBits::GRAPHICS, 0);
    assert_eq!(result.initial_presentation_supported, 0);

    let unsupported = FfiCapabilityQueryRequest {
        requested_feature_bits: FfiFeatureBits::GRAPHICS | FfiFeatureBits::INDIRECT_DRAW,
        ..request
    };
    assert_code(
        unsafe { answer_capability_query(&unsupported, opengl_capabilities()) },
        super::StatusCode::UnsupportedFeature,
    );

    let unknown = FfiCapabilityQueryRequest {
        requested_feature_bits: 1_u64 << 63,
        ..request
    };
    assert_code(
        unsafe { answer_capability_query(&unknown, vulkan_capabilities()) },
        super::StatusCode::UnknownEnum,
    );
}

#[test]
fn ffi_resource_batch_decodes_fixed_layout_descriptions_and_copies_inputs() {
    let mut buffer_label = b"terrain-upload".to_vec();
    let mut shader_label = b"terrain-vertex".to_vec();
    let mut entry = b"main".to_vec();
    let mut code = b"#version 450\nvoid main(){}".to_vec();
    let mut update_bytes = vec![1_u8, 2, 3, 4];
    let buffer = FfiBufferDescAbi {
        byte_size: size_of::<FfiBufferDescAbi>() as u32,
        request_id: 10,
        label: ffi_bytes(&buffer_label),
        size: 4096,
        memory_domain: MemoryDomain::Upload as u32,
        usage_bits: (1 << 0) | (1 << 5) | (1 << 8),
    };
    let shader = FfiShaderModuleDescAbi {
        byte_size: size_of::<FfiShaderModuleDescAbi>() as u32,
        request_id: 11,
        label: ffi_bytes(&shader_label),
        stage: ShaderStage::Vertex as u32,
        code_format: ShaderCodeFormat::Glsl as u32,
        code: ffi_bytes(&code),
        entry_point: ffi_bytes(&entry),
    };
    let update = FfiBufferUpdateAbi {
        byte_size: size_of::<FfiBufferUpdateAbi>() as u32,
        buffer: ffi_handle(HandleKind::Buffer, 1),
        offset: 8,
        data: ffi_bytes(&update_bytes),
    };
    let destroy = FfiDestroyDescAbi {
        byte_size: size_of::<FfiDestroyDescAbi>() as u32,
        handle: ffi_handle(HandleKind::Buffer, 2),
        expected_kind: HandleKind::Buffer as u32,
    };
    let mut batch = empty_resource_batch();
    batch.buffers = ffi_slice(std::slice::from_ref(&buffer));
    batch.shaders = ffi_slice(std::slice::from_ref(&shader));
    batch.buffer_updates = ffi_slice(std::slice::from_ref(&update));
    batch.destroys = ffi_slice(std::slice::from_ref(&destroy));
    batch.negotiated_feature_bits = FfiFeatureBits::GRAPHICS | FfiFeatureBits::HOST_BUFFER_ACCESS;

    let owned = unsafe { decode_resource_batch(&batch, opengl_capabilities()) }
        .expect("resource batch should decode into owned GAL descriptions");
    buffer_label.fill(b'X');
    shader_label.fill(b'Y');
    entry.fill(b'Z');
    code.fill(0);
    update_bytes.fill(9);

    assert_eq!(owned.buffers[0].request_id, 10);
    assert_eq!(owned.buffers[0].desc.label, "terrain-upload");
    assert_eq!(owned.shaders[0].desc.label, "terrain-vertex");
    assert_eq!(owned.shaders[0].desc.entry_point, "main");
    assert!(owned.shaders[0].desc.code.starts_with(b"#version 450"));
    assert_eq!(owned.buffer_updates[0].data, vec![1, 2, 3, 4]);
    assert_eq!(owned.destroys[0].1, HandleKind::Buffer);

    let serialized = serialize_resource_batch_canonical(&owned);
    let reparsed = serialize_resource_batch_canonical(&owned);
    assert_eq!(serialized, reparsed);
}

#[test]
fn ffi_resource_batch_covers_layout_sets_pipelines_targets_and_passes() {
    let layout_binding = FfiResourceBindingDescAbi {
        byte_size: size_of::<FfiResourceBindingDescAbi>() as u32,
        binding: 0,
        kind: ResourceBindingKind::UniformBuffer as u32,
        stage_bits: PipelineStageFlags::DRAW.0,
        array_count: 2,
        optional: 1,
        dynamic_offset_count: 1,
    };
    let layout_label = b"layout";
    let layout = FfiResourceLayoutDescAbi {
        byte_size: size_of::<FfiResourceLayoutDescAbi>() as u32,
        request_id: 20,
        label: ffi_bytes(layout_label),
        bindings: FfiRange {
            offset: 0,
            count: 1,
        },
    };
    let offsets = [64_u64];
    let set_binding = FfiResourceBindingAbi {
        byte_size: size_of::<FfiResourceBindingAbi>() as u32,
        binding: 0,
        array_index: 1,
        resource: ffi_handle(HandleKind::Buffer, 1),
        kind: ResourceBindingKind::UniformBuffer as u32,
        access_bits: AccessFlags::READ.0,
        dynamic_offsets: FfiRange {
            offset: 0,
            count: 1,
        },
    };
    let set = FfiResourceSetDescAbi {
        byte_size: size_of::<FfiResourceSetDescAbi>() as u32,
        request_id: 21,
        label: ffi_bytes(b"set"),
        layout: ffi_handle(HandleKind::ResourceLayout, 1),
        bindings: FfiRange {
            offset: 0,
            count: 1,
        },
    };
    let pipeline_layouts = [ffi_handle(HandleKind::ResourceLayout, 1)];
    let pipeline_layout = FfiPipelineLayoutDescAbi {
        byte_size: size_of::<FfiPipelineLayoutDescAbi>() as u32,
        request_id: 22,
        label: ffi_bytes(b"pipeline-layout"),
        resource_layouts: FfiRange {
            offset: 0,
            count: 1,
        },
    };
    let color_formats = [TextureFormat::Rgba8Unorm as u32];
    let pipeline = FfiGraphicsPipelineDescAbi {
        byte_size: size_of::<FfiGraphicsPipelineDescAbi>() as u32,
        request_id: 23,
        label: ffi_bytes(b"pipeline"),
        layout: ffi_handle(HandleKind::PipelineLayout, 1),
        vertex_shader: ffi_handle(HandleKind::ShaderModule, 1),
        fragment_shader: ffi_handle(HandleKind::ShaderModule, 2),
        topology: PrimitiveTopology::Triangles as u32,
        cull_mode: CullMode::Back as u32,
        blend: BlendMode::Alpha as u32,
        depth_compare: CompareOp::LessOrEqual as u32,
        color_formats: FfiRange {
            offset: 0,
            count: 1,
        },
        depth_format: TextureFormat::Depth32Float as u32,
    };
    let color_views = [ffi_handle(HandleKind::TextureView, 1)];
    let target = FfiRenderTargetDescAbi {
        byte_size: size_of::<FfiRenderTargetDescAbi>() as u32,
        request_id: 24,
        label: ffi_bytes(b"target"),
        color_views: FfiRange {
            offset: 0,
            count: 1,
        },
        depth_stencil_view: ffi_handle(HandleKind::TextureView, 2),
        extent: FfiExtent3d {
            width: 64,
            height: 64,
            depth: 1,
        },
    };
    let pass = FfiRenderPassDescAbi {
        byte_size: size_of::<FfiRenderPassDescAbi>() as u32,
        request_id: 25,
        label: ffi_bytes(b"pass"),
        target: ffi_handle(HandleKind::RenderTarget, 1),
        color_formats: FfiRange {
            offset: 0,
            count: 1,
        },
        depth_format: TextureFormat::Depth32Float as u32,
    };
    let mut batch = empty_resource_batch();
    batch.resource_layouts = ffi_slice(std::slice::from_ref(&layout));
    batch.resource_layout_bindings = ffi_slice(std::slice::from_ref(&layout_binding));
    batch.resource_sets = ffi_slice(std::slice::from_ref(&set));
    batch.resource_set_bindings = ffi_slice(std::slice::from_ref(&set_binding));
    batch.dynamic_offsets = ffi_slice(&offsets);
    batch.pipeline_layouts = ffi_slice(std::slice::from_ref(&pipeline_layout));
    batch.pipeline_layout_resource_layouts = ffi_slice(&pipeline_layouts);
    batch.graphics_pipelines = ffi_slice(std::slice::from_ref(&pipeline));
    batch.render_targets = ffi_slice(std::slice::from_ref(&target));
    batch.render_target_color_views = ffi_slice(&color_views);
    batch.render_passes = ffi_slice(std::slice::from_ref(&pass));
    batch.render_pass_color_formats = ffi_slice(&color_formats);
    batch.negotiated_feature_bits = FfiFeatureBits::GRAPHICS
        | FfiFeatureBits::DESCRIPTOR_ARRAYS
        | FfiFeatureBits::OPTIONAL_BINDINGS
        | FfiFeatureBits::DYNAMIC_BUFFER_OFFSETS
        | FfiFeatureBits::UNIFORM_BUFFERS
        | FfiFeatureBits::MULTIPLE_COLOR_ATTACHMENTS
        | FfiFeatureBits::BLENDED_PASS;

    let owned = unsafe { decode_resource_batch(&batch, opengl_capabilities()) }
        .expect("layout/set/pipeline/pass ABI should decode");
    assert_eq!(owned.resource_layouts[0].desc.bindings[0].array_count, 2);
    assert_eq!(
        owned.resource_sets[0].desc.bindings[0].dynamic_offsets,
        vec![64]
    );
    assert_eq!(owned.pipeline_layouts[0].desc.resource_layouts.len(), 1);
    assert_eq!(owned.graphics_pipelines[0].desc.blend, BlendMode::Alpha);
    assert_eq!(owned.render_targets[0].desc.color_views.len(), 1);
    assert_eq!(
        owned.render_passes[0].desc.depth_format,
        Some(TextureFormat::Depth32Float)
    );
}

#[test]
fn ffi_resource_batch_counts_binding_tables_in_aggregate_limit() {
    let dynamic_offsets = vec![0u64; FFI_MAX_BATCH_ITEMS / 2];
    let pipeline_layout_resources = vec![FfiHandle { raw: 0 }; FFI_MAX_BATCH_ITEMS / 2 + 1];
    let mut batch = empty_resource_batch();
    batch.dynamic_offsets = ffi_slice(&dynamic_offsets);
    batch.pipeline_layout_resource_layouts = ffi_slice(&pipeline_layout_resources);
    let error = unsafe { decode_resource_batch(&batch, opengl_capabilities()) }
        .expect_err("resource binding tables must share the aggregate batch bound");
    assert_eq!(super::StatusCode::LengthOverflow, error.code);
}

#[test]
fn ffi_submission_batch_decodes_complete_commands_and_copies_inline_uploads() {
    let color_attachment = FfiPassAttachmentAbi {
        byte_size: size_of::<FfiPassAttachmentAbi>() as u32,
        view: ffi_handle(HandleKind::TextureView, 1),
        load_op: AttachmentLoadOp::Clear as u32,
        store_op: AttachmentStoreOp::Store as u32,
        has_clear_color: 1,
        clear_color: FfiClearColor {
            r: 0.25,
            g: 0.5,
            b: 0.75,
            a: 1.0,
        },
    };
    let mut upload = vec![1_u8, 3, 5, 7];
    let ops = [
        FfiCommandOpAbi {
            byte_size: size_of::<FfiCommandOpAbi>() as u32,
            op_kind: FfiCommandOpKind::BeginPass as u32,
            primary: ffi_handle(HandleKind::RenderPass, 1),
            secondary: ffi_handle(HandleKind::RenderTarget, 1),
            tertiary: FfiHandle { raw: 0 },
            set_index: 0,
            slot: 0,
            offset: 0,
            size: 0,
            count0: 0,
            count1: 0,
            count2: 0,
            colors: FfiRange {
                offset: 0,
                count: 1,
            },
            depth_stencil: FfiRange {
                offset: 0,
                count: 0,
            },
            copy_region: FfiRange {
                offset: 0,
                count: 0,
            },
            barrier: FfiRange {
                offset: 0,
                count: 0,
            },
            inline_bytes: FfiBytes {
                ptr: std::ptr::null(),
                len: 0,
            },
            subresources: FfiTextureSubresourceRange::default(),
        },
        FfiCommandOpAbi {
            byte_size: size_of::<FfiCommandOpAbi>() as u32,
            op_kind: FfiCommandOpKind::BindGraphicsPipeline as u32,
            primary: ffi_handle(HandleKind::GraphicsPipeline, 1),
            secondary: FfiHandle { raw: 0 },
            tertiary: FfiHandle { raw: 0 },
            set_index: 0,
            slot: 0,
            offset: 0,
            size: 0,
            count0: 0,
            count1: 0,
            count2: 0,
            colors: FfiRange {
                offset: 0,
                count: 0,
            },
            depth_stencil: FfiRange {
                offset: 0,
                count: 0,
            },
            copy_region: FfiRange {
                offset: 0,
                count: 0,
            },
            barrier: FfiRange {
                offset: 0,
                count: 0,
            },
            inline_bytes: FfiBytes {
                ptr: std::ptr::null(),
                len: 0,
            },
            subresources: FfiTextureSubresourceRange::default(),
        },
        FfiCommandOpAbi {
            byte_size: size_of::<FfiCommandOpAbi>() as u32,
            op_kind: FfiCommandOpKind::HostWriteBuffer as u32,
            primary: ffi_handle(HandleKind::Buffer, 1),
            secondary: FfiHandle { raw: 0 },
            tertiary: FfiHandle { raw: 0 },
            set_index: 0,
            slot: 0,
            offset: 16,
            size: 0,
            count0: 0,
            count1: 0,
            count2: 0,
            colors: FfiRange {
                offset: 0,
                count: 0,
            },
            depth_stencil: FfiRange {
                offset: 0,
                count: 0,
            },
            copy_region: FfiRange {
                offset: 0,
                count: 0,
            },
            barrier: FfiRange {
                offset: 0,
                count: 0,
            },
            inline_bytes: ffi_bytes(&upload),
            subresources: FfiTextureSubresourceRange::default(),
        },
        FfiCommandOpAbi {
            byte_size: size_of::<FfiCommandOpAbi>() as u32,
            op_kind: FfiCommandOpKind::DrawIndexed as u32,
            primary: FfiHandle { raw: 0 },
            secondary: FfiHandle { raw: 0 },
            tertiary: FfiHandle { raw: 0 },
            set_index: 0,
            slot: 0,
            offset: 0,
            size: 0,
            count0: 6,
            count1: 1,
            count2: 0,
            colors: FfiRange {
                offset: 0,
                count: 0,
            },
            depth_stencil: FfiRange {
                offset: 0,
                count: 0,
            },
            copy_region: FfiRange {
                offset: 0,
                count: 0,
            },
            barrier: FfiRange {
                offset: 0,
                count: 0,
            },
            inline_bytes: FfiBytes {
                ptr: std::ptr::null(),
                len: 0,
            },
            subresources: FfiTextureSubresourceRange::default(),
        },
        FfiCommandOpAbi {
            byte_size: size_of::<FfiCommandOpAbi>() as u32,
            op_kind: FfiCommandOpKind::EndPass as u32,
            primary: FfiHandle { raw: 0 },
            secondary: FfiHandle { raw: 0 },
            tertiary: FfiHandle { raw: 0 },
            set_index: 0,
            slot: 0,
            offset: 0,
            size: 0,
            count0: 0,
            count1: 0,
            count2: 0,
            colors: FfiRange {
                offset: 0,
                count: 0,
            },
            depth_stencil: FfiRange {
                offset: 0,
                count: 0,
            },
            copy_region: FfiRange {
                offset: 0,
                count: 0,
            },
            barrier: FfiRange {
                offset: 0,
                count: 0,
            },
            inline_bytes: FfiBytes {
                ptr: std::ptr::null(),
                len: 0,
            },
            subresources: FfiTextureSubresourceRange::default(),
        },
    ];
    let list_label = b"list";
    let list = FfiCommandListAbi {
        byte_size: size_of::<FfiCommandListAbi>() as u32,
        label: ffi_bytes(list_label),
        operations: FfiRange {
            offset: 0,
            count: ops.len() as u64,
        },
    };
    let batch = FfiSubmissionBatchAbi {
        header: ffi_header_for::<FfiSubmissionBatchAbi>(),
        label: ffi_bytes(b"submission"),
        command_lists: ffi_slice(std::slice::from_ref(&list)),
        operations: ffi_slice(&ops),
        pass_attachments: ffi_slice(std::slice::from_ref(&color_attachment)),
        copy_regions: empty_ffi_slice(),
        barriers: empty_ffi_slice(),
        negotiated_feature_bits: FfiFeatureBits::GRAPHICS | FfiFeatureBits::HOST_BUFFER_ACCESS,
    };

    let decoded = unsafe { decode_submission_batch(&batch, opengl_capabilities()) }
        .expect("submission batch should decode");
    upload.fill(9);
    assert_eq!(decoded.label, "submission");
    assert_eq!(decoded.command_lists[0].operations.len(), ops.len());
    match &decoded.command_lists[0].operations[2] {
        CommandOp::HostWriteBuffer { data, offset, .. } => {
            assert_eq!(*offset, 16);
            assert_eq!(data, &vec![1, 3, 5, 7]);
        }
        other => panic!("unexpected decoded op {other:?}"),
    }
    assert_eq!(
        serialize_submission_batch_canonical(&decoded),
        serialize_submission_batch_canonical(&decoded)
    );
}

#[test]
fn ffi_rejects_malformed_ranges_unknown_enums_and_unsupported_features() {
    let bad_buffer = FfiBufferDescAbi {
        byte_size: size_of::<FfiBufferDescAbi>() as u32,
        request_id: 1,
        label: ffi_bytes(b"bad"),
        size: 4,
        memory_domain: 77,
        usage_bits: 1,
    };
    let mut batch = empty_resource_batch();
    batch.buffers = ffi_slice(std::slice::from_ref(&bad_buffer));
    assert_code(
        unsafe { decode_resource_batch(&batch, vulkan_capabilities()) },
        super::StatusCode::UnknownEnum,
    );

    let storage_texture = FfiTextureDescAbi {
        byte_size: size_of::<FfiTextureDescAbi>() as u32,
        request_id: 2,
        label: ffi_bytes(b"storage-image"),
        dimension: TextureDimension::D2 as u32,
        format: TextureFormat::Rgba8Unorm as u32,
        extent: FfiExtent3d {
            width: 4,
            height: 4,
            depth: 1,
        },
        mip_levels: 1,
        array_layers: 1,
        usage_bits: 1 << 1,
    };
    let mut batch = empty_resource_batch();
    batch.textures = ffi_slice(std::slice::from_ref(&storage_texture));
    assert_code(
        unsafe { decode_resource_batch(&batch, opengl_capabilities()) },
        super::StatusCode::UnsupportedFeature,
    );

    let private_d3_texture = FfiTextureDescAbi {
        byte_size: size_of::<FfiTextureDescAbi>() as u32,
        request_id: 3,
        label: ffi_bytes(b"private-d3-volume"),
        dimension: TextureDimension::D3 as u32,
        format: TextureFormat::Rgba16Float as u32,
        extent: FfiExtent3d {
            width: 4,
            height: 4,
            depth: 4,
        },
        mip_levels: 1,
        array_layers: 1,
        usage_bits: 1 << 0,
    };
    let mut batch = empty_resource_batch();
    batch.textures = ffi_slice(std::slice::from_ref(&private_d3_texture));
    let error = unsafe { decode_resource_batch(&batch, vulkan_capabilities()) }
        .expect_err("stable Java FFI must not admit private D3 resources");
    assert_eq!(error.code, super::StatusCode::UnsupportedFeature);
    assert!(error.message.contains("internal-only"));

    let op = FfiCommandOpAbi {
        byte_size: size_of::<FfiCommandOpAbi>() as u32,
        op_kind: FfiCommandOpKind::DrawIndirect as u32,
        primary: ffi_handle(HandleKind::Buffer, 1),
        secondary: FfiHandle { raw: 0 },
        tertiary: FfiHandle { raw: 0 },
        set_index: 0,
        slot: 0,
        offset: 0,
        size: 0,
        count0: 1,
        count1: 0,
        count2: 0,
        colors: FfiRange {
            offset: 0,
            count: 0,
        },
        depth_stencil: FfiRange {
            offset: 0,
            count: 0,
        },
        copy_region: FfiRange {
            offset: 0,
            count: 0,
        },
        barrier: FfiRange {
            offset: 0,
            count: 0,
        },
        inline_bytes: FfiBytes {
            ptr: std::ptr::null(),
            len: 0,
        },
        subresources: FfiTextureSubresourceRange::default(),
    };
    let list = FfiCommandListAbi {
        byte_size: size_of::<FfiCommandListAbi>() as u32,
        label: ffi_bytes(b"list"),
        operations: FfiRange {
            offset: 0,
            count: 1,
        },
    };
    let submission = FfiSubmissionBatchAbi {
        header: ffi_header_for::<FfiSubmissionBatchAbi>(),
        label: ffi_bytes(b"submission"),
        command_lists: ffi_slice(std::slice::from_ref(&list)),
        operations: ffi_slice(std::slice::from_ref(&op)),
        pass_attachments: empty_ffi_slice(),
        copy_regions: empty_ffi_slice(),
        barriers: empty_ffi_slice(),
        negotiated_feature_bits: FfiFeatureBits::GRAPHICS,
    };
    assert_code(
        unsafe { decode_submission_batch(&submission, opengl_capabilities()) },
        super::StatusCode::UnsupportedFeature,
    );

    let malformed_list = FfiCommandListAbi {
        operations: FfiRange {
            offset: 10,
            count: 1,
        },
        ..list
    };
    let malformed = FfiSubmissionBatchAbi {
        command_lists: ffi_slice(std::slice::from_ref(&malformed_list)),
        ..submission
    };
    assert_code(
        unsafe { decode_submission_batch(&malformed, vulkan_capabilities()) },
        super::StatusCode::InvalidArgument,
    );
}

#[test]
fn ffi_large_batches_and_partial_failures_are_deterministic() {
    let mut ops = Vec::new();
    for _ in 0..128 {
        ops.push(FfiCommandOpAbi {
            byte_size: size_of::<FfiCommandOpAbi>() as u32,
            op_kind: FfiCommandOpKind::Draw as u32,
            primary: FfiHandle { raw: 0 },
            secondary: FfiHandle { raw: 0 },
            tertiary: FfiHandle { raw: 0 },
            set_index: 0,
            slot: 0,
            offset: 0,
            size: 0,
            count0: 3,
            count1: 1,
            count2: 0,
            colors: FfiRange {
                offset: 0,
                count: 0,
            },
            depth_stencil: FfiRange {
                offset: 0,
                count: 0,
            },
            copy_region: FfiRange {
                offset: 0,
                count: 0,
            },
            barrier: FfiRange {
                offset: 0,
                count: 0,
            },
            inline_bytes: FfiBytes {
                ptr: std::ptr::null(),
                len: 0,
            },
            subresources: FfiTextureSubresourceRange::default(),
        });
    }
    let list = FfiCommandListAbi {
        byte_size: size_of::<FfiCommandListAbi>() as u32,
        label: ffi_bytes(b"large-list"),
        operations: FfiRange {
            offset: 0,
            count: ops.len() as u64,
        },
    };
    let submission = FfiSubmissionBatchAbi {
        header: ffi_header_for::<FfiSubmissionBatchAbi>(),
        label: ffi_bytes(b"large"),
        command_lists: ffi_slice(std::slice::from_ref(&list)),
        operations: ffi_slice(&ops),
        pass_attachments: empty_ffi_slice(),
        copy_regions: empty_ffi_slice(),
        barriers: empty_ffi_slice(),
        negotiated_feature_bits: FfiFeatureBits::GRAPHICS,
    };
    let decoded = unsafe { decode_submission_batch(&submission, opengl_capabilities()) }
        .expect("large but bounded command batch should decode");
    assert_eq!(decoded.command_lists[0].operations.len(), 128);

    let good = FfiBufferDescAbi {
        byte_size: size_of::<FfiBufferDescAbi>() as u32,
        request_id: 1,
        label: ffi_bytes(b"good"),
        size: 64,
        memory_domain: MemoryDomain::Upload as u32,
        usage_bits: 1,
    };
    let bad = FfiShaderModuleDescAbi {
        byte_size: size_of::<FfiShaderModuleDescAbi>() as u32,
        request_id: 2,
        label: ffi_bytes(b"bad"),
        stage: 999,
        code_format: ShaderCodeFormat::Glsl as u32,
        code: ffi_bytes(b"void main(){}"),
        entry_point: ffi_bytes(b"main"),
    };
    let mut batch = empty_resource_batch();
    batch.buffers = ffi_slice(std::slice::from_ref(&good));
    batch.shaders = ffi_slice(std::slice::from_ref(&bad));
    let first = unsafe { decode_resource_batch(&batch, vulkan_capabilities()) };
    let second = unsafe { decode_resource_batch(&batch, vulkan_capabilities()) };
    assert_code(first, super::StatusCode::UnknownEnum);
    assert_code(second, super::StatusCode::UnknownEnum);
}

#[test]
fn ffi_abi_fuzz_rejects_unknown_versions_enums_and_lengths() {
    let base = FfiCapabilityQueryRequest {
        header: ffi_header_for::<FfiCapabilityQueryRequest>(),
        requested_feature_bits: FfiFeatureBits::GRAPHICS,
        reserved0: 0,
    };
    for version in [0, FFI_ABI_V27_VERSION, FFI_ABI_V28_VERSION, FFI_ABI_VERSION + 1, u32::MAX] {
        let mut request = base;
        request.header.version = version;
        assert_code(
            unsafe { answer_capability_query(&request, vulkan_capabilities()) },
            super::StatusCode::InvalidArgument,
        );
    }
    let mut v1_request = base;
    v1_request.header.version = FFI_ABI_V1_VERSION;
    assert!(unsafe { answer_capability_query(&v1_request, vulkan_capabilities()) }.is_ok());

    let tiny_label = [b'a'; FFI_MAX_LABEL_BYTES + 1];
    let buffer = FfiBufferDescAbi {
        byte_size: size_of::<FfiBufferDescAbi>() as u32,
        request_id: 1,
        label: ffi_bytes(&tiny_label),
        size: 64,
        memory_domain: MemoryDomain::Upload as u32,
        usage_bits: 1,
    };
    let mut batch = empty_resource_batch();
    batch.buffers = ffi_slice(std::slice::from_ref(&buffer));
    assert_code(
        unsafe { decode_resource_batch(&batch, vulkan_capabilities()) },
        super::StatusCode::LengthOverflow,
    );

    let huge_ops = FfiSlice::<FfiCommandOpAbi> {
        ptr: NonNull::<FfiCommandOpAbi>::dangling().as_ptr(),
        count: u64::MAX,
    };
    let list = FfiCommandListAbi {
        byte_size: size_of::<FfiCommandListAbi>() as u32,
        label: ffi_bytes(b"list"),
        operations: FfiRange {
            offset: 0,
            count: 1,
        },
    };
    let submission = FfiSubmissionBatchAbi {
        header: ffi_header_for::<FfiSubmissionBatchAbi>(),
        label: ffi_bytes(b"submission"),
        command_lists: ffi_slice(std::slice::from_ref(&list)),
        operations: huge_ops,
        pass_attachments: empty_ffi_slice(),
        copy_regions: empty_ffi_slice(),
        barriers: empty_ffi_slice(),
        negotiated_feature_bits: 0,
    };
    assert_code(
        unsafe { decode_submission_batch(&submission, vulkan_capabilities()) },
        super::StatusCode::LengthOverflow,
    );
}

#[test]
fn ffi_status_completion_and_retirement_results_are_structured() {
    let error = GalError::unsupported_feature("backend does not support indirect draw");
    let status = status_result_from_error(&error);
    assert_eq!(status.header.version, FFI_ABI_VERSION);
    assert_eq!(status.status, super::StatusCode::UnsupportedFeature as i32);
    assert_eq!(status.error_domain, ErrorDomain::Resource as u32);
    assert_eq!(
        status.unsupported_feature,
        BackendFeature::IndirectDraw as u32
    );

    let bad_completion = FfiCompletionQueryRequest {
        header: ffi_header_for::<FfiCompletionQueryRequest>(),
        submission_id: 0,
    };
    assert_code(
        unsafe { validate_completion_query(&bad_completion) },
        super::StatusCode::InvalidArgument,
    );
    let good_completion = FfiCompletionQueryRequest {
        submission_id: 7,
        ..bad_completion
    };
    let decoded = unsafe { validate_completion_query(&good_completion) }
        .expect("non-zero completion query should decode");
    assert_eq!(decoded.submission_id, 7);
    let incomplete = completion_result_for(SubmissionId(7), SubmissionId(6));
    let complete = completion_result_for(SubmissionId(7), SubmissionId(7));
    assert_eq!(incomplete.is_complete, 0);
    assert_eq!(complete.is_complete, 1);

    let handles = [
        ffi_handle(HandleKind::Buffer, 1),
        ffi_handle(HandleKind::Texture, 2),
    ];
    let retirement = FfiRetirementBatch {
        header: ffi_header_for::<FfiRetirementBatch>(),
        completed_submission_id: 9,
        handles: ffi_slice(&handles),
    };
    let (completed, owned) =
        unsafe { decode_retirement_batch(&retirement) }.expect("retirement batch should decode");
    assert_eq!(completed, SubmissionId(9));
    assert_eq!(owned.len(), 2);
    assert_eq!(owned[0].kind(), Some(HandleKind::Buffer));
}

#[test]
fn backend_specific_api_tokens_do_not_leak_into_gal_core() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("render/vulkanic");
    for relative in [
        "commands.rs",
        "error.rs",
        "ffi/abi.rs",
        "ffi/context.rs",
        "ffi/frame.rs",
        "ffi/gui.rs",
        "ffi/layout.rs",
        "ffi/material.rs",
        "ffi/memory.rs",
        "ffi/mod.rs",
        "ffi/resources.rs",
        "ffi/status.rs",
        "ffi/submission.rs",
        "ffi/world.rs",
        "frame.rs",
        "gal.rs",
        "handles.rs",
        "metrics.rs",
        "mod.rs",
        "resources.rs",
        "sync.rs",
    ] {
        let source = fs::read_to_string(root.join(relative)).unwrap();
        for forbidden in [
            concat!("ash", "::"),
            concat!("glow", "::"),
            concat!("vk", "::"),
            concat!("GL", "_"),
            concat!("VK", "_"),
        ] {
            assert!(
                !source.contains(forbidden),
                "{relative} leaks backend-specific token {forbidden}"
            );
        }
    }
}
