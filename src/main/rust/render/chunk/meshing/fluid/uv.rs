//! Fluid sprite, UV, overlay, and Java math compatibility helpers.
//!
//! Still, flowing, and overlay sprites keep Java's atlas interpolation and shrink
//! behavior. Flowing top faces use the same sine table and atan2 approximation so
//! replayed native meshes retain byte-for-byte UV compatibility where possible.

use super::*;

#[inline(always)]
pub(in crate::render::chunk::meshing) fn fluid_sprite_mask(
    fluid_type: i32,
    still: bool,
    overlay: bool,
) -> i32 {
    match (fluid_type, still, overlay) {
        (FLUID_WATER, true, _) => FLUID_SPRITE_WATER_STILL,
        (FLUID_WATER, false, true) => FLUID_SPRITE_WATER_OVERLAY,
        (FLUID_WATER, false, false) => FLUID_SPRITE_WATER_FLOW,
        (FLUID_LAVA, true, _) => FLUID_SPRITE_LAVA_STILL,
        (FLUID_LAVA, false, _) => FLUID_SPRITE_LAVA_FLOW,
        _ => 0,
    }
}

pub(in crate::render::chunk::meshing) fn fluid_side_uses_overlay(
    block: &NativeSectionBlockRecord,
    state: NativeMeshingState,
    states: &[Option<NativeMeshingState>],
    direction: i32,
) -> bool {
    if state.fluid_type != FLUID_WATER || state.fluid_overlay_valid == 0 {
        return false;
    }
    let (dx, dy, dz) = dir_step(direction);
    let Some(neighbor_id) = neighborhood_state_id(block, dx, dy, dz) else {
        return false;
    };
    let Some(neighbor) = state_by_id(states, neighbor_id) else {
        return false;
    };
    (neighbor.flags & STATE_FLAG_FLUID_OVERLAY_TRANSPARENT) != 0
}

pub(in crate::render::chunk::meshing) fn sprite_u(sprite: FluidSprite, value: f32) -> f32 {
    sprite.u0 + (sprite.u1 - sprite.u0) * value
}

pub(in crate::render::chunk::meshing) fn sprite_v(sprite: FluidSprite, value: f32) -> f32 {
    sprite.v0 + (sprite.v1 - sprite.v0) * value
}

pub(in crate::render::chunk::meshing) fn mth_sin(value: f32) -> f32 {
    mth_sin_table()[((value * 10430.378_f32) as i32 & 65_535) as usize]
}

pub(in crate::render::chunk::meshing) fn mth_cos(value: f32) -> f32 {
    mth_sin_table()[((value * 10430.378_f32 + 16_384.0_f32) as i32 & 65_535) as usize]
}

pub(in crate::render::chunk::meshing) fn mth_sin_table() -> &'static [f32; 65_536] {
    static TABLE: OnceLock<Box<[f32; 65_536]>> = OnceLock::new();
    TABLE
        .get_or_init(|| {
            let mut table = vec![0.0_f32; 65_536].into_boxed_slice();
            for (index, value) in table.iter_mut().enumerate() {
                *value = ((index as f64) * std::f64::consts::PI * 2.0 / 65_536.0).sin() as f32;
            }
            match table.try_into() {
                Ok(table) => table,
                Err(_) => unreachable!("native fluid sine table has a fixed length"),
            }
        })
        .as_ref()
}

#[cfg(test)]
pub(in crate::render::chunk::meshing) fn flowing_top_trig_for_test(
    flow_x: f32,
    flow_z: f32,
) -> (f32, f32, f32) {
    let dir = (mth_atan2(flow_z as f64, flow_x as f64) as f32) - 1.5707964_f32;
    (dir, mth_sin(dir), mth_cos(dir))
}

pub(in crate::render::chunk::meshing) fn mth_atan2(mut y: f64, mut x: f64) -> f64 {
    let square = x * x + y * y;
    if square.is_nan() {
        return f64::NAN;
    }

    let y_negative = y < 0.0;
    if y_negative {
        y = -y;
    }

    let x_negative = x < 0.0;
    if x_negative {
        x = -x;
    }

    let swapped = y > x;
    if swapped {
        std::mem::swap(&mut x, &mut y);
    }

    let inv = mth_fast_inv_sqrt(square);
    x *= inv;
    y *= inv;

    let biased = f64::from_bits(4_805_340_802_404_319_232_u64) + y;
    let index = (biased.to_bits() as u32 as usize).min(256);
    let asin = (index as f64 / 256.0).asin();
    let cos = asin.cos();
    let lookup = biased - f64::from_bits(4_805_340_802_404_319_232_u64);
    let delta = y * cos - x * lookup;
    let correction = (6.0 + delta * delta) * delta * (1.0 / 6.0);
    let mut angle = asin + correction;

    if swapped {
        angle = std::f64::consts::FRAC_PI_2 - angle;
    }
    if x_negative {
        angle = std::f64::consts::PI - angle;
    }
    if y_negative {
        angle = -angle;
    }

    angle
}

pub(in crate::render::chunk::meshing) fn mth_fast_inv_sqrt(value: f64) -> f64 {
    let half = 0.5 * value;
    let bits = 6_910_469_410_427_058_090_u64.wrapping_sub(value.to_bits() >> 1);
    let estimate = f64::from_bits(bits);
    estimate * (1.5 - half * estimate * estimate)
}

#[inline(always)]
pub(in crate::render::chunk::meshing) fn sprite_mid_u(sprite: FluidSprite) -> f32 {
    (sprite.u0 + sprite.u1) * 0.5
}

#[inline(always)]
pub(in crate::render::chunk::meshing) fn sprite_mid_v(sprite: FluidSprite) -> f32 {
    (sprite.v0 + sprite.v1) * 0.5
}

pub(in crate::render::chunk::meshing) fn still_fluid_top_uvs(
    sprite: FluidSprite,
) -> [(f32, f32); 4] {
    if sprite.shrink == 0.0 {
        return [
            (sprite.u0, sprite.v0),
            (sprite.u0, sprite.v1),
            (sprite.u1, sprite.v1),
            (sprite.u1, sprite.v0),
        ];
    }
    let mid_u = sprite_mid_u(sprite);
    let mid_v = sprite_mid_v(sprite);
    let u0 = sprite.u0 + (mid_u - sprite.u0) * sprite.shrink;
    let u1 = sprite.u1 + (mid_u - sprite.u1) * sprite.shrink;
    let v0 = sprite.v0 + (mid_v - sprite.v0) * sprite.shrink;
    let v1 = sprite.v1 + (mid_v - sprite.v1) * sprite.shrink;
    [(u0, v0), (u0, v1), (u1, v1), (u1, v0)]
}

pub(in crate::render::chunk::meshing) fn shrink_fluid_uvs(
    mut uvs: [(f32, f32); 4],
    shrink: f32,
) -> [(f32, f32); 4] {
    if shrink == 0.0 {
        return uvs;
    }
    let avg_u = (uvs[0].0 + uvs[1].0 + uvs[2].0 + uvs[3].0) * 0.25;
    let avg_v = (uvs[0].1 + uvs[1].1 + uvs[2].1 + uvs[3].1) * 0.25;
    for uv in &mut uvs {
        uv.0 += (avg_u - uv.0) * shrink;
        uv.1 += (avg_v - uv.1) * shrink;
    }
    uvs
}
