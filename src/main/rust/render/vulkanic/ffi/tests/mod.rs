use super::*;
use crate::render::vulkanic::resources::{BackendApi, BackendFeatureFlags, BackendLimits};
use crate::render::vulkanic::world_primitive_frontend::world_text::WORLD_TEXT_DEPTH_POLYGON_OFFSET;
use crate::render::vulkanic::world_primitive_frontend::{
    WorldShaderEnvironmentFrame, WorldVoxelVolumeFrame, WORLD_LOD_FLAG_RUST_OPAQUE_ROUTE_SELECTED,
    WORLD_LOD_LAYER_OPAQUE, WORLD_LOD_VARIANT_EXACT, WORLD_LOD_VERTEX_LAYOUT_V1,
    WORLD_MATERIAL_ID_OPAQUE_TEXTURED, WORLD_MATERIAL_ID_TRANSLUCENT_TEXTURED,
    WORLD_MATERIAL_SOURCE_CLOUDS, WORLD_MATERIAL_SOURCE_ENTITY_MODEL,
    WORLD_MATERIAL_SOURCE_PARTICLES, WORLD_MATERIAL_SOURCE_TEXTURED,
    WORLD_MATERIAL_SOURCE_UNSPECIFIED, WORLD_MATERIAL_SOURCE_UV_LOCAL_TEXTURE,
    WORLD_MATERIAL_SOURCE_UV_MINECRAFT_BLOCK_ATLAS, WORLD_MATERIAL_SOURCE_WEATHER,
    WORLD_MATERIAL_TEXTURE_EXPERIENCE_ORB, WORLD_MATERIAL_TEXTURE_STONE, WORLD_MAX_MESH_VERTICES,
    WORLD_MESH_TEXTURE_TERRAIN_BLOCK_ATLAS,
};

fn test_capabilities() -> BackendCapabilities {
    BackendCapabilities {
        api: BackendApi::Mock,
        name: "ffi-test",
        features: BackendFeatureFlags {
            graphics: true,
            descriptor_arrays: true,
            optional_bindings: true,
            uniform_buffers: true,
            storage_buffers: true,
            texture_subresource_copies: true,
            host_buffer_access: true,
            presentation: true,
            ..BackendFeatureFlags::default()
        },
        limits: BackendLimits {
            max_buffer_size: 1024 * 1024,
            max_texture_extent_2d: 4096,
            max_texture_extent_3d: 0,
            max_texture_mip_levels: 1,
            max_texture_array_layers: 1,
            max_resource_layout_bindings: 16,
            max_binding_array_count: 16,
            max_color_attachments: 1,
            max_dynamic_offsets_per_binding: 0,
            max_command_lists_per_submission: 16,
            max_commands_per_list: 1024,
            max_draw_count: 1024,
            max_dispatch_groups_per_axis: 1,
        },
    }
}

fn semantic_particle_record() -> FfiWorldParticleQuadRequest {
    FfiWorldParticleQuadRequest {
        byte_size: size_of::<FfiWorldParticleQuadRequest>() as u32,
        texture_id: 0x50415254, surface_kind: 1, material_index: 0,
        center: [1.0,2.0,3.0], rotation: [0.0,0.0,0.0,1.0], size: 0.5,
        uv_bounds: [0.0,1.0,0.0,1.0], color_argb: 0x80ffffff, packed_light: 240,
    }
}

fn semantic_orb_instance() -> FfiWorldExperienceOrbInstanceRecord {
    FfiWorldExperienceOrbInstanceRecord {
        byte_size:size_of::<FfiWorldExperienceOrbInstanceRecord>() as u32,
        mesh_index:0, mesh_key:0x0b01, mesh_generation:2,
        entity_transform:[1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0,
            0.0,0.0,1.0,0.0, 1.25,-0.7,2.5,1.0],
        camera_orientation:[0.0,0.0,0.0,1.0], entity_id:42, reserved0:0,
    }
}

#[test]
fn experience_orb_frame_transport_lowers_placement_and_preserves_mesh_order() {
    let meshes = [mesh_instance(), FfiWorldMeshInstanceRecord {mesh_key:45,..mesh_instance()}];
    let mut orbs = [semantic_orb_instance(), FfiWorldExperienceOrbInstanceRecord {
        mesh_key:0x0b02, mesh_index:1,..semantic_orb_instance()
    }, FfiWorldExperienceOrbInstanceRecord {
        mesh_key:0x0b03, mesh_index:1,..semantic_orb_instance()
    }, FfiWorldExperienceOrbInstanceRecord {
        mesh_key:0x0b04, mesh_index:2,..semantic_orb_instance()
    }];
    let mut request = whole_frame_request_with_mesh_instances(&meshes);
    request.world_experience_orbs = FfiSlice { ptr:orbs.as_ptr(), count:4 };
    let (_,_,frame,_) = unsafe {decode_whole_frame_submit(&request, test_vulkan_capabilities())}.unwrap();
    assert_eq!(frame.mesh_instances.iter().map(|i|i.mesh_key).collect::<Vec<_>>(),
        vec![0x0b01,44,0x0b02,0x0b03,45,0x0b04]);
    let instance = &frame.mesh_instances[0];
    assert_eq!(instance.depth_policy, WORLD_DEPTH_POLICY_TEST_WRITE);
    assert_eq!(instance.stratum, WORLD_STRATUM_ENTITY_MESH);
    assert_eq!(instance.cull_policy, WORLD_CULL_BACK);
    assert_eq!(instance.entity_id,42);
    assert_eq!(instance.transform[0],0.3);
    assert!((instance.transform[13]+0.6).abs()<1e-6);
    orbs[0].entity_transform[12] = 999.0;
    assert_eq!(instance.transform[12],1.25);
    assert_eq!(orbs[0].entity_transform[12],999.0);
    let layout = super::layout::layout_for_struct(110).unwrap();
    assert_eq!(layout.byte_size,112);
    assert_eq!(&layout.field_offsets[..8],&[0,4,8,16,24,88,104,108]);
    let layout = super::layout::layout_for_struct(53).unwrap();
    assert_eq!(layout.field_offsets[45],std::mem::offset_of!(FfiWholeFrameSubmitRequest,world_experience_orbs) as u32);
}

#[test]
fn experience_orb_frame_transport_rejects_bad_placement_order_and_bounds() {
    let good = semantic_orb_instance();
    for bad in [
        FfiWorldExperienceOrbInstanceRecord {byte_size:0,..good},
        FfiWorldExperienceOrbInstanceRecord {mesh_key:0,..good},
        FfiWorldExperienceOrbInstanceRecord {mesh_index:1,..good},
        FfiWorldExperienceOrbInstanceRecord {reserved0:1,..good},
        FfiWorldExperienceOrbInstanceRecord {camera_orientation:[0.0;4],..good},
        FfiWorldExperienceOrbInstanceRecord {entity_transform:[f32::NAN;16],..good},
    ] {
        let mut request = whole_frame_request(&[],&[]);
        request.world_experience_orbs = FfiSlice {ptr:&bad,count:1};
        assert!(unsafe {decode_whole_frame_submit(&request,test_vulkan_capabilities())}.is_err());
    }
    let meshes = [mesh_instance()];
    let descending = [FfiWorldExperienceOrbInstanceRecord {mesh_index:1,..good},good];
    let mut request = whole_frame_request_with_mesh_instances(&meshes);
    request.world_experience_orbs = FfiSlice {ptr:descending.as_ptr(),count:2};
    assert!(unsafe {decode_whole_frame_submit(&request,test_vulkan_capabilities())}.is_err());
    request.world_experience_orbs = FfiSlice {ptr:ptr::null(),count:u64::MAX};
    assert!(unsafe {decode_whole_frame_submit(&request,test_vulkan_capabilities())}.is_err());
}

#[test]
fn experience_orb_frame_transport_rejects_old_header_before_reading_extended_fields() {
    for header in [FfiHeader {version:53,byte_size:8},
        FfiHeader {version:FFI_ABI_VERSION,byte_size:8}] {
        assert!(unsafe {decode_whole_frame_submit(
            (&header as *const FfiHeader).cast(),test_vulkan_capabilities())}.is_err());
    }
}

#[test]
fn particle_semantic_transport_lowers_owned_geometry_in_the_whole_frame() {
    let mut p = semantic_particle_record();
    let mut request = whole_frame_request(&[], &[]);
    request.world_particle_quads = FfiSlice { ptr: &p, count: 1 };
    let mut capabilities = test_capabilities(); capabilities.api = BackendApi::Vulkan;
    let decoded = unsafe { decode_whole_frame_submit(&request, capabilities) }.unwrap();
    let quad = &decoded.2.material_quads[0];
    assert_eq!(quad.vertices, [[1.5,1.5,3.0],[1.5,2.5,3.0],[0.5,2.5,3.0],[0.5,1.5,3.0]]);
    assert_eq!(quad.source_program, WORLD_MATERIAL_SOURCE_PARTICLES);
    assert_eq!(quad.color_argb, 0x80ffffff);
    p.center[0] = 100.0;
    assert_eq!(quad.vertices[0][0], 1.5, "decoded frame owns its geometry");
    assert_eq!(p.center[0], 100.0);
    let layout = super::layout::layout_for_struct(108).unwrap();
    assert_eq!(layout.field_count, 10);
    assert_eq!(layout.byte_size, size_of::<FfiWorldParticleQuadRequest>() as u32);
    assert_eq!(layout.field_offsets[5], std::mem::offset_of!(FfiWorldParticleQuadRequest, rotation) as u32);
    let layout = super::layout::layout_for_struct(53).unwrap();
    assert_eq!(layout.field_offsets[44], std::mem::offset_of!(FfiWholeFrameSubmitRequest, world_particle_quads) as u32);
}

#[test]
fn terrain_surface_transport_uses_rust_block_atlas_and_cutout_policy() {
    use crate::render::vulkanic::world_primitive_frontend::{
        WORLD_MATERIAL_MODE_OPAQUE, WORLD_MATERIAL_MODE_CUTOUT, WORLD_MATERIAL_MODE_TRANSLUCENT};
    for surface_kind in [2,3,4] {
        let mut p = semantic_particle_record();
        p.surface_kind = surface_kind;
        p.uv_bounds = [0.5,0.25,0.25,0.5];
        let mut request = whole_frame_request(&[], &[]);
        request.world_particle_quads = FfiSlice { ptr: &p, count: 1 };
        let mut capabilities = test_capabilities(); capabilities.api = BackendApi::Vulkan;
        let decoded = unsafe { decode_whole_frame_submit(&request, capabilities) }.unwrap();
        let quad = &decoded.2.material_quads[0];
        assert_eq!(quad.source_uv_space, WORLD_MATERIAL_SOURCE_UV_MINECRAFT_BLOCK_ATLAS);
        assert_eq!(quad.material_mode, match surface_kind {
            3 => WORLD_MATERIAL_MODE_CUTOUT, 4 => WORLD_MATERIAL_MODE_TRANSLUCENT,
            _ => WORLD_MATERIAL_MODE_OPAQUE });
        assert_eq!(quad.uvs, [[0.25,0.5],[0.25,0.25],[0.5,0.25],[0.5,0.5]]);
        assert_eq!(quad.vertex_packed_light, [240;4]);
        request.header.version = 51;
        assert!(unsafe { decode_whole_frame_submit(&request, capabilities) }.is_err(),
            "old particle surface contract must not cross ABI 52");
    }
}

#[test]
fn particle_semantic_transport_preserves_interleaving_and_equal_index_order() {
    let base = semantic_particle_record();
    let materials = super::world::merge_particle_semantics(vec![], &[base,base], [1280,720]).unwrap();
    let mut particles = [base;4];
    for (p,(index,color)) in particles.iter_mut().zip([(0,1),(1,2),(1,3),(2,4)]) {
        p.material_index=index; p.color_argb=color;
    }
    let result = super::world::merge_particle_semantics(materials.clone(), &particles, [1280,720]).unwrap();
    assert_eq!(result.iter().map(|q| q.color_argb).collect::<Vec<_>>(),
        vec![1,base.color_argb,2,3,base.color_argb,4]);
    particles[2].material_index = 0;
    assert!(super::world::merge_particle_semantics(materials.clone(), &particles, [1280,720]).is_err());
    particles[2].material_index = 3;
    assert!(super::world::merge_particle_semantics(materials, &particles, [1280,720]).is_err());
}

#[test]
fn particle_semantic_transport_rejects_bad_records_and_bounds_before_reading() {
    let good = semantic_particle_record();
    let mut request = whole_frame_request(&[], &[]);
    let mut capabilities = test_capabilities(); capabilities.api = BackendApi::Vulkan;
    let mut bads = [good;4];
    bads[0].byte_size -= 4;
    bads[1].surface_kind = 5;
    bads[2].rotation = [0.0;4];
    bads[3].material_index = 1;
    for bad in bads {
        request.world_particle_quads = FfiSlice { ptr: &bad, count: 1 };
        assert!(unsafe { decode_whole_frame_submit(&request, capabilities) }.is_err());
    }
    request.world_particle_quads = FfiSlice { ptr: std::ptr::NonNull::dangling().as_ptr(), count: u64::MAX };
    assert_eq!(input_bytes_for_whole_frame(&request), u64::MAX);
    assert!(unsafe { decode_whole_frame_submit(&request, capabilities) }.is_err());
    request.world_particle_quads.count = FFI_MAX_BATCH_ITEMS as u64 + 1;
    assert!(unsafe { decode_whole_frame_submit(&request, capabilities) }.is_err());
}

#[test]
fn gui_atlas_reference_transport_is_owned_bounded_and_layout_described() {
    let mut item = FfiGuiAtlasReference { byte_size: size_of::<FfiGuiAtlasReference>() as u32,
        texture_id: 17, asset_id: 101, atlas_generation: 7, atlas_width: 8, atlas_height: 8,
        x: 2, y: 3, width: 4, height: 2 };
    let mut request = FfiGuiAtlasReferenceUpdate {
        header: FfiHeader { version: FFI_ABI_VERSION, byte_size: size_of::<FfiGuiAtlasReferenceUpdate>() as u32 },
        revision: 1, references: FfiSlice { ptr: &item, count: 1 }, negotiated_feature_bits: 0,
    };
    let (revision, copied) = unsafe { decode_gui_atlas_reference_update(&request, test_capabilities()) }.unwrap();
    item.x = 0;
    assert_eq!(revision, 1);
    assert_eq!(copied[0].x, 2, "decoded declaration cannot borrow producer memory");
    assert_eq!(48, size_of::<FfiGuiAtlasReference>());
    assert_eq!(40, size_of::<FfiGuiAtlasReferenceUpdate>());
    assert!(super::layout::layout_for_struct(105).is_ok());
    assert!(super::layout::layout_for_struct(106).is_ok());
    for count in [4097, u64::MAX] {
        request.references = FfiSlice { ptr: 1usize as *const FfiGuiAtlasReference, count };
        assert!(unsafe { decode_gui_atlas_reference_update(&request, test_capabilities()) }.is_err());
    }
    let duplicate = [item, item];
    request.references = FfiSlice { ptr: duplicate.as_ptr(), count: 2 };
    assert!(unsafe { decode_gui_atlas_reference_update(&request, test_capabilities()) }.is_err());
    request.references = FfiSlice { ptr: &item, count: 1 };
    item.width = u32::MAX;
    assert!(unsafe { decode_gui_atlas_reference_update(&request, test_capabilities()) }.is_err());
    request.references = FfiSlice { ptr: ptr::null(), count: 0 };
    assert!(unsafe { decode_gui_atlas_reference_update(&request, test_capabilities()) }.unwrap().1.is_empty());
    request.revision = 0;
    assert!(unsafe { decode_gui_atlas_reference_update(&request, test_capabilities()) }.is_err());
}

#[test]
fn gui_submit_ffi_consumes_context_owned_atlas_declarations_without_raw_images() {
    // Registry/FFI integration uses the mock presentation surface; real Vulkan
    // region readback is covered by the mixed GUI encoder tests.
    let create = FfiContextCreateRequest {
        header: FfiHeader { version: FFI_ABI_VERSION, byte_size: size_of::<FfiContextCreateRequest>() as u32 },
        backend_kind: 1, tracy_enabled: 0, label: FfiBytes { ptr: b"GUI submit FFI".as_ptr(), len: 14 },
    };
    let mut result = FfiContextResult::default();
    assert_eq!(unsafe { mattmc_vulkanic_gal_context_create(&create, &mut result) }, 0);
    let id = result.context_id;
    let mut encoded = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut encoded, 2, 2);
        encoder.set_color(png::ColorType::Rgba); encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(&[255; 16]).unwrap();
    }
    let target = with_registry_mut(|registry| {
        let context = registry.contexts.get_mut(&id).unwrap();
        let mut capabilities = test_capabilities();
        capabilities.features.blended_pass = true;
        context.gal = VulkanicGal::new_with_backend(Box::new(
            crate::render::vulkanic::backends::mock::MockBackend::with_capabilities(capabilities)), false);
        context.world_primitive_frontend.apply_world_mesh_asset_update(&mut context.gal, 1, Vec::new(),
            vec![WorldMeshTextureAssetPayload {
                texture_id: 17, png_bytes: encoded, mip_png_bytes: Vec::new(),
                frame_width: 0, frame_height: 0, frame_count: 1, frame_ticks: 1,
                animation_flags: 0, frame_row_size: 0, interpolation_policy: 0,
                animation_frames: Vec::new(), coordinate_origin: 0, sampling: None, requested_mip_levels: 1,
            }]).unwrap();
        let extent = Extent3d { width: 320, height: 180, depth: 1 };
        context.gal.configure_frame_surface(FrameSurfaceDesc { label: "FFI test surface".into(),
            extent, color_format: TextureFormat::Rgba8Unorm, present_mode: PresentMode::Fifo,
            max_frames_in_flight: 2 }).unwrap();
        context.gal.create_frame_target(FrameTargetDesc { label: "FFI test target".into(),
            frame_id: 11, render_target: FrameRenderTargetId(1), extent,
            color_format: TextureFormat::Rgba8Unorm }).unwrap()
    });
    let quad = affine_quad_request();
    let reference = FfiGuiAtlasReference { byte_size: size_of::<FfiGuiAtlasReference>() as u32,
        texture_id: 17, asset_id: quad.asset_id, atlas_generation: 1, atlas_width: 2, atlas_height: 2,
        x: 0, y: 0, width: 2, height: 2 };
    let declaration = FfiGuiAtlasReferenceUpdate {
        header: FfiHeader { version: FFI_ABI_VERSION, byte_size: size_of::<FfiGuiAtlasReferenceUpdate>() as u32 },
        revision: 1, references: FfiSlice { ptr: &reference, count: 1 }, negotiated_feature_bits: 0,
    };
    assert_eq!(unsafe { mattmc_vulkanic_gal_gui_update_atlas_references(id, &declaration, ptr::null_mut()) }, 0);
    let mut frame = frame_request(&[]);
    frame.frame_target = FfiHandle::from(target);
    frame.affine_quads = FfiSlice { ptr: &quad, count: 1 };
    let mut submitted = FfiGuiFrameSubmitResult::default();
    let status = unsafe { mattmc_vulkanic_gal_gui_submit_frame(id, &frame, &mut submitted) };
    let error = with_registry_mut(|registry| registry.contexts.get(&id).unwrap().last_error.clone());
    assert_eq!(status, 0, "{error}");
    assert!(submitted.submission_id > 0);
    with_registry_mut(|registry| {
        let context = registry.contexts.get_mut(&id).unwrap();
        context.gui_frontend.reset(&mut context.gal).unwrap();
        context.world_primitive_frontend.reset(&mut context.gal);
        context.gal.destroy(target).unwrap();
        context.gal.retire_through(context.gal.latest_submission_id()).unwrap();
        assert_eq!(context.gal.metrics().resource_creates, context.gal.metrics().resource_destroys);
    });
    assert_eq!(unsafe { mattmc_vulkanic_gal_context_destroy(id, ptr::null_mut()) }, 0);
}

fn gui_tiled_record() -> FfiGuiTiledQuadRequest {
    FfiGuiTiledQuadRequest {
        byte_size: size_of::<FfiGuiTiledQuadRequest>() as u32,
        stratum: 3, asset_id: 41, bounds: [7, 11, 77, 50], tile_extent: [32, 32],
        uv: [0.25, 0.5, 0.75, 1.0], pose: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        z: 0.25, color_argb: 0xffaabbcc, sequence: 11, clip_mode: 1, clip: [1, 2, 100, 50],
    }
}

#[test]
fn semantic_gui_tiled_private_decoder_copies_without_expanding() {
    let mut record = gui_tiled_record();
    record.bounds = [0, 0, 3840, 2160];
    record.uv = [0.0, 0.0, 1.0, 1.0];
    let decoded = unsafe { decode_gui_tiled_quads(
        FfiSlice { ptr: &record, count: 1 }, [320, 180], [319.75, 179.5], 0,
    ) }.unwrap();
    record.bounds[2] = 1;
    record.pose[0] = 0.0;
    record.clip[0] = 90;
    assert_eq!(1, decoded.len(), "transport must retain a typed parent, not 8160 Java-style quads");
    assert_eq!([0, 0, 3840, 2160], decoded[0].geometry.bounds);
    assert_eq!(1.0, decoded[0].geometry.pose[0]);
    assert_eq!(Some([1, 2, 100, 50]), decoded[0].clip);
    assert_eq!([319.75, 179.5], decoded[0].projection_extent);
    assert_eq!(11, decoded[0].sequence);
    assert_eq!(8160, crate::render::vulkanic::gui_tiling::tile_segment_count(decoded[0].geometry).unwrap());
    assert_eq!(1, record.bounds[2]);
}

#[test]
fn semantic_gui_tiled_private_decoder_rejects_malformed_records_and_duplicates() {
    let good = gui_tiled_record();
    for bad in [
        FfiGuiTiledQuadRequest { byte_size: 1, ..good },
        FfiGuiTiledQuadRequest { sequence: 0, ..good },
        FfiGuiTiledQuadRequest { asset_id: 0, ..good },
        FfiGuiTiledQuadRequest { tile_extent: [0, 32], ..good },
        FfiGuiTiledQuadRequest { clip_mode: 2, ..good },
        FfiGuiTiledQuadRequest { clip_mode: 0, ..good },
        FfiGuiTiledQuadRequest { clip: [319, 0, 2, 1], ..good },
        FfiGuiTiledQuadRequest { pose: [f32::NAN; 6], ..good },
    ] {
        assert!(unsafe { decode_gui_tiled_quads(FfiSlice { ptr: &bad, count: 1 },
            [320, 180], [320.0, 180.0], 0) }.is_err(), "{bad:?}");
    }
    let duplicate = [good, good];
    assert!(unsafe { decode_gui_tiled_quads(FfiSlice { ptr: duplicate.as_ptr(), count: 2 },
        [320, 180], [320.0, 180.0], 0) }.is_err());
    assert!(unsafe { decode_gui_tiled_quads(FfiSlice { ptr: &good, count: 1 },
        [320, 180], [f32::NAN, 180.0], 0) }.is_err());
}

#[test]
fn semantic_gui_tiled_private_decoder_preflights_aggregate_budget_and_hostile_count() {
    let mut records = [gui_tiled_record(); 5];
    for (index, record) in records.iter_mut().enumerate() {
        record.sequence = index as u64 + 1;
        record.bounds = [0, 0, 4096, 4096];
        record.uv = [0.0, 0.0, 1.0, 1.0];
    }
    assert!(unsafe { decode_gui_tiled_quads(FfiSlice { ptr: records.as_ptr(), count: 4 },
        [320, 180], [320.0, 180.0], 0) }.is_ok());
    assert!(unsafe { decode_gui_tiled_quads(FfiSlice { ptr: records.as_ptr(), count: 4 },
        [320, 180], [320.0, 180.0], 1) }.is_err());
    assert!(unsafe { decode_gui_tiled_quads(FfiSlice { ptr: records.as_ptr(), count: 5 },
        [320, 180], [320.0, 180.0], 0) }.is_err());
    // Must reject the count before attempting to form/read a caller slice.
    assert!(unsafe { decode_gui_tiled_quads(FfiSlice {
        ptr: std::ptr::NonNull::dangling().as_ptr(), count: u64::MAX,
    }, [320, 180], [320.0, 180.0], 0) }.is_err());
}

#[test]
fn semantic_gui_tiled_frame_transport_forwards_owned_parents_and_rejects_collisions() {
    let tile = gui_tiled_record();
    let mut frame = frame_request(&[]);
    frame.tiled_quads = FfiSlice { ptr: &tile, count: 1 };
    frame.gui_projection_width = 319.75;
    frame.gui_projection_height = 179.5;
    let decoded = unsafe { decode_gui_frame_submit_with_tiles(&frame, test_capabilities()) }.unwrap();
    assert_eq!(1, decoded.5.len());
    assert_eq!([319.75, 179.5], decoded.5[0].projection_extent);
    assert!(unsafe { decode_gui_frame_submit_with_mesh(&frame, test_capabilities()) }.is_err(),
        "a legacy consumer must not silently discard typed tiles");
    let mut sprite = sprite_request();
    sprite.sequence = tile.sequence;
    frame.sprites = FfiSlice { ptr: &sprite, count: 1 };
    assert!(unsafe { decode_gui_frame_submit_with_tiles(&frame, test_capabilities()) }.is_err());

    let mut whole = whole_frame_request(&[], &[]);
    whole.gui_tiled_quads = FfiSlice { ptr: &tile, count: 1 };
    let mut capabilities = test_capabilities();
    capabilities.api = BackendApi::Vulkan;
    let decoded = unsafe { decode_whole_frame_submit_with_tiled_gui(&whole, capabilities) }.unwrap();
    assert_eq!(tile.sequence, decoded.9[0].sequence);
    assert!(unsafe { decode_whole_frame_submit_with_gui(&whole, capabilities) }.is_err());
    assert!(unsafe { decode_world_primitive_submit(&whole, capabilities) }.unwrap_err().to_string().contains("does not accept GUI work"));
}

#[test]
fn ffi_barrier_rejects_deprecated_stage_access_bits() {
    let barrier = FfiResourceBarrierAbi {
        byte_size: std::mem::size_of::<FfiResourceBarrierAbi>() as u32,
        resource: FfiHandle {
            raw: Handle::new(HandleKind::Texture, 1, 1).unwrap().raw(),
        },
        has_subresources: 0,
        subresources: FfiTextureSubresourceRange {
            base_mip: 0,
            mip_count: 0,
            base_layer: 0,
            layer_count: 0,
        },
        before: TextureUsageState::TransferDst as u32,
        after: TextureUsageState::ShaderRead as u32,
        stage_bits: 1,
        access_bits: 0,
        src_queue: QueueClass::Graphics as u32,
        dst_queue: QueueClass::Graphics as u32,
    };
    let error = super::submission::decode_barrier(&barrier).unwrap_err();
    assert_eq!(StatusCode::InvalidArgument, error.code);
}

#[test]
fn stable_ffi_rejects_internal_d3_texture_resources_even_when_the_backend_supports_them() {
    let mut capabilities = test_capabilities();
    capabilities.features.texture_3d = true;
    capabilities.features.storage_textures = true;
    capabilities.limits.max_texture_extent_3d = 64;
    let desc = TextureDesc {
        label: "private-volume-through-stable-ffi".to_string(),
        dimension: TextureDimension::D3,
        format: TextureFormat::R8Uint,
        extent: Extent3d {
            width: 8,
            height: 8,
            depth: 8,
        },
        mip_levels: 1,
        array_layers: 1,
        usages: vec![
            TextureUsage::Sampled,
            TextureUsage::Storage,
            TextureUsage::TransferDst,
        ],
    };

    let error = check_texture_capabilities(&desc, capabilities).unwrap_err();
    assert_eq!(StatusCode::UnsupportedFeature, error.code);
    assert!(error.message.contains("internal-only"));
}

fn test_vulkan_capabilities() -> BackendCapabilities {
    BackendCapabilities {
        api: BackendApi::Vulkan,
        name: "ffi-test-vulkan",
        ..test_capabilities()
    }
}

fn sprite_request() -> FfiGuiSpriteRequest {
    FfiGuiSpriteRequest {
        byte_size: size_of::<FfiGuiSpriteRequest>() as u32,
        stratum: 50,
        sprite_id: 1,
        selected_slot: -1,
        progress_fraction: 1.0,
        fill_direction: 0,
        color_argb: 0xffff_ffff,
        x: 10,
        y: 20,
        width: 15,
        height: 15,
        gui_width: 320,
        gui_height: 180,
        sequence: 1,
    }
}

fn affine_quad_request() -> FfiGuiAffineQuadRequest {
    FfiGuiAffineQuadRequest {
        item_raster_layers: FfiSlice::default(),
        item_raster_scale: 0,
        item_raster_corners: [0.0,0.0,16.0,0.0,0.0,16.0],
        material_mode: 0,
        byte_size: size_of::<FfiGuiAffineQuadRequest>() as u32,
        stratum: 50,
        asset_id: 0x4a55_4941,
        x0: 10.0,
        y0: 20.0,
        x1: 18.0,
        y1: 20.0,
        x3: 10.0,
        y3: 28.0,
        z: 0.0,
        u0: 0.0,
        v0: 0.0,
        u1: 1.0,
        v1: 1.0,
        color_argb: 0xff80_40ff,
        gui_width: 320,
        gui_height: 180,
        sequence: 2,
        clip_mode: 0,
        clip_left: 0,
        clip_top: 0,
        clip_width: 0,
        clip_height: 0,
    }
}

fn gui_mesh_vertices() -> [FfiGuiMeshVertex; 3] {
    [
        FfiGuiMeshVertex {
            position: [0.0, 0.0, 0.0],
            atlas_uv: [0.25, 0.25],
            local_uv: [0.0, 0.0],
            color_argb: 0xffff_ffff,
            normal_packed: 0, source_face: 0, source_foil_type: 0,
        },
        FfiGuiMeshVertex {
            position: [1.0, 0.0, 0.0],
            atlas_uv: [0.75, 0.25],
            local_uv: [1.0, 0.0],
            color_argb: 0xffff_ffff,
            normal_packed: 0, source_face: 0, source_foil_type: 0,
        },
        FfiGuiMeshVertex {
            position: [0.0, 1.0, 0.0],
            atlas_uv: [0.25, 0.75],
            local_uv: [0.0, 1.0],
            color_argb: 0xffff_ffff,
            normal_packed: 0, source_face: 0, source_foil_type: 0,
        },
    ]
}

fn gui_mesh_batch_request(
    vertices: &[FfiGuiMeshVertex],
    indices: &[u32],
) -> FfiGuiMeshBatchRequest {
    FfiGuiMeshBatchRequest {
        item_cache_identity: 0,
        item_cache_mode: 0,
        decal_foil_mode: 0,
        decal_model_pose: [0.;16],
        decal_normal_pose: [0.;9],
        block_item_scale: 0,
        block_model_bounds: [0.;6],
        block_item_layout: 0,
        item_raster_scale: 0,
        item_foil_mode: 0,
        item_foil_clock_millis: 0,
        item_foil_speed: 0.0,
        item_foil_strength: 0.0,
        byte_size: size_of::<FfiGuiMeshBatchRequest>() as u32,
        stratum: 420,
        layer_index: 0,
        material_mode: 2,
        lighting_mode: 2,
        asset_id: 7,
        sequence: 9,
        alpha_cutoff: 0.5,
        reserved0: 0,
        model_transform: [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ],
        gui_pose: [1.0, 0.0, 0.0, 1.0, 12.0, 34.0],
        left: 12,
        top: 34,
        right: 28,
        bottom: 50,
        gui_width: 320,
        gui_height: 180,
        render_width: 34,
        render_height: 34,
        guard_pixels: 1,
        clip_mode: 0,
        clip_left: 0,
        clip_top: 0,
        clip_width: 0,
        clip_height: 0,
        vertices: FfiSlice {
            ptr: vertices.as_ptr(),
            count: vertices.len() as u64,
        },
        indices: FfiSlice {
            ptr: indices.as_ptr(),
            count: indices.len() as u64,
        },
    }
}

#[test]
fn gui_layout_exports_cover_whole_frame_sequence_and_clip_fields() {
    let sprite = super::layout::layout_for_struct(44).expect("GUI sprite layout");
    assert_eq!(14, sprite.field_count);
    assert_eq!(
        std::mem::offset_of!(FfiGuiSpriteRequest, sequence) as u32,
        sprite.field_offsets[13]
    );

    let affine = super::layout::layout_for_struct(92).expect("GUI affine layout");
    assert_eq!(27, affine.field_count);
    assert_eq!(152, size_of::<FfiGuiAffineQuadRequest>());
    assert_eq!(136, affine.field_offsets[26]);
    let layer = super::layout::layout_for_struct(107).unwrap();
    assert_eq!(7,layer.field_count);
    assert_eq!(128,size_of::<FfiGuiItemRasterLayer>());
    assert_eq!(60,layer.field_offsets[6]);
    assert_eq!(std::mem::offset_of!(FfiGuiAffineQuadRequest, item_raster_corners) as u32,
        affine.field_offsets[25]);
    assert_eq!(std::mem::offset_of!(FfiGuiAffineQuadRequest, item_raster_scale) as u32,
        affine.field_offsets[24]);
    assert_eq!(std::mem::offset_of!(FfiGuiAffineQuadRequest, material_mode) as u32,
        affine.field_offsets[23]);
    assert_eq!(
        std::mem::offset_of!(FfiGuiAffineQuadRequest, sequence) as u32,
        affine.field_offsets[17]
    );
    assert_eq!(
        std::mem::offset_of!(FfiGuiAffineQuadRequest, clip_height) as u32,
        affine.field_offsets[22]
    );

    let mesh_vertex = super::layout::layout_for_struct(96).expect("GUI mesh vertex layout");
    assert_eq!(7, mesh_vertex.field_count);
    assert_eq!(std::mem::offset_of!(FfiGuiMeshVertex, source_face) as u32, mesh_vertex.field_offsets[5]);
    assert_eq!(std::mem::offset_of!(FfiGuiMeshVertex, source_foil_type) as u32, mesh_vertex.field_offsets[6]);
    assert_eq!(
        std::mem::offset_of!(FfiGuiMeshVertex, local_uv) as u32,
        mesh_vertex.field_offsets[2]
    );
    let mesh_batch = super::layout::layout_for_struct(97).expect("GUI mesh batch layout");
    assert_eq!(40, mesh_batch.field_count);
    assert_eq!(std::mem::offset_of!(FfiGuiMeshBatchRequest, item_cache_identity) as u32, mesh_batch.field_offsets[38]);
    assert_eq!(std::mem::offset_of!(FfiGuiMeshBatchRequest, item_cache_mode) as u32, mesh_batch.field_offsets[39]);
    assert_eq!(std::mem::offset_of!(FfiGuiMeshBatchRequest, block_item_scale) as u32, mesh_batch.field_offsets[35]);
    assert_eq!(std::mem::offset_of!(FfiGuiMeshBatchRequest, block_model_bounds) as u32, mesh_batch.field_offsets[36]);
    assert_eq!(std::mem::offset_of!(FfiGuiMeshBatchRequest, block_item_layout) as u32, mesh_batch.field_offsets[37]);
    assert_eq!(std::mem::offset_of!(FfiGuiMeshBatchRequest, decal_foil_mode) as u32, mesh_batch.field_offsets[32]);
    assert_eq!(std::mem::offset_of!(FfiGuiMeshBatchRequest, decal_model_pose) as u32, mesh_batch.field_offsets[33]);
    assert_eq!(std::mem::offset_of!(FfiGuiMeshBatchRequest, decal_normal_pose) as u32, mesh_batch.field_offsets[34]);
    assert_eq!(std::mem::offset_of!(FfiGuiMeshBatchRequest, item_raster_scale) as u32, mesh_batch.field_offsets[31]);
    assert_eq!(std::mem::offset_of!(FfiGuiMeshBatchRequest, item_foil_mode) as u32, mesh_batch.field_offsets[27]);
    assert_eq!(std::mem::offset_of!(FfiGuiMeshBatchRequest, item_foil_clock_millis) as u32, mesh_batch.field_offsets[28]);
    assert_eq!(std::mem::offset_of!(FfiGuiMeshBatchRequest, item_foil_speed) as u32, mesh_batch.field_offsets[29]);
    assert_eq!(std::mem::offset_of!(FfiGuiMeshBatchRequest, item_foil_strength) as u32, mesh_batch.field_offsets[30]);
    assert_eq!(
        std::mem::offset_of!(FfiGuiMeshBatchRequest, indices) as u32,
        mesh_batch.field_offsets[26]
    );

    let whole_result = super::layout::layout_for_struct(54).expect("whole-frame result layout");
    assert_eq!(49, whole_result.field_count);
    assert_eq!(
        std::mem::offset_of!(FfiWholeFrameSubmitResult, gui_mesh_draw_count) as u32,
        whole_result.field_offsets[41]
    );

    let first_person =
        super::layout::layout_for_struct(98).expect("world first-person frame layout");
    assert_eq!(6, first_person.field_count);
    assert_eq!(
        std::mem::offset_of!(FfiWorldFirstPersonFrame, projection_matrix) as u32,
        first_person.field_offsets[4]
    );
    assert_eq!(
        std::mem::offset_of!(FfiWorldFirstPersonFrame, model_view_matrix) as u32,
        first_person.field_offsets[5]
    );
    let whole_frame = super::layout::layout_for_struct(53).expect("whole-frame layout");
    let texture = super::layout::layout_for_struct(67).expect("mesh texture layout");
    assert_eq!(16, texture.field_count);
    assert_eq!(std::mem::offset_of!(FfiWorldMeshTextureAssetPayload, requested_mip_levels) as u32,
        texture.field_offsets[15]);
    assert_eq!(46, whole_frame.field_count);
    assert_eq!(std::mem::offset_of!(FfiWholeFrameSubmitRequest, world_experience_orbs) as u32,
        whole_frame.field_offsets[45]);
    assert_eq!(std::mem::offset_of!(FfiWholeFrameSubmitRequest, engine_globals_present) as u32,
        whole_frame.field_offsets[37]);
    assert_eq!(std::mem::offset_of!(FfiWholeFrameSubmitRequest, engine_menu_blur_radius) as u32,
        whole_frame.field_offsets[43]);
    assert_eq!(std::mem::offset_of!(FfiWholeFrameSubmitRequest, gui_tiled_quads) as u32,
        whole_frame.field_offsets[36]);
    assert_eq!(std::mem::offset_of!(FfiWholeFrameSubmitRequest, gui_projection_width) as u32,
        whole_frame.field_offsets[34]);
    assert_eq!(std::mem::offset_of!(FfiWholeFrameSubmitRequest, gui_projection_height) as u32,
        whole_frame.field_offsets[35]);
    let gui_frame = super::layout::layout_for_struct(45).unwrap();
    assert_eq!(13, gui_frame.field_count);
    assert_eq!(std::mem::offset_of!(FfiGuiFrameSubmitRequest, tiled_quads) as u32,
        gui_frame.field_offsets[12]);
    let tiled = super::layout::layout_for_struct(101).unwrap();
    assert_eq!(12, tiled.field_count);
    assert_eq!(size_of::<FfiGuiTiledQuadRequest>() as u32, tiled.byte_size);
    assert_eq!(std::mem::offset_of!(FfiGuiTiledQuadRequest, pose) as u32, tiled.field_offsets[6]);
    assert_eq!(std::mem::offset_of!(FfiGuiTiledQuadRequest, sequence) as u32, tiled.field_offsets[9]);
    assert_eq!(std::mem::offset_of!(FfiGuiTiledQuadRequest, clip) as u32, tiled.field_offsets[11]);
    assert_eq!(std::mem::offset_of!(FfiGuiFrameSubmitRequest, gui_projection_width) as u32,
        gui_frame.field_offsets[10]);
    assert_eq!(std::mem::offset_of!(FfiGuiFrameSubmitRequest, gui_projection_height) as u32,
        gui_frame.field_offsets[11]);
    assert_eq!(
        std::mem::offset_of!(
            FfiWholeFrameSubmitRequest,
            world_first_person_mesh_instances
        ) as u32,
        whole_frame.field_offsets[30]
    );
    assert_eq!(
        std::mem::offset_of!(FfiWholeFrameSubmitRequest, gui_blur_before_stratum) as u32,
        whole_frame.field_offsets[31]
    );
    assert_eq!(
        std::mem::offset_of!(FfiWholeFrameSubmitRequest, gui_blur_radius) as u32,
        whole_frame.field_offsets[32]
    );
    assert_eq!(
        std::mem::offset_of!(FfiWholeFrameSubmitRequest, post_effect_id) as u32,
        whole_frame.field_offsets[33]
    );
}

#[test]
fn gui_mesh_decal_transport_copies_poses_and_rejects_noncanonical_or_incomplete_state() {
    let mut vertices = gui_mesh_vertices();
    for vertex in &mut vertices { vertex.normal_packed = 0x007f_0000; }
    let indices = [0_u32,1,2];
    let mut request = gui_mesh_batch_request(&vertices, &indices);
    let decode = |request: &FfiGuiMeshBatchRequest| unsafe {
        super::gui::decode_gui_mesh_batches(FfiSlice { ptr: request, count: 1 },320,180)
    };
    assert!(decode(&request).unwrap()[0].decal_foil.is_none());
    request.decal_normal_pose[0] = -0.0;
    assert!(decode(&request).is_err());
    request.decal_foil_mode = 1;
    request.decal_model_pose = request.model_transform;
    request.decal_normal_pose = [1.,0.,0.,0.,1.,0.,0.,0.,1.];
    assert!(decode(&request).is_err()); // no explicit foil timing/material
    request.item_foil_mode = 1;
    request.item_foil_strength = 0.375;
    request.material_mode = 4;
    request.lighting_mode = 1;
    let copied = decode(&request).unwrap().remove(0);
    assert_eq!(copied.decal_foil.unwrap().model_pose,request.model_transform);
    request.decal_model_pose[12] = 17.;
    request.decal_normal_pose[0] = 2.;
    assert_eq!(copied.decal_foil.unwrap().model_pose[12],0.);
    assert_eq!(copied.decal_foil.unwrap().normal_pose[0],1.);
    super::super::gui_mesh_frontend::prepare_draws(&[copied])
        .expect("decoded poses and valid original vertices must lower without Java UV preparation");
    request.decal_foil_mode = 3;
    assert!(decode(&request).is_err());
    request.decal_foil_mode = 1;
    request.decal_normal_pose = [0.;9];
    assert!(decode(&request).is_err());
    request.decal_normal_pose = [1.,0.,0.,0.,1.,0.,0.,0.,1.];
    request.decal_model_pose[3] = 1.;
    assert!(decode(&request).is_err());
    request.decal_model_pose[3] = 0.;
    request.decal_model_pose[12] = f32::NAN;
    assert!(decode(&request).is_err());
}

#[test]
fn gui_mesh_block_layout_transport_preserves_double_bounds_and_original_normals() {
    let vertices=gui_mesh_vertices();let indices=[0_u32,1,2];
    let mut request=gui_mesh_batch_request(&vertices,&indices);
    let decode=|request: &FfiGuiMeshBatchRequest| unsafe {
        super::gui::decode_gui_mesh_batches(FfiSlice {ptr:request,count:1},320,180)
    };
    assert!(decode(&request).unwrap()[0].block_item_raster.is_none());
    request.block_model_bounds[0]=-0.0;assert!(decode(&request).is_err());
    request.block_item_layout=1;
    request.block_item_scale=2;request.block_model_bounds=[-0.5,-0.5,-0.5,0.5+1e-9,0.5,0.5];
    request.render_width=0;request.render_height=0;request.guard_pixels=0;request.lighting_mode=3;
    let copied=decode(&request).unwrap().remove(0);
    let layout=copied.block_item_raster.unwrap();
    assert_eq!(layout.gui_scale,2);assert_eq!(layout.model_max[0],0.5+1e-9);
    assert!(!layout.oversized_gui);
    request.block_item_layout=2;
    assert!(decode(&request).unwrap()[0].block_item_raster.unwrap().oversized_gui);
    request.block_item_layout=3;assert!(decode(&request).is_err());
    request.block_item_layout=0;assert!(decode(&request).is_err());
    request.block_item_layout=1;
    assert_eq!(copied.vertices[0].normal_packed,vertices[0].normal_packed);
    assert_eq!(copied.model_transform,request.model_transform);
    assert!(copied.item_lighting.is_none());
    request.block_model_bounds[3]=0.75;
    assert_eq!(layout.model_max[0],0.5+1e-9);
    for value in [f64::NAN,f64::INFINITY,-1.0] {
        request.block_model_bounds[3]=value;assert!(decode(&request).is_err());
    }
    request.block_model_bounds[3]=0.5;
    request.block_item_scale=u32::MAX;assert!(decode(&request).is_err());
    request.block_item_scale=2;request.item_raster_scale=2;assert!(decode(&request).is_err());
    request.item_raster_scale=0;request.guard_pixels=1;assert!(decode(&request).is_err());
    request.guard_pixels=0;request.render_width=34;assert!(decode(&request).is_err());
    request.render_width=0;request.lighting_mode=2;assert!(decode(&request).is_err());
}

#[test]
fn gui_mesh_flat_scale_transport_requires_native_layout_and_unresolved_frame_lighting() {
    let vertices = gui_mesh_vertices();
    let indices = [0_u32, 1, 2];
    let mut request = gui_mesh_batch_request(&vertices, &indices);
    request.item_raster_scale = 2;
    request.lighting_mode = 1;
    request.render_width = 0;
    request.render_height = 0;
    request.guard_pixels = 0;
    request.model_transform = super::super::gui_item_raster::GuiItemModelTransform::default().0;
    let decode = |request: &FfiGuiMeshBatchRequest| unsafe {
        super::gui::decode_gui_mesh_batches(FfiSlice { ptr: request, count: 1 }, 320, 180)
    };
    let decoded = decode(&request).unwrap().remove(0);
    assert_eq!(decoded.item_raster_scale, 2);
    assert_eq!(decoded.render_extent, [0, 0]);
    assert!(decoded.item_lighting.is_none());
    assert!(super::super::gui_mesh_frontend::prepare_draws(&[decoded]).is_err());
    request.render_width = 32;
    assert!(decode(&request).is_err());
    request.render_width = 0;
    request.guard_pixels = 1;
    assert!(decode(&request).is_err());
    request.guard_pixels = 0;
    request.item_raster_scale = u32::MAX;
    assert!(decode(&request).is_err());
}

#[test]
fn gui_mesh_foil_transport_preserves_semantics_and_rejects_invalid_modes() {
    let vertices = gui_mesh_vertices();
    let indices = [0_u32, 1, 2];
    let mut request = gui_mesh_batch_request(&vertices, &indices);
    let decode = |request: &FfiGuiMeshBatchRequest| unsafe {
        super::gui::decode_gui_mesh_batches(FfiSlice { ptr: request, count: 1 }, 320, 180)
    };
    assert!(decode(&request).unwrap()[0].item_foil.is_none());
    request.material_mode = 4;
    request.item_foil_mode = 1;
    request.item_foil_clock_millis = 12_345;
    request.item_foil_speed = 0.5;
    request.item_foil_strength = 0.5;
    let copied = decode(&request).unwrap().remove(0);
    assert_eq!(copied.item_foil.unwrap(), super::super::gui_mesh_frontend::GuiItemFoil {
        kind: crate::render::vulkanic::item_foil::StandardFoilKind::Item,
        clock_millis: 12_345, speed: 0.5, strength: 0.5,
    });
    assert_eq!(copied.vertices[0].atlas_uv, vertices[0].atlas_uv);
    request.item_foil_clock_millis = 0;
    assert_eq!(copied.item_foil.unwrap().clock_millis, 12_345);
    for mode in [0, 2, u32::MAX] {
        request.item_foil_mode = mode;
        assert!(decode(&request).is_err());
    }
    request.item_foil_mode = 1;
    request.item_foil_speed = f64::NAN;
    assert!(decode(&request).is_err());
    request.item_foil_speed = 0.5;
    request.item_foil_strength = 1.1;
    assert!(decode(&request).is_err());
    request.item_foil_strength = 0.5;
    request.material_mode = 2;
    assert!(decode(&request).is_err());
}

#[test]
fn gui_inventory_block_lighting_transport_is_explicit_and_closed() {
    let vertices = gui_mesh_vertices();
    let indices = [0_u32, 1, 2];
    let mut request = gui_mesh_batch_request(&vertices, &indices);
    request.lighting_mode = 3;
    let decode = |request: &FfiGuiMeshBatchRequest| unsafe {
        super::gui::decode_gui_mesh_batches(FfiSlice { ptr: request, count: 1 }, 320, 180)
    };
    let decoded = decode(&request).unwrap();
    assert_eq!(decoded[0].lighting_mode,
        super::super::gui_mesh_frontend::GuiMeshLightingMode::InventoryBlock);
    request.lighting_mode = 4;
    assert!(decode(&request).is_err());
}

#[test]
fn gui_mesh_transport_copies_and_rejects_malformed_payloads() {
    let vertices = gui_mesh_vertices();
    let indices = [0_u32, 1, 2];
    let request = gui_mesh_batch_request(&vertices, &indices);
    let decoded = unsafe {
        super::gui::decode_gui_mesh_batches(
            FfiSlice {
                ptr: &request,
                count: 1,
            },
            320,
            180,
        )
    }
    .expect("valid copied GUI mesh batch");
    assert_eq!(1, decoded.len());
    assert_eq!([0.0, 0.0], decoded[0].vertices[0].local_uv);
    assert_eq!(vec![0, 1, 2], decoded[0].indices);

    let mut panorama = gui_mesh_batch_request(&vertices, &indices);
    panorama.material_mode = super::gui::GUI_MESH_MATERIAL_PANORAMA;
    let panorama_decoded = unsafe {
        super::gui::decode_gui_mesh_batches(
            FfiSlice {
                ptr: &panorama,
                count: 1,
            },
            320,
            180,
        )
    }
    .expect("explicit semantic panorama material decodes");
    assert_eq!(
        super::super::gui_mesh_frontend::GuiMeshMaterialMode::Panorama,
        panorama_decoded[0].material_mode,
        "the FFI must preserve the no-cull/no-depth panorama policy rather than coercing it to item geometry",
    );

    let invalid_indices = [0_u32, 1, 3];
    let invalid = gui_mesh_batch_request(&vertices, &invalid_indices);
    let error = unsafe {
        super::gui::decode_gui_mesh_batches(
            FfiSlice {
                ptr: &invalid,
                count: 1,
            },
            320,
            180,
        )
    }
    .expect_err("out-of-range copied GUI mesh index must be rejected");
    assert_eq!(StatusCode::InvalidArgument, error.code);
}

#[test]
fn gui_mesh_transport_rejects_aggregate_payload_before_copying_vertices() {
    let vertices = gui_mesh_vertices();
    let indices = [0_u32, 1, 2];
    let mut batch = gui_mesh_batch_request(&vertices, &indices);
    batch.vertices.count = (GUI_MESH_MAX_VERTICES - 1) as u64;
    batch.indices.count = (GUI_MESH_MAX_INDICES - 3) as u64;
    let batches = vec![batch; 70];
    let error = unsafe {
        super::gui::decode_gui_mesh_batches(
            FfiSlice {
                ptr: batches.as_ptr(),
                count: batches.len() as u64,
            },
            320,
            180,
        )
    }
    .expect_err("aggregate GUI mesh payload must remain bounded");
    assert_eq!(StatusCode::LengthOverflow, error.code);
    assert!(error.message.contains("frame payload"));
}

#[test]
fn gui_frame_mesh_transport_is_owned_and_shares_one_item_sequence() {
    let mut vertices = gui_mesh_vertices();
    let indices = [0_u32, 1, 2];
    let mut batches = vec![gui_mesh_batch_request(&vertices, &indices)];
    let mut request = frame_request(&[]);
    request.mesh_batches = FfiSlice {
        ptr: batches.as_ptr(),
        count: batches.len() as u64,
    };
    let (_, _, sprites, affine, decoded) = unsafe {
        super::gui::decode_gui_frame_submit_with_mesh(&request, test_capabilities()).unwrap()
    };
    assert!(sprites.is_empty());
    assert!(affine.is_empty());
    assert_eq!(1, decoded.len());
    vertices[0].position[0] = 99.0;
    batches[0].layer_index = 17;
    assert_eq!(99.0, vertices[0].position[0]);
    assert_eq!(0.0, decoded[0].vertices[0].position[0]);
    assert_eq!(0, decoded[0].layer_index);
}

fn frame_request(sprites: &[FfiGuiSpriteRequest]) -> FfiGuiFrameSubmitRequest {
    FfiGuiFrameSubmitRequest {
        tiled_quads: FfiSlice { ptr: std::ptr::null(), count: 0 },
        gui_projection_width: 320.0,
        gui_projection_height: 180.0,
        header: FfiHeader {
            version: FFI_ABI_VERSION,
            byte_size: size_of::<FfiGuiFrameSubmitRequest>() as u32,
        },
        generation: 7,
        frame_id: 11,
        frame_target: FfiHandle::from(
            Handle::new(HandleKind::FrameTarget, 3, 1).expect("test handle"),
        ),
        gui_width: 320,
        gui_height: 180,
        sprites: FfiSlice {
            ptr: sprites.as_ptr(),
            count: sprites.len() as u64,
        },
        affine_quads: FfiSlice {
            ptr: std::ptr::null(),
            count: 0,
        },
        negotiated_feature_bits: FfiFeatureBits::GRAPHICS
            | FfiFeatureBits::DESCRIPTOR_ARRAYS
            | FfiFeatureBits::OPTIONAL_BINDINGS
            | FfiFeatureBits::UNIFORM_BUFFERS
            | FfiFeatureBits::STORAGE_BUFFERS
            | FfiFeatureBits::TEXTURE_SUBRESOURCE_COPIES
            | FfiFeatureBits::HOST_BUFFER_ACCESS
            | FfiFeatureBits::PRESENTATION,
        mesh_batches: FfiSlice {
            ptr: std::ptr::null(),
            count: 0,
        },
    }
}

fn line_segment_request() -> FfiWorldLineSegmentRequest {
    FfiWorldLineSegmentRequest {
        byte_size: size_of::<FfiWorldLineSegmentRequest>() as u32,
        stratum: 100,
        style: 1,
        depth_policy: 0,
        color_argb: 0x6600_0000,
        line_width: 1.0,
        start_x: 1.0,
        start_y: 2.0,
        start_z: 3.0,
        end_x: 4.0,
        end_y: 5.0,
        end_z: 6.0,
        viewport_width: 854,
        viewport_height: 480,
    }
}

fn crack_quad_request() -> FfiWorldCrackQuadRequest {
    FfiWorldCrackQuadRequest {
        byte_size: size_of::<FfiWorldCrackQuadRequest>() as u32,
        stratum: 90,
        stage: 4,
        depth_policy: 1,
        blend_policy: 1,
        cull_policy: 0,
        color_argb: 0xffff_ffff,
        reserved0: 0,
        p0_x: 1.0,
        p0_y: 2.0,
        p0_z: 3.0,
        p1_x: 4.0,
        p1_y: 5.0,
        p1_z: 6.0,
        p2_x: 7.0,
        p2_y: 8.0,
        p2_z: 9.0,
        p3_x: 10.0,
        p3_y: 11.0,
        p3_z: 12.0,
        viewport_width: 854,
        viewport_height: 480,
    }
}

fn border_quad_request() -> FfiWorldBorderQuadRequest {
    FfiWorldBorderQuadRequest {
        byte_size: size_of::<FfiWorldBorderQuadRequest>() as u32,
        stratum: 80,
        texture_id: 1,
        depth_policy: 1,
        blend_policy: 1,
        cull_policy: 0,
        color_argb: 0xdd55_ff55,
        reserved0: 0,
        border_size: 8.0,
        distance_to_border: 2.0,
        scroll_u: 0.25,
        scroll_v: 0.25,
        uv_u: 0.0,
        uv_v: 0.0,
        uv_width: 1.0,
        uv_height: -4.0,
        p0_x: -1.0,
        p0_y: -2.0,
        p0_z: -3.0,
        p1_x: 1.0,
        p1_y: -2.0,
        p1_z: -3.0,
        p2_x: 1.0,
        p2_y: 2.0,
        p2_z: -3.0,
        p3_x: -1.0,
        p3_y: 2.0,
        p3_z: -3.0,
        viewport_width: 854,
        viewport_height: 480,
    }
}

fn material_quad_request() -> FfiWorldMaterialQuadRequest {
    FfiWorldMaterialQuadRequest {
        byte_size: size_of::<FfiWorldMaterialQuadRequest>() as u32,
        stratum: WORLD_STRATUM_OPAQUE_TEXTURED_GEOMETRY,
        material_id: WORLD_MATERIAL_ID_OPAQUE_TEXTURED,
        texture_id: WORLD_MATERIAL_TEXTURE_STONE,
        material_mode: WORLD_MATERIAL_MODE_OPAQUE,
        depth_policy: WORLD_DEPTH_POLICY_TEST_WRITE,
        cull_policy: WORLD_CULL_BACK,
        topology: WORLD_TOPOLOGY_TRIANGLES,
        color_argb: 0xffff_ffff,
        winding: 0,
        p0_x: -1.0,
        p0_y: -1.0,
        p0_z: -2.0,
        p1_x: 1.0,
        p1_y: -1.0,
        p1_z: -2.0,
        p2_x: 1.0,
        p2_y: 1.0,
        p2_z: -2.0,
        p3_x: -1.0,
        p3_y: 1.0,
        p3_z: -2.0,
        uv0_u: 0.0,
        uv0_v: 0.0,
        uv1_u: 1.0,
        uv1_v: 0.0,
        uv2_u: 1.0,
        uv2_v: 1.0,
        uv3_u: 0.0,
        uv3_v: 1.0,
        source_program: WORLD_MATERIAL_SOURCE_UNSPECIFIED,
        source_color_argb: 0xffff_ffff,
        packed_light: 0,
        source_uv_space: WORLD_MATERIAL_SOURCE_UV_LOCAL_TEXTURE,
        viewport_width: 854,
        viewport_height: 480,
        vertex0_color_argb: 0xffff_ffff,
        vertex1_color_argb: 0xffff_ffff,
        vertex2_color_argb: 0xffff_ffff,
        vertex3_color_argb: 0xffff_ffff,
        vertex0_packed_light: 0,
        vertex1_packed_light: 0,
        vertex2_packed_light: 0,
        vertex3_packed_light: 0,
        block_entity_id: -1,
    }
}

fn material_table_record() -> FfiWorldMaterialTableRecord {
    FfiWorldMaterialTableRecord {
        byte_size: size_of::<FfiWorldMaterialTableRecord>() as u32,
        stratum: WORLD_STRATUM_OPAQUE_TEXTURED_GEOMETRY,
        material_id: WORLD_MATERIAL_ID_OPAQUE_TEXTURED,
        texture_id: WORLD_MATERIAL_TEXTURE_STONE,
        material_mode: WORLD_MATERIAL_MODE_OPAQUE,
        depth_policy: WORLD_DEPTH_POLICY_TEST_WRITE,
        cull_policy: WORLD_CULL_BACK,
        topology: WORLD_TOPOLOGY_TRIANGLES,
        winding: WORLD_WINDING_CCW,
        source_program: WORLD_MATERIAL_SOURCE_UNSPECIFIED,
    }
}

fn compact_material_quad_request() -> FfiWorldMaterialCompactQuadRequest {
    FfiWorldMaterialCompactQuadRequest {
        byte_size: size_of::<FfiWorldMaterialCompactQuadRequest>() as u32,
        material_index: 0,
        color_argb: 0xffff_ffff,
        source_uv_space: WORLD_MATERIAL_SOURCE_UV_LOCAL_TEXTURE,
        p0_x: -1.0,
        p0_y: -1.0,
        p0_z: -2.0,
        p1_x: 1.0,
        p1_y: -1.0,
        p1_z: -2.0,
        p2_x: 1.0,
        p2_y: 1.0,
        p2_z: -2.0,
        p3_x: -1.0,
        p3_y: 1.0,
        p3_z: -2.0,
        uv0_u: 0.0,
        uv0_v: 0.0,
        uv1_u: 1.0,
        uv1_v: 0.0,
        uv2_u: 1.0,
        uv2_v: 1.0,
        uv3_u: 0.0,
        uv3_v: 1.0,
        source_color_argb: 0xffff_ffff,
        packed_light: 0,
        block_entity_id: -1,
    }
}

fn whole_frame_request(
    segments: &[FfiWorldLineSegmentRequest],
    sprites: &[FfiGuiSpriteRequest],
) -> FfiWholeFrameSubmitRequest {
    let mut view_matrix = [0.0; 16];
    let mut projection_matrix = [0.0; 16];
    view_matrix[0] = 1.0;
    view_matrix[5] = 1.0;
    view_matrix[10] = 1.0;
    view_matrix[15] = 1.0;
    projection_matrix[0] = 1.0;
    projection_matrix[5] = 1.0;
    projection_matrix[10] = 1.0;
    projection_matrix[15] = 1.0;
    FfiWholeFrameSubmitRequest {
        gui_tiled_quads: FfiSlice { ptr: std::ptr::null(), count: 0 },
        gui_projection_width: 320.0,
        gui_projection_height: 180.0,
        header: FfiHeader {
            version: FFI_ABI_VERSION,
            byte_size: size_of::<FfiWholeFrameSubmitRequest>() as u32,
        },
        generation: 7,
        frame_id: 11,
        correlation_id: 13,
        frame_target: FfiHandle::from(
            Handle::new(HandleKind::FrameTarget, 3, 1).expect("test handle"),
        ),
        gui_width: 320,
        gui_height: 180,
        viewport_width: 854,
        viewport_height: 480,
        view_matrix,
        projection_matrix,
        world_background: FfiWorldBackgroundRequest {
            byte_size: size_of::<FfiWorldBackgroundRequest>() as u32,
            enabled: 1,
            sky_type: WORLD_BACKGROUND_SKY_OVERWORLD,
            load_intent: WORLD_BACKGROUND_LOAD_CLEAR,
            store_intent: WORLD_BACKGROUND_STORE_STORE,
            color_argb: 0xff102844,
            viewport_width: 854,
            viewport_height: 480,
            sky_visible: 0,
            sky_sunrise_or_sunset: 0,
            sky_dark_disc: 0,
            sky_reserved0: 0,
            sky_sun_angle: 0.0,
            sky_time_of_day: 0.0,
            sky_rain_brightness: 0.0,
            sky_star_brightness: 0.0,
            sky_sunrise_and_sunset_color_argb: 0,
            sky_moon_phase: 0,
            sky_end_flash_intensity: 0.0,
            sky_end_flash_x_angle: 0.0,
            sky_end_flash_y_angle: 0.0,
            sky_color_argb: 0,
        },
        world_segments: FfiSlice {
            ptr: segments.as_ptr(),
            count: segments.len() as u64,
        },
        world_crack_quads: FfiSlice {
            ptr: std::ptr::null(),
            count: 0,
        },
        world_border_quads: FfiSlice {
            ptr: std::ptr::null(),
            count: 0,
        },
        world_material_quads: FfiSlice {
            ptr: std::ptr::null(),
            count: 0,
        },
        world_material_table: FfiSlice {
            ptr: std::ptr::null(),
            count: 0,
        },
        world_material_compact_quads: FfiSlice {
            ptr: std::ptr::null(),
            count: 0,
        },
        world_mesh_instances: FfiSlice {
            ptr: std::ptr::null(),
            count: 0,
        },
        world_text_quads: FfiSlice {
            ptr: std::ptr::null(),
            count: 0,
        },
        gui_sprites: FfiSlice {
            ptr: sprites.as_ptr(),
            count: sprites.len() as u64,
        },
        gui_affine_quads: FfiSlice {
            ptr: std::ptr::null(),
            count: 0,
        },
        negotiated_feature_bits: FfiFeatureBits::GRAPHICS
            | FfiFeatureBits::DESCRIPTOR_ARRAYS
            | FfiFeatureBits::OPTIONAL_BINDINGS
            | FfiFeatureBits::UNIFORM_BUFFERS
            | FfiFeatureBits::STORAGE_BUFFERS
            | FfiFeatureBits::TEXTURE_SUBRESOURCE_COPIES
            | FfiFeatureBits::HOST_BUFFER_ACCESS
            | FfiFeatureBits::PRESENTATION,
        voxel_volume_frame: FfiWorldVoxelVolumeFrame {
            byte_size: size_of::<FfiWorldVoxelVolumeFrame>() as u32,
            enabled: 0,
            reserved0: 0,
            reserved1: 0,
            world_generation: 0,
            resource_generation: 0,
            camera_x: 0.0,
            camera_y: 0.0,
            camera_z: 0.0,
            reserved2: 0,
        },
        shader_environment_frame: FfiWorldShaderEnvironmentFrame {
            byte_size: size_of::<FfiWorldShaderEnvironmentFrame>() as u32,
            enabled: 0,
            frame_counter: 0,
            world_day: 0,
            world_generation: 0,
            world_time: 0,
            frame_time_seconds: 0.0,
            frame_time_counter: 0.0,
            time_of_day: 0.0,
            rain_strength: 0.0,
            thunder_strength: 0.0,
            sky_darken: 0.0,
            moon_phase: 0,
            eye_submersion: 0,
            screen_brightness: 0.0,
            far_plane: 0.0,
            relative_eye_x: 0.0,
            relative_eye_y: 0.0,
            relative_eye_z: 0.0,
            sky_color_r: 0.0,
            sky_color_g: 0.0,
            sky_color_b: 0.0,
            darkness_light_factor: 0.0,
            night_vision: 0.0,
            fog_color_r: 0.0,
            fog_color_g: 0.0,
            fog_color_b: 0.0,
            biome_precipitation: 0,
            biome_resource_location_utf8: FfiBytes {
                ptr: core::ptr::null(),
                len: 0,
            },
            main_hand_item_model_resource_location_utf8: FfiBytes {
                ptr: core::ptr::null(),
                len: 0,
            },
            off_hand_item_model_resource_location_utf8: FfiBytes {
                ptr: core::ptr::null(),
                len: 0,
            },
            main_hand_item_light_emission: 0,
            off_hand_item_light_emission: 0,
            lightmap_enabled: 0,
            lightmap_reserved: 0,
            lightmap_generation: 0,
            lightmap_ambient_light_factor: 0.0,
            lightmap_sky_factor: 0.0,
            lightmap_block_factor: 0.0,
            lightmap_night_vision_factor: 0.0,
            lightmap_darkness_scale: 0.0,
            lightmap_darken_world_factor: 0.0,
            lightmap_brightness_factor: 0.0,
            lightmap_sky_light_r: 0.0,
            lightmap_sky_light_g: 0.0,
            lightmap_sky_light_b: 0.0,
            lightmap_ambient_r: 0.0,
            lightmap_ambient_g: 0.0,
            lightmap_ambient_b: 0.0,
            blindness: 0.0,
            darkness_factor: 0.0,
            eye_brightness_block: 0,
            eye_brightness_sky: 0,
            fog_parameter_color_r: 0.0,
            fog_parameter_color_g: 0.0,
            fog_parameter_color_b: 0.0,
            fog_parameter_color_a: 0.0,
            fog_environmental_start: 0.0,
            fog_environmental_end: 0.0,
            fog_render_distance_start: 0.0,
            fog_render_distance_end: 0.0,
            fog_sky_end: 0.0,
            fog_clouds_end: 0.0,
            distant_horizons_render_distance: 0,
        },
        world_feature_coverage: FfiWorldFeatureCoverage {
            byte_size: size_of::<FfiWorldFeatureCoverage>() as u32,
            ..FfiWorldFeatureCoverage::default()
        },
        world_first_person_frame: FfiWorldFirstPersonFrame {
            byte_size: size_of::<FfiWorldFirstPersonFrame>() as u32,
            enabled: 0,
            clear_depth_before: 0,
            main_hand_instance_count: 0,
            projection_matrix: [0.0; 16],
            model_view_matrix: [0.0; 16],
        },
        world_first_person_mesh_instances: FfiSlice {
            ptr: std::ptr::null(),
            count: 0,
        },
        gui_blur_before_stratum: -1,
        gui_blur_radius: -1,
        engine_globals_present: 0,
        engine_screen_width: 0,
        engine_screen_height: 0,
        engine_game_ticks: 0,
        engine_partial_tick: 0.0,
        engine_glint_alpha: 0.0,
        engine_menu_blur_radius: 0,
        world_particle_quads: FfiSlice { ptr: std::ptr::null(), count: 0 },
        world_experience_orbs: FfiSlice { ptr: std::ptr::null(), count: 0 },
        world_lod_instances: FfiSlice {
            ptr: std::ptr::null(),
            count: 0,
        },
        world_lod_render_frame: FfiWorldLodRenderFrame {
            byte_size: size_of::<FfiWorldLodRenderFrame>() as u32,
            enabled: 0,
            flags: 0,
            world_y_offset: 0,
            combined_matrix: [0.0; 16],
            model_view_matrix: [0.0; 16],
            projection_matrix: [0.0; 16],
            projection_inverse_matrix: [0.0; 16],
            clip_distance: 0.0,
            micro_offset: 0.0,
            noise_intensity: 0.0,
            earth_radius: 0.0,
            noise_steps: 0,
            noise_dropoff: 0,
            reserved0: 0,
            camera_world_x: 0.0,
            camera_world_y: 0.0,
            camera_world_z: 0.0,
        },
        gui_mesh_batches: FfiSlice {
            ptr: std::ptr::null(),
            count: 0,
        },
        post_effect_id: FfiBytes {
            ptr: std::ptr::null(),
            len: 0,
        },
    }
}

fn whole_frame_request_with_borders(
    borders: &[FfiWorldBorderQuadRequest],
    sprites: &[FfiGuiSpriteRequest],
) -> FfiWholeFrameSubmitRequest {
    let mut request = whole_frame_request(&[], sprites);
    request.world_border_quads = FfiSlice {
        ptr: borders.as_ptr(),
        count: borders.len() as u64,
    };
    request
}

fn whole_frame_request_with_cracks(
    segments: &[FfiWorldLineSegmentRequest],
    cracks: &[FfiWorldCrackQuadRequest],
    sprites: &[FfiGuiSpriteRequest],
) -> FfiWholeFrameSubmitRequest {
    let mut request = whole_frame_request(segments, sprites);
    request.world_crack_quads = FfiSlice {
        ptr: cracks.as_ptr(),
        count: cracks.len() as u64,
    };
    request
}

fn whole_frame_request_with_materials(
    materials: &[FfiWorldMaterialQuadRequest],
) -> FfiWholeFrameSubmitRequest {
    let mut request = whole_frame_request(&[], &[]);
    request.world_material_quads = FfiSlice {
        ptr: materials.as_ptr(),
        count: materials.len() as u64,
    };
    request
}

fn whole_frame_request_with_compact_materials(
    table: &[FfiWorldMaterialTableRecord],
    materials: &[FfiWorldMaterialCompactQuadRequest],
) -> FfiWholeFrameSubmitRequest {
    let mut request = whole_frame_request(&[], &[]);
    request.world_material_table = FfiSlice {
        ptr: table.as_ptr(),
        count: table.len() as u64,
    };
    request.world_material_compact_quads = FfiSlice {
        ptr: materials.as_ptr(),
        count: materials.len() as u64,
    };
    request
}

fn whole_frame_request_with_mesh_instances(
    instances: &[FfiWorldMeshInstanceRecord],
) -> FfiWholeFrameSubmitRequest {
    let mut request = whole_frame_request(&[], &[]);
    request.world_mesh_instances = FfiSlice {
        ptr: instances.as_ptr(),
        count: instances.len() as u64,
    };
    request
}

fn asset_update_request(assets: &[FfiGuiAssetPayload]) -> FfiGuiAssetUpdateRequest {
    FfiGuiAssetUpdateRequest {
        header: FfiHeader {
            version: FFI_ABI_VERSION,
            byte_size: size_of::<FfiGuiAssetUpdateRequest>() as u32,
        },
        generation: 9,
        assets: FfiSlice {
            ptr: assets.as_ptr(),
            count: assets.len() as u64,
        },
        negotiated_feature_bits: FfiFeatureBits::GRAPHICS
            | FfiFeatureBits::DESCRIPTOR_ARRAYS
            | FfiFeatureBits::OPTIONAL_BINDINGS
            | FfiFeatureBits::UNIFORM_BUFFERS
            | FfiFeatureBits::STORAGE_BUFFERS
            | FfiFeatureBits::TEXTURE_SUBRESOURCE_COPIES
            | FfiFeatureBits::HOST_BUFFER_ACCESS
            | FfiFeatureBits::PRESENTATION,
    }
}

fn world_border_asset_update_request(bytes: &[u8]) -> FfiWorldBorderAssetUpdateRequest {
    FfiWorldBorderAssetUpdateRequest {
        header: FfiHeader {
            version: FFI_ABI_VERSION,
            byte_size: size_of::<FfiWorldBorderAssetUpdateRequest>() as u32,
        },
        generation: 9,
        texture_id: 1,
        reserved0: 0,
        png_bytes: FfiBytes {
            ptr: bytes.as_ptr(),
            len: bytes.len() as u64,
        },
        negotiated_feature_bits: FfiFeatureBits::GRAPHICS
            | FfiFeatureBits::DESCRIPTOR_ARRAYS
            | FfiFeatureBits::OPTIONAL_BINDINGS
            | FfiFeatureBits::UNIFORM_BUFFERS
            | FfiFeatureBits::STORAGE_BUFFERS
            | FfiFeatureBits::TEXTURE_SUBRESOURCE_COPIES
            | FfiFeatureBits::HOST_BUFFER_ACCESS
            | FfiFeatureBits::PRESENTATION,
    }
}

fn world_crack_asset_update_request(
    assets: &[FfiWorldCrackAssetPayload],
) -> FfiWorldCrackAssetUpdateRequest {
    FfiWorldCrackAssetUpdateRequest {
        header: FfiHeader {
            version: FFI_ABI_VERSION,
            byte_size: size_of::<FfiWorldCrackAssetUpdateRequest>() as u32,
        },
        generation: 9,
        assets: FfiSlice {
            ptr: assets.as_ptr(),
            count: assets.len() as u64,
        },
        negotiated_feature_bits: FfiFeatureBits::GRAPHICS
            | FfiFeatureBits::DESCRIPTOR_ARRAYS
            | FfiFeatureBits::OPTIONAL_BINDINGS
            | FfiFeatureBits::UNIFORM_BUFFERS
            | FfiFeatureBits::STORAGE_BUFFERS
            | FfiFeatureBits::TEXTURE_SUBRESOURCE_COPIES
            | FfiFeatureBits::HOST_BUFFER_ACCESS
            | FfiFeatureBits::PRESENTATION,
    }
}

fn world_material_asset_update_request(
    assets: &[FfiWorldMaterialAssetPayload],
) -> FfiWorldMaterialAssetUpdateRequest {
    FfiWorldMaterialAssetUpdateRequest {
        header: FfiHeader {
            version: FFI_ABI_VERSION,
            byte_size: size_of::<FfiWorldMaterialAssetUpdateRequest>() as u32,
        },
        generation: 9,
        assets: FfiSlice {
            ptr: assets.as_ptr(),
            count: assets.len() as u64,
        },
        negotiated_feature_bits: FfiFeatureBits::GRAPHICS
            | FfiFeatureBits::DESCRIPTOR_ARRAYS
            | FfiFeatureBits::OPTIONAL_BINDINGS
            | FfiFeatureBits::UNIFORM_BUFFERS
            | FfiFeatureBits::STORAGE_BUFFERS
            | FfiFeatureBits::TEXTURE_SUBRESOURCE_COPIES
            | FfiFeatureBits::HOST_BUFFER_ACCESS
            | FfiFeatureBits::PRESENTATION,
    }
}

fn shader_pack_source_update_request(
    pack_name: &[u8],
    files: &[FfiShaderPackSourceFile],
) -> FfiShaderPackSourceUpdateRequest {
    FfiShaderPackSourceUpdateRequest {
        header: FfiHeader {
            version: FFI_ABI_VERSION,
            byte_size: size_of::<FfiShaderPackSourceUpdateRequest>() as u32,
        },
        generation: 9,
        pack_name_utf8: FfiBytes {
            ptr: pack_name.as_ptr(),
            len: pack_name.len() as u64,
        },
        files: FfiSlice {
            ptr: files.as_ptr(),
            count: files.len() as u64,
        },
    }
}

fn shader_pack_asset_update_request(
    pack_name: &[u8],
    files: &[FfiShaderPackAssetFile],
) -> FfiShaderPackAssetUpdateRequest {
    FfiShaderPackAssetUpdateRequest {
        header: FfiHeader {
            version: FFI_ABI_VERSION,
            byte_size: size_of::<FfiShaderPackAssetUpdateRequest>() as u32,
        },
        generation: 9,
        pack_name_utf8: FfiBytes {
            ptr: pack_name.as_ptr(),
            len: pack_name.len() as u64,
        },
        files: FfiSlice {
            ptr: files.as_ptr(),
            count: files.len() as u64,
        },
    }
}

fn world_mesh_asset_update_request(
    meshes: &[FfiWorldMeshAssetRecord],
    textures: &[FfiWorldMeshTextureAssetPayload],
) -> FfiWorldMeshAssetUpdateRequest {
    FfiWorldMeshAssetUpdateRequest {
        header: FfiHeader {
            version: FFI_ABI_VERSION,
            byte_size: size_of::<FfiWorldMeshAssetUpdateRequest>() as u32,
        },
        generation: 9,
        meshes: FfiSlice {
            ptr: meshes.as_ptr(),
            count: meshes.len() as u64,
        },
        textures: FfiSlice {
            ptr: textures.as_ptr(),
            count: textures.len() as u64,
        },
        sorted_indices: FfiSlice {
            ptr: std::ptr::null(),
            count: 0,
        },
        negotiated_feature_bits: FfiFeatureBits::GRAPHICS
            | FfiFeatureBits::DESCRIPTOR_ARRAYS
            | FfiFeatureBits::OPTIONAL_BINDINGS
            | FfiFeatureBits::UNIFORM_BUFFERS
            | FfiFeatureBits::STORAGE_BUFFERS
            | FfiFeatureBits::TEXTURE_SUBRESOURCE_COPIES
            | FfiFeatureBits::HOST_BUFFER_ACCESS
            | FfiFeatureBits::PRESENTATION,
        retirements: FfiSlice {
            ptr: std::ptr::null(),
            count: 0,
        },
        experience_orbs: FfiSlice { ptr: std::ptr::null(), count: 0 },
    }
}

fn orb_asset() -> FfiWorldExperienceOrbAssetRecord {
    FfiWorldExperienceOrbAssetRecord {
        byte_size: size_of::<FfiWorldExperienceOrbAssetRecord>() as u32,
        icon: 10, mesh_key: 0x0b01, mesh_generation: 2,
        red: 127, blue: 5, packed_light: 0x00b00070, reserved0: 0,
    }
}

#[test]
fn experience_orb_asset_transport_lowers_copied_appearance_without_caller_geometry() {
    let mut orb = orb_asset();
    let mut request = world_mesh_asset_update_request(&[], &[]);
    request.experience_orbs = FfiSlice { ptr: &orb, count: 1 };
    let (_, meshes, textures, sorted, retirements) = unsafe {
        decode_world_mesh_asset_update(&request, test_capabilities()).unwrap()
    };
    orb.red = 0;
    assert_eq!(orb.red, 0);
    assert!(textures.is_empty() && sorted.is_empty() && retirements.is_empty());
    assert_eq!(meshes.len(), 1);
    let mesh = &meshes[0];
    assert_eq!((mesh.mesh_key, mesh.mesh_generation), (0x0b01,2));
    assert_eq!(mesh.vertices[0].color_argb, 0x807fff05);
    assert_eq!(mesh.vertices[0].normal_packed, 0x7f00);
    assert_eq!(mesh.vertices[0].light, 0x00b00070);
    assert_eq!(mesh.vertices[0].uv, [0.5,0.75]);
    assert_eq!(mesh.vertices.len(), 4);
    assert_eq!(mesh.index_bytes.len(), 12);
    assert_eq!(mesh.entity_identity, "minecraft:experience_orb");
    let layout = super::layout::layout_for_struct(109).unwrap();
    assert_eq!(layout.byte_size, 40);
    assert_eq!(&layout.field_offsets[..8], &[0,4,8,16,24,28,32,36]);
    let update_layout = super::layout::layout_for_struct(68).unwrap();
    assert_eq!(update_layout.field_count, 8);
    let empty = world_mesh_asset_update_request(&[], &[]);
    assert_eq!(super::status::input_bytes_for_world_mesh_asset_update(&request)
        - super::status::input_bytes_for_world_mesh_asset_update(&empty), 40);
}

#[test]
fn experience_orb_asset_transport_rejects_invalid_and_duplicate_semantics() {
    let good = orb_asset();
    for bad in [
        FfiWorldExperienceOrbAssetRecord {byte_size:0,..good},
        FfiWorldExperienceOrbAssetRecord {mesh_key:0,..good},
        FfiWorldExperienceOrbAssetRecord {mesh_generation:0,..good},
        FfiWorldExperienceOrbAssetRecord {icon:11,..good},
        FfiWorldExperienceOrbAssetRecord {red:256,..good},
        FfiWorldExperienceOrbAssetRecord {blue:u32::MAX,..good},
        FfiWorldExperienceOrbAssetRecord {reserved0:1,..good},
    ] {
        let mut request = world_mesh_asset_update_request(&[], &[]);
        request.experience_orbs = FfiSlice { ptr:&bad, count:1 };
        assert!(unsafe {decode_world_mesh_asset_update(&request, test_capabilities())}.is_err());
    }
    let duplicates = [good,good];
    let mut request = world_mesh_asset_update_request(&[], &[]);
    request.experience_orbs = FfiSlice { ptr:duplicates.as_ptr(), count:2 };
    assert!(unsafe {decode_world_mesh_asset_update(&request, test_capabilities())}.is_err());
}

#[test]
fn experience_orb_asset_transport_checks_residency_before_reading_array() {
    let mut request = world_mesh_asset_update_request(&[], &[]);
    request.experience_orbs = FfiSlice { ptr:std::ptr::null(), count:u64::MAX };
    let error = unsafe {decode_world_mesh_asset_update(&request, test_capabilities())}.unwrap_err();
    assert!(error.message.contains("residency"));
}

#[test]
fn experience_orb_asset_transport_rejects_header_only_old_or_short_requests() {
    for header in [
        FfiHeader { version:52, byte_size:96 },
        FfiHeader { version:FFI_ABI_VERSION, byte_size:8 },
    ] {
        let pointer = (&header as *const FfiHeader).cast::<FfiWorldMeshAssetUpdateRequest>();
        assert!(unsafe {decode_world_mesh_asset_update(pointer, test_capabilities())}.is_err());
    }
}

#[test]
fn experience_orb_asset_export_preserves_generation_and_rejects_old_header() {
    let create = FfiContextCreateRequest {
        header: FfiHeader { version:FFI_ABI_VERSION, byte_size:size_of::<FfiContextCreateRequest>() as u32 },
        backend_kind:1, tracy_enabled:0,
        label: FfiBytes { ptr:b"orb ABI".as_ptr(), len:7 },
    };
    let mut context = FfiContextResult::default();
    assert_eq!(unsafe {mattmc_vulkanic_gal_context_create(&create, &mut context)}, 0);
    let mut request = world_mesh_asset_update_request(&[], &[]);
    request.negotiated_feature_bits = context.supported_feature_bits;
    let orb = orb_asset();
    request.experience_orbs = FfiSlice { ptr:&orb, count:1 };
    let publish = |request: &FfiWorldMeshAssetUpdateRequest| unsafe {
        mattmc_vulkanic_gal_world_mesh_update_assets(context.context_id, request, ptr::null_mut())
    };
    assert_eq!(publish(&request), 0);
    assert_ne!(publish(&request), 0, "stale update must not be accepted");
    let header = FfiHeader { version:52, byte_size:96 };
    assert_ne!(unsafe {mattmc_vulkanic_gal_world_mesh_update_assets(context.context_id,
        (&header as *const FfiHeader).cast(), ptr::null_mut())}, 0);
    request.generation += 1;
    let replacement = FfiWorldExperienceOrbAssetRecord {mesh_generation:3, red:200,..orb};
    request.experience_orbs = FfiSlice { ptr:&replacement, count:1 };
    assert_eq!(publish(&request), 0, "rejection must leave context usable");
    request.generation += 1;
    request.experience_orbs = FfiSlice { ptr:ptr::null(), count:0 };
    let retirement = FfiWorldMeshAssetRetirementRecord {
        byte_size:size_of::<FfiWorldMeshAssetRetirementRecord>() as u32, reserved0:0,
        mesh_key:orb.mesh_key, mesh_generation:3,
    };
    request.retirements = FfiSlice { ptr:&retirement, count:1 };
    assert_eq!(publish(&request), 0);
    assert_eq!(unsafe {mattmc_vulkanic_gal_context_destroy(context.context_id, ptr::null_mut())}, 0);
}

fn world_lod_asset_update_request(
    assets: &[FfiWorldLodColumnAssetRecord],
    retirements: &[FfiWorldLodColumnRetirementRecord],
) -> FfiWorldLodAssetUpdateRequest {
    FfiWorldLodAssetUpdateRequest {
        header: FfiHeader {
            version: FFI_ABI_VERSION,
            byte_size: size_of::<FfiWorldLodAssetUpdateRequest>() as u32,
        },
        generation: 17,
        assets: FfiSlice {
            ptr: assets.as_ptr(),
            count: assets.len() as u64,
        },
        retirements: FfiSlice {
            ptr: retirements.as_ptr(),
            count: retirements.len() as u64,
        },
        negotiated_feature_bits: FfiFeatureBits::GRAPHICS
            | FfiFeatureBits::DESCRIPTOR_ARRAYS
            | FfiFeatureBits::OPTIONAL_BINDINGS
            | FfiFeatureBits::UNIFORM_BUFFERS
            | FfiFeatureBits::STORAGE_BUFFERS
            | FfiFeatureBits::TEXTURE_SUBRESOURCE_COPIES
            | FfiFeatureBits::HOST_BUFFER_ACCESS
            | FfiFeatureBits::PRESENTATION,
        material_provenance: FfiSlice {
            ptr: std::ptr::null(),
            count: 0,
        },
    }
}

fn lod_vertex() -> FfiWorldLodVertex {
    FfiWorldLodVertex {
        byte_size: size_of::<FfiWorldLodVertex>() as u32,
        local_x: 12,
        local_y: 34,
        local_z: 56,
        packed_light_and_micro_offset: 0xa57b,
        color_rgba: u32::from_le_bytes([10, 20, 30, 40]),
        material_id: 7,
        normal_index: 5,
    }
}

fn mesh_vertex() -> FfiWorldMeshVertex {
    FfiWorldMeshVertex {
        byte_size: size_of::<FfiWorldMeshVertex>() as u32,
        color_argb: 0xffff_ffff,
        normal_packed: 0,
        light: 0xf000_f000,
        x: 0.0,
        y: 0.0,
        z: -1.0,
        u: 0.0,
        v: 0.0,
        atlas_u: 0.25,
        atlas_v: 0.5,
        shader_block_id: 10232,
        shader_material_type: -1,
        terrain_material_bits: 0,
        mid_block_packed: 0x0020_2020,
    }
}

fn mesh_section(texture_id: u32) -> FfiWorldMeshSectionRecord {
    FfiWorldMeshSectionRecord {
        byte_size: size_of::<FfiWorldMeshSectionRecord>() as u32,
        material_id: WORLD_MATERIAL_ID_OPAQUE_TEXTURED,
        texture_id,
        material_mode: WORLD_MATERIAL_MODE_OPAQUE,
        cull_policy: WORLD_CULL_BACK,
        winding: WORLD_WINDING_CCW,
        index_offset: 0,
        index_count: 3,
    }
}

fn mesh_asset<'a>(
    vertices: &'a [FfiWorldMeshVertex],
    index_bytes: &'a [u8],
    sections: &'a [FfiWorldMeshSectionRecord],
) -> FfiWorldMeshAssetRecord {
    FfiWorldMeshAssetRecord {
        byte_size: size_of::<FfiWorldMeshAssetRecord>() as u32,
        vertex_layout_version: 1,
        index_type: IndexType::U16 as u32,
        reserved0: 0,
        mesh_key: 44,
        mesh_generation: 9,
        vertices: FfiSlice {
            ptr: vertices.as_ptr(),
            count: vertices.len() as u64,
        },
        index_bytes: FfiBytes {
            ptr: index_bytes.as_ptr(),
            len: index_bytes.len() as u64,
        },
        sections: FfiSlice {
            ptr: sections.as_ptr(),
            count: sections.len() as u64,
        },
        entity_identity_utf8: FfiBytes {
            ptr: core::ptr::null(),
            len: 0,
        },
    }
}

fn mesh_instance() -> FfiWorldMeshInstanceRecord {
    FfiWorldMeshInstanceRecord {
        item_foil_mode: 0, item_foil_clock_millis: 0, item_foil_speed: 0.0, item_foil_strength: 0.0,
        terrain_placement_mode: 0, terrain_origin: [0;3], terrain_camera: [0.0;3],
        byte_size: size_of::<FfiWorldMeshInstanceRecord>() as u32,
        stratum: WORLD_STRATUM_OPAQUE_TEXTURED_GEOMETRY,
        mesh_section_index: 0,
        depth_policy: WORLD_DEPTH_POLICY_TEST_WRITE,
        cull_policy: WORLD_CULL_BACK,
        winding: WORLD_WINDING_CCW,
        color_argb: 0xffff_ffff,
        viewport_width: 854,
        viewport_height: 480,
        mesh_key: 44,
        mesh_generation: 9,
        entity_id: 0,
        entity_color_argb: 0,
        outline_color_argb: 0,
        flags: 0,
        block_entity_id: -1,
        transform: [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ],
    }
}

#[test]
fn semantic_gui_ffi_decode_copies_caller_memory() {
    let mut sprites = vec![sprite_request()];
    let request = frame_request(&sprites);
    let (_generation, _target, owned) =
        unsafe { decode_gui_frame_submit(&request, test_capabilities()).unwrap() };
    sprites[0].x = 99;
    assert_eq!(owned[0].x, 10);
    assert_eq!(owned[0].sprite_id, 1);
    assert_eq!(owned[0].gui_width, 320);
}

#[test]
fn semantic_affine_material_is_copied_and_unknown_modes_are_rejected() {
    use crate::render::vulkanic::gui_item_material::GuiAffineMaterial;
    let mut affine = affine_quad_request();
    affine.material_mode = 1;
    let mut request = frame_request(&[]);
    request.affine_quads = FfiSlice { ptr: &affine, count: 1 };
    let (_, _, _, decoded, _) = unsafe {
        super::gui::decode_gui_frame_submit_with_mesh(&request, test_capabilities()).unwrap()
    };
    affine.material_mode = 2;
    assert_eq!(decoded[0].material, GuiAffineMaterial::FlatItemPending);
    let (_, _, _, cutout, _) = unsafe {
        super::gui::decode_gui_frame_submit_with_mesh(&request, test_capabilities()).unwrap()
    };
    assert_eq!(cutout[0].material, GuiAffineMaterial::FlatItemCutoutPending);
    affine.material_mode = 3;
    assert!(unsafe {
        super::gui::decode_gui_frame_submit_with_mesh(&request, test_capabilities())
    }.is_err());
}

#[test]
fn full_item_raster_ffi_copies_scale_and_rejects_incoherent_semantics() {
    let mut affine = affine_quad_request();
    affine.material_mode = 1;
    affine.item_raster_scale = 3;
    affine.item_raster_corners = [4.0,2.0,12.0,2.0,4.0,14.0];
    let decode = |affine: &FfiGuiAffineQuadRequest| {
        let mut frame = frame_request(&[]);
        frame.affine_quads = FfiSlice {ptr:affine,count:1};
        unsafe {super::gui::decode_gui_frame_submit_with_mesh(&frame,test_capabilities())}
    };
    let (_,_,_,owned,_) = decode(&affine).unwrap();
    assert_eq!(owned[0].item_raster_geometry.corners, [4.0,2.0,12.0,2.0,4.0,14.0]);
    affine.item_raster_corners[0] = f32::NAN;
    assert!(decode(&affine).is_err());
    assert_eq!(owned[0].item_raster_geometry.corners[0],4.0,"transport owns its semantic copy");
    affine.item_raster_corners = [0.0,0.0,16.0,4.0,4.0,16.0];
    assert!(decode(&affine).is_err(),"the implied fourth corner must fit the cell");
    affine.item_raster_corners = [4.0,2.0,12.0,2.0,4.0,14.0];
    affine.item_raster_scale = 4;
    assert_eq!(owned[0].item_raster_scale,3);
    assert_eq!(decode(&affine).unwrap().3[0].item_raster_scale,4);
    for scale in [257,u32::MAX] {affine.item_raster_scale=scale; assert!(decode(&affine).is_err());}
    affine.item_raster_scale=3;
    affine.material_mode=0;
    assert!(decode(&affine).is_err());
    affine.material_mode=1;
    affine.u0=0.25;
    assert_eq!(decode(&affine).unwrap().3[0].u0,0.25);
    affine.u0=-0.001;
    assert!(decode(&affine).is_err());
    affine.u0=0.0;
    affine.byte_size=136;
    assert!(decode(&affine).is_err(),"ABI37 records cannot silently omit the nested layer slice");
    affine.byte_size=112;
    assert!(decode(&affine).is_err(),"ABI36 records cannot silently omit item-local corners");
    affine.byte_size=104;
    assert!(decode(&affine).is_err(),"old record sizes cannot silently omit raster semantics");
}

#[test]
fn item_layer_transport_copies_order_and_rejects_invalid_nested_data_before_admission() {
    let layer=FfiGuiItemRasterLayer {byte_size:size_of::<FfiGuiItemRasterLayer>() as u32,
        material_mode:1,asset_id:17,color_argb:0x80ff0000,
        corners:[0.0,0.0,16.0,0.0,0.0,16.0],uv:[0.0,0.0,1.0,1.0],
        model_transform:super::super::gui_item_raster::GuiItemModelTransform::default().0};
    let mut layers=[layer, FfiGuiItemRasterLayer {asset_id:18,material_mode:2,..layer}];
    layers[1].model_transform[0]=0.5;
    layers[1].model_transform[12]=-0.25;
    let mut affine=affine_quad_request();
    affine.material_mode=1; affine.item_raster_scale=2;
    affine.item_raster_layers=FfiSlice {ptr:layers.as_ptr(),count:2};
    let decode=|value:&FfiGuiAffineQuadRequest| {
        let mut frame=frame_request(&[]);
        frame.affine_quads=FfiSlice {ptr:value,count:1};
        unsafe {super::gui::decode_gui_frame_submit_with_mesh(&frame,test_capabilities())}
    };
    let owned=decode(&affine).unwrap().3;
    assert_eq!(owned[0].item_raster_layers.iter().map(|v|v.asset_id).collect::<Vec<_>>(),vec![17,18]);
    layers[1].asset_id=99;
    layers[1].model_transform[0]=0.75;
    assert_eq!(owned[0].item_raster_layers[1].model_transform.0[0],0.5,"matrix is owned, not borrowed");
    assert_eq!(decode(&affine).unwrap().3[0].item_raster_layers[1].model_transform.0[0],0.75);
    assert_eq!(owned[0].item_raster_layers[1].asset_id,18,"no borrowed nested memory");
    assert_eq!(decode(&affine).unwrap().3[0].item_raster_layers[1].asset_id,99);
    for defect in 0..9 {
        layers[1]=layer;
        match defect {
            0 => layers[1].byte_size=0,
            1 => layers[1].asset_id=0,
            2 => layers[1].material_mode=0,
            3 => layers[1].uv[0]=f32::NAN,
            4 => layers[1].uv[2]=0.0,
            5 => layers[1].corners[2]=17.0,
            6 => layers[1].byte_size=64, // ABI38 nested record is no longer sufficient.
            7 => layers[1].model_transform[0]=f32::NAN,
            _ => layers[1].model_transform[3]=0.25,
        }
        assert!(decode(&affine).is_err(),"malformed last layer {defect}");
    }
    layers[1]=layer;
    affine.item_raster_scale=0;
    assert!(decode(&affine).is_err());
    affine.item_raster_scale=2;
    affine.item_raster_layers=FfiSlice {ptr:std::ptr::null(),count:65};
    assert!(decode(&affine).is_err(),"count rejected before dereferencing");
    affine.item_raster_layers.count=1;
    assert!(decode(&affine).is_err());
}

#[test]
fn semantic_gui_projection_is_owned_exact_and_validated_for_every_family() {
    let sprites = [sprite_request()];
    let mut affine = affine_quad_request();
    affine.sequence = 2;
    let vertices = gui_mesh_vertices();
    let indices = [0, 1, 2];
    let mut mesh = gui_mesh_batch_request(&vertices, &indices);
    mesh.sequence = 3;
    let mut request = frame_request(&sprites);
    request.affine_quads = FfiSlice { ptr: &affine, count: 1 };
    request.mesh_batches = FfiSlice { ptr: &mesh, count: 1 };
    request.gui_projection_width = 319.75;
    request.gui_projection_height = 179.5;
    let (_, _, sprites, affine, meshes) = unsafe {
        super::gui::decode_gui_frame_submit_with_mesh(&request, test_capabilities()).unwrap()
    };
    request.gui_projection_width = 320.0;
    assert_eq!([319.75, 179.5], sprites[0].projection_extent);
    assert_eq!([319.75, 179.5], affine[0].projection_extent);
    assert_eq!([319.75, 179.5], meshes[0].projection_extent);
    assert_eq!([320, 180], meshes[0].gui_extent);
    for invalid in [f32::NAN, f32::INFINITY, 0.0, -1.0, 319.0, 320.5] {
        request.gui_projection_width = invalid;
        assert!(unsafe { super::gui::decode_gui_frame_submit_with_mesh(&request, test_capabilities()) }.is_err());
    }
}

#[test]
fn semantic_gui_ffi_rejects_oversized_viewport_before_copying() {
    let mut request = frame_request(&[]);
    request.gui_width = 16_385;
    let error = unsafe { decode_gui_frame_submit(&request, test_capabilities()) }
        .expect_err("oversized GUI viewport must fail before semantic copy");
    assert_eq!(StatusCode::InvalidArgument, error.code);
    assert!(error.message.contains("bounded positive axis"));
}

#[test]
fn semantic_gui_affine_quad_ffi_copies_and_rejects_malformed_input() {
    let mut affine_quads = vec![affine_quad_request()];
    let mut request = frame_request(&[]);
    request.affine_quads = FfiSlice {
        ptr: affine_quads.as_ptr(),
        count: affine_quads.len() as u64,
    };
    let (_generation, _target, sprites, owned) = unsafe {
        super::gui::decode_gui_frame_submit_with_affine(&request, test_capabilities()).unwrap()
    };
    assert!(sprites.is_empty());
    affine_quads[0].x0 = 99.0;
    assert_eq!(owned.len(), 1);
    assert_eq!(owned[0].x0, 10.0);
    assert_eq!(owned[0].asset_id, 0x4a55_4941);

    let mut malformed = affine_quad_request();
    malformed.byte_size -= 4;
    request.affine_quads = FfiSlice {
        ptr: &malformed,
        count: 1,
    };
    let error =
        unsafe { super::gui::decode_gui_frame_submit_with_affine(&request, test_capabilities()) }
            .expect_err("malformed affine quad must fail");
    assert_eq!(error.code, StatusCode::InvalidArgument);
    assert!(error.message.contains("GUI affine quad byte size mismatch"));

    let mut invalid_clip = affine_quad_request();
    invalid_clip.clip_mode = 1;
    invalid_clip.clip_left = 300;
    invalid_clip.clip_width = 30;
    request.affine_quads = FfiSlice {
        ptr: &invalid_clip,
        count: 1,
    };
    let error =
        unsafe { super::gui::decode_gui_frame_submit_with_affine(&request, test_capabilities()) }
            .expect_err("out-of-bounds GUI clip must fail");
    assert_eq!(error.code, StatusCode::InvalidArgument);
    assert!(error.message.contains("clip"));
}

#[test]
fn semantic_gui_ffi_rejects_missing_or_duplicate_cross_family_sequences() {
    let mut sprite = sprite_request();
    sprite.sequence = 0;
    let sprites = vec![sprite];
    let request = frame_request(&sprites);
    let error =
        unsafe { super::gui::decode_gui_frame_submit_with_affine(&request, test_capabilities()) }
            .expect_err("zero scheduler sequence must fail");
    assert_eq!(StatusCode::InvalidArgument, error.code);
    assert!(error.message.contains("non-zero scheduler sequences"));

    let sprites = vec![sprite_request()];
    let mut affine_quads = vec![affine_quad_request()];
    affine_quads[0].sequence = sprites[0].sequence;
    let mut request = frame_request(&sprites);
    request.affine_quads = FfiSlice {
        ptr: affine_quads.as_ptr(),
        count: affine_quads.len() as u64,
    };
    let error =
        unsafe { super::gui::decode_gui_frame_submit_with_affine(&request, test_capabilities()) }
            .expect_err("duplicate cross-family sequence must fail");
    assert_eq!(StatusCode::InvalidArgument, error.code);
    assert!(error.message.contains("must be unique"));
}

#[test]
fn semantic_gui_ffi_rejects_malformed_sprite_records() {
    let mut sprites = vec![sprite_request()];
    sprites[0].byte_size -= 4;
    let request = frame_request(&sprites);
    let error = unsafe { decode_gui_frame_submit(&request, test_capabilities()) }
        .expect_err("malformed sprite must fail");
    assert_eq!(error.code, StatusCode::InvalidArgument);
    assert!(error.message.contains("GUI sprite byte size mismatch"));
}

#[test]
fn semantic_gui_ffi_rejects_wrong_frame_target_kind() {
    let sprites = vec![sprite_request()];
    let mut request = frame_request(&sprites);
    request.frame_target =
        FfiHandle::from(Handle::new(HandleKind::Texture, 3, 1).expect("test handle"));
    let error = unsafe { decode_gui_frame_submit(&request, test_capabilities()) }
        .expect_err("wrong frame target kind must fail");
    assert_eq!(error.code, StatusCode::WrongHandleType);
}

#[test]
fn whole_frame_engine_globals_are_copied_and_validated() {
    let mut request = whole_frame_request(&[], &[]);
    request.engine_globals_present = 1;
    request.engine_screen_width = 854;
    request.engine_screen_height = 480;
    request.engine_game_ticks = 48001;
    request.engine_partial_tick = 0.5;
    request.engine_glint_alpha = 0.75;
    request.engine_menu_blur_radius = 7;
    let (_, _, frame, _) = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }.unwrap();
    request.engine_game_ticks = 0;
    let values = frame.engine_globals.unwrap();
    assert_eq!(48001, values.game_ticks);
    assert_eq!((854, 480), (values.screen_width, values.screen_height));
    assert_eq!(0.5, values.partial_tick);
    assert_eq!(0.75, values.glint_alpha);
    assert_eq!(7, values.menu_blur_radius);
    request.engine_partial_tick = f32::NAN;
    assert!(unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }.is_err());
    request.engine_partial_tick = 0.0;
    request.engine_screen_width = 0;
    assert!(unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }.is_err());
    request.engine_globals_present = 2;
    assert!(unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }.is_err());
    request.engine_globals_present = 0;
    let (_, _, frame, _) = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }.unwrap();
    assert!(frame.engine_globals.is_none());
}

#[test]
fn whole_frame_world_primitive_ffi_decode_copies_caller_memory() {
    let mut segments = vec![line_segment_request()];
    let sprites = vec![sprite_request()];
    let request = whole_frame_request(&segments, &sprites);
    let (_generation, _target, frame, gui) =
        unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()).unwrap() };
    segments[0].start_x = 99.0;
    assert_eq!(frame.frame_id, 11);
    assert_eq!(frame.correlation_id, 13);
    assert!(frame.background.enabled);
    assert_eq!(WORLD_BACKGROUND_SKY_OVERWORLD, frame.background.sky_type);
    assert_eq!(0xff102844, frame.background.color_argb);
    assert_eq!(frame.segments[0].start[0], 1.0);
    assert_eq!(frame.segments[0].end[2], 6.0);
    assert_eq!(gui.len(), 1);
}

#[test]
fn whole_frame_mesh_instance_ffi_rejects_zero_semantic_identity_before_copying() {
    let mut instances = vec![mesh_instance()];
    let mut request = whole_frame_request(&[], &[]);
    request.world_mesh_instances = FfiSlice {
        ptr: instances.as_ptr(),
        count: instances.len() as u64,
    };

    instances[0].mesh_key = 0;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("zero world mesh key must fail at FFI admission");
    assert_eq!(StatusCode::InvalidArgument, error.code);
    assert!(error
        .message
        .contains("key and generation must be non-zero"));

    instances[0] = mesh_instance();
    instances[0].mesh_generation = 0;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("zero world mesh generation must fail at FFI admission");
    assert_eq!(StatusCode::InvalidArgument, error.code);
    assert!(error
        .message
        .contains("key and generation must be non-zero"));

    instances[0] = mesh_instance();
    instances[0].flags = 4;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("unknown mesh semantic flags must fail at FFI admission");
    assert_eq!(StatusCode::InvalidArgument, error.code);
    assert!(error.message.contains("unknown semantic flags"));

    instances[0] = mesh_instance();
    instances[0].flags = 1;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("outline-only non-entity mesh must fail at FFI admission");
    assert_eq!(StatusCode::InvalidArgument, error.code);
    assert!(error.message.contains("outline-only flag"));

    instances[0] = mesh_instance();
    instances[0].block_entity_id = -2;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("invalid block-entity identity must fail at FFI admission");
    assert_eq!(StatusCode::InvalidArgument, error.code);
    assert!(error.message.contains("block entity id must be >= -1"));
}

#[test]
fn whole_frame_ffi_preserves_explicit_camera_sort_for_depth_writing_terrain() {
    use crate::render::vulkanic::world_primitive_frontend::{WORLD_MESH_INSTANCE_FLAG_CAMERA_SORTED_QUADS, WORLD_MESH_SECTION_ALL};
    let mut instances = vec![mesh_instance()];
    instances[0].stratum = WORLD_STRATUM_TERRAIN;
    instances[0].mesh_section_index = WORLD_MESH_SECTION_ALL;
    instances[0].depth_policy = WORLD_DEPTH_POLICY_TEST_WRITE;
    instances[0].flags = WORLD_MESH_INSTANCE_FLAG_CAMERA_SORTED_QUADS;
    let mut request = whole_frame_request(&[], &[]);
    request.world_mesh_instances = FfiSlice { ptr: instances.as_ptr(), count: 1 };
    let (_, _, frame, _) = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }.unwrap();
    instances[0].flags = 0;
    assert_eq!(frame.mesh_instances[0].flags, WORLD_MESH_INSTANCE_FLAG_CAMERA_SORTED_QUADS);
    assert_eq!(frame.mesh_instances[0].depth_policy, WORLD_DEPTH_POLICY_TEST_WRITE);
    instances[0].flags = WORLD_MESH_INSTANCE_FLAG_CAMERA_SORTED_QUADS;
    instances[0].mesh_section_index = 0;
    assert!(unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }.is_err());
    instances[0].mesh_section_index = WORLD_MESH_SECTION_ALL;
    instances[0].stratum = WORLD_STRATUM_ENTITY_MESH;
    assert!(unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }.is_err());
}

#[test]
fn whole_frame_ffi_rejects_oversized_viewport_before_copying() {
    let mut request = whole_frame_request(&[], &[]);
    request.viewport_width = GUI_MAX_VIEWPORT_AXIS + 1;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("whole-frame viewport must remain bounded");
    assert_eq!(StatusCode::InvalidArgument, error.code);
    assert!(error.message.contains("bounded axis"));
}

#[test]
fn whole_frame_ffi_rejects_oversized_declared_payload_before_copying() {
    let mut request = whole_frame_request(&[], &[]);
    request.gui_affine_quads = FfiSlice {
        ptr: std::ptr::NonNull::<FfiGuiAffineQuadRequest>::dangling().as_ptr(),
        count: FFI_MAX_WHOLE_FRAME_INPUT_BYTES / size_of::<FfiGuiAffineQuadRequest>() as u64 + 1,
    };
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("whole-frame payload must remain bounded before copying");
    assert_eq!(StatusCode::InvalidArgument, error.code);
    assert!(error.message.contains("bounded input size"));
}

#[test]
fn whole_frame_first_person_transport_copies_projection_and_rejects_malformed_depth_domain() {
    let mut request = whole_frame_request(&[], &[]);
    let mut projection = [0.0; 16];
    let mut model_view = [0.0; 16];
    projection[0] = 1.0;
    projection[5] = 1.0;
    projection[10] = 1.0;
    projection[15] = 1.0;
    model_view[0] = 1.0;
    model_view[5] = 1.0;
    model_view[10] = 1.0;
    model_view[12] = 0.25;
    model_view[15] = 1.0;
    request.world_first_person_frame = FfiWorldFirstPersonFrame {
        byte_size: size_of::<FfiWorldFirstPersonFrame>() as u32,
        enabled: 1,
        clear_depth_before: 1,
        main_hand_instance_count: 0,
        projection_matrix: projection,
        model_view_matrix: model_view,
    };
    let (_, _, frame, _) = unsafe {
        decode_whole_frame_submit(&request, test_vulkan_capabilities())
            .expect("valid first-person semantic frame")
    };
    projection[0] = 99.0;
    model_view[12] = 99.0;
    assert!(frame.first_person.enabled);
    assert!(frame.first_person.clear_depth_before);
    assert_eq!(1.0, frame.first_person.projection_matrix[0]);
    assert_eq!(0.25, frame.first_person.model_view_matrix[12]);

    request.world_first_person_frame.clear_depth_before = 0;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("an enabled hand frame cannot inherit world depth");
    assert_eq!(StatusCode::InvalidArgument, error.code);
    assert!(error.message.contains("explicitly clear its depth domain"));

    request.world_first_person_frame.clear_depth_before = 1;
    request.world_first_person_frame.model_view_matrix = [0.0; 16];
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("an enabled hand frame requires an explicit model-view domain");
    assert_eq!(StatusCode::InvalidArgument, error.code);
    assert!(error.message.contains("model-view matrix"));

    request.world_first_person_frame.enabled = 0;
    request.world_first_person_frame.projection_matrix = projection;
    request.world_first_person_frame.model_view_matrix = model_view;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("disabled hand data cannot carry stale projection state");
    assert_eq!(StatusCode::InvalidArgument, error.code);
    assert!(error.message.contains("must be zeroed"));
}

#[test]
fn whole_frame_first_person_mesh_stream_is_copied_and_requires_its_own_domain() {
    let mut request = whole_frame_request(&[], &[]);
    let projection = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    request.world_first_person_frame = FfiWorldFirstPersonFrame {
        byte_size: size_of::<FfiWorldFirstPersonFrame>() as u32,
        enabled: 1,
        clear_depth_before: 1,
        main_hand_instance_count: 0,
        projection_matrix: projection,
        model_view_matrix: projection,
    };
    let mut hands = vec![FfiWorldMeshInstanceRecord {
        item_foil_mode: 0, item_foil_clock_millis: 0, item_foil_speed: 0.0, item_foil_strength: 0.0,
        terrain_placement_mode: 0, terrain_origin: [0;3], terrain_camera: [0.0;3],
        byte_size: size_of::<FfiWorldMeshInstanceRecord>() as u32,
        stratum: WORLD_STRATUM_ENTITY_MESH,
        mesh_section_index: u32::MAX,
        depth_policy: WORLD_DEPTH_POLICY_TEST_WRITE,
        cull_policy: WORLD_CULL_BACK,
        winding: WORLD_WINDING_CCW,
        color_argb: 0xffff_ffff,
        viewport_width: 128,
        viewport_height: 128,
        mesh_key: 17,
        mesh_generation: 3,
        entity_id: 0,
        entity_color_argb: 0,
        outline_color_argb: 0,
        flags: 0,
        block_entity_id: -1,
        transform: projection,
    }];
    request.world_first_person_mesh_instances = FfiSlice {
        ptr: hands.as_ptr(),
        count: hands.len() as u64,
    };
    request.world_first_person_frame.main_hand_instance_count = 1;
    let (_, _, frame, _) = unsafe {
        decode_whole_frame_submit(&request, test_vulkan_capabilities())
            .expect("valid first-person mesh stream")
    };
    hands[0].transform[0] = 99.0;
    assert_eq!(1, frame.first_person_mesh_instances.len());
    assert_eq!(17, frame.first_person_mesh_instances[0].mesh_key);
    assert_eq!(1.0, frame.first_person_mesh_instances[0].transform[0]);
    assert_eq!(1, frame.first_person.main_hand_instance_count);

    hands[0].transform = projection;
    hands[0].stratum = WORLD_STRATUM_TERRAIN;
    hands[0].terrain_placement_mode = 1;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("terrain placement must never be silently ignored in the hand domain");
    assert_eq!(StatusCode::InvalidArgument, error.code);
    assert!(error.message.contains("first-person mesh instances cannot use terrain placement"));
    hands[0].stratum = WORLD_STRATUM_ENTITY_MESH;
    hands[0].terrain_placement_mode = 0;

    hands[0].viewport_width = GUI_MAX_VIEWPORT_AXIS + 1;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("oversized first-person mesh viewport must fail before admission");
    assert_eq!(StatusCode::InvalidArgument, error.code);
    assert!(error.message.contains("bounded positive range"));
    hands[0].viewport_width = 128;

    request.world_first_person_frame.main_hand_instance_count = 2;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("the hand split cannot exceed the copied semantic stream");
    assert_eq!(StatusCode::InvalidArgument, error.code);
    assert!(error.message.contains("main-hand instance count"));
    request.world_first_person_frame.main_hand_instance_count = 1;

    request.world_first_person_frame.enabled = 0;
    request.world_first_person_frame.clear_depth_before = 0;
    request.world_first_person_frame.projection_matrix = [0.0; 16];
    request.world_first_person_frame.model_view_matrix = [0.0; 16];
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("first-person meshes cannot inherit world projection");
    assert_eq!(StatusCode::InvalidArgument, error.code);
    assert!(error
        .message
        .contains("require an enabled first-person frame"));

    request.world_first_person_frame.enabled = 1;
    request.world_first_person_frame.clear_depth_before = 1;
    request.world_first_person_frame.projection_matrix = projection;
    request.world_first_person_frame.model_view_matrix = projection;
    hands[0].stratum = WORLD_STRATUM_MOVING_MESH;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("first-person stream must reject camera-space mesh strata");
    assert_eq!(StatusCode::InvalidArgument, error.code);
    assert!(error
        .message
        .contains("generic entity-mesh semantic stratum"));

    hands[0].stratum = WORLD_STRATUM_ENTITY_MESH;
    hands[0].flags = 1;
    hands[0].outline_color_argb = 0xff_112233;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("first-person outline-only meshes must fail before semantic copy");
    assert_eq!(StatusCode::InvalidArgument, error.code);
    assert!(error.message.contains("cannot be outline-only"));

    hands[0].flags = 0;
    hands[0].outline_color_argb = 0;
    hands[0].block_entity_id = 7;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("first-person meshes must not carry block-entity identity");
    assert_eq!(StatusCode::InvalidArgument, error.code);
    assert!(error.message.contains("cannot carry block-entity identity"));

    hands[0].block_entity_id = -1;
    hands[0].entity_id = 13;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("first-person meshes must not carry shader-pack entity identity");
    assert_eq!(StatusCode::InvalidArgument, error.code);
    assert!(error
        .message
        .contains("cannot carry shader-pack entity identity"));
}

#[test]
fn world_and_hand_standard_foil_transport_copies_and_rejects_noncanonical_payloads() {
    let mut source = mesh_instance();
    source.stratum = WORLD_STRATUM_ENTITY_MESH;
    source.item_foil_mode = 1;
    source.item_foil_clock_millis = 12345;
    source.item_foil_speed = 0.125;
    source.item_foil_strength = 0.1234567;
    let mut request = whole_frame_request(&[], &[]);
    request.world_mesh_instances = FfiSlice { ptr: &source, count: 1 };
    request.world_first_person_mesh_instances = FfiSlice { ptr: &source, count: 1 };
    request.world_first_person_frame = FfiWorldFirstPersonFrame {
        byte_size: size_of::<FfiWorldFirstPersonFrame>() as u32,
        enabled: 1, clear_depth_before: 1, main_hand_instance_count: 1,
        projection_matrix: source.transform, model_view_matrix: source.transform,
    };
    let (_, _, frame, _) = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }.unwrap();
    let expected = crate::render::vulkanic::item_foil::StandardItemFoil {
        kind: crate::render::vulkanic::item_foil::StandardFoilKind::Item,
        clock_millis: 12345, speed: 0.125, strength: 0.1234567,
    };
    source.item_foil_clock_millis = 999;
    assert_eq!(frame.mesh_instances[0].item_foil, Some(expected));
    assert_eq!(frame.first_person_mesh_instances[0].item_foil, Some(expected));
    source.item_foil_mode = 2;
    source.item_foil_clock_millis = 12345;
    let (_, _, entity_frame, _) = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }.unwrap();
    let entity_expected = crate::render::vulkanic::item_foil::StandardItemFoil {
        kind: crate::render::vulkanic::item_foil::StandardFoilKind::Entity, ..expected
    };
    assert_eq!(entity_frame.mesh_instances[0].item_foil, Some(entity_expected));
    assert_eq!(entity_frame.first_person_mesh_instances[0].item_foil, Some(entity_expected));
    for (mode, clock, speed, strength) in [
        (0, 1, 0.0, 0.0), (0, 0, -0.0, 0.0), (0, 0, 0.0, -0.0),
        (3, 0, 0.0, 0.0), (1, u64::MAX, 0.0, 0.0),
        (1, 0, f64::NAN, 0.5), (1, 0, 0.5, f32::INFINITY),
        (1, 0, 1.01, 0.5), (1, 0, 0.5, -0.01),
    ] {
        source.item_foil_mode = mode;
        source.item_foil_clock_millis = clock;
        source.item_foil_speed = speed;
        source.item_foil_strength = strength;
        assert!(unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }.is_err());
        request.world_mesh_instances.count = 0;
        assert!(unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }.is_err(), "hand decoder must independently reject malformed foil");
        request.world_mesh_instances.count = 1;
    }
    source.item_foil_mode = 0;
    source.item_foil_clock_millis = 0;
    source.item_foil_speed = 0.0;
    source.item_foil_strength = 0.0;
    let (_, _, frame, _) = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }.unwrap();
    assert_eq!(frame.mesh_instances[0].item_foil, None);
    assert_eq!(frame.first_person_mesh_instances[0].item_foil, None);
    source.item_foil_mode = 1;
    source.block_entity_id = 0;
    assert!(unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }.is_err());
}

#[test]
fn whole_frame_world_text_ffi_rejects_malformed_semantics_before_submission() {
    let mut text_quads = vec![FfiWorldTextQuadRequest {
        byte_size: size_of::<FfiWorldTextQuadRequest>() as u32,
        flags: 0,
        depth_policy: WORLD_TEXT_DEPTH_POLYGON_OFFSET,
        packed_light: 0,
        color_argb: 0xffff_ffff,
        reserved0: 0,
        asset_id: 7,
        atlas_generation: 3,
        atlas_revision: 5,
        distance_to_camera_sq: 4.0,
        model_view_matrix: [1.0; 16],
        positions: [0.0; 12],
        uvs: [0.0; 8],
        block_entity_id: -1,
    }];
    let mut request = whole_frame_request(&[], &[]);
    request.world_text_quads = FfiSlice {
        ptr: text_quads.as_ptr(),
        count: text_quads.len() as u64,
    };

    let (_, _, frame, _) = unsafe {
        decode_whole_frame_submit(&request, test_vulkan_capabilities())
            .expect("valid copied world text transport")
    };
    text_quads[0].positions[0] = 99.0;
    assert_eq!(frame.text_quads.len(), 1);
    assert_eq!(
        frame.text_quads[0].depth_policy,
        WORLD_TEXT_DEPTH_POLYGON_OFFSET
    );
    assert_eq!(frame.text_quads[0].positions[0][0], 0.0);

    request.world_text_quads = FfiSlice {
        ptr: text_quads.as_ptr(),
        count: text_quads.len() as u64,
    };
    text_quads[0].flags = 2;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("unknown world text flags must fail");
    assert_eq!(error.code, StatusCode::InvalidArgument);
}

#[test]
fn world_text_image_update_copies_pixels_and_rejects_invalid_formats() {
    let pixels = [0u8, 64, 128, 255];
    let assets = [FfiWorldTextImageAssetPayload {
        byte_size: size_of::<FfiWorldTextImageAssetPayload>() as u32,
        format: 1,
        width: 2,
        height: 2,
        asset_id: 7,
        atlas_generation: 3,
        atlas_revision: 5,
        pixels: FfiBytes {
            ptr: pixels.as_ptr(),
            len: pixels.len() as u64,
        },
    }];
    let request = FfiWorldTextImageUpdateRequest {
        header: FfiHeader {
            version: FFI_ABI_VERSION,
            byte_size: size_of::<FfiWorldTextImageUpdateRequest>() as u32,
        },
        generation: 9,
        assets: FfiSlice {
            ptr: assets.as_ptr(),
            count: assets.len() as u64,
        },
        negotiated_feature_bits: FfiFeatureBits::GRAPHICS,
    };
    let (_, decoded) = unsafe { decode_world_text_image_update(&request, test_capabilities()) }
        .expect("valid image update");
    assert_eq!(decoded[0].pixels, pixels);
    let invalid_assets = [FfiWorldTextImageAssetPayload {
        format: 99,
        ..assets[0]
    }];
    let invalid_request = FfiWorldTextImageUpdateRequest {
        assets: FfiSlice {
            ptr: invalid_assets.as_ptr(),
            count: invalid_assets.len() as u64,
        },
        ..request
    };
    let error = unsafe { decode_world_text_image_update(&invalid_request, test_capabilities()) }
        .expect_err("unknown image format must fail");
    assert_eq!(error.code, StatusCode::UnknownEnum);
}

#[test]
fn world_text_image_update_rejects_oversized_asset_batches_before_copying() {
    let pixels = [0u8; 4];
    let asset = FfiWorldTextImageAssetPayload {
        byte_size: size_of::<FfiWorldTextImageAssetPayload>() as u32,
        format: 1,
        width: 1,
        height: 1,
        asset_id: 7,
        atlas_generation: 3,
        atlas_revision: 5,
        pixels: FfiBytes {
            ptr: pixels.as_ptr(),
            len: pixels.len() as u64,
        },
    };
    let assets = vec![asset; 4_097];
    let request = FfiWorldTextImageUpdateRequest {
        header: FfiHeader {
            version: FFI_ABI_VERSION,
            byte_size: size_of::<FfiWorldTextImageUpdateRequest>() as u32,
        },
        generation: 9,
        assets: FfiSlice {
            ptr: assets.as_ptr(),
            count: assets.len() as u64,
        },
        negotiated_feature_bits: FfiFeatureBits::GRAPHICS,
    };
    let error = unsafe { decode_world_text_image_update(&request, test_capabilities()) }
        .expect_err("oversized image batches must fail before payload copying");
    assert_eq!(error.code, StatusCode::LengthOverflow);
}

#[test]
fn whole_frame_gui_affine_quads_share_the_owned_frame_decode() {
    let segments = vec![line_segment_request()];
    let sprites = vec![sprite_request()];
    let mut affine_quads = vec![affine_quad_request()];
    let mut request = whole_frame_request(&segments, &sprites);
    request.gui_projection_width = 319.75;
    request.gui_projection_height = 179.5;
    request.gui_affine_quads = FfiSlice {
        ptr: affine_quads.as_ptr(),
        count: affine_quads.len() as u64,
    };
    let (
        _generation,
        _target,
        _frame,
        decoded_sprites,
        decoded_affine_quads,
        decoded_mesh_batches,
        _blur_boundary,
        _blur_radius,
        _post_effect_id,
    ) = unsafe {
        decode_whole_frame_submit_with_gui(&request, test_vulkan_capabilities()).unwrap()
    };
    affine_quads[0].y3 = 99.0;
    assert_eq!(decoded_sprites.len(), 1);
    assert_eq!(decoded_affine_quads.len(), 1);
    assert_eq!([319.75, 179.5], decoded_sprites[0].projection_extent);
    assert_eq!([319.75, 179.5], decoded_affine_quads[0].projection_extent);
    assert!(decoded_mesh_batches.is_empty());
    assert_eq!(decoded_affine_quads[0].y3, 28.0);

    let error = unsafe { decode_world_primitive_submit(&request, test_vulkan_capabilities()) }
        .expect_err("world-only submission must reject GUI affine work");
    assert_eq!(error.code, StatusCode::InvalidArgument);
    assert!(error.message.contains("does not accept GUI work"));
}

#[test]
fn whole_frame_feature_coverage_decodes_as_copied_semantic_counts() {
    let mut request = whole_frame_request(&[], &[]);
    request.world_feature_coverage = FfiWorldFeatureCoverage {
        byte_size: size_of::<FfiWorldFeatureCoverage>() as u32,
        model_submits: 3,
        model_part_submits: 5,
        block_model_submits: 7,
        ordinary_block_submits: 11,
        item_submits: 13,
        custom_geometry_submits: 17,
        shadow_submits: 19,
        flame_submits: 23,
        name_tag_submits: 29,
        text_submits: 31,
        hitbox_submits: 37,
        leash_submits: 41,
        particle_group_submits: 43,
    };
    let (_generation, _target, frame, _gui) =
        unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()).unwrap() };
    assert_eq!(frame.feature_coverage.model_submits, 3);
    assert_eq!(frame.feature_coverage.model_part_submits, 5);
    assert_eq!(frame.feature_coverage.particle_group_submits, 43);

    request.world_feature_coverage.byte_size -= 4;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("malformed feature coverage must fail before route admission");
    assert_eq!(error.code, StatusCode::InvalidArgument);
    assert!(error
        .message
        .contains("world feature coverage frame byte size mismatch"));
}

#[test]
fn whole_frame_voxel_volume_semantics_decode_and_reject_malformed_state() {
    let mut request = whole_frame_request(&[], &[]);
    request.voxel_volume_frame = FfiWorldVoxelVolumeFrame {
        byte_size: size_of::<FfiWorldVoxelVolumeFrame>() as u32,
        enabled: 1,
        reserved0: 0,
        reserved1: 0,
        world_generation: 17,
        resource_generation: 23,
        camera_x: 12.5,
        camera_y: 64.0,
        camera_z: -3.25,
        reserved2: 0,
    };
    let (_generation, _target, frame, _gui) =
        unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()).unwrap() };
    assert_eq!(
        frame.voxel_volume,
        WorldVoxelVolumeFrame {
            enabled: true,
            world_generation: 17,
            resource_generation: 23,
            camera_world_position: [12.5, 64.0, -3.25],
        }
    );

    request.voxel_volume_frame.world_generation = 0;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("enabled voxel volume must have a world generation");
    assert_eq!(error.code, StatusCode::InvalidArgument);

    request = whole_frame_request(&[], &[]);
    request.voxel_volume_frame.camera_x = 1.0;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("disabled voxel volume must be zeroed");
    assert_eq!(error.code, StatusCode::InvalidArgument);

    request = whole_frame_request(&[], &[]);
    request.voxel_volume_frame.enabled = 1;
    request.voxel_volume_frame.world_generation = 17;
    request.voxel_volume_frame.resource_generation = 23;
    request.voxel_volume_frame.camera_x = f32::NAN;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("voxel volume camera coordinates must be finite");
    assert_eq!(error.code, StatusCode::InvalidArgument);
}

#[test]
fn whole_frame_shader_environment_semantics_decode_and_reject_malformed_state() {
    let biome_resource_location = b"minecraft:snowy_plains";
    let main_hand_item_model_resource_location = b"minecraft:lava_bucket";
    let off_hand_item_model_resource_location = b"minecraft:totem_of_undying";
    let mut request = whole_frame_request(&[], &[]);
    request.shader_environment_frame = FfiWorldShaderEnvironmentFrame {
        byte_size: size_of::<FfiWorldShaderEnvironmentFrame>() as u32,
        enabled: 1,
        frame_counter: 42,
        world_day: 17,
        world_generation: 17,
        world_time: 24_013,
        frame_time_seconds: 0.016,
        frame_time_counter: 12.5,
        time_of_day: 0.25,
        rain_strength: 0.2,
        thunder_strength: 0.1,
        sky_darken: 0.75,
        moon_phase: 3,
        eye_submersion: 1,
        screen_brightness: 0.5,
        far_plane: 192.0,
        relative_eye_x: 0.25,
        relative_eye_y: -0.5,
        relative_eye_z: 0.75,
        sky_color_r: 0.2,
        sky_color_g: 0.4,
        sky_color_b: 0.6,
        darkness_light_factor: 0.125,
        night_vision: 0.875,
        fog_color_r: 0.3,
        fog_color_g: 0.5,
        fog_color_b: 0.7,
        biome_precipitation: 2,
        biome_resource_location_utf8: FfiBytes {
            ptr: biome_resource_location.as_ptr(),
            len: biome_resource_location.len() as u64,
        },
        main_hand_item_model_resource_location_utf8: FfiBytes {
            ptr: main_hand_item_model_resource_location.as_ptr(),
            len: main_hand_item_model_resource_location.len() as u64,
        },
        off_hand_item_model_resource_location_utf8: FfiBytes {
            ptr: off_hand_item_model_resource_location.as_ptr(),
            len: off_hand_item_model_resource_location.len() as u64,
        },
        main_hand_item_light_emission: 13,
        off_hand_item_light_emission: 7,
        lightmap_enabled: 1,
        lightmap_reserved: 0,
        lightmap_generation: 31,
        lightmap_ambient_light_factor: 0.125,
        lightmap_sky_factor: 0.75,
        lightmap_block_factor: 1.5,
        lightmap_night_vision_factor: 0.5,
        lightmap_darkness_scale: 0.25,
        lightmap_darken_world_factor: 0.4,
        lightmap_brightness_factor: 0.8,
        lightmap_sky_light_r: 0.6,
        lightmap_sky_light_g: 0.7,
        lightmap_sky_light_b: 0.9,
        lightmap_ambient_r: 1.0,
        lightmap_ambient_g: 0.5,
        lightmap_ambient_b: 0.25,
        blindness: 0.25,
        darkness_factor: 0.5,
        eye_brightness_block: 80,
        eye_brightness_sky: 240,
        fog_parameter_color_r: 0.1,
        fog_parameter_color_g: 0.2,
        fog_parameter_color_b: 0.4,
        fog_parameter_color_a: 0.8,
        fog_environmental_start: 12.0,
        fog_environmental_end: 96.0,
        fog_render_distance_start: 24.0,
        fog_render_distance_end: 128.0,
        fog_sky_end: 128.0,
        fog_clouds_end: 96.0,
        distant_horizons_render_distance: 256,
    };
    let (_generation, _target, frame, _gui) =
        unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()).unwrap() };
    assert_eq!(
        frame.shader_environment,
        WorldShaderEnvironmentFrame {
            enabled: true,
            world_generation: 17,
            world_time: 24_013,
            frame_counter: 42,
            frame_time_seconds: 0.016,
            frame_time_counter: 12.5,
            world_day: 17,
            moon_phase: 3,
            time_of_day: 0.25,
            rain_strength: 0.2,
            thunder_strength: 0.1,
            sky_darken: 0.75,
            eye_submersion: 1,
            screen_brightness: 0.5,
            far_plane: 192.0,
            distant_horizons_render_distance: 256,
            relative_eye_position: [0.25, -0.5, 0.75],
            sky_color: [0.2, 0.4, 0.6],
            darkness_light_factor: 0.125,
            blindness: 0.25,
            darkness_factor: 0.5,
            eye_brightness: [80, 240],
            night_vision: 0.875,
            fog_color: [0.3, 0.5, 0.7],
            fog_parameter_color: [0.1, 0.2, 0.4, 0.8],
            fog_environmental_start: 12.0,
            fog_environmental_end: 96.0,
            fog_render_distance_start: 24.0,
            fog_render_distance_end: 128.0,
            fog_sky_end: 128.0,
            fog_clouds_end: 96.0,
            biome_precipitation: 2,
            biome_resource_location: "minecraft:snowy_plains".to_string(),
            main_hand_item_model_resource_location: "minecraft:lava_bucket".to_string(),
            off_hand_item_model_resource_location: "minecraft:totem_of_undying".to_string(),
            main_hand_item_light_emission: 13,
            off_hand_item_light_emission: 7,
            vanilla_lightmap: Some(
                crate::render::vulkanic::shader_pack::lightmap::VanillaLightmapFrame {
                    generation: 31,
                    inputs: crate::render::vulkanic::shader_pack::lightmap::VanillaLightmapInputs {
                        ambient_light_factor: 0.125,
                        sky_factor: 0.75,
                        block_factor: 1.5,
                        night_vision_factor: 0.5,
                        darkness_scale: 0.25,
                        darken_world_factor: 0.4,
                        brightness_factor: 0.8,
                        sky_light_color: [0.6, 0.7, 0.9],
                        ambient_color: [1.0, 0.5, 0.25],
                    },
                }
            ),
        }
    );

    request.shader_environment_frame.lightmap_generation = 0;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("enabled lightmap must require a nonzero generation");
    assert_eq!(error.code, StatusCode::InvalidArgument);

    request.shader_environment_frame.lightmap_generation = 31;
    request.shader_environment_frame.blindness = f32::NAN;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("non-finite blindness must be rejected");
    assert_eq!(error.code, StatusCode::InvalidArgument);

    request.shader_environment_frame.blindness = 0.25;
    request.shader_environment_frame.darkness_factor = 0.5;
    request.shader_environment_frame.eye_brightness_block = 81;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("unpacked eye brightness must be rejected");
    assert_eq!(error.code, StatusCode::InvalidArgument);
    request.shader_environment_frame.eye_brightness_block = 80;
    request.shader_environment_frame.fog_environmental_end = f32::NAN;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("non-finite copied fog range must be rejected");
    assert_eq!(error.code, StatusCode::InvalidArgument);

    request.shader_environment_frame.fog_environmental_end = 96.0;
    request
        .shader_environment_frame
        .distant_horizons_render_distance = -1;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("negative Distant Horizons render distance must be rejected");
    assert_eq!(error.code, StatusCode::InvalidArgument);

    request
        .shader_environment_frame
        .distant_horizons_render_distance = 256;
    request.shader_environment_frame.lightmap_reserved = 1;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("lightmap reserved field must be rejected");
    assert_eq!(error.code, StatusCode::InvalidArgument);

    request.shader_environment_frame.lightmap_reserved = 0;
    request.shader_environment_frame.lightmap_enabled = 0;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("disabled lightmap payload must be zeroed");
    assert_eq!(error.code, StatusCode::InvalidArgument);

    request.shader_environment_frame.lightmap_generation = 0;
    request
        .shader_environment_frame
        .lightmap_ambient_light_factor = 0.0;
    request.shader_environment_frame.lightmap_sky_factor = 0.0;
    request.shader_environment_frame.lightmap_block_factor = 0.0;
    request
        .shader_environment_frame
        .lightmap_night_vision_factor = 0.0;
    request.shader_environment_frame.lightmap_darkness_scale = 0.0;
    request
        .shader_environment_frame
        .lightmap_darken_world_factor = 0.0;
    request.shader_environment_frame.lightmap_brightness_factor = 0.0;
    request.shader_environment_frame.lightmap_sky_light_r = 0.0;
    request.shader_environment_frame.lightmap_sky_light_g = 0.0;
    request.shader_environment_frame.lightmap_sky_light_b = 0.0;
    request.shader_environment_frame.lightmap_ambient_r = 0.0;
    request.shader_environment_frame.lightmap_ambient_g = 0.0;
    request.shader_environment_frame.lightmap_ambient_b = 0.0;

    request.shader_environment_frame.time_of_day = f32::NAN;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("shader environment must reject non-finite values");
    assert_eq!(error.code, StatusCode::InvalidArgument);

    request.shader_environment_frame.time_of_day = 0.25;
    request
        .shader_environment_frame
        .main_hand_item_light_emission = 16;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("shader environment must reject out-of-range held-item emission");
    assert_eq!(error.code, StatusCode::InvalidArgument);

    request.shader_environment_frame.time_of_day = 0.25;
    let malformed_biome_resource_location = b"Minecraft:Snowy Plains";
    request
        .shader_environment_frame
        .biome_resource_location_utf8 = FfiBytes {
        ptr: malformed_biome_resource_location.as_ptr(),
        len: malformed_biome_resource_location.len() as u64,
    };
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("shader environment must reject a non-canonical biome identity");
    assert_eq!(error.code, StatusCode::InvalidArgument);
    request
        .shader_environment_frame
        .biome_resource_location_utf8 = FfiBytes {
        ptr: biome_resource_location.as_ptr(),
        len: biome_resource_location.len() as u64,
    };
    let malformed_main_hand_item_model_resource_location = b"Minecraft:Lava Bucket";
    request
        .shader_environment_frame
        .main_hand_item_model_resource_location_utf8 = FfiBytes {
        ptr: malformed_main_hand_item_model_resource_location.as_ptr(),
        len: malformed_main_hand_item_model_resource_location.len() as u64,
    };
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("shader environment must reject a non-canonical held-item identity");
    assert_eq!(error.code, StatusCode::InvalidArgument);
    request
        .shader_environment_frame
        .main_hand_item_model_resource_location_utf8 = FfiBytes {
        ptr: main_hand_item_model_resource_location.as_ptr(),
        len: main_hand_item_model_resource_location.len() as u64,
    };
    request.shader_environment_frame.night_vision = f32::NAN;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("shader environment must reject non-finite night vision");
    assert_eq!(error.code, StatusCode::InvalidArgument);
    request.shader_environment_frame.night_vision = 0.875;
    request.shader_environment_frame.fog_color_g = f32::NAN;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("shader environment must reject non-finite fog color");
    assert_eq!(error.code, StatusCode::InvalidArgument);
    request.shader_environment_frame.fog_color_g = 0.5;
    request.shader_environment_frame.biome_precipitation = 3;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("shader environment must reject unknown biome precipitation");
    assert_eq!(error.code, StatusCode::InvalidArgument);
    request.shader_environment_frame.biome_precipitation = 2;
    request.shader_environment_frame.frame_counter = 720_720;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("shader environment must reject an unwrapped frame counter");
    assert_eq!(error.code, StatusCode::InvalidArgument);

    request.shader_environment_frame.frame_counter = 42;
    request.shader_environment_frame.eye_submersion = 4;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("shader environment must reject unknown camera submersion");
    assert_eq!(error.code, StatusCode::InvalidArgument);

    request.shader_environment_frame.eye_submersion = 1;
    request.shader_environment_frame.sky_color_b = f32::NAN;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("shader environment must reject non-finite sky color");
    assert_eq!(error.code, StatusCode::InvalidArgument);

    request.shader_environment_frame.sky_color_b = 0.6;
    request.shader_environment_frame.far_plane = 0.0;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("shader environment must reject a non-positive far plane");
    assert_eq!(error.code, StatusCode::InvalidArgument);

    request.shader_environment_frame.far_plane = 192.0;
    request.shader_environment_frame.frame_time_seconds = f32::NAN;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("shader environment must reject a non-finite frame time");
    assert_eq!(error.code, StatusCode::InvalidArgument);

    request.shader_environment_frame.frame_time_seconds = 0.016;
    request.shader_environment_frame.relative_eye_z = f32::NAN;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("shader environment must reject a non-finite relative eye position");
    assert_eq!(error.code, StatusCode::InvalidArgument);

    request.shader_environment_frame.frame_counter = 42;
    request.shader_environment_frame.frame_time_counter = 3600.0;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("shader environment must reject an unreset frame timer");
    assert_eq!(error.code, StatusCode::InvalidArgument);

    request.shader_environment_frame.frame_time_counter = 12.5;
    request.shader_environment_frame.moon_phase = 8;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("shader environment must reject an invalid moon phase");
    assert_eq!(error.code, StatusCode::InvalidArgument);

    request = whole_frame_request(&[], &[]);
    request.shader_environment_frame.world_time = 1;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("disabled shader environment must be zeroed");
    assert_eq!(error.code, StatusCode::InvalidArgument);
}

#[test]
fn whole_frame_world_primitive_ffi_rejects_bad_segment_size_and_non_vulkan() {
    let mut segments = vec![line_segment_request()];
    let sprites = vec![sprite_request()];
    let request = whole_frame_request(&segments, &sprites);
    let error = unsafe { decode_whole_frame_submit(&request, test_capabilities()) }
        .expect_err("non-Vulkan whole-frame submit must fail");
    assert_eq!(error.code, StatusCode::UnsupportedFeature);
    segments[0].byte_size -= 4;
    let request = whole_frame_request(&segments, &sprites);
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("malformed world line segment must fail");
    assert_eq!(error.code, StatusCode::InvalidArgument);

    segments[0] = line_segment_request();
    segments[0].viewport_width = GUI_MAX_VIEWPORT_AXIS + 1;
    let request = whole_frame_request(&segments, &sprites);
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("oversized world line viewport must fail before admission");
    assert_eq!(error.code, StatusCode::InvalidArgument);
    assert!(error.message.contains("bounded positive range"));
}

#[test]
fn whole_frame_world_background_ffi_rejects_malformed_payloads() {
    let mut request = whole_frame_request(&[], &[]);
    request.world_background.byte_size -= 4;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("malformed world background must fail");
    assert_eq!(error.code, StatusCode::InvalidArgument);

    request = whole_frame_request(&[], &[]);
    request.world_background.sky_type = 99;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("unknown sky type must fail validation");
    assert_eq!(error.code, StatusCode::UnknownEnum);

    request = whole_frame_request(&[], &[]);
    request.world_background.sky_reserved0 = 1;
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("non-zero sky reserved field must fail validation");
    assert_eq!(error.code, StatusCode::InvalidArgument);

    request = whole_frame_request(&[], &[]);
    request.world_background.sky_visible = 1;
    request.world_background.sky_moon_phase = 8;
    let (_generation, _target, frame, _gui) = unsafe {
        decode_whole_frame_submit(&request, test_vulkan_capabilities())
            .expect("FFI must copy semantic sky fields for frontend validation")
    };
    assert!(frame.background.sky.visible);
    assert_eq!(8, frame.background.sky.moon_phase);
}

#[test]
fn whole_frame_world_crack_ffi_copies_and_rejects_malformed_payloads() {
    let mut cracks = vec![crack_quad_request()];
    let request = whole_frame_request_with_cracks(&[], &cracks, &[]);
    let (_generation, _target, frame, gui) =
        unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()).unwrap() };
    cracks[0].p0_x = 99.0;
    assert_eq!(frame.crack_quads.len(), 1);
    assert_eq!(frame.crack_quads[0].vertices[0][0], 1.0);
    assert_eq!(frame.crack_quads[0].stage, 4);
    assert!(gui.is_empty());

    cracks[0] = crack_quad_request();
    cracks[0].blend_policy = 99;
    let request = whole_frame_request_with_cracks(&[], &cracks, &[]);
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("unknown crack blend policy must fail");
    assert_eq!(error.code, StatusCode::UnknownEnum);

    cracks[0] = crack_quad_request();
    cracks[0].byte_size -= 4;
    let request = whole_frame_request_with_cracks(&[], &cracks, &[]);
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("malformed crack quad must fail");
    assert_eq!(error.code, StatusCode::InvalidArgument);
}

#[test]
fn whole_frame_world_border_ffi_copies_and_rejects_malformed_payloads() {
    let mut borders = vec![border_quad_request()];
    let request = whole_frame_request_with_borders(&borders, &[]);
    let (_generation, _target, frame, gui) =
        unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()).unwrap() };
    borders[0].p0_x = 99.0;
    assert_eq!(frame.border_quads.len(), 1);
    assert_eq!(frame.border_quads[0].vertices[0][0], -1.0);
    assert_eq!(frame.border_quads[0].texture_id, 1);
    assert_eq!(frame.border_quads[0].uv_region[3], -4.0);
    assert!(gui.is_empty());

    borders[0] = border_quad_request();
    borders[0].texture_id = 99;
    let request = whole_frame_request_with_borders(&borders, &[]);
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("unknown border texture must fail");
    assert_eq!(error.code, StatusCode::UnknownEnum);

    borders[0] = border_quad_request();
    borders[0].byte_size -= 4;
    let request = whole_frame_request_with_borders(&borders, &[]);
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("malformed border quad must fail");
    assert_eq!(error.code, StatusCode::InvalidArgument);
}

#[test]
fn whole_frame_world_material_ffi_copies_and_rejects_malformed_payloads() {
    let mut materials = vec![material_quad_request()];
    let request = whole_frame_request_with_materials(&materials);
    let (_generation, _target, frame, gui) =
        unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()).unwrap() };
    materials[0].p0_x = 99.0;
    assert_eq!(frame.material_quads.len(), 1);
    assert_eq!(frame.material_quads[0].vertices[0][0], -1.0);
    assert_eq!(frame.material_quads[0].uvs[2], [1.0, 1.0]);
    assert_eq!(
        frame.material_quads[0].material_id,
        WORLD_MATERIAL_ID_OPAQUE_TEXTURED
    );
    assert!(gui.is_empty());

    materials[0] = material_quad_request();
    materials[0].material_id = 99;
    let request = whole_frame_request_with_materials(&materials);
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("unknown material id must fail validation");
    assert_eq!(error.code, StatusCode::UnknownEnum);

    materials[0] = material_quad_request();
    materials[0].byte_size -= 4;
    let request = whole_frame_request_with_materials(&materials);
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("malformed material quad must fail");
    assert_eq!(error.code, StatusCode::InvalidArgument);
}

#[test]
fn compact_world_material_ffi_decodes_copies_and_deduplicates_table() {
    let mut table = vec![material_table_record()];
    table[0].source_program = WORLD_MATERIAL_SOURCE_TEXTURED;
    let mut materials = vec![
        compact_material_quad_request(),
        compact_material_quad_request(),
    ];
    materials[1].p0_x = -2.0;
    materials[1].color_argb = 0xff80_4020;
    materials[1].source_color_argb = 0xff90_5030;
    materials[1].packed_light = 0x00e0_00b0;
    materials[1].source_uv_space = WORLD_MATERIAL_SOURCE_UV_MINECRAFT_BLOCK_ATLAS;
    let request = whole_frame_request_with_compact_materials(&table, &materials);
    let (_generation, _target, frame, gui) =
        unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()).unwrap() };
    materials[0].p0_x = 99.0;
    assert_eq!(2, frame.material_quads.len());
    assert_eq!(
        WORLD_MATERIAL_ID_OPAQUE_TEXTURED,
        frame.material_quads[0].material_id
    );
    assert_eq!(
        WORLD_MATERIAL_TEXTURE_STONE,
        frame.material_quads[0].texture_id
    );
    assert_eq!(-1.0, frame.material_quads[0].vertices[0][0]);
    assert_eq!(-2.0, frame.material_quads[1].vertices[0][0]);
    assert_eq!(0xff80_4020, frame.material_quads[1].color_argb);
    assert_eq!(
        WORLD_MATERIAL_SOURCE_TEXTURED,
        frame.material_quads[0].source_program
    );
    assert_eq!(0xffff_ffff, frame.material_quads[0].source_color_argb);
    assert_eq!(0xff90_5030, frame.material_quads[1].source_color_argb);
    assert_eq!(0x00e0_00b0, frame.material_quads[1].packed_light);
    assert_eq!(
        WORLD_MATERIAL_SOURCE_UV_MINECRAFT_BLOCK_ATLAS,
        frame.material_quads[1].source_uv_space
    );
    assert_eq!([1.0, 1.0], frame.material_quads[0].uvs[2]);
    assert!(gui.is_empty());
}

#[test]
fn compact_world_material_ffi_accepts_the_explicit_weather_source_family() {
    let mut table = vec![material_table_record()];
    table[0].material_mode = WORLD_MATERIAL_MODE_TRANSLUCENT;
    table[0].material_id = WORLD_MATERIAL_ID_TRANSLUCENT_TEXTURED;
    table[0].depth_policy = WORLD_DEPTH_POLICY_TEST_NO_WRITE;
    table[0].cull_policy = WORLD_CULL_NONE;
    table[0].source_program = WORLD_MATERIAL_SOURCE_WEATHER;
    let compact = vec![compact_material_quad_request()];
    let request = whole_frame_request_with_compact_materials(&table, &compact);
    let (_generation, _target, frame, _gui) =
        unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()).unwrap() };

    assert_eq!(1, frame.material_quads.len());
    assert_eq!(
        WORLD_MATERIAL_SOURCE_WEATHER,
        frame.material_quads[0].source_program
    );
    assert_eq!(
        WORLD_MATERIAL_MODE_TRANSLUCENT,
        frame.material_quads[0].material_mode
    );
    assert_eq!(
        WORLD_DEPTH_POLICY_TEST_NO_WRITE,
        frame.material_quads[0].depth_policy
    );
}

#[test]
fn compact_world_material_ffi_rejects_weather_atlas_uv_semantics() {
    let mut table = vec![material_table_record()];
    table[0].material_mode = WORLD_MATERIAL_MODE_TRANSLUCENT;
    table[0].material_id = WORLD_MATERIAL_ID_TRANSLUCENT_TEXTURED;
    table[0].depth_policy = WORLD_DEPTH_POLICY_TEST_NO_WRITE;
    table[0].cull_policy = WORLD_CULL_NONE;
    table[0].source_program = WORLD_MATERIAL_SOURCE_WEATHER;
    let mut compact = vec![compact_material_quad_request()];
    compact[0].source_uv_space = WORLD_MATERIAL_SOURCE_UV_MINECRAFT_BLOCK_ATLAS;
    let request = whole_frame_request_with_compact_materials(&table, &compact);
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("weather must not reinterpret standalone textures as atlas coordinates");
    assert_eq!(StatusCode::UnsupportedFeature, error.code);
    assert!(error.message.contains("weather and cloud material quads"));
}

#[test]
fn compact_world_material_ffi_accepts_the_experience_orb_semantic_sheet() {
    let mut table = vec![material_table_record()];
    table[0].material_mode = WORLD_MATERIAL_MODE_TRANSLUCENT;
    table[0].material_id = WORLD_MATERIAL_ID_TRANSLUCENT_TEXTURED;
    table[0].texture_id = WORLD_MATERIAL_TEXTURE_EXPERIENCE_ORB;
    table[0].depth_policy = WORLD_DEPTH_POLICY_TEST_NO_WRITE;
    let compact = vec![compact_material_quad_request()];
    let request = whole_frame_request_with_compact_materials(&table, &compact);
    let (_generation, _target, frame, _gui) =
        unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()).unwrap() };

    assert_eq!(1, frame.material_quads.len());
    assert_eq!(
        WORLD_MATERIAL_TEXTURE_EXPERIENCE_ORB,
        frame.material_quads[0].texture_id
    );
    assert_eq!(
        WORLD_MATERIAL_MODE_TRANSLUCENT,
        frame.material_quads[0].material_mode
    );
}

#[test]
fn compact_world_material_ffi_admits_owned_atlas_only_with_atlas_uvs() {
    let mut table = vec![material_table_record()];
    table[0].texture_id = WORLD_MESH_TEXTURE_TERRAIN_BLOCK_ATLAS;
    let local = vec![compact_material_quad_request()];
    let request = whole_frame_request_with_compact_materials(&table, &local);
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("the runtime atlas must not be interpreted with local UV semantics");
    assert_eq!(StatusCode::InvalidArgument, error.code);
    assert!(error
        .message
        .contains("requires Minecraft atlas UV semantics"));

    let mut atlas = vec![compact_material_quad_request()];
    atlas[0].source_uv_space = WORLD_MATERIAL_SOURCE_UV_MINECRAFT_BLOCK_ATLAS;
    let request = whole_frame_request_with_compact_materials(&table, &atlas);
    let (_generation, _target, frame, _gui) =
        unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()).unwrap() };
    assert_eq!(
        WORLD_MESH_TEXTURE_TERRAIN_BLOCK_ATLAS,
        frame.material_quads[0].texture_id
    );
}

#[test]
fn compact_world_material_ffi_accepts_the_explicit_cloud_source_family() {
    let mut table = vec![material_table_record()];
    table[0].material_mode = WORLD_MATERIAL_MODE_TRANSLUCENT;
    table[0].material_id = WORLD_MATERIAL_ID_TRANSLUCENT_TEXTURED;
    table[0].depth_policy = WORLD_DEPTH_POLICY_TEST_NO_WRITE;
    table[0].cull_policy = WORLD_CULL_NONE;
    table[0].source_program = WORLD_MATERIAL_SOURCE_CLOUDS;
    let request =
        whole_frame_request_with_compact_materials(&table, &[compact_material_quad_request()]);
    let (_generation, _target, frame, _gui) =
        unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()).unwrap() };

    assert_eq!(
        WORLD_MATERIAL_SOURCE_CLOUDS,
        frame.material_quads[0].source_program
    );
}

#[test]
fn compact_world_material_ffi_rejects_cloud_atlas_uv_semantics() {
    let mut table = vec![material_table_record()];
    table[0].material_mode = WORLD_MATERIAL_MODE_TRANSLUCENT;
    table[0].material_id = WORLD_MATERIAL_ID_TRANSLUCENT_TEXTURED;
    table[0].depth_policy = WORLD_DEPTH_POLICY_TEST_NO_WRITE;
    table[0].cull_policy = WORLD_CULL_NONE;
    table[0].source_program = WORLD_MATERIAL_SOURCE_CLOUDS;
    let mut compact = vec![compact_material_quad_request()];
    compact[0].source_uv_space = WORLD_MATERIAL_SOURCE_UV_MINECRAFT_BLOCK_ATLAS;
    let request = whole_frame_request_with_compact_materials(&table, &compact);
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("clouds must not reinterpret standalone textures as atlas coordinates");
    assert_eq!(StatusCode::UnsupportedFeature, error.code);
    assert!(error.message.contains("weather and cloud material quads"));
}

#[test]
fn compact_world_material_ffi_accepts_the_explicit_particle_source_family() {
    let mut table = vec![material_table_record()];
    table[0].material_mode = WORLD_MATERIAL_MODE_TRANSLUCENT;
    table[0].material_id = WORLD_MATERIAL_ID_TRANSLUCENT_TEXTURED;
    table[0].depth_policy = WORLD_DEPTH_POLICY_TEST_NO_WRITE;
    table[0].cull_policy = WORLD_CULL_NONE;
    table[0].source_program = WORLD_MATERIAL_SOURCE_PARTICLES;
    let request =
        whole_frame_request_with_compact_materials(&table, &[compact_material_quad_request()]);
    let (_generation, _target, frame, _gui) =
        unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()).unwrap() };

    assert_eq!(
        WORLD_MATERIAL_SOURCE_PARTICLES,
        frame.material_quads[0].source_program
    );
}

#[test]
fn compact_world_material_ffi_accepts_the_explicit_entity_model_source_family() {
    let mut table = vec![material_table_record()];
    table[0].material_mode = WORLD_MATERIAL_MODE_TRANSLUCENT;
    table[0].material_id = WORLD_MATERIAL_ID_TRANSLUCENT_TEXTURED;
    table[0].depth_policy = WORLD_DEPTH_POLICY_TEST_NO_WRITE;
    table[0].cull_policy = WORLD_CULL_NONE;
    table[0].source_program = WORLD_MATERIAL_SOURCE_ENTITY_MODEL;
    let request =
        whole_frame_request_with_compact_materials(&table, &[compact_material_quad_request()]);
    let (_generation, _target, frame, _gui) =
        unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()).unwrap() };

    assert_eq!(
        WORLD_MATERIAL_SOURCE_ENTITY_MODEL,
        frame.material_quads[0].source_program
    );
}

#[test]
fn compact_world_material_ffi_rejects_malformed_indexes_and_preserves_mixed_full_records() {
    let table = vec![material_table_record()];
    let mut compact = vec![compact_material_quad_request()];
    compact[0].material_index = 1;
    let request = whole_frame_request_with_compact_materials(&table, &compact);
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("out-of-range compact material index must fail");
    assert_eq!(StatusCode::InvalidArgument, error.code);

    compact[0] = compact_material_quad_request();
    compact[0].byte_size -= 4;
    let request = whole_frame_request_with_compact_materials(&table, &compact);
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("malformed compact material record must fail");
    assert_eq!(StatusCode::InvalidArgument, error.code);

    let mut bad_table = vec![material_table_record()];
    bad_table[0].material_mode = 999;
    let request =
        whole_frame_request_with_compact_materials(&bad_table, &[compact_material_quad_request()]);
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("unknown compact material mode must fail");
    assert_eq!(StatusCode::UnknownEnum, error.code);

    let mut bad_uv_space = vec![compact_material_quad_request()];
    bad_uv_space[0].source_uv_space = 99;
    let request = whole_frame_request_with_compact_materials(&table, &bad_uv_space);
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("unknown compact material source UV space must fail");
    assert_eq!(StatusCode::UnknownEnum, error.code);

    let mut bad_source_table = vec![material_table_record()];
    bad_source_table[0].source_program = 77;
    let request = whole_frame_request_with_compact_materials(
        &bad_source_table,
        &[compact_material_quad_request()],
    );
    let error = unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }
        .expect_err("unknown compact material source program must fail");
    assert_eq!(StatusCode::UnknownEnum, error.code);

    let legacy = vec![material_quad_request()];
    let compact_record = compact_material_quad_request();
    let mut request = whole_frame_request_with_compact_materials(&table, &[compact_record]);
    request.world_material_quads = FfiSlice {
        ptr: legacy.as_ptr(),
        count: legacy.len() as u64,
    };
    let (_generation, _target, frame, _gui) =
        unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()).unwrap() };
    assert_eq!(2, frame.material_quads.len());
    assert_eq!(
        legacy[0].vertex0_color_argb,
        frame.material_quads[0].vertex_color_argb[0]
    );
    assert_eq!(
        compact_record.source_color_argb,
        frame.material_quads[1].vertex_color_argb[0]
    );
}

#[test]
fn semantic_gui_asset_ffi_copies_payload_memory() {
    let mut bytes = vec![7u8, 8, 9, 10];
    let assets = vec![FfiGuiAssetPayload {
        byte_size: size_of::<FfiGuiAssetPayload>() as u32,
        sprite_id: 1,
        png_bytes: FfiBytes {
            ptr: bytes.as_ptr(),
            len: bytes.len() as u64,
        },
    }];
    let request = asset_update_request(&assets);
    let (_generation, owned) =
        unsafe { decode_gui_asset_update(&request, test_capabilities()).unwrap() };
    bytes.fill(0);
    assert_eq!(vec![7u8, 8, 9, 10], owned[0].png_bytes);
}

#[test]
fn semantic_gui_asset_ffi_rejects_duplicates_and_bad_item_size() {
    let bytes = [1u8, 2, 3];
    let mut assets = vec![
        FfiGuiAssetPayload {
            byte_size: size_of::<FfiGuiAssetPayload>() as u32,
            sprite_id: 1,
            png_bytes: FfiBytes {
                ptr: bytes.as_ptr(),
                len: bytes.len() as u64,
            },
        },
        FfiGuiAssetPayload {
            byte_size: size_of::<FfiGuiAssetPayload>() as u32,
            sprite_id: 1,
            png_bytes: FfiBytes {
                ptr: bytes.as_ptr(),
                len: bytes.len() as u64,
            },
        },
    ];
    let duplicate =
        unsafe { decode_gui_asset_update(&asset_update_request(&assets), test_capabilities()) }
            .unwrap_err();
    assert_eq!(StatusCode::InvalidArgument, duplicate.code);
    assets[1].sprite_id = 2;
    assets[1].byte_size -= 4;
    let malformed =
        unsafe { decode_gui_asset_update(&asset_update_request(&assets), test_capabilities()) }
            .unwrap_err();
    assert_eq!(StatusCode::InvalidArgument, malformed.code);
}

#[test]
fn semantic_gui_asset_ffi_rejects_oversized_batches_before_copying() {
    let bytes = [1u8, 2, 3];
    let asset = FfiGuiAssetPayload {
        byte_size: size_of::<FfiGuiAssetPayload>() as u32,
        sprite_id: 7,
        png_bytes: FfiBytes {
            ptr: bytes.as_ptr(),
            len: bytes.len() as u64,
        },
    };
    let assets = vec![asset; 4_097];
    let error =
        unsafe { decode_gui_asset_update(&asset_update_request(&assets), test_capabilities()) }
            .expect_err("oversized GUI asset batches must fail before copying");
    assert_eq!(StatusCode::LengthOverflow, error.code);
}

#[test]
fn semantic_raw_gui_image_ffi_copies_and_validates_pixels() {
    let mut pixels = vec![0u8, 64, 128, 255];
    let assets = [FfiGuiRawImageAssetPayload { sampling_filter: 0, sampling_address: 0,
        byte_size: size_of::<FfiGuiRawImageAssetPayload>() as u32,
        format: 1,
        asset_id: 17,
        width: 2,
        height: 2,
        pixels: FfiBytes {
            ptr: pixels.as_ptr(),
            len: pixels.len() as u64,
        },
    }];
    let request = FfiGuiRawImageUpdateRequest {
        header: FfiHeader {
            version: FFI_ABI_VERSION,
            byte_size: size_of::<FfiGuiRawImageUpdateRequest>() as u32,
        },
        generation: 12,
        assets: FfiSlice {
            ptr: assets.as_ptr(),
            count: assets.len() as u64,
        },
        negotiated_feature_bits: 0,
    };
    let (generation, owned) =
        unsafe { decode_gui_raw_image_update(&request, test_capabilities()).unwrap() };
    pixels.fill(3);
    assert_eq!(12, generation);
    assert_eq!(17, owned[0].asset_id);
    assert_eq!(vec![0, 64, 128, 255], owned[0].pixels);

    for filter in 0..=3 {
        for address in 0..=3 {
            let mut candidate = assets[0];
            candidate.sampling_filter = filter;
            candidate.sampling_address = address;
            let changed = FfiGuiRawImageUpdateRequest {
                assets: FfiSlice { ptr: &candidate, count: 1 }, ..request
            };
            let result = unsafe { decode_gui_raw_image_update(&changed, test_capabilities()) };
            assert_eq!((filter == 0 && address == 0) || ((1..=2).contains(&filter) && (1..=2).contains(&address)), result.is_ok());
            if filter > 0 && filter <= 2 && address > 0 && address <= 2 {
                let sampling = result.unwrap().1[0].sampling.unwrap();
                assert_eq!(filter == 2, sampling.0 == super::super::resources::SamplerFilter::Linear);
                assert_eq!(address == 2, sampling.1 == super::super::resources::SamplerAddressMode::ClampToEdge);
            }
        }
    }
    let mut old_layout = assets[0];
    old_layout.byte_size -= 8;
    let old_request = FfiGuiRawImageUpdateRequest { assets: FfiSlice { ptr: &old_layout, count: 1 }, ..request };
    assert!(unsafe { decode_gui_raw_image_update(&old_request, test_capabilities()) }.is_err());

    let mut malformed = assets[0];
    malformed.format = 99;
    let invalid = FfiGuiRawImageUpdateRequest {
        assets: FfiSlice {
            ptr: &malformed,
            count: 1,
        },
        ..request
    };
    let error = unsafe { decode_gui_raw_image_update(&invalid, test_capabilities()) }.unwrap_err();
    assert_eq!(StatusCode::UnknownEnum, error.code);
}

#[test]
fn semantic_raw_gui_image_ffi_preserves_frozen_rgba8_metadata() {
    let pixels = [152u8, 152, 152, 255];
    let assets = [FfiGuiRawImageAssetPayload { sampling_filter: 0, sampling_address: 0,
        byte_size: size_of::<FfiGuiRawImageAssetPayload>() as u32,
        format: 2,
        asset_id: 18,
        width: 1,
        height: 1,
        pixels: FfiBytes {
            ptr: pixels.as_ptr(),
            len: pixels.len() as u64,
        },
    }];
    let request = FfiGuiRawImageUpdateRequest {
        header: FfiHeader {
            version: FFI_ABI_VERSION,
            byte_size: size_of::<FfiGuiRawImageUpdateRequest>() as u32,
        },
        generation: 13,
        assets: FfiSlice {
            ptr: assets.as_ptr(),
            count: assets.len() as u64,
        },
        negotiated_feature_bits: 0,
    };
    let (_, owned) = unsafe { decode_gui_raw_image_update(&request, test_capabilities()).unwrap() };
    assert_eq!(GuiRawImageFormat::Rgba8, owned[0].format);
    assert_eq!(pixels, owned[0].pixels.as_slice());
}

#[test]
fn semantic_raw_gui_image_ffi_rejects_oversized_dimensions_before_copying() {
    let pixels = [0u8; 4];
    let asset = FfiGuiRawImageAssetPayload { sampling_filter: 0, sampling_address: 0,
        byte_size: size_of::<FfiGuiRawImageAssetPayload>() as u32,
        format: 2,
        asset_id: 21,
        width: i32::MAX,
        height: i32::MAX,
        pixels: FfiBytes {
            ptr: pixels.as_ptr(),
            len: pixels.len() as u64,
        },
    };
    let request = FfiGuiRawImageUpdateRequest {
        header: FfiHeader {
            version: FFI_ABI_VERSION,
            byte_size: size_of::<FfiGuiRawImageUpdateRequest>() as u32,
        },
        generation: 13,
        assets: FfiSlice {
            ptr: &asset,
            count: 1,
        },
        negotiated_feature_bits: 0,
    };
    let error = unsafe { decode_gui_raw_image_update(&request, test_capabilities()) }
        .expect_err("oversized raw GUI dimensions must fail before pixel copying");
    assert_eq!(StatusCode::LengthOverflow, error.code);
}

#[test]
fn world_border_asset_ffi_copies_payload_memory() {
    let mut bytes = vec![11u8, 12, 13, 14];
    let request = world_border_asset_update_request(&bytes);
    let (generation, owned) =
        unsafe { decode_world_border_asset_update(&request, test_capabilities()).unwrap() };
    bytes.fill(0);
    assert_eq!(9, generation);
    assert_eq!(1, owned.texture_id);
    assert_eq!(vec![11u8, 12, 13, 14], owned.png_bytes);
}

#[test]
fn world_border_asset_ffi_rejects_bad_generation_and_size() {
    let bytes = [1u8, 2, 3];
    let mut request = world_border_asset_update_request(&bytes);
    request.generation = 0;
    let bad_generation =
        unsafe { decode_world_border_asset_update(&request, test_capabilities()) }.unwrap_err();
    assert_eq!(StatusCode::InvalidArgument, bad_generation.code);

    request = world_border_asset_update_request(&bytes);
    request.header.byte_size -= 4;
    let bad_size =
        unsafe { decode_world_border_asset_update(&request, test_capabilities()) }.unwrap_err();
    assert_eq!(StatusCode::InvalidArgument, bad_size.code);
}

#[test]
fn world_crack_asset_ffi_copies_payload_memory() {
    let mut bytes = vec![21u8, 22, 23, 24];
    let assets = vec![FfiWorldCrackAssetPayload {
        byte_size: size_of::<FfiWorldCrackAssetPayload>() as u32,
        stage: 4,
        png_bytes: FfiBytes {
            ptr: bytes.as_ptr(),
            len: bytes.len() as u64,
        },
    }];
    let request = world_crack_asset_update_request(&assets);
    let (generation, owned) =
        unsafe { decode_world_crack_asset_update(&request, test_capabilities()).unwrap() };
    bytes.fill(0);
    assert_eq!(9, generation);
    assert_eq!(4, owned[0].stage);
    assert_eq!(vec![21u8, 22, 23, 24], owned[0].png_bytes);
}

#[test]
fn world_crack_asset_ffi_rejects_duplicates_bad_stage_and_size() {
    let bytes = [1u8, 2, 3];
    let mut assets = vec![
        FfiWorldCrackAssetPayload {
            byte_size: size_of::<FfiWorldCrackAssetPayload>() as u32,
            stage: 4,
            png_bytes: FfiBytes {
                ptr: bytes.as_ptr(),
                len: bytes.len() as u64,
            },
        },
        FfiWorldCrackAssetPayload {
            byte_size: size_of::<FfiWorldCrackAssetPayload>() as u32,
            stage: 4,
            png_bytes: FfiBytes {
                ptr: bytes.as_ptr(),
                len: bytes.len() as u64,
            },
        },
    ];
    let duplicate = unsafe {
        decode_world_crack_asset_update(
            &world_crack_asset_update_request(&assets),
            test_capabilities(),
        )
    }
    .unwrap_err();
    assert_eq!(StatusCode::InvalidArgument, duplicate.code);

    assets[1].stage = 10;
    let bad_stage = unsafe {
        decode_world_crack_asset_update(
            &world_crack_asset_update_request(&assets),
            test_capabilities(),
        )
    }
    .unwrap_err();
    assert_eq!(StatusCode::UnknownEnum, bad_stage.code);

    assets[1].stage = 5;
    assets[1].byte_size -= 4;
    let malformed = unsafe {
        decode_world_crack_asset_update(
            &world_crack_asset_update_request(&assets),
            test_capabilities(),
        )
    }
    .unwrap_err();
    assert_eq!(StatusCode::InvalidArgument, malformed.code);
}

#[test]
fn world_material_asset_ffi_copies_payload_memory() {
    let mut bytes = vec![31u8, 32, 33, 34];
    let assets = vec![FfiWorldMaterialAssetPayload {
        byte_size: size_of::<FfiWorldMaterialAssetPayload>() as u32,
        texture_id: WORLD_MATERIAL_TEXTURE_STONE,
        png_bytes: FfiBytes {
            ptr: bytes.as_ptr(),
            len: bytes.len() as u64,
        },
    }];
    let request = world_material_asset_update_request(&assets);
    let (generation, owned) =
        unsafe { decode_world_material_asset_update(&request, test_capabilities()).unwrap() };
    bytes.fill(0);
    assert_eq!(9, generation);
    assert_eq!(WORLD_MATERIAL_TEXTURE_STONE, owned[0].texture_id);
    assert_eq!(vec![31u8, 32, 33, 34], owned[0].png_bytes);
}

#[test]
fn world_material_asset_ffi_rejects_duplicates_and_bad_item_size() {
    let bytes = [1u8, 2, 3];
    let mut assets = vec![
        FfiWorldMaterialAssetPayload {
            byte_size: size_of::<FfiWorldMaterialAssetPayload>() as u32,
            texture_id: WORLD_MATERIAL_TEXTURE_STONE,
            png_bytes: FfiBytes {
                ptr: bytes.as_ptr(),
                len: bytes.len() as u64,
            },
        },
        FfiWorldMaterialAssetPayload {
            byte_size: size_of::<FfiWorldMaterialAssetPayload>() as u32,
            texture_id: WORLD_MATERIAL_TEXTURE_STONE,
            png_bytes: FfiBytes {
                ptr: bytes.as_ptr(),
                len: bytes.len() as u64,
            },
        },
    ];
    let duplicate = unsafe {
        decode_world_material_asset_update(
            &world_material_asset_update_request(&assets),
            test_capabilities(),
        )
    }
    .unwrap_err();
    assert_eq!(StatusCode::InvalidArgument, duplicate.code);

    assets[1].texture_id = 2;
    assets[1].byte_size -= 4;
    let malformed = unsafe {
        decode_world_material_asset_update(
            &world_material_asset_update_request(&assets),
            test_capabilities(),
        )
    }
    .unwrap_err();
    assert_eq!(StatusCode::InvalidArgument, malformed.code);
}

#[test]
fn world_material_asset_ffi_rejects_oversized_batches_before_copying() {
    let bytes = [1u8, 2, 3];
    let asset = FfiWorldMaterialAssetPayload {
        byte_size: size_of::<FfiWorldMaterialAssetPayload>() as u32,
        texture_id: WORLD_MATERIAL_TEXTURE_STONE,
        png_bytes: FfiBytes {
            ptr: bytes.as_ptr(),
            len: bytes.len() as u64,
        },
    };
    let assets = vec![asset; 4_097];
    let error = unsafe {
        decode_world_material_asset_update(
            &world_material_asset_update_request(&assets),
            test_capabilities(),
        )
    }
    .expect_err("oversized world material batches must fail before copying");
    assert_eq!(StatusCode::LengthOverflow, error.code);
}

#[test]
fn whole_frame_terrain_placement_is_lowered_from_owned_full_precision_inputs() {
    use crate::render::vulkanic::world_primitive_frontend::WORLD_MESH_SECTION_ALL;
    let mut instance = mesh_instance();
    instance.stratum = WORLD_STRATUM_TERRAIN;
    instance.mesh_section_index = WORLD_MESH_SECTION_ALL;
    instance.terrain_placement_mode = 1;
    instance.terrain_origin = [112, 80, 528];
    instance.terrain_camera = [150.5, 101.62, 530.5];
    let decode = |value| unsafe {
        decode_whole_frame_submit(
            &whole_frame_request_with_mesh_instances(&[value]),
            test_vulkan_capabilities(),
        )
    };
    let (_, _, frame, _) = decode(instance).unwrap();
    assert_eq!(&frame.mesh_instances[0].transform[12..15], &[-38.5, -21.6199951171875, -2.5]);
    instance.terrain_camera[1] = 102.62;
    assert_eq!(frame.mesh_instances[0].transform[13], -21.6199951171875);
    let valid = instance;
    for case in 0..7 {
        let mut invalid = valid;
        match case {
            0 => invalid.terrain_placement_mode = 2,
            1 => invalid.terrain_placement_mode = 0,
            2 => invalid.transform[12] = 1.0,
            3 => invalid.terrain_origin[0] = 113,
            4 => invalid.terrain_camera[0] = f64::NAN,
            5 => invalid.stratum = WORLD_STRATUM_OPAQUE_TEXTURED_GEOMETRY,
            _ => invalid.mesh_section_index = 0,
        }
        assert_eq!(decode(invalid).unwrap_err().code, StatusCode::InvalidArgument, "case {case}");
    }
}

#[test]
fn whole_frame_world_mesh_ffi_copies_and_rejects_malformed_payloads() {
    let mut instances = vec![mesh_instance()];
    instances[0].outline_color_argb = 0x80_10_20_30;
    let request = whole_frame_request_with_mesh_instances(&instances);
    let (_generation, _target, frame, _gui) =
        unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()).unwrap() };
    instances[0].mesh_key = 99;
    instances[0].entity_id = 50_004;
    instances[0].entity_color_argb = 0xff_12_34_56;
    instances[0].transform[12] = 42.0;

    assert_eq!(1, frame.mesh_instances.len());
    assert_eq!(44, frame.mesh_instances[0].mesh_key);
    assert_eq!(9, frame.mesh_instances[0].mesh_generation);
    assert_eq!(0, frame.mesh_instances[0].entity_id);
    assert_eq!(0, frame.mesh_instances[0].entity_color_argb);
    assert_eq!(0x80_10_20_30, frame.mesh_instances[0].outline_color_argb);
    assert_eq!(0.0, frame.mesh_instances[0].transform[12]);

    instances[0] = mesh_instance();
    instances[0].stratum = WORLD_STRATUM_TERRAIN;
    let (_generation, _target, frame, _gui) = unsafe {
        decode_whole_frame_submit(
            &whole_frame_request_with_mesh_instances(&instances),
            test_vulkan_capabilities(),
        )
        .unwrap()
    };
    assert_eq!(WORLD_STRATUM_TERRAIN, frame.mesh_instances[0].stratum);

    instances[0].stratum = 77;
    let error = unsafe {
        decode_whole_frame_submit(
            &whole_frame_request_with_mesh_instances(&instances),
            test_vulkan_capabilities(),
        )
    }
    .unwrap_err();
    assert_eq!(StatusCode::UnknownEnum, error.code);

    instances[0] = mesh_instance();
    instances[0].byte_size -= 4;
    let error = unsafe {
        decode_whole_frame_submit(
            &whole_frame_request_with_mesh_instances(&instances),
            test_vulkan_capabilities(),
        )
    }
    .unwrap_err();
    assert_eq!(StatusCode::InvalidArgument, error.code);
}

#[test]
fn world_mesh_asset_ffi_retires_gui_views_before_replacing_their_owner() {
    use crate::render::vulkanic::gui_atlas_reference::GuiAtlasReference;
    let create = FfiContextCreateRequest {
        header: FfiHeader { version: FFI_ABI_VERSION, byte_size: size_of::<FfiContextCreateRequest>() as u32 },
        backend_kind: 1, tracy_enabled: 0,
        label: FfiBytes { ptr: b"GUI atlas FFI lifetime".as_ptr(), len: 22 },
    };
    let mut result = FfiContextResult::default();
    assert_eq!(unsafe { mattmc_vulkanic_gal_context_create(&create, &mut result) }, 0);
    let context_id = result.context_id;
    let mut png_bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_bytes, 2, 2);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(&[255; 16]).unwrap();
    }
    let mut texture = FfiWorldMeshTextureAssetPayload {
        byte_size: size_of::<FfiWorldMeshTextureAssetPayload>() as u32,
        texture_id: WORLD_MATERIAL_TEXTURE_STONE,
        sampling_filter: 0, sampling_address: 0, requested_mip_levels: 1,
        png_bytes: FfiBytes { ptr: png_bytes.as_ptr(), len: png_bytes.len() as u64 },
        mip_png_bytes: FfiSlice { ptr: ptr::null(), count: 0 },
        frame_width: 0, frame_height: 0, frame_count: 1, frame_ticks: 1,
        animation_flags: 0, frame_row_size: 0, interpolation_policy: 0, reserved0: 0,
        animation_frames: FfiSlice { ptr: ptr::null(), count: 0 },
    };
    let publish = |generation, texture: &FfiWorldMeshTextureAssetPayload| {
        let mut request = world_mesh_asset_update_request(&[], std::slice::from_ref(texture));
        request.generation = generation;
        request.negotiated_feature_bits = result.supported_feature_bits;
        unsafe { mattmc_vulkanic_gal_world_mesh_update_assets(context_id, &request, ptr::null_mut()) }
    };
    assert_eq!(publish(1, &texture), 0);
    let reference = with_registry_mut(|registry| {
        let context = registry.contexts.get_mut(&context_id).unwrap();
        let atlas = context.world_primitive_frontend.accepted_gui_atlas_incarnation(texture.texture_id).unwrap();
        let reference = GuiAtlasReference { asset_id: 101, atlas, x: 0, y: 0, width: 2, height: 2 };
        context.gui_frontend.stage_owned_atlas_references(&mut context.gal, &context.world_primitive_frontend, 1, &[reference]).unwrap();
        context.gui_frontend.prepare_owned_atlas_view(&mut context.gal, &mut context.world_primitive_frontend, 101).unwrap();
        reference
    });
    let valid_bytes = texture.png_bytes;
    texture.png_bytes = FfiBytes { ptr: b"bad".as_ptr(), len: 3 };
    assert_ne!(publish(2, &texture), 0);
    with_registry_mut(|registry| {
        let context = registry.contexts.get_mut(&context_id).unwrap();
        assert_eq!(context.world_primitive_frontend.accepted_gui_atlas_incarnation(texture.texture_id), Some(reference.atlas));
        context.gui_frontend.prepare_owned_atlas_view(&mut context.gal, &mut context.world_primitive_frontend, 101).unwrap();
    });
    texture.png_bytes = valid_bytes;
    assert_eq!(publish(2, &texture), 0);
    let (dependent, sampler) = with_registry_mut(|registry| {
        let context = registry.contexts.get_mut(&context_id).unwrap();
        assert!(context.gui_frontend.prepare_owned_atlas_view(&mut context.gal, &mut context.world_primitive_frontend, 101).is_err());
        let replacement = GuiAtlasReference {
            atlas: context.world_primitive_frontend.accepted_gui_atlas_incarnation(texture.texture_id).unwrap(),
            ..reference
        };
        context.gui_frontend.stage_owned_atlas_references(&mut context.gal, &context.world_primitive_frontend, 2, &[replacement]).unwrap();
        let view = context.gui_frontend.prepare_owned_atlas_view(&mut context.gal, &mut context.world_primitive_frontend, 101).unwrap();
        let sampler = context.gal.create_sampler(SamplerDesc {
            label: "test dependent sampler".into(), min_filter: SamplerFilter::Nearest,
            mag_filter: SamplerFilter::Nearest, mip_filter: SamplerFilter::Nearest,
            address_u: SamplerAddressMode::ClampToEdge, address_v: SamplerAddressMode::ClampToEdge,
            address_w: SamplerAddressMode::ClampToEdge, comparison: None,
        }).unwrap();
        let dependent = context.gal.create_combined_texture_sampler(
            crate::render::vulkanic::resources::CombinedTextureSamplerDesc {
                label: "test external dependent".into(), texture_view: view, sampler,
            }).unwrap();
        (dependent, sampler)
    });
    assert_eq!(unsafe { mattmc_vulkanic_gal_context_destroy(context_id, ptr::null_mut()) },
        StatusCode::DependencyViolation as i32);
    with_registry_mut(|registry| {
        let context = registry.contexts.get_mut(&context_id).expect("failed reset must retain context ownership for retry");
        context.gal.destroy(dependent).unwrap();
        context.gal.destroy(sampler).unwrap();
        context.gui_frontend.reset(&mut context.gal).unwrap();
        context.world_primitive_frontend.reset(&mut context.gal);
        context.gal.retire_through(context.gal.latest_submission_id()).unwrap();
        assert_eq!(context.gal.metrics().resource_creates, context.gal.metrics().resource_destroys,
            "the real FFI update must release GUI dependencies before owner destruction, not leak rejected destroys");
    });
    assert_eq!(unsafe { mattmc_vulkanic_gal_context_destroy(context_id, ptr::null_mut()) }, 0);
}

#[test]
fn world_mesh_asset_ffi_copies_payload_memory() {
    let mut vertices = vec![mesh_vertex()];
    let mut index_bytes = vec![0u8, 0, 1, 0, 2, 0];
    let sections = vec![mesh_section(123)];
    let mut entity_identity = b"minecraft:arrow".to_vec();
    let mut meshes = vec![mesh_asset(&vertices, &index_bytes, &sections)];
    meshes[0].entity_identity_utf8 = FfiBytes {
        ptr: entity_identity.as_ptr(),
        len: entity_identity.len() as u64,
    };
    let mut png = vec![41u8, 42, 43, 44];
    let textures = vec![FfiWorldMeshTextureAssetPayload {
        sampling_filter: 2,
        sampling_address: 2,
        requested_mip_levels: 1,
        byte_size: size_of::<FfiWorldMeshTextureAssetPayload>() as u32,
        texture_id: 123,
        png_bytes: FfiBytes {
            ptr: png.as_ptr(),
            len: png.len() as u64,
        },
        mip_png_bytes: FfiSlice { ptr: std::ptr::null(), count: 0 },
        frame_width: 0,
        frame_height: 0,
        frame_count: 1,
        frame_ticks: 1,
        animation_flags: 0,
        frame_row_size: 0,
        interpolation_policy: 0,
        reserved0: 0,
        animation_frames: FfiSlice {
            ptr: std::ptr::null(),
            count: 0,
        },
    }];
    let request = world_mesh_asset_update_request(&meshes, &textures);
    let (generation, owned_meshes, owned_textures, owned_sorted_indices, owned_retirements) =
        unsafe { decode_world_mesh_asset_update(&request, test_capabilities()).unwrap() };

    vertices[0].x = 99.0;
    index_bytes.fill(9);
    entity_identity.fill(b'x');
    png.fill(0);

    assert_eq!(9, generation);
    assert!(owned_sorted_indices.is_empty());
    assert!(owned_retirements.is_empty());
    assert_eq!(44, owned_meshes[0].mesh_key);
    assert_eq!(0.0, owned_meshes[0].vertices[0].position[0]);
    assert_eq!([0.25, 0.5], owned_meshes[0].vertices[0].shader_atlas_uv);
    assert_eq!(10232, owned_meshes[0].vertices[0].shader_block_id);
    assert_eq!(-1, owned_meshes[0].vertices[0].shader_material_type);
    assert_eq!(vec![0u8, 0, 1, 0, 2, 0], owned_meshes[0].index_bytes);
    assert_eq!(123, owned_meshes[0].sections[0].texture_id);
    assert_eq!("minecraft:arrow", owned_meshes[0].entity_identity);
    assert_eq!(vec![41u8, 42, 43, 44], owned_textures[0].png_bytes);
    assert_eq!(1, owned_textures[0].requested_mip_levels);
    assert_eq!(owned_textures[0].sampling,
        Some(super::super::texture_sampling::TextureSampling::from_texture_metadata(true, true)));
}

#[test]
fn world_mesh_asset_ffi_rejects_duplicate_bad_index_and_bad_item_size() {
    let vertices = vec![mesh_vertex()];
    let index_bytes = vec![0u8, 0, 1, 0, 2, 0];
    let sections = vec![mesh_section(123)];
    let mut meshes = vec![
        mesh_asset(&vertices, &index_bytes, &sections),
        mesh_asset(&vertices, &index_bytes, &sections),
    ];
    let duplicate = unsafe {
        decode_world_mesh_asset_update(
            &world_mesh_asset_update_request(&meshes, &[]),
            test_capabilities(),
        )
    }
    .unwrap_err();
    assert_eq!(StatusCode::InvalidArgument, duplicate.code);

    meshes.pop();
    meshes[0].index_type = 99;
    let bad_index = unsafe {
        decode_world_mesh_asset_update(
            &world_mesh_asset_update_request(&meshes, &[]),
            test_capabilities(),
        )
    }
    .unwrap_err();
    assert_eq!(StatusCode::UnknownEnum, bad_index.code);

    meshes[0].index_type = IndexType::U16 as u32;
    meshes[0].byte_size -= 4;
    let malformed = unsafe {
        decode_world_mesh_asset_update(
            &world_mesh_asset_update_request(&meshes, &[]),
            test_capabilities(),
        )
    }
    .unwrap_err();
    assert_eq!(StatusCode::InvalidArgument, malformed.code);

    let malformed_identity = b"Minecraft:Arrow";
    meshes[0].byte_size = size_of::<FfiWorldMeshAssetRecord>() as u32;
    meshes[0].entity_identity_utf8 = FfiBytes {
        ptr: malformed_identity.as_ptr(),
        len: malformed_identity.len() as u64,
    };
    let malformed_identity = unsafe {
        decode_world_mesh_asset_update(
            &world_mesh_asset_update_request(&meshes, &[]),
            test_capabilities(),
        )
    }
    .expect_err("mesh asset entity identity must remain canonical gameplay text");
    assert_eq!(StatusCode::InvalidArgument, malformed_identity.code);
}

#[test]
fn world_mesh_asset_ffi_rejects_oversized_vertex_lists_before_copying() {
    let vertices = vec![mesh_vertex(); WORLD_MAX_MESH_VERTICES + 1];
    let index_bytes = vec![0u8, 0, 1, 0, 2, 0];
    let sections = vec![mesh_section(123)];
    let meshes = vec![mesh_asset(&vertices, &index_bytes, &sections)];
    let error = unsafe {
        decode_world_mesh_asset_update(
            &world_mesh_asset_update_request(&meshes, &[]),
            test_capabilities(),
        )
    }
    .expect_err("oversized world mesh vertex lists must fail before copying");
    assert_eq!(StatusCode::LengthOverflow, error.code);
}

#[test]
fn world_lod_asset_ffi_copies_semantic_columns_and_rejects_malformed_records() {
    let mut vertices = vec![lod_vertex(); 4];
    let segments = [FfiWorldLodSegmentRecord {
        byte_size: size_of::<FfiWorldLodSegmentRecord>() as u32,
        layer: WORLD_LOD_LAYER_OPAQUE,
        vertices: FfiSlice {
            ptr: vertices.as_ptr(),
            count: vertices.len() as u64,
        },
        packed_vertices: FfiSlice::default(),
    }];
    let assets = [FfiWorldLodColumnAssetRecord {
        byte_size: size_of::<FfiWorldLodColumnAssetRecord>() as u32,
        vertex_layout_version: WORLD_LOD_VERTEX_LAYOUT_V1,
        origin_x: -128,
        origin_y: 64,
        origin_z: 256,
        reserved0: 0,
        column_key: 0x4448_4c4f_445f_3031,
        column_generation: 3,
        segments: FfiSlice {
            ptr: segments.as_ptr(),
            count: segments.len() as u64,
        },
    }];
    let request = world_lod_asset_update_request(&assets, &[]);
    let (generation, owned, retirements, material_provenance) =
        unsafe { decode_world_lod_asset_update(&request, test_capabilities()).unwrap() };
    vertices[0].local_x = 99;
    assert_eq!(17, generation);
    assert!(retirements.is_empty());
    assert!(material_provenance.is_empty());
    assert_eq!([-128, 64, 256], owned[0].origin);
    assert_eq!(
        [12, 34, 56],
        owned[0].segments[0].vertices[0].local_position
    );
    assert_eq!(
        [10, 20, 30, 40],
        owned[0].segments[0].vertices[0].color_rgba
    );

    let mut block_state = b"minecraft:grass_block[snowy=false]".to_vec();
    let biome = b"minecraft:plains".to_vec();
    let identities = [FfiWorldLodMaterialIdentityRecord {
        byte_size: size_of::<FfiWorldLodMaterialIdentityRecord>() as u32,
        reserved0: 0,
        block_state_identity_utf8: FfiBytes {
            ptr: block_state.as_ptr(),
            len: block_state.len() as u64,
        },
        biome_identity_utf8: FfiBytes {
            ptr: biome.as_ptr(),
            len: biome.len() as u64,
        },
    }];
    let quad_material_ids = [1_u32];
    let quad_variant_states = [WORLD_LOD_VARIANT_EXACT];
    let quad_variant_positions = [0_u64];
    let atlas_identity = b"minecraft:textures/atlas/blocks.png".to_vec();
    let sprite_identity = b"minecraft:block/grass_block_top".to_vec();
    let face_materials = [FfiWorldLodFaceMaterialRecord {
        byte_size: size_of::<FfiWorldLodFaceMaterialRecord>() as u32,
        material_id: 1,
        face: 1,
        face_layer: 0,
        atlas_identity_utf8: FfiBytes {
            ptr: atlas_identity.as_ptr(),
            len: atlas_identity.len() as u64,
        },
        sprite_identity_utf8: FfiBytes {
            ptr: sprite_identity.as_ptr(),
            len: sprite_identity.len() as u64,
        },
        u0: 0.25,
        v0: 0.5,
        u1: 0.3125,
        v1: 0.5625,
        uv_corner_order: 0x78,
        variant_position: 0,
    }];
    let provenance_segments = [FfiWorldLodSegmentMaterialProvenanceRecord {
        byte_size: size_of::<FfiWorldLodSegmentMaterialProvenanceRecord>() as u32,
        layer: WORLD_LOD_LAYER_OPAQUE,
        segment_index: 0,
        reserved0: 0,
        quad_material_ids: FfiSlice {
            ptr: quad_material_ids.as_ptr(),
            count: quad_material_ids.len() as u64,
        },
        quad_variant_states: FfiSlice {
            ptr: quad_variant_states.as_ptr(),
            count: quad_variant_states.len() as u64,
        },
        quad_variant_positions: FfiSlice {
            ptr: quad_variant_positions.as_ptr(),
            count: quad_variant_positions.len() as u64,
        },
    }];
    let provenance_columns = [FfiWorldLodColumnMaterialProvenanceRecord {
        byte_size: size_of::<FfiWorldLodColumnMaterialProvenanceRecord>() as u32,
        reserved0: 0,
        column_key: assets[0].column_key,
        column_generation: assets[0].column_generation,
        identities: FfiSlice {
            ptr: identities.as_ptr(),
            count: identities.len() as u64,
        },
        segments: FfiSlice {
            ptr: provenance_segments.as_ptr(),
            count: provenance_segments.len() as u64,
        },
        face_materials: FfiSlice {
            ptr: face_materials.as_ptr(),
            count: face_materials.len() as u64,
        },
    }];
    let mut provenance_request = world_lod_asset_update_request(&assets, &[]);
    provenance_request.material_provenance = FfiSlice {
        ptr: provenance_columns.as_ptr(),
        count: provenance_columns.len() as u64,
    };
    let (_, _, _, copied_provenance) =
        unsafe { decode_world_lod_asset_update(&provenance_request, test_capabilities()).unwrap() };
    block_state[10] = b'X';
    assert_eq!(
        "minecraft:grass_block[snowy=false]",
        copied_provenance[0].identities[0].block_state_identity
    );
    assert_eq!(vec![1], copied_provenance[0].segments[0].quad_material_ids);
    assert_eq!(
        "minecraft:block/grass_block_top",
        copied_provenance[0].face_materials[0].sprite_identity
    );
    assert_eq!(
        [0.25, 0.5, 0.3125, 0.5625],
        copied_provenance[0].face_materials[0].atlas_uv
    );
    assert_eq!(0x78, copied_provenance[0].face_materials[0].uv_corner_order);
    assert!(!copied_provenance[0].face_materials[0].tinted);

    let mut tinted_face_materials = face_materials;
    tinted_face_materials[0].face_layer = 0x4 | (0x4f << 3) | (0xa1 << 11) | (0x3c << 19);
    let mut tinted_columns = provenance_columns;
    tinted_columns[0].face_materials = FfiSlice {
        ptr: tinted_face_materials.as_ptr(),
        count: tinted_face_materials.len() as u64,
    };
    let mut tinted_request = world_lod_asset_update_request(&assets, &[]);
    tinted_request.material_provenance = FfiSlice {
        ptr: tinted_columns.as_ptr(),
        count: tinted_columns.len() as u64,
    };
    let (_, _, _, tinted_provenance) =
        unsafe { decode_world_lod_asset_update(&tinted_request, test_capabilities()).unwrap() };
    assert!(tinted_provenance[0].face_materials[0].tinted);
    assert_eq!(
        [
            0x4f as f32 / 255.0,
            0xa1 as f32 / 255.0,
            0x3c as f32 / 255.0
        ],
        tinted_provenance[0].face_materials[0].tint_rgb
    );

    let mut malformed_variant_segments = provenance_segments;
    malformed_variant_segments[0].quad_variant_positions.count = 0;
    let mut malformed_variant_columns = provenance_columns;
    malformed_variant_columns[0].segments = FfiSlice {
        ptr: malformed_variant_segments.as_ptr(),
        count: malformed_variant_segments.len() as u64,
    };
    let mut malformed_variant_request = world_lod_asset_update_request(&assets, &[]);
    malformed_variant_request.material_provenance = FfiSlice {
        ptr: malformed_variant_columns.as_ptr(),
        count: malformed_variant_columns.len() as u64,
    };
    let malformed_variant_error = unsafe {
        decode_world_lod_asset_update(&malformed_variant_request, test_capabilities()).unwrap_err()
    };
    assert_eq!(StatusCode::InvalidArgument, malformed_variant_error.code);
    assert!(malformed_variant_error
        .to_string()
        .contains("variant provenance must align"));

    let duplicate_face_materials = [face_materials[0], face_materials[0]];
    let mut duplicate_face_columns = provenance_columns;
    duplicate_face_columns[0].face_materials = FfiSlice {
        ptr: duplicate_face_materials.as_ptr(),
        count: duplicate_face_materials.len() as u64,
    };
    let mut duplicate_face_request = world_lod_asset_update_request(&assets, &[]);
    duplicate_face_request.material_provenance = FfiSlice {
        ptr: duplicate_face_columns.as_ptr(),
        count: duplicate_face_columns.len() as u64,
    };
    let duplicate_face_error = unsafe {
        decode_world_lod_asset_update(&duplicate_face_request, test_capabilities()).unwrap_err()
    };
    assert_eq!(StatusCode::InvalidArgument, duplicate_face_error.code);
    assert!(duplicate_face_error
        .to_string()
        .contains("unique normalized atlas region"));

    let mut invalid_uv_order_faces = face_materials;
    invalid_uv_order_faces[0].uv_corner_order = 0;
    let mut invalid_uv_order_columns = provenance_columns;
    invalid_uv_order_columns[0].face_materials = FfiSlice {
        ptr: invalid_uv_order_faces.as_ptr(),
        count: invalid_uv_order_faces.len() as u64,
    };
    let mut invalid_uv_order_request = world_lod_asset_update_request(&assets, &[]);
    invalid_uv_order_request.material_provenance = FfiSlice {
        ptr: invalid_uv_order_columns.as_ptr(),
        count: invalid_uv_order_columns.len() as u64,
    };
    let invalid_uv_order_error = unsafe {
        decode_world_lod_asset_update(&invalid_uv_order_request, test_capabilities()).unwrap_err()
    };
    assert_eq!(StatusCode::InvalidArgument, invalid_uv_order_error.code);

    let mut bad_provenance = provenance_columns;
    bad_provenance[0].segments.count = 0;
    let mut bad_provenance_request = world_lod_asset_update_request(&assets, &[]);
    bad_provenance_request.material_provenance = FfiSlice {
        ptr: bad_provenance.as_ptr(),
        count: bad_provenance.len() as u64,
    };
    let (_, _, _, decoded_bad_provenance) = unsafe {
        decode_world_lod_asset_update(&bad_provenance_request, test_capabilities()).unwrap()
    };
    assert!(decoded_bad_provenance[0].segments.is_empty());

    let mut origin_assets = assets;
    origin_assets[0].column_key = 0;
    let origin_request = world_lod_asset_update_request(&origin_assets, &[]);
    let (_, origin_owned, _, origin_material_provenance) =
        unsafe { decode_world_lod_asset_update(&origin_request, test_capabilities()).unwrap() };
    assert_eq!(0, origin_owned[0].column_key);
    assert!(origin_material_provenance.is_empty());

    let mut malformed_assets = assets;
    malformed_assets[0].segments.count = 0;
    let error = unsafe {
        decode_world_lod_asset_update(
            &world_lod_asset_update_request(&malformed_assets, &[]),
            test_capabilities(),
        )
    }
    .unwrap_err();
    assert_eq!(StatusCode::InvalidArgument, error.code);

    let malformed_vertices = [lod_vertex(); 3];
    let malformed_segments = [FfiWorldLodSegmentRecord {
        byte_size: size_of::<FfiWorldLodSegmentRecord>() as u32,
        layer: WORLD_LOD_LAYER_OPAQUE,
        vertices: FfiSlice {
            ptr: malformed_vertices.as_ptr(),
            count: malformed_vertices.len() as u64,
        },
        packed_vertices: FfiSlice::default(),
    }];
    let mut malformed_quad_assets = assets;
    malformed_quad_assets[0].segments = FfiSlice {
        ptr: malformed_segments.as_ptr(),
        count: malformed_segments.len() as u64,
    };
    let error = unsafe {
        decode_world_lod_asset_update(
            &world_lod_asset_update_request(&malformed_quad_assets, &[]),
            test_capabilities(),
        )
    }
    .unwrap_err();
    assert_eq!(StatusCode::InvalidArgument, error.code);
}

#[test]
fn world_lod_asset_ffi_accepts_the_bounded_packed_dh_vertex_stream() {
    // The Java DH producer writes this exact native-endian, 16-byte semantic
    // layout. Keep the test independent of the legacy structured ABI so a
    // generic FFI item-count limit cannot accidentally reject a valid byte
    // packet (up to 65,536 vertices / 1 MiB).
    let mut packed_vertices = Vec::with_capacity(4 * 16);
    for _ in 0..4 {
        packed_vertices.extend_from_slice(&12_u16.to_ne_bytes());
        packed_vertices.extend_from_slice(&34_u16.to_ne_bytes());
        packed_vertices.extend_from_slice(&56_u16.to_ne_bytes());
        packed_vertices.extend_from_slice(&0xa57b_u16.to_ne_bytes());
        packed_vertices.extend_from_slice(&[10, 20, 30, 40, 7, 5, 0, 0]);
    }
    let segments = [FfiWorldLodSegmentRecord {
        byte_size: size_of::<FfiWorldLodSegmentRecord>() as u32,
        layer: WORLD_LOD_LAYER_OPAQUE,
        vertices: FfiSlice {
            ptr: std::ptr::null(),
            count: 0,
        },
        packed_vertices: FfiSlice {
            ptr: packed_vertices.as_ptr(),
            count: packed_vertices.len() as u64,
        },
    }];
    let assets = [FfiWorldLodColumnAssetRecord {
        byte_size: size_of::<FfiWorldLodColumnAssetRecord>() as u32,
        vertex_layout_version: WORLD_LOD_VERTEX_LAYOUT_V1,
        origin_x: -128,
        origin_y: 64,
        origin_z: 256,
        reserved0: 0,
        column_key: 0x4448_4c4f_445f_3032,
        column_generation: 4,
        segments: FfiSlice {
            ptr: segments.as_ptr(),
            count: segments.len() as u64,
        },
    }];

    let (_, owned, _, _) = unsafe {
        decode_world_lod_asset_update(
            &world_lod_asset_update_request(&assets, &[]),
            test_capabilities(),
        )
        .unwrap()
    };
    assert_eq!(4, owned[0].segments[0].vertices.len());
    assert_eq!(
        [12, 34, 56],
        owned[0].segments[0].vertices[0].local_position
    );
    assert_eq!(
        0xa57b,
        owned[0].segments[0].vertices[0].packed_light_and_micro_offset
    );
    assert_eq!(
        [10, 20, 30, 40],
        owned[0].segments[0].vertices[0].color_rgba
    );
    assert_eq!(7, owned[0].segments[0].vertices[0].material_id);
    assert_eq!(5, owned[0].segments[0].vertices[0].normal_index);
}

#[test]
fn whole_frame_ffi_copies_bounded_lod_segment_references() {
    let instances = [FfiWorldLodColumnInstanceRecord {
        byte_size: size_of::<FfiWorldLodColumnInstanceRecord>() as u32,
        layer: WORLD_LOD_LAYER_OPAQUE,
        segment_index: 2,
        order: 17,
        column_key: 0,
        column_generation: 9,
    }];
    let mut request = whole_frame_request(&[], &[]);
    request.world_lod_instances = FfiSlice {
        ptr: instances.as_ptr(),
        count: instances.len() as u64,
    };
    request.world_lod_render_frame = FfiWorldLodRenderFrame {
        byte_size: size_of::<FfiWorldLodRenderFrame>() as u32,
        enabled: 1,
        flags: 0b0111 | WORLD_LOD_FLAG_RUST_OPAQUE_ROUTE_SELECTED,
        world_y_offset: -64,
        combined_matrix: [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 4.0, 8.0, 12.0, 1.0,
        ],
        model_view_matrix: [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 4.0, 8.0, 12.0, 1.0,
        ],
        projection_matrix: [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ],
        projection_inverse_matrix: [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ],
        clip_distance: 24.0,
        micro_offset: 0.01,
        noise_intensity: 0.25,
        earth_radius: 6_371_000.0,
        noise_steps: 4,
        noise_dropoff: 96,
        reserved0: 0,
        camera_world_x: 12.5,
        camera_world_y: 64.0,
        camera_world_z: -3.25,
    };
    let (_, _, frame, _) =
        unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()).unwrap() };
    assert_eq!(1, frame.lod_instances.len());
    assert_eq!(0, frame.lod_instances[0].column_key);
    assert_eq!(2, frame.lod_instances[0].segment_index);
    assert_eq!(17, frame.lod_instances[0].order);
    assert!(frame.lod_render_frame.enabled);
    assert_eq!(
        [12.5, 64.0, -3.25],
        frame.lod_render_frame.camera_world_position,
        "LOD camera semantics must not be borrowed from the optional voxel-volume frame"
    );
    assert_eq!(
        0b0111 | WORLD_LOD_FLAG_RUST_OPAQUE_ROUTE_SELECTED,
        frame.lod_render_frame.flags
    );
    assert!(frame.lod_render_frame.rust_opaque_route_selected());
    assert_eq!(-64, frame.lod_render_frame.world_y_offset);
    assert_eq!(24.0, frame.lod_render_frame.clip_distance);
    assert_eq!(
        request.world_lod_render_frame.model_view_matrix,
        frame.lod_render_frame.model_view_matrix
    );
    assert_eq!(
        request.world_lod_render_frame.projection_matrix,
        frame.lod_render_frame.projection_matrix
    );
    assert_eq!(
        request.world_lod_render_frame.projection_inverse_matrix,
        frame.lod_render_frame.projection_inverse_matrix
    );

    let mut malformed = instances;
    malformed[0].byte_size -= 4;
    request.world_lod_instances = FfiSlice {
        ptr: malformed.as_ptr(),
        count: malformed.len() as u64,
    };
    let error =
        unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }.unwrap_err();
    assert_eq!(StatusCode::InvalidArgument, error.code);

    let mut unknown_route_flag = whole_frame_request(&[], &[]);
    unknown_route_flag.world_lod_render_frame.enabled = 1;
    unknown_route_flag.world_lod_render_frame.micro_offset = 0.01;
    unknown_route_flag.world_lod_render_frame.flags =
        WORLD_LOD_FLAG_RUST_OPAQUE_ROUTE_SELECTED | 0x20;
    let error =
        unsafe { decode_whole_frame_submit(&unknown_route_flag, test_vulkan_capabilities()) }
            .unwrap_err();
    assert_eq!(StatusCode::InvalidArgument, error.code);

    let mut invalid_render_frame = whole_frame_request(&[], &[]);
    invalid_render_frame.world_lod_render_frame.enabled = 0;
    invalid_render_frame.world_lod_render_frame.micro_offset = 0.01;
    let error =
        unsafe { decode_whole_frame_submit(&invalid_render_frame, test_vulkan_capabilities()) }
            .unwrap_err();
    assert_eq!(StatusCode::InvalidArgument, error.code);

    let mut non_finite_render_frame = whole_frame_request(&[], &[]);
    non_finite_render_frame.world_lod_render_frame.enabled = 1;
    non_finite_render_frame.world_lod_render_frame.micro_offset = 0.01;
    non_finite_render_frame
        .world_lod_render_frame
        .projection_inverse_matrix[0] = f32::NAN;
    let error =
        unsafe { decode_whole_frame_submit(&non_finite_render_frame, test_vulkan_capabilities()) }
            .unwrap_err();
    assert_eq!(StatusCode::InvalidArgument, error.code);
}

#[test]
fn whole_frame_ffi_keeps_gui_blur_semantic_boundary_explicit_and_bounded() {
    let mut request = whole_frame_request(&[], &[]);
    request.gui_blur_before_stratum = -2;
    let error =
        unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }.unwrap_err();
    assert_eq!(StatusCode::InvalidArgument, error.code);

    request.gui_blur_before_stratum = 3;
    request.gui_blur_radius = 65;
    let error =
        unsafe { decode_whole_frame_submit(&request, test_vulkan_capabilities()) }.unwrap_err();
    assert_eq!(StatusCode::InvalidArgument, error.code);
    request.gui_blur_radius = -1;
    let decoded = unsafe {
        decode_whole_frame_submit_with_gui(&request, test_vulkan_capabilities()).unwrap()
    };
    assert_eq!(3, decoded.6);
    assert_eq!(-1, decoded.7);
}

#[test]
fn whole_frame_post_effect_identity_is_bounded_utf8_semantic_data() {
    let mut request = whole_frame_request(&[], &[]);
    let id = b"minecraft:custom";
    request.post_effect_id = FfiBytes {
        ptr: id.as_ptr(),
        len: id.len() as u64,
    };
    unsafe {
        super::world::decode_whole_frame_submit_with_gui(&request, test_vulkan_capabilities())
            .expect("valid copied post-effect identity must decode");
    }
    let invalid = [0xffu8];
    request.post_effect_id = FfiBytes {
        ptr: invalid.as_ptr(),
        len: invalid.len() as u64,
    };
    let error = unsafe {
        super::world::decode_whole_frame_submit_with_gui(&request, test_vulkan_capabilities())
    }
    .unwrap_err();
    assert!(error.to_string().contains("UTF-8"));
}

#[test]
fn shader_pack_source_ffi_copies_owned_utf8_files() {
    let mut pack_name = b"complementary-test".to_vec();
    let mut path = b"program/gbuffers_terrain.glsl".to_vec();
    let mut contents = b"#version 150\nvoid main() {}\n".to_vec();
    let files = [FfiShaderPackSourceFile {
        byte_size: size_of::<FfiShaderPackSourceFile>() as u32,
        reserved0: 0,
        path_utf8: FfiBytes {
            ptr: path.as_ptr(),
            len: path.len() as u64,
        },
        contents_utf8: FfiBytes {
            ptr: contents.as_ptr(),
            len: contents.len() as u64,
        },
    }];
    let request = shader_pack_source_update_request(&pack_name, &files);
    let decoded = unsafe { super::shader_pack::decode_shader_pack_source_update(&request) }
        .expect("valid source update");

    pack_name.fill(b'x');
    path.fill(b'x');
    contents.fill(b'x');

    assert_eq!("complementary-test", decoded.pack_name);
    assert_eq!(9, decoded.generation);
    assert_eq!("program/gbuffers_terrain.glsl", decoded.files[0].path);
    assert_eq!("#version 150\nvoid main() {}\n", decoded.files[0].contents);
}

#[test]
fn shader_pack_source_ffi_rejects_malformed_file_records() {
    let pack_name = b"test";
    let path = b"program/gbuffers_terrain.glsl";
    let contents = b"void main() {}";
    let mut files = [FfiShaderPackSourceFile {
        byte_size: size_of::<FfiShaderPackSourceFile>() as u32,
        reserved0: 1,
        path_utf8: FfiBytes {
            ptr: path.as_ptr(),
            len: path.len() as u64,
        },
        contents_utf8: FfiBytes {
            ptr: contents.as_ptr(),
            len: contents.len() as u64,
        },
    }];
    let request = shader_pack_source_update_request(pack_name, &files);
    let reserved = unsafe { super::shader_pack::decode_shader_pack_source_update(&request) }
        .expect_err("reserved field must be rejected");
    assert_eq!(StatusCode::InvalidArgument, reserved.code);

    files[0].reserved0 = 0;
    files[0].byte_size -= 4;
    let malformed_request = shader_pack_source_update_request(pack_name, &files);
    let malformed =
        unsafe { super::shader_pack::decode_shader_pack_source_update(&malformed_request) }
            .expect_err("truncated item must be rejected");
    assert_eq!(StatusCode::InvalidArgument, malformed.code);
}

#[test]
fn shader_pack_source_ffi_accepts_an_explicit_empty_generation() {
    let pack_name = b"disabled";
    let request = shader_pack_source_update_request(pack_name, &[]);
    let decoded = unsafe { super::shader_pack::decode_shader_pack_source_update(&request) }
        .expect("empty source generation clears a prior pack without stale reuse");
    assert_eq!("disabled", decoded.pack_name);
    assert!(decoded.files.is_empty());
}

#[test]
fn shader_pack_asset_ffi_copies_owned_binary_files() {
    let mut pack_name = b"complementary-test".to_vec();
    let mut path = b"textures/noise.png".to_vec();
    let mut contents = vec![0x89, b'P', b'N', b'G', 0, 1, 2, 3];
    let files = [FfiShaderPackAssetFile {
        byte_size: size_of::<FfiShaderPackAssetFile>() as u32,
        reserved0: 0,
        path_utf8: FfiBytes {
            ptr: path.as_ptr(),
            len: path.len() as u64,
        },
        contents: FfiBytes {
            ptr: contents.as_ptr(),
            len: contents.len() as u64,
        },
    }];
    let request = shader_pack_asset_update_request(&pack_name, &files);
    let decoded = unsafe { super::shader_pack::decode_shader_pack_asset_update(&request) }
        .expect("valid asset update");

    pack_name.fill(b'x');
    path.fill(b'x');
    contents.fill(0xff);

    assert_eq!("complementary-test", decoded.pack_name);
    assert_eq!(9, decoded.generation);
    assert_eq!("textures/noise.png", decoded.files[0].path);
    assert_eq!(
        vec![0x89, b'P', b'N', b'G', 0, 1, 2, 3],
        decoded.files[0].bytes
    );
}

#[test]
fn shader_pack_asset_ffi_rejects_malformed_file_records() {
    let pack_name = b"test";
    let path = b"textures/noise.png";
    let contents = [1u8, 2, 3];
    let mut files = [FfiShaderPackAssetFile {
        byte_size: size_of::<FfiShaderPackAssetFile>() as u32,
        reserved0: 1,
        path_utf8: FfiBytes {
            ptr: path.as_ptr(),
            len: path.len() as u64,
        },
        contents: FfiBytes {
            ptr: contents.as_ptr(),
            len: contents.len() as u64,
        },
    }];
    let request = shader_pack_asset_update_request(pack_name, &files);
    let reserved = unsafe { super::shader_pack::decode_shader_pack_asset_update(&request) }
        .expect_err("reserved field must be rejected");
    assert_eq!(StatusCode::InvalidArgument, reserved.code);

    files[0].reserved0 = 0;
    files[0].byte_size -= 4;
    let malformed_request = shader_pack_asset_update_request(pack_name, &files);
    let malformed =
        unsafe { super::shader_pack::decode_shader_pack_asset_update(&malformed_request) }
            .expect_err("truncated item must be rejected");
    assert_eq!(StatusCode::InvalidArgument, malformed.code);
}
