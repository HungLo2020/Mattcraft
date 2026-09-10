use crate::render::vulkanic::commands::{BufferImageCopyRegion, TextureOrigin3d};
use crate::render::vulkanic::error::{GalError, StatusCode};
use crate::render::vulkanic::gal::VulkanicGal;
use crate::render::vulkanic::handles::Handle;
use crate::render::vulkanic::resources::*;
use crate::render::vulkanic::{
    AttachmentLoadOp, AttachmentStoreOp, BufferDesc, BufferUsage, ClearColor, CommandListDesc,
    CommandOp, MemoryDomain, PassAttachment, QueueClass, RenderPassDesc, RenderTargetDesc,
    ResourceBarrier, SubmissionBatch, TextureDesc, TextureFormat, TextureSubresourceRange,
    TextureUsage, TextureUsageState, TextureViewDesc,
};

/// Vignette is a destination-only blend: vanilla writes a grayscale mask and
/// multiplies the already-composed frame by `1 - source.rgb`.  Keep an actual
/// Vulkan readback here because descriptor/pipeline construction checks alone
/// cannot prove that dynamic rendering preserves the loaded color attachment
/// between the world and GUI passes.
#[test]
fn isolated_vulkan_vignette_blend_preserves_and_darkens_loaded_color() {
    isolated_loaded_blend(BlendMode::Vignette,[0.75,0.75,0.75,1.0],
        ClearColor {r:0.8,g:0.8,b:0.8,a:1.0},[51,51,51,255],1);
}

#[test]
fn isolated_vulkan_glint_blend_matches_frozen_and_preserves_destination_alpha() {
    // Frozen Java OpenGL BlendFunction.GLINT: SRC_COLOR, ONE, ZERO, ONE.
    // Distinct RGB and nonopaque alpha distinguish it from multiply, alpha
    // blend, source-alpha replacement and plain additive blending.
    // Avoid the hardware's half-intensity blend-factor rounding boundary;
    // quarter-intensity probes retain exact byte assertions on this fixture.
    isolated_loaded_blend(BlendMode::Glint,[0.25,0.25,0.75,0.125],
        ClearColor {r:0.125,g:0.375,b:0.0625,a:0.375},[48,112,159,96],0);
}

fn isolated_loaded_blend(blend:BlendMode, source:[f32;4], clear:ClearColor, expected:[u8;4], tolerance:i16) {
    isolated_loaded_raster(blend, source, clear, expected, tolerance, false, None);
}

#[test]
fn explicit_provoking_vertex_selects_the_declared_flat_value_on_both_backends() {
    for opengl in [false, true] {
        for (mode, red) in [(ProvokingVertex::First, 0), (ProvokingVertex::Last, 255)] {
            isolated_loaded_raster(BlendMode::Disabled, [0.0;4],
                ClearColor {r:0.0,g:1.0,b:0.0,a:1.0}, [red,0,0,255], 0, opengl, Some(mode));
        }
    }
}

fn isolated_loaded_raster(blend:BlendMode, source:[f32;4], clear:ClearColor, expected:[u8;4], tolerance:i16,
    opengl: bool, provoking: Option<ProvokingVertex>) {
    let mut gal = if opengl {
        VulkanicGal::new_with_backend(Box::new(super::opengl::OpenGlBackend::new(
            "explicit provoking vertex OpenGL conformance").expect("OpenGL required for provoking vertex readback")), false)
    } else {
    let backend = match super::vulkan::VulkanBackend::new("MattMC VulkanicGAL vignette conformance") {
        Ok(backend) => backend,
        Err(error) => {
            assert!(provoking.is_none(), "Vulkan required for provoking vertex readback: {error}");
            assert_ne!(blend,BlendMode::Glint,"Vulkan required for glint blend regression: {error}");
            eprintln!("skipping Vulkan vignette conformance: {error}");
            return;
        }
    };
    VulkanicGal::new_with_backend(Box::new(backend), false)
    };
    let extent = Extent3d {
        width: 1,
        height: 1,
        depth: 1,
    };
    let color = gal
        .create_texture(TextureDesc {
            label: "vignette-conformance.color".to_owned(),
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            extent,
            mip_levels: 1,
            array_layers: 1,
            usages: vec![TextureUsage::ColorAttachment, TextureUsage::TransferSrc],
        })
        .unwrap();
    let color_view = gal
        .create_texture_view(view(
            "vignette-conformance.color-view",
            color,
            TextureFormat::Rgba8Unorm,
        ))
        .unwrap();
    let target = gal
        .create_render_target(RenderTargetDesc {
            label: "vignette-conformance.target".to_owned(),
            color_views: vec![color_view],
            depth_stencil_view: None,
            extent,
        })
        .unwrap();
    let pass = gal
        .create_render_pass(RenderPassDesc {
            label: "vignette-conformance.pass".to_owned(),
            target,
            color_formats: vec![TextureFormat::Rgba8Unorm],
            depth_format: None,
        })
        .unwrap();
    let layout = gal
        .create_pipeline_layout(PipelineLayoutDesc {
            label: "vignette-conformance.layout".to_owned(),
            resource_layouts: Vec::new(),
        })
        .unwrap();
    let vertex = gal
        .create_shader_module(ShaderModuleDesc {
            label: "vignette-conformance.vertex".to_owned(),
            stage: ShaderStage::Vertex,
            code_format: ShaderCodeFormat::Glsl,
            code: if provoking.is_some() {
                let id = if opengl { "gl_VertexID" } else { "gl_VertexIndex" };
                format!("#version 450\nlayout(location=0) flat out float selected;\nconst vec2 p[3]=vec2[3](vec2(-1,-1),vec2(3,-1),vec2(-1,3));\nvoid main() {{ gl_Position=vec4(p[{id}],0,1); selected=float({id})/2.0; }}").into_bytes()
            } else { br#"#version 450
const vec2 p[3] = vec2[3](vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
void main() { gl_Position = vec4(p[gl_VertexIndex], 0.0, 1.0); }
"#
            .to_vec() },
            entry_point: "main".to_owned(),
        })
        .unwrap();
    let fragment = gal
        .create_shader_module(ShaderModuleDesc {
            label: "vignette-conformance.fragment".to_owned(),
            stage: ShaderStage::Fragment,
            code_format: ShaderCodeFormat::Glsl,
            code: if provoking.is_some() {
                b"#version 450\nlayout(location=0) flat in float selected;\nlayout(location=0) out vec4 out_color;\nvoid main() { out_color=vec4(selected,0,0,1); }".to_vec()
            } else { format!("#version 450\nlayout(location = 0) out vec4 out_color;\nvoid main() {{ out_color = vec4({}, {}, {}, {}); }}\n",
                source[0],source[1],source[2],source[3]).into_bytes() },
            entry_point: "main".to_owned(),
        })
        .unwrap();
    let pipeline = gal
        .create_graphics_pipeline(GraphicsPipelineDesc {
            label: "vignette-conformance.pipeline".to_owned(),
            layout,
            vertex_shader: vertex,
            fragment_shader: fragment,
            topology: PrimitiveTopology::Triangles,
            cull_mode: CullMode::None,
            front_face: crate::render::vulkanic::resources::FrontFace::CounterClockwise,
            provoking_vertex: provoking.unwrap_or(ProvokingVertex::Last),
            raster_y_direction: crate::render::vulkanic::resources::RasterYDirection::Up,
            blend,
            depth_compare: None,
            depth_write: false,
            depth_bias: None,
            color_formats: vec![TextureFormat::Rgba8Unorm],
            depth_format: None,
            stencil: None,
        })
        .unwrap();
    let readback = gal
        .create_buffer(BufferDesc {
            label: "vignette-conformance.readback".to_owned(),
            size: 4,
            memory: MemoryDomain::Readback,
            usages: vec![BufferUsage::TransferDst, BufferUsage::HostRead],
        })
        .unwrap();
    let loaded = PassAttachment {
        view: color_view,
        load_op: AttachmentLoadOp::Load,
        store_op: AttachmentStoreOp::Store,
        clear_color: None,
    };
    let command_list = gal
        .create_command_list(CommandListDesc {
            label: "vignette-conformance.commands".to_owned(),
            operations: vec![
                texture_barrier(
                    color,
                    TextureUsageState::Undefined,
                    TextureUsageState::ColorAttachment,
                ),
                CommandOp::BeginPass {
                    pass,
                    target,
                    colors: vec![PassAttachment {
                        view: color_view,
                        load_op: AttachmentLoadOp::Clear,
                        store_op: AttachmentStoreOp::Store,
                        clear_color: Some(clear),
                    }],
                    depth_stencil: None,
                },
                CommandOp::EndPass,
                CommandOp::BeginPass {
                    pass,
                    target,
                    colors: vec![loaded],
                    depth_stencil: None,
                },
                CommandOp::BindGraphicsPipeline(pipeline),
                CommandOp::Draw {
                    vertices: 3,
                    instances: 1,
                },
                CommandOp::EndPass,
                texture_barrier(
                    color,
                    TextureUsageState::ColorAttachment,
                    TextureUsageState::TransferSrc,
                ),
                CommandOp::CopyTextureToBuffer(BufferImageCopyRegion {
                    buffer: readback,
                    buffer_offset: 0,
                    bytes_per_row: 4,
                    rows_per_image: 1,
                    texture: color,
                    texture_mip: 0,
                    texture_layer: 0,
                    texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
                    extent,
                }),
                CommandOp::Barrier(ResourceBarrier { resource: readback, subresources: None, before: TextureUsageState::TransferDst, after: TextureUsageState::ShaderRead, src_queue: QueueClass::Graphics, dst_queue: QueueClass::Graphics }),
                CommandOp::HostReadBuffer {
                    buffer: readback,
                    offset: 0,
                    size: 4,
                },
            ],
        })
        .unwrap();
    let token = gal
        .submit(SubmissionBatch {
            label: "vignette-conformance.submit".to_owned(),
            command_lists: vec![command_list],
        })
        .unwrap();
    gal.retire_through_for_test(token.submission).unwrap();
    let bytes = gal
        .completed_host_reads()
        .iter()
        .rev()
        .find(|read| read.buffer == readback)
        .expect("vignette conformance must produce a readback")
        .bytes
        .clone();
    for channel in 0..3 {
        assert!((i16::from(bytes[channel])-i16::from(expected[channel])).abs()<=tolerance,
            "{blend:?} channel{channel}: actual={bytes:?}, expected={expected:?}");
    }
    assert_eq!(expected[3],bytes[3],"{blend:?} must preserve destination alpha");
}

const WIDTH: u32 = 96;
const HEIGHT: u32 = 64;

#[test]
fn vulkan_and_opengl_clean_conformance_match_for_shared_graphics_subset() {
    let extent = Extent3d {
        width: WIDTH,
        height: HEIGHT,
        depth: 1,
    };
    let vulkan = match super::vulkan::conformance::run_conformance("matrix-shared", extent) {
        Ok(report) => report,
        Err(error) => {
            assert_environment_gap(&error, "Vulkan");
            return;
        }
    };
    let opengl = match super::opengl::conformance::run_conformance("matrix-shared", extent) {
        Ok(report) => report,
        Err(error) => {
            assert_environment_gap(&error, "OpenGL");
            return;
        }
    };
    assert_eq!(vulkan.width, opengl.width);
    assert_eq!(vulkan.height, opengl.height);
    assert_eq!(vulkan.pixel_hash, opengl.pixel_hash);
    assert_eq!(vulkan.non_zero_pixels, opengl.non_zero_pixels);
    assert_eq!(vulkan.evidence_json, opengl.evidence_json);
    assert!(vulkan
        .capabilities_json
        .contains("\"name\":\"Rust Vulkan\""));
    assert!(opengl
        .capabilities_json
        .contains("\"name\":\"Rust OpenGL\""));
    assert!(vulkan.capabilities_json.contains("\"compute\":true"));
    assert!(opengl.capabilities_json.contains("\"compute\":false"));
}

#[test]
fn shared_pass_shape_conformance_covers_mrt_and_depth_only() {
    for backend in [BackendKind::Vulkan, BackendKind::OpenGl] {
        let mut gal = match gal_for(backend, "MattMC pass shape conformance") {
            Ok(gal) => gal,
            Err(error) => {
                assert_environment_gap(&error, backend.name());
                continue;
            }
        };
        submit_mrt_clear(&mut gal)
            .unwrap_or_else(|error| panic!("{} MRT pass shape failed: {error}", backend.name()));
        submit_depth_only_clear(&mut gal).unwrap_or_else(|error| {
            panic!("{} depth-only pass shape failed: {error}", backend.name())
        });
    }
}

#[test]
fn shared_compare_sampler_conformance_creates_depth_only_pairs() {
    for backend in [BackendKind::Vulkan, BackendKind::OpenGl] {
        let mut gal = match gal_for(backend, "MattMC comparison sampler conformance") {
            Ok(gal) => gal,
            Err(error) => {
                assert_environment_gap(&error, backend.name());
                continue;
            }
        };
        let depth = gal
            .create_texture(TextureDesc {
                label: format!("{}.comparison-depth", backend.name()),
                dimension: TextureDimension::D2,
                format: TextureFormat::Depth32Float,
                extent: Extent3d {
                    width: 8,
                    height: 8,
                    depth: 1,
                },
                mip_levels: 1,
                array_layers: 1,
                usages: vec![TextureUsage::Sampled, TextureUsage::DepthStencilAttachment],
            })
            .unwrap_or_else(|error| panic!("{} depth texture failed: {error}", backend.name()));
        let depth_view = gal
            .create_texture_view(view(
                &format!("{}.comparison-depth-view", backend.name()),
                depth,
                TextureFormat::Depth32Float,
            ))
            .unwrap_or_else(|error| panic!("{} depth view failed: {error}", backend.name()));
        let sampler = gal
            .create_sampler(SamplerDesc {
                label: format!("{}.comparison-sampler", backend.name()),
                min_filter: SamplerFilter::Nearest,
                mag_filter: SamplerFilter::Nearest,
                mip_filter: SamplerFilter::Nearest,
                address_u: SamplerAddressMode::ClampToEdge,
                address_v: SamplerAddressMode::ClampToEdge,
                address_w: SamplerAddressMode::ClampToEdge,
                comparison: Some(CompareOp::LessOrEqual),
            })
            .unwrap_or_else(|error| {
                panic!("{} comparison sampler failed: {error}", backend.name())
            });
        let combined = gal
            .create_combined_texture_sampler(CombinedTextureSamplerDesc {
                label: format!("{}.comparison-combined", backend.name()),
                texture_view: depth_view,
                sampler,
            })
            .unwrap_or_else(|error| panic!("{} comparison pair failed: {error}", backend.name()));
        gal.destroy(combined).unwrap();
        gal.destroy(sampler).unwrap();
        gal.destroy(depth_view).unwrap();
        gal.destroy(depth).unwrap();
    }
}

#[test]
fn shared_mip_generation_round_trips_a_color_texel_on_both_backends() {
    for backend in [BackendKind::Vulkan, BackendKind::OpenGl] {
        let mut gal = match gal_for(backend, "MattMC mip generation conformance") {
            Ok(gal) => gal,
            Err(error) => {
                assert_environment_gap(&error, backend.name());
                continue;
            }
        };
        submit_mip_generation_round_trip(&mut gal, backend.name()).unwrap_or_else(|error| {
            panic!(
                "{} mip generation conformance failed: {error}",
                backend.name()
            )
        });
    }
}

/// Source-derived color targets must be created in their declared format on
/// both native backends. This intentionally stops before pass execution: it
/// proves that later source-plan admission cannot silently substitute a
/// nearby attachment format for a pack declaration.
#[test]
fn shared_shader_pack_color_target_formats_create_exact_private_resources() {
    let formats = [
        TextureFormat::R11fG11fB10f,
        TextureFormat::R32Float,
        TextureFormat::Rgb16Float,
        TextureFormat::Rgba8Unorm,
        TextureFormat::R8Unorm,
        TextureFormat::Rgba8Snorm,
        TextureFormat::Rgba16Float,
    ];
    for backend in [BackendKind::Vulkan, BackendKind::OpenGl] {
        let mut gal = match gal_for(backend, "MattMC source color target format conformance") {
            Ok(gal) => gal,
            Err(error) => {
                assert_environment_gap(&error, backend.name());
                continue;
            }
        };
        for format in formats {
            let texture = match gal.create_texture(TextureDesc {
                label: format!("{}.source-color.{format:?}.texture", backend.name()),
                dimension: TextureDimension::D2,
                format,
                extent: Extent3d {
                    width: 16,
                    height: 8,
                    depth: 1,
                },
                mip_levels: 1,
                array_layers: 1,
                usages: vec![
                    TextureUsage::ColorAttachment,
                    TextureUsage::Sampled,
                    TextureUsage::TransferSrc,
                ],
            }) {
                Ok(texture) => texture,
                Err(error) => {
                    assert_eq!(
                        StatusCode::UnsupportedFeature,
                        error.code,
                        "{} must report an unsupported exact shader-pack color format before source-plan admission, not fail later: {error}",
                        backend.name()
                    );
                    assert!(error.message.contains(&format!("{format:?}")));
                    continue;
                }
            };
            let texture_view = gal
                .create_texture_view(view(
                    &format!("{}.source-color.{format:?}.view", backend.name()),
                    texture,
                    format,
                ))
                .unwrap_or_else(|error| {
                    panic!(
                        "{} must create an exact shader-pack color view for {format:?}: {error}",
                        backend.name()
                    )
                });
            gal.destroy(texture_view).unwrap();
            gal.destroy(texture).unwrap();
        }
    }
}

#[test]
fn shared_large_copy_lifecycle_conformance_reuses_and_retires_resources() {
    for backend in [BackendKind::Vulkan, BackendKind::OpenGl] {
        let mut gal = match gal_for(backend, "MattMC large copy lifecycle conformance") {
            Ok(gal) => gal,
            Err(error) => {
                assert_environment_gap(&error, backend.name());
                continue;
            }
        };
        let src = gal
            .create_buffer(BufferDesc {
                label: "large-copy-src".to_owned(),
                size: 4096,
                memory: MemoryDomain::Upload,
                usages: vec![
                    BufferUsage::HostWrite,
                    BufferUsage::TransferSrc,
                    BufferUsage::TransferDst,
                ],
            })
            .unwrap();
        let dst = gal
            .create_buffer(BufferDesc {
                label: "large-copy-dst".to_owned(),
                size: 4096,
                memory: MemoryDomain::Readback,
                usages: vec![BufferUsage::TransferDst, BufferUsage::HostRead],
            })
            .unwrap();
        let mut lists = Vec::new();
        lists.push(
            gal.create_command_list(CommandListDesc {
                label: "large-copy-upload".to_owned(),
                operations: vec![
                    CommandOp::HostWriteBuffer {
                        buffer: src,
                        offset: 0,
                        data: vec![backend.seed(); 4096],
                    },
                    buffer_barrier(src),
                ],
            })
            .unwrap(),
        );
        for index in 0..24 {
            lists.push(
                gal.create_command_list(CommandListDesc {
                    label: format!("large-copy-{index}"),
                    operations: vec![
                        CommandOp::CopyBuffer {
                            src,
                            dst,
                            size: 256,
                        },
                        buffer_barrier(dst),
                    ],
                })
                .unwrap(),
            );
        }
        lists.push(
            gal.create_command_list(CommandListDesc {
                label: "large-copy-read".to_owned(),
                operations: vec![CommandOp::HostReadBuffer {
                    buffer: dst,
                    offset: 0,
                    size: 256,
                }],
            })
            .unwrap(),
        );
        let token = gal
            .submit(SubmissionBatch {
                label: format!("{}-large-copy-submit", backend.name()),
                command_lists: lists,
            })
            .unwrap();
        gal.destroy(src).unwrap();
        gal.destroy(dst).unwrap();
        let retired = gal.retire_through_for_test(token.submission).unwrap();
        assert_eq!(2, retired.len(), "{} should retire src/dst", backend.name());
    }
}

/// One semantic D3 fixture is deliberately shared by both private lowerings.
/// Native compute/storage-image execution stays in the backend conformance
/// modules, where shader compilation and descriptor details belong.
#[test]
fn shared_d3_pattern_fixture_round_trips_r8uint_and_rgba16float() {
    for backend in [BackendKind::Vulkan, BackendKind::OpenGl] {
        let mut gal = match gal_for(backend, "MattMC shared D3 pattern conformance") {
            Ok(gal) => gal,
            Err(error) => {
                assert_environment_gap(&error, backend.name());
                continue;
            }
        };
        assert!(gal.capabilities().supports(BackendFeature::Texture3d));
        submit_d3_pattern_round_trip(
            &mut gal,
            backend.name(),
            TextureFormat::R8Uint,
            Extent3d {
                width: 4,
                height: 2,
                depth: 2,
            },
            (0_u8..16).collect(),
        )
        .unwrap_or_else(|error| panic!("{} R8Uint D3 fixture failed: {error}", backend.name()));
        submit_d3_pattern_round_trip(
            &mut gal,
            backend.name(),
            TextureFormat::Rgba16Float,
            Extent3d {
                width: 2,
                height: 1,
                depth: 2,
            },
            (0_u8..32).collect(),
        )
        .unwrap_or_else(|error| {
            panic!("{} Rgba16Float D3 fixture failed: {error}", backend.name())
        });
    }
}

#[test]
fn shared_d3_pattern_fixture_round_trips_a_nonzero_mip_on_both_backends() {
    for backend in [BackendKind::Vulkan, BackendKind::OpenGl] {
        let mut gal = match gal_for(backend, "MattMC shared D3 mip conformance") {
            Ok(gal) => gal,
            Err(error) => {
                assert_environment_gap(&error, backend.name());
                continue;
            }
        };
        assert!(gal
            .capabilities()
            .supports(BackendFeature::TextureMipLevels));
        submit_d3_pattern_round_trip_at_mip(
            &mut gal,
            backend.name(),
            TextureFormat::R8Uint,
            Extent3d {
                width: 8,
                height: 4,
                depth: 2,
            },
            3,
            1,
            Extent3d {
                width: 4,
                height: 2,
                depth: 1,
            },
            (0x40_u8..0x48).collect(),
        )
        .unwrap_or_else(|error| panic!("{} D3 mip fixture failed: {error}", backend.name()));
    }
}

/// Exercises the normal sampled-texture resource path rather than only a
/// transfer round-trip. The selected texel is deliberately on the first Y
/// row, so an accidental reuse of the OpenGL 2D row flip cannot pass.
#[test]
fn shared_d3_pattern_fixture_samples_asymmetric_volume_coordinates() {
    for backend in [BackendKind::Vulkan, BackendKind::OpenGl] {
        let mut gal = match gal_for(backend, "MattMC shared D3 sampling conformance") {
            Ok(gal) => gal,
            Err(error) => {
                assert_environment_gap(&error, backend.name());
                continue;
            }
        };
        submit_d3_sampled_texel(&mut gal, backend.name()).unwrap_or_else(|error| {
            panic!(
                "{} sampled D3 coordinate fixture failed: {error}",
                backend.name()
            )
        });
    }
}

/// The two backends use their own native execution mechanisms (Vulkan compute
/// and an OpenGL storage-image draw), but this fixture makes the semantic
/// contract explicit: writing the same D3 coordinate then transitioning it to
/// transfer-read must yield identical bytes. Backend shader/module details stay
/// in the private backend conformance modules.
#[test]
fn shared_d3_storage_fixture_matches_required_volume_formats() {
    for format in [TextureFormat::R8Uint, TextureFormat::Rgba16Float] {
        let vulkan = super::vulkan::conformance::shared_d3_storage_fixture_bytes(format);
        let opengl = super::opengl::conformance::shared_d3_storage_fixture_bytes(format);
        match (vulkan, opengl) {
            (Some(vulkan), Some(opengl)) => assert_eq!(
                vulkan, opengl,
                "Vulkan and OpenGL D3 storage transitions must agree for {format:?}"
            ),
            (None, None) => continue,
            (None, Some(_)) => {
                // The Vulkan fixture already classified the missing native
                // environment. Do not turn an unavailable driver into a
                // false semantic conformance failure.
                continue;
            }
            (Some(_), None) => {
                // Ditto for a headless OpenGL environment.
                continue;
            }
        }
    }
}

fn submit_d3_pattern_round_trip(
    gal: &mut VulkanicGal,
    backend_name: &str,
    format: TextureFormat,
    extent: Extent3d,
    pattern: Vec<u8>,
) -> Result<(), GalError> {
    submit_d3_pattern_round_trip_at_mip(gal, backend_name, format, extent, 1, 0, extent, pattern)
}

fn submit_mip_generation_round_trip(
    gal: &mut VulkanicGal,
    backend_name: &str,
) -> Result<(), GalError> {
    let label = format!("shared-mip-generation-{}", backend_name.to_lowercase());
    let source_texel = [31_u8, 79, 163, 255];
    let source_pattern = source_texel.repeat(16);
    let upload = gal.create_buffer(BufferDesc {
        label: format!("{label}.upload"),
        size: source_pattern.len() as u64,
        memory: MemoryDomain::Upload,
        usages: vec![BufferUsage::HostWrite, BufferUsage::TransferSrc],
    })?;
    let readback = gal.create_buffer(BufferDesc {
        label: format!("{label}.readback"),
        size: 4,
        memory: MemoryDomain::Readback,
        usages: vec![BufferUsage::TransferDst, BufferUsage::HostRead],
    })?;
    let texture = gal.create_texture(TextureDesc {
        label: format!("{label}.texture"),
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8Unorm,
        extent: Extent3d {
            width: 4,
            height: 4,
            depth: 1,
        },
        mip_levels: 3,
        array_layers: 1,
        usages: vec![
            TextureUsage::TransferDst,
            TextureUsage::TransferSrc,
            TextureUsage::Sampled,
        ],
    })?;
    let descendants = TextureSubresourceRange {
        base_mip: 1,
        mip_count: 2,
        base_layer: 0,
        layer_count: 1,
    };
    let full_range = TextureSubresourceRange {
        base_mip: 0,
        mip_count: 3,
        base_layer: 0,
        layer_count: 1,
    };
    let base_upload = BufferImageCopyRegion {
        buffer: upload,
        buffer_offset: 0,
        bytes_per_row: 16,
        rows_per_image: 4,
        texture,
        texture_mip: 0,
        texture_layer: 0,
        texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
        extent: Extent3d {
            width: 4,
            height: 4,
            depth: 1,
        },
    };
    let final_read = BufferImageCopyRegion {
        buffer: readback,
        buffer_offset: 0,
        bytes_per_row: 4,
        rows_per_image: 1,
        texture,
        texture_mip: 2,
        texture_layer: 0,
        texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
        extent: Extent3d {
            width: 1,
            height: 1,
            depth: 1,
        },
    };
    let list = gal.create_command_list(CommandListDesc {
        label: format!("{label}.commands"),
        operations: vec![
            CommandOp::HostWriteBuffer {
                buffer: upload,
                offset: 0,
                data: source_pattern,
            },
            buffer_barrier(upload),
            texture_mip_barrier(
                texture,
                0,
                TextureUsageState::Undefined,
                TextureUsageState::TransferDst,
            ),
            CommandOp::CopyBufferToTexture(base_upload),
            texture_mip_barrier(
                texture,
                0,
                TextureUsageState::TransferDst,
                TextureUsageState::TransferSrc,
            ),
            CommandOp::Barrier(ResourceBarrier {
                resource: texture,
                subresources: Some(descendants),
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
            CommandOp::Barrier(ResourceBarrier {
                resource: texture,
                subresources: Some(TextureSubresourceRange {
                    base_mip: 2,
                    mip_count: 1,
                    base_layer: 0,
                    layer_count: 1,
                }),
                before: TextureUsageState::ShaderRead,
                after: TextureUsageState::TransferSrc,
                src_queue: QueueClass::Graphics,
                dst_queue: QueueClass::Graphics,
            }),
            CommandOp::CopyTextureToBuffer(final_read),
            buffer_barrier(readback),
            CommandOp::HostReadBuffer {
                buffer: readback,
                offset: 0,
                size: 4,
            },
        ],
    })?;
    let submission = gal.submit(SubmissionBatch {
        label: format!("{label}.submit"),
        command_lists: vec![list],
    })?;
    gal.destroy(texture)?;
    gal.destroy(upload)?;
    gal.destroy(readback)?;
    gal.retire_through_for_test(submission.submission)?;
    let read = gal
        .completed_host_reads()
        .into_iter()
        .find(|read| read.buffer == readback)
        .ok_or_else(|| GalError::backend("mip generation fixture completed without readback"))?;
    if read.bytes != source_texel {
        return Err(GalError::backend(format!(
            "mip generation fixture read {:?}, expected {:?}",
            read.bytes, source_texel
        )));
    }
    Ok(())
}

fn submit_d3_pattern_round_trip_at_mip(
    gal: &mut VulkanicGal,
    backend_name: &str,
    format: TextureFormat,
    texture_extent: Extent3d,
    mip_levels: u32,
    texture_mip: u32,
    extent: Extent3d,
    pattern: Vec<u8>,
) -> Result<(), GalError> {
    let bytes_per_texel = format
        .copy_bytes_per_texel()
        .ok_or_else(|| GalError::invalid_argument("D3 fixture format has no texel size"))?;
    let bytes_per_row = extent
        .width
        .checked_mul(bytes_per_texel)
        .ok_or_else(|| GalError::invalid_argument("D3 fixture row pitch overflows"))?;
    let expected_len = u64::from(bytes_per_row)
        .checked_mul(u64::from(extent.height))
        .and_then(|value| value.checked_mul(u64::from(extent.depth)))
        .ok_or_else(|| GalError::invalid_argument("D3 fixture byte length overflows"))?;
    if usize::try_from(expected_len).ok() != Some(pattern.len()) {
        return Err(GalError::invalid_argument(
            "D3 fixture pattern length does not match its semantic volume extent",
        ));
    }
    let label = format!("shared-d3-{}-{format:?}", backend_name.to_lowercase());
    let upload = gal.create_buffer(BufferDesc {
        label: format!("{label}.upload"),
        size: expected_len,
        memory: MemoryDomain::Upload,
        usages: vec![BufferUsage::HostWrite, BufferUsage::TransferSrc],
    })?;
    let readback = gal.create_buffer(BufferDesc {
        label: format!("{label}.readback"),
        size: expected_len,
        memory: MemoryDomain::Readback,
        usages: vec![BufferUsage::TransferDst, BufferUsage::HostRead],
    })?;
    let texture = gal.create_texture(TextureDesc {
        label: format!("{label}.texture"),
        dimension: TextureDimension::D3,
        format,
        extent: texture_extent,
        mip_levels,
        array_layers: 1,
        usages: vec![
            TextureUsage::TransferDst,
            TextureUsage::TransferSrc,
            TextureUsage::Sampled,
        ],
    })?;
    let region = BufferImageCopyRegion {
        buffer: upload,
        buffer_offset: 0,
        bytes_per_row,
        rows_per_image: extent.height,
        texture,
        texture_mip,
        texture_layer: 0,
        texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
        extent,
    };
    let list = gal.create_command_list(CommandListDesc {
        label: format!("{label}.commands"),
        operations: vec![
            CommandOp::HostWriteBuffer {
                buffer: upload,
                offset: 0,
                data: pattern.clone(),
            },
            buffer_barrier(upload),
            texture_mip_barrier(
                texture,
                texture_mip,
                TextureUsageState::Undefined,
                TextureUsageState::TransferDst,
            ),
            CommandOp::CopyBufferToTexture(region.clone()),
            texture_mip_barrier(
                texture,
                texture_mip,
                TextureUsageState::TransferDst,
                TextureUsageState::TransferSrc,
            ),
            CommandOp::CopyTextureToBuffer(BufferImageCopyRegion {
                buffer: readback,
                ..region
            }),
            buffer_barrier(readback),
            CommandOp::HostReadBuffer {
                buffer: readback,
                offset: 0,
                size: expected_len,
            },
        ],
    })?;
    let token = gal.submit(SubmissionBatch {
        label: format!("{label}.submit"),
        command_lists: vec![list],
    })?;
    gal.destroy(texture)?;
    gal.destroy(upload)?;
    gal.destroy(readback)?;
    gal.retire_through_for_test(token.submission)?;
    let read = gal
        .completed_host_reads()
        .into_iter()
        .find(|read| read.buffer == readback)
        .ok_or_else(|| GalError::backend("D3 fixture completed without its readback"))?;
    if read.bytes != pattern {
        return Err(GalError::backend(
            "D3 fixture readback did not match its upload pattern",
        ));
    }
    Ok(())
}

fn submit_d3_sampled_texel(gal: &mut VulkanicGal, backend_name: &str) -> Result<(), GalError> {
    const VOLUME_EXTENT: Extent3d = Extent3d {
        width: 4,
        height: 3,
        depth: 2,
    };
    const SAMPLE_X: u32 = 2;
    const SAMPLE_Y: u32 = 0;
    const SAMPLE_Z: u32 = 1;
    const SAMPLE_VALUE: u8 = 0x5a;

    let mut volume_bytes =
        vec![0_u8; (VOLUME_EXTENT.width * VOLUME_EXTENT.height * VOLUME_EXTENT.depth) as usize];
    let sample_offset = (SAMPLE_Z * VOLUME_EXTENT.width * VOLUME_EXTENT.height
        + SAMPLE_Y * VOLUME_EXTENT.width
        + SAMPLE_X) as usize;
    volume_bytes[sample_offset] = SAMPLE_VALUE;
    // A distinct last row catches a stale D2-style upload flip.
    volume_bytes[(SAMPLE_Z * VOLUME_EXTENT.width * VOLUME_EXTENT.height
        + (VOLUME_EXTENT.height - 1) * VOLUME_EXTENT.width
        + SAMPLE_X) as usize] = 0xc3;

    let label = format!("shared-d3-sampled-{}", backend_name.to_lowercase());
    let upload = gal.create_buffer(BufferDesc {
        label: format!("{label}.upload"),
        size: volume_bytes.len() as u64,
        memory: MemoryDomain::Upload,
        usages: vec![BufferUsage::HostWrite, BufferUsage::TransferSrc],
    })?;
    let readback = gal.create_buffer(BufferDesc {
        label: format!("{label}.readback"),
        size: 4,
        memory: MemoryDomain::Readback,
        usages: vec![BufferUsage::TransferDst, BufferUsage::HostRead],
    })?;
    let volume = gal.create_texture(TextureDesc {
        label: format!("{label}.volume"),
        dimension: TextureDimension::D3,
        format: TextureFormat::R8Uint,
        extent: VOLUME_EXTENT,
        mip_levels: 1,
        array_layers: 1,
        usages: vec![TextureUsage::TransferDst, TextureUsage::Sampled],
    })?;
    let color = gal.create_texture(TextureDesc {
        label: format!("{label}.color"),
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8Unorm,
        extent: Extent3d {
            width: 1,
            height: 1,
            depth: 1,
        },
        mip_levels: 1,
        array_layers: 1,
        usages: vec![TextureUsage::ColorAttachment, TextureUsage::TransferSrc],
    })?;
    let volume_view = gal.create_texture_view(view(
        &format!("{label}.volume.view"),
        volume,
        TextureFormat::R8Uint,
    ))?;
    let color_view = gal.create_texture_view(view(
        &format!("{label}.color.view"),
        color,
        TextureFormat::Rgba8Unorm,
    ))?;
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
    let target = gal.create_render_target(RenderTargetDesc {
        label: format!("{label}.target"),
        color_views: vec![color_view],
        depth_stencil_view: None,
        extent: Extent3d {
            width: 1,
            height: 1,
            depth: 1,
        },
    })?;
    let pass = gal.create_render_pass(RenderPassDesc {
        label: format!("{label}.pass"),
        target,
        color_formats: vec![TextureFormat::Rgba8Unorm],
        depth_format: None,
    })?;
    let empty_layout = gal.create_resource_layout(ResourceLayoutDesc {
        label: format!("{label}.empty-layout"),
        bindings: Vec::new(),
    })?;
    let sampled_layout = gal.create_resource_layout(ResourceLayoutDesc {
        label: format!("{label}.sampled-layout"),
        bindings: vec![
            ResourceBindingDesc {
                binding: 0,
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
    let sampled_set = gal.create_resource_set(ResourceSetDesc {
        label: format!("{label}.sampled-set"),
        layout: sampled_layout,
        bindings: vec![
            ResourceBinding {
                binding: 0,
                array_index: 0,
                resource: volume_view,
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
    let pipeline_layout = gal.create_pipeline_layout(PipelineLayoutDesc {
        label: format!("{label}.pipeline-layout"),
        resource_layouts: vec![empty_layout, sampled_layout],
    })?;
    let vertex = gal.create_shader_module(ShaderModuleDesc {
        label: format!("{label}.vertex"),
        stage: ShaderStage::Vertex,
        code_format: ShaderCodeFormat::Glsl,
        code: D3_SAMPLED_VERTEX_SHADER.as_bytes().to_vec(),
        entry_point: "main".to_owned(),
    })?;
    let fragment = gal.create_shader_module(ShaderModuleDesc {
        label: format!("{label}.fragment"),
        stage: ShaderStage::Fragment,
        code_format: ShaderCodeFormat::Glsl,
        code: D3_SAMPLED_FRAGMENT_SHADER.as_bytes().to_vec(),
        entry_point: "main".to_owned(),
    })?;
    let pipeline = gal.create_graphics_pipeline(GraphicsPipelineDesc {
        label: format!("{label}.pipeline"),
        layout: pipeline_layout,
        vertex_shader: vertex,
        fragment_shader: fragment,
        topology: PrimitiveTopology::Triangles,
        cull_mode: CullMode::None,
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
    })?;
    let upload_region = BufferImageCopyRegion {
        buffer: upload,
        buffer_offset: 0,
        bytes_per_row: VOLUME_EXTENT.width,
        rows_per_image: VOLUME_EXTENT.height,
        texture: volume,
        texture_mip: 0,
        texture_layer: 0,
        texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
        extent: VOLUME_EXTENT,
    };
    let list = gal.create_command_list(CommandListDesc {
        label: format!("{label}.commands"),
        operations: vec![
            CommandOp::HostWriteBuffer {
                buffer: upload,
                offset: 0,
                data: volume_bytes,
            },
            buffer_barrier(upload),
            texture_barrier(
                volume,
                TextureUsageState::Undefined,
                TextureUsageState::TransferDst,
            ),
            CommandOp::CopyBufferToTexture(upload_region),
            texture_barrier(
                volume,
                TextureUsageState::TransferDst,
                TextureUsageState::ShaderRead,
            ),
            texture_barrier(
                color,
                TextureUsageState::Undefined,
                TextureUsageState::ColorAttachment,
            ),
            CommandOp::BeginPass {
                pass,
                target,
                colors: vec![color_attachment(color_view, 0.0, 0.0, 0.0, 1.0)],
                depth_stencil: None,
            },
            CommandOp::BindGraphicsPipeline(pipeline),
            CommandOp::BindResourceSet {
                pipeline_layout,
                set_index: 1,
                set: sampled_set,
                dynamic_offsets: Vec::new(),
            },
            CommandOp::Draw {
                vertices: 3,
                instances: 1,
            },
            CommandOp::EndPass,
            texture_barrier(
                color,
                TextureUsageState::ColorAttachment,
                TextureUsageState::TransferSrc,
            ),
            CommandOp::CopyTextureToBuffer(BufferImageCopyRegion {
                buffer: readback,
                buffer_offset: 0,
                bytes_per_row: 4,
                rows_per_image: 1,
                texture: color,
                texture_mip: 0,
                texture_layer: 0,
                texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
                extent: Extent3d {
                    width: 1,
                    height: 1,
                    depth: 1,
                },
            }),
            buffer_barrier(readback),
            CommandOp::HostReadBuffer {
                buffer: readback,
                offset: 0,
                size: 4,
            },
        ],
    })?;
    let token = gal.submit(SubmissionBatch {
        label: format!("{label}.submit"),
        command_lists: vec![list],
    })?;
    gal.retire_through_for_test(token.submission)?;
    let bytes = gal
        .completed_host_reads()
        .into_iter()
        .find(|read| read.buffer == readback)
        .ok_or_else(|| GalError::backend("sampled D3 fixture completed without color readback"))?
        .bytes;
    if bytes != [SAMPLE_VALUE, 0, 0, 255] {
        return Err(GalError::backend(format!(
            "sampled D3 texel did not preserve semantic coordinate: expected [90, 0, 0, 255], got {bytes:?}"
        )));
    }
    Ok(())
}

fn assert_environment_gap(error: &GalError, backend: &str) {
    let text = error.to_string();
    assert!(
        text.contains(backend)
            || text.contains("physical device")
            || text.contains("EGL")
            || text.contains("GL"),
        "unexpected cross-backend conformance failure: {text}"
    );
}

const D3_SAMPLED_VERTEX_SHADER: &str = r#"
#version 450
vec2 positions[3] = vec2[](
    vec2(-1.0, -1.0),
    vec2(3.0, -1.0),
    vec2(-1.0, 3.0)
);
void main() {
    gl_Position = vec4(positions[gl_VertexIndex], 0.0, 1.0);
}
"#;

const D3_SAMPLED_FRAGMENT_SHADER: &str = r#"
#version 450
layout(set = 1, binding = 0) uniform utexture3D TerrainVoxelOccupancy;
layout(set = 1, binding = 2) uniform sampler TerrainVoxelLightSampler;
layout(location = 0) out vec4 out_color;
void main() {
    uint value = texelFetch(
        usampler3D(TerrainVoxelOccupancy, TerrainVoxelLightSampler),
        ivec3(2, 0, 1),
        0
    ).r;
    out_color = vec4(float(value) / 255.0, 0.0, 0.0, 1.0);
}
"#;

#[derive(Clone, Copy)]
enum BackendKind {
    Vulkan,
    OpenGl,
}

impl BackendKind {
    fn name(self) -> &'static str {
        match self {
            Self::Vulkan => "Vulkan",
            Self::OpenGl => "OpenGL",
        }
    }

    fn seed(self) -> u8 {
        match self {
            Self::Vulkan => 0x56,
            Self::OpenGl => 0x47,
        }
    }
}

fn gal_for(backend: BackendKind, label: &str) -> Result<VulkanicGal, GalError> {
    match backend {
        BackendKind::Vulkan => Ok(VulkanicGal::new_with_backend(
            Box::new(super::vulkan::VulkanBackend::new(label)?),
            false,
        )),
        BackendKind::OpenGl => Ok(VulkanicGal::new_with_backend(
            Box::new(super::opengl::OpenGlBackend::new(label)?),
            false,
        )),
    }
}

fn submit_mrt_clear(gal: &mut VulkanicGal) -> Result<(), GalError> {
    let extent = extent();
    let color_a = color_texture(gal, "mrt.color.a", extent)?;
    let color_b = color_texture(gal, "mrt.color.b", extent)?;
    let view_a =
        gal.create_texture_view(view("mrt.color.a.view", color_a, TextureFormat::Rgba8Unorm))?;
    let view_b =
        gal.create_texture_view(view("mrt.color.b.view", color_b, TextureFormat::Rgba8Unorm))?;
    let target = gal.create_render_target(RenderTargetDesc {
        label: "mrt.target".to_owned(),
        color_views: vec![view_a, view_b],
        depth_stencil_view: None,
        extent,
    })?;
    let pass = gal.create_render_pass(RenderPassDesc {
        label: "mrt.pass".to_owned(),
        target,
        color_formats: vec![TextureFormat::Rgba8Unorm, TextureFormat::Rgba8Unorm],
        depth_format: None,
    })?;
    let list = gal.create_command_list(CommandListDesc {
        label: "mrt.clear".to_owned(),
        operations: vec![
            texture_barrier(
                color_a,
                TextureUsageState::Undefined,
                TextureUsageState::ColorAttachment,
            ),
            texture_barrier(
                color_b,
                TextureUsageState::Undefined,
                TextureUsageState::ColorAttachment,
            ),
            CommandOp::BeginPass {
                pass,
                target,
                colors: vec![
                    color_attachment(view_a, 0.25, 0.0, 0.0, 1.0),
                    color_attachment(view_b, 0.0, 0.25, 0.0, 1.0),
                ],
                depth_stencil: None,
            },
            CommandOp::EndPass,
        ],
    })?;
    let token = gal.submit(SubmissionBatch {
        label: "mrt.submit".to_owned(),
        command_lists: vec![list],
    })?;
    gal.retire_through_for_test(token.submission)?;
    Ok(())
}

fn submit_depth_only_clear(gal: &mut VulkanicGal) -> Result<(), GalError> {
    let extent = extent();
    let depth = gal.create_texture(TextureDesc {
        label: "depth-only.depth".to_owned(),
        dimension: TextureDimension::D2,
        format: TextureFormat::Depth32Float,
        extent,
        mip_levels: 1,
        array_layers: 1,
        usages: vec![TextureUsage::DepthStencilAttachment],
    })?;
    let depth_view =
        gal.create_texture_view(view("depth-only.view", depth, TextureFormat::Depth32Float))?;
    let target = gal.create_render_target(RenderTargetDesc {
        label: "depth-only.target".to_owned(),
        color_views: vec![],
        depth_stencil_view: Some(depth_view),
        extent,
    })?;
    let pass = gal.create_render_pass(RenderPassDesc {
        label: "depth-only.pass".to_owned(),
        target,
        color_formats: vec![],
        depth_format: Some(TextureFormat::Depth32Float),
    })?;
    let list = gal.create_command_list(CommandListDesc {
        label: "depth-only.clear".to_owned(),
        operations: vec![
            texture_barrier(
                depth,
                TextureUsageState::Undefined,
                TextureUsageState::DepthStencilAttachment,
            ),
            CommandOp::BeginPass {
                pass,
                target,
                colors: vec![],
                depth_stencil: Some(PassAttachment {
                    view: depth_view,
                    load_op: AttachmentLoadOp::Clear,
                    store_op: AttachmentStoreOp::DontCare,
                    clear_color: None,
                }),
            },
            CommandOp::EndPass,
        ],
    })?;
    let token = gal.submit(SubmissionBatch {
        label: "depth-only.submit".to_owned(),
        command_lists: vec![list],
    })?;
    gal.retire_through_for_test(token.submission)?;
    Ok(())
}

fn color_texture(gal: &mut VulkanicGal, label: &str, extent: Extent3d) -> Result<Handle, GalError> {
    gal.create_texture(TextureDesc {
        label: label.to_owned(),
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8Unorm,
        extent,
        mip_levels: 1,
        array_layers: 1,
        usages: vec![TextureUsage::ColorAttachment],
    })
}

fn extent() -> Extent3d {
    Extent3d {
        width: 32,
        height: 32,
        depth: 1,
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

fn color_attachment(view: Handle, r: f32, g: f32, b: f32, a: f32) -> PassAttachment {
    PassAttachment {
        view,
        load_op: AttachmentLoadOp::Clear,
        store_op: AttachmentStoreOp::Store,
        clear_color: Some(ClearColor { r, g, b, a }),
    }
}

fn texture_barrier(
    resource: Handle,
    before: TextureUsageState,
    after: TextureUsageState,
) -> CommandOp {
    texture_mip_barrier(resource, 0, before, after)
}

fn texture_mip_barrier(
    resource: Handle,
    mip: u32,
    before: TextureUsageState,
    after: TextureUsageState,
) -> CommandOp {
    CommandOp::Barrier(ResourceBarrier {
        resource,
        subresources: Some(TextureSubresourceRange {
            base_mip: mip,
            mip_count: 1,
            base_layer: 0,
            layer_count: 1,
        }),
        before,
        after,
        src_queue: QueueClass::Graphics,
        dst_queue: QueueClass::Graphics,
    })
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
