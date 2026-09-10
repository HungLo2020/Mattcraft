use super::*;

#[derive(Clone, Debug, PartialEq)]
pub struct FfiOwnedCreate<T> {
    pub request_id: u64,
    pub desc: T,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FfiOwnedBufferUpdate {
    pub buffer: Handle,
    pub offset: u64,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FfiOwnedTextureUpdate {
    pub texture: Handle,
    pub mip_level: u32,
    pub array_layer: u32,
    pub origin: TextureOrigin3d,
    pub extent: Extent3d,
    pub bytes_per_row: u32,
    pub rows_per_image: u32,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FfiOwnedResourceBatch {
    pub buffers: Vec<FfiOwnedCreate<BufferDesc>>,
    pub textures: Vec<FfiOwnedCreate<TextureDesc>>,
    pub texture_views: Vec<FfiOwnedCreate<TextureViewDesc>>,
    pub samplers: Vec<FfiOwnedCreate<SamplerDesc>>,
    pub shaders: Vec<FfiOwnedCreate<ShaderModuleDesc>>,
    pub resource_layouts: Vec<FfiOwnedCreate<ResourceLayoutDesc>>,
    pub resource_sets: Vec<FfiOwnedCreate<ResourceSetDesc>>,
    pub pipeline_layouts: Vec<FfiOwnedCreate<PipelineLayoutDesc>>,
    pub graphics_pipelines: Vec<FfiOwnedCreate<GraphicsPipelineDesc>>,
    pub compute_pipelines: Vec<FfiOwnedCreate<ComputePipelineDesc>>,
    pub render_targets: Vec<FfiOwnedCreate<RenderTargetDesc>>,
    pub render_passes: Vec<FfiOwnedCreate<RenderPassDesc>>,
    pub buffer_updates: Vec<FfiOwnedBufferUpdate>,
    pub texture_updates: Vec<FfiOwnedTextureUpdate>,
    pub destroys: Vec<(Handle, HandleKind)>,
    pub negotiated_feature_bits: u64,
}
pub unsafe fn decode_resource_batch(
    batch: *const FfiResourceBatch,
    capabilities: BackendCapabilities,
) -> GalResult<FfiOwnedResourceBatch> {
    let batch = read_struct(batch, "resource batch")?;
    validate_header::<FfiResourceBatch>(batch.header)?;
    reject_unknown_feature_bits(batch.negotiated_feature_bits)?;
    require_negotiated_features(batch.negotiated_feature_bits, capabilities)?;

    let buffers = read_limited_slice(batch.buffers, true, "buffer creates")?;
    let textures = read_limited_slice(batch.textures, true, "texture creates")?;
    let texture_views = read_limited_slice(batch.texture_views, true, "texture view creates")?;
    let samplers = read_limited_slice(batch.samplers, true, "sampler creates")?;
    let shaders = read_limited_slice(batch.shaders, true, "shader creates")?;
    let resource_layouts =
        read_limited_slice(batch.resource_layouts, true, "resource layout creates")?;
    let resource_layout_bindings = read_limited_slice(
        batch.resource_layout_bindings,
        true,
        "resource layout binding table",
    )?;
    let resource_sets = read_limited_slice(batch.resource_sets, true, "resource set creates")?;
    let resource_set_bindings = read_limited_slice(
        batch.resource_set_bindings,
        true,
        "resource set binding table",
    )?;
    let dynamic_offsets = read_limited_slice(batch.dynamic_offsets, true, "dynamic offset table")?;
    let pipeline_layouts =
        read_limited_slice(batch.pipeline_layouts, true, "pipeline layout creates")?;
    let pipeline_layout_resource_layouts = read_limited_slice(
        batch.pipeline_layout_resource_layouts,
        true,
        "pipeline layout resource layout table",
    )?;
    let graphics_pipelines =
        read_limited_slice(batch.graphics_pipelines, true, "graphics pipeline creates")?;
    let compute_pipelines =
        read_limited_slice(batch.compute_pipelines, true, "compute pipeline creates")?;
    let render_targets = read_limited_slice(batch.render_targets, true, "render target creates")?;
    let render_target_color_views = read_limited_slice(
        batch.render_target_color_views,
        true,
        "render target color view table",
    )?;
    let render_passes = read_limited_slice(batch.render_passes, true, "render pass creates")?;
    let render_pass_color_formats = read_limited_slice(
        batch.render_pass_color_formats,
        true,
        "render pass color format table",
    )?;
    let buffer_updates = read_limited_slice(batch.buffer_updates, true, "buffer updates")?;
    let texture_updates = read_limited_slice(batch.texture_updates, true, "texture updates")?;
    let destroys = read_limited_slice(batch.destroys, true, "destroys")?;
    let total_items = buffers.len()
        + textures.len()
        + texture_views.len()
        + samplers.len()
        + shaders.len()
        + resource_layouts.len()
        + resource_sets.len()
        + pipeline_layouts.len()
        + graphics_pipelines.len()
        + compute_pipelines.len()
        + render_targets.len()
        + render_passes.len()
        + buffer_updates.len()
        + texture_updates.len()
        + destroys.len()
        + resource_layout_bindings.len()
        + resource_set_bindings.len()
        + dynamic_offsets.len()
        + pipeline_layout_resource_layouts.len()
        + render_target_color_views.len()
        + render_pass_color_formats.len();
    if total_items > FFI_MAX_BATCH_ITEMS {
        return Err(GalError::ffi(
            StatusCode::LengthOverflow,
            "resource batch item count exceeds ABI maximum",
        ));
    }

    let mut owned = FfiOwnedResourceBatch {
        negotiated_feature_bits: batch.negotiated_feature_bits,
        ..FfiOwnedResourceBatch::default()
    };
    for item in buffers {
        validate_item_size::<FfiBufferDescAbi>(item.byte_size, "buffer create")?;
        let desc = BufferDesc {
            label: read_label(item.label, "buffer label")?,
            size: item.size,
            memory: memory_domain(item.memory_domain)?,
            usages: buffer_usage_bits(item.usage_bits)?,
        };
        check_buffer_capabilities(&desc, capabilities)?;
        owned.buffers.push(FfiOwnedCreate {
            request_id: item.request_id,
            desc,
        });
    }
    for item in textures {
        validate_item_size::<FfiTextureDescAbi>(item.byte_size, "texture create")?;
        let desc = TextureDesc {
            label: read_label(item.label, "texture label")?,
            dimension: texture_dimension(item.dimension)?,
            format: texture_format(item.format)?,
            extent: item.extent.into(),
            mip_levels: item.mip_levels,
            array_layers: item.array_layers,
            usages: texture_usage_bits(item.usage_bits)?,
        };
        check_texture_capabilities(&desc, capabilities)?;
        owned.textures.push(FfiOwnedCreate {
            request_id: item.request_id,
            desc,
        });
    }
    for item in texture_views {
        validate_item_size::<FfiTextureViewDescAbi>(item.byte_size, "texture view create")?;
        owned.texture_views.push(FfiOwnedCreate {
            request_id: item.request_id,
            desc: TextureViewDesc {
                label: read_label(item.label, "texture view label")?,
                texture: require_handle(item.texture, HandleKind::Texture, "texture view texture")?,
                format: texture_format(item.format)?,
                base_mip: item.base_mip,
                mip_count: item.mip_count,
                base_layer: item.base_layer,
                layer_count: item.layer_count,
            },
        });
    }
    for item in samplers {
        validate_item_size::<FfiSamplerDescAbi>(item.byte_size, "sampler create")?;
        owned.samplers.push(FfiOwnedCreate {
            request_id: item.request_id,
            desc: SamplerDesc {
                label: read_label(item.label, "sampler label")?,
                min_filter: sampler_filter(item.min_filter)?,
                mag_filter: sampler_filter(item.mag_filter)?,
                mip_filter: sampler_filter(item.mip_filter)?,
                address_u: sampler_address(item.address_u)?,
                address_v: sampler_address(item.address_v)?,
                address_w: sampler_address(item.address_w)?,
                comparison: None,
            },
        });
    }
    for item in shaders {
        validate_item_size::<FfiShaderModuleDescAbi>(item.byte_size, "shader create")?;
        let code = read_bounded_bytes(item.code, false, FFI_MAX_SHADER_BYTES, "shader code")?;
        owned.shaders.push(FfiOwnedCreate {
            request_id: item.request_id,
            desc: ShaderModuleDesc {
                label: read_label(item.label, "shader label")?,
                stage: shader_stage(item.stage)?,
                code_format: shader_code_format(item.code_format)?,
                code,
                entry_point: read_label(item.entry_point, "shader entry point")?,
            },
        });
    }
    for item in resource_layouts {
        validate_item_size::<FfiResourceLayoutDescAbi>(item.byte_size, "resource layout create")?;
        let binding_items =
            range_slice(resource_layout_bindings, item.bindings, "layout bindings")?;
        let mut bindings = Vec::with_capacity(binding_items.len());
        for binding in binding_items {
            validate_item_size::<FfiResourceBindingDescAbi>(
                binding.byte_size,
                "resource layout binding",
            )?;
            bindings.push(ResourceBindingDesc {
                binding: binding.binding,
                kind: resource_binding_kind(binding.kind)?,
                stages: stage_flags(binding.stage_bits)?,
                array_count: binding.array_count,
                optional: bool_flag(binding.optional, "resource layout optional binding")?,
                dynamic_offset_count: binding.dynamic_offset_count,
            });
        }
        owned.resource_layouts.push(FfiOwnedCreate {
            request_id: item.request_id,
            desc: ResourceLayoutDesc {
                label: read_label(item.label, "resource layout label")?,
                bindings,
            },
        });
    }
    for item in resource_sets {
        validate_item_size::<FfiResourceSetDescAbi>(item.byte_size, "resource set create")?;
        let binding_items = range_slice(
            resource_set_bindings,
            item.bindings,
            "resource set bindings",
        )?;
        let mut bindings = Vec::with_capacity(binding_items.len());
        for binding in binding_items {
            validate_item_size::<FfiResourceBindingAbi>(binding.byte_size, "resource set binding")?;
            let dynamic_offsets =
                range_slice(dynamic_offsets, binding.dynamic_offsets, "dynamic offsets")?.to_vec();
            bindings.push(ResourceBinding {
                binding: binding.binding,
                array_index: binding.array_index,
                resource: require_any_handle(binding.resource, "resource set binding resource")?,
                kind: resource_binding_kind(binding.kind)?,
                access: access_flags(binding.access_bits)?,
                dynamic_offsets,
                buffer_range: None,
            });
        }
        owned.resource_sets.push(FfiOwnedCreate {
            request_id: item.request_id,
            desc: ResourceSetDesc {
                label: read_label(item.label, "resource set label")?,
                layout: require_handle(
                    item.layout,
                    HandleKind::ResourceLayout,
                    "resource set layout",
                )?,
                bindings,
            },
        });
    }
    for item in pipeline_layouts {
        validate_item_size::<FfiPipelineLayoutDescAbi>(item.byte_size, "pipeline layout create")?;
        let layouts = range_slice(
            pipeline_layout_resource_layouts,
            item.resource_layouts,
            "pipeline layout resource layouts",
        )?
        .iter()
        .map(|handle| require_handle(*handle, HandleKind::ResourceLayout, "pipeline layout set"))
        .collect::<GalResult<Vec<_>>>()?;
        owned.pipeline_layouts.push(FfiOwnedCreate {
            request_id: item.request_id,
            desc: PipelineLayoutDesc {
                label: read_label(item.label, "pipeline layout label")?,
                resource_layouts: layouts,
            },
        });
    }
    for item in graphics_pipelines {
        validate_item_size::<FfiGraphicsPipelineDescAbi>(
            item.byte_size,
            "graphics pipeline create",
        )?;
        let color_formats = range_slice(
            render_pass_color_formats,
            item.color_formats,
            "graphics pipeline color formats",
        )?
        .iter()
        .map(|format| texture_format(*format))
        .collect::<GalResult<Vec<_>>>()?;
        let depth_compare = optional_compare_op(item.depth_compare)?;
        let desc = GraphicsPipelineDesc {
            label: read_label(item.label, "graphics pipeline label")?,
            layout: require_handle(
                item.layout,
                HandleKind::PipelineLayout,
                "graphics pipeline layout",
            )?,
            vertex_shader: require_handle(
                item.vertex_shader,
                HandleKind::ShaderModule,
                "graphics pipeline vertex shader",
            )?,
            fragment_shader: require_handle(
                item.fragment_shader,
                HandleKind::ShaderModule,
                "graphics pipeline fragment shader",
            )?,
            topology: primitive_topology(item.topology)?,
            cull_mode: cull_mode(item.cull_mode)?,
            front_face: crate::render::vulkanic::resources::FrontFace::CounterClockwise,
            provoking_vertex: crate::render::vulkanic::resources::ProvokingVertex::Last,
            raster_y_direction: crate::render::vulkanic::resources::RasterYDirection::Up,
            blend: blend_mode(item.blend)?,
            depth_compare,
            depth_write: depth_compare.is_some(),
            depth_bias: None,
            color_formats,
            depth_format: optional_texture_format(item.depth_format)?,
            stencil: None,
        };
        check_graphics_pipeline_capabilities(&desc, capabilities)?;
        owned.graphics_pipelines.push(FfiOwnedCreate {
            request_id: item.request_id,
            desc,
        });
    }
    for item in compute_pipelines {
        validate_item_size::<FfiComputePipelineDescAbi>(item.byte_size, "compute pipeline create")?;
        require_feature(capabilities, BackendFeature::Compute, "compute pipeline")?;
        owned.compute_pipelines.push(FfiOwnedCreate {
            request_id: item.request_id,
            desc: ComputePipelineDesc {
                label: read_label(item.label, "compute pipeline label")?,
                layout: require_handle(
                    item.layout,
                    HandleKind::PipelineLayout,
                    "compute pipeline layout",
                )?,
                shader: require_handle(
                    item.shader,
                    HandleKind::ShaderModule,
                    "compute pipeline shader",
                )?,
            },
        });
    }
    for item in render_targets {
        validate_item_size::<FfiRenderTargetDescAbi>(item.byte_size, "render target create")?;
        let colors = range_slice(
            render_target_color_views,
            item.color_views,
            "render target color views",
        )?
        .iter()
        .map(|handle| require_handle(*handle, HandleKind::TextureView, "render target color view"))
        .collect::<GalResult<Vec<_>>>()?;
        check_attachment_count(colors.len(), item.depth_stencil_view.raw != 0, capabilities)?;
        owned.render_targets.push(FfiOwnedCreate {
            request_id: item.request_id,
            desc: RenderTargetDesc {
                label: read_label(item.label, "render target label")?,
                color_views: colors,
                depth_stencil_view: optional_handle(
                    item.depth_stencil_view,
                    HandleKind::TextureView,
                    "render target depth view",
                )?,
                extent: item.extent.into(),
            },
        });
    }
    for item in render_passes {
        validate_item_size::<FfiRenderPassDescAbi>(item.byte_size, "render pass create")?;
        let color_formats = range_slice(
            render_pass_color_formats,
            item.color_formats,
            "render pass color formats",
        )?
        .iter()
        .map(|format| texture_format(*format))
        .collect::<GalResult<Vec<ColorFormat>>>()?;
        check_attachment_count(color_formats.len(), item.depth_format != 0, capabilities)?;
        owned.render_passes.push(FfiOwnedCreate {
            request_id: item.request_id,
            desc: RenderPassDesc {
                label: read_label(item.label, "render pass label")?,
                target: require_handle_any(
                    item.target,
                    &[HandleKind::RenderTarget, HandleKind::FrameTarget],
                    "render pass target",
                )?,
                color_formats,
                depth_format: optional_texture_format(item.depth_format)?,
            },
        });
    }
    for item in buffer_updates {
        validate_item_size::<FfiBufferUpdateAbi>(item.byte_size, "buffer update")?;
        owned.buffer_updates.push(FfiOwnedBufferUpdate {
            buffer: require_handle(item.buffer, HandleKind::Buffer, "buffer update target")?,
            offset: item.offset,
            data: read_bounded_bytes(item.data, false, FFI_MAX_INLINE_BYTES, "buffer update data")?,
        });
    }
    for item in texture_updates {
        validate_item_size::<FfiTextureUpdateAbi>(item.byte_size, "texture update")?;
        owned.texture_updates.push(FfiOwnedTextureUpdate {
            texture: require_handle(item.texture, HandleKind::Texture, "texture update target")?,
            mip_level: item.mip_level,
            array_layer: item.array_layer,
            origin: item.origin.into(),
            extent: item.extent.into(),
            bytes_per_row: item.bytes_per_row,
            rows_per_image: item.rows_per_image,
            data: read_bounded_bytes(
                item.data,
                false,
                FFI_MAX_INLINE_BYTES,
                "texture update data",
            )?,
        });
    }
    for item in destroys {
        validate_item_size::<FfiDestroyDescAbi>(item.byte_size, "destroy")?;
        let kind = handle_kind(item.expected_kind)?;
        owned
            .destroys
            .push((require_handle(item.handle, kind, "destroy handle")?, kind));
    }
    Ok(owned)
}
pub fn serialize_resource_batch_canonical(batch: &FfiOwnedResourceBatch) -> Vec<u8> {
    let mut out = Vec::new();
    push_u32(&mut out, FFI_ABI_VERSION);
    push_u64(&mut out, batch.negotiated_feature_bits);
    push_u64(&mut out, batch.buffers.len() as u64);
    for item in &batch.buffers {
        push_create_prefix(&mut out, item.request_id, &item.desc.label);
        push_u64(&mut out, item.desc.size);
        push_u32(&mut out, item.desc.memory as u32);
        push_u64(&mut out, buffer_usage_bits_from_desc(&item.desc.usages));
    }
    push_u64(&mut out, batch.textures.len() as u64);
    for item in &batch.textures {
        push_create_prefix(&mut out, item.request_id, &item.desc.label);
        push_u32(&mut out, item.desc.dimension as u32);
        push_u32(&mut out, item.desc.format as u32);
        push_extent(&mut out, item.desc.extent);
        push_u32(&mut out, item.desc.mip_levels);
        push_u32(&mut out, item.desc.array_layers);
        push_u64(&mut out, texture_usage_bits_from_desc(&item.desc.usages));
    }
    push_u64(&mut out, batch.shaders.len() as u64);
    for item in &batch.shaders {
        push_create_prefix(&mut out, item.request_id, &item.desc.label);
        push_u32(&mut out, item.desc.stage as u32);
        push_u32(&mut out, item.desc.code_format as u32);
        push_bytes(&mut out, &item.desc.code);
        push_str(&mut out, &item.desc.entry_point);
    }
    push_u64(&mut out, batch.destroys.len() as u64);
    for (handle, kind) in &batch.destroys {
        push_u64(&mut out, handle.raw());
        push_u32(&mut out, *kind as u32);
    }
    out
}
pub(crate) fn create_result_capacity_required(batch: &FfiOwnedResourceBatch) -> usize {
    batch.buffers.len()
        + batch.textures.len()
        + batch.texture_views.len()
        + batch.samplers.len()
        + batch.shaders.len()
        + batch.resource_layouts.len()
        + batch.resource_sets.len()
        + batch.pipeline_layouts.len()
        + batch.graphics_pipelines.len()
        + batch.compute_pipelines.len()
        + batch.render_targets.len()
        + batch.render_passes.len()
}

pub(crate) fn execute_create<T>(
    results: &mut Vec<FfiCreateResultEntry>,
    item: &FfiOwnedCreate<T>,
    create: impl FnOnce() -> GalResult<Handle>,
) -> GalResult<()> {
    match create() {
        Ok(handle) => {
            results.push(FfiCreateResultEntry {
                request_id: item.request_id,
                handle: handle.into(),
                status: StatusCode::Ok as i32,
                error_domain: 0,
            });
            Ok(())
        }
        Err(error) => {
            results.push(FfiCreateResultEntry {
                request_id: item.request_id,
                handle: FfiHandle::default(),
                status: error.code as i32,
                error_domain: error.domain as u32,
            });
            Err(error)
        }
    }
}

pub(crate) fn execute_resource_batch(
    context: &mut BridgeContext,
    batch: FfiOwnedResourceBatch,
) -> GalResult<Vec<FfiCreateResultEntry>> {
    if !batch.texture_updates.is_empty() {
        return Err(GalError::unsupported_feature(
            "texture update resource batches are not part of the initial bridge path; use command uploads/copies",
        ));
    }
    let mut results = Vec::with_capacity(create_result_capacity_required(&batch));
    for item in &batch.buffers {
        execute_create(&mut results, item, || {
            context.gal.create_buffer(item.desc.clone())
        })?;
    }
    for item in &batch.textures {
        execute_create(&mut results, item, || {
            context.gal.create_texture(item.desc.clone())
        })?;
    }
    for item in &batch.texture_views {
        execute_create(&mut results, item, || {
            context.gal.create_texture_view(item.desc.clone())
        })?;
    }
    for item in &batch.samplers {
        execute_create(&mut results, item, || {
            context.gal.create_sampler(item.desc.clone())
        })?;
    }
    for item in &batch.shaders {
        execute_create(&mut results, item, || {
            context.gal.create_shader_module(item.desc.clone())
        })?;
    }
    for item in &batch.resource_layouts {
        execute_create(&mut results, item, || {
            context.gal.create_resource_layout(item.desc.clone())
        })?;
    }
    for item in &batch.resource_sets {
        execute_create(&mut results, item, || {
            context.gal.create_resource_set(item.desc.clone())
        })?;
    }
    for item in &batch.pipeline_layouts {
        execute_create(&mut results, item, || {
            context.gal.create_pipeline_layout(item.desc.clone())
        })?;
    }
    for item in &batch.graphics_pipelines {
        execute_create(&mut results, item, || {
            context.gal.create_graphics_pipeline(item.desc.clone())
        })?;
    }
    for item in &batch.compute_pipelines {
        execute_create(&mut results, item, || {
            context.gal.create_compute_pipeline(item.desc.clone())
        })?;
    }
    for item in &batch.render_targets {
        execute_create(&mut results, item, || {
            context.gal.create_render_target(item.desc.clone())
        })?;
    }
    for item in &batch.render_passes {
        execute_create(&mut results, item, || {
            context.gal.create_render_pass(item.desc.clone())
        })?;
    }
    if !batch.buffer_updates.is_empty() {
        let operations = batch
            .buffer_updates
            .into_iter()
            .map(|update| CommandOp::HostWriteBuffer {
                buffer: update.buffer,
                offset: update.offset,
                data: update.data,
            })
            .collect();
        let list = context.gal.create_command_list(CommandListDesc {
            label: "ffi.resource-buffer-updates".to_string(),
            operations,
        })?;
        let _ = context.gal.submit(SubmissionBatch {
            label: "ffi.resource-buffer-update-submit".to_string(),
            command_lists: vec![list],
        })?;
        context.gal.retire_completed()?;
    }
    for (handle, _kind) in batch.destroys {
        context
            .frame_targets
            .retain(|_identity, cached| cached.handle != handle);
        context.stale_frame_targets.retain(|stale| *stale != handle);
        context.gal.destroy(handle)?;
    }
    Ok(results)
}

#[no_mangle]
pub unsafe extern "C" fn mattmc_vulkanic_gal_resource_batch(
    context_id: u64,
    batch: *const FfiResourceBatch,
    results_out: *mut FfiCreateResultEntry,
    results_capacity: u64,
    status_out: *mut FfiStatusResult,
) -> i32 {
    with_registry_mut(|registry| {
        let Some(context) = registry.contexts.get_mut(&context_id) else {
            let error = GalError::ffi(
                StatusCode::StaleHandle,
                format!("unknown context id {context_id}"),
            );
            write_status_out(status_out, status_result_from_error(&error));
            return error.code as i32;
        };
        let input_bytes = if batch.is_null() {
            0
        } else {
            input_bytes_for_resource_batch(&*batch)
        };
        context.ffi_calls += 1;
        context.ffi_input_bytes = context.ffi_input_bytes.saturating_add(input_bytes);
        context.ffi_output_bytes = context
            .ffi_output_bytes
            .saturating_add(size_of::<FfiStatusResult>() as u64)
            .saturating_add(output_bytes_for_resource_results(results_capacity));
        let result = match decode_resource_batch(batch, context.gal.capabilities()).and_then(|owned| {
        let required = create_result_capacity_required(&owned);
        if required > 0 && results_out.is_null() {
            return Err(GalError::ffi(StatusCode::NullPointer, "create results pointer is null"));
        }
        if usize::try_from(results_capacity).unwrap_or(usize::MAX) < required {
            return Err(GalError::ffi(StatusCode::LengthOverflow, format!("create result capacity {results_capacity} is less than required {required}")));
        }
        execute_resource_batch(context, owned)
    }) {
        Ok(results) => {
            for (index, result) in results.iter().copied().enumerate() {
                ptr::write(results_out.add(index), result);
            }
            write_status_out(status_out, status_ok(context));
            StatusCode::Ok as i32
        }
        Err(error) => {
            set_last_error(context, &error);
            write_status_out(status_out, status_error(Some(context), &error));
            error.code as i32
        }
    };
        result
    })
}
