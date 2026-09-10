//! Native sheeted/decal item-foil projection prerequisite.
//!
//! Inputs are copied model poses and emitted vertex semantics, not caller-made
//! decal UVs or GPU handles. This remains private and unadmitted until the GUI
//! and world mesh contracts carry these semantics and paired captures pass.
use super::error::{GalError, GalResult};

#[derive(Clone, Copy, Debug)]
pub(super) enum FoilDisplayContext {
    Gui,
    FirstPerson,
    World,
}

pub(super) struct SpecialFoilProjection {
    inverse_model: [f32; 9],
    inverse_normal: [f32; 9],
    translation: [f32; 3],
    context_scale: f32,
}

impl SpecialFoilProjection {
    /// Native GUI layout owns both the raster pose and its normal basis. No
    /// Java offscreen transform or inverse projection is needed for flat items.
    pub(super) fn from_native_gui_model(model_pose: [f32;16]) -> GalResult<Self> {
        let inverse = inverse3([model_pose[0],model_pose[1],model_pose[2],
            model_pose[4],model_pose[5],model_pose[6],model_pose[8],model_pose[9],model_pose[10]])?;
        let normal_pose = std::array::from_fn(|index| inverse[(index % 3)*3 + index/3]);
        Self::new(model_pose,normal_pose,FoilDisplayContext::Gui)
    }
    pub(super) fn new(
        model_pose: [f32; 16],
        normal_pose: [f32; 9],
        context: FoilDisplayContext,
    ) -> GalResult<Self> {
        if model_pose.iter().any(|x| !x.is_finite())
            || model_pose[3] != 0.0
            || model_pose[7] != 0.0
            || model_pose[11] != 0.0
            || model_pose[15] != 1.0
        {
            return Err(GalError::invalid_argument(
                "special foil requires a finite affine model pose",
            ));
        }
        Ok(Self {
            inverse_model: inverse3([
                model_pose[0],
                model_pose[1],
                model_pose[2],
                model_pose[4],
                model_pose[5],
                model_pose[6],
                model_pose[8],
                model_pose[9],
                model_pose[10],
            ])?,
            inverse_normal: inverse3(normal_pose)?,
            translation: [model_pose[12], model_pose[13], model_pose[14]],
            // Frozen ItemRenderer scales every component of the copied pose,
            // including homogeneous W, not merely its upper-left 3x3 matrix.
            context_scale: match context {
                FoilDisplayContext::Gui => 0.5,
                FoilDisplayContext::FirstPerson => 0.75,
                FoilDisplayContext::World => 1.0,
            },
        })
    }

    pub(super) fn texture_uv(&self, position: [f32; 3], normal: [f32; 3]) -> GalResult<[f32; 2]> {
        if position.iter().chain(normal.iter()).any(|x| !x.is_finite()) {
            return Err(GalError::invalid_argument("non-finite special foil vertex"));
        }
        let p = transform3(
            self.inverse_model,
            std::array::from_fn(|i| position[i] - self.translation[i]),
        );
        let n = transform3(self.inverse_normal, normal);
        if p.iter().chain(n.iter()).any(|x| !x.is_finite()) {
            return Err(GalError::invalid_argument(
                "special foil projection overflow",
            ));
        }
        // Exact axis rotations corresponding to SheetedDecalTextureGenerator:
        // rotateY(PI), rotateX(-PI/2), then Direction.getRotation(). Preserve
        // Frozen's zero-vector NORTH and Y,Z,X tie ordering. No UV wrapping:
        // native standard glint animation and the explicit sampler follow this.
        let [x, y, z] = p;
        let uv = if n == [0.0; 3] {
            [-x, -y]
        } else if n[1].abs() >= n[2].abs() && n[1].abs() >= n[0].abs() {
            if n[1] <= 0.0 {
                [x, -z]
            } else {
                [x, z]
            }
        } else if n[2].abs() >= n[0].abs() {
            if n[2] <= 0.0 {
                [-x, -y]
            } else {
                [x, -y]
            }
        } else if n[0] <= 0.0 {
            [-z, -y]
        } else {
            [z, -y]
        };
        let result = uv.map(|v| v / self.context_scale * 0.0078125);
        if result.iter().any(|x| !x.is_finite()) {
            return Err(GalError::invalid_argument("special foil UV overflow"));
        }
        Ok(result)
    }
}

fn transform3(m: [f32; 9], p: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|r| m[r] * p[0] + m[3 + r] * p[1] + m[6 + r] * p[2])
}

fn inverse3(m: [f32; 9]) -> GalResult<[f32; 9]> {
    if m.iter().any(|v| !v.is_finite()) {
        return Err(GalError::invalid_argument("non-finite special foil pose"));
    }
    let [a, d, g, b, e, h, c, f, i] = m.map(f64::from);
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    if det == 0.0 || !det.is_finite() {
        return Err(GalError::invalid_argument("singular special foil pose"));
    }
    let inverse = [
        e * i - f * h,
        f * g - d * i,
        d * h - e * g,
        c * h - b * i,
        a * i - c * g,
        b * g - a * h,
        b * f - c * e,
        c * d - a * f,
        a * e - b * d,
    ]
    .map(|v| (v / det) as f32);
    if inverse.iter().any(|v| !v.is_finite()) {
        return Err(GalError::invalid_argument("special foil inverse overflow"));
    }
    Ok(inverse)
}

#[cfg(test)]
mod tests {
    use super::*;
    const ID4: [f32; 16] = [
        1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
    ];
    const ID3: [f32; 9] = [1., 0., 0., 0., 1., 0., 0., 0., 1.];

    #[test]
    fn all_faces_and_display_scales_match_decal_axis_convention() {
        let faces = [
            ([0., -1., 0.], [2., -5.]),
            ([0., 1., 0.], [2., 5.]),
            ([0., 0., -1.], [-2., -3.]),
            ([0., 0., 1.], [2., -3.]),
            ([-1., 0., 0.], [-5., -3.]),
            ([1., 0., 0.], [5., -3.]),
        ];
        for (context, scale) in [
            (FoilDisplayContext::Gui, 2.),
            (FoilDisplayContext::FirstPerson, 4. / 3.),
            (FoilDisplayContext::World, 1.),
        ] {
            let projection = SpecialFoilProjection::new(ID4, ID3, context).unwrap();
            for (normal, expected) in faces {
                let actual = projection.texture_uv([2., 3., 5.], normal).unwrap();
                for axis in 0..2 {
                    assert!((actual[axis] - expected[axis] * scale / 128.).abs() < 1e-7);
                }
            }
        }
    }

    #[test]
    fn translation_nonuniform_pose_and_normal_pose_are_inverted_independently() {
        let mut pose = ID4;
        pose[0] = 2.;
        pose[5] = -3.;
        pose[10] = 4.;
        pose[12] = 10.;
        pose[13] = 20.;
        pose[14] = 30.;
        let normals = [0.5, 0., 0., 0., -1. / 3., 0., 0., 0., 0.25];
        let projection =
            SpecialFoilProjection::new(pose, normals, FoilDisplayContext::Gui).unwrap();
        assert_eq!(
            projection
                .texture_uv([14., 11., 50.], [0., -1. / 3., 0.])
                .unwrap(),
            [4. / 128., 10. / 128.]
        );
        let rotated = [
            0., 2., 0., 0., -3., 0., 0., 0., 0., 0., 4., 0., 10., 20., 30., 1.,
        ];
        let rotated_normal = [0., 0.5, 0., -1. / 3., 0., 0., 0., 0., 0.25];
        let projection =
            SpecialFoilProjection::new(rotated, rotated_normal, FoilDisplayContext::Gui).unwrap();
        assert_eq!(
            projection
                .texture_uv([1., 24., 50.], [-1. / 3., 0., 0.])
                .unwrap(),
            [4. / 128., 10. / 128.]
        );
    }

    #[test]
    fn decal_uvs_remain_unwrapped_for_native_animation_and_explicit_sampler() {
        let p = SpecialFoilProjection::new(ID4, ID3, FoilDisplayContext::World).unwrap();
        assert_eq!(
            p.texture_uv([512., 1024., 2048.], [0., 1., 0.]).unwrap(),
            [4., 16.]
        );
    }

    #[test]
    fn zero_and_ties_preserve_frozen_direction_order() {
        let p = SpecialFoilProjection::new(ID4, ID3, FoilDisplayContext::World).unwrap();
        for (normal, expected) in [
            ([0., 0., 0.], [-2., -3.]),
            ([1., 1., 1.], [2., 5.]),
            ([1., -1., 1.], [2., -5.]),
            ([1., 0., -1.], [-2., -3.]),
        ] {
            assert_eq!(
                p.texture_uv([2., 3., 5.], normal).unwrap(),
                expected.map(|v| v / 128.)
            );
        }
    }

    #[test]
    fn invalid_poses_and_vertices_cannot_generate_native_draw_data() {
        assert!(SpecialFoilProjection::new([0.; 16], ID3, FoilDisplayContext::Gui).is_err());
        assert!(SpecialFoilProjection::new(ID4, [0.; 9], FoilDisplayContext::Gui).is_err());
        let mut pose = ID4;
        pose[3] = 1.;
        assert!(SpecialFoilProjection::new(pose, ID3, FoilDisplayContext::Gui).is_err());
        let mut pose = ID4;
        pose[0] = f32::from_bits(1);
        assert!(SpecialFoilProjection::new(pose, ID3, FoilDisplayContext::Gui).is_err());
        let p = SpecialFoilProjection::new(ID4, ID3, FoilDisplayContext::World).unwrap();
        assert!(p.texture_uv([f32::NAN, 0., 0.], [0., 1., 0.]).is_err());
        assert!(p.texture_uv([0.; 3], [f32::INFINITY, 0., 0.]).is_err());
        let mut pose = ID4;
        pose[12] = -f32::MAX;
        let p = SpecialFoilProjection::new(pose, ID3, FoilDisplayContext::World).unwrap();
        assert!(p.texture_uv([f32::MAX, 0., 0.], [0., 1., 0.]).is_err());
    }
}
