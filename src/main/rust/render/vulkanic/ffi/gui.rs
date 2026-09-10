use super::*;
use crate::render::vulkanic::gui_frontend::{
    GUI_MAX_RAW_IMAGE_BYTES_TOTAL, GUI_MAX_RAW_IMAGE_PIXELS, GUI_MAX_VIEWPORT_AXIS,
};
use crate::render::vulkanic::gui_mesh_frontend::GUI_MESH_MAX_FRAME_PAYLOAD_BYTES;

const GUI_MAX_AFFINE_QUADS: usize = super::super::gui_frontend::GUI_MAX_EXPANDED_AFFINE_QUADS;

pub(crate) unsafe fn decode_gui_atlas_reference_update(
    request: *const FfiGuiAtlasReferenceUpdate, capabilities: BackendCapabilities,
) -> GalResult<(u64, Vec<super::super::gui_atlas_reference::GuiAtlasReference>)> {
    use super::super::gui_atlas_reference::{GuiAtlasReference, AcceptedAtlasIncarnation, MAX_GUI_ATLAS_REFERENCES};
    let request = read_struct(request, "GUI atlas reference update")?;
    validate_header::<FfiGuiAtlasReferenceUpdate>(request.header)?;
    reject_unknown_feature_bits(request.negotiated_feature_bits)?;
    if request.negotiated_feature_bits & !capability_feature_bits(capabilities) != 0 {
        return Err(GalError::unsupported_feature("unsupported GUI atlas reference feature bits"));
    }
    if request.revision == 0 || request.references.count > MAX_GUI_ATLAS_REFERENCES as u64 {
        return Err(GalError::invalid_argument("invalid GUI atlas reference revision or count"));
    }
    let items = read_limited_slice(request.references, true, "GUI atlas references")?;
    let mut ids = std::collections::BTreeSet::new();
    let mut owned = Vec::with_capacity(items.len());
    for item in items {
        validate_item_size::<FfiGuiAtlasReference>(item.byte_size, "GUI atlas reference")?;
        let reference = GuiAtlasReference { asset_id: item.asset_id,
            atlas: AcceptedAtlasIncarnation { texture_id: item.texture_id,
                generation: item.atlas_generation, width: item.atlas_width, height: item.atlas_height },
            x: item.x, y: item.y, width: item.width, height: item.height };
        reference.validate()?;
        if !ids.insert(reference.asset_id) {
            return Err(GalError::invalid_argument("duplicate GUI atlas reference identity"));
        }
        owned.push(reference);
    }
    Ok((request.revision, owned))
}

/// Declaration transport only. No rendering capability is admitted here.
#[no_mangle]
pub unsafe extern "C" fn mattmc_vulkanic_gal_gui_update_atlas_references(
    context_id: u64, request: *const FfiGuiAtlasReferenceUpdate, status_out: *mut FfiStatusResult,
) -> i32 {
    with_registry_mut(|registry| {
        let Some(context) = registry.contexts.get_mut(&context_id) else {
            let error = GalError::ffi(StatusCode::StaleHandle, format!("unknown context id {context_id}"));
            write_status_out(status_out, status_result_from_error(&error));
            return error.code as i32;
        };
        context.ffi_calls += 1;
        context.ffi_output_bytes = context.ffi_output_bytes.saturating_add(size_of::<FfiStatusResult>() as u64);
        let result = decode_gui_atlas_reference_update(request, context.gal.capabilities())
            .and_then(|(revision, references)| {
                context.ffi_input_bytes = context.ffi_input_bytes.saturating_add(
                    size_of::<FfiGuiAtlasReferenceUpdate>() as u64
                        + references.len() as u64 * size_of::<FfiGuiAtlasReference>() as u64);
                context.gui_frontend.stage_owned_atlas_references(&mut context.gal,
                    &context.world_primitive_frontend, revision, &references)
            });
        match result {
            Ok(()) => { write_status_out(status_out, status_ok(context)); StatusCode::Ok as i32 },
            Err(error) => {
                set_last_error(context, &error);
                write_status_out(status_out, status_error(Some(context), &error));
                error.code as i32
            },
        }
    })
}

/// Copies ABI v29 tiled semantics with aggregate preflight before geometry
/// expansion. Frame integration checks parent sequences across GUI families
/// before partitioning at blur boundaries. Producer admission remains private.
pub(crate) unsafe fn decode_gui_tiled_quads(
    raw: FfiSlice<FfiGuiTiledQuadRequest>, gui_extent: [u32; 2],
    projection_extent: [f32; 2], ordinary_affine_count: usize,
) -> GalResult<Vec<super::super::gui_frontend::GuiTiledQuadRequest>> {
    use super::super::gui_frontend::{GuiTiledQuadRequest, preflight_tiled_affine_count};
    if raw.count > GUI_MAX_AFFINE_QUADS as u64 {
        return Err(GalError::invalid_argument("tiled GUI request count exceeds bounded limit"));
    }
    let items = read_slice(raw, true, "GUI tiled quad requests")?;
    let mut count = preflight_tiled_affine_count(&[], ordinary_affine_count)?;
    let mut owned = Vec::with_capacity(items.len());
    let mut sequences = std::collections::BTreeSet::new();
    for item in items {
        validate_item_size::<FfiGuiTiledQuadRequest>(item.byte_size, "GUI tiled quad")?;
        let clip = match item.clip_mode {
            0 if item.clip == [0; 4] => None,
            1 => Some(item.clip),
            _ => return Err(GalError::invalid_argument("invalid tiled GUI clip mode")),
        };
        let request = GuiTiledQuadRequest {
            geometry: super::super::gui_tiling::GuiTileGeometry {
                bounds: item.bounds, tile_extent: item.tile_extent, uv: item.uv, pose: item.pose,
            },
            stratum: item.stratum, asset_id: item.asset_id, z: item.z, color_argb: item.color_argb,
            gui_extent, projection_extent, sequence: item.sequence, clip,
        };
        count = preflight_tiled_affine_count(std::slice::from_ref(&request), count)?;
        if !sequences.insert(request.sequence) {
            return Err(GalError::invalid_argument("duplicate tiled GUI parent sequence"));
        }
        owned.push(request);
    }
    Ok(owned)
}
/// Stable `GuiMeshBatchRecord.materialMode` ABI value for title panoramas.
pub(super) const GUI_MESH_MATERIAL_PANORAMA: u32 = 5;

pub(crate) unsafe fn decode_gui_frame_submit(
    request: *const FfiGuiFrameSubmitRequest,
    capabilities: BackendCapabilities,
) -> GalResult<(u64, Handle, Vec<GuiSpriteRequest>)> {
    let (generation, frame_target, sprites, _) =
        decode_gui_frame_submit_with_affine(request, capabilities)?;
    Ok((generation, frame_target, sprites))
}

pub(crate) unsafe fn decode_gui_frame_submit_with_affine(
    request: *const FfiGuiFrameSubmitRequest,
    capabilities: BackendCapabilities,
) -> GalResult<(
    u64,
    Handle,
    Vec<GuiSpriteRequest>,
    Vec<GuiAffineQuadRequest>,
)> {
    let (generation, frame_target, sprites, affine_quads, mesh_batches) =
        decode_gui_frame_submit_with_mesh(request, capabilities)?;
    if !mesh_batches.is_empty() {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "GUI mesh batches require the mesh-aware frame submit path",
        ));
    }
    Ok((generation, frame_target, sprites, affine_quads))
}

pub(crate) unsafe fn decode_gui_frame_submit_with_mesh(
    request: *const FfiGuiFrameSubmitRequest,
    capabilities: BackendCapabilities,
) -> GalResult<(
    u64,
    Handle,
    Vec<GuiSpriteRequest>,
    Vec<GuiAffineQuadRequest>,
    Vec<GuiMeshBatchRequest>,
)> {
    let (generation, target, sprites, affine, meshes, tiles) =
        decode_gui_frame_submit_with_tiles(request, capabilities)?;
    if !tiles.is_empty() {
        return Err(GalError::invalid_argument("tiled GUI requires the typed frame submit path"));
    }
    Ok((generation, target, sprites, affine, meshes))
}

pub(crate) unsafe fn decode_gui_frame_submit_with_tiles(
    request: *const FfiGuiFrameSubmitRequest, capabilities: BackendCapabilities,
) -> GalResult<(u64, Handle, Vec<GuiSpriteRequest>, Vec<GuiAffineQuadRequest>,
    Vec<GuiMeshBatchRequest>, Vec<GuiTiledQuadRequest>)> {
    let request = read_struct(request, "GUI frame submit request")?;
    validate_header::<FfiGuiFrameSubmitRequest>(request.header)?;
    reject_unknown_feature_bits(request.negotiated_feature_bits)?;
    let supported = capability_feature_bits(capabilities);
    if request.negotiated_feature_bits & !supported != 0 {
        return Err(GalError::unsupported_feature(format!(
            "requested unsupported GUI feature bits 0x{:x}",
            request.negotiated_feature_bits & !supported
        )));
    }
    if request.generation == 0 || request.frame_id == 0 {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "GUI frame submit requires non-zero generation and frame id",
        ));
    }
    if request.gui_width <= 0
        || request.gui_height <= 0
        || request.gui_width > GUI_MAX_VIEWPORT_AXIS
        || request.gui_height > GUI_MAX_VIEWPORT_AXIS
    {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!(
                "GUI frame submit dimensions {}x{} exceed bounded positive axis {}",
                request.gui_width, request.gui_height, GUI_MAX_VIEWPORT_AXIS
            ),
        ));
    }
    let frame_target = Handle::from(request.frame_target);
    let projection_extent = [request.gui_projection_width, request.gui_projection_height];
    super::super::gui_frontend::validate_gui_projection(
        [request.gui_width as u32, request.gui_height as u32], projection_extent)?;
    if frame_target.is_null() || frame_target.kind() != Some(HandleKind::FrameTarget) {
        return Err(GalError::ffi(
            StatusCode::WrongHandleType,
            "GUI frame submit requires a frame-target handle",
        ));
    }
    let sprites = read_slice(request.sprites, true, "GUI sprite requests")?;
    if sprites.len() > FFI_MAX_BATCH_ITEMS {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!(
                "GUI frame submit sprite count {} exceeds max {}",
                sprites.len(),
                FFI_MAX_BATCH_ITEMS
            ),
        ));
    }
    let mut owned = Vec::with_capacity(sprites.len());
    for sprite in sprites {
        if sprite.byte_size as usize != size_of::<FfiGuiSpriteRequest>() {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                format!(
                    "GUI sprite byte size mismatch: got {}, expected {}",
                    sprite.byte_size,
                    size_of::<FfiGuiSpriteRequest>()
                ),
            ));
        }
        if sprite.gui_width != request.gui_width || sprite.gui_height != request.gui_height {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "GUI sprite dimensions must match frame GUI dimensions",
            ));
        }
        let to_u32 = |value: i32, field: &str| -> GalResult<u32> {
            u32::try_from(value).map_err(|_| {
                GalError::ffi(
                    StatusCode::InvalidArgument,
                    format!("GUI sprite {field} must be non-negative, got {value}"),
                )
            })
        };
        owned.push(GuiSpriteRequest {
            stratum: sprite.stratum,
            sprite_id: sprite.sprite_id,
            selected_slot: sprite.selected_slot,
            progress_fraction: sprite.progress_fraction,
            fill_direction: sprite.fill_direction,
            color_argb: sprite.color_argb,
            x: sprite.x,
            y: sprite.y,
            width: to_u32(sprite.width, "width")?,
            height: to_u32(sprite.height, "height")?,
            gui_width: to_u32(sprite.gui_width, "gui_width")?,
            gui_height: to_u32(sprite.gui_height, "gui_height")?,
            projection_extent,
            sequence: sprite.sequence,
        });
    }
    let mut affine_quads =
        decode_gui_affine_quads(request.affine_quads, request.gui_width, request.gui_height)?;
    let mut mesh_batches =
        decode_gui_mesh_batches(request.mesh_batches, request.gui_width, request.gui_height)?;
    for quad in &mut affine_quads { quad.projection_extent = projection_extent; }
    for mesh in &mut mesh_batches { mesh.projection_extent = projection_extent; }
    let tiled_quads = decode_gui_tiled_quads(request.tiled_quads,
        [request.gui_width as u32, request.gui_height as u32], projection_extent, affine_quads.len())?;
    super::super::gui_frontend::validate_gui_frame_sequences(&owned, &affine_quads, &mesh_batches, &tiled_quads)?;
    Ok((
        request.generation,
        frame_target,
        owned,
        affine_quads,
        mesh_batches,
        tiled_quads,
    ))
}

fn decode_gui_affine_quads(
    raw: FfiSlice<FfiGuiAffineQuadRequest>,
    gui_width: i32,
    gui_height: i32,
) -> GalResult<Vec<GuiAffineQuadRequest>> {
    let quads = unsafe { read_slice(raw, true, "GUI affine quad requests") }?;
    if quads.len() > GUI_MAX_AFFINE_QUADS {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!(
                "GUI affine quad count {} exceeds max {}",
                quads.len(),
                GUI_MAX_AFFINE_QUADS
            ),
        ));
    }
    let mut owned = Vec::with_capacity(quads.len());
    let mut total_layers = 0_usize;
    for quad in quads {
        validate_item_size::<FfiGuiAffineQuadRequest>(quad.byte_size, "GUI affine quad")?;
        if quad.item_raster_layers.count > super::super::gui_item_raster::MAX_ITEM_LAYERS as u64
            || (quad.item_raster_layers.count != 0 && quad.item_raster_scale == 0) {
            return Err(GalError::invalid_argument("invalid bounded GUI item layers"));
        }
        total_layers += quad.item_raster_layers.count as usize;
        if total_layers > super::super::gui_frontend::GUI_MAX_RAW_IMAGES {
            return Err(GalError::invalid_argument("GUI item layer stream exceeds frame bound"));
        }
        let layers = unsafe { read_slice(quad.item_raster_layers,true,"GUI item layers") }?;
        let mut item_raster_layers = Vec::with_capacity(layers.len());
        for layer in layers {
            validate_item_size::<FfiGuiItemRasterLayer>(layer.byte_size,"GUI item layer")?;
            if layer.asset_id == 0 || !matches!(layer.material_mode,1|2) {
                return Err(GalError::invalid_argument("invalid GUI item layer resource/material"));
            }
            let geometry = super::super::gui_item_raster::GuiItemRasterGeometry {corners:layer.corners};
            geometry.identity()?;
            super::super::gui_item_raster::item_uv_identity(layer.uv)?;
            let model_transform=super::super::gui_item_raster::GuiItemModelTransform(layer.model_transform);
            model_transform.validate()?;
            item_raster_layers.push(super::super::gui_frontend::GuiItemRasterLayer {
                asset_id:layer.asset_id,color_argb:layer.color_argb,geometry,uv:layer.uv,model_transform,
                material:super::super::gui_item_material::GuiAffineMaterial::decode(layer.material_mode)?,
            });
        }
        if quad.asset_id == 0 || quad.gui_width != gui_width || quad.gui_height != gui_height {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "GUI affine quad asset and viewport must match its frame",
            ));
        }
        let to_u32 = |value: i32, field: &str| -> GalResult<u32> {
            u32::try_from(value).map_err(|_| {
                GalError::ffi(
                    StatusCode::InvalidArgument,
                    format!("GUI affine quad {field} must be non-negative, got {value}"),
                )
            })
        };
        let request = GuiAffineQuadRequest {
            item_raster_layers,
            item_raster_scale: quad.item_raster_scale,
            item_raster_geometry: super::super::gui_item_raster::GuiItemRasterGeometry {
                corners: quad.item_raster_corners },
            material: super::super::gui_item_material::GuiAffineMaterial::decode(quad.material_mode)?,
            stratum: quad.stratum,
            asset_id: quad.asset_id,
            x0: quad.x0,
            y0: quad.y0,
            x1: quad.x1,
            y1: quad.y1,
            x3: quad.x3,
            y3: quad.y3,
            z: quad.z,
            u0: quad.u0,
            v0: quad.v0,
            u1: quad.u1,
            v1: quad.v1,
            color_argb: quad.color_argb,
            gui_width: to_u32(quad.gui_width, "gui_width")?,
            gui_height: to_u32(quad.gui_height, "gui_height")?,
            projection_extent: [gui_width as f32, gui_height as f32],
            sequence: quad.sequence,
            clip_mode: quad.clip_mode,
            clip_left: quad.clip_left,
            clip_top: quad.clip_top,
            clip_width: quad.clip_width,
            clip_height: quad.clip_height,
        };
        super::super::gui_frontend::validate_affine_quad(&request)?;
        owned.push(request);
    }
    Ok(owned)
}

/// Decodes the private coarse GUI mesh family. Callers may use this only once
/// the owned GUI mesh pass is available; defining the transport does not arm a
/// route or expose a backend capability.
pub(crate) unsafe fn decode_gui_mesh_batches(
    raw: FfiSlice<FfiGuiMeshBatchRequest>,
    gui_width: i32,
    gui_height: i32,
) -> GalResult<Vec<GuiMeshBatchRequest>> {
    let batches = read_slice(raw, true, "GUI mesh batch requests")?;
    if batches.len() > GUI_MESH_MAX_BATCHES {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!(
                "GUI mesh batch count {} exceeds max {}",
                batches.len(),
                GUI_MESH_MAX_BATCHES
            ),
        ));
    }
    // Inspect every nested slice descriptor before copying any geometry. The
    // per-batch limits alone would otherwise allow their product to exceed a
    // safe frame-sized allocation.
    let mut payload_bytes = 0_u64;
    for batch in batches {
        let vertices = read_slice(batch.vertices, true, "GUI mesh vertices")?;
        let indices = read_slice(batch.indices, true, "GUI mesh indices")?;
        if vertices.len() > GUI_MESH_MAX_VERTICES || indices.len() > GUI_MESH_MAX_INDICES {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "GUI mesh payload exceeds its bounded vertex or index capacity",
            ));
        }
        payload_bytes = payload_bytes
            .saturating_add(
                (vertices.len() as u64)
                    .saturating_mul(std::mem::size_of::<FfiGuiMeshVertex>() as u64),
            )
            .saturating_add(
                (indices.len() as u64).saturating_mul(std::mem::size_of::<u32>() as u64),
            );
        if payload_bytes > GUI_MESH_MAX_FRAME_PAYLOAD_BYTES {
            return Err(GalError::ffi(
                StatusCode::LengthOverflow,
                format!(
                    "GUI mesh frame payload {} exceeds bounded limit {}",
                    payload_bytes, GUI_MESH_MAX_FRAME_PAYLOAD_BYTES
                ),
            ));
        }
    }
    let mut owned = Vec::with_capacity(batches.len());
    for batch in batches {
        validate_item_size::<FfiGuiMeshBatchRequest>(batch.byte_size, "GUI mesh batch")?;
        if batch.reserved0 != 0 || batch.gui_width != gui_width || batch.gui_height != gui_height {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "GUI mesh batch reserved fields and frame GUI extent must match",
            ));
        }
        let material_mode = match batch.material_mode {
            1 => GuiMeshMaterialMode::Opaque,
            2 => GuiMeshMaterialMode::Cutout,
            3 => GuiMeshMaterialMode::Translucent,
            4 => GuiMeshMaterialMode::Glint,
            GUI_MESH_MATERIAL_PANORAMA => GuiMeshMaterialMode::Panorama,
            6 => GuiMeshMaterialMode::ModelOverlay,
            other => {
                return Err(GalError::ffi(
                    StatusCode::UnknownEnum,
                    format!("unknown GUI mesh material mode {other}"),
                ));
            }
        };
        let lighting_mode = match batch.lighting_mode {
            1 => GuiMeshLightingMode::Flat,
            2 => GuiMeshLightingMode::Block,
            3 => GuiMeshLightingMode::InventoryBlock,
            4 => GuiMeshLightingMode::FrontModel,
            other => {
                return Err(GalError::ffi(
                    StatusCode::UnknownEnum,
                    format!("unknown GUI mesh lighting mode {other}"),
                ));
            }
        };
        let vertices = read_slice(batch.vertices, true, "GUI mesh vertices")?;
        let indices = read_slice(batch.indices, true, "GUI mesh indices")?;
        if vertices.len() > GUI_MESH_MAX_VERTICES || indices.len() > GUI_MESH_MAX_INDICES {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "GUI mesh payload exceeds its bounded vertex or index capacity",
            ));
        }
        let vertices = vertices
            .iter()
            .map(|vertex| GuiMeshVertex {
                position: vertex.position,
                atlas_uv: vertex.atlas_uv,
                local_uv: vertex.local_uv,
                color_argb: vertex.color_argb,
                normal_packed: vertex.normal_packed,
                source_face: vertex.source_face,
                source_foil_type: vertex.source_foil_type,
            })
            .collect();
        let request = GuiMeshBatchRequest {
            item_cache: super::super::gui_mesh_frontend::GuiItemCache::decode(batch.item_cache_identity, batch.item_cache_mode)?,
            block_item_raster: super::super::gui_mesh_frontend::GuiBlockItemRaster::decode(
                batch.block_item_scale, batch.block_model_bounds, batch.block_item_layout)?,
            decal_foil: super::super::gui_mesh_frontend::GuiDecalFoilProjection::decode(
                batch.decal_foil_mode, batch.decal_model_pose, batch.decal_normal_pose)?,
            item_raster_scale: batch.item_raster_scale,
            item_lighting: None,
            item_foil: super::super::item_foil::StandardItemFoil::decode(
                batch.item_foil_mode, batch.item_foil_clock_millis,
                batch.item_foil_speed, batch.item_foil_strength)?,
            stratum: batch.stratum,
            layer_index: batch.layer_index,
            sequence: batch.sequence,
            asset_id: batch.asset_id,
            material_mode,
            lighting_mode,
            alpha_cutoff: batch.alpha_cutoff,
            model_transform: batch.model_transform,
            gui_pose: batch.gui_pose,
            bounds: [batch.left, batch.top, batch.right, batch.bottom],
            gui_extent: [
                u32::try_from(batch.gui_width).map_err(|_| {
                    GalError::ffi(
                        StatusCode::InvalidArgument,
                        "GUI mesh width must be positive",
                    )
                })?,
                u32::try_from(batch.gui_height).map_err(|_| {
                    GalError::ffi(
                        StatusCode::InvalidArgument,
                        "GUI mesh height must be positive",
                    )
                })?,
            ],
            projection_extent: [gui_width as f32, gui_height as f32],
            render_extent: [
                u32::try_from(batch.render_width).map_err(|_| {
                    GalError::ffi(
                        StatusCode::InvalidArgument,
                        "GUI mesh render width must be positive",
                    )
                })?,
                u32::try_from(batch.render_height).map_err(|_| {
                    GalError::ffi(
                        StatusCode::InvalidArgument,
                        "GUI mesh render height must be positive",
                    )
                })?,
            ],
            guard_pixels: batch.guard_pixels,
            clip_mode: batch.clip_mode,
            clip_left: batch.clip_left,
            clip_top: batch.clip_top,
            clip_width: batch.clip_width,
            clip_height: batch.clip_height,
            vertices,
            indices: indices.to_vec(),
        };
        validate_gui_mesh_batch(&request)?;
        owned.push(request);
    }
    validate_gui_mesh_batches(&owned)?;
    Ok(owned)
}

fn validate_gui_request_sequences(
    sprites: &[GuiSpriteRequest],
    affine_quads: &[GuiAffineQuadRequest],
    mesh_batches: &[GuiMeshBatchRequest],
) -> GalResult<()> {
    let mut sequences = Vec::with_capacity(sprites.len() + affine_quads.len());
    sequences.extend(sprites.iter().map(|request| request.sequence));
    sequences.extend(affine_quads.iter().map(|request| request.sequence));
    // Layers intentionally share their item's sequence. The mesh frontend
    // validates contiguous layer indices and treats them as one ordered item.
    let mesh_sequences = mesh_batches
        .iter()
        .map(|request| request.sequence)
        .collect::<std::collections::BTreeSet<_>>();
    sequences.extend(mesh_sequences);
    if sequences.iter().any(|sequence| *sequence == 0) {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "GUI requests require non-zero scheduler sequences",
        ));
    }
    sequences.sort_unstable();
    if sequences.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "GUI request scheduler sequences must be unique within one frame",
        ));
    }
    Ok(())
}

pub(crate) unsafe fn decode_gui_asset_update(
    request: *const FfiGuiAssetUpdateRequest,
    capabilities: BackendCapabilities,
) -> GalResult<(u64, Vec<GuiAssetPayload>)> {
    let request = read_struct(request, "GUI asset update request")?;
    validate_header::<FfiGuiAssetUpdateRequest>(request.header)?;
    reject_unknown_feature_bits(request.negotiated_feature_bits)?;
    let supported = capability_feature_bits(capabilities);
    if request.negotiated_feature_bits & !supported != 0 {
        return Err(GalError::unsupported_feature(format!(
            "requested unsupported GUI asset feature bits 0x{:x}",
            request.negotiated_feature_bits & !supported
        )));
    }
    if request.generation == 0 {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "GUI asset generation must be non-zero",
        ));
    }
    let assets = read_limited_slice(request.assets, true, "GUI asset payloads")?;
    if assets.len() > GUI_MAX_RAW_IMAGES {
        return Err(GalError::ffi(
            StatusCode::LengthOverflow,
            format!(
                "GUI asset payload count {} exceeds bounded limit {GUI_MAX_RAW_IMAGES}",
                assets.len()
            ),
        ));
    }
    let mut seen = BTreeMap::new();
    let mut owned = Vec::with_capacity(assets.len());
    let mut png_bytes_total = 0usize;
    for asset in assets {
        validate_item_size::<FfiGuiAssetPayload>(asset.byte_size, "GUI asset payload")?;
        if seen.insert(asset.sprite_id, ()).is_some() {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                format!(
                    "duplicate GUI asset payload for sprite id {}",
                    asset.sprite_id
                ),
            ));
        }
        png_bytes_total = png_bytes_total
            .checked_add(asset.png_bytes.len as usize)
            .ok_or_else(|| {
                GalError::ffi(
                    StatusCode::LengthOverflow,
                    "GUI asset PNG byte count overflow",
                )
            })?;
        if png_bytes_total > GUI_MAX_RAW_IMAGE_BYTES_TOTAL {
            return Err(GalError::ffi(
                StatusCode::LengthOverflow,
                format!(
                    "GUI asset PNG bytes {} exceed bounded total {GUI_MAX_RAW_IMAGE_BYTES_TOTAL}",
                    png_bytes_total
                ),
            ));
        }
        let png_bytes = read_bounded_bytes(
            asset.png_bytes,
            true,
            FFI_MAX_GUI_ASSET_BYTES,
            "GUI asset PNG bytes",
        )?;
        owned.push(GuiAssetPayload {
            sprite_id: asset.sprite_id,
            png_bytes,
        });
    }
    Ok((request.generation, owned))
}

pub(crate) unsafe fn decode_gui_raw_image_update(
    request: *const FfiGuiRawImageUpdateRequest,
    capabilities: BackendCapabilities,
) -> GalResult<(u64, Vec<GuiRawImageAssetPayload>)> {
    let request = read_struct(request, "raw GUI image update request")?;
    validate_header::<FfiGuiRawImageUpdateRequest>(request.header)?;
    reject_unknown_feature_bits(request.negotiated_feature_bits)?;
    let supported = capability_feature_bits(capabilities);
    if request.negotiated_feature_bits & !supported != 0 {
        return Err(GalError::unsupported_feature(format!(
            "requested unsupported raw GUI image feature bits 0x{:x}",
            request.negotiated_feature_bits & !supported
        )));
    }
    if request.generation == 0 {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "raw GUI image generation must be non-zero",
        ));
    }
    let assets = read_limited_slice(request.assets, true, "raw GUI image payloads")?;
    if assets.len() > GUI_MAX_RAW_IMAGES {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!(
                "raw GUI image payload count {} exceeds bounded limit {GUI_MAX_RAW_IMAGES}",
                assets.len()
            ),
        ));
    }
    let mut seen = BTreeMap::new();
    let mut owned = Vec::with_capacity(assets.len());
    let mut total_pixels = 0usize;
    for asset in assets {
        validate_item_size::<FfiGuiRawImageAssetPayload>(asset.byte_size, "raw GUI image payload")?;
        if asset.asset_id == 0 || seen.insert(asset.asset_id, ()).is_some() {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "raw GUI image asset ids must be unique and non-zero",
            ));
        }
        let format = match asset.format {
            1 => GuiRawImageFormat::Alpha8,
            2 => GuiRawImageFormat::Rgba8,
            other => {
                return Err(GalError::ffi(
                    StatusCode::UnknownEnum,
                    format!("unknown raw GUI image format {other}"),
                ));
            }
        };
        let width = u32::try_from(asset.width).map_err(|_| {
            GalError::ffi(
                StatusCode::InvalidArgument,
                "raw GUI image width must be positive",
            )
        })?;
        let height = u32::try_from(asset.height).map_err(|_| {
            GalError::ffi(
                StatusCode::InvalidArgument,
                "raw GUI image height must be positive",
            )
        })?;
        let pixel_count = usize::try_from(width)
            .ok()
            .and_then(|width| {
                usize::try_from(height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .ok_or_else(|| {
                GalError::ffi(
                    StatusCode::LengthOverflow,
                    "raw GUI image pixel count overflows",
                )
            })?;
        if pixel_count == 0 || pixel_count > GUI_MAX_RAW_IMAGE_PIXELS {
            return Err(GalError::ffi(
                StatusCode::LengthOverflow,
                format!(
                    "raw GUI image {} has {} pixels; maximum is {GUI_MAX_RAW_IMAGE_PIXELS}",
                    asset.asset_id, pixel_count
                ),
            ));
        }
        let bytes_per_pixel = match format {
            GuiRawImageFormat::Alpha8 => 1usize,
            GuiRawImageFormat::Rgba8 => 4usize,
        };
        let expected_bytes = pixel_count.checked_mul(bytes_per_pixel).ok_or_else(|| {
            GalError::ffi(
                StatusCode::LengthOverflow,
                "raw GUI image byte count overflows",
            )
        })?;
        let incoming_bytes = usize::try_from(asset.pixels.len).map_err(|_| {
            GalError::ffi(
                StatusCode::LengthOverflow,
                "raw GUI image byte length exceeds usize",
            )
        })?;
        if incoming_bytes != expected_bytes {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                format!(
                    "raw GUI image {} has {} bytes; expected {expected_bytes}",
                    asset.asset_id, incoming_bytes
                ),
            ));
        }
        total_pixels = total_pixels.checked_add(expected_bytes).ok_or_else(|| {
            GalError::ffi(
                StatusCode::LengthOverflow,
                "raw GUI image aggregate byte count overflows",
            )
        })?;
        if total_pixels > GUI_MAX_RAW_IMAGE_BYTES_TOTAL {
            return Err(GalError::ffi(
                StatusCode::LengthOverflow,
                format!(
                    "raw GUI image aggregate bytes {total_pixels} exceed bounded limit {GUI_MAX_RAW_IMAGE_BYTES_TOTAL}"
                ),
            ));
        }
        let pixels = read_bounded_bytes(
            asset.pixels,
            true,
            FFI_MAX_GUI_ASSET_BYTES,
            "raw GUI image pixels",
        )?;
        owned.push(GuiRawImageAssetPayload {
            sampling: match (asset.sampling_filter, asset.sampling_address) {
                (0, 0) => None,
                (filter @ 1..=2, address @ 1..=2) => Some((
                    if filter == 1 { super::super::resources::SamplerFilter::Nearest } else { super::super::resources::SamplerFilter::Linear },
                    if address == 1 { super::super::resources::SamplerAddressMode::Repeat } else { super::super::resources::SamplerAddressMode::ClampToEdge })),
                _ => return Err(GalError::invalid_argument("invalid explicit raw GUI image sampling")),
            },
            asset_id: asset.asset_id,
            format,
            width,
            height,
            pixels,
        });
    }
    Ok((request.generation, owned))
}

#[no_mangle]
pub unsafe extern "C" fn mattmc_vulkanic_gal_gui_submit_frame(
    context_id: u64,
    request: *const FfiGuiFrameSubmitRequest,
    out: *mut FfiGuiFrameSubmitResult,
) -> i32 {
    with_registry_mut(|registry| {
        let Some(context) = registry.contexts.get_mut(&context_id) else {
            let error = GalError::ffi(
                StatusCode::StaleHandle,
                format!("unknown context id {context_id}"),
            );
            let _ = write_out(
                out,
                FfiGuiFrameSubmitResult {
                    status: error.code as i32,
                    error_domain: error.domain as u32,
                    ..FfiGuiFrameSubmitResult::default()
                },
                "GUI frame submit result",
            );
            return error.code as i32;
        };
        let input_bytes = if request.is_null() {
            0
        } else {
            input_bytes_for_gui_frame(&*request)
        };
        context.ffi_calls += 1;
        context.ffi_input_bytes = context.ffi_input_bytes.saturating_add(input_bytes);
        context.ffi_output_bytes = context
            .ffi_output_bytes
            .saturating_add(size_of::<FfiGuiFrameSubmitResult>() as u64);
        let result = decode_gui_frame_submit_with_tiles(request, context.gal.capabilities())
            .and_then(
                |(generation, frame_target, sprites, affine_quads, mesh_batches, tiled_quads)| {
                    context.ffi_input_bytes = context.ffi_input_bytes.saturating_add(
                        affine_quads.iter().map(|quad|quad.item_raster_layers.len() as u64
                            * size_of::<FfiGuiItemRasterLayer>() as u64).sum::<u64>());
                    let stats = context
                        .gui_frontend
                        .submit_frame_with_owned_atlases(
                            &mut context.gal,
                            Some(&mut context.world_primitive_frontend),
                            generation,
                            frame_target,
                            sprites,
                            affine_quads,
                            mesh_batches,
                            tiled_quads,
                        )?;
                    destroy_stale_frame_targets(context)?;
                    Ok(stats)
                },
            );
        match result {
            Ok(stats) => {
                let value = gui_frame_result_ok(context, stats);
                let _ = write_out(out, value, "GUI frame submit result");
                StatusCode::Ok as i32
            }
            Err(error) => {
                set_last_error(context, &error);
                let _ = write_out(
                    out,
                    FfiGuiFrameSubmitResult {
                        status: error.code as i32,
                        error_domain: error.domain as u32,
                        metrics: context_metrics(context),
                        ..FfiGuiFrameSubmitResult::default()
                    },
                    "GUI frame submit result",
                );
                error.code as i32
            }
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn mattmc_vulkanic_gal_gui_update_assets(
    context_id: u64,
    request: *const FfiGuiAssetUpdateRequest,
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
            input_bytes_for_gui_asset_update(&*request)
        };
        context.ffi_calls += 1;
        context.ffi_input_bytes = context.ffi_input_bytes.saturating_add(input_bytes);
        context.ffi_output_bytes = context
            .ffi_output_bytes
            .saturating_add(size_of::<FfiStatusResult>() as u64);
        let result = decode_gui_asset_update(request, context.gal.capabilities()).and_then(
            |(generation, assets)| {
                context
                    .gui_frontend
                    .apply_asset_update(&mut context.gal, generation, assets)
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

#[no_mangle]
pub unsafe extern "C" fn mattmc_vulkanic_gal_gui_update_raw_images(
    context_id: u64,
    request: *const FfiGuiRawImageUpdateRequest,
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
            input_bytes_for_gui_raw_image_update(&*request)
        };
        context.ffi_calls += 1;
        context.ffi_input_bytes = context.ffi_input_bytes.saturating_add(input_bytes);
        context.ffi_output_bytes = context
            .ffi_output_bytes
            .saturating_add(size_of::<FfiStatusResult>() as u64);
        let result = decode_gui_raw_image_update(request, context.gal.capabilities()).and_then(
            |(generation, assets)| {
                context
                    .gui_frontend
                    .apply_raw_image_update(&mut context.gal, generation, assets)
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
