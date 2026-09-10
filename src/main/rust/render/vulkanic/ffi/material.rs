use super::*;
use crate::render::vulkanic::world_primitive_frontend::{
    material as world_material_semantics, WorldMeshAnimationFrame, WORLD_MAX_MESH_ANIMATION_FRAMES,
    WORLD_MAX_MESH_INDEX_BYTES, WORLD_MAX_MESH_SECTIONS, WORLD_MAX_MESH_TEXTURE_ASSETS,
    WORLD_MAX_MESH_TEXTURE_DECODED_BYTES, WORLD_MAX_MESH_VERTICES, WORLD_MESH_ASSET_RESIDENCY,
    WORLD_MESH_TEXTURE_RESIDENCY,
};

const MAX_WORLD_MESH_TEXTURE_PNG_BYTES_TOTAL: usize = WORLD_MAX_MESH_TEXTURE_DECODED_BYTES;
const MAX_WORLD_MATERIAL_ASSET_COUNT: usize = WORLD_MAX_MESH_TEXTURE_ASSETS;

pub(crate) unsafe fn decode_world_material_asset_update(
    request: *const FfiWorldMaterialAssetUpdateRequest,
    capabilities: BackendCapabilities,
) -> GalResult<(u64, Vec<WorldMaterialAssetPayload>)> {
    let request = read_struct(request, "world material asset update request")?;
    validate_header::<FfiWorldMaterialAssetUpdateRequest>(request.header)?;
    reject_unknown_feature_bits(request.negotiated_feature_bits)?;
    let supported = capability_feature_bits(capabilities);
    if request.negotiated_feature_bits & !supported != 0 {
        return Err(GalError::unsupported_feature(format!(
            "requested unsupported world material asset feature bits 0x{:x}",
            request.negotiated_feature_bits & !supported
        )));
    }
    if request.generation == 0 {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "world material asset generation must be non-zero",
        ));
    }
    let raw_assets = read_limited_slice(request.assets, true, "world material asset payloads")?;
    if raw_assets.len() > MAX_WORLD_MATERIAL_ASSET_COUNT {
        return Err(GalError::ffi(
            StatusCode::LengthOverflow,
            format!(
                "world material asset payload count {} exceeds bounded limit {MAX_WORLD_MATERIAL_ASSET_COUNT}",
                raw_assets.len()
            ),
        ));
    }
    let mut seen = BTreeMap::new();
    let mut assets = Vec::with_capacity(raw_assets.len());
    let mut png_bytes_total = 0usize;
    for asset in raw_assets {
        validate_item_size::<FfiWorldMaterialAssetPayload>(
            asset.byte_size,
            "world material asset payload",
        )?;
        let texture_id = world_material_semantics::canonical_texture_id(asset.texture_id)
            .ok_or_else(|| {
                GalError::ffi(
                    StatusCode::UnknownEnum,
                    format!(
                        "unknown world material asset texture id {}",
                        asset.texture_id
                    ),
                )
            })?;
        if seen.insert(texture_id, ()).is_some() {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                format!(
                    "duplicate world material asset payload for texture {}",
                    texture_id
                ),
            ));
        }
        png_bytes_total = png_bytes_total
            .checked_add(asset.png_bytes.len as usize)
            .ok_or_else(|| {
                GalError::ffi(
                    StatusCode::LengthOverflow,
                    "world material asset PNG byte count overflow",
                )
            })?;
        if png_bytes_total > MAX_WORLD_MESH_TEXTURE_PNG_BYTES_TOTAL {
            return Err(GalError::ffi(
                StatusCode::LengthOverflow,
                format!(
                    "world material asset PNG bytes {} exceed bounded total {MAX_WORLD_MESH_TEXTURE_PNG_BYTES_TOTAL}",
                    png_bytes_total
                ),
            ));
        }
        let png_bytes = read_bounded_bytes(
            asset.png_bytes,
            true,
            FFI_MAX_WORLD_MATERIAL_ASSET_BYTES,
            "world material asset PNG bytes",
        )?;
        assets.push(WorldMaterialAssetPayload {
            texture_id,
            png_bytes,
        });
    }
    Ok((request.generation, assets))
}

unsafe fn read_world_mesh_asset_update_request(
    request: *const FfiWorldMeshAssetUpdateRequest,
) -> GalResult<FfiWorldMeshAssetUpdateRequest> {
    // Read only the common header until the caller's full layout is proven.
    // In particular, old ABI requests do not contain the appended orb slice.
    let header = read_struct(request.cast::<FfiHeader>(), "world mesh asset update header")?;
    validate_header::<FfiWorldMeshAssetUpdateRequest>(header)?;
    read_struct(request, "world mesh asset update request")
}

pub(crate) unsafe fn decode_world_mesh_asset_update(
    request: *const FfiWorldMeshAssetUpdateRequest,
    capabilities: BackendCapabilities,
) -> GalResult<(
    u64,
    Vec<WorldMeshAsset>,
    Vec<WorldMeshTextureAssetPayload>,
    Vec<WorldMeshSortedIndexUpdate>,
    Vec<(u64, u64)>,
)> {
    let request = read_world_mesh_asset_update_request(request)?;
    if request.header.version != FFI_ABI_VERSION {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "world mesh asset updates require ABI v3 semantic vertex records",
        ));
    }
    reject_unknown_feature_bits(request.negotiated_feature_bits)?;
    let supported = capability_feature_bits(capabilities);
    if request.negotiated_feature_bits & !supported != 0 {
        return Err(GalError::unsupported_feature(format!(
            "requested unsupported world mesh asset feature bits 0x{:x}",
            request.negotiated_feature_bits & !supported
        )));
    }
    if request.generation == 0 {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "world mesh asset generation must be non-zero",
        ));
    }
    let raw_textures = read_limited_slice(request.textures, true, "world mesh texture payloads")?;
    if raw_textures.len() > WORLD_MESH_TEXTURE_RESIDENCY {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!(
                "world mesh texture payload count {} exceeds bounded limit {WORLD_MESH_TEXTURE_RESIDENCY}",
                raw_textures.len()
            ),
        ));
    }
    let mut seen_textures = BTreeMap::new();
    let mut textures = Vec::with_capacity(raw_textures.len());
    let mut texture_png_bytes_total = 0usize;
    for texture in raw_textures {
        validate_item_size::<FfiWorldMeshTextureAssetPayload>(
            texture.byte_size,
            "world mesh texture payload",
        )?;
        if texture.texture_id == 0 {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "world mesh texture id must be non-zero",
            ));
        }
        if seen_textures.insert(texture.texture_id, ()).is_some() {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                format!("duplicate world mesh texture {}", texture.texture_id),
            ));
        }
        let raw_mip_pngs = read_limited_slice(texture.mip_png_bytes, true, "world mesh texture mip PNGs")?;
        let mip_png_byte_count = raw_mip_pngs.iter().try_fold(0usize, |total, mip| {
            total.checked_add(mip.len as usize).ok_or_else(|| {
                GalError::ffi(StatusCode::LengthOverflow, "world mesh texture mip PNG byte count overflow")
            })
        })?;
        texture_png_bytes_total = texture_png_bytes_total
            .checked_add(texture.png_bytes.len as usize)
            .and_then(|total| total.checked_add(mip_png_byte_count))
            .ok_or_else(|| {
                GalError::ffi(
                    StatusCode::LengthOverflow,
                    "world mesh texture PNG byte count overflow",
                )
            })?;
        if texture_png_bytes_total > MAX_WORLD_MESH_TEXTURE_PNG_BYTES_TOTAL {
            return Err(GalError::ffi(
                StatusCode::LengthOverflow,
                format!(
                    "world mesh texture PNG bytes {} exceed bounded total {MAX_WORLD_MESH_TEXTURE_PNG_BYTES_TOTAL}",
                    texture_png_bytes_total
                ),
            ));
        }
        let png_bytes = read_bounded_bytes(
            texture.png_bytes,
            true,
            FFI_MAX_WORLD_MESH_TEXTURE_ASSET_BYTES,
            "world mesh texture PNG bytes",
        )?;
        let mip_png_bytes = raw_mip_pngs
            .iter()
            .enumerate()
            .map(|(mip, bytes)| read_bounded_bytes(
                *bytes,
                true,
                FFI_MAX_WORLD_MESH_TEXTURE_ASSET_BYTES,
                &format!("world mesh texture mip {} PNG bytes", mip + 1),
            ))
            .collect::<GalResult<Vec<_>>>()?;
        let raw_frames = read_limited_slice(
            texture.animation_frames,
            true,
            "world mesh animation frames",
        )?;
        if raw_frames.len() > WORLD_MAX_MESH_ANIMATION_FRAMES {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                format!(
                    "world mesh animation frame table length {} exceeds {}",
                    raw_frames.len(),
                    WORLD_MAX_MESH_ANIMATION_FRAMES
                ),
            ));
        }
        let mut animation_frames = Vec::with_capacity(raw_frames.len());
        for frame in raw_frames {
            validate_item_size::<FfiWorldMeshAnimationFrameRecord>(
                frame.byte_size,
                "world mesh animation frame",
            )?;
            animation_frames.push(WorldMeshAnimationFrame {
                frame_index: frame.frame_index,
                duration_ticks: frame.duration_ticks,
            });
        }
        if texture.requested_mip_levels > 32 {
            return Err(GalError::invalid_argument("invalid explicit texture mip count"));
        }
        textures.push(WorldMeshTextureAssetPayload {
            requested_mip_levels: texture.requested_mip_levels,
            texture_id: texture.texture_id,
            png_bytes,
            mip_png_bytes,
            frame_width: texture.frame_width,
            frame_height: texture.frame_height,
            frame_count: texture.frame_count,
            frame_ticks: texture.frame_ticks,
            animation_flags: texture.animation_flags,
            frame_row_size: texture.frame_row_size,
            interpolation_policy: texture.interpolation_policy,
            animation_frames,
            coordinate_origin: texture.reserved0,
            sampling: super::super::texture_sampling::TextureSampling::decode(
                texture.sampling_filter, texture.sampling_address)?,
        });
    }
    let raw_meshes = read_limited_slice(request.meshes, true, "world mesh assets")?;
    if raw_meshes.len() > WORLD_MESH_ASSET_RESIDENCY {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!(
                "world mesh asset payload count {} exceeds bounded limit {WORLD_MESH_ASSET_RESIDENCY}",
                raw_meshes.len()
            ),
        ));
    }
    // Check the combined residency before dereferencing the semantic array.
    let total_meshes = (raw_meshes.len() as u64).checked_add(request.experience_orbs.count)
        .filter(|&count| count <= WORLD_MESH_ASSET_RESIDENCY as u64)
        .ok_or_else(|| GalError::invalid_argument("combined world mesh/orb asset count exceeds residency"))?;
    let raw_orbs = read_limited_slice(request.experience_orbs, true, "experience orb assets")?;
    let mut seen_meshes = BTreeMap::new();
    let mut meshes = Vec::with_capacity(total_meshes as usize);
    for mesh in raw_meshes {
        validate_item_size::<FfiWorldMeshAssetRecord>(mesh.byte_size, "world mesh asset")?;
        if mesh.mesh_key == 0 || mesh.mesh_generation == 0 {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "world mesh asset key and generation must be non-zero",
            ));
        }
        if seen_meshes.insert(mesh.mesh_key, ()).is_some() {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                format!("duplicate world mesh asset {}", mesh.mesh_key),
            ));
        }
        let index_type = ffi_index_type(mesh.index_type)?;
        let raw_vertices = read_limited_slice(mesh.vertices, false, "world mesh vertices")?;
        if raw_vertices.len() > WORLD_MAX_MESH_VERTICES {
            return Err(GalError::ffi(
                StatusCode::LengthOverflow,
                format!(
                    "world mesh vertex count {} exceeds bounded limit {WORLD_MAX_MESH_VERTICES}",
                    raw_vertices.len()
                ),
            ));
        }
        let mut vertices = Vec::with_capacity(raw_vertices.len());
        for vertex in raw_vertices {
            validate_item_size::<FfiWorldMeshVertex>(vertex.byte_size, "world mesh vertex")?;
            vertices.push(WorldMeshVertex {
                position: [vertex.x, vertex.y, vertex.z],
                uv: [vertex.u, vertex.v],
                shader_atlas_uv: [vertex.atlas_u, vertex.atlas_v],
                shader_block_id: vertex.shader_block_id,
                shader_material_type: vertex.shader_material_type,
                terrain_material_bits: vertex.terrain_material_bits,
                mid_block_packed: vertex.mid_block_packed,
                color_argb: vertex.color_argb,
                normal_packed: vertex.normal_packed,
                light: vertex.light,
            });
        }
        let index_bytes = read_bounded_bytes(
            mesh.index_bytes,
            false,
            WORLD_MAX_MESH_INDEX_BYTES,
            "world mesh index bytes",
        )?;
        let raw_sections = read_limited_slice(mesh.sections, false, "world mesh sections")?;
        if raw_sections.len() > WORLD_MAX_MESH_SECTIONS {
            return Err(GalError::ffi(
                StatusCode::LengthOverflow,
                format!(
                    "world mesh section count {} exceeds bounded limit {WORLD_MAX_MESH_SECTIONS}",
                    raw_sections.len()
                ),
            ));
        }
        let mut sections = Vec::with_capacity(raw_sections.len());
        for section in raw_sections {
            validate_item_size::<FfiWorldMeshSectionRecord>(
                section.byte_size,
                "world mesh section",
            )?;
            let material_id = world_material_semantics::canonical_material_id(section.material_id)
                .ok_or_else(|| {
                    GalError::ffi(
                        StatusCode::UnknownEnum,
                        format!("unknown world mesh material id {}", section.material_id),
                    )
                })?;
            sections.push(WorldMeshSection {
                material_id,
                texture_id: section.texture_id,
                material_mode: section.material_mode,
                cull_policy: section.cull_policy,
                winding: section.winding,
                index_offset: section.index_offset,
                index_count: section.index_count,
            });
        }
        let entity_identity = read_label(
            mesh.entity_identity_utf8,
            "world mesh asset entity identity",
        )?;
        if !entity_identity.is_empty()
            && !super::world::is_canonical_resource_location(&entity_identity)
        {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "world mesh asset entity identity must be canonical namespace:path text",
            ));
        }
        meshes.push(WorldMeshAsset {
            mesh_key: mesh.mesh_key,
            mesh_generation: mesh.mesh_generation,
            vertex_layout_version: mesh.vertex_layout_version,
            index_type,
            vertices,
            index_bytes,
            sections,
            entity_identity,
        });
    }
    for orb in raw_orbs {
        validate_item_size::<FfiWorldExperienceOrbAssetRecord>(orb.byte_size, "experience orb asset")?;
        if orb.reserved0 != 0 || orb.red > 255 || orb.blue > 255 {
            return Err(GalError::invalid_argument("invalid experience orb appearance channels or reserved bits"));
        }
        if seen_meshes.insert(orb.mesh_key, ()).is_some() {
            return Err(GalError::invalid_argument("duplicate world mesh/orb asset identity"));
        }
        meshes.push(crate::render::vulkanic::world_primitive_frontend::experience_orb::ExperienceOrbAppearance {
            icon: orb.icon, red: orb.red as u8, blue: orb.blue as u8, packed_light: orb.packed_light,
        }.mesh(orb.mesh_key, orb.mesh_generation)?);
    }
    let raw_sorted_indices = read_limited_slice(
        request.sorted_indices,
        true,
        "world mesh sorted index updates",
    )?;
    if raw_sorted_indices.len() > WORLD_MESH_ASSET_RESIDENCY {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!(
                "world mesh sorted index update count {} exceeds bounded limit {WORLD_MESH_ASSET_RESIDENCY}",
                raw_sorted_indices.len()
            ),
        ));
    }
    let mut sorted_indices = Vec::with_capacity(raw_sorted_indices.len());
    for update in raw_sorted_indices {
        validate_item_size::<FfiWorldMeshSortedIndexRecord>(
            update.byte_size,
            "world mesh sorted index update",
        )?;
        if update.mesh_key == 0 || update.mesh_generation == 0 || update.index_generation == 0 {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "world mesh sorted index update key and generations must be non-zero",
            ));
        }
        sorted_indices.push(WorldMeshSortedIndexUpdate {
            mesh_key: update.mesh_key,
            mesh_generation: update.mesh_generation,
            index_generation: update.index_generation,
            index_type: ffi_index_type(update.index_type)?,
            index_bytes: read_bounded_bytes(
                update.index_bytes,
                false,
                FFI_MAX_WORLD_MESH_INDEX_BYTES,
                "world mesh sorted index bytes",
            )?,
        });
    }
    let raw_retirements =
        read_limited_slice(request.retirements, true, "world mesh asset retirements")?;
    if raw_retirements.len() > WORLD_MESH_ASSET_RESIDENCY {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!(
                "world mesh asset retirement count {} exceeds bounded limit {WORLD_MESH_ASSET_RESIDENCY}",
                raw_retirements.len()
            ),
        ));
    }
    let mut seen_retirements = BTreeMap::new();
    let mut retirements = Vec::with_capacity(raw_retirements.len());
    for retirement in raw_retirements {
        validate_item_size::<FfiWorldMeshAssetRetirementRecord>(
            retirement.byte_size,
            "world mesh asset retirement",
        )?;
        if retirement.mesh_key == 0 || retirement.mesh_generation == 0 {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "world mesh retirement key and generation must be non-zero",
            ));
        }
        if seen_retirements.insert(retirement.mesh_key, ()).is_some() {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                format!("duplicate world mesh retirement {}", retirement.mesh_key),
            ));
        }
        retirements.push((retirement.mesh_key, retirement.mesh_generation));
    }
    Ok((
        request.generation,
        meshes,
        textures,
        sorted_indices,
        retirements,
    ))
}

#[no_mangle]
pub unsafe extern "C" fn mattmc_vulkanic_gal_world_material_update_assets(
    context_id: u64,
    request: *const FfiWorldMaterialAssetUpdateRequest,
    status_out: *mut FfiStatusResult,
) -> i32 {
    with_registry_mut(|registry| {
        let Some(context) = registry.contexts.get_mut(&context_id) else {
            let error = GalError::ffi(
                StatusCode::StaleHandle,
                format!("unknown context id {context_id}"),
            );
            write_status_out(status_out, status_result_from_error(&error));
            return error.code as i32;
        };
        let input_bytes = if request.is_null() {
            0
        } else {
            input_bytes_for_world_material_asset_update(&*request)
        };
        context.ffi_calls += 1;
        context.ffi_input_bytes = context.ffi_input_bytes.saturating_add(input_bytes);
        context.ffi_output_bytes = context
            .ffi_output_bytes
            .saturating_add(size_of::<FfiStatusResult>() as u64);
        let result = decode_world_material_asset_update(request, context.gal.capabilities())
            .and_then(|(generation, payloads)| {
                context
                    .world_primitive_frontend
                    .apply_world_material_asset_update(&mut context.gal, generation, payloads)
            });
        match result {
            Ok(()) => {
                write_status_out(status_out, status_ok(context));
                StatusCode::Ok as i32
            }
            Err(error) => {
                set_last_error(context, &error);
                write_status_out(status_out, status_error(Some(context), &error));
                error.code as i32
            }
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn mattmc_vulkanic_gal_world_mesh_update_assets(
    context_id: u64,
    request: *const FfiWorldMeshAssetUpdateRequest,
    status_out: *mut FfiStatusResult,
) -> i32 {
    with_registry_mut(|registry| {
        let Some(context) = registry.contexts.get_mut(&context_id) else {
            let error = GalError::ffi(
                StatusCode::StaleHandle,
                format!("unknown context id {context_id}"),
            );
            write_status_out(status_out, status_result_from_error(&error));
            return error.code as i32;
        };
        let checked_request = read_world_mesh_asset_update_request(request);
        let input_bytes = checked_request.as_ref()
            .map(input_bytes_for_world_mesh_asset_update).unwrap_or(0);
        context.ffi_calls += 1;
        context.ffi_input_bytes = context.ffi_input_bytes.saturating_add(input_bytes);
        context.ffi_output_bytes = context
            .ffi_output_bytes
            .saturating_add(size_of::<FfiStatusResult>() as u64);
        let result = checked_request.and_then(|request|
            decode_world_mesh_asset_update(&request, context.gal.capabilities())).and_then(
            |(generation, meshes, textures, sorted_indices, retirements)| {
                context.gui_frontend.invalidate_atlas_texture_views(
                    &mut context.gal, textures.iter().map(|texture| texture.texture_id))?;
                context
                    .world_primitive_frontend
                    .apply_world_mesh_asset_update_with_sorted_and_retirements(
                        &mut context.gal,
                        generation,
                        meshes,
                        textures,
                        sorted_indices,
                        retirements,
                    )
            },
        );
        match result {
            Ok(()) => {
                write_status_out(status_out, status_ok(context));
                StatusCode::Ok as i32
            }
            Err(error) => {
                set_last_error(context, &error);
                write_status_out(status_out, status_error(Some(context), &error));
                error.code as i32
            }
        }
    })
}
