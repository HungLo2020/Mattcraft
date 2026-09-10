//! Supplemental GPU test of the explicit graphics-storage observation path.
//! This is not a replacement for Frozen/game visual parity.
use crate::render::vulkanic::commands::*;
use crate::render::vulkanic::gal::VulkanicGal;
use crate::render::vulkanic::handles::Handle;
use crate::render::vulkanic::resources::*;

const LIMIT: usize = 32;
const WORDS: usize = 3 + LIMIT * 8;
const BYTES: u64 = (WORDS * 4) as u64;

fn barrier(resource: Handle, before: TextureUsageState, after: TextureUsageState) -> CommandOp {
    CommandOp::Barrier(ResourceBarrier {
        resource, subresources: None, before, after,
        src_queue: QueueClass::Graphics, dst_queue: QueueClass::Graphics,
    })
}

#[test]
fn graphics_storage_observes_every_vertex_and_fragment_with_explicit_reset_copy_and_retirement() {
    for opengl in [false, true] {
        let mut gal = if opengl {
            VulkanicGal::new_with_backend(Box::new(super::opengl::OpenGlBackend::new(
                "graphics storage OpenGL").expect("OpenGL required for storage conformance")), false)
        } else {
            VulkanicGal::new_with_backend(Box::new(super::vulkan::VulkanBackend::new(
                "graphics storage Vulkan").expect("Vulkan required for storage conformance")), false)
        };
        let mut owned = Vec::new();
        macro_rules! own {
            ($call:expr) => {{ let handle = $call.unwrap(); owned.push(handle); handle }};
        }
        let seed = own!(gal.create_buffer(BufferDesc {
            label: "graphics-storage.seed".into(), size: BYTES, memory: MemoryDomain::Upload,
            usages: vec![BufferUsage::HostWrite, BufferUsage::TransferSrc],
        }));
        let output = own!(gal.create_buffer(BufferDesc {
            label: "graphics-storage.output".into(), size: BYTES, memory: MemoryDomain::DeviceLocal,
            usages: vec![BufferUsage::Storage, BufferUsage::TransferSrc, BufferUsage::TransferDst],
        }));
        let readback = own!(gal.create_buffer(BufferDesc {
            label: "graphics-storage.readback".into(), size: BYTES, memory: MemoryDomain::Readback,
            usages: vec![BufferUsage::TransferDst, BufferUsage::HostRead],
        }));
        let color_readback = own!(gal.create_buffer(BufferDesc {
            label: "graphics-storage.color-readback".into(), size: 4, memory: MemoryDomain::Readback,
            usages: vec![BufferUsage::TransferDst, BufferUsage::HostRead],
        }));
        let texture_readback = own!(gal.create_buffer(BufferDesc {
            label: "graphics-storage.texture-readback".into(), size: BYTES, memory: MemoryDomain::Readback,
            usages: vec![BufferUsage::TransferDst, BufferUsage::HostRead],
        }));
        let partial_readback = own!(gal.create_buffer(BufferDesc {
            label: "graphics-storage.partial-readback".into(), size: BYTES, memory: MemoryDomain::Readback,
            usages: vec![BufferUsage::TransferDst, BufferUsage::HostRead],
        }));
        let byte_extent = Extent3d { width: WORDS as u32, height: 1, depth: 1 };
        let byte_texture = own!(gal.create_texture(TextureDesc {
            label: "graphics-storage.byte-texture".into(), dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm, extent: byte_extent, mip_levels: 1, array_layers: 1,
            usages: vec![TextureUsage::TransferDst, TextureUsage::TransferSrc],
        }));
        let extent = Extent3d { width: 1, height: 1, depth: 1 };
        let color = own!(gal.create_texture(TextureDesc {
            label: "graphics-storage.color".into(), dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm, extent, mip_levels: 1, array_layers: 1,
            usages: vec![TextureUsage::ColorAttachment, TextureUsage::TransferSrc],
        }));
        let view = own!(gal.create_texture_view(TextureViewDesc {
            label: "graphics-storage.view".into(), texture: color, format: TextureFormat::Rgba8Unorm,
            base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1,
        }));
        let target = own!(gal.create_render_target(RenderTargetDesc {
            label: "graphics-storage.target".into(), color_views: vec![view], depth_stencil_view: None, extent,
        }));
        let pass = own!(gal.create_render_pass(RenderPassDesc {
            label: "graphics-storage.pass".into(), target, color_formats: vec![TextureFormat::Rgba8Unorm], depth_format: None,
        }));
        let resource_layout = own!(gal.create_resource_layout(ResourceLayoutDesc {
            label: "graphics-storage.layout".into(), bindings: vec![ResourceBindingDesc {
                binding: 0, kind: ResourceBindingKind::StorageBuffer, stages: PipelineStageFlags::DRAW,
                array_count: 1, optional: false, dynamic_offset_count: 0,
            }],
        }));
        let set = own!(gal.create_resource_set(ResourceSetDesc {
            label: "graphics-storage.set".into(), layout: resource_layout,
            bindings: vec![ResourceBinding {
                binding: 0, array_index: 0, resource: output, kind: ResourceBindingKind::StorageBuffer,
                access: AccessFlags(AccessFlags::READ.0 | AccessFlags::WRITE.0),
                dynamic_offsets: vec![], buffer_range: Some(BYTES),
            }],
        }));
        let layout = own!(gal.create_pipeline_layout(PipelineLayoutDesc {
            label: "graphics-storage.pipeline-layout".into(), resource_layouts: vec![resource_layout],
        }));
        let id = if opengl { "gl_VertexID" } else { "gl_VertexIndex" };
        let qualifier = if opengl { "" } else { "set=0," };
        // Use the portable shader interface's canonical name for binding zero.
        let declaration = format!("layout({qualifier}binding=0,std430) buffer Storage0 {{ uint words[]; }};\n");
        let vertex_source = format!(r#"#version 450
{declaration}
const vec2 p[3]=vec2[3](vec2(-1,-1),vec2(3,-1),vec2(-1,3));
void main() {{
    gl_Position=vec4(p[{id}],0,1);
    uint slot=atomicAdd(words[0],1u);
    if (slot<32u) {{
        uint offset=3u+slot*8u;
        words[offset]=uint({id});
        uvec4 clip=floatBitsToUint(gl_Position);
        words[offset+1u]=clip.x; words[offset+2u]=clip.y;
        words[offset+3u]=clip.z; words[offset+4u]=clip.w;
        words[offset+5u]=17u; words[offset+6u]=23u; words[offset+7u]=words[2];
    }}
}}
"#);
        let fragment_source = format!("#version 450\n{declaration}\nlayout(location=0) out vec4 color;\nvoid main() {{ atomicAdd(words[1],1u); color=vec4(1,0,1,1); }}");
        let vertex = own!(gal.create_shader_module(ShaderModuleDesc {
            label: "graphics-storage.vertex".into(), stage: ShaderStage::Vertex,
            code_format: ShaderCodeFormat::Glsl, code: vertex_source.into_bytes(), entry_point: "main".into(),
        }));
        let fragment = own!(gal.create_shader_module(ShaderModuleDesc {
            label: "graphics-storage.fragment".into(), stage: ShaderStage::Fragment,
            code_format: ShaderCodeFormat::Glsl, code: fragment_source.into_bytes(), entry_point: "main".into(),
        }));
        let pipeline = own!(gal.create_graphics_pipeline(GraphicsPipelineDesc {
            label: "graphics-storage.pipeline".into(), layout, vertex_shader: vertex, fragment_shader: fragment,
            topology: PrimitiveTopology::Triangles, cull_mode: CullMode::None,
            front_face: FrontFace::CounterClockwise, provoking_vertex: ProvokingVertex::Last,
            raster_y_direction: crate::render::vulkanic::resources::RasterYDirection::Up,
            blend: BlendMode::Disabled, depth_compare: None, depth_write: false, depth_bias: None,
            color_formats: vec![TextureFormat::Rgba8Unorm], depth_format: None, stencil: None,
        }));
        let mut first_count = None;
        for epoch in [41u32, 42u32] {
            let first = epoch == 41;
            let mut initial = vec![0xdeadbeefu32; WORDS];
            initial[..3].copy_from_slice(&[0, 0, epoch]);
            let mut operations = Vec::new();
            if !first {
                operations.push(barrier(seed, TextureUsageState::TransferSrc, TextureUsageState::TransferDst));
                operations.push(barrier(readback, TextureUsageState::ShaderRead, TextureUsageState::TransferDst));
                operations.push(barrier(color_readback, TextureUsageState::ShaderRead, TextureUsageState::TransferDst));
                operations.push(barrier(texture_readback, TextureUsageState::ShaderRead, TextureUsageState::TransferDst));
                operations.push(barrier(partial_readback, TextureUsageState::ShaderRead, TextureUsageState::TransferDst));
            }
            operations.extend([
                CommandOp::HostWriteBuffer { buffer: seed, offset: 0, data: initial.iter().flat_map(|v| v.to_le_bytes()).collect() },
                barrier(seed, TextureUsageState::TransferDst, TextureUsageState::TransferSrc),
                barrier(output, if first { TextureUsageState::Undefined } else { TextureUsageState::TransferSrc }, TextureUsageState::TransferDst),
                CommandOp::CopyBuffer { src: seed, dst: output, size: BYTES },
                barrier(output, TextureUsageState::TransferDst, TextureUsageState::ShaderWrite),
                CommandOp::Barrier(ResourceBarrier { resource: color,
                    subresources: Some(TextureSubresourceRange { base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 }),
                    before: if first { TextureUsageState::Undefined } else { TextureUsageState::TransferSrc },
                    after: TextureUsageState::ColorAttachment, src_queue: QueueClass::Graphics, dst_queue: QueueClass::Graphics }),
                CommandOp::BeginPass { pass, target, colors: vec![PassAttachment {
                    view, load_op: AttachmentLoadOp::Clear, store_op: AttachmentStoreOp::Store,
                    clear_color: Some(ClearColor { r: 0.0, g: 1.0, b: 0.0, a: 1.0 }),
                }], depth_stencil: None },
                CommandOp::BindGraphicsPipeline(pipeline),
                CommandOp::BindResourceSet { pipeline_layout: layout, set_index: 0, set, dynamic_offsets: vec![] },
                CommandOp::Draw { vertices: 3, instances: 1 }, CommandOp::EndPass,
                barrier(output, TextureUsageState::ShaderWrite, TextureUsageState::TransferSrc),
                CommandOp::CopyBuffer { src: output, dst: readback, size: BYTES },
                barrier(readback, TextureUsageState::TransferDst, TextureUsageState::ShaderRead),
                CommandOp::HostReadBuffer { buffer: readback, offset: 0, size: BYTES },
                CommandOp::Barrier(ResourceBarrier { resource: byte_texture,
                    subresources: Some(TextureSubresourceRange { base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 }),
                    before: if first { TextureUsageState::Undefined } else { TextureUsageState::TransferSrc },
                    after: TextureUsageState::TransferDst, src_queue: QueueClass::Graphics, dst_queue: QueueClass::Graphics }),
                CommandOp::CopyBufferToTexture(BufferImageCopyRegion { buffer: output, buffer_offset: 0,
                    bytes_per_row: BYTES as u32, rows_per_image: 1, texture: byte_texture,
                    texture_mip: 0, texture_layer: 0, texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 }, extent: byte_extent }),
                CommandOp::Barrier(ResourceBarrier { resource: byte_texture,
                    subresources: Some(TextureSubresourceRange { base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 }),
                    before: TextureUsageState::TransferDst, after: TextureUsageState::TransferSrc,
                    src_queue: QueueClass::Graphics, dst_queue: QueueClass::Graphics }),
                CommandOp::CopyTextureToBuffer(BufferImageCopyRegion { buffer: texture_readback, buffer_offset: 0,
                    bytes_per_row: BYTES as u32, rows_per_image: 1, texture: byte_texture,
                    texture_mip: 0, texture_layer: 0, texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 }, extent: byte_extent }),
                barrier(texture_readback, TextureUsageState::TransferDst, TextureUsageState::ShaderRead),
                CommandOp::HostReadBuffer { buffer: texture_readback, offset: 0, size: BYTES },
                CommandOp::Barrier(ResourceBarrier { resource: color,
                    subresources: Some(TextureSubresourceRange { base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 }),
                    before: TextureUsageState::ColorAttachment, after: TextureUsageState::TransferSrc,
                    src_queue: QueueClass::Graphics, dst_queue: QueueClass::Graphics }),
                CommandOp::CopyTextureToBuffer(BufferImageCopyRegion { buffer: color_readback,
                    buffer_offset: 0, bytes_per_row: 4, rows_per_image: 1, texture: color,
                    texture_mip: 0, texture_layer: 0, texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 }, extent }),
                barrier(color_readback, TextureUsageState::TransferDst, TextureUsageState::ShaderRead),
                CommandOp::HostReadBuffer { buffer: color_readback, offset: 0, size: 4 },
                barrier(output, TextureUsageState::TransferSrc, TextureUsageState::TransferDst),
                CommandOp::CopyTextureToBuffer(BufferImageCopyRegion { buffer: output, buffer_offset: 0,
                    bytes_per_row: 8, rows_per_image: 1, texture: color, texture_mip: 0, texture_layer: 0,
                    texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 }, extent }),
                barrier(output, TextureUsageState::TransferDst, TextureUsageState::TransferSrc),
                CommandOp::CopyBuffer { src: output, dst: partial_readback, size: BYTES },
                barrier(partial_readback, TextureUsageState::TransferDst, TextureUsageState::ShaderRead),
                CommandOp::HostReadBuffer { buffer: partial_readback, offset: 0, size: BYTES },
            ]);
            let list = gal.create_command_list(CommandListDesc { label: "graphics-storage.commands".into(), operations }).unwrap();
            let token = gal.submit(SubmissionBatch { label: "graphics-storage.submit".into(), command_lists: vec![list] }).unwrap();
            gal.retire_through_for_test(token.submission).unwrap();
            let reads = gal.completed_host_reads();
            let read = reads.iter().rev().find(|r| r.buffer == readback).unwrap();
            assert_eq!(reads.iter().rev().find(|r| r.buffer == texture_readback).unwrap().bytes,
                read.bytes, "GPU buffer-to-texture copy must not use an obsolete upload shadow");
            let partial = &reads.iter().rev().find(|r| r.buffer == partial_readback).unwrap().bytes;
            assert_eq!(&partial[..4], &[255,0,255,255]);
            assert_eq!(&partial[4..], &read.bytes[4..],
                "partial image copies must preserve GPU-written row padding and adjacent data");
            let words: Vec<u32> = read.bytes.chunks_exact(4).map(|v| u32::from_le_bytes(v.try_into().unwrap())).collect();
            let count = words[0] as usize;
            assert!((3..=LIMIT).contains(&count), "bounded vertex observations: {count}, OpenGL={opengl}, epoch={epoch}, header={:?}", &words[..3]);
            assert_eq!(*first_count.get_or_insert(count), count, "reset must not accumulate previous observations");
            assert_eq!(words[1], 1, "the normal fragment executes once");
            assert_eq!(words[2], epoch);
            let expected = [[-1.0f32,-1.0,0.0,1.0], [3.0,-1.0,0.0,1.0], [-1.0,3.0,0.0,1.0]];
            let mut seen = [false; 3];
            for sample in words[3..3+count*8].chunks_exact(8) {
                let index = sample[0] as usize;
                assert!(index < 3);
                seen[index] = true;
                assert_eq!(&sample[1..5], &expected[index].map(f32::to_bits));
                assert_eq!(&sample[5..], &[17,23,epoch], "must observe this submission's initial data");
            }
            assert!(seen.into_iter().all(|value| value));
            assert!(words[3+count*8..].iter().all(|value| *value == 0xdeadbeef), "unused slots must stay untouched");
            assert_eq!(reads.iter().rev().find(|r| r.buffer == color_readback).unwrap().bytes, [255,0,255,255]);
        }
        for handle in owned.into_iter().rev() { gal.destroy(handle).unwrap(); }
        assert_eq!(gal.metrics().resource_creates, gal.metrics().resource_destroys);
        assert_eq!(gal.metrics().validation_failures, 0);
    }
}
