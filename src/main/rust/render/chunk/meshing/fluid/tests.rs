use super::*;

#[derive(Default)]
struct RecordedFluidFaces {
    profile: NativeMeshingProfile,
    faces: Vec<NativeFluidFace>,
    sprite_mask: i32,
}

impl NativeFluidFaceSink for RecordedFluidFaces {
    fn profile(&mut self) -> &mut NativeMeshingProfile { &mut self.profile }
    fn mark_fluid_sprite(&mut self, mask: i32) { self.sprite_mask |= mask; }
    fn emit(&mut self, face: NativeFluidFace) -> Result<(), i32> {
        self.faces.push(face);
        Ok(())
    }
}

#[test]
fn emitted_water_and_lava_faces_report_only_their_used_sprites() {
    for (fluid_type, expected) in [
        (FLUID_WATER, FLUID_SPRITE_WATER_STILL | FLUID_SPRITE_WATER_FLOW),
        (FLUID_LAVA, FLUID_SPRITE_LAVA_STILL | FLUID_SPRITE_LAVA_FLOW),
    ] {
        let fluid = NativeMeshingState {
            fluid_type, fluid_own_height: 8.0 / 9.0,
            ..NativeMeshingState::default()
        };
        let states = [Some(NativeMeshingState::default()), Some(fluid)];
        for enclosed in [false, true] {
            let neighbor = if enclosed { 1 } else { 0 };
            let block = NativeSectionBlockRecord {
                neighbor_state_ids: [neighbor; 6], neighborhood_state_ids: [neighbor; 27],
                ..NativeSectionBlockRecord::default()
            };
            let mut sink = RecordedFluidFaces::default();
            let count = native_section_fluid_faces_to_sink(&block, fluid, &states, &mut sink).unwrap();
            assert_eq!(count, sink.faces.len());
            assert_eq!(enclosed, sink.faces.is_empty());
            assert_eq!(if enclosed { 0 } else { expected }, sink.sprite_mask,
                "fluid={fluid_type}, enclosed={enclosed}");
        }
    }
}

#[test]
fn fluid_ceiling_exposure_matches_shape_boundary_tolerance() {
    let states = [Some(NativeMeshingState {
        flags: STATE_FLAG_CAN_OCCLUDE | STATE_FLAG_FULL_OCCLUSION,
        ..NativeMeshingState::default()
    })];
    let block = NativeSectionBlockRecord {
        neighborhood_state_ids: [0; 27],
        ..NativeSectionBlockRecord::default()
    };
    for (height, exposed) in [(0.5, true), (8.0 / 9.0, true),
            (f32::from_bits(1.0f32.to_bits() - 2), true),
            (f32::from_bits(1.0f32.to_bits() - 1), false), (1.0, false)] {
        assert_eq!(exposed, fluid_side_exposed(&block, &states, 0, 1, 0, height));
        for direction in [0, 2, 3, 4, 5] {
            let (dx, dy, dz) = dir_step(direction);
            assert!(!fluid_side_exposed(&block, &states, dx, dy, dz, height));
        }
    }
}

#[test]
fn partial_height_fluid_keeps_top_and_backface_below_solid_ceiling() {
    for fluid_type in [FLUID_WATER, FLUID_LAVA] {
        for height in [8.0 / 9.0, 1.0] {
            let fluid = NativeMeshingState {
                fluid_type, fluid_own_height: height,
                ..NativeMeshingState::default()
            };
            let states = [Some(NativeMeshingState::default()), Some(fluid),
                Some(NativeMeshingState {
                    flags: STATE_FLAG_CAN_OCCLUDE | STATE_FLAG_FULL_OCCLUSION | STATE_FLAG_SOLID_RENDER,
                    ..NativeMeshingState::default()
                })];
            let mut block = NativeSectionBlockRecord {
                neighbor_state_ids: [1; 6], neighborhood_state_ids: [1; 27],
                ..NativeSectionBlockRecord::default()
            };
            // Keep the neighboring surface at this same partial height: fluid
            // above a horizontal neighbor would correctly raise its corners
            // to a full block, eliminating the gap being tested.
            for dz in -1..=1 {
                for dx in -1..=1 {
                    block.neighborhood_state_ids[neighborhood_index(dx, 1, dz)] = 0;
                }
            }
            block.neighbor_state_ids[1] = 2;
            block.neighborhood_state_ids[neighborhood_index(0, 1, 0)] = 2;
            // Open diagonal above the surface requires the ordinary top backface.
            block.neighborhood_state_ids[neighborhood_index(1, 1, 1)] = 0;
            let mut sink = RecordedFluidFaces::default();
            native_section_fluid_faces_to_sink(&block, fluid, &states, &mut sink).unwrap();
            assert_eq!(if height < 1.0 { 2 } else { 0 }, sink.faces.len(),
                "fluid={fluid_type}, height={height}");
        }
    }
}

#[test]
fn fluid_bottom_exposure_is_independent_of_horizontal_solid_neighbors() {
    let water = NativeMeshingState {
        fluid_type: FLUID_WATER,
        fluid_own_height: 8.0 / 9.0,
        ..NativeMeshingState::default()
    };
    let states = [Some(NativeMeshingState::default()), Some(water),
        Some(NativeMeshingState {
            flags: STATE_FLAG_CAN_OCCLUDE | STATE_FLAG_FULL_OCCLUSION | STATE_FLAG_BLOCKS_MOTION,
            ..NativeMeshingState::default()
        })];
    for direction in 2..6 {
        for solid_below in [false, true] {
            let mut block = NativeSectionBlockRecord {
                neighbor_state_ids: [0; 6],
                neighborhood_state_ids: [0; 27],
                ..NativeSectionBlockRecord::default()
            };
            block.neighborhood_state_ids[13] = 1;
            let (dx, dy, dz) = dir_step(direction);
            block.neighbor_state_ids[direction as usize] = 2;
            block.neighborhood_state_ids[neighborhood_index(dx, dy, dz)] = 2;
            if solid_below {
                block.neighbor_state_ids[0] = 2;
                block.neighborhood_state_ids[neighborhood_index(0, -1, 0)] = 2;
            }
            let mut sink = RecordedFluidFaces::default();
            native_section_fluid_faces_to_sink(&block, water, &states, &mut sink).unwrap();
            assert_eq!(if solid_below { 0 } else { 1 },
                sink.faces.iter().filter(|face| face.face_kind == FLUID_FACE_BOTTOM).count(),
                "horizontal direction={direction}, solid below={solid_below}");
        }
    }
}

#[test]
fn declared_overlay_controls_emitted_uvs_backfaces_and_sprite_usage() {
    // Exercise the real face producer, not just its selection predicate.
    // Matching water hides five neighbors; the sixth represents either a
    // non-occluding door or an explicitly transparent enclosure. Java's fluid
    // renderer emits a backface only for the non-overlay side.
    let water = NativeMeshingState {
        fluid_type: FLUID_WATER,
        fluid_own_height: 8.0 / 9.0,
        fluid_overlay_valid: 1,
        fluid_flow: FluidSprite { u0: 0.1, u1: 0.3, v0: 0.2, v1: 0.4, shrink: 0.0 },
        fluid_overlay: FluidSprite { u0: 0.6, u1: 0.8, v0: 0.7, v1: 0.9, shrink: 0.0 },
        ..NativeMeshingState::default()
    };
    for direction in 2..6 {
        for transparent in [false, true] {
            let mut block = NativeSectionBlockRecord {
                neighbor_state_ids: [0; 6],
                neighborhood_state_ids: [0; 27],
                ..NativeSectionBlockRecord::default()
            };
            let (dx, dy, dz) = dir_step(direction);
            block.neighbor_state_ids[direction as usize] = 1;
            block.neighborhood_state_ids[neighborhood_index(dx, dy, dz)] = 1;
            let states = [Some(water), Some(NativeMeshingState {
                flags: if transparent { STATE_FLAG_FLUID_OVERLAY_TRANSPARENT } else { 0 },
                ..NativeMeshingState::default()
            })];
            let mut sink = RecordedFluidFaces::default();
            let emitted = native_section_fluid_faces_to_sink(&block, water, &states, &mut sink).unwrap();
            assert_eq!(if transparent { 1 } else { 2 }, emitted);
            assert_eq!(emitted, sink.faces.len());
            assert_eq!(if transparent { FLUID_SPRITE_WATER_OVERLAY } else { FLUID_SPRITE_WATER_FLOW }, sink.sprite_mask);
            let sprite = if transparent { water.fluid_overlay } else { water.fluid_flow };
            for face in &sink.faces {
                assert_eq!(FLUID_FACE_SIDE, face.face_kind);
                assert_eq!(FLUID_WATER, face.fluid_type);
                for vertex in &face.vertices {
                    assert!(vertex.u >= sprite.u0 && vertex.u <= sprite.u1);
                    assert!(vertex.v >= sprite.v0 && vertex.v <= sprite.v1);
                }
            }
            if !transparent {
                let front = &sink.faces[0];
                let back = &sink.faces[1];
                // Frozen writeQuad receives the opposite bucket and flip=true;
                // its collector flips that bucket's aligned normal. Do not
                // substitute a normal derived from the front winding here.
                assert_ne!(front.facing, back.facing);
                assert_eq!(flip_packed_normal(packed_fluid_normal(back.facing, &front.vertices)), back.packed_normal);
                for (i, j) in [(0, 0), (1, 3), (2, 2), (3, 1)] {
                    let a = front.vertices[i];
                    let b = back.vertices[j];
                    assert_eq!((a.x, a.y, a.z, a.u, a.v), (b.x, b.y, b.z, b.u, b.v));
                }
            }
        }
    }
}

#[test]
fn overlay_selection_consumes_declared_transparency_not_occlusion() {
    let block = NativeSectionBlockRecord {
        neighborhood_state_ids: [0; 27],
        ..NativeSectionBlockRecord::default()
    };
    let mut water = NativeMeshingState {
        fluid_type: FLUID_WATER,
        fluid_overlay_valid: 1,
        ..NativeMeshingState::default()
    };
    // All older state bits are independent of the explicit property. This
    // includes non-occluding non-air plants (false) and registry overrides
    // declaring an otherwise occluding block transparent (true).
    for flags in 0..STATE_FLAG_FLUID_OVERLAY_TRANSPARENT {
        for declared in [false, true] {
            let states = [Some(NativeMeshingState {
                flags: flags | if declared { STATE_FLAG_FLUID_OVERLAY_TRANSPARENT } else { 0 },
                ..NativeMeshingState::default()
            })];
            for direction in 2..6 {
                assert_eq!(declared, fluid_side_uses_overlay(&block, water, &states, direction));
            }
        }
    }
    let states = [Some(NativeMeshingState {
        flags: STATE_FLAG_FLUID_OVERLAY_TRANSPARENT,
        ..NativeMeshingState::default()
    })];
    water.fluid_overlay_valid = 0;
    assert!(!fluid_side_uses_overlay(&block, water, &states, 2));
    water.fluid_overlay_valid = 1;
    water.fluid_type = FLUID_LAVA;
    assert!(!fluid_side_uses_overlay(&block, water, &states, 2));
    water.fluid_type = FLUID_WATER;
    assert!(!fluid_side_uses_overlay(&block, water, &[None], 2));
}

#[test]
fn native_fluid_admission_is_explicitly_bounded_to_water_and_lava() {
    assert!(is_supported_native_fluid_type(FLUID_WATER));
    assert!(is_supported_native_fluid_type(FLUID_LAVA));
    assert!(!is_supported_native_fluid_type(0));
    assert!(!is_supported_native_fluid_type(99));
}

#[test]
fn fluid_face_record_expands_semantic_side_face_to_quad() {
    let mut record = FluidFaceRecord {
        packed_normal: 0,
        material_bits: 5,
        block_emission: 7,
        render_type: 1,
        ignore_mid_block: 0,
        block_id: 41,
        local_x: 4,
        local_y: 5,
        local_z: 6,
        face_kind: 3,
        flip: 0,
        origin_x: 10.0,
        origin_y: 20.0,
        origin_z: 30.0,
        y_offset: 0.001,
        heights: [0.75, 0.5, 0.0, 0.0],
        side_coords: [0.0, 1.0, 1.0, 1.0],
        uvs: [0.0, 0.2, 0.5, 0.6, 1.0, 0.6, 1.0, 0.1],
        colors: [1, 2, 3, 4],
        aos: [0.1, 0.2, 0.3, 0.4],
        lights: [11, 12, 13, 14],
        primitive_kind: TERRAIN_PRIMITIVE_BUILTIN_WATER,
    };

    let quad = fluid_face_record_to_quad(record).unwrap();
    assert_eq!(11.0, quad.vertices[0].x);
    assert_eq!(20.5, quad.vertices[0].y);
    assert_eq!(31.0, quad.vertices[0].z);
    assert_eq!(10.0, quad.vertices[3].x);
    assert_eq!(20.75, quad.vertices[3].y);
    assert_eq!(31.0, quad.vertices[3].z);
    assert_eq!(5, quad.material_bits);
    assert_eq!(41, quad.block_id);

    record.flip = 1;
    let flipped = fluid_face_record_to_quad(record).unwrap();
    assert_eq!(quad.vertices[0].x, flipped.vertices[0].x);
    assert_eq!(quad.vertices[3].x, flipped.vertices[1].x);
    assert_eq!(quad.vertices[2].x, flipped.vertices[2].x);
    assert_eq!(quad.vertices[1].x, flipped.vertices[3].x);
}

#[test]
fn flipped_fluid_faces_flip_packed_normals_like_java_writer() {
    assert_eq!(0x00007f00, flip_packed_normal(0x00008100));
    assert_eq!(0x00008100, flip_packed_normal(0x00007f00));
    assert_eq!(0x0081007f, flip_packed_normal(0x007f0081));
}

#[test]
fn native_fluid_uses_fluid_shader_block_id_not_container_block_id() {
    let mut block = NativeSectionBlockRecord {
        block_id: 1234,
        fluid_block_id: 5678,
        fluid_tint: 0xff3f_76e4u32 as i32,
        ..NativeSectionBlockRecord::default()
    };
    block.absolute_x = block.local_x;
    block.absolute_y = block.local_y;
    block.absolute_z = block.local_z;

    let state = NativeMeshingState {
        fluid_type: FLUID_WATER,
        fluid_block_id: 9012,
        fluid_material_bits: 9,
        fluid_still: FluidSprite {
            u0: 0.0,
            u1: 1.0,
            v0: 0.0,
            v1: 1.0,
            shrink: 0.0,
        },
        ..NativeMeshingState::default()
    };

    let (record, _) = fluid_semantic_face(
        state,
        &block,
        MODEL_QUAD_FACING_POS_Y,
        false,
        FLUID_FACE_TOP_NW_SE,
        0.0,
        [1.0; 4],
        [0.0; 4],
        [(0.0, 0.0), (0.0, 1.0), (1.0, 1.0), (1.0, 0.0)],
        argb_to_abgr(block.fluid_tint),
        1.0,
        LIGHT_FULL_BRIGHT,
    );
    let quad = fluid_face_record_to_quad(record).unwrap();

    assert_eq!(5678, quad.block_id);
    assert_eq!(1, quad.render_type);
}

#[test]
fn flowing_fluid_top_trig_initializes_without_native_stack_table() {
    let (dir, sin, cos) = flowing_top_trig_for_test(0.70710677, 0.70710677);

    assert!(dir.is_finite());
    assert!(sin.is_finite());
    assert!(cos.is_finite());
    assert!((dir + std::f32::consts::FRAC_PI_4).abs() < 0.001);
}
