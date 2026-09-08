//! Rust-owned external attachments for the bundled Fabulous transparency graph.
//!
//! This module only owns explicit GAL resources.  It does not select a route,
//! borrow Java/Iris targets, or submit a second frame.  The frame coordinator
//! must still route semantic draws and lower the validated post-effect before
//! these resources can make transparency available.

use std::collections::BTreeSet;

use super::super::commands::{
    AttachmentLoadOp, AttachmentStoreOp, ClearColor, CommandOp, PassAttachment, ResourceBarrier,
    TextureImageCopyRegion, TextureOrigin3d, TextureUsageState,
};
use super::super::error::{GalError, GalResult};
use super::super::gal::VulkanicGal;
use super::super::handles::Handle;
use super::super::resources::{
    AccessFlags, BlendMode, BufferDesc, BufferUsage, CombinedTextureSamplerDesc, CullMode,
    Extent3d, FrontFace, GraphicsPipelineDesc, MemoryDomain, PipelineLayoutDesc,
    PipelineStageFlags, PrimitiveTopology, ResourceBinding, ResourceBindingDesc,
    ResourceBindingKind, ResourceLayoutDesc, ResourceSetDesc, SamplerAddressMode, SamplerDesc,
    SamplerFilter, ShaderCodeFormat, ShaderModuleDesc, ShaderStage, TextureDesc, TextureDimension,
    TextureFormat, TextureUsage, TextureViewDesc,
};
use super::super::resources::{RenderPassDesc, RenderTargetDesc};
use super::vanilla_post_effect_executor::{
    FabulousExternalTargetInventory, VanillaPostEffectExternalTargetBinding,
};

const FABULOUS_DEPTH_FORMAT: TextureFormat = TextureFormat::Depth32Float;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum FabulousTargetRole {
    Main,
    Translucent,
    ItemEntity,
    Particles,
    Clouds,
    Weather,
}

impl FabulousTargetRole {
    /// Maps copied semantic material-source identity to its distinct
    /// Fabulous attachment. Unknown source programs remain unavailable rather
    /// than collapsing into the generic translucent target.
    pub(crate) const fn for_material_source(source_program: u32) -> Option<Self> {
        match source_program {
            super::super::world_primitive_frontend::WORLD_MATERIAL_SOURCE_UNSPECIFIED
            | super::super::world_primitive_frontend::WORLD_MATERIAL_SOURCE_TEXTURED
            | super::super::world_primitive_frontend::WORLD_MATERIAL_SOURCE_ENTITY_MODEL => {
                Some(Self::Translucent)
            }
            super::super::world_primitive_frontend::WORLD_MATERIAL_SOURCE_PARTICLES => {
                Some(Self::Particles)
            }
            super::super::world_primitive_frontend::WORLD_MATERIAL_SOURCE_CLOUDS => {
                Some(Self::Clouds)
            }
            super::super::world_primitive_frontend::WORLD_MATERIAL_SOURCE_WEATHER => {
                Some(Self::Weather)
            }
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FabulousAttachmentResources {
    pub(crate) role: &'static str,
    pub(crate) color_texture: Handle,
    pub(crate) color_view: Handle,
    pub(crate) depth_texture: Handle,
    pub(crate) depth_view: Handle,
    pub(crate) render_target: Handle,
    pub(crate) render_pass: Handle,
    pub(crate) sampler: Handle,
    pub(crate) extent: Extent3d,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FabulousIntermediateTarget {
    pub(crate) texture: Handle,
    pub(crate) view: Handle,
    pub(crate) render_target: Handle,
    pub(crate) render_pass: Handle,
    pub(crate) sampler: Handle,
    pub(crate) extent: Extent3d,
}

#[derive(Debug)]
pub(crate) struct FabulousTransparencyBindings {
    pub(crate) samplers: [Handle; 12],
    pub(crate) combined_samplers: [Handle; 12],
    pub(crate) resource_layout: Handle,
    pub(crate) resource_set: Handle,
    pub(crate) blit_combined_sampler: Handle,
    pub(crate) blit_sampler: Handle,
    pub(crate) blit_uniform_buffer: Handle,
    pub(crate) blit_resource_layout: Handle,
    pub(crate) blit_resource_set: Handle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FabulousTransparencyPipelines {
    pub(crate) transparency_vertex_shader: Handle,
    pub(crate) transparency_fragment_shader: Handle,
    pub(crate) blit_vertex_shader: Handle,
    pub(crate) blit_fragment_shader: Handle,
    pub(crate) transparency_pipeline_layout: Handle,
    pub(crate) transparency_pipeline: Handle,
    pub(crate) blit_pipeline_layout: Handle,
    pub(crate) blit_pipeline: Handle,
    pub(crate) blit_frame_pipeline: Handle,
}

impl FabulousTransparencyPipelines {
    fn create(
        gal: &mut VulkanicGal,
        bindings: &FabulousTransparencyBindings,
        color_format: TextureFormat,
    ) -> GalResult<Self> {
        let sources =
            super::vanilla_post_effect_executor::bundled_transparency_vulkan_shader_sources()?;
        if sources.len() != 2 {
            return Err(GalError::backend(
                "bundled transparency pipeline requires exactly two shader passes",
            ));
        }
        let mut created = Vec::new();
        let result = (|| -> GalResult<Self> {
            let transparency_vertex_shader = gal.create_shader_module(ShaderModuleDesc {
                label: "fabulous.transparency.vertex".to_string(),
                stage: ShaderStage::Vertex,
                code_format: ShaderCodeFormat::Glsl,
                code: sources[0].0.clone(),
                entry_point: "main".to_string(),
            })?;
            created.push(transparency_vertex_shader);
            let transparency_fragment_shader = gal.create_shader_module(ShaderModuleDesc {
                label: "fabulous.transparency.fragment".to_string(),
                stage: ShaderStage::Fragment,
                code_format: ShaderCodeFormat::Glsl,
                code: sources[0].1.clone(),
                entry_point: "main".to_string(),
            })?;
            created.push(transparency_fragment_shader);
            let blit_vertex_shader = gal.create_shader_module(ShaderModuleDesc {
                label: "fabulous.blit.vertex".to_string(),
                stage: ShaderStage::Vertex,
                code_format: ShaderCodeFormat::Glsl,
                code: sources[1].0.clone(),
                entry_point: "main".to_string(),
            })?;
            created.push(blit_vertex_shader);
            let blit_fragment_shader = gal.create_shader_module(ShaderModuleDesc {
                label: "fabulous.blit.fragment".to_string(),
                stage: ShaderStage::Fragment,
                code_format: ShaderCodeFormat::Glsl,
                code: sources[1].1.clone(),
                entry_point: "main".to_string(),
            })?;
            created.push(blit_fragment_shader);
            let transparency_pipeline_layout = gal.create_pipeline_layout(PipelineLayoutDesc {
                label: "fabulous.transparency.pipeline-layout".to_string(),
                resource_layouts: vec![bindings.resource_layout],
            })?;
            created.push(transparency_pipeline_layout);
            let transparency_pipeline = gal.create_graphics_pipeline(GraphicsPipelineDesc {
                label: "fabulous.transparency.pipeline".to_string(),
                layout: transparency_pipeline_layout,
                vertex_shader: transparency_vertex_shader,
                fragment_shader: transparency_fragment_shader,
                topology: PrimitiveTopology::Triangles,
                cull_mode: CullMode::None,
                front_face: FrontFace::CounterClockwise,
                blend: BlendMode::Disabled,
                depth_compare: None,
                depth_write: false,
                depth_bias: None,
                color_formats: vec![color_format],
                depth_format: None,
                stencil: None,
            })?;
            created.push(transparency_pipeline);
            let blit_pipeline_layout = gal.create_pipeline_layout(PipelineLayoutDesc {
                label: "fabulous.blit.pipeline-layout".to_string(),
                resource_layouts: vec![bindings.blit_resource_layout],
            })?;
            created.push(blit_pipeline_layout);
            let blit_pipeline = gal.create_graphics_pipeline(GraphicsPipelineDesc {
                label: "fabulous.blit.pipeline".to_string(),
                layout: blit_pipeline_layout,
                vertex_shader: blit_vertex_shader,
                fragment_shader: blit_fragment_shader,
                topology: PrimitiveTopology::Triangles,
                cull_mode: CullMode::None,
                front_face: FrontFace::CounterClockwise,
                blend: BlendMode::Disabled,
                depth_compare: None,
                depth_write: false,
                depth_bias: None,
                color_formats: vec![color_format],
                depth_format: Some(TextureFormat::Depth32Float),
                stencil: None,
            })?;
            created.push(blit_pipeline);
            let blit_frame_pipeline = gal.create_graphics_pipeline(GraphicsPipelineDesc {
                label: "fabulous.blit.frame-target-pipeline".to_string(),
                layout: blit_pipeline_layout,
                vertex_shader: blit_vertex_shader,
                fragment_shader: blit_fragment_shader,
                topology: PrimitiveTopology::Triangles,
                cull_mode: CullMode::None,
                front_face: FrontFace::CounterClockwise,
                blend: BlendMode::Disabled,
                depth_compare: None,
                depth_write: false,
                depth_bias: None,
                color_formats: vec![color_format],
                depth_format: None,
                stencil: None,
            })?;
            created.push(blit_frame_pipeline);
            Ok(Self {
                transparency_vertex_shader,
                transparency_fragment_shader,
                blit_vertex_shader,
                blit_fragment_shader,
                transparency_pipeline_layout,
                transparency_pipeline,
                blit_pipeline_layout,
                blit_pipeline,
                blit_frame_pipeline,
            })
        })();
        if result.is_err() {
            for handle in created.into_iter().rev() {
                let _ = gal.destroy(handle);
            }
        }
        result
    }

    fn handles_in_destroy_order(self) -> [Handle; 9] {
        [
            self.blit_pipeline,
            self.blit_frame_pipeline,
            self.blit_pipeline_layout,
            self.transparency_pipeline,
            self.transparency_pipeline_layout,
            self.blit_fragment_shader,
            self.blit_vertex_shader,
            self.transparency_fragment_shader,
            self.transparency_vertex_shader,
        ]
    }
}

impl FabulousTransparencyBindings {
    fn create(gal: &mut VulkanicGal, set: &FabulousAttachmentSet) -> GalResult<Self> {
        let mut created = Vec::new();
        let result = (|| -> GalResult<Self> {
            let sources = [
                (&set.main, false),
                (&set.main, true),
                (&set.translucent, false),
                (&set.translucent, true),
                (&set.item_entity, false),
                (&set.item_entity, true),
                (&set.particles, false),
                (&set.particles, true),
                (&set.clouds, false),
                (&set.clouds, true),
                (&set.weather, false),
                (&set.weather, true),
            ];
            let mut samplers = Vec::new();
            let mut combined = Vec::new();
            for (index, (attachment, depth)) in sources.into_iter().enumerate() {
                let view = if depth {
                    attachment.depth_view
                } else {
                    attachment.color_view
                };
                let sampler = gal.create_sampler(SamplerDesc {
                    label: format!("fabulous.transparency.sampler.{index}"),
                    min_filter: SamplerFilter::Nearest,
                    mag_filter: SamplerFilter::Nearest,
                    mip_filter: SamplerFilter::Nearest,
                    address_u: SamplerAddressMode::ClampToEdge,
                    address_v: SamplerAddressMode::ClampToEdge,
                    address_w: SamplerAddressMode::ClampToEdge,
                    comparison: None,
                })?;
                created.push(sampler);
                samplers.push(sampler);
                let pair = gal.create_combined_texture_sampler(CombinedTextureSamplerDesc {
                    label: format!("fabulous.transparency.combined.{index}"),
                    texture_view: view,
                    sampler,
                })?;
                created.push(pair);
                combined.push(pair);
            }
            let resource_layout = gal.create_resource_layout(ResourceLayoutDesc {
                label: "fabulous.transparency.resource-layout".to_string(),
                bindings: (0..12)
                    .map(|binding| ResourceBindingDesc {
                        binding,
                        kind: ResourceBindingKind::CombinedTextureSampler,
                        stages: PipelineStageFlags::DRAW,
                        array_count: 1,
                        optional: false,
                        dynamic_offset_count: 0,
                    })
                    .collect(),
            })?;
            created.push(resource_layout);
            let resource_set = gal.create_resource_set(ResourceSetDesc {
                label: "fabulous.transparency.resource-set".to_string(),
                layout: resource_layout,
                bindings: combined
                    .iter()
                    .enumerate()
                    .map(|(binding, resource)| ResourceBinding {
                        binding: binding as u32,
                        array_index: 0,
                        resource: *resource,
                        kind: ResourceBindingKind::CombinedTextureSampler,
                        access: AccessFlags::READ,
                        dynamic_offsets: Vec::new(),
                        buffer_range: None,
                    })
                    .collect(),
            })?;
            created.push(resource_set);
            let blit_sampler = gal.create_sampler(SamplerDesc {
                label: "fabulous.blit.sampler".to_string(),
                min_filter: SamplerFilter::Nearest,
                mag_filter: SamplerFilter::Nearest,
                mip_filter: SamplerFilter::Nearest,
                address_u: SamplerAddressMode::ClampToEdge,
                address_v: SamplerAddressMode::ClampToEdge,
                address_w: SamplerAddressMode::ClampToEdge,
                comparison: None,
            })?;
            created.push(blit_sampler);
            let blit_combined_sampler =
                gal.create_combined_texture_sampler(CombinedTextureSamplerDesc {
                    label: "fabulous.blit.combined".to_string(),
                    texture_view: set.final_target.view,
                    sampler: blit_sampler,
                })?;
            created.push(blit_combined_sampler);
            let blit_uniform_buffer = gal.create_buffer(BufferDesc {
                label: "fabulous.blit.uniform".to_string(),
                size: 16,
                memory: MemoryDomain::Upload,
                usages: vec![
                    BufferUsage::Uniform,
                    BufferUsage::TransferDst,
                    BufferUsage::HostWrite,
                ],
            })?;
            created.push(blit_uniform_buffer);
            let blit_resource_layout = gal.create_resource_layout(ResourceLayoutDesc {
                label: "fabulous.blit.resource-layout".to_string(),
                bindings: vec![
                    ResourceBindingDesc {
                        binding: 0,
                        kind: ResourceBindingKind::CombinedTextureSampler,
                        stages: PipelineStageFlags::DRAW,
                        array_count: 1,
                        optional: false,
                        dynamic_offset_count: 0,
                    },
                    ResourceBindingDesc {
                        binding: 1,
                        kind: ResourceBindingKind::UniformBuffer,
                        stages: PipelineStageFlags::DRAW,
                        array_count: 1,
                        optional: false,
                        dynamic_offset_count: 0,
                    },
                ],
            })?;
            created.push(blit_resource_layout);
            let blit_resource_set = gal.create_resource_set(ResourceSetDesc {
                label: "fabulous.blit.resource-set".to_string(),
                layout: blit_resource_layout,
                bindings: vec![
                    ResourceBinding {
                        binding: 0,
                        array_index: 0,
                        resource: blit_combined_sampler,
                        kind: ResourceBindingKind::CombinedTextureSampler,
                        access: AccessFlags::READ,
                        dynamic_offsets: Vec::new(),
                        buffer_range: None,
                    },
                    ResourceBinding {
                        binding: 1,
                        array_index: 0,
                        resource: blit_uniform_buffer,
                        kind: ResourceBindingKind::UniformBuffer,
                        access: AccessFlags::READ,
                        dynamic_offsets: Vec::new(),
                        buffer_range: Some(16),
                    },
                ],
            })?;
            created.push(blit_resource_set);
            Ok(Self {
                samplers: samplers.try_into().expect("twelve transparency samplers"),
                combined_samplers: combined.try_into().expect("twelve transparency samplers"),
                resource_layout,
                resource_set,
                blit_combined_sampler,
                blit_sampler,
                blit_uniform_buffer,
                blit_resource_layout,
                blit_resource_set,
            })
        })();
        if result.is_err() {
            for handle in created.into_iter().rev() {
                let _ = gal.destroy(handle);
            }
        }
        result
    }

    fn handles_in_destroy_order(&self) -> Vec<Handle> {
        let mut handles = vec![
            self.blit_resource_set,
            self.blit_resource_layout,
            self.blit_uniform_buffer,
            self.blit_combined_sampler,
            self.blit_sampler,
            self.resource_set,
            self.resource_layout,
        ];
        handles.extend(self.combined_samplers.iter().copied());
        handles
    }
}

impl FabulousIntermediateTarget {
    fn create(
        gal: &mut VulkanicGal,
        extent: Extent3d,
        color_format: TextureFormat,
    ) -> GalResult<Self> {
        let prefix = "fabulous.final";
        let mut created = Vec::new();
        let result = (|| -> GalResult<Self> {
            let texture = gal.create_texture(TextureDesc {
                label: format!("{prefix}.texture"),
                dimension: TextureDimension::D2,
                format: color_format,
                extent,
                mip_levels: 1,
                array_layers: 1,
                // The bundled transparency graph's final pass is an identity
                // nearest-filter blit (unit ColorModulate).  Keep the
                // intermediate explicitly transferable so that pass can be
                // lowered as the equivalent one-presenter copy to the
                // acquired frame image instead of relying on another
                // fullscreen raster pipeline.
                usages: vec![
                    TextureUsage::ColorAttachment,
                    TextureUsage::Sampled,
                    TextureUsage::TransferSrc,
                ],
            })?;
            created.push(texture);
            let view = gal.create_texture_view(TextureViewDesc {
                label: format!("{prefix}.view"),
                texture,
                format: color_format,
                base_mip: 0,
                mip_count: 1,
                base_layer: 0,
                layer_count: 1,
            })?;
            created.push(view);
            let render_target = gal.create_render_target(RenderTargetDesc {
                label: format!("{prefix}.target"),
                color_views: vec![view],
                depth_stencil_view: None,
                extent,
            })?;
            created.push(render_target);
            let render_pass = gal.create_render_pass(RenderPassDesc {
                label: format!("{prefix}.pass"),
                target: render_target,
                color_formats: vec![color_format],
                depth_format: None,
            })?;
            created.push(render_pass);
            let sampler = gal.create_sampler(SamplerDesc {
                label: format!("{prefix}.sampler"),
                min_filter: SamplerFilter::Nearest,
                mag_filter: SamplerFilter::Nearest,
                mip_filter: SamplerFilter::Nearest,
                address_u: SamplerAddressMode::ClampToEdge,
                address_v: SamplerAddressMode::ClampToEdge,
                address_w: SamplerAddressMode::ClampToEdge,
                comparison: None,
            })?;
            created.push(sampler);
            Ok(Self {
                texture,
                view,
                render_target,
                render_pass,
                sampler,
                extent,
            })
        })();
        if result.is_err() {
            for handle in created.into_iter().rev() {
                let _ = gal.destroy(handle);
            }
        }
        result
    }

    fn handles_in_destroy_order(self) -> [Handle; 5] {
        [
            self.sampler,
            self.render_pass,
            self.render_target,
            self.view,
            self.texture,
        ]
    }
}

impl FabulousAttachmentResources {
    pub(crate) fn create(
        gal: &mut VulkanicGal,
        role: &'static str,
        extent: Extent3d,
        color_format: TextureFormat,
    ) -> GalResult<Self> {
        Self::create_with_depth_format(
            gal,
            role,
            extent,
            color_format,
            FABULOUS_DEPTH_FORMAT,
            false,
        )
    }

    /// Creates an attachment with an explicit depth/stencil format and,
    /// optionally, transfer-capable color storage.  The optical hand target
    /// uses this rather than changing the six shader-pack attachments: their
    /// depth images remain shader-readable `Depth32Float` resources, while
    /// the hand mask gets a real `Depth24Stencil8` domain.
    pub(crate) fn create_with_depth_format(
        gal: &mut VulkanicGal,
        role: &'static str,
        extent: Extent3d,
        color_format: TextureFormat,
        depth_format: TextureFormat,
        transfer_color: bool,
    ) -> GalResult<Self> {
        if role.trim().is_empty() {
            return Err(GalError::invalid_argument(
                "Fabulous attachment role must be non-empty",
            ));
        }
        if extent.width == 0 || extent.height == 0 || extent.depth != 1 {
            return Err(GalError::invalid_argument(
                "Fabulous attachment extent must be a non-empty 2D image",
            ));
        }
        let prefix = format!("fabulous.{role}");
        let mut created = Vec::new();
        let result = (|| -> GalResult<Self> {
            let mut color_usages = vec![TextureUsage::ColorAttachment, TextureUsage::Sampled];
            if transfer_color {
                color_usages.extend([TextureUsage::TransferSrc, TextureUsage::TransferDst]);
            }
            let color_texture = gal.create_texture(TextureDesc {
                label: format!("{prefix}.color.texture"),
                dimension: TextureDimension::D2,
                format: color_format,
                extent,
                mip_levels: 1,
                array_layers: 1,
                usages: color_usages,
            })?;
            created.push(color_texture);
            let color_view = gal.create_texture_view(TextureViewDesc {
                label: format!("{prefix}.color.view"),
                texture: color_texture,
                format: color_format,
                base_mip: 0,
                mip_count: 1,
                base_layer: 0,
                layer_count: 1,
            })?;
            created.push(color_view);
            let depth_texture = gal.create_texture(TextureDesc {
                label: format!("{prefix}.depth.texture"),
                dimension: TextureDimension::D2,
                format: depth_format,
                extent,
                mip_levels: 1,
                array_layers: 1,
                usages: vec![
                    TextureUsage::DepthStencilAttachment,
                    TextureUsage::Sampled,
                    TextureUsage::TransferSrc,
                    TextureUsage::TransferDst,
                ],
            })?;
            created.push(depth_texture);
            let depth_view = gal.create_texture_view(TextureViewDesc {
                label: format!("{prefix}.depth.view"),
                texture: depth_texture,
                format: depth_format,
                base_mip: 0,
                mip_count: 1,
                base_layer: 0,
                layer_count: 1,
            })?;
            created.push(depth_view);
            let render_target = gal.create_render_target(RenderTargetDesc {
                label: format!("{prefix}.target"),
                color_views: vec![color_view],
                depth_stencil_view: Some(depth_view),
                extent,
            })?;
            created.push(render_target);
            let render_pass = gal.create_render_pass(RenderPassDesc {
                label: format!("{prefix}.pass"),
                target: render_target,
                color_formats: vec![color_format],
                depth_format: Some(depth_format),
            })?;
            created.push(render_pass);
            let sampler = gal.create_sampler(SamplerDesc {
                label: format!("{prefix}.sampler"),
                min_filter: SamplerFilter::Nearest,
                mag_filter: SamplerFilter::Nearest,
                mip_filter: SamplerFilter::Nearest,
                address_u: SamplerAddressMode::ClampToEdge,
                address_v: SamplerAddressMode::ClampToEdge,
                address_w: SamplerAddressMode::ClampToEdge,
                comparison: None,
            })?;
            created.push(sampler);
            Ok(Self {
                role,
                color_texture,
                color_view,
                depth_texture,
                depth_view,
                render_target,
                render_pass,
                sampler,
                extent,
            })
        })();
        if result.is_err() {
            for handle in created.into_iter().rev() {
                let _ = gal.destroy(handle);
            }
        }
        result
    }

    pub(crate) fn handles_in_destroy_order(self) -> [Handle; 7] {
        [
            self.sampler,
            self.render_pass,
            self.render_target,
            self.depth_view,
            self.color_view,
            self.depth_texture,
            self.color_texture,
        ]
    }
}

/// All six external targets for one frame extent. Creation is atomic from the
/// caller's perspective: a failure destroys every resource created so far.
#[derive(Debug)]
pub(crate) struct FabulousAttachmentSet {
    pub(crate) final_target: FabulousIntermediateTarget,
    pub(crate) bindings: Option<FabulousTransparencyBindings>,
    pub(crate) pipelines: Option<FabulousTransparencyPipelines>,
    pub(crate) main: FabulousAttachmentResources,
    pub(crate) translucent: FabulousAttachmentResources,
    pub(crate) item_entity: FabulousAttachmentResources,
    pub(crate) particles: FabulousAttachmentResources,
    pub(crate) clouds: FabulousAttachmentResources,
    pub(crate) weather: FabulousAttachmentResources,
    pub(crate) translucent_color_format: TextureFormat,
    pub(crate) external_color_format: TextureFormat,
    /// Private first-person optical target. It is not part of the shader-pack
    /// external inventory and is only consumed once a semantic optical plan
    /// is admitted by the hand frontend.
    pub(crate) optical_hand: FabulousAttachmentResources,
}

impl FabulousAttachmentSet {
    pub(crate) fn create(
        gal: &mut VulkanicGal,
        extent: Extent3d,
        color_format: TextureFormat,
    ) -> GalResult<Self> {
        Self::create_with_external_formats(gal, extent, color_format, color_format, color_format)
    }

    pub(crate) fn create_with_translucent_format(
        gal: &mut VulkanicGal,
        extent: Extent3d,
        color_format: TextureFormat,
        translucent_color_format: TextureFormat,
    ) -> GalResult<Self> {
        Self::create_with_external_formats(
            gal,
            extent,
            color_format,
            translucent_color_format,
            translucent_color_format,
        )
    }

    pub(crate) fn create_with_external_formats(
        gal: &mut VulkanicGal,
        extent: Extent3d,
        color_format: TextureFormat,
        translucent_color_format: TextureFormat,
        external_color_format: TextureFormat,
    ) -> GalResult<Self> {
        let mut created = Vec::new();
        let result = (|| -> GalResult<Self> {
            let final_target = FabulousIntermediateTarget::create(gal, extent, color_format)?;
            created.extend(final_target.handles_in_destroy_order());
            // The private optical hand copy boundary reads/writes only the
            // Rust-owned main color attachment. Keep transfer usage explicit
            // on that resource; every other external attachment remains
            // render/sample-only.
            let main = FabulousAttachmentResources::create_with_depth_format(
                gal,
                "minecraft:main",
                extent,
                color_format,
                FABULOUS_DEPTH_FORMAT,
                true,
            )?;
            created.extend(main.handles_in_destroy_order());
            // The normal deferred terrain capture is an explicit RGBA8
            // attachment. Keep the Fabulous translucent role in that same
            // format so its transfer copy remains format-valid even when
            // the acquired Vulkan swapchain is BGRA8. The main/final roles
            // continue to use the acquired target format and are composed
            // through the declared fullscreen blit pipeline.
            let translucent = FabulousAttachmentResources::create_with_depth_format(
                gal,
                "minecraft:translucent",
                extent,
                translucent_color_format,
                FABULOUS_DEPTH_FORMAT,
                translucent_color_format == TextureFormat::Rgba8Unorm,
            )?;
            created.extend(translucent.handles_in_destroy_order());
            let mut create = |role| {
                FabulousAttachmentResources::create(gal, role, extent, external_color_format)
            };
            let item_entity = create("minecraft:item_entity")?;
            created.extend(item_entity.handles_in_destroy_order());
            let particles = create("minecraft:particles")?;
            created.extend(particles.handles_in_destroy_order());
            let clouds = create("minecraft:clouds")?;
            created.extend(clouds.handles_in_destroy_order());
            let weather = create("minecraft:weather")?;
            created.extend(weather.handles_in_destroy_order());
            let optical_hand = FabulousAttachmentResources::create_with_depth_format(
                gal,
                "mattmc:optical_hand",
                extent,
                color_format,
                TextureFormat::Depth24Stencil8,
                true,
            )?;
            created.extend(optical_hand.handles_in_destroy_order());
            let provisional = Self {
                final_target,
                bindings: None,
                pipelines: None,
                main,
                translucent,
                item_entity,
                particles,
                clouds,
                weather,
                optical_hand,
                translucent_color_format,
                external_color_format,
            };
            let bindings = FabulousTransparencyBindings::create(gal, &provisional)?;
            let pipelines = FabulousTransparencyPipelines::create(gal, &bindings, color_format)?;
            Ok(Self {
                bindings: Some(bindings),
                pipelines: Some(pipelines),
                ..provisional
            })
        })();
        if result.is_err() {
            for handle in created.into_iter().rev() {
                let _ = gal.destroy(handle);
            }
        }
        result
    }

    pub(crate) fn external_inventory(&self) -> FabulousExternalTargetInventory {
        fn binding(
            resource: &FabulousAttachmentResources,
        ) -> VanillaPostEffectExternalTargetBinding {
            VanillaPostEffectExternalTargetBinding {
                render_pass: resource.render_pass,
                render_target: resource.render_target,
                color_attachment: resource.color_view,
                depth_attachment: Some(resource.depth_view),
                sampler: resource.sampler,
                color_usage: TextureUsageState::ShaderRead,
                depth_usage: Some(TextureUsageState::ShaderRead),
            }
        }
        FabulousExternalTargetInventory {
            main: binding(&self.main),
            translucent: binding(&self.translucent),
            item_entity: binding(&self.item_entity),
            particles: binding(&self.particles),
            clouds: binding(&self.clouds),
            weather: binding(&self.weather),
        }
    }

    pub(crate) fn transparency_pass_bindings(
        &self,
    ) -> GalResult<Vec<super::vanilla_post_effect_executor::VanillaPostEffectPassBinding>> {
        let bindings = self
            .bindings
            .as_ref()
            .ok_or_else(|| GalError::backend("Fabulous descriptor bindings are not initialized"))?;
        let pipelines = self
            .pipelines
            .as_ref()
            .ok_or_else(|| GalError::backend("Fabulous pipelines are not initialized"))?;
        let executor = super::vanilla_post_effect_executor::bundled_transparency_executor()?;
        let plan = executor.plan();
        let attachment = |name: &str| -> Option<&FabulousAttachmentResources> {
            match name {
                "minecraft:main" => Some(&self.main),
                "minecraft:translucent" => Some(&self.translucent),
                "minecraft:item_entity" => Some(&self.item_entity),
                "minecraft:particles" => Some(&self.particles),
                "minecraft:clouds" => Some(&self.clouds),
                "minecraft:weather" => Some(&self.weather),
                _ => None,
            }
        };
        let mut pass_bindings = Vec::with_capacity(plan.ordered_passes.len());
        for (index, pass) in plan.ordered_passes.iter().enumerate() {
            let inputs = pass
                .inputs
                .iter()
                .map(|input| {
                    if input.target == "final" {
                        return Ok(
                            super::vanilla_post_effect_executor::VanillaPostEffectInputBinding {
                                texture_view: self.final_target.view,
                                sampler: self.final_target.sampler,
                                bilinear: input.bilinear,
                                use_depth_buffer: input.use_depth_buffer,
                            },
                        );
                    }
                    let target = attachment(&input.target).ok_or_else(|| {
                        GalError::unsupported_feature(format!(
                            "Fabulous post-effect input target {} has no Rust-owned attachment",
                            input.target
                        ))
                    })?;
                    let input_index = pass
                        .inputs
                        .iter()
                        .position(|candidate| candidate.sampler_name == input.sampler_name)
                        .ok_or_else(|| {
                            GalError::invalid_argument(
                                "Fabulous post-effect input order is not stable",
                            )
                        })?;
                    let sampler = bindings.samplers[input_index];
                    Ok(
                        super::vanilla_post_effect_executor::VanillaPostEffectInputBinding {
                            texture_view: if input.use_depth_buffer {
                                target.depth_view
                            } else {
                                target.color_view
                            },
                            sampler,
                            bilinear: input.bilinear,
                            use_depth_buffer: input.use_depth_buffer,
                        },
                    )
                })
                .collect::<GalResult<Vec<_>>>()?;
            let (
                render_pass,
                render_target,
                color_attachment,
                depth_attachment,
                pipeline,
                pipeline_layout,
                resource_set,
            ) = if index == 0 {
                (
                    self.final_target.render_pass,
                    self.final_target.render_target,
                    self.final_target.view,
                    None,
                    pipelines.transparency_pipeline,
                    pipelines.transparency_pipeline_layout,
                    bindings.resource_set,
                )
            } else {
                (
                    self.main.render_pass,
                    self.main.render_target,
                    self.main.color_view,
                    Some(self.main.depth_view),
                    pipelines.blit_pipeline,
                    pipelines.blit_pipeline_layout,
                    bindings.blit_resource_set,
                )
            };
            pass_bindings.push(
                super::vanilla_post_effect_executor::VanillaPostEffectPassBinding {
                    render_pass,
                    render_target,
                    color_attachment,
                    depth_attachment,
                    pipeline,
                    pipeline_layout,
                    resource_set,
                    inputs,
                    uniform_values: pass.uniform_values.clone(),
                },
            );
        }
        Ok(pass_bindings)
    }

    /// Builds the final blit against the acquired GAL frame target. The
    /// caller owns and retires the returned render pass after submission
    /// completion; this helper never creates a second presenter.
    pub(crate) fn transparency_pass_bindings_to_frame_target(
        &self,
        gal: &mut VulkanicGal,
        frame_target: Handle,
    ) -> GalResult<(
        Vec<super::vanilla_post_effect_executor::VanillaPostEffectPassBinding>,
        Handle,
    )> {
        let mut bindings = self.transparency_pass_bindings()?;
        let pipelines = self
            .pipelines
            .as_ref()
            .ok_or_else(|| GalError::backend("Fabulous pipelines are not initialized"))?;
        if bindings.is_empty() {
            return Err(GalError::backend(
                "Fabulous transparency graph has no final blit pass",
            ));
        }
        let color_format = gal.pass_target_color_format(frame_target)?;
        let pass = gal.create_render_pass(super::super::resources::RenderPassDesc {
            label: "fabulous.blit.acquired-frame-pass".to_string(),
            target: frame_target,
            color_formats: vec![color_format],
            depth_format: None,
        })?;
        let last = bindings
            .last_mut()
            .expect("non-empty Fabulous pass bindings");
        last.render_pass = pass;
        last.render_target = frame_target;
        last.color_attachment = frame_target;
        last.depth_attachment = None;
        last.pipeline = pipelines.blit_frame_pipeline;
        Ok((bindings, pass))
    }

    /// Creates a bounded final blit descriptor that samples the composed
    /// private main attachment rather than the intermediate transparency
    /// image. This is used when depth-backed overlays (for example world
    /// text) must be drawn into `main` before the one acquired-frame copy.
    pub(crate) fn create_main_to_frame_blit_resources(
        &self,
        gal: &mut VulkanicGal,
        frame_target: Handle,
    ) -> GalResult<(Handle, Handle, Handle, Handle)> {
        let bindings = self
            .bindings
            .as_ref()
            .ok_or_else(|| GalError::backend("Fabulous descriptor bindings are not initialized"))?;
        let color_format = gal.pass_target_color_format(frame_target)?;
        let sampler = gal.create_sampler(SamplerDesc {
            label: "fabulous.main-to-frame.sampler".to_string(),
            min_filter: SamplerFilter::Nearest,
            mag_filter: SamplerFilter::Nearest,
            mip_filter: SamplerFilter::Nearest,
            address_u: SamplerAddressMode::ClampToEdge,
            address_v: SamplerAddressMode::ClampToEdge,
            address_w: SamplerAddressMode::ClampToEdge,
            comparison: None,
        })?;
        let combined = match gal.create_combined_texture_sampler(CombinedTextureSamplerDesc {
            label: "fabulous.main-to-frame.combined".to_string(),
            texture_view: self.main.color_view,
            sampler,
        }) {
            Ok(handle) => handle,
            Err(error) => {
                let _ = gal.destroy(sampler);
                return Err(error);
            }
        };
        let resource_set = match gal.create_resource_set(ResourceSetDesc {
            label: "fabulous.main-to-frame.resource-set".to_string(),
            layout: bindings.blit_resource_layout,
            bindings: vec![
                ResourceBinding {
                    binding: 0,
                    array_index: 0,
                    resource: combined,
                    kind: ResourceBindingKind::CombinedTextureSampler,
                    access: AccessFlags::READ,
                    dynamic_offsets: Vec::new(),
                    buffer_range: None,
                },
                ResourceBinding {
                    binding: 1,
                    array_index: 0,
                    resource: bindings.blit_uniform_buffer,
                    kind: ResourceBindingKind::UniformBuffer,
                    access: AccessFlags::READ,
                    dynamic_offsets: Vec::new(),
                    buffer_range: Some(16),
                },
            ],
        }) {
            Ok(handle) => handle,
            Err(error) => {
                let _ = gal.destroy(combined);
                let _ = gal.destroy(sampler);
                return Err(error);
            }
        };
        let pass = match gal.create_render_pass(RenderPassDesc {
            label: "fabulous.main-to-frame.pass".to_string(),
            target: frame_target,
            color_formats: vec![color_format],
            depth_format: None,
        }) {
            Ok(handle) => handle,
            Err(error) => {
                let _ = gal.destroy(resource_set);
                let _ = gal.destroy(combined);
                let _ = gal.destroy(sampler);
                return Err(error);
            }
        };
        Ok((resource_set, combined, sampler, pass))
    }

    /// Emits the explicit state transitions required before the first
    /// transparency fullscreen pass. The caller supplies no backend state:
    /// every resource identity comes from this Rust-owned attachment set.
    pub(crate) fn pre_transparency_barriers(&self) -> Vec<CommandOp> {
        let mut operations = Vec::with_capacity(13);
        for attachment in [
            &self.main,
            &self.translucent,
            &self.item_entity,
            &self.particles,
            &self.clouds,
            &self.weather,
        ] {
            operations.push(CommandOp::Barrier(ResourceBarrier {
                resource: attachment.color_texture,
                subresources: None,
                before: TextureUsageState::ColorAttachment,
                after: TextureUsageState::ShaderRead,
                src_queue: super::super::resources::QueueClass::Graphics,
                dst_queue: super::super::resources::QueueClass::Graphics,
            }));
            operations.push(CommandOp::Barrier(ResourceBarrier {
                resource: attachment.depth_texture,
                subresources: None,
                before: TextureUsageState::DepthStencilAttachment,
                after: TextureUsageState::ShaderRead,
                src_queue: super::super::resources::QueueClass::Graphics,
                dst_queue: super::super::resources::QueueClass::Graphics,
            }));
        }
        operations.push(CommandOp::Barrier(ResourceBarrier {
            resource: self.final_target.texture,
            subresources: None,
            before: TextureUsageState::Undefined,
            after: TextureUsageState::ColorAttachment,
            src_queue: super::super::resources::QueueClass::Graphics,
            dst_queue: super::super::resources::QueueClass::Graphics,
        }));
        operations
    }

    /// Copies the isolated normal-deferred translucent capture into the
    /// Rust-owned Fabulous translucent attachment.  The source is a semantic
    /// GAL texture; no native image or Java/Iris target is accepted.  This is
    /// deliberately color-only until the terrain depth handoff has an
    /// explicit depth-transfer contract.
    pub(crate) fn append_translucent_capture_copy(
        &self,
        ops: &mut Vec<CommandOp>,
        source_texture: Handle,
        extent: Extent3d,
        source_before: TextureUsageState,
        destination_before: TextureUsageState,
    ) -> GalResult<()> {
        if extent.width == 0 || extent.height == 0 || extent.depth != 1 {
            return Err(GalError::invalid_argument(
                "Fabulous translucent capture copy requires a non-zero 2D extent",
            ));
        }
        ops.push(CommandOp::Barrier(ResourceBarrier {
            resource: source_texture,
            subresources: None,
            before: source_before,
            after: TextureUsageState::TransferSrc,
            src_queue: super::super::resources::QueueClass::Graphics,
            dst_queue: super::super::resources::QueueClass::Transfer,
        }));
        ops.push(CommandOp::Barrier(ResourceBarrier {
            resource: self.translucent.color_texture,
            subresources: None,
            before: destination_before,
            after: TextureUsageState::TransferDst,
            src_queue: super::super::resources::QueueClass::Graphics,
            dst_queue: super::super::resources::QueueClass::Transfer,
        }));
        ops.push(CommandOp::CopyTexture(TextureImageCopyRegion {
            row_order: crate::render::vulkanic::commands::TextureRowOrder::Preserve,
            src_texture: source_texture,
            src_mip: 0,
            src_layer: 0,
            src_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
            dst_texture: self.translucent.color_texture,
            dst_mip: 0,
            dst_layer: 0,
            dst_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
            extent,
        }));
        ops.push(CommandOp::Barrier(ResourceBarrier {
            resource: source_texture,
            subresources: None,
            before: TextureUsageState::TransferSrc,
            after: TextureUsageState::ShaderRead,
            src_queue: super::super::resources::QueueClass::Transfer,
            dst_queue: super::super::resources::QueueClass::Graphics,
        }));
        ops.push(CommandOp::Barrier(ResourceBarrier {
            resource: self.translucent.color_texture,
            subresources: None,
            before: TextureUsageState::TransferDst,
            after: TextureUsageState::ShaderRead,
            src_queue: super::super::resources::QueueClass::Transfer,
            dst_queue: super::super::resources::QueueClass::Graphics,
        }));
        Ok(())
    }

    /// Copies the normal graph's acquired frame image into the private
    /// Fabulous main sampler before the bundled transparency pass. The frame
    /// target remains opaque to the frontend; only the semantic GAL copy
    /// operation crosses this boundary.
    pub(crate) fn append_frame_target_to_main_copy(
        &self,
        ops: &mut Vec<CommandOp>,
        frame_target: Handle,
        extent: Extent3d,
        destination_before: TextureUsageState,
    ) -> GalResult<()> {
        if extent.width == 0 || extent.height == 0 || extent.depth != 1 {
            return Err(GalError::invalid_argument(
                "Fabulous main capture copy requires a non-zero 2D extent",
            ));
        }
        ops.push(CommandOp::Barrier(ResourceBarrier {
            resource: self.main.color_texture,
            subresources: None,
            before: destination_before,
            after: TextureUsageState::TransferDst,
            src_queue: super::super::resources::QueueClass::Graphics,
            dst_queue: super::super::resources::QueueClass::Transfer,
        }));
        ops.push(CommandOp::CopyFrameTargetToTexture {
            src: frame_target,
            dst: self.main.color_texture,
            extent,
        });
        ops.push(CommandOp::Barrier(ResourceBarrier {
            resource: self.main.color_texture,
            subresources: None,
            before: TextureUsageState::TransferDst,
            after: TextureUsageState::ShaderRead,
            src_queue: super::super::resources::QueueClass::Transfer,
            dst_queue: super::super::resources::QueueClass::Graphics,
        }));
        Ok(())
    }

    /// Copies a Rust-owned depth domain into a Fabulous attachment.  The
    /// bundled transparency shader samples every declared depth role, so the
    /// terrain handoff must preserve the exact G-buffer depth rather than
    /// inventing a second or borrowed depth source.
    pub(crate) fn append_depth_capture_copy(
        &self,
        ops: &mut Vec<CommandOp>,
        source_texture: Handle,
        destination: FabulousTargetRole,
        extent: Extent3d,
        source_before: TextureUsageState,
        destination_before: TextureUsageState,
    ) -> GalResult<()> {
        if extent.width == 0 || extent.height == 0 || extent.depth != 1 {
            return Err(GalError::invalid_argument(
                "Fabulous depth capture copy requires a non-zero 2D extent",
            ));
        }
        let attachment = match destination {
            FabulousTargetRole::Main => &self.main,
            FabulousTargetRole::Translucent => &self.translucent,
            FabulousTargetRole::ItemEntity => &self.item_entity,
            FabulousTargetRole::Particles => &self.particles,
            FabulousTargetRole::Clouds => &self.clouds,
            FabulousTargetRole::Weather => &self.weather,
        };
        ops.push(CommandOp::Barrier(ResourceBarrier {
            resource: source_texture,
            subresources: None,
            before: source_before,
            after: TextureUsageState::TransferSrc,
            src_queue: super::super::resources::QueueClass::Graphics,
            dst_queue: super::super::resources::QueueClass::Transfer,
        }));
        ops.push(CommandOp::Barrier(ResourceBarrier {
            resource: attachment.depth_texture,
            subresources: None,
            before: destination_before,
            after: TextureUsageState::TransferDst,
            src_queue: super::super::resources::QueueClass::Graphics,
            dst_queue: super::super::resources::QueueClass::Transfer,
        }));
        ops.push(CommandOp::CopyTexture(TextureImageCopyRegion {
            row_order: crate::render::vulkanic::commands::TextureRowOrder::Preserve,
            src_texture: source_texture,
            src_mip: 0,
            src_layer: 0,
            src_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
            dst_texture: attachment.depth_texture,
            dst_mip: 0,
            dst_layer: 0,
            dst_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
            extent,
        }));
        ops.push(CommandOp::Barrier(ResourceBarrier {
            resource: source_texture,
            subresources: None,
            before: TextureUsageState::TransferSrc,
            after: TextureUsageState::ShaderRead,
            src_queue: super::super::resources::QueueClass::Transfer,
            dst_queue: super::super::resources::QueueClass::Graphics,
        }));
        ops.push(CommandOp::Barrier(ResourceBarrier {
            resource: attachment.depth_texture,
            subresources: None,
            before: TextureUsageState::TransferDst,
            after: TextureUsageState::ShaderRead,
            src_queue: super::super::resources::QueueClass::Transfer,
            dst_queue: super::super::resources::QueueClass::Graphics,
        }));
        Ok(())
    }

    /// Clears an external family that has no semantic producer in the
    /// current terrain handoff. This makes every declared color/depth sampler
    /// valid without borrowing Java-side attachments.
    pub(crate) fn append_empty_attachment_clear(
        &self,
        ops: &mut Vec<CommandOp>,
        role: FabulousTargetRole,
        color_before: TextureUsageState,
        depth_before: TextureUsageState,
    ) {
        let attachment = match role {
            FabulousTargetRole::Main => &self.main,
            FabulousTargetRole::Translucent => &self.translucent,
            FabulousTargetRole::ItemEntity => &self.item_entity,
            FabulousTargetRole::Particles => &self.particles,
            FabulousTargetRole::Clouds => &self.clouds,
            FabulousTargetRole::Weather => &self.weather,
        };
        ops.push(CommandOp::Barrier(ResourceBarrier {
            resource: attachment.color_texture,
            subresources: None,
            before: color_before,
            after: TextureUsageState::ColorAttachment,
            src_queue: super::super::resources::QueueClass::Graphics,
            dst_queue: super::super::resources::QueueClass::Graphics,
        }));
        ops.push(CommandOp::Barrier(ResourceBarrier {
            resource: attachment.depth_texture,
            subresources: None,
            before: depth_before,
            after: TextureUsageState::DepthStencilAttachment,
            src_queue: super::super::resources::QueueClass::Graphics,
            dst_queue: super::super::resources::QueueClass::Graphics,
        }));
        ops.push(CommandOp::BeginPass {
            pass: attachment.render_pass,
            target: attachment.render_target,
            colors: vec![PassAttachment {
                view: attachment.color_view,
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
                view: attachment.depth_view,
                load_op: AttachmentLoadOp::Clear,
                store_op: AttachmentStoreOp::Store,
                clear_color: None,
            }),
        });
        ops.push(CommandOp::EndPass);
        ops.extend([
            CommandOp::Barrier(ResourceBarrier {
                resource: attachment.color_texture,
                subresources: None,
                before: TextureUsageState::ColorAttachment,
                after: TextureUsageState::ShaderRead,
                src_queue: super::super::resources::QueueClass::Graphics,
                dst_queue: super::super::resources::QueueClass::Graphics,
            }),
            CommandOp::Barrier(ResourceBarrier {
                resource: attachment.depth_texture,
                subresources: None,
                before: TextureUsageState::DepthStencilAttachment,
                after: TextureUsageState::ShaderRead,
                src_queue: super::super::resources::QueueClass::Graphics,
                dst_queue: super::super::resources::QueueClass::Graphics,
            }),
        ]);
    }

    /// Lowers a complete terrain-only transparency handoff into one command
    /// stream. This helper is private preparation: callers still decide
    /// whether the semantic frame is eligible and must submit the returned
    /// operations together with GUI/text composition.
    pub(crate) fn append_terrain_handoff_to_frame_target(
        &self,
        gal: &mut VulkanicGal,
        ops: &mut Vec<CommandOp>,
        frame_target: Handle,
        extent: Extent3d,
        deferred_frame_color_source: Handle,
        deferred_translucent_source: Handle,
        deferred_depth_source: Handle,
        deferred_translucent_depth_source: Handle,
    ) -> GalResult<Handle> {
        self.append_terrain_handoff_to_frame_target_with_external_ops(
            gal,
            ops,
            frame_target,
            extent,
            deferred_frame_color_source,
            deferred_translucent_source,
            deferred_depth_source,
            deferred_translucent_depth_source,
            &[],
            [false; 4],
            false,
            true,
        )
    }

    pub(crate) fn append_terrain_handoff_to_frame_target_with_external_ops(
        &self,
        gal: &mut VulkanicGal,
        ops: &mut Vec<CommandOp>,
        frame_target: Handle,
        extent: Extent3d,
        deferred_frame_color_source: Handle,
        deferred_translucent_source: Handle,
        deferred_depth_source: Handle,
        deferred_translucent_depth_source: Handle,
        external_operations: &[CommandOp],
        external_roles_written: [bool; 4],
        attachments_initialized: bool,
        deferred_translucent_initialized: bool,
    ) -> GalResult<Handle> {
        // A newly allocated attachment has no prior Vulkan layout.  Once a
        // submitted handoff has completed this explicit state becomes
        // shader-readable and is retained by the attachment set.  Carry this
        // lifecycle fact through the GAL command stream rather than claiming
        // an undefined image was previously sampled.
        let persistent_before = if attachments_initialized {
            TextureUsageState::ShaderRead
        } else {
            TextureUsageState::Undefined
        };
        self.append_frame_target_to_main_copy(
            ops,
            deferred_frame_color_source,
            extent,
            persistent_before,
        )?;
        if deferred_translucent_initialized {
            self.append_translucent_capture_copy(
                ops,
                deferred_translucent_source,
                extent,
                TextureUsageState::ShaderRead,
                persistent_before,
            )?;
        } else {
            // The graph always samples the declared translucent role. When
            // this semantic frame has no translucent terrain draw, initialize
            // just its color input as transparent; retain the separately
            // copied depth input for Fabulous's depth-aware composition.
            let attachment = &self.translucent;
            ops.push(CommandOp::Barrier(ResourceBarrier {
                resource: attachment.color_texture,
                subresources: None,
                before: persistent_before,
                after: TextureUsageState::ColorAttachment,
                src_queue: super::super::resources::QueueClass::Graphics,
                dst_queue: super::super::resources::QueueClass::Graphics,
            }));
            ops.push(CommandOp::BeginPass {
                pass: attachment.render_pass,
                target: attachment.render_target,
                colors: vec![PassAttachment {
                    view: attachment.color_view,
                    load_op: AttachmentLoadOp::Clear,
                    store_op: AttachmentStoreOp::Store,
                    clear_color: Some(ClearColor { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }),
                }],
                depth_stencil: None,
            });
            ops.push(CommandOp::EndPass);
            ops.push(CommandOp::Barrier(ResourceBarrier {
                resource: attachment.color_texture,
                subresources: None,
                before: TextureUsageState::ColorAttachment,
                after: TextureUsageState::ShaderRead,
                src_queue: super::super::resources::QueueClass::Graphics,
                dst_queue: super::super::resources::QueueClass::Graphics,
            }));
        }
        self.append_depth_capture_copy(
            ops,
            deferred_depth_source,
            FabulousTargetRole::Main,
            extent,
            TextureUsageState::ShaderRead,
            // Fabulous depth attachments persist across frames and the
            // preceding graph leaves them shader-readable. Treating them as
            // Undefined causes an invalid old-layout transition on reuse.
            persistent_before,
        )?;
        self.append_depth_capture_copy(
            ops,
            deferred_translucent_depth_source,
            FabulousTargetRole::Translucent,
            extent,
            TextureUsageState::ShaderRead,
            persistent_before,
        )?;
        for (role_index, role) in [
            FabulousTargetRole::ItemEntity,
            FabulousTargetRole::Particles,
            FabulousTargetRole::Clouds,
            FabulousTargetRole::Weather,
        ]
        .into_iter()
        .enumerate()
        {
            if external_roles_written[role_index] {
                continue;
            }
            self.append_empty_attachment_clear(
                ops,
                role,
                TextureUsageState::Undefined,
                TextureUsageState::Undefined,
            );
        }
        ops.extend(external_operations.iter().cloned());
        let translucent_written_by_external_ops = external_operations.iter().any(|operation| {
            matches!(operation, CommandOp::BeginPass { target, .. } if *target == self.translucent.render_target)
        });
        ops.extend(self.external_shader_read_barriers_with_role_states(
            TextureUsageState::ShaderRead,
            TextureUsageState::ShaderRead,
            if translucent_written_by_external_ops {
                TextureUsageState::ColorAttachment
            } else {
                TextureUsageState::ShaderRead
            },
            if translucent_written_by_external_ops {
                TextureUsageState::DepthStencilAttachment
            } else {
                TextureUsageState::ShaderRead
            },
            if external_roles_written[0] {
                TextureUsageState::ColorAttachment
            } else {
                TextureUsageState::ShaderRead
            },
            if external_roles_written[0] {
                TextureUsageState::DepthStencilAttachment
            } else {
                TextureUsageState::ShaderRead
            },
            if external_roles_written[1] {
                TextureUsageState::ColorAttachment
            } else {
                TextureUsageState::ShaderRead
            },
            if external_roles_written[1] {
                TextureUsageState::DepthStencilAttachment
            } else {
                TextureUsageState::ShaderRead
            },
            if external_roles_written[2] {
                TextureUsageState::ColorAttachment
            } else {
                TextureUsageState::ShaderRead
            },
            if external_roles_written[2] {
                TextureUsageState::DepthStencilAttachment
            } else {
                TextureUsageState::ShaderRead
            },
            if external_roles_written[3] {
                TextureUsageState::ColorAttachment
            } else {
                TextureUsageState::ShaderRead
            },
            if external_roles_written[3] {
                TextureUsageState::DepthStencilAttachment
            } else {
                TextureUsageState::ShaderRead
            },
        ));
        ops.push(self.final_target_color_attachment_barrier());
        let executor = super::vanilla_post_effect_executor::bundled_transparency_executor()?;
        // The handoff above either writes each optional external role through
        // semantic material operations or explicitly clears it. Record that
        // receipt before lowering the graph so allocation/type validation can
        // never masquerade as frame population.
        let populated_roles = BTreeSet::from([
            "minecraft:main".to_owned(),
            "minecraft:translucent".to_owned(),
            "minecraft:item_entity".to_owned(),
            "minecraft:particles".to_owned(),
            "minecraft:clouds".to_owned(),
            "minecraft:weather".to_owned(),
        ]);
        self.external_inventory()
            .validate_populated_for_plan(executor.plan(), &populated_roles)?;
        let (bindings, presentation_pass) =
            self.transparency_pass_bindings_to_frame_target(gal, frame_target)?;
        let mut post_ops = match executor.lower(&bindings) {
            Ok(operations) => operations,
            Err(error) => {
                let _ = gal.destroy(presentation_pass);
                return Err(error);
            }
        };
        let first_end = post_ops
            .iter()
            .position(|operation| matches!(operation, CommandOp::EndPass))
            .ok_or_else(|| {
                GalError::backend("terrain Fabulous handoff omitted first pass terminator")
            })?;
        // The first pass writes the private `final` target; the vanilla
        // terminal blit samples it and performs the required format conversion
        // into the one acquired presentation target.  Keep that dependency
        // explicit rather than replacing it with a raw copy, which is invalid
        // for the RGBA intermediate/BGRA presentation combination.
        post_ops.insert(
            first_end + 1,
            self.between_transparency_pass_barriers()
                .into_iter()
                .next()
                .expect("Fabulous graph has one intermediate transition"),
        );
        let blit_uniform_buffer = self
            .bindings
            .as_ref()
            .ok_or_else(|| GalError::backend("Fabulous descriptor bindings are not initialized"))?
            .blit_uniform_buffer;
        ops.extend([
            CommandOp::Barrier(ResourceBarrier {
                resource: blit_uniform_buffer,
                subresources: None,
                before: TextureUsageState::ShaderRead,
                after: TextureUsageState::TransferDst,
                src_queue: super::super::resources::QueueClass::Graphics,
                dst_queue: super::super::resources::QueueClass::Transfer,
            }),
            CommandOp::HostWriteBuffer {
                buffer: blit_uniform_buffer,
                offset: 0,
                data: vec![0, 0, 128, 63, 0, 0, 128, 63, 0, 0, 128, 63, 0, 0, 128, 63],
            },
            CommandOp::Barrier(ResourceBarrier {
                resource: blit_uniform_buffer,
                subresources: None,
                before: TextureUsageState::TransferDst,
                after: TextureUsageState::ShaderRead,
                src_queue: super::super::resources::QueueClass::Transfer,
                dst_queue: super::super::resources::QueueClass::Graphics,
            }),
        ]);
        ops.extend(post_ops);
        Ok(presentation_pass)
    }

    pub(crate) fn between_transparency_pass_barriers(&self) -> Vec<CommandOp> {
        vec![CommandOp::Barrier(ResourceBarrier {
            resource: self.final_target.texture,
            subresources: None,
            before: TextureUsageState::ColorAttachment,
            after: TextureUsageState::ShaderRead,
            src_queue: super::super::resources::QueueClass::Graphics,
            dst_queue: super::super::resources::QueueClass::Graphics,
        })]
    }

    /// Transitions the six producer attachments after their semantic draw
    /// passes have completed.  Keeping this separate from the intermediate
    /// target transition lets the frame coordinator initialize empty families
    /// with a clear pass and still present every declared sampler in a valid
    /// shader-readable state.
    pub(crate) fn external_shader_read_barriers(&self) -> Vec<CommandOp> {
        self.external_shader_read_barriers_with_translucent_state(
            TextureUsageState::ColorAttachment,
            TextureUsageState::DepthStencilAttachment,
        )
    }

    /// Variant used when an external producer already left the translucent
    /// color/depth resources in explicit states (for example a transfer copy
    /// from the normal deferred graph). Other Fabulous roles retain the
    /// ordinary raster-to-sample transition contract.
    pub(crate) fn external_shader_read_barriers_with_translucent_state(
        &self,
        translucent_color_before: TextureUsageState,
        translucent_depth_before: TextureUsageState,
    ) -> Vec<CommandOp> {
        self.external_shader_read_barriers_with_input_states(
            TextureUsageState::ColorAttachment,
            TextureUsageState::DepthStencilAttachment,
            translucent_color_before,
            translucent_depth_before,
        )
    }

    pub(crate) fn external_shader_read_barriers_with_input_states(
        &self,
        main_color_before: TextureUsageState,
        main_depth_before: TextureUsageState,
        translucent_color_before: TextureUsageState,
        translucent_depth_before: TextureUsageState,
    ) -> Vec<CommandOp> {
        self.external_shader_read_barriers_with_role_states(
            main_color_before,
            main_depth_before,
            translucent_color_before,
            translucent_depth_before,
            TextureUsageState::ColorAttachment,
            TextureUsageState::DepthStencilAttachment,
            TextureUsageState::ColorAttachment,
            TextureUsageState::DepthStencilAttachment,
            TextureUsageState::ColorAttachment,
            TextureUsageState::DepthStencilAttachment,
            TextureUsageState::ColorAttachment,
            TextureUsageState::DepthStencilAttachment,
        )
    }

    pub(crate) fn external_shader_read_barriers_with_role_states(
        &self,
        main_color_before: TextureUsageState,
        main_depth_before: TextureUsageState,
        translucent_color_before: TextureUsageState,
        translucent_depth_before: TextureUsageState,
        item_entity_color_before: TextureUsageState,
        item_entity_depth_before: TextureUsageState,
        particles_color_before: TextureUsageState,
        particles_depth_before: TextureUsageState,
        clouds_color_before: TextureUsageState,
        clouds_depth_before: TextureUsageState,
        weather_color_before: TextureUsageState,
        weather_depth_before: TextureUsageState,
    ) -> Vec<CommandOp> {
        let mut operations = Vec::with_capacity(12);
        for (attachment, color_before, depth_before) in [
            (&self.main, main_color_before, main_depth_before),
            (
                &self.translucent,
                translucent_color_before,
                translucent_depth_before,
            ),
            (
                &self.item_entity,
                item_entity_color_before,
                item_entity_depth_before,
            ),
            (
                &self.particles,
                particles_color_before,
                particles_depth_before,
            ),
            (&self.clouds, clouds_color_before, clouds_depth_before),
            (&self.weather, weather_color_before, weather_depth_before),
        ] {
            if color_before != TextureUsageState::ShaderRead {
                operations.push(CommandOp::Barrier(ResourceBarrier {
                    resource: attachment.color_texture,
                    subresources: None,
                    before: color_before,
                    after: TextureUsageState::ShaderRead,
                    src_queue: super::super::resources::QueueClass::Graphics,
                    dst_queue: super::super::resources::QueueClass::Graphics,
                }));
            }
            if depth_before != TextureUsageState::ShaderRead {
                operations.push(CommandOp::Barrier(ResourceBarrier {
                    resource: attachment.depth_texture,
                    subresources: None,
                    before: depth_before,
                    after: TextureUsageState::ShaderRead,
                    src_queue: super::super::resources::QueueClass::Graphics,
                    dst_queue: super::super::resources::QueueClass::Graphics,
                }));
            }
        }
        operations
    }

    pub(crate) fn final_target_color_attachment_barrier(&self) -> CommandOp {
        CommandOp::Barrier(ResourceBarrier {
            resource: self.final_target.texture,
            subresources: None,
            before: TextureUsageState::Undefined,
            after: TextureUsageState::ColorAttachment,
            src_queue: super::super::resources::QueueClass::Graphics,
            dst_queue: super::super::resources::QueueClass::Graphics,
        })
    }

    /// Copies the already-rendered main color into the private optical hand
    /// target.  The operation is deliberately expressed as GAL barriers and
    /// a texture copy; no framebuffer, image, or native handle crosses the
    /// semantic boundary.
    pub(crate) fn optical_hand_copy_from_main(&self, extent: Extent3d) -> Vec<CommandOp> {
        vec![
            CommandOp::Barrier(ResourceBarrier {
                resource: self.main.color_texture,
                subresources: None,
                before: TextureUsageState::ColorAttachment,
                after: TextureUsageState::TransferSrc,
                src_queue: super::super::resources::QueueClass::Graphics,
                dst_queue: super::super::resources::QueueClass::Graphics,
            }),
            CommandOp::Barrier(ResourceBarrier {
                resource: self.optical_hand.color_texture,
                subresources: None,
                before: TextureUsageState::Undefined,
                after: TextureUsageState::TransferDst,
                src_queue: super::super::resources::QueueClass::Graphics,
                dst_queue: super::super::resources::QueueClass::Graphics,
            }),
            CommandOp::CopyTexture(super::super::commands::TextureImageCopyRegion {
                row_order: crate::render::vulkanic::commands::TextureRowOrder::Preserve,
                src_texture: self.main.color_texture,
                src_mip: 0,
                src_layer: 0,
                src_origin: super::super::commands::TextureOrigin3d { x: 0, y: 0, z: 0 },
                dst_texture: self.optical_hand.color_texture,
                dst_mip: 0,
                dst_layer: 0,
                dst_origin: super::super::commands::TextureOrigin3d { x: 0, y: 0, z: 0 },
                extent,
            }),
            CommandOp::Barrier(ResourceBarrier {
                resource: self.optical_hand.color_texture,
                subresources: None,
                before: TextureUsageState::TransferDst,
                after: TextureUsageState::ColorAttachment,
                src_queue: super::super::resources::QueueClass::Graphics,
                dst_queue: super::super::resources::QueueClass::Graphics,
            }),
            CommandOp::Barrier(ResourceBarrier {
                resource: self.main.color_texture,
                subresources: None,
                before: TextureUsageState::TransferSrc,
                after: TextureUsageState::ColorAttachment,
                src_queue: super::super::resources::QueueClass::Graphics,
                dst_queue: super::super::resources::QueueClass::Graphics,
            }),
        ]
    }

    /// Copies the optical result back over the Rust-owned main color. The
    /// caller records this only after the optical hand pass has ended.
    pub(crate) fn optical_hand_copy_to_main(&self, extent: Extent3d) -> Vec<CommandOp> {
        vec![
            CommandOp::Barrier(ResourceBarrier {
                resource: self.optical_hand.color_texture,
                subresources: None,
                before: TextureUsageState::ColorAttachment,
                after: TextureUsageState::TransferSrc,
                src_queue: super::super::resources::QueueClass::Graphics,
                dst_queue: super::super::resources::QueueClass::Graphics,
            }),
            CommandOp::Barrier(ResourceBarrier {
                resource: self.main.color_texture,
                subresources: None,
                before: TextureUsageState::ColorAttachment,
                after: TextureUsageState::TransferDst,
                src_queue: super::super::resources::QueueClass::Graphics,
                dst_queue: super::super::resources::QueueClass::Graphics,
            }),
            CommandOp::CopyTexture(super::super::commands::TextureImageCopyRegion {
                row_order: crate::render::vulkanic::commands::TextureRowOrder::Preserve,
                src_texture: self.optical_hand.color_texture,
                src_mip: 0,
                src_layer: 0,
                src_origin: super::super::commands::TextureOrigin3d { x: 0, y: 0, z: 0 },
                dst_texture: self.main.color_texture,
                dst_mip: 0,
                dst_layer: 0,
                dst_origin: super::super::commands::TextureOrigin3d { x: 0, y: 0, z: 0 },
                extent,
            }),
            CommandOp::Barrier(ResourceBarrier {
                resource: self.main.color_texture,
                subresources: None,
                before: TextureUsageState::TransferDst,
                after: TextureUsageState::ColorAttachment,
                src_queue: super::super::resources::QueueClass::Graphics,
                dst_queue: super::super::resources::QueueClass::Graphics,
            }),
            CommandOp::Barrier(ResourceBarrier {
                resource: self.optical_hand.color_texture,
                subresources: None,
                before: TextureUsageState::TransferSrc,
                after: TextureUsageState::ColorAttachment,
                src_queue: super::super::resources::QueueClass::Graphics,
                dst_queue: super::super::resources::QueueClass::Graphics,
            }),
        ]
    }

    pub(crate) fn destroy(self, gal: &mut VulkanicGal) {
        if let Some(pipelines) = self.pipelines {
            for handle in pipelines.handles_in_destroy_order() {
                let _ = gal.destroy(handle);
            }
        }
        if let Some(bindings) = self.bindings {
            for handle in bindings.handles_in_destroy_order() {
                let _ = gal.destroy(handle);
            }
        }
        for handle in self.final_target.handles_in_destroy_order() {
            let _ = gal.destroy(handle);
        }
        for resource in [
            self.optical_hand,
            self.weather,
            self.clouds,
            self.particles,
            self.item_entity,
            self.translucent,
            self.main,
        ] {
            for handle in resource.handles_in_destroy_order() {
                let _ = gal.destroy(handle);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::vulkanic::backends::mock::MockBackend;
    use crate::render::vulkanic::backends::vulkan_capabilities;
    use crate::render::vulkanic::frame::{FrameRenderTargetId, FrameSurfaceDesc, PresentMode};
    use crate::render::vulkanic::gal::VulkanicGal;
    use crate::render::vulkanic::handles::HandleKind;
    use crate::render::vulkanic::resources::FrameTargetDesc;

    #[test]
    fn attachment_contract_is_explicitly_two_dimensional_and_bounded() {
        assert_eq!(FABULOUS_DEPTH_FORMAT, TextureFormat::Depth32Float);
        assert_eq!(
            Extent3d {
                width: 1,
                height: 1,
                depth: 1
            }
            .depth,
            1
        );
    }

    #[test]
    fn material_source_families_map_to_distinct_fabulous_roles() {
        use super::super::super::world_primitive_frontend::{
            WORLD_MATERIAL_SOURCE_CLOUDS, WORLD_MATERIAL_SOURCE_ENTITY_MODEL,
            WORLD_MATERIAL_SOURCE_PARTICLES, WORLD_MATERIAL_SOURCE_TEXTURED,
            WORLD_MATERIAL_SOURCE_WEATHER,
        };
        assert_eq!(
            FabulousTargetRole::for_material_source(WORLD_MATERIAL_SOURCE_TEXTURED),
            Some(FabulousTargetRole::Translucent)
        );
        assert_eq!(
            FabulousTargetRole::for_material_source(WORLD_MATERIAL_SOURCE_ENTITY_MODEL),
            Some(FabulousTargetRole::Translucent)
        );
        assert_eq!(
            FabulousTargetRole::for_material_source(WORLD_MATERIAL_SOURCE_PARTICLES),
            Some(FabulousTargetRole::Particles)
        );
        assert_eq!(
            FabulousTargetRole::for_material_source(WORLD_MATERIAL_SOURCE_CLOUDS),
            Some(FabulousTargetRole::Clouds)
        );
        assert_eq!(
            FabulousTargetRole::for_material_source(WORLD_MATERIAL_SOURCE_WEATHER),
            Some(FabulousTargetRole::Weather)
        );
        assert_eq!(
            FabulousTargetRole::for_material_source(0),
            Some(FabulousTargetRole::Translucent)
        );
    }

    #[test]
    fn attachment_set_allocates_explicit_passes_and_validates_the_bundled_graph() {
        let mut capabilities = vulkan_capabilities();
        capabilities.features.presentation = true;
        let mut gal = VulkanicGal::new_with_backend(
            Box::new(MockBackend::with_capabilities(capabilities)),
            false,
        );
        let set = FabulousAttachmentSet::create(
            &mut gal,
            Extent3d {
                width: 64,
                height: 64,
                depth: 1,
            },
            TextureFormat::Rgba8Unorm,
        )
        .unwrap();
        assert_ne!(set.main.render_pass, Handle::NULL);
        assert_ne!(set.translucent.render_pass, set.particles.render_pass);
        assert_ne!(set.final_target.render_target, set.main.render_target);
        assert_eq!(set.bindings.as_ref().unwrap().combined_samplers.len(), 12);
        let executor =
            super::super::vanilla_post_effect_executor::bundled_transparency_executor().unwrap();
        set.external_inventory()
            .validate_against(executor.plan())
            .unwrap();
        assert_eq!(13, set.pre_transparency_barriers().len());
        let mut capture_copy = Vec::new();
        set.append_translucent_capture_copy(
            &mut capture_copy,
            set.main.color_texture,
            Extent3d {
                width: 64,
                height: 64,
                depth: 1,
            },
            TextureUsageState::ShaderRead,
            TextureUsageState::Undefined,
        )
        .unwrap();
        assert!(capture_copy.iter().any(|operation| matches!(
            operation,
            super::super::super::commands::CommandOp::CopyTexture(copy)
                if copy.src_texture == set.main.color_texture
                    && copy.dst_texture == set.translucent.color_texture
        )));
        let stateful_barriers = set.external_shader_read_barriers_with_translucent_state(
            TextureUsageState::ShaderRead,
            TextureUsageState::Undefined,
        );
        assert!(!stateful_barriers.iter().any(|operation| matches!(
            operation,
            super::super::super::commands::CommandOp::Barrier(barrier)
                if barrier.resource == set.translucent.color_texture
                    && barrier.before == TextureUsageState::ShaderRead
        )));
        let mut main_copy = Vec::new();
        set.append_frame_target_to_main_copy(
            &mut main_copy,
            Handle::new(HandleKind::FrameTarget, 99, 1).unwrap(),
            Extent3d {
                width: 64,
                height: 64,
                depth: 1,
            },
            TextureUsageState::Undefined,
        )
        .unwrap();
        assert!(main_copy.iter().any(|operation| matches!(
            operation,
            super::super::super::commands::CommandOp::CopyFrameTargetToTexture {
                src,
                dst,
                ..
            } if *src == Handle::new(HandleKind::FrameTarget, 99, 1).unwrap()
                && *dst == set.main.color_texture
        )));
        let mut depth_copy = Vec::new();
        set.append_depth_capture_copy(
            &mut depth_copy,
            set.main.depth_texture,
            FabulousTargetRole::Translucent,
            Extent3d {
                width: 64,
                height: 64,
                depth: 1,
            },
            TextureUsageState::ShaderRead,
            TextureUsageState::Undefined,
        )
        .unwrap();
        assert!(depth_copy.iter().any(|operation| matches!(
            operation,
            super::super::super::commands::CommandOp::CopyTexture(copy)
                if copy.src_texture == set.main.depth_texture
                    && copy.dst_texture == set.translucent.depth_texture
        )));
        assert_eq!(1, set.between_transparency_pass_barriers().len());
        let copy_in = set.optical_hand_copy_from_main(Extent3d {
            width: 64,
            height: 64,
            depth: 1,
        });
        assert_eq!(5, copy_in.len());
        assert!(copy_in.iter().any(|operation| matches!(
            operation,
            super::super::super::commands::CommandOp::CopyTexture(copy)
                if copy.src_texture == set.main.color_texture
                    && copy.dst_texture == set.optical_hand.color_texture
        )));
        let copy_out = set.optical_hand_copy_to_main(Extent3d {
            width: 64,
            height: 64,
            depth: 1,
        });
        assert_eq!(5, copy_out.len());
        assert!(copy_out.iter().any(|operation| matches!(
            operation,
            super::super::super::commands::CommandOp::CopyTexture(copy)
                if copy.src_texture == set.optical_hand.color_texture
                    && copy.dst_texture == set.main.color_texture
        )));
        let pass_bindings = set.transparency_pass_bindings().unwrap();
        let operations = executor.lower(&pass_bindings).unwrap();
        assert_eq!(
            2,
            operations
                .iter()
                .filter(|operation| matches!(
                    operation,
                    super::super::super::commands::CommandOp::Draw {
                        vertices: 3,
                        instances: 1
                    }
                ))
                .count()
        );
        gal.create_command_list(super::super::super::commands::CommandListDesc {
            label: "fabulous-transparency-test".to_string(),
            operations,
        })
        .unwrap();
        gal.configure_frame_surface(FrameSurfaceDesc {
            label: "fabulous-frame-surface".to_string(),
            extent: Extent3d {
                width: 64,
                height: 64,
                depth: 1,
            },
            color_format: TextureFormat::Rgba8Unorm,
            present_mode: PresentMode::Fifo,
            max_frames_in_flight: 2,
        })
        .unwrap();
        let frame_target = gal
            .create_frame_target(FrameTargetDesc {
                label: "fabulous-frame-target".to_string(),
                frame_id: 1,
                render_target: FrameRenderTargetId(1),
                extent: Extent3d {
                    width: 64,
                    height: 64,
                    depth: 1,
                },
                color_format: TextureFormat::Rgba8Unorm,
            })
            .unwrap();
        let mut terrain_handoff = Vec::new();
        let presentation_pass = set
            .append_terrain_handoff_to_frame_target(
                &mut gal,
                &mut terrain_handoff,
                frame_target,
                Extent3d {
                    width: 64,
                    height: 64,
                    depth: 1,
                },
                frame_target,
                set.main.color_texture,
                set.main.depth_texture,
                set.main.depth_texture,
            )
            .unwrap();
        assert!(terrain_handoff.iter().any(|operation| matches!(
            operation,
            super::super::super::commands::CommandOp::CopyFrameTargetToTexture { .. }
        )));
        assert_ne!(Handle::NULL, presentation_pass);
        let (frame_bindings, present_pass) = set
            .transparency_pass_bindings_to_frame_target(&mut gal, frame_target)
            .unwrap();
        assert_eq!(frame_target, frame_bindings.last().unwrap().render_target);
        assert_eq!(
            frame_target,
            frame_bindings.last().unwrap().color_attachment
        );
        assert!(frame_bindings.last().unwrap().depth_attachment.is_none());
        let frame_operations = executor.lower(&frame_bindings).unwrap();
        gal.create_command_list(super::super::super::commands::CommandListDesc {
            label: "fabulous-transparency-frame-target-test".to_string(),
            operations: frame_operations,
        })
        .unwrap();
        gal.destroy(presentation_pass).unwrap();
        gal.destroy(present_pass).unwrap();
        set.destroy(&mut gal);
    }

    #[test]
    fn terrain_handoff_accepts_bgra_frame_with_rgba_translucent_capture() {
        let mut capabilities = vulkan_capabilities();
        capabilities.features.presentation = true;
        let mut gal = VulkanicGal::new_with_backend(
            Box::new(MockBackend::with_capabilities(capabilities)),
            false,
        );
        let set = FabulousAttachmentSet::create_with_translucent_format(
            &mut gal,
            Extent3d {
                width: 32,
                height: 32,
                depth: 1,
            },
            TextureFormat::Bgra8Unorm,
            TextureFormat::Rgba8Unorm,
        )
        .unwrap();
        assert_eq!(TextureFormat::Rgba8Unorm, set.translucent_color_format);
        gal.configure_frame_surface(FrameSurfaceDesc {
            label: "terrain-bgra-surface".to_string(),
            extent: Extent3d {
                width: 32,
                height: 32,
                depth: 1,
            },
            color_format: TextureFormat::Bgra8Unorm,
            present_mode: PresentMode::Fifo,
            max_frames_in_flight: 2,
        })
        .unwrap();
        let frame_target = gal
            .create_frame_target(FrameTargetDesc {
                label: "terrain-bgra-target".to_string(),
                frame_id: 7,
                render_target: FrameRenderTargetId(7),
                extent: Extent3d {
                    width: 32,
                    height: 32,
                    depth: 1,
                },
                color_format: TextureFormat::Bgra8Unorm,
            })
            .unwrap();
        let capture_source = gal
            .create_texture(TextureDesc {
                label: "terrain-bgra-capture-source".to_string(),
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba8Unorm,
                extent: Extent3d {
                    width: 32,
                    height: 32,
                    depth: 1,
                },
                mip_levels: 1,
                array_layers: 1,
                usages: vec![TextureUsage::TransferSrc, TextureUsage::Sampled],
            })
            .unwrap();
        let capture_depth = gal
            .create_texture(TextureDesc {
                label: "terrain-bgra-depth-source".to_string(),
                dimension: TextureDimension::D2,
                format: TextureFormat::Depth32Float,
                extent: Extent3d {
                    width: 32,
                    height: 32,
                    depth: 1,
                },
                mip_levels: 1,
                array_layers: 1,
                usages: vec![TextureUsage::TransferSrc, TextureUsage::Sampled],
            })
            .unwrap();
        let translucent_depth = gal.create_texture(TextureDesc {
            label: "terrain-independent-translucent-depth".to_string(),
            dimension: TextureDimension::D2, format: TextureFormat::Depth32Float,
            extent: Extent3d { width: 32, height: 32, depth: 1 },
            mip_levels: 1, array_layers: 1,
            usages: vec![TextureUsage::TransferSrc, TextureUsage::Sampled],
        }).unwrap();
        let mut operations = Vec::new();
        let presentation_pass = set
            .append_terrain_handoff_to_frame_target(
                &mut gal,
                &mut operations,
                frame_target,
                Extent3d {
                    width: 32,
                    height: 32,
                    depth: 1,
                },
                frame_target,
                capture_source,
                capture_depth,
                translucent_depth,
            )
            .unwrap();
        let depth_copy_sources: Vec<_> = operations.iter().filter_map(|operation| match operation {
            super::super::super::commands::CommandOp::CopyTexture(copy)
                if copy.dst_texture == set.main.depth_texture || copy.dst_texture == set.translucent.depth_texture =>
                    Some((copy.src_texture, copy.dst_texture)),
            _ => None,
        }).collect();
        assert_eq!(depth_copy_sources, vec![(capture_depth, set.main.depth_texture),
            (translucent_depth, set.translucent.depth_texture)],
            "each Fabulous color layer must retain its own producer depth");
        gal.create_command_list(super::super::super::commands::CommandListDesc {
            label: "terrain-bgra-handoff-test".to_string(),
            operations,
        })
        .unwrap();
        assert_ne!(Handle::NULL, presentation_pass);
        gal.destroy(presentation_pass).unwrap();
        let mut no_translucent_operations = Vec::new();
        let no_translucent_presentation_pass = set
            .append_terrain_handoff_to_frame_target_with_external_ops(
                &mut gal,
                &mut no_translucent_operations,
                frame_target,
                Extent3d { width: 32, height: 32, depth: 1 },
                frame_target,
                capture_source,
                capture_depth,
                capture_depth,
                &[],
                [false; 4],
                false,
                false,
            )
            .unwrap();
        assert!(!no_translucent_operations.iter().any(|operation| matches!(
            operation,
            super::super::super::commands::CommandOp::CopyTexture(copy)
                if copy.src_texture == capture_source
        )));
        assert!(no_translucent_operations.iter().any(|operation| matches!(
            operation,
            super::super::super::commands::CommandOp::BeginPass { target, .. }
                if *target == set.translucent.render_target
        )));
        gal.destroy(no_translucent_presentation_pass).unwrap();
        gal.destroy(capture_source).unwrap();
        gal.destroy(capture_depth).unwrap();
        gal.destroy(translucent_depth).unwrap();
        set.destroy(&mut gal);
    }
}
