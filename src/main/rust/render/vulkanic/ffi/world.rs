use super::*;
use crate::render::vulkanic::shader_pack::lightmap::{VanillaLightmapFrame, VanillaLightmapInputs};
use crate::render::vulkanic::world_primitive_frontend::material as world_material_semantics;
use crate::render::vulkanic::world_primitive_frontend::world_text::{
    WorldTextImageAsset, WorldTextImageFormat, WorldTextQuadRequest, MAX_WORLD_TEXT_IMAGES,
    MAX_WORLD_TEXT_IMAGE_BYTES_TOTAL, WORLD_TEXT_DEPTH_NORMAL, WORLD_TEXT_DEPTH_POLYGON_OFFSET,
    WORLD_TEXT_DEPTH_SEE_THROUGH,
};
use crate::render::vulkanic::world_primitive_frontend::{
    WorldFeatureCoverageFrame, WorldFirstPersonFrame, WorldLodRenderFrame,
    WorldShaderEnvironmentFrame, WorldVoxelVolumeFrame, WORLD_LOD_MAX_COLUMNS,
    WORLD_LOD_MAX_NORMAL_INDEX, WORLD_LOD_MAX_SEGMENTS_PER_COLUMN,
    WORLD_LOD_MAX_VERTICES_PER_SEGMENT, WORLD_LOD_MAX_VISIBLE_SEGMENTS,
    WORLD_MATERIAL_SOURCE_CLOUDS, WORLD_MATERIAL_SOURCE_ENTITY_MODEL,
    WORLD_MATERIAL_SOURCE_PARTICLES, WORLD_MATERIAL_SOURCE_TEXTURED,
    WORLD_MATERIAL_SOURCE_UNSPECIFIED, WORLD_MATERIAL_SOURCE_UV_LOCAL_TEXTURE,
    WORLD_MATERIAL_SOURCE_UV_MINECRAFT_BLOCK_ATLAS, WORLD_MATERIAL_SOURCE_WEATHER,
    WORLD_MESH_INSTANCE_FLAG_OUTLINE_ONLY,
    WORLD_MESH_INSTANCE_FLAG_CAMERA_SORTED_QUADS,
    WORLD_MESH_SECTION_ALL,
};
use std::collections::BTreeSet;

fn decode_world_item_foil(instance: &FfiWorldMeshInstanceRecord) -> GalResult<Option<super::super::item_foil::StandardItemFoil>> {
    let foil = super::super::item_foil::StandardItemFoil::decode(instance.item_foil_mode,
        instance.item_foil_clock_millis, instance.item_foil_speed, instance.item_foil_strength)?;
    if foil.is_some() && (instance.stratum != WORLD_STRATUM_ENTITY_MESH
        || instance.terrain_placement_mode != 0 || instance.flags != 0 || instance.block_entity_id != -1) {
        return Err(GalError::invalid_argument("standard foil requires an ordinary entity mesh instance"));
    }
    Ok(foil)
}

fn is_world_mesh_stratum(stratum: u32) -> bool {
    matches!(
        stratum,
        WORLD_STRATUM_TERRAIN
            | WORLD_STRATUM_OPAQUE_TEXTURED_GEOMETRY
            | WORLD_STRATUM_MOVING_MESH
            | WORLD_STRATUM_ENTITY_MESH
    )
}

fn decode_world_viewport_axis(value: i32, label: &str) -> GalResult<u32> {
    if value <= 0 || value > GUI_MAX_VIEWPORT_AXIS {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!(
                "{label} must be within bounded positive range 1..={GUI_MAX_VIEWPORT_AXIS}, got {value}"
            ),
        ));
    }
    Ok(value as u32)
}

fn validate_mesh_instance_semantic_identity(
    instance: &FfiWorldMeshInstanceRecord,
    label: &str,
) -> GalResult<()> {
    decode_mesh_instance_transform(instance)?;
    if instance.mesh_key == 0 || instance.mesh_generation == 0 {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!("{label} key and generation must be non-zero"),
        ));
    }
    if instance.flags & !(WORLD_MESH_INSTANCE_FLAG_OUTLINE_ONLY | WORLD_MESH_INSTANCE_FLAG_CAMERA_SORTED_QUADS) != 0 {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!("{label} contains unknown semantic flags"),
        ));
    }
    if instance.flags & WORLD_MESH_INSTANCE_FLAG_CAMERA_SORTED_QUADS != 0
        && (instance.stratum != WORLD_STRATUM_TERRAIN
            || instance.mesh_section_index != WORLD_MESH_SECTION_ALL
            || !matches!(instance.depth_policy, WORLD_DEPTH_POLICY_TEST_WRITE | WORLD_DEPTH_POLICY_TEST_NO_WRITE))
    {
        return Err(GalError::ffi(StatusCode::InvalidArgument,
            format!("{label} camera-sorted quads require complete translucent terrain")));
    }
    if instance.flags & WORLD_MESH_INSTANCE_FLAG_OUTLINE_ONLY != 0
        && (instance.stratum != WORLD_STRATUM_ENTITY_MESH || instance.outline_color_argb == 0)
    {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!("{label} outline-only flag requires entity stratum and outline color"),
        ));
    }
    if instance.block_entity_id < -1 {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!("{label} block entity id must be >= -1"),
        ));
    }
    Ok(())
}

fn decode_mesh_instance_transform(instance: &FfiWorldMeshInstanceRecord) -> GalResult<[f32;16]> {
    match instance.terrain_placement_mode {
        0 if instance.terrain_origin == [0;3] && instance.terrain_camera == [0.0;3] => Ok(instance.transform),
        1 if instance.stratum == WORLD_STRATUM_TERRAIN && instance.mesh_section_index == WORLD_MESH_SECTION_ALL
            && instance.entity_id == 0 && instance.block_entity_id == -1
            && instance.transform == [1.0,0.0,0.0,0.0,0.0,1.0,0.0,0.0,0.0,0.0,1.0,0.0,0.0,0.0,0.0,1.0] => {
            crate::render::vulkanic::terrain::placement::TerrainSectionPlacement {
                origin:instance.terrain_origin, camera:instance.terrain_camera,
            }.lower()
        }
        _ => Err(GalError::invalid_argument("incoherent semantic terrain placement")),
    }
}

pub(crate) unsafe fn decode_world_text_image_update(
    request: *const FfiWorldTextImageUpdateRequest,
    capabilities: BackendCapabilities,
) -> GalResult<(u64, Vec<WorldTextImageAsset>)> {
    let request = read_struct(request, "world text image update request")?;
    validate_header::<FfiWorldTextImageUpdateRequest>(request.header)?;
    reject_unknown_feature_bits(request.negotiated_feature_bits)?;
    if request.negotiated_feature_bits & !capability_feature_bits(capabilities) != 0 {
        return Err(GalError::unsupported_feature(
            "world text image update requests unsupported backend features",
        ));
    }
    if request.generation == 0 {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "world text image update generation must be non-zero",
        ));
    }
    let raw_assets = read_limited_slice(request.assets, true, "world text image assets")?;
    if raw_assets.len() > MAX_WORLD_TEXT_IMAGES {
        return Err(GalError::ffi(
            StatusCode::LengthOverflow,
            format!(
                "world text image asset count {} exceeds bounded limit {MAX_WORLD_TEXT_IMAGES}",
                raw_assets.len()
            ),
        ));
    }
    let mut assets = Vec::with_capacity(raw_assets.len());
    let mut total_bytes = 0usize;
    for asset in raw_assets {
        validate_item_size::<FfiWorldTextImageAssetPayload>(
            asset.byte_size,
            "world text image asset",
        )?;
        let pixels = read_bounded_bytes(
            asset.pixels,
            true,
            4 * 1024 * 1024,
            "world text image pixels",
        )?;
        total_bytes = total_bytes.checked_add(pixels.len()).ok_or_else(|| {
            GalError::ffi(
                StatusCode::LengthOverflow,
                "world text image pixel byte count overflow",
            )
        })?;
        if total_bytes > MAX_WORLD_TEXT_IMAGE_BYTES_TOTAL {
            return Err(GalError::ffi(
                StatusCode::LengthOverflow,
                format!(
                    "world text image pixels {} exceed bounded total {MAX_WORLD_TEXT_IMAGE_BYTES_TOTAL}",
                    total_bytes
                ),
            ));
        }
        assets.push(WorldTextImageAsset {
            asset_id: asset.asset_id,
            atlas_generation: asset.atlas_generation,
            atlas_revision: asset.atlas_revision,
            format: WorldTextImageFormat::try_from(asset.format)?,
            width: asset.width,
            height: asset.height,
            pixels,
        });
    }
    Ok((request.generation, assets))
}

/// Decodes one copied DH LOD asset generation. Rendering admission is
/// intentionally separate: this endpoint owns only transport validation and
/// caller-memory copying into the Rust frontend cache.
pub(crate) unsafe fn decode_world_lod_asset_update(
    request: *const FfiWorldLodAssetUpdateRequest,
    capabilities: BackendCapabilities,
) -> GalResult<(
    u64,
    Vec<WorldLodColumnAsset>,
    Vec<WorldLodColumnRetirement>,
    Vec<WorldLodColumnMaterialProvenance>,
)> {
    let request = read_struct(request, "world LOD asset update request")?;
    validate_header::<FfiWorldLodAssetUpdateRequest>(request.header)?;
    reject_unknown_feature_bits(request.negotiated_feature_bits)?;
    let supported = capability_feature_bits(capabilities);
    if request.negotiated_feature_bits & !supported != 0 {
        return Err(GalError::unsupported_feature(format!(
            "requested unsupported world LOD feature bits 0x{:x}",
            request.negotiated_feature_bits & !supported
        )));
    }
    if request.generation == 0 {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "world LOD asset update generation must be non-zero",
        ));
    }
    let raw_assets = read_limited_slice(request.assets, true, "world LOD column assets")?;
    if raw_assets.len() > WORLD_LOD_MAX_COLUMNS {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!(
                "world LOD column asset count {} exceeds bounded limit {WORLD_LOD_MAX_COLUMNS}",
                raw_assets.len()
            ),
        ));
    }
    let mut assets = Vec::with_capacity(raw_assets.len());
    let mut asset_keys = BTreeMap::new();
    for asset in raw_assets {
        validate_item_size::<FfiWorldLodColumnAssetRecord>(
            asset.byte_size,
            "world LOD column asset",
        )?;
        if asset.reserved0 != 0 {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "world LOD column asset reserved field must be zero",
            ));
        }
        if asset.column_generation == 0 {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "world LOD column asset generation must be non-zero",
            ));
        }
        if asset_keys.insert(asset.column_key, ()).is_some() {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                format!("duplicate world LOD column asset {}", asset.column_key),
            ));
        }
        let raw_segments = read_limited_slice(asset.segments, false, "world LOD segments")?;
        if raw_segments.is_empty() || raw_segments.len() > WORLD_LOD_MAX_SEGMENTS_PER_COLUMN {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                format!(
                    "world LOD column {} has {} segments; expected 1..={}",
                    asset.column_key,
                    raw_segments.len(),
                    WORLD_LOD_MAX_SEGMENTS_PER_COLUMN
                ),
            ));
        }
        let mut segments = Vec::with_capacity(raw_segments.len());
        for segment in raw_segments {
            validate_item_size::<FfiWorldLodSegmentRecord>(segment.byte_size, "world LOD segment")?;
            let raw_vertices = read_limited_slice(segment.vertices, true, "world LOD vertices")?;
            // This is a byte stream, not a batch of records: its bound is
            // derived from the explicit vertex limit rather than the generic
            // FFI item-count ceiling.
            let packed_vertices =
                read_slice(segment.packed_vertices, true, "packed world LOD vertices")?;
            if packed_vertices.len() > WORLD_LOD_MAX_VERTICES_PER_SEGMENT * 16 {
                return Err(GalError::ffi(
                    StatusCode::LengthOverflow,
                    "packed world LOD vertices byte length exceeds ABI maximum",
                ));
            }
            if !raw_vertices.is_empty() && !packed_vertices.is_empty() {
                return Err(GalError::ffi(
                    StatusCode::InvalidArgument,
                    "world LOD segment must use either structured or packed vertices, not both",
                ));
            }
            let vertex_count = if !packed_vertices.is_empty() {
                const PACKED_DH_VERTEX_BYTES: usize = 16;
                if packed_vertices.len() % PACKED_DH_VERTEX_BYTES != 0 {
                    return Err(GalError::ffi(
                        StatusCode::InvalidArgument,
                        "packed world LOD vertices are not 16-byte aligned",
                    ));
                }
                packed_vertices.len() / PACKED_DH_VERTEX_BYTES
            } else {
                raw_vertices.len()
            };
            if vertex_count == 0
                || vertex_count > WORLD_LOD_MAX_VERTICES_PER_SEGMENT
                || vertex_count % 4 != 0
            {
                return Err(GalError::ffi(
                    StatusCode::InvalidArgument,
                    format!(
                        "world LOD segment has {vertex_count} vertices; expected quad-aligned 1..={}",
                        WORLD_LOD_MAX_VERTICES_PER_SEGMENT
                    ),
                ));
            }
            let mut vertices = Vec::with_capacity(vertex_count);
            if !packed_vertices.is_empty() {
                for bytes in packed_vertices.chunks_exact(16) {
                    let local_x = u16::from_ne_bytes([bytes[0], bytes[1]]);
                    let local_y = u16::from_ne_bytes([bytes[2], bytes[3]]);
                    let local_z = u16::from_ne_bytes([bytes[4], bytes[5]]);
                    let packed_light_and_micro_offset = u16::from_ne_bytes([bytes[6], bytes[7]]);
                    let color_rgba = [bytes[8], bytes[9], bytes[10], bytes[11]];
                    let material_id = bytes[12];
                    let normal_index = bytes[13];
                    if bytes[14] != 0 || bytes[15] != 0 {
                        return Err(GalError::ffi(
                            StatusCode::InvalidArgument,
                            "packed world LOD vertex has non-zero reserved padding",
                        ));
                    }
                    if material_id > 15 || normal_index > WORLD_LOD_MAX_NORMAL_INDEX {
                        return Err(GalError::ffi(
                            StatusCode::InvalidArgument,
                            "packed world LOD vertex contains an out-of-range material or normal",
                        ));
                    }
                    vertices.push(WorldLodVertex {
                        local_position: [local_x, local_y, local_z],
                        packed_light_and_micro_offset,
                        color_rgba,
                        material_id,
                        normal_index,
                    });
                }
            } else {
                for vertex in raw_vertices {
                    validate_item_size::<FfiWorldLodVertex>(vertex.byte_size, "world LOD vertex")?;
                    let material_id = u8::try_from(vertex.material_id).map_err(|_| {
                        GalError::ffi(
                            StatusCode::InvalidArgument,
                            format!("world LOD material id {} exceeds u8", vertex.material_id),
                        )
                    })?;
                    let normal_index = u8::try_from(vertex.normal_index).map_err(|_| {
                        GalError::ffi(
                            StatusCode::InvalidArgument,
                            format!("world LOD normal index {} exceeds u8", vertex.normal_index),
                        )
                    })?;
                    vertices.push(WorldLodVertex {
                        local_position: [vertex.local_x, vertex.local_y, vertex.local_z],
                        packed_light_and_micro_offset: vertex.packed_light_and_micro_offset,
                        color_rgba: vertex.color_rgba.to_le_bytes(),
                        material_id,
                        normal_index,
                    });
                }
            }
            segments.push(WorldLodSegment {
                layer: segment.layer,
                vertices,
            });
        }
        assets.push(WorldLodColumnAsset {
            column_key: asset.column_key,
            column_generation: asset.column_generation,
            vertex_layout_version: asset.vertex_layout_version,
            origin: [asset.origin_x, asset.origin_y, asset.origin_z],
            segments,
        });
    }
    let raw_retirements = read_limited_slice(request.retirements, true, "world LOD retirements")?;
    if raw_retirements.len() > WORLD_LOD_MAX_COLUMNS {
        return Err(GalError::ffi(
            StatusCode::LengthOverflow,
            format!(
                "world LOD retirement count {} exceeds bounded limit {WORLD_LOD_MAX_COLUMNS}",
                raw_retirements.len()
            ),
        ));
    }
    let mut retirements = Vec::with_capacity(raw_retirements.len());
    let mut retirement_keys = BTreeMap::new();
    for retirement in raw_retirements {
        validate_item_size::<FfiWorldLodColumnRetirementRecord>(
            retirement.byte_size,
            "world LOD retirement",
        )?;
        if retirement.reserved0 != 0 || retirement.column_generation == 0 {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "world LOD retirement reserved field and generation must be valid",
            ));
        }
        if retirement_keys.insert(retirement.column_key, ()).is_some()
            || asset_keys.contains_key(&retirement.column_key)
        {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                format!(
                    "duplicate or conflicting world LOD retirement {}",
                    retirement.column_key
                ),
            ));
        }
        retirements.push(WorldLodColumnRetirement {
            column_key: retirement.column_key,
            column_generation: retirement.column_generation,
        });
    }
    let raw_provenance = read_limited_slice(
        request.material_provenance,
        true,
        "world LOD material provenance columns",
    )?;
    if raw_provenance.len() > WORLD_LOD_MAX_COLUMNS {
        return Err(GalError::ffi(
            StatusCode::LengthOverflow,
            format!(
                "world LOD material provenance column count {} exceeds bounded limit {WORLD_LOD_MAX_COLUMNS}",
                raw_provenance.len()
            ),
        ));
    }
    let mut provenance = Vec::with_capacity(raw_provenance.len());
    for column in raw_provenance {
        validate_item_size::<FfiWorldLodColumnMaterialProvenanceRecord>(
            column.byte_size,
            "world LOD material provenance column",
        )?;
        if column.reserved0 != 0 || column.column_generation == 0 {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "world LOD material provenance reserved field and generation must be valid",
            ));
        }
        let raw_identities = read_limited_slice(
            column.identities,
            true,
            "world LOD material provenance identities",
        )?;
        if raw_identities.len() > WORLD_LOD_MAX_MATERIAL_IDENTITIES_PER_COLUMN {
            return Err(GalError::ffi(
                StatusCode::LengthOverflow,
                "world LOD material provenance identity count exceeds ABI maximum",
            ));
        }
        let mut identities = Vec::with_capacity(raw_identities.len());
        for identity in raw_identities {
            validate_item_size::<FfiWorldLodMaterialIdentityRecord>(
                identity.byte_size,
                "world LOD material identity",
            )?;
            if identity.reserved0 != 0 {
                return Err(GalError::ffi(
                    StatusCode::InvalidArgument,
                    "world LOD material identity reserved field must be zero",
                ));
            }
            let block_state_identity = read_label(
                identity.block_state_identity_utf8,
                "world LOD material block-state identity",
            )?;
            let biome_identity = read_label(
                identity.biome_identity_utf8,
                "world LOD material biome identity",
            )?;
            if block_state_identity.is_empty() || biome_identity.is_empty() {
                return Err(GalError::ffi(
                    StatusCode::InvalidArgument,
                    "world LOD material identities must be non-empty",
                ));
            }
            identities.push(WorldLodMaterialIdentity {
                block_state_identity,
                biome_identity,
            });
        }
        let raw_segments = read_limited_slice(
            column.segments,
            true,
            "world LOD segment material provenance",
        )?;
        if raw_segments.len() > WORLD_LOD_MAX_SEGMENTS_PER_COLUMN {
            return Err(GalError::ffi(
                StatusCode::LengthOverflow,
                format!(
                    "world LOD material provenance segment count {} exceeds bounded limit {WORLD_LOD_MAX_SEGMENTS_PER_COLUMN}",
                    raw_segments.len()
                ),
            ));
        }
        let mut segments = Vec::with_capacity(raw_segments.len());
        for segment in raw_segments {
            validate_item_size::<FfiWorldLodSegmentMaterialProvenanceRecord>(
                segment.byte_size,
                "world LOD segment material provenance",
            )?;
            if segment.reserved0 != 0 {
                return Err(GalError::ffi(
                    StatusCode::InvalidArgument,
                    "world LOD segment material provenance reserved field must be zero",
                ));
            }
            let quad_material_ids = read_limited_slice(
                segment.quad_material_ids,
                true,
                "world LOD quad material IDs",
            )?
            .to_vec();
            let quad_variant_states = read_limited_slice(
                segment.quad_variant_states,
                true,
                "world LOD quad variant states",
            )?
            .to_vec();
            let quad_variant_positions = read_limited_slice(
                segment.quad_variant_positions,
                true,
                "world LOD quad variant positions",
            )?
            .to_vec();
            if quad_variant_states.len() != quad_material_ids.len()
                || quad_variant_positions.len() != quad_material_ids.len()
            {
                return Err(GalError::ffi(
                    StatusCode::InvalidArgument,
                    "world LOD quad variant provenance must align with material IDs",
                ));
            }
            segments.push(WorldLodSegmentMaterialProvenance {
                layer: segment.layer,
                segment_index: segment.segment_index,
                quad_material_ids,
                quad_variant_states,
                quad_variant_positions,
            });
        }
        let raw_face_materials = read_limited_slice(
            column.face_materials,
            true,
            "world LOD face material records",
        )?;
        let mut face_materials = Vec::with_capacity(raw_face_materials.len());
        let mut seen_faces = BTreeSet::new();
        for face_material in raw_face_materials {
            validate_item_size::<FfiWorldLodFaceMaterialRecord>(
                face_material.byte_size,
                "world LOD face material",
            )?;
            if face_material.material_id == 0
                || face_material.material_id as usize > identities.len()
                || face_material.face > 5
                || face_material.face_layer & !0x07ff_ffff != 0
            {
                return Err(GalError::ffi(
                    StatusCode::InvalidArgument,
                    "world LOD face material has an invalid identity, face, or layer",
                ));
            }
            let atlas_identity = read_label(
                face_material.atlas_identity_utf8,
                "world LOD face material atlas identity",
            )?;
            let sprite_identity = read_label(
                face_material.sprite_identity_utf8,
                "world LOD face material sprite identity",
            )?;
            let atlas_uv = [
                face_material.u0,
                face_material.v0,
                face_material.u1,
                face_material.v1,
            ];
            if atlas_identity.is_empty()
                || sprite_identity.is_empty()
                || !atlas_uv.iter().all(|value| value.is_finite())
                || atlas_uv[0] < 0.0
                || atlas_uv[1] < 0.0
                || atlas_uv[2] > 1.0
                || atlas_uv[3] > 1.0
                || atlas_uv[0] >= atlas_uv[2]
                || atlas_uv[1] >= atlas_uv[3]
                || !valid_world_lod_uv_corner_order(face_material.uv_corner_order)
                || !seen_faces.insert((
                    face_material.material_id,
                    face_material.face,
                    face_material.face_layer & 0x3,
                    face_material.variant_position,
                ))
            {
                return Err(GalError::ffi(
                    StatusCode::InvalidArgument,
                    "world LOD face material must be a unique normalized atlas region",
                ));
            }
            face_materials.push(WorldLodFaceMaterial {
                material_id: face_material.material_id,
                face: face_material.face,
                face_layer: face_material.face_layer & 0x3,
                atlas_identity,
                sprite_identity,
                atlas_uv,
                uv_corner_order: face_material.uv_corner_order,
                tinted: face_material.face_layer & 0x4 != 0,
                tint_rgb: [
                    ((face_material.face_layer >> 3) & 0xff) as f32 / 255.0,
                    ((face_material.face_layer >> 11) & 0xff) as f32 / 255.0,
                    ((face_material.face_layer >> 19) & 0xff) as f32 / 255.0,
                ],
                variant_position: face_material.variant_position,
            });
        }
        provenance.push(WorldLodColumnMaterialProvenance {
            column_key: column.column_key,
            column_generation: column.column_generation,
            identities,
            segments,
            face_materials,
        });
    }
    Ok((request.generation, assets, retirements, provenance))
}

fn valid_world_lod_uv_corner_order(order: u32) -> bool {
    if order & !0xff != 0 {
        return false;
    }
    let mut seen = 0u8;
    for index in 0..4 {
        let corner = ((order >> (index * 2)) & 0x3) as u8;
        let bit = 1u8 << corner;
        if seen & bit != 0 {
            return false;
        }
        seen |= bit;
    }
    seen == 0x0f
}

pub(crate) fn merge_particle_semantics(
    materials: Vec<WorldMaterialQuadRequest>,
    particles: &[FfiWorldParticleQuadRequest],
    viewport: [u32; 2],
) -> GalResult<Vec<WorldMaterialQuadRequest>> {
    let count = materials.len().checked_add(particles.len())
        .filter(|&n| n <= FFI_MAX_BATCH_ITEMS)
        .ok_or_else(|| GalError::invalid_argument("combined particle/material frame bound exceeded"))?;
    let material_count = materials.len();
    let mut source = materials.into_iter();
    let mut output = Vec::with_capacity(count);
    let mut cursor = 0;
    for p in particles {
        validate_item_size::<FfiWorldParticleQuadRequest>(p.byte_size, "particle semantics")?;
        let index = p.material_index as usize;
        if index < cursor || index > material_count {
            return Err(GalError::invalid_argument("invalid particle surface or material ordering"));
        }
        let surface = crate::render::vulkanic::world_primitive_frontend::particle::ParticleSurface::from_wire(p.surface_kind)?;
        let quad = crate::render::vulkanic::world_primitive_frontend::particle::ParticleQuad {
            center: p.center, rotation: p.rotation, size: p.size, uv_bounds: p.uv_bounds,
            texture_id: p.texture_id, translucent: surface.translucent(),
            color_argb: p.color_argb, packed_light: p.packed_light,
        };
        let quad = quad.lower_surface(surface, viewport)?;
        output.extend(source.by_ref().take(index-cursor));
        cursor = index;
        output.push(quad);
    }
    output.extend(source);
    Ok(output)
}

pub(crate) unsafe fn decode_whole_frame_submit(
    request: *const FfiWholeFrameSubmitRequest,
    capabilities: BackendCapabilities,
) -> GalResult<(u64, Handle, WorldPrimitiveFrame, Vec<GuiSpriteRequest>)> {
    let (generation, frame_target, frame, gui_sprites, _, _, _, _, _) =
        decode_whole_frame_submit_with_gui(request, capabilities)?;
    Ok((generation, frame_target, frame, gui_sprites))
}

pub(crate) unsafe fn decode_whole_frame_submit_with_gui(
    request: *const FfiWholeFrameSubmitRequest,
    capabilities: BackendCapabilities,
) -> GalResult<(
    u64,
    Handle,
    WorldPrimitiveFrame,
    Vec<GuiSpriteRequest>,
    Vec<GuiAffineQuadRequest>,
    Vec<GuiMeshBatchRequest>,
    i32,
    i32,
    Vec<u8>,
)> {
    let (generation, target, frame, sprites, affine, meshes, boundary, radius, effect, tiles) =
        decode_whole_frame_submit_with_backend_policy(request, capabilities, true)?;
    if !tiles.is_empty() {
        return Err(GalError::invalid_argument("tiled GUI requires the typed whole-frame submit path"));
    }
    Ok((generation, target, frame, sprites, affine, meshes, boundary, radius, effect))
}

pub(crate) unsafe fn decode_whole_frame_submit_with_tiled_gui(
    request: *const FfiWholeFrameSubmitRequest, capabilities: BackendCapabilities,
) -> GalResult<(u64, Handle, WorldPrimitiveFrame, Vec<GuiSpriteRequest>,
    Vec<GuiAffineQuadRequest>, Vec<GuiMeshBatchRequest>, i32, i32, Vec<u8>, Vec<GuiTiledQuadRequest>)> {
    decode_whole_frame_submit_with_backend_policy(request, capabilities, true)
}

pub(crate) unsafe fn decode_world_primitive_submit(
    request: *const FfiWholeFrameSubmitRequest,
    capabilities: BackendCapabilities,
) -> GalResult<(u64, Handle, WorldPrimitiveFrame)> {
    let (
        generation,
        frame_target,
        frame,
        gui_sprites,
        gui_affine_quads,
        gui_mesh_batches,
        gui_blur_before_stratum,
        gui_blur_radius,
        _post_effect_id,
        gui_tiled_quads,
    ) = decode_whole_frame_submit_with_backend_policy(request, capabilities, false)?;
    if !gui_sprites.is_empty() || !gui_affine_quads.is_empty() || !gui_mesh_batches.is_empty() || !gui_tiled_quads.is_empty() {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "world primitive submit does not accept GUI work",
        ));
    }
    if gui_blur_before_stratum >= 0 {
        return Err(GalError::unsupported_feature(
            "GUI blur is unavailable on the world-only submit route",
        ));
    }
    if gui_blur_radius >= 0 && gui_blur_radius > 64 {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "GUI blur radius must be within the bounded range 0..=64",
        ));
    }
    Ok((generation, frame_target, frame))
}

unsafe fn read_whole_frame_request(request: *const FfiWholeFrameSubmitRequest)
    -> GalResult<FfiWholeFrameSubmitRequest> {
    if request.is_null() {
        return Err(GalError::invalid_argument("whole-frame submit request is null"));
    }
    let header = read_struct(request.cast::<FfiHeader>(), "whole-frame header")?;
    validate_header::<FfiWholeFrameSubmitRequest>(header)?;
    read_struct(request, "whole-frame submit request")
}

pub(crate) fn merge_experience_orb_instances(
    meshes: Vec<WorldMeshInstanceRequest>,
    orbs: &[FfiWorldExperienceOrbInstanceRecord],
    viewport: [u32; 2],
) -> GalResult<Vec<WorldMeshInstanceRequest>> {
    let count = meshes.len().checked_add(orbs.len())
        .filter(|&count| count <= FFI_MAX_BATCH_ITEMS)
        .ok_or_else(|| GalError::invalid_argument("combined mesh/orb frame bound exceeded"))?;
    let mesh_count = meshes.len();
    let mut source = meshes.into_iter();
    let mut output = Vec::with_capacity(count);
    let mut cursor = 0;
    for orb in orbs {
        validate_item_size::<FfiWorldExperienceOrbInstanceRecord>(orb.byte_size, "orb placement")?;
        let index = orb.mesh_index as usize;
        if orb.reserved0 != 0 || index < cursor || index > mesh_count {
            return Err(GalError::invalid_argument("invalid orb placement ordering or reserved bits"));
        }
        let instance = crate::render::vulkanic::world_primitive_frontend::experience_orb::ExperienceOrbPlacement {
            entity_transform: orb.entity_transform,
            camera_orientation: orb.camera_orientation,
            entity_id: orb.entity_id,
        }.instance(orb.mesh_key, orb.mesh_generation, viewport)?;
        output.extend(source.by_ref().take(index-cursor));
        cursor = index;
        output.push(instance);
    }
    output.extend(source);
    Ok(output)
}

pub(crate) unsafe fn decode_whole_frame_submit_with_backend_policy(
    request: *const FfiWholeFrameSubmitRequest,
    capabilities: BackendCapabilities,
    require_vulkan_whole_frame: bool,
) -> GalResult<(
    u64,
    Handle,
    WorldPrimitiveFrame,
    Vec<GuiSpriteRequest>,
    Vec<GuiAffineQuadRequest>,
    Vec<GuiMeshBatchRequest>,
    i32,
    i32,
    Vec<u8>,
    Vec<GuiTiledQuadRequest>,
)> {
    if request.is_null() {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "whole-frame submit request is null",
        ));
    }
    let request = read_whole_frame_request(request)?;
    let input_bytes = input_bytes_for_whole_frame(&request);
    if input_bytes > FFI_MAX_WHOLE_FRAME_INPUT_BYTES {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!(
                "whole-frame semantic request exceeds bounded input size {} bytes (got {})",
                FFI_MAX_WHOLE_FRAME_INPUT_BYTES, input_bytes
            ),
        ));
    }
    reject_unknown_feature_bits(request.negotiated_feature_bits)?;
    let supported = capability_feature_bits(capabilities);
    if request.negotiated_feature_bits & !supported != 0 {
        return Err(GalError::unsupported_feature(format!(
            "requested unsupported whole-frame feature bits 0x{:x}",
            request.negotiated_feature_bits & !supported
        )));
    }
    if require_vulkan_whole_frame && capabilities.api != BackendApi::Vulkan {
        return Err(GalError::unsupported_feature(
            "whole-frame world primitive submit requires the Rust Vulkan backend",
        ));
    }
    if request.generation == 0 || request.frame_id == 0 || request.correlation_id == 0 {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "whole-frame submit requires non-zero generation, frame id, and correlation id",
        ));
    }
    if request.gui_width <= 0
        || request.gui_height <= 0
        || request.viewport_width <= 0
        || request.viewport_height <= 0
    {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "whole-frame submit requires positive GUI and viewport dimensions",
        ));
    }
    if request.gui_width > GUI_MAX_VIEWPORT_AXIS
        || request.gui_height > GUI_MAX_VIEWPORT_AXIS
        || request.viewport_width > GUI_MAX_VIEWPORT_AXIS
        || request.viewport_height > GUI_MAX_VIEWPORT_AXIS
    {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!(
                "whole-frame GUI and viewport dimensions exceed bounded axis {GUI_MAX_VIEWPORT_AXIS}"
            ),
        ));
    }
    if request.gui_blur_before_stratum < -1 {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "GUI blur boundary must be -1 or a non-negative source stratum index",
        ));
    }
    if request.gui_blur_radius < -1 || request.gui_blur_radius > 64 {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "GUI blur radius must be -1 or within the bounded range 0..=64",
        ));
    }
    let post_effect_id = read_bounded_bytes(
        request.post_effect_id,
        true,
        256,
        "whole-frame post-effect id",
    )?;
    if !post_effect_id.is_empty() {
        let id = std::str::from_utf8(&post_effect_id).map_err(|_| {
            GalError::ffi(
                StatusCode::InvalidArgument,
                "whole-frame post-effect id must be UTF-8",
            )
        })?;
        if id.trim().is_empty() || id.chars().any(char::is_control) {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "whole-frame post-effect id is empty or contains control characters",
            ));
        }
    }
    let frame_target = Handle::from(request.frame_target);
    if frame_target.is_null() || frame_target.kind() != Some(HandleKind::FrameTarget) {
        return Err(GalError::ffi(
            StatusCode::WrongHandleType,
            "whole-frame submit requires a frame-target handle",
        ));
    }
    for value in request
        .view_matrix
        .iter()
        .chain(request.projection_matrix.iter())
    {
        if !value.is_finite() {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "whole-frame matrices must contain finite values",
            ));
        }
    }
    let voxel_volume = decode_world_voxel_volume_frame(request.voxel_volume_frame)?;
    let shader_environment =
        decode_world_shader_environment_frame(request.shader_environment_frame)?;
    let feature_coverage = decode_world_feature_coverage(request.world_feature_coverage)?;
    let background = decode_world_background_request(
        request.world_background,
        request.viewport_width,
        request.viewport_height,
    )?;
    let raw_segments = read_slice(
        request.world_segments,
        true,
        "world primitive line segments",
    )?;
    if raw_segments.len() > FFI_MAX_BATCH_ITEMS {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!(
                "world primitive segment count {} exceeds max {}",
                raw_segments.len(),
                FFI_MAX_BATCH_ITEMS
            ),
        ));
    }
    let mut segments = Vec::with_capacity(raw_segments.len());
    for segment in raw_segments {
        validate_item_size::<FfiWorldLineSegmentRequest>(
            segment.byte_size,
            "world primitive line segment",
        )?;
        let viewport_width = decode_world_viewport_axis(
            segment.viewport_width,
            "world primitive segment viewport width",
        )?;
        let viewport_height = decode_world_viewport_axis(
            segment.viewport_height,
            "world primitive segment viewport height",
        )?;
        segments.push(WorldLineSegmentRequest {
            stratum: segment.stratum,
            style: segment.style,
            depth_policy: segment.depth_policy,
            color_argb: segment.color_argb,
            line_width: segment.line_width,
            start: [segment.start_x, segment.start_y, segment.start_z],
            end: [segment.end_x, segment.end_y, segment.end_z],
            viewport_width,
            viewport_height,
        });
    }
    let raw_cracks = read_slice(
        request.world_crack_quads,
        true,
        "world primitive crack quads",
    )?;
    if raw_cracks.len() > FFI_MAX_BATCH_ITEMS {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!(
                "world primitive crack quad count {} exceeds max {}",
                raw_cracks.len(),
                FFI_MAX_BATCH_ITEMS
            ),
        ));
    }
    let mut crack_quads = Vec::with_capacity(raw_cracks.len());
    for quad in raw_cracks {
        validate_item_size::<FfiWorldCrackQuadRequest>(
            quad.byte_size,
            "world primitive crack quad",
        )?;
        if quad.blend_policy != 1 {
            return Err(GalError::ffi(
                StatusCode::UnknownEnum,
                format!("unknown world crack blend policy {}", quad.blend_policy),
            ));
        }
        if quad.cull_policy != 0 {
            return Err(GalError::ffi(
                StatusCode::UnknownEnum,
                format!("unknown world crack cull policy {}", quad.cull_policy),
            ));
        }
        let viewport_width =
            decode_world_viewport_axis(quad.viewport_width, "world crack quad viewport width")?;
        let viewport_height =
            decode_world_viewport_axis(quad.viewport_height, "world crack quad viewport height")?;
        crack_quads.push(WorldCrackQuadRequest {
            stratum: quad.stratum,
            stage: quad.stage,
            depth_policy: quad.depth_policy,
            color_argb: quad.color_argb,
            vertices: [
                [quad.p0_x, quad.p0_y, quad.p0_z],
                [quad.p1_x, quad.p1_y, quad.p1_z],
                [quad.p2_x, quad.p2_y, quad.p2_z],
                [quad.p3_x, quad.p3_y, quad.p3_z],
            ],
            viewport_width,
            viewport_height,
        });
    }
    let raw_borders = read_slice(
        request.world_border_quads,
        true,
        "world primitive border quads",
    )?;
    if raw_borders.len() > FFI_MAX_BATCH_ITEMS {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!(
                "world primitive border quad count {} exceeds max {}",
                raw_borders.len(),
                FFI_MAX_BATCH_ITEMS
            ),
        ));
    }
    let mut border_quads = Vec::with_capacity(raw_borders.len());
    for quad in raw_borders {
        validate_item_size::<FfiWorldBorderQuadRequest>(
            quad.byte_size,
            "world primitive border quad",
        )?;
        if quad.texture_id != 1 {
            return Err(GalError::ffi(
                StatusCode::UnknownEnum,
                format!("unknown world border texture id {}", quad.texture_id),
            ));
        }
        if quad.blend_policy != 1 {
            return Err(GalError::ffi(
                StatusCode::UnknownEnum,
                format!("unknown world border blend policy {}", quad.blend_policy),
            ));
        }
        if quad.cull_policy != 0 {
            return Err(GalError::ffi(
                StatusCode::UnknownEnum,
                format!("unknown world border cull policy {}", quad.cull_policy),
            ));
        }
        let viewport_width =
            decode_world_viewport_axis(quad.viewport_width, "world border quad viewport width")?;
        let viewport_height =
            decode_world_viewport_axis(quad.viewport_height, "world border quad viewport height")?;
        border_quads.push(WorldBorderQuadRequest {
            stratum: quad.stratum,
            texture_id: quad.texture_id,
            depth_policy: quad.depth_policy,
            blend_policy: quad.blend_policy,
            cull_policy: quad.cull_policy,
            color_argb: quad.color_argb,
            border_size: quad.border_size,
            distance_to_border: quad.distance_to_border,
            scroll: [quad.scroll_u, quad.scroll_v],
            uv_region: [quad.uv_u, quad.uv_v, quad.uv_width, quad.uv_height],
            vertices: [
                [quad.p0_x, quad.p0_y, quad.p0_z],
                [quad.p1_x, quad.p1_y, quad.p1_z],
                [quad.p2_x, quad.p2_y, quad.p2_z],
                [quad.p3_x, quad.p3_y, quad.p3_z],
            ],
            viewport_width,
            viewport_height,
        });
    }
    if request.world_particle_quads.count > FFI_MAX_BATCH_ITEMS as u64 {
        return Err(GalError::invalid_argument("particle semantic count exceeds frame bound"));
    }
    let raw_particles = read_slice(request.world_particle_quads, true, "world particle semantics")?;
    let raw_materials = read_slice(
        request.world_material_quads,
        true,
        "world primitive material quads",
    )?;
    let raw_material_table = read_slice(
        request.world_material_table,
        true,
        "world primitive compact material table",
    )?;
    let raw_compact_materials = read_slice(
        request.world_material_compact_quads,
        true,
        "world primitive compact material quads",
    )?;
    if raw_materials.len() > FFI_MAX_BATCH_ITEMS {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!(
                "world primitive material quad count {} exceeds max {}",
                raw_materials.len(),
                FFI_MAX_BATCH_ITEMS
            ),
        ));
    }
    if raw_material_table.len() > FFI_MAX_BATCH_ITEMS {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!(
                "world primitive compact material table count {} exceeds max {}",
                raw_material_table.len(),
                FFI_MAX_BATCH_ITEMS
            ),
        ));
    }
    if raw_compact_materials.len() > FFI_MAX_BATCH_ITEMS {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!(
                "world primitive compact material quad count {} exceeds max {}",
                raw_compact_materials.len(),
                FFI_MAX_BATCH_ITEMS
            ),
        ));
    }
    let material_quad_count = raw_materials
        .len()
        .checked_add(raw_compact_materials.len())
        .and_then(|count| count.checked_add(raw_particles.len()))
        .ok_or_else(|| {
            GalError::ffi(
                StatusCode::LengthOverflow,
                "world material quad count overflows",
            )
        })?;
    if material_quad_count > FFI_MAX_BATCH_ITEMS {
        return Err(GalError::ffi(
            StatusCode::LengthOverflow,
            format!(
                "world material quad count {} exceeds max {}",
                material_quad_count, FFI_MAX_BATCH_ITEMS
            ),
        ));
    }
    if raw_material_table.is_empty() && !raw_compact_materials.is_empty() {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "compact world material quads require a non-empty material table",
        ));
    }
    if !raw_material_table.is_empty() && raw_compact_materials.is_empty() {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "compact world material table requires compact quad records",
        ));
    }
    // Full records carry the exceptional per-vertex modulation path while
    // compact records remain the normal steady-state representation. A frame
    // may contain both; each form is decoded and validated independently.
    let mut material_quads = Vec::with_capacity(raw_materials.len() + raw_compact_materials.len());
    for quad in raw_materials {
        validate_item_size::<FfiWorldMaterialQuadRequest>(
            quad.byte_size,
            "world primitive material quad",
        )?;
        if quad.stratum != WORLD_STRATUM_OPAQUE_TEXTURED_GEOMETRY {
            return Err(GalError::ffi(
                StatusCode::UnknownEnum,
                format!("unknown world material stratum {}", quad.stratum),
            ));
        }
        let material_id = world_material_semantics::canonical_material_id(quad.material_id)
            .ok_or_else(|| {
                GalError::ffi(
                    StatusCode::UnknownEnum,
                    format!("unknown world material id {}", quad.material_id),
                )
            })?;
        let texture_id = world_material_semantics::canonical_texture_id(quad.texture_id)
            .ok_or_else(|| {
                GalError::ffi(
                    StatusCode::UnknownEnum,
                    format!(
                        "unknown legacy world material quad texture id {}",
                        quad.texture_id
                    ),
                )
            })?;
        if quad.material_mode != WORLD_MATERIAL_MODE_OPAQUE
            && quad.material_mode != WORLD_MATERIAL_MODE_CUTOUT
            && quad.material_mode != WORLD_MATERIAL_MODE_TRANSLUCENT
        {
            return Err(GalError::ffi(
                StatusCode::UnknownEnum,
                format!("unknown world material mode {}", quad.material_mode),
            ));
        }
        if !world_material_semantics::material_matches_mode(material_id, quad.material_mode) {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                format!(
                    "world material id {} is incompatible with mode {}",
                    quad.material_id, quad.material_mode
                ),
            ));
        }
        if quad.depth_policy != WORLD_DEPTH_POLICY_DISABLED
            && quad.depth_policy != WORLD_DEPTH_POLICY_TEST_WRITE
            && quad.depth_policy != WORLD_DEPTH_POLICY_TEST_NO_WRITE
        {
            return Err(GalError::ffi(
                StatusCode::UnknownEnum,
                format!("unknown world material depth policy {}", quad.depth_policy),
            ));
        }
        if quad.cull_policy != WORLD_CULL_NONE
            && quad.cull_policy != WORLD_CULL_BACK
            && quad.cull_policy != WORLD_CULL_FRONT
        {
            return Err(GalError::ffi(
                StatusCode::UnknownEnum,
                format!("unknown world material cull policy {}", quad.cull_policy),
            ));
        }
        if quad.topology != WORLD_TOPOLOGY_TRIANGLES {
            return Err(GalError::ffi(
                StatusCode::UnknownEnum,
                format!("unknown world material topology {}", quad.topology),
            ));
        }
        let winding = if quad.winding == 0 {
            WORLD_WINDING_CCW
        } else {
            quad.winding
        };
        if winding != WORLD_WINDING_CCW && winding != WORLD_WINDING_CW {
            return Err(GalError::ffi(
                StatusCode::UnknownEnum,
                format!("unknown world material winding {}", winding),
            ));
        }
        if !matches!(
            quad.source_program,
            WORLD_MATERIAL_SOURCE_UNSPECIFIED
                | WORLD_MATERIAL_SOURCE_TEXTURED
                | WORLD_MATERIAL_SOURCE_ENTITY_MODEL
                | WORLD_MATERIAL_SOURCE_PARTICLES
                | WORLD_MATERIAL_SOURCE_WEATHER
                | WORLD_MATERIAL_SOURCE_CLOUDS
        ) {
            return Err(GalError::ffi(
                StatusCode::UnknownEnum,
                format!(
                    "unknown world material source program {}",
                    quad.source_program
                ),
            ));
        }
        if !matches!(
            quad.source_uv_space,
            WORLD_MATERIAL_SOURCE_UV_LOCAL_TEXTURE | WORLD_MATERIAL_SOURCE_UV_MINECRAFT_BLOCK_ATLAS
        ) {
            return Err(GalError::ffi(
                StatusCode::UnknownEnum,
                format!(
                    "unknown world material source UV space {}",
                    quad.source_uv_space
                ),
            ));
        }
        if !world_material_semantics::source_program_supports_uv_space(
            quad.source_program,
            quad.source_uv_space,
        ) {
            return Err(GalError::unsupported_feature(
                "weather and cloud material quads require Rust-owned local texture UV semantics",
            ));
        }
        if !world_material_semantics::texture_supports_uv_space(texture_id, quad.source_uv_space) {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                format!(
                    "world material texture {} requires Minecraft atlas UV semantics",
                    quad.texture_id
                ),
            ));
        }
        let viewport_width =
            decode_world_viewport_axis(quad.viewport_width, "world material quad viewport width")?;
        let viewport_height = decode_world_viewport_axis(
            quad.viewport_height,
            "world material quad viewport height",
        )?;
        material_quads.push(WorldMaterialQuadRequest {
            stratum: quad.stratum,
            material_id,
            texture_id,
            material_mode: quad.material_mode,
            depth_policy: quad.depth_policy,
            cull_policy: quad.cull_policy,
            topology: quad.topology,
            winding,
            color_argb: quad.color_argb,
            vertices: [
                [quad.p0_x, quad.p0_y, quad.p0_z],
                [quad.p1_x, quad.p1_y, quad.p1_z],
                [quad.p2_x, quad.p2_y, quad.p2_z],
                [quad.p3_x, quad.p3_y, quad.p3_z],
            ],
            uvs: [
                [quad.uv0_u, quad.uv0_v],
                [quad.uv1_u, quad.uv1_v],
                [quad.uv2_u, quad.uv2_v],
                [quad.uv3_u, quad.uv3_v],
            ],
            viewport_width,
            viewport_height,
            source_program: quad.source_program,
            source_uv_space: quad.source_uv_space,
            source_color_argb: quad.source_color_argb,
            packed_light: quad.packed_light,
            vertex_color_argb: [
                quad.vertex0_color_argb,
                quad.vertex1_color_argb,
                quad.vertex2_color_argb,
                quad.vertex3_color_argb,
            ],
            vertex_packed_light: [
                quad.vertex0_packed_light,
                quad.vertex1_packed_light,
                quad.vertex2_packed_light,
                quad.vertex3_packed_light,
            ],
            block_entity_id: quad.block_entity_id,
        });
    }
    if !raw_compact_materials.is_empty() {
        let viewport_width_for_materials = u32::try_from(request.viewport_width).map_err(|_| {
            GalError::ffi(
                StatusCode::InvalidArgument,
                format!(
                    "whole-frame viewport width must be non-negative, got {}",
                    request.viewport_width
                ),
            )
        })?;
        let viewport_height_for_materials =
            u32::try_from(request.viewport_height).map_err(|_| {
                GalError::ffi(
                    StatusCode::InvalidArgument,
                    format!(
                        "whole-frame viewport height must be non-negative, got {}",
                        request.viewport_height
                    ),
                )
            })?;
        let mut table = Vec::with_capacity(raw_material_table.len());
        for record in raw_material_table {
            validate_item_size::<FfiWorldMaterialTableRecord>(
                record.byte_size,
                "world primitive compact material table record",
            )?;
            if record.stratum != WORLD_STRATUM_OPAQUE_TEXTURED_GEOMETRY {
                return Err(GalError::ffi(
                    StatusCode::UnknownEnum,
                    format!("unknown compact world material stratum {}", record.stratum),
                ));
            }
            let material_id = world_material_semantics::canonical_material_id(record.material_id)
                .ok_or_else(|| {
                GalError::ffi(
                    StatusCode::UnknownEnum,
                    format!("unknown compact world material id {}", record.material_id),
                )
            })?;
            let texture_id = world_material_semantics::canonical_texture_id(record.texture_id)
                .ok_or_else(|| {
                    GalError::ffi(
                        StatusCode::UnknownEnum,
                        format!(
                            "unknown compact world material texture id {}",
                            record.texture_id
                        ),
                    )
                })?;
            if record.material_mode != WORLD_MATERIAL_MODE_OPAQUE
                && record.material_mode != WORLD_MATERIAL_MODE_CUTOUT
                && record.material_mode != WORLD_MATERIAL_MODE_TRANSLUCENT
            {
                return Err(GalError::ffi(
                    StatusCode::UnknownEnum,
                    format!(
                        "unknown compact world material mode {}",
                        record.material_mode
                    ),
                ));
            }
            if !world_material_semantics::material_matches_mode(material_id, record.material_mode) {
                return Err(GalError::ffi(
                    StatusCode::InvalidArgument,
                    format!(
                        "compact world material id {} is incompatible with mode {}",
                        record.material_id, record.material_mode
                    ),
                ));
            }
            if record.depth_policy != WORLD_DEPTH_POLICY_DISABLED
                && record.depth_policy != WORLD_DEPTH_POLICY_TEST_WRITE
                && record.depth_policy != WORLD_DEPTH_POLICY_TEST_NO_WRITE
            {
                return Err(GalError::ffi(
                    StatusCode::UnknownEnum,
                    format!(
                        "unknown compact world material depth policy {}",
                        record.depth_policy
                    ),
                ));
            }
            if record.cull_policy != WORLD_CULL_NONE
                && record.cull_policy != WORLD_CULL_BACK
                && record.cull_policy != WORLD_CULL_FRONT
            {
                return Err(GalError::ffi(
                    StatusCode::UnknownEnum,
                    format!(
                        "unknown compact world material cull policy {}",
                        record.cull_policy
                    ),
                ));
            }
            if record.topology != WORLD_TOPOLOGY_TRIANGLES {
                return Err(GalError::ffi(
                    StatusCode::UnknownEnum,
                    format!(
                        "unknown compact world material topology {}",
                        record.topology
                    ),
                ));
            }
            if record.winding != WORLD_WINDING_CCW && record.winding != WORLD_WINDING_CW {
                return Err(GalError::ffi(
                    StatusCode::UnknownEnum,
                    format!("unknown compact world material winding {}", record.winding),
                ));
            }
            if !matches!(
                record.source_program,
                WORLD_MATERIAL_SOURCE_UNSPECIFIED
                    | WORLD_MATERIAL_SOURCE_TEXTURED
                    | WORLD_MATERIAL_SOURCE_ENTITY_MODEL
                    | WORLD_MATERIAL_SOURCE_PARTICLES
                    | WORLD_MATERIAL_SOURCE_WEATHER
                    | WORLD_MATERIAL_SOURCE_CLOUDS
            ) {
                return Err(GalError::ffi(
                    StatusCode::UnknownEnum,
                    format!(
                        "unknown compact world material source program {}",
                        record.source_program
                    ),
                ));
            }
            let mut canonical_record = *record;
            canonical_record.material_id = material_id;
            canonical_record.texture_id = texture_id;
            table.push(canonical_record);
        }
        material_quads.reserve(raw_compact_materials.len());
        for quad in raw_compact_materials {
            validate_item_size::<FfiWorldMaterialCompactQuadRequest>(
                quad.byte_size,
                "world primitive compact material quad",
            )?;
            let material_index = usize::try_from(quad.material_index).map_err(|_| {
                GalError::ffi(
                    StatusCode::InvalidArgument,
                    format!(
                        "compact world material index {} overflowed usize",
                        quad.material_index
                    ),
                )
            })?;
            let Some(key) = table.get(material_index) else {
                return Err(GalError::ffi(
                    StatusCode::InvalidArgument,
                    format!(
                        "compact world material quad references table index {} but table has {} entries",
                        quad.material_index,
                        table.len()
                    ),
                ));
            };
            if !matches!(
                quad.source_uv_space,
                WORLD_MATERIAL_SOURCE_UV_LOCAL_TEXTURE
                    | WORLD_MATERIAL_SOURCE_UV_MINECRAFT_BLOCK_ATLAS
            ) {
                return Err(GalError::ffi(
                    StatusCode::UnknownEnum,
                    format!(
                        "unknown compact world material source UV space {}",
                        quad.source_uv_space
                    ),
                ));
            }
            if !world_material_semantics::source_program_supports_uv_space(
                key.source_program,
                quad.source_uv_space,
            ) {
                return Err(GalError::unsupported_feature(
                    "weather and cloud material quads require Rust-owned local texture UV semantics",
                ));
            }
            if !world_material_semantics::texture_supports_uv_space(
                key.texture_id,
                quad.source_uv_space,
            ) {
                return Err(GalError::ffi(
                    StatusCode::InvalidArgument,
                    format!(
                        "compact world material texture {} requires Minecraft atlas UV semantics",
                        key.texture_id
                    ),
                ));
            }
            if quad.block_entity_id < -1 {
                return Err(GalError::ffi(
                    StatusCode::InvalidArgument,
                    format!(
                        "compact world material quad block entity id must be >= -1, got {}",
                        quad.block_entity_id
                    ),
                ));
            }
            material_quads.push(WorldMaterialQuadRequest {
                stratum: key.stratum,
                material_id: key.material_id,
                texture_id: key.texture_id,
                material_mode: key.material_mode,
                depth_policy: key.depth_policy,
                cull_policy: key.cull_policy,
                topology: key.topology,
                winding: key.winding,
                color_argb: quad.color_argb,
                vertices: [
                    [quad.p0_x, quad.p0_y, quad.p0_z],
                    [quad.p1_x, quad.p1_y, quad.p1_z],
                    [quad.p2_x, quad.p2_y, quad.p2_z],
                    [quad.p3_x, quad.p3_y, quad.p3_z],
                ],
                uvs: [
                    [quad.uv0_u, quad.uv0_v],
                    [quad.uv1_u, quad.uv1_v],
                    [quad.uv2_u, quad.uv2_v],
                    [quad.uv3_u, quad.uv3_v],
                ],
                viewport_width: viewport_width_for_materials,
                viewport_height: viewport_height_for_materials,
                source_program: key.source_program,
                source_uv_space: quad.source_uv_space,
                source_color_argb: quad.source_color_argb,
                packed_light: quad.packed_light,
                vertex_color_argb: [quad.source_color_argb; 4],
                vertex_packed_light: [quad.packed_light; 4],
                block_entity_id: quad.block_entity_id,
            });
        }
    }
    if !raw_particles.is_empty() {
        material_quads = merge_particle_semantics(material_quads, raw_particles,
            [decode_world_viewport_axis(request.viewport_width, "particle viewport width")?,
             decode_world_viewport_axis(request.viewport_height, "particle viewport height")?])?;
    }
    let raw_mesh_instances = read_slice(
        request.world_mesh_instances,
        true,
        "world primitive mesh instances",
    )?;
    if raw_mesh_instances.len() > FFI_MAX_BATCH_ITEMS {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!(
                "world primitive mesh instance count {} exceeds max {}",
                raw_mesh_instances.len(),
                FFI_MAX_BATCH_ITEMS
            ),
        ));
    }
    if request.world_experience_orbs.count > (FFI_MAX_BATCH_ITEMS - raw_mesh_instances.len()) as u64 {
        return Err(GalError::invalid_argument("combined mesh/orb frame bound exceeded"));
    }
    let raw_orbs = read_slice(request.world_experience_orbs, true, "orb placements")?;
    let mut mesh_instances = Vec::with_capacity(raw_mesh_instances.len());
    for instance in raw_mesh_instances {
        validate_item_size::<FfiWorldMeshInstanceRecord>(
            instance.byte_size,
            "world primitive mesh instance",
        )?;
        validate_mesh_instance_semantic_identity(instance, "world primitive mesh instance")?;
        if !is_world_mesh_stratum(instance.stratum) {
            return Err(GalError::ffi(
                StatusCode::UnknownEnum,
                format!("unknown world mesh stratum {}", instance.stratum),
            ));
        }
        if instance.depth_policy != WORLD_DEPTH_POLICY_DISABLED
            && instance.depth_policy != WORLD_DEPTH_POLICY_TEST_WRITE
            && instance.depth_policy != WORLD_DEPTH_POLICY_TEST_NO_WRITE
        {
            return Err(GalError::ffi(
                StatusCode::UnknownEnum,
                format!("unknown world mesh depth policy {}", instance.depth_policy),
            ));
        }
        if instance.cull_policy != WORLD_CULL_NONE
            && instance.cull_policy != WORLD_CULL_BACK
            && instance.cull_policy != WORLD_CULL_FRONT
        {
            return Err(GalError::ffi(
                StatusCode::UnknownEnum,
                format!("unknown world mesh cull policy {}", instance.cull_policy),
            ));
        }
        if instance.winding != WORLD_WINDING_CCW && instance.winding != WORLD_WINDING_CW {
            return Err(GalError::ffi(
                StatusCode::UnknownEnum,
                format!("unknown world mesh winding {}", instance.winding),
            ));
        }
        let viewport_width =
            decode_world_viewport_axis(instance.viewport_width, "world mesh viewport width")?;
        let viewport_height =
            decode_world_viewport_axis(instance.viewport_height, "world mesh viewport height")?;
        mesh_instances.push(WorldMeshInstanceRequest {
            item_foil: decode_world_item_foil(instance)?,
            stratum: instance.stratum,
            mesh_key: instance.mesh_key,
            mesh_generation: instance.mesh_generation,
            mesh_section_index: instance.mesh_section_index,
            depth_policy: instance.depth_policy,
            cull_policy: instance.cull_policy,
            winding: instance.winding,
            color_argb: instance.color_argb,
            entity_id: instance.entity_id,
            entity_color_argb: instance.entity_color_argb,
            transform: decode_mesh_instance_transform(instance)?,
            outline_color_argb: instance.outline_color_argb,
            flags: instance.flags,
            block_entity_id: instance.block_entity_id,
            viewport_width,
            viewport_height,
        });
    }
    if !raw_orbs.is_empty() {
        mesh_instances = merge_experience_orb_instances(mesh_instances, raw_orbs, [
            decode_world_viewport_axis(request.viewport_width, "orb viewport width")?,
            decode_world_viewport_axis(request.viewport_height, "orb viewport height")?,
        ])?;
    }
    let raw_text_quads = read_slice(request.world_text_quads, true, "world text quads")?;
    if raw_text_quads.len() > FFI_MAX_BATCH_ITEMS {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!(
                "world text quad count {} exceeds max {}",
                raw_text_quads.len(),
                FFI_MAX_BATCH_ITEMS
            ),
        ));
    }
    let mut text_quads = Vec::with_capacity(raw_text_quads.len());
    for quad in raw_text_quads {
        validate_item_size::<FfiWorldTextQuadRequest>(quad.byte_size, "world text quad")?;
        if quad.flags & !1 != 0 || quad.reserved0 != 0 {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "world text quad has unknown flags or a non-zero reserved field",
            ));
        }
        if !matches!(
            quad.depth_policy,
            WORLD_TEXT_DEPTH_SEE_THROUGH
                | WORLD_TEXT_DEPTH_NORMAL
                | WORLD_TEXT_DEPTH_POLYGON_OFFSET
        ) {
            return Err(GalError::ffi(
                StatusCode::UnknownEnum,
                format!("unknown world text depth policy {}", quad.depth_policy),
            ));
        }
        if quad.asset_id == 0 || quad.atlas_generation == 0 || quad.atlas_revision == 0 {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "world text quad requires non-zero atlas identity, generation, and revision",
            ));
        }
        if !quad.distance_to_camera_sq.is_finite()
            || quad.distance_to_camera_sq < 0.0
            || quad
                .model_view_matrix
                .iter()
                .chain(quad.positions.iter())
                .chain(quad.uvs.iter())
                .any(|value| !value.is_finite())
        {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "world text quad contains invalid geometry",
            ));
        }
        if quad.block_entity_id < -1 {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                format!(
                    "world text quad block entity id must be >= -1, got {}",
                    quad.block_entity_id
                ),
            ));
        }
        text_quads.push(WorldTextQuadRequest {
            asset_id: quad.asset_id,
            atlas_generation: quad.atlas_generation,
            atlas_revision: quad.atlas_revision,
            colored: quad.flags & 1 != 0,
            depth_policy: quad.depth_policy,
            packed_light: quad.packed_light,
            distance_to_camera_sq: quad.distance_to_camera_sq,
            model_view_matrix: quad.model_view_matrix,
            positions: [
                [quad.positions[0], quad.positions[1], quad.positions[2]],
                [quad.positions[3], quad.positions[4], quad.positions[5]],
                [quad.positions[6], quad.positions[7], quad.positions[8]],
                [quad.positions[9], quad.positions[10], quad.positions[11]],
            ],
            uvs: [
                [quad.uvs[0], quad.uvs[1]],
                [quad.uvs[2], quad.uvs[3]],
                [quad.uvs[4], quad.uvs[5]],
                [quad.uvs[6], quad.uvs[7]],
            ],
            color_argb: quad.color_argb,
            block_entity_id: quad.block_entity_id,
        });
    }
    let raw_lod_instances = read_slice(
        request.world_lod_instances,
        true,
        "world primitive LOD instances",
    )?;
    if raw_lod_instances.len() > WORLD_LOD_MAX_VISIBLE_SEGMENTS {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!(
                "world primitive LOD instance count {} exceeds max {}",
                raw_lod_instances.len(),
                WORLD_LOD_MAX_VISIBLE_SEGMENTS
            ),
        ));
    }
    let mut lod_instances = Vec::with_capacity(raw_lod_instances.len());
    for instance in raw_lod_instances {
        validate_item_size::<FfiWorldLodColumnInstanceRecord>(
            instance.byte_size,
            "world primitive LOD instance",
        )?;
        lod_instances.push(WorldLodColumnInstanceRequest {
            column_key: instance.column_key,
            column_generation: instance.column_generation,
            layer: instance.layer,
            segment_index: instance.segment_index,
            order: instance.order,
        });
    }
    let lod_render_frame = decode_world_lod_render_frame(request.world_lod_render_frame)?;
    let first_person = decode_world_first_person_frame(request.world_first_person_frame)?;
    let first_person_mesh_instances = unsafe {
        decode_world_first_person_mesh_instances(
            request.world_first_person_mesh_instances,
            &first_person,
        )?
    };
    let raw_gui = FfiGuiFrameSubmitRequest {
        header: FfiHeader {
            version: request.header.version,
            byte_size: size_of::<FfiGuiFrameSubmitRequest>() as u32,
        },
        generation: request.generation,
        frame_id: request.frame_id,
        frame_target: request.frame_target,
        gui_width: request.gui_width,
        gui_height: request.gui_height,
        sprites: request.gui_sprites,
        affine_quads: request.gui_affine_quads,
        negotiated_feature_bits: request.negotiated_feature_bits,
        mesh_batches: request.gui_mesh_batches,
        gui_projection_width: request.gui_projection_width,
        gui_projection_height: request.gui_projection_height,
        tiled_quads: request.gui_tiled_quads,
    };
    let (_, _, gui_sprites, gui_affine_quads, gui_mesh_batches, gui_tiled_quads) =
        decode_gui_frame_submit_with_tiles(&raw_gui, capabilities)?;
    let viewport_width = u32::try_from(request.viewport_width).map_err(|_| {
        GalError::ffi(
            StatusCode::InvalidArgument,
            format!(
                "whole-frame viewport width must be non-negative, got {}",
                request.viewport_width
            ),
        )
    })?;
    let viewport_height = u32::try_from(request.viewport_height).map_err(|_| {
        GalError::ffi(
            StatusCode::InvalidArgument,
            format!(
                "whole-frame viewport height must be non-negative, got {}",
                request.viewport_height
            ),
        )
    })?;
    Ok((
        request.generation,
        frame_target,
        WorldPrimitiveFrame {
            engine_globals: match request.engine_globals_present {
                0 => None,
                1 => {
                    let values = crate::render::vulkanic::shader_pack::engine_globals::EngineGlobals {
                        screen_width: request.engine_screen_width,
                        screen_height: request.engine_screen_height,
                        game_ticks: request.engine_game_ticks,
                        partial_tick: request.engine_partial_tick,
                        glint_alpha: request.engine_glint_alpha,
                        menu_blur_radius: request.engine_menu_blur_radius,
                    };
                    values.uniforms()?;
                    Some(values)
                }
                _ => return Err(GalError::invalid_argument("engine Globals presence must be zero or one")),
            },
            frame_id: request.frame_id,
            correlation_id: request.correlation_id,
            viewport_width,
            viewport_height,
            view_matrix: request.view_matrix,
            projection_matrix: request.projection_matrix,
            voxel_volume,
            shader_environment,
            feature_coverage,
            first_person,
            first_person_mesh_instances,
            background,
            segments,
            crack_quads,
            border_quads,
            material_quads,
            mesh_instances,
            text_quads,
            lod_instances,
            lod_render_frame,
        },
        gui_sprites,
        gui_affine_quads,
        gui_mesh_batches,
        request.gui_blur_before_stratum,
        request.gui_blur_radius,
        post_effect_id,
        gui_tiled_quads,
    ))
}

/// Decodes the append-only first-person instance stream. It intentionally
/// reuses the stable indexed-mesh record instead of creating a hand-specific
/// ABI schema, while rejecting camera-space strata before any frontend route
/// can consume the data.
unsafe fn decode_world_first_person_mesh_instances(
    request: FfiSlice<FfiWorldMeshInstanceRecord>,
    first_person: &WorldFirstPersonFrame,
) -> GalResult<Vec<WorldMeshInstanceRequest>> {
    let raw_instances = read_slice(request, true, "world first-person mesh instances")?;
    if raw_instances.len() > FFI_MAX_BATCH_ITEMS {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!(
                "world first-person mesh instance count {} exceeds max {}",
                raw_instances.len(),
                FFI_MAX_BATCH_ITEMS
            ),
        ));
    }
    if !raw_instances.is_empty() && !first_person.enabled {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "world first-person mesh instances require an enabled first-person frame",
        ));
    }
    if usize::try_from(first_person.main_hand_instance_count)
        .ok()
        .is_none_or(|count| count > raw_instances.len())
    {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!(
                "world first-person main-hand instance count {} exceeds {} copied instances",
                first_person.main_hand_instance_count,
                raw_instances.len()
            ),
        ));
    }
    let mut instances = Vec::with_capacity(raw_instances.len());
    for instance in raw_instances {
        validate_item_size::<FfiWorldMeshInstanceRecord>(
            instance.byte_size,
            "world first-person mesh instance",
        )?;
        validate_mesh_instance_semantic_identity(instance, "world first-person mesh instance")?;
        if instance.terrain_placement_mode != 0 {
            return Err(GalError::invalid_argument(
                "first-person mesh instances cannot use terrain placement",
            ));
        }
        if instance.flags & WORLD_MESH_INSTANCE_FLAG_OUTLINE_ONLY != 0 {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "world first-person mesh instances cannot be outline-only",
            ));
        }
        if instance.block_entity_id != -1 {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "world first-person mesh instances cannot carry block-entity identity",
            ));
        }
        if instance.entity_id != 0 {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "world first-person mesh instances cannot carry shader-pack entity identity",
            ));
        }
        if instance.stratum != WORLD_STRATUM_ENTITY_MESH {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "world first-person mesh instances must use the generic entity-mesh semantic stratum",
            ));
        }
        if instance.depth_policy != WORLD_DEPTH_POLICY_DISABLED
            && instance.depth_policy != WORLD_DEPTH_POLICY_TEST_WRITE
            && instance.depth_policy != WORLD_DEPTH_POLICY_TEST_NO_WRITE
        {
            return Err(GalError::ffi(
                StatusCode::UnknownEnum,
                format!(
                    "unknown world first-person mesh depth policy {}",
                    instance.depth_policy
                ),
            ));
        }
        if instance.cull_policy != WORLD_CULL_NONE
            && instance.cull_policy != WORLD_CULL_BACK
            && instance.cull_policy != WORLD_CULL_FRONT
        {
            return Err(GalError::ffi(
                StatusCode::UnknownEnum,
                format!(
                    "unknown world first-person mesh cull policy {}",
                    instance.cull_policy
                ),
            ));
        }
        if instance.winding != WORLD_WINDING_CCW && instance.winding != WORLD_WINDING_CW {
            return Err(GalError::ffi(
                StatusCode::UnknownEnum,
                format!(
                    "unknown world first-person mesh winding {}",
                    instance.winding
                ),
            ));
        }
        let viewport_width = decode_world_viewport_axis(
            instance.viewport_width,
            "world first-person mesh viewport width",
        )?;
        let viewport_height = decode_world_viewport_axis(
            instance.viewport_height,
            "world first-person mesh viewport height",
        )?;
        instances.push(WorldMeshInstanceRequest {
            item_foil: decode_world_item_foil(instance)?,
            stratum: instance.stratum,
            mesh_key: instance.mesh_key,
            mesh_generation: instance.mesh_generation,
            mesh_section_index: instance.mesh_section_index,
            depth_policy: instance.depth_policy,
            cull_policy: instance.cull_policy,
            winding: instance.winding,
            color_argb: instance.color_argb,
            entity_id: instance.entity_id,
            entity_color_argb: instance.entity_color_argb,
            transform: instance.transform,
            outline_color_argb: instance.outline_color_argb,
            flags: instance.flags,
            block_entity_id: instance.block_entity_id,
            viewport_width,
            viewport_height,
        });
    }
    Ok(instances)
}

fn decode_world_first_person_frame(
    request: FfiWorldFirstPersonFrame,
) -> GalResult<WorldFirstPersonFrame> {
    validate_item_size::<FfiWorldFirstPersonFrame>(request.byte_size, "world first-person frame")?;
    if request.enabled > 1 || request.clear_depth_before > 1 {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "world first-person frame has invalid flags",
        ));
    }
    if request.enabled == 0 {
        if request.clear_depth_before != 0
            || request.projection_matrix.iter().any(|value| *value != 0.0)
            || request.model_view_matrix.iter().any(|value| *value != 0.0)
        {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "disabled world first-person frame must be zeroed",
            ));
        }
    } else {
        if request.clear_depth_before == 0 {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "enabled world first-person frame must explicitly clear its depth domain",
            ));
        }
        if request
            .projection_matrix
            .iter()
            .any(|value| !value.is_finite())
            || request.projection_matrix.iter().all(|value| *value == 0.0)
        {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "enabled world first-person frame requires a finite non-zero projection matrix",
            ));
        }
        if request
            .model_view_matrix
            .iter()
            .any(|value| !value.is_finite())
            || request.model_view_matrix.iter().all(|value| *value == 0.0)
        {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "enabled world first-person frame requires a finite non-zero model-view matrix",
            ));
        }
    }
    Ok(WorldFirstPersonFrame {
        enabled: request.enabled != 0,
        clear_depth_before: request.clear_depth_before != 0,
        main_hand_instance_count: request.main_hand_instance_count,
        projection_matrix: request.projection_matrix,
        model_view_matrix: request.model_view_matrix,
    })
}

fn decode_world_feature_coverage(
    request: FfiWorldFeatureCoverage,
) -> GalResult<WorldFeatureCoverageFrame> {
    validate_item_size::<FfiWorldFeatureCoverage>(
        request.byte_size,
        "world feature coverage frame",
    )?;
    Ok(WorldFeatureCoverageFrame {
        model_submits: request.model_submits,
        model_part_submits: request.model_part_submits,
        block_model_submits: request.block_model_submits,
        ordinary_block_submits: request.ordinary_block_submits,
        item_submits: request.item_submits,
        custom_geometry_submits: request.custom_geometry_submits,
        shadow_submits: request.shadow_submits,
        flame_submits: request.flame_submits,
        name_tag_submits: request.name_tag_submits,
        text_submits: request.text_submits,
        hitbox_submits: request.hitbox_submits,
        leash_submits: request.leash_submits,
        particle_group_submits: request.particle_group_submits,
    })
}

fn decode_world_lod_render_frame(
    request: FfiWorldLodRenderFrame,
) -> GalResult<WorldLodRenderFrame> {
    validate_item_size::<FfiWorldLodRenderFrame>(request.byte_size, "world LOD render frame")?;
    if request.reserved0 != 0 {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "world LOD render frame reserved field must be zero",
        ));
    }
    let enabled = bool_flag(request.enabled, "world LOD render frame enabled")?;
    if request.flags & !0x1f != 0 {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!(
                "world LOD render frame has unknown flags 0x{:x}",
                request.flags
            ),
        ));
    }
    let finite = request
        .combined_matrix
        .into_iter()
        .chain(request.model_view_matrix)
        .chain(request.projection_matrix)
        .chain(request.projection_inverse_matrix)
        .chain([
            request.clip_distance,
            request.micro_offset,
            request.noise_intensity,
            request.earth_radius,
        ])
        .chain([
            request.camera_world_x,
            request.camera_world_y,
            request.camera_world_z,
        ])
        .all(f32::is_finite);
    if !finite {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "world LOD render frame contains non-finite scalar or matrix data",
        ));
    }
    let frame = WorldLodRenderFrame {
        enabled,
        flags: request.flags,
        world_y_offset: request.world_y_offset,
        camera_world_position: [
            request.camera_world_x,
            request.camera_world_y,
            request.camera_world_z,
        ],
        combined_matrix: request.combined_matrix,
        model_view_matrix: request.model_view_matrix,
        projection_matrix: request.projection_matrix,
        projection_inverse_matrix: request.projection_inverse_matrix,
        clip_distance: request.clip_distance,
        micro_offset: request.micro_offset,
        noise_intensity: request.noise_intensity,
        earth_radius: request.earth_radius,
        noise_steps: request.noise_steps,
        noise_dropoff: request.noise_dropoff,
    };
    if !enabled && frame != WorldLodRenderFrame::default() {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "disabled world LOD render frame must be zeroed",
        ));
    }
    if enabled && (frame.micro_offset <= 0.0 || frame.clip_distance < 0.0) {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "enabled world LOD render frame has invalid clip or micro-offset semantics",
        ));
    }
    Ok(frame)
}

fn decode_world_shader_environment_frame(
    request: FfiWorldShaderEnvironmentFrame,
) -> GalResult<WorldShaderEnvironmentFrame> {
    validate_item_size::<FfiWorldShaderEnvironmentFrame>(
        request.byte_size,
        "world shader environment frame",
    )?;
    let enabled = bool_flag(request.enabled, "world shader environment frame enabled")?;
    let biome_resource_location = unsafe {
        read_label(
            request.biome_resource_location_utf8,
            "world shader environment biome resource location",
        )
    }?;
    let main_hand_item_model_resource_location = unsafe {
        read_label(
            request.main_hand_item_model_resource_location_utf8,
            "world shader environment main-hand item-model resource location",
        )
    }?;
    let off_hand_item_model_resource_location = unsafe {
        read_label(
            request.off_hand_item_model_resource_location_utf8,
            "world shader environment off-hand item-model resource location",
        )
    }?;
    if request.lightmap_reserved != 0 {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "world shader environment lightmap reserved field must be zero",
        ));
    }
    let vanilla_lightmap = if bool_flag(
        request.lightmap_enabled,
        "world shader environment lightmap enabled",
    )? {
        Some(VanillaLightmapFrame {
            generation: request.lightmap_generation,
            inputs: VanillaLightmapInputs {
                ambient_light_factor: request.lightmap_ambient_light_factor,
                sky_factor: request.lightmap_sky_factor,
                block_factor: request.lightmap_block_factor,
                night_vision_factor: request.lightmap_night_vision_factor,
                darkness_scale: request.lightmap_darkness_scale,
                darken_world_factor: request.lightmap_darken_world_factor,
                brightness_factor: request.lightmap_brightness_factor,
                sky_light_color: [
                    request.lightmap_sky_light_r,
                    request.lightmap_sky_light_g,
                    request.lightmap_sky_light_b,
                ],
                ambient_color: [
                    request.lightmap_ambient_r,
                    request.lightmap_ambient_g,
                    request.lightmap_ambient_b,
                ],
            },
        })
    } else {
        if request.lightmap_generation != 0
            || [
                request.lightmap_ambient_light_factor,
                request.lightmap_sky_factor,
                request.lightmap_block_factor,
                request.lightmap_night_vision_factor,
                request.lightmap_darkness_scale,
                request.lightmap_darken_world_factor,
                request.lightmap_brightness_factor,
                request.lightmap_sky_light_r,
                request.lightmap_sky_light_g,
                request.lightmap_sky_light_b,
                request.lightmap_ambient_r,
                request.lightmap_ambient_g,
                request.lightmap_ambient_b,
            ]
            .into_iter()
            .any(|value| value != 0.0)
        {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "disabled world shader environment lightmap must be zeroed",
            ));
        }
        None
    };
    if let Some(lightmap) = vanilla_lightmap {
        lightmap.validate().map_err(|error| {
            GalError::ffi(
                StatusCode::InvalidArgument,
                format!("world shader environment lightmap is invalid: {error}"),
            )
        })?;
    }
    let environment = WorldShaderEnvironmentFrame {
        enabled,
        world_generation: request.world_generation,
        world_time: request.world_time,
        frame_time_seconds: request.frame_time_seconds,
        frame_counter: request.frame_counter,
        frame_time_counter: request.frame_time_counter,
        world_day: request.world_day,
        moon_phase: request.moon_phase,
        time_of_day: request.time_of_day,
        rain_strength: request.rain_strength,
        thunder_strength: request.thunder_strength,
        sky_darken: request.sky_darken,
        eye_submersion: request.eye_submersion,
        screen_brightness: request.screen_brightness,
        far_plane: request.far_plane,
        distant_horizons_render_distance: request.distant_horizons_render_distance,
        relative_eye_position: [
            request.relative_eye_x,
            request.relative_eye_y,
            request.relative_eye_z,
        ],
        sky_color: [
            request.sky_color_r,
            request.sky_color_g,
            request.sky_color_b,
        ],
        darkness_light_factor: request.darkness_light_factor,
        blindness: request.blindness,
        darkness_factor: request.darkness_factor,
        eye_brightness: [request.eye_brightness_block, request.eye_brightness_sky],
        night_vision: request.night_vision,
        fog_color: [
            request.fog_color_r,
            request.fog_color_g,
            request.fog_color_b,
        ],
        fog_parameter_color: [
            request.fog_parameter_color_r,
            request.fog_parameter_color_g,
            request.fog_parameter_color_b,
            request.fog_parameter_color_a,
        ],
        fog_environmental_start: request.fog_environmental_start,
        fog_environmental_end: request.fog_environmental_end,
        fog_render_distance_start: request.fog_render_distance_start,
        fog_render_distance_end: request.fog_render_distance_end,
        fog_sky_end: request.fog_sky_end,
        fog_clouds_end: request.fog_clouds_end,
        biome_precipitation: request.biome_precipitation,
        biome_resource_location,
        main_hand_item_model_resource_location,
        off_hand_item_model_resource_location,
        main_hand_item_light_emission: request.main_hand_item_light_emission,
        off_hand_item_light_emission: request.off_hand_item_light_emission,
        vanilla_lightmap,
    };
    if !enabled {
        if environment != WorldShaderEnvironmentFrame::default() {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "disabled world shader environment frame must be zeroed",
            ));
        }
        return Ok(environment);
    }
    if environment.world_generation == 0 {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "enabled world shader environment frame requires a world generation",
        ));
    }
    for (label, value) in [
        ("time of day", environment.time_of_day),
        ("rain strength", environment.rain_strength),
        ("thunder strength", environment.thunder_strength),
        ("sky darken", environment.sky_darken),
    ] {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                format!("world shader environment {label} must be finite and within [0, 1]"),
            ));
        }
    }
    if !environment.darkness_light_factor.is_finite() || environment.darkness_light_factor < 0.0 {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "world shader environment darkness light factor must be finite and non-negative",
        ));
    }
    if environment.distant_horizons_render_distance < 0 {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "world shader environment Distant Horizons render distance must be non-negative",
        ));
    }
    for (label, value) in [
        ("blindness", environment.blindness),
        ("darkness factor", environment.darkness_factor),
    ] {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                format!("world shader environment {label} must be finite and within [0, 1]"),
            ));
        }
    }
    if environment
        .eye_brightness
        .iter()
        .any(|value| !(0..=240).contains(value) || value % 16 != 0)
    {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "world shader environment eye brightness must contain packed vanilla light values in [0, 240]",
        ));
    }
    if !environment.night_vision.is_finite() || !(0.0..=1.0).contains(&environment.night_vision) {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "world shader environment night vision must be finite and within [0, 1]",
        ));
    }
    if environment
        .fog_color
        .iter()
        .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
    {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "world shader environment fog color must be finite and within [0, 1]",
        ));
    }
    for (label, value) in [
        ("fog parameter red", environment.fog_parameter_color[0]),
        ("fog parameter green", environment.fog_parameter_color[1]),
        ("fog parameter blue", environment.fog_parameter_color[2]),
        ("fog parameter alpha", environment.fog_parameter_color[3]),
        (
            "fog environmental start",
            environment.fog_environmental_start,
        ),
        ("fog environmental end", environment.fog_environmental_end),
        (
            "fog render-distance start",
            environment.fog_render_distance_start,
        ),
        (
            "fog render-distance end",
            environment.fog_render_distance_end,
        ),
        ("fog sky end", environment.fog_sky_end),
    ] {
        if !value.is_finite() {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                format!("world shader environment {label} must be finite"),
            ));
        }
    }
    if !(0..720_720).contains(&environment.frame_counter) {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "world shader environment frame counter must be within [0, 720720)",
        ));
    }
    if !(0.0..3600.0).contains(&environment.frame_time_counter) {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "world shader environment frame time counter must be within [0, 3600)",
        ));
    }
    if !environment.frame_time_seconds.is_finite()
        || !(0.0..3600.0).contains(&environment.frame_time_seconds)
    {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "world shader environment frame time must be finite and within [0, 3600)",
        ));
    }
    if !(0..=7).contains(&environment.moon_phase) {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "world shader environment moon phase must be within [0, 7]",
        ));
    }
    if !(0..=3).contains(&environment.eye_submersion) {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "world shader environment eye submersion must be within [0, 3]",
        ));
    }
    if !(0..=2).contains(&environment.biome_precipitation) {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "world shader environment biome precipitation must be 0 (none), 1 (rain), or 2 (snow)",
        ));
    }
    if !is_canonical_resource_location(&environment.biome_resource_location) {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "world shader environment biome resource location must be canonical namespace:path text",
        ));
    }
    for (label, value) in [
        (
            "main-hand item-model resource location",
            &environment.main_hand_item_model_resource_location,
        ),
        (
            "off-hand item-model resource location",
            &environment.off_hand_item_model_resource_location,
        ),
    ] {
        if !value.is_empty() && !is_canonical_resource_location(value) {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                format!(
                    "world shader environment {label} must be empty or canonical namespace:path text"
                ),
            ));
        }
    }
    if !environment.screen_brightness.is_finite() || environment.screen_brightness < 0.0 {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "world shader environment screen brightness must be finite and non-negative",
        ));
    }
    if !environment.far_plane.is_finite() || environment.far_plane <= 0.0 {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "world shader environment far plane must be finite and positive",
        ));
    }
    if environment
        .relative_eye_position
        .iter()
        .any(|value| !value.is_finite())
    {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "world shader environment relative eye position must be finite",
        ));
    }
    if environment
        .sky_color
        .iter()
        .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
    {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "world shader environment sky color must be finite and within [0, 1]",
        ));
    }
    for (label, emission) in [
        (
            "main-hand item light emission",
            environment.main_hand_item_light_emission,
        ),
        (
            "off-hand item light emission",
            environment.off_hand_item_light_emission,
        ),
    ] {
        if !(0..=15).contains(&emission) {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                format!("world shader environment {label} must be within [0, 15]"),
            ));
        }
    }
    Ok(environment)
}

pub(crate) fn is_canonical_resource_location(value: &str) -> bool {
    let Some((namespace, path)) = value.split_once(':') else {
        return false;
    };
    !namespace.is_empty()
        && !path.is_empty()
        && !path.contains(':')
        && namespace.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-' | b'.')
        })
        && path.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'_' | b'-' | b'.' | b'/')
        })
}

fn decode_world_voxel_volume_frame(
    request: FfiWorldVoxelVolumeFrame,
) -> GalResult<WorldVoxelVolumeFrame> {
    validate_item_size::<FfiWorldVoxelVolumeFrame>(request.byte_size, "world voxel-volume frame")?;
    let enabled = bool_flag(request.enabled, "world voxel-volume frame enabled")?;
    if !enabled {
        if request.world_generation != 0
            || request.resource_generation != 0
            || request.camera_x != 0.0
            || request.camera_y != 0.0
            || request.camera_z != 0.0
        {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "disabled world voxel-volume frame must be zeroed",
            ));
        }
        return Ok(WorldVoxelVolumeFrame::default());
    }
    if request.world_generation == 0 || request.resource_generation == 0 {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "enabled world voxel-volume frame requires non-zero world and resource generations",
        ));
    }
    let camera_world_position = [request.camera_x, request.camera_y, request.camera_z];
    if camera_world_position.iter().any(|value| !value.is_finite()) {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "world voxel-volume camera position must be finite",
        ));
    }
    Ok(WorldVoxelVolumeFrame {
        enabled,
        world_generation: request.world_generation,
        resource_generation: request.resource_generation,
        camera_world_position,
    })
}

pub(crate) fn decode_world_background_request(
    request: FfiWorldBackgroundRequest,
    frame_viewport_width: i32,
    frame_viewport_height: i32,
) -> GalResult<WorldBackgroundRequest> {
    validate_item_size::<FfiWorldBackgroundRequest>(request.byte_size, "world background")?;
    let enabled = bool_flag(request.enabled, "world background enabled")?;
    if !enabled {
        return Ok(WorldBackgroundRequest::default());
    }
    if !matches!(
        request.sky_type,
        WORLD_BACKGROUND_SKY_OVERWORLD
            | WORLD_BACKGROUND_SKY_NETHER
            | WORLD_BACKGROUND_SKY_END
            | WORLD_BACKGROUND_SKY_CUSTOM
    ) {
        return Err(GalError::ffi(
            StatusCode::UnknownEnum,
            format!("unknown world background sky type {}", request.sky_type),
        ));
    }
    if request.load_intent != WORLD_BACKGROUND_LOAD_CLEAR {
        return Err(GalError::ffi(
            StatusCode::UnknownEnum,
            format!(
                "unknown world background load intent {}",
                request.load_intent
            ),
        ));
    }
    if request.store_intent != WORLD_BACKGROUND_STORE_STORE {
        return Err(GalError::ffi(
            StatusCode::UnknownEnum,
            format!(
                "unknown world background store intent {}",
                request.store_intent
            ),
        ));
    }
    let viewport_width = u32::try_from(request.viewport_width).map_err(|_| {
        GalError::ffi(
            StatusCode::InvalidArgument,
            format!(
                "world background viewport width must be non-negative, got {}",
                request.viewport_width
            ),
        )
    })?;
    let viewport_height = u32::try_from(request.viewport_height).map_err(|_| {
        GalError::ffi(
            StatusCode::InvalidArgument,
            format!(
                "world background viewport height must be non-negative, got {}",
                request.viewport_height
            ),
        )
    })?;
    if request.viewport_width != frame_viewport_width
        || request.viewport_height != frame_viewport_height
    {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "world background viewport metadata must match whole-frame viewport",
        ));
    }
    if request.sky_reserved0 != 0 {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "world background sky reserved field must be zero",
        ));
    }
    Ok(WorldBackgroundRequest {
        enabled,
        sky_type: request.sky_type,
        color_argb: request.color_argb,
        load_intent: request.load_intent,
        store_intent: request.store_intent,
        viewport_width,
        viewport_height,
        sky: crate::render::vulkanic::world_primitive_frontend::WorldSkyRequest {
            visible: bool_flag(request.sky_visible, "world sky visible")?,
            sunrise_or_sunset: bool_flag(
                request.sky_sunrise_or_sunset,
                "world sky sunrise or sunset",
            )?,
            dark_disc: bool_flag(request.sky_dark_disc, "world sky dark disc")?,
            sun_angle: request.sky_sun_angle,
            time_of_day: request.sky_time_of_day,
            rain_brightness: request.sky_rain_brightness,
            star_brightness: request.sky_star_brightness,
            sunrise_and_sunset_color_argb: request.sky_sunrise_and_sunset_color_argb,
            moon_phase: request.sky_moon_phase,
            end_flash_intensity: request.sky_end_flash_intensity,
            end_flash_x_angle: request.sky_end_flash_x_angle,
            end_flash_y_angle: request.sky_end_flash_y_angle,
            sky_color_argb: request.sky_color_argb,
        },
    })
}

pub(crate) unsafe fn decode_world_border_asset_update(
    request: *const FfiWorldBorderAssetUpdateRequest,
    capabilities: BackendCapabilities,
) -> GalResult<(u64, WorldBorderAssetPayload)> {
    let request = read_struct(request, "world-border asset update request")?;
    validate_header::<FfiWorldBorderAssetUpdateRequest>(request.header)?;
    reject_unknown_feature_bits(request.negotiated_feature_bits)?;
    let supported = capability_feature_bits(capabilities);
    if request.negotiated_feature_bits & !supported != 0 {
        return Err(GalError::unsupported_feature(format!(
            "requested unsupported world-border asset feature bits 0x{:x}",
            request.negotiated_feature_bits & !supported
        )));
    }
    if request.generation == 0 {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "world-border asset generation must be non-zero",
        ));
    }
    let png_bytes = read_bounded_bytes(
        request.png_bytes,
        true,
        FFI_MAX_WORLD_BORDER_ASSET_BYTES,
        "world-border texture PNG bytes",
    )?;
    Ok((
        request.generation,
        WorldBorderAssetPayload {
            texture_id: request.texture_id,
            png_bytes,
        },
    ))
}

pub(crate) unsafe fn decode_world_crack_asset_update(
    request: *const FfiWorldCrackAssetUpdateRequest,
    capabilities: BackendCapabilities,
) -> GalResult<(u64, Vec<WorldCrackAssetPayload>)> {
    let request = read_struct(request, "world crack asset update request")?;
    validate_header::<FfiWorldCrackAssetUpdateRequest>(request.header)?;
    reject_unknown_feature_bits(request.negotiated_feature_bits)?;
    let supported = capability_feature_bits(capabilities);
    if request.negotiated_feature_bits & !supported != 0 {
        return Err(GalError::unsupported_feature(format!(
            "requested unsupported world crack asset feature bits 0x{:x}",
            request.negotiated_feature_bits & !supported
        )));
    }
    if request.generation == 0 {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "world crack asset generation must be non-zero",
        ));
    }
    let raw_assets = read_limited_slice(request.assets, true, "world crack asset payloads")?;
    let mut seen = BTreeMap::new();
    let mut assets = Vec::with_capacity(raw_assets.len());
    for asset in raw_assets {
        validate_item_size::<FfiWorldCrackAssetPayload>(
            asset.byte_size,
            "world crack asset payload",
        )?;
        if asset.stage >= 10 {
            return Err(GalError::ffi(
                StatusCode::UnknownEnum,
                format!("unknown world crack stage {}", asset.stage),
            ));
        }
        if seen.insert(asset.stage, ()).is_some() {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                format!(
                    "duplicate world crack asset payload for stage {}",
                    asset.stage
                ),
            ));
        }
        let png_bytes = read_bounded_bytes(
            asset.png_bytes,
            true,
            FFI_MAX_WORLD_CRACK_ASSET_BYTES,
            "world crack asset PNG bytes",
        )?;
        assets.push(WorldCrackAssetPayload {
            stage: asset.stage,
            png_bytes,
        });
    }
    Ok((request.generation, assets))
}

#[no_mangle]
pub unsafe extern "C" fn mattmc_vulkanic_gal_whole_frame_submit(
    context_id: u64,
    request: *const FfiWholeFrameSubmitRequest,
    out: *mut FfiWholeFrameSubmitResult,
) -> i32 {
    with_registry_mut(|registry| {
        let Some(context) = registry.contexts.get_mut(&context_id) else {
            let error = GalError::ffi(
                StatusCode::StaleHandle,
                format!("unknown context id {context_id}"),
            );
            let _ = write_out(
                out,
                FfiWholeFrameSubmitResult {
                    status: error.code as i32,
                    error_domain: error.domain as u32,
                    ..FfiWholeFrameSubmitResult::default()
                },
                "whole-frame submit result",
            );
            return error.code as i32;
        };
        let input_bytes = read_whole_frame_request(request).as_ref()
            .map(input_bytes_for_whole_frame).unwrap_or(0);
        context.ffi_calls += 1;
        context.ffi_input_bytes = context.ffi_input_bytes.saturating_add(input_bytes);
        context.ffi_output_bytes = context
            .ffi_output_bytes
            .saturating_add(size_of::<FfiWholeFrameSubmitResult>() as u64);
        let decode_started = std::time::Instant::now();
        let result = decode_whole_frame_submit_with_tiled_gui(request, context.gal.capabilities())
            .and_then(
                |(
                    generation,
                    frame_target,
                    world_frame,
                    gui_sprites,
                    gui_affine_quads,
                    gui_mesh_batches,
                    gui_blur_before_stratum,
                    gui_blur_radius,
                    post_effect_id,
                    gui_tiled_quads,
                )| {
                    let world_frame_id = world_frame.frame_id;
                    if std::env::var_os("MATTMC_TRACE_WHOLE_FRAME").is_some() {
                        // Observe decoded native item meshes, not Java producer
                        // counts. A group is the real scheduler item identity.
                        let (groups, layers, nonidentity, distinct) =
                            super::super::gui_mesh_frontend::flat_item_mesh_decode_counts(&gui_mesh_batches);
                        if layers != 0 {
                            whole_frame_trace(&format!("whole-frame.gui-item-mesh-layers groups={} layers={}", groups, layers));
                            whole_frame_trace(&format!("whole-frame.gui-item-mesh-transforms layers={} nonidentity={} distinct={}", layers, nonidentity, distinct));
                        }
                    }
                    let item_layer_count = gui_affine_quads.iter().map(|quad|quad.item_raster_layers.len()).sum::<usize>();
                    context.ffi_input_bytes = context.ffi_input_bytes.saturating_add(
                        item_layer_count as u64 * size_of::<FfiGuiItemRasterLayer>() as u64);
                    if item_layer_count != 0 {
                        whole_frame_trace(&format!("whole-frame.gui-item-layers groups={} layers={}",
                            gui_affine_quads.iter().filter(|quad|!quad.item_raster_layers.is_empty()).count(),item_layer_count));
                        if std::env::var_os("MATTMC_TRACE_WHOLE_FRAME").is_some() {
                            let matrices=gui_affine_quads.iter().flat_map(|quad| &quad.item_raster_layers)
                                .map(|layer| layer.model_transform);
                            let mut distinct=BTreeSet::new();
                            let mut nonidentity=0;
                            for matrix in matrices {
                                nonidentity+=usize::from(matrix!=Default::default());
                                distinct.insert(matrix.0.map(|v| if v==0.0 {0} else {v.to_bits()}));
                            }
                            whole_frame_trace(&format!("whole-frame.gui-item-transforms layers={} nonidentity={} distinct={}",
                                item_layer_count,nonidentity,distinct.len()));
                        }
                    }
                    let ffi_decode_nanos =
                        crate::render::vulkanic::metrics::elapsed_nanos_u64(decode_started);
                    let gui_started = std::time::Instant::now();
                    context
                        .world_primitive_frontend
                        .validate_post_effect_request_with_globals(&post_effect_id, world_frame.engine_globals)?;
                    whole_frame_trace(&format!(
                        "whole-frame.frontend.begin generation={} frame={} decode_nanos={}",
                        generation, world_frame_id, ffi_decode_nanos
                    ));
                    let frontend_started = std::time::Instant::now();
                    // Capture short menu transitions without sampling past the
                    // requested frame. Bound diagnostics across the process.
                    static TILED_RECEIPTS: std::sync::atomic::AtomicUsize =
                        std::sync::atomic::AtomicUsize::new(0);
                    let tiled_receipt = if !gui_tiled_quads.is_empty()
                        && std::env::var_os("MATTMC_TRACE_GUI_TILES").is_some()
                        && TILED_RECEIPTS.fetch_update(
                            std::sync::atomic::Ordering::Relaxed,
                            std::sync::atomic::Ordering::Relaxed,
                            |count| (count < 512).then_some(count + 1),
                        ).is_ok()
                    {
                        Some((gui_tiled_quads.len(),
                            crate::render::vulkanic::gui_frontend::preflight_tiled_affine_count(
                                &gui_tiled_quads, 0)?))
                    } else { None };
                    let frontend_result = context
                        .world_primitive_frontend
                        .submit_whole_frame_with_tiled_gui_frontend(
                            &mut context.gal,
                            generation,
                            frame_target,
                            world_frame,
                            &mut context.gui_frontend,
                            gui_sprites,
                            gui_affine_quads,
                            gui_mesh_batches,
                            post_effect_id,
                            gui_blur_before_stratum,
                            gui_blur_radius,
                            gui_tiled_quads,
                        );
                    whole_frame_trace(&format!(
                        "whole-frame.frontend.end generation={} frame={} elapsed_nanos={}",
                        generation,
                        world_frame_id,
                        crate::render::vulkanic::metrics::elapsed_nanos_u64(frontend_started)
                    ));
                    let (mut world_stats, gui_stats) = frontend_result?;
                    if let Some((parents, children)) = tiled_receipt {
                        eprintln!("whole-frame.gui-tiles.submitted frame={} parents={} children={}",
                            world_frame_id, parents, children);
                    }
                    let gui_frontend_nanos =
                        crate::render::vulkanic::metrics::elapsed_nanos_u64(gui_started);
                    world_stats.profile.ffi_decode_nanos = ffi_decode_nanos;
                    world_stats.profile.gui_frontend_nanos = gui_frontend_nanos;
                    whole_frame_trace(&format!(
                        "whole-frame.stale-targets.begin generation={} frame={}",
                        generation, world_frame_id
                    ));
                    destroy_stale_frame_targets(context)?;
                    whole_frame_trace(&format!(
                        "whole-frame.stale-targets.end generation={} frame={}",
                        generation, world_frame_id
                    ));
                    world_stats.command_lists = 1;
                    Ok((world_stats, gui_stats))
                },
            );
        match result {
            Ok((world_stats, gui_stats)) => {
                let value = whole_frame_result_ok(context, world_stats, gui_stats);
                let _ = write_out(out, value, "whole-frame submit result");
                StatusCode::Ok as i32
            }
            Err(error) => {
                set_last_error(context, &error);
                let _ = write_out(
                    out,
                    FfiWholeFrameSubmitResult {
                        status: error.code as i32,
                        error_domain: error.domain as u32,
                        metrics: context_metrics(context),
                        ..FfiWholeFrameSubmitResult::default()
                    },
                    "whole-frame submit result",
                );
                error.code as i32
            }
        }
    })
}

/// Emits phase boundaries only for an explicitly requested diagnostic run.
/// The ABI deliberately remains one call; this distinguishes semantic/GAL
/// work from stale-target retirement without changing rendering behavior.
fn whole_frame_trace(message: &str) {
    if std::env::var_os("MATTMC_TRACE_WHOLE_FRAME").is_some() {
        eprintln!("{message}");
    }
}

#[no_mangle]
pub unsafe extern "C" fn mattmc_vulkanic_gal_world_primitives_submit(
    context_id: u64,
    request: *const FfiWholeFrameSubmitRequest,
    out: *mut FfiWholeFrameSubmitResult,
) -> i32 {
    with_registry_mut(|registry| {
        let Some(context) = registry.contexts.get_mut(&context_id) else {
            let error = GalError::ffi(
                StatusCode::StaleHandle,
                format!("unknown context id {context_id}"),
            );
            let _ = write_out(
                out,
                FfiWholeFrameSubmitResult {
                    status: error.code as i32,
                    error_domain: error.domain as u32,
                    ..FfiWholeFrameSubmitResult::default()
                },
                "world primitive submit result",
            );
            return error.code as i32;
        };
        let input_bytes = read_whole_frame_request(request).as_ref()
            .map(input_bytes_for_whole_frame).unwrap_or(0);
        context.ffi_calls += 1;
        context.ffi_input_bytes = context.ffi_input_bytes.saturating_add(input_bytes);
        context.ffi_output_bytes = context
            .ffi_output_bytes
            .saturating_add(size_of::<FfiWholeFrameSubmitResult>() as u64);
        let result = decode_world_primitive_submit(request, context.gal.capabilities()).and_then(
            |(generation, frame_target, world_frame)| {
                let world_stats = context.world_primitive_frontend.submit_partial_frame(
                    &mut context.gal,
                    generation,
                    frame_target,
                    world_frame,
                )?;
                destroy_stale_frame_targets(context)?;
                Ok(world_stats)
            },
        );
        match result {
            Ok(world_stats) => {
                let value = whole_frame_result_ok(context, world_stats, GuiSubmitStats::default());
                let _ = write_out(out, value, "world primitive submit result");
                StatusCode::Ok as i32
            }
            Err(error) => {
                set_last_error(context, &error);
                let _ = write_out(
                    out,
                    FfiWholeFrameSubmitResult {
                        status: error.code as i32,
                        error_domain: error.domain as u32,
                        metrics: context_metrics(context),
                        ..FfiWholeFrameSubmitResult::default()
                    },
                    "world primitive submit result",
                );
                error.code as i32
            }
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn mattmc_vulkanic_gal_world_border_update_asset(
    context_id: u64,
    request: *const FfiWorldBorderAssetUpdateRequest,
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
            input_bytes_for_world_border_asset_update(&*request)
        };
        context.ffi_calls += 1;
        context.ffi_input_bytes = context.ffi_input_bytes.saturating_add(input_bytes);
        context.ffi_output_bytes = context
            .ffi_output_bytes
            .saturating_add(size_of::<FfiStatusResult>() as u64);
        let result = decode_world_border_asset_update(request, context.gal.capabilities())
            .and_then(|(generation, payload)| {
                context
                    .world_primitive_frontend
                    .apply_world_border_asset_update(&mut context.gal, generation, payload)
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
pub unsafe extern "C" fn mattmc_vulkanic_gal_world_text_update_images(
    context_id: u64,
    request: *const FfiWorldTextImageUpdateRequest,
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
        context.ffi_calls = context.ffi_calls.saturating_add(1);
        context.ffi_input_bytes = context
            .ffi_input_bytes
            .saturating_add(if request.is_null() {
                0
            } else {
                input_bytes_for_world_text_image_update(&*request)
            });
        context.ffi_output_bytes = context
            .ffi_output_bytes
            .saturating_add(size_of::<FfiStatusResult>() as u64);
        let result = decode_world_text_image_update(request, context.gal.capabilities()).and_then(
            |(generation, assets)| {
                context
                    .world_primitive_frontend
                    .apply_world_text_image_update(generation, assets)
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
pub unsafe extern "C" fn mattmc_vulkanic_gal_world_crack_update_assets(
    context_id: u64,
    request: *const FfiWorldCrackAssetUpdateRequest,
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
            input_bytes_for_world_crack_asset_update(&*request)
        };
        context.ffi_calls += 1;
        context.ffi_input_bytes = context.ffi_input_bytes.saturating_add(input_bytes);
        context.ffi_output_bytes = context
            .ffi_output_bytes
            .saturating_add(size_of::<FfiStatusResult>() as u64);
        let result = decode_world_crack_asset_update(request, context.gal.capabilities()).and_then(
            |(generation, payloads)| {
                context
                    .world_primitive_frontend
                    .apply_world_crack_asset_update(&mut context.gal, generation, payloads)
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
pub unsafe extern "C" fn mattmc_vulkanic_gal_world_lod_update_assets(
    context_id: u64,
    request: *const FfiWorldLodAssetUpdateRequest,
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
            input_bytes_for_world_lod_asset_update(&*request)
        };
        context.ffi_calls += 1;
        context.ffi_input_bytes = context.ffi_input_bytes.saturating_add(input_bytes);
        context.ffi_output_bytes = context
            .ffi_output_bytes
            .saturating_add(size_of::<FfiStatusResult>() as u64);
        let result = decode_world_lod_asset_update(request, context.gal.capabilities()).and_then(
            |(generation, assets, retirements, material_provenance)| {
                context
                    .world_primitive_frontend
                    .apply_world_lod_column_asset_update_with_provenance(
                        &mut context.gal,
                        generation,
                        assets,
                        retirements,
                        material_provenance,
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
