//! Owned world attachments with an explicit raster coordinate convention.
//! No presentation ownership, native handles, or inferred attachment state.
use crate::render::vulkanic::commands::*;
use crate::render::vulkanic::error::{GalError, GalResult};
use crate::render::vulkanic::gal::VulkanicGal;
use crate::render::vulkanic::handles::Handle;
use crate::render::vulkanic::resources::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::render::vulkanic) struct WorldTargetDesc {
    pub extent: Extent3d,
    pub color_format: TextureFormat,
    pub raster_y_direction: RasterYDirection,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::render::vulkanic) struct WorldAttachmentStates {
    pub color: TextureUsageState,
    pub depth: TextureUsageState,
}

impl WorldAttachmentStates {
    pub const ATTACHMENTS: Self = Self {
        color: TextureUsageState::ColorAttachment,
        depth: TextureUsageState::DepthStencilAttachment,
    };
    pub const UNDEFINED: Self = Self {
        color: TextureUsageState::Undefined,
        depth: TextureUsageState::Undefined,
    };
}

pub(in crate::render::vulkanic) struct OrientedWorldTarget {
    pub desc: WorldTargetDesc,
    pub color_texture: Handle,
    pub color_view: Handle,
    pub depth_texture: Handle,
    pub depth_view: Handle,
    pub target: Handle,
    pub pass: Handle,
}

/// Non-owning references to an explicitly described Rust GAL color/depth pair.
/// Ownership stays with the graph that allocated the images; no native handles
/// or presentation state cross this boundary.
#[derive(Clone, Copy)]
pub(in crate::render::vulkanic) struct WorldAttachmentImages {
    pub desc: WorldTargetDesc,
    pub color_texture: Handle,
    pub depth_texture: Handle,
    pub target: Handle,
}

impl OrientedWorldTarget {
    pub fn images(&self) -> WorldAttachmentImages {
        WorldAttachmentImages { desc: self.desc, color_texture: self.color_texture,
            depth_texture: self.depth_texture, target: self.target }
    }

    pub fn create(gal: &mut VulkanicGal, label: &str, desc: WorldTargetDesc) -> GalResult<Self> {
        if desc.extent.width == 0 || desc.extent.height == 0 || desc.extent.depth != 1
            || matches!(desc.color_format, TextureFormat::Depth32Float | TextureFormat::Depth24Stencil8)
        {
            return Err(GalError::invalid_argument("world target requires a nonempty 2D color extent"));
        }
        let mut created = Vec::new();
        let result = (|| {
            let mut pairs = Vec::new();
            for (name, format, usage) in [
                ("color", desc.color_format, TextureUsage::ColorAttachment),
                ("depth", TextureFormat::Depth32Float, TextureUsage::DepthStencilAttachment),
            ] {
                let texture = gal.create_texture(TextureDesc {
                    label: format!("{label}.{name}"), dimension: TextureDimension::D2,
                    format, extent: desc.extent, mip_levels: 1, array_layers: 1,
                    usages: vec![usage, TextureUsage::Sampled, TextureUsage::TransferSrc, TextureUsage::TransferDst],
                })?;
                created.push(texture);
                let view = gal.create_texture_view(TextureViewDesc {
                    label: format!("{label}.{name}.view"), texture, format,
                    base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1,
                })?;
                created.push(view);
                pairs.push((texture, view));
            }
            let target = gal.create_render_target(RenderTargetDesc {
                label: label.into(), color_views: vec![pairs[0].1],
                depth_stencil_view: Some(pairs[1].1), extent: desc.extent,
            })?;
            created.push(target);
            let pass = gal.create_render_pass(RenderPassDesc {
                label: format!("{label}.pass"), target, color_formats: vec![desc.color_format],
                depth_format: Some(TextureFormat::Depth32Float),
            })?;
            created.push(pass);
            Ok(Self { desc, color_texture: pairs[0].0, color_view: pairs[0].1,
                depth_texture: pairs[1].0, depth_view: pairs[1].1, target, pass })
        })();
        if result.is_err() {
            for handle in created.into_iter().rev() { gal.destroy(handle)?; }
        }
        result
    }

    /// The owner must retire dependent passes/sets before these handles. GAL
    /// tracks submitted uses and defers native destruction through completion.
    pub fn handles_in_destroy_order(&self) -> [Handle; 6] {
        [self.pass, self.target, self.depth_view, self.depth_texture, self.color_view, self.color_texture]
    }

    /// Compose canonical owned world output into the existing acquired target.
    /// Input contract: source color/depth are in their attachment usages. This
    /// fully overwrites (discards) destination depth and leaves it in attachment
    /// usage; source images end in TransferSrc. It does not commit GAL's frame
    /// depth-population bookkeeping, which belongs to successful submission.
    /// Down output must first be explicitly converted to a canonical owner;
    /// this operation never adds another presenter or hides a row reversal.
    pub fn copy_to_frame(&self, gal: &VulkanicGal, frame: Handle) -> GalResult<Vec<CommandOp>> {
        if self.desc.raster_y_direction != RasterYDirection::Up
            || gal.pass_target_extent(frame)? != self.desc.extent
            || gal.pass_target_color_format(frame)? != self.desc.color_format
        {
            return Err(GalError::invalid_argument("frame composition requires matching canonical world output"));
        }
        let (depth, _) = gal.frame_target_owned_depth_attachment(frame)?;
        let range = Some(TextureSubresourceRange { base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 });
        let barrier = |resource, before, after| CommandOp::Barrier(ResourceBarrier {
            resource, before, after, subresources: range,
            src_queue: QueueClass::Graphics, dst_queue: QueueClass::Graphics,
        });
        Ok(vec![
            barrier(self.color_texture, TextureUsageState::ColorAttachment, TextureUsageState::TransferSrc),
            CommandOp::CopyTextureToFrameTarget { src: self.color_texture, dst: frame, extent: self.desc.extent },
            barrier(self.depth_texture, TextureUsageState::DepthStencilAttachment, TextureUsageState::TransferSrc),
            // This is a complete overwrite of the newly acquired depth domain.
            barrier(depth, TextureUsageState::Undefined, TextureUsageState::TransferDst),
            CommandOp::CopyTexture(TextureImageCopyRegion {
                row_order: TextureRowOrder::Preserve, src_texture: self.depth_texture, src_mip: 0, src_layer: 0,
                src_origin: TextureOrigin3d { x: 0, y: 0, z: 0 }, dst_texture: depth, dst_mip: 0, dst_layer: 0,
                dst_origin: TextureOrigin3d { x: 0, y: 0, z: 0 }, extent: self.desc.extent,
            }),
            barrier(depth, TextureUsageState::TransferDst, TextureUsageState::DepthStencilAttachment),
        ])
    }

    /// Record a complete color AND depth transfer. The caller supplies actual
    /// states and commits its own bookkeeping only after successful submission.
    /// This never changes raster pipelines or makes an unsupported copy fallback.
    pub fn transfer_to(
        &self,
        destination: &Self,
        source_before: WorldAttachmentStates,
        destination_before: WorldAttachmentStates,
        destination_after: WorldAttachmentStates,
    ) -> GalResult<Vec<CommandOp>> {
        self.images().transfer_to(destination.images(), source_before, destination_before, destination_after)
    }
}

impl WorldAttachmentImages {
    pub fn transfer_to(
        self,
        destination: Self,
        source_before: WorldAttachmentStates,
        destination_before: WorldAttachmentStates,
        destination_after: WorldAttachmentStates,
    ) -> GalResult<Vec<CommandOp>> {
        let source_images = [self.color_texture, self.depth_texture];
        let destination_images = [destination.color_texture, destination.depth_texture];
        if self.target == destination.target || self.desc.extent != destination.desc.extent
            || self.desc.color_format != destination.desc.color_format
            || self.desc.extent.width == 0 || self.desc.extent.height == 0 || self.desc.extent.depth != 1
            || self.color_texture == self.depth_texture
            || destination.color_texture == destination.depth_texture
            || source_images.iter().any(|handle| destination_images.contains(handle))
            || source_images.iter().chain(destination_images.iter())
                .any(|handle| handle.kind() != Some(crate::render::vulkanic::handles::HandleKind::Texture))
        {
            return Err(GalError::invalid_argument("world attachment transfer requires distinct matching targets"));
        }
        let row_order = if self.desc.raster_y_direction == destination.desc.raster_y_direction {
            TextureRowOrder::Preserve
        } else { TextureRowOrder::Reverse };
        let range = Some(TextureSubresourceRange { base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 });
        let barrier = |resource, before, after| CommandOp::Barrier(ResourceBarrier {
            resource, before, after, subresources: range,
            src_queue: QueueClass::Graphics, dst_queue: QueueClass::Graphics,
        });
        let mut operations = Vec::with_capacity(8);
        for (src, dst, before, dst_before, after) in [
            (self.color_texture, destination.color_texture, source_before.color, destination_before.color, destination_after.color),
            (self.depth_texture, destination.depth_texture, source_before.depth, destination_before.depth, destination_after.depth),
        ] {
            operations.extend([
                barrier(src, before, TextureUsageState::TransferSrc),
                barrier(dst, dst_before, TextureUsageState::TransferDst),
                CommandOp::CopyTexture(TextureImageCopyRegion {
                    row_order, src_texture: src, src_mip: 0, src_layer: 0,
                    src_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
                    dst_texture: dst, dst_mip: 0, dst_layer: 0,
                    dst_origin: TextureOrigin3d { x: 0, y: 0, z: 0 }, extent: self.desc.extent,
                }),
                barrier(dst, TextureUsageState::TransferDst, after),
            ]);
        }
        Ok(operations)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::vulkanic::backends::mock::MockBackend;

    fn desc(direction: RasterYDirection) -> WorldTargetDesc {
        WorldTargetDesc { extent: Extent3d { width: 8, height: 8, depth: 1 },
            color_format: TextureFormat::Rgba8Unorm, raster_y_direction: direction }
    }

    #[test]
    fn world_target_allocation_failure_rolls_back_every_partial_owner() {
        for failure in 0..6 {
            let mut backend = MockBackend::default();
            backend.fail_create_after = Some(failure);
            let mut gal = VulkanicGal::new_with_backend(Box::new(backend), false);
            assert!(OrientedWorldTarget::create(&mut gal, "failure", desc(RasterYDirection::Down)).is_err());
            assert_eq!(gal.metrics().resource_creates, failure as u64);
            assert_eq!(gal.metrics().resource_creates, gal.metrics().resource_destroys);
            assert!(gal.mock_backend().unwrap().live.is_empty());
            let retry = OrientedWorldTarget::create(&mut gal, "retry", desc(RasterYDirection::Down)).unwrap();
            for handle in retry.handles_in_destroy_order() { gal.destroy(handle).unwrap(); }
            assert_eq!(gal.metrics().resource_creates, gal.metrics().resource_destroys);
        }
    }

    #[test]
    fn world_target_transfer_declares_both_orientations_and_attachment_states() {
        let mut gal = VulkanicGal::new_with_backend(Box::new(MockBackend::default()), false);
        for source_direction in [RasterYDirection::Up, RasterYDirection::Down] {
            for destination_direction in [RasterYDirection::Up, RasterYDirection::Down] {
                let source = OrientedWorldTarget::create(&mut gal, "source", desc(source_direction)).unwrap();
                let destination = OrientedWorldTarget::create(&mut gal, "destination", desc(destination_direction)).unwrap();
                let operations = source.transfer_to(&destination, WorldAttachmentStates::ATTACHMENTS,
                    WorldAttachmentStates::UNDEFINED, WorldAttachmentStates::ATTACHMENTS).unwrap();
                assert_eq!(operations.len(), 8);
                for (offset, src, dst, state) in [(0, source.color_texture, destination.color_texture, TextureUsageState::ColorAttachment),
                    (4, source.depth_texture, destination.depth_texture, TextureUsageState::DepthStencilAttachment)] {
                    let CommandOp::Barrier(before) = &operations[offset] else { panic!("source barrier missing") };
                    assert_eq!((before.resource, before.before, before.after), (src, state, TextureUsageState::TransferSrc));
                    let CommandOp::CopyTexture(copy) = &operations[offset+2] else { panic!("explicit copy missing") };
                    assert_eq!((copy.src_texture, copy.dst_texture), (src, dst));
                    assert_eq!(copy.row_order, if source_direction == destination_direction { TextureRowOrder::Preserve } else { TextureRowOrder::Reverse });
                    let CommandOp::Barrier(after) = &operations[offset+3] else { panic!("destination barrier missing") };
                    assert_eq!((after.resource, after.before, after.after), (dst, TextureUsageState::TransferDst, state));
                }
                assert!(source.transfer_to(&source, WorldAttachmentStates::ATTACHMENTS,
                    WorldAttachmentStates::UNDEFINED, WorldAttachmentStates::ATTACHMENTS).is_err());
                for target in [destination, source] {
                    for handle in target.handles_in_destroy_order() { gal.destroy(handle).unwrap(); }
                }
            }
        }
        assert_eq!(gal.metrics().resource_creates, gal.metrics().resource_destroys);
    }

    #[test]
    fn borrowed_world_image_pairs_reject_aliasing_before_recording_copies() {
        let mut gal = VulkanicGal::new_with_backend(Box::new(MockBackend::default()), false);
        let source = OrientedWorldTarget::create(&mut gal, "source", desc(RasterYDirection::Down)).unwrap();
        let destination = OrientedWorldTarget::create(&mut gal, "destination", desc(RasterYDirection::Up)).unwrap();
        for (color_texture, depth_texture) in [
            (source.color_texture, destination.depth_texture),
            (destination.color_texture, source.depth_texture),
            (source.depth_texture, destination.depth_texture),
            (destination.color_texture, source.color_texture),
            (destination.depth_texture, destination.depth_texture),
            (Handle::NULL, destination.depth_texture),
        ] {
            let alias = WorldAttachmentImages { color_texture, depth_texture, ..destination.images() };
            assert!(source.images().transfer_to(alias, WorldAttachmentStates::ATTACHMENTS,
                WorldAttachmentStates::UNDEFINED, WorldAttachmentStates::ATTACHMENTS).is_err());
        }
        for owner in [destination, source] {
            for handle in owner.handles_in_destroy_order() { gal.destroy(handle).unwrap(); }
        }
        assert_eq!(gal.metrics().resource_creates, gal.metrics().resource_destroys);
    }

    #[test]
    fn world_target_resize_and_format_changes_require_distinct_compatible_owners() {
        let mut gal = VulkanicGal::new_with_backend(Box::new(MockBackend::default()), false);
        let original_desc = desc(RasterYDirection::Down);
        let original = OrientedWorldTarget::create(&mut gal, "original", original_desc).unwrap();
        for replacement_desc in [
            WorldTargetDesc { extent: Extent3d { width: 16, height: 8, depth: 1 }, ..original_desc },
            WorldTargetDesc { color_format: TextureFormat::Rgba16Float, ..original_desc },
        ] {
            let replacement = OrientedWorldTarget::create(&mut gal, "replacement", replacement_desc).unwrap();
            assert!(original.transfer_to(&replacement, WorldAttachmentStates::ATTACHMENTS,
                WorldAttachmentStates::UNDEFINED, WorldAttachmentStates::ATTACHMENTS).is_err());
            assert_eq!(gal.pass_target_extent(original.target).unwrap(), original_desc.extent);
            assert_eq!(gal.pass_target_color_format(original.target).unwrap(), original_desc.color_format);
            for handle in replacement.handles_in_destroy_order() { gal.destroy(handle).unwrap(); }
        }
        for handle in original.handles_in_destroy_order() { gal.destroy(handle).unwrap(); }
        let creates = gal.metrics().resource_creates;
        for invalid_desc in [
            WorldTargetDesc { extent: Extent3d { width: 0, height: 8, depth: 1 }, ..original_desc },
            WorldTargetDesc { extent: Extent3d { width: 8, height: 8, depth: 2 }, ..original_desc },
            WorldTargetDesc { color_format: TextureFormat::Depth32Float, ..original_desc },
        ] {
            assert!(OrientedWorldTarget::create(&mut gal, "invalid", invalid_desc).is_err());
            assert_eq!(gal.metrics().resource_creates, creates);
        }
        assert_eq!(gal.metrics().resource_creates, gal.metrics().resource_destroys);
        assert!(gal.mock_backend().unwrap().live.is_empty());
    }
}
