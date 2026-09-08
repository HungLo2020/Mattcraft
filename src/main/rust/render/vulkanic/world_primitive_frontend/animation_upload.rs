//! Explicit patch uploads for already sampled Rust-owned atlas resources.
use super::*;
use crate::render::vulkanic::sprite_interpolation::PreparedAtlasTick;

#[derive(Default)]
pub(super) struct UploadQueue {
    pending: std::collections::VecDeque<(SubmissionId, Handle, u64)>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum UploadAttempt {
    Accepted(Option<SubmissionId>),
    PendingCompletion,
}

impl UploadQueue {
    /// Reclaim only the oldest owned lease, never device-idle or unrelated work.
    pub(super) fn wait_for_oldest(&mut self, gal: &mut VulkanicGal) -> GalResult<()> {
        let submission = self.pending.front().map(|entry| entry.0)
            .ok_or_else(|| GalError::invalid_argument("animation backpressure has no owned completion"))?;
        gal.retire_through(submission)?;
        self.reap(gal)
    }

    pub(super) fn release(&mut self, gal: &mut VulkanicGal) -> GalResult<()> {
        while let Some((_, handle, _)) = self.pending.front().copied() {
            gal.destroy(handle)?;
            self.pending.pop_front();
        }
        Ok(())
    }

    pub(super) fn reap(&mut self, gal: &mut VulkanicGal) -> GalResult<()> {
        let completed = gal.poll_completed();
        gal.retire_completed()?;
        while let Some((submission, handle, _)) = self.pending.front().copied() {
            if submission > completed {
                break;
            }
            gal.destroy(handle)?;
            self.pending.pop_front();
        }
        Ok(())
    }

    /// Candidate ownership stays with the caller on rejection or backpressure.
    /// Success consumes it only after an actual submit, never a deferred append.
    pub(super) fn submit(
        &mut self,
        gal: &mut VulkanicGal,
        resources: &MeshTextureResources,
        animation: &mut super::super::sprite_interpolation::OwnedAtlasAnimationUpdate,
        retained: &mut WorldMaterialTextureAsset,
        candidate: &mut Option<PreparedAtlasTick>,
    ) -> GalResult<UploadAttempt> {
        self.reap(gal)?;
        let prepared = candidate
            .as_ref()
            .ok_or_else(|| GalError::invalid_argument("missing animation candidate"))?;
        animation.validate_commit(prepared)?;
        if retained.animation_generation != animation.generation
            || retained.width != resources.width
            || retained.height != resources.height
            || retained.mip_rgba.len() + 1 != resources.mip_levels as usize
            || resources.mip_levels == 0
            || resources.mip_levels > texture_mip_level_count(resources.width, resources.height)
        {
            return Err(GalError::invalid_argument(
                "animation retained atlas incarnation mismatch",
            ));
        }
        if retained.coordinate_origin != WorldMeshTextureCoordinateOrigin::Vulkanic {
            return Err(GalError::unsupported_feature(
                "animation uploads require canonical terrain atlas rows",
            ));
        }
        for (mip, pixels) in std::iter::once(&retained.rgba)
            .chain(&retained.mip_rgba)
            .enumerate()
        {
            let expected = u64::from((retained.width >> mip).max(1))
                .checked_mul(u64::from((retained.height >> mip).max(1)))
                .and_then(|pixels| pixels.checked_mul(4))
                .ok_or_else(|| GalError::invalid_argument("retained atlas size overflow"))?;
            if pixels.len() as u64 != expected {
                return Err(GalError::invalid_argument(
                    "retained atlas pixel extent mismatch",
                ));
            }
        }
        let bytes = prepared
            .patches()
            .iter()
            .flat_map(|patch| &patch.mip_pixels)
            .try_fold(0u64, |total, pixels| total.checked_add(pixels.len() as u64))
            .ok_or_else(|| GalError::invalid_argument("animation lease size overflow"))?;
        if bytes == 0 {
            animation.commit_tick(candidate.take().expect("validated candidate"))?;
            return Ok(UploadAttempt::Accepted(None));
        }
        if bytes > 96 * 1024 * 1024 {
            return Err(GalError::invalid_argument(
                "animation lease exceeds byte bound",
            ));
        }
        let resident = self.pending.iter().map(|entry| entry.2).sum::<u64>();
        if self.pending.len() >= 3 || resident + bytes > 96 * 1024 * 1024 {
            return Ok(UploadAttempt::PendingCompletion);
        }
        let upload = gal.create_buffer(BufferDesc {
            label: "terrain-animation.upload-lease".into(),
            size: bytes,
            memory: MemoryDomain::Upload,
            usages: vec![
                BufferUsage::HostWrite,
                BufferUsage::TransferSrc,
                BufferUsage::TransferDst,
            ],
        })?;
        let result = operations(resources, upload, bytes, prepared).and_then(|operations| {
            gal.submit(SubmissionBatch {
                label: "terrain-animation.upload".into(),
                command_lists: vec![CommandList::from(CommandListDesc {
                    label: "terrain-animation.upload.commands".into(),
                    operations,
                })],
            })
        });
        let token = match result {
            Ok(token) => token,
            Err(error) => {
                // No accepted submission owns this fresh lease. GAL handles any
                // backend failure retirement; the clock candidate is unchanged.
                let _ = gal.destroy(upload);
                return Err(error);
            }
        };
        self.pending.push_back((token.submission, upload, bytes));
        // The accepted upload and retained CPU incarnation must agree. These
        // copies cannot fail: all extents were validated before submission and
        // neither destination nor candidate can be mutated through the GAL call.
        for patch in prepared.patches() {
            for (mip, source) in patch.mip_pixels.iter().enumerate() {
                let stride = (retained.width >> mip).max(1) as usize * 4;
                let row_bytes = (patch.region.width >> mip) as usize * 4;
                let start = (patch.region.y >> mip) as usize * stride
                    + (patch.region.x >> mip) as usize * 4;
                let target = if mip == 0 {
                    &mut retained.rgba
                } else {
                    &mut retained.mip_rgba[mip - 1]
                };
                for row in 0..(patch.region.height >> mip) as usize {
                    target[start + row * stride..start + row * stride + row_bytes]
                        .copy_from_slice(&source[row * row_bytes..(row + 1) * row_bytes]);
                }
            }
        }
        animation.commit_tick(candidate.take().expect("validated candidate"))?;
        Ok(UploadAttempt::Accepted(Some(token.submission)))
    }
}

/// `upload` must be a fresh or completion-retired upload-buffer lease. This
/// lowering never reuses the atlas's initial upload buffer implicitly and does
/// not commit clocks: submission acceptance is the caller's responsibility.
pub(super) fn operations(
    resources: &MeshTextureResources,
    upload: Handle,
    capacity: u64,
    prepared: &PreparedAtlasTick,
) -> GalResult<Vec<CommandOp>> {
    if resources.width == 0
        || resources.height == 0
        || resources.mip_levels == 0
        || resources.mip_levels > texture_mip_level_count(resources.width, resources.height)
    {
        return Err(GalError::invalid_argument(
            "invalid animation upload target",
        ));
    }
    let mut total = 0u64;
    for patch in prepared.patches() {
        if patch.mip_pixels.len() != resources.mip_levels as usize {
            return Err(GalError::invalid_argument(
                "animation upload mip count mismatch",
            ));
        }
        for (level, pixels) in patch.mip_pixels.iter().enumerate() {
            let w = patch.region.width >> level;
            let h = patch.region.height >> level;
            let expected = u64::from(w) * u64::from(h) * 4;
            total = total
                .checked_add(expected)
                .ok_or_else(|| GalError::invalid_argument("animation upload size overflow"))?;
            if w == 0
                || h == 0
                || pixels.len() as u64 != expected
                || u64::from(patch.region.x >> level) + u64::from(w)
                    > u64::from((resources.width >> level).max(1))
                || u64::from(patch.region.y >> level) + u64::from(h)
                    > u64::from((resources.height >> level).max(1))
                || total > capacity
                || total > 96 * 1024 * 1024
            {
                return Err(GalError::invalid_argument(
                    "invalid or over-budget animation upload patch",
                ));
            }
        }
    }
    if total == 0 {
        return Ok(Vec::new());
    }
    let mut bytes = Vec::with_capacity(total as usize);
    let mut copies = Vec::new();
    let mut written_mips = BTreeSet::new();
    for patch in prepared.patches() {
        for (level, pixels) in patch.mip_pixels.iter().enumerate() {
            let w = patch.region.width >> level;
            let h = patch.region.height >> level;
            // GAL tracks texture hazards per subresource, not pixel rectangle.
            // Distinct sprite rectangles still require an explicit ordered
            // transfer-write dependency when they target the same mip.
            if !written_mips.insert(level) {
                copies.push(CommandOp::Barrier(texture_subresource_barrier(
                    resources.texture,
                    TextureSubresourceRange {
                        base_mip: level as u32, mip_count: 1, base_layer: 0, layer_count: 1,
                    },
                    TextureUsageState::TransferDst, TextureUsageState::TransferDst,
                )));
            }
            copies.push(CommandOp::CopyBufferToTexture(BufferImageCopyRegion {
                buffer: upload,
                buffer_offset: bytes.len() as u64,
                bytes_per_row: w * 4,
                rows_per_image: h,
                texture: resources.texture,
                texture_mip: level as u32,
                texture_layer: 0,
                texture_origin: TextureOrigin3d {
                    x: patch.region.x >> level,
                    y: patch.region.y >> level,
                    z: 0,
                },
                extent: Extent3d {
                    width: w,
                    height: h,
                    depth: 1,
                },
            }));
            bytes.extend_from_slice(pixels);
        }
    }
    let mips = TextureSubresourceRange {
        base_mip: 0,
        mip_count: resources.mip_levels,
        base_layer: 0,
        layer_count: 1,
    };
    let mut result = vec![
        CommandOp::HostWriteBuffer {
            buffer: upload,
            offset: 0,
            data: bytes,
        },
        CommandOp::Barrier(buffer_barrier(
            upload,
            TextureUsageState::TransferDst,
            TextureUsageState::TransferSrc,
        )),
        CommandOp::Barrier(texture_subresource_barrier(
            resources.texture,
            mips,
            TextureUsageState::ShaderRead,
            TextureUsageState::TransferDst,
        )),
    ];
    result.extend(copies);
    result.push(CommandOp::Barrier(texture_subresource_barrier(
        resources.texture,
        mips,
        TextureUsageState::TransferDst,
        TextureUsageState::ShaderRead,
    )));
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::vulkanic::backends::vulkan::VulkanBackend;
    use crate::render::vulkanic::sprite_interpolation::*;

    fn retained_atlas(
        width: u32,
        height: u32,
        rgba: Vec<u8>,
        mip_rgba: Vec<Vec<u8>>,
    ) -> WorldMaterialTextureAsset {
        WorldMaterialTextureAsset {
            sampling: None,
            requested_mip_levels: 0,
            width,
            height,
            rgba,
            mip_rgba,
            frame_width: width,
            frame_height: height,
            frame_count: 1,
            frame_ticks: 1,
            animation_flags: 0,
            frame_row_size: 1,
            interpolation_policy: 0,
            animation_frames: Vec::new(),
            animation_total_ticks: 1,
            animation_generation: 1,
            coordinate_origin: WorldMeshTextureCoordinateOrigin::Vulkanic,
        }
    }

    #[test]
    fn animation_upload_queue_bounds_pending_buffers_and_preserves_failed_candidates() {
        use crate::render::vulkanic::backends::mock::MockBackend;
        let mut gal = VulkanicGal::new_with_backend(Box::new(MockBackend::default()), false);
        let upload = gal
            .create_buffer(BufferDesc {
                label: "initial".into(),
                size: 8,
                memory: MemoryDomain::Upload,
                usages: vec![
                    BufferUsage::HostWrite,
                    BufferUsage::TransferDst,
                    BufferUsage::TransferSrc,
                ],
            })
            .unwrap();
        let texture = gal
            .create_texture(TextureDesc {
                label: "atlas".into(),
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba8Unorm,
                extent: Extent3d {
                    width: 2,
                    height: 1,
                    depth: 1,
                },
                mip_levels: 1,
                array_layers: 1,
                usages: vec![TextureUsage::Sampled, TextureUsage::TransferDst],
            })
            .unwrap();
        let resources = MeshTextureResources {
            upload_buffer: upload,
            texture,
            sampler: Handle::NULL,
            view: Handle::NULL,
            width: 2,
            height: 1,
            mip_levels: 1,
        };
        gal.submit(SubmissionBatch {
            label: "initial".into(),
            command_lists: vec![CommandList::from(CommandListDesc {
                label: "initial".into(),
                operations: WorldPrimitiveFrontend::mesh_texture_exact_mip_upload_ops(
                    &resources,
                    vec![vec![0; 8]],
                    2,
                    1,
                )
                .unwrap(),
            })],
        })
        .unwrap();
        let mut animation = OwnedAtlasAnimationUpdate {
            texture_id: 1,
            generation: 1,
            sprites: vec![OwnedSpriteAnimation {
                sprite_id: 1,
                region: SpriteAtlasRegion {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                },
                clock: SpriteAnimationClock::new(
                    vec![
                        SpriteAnimationFrame {
                            index: 0,
                            duration_ticks: 2,
                        },
                        SpriteAnimationFrame {
                            index: 1,
                            duration_ticks: 2,
                        },
                    ],
                    2,
                    true,
                    0,
                )
                .unwrap(),
                sheets: vec![SpriteMipSheet {
                    width: 2,
                    height: 1,
                    rgba: vec![10, 20, 30, 7, 110, 120, 130, 99],
                }],
            }],
        };
        let mut retained = retained_atlas(2, 1, vec![0; 8], vec![]);
        let mut queue = UploadQueue::default();
        for tick in 1..=3 {
            let mut pending = Some(
                animation
                    .prepare_tick(2, 1, 1, tick, &BTreeSet::from([1]), true)
                    .unwrap(),
            );
            assert!(matches!(
                queue
                    .submit(
                        &mut gal,
                        &resources,
                        &mut animation,
                        &mut retained,
                        &mut pending
                    )
                    .unwrap(),
                UploadAttempt::Accepted(Some(_))
            ));
            assert!(pending.is_none());
        }
        assert_eq!(queue.pending.len(), 3);
        assert_eq!(queue.pending.iter().map(|entry| entry.2).sum::<u64>(), 12);
        let mut pending = Some(
            animation
                .prepare_tick(2, 1, 1, 4, &BTreeSet::from([1]), true)
                .unwrap(),
        );
        let creates = gal.mock_backend().unwrap().creates.len();
        let before = retained.rgba.clone();
        assert_eq!(
            queue
                .submit(
                    &mut gal,
                    &resources,
                    &mut animation,
                    &mut retained,
                    &mut pending
                )
                .unwrap(),
            UploadAttempt::PendingCompletion
        );
        assert!(pending.is_some());
        assert_eq!(retained.rgba, before);
        assert_eq!(gal.mock_backend().unwrap().creates.len(), creates);
        gal.mock_backend_mut().unwrap().completed = queue.pending.back().unwrap().0;
        assert!(matches!(
            queue
                .submit(
                    &mut gal,
                    &resources,
                    &mut animation,
                    &mut retained,
                    &mut pending
                )
                .unwrap(),
            UploadAttempt::Accepted(Some(_))
        ));
        assert_eq!(queue.pending.len(), 1);
        let mut pending = Some(
            animation
                .prepare_tick(2, 1, 1, 5, &BTreeSet::from([1]), true)
                .unwrap(),
        );
        let live = gal.mock_backend().unwrap().live.len();
        let before = retained.rgba.clone();
        // Invalid retained storage must reject without consuming the tick or
        // creating an upload lease, so a corrected incarnation can retry it.
        retained.rgba.pop();
        assert!(
            queue
                .submit(
                    &mut gal,
                    &resources,
                    &mut animation,
                    &mut retained,
                    &mut pending
                )
                .is_err()
        );
        assert!(pending.is_some());
        assert_eq!(gal.mock_backend().unwrap().live.len(), live);
        retained.rgba = before.clone();
        retained.animation_generation += 1;
        assert!(
            queue
                .submit(
                    &mut gal,
                    &resources,
                    &mut animation,
                    &mut retained,
                    &mut pending
                )
                .is_err()
        );
        retained.animation_generation -= 1;
        assert!(pending.is_some());
        assert_eq!(retained.rgba, before);
        gal.mock_backend_mut().unwrap().fail_next_submit = true;
        assert!(
            queue
                .submit(
                    &mut gal,
                    &resources,
                    &mut animation,
                    &mut retained,
                    &mut pending
                )
                .is_err()
        );
        assert!(pending.is_some());
        assert_eq!(gal.mock_backend().unwrap().live.len(), live);
        assert_eq!(retained.rgba, before);
        assert!(matches!(
            queue
                .submit(
                    &mut gal,
                    &resources,
                    &mut animation,
                    &mut retained,
                    &mut pending
                )
                .unwrap(),
            UploadAttempt::Accepted(Some(_))
        ));
        let completed = queue.pending.back().unwrap().0;
        let live_before_release = gal.mock_backend().unwrap().live.len();
        queue.release(&mut gal).unwrap();
        assert!(queue.pending.is_empty());
        // Logical teardown must not free an in-flight backend allocation.
        assert_eq!(gal.mock_backend().unwrap().live.len(), live_before_release);
        gal.mock_backend_mut().unwrap().completed = completed;
        gal.retire_completed().unwrap();
        assert_eq!(retained.rgba, vec![60, 70, 80, 7, 0, 0, 0, 0]);
        assert!(queue.pending.is_empty());
        gal.destroy(upload).unwrap();
        gal.destroy(texture).unwrap();
        assert!(gal.mock_backend().unwrap().live.is_empty());
    }

    #[test]
    fn atlas_animation_registry_updates_two_vulkan_images_without_cross_atlas_writes() {
        let mut gal = VulkanicGal::new_with_backend(
            Box::new(VulkanBackend::new("independent atlas animation readback").unwrap()), false);
        let mut frontend = WorldPrimitiveFrontend::default();
        for (id, baseline, base) in [(101, 9, 20u8), (202, 8, 120u8)] {
            let mut asset = retained_atlas(2, 1, vec![baseline; 8], vec![]);
            asset.requested_mip_levels = 1;
            frontend.mesh_texture_assets.insert(id, asset);
            frontend.ensure_mesh_texture_resources(&mut gal, id, "atlas-isolation-test").unwrap();
            frontend.stage_atlas_animation_assets(OwnedAtlasAnimationUpdate {
                texture_id: id, generation: 1,
                sprites: vec![OwnedSpriteAnimation {
                    sprite_id: 1,
                    region: SpriteAtlasRegion { x: 0, y: 0, width: 1, height: 1 },
                    clock: SpriteAnimationClock::new(vec![
                        SpriteAnimationFrame { index: 0, duration_ticks: 2 },
                        SpriteAnimationFrame { index: 1, duration_ticks: 2 },
                    ], 2, true, 0).unwrap(),
                    sheets: vec![SpriteMipSheet { width: 2, height: 1,
                        rgba: vec![base, 0, 0, 255, base + 60, 0, 0, 255] }],
                }],
            }).unwrap();
        }
        assert_ne!(frontend.mesh_texture_resources[&101].texture, frontend.mesh_texture_resources[&202].texture);
        let read_images = |gal: &mut VulkanicGal, frontend: &WorldPrimitiveFrontend| {
            let readback = gal.create_buffer(BufferDesc {
                label: "two-atlas.readback".into(), size: 16, memory: MemoryDomain::Readback,
                usages: vec![BufferUsage::TransferDst, BufferUsage::HostRead],
            }).unwrap();
            let mut operations = Vec::new();
            for (id, offset) in [(101, 0), (202, 8)] {
                let texture = frontend.mesh_texture_resources[&id].texture;
                let range = TextureSubresourceRange { base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 };
                operations.push(CommandOp::Barrier(texture_subresource_barrier(texture, range,
                    TextureUsageState::ShaderRead, TextureUsageState::TransferSrc)));
                operations.push(CommandOp::CopyTextureToBuffer(BufferImageCopyRegion {
                    buffer: readback, buffer_offset: offset, bytes_per_row: 8, rows_per_image: 1,
                    texture, texture_mip: 0, texture_layer: 0,
                    texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
                    extent: Extent3d { width: 2, height: 1, depth: 1 },
                }));
                operations.push(CommandOp::Barrier(texture_subresource_barrier(texture, range,
                    TextureUsageState::TransferSrc, TextureUsageState::ShaderRead)));
            }
            operations.push(CommandOp::Barrier(buffer_barrier(readback,
                TextureUsageState::TransferDst, TextureUsageState::ShaderRead)));
            operations.push(CommandOp::HostReadBuffer { buffer: readback, offset: 0, size: 16 });
            let receipt = gal.submit(SubmissionBatch { label: "two-atlas.readback".into(),
                command_lists: vec![CommandList::from(CommandListDesc {
                    label: "two-atlas.readback.commands".into(), operations,
                })],
            }).unwrap();
            gal.retire_through_for_test(receipt.submission).unwrap();
            let bytes = gal.completed_host_reads().iter().rev()
                .find(|read| read.buffer == readback).unwrap().bytes.clone();
            gal.destroy(readback).unwrap();
            bytes
        };
        let event = |texture_id, tick| AtlasAnimationTickEvent {
            texture_id, generation: 1, tick, visible: BTreeSet::from([1]), animate_only_visible: true,
        };
        assert!(frontend.advance_atlas_animation(&mut gal, event(101, 1)).unwrap());
        assert_eq!(read_images(&mut gal, &frontend),
            vec![50, 0, 0, 255, 9, 9, 9, 9, 8, 8, 8, 8, 8, 8, 8, 8]);
        assert!(frontend.advance_atlas_animation(&mut gal, event(202, 1)).unwrap());
        assert_eq!(read_images(&mut gal, &frontend),
            vec![50, 0, 0, 255, 9, 9, 9, 9, 150, 0, 0, 255, 8, 8, 8, 8]);
        assert!(frontend.advance_atlas_animation(&mut gal, event(101, 2)).unwrap());
        assert_eq!(read_images(&mut gal, &frontend),
            vec![80, 0, 0, 255, 9, 9, 9, 9, 150, 0, 0, 255, 8, 8, 8, 8]);
        frontend.reset(&mut gal);
        assert!(frontend.mesh_texture_resources.is_empty());
        assert!(frontend.staged_atlas_animations.is_empty());
        assert!(frontend.atlas_animation_uploads.pending.is_empty());
    }

    #[test]
    fn animation_patch_upload_reaches_vulkan_mips_and_preserves_neighbours() {
        let mut gal = VulkanicGal::new_with_backend(
            Box::new(VulkanBackend::new("animation patch test").unwrap()),
            false,
        );
        let buffer = |gal: &mut VulkanicGal, label: &str, size, memory, usages| {
            gal.create_buffer(BufferDesc {
                label: label.into(),
                size,
                memory,
                usages,
            })
            .unwrap()
        };
        let upload = buffer(
            &mut gal,
            "initial",
            40,
            MemoryDomain::Upload,
            vec![
                BufferUsage::HostWrite,
                BufferUsage::TransferDst,
                BufferUsage::TransferSrc,
            ],
        );
        let patch_upload = buffer(
            &mut gal,
            "patch",
            20,
            MemoryDomain::Upload,
            vec![
                BufferUsage::HostWrite,
                BufferUsage::TransferDst,
                BufferUsage::TransferSrc,
            ],
        );
        let readback = buffer(
            &mut gal,
            "readback",
            40,
            MemoryDomain::Readback,
            vec![BufferUsage::TransferDst, BufferUsage::HostRead],
        );
        let texture = gal
            .create_texture(TextureDesc {
                label: "atlas".into(),
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba8Unorm,
                extent: Extent3d {
                    width: 4,
                    height: 2,
                    depth: 1,
                },
                mip_levels: 2,
                array_layers: 1,
                usages: vec![
                    TextureUsage::Sampled,
                    TextureUsage::TransferSrc,
                    TextureUsage::TransferDst,
                ],
            })
            .unwrap();
        let resources = MeshTextureResources {
            upload_buffer: upload,
            texture,
            sampler: Handle::NULL,
            view: Handle::NULL,
            width: 4,
            height: 2,
            mip_levels: 2,
        };
        let submit = |gal: &mut VulkanicGal, operations| {
            gal.submit(SubmissionBatch {
                label: "animation-test".into(),
                command_lists: vec![CommandList::from(CommandListDesc {
                    label: "animation-test.commands".into(),
                    operations,
                })],
            })
            .unwrap()
        };
        let initial = submit(
            &mut gal,
            WorldPrimitiveFrontend::mesh_texture_exact_mip_upload_ops(
                &resources,
                vec![vec![9; 32], vec![8; 8]],
                4,
                2,
            )
            .unwrap(),
        );
        gal.retire_through_for_test(initial.submission).unwrap();
        let mut animation = OwnedAtlasAnimationUpdate {
            texture_id: 1,
            generation: 1,
            sprites: vec![OwnedSpriteAnimation {
                sprite_id: 1,
                region: SpriteAtlasRegion {
                    x: 2,
                    y: 0,
                    width: 2,
                    height: 2,
                },
                clock: SpriteAnimationClock::new(
                    vec![
                        SpriteAnimationFrame {
                            index: 0,
                            duration_ticks: 2,
                        },
                        SpriteAnimationFrame {
                            index: 1,
                            duration_ticks: 2,
                        },
                    ],
                    2,
                    true,
                    0,
                )
                .unwrap(),
                sheets: vec![
                    SpriteMipSheet {
                        width: 4,
                        height: 2,
                        rgba: [
                            [10, 20, 30, 7].repeat(2),
                            [110, 120, 130, 99].repeat(2),
                            [10, 20, 30, 7].repeat(2),
                            [110, 120, 130, 99].repeat(2),
                        ]
                        .concat(),
                    },
                    SpriteMipSheet {
                        width: 2,
                        height: 1,
                        rgba: vec![0, 10, 20, 40, 200, 210, 220, 80],
                    },
                ],
            }],
        };
        let pending = animation
            .prepare_tick(4, 2, 2, 1, &BTreeSet::from([1]), true)
            .unwrap();
        assert!(operations(&resources, patch_upload, 19, &pending).is_err());
        let ops = operations(&resources, patch_upload, 20, &pending).unwrap();
        assert!(
            !ops.iter()
                .any(|op| matches!(op, CommandOp::GenerateMipmaps { .. }))
        );
        let mut candidate = Some(pending);
        let mut retained = retained_atlas(4, 2, vec![9; 32], vec![vec![8; 8]]);
        let mut queue = UploadQueue::default();
        assert!(matches!(
            queue
                .submit(
                    &mut gal,
                    &resources,
                    &mut animation,
                    &mut retained,
                    &mut candidate
                )
                .unwrap(),
            UploadAttempt::Accepted(Some(_))
        ));
        assert!(candidate.is_none());
        let read_operations = |texture, readback| {
            let mut ops = Vec::new();
            ops.push(CommandOp::Barrier(texture_subresource_barrier(
                texture,
                TextureSubresourceRange {
                    base_mip: 0,
                    mip_count: 2,
                    base_layer: 0,
                    layer_count: 1,
                },
                TextureUsageState::ShaderRead,
                TextureUsageState::TransferSrc,
            )));
            for (mip, offset, w, h) in [(0, 0, 4, 2), (1, 32, 2, 1)] {
                ops.push(CommandOp::CopyTextureToBuffer(BufferImageCopyRegion {
                    buffer: readback,
                    buffer_offset: offset,
                    bytes_per_row: w * 4,
                    rows_per_image: h,
                    texture,
                    texture_mip: mip,
                    texture_layer: 0,
                    texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
                    extent: Extent3d {
                        width: w,
                        height: h,
                        depth: 1,
                    },
                }));
            }
            ops.push(CommandOp::Barrier(buffer_barrier(
                readback,
                TextureUsageState::TransferDst,
                TextureUsageState::ShaderRead,
            )));
            ops.push(CommandOp::HostReadBuffer {
                buffer: readback,
                offset: 0,
                size: 40,
            });
            ops
        };
        let token = submit(&mut gal, read_operations(texture, readback));
        gal.retire_through_for_test(token.submission).unwrap();
        queue.reap(&mut gal).unwrap();
        assert!(queue.pending.is_empty());
        let reads = gal.completed_host_reads();
        let bytes = &reads
            .iter()
            .rev()
            .find(|read| read.buffer == readback)
            .unwrap()
            .bytes;
        assert_eq!(&bytes[..8], &[9; 8]);
        assert_eq!(&bytes[8..16], &[60, 70, 80, 7].repeat(2));
        assert_eq!(&bytes[16..24], &[9; 8]);
        assert_eq!(&bytes[24..32], &[60, 70, 80, 7].repeat(2));
        assert_eq!(&bytes[32..36], &[8; 4]);
        assert_eq!(&bytes[36..40], &[100, 110, 120, 40]);
        assert_eq!(&bytes[..32], retained.rgba.as_slice());
        assert_eq!(&bytes[32..40], retained.mip_rgba[0].as_slice());
        let expected = bytes.clone();
        for handle in [readback, patch_upload, upload, texture] {
            gal.destroy(handle).unwrap();
        }
        // Recreate from the actual frontend's retained-asset extraction path,
        // after destroying the original GPU image and all its upload leases.
        let mut frontend = WorldPrimitiveFrontend::default();
        frontend.mesh_texture_assets.insert(1, retained);
        let (levels, width, height) = frontend.world_mesh_texture_mip_bytes(1).unwrap();
        let upload = buffer(
            &mut gal,
            "recreated.upload",
            40,
            MemoryDomain::Upload,
            vec![
                BufferUsage::HostWrite,
                BufferUsage::TransferDst,
                BufferUsage::TransferSrc,
            ],
        );
        let readback = buffer(
            &mut gal,
            "recreated.readback",
            40,
            MemoryDomain::Readback,
            vec![BufferUsage::TransferDst, BufferUsage::HostRead],
        );
        let texture = gal
            .create_texture(TextureDesc {
                label: "recreated.atlas".into(),
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba8Unorm,
                extent: Extent3d {
                    width,
                    height,
                    depth: 1,
                },
                mip_levels: 2,
                array_layers: 1,
                usages: vec![
                    TextureUsage::Sampled,
                    TextureUsage::TransferSrc,
                    TextureUsage::TransferDst,
                ],
            })
            .unwrap();
        let resources = MeshTextureResources {
            upload_buffer: upload,
            texture,
            sampler: Handle::NULL,
            view: Handle::NULL,
            width,
            height,
            mip_levels: 2,
        };
        submit(
            &mut gal,
            WorldPrimitiveFrontend::mesh_texture_exact_mip_upload_ops(
                &resources, levels, width, height,
            )
            .unwrap(),
        );
        let token = submit(&mut gal, read_operations(texture, readback));
        gal.retire_through_for_test(token.submission).unwrap();
        let reads = gal.completed_host_reads();
        assert_eq!(
            reads
                .iter()
                .rev()
                .find(|read| read.buffer == readback)
                .unwrap()
                .bytes,
            expected
        );
        // Two distinct sprites write the same mip in one tick. Exercise the
        // explicit dependencies as well as exact output on a real Vulkan image.
        animation.sprites.push(OwnedSpriteAnimation {
            sprite_id: 2,
            region: SpriteAtlasRegion { x: 0, y: 0, width: 2, height: 2 },
            clock: SpriteAnimationClock::new(vec![
                SpriteAnimationFrame { index: 0, duration_ticks: 2 },
                SpriteAnimationFrame { index: 1, duration_ticks: 2 },
            ], 2, true, 1).unwrap(),
            sheets: vec![
                SpriteMipSheet { width: 4, height: 2, rgba: [
                    [20, 40, 60, 80].repeat(2), [100, 120, 140, 160].repeat(2),
                    [20, 40, 60, 80].repeat(2), [100, 120, 140, 160].repeat(2),
                ].concat() },
                SpriteMipSheet { width: 2, height: 1,
                    rgba: vec![30, 50, 70, 90, 110, 130, 150, 170] },
            ],
        });
        submit(&mut gal, vec![
            CommandOp::Barrier(texture_subresource_barrier(texture,
                TextureSubresourceRange { base_mip: 0, mip_count: 2, base_layer: 0, layer_count: 1 },
                TextureUsageState::TransferSrc, TextureUsageState::ShaderRead)),
            CommandOp::Barrier(buffer_barrier(readback,
                TextureUsageState::ShaderRead, TextureUsageState::TransferDst)),
        ]);
        let mut candidate = Some(animation.prepare_tick(4, 2, 2, 2, &BTreeSet::from([1, 2]), true).unwrap());
        assert_eq!(candidate.as_ref().unwrap().patches().len(), 2);
        assert!(matches!(queue.submit(&mut gal, &resources, &mut animation,
            frontend.mesh_texture_assets.get_mut(&1).unwrap(), &mut candidate).unwrap(),
            UploadAttempt::Accepted(Some(_))));
        let token = submit(&mut gal, read_operations(texture, readback));
        gal.retire_through_for_test(token.submission).unwrap();
        queue.reap(&mut gal).unwrap();
        assert!(queue.pending.is_empty());
        let reads = gal.completed_host_reads();
        let bytes = &reads.iter().rev().find(|read| read.buffer == readback).unwrap().bytes;
        let expected = [
            [60, 80, 100, 80].repeat(2), [110, 120, 130, 99].repeat(2),
            [60, 80, 100, 80].repeat(2), [110, 120, 130, 99].repeat(2),
            vec![70, 90, 110, 90, 200, 210, 220, 80],
        ].concat();
        assert_eq!(bytes, &expected);
        let retained = frontend.mesh_texture_assets.get(&1).unwrap();
        assert_eq!(&bytes[..32], retained.rgba.as_slice());
        assert_eq!(&bytes[32..], retained.mip_rgba[0].as_slice());
        for handle in [readback, upload, texture] {
            gal.destroy(handle).unwrap();
        }
    }
}
