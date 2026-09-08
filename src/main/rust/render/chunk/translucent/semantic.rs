//! Owned CPU geometry for semantic callers, without FFI handles or renderer state.
//! Returned ordinals always refer to the caller's original quad order (not the
//! mesher's facing buckets). Material boundaries are intentionally absent.

use super::*;

pub(crate) struct SemanticTranslucentGeometry(NativeTranslucentSectionGeometry);

impl SemanticTranslucentGeometry {
    pub(crate) fn new(positions: &[[[f32; 3]; 4]]) -> Option<Self> {
        let mut quads = Vec::with_capacity(positions.len());
        for p in positions {
            if !p.iter().flatten().all(|value| value.is_finite()) {
                return None;
            }
            // The diagonal cross product is also used by the native full-quad
            // implementation. It includes all four corners, including a
            // triangle represented by a duplicated fourth corner.
            let a: [f32; 3] = std::array::from_fn(|i| p[2][i] - p[0][i]);
            let b: [f32; 3] = std::array::from_fn(|i| p[3][i] - p[1][i]);
            let normal = normalize3(
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            );
            if ![normal.0, normal.1, normal.2].iter().all(|v| v.is_finite()) {
                return None;
            }
            let record = TranslucentQuadRecord {
                positions: std::array::from_fn(|i| p[i / 3][i % 3]),
                facing: aligned_facing_from_normal(normal).unwrap_or(FACING_UNASSIGNED),
                packed_normal: pack_normal(
                    (normal.0 * 127.0) as i8,
                    (normal.1 * 127.0) as i8,
                    (normal.2 * 127.0) as i8,
                ),
            };
            quads.push(build_quad_info(&record));
        }
        let aligned_separator_distances = build_aligned_separator_distances(&quads);
        Some(Self(NativeTranslucentSectionGeometry { quads, aligned_separator_distances }))
    }

    pub(crate) fn order(&self, camera: [f32; 3]) -> Option<Vec<usize>> {
        if !camera.iter().all(|value| value.is_finite()) {
            return None;
        }
        if let Some(order) = dynamic_topo_graph_sort(&self.0, (camera[0], camera[1], camera[2]), false) {
            return Some(order.into_iter().map(|i| i as usize).collect());
        }
        // This is the native dynamic sorter's cycle policy, not a backend or
        // Java rendering fallback. Preserve its index-key/tie semantics.
        let mut indices = vec![0; self.0.quads.len().checked_mul(6)?];
        if write_distance_sorted_index_buffer(&self.0, &mut indices, camera[0], camera[1], camera[2]) != OK {
            return None;
        }
        Some(indices.chunks_exact(6).map(|quad| quad[0] as usize / 4).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(z: f32, reverse: bool) -> [[f32; 3]; 4] {
        let mut p = [[-1., -1., z], [1., -1., z], [1., 1., z], [-1., 1., z]];
        if reverse { p.reverse(); }
        p
    }

    #[test]
    fn semantic_order_preserves_source_ordinals_across_facing_buckets() {
        let geometry = SemanticTranslucentGeometry::new(&[
            pane(1., false), pane(3., false), pane(2., false),
        ]).unwrap();
        assert_eq!(geometry.order([0., 0., 5.]).unwrap(), vec![0, 2, 1]);
        let reverse = SemanticTranslucentGeometry::new(&[
            pane(1., true), pane(3., true), pane(2., true),
        ]).unwrap();
        assert_eq!(reverse.order([0., 0., 0.]).unwrap(), vec![1, 2, 0]);
    }

    #[test]
    fn semantic_order_rejects_nonfinite_input_and_retains_every_quad() {
        let mut bad = pane(1., false);
        bad[2][0] = f32::NAN;
        assert!(SemanticTranslucentGeometry::new(&[bad]).is_none());
        let geometry = SemanticTranslucentGeometry::new(&[
            pane(1., false), pane(1., true), pane(3., false), pane(3., true),
        ]).unwrap();
        assert!(geometry.order([f32::INFINITY, 0., 0.]).is_none());
        for camera in [[0., 0., 0.], [0., 0., 2.], [0., 0., 5.]] {
            let mut order = geometry.order(camera).unwrap();
            order.sort_unstable();
            assert_eq!(order, vec![0, 1, 2, 3]);
        }
    }
}
