//! Interpolation of immutable resource sprite pixels, wholly owned by Rust.
//! No Java objects, GPU handles, backend state, or presentation enter here.
//! Private until the terrain transport supplies complete animation semantics.

use super::{GalError, GalResult};

// Same maximum base-pixel count as the copied semantic atlas. Including every
// mip fits below 96 MiB; validate the whole request before allocating outputs.
const MAX_BASE_PIXELS: u64 = 16 * 1024 * 1024;
const MAX_OUTPUT_BYTES: usize = 96 * 1024 * 1024;

pub(crate) struct OwnedAtlasAnimationUpdate {
    pub texture_id: u32,
    pub generation: u64,
    pub sprites: Vec<OwnedSpriteAnimation>,
}

pub(crate) struct OwnedSpriteAnimation {
    pub sprite_id: u32,
    pub region: SpriteAtlasRegion,
    pub clock: SpriteAnimationClock,
    pub sheets: Vec<SpriteMipSheet>,
}

pub(crate) struct SpriteAtlasPatch {
    pub sprite_id: u32,
    pub region: SpriteAtlasRegion,
    pub mip_pixels: Vec<Vec<u8>>,
}

/// Pixel patches and uncommitted clocks for one atlas event. No authoritative
/// atlas pixels or clock state are changed while this candidate is prepared.
pub(crate) struct PreparedAtlasTick {
    texture_id: u32,
    generation: u64,
    steps: Vec<(u32, PreparedSpriteTick)>,
    patches: Vec<SpriteAtlasPatch>,
}

impl PreparedAtlasTick {
    pub(crate) fn texture_id(&self) -> u32 { self.texture_id }
    pub(crate) fn patches(&self) -> &[SpriteAtlasPatch] {
        &self.patches
    }
}

impl OwnedAtlasAnimationUpdate {
    pub(crate) fn source_metadata_counts(&self) -> GalResult<(usize, usize)> {
        self.sprites.iter().try_fold((0usize, 0usize), |(frames, mips), sprite| {
            let invalid = || GalError::invalid_argument("animation metadata count overflow");
            Ok((frames.checked_add(sprite.clock.frames.len()).ok_or_else(invalid)?,
                mips.checked_add(sprite.sheets.len()).ok_or_else(invalid)?))
        })
    }
    pub(crate) fn source_payload_bytes(&self) -> GalResult<usize> {
        self.sprites.iter().flat_map(|sprite| &sprite.sheets).try_fold(0usize, |total, sheet|
            total.checked_add(sheet.rgba.len())
                .ok_or_else(|| GalError::invalid_argument("animation source byte count overflow")))
    }
    pub(crate) fn prepare_tick(
        &self,
        width: u32,
        height: u32,
        mip_count: usize,
        tick: u64,
        visible: &std::collections::BTreeSet<u32>,
        animate_only_visible: bool,
    ) -> GalResult<PreparedAtlasTick> {
        self.validate_for_atlas(width, height, mip_count)?;
        let known = self
            .sprites
            .iter()
            .map(|sprite| sprite.sprite_id)
            .collect::<std::collections::BTreeSet<_>>();
        if !visible.is_subset(&known) {
            return Err(invalid());
        }
        let mut steps = Vec::with_capacity(self.sprites.len());
        let mut bytes = 0usize;
        for sprite in &self.sprites {
            let step = sprite.clock.prepare_tick(
                tick,
                visible.contains(&sprite.sprite_id),
                animate_only_visible,
            )?;
            if step.update().is_some() {
                for mip in 0..mip_count {
                    let size = u64::from(sprite.region.width >> mip)
                        * u64::from(sprite.region.height >> mip)
                        * 4;
                    bytes = bytes
                        .checked_add(usize::try_from(size).map_err(|_| invalid())?)
                        .ok_or_else(invalid)?;
                    if bytes > MAX_OUTPUT_BYTES {
                        return Err(invalid());
                    }
                }
            }
            steps.push((sprite.sprite_id, step));
        }
        // Pixel allocation begins only after every clock transition and the
        // aggregate patch footprint have been validated. Only changed sprites
        // get scratch pixels; the full atlas is never cloned for an animation.
        let mut patches = Vec::new();
        for (sprite, (_, step)) in self.sprites.iter().zip(&steps) {
            let Some(update) = step.update() else {
                continue;
            };
            let region = sprite.region;
            let mut pixels = (0..mip_count)
                .map(|mip| {
                    vec![
                        0;
                        (u64::from(region.width >> mip) * u64::from(region.height >> mip) * 4)
                            as usize
                    ]
                })
                .collect::<Vec<_>>();
            let local = SpriteAtlasRegion {
                x: 0,
                y: 0,
                width: region.width,
                height: region.height,
            };
            apply_sprite_sheet_update(
                region.width,
                region.height,
                &mut pixels,
                &local,
                &sprite.sheets,
                update,
            )?;
            patches.push(SpriteAtlasPatch {
                sprite_id: sprite.sprite_id,
                region,
                mip_pixels: pixels,
            });
        }
        Ok(PreparedAtlasTick {
            texture_id: self.texture_id,
            generation: self.generation,
            steps,
            patches,
        })
    }

    /// Commit after acceptance of the whole explicit upload transaction. Check
    /// all identities/cursors first, so a stale later sprite cannot partly
    /// advance the atlas. GAL still owns GPU resource completion/retirement.
    pub(crate) fn commit_tick(&mut self, prepared: PreparedAtlasTick) -> GalResult<()> {
        self.validate_commit(&prepared)?;
        for (sprite, (_, step)) in self.sprites.iter_mut().zip(prepared.steps) {
            sprite.clock.apply_accepted_step(step);
        }
        Ok(())
    }

    pub(crate) fn validate_commit(&self, prepared: &PreparedAtlasTick) -> GalResult<()> {
        if prepared.texture_id != self.texture_id
            || prepared.generation != self.generation
            || prepared.steps.len() != self.sprites.len()
            || self
                .sprites
                .iter()
                .zip(&prepared.steps)
                .any(|(sprite, (id, step))| {
                    sprite.sprite_id != *id || !sprite.clock.can_commit(step)
                })
        {
            return Err(invalid());
        }
        Ok(())
    }

    /// Validate against the actual Rust atlas incarnation before replacing a
    /// retained declaration. FFI decoding is not a substitute for this binding.
    pub(crate) fn validate_for_atlas(
        &self,
        width: u32,
        height: u32,
        mip_count: usize,
    ) -> GalResult<()> {
        if self.texture_id == 0
            || self.generation == 0
            || width == 0
            || height == 0
            || u64::from(width) * u64::from(height) > MAX_BASE_PIXELS
            || mip_count == 0
            || mip_count > (width.max(height).ilog2() + 1) as usize
            || self.sprites.len() > 16_384
        {
            return Err(invalid());
        }
        let alignment = 1u32 << (mip_count - 1);
        let mut ids = std::collections::BTreeSet::new();
        let mut bytes = 0usize;
        let mut frames = 0usize;
        let mut mips = 0usize;
        for (index, sprite) in self.sprites.iter().enumerate() {
            let r = &sprite.region;
            if sprite.sprite_id == 0
                || !ids.insert(sprite.sprite_id)
                || r.width == 0
                || r.height == 0
                || u64::from(r.x) + u64::from(r.width) > u64::from(width)
                || u64::from(r.y) + u64::from(r.height) > u64::from(height)
                || [r.x, r.y, r.width, r.height]
                    .iter()
                    .any(|v| v % alignment != 0)
                || sprite.sheets.len() != mip_count
            {
                return Err(invalid());
            }
            // Overlapping semantic sprite regions would make upload order
            // choose the resulting pixels. Reject that ambiguous declaration.
            for previous in &self.sprites[..index] {
                let p = &previous.region;
                if r.x < p.x + p.width
                    && p.x < r.x + r.width
                    && r.y < p.y + p.height
                    && p.y < r.y + r.height
                {
                    return Err(invalid());
                }
            }
            frames = frames
                .checked_add(sprite.clock.frames.len())
                .ok_or_else(invalid)?;
            mips = mips.checked_add(sprite.sheets.len()).ok_or_else(invalid)?;
            if frames > 65_536 || mips > 65_536 {
                return Err(invalid());
            }
            let mut grid = None;
            for (level, sheet) in sprite.sheets.iter().enumerate() {
                let fw = r.width >> level;
                let fh = r.height >> level;
                if sheet.width == 0
                    || sheet.height == 0
                    || sheet.width % fw != 0
                    || sheet.height % fh != 0
                {
                    return Err(invalid());
                }
                let pixels = u64::from(sheet.width) * u64::from(sheet.height);
                if pixels > MAX_OUTPUT_BYTES as u64 / 4 || pixels * 4 != sheet.rgba.len() as u64 {
                    return Err(invalid());
                }
                bytes = bytes.checked_add(sheet.rgba.len()).ok_or_else(invalid)?;
                if bytes > MAX_OUTPUT_BYTES {
                    return Err(invalid());
                }
                let shape = (sheet.width / fw, sheet.height / fh);
                if grid.is_some_and(|previous| previous != shape) {
                    return Err(invalid());
                }
                grid = Some(shape);
                let count = u64::from(shape.0) * u64::from(shape.1);
                if sprite
                    .clock
                    .frames
                    .iter()
                    .any(|frame| u64::from(frame.index) >= count)
                {
                    return Err(invalid());
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub(crate) struct SpriteAnimationFrame {
    pub index: u32,
    pub duration_ticks: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SpriteFrameUpdate {
    Copy {
        index: u32,
    },
    Interpolate {
        current: u32,
        next: u32,
        subframe: u32,
        duration: u32,
    },
}

/// Resource-owned animation state. The producer sends game tick identity and
/// semantic visibility, never a selected frame or a backend upload decision.
/// Displayed pixels are deliberately not inferred from absolute time: Frozen
/// advances invisible animations but leaves their previously uploaded pixels.
pub(crate) struct SpriteAnimationClock {
    frames: std::sync::Arc<[SpriteAnimationFrame]>,
    interpolate: bool,
    frame: usize,
    subframe: u32,
    last_tick: u64,
    // Observation of pixels, not another clock. Invisible ticks must not
    // relabel the last accepted upload with a later simulation phase.
    retained_pixel_state: (usize, u32, u64),
}

/// An uncommitted clock transition. Retains only the immutable timeline, not
/// image pixels or backend resources. Dropping a rejected upload's transition
/// leaves the clock untouched. Its identity prevents use after atlas replacement.
pub(crate) struct PreparedSpriteTick {
    timeline: std::sync::Arc<[SpriteAnimationFrame]>,
    previous: (usize, u32, u64),
    next: (usize, u32, u64),
    update: Option<SpriteFrameUpdate>,
}

impl PreparedSpriteTick {
    pub(crate) fn update(&self) -> Option<SpriteFrameUpdate> {
        self.update
    }
}

impl SpriteAnimationClock {
    /// Observation only: declared frame position, sheet frame, subframe and accepted tick.
    pub(crate) fn diagnostic_state(&self) -> (usize, u32, u32, u64) {
        (self.frame, self.frames[self.frame].index, self.subframe, self.last_tick)
    }

    pub(crate) fn diagnostic_retained_pixel_state(&self) -> (usize, u32, u32, u64) {
        let (frame, subframe, tick) = self.retained_pixel_state;
        (frame, self.frames[frame].index, subframe, tick)
    }

    pub(crate) fn declared_frame_count(&self) -> usize {
        self.frames.len()
    }

    pub(crate) fn new(
        frames: Vec<SpriteAnimationFrame>,
        sheet_frame_count: u32,
        interpolate: bool,
        initial_tick: u64,
    ) -> GalResult<Self> {
        if frames.is_empty()
            || frames.len() > 16_384
            || sheet_frame_count == 0
            || frames.iter().any(|frame| {
                frame.index >= sheet_frame_count
                    || frame.duration_ticks == 0
                    || frame.duration_ticks > i32::MAX as u32
            })
        {
            return Err(invalid());
        }
        Ok(Self {
            frames: frames.into(),
            interpolate,
            frame: 0,
            subframe: 0,
            last_tick: initial_tick,
            retained_pixel_state: (0, 0, initial_tick),
        })
    }

    pub(crate) fn initial_frame(&self) -> SpriteFrameUpdate {
        SpriteFrameUpdate::Copy {
            index: self.frames[0].index,
        }
    }

    /// Exactly one game tick, with repeated presentations of that tick a no-op.
    /// Missing ticks cannot be reconstructed without their visibility history;
    /// reject gaps before changing state instead of inventing upload history.
    #[cfg(test)]
    pub(crate) fn tick(
        &mut self,
        tick: u64,
        visible: bool,
        animate_only_visible: bool,
    ) -> GalResult<Option<SpriteFrameUpdate>> {
        let prepared = self.prepare_tick(tick, visible, animate_only_visible)?;
        let update = prepared.update();
        self.commit_tick(prepared)?;
        Ok(update)
    }

    pub(crate) fn prepare_tick(
        &self,
        tick: u64,
        visible: bool,
        animate_only_visible: bool,
    ) -> GalResult<PreparedSpriteTick> {
        let previous = (self.frame, self.subframe, self.last_tick);
        if tick == self.last_tick {
            return Ok(PreparedSpriteTick {
                timeline: self.frames.clone(),
                previous,
                next: previous,
                update: None,
            });
        }
        if self.last_tick.checked_add(1) != Some(tick) {
            return Err(invalid());
        }
        let current = self.frames[self.frame];
        let mut frame = self.frame;
        let mut subframe = self.subframe + 1;
        let update = if subframe >= current.duration_ticks {
            frame = (frame + 1) % self.frames.len();
            subframe = 0;
            let next = self.frames[frame];
            ((!animate_only_visible || visible) && current.index != next.index)
                .then_some(SpriteFrameUpdate::Copy { index: next.index })
        } else {
            let next = self.frames[(frame + 1) % self.frames.len()];
            ((!animate_only_visible || visible) && self.interpolate && current.index != next.index)
                .then_some(SpriteFrameUpdate::Interpolate {
                    current: current.index,
                    next: next.index,
                    subframe,
                    duration: current.duration_ticks,
                })
        };
        Ok(PreparedSpriteTick {
            timeline: self.frames.clone(),
            previous,
            next: (frame, subframe, tick),
            update,
        })
    }

    /// Call only when the owning atlas upload transaction has been accepted.
    /// The upload buffer/image still obey GAL submission/completion lifetimes.
    pub(crate) fn commit_tick(&mut self, prepared: PreparedSpriteTick) -> GalResult<()> {
        if !self.can_commit(&prepared) {
            return Err(GalError::invalid_argument(
                "stale or foreign sprite clock transaction",
            ));
        }
        self.apply_accepted_step(prepared);
        Ok(())
    }

    fn apply_accepted_step(&mut self, prepared: PreparedSpriteTick) {
        if prepared.update.is_some() {
            self.retained_pixel_state = prepared.next;
        }
        (self.frame, self.subframe, self.last_tick) = prepared.next;
    }

    fn can_commit(&self, prepared: &PreparedSpriteTick) -> bool {
        std::sync::Arc::ptr_eq(&self.frames, &prepared.timeline)
            && prepared.previous == (self.frame, self.subframe, self.last_tick)
    }
}

pub(crate) struct SpriteMipFrames<'a> {
    pub current_rgba: &'a [u8],
    pub next_rgba: &'a [u8],
}

/// Owned immutable resource pixels, in RGBA order, including the entire frame
/// sheet. No Java image or borrowed native allocation survives this boundary.
pub(crate) struct SpriteMipSheet {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Applies a Rust clock decision directly from full source sheets. This is the
/// resource path needed by the Java declaration extractor: no per-tick sprite
/// crops, PNG encodes, or temporary interpolated images are necessary.
pub(crate) fn apply_sprite_sheet_update(
    atlas_width: u32,
    atlas_height: u32,
    atlas_mips: &mut [Vec<u8>],
    region: &SpriteAtlasRegion,
    sheets: &[SpriteMipSheet],
    update: SpriteFrameUpdate,
) -> GalResult<()> {
    if atlas_width == 0
        || atlas_height == 0
        || region.width == 0
        || region.height == 0
        || u64::from(atlas_width) * u64::from(atlas_height) > MAX_BASE_PIXELS
        || sheets.is_empty()
        || sheets.len() != atlas_mips.len()
        || sheets.len() > (region.width.min(region.height).ilog2() + 1) as usize
    {
        return Err(invalid());
    }
    let alignment = 1u32 << (sheets.len() - 1);
    if [region.x, region.y, region.width, region.height]
        .iter()
        .any(|v| v % alignment != 0)
    {
        return Err(invalid());
    }
    let (current, next, weight) = match update {
        SpriteFrameUpdate::Copy { index } => (index, index, 255),
        SpriteFrameUpdate::Interpolate {
            current,
            next,
            subframe,
            duration,
        } => {
            if duration == 0 || duration > i32::MAX as u32 || subframe >= duration {
                return Err(invalid());
            }
            (
                current,
                next,
                ((1.0_f32 - subframe as f32 / duration as f32) * 255.0) as u32,
            )
        }
    };
    let mut sheet_bytes = 0u64;
    let mut atlas_bytes = 0u64;
    let mut frame_grid = None;
    // Entire-request validation precedes mutation, including frame indices and
    // matching sheet grids at every mip. A bad later level cannot corrupt mip 0.
    for (mip, (sheet, atlas)) in sheets.iter().zip(atlas_mips.iter()).enumerate() {
        let frame_w = region.width >> mip;
        let frame_h = region.height >> mip;
        let atlas_w = (atlas_width >> mip).max(1);
        let atlas_h = (atlas_height >> mip).max(1);
        if sheet.width == 0
            || sheet.height == 0
            || sheet.width % frame_w != 0
            || sheet.height % frame_h != 0
        {
            return Err(invalid());
        }
        let pixels = u64::from(sheet.width) * u64::from(sheet.height);
        if pixels > MAX_OUTPUT_BYTES as u64 / 4 {
            return Err(invalid());
        }
        sheet_bytes = sheet_bytes.checked_add(pixels * 4).ok_or_else(invalid)?;
        let atlas_len = u64::from(atlas_w) * u64::from(atlas_h) * 4;
        atlas_bytes = atlas_bytes.checked_add(atlas_len).ok_or_else(invalid)?;
        let grid = (sheet.width / frame_w, sheet.height / frame_h);
        let count = u64::from(grid.0) * u64::from(grid.1);
        if sheet.rgba.len() as u64 != pixels * 4
            || atlas.len() as u64 != atlas_len
            || sheet_bytes > MAX_OUTPUT_BYTES as u64
            || atlas_bytes > MAX_OUTPUT_BYTES as u64
            || u64::from(current) >= count
            || u64::from(next) >= count
            || frame_grid.is_some_and(|previous| previous != grid)
            || u64::from(region.x >> mip) + u64::from(frame_w) > u64::from(atlas_w)
            || u64::from(region.y >> mip) + u64::from(frame_h) > u64::from(atlas_h)
        {
            return Err(invalid());
        }
        frame_grid = Some(grid);
    }
    for (mip, (sheet, atlas)) in sheets.iter().zip(atlas_mips.iter_mut()).enumerate() {
        let frame_w = (region.width >> mip) as usize;
        let frame_h = (region.height >> mip) as usize;
        let columns = sheet.width as usize / frame_w;
        let sheet_stride = sheet.width as usize * 4;
        let atlas_stride = (atlas_width >> mip).max(1) as usize * 4;
        let frame_start = |index: u32| {
            (index as usize / columns) * frame_h * sheet_stride
                + (index as usize % columns) * frame_w * 4
        };
        let current_start = frame_start(current);
        let next_start = frame_start(next);
        let target_start =
            (region.y >> mip) as usize * atlas_stride + (region.x >> mip) as usize * 4;
        for y in 0..frame_h {
            let a = current_start + y * sheet_stride;
            let b = next_start + y * sheet_stride;
            let target = target_start + y * atlas_stride;
            interpolate_pixels(
                &sheet.rgba[a..a + frame_w * 4],
                &sheet.rgba[b..b + frame_w * 4],
                weight,
                &mut atlas[target..target + frame_w * 4],
            );
        }
    }
    Ok(())
}

/// The caller supplies game animation time, not a precomputed blend policy.
/// Frame boundaries select the next frame upstream; interpolation accepts only
/// times strictly inside the current frame, including its initial time zero.
pub(crate) fn interpolate_sprite_mips(
    width: u32,
    height: u32,
    subframe: u32,
    duration: u32,
    levels: &[SpriteMipFrames<'_>],
) -> GalResult<Vec<Vec<u8>>> {
    let weight = validate_sprite_mips(width, height, subframe, duration, levels)?;
    Ok(levels
        .iter()
        .map(|frames| {
            let mut output = vec![0; frames.current_rgba.len()];
            interpolate_pixels(frames.current_rgba, frames.next_rgba, weight, &mut output);
            output
        })
        .collect())
}

fn validate_sprite_mips(
    width: u32,
    height: u32,
    subframe: u32,
    duration: u32,
    levels: &[SpriteMipFrames<'_>],
) -> GalResult<u32> {
    if width == 0
        || height == 0
        || u64::from(width) * u64::from(height) > MAX_BASE_PIXELS
        || duration == 0
        || duration > i32::MAX as u32
        || subframe >= duration
        || levels.is_empty()
        || levels.len() > (width.max(height).ilog2() + 1) as usize
    {
        return Err(invalid());
    }
    let mut total = 0usize;
    for (mip, frames) in levels.iter().enumerate() {
        let bytes = usize::try_from(
            u64::from((width >> mip).max(1)) * u64::from((height >> mip).max(1)) * 4,
        )
        .map_err(|_| invalid())?;
        total = total.checked_add(bytes).ok_or_else(invalid)?;
        if total > MAX_OUTPUT_BYTES
            || frames.current_rgba.len() != bytes
            || frames.next_rgba.len() != bytes
        {
            return Err(invalid());
        }
    }
    // Frozen SpriteContents uses f32 subframe division, then ColorU8 truncates
    // the current-frame weight to eight bits. Do not replace with float RGB
    // interpolation or round-to-nearest weight conversion.
    Ok(((1.0_f32 - subframe as f32 / duration as f32) * 255.0) as u32)
}

fn interpolate_pixels(current: &[u8], next: &[u8], weight: u32, output: &mut [u8]) {
    for ((current, next), pixel) in current
        .chunks_exact(4)
        .zip(next.chunks_exact(4))
        .zip(output.chunks_exact_mut(4))
    {
        for channel in 0..3 {
            pixel[channel] = ((u32::from(current[channel]) * weight
                + u32::from(next[channel]) * (255 - weight)
                + 255)
                >> 8) as u8;
        }
        pixel[3] = current[3];
    }
}

/// Immutable stitched resource region, expressed in base-mip texels.
#[derive(Clone, Copy)]
pub(crate) struct SpriteAtlasRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Updates a Rust-owned CPU atlas without making another full atlas or sprite
/// allocation. All mip payloads and destinations are checked before any write,
/// so a malformed later mip cannot partially update the currently owned image.
/// The resulting pixels still require an explicit GAL upload transaction.
pub(crate) fn interpolate_sprite_into_atlas(
    atlas_width: u32,
    atlas_height: u32,
    atlas_mips: &mut [Vec<u8>],
    region: &SpriteAtlasRegion,
    subframe: u32,
    duration: u32,
    frames: &[SpriteMipFrames<'_>],
) -> GalResult<()> {
    let weight = validate_sprite_mips(region.width, region.height, subframe, duration, frames)?;
    if atlas_width == 0
        || atlas_height == 0
        || u64::from(atlas_width) * u64::from(atlas_height) > MAX_BASE_PIXELS
        || atlas_mips.len() != frames.len()
        || atlas_mips.len() > (atlas_width.max(atlas_height).ilog2() + 1) as usize
    {
        return Err(invalid());
    }
    // Stitched mip regions must not alias their neighbours after shifting.
    let alignment = 1u32 << (atlas_mips.len() - 1);
    if [region.x, region.y, region.width, region.height]
        .iter()
        .any(|value| value % alignment != 0)
    {
        return Err(invalid());
    }
    let mut total = 0usize;
    for (mip, pixels) in atlas_mips.iter().enumerate() {
        let width = (atlas_width >> mip).max(1);
        let height = (atlas_height >> mip).max(1);
        let bytes =
            usize::try_from(u64::from(width) * u64::from(height) * 4).map_err(|_| invalid())?;
        total = total.checked_add(bytes).ok_or_else(invalid)?;
        if pixels.len() != bytes
            || total > MAX_OUTPUT_BYTES
            || u64::from(region.x >> mip) + u64::from(region.width >> mip) > u64::from(width)
            || u64::from(region.y >> mip) + u64::from(region.height >> mip) > u64::from(height)
        {
            return Err(invalid());
        }
    }
    for (mip, (pixels, frames)) in atlas_mips.iter_mut().zip(frames).enumerate() {
        let stride = (atlas_width >> mip).max(1) as usize * 4;
        let row_bytes = (region.width >> mip) as usize * 4;
        let x = (region.x >> mip) as usize * 4;
        let y = (region.y >> mip) as usize;
        for row in 0..(region.height >> mip) as usize {
            let source = row * row_bytes;
            let destination = (y + row) * stride + x;
            interpolate_pixels(
                &frames.current_rgba[source..source + row_bytes],
                &frames.next_rgba[source..source + row_bytes],
                weight,
                &mut pixels[destination..destination + row_bytes],
            );
        }
    }
    Ok(())
}

fn invalid() -> GalError {
    GalError::invalid_argument("invalid or over-budget semantic sprite interpolation")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_pixel_phase_changes_only_when_an_upload_transition_is_committed() {
        let mut atlas = two_sprite_atlas();
        let visible = std::collections::BTreeSet::from([1]);
        assert_eq!(atlas.sprites[0].clock.diagnostic_retained_pixel_state(), (0, 0, 0, 0));
        let rejected = atlas.prepare_tick(2, 1, 1, 1, &visible, true).unwrap();
        drop(rejected);
        assert_eq!(atlas.sprites[0].clock.diagnostic_retained_pixel_state(), (0, 0, 0, 0));
        let accepted = atlas.prepare_tick(2, 1, 1, 1, &visible, true).unwrap();
        atlas.commit_tick(accepted).unwrap();
        assert_eq!(atlas.sprites[0].clock.diagnostic_retained_pixel_state(), (0, 0, 1, 1));
        assert_eq!(atlas.sprites[1].clock.diagnostic_retained_pixel_state(), (0, 0, 0, 0));
        let invisible = atlas.prepare_tick(2, 1, 1, 2, &Default::default(), true).unwrap();
        atlas.commit_tick(invisible).unwrap();
        assert_eq!(atlas.sprites[0].clock.diagnostic_state(), (1, 1, 0, 2));
        assert_eq!(atlas.sprites[0].clock.diagnostic_retained_pixel_state(), (0, 0, 1, 1));
        let mut other = two_sprite_atlas();
        let foreign = other.prepare_tick(2, 1, 1, 1, &visible, true).unwrap();
        assert!(atlas.commit_tick(foreign).is_err());
        assert_eq!(atlas.sprites[0].clock.diagnostic_retained_pixel_state(), (0, 0, 1, 1));
        // The single-clock transaction path follows the same acceptance rule.
        let clock = &mut other.sprites[0].clock;
        let step = clock.prepare_tick(1, true, true).unwrap();
        clock.commit_tick(step).unwrap();
        assert_eq!(clock.diagnostic_retained_pixel_state(), (0, 0, 1, 1));
        clock.tick(2, false, true).unwrap();
        assert_eq!(clock.diagnostic_retained_pixel_state(), (0, 0, 1, 1));
    }

    fn two_sprite_atlas() -> OwnedAtlasAnimationUpdate {
        OwnedAtlasAnimationUpdate {
            texture_id: 1,
            generation: 7,
            sprites: (0..2)
                .map(|x| OwnedSpriteAnimation {
                    sprite_id: x + 1,
                    region: SpriteAtlasRegion {
                        x,
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
                })
                .collect(),
        }
    }

    #[test]
    fn sprite_interpolation_atlas_tick_prepares_only_changed_patches_without_advancing() {
        let mut atlas = two_sprite_atlas();
        let visible = std::collections::BTreeSet::from([1]);
        let candidate = atlas.prepare_tick(2, 1, 1, 1, &visible, true).unwrap();
        assert_eq!(candidate.patches().len(), 1);
        assert_eq!(candidate.patches()[0].sprite_id, 1);
        assert_eq!(candidate.patches()[0].region.x, 0);
        assert_eq!(candidate.patches()[0].mip_pixels, vec![vec![60, 70, 80, 7]]);
        assert!(
            atlas
                .sprites
                .iter()
                .all(|sprite| sprite.clock.last_tick == 0)
        );
        assert_eq!(
            atlas.sprites[0].sheets[0].rgba,
            [10, 20, 30, 7, 110, 120, 130, 99]
        );
        drop(candidate); // upload rejected/cancelled: same tick remains retryable
        let retry = atlas.prepare_tick(2, 1, 1, 1, &visible, true).unwrap();
        atlas.commit_tick(retry).unwrap();
        assert!(
            atlas
                .sprites
                .iter()
                .all(|sprite| sprite.clock.last_tick == 1)
        );
        assert!(
            atlas
                .prepare_tick(2, 1, 1, 1, &visible, true)
                .unwrap()
                .patches()
                .is_empty()
        );
        let invisible = atlas
            .prepare_tick(2, 1, 1, 2, &std::collections::BTreeSet::new(), true)
            .unwrap();
        assert!(invisible.patches().is_empty());
        atlas.commit_tick(invisible).unwrap();
        assert!(
            atlas
                .sprites
                .iter()
                .all(|sprite| sprite.clock.last_tick == 2)
        );
        assert!(
            atlas
                .prepare_tick(2, 1, 1, 3, &std::collections::BTreeSet::from([99]), true)
                .is_err()
        );
    }

    #[test]
    fn sprite_interpolation_atlas_commit_checks_every_clock_before_mutating_any() {
        let mut atlas = two_sprite_atlas();
        let visible = std::collections::BTreeSet::from([1, 2]);
        let pending = atlas.prepare_tick(2, 1, 1, 1, &visible, true).unwrap();
        // Make only the later sprite stale. No earlier sprite may be committed.
        let step = atlas.sprites[1].clock.prepare_tick(1, true, true).unwrap();
        atlas.sprites[1].clock.commit_tick(step).unwrap();
        assert!(atlas.commit_tick(pending).is_err());
        assert_eq!(atlas.sprites[0].clock.last_tick, 0);
        assert_eq!(atlas.sprites[1].clock.last_tick, 1);
        let original = two_sprite_atlas();
        let mut replacement = two_sprite_atlas();
        let pending = original.prepare_tick(2, 1, 1, 1, &visible, true).unwrap();
        assert!(replacement.commit_tick(pending).is_err());
        assert!(
            replacement
                .sprites
                .iter()
                .all(|sprite| sprite.clock.last_tick == 0)
        );
    }

    #[test]
    fn sprite_interpolation_clock_commit_is_retryable_and_rejects_foreign_or_stale_plans() {
        let make_clock = || {
            SpriteAnimationClock::new(
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
                40,
            )
            .unwrap()
        };
        let mut clock = make_clock();
        let rejected = clock.prepare_tick(41, true, true).unwrap();
        assert_eq!(
            rejected.update(),
            Some(SpriteFrameUpdate::Interpolate {
                current: 0,
                next: 1,
                subframe: 1,
                duration: 2,
            })
        );
        // A later game tick cannot bypass an uncommitted upload.
        assert!(clock.prepare_tick(42, true, true).is_err());
        drop(rejected);
        let retry = clock.prepare_tick(41, true, true).unwrap();
        let stale = clock.prepare_tick(41, true, true).unwrap();
        // Identical metadata on a new atlas incarnation is not the same clock.
        let mut replacement = make_clock();
        assert!(
            replacement
                .commit_tick(clock.prepare_tick(41, true, true).unwrap())
                .is_err()
        );
        assert!(replacement.prepare_tick(42, true, true).is_err());
        clock.commit_tick(retry).unwrap();
        assert!(clock.commit_tick(stale).is_err());
        assert_eq!(clock.prepare_tick(41, true, true).unwrap().update(), None);
        let next = clock.prepare_tick(42, true, true).unwrap();
        assert_eq!(next.update(), Some(SpriteFrameUpdate::Copy { index: 1 }));
        clock.commit_tick(next).unwrap();
        assert!(clock.prepare_tick(41, true, true).is_err());
    }

    #[test]
    fn diagnostic_clock_observation_does_not_commit_prepared_ticks() {
        let mut clock = SpriteAnimationClock::new(vec![SpriteAnimationFrame {
            index: 2, duration_ticks: 4,
        }], 3, true, 0).unwrap();
        assert_eq!(clock.diagnostic_state(), (0, 2, 0, 0));
        let pending = clock.prepare_tick(1, true, true).unwrap();
        assert_eq!(clock.diagnostic_state(), (0, 2, 0, 0));
        clock.commit_tick(pending).unwrap();
        assert_eq!(clock.diagnostic_state(), (0, 2, 1, 1));
        assert_eq!(clock.diagnostic_state(), (0, 2, 1, 1));
    }

    #[test]
    fn sprite_interpolation_rejected_pixel_update_does_not_consume_clock_tick() {
        let mut clock = SpriteAnimationClock::new(
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
        .unwrap();
        let sheets = [SpriteMipSheet {
            width: 2,
            height: 1,
            rgba: vec![10, 20, 30, 7, 110, 120, 130, 99],
        }];
        let region = SpriteAtlasRegion {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        };
        let mut atlas = vec![vec![0; 3]];
        let candidate = clock.prepare_tick(1, true, true).unwrap();
        assert!(
            apply_sprite_sheet_update(
                1,
                1,
                &mut atlas,
                &region,
                &sheets,
                candidate.update().unwrap()
            )
            .is_err()
        );
        drop(candidate);
        atlas[0].push(0);
        let retry = clock.prepare_tick(1, true, true).unwrap();
        apply_sprite_sheet_update(1, 1, &mut atlas, &region, &sheets, retry.update().unwrap())
            .unwrap();
        clock.commit_tick(retry).unwrap();
        assert_eq!(atlas[0], [60, 70, 80, 7]);
    }

    #[test]
    fn sprite_interpolation_clock_updates_owned_atlas_from_full_sheets() {
        let mut clock = SpriteAnimationClock::new(
            vec![
                SpriteAnimationFrame {
                    index: 1,
                    duration_ticks: 2,
                },
                SpriteAnimationFrame {
                    index: 0,
                    duration_ticks: 1,
                },
            ],
            2,
            true,
            0,
        )
        .unwrap();
        // Two horizontal frames, with independent per-level source pixels.
        let sheets = vec![
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
        ];
        let region = SpriteAtlasRegion {
            x: 2,
            y: 0,
            width: 2,
            height: 2,
        };
        let mut atlas = vec![vec![9; 32], vec![8; 8]];
        apply_sprite_sheet_update(4, 2, &mut atlas, &region, &sheets, clock.initial_frame())
            .unwrap();
        assert_eq!(&atlas[0][8..16], &[110, 120, 130, 99].repeat(2));
        apply_sprite_sheet_update(
            4,
            2,
            &mut atlas,
            &region,
            &sheets,
            clock.tick(1, true, true).unwrap().unwrap(),
        )
        .unwrap();
        assert_eq!(&atlas[0][8..16], &[60, 70, 80, 99].repeat(2));
        assert_eq!(&atlas[1][4..8], &[100, 110, 120, 80]);
        assert_eq!(&atlas[0][..8], &[9; 8]);
        apply_sprite_sheet_update(
            4,
            2,
            &mut atlas,
            &region,
            &sheets,
            clock.tick(2, true, true).unwrap().unwrap(),
        )
        .unwrap();
        assert_eq!(&atlas[0][8..16], &[10, 20, 30, 7].repeat(2));
    }

    #[test]
    fn sprite_interpolation_sheet_grid_and_frame_errors_are_atomic() {
        let mut sheets = vec![
            SpriteMipSheet {
                width: 4,
                height: 2,
                rgba: vec![0; 32],
            },
            SpriteMipSheet {
                width: 3,
                height: 1,
                rgba: vec![0; 12],
            },
        ];
        let region = SpriteAtlasRegion {
            x: 0,
            y: 0,
            width: 2,
            height: 2,
        };
        let mut atlas = vec![vec![9; 16], vec![8; 4]];
        let original = atlas.clone();
        assert!(
            apply_sprite_sheet_update(
                2,
                2,
                &mut atlas,
                &region,
                &sheets,
                SpriteFrameUpdate::Copy { index: 0 }
            )
            .is_err()
        );
        assert_eq!(atlas, original);
        sheets[1] = SpriteMipSheet {
            width: 2,
            height: 1,
            rgba: vec![0; 8],
        };
        assert!(
            apply_sprite_sheet_update(
                2,
                2,
                &mut atlas,
                &region,
                &sheets,
                SpriteFrameUpdate::Copy { index: 2 }
            )
            .is_err()
        );
        assert_eq!(atlas, original);
    }

    #[test]
    fn sprite_interpolation_clock_preserves_variable_durations_repeats_and_visibility() {
        let mut clock = SpriteAnimationClock::new(
            vec![
                SpriteAnimationFrame {
                    index: 2,
                    duration_ticks: 3,
                },
                SpriteAnimationFrame {
                    index: 2,
                    duration_ticks: 2,
                },
                SpriteAnimationFrame {
                    index: 0,
                    duration_ticks: 2,
                },
            ],
            3,
            true,
            100,
        )
        .unwrap();
        assert_eq!(clock.initial_frame(), SpriteFrameUpdate::Copy { index: 2 });
        assert_eq!(clock.tick(101, true, true).unwrap(), None); // repeated image
        assert_eq!(clock.tick(102, true, true).unwrap(), None);
        assert_eq!(clock.tick(103, true, true).unwrap(), None); // same-index boundary
        assert_eq!(
            clock.tick(104, true, true).unwrap(),
            Some(SpriteFrameUpdate::Interpolate {
                current: 2,
                next: 0,
                subframe: 1,
                duration: 2,
            })
        );
        assert_eq!(clock.tick(105, false, true).unwrap(), None); // advance, no upload
        assert_eq!(clock.tick(105, true, true).unwrap(), None); // presentation isn't a tick
        assert_eq!(
            clock.tick(106, true, true).unwrap(),
            Some(SpriteFrameUpdate::Interpolate {
                current: 0,
                next: 2,
                subframe: 1,
                duration: 2,
            })
        );
        assert_eq!(
            clock.tick(107, true, true).unwrap(),
            Some(SpriteFrameUpdate::Copy { index: 2 })
        );
    }

    #[test]
    fn sprite_interpolation_clock_rejects_gaps_and_backward_time_without_advancing() {
        let mut clock = SpriteAnimationClock::new(
            vec![
                SpriteAnimationFrame {
                    index: 0,
                    duration_ticks: 1,
                },
                SpriteAnimationFrame {
                    index: 1,
                    duration_ticks: 1,
                },
            ],
            2,
            false,
            10,
        )
        .unwrap();
        assert!(clock.tick(12, true, false).is_err());
        assert!(clock.tick(9, true, false).is_err());
        assert_eq!(
            clock.tick(11, false, false).unwrap(),
            Some(SpriteFrameUpdate::Copy { index: 1 })
        );
        assert_eq!(
            clock.tick(12, false, false).unwrap(),
            Some(SpriteFrameUpdate::Copy { index: 0 })
        );
        assert!(SpriteAnimationClock::new(vec![], 1, false, 0).is_err());
        for frame in [
            SpriteAnimationFrame {
                index: 1,
                duration_ticks: 1,
            },
            SpriteAnimationFrame {
                index: 0,
                duration_ticks: 0,
            },
            SpriteAnimationFrame {
                index: 0,
                duration_ticks: u32::MAX,
            },
        ] {
            assert!(SpriteAnimationClock::new(vec![frame], 1, false, 0).is_err());
        }
    }

    #[test]
    fn sprite_interpolation_atlas_patch_preserves_neighbours_and_allocations() {
        let mut atlas = vec![vec![9; 8 * 4 * 4], vec![8; 4 * 2 * 4]];
        let pointers = atlas.iter().map(|mip| mip.as_ptr()).collect::<Vec<_>>();
        let current = [10, 20, 30, 7].repeat(8);
        let next = [110, 120, 130, 255].repeat(8);
        let mip = [255, 0, 10, 99].repeat(2);
        let mip_next = [0, 255, 100, 0].repeat(2);
        interpolate_sprite_into_atlas(
            8,
            4,
            &mut atlas,
            &SpriteAtlasRegion {
                x: 2,
                y: 2,
                width: 4,
                height: 2,
            },
            1,
            2,
            &[
                SpriteMipFrames {
                    current_rgba: &current,
                    next_rgba: &next,
                },
                SpriteMipFrames {
                    current_rgba: &mip,
                    next_rgba: &mip_next,
                },
            ],
        )
        .unwrap();
        for y in 0..4 {
            for x in 0..8 {
                let offset = (y * 8 + x) * 4;
                let expected = if y >= 2 && (2..6).contains(&x) {
                    [60, 70, 80, 7]
                } else {
                    [9; 4]
                };
                assert_eq!(&atlas[0][offset..offset + 4], &expected);
            }
        }
        assert_eq!(&atlas[1][20..28], &[127, 128, 55, 99].repeat(2));
        assert!(
            atlas[1][..20]
                .iter()
                .chain(&atlas[1][28..])
                .all(|byte| *byte == 8)
        );
        assert_eq!(
            pointers,
            atlas.iter().map(|mip| mip.as_ptr()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn sprite_interpolation_atlas_rejection_never_partially_writes() {
        let base = [0; 16];
        let mip = [0; 4];
        let frames = [
            SpriteMipFrames {
                current_rgba: &base,
                next_rgba: &base,
            },
            SpriteMipFrames {
                current_rgba: &mip,
                next_rgba: &mip,
            },
        ];
        for region in [
            SpriteAtlasRegion {
                x: 1,
                y: 0,
                width: 2,
                height: 2,
            },
            SpriteAtlasRegion {
                x: 4,
                y: 0,
                width: 2,
                height: 2,
            },
            SpriteAtlasRegion {
                x: u32::MAX - 1,
                y: 0,
                width: 2,
                height: 2,
            },
        ] {
            let mut atlas = vec![vec![11; 64], vec![12; 16]];
            let original = atlas.clone();
            assert!(
                interpolate_sprite_into_atlas(4, 4, &mut atlas, &region, 1, 2, &frames).is_err()
            );
            assert_eq!(atlas, original);
        }
        let mut atlas = vec![vec![11; 64], vec![12; 15]];
        let original = atlas.clone();
        assert!(
            interpolate_sprite_into_atlas(
                4,
                4,
                &mut atlas,
                &SpriteAtlasRegion {
                    x: 0,
                    y: 0,
                    width: 2,
                    height: 2
                },
                1,
                2,
                &frames
            )
            .is_err()
        );
        assert_eq!(atlas, original);
    }

    // Independent packed-lane expression from Frozen ColorMixer; unlike the
    // implementation above it computes two separated channel pairs at once.
    fn frozen_packed(current: u32, next: u32, weight: u64) -> u32 {
        let a = u64::from(current);
        let b = u64::from(next);
        let hi = (a & 0x00ff00ff) * weight + (b & 0x00ff00ff) * (255 - weight);
        let lo = (a & 0xff00ff00) * weight + (b & 0xff00ff00) * (255 - weight);
        let mixed =
            (((hi + 0x00ff00ff) >> 8) & 0x00ff00ff) | (((lo + 0xff00ff00) >> 8) & 0xff00ff00);
        (mixed as u32 & 0x00ffffff) | (current & 0xff000000)
    }

    #[test]
    fn sprite_interpolation_matches_frozen_packed_arithmetic_and_preserves_alpha() {
        let mut state = 0x89abcdef_u32;
        for duration in [1, 2, 3, 7, 20, 255, 256, 257] {
            for subframe in 0..duration {
                for _ in 0..64 {
                    state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                    let current = state.to_le_bytes();
                    state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                    let next = state.to_le_bytes();
                    let output = interpolate_sprite_mips(
                        1,
                        1,
                        subframe,
                        duration,
                        &[SpriteMipFrames {
                            current_rgba: &current,
                            next_rgba: &next,
                        }],
                    )
                    .unwrap();
                    let weight = ((1.0_f32 - subframe as f32 / duration as f32) * 255.0) as u64;
                    assert_eq!(
                        output[0],
                        frozen_packed(
                            u32::from_le_bytes(current),
                            u32::from_le_bytes(next),
                            weight
                        )
                        .to_le_bytes()
                    );
                }
            }
        }
    }

    #[test]
    fn sprite_interpolation_processes_each_supplied_mip_without_cross_sprite_filtering() {
        let current = [10, 20, 30, 7].repeat(8);
        let next = [110, 120, 130, 255].repeat(8);
        let mip = [255, 0, 10, 99].repeat(2);
        let mip_next = [0, 255, 100, 0].repeat(2);
        let output = interpolate_sprite_mips(
            4,
            2,
            1,
            2,
            &[
                SpriteMipFrames {
                    current_rgba: &current,
                    next_rgba: &next,
                },
                SpriteMipFrames {
                    current_rgba: &mip,
                    next_rgba: &mip_next,
                },
            ],
        )
        .unwrap();
        assert_eq!(output[0], [60, 70, 80, 7].repeat(8));
        assert_eq!(output[1], [127, 128, 55, 99].repeat(2));
        assert_eq!(current, [10, 20, 30, 7].repeat(8));
    }

    #[test]
    fn sprite_interpolation_rejects_malformed_time_dimensions_and_later_mips() {
        let pixel = [0; 4];
        let frames = [SpriteMipFrames {
            current_rgba: &pixel,
            next_rgba: &pixel,
        }];
        for (width, height, subframe, duration) in [
            (0, 1, 0, 1),
            (u32::MAX, u32::MAX, 0, 1),
            (1, 1, 0, 0),
            (1, 1, 1, 1),
            (1, 1, 0, u32::MAX),
            (2, 1, 0, 1),
        ] {
            assert!(interpolate_sprite_mips(width, height, subframe, duration, &frames).is_err());
        }
        assert!(interpolate_sprite_mips(1, 1, 0, 1, &[]).is_err());
        let base = [0; 16];
        assert!(
            interpolate_sprite_mips(
                2,
                2,
                0,
                1,
                &[
                    SpriteMipFrames {
                        current_rgba: &base,
                        next_rgba: &base
                    },
                    SpriteMipFrames {
                        current_rgba: &pixel,
                        next_rgba: &[]
                    },
                ]
            )
            .is_err()
        );
    }
}
/// One semantic texture-manager tick, independent of presentation cadence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AtlasAnimationTickEvent {
    pub texture_id: u32,
    pub generation: u64,
    pub tick: u64,
    pub visible: std::collections::BTreeSet<u32>,
    pub animate_only_visible: bool,
}
