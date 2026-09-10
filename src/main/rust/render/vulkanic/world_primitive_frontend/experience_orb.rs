//! Local orb geometry for the entity-lighting mesh path. Not admitted yet:
//! transport and the normal entity shader must be wired and paired against Frozen.
use super::*;

/// Immutable gameplay appearance. Model/camera transforms belong to the mesh
/// instance; no Java-expanded vertices, raster policy or native handles enter here.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ExperienceOrbAppearance {
    pub icon: u32,
    pub red: u8,
    pub blue: u8,
    pub packed_light: u32,
}

/// Entity placement before the renderer-specific billboard recipe. Rust owns
/// the 0.1 vertical offset, camera-facing rotation and 0.3 model scale.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ExperienceOrbPlacement {
    pub entity_transform: [f32; 16],
    pub camera_orientation: [f32; 4],
    pub entity_id: i32,
}

impl ExperienceOrbPlacement {
    pub(crate) fn instance(
        self,
        mesh_key: u64,
        mesh_generation: u64,
        viewport: [u32; 2],
    ) -> GalResult<WorldMeshInstanceRequest> {
        if mesh_key == 0
            || mesh_generation == 0
            || viewport
                .iter()
                .any(|&axis| axis == 0 || axis > super::super::SEMANTIC_MAX_VIEWPORT_AXIS as u32)
        {
            return Err(GalError::invalid_argument(
                "orb requires a mesh identity and bounded viewport",
            ));
        }
        if self
            .entity_transform
            .iter()
            .chain(self.camera_orientation.iter())
            .any(|v| !v.is_finite())
            || [
                self.entity_transform[3],
                self.entity_transform[7],
                self.entity_transform[11],
                self.entity_transform[15],
            ] != [0.0, 0.0, 0.0, 1.0]
        {
            return Err(GalError::invalid_argument(
                "orb placement requires a finite affine entity transform",
            ));
        }
        let [x, y, z, w] = self.camera_orientation;
        let (xx, yy, zz, ww) = (x * x, y * y, z * z, w * w);
        let norm = xx + yy + zz + ww;
        if !norm.is_finite() || norm <= 1.0e-8 {
            return Err(GalError::invalid_argument(
                "orb camera orientation must be nonzero and bounded",
            ));
        }
        // JOML Matrix4f.rotate(Quaternionfc), used by Frozen's PoseStack,
        // preserves quaternion magnitudes; do not substitute Vector3f's
        // normalized quaternion transform here.
        let (xy, xz, yz, xw, yw, zw) = (x * y, x * z, y * z, x * w, y * w, z * w);
        let local = [
            (ww + xx - yy - zz) * 0.3,
            2.0 * (xy + zw) * 0.3,
            2.0 * (xz - yw) * 0.3,
            0.0,
            2.0 * (xy - zw) * 0.3,
            (ww - xx + yy - zz) * 0.3,
            2.0 * (yz + xw) * 0.3,
            0.0,
            2.0 * (xz + yw) * 0.3,
            2.0 * (yz - xw) * 0.3,
            (ww - xx - yy + zz) * 0.3,
            0.0,
            0.0,
            0.1,
            0.0,
            1.0,
        ];
        let transform = matrix4_column_major_multiply(self.entity_transform, local);
        if transform.iter().any(|v| !v.is_finite()) {
            return Err(GalError::invalid_argument(
                "orb billboard transform overflow",
            ));
        }
        Ok(WorldMeshInstanceRequest {
            item_foil: None,
            stratum: WORLD_STRATUM_ENTITY_MESH,
            mesh_key,
            mesh_generation,
            mesh_section_index: WORLD_MESH_SECTION_ALL,
            // Frozen ITEM_ENTITY_TRANSLUCENT_CULL retains LEQUAL + depth writes.
            depth_policy: WORLD_DEPTH_POLICY_TEST_WRITE,
            cull_policy: WORLD_CULL_BACK,
            winding: WORLD_WINDING_CCW,
            color_argb: 0xffffffff,
            entity_id: self.entity_id,
            entity_color_argb: 0,
            outline_color_argb: 0,
            flags: 0,
            block_entity_id: -1,
            transform,
            viewport_width: viewport[0],
            viewport_height: viewport[1],
        })
    }
}

impl ExperienceOrbAppearance {
    /// Build an explicit, Rust-owned mesh. Keys are GAL resource identities,
    /// never borrowed GPU handles. The caller owns lifetime and instance state.
    pub(crate) fn mesh(self, mesh_key: u64, mesh_generation: u64) -> GalResult<WorldMeshAsset> {
        let mesh = WorldMeshAsset {
            mesh_key,
            mesh_generation,
            vertex_layout_version: WORLD_MESH_VERTEX_LAYOUT_V2,
            index_type: IndexType::U16,
            vertices: self.vertices()?.to_vec(),
            index_bytes: INDICES.into_iter().flat_map(u16::to_le_bytes).collect(),
            sections: vec![WorldMeshSection {
                material_id: WORLD_MATERIAL_ID_TRANSLUCENT_CUTOUT_TEXTURED,
                texture_id: WORLD_MATERIAL_TEXTURE_EXPERIENCE_ORB,
                material_mode: WORLD_MATERIAL_MODE_TRANSLUCENT_CUTOUT,
                cull_policy: WORLD_CULL_BACK,
                winding: WORLD_WINDING_CCW,
                index_offset: 0,
                index_count: 6,
            }],
            entity_identity: "minecraft:experience_orb".to_owned(),
        };
        validate_mesh_asset(&mesh)?;
        Ok(mesh)
    }

    pub(super) fn vertices(self) -> GalResult<[WorldMeshVertex; 4]> {
        // Vanilla ExperienceOrb.getIcon selects 0..10 from its 4-column sheet.
        if self.icon > 10 {
            return Err(GalError::invalid_argument(
                "unsupported experience orb icon",
            ));
        }
        let u = (self.icon % 4) as f32 * 0.25;
        let v = (self.icon / 4) as f32 * 0.25;
        let color = 0x8000_ff00 | (self.red as u32) << 16 | self.blue as u32;
        Ok([
            ([-0.5, -0.25, 0.0], [u, v + 0.25]),
            ([0.5, -0.25, 0.0], [u + 0.25, v + 0.25]),
            ([0.5, 0.75, 0.0], [u + 0.25, v]),
            ([-0.5, 0.75, 0.0], [u, v]),
        ]
        .map(|(position, uv)| WorldMeshVertex {
            position,
            uv,
            shader_atlas_uv: uv,
            shader_block_id: -1,
            shader_material_type: 0,
            terrain_material_bits: 0,
            mid_block_packed: 0,
            color_argb: color,
            // Frozen submits (0,1,0), NOT the geometric +Z face normal.
            // The entity mesh instance transforms this for directional lighting.
            normal_packed: 0x0000_7f00,
            light: self.packed_light,
        }))
    }
}

pub(super) const INDICES: [u16; 6] = [0, 1, 2, 2, 3, 0];

#[cfg(test)]
mod tests {
    use super::*;

    fn placement() -> ExperienceOrbPlacement {
        ExperienceOrbPlacement {
            entity_transform: matrix4_identity(),
            camera_orientation: [0.0, 0.0, 0.0, 1.0],
            entity_id: 42,
        }
    }

    #[test]
    fn placement_matches_frozen_joml_billboard_recipe_and_depth_policy() {
        // JOML 1.10.5: translation(1.25,-.7,2.5).rotateY(.25), then
        // translate(0,.1,0).rotate(rotationXYZ(.37,-.81,.12)).scale(.3).
        let mut p = placement();
        let (s, c) = 0.25_f32.sin_cos();
        p.entity_transform = [
            c, 0.0, -s, 0.0, 0.0, 1.0, 0.0, 0.0, s, 0.0, c, 0.0, 1.25, -0.7, 2.5, 1.0,
        ];
        p.camera_orientation = [0.14553767, -0.39673626, -0.018175337, 0.90613943];
        let instance = p.instance(7, 2, [1280, 720]).unwrap();
        let expected = [
            0.2519499,
            -0.044525683,
            0.15664832,
            0.0,
            -0.0033460178,
            0.28709304,
            0.08698492,
            0.0,
            -0.16281906,
            -0.07479997,
            0.24061361,
            0.0,
            1.25,
            -0.59999996,
            2.5,
            1.0,
        ];
        for (actual, expected) in instance.transform.into_iter().zip(expected) {
            assert!((actual - expected).abs() < 1.0e-6, "{actual} != {expected}");
        }
        assert_eq!(instance.depth_policy, WORLD_DEPTH_POLICY_TEST_WRITE);
        assert_eq!(instance.stratum, WORLD_STRATUM_ENTITY_MESH);
        assert_eq!(instance.entity_id, 42);
        assert_eq!(instance.entity_color_argb, 0);
        assert_eq!(instance.block_entity_id, -1);
    }

    #[test]
    fn placement_rejects_invalid_inputs_before_resource_work() {
        let p = placement();
        for viewport in [[0, 720], [1280, 0], [u32::MAX, 720]] {
            assert!(p.instance(7, 2, viewport).is_err());
        }
        assert!(p.instance(0, 2, [1280, 720]).is_err());
        assert!(p.instance(7, 0, [1280, 720]).is_err());
        for q in [[0.0; 4], [f32::NAN, 0.0, 0.0, 1.0], [f32::MAX; 4]] {
            assert!(ExperienceOrbPlacement {
                camera_orientation: q,
                ..p
            }
            .instance(7, 2, [1280, 720])
            .is_err());
        }
        for (index, value) in [(0, f32::NAN), (3, 0.1), (15, 0.0)] {
            let mut bad = p;
            bad.entity_transform[index] = value;
            assert!(bad.instance(7, 2, [1280, 720]).is_err());
        }
    }

    #[test]
    fn explicit_mesh_uses_blended_alpha_cutout_and_validated_resource_identity() {
        let appearance = ExperienceOrbAppearance {
            icon: 10,
            red: 255,
            blue: 51,
            packed_light: 0x00f00070,
        };
        let mesh = appearance.mesh(7, 2).unwrap();
        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.index_bytes.len(), 12);
        assert_eq!(mesh.entity_identity, "minecraft:experience_orb");
        let section = &mesh.sections[0];
        assert_eq!(
            section.material_mode,
            WORLD_MATERIAL_MODE_TRANSLUCENT_CUTOUT
        );
        assert_eq!(section.texture_id, WORLD_MATERIAL_TEXTURE_EXPERIENCE_ORB);
        assert_eq!(section.cull_policy, WORLD_CULL_BACK);
        assert_eq!(section.winding, WORLD_WINDING_CCW);
        for (index, expected) in INDICES.into_iter().enumerate() {
            assert_eq!(
                mesh_index_value(&mesh.index_bytes, mesh.index_type, index).unwrap(),
                expected as u32
            );
        }
        assert!(appearance.mesh(0, 2).is_err());
        assert!(appearance.mesh(7, 0).is_err());
    }

    #[test]
    fn frozen_orb_corners_winding_normal_and_pulse_channels_are_preserved() {
        let q = ExperienceOrbAppearance {
            icon: 0,
            red: 127,
            blue: 5,
            packed_light: 0x00b00070,
        }
        .vertices()
        .unwrap();
        assert_eq!(
            q.map(|v| v.position),
            [
                [-0.5, -0.25, 0.0],
                [0.5, -0.25, 0.0],
                [0.5, 0.75, 0.0],
                [-0.5, 0.75, 0.0]
            ]
        );
        assert_eq!(INDICES, [0, 1, 2, 2, 3, 0]);
        for vertex in q {
            assert_eq!(vertex.normal_packed, 0x7f00);
            assert_eq!(vertex.light, 0x00b00070);
            assert_eq!(vertex.color_argb, 0x807fff05);
            assert_eq!(vertex.shader_atlas_uv, vertex.uv);
        }
    }

    #[test]
    fn every_vanilla_icon_addresses_its_actual_sheet_cell() {
        for icon in 0..=10 {
            let q = ExperienceOrbAppearance {
                icon,
                red: 255,
                blue: 51,
                packed_light: 0,
            }
            .vertices()
            .unwrap();
            let x = (icon % 4) as f32 / 4.0;
            let y = (icon / 4) as f32 / 4.0;
            assert_eq!(
                q.map(|v| v.uv),
                [[x, y + 0.25], [x + 0.25, y + 0.25], [x + 0.25, y], [x, y]]
            );
        }
        for icon in [11, 16, u32::MAX] {
            assert!(ExperienceOrbAppearance {
                icon,
                red: 0,
                blue: 0,
                packed_light: 0
            }
            .vertices()
            .is_err());
        }
    }
}
