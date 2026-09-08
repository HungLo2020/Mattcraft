//! Fluid visibility and occlusion decisions.
//!
//! Occlusion checks decide whether neighboring fluid or full-occlusion blocks hide
//! a face. Backface decisions preserve the legacy native output for exposed top
//! and side faces without changing the callback fallback path.

use super::*;

pub(in crate::render::chunk::meshing) fn fluid_side_occluded(
    block: &NativeSectionBlockRecord,
    state: NativeMeshingState,
    states: &[Option<NativeMeshingState>],
    dir: i32,
) -> bool {
    let neighbor_id = block.neighbor_state_ids[dir as usize];
    let Some(neighbor) = state_by_id(states, neighbor_id) else {
        return false;
    };
    neighbor.fluid_type == state.fluid_type
        || ((neighbor.flags & STATE_FLAG_FULL_OCCLUSION) != 0 && dir != 1)
}

pub(in crate::render::chunk::meshing) fn fluid_side_exposed(
    block: &NativeSectionBlockRecord,
    states: &[Option<NativeMeshingState>],
    dx: i32,
    dy: i32,
    dz: i32,
    height: f32,
) -> bool {
    // A partial-height fluid does not touch the ceiling's lower face. Match
    // Shapes.blockOccludes' boundary test (double precision, epsilon 1e-7)
    // before considering the neighbor's full-occlusion shape. Side and bottom
    // faces still touch their cell boundaries and keep the normal test below.
    if dy == 1 && (f64::from(height) - 1.0).abs() > 1.0e-7 {
        return true;
    }
    neighborhood_state_id(block, dx, dy, dz)
        .and_then(|id| state_by_id(states, id))
        .map(|state| {
            (state.flags & STATE_FLAG_CAN_OCCLUDE) == 0
                || (state.flags & STATE_FLAG_FULL_OCCLUSION) == 0
        })
        .unwrap_or(true)
}

pub(in crate::render::chunk::meshing) fn fluid_backward_up_face(
    block: &NativeSectionBlockRecord,
    state: NativeMeshingState,
    states: &[Option<NativeMeshingState>],
) -> bool {
    for dz in -1..=1 {
        for dx in -1..=1 {
            let Some(id) = neighborhood_state_id(block, dx, 1, dz) else {
                return true;
            };
            let Some(sample) = state_by_id(states, id) else {
                return true;
            };
            if sample.fluid_type != state.fluid_type
                && (sample.flags & STATE_FLAG_SOLID_RENDER) == 0
            {
                return true;
            }
        }
    }
    false
}
