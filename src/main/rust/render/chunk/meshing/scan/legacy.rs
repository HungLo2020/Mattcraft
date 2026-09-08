//! Legacy native-section record scanner.
//!
//! This module preserves the historical 316-byte `NativeSectionBlockRecord`
//! path for tests, benchmarks, and replay compatibility. Production compact
//! scanning must not depend on this module. The pass-cache and optional raw-quad
//! storage behavior remain here because they belong to the legacy bridge.

use std::slice;
use std::time::Instant;

use super::*;

// The replay ABI must not inherit fields added to the internal compact
// scanner record. Java still writes the historical 316-byte layout.
#[repr(C)]
#[derive(Clone, Copy)]
struct LegacySectionBlockRecord {
    state_id: i32,
    block_id: i32,
    local_x: i32,
    local_y: i32,
    local_z: i32,
    seed_lo: i32,
    seed_hi: i32,
    neighbor_state_ids: [i32; 6],
    light_words: [i32; 27],
    neighborhood_state_ids: [i32; 27],
    tint: i32,
    fluid_tint: i32,
    fluid_flow_x: f32,
    fluid_flow_z: f32,
    absolute_x: i32,
    absolute_y: i32,
    absolute_z: i32,
    legacy_offset_x: f32,
    legacy_offset_y: f32,
    legacy_offset_z: f32,
    fluid_block_id: i32,
    flags: i32,
}

impl LegacySectionBlockRecord {
    fn decoded(self) -> NativeSectionBlockRecord {
        NativeSectionBlockRecord {
            state_id: self.state_id, block_id: self.block_id,
            local_x: self.local_x, local_y: self.local_y, local_z: self.local_z,
            seed_lo: self.seed_lo, seed_hi: self.seed_hi,
            neighbor_state_ids: self.neighbor_state_ids,
            light_words: self.light_words,
            neighborhood_state_ids: self.neighborhood_state_ids,
            // Legacy records carry one tint, not spatial provider samples.
            tint_lattice: [[[self.tint; 4]; 4]; 4],
            tint: self.tint, fluid_tint: self.fluid_tint,
            fluid_flow_x: self.fluid_flow_x, fluid_flow_z: self.fluid_flow_z,
            absolute_x: self.absolute_x, absolute_y: self.absolute_y, absolute_z: self.absolute_z,
            legacy_offset_x: self.legacy_offset_x, legacy_offset_y: self.legacy_offset_y,
            legacy_offset_z: self.legacy_offset_z,
            fluid_block_id: self.fluid_block_id, flags: self.flags,
        }
    }
}

#[test]
fn legacy_record_keeps_its_316_byte_wire_layout() {
    assert_eq!(316, std::mem::size_of::<LegacySectionBlockRecord>());
    assert_eq!(268, std::mem::offset_of!(LegacySectionBlockRecord, tint));
    assert_eq!(312, std::mem::offset_of!(LegacySectionBlockRecord, flags));
}

pub(in crate::render::chunk::meshing) unsafe fn section_builder_append_native_section_records_encoded(
    builder: &mut NativeSectionMeshBuilder,
    record_address: u64,
    record_count: usize,
    record_stride: usize,
    pass_id: i32,
    analyzer: Option<u64>,
    format: NativeFormat,
    store_raw_quads: bool,
) -> Result<i32, i32> {
    if record_count == 0 {
        return Ok(0);
    }
    if record_address == 0 {
        return Err(ERR_NULL_POINTER);
    }
    if record_stride != std::mem::size_of::<LegacySectionBlockRecord>() {
        return Err(ERR_INVALID_ARGUMENT);
    }
    if pass_id >= 0
        && pass_id < 32
        && builder.section_pass_cache_valid
        && builder.section_pass_cache_address == record_address
        && builder.section_pass_cache_count == record_count
        && (builder.section_pass_cache_mask & (1u32 << pass_id)) == 0
    {
        return Ok(0);
    }

    let records = slice::from_raw_parts(
        record_address as *const LegacySectionBlockRecord,
        record_count,
    );
    let emit_all_passes = pass_id < 0;
    builder
        .profile
        .add_count(PROFILE_COUNT_SCANNED_BLOCKS, record_count);
    let profile_scan_substages = scan_substage_profile_enabled();
    let metadata_started = Instant::now();
    let cache_lookup_started = profile_start(profile_scan_substages);
    let states_guard = native_meshing_states()
        .lock()
        .map_err(|_| ERR_INVALID_ARGUMENT)?;
    let selectors_guard = native_model_selectors()
        .lock()
        .map_err(|_| ERR_INVALID_ARGUMENT)?;
    let models_guard = static_model_cache()
        .lock()
        .map_err(|_| ERR_INVALID_ARGUMENT)?;
    builder
        .profile
        .add_stage(PROFILE_MATERIAL_PASS, metadata_started);
    builder
        .profile
        .add_optional_stage(PROFILE_SCAN_CACHE_LOOKUP, cache_lookup_started);
    let mut total_committed = 0i32;
    let mut pending_counts = [0usize; MODEL_QUAD_FACING_COUNT];
    let mut model_ids = Vec::with_capacity(8);
    let mut last_state_id = i32::MIN;
    let mut last_state = None;
    let mut last_direct_selector_id = i32::MIN;
    let mut last_direct_selector_model_id = None;
    let mut last_model_id = i32::MIN;
    let mut last_model = None;
    let mut discovered_pass_mask = 0u32;
    let profile_static_substages = static_model_substage_profile_enabled();
    let profile_staging_substages = staging_substage_profile_enabled();

    let scan_started = Instant::now();
    for record in records {
        let decoded = record.decoded();
        let record = &decoded;
        let iteration_started = profile_start(profile_scan_substages);
        let decoding_started = profile_start(profile_scan_substages);
        let state_lookup_started = profile_start(profile_static_substages);
        let state = if record.state_id == last_state_id {
            last_state
        } else {
            last_state_id = record.state_id;
            last_state = state_by_id(&states_guard, record.state_id);
            last_state
        };
        builder
            .profile
            .add_optional_stage(PROFILE_STATIC_STATE_SELECTOR_LOOKUP, state_lookup_started);
        builder
            .profile
            .add_optional_stage(PROFILE_SCAN_RECORD_DECODING, decoding_started);
        let Some(state) = state else {
            builder
                .profile
                .add_optional_stage(PROFILE_SCAN_ACTIVE_RECORD_ITERATION, iteration_started);
            continue;
        };
        let flags = state.flags;
        if (flags & STATE_FLAG_AIR) != 0 {
            builder
                .profile
                .add_optional_stage(PROFILE_SCAN_ACTIVE_RECORD_ITERATION, iteration_started);
            continue;
        }
        let dispatch_started = profile_start(profile_scan_substages);
        let dispatch = scan_dispatch(flags, record.flags);
        let has_light_block = dispatch.has_light_block;
        let has_model = dispatch.has_model;
        let has_fluid = dispatch.has_fluid;
        if has_light_block {
            discovered_pass_mask |= 1u32 << 1;
        }
        if has_model && state.pass_id >= 0 && state.pass_id < 32 {
            discovered_pass_mask |= 1u32 << state.pass_id;
        } else if has_model && state.pass_id < 0 {
            discovered_pass_mask |= 0b111;
        }
        if has_fluid && state.fluid_pass_id >= 0 && state.fluid_pass_id < 32 {
            discovered_pass_mask |= 1u32 << state.fluid_pass_id;
        }
        builder
            .profile
            .add_optional_stage(PROFILE_SCAN_DISPATCH, dispatch_started);

        if has_light_block && (emit_all_passes || pass_id == 1) {
            let model_started = Instant::now();
            let append_started = profile_start(profile_scan_substages);
            push_native_section_quad(
                builder,
                light_block_record_to_quad(LightBlockRecord {
                    material_bits: state.material_bits,
                    block_emission: state.block_emission,
                    block_id: choose_block_id(record.block_id, state.block_id),
                    local_x: record.local_x,
                    local_y: record.local_y,
                    local_z: record.local_z,
                }),
                0,
                TERRAIN_PRIMITIVE_UNKNOWN,
                MODEL_QUAD_FACING_UNASSIGNED,
                &mut pending_counts,
                analyzer,
                format,
                store_raw_quads,
                profile_staging_substages,
                &mut total_committed,
            )?;
            builder
                .profile
                .add_optional_stage(PROFILE_SCAN_QUAD_APPEND, append_started);
            builder
                .profile
                .add_stage(PROFILE_MODEL_LOOKUP_EMIT, model_started);
            builder
                .profile
                .add_count(PROFILE_COUNT_NATIVE_MODEL_QUADS, 1);
        }

        if has_model && (emit_all_passes || state.pass_id < 0 || state.pass_id == pass_id) {
            let model_started = Instant::now();
            let scan_model_started = profile_start(profile_scan_substages);
            builder
                .profile
                .add_count(PROFILE_COUNT_NATIVE_MODEL_BLOCKS, 1);
            let selector_started = profile_start(profile_static_substages);
            let mut selector_lookup_recorded = false;
            let direct_model_storage;
            let model_id_slice: &[i32];
            if state.selector_id == last_direct_selector_id {
                builder
                    .profile
                    .add_count(PROFILE_COUNT_SELECTOR_CACHE_HITS, 1);
                if let Some(model_id) = last_direct_selector_model_id {
                    direct_model_storage = [model_id];
                    model_id_slice = &direct_model_storage;
                } else {
                    model_id_slice = &[];
                }
            } else {
                let cache_lookup_started = profile_start(profile_scan_substages);
                let direct_model_id =
                    selector_by_id(&selectors_guard, state.selector_id).and_then(|selector| {
                        if selector.kind == SELECTOR_DIRECT {
                            selector.entries.first().map(|entry| entry.target_id)
                        } else {
                            None
                        }
                    });
                builder
                    .profile
                    .add_optional_stage(PROFILE_SCAN_CACHE_LOOKUP, cache_lookup_started);
                if let Some(model_id) = direct_model_id {
                    last_direct_selector_id = state.selector_id;
                    last_direct_selector_model_id = Some(model_id);
                    builder
                        .profile
                        .add_count(PROFILE_COUNT_SELECTOR_CACHE_HITS, 1);
                    direct_model_storage = [model_id];
                    model_id_slice = &direct_model_storage;
                } else {
                    last_direct_selector_id = i32::MIN;
                    last_direct_selector_model_id = None;
                    builder
                        .profile
                        .add_count(PROFILE_COUNT_SELECTOR_CACHE_MISSES, 1);
                    builder
                        .profile
                        .add_optional_stage(PROFILE_STATIC_STATE_SELECTOR_LOOKUP, selector_started);
                    selector_lookup_recorded = true;
                    model_ids.clear();
                    builder
                        .profile
                        .add_count(PROFILE_COUNT_TEMP_VECTOR_CLEARS, 1);
                    let resolution_started = profile_start(profile_static_substages);
                    resolve_selector_model_ids(
                        state.selector_id,
                        record_seed(*record),
                        &selectors_guard,
                        &mut model_ids,
                        &mut builder.profile,
                    )?;
                    builder.profile.add_optional_stage(
                        PROFILE_STATIC_WEIGHTED_MULTIPART_RESOLUTION,
                        resolution_started,
                    );
                    model_id_slice = &model_ids;
                }
            }
            if !selector_lookup_recorded {
                builder
                    .profile
                    .add_optional_stage(PROFILE_STATIC_STATE_SELECTOR_LOOKUP, selector_started);
            }

            for model_id in model_id_slice {
                let model_lookup_started = profile_start(profile_static_substages);
                let scan_cache_lookup_started = profile_start(profile_scan_substages);
                let model = if *model_id == last_model_id {
                    builder.profile.add_count(PROFILE_COUNT_MODEL_CACHE_HITS, 1);
                    last_model
                } else {
                    last_model_id = *model_id;
                    last_model = model_by_id(&models_guard, *model_id);
                    builder
                        .profile
                        .add_count(PROFILE_COUNT_MODEL_CACHE_MISSES, 1);
                    last_model
                };
                builder
                    .profile
                    .add_optional_stage(PROFILE_STATIC_CACHED_MODEL_LOOKUP, model_lookup_started);
                builder
                    .profile
                    .add_optional_stage(PROFILE_SCAN_CACHE_LOOKUP, scan_cache_lookup_started);
                let Some(model) = model else {
                    continue;
                };

                for quad_record in model {
                    let quad_iteration_started = profile_start(profile_static_substages);
                    let quad_record = *quad_record;
                    builder
                        .profile
                        .add_optional_stage(PROFILE_STATIC_QUAD_ITERATION, quad_iteration_started);
                    if !emit_all_passes
                        && quad_record.pass_id >= 0
                        && quad_record.pass_id != pass_id
                    {
                        continue;
                    }
                    let culling_started = profile_start(profile_static_substages);
                    let scan_culling_started = profile_start(profile_scan_substages);
                    if native_section_culls_quad(record, state, quad_record, &states_guard) {
                        builder
                            .profile
                            .add_optional_stage(PROFILE_STATIC_CULLING, culling_started);
                        builder
                            .profile
                            .add_optional_stage(PROFILE_SCAN_CULLING, scan_culling_started);
                        continue;
                    }
                    builder
                        .profile
                        .add_optional_stage(PROFILE_STATIC_CULLING, culling_started);
                    builder
                        .profile
                        .add_optional_stage(PROFILE_SCAN_CULLING, scan_culling_started);

                    let quad_iteration_started = profile_start(profile_static_substages);
                    let facing = match usize::try_from(quad_record.normal_face) {
                        Ok(value) if value < MODEL_QUAD_FACING_COUNT => value,
                        _ => MODEL_QUAD_FACING_UNASSIGNED,
                    };
                    builder
                        .profile
                        .add_optional_stage(PROFILE_STATIC_QUAD_ITERATION, quad_iteration_started);
                    let quad = static_model_quad_to_native_section(
                        *record,
                        state,
                        quad_record,
                        format.separate_ao,
                        &mut builder.profile,
                        profile_static_substages,
                        profile_scan_substages,
                    );
                    let staging_started = profile_start(profile_static_substages);
                    let append_started = profile_start(profile_scan_substages);
                    push_native_section_quad(
                        builder,
                        quad,
                        quad_record.packed_normal,
                        TERRAIN_PRIMITIVE_UNKNOWN,
                        facing,
                        &mut pending_counts,
                        analyzer,
                        format,
                        store_raw_quads,
                        profile_staging_substages,
                        &mut total_committed,
                    )?;
                    builder
                        .profile
                        .add_optional_stage(PROFILE_SCAN_QUAD_APPEND, append_started);
                    builder
                        .profile
                        .add_optional_stage(PROFILE_STATIC_STAGING, staging_started);
                    builder
                        .profile
                        .add_count(PROFILE_COUNT_NATIVE_MODEL_QUADS, 1);
                }
            }
            builder
                .profile
                .add_stage(PROFILE_MODEL_LOOKUP_EMIT, model_started);
            builder
                .profile
                .add_optional_stage(PROFILE_SCAN_MODEL_EMISSION, scan_model_started);
        }

        if has_fluid && (emit_all_passes || state.fluid_pass_id == pass_id) {
            let scan_fluid_started = profile_start(profile_scan_substages);
            if native_fluid_diag_enabled() {
                eprintln!(
                    "MATTMC_NATIVE_FLUID_DIAG scan-fluid pass={} pos={},{},{} local={},{},{} state={} fluid_pass={} analyzer={} store_raw={}",
                    pass_id,
                    record.absolute_x,
                    record.absolute_y,
                    record.absolute_z,
                    record.local_x,
                    record.local_y,
                    record.local_z,
                    record.state_id,
                    state.fluid_pass_id,
                    analyzer.is_some(),
                    store_raw_quads
                );
            }
            builder.profile.add_count(PROFILE_COUNT_FLUID_BLOCKS, 1);
            let fluid_face_count = emit_native_section_fluid_faces(
                record,
                state,
                &states_guard,
                builder,
                &mut pending_counts,
                analyzer,
                format,
                store_raw_quads,
                profile_scan_substages,
                profile_staging_substages,
                &mut total_committed,
            )?;
            builder
                .profile
                .add_count(PROFILE_COUNT_FLUID_FACES, fluid_face_count);
            builder
                .profile
                .add_optional_stage(PROFILE_SCAN_FLUID_EMISSION, scan_fluid_started);
        }
        builder
            .profile
            .add_optional_stage(PROFILE_SCAN_ACTIVE_RECORD_ITERATION, iteration_started);
    }
    builder
        .profile
        .add_stage(PROFILE_SECTION_SCAN, scan_started);
    if pass_id >= 0 && pass_id < 32 {
        builder.section_pass_cache_address = record_address;
        builder.section_pass_cache_count = record_count;
        builder.section_pass_cache_mask = discovered_pass_mask;
        builder.section_pass_cache_valid = true;
    }

    for facing in 0..MODEL_QUAD_FACING_COUNT {
        flush_static_model_pending_face(
            builder,
            facing,
            &mut pending_counts,
            analyzer,
            format,
            store_raw_quads,
            &mut total_committed,
        )?;
    }

    Ok(total_committed)
}
