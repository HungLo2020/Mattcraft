//! Opt-in, bounded observation of real terrain VS outputs. No backend objects.
use super::*;

pub(super) const LIMIT: usize = 128;
const RECORD_WORDS: usize = 12;
pub(super) const BYTES: u64 = ((1 + LIMIT * RECORD_WORDS) * 4) as u64;
const DRAW_LIMIT: usize = 1024;
const DRAW_STRIDE: u64 = BYTES.div_ceil(256) * 256;
const POOL_BYTES: u64 = DRAW_STRIDE * DRAW_LIMIT as u64;

/// Every draw gets a disjoint explicit dynamic range. Shader atomics coordinate
/// vertices within that draw; no unsynchronized overlapping writes across draws.
pub(super) fn assign_draw_ranges(ops: &mut Vec<CommandOp>, pipelines: &BTreeMap<Handle, Handle>) -> GalResult<()> {
    if pipelines.is_empty() { return Ok(()); }
    let mut current_pipeline = None;
    let mut sets = BTreeMap::new();
    let mut counts = BTreeMap::<Handle, usize>::new();
    let mut result = Vec::with_capacity(ops.len());
    for mut op in ops.iter().cloned() {
        match &mut op {
            CommandOp::BindGraphicsPipeline(pipeline) => current_pipeline = Some(*pipeline),
            CommandOp::EndPass => { current_pipeline = None; sets.clear(); }
            CommandOp::BindResourceSet { pipeline_layout, set_index: 0, dynamic_offsets, .. }
                if pipelines.values().any(|layout| layout == pipeline_layout) => {
                    if dynamic_offsets.len() != 2 {
                        return Err(GalError::invalid_argument("observer expects two semantic mesh offsets"));
                    }
                    dynamic_offsets.push(0);
                    sets.insert(*pipeline_layout, op.clone());
                    // Emit this explicit binding only when its draw receives
                    // a range; do not bind a placeholder writable range first.
                    continue;
                }
            CommandOp::Draw { .. } | CommandOp::DrawIndexed { .. } => {
                if let Some(layout) = current_pipeline.and_then(|p| pipelines.get(&p)) {
                    let slot = counts.entry(*layout).or_default();
                    if *slot >= DRAW_LIMIT {
                        return Err(GalError::unsupported_feature("vertex observation exceeds 1024 bounded draws"));
                    }
                    let mut set = sets.get(layout).cloned()
                        .ok_or_else(|| GalError::invalid_argument("observed draw has no explicit mesh resource set"))?;
                    if let CommandOp::BindResourceSet { dynamic_offsets, .. } = &mut set {
                        dynamic_offsets[2] = *slot as u64 * DRAW_STRIDE;
                    }
                    *slot += 1;
                    result.push(set);
                }
            }
            _ => {}
        }
        result.push(op);
    }
    *ops = result;
    Ok(())
}

pub(super) fn selected(identity: &str) -> bool {
    std::env::var("MATTMC_RUST_TERRAIN_COORDINATE_PROBE").ok().as_deref()
        == Some("vertex-storage")
        && identity == "vulkanic:builtin/direct_terrain_opaque_v1"
}

pub(super) fn instrument(source: &str) -> GalResult<String> {
    let anchor = "vec4 clip = projection * view * world;";
    if source.matches(anchor).count() != 1 || !source.starts_with("#version 450\n") {
        return Err(GalError::invalid_argument("vertex observer requires exact builtin terrain source"));
    }
    let declarations = "#version 450\nlayout(set = 0, binding = 5, std430) buffer Storage5 { uint observed[]; };\n";
    // Camera-relative box of the isolated stone fixture. Record actual world and
    // clip outputs, not CPU reconstructions; bounds are diagnostic selection only.
    let observation = r#"
    uint vertex_observation_offset = 0u;
    if (all(greaterThanEqual(world.xyz, vec3(-3.51,-0.63,-1.51))) &&
        all(lessThanEqual(world.xyz, vec3(-2.49,0.39,-0.49)))) {
        uint slot = atomicAdd(observed[0], 1u);
        if (slot < 128u) {
            uint o = 1u + slot * 12u;
            vertex_observation_offset = o;
            observed[o] = uint(gl_VertexIndex);
            observed[o+1u] = uint(gl_InstanceIndex);
            for (uint c=0u;c<4u;c++) {
                observed[o+2u+c] = floatBitsToUint(world[c]);
                observed[o+6u+c] = floatBitsToUint(clip[c]);
            }
        }
    }
"#;
    let mut source = source.replacen("#version 450\n", declarations, 1)
        .replacen(anchor, &format!("{anchor}{observation}"), 1);
    let end = source.rfind("\n}").ok_or_else(|| GalError::invalid_argument("missing terrain main end"))?;
    source.insert_str(end, "\n    if (vertex_observation_offset != 0u) {\n        observed[vertex_observation_offset+10u] = floatBitsToUint(v_uv.x);\n        observed[vertex_observation_offset+11u] = floatBitsToUint(v_uv.y);\n    }\n");
    Ok(source)
}

pub(super) struct Observation {
    pub output: Handle,
    seed: Handle,
    readback: Handle,
    validation_layout: Handle,
    validation_set: Handle,
    submitted: bool,
}

impl Observation {
    // All allocations participate in the caller's transactional rollback.
    pub fn create(gal: &mut VulkanicGal, created: &mut Vec<Handle>) -> GalResult<Self> {
        let mut buffer = |label: &str, memory, usages| -> GalResult<Handle> {
            let h = gal.create_buffer(BufferDesc { label: label.into(), size: POOL_BYTES, memory, usages })?;
            created.push(h); Ok(h)
        };
        let seed = buffer("vertex-observation.seed", MemoryDomain::Upload,
            vec![BufferUsage::HostWrite, BufferUsage::TransferSrc])?;
        let output = buffer("vertex-observation.output", MemoryDomain::DeviceLocal,
            vec![BufferUsage::Storage, BufferUsage::TransferSrc, BufferUsage::TransferDst])?;
        let readback = buffer("vertex-observation.readback", MemoryDomain::Readback,
            vec![BufferUsage::TransferDst, BufferUsage::HostRead])?;
        // Validate writable DRAW storage support before any instrumented shader
        // or native pipeline is created. The actual mesh layout also declares it.
        let validation_layout = gal.create_resource_layout(ResourceLayoutDesc {
            label: "vertex-observation.validation-layout".into(), bindings: vec![declaration()],
        })?;
        created.push(validation_layout);
        let validation_set = gal.create_resource_set(ResourceSetDesc {
            label: "vertex-observation.validation-set".into(), layout: validation_layout,
            bindings: vec![binding(output)],
        })?;
        created.push(validation_set);
        Ok(Self { output, seed, readback, validation_layout, validation_set, submitted: false })
    }

    pub fn handles(&self) -> [Handle; 5] {
        [self.validation_set, self.validation_layout, self.readback, self.output, self.seed]
    }

    pub fn begin(&self, ops: &mut Vec<CommandOp>) {
        if !self.submitted {
            ops.push(CommandOp::HostWriteBuffer { buffer: self.seed, offset: 0, data: vec![0; POOL_BYTES as usize] });
            ops.push(CommandOp::Barrier(buffer_barrier(self.seed,
                TextureUsageState::TransferDst, TextureUsageState::TransferSrc)));
        }
        ops.push(CommandOp::Barrier(buffer_barrier(self.output,
            if self.submitted { TextureUsageState::TransferSrc } else { TextureUsageState::Undefined },
            TextureUsageState::TransferDst)));
        ops.push(CommandOp::CopyBuffer { src: self.seed, dst: self.output, size: POOL_BYTES });
        ops.push(CommandOp::Barrier(buffer_barrier(self.output,
            TextureUsageState::TransferDst, TextureUsageState::ShaderWrite)));
    }

    pub fn end(&self, ops: &mut Vec<CommandOp>) {
        ops.push(CommandOp::Barrier(buffer_barrier(self.output,
            TextureUsageState::ShaderWrite, TextureUsageState::TransferSrc)));
        ops.push(CommandOp::Barrier(buffer_barrier(self.readback,
            if self.submitted { TextureUsageState::ShaderRead } else { TextureUsageState::Undefined },
            TextureUsageState::TransferDst)));
        ops.push(CommandOp::CopyBuffer { src: self.output, dst: self.readback, size: POOL_BYTES });
        ops.push(CommandOp::Barrier(buffer_barrier(self.readback,
            TextureUsageState::TransferDst, TextureUsageState::ShaderRead)));
        ops.push(CommandOp::HostReadBuffer { buffer: self.readback, offset: 0, size: POOL_BYTES });
    }

    pub fn complete(&mut self, reads: &[CompletedHostRead], submission: u64, emit: bool) -> GalResult<()> {
        let read = reads.iter().find(|r| r.buffer == self.readback && r.submission.0 == submission
            && r.offset == 0 && r.bytes.len() == POOL_BYTES as usize)
            .ok_or_else(|| GalError::backend("missing completed terrain vertex observation"))?;
        self.submitted = true;
        if emit {
            let mut count = 0usize;
            let mut truncated = false;
            let mut records = Vec::new();
            for draw in read.bytes.chunks_exact(DRAW_STRIDE as usize) {
                let n = u32::from_le_bytes(draw[..4].try_into().unwrap()) as usize;
                count += n;
                truncated |= n > LIMIT;
                for row in draw[4..BYTES as usize].chunks_exact(RECORD_WORDS * 4).take(n.min(LIMIT)) {
                    records.push(row.chunks_exact(4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).collect::<Vec<_>>());
                }
            }
            eprintln!("terrain-vertex-observation submission={submission} count={count} truncated={truncated} records={records:?}");
        }
        Ok(())
    }
}

pub(super) fn declaration() -> ResourceBindingDesc {
    ResourceBindingDesc { binding: 5, kind: ResourceBindingKind::StorageBuffer,
        stages: PipelineStageFlags::DRAW, array_count: 1, optional: false, dynamic_offset_count: 1 }
}

pub(super) fn binding(output: Handle) -> ResourceBinding {
    ResourceBinding { binding: 5, array_index: 0, resource: output, kind: ResourceBindingKind::StorageBuffer,
        access: AccessFlags(AccessFlags::READ.0 | AccessFlags::WRITE.0), dynamic_offsets: vec![0], buffer_range: Some(BYTES) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vertex_observation_assigns_disjoint_ranges_without_changing_draws() {
        let pipeline = Handle::from_raw(1);
        let layout = Handle::from_raw(2);
        let set = Handle::from_raw(3);
        let original = vec![CommandOp::BindGraphicsPipeline(pipeline),
            CommandOp::BindResourceSet { pipeline_layout: layout, set_index: 0, set,
                dynamic_offsets: vec![256, 512] },
            CommandOp::DrawIndexed { indices: 6, instances: 1 },
            CommandOp::DrawIndexed { indices: 12, instances: 1 }];
        let mut untouched = original.clone();
        assign_draw_ranges(&mut untouched, &BTreeMap::new()).unwrap();
        assert_eq!(untouched, original);
        let mut observed = original.clone();
        assign_draw_ranges(&mut observed, &BTreeMap::from([(pipeline, layout)])).unwrap();
        let offsets: Vec<_> = observed.iter().filter_map(|op| match op {
            CommandOp::BindResourceSet { dynamic_offsets, .. } => Some(dynamic_offsets.clone()),
            _ => None,
        }).collect();
        assert_eq!(offsets, vec![vec![256,512,0],vec![256,512,DRAW_STRIDE]]);
        assert!(DRAW_STRIDE >= BYTES && DRAW_STRIDE % 256 == 0);
        let draws = |ops: Vec<CommandOp>| ops.into_iter().filter(|op| matches!(op, CommandOp::DrawIndexed { .. })).collect::<Vec<_>>();
        assert_eq!(draws(observed), draws(original));
        let mut missing = vec![CommandOp::BindGraphicsPipeline(pipeline), CommandOp::Draw { vertices: 3, instances: 1 }];
        assert!(assign_draw_ranges(&mut missing, &BTreeMap::from([(pipeline,layout)])).is_err());
    }
    #[test]
    fn vertex_observation_keeps_original_position_expression_and_bounds_every_write() {
        let normal = minimal_direct_terrain_solid_program().vertex.source;
        let observed = instrument(&normal).unwrap();
        assert_eq!(observed.matches("vec4 clip = projection * view * world;").count(), 1);
        assert!(observed.contains("slot < 128u"));
        assert!(observed.contains("gl_Position = clip;"));
        assert_eq!(BYTES, 6148);
        assert!(instrument("void main() {}").is_err());
        assert!(observed.find("floatBitsToUint(clip[c])").unwrap()
            < observed.find("clip.z = clip.z * 0.5").unwrap());
    }
}
