pub mod assets;
pub(crate) mod vanilla_imports;
pub(crate) mod vanilla_sources;
pub(crate) mod engine_globals;
pub mod cloud_contract;
pub mod custom_uniform_policy;
pub mod diagnostics;
pub mod dialect;
pub mod distant_horizons_contract;
pub mod entity_contract;
pub mod entity_id_map;
pub(crate) mod fullscreen;
pub mod fullscreen_contract;
pub mod hand_contract;
pub mod held_light_policy;
pub mod interface;
pub mod item_id_map;
pub mod lightmap;
pub mod lowering;
pub mod manifest;
pub mod material_contract;
pub mod pass_graph;
pub mod preprocess;
pub mod programs;
pub mod resources;
pub(crate) mod runtime;
pub mod shadow_policy;
pub mod source;
pub mod source_targets;
// Source-selected shader-pack resources are private Rust-owned candidate
// preparation state. Runtime admission remains fail-closed until the complete
// source plan is wired into the production submit path; keep the candidate
// implementation compiled and unit-tested without presenting it as admitted.
pub(crate) mod fabulous_targets;
#[allow(dead_code)]
pub(crate) mod source_assets;
pub mod source_temporal;
pub mod source_uniforms;
pub mod terrain_contract;
pub mod terrain_source_resources;
pub mod terrain_voxelization;
pub mod uniforms;
pub mod vanilla_post_effect_contract;
pub mod vanilla_post_effect_executor;
pub mod voxel_emission_table;
pub mod voxel_light_volume;
pub mod voxel_material_map;
pub mod weather_contract;
pub mod wetness_policy;

#[cfg(test)]
mod tests {
    use super::manifest::ShaderPackConfig;
    use super::manifest::ShaderPackManifest;
    use super::pass_graph::{
        builtin_terrain_material_pass_graph, distant_horizons_opaque_pass_graph, AttachmentRole,
        LoadIntent, PassGraph, PassIdentity, ShaderPassDesc, StoreIntent,
    };
    use super::preprocess::{preprocess, PreprocessInput};
    use super::programs::{
        complementary_terrain_subset_program, complementary_terrain_subset_program_with_resources,
        minimal_composite_color_grade_program, minimal_composite_depth_fog_program,
        minimal_deferred_lighting_program, minimal_final_copy_program,
        minimal_shadow_depth_program, minimal_terrain_cutout_program,
        minimal_terrain_solid_program, ProgramIdentity, ShaderStageKind,
        TerrainMaterialProgramKind, TerrainProgramResource,
    };
    use super::resources::{
        ShaderPackResourceManifest, ShaderPackResources, ShaderPackRuntimePlan,
    };
    use super::source::{ShaderPackSource, ShaderSourceFile};
    use super::terrain_contract::{
        bundled_complementary_hung_loified_source, derive_complementary_terrain_contract,
        TerrainPassOutput, TerrainPassRequiredResource,
    };
    use super::voxel_light_volume::{
        VoxelLightVolumeCache, VoxelLightVolumeDescriptor, VoxelLightVolumeExtent,
        VoxelLightVolumeIdentity, VoxelLightVolumeKind, VoxelLightVolumeMapping,
        VoxelLightVolumeRegion, VoxelLightVolumeUpdate,
    };

    #[test]
    fn minimal_terrain_solid_program_is_rust_owned_and_backend_neutral() {
        let program = minimal_terrain_solid_program();
        assert_eq!(
            ProgramIdentity::new("vulkanic:builtin/terrain_opaque_v1"),
            program.identity
        );
        assert_eq!(ShaderStageKind::Vertex, program.vertex.stage);
        assert_eq!(ShaderStageKind::Fragment, program.fragment.stage);
        assert!(program.vertex.source.contains("WorldMeshVertices"));
        assert!(program.fragment.source.contains("sampler2D"));
        assert!(program.fragment.source.contains("out_terrain_lit_color"));
        assert!(program
            .fragment
            .source
            .contains("out_terrain_material_auxiliary"));
        assert!(!program.requires(TerrainProgramResource::ColoredVoxelLightVolume));
        assert!(!program.vertex.source.contains("IrisRenderSystem"));
        assert!(!program.fragment.source.contains("IrisRenderSystem"));

        let cutout = minimal_terrain_cutout_program();
        assert_eq!(
            ProgramIdentity::new("vulkanic:builtin/terrain_cutout_v1"),
            cutout.identity
        );
        assert_eq!(program.vertex.source, cutout.vertex.source);
        assert!(cutout.fragment.source.contains("discard"));
    }

    #[test]
    fn preprocessor_expands_includes_and_defines_without_java_state() {
        let source = ShaderPackSource::new(
            "test-pack",
            7,
            vec![
                ShaderSourceFile::new("common.glsl", "vec3 shade(vec3 v) { return v; }\n"),
                ShaderSourceFile::new("terrain.vsh", "#include \"common.glsl\"\nvoid main() {}\n"),
            ],
        )
        .unwrap();
        let output = preprocess(PreprocessInput {
            source: &source,
            entry: "terrain.vsh",
            defines: &[("VULKANIC_TEST", "1")],
        })
        .unwrap();
        assert!(output.contains("#define VULKANIC_TEST 1"));
        assert!(output.contains("vec3 shade"));
    }

    #[test]
    fn pass_graph_names_explicit_attachments_and_ordering() {
        let graph = PassGraph::new(vec![ShaderPassDesc {
            identity: PassIdentity::new("vulkanic:pass/terrain-solid-test"),
            label: "terrain-solid".to_string(),
            program: ProgramIdentity::new("vulkanic:builtin/terrain_opaque_v1"),
            colors: vec![AttachmentRole::GBufferAlbedo],
            depth: Some(AttachmentRole::Depth),
            load: LoadIntent::Load,
            store: StoreIntent::Store,
        }])
        .unwrap();
        assert_eq!(1, graph.passes().len());
        assert_eq!("terrain-solid", graph.passes()[0].label);
    }

    #[test]
    fn builtin_material_pass_graph_declares_g_buffer_and_final_output() {
        let graph = builtin_terrain_material_pass_graph().unwrap();
        assert_eq!(8, graph.passes().len());
        assert_eq!(
            "vulkanic:pass/shadow_depth",
            graph.passes()[0].identity.as_str()
        );
        assert_eq!(LoadIntent::Clear, graph.passes()[0].load);
        assert_eq!(Some(AttachmentRole::ShadowDepth), graph.passes()[0].depth);
        assert_eq!(
            ProgramIdentity::new("vulkanic:builtin/terrain_opaque_v1"),
            graph.passes()[1].program
        );
        assert_eq!(LoadIntent::Clear, graph.passes()[1].load);
        assert_eq!(
            "vulkanic:pass/terrain_opaque",
            graph.passes()[1].identity.as_str()
        );
        assert_eq!(
            "vulkanic:pass/terrain_cutout",
            graph.passes()[2].identity.as_str()
        );
        assert_eq!(LoadIntent::Load, graph.passes()[2].load);
        assert_eq!(
            vec![
                AttachmentRole::GBufferAlbedo,
                AttachmentRole::GBufferNormal,
                AttachmentRole::GBufferMaterialLight,
                AttachmentRole::GBufferWorldPosition,
            ],
            graph.passes()[1].colors
        );
        assert_eq!(
            "vulkanic:pass/deferred_lighting",
            graph.passes()[3].identity.as_str()
        );
        assert_eq!(
            vec![AttachmentRole::DeferredLitColor],
            graph.passes()[3].colors
        );
        assert_eq!(
            "vulkanic:pass/terrain_translucent",
            graph.passes()[4].identity.as_str()
        );
        assert_eq!(
            vec![AttachmentRole::DeferredLitColor],
            graph.passes()[4].colors
        );
        assert_eq!(
            "vulkanic:pass/composite_0",
            graph.passes()[5].identity.as_str()
        );
        assert_eq!(vec![AttachmentRole::Composite0], graph.passes()[5].colors);
        assert_eq!(
            "vulkanic:pass/composite_1",
            graph.passes()[6].identity.as_str()
        );
        assert_eq!(vec![AttachmentRole::Composite1], graph.passes()[6].colors);
        assert_eq!(
            "vulkanic:pass/final_output",
            graph.passes()[7].identity.as_str()
        );
        let deferred = minimal_deferred_lighting_program();
        assert!(deferred.fragment.source.contains("AlbedoTex"));
        assert!(deferred.fragment.source.contains("NormalTex"));
        assert!(deferred.fragment.source.contains("ShadowDepthTex"));
        let color_grade = minimal_composite_color_grade_program();
        assert!(color_grade.fragment.source.contains("color_grade_params"));
        let fog = minimal_composite_depth_fog_program();
        // Builtin fog reconstructs view-space distance from the explicit
        // main-depth attachment. It must not consume the former lossy packed
        // world-position color target.
        assert!(fog.fragment.source.contains("MainDepthTex"));
        assert!(fog.fragment.source.contains("projection_inverse"));
        assert!(!fog.fragment.source.contains("WorldPositionTex"));
        let final_copy = minimal_final_copy_program();
        assert!(final_copy.fragment.source.contains("Tex0"));
        let shadow = minimal_shadow_depth_program();
        assert_eq!(
            ProgramIdentity::new("vulkanic:builtin/shadow_depth_v1"),
            shadow.identity
        );
        assert!(shadow.fragment.source.contains("discard"));
    }

    #[test]
    fn distant_horizons_graph_keeps_distant_depth_out_of_near_terrain_targets() {
        let graph =
            distant_horizons_opaque_pass_graph(ProgramIdentity::new("pack:dh_terrain")).unwrap();
        assert_eq!(1, graph.passes().len());
        assert_eq!(
            vec![AttachmentRole::ShaderPackPrimaryColor],
            graph.passes()[0].colors
        );
        assert_eq!(Some(AttachmentRole::DistantDepth), graph.passes()[0].depth);
    }

    #[test]
    fn shader_pack_resource_manifest_is_versioned_and_backend_neutral() {
        let resources = ShaderPackResourceManifest::terrain_material_v1(12).unwrap();
        assert_eq!(12, resources.generation.0);
        assert!(resources
            .attachments
            .iter()
            .any(|attachment| attachment.as_str() == "vulkanic:attachment/shadow_depth"));
        assert!(resources
            .attachments
            .iter()
            .any(|attachment| attachment.as_str() == "vulkanic:attachment/g_buffer_albedo"));
        assert!(resources
            .attachments
            .iter()
            .any(|attachment| attachment.as_str() == "vulkanic:attachment/g_buffer_normal"));
        assert!(resources.attachments.iter().any(|attachment| {
            attachment.as_str() == "vulkanic:attachment/g_buffer_material_light"
        }));
        assert!(resources.attachments.iter().any(|attachment| {
            attachment.as_str() == "vulkanic:attachment/g_buffer_world_position"
        }));
        assert!(resources
            .attachments
            .iter()
            .any(|attachment| attachment.as_str() == "vulkanic:attachment/deferred_lit_color"));
        assert!(resources
            .attachments
            .iter()
            .any(|attachment| attachment.as_str() == "vulkanic:attachment/composite_0"));
        assert!(resources
            .attachments
            .iter()
            .any(|attachment| attachment.as_str() == "vulkanic:attachment/composite_1"));
        assert!(resources
            .materials
            .iter()
            .any(|material| material.as_str() == "minecraft:material/cutout_textured"));
        assert!(resources
            .samplers
            .iter()
            .any(|sampler| sampler.as_str() == "vulkanic:sampler/nearest_clamp"));
    }

    #[test]
    fn shader_pack_config_declares_ordered_multipass_chain() {
        let config = ShaderPackConfig::internal_shadow_composite_fixture(4).unwrap();
        let graph = config.pass_graph().unwrap();
        let passes = graph
            .passes()
            .iter()
            .map(|pass| pass.identity.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            passes,
            vec![
                "vulkanic:pass/shadow_depth",
                "vulkanic:pass/terrain_opaque",
                "vulkanic:pass/terrain_cutout",
                "vulkanic:pass/deferred_lighting",
                "vulkanic:pass/terrain_translucent",
                "vulkanic:pass/composite_0",
                "vulkanic:pass/composite_1",
                "vulkanic:pass/final_output",
            ]
        );
        assert!(config
            .passes
            .iter()
            .any(|pass| pass.reads.contains(&AttachmentRole::ShadowDepth)
                && pass.colors.contains(&AttachmentRole::DeferredLitColor)));
    }

    #[test]
    fn runtime_plan_owns_composite_chain_program_and_resource_selection() {
        let plan = ShaderPackRuntimePlan::terrain_material_multipass_v1(8).unwrap();
        assert_eq!(8, plan.generation);
        assert_eq!(8, plan.graph.passes().len());
        assert_eq!(
            "vulkanic:builtin/deferred_lighting_v1",
            plan.programs.deferred_lighting.identity.as_str()
        );
        assert_eq!(
            "vulkanic:builtin/composite_color_grade_v1",
            plan.programs.composite_0.identity.as_str()
        );
        assert_eq!(
            "vulkanic:builtin/composite_depth_fog_v1",
            plan.programs.composite_1.identity.as_str()
        );
        assert!(plan
            .declared_attachment_roles()
            .contains(&AttachmentRole::Composite1));
        assert!(plan
            .resources
            .programs
            .iter()
            .any(|program| { program.as_str() == plan.programs.final_output.identity.as_str() }));
    }

    #[test]
    fn complementary_terrain_contract_exposes_only_source_writable_named_outputs() {
        let source = bundled_complementary_hung_loified_source(9).unwrap();
        let contract = derive_complementary_terrain_contract(&source).unwrap();
        assert!(contract
            .outputs
            .contains(&TerrainPassOutput::LitTerrainColor));
        assert!(contract
            .outputs
            .contains(&TerrainPassOutput::MaterialAuxiliary));
        assert!(
            contract
                .outputs
                .contains(&TerrainPassOutput::ViewSpaceNormal),
            "the selected source now declares its normal output through an active DRAWBUFFERS target"
        );
        assert_eq!(
            Some(0),
            contract.output_color_slot(TerrainPassOutput::LitTerrainColor)
        );
        assert_eq!(
            Some(6),
            contract.output_color_slot(TerrainPassOutput::MaterialAuxiliary)
        );
        assert_eq!(
            Some(5),
            contract.output_color_slot(TerrainPassOutput::ViewSpaceNormal)
        );
        assert!(contract.material_ids.contains_key(&10009));
        let diagnostic_plan = ShaderPackRuntimePlan::terrain_material_multipass_v1(9).unwrap();
        let mut diagnostic_plan = diagnostic_plan;
        diagnostic_plan.terrain_contract = Some(contract);
        let diagnostic = diagnostic_plan.terrain_contract_diagnostic_json();
        assert!(diagnostic.contains("source_contract_discovered"));
        assert!(diagnostic.contains("terrain_lit_color"));
        assert!(diagnostic.contains("WorldMeshVertex.shader_block_id"));
    }

    #[test]
    fn runtime_plan_rejects_source_generation_mismatch() {
        let source = bundled_complementary_hung_loified_source(3).unwrap();
        assert!(ShaderPackRuntimePlan::terrain_material_from_source(4, &source).is_err());
    }

    #[test]
    fn complementary_subset_program_is_source_admitted_and_rejects_current_profile() {
        let supported = derive_complementary_terrain_contract(&ShaderPackSource::new(
            "supported",
            5,
            vec![
                ShaderSourceFile::new(
                    "program/gbuffers_terrain.glsl",
                    "void DoLighting() {}\n/* DRAWBUFFERS:06 */\nvoid main() { vec4 color = texture2D(tex, texCoord); if (color.a <= 0.00001) discard; color.rgb *= glColor.rgb; DoLighting(); gl_FragData[0] = color; gl_FragData[1] = vec4(smoothnessD, materialMask, skyLightFactor, 1.0); }",
                ),
                ShaderSourceFile::new("lib/common.glsl", "#define COLORED_LIGHTING 0\n#define RP_MODE 0\n#define BLOCK_REFLECT_QUALITY 0\n"),
                ShaderSourceFile::new("shaders.properties", "profile.MATTMC=COLORED_LIGHTING=0 RP_MODE=0 BLOCK_REFLECT_QUALITY=0\n"),
                ShaderSourceFile::new("block.properties", "block.1=minecraft:stone\n"),
            ],
        ).unwrap()).unwrap();
        let program =
            complementary_terrain_subset_program(&supported, TerrainMaterialProgramKind::Opaque)
                .unwrap();
        assert!(program.fragment.source.contains("sampled_atlas_color"));
        assert!(program
            .fragment
            .source
            .contains("out_terrain_material_auxiliary"));

        let selected = derive_complementary_terrain_contract(
            &bundled_complementary_hung_loified_source(5).unwrap(),
        )
        .unwrap();
        assert!(selected
            .required_resources
            .contains(&TerrainPassRequiredResource::ColoredVoxelLightVolume));
        assert!(complementary_terrain_subset_program(
            &selected,
            TerrainMaterialProgramKind::Cutout
        )
        .is_err());
        assert!(ShaderPackRuntimePlan::complementary_terrain_contract_v1(5).is_err());
    }

    #[test]
    fn selected_source_plan_accepts_a_complete_colored_volume_and_declares_its_binding() {
        let source = ShaderPackSource::new(
            "selected-volume",
            17,
            vec![
                ShaderSourceFile::new(
                    "program/gbuffers_terrain.glsl",
                    "void DoLighting() {}\n/* DRAWBUFFERS:06 */\nvoid main() { vec4 color = texture2D(tex, texCoord); if (color.a <= 0.00001) discard; color.rgb *= glColor.rgb; DoLighting(); gl_FragData[0] = color; gl_FragData[1] = vec4(smoothnessD, materialMask, skyLightFactor, 1.0); }",
                ),
                ShaderSourceFile::new("lib/common.glsl", "#define COLORED_LIGHTING 128\n#define RP_MODE 0\n#define BLOCK_REFLECT_QUALITY 0\n"),
                ShaderSourceFile::new(
                    "program/shadowcomp.glsl",
                    "void main() { vec3 posOffset = floor(previousCameraPosition) - floor(cameraPosition); }",
                ),
                ShaderSourceFile::new("shaders.properties", "profile.MATTMC=COLORED_LIGHTING=128 RP_MODE=0 BLOCK_REFLECT_QUALITY=0\nimage.voxel_img = voxel_sampler red_integer r8ui unsigned_int true false 128 64 128\nimage.floodfill_img = floodfill_sampler rgba rgba16f half_float false false 128 64 128\nimage.floodfill_img_copy = floodfill_sampler_copy rgba rgba16f half_float false false 128 64 128\n"),
                ShaderSourceFile::new("block.properties", "block.1=minecraft:stone\n"),
            ],
        )
        .unwrap();
        let extent = VoxelLightVolumeExtent {
            width: 128,
            height: 64,
            depth: 128,
        };
        let contract = derive_complementary_terrain_contract(&source).unwrap();
        let descriptor = VoxelLightVolumeDescriptor {
            identity: VoxelLightVolumeIdentity::new("shader-pack:selected-volume/light").unwrap(),
            shader_pack_generation: 17,
            world_generation: 4,
            resource_generation: 9,
            extent,
            // This fixture declares temporal reprojection only. Keep the
            // volume descriptor source-derived so it does not accidentally
            // claim the bundled pack's unsupported behind-view policy.
            requirements: contract.voxel_light_volume_requirements.unwrap(),
            mapping: VoxelLightVolumeMapping::complementary(extent, [0, 64, 0], [0.0; 3]).unwrap(),
        };
        let mut volume = VoxelLightVolumeCache::new();
        volume.replace_descriptor(descriptor.clone()).unwrap();
        assert!(
            ShaderPackRuntimePlan::terrain_material_from_source_with_voxel_light_volume(
                17,
                &source,
                &volume.readiness().unwrap().unwrap(),
                0
            )
            .is_err()
        );
        for kind in [
            VoxelLightVolumeKind::Occupancy,
            VoxelLightVolumeKind::FloodFillEven,
            VoxelLightVolumeKind::FloodFillOdd,
        ] {
            let region = VoxelLightVolumeRegion::whole(extent);
            volume
                .apply_update(VoxelLightVolumeUpdate {
                    identity: descriptor.identity.clone(),
                    shader_pack_generation: descriptor.shader_pack_generation,
                    world_generation: descriptor.world_generation,
                    resource_generation: descriptor.resource_generation,
                    kind,
                    region,
                    texels: vec![0; region.extent.byte_len(kind.format()) as usize],
                })
                .unwrap();
        }
        let readiness = volume.readiness().unwrap().unwrap();
        let program = complementary_terrain_subset_program_with_resources(
            &contract,
            TerrainMaterialProgramKind::Opaque,
            Some(&readiness),
            0,
        )
        .unwrap();
        assert!(program.requires(TerrainProgramResource::ColoredVoxelLightVolume));
        assert!(program
            .fragment
            .source
            .contains("#define VULKANIC_TERRAIN_COLORED_VOXEL_LIGHT 1"));
        assert!(program
            .fragment
            .source
            .contains("layout(set = 1, binding = 0) uniform utexture3D TerrainVoxelOccupancy"));
        assert!(program
            .fragment
            .source
            .contains("layout(set = 1, binding = 1) uniform texture3D TerrainColoredVoxelLight"));
        assert!(program
            .fragment
            .source
            .contains("layout(set = 1, binding = 3, std140) uniform TerrainVoxelLightMapping"));
        assert!(program
            .fragment
            .source
            .contains("texture(sampler3D(TerrainColoredVoxelLight, TerrainVoxelLightSampler)"));
        let plan = ShaderPackRuntimePlan::terrain_material_from_source_with_voxel_light_volume(
            17, &source, &readiness, 0,
        )
        .unwrap();
        assert_eq!(Some(&descriptor), plan.voxel_light_volume.as_ref());
        assert!(plan
            .programs
            .terrain_opaque
            .requires(TerrainProgramResource::ColoredVoxelLightVolume));
        assert!(plan
            .terrain_contract_diagnostic_json()
            .contains("\"selected_source_plan_prepared\":true"));
        assert!(plan.terrain_contract_diagnostic_json().contains(
            "\"voxel_light_update_policy\":{\"temporal_reprojection\":true,\"alternate_x_half_rate\":false,\"preserve_behind_view\":false}"
        ));
        assert!(plan.terrain_contract_diagnostic_json().contains(
            "\"selected_source_execution_admission_scope\":\"per-frame-runtime-status\""
        ));
    }

    #[test]
    fn shader_pack_config_rejects_reads_before_writes_and_duplicate_writers() {
        let mut read_before_write = ShaderPackConfig::internal_shadow_composite_fixture(5).unwrap();
        read_before_write.passes.swap(3, 4);
        let error = read_before_write.validate().unwrap_err();
        assert!(error.to_string().contains("before"));

        let mut duplicate_writer = ShaderPackConfig::internal_shadow_composite_fixture(6).unwrap();
        duplicate_writer.passes[5].colors = vec![AttachmentRole::DeferredLitColor];
        let error = duplicate_writer.validate().unwrap_err();
        assert!(error.to_string().contains("multiple"));
    }

    #[test]
    fn resource_update_rolls_back_malformed_generation() {
        let mut resources = ShaderPackResources::default();
        resources
            .apply_generation(
                1,
                ShaderPackManifest::new(
                    "valid-pack",
                    1,
                    vec![ProgramIdentity::new("vulkanic:builtin/terrain_opaque_v1")],
                )
                .unwrap(),
            )
            .unwrap();
        let active = resources.active_generation();
        let stale_manifest = ShaderPackManifest::new(
            "bad-pack",
            2,
            vec![ProgramIdentity::new("vulkanic:builtin/terrain_opaque_v1")],
        )
        .unwrap();
        assert!(resources.apply_generation(1, stale_manifest).is_err());
        assert_eq!(active, resources.active_generation());
    }

    #[test]
    fn malformed_resource_generation_rolls_back_to_last_valid_manifest() {
        let mut resources = ShaderPackResources::default();
        let manifest = ShaderPackManifest::new(
            "valid-pack",
            4,
            vec![
                ProgramIdentity::new("vulkanic:builtin/terrain_opaque_v1"),
                ProgramIdentity::new("vulkanic:builtin/terrain_cutout_v1"),
            ],
        )
        .unwrap();
        resources
            .apply_resource_generation(
                4,
                manifest,
                ShaderPackResourceManifest::terrain_material_v1(4).unwrap(),
            )
            .unwrap();
        let bad_manifest = ShaderPackManifest::new(
            "bad-pack",
            5,
            vec![ProgramIdentity::new("vulkanic:builtin/terrain_opaque_v1")],
        )
        .unwrap();
        assert!(resources
            .apply_resource_generation(
                5,
                bad_manifest,
                ShaderPackResourceManifest::terrain_material_v1(6).unwrap(),
            )
            .is_err());
        assert_eq!(Some(4), resources.active_generation());
        assert_eq!("valid-pack", resources.active().unwrap().manifest.name());
    }
}
