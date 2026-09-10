use super::*;
use crate::render::vulkanic::metrics::WholeFrameProfile;

pub fn status_result_from_error(error: &GalError) -> FfiStatusResult {
    FfiStatusResult {
        status: error.code as i32,
        error_domain: error.domain as u32,
        unsupported_feature: unsupported_feature_from_message(error.message.as_str()),
        ..FfiStatusResult::default()
    }
}

pub fn capability_feature_bits(capabilities: BackendCapabilities) -> u64 {
    let features = capabilities.features;
    let mut bits = 0;
    set_feature(&mut bits, FfiFeatureBits::GRAPHICS, features.graphics);
    set_feature(&mut bits, FfiFeatureBits::COMPUTE, features.compute);
    set_feature(
        &mut bits,
        FfiFeatureBits::DESCRIPTOR_ARRAYS,
        features.descriptor_arrays,
    );
    set_feature(
        &mut bits,
        FfiFeatureBits::OPTIONAL_BINDINGS,
        features.optional_bindings,
    );
    set_feature(
        &mut bits,
        FfiFeatureBits::DYNAMIC_BUFFER_OFFSETS,
        features.dynamic_buffer_offsets,
    );
    set_feature(
        &mut bits,
        FfiFeatureBits::UNIFORM_BUFFERS,
        features.uniform_buffers,
    );
    set_feature(
        &mut bits,
        FfiFeatureBits::STORAGE_BUFFERS,
        features.storage_buffers,
    );
    set_feature(
        &mut bits,
        FfiFeatureBits::STORAGE_TEXTURES,
        features.storage_textures,
    );
    set_feature(
        &mut bits,
        FfiFeatureBits::INDIRECT_DRAW,
        features.indirect_draw,
    );
    set_feature(
        &mut bits,
        FfiFeatureBits::INDIRECT_DISPATCH,
        features.indirect_dispatch,
    );
    set_feature(
        &mut bits,
        FfiFeatureBits::MULTIPLE_COLOR_ATTACHMENTS,
        features.multiple_color_attachments,
    );
    set_feature(
        &mut bits,
        FfiFeatureBits::DEPTH_ONLY_PASS,
        features.depth_only_pass,
    );
    set_feature(
        &mut bits,
        FfiFeatureBits::BLENDED_PASS,
        features.blended_pass,
    );
    set_feature(
        &mut bits,
        FfiFeatureBits::TEXTURE_SUBRESOURCE_COPIES,
        features.texture_subresource_copies,
    );
    set_feature(
        &mut bits,
        FfiFeatureBits::TEXTURE_MIP_LEVELS,
        features.texture_mip_levels,
    );
    set_feature(
        &mut bits,
        FfiFeatureBits::TEXTURE_ARRAY_LAYERS,
        features.texture_array_layers,
    );
    set_feature(
        &mut bits,
        FfiFeatureBits::HOST_BUFFER_ACCESS,
        features.host_buffer_access,
    );
    set_feature(
        &mut bits,
        FfiFeatureBits::PRESENTATION,
        features.presentation,
    );
    set_feature(
        &mut bits,
        FfiFeatureBits::RENDERDOC_CAPTURE,
        features.renderdoc_capture,
    );
    set_feature(&mut bits, FfiFeatureBits::TRACY_ZONES, features.tracy_zones);
    bits
}

pub unsafe fn answer_capability_query(
    request: *const FfiCapabilityQueryRequest,
    capabilities: BackendCapabilities,
) -> GalResult<FfiCapabilityResult> {
    let request = read_struct(request, "capability query")?;
    validate_header::<FfiCapabilityQueryRequest>(request.header)?;
    reject_unknown_feature_bits(request.requested_feature_bits)?;
    let supported = capability_feature_bits(capabilities);
    if request.requested_feature_bits & !supported != 0 {
        return Err(GalError::unsupported_feature(format!(
            "requested unsupported feature bits 0x{:x}",
            request.requested_feature_bits & !supported
        )));
    }
    Ok(FfiCapabilityResult {
        header: FfiHeader {
            version: FFI_ABI_VERSION,
            byte_size: size_of::<FfiCapabilityResult>() as u32,
        },
        status: StatusCode::Ok as i32,
        error_domain: 0,
        supported_feature_bits: supported,
        negotiated_feature_bits: request.requested_feature_bits,
        limits: FfiBackendLimits::from(capabilities.limits),
        initial_presentation_supported: u32::from(capabilities.features.presentation),
    })
}
pub(crate) fn context_metrics(context: &BridgeContext) -> FfiMetricsSnapshot {
    let mut metrics = FfiMetricsSnapshot::from(context.gal.metrics());
    let backend = context.gal.backend_runtime_metrics();
    metrics.command_lists = backend.command_lists;
    metrics.command_ops = backend.command_ops;
    metrics.backend_submissions = backend.command_batches;
    metrics.backend_waits = backend.gl_fences_waited;
    metrics.ffi_calls = context.ffi_calls;
    metrics.ffi_input_bytes = context.ffi_input_bytes;
    metrics.ffi_output_bytes = context.ffi_output_bytes;
    metrics
}

pub(crate) fn status_ok(context: &BridgeContext) -> FfiStatusResult {
    FfiStatusResult {
        metrics: context_metrics(context),
        ..FfiStatusResult::default()
    }
}

pub(crate) fn gui_frame_result_ok(
    context: &BridgeContext,
    stats: GuiSubmitStats,
) -> FfiGuiFrameSubmitResult {
    FfiGuiFrameSubmitResult {
        submission_id: stats.submission_id,
        sprite_count: stats.sprite_count,
        sprite_batch_count: stats.sprite_batch_count,
        cache_hits: stats.cache_hits,
        cache_misses: stats.cache_misses,
        resource_creates: stats.resource_creates,
        command_lists: stats.command_lists,
        command_ops: stats.command_ops,
        metrics: context_metrics(context),
        ..FfiGuiFrameSubmitResult::default()
    }
}

pub(crate) fn whole_frame_result_ok(
    context: &BridgeContext,
    world: WorldPrimitiveSubmitStats,
    gui: GuiSubmitStats,
) -> FfiWholeFrameSubmitResult {
    FfiWholeFrameSubmitResult {
        submission_id: world.submission_id,
        world_segment_count: world.segment_count,
        world_vertex_count: world.vertex_count,
        world_batch_count: world.primitive_batch_count,
        world_draw_count: world.world_draws,
        world_crack_quad_count: world.crack_quad_count,
        world_crack_batch_count: world.crack_batch_count,
        world_crack_draw_count: world.crack_draw_count,
        world_border_quad_count: world.border_quad_count,
        world_border_batch_count: world.border_batch_count,
        world_border_draw_count: world.border_draw_count,
        world_material_quad_count: world.material_quad_count,
        world_material_batch_count: world.material_batch_count,
        world_material_draw_count: world.material_draw_count,
        world_mesh_instance_count: world.mesh_instance_count,
        world_mesh_batch_count: world.mesh_batch_count,
        world_mesh_draw_count: world.mesh_draw_count,
        world_background_clear_count: world.background_clear_count,
        world_background_diagnostic_fallback_count: world.background_diagnostic_fallback_count,
        world_background_sky_type: world.background_sky_type,
        world_background_color_argb: world.background_color_argb,
        depth_attachment_creates: world.depth_attachment_creates,
        depth_attachment_reuses: world.depth_attachment_reuses,
        depth_attachment_retires: world.depth_attachment_retires,
        outline_cache_hits: world.outline_cache_hits,
        outline_cache_misses: world.outline_cache_misses,
        crack_cache_hits: world.crack_cache_hits,
        crack_cache_misses: world.crack_cache_misses,
        border_cache_hits: world.border_cache_hits,
        border_cache_misses: world.border_cache_misses,
        material_cache_hits: world.material_cache_hits,
        material_cache_misses: world.material_cache_misses,
        mesh_cache_hits: world.mesh_cache_hits,
        mesh_cache_misses: world.mesh_cache_misses,
        sprite_count: gui.sprite_count,
        sprite_batch_count: gui.sprite_batch_count,
        gui_mesh_item_count: gui.mesh_item_count,
        gui_mesh_batch_count: gui.mesh_batch_count,
        gui_mesh_draw_count: gui.mesh_draw_count,
        cache_hits: world.cache_hits.saturating_add(gui.cache_hits),
        cache_misses: world.cache_misses.saturating_add(gui.cache_misses),
        resource_creates: world.resource_creates.saturating_add(gui.resource_creates),
        command_lists: world.command_lists.max(gui.command_lists),
        command_ops: world.command_ops,
        metrics: context_metrics(context),
        profile: FfiWholeFrameProfileSnapshot::from(world.profile),
        ..FfiWholeFrameSubmitResult::default()
    }
}

impl From<WholeFrameProfile> for FfiWholeFrameProfileSnapshot {
    fn from(profile: WholeFrameProfile) -> Self {
        Self {
            ffi_decode_nanos: profile.ffi_decode_nanos,
            gui_frontend_nanos: profile.gui_frontend_nanos,
            world_frontend_total_nanos: profile.world_frontend_total_nanos,
            world_validate_frame_nanos: profile.world_validate_frame_nanos,
            world_batching_nanos: profile.world_batching_nanos,
            world_resource_prepare_nanos: profile.world_resource_prepare_nanos,
            world_prepare_target_query_nanos: profile.world_prepare_target_query_nanos,
            world_prepare_render_resources_nanos: profile.world_prepare_render_resources_nanos,
            world_prepare_depth_attachment_nanos: profile.world_prepare_depth_attachment_nanos,
            world_prepare_g_buffer_resources_nanos: profile.world_prepare_g_buffer_resources_nanos,
            world_prepare_g_buffer_cache_check_nanos: profile
                .world_prepare_g_buffer_cache_check_nanos,
            world_prepare_g_buffer_destroy_nanos: profile.world_prepare_g_buffer_destroy_nanos,
            world_prepare_g_buffer_plan_nanos: profile.world_prepare_g_buffer_plan_nanos,
            world_prepare_g_buffer_create_nanos: profile.world_prepare_g_buffer_create_nanos,
            world_prepare_frame_pass_nanos: profile.world_prepare_frame_pass_nanos,
            world_mesh_section_expand_group_nanos: profile.world_mesh_section_expand_group_nanos,
            shader_plan_lookup_nanos: profile.shader_plan_lookup_nanos,
            gal_command_generation_nanos: profile.gal_command_generation_nanos,
            gal_submit_total_nanos: profile.gal_submit_total_nanos,
            gal_validate_ops_nanos: profile.gal_validate_ops_nanos,
            gal_validate_handles_nanos: profile.gal_validate_handles_nanos,
            gal_hazard_analysis_nanos: profile.gal_hazard_analysis_nanos,
            backend_encode_nanos: profile.backend_encode_nanos,
            backend_submit_nanos: profile.backend_submit_nanos,
            backend_retire_nanos: profile.backend_retire_nanos,
            vulkan_command_buffer_alloc_nanos: profile.vulkan_command_buffer_alloc_nanos,
            vulkan_command_buffer_begin_nanos: profile.vulkan_command_buffer_begin_nanos,
            vulkan_command_recording_nanos: profile.vulkan_command_recording_nanos,
            vulkan_command_buffer_end_nanos: profile.vulkan_command_buffer_end_nanos,
            vulkan_queue_submit_nanos: profile.vulkan_queue_submit_nanos,
            vulkan_timeline_poll_nanos: profile.vulkan_timeline_poll_nanos,
            vulkan_timeline_wait_nanos: profile.vulkan_timeline_wait_nanos,
            vulkan_device_wait_idle_nanos: profile.vulkan_device_wait_idle_nanos,
            vulkan_command_buffers_allocated: profile.vulkan_command_buffers_allocated,
            vulkan_command_buffers_freed: profile.vulkan_command_buffers_freed,
            vulkan_wait_count: profile.vulkan_wait_count,
            vulkan_device_wait_idle_count: profile.vulkan_device_wait_idle_count,
            resource_creates_delta: profile.resource_creates_delta,
            resource_destroys_delta: profile.resource_destroys_delta,
            host_write_ops: profile.host_write_ops,
            host_write_bytes: profile.host_write_bytes,
            barrier_ops: profile.barrier_ops,
            pass_count: profile.pass_count,
            draw_ops: profile.draw_ops,
            draw_indexed_ops: profile.draw_indexed_ops,
            pipeline_binds: profile.pipeline_binds,
            resource_set_binds: profile.resource_set_binds,
            gpu_timestamp_status: profile.gpu_timestamp_status,
            gpu_shadow_depth_nanos: profile.gpu_shadow_depth_nanos,
            gpu_terrain_opaque_nanos: profile.gpu_terrain_opaque_nanos,
            gpu_terrain_cutout_nanos: profile.gpu_terrain_cutout_nanos,
            gpu_deferred_lighting_nanos: profile.gpu_deferred_lighting_nanos,
            gpu_composite0_nanos: profile.gpu_composite0_nanos,
            gpu_composite1_nanos: profile.gpu_composite1_nanos,
            gpu_final_output_nanos: profile.gpu_final_output_nanos,
            gpu_frame_total_nanos: profile.gpu_frame_total_nanos,
            g_buffer_persistent_cache_hits: profile.g_buffer_persistent_cache_hits,
            g_buffer_persistent_cache_misses: profile.g_buffer_persistent_cache_misses,
            g_buffer_final_binding_cache_hits: profile.g_buffer_final_binding_cache_hits,
            g_buffer_final_binding_cache_misses: profile.g_buffer_final_binding_cache_misses,
            g_buffer_attachment_creates: profile.g_buffer_attachment_creates,
            g_buffer_pipeline_creates: profile.g_buffer_pipeline_creates,
            g_buffer_shader_module_creates: profile.g_buffer_shader_module_creates,
            g_buffer_descriptor_creates: profile.g_buffer_descriptor_creates,
            g_buffer_render_target_creates: profile.g_buffer_render_target_creates,
            g_buffer_resources_retired: profile.g_buffer_resources_retired,
            world_prepare_g_buffer_persistent_key_nanos: profile
                .world_prepare_g_buffer_persistent_key_nanos,
            world_prepare_g_buffer_persistent_lookup_nanos: profile
                .world_prepare_g_buffer_persistent_lookup_nanos,
            world_prepare_g_buffer_final_key_nanos: profile.world_prepare_g_buffer_final_key_nanos,
            world_prepare_g_buffer_final_lookup_nanos: profile
                .world_prepare_g_buffer_final_lookup_nanos,
            world_prepare_g_buffer_final_create_nanos: profile
                .world_prepare_g_buffer_final_create_nanos,
            world_prepare_frame_target_attachment_query_nanos: profile
                .world_prepare_frame_target_attachment_query_nanos,
            world_prepare_mesh_material_asset_nanos: profile
                .world_prepare_mesh_material_asset_nanos,
            world_prepare_metrics_accounting_nanos: profile.world_prepare_metrics_accounting_nanos,
            g_buffer_final_pass_creates: profile.g_buffer_final_pass_creates,
            vulkan_acquire_nanos: profile.vulkan_acquire_nanos,
            vulkan_present_nanos: profile.vulkan_present_nanos,
            vulkan_present_wait_nanos: profile.vulkan_present_wait_nanos,
            vulkan_present_mode: profile.vulkan_present_mode,
            vulkan_requested_present_mode: profile.vulkan_requested_present_mode,
            vulkan_supported_present_modes: profile.vulkan_supported_present_modes,
            vulkan_present_mode_fallback_reason: profile.vulkan_present_mode_fallback_reason,
            vulkan_acquired_image_index: profile.vulkan_acquired_image_index,
            vulkan_swapchain_generation: profile.vulkan_swapchain_generation,
            vulkan_swapchain_image_count: profile.vulkan_swapchain_image_count,
            vulkan_surface_min_image_count: profile.vulkan_surface_min_image_count,
            vulkan_surface_max_image_count: profile.vulkan_surface_max_image_count,
            vulkan_configured_frames_in_flight: profile.vulkan_configured_frames_in_flight,
            vulkan_images_in_flight: profile.vulkan_images_in_flight,
            vulkan_available_frame_slots: profile.vulkan_available_frame_slots,
            gal_hazard_read_events: profile.gal_hazard_read_events,
            gal_hazard_write_events: profile.gal_hazard_write_events,
            gal_hazard_candidates_examined: profile.gal_hazard_candidates_examined,
            gal_hazard_conflicts: profile.gal_hazard_conflicts,
            gal_hazard_barriers_applied: profile.gal_hazard_barriers_applied,
            gal_hazard_active_read_entries: profile.gal_hazard_active_read_entries,
            gal_hazard_active_write_entries: profile.gal_hazard_active_write_entries,
            gal_command_ops_before_normalize: profile.gal_command_ops_before_normalize,
            gal_command_ops_after_normalize: profile.gal_command_ops_after_normalize,
            gal_redundant_pipeline_binds_removed: profile.gal_redundant_pipeline_binds_removed,
            gal_redundant_resource_set_binds_removed: profile
                .gal_redundant_resource_set_binds_removed,
            gal_redundant_vertex_buffer_binds_removed: profile
                .gal_redundant_vertex_buffer_binds_removed,
            gal_redundant_index_buffer_binds_removed: profile
                .gal_redundant_index_buffer_binds_removed,
            world_prepare_mesh_cache_scan_nanos: profile.world_prepare_mesh_cache_scan_nanos,
            world_prepare_material_resource_nanos: profile.world_prepare_material_resource_nanos,
            world_prepare_mesh_stream_capacity_nanos: profile
                .world_prepare_mesh_stream_capacity_nanos,
            world_prepare_mesh_stream_lookup_nanos: profile.world_prepare_mesh_stream_lookup_nanos,
            world_prepare_mesh_stream_grow_nanos: profile.world_prepare_mesh_stream_grow_nanos,
            world_prepare_mesh_resource_nanos: profile.world_prepare_mesh_resource_nanos,
            world_prepare_material_slot_check_nanos: profile
                .world_prepare_material_slot_check_nanos,
            world_prepare_mesh_slot_check_nanos: profile.world_prepare_mesh_slot_check_nanos,
            world_prepare_mesh_batch_count: profile.world_prepare_mesh_batch_count,
            world_prepare_mesh_stream_required_bytes: profile
                .world_prepare_mesh_stream_required_bytes,
            world_prepare_mesh_stream_capacity_bytes: profile
                .world_prepare_mesh_stream_capacity_bytes,
            world_prepare_mesh_stream_grows: profile.world_prepare_mesh_stream_grows,
            world_mesh_stream_payload_pack_nanos: profile.world_mesh_stream_payload_pack_nanos,
            world_mesh_draw_record_nanos: profile.world_mesh_draw_record_nanos,
            world_mesh_stream_payload_bytes: profile.world_mesh_stream_payload_bytes,
            world_mesh_dynamic_offset_count: profile.world_mesh_dynamic_offset_count,
        }
    }
}

pub(crate) fn status_error(context: Option<&BridgeContext>, error: &GalError) -> FfiStatusResult {
    let mut status = status_result_from_error(error);
    if let Some(context) = context {
        status.metrics = context_metrics(context);
    }
    status
}

pub(crate) fn set_last_error(context: &mut BridgeContext, error: &GalError) {
    context.last_error = error.to_string();
}

pub(crate) fn output_bytes_for_resource_results(capacity: u64) -> u64 {
    capacity.saturating_mul(size_of::<FfiCreateResultEntry>() as u64)
}

pub(crate) fn input_bytes_for_resource_batch(batch: &FfiResourceBatch) -> u64 {
    (size_of::<FfiResourceBatch>() as u64)
        .saturating_add(
            batch
                .buffers
                .count
                .saturating_mul(size_of::<FfiBufferDescAbi>() as u64),
        )
        .saturating_add(
            batch
                .textures
                .count
                .saturating_mul(size_of::<FfiTextureDescAbi>() as u64),
        )
        .saturating_add(
            batch
                .texture_views
                .count
                .saturating_mul(size_of::<FfiTextureViewDescAbi>() as u64),
        )
        .saturating_add(
            batch
                .samplers
                .count
                .saturating_mul(size_of::<FfiSamplerDescAbi>() as u64),
        )
        .saturating_add(
            batch
                .shaders
                .count
                .saturating_mul(size_of::<FfiShaderModuleDescAbi>() as u64),
        )
        .saturating_add(
            batch
                .resource_layouts
                .count
                .saturating_mul(size_of::<FfiResourceLayoutDescAbi>() as u64),
        )
        .saturating_add(
            batch
                .resource_layout_bindings
                .count
                .saturating_mul(size_of::<FfiResourceBindingDescAbi>() as u64),
        )
        .saturating_add(
            batch
                .resource_sets
                .count
                .saturating_mul(size_of::<FfiResourceSetDescAbi>() as u64),
        )
        .saturating_add(
            batch
                .resource_set_bindings
                .count
                .saturating_mul(size_of::<FfiResourceBindingAbi>() as u64),
        )
        .saturating_add(
            batch
                .pipeline_layouts
                .count
                .saturating_mul(size_of::<FfiPipelineLayoutDescAbi>() as u64),
        )
        .saturating_add(
            batch
                .graphics_pipelines
                .count
                .saturating_mul(size_of::<FfiGraphicsPipelineDescAbi>() as u64),
        )
        .saturating_add(
            batch
                .compute_pipelines
                .count
                .saturating_mul(size_of::<FfiComputePipelineDescAbi>() as u64),
        )
        .saturating_add(
            batch
                .render_targets
                .count
                .saturating_mul(size_of::<FfiRenderTargetDescAbi>() as u64),
        )
        .saturating_add(
            batch
                .render_passes
                .count
                .saturating_mul(size_of::<FfiRenderPassDescAbi>() as u64),
        )
        .saturating_add(
            batch
                .buffer_updates
                .count
                .saturating_mul(size_of::<FfiBufferUpdateAbi>() as u64),
        )
        .saturating_add(
            batch
                .texture_updates
                .count
                .saturating_mul(size_of::<FfiTextureUpdateAbi>() as u64),
        )
        .saturating_add(
            batch
                .destroys
                .count
                .saturating_mul(size_of::<FfiDestroyDescAbi>() as u64),
        )
}

pub(crate) fn input_bytes_for_submission(batch: &FfiSubmissionBatchAbi) -> u64 {
    (size_of::<FfiSubmissionBatchAbi>() as u64)
        .saturating_add(
            batch
                .command_lists
                .count
                .saturating_mul(size_of::<FfiCommandListAbi>() as u64),
        )
        .saturating_add(
            batch
                .operations
                .count
                .saturating_mul(size_of::<FfiCommandOpAbi>() as u64),
        )
        .saturating_add(
            batch
                .pass_attachments
                .count
                .saturating_mul(size_of::<FfiPassAttachmentAbi>() as u64),
        )
        .saturating_add(
            batch
                .copy_regions
                .count
                .saturating_mul(size_of::<FfiBufferImageCopyAbi>() as u64),
        )
        .saturating_add(
            batch
                .barriers
                .count
                .saturating_mul(size_of::<FfiResourceBarrierAbi>() as u64),
        )
}

pub(crate) fn input_bytes_for_gui_frame(request: &FfiGuiFrameSubmitRequest) -> u64 {
    (size_of::<FfiGuiFrameSubmitRequest>() as u64)
        .saturating_add(request.tiled_quads.count.saturating_mul(size_of::<FfiGuiTiledQuadRequest>() as u64))
        .saturating_add(
            request
                .sprites
                .count
                .saturating_mul(size_of::<FfiGuiSpriteRequest>() as u64),
        )
        .saturating_add(
            request
                .affine_quads
                .count
                .saturating_mul(size_of::<FfiGuiAffineQuadRequest>() as u64),
        )
}

pub(crate) fn input_bytes_for_whole_frame(request: &FfiWholeFrameSubmitRequest) -> u64 {
    (size_of::<FfiWholeFrameSubmitRequest>() as u64)
        .saturating_add(request.world_particle_quads.count.saturating_mul(size_of::<FfiWorldParticleQuadRequest>() as u64))
        .saturating_add(request.world_experience_orbs.count.saturating_mul(size_of::<FfiWorldExperienceOrbInstanceRecord>() as u64))
        .saturating_add(request.gui_tiled_quads.count.saturating_mul(size_of::<FfiGuiTiledQuadRequest>() as u64))
        .saturating_add(
            request
                .world_segments
                .count
                .saturating_mul(size_of::<FfiWorldLineSegmentRequest>() as u64),
        )
        .saturating_add(
            request
                .world_crack_quads
                .count
                .saturating_mul(size_of::<FfiWorldCrackQuadRequest>() as u64),
        )
        .saturating_add(
            request
                .world_border_quads
                .count
                .saturating_mul(size_of::<FfiWorldBorderQuadRequest>() as u64),
        )
        .saturating_add(
            request
                .world_material_quads
                .count
                .saturating_mul(size_of::<FfiWorldMaterialQuadRequest>() as u64),
        )
        .saturating_add(
            request
                .world_material_table
                .count
                .saturating_mul(size_of::<FfiWorldMaterialTableRecord>() as u64),
        )
        .saturating_add(
            request
                .world_material_compact_quads
                .count
                .saturating_mul(size_of::<FfiWorldMaterialCompactQuadRequest>() as u64),
        )
        .saturating_add(
            request
                .world_mesh_instances
                .count
                .saturating_mul(size_of::<FfiWorldMeshInstanceRecord>() as u64),
        )
        .saturating_add(
            request
                .world_lod_instances
                .count
                .saturating_mul(size_of::<FfiWorldLodColumnInstanceRecord>() as u64),
        )
        .saturating_add(
            request
                .gui_sprites
                .count
                .saturating_mul(size_of::<FfiGuiSpriteRequest>() as u64),
        )
        .saturating_add(
            request
                .gui_affine_quads
                .count
                .saturating_mul(size_of::<FfiGuiAffineQuadRequest>() as u64),
        )
        .saturating_add(
            request
                .world_text_quads
                .count
                .saturating_mul(size_of::<FfiWorldTextQuadRequest>() as u64),
        )
        .saturating_add(
            request
                .gui_mesh_batches
                .count
                .saturating_mul(size_of::<FfiGuiMeshBatchRequest>() as u64),
        )
        .saturating_add(
            request
                .world_first_person_mesh_instances
                .count
                .saturating_mul(size_of::<FfiWorldMeshInstanceRecord>() as u64),
        )
        .saturating_add(request.post_effect_id.len)
}

pub(crate) fn input_bytes_for_gui_asset_update(request: &FfiGuiAssetUpdateRequest) -> u64 {
    let payload_headers = request
        .assets
        .count
        .saturating_mul(size_of::<FfiGuiAssetPayload>() as u64);
    let payload_bytes = unsafe { read_slice(request.assets, true, "GUI asset payloads") }
        .map(|items| {
            items
                .iter()
                .fold(0u64, |sum, item| sum.saturating_add(item.png_bytes.len))
        })
        .unwrap_or(0);
    (size_of::<FfiGuiAssetUpdateRequest>() as u64)
        .saturating_add(payload_headers)
        .saturating_add(payload_bytes)
}

pub(crate) fn input_bytes_for_gui_raw_image_update(request: &FfiGuiRawImageUpdateRequest) -> u64 {
    let payload_headers = request
        .assets
        .count
        .saturating_mul(size_of::<FfiGuiRawImageAssetPayload>() as u64);
    let payload_bytes = unsafe { read_slice(request.assets, true, "raw GUI image payloads") }
        .map(|items| {
            items
                .iter()
                .fold(0u64, |sum, item| sum.saturating_add(item.pixels.len))
        })
        .unwrap_or(0);
    (size_of::<FfiGuiRawImageUpdateRequest>() as u64)
        .saturating_add(payload_headers)
        .saturating_add(payload_bytes)
}

pub(crate) fn input_bytes_for_world_text_image_update(
    request: &FfiWorldTextImageUpdateRequest,
) -> u64 {
    let payload_headers = request
        .assets
        .count
        .saturating_mul(size_of::<FfiWorldTextImageAssetPayload>() as u64);
    let payload_bytes = unsafe { read_slice(request.assets, true, "world text image assets") }
        .map(|items| {
            items
                .iter()
                .fold(0u64, |sum, item| sum.saturating_add(item.pixels.len))
        })
        .unwrap_or(0);
    (size_of::<FfiWorldTextImageUpdateRequest>() as u64)
        .saturating_add(payload_headers)
        .saturating_add(payload_bytes)
}

pub(crate) fn input_bytes_for_world_border_asset_update(
    request: &FfiWorldBorderAssetUpdateRequest,
) -> u64 {
    (size_of::<FfiWorldBorderAssetUpdateRequest>() as u64).saturating_add(request.png_bytes.len)
}

pub(crate) fn input_bytes_for_world_crack_asset_update(
    request: &FfiWorldCrackAssetUpdateRequest,
) -> u64 {
    let payload_headers = request
        .assets
        .count
        .saturating_mul(size_of::<FfiWorldCrackAssetPayload>() as u64);
    let payload_bytes = unsafe { read_slice(request.assets, true, "world crack asset payloads") }
        .map(|items| {
            items
                .iter()
                .fold(0u64, |sum, item| sum.saturating_add(item.png_bytes.len))
        })
        .unwrap_or(0);
    (size_of::<FfiWorldCrackAssetUpdateRequest>() as u64)
        .saturating_add(payload_headers)
        .saturating_add(payload_bytes)
}

pub(crate) fn input_bytes_for_world_material_asset_update(
    request: &FfiWorldMaterialAssetUpdateRequest,
) -> u64 {
    let payload_headers = request
        .assets
        .count
        .saturating_mul(size_of::<FfiWorldMaterialAssetPayload>() as u64);
    let payload_bytes =
        unsafe { read_slice(request.assets, true, "world material asset payloads") }
            .map(|items| {
                items
                    .iter()
                    .fold(0u64, |sum, item| sum.saturating_add(item.png_bytes.len))
            })
            .unwrap_or(0);
    (size_of::<FfiWorldMaterialAssetUpdateRequest>() as u64)
        .saturating_add(payload_headers)
        .saturating_add(payload_bytes)
}

pub(crate) fn input_bytes_for_world_mesh_asset_update(
    request: &FfiWorldMeshAssetUpdateRequest,
) -> u64 {
    let mesh_headers = request
        .meshes
        .count
        .saturating_mul(size_of::<FfiWorldMeshAssetRecord>() as u64);
    let texture_headers = request
        .textures
        .count
        .saturating_mul(size_of::<FfiWorldMeshTextureAssetPayload>() as u64);
    let sorted_index_headers = request
        .sorted_indices
        .count
        .saturating_mul(size_of::<FfiWorldMeshSortedIndexRecord>() as u64);
    let retirement_headers = request
        .retirements
        .count
        .saturating_mul(size_of::<FfiWorldMeshAssetRetirementRecord>() as u64);
    let sorted_index_payload_bytes = unsafe {
        read_slice(
            request.sorted_indices,
            true,
            "world mesh sorted index updates",
        )
    }
    .map(|items| {
        items
            .iter()
            .fold(0u64, |sum, item| sum.saturating_add(item.index_bytes.len))
    })
    .unwrap_or(0);
    (size_of::<FfiWorldMeshAssetUpdateRequest>() as u64)
        .saturating_add(mesh_headers)
        .saturating_add(request.experience_orbs.count.saturating_mul(size_of::<FfiWorldExperienceOrbAssetRecord>() as u64))
        .saturating_add(texture_headers)
        .saturating_add(sorted_index_headers)
        .saturating_add(retirement_headers)
        .saturating_add(sorted_index_payload_bytes)
}

pub(crate) fn input_bytes_for_world_lod_asset_update(
    request: &FfiWorldLodAssetUpdateRequest,
) -> u64 {
    let retirement_headers = request
        .retirements
        .count
        .saturating_mul(size_of::<FfiWorldLodColumnRetirementRecord>() as u64);
    let asset_bytes =
        unsafe { read_slice(request.assets, true, "world LOD assets") }
            .map(|assets| {
                assets.iter().fold(0u64, |sum, asset| {
                    let segment_bytes =
                        unsafe { read_slice(asset.segments, true, "world LOD asset segments") }
                            .map(|segments| {
                                segments.iter().fold(0u64, |segment_sum, segment| {
                                    segment_sum
                            .saturating_add(size_of::<FfiWorldLodSegmentRecord>() as u64)
                            .saturating_add(
                                segment.vertices.count.saturating_mul(
                                    size_of::<FfiWorldLodVertex>() as u64,
                                ),
                            )
                                })
                            })
                            .unwrap_or(0);
                    sum.saturating_add(size_of::<FfiWorldLodColumnAssetRecord>() as u64)
                        .saturating_add(segment_bytes)
                })
            })
            .unwrap_or(0);
    let provenance_bytes = unsafe {
        read_slice(
            request.material_provenance,
            true,
            "world LOD material provenance",
        )
    }
    .map(|columns| {
        columns.iter().fold(0u64, |sum, column| {
            let identity_bytes =
                unsafe { read_slice(column.identities, true, "world LOD material identities") }
                    .map(|identities| {
                        identities.iter().fold(0u64, |identity_sum, identity| {
                            identity_sum
                                .saturating_add(
                                    size_of::<FfiWorldLodMaterialIdentityRecord>() as u64
                                )
                                .saturating_add(identity.block_state_identity_utf8.len)
                                .saturating_add(identity.biome_identity_utf8.len)
                        })
                    })
                    .unwrap_or(0);
            let segment_bytes = unsafe {
                read_slice(
                    column.segments,
                    true,
                    "world LOD segment material provenance",
                )
            }
            .map(|segments| {
                segments.iter().fold(0u64, |segment_sum, segment| {
                    segment_sum
                        .saturating_add(
                            size_of::<FfiWorldLodSegmentMaterialProvenanceRecord>() as u64
                        )
                        .saturating_add(
                            segment
                                .quad_material_ids
                                .count
                                .saturating_mul(size_of::<u32>() as u64),
                        )
                        .saturating_add(segment.quad_variant_states.count)
                        .saturating_add(
                            segment
                                .quad_variant_positions
                                .count
                                .saturating_mul(size_of::<u64>() as u64),
                        )
                })
            })
            .unwrap_or(0);
            sum.saturating_add(size_of::<FfiWorldLodColumnMaterialProvenanceRecord>() as u64)
                .saturating_add(identity_bytes)
                .saturating_add(segment_bytes)
        })
    })
    .unwrap_or(0);
    (size_of::<FfiWorldLodAssetUpdateRequest>() as u64)
        .saturating_add(asset_bytes)
        .saturating_add(retirement_headers)
        .saturating_add(provenance_bytes)
}

pub(crate) fn input_bytes_for_shader_pack_source_update(
    request: &FfiShaderPackSourceUpdateRequest,
) -> u64 {
    let file_headers = request
        .files
        .count
        .saturating_mul(size_of::<FfiShaderPackSourceFile>() as u64);
    let file_bytes = unsafe { read_slice(request.files, true, "shader-pack source files") }
        .map(|files| {
            files.iter().fold(0u64, |sum, file| {
                sum.saturating_add(file.path_utf8.len)
                    .saturating_add(file.contents_utf8.len)
            })
        })
        .unwrap_or(0);
    (size_of::<FfiShaderPackSourceUpdateRequest>() as u64)
        .saturating_add(request.pack_name_utf8.len)
        .saturating_add(file_headers)
        .saturating_add(file_bytes)
}

pub(crate) fn input_bytes_for_shader_pack_asset_update(
    request: &FfiShaderPackAssetUpdateRequest,
) -> u64 {
    let file_headers = request
        .files
        .count
        .saturating_mul(size_of::<FfiShaderPackAssetFile>() as u64);
    let file_bytes = unsafe { read_slice(request.files, true, "shader-pack asset files") }
        .map(|files| {
            files.iter().fold(0u64, |sum, file| {
                sum.saturating_add(file.path_utf8.len)
                    .saturating_add(file.contents.len)
            })
        })
        .unwrap_or(0);
    (size_of::<FfiShaderPackAssetUpdateRequest>() as u64)
        .saturating_add(request.pack_name_utf8.len)
        .saturating_add(file_headers)
        .saturating_add(file_bytes)
}

pub(crate) fn set_feature(bits: &mut u64, feature: u64, enabled: bool) {
    if enabled {
        *bits |= feature;
    }
}

pub(crate) fn feature_from_bit(bit: u64) -> Option<BackendFeature> {
    match bit {
        FfiFeatureBits::GRAPHICS => Some(BackendFeature::Graphics),
        FfiFeatureBits::COMPUTE => Some(BackendFeature::Compute),
        FfiFeatureBits::DESCRIPTOR_ARRAYS => Some(BackendFeature::DescriptorArrays),
        FfiFeatureBits::OPTIONAL_BINDINGS => Some(BackendFeature::OptionalBindings),
        FfiFeatureBits::DYNAMIC_BUFFER_OFFSETS => Some(BackendFeature::DynamicBufferOffsets),
        FfiFeatureBits::UNIFORM_BUFFERS => Some(BackendFeature::UniformBuffers),
        FfiFeatureBits::STORAGE_BUFFERS => Some(BackendFeature::StorageBuffers),
        FfiFeatureBits::STORAGE_TEXTURES => Some(BackendFeature::StorageTextures),
        FfiFeatureBits::INDIRECT_DRAW => Some(BackendFeature::IndirectDraw),
        FfiFeatureBits::INDIRECT_DISPATCH => Some(BackendFeature::IndirectDispatch),
        FfiFeatureBits::MULTIPLE_COLOR_ATTACHMENTS => {
            Some(BackendFeature::MultipleColorAttachments)
        }
        FfiFeatureBits::DEPTH_ONLY_PASS => Some(BackendFeature::DepthOnlyPass),
        FfiFeatureBits::BLENDED_PASS => Some(BackendFeature::BlendedPass),
        FfiFeatureBits::TEXTURE_SUBRESOURCE_COPIES => {
            Some(BackendFeature::TextureSubresourceCopies)
        }
        FfiFeatureBits::TEXTURE_MIP_LEVELS => Some(BackendFeature::TextureMipLevels),
        FfiFeatureBits::TEXTURE_ARRAY_LAYERS => Some(BackendFeature::TextureArrayLayers),
        FfiFeatureBits::HOST_BUFFER_ACCESS => Some(BackendFeature::HostBufferAccess),
        FfiFeatureBits::PRESENTATION => Some(BackendFeature::Presentation),
        FfiFeatureBits::RENDERDOC_CAPTURE => Some(BackendFeature::RenderDocCapture),
        FfiFeatureBits::TRACY_ZONES => Some(BackendFeature::TracyZones),
        _ => None,
    }
}

pub(crate) fn reject_unknown_feature_bits(bits: u64) -> GalResult<()> {
    if bits & !FfiFeatureBits::ALL_KNOWN != 0 {
        return Err(GalError::ffi(
            StatusCode::UnknownEnum,
            format!(
                "unknown negotiated feature bits 0x{:x}",
                bits & !FfiFeatureBits::ALL_KNOWN
            ),
        ));
    }
    Ok(())
}

pub(crate) fn require_negotiated_features(
    bits: u64,
    capabilities: BackendCapabilities,
) -> GalResult<()> {
    for index in 0..64 {
        let bit = 1_u64 << index;
        if bits & bit == 0 {
            continue;
        }
        let Some(feature) = feature_from_bit(bit) else {
            continue;
        };
        if !capabilities.supports(feature) {
            return Err(GalError::unsupported_feature(format!(
                "negotiated feature {:?} is unsupported by backend '{}'",
                feature, capabilities.name
            )));
        }
    }
    Ok(())
}

pub(crate) fn require_any_handle(handle: FfiHandle, label: &str) -> GalResult<Handle> {
    let handle = Handle::from(handle);
    if handle.is_null() || handle.kind().is_none() {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!("{label} is null or has unknown kind"),
        ));
    }
    Ok(handle)
}

pub(crate) fn require_handle(
    handle: FfiHandle,
    kind: HandleKind,
    label: &str,
) -> GalResult<Handle> {
    let handle = Handle::from(handle);
    if handle.is_null() {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!("{label} handle is null"),
        ));
    }
    handle.require_kind(kind)?;
    Ok(handle)
}

pub(crate) fn require_handle_any(
    handle: FfiHandle,
    kinds: &[HandleKind],
    label: &str,
) -> GalResult<Handle> {
    let handle = Handle::from(handle);
    if handle.is_null() {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!("{label} handle is null"),
        ));
    }
    let Some(actual) = handle.kind() else {
        return Err(GalError::ffi(
            StatusCode::WrongHandleType,
            format!("{label} has unknown handle kind"),
        ));
    };
    if !kinds.contains(&actual) {
        return Err(GalError::ffi(
            StatusCode::WrongHandleType,
            format!("{label} has kind {actual:?}, expected one of {kinds:?}"),
        ));
    }
    Ok(handle)
}

pub(crate) fn optional_handle(
    handle: FfiHandle,
    kind: HandleKind,
    label: &str,
) -> GalResult<Option<Handle>> {
    if handle.raw == 0 {
        return Ok(None);
    }
    Ok(Some(require_handle(handle, kind, label)?))
}

pub(crate) fn handle_kind(raw: u32) -> GalResult<HandleKind> {
    HandleKind::from_raw(u8::try_from(raw).unwrap_or(0)).ok_or_else(|| {
        GalError::ffi(
            StatusCode::UnknownEnum,
            format!("unknown handle kind {raw}"),
        )
    })
}

pub(crate) fn memory_domain(raw: u32) -> GalResult<MemoryDomain> {
    match raw {
        1 => Ok(MemoryDomain::DeviceLocal),
        2 => Ok(MemoryDomain::Upload),
        3 => Ok(MemoryDomain::Readback),
        _ => Err(GalError::ffi(
            StatusCode::UnknownEnum,
            format!("unknown memory domain {raw}"),
        )),
    }
}

pub(crate) fn texture_dimension(raw: u32) -> GalResult<TextureDimension> {
    match raw {
        1 => Ok(TextureDimension::D1),
        2 => Ok(TextureDimension::D2),
        3 => Ok(TextureDimension::D3),
        4 => Ok(TextureDimension::Cube),
        _ => Err(GalError::ffi(
            StatusCode::UnknownEnum,
            format!("unknown texture dimension {raw}"),
        )),
    }
}

pub(crate) fn texture_format(raw: u32) -> GalResult<TextureFormat> {
    match raw {
        1 => Ok(TextureFormat::Rgba8Unorm),
        2 => Ok(TextureFormat::Bgra8Unorm),
        3 => Ok(TextureFormat::Rgba16Float),
        4 => Ok(TextureFormat::Depth24Stencil8),
        5 => Ok(TextureFormat::Depth32Float),
        6 => Ok(TextureFormat::R8Uint),
        7 => Ok(TextureFormat::R11fG11fB10f),
        8 => Ok(TextureFormat::R32Float),
        9 => Ok(TextureFormat::Rgb16Float),
        10 => Ok(TextureFormat::R8Unorm),
        11 => Ok(TextureFormat::Rgba8Snorm),
        _ => Err(GalError::ffi(
            StatusCode::UnknownEnum,
            format!("unknown texture format {raw}"),
        )),
    }
}

pub(crate) fn optional_texture_format(raw: u32) -> GalResult<Option<TextureFormat>> {
    if raw == 0 {
        Ok(None)
    } else {
        texture_format(raw).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texture_format_decodes_append_only_shader_pack_color_formats() {
        assert_eq!(TextureFormat::R8Uint, texture_format(6).unwrap());
        assert_eq!(TextureFormat::R11fG11fB10f, texture_format(7).unwrap());
        assert_eq!(TextureFormat::R32Float, texture_format(8).unwrap());
        assert_eq!(TextureFormat::Rgb16Float, texture_format(9).unwrap());
        assert_eq!(TextureFormat::R8Unorm, texture_format(10).unwrap());
        assert_eq!(TextureFormat::Rgba8Snorm, texture_format(11).unwrap());
        assert!(texture_format(12).is_err());
    }
}

pub(crate) fn present_mode(raw: u32) -> GalResult<PresentMode> {
    match raw {
        1 => Ok(PresentMode::Immediate),
        2 => Ok(PresentMode::Mailbox),
        3 => Ok(PresentMode::Fifo),
        4 => Ok(PresentMode::AutoVsync),
        5 => Ok(PresentMode::AutoNoVsync),
        6 => Ok(PresentMode::FifoRelaxed),
        _ => Err(GalError::ffi(
            StatusCode::UnknownEnum,
            format!("unknown present mode {raw}"),
        )),
    }
}

pub(crate) fn acquire_status_raw(status: FrameAcquireStatus) -> u32 {
    status as u32
}

pub(crate) fn present_status_raw(status: FramePresentStatus) -> u32 {
    status as u32
}

pub(crate) fn sampler_filter(raw: u32) -> GalResult<SamplerFilter> {
    match raw {
        1 => Ok(SamplerFilter::Nearest),
        2 => Ok(SamplerFilter::Linear),
        _ => Err(GalError::ffi(
            StatusCode::UnknownEnum,
            format!("unknown sampler filter {raw}"),
        )),
    }
}

pub(crate) fn sampler_address(raw: u32) -> GalResult<SamplerAddressMode> {
    match raw {
        1 => Ok(SamplerAddressMode::ClampToEdge),
        2 => Ok(SamplerAddressMode::Repeat),
        3 => Ok(SamplerAddressMode::MirroredRepeat),
        _ => Err(GalError::ffi(
            StatusCode::UnknownEnum,
            format!("unknown sampler address mode {raw}"),
        )),
    }
}

pub(crate) fn shader_stage(raw: u32) -> GalResult<ShaderStage> {
    match raw {
        1 => Ok(ShaderStage::Vertex),
        2 => Ok(ShaderStage::Fragment),
        3 => Ok(ShaderStage::Compute),
        4 => Ok(ShaderStage::Geometry),
        5 => Ok(ShaderStage::TessControl),
        6 => Ok(ShaderStage::TessEvaluation),
        _ => Err(GalError::ffi(
            StatusCode::UnknownEnum,
            format!("unknown shader stage {raw}"),
        )),
    }
}

pub(crate) fn shader_code_format(raw: u32) -> GalResult<ShaderCodeFormat> {
    match raw {
        1 => Ok(ShaderCodeFormat::Spirv),
        2 => Ok(ShaderCodeFormat::BackendPortableIr),
        3 => Ok(ShaderCodeFormat::Glsl),
        _ => Err(GalError::ffi(
            StatusCode::UnknownEnum,
            format!("unknown shader code format {raw}"),
        )),
    }
}

pub(crate) fn resource_binding_kind(raw: u32) -> GalResult<ResourceBindingKind> {
    match raw {
        1 => Ok(ResourceBindingKind::UniformBuffer),
        2 => Ok(ResourceBindingKind::StorageBuffer),
        3 => Ok(ResourceBindingKind::SampledTexture),
        4 => Ok(ResourceBindingKind::StorageTexture),
        5 => Ok(ResourceBindingKind::Sampler),
        _ => Err(GalError::ffi(
            StatusCode::UnknownEnum,
            format!("unknown resource binding kind {raw}"),
        )),
    }
}

pub(crate) fn primitive_topology(raw: u32) -> GalResult<PrimitiveTopology> {
    match raw {
        1 => Ok(PrimitiveTopology::Points),
        2 => Ok(PrimitiveTopology::Lines),
        3 => Ok(PrimitiveTopology::Triangles),
        4 => Ok(PrimitiveTopology::TriangleFan),
        _ => Err(GalError::ffi(
            StatusCode::UnknownEnum,
            format!("unknown primitive topology {raw}"),
        )),
    }
}

pub(crate) fn cull_mode(raw: u32) -> GalResult<CullMode> {
    match raw {
        1 => Ok(CullMode::None),
        2 => Ok(CullMode::Front),
        3 => Ok(CullMode::Back),
        _ => Err(GalError::ffi(
            StatusCode::UnknownEnum,
            format!("unknown cull mode {raw}"),
        )),
    }
}

pub(crate) fn blend_mode(raw: u32) -> GalResult<BlendMode> {
    match raw {
        1 => Ok(BlendMode::Disabled),
        2 => Ok(BlendMode::Alpha),
        3 => Ok(BlendMode::Additive),
        4 => Ok(BlendMode::Invert),
        5 => Ok(BlendMode::Multiply),
        6 => Ok(BlendMode::Overlay),
        8 => Ok(BlendMode::Vignette),
        _ => Err(GalError::ffi(
            StatusCode::UnknownEnum,
            format!("unknown blend mode {raw}"),
        )),
    }
}

pub(crate) fn compare_op(raw: u32) -> GalResult<CompareOp> {
    match raw {
        1 => Ok(CompareOp::Always),
        2 => Ok(CompareOp::Less),
        3 => Ok(CompareOp::LessOrEqual),
        4 => Ok(CompareOp::Equal),
        _ => Err(GalError::ffi(
            StatusCode::UnknownEnum,
            format!("unknown compare op {raw}"),
        )),
    }
}

pub(crate) fn optional_compare_op(raw: u32) -> GalResult<Option<CompareOp>> {
    if raw == 0 {
        Ok(None)
    } else {
        compare_op(raw).map(Some)
    }
}

pub(crate) fn load_op(raw: u32) -> GalResult<AttachmentLoadOp> {
    match raw {
        1 => Ok(AttachmentLoadOp::Load),
        2 => Ok(AttachmentLoadOp::Clear),
        3 => Ok(AttachmentLoadOp::DontCare),
        _ => Err(GalError::ffi(
            StatusCode::UnknownEnum,
            format!("unknown attachment load op {raw}"),
        )),
    }
}

pub(crate) fn store_op(raw: u32) -> GalResult<AttachmentStoreOp> {
    match raw {
        1 => Ok(AttachmentStoreOp::Store),
        2 => Ok(AttachmentStoreOp::DontCare),
        _ => Err(GalError::ffi(
            StatusCode::UnknownEnum,
            format!("unknown attachment store op {raw}"),
        )),
    }
}

pub(crate) fn texture_usage_state(raw: u32) -> GalResult<TextureUsageState> {
    match raw {
        1 => Ok(TextureUsageState::Undefined),
        2 => Ok(TextureUsageState::ShaderRead),
        3 => Ok(TextureUsageState::ShaderWrite),
        4 => Ok(TextureUsageState::ColorAttachment),
        5 => Ok(TextureUsageState::DepthStencilAttachment),
        6 => Ok(TextureUsageState::TransferSrc),
        7 => Ok(TextureUsageState::TransferDst),
        8 => Ok(TextureUsageState::Present),
        9 => Ok(TextureUsageState::IndexRead),
        10 => Ok(TextureUsageState::ShaderStorageRead),
        _ => Err(GalError::ffi(
            StatusCode::UnknownEnum,
            format!("unknown texture usage state {raw}"),
        )),
    }
}

pub(crate) fn ffi_index_type(raw: u32) -> GalResult<IndexType> {
    match raw {
        0 | 2 => Ok(IndexType::U32),
        1 => Ok(IndexType::U16),
        _ => Err(GalError::ffi(
            StatusCode::UnknownEnum,
            format!("unknown index type {raw}"),
        )),
    }
}

pub(crate) fn queue_class(raw: u32) -> GalResult<QueueClass> {
    match raw {
        1 => Ok(QueueClass::Graphics),
        2 => Ok(QueueClass::Compute),
        3 => Ok(QueueClass::Transfer),
        4 => Ok(QueueClass::Present),
        5 => Ok(QueueClass::External),
        _ => Err(GalError::ffi(
            StatusCode::UnknownEnum,
            format!("unknown queue class {raw}"),
        )),
    }
}

pub(crate) fn stage_flags(bits: u32) -> GalResult<PipelineStageFlags> {
    let known = PipelineStageFlags::DRAW.0
        | PipelineStageFlags::COMPUTE.0
        | PipelineStageFlags::TRANSFER.0
        | PipelineStageFlags::PRESENT.0;
    if bits & !known != 0 {
        return Err(GalError::ffi(
            StatusCode::UnknownEnum,
            format!("unknown pipeline stage bits 0x{:x}", bits & !known),
        ));
    }
    Ok(PipelineStageFlags(bits))
}

pub(crate) fn access_flags(bits: u32) -> GalResult<AccessFlags> {
    let known = AccessFlags::READ.0
        | AccessFlags::WRITE.0
        | AccessFlags::COLOR_ATTACHMENT.0
        | AccessFlags::DEPTH_STENCIL.0
        | AccessFlags::TRANSFER.0;
    if bits & !known != 0 {
        return Err(GalError::ffi(
            StatusCode::UnknownEnum,
            format!("unknown access bits 0x{:x}", bits & !known),
        ));
    }
    Ok(AccessFlags(bits))
}

pub(crate) fn buffer_usage_bits(bits: u64) -> GalResult<Vec<BufferUsage>> {
    let table = [
        (1_u64 << 0, BufferUsage::Vertex),
        (1_u64 << 1, BufferUsage::Index),
        (1_u64 << 2, BufferUsage::Uniform),
        (1_u64 << 3, BufferUsage::Storage),
        (1_u64 << 4, BufferUsage::TransferSrc),
        (1_u64 << 5, BufferUsage::TransferDst),
        (1_u64 << 6, BufferUsage::Indirect),
        (1_u64 << 7, BufferUsage::HostRead),
        (1_u64 << 8, BufferUsage::HostWrite),
    ];
    usage_bits(bits, &table, "buffer usage")
}

pub(crate) fn texture_usage_bits(bits: u64) -> GalResult<Vec<TextureUsage>> {
    let table = [
        (1_u64 << 0, TextureUsage::Sampled),
        (1_u64 << 1, TextureUsage::Storage),
        (1_u64 << 2, TextureUsage::ColorAttachment),
        (1_u64 << 3, TextureUsage::DepthStencilAttachment),
        (1_u64 << 4, TextureUsage::TransferSrc),
        (1_u64 << 5, TextureUsage::TransferDst),
        (1_u64 << 6, TextureUsage::Present),
        (1_u64 << 7, TextureUsage::HostRead),
        (1_u64 << 8, TextureUsage::HostWrite),
    ];
    usage_bits(bits, &table, "texture usage")
}

pub(crate) fn usage_bits<T: Copy>(bits: u64, table: &[(u64, T)], label: &str) -> GalResult<Vec<T>> {
    let known = table.iter().fold(0_u64, |acc, (bit, _)| acc | *bit);
    if bits == 0 {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!("{label} bits must be non-zero"),
        ));
    }
    if bits & !known != 0 {
        return Err(GalError::ffi(
            StatusCode::UnknownEnum,
            format!("unknown {label} bits 0x{:x}", bits & !known),
        ));
    }
    Ok(table
        .iter()
        .filter_map(|(bit, value)| if bits & *bit != 0 { Some(*value) } else { None })
        .collect())
}

pub(crate) fn check_buffer_capabilities(
    desc: &BufferDesc,
    capabilities: BackendCapabilities,
) -> GalResult<()> {
    if desc.size > capabilities.limits.max_buffer_size {
        return Err(GalError::unsupported_feature(
            "buffer size exceeds backend limit",
        ));
    }
    if desc.usages.contains(&BufferUsage::Storage) {
        require_feature(
            capabilities,
            BackendFeature::StorageBuffers,
            "storage buffer",
        )?;
    }
    if desc.usages.contains(&BufferUsage::Indirect) {
        require_any_feature(
            capabilities,
            &[
                BackendFeature::IndirectDraw,
                BackendFeature::IndirectDispatch,
            ],
            "indirect buffer",
        )?;
    }
    if desc.usages.contains(&BufferUsage::HostRead) || desc.usages.contains(&BufferUsage::HostWrite)
    {
        require_feature(
            capabilities,
            BackendFeature::HostBufferAccess,
            "host buffer access",
        )?;
    }
    Ok(())
}

pub(crate) fn check_texture_capabilities(
    desc: &TextureDesc,
    capabilities: BackendCapabilities,
) -> GalResult<()> {
    // Stable Java resource batches intentionally remain D2-only. D3 is a
    // private backend/GAL prerequisite until a versioned semantic volume
    // transport owns generation, updates, and lifetime end to end.
    if desc.dimension == TextureDimension::D3 {
        return Err(GalError::unsupported_feature(
            "D3 texture resources are internal-only; the stable FFI has no semantic volume transport",
        ));
    }
    if desc.extent.width > capabilities.limits.max_texture_extent_2d
        || desc.extent.height > capabilities.limits.max_texture_extent_2d
    {
        return Err(GalError::unsupported_feature(
            "texture extent exceeds backend limit",
        ));
    }
    if desc.mip_levels > capabilities.limits.max_texture_mip_levels {
        return Err(GalError::unsupported_feature(
            "texture mip count exceeds backend limit",
        ));
    }
    if desc.array_layers > capabilities.limits.max_texture_array_layers {
        return Err(GalError::unsupported_feature(
            "texture layer count exceeds backend limit",
        ));
    }
    if desc.mip_levels > 1 {
        require_feature(
            capabilities,
            BackendFeature::TextureMipLevels,
            "texture mip levels",
        )?;
    }
    if desc.array_layers > 1 {
        require_feature(
            capabilities,
            BackendFeature::TextureArrayLayers,
            "texture array layers",
        )?;
    }
    if desc.usages.contains(&TextureUsage::Storage) {
        require_feature(
            capabilities,
            BackendFeature::StorageTextures,
            "storage texture",
        )?;
    }
    if desc.usages.contains(&TextureUsage::Present) {
        require_feature(
            capabilities,
            BackendFeature::Presentation,
            "presentation texture",
        )?;
        return Err(GalError::unsupported_feature(
            "presentation texture creation is outside the batch ABI; use ABI v2 frame targets",
        ));
    }
    Ok(())
}

pub(crate) fn check_graphics_pipeline_capabilities(
    desc: &GraphicsPipelineDesc,
    capabilities: BackendCapabilities,
) -> GalResult<()> {
    require_feature(capabilities, BackendFeature::Graphics, "graphics pipeline")?;
    check_attachment_count(
        desc.color_formats.len(),
        desc.depth_format.is_some(),
        capabilities,
    )?;
    if desc.blend != BlendMode::Disabled {
        require_feature(capabilities, BackendFeature::BlendedPass, "blended pass")?;
    }
    Ok(())
}

pub(crate) fn check_attachment_count(
    color_count: usize,
    has_depth: bool,
    capabilities: BackendCapabilities,
) -> GalResult<()> {
    if color_count > capabilities.limits.max_color_attachments as usize {
        return Err(GalError::unsupported_feature(
            "color attachment count exceeds backend limit",
        ));
    }
    if color_count > 1 {
        require_feature(
            capabilities,
            BackendFeature::MultipleColorAttachments,
            "multiple color attachments",
        )?;
    }
    if color_count == 0 && has_depth {
        require_feature(
            capabilities,
            BackendFeature::DepthOnlyPass,
            "depth-only pass",
        )?;
    }
    Ok(())
}

pub(crate) fn require_feature(
    capabilities: BackendCapabilities,
    feature: BackendFeature,
    label: &str,
) -> GalResult<()> {
    if capabilities.supports(feature) {
        Ok(())
    } else {
        Err(GalError::unsupported_feature(format!(
            "backend '{}' does not support {label}",
            capabilities.name
        )))
    }
}

pub(crate) fn require_any_feature(
    capabilities: BackendCapabilities,
    features: &[BackendFeature],
    label: &str,
) -> GalResult<()> {
    if features
        .iter()
        .any(|feature| capabilities.supports(*feature))
    {
        Ok(())
    } else {
        Err(GalError::unsupported_feature(format!(
            "backend '{}' does not support {label}",
            capabilities.name
        )))
    }
}
