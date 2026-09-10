use super::*;
use std::mem::offset_of;

pub(crate) fn layout_for_struct(struct_id: u32) -> GalResult<FfiStructLayout> {
    macro_rules! layout {
        ($id:expr, $ty:ty, [$($field:tt),* $(,)?]) => {{
            let mut offsets = [0_u32; 64];
            let fields = [$(offset_of!($ty, $field) as u32),*];
            offsets[..fields.len()].copy_from_slice(&fields);
            FfiStructLayout {
                header: FfiHeader {
                    version: FFI_ABI_VERSION,
                    byte_size: size_of::<FfiStructLayout>() as u32,
                },
                struct_id: $id,
                byte_size: size_of::<$ty>() as u32,
                alignment: align_of::<$ty>() as u32,
                field_count: fields.len() as u32,
                field_offsets: offsets,
            }
        }};
    }
    Ok(match struct_id {
        1 => layout!(1, FfiHeader, [version, byte_size]),
        2 => layout!(2, FfiBytes, [ptr, len]),
        3 => layout!(3, FfiHandle, [raw]),
        4 => layout!(
            4,
            FfiContextCreateRequest,
            [header, backend_kind, tracy_enabled, label]
        ),
        5 => layout!(
            5,
            FfiContextResult,
            [
                header,
                status,
                error_domain,
                context_id,
                supported_feature_bits,
                limits,
                metrics
            ]
        ),
        6 => layout!(
            6,
            FfiCapabilityQueryRequest,
            [header, requested_feature_bits, reserved0]
        ),
        7 => layout!(
            7,
            FfiCapabilityResult,
            [
                header,
                status,
                error_domain,
                supported_feature_bits,
                negotiated_feature_bits,
                limits,
                initial_presentation_supported
            ]
        ),
        8 => layout!(
            8,
            FfiStatusResult,
            [
                header,
                status,
                error_domain,
                unsupported_feature,
                primary_handle,
                submission_id,
                required_bytes,
                metrics
            ]
        ),
        9 => layout!(
            9,
            FfiCreateResultEntry,
            [request_id, handle, status, error_domain]
        ),
        10 => layout!(
            10,
            FfiBufferDescAbi,
            [
                byte_size,
                request_id,
                label,
                size,
                memory_domain,
                usage_bits
            ]
        ),
        11 => layout!(
            11,
            FfiTextureDescAbi,
            [
                byte_size,
                request_id,
                label,
                dimension,
                format,
                extent,
                mip_levels,
                array_layers,
                usage_bits
            ]
        ),
        12 => layout!(
            12,
            FfiTextureViewDescAbi,
            [
                byte_size,
                request_id,
                label,
                texture,
                format,
                base_mip,
                mip_count,
                base_layer,
                layer_count
            ]
        ),
        13 => layout!(
            13,
            FfiSamplerDescAbi,
            [
                byte_size, request_id, label, min_filter, mag_filter, mip_filter, address_u,
                address_v, address_w
            ]
        ),
        14 => layout!(
            14,
            FfiShaderModuleDescAbi,
            [
                byte_size,
                request_id,
                label,
                stage,
                code_format,
                code,
                entry_point
            ]
        ),
        15 => layout!(
            15,
            FfiResourceBindingDescAbi,
            [
                byte_size,
                binding,
                kind,
                stage_bits,
                array_count,
                optional,
                dynamic_offset_count
            ]
        ),
        16 => layout!(
            16,
            FfiResourceLayoutDescAbi,
            [byte_size, request_id, label, bindings]
        ),
        17 => layout!(
            17,
            FfiResourceBindingAbi,
            [
                byte_size,
                binding,
                array_index,
                resource,
                kind,
                access_bits,
                dynamic_offsets
            ]
        ),
        18 => layout!(
            18,
            FfiResourceSetDescAbi,
            [byte_size, request_id, label, layout, bindings]
        ),
        19 => layout!(
            19,
            FfiPipelineLayoutDescAbi,
            [byte_size, request_id, label, resource_layouts]
        ),
        20 => layout!(
            20,
            FfiGraphicsPipelineDescAbi,
            [
                byte_size,
                request_id,
                label,
                layout,
                vertex_shader,
                fragment_shader,
                topology,
                cull_mode,
                blend,
                depth_compare,
                color_formats,
                depth_format
            ]
        ),
        21 => layout!(
            21,
            FfiRenderTargetDescAbi,
            [
                byte_size,
                request_id,
                label,
                color_views,
                depth_stencil_view,
                extent
            ]
        ),
        22 => layout!(
            22,
            FfiRenderPassDescAbi,
            [
                byte_size,
                request_id,
                label,
                target,
                color_formats,
                depth_format
            ]
        ),
        23 => layout!(
            23,
            FfiResourceBatch,
            [
                header,
                buffers,
                textures,
                texture_views,
                samplers,
                shaders,
                resource_layouts,
                resource_layout_bindings,
                resource_sets,
                resource_set_bindings,
                dynamic_offsets,
                pipeline_layouts,
                pipeline_layout_resource_layouts,
                graphics_pipelines,
                compute_pipelines,
                render_targets,
                render_target_color_views,
                render_passes,
                render_pass_color_formats,
                buffer_updates,
                texture_updates,
                destroys,
                negotiated_feature_bits
            ]
        ),
        24 => layout!(
            24,
            FfiPassAttachmentAbi,
            [
                byte_size,
                view,
                load_op,
                store_op,
                has_clear_color,
                clear_color
            ]
        ),
        25 => layout!(
            25,
            FfiBufferImageCopyAbi,
            [
                byte_size,
                buffer,
                buffer_offset,
                bytes_per_row,
                rows_per_image,
                texture,
                texture_mip,
                texture_layer,
                texture_origin,
                extent
            ]
        ),
        26 => layout!(
            26,
            FfiResourceBarrierAbi,
            [
                byte_size,
                resource,
                has_subresources,
                subresources,
                before,
                after,
                stage_bits,
                access_bits,
                src_queue,
                dst_queue
            ]
        ),
        27 => layout!(
            27,
            FfiCommandOpAbi,
            [
                byte_size,
                op_kind,
                primary,
                secondary,
                tertiary,
                set_index,
                slot,
                offset,
                size,
                count0,
                count1,
                count2,
                colors,
                depth_stencil,
                copy_region,
                barrier,
                inline_bytes,
                subresources
            ]
        ),
        28 => layout!(28, FfiCommandListAbi, [byte_size, label, operations]),
        29 => layout!(
            29,
            FfiSubmissionBatchAbi,
            [
                header,
                label,
                command_lists,
                operations,
                pass_attachments,
                copy_regions,
                barriers,
                negotiated_feature_bits
            ]
        ),
        30 => layout!(30, FfiCompletionQueryRequest, [header, submission_id]),
        31 => layout!(
            31,
            FfiCompletionResult,
            [
                header,
                status,
                error_domain,
                requested_submission_id,
                completed_submission_id,
                is_complete
            ]
        ),
        32 => layout!(
            32,
            FfiRetirementBatch,
            [header, completed_submission_id, handles]
        ),
        33 => layout!(
            33,
            FfiReadbackRequest,
            [header, submission_id, buffer, offset, size]
        ),
        34 => layout!(
            34,
            FfiReadbackResult,
            [
                header,
                status,
                error_domain,
                submission_id,
                required_bytes,
                written_bytes,
                metrics
            ]
        ),
        35 => layout!(
            35,
            FfiBorrowedOpenGlContextCreateRequest,
            [header, stable_window_id, tracy_enabled, reserved0, label]
        ),
        36 => layout!(
            36,
            FfiFrameSurfaceConfigRequest,
            [
                header,
                label,
                extent,
                color_format,
                present_mode,
                max_frames_in_flight
            ]
        ),
        37 => layout!(
            37,
            FfiFrameAcquireRequest,
            [header, correlation_id, expected_extent]
        ),
        38 => layout!(
            38,
            FfiFrameAcquireResult,
            [
                header,
                status,
                error_domain,
                frame_id,
                correlation_id,
                acquire_status,
                frame_target,
                frame_target_identity,
                extent,
                color_format,
                metrics
            ]
        ),
        39 => layout!(39, FfiFrameResizeRequest, [header, correlation_id, extent]),
        40 => layout!(
            40,
            FfiFrameResizeResult,
            [header, status, error_domain, resize_status, extent]
        ),
        41 => layout!(
            41,
            FfiFramePresentRequest,
            [header, frame_id, correlation_id, wait_submission_id]
        ),
        42 => layout!(
            42,
            FfiFramePresentResult,
            [
                header,
                status,
                error_domain,
                frame_id,
                correlation_id,
                present_status,
                completed_submission_id,
                frame_target_identity
            ]
        ),
        100 => layout!(
            100,
            FfiFrameCancelRequest,
            [header, frame_id, correlation_id]
        ),
        43 => layout!(43, FfiDestroyDescAbi, [byte_size, handle, expected_kind]),
        44 => layout!(
            44,
            FfiGuiSpriteRequest,
            [
                byte_size,
                stratum,
                sprite_id,
                selected_slot,
                progress_fraction,
                fill_direction,
                color_argb,
                x,
                y,
                width,
                height,
                gui_width,
                gui_height,
                sequence
            ]
        ),
        45 => layout!(
            45,
            FfiGuiFrameSubmitRequest,
            [
                header,
                generation,
                frame_id,
                frame_target,
                gui_width,
                gui_height,
                sprites,
                affine_quads,
                negotiated_feature_bits,
                mesh_batches,
                gui_projection_width,
                gui_projection_height,
                tiled_quads
            ]
        ),
        46 => layout!(
            46,
            FfiGuiFrameSubmitResult,
            [
                header,
                status,
                error_domain,
                submission_id,
                sprite_count,
                sprite_batch_count,
                cache_hits,
                cache_misses,
                resource_creates,
                command_lists,
                command_ops,
                metrics
            ]
        ),
        47 => layout!(47, FfiGuiAssetPayload, [byte_size, sprite_id, png_bytes]),
        48 => layout!(
            48,
            FfiGuiAssetUpdateRequest,
            [header, generation, assets, negotiated_feature_bits]
        ),
        90 => layout!(
            90,
            FfiGuiRawImageAssetPayload,
            [byte_size, format, asset_id, width, height, pixels, sampling_filter, sampling_address]
        ),
        92 => layout!(
            92,
            FfiGuiAffineQuadRequest,
            [
                byte_size,
                stratum,
                asset_id,
                x0,
                y0,
                x1,
                y1,
                x3,
                y3,
                z,
                u0,
                v0,
                u1,
                v1,
                color_argb,
                gui_width,
                gui_height,
                sequence,
                clip_mode,
                clip_left,
                clip_top,
                clip_width,
                clip_height,
                material_mode,
                item_raster_scale,
                item_raster_corners,
                item_raster_layers
            ]
        ),
        96 => layout!(
            96,
            FfiGuiMeshVertex,
            [position, atlas_uv, local_uv, color_argb, normal_packed, source_face, source_foil_type]
        ),
        97 => layout!(
            97,
            FfiGuiMeshBatchRequest,
            [
                byte_size,
                stratum,
                layer_index,
                material_mode,
                lighting_mode,
                asset_id,
                sequence,
                alpha_cutoff,
                reserved0,
                model_transform,
                gui_pose,
                left,
                top,
                right,
                bottom,
                gui_width,
                gui_height,
                render_width,
                render_height,
                guard_pixels,
                clip_mode,
                clip_left,
                clip_top,
                clip_width,
                clip_height,
                vertices,
                indices,
                item_foil_mode,
                item_foil_clock_millis,
                item_foil_speed,
                item_foil_strength,
                item_raster_scale,
                decal_foil_mode,
                decal_model_pose,
                decal_normal_pose,
                block_item_scale,
                block_model_bounds,
                block_item_layout,
                item_cache_identity,
                item_cache_mode
            ]
        ),
        91 => layout!(
            91,
            FfiGuiRawImageUpdateRequest,
            [header, generation, assets, negotiated_feature_bits]
        ),
        49 => layout!(
            49,
            FfiWindowedVulkanContextCreateRequest,
            [
                header,
                platform,
                tracy_enabled,
                stable_window_id,
                native_display,
                native_window,
                label,
                surface_label,
                extent,
                color_format,
                present_mode,
                max_frames_in_flight
            ]
        ),
        50 => layout!(
            50,
            FfiWorldLineSegmentRequest,
            [
                byte_size,
                stratum,
                style,
                depth_policy,
                color_argb,
                line_width,
                start_x,
                start_y,
                start_z,
                end_x,
                end_y,
                end_z,
                viewport_width,
                viewport_height
            ]
        ),
        51 => layout!(
            51,
            FfiWorldCrackQuadRequest,
            [
                byte_size,
                stratum,
                stage,
                depth_policy,
                blend_policy,
                cull_policy,
                color_argb,
                reserved0,
                p0_x,
                p0_y,
                p0_z,
                p1_x,
                p1_y,
                p1_z,
                p2_x,
                p2_y,
                p2_z,
                p3_x,
                p3_y,
                p3_z,
                viewport_width,
                viewport_height
            ]
        ),
        52 => layout!(
            52,
            FfiWorldBorderQuadRequest,
            [
                byte_size,
                stratum,
                texture_id,
                depth_policy,
                blend_policy,
                cull_policy,
                color_argb,
                reserved0,
                border_size,
                distance_to_border,
                scroll_u,
                scroll_v,
                uv_u,
                uv_v,
                uv_width,
                uv_height,
                p0_x,
                p0_y,
                p0_z,
                p1_x,
                p1_y,
                p1_z,
                p2_x,
                p2_y,
                p2_z,
                p3_x,
                p3_y,
                p3_z,
                viewport_width,
                viewport_height
            ]
        ),
        53 => layout!(
            53,
            FfiWholeFrameSubmitRequest,
            [
                header,
                generation,
                frame_id,
                correlation_id,
                frame_target,
                gui_width,
                gui_height,
                viewport_width,
                viewport_height,
                view_matrix,
                projection_matrix,
                world_background,
                world_segments,
                world_crack_quads,
                world_border_quads,
                world_material_quads,
                world_material_table,
                world_material_compact_quads,
                world_mesh_instances,
                gui_sprites,
                gui_affine_quads,
                negotiated_feature_bits,
                voxel_volume_frame,
                shader_environment_frame,
                world_lod_instances,
                world_lod_render_frame,
                world_feature_coverage,
                world_text_quads,
                gui_mesh_batches,
                world_first_person_frame,
                world_first_person_mesh_instances,
                gui_blur_before_stratum,
                gui_blur_radius,
                post_effect_id,
                gui_projection_width,
                gui_projection_height,
                gui_tiled_quads,
                engine_globals_present,
                engine_screen_width,
                engine_screen_height,
                engine_game_ticks,
                engine_partial_tick,
                engine_glint_alpha,
                engine_menu_blur_radius,
                world_particle_quads,
                world_experience_orbs
            ]
        ),
        89 => layout!(
            89,
            FfiWorldFeatureCoverage,
            [
                byte_size,
                model_submits,
                model_part_submits,
                block_model_submits,
                ordinary_block_submits,
                item_submits,
                custom_geometry_submits,
                shadow_submits,
                flame_submits,
                name_tag_submits,
                text_submits,
                hitbox_submits,
                leash_submits,
                particle_group_submits
            ]
        ),
        93 => layout!(
            93,
            FfiWorldTextQuadRequest,
            [
                byte_size,
                flags,
                depth_policy,
                packed_light,
                color_argb,
                reserved0,
                asset_id,
                atlas_generation,
                atlas_revision,
                distance_to_camera_sq,
                model_view_matrix,
                positions,
                uvs,
                block_entity_id
            ]
        ),
        94 => layout!(
            94,
            FfiWorldTextImageAssetPayload,
            [
                byte_size,
                format,
                width,
                height,
                asset_id,
                atlas_generation,
                atlas_revision,
                pixels
            ]
        ),
        95 => layout!(
            95,
            FfiWorldTextImageUpdateRequest,
            [header, generation, assets, negotiated_feature_bits]
        ),
        54 => layout!(
            54,
            FfiWholeFrameSubmitResult,
            [
                header,
                status,
                error_domain,
                submission_id,
                world_segment_count,
                world_vertex_count,
                world_batch_count,
                world_draw_count,
                world_crack_quad_count,
                world_crack_batch_count,
                world_crack_draw_count,
                world_border_quad_count,
                world_border_batch_count,
                world_border_draw_count,
                world_material_quad_count,
                world_material_batch_count,
                world_material_draw_count,
                world_mesh_instance_count,
                world_mesh_batch_count,
                world_mesh_draw_count,
                world_background_clear_count,
                world_background_diagnostic_fallback_count,
                world_background_sky_type,
                world_background_color_argb,
                depth_attachment_creates,
                depth_attachment_reuses,
                depth_attachment_retires,
                outline_cache_hits,
                outline_cache_misses,
                crack_cache_hits,
                crack_cache_misses,
                border_cache_hits,
                border_cache_misses,
                material_cache_hits,
                material_cache_misses,
                mesh_cache_hits,
                mesh_cache_misses,
                sprite_count,
                sprite_batch_count,
                gui_mesh_item_count,
                gui_mesh_batch_count,
                gui_mesh_draw_count,
                cache_hits,
                cache_misses,
                resource_creates,
                command_lists,
                command_ops,
                metrics,
                profile
            ]
        ),
        55 => layout!(
            55,
            FfiWorldBorderAssetUpdateRequest,
            [
                header,
                generation,
                texture_id,
                reserved0,
                png_bytes,
                negotiated_feature_bits
            ]
        ),
        56 => layout!(
            56,
            FfiWorldBackgroundRequest,
            [
                byte_size,
                enabled,
                sky_type,
                load_intent,
                store_intent,
                color_argb,
                viewport_width,
                viewport_height,
                sky_visible,
                sky_sunrise_or_sunset,
                sky_dark_disc,
                sky_reserved0,
                sky_sun_angle,
                sky_time_of_day,
                sky_rain_brightness,
                sky_star_brightness,
                sky_sunrise_and_sunset_color_argb,
                sky_moon_phase,
                sky_end_flash_intensity,
                sky_end_flash_x_angle,
                sky_end_flash_y_angle,
                sky_color_argb
            ]
        ),
        57 => layout!(57, FfiWorldCrackAssetPayload, [byte_size, stage, png_bytes]),
        58 => layout!(
            58,
            FfiWorldCrackAssetUpdateRequest,
            [header, generation, assets, negotiated_feature_bits]
        ),
        59 => layout!(
            59,
            FfiWorldMaterialQuadRequest,
            [
                byte_size,
                stratum,
                material_id,
                texture_id,
                material_mode,
                depth_policy,
                cull_policy,
                topology,
                color_argb,
                winding,
                p0_x,
                p0_y,
                p0_z,
                p1_x,
                p1_y,
                p1_z,
                p2_x,
                p2_y,
                p2_z,
                p3_x,
                p3_y,
                p3_z,
                uv0_u,
                uv0_v,
                uv1_u,
                uv1_v,
                uv2_u,
                uv2_v,
                uv3_u,
                uv3_v,
                viewport_width,
                viewport_height,
                source_program,
                source_color_argb,
                packed_light,
                source_uv_space,
                vertex0_color_argb,
                vertex1_color_argb,
                vertex2_color_argb,
                vertex3_color_argb,
                vertex0_packed_light,
                vertex1_packed_light,
                vertex2_packed_light,
                vertex3_packed_light,
                block_entity_id
            ]
        ),
        60 => layout!(
            60,
            FfiWorldMaterialAssetPayload,
            [byte_size, texture_id, png_bytes]
        ),
        61 => layout!(
            61,
            FfiWorldMaterialAssetUpdateRequest,
            [header, generation, assets, negotiated_feature_bits]
        ),
        62 => layout!(
            62,
            FfiWorldMaterialTableRecord,
            [
                byte_size,
                stratum,
                material_id,
                texture_id,
                material_mode,
                depth_policy,
                cull_policy,
                topology,
                winding,
                source_program
            ]
        ),
        63 => layout!(
            63,
            FfiWorldMaterialCompactQuadRequest,
            [
                byte_size,
                material_index,
                color_argb,
                source_uv_space,
                p0_x,
                p0_y,
                p0_z,
                p1_x,
                p1_y,
                p1_z,
                p2_x,
                p2_y,
                p2_z,
                p3_x,
                p3_y,
                p3_z,
                uv0_u,
                uv0_v,
                uv1_u,
                uv1_v,
                uv2_u,
                uv2_v,
                uv3_u,
                uv3_v,
                source_color_argb,
                packed_light,
                block_entity_id
            ]
        ),
        64 => layout!(
            64,
            FfiWorldMeshVertex,
            [
                byte_size,
                color_argb,
                normal_packed,
                light,
                x,
                y,
                z,
                u,
                v,
                atlas_u,
                atlas_v,
                shader_block_id,
                shader_material_type,
                terrain_material_bits,
                mid_block_packed
            ]
        ),
        65 => layout!(
            65,
            FfiWorldMeshSectionRecord,
            [
                byte_size,
                material_id,
                texture_id,
                material_mode,
                cull_policy,
                winding,
                index_offset,
                index_count
            ]
        ),
        66 => layout!(
            66,
            FfiWorldMeshAssetRecord,
            [
                byte_size,
                vertex_layout_version,
                index_type,
                reserved0,
                mesh_key,
                mesh_generation,
                vertices,
                index_bytes,
                sections,
                entity_identity_utf8
            ]
        ),
        67 => layout!(
            67,
            FfiWorldMeshTextureAssetPayload,
            [
                byte_size,
                texture_id,
                png_bytes,
                frame_width,
                frame_height,
                frame_count,
                frame_ticks,
                animation_flags,
                frame_row_size,
                interpolation_policy,
                reserved0,
                animation_frames,
                mip_png_bytes,
                sampling_filter,
                sampling_address,
                requested_mip_levels
            ]
        ),
        71 => layout!(
            71,
            FfiWorldMeshAnimationFrameRecord,
            [byte_size, frame_index, duration_ticks, reserved0]
        ),
        68 => layout!(
            68,
            FfiWorldMeshAssetUpdateRequest,
            [
                header,
                generation,
                meshes,
                textures,
                sorted_indices,
                negotiated_feature_bits,
                retirements,
                experience_orbs
            ]
        ),
        69 => layout!(
            69,
            FfiWorldMeshInstanceRecord,
            [
                byte_size,
                stratum,
                mesh_section_index,
                depth_policy,
                cull_policy,
                winding,
                color_argb,
                viewport_width,
                viewport_height,
                mesh_key,
                mesh_generation,
                entity_id,
                entity_color_argb,
                transform,
                outline_color_argb,
                flags,
                block_entity_id,
                terrain_placement_mode,
                terrain_origin,
                terrain_camera,
                item_foil_mode,
                item_foil_clock_millis,
                item_foil_speed,
                item_foil_strength
            ]
        ),
        70 => layout!(
            70,
            FfiWorldMeshSortedIndexRecord,
            [
                byte_size,
                index_type,
                reserved0,
                mesh_key,
                mesh_generation,
                index_generation,
                index_bytes
            ]
        ),
        99 => layout!(
            99,
            FfiWorldMeshAssetRetirementRecord,
            [byte_size, reserved0, mesh_key, mesh_generation]
        ),
        72 => layout!(
            72,
            FfiWorldVoxelVolumeFrame,
            [
                byte_size,
                enabled,
                reserved0,
                reserved1,
                world_generation,
                resource_generation,
                camera_x,
                camera_y,
                camera_z,
                reserved2
            ]
        ),
        74 => layout!(
            74,
            FfiShaderPackSourceFile,
            [byte_size, reserved0, path_utf8, contents_utf8]
        ),
        75 => layout!(
            75,
            FfiShaderPackSourceUpdateRequest,
            [header, generation, pack_name_utf8, files]
        ),
        76 => layout!(
            76,
            FfiShaderPackAssetFile,
            [byte_size, reserved0, path_utf8, contents]
        ),
        77 => layout!(
            77,
            FfiShaderPackAssetUpdateRequest,
            [header, generation, pack_name_utf8, files]
        ),
        78 => layout!(
            78,
            FfiWorldLodVertex,
            [
                byte_size,
                local_x,
                local_y,
                local_z,
                packed_light_and_micro_offset,
                color_rgba,
                material_id,
                normal_index
            ]
        ),
        79 => layout!(
            79,
            FfiWorldLodSegmentRecord,
            [byte_size, layer, vertices, packed_vertices]
        ),
        80 => layout!(
            80,
            FfiWorldLodColumnAssetRecord,
            [
                byte_size,
                vertex_layout_version,
                origin_x,
                origin_y,
                origin_z,
                reserved0,
                column_key,
                column_generation,
                segments
            ]
        ),
        81 => layout!(
            81,
            FfiWorldLodColumnRetirementRecord,
            [byte_size, reserved0, column_key, column_generation]
        ),
        82 => layout!(
            82,
            FfiWorldLodAssetUpdateRequest,
            [
                header,
                generation,
                assets,
                retirements,
                negotiated_feature_bits,
                material_provenance
            ]
        ),
        83 => layout!(
            83,
            FfiWorldLodColumnInstanceRecord,
            [
                byte_size,
                layer,
                segment_index,
                order,
                column_key,
                column_generation
            ]
        ),
        84 => layout!(
            84,
            FfiWorldLodRenderFrame,
            [
                byte_size,
                enabled,
                flags,
                world_y_offset,
                combined_matrix,
                model_view_matrix,
                projection_matrix,
                projection_inverse_matrix,
                clip_distance,
                micro_offset,
                noise_intensity,
                earth_radius,
                noise_steps,
                noise_dropoff,
                reserved0,
                camera_world_x,
                camera_world_y,
                camera_world_z
            ]
        ),
        85 => layout!(
            85,
            FfiWorldLodMaterialIdentityRecord,
            [
                byte_size,
                reserved0,
                block_state_identity_utf8,
                biome_identity_utf8
            ]
        ),
        86 => layout!(
            86,
            FfiWorldLodSegmentMaterialProvenanceRecord,
            [
                byte_size,
                layer,
                segment_index,
                reserved0,
                quad_material_ids,
                quad_variant_states,
                quad_variant_positions
            ]
        ),
        87 => layout!(
            87,
            FfiWorldLodColumnMaterialProvenanceRecord,
            [
                byte_size,
                reserved0,
                column_key,
                column_generation,
                identities,
                segments,
                face_materials
            ]
        ),
        88 => layout!(
            88,
            FfiWorldLodFaceMaterialRecord,
            [
                byte_size,
                material_id,
                face,
                face_layer,
                atlas_identity_utf8,
                sprite_identity_utf8,
                u0,
                v0,
                u1,
                v1,
                uv_corner_order,
                variant_position
            ]
        ),
        73 => layout!(
            73,
            FfiWorldShaderEnvironmentFrame,
            [
                byte_size,
                enabled,
                frame_counter,
                world_day,
                world_generation,
                world_time,
                frame_time_seconds,
                frame_time_counter,
                time_of_day,
                rain_strength,
                thunder_strength,
                sky_darken,
                moon_phase,
                eye_submersion,
                screen_brightness,
                far_plane,
                relative_eye_x,
                relative_eye_y,
                relative_eye_z,
                sky_color_r,
                sky_color_g,
                sky_color_b,
                darkness_light_factor,
                night_vision,
                fog_color_r,
                fog_color_g,
                fog_color_b,
                biome_precipitation,
                biome_resource_location_utf8,
                main_hand_item_model_resource_location_utf8,
                off_hand_item_model_resource_location_utf8,
                main_hand_item_light_emission,
                off_hand_item_light_emission,
                lightmap_enabled,
                lightmap_reserved,
                lightmap_generation,
                lightmap_ambient_light_factor,
                lightmap_sky_factor,
                lightmap_block_factor,
                lightmap_night_vision_factor,
                lightmap_darkness_scale,
                lightmap_darken_world_factor,
                lightmap_brightness_factor,
                lightmap_sky_light_r,
                lightmap_sky_light_g,
                lightmap_sky_light_b,
                lightmap_ambient_r,
                lightmap_ambient_g,
                lightmap_ambient_b,
                blindness,
                darkness_factor,
                eye_brightness_block,
                eye_brightness_sky,
                fog_parameter_color_r,
                fog_parameter_color_g,
                fog_parameter_color_b,
                fog_parameter_color_a,
                fog_environmental_start,
                fog_environmental_end,
                fog_render_distance_start,
                fog_render_distance_end,
                distant_horizons_render_distance,
                fog_sky_end,
                fog_clouds_end
            ]
        ),
        98 => layout!(
            98,
            FfiWorldFirstPersonFrame,
            [
                byte_size,
                enabled,
                clear_depth_before,
                main_hand_instance_count,
                projection_matrix,
                model_view_matrix
            ]
        ),
        101 => layout!(101, FfiGuiTiledQuadRequest,
            [byte_size, stratum, asset_id, bounds, tile_extent, uv, pose, z,
             color_argb, sequence, clip_mode, clip]),
        102 => layout!(102, FfiSpriteAnimationMip,
            [byte_size, width, height, reserved0, rgba]),
        103 => layout!(103, FfiSpriteAnimationSource,
            [byte_size, sprite_id, atlas_x, atlas_y, frame_width, frame_height,
             interpolate, reserved0, frames, mips]),
        104 => layout!(104, FfiAtlasAnimationAssetUpdate,
            [header, texture_id, reserved0, generation, initial_tick, sprites]),
        105 => layout!(105, FfiGuiAtlasReference,
            [byte_size, texture_id, asset_id, atlas_generation, atlas_width, atlas_height,
             x, y, width, height]),
        106 => layout!(106, FfiGuiAtlasReferenceUpdate,
            [header, revision, references, negotiated_feature_bits]),
        107 => layout!(107, FfiGuiItemRasterLayer,
            [byte_size, material_mode, asset_id, color_argb, corners, uv, model_transform]),
        108 => layout!(108, FfiWorldParticleQuadRequest,
            [byte_size, texture_id, surface_kind, material_index, center, rotation,
             size, uv_bounds, color_argb, packed_light]),
        109 => layout!(109, FfiWorldExperienceOrbAssetRecord,
            [byte_size, icon, mesh_key, mesh_generation, red, blue, packed_light, reserved0]),
        110 => layout!(110, FfiWorldExperienceOrbInstanceRecord,
            [byte_size, mesh_index, mesh_key, mesh_generation, entity_transform,
             camera_orientation, entity_id, reserved0]),
        _ => {
            return Err(GalError::ffi(
                StatusCode::UnknownEnum,
                format!("unknown ABI struct id {struct_id}"),
            ));
        }
    })
}
#[no_mangle]
pub unsafe extern "C" fn mattmc_vulkanic_gal_abi_struct_layout(
    struct_id: u32,
    out: *mut FfiStructLayout,
) -> i32 {
    match layout_for_struct(struct_id).and_then(|layout| write_out(out, layout, "layout result")) {
        Ok(()) => StatusCode::Ok as i32,
        Err(error) => error.code as i32,
    }
}
