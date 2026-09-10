#[cfg(test)]
mod architecture_boundary;

mod backends;

// Private resource-processing prerequisite; not yet admitted through terrain FFI.
mod sprite_interpolation;

pub mod commands;
pub mod error;
pub mod ffi;
pub mod frame;
pub mod gal;
pub mod gui_frontend;
// Explicit same-context atlas references; private until native GUI sampling is wired.
mod gui_atlas_reference;
mod item_foil;
mod special_item_foil;
mod gui_item_material;
mod gui_item_raster;
mod gui_item_layout;
/// Private semantic tiled-GUI lowering; not yet a frame/FFI-admitted route.
mod gui_tiling;
/// Backend-neutral GUI mesh semantics. This is not an FFI-admitted route
/// until the owned offscreen renderer consumes it.
pub mod gui_mesh_frontend;
pub mod handles;
pub mod metrics;
pub mod resources;
pub mod shader_pack;
pub mod sync;
pub mod terrain;
pub mod world_primitive_frontend;
mod texture_sampling;

/// Maximum viewport axis admitted by semantic frame and GUI submissions.
/// Keeping this finite prevents hostile FFI dimensions from driving unbounded
/// staging, attachment, or uniform allocations.
pub(crate) const SEMANTIC_MAX_VIEWPORT_AXIS: i32 = 16_384;

pub use commands::{
    AttachmentLoadOp, AttachmentStoreOp, BufferImageCopyRegion, ClearColor, CommandList,
    CommandListDesc, CommandOp, PassAttachment, ResourceBarrier, SubmissionBatch, TextureOrigin3d,
    TextureUsageState,
};
pub use error::{GalError, GalResult, StatusCode};
pub use frame::{
    AcquiredFrame, FrameAcquireDesc, FrameAcquireStatus, FrameCorrelationId, FrameId,
    FramePresentStatus, FrameRenderTargetId, FrameResizeDesc, FrameResizeResult, FrameSurfaceDesc,
    PresentFrameDesc, PresentMode, PresentedFrame,
};
pub use gal::VulkanicGal;
pub use handles::{Handle, HandleKind};
pub use metrics::{Metrics, TracyZone};
pub use resources::{
    AccessFlags, BackendCapabilities, BackendFeature, BackendFeatureFlags, BackendLimits,
    BlendMode, BufferDesc, BufferUsage, ColorFormat, CompareOp, ComputePipelineDesc, CullMode,
    Extent3d, GraphicsPipelineDesc, IndexType, MemoryDomain, PipelineLayoutDesc,
    PipelineStageFlags, PrimitiveTopology, QueueClass, RenderPassDesc, RenderTargetDesc,
    ResourceBinding, ResourceBindingDesc, ResourceBindingKind, ResourceLayoutDesc, ResourceSetDesc,
    SamplerAddressMode, SamplerDesc, SamplerFilter, ShaderCodeFormat, ShaderModuleDesc,
    ShaderStage, TextureDesc, TextureDimension, TextureFormat, TextureSubresourceRange,
    TextureUsage, TextureViewDesc,
};
pub use sync::{RetirementQueue, SubmissionId, SyncToken};

#[cfg(test)]
mod tests;
