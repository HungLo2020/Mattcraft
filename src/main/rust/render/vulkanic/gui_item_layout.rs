//! Rust-owned item raster extents and axis conventions. No renderer/GPU inputs.
//! Expanded block layout is a private prerequisite, not a newly admitted route.
use super::error::{GalError, GalResult};

const MAX_AXIS: u32 = 4096;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GuiItemRasterLayout {
    pub extent: [u32; 2],
    pub guard_pixels: u32,
    scale: f32,
    center: [f32; 2],
}

impl GuiItemRasterLayout {
    pub fn flat(gui_scale: u32) -> GalResult<Self> {
        Self::new([16,16], gui_scale, 0, [0.0,0.0])
    }

    /// Ordinary inventory items keep their atlas cell even when a resource
    /// pack extends geometry outside it. Raster clipping, not fit-to-bounds.
    pub fn inventory_block(gui_scale: u32) -> GalResult<Self> {
        Self::new([16,16], gui_scale, 1, [0.0,0.0])
    }

    /// Explicit oversized_in_gui placement, matching Frozen's GUI bounds and
    /// OversizedItemRenderer/PictureInPictureRenderer argument conventions.
    /// Bounds are derived here from immutable model semantics, not Java pixels.
    pub fn oversized_gui(min: [f64;3], max: [f64;3], gui_scale: u32, origin: [i32;2])
        -> GalResult<Option<(Self,[i32;4])>> {
        if min.iter().chain(max.iter()).any(|v| !v.is_finite())
            || (0..3).any(|axis| min[axis] > max[axis]) {
            return Err(GalError::invalid_argument("invalid oversized GUI model bounds"));
        }
        let spans=[(max[0]-min[0])*16.0,(max[1]-min[1])*16.0];
        if spans.iter().any(|v| !v.is_finite() || *v > MAX_AXIS as f64) {
            return Err(GalError::unsupported_feature("oversized GUI model exceeds raster limit"));
        }
        let logical=spans.map(|v|v.ceil() as u32);
        if logical.iter().all(|v|*v <= 16) { return Ok(None); }
        if logical.contains(&0) { return Err(GalError::invalid_argument("empty oversized GUI axis")); }
        // GuiItemRenderState narrows these products to float BEFORE floor,
        // but rounds the spans above in double precision.
        let anchors=[((min[0]*16.0) as f32).floor() as f64,((max[1]*16.0) as f32).floor() as f64];
        if anchors.iter().any(|v| !v.is_finite() || *v < i32::MIN as f64 || *v > i32::MAX as f64) {
            return Err(GalError::invalid_argument("oversized GUI anchor overflow"));
        }
        let left=origin[0] as i64+anchors[0] as i64+8;
        let top=origin[1] as i64-anchors[1] as i64+8;
        let wide=[left,top,left+logical[0] as i64,top+logical[1] as i64];
        if wide.iter().any(|v| *v < i32::MIN as i64 || *v > i32::MAX as i64) {
            return Err(GalError::invalid_argument("oversized GUI placement overflow"));
        }
        let mut layout=Self::new(logical,gui_scale,1,[0.,0.])?;
        // Frozen passes (textureHeight, guiScale) to getTranslateY; the
        // explicitly oversized branch returns guiScale/2. Preserve that basis.
        layout.center=[1.-anchors[0] as f32*gui_scale as f32,
            1.+(0.5+anchors[1] as f32-logical[1] as f32*0.5)*gui_scale as f32];
        Ok(Some((layout,wide.map(|v|v as i32))))
    }

    /// Copies model-space AABB semantics, not Java's offscreen extent or matrix.
    /// Frozen OpenGL OversizedItemRenderer rounds DOUBLE spans before scaling.
    /// One private guard pixel preserves this backend's existing mesh padding;
    /// it is not attributed to Frozen and must be excluded during composition.
    pub fn block_bounds(min: [f64;3], max: [f64;3], gui_scale: u32) -> GalResult<Self> {
        if min.iter().chain(max.iter()).any(|v| !v.is_finite())
            || (0..3).any(|axis| min[axis] > max[axis]) {
            return Err(GalError::invalid_argument("invalid semantic item bounds"));
        }
        let spans = [(max[0]-min[0])*16.0, (max[1]-min[1])*16.0];
        if spans.iter().any(|v| !v.is_finite() || *v > MAX_AXIS as f64) {
            return Err(GalError::unsupported_feature("item bounds exceed raster limit"));
        }
        let logical = spans.map(|v| v.ceil().max(16.0) as u32);
        let center = if logical.iter().any(|v| *v > 16) {
            [(-(min[0]+max[0])/2.0) as f32, (-(min[1]+max[1])/2.0) as f32]
        } else { [0.0,0.0] };
        Self::new(logical, gui_scale, 1, center)
    }

    fn new(logical: [u32;2], gui_scale: u32, guard_pixels: u32, offset: [f32;2]) -> GalResult<Self> {
        let axis = |v: u32| v.checked_mul(gui_scale).and_then(|v| v.checked_add(2*guard_pixels))
            .filter(|v| gui_scale > 0 && *v <= MAX_AXIS)
            .ok_or_else(|| GalError::unsupported_feature("item raster exceeds bounded extent"));
        let extent = [axis(logical[0])?, axis(logical[1])?];
        let scale = gui_scale as f32 * 16.0;
        let center = [extent[0] as f32*0.5 + scale*offset[0],
                      extent[1] as f32*0.5 - scale*offset[1]];
        if center.iter().any(|v| !v.is_finite()) {
            return Err(GalError::invalid_argument("item raster center is not finite"));
        }
        Ok(Self {extent, guard_pixels, scale, center})
    }

    pub fn compose(self, model: [f32;16]) -> GalResult<[f32;16]> {
        if model.iter().any(|v| !v.is_finite()) || [3,7,11].iter().any(|i| model[*i]!=0.0)
            || model[15] != 1.0 {
            return Err(GalError::invalid_argument("item model transform must be finite affine"));
        }
        let mut matrix = model;
        for column in 0..4 {
            let i = column*4;
            matrix[i] = self.scale*model[i] + self.center[0]*model[i+3];
            matrix[i+1] = -self.scale*model[i+1] + self.center[1]*model[i+3];
            matrix[i+2] = self.scale*model[i+2];
        }
        if matrix.iter().any(|v| !v.is_finite()) {
            return Err(GalError::invalid_argument("item raster transform overflow"));
        }
        Ok(matrix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const ID: [f32;16] = [1.,0.,0.,0.,0.,1.,0.,0.,0.,0.,1.,0.,0.,0.,0.,1.];

    #[test]
    fn explicit_oversized_layout_preserves_rounded_placement_and_frozen_pip_basis() {
        for scale in 1..=3 {
            let (layout,bounds)=GuiItemRasterLayout::oversized_gui(
                [-0.8,-0.4,-0.5],[1.1,0.8,0.5],scale,[10,20]).unwrap().unwrap();
            assert_eq!(bounds,[5,16,36,36]);
            assert_eq!(layout.extent,[31*scale+2,20*scale+2]);
            let m=layout.compose(ID).unwrap();
            assert_eq!([m[12],m[13]],[1.+13.*scale as f32,1.+2.5*scale as f32]);
        }
        assert!(GuiItemRasterLayout::oversized_gui([-0.5;3],[0.5;3],2,[0,0]).unwrap().is_none());
        assert!(GuiItemRasterLayout::oversized_gui([0.;3],[2.,0.,1.],2,[0,0]).is_err());
        assert!(GuiItemRasterLayout::oversized_gui([-0.8;3],[1.1;3],2,[i32::MAX,0]).is_err());
    }

    #[test]
    fn regular_layouts_preserve_gui_y_reflection_and_private_padding() {
        for scale in 1..=4 {
            let flat=GuiItemRasterLayout::flat(scale).unwrap();
            let block=GuiItemRasterLayout::block_bounds([-0.5;3],[0.5;3],scale).unwrap();
            assert_eq!(flat.extent,[16*scale;2]);
            assert_eq!(block.extent,[16*scale+2;2]);
            assert_eq!((flat.guard_pixels,block.guard_pixels),(0,1));
            let a=flat.compose(ID).unwrap();let b=block.compose(ID).unwrap();
            assert_eq!([a[0],a[5],a[10]],[16.*scale as f32,-16.*scale as f32,16.*scale as f32]);
            assert_eq!([a[12],a[13]],[8.*scale as f32;2]);
            assert_eq!([b[12],b[13]],[8.*scale as f32+1.;2]);
        }
    }

    #[test]
    fn expanded_bounds_center_in_content_region_without_rounding_spans_to_float() {
        let layout=GuiItemRasterLayout::block_bounds([1.,2.,-0.5],[3.,5.,0.5],2).unwrap();
        assert_eq!(layout.extent,[66,98]);
        let m=layout.compose(ID).unwrap();assert_eq!([m[12],m[13]],[-31.,161.]);
        assert_eq!([m[0]+m[12],3.*m[0]+m[12]],[1.,65.]);
        assert_eq!([2.*m[5]+m[13],5.*m[5]+m[13]],[97.,1.]);
        // Frozen ceil(double span*16) differs from premature Java float narrowing.
        let edge=GuiItemRasterLayout::block_bounds([0.;3],[1.+1e-9,1.,1.],1).unwrap();
        assert_eq!(edge.extent,[19,18]);
    }

    #[test]
    fn malformed_bounds_scales_and_matrix_overflow_fail_closed() {
        for scale in [0,257,u32::MAX] { assert!(GuiItemRasterLayout::flat(scale).is_err()); }
        assert!(GuiItemRasterLayout::flat(256).is_ok());
        assert!(GuiItemRasterLayout::block_bounds([-0.5;3],[0.5;3],256).is_err());
        for (min,max) in [([f64::NAN;3],[1.;3]),([2.;3],[1.;3]),([-f64::MAX;3],[f64::MAX;3])] {
            assert!(GuiItemRasterLayout::block_bounds(min,max,1).is_err());
        }
        let layout=GuiItemRasterLayout::flat(2).unwrap();
        for (i,value) in [(3,1.),(15,0.),(0,f32::INFINITY),(12,f32::MAX)] {
            let mut m=ID;m[i]=value;assert!(layout.compose(m).is_err());
        }
    }
}
