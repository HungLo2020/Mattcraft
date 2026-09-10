//! Supplemental GPU observation using exact r526 game VS output bits.
//! This diagnostic does not admit visual parity or substitute for Frozen.
use crate::render::vulkanic::commands::*;
use crate::render::vulkanic::gal::VulkanicGal;
use crate::render::vulkanic::resources::*;
use crate::render::vulkanic::world_primitive_frontend::oriented_target::{
    OrientedWorldTarget, WorldAttachmentStates, WorldTargetDesc,
};

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires a native desktop; explicitly run for frame-target composition changes"]
fn acquired_frame_load_preserves_owned_world_copy_and_explicit_clear() {
    use crate::render::vulkanic::frame::*;
    let event_loop = super::vulkan::winit_event_loop().unwrap();
    let window = super::vulkan::winit_test_window(&event_loop, 128, 128, 931).unwrap();
    let extent = Extent3d { width: 128, height: 128, depth: 1 };
    let surface = FrameSurfaceDesc { label: "explicit-frame-load".into(), extent,
        color_format: TextureFormat::Bgra8Unorm, present_mode: PresentMode::Fifo, max_frames_in_flight: 2 };
    let backend = super::vulkan::VulkanBackend::new_windowed_for_test("explicit frame load", &window, surface.clone()).unwrap();
    let mut gal = VulkanicGal::new_with_backend(Box::new(backend), false);
    gal.configure_frame_surface(surface).unwrap();
    let acquired = gal.acquire_frame(FrameAcquireDesc { correlation_id: FrameCorrelationId(1), expected_extent: extent }).unwrap();
    let frame = gal.create_frame_target(FrameTargetDesc { label: "explicit-frame".into(), frame_id: acquired.frame.0,
        render_target: acquired.render_target, extent: acquired.extent, color_format: acquired.color_format }).unwrap();
    let owner = OrientedWorldTarget::create(&mut gal, "explicit-world", WorldTargetDesc {
        extent: acquired.extent, color_format: acquired.color_format, raster_y_direction: RasterYDirection::Up,
    }).unwrap();
    let pass = gal.create_render_pass(RenderPassDesc { label: "explicit-frame.pass".into(), target: frame,
        color_formats: vec![acquired.color_format], depth_format: None }).unwrap();
    let readback = gal.create_buffer(BufferDesc { label: "explicit-frame.readback".into(), size: 4,
        memory: MemoryDomain::Readback, usages: vec![BufferUsage::TransferDst, BufferUsage::HostRead] }).unwrap();
    let snapshot = gal.create_texture(TextureDesc { label: "explicit-frame.snapshot".into(),
        dimension: TextureDimension::D2, format: acquired.color_format, extent: acquired.extent,
        mip_levels: 1, array_layers: 1, usages: vec![TextureUsage::TransferDst, TextureUsage::TransferSrc] }).unwrap();
    let barrier = |resource, before, after, texture| CommandOp::Barrier(ResourceBarrier { resource, before, after,
        subresources: if texture { Some(TextureSubresourceRange { base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 }) } else { None },
        src_queue: QueueClass::Graphics, dst_queue: QueueClass::Graphics });
    let red = ClearColor { r: 1., g: 0., b: 0., a: 1. };
    let green = ClearColor { r: 0., g: 1., b: 0., a: 1. };
    let mut observed = Vec::new();
    for (epoch, load) in [AttachmentLoadOp::Load, AttachmentLoadOp::Clear, AttachmentLoadOp::Load].into_iter().enumerate() {
        let mut ops = Vec::new();
        if epoch == 0 {
            ops.extend([
                barrier(owner.color_texture, TextureUsageState::Undefined, TextureUsageState::ColorAttachment, true),
                barrier(owner.depth_texture, TextureUsageState::Undefined, TextureUsageState::DepthStencilAttachment, true),
                CommandOp::BeginPass { pass: owner.pass, target: owner.target,
                    colors: vec![PassAttachment { view: owner.color_view, load_op: AttachmentLoadOp::Clear,
                        store_op: AttachmentStoreOp::Store, clear_color: Some(red) }],
                    depth_stencil: Some(PassAttachment { view: owner.depth_view, load_op: AttachmentLoadOp::Clear,
                        store_op: AttachmentStoreOp::Store, clear_color: None }) },
                CommandOp::EndPass,
            ]);
            ops.extend(owner.copy_to_frame(&gal, frame).unwrap());
        }
        ops.extend([
            CommandOp::BeginPass { pass, target: frame, colors: vec![PassAttachment {
                view: frame, load_op: load, store_op: AttachmentStoreOp::Store,
                clear_color: Some(if epoch == 2 { red } else { green }) }], depth_stencil: None },
            CommandOp::EndPass,
            barrier(snapshot, TextureUsageState::Undefined, TextureUsageState::TransferDst, true),
            CommandOp::CopyFrameTargetToTexture { src: frame, dst: snapshot, extent: acquired.extent },
            barrier(snapshot, TextureUsageState::TransferDst, TextureUsageState::TransferSrc, true),
            CommandOp::CopyTextureToBuffer(BufferImageCopyRegion { buffer: readback, buffer_offset: 0, bytes_per_row: 4,
                rows_per_image: 1, texture: snapshot, texture_mip: 0, texture_layer: 0,
                texture_origin: TextureOrigin3d { x: 64, y: 64, z: 0 }, extent: Extent3d { width: 1, height: 1, depth: 1 } }),
            barrier(readback, TextureUsageState::TransferDst, TextureUsageState::ShaderRead, false),
            CommandOp::HostReadBuffer { buffer: readback, offset: 0, size: 4 },
        ]);
        let list = gal.create_command_list(CommandListDesc { label: format!("explicit-load.{epoch}"), operations: ops }).unwrap();
        let token = gal.submit(SubmissionBatch { label: format!("explicit-load.{epoch}"), command_lists: vec![list] }).unwrap();
        gal.retire_through(token.submission).unwrap();
        observed.push(gal.completed_host_reads().iter().find(|r| r.buffer == readback && r.submission == token.submission).unwrap().bytes.clone());
    }
    gal.present_frame(PresentFrameDesc { frame: acquired.frame, correlation_id: acquired.correlation_id,
        wait_for: gal.latest_submission_id() }).unwrap();
    // A second acquired frame has no frame-target pass at all: hidden HUD must
    // still present the explicitly copied image with the proper final layout.
    let next = gal.acquire_frame(FrameAcquireDesc { correlation_id: FrameCorrelationId(2), expected_extent: extent }).unwrap();
    let copy_only_frame = gal.create_frame_target(FrameTargetDesc { label: "copy-only-frame".into(), frame_id: next.frame.0,
        render_target: next.render_target, extent: next.extent, color_format: next.color_format }).unwrap();
    let mut ops = vec![
        barrier(owner.color_texture, TextureUsageState::TransferSrc, TextureUsageState::ColorAttachment, true),
        barrier(owner.depth_texture, TextureUsageState::TransferSrc, TextureUsageState::DepthStencilAttachment, true),
        CommandOp::BeginPass { pass: owner.pass, target: owner.target,
            colors: vec![PassAttachment { view: owner.color_view, load_op: AttachmentLoadOp::Clear,
                store_op: AttachmentStoreOp::Store, clear_color: Some(red) }],
            depth_stencil: Some(PassAttachment { view: owner.depth_view, load_op: AttachmentLoadOp::Clear,
                store_op: AttachmentStoreOp::Store, clear_color: None }) },
        CommandOp::EndPass,
    ];
    ops.extend(owner.copy_to_frame(&gal, copy_only_frame).unwrap());
    ops.extend([
        barrier(snapshot, TextureUsageState::Undefined, TextureUsageState::TransferDst, true),
        CommandOp::CopyFrameTargetToTexture { src: copy_only_frame, dst: snapshot, extent: next.extent },
        barrier(snapshot, TextureUsageState::TransferDst, TextureUsageState::TransferSrc, true),
        CommandOp::CopyTextureToBuffer(BufferImageCopyRegion { buffer: readback, buffer_offset: 0, bytes_per_row: 4,
            rows_per_image: 1, texture: snapshot, texture_mip: 0, texture_layer: 0,
            texture_origin: TextureOrigin3d { x: 64, y: 64, z: 0 }, extent: Extent3d { width: 1, height: 1, depth: 1 } }),
        barrier(readback, TextureUsageState::TransferDst, TextureUsageState::ShaderRead, false),
        CommandOp::HostReadBuffer { buffer: readback, offset: 0, size: 4 },
    ]);
    let list = gal.create_command_list(CommandListDesc { label: "copy-only.commands".into(), operations: ops }).unwrap();
    let token = gal.submit(SubmissionBatch { label: "copy-only.submit".into(), command_lists: vec![list] }).unwrap();
    gal.retire_through(token.submission).unwrap();
    observed.push(gal.completed_host_reads().iter().find(|r| r.buffer == readback && r.submission == token.submission).unwrap().bytes.clone());
    gal.present_frame(PresentFrameDesc { frame: next.frame, correlation_id: next.correlation_id, wait_for: token.submission }).unwrap();
    gal.destroy(copy_only_frame).unwrap();
    for handle in [snapshot, readback, pass] { gal.destroy(handle).unwrap(); }
    for handle in owner.handles_in_destroy_order() { gal.destroy(handle).unwrap(); }
    gal.destroy(frame).unwrap();
    assert_eq!(gal.metrics().resource_creates, gal.metrics().resource_destroys);
    assert_eq!(gal.metrics().validation_failures, 0);
    assert_eq!(observed, vec![vec![0,0,255,255], vec![0,255,0,255], vec![0,255,0,255], vec![0,0,255,255]],
        "BGRA readback must retain copied red, honor green Clear, retain green across submissions, and present copy-only red");
}

#[test]
#[ignore = "opt-in numeric diagnostic, not a visual acceptance gate"]
fn observe_frozen_triangle_interpolation_and_y_reflection() {
    for opengl in [false, true] {
        for mirrored in [false, true] {
          for rotation in 0..3 {
            for expression in ["v_uv.x", "v_uv.y", "gl_FragCoord.w", "gl_FragCoord.z"] {
                let bits = observe(opengl, mirrored, rotation, expression, CullMode::None);
                eprintln!("terrain-interpolation-observation backend={} mirrored={mirrored} rotation={rotation} value={expression} bits={bits:#010x} float={}",
                    if opengl { "opengl" } else { "vulkan" }, f32::from_bits(bits));
                assert!(f32::from_bits(bits).is_finite());
                assert!((0.0..1.0).contains(&f32::from_bits(bits)), "must read covered triangle, not clear/background");
            }
          }
        }
    }
}

#[test]
fn explicit_raster_y_direction_preserves_culling_and_observes_frozen_orientation() {
    let mut visible = Vec::new();
    for opengl in [false, true] {
        let mut modes = Vec::new();
        for mirrored in [false, true] {
            let none = observe(opengl, mirrored, 0, "v_uv.y", CullMode::None);
            let back = observe(opengl, mirrored, 0, "v_uv.y", CullMode::Back);
            let front = observe(opengl, mirrored, 0, "v_uv.y", CullMode::Front);
            assert_ne!(none, 0);
            assert_ne!(back == 0, front == 0, "exactly one cull face must retain the triangle");
            assert_eq!(if back != 0 { back } else { front }, none);
            modes.push((none, back != 0));
        }
        assert_eq!(modes[0].1, modes[1].1, "Y direction must preserve logical front-face meaning");
        visible.push(modes);
    }
    assert_eq!(visible[0][1].0, visible[1][0].0, "explicit Vulkan Down must match measured Frozen/OpenGL orientation on this fixture");
}

/// Exercise the attachment boundary required before adopting Down in the world
/// graph. Color-only readback cannot detect an upside-down depth consumer.
#[test]
fn explicit_raster_world_color_and_depth_compose_before_canonical_overlay() {
    let bytes = compose_world_attachments(TextureRowOrder::Reverse, TextureRowOrder::Reverse, RasterYDirection::Up);
    for y in 0..8 {
        for x in 0..8 {
            let expected = if y >= 4 { [0,255,0,255] } else if x < 4 { [255,255,0,255] } else { [255,0,0,255] };
            assert_eq!(&bytes[(y*8+x)*4..(y*8+x+1)*4], &expected, "pixel ({x},{y})");
        }
    }
    // Deliberately wrong *explicit* command streams prove the fixture detects
    // independently reversed color, unreversed depth, and a flipped GUI.
    // These are negative controls, never alternative accepted game paths.
    let wrong_color = compose_world_attachments(TextureRowOrder::Preserve, TextureRowOrder::Reverse, RasterYDirection::Up);
    let wrong_depth = compose_world_attachments(TextureRowOrder::Reverse, TextureRowOrder::Preserve, RasterYDirection::Up);
    let wrong_gui = compose_world_attachments(TextureRowOrder::Reverse, TextureRowOrder::Reverse, RasterYDirection::Down);
    let pixel = |data: &[u8], x: usize, y: usize| <[u8; 4]>::try_from(&data[(y*8+x)*4..(y*8+x+1)*4]).unwrap();
    assert_eq!(pixel(&wrong_color, 6, 1), [0,0,255,255], "unreversed color must expose the blue lower world half");
    assert_eq!(pixel(&wrong_depth, 6, 1), [0,255,0,255], "unreversed depth must incorrectly admit the green overlay");
    assert_eq!(pixel(&wrong_gui, 1, 6), [255,255,0,255], "Down GUI must visibly land in the wrong half");
    assert_ne!(wrong_color, bytes);
    assert_ne!(wrong_depth, bytes);
    assert_ne!(wrong_gui, bytes);
}

fn compose_world_attachments(color_rows: TextureRowOrder, depth_rows: TextureRowOrder, gui_direction: RasterYDirection) -> Vec<u8> {
    let mut gal = VulkanicGal::new_with_backend(Box::new(
        super::vulkan::VulkanBackend::new("oriented world composition").unwrap()), false);
    let mut owned = Vec::new();
    macro_rules! own { ($call:expr) => {{ let handle = $call.unwrap(); owned.push(handle); handle }}; }
    let extent = Extent3d { width: 8, height: 8, depth: 1 };
    let mut attachments = Vec::new();
    let mut targets = Vec::new();
    for (label, direction) in [("world", RasterYDirection::Down), ("canonical", RasterYDirection::Up)] {
        let target = OrientedWorldTarget::create(&mut gal, label, WorldTargetDesc {
            extent, color_format: TextureFormat::Rgba8Unorm, raster_y_direction: direction,
        }).unwrap();
        owned.extend(target.handles_in_destroy_order().into_iter().rev());
        attachments.push((vec![(target.color_texture, target.color_view), (target.depth_texture, target.depth_view)], target.target, target.pass));
        targets.push(target);
    }
    let layout = own!(gal.create_pipeline_layout(PipelineLayoutDesc {
        label: "composition.layout".into(), resource_layouts: vec![],
    }));
    let vertex_shader = own!(gal.create_shader_module(ShaderModuleDesc {
        label: "composition.vertex".into(), stage: ShaderStage::Vertex,
        code_format: ShaderCodeFormat::Glsl, entry_point: "main".into(),
        code: br#"#version 450
layout(location=0) out vec2 logical_position;
const vec2 p[6]=vec2[6](vec2(-1,-1),vec2(1,-1),vec2(1,1),vec2(-1,-1),vec2(1,1),vec2(-1,1));
void main(){ logical_position=p[gl_VertexIndex]; gl_Position=vec4(logical_position,0.5,1); }
"#.to_vec(),
    }));
    let mut pipelines = Vec::new();
    for (label, direction, depth_compare, depth_write, body) in [
        ("world", RasterYDirection::Down, Some(CompareOp::Always), true,
            "bool top=logical_position.y>0; color=top?vec4(1,0,0,1):vec4(0,0,1,1); gl_FragDepth=top?0.25:0.75;"),
        ("depth-consumer", RasterYDirection::Up, Some(CompareOp::Less), false,
            "color=vec4(0,1,0,1);"),
        ("gui", gui_direction, None, false,
            "if(logical_position.x>=0 || logical_position.y<=0) discard; color=vec4(1,1,0,1);"),
    ] {
        let fragment_shader = own!(gal.create_shader_module(ShaderModuleDesc {
            label: format!("{label}.fragment"), stage: ShaderStage::Fragment,
            code_format: ShaderCodeFormat::Glsl, entry_point: "main".into(),
            code: format!("#version 450\nlayout(location=0) in vec2 logical_position;\nlayout(location=0) out vec4 color;\nvoid main(){{{body}}}").into_bytes(),
        }));
        let pipeline = own!(gal.create_graphics_pipeline(GraphicsPipelineDesc {
            label: label.into(), layout, vertex_shader, fragment_shader,
            topology: PrimitiveTopology::Triangles, cull_mode: CullMode::None,
            front_face: FrontFace::CounterClockwise, provoking_vertex: ProvokingVertex::Last,
            raster_y_direction: direction, blend: BlendMode::Disabled,
            depth_compare, depth_write, depth_bias: None,
            color_formats: vec![TextureFormat::Rgba8Unorm], depth_format: Some(TextureFormat::Depth32Float), stencil: None,
        }));
        pipelines.push(pipeline);
    }
    let readback = own!(gal.create_buffer(BufferDesc {
        label: "composition.readback".into(), size: 256, memory: MemoryDomain::Readback,
        usages: vec![BufferUsage::TransferDst, BufferUsage::HostRead],
    }));
    let range = Some(TextureSubresourceRange { base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 });
    let barrier = |resource, before, after, subresources| CommandOp::Barrier(ResourceBarrier {
        resource, before, after, subresources, src_queue: QueueClass::Graphics, dst_queue: QueueClass::Graphics,
    });
    let begin = |index: usize, load_op| {
        let (pair, target, pass) = &attachments[index];
        CommandOp::BeginPass { pass: *pass, target: *target,
            colors: vec![PassAttachment { view: pair[0].1, load_op,
                store_op: AttachmentStoreOp::Store, clear_color: Some(ClearColor { r: 0., g: 0., b: 0., a: 0. }) }],
            depth_stencil: Some(PassAttachment { view: pair[1].1, load_op,
                store_op: AttachmentStoreOp::Store, clear_color: None }),
        }
    };
    let mut operations = Vec::new();
    for (i, usage) in [TextureUsageState::ColorAttachment, TextureUsageState::DepthStencilAttachment].into_iter().enumerate() {
        operations.push(barrier(attachments[0].0[i].0, TextureUsageState::Undefined, usage, range));
    }
    operations.extend([begin(0, AttachmentLoadOp::Clear), CommandOp::BindGraphicsPipeline(pipelines[0]),
        CommandOp::Draw { vertices: 6, instances: 1 }, CommandOp::EndPass]);
    let mut transfer = targets[0].transfer_to(&targets[1], WorldAttachmentStates::ATTACHMENTS,
        WorldAttachmentStates::UNDEFINED, WorldAttachmentStates::ATTACHMENTS).unwrap();
    for operation in &mut transfer {
        if let CommandOp::CopyTexture(copy) = operation {
            // Only negative controls alter the production transfer plan.
            let requested = if copy.src_texture == targets[0].color_texture { color_rows } else { depth_rows };
            if requested != TextureRowOrder::Reverse { copy.row_order = requested; }
        }
    }
    operations.extend(transfer);
    operations.extend([
        begin(1, AttachmentLoadOp::Load),
        // Exercise direction changes in one pass, not only at BeginPass.
        CommandOp::BindGraphicsPipeline(pipelines[0]),
        CommandOp::BindGraphicsPipeline(pipelines[1]), CommandOp::Draw { vertices: 6, instances: 1 },
        CommandOp::BindGraphicsPipeline(pipelines[2]), CommandOp::Draw { vertices: 6, instances: 1 },
        CommandOp::EndPass,
        barrier(attachments[1].0[0].0, TextureUsageState::ColorAttachment, TextureUsageState::TransferSrc, range),
        CommandOp::CopyTextureToBuffer(BufferImageCopyRegion {
            buffer: readback, buffer_offset: 0, bytes_per_row: 32, rows_per_image: 8,
            texture: attachments[1].0[0].0, texture_mip: 0, texture_layer: 0,
            texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 }, extent,
        }),
        barrier(readback, TextureUsageState::TransferDst, TextureUsageState::ShaderRead, None),
        CommandOp::HostReadBuffer { buffer: readback, offset: 0, size: 256 },
    ]);
    let list = gal.create_command_list(CommandListDesc { label: "composition.commands".into(), operations }).unwrap();
    let token = gal.submit(SubmissionBatch { label: "composition.submit".into(), command_lists: vec![list] }).unwrap();
    gal.retire_through(token.submission).unwrap();
    let reads = gal.completed_host_reads();
    let bytes = reads.iter().find(|r| r.buffer == readback).unwrap().bytes.clone();
    for handle in owned.into_iter().rev() { gal.destroy(handle).unwrap(); }
    assert_eq!(gal.metrics().validation_failures, 0);
    assert_eq!(gal.metrics().resource_creates, gal.metrics().resource_destroys);
    bytes
}

fn observe(opengl: bool, mirrored: bool, rotation: u32, expression: &str, cull_mode: CullMode) -> u32 {
    let mut gal = if opengl {
        VulkanicGal::new_with_backend(Box::new(super::opengl::OpenGlBackend::new("terrain interpolation observation").unwrap()), false)
    } else {
        VulkanicGal::new_with_backend(Box::new(super::vulkan::VulkanBackend::new("terrain interpolation observation").unwrap()), false)
    };
    let mut owned = Vec::new();
    macro_rules! own { ($call:expr) => {{ let h = $call.unwrap(); owned.push(h); h }}; }
    let extent = Extent3d { width: 1280, height: 720, depth: 1 };
    let color = own!(gal.create_texture(TextureDesc { label: "interpolation.color".into(),
        dimension: TextureDimension::D2, format: TextureFormat::Rgba8Unorm, extent, mip_levels: 1, array_layers: 1,
        usages: vec![TextureUsage::ColorAttachment, TextureUsage::TransferSrc] }));
    let view = own!(gal.create_texture_view(TextureViewDesc { label: "interpolation.view".into(), texture: color,
        format: TextureFormat::Rgba8Unorm, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 }));
    let target = own!(gal.create_render_target(RenderTargetDesc { label: "interpolation.target".into(),
        color_views: vec![view], depth_stencil_view: None, extent }));
    let pass = own!(gal.create_render_pass(RenderPassDesc { label: "interpolation.pass".into(),
        target, color_formats: vec![TextureFormat::Rgba8Unorm], depth_format: None }));
    let layout = own!(gal.create_pipeline_layout(PipelineLayoutDesc { label: "interpolation.layout".into(), resource_layouts: vec![] }));
    let id = format!("({}+{rotation})%3", if opengl { "gl_VertexID" } else { "gl_VertexIndex" });
    // Actual second front-face triangle: vertex indices 2,3,0. All values below
    // were measured equal on Rust Vulkan and Frozen OpenGL (r526), not CPU math.
    let vertex = format!(r#"#version 450
layout(location=0) out vec2 v_uv;
const uvec4 p[3] = uvec4[3](
    uvec4(1058089181u,3189729878u,1076964127u,1077382980u),
    uvec4(1058089181u,1066044388u,1076235759u,1076654648u),
    uvec4(3186467614u,1065570418u,1075166633u,1075585574u));
const uvec2 t[3] = uvec2[3](uvec2(1054998560u,1062469600u),
    uvec2(1054998560u,1062338592u),uvec2(1055129568u,1062338592u));
void main() {{
    gl_Position=uintBitsToFloat(p[{id}]);
    v_uv=uintBitsToFloat(t[{id}]);
    {depth}
    {mirror}
}}
"#, depth=if opengl { "" } else { "gl_Position.z=gl_Position.z*0.5+gl_Position.w*0.5;" },
        mirror="");
    let vertex_shader = own!(gal.create_shader_module(ShaderModuleDesc { label: "interpolation.vertex".into(),
        stage: ShaderStage::Vertex, code_format: ShaderCodeFormat::Glsl, code: vertex.into_bytes(), entry_point: "main".into() }));
    let fragment = format!(r#"#version 450
layout(location=0) in vec2 v_uv;
layout(location=0) out vec4 color;
void main() {{ uint w=floatBitsToUint({expression}); color=vec4(uvec4(w,w>>8u,w>>16u,w>>24u)&255u)/255.0; }}
"#);
    let fragment_shader = own!(gal.create_shader_module(ShaderModuleDesc { label: "interpolation.fragment".into(),
        stage: ShaderStage::Fragment, code_format: ShaderCodeFormat::Glsl, code: fragment.into_bytes(), entry_point: "main".into() }));
    let pipeline = own!(gal.create_graphics_pipeline(GraphicsPipelineDesc { label: "interpolation.pipeline".into(),
        layout, vertex_shader, fragment_shader, topology: PrimitiveTopology::Triangles, cull_mode,
        front_face: FrontFace::CounterClockwise, provoking_vertex: ProvokingVertex::Last,
        raster_y_direction: if mirrored { RasterYDirection::Down } else { RasterYDirection::Up }, blend: BlendMode::Disabled,
        depth_compare: None, depth_write: false, depth_bias: None, color_formats: vec![TextureFormat::Rgba8Unorm],
        depth_format: None, stencil: None }));
    let readback = own!(gal.create_buffer(BufferDesc { label: "interpolation.readback".into(), size: 4,
        memory: MemoryDomain::Readback, usages: vec![BufferUsage::TransferDst, BufferUsage::HostRead] }));
    let barrier = |resource, before, after, subresources| CommandOp::Barrier(ResourceBarrier {
        resource, before, after, subresources, src_queue: QueueClass::Graphics, dst_queue: QueueClass::Graphics });
    let range = Some(TextureSubresourceRange { base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 });
    let mut operations = vec![
        barrier(color, TextureUsageState::Undefined, TextureUsageState::ColorAttachment, range),
        CommandOp::BeginPass { pass, target, colors: vec![PassAttachment { view, load_op: AttachmentLoadOp::Clear,
            store_op: AttachmentStoreOp::Store, clear_color: Some(ClearColor { r: 0., g: 0., b: 0., a: 0. }) }], depth_stencil: None },
        CommandOp::BindGraphicsPipeline(pipeline), CommandOp::Draw { vertices: 3, instances: 1 }, CommandOp::EndPass,
        barrier(color, TextureUsageState::ColorAttachment, TextureUsageState::TransferSrc, range),
    ];
    // Vulkan owns the complete raster-plus-row-copy prerequisite. Rust OpenGL
    // does not advertise row reversal; its direction is observed at the reflected
    // coordinate without submitting an unsupported operation or a fallback.
    let canonical_color = if mirrored && !opengl {
        let canonical = own!(gal.create_texture(TextureDesc { label: "interpolation.canonical-color".into(),
            dimension: TextureDimension::D2, format: TextureFormat::Rgba8Unorm, extent, mip_levels: 1, array_layers: 1,
            usages: vec![TextureUsage::TransferDst, TextureUsage::TransferSrc] }));
        operations.push(barrier(canonical, TextureUsageState::Undefined, TextureUsageState::TransferDst, range));
        operations.push(CommandOp::CopyTexture(TextureImageCopyRegion { row_order: TextureRowOrder::Reverse,
            src_texture: color, src_mip: 0, src_layer: 0, src_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
            dst_texture: canonical, dst_mip: 0, dst_layer: 0, dst_origin: TextureOrigin3d { x: 0, y: 0, z: 0 }, extent }));
        operations.push(barrier(canonical, TextureUsageState::TransferDst, TextureUsageState::TransferSrc, range));
        canonical
    } else { color };
    operations.extend([
        CommandOp::CopyTextureToBuffer(BufferImageCopyRegion { buffer: readback, buffer_offset: 0, bytes_per_row: 4,
            rows_per_image: 1, texture: canonical_color, texture_mip: 0, texture_layer: 0,
            texture_origin: TextureOrigin3d { x: 764, y: if mirrored && opengl { 399 } else { 320 }, z: 0 },
            extent: Extent3d { width: 1, height: 1, depth: 1 } }),
        barrier(readback, TextureUsageState::TransferDst, TextureUsageState::ShaderRead, None),
        CommandOp::HostReadBuffer { buffer: readback, offset: 0, size: 4 },
    ]);
    let list = gal.create_command_list(CommandListDesc { label: "interpolation.commands".into(), operations }).unwrap();
    let token = gal.submit(SubmissionBatch { label: "interpolation.submit".into(), command_lists: vec![list] }).unwrap();
    gal.retire_through(token.submission).unwrap();
    let reads = gal.completed_host_reads();
    let bytes = &reads.iter().find(|r| r.buffer == readback).unwrap().bytes;
    let word = u32::from_le_bytes(bytes.as_slice().try_into().unwrap());
    for handle in owned.into_iter().rev() { gal.destroy(handle).unwrap(); }
    assert_eq!(gal.metrics().validation_failures, 0);
    assert_eq!(gal.metrics().resource_creates, gal.metrics().resource_destroys);
    word
}
