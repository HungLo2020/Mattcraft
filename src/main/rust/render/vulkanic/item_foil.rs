//! Standard item foil semantics. No texture handles or caller-computed matrices.
//!
//! This does not implement special/decal foil, whose coordinates also depend on
//! model geometry. Those requests must not be admitted as standard item foil.

use super::error::{GalError, GalResult};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StandardItemFoil {
    pub clock_millis: u64,
    pub speed: f64,
    pub strength: f32,
}

impl StandardItemFoil {
    pub fn decode(mode: u32, clock_millis: u64, speed: f64, strength: f32) -> GalResult<Option<Self>> {
        match mode {
            0 if clock_millis == 0 && speed.to_bits() == 0 && strength.to_bits() == 0 => Ok(None),
            1 => {
                let value = Self { clock_millis, speed, strength };
                value.validate()?;
                Ok(Some(value))
            }
            _ => Err(GalError::invalid_argument("invalid or noncanonical standard item foil mode")),
        }
    }

    pub fn validate(self) -> GalResult<()> {
        if self.clock_millis > i64::MAX as u64
            || !self.speed.is_finite()
            || !(0.0..=1.0).contains(&self.speed)
            || !self.strength.is_finite()
            || !(0.0..=1.0).contains(&self.strength)
        {
            return Err(GalError::invalid_argument("invalid standard item foil inputs"));
        }
        Ok(())
    }

    /// Explicit column-major affine transform for original resource UVs.
    /// Shared by mesh preparation and eventual world-model uniform lowering;
    /// callers provide time/options, never a precomputed animation matrix.
    pub fn texture_transform(self) -> GalResult<[[f32; 2]; 3]> {
        self.validate()?;
        // Match Frozen RenderStateShard.setupGlintTexturing: multiply in
        // double precision, then truncate/saturate to signed long before the
        // two independent remainders. Reducing the clock first is not equal.
        let ticks = (self.clock_millis as f64 * self.speed * 8.0) as i64;
        let g = (ticks % 110_000) as f32 / 110_000.0;
        let h = (ticks % 30_000) as f32 / 30_000.0;
        let angle = (std::f64::consts::PI / 18.0) as f32;
        let sin = (angle as f64).sin() as f32;
        // JOML's default cosFromSin at this positive first-quadrant angle.
        let cos = ((1.0 - sin * sin) as f64).sqrt() as f32;
        let sin = sin * 8.0;
        let cos = cos * 8.0;
        Ok([[cos, sin], [-sin, cos], [-g, h]])
    }

    pub fn texture_uv(self, source_uv: [f32; 2]) -> GalResult<[f32; 2]> {
        let matrix = self.texture_transform()?;
        if source_uv.iter().any(|value| !value.is_finite()) {
            return Err(GalError::invalid_argument("non-finite item foil source UV"));
        }
        let uv = [
            matrix[0][0] * source_uv[0] + matrix[1][0] * source_uv[1] + matrix[2][0],
            matrix[0][1] * source_uv[0] + matrix[1][1] * source_uv[1] + matrix[2][1],
        ];
        if uv.iter().any(|value| !value.is_finite()) {
            return Err(GalError::invalid_argument("item foil UV overflow"));
        }
        // Do not wrap here. The explicit repeating sampler owns addressing,
        // including derivatives and filtering across a repeat boundary.
        Ok(uv)
    }

    /// std430 ItemFoilInstance: two affine rows and one parameter vector.
    /// This is Rust-owned shader packing, not part of the caller/FFI ABI.
    pub fn packed_instance(self) -> GalResult<[u8; 48]> {
        let matrix = self.texture_transform()?;
        let values = [
            matrix[0][0], matrix[1][0], matrix[2][0], 0.0,
            matrix[0][1], matrix[1][1], matrix[2][1], 0.0,
            self.strength, 0.0, 0.0, 0.0,
        ];
        let mut bytes = [0u8; 48];
        for (target, value) in bytes.chunks_exact_mut(4).zip(values) {
            target.copy_from_slice(&value.to_le_bytes());
        }
        Ok(bytes)
    }

    pub fn color(self) -> GalResult<[f32; 4]> {
        self.validate()?;
        // Frozen glint.fsh multiplies RGB by GlintAlpha, not texture alpha.
        // Keep the semantic floating-point strength; do not quantize to ARGB.
        Ok([self.strength, self.strength, self.strength, 1.0])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn foil(clock_millis: u64) -> StandardItemFoil {
        StandardItemFoil { clock_millis, speed: 0.5, strength: 0.5 }
    }

    #[test]
    fn instance_packing_keeps_affine_rows_and_unquantized_rgb_strength() {
        let input = StandardItemFoil { strength: 0.1234567, ..foil(12_345) };
        let bytes = input.packed_instance().unwrap();
        let values = bytes.chunks_exact(4).map(|v| f32::from_le_bytes(v.try_into().unwrap()))
            .collect::<Vec<_>>();
        let matrix = input.texture_transform().unwrap();
        assert_eq!(&values[..4], &[matrix[0][0], matrix[1][0], matrix[2][0], 0.0]);
        assert_eq!(&values[4..8], &[matrix[0][1], matrix[1][1], matrix[2][1], 0.0]);
        assert_eq!(&values[8..], &[input.strength, 0.0, 0.0, 0.0]);
        assert!(StandardItemFoil { strength: f32::NAN, ..input }.packed_instance().is_err());
    }

    #[test]
    fn explicit_transform_preserves_frozen_column_order_and_translation() {
        let matrix = foil(12_345).texture_transform().unwrap();
        let expected = [
            [7.878462315, 1.389185429],
            [-1.389185429, 7.878462315],
            [-0.448909104, 0.646000028],
        ];
        for column in 0..3 {
            for row in 0..2 {
                assert!((matrix[column][row] - expected[column][row]).abs() < 0.000002);
            }
        }
        // Strength belongs to fragment RGB, never UV animation or alpha.
        assert_eq!(matrix, StandardItemFoil { strength: 0.0, ..foil(12_345) }
            .texture_transform().unwrap());
        // Animated translation must not alter the source-space basis.
        assert_eq!(&matrix[..2], &foil(0).texture_transform().unwrap()[..2]);
        assert!(foil(u64::MAX).texture_transform().is_err());
    }

    #[test]
    fn standard_foil_matches_frozen_joml_matrix_golden_coordinates() {
        // Independently evaluated with Frozen's Matrix4f translation,
        // rotateZ(float(PI/18)), scale(8), using JOML 1.10.5. This is a CPU
        // arithmetic tolerance, not an image-parity acceptance threshold.
        let cases = [
            (0, [1.0, 1.0], [6.489276409, 9.267646790]),
            (0, [0.25, 0.75], [0.927726388, 6.256142616]),
            (12_345, [0.0, 0.0], [-0.448909104, 0.646000028]),
            (12_345, [0.25, 0.75], [0.478817225, 6.902142525]),
            (12_345, [-0.5, 2.0], [-7.166510582, 15.708331108]),
            (13_750, [1.0, 1.0], [5.989276409, 10.100980759]),
            (3_750, [1.0, 1.0], [6.352912903, 9.767646790]),
            (i64::MAX as u64, [0.0, 0.0], [-0.780063629, 0.860233307]),
        ];
        for (clock, source, expected) in cases {
            let actual = foil(clock).texture_uv(source).unwrap();
            for axis in 0..2 {
                assert!((actual[axis] - expected[axis]).abs() <= 0.000002,
                    "clock={clock} source={source:?}: {actual:?} != {expected:?}");
            }
        }
    }

    #[test]
    fn standard_foil_has_independent_periods_and_static_zero_speed() {
        assert_eq!(foil(7_500).texture_uv([0.0; 2]).unwrap(), [-30_000.0 / 110_000.0, 0.0]);
        assert_eq!(foil(27_500).texture_uv([0.0; 2]).unwrap(), [0.0, 20_000.0 / 30_000.0]);
        let mut stopped = foil(i64::MAX as u64);
        stopped.speed = 0.0;
        assert_eq!(stopped.texture_uv([0.25, 0.75]).unwrap(), foil(0).texture_uv([0.25, 0.75]).unwrap());
    }

    #[test]
    fn standard_foil_preserves_strength_precision_and_texture_alpha() {
        for strength in [0.0, 0.1, 0.5, 1.0] {
            let mut input = foil(0);
            input.strength = strength;
            assert_eq!(input.color().unwrap(), [strength, strength, strength, 1.0]);
        }
    }

    #[test]
    fn standard_foil_rejects_invalid_semantics_and_overflow() {
        for speed in [f64::NAN, f64::INFINITY, -0.01, 1.01] {
            assert!(StandardItemFoil { speed, ..foil(0) }.texture_transform().is_err());
        }
        for strength in [f32::NAN, f32::INFINITY, -0.01, 1.01] {
            assert!(StandardItemFoil { strength, ..foil(0) }.color().is_err());
        }
        assert!(foil(u64::MAX).validate().is_err());
        for uv in [[f32::NAN, 0.0], [0.0, f32::INFINITY], [f32::MAX, 0.0]] {
            assert!(foil(0).texture_uv(uv).is_err());
        }
    }
}
