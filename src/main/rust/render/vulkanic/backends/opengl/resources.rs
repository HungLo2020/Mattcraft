use std::collections::BTreeMap;
use std::rc::Rc;

use glow::HasContext;

use super::trace;
use crate::render::vulkanic::backends::{BackendCreateDesc, BackendToken};
use crate::render::vulkanic::commands::{CommandOp, ValidatedSubmissionBatch};
use crate::render::vulkanic::error::{GalError, GalResult};
use crate::render::vulkanic::handles::{Handle, HandleKind};
use crate::render::vulkanic::resources::*;

// OpenGL has no descriptor-set namespace. Keep that reconstruction entirely
// private by assigning each GAL set a stable, disjoint binding range.
// GLSL 4.50 guarantees at least 16 fragment texture-image units. The current
// two-set terrain contract uses bindings 0..=3 in set 1, so an eight-slot
// stride keeps every translated sampler binding inside that portable range.
const OPENGL_RESOURCE_SET_BINDING_STRIDE: u32 = 8;

pub(super) fn opengl_resource_binding_point(set_index: u32, binding: u32) -> GalResult<u32> {
    set_index
        .checked_mul(OPENGL_RESOURCE_SET_BINDING_STRIDE)
        .and_then(|base| base.checked_add(binding))
        .ok_or_else(|| GalError::backend("OpenGL resource set binding point overflows"))
}

pub(super) struct OpenGlObjects {
    gl: Rc<glow::Context>,
    objects: BTreeMap<Handle, OpenGlObject>,
    frame_target_framebuffers: BTreeMap<u64, Option<glow::Framebuffer>>,
    current_frame_target_framebuffer: Option<Option<glow::Framebuffer>>,
    next_token: u64,
}

impl OpenGlObjects {
    pub(super) fn new(gl: Rc<glow::Context>) -> Self {
        Self {
            gl,
            objects: BTreeMap::new(),
            frame_target_framebuffers: BTreeMap::new(),
            current_frame_target_framebuffer: None,
            next_token: 1,
        }
    }

    pub(super) fn set_frame_target_framebuffer(
        &mut self,
        frame_id: u64,
        framebuffer: Option<glow::Framebuffer>,
    ) {
        self.current_frame_target_framebuffer = Some(framebuffer);
        self.frame_target_framebuffers.insert(frame_id, framebuffer);
    }

    pub(super) fn create(
        &mut self,
        handle: Handle,
        desc: BackendCreateDesc<'_>,
    ) -> GalResult<BackendToken> {
        let _zone = trace::Zone::new("opengl.resources.create-native");
        let token = BackendToken(self.next_token);
        self.next_token += 1;
        let object = match desc {
            BackendCreateDesc::Buffer(desc) => {
                OpenGlObject::Buffer(self.create_buffer(desc, token)?)
            }
            BackendCreateDesc::Texture(desc) => {
                OpenGlObject::Texture(self.create_texture(desc, token)?)
            }
            BackendCreateDesc::TextureView(desc) => OpenGlObject::TextureView(TextureViewObject {
                token,
                texture: desc.texture,
                format: desc.format,
                base_mip: desc.base_mip,
                mip_count: desc.mip_count,
                base_layer: desc.base_layer,
                layer_count: desc.layer_count,
            }),
            BackendCreateDesc::Sampler(desc) => {
                OpenGlObject::Sampler(self.create_sampler(desc, token)?)
            }
            BackendCreateDesc::CombinedTextureSampler(desc) => {
                OpenGlObject::CombinedTextureSampler(CombinedTextureSamplerObject {
                    token,
                    texture_view: desc.texture_view,
                    sampler: desc.sampler,
                })
            }
            BackendCreateDesc::ShaderModule(desc) => {
                OpenGlObject::ShaderModule(self.create_shader_module(desc, token)?)
            }
            BackendCreateDesc::ResourceLayout(desc) => {
                OpenGlObject::ResourceLayout(ResourceLayoutObject {
                    token,
                    bindings: desc.bindings.clone(),
                })
            }
            BackendCreateDesc::ResourceSet(desc) => OpenGlObject::ResourceSet(ResourceSetObject {
                token,
                layout: desc.layout,
                bindings: desc.bindings.clone(),
            }),
            BackendCreateDesc::PipelineLayout(desc) => {
                OpenGlObject::PipelineLayout(PipelineLayoutObject {
                    token,
                    resource_layouts: desc.resource_layouts.clone(),
                })
            }
            BackendCreateDesc::GraphicsPipeline(desc) => {
                OpenGlObject::GraphicsPipeline(self.create_graphics_pipeline(desc, token)?)
            }
            BackendCreateDesc::ComputePipeline(desc) => {
                OpenGlObject::ComputePipeline(self.create_compute_pipeline(desc, token)?)
            }
            BackendCreateDesc::RenderTarget(desc) => {
                OpenGlObject::RenderTarget(self.create_render_target(desc, token)?)
            }
            BackendCreateDesc::FrameTarget(desc) => OpenGlObject::FrameTarget(FrameTargetObject {
                token,
                frame_id: desc.frame_id,
                framebuffer: self
                    .frame_target_framebuffers
                    .remove(&desc.frame_id)
                    .or(self.current_frame_target_framebuffer)
                    .unwrap_or(None),
                extent: desc.extent,
                color_format: desc.color_format,
            }),
            BackendCreateDesc::RenderPass(desc) => OpenGlObject::RenderPass(RenderPassObject {
                token,
                target: desc.target,
                color_formats: desc.color_formats.clone(),
                depth_format: desc.depth_format,
            }),
        };
        self.objects.insert(handle, object);
        Ok(token)
    }

    pub(super) fn destroy(
        &mut self,
        handle: Handle,
        kind: HandleKind,
        token: BackendToken,
    ) -> GalResult<()> {
        let _zone = trace::Zone::new("opengl.resources.destroy-native");
        let Some(object) = self.objects.remove(&handle) else {
            return Err(GalError::backend("OpenGL destroy for unknown handle"));
        };
        if object.kind() != kind || object.token() != token {
            self.objects.insert(handle, object);
            return Err(GalError::backend("OpenGL destroy kind or token mismatch"));
        }
        self.destroy_object(object);
        Ok(())
    }

    pub(super) fn buffer(&self, handle: Handle) -> GalResult<&BufferObject> {
        match self.objects.get(&handle) {
            Some(OpenGlObject::Buffer(object)) => Ok(object),
            _ => Err(expected("buffer", handle)),
        }
    }

    pub(super) fn buffer_mut(&mut self, handle: Handle) -> GalResult<&mut BufferObject> {
        match self.objects.get_mut(&handle) {
            Some(OpenGlObject::Buffer(object)) => Ok(object),
            _ => Err(expected("buffer", handle)),
        }
    }

    pub(super) fn texture(&self, handle: Handle) -> GalResult<&TextureObject> {
        match self.objects.get(&handle) {
            Some(OpenGlObject::Texture(object)) => Ok(object),
            _ => Err(expected("texture", handle)),
        }
    }

    pub(super) fn texture_view(&self, handle: Handle) -> GalResult<&TextureViewObject> {
        match self.objects.get(&handle) {
            Some(OpenGlObject::TextureView(object)) => Ok(object),
            _ => Err(expected("texture view", handle)),
        }
    }

    pub(super) fn sampler(&self, handle: Handle) -> GalResult<&SamplerObject> {
        match self.objects.get(&handle) {
            Some(OpenGlObject::Sampler(object)) => Ok(object),
            _ => Err(expected("sampler", handle)),
        }
    }

    pub(super) fn combined_texture_sampler(
        &self,
        handle: Handle,
    ) -> GalResult<&CombinedTextureSamplerObject> {
        match self.objects.get(&handle) {
            Some(OpenGlObject::CombinedTextureSampler(object)) => Ok(object),
            _ => Err(expected("combined texture sampler", handle)),
        }
    }

    pub(super) fn resource_set(&self, handle: Handle) -> GalResult<&ResourceSetObject> {
        match self.objects.get(&handle) {
            Some(OpenGlObject::ResourceSet(object)) => Ok(object),
            _ => Err(expected("resource set", handle)),
        }
    }

    pub(super) fn submission_uses_storage_textures(
        &self,
        batch: &ValidatedSubmissionBatch,
    ) -> bool {
        batch.command_lists.iter().any(|list| {
            list.operations.iter().any(|operation| {
                let CommandOp::BindResourceSet { set, .. } = operation else {
                    return false;
                };
                self.resource_set(*set)
                    .map(|resource_set| {
                        resource_set
                            .bindings
                            .iter()
                            .any(|binding| binding.kind == ResourceBindingKind::StorageTexture)
                    })
                    .unwrap_or(false)
            })
        })
    }

    pub(super) fn resource_layout(&self, handle: Handle) -> GalResult<&ResourceLayoutObject> {
        match self.objects.get(&handle) {
            Some(OpenGlObject::ResourceLayout(object)) => Ok(object),
            _ => Err(expected("resource layout", handle)),
        }
    }

    pub(super) fn pipeline_layout(&self, handle: Handle) -> GalResult<&PipelineLayoutObject> {
        match self.objects.get(&handle) {
            Some(OpenGlObject::PipelineLayout(object)) => Ok(object),
            _ => Err(expected("pipeline layout", handle)),
        }
    }

    pub(super) fn graphics_pipeline(&self, handle: Handle) -> GalResult<&GraphicsPipelineObject> {
        match self.objects.get(&handle) {
            Some(OpenGlObject::GraphicsPipeline(object)) => Ok(object),
            _ => Err(expected("graphics pipeline", handle)),
        }
    }

    pub(super) fn compute_pipeline(&self, handle: Handle) -> GalResult<&ComputePipelineObject> {
        match self.objects.get(&handle) {
            Some(OpenGlObject::ComputePipeline(object)) => Ok(object),
            _ => Err(expected("compute pipeline", handle)),
        }
    }

    pub(super) fn pass_target(&self, handle: Handle) -> GalResult<PassTargetObject> {
        match self.objects.get(&handle) {
            Some(OpenGlObject::RenderTarget(object)) => Ok(PassTargetObject {
                framebuffer: Some(object.framebuffer),
                extent: object.extent,
            }),
            Some(OpenGlObject::FrameTarget(object)) => Ok(PassTargetObject {
                // ABI v2 intentionally reuses the same GAL frame-target handle
                // across steady-state borrowed-context frames. Refresh the
                // native OpenGL draw framebuffer on acquire so screen
                // transitions cannot leave the persistent GAL handle pointing
                // at a stale Java framebuffer.
                framebuffer: self
                    .current_frame_target_framebuffer
                    .unwrap_or(object.framebuffer),
                extent: object.extent,
            }),
            _ => Err(expected("render target or frame target", handle)),
        }
    }

    pub(super) fn render_pass(&self, handle: Handle) -> GalResult<&RenderPassObject> {
        match self.objects.get(&handle) {
            Some(OpenGlObject::RenderPass(object)) => Ok(object),
            _ => Err(expected("render pass", handle)),
        }
    }

    pub(super) fn destroy_all(&mut self) {
        let _zone = trace::Zone::new("opengl.resources.destroy-all");
        let objects = std::mem::take(&mut self.objects);
        for (_, object) in objects.into_iter().rev() {
            self.destroy_object(object);
        }
    }

    fn create_buffer(&self, desc: &BufferDesc, token: BackendToken) -> GalResult<BufferObject> {
        let size = i32::try_from(desc.size)
            .map_err(|_| GalError::backend("OpenGL buffer size exceeds i32"))?;
        let buffer = unsafe { self.gl.create_buffer() }.map_err(|error| {
            GalError::backend(format!("failed to create OpenGL buffer: {error}"))
        })?;
        unsafe {
            self.gl.bind_buffer(glow::COPY_WRITE_BUFFER, Some(buffer));
            self.gl.buffer_data_size(
                glow::COPY_WRITE_BUFFER,
                size,
                glow::DYNAMIC_DRAW,
            );
            self.gl.bind_buffer(glow::COPY_WRITE_BUFFER, None);
        }
        Ok(BufferObject {
            token,
            buffer,
            size: desc.size,
        })
    }

    fn create_texture(&self, desc: &TextureDesc, token: BackendToken) -> GalResult<TextureObject> {
        if !matches!(desc.dimension, TextureDimension::D2 | TextureDimension::D3)
            || desc.array_layers != 1
        {
            return Err(GalError::backend(
                "OpenGL backend requires a single-layer D2 or D3 texture",
            ));
        }
        let format = texture_format(desc.format)?;
        let target = texture_target(desc.dimension);
        let texture = unsafe { self.gl.create_texture() }.map_err(|error| {
            GalError::backend(format!("failed to create OpenGL texture: {error}"))
        })?;
        unsafe {
            self.gl.bind_texture(target, Some(texture));
            let min_filter = if format.integer {
                if desc.mip_levels > 1 {
                    glow::NEAREST_MIPMAP_NEAREST as i32
                } else {
                    glow::NEAREST as i32
                }
            } else if desc.mip_levels > 1 {
                glow::LINEAR_MIPMAP_NEAREST as i32
            } else {
                glow::LINEAR as i32
            };
            self.gl
                .tex_parameter_i32(target, glow::TEXTURE_MIN_FILTER, min_filter);
            self.gl.tex_parameter_i32(
                target,
                glow::TEXTURE_MAG_FILTER,
                if format.integer {
                    glow::NEAREST as i32
                } else {
                    glow::LINEAR as i32
                },
            );
            self.gl
                .tex_parameter_i32(target, glow::TEXTURE_BASE_LEVEL, 0);
            self.gl.tex_parameter_i32(
                target,
                glow::TEXTURE_MAX_LEVEL,
                i32::try_from(desc.mip_levels - 1)
                    .map_err(|_| GalError::backend("texture mip count exceeds i32"))?,
            );
            for mip in 0..desc.mip_levels {
                let extent = texture_mip_extent(desc.extent, mip);
                match desc.dimension {
                    TextureDimension::D2 => self.gl.tex_image_2d(
                        target,
                        i32::try_from(mip)
                            .map_err(|_| GalError::backend("texture mip exceeds i32"))?,
                        format.internal,
                        i32::try_from(extent.width)
                            .map_err(|_| GalError::backend("texture width exceeds i32"))?,
                        i32::try_from(extent.height)
                            .map_err(|_| GalError::backend("texture height exceeds i32"))?,
                        0,
                        format.external,
                        format.ty,
                        glow::PixelUnpackData::Slice(None),
                    ),
                    TextureDimension::D3 => self.gl.tex_image_3d(
                        target,
                        i32::try_from(mip)
                            .map_err(|_| GalError::backend("texture mip exceeds i32"))?,
                        format.internal,
                        i32::try_from(extent.width)
                            .map_err(|_| GalError::backend("texture width exceeds i32"))?,
                        i32::try_from(extent.height)
                            .map_err(|_| GalError::backend("texture height exceeds i32"))?,
                        i32::try_from(extent.depth)
                            .map_err(|_| GalError::backend("texture depth exceeds i32"))?,
                        0,
                        format.external,
                        format.ty,
                        glow::PixelUnpackData::Slice(None),
                    ),
                    _ => unreachable!("GAL validated texture dimension"),
                }
            }
            self.gl.bind_texture(target, None);
        }
        Ok(TextureObject {
            token,
            texture,
            format: desc.format,
            extent: desc.extent,
            dimension: desc.dimension,
            mip_levels: desc.mip_levels,
            array_layers: desc.array_layers,
        })
    }

    fn create_sampler(&self, desc: &SamplerDesc, token: BackendToken) -> GalResult<SamplerObject> {
        let sampler = unsafe { self.gl.create_sampler() }.map_err(|error| {
            GalError::backend(format!("failed to create OpenGL sampler: {error}"))
        })?;
        unsafe {
            self.gl
                .sampler_parameter_i32(sampler, glow::TEXTURE_MIN_FILTER, min_filter(desc));
            self.gl.sampler_parameter_i32(
                sampler,
                glow::TEXTURE_MAG_FILTER,
                filter(desc.mag_filter),
            );
            self.gl
                .sampler_parameter_i32(sampler, glow::TEXTURE_WRAP_S, address(desc.address_u));
            self.gl
                .sampler_parameter_i32(sampler, glow::TEXTURE_WRAP_T, address(desc.address_v));
            self.gl
                .sampler_parameter_i32(sampler, glow::TEXTURE_WRAP_R, address(desc.address_w));
            self.gl.sampler_parameter_i32(
                sampler,
                glow::TEXTURE_COMPARE_MODE,
                if desc.comparison.is_some() {
                    glow::COMPARE_REF_TO_TEXTURE as i32
                } else {
                    glow::NONE as i32
                },
            );
            if let Some(compare) = desc.comparison {
                self.gl.sampler_parameter_i32(
                    sampler,
                    glow::TEXTURE_COMPARE_FUNC,
                    super::lowering::compare_op(compare) as i32,
                );
            }
        }
        Ok(SamplerObject { token, sampler })
    }

    fn create_shader_module(
        &self,
        desc: &ShaderModuleDesc,
        token: BackendToken,
    ) -> GalResult<ShaderModuleObject> {
        if desc.code_format != ShaderCodeFormat::Glsl {
            return Err(GalError::backend(
                "OpenGL backend requires GLSL shader source",
            ));
        }
        let ty = shader_stage(desc.stage)?;
        let source = std::str::from_utf8(&desc.code)
            .map_err(|_| GalError::backend("OpenGL GLSL shader source is not UTF-8"))?;
        let source = opengl_shader_source(source);
        let shader = unsafe { self.gl.create_shader(ty) }.map_err(|error| {
            GalError::backend(format!(
                "failed to create OpenGL shader '{}': {error}",
                desc.label
            ))
        })?;
        unsafe {
            self.gl.shader_source(shader, &source);
            self.gl.compile_shader(shader);
            if !self.gl.get_shader_compile_status(shader) {
                let log = self.gl.get_shader_info_log(shader);
                self.gl.delete_shader(shader);
                return Err(GalError::backend(format!(
                    "OpenGL shader '{}' failed to compile: {log}",
                    desc.label
                )));
            }
        }
        Ok(ShaderModuleObject {
            token,
            shader,
            stage: desc.stage,
            sampler_bindings: parse_sampler2d_layout_bindings(&source),
        })
    }

    fn create_graphics_pipeline(
        &self,
        desc: &GraphicsPipelineDesc,
        token: BackendToken,
    ) -> GalResult<GraphicsPipelineObject> {
        let vertex = self.shader(desc.vertex_shader)?;
        let fragment = self.shader(desc.fragment_shader)?;
        let program = unsafe { self.gl.create_program() }.map_err(|error| {
            GalError::backend(format!(
                "failed to create OpenGL program '{}': {error}",
                desc.label
            ))
        })?;
        unsafe {
            self.gl.attach_shader(program, vertex.shader);
            self.gl.attach_shader(program, fragment.shader);
            self.gl.link_program(program);
            self.gl.detach_shader(program, vertex.shader);
            self.gl.detach_shader(program, fragment.shader);
            if !self.gl.get_program_link_status(program) {
                let log = self.gl.get_program_info_log(program);
                self.gl.delete_program(program);
                return Err(GalError::backend(format!(
                    "OpenGL program '{}' failed to link: {log}",
                    desc.label
                )));
            }
            let mut sampler_bindings = vertex.sampler_bindings.clone();
            sampler_bindings.extend(fragment.sampler_bindings.clone());
            self.bind_program_interfaces(program, desc.layout, &sampler_bindings)?;
        }
        let vao = unsafe { self.gl.create_vertex_array() }.map_err(|error| {
            unsafe { self.gl.delete_program(program) };
            GalError::backend(format!(
                "failed to create OpenGL VAO '{}': {error}",
                desc.label
            ))
        })?;
        Ok(GraphicsPipelineObject {
            token,
            program,
            vao,
            layout: desc.layout,
            topology: desc.topology,
            cull_mode: desc.cull_mode,
            front_face: desc.front_face,
            provoking_vertex: desc.provoking_vertex,
            raster_y_direction: desc.raster_y_direction,
            blend: desc.blend,
            depth_compare: desc.depth_compare,
            depth_write: desc.depth_write,
            depth_bias: desc.depth_bias,
            stencil: desc.stencil,
        })
    }

    fn create_compute_pipeline(
        &self,
        desc: &ComputePipelineDesc,
        token: BackendToken,
    ) -> GalResult<ComputePipelineObject> {
        let shader = self.shader(desc.shader)?;
        if shader.stage != ShaderStage::Compute {
            return Err(GalError::backend(
                "OpenGL compute pipeline requires a compute shader module",
            ));
        }
        let program = unsafe { self.gl.create_program() }.map_err(|error| {
            GalError::backend(format!(
                "failed to create OpenGL compute program '{}': {error}",
                desc.label
            ))
        })?;
        unsafe {
            self.gl.attach_shader(program, shader.shader);
            self.gl.link_program(program);
            self.gl.detach_shader(program, shader.shader);
            if !self.gl.get_program_link_status(program) {
                let log = self.gl.get_program_info_log(program);
                self.gl.delete_program(program);
                return Err(GalError::backend(format!(
                    "OpenGL compute program '{}' failed to link: {log}",
                    desc.label
                )));
            }
            self.bind_program_interfaces(program, desc.layout, &shader.sampler_bindings)?;
        }
        Ok(ComputePipelineObject {
            token,
            program,
            layout: desc.layout,
        })
    }

    fn create_render_target(
        &self,
        desc: &RenderTargetDesc,
        token: BackendToken,
    ) -> GalResult<RenderTargetObject> {
        let framebuffer = unsafe { self.gl.create_framebuffer() }.map_err(|error| {
            GalError::backend(format!(
                "failed to create OpenGL FBO '{}': {error}",
                desc.label
            ))
        })?;
        unsafe {
            self.gl
                .bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            for (index, view) in desc.color_views.iter().enumerate() {
                let view_object = self.texture_view(*view)?;
                let texture = self.texture(view_object.texture)?;
                self.gl.framebuffer_texture_2d(
                    glow::FRAMEBUFFER,
                    glow::COLOR_ATTACHMENT0
                        + u32::try_from(index)
                            .map_err(|_| GalError::backend("too many color attachments"))?,
                    glow::TEXTURE_2D,
                    Some(texture.texture),
                    i32::try_from(view_object.base_mip)
                        .map_err(|_| GalError::backend("mip level exceeds i32"))?,
                );
            }
            if let Some(view) = desc.depth_stencil_view {
                let view_object = self.texture_view(view)?;
                let texture = self.texture(view_object.texture)?;
                self.gl.framebuffer_texture_2d(
                    glow::FRAMEBUFFER,
                    if texture.format == TextureFormat::Depth24Stencil8 {
                        glow::DEPTH_STENCIL_ATTACHMENT
                    } else {
                        glow::DEPTH_ATTACHMENT
                    },
                    glow::TEXTURE_2D,
                    Some(texture.texture),
                    i32::try_from(view_object.base_mip)
                        .map_err(|_| GalError::backend("mip level exceeds i32"))?,
                );
            }
            let draw_buffers = desc
                .color_views
                .iter()
                .enumerate()
                .map(|(index, _)| glow::COLOR_ATTACHMENT0 + index as u32)
                .collect::<Vec<_>>();
            if !draw_buffers.is_empty() {
                self.gl.draw_buffers(&draw_buffers);
            } else {
                self.gl.draw_buffer(glow::NONE);
                self.gl.read_buffer(glow::NONE);
            }
            let status = self.gl.check_framebuffer_status(glow::FRAMEBUFFER);
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            if status != glow::FRAMEBUFFER_COMPLETE {
                self.gl.delete_framebuffer(framebuffer);
                return Err(GalError::backend(format!(
                    "OpenGL FBO '{}' is incomplete: 0x{status:04x}",
                    desc.label
                )));
            }
        }
        Ok(RenderTargetObject {
            token,
            framebuffer,
            color_views: desc.color_views.clone(),
            depth_stencil_view: desc.depth_stencil_view,
            extent: desc.extent,
        })
    }

    fn shader(&self, handle: Handle) -> GalResult<&ShaderModuleObject> {
        match self.objects.get(&handle) {
            Some(OpenGlObject::ShaderModule(shader)) => Ok(shader),
            _ => Err(expected("shader", handle)),
        }
    }

    fn bind_program_interfaces(
        &self,
        program: glow::Program,
        layout: Handle,
        sampler_bindings: &BTreeMap<String, u32>,
    ) -> GalResult<()> {
        let pipeline_layout = self.pipeline_layout(layout)?;
        for (set_index, resource_layout) in pipeline_layout.resource_layouts.iter().enumerate() {
            let resource_layout = self.resource_layout(*resource_layout)?;
            for binding in &resource_layout.bindings {
                let binding_point = opengl_resource_binding_point(
                    u32::try_from(set_index)
                        .map_err(|_| GalError::backend("OpenGL resource set index exceeds u32"))?,
                    binding.binding,
                )?;
                unsafe {
                    match binding.kind {
                        ResourceBindingKind::UniformBuffer => {
                            let mut bound = false;
                            for name in uniform_block_names(binding_point) {
                                if let Some(index) = self.gl.get_uniform_block_index(program, &name)
                                {
                                    self.gl.uniform_block_binding(program, index, binding_point);
                                    bound = true;
                                }
                            }
                            if !bound {
                                return Err(GalError::backend(format!(
                                    "OpenGL program is missing uniform block for binding {}",
                                    binding_point
                                )));
                            }
                        }
                        ResourceBindingKind::StorageBuffer => {
                            let mut bound = false;
                            for name in storage_block_names(binding_point) {
                                if let Some(index) =
                                    self.gl.get_shader_storage_block_index(program, &name)
                                {
                                    self.gl.shader_storage_block_binding(
                                        program,
                                        index,
                                        binding_point,
                                    );
                                    bound = true;
                                }
                            }
                            if !bound {
                                return Err(GalError::backend(format!(
                                    "OpenGL program is missing storage block for binding {}",
                                    binding_point
                                )));
                            }
                        }
                        ResourceBindingKind::SampledTexture
                        | ResourceBindingKind::CombinedTextureSampler => {
                            let mut names = sampler_bindings
                                .iter()
                                .filter_map(|(name, declared_binding)| {
                                    (*declared_binding == binding_point).then(|| name.clone())
                                })
                                .collect::<Vec<_>>();
                            if names.is_empty() {
                                names = sampler_uniform_names(binding_point)
                                    .into_iter()
                                    .filter(|name| {
                                        sampler_bindings
                                            .get(name)
                                            .map(|declared| *declared == binding_point)
                                            .unwrap_or(true)
                                    })
                                    .collect();
                            }
                            for name in names {
                                if let Some(location) = self.gl.get_uniform_location(program, &name)
                                {
                                    self.gl.use_program(Some(program));
                                    self.gl.uniform_1_i32(
                                        Some(&location),
                                        i32::try_from(binding_point).map_err(|_| {
                                            GalError::backend("OpenGL sampler binding exceeds i32")
                                        })?,
                                    );
                                }
                            }
                        }
                        ResourceBindingKind::Sampler | ResourceBindingKind::StorageTexture => {}
                    }
                }
            }
        }
        unsafe {
            self.gl.use_program(None);
        }
        Ok(())
    }

    fn destroy_object(&self, object: OpenGlObject) {
        unsafe {
            match object {
                OpenGlObject::Buffer(object) => self.gl.delete_buffer(object.buffer),
                OpenGlObject::Texture(object) => self.gl.delete_texture(object.texture),
                OpenGlObject::TextureView(_) => {}
                OpenGlObject::Sampler(object) => self.gl.delete_sampler(object.sampler),
                // This is a logical GAL pairing. Its native texture and sampler
                // remain independently owned and retired by their own records.
                OpenGlObject::CombinedTextureSampler(_) => {}
                OpenGlObject::ShaderModule(object) => self.gl.delete_shader(object.shader),
                OpenGlObject::ResourceLayout(_) => {}
                OpenGlObject::ResourceSet(_) => {}
                OpenGlObject::PipelineLayout(_) => {}
                OpenGlObject::GraphicsPipeline(object) => {
                    self.gl.delete_vertex_array(object.vao);
                    self.gl.delete_program(object.program);
                }
                OpenGlObject::ComputePipeline(object) => self.gl.delete_program(object.program),
                OpenGlObject::RenderTarget(object) => {
                    self.gl.delete_framebuffer(object.framebuffer)
                }
                OpenGlObject::FrameTarget(_) => {}
                OpenGlObject::RenderPass(_) => {}
            }
        }
    }
}

fn texture_mip_extent(base: Extent3d, mip: u32) -> Extent3d {
    Extent3d {
        width: (base.width >> mip).max(1),
        height: (base.height >> mip).max(1),
        depth: (base.depth >> mip).max(1),
    }
}

impl Drop for OpenGlObjects {
    fn drop(&mut self) {
        self.destroy_all();
    }
}

enum OpenGlObject {
    Buffer(BufferObject),
    Texture(TextureObject),
    TextureView(TextureViewObject),
    Sampler(SamplerObject),
    CombinedTextureSampler(CombinedTextureSamplerObject),
    ShaderModule(ShaderModuleObject),
    ResourceLayout(ResourceLayoutObject),
    ResourceSet(ResourceSetObject),
    PipelineLayout(PipelineLayoutObject),
    GraphicsPipeline(GraphicsPipelineObject),
    ComputePipeline(ComputePipelineObject),
    RenderTarget(RenderTargetObject),
    FrameTarget(FrameTargetObject),
    RenderPass(RenderPassObject),
}

impl OpenGlObject {
    fn token(&self) -> BackendToken {
        match self {
            Self::Buffer(object) => object.token,
            Self::Texture(object) => object.token,
            Self::TextureView(object) => object.token,
            Self::Sampler(object) => object.token,
            Self::CombinedTextureSampler(object) => object.token,
            Self::ShaderModule(object) => object.token,
            Self::ResourceLayout(object) => object.token,
            Self::ResourceSet(object) => object.token,
            Self::PipelineLayout(object) => object.token,
            Self::GraphicsPipeline(object) => object.token,
            Self::ComputePipeline(object) => object.token,
            Self::RenderTarget(object) => object.token,
            Self::FrameTarget(object) => object.token,
            Self::RenderPass(object) => object.token,
        }
    }

    fn kind(&self) -> HandleKind {
        match self {
            Self::Buffer(_) => HandleKind::Buffer,
            Self::Texture(_) => HandleKind::Texture,
            Self::TextureView(_) => HandleKind::TextureView,
            Self::Sampler(_) => HandleKind::Sampler,
            Self::CombinedTextureSampler(_) => HandleKind::CombinedTextureSampler,
            Self::ShaderModule(_) => HandleKind::ShaderModule,
            Self::ResourceLayout(_) => HandleKind::ResourceLayout,
            Self::ResourceSet(_) => HandleKind::ResourceSet,
            Self::PipelineLayout(_) => HandleKind::PipelineLayout,
            Self::GraphicsPipeline(_) => HandleKind::GraphicsPipeline,
            Self::ComputePipeline(_) => HandleKind::ComputePipeline,
            Self::RenderTarget(_) => HandleKind::RenderTarget,
            Self::FrameTarget(_) => HandleKind::FrameTarget,
            Self::RenderPass(_) => HandleKind::RenderPass,
        }
    }
}

pub(super) struct BufferObject {
    pub(super) token: BackendToken,
    pub(super) buffer: glow::Buffer,
    pub(super) size: u64,
}

#[allow(dead_code)]
pub(super) struct TextureObject {
    pub(super) token: BackendToken,
    pub(super) texture: glow::Texture,
    pub(super) format: TextureFormat,
    pub(super) extent: Extent3d,
    pub(super) dimension: TextureDimension,
    pub(super) mip_levels: u32,
    pub(super) array_layers: u32,
}

pub(super) fn texture_target(dimension: TextureDimension) -> u32 {
    match dimension {
        TextureDimension::D2 => glow::TEXTURE_2D,
        TextureDimension::D3 => glow::TEXTURE_3D,
        _ => unreachable!("GAL validated texture dimension"),
    }
}

#[allow(dead_code)]
pub(super) struct TextureViewObject {
    pub(super) token: BackendToken,
    pub(super) texture: Handle,
    pub(super) format: TextureFormat,
    pub(super) base_mip: u32,
    pub(super) mip_count: u32,
    pub(super) base_layer: u32,
    pub(super) layer_count: u32,
}

pub(super) struct SamplerObject {
    pub(super) token: BackendToken,
    pub(super) sampler: glow::Sampler,
}

pub(super) struct CombinedTextureSamplerObject {
    pub(super) token: BackendToken,
    pub(super) texture_view: Handle,
    pub(super) sampler: Handle,
}

#[allow(dead_code)]
pub(super) struct ShaderModuleObject {
    pub(super) token: BackendToken,
    pub(super) shader: glow::Shader,
    pub(super) stage: ShaderStage,
    pub(super) sampler_bindings: BTreeMap<String, u32>,
}

#[allow(dead_code)]
pub(super) struct ResourceLayoutObject {
    pub(super) token: BackendToken,
    pub(super) bindings: Vec<ResourceBindingDesc>,
}

pub(super) struct ResourceSetObject {
    pub(super) token: BackendToken,
    pub(super) layout: Handle,
    pub(super) bindings: Vec<ResourceBinding>,
}

pub(super) struct PipelineLayoutObject {
    pub(super) token: BackendToken,
    pub(super) resource_layouts: Vec<Handle>,
}

pub(super) struct GraphicsPipelineObject {
    pub(super) token: BackendToken,
    pub(super) program: glow::Program,
    pub(super) vao: glow::VertexArray,
    pub(super) layout: Handle,
    pub(super) topology: PrimitiveTopology,
    pub(super) cull_mode: CullMode,
    pub(super) front_face: crate::render::vulkanic::resources::FrontFace,
    pub(super) provoking_vertex: crate::render::vulkanic::resources::ProvokingVertex,
    pub(super) raster_y_direction: crate::render::vulkanic::resources::RasterYDirection,
    pub(super) blend: BlendMode,
    pub(super) depth_compare: Option<CompareOp>,
    pub(super) depth_write: bool,
    pub(super) depth_bias: Option<crate::render::vulkanic::resources::DepthBias>,
    pub(super) stencil: Option<crate::render::vulkanic::resources::StencilState>,
}

pub(super) struct ComputePipelineObject {
    pub(super) token: BackendToken,
    pub(super) program: glow::Program,
    pub(super) layout: Handle,
}

#[allow(dead_code)]
pub(super) struct RenderTargetObject {
    pub(super) token: BackendToken,
    pub(super) framebuffer: glow::Framebuffer,
    pub(super) color_views: Vec<Handle>,
    pub(super) depth_stencil_view: Option<Handle>,
    pub(super) extent: Extent3d,
}

#[allow(dead_code)]
pub(super) struct FrameTargetObject {
    pub(super) token: BackendToken,
    pub(super) frame_id: u64,
    pub(super) framebuffer: Option<glow::Framebuffer>,
    pub(super) extent: Extent3d,
    pub(super) color_format: TextureFormat,
}

#[derive(Clone, Copy)]
pub(super) struct PassTargetObject {
    pub(super) framebuffer: Option<glow::Framebuffer>,
    pub(super) extent: Extent3d,
}

#[cfg(test)]
mod tests {
    use super::super::OpenGlBackend;
    use super::*;
    use std::num::NonZeroU32;

    #[test]
    fn triangle_fan_topology_lowers_to_native_opengl_fan_assembly() {
        assert_eq!(glow::TRIANGLE_FAN, topology(PrimitiveTopology::TriangleFan));
    }

    fn fake_framebuffer(id: u32) -> glow::Framebuffer {
        glow::NativeFramebuffer(NonZeroU32::new(id).unwrap())
    }

    fn frame_target_desc(frame_id: u64) -> FrameTargetDesc {
        FrameTargetDesc {
            label: format!("test-frame-target-{frame_id}"),
            frame_id,
            render_target: crate::render::vulkanic::frame::FrameRenderTargetId(frame_id),
            extent: Extent3d {
                width: 64,
                height: 48,
                depth: 1,
            },
            color_format: TextureFormat::Rgba8Unorm,
        }
    }

    #[test]
    fn shader_pack_color_formats_have_explicit_native_copy_formats() {
        let r8 = texture_format(TextureFormat::R8Uint).unwrap();
        assert_eq!(glow::R8UI as i32, r8.internal);
        assert_eq!(1, r8.bytes_per_pixel);
        assert!(r8.integer);
        let rgba16 = texture_format(TextureFormat::Rgba16Float).unwrap();
        assert_eq!(glow::RGBA16F as i32, rgba16.internal);
        assert_eq!(glow::HALF_FLOAT, rgba16.ty);
        assert_eq!(8, rgba16.bytes_per_pixel);
        assert!(!rgba16.integer);

        let r11 = texture_format(TextureFormat::R11fG11fB10f).unwrap();
        assert_eq!(glow::R11F_G11F_B10F as i32, r11.internal);
        assert_eq!(glow::UNSIGNED_INT_10F_11F_11F_REV, r11.ty);
        assert_eq!(4, r11.bytes_per_pixel);

        let r32 = texture_format(TextureFormat::R32Float).unwrap();
        assert_eq!(glow::R32F as i32, r32.internal);
        assert_eq!(glow::FLOAT, r32.ty);
        assert_eq!(4, r32.bytes_per_pixel);

        let rgb16 = texture_format(TextureFormat::Rgb16Float).unwrap();
        assert_eq!(glow::RGB16F as i32, rgb16.internal);
        assert_eq!(6, rgb16.bytes_per_pixel);

        let r8unorm = texture_format(TextureFormat::R8Unorm).unwrap();
        assert_eq!(glow::R8 as i32, r8unorm.internal);
        assert_eq!(1, r8unorm.bytes_per_pixel);

        let snorm = texture_format(TextureFormat::Rgba8Snorm).unwrap();
        assert_eq!(glow::RGBA8_SNORM as i32, snorm.internal);
        assert_eq!(glow::BYTE, snorm.ty);
        assert_eq!(4, snorm.bytes_per_pixel);
    }

    #[test]
    fn borrowed_frame_targets_follow_latest_acquired_framebuffer() {
        let mut backend = match OpenGlBackend::new("MattMC OpenGL frame-target refresh test") {
            Ok(backend) => backend,
            Err(error) => {
                assert!(
                    error.to_string().contains("OpenGL") || error.to_string().contains("EGL"),
                    "unexpected OpenGL bootstrap failure: {error}"
                );
                return;
            }
        };
        let objects = &mut backend.objects;
        let handle = Handle::new(HandleKind::FrameTarget, 1, 1).unwrap();
        let first_framebuffer = fake_framebuffer(101);
        let second_framebuffer = fake_framebuffer(202);

        objects.set_frame_target_framebuffer(1, Some(first_framebuffer));
        objects
            .create(
                handle,
                BackendCreateDesc::FrameTarget(&frame_target_desc(1)),
            )
            .unwrap();
        assert_eq!(
            objects.pass_target(handle).unwrap().framebuffer,
            Some(first_framebuffer)
        );

        objects.set_frame_target_framebuffer(2, Some(second_framebuffer));
        assert_eq!(
            objects.pass_target(handle).unwrap().framebuffer,
            Some(second_framebuffer)
        );

        objects.set_frame_target_framebuffer(3, None);
        assert_eq!(objects.pass_target(handle).unwrap().framebuffer, None);
    }

    #[test]
    fn opengl_shader_source_maps_vulkan_style_vertex_builtins() {
        let source = "layout(set = 0, binding = 0, std140) uniform U { mat4 m; }; int v = gl_VertexIndex; int i = gl_InstanceIndex;";
        let normalized = opengl_shader_source(source);
        assert!(normalized.contains("layout(binding = 0, std140)"));
        assert!(!normalized.contains("set = 0"));
        assert!(normalized.contains("gl_VertexID"));
        assert!(normalized.contains("gl_InstanceID"));
        assert!(!normalized.contains("gl_VertexIndex"));
        assert!(!normalized.contains("gl_InstanceIndex"));
    }

    #[test]
    fn opengl_shader_source_maps_descriptor_sets_to_disjoint_binding_ranges() {
        let source = "\
layout(set = 0, binding = 6, std430) readonly buffer Mesh { vec4 data[]; };
layout(set = 1, binding = 0) uniform utexture3D TerrainVoxelOccupancy;
layout(set=1,binding=3,std140) uniform TerrainVoxelLightMapping { vec4 mapping; };
";
        let normalized = opengl_shader_source(source);
        assert!(normalized.contains("layout(binding = 6, std430)"));
        assert!(
            normalized.contains("layout(binding = 8) uniform usampler3D TerrainVoxelOccupancy;")
        );
        assert!(
            normalized.contains("layout(binding = 11, std140) uniform TerrainVoxelLightMapping")
        );
        assert!(!normalized.contains("set ="));
        assert_eq!(11, opengl_resource_binding_point(1, 3).unwrap());
    }

    #[test]
    fn opengl_shader_source_maps_colored_voxel_sampler_pair() {
        let source = "\
layout(set = 1, binding = 1) uniform texture3D TerrainColoredVoxelLight;
layout(set = 1, binding = 2) uniform sampler TerrainVoxelLightSampler;
void main() { vec3 light = texture(sampler3D(TerrainColoredVoxelLight, TerrainVoxelLightSampler), vec3(0.5)).rgb; }
";
        let normalized = opengl_shader_source(source);
        assert!(
            normalized.contains("layout(binding = 9) uniform sampler3D TerrainColoredVoxelLight;")
        );
        assert!(!normalized.contains("TerrainVoxelLightSampler"));
        assert!(normalized.contains("texture(TerrainColoredVoxelLight, vec3(0.5))"));
    }

    #[test]
    fn opengl_shader_source_maps_semantic_terrain_atlas_sampler_pair() {
        let source = "\
layout(set = 0, binding = 2) uniform texture2D TerrainAtlasColor;
layout(set = 0, binding = 3) uniform sampler TerrainAtlasSampler;
void main() { vec4 color = texture(sampler2D(TerrainAtlasColor, TerrainAtlasSampler), vec2(0.5)); }
";
        let normalized = opengl_shader_source(source);
        assert!(normalized.contains("layout(binding = 2) uniform sampler2D TerrainAtlasColor;"));
        assert!(!normalized.contains("TerrainAtlasSampler"));
        assert!(normalized.contains("texture(TerrainAtlasColor, vec2(0.5))"));
    }

    #[test]
    fn opengl_shader_source_maps_distant_horizons_lightmap_sampler_pair() {
        let source = "\
layout(set = 1, binding = 0) uniform texture2D LightmapTexture;
layout(set = 1, binding = 1) uniform sampler LightmapSampler;
void main() { vec4 color = texture(sampler2D(LightmapTexture, LightmapSampler), vec2(0.5)); }
";
        let normalized = opengl_shader_source(source);
        assert!(normalized.contains("layout(binding = 8) uniform sampler2D LightmapTexture;"));
        assert!(!normalized.contains("LightmapSampler"));
        assert!(normalized.contains("texture(LightmapTexture, vec2(0.5))"));
        assert!(
            sampler_uniform_names(8)
                .iter()
                .any(|name| name == "LightmapTexture")
        );
    }

    #[test]
    fn opengl_shader_source_maps_separate_texture_sampler_pair() {
        let source = "\
layout(set = 0, binding = 1) uniform texture2D Tex0;
layout(set = 0, binding = 2) uniform sampler Samp0;
void main() { vec4 color = texture(sampler2D(Tex0, Samp0), vec2(0.5)); }";
        let normalized = opengl_shader_source(source);
        assert!(normalized.contains("layout(binding = 1) uniform sampler2D Tex0;"));
        assert!(!normalized.contains("uniform sampler Samp0"));
        assert!(!normalized.contains("sampler2D(Tex0, Samp0)"));
        assert!(normalized.contains("texture(Tex0, vec2(0.5))"));
    }

    #[test]
    fn opengl_shader_source_maps_shadow_composite_texture_pairs() {
        let source = "\
layout(set = 0, binding = 3) uniform texture2D WorldPositionTex;
layout(set = 0, binding = 4) uniform texture2D ShadowDepthTex;
layout(set = 0, binding = 5) uniform sampler Samp0;
void main() {
    vec4 p = texture(sampler2D(WorldPositionTex, Samp0), vec2(0.5));
    float d = texture(sampler2D(ShadowDepthTex, Samp0), vec2(0.5)).r;
}";
        let normalized = opengl_shader_source(source);
        assert!(normalized.contains("uniform sampler2D WorldPositionTex;"));
        assert!(normalized.contains("uniform sampler2D ShadowDepthTex;"));
        assert!(!normalized.contains("uniform sampler Samp0"));
        assert!(normalized.contains("texture(WorldPositionTex, vec2(0.5))"));
        assert!(normalized.contains("texture(ShadowDepthTex, vec2(0.5)).r"));
    }

    #[test]
    fn opengl_shader_source_maps_main_depth_composite_texture_pair() {
        let source = "\
layout(set = 0, binding = 0) uniform texture2D Tex0;
layout(set = 0, binding = 4) uniform texture2D MainDepthTex;
layout(set = 0, binding = 5) uniform sampler Samp0;
void main() {
    float depth = texture(sampler2D(MainDepthTex, Samp0), vec2(0.5)).r;
    vec4 color = texture(sampler2D(Tex0, Samp0), vec2(0.5));
}";
        let normalized = opengl_shader_source(source);
        assert!(normalized.contains("uniform sampler2D MainDepthTex;"));
        assert!(!normalized.contains("uniform sampler Samp0"));
        assert!(!normalized.contains("sampler2D(MainDepthTex, Samp0)"));
        assert!(normalized.contains("texture(MainDepthTex, vec2(0.5)).r"));
        assert!(
            sampler_uniform_names(4)
                .iter()
                .any(|name| name == "MainDepthTex")
        );
    }

    #[test]
    fn opengl_sampler_aliases_cover_existing_tex0_bindings() {
        assert!(sampler_uniform_names(0).iter().any(|name| name == "Tex0"));
        assert!(sampler_uniform_names(1).iter().any(|name| name == "Tex0"));
    }

    #[test]
    fn opengl_sampler_layout_parser_tracks_declared_bindings() {
        let source = "\
layout(binding = 0) uniform sampler2D Tex0;
layout(binding=1) uniform sampler2D NormalTex;
layout(binding = 4) uniform sampler2D ShadowDepthTex;
";
        let bindings = parse_sampler2d_layout_bindings(source);
        assert_eq!(bindings.get("Tex0"), Some(&0));
        assert_eq!(bindings.get("NormalTex"), Some(&1));
        assert_eq!(bindings.get("ShadowDepthTex"), Some(&4));
    }

    #[test]
    fn opengl_program_interface_aliases_include_owned_mesh_blocks() {
        assert!(uniform_block_names(1).iter().any(|name| name == "GuiMeshFrame"));
        assert!(
            storage_block_names(0)
                .iter()
                .any(|name| name == "WorldMeshVertices")
        );
        assert!(
            storage_block_names(0)
                .iter()
                .any(|name| name == "GuiMeshVertices")
        );
        assert!(
            storage_block_names(1)
                .iter()
                .any(|name| name == "WorldMeshInstances")
        );
        assert!(
            storage_block_names(6)
                .iter()
                .any(|name| name == "CompositeShadowUniforms")
        );
    }

    #[test]
    fn opengl_depth24_stencil8_preserves_packed_attachment_format() {
        let format = texture_format(TextureFormat::Depth24Stencil8).unwrap();
        assert_eq!(format.internal, glow::DEPTH24_STENCIL8 as i32);
        assert_eq!(format.external, glow::DEPTH_STENCIL);
        assert_eq!(format.ty, glow::UNSIGNED_INT_24_8);
        assert_eq!(format.bytes_per_pixel, 4);
        assert!(!format.integer);
        // Host transfers still require an explicit aspect contract in GAL.
        assert_eq!(TextureFormat::Depth24Stencil8.copy_bytes_per_texel(), None);
    }
}

#[allow(dead_code)]
pub(super) struct RenderPassObject {
    pub(super) token: BackendToken,
    pub(super) target: Handle,
    pub(super) color_formats: Vec<TextureFormat>,
    pub(super) depth_format: Option<TextureFormat>,
}

pub(super) struct GlTextureFormat {
    pub(super) internal: i32,
    pub(super) external: u32,
    pub(super) ty: u32,
    pub(super) bytes_per_pixel: u32,
    pub(super) integer: bool,
}

pub(super) fn texture_format(format: TextureFormat) -> GalResult<GlTextureFormat> {
    match format {
        TextureFormat::Rgba8Unorm => Ok(GlTextureFormat {
            internal: glow::RGBA8 as i32,
            external: glow::RGBA,
            ty: glow::UNSIGNED_BYTE,
            bytes_per_pixel: 4,
            integer: false,
        }),
        TextureFormat::Rgba16Float => Ok(GlTextureFormat {
            internal: glow::RGBA16F as i32,
            external: glow::RGBA,
            ty: glow::HALF_FLOAT,
            bytes_per_pixel: 8,
            integer: false,
        }),
        TextureFormat::Depth32Float => Ok(GlTextureFormat {
            internal: glow::DEPTH_COMPONENT32F as i32,
            external: glow::DEPTH_COMPONENT,
            ty: glow::FLOAT,
            bytes_per_pixel: 4,
            integer: false,
        }),
        TextureFormat::Depth24Stencil8 => Ok(GlTextureFormat {
            internal: glow::DEPTH24_STENCIL8 as i32,
            external: glow::DEPTH_STENCIL,
            ty: glow::UNSIGNED_INT_24_8,
            bytes_per_pixel: 4,
            integer: false,
        }),
        TextureFormat::R8Uint => Ok(GlTextureFormat {
            internal: glow::R8UI as i32,
            external: glow::RED_INTEGER,
            ty: glow::UNSIGNED_BYTE,
            bytes_per_pixel: 1,
            integer: true,
        }),
        TextureFormat::R11fG11fB10f => Ok(GlTextureFormat {
            internal: glow::R11F_G11F_B10F as i32,
            external: glow::RGB,
            ty: glow::UNSIGNED_INT_10F_11F_11F_REV,
            bytes_per_pixel: 4,
            integer: false,
        }),
        TextureFormat::R32Float => Ok(GlTextureFormat {
            internal: glow::R32F as i32,
            external: glow::RED,
            ty: glow::FLOAT,
            bytes_per_pixel: 4,
            integer: false,
        }),
        TextureFormat::Rgb16Float => Ok(GlTextureFormat {
            internal: glow::RGB16F as i32,
            external: glow::RGB,
            ty: glow::HALF_FLOAT,
            bytes_per_pixel: 6,
            integer: false,
        }),
        TextureFormat::R8Unorm => Ok(GlTextureFormat {
            internal: glow::R8 as i32,
            external: glow::RED,
            ty: glow::UNSIGNED_BYTE,
            bytes_per_pixel: 1,
            integer: false,
        }),
        TextureFormat::Rgba8Snorm => Ok(GlTextureFormat {
            internal: glow::RGBA8_SNORM as i32,
            external: glow::RGBA,
            ty: glow::BYTE,
            bytes_per_pixel: 4,
            integer: false,
        }),
        TextureFormat::Bgra8Unorm => Err(GalError::backend(
            format!("OpenGL texture format {format:?} is not supported in the isolated path"),
        )),
    }
}

pub(super) fn filter(filter: SamplerFilter) -> i32 {
    match filter {
        SamplerFilter::Nearest => glow::NEAREST as i32,
        SamplerFilter::Linear => glow::LINEAR as i32,
    }
}

fn opengl_shader_source(source: &str) -> String {
    normalize_vulkan_resource_set_layouts(source)
        .replace("uniform texture2D Tex0;", "uniform sampler2D Tex0;")
        .replace("uniform texture2D tex0;", "uniform sampler2D tex0;")
        .replace(
            "uniform texture2D TerrainAtlasColor;",
            "uniform sampler2D TerrainAtlasColor;",
        )
        .replace(
            "uniform texture2D AlbedoTex;",
            "uniform sampler2D AlbedoTex;",
        )
        .replace(
            "uniform texture2D NormalTex;",
            "uniform sampler2D NormalTex;",
        )
        .replace(
            "uniform texture2D MaterialLightTex;",
            "uniform sampler2D MaterialLightTex;",
        )
        .replace(
            "uniform texture2D WorldPositionTex;",
            "uniform sampler2D WorldPositionTex;",
        )
        .replace(
            "uniform texture2D ShadowDepthTex;",
            "uniform sampler2D ShadowDepthTex;",
        )
        .replace(
            "uniform texture2D MainDepthTex;",
            "uniform sampler2D MainDepthTex;",
        )
        .replace(
            "uniform texture2D LightmapTexture;",
            "uniform sampler2D LightmapTexture;",
        )
        .replace(
            "uniform texture2D vulkanic_source_dh_atlas_texture;",
            "uniform sampler2D vulkanic_source_dh_atlas_texture;",
        )
        .replace(
            "uniform utexture3D TerrainVoxelOccupancy;",
            "uniform usampler3D TerrainVoxelOccupancy;",
        )
        .replace(
            "uniform texture3D TerrainColoredVoxelLight;",
            "uniform sampler3D TerrainColoredVoxelLight;",
        )
        .replace("layout(binding = 2) uniform sampler Samp0;\n", "")
        .replace("layout(binding=2) uniform sampler Samp0;\n", "")
        .replace("layout(binding = 3) uniform sampler Samp0;\n", "")
        .replace("layout(binding=3) uniform sampler Samp0;\n", "")
        .replace(
            "layout(binding = 3) uniform sampler TerrainAtlasSampler;\n",
            "",
        )
        .replace(
            "layout(binding=3) uniform sampler TerrainAtlasSampler;\n",
            "",
        )
        .replace("layout(binding = 5) uniform sampler Samp0;\n", "")
        .replace("layout(binding=5) uniform sampler Samp0;\n", "")
        .replace("layout(binding = 9) uniform sampler LightmapSampler;\n", "")
        .replace("layout(binding=9) uniform sampler LightmapSampler;\n", "")
        .replace(
            "layout(binding = 17) uniform sampler vulkanic_source_dh_atlas_sampler;\n",
            "",
        )
        .replace(
            "layout(binding=17) uniform sampler vulkanic_source_dh_atlas_sampler;\n",
            "",
        )
        .replace("layout(binding = 1) uniform sampler samp0;\n", "")
        .replace("layout(binding=1) uniform sampler samp0;\n", "")
        .replace(
            "layout(binding = 10) uniform sampler TerrainVoxelLightSampler;\n",
            "",
        )
        .replace(
            "layout(binding=10) uniform sampler TerrainVoxelLightSampler;\n",
            "",
        )
        .replace("sampler2D(Tex0, Samp0)", "Tex0")
        .replace("sampler2D(tex0, samp0)", "tex0")
        .replace(
            "sampler2D(TerrainAtlasColor, TerrainAtlasSampler)",
            "TerrainAtlasColor",
        )
        .replace("sampler2D(AlbedoTex, Samp0)", "AlbedoTex")
        .replace("sampler2D(NormalTex, Samp0)", "NormalTex")
        .replace("sampler2D(MaterialLightTex, Samp0)", "MaterialLightTex")
        .replace("sampler2D(WorldPositionTex, Samp0)", "WorldPositionTex")
        .replace("sampler2D(ShadowDepthTex, Samp0)", "ShadowDepthTex")
        .replace("sampler2D(MainDepthTex, Samp0)", "MainDepthTex")
        .replace(
            "sampler2D(LightmapTexture, LightmapSampler)",
            "LightmapTexture",
        )
        .replace(
            "sampler2D(vulkanic_source_dh_atlas_texture, vulkanic_source_dh_atlas_sampler)",
            "vulkanic_source_dh_atlas_texture",
        )
        .replace(
            "sampler3D(TerrainColoredVoxelLight, TerrainVoxelLightSampler)",
            "TerrainColoredVoxelLight",
        )
        .replace(
            "usampler3D(TerrainVoxelOccupancy, TerrainVoxelLightSampler)",
            "TerrainVoxelOccupancy",
        )
        .replace("gl_VertexIndex", "gl_VertexID")
        .replace("gl_InstanceIndex", "gl_InstanceID")
}

fn normalize_vulkan_resource_set_layouts(source: &str) -> String {
    let mut normalized = String::with_capacity(source.len());
    let mut remaining = source;
    while let Some(layout_start) = remaining.find("layout(") {
        normalized.push_str(&remaining[..layout_start]);
        let layout = &remaining[layout_start..];
        let Some(layout_end) = layout.find(')') else {
            normalized.push_str(layout);
            return normalized;
        };
        let qualifiers = &layout[7..layout_end];
        let mut set = None;
        let mut binding = None;
        let mut retained = Vec::new();
        for qualifier in qualifiers.split(',') {
            let qualifier = qualifier.trim();
            let Some((name, value)) = qualifier.split_once('=') else {
                retained.push(qualifier);
                continue;
            };
            match name.trim() {
                "set" => set = value.trim().parse::<u32>().ok(),
                "binding" => binding = value.trim().parse::<u32>().ok(),
                _ => retained.push(qualifier),
            }
        }
        if let (Some(set), Some(binding)) = (set, binding) {
            if let Ok(binding_point) = opengl_resource_binding_point(set, binding) {
                let mut rebuilt = vec![format!("binding = {binding_point}")];
                rebuilt.extend(retained.into_iter().map(str::to_string));
                normalized.push_str("layout(");
                normalized.push_str(&rebuilt.join(", "));
                normalized.push(')');
            } else {
                normalized.push_str(&layout[..=layout_end]);
            }
        } else {
            normalized.push_str(&layout[..=layout_end]);
        }
        remaining = &layout[layout_end + 1..];
    }
    normalized.push_str(remaining);
    normalized
}

fn parse_sampler2d_layout_bindings(source: &str) -> BTreeMap<String, u32> {
    let mut bindings = BTreeMap::new();
    for line in source.lines() {
        let line = line.trim();
        if !line.starts_with("layout(") {
            continue;
        }
        let Some(declaration) = [
            "uniform sampler2D ",
            "uniform sampler3D ",
            "uniform usampler3D ",
            "uniform isampler3D ",
        ]
        .into_iter()
        .find(|declaration| line.contains(declaration)) else {
            continue;
        };
        let Some(binding_start) = line.find("binding") else {
            continue;
        };
        let Some(eq) = line[binding_start..].find('=') else {
            continue;
        };
        let number_start = binding_start + eq + 1;
        let number = line[number_start..]
            .trim_start()
            .chars()
            .take_while(|ch| ch.is_ascii_digit())
            .collect::<String>();
        let Ok(binding) = number.parse::<u32>() else {
            continue;
        };
        let Some(name_start) = line.find(declaration) else {
            continue;
        };
        let name = line[name_start + declaration.len()..]
            .trim()
            .trim_end_matches(';')
            .split(['[', ' ', '\t'])
            .next()
            .unwrap_or("")
            .trim();
        if !name.is_empty() {
            bindings.insert(name.to_string(), binding);
        }
    }
    bindings
}

pub(super) fn min_filter(desc: &SamplerDesc) -> i32 {
    filter(desc.min_filter)
}

pub(super) fn address(mode: SamplerAddressMode) -> i32 {
    match mode {
        SamplerAddressMode::ClampToEdge => glow::CLAMP_TO_EDGE as i32,
        SamplerAddressMode::Repeat => glow::REPEAT as i32,
        SamplerAddressMode::MirroredRepeat => glow::MIRRORED_REPEAT as i32,
    }
}

pub(super) fn topology(topology: PrimitiveTopology) -> u32 {
    match topology {
        PrimitiveTopology::Points => glow::POINTS,
        PrimitiveTopology::Lines => glow::LINES,
        PrimitiveTopology::Triangles => glow::TRIANGLES,
        PrimitiveTopology::TriangleFan => glow::TRIANGLE_FAN,
    }
}

fn uniform_block_names(binding: u32) -> Vec<String> {
    match binding {
        0 => vec![
            "GuiRect".to_string(),
            "GuiSpriteBatch".to_string(),
            "WorldLineBatch".to_string(),
            "CrackQuadBatch".to_string(),
            "WorldBorderBatch".to_string(),
            "DynamicTransforms".to_string(),
            "Projection".to_string(),
        ],
        1 => vec![
            "GuiMeshFrame".to_string(),
            "WorldMeshInstance".to_string(),
            "DistantHorizonsLodFrame".to_string(),
            "Uniforms1".to_string(),
        ],
        11 => vec![
            "TerrainVoxelLightMapping".to_string(),
            "Uniforms11".to_string(),
        ],
        _ => vec![format!("Uniforms{binding}")],
    }
}

fn storage_block_names(binding: u32) -> Vec<String> {
    match binding {
        0 => vec![
            "WorldMaterialBatch".to_string(),
            "WorldMeshVertices".to_string(),
            // Rust-owned GUI mesh rasterization uses the same explicit GAL
            // storage binding, but deliberately has a separate shader block
            // name from world geometry. Keep this OpenGL-only interface
            // reconstruction private to the backend.
            "GuiMeshVertices".to_string(),
            "DistantHorizonsLodVertices".to_string(),
            "Storage0".to_string(),
        ],
        1 => vec!["WorldMeshInstances".to_string(), "Storage1".to_string()],
        6 => vec![
            "CompositeShadowUniforms".to_string(),
            "ShaderCompositeUniforms".to_string(),
            "Storage6".to_string(),
        ],
        _ => vec![format!("Storage{binding}")],
    }
}

pub(super) fn sampler_uniform_names(binding: u32) -> Vec<String> {
    match binding {
        0 => vec![
            "Sampler0".to_string(),
            "tex0".to_string(),
            "Tex0".to_string(),
            "AlbedoTex".to_string(),
        ],
        1 => vec![
            "Sampler0".to_string(),
            "tex0".to_string(),
            "Tex0".to_string(),
            "Sampler1".to_string(),
            "tex1".to_string(),
            "NormalTex".to_string(),
        ],
        2 => vec![
            "Samp0".to_string(),
            "Sampler2".to_string(),
            "tex2".to_string(),
            "MaterialLightTex".to_string(),
        ],
        3 => vec![
            "Samp0".to_string(),
            "Sampler3".to_string(),
            "tex3".to_string(),
            "WorldPositionTex".to_string(),
        ],
        4 => vec![
            "Samp0".to_string(),
            "Sampler4".to_string(),
            "tex4".to_string(),
            "ShadowDepthTex".to_string(),
            "DepthTex".to_string(),
            "MainDepthTex".to_string(),
        ],
        8 => vec!["LightmapTexture".to_string(), "Sampler8".to_string()],
        _ => vec![format!("Sampler{binding}"), format!("tex{binding}")],
    }
}

pub(super) fn shader_stage(stage: ShaderStage) -> GalResult<u32> {
    match stage {
        ShaderStage::Vertex => Ok(glow::VERTEX_SHADER),
        ShaderStage::Fragment => Ok(glow::FRAGMENT_SHADER),
        ShaderStage::Compute => Ok(glow::COMPUTE_SHADER),
        ShaderStage::Geometry | ShaderStage::TessControl | ShaderStage::TessEvaluation => {
            Err(GalError::backend(format!(
                "OpenGL shader stage {stage:?} is not supported in the isolated path"
            )))
        }
    }
}

fn expected(kind: &str, handle: Handle) -> GalError {
    GalError::backend(format!(
        "expected OpenGL {kind} for handle 0x{:016x}",
        handle.raw()
    ))
}
