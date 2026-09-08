//! Section fluid face construction.
//!
//! This module decides which top, bottom, side, overlay, and backface quads exist
//! for a block, then delegates ownership and encoding to a NativeFluidFaceSink.
//! The control flow mirrors the Java fluid mesher and intentionally avoids output
//! changes while the architecture is being split.

use super::*;

#[inline]
pub(super) fn is_supported_native_fluid_type(fluid_type: i32) -> bool {
    matches!(fluid_type, FLUID_WATER | FLUID_LAVA)
}

fn mark_renderable_fluid_sprite<S: NativeFluidFaceSink>(
    sink: &mut S,
    state: NativeMeshingState,
    still: bool,
    overlay: bool,
) {
    // Usage is geometry semantics, not backend admission. Every emitted
    // supported fluid face must declare the sprite it actually samples.
    sink.mark_fluid_sprite(fluid_sprite_mask(state.fluid_type, still, overlay));
}

pub(in crate::render::chunk::meshing) unsafe fn emit_native_section_fluid_faces(
    block: &NativeSectionBlockRecord,
    state: NativeMeshingState,
    states: &[Option<NativeMeshingState>],
    builder: &mut NativeSectionMeshBuilder,
    pending_counts: &mut [usize; MODEL_QUAD_FACING_COUNT],
    analyzer: Option<u64>,
    format: NativeFormat,
    store_raw_quads: bool,
    profile_scan_substages: bool,
    profile_staging_substages: bool,
    total_committed: &mut i32,
) -> Result<usize, i32> {
    let mut sink = BuilderFluidFaceSink {
        builder,
        pending_counts,
        analyzer,
        format,
        store_raw_quads,
        profile_scan_substages,
        profile_staging_substages,
        total_committed,
    };
    native_section_fluid_faces_to_sink(block, state, states, &mut sink)
}

pub(in crate::render::chunk::meshing) fn native_section_fluid_faces_to_sink<
    S: NativeFluidFaceSink,
>(
    block: &NativeSectionBlockRecord,
    state: NativeMeshingState,
    states: &[Option<NativeMeshingState>],
    sink: &mut S,
) -> Result<usize, i32> {
    let mut emitted = 0usize;
    let profile_fluid_substages = fluid_substage_profile_enabled();
    let fluid_diag_enabled = native_fluid_diag_enabled();
    macro_rules! fluid_log {
        ($($arg:tt)*) => {
            if fluid_diag_enabled {
                native_fluid_diag_log(format_args!($($arg)*));
            }
        };
    }
    let visibility_started = Instant::now();
    fluid_log!(
        "begin pos={},{},{} local={},{},{} state={} fluid_type={} pass={} material={} block_id={}",
        block.absolute_x,
        block.absolute_y,
        block.absolute_z,
        block.local_x,
        block.local_y,
        block.local_z,
        block.state_id,
        state.fluid_type,
        state.fluid_pass_id,
        state.fluid_material_bits,
        state.fluid_block_id,
    );
    if !is_supported_native_fluid_type(state.fluid_type) {
        sink.profile()
            .add_stage(PROFILE_FLUID_VIS_HEIGHT, visibility_started);
        fluid_log!(
            "skip-unsupported pos={},{},{} fluid_type={}",
            block.absolute_x,
            block.absolute_y,
            block.absolute_z,
            state.fluid_type
        );
        return Ok(0);
    }
    let cull_up = fluid_side_occluded(block, state, states, 1);
    let cull_down = fluid_side_occluded(block, state, states, 0);
    let cull_north = fluid_side_occluded(block, state, states, 2);
    let cull_south = fluid_side_occluded(block, state, states, 3);
    let cull_west = fluid_side_occluded(block, state, states, 4);
    let cull_east = fluid_side_occluded(block, state, states, 5);
    if cull_up && cull_down && cull_north && cull_south && cull_west && cull_east {
        sink.profile()
            .add_stage(PROFILE_FLUID_VIS_HEIGHT, visibility_started);
        fluid_log!(
            "skip-occluded pos={},{},{}",
            block.absolute_x,
            block.absolute_y,
            block.absolute_z
        );
        return Ok(0);
    }

    let h = fluid_height(block, state, states, 0, 0, 0, 1);
    fluid_log!(
        "own-height pos={},{},{} height={:.4}",
        block.absolute_x,
        block.absolute_y,
        block.absolute_z,
        h
    );
    let heights = if h >= 1.0 {
        [1.0; 4]
    } else {
        let corner_started = profile_start(profile_fluid_substages);
        let hn = fluid_height(block, state, states, 0, 0, -1, 2);
        let hs = fluid_height(block, state, states, 0, 0, 1, 3);
        let hw = fluid_height(block, state, states, -1, 0, 0, 4);
        let he = fluid_height(block, state, states, 1, 0, 0, 5);
        let heights = [
            fluid_corner_height(block, state, states, h, hn, hw, -1, 0, -1),
            fluid_corner_height(block, state, states, h, hs, hw, -1, 0, 1),
            fluid_corner_height(block, state, states, h, hs, he, 1, 0, 1),
            fluid_corner_height(block, state, states, h, hn, he, 1, 0, -1),
        ];
        sink.profile()
            .add_optional_stage(PROFILE_FLUID_CORNER_HEIGHT_USE, corner_started);
        heights
    };
    let cull_down = cull_down || !fluid_side_exposed(block, states, 0, -1, 0, 0.8888889);
    let lighting_tint_started = profile_start(profile_fluid_substages);
    let color = argb_to_abgr(native_tint_color(block, state, true));
    let light = get_emissive_lightmap(block.light_words[13]);
    sink.profile()
        .add_optional_stage(PROFILE_FLUID_LIGHTING_TINT, lighting_tint_started);
    let y_offset = if cull_down { 0.0 } else { 0.001 };
    let top_exposed = fluid_side_exposed(
        block,
        states,
        0,
        1,
        0,
        heights.iter().copied().fold(1.0, f32::min),
    );
    let mut render_heights = heights;
    let emit_top = !cull_up && top_exposed;
    if emit_top {
        for height in &mut render_heights {
            *height -= 0.001;
        }
    }
    let sides = [
        (
            2,
            MODEL_QUAD_FACING_NEG_Z,
            MODEL_QUAD_FACING_POS_Z,
            0.8,
            render_heights[0],
            render_heights[3],
            0.0,
            0.001,
            1.0,
            0.001,
        ),
        (
            3,
            MODEL_QUAD_FACING_POS_Z,
            MODEL_QUAD_FACING_NEG_Z,
            0.8,
            render_heights[2],
            render_heights[1],
            1.0,
            0.999,
            0.0,
            0.999,
        ),
        (
            4,
            MODEL_QUAD_FACING_NEG_X,
            MODEL_QUAD_FACING_POS_X,
            0.6,
            render_heights[1],
            render_heights[0],
            0.001,
            1.0,
            0.001,
            0.0,
        ),
        (
            5,
            MODEL_QUAD_FACING_POS_X,
            MODEL_QUAD_FACING_NEG_X,
            0.6,
            render_heights[3],
            render_heights[2],
            0.999,
            0.0,
            0.999,
            1.0,
        ),
    ];
    let mut visible_sides = [false; 4];
    let mut overlay_sides = [false; 4];
    let side_culls = [cull_north, cull_south, cull_west, cull_east];
    for (index, (dir, _, _, _, h1, h2, _, _, _, _)) in sides.iter().copied().enumerate() {
        let step = dir_step(dir);
        visible_sides[index] = !side_culls[index]
            && fluid_side_exposed(block, states, step.0, step.1, step.2, h1.max(h2));
        if visible_sides[index] {
            let overlay_started = profile_start(profile_fluid_substages);
            overlay_sides[index] = fluid_side_uses_overlay(block, state, states, dir);
            sink.profile()
                .add_optional_stage(PROFILE_FLUID_OVERLAY_SELECTION, overlay_started);
        }
    }
    sink.profile()
        .add_stage(PROFILE_FLUID_VIS_HEIGHT, visibility_started);
    if fluid_diag_enabled {
        fluid_diag(
            block,
            state,
            "top-check",
            cull_up,
            top_exposed,
            heights,
            color,
            light,
            None,
        );
    }

    let geometry_started = Instant::now();
    if emit_top {
        let uv_started = profile_start(profile_fluid_substages);
        fluid_log!(
            "top-uv-start pos={},{},{} flow={:.4},{:.4} falling={}",
            block.absolute_x,
            block.absolute_y,
            block.absolute_z,
            block.fluid_flow_x,
            block.fluid_flow_z,
            state.fluid_falling
        );
        let top_uses_still =
            block.fluid_flow_x == 0.0 && block.fluid_flow_z == 0.0 && state.fluid_falling == 0;
        let top = if top_uses_still {
            fluid_log!(
                "top-uv-still pos={},{},{}",
                block.absolute_x,
                block.absolute_y,
                block.absolute_z
            );
            still_fluid_top_uvs(state.fluid_still)
        } else {
            let sprite = state.fluid_flow;
            fluid_log!(
                "top-uv-flowing-before-atan pos={},{},{}",
                block.absolute_x,
                block.absolute_y,
                block.absolute_z
            );
            let dir = (mth_atan2(block.fluid_flow_z as f64, block.fluid_flow_x as f64) as f32)
                - 1.5707964_f32;
            fluid_log!(
                "top-uv-flowing-after-atan pos={},{},{} dir={:.6}",
                block.absolute_x,
                block.absolute_y,
                block.absolute_z,
                dir
            );
            let sin = mth_sin(dir) * 0.25;
            let cos = mth_cos(dir) * 0.25;
            fluid_log!(
                "top-uv-flowing-after-trig pos={},{},{} sin={:.6} cos={:.6}",
                block.absolute_x,
                block.absolute_y,
                block.absolute_z,
                sin,
                cos
            );
            shrink_fluid_uvs(
                [
                    (
                        sprite_u(sprite, 0.5 + (-cos - sin)),
                        sprite_v(sprite, 0.5 + -cos + sin),
                    ),
                    (
                        sprite_u(sprite, 0.5 + -cos + sin),
                        sprite_v(sprite, 0.5 + cos + sin),
                    ),
                    (
                        sprite_u(sprite, 0.5 + cos + sin),
                        sprite_v(sprite, 0.5 + (cos - sin)),
                    ),
                    (
                        sprite_u(sprite, 0.5 + (cos - sin)),
                        sprite_v(sprite, 0.5 + (-cos - sin)),
                    ),
                ],
                state.fluid_still.shrink,
            )
        };
        sink.profile()
            .add_optional_stage(PROFILE_FLUID_STILL_FLOWING_UV, uv_started);
        fluid_log!(
            "top-uv-end pos={},{},{} uv0={:.5},{:.5}",
            block.absolute_x,
            block.absolute_y,
            block.absolute_z,
            top[0].0,
            top[0].1
        );
        let top_started = profile_start(profile_fluid_substages);
        let top_aligned = fluid_top_aligned(render_heights);
        let top_facing = if top_aligned {
            MODEL_QUAD_FACING_POS_Y
        } else {
            MODEL_QUAD_FACING_UNASSIGNED
        };
        let top_face_kind = if fluid_top_crease_ne_sw(render_heights, top_aligned) {
            FLUID_FACE_TOP_NE_SW
        } else {
            FLUID_FACE_TOP_NW_SE
        };
        let top_record_uvs = if top_face_kind == FLUID_FACE_TOP_NE_SW {
            [top[3], top[0], top[1], top[2]]
        } else {
            top
        };
        fluid_log!(
            "top-face-before-quad pos={},{},{} facing={} kind={}",
            block.absolute_x,
            block.absolute_y,
            block.absolute_z,
            top_facing,
            top_face_kind
        );
        let top_face = fluid_semantic_native_face(
            state,
            block,
            top_facing,
            false,
            MODEL_QUAD_FACING_POS_Y as i32,
            0,
            top_face_kind,
            0.0,
            render_heights,
            [0.0; 4],
            top_record_uvs,
            color,
            1.0,
            light,
        );
        fluid_log!(
            "top-face-after-quad pos={},{},{} normal=0x{:08x}",
            block.absolute_x,
            block.absolute_y,
            block.absolute_z,
            top_face.packed_normal
        );
        if fluid_diag_enabled {
            fluid_semantic_record_diag(
                block,
                "top-record",
                state,
                top_face.facing,
                false,
                top_face_kind,
                0.0,
                render_heights,
                [0.0; 4],
                top_record_uvs,
                color,
                1.0,
                light,
                top_face.packed_normal,
            );
        }
        let append_started = profile_start(profile_fluid_substages);
        fluid_log!(
            "emit-top pos={},{},{} facing={} kind={} heights={:.4},{:.4},{:.4},{:.4}",
            block.absolute_x,
            block.absolute_y,
            block.absolute_z,
            top_face.facing,
            top_face_kind,
            render_heights[0],
            render_heights[1],
            render_heights[2],
            render_heights[3]
        );
        sink.emit(top_face)?;
        mark_renderable_fluid_sprite(sink, state, top_uses_still, false);
        fluid_log!(
            "emit-top-done pos={},{},{}",
            block.absolute_x,
            block.absolute_y,
            block.absolute_z
        );
        emitted += 1;
        sink.profile()
            .add_optional_stage(PROFILE_FLUID_NATIVE_QUAD_APPEND, append_started);
        sink.profile()
            .add_optional_stage(PROFILE_FLUID_TOP_FACE_CONSTRUCTION, top_started);
        if fluid_diag_enabled {
            fluid_diag(
                block,
                state,
                "top-emitted",
                cull_up,
                top_exposed,
                render_heights,
                color,
                light,
                Some(top_facing),
            );
        }
        let normal_backface_started = profile_start(profile_fluid_substages);
        if fluid_backward_up_face(block, state, states) {
            let backward_facing = if top_facing == MODEL_QUAD_FACING_POS_Y {
                MODEL_QUAD_FACING_NEG_Y
            } else {
                MODEL_QUAD_FACING_UNASSIGNED
            };
            let backward_face = flipped_fluid_back_face(top_face, backward_facing);
            if fluid_diag_enabled {
                fluid_semantic_record_diag(
                    block,
                    "top-back-record",
                    state,
                    backward_face.facing,
                    true,
                    top_face_kind,
                    0.0,
                    render_heights,
                    [0.0; 4],
                    top_record_uvs,
                    color,
                    1.0,
                    light,
                    backward_face.packed_normal,
                );
            }
            let append_started = profile_start(profile_fluid_substages);
            fluid_log!(
                "emit-top-back pos={},{},{} facing={} kind={}",
                block.absolute_x,
                block.absolute_y,
                block.absolute_z,
                backward_face.facing,
                top_face_kind
            );
            sink.emit(backward_face)?;
            mark_renderable_fluid_sprite(sink, state, top_uses_still, false);
            emitted += 1;
            sink.profile()
                .add_optional_stage(PROFILE_FLUID_NATIVE_QUAD_APPEND, append_started);
        }
        sink.profile()
            .add_optional_stage(PROFILE_FLUID_NORMAL_BACKFACE, normal_backface_started);
    }

    if !cull_down {
        let bottom_started = profile_start(profile_fluid_substages);
        let sprite = state.fluid_still;
        let material_started = profile_start(profile_fluid_substages);
        let uvs = [
            (sprite.u0, sprite.v1),
            (sprite.u0, sprite.v0),
            (sprite.u1, sprite.v0),
            (sprite.u1, sprite.v1),
        ];
        sink.profile()
            .add_optional_stage(PROFILE_FLUID_MATERIAL_SPRITE_ROUTING, material_started);
        let bottom_face = fluid_semantic_native_face(
            state,
            block,
            MODEL_QUAD_FACING_NEG_Y,
            false,
            0,
            0,
            FLUID_FACE_BOTTOM,
            y_offset,
            [0.0; 4],
            [0.0; 4],
            uvs,
            color,
            1.0,
            light,
        );
        if fluid_diag_enabled {
            fluid_semantic_record_diag(
                block,
                "bottom-record",
                state,
                MODEL_QUAD_FACING_NEG_Y,
                false,
                FLUID_FACE_BOTTOM,
                y_offset,
                [0.0; 4],
                [0.0; 4],
                uvs,
                color,
                1.0,
                light,
                bottom_face.packed_normal,
            );
        }
        let append_started = profile_start(profile_fluid_substages);
        fluid_log!(
            "emit-bottom pos={},{},{} facing={}",
            block.absolute_x,
            block.absolute_y,
            block.absolute_z,
            bottom_face.facing
        );
        sink.emit(bottom_face)?;
        mark_renderable_fluid_sprite(sink, state, true, false);
        emitted += 1;
        sink.profile()
            .add_optional_stage(PROFILE_FLUID_NATIVE_QUAD_APPEND, append_started);
        sink.profile()
            .add_optional_stage(PROFILE_FLUID_BOTTOM_FACE_CONSTRUCTION, bottom_started);
    }

    let flow_sprite = state.fluid_flow;
    let flow_u1 = flow_sprite.u0;
    let flow_u2 = sprite_mid_u(flow_sprite);
    let flow_v3 = sprite_mid_v(flow_sprite);
    let overlay_sprite = state.fluid_overlay;
    let overlay_u1 = overlay_sprite.u0;
    let overlay_u2 = sprite_mid_u(overlay_sprite);
    let overlay_v3 = sprite_mid_v(overlay_sprite);

    for (index, (dir, facing, opposite_facing, shade, h1, h2, x1, z1, x2, z2)) in
        sides.iter().copied().enumerate()
    {
        if visible_sides[index] {
            let side_started = profile_start(profile_fluid_substages);
            let material_started = profile_start(profile_fluid_substages);
            let is_overlay = overlay_sides[index];
            let (sprite, u1, u2, v3) = if is_overlay {
                (overlay_sprite, overlay_u1, overlay_u2, overlay_v3)
            } else {
                (flow_sprite, flow_u1, flow_u2, flow_v3)
            };
            sink.profile()
                .add_optional_stage(PROFILE_FLUID_MATERIAL_SPRITE_ROUTING, material_started);
            let uv_started = profile_start(profile_fluid_substages);
            let v1 = sprite_v(sprite, (1.0 - h1) * 0.5);
            let v2 = sprite_v(sprite, (1.0 - h2) * 0.5);
            let uvs = [(u2, v2), (u2, v3), (u1, v3), (u1, v1)];
            sink.profile()
                .add_optional_stage(PROFILE_FLUID_STILL_FLOWING_UV, uv_started);
            let side_face = fluid_semantic_native_face(
                state,
                block,
                facing,
                false,
                dir,
                MODEL_QUAD_FLAG_PARALLEL | MODEL_QUAD_FLAG_ALIGNED,
                FLUID_FACE_SIDE,
                y_offset,
                [h1, h2, 0.0, 0.0],
                [x1, z1, x2, z2],
                uvs,
                color,
                shade,
                light,
            );
            if fluid_diag_enabled {
                fluid_semantic_record_diag(
                    block,
                    "side-record",
                    state,
                    facing,
                    false,
                    FLUID_FACE_SIDE,
                    y_offset,
                    [h1, h2, 0.0, 0.0],
                    [x1, z1, x2, z2],
                    uvs,
                    color,
                    shade,
                    light,
                    side_face.packed_normal,
                );
            }
            let append_started = profile_start(profile_fluid_substages);
            fluid_log!(
                "emit-side pos={},{},{} dir={} facing={} overlay={} h={:.4},{:.4}",
                block.absolute_x,
                block.absolute_y,
                block.absolute_z,
                dir,
                side_face.facing,
                is_overlay,
                h1,
                h2
            );
            sink.emit(side_face)?;
            mark_renderable_fluid_sprite(sink, state, false, is_overlay);
            emitted += 1;
            sink.profile()
                .add_optional_stage(PROFILE_FLUID_NATIVE_QUAD_APPEND, append_started);
            if !is_overlay {
                let normal_backface_started = profile_start(profile_fluid_substages);
                let back_face = flipped_fluid_back_face(side_face, opposite_facing);
                if fluid_diag_enabled {
                    fluid_semantic_record_diag(
                        block,
                        "side-back-record",
                        state,
                        opposite_facing,
                        true,
                        FLUID_FACE_SIDE,
                        y_offset,
                        [h1, h2, 0.0, 0.0],
                        [x1, z1, x2, z2],
                        uvs,
                        color,
                        shade,
                        light,
                        back_face.packed_normal,
                    );
                }
                let append_started = profile_start(profile_fluid_substages);
                fluid_log!(
                    "emit-side-back pos={},{},{} dir={} facing={} h={:.4},{:.4}",
                    block.absolute_x,
                    block.absolute_y,
                    block.absolute_z,
                    dir,
                    back_face.facing,
                    h1,
                    h2
                );
                sink.emit(back_face)?;
                mark_renderable_fluid_sprite(sink, state, false, false);
                emitted += 1;
                sink.profile()
                    .add_optional_stage(PROFILE_FLUID_NATIVE_QUAD_APPEND, append_started);
                sink.profile()
                    .add_optional_stage(PROFILE_FLUID_NORMAL_BACKFACE, normal_backface_started);
            }
            sink.profile()
                .add_optional_stage(PROFILE_FLUID_SIDE_FACE_CONSTRUCTION, side_started);
        }
    }
    sink.profile()
        .add_stage(PROFILE_FLUID_GEOM_UV, geometry_started);
    fluid_log!(
        "end pos={},{},{} emitted={}",
        block.absolute_x,
        block.absolute_y,
        block.absolute_z,
        emitted
    );
    Ok(emitted)
}
