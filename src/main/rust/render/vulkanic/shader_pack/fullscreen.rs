//! Backend-neutral preparation for Rust-owned source-derived fullscreen passes.
//!
//! This module closes the semantic gap between separately owned source color
//! targets and other owned inputs (depth, shadow, material, and pack assets),
//! then compiles/binds an explicit fullscreen pass. Command recording remains
//! separate so route selection and frame ownership cannot hide here.

use std::collections::BTreeMap;

use crate::render::vulkanic::commands::{
    AttachmentLoadOp, AttachmentStoreOp, CommandOp, PassAttachment, ResourceBarrier,
    TextureUsageState,
};
use crate::render::vulkanic::error::{GalError, GalResult};
use crate::render::vulkanic::gal::VulkanicGal;
use crate::render::vulkanic::handles::Handle;
use crate::render::vulkanic::resources::{
    AccessFlags, BlendMode, BufferDesc, BufferUsage, CombinedTextureSamplerDesc, CullMode,
    GraphicsPipelineDesc, MemoryDomain, PipelineLayoutDesc, PipelineStageFlags, PrimitiveTopology,
    QueueClass, RenderPassDesc, RenderTargetDesc, ResourceBinding, ResourceBindingDesc,
    ResourceBindingKind, ResourceLayoutDesc, ResourceSetDesc, SamplerAddressMode, SamplerDesc,
    SamplerFilter, ShaderCodeFormat, ShaderModuleDesc, ShaderStage, TextureDesc, TextureDimension,
    TextureFormat, TextureUsage, TextureViewDesc,
};

use super::lowering::TerrainSourceOpaqueResourceKind;
use super::programs::{shader_stage_code_for_backend, LoweredFullscreenSourceProgram};
use super::source_targets::{
    prepare_fullscreen_source_color_resources, resolve_fullscreen_source_color_attachments,
    source_color_clear_color, FullscreenSourceColorAttachment, ShaderPackColorBootstrapClearValues,
    ShaderPackColorFramePlan, ShaderPackColorTargetManifest, ShaderPackColorTargets,
    ShaderPackSourceColorResources,
};
use super::terrain_source_resources::{TerrainSourceOwnedResourceSet, TerrainSourceResourceRole};

/// Stable, backend-neutral compatibility identity for one final source-copy
/// binding. `render_target` is the acquired frame slot identity supplied by
/// GAL, not a native image/view or a transient GAL handle.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct SourceFinalOutputIdentity {
    pub world_generation: u64,
    pub shader_pack_generation: u64,
    /// The source main-depth generation is part of the compatibility contract
    /// for the private overlay target. A swapchain slot can be reused across a
    /// resize or graph rebuild, but it may not retain a target that borrows an
    /// obsolete depth view.
    pub graph_generation: u64,
    pub source_role: TerrainSourceResourceRole,
    pub render_target: crate::render::vulkanic::frame::FrameRenderTargetId,
    pub extent: [u32; 3],
    pub color_format: crate::render::vulkanic::resources::TextureFormat,
}

/// A frame-local reservation into the persistent final-output cache. A newly
/// staged entry is removed on a later frame-recording failure; confirmed
/// entries remain owned by the cache until their source generation retires.
#[derive(Clone, Debug)]
pub(crate) struct SourceFinalOutputReservation {
    identity: SourceFinalOutputIdentity,
    newly_staged: bool,
}

impl SourceFinalOutputReservation {
    pub(crate) fn identity(&self) -> &SourceFinalOutputIdentity {
        &self.identity
    }

    pub(crate) fn newly_staged(&self) -> bool {
        self.newly_staged
    }
}

impl SourceFinalOutputIdentity {
    const fn extent3d(&self) -> crate::render::vulkanic::resources::Extent3d {
        crate::render::vulkanic::resources::Extent3d {
            width: self.extent[0],
            height: self.extent[1],
            depth: self.extent[2],
        }
    }
}

/// Persistent final-copy bindings keyed only by semantic compatibility facts.
/// In particular, swapchain image rotation is represented by GAL's stable
/// `FrameRenderTargetId`, never a native image/view or transient GAL handle.
#[derive(Default)]
pub(crate) struct SourceFinalOutputCache {
    plans: BTreeMap<SourceFinalOutputIdentity, SourceFinalOutputPlan>,
}

/// Private Rust-owned color target for source-frame world overlays. It pairs
/// an owned color image with an already-owned source depth view, so overlays
/// can retain depth testing without pretending an acquired swapchain image
/// has a depth attachment. The caller controls pass ordering and final
/// presentation; this type only owns the explicit target resources.
#[derive(Debug)]
pub(crate) struct SourceOverlayTarget {
    target: Handle,
    color_texture: Handle,
    color_view: Handle,
    depth_view: Handle,
    extent: crate::render::vulkanic::resources::Extent3d,
    color_format: TextureFormat,
}

impl SourceOverlayTarget {
    pub(crate) fn stage(
        gal: &mut VulkanicGal,
        label: &str,
        extent: crate::render::vulkanic::resources::Extent3d,
        color_format: TextureFormat,
        depth_view: Handle,
    ) -> GalResult<Self> {
        if extent.width == 0 || extent.height == 0 || extent.depth != 1 {
            return Err(GalError::invalid_argument(
                "source overlay target requires a non-zero two-dimensional extent",
            ));
        }
        let mut created = Vec::new();
        let result = (|| -> GalResult<Self> {
            let color_texture = gal.create_texture(TextureDesc {
                label: format!("{label}.color"),
                dimension: TextureDimension::D2,
                format: color_format,
                extent,
                mip_levels: 1,
                array_layers: 1,
                // The normal source path writes this target as a color attachment
                // and samples it for the final copy. Bounded source diagnostics may
                // additionally read it back after the source chain completes.
                usages: vec![
                    TextureUsage::ColorAttachment,
                    TextureUsage::Sampled,
                    TextureUsage::TransferSrc,
                ],
            })?;
            created.push(color_texture);
            let color_view = gal.create_texture_view(TextureViewDesc {
                label: format!("{label}.color-view"),
                texture: color_texture,
                format: color_format,
                base_mip: 0,
                mip_count: 1,
                base_layer: 0,
                layer_count: 1,
            })?;
            created.push(color_view);
            let target = gal.create_render_target(RenderTargetDesc {
                label: format!("{label}.target"),
                color_views: vec![color_view],
                depth_stencil_view: Some(depth_view),
                extent,
            })?;
            created.push(target);
            Ok(Self {
                target,
                color_texture,
                color_view,
                depth_view,
                extent,
                color_format,
            })
        })();
        if result.is_err() {
            for handle in created.into_iter().rev() {
                let _ = gal.destroy(handle);
            }
        }
        result
    }

    pub(crate) const fn target(&self) -> Handle {
        self.target
    }

    pub(crate) const fn color_view(&self) -> Handle {
        self.color_view
    }

    pub(crate) const fn color_texture(&self) -> Handle {
        self.color_texture
    }

    pub(crate) const fn depth_view(&self) -> Handle {
        self.depth_view
    }

    pub(crate) const fn extent(&self) -> crate::render::vulkanic::resources::Extent3d {
        self.extent
    }

    pub(crate) const fn color_format(&self) -> TextureFormat {
        self.color_format
    }

    pub(crate) fn destroy(self, gal: &mut VulkanicGal) {
        for handle in [self.target, self.color_view, self.color_texture] {
            let _ = gal.destroy(handle);
        }
    }
}

/// One fully-owned source fullscreen pass input/output contract. It contains
/// only semantic roles and opaque GAL handles; there are no Iris attachment
/// numbers, GL units, backend descriptors, or route decisions here.
#[derive(Debug)]
pub(crate) struct PreparedFullscreenSourcePass {
    pub program_identity: String,
    pub inputs: TerrainSourceOwnedResourceSet,
    /// Exact outputs of this stage in lowered GLSL location order. Their
    /// semantic shader-pack destinations may be sparse because `DRAWBUFFERS`
    /// maps a fragment output ordinal to a named source color target.
    color_targets: Vec<FullscreenSourceColorAttachment>,
    /// The subset actually written by this source program. This is used for
    /// feedback validation and semantic diagnostics, not target layout.
    pub outputs: Vec<FullscreenSourceColorAttachment>,
    color_resources: ShaderPackSourceColorResources,
}

/// Static GAL objects for a prepared fullscreen source pass. Per-frame UBO
/// writes and pass commands are intentionally a later executor concern; this
/// owner proves that source-declared fullscreen stages compile through the
/// same explicit VulkanicGAL pipeline path on Vulkan and OpenGL.
#[derive(Debug)]
pub(crate) struct CompiledFullscreenSourcePass {
    pub target: Handle,
    pub pass: Handle,
    pub source_data_layout: Handle,
    pub pack_resources_layout: Handle,
    pub pipeline_layout: Handle,
    pub vertex_shader: Handle,
    pub fragment_shader: Handle,
    pub pipeline: Handle,
}

/// Resource sets and owned buffers required by one compiled source pass.
/// Dynamic data is written by a later command recorder; this type establishes
/// the exact set-zero/set-one ABI without exposing backend descriptors.
#[derive(Debug)]
pub(crate) struct BoundFullscreenSourcePass {
    pub source_data_set: Handle,
    pub pack_resources_set: Handle,
    pub texture_transform_buffer: Handle,
    pub scalar_uniform_buffer: Option<Handle>,
}

/// Per-execution state for one compiled source fullscreen pass. The caller
/// declares every prior semantic usage; this recorder does not infer an Iris,
/// OpenGL, or backend target state. All pass outputs leave shader-readable.
#[derive(Debug)]
pub(crate) struct FullscreenSourcePassFrame {
    pub texture_transforms: Vec<u8>,
    pub scalar_uniforms: Vec<u8>,
    pub texture_transform_before: TextureUsageState,
    pub scalar_uniform_before: Option<TextureUsageState>,
    /// Frame semantic clear values for pack-declared color targets.
    pub clear_values: ShaderPackColorBootstrapClearValues,
    /// Exact lowered GLSL output-location order of the compiled render target.
    pub color_attachment_before: Vec<TextureUsageState>,
}

/// Complete Rust-owned execution preparation for one source-derived
/// fullscreen stage. Staging is atomic across semantic preparation, GAL
/// compilation, and resource-set binding; this reusable owner is shared by
/// future vanilla and Distant Horizons scheduling without selecting either
/// route here.
#[derive(Debug)]
pub(crate) struct FullscreenSourceExecutionPlan {
    prepared: PreparedFullscreenSourcePass,
    compiled: CompiledFullscreenSourcePass,
    bound: BoundFullscreenSourcePass,
}

/// One transient Rust-owned presentation node for a completed source frame.
/// The selected pack's `final` source stage first writes its declared named
/// color output; this node samples that semantic result and writes the
/// acquired backend-owned frame target. It contains only GAL handles and a
/// pack-level role, never a Java/Iris target, program, or native handle.
#[derive(Debug)]
pub(crate) struct SourceFinalOutputPlan {
    identity: SourceFinalOutputIdentity,
    source_role: TerrainSourceResourceRole,
    frame_target: Handle,
    frame_format: TextureFormat,
    overlay: SourceOverlayTarget,
    overlay_pass: Handle,
    source_copy: SourceColorCopyPlan,
    present_copy: SourceColorCopyPlan,
}

/// Frame-local diagnostic mirror of the final present copy. It deliberately
/// uses the identical owned source and copy program as the acquired-target
/// presentation path, so a capture can prove the displayed image without
/// reading back a swapchain image or exposing backend objects.
#[derive(Debug)]
pub(crate) struct SourceFinalPresentationCapture {
    target: Handle,
    color_texture: Handle,
    color_view: Handle,
    format: TextureFormat,
    copy: SourceColorCopyPlan,
}

/// One explicit sampled-color copy. It is deliberately private to the
/// source-frame presenter: callers only compose semantic source stages and
/// never see the target's backend state or native identity.
#[derive(Debug)]
struct SourceColorCopyPlan {
    target: Handle,
    color_attachment: Handle,
    depth_attachment: Option<Handle>,
    pass: Handle,
    resource_layout: Handle,
    pipeline_layout: Handle,
    resource_set: Handle,
    sampler: Handle,
    combined_sampler: Handle,
    vertex_shader: Handle,
    fragment_shader: Handle,
    pipeline: Handle,
}

struct SourceFinalOutputStageInput {
    identity: SourceFinalOutputIdentity,
    source_role: TerrainSourceResourceRole,
    source_view: Handle,
    source_name: String,
    frame_target: Handle,
    color_attachment: Handle,
    frame_format: crate::render::vulkanic::resources::TextureFormat,
    source_format: crate::render::vulkanic::resources::TextureFormat,
    main_depth_view: Handle,
}

impl FullscreenSourceExecutionPlan {
    /// Semantic outputs written by this stage in the exact source declaration
    /// order. This is intentionally diagnostic-facing only: callers can
    /// correlate a bounded readback to a source-stage output without learning
    /// attachment numbers or backend resource identity.
    pub(crate) fn outputs(&self) -> &[FullscreenSourceColorAttachment] {
        &self.prepared.outputs
    }

    pub(crate) fn stage(
        gal: &mut VulkanicGal,
        program: &LoweredFullscreenSourceProgram,
        manifest: &ShaderPackColorTargetManifest,
        targets: &ShaderPackColorTargets,
        external_inputs: impl IntoIterator<Item = TerrainSourceOwnedResourceSet>,
        extent: crate::render::vulkanic::resources::Extent3d,
    ) -> GalResult<Self> {
        if targets.identity.extent != extent {
            return Err(GalError::invalid_argument(format!(
                "fullscreen source target extent {:?} does not match requested pass extent {:?}",
                targets.identity.extent, extent
            )));
        }
        let prepared = PreparedFullscreenSourcePass::prepare(
            gal,
            program,
            manifest,
            targets,
            external_inputs,
        )?;
        let compiled = match prepared.compile(gal, program, extent) {
            Ok(compiled) => compiled,
            Err(error) => {
                prepared.destroy(gal);
                return Err(error);
            }
        };
        let bound = match prepared.bind_resources(gal, program, &compiled) {
            Ok(bound) => bound,
            Err(error) => {
                compiled.destroy(gal);
                prepared.destroy(gal);
                return Err(error);
            }
        };
        Ok(Self {
            prepared,
            compiled,
            bound,
        })
    }

    pub(crate) fn append_draw(
        &self,
        program: &LoweredFullscreenSourceProgram,
        frame: FullscreenSourcePassFrame,
        operations: &mut Vec<CommandOp>,
    ) -> GalResult<()> {
        self.prepared
            .append_draw(program, &self.compiled, &self.bound, frame, operations)
    }

    /// Records one source stage through the submit-confirmed semantic color
    /// scheduler. This is the route-facing API for both vanilla and Distant
    /// Horizons source chains: callers cannot invent color attachment states
    /// or sample current/feedback images before the scheduler established
    /// them. Native target identity remains entirely inside the GAL objects.
    pub(crate) fn append_draw_with_color_frame(
        &self,
        program: &LoweredFullscreenSourceProgram,
        color_frame: &mut ShaderPackColorFramePlan,
        mut frame: FullscreenSourcePassFrame,
        operations: &mut Vec<CommandOp>,
    ) -> GalResult<()> {
        for binding in program.opaque_resource_bindings.bindings() {
            if !matches!(
                binding.kind(),
                TerrainSourceOpaqueResourceKind::CombinedTextureSampler
            ) || !matches!(
                binding.role(),
                TerrainSourceResourceRole::ShaderPackColor(_)
            ) {
                continue;
            }
            let feedback = program.feedback_requirements.iter().any(|requirement| {
                requirement.role == binding.role()
                    && requirement.sampled_binding == binding.binding()
            });
            let mipmapped = program.mipmap_requirements.iter().any(|requirement| {
                requirement.role == binding.role()
                    && requirement.sampled_binding == binding.binding()
            });
            color_frame.require_sample_with_mips(&binding.role(), feedback, mipmapped)?;
        }
        frame.color_attachment_before =
            color_frame.attachment_states(&self.prepared.color_targets)?;
        self.append_draw(program, frame, operations)?;
        color_frame.record_pass(&self.prepared.color_targets, &self.prepared.outputs)
    }

    pub(crate) fn destroy(self, gal: &mut VulkanicGal) {
        self.bound.destroy(gal);
        self.compiled.destroy(gal);
        self.prepared.destroy(gal);
    }
}

impl SourceFinalOutputPlan {
    /// Stages the explicit last copy from the selected source final stage to
    /// one acquired Rust frame target. The source program must expose exactly
    /// one location-zero named color output; multi-output or nonzero final
    /// stages require a richer semantic presentation contract and are
    /// rejected instead of being guessed at.
    pub(crate) fn stage(
        gal: &mut VulkanicGal,
        final_program: &LoweredFullscreenSourceProgram,
        targets: &ShaderPackColorTargets,
        frame_target: Handle,
        color_attachment: Handle,
        main_depth_view: Handle,
        graph_generation: u64,
    ) -> GalResult<Self> {
        let input = Self::stage_input(
            gal,
            final_program,
            targets,
            frame_target,
            color_attachment,
            main_depth_view,
            graph_generation,
        )?;
        Self::stage_from_input(gal, input)
    }

    fn stage_input(
        gal: &VulkanicGal,
        final_program: &LoweredFullscreenSourceProgram,
        targets: &ShaderPackColorTargets,
        frame_target: Handle,
        color_attachment: Handle,
        main_depth_view: Handle,
        graph_generation: u64,
    ) -> GalResult<SourceFinalOutputStageInput> {
        if frame_target.kind() != Some(crate::render::vulkanic::handles::HandleKind::FrameTarget) {
            return Err(GalError::invalid_argument(
                "source final output requires an acquired GAL frame target",
            ));
        }
        if color_attachment != frame_target {
            return Err(GalError::invalid_argument(
                "source final output must use the acquired frame target as its color attachment",
            ));
        }
        let frame_target_desc = gal.frame_target_desc(frame_target)?;
        let outputs = &final_program.outputs;
        let [output] = outputs.as_slice() else {
            return Err(GalError::unsupported_feature(format!(
                "source final program '{}' must declare exactly one named color output",
                final_program.identity.as_str()
            )));
        };
        if output.source_location() != 0 {
            return Err(GalError::unsupported_feature(format!(
                "source final program '{}' writes location {}; only semantic final location zero is supported",
                final_program.identity.as_str(),
                output.source_location()
            )));
        }
        let source_role = output.role();
        let source_name = source_role
            .shader_pack_color_name()
            .ok_or_else(|| {
                GalError::invalid_argument(
                    "source final program output is not a named shader-pack color role",
                )
            })?
            .to_string();
        let source = targets.target(&source_name).ok_or_else(|| {
            GalError::invalid_argument(format!(
                "source final output '{}' has no staged semantic color target",
                source_name
            ))
        })?;
        // `SourceOverlayTarget::stage` validates the exact depth view against
        // the render target. The source runtime owns and already validates the
        // main-depth semantic resource; this presentation helper intentionally
        // does not need a general texture-view inspection API.
        if frame_target_desc.extent != targets.identity.extent {
            return Err(GalError::invalid_argument(format!(
                "source final output extent {:?} does not match acquired frame target extent {:?}",
                targets.identity.extent, frame_target_desc.extent
            )));
        }
        let frame_format = frame_target_desc.color_format;
        let identity = SourceFinalOutputIdentity {
            world_generation: targets.identity.world_generation,
            shader_pack_generation: targets.identity.shader_pack_generation,
            graph_generation,
            source_role: source_role.clone(),
            render_target: frame_target_desc.render_target,
            extent: [
                frame_target_desc.extent.width,
                frame_target_desc.extent.height,
                frame_target_desc.extent.depth,
            ],
            color_format: frame_format,
        };
        Ok(SourceFinalOutputStageInput {
            identity,
            source_role,
            source_view: source.current_view,
            source_name,
            frame_target,
            color_attachment,
            frame_format,
            source_format: source.format,
            main_depth_view,
        })
    }

    fn stage_from_input(
        gal: &mut VulkanicGal,
        input: SourceFinalOutputStageInput,
    ) -> GalResult<Self> {
        let SourceFinalOutputStageInput {
            identity,
            source_role,
            source_view,
            source_name,
            frame_target,
            color_attachment,
            frame_format,
            source_format,
            main_depth_view,
        } = input;
        let label = format!(
            "source-final-output.world{}-pack{}.{}",
            identity.world_generation, identity.shader_pack_generation, source_name
        );
        let overlay = SourceOverlayTarget::stage(
            gal,
            &format!("{label}.overlay"),
            identity.extent3d(),
            source_format,
            main_depth_view,
        )?;
        let overlay_pass = match gal.create_render_pass(RenderPassDesc {
            label: format!("{label}.overlay.pass"),
            target: overlay.target(),
            color_formats: vec![source_format],
            depth_format: Some(TextureFormat::Depth32Float),
        }) {
            Ok(pass) => pass,
            Err(error) => {
                overlay.destroy(gal);
                return Err(error);
            }
        };
        let source_copy = match SourceColorCopyPlan::stage(
            gal,
            &format!("{label}.source-copy"),
            source_view,
            overlay.target(),
            overlay.color_view(),
            source_format,
            Some(overlay.depth_view()),
        ) {
            Ok(copy) => copy,
            Err(error) => {
                let _ = gal.destroy(overlay_pass);
                overlay.destroy(gal);
                return Err(error);
            }
        };
        let present_copy = match SourceColorCopyPlan::stage(
            gal,
            &format!("{label}.present-copy"),
            overlay.color_view(),
            frame_target,
            color_attachment,
            frame_format,
            None,
        ) {
            Ok(copy) => copy,
            Err(error) => {
                source_copy.destroy(gal);
                let _ = gal.destroy(overlay_pass);
                overlay.destroy(gal);
                return Err(error);
            }
        };
        Ok(Self {
            identity,
            source_role,
            frame_target,
            frame_format,
            overlay,
            overlay_pass,
            source_copy,
            present_copy,
        })
    }

    /// Appends the final source color copy after the source color transaction
    /// has made the selected output shader-readable. This neither submits nor
    /// presents; the owning frame coordinator remains solely responsible for
    /// the single Rust submission and present.
    pub(crate) fn append_source_copy(&self, operations: &mut Vec<CommandOp>) {
        // A newly staged overlay has no prior attachment writer. Establish
        // the explicit color-attachment layout before the first source copy;
        // the final present copy below returns it to sampled state.
        operations.push(CommandOp::Barrier(texture_barrier(
            self.overlay.color_texture(),
            TextureUsageState::Undefined,
            TextureUsageState::ColorAttachment,
        )));
        // Source fullscreen consumers may have sampled the exact main depth
        // before the final color copy. Reattaching it to the private overlay
        // target is an explicit semantic transition; it is not an implicit
        // backend layout guess.
        operations.push(CommandOp::Barrier(texture_barrier(
            self.overlay.depth_view(),
            TextureUsageState::ShaderRead,
            TextureUsageState::DepthStencilAttachment,
        )));
        self.source_copy.append_draw(operations);
    }

    /// Same source copy with the actual prior semantic state of a cached
    /// frame-slot overlay. Newly staged plans begin Undefined; confirmed
    /// plans return from the prior present copy in ShaderRead. Keeping that
    /// distinction explicit prevents an alternating swapchain slot from
    /// being reattached through an invalid Undefined transition.
    pub(crate) fn append_source_copy_from_state(
        &self,
        operations: &mut Vec<CommandOp>,
        overlay_before: TextureUsageState,
    ) {
        operations.push(CommandOp::Barrier(texture_barrier(
            self.overlay.color_texture(),
            overlay_before,
            TextureUsageState::ColorAttachment,
        )));
        operations.push(CommandOp::Barrier(texture_barrier(
            self.overlay.depth_view(),
            overlay_before,
            TextureUsageState::DepthStencilAttachment,
        )));
        self.source_copy.append_draw(operations);
    }

    /// Returns the private target after the selected source chain has copied
    /// its final named color into it. World overlays may be added between this
    /// copy and `append_present_copy`, retaining the source main depth without
    /// exposing backend state through a frontend or Java boundary.
    pub(crate) fn overlay_target(&self) -> Handle {
        self.overlay.target()
    }

    /// The acquired Rust-owned final target after the source color has been
    /// presented into it. Screen-space GUI is composed here so the source
    /// color's framebuffer-to-sampled-image row conversion cannot invert
    /// top-left GUI coordinates. This is still part of the one Rust command
    /// stream and does not expose a backend target identity outside the
    /// source-frame presenter.
    pub(crate) fn presentation_target(&self) -> Handle {
        self.frame_target
    }

    pub(crate) fn overlay_color_attachment(&self) -> Handle {
        self.overlay.color_view()
    }

    /// The private source result is readable only by the Rust source-frame
    /// owner. This exists for bounded selected-source diagnostics; callers
    /// still receive no backend target identity or presentation control.
    pub(crate) fn overlay_color_texture(&self) -> Handle {
        self.overlay.color_texture()
    }

    pub(crate) fn overlay_depth_attachment(&self) -> Handle {
        self.overlay.depth_view()
    }

    #[allow(dead_code)]
    pub(crate) fn overlay_pass(&self) -> Handle {
        self.overlay_pass
    }

    pub(crate) fn overlay_color_format(&self) -> TextureFormat {
        self.overlay.color_format()
    }

    pub(crate) fn overlay_extent(&self) -> crate::render::vulkanic::resources::Extent3d {
        self.overlay.extent()
    }

    pub(crate) fn append_present_copy(&self, operations: &mut Vec<CommandOp>) {
        // The source copy and any following world overlays write the same
        // private color attachment. Transition it exactly once after the last
        // attachment writer, immediately before the present copy samples it.
        operations.push(CommandOp::Barrier(texture_barrier(
            self.overlay.color_texture(),
            TextureUsageState::ColorAttachment,
            TextureUsageState::ShaderRead,
        )));
        self.present_copy.append_draw(operations);
    }

    /// Stages a bounded diagnostic target that receives the same final copy as
    /// the acquired frame target. This has no presenter, no Java dependency,
    /// and exists only for a selected capture frame.
    pub(crate) fn stage_presented_capture(
        &self,
        gal: &mut VulkanicGal,
        label: &str,
    ) -> GalResult<SourceFinalPresentationCapture> {
        let mut created = Vec::new();
        let result = (|| -> GalResult<SourceFinalPresentationCapture> {
            let color_texture = gal.create_texture(TextureDesc {
                label: format!("{label}.color"),
                dimension: TextureDimension::D2,
                // This mirror represents the acquired presentation target,
                // not the source overlay. Its format must match the final
                // target so diagnostic GUI replay cannot hide conversion.
                format: self.frame_format,
                extent: self.overlay.extent(),
                mip_levels: 1,
                array_layers: 1,
                usages: vec![TextureUsage::ColorAttachment, TextureUsage::TransferSrc],
            })?;
            created.push(color_texture);
            let color_view = gal.create_texture_view(TextureViewDesc {
                label: format!("{label}.color-view"),
                texture: color_texture,
                format: self.frame_format,
                base_mip: 0,
                mip_count: 1,
                base_layer: 0,
                layer_count: 1,
            })?;
            created.push(color_view);
            let target = gal.create_render_target(RenderTargetDesc {
                label: format!("{label}.target"),
                color_views: vec![color_view],
                depth_stencil_view: None,
                extent: self.overlay.extent(),
            })?;
            created.push(target);
            let copy = SourceColorCopyPlan::stage(
                gal,
                &format!("{label}.copy"),
                self.overlay.color_view(),
                target,
                color_view,
                self.frame_format,
                None,
            )?;
            Ok(SourceFinalPresentationCapture {
                target,
                color_texture,
                color_view,
                format: self.frame_format,
                copy,
            })
        })();
        if result.is_err() {
            for handle in created.into_iter().rev() {
                let _ = gal.destroy(handle);
            }
        }
        result
    }

    pub(crate) fn source_role(&self) -> &TerrainSourceResourceRole {
        &self.source_role
    }

    pub(crate) fn identity(&self) -> &SourceFinalOutputIdentity {
        &self.identity
    }

    pub(crate) fn destroy(self, gal: &mut VulkanicGal) {
        self.present_copy.destroy(gal);
        self.source_copy.destroy(gal);
        let _ = gal.destroy(self.overlay_pass);
        self.overlay.destroy(gal);
    }
}

impl SourceFinalPresentationCapture {
    /// Frame-local Rust-owned target used only to retain a semantic replay of
    /// the final presentation image for one selected diagnostic frame.
    pub(crate) fn target(&self) -> Handle {
        self.target
    }

    pub(crate) fn color_attachment(&self) -> Handle {
        self.color_view
    }

    pub(crate) fn color_texture(&self) -> Handle {
        self.color_texture
    }

    pub(crate) fn format(&self) -> TextureFormat {
        self.format
    }

    pub(crate) fn append_draw(&self, operations: &mut Vec<CommandOp>) {
        // The mirror is frame-local and its first writer is the exact final
        // presentation copy. Establish the target attachment layout explicitly
        // before that draw; the subsequent GUI replay remains in the same
        // color-attachment state until the readback path transitions it.
        operations.push(CommandOp::Barrier(texture_barrier(
            self.color_texture,
            TextureUsageState::Undefined,
            TextureUsageState::ColorAttachment,
        )));
        self.copy.append_draw(operations);
    }

    pub(crate) fn destroy(self, gal: &mut VulkanicGal) {
        self.copy.destroy(gal);
        for handle in [self.target, self.color_view, self.color_texture] {
            let _ = gal.destroy(handle);
        }
    }
}

impl SourceColorCopyPlan {
    fn stage(
        gal: &mut VulkanicGal,
        label: &str,
        source_view: Handle,
        target: Handle,
        color_attachment: Handle,
        color_format: TextureFormat,
        depth_attachment: Option<Handle>,
    ) -> GalResult<Self> {
        let mut created = Vec::new();
        let result = (|| -> GalResult<Self> {
            let sampler = gal.create_sampler(SamplerDesc {
                label: format!("{label}.sampler"),
                min_filter: SamplerFilter::Nearest,
                mag_filter: SamplerFilter::Nearest,
                mip_filter: SamplerFilter::Nearest,
                address_u: SamplerAddressMode::ClampToEdge,
                address_v: SamplerAddressMode::ClampToEdge,
                address_w: SamplerAddressMode::ClampToEdge,
                comparison: None,
            })?;
            created.push(sampler);
            let combined_sampler =
                gal.create_combined_texture_sampler(CombinedTextureSamplerDesc {
                    label: format!("{label}.combined-sampler"),
                    texture_view: source_view,
                    sampler,
                })?;
            created.push(combined_sampler);
            let resource_layout = gal.create_resource_layout(ResourceLayoutDesc {
                label: format!("{label}.resource-layout"),
                bindings: vec![ResourceBindingDesc {
                    binding: 0,
                    kind: ResourceBindingKind::CombinedTextureSampler,
                    stages: PipelineStageFlags::DRAW,
                    array_count: 1,
                    optional: false,
                    dynamic_offset_count: 0,
                }],
            })?;
            created.push(resource_layout);
            let resource_set = gal.create_resource_set(ResourceSetDesc {
                label: format!("{label}.resource-set"),
                layout: resource_layout,
                bindings: vec![ResourceBinding {
                    binding: 0,
                    array_index: 0,
                    resource: combined_sampler,
                    kind: ResourceBindingKind::CombinedTextureSampler,
                    access: AccessFlags::READ,
                    dynamic_offsets: Vec::new(),
                    buffer_range: None,
                }],
            })?;
            created.push(resource_set);
            let pipeline_layout = gal.create_pipeline_layout(PipelineLayoutDesc {
                label: format!("{label}.pipeline-layout"),
                resource_layouts: vec![resource_layout],
            })?;
            created.push(pipeline_layout);
            let [vertex_desc, fragment_desc] =
                source_final_copy_shader_modules(gal.capabilities().api, label);
            let vertex_shader = gal.create_shader_module(vertex_desc)?;
            created.push(vertex_shader);
            let fragment_shader = gal.create_shader_module(fragment_desc)?;
            created.push(fragment_shader);
            let depth_format = depth_attachment.map(|_| TextureFormat::Depth32Float);
            let pass = gal.create_render_pass(RenderPassDesc {
                label: format!("{label}.pass"),
                target,
                color_formats: vec![color_format],
                depth_format,
            })?;
            created.push(pass);
            let pipeline = gal.create_graphics_pipeline(GraphicsPipelineDesc {
                label: format!("{label}.pipeline"),
                layout: pipeline_layout,
                vertex_shader,
                fragment_shader,
                topology: PrimitiveTopology::Triangles,
                cull_mode: CullMode::None,
                front_face: crate::render::vulkanic::resources::FrontFace::CounterClockwise,
                provoking_vertex: crate::render::vulkanic::resources::ProvokingVertex::Last,
                raster_y_direction: crate::render::vulkanic::resources::RasterYDirection::Up,
                blend: BlendMode::Disabled,
                depth_compare: None,
                depth_write: false,
                depth_bias: None,
                color_formats: vec![color_format],
                depth_format,
                stencil: None,
            })?;
            created.push(pipeline);
            Ok(Self {
                target,
                color_attachment,
                depth_attachment,
                pass,
                resource_layout,
                pipeline_layout,
                resource_set,
                sampler,
                combined_sampler,
                vertex_shader,
                fragment_shader,
                pipeline,
            })
        })();
        if result.is_err() {
            for handle in created.into_iter().rev() {
                let _ = gal.destroy(handle);
            }
        }
        result
    }

    fn append_draw(&self, operations: &mut Vec<CommandOp>) {
        operations.push(CommandOp::BeginPass {
            pass: self.pass,
            target: self.target,
            colors: vec![PassAttachment {
                view: self.color_attachment,
                load_op: AttachmentLoadOp::DontCare,
                store_op: AttachmentStoreOp::Store,
                clear_color: None,
            }],
            depth_stencil: self.depth_attachment.map(|view| PassAttachment {
                view,
                load_op: AttachmentLoadOp::Load,
                store_op: AttachmentStoreOp::Store,
                clear_color: None,
            }),
        });
        operations.push(CommandOp::BindGraphicsPipeline(self.pipeline));
        operations.push(CommandOp::BindResourceSet {
            pipeline_layout: self.pipeline_layout,
            set_index: 0,
            set: self.resource_set,
            dynamic_offsets: Vec::new(),
        });
        operations.push(CommandOp::Draw {
            vertices: 3,
            instances: 1,
        });
        operations.push(CommandOp::EndPass);
    }

    fn destroy(self, gal: &mut VulkanicGal) {
        for handle in [
            self.pipeline,
            self.pass,
            self.fragment_shader,
            self.vertex_shader,
            self.pipeline_layout,
            self.resource_set,
            self.resource_layout,
            self.combined_sampler,
            self.sampler,
        ] {
            let _ = gal.destroy(handle);
        }
    }
}

impl SourceFinalOutputCache {
    /// Keep a small retirement-safe history for a reused frame slot.  A few
    /// generations may overlap while the backend drains in-flight work, but
    /// the cache must not grow with every graph rebuild or world generation.
    const MAX_PLANS_PER_FRAME_TARGET: usize = 8;

    /// Ensures one persistent final-copy binding for a semantic source output
    /// and acquired swapchain image slot. Validation happens before cache
    /// lookup so an otherwise warm cache cannot conceal an incompatible frame
    /// target or source target generation.
    pub(crate) fn reserve(
        &mut self,
        gal: &mut VulkanicGal,
        final_program: &LoweredFullscreenSourceProgram,
        targets: &ShaderPackColorTargets,
        frame_target: Handle,
        color_attachment: Handle,
        main_depth_view: Handle,
        graph_generation: u64,
    ) -> GalResult<SourceFinalOutputReservation> {
        let input = SourceFinalOutputPlan::stage_input(
            gal,
            final_program,
            targets,
            frame_target,
            color_attachment,
            main_depth_view,
            graph_generation,
        )?;
        // A frame slot is reused across source/world graph generations.  The
        // old final-copy plan cannot be reused once any compatibility fact
        // changes, but retaining every generation until swapchain teardown
        // would make the cache grow with gameplay time.  Retire stale plans
        // for this exact acquired slot and semantic output now.  `destroy`
        // is GAL-owned and therefore defers native destruction until the last
        // submission that referenced the plan has completed; in-flight work
        // remains valid while the cache stays bounded by active frame slots.
        let mut stale_identities = self
            .plans
            .keys()
            .filter(|identity| {
                identity.render_target == input.identity.render_target
                    && identity.source_role == input.identity.source_role
                    && **identity != input.identity
            })
            .cloned()
            .collect::<Vec<_>>();
        if stale_identities.len() >= Self::MAX_PLANS_PER_FRAME_TARGET {
            // BTreeMap order is stable and the identity's generation fields
            // are monotonic in normal operation, so removing the oldest
            // entries gives the backend a bounded retirement window while
            // retaining the most recent plans for overlapping submissions.
            stale_identities.sort_by_key(|identity| {
                (
                    identity.world_generation,
                    identity.shader_pack_generation,
                    identity.graph_generation,
                )
            });
            let remove_count = stale_identities
                .len()
                .saturating_sub(Self::MAX_PLANS_PER_FRAME_TARGET - 1);
            stale_identities.truncate(remove_count);
        } else {
            stale_identities.clear();
        }
        for identity in stale_identities {
            if let Some(plan) = self.plans.remove(&identity) {
                plan.destroy(gal);
            }
        }
        if self.plans.contains_key(&input.identity) {
            return Ok(SourceFinalOutputReservation {
                identity: input.identity,
                newly_staged: false,
            });
        }
        let identity = input.identity.clone();
        let plan = SourceFinalOutputPlan::stage_from_input(gal, input)?;
        self.plans.insert(identity.clone(), plan);
        Ok(SourceFinalOutputReservation {
            identity,
            newly_staged: true,
        })
    }

    pub(crate) fn plan(
        &self,
        reservation: &SourceFinalOutputReservation,
    ) -> GalResult<&SourceFinalOutputPlan> {
        self.plans.get(&reservation.identity).ok_or_else(|| {
            GalError::backend(
                "source final-output reservation was retired before frame recording completed",
            )
        })
    }

    /// A successfully submitted entry remains warm. Keeping this explicit
    /// makes the transaction boundary visible beside the failure rollback.
    pub(crate) fn confirm(&self, reservation: &SourceFinalOutputReservation) -> GalResult<()> {
        self.plan(reservation).map(|_| ())
    }

    /// Removes only a binding first allocated by the unsubmitted frame. A
    /// warm binding is never retired by an unrelated record failure.
    pub(crate) fn discard(
        &mut self,
        reservation: SourceFinalOutputReservation,
        gal: &mut VulkanicGal,
    ) {
        if reservation.newly_staged {
            if let Some(plan) = self.plans.remove(&reservation.identity) {
                plan.destroy(gal);
            }
        }
    }

    /// Retires bindings that reference frame targets about to be recreated or
    /// destroyed. The public cache identity remains semantic, while this
    /// private owner performs the necessary GAL dependency cleanup before a
    /// backend can retire its acquired image slot.
    pub(crate) fn retire_frame_targets(&mut self, gal: &mut VulkanicGal, targets: &[Handle]) {
        let identities = self
            .plans
            .iter()
            .filter_map(|(identity, plan)| {
                targets
                    .contains(&plan.frame_target)
                    .then(|| identity.clone())
            })
            .collect::<Vec<_>>();
        for identity in identities {
            if let Some(plan) = self.plans.remove(&identity) {
                plan.destroy(gal);
            }
        }
    }

    pub(crate) fn destroy(&mut self, gal: &mut VulkanicGal) {
        let plans = std::mem::take(&mut self.plans);
        for (_, plan) in plans {
            plan.destroy(gal);
        }
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.plans.len()
    }
}

const SOURCE_FINAL_COPY_VERTEX: &str = r#"#version 450
layout(location = 0) out vec2 source_uv;
void main() {
    const vec2 positions[3] = vec2[3](vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    vec2 position = positions[gl_VertexIndex];
    source_uv = position * 0.5 + 0.5;
    gl_Position = vec4(position, 0.0, 1.0);
}
"#;

const SOURCE_FINAL_COPY_FRAGMENT: &str = r#"#version 450
layout(set = 0, binding = 0) uniform sampler2D source_final_color;
layout(location = 0) in vec2 source_uv;
layout(location = 0) out vec4 final_color;
void main() {
    final_color = texture(source_final_color, source_uv);
}
"#;

fn source_final_copy_shader_modules(
    api: crate::render::vulkanic::resources::BackendApi,
    label: &str,
) -> [ShaderModuleDesc; 2] {
    [
        ShaderModuleDesc {
            label: format!("{label}.vertex"),
            stage: ShaderStage::Vertex,
            code_format: ShaderCodeFormat::Glsl,
            code: shader_stage_code_for_backend(api, SOURCE_FINAL_COPY_VERTEX),
            entry_point: "main".to_string(),
        },
        ShaderModuleDesc {
            label: format!("{label}.fragment"),
            stage: ShaderStage::Fragment,
            code_format: ShaderCodeFormat::Glsl,
            code: shader_stage_code_for_backend(api, SOURCE_FINAL_COPY_FRAGMENT),
            entry_point: "main".to_string(),
        },
    ]
}

impl PreparedFullscreenSourcePass {
    /// Prepares a complete, generation-coherent source pass contract. Source
    /// colors are allocated by the private target cache; all non-color roles
    /// must arrive through independently owned semantic resource sets.
    ///
    /// This cannot execute a draw. Keeping preparation separate makes an
    /// incomplete source plan reject before a backend can construct a pass or
    /// accidentally mix an internal fixture with shader-pack resources.
    pub(crate) fn prepare(
        gal: &mut VulkanicGal,
        program: &LoweredFullscreenSourceProgram,
        manifest: &ShaderPackColorTargetManifest,
        targets: &ShaderPackColorTargets,
        external_inputs: impl IntoIterator<Item = TerrainSourceOwnedResourceSet>,
    ) -> GalResult<Self> {
        let color_resources = prepare_fullscreen_source_color_resources(gal, program, targets)?;
        let mut input_sets = external_inputs.into_iter();
        let Some(mut inputs) = input_sets.next() else {
            color_resources.destroy(gal);
            return Err(GalError::invalid_argument(
                "fullscreen source pass requires an exact source resource snapshot",
            ));
        };
        for resources in input_sets.chain(std::iter::once(color_resources.resources().clone())) {
            let unique = match resources.excluding_roles_already_owned_by(&inputs) {
                Ok(unique) => unique,
                Err(error) => {
                    color_resources.destroy(gal);
                    return Err(error);
                }
            };
            if unique.len() == 0 {
                continue;
            }
            inputs = match TerrainSourceOwnedResourceSet::merge([&inputs, &unique]) {
                Ok(merged) => merged,
                Err(error) => {
                    color_resources.destroy(gal);
                    return Err(error);
                }
            };
        }
        let outputs = match resolve_fullscreen_source_color_attachments(program, manifest, targets)
        {
            Ok(outputs) => outputs,
            Err(error) => {
                color_resources.destroy(gal);
                return Err(error);
            }
        };
        let color_targets = outputs.clone();

        if inputs.availability().shader_pack_generation() != program.shader_pack_generation {
            color_resources.destroy(gal);
            return Err(GalError::invalid_argument(format!(
                "fullscreen source program generation {} does not match its prepared input generation {}",
                program.shader_pack_generation,
                inputs.availability().shader_pack_generation()
            )));
        }
        if let Err(error) = program.require_semantic_resources(inputs.availability()) {
            color_resources.destroy(gal);
            let available = inputs
                .availability()
                .resources()
                .map(|resource| resource.role.semantic_name().to_string())
                .collect::<Vec<_>>()
                .join(",");
            return Err(GalError::invalid_argument(format!(
                "fullscreen source program '{}' semantic resources unavailable: {error}; available=[{}]",
                program.identity.as_str(), available
            )));
        }
        if let Err(error) = validate_feedback_separation(program, &outputs) {
            color_resources.destroy(gal);
            return Err(error);
        }
        if let Err(error) = validate_output_target_slots(&outputs, &color_targets) {
            color_resources.destroy(gal);
            return Err(error);
        }

        Ok(Self {
            program_identity: program.identity.as_str().to_string(),
            inputs,
            color_targets,
            outputs,
            color_resources,
        })
    }

    /// Explicit destruction of the program-local color samplers. Source color
    /// targets themselves remain owned by their generation cache.
    pub(crate) fn destroy(self, gal: &mut VulkanicGal) {
        self.color_resources.destroy(gal);
    }

    /// Compiles an explicit backend-neutral fullscreen pass from this already
    /// validated semantic contract. It owns no frame target and issues no
    /// draw, so compiling cannot alter route selection or presentation.
    pub(crate) fn compile(
        &self,
        gal: &mut VulkanicGal,
        program: &LoweredFullscreenSourceProgram,
        extent: crate::render::vulkanic::resources::Extent3d,
    ) -> GalResult<CompiledFullscreenSourcePass> {
        if self.program_identity != program.identity.as_str()
            || self.inputs.availability().shader_pack_generation() != program.shader_pack_generation
        {
            return Err(GalError::invalid_argument(
                "fullscreen source program does not match the prepared semantic contract",
            ));
        }
        if extent.width == 0 || extent.height == 0 || extent.depth != 1 {
            return Err(GalError::invalid_argument(
                "fullscreen source pass requires a non-zero two-dimensional extent",
            ));
        }
        let layouts = program.execution_resource_layouts()?;
        let color_formats = self
            .color_targets
            .iter()
            .map(|output| output.format)
            .collect::<Vec<_>>();
        let color_views = self
            .color_targets
            .iter()
            .map(|output| output.view)
            .collect::<Vec<_>>();
        if color_formats.is_empty() {
            return Err(GalError::invalid_argument(
                "fullscreen source pass requires at least one semantic color output",
            ));
        }
        let label = format!("fullscreen-source.{}", self.program_identity);
        let mut created = Vec::new();
        let result = (|| -> GalResult<CompiledFullscreenSourcePass> {
            let source_data_layout = gal.create_resource_layout(ResourceLayoutDesc {
                label: format!("{label}.source-data"),
                bindings: layouts.source_data.bindings,
            })?;
            created.push(source_data_layout);
            let pack_resources_layout = gal.create_resource_layout(ResourceLayoutDesc {
                label: format!("{label}.pack-resources"),
                bindings: layouts.pack_resources.bindings,
            })?;
            created.push(pack_resources_layout);
            let pipeline_layout = gal.create_pipeline_layout(PipelineLayoutDesc {
                label: format!("{label}.layout"),
                resource_layouts: vec![source_data_layout, pack_resources_layout],
            })?;
            created.push(pipeline_layout);
            let [vertex, fragment] = program.shader_module_descriptors(gal.capabilities().api);
            let vertex_shader = gal.create_shader_module(vertex)?;
            created.push(vertex_shader);
            let fragment_shader = gal.create_shader_module(fragment)?;
            created.push(fragment_shader);
            let target = gal.create_render_target(RenderTargetDesc {
                label: format!("{label}.target"),
                color_views,
                depth_stencil_view: None,
                extent,
            })?;
            created.push(target);
            let pass = gal.create_render_pass(RenderPassDesc {
                label: format!("{label}.pass"),
                target,
                color_formats: color_formats.clone(),
                depth_format: None,
            })?;
            created.push(pass);
            let pipeline = gal.create_graphics_pipeline(GraphicsPipelineDesc {
                label: format!("{label}.pipeline"),
                layout: pipeline_layout,
                vertex_shader,
                fragment_shader,
                topology: PrimitiveTopology::Triangles,
                cull_mode: CullMode::None,
                front_face: crate::render::vulkanic::resources::FrontFace::CounterClockwise,
                provoking_vertex: crate::render::vulkanic::resources::ProvokingVertex::Last,
                raster_y_direction: crate::render::vulkanic::resources::RasterYDirection::Up,
                blend: BlendMode::Disabled,
                depth_compare: None,
                depth_write: false,
                depth_bias: None,
                color_formats,
                depth_format: None,
                stencil: None,
            })?;
            created.push(pipeline);
            Ok(CompiledFullscreenSourcePass {
                target,
                pass,
                source_data_layout,
                pack_resources_layout,
                pipeline_layout,
                vertex_shader,
                fragment_shader,
                pipeline,
            })
        })();
        if result.is_err() {
            for handle in created.into_iter().rev() {
                let _ = gal.destroy(handle);
            }
        }
        result
    }

    /// Materializes the two explicit descriptor/resource sets required for
    /// this program. Buffers are Rust-owned upload resources; no Java memory
    /// or renderer uniform object is retained. The caller must write exactly
    /// the program ABI byte counts before recording the fullscreen draw.
    pub(crate) fn bind_resources(
        &self,
        gal: &mut VulkanicGal,
        program: &LoweredFullscreenSourceProgram,
        compiled: &CompiledFullscreenSourcePass,
    ) -> GalResult<BoundFullscreenSourcePass> {
        if self.program_identity != program.identity.as_str() {
            return Err(GalError::invalid_argument(
                "fullscreen source resources cannot bind a different program",
            ));
        }
        let interface = &program.execution_interface;
        interface.validate()?;
        let label = format!("fullscreen-source.{}", self.program_identity);
        let mut created = Vec::new();
        let result = (|| -> GalResult<BoundFullscreenSourcePass> {
            let texture_transform_buffer = gal.create_buffer(BufferDesc {
                label: format!("{label}.texture-transforms"),
                size: u64::from(interface.texture_transform_bytes),
                memory: MemoryDomain::Upload,
                usages: vec![BufferUsage::Uniform, BufferUsage::HostWrite],
            })?;
            created.push(texture_transform_buffer);
            let scalar_uniform_buffer = if let Some(_) = interface.scalar_uniforms {
                let buffer = gal.create_buffer(BufferDesc {
                    label: format!("{label}.scalar-uniforms"),
                    size: u64::from(interface.scalar_uniform_bytes),
                    memory: MemoryDomain::Upload,
                    usages: vec![BufferUsage::Uniform, BufferUsage::HostWrite],
                })?;
                created.push(buffer);
                Some(buffer)
            } else {
                None
            };
            let mut source_bindings = vec![ResourceBinding {
                binding: interface.texture_transforms.binding,
                array_index: 0,
                resource: texture_transform_buffer,
                kind: ResourceBindingKind::UniformBuffer,
                access: AccessFlags::READ,
                dynamic_offsets: vec![0],
                buffer_range: Some(u64::from(interface.texture_transform_bytes)),
            }];
            if let (Some(binding), Some(buffer)) =
                (interface.scalar_uniforms, scalar_uniform_buffer)
            {
                source_bindings.push(ResourceBinding {
                    binding: binding.binding,
                    array_index: 0,
                    resource: buffer,
                    kind: ResourceBindingKind::UniformBuffer,
                    access: AccessFlags::READ,
                    dynamic_offsets: vec![0],
                    buffer_range: Some(u64::from(interface.scalar_uniform_bytes)),
                });
            }
            source_bindings.sort_by_key(|binding| binding.binding);
            let source_data_set = gal.create_resource_set(ResourceSetDesc {
                label: format!("{label}.source-data-set"),
                layout: compiled.source_data_layout,
                bindings: source_bindings,
            })?;
            created.push(source_data_set);
            let pack_resources_set = gal.create_resource_set(program.pack_resource_set_desc(
                format!("{label}.pack-resources-set"),
                compiled.pack_resources_layout,
                &self.inputs,
            )?)?;
            created.push(pack_resources_set);
            Ok(BoundFullscreenSourcePass {
                source_data_set,
                pack_resources_set,
                texture_transform_buffer,
                scalar_uniform_buffer,
            })
        })();
        if result.is_err() {
            for handle in created.into_iter().rev() {
                let _ = gal.destroy(handle);
            }
        }
        result
    }

    /// Appends one fully explicit fullscreen draw. It neither submits nor
    /// presents, and cannot select a gameplay route. A higher-level runtime
    /// must own frame sequencing for both vanilla and Distant Horizons.
    pub(crate) fn append_draw(
        &self,
        program: &LoweredFullscreenSourceProgram,
        compiled: &CompiledFullscreenSourcePass,
        bound: &BoundFullscreenSourcePass,
        frame: FullscreenSourcePassFrame,
        operations: &mut Vec<CommandOp>,
    ) -> GalResult<()> {
        if self.program_identity != program.identity.as_str() {
            return Err(GalError::invalid_argument(
                "fullscreen source draw cannot use a different program than its prepared contract",
            ));
        }
        let interface = &program.execution_interface;
        interface.validate()?;
        if frame.texture_transforms.len() != interface.texture_transform_bytes as usize {
            return Err(GalError::invalid_argument(format!(
                "fullscreen source texture transform payload is {} bytes but program ABI requires {}",
                frame.texture_transforms.len(),
                interface.texture_transform_bytes
            )));
        }
        match interface.scalar_uniforms {
            Some(_) if frame.scalar_uniforms.len() != interface.scalar_uniform_bytes as usize => {
                return Err(GalError::invalid_argument(format!(
                    "fullscreen source scalar uniform payload is {} bytes but program ABI requires {}",
                    frame.scalar_uniforms.len(),
                    interface.scalar_uniform_bytes
                )));
            }
            None if !frame.scalar_uniforms.is_empty() || frame.scalar_uniform_before.is_some() => {
                return Err(GalError::invalid_argument(
                    "fullscreen source program has no scalar uniform binding but scalar frame data was supplied",
                ));
            }
            Some(_) if frame.scalar_uniform_before.is_none() => {
                return Err(GalError::invalid_argument(
                    "fullscreen source scalar uniform binding requires an explicit prior state",
                ));
            }
            _ => {}
        }
        if frame.color_attachment_before.len() != self.color_targets.len() {
            return Err(GalError::invalid_argument(format!(
                "fullscreen source frame supplies {} color attachment states for {} source slots",
                frame.color_attachment_before.len(),
                self.color_targets.len()
            )));
        }
        if frame.texture_transform_before == TextureUsageState::TransferDst
            || frame.scalar_uniform_before == Some(TextureUsageState::TransferDst)
            || frame
                .color_attachment_before
                .iter()
                .any(|state| *state == TextureUsageState::ColorAttachment)
        {
            return Err(GalError::invalid_argument(
                "fullscreen source frame cannot begin from an in-progress transfer or color-attachment state",
            ));
        }
        let colors = self
            .color_targets
            .iter()
            .zip(frame.color_attachment_before.iter().copied())
            .map(|(attachment, before)| {
                if attachment.clear_each_frame {
                    return Ok(PassAttachment {
                        view: attachment.view,
                        // `Clear = true` is source-pack frame semantics, not
                        // one-time allocation initialization. The prior
                        // layout still matters to the barrier below, but the
                        // attachment must be cleared on every source-frame
                        // use after it becomes a valid history image.
                        load_op: AttachmentLoadOp::Clear,
                        store_op: AttachmentStoreOp::Store,
                        clear_color: Some(source_color_clear_color(
                            attachment.source_slot,
                            attachment.clear_color_bits,
                            frame.clear_values.fog_color,
                        )),
                    });
                }
                if before == TextureUsageState::Undefined {
                    let writes_attachment = self
                        .outputs
                        .iter()
                        .any(|output| output.role == attachment.role);
                    if !attachment.clear_each_frame && !writes_attachment {
                        return Err(GalError::invalid_argument(format!(
                            "fullscreen source target '{}' is undefined, is not cleared, and is not written by this pass",
                            role_name(&attachment.role)
                        )));
                    }
                    return Ok(PassAttachment {
                        view: attachment.view,
                        // A source target with `Clear = false` explicitly
                        // leaves its prior contents unspecified. Only a pass
                        // that writes that exact target may discard them on a
                        // first use; untouched attachments must retain valid
                        // history and therefore reject while undefined.
                        // Feedback sampling remains separate and must be
                        // proven by its own previous image.
                        load_op: AttachmentLoadOp::DontCare,
                        store_op: AttachmentStoreOp::Store,
                        clear_color: None,
                    });
                }
                Ok(PassAttachment {
                    view: attachment.view,
                    load_op: AttachmentLoadOp::Load,
                    store_op: AttachmentStoreOp::Store,
                    clear_color: None,
                })
            })
            .collect::<GalResult<Vec<_>>>()?;

        operations.push(CommandOp::Barrier(buffer_barrier(
            bound.texture_transform_buffer,
            frame.texture_transform_before,
            TextureUsageState::TransferDst,
        )));
        operations.push(CommandOp::HostWriteBuffer {
            buffer: bound.texture_transform_buffer,
            offset: 0,
            data: frame.texture_transforms,
        });
        operations.push(CommandOp::Barrier(buffer_barrier(
            bound.texture_transform_buffer,
            TextureUsageState::TransferDst,
            TextureUsageState::ShaderRead,
        )));
        if let Some(buffer) = bound.scalar_uniform_buffer {
            let before = frame
                .scalar_uniform_before
                .expect("validated scalar uniform prior state");
            operations.push(CommandOp::Barrier(buffer_barrier(
                buffer,
                before,
                TextureUsageState::TransferDst,
            )));
            operations.push(CommandOp::HostWriteBuffer {
                buffer,
                offset: 0,
                data: frame.scalar_uniforms,
            });
            operations.push(CommandOp::Barrier(buffer_barrier(
                buffer,
                TextureUsageState::TransferDst,
                TextureUsageState::ShaderRead,
            )));
        }
        for (attachment, before) in self
            .color_targets
            .iter()
            .zip(frame.color_attachment_before.iter().copied())
        {
            operations.push(CommandOp::Barrier(texture_barrier(
                attachment.texture,
                before,
                TextureUsageState::ColorAttachment,
            )));
        }
        operations.push(CommandOp::BeginPass {
            pass: compiled.pass,
            target: compiled.target,
            colors,
            depth_stencil: None,
        });
        operations.push(CommandOp::BindGraphicsPipeline(compiled.pipeline));
        operations.push(CommandOp::BindResourceSet {
            pipeline_layout: compiled.pipeline_layout,
            set_index: 0,
            set: bound.source_data_set,
            dynamic_offsets: vec![0; usize::from(interface.scalar_uniforms.is_some()) + 1],
        });
        operations.push(CommandOp::BindResourceSet {
            pipeline_layout: compiled.pipeline_layout,
            set_index: 1,
            set: bound.pack_resources_set,
            dynamic_offsets: Vec::new(),
        });
        operations.push(CommandOp::Draw {
            vertices: program.raster_primitive.vertex_count(),
            instances: 1,
        });
        operations.push(CommandOp::EndPass);
        for attachment in &self.color_targets {
            operations.push(CommandOp::Barrier(texture_barrier(
                attachment.texture,
                TextureUsageState::ColorAttachment,
                TextureUsageState::ShaderRead,
            )));
        }
        Ok(())
    }
}

impl CompiledFullscreenSourcePass {
    pub(crate) fn destroy(self, gal: &mut VulkanicGal) {
        for handle in [
            self.pipeline,
            self.pass,
            self.target,
            self.vertex_shader,
            self.fragment_shader,
            self.pipeline_layout,
            self.source_data_layout,
            self.pack_resources_layout,
        ] {
            let _ = gal.destroy(handle);
        }
    }
}

impl BoundFullscreenSourcePass {
    pub(crate) fn destroy(self, gal: &mut VulkanicGal) {
        for handle in [
            Some(self.pack_resources_set),
            Some(self.source_data_set),
            self.scalar_uniform_buffer,
            Some(self.texture_transform_buffer),
        ]
        .into_iter()
        .flatten()
        {
            let _ = gal.destroy(handle);
        }
    }
}

fn validate_feedback_separation(
    program: &LoweredFullscreenSourceProgram,
    outputs: &[FullscreenSourceColorAttachment],
) -> GalResult<()> {
    for output in outputs {
        let sampled_same_role = program
            .opaque_resource_bindings
            .bindings()
            .iter()
            .any(|binding| binding.role() == output.role);
        if !sampled_same_role {
            continue;
        }
        let output_location = program
            .outputs
            .iter()
            .find(|candidate| candidate.role() == output.role)
            .map(|candidate| candidate.source_location())
            .ok_or_else(|| {
                GalError::backend("fullscreen output attachment lost its lowered GLSL location")
            })?;
        let has_feedback_pair = program.feedback_requirements.iter().any(|requirement| {
            requirement.role == output.role && requirement.output_location == output_location
        });
        if !has_feedback_pair {
            return Err(GalError::invalid_argument(format!(
                "fullscreen source program '{}' samples and writes semantic color '{}' without an explicit feedback pair",
                program.identity.as_str(),
                role_name(&output.role)
            )));
        }
    }
    Ok(())
}

fn validate_output_target_slots(
    outputs: &[FullscreenSourceColorAttachment],
    color_targets: &[FullscreenSourceColorAttachment],
) -> GalResult<()> {
    if outputs != color_targets {
        return Err(GalError::invalid_argument(
            "fullscreen source outputs differ from the staged lowered-location attachment set",
        ));
    }
    Ok(())
}

fn role_name(role: &TerrainSourceResourceRole) -> String {
    role.diagnostic_name()
}

fn buffer_barrier(
    resource: Handle,
    before: TextureUsageState,
    after: TextureUsageState,
) -> ResourceBarrier {
    ResourceBarrier {
        resource,
        subresources: None,
        before,
        after,
        src_queue: QueueClass::Graphics,
        dst_queue: QueueClass::Graphics,
    }
}

fn texture_barrier(
    resource: Handle,
    before: TextureUsageState,
    after: TextureUsageState,
) -> ResourceBarrier {
    ResourceBarrier {
        resource,
        subresources: None,
        before,
        after,
        src_queue: QueueClass::Graphics,
        dst_queue: QueueClass::Graphics,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::vulkanic::backends::{
        mock::MockBackend, presentation_capabilities, vulkan_capabilities,
    };
    use crate::render::vulkanic::backends::{opengl::OpenGlBackend, vulkan::VulkanBackend};
    use crate::render::vulkanic::commands::{
        CommandList, CommandListDesc, ResourceBarrier, SubmissionBatch,
    };
    use crate::render::vulkanic::frame::FrameRenderTargetId;
    use crate::render::vulkanic::resources::{
        Extent3d, FrameTargetDesc, TextureDesc, TextureDimension, TextureFormat, TextureUsage,
        TextureViewDesc,
    };
    use crate::render::vulkanic::shader_pack::lowering::lower_fullscreen_source_pair;
    use crate::render::vulkanic::shader_pack::preprocess::preprocess_source_stage_pair;
    use crate::render::vulkanic::shader_pack::programs::prepare_lowered_fullscreen_source_program;
    use crate::render::vulkanic::shader_pack::source::{ShaderPackSource, ShaderSourceFile};
    use crate::render::vulkanic::shader_pack::terrain_contract::{
        TerrainSourceStage, TerrainSourceStages,
    };
    use crate::render::vulkanic::shader_pack::terrain_source_resources::{
        TerrainSourceOwnedResourceSet, TerrainSourceResourceAvailabilitySet,
        TerrainSourceResourceBindings, TERRAIN_RESOURCE_BINDINGS_PATH,
    };

    const SETTINGS: &str = concat!(
        "const int colortex0Format = RGBA8;\n",
        "const int colortex1Format = RGBA8;\n",
        "const int colortex2Format = RGBA8;\n",
        "const int colortex3Format = RGBA8;\n",
        "const int colortex4Format = RGBA8;\n",
        "const int colortex5Format = RGBA8;\n",
        "const int colortex6Format = RGBA8;\n",
        "const int colortex7Format = RGBA8;\n",
    );
    const BINDINGS: &str = concat!(
        "colortex0=shader_pack_color:primary\n",
        "colortex1=shader_pack_color:previous_depth\n",
        "colortex2=shader_pack_color:temporal_aa\n",
        "colortex3=shader_pack_color:translucent_final\n",
        "colortex4=shader_pack_color:volumetric_factor\n",
        "colortex5=shader_pack_color:normal_scene\n",
        "colortex6=shader_pack_color:material_auxiliary\n",
        "colortex7=shader_pack_color:temporal_reflection\n",
    );

    fn source(fragment: &str) -> ShaderPackSource {
        source_with_settings(fragment, SETTINGS)
    }

    fn source_with_settings(fragment: &str, settings: &str) -> ShaderPackSource {
        let fragment = if fragment.contains("DRAWBUFFERS:") {
            fragment.to_string()
        } else {
            fragment.replacen("#version 130\n", "#version 130\n/* DRAWBUFFERS:0 */\n", 1)
        };
        ShaderPackSource::new(
            "fullscreen-plan",
            9,
            vec![
                ShaderSourceFile::new("lib/pipelineSettings.glsl", settings),
                ShaderSourceFile::new(TERRAIN_RESOURCE_BINDINGS_PATH, BINDINGS),
                ShaderSourceFile::new(
                    "world0/deferred1.vsh",
                    "#version 130\nout vec2 uv;\nvoid main() { uv = gl_MultiTexCoord0.xy; gl_Position = ftransform(); }",
                ),
                ShaderSourceFile::new("world0/deferred1.fsh", fragment),
            ],
        )
        .unwrap()
    }

    fn program(source: &ShaderPackSource) -> LoweredFullscreenSourceProgram {
        let stages = TerrainSourceStages {
            vertex: TerrainSourceStage {
                path: "world0/deferred1.vsh".to_string(),
                defines: Default::default(),
            },
            fragment: TerrainSourceStage {
                path: "world0/deferred1.fsh".to_string(),
                defines: Default::default(),
            },
        };
        let artifacts = preprocess_source_stage_pair(source, &stages).unwrap();
        let bindings = TerrainSourceResourceBindings::from_source(source).unwrap();
        let lowered =
            lower_fullscreen_source_pair(&artifacts.vertex, &artifacts.fragment, &bindings)
                .unwrap();
        let opaque = lowered
            .opaque_resource_contract()
            .bind_semantic_roles(&bindings)
            .unwrap();
        prepare_lowered_fullscreen_source_program(
            source.name(),
            source.generation(),
            "world0/deferred1.fsh",
            &lowered,
            &opaque,
        )
        .unwrap()
    }

    fn staged(
        source: &ShaderPackSource,
        gal: &mut VulkanicGal,
        feedback: bool,
    ) -> (
        ShaderPackColorTargetManifest,
        ShaderPackColorTargets,
        super::super::source_targets::ShaderPackColorTargetCache,
    ) {
        let bindings = TerrainSourceResourceBindings::from_source(source).unwrap();
        let manifest = ShaderPackColorTargetManifest::from_source(source, &bindings).unwrap();
        let identity = super::super::source_targets::ShaderPackColorTargetIdentity::new(
            13,
            source.generation(),
            Extent3d {
                width: 16,
                height: 16,
                depth: 1,
            },
            feedback.then(|| "primary".to_string()).into_iter(),
            std::iter::empty::<String>(),
        )
        .unwrap();
        let mut cache = super::super::source_targets::ShaderPackColorTargetCache::default();
        let targets = cache.stage(gal, identity, &manifest).unwrap();
        (manifest, targets, cache)
    }

    /// Fullscreen staging requires an exact snapshot even when this focused
    /// fixture has no external semantic resources. That distinguishes an
    /// intentionally empty source contract from a caller that forgot to
    /// provide its generation-bound resource snapshot.
    fn empty_source_resource_snapshot(source: &ShaderPackSource) -> TerrainSourceOwnedResourceSet {
        TerrainSourceOwnedResourceSet::new(
            TerrainSourceResourceAvailabilitySet::new(source.generation(), 13, []).unwrap(),
            [],
        )
        .unwrap()
    }

    fn source_main_depth(gal: &mut VulkanicGal) -> (Handle, Handle) {
        let texture = gal
            .create_texture(TextureDesc {
                label: "source-final-output.depth".to_string(),
                dimension: TextureDimension::D2,
                format: TextureFormat::Depth32Float,
                extent: Extent3d {
                    width: 16,
                    height: 16,
                    depth: 1,
                },
                mip_levels: 1,
                array_layers: 1,
                usages: vec![TextureUsage::DepthStencilAttachment, TextureUsage::Sampled],
            })
            .unwrap();
        let view = gal
            .create_texture_view(TextureViewDesc {
                label: "source-final-output.depth-view".to_string(),
                texture,
                format: TextureFormat::Depth32Float,
                base_mip: 0,
                mip_count: 1,
                base_layer: 0,
                layer_count: 1,
            })
            .unwrap();
        (texture, view)
    }

    #[test]
    fn rejects_feedback_execution_before_a_confirmed_source_history_frame() {
        let source = source(
            "#version 130\nin vec2 uv;\nuniform sampler2D colortex0;\nvoid main() { gl_FragData[0] = texture2D(colortex0, uv); }",
        );
        let program = program(&source);
        let mut gal = VulkanicGal::new_with_backend(Box::new(MockBackend::default()), false);
        let (manifest, targets, mut cache) = staged(&source, &mut gal, true);
        let plan = FullscreenSourceExecutionPlan::stage(
            &mut gal,
            &program,
            &manifest,
            &targets,
            std::iter::once(empty_source_resource_snapshot(&source)),
            Extent3d {
                width: 16,
                height: 16,
                depth: 1,
            },
        )
        .unwrap();
        assert_eq!(program.identity.as_str(), plan.prepared.program_identity);
        assert_eq!(1, plan.prepared.inputs.len());
        assert_eq!(1, plan.prepared.outputs.len());
        let texture_transforms = program
            .pack_texture_transforms(
                &crate::render::vulkanic::shader_pack::programs::TerrainSourceTextureTransforms::canonical_minecraft_terrain(),
            )
            .unwrap();
        let mut operations = Vec::new();
        let mut color_frame = cache.begin_frame(&targets).unwrap();
        let error = plan
            .append_draw_with_color_frame(
                &program,
                &mut color_frame,
                FullscreenSourcePassFrame {
                    texture_transforms,
                    scalar_uniforms: Vec::new(),
                    texture_transform_before: TextureUsageState::Undefined,
                    scalar_uniform_before: None,
                    clear_values: ShaderPackColorBootstrapClearValues {
                        fog_color: crate::render::vulkanic::commands::ClearColor {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 1.0,
                        },
                    },
                    // The scheduler owns these states; this caller value is
                    // deliberately ignored by the route-facing recorder.
                    color_attachment_before: Vec::new(),
                },
                &mut operations,
            )
            .unwrap_err();
        assert!(error.to_string().contains("feedback history"));
        assert!(operations.is_empty());
        plan.destroy(&mut gal);
        cache.destroy(&mut gal);
    }

    #[test]
    fn final_output_plan_copies_only_the_final_stage_named_output_to_a_frame_target() {
        let source = source(
            "#version 130\nin vec2 uv;\nvoid main() { gl_FragData[0] = vec4(uv, 0.0, 1.0); }",
        );
        let program = program(&source);
        let mut gal = VulkanicGal::new_with_backend(
            Box::new(MockBackend::with_capabilities(presentation_capabilities(
                vulkan_capabilities(),
            ))),
            false,
        );
        let (_manifest, targets, mut cache) = staged(&source, &mut gal, false);
        let frame_target = gal
            .create_frame_target(FrameTargetDesc {
                label: "source-final-output.frame".to_string(),
                frame_id: 17,
                render_target: FrameRenderTargetId(17),
                extent: Extent3d {
                    width: 16,
                    height: 16,
                    depth: 1,
                },
                color_format: TextureFormat::Rgba8Unorm,
            })
            .unwrap();
        let (depth_texture, depth_view) = source_main_depth(&mut gal);
        let plan = SourceFinalOutputPlan::stage(
            &mut gal,
            &program,
            &targets,
            frame_target,
            frame_target,
            depth_view,
            1,
        )
        .unwrap();
        assert_eq!(
            &TerrainSourceResourceRole::ShaderPackColor("primary".to_string()),
            plan.source_role()
        );
        assert_eq!(FrameRenderTargetId(17), plan.identity().render_target);
        assert_eq!(
            [
                targets.identity.extent.width,
                targets.identity.extent.height,
                targets.identity.extent.depth,
            ],
            plan.identity().extent
        );
        assert_eq!(TextureFormat::Rgba8Unorm, plan.identity().color_format);
        let source_target = targets.target("primary").unwrap();
        let overlay_target = plan.overlay_target();
        assert_eq!(TextureFormat::Rgba8Unorm, plan.overlay_color_format());
        assert_ne!(frame_target, plan.overlay_color_attachment());
        assert_eq!(depth_view, plan.overlay_depth_attachment());
        let mut operations = vec![CommandOp::Barrier(ResourceBarrier {
            resource: source_target.current_texture,
            subresources: None,
            before: TextureUsageState::Undefined,
            after: TextureUsageState::ShaderRead,
            src_queue: QueueClass::Graphics,
            dst_queue: QueueClass::Graphics,
        })];
        plan.append_source_copy(&mut operations);
        // Model a world overlay pass between the source copy and final
        // presentation. The color target must remain an attachment until the
        // last overlay writer has completed.
        operations.push(CommandOp::BeginPass {
            pass: plan.overlay_pass(),
            target: overlay_target,
            colors: vec![PassAttachment {
                view: plan.overlay_color_attachment(),
                load_op: AttachmentLoadOp::Load,
                store_op: AttachmentStoreOp::Store,
                clear_color: None,
            }],
            depth_stencil: Some(PassAttachment {
                view: depth_view,
                load_op: AttachmentLoadOp::Load,
                store_op: AttachmentStoreOp::Store,
                clear_color: None,
            }),
        });
        operations.push(CommandOp::EndPass);
        plan.append_present_copy(&mut operations);
        let presented_capture = plan
            .stage_presented_capture(&mut gal, "source-final-output.presented-capture")
            .unwrap();
        presented_capture.append_draw(&mut operations);
        assert!(operations.iter().any(|operation| matches!(
            operation,
            CommandOp::BeginPass { target, depth_stencil: Some(depth), .. }
                if *target == overlay_target && depth.view == depth_view
        )));
        assert!(operations.iter().any(|operation| matches!(
            operation,
            CommandOp::BeginPass { target, colors, .. }
                if *target == frame_target && colors[0].view == frame_target
        )));
        let present_begin = operations
            .iter()
            .position(|operation| {
                matches!(
                    operation,
                    CommandOp::BeginPass { target, .. } if *target == frame_target
                )
            })
            .unwrap();
        let capture_begin = operations
            .iter()
            .position(|operation| {
                matches!(
                    operation,
                    CommandOp::BeginPass { target, .. }
                        if *target == presented_capture.target
                )
            })
            .unwrap();
        assert!(
            present_begin < capture_begin,
            "the diagnostic mirror must copy after the acquired-target present copy"
        );
        let overlay_begin = operations
            .iter()
            .position(|operation| {
                matches!(
                    operation,
                    CommandOp::BeginPass { pass, target, .. }
                        if *pass == plan.overlay_pass() && *target == overlay_target
                )
            })
            .unwrap();
        let overlay_end = overlay_begin + 1;
        assert!(matches!(operations[overlay_end], CommandOp::EndPass));
        let present_transition = operations
            .iter()
            .position(|operation| {
                matches!(
                    operation,
                    CommandOp::Barrier(barrier)
                        if barrier.resource == plan.overlay.color_texture()
                            && barrier.before == TextureUsageState::ColorAttachment
                            && barrier.after == TextureUsageState::ShaderRead
                )
            })
            .unwrap();
        assert!(
            overlay_end < present_transition,
            "the overlay target must become shader-readable only after its final attachment writer"
        );
        gal.submit(SubmissionBatch {
            label: "source-final-output".to_string(),
            command_lists: vec![CommandList::from(CommandListDesc {
                label: "source-final-output.commands".to_string(),
                operations,
            })],
        })
        .expect("the source final copy must be an ordinary explicit GAL submission");
        presented_capture.destroy(&mut gal);
        plan.destroy(&mut gal);
        gal.destroy(depth_view).unwrap();
        gal.destroy(depth_texture).unwrap();
        cache.destroy(&mut gal);
    }

    #[test]
    fn source_overlay_target_owns_color_and_keeps_the_source_depth_dependency_explicit() {
        let mut gal = VulkanicGal::new_with_backend(Box::new(MockBackend::default()), false);
        let extent = Extent3d {
            width: 16,
            height: 16,
            depth: 1,
        };
        let depth_texture = gal
            .create_texture(TextureDesc {
                label: "source-overlay.depth".to_string(),
                dimension: TextureDimension::D2,
                format: TextureFormat::Depth32Float,
                extent,
                mip_levels: 1,
                array_layers: 1,
                usages: vec![TextureUsage::DepthStencilAttachment, TextureUsage::Sampled],
            })
            .unwrap();
        let depth_view = gal
            .create_texture_view(TextureViewDesc {
                label: "source-overlay.depth-view".to_string(),
                texture: depth_texture,
                format: TextureFormat::Depth32Float,
                base_mip: 0,
                mip_count: 1,
                base_layer: 0,
                layer_count: 1,
            })
            .unwrap();
        let overlay = SourceOverlayTarget::stage(
            &mut gal,
            "source-overlay",
            extent,
            TextureFormat::Rgba16Float,
            depth_view,
        )
        .unwrap();
        assert_eq!(extent, overlay.extent());
        assert_eq!(TextureFormat::Rgba16Float, overlay.color_format());
        assert_eq!(depth_view, overlay.depth_view());
        assert_eq!(
            overlay.color_view(),
            gal.pass_target_color_attachment(overlay.target()).unwrap()
        );
        assert_eq!(
            Some((depth_texture, depth_view)),
            gal.pass_target_depth_attachment(overlay.target()).unwrap()
        );
        assert!(gal.destroy(depth_view).is_err());

        overlay.destroy(&mut gal);
        gal.destroy(depth_view).unwrap();
        gal.destroy(depth_texture).unwrap();
    }

    #[test]
    fn final_output_cache_reuses_a_semantic_swapchain_slot_and_discards_only_unsubmitted_staging() {
        let source = source(
            "#version 130\nin vec2 uv;\nvoid main() { gl_FragData[0] = vec4(uv, 0.0, 1.0); }",
        );
        let program = program(&source);
        let mut gal = VulkanicGal::new_with_backend(
            Box::new(MockBackend::with_capabilities(presentation_capabilities(
                vulkan_capabilities(),
            ))),
            false,
        );
        let (_manifest, targets, mut source_targets) = staged(&source, &mut gal, false);
        let frame_target = gal
            .create_frame_target(FrameTargetDesc {
                label: "source-final-output.cached-frame".to_string(),
                frame_id: 19,
                render_target: FrameRenderTargetId(3),
                extent: Extent3d {
                    width: 16,
                    height: 16,
                    depth: 1,
                },
                color_format: TextureFormat::Rgba8Unorm,
            })
            .unwrap();
        let (depth_texture, depth_view) = source_main_depth(&mut gal);
        let mut final_cache = SourceFinalOutputCache::default();
        let first = final_cache
            .reserve(
                &mut gal,
                &program,
                &targets,
                frame_target,
                frame_target,
                depth_view,
                1,
            )
            .unwrap();
        assert!(first.newly_staged);
        assert_eq!(FrameRenderTargetId(3), first.identity().render_target);
        let creates_after_first = gal.metrics().resource_creates;
        let second = final_cache
            .reserve(
                &mut gal,
                &program,
                &targets,
                frame_target,
                frame_target,
                depth_view,
                1,
            )
            .unwrap();
        assert!(!second.newly_staged);
        assert_eq!(first.identity(), second.identity());
        assert_eq!(creates_after_first, gal.metrics().resource_creates);
        assert_eq!(1, final_cache.len());

        let after_graph_rebuild = final_cache
            .reserve(
                &mut gal,
                &program,
                &targets,
                frame_target,
                frame_target,
                depth_view,
                2,
            )
            .unwrap();
        assert!(after_graph_rebuild.newly_staged);
        assert_ne!(first.identity(), after_graph_rebuild.identity());
        assert_eq!(2, final_cache.len());
        final_cache.discard(after_graph_rebuild, &mut gal);
        assert_eq!(1, final_cache.len());

        final_cache.confirm(&first).unwrap();
        final_cache.discard(second, &mut gal);
        assert_eq!(1, final_cache.len());

        // Repeated graph rebuilds for one acquired slot must not retain a
        // plan per generation forever.  Confirm each replacement to model
        // ordinary submitted frames; GAL retirement keeps any in-flight
        // native use safe while the semantic cache remains bounded.
        for graph_generation in 3..=16 {
            let replacement = final_cache
                .reserve(
                    &mut gal,
                    &program,
                    &targets,
                    frame_target,
                    frame_target,
                    depth_view,
                    graph_generation,
                )
                .unwrap();
            assert!(replacement.newly_staged);
            final_cache.confirm(&replacement).unwrap();
        }
        assert!(
            final_cache.len() <= SourceFinalOutputCache::MAX_PLANS_PER_FRAME_TARGET,
            "repeated graph rebuilds must retain only a bounded frame-slot history"
        );

        let other_target = gal
            .create_frame_target(FrameTargetDesc {
                label: "source-final-output.cached-frame-other-slot".to_string(),
                frame_id: 20,
                render_target: FrameRenderTargetId(4),
                extent: Extent3d {
                    width: 16,
                    height: 16,
                    depth: 1,
                },
                color_format: TextureFormat::Rgba8Unorm,
            })
            .unwrap();
        let frame_slot_history = final_cache.len();
        let unsubmitted = final_cache
            .reserve(
                &mut gal,
                &program,
                &targets,
                other_target,
                other_target,
                depth_view,
                1,
            )
            .unwrap();
        assert!(unsubmitted.newly_staged);
        assert_eq!(
            frame_slot_history + 1,
            final_cache.len(),
            "a distinct acquired slot may add one independent final-output plan"
        );
        final_cache.discard(unsubmitted, &mut gal);
        assert_eq!(frame_slot_history, final_cache.len());

        final_cache.retire_frame_targets(&mut gal, &[frame_target]);
        assert_eq!(
            0,
            final_cache.len(),
            "swapchain-target retirement must release the cached pass before the target",
        );
        gal.destroy(frame_target)
            .expect("the cached final pass must not keep a retired frame target alive");
        gal.destroy(other_target).unwrap();
        final_cache.destroy(&mut gal);
        gal.destroy(depth_view).unwrap();
        gal.destroy(depth_texture).unwrap();
        source_targets.destroy(&mut gal);
    }

    #[test]
    fn final_output_plan_rejects_a_named_color_texture_as_a_presentation_target() {
        let source = source(
            "#version 130\nin vec2 uv;\nvoid main() { gl_FragData[0] = vec4(uv, 0.0, 1.0); }",
        );
        let program = program(&source);
        let mut gal = VulkanicGal::new_with_backend(Box::new(MockBackend::default()), false);
        let (_manifest, targets, mut cache) = staged(&source, &mut gal, false);
        let color_texture = targets.target("primary").unwrap().current_texture;
        let creates_before = gal.metrics().resource_creates;
        let error = SourceFinalOutputPlan::stage(
            &mut gal,
            &program,
            &targets,
            color_texture,
            color_texture,
            color_texture,
            1,
        )
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("requires an acquired GAL frame target"));
        assert_eq!(
            creates_before,
            gal.metrics().resource_creates,
            "an invalid final target must reject before staging any copy resources"
        );
        cache.destroy(&mut gal);
    }

    #[test]
    fn final_output_plan_rejects_an_incompatible_acquired_target_extent_before_staging() {
        let source = source(
            "#version 130\nin vec2 uv;\nvoid main() { gl_FragData[0] = vec4(uv, 0.0, 1.0); }",
        );
        let program = program(&source);
        let mut gal = VulkanicGal::new_with_backend(
            Box::new(MockBackend::with_capabilities(presentation_capabilities(
                vulkan_capabilities(),
            ))),
            false,
        );
        let (_manifest, targets, mut cache) = staged(&source, &mut gal, false);
        let frame_target = gal
            .create_frame_target(FrameTargetDesc {
                label: "source-final-output.wrong-extent".to_string(),
                frame_id: 18,
                render_target: FrameRenderTargetId(18),
                extent: Extent3d {
                    width: 8,
                    height: 8,
                    depth: 1,
                },
                color_format: TextureFormat::Rgba8Unorm,
            })
            .unwrap();
        let creates_before = gal.metrics().resource_creates;
        let error = SourceFinalOutputPlan::stage(
            &mut gal,
            &program,
            &targets,
            frame_target,
            frame_target,
            frame_target,
            1,
        )
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("does not match acquired frame target extent"));
        assert_eq!(creates_before, gal.metrics().resource_creates);
        gal.destroy(frame_target).unwrap();
        cache.destroy(&mut gal);
    }

    #[test]
    fn source_final_copy_shaders_compile_at_the_vulkan_boundary() {
        let backend = match VulkanBackend::new("MattMC source final-copy Vulkan conformance") {
            Ok(backend) => backend,
            Err(error) => {
                let text = error.to_string();
                assert!(
                    text.contains("Vulkan")
                        || text.contains("vulkan")
                        || text.contains("physical device"),
                    "unexpected Vulkan source final-copy setup failure: {text}"
                );
                return;
            }
        };
        let mut gal = VulkanicGal::new_with_backend(Box::new(backend), false);
        for module in source_final_copy_shader_modules(
            crate::render::vulkanic::resources::BackendApi::Vulkan,
            "source-final-copy.vulkan",
        ) {
            gal.create_shader_module(module).unwrap_or_else(|error| {
                panic!("Rust-owned source final-copy shader must lower through Vulkan: {error}")
            });
        }
    }

    #[test]
    fn source_final_copy_shaders_compile_at_the_opengl_boundary() {
        let backend = match OpenGlBackend::new("MattMC source final-copy OpenGL conformance") {
            Ok(backend) => backend,
            Err(error) => {
                let text = error.to_string();
                assert!(
                    text.contains("OpenGL") || text.contains("EGL") || text.contains("GL"),
                    "unexpected OpenGL source final-copy setup failure: {text}"
                );
                return;
            }
        };
        let mut gal = VulkanicGal::new_with_backend(Box::new(backend), false);
        for module in source_final_copy_shader_modules(
            crate::render::vulkanic::resources::BackendApi::OpenGl,
            "source-final-copy.opengl",
        ) {
            gal.create_shader_module(module).unwrap_or_else(|error| {
                panic!("Rust-owned source final-copy shader must lower through OpenGL: {error}")
            });
        }
    }

    #[test]
    fn nonclearing_source_output_records_dont_care_on_first_write() {
        let settings = concat!(
            "const int colortex0Format = RGBA8;\n",
            "const bool colortex0Clear = false;\n",
        );
        let source = source_with_settings(
            "#version 130\nin vec2 uv;\nvoid main() { gl_FragData[0] = vec4(uv, 0.0, 1.0); }",
            settings,
        );
        let program = program(&source);
        let mut gal = VulkanicGal::new_with_backend(Box::new(MockBackend::default()), false);
        let (manifest, targets, mut cache) = staged(&source, &mut gal, false);
        let plan = FullscreenSourceExecutionPlan::stage(
            &mut gal,
            &program,
            &manifest,
            &targets,
            std::iter::once(empty_source_resource_snapshot(&source)),
            Extent3d {
                width: 16,
                height: 16,
                depth: 1,
            },
        )
        .unwrap();
        let mut operations = Vec::new();
        let mut color_frame = cache.begin_frame(&targets).unwrap();
        plan.append_draw_with_color_frame(
            &program,
            &mut color_frame,
            FullscreenSourcePassFrame {
                texture_transforms: program
                    .pack_texture_transforms(
                        &crate::render::vulkanic::shader_pack::programs::TerrainSourceTextureTransforms::canonical_minecraft_terrain(),
                    )
                    .unwrap(),
                scalar_uniforms: Vec::new(),
                texture_transform_before: TextureUsageState::Undefined,
                scalar_uniform_before: None,
                clear_values: ShaderPackColorBootstrapClearValues {
                    fog_color: crate::render::vulkanic::commands::ClearColor {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 1.0,
                    },
                },
                color_attachment_before: Vec::new(),
            },
            &mut operations,
        )
        .unwrap();
        assert!(operations.iter().any(|operation| matches!(
            operation,
            CommandOp::BeginPass { colors, .. }
                if colors[0].load_op == AttachmentLoadOp::DontCare
        )));
        gal.submit(SubmissionBatch {
            label: "fullscreen-source-nonclearing-first-write".to_string(),
            command_lists: vec![CommandList::from(CommandListDesc {
                label: "fullscreen-source-nonclearing-first-write.commands".to_string(),
                operations,
            })],
        })
        .expect("a non-clearing source output may be first-written without invented history");
        cache
            .confirm_frame_submission(&mut gal, color_frame)
            .expect("the successful source submission must seed only confirmed color history");
        plan.destroy(&mut gal);
        cache.destroy(&mut gal);
    }

    #[test]
    fn clearing_source_output_clears_again_after_a_prior_frame() {
        let source = source(
            "#version 130\nin vec2 uv;\nvoid main() { gl_FragData[0] = vec4(uv, 0.0, 1.0); }",
        );
        let program = program(&source);
        let mut gal = VulkanicGal::new_with_backend(Box::new(MockBackend::default()), false);
        let (manifest, targets, mut cache) = staged(&source, &mut gal, false);
        let plan = FullscreenSourceExecutionPlan::stage(
            &mut gal,
            &program,
            &manifest,
            &targets,
            std::iter::once(empty_source_resource_snapshot(&source)),
            Extent3d {
                width: 16,
                height: 16,
                depth: 1,
            },
        )
        .unwrap();
        let mut operations = Vec::new();
        plan.append_draw(
            &program,
            FullscreenSourcePassFrame {
                texture_transforms: program
                    .pack_texture_transforms(
                        &crate::render::vulkanic::shader_pack::programs::TerrainSourceTextureTransforms::canonical_minecraft_terrain(),
                    )
                    .unwrap(),
                scalar_uniforms: Vec::new(),
                texture_transform_before: TextureUsageState::ShaderRead,
                scalar_uniform_before: None,
                clear_values: ShaderPackColorBootstrapClearValues {
                    fog_color: crate::render::vulkanic::commands::ClearColor {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 1.0,
                    },
                },
                color_attachment_before: vec![TextureUsageState::ShaderRead],
            },
            &mut operations,
        )
        .unwrap();
        assert!(operations.iter().any(|operation| matches!(
            operation,
            CommandOp::BeginPass { colors, .. }
                if colors[0].load_op == AttachmentLoadOp::Clear
        )));
        plan.destroy(&mut gal);
        cache.destroy(&mut gal);
    }

    #[test]
    fn clearing_primary_source_output_uses_the_semantic_fog_clear() {
        let source = source(
            "#version 130\nin vec2 uv;\nvoid main() { gl_FragData[0] = vec4(uv, 0.0, 1.0); }",
        );
        let program = program(&source);
        let mut gal = VulkanicGal::new_with_backend(Box::new(MockBackend::default()), false);
        let (manifest, targets, _cache) = staged(&source, &mut gal, false);
        let plan = FullscreenSourceExecutionPlan::stage(
            &mut gal,
            &program,
            &manifest,
            &targets,
            std::iter::once(empty_source_resource_snapshot(&source)),
            Extent3d {
                width: 16,
                height: 16,
                depth: 1,
            },
        )
        .unwrap();
        let mut operations = Vec::new();
        let fog_color = crate::render::vulkanic::commands::ClearColor {
            r: 0.25,
            g: 0.5,
            b: 0.75,
            a: 1.0,
        };
        plan.append_draw(
            &program,
            FullscreenSourcePassFrame {
                texture_transforms: program
                    .pack_texture_transforms(
                        &crate::render::vulkanic::shader_pack::programs::TerrainSourceTextureTransforms::canonical_minecraft_terrain(),
                    )
                    .unwrap(),
                scalar_uniforms: Vec::new(),
                texture_transform_before: TextureUsageState::Undefined,
                scalar_uniform_before: None,
                clear_values: ShaderPackColorBootstrapClearValues { fog_color },
                color_attachment_before: vec![TextureUsageState::Undefined],
            },
            &mut operations,
        )
        .unwrap();
        assert!(operations.iter().any(|operation| matches!(
            operation,
            CommandOp::BeginPass { colors, .. }
                if colors[0].load_op == AttachmentLoadOp::Clear
                    && colors[0].clear_color == Some(fog_color)
        )));
        plan.destroy(&mut gal);
    }

    #[test]
    fn source_sky_disc_records_its_owned_twenty_four_vertex_geometry() {
        let source = source(
            "#version 130\nin vec2 uv;\nvoid main() { gl_FragData[0] = vec4(uv, 0.0, 1.0); }",
        );
        let mut program = program(&source);
        program.raster_primitive =
            super::super::lowering::FullscreenSourceRasterPrimitive::VanillaSkyDisc;
        let mut gal = VulkanicGal::new_with_backend(Box::new(MockBackend::default()), false);
        let (manifest, targets, _) = staged(&source, &mut gal, false);
        let plan = FullscreenSourceExecutionPlan::stage(
            &mut gal,
            &program,
            &manifest,
            &targets,
            std::iter::once(empty_source_resource_snapshot(&source)),
            Extent3d {
                width: 16,
                height: 16,
                depth: 1,
            },
        )
        .unwrap();
        let mut operations = Vec::new();
        plan.append_draw(
            &program,
            FullscreenSourcePassFrame {
                texture_transforms: program
                    .pack_texture_transforms(&crate::render::vulkanic::shader_pack::programs::TerrainSourceTextureTransforms::canonical_minecraft_terrain())
                    .unwrap(),
                scalar_uniforms: Vec::new(),
                texture_transform_before: TextureUsageState::Undefined,
                scalar_uniform_before: None,
                clear_values: ShaderPackColorBootstrapClearValues {
                    fog_color: crate::render::vulkanic::commands::ClearColor {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 1.0,
                    },
                },
                color_attachment_before: vec![TextureUsageState::Undefined],
            },
            &mut operations,
        )
        .unwrap();
        assert!(operations.iter().any(|operation| matches!(
            operation,
            CommandOp::Draw {
                vertices: 24,
                instances: 1
            }
        )));
        plan.destroy(&mut gal);
    }

    #[test]
    fn source_celestial_quad_records_owned_six_vertex_geometry() {
        let source = source(
            "#version 130\nin vec2 uv;\nvoid main() { gl_FragData[0] = vec4(uv, 0.0, 1.0); }",
        );
        let mut program = program(&source);
        program.raster_primitive =
            super::super::lowering::FullscreenSourceRasterPrimitive::VanillaCelestialQuad;
        let mut gal = VulkanicGal::new_with_backend(Box::new(MockBackend::default()), false);
        let (manifest, targets, _) = staged(&source, &mut gal, false);
        let plan = FullscreenSourceExecutionPlan::stage(
            &mut gal,
            &program,
            &manifest,
            &targets,
            std::iter::once(empty_source_resource_snapshot(&source)),
            Extent3d {
                width: 16,
                height: 16,
                depth: 1,
            },
        )
        .unwrap();
        let mut operations = Vec::new();
        plan.append_draw(
            &program,
            FullscreenSourcePassFrame {
                texture_transforms: program
                    .pack_texture_transforms(&crate::render::vulkanic::shader_pack::programs::TerrainSourceTextureTransforms::canonical_minecraft_terrain())
                    .unwrap(),
                scalar_uniforms: Vec::new(),
                texture_transform_before: TextureUsageState::Undefined,
                scalar_uniform_before: None,
                clear_values: ShaderPackColorBootstrapClearValues {
                    fog_color: crate::render::vulkanic::commands::ClearColor {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 1.0,
                    },
                },
                color_attachment_before: vec![TextureUsageState::Undefined],
            },
            &mut operations,
        )
        .unwrap();
        assert!(operations.iter().any(|operation| matches!(
            operation,
            CommandOp::Draw {
                vertices: 6,
                instances: 1
            }
        )));
        plan.destroy(&mut gal);
    }

    #[test]
    fn rejects_a_declared_feedback_pass_when_the_staged_target_has_no_previous_image() {
        let source = source(
            "#version 130\nin vec2 uv;\nuniform sampler2D colortex0;\nvoid main() { gl_FragData[0] = texture2D(colortex0, uv); }",
        );
        let program = program(&source);
        let mut gal = VulkanicGal::new_with_backend(Box::new(MockBackend::default()), false);
        let (manifest, targets, mut cache) = staged(&source, &mut gal, false);
        let error = PreparedFullscreenSourcePass::prepare(
            &mut gal,
            &program,
            &manifest,
            &targets,
            std::iter::empty::<TerrainSourceOwnedResourceSet>(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("no previous feedback view"));
        cache.destroy(&mut gal);
    }

    #[test]
    fn rejects_sparse_source_output_locations_before_staging() {
        let source = source(
            "#version 130\n/* DRAWBUFFERS:01234 */\nin vec2 uv;\nvoid main() { gl_FragData[4] = vec4(uv, 0.0, 1.0); }",
        );
        let stages = TerrainSourceStages {
            vertex: TerrainSourceStage {
                path: "world0/deferred1.vsh".to_string(),
                defines: Default::default(),
            },
            fragment: TerrainSourceStage {
                path: "world0/deferred1.fsh".to_string(),
                defines: Default::default(),
            },
        };
        let artifacts = preprocess_source_stage_pair(&source, &stages).unwrap();
        let bindings = TerrainSourceResourceBindings::from_source(&source).unwrap();
        let error = lower_fullscreen_source_pair(&artifacts.vertex, &artifacts.fragment, &bindings)
            .unwrap_err();
        assert!(error.to_string().contains("sparse gl_FragData locations"));
    }
}
