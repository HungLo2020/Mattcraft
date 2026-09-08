//! Immutable semantic image references, not copied animation frames or GPU handles.
//! The eventual GUI consumer must resolve the exact accepted atlas incarnation
//! and declare sampled-resource use in its command stream. This table owns no
//! pixels, GPU resources, animation clocks, submissions, or presentation state.
use std::collections::{BTreeMap, BTreeSet};
use super::error::{GalError, GalResult, StatusCode};

pub(crate) const MAX_GUI_ATLAS_REFERENCES: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AcceptedAtlasIncarnation {
    pub texture_id: u32,
    pub generation: u64,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GuiAtlasReference {
    pub asset_id: u64,
    pub atlas: AcceptedAtlasIncarnation,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl GuiAtlasReference {
    pub(crate) fn validate(self) -> GalResult<()> {
        if self.asset_id == 0 || self.atlas.texture_id == 0 || self.atlas.generation == 0
            || self.width == 0 || self.height == 0 || self.atlas.width == 0 || self.atlas.height == 0
            || u64::from(self.atlas.width) * u64::from(self.atlas.height) > 16 * 1024 * 1024
            || self.x.checked_add(self.width).is_none_or(|end| end > self.atlas.width)
            || self.y.checked_add(self.height).is_none_or(|end| end > self.atlas.height)
        {
            return Err(GalError::ffi(StatusCode::InvalidArgument, "invalid GUI atlas reference extent or identity"));
        }
        Ok(())
    }

    /// Preserve orientation and out-of-range UV semantics; never clamp or select a frame.
    pub(crate) fn atlas_uv(self, local: [f32; 2]) -> GalResult<[f32; 2]> {
        self.validate()?;
        if local.iter().any(|value| !value.is_finite()) {
            return Err(GalError::ffi(StatusCode::InvalidArgument, "non-finite GUI atlas UV"));
        }
        let result = [
            (self.x as f32 + local[0] * self.width as f32) / self.atlas.width as f32,
            (self.y as f32 + local[1] * self.height as f32) / self.atlas.height as f32,
        ];
        if result.iter().any(|value| !value.is_finite()) {
            return Err(GalError::ffi(StatusCode::InvalidArgument, "GUI atlas UV overflows"));
        }
        Ok(result)
    }
}

#[derive(Clone, Default)]
pub(crate) struct GuiAtlasReferences {
    revision: u64,
    entries: BTreeMap<u64, GuiAtlasReference>,
}

impl GuiAtlasReferences {
    pub(crate) fn contains(&self, asset: u64) -> bool { self.entries.contains_key(&asset) }
    pub(crate) fn clear(&mut self) { self.entries.clear(); self.revision = 0; }

    /// Complete bounded transaction. Resolve only accepted metadata in the same
    /// Rust context; a failed member must not partially replace earlier bindings.
    pub(crate) fn replace(&mut self, revision: u64, references: &[GuiAtlasReference],
        copied_image_ids: &BTreeSet<u64>, mut accepted: impl FnMut(u32) -> Option<AcceptedAtlasIncarnation>) -> GalResult<()> {
        if revision == 0 || revision < self.revision || references.len() > MAX_GUI_ATLAS_REFERENCES {
            return Err(GalError::ffi(StatusCode::InvalidArgument, "invalid GUI atlas reference revision or count"));
        }
        let mut next = BTreeMap::new();
        for reference in references {
            reference.validate()?;
            if copied_image_ids.contains(&reference.asset_id) || next.insert(reference.asset_id, *reference).is_some() {
                return Err(GalError::ffi(StatusCode::InvalidArgument, "GUI atlas reference image identity collision"));
            }
            if accepted(reference.atlas.texture_id) != Some(reference.atlas) {
                return Err(GalError::ffi(StatusCode::StaleHandle, "GUI atlas incarnation is not accepted"));
            }
        }
        if revision == self.revision && next != self.entries {
            return Err(GalError::ffi(StatusCode::InvalidArgument, "GUI atlas reference revision reused with different data"));
        }
        self.entries = next;
        self.revision = revision;
        Ok(())
    }

    /// Revalidate at consumption: a reload may retire an atlas after declaration.
    pub(crate) fn resolve(&self, asset: u64,
        accepted: impl FnOnce(u32) -> Option<AcceptedAtlasIncarnation>) -> GalResult<GuiAtlasReference> {
        let reference = self.entries.get(&asset).copied().ok_or_else(||
            GalError::ffi(StatusCode::InvalidArgument, "unknown GUI atlas reference"))?;
        if accepted(reference.atlas.texture_id) != Some(reference.atlas) {
            return Err(GalError::ffi(StatusCode::StaleHandle, "GUI atlas incarnation retired before consumption"));
        }
        Ok(reference)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn reference() -> GuiAtlasReference {
        GuiAtlasReference { asset_id: 101, atlas: AcceptedAtlasIncarnation {
            texture_id: 202, generation: 3, width: 64, height: 32 }, x: 16, y: 8, width: 16, height: 8 }
    }
    #[test]
    fn exact_incarnation_and_retirement_are_checked_without_gpu_handles() {
        let r = reference(); let mut table = GuiAtlasReferences::default();
        table.replace(1, &[r], &BTreeSet::new(), |_| Some(r.atlas)).unwrap();
        assert_eq!(table.resolve(101, |_| Some(r.atlas)).unwrap(), r);
        assert!(table.resolve(101, |_| None).is_err());
        assert!(table.resolve(101, |_| Some(AcceptedAtlasIncarnation { generation: 4, ..r.atlas })).is_err());
        assert!(table.resolve(101, |_| Some(AcceptedAtlasIncarnation { width: 128, ..r.atlas })).is_err());
        table.clear(); assert!(!table.contains(101));
    }
    #[test]
    fn failed_transactions_preserve_old_binding_and_revision_for_retry() {
        let r = reference(); let mut table = GuiAtlasReferences::default();
        table.replace(1, &[r], &BTreeSet::new(), |_| Some(r.atlas)).unwrap();
        let next = GuiAtlasReference { asset_id: 102, ..r };
        assert!(table.replace(2, &[next, next], &BTreeSet::new(), |_| Some(r.atlas)).is_err());
        assert!(table.replace(2, &[next], &BTreeSet::from([102]), |_| Some(r.atlas)).is_err());
        assert!(table.replace(2, &[next], &BTreeSet::new(), |_| None).is_err());
        assert!(table.contains(101)); assert!(!table.contains(102));
        table.replace(2, &[next], &BTreeSet::new(), |_| Some(r.atlas)).unwrap();
        table.replace(2, &[next], &BTreeSet::new(), |_| Some(r.atlas)).unwrap();
        assert!(!table.contains(101));
        assert!(table.replace(1, &[r], &BTreeSet::new(), |_| Some(r.atlas)).is_err());
        assert!(table.replace(2, &[r], &BTreeSet::new(), |_| Some(r.atlas)).is_err());
    }
    #[test]
    fn bounded_count_extents_and_uv_orientation() {
        let r = reference(); let mut table = GuiAtlasReferences::default();
        let many = vec![r; MAX_GUI_ATLAS_REFERENCES + 1];
        let mut calls = 0;
        assert!(table.replace(1, &many, &BTreeSet::new(), |_| { calls += 1; Some(r.atlas) }).is_err());
        assert_eq!(calls, 0);
        for bad in [GuiAtlasReference { x: u32::MAX, ..r }, GuiAtlasReference { width: 0, ..r },
            GuiAtlasReference { height: 100, ..r }, GuiAtlasReference { asset_id: 0, ..r }] {
            assert!(table.replace(1, &[bad], &BTreeSet::new(), |_| Some(r.atlas)).is_err());
        }
        assert_eq!(r.atlas_uv([1., 0.]).unwrap(), [0.5, 0.25]);
        assert_eq!(r.atlas_uv([0., 1.]).unwrap(), [0.25, 0.5]);
        assert_eq!(r.atlas_uv([-1., 2.]).unwrap(), [0., 0.75]);
        assert!(r.atlas_uv([f32::NAN, 0.]).is_err());
        assert!(r.atlas_uv([f32::MAX, 0.]).is_err());
    }
}
