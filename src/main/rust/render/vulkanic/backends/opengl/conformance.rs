use std::path::PathBuf;

use xxhash_rust::xxh32::xxh32;

use super::{OpenGlBackend, OpenGlSyncStats, StateCacheSnapshot};
use crate::render::vulkanic::commands::{
    AttachmentLoadOp, AttachmentStoreOp, BufferImageCopyRegion, ClearColor, CommandListDesc,
    CommandOp, PassAttachment, ResourceBarrier, SubmissionBatch, TextureOrigin3d,
    TextureUsageState,
};
use crate::render::vulkanic::error::{GalError, GalResult};
use crate::render::vulkanic::gal::VulkanicGal;
use crate::render::vulkanic::handles::Handle;
use crate::render::vulkanic::resources::*;
use crate::render::vulkanic::shader_pack::programs::{
    distant_horizons_lod_opaque_resource_layouts,
    minimal_distant_horizons_lod_exact_atlas_opaque_program,
    minimal_distant_horizons_lod_opaque_program, minimal_distant_horizons_lod_transparent_program,
    prepare_lowered_distant_horizons_exact_atlas_source_program,
    prepare_lowered_distant_horizons_source_program, prepare_lowered_fullscreen_source_program,
    prepare_lowered_hand_source_program, prepare_lowered_terrain_source_program,
    prepare_lowered_textured_material_source_program, prepare_lowered_weather_source_program,
    TerrainMaterialProgramKind, COMPLEMENTARY_TERRAIN_SUBSET_FRAGMENT,
    MINIMAL_TERRAIN_MATERIAL_VERTEX,
};
use crate::render::vulkanic::shader_pack::{
    distant_horizons_contract::{
        derive_distant_horizons_opaque_contract, derive_distant_horizons_translucent_contract,
    },
    hand_contract::{bind_hand_source_resources, derive_hand_contract, lower_hand_source_pair},
    lowering::{
        lower_distant_horizons_source_pair, lower_fullscreen_source_pair, lower_terrain_source_pair,
    },
    material_contract::{derive_textured_material_contract, lower_textured_material_source_pair},
    preprocess::{
        complete_bundled_pack_source_for_test, preprocess_distant_horizons_sources,
        preprocess_source_stage_pair, preprocess_terrain_sources,
    },
    source::{ShaderPackSource, ShaderSourceFile, RUNTIME_OPTIONS_PATH},
    terrain_contract::{
        derive_complementary_terrain_contract, TerrainProgramScope, TerrainSourceStage,
        TerrainSourceStages,
    },
    terrain_source_resources::{TerrainSourceResourceBindings, TERRAIN_RESOURCE_BINDINGS_PATH},
    weather_contract::{derive_weather_pass_contract, lower_weather_source_pair},
};

const WIDTH: u32 = 96;
const HEIGHT: u32 = 64;

#[test]
fn gui_panorama_material_compiles_through_the_opengl_lowering() {
    let backend = match OpenGlBackend::new("MattMC GUI panorama OpenGL conformance") {
        Ok(backend) => backend,
        Err(error) => {
            let text = error.to_string();
            assert!(
                text.contains("OpenGL") || text.contains("EGL") || text.contains("GL"),
                "unexpected OpenGL panorama setup failure: {text}"
            );
            return;
        }
    };
    let mut gal = VulkanicGal::new_with_backend(Box::new(backend), false);
    let (vertex, fragment) =
        crate::render::vulkanic::gui_mesh_frontend::opengl_panorama_shader_sources_for_backend_test(
        );
    for (stage, code, label) in [
        (ShaderStage::Vertex, vertex, "gui-panorama.vertex"),
        (ShaderStage::Fragment, fragment, "gui-panorama.fragment"),
    ] {
        gal.create_shader_module(ShaderModuleDesc {
            label: label.to_string(),
            stage,
            code_format: ShaderCodeFormat::Glsl,
            code: code.as_bytes().to_vec(),
            entry_point: "main".to_string(),
        })
        .unwrap_or_else(|error| {
            panic!("Rust-owned GUI panorama shader must compile through OpenGL lowering: {error}")
        });
    }
}

#[test]
fn distant_horizons_lod_opaque_program_compiles_at_the_opengl_boundary() {
    let backend = match OpenGlBackend::new("MattMC Distant Horizons LOD OpenGL conformance") {
        Ok(backend) => backend,
        Err(error) => {
            let text = error.to_string();
            assert!(
                text.contains("OpenGL") || text.contains("EGL") || text.contains("GL"),
                "unexpected OpenGL DH LOD setup failure: {text}"
            );
            return;
        }
    };
    let mut gal = VulkanicGal::new_with_backend(Box::new(backend), false);
    let program = minimal_distant_horizons_lod_opaque_program();
    for module in program.shader_module_descriptors(BackendApi::OpenGl) {
        gal.create_shader_module(module).unwrap_or_else(|error| {
            panic!(
                "Rust-owned Distant Horizons opaque LOD shader must compile through OpenGL lowering: {error}"
            )
        });
    }
}

#[test]
fn distant_horizons_lod_exact_atlas_program_compiles_at_the_opengl_boundary() {
    let backend = match OpenGlBackend::new("MattMC Distant Horizons exact-atlas OpenGL conformance")
    {
        Ok(backend) => backend,
        Err(error) => {
            let text = error.to_string();
            assert!(
                text.contains("OpenGL") || text.contains("EGL") || text.contains("GL"),
                "unexpected OpenGL exact-atlas DH LOD setup failure: {text}"
            );
            return;
        }
    };
    let mut gal = VulkanicGal::new_with_backend(Box::new(backend), false);
    let program = minimal_distant_horizons_lod_exact_atlas_opaque_program();
    for module in program.shader_module_descriptors(BackendApi::OpenGl) {
        gal.create_shader_module(module).unwrap_or_else(|error| {
            panic!(
                "Rust-owned Distant Horizons exact-atlas LOD shader must compile through OpenGL lowering: {error}"
            )
        });
    }
}

#[test]
fn selected_source_exact_atlas_distant_horizons_program_compiles_at_the_opengl_boundary() {
    let backend =
        match OpenGlBackend::new("MattMC selected-source exact-atlas DH OpenGL conformance") {
            Ok(backend) => backend,
            Err(error) => {
                let text = error.to_string();
                assert!(
                    text.contains("OpenGL") || text.contains("EGL") || text.contains("GL"),
                    "unexpected OpenGL selected-source exact-atlas DH setup failure: {text}"
                );
                return;
            }
        };
    let mut gal = VulkanicGal::new_with_backend(Box::new(backend), false);
    let source = complete_bundled_pack_source_for_test();
    let contract =
        derive_distant_horizons_opaque_contract(&source, TerrainProgramScope::Overworld).unwrap();
    let artifacts = preprocess_distant_horizons_sources(&source, &contract.source_stages).unwrap();
    let lowered =
        lower_distant_horizons_source_pair(&artifacts.vertex, &artifacts.fragment).unwrap();
    let declarations = TerrainSourceResourceBindings::from_source(&source).unwrap();
    let bindings = lowered
        .opaque_resource_contract()
        .bind_semantic_roles(&declarations)
        .unwrap();
    let source_program =
        prepare_lowered_distant_horizons_source_program(&contract, &lowered, &bindings).unwrap();
    let program =
        prepare_lowered_distant_horizons_exact_atlas_source_program(&source_program).unwrap();

    for module in program.shader_module_descriptors(BackendApi::OpenGl) {
        gal.create_shader_module(module).unwrap_or_else(|error| {
            panic!(
                "selected-source exact-atlas Distant Horizons shader must compile through OpenGL lowering: {error}"
            )
        });
    }
}

#[test]
fn distant_horizons_lod_transparent_program_compiles_at_the_opengl_boundary() {
    let backend =
        match OpenGlBackend::new("MattMC Distant Horizons transparent LOD OpenGL conformance") {
            Ok(backend) => backend,
            Err(error) => {
                let text = error.to_string();
                assert!(
                    text.contains("OpenGL") || text.contains("EGL") || text.contains("GL"),
                    "unexpected OpenGL transparent DH LOD setup failure: {text}"
                );
                return;
            }
        };
    let mut gal = VulkanicGal::new_with_backend(Box::new(backend), false);
    let program = minimal_distant_horizons_lod_transparent_program();
    for module in program.shader_module_descriptors(BackendApi::OpenGl) {
        gal.create_shader_module(module).unwrap_or_else(|error| {
            panic!(
                "Rust-owned Distant Horizons transparent LOD shader must compile through OpenGL lowering: {error}"
            )
        });
    }
}

#[test]
fn lowered_fullscreen_source_compiles_at_the_opengl_boundary_without_a_vertex_stream() {
    let backend = match OpenGlBackend::new("MattMC fullscreen source OpenGL conformance") {
        Ok(backend) => backend,
        Err(error) => {
            let text = error.to_string();
            assert!(
                text.contains("OpenGL") || text.contains("EGL") || text.contains("GL"),
                "unexpected OpenGL fullscreen source setup failure: {text}"
            );
            return;
        }
    };
    let mut gal = VulkanicGal::new_with_backend(Box::new(backend), false);
    let source = ShaderPackSource::new(
        "fullscreen-source-opengl",
        1,
        vec![
            ShaderSourceFile::new(
                "world0/deferred1.vsh",
                "#version 130\nout vec2 uv;\nvoid main() { uv = gl_MultiTexCoord0.xy; gl_Position = ftransform(); }",
            ),
                ShaderSourceFile::new(
                    "world0/deferred1.fsh",
                    "#version 130\nin vec2 uv;\nuniform sampler2D colortex0;\n/* DRAWBUFFERS:0 */\nvoid main() { gl_FragData[0] = texture2D(colortex0, uv); }",
            ),
            ShaderSourceFile::new(
                TERRAIN_RESOURCE_BINDINGS_PATH,
                "colortex0=shader_pack_color:primary\n",
            ),
        ],
    )
    .unwrap();
    let stages = TerrainSourceStages {
        vertex: TerrainSourceStage {
            path: "world0/deferred1.vsh".to_string(),
            defines: Default::default(),
        },
        fragment: TerrainSourceStage {
            path: "world0/deferred1.fsh".to_string(),
            defines: Default::default(),
        },
    };
    let artifacts = preprocess_source_stage_pair(&source, &stages).unwrap();
    let declarations = TerrainSourceResourceBindings::from_source(&source).unwrap();
    let lowered =
        lower_fullscreen_source_pair(&artifacts.vertex, &artifacts.fragment, &declarations)
            .unwrap();
    let bindings = lowered
        .opaque_resource_contract()
        .bind_semantic_roles(&declarations)
        .unwrap();
    let program = prepare_lowered_fullscreen_source_program(
        source.name(),
        source.generation(),
        "world0/deferred1.fsh",
        &lowered,
        &bindings,
    )
    .unwrap();
    for module in program.shader_module_descriptors(BackendApi::OpenGl) {
        gal.create_shader_module(module).unwrap_or_else(|error| {
            panic!(
                "lowered fullscreen source must compile through OpenGL without a vertex stream: {error}"
            )
        });
    }
}

#[test]
fn lowered_complete_complementary_distant_horizons_pair_compiles_at_the_opengl_boundary() {
    let backend = match OpenGlBackend::new("MattMC complete source DH OpenGL conformance") {
        Ok(backend) => backend,
        Err(error) => {
            let text = error.to_string();
            assert!(
                text.contains("OpenGL") || text.contains("EGL") || text.contains("GL"),
                "unexpected OpenGL complete source DH setup failure: {text}"
            );
            return;
        }
    };
    let mut gal = VulkanicGal::new_with_backend(Box::new(backend), false);
    let source = complete_bundled_pack_source_for_test();
    let contract = derive_distant_horizons_opaque_contract(&source, TerrainProgramScope::Overworld)
        .expect("the bundled Complementary source must expose an Overworld DH pair");
    let artifacts = preprocess_distant_horizons_sources(&source, &contract.source_stages).unwrap();
    let lowered =
        lower_distant_horizons_source_pair(&artifacts.vertex, &artifacts.fragment).unwrap();
    let declarations = TerrainSourceResourceBindings::from_source(&source).unwrap();
    let bindings = lowered
        .opaque_resource_contract()
        .bind_semantic_roles(&declarations)
        .unwrap();
    let program =
        prepare_lowered_distant_horizons_source_program(&contract, &lowered, &bindings).unwrap();

    for module in program.shader_module_descriptors(BackendApi::OpenGl) {
        gal.create_shader_module(module).unwrap_or_else(|error| {
            panic!(
                "complete lowered Complementary Distant Horizons shader must compile through OpenGL lowering: {error}"
            )
        });
    }
}

/// The private OpenGL lowering must compile the same source-derived
/// `dh_water` ABI as Vulkan. This remains conformance only, not a borrowed
/// Iris path or a Java compatibility draw.
#[test]
fn lowered_complete_complementary_distant_horizons_water_pair_compiles_at_the_opengl_boundary() {
    let backend = match OpenGlBackend::new("MattMC complete source DH water OpenGL conformance") {
        Ok(backend) => backend,
        Err(error) => {
            let text = error.to_string();
            assert!(
                text.contains("OpenGL") || text.contains("EGL") || text.contains("GL"),
                "unexpected OpenGL complete source DH water setup failure: {text}"
            );
            return;
        }
    };
    let mut gal = VulkanicGal::new_with_backend(Box::new(backend), false);
    let bundled = complete_bundled_pack_source_for_test();
    let source = ShaderPackSource::new(
        "complete-dh-water-opengl-boundary",
        93,
        bundled
            .files()
            .into_iter()
            .chain(std::iter::once(ShaderSourceFile::new(
                RUNTIME_OPTIONS_PATH,
                "DISTANT_HORIZONS=1\nSHADOW_QUALITY=-1\nFXAA_DEFINE=-1\nCOLORED_LIGHTING=0\nENTITY_SHADOWS_DEFINE=-1\nPLAYER_SHADOW=-1\nRAIN_PUDDLES=0\n",
            )))
            .collect(),
    )
    .unwrap();
    let contract =
        derive_distant_horizons_translucent_contract(&source, TerrainProgramScope::Overworld)
            .expect("the bundled Complementary source must expose an Overworld dh_water pair");
    let artifacts = preprocess_distant_horizons_sources(&source, &contract.source_stages).unwrap();
    let lowered =
        lower_distant_horizons_source_pair(&artifacts.vertex, &artifacts.fragment).unwrap();
    let declarations = TerrainSourceResourceBindings::from_source(&source).unwrap();
    let bindings = lowered
        .opaque_resource_contract()
        .bind_semantic_roles(&declarations)
        .unwrap();
    let program =
        prepare_lowered_distant_horizons_source_program(&contract, &lowered, &bindings).unwrap();

    for module in program.shader_module_descriptors(BackendApi::OpenGl) {
        gal.create_shader_module(module).unwrap_or_else(|error| {
            panic!(
                "complete lowered Complementary Distant Horizons water shader must compile through OpenGL lowering: {error}"
            )
        });
    }
}

#[test]
fn distant_horizons_lod_opaque_pipeline_uses_explicit_two_set_gal_layout() {
    let backend =
        match OpenGlBackend::new("MattMC Distant Horizons LOD OpenGL pipeline conformance") {
            Ok(backend) => backend,
            Err(error) => {
                let text = error.to_string();
                assert!(
                    text.contains("OpenGL") || text.contains("EGL") || text.contains("GL"),
                    "unexpected OpenGL DH LOD setup failure: {text}"
                );
                return;
            }
        };
    let mut gal = VulkanicGal::new_with_backend(Box::new(backend), false);
    let program = minimal_distant_horizons_lod_opaque_program();
    let [geometry_and_frame, lightmap] =
        distant_horizons_lod_opaque_resource_layouts("dh-lod.opengl");
    let geometry_and_frame = gal.create_resource_layout(geometry_and_frame).unwrap();
    let lightmap = gal.create_resource_layout(lightmap).unwrap();
    let pipeline_layout = gal
        .create_pipeline_layout(PipelineLayoutDesc {
            label: "dh-lod.opengl.pipeline-layout".to_string(),
            resource_layouts: vec![geometry_and_frame, lightmap],
        })
        .unwrap();
    let [vertex, fragment] = program.shader_module_descriptors(BackendApi::OpenGl);
    let vertex_shader = gal.create_shader_module(vertex).unwrap();
    let fragment_shader = gal.create_shader_module(fragment).unwrap();
    gal.create_graphics_pipeline(GraphicsPipelineDesc {
        label: "dh-lod.opengl.pipeline".to_string(),
        layout: pipeline_layout,
        vertex_shader,
        fragment_shader,
        topology: PrimitiveTopology::Triangles,
        cull_mode: CullMode::Back,
        front_face: crate::render::vulkanic::resources::FrontFace::CounterClockwise,
        provoking_vertex: crate::render::vulkanic::resources::ProvokingVertex::Last,
        raster_y_direction: crate::render::vulkanic::resources::RasterYDirection::Up,
        blend: BlendMode::Disabled,
        depth_compare: Some(CompareOp::LessOrEqual),
        depth_write: true,
        depth_bias: None,
        color_formats: vec![TextureFormat::Rgba8Unorm; 4],
        depth_format: Some(TextureFormat::Depth32Float),
        stencil: None,
    })
    .expect("Rust-owned DH LOD pipeline must lower through OpenGL");
}

/// The source-derived terrain ABI must compile through the private OpenGL
/// lowering too. This remains isolated conformance only: it neither selects
/// a gameplay route nor borrows a shader-pack program from Java.
#[test]
fn prepared_lowered_terrain_program_compiles_at_the_opengl_boundary() {
    let backend = match OpenGlBackend::new("MattMC prepared source terrain OpenGL conformance") {
        Ok(backend) => backend,
        Err(error) => {
            let text = error.to_string();
            assert!(
                text.contains("OpenGL") || text.contains("EGL") || text.contains("GL"),
                "unexpected OpenGL source-terrain setup failure: {text}"
            );
            return;
        }
    };
    let mut gal = VulkanicGal::new_with_backend(Box::new(backend), false);
    let source = ShaderPackSource::new(
        "prepared-source-opengl-boundary",
        37,
        vec![
            ShaderSourceFile::new(
                "gbuffers_terrain.vsh",
                "#version 130\nout vec2 texCoord;\nout vec4 glColor;\nout float smoothnessD;\nout float materialMask;\nout float skyLightFactor;\nuniform sampler2D tex;\nuniform mat4 gbufferModelView;\nuniform mat4 gbufferProjection;\nvoid main() { texCoord = gl_MultiTexCoord0.xy; glColor = vec4(1.0); smoothnessD = 0.0; materialMask = 0.0; skyLightFactor = 1.0; gl_Position = ftransform(); }",
            ),
            ShaderSourceFile::new(
                "gbuffers_terrain.fsh",
                "#version 130\nin vec2 texCoord;\nin vec4 glColor;\nin float smoothnessD;\nin float materialMask;\nin float skyLightFactor;\nuniform sampler2D tex;\nvoid DoLighting() {}\n/* DRAWBUFFERS:06 */\nvoid main() { vec4 color = texture2D(tex, texCoord); if (color.a <= 0.00001) discard; color.rgb *= glColor.rgb; DoLighting(); gl_FragData[0] = color; gl_FragData[1] = vec4(smoothnessD, materialMask, skyLightFactor, 1.0); }",
            ),
            ShaderSourceFile::new("lib/common.glsl", "#define TEST 1\n"),
            ShaderSourceFile::new("shaders.properties", ""),
            ShaderSourceFile::new("block.properties", ""),
            ShaderSourceFile::new(TERRAIN_RESOURCE_BINDINGS_PATH, "tex=material_atlas\n"),
        ],
    )
    .unwrap();
    let contract = derive_complementary_terrain_contract(&source).unwrap();
    let artifacts =
        preprocess_terrain_sources(&source, &contract.source_stages().unwrap()).unwrap();
    let lowered = lower_terrain_source_pair(&artifacts.vertex, &artifacts.fragment).unwrap();
    let declarations = TerrainSourceResourceBindings::from_source(&source).unwrap();
    let bindings = lowered
        .opaque_resource_contract()
        .bind_semantic_roles(&declarations)
        .unwrap();
    let program = prepare_lowered_terrain_source_program(
        &contract,
        &lowered,
        &bindings,
        TerrainMaterialProgramKind::Opaque,
    )
    .unwrap();

    for module in program.shader_module_descriptors(BackendApi::OpenGl) {
        gal.create_shader_module(module).unwrap_or_else(|error| {
            panic!(
                "prepared source terrain shader must compile through the OpenGL lowering: {error}"
            )
        });
    }
}

/// Compiles the real bundled Complementary terrain pair through the private
/// OpenGL backend. This is source-program conformance only: no Java/Iris
/// state is acquired and the test does not select a gameplay route.
#[test]
fn lowered_complete_complementary_textured_material_pair_compiles_at_the_opengl_boundary() {
    let backend =
        match OpenGlBackend::new("MattMC complete source textured-material OpenGL conformance") {
            Ok(backend) => backend,
            Err(error) => {
                let text = error.to_string();
                assert!(
                    text.contains("OpenGL") || text.contains("EGL") || text.contains("GL"),
                    "unexpected OpenGL complete source textured-material setup failure: {text}"
                );
                return;
            }
        };
    let mut gal = VulkanicGal::new_with_backend(Box::new(backend), false);
    let source = complete_bundled_pack_source_for_test();
    let contract = derive_textured_material_contract(&source, TerrainProgramScope::Overworld)
        .expect("the bundled Complementary scope must expose gbuffers_textured");
    let lowered = lower_textured_material_source_pair(&source, &contract).unwrap();
    let declarations = TerrainSourceResourceBindings::from_source(&source).unwrap();
    let bindings = lowered
        .opaque_resource_contract()
        .bind_semantic_roles(&declarations)
        .unwrap();
    let program =
        prepare_lowered_textured_material_source_program(&contract, &lowered, &bindings).unwrap();

    for module in program.shader_module_descriptors(BackendApi::OpenGl) {
        gal.create_shader_module(module).unwrap_or_else(|error| {
            panic!(
                "complete lowered Complementary textured-material shader must compile through the OpenGL lowering: {error}"
            )
        });
    }
}

/// Compiles the selected bundled weather source through the private OpenGL
/// lowering. This checks Rust-owned source lowering only; it neither borrows
/// Iris state nor selects a mixed gameplay route.
#[test]
fn lowered_complete_complementary_weather_pair_compiles_at_the_opengl_boundary() {
    let backend = match OpenGlBackend::new("MattMC complete source weather OpenGL conformance") {
        Ok(backend) => backend,
        Err(error) => {
            let text = error.to_string();
            assert!(
                text.contains("OpenGL") || text.contains("EGL") || text.contains("GL"),
                "unexpected OpenGL complete source weather setup failure: {text}"
            );
            return;
        }
    };
    let mut gal = VulkanicGal::new_with_backend(Box::new(backend), false);
    let source = complete_bundled_pack_source_for_test();
    let contract = derive_weather_pass_contract(&source, TerrainProgramScope::Overworld)
        .expect("the bundled Complementary scope must expose gbuffers_weather");
    let lowered = lower_weather_source_pair(&source, &contract).unwrap();
    let declarations = TerrainSourceResourceBindings::from_source(&source).unwrap();
    let bindings = lowered
        .opaque_resource_contract()
        .bind_semantic_roles(&declarations)
        .unwrap();
    let program = prepare_lowered_weather_source_program(&contract, &lowered, &bindings).unwrap();

    for module in program.shader_module_descriptors(BackendApi::OpenGl) {
        gal.create_shader_module(module).unwrap_or_else(|error| {
            panic!(
                "complete lowered Complementary weather shader must compile through the OpenGL lowering: {error}"
            )
        });
    }
}

#[test]
fn lowered_complete_complementary_terrain_pair_compiles_at_the_opengl_boundary() {
    let backend = match OpenGlBackend::new("MattMC complete source terrain OpenGL conformance") {
        Ok(backend) => backend,
        Err(error) => {
            let text = error.to_string();
            assert!(
                text.contains("OpenGL") || text.contains("EGL") || text.contains("GL"),
                "unexpected OpenGL complete source terrain setup failure: {text}"
            );
            return;
        }
    };
    let mut gal = VulkanicGal::new_with_backend(Box::new(backend), false);
    let source = complete_bundled_pack_source_for_test();
    let stages = TerrainSourceStages {
        vertex: TerrainSourceStage {
            path: "world0/gbuffers_terrain.vsh".to_string(),
            defines: Default::default(),
        },
        fragment: TerrainSourceStage {
            path: "world0/gbuffers_terrain.fsh".to_string(),
            defines: Default::default(),
        },
    };
    let contract = derive_complementary_terrain_contract(&source).unwrap();
    let artifacts = preprocess_terrain_sources(&source, &stages).unwrap();
    let lowered = lower_terrain_source_pair(&artifacts.vertex, &artifacts.fragment).unwrap();
    let declarations = TerrainSourceResourceBindings::from_source(&source).unwrap();
    let bindings = lowered
        .opaque_resource_contract()
        .bind_semantic_roles(&declarations)
        .unwrap();
    let program = prepare_lowered_terrain_source_program(
        &contract,
        &lowered,
        &bindings,
        TerrainMaterialProgramKind::Opaque,
    )
    .unwrap();

    for module in program.shader_module_descriptors(BackendApi::OpenGl) {
        gal.create_shader_module(module).unwrap_or_else(|error| {
            panic!(
                "complete lowered Complementary terrain shader must compile through the OpenGL lowering: {error}"
            )
        });
    }
}

/// Compiles the selected hand source through the private OpenGL boundary.
/// This is source-program conformance only: it does not borrow Iris state or
/// select a hand gameplay route.
#[test]
fn lowered_complete_complementary_hand_pair_compiles_at_the_opengl_boundary() {
    let backend = match OpenGlBackend::new("MattMC complete source hand OpenGL conformance") {
        Ok(backend) => backend,
        Err(error) => {
            let text = error.to_string();
            assert!(
                text.contains("OpenGL") || text.contains("EGL") || text.contains("GL"),
                "unexpected OpenGL complete source hand setup failure: {text}"
            );
            return;
        }
    };
    let mut gal = VulkanicGal::new_with_backend(Box::new(backend), false);
    let source = complete_bundled_pack_source_for_test();
    let contract = derive_hand_contract(&source, TerrainProgramScope::Overworld)
        .expect("the bundled Complementary scope must expose gbuffers_hand");
    let lowered = lower_hand_source_pair(&source, &contract).unwrap();
    let declarations = TerrainSourceResourceBindings::from_source(&source).unwrap();
    let bindings = bind_hand_source_resources(&lowered, &declarations).unwrap();
    let program = prepare_lowered_hand_source_program(&contract, &lowered, &bindings).unwrap();

    for module in program.shader_module_descriptors(BackendApi::OpenGl) {
        gal.create_shader_module(module).unwrap_or_else(|error| {
            panic!(
                "complete lowered Complementary hand shader must compile through the OpenGL lowering: {error}"
            )
        });
    }
}

#[test]
fn selected_terrain_pipeline_layout_matches_optional_colored_voxel_interface() {
    let backend =
        match OpenGlBackend::new("MattMC VulkanicGAL selected terrain pipeline conformance") {
            Ok(backend) => backend,
            Err(error) => {
                let text = error.to_string();
                assert!(
                    text.contains("OpenGL") || text.contains("EGL") || text.contains("GL"),
                    "unexpected selected terrain pipeline setup failure: {text}"
                );
                return;
            }
        };
    let mut gal = VulkanicGal::new_with_backend(Box::new(backend), false);

    for uses_colored_voxel_light in [false, true] {
        let suffix = if uses_colored_voxel_light {
            "colored-voxel"
        } else {
            "base"
        };
        let mesh_layout = gal
            .create_resource_layout(ResourceLayoutDesc {
                label: format!("selected-terrain.{suffix}.mesh-layout"),
                bindings: vec![
                    ResourceBindingDesc {
                        binding: 0,
                        kind: ResourceBindingKind::StorageBuffer,
                        stages: PipelineStageFlags::DRAW,
                        array_count: 1,
                        optional: false,
                        dynamic_offset_count: 0,
                    },
                    ResourceBindingDesc {
                        binding: 1,
                        kind: ResourceBindingKind::StorageBuffer,
                        stages: PipelineStageFlags::DRAW,
                        array_count: 1,
                        optional: false,
                        dynamic_offset_count: 0,
                    },
                    ResourceBindingDesc {
                        binding: 2,
                        kind: ResourceBindingKind::SampledTexture,
                        stages: PipelineStageFlags::DRAW,
                        array_count: 1,
                        optional: false,
                        dynamic_offset_count: 0,
                    },
                    ResourceBindingDesc {
                        binding: 3,
                        kind: ResourceBindingKind::Sampler,
                        stages: PipelineStageFlags::DRAW,
                        array_count: 1,
                        optional: false,
                        dynamic_offset_count: 0,
                    },
                ],
            })
            .unwrap();
        let mut layouts = vec![mesh_layout];
        if uses_colored_voxel_light {
            let voxel_layout = gal
                .create_resource_layout(ResourceLayoutDesc {
                    label: format!("selected-terrain.{suffix}.voxel-layout"),
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
                        ResourceBindingDesc {
                            binding: 3,
                            kind: ResourceBindingKind::UniformBuffer,
                            stages: PipelineStageFlags::DRAW,
                            array_count: 1,
                            optional: false,
                            dynamic_offset_count: 0,
                        },
                    ],
                })
                .unwrap();
            create_selected_terrain_colored_voxel_resource_set(
                &mut gal,
                voxel_layout,
                &format!("selected-terrain.{suffix}"),
            );
            layouts.push(voxel_layout);
        }
        let pipeline_layout = gal
            .create_pipeline_layout(PipelineLayoutDesc {
                label: format!("selected-terrain.{suffix}.pipeline-layout"),
                resource_layouts: layouts,
            })
            .unwrap();
        let fragment_source = if uses_colored_voxel_light {
            COMPLEMENTARY_TERRAIN_SUBSET_FRAGMENT.replacen(
                "#version 450\n",
                "#version 450\n#define VULKANIC_TERRAIN_COLORED_VOXEL_LIGHT 1\n",
                1,
            )
        } else {
            COMPLEMENTARY_TERRAIN_SUBSET_FRAGMENT.to_owned()
        };
        let vertex = gal
            .create_shader_module(shader_module(
                &format!("selected-terrain.{suffix}.vertex"),
                ShaderStage::Vertex,
                MINIMAL_TERRAIN_MATERIAL_VERTEX,
            ))
            .unwrap();
        let fragment = gal
            .create_shader_module(shader_module(
                &format!("selected-terrain.{suffix}.fragment"),
                ShaderStage::Fragment,
                &fragment_source,
            ))
            .unwrap();
        gal.create_graphics_pipeline(GraphicsPipelineDesc {
            label: format!("selected-terrain.{suffix}.pipeline"),
            layout: pipeline_layout,
            vertex_shader: vertex,
            fragment_shader: fragment,
            topology: PrimitiveTopology::Triangles,
            cull_mode: CullMode::Back,
            front_face: crate::render::vulkanic::resources::FrontFace::CounterClockwise,
            provoking_vertex: crate::render::vulkanic::resources::ProvokingVertex::Last,
            raster_y_direction: crate::render::vulkanic::resources::RasterYDirection::Up,
            blend: BlendMode::Disabled,
            depth_compare: Some(CompareOp::LessOrEqual),
            depth_write: true,
            depth_bias: None,
            color_formats: vec![TextureFormat::Rgba16Float; 4],
            depth_format: Some(TextureFormat::Depth32Float),
            stencil: None,
        })
        .unwrap_or_else(|error| {
            panic!("selected terrain {suffix} pipeline/layout must lower for OpenGL: {error}")
        });
    }
}

fn create_selected_terrain_colored_voxel_resource_set(
    gal: &mut VulkanicGal,
    layout: Handle,
    label: &str,
) {
    let extent = Extent3d {
        width: 4,
        height: 4,
        depth: 4,
    };
    let occupancy = gal
        .create_texture(TextureDesc {
            label: format!("{label}.occupancy"),
            dimension: TextureDimension::D3,
            format: TextureFormat::R8Uint,
            extent,
            mip_levels: 1,
            array_layers: 1,
            usages: vec![TextureUsage::Sampled],
        })
        .unwrap();
    let colored_light = gal
        .create_texture(TextureDesc {
            label: format!("{label}.colored-light"),
            dimension: TextureDimension::D3,
            format: TextureFormat::Rgba16Float,
            extent,
            mip_levels: 1,
            array_layers: 1,
            usages: vec![TextureUsage::Sampled],
        })
        .unwrap();
    let occupancy_view = gal
        .create_texture_view(view(
            &format!("{label}.occupancy-view"),
            occupancy,
            TextureFormat::R8Uint,
        ))
        .unwrap();
    let colored_light_view = gal
        .create_texture_view(view(
            &format!("{label}.colored-light-view"),
            colored_light,
            TextureFormat::Rgba16Float,
        ))
        .unwrap();
    let sampler = gal
        .create_sampler(SamplerDesc {
            label: format!("{label}.voxel-sampler"),
            min_filter: SamplerFilter::Nearest,
            mag_filter: SamplerFilter::Nearest,
            mip_filter: SamplerFilter::Nearest,
            address_u: SamplerAddressMode::ClampToEdge,
            address_v: SamplerAddressMode::ClampToEdge,
            address_w: SamplerAddressMode::ClampToEdge,
            comparison: None,
        })
        .unwrap();
    let mapping = gal
        .create_buffer(BufferDesc {
            label: format!("{label}.mapping"),
            size: 96,
            memory: MemoryDomain::DeviceLocal,
            usages: vec![BufferUsage::Uniform],
        })
        .unwrap();
    gal.create_resource_set(ResourceSetDesc {
        label: format!("{label}.voxel-set"),
        layout,
        bindings: vec![
            selected_resource_binding(0, occupancy_view, ResourceBindingKind::SampledTexture),
            selected_resource_binding(1, colored_light_view, ResourceBindingKind::SampledTexture),
            selected_resource_binding(2, sampler, ResourceBindingKind::Sampler),
            selected_resource_binding(3, mapping, ResourceBindingKind::UniformBuffer),
        ],
    })
    .expect("selected terrain colored-voxel resource set must be constructed");
}

fn selected_resource_binding(
    binding: u32,
    resource: Handle,
    kind: ResourceBindingKind,
) -> ResourceBinding {
    ResourceBinding {
        binding,
        array_index: 0,
        resource,
        kind,
        access: AccessFlags::READ,
        dynamic_offsets: Vec::new(),
        buffer_range: None,
    }
}

#[test]
fn isolated_opengl_conformance_renders_indexed_textured_draw() {
    let result = run_conformance(
        "standard",
        Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth: 1,
        },
    );
    match result {
        Ok(report) => {
            assert_ne!(report.pixel_hash, 0);
            assert!(report.non_zero_pixels > 0);
            assert!(report.state_cache.program_binds > 0);
            assert!(report.state_cache.framebuffer_binds > 0);
            assert_eq!(report.sync_stats.finishes, 0);
            assert_eq!(report.sync_stats.flushes, 0);
            assert!(report.sync_stats.fences_inserted > 0);
            assert!(report.sync_stats.command_ops > 0);
            assert!(report.sync_stats.gl_calls > 0);
        }
        Err(error) => {
            let text = error.to_string();
            assert!(
                text.contains("OpenGL") || text.contains("EGL") || text.contains("GL"),
                "unexpected OpenGL conformance failure: {text}"
            );
        }
    }
}

#[test]
fn isolated_opengl_conformance_round_trips_a_partial_r8uint_d3_box() {
    let backend = match OpenGlBackend::new("MattMC VulkanicGAL OpenGL D3 conformance") {
        Ok(backend) => backend,
        Err(error) => {
            let text = error.to_string();
            assert!(
                text.contains("OpenGL") || text.contains("EGL") || text.contains("GL"),
                "unexpected OpenGL D3 setup failure: {text}"
            );
            return;
        }
    };
    let mut gal = VulkanicGal::new_with_backend(Box::new(backend), false);
    assert!(gal.capabilities().supports(BackendFeature::Texture3d));
    let pattern = (0_u8..32).collect::<Vec<_>>();
    let upload = gal
        .create_buffer(BufferDesc {
            label: "d3.upload".to_owned(),
            size: pattern.len() as u64,
            memory: MemoryDomain::Upload,
            usages: vec![BufferUsage::HostWrite, BufferUsage::TransferSrc],
        })
        .unwrap();
    let readback = gal
        .create_buffer(BufferDesc {
            label: "d3.readback".to_owned(),
            size: pattern.len() as u64,
            memory: MemoryDomain::Readback,
            usages: vec![BufferUsage::TransferDst, BufferUsage::HostRead],
        })
        .unwrap();
    let texture = gal
        .create_texture(TextureDesc {
            label: "d3.r8uint".to_owned(),
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
    let commands = gal
        .create_command_list(CommandListDesc {
            label: "d3.round-trip".to_owned(),
            operations: vec![
                CommandOp::HostWriteBuffer {
                    buffer: upload,
                    offset: 0,
                    data: pattern.clone(),
                },
                buffer_barrier(
                    upload,
                    TextureUsageState::TransferDst,
                    TextureUsageState::TransferSrc,
                ),
                texture_barrier(
                    texture,
                    TextureUsageState::Undefined,
                    TextureUsageState::TransferDst,
                ),
                CommandOp::CopyBufferToTexture(region.clone()),
                texture_barrier(
                    texture,
                    TextureUsageState::TransferDst,
                    TextureUsageState::TransferSrc,
                ),
                CommandOp::CopyTextureToBuffer(BufferImageCopyRegion {
                    buffer: readback,
                    ..region
                }),
                buffer_barrier(
                    readback,
                    TextureUsageState::TransferDst,
                    TextureUsageState::ShaderRead,
                ),
                CommandOp::HostReadBuffer {
                    buffer: readback,
                    offset: 0,
                    size: pattern.len() as u64,
                },
            ],
        })
        .unwrap();
    let token = gal
        .submit(SubmissionBatch {
            label: "d3.round-trip.submit".to_owned(),
            command_lists: vec![commands],
        })
        .unwrap();
    gal.retire_through_for_test(token.submission).unwrap();
    let read = gal
        .completed_host_reads()
        .into_iter()
        .find(|read| read.buffer == readback)
        .expect("D3 readback should complete");
    assert_eq!(pattern, read.bytes);

    let rgba16_pattern = (0_u8..64).collect::<Vec<_>>();
    let rgba16_upload = gal
        .create_buffer(BufferDesc {
            label: "d3.rgba16.upload".to_owned(),
            size: rgba16_pattern.len() as u64,
            memory: MemoryDomain::Upload,
            usages: vec![BufferUsage::HostWrite, BufferUsage::TransferSrc],
        })
        .unwrap();
    let rgba16_readback = gal
        .create_buffer(BufferDesc {
            label: "d3.rgba16.readback".to_owned(),
            size: rgba16_pattern.len() as u64,
            memory: MemoryDomain::Readback,
            usages: vec![BufferUsage::TransferDst, BufferUsage::HostRead],
        })
        .unwrap();
    let rgba16_texture = gal
        .create_texture(TextureDesc {
            label: "d3.rgba16float".to_owned(),
            dimension: TextureDimension::D3,
            format: TextureFormat::Rgba16Float,
            extent: Extent3d {
                width: 2,
                height: 2,
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
    let rgba16_region = BufferImageCopyRegion {
        buffer: rgba16_upload,
        buffer_offset: 0,
        bytes_per_row: 16,
        rows_per_image: 2,
        texture: rgba16_texture,
        texture_mip: 0,
        texture_layer: 0,
        texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
        extent: Extent3d {
            width: 2,
            height: 2,
            depth: 2,
        },
    };
    let rgba16_commands = gal
        .create_command_list(CommandListDesc {
            label: "d3.rgba16.round-trip".to_owned(),
            operations: vec![
                CommandOp::HostWriteBuffer {
                    buffer: rgba16_upload,
                    offset: 0,
                    data: rgba16_pattern.clone(),
                },
                buffer_barrier(
                    rgba16_upload,
                    TextureUsageState::TransferDst,
                    TextureUsageState::TransferSrc,
                ),
                texture_barrier(
                    rgba16_texture,
                    TextureUsageState::Undefined,
                    TextureUsageState::TransferDst,
                ),
                CommandOp::CopyBufferToTexture(rgba16_region.clone()),
                texture_barrier(
                    rgba16_texture,
                    TextureUsageState::TransferDst,
                    TextureUsageState::TransferSrc,
                ),
                CommandOp::CopyTextureToBuffer(BufferImageCopyRegion {
                    buffer: rgba16_readback,
                    ..rgba16_region
                }),
                buffer_barrier(
                    rgba16_readback,
                    TextureUsageState::TransferDst,
                    TextureUsageState::ShaderRead,
                ),
                CommandOp::HostReadBuffer {
                    buffer: rgba16_readback,
                    offset: 0,
                    size: rgba16_pattern.len() as u64,
                },
            ],
        })
        .unwrap();
    let rgba16_submission = gal
        .submit(SubmissionBatch {
            label: "d3.rgba16.round-trip.submit".to_owned(),
            command_lists: vec![rgba16_commands],
        })
        .unwrap();
    gal.retire_through_for_test(rgba16_submission.submission)
        .unwrap();
    let rgba16_read = gal
        .completed_host_reads()
        .into_iter()
        .find(|read| read.buffer == rgba16_readback)
        .expect("Rgba16Float D3 readback should complete");
    assert_eq!(rgba16_pattern, rgba16_read.bytes);
}

#[test]
fn isolated_opengl_conformance_dispatches_a_d3_storage_compute_shader() {
    let backend = match OpenGlBackend::new("MattMC VulkanicGAL OpenGL D3 compute conformance") {
        Ok(backend) => backend,
        Err(error) => {
            let text = error.to_string();
            assert!(
                text.contains("OpenGL") || text.contains("EGL") || text.contains("GL"),
                "unexpected OpenGL D3 compute setup failure: {text}"
            );
            return;
        }
    };
    let mut gal = VulkanicGal::new_with_backend(Box::new(backend), false);
    if !gal.capabilities().supports(BackendFeature::Compute) {
        return;
    }
    let sampled_volume = gal
        .create_texture(TextureDesc {
            label: "d3.compute.sampled-volume".to_owned(),
            dimension: TextureDimension::D3,
            format: TextureFormat::R8Uint,
            extent: Extent3d {
                width: 4,
                height: 4,
                depth: 4,
            },
            mip_levels: 2,
            array_layers: 1,
            usages: vec![TextureUsage::Sampled],
        })
        .unwrap();
    let sampled_mip_view = gal
        .create_texture_view(TextureViewDesc {
            label: "d3.compute.sampled-mip-view".to_owned(),
            texture: sampled_volume,
            format: TextureFormat::R8Uint,
            base_mip: 1,
            mip_count: 1,
            base_layer: 0,
            layer_count: 1,
        })
        .unwrap();
    let volume = gal
        .create_texture(TextureDesc {
            label: "d3.compute.volume".to_owned(),
            dimension: TextureDimension::D3,
            format: TextureFormat::R8Uint,
            extent: Extent3d {
                width: 4,
                height: 4,
                depth: 4,
            },
            mip_levels: 1,
            array_layers: 1,
            usages: vec![TextureUsage::Storage, TextureUsage::TransferSrc],
        })
        .unwrap();
    let volume_view = gal
        .create_texture_view(view(
            "d3.compute.volume.view",
            volume,
            TextureFormat::R8Uint,
        ))
        .unwrap();
    let layout = gal
        .create_resource_layout(ResourceLayoutDesc {
            label: "d3.compute.layout".to_owned(),
            bindings: vec![
                ResourceBindingDesc {
                    binding: 0,
                    kind: ResourceBindingKind::StorageTexture,
                    stages: PipelineStageFlags::COMPUTE,
                    array_count: 1,
                    optional: false,
                    dynamic_offset_count: 0,
                },
                ResourceBindingDesc {
                    binding: 1,
                    kind: ResourceBindingKind::SampledTexture,
                    stages: PipelineStageFlags::COMPUTE,
                    array_count: 1,
                    optional: false,
                    dynamic_offset_count: 0,
                },
            ],
        })
        .unwrap();
    let set = gal
        .create_resource_set(ResourceSetDesc {
            label: "d3.compute.set".to_owned(),
            layout,
            bindings: vec![
                ResourceBinding {
                    binding: 0,
                    array_index: 0,
                    resource: volume_view,
                    kind: ResourceBindingKind::StorageTexture,
                    access: AccessFlags::WRITE,
                    dynamic_offsets: Vec::new(),
                    buffer_range: None,
                },
                ResourceBinding {
                    binding: 1,
                    array_index: 0,
                    resource: sampled_mip_view,
                    kind: ResourceBindingKind::SampledTexture,
                    access: AccessFlags::READ,
                    dynamic_offsets: Vec::new(),
                    buffer_range: None,
                },
            ],
        })
        .unwrap();
    let pipeline_layout = gal
        .create_pipeline_layout(PipelineLayoutDesc {
            label: "d3.compute.pipeline-layout".to_owned(),
            resource_layouts: vec![layout],
        })
        .unwrap();
    let shader = gal
        .create_shader_module(shader_module(
            "d3.compute.shader",
            ShaderStage::Compute,
            D3_COMPUTE_R8UINT_SHADER,
        ))
        .unwrap();
    let pipeline = gal
        .create_compute_pipeline(ComputePipelineDesc {
            label: "d3.compute.pipeline".to_owned(),
            layout: pipeline_layout,
            shader,
        })
        .unwrap();
    let readback = gal
        .create_buffer(BufferDesc {
            label: "d3.compute.readback".to_owned(),
            size: 1,
            memory: MemoryDomain::Readback,
            usages: vec![BufferUsage::TransferDst, BufferUsage::HostRead],
        })
        .unwrap();
    let commands = gal
        .create_command_list(CommandListDesc {
            label: "d3.compute.commands".to_owned(),
            operations: vec![
                texture_barrier(
                    sampled_volume,
                    TextureUsageState::Undefined,
                    TextureUsageState::ShaderRead,
                ),
                texture_barrier(
                    volume,
                    TextureUsageState::Undefined,
                    TextureUsageState::ShaderWrite,
                ),
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
                texture_barrier(
                    volume,
                    TextureUsageState::ShaderWrite,
                    TextureUsageState::TransferSrc,
                ),
                CommandOp::CopyTextureToBuffer(BufferImageCopyRegion {
                    buffer: readback,
                    buffer_offset: 0,
                    bytes_per_row: 1,
                    rows_per_image: 1,
                    texture: volume,
                    texture_mip: 0,
                    texture_layer: 0,
                    texture_origin: TextureOrigin3d { x: 1, y: 2, z: 3 },
                    extent: Extent3d {
                        width: 1,
                        height: 1,
                        depth: 1,
                    },
                }),
                buffer_barrier(
                    readback,
                    TextureUsageState::TransferDst,
                    TextureUsageState::ShaderRead,
                ),
                CommandOp::HostReadBuffer {
                    buffer: readback,
                    offset: 0,
                    size: 1,
                },
            ],
        })
        .unwrap();
    let token = gal
        .submit(SubmissionBatch {
            label: "d3.compute.submit".to_owned(),
            command_lists: vec![commands],
        })
        .unwrap();
    assert_eq!(
        (1, 1),
        gal.opengl_backend()
            .expect("OpenGL backend should remain available")
            .texture_mip_range_for_test(sampled_volume)
            .unwrap(),
        "the sampled D3 view must constrain OpenGL to its requested mip"
    );
    gal.retire_through_for_test(token.submission).unwrap();
    let read = gal
        .completed_host_reads()
        .into_iter()
        .find(|read| read.buffer == readback)
        .expect("OpenGL compute readback should complete");
    assert_eq!(vec![0x5a], read.bytes);
}

#[test]
fn isolated_opengl_conformance_writes_and_reads_a_r8uint_d3_storage_texel() {
    let _ = run_d3_storage_write_read_test(
        "r8uint",
        TextureFormat::R8Uint,
        D3_STORAGE_R8UINT_FRAGMENT_SHADER,
        &[0x5a],
    );
}

#[test]
fn isolated_opengl_conformance_writes_and_reads_a_rgba16float_d3_storage_texel() {
    let _ = run_d3_storage_write_read_test(
        "rgba16float",
        TextureFormat::Rgba16Float,
        D3_STORAGE_RGBA16FLOAT_FRAGMENT_SHADER,
        &[0x00, 0x38, 0x00, 0x34, 0x00, 0x3a, 0x00, 0x3c],
    );
}

fn run_d3_storage_write_read_test(
    format_label: &str,
    format: TextureFormat,
    fragment_shader: &str,
    expected_bytes: &[u8],
) -> Option<Vec<u8>> {
    let backend = match OpenGlBackend::new("MattMC VulkanicGAL OpenGL D3 storage conformance") {
        Ok(backend) => backend,
        Err(error) => {
            let text = error.to_string();
            assert!(
                text.contains("OpenGL") || text.contains("EGL") || text.contains("GL"),
                "unexpected OpenGL D3 storage setup failure: {text}"
            );
            return None;
        }
    };
    let mut gal = VulkanicGal::new_with_backend(Box::new(backend), false);
    assert!(gal.capabilities().supports(BackendFeature::StorageTextures));

    let volume = gal
        .create_texture(TextureDesc {
            label: "d3.storage.volume".to_owned(),
            dimension: TextureDimension::D3,
            format,
            extent: Extent3d {
                width: 4,
                height: 4,
                depth: 4,
            },
            mip_levels: 1,
            array_layers: 1,
            usages: vec![TextureUsage::Storage, TextureUsage::TransferSrc],
        })
        .unwrap();
    let volume_view = gal
        .create_texture_view(view("d3.storage.volume.view", volume, format))
        .unwrap();
    let color = gal
        .create_texture(TextureDesc {
            label: "d3.storage.color".to_owned(),
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            extent: Extent3d {
                width: 1,
                height: 1,
                depth: 1,
            },
            mip_levels: 1,
            array_layers: 1,
            usages: vec![TextureUsage::ColorAttachment],
        })
        .unwrap();
    let color_view = gal
        .create_texture_view(view(
            "d3.storage.color.view",
            color,
            TextureFormat::Rgba8Unorm,
        ))
        .unwrap();
    let target = gal
        .create_render_target(RenderTargetDesc {
            label: "d3.storage.target".to_owned(),
            color_views: vec![color_view],
            depth_stencil_view: None,
            extent: Extent3d {
                width: 1,
                height: 1,
                depth: 1,
            },
        })
        .unwrap();
    let pass = gal
        .create_render_pass(RenderPassDesc {
            label: "d3.storage.pass".to_owned(),
            target,
            color_formats: vec![TextureFormat::Rgba8Unorm],
            depth_format: None,
        })
        .unwrap();
    let layout = gal
        .create_resource_layout(ResourceLayoutDesc {
            label: "d3.storage.layout".to_owned(),
            bindings: vec![ResourceBindingDesc {
                binding: 0,
                kind: ResourceBindingKind::StorageTexture,
                stages: PipelineStageFlags::DRAW,
                array_count: 1,
                optional: false,
                dynamic_offset_count: 0,
            }],
        })
        .unwrap();
    let set = gal
        .create_resource_set(ResourceSetDesc {
            label: "d3.storage.set".to_owned(),
            layout,
            bindings: vec![ResourceBinding {
                binding: 0,
                array_index: 0,
                resource: volume_view,
                kind: ResourceBindingKind::StorageTexture,
                access: AccessFlags::WRITE,
                dynamic_offsets: Vec::new(),
                buffer_range: None,
            }],
        })
        .unwrap();
    let pipeline_layout = gal
        .create_pipeline_layout(PipelineLayoutDesc {
            label: "d3.storage.pipeline-layout".to_owned(),
            resource_layouts: vec![layout],
        })
        .unwrap();
    let vertex = gal
        .create_shader_module(shader_module(
            "d3.storage.vertex",
            ShaderStage::Vertex,
            D3_STORAGE_VERTEX_SHADER,
        ))
        .unwrap();
    let fragment = gal
        .create_shader_module(shader_module(
            "d3.storage.fragment",
            ShaderStage::Fragment,
            fragment_shader,
        ))
        .unwrap();
    let pipeline = gal
        .create_graphics_pipeline(GraphicsPipelineDesc {
            label: "d3.storage.pipeline".to_owned(),
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
        })
        .unwrap();
    let readback = gal
        .create_buffer(BufferDesc {
            label: "d3.storage.readback".to_owned(),
            size: expected_bytes.len() as u64,
            memory: MemoryDomain::Readback,
            usages: vec![BufferUsage::TransferDst, BufferUsage::HostRead],
        })
        .unwrap();
    let read_region = BufferImageCopyRegion {
        buffer: readback,
        buffer_offset: 0,
        bytes_per_row: expected_bytes.len() as u32,
        rows_per_image: 1,
        texture: volume,
        texture_mip: 0,
        texture_layer: 0,
        // A D3 image is a semantic volume rather than a framebuffer. The
        // shader image-store and backend-neutral copy share the same Y axis.
        texture_origin: TextureOrigin3d { x: 1, y: 2, z: 3 },
        extent: Extent3d {
            width: 1,
            height: 1,
            depth: 1,
        },
    };
    let list = gal
        .create_command_list(CommandListDesc {
            label: "d3.storage.commands".to_owned(),
            operations: vec![
                texture_barrier(
                    volume,
                    TextureUsageState::Undefined,
                    TextureUsageState::ShaderWrite,
                ),
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
                        store_op: AttachmentStoreOp::DontCare,
                        clear_color: Some(ClearColor {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                    }],
                    depth_stencil: None,
                },
                CommandOp::BindGraphicsPipeline(pipeline),
                CommandOp::BindResourceSet {
                    pipeline_layout,
                    set_index: 0,
                    set,
                    dynamic_offsets: Vec::new(),
                },
                CommandOp::Draw {
                    vertices: 3,
                    instances: 1,
                },
                CommandOp::EndPass,
                texture_barrier(
                    volume,
                    TextureUsageState::ShaderWrite,
                    TextureUsageState::TransferSrc,
                ),
                CommandOp::CopyTextureToBuffer(read_region),
                buffer_barrier(
                    readback,
                    TextureUsageState::TransferDst,
                    TextureUsageState::ShaderRead,
                ),
                CommandOp::HostReadBuffer {
                    buffer: readback,
                    offset: 0,
                    size: expected_bytes.len() as u64,
                },
            ],
        })
        .unwrap();
    let token = gal
        .submit(SubmissionBatch {
            label: format!("d3.storage.{format_label}.submit"),
            command_lists: vec![list],
        })
        .unwrap();
    gal.retire_through_for_test(token.submission).unwrap();
    let backend = gal.opengl_backend().unwrap();
    assert!(backend.gl_errors_for_test().is_empty());
    let bytes = backend
        .completed_host_reads_for_test()
        .iter()
        .rev()
        .find(|read| read.buffer == readback)
        .map(|read| read.bytes.clone())
        .unwrap();
    assert_eq!(expected_bytes, bytes.as_slice());
    Some(bytes)
}

pub(in crate::render::vulkanic::backends) fn shared_d3_storage_fixture_bytes(
    format: TextureFormat,
) -> Option<Vec<u8>> {
    match format {
        TextureFormat::R8Uint => run_d3_storage_write_read_test(
            "shared-r8uint",
            format,
            D3_STORAGE_R8UINT_FRAGMENT_SHADER,
            &[0x5a],
        ),
        TextureFormat::Rgba16Float => run_d3_storage_write_read_test(
            "shared-rgba16float",
            format,
            D3_STORAGE_RGBA16FLOAT_FRAGMENT_SHADER,
            &[0x00, 0x38, 0x00, 0x34, 0x00, 0x3a, 0x00, 0x3c],
        ),
        _ => panic!("shared D3 storage fixture only covers required volume formats"),
    }
}

#[test]
fn isolated_opengl_conformance_pins_gal_coordinate_and_state_conventions() {
    match run_conformance(
        "conventions",
        Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth: 1,
        },
    ) {
        Ok(report) => {
            assert_conformance_conventions(&report);
            assert!(report.state_cache.program_binds > 0);
            assert!(report.state_cache.framebuffer_binds > 0);
            assert!(report.gl_errors.is_empty());
        }
        Err(error) => {
            let text = error.to_string();
            assert!(
                text.contains("OpenGL") || text.contains("EGL") || text.contains("GL"),
                "unexpected OpenGL convention conformance failure: {text}"
            );
        }
    }
}

#[test]
fn isolated_opengl_conformance_supports_resize_recreation() {
    if let Err(error) = run_conformance(
        "resize-small",
        Extent3d {
            width: 32,
            height: 32,
            depth: 1,
        },
    ) {
        assert!(
            error.to_string().contains("OpenGL") || error.to_string().contains("EGL"),
            "unexpected resize failure: {error}"
        );
        return;
    }
    let report = run_conformance(
        "resize-large",
        Extent3d {
            width: 64,
            height: 48,
            depth: 1,
        },
    )
    .expect("second OpenGL conformance run should recreate resources cleanly");
    assert_ne!(report.pixel_hash, 0);
}

#[test]
fn isolated_opengl_conformance_rejects_partial_failure_cleanly() {
    let backend = match OpenGlBackend::new("MattMC OpenGL partial failure") {
        Ok(backend) => backend,
        Err(error) => {
            assert!(
                error.to_string().contains("OpenGL") || error.to_string().contains("EGL"),
                "unexpected bootstrap failure: {error}"
            );
            return;
        }
    };
    let mut gal = VulkanicGal::new_with_backend(Box::new(backend), false);
    let error = gal
        .create_texture(TextureDesc {
            label: "unsupported-format".to_string(),
            dimension: TextureDimension::D2,
            format: TextureFormat::Bgra8Unorm,
            extent: Extent3d {
                width: 4,
                height: 4,
                depth: 1,
            },
            mip_levels: 1,
            array_layers: 1,
            usages: vec![TextureUsage::Sampled],
        })
        .expect_err("unsupported OpenGL texture format should fail cleanly");
    assert!(error.to_string().contains("OpenGL texture format"));
    let _ = gal.retire_completed().expect("device should remain usable");
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::render::vulkanic::backends) struct ConformanceReport {
    pub(in crate::render::vulkanic::backends) mode: String,
    pub(in crate::render::vulkanic::backends) width: u32,
    pub(in crate::render::vulkanic::backends) height: u32,
    pub(in crate::render::vulkanic::backends) pixel_hash: u32,
    pub(in crate::render::vulkanic::backends) non_zero_pixels: usize,
    pub(in crate::render::vulkanic::backends) evidence_json: String,
    pub(in crate::render::vulkanic::backends) capabilities_json: String,
    pub(in crate::render::vulkanic::backends) gl_errors: Vec<String>,
    pub(in crate::render::vulkanic::backends) state_cache: StateCacheSnapshot,
    pub(in crate::render::vulkanic::backends) sync_stats: OpenGlSyncStats,
}

fn assert_conformance_conventions(report: &ConformanceReport) {
    assert_eq!(report.pixel_hash, 0xfc90_d68e);
    assert_eq!(report.non_zero_pixels, 15_362);
    assert!(report.evidence_json.contains("\"top_left\":[0,0,0,255]"));
    assert!(report.evidence_json.contains("\"top_mid\":[0,0,0,255]"));
    assert!(report
        .evidence_json
        .contains("\"upper_inner\":[191,0,191,255]"));
    assert!(report.evidence_json.contains("\"center\":[96,0,191,255]"));
    assert!(report
        .evidence_json
        .contains("\"lower_inner\":[96,0,191,255]"));
    assert!(report.evidence_json.contains("\"bottom_mid\":[0,0,0,255]"));
    assert!(report
        .evidence_json
        .contains("\"alpha_counts\":{\"255\":6144}"));
}

pub(in crate::render::vulkanic::backends) fn run_conformance(
    mode: &str,
    extent: Extent3d,
) -> GalResult<ConformanceReport> {
    super::trace::message("rust-opengl-conformance-start");
    let backend = OpenGlBackend::new("MattMC VulkanicGAL OpenGL conformance")?;
    let mut gal = VulkanicGal::new_with_backend(Box::new(backend), tracy_enabled_from_env());
    let capabilities_json = gal.capabilities().fingerprint_json();
    let _renderdoc_frame = super::renderdoc::RenderDocFrame::start_if_requested();

    let texture_bytes = texture_bytes();
    let index_bytes = index_bytes();
    let upload_texture = gal.create_buffer(BufferDesc {
        label: "conformance.texture-upload".to_string(),
        size: texture_bytes.len() as u64,
        memory: MemoryDomain::Upload,
        usages: vec![BufferUsage::HostWrite, BufferUsage::TransferSrc],
    })?;
    let index = gal.create_buffer(BufferDesc {
        label: "conformance.index-upload".to_string(),
        size: index_bytes.len() as u64,
        memory: MemoryDomain::Upload,
        usages: vec![BufferUsage::HostWrite, BufferUsage::Index],
    })?;
    let readback_size = u64::from(extent.width) * u64::from(extent.height) * 4;
    let readback = gal.create_buffer(BufferDesc {
        label: "conformance.readback".to_string(),
        size: readback_size,
        memory: MemoryDomain::Readback,
        usages: vec![BufferUsage::TransferDst, BufferUsage::HostRead],
    })?;
    let uniform_a = gal.create_buffer(BufferDesc {
        label: "conformance.uniform.a".to_string(),
        size: 64,
        memory: MemoryDomain::DeviceLocal,
        usages: vec![BufferUsage::Uniform],
    })?;
    let uniform_b = gal.create_buffer(BufferDesc {
        label: "conformance.uniform.b".to_string(),
        size: 64,
        memory: MemoryDomain::DeviceLocal,
        usages: vec![BufferUsage::Uniform],
    })?;
    let storage = gal.create_buffer(BufferDesc {
        label: "conformance.storage".to_string(),
        size: 128,
        memory: MemoryDomain::DeviceLocal,
        usages: vec![BufferUsage::Storage],
    })?;
    let sampled = gal.create_texture(TextureDesc {
        label: "conformance.sampled".to_string(),
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8Unorm,
        extent: Extent3d {
            width: 4,
            height: 4,
            depth: 1,
        },
        mip_levels: 1,
        array_layers: 1,
        usages: vec![TextureUsage::TransferDst, TextureUsage::Sampled],
    })?;
    let color = gal.create_texture(TextureDesc {
        label: "conformance.color".to_string(),
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8Unorm,
        extent,
        mip_levels: 1,
        array_layers: 1,
        usages: vec![TextureUsage::ColorAttachment, TextureUsage::TransferSrc],
    })?;
    let depth = gal.create_texture(TextureDesc {
        label: "conformance.depth".to_string(),
        dimension: TextureDimension::D2,
        format: TextureFormat::Depth32Float,
        extent,
        mip_levels: 1,
        array_layers: 1,
        usages: vec![TextureUsage::DepthStencilAttachment],
    })?;
    let sampled_view = gal.create_texture_view(view(
        "conformance.sampled.view",
        sampled,
        TextureFormat::Rgba8Unorm,
    ))?;
    let color_view = gal.create_texture_view(view(
        "conformance.color.view",
        color,
        TextureFormat::Rgba8Unorm,
    ))?;
    let depth_view = gal.create_texture_view(view(
        "conformance.depth.view",
        depth,
        TextureFormat::Depth32Float,
    ))?;
    let sampler = gal.create_sampler(SamplerDesc {
        label: "conformance.sampler".to_string(),
        min_filter: SamplerFilter::Nearest,
        mag_filter: SamplerFilter::Nearest,
        mip_filter: SamplerFilter::Nearest,
        address_u: SamplerAddressMode::ClampToEdge,
        address_v: SamplerAddressMode::ClampToEdge,
        address_w: SamplerAddressMode::ClampToEdge,
        comparison: None,
    })?;
    let combined_sampler = gal.create_combined_texture_sampler(CombinedTextureSamplerDesc {
        label: "conformance.combined-sampler".to_string(),
        texture_view: sampled_view,
        sampler,
    })?;
    let resource_layout = gal.create_resource_layout(ResourceLayoutDesc {
        label: "conformance.resource-layout".to_string(),
        bindings: vec![
            ResourceBindingDesc {
                binding: 1,
                kind: ResourceBindingKind::CombinedTextureSampler,
                stages: PipelineStageFlags::DRAW,
                array_count: 1,
                optional: false,
                dynamic_offset_count: 0,
            },
            ResourceBindingDesc {
                binding: 3,
                kind: ResourceBindingKind::UniformBuffer,
                stages: PipelineStageFlags::DRAW,
                array_count: 2,
                optional: false,
                dynamic_offset_count: 0,
            },
            ResourceBindingDesc {
                binding: 4,
                kind: ResourceBindingKind::StorageBuffer,
                stages: PipelineStageFlags::DRAW,
                array_count: 1,
                optional: false,
                dynamic_offset_count: 0,
            },
            ResourceBindingDesc {
                binding: 5,
                kind: ResourceBindingKind::Sampler,
                stages: PipelineStageFlags::DRAW,
                array_count: 1,
                optional: true,
                dynamic_offset_count: 0,
            },
        ],
    })?;
    let resource_set = gal.create_resource_set(ResourceSetDesc {
        label: "conformance.resource-set".to_string(),
        layout: resource_layout,
        bindings: vec![
            ResourceBinding {
                binding: 1,
                array_index: 0,
                resource: combined_sampler,
                kind: ResourceBindingKind::CombinedTextureSampler,
                access: AccessFlags::READ,
                dynamic_offsets: Vec::new(),
                buffer_range: None,
            },
            ResourceBinding {
                binding: 3,
                array_index: 0,
                resource: uniform_a,
                kind: ResourceBindingKind::UniformBuffer,
                access: AccessFlags::READ,
                dynamic_offsets: Vec::new(),
                buffer_range: None,
            },
            ResourceBinding {
                binding: 3,
                array_index: 1,
                resource: uniform_b,
                kind: ResourceBindingKind::UniformBuffer,
                access: AccessFlags::READ,
                dynamic_offsets: Vec::new(),
                buffer_range: None,
            },
            ResourceBinding {
                binding: 4,
                array_index: 0,
                resource: storage,
                kind: ResourceBindingKind::StorageBuffer,
                access: AccessFlags::READ,
                dynamic_offsets: Vec::new(),
                buffer_range: None,
            },
        ],
    })?;
    let pipeline_layout = gal.create_pipeline_layout(PipelineLayoutDesc {
        label: "conformance.pipeline-layout".to_string(),
        resource_layouts: vec![resource_layout],
    })?;
    let vertex_shader = gal.create_shader_module(shader_module(
        "conformance.vertex",
        ShaderStage::Vertex,
        VERTEX_SHADER,
    ))?;
    let fragment_shader = gal.create_shader_module(shader_module(
        "conformance.fragment",
        ShaderStage::Fragment,
        FRAGMENT_SHADER,
    ))?;
    let pipeline = gal.create_graphics_pipeline(GraphicsPipelineDesc {
        label: "conformance.pipeline".to_string(),
        layout: pipeline_layout,
        vertex_shader,
        fragment_shader,
        topology: PrimitiveTopology::Triangles,
        cull_mode: CullMode::Back,
        front_face: crate::render::vulkanic::resources::FrontFace::CounterClockwise,
        provoking_vertex: crate::render::vulkanic::resources::ProvokingVertex::Last,
        raster_y_direction: crate::render::vulkanic::resources::RasterYDirection::Up,
        blend: BlendMode::Alpha,
        depth_compare: Some(CompareOp::LessOrEqual),
        depth_write: true,
        depth_bias: None,
        color_formats: vec![TextureFormat::Rgba8Unorm],
        depth_format: Some(TextureFormat::Depth32Float),
        stencil: None,
    })?;
    let target = gal.create_render_target(RenderTargetDesc {
        label: "conformance.target".to_string(),
        color_views: vec![color_view],
        depth_stencil_view: Some(depth_view),
        extent,
    })?;
    let pass = gal.create_render_pass(RenderPassDesc {
        label: "conformance.pass".to_string(),
        target,
        color_formats: vec![TextureFormat::Rgba8Unorm],
        depth_format: Some(TextureFormat::Depth32Float),
    })?;
    let command_list = gal.create_command_list(CommandListDesc {
        label: "conformance.commands".to_string(),
        operations: conformance_ops(
            upload_texture,
            index,
            readback,
            sampled,
            color,
            depth,
            pass,
            target,
            color_view,
            depth_view,
            pipeline,
            pipeline_layout,
            resource_set,
            texture_bytes,
            index_bytes,
            extent,
        ),
    })?;
    let token = gal.submit(SubmissionBatch {
        label: "conformance.submit".to_string(),
        command_lists: vec![command_list],
    })?;
    gal.retire_through_for_test(token.submission)?;
    let backend = gal
        .opengl_backend()
        .ok_or_else(|| GalError::backend("conformance backend was not OpenGL"))?;
    let reads = backend.completed_host_reads_for_test();
    let pixels = reads
        .iter()
        .rev()
        .find(|read| read.buffer == readback)
        .map(|read| read.bytes.clone())
        .ok_or_else(|| GalError::backend("OpenGL conformance readback produced no bytes"))?;
    let gl_errors = backend.gl_errors_for_test();
    let state_cache = backend.state_cache_for_test();
    let sync_stats = backend.sync_stats_for_test();
    if !gl_errors.is_empty() {
        return Err(GalError::backend(format!(
            "OpenGL conformance produced errors: {}",
            gl_errors.join("; ")
        )));
    }
    let pixel_hash = xxh32(&pixels, 0x4d_43_47_41);
    let non_zero_pixels = pixels.iter().filter(|byte| **byte != 0).count();
    let evidence_json = pixel_evidence_json(&pixels, extent);
    let report = ConformanceReport {
        mode: mode.to_string(),
        width: extent.width,
        height: extent.height,
        pixel_hash,
        non_zero_pixels,
        evidence_json,
        capabilities_json,
        gl_errors,
        state_cache,
        sync_stats,
    };
    write_report(&report)?;
    super::trace::message("rust-opengl-conformance-complete");
    Ok(report)
}

#[allow(clippy::too_many_arguments)]
fn conformance_ops(
    upload_texture: crate::render::vulkanic::handles::Handle,
    index: crate::render::vulkanic::handles::Handle,
    readback: crate::render::vulkanic::handles::Handle,
    sampled: crate::render::vulkanic::handles::Handle,
    color: crate::render::vulkanic::handles::Handle,
    depth: crate::render::vulkanic::handles::Handle,
    pass: crate::render::vulkanic::handles::Handle,
    target: crate::render::vulkanic::handles::Handle,
    color_view: crate::render::vulkanic::handles::Handle,
    depth_view: crate::render::vulkanic::handles::Handle,
    pipeline: crate::render::vulkanic::handles::Handle,
    pipeline_layout: crate::render::vulkanic::handles::Handle,
    resource_set: crate::render::vulkanic::handles::Handle,
    texture_bytes: Vec<u8>,
    index_bytes: Vec<u8>,
    extent: Extent3d,
) -> Vec<CommandOp> {
    let tex_copy = BufferImageCopyRegion {
        buffer: upload_texture,
        buffer_offset: 0,
        bytes_per_row: 16,
        rows_per_image: 4,
        texture: sampled,
        texture_mip: 0,
        texture_layer: 0,
        texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
        extent: Extent3d {
            width: 4,
            height: 4,
            depth: 1,
        },
    };
    let read_copy = BufferImageCopyRegion {
        buffer: readback,
        buffer_offset: 0,
        bytes_per_row: extent.width * 4,
        rows_per_image: extent.height,
        texture: color,
        texture_mip: 0,
        texture_layer: 0,
        texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
        extent,
    };
    vec![
        CommandOp::HostWriteBuffer {
            buffer: upload_texture,
            offset: 0,
            data: texture_bytes,
        },
        buffer_barrier(
            upload_texture,
            TextureUsageState::TransferDst,
            TextureUsageState::TransferSrc,
        ),
        CommandOp::HostWriteBuffer {
            buffer: index,
            offset: 0,
            data: index_bytes,
        },
        buffer_barrier(
            index,
            TextureUsageState::TransferDst,
            TextureUsageState::IndexRead,
        ),
        texture_barrier(
            sampled,
            TextureUsageState::Undefined,
            TextureUsageState::TransferDst,
        ),
        CommandOp::CopyBufferToTexture(tex_copy),
        texture_barrier(
            sampled,
            TextureUsageState::TransferDst,
            TextureUsageState::ShaderRead,
        ),
        texture_barrier(
            color,
            TextureUsageState::Undefined,
            TextureUsageState::ColorAttachment,
        ),
        texture_barrier(
            depth,
            TextureUsageState::Undefined,
            TextureUsageState::DepthStencilAttachment,
        ),
        CommandOp::BeginPass {
            pass,
            target,
            colors: vec![PassAttachment {
                view: color_view,
                load_op: AttachmentLoadOp::Clear,
                store_op: AttachmentStoreOp::Store,
                clear_color: Some(ClearColor {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 1.0,
                }),
            }],
            depth_stencil: Some(PassAttachment {
                view: depth_view,
                load_op: AttachmentLoadOp::Clear,
                store_op: AttachmentStoreOp::DontCare,
                clear_color: None,
            }),
        },
        CommandOp::BindGraphicsPipeline(pipeline),
        CommandOp::BindResourceSet {
            pipeline_layout,
            set_index: 0,
            set: resource_set,
            dynamic_offsets: Vec::new(),
        },
        CommandOp::SetIndexBuffer {
            buffer: index,
            offset: 0,
            index_type: crate::render::vulkanic::resources::IndexType::U32,
        },
        CommandOp::DrawIndexed {
            indices: 6,
            instances: 1,
        },
        CommandOp::EndPass,
        texture_barrier(
            color,
            TextureUsageState::ColorAttachment,
            TextureUsageState::TransferSrc,
        ),
        CommandOp::CopyTextureToBuffer(read_copy),
        buffer_barrier(
            readback,
            TextureUsageState::TransferDst,
            TextureUsageState::ShaderRead,
        ),
        CommandOp::HostReadBuffer {
            buffer: readback,
            offset: 0,
            size: u64::from(extent.width) * u64::from(extent.height) * 4,
        },
    ]
}

fn buffer_barrier(
    resource: crate::render::vulkanic::handles::Handle,
    before: TextureUsageState,
    after: TextureUsageState,
) -> CommandOp {
    CommandOp::Barrier(ResourceBarrier {
        resource,
        subresources: None,
        before,
        after,
        src_queue: QueueClass::Graphics,
        dst_queue: QueueClass::Graphics,
    })
}

fn texture_barrier(
    resource: crate::render::vulkanic::handles::Handle,
    before: TextureUsageState,
    after: TextureUsageState,
) -> CommandOp {
    CommandOp::Barrier(ResourceBarrier {
        resource,
        subresources: Some(TextureSubresourceRange {
            base_mip: 0,
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

fn view(
    label: &str,
    texture: crate::render::vulkanic::handles::Handle,
    format: TextureFormat,
) -> TextureViewDesc {
    TextureViewDesc {
        label: label.to_string(),
        texture,
        format,
        base_mip: 0,
        mip_count: 1,
        base_layer: 0,
        layer_count: 1,
    }
}

fn shader_module(label: &str, stage: ShaderStage, source: &str) -> ShaderModuleDesc {
    ShaderModuleDesc {
        label: label.to_string(),
        stage,
        code_format: ShaderCodeFormat::Glsl,
        code: source.as_bytes().to_vec(),
        entry_point: "main".to_string(),
    }
}

const D3_COMPUTE_R8UINT_SHADER: &str = r#"
#version 430 core
layout(local_size_x = 1, local_size_y = 1, local_size_z = 1) in;
layout(binding = 0, r8ui) uniform writeonly uimage3D volume;
void main() {
    imageStore(volume, ivec3(1, 2, 3), uvec4(0x5au));
}
"#;

fn texture_bytes() -> Vec<u8> {
    vec![
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255, 255, 255, 255, 255, 0,
        128, 255, 255, 255, 0, 255, 255, 32, 32, 32, 255, 255, 128, 0, 255, 0, 255, 128, 255, 128,
        0, 255, 255, 255, 255, 255, 255, 64, 64, 64, 255, 255, 0, 128, 255, 0, 255, 255, 255, 128,
        255, 0, 255,
    ]
}

fn index_bytes() -> Vec<u8> {
    [0_u32, 1, 2, 2, 3, 0]
        .into_iter()
        .flat_map(u32::to_ne_bytes)
        .collect()
}

fn tracy_enabled_from_env() -> bool {
    std::env::var("MATTMC_RUST_TRACY")
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn write_report(report: &ConformanceReport) -> GalResult<()> {
    let path = report_path()?;
    std::fs::create_dir_all(path.parent().expect("report path has parent")).map_err(|error| {
        GalError::backend(format!(
            "failed to create OpenGL conformance log dir: {error}"
        ))
    })?;
    let json = format!(
        "{{\n  \"artifact_class\": \"rust_opengl_conformance\",\n  \"backend\": \"Rust OpenGL\",\n  \"mode\": \"{}\",\n  \"instrumentation\": \"clean-conformance\",\n  \"width\": {},\n  \"height\": {},\n  \"pixel_hash_xxh32\": \"{:08x}\",\n  \"non_zero_pixels\": {},\n  \"pixel_evidence\": {},\n  \"backend_capabilities\": {},\n  \"gl_error_count\": {},\n  \"sync\": {{\"command_batches\": {}, \"command_lists\": {}, \"command_ops\": {}, \"gl_calls\": {}, \"flushes\": {}, \"finishes\": {}, \"fences_inserted\": {}, \"fences_polled\": {}, \"fences_waited\": {}, \"fences_deleted\": {}}}\n}}\n",
        report.mode,
        report.width,
        report.height,
        report.pixel_hash,
        report.non_zero_pixels,
        report.evidence_json,
        report.capabilities_json,
        report.gl_errors.len(),
        report.sync_stats.command_batches,
        report.sync_stats.command_lists,
        report.sync_stats.command_ops,
        report.sync_stats.gl_calls,
        report.sync_stats.flushes,
        report.sync_stats.finishes,
        report.sync_stats.fences_inserted,
        report.sync_stats.fences_polled,
        report.sync_stats.fences_waited,
        report.sync_stats.fences_deleted
    );
    std::fs::write(&path, json).map_err(|error| {
        GalError::backend(format!(
            "failed to write OpenGL conformance report: {error}"
        ))
    })
}

fn pixel_evidence_json(pixels: &[u8], extent: Extent3d) -> String {
    let width = extent.width as usize;
    let height = extent.height as usize;
    let row_bytes = width * 4;
    let top_row = &pixels[..row_bytes];
    let bottom_row = &pixels[(height - 1) * row_bytes..height * row_bytes];
    let mut alpha_counts = std::collections::BTreeMap::<u8, usize>::new();
    for pixel in pixels.chunks_exact(4) {
        *alpha_counts.entry(pixel[3]).or_default() += 1;
    }
    let alpha_json = alpha_counts
        .into_iter()
        .map(|(alpha, count)| format!("\"{}\":{}", alpha, count))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"top_left\":{},\"top_mid\":{},\"upper_inner\":{},\"center\":{},\"lower_inner\":{},\"bottom_mid\":{},\"bottom_left\":{},\"top_row_hash\":\"{:08x}\",\"bottom_row_hash\":\"{:08x}\",\"alpha_counts\":{{{}}}}}",
        pixel_json(pixels, width, 0, 0),
        pixel_json(pixels, width, width / 2, 0),
        pixel_json(pixels, width, width / 2, height / 3),
        pixel_json(pixels, width, width / 2, height / 2),
        pixel_json(pixels, width, width / 2, (height * 2) / 3),
        pixel_json(pixels, width, width / 2, height - 1),
        pixel_json(pixels, width, 0, height - 1),
        xxh32(top_row, 0x4d_43_47_41),
        xxh32(bottom_row, 0x4d_43_47_41),
        alpha_json
    )
}

fn pixel_json(pixels: &[u8], width: usize, x: usize, y: usize) -> String {
    let index = (y * width + x) * 4;
    format!(
        "[{},{},{},{}]",
        pixels[index],
        pixels[index + 1],
        pixels[index + 2],
        pixels[index + 3]
    )
}

fn report_path() -> GalResult<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .and_then(|path| path.parent())
        .map(PathBuf::from)
        .ok_or_else(|| GalError::backend("failed to locate repository root from Cargo manifest"))?;
    Ok(root.join("logs/rust-opengl/conformance/latest.json"))
}

const D3_STORAGE_VERTEX_SHADER: &str = r#"
#version 430 core
const vec2 positions[3] = vec2[3](
    vec2(-1.0, -1.0),
    vec2( 3.0, -1.0),
    vec2(-1.0,  3.0)
);
void main() {
    gl_Position = vec4(positions[gl_VertexID], 0.0, 1.0);
}
"#;

const D3_STORAGE_R8UINT_FRAGMENT_SHADER: &str = r#"
#version 430 core
layout(binding = 0, r8ui) uniform writeonly uimage3D volume;
layout(location = 0) out vec4 out_color;
void main() {
    imageStore(volume, ivec3(1, 2, 3), uvec4(0x5au));
    out_color = vec4(0.0);
}
"#;

const D3_STORAGE_RGBA16FLOAT_FRAGMENT_SHADER: &str = r#"
#version 430 core
layout(binding = 0, rgba16f) uniform writeonly image3D volume;
layout(location = 0) out vec4 out_color;
void main() {
    imageStore(volume, ivec3(1, 2, 3), vec4(0.5, 0.25, 0.75, 1.0));
    out_color = vec4(0.0);
}
"#;

const VERTEX_SHADER: &str = r#"
#version 330 core
out vec2 v_uv;
const vec2 pos[4] = vec2[4](
    vec2(-0.85, -0.85),
    vec2( 0.85, -0.85),
    vec2( 0.85,  0.85),
    vec2(-0.85,  0.85)
);
const vec2 uv[4] = vec2[4](
    vec2(0.0, 0.0),
    vec2(1.0, 0.0),
    vec2(1.0, 1.0),
    vec2(0.0, 1.0)
);
void main() {
    gl_Position = vec4(pos[gl_VertexID], 0.5, 1.0);
    v_uv = uv[gl_VertexID];
}
"#;

const FRAGMENT_SHADER: &str = r#"
#version 330 core
uniform sampler2D tex0;
in vec2 v_uv;
out vec4 out_color;
void main() {
    out_color = texture(tex0, v_uv) * vec4(1.0, 1.0, 1.0, 0.75);
}
"#;
