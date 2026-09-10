//! Rust-owned ordering over immutable semantic terrain buffers. The output is
//! explicit indexed draw ranges; it neither rewrites GPU memory nor reconstructs
//! backend state. One cache per mesh generation bounds retained sort state.

use super::*;
use crate::render::chunk::translucent::semantic::SemanticTranslucentGeometry;

pub(super) struct CachedOrder {
    geometry: SemanticTranslucentGeometry,
    quads: Vec<(u32, u64)>,
    camera: Option<[f32; 3]>,
    order: Vec<usize>,
}

pub(super) fn validate_instance(instance: &WorldMeshInstanceRequest) -> GalResult<()> {
    if instance.stratum != WORLD_STRATUM_TERRAIN
        || instance.mesh_section_index != WORLD_MESH_SECTION_ALL
        || !matches!(instance.depth_policy, WORLD_DEPTH_POLICY_TEST_WRITE | WORLD_DEPTH_POLICY_TEST_NO_WRITE)
    {
        return Err(GalError::invalid_argument("camera-sorted quads require a complete translucent terrain instance"));
    }
    // The semantic contract is a camera-relative translation. Do not silently
    // interpret an entity/model transform as a camera position.
    for i in 0..16 {
        if (12..15).contains(&i) { continue; }
        let expected = if [0, 5, 10, 15].contains(&i) { 1.0 } else { 0.0 };
        if instance.transform[i] != expected {
            return Err(GalError::invalid_argument("camera-sorted terrain requires a translation-only transform"));
        }
    }
    if !instance.transform.iter().all(|value| value.is_finite()) {
        return Err(GalError::invalid_argument("camera-sorted terrain transform must be finite"));
    }
    Ok(())
}

fn prepare(asset: &MeshAssetStore) -> GalResult<CachedOrder> {
    let stride = index_stride(asset.index_type) as usize;
    let mut positions = Vec::new();
    let mut quads = Vec::new();
    let mut expected_offset = 0usize;
    for (section_index, section) in asset.sections.iter().enumerate() {
        if section.material_mode != WORLD_MATERIAL_MODE_TRANSLUCENT
            || section.index_count % 6 != 0
            || section.index_offset as usize != expected_offset
        {
            return Err(GalError::invalid_argument("camera-sorted terrain requires complete non-overlapping translucent quad ranges"));
        }
        for quad in 0..section.index_count as usize / 6 {
            let offset = section.index_offset as usize + quad * 6 * stride;
            let mut indices = [0u32; 6];
            for (lane, value) in indices.iter_mut().enumerate() {
                *value = mesh_index_value(&asset.index_bytes, asset.index_type, offset / stride + lane)?;
            }
            if indices[2] != indices[3] || indices[0] != indices[5] {
                return Err(GalError::invalid_argument("camera-sorted terrain requires canonical quad triangle indices"));
            }
            let mut corners = [[0f32; 3]; 4];
            for (corner, lane) in [0, 1, 2, 4].into_iter().enumerate() {
                let start = indices[lane] as usize * WORLD_MESH_GPU_VERTEX_BYTES;
                let vertex = asset.vertex_bytes.get(start..start + 12).ok_or_else(||
                    GalError::invalid_argument("camera-sorted terrain vertex is outside its immutable buffer"))?;
                for axis in 0..3 {
                    corners[corner][axis] = f32::from_ne_bytes(vertex[axis * 4..axis * 4 + 4].try_into().unwrap());
                }
            }
            if section.winding == WORLD_WINDING_CW { corners.reverse(); }
            positions.push(corners);
            quads.push((section_index as u32, offset as u64));
        }
        expected_offset += section.index_count as usize * stride;
    }
    if expected_offset != asset.index_bytes.len() {
        return Err(GalError::invalid_argument("camera-sorted terrain ranges do not cover their index buffer"));
    }
    let geometry = SemanticTranslucentGeometry::new(&positions).ok_or_else(||
        GalError::invalid_argument("invalid camera-sorted terrain geometry"))?;
    Ok(CachedOrder { geometry, quads, camera: None, order: Vec::new() })
}

pub(super) fn append_batches(
    instance: &WorldMeshInstanceRequest,
    asset: &MeshAssetStore,
    instance_index: usize,
    color_format: ColorFormat,
    raster_y_direction: RasterYDirection,
    g_buffer: bool,
    batches: &mut Vec<MeshBatch>,
) -> GalResult<()> {
    validate_instance(instance)?;
    let camera = [-instance.transform[12], -instance.transform[13], -instance.transform[14]];
    let mut retained = asset.translucent_order.borrow_mut();
    if retained.is_none() { *retained = Some(prepare(asset)?); }
    let cache = retained.as_mut().unwrap();
    if cache.camera != Some(camera) {
        let order = cache.geometry.order(camera).ok_or_else(|| GalError::invalid_argument("invalid translucent camera ordering"))?;
        cache.order = order;
        cache.camera = Some(camera);
    }
    for &ordinal in &cache.order {
        let (section_index, index_offset) = cache.quads[ordinal];
        let section = &asset.sections[section_index as usize];
        let key = mesh_key_for_section(instance, section, section_index, section.cull_policy,
            asset_generation_for_key(instance.mesh_key, asset)?, color_format, raster_y_direction, g_buffer);
        // Only contiguous indices with identical resources and the same
        // instance can form one draw. Never move a pane across another material.
        if let Some(last) = batches.last_mut().filter(|last|
            last.key == key && last.indices.as_slice() == [instance_index]
            && last.index_offset + u64::from(last.index_count) * index_stride(asset.index_type) == index_offset)
        {
            last.index_count += 6;
        } else {
            batches.push(MeshBatch { key, index_offset, index_count: 6, indices: vec![instance_index] });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_sort_rejects_partial_geometry_and_non_quad_indices() {
        let mut asset = MeshAssetStore::default();
        asset.vertex_bytes = vec![0; WORLD_MESH_GPU_VERTEX_BYTES * 4];
        asset.index_bytes = [0u16, 1, 2, 2, 3, 0].into_iter().flat_map(u16::to_ne_bytes).collect();
        asset.sections = vec![WorldMeshSection {
            material_id: WORLD_MATERIAL_ID_TRANSLUCENT_TEXTURED,
            texture_id: WORLD_MESH_TEXTURE_TERRAIN_BLOCK_ATLAS,
            material_mode: WORLD_MATERIAL_MODE_TRANSLUCENT,
            cull_policy: WORLD_CULL_BACK, winding: WORLD_WINDING_CCW,
            index_offset: 0, index_count: 6,
        }];
        assert!(prepare(&asset).is_ok());
        asset.sections[0].index_offset = 2;
        assert!(prepare(&asset).is_err());
        asset.sections[0].index_offset = 0;
        asset.sections[0].index_count = 3;
        assert!(prepare(&asset).is_err());
        asset.sections[0].index_count = 6;
        asset.index_bytes[6] = 1;
        assert!(prepare(&asset).is_err());
        asset.index_bytes[6] = 2;
        asset.sections[0].material_mode = WORLD_MATERIAL_MODE_OPAQUE;
        assert!(prepare(&asset).is_err());
    }
}
