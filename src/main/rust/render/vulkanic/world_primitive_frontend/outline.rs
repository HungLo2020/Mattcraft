use super::*;
use crate::render::vulkanic::commands::CommandOp;
use crate::render::vulkanic::ffi::FFI_MAX_BATCH_ITEMS;
use crate::render::vulkanic::handles::HandleKind;
use crate::render::vulkanic::shader_pack::vanilla_post_effect_contract::VanillaPostEffectExecutionPlan;
use crate::render::vulkanic::shader_pack::vanilla_post_effect_executor::{
    VanillaPostEffectExecutor, VanillaPostEffectInputBinding, VanillaPostEffectPassBinding,
    bundled_entity_outline_executor, bundled_entity_outline_shader_sources, pack_uniform_block,
};

/// Backend-neutral input for the Rust-owned entity-outline mask pass. It
/// intentionally contains only copied mesh identity, transform, and color;
/// resource handles and post-effect targets are resolved by the frontend when
/// the mask graph is admitted.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EntityOutlineMaskInstance {
    pub mesh_key: u64,
    pub mesh_generation: u64,
    pub mesh_section_index: u32,
    pub depth_policy: u32,
    pub cull_policy: u32,
    pub winding: u32,
    pub transform: [f32; 16],
    pub color_argb: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EntityOutlineMaskPlan {
    pub viewport_width: u32,
    pub viewport_height: u32,
    pub instances: Vec<EntityOutlineMaskInstance>,
}

/// Rust-resolved indexed geometry for one outline mask draw. The copied
/// instances are grouped only by explicit mesh generation/section/raster
/// semantics; no Java vertex-buffer identity crosses this boundary.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EntityOutlineMaskDraw {
    pub mesh_key: u64,
    pub mesh_generation: u64,
    pub section_index: u32,
    pub index_offset: u32,
    pub index_count: u32,
    pub index_type: IndexType,
    pub depth_policy: u32,
    pub cull_policy: u32,
    pub winding: u32,
    pub instances: Vec<EntityOutlineMaskInstance>,
}

/// GAL-owned resources prepared for one outline mask submission. The upload
/// bytes remain semantic frontend data until the enclosing command transaction
/// records their HostWriteBuffer operation.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EntityOutlineMaskGpuDraw {
    pub mesh_key: u64,
    pub mesh_generation: u64,
    pub section_index: u32,
    pub index_buffer: Handle,
    pub index_offset: u32,
    pub index_count: u32,
    pub index_type: IndexType,
    pub pipeline: Handle,
    pub pipeline_layout: Handle,
    pub resource_set: Handle,
    pub dynamic_offset: u64,
    pub instance_count: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EntityOutlineMaskGpuResources {
    pub instance_buffer: Handle,
    pub instance_bytes: Vec<u8>,
    pub draws: Vec<EntityOutlineMaskGpuDraw>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EntityOutlinePostEffectPipelines {
    pub color_format: TextureFormat,
    pub vertex_shader: Handle,
    pub sobel_shader: Handle,
    pub blur_shader: Handle,
    pub blit_shader: Handle,
    pub sobel_resource_layout: Handle,
    pub blur_resource_layout: Handle,
    pub blit_resource_layout: Handle,
    pub sobel_layout: Handle,
    pub blur_layout: Handle,
    pub blit_layout: Handle,
    pub sobel_pipeline: Handle,
    pub blur_pipeline: Handle,
    pub blit_pipeline: Handle,
    pub blit_depthless_pipeline: Handle,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EntityOutlinePostEffectResourceSets {
    pub color_format: TextureFormat,
    pub uniform_buffers: [Handle; 3],
    pub resource_sets: [Handle; 4],
}

impl EntityOutlinePostEffectResourceSets {
    pub(crate) fn handles_in_destroy_order(&self) -> [Handle; 7] {
        [
            self.resource_sets[3],
            self.resource_sets[2],
            self.resource_sets[1],
            self.resource_sets[0],
            self.uniform_buffers[2],
            self.uniform_buffers[1],
            self.uniform_buffers[0],
        ]
    }
}

fn pack_outline_post_uniform(values: &[f32; 4]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

/// Creates the four pass-specific resource sets for the bundled outline
/// graph. The final pass still targets the enclosing frame at command time;
/// only its input set is cached here.
pub(crate) fn create_entity_outline_post_effect_resource_sets(
    gal: &mut VulkanicGal,
    targets: &EntityOutlineTargetResources,
    pipelines: &EntityOutlinePostEffectPipelines,
) -> GalResult<EntityOutlinePostEffectResourceSets> {
    if targets.color_format != pipelines.color_format {
        return Err(GalError::invalid_argument(
            "entity outline target and pipeline color formats differ",
        ));
    }
    let mut created = Vec::new();
    let result = (|| -> GalResult<EntityOutlinePostEffectResourceSets> {
        let mut uniform_buffers = Vec::new();
        for (index, values) in [
            [1.0, 0.0, 2.0, 0.0],
            [0.0, 1.0, 2.0, 0.0],
            [1.0, 1.0, 1.0, 1.0],
        ]
        .into_iter()
        .enumerate()
        {
            let buffer = gal.create_buffer(BufferDesc {
                label: format!("minecraft.entity-outline.uniform-{index}"),
                size: 16,
                memory: MemoryDomain::Upload,
                usages: vec![BufferUsage::Uniform, BufferUsage::HostWrite],
            })?;
            created.push(buffer);
            // The bytes are retained in the eventual command transaction; the
            // resource set only establishes the explicit binding and range.
            let _ = pack_outline_post_uniform(&values);
            uniform_buffers.push(buffer);
        }
        let sobel_set = gal.create_resource_set(ResourceSetDesc {
            label: "minecraft.entity-outline.sobel.resource-set".to_string(),
            layout: pipelines.sobel_resource_layout,
            bindings: vec![
                ResourceBinding {
                    binding: 0,
                    array_index: 0,
                    resource: targets.mask_view,
                    kind: ResourceBindingKind::SampledTexture,
                    access: AccessFlags::READ,
                    dynamic_offsets: Vec::new(),
                    buffer_range: None,
                },
                ResourceBinding {
                    binding: 1,
                    array_index: 0,
                    resource: targets.sampler,
                    kind: ResourceBindingKind::Sampler,
                    access: AccessFlags::READ,
                    dynamic_offsets: Vec::new(),
                    buffer_range: None,
                },
            ],
        })?;
        created.push(sobel_set);
        let mut make_uniform_set =
            |label: &str, layout: Handle, input: Handle, uniform: Handle| -> GalResult<Handle> {
                gal.create_resource_set(ResourceSetDesc {
                    label: label.to_string(),
                    layout,
                    bindings: vec![
                        ResourceBinding {
                            binding: 0,
                            array_index: 0,
                            resource: input,
                            kind: ResourceBindingKind::SampledTexture,
                            access: AccessFlags::READ,
                            dynamic_offsets: Vec::new(),
                            buffer_range: None,
                        },
                        ResourceBinding {
                            binding: 1,
                            array_index: 0,
                            resource: targets.sampler,
                            kind: ResourceBindingKind::Sampler,
                            access: AccessFlags::READ,
                            dynamic_offsets: Vec::new(),
                            buffer_range: None,
                        },
                        ResourceBinding {
                            binding: 2,
                            array_index: 0,
                            resource: uniform,
                            kind: ResourceBindingKind::UniformBuffer,
                            access: AccessFlags::READ,
                            dynamic_offsets: Vec::new(),
                            buffer_range: Some(16),
                        },
                    ],
                })
            };
        let blur_horizontal = make_uniform_set(
            "minecraft.entity-outline.blur-horizontal.resource-set",
            pipelines.blur_resource_layout,
            targets.swap_view,
            uniform_buffers[0],
        )?;
        created.push(blur_horizontal);
        let blur_vertical = make_uniform_set(
            "minecraft.entity-outline.blur-vertical.resource-set",
            pipelines.blur_resource_layout,
            targets.outline_view,
            uniform_buffers[1],
        )?;
        created.push(blur_vertical);
        let blit = make_uniform_set(
            "minecraft.entity-outline.blit.resource-set",
            pipelines.blit_resource_layout,
            targets.swap_view,
            uniform_buffers[2],
        )?;
        created.push(blit);
        Ok(EntityOutlinePostEffectResourceSets {
            color_format: targets.color_format,
            uniform_buffers: uniform_buffers.try_into().map_err(|_| {
                GalError::backend("entity outline uniform buffer count changed unexpectedly")
            })?,
            resource_sets: [sobel_set, blur_horizontal, blur_vertical, blit],
        })
    })();
    match result {
        Ok(resources) => Ok(resources),
        Err(error) => {
            for handle in created.into_iter().rev() {
                let _ = gal.destroy(handle);
            }
            Err(error)
        }
    }
}

/// Resolves the four validated semantic passes to the private GAL handles.
/// The final pass is deliberately supplied with the enclosing frame target at
/// call time, so this graph cannot retain or create a second presenter.
pub(crate) fn bind_entity_outline_post_effect_passes(
    plan: &EntityOutlinePostEffectPlan,
    targets: &EntityOutlineTargetResources,
    pipelines: &EntityOutlinePostEffectPipelines,
    sets: &EntityOutlinePostEffectResourceSets,
    frame_pass: Handle,
    frame_target: Handle,
    frame_color_attachment: Handle,
    blit_pipeline: Handle,
) -> GalResult<Vec<VanillaPostEffectPassBinding>> {
    if plan.effect.ordered_passes.len() != 4
        || targets.color_format != pipelines.color_format
        || targets.color_format != sets.color_format
    {
        return Err(GalError::invalid_argument(
            "entity outline pass binding resources do not match the four-pass graph",
        ));
    }
    let output_bindings = [
        (targets.swap_pass, targets.swap_target, targets.swap_view),
        (
            targets.outline_pass,
            targets.outline_target,
            targets.outline_view,
        ),
        (targets.swap_pass, targets.swap_target, targets.swap_view),
        (frame_pass, frame_target, frame_color_attachment),
    ];
    let pipelines_for_pass = [
        (pipelines.sobel_pipeline, pipelines.sobel_layout),
        (pipelines.blur_pipeline, pipelines.blur_layout),
        (pipelines.blur_pipeline, pipelines.blur_layout),
        (blit_pipeline, pipelines.blit_layout),
    ];
    let inputs_for_pass = [
        [(targets.mask_view, false)],
        [(targets.swap_view, true)],
        [(targets.outline_view, true)],
        [(targets.swap_view, false)],
    ];
    let sets_for_pass = sets.resource_sets;
    let mut bindings = Vec::with_capacity(4);
    for index in 0..4 {
        let inputs = plan.effect.ordered_passes[index]
            .inputs
            .iter()
            .zip(inputs_for_pass[index])
            .map(
                |(input, (texture_view, bilinear))| VanillaPostEffectInputBinding {
                    texture_view,
                    sampler: targets.sampler,
                    bilinear: input.bilinear || bilinear,
                    use_depth_buffer: input.use_depth_buffer,
                },
            )
            .collect::<Vec<_>>();
        bindings.push(VanillaPostEffectPassBinding {
            render_pass: output_bindings[index].0,
            render_target: output_bindings[index].1,
            color_attachment: output_bindings[index].2,
            depth_attachment: None,
            pipeline: pipelines_for_pass[index].0,
            pipeline_layout: pipelines_for_pass[index].1,
            resource_set: sets_for_pass[index],
            inputs,
            uniform_values: plan.effect.ordered_passes[index].uniform_values.clone(),
        });
    }
    Ok(bindings)
}

/// Lowers the complete four-pass outline graph with target transitions and
/// typed uniform uploads interleaved at their consuming pass. The frame color
/// target is supplied by the caller only for the final pass.
pub(crate) fn lower_entity_outline_post_effect_with_resources(
    plan: &EntityOutlinePostEffectPlan,
    targets: &EntityOutlineTargetResources,
    pipelines: &EntityOutlinePostEffectPipelines,
    sets: &EntityOutlinePostEffectResourceSets,
    frame_pass: Handle,
    frame_target: Handle,
    frame_color_attachment: Handle,
    blit_pipeline: Handle,
    uniform_before: TextureUsageState,
    swap_before: TextureUsageState,
    outline_before: TextureUsageState,
) -> GalResult<Vec<CommandOp>> {
    let bindings = bind_entity_outline_post_effect_passes(
        plan,
        targets,
        pipelines,
        sets,
        frame_pass,
        frame_target,
        frame_color_attachment,
        blit_pipeline,
    )?;
    let mut operations = Vec::with_capacity(40);
    let uniform_buffers = sets.uniform_buffers;
    for (pass_index, block_name, buffer_index) in [
        (1usize, "BlurConfig", 0usize),
        (2usize, "BlurConfig", 1usize),
        (3usize, "BlitConfig", 2usize),
    ] {
        let bytes = pack_uniform_block(&plan.effect.ordered_passes[pass_index], block_name)?;
        operations.push(CommandOp::Barrier(super::buffer_barrier(
            uniform_buffers[buffer_index],
            uniform_before,
            TextureUsageState::TransferDst,
        )));
        operations.push(CommandOp::HostWriteBuffer {
            buffer: uniform_buffers[buffer_index],
            offset: 0,
            data: bytes,
        });
        operations.push(CommandOp::Barrier(super::buffer_barrier(
            uniform_buffers[buffer_index],
            TextureUsageState::TransferDst,
            TextureUsageState::ShaderRead,
        )));
    }
    let output_textures = [
        Some((targets.swap_texture, swap_before)),
        Some((targets.outline_texture, outline_before)),
        Some((targets.swap_texture, TextureUsageState::ShaderRead)),
        None,
    ];
    let input_after = [
        Some((targets.swap_texture, TextureUsageState::ShaderRead)),
        Some((targets.outline_texture, TextureUsageState::ShaderRead)),
        Some((targets.swap_texture, TextureUsageState::ShaderRead)),
        None,
    ];
    for index in 0..4 {
        if let Some((texture, before)) = output_textures[index] {
            operations.push(CommandOp::Barrier(super::texture_barrier(
                texture,
                before,
                TextureUsageState::ColorAttachment,
            )));
        }
        let single_pass_executor =
            VanillaPostEffectExecutor::new(VanillaPostEffectExecutionPlan {
                effect_name: plan.effect.effect_name.clone(),
                intermediate_targets: plan.effect.intermediate_targets.clone(),
                ordered_passes: vec![plan.effect.ordered_passes[index].clone()],
            })?;
        operations.extend(single_pass_executor.lower(&bindings[index..=index])?);
        if let Some((texture, after)) = input_after[index] {
            operations.push(CommandOp::Barrier(super::texture_barrier(
                texture,
                TextureUsageState::ColorAttachment,
                after,
            )));
        }
    }
    Ok(operations)
}

impl EntityOutlinePostEffectPipelines {
    pub(crate) fn handles_in_destroy_order(&self) -> [Handle; 14] {
        [
            self.blit_pipeline,
            self.blit_depthless_pipeline,
            self.blur_pipeline,
            self.sobel_pipeline,
            self.blit_layout,
            self.blur_layout,
            self.sobel_layout,
            self.blit_resource_layout,
            self.blur_resource_layout,
            self.sobel_resource_layout,
            self.blit_shader,
            self.blur_shader,
            self.sobel_shader,
            self.vertex_shader,
        ]
    }
}

/// Creates the Rust-owned fullscreen programs used by the bundled outline
/// effect. Resource sets remain frame/target-specific, while these immutable
/// shader and pipeline objects can be cached by color format.
pub(crate) fn create_entity_outline_post_effect_pipelines(
    gal: &mut VulkanicGal,
    color_format: TextureFormat,
) -> GalResult<EntityOutlinePostEffectPipelines> {
    let mut created = Vec::new();
    let result = (|| -> GalResult<EntityOutlinePostEffectPipelines> {
        let mut module = |label: &str, stage: ShaderStage, source: &str| {
            gal.create_shader_module(ShaderModuleDesc {
                label: label.to_string(),
                stage,
                code_format: ShaderCodeFormat::Glsl,
                code: super::shader_stage_code_for_backend(BackendApi::Vulkan, source),
                entry_point: "main".to_string(),
            })
        };
        let vertex_shader = module(
            "minecraft.entity-outline.fullscreen.vertex",
            ShaderStage::Vertex,
            MINIMAL_ENTITY_OUTLINE_FULLSCREEN_VERTEX,
        )?;
        created.push(vertex_shader);
        let sobel_shader = module(
            "minecraft.entity-outline.sobel.fragment",
            ShaderStage::Fragment,
            MINIMAL_ENTITY_OUTLINE_SOBEL_FRAGMENT,
        )?;
        created.push(sobel_shader);
        let blur_shader = module(
            "minecraft.entity-outline.blur.fragment",
            ShaderStage::Fragment,
            MINIMAL_ENTITY_OUTLINE_BLUR_FRAGMENT,
        )?;
        created.push(blur_shader);
        let blit_shader = module(
            "minecraft.entity-outline.blit.fragment",
            ShaderStage::Fragment,
            MINIMAL_ENTITY_OUTLINE_BLIT_FRAGMENT,
        )?;
        created.push(blit_shader);
        let mut layout = |label: &str, uniform: bool| {
            let mut bindings = vec![
                ResourceBindingDesc {
                    binding: 0,
                    kind: ResourceBindingKind::SampledTexture,
                    stages: PipelineStageFlags::DRAW,
                    array_count: 1,
                    optional: false,
                    dynamic_offset_count: 0,
                },
                ResourceBindingDesc {
                    binding: 1,
                    kind: ResourceBindingKind::Sampler,
                    stages: PipelineStageFlags::DRAW,
                    array_count: 1,
                    optional: false,
                    dynamic_offset_count: 0,
                },
            ];
            if uniform {
                bindings.push(ResourceBindingDesc {
                    binding: 2,
                    kind: ResourceBindingKind::UniformBuffer,
                    stages: PipelineStageFlags::DRAW,
                    array_count: 1,
                    optional: false,
                    dynamic_offset_count: 0,
                });
            }
            gal.create_resource_layout(ResourceLayoutDesc {
                label: label.to_string(),
                bindings,
            })
        };
        let sobel_resource_layout = layout("minecraft.entity-outline.sobel.layout", false)?;
        created.push(sobel_resource_layout);
        let blur_resource_layout = layout("minecraft.entity-outline.blur.layout", true)?;
        created.push(blur_resource_layout);
        let blit_resource_layout = layout("minecraft.entity-outline.blit.layout", true)?;
        created.push(blit_resource_layout);
        let sobel_layout = gal.create_pipeline_layout(PipelineLayoutDesc {
            label: "minecraft.entity-outline.sobel.pipeline-layout".to_string(),
            resource_layouts: vec![sobel_resource_layout],
        })?;
        created.push(sobel_layout);
        let blur_layout = gal.create_pipeline_layout(PipelineLayoutDesc {
            label: "minecraft.entity-outline.blur.pipeline-layout".to_string(),
            resource_layouts: vec![blur_resource_layout],
        })?;
        created.push(blur_layout);
        let blit_layout = gal.create_pipeline_layout(PipelineLayoutDesc {
            label: "minecraft.entity-outline.blit.pipeline-layout".to_string(),
            resource_layouts: vec![blit_resource_layout],
        })?;
        created.push(blit_layout);
        let mut pipeline = |label: &str, layout: Handle, fragment_shader: Handle, depth_format| {
            gal.create_graphics_pipeline(GraphicsPipelineDesc {
                label: label.to_string(),
                layout,
                vertex_shader,
                fragment_shader,
                topology: PrimitiveTopology::Triangles,
                cull_mode: CullMode::None,
                front_face: FrontFace::CounterClockwise,
                blend: BlendMode::Disabled,
                depth_compare: None,
                depth_write: false,
                depth_bias: None,
                color_formats: vec![color_format],
                depth_format,
                stencil: None,
            })
        };
        let sobel_pipeline = pipeline(
            "minecraft.entity-outline.sobel.pipeline",
            sobel_layout,
            sobel_shader,
            None,
        )?;
        created.push(sobel_pipeline);
        let blur_pipeline = pipeline(
            "minecraft.entity-outline.blur.pipeline",
            blur_layout,
            blur_shader,
            None,
        )?;
        created.push(blur_pipeline);
        let blit_pipeline = pipeline(
            "minecraft.entity-outline.blit.pipeline",
            blit_layout,
            blit_shader,
            Some(TextureFormat::Depth32Float),
        )?;
        created.push(blit_pipeline);
        let blit_depthless_pipeline = pipeline(
            "minecraft.entity-outline.blit-depthless.pipeline",
            blit_layout,
            blit_shader,
            None,
        )?;
        created.push(blit_depthless_pipeline);
        Ok(EntityOutlinePostEffectPipelines {
            color_format,
            vertex_shader,
            sobel_shader,
            blur_shader,
            blit_shader,
            sobel_resource_layout,
            blur_resource_layout,
            blit_resource_layout,
            sobel_layout,
            blur_layout,
            blit_layout,
            sobel_pipeline,
            blur_pipeline,
            blit_pipeline,
            blit_depthless_pipeline,
        })
    })();
    match result {
        Ok(resources) => Ok(resources),
        Err(error) => {
            for handle in created.into_iter().rev() {
                let _ = gal.destroy(handle);
            }
            Err(error)
        }
    }
}

impl EntityOutlineMaskGpuResources {
    pub(crate) fn handles_in_destroy_order(&self) -> Vec<Handle> {
        let mut handles = self
            .draws
            .iter()
            .map(|draw| draw.resource_set)
            .collect::<Vec<_>>();
        handles.push(self.instance_buffer);
        handles
    }
}

/// Lowers the prepared entity mask into one explicit depth-tested pass. The
/// enclosing frame supplies the tracked states of its shared depth image; the
/// mask builder never guesses or mutates backend state implicitly.
pub(crate) fn lower_entity_outline_mask_pass(
    targets: &EntityOutlineTargetResources,
    gpu: &EntityOutlineMaskGpuResources,
    depth_view: Handle,
    depth_before: TextureUsageState,
    depth_after: TextureUsageState,
    mask_before: TextureUsageState,
) -> GalResult<Vec<CommandOp>> {
    depth_view
        .require_kind(HandleKind::TextureView)
        .map_err(|error| {
            GalError::invalid_argument(format!("entity outline depth view is invalid: {error}"))
        })?;
    if targets.mask_depth_view != Some(depth_view) {
        return Err(GalError::invalid_argument(
            "entity outline mask target is not bound to the supplied depth view",
        ));
    }
    let mut operations = Vec::with_capacity(12 + gpu.draws.len() * 5);
    operations.push(CommandOp::Barrier(super::buffer_barrier(
        gpu.instance_buffer,
        TextureUsageState::Undefined,
        TextureUsageState::TransferDst,
    )));
    operations.push(CommandOp::HostWriteBuffer {
        buffer: gpu.instance_buffer,
        offset: 0,
        data: gpu.instance_bytes.clone(),
    });
    operations.push(CommandOp::Barrier(super::buffer_barrier(
        gpu.instance_buffer,
        TextureUsageState::TransferDst,
        TextureUsageState::ShaderRead,
    )));
    operations.push(CommandOp::Barrier(super::texture_barrier(
        targets.mask_texture,
        mask_before,
        TextureUsageState::ColorAttachment,
    )));
    operations.push(CommandOp::Barrier(super::texture_barrier(
        depth_view,
        depth_before,
        TextureUsageState::DepthStencilAttachment,
    )));
    operations.push(CommandOp::BeginPass {
        pass: targets.mask_pass,
        target: targets.mask_target,
        colors: vec![PassAttachment {
            view: targets.mask_view,
            load_op: AttachmentLoadOp::Clear,
            store_op: AttachmentStoreOp::Store,
            clear_color: Some(ClearColor {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }),
        }],
        depth_stencil: Some(PassAttachment {
            view: depth_view,
            load_op: AttachmentLoadOp::Load,
            store_op: AttachmentStoreOp::Store,
            clear_color: None,
        }),
    });
    for draw in &gpu.draws {
        operations.push(CommandOp::BindGraphicsPipeline(draw.pipeline));
        operations.push(CommandOp::BindResourceSet {
            pipeline_layout: draw.pipeline_layout,
            set_index: 0,
            set: draw.resource_set,
            dynamic_offsets: vec![0, draw.dynamic_offset],
        });
        operations.push(CommandOp::SetIndexBuffer {
            buffer: draw.index_buffer,
            offset: draw.index_offset as u64,
            index_type: draw.index_type,
        });
        operations.push(CommandOp::DrawIndexed {
            indices: draw.index_count,
            instances: draw.instance_count,
        });
    }
    operations.push(CommandOp::EndPass);
    operations.push(CommandOp::Barrier(super::texture_barrier(
        targets.mask_texture,
        TextureUsageState::ColorAttachment,
        TextureUsageState::ShaderRead,
    )));
    operations.push(CommandOp::Barrier(super::texture_barrier(
        depth_view,
        TextureUsageState::DepthStencilAttachment,
        depth_after,
    )));
    Ok(operations)
}

/// Immutable semantic preparation for the complete vanilla outline chain.
/// Resource handles are deliberately absent; the Vulkan writer can only admit
/// this plan after it has created private mask/intermediate targets and the
/// corresponding pipelines/resource sets.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EntityOutlinePostEffectPlan {
    pub mask: EntityOutlineMaskPlan,
    pub effect: VanillaPostEffectExecutionPlan,
    pub shader_sources:
        Vec<crate::render::vulkanic::shader_pack::vanilla_post_effect_contract::VanillaPostEffectShaderSource>,
}

/// Packs the outline instances into the existing explicit mesh-instance ABI,
/// replacing terrain color/animation payloads with the copied outline color
/// and deterministic zero animation state. This remains a preparation helper;
/// no command path calls it until outline resource-set binding is admitted.
pub(crate) fn pack_entity_outline_instances(
    instances: &[EntityOutlineMaskInstance],
) -> GalResult<Vec<u8>> {
    if instances.len() > FFI_MAX_BATCH_ITEMS {
        return Err(GalError::invalid_argument(
            "too many entity outline instances",
        ));
    }
    let mut bytes = Vec::with_capacity(instances.len() * super::WORLD_MESH_INSTANCE_BYTES);
    for instance in instances {
        for value in instance.transform {
            super::push_f32(&mut bytes, value);
        }
        for value in super::argb_to_rgba(instance.color_argb) {
            super::push_f32(&mut bytes, value);
        }
        // cutout threshold, animation flag/interpolation/generation, and the
        // two atlas regions are unused by the solid-color outline shader.
        for _ in 0..(super::WORLD_MESH_INSTANCE_BYTES / 4 - 20) {
            super::push_f32(&mut bytes, 0.0);
        }
    }
    Ok(bytes)
}

/// Packs one complete dynamic mesh-instance stream payload, including the
/// explicit camera header consumed by the shared mesh shader ABI. Offsets are
/// returned in draw order and are suitable for a future dynamic resource-set
/// binding.
pub(crate) fn pack_entity_outline_instance_stream(
    frame: &WorldPrimitiveFrame,
    draws: &[EntityOutlineMaskDraw],
) -> GalResult<(Vec<u8>, Vec<u64>)> {
    let mut stream = Vec::new();
    let mut offsets = Vec::with_capacity(draws.len());
    for draw in draws {
        let aligned = super::align_up_u64(
            stream.len() as u64,
            super::WORLD_MESH_INSTANCE_STREAM_ALIGNMENT as u64,
        )?;
        if aligned > stream.len() as u64 {
            stream.resize(aligned as usize, 0);
        }
        offsets.push(aligned);
        stream.extend_from_slice(&super::packed_mesh_uniform_header(frame));
        stream.extend_from_slice(&pack_entity_outline_instances(&draw.instances)?);
    }
    Ok((stream, offsets))
}

/// Persistent Rust-owned color targets used by the entity-outline graph. The
/// acquired frame target is intentionally not stored here; the final blit is
/// bound to the enclosing frame transaction.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EntityOutlineTargetResources {
    pub width: u32,
    pub height: u32,
    pub color_format: TextureFormat,
    /// The current frame's explicit depth attachment used only by the mask
    /// pass. It is borrowed by handle identity and is never destroyed here.
    pub mask_depth_view: Option<Handle>,
    pub sampler: Handle,
    pub mask_texture: Handle,
    pub mask_view: Handle,
    pub mask_target: Handle,
    pub mask_pass: Handle,
    pub swap_texture: Handle,
    pub swap_view: Handle,
    pub swap_target: Handle,
    pub swap_pass: Handle,
    pub outline_texture: Handle,
    pub outline_view: Handle,
    pub outline_target: Handle,
    pub outline_pass: Handle,
}

impl EntityOutlineTargetResources {
    pub(crate) fn handles_in_destroy_order(&self) -> [Handle; 13] {
        [
            self.sampler,
            self.outline_pass,
            self.outline_target,
            self.swap_pass,
            self.swap_target,
            self.mask_pass,
            self.mask_target,
            self.outline_view,
            self.outline_texture,
            self.swap_view,
            self.swap_texture,
            self.mask_view,
            self.mask_texture,
        ]
    }
}

pub(crate) fn create_entity_outline_target_resources(
    gal: &mut VulkanicGal,
    width: u32,
    height: u32,
    color_format: TextureFormat,
) -> GalResult<EntityOutlineTargetResources> {
    create_entity_outline_target_resources_with_depth(gal, width, height, color_format, None)
}

pub(crate) fn create_entity_outline_target_resources_with_depth(
    gal: &mut VulkanicGal,
    width: u32,
    height: u32,
    color_format: TextureFormat,
    mask_depth_view: Option<Handle>,
) -> GalResult<EntityOutlineTargetResources> {
    if width == 0 || height == 0 {
        return Err(GalError::invalid_argument(
            "entity outline targets require a non-zero extent",
        ));
    }
    let extent = Extent3d {
        width,
        height,
        depth: 1,
    };
    let mut created = Vec::new();
    let result = (|| -> GalResult<EntityOutlineTargetResources> {
        let sampler = gal.create_sampler(SamplerDesc {
            label: "minecraft.entity-outline.sampler".to_string(),
            min_filter: SamplerFilter::Linear,
            mag_filter: SamplerFilter::Linear,
            mip_filter: SamplerFilter::Nearest,
            address_u: SamplerAddressMode::ClampToEdge,
            address_v: SamplerAddressMode::ClampToEdge,
            address_w: SamplerAddressMode::ClampToEdge,
            comparison: None,
        })?;
        created.push(sampler);
        let create_target = |gal: &mut VulkanicGal,
                             label: &str,
                             depth_view: Option<Handle>|
         -> GalResult<(Handle, Handle, Handle, Handle)> {
            let texture = gal.create_texture(TextureDesc {
                label: format!("{label}.texture"),
                dimension: TextureDimension::D2,
                format: color_format,
                extent,
                mip_levels: 1,
                array_layers: 1,
                usages: vec![TextureUsage::Sampled, TextureUsage::ColorAttachment],
            })?;
            let view = gal.create_texture_view(TextureViewDesc {
                label: format!("{label}.view"),
                texture,
                format: color_format,
                base_mip: 0,
                mip_count: 1,
                base_layer: 0,
                layer_count: 1,
            })?;
            let target = gal.create_render_target(RenderTargetDesc {
                label: format!("{label}.target"),
                color_views: vec![view],
                depth_stencil_view: depth_view,
                extent,
            })?;
            let pass = gal.create_render_pass(RenderPassDesc {
                label: format!("{label}.pass"),
                target,
                color_formats: vec![color_format],
                depth_format: depth_view.map(|_| TextureFormat::Depth32Float),
            })?;
            Ok((texture, view, target, pass))
        };
        let (mask_texture, mask_view, mask_target, mask_pass) =
            create_target(gal, "minecraft.entity-outline.mask", mask_depth_view)?;
        created.extend([mask_texture, mask_view, mask_target, mask_pass]);
        let (swap_texture, swap_view, swap_target, swap_pass) =
            create_target(gal, "minecraft.entity-outline.swap", None)?;
        created.extend([swap_texture, swap_view, swap_target, swap_pass]);
        let (outline_texture, outline_view, outline_target, outline_pass) =
            create_target(gal, "minecraft.entity-outline.outline", None)?;
        created.extend([outline_texture, outline_view, outline_target, outline_pass]);
        Ok(EntityOutlineTargetResources {
            width,
            height,
            color_format,
            mask_depth_view,
            sampler,
            mask_texture,
            mask_view,
            mask_target,
            mask_pass,
            swap_texture,
            swap_view,
            swap_target,
            swap_pass,
            outline_texture,
            outline_view,
            outline_target,
            outline_pass,
        })
    })();
    if result.is_err() {
        for handle in created.into_iter().rev() {
            let _ = gal.destroy(handle);
        }
    }
    result
}

/// Lowers a prepared outline graph only when every private GAL binding is
/// supplied by the enclosing Rust resource transaction.
pub(crate) fn lower_entity_outline_post_effect(
    plan: &EntityOutlinePostEffectPlan,
    bindings: &[VanillaPostEffectPassBinding],
) -> GalResult<Vec<CommandOp>> {
    let executor = bundled_entity_outline_executor()?;
    if plan.effect != *executor.plan() {
        return Err(GalError::invalid_argument(
            "entity outline plan does not match the bundled Rust executor",
        ));
    }
    executor.lower(bindings)
}

/// Emits the explicit ownership transitions surrounding the mask writer. The
/// caller chooses the tracked prior state because persistent targets survive
/// frames; no implicit Vulkan image state is inferred here.
pub(crate) fn entity_outline_mask_transition_ops(
    resources: &EntityOutlineTargetResources,
    prior_mask_state: TextureUsageState,
) -> Vec<CommandOp> {
    vec![
        CommandOp::Barrier(super::texture_barrier(
            resources.mask_texture,
            prior_mask_state,
            TextureUsageState::ColorAttachment,
        )),
        CommandOp::Barrier(super::texture_barrier(
            resources.mask_texture,
            TextureUsageState::ColorAttachment,
            TextureUsageState::ShaderRead,
        )),
    ]
}

pub(crate) fn prepare_entity_outline_post_effect(
    frame: &WorldPrimitiveFrame,
) -> GalResult<Option<EntityOutlinePostEffectPlan>> {
    let Some(mask) = plan_entity_outline_mask(frame)? else {
        return Ok(None);
    };
    let effect = bundled_entity_outline_executor()?.plan().clone();
    let shader_sources = bundled_entity_outline_shader_sources()?;
    if effect.ordered_passes.len() != 4
        || effect.ordered_passes.first().is_none_or(|pass| {
            pass.inputs
                .first()
                .is_none_or(|input| input.target != "minecraft:entity_outline")
        })
        || effect
            .ordered_passes
            .last()
            .is_none_or(|pass| pass.output != "minecraft:entity_outline")
    {
        return Err(GalError::invalid_argument(
            "bundled entity outline contract does not contain the required mask-to-blit chain",
        ));
    }
    if shader_sources.len() != effect.ordered_passes.len() {
        return Err(GalError::invalid_argument(
            "entity outline shader source count does not match its pass graph",
        ));
    }
    Ok(Some(EntityOutlinePostEffectPlan {
        mask,
        effect,
        shader_sources,
    }))
}

/// Resolves the semantic outline plan against copied Rust mesh assets. This
/// preparation is intentionally handle-free and remains private until a
/// complete mask resource-set writer is available.
pub(crate) fn resolve_entity_outline_mask_draws(
    frontend: &WorldPrimitiveFrontend,
    frame: &WorldPrimitiveFrame,
) -> GalResult<Option<Vec<EntityOutlineMaskDraw>>> {
    let Some(plan) = plan_entity_outline_mask(frame)? else {
        return Ok(None);
    };
    let mut grouped = BTreeMap::<(u64, u64, u32, u32, u32, u32, u32), EntityOutlineMaskDraw>::new();
    for instance in plan.instances {
        let asset = frontend
            .mesh_assets
            .get(&instance.mesh_key)
            .ok_or_else(|| {
                GalError::invalid_argument(format!(
                    "entity outline references unknown mesh key {}",
                    instance.mesh_key
                ))
            })?;
        if asset.mesh_generation != instance.mesh_generation {
            return Err(GalError::invalid_argument(format!(
                "entity outline mesh {} generation {} does not match Rust asset generation {}",
                instance.mesh_key, instance.mesh_generation, asset.mesh_generation
            )));
        }
        let sections = if instance.mesh_section_index == WORLD_MESH_SECTION_ALL {
            (0..asset.sections.len())
                .map(|index| {
                    u32::try_from(index).map_err(|_| {
                        GalError::invalid_argument("entity outline section ordinal exceeds u32")
                    })
                })
                .collect::<GalResult<Vec<_>>>()?
        } else {
            vec![instance.mesh_section_index]
        };
        for section_index in sections {
            let section = asset.sections.get(section_index as usize).ok_or_else(|| {
                GalError::invalid_argument(format!(
                    "entity outline mesh {} selects missing section {}",
                    instance.mesh_key, section_index
                ))
            })?;
            if section.index_count == 0 {
                return Err(GalError::invalid_argument(
                    "entity outline section has no indexed geometry",
                ));
            }
            let cull_policy = if instance.mesh_section_index == WORLD_MESH_SECTION_ALL {
                section.cull_policy
            } else {
                instance.cull_policy
            };
            let winding = if instance.mesh_section_index == WORLD_MESH_SECTION_ALL {
                section.winding
            } else {
                instance.winding
            };
            let key = (
                instance.mesh_key,
                asset.mesh_generation,
                section_index,
                instance.depth_policy,
                cull_policy,
                winding,
                asset.index_type as u32,
            );
            grouped
                .entry(key)
                .and_modify(|draw| draw.instances.push(instance.clone()))
                .or_insert_with(|| EntityOutlineMaskDraw {
                    mesh_key: instance.mesh_key,
                    mesh_generation: asset.mesh_generation,
                    section_index,
                    index_offset: section.index_offset,
                    index_count: section.index_count,
                    index_type: asset.index_type,
                    depth_policy: instance.depth_policy,
                    cull_policy,
                    winding,
                    instances: vec![instance.clone()],
                });
        }
    }
    if grouped
        .values()
        .map(|draw| draw.instances.len())
        .sum::<usize>()
        > FFI_MAX_BATCH_ITEMS
    {
        return Err(GalError::invalid_argument(
            "entity outline mask draw expansion exceeds the bounded instance limit",
        ));
    }
    Ok(Some(grouped.into_values().collect()))
}

/// Collects outlined entity instances without creating resources or recording
/// commands. The caller must still provide the explicit mask target/pipeline
/// graph before this plan can become executable.
pub(crate) fn plan_entity_outline_mask(
    frame: &WorldPrimitiveFrame,
) -> GalResult<Option<EntityOutlineMaskPlan>> {
    let mut instances = Vec::new();
    for instance in &frame.mesh_instances {
        if instance.outline_color_argb == 0 {
            continue;
        }
        if instance.stratum != WORLD_STRATUM_ENTITY_MESH {
            return Err(GalError::invalid_argument(
                "entity outline requests must use the entity mesh stratum",
            ));
        }
        if instance.mesh_key == 0 || instance.mesh_generation == 0 {
            return Err(GalError::invalid_argument(
                "entity outline mesh identity must be non-zero",
            ));
        }
        if instance.transform.iter().any(|value| !value.is_finite()) {
            return Err(GalError::invalid_argument(
                "entity outline transform is not finite",
            ));
        }
        if instances.len() >= FFI_MAX_BATCH_ITEMS {
            return Err(GalError::invalid_argument(format!(
                "entity outline instance count exceeds max {}",
                FFI_MAX_BATCH_ITEMS
            )));
        }
        instances.push(EntityOutlineMaskInstance {
            mesh_key: instance.mesh_key,
            mesh_generation: instance.mesh_generation,
            mesh_section_index: instance.mesh_section_index,
            depth_policy: instance.depth_policy,
            cull_policy: instance.cull_policy,
            winding: instance.winding,
            transform: instance.transform,
            color_argb: instance.outline_color_argb,
        });
    }
    if instances.is_empty() {
        return Ok(None);
    }
    Ok(Some(EntityOutlineMaskPlan {
        viewport_width: frame.viewport_width,
        viewport_height: frame.viewport_height,
        instances,
    }))
}

pub(super) fn validate_segment(
    segment: &WorldLineSegmentRequest,
    frame: &WorldPrimitiveFrame,
) -> GalResult<()> {
    if segment.stratum != WORLD_STRATUM_BLOCK_OUTLINE {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!("unsupported world primitive stratum {}", segment.stratum),
        ));
    }
    if segment.style > 2 {
        return Err(GalError::ffi(
            StatusCode::UnknownEnum,
            format!("unknown world primitive style {}", segment.style),
        ));
    }
    if segment.depth_policy > WORLD_DEPTH_POLICY_TEST_NO_WRITE {
        return Err(GalError::ffi(
            StatusCode::UnknownEnum,
            format!(
                "unknown world primitive depth policy {}",
                segment.depth_policy
            ),
        ));
    }
    if segment.line_width <= 0.0 || !segment.line_width.is_finite() {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "world primitive line width must be finite and positive",
        ));
    }
    if segment.viewport_width != frame.viewport_width
        || segment.viewport_height != frame.viewport_height
    {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "world primitive segment viewport metadata must match the frame viewport",
        ));
    }
    for value in segment.start.iter().chain(segment.end.iter()) {
        if !value.is_finite() {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "world primitive segment coordinates must be finite",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::vulkanic::handles::{Handle, HandleKind};

    fn frame_with_instances(instances: Vec<WorldMeshInstanceRequest>) -> WorldPrimitiveFrame {
        WorldPrimitiveFrame {
            engine_globals: None,
            frame_id: 1,
            correlation_id: 1,
            viewport_width: 128,
            viewport_height: 128,
            view_matrix: [0.0; 16],
            projection_matrix: [0.0; 16],
            voxel_volume: WorldVoxelVolumeFrame::default(),
            shader_environment: WorldShaderEnvironmentFrame::default(),
            feature_coverage: WorldFeatureCoverageFrame::default(),
            first_person: WorldFirstPersonFrame::default(),
            first_person_mesh_instances: Vec::new(),
            background: WorldBackgroundRequest::default(),
            segments: Vec::new(),
            crack_quads: Vec::new(),
            border_quads: Vec::new(),
            material_quads: Vec::new(),
            mesh_instances: instances,
            text_quads: Vec::new(),
            lod_instances: Vec::new(),
            lod_render_frame: WorldLodRenderFrame::default(),
        }
    }

    #[test]
    fn entity_outline_plan_copies_only_outlined_entity_instances() {
        let plain = test_mesh_instance(1);
        let mut outlined = test_mesh_instance(2);
        outlined.outline_color_argb = 0xff_12_34_56;
        let plan = plan_entity_outline_mask(&frame_with_instances(vec![plain, outlined]))
            .unwrap()
            .unwrap();
        assert_eq!(128, plan.viewport_width);
        assert_eq!(1, plan.instances.len());
        assert_eq!(2, plan.instances[0].mesh_key);
        assert_eq!(0xff_12_34_56, plan.instances[0].color_argb);
    }

    #[test]
    fn entity_outline_plan_retains_outline_only_instances_for_mask_execution() {
        let mut outline_only = test_mesh_instance(7);
        outline_only.flags = 1;
        outline_only.outline_color_argb = 0xff_33_66_cc;

        let plan = plan_entity_outline_mask(&frame_with_instances(vec![outline_only]))
            .unwrap()
            .expect("outline-only semantic work must remain available to the mask planner");

        assert_eq!(1, plan.instances.len());
        assert_eq!(7, plan.instances[0].mesh_key);
        assert_eq!(0xff_33_66_cc, plan.instances[0].color_argb);
    }

    #[test]
    fn entity_outline_plan_rejects_non_entity_strata() {
        let mut instance = test_mesh_instance(3);
        instance.stratum = WORLD_STRATUM_TERRAIN;
        instance.outline_color_argb = 0xff_ff_ff_ff;
        let error = plan_entity_outline_mask(&frame_with_instances(vec![instance])).unwrap_err();
        assert!(error.to_string().contains("entity mesh stratum"));
    }

    #[test]
    fn entity_outline_instances_pack_color_and_preserve_mesh_instance_abi() {
        let mut instance = test_mesh_instance(9);
        instance.outline_color_argb = 0xff_12_34_56;
        let plan = plan_entity_outline_mask(&frame_with_instances(vec![instance]))
            .unwrap()
            .unwrap();
        let bytes = pack_entity_outline_instances(&plan.instances).unwrap();
        assert_eq!(super::super::WORLD_MESH_INSTANCE_BYTES, bytes.len());
        let color = &bytes[16 * 4..20 * 4];
        assert_eq!(
            f32::from_le_bytes(color[0..4].try_into().unwrap()),
            0x12 as f32 / 255.0
        );
        assert_eq!(
            f32::from_le_bytes(color[3 * 4..4 * 4].try_into().unwrap()),
            1.0
        );
    }

    #[test]
    fn entity_outline_post_effect_plan_binds_mask_to_the_bundled_four_pass_chain() {
        let mut instance = test_mesh_instance(4);
        instance.outline_color_argb = 0xff_ff_00_00;
        let plan = prepare_entity_outline_post_effect(&frame_with_instances(vec![instance]))
            .unwrap()
            .unwrap();
        assert_eq!(1, plan.mask.instances.len());
        assert_eq!(4, plan.effect.ordered_passes.len());
        assert_eq!(4, plan.shader_sources.len());
        assert!(plan.shader_sources.iter().all(|source| {
            !source.vertex_shader.is_empty() && !source.fragment_shader.is_empty()
        }));
        assert_eq!("minecraft:entity_outline", plan.effect.effect_name);
        assert_eq!(
            "minecraft:entity_outline",
            plan.effect.ordered_passes[0].inputs[0].target
        );
        assert_eq!(
            "minecraft:entity_outline",
            plan.effect.ordered_passes[3].output
        );
    }

    #[test]
    fn entity_outline_geometry_resolution_expands_all_sections_from_owned_asset() {
        let mut instance = test_mesh_instance(19);
        instance.mesh_section_index = WORLD_MESH_SECTION_ALL;
        instance.outline_color_argb = 0xff_ff_00_00;
        let frame = frame_with_instances(vec![instance]);
        let mut frontend = WorldPrimitiveFrontend::default();
        frontend.mesh_assets.insert(
            19,
            super::MeshAssetStore {
                translucent_order: Default::default(),
                mesh_generation: 1,
                index_generation: 1,
                vertex_layout_version: 0,
                vertex_bytes: Vec::new(),
                source_input: None,
                entity_identity: String::new(),
                terrain_voxel_vertices: None,
                terrain_voxel_indices: None,
                index_bytes: Vec::new(),
                index_type: IndexType::U16,
                sections: vec![
                    WorldMeshSection {
                        material_id: 1,
                        texture_id: 1,
                        material_mode: WORLD_MATERIAL_MODE_OPAQUE,
                        cull_policy: WORLD_CULL_BACK,
                        winding: WORLD_WINDING_CCW,
                        index_offset: 0,
                        index_count: 6,
                    },
                    WorldMeshSection {
                        material_id: 2,
                        texture_id: 2,
                        material_mode: WORLD_MATERIAL_MODE_CUTOUT,
                        cull_policy: WORLD_CULL_NONE,
                        winding: WORLD_WINDING_CW,
                        index_offset: 12,
                        index_count: 6,
                    },
                ],
            },
        );
        let draws = resolve_entity_outline_mask_draws(&frontend, &frame)
            .unwrap()
            .unwrap();
        assert_eq!(2, draws.len());
        assert_eq!(
            vec![0, 1],
            draws
                .iter()
                .map(|draw| draw.section_index)
                .collect::<Vec<_>>()
        );
        assert!(draws.iter().all(|draw| draw.instances.len() == 1));
        let (stream, offsets) = pack_entity_outline_instance_stream(&frame, &draws).unwrap();
        assert_eq!(vec![0, 512], offsets);
        assert_eq!(
            512 + super::WORLD_MESH_BATCH_HEADER_BYTES + super::WORLD_MESH_INSTANCE_BYTES,
            stream.len()
        );
    }

    #[test]
    fn entity_outline_lowering_requires_private_bindings_for_every_pass() {
        let mut instance = test_mesh_instance(5);
        instance.outline_color_argb = 0xff_00_ff_00;
        let plan = prepare_entity_outline_post_effect(&frame_with_instances(vec![instance]))
            .unwrap()
            .unwrap();
        let handle = |kind: HandleKind, index: u32| Handle::new(kind, index, 1).unwrap();
        let bindings = plan
            .effect
            .ordered_passes
            .iter()
            .enumerate()
            .map(|(index, pass)| VanillaPostEffectPassBinding {
                render_pass: handle(HandleKind::RenderPass, index as u32 + 1),
                render_target: handle(HandleKind::RenderTarget, index as u32 + 1),
                color_attachment: handle(HandleKind::TextureView, index as u32 + 1),
                depth_attachment: None,
                pipeline: handle(HandleKind::GraphicsPipeline, index as u32 + 1),
                pipeline_layout: handle(HandleKind::PipelineLayout, index as u32 + 1),
                resource_set: handle(HandleKind::ResourceSet, index as u32 + 1),
                inputs: pass
                    .inputs
                    .iter()
                    .map(|input| VanillaPostEffectInputBinding {
                        texture_view: handle(HandleKind::TextureView, index as u32 + 10),
                        sampler: handle(HandleKind::Sampler, index as u32 + 10),
                        bilinear: input.bilinear,
                        use_depth_buffer: input.use_depth_buffer,
                    })
                    .collect(),
                uniform_values: pass.uniform_values.clone(),
            })
            .collect::<Vec<_>>();
        let operations = lower_entity_outline_post_effect(&plan, &bindings).unwrap();
        assert_eq!(20, operations.len());
        assert!(operations.iter().any(|operation| matches!(
            operation,
            CommandOp::Draw {
                vertices: 3,
                instances: 1
            }
        )));
    }

    #[test]
    fn entity_outline_target_resources_reject_zero_extent_before_allocation() {
        let mut gal = super::super::tests::gal();
        let error =
            create_entity_outline_target_resources(&mut gal, 0, 128, TextureFormat::Rgba8Unorm)
                .unwrap_err();
        assert!(error.to_string().contains("non-zero extent"));
    }

    #[test]
    fn entity_outline_mask_target_binds_explicit_depth_view_without_owning_it() {
        let mut gal = super::super::tests::gal();
        let depth = gal
            .create_texture(TextureDesc {
                label: "outline-test-depth".to_string(),
                dimension: TextureDimension::D2,
                format: TextureFormat::Depth32Float,
                extent: Extent3d {
                    width: 64,
                    height: 64,
                    depth: 1,
                },
                mip_levels: 1,
                array_layers: 1,
                usages: vec![TextureUsage::DepthStencilAttachment],
            })
            .unwrap();
        let depth_view = gal
            .create_texture_view(TextureViewDesc {
                label: "outline-test-depth-view".to_string(),
                texture: depth,
                format: TextureFormat::Depth32Float,
                base_mip: 0,
                mip_count: 1,
                base_layer: 0,
                layer_count: 1,
            })
            .unwrap();
        let resources = create_entity_outline_target_resources_with_depth(
            &mut gal,
            64,
            64,
            TextureFormat::Rgba8Unorm,
            Some(depth_view),
        )
        .unwrap();
        assert_eq!(Some(depth_view), resources.mask_depth_view);
        assert!(
            resources
                .handles_in_destroy_order()
                .iter()
                .all(|handle| { *handle != depth && *handle != depth_view })
        );
        for handle in resources.handles_in_destroy_order() {
            gal.destroy(handle).unwrap();
        }
        gal.destroy(depth_view).unwrap();
        gal.destroy(depth).unwrap();
    }

    #[test]
    fn entity_outline_mask_lowering_emits_depth_tested_indexed_pass_and_restores_state() {
        let handle = |kind: HandleKind, index: u32| Handle::new(kind, index, 1).unwrap();
        let targets = EntityOutlineTargetResources {
            width: 64,
            height: 64,
            color_format: TextureFormat::Rgba8Unorm,
            mask_depth_view: Some(handle(HandleKind::TextureView, 20)),
            sampler: handle(HandleKind::Sampler, 13),
            mask_texture: handle(HandleKind::Texture, 1),
            mask_view: handle(HandleKind::TextureView, 2),
            mask_target: handle(HandleKind::RenderTarget, 3),
            mask_pass: handle(HandleKind::RenderPass, 4),
            swap_texture: handle(HandleKind::Texture, 5),
            swap_view: handle(HandleKind::TextureView, 6),
            swap_target: handle(HandleKind::RenderTarget, 7),
            swap_pass: handle(HandleKind::RenderPass, 8),
            outline_texture: handle(HandleKind::Texture, 9),
            outline_view: handle(HandleKind::TextureView, 10),
            outline_target: handle(HandleKind::RenderTarget, 11),
            outline_pass: handle(HandleKind::RenderPass, 12),
        };
        let gpu = EntityOutlineMaskGpuResources {
            instance_buffer: handle(HandleKind::Buffer, 30),
            instance_bytes: vec![0; 336],
            draws: vec![EntityOutlineMaskGpuDraw {
                mesh_key: 1,
                mesh_generation: 1,
                section_index: 0,
                index_buffer: handle(HandleKind::Buffer, 31),
                index_offset: 0,
                index_count: 6,
                index_type: IndexType::U16,
                pipeline: handle(HandleKind::GraphicsPipeline, 32),
                pipeline_layout: handle(HandleKind::PipelineLayout, 33),
                resource_set: handle(HandleKind::ResourceSet, 34),
                dynamic_offset: 0,
                instance_count: 1,
            }],
        };
        let depth_view = targets.mask_depth_view.unwrap();
        let operations = lower_entity_outline_mask_pass(
            &targets,
            &gpu,
            depth_view,
            TextureUsageState::ShaderRead,
            TextureUsageState::ShaderRead,
            TextureUsageState::ShaderRead,
        )
        .unwrap();
        assert!(operations.iter().any(|operation| matches!(
            operation,
            CommandOp::BeginPass {
                depth_stencil: Some(PassAttachment { view, .. }), ..
            } if *view == depth_view
        )));
        assert!(operations.iter().any(|operation| matches!(
            operation,
            CommandOp::DrawIndexed {
                indices: 6,
                instances: 1
            }
        )));
        assert!(
            matches!(operations.last(), Some(CommandOp::Barrier(barrier))
            if barrier.resource == depth_view
                && barrier.after == TextureUsageState::ShaderRead)
        );
    }

    #[test]
    fn entity_outline_target_resources_are_cached_and_reset_owned() {
        let mut gal = super::super::tests::gal();
        let mut frontend = WorldPrimitiveFrontend::default();
        let first = frontend
            .ensure_entity_outline_target_resources(&mut gal, 64, 64, TextureFormat::Rgba8Unorm)
            .unwrap()
            .clone();
        let second = frontend
            .ensure_entity_outline_target_resources(&mut gal, 64, 64, TextureFormat::Rgba8Unorm)
            .unwrap()
            .clone();
        assert_eq!(first, second);
        frontend.reset(&mut gal);
        assert!(frontend.entity_outline_targets.is_none());
    }

    #[test]
    fn entity_outline_mask_transitions_are_explicit_and_persistent_state_aware() {
        let handle = |kind: HandleKind, index: u32| Handle::new(kind, index, 1).unwrap();
        let resources = EntityOutlineTargetResources {
            width: 64,
            height: 64,
            color_format: TextureFormat::Rgba8Unorm,
            mask_depth_view: None,
            sampler: handle(HandleKind::Sampler, 13),
            mask_texture: handle(HandleKind::Texture, 1),
            mask_view: handle(HandleKind::TextureView, 2),
            mask_target: handle(HandleKind::RenderTarget, 3),
            mask_pass: handle(HandleKind::RenderPass, 4),
            swap_texture: handle(HandleKind::Texture, 5),
            swap_view: handle(HandleKind::TextureView, 6),
            swap_target: handle(HandleKind::RenderTarget, 7),
            swap_pass: handle(HandleKind::RenderPass, 8),
            outline_texture: handle(HandleKind::Texture, 9),
            outline_view: handle(HandleKind::TextureView, 10),
            outline_target: handle(HandleKind::RenderTarget, 11),
            outline_pass: handle(HandleKind::RenderPass, 12),
        };
        let operations =
            entity_outline_mask_transition_ops(&resources, TextureUsageState::ShaderRead);
        assert!(matches!(operations[0], CommandOp::Barrier(ref barrier)
            if barrier.resource == resources.mask_texture
                && barrier.before == TextureUsageState::ShaderRead
                && barrier.after == TextureUsageState::ColorAttachment));
        assert!(matches!(operations[1], CommandOp::Barrier(ref barrier)
            if barrier.before == TextureUsageState::ColorAttachment
                && barrier.after == TextureUsageState::ShaderRead));
    }

    #[test]
    fn entity_outline_pipeline_uses_dedicated_solid_color_program() {
        let mut gal = super::super::tests::gal();
        let mut frontend = WorldPrimitiveFrontend::default();
        let key = frontend
            .ensure_entity_outline_pipeline_resources(
                &mut gal,
                ColorFormat::Rgba8Unorm,
                WORLD_WINDING_CCW,
                WORLD_DEPTH_POLICY_TEST_WRITE,
                WORLD_CULL_BACK,
            )
            .unwrap();
        assert_eq!(
            "vulkanic:builtin/entity_outline_mask_v1",
            key.shader_program_identity.as_str()
        );
        let resources = frontend.mesh_pipeline_resources.get(&key).unwrap();
        assert!(resources.shadow_pipeline.is_none());
        frontend.reset(&mut gal);
        assert!(frontend.mesh_pipeline_resources.is_empty());
    }

    #[test]
    fn entity_outline_post_effect_pipelines_are_rust_owned_and_destroyable() {
        let mut gal = super::super::tests::gal();
        let resources =
            create_entity_outline_post_effect_pipelines(&mut gal, TextureFormat::Rgba8Unorm)
                .unwrap();
        assert_ne!(Handle::NULL, resources.sobel_pipeline);
        assert_ne!(Handle::NULL, resources.blur_pipeline);
        assert_ne!(Handle::NULL, resources.blit_pipeline);
        for handle in resources.handles_in_destroy_order() {
            gal.destroy(handle).unwrap();
        }
    }

    #[test]
    fn entity_outline_post_effect_resource_sets_bind_owned_targets_and_uniforms() {
        let mut gal = super::super::tests::gal();
        let targets =
            create_entity_outline_target_resources(&mut gal, 64, 64, TextureFormat::Rgba8Unorm)
                .unwrap();
        let pipelines =
            create_entity_outline_post_effect_pipelines(&mut gal, TextureFormat::Rgba8Unorm)
                .unwrap();
        let sets = create_entity_outline_post_effect_resource_sets(&mut gal, &targets, &pipelines)
            .unwrap();
        assert_eq!(4, sets.resource_sets.len());
        assert_eq!(3, sets.uniform_buffers.len());
        for handle in sets.handles_in_destroy_order() {
            gal.destroy(handle).unwrap();
        }
        for handle in pipelines.handles_in_destroy_order() {
            gal.destroy(handle).unwrap();
        }
        for handle in targets.handles_in_destroy_order() {
            gal.destroy(handle).unwrap();
        }
    }

    #[test]
    fn entity_outline_frontend_caches_and_retires_post_effect_sets() {
        let mut gal = super::super::tests::gal();
        let mut frontend = WorldPrimitiveFrontend::default();
        frontend
            .ensure_entity_outline_target_resources(&mut gal, 64, 64, TextureFormat::Rgba8Unorm)
            .unwrap();
        frontend
            .ensure_entity_outline_post_effect_pipelines(&mut gal, TextureFormat::Rgba8Unorm)
            .unwrap();
        let first = frontend
            .ensure_entity_outline_post_effect_resource_sets(&mut gal, TextureFormat::Rgba8Unorm)
            .unwrap()
            .clone();
        let second = frontend
            .ensure_entity_outline_post_effect_resource_sets(&mut gal, TextureFormat::Rgba8Unorm)
            .unwrap()
            .clone();
        assert_eq!(first, second);
        frontend.reset(&mut gal);
        assert!(frontend.entity_outline_post_effect_sets.is_none());
    }

    #[test]
    fn entity_outline_pass_binding_uses_frame_target_only_for_final_pass() {
        let mut gal = super::super::tests::gal();
        let targets =
            create_entity_outline_target_resources(&mut gal, 64, 64, TextureFormat::Rgba8Unorm)
                .unwrap();
        let pipelines =
            create_entity_outline_post_effect_pipelines(&mut gal, TextureFormat::Rgba8Unorm)
                .unwrap();
        let sets = create_entity_outline_post_effect_resource_sets(&mut gal, &targets, &pipelines)
            .unwrap();
        let mut instance = test_mesh_instance(44);
        instance.outline_color_argb = 0xff_ff_ff_ff;
        let plan = prepare_entity_outline_post_effect(&frame_with_instances(vec![instance]))
            .unwrap()
            .unwrap();
        let frame_pass = Handle::new(HandleKind::RenderPass, 80, 1).unwrap();
        let frame_target = Handle::new(HandleKind::RenderTarget, 81, 1).unwrap();
        let frame_color = Handle::new(HandleKind::TextureView, 82, 1).unwrap();
        let bindings = bind_entity_outline_post_effect_passes(
            &plan,
            &targets,
            &pipelines,
            &sets,
            frame_pass,
            frame_target,
            frame_color,
            pipelines.blit_depthless_pipeline,
        )
        .unwrap();
        assert_eq!(4, bindings.len());
        assert_eq!(targets.swap_target, bindings[0].render_target);
        assert_eq!(targets.outline_target, bindings[1].render_target);
        assert_eq!(targets.swap_target, bindings[2].render_target);
        assert_eq!(frame_target, bindings[3].render_target);
        for handle in sets.handles_in_destroy_order() {
            gal.destroy(handle).unwrap();
        }
        for handle in pipelines.handles_in_destroy_order() {
            gal.destroy(handle).unwrap();
        }
        for handle in targets.handles_in_destroy_order() {
            gal.destroy(handle).unwrap();
        }
    }

    #[test]
    fn entity_outline_post_effect_lowering_interleaves_uniforms_targets_and_draws() {
        let mut gal = super::super::tests::gal();
        let targets =
            create_entity_outline_target_resources(&mut gal, 64, 64, TextureFormat::Rgba8Unorm)
                .unwrap();
        let pipelines =
            create_entity_outline_post_effect_pipelines(&mut gal, TextureFormat::Rgba8Unorm)
                .unwrap();
        let sets = create_entity_outline_post_effect_resource_sets(&mut gal, &targets, &pipelines)
            .unwrap();
        let mut instance = test_mesh_instance(45);
        instance.outline_color_argb = 0xff_ff_ff_ff;
        let plan = prepare_entity_outline_post_effect(&frame_with_instances(vec![instance]))
            .unwrap()
            .unwrap();
        let operations = lower_entity_outline_post_effect_with_resources(
            &plan,
            &targets,
            &pipelines,
            &sets,
            Handle::new(HandleKind::RenderPass, 90, 1).unwrap(),
            Handle::new(HandleKind::RenderTarget, 91, 1).unwrap(),
            Handle::new(HandleKind::TextureView, 92, 1).unwrap(),
            pipelines.blit_depthless_pipeline,
            TextureUsageState::ShaderRead,
            TextureUsageState::Undefined,
            TextureUsageState::Undefined,
        )
        .unwrap();
        assert_eq!(
            3,
            operations
                .iter()
                .filter(|operation| matches!(operation, CommandOp::HostWriteBuffer { .. }))
                .count()
        );
        assert_eq!(
            4,
            operations
                .iter()
                .filter(|operation| matches!(
                    operation,
                    CommandOp::Draw {
                        vertices: 3,
                        instances: 1
                    }
                ))
                .count()
        );
        assert!(operations.iter().any(|operation| matches!(
            operation,
            CommandOp::Barrier(barrier)
                if barrier.resource == targets.swap_texture
                    && barrier.after == TextureUsageState::ColorAttachment
        )));
        for handle in sets.handles_in_destroy_order() {
            gal.destroy(handle).unwrap();
        }
        for handle in pipelines.handles_in_destroy_order() {
            gal.destroy(handle).unwrap();
        }
        for handle in targets.handles_in_destroy_order() {
            gal.destroy(handle).unwrap();
        }
    }

    #[test]
    fn entity_outline_mask_gpu_preparation_owns_stream_and_resource_sets() {
        let mut gal = super::super::tests::gal();
        let mut frontend = WorldPrimitiveFrontend::default();
        let asset = super::super::tests::mesh_asset(23, 1, IndexType::U16);
        frontend.mesh_assets.insert(
            23,
            super::super::MeshAssetStore {
                translucent_order: Default::default(),
                mesh_generation: asset.mesh_generation,
                index_generation: 1,
                vertex_layout_version: asset.vertex_layout_version,
                vertex_bytes: super::super::packed_mesh_vertices(&asset.vertices),
                source_input: None,
                entity_identity: String::new(),
                terrain_voxel_vertices: None,
                terrain_voxel_indices: None,
                index_bytes: asset.index_bytes.clone(),
                index_type: asset.index_type,
                sections: asset.sections.clone(),
            },
        );
        let mut instance = test_mesh_instance(23);
        instance.outline_color_argb = 0xff_00_80_ff;
        let frame = frame_with_instances(vec![instance]);
        let resources = frontend
            .prepare_entity_outline_mask_gpu_resources(&mut gal, &frame, ColorFormat::Rgba8Unorm)
            .unwrap()
            .unwrap()
            .clone();
        assert_eq!(1, resources.draws.len());
        assert_eq!(1, resources.draws[0].instance_count);
        assert!(resources.instance_bytes.len() >= super::WORLD_MESH_BATCH_HEADER_BYTES);
        assert_ne!(Handle::NULL, resources.instance_buffer);
        assert_ne!(Handle::NULL, resources.draws[0].resource_set);
        frontend.reset(&mut gal);
        assert!(frontend.entity_outline_mask_gpu.is_none());
    }

    fn test_mesh_instance(mesh_key: u64) -> WorldMeshInstanceRequest {
        WorldMeshInstanceRequest {
            item_foil: None,
            stratum: WORLD_STRATUM_ENTITY_MESH,
            mesh_key,
            mesh_generation: 1,
            mesh_section_index: WORLD_MESH_SECTION_ALL,
            depth_policy: WORLD_DEPTH_POLICY_TEST_WRITE,
            cull_policy: WORLD_CULL_BACK,
            winding: WORLD_WINDING_CCW,
            color_argb: 0xffff_ffff,
            entity_id: 0,
            entity_color_argb: 0,
            outline_color_argb: 0,
            flags: 0,
            block_entity_id: -1,
            transform: [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
            viewport_width: 128,
            viewport_height: 128,
        }
    }
}
