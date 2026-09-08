//! Semantic vanilla terrain placement, independent of region GPU storage.
//! Preserve the authored camera precision contract before narrowing to the
//! explicit instance matrix. No renderer state or native handles are inputs.
use crate::render::vulkanic::error::{GalError, GalResult};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerrainSectionPlacement {
    pub origin: [i32; 3],
    pub camera: [f64; 3],
}

impl TerrainSectionPlacement {
    pub fn lower(self) -> GalResult<[f32; 16]> {
        if self.origin.iter().any(|value| value % 16 != 0 || i64::from(*value).abs() > 30_000_000)
            || self.camera.iter().any(|value| !value.is_finite() || value.abs() > 30_000_000.0) {
            return Err(GalError::invalid_argument("invalid semantic terrain section placement"));
        }
        let mut transform = [0.0; 16];
        for index in [0,5,10,15] { transform[index] = 1.0; }
        for axis in 0..3 {
            // This grouping is part of vanilla terrain's position-precision
            // policy, not a request to allocate or borrow renderer regions.
            let span = [128,64,128][axis];
            let group_origin = self.origin[axis].div_euclid(span) * span;
            let integral = self.camera[axis] as i32;
            let fractional = (self.camera[axis] - f64::from(integral)) as f32;
            let modifier = 128.0_f32.copysign(fractional);
            let fraction = (fractional + modifier) - modifier;
            let group_translation = (group_origin - integral) as f32 - fraction;
            transform[12 + axis] = group_translation + (self.origin[axis] - group_origin) as f32;
        }
        Ok(transform)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractional_camera_preserves_the_authored_precision_grid() {
        let matrix = TerrainSectionPlacement {origin:[112,80,528],camera:[150.5,101.62,530.5]}.lower().unwrap();
        assert_eq!(&matrix[12..15], &[-38.5,-21.6199951171875,-2.5]);
        assert_ne!(matrix[13], (80.0_f64-101.62) as f32,
            "direct narrowing loses the baseline camera-fraction quantization");
        assert_eq!([matrix[0],matrix[5],matrix[10],matrix[15]], [1.0;4]);
    }

    #[test]
    fn negative_coordinates_and_group_boundaries_keep_section_spacing() {
        for camera in [-101.62,101.62,29_999_980.62,-29_999_980.62] {
            let near = (camera as i32).div_euclid(16)*16;
            for axis in 0..3 {
                let mut origin = [near;3];
                let first = TerrainSectionPlacement {origin,camera:[camera;3]}.lower().unwrap();
                origin[axis] += 16;
                let second = TerrainSectionPlacement {origin,camera:[camera;3]}.lower().unwrap();
                assert_eq!(second[12+axis]-first[12+axis],16.0);
            }
        }
        let negative = TerrainSectionPlacement {origin:[-96;3],camera:[-101.62;3]}.lower().unwrap();
        assert_eq!(&negative[12..15], &[5.6199951171875;3]);
    }

    #[test]
    fn invalid_inputs_are_rejected_before_any_render_resources_exist() {
        for camera in [f64::NAN,f64::INFINITY,30_000_001.0] {
            assert!(TerrainSectionPlacement {origin:[0;3],camera:[camera;3]}.lower().is_err());
        }
        for origin in [1,i32::MIN,i32::MAX] {
            assert!(TerrainSectionPlacement {origin:[origin;3],camera:[0.0;3]}.lower().is_err());
        }
    }
}
