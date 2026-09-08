//! Private explicit layout for GUI item rasterization before GUI composition.
//! The caller supplies semantic item identities/order; no Java atlas positions
//! or GPU objects are accepted. The intermediate row order is explicit;
//! backend framebuffer conventions are not reconstructed here.
use super::error::{GalError, GalResult};
use super::gal::VulkanicGal;
use super::handles::Handle;
use super::resources::{Extent3d, TextureDesc, TextureDimension, TextureFormat,
    TextureUsage, TextureViewDesc, RenderTargetDesc, RenderPassDesc};
use std::collections::{BTreeMap, BTreeSet};

const MAX_ITEMS: u32 = 4096;
pub(crate) const MAX_ITEM_LAYERS: usize = 64;

/// Immutable resolved model transform, column-major, in model coordinates.
/// This flat family retains a common Z plane and front-facing normal. General
/// 3D, perspective, and reflected-face transforms require the mesh family.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuiItemModelTransform(pub [f32;16]);

impl Default for GuiItemModelTransform {
    fn default() -> Self {
        Self([1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, -0.5,-0.5,-0.5,1.0])
    }
}

impl GuiItemModelTransform {
    pub fn validate(self) -> GalResult<()> {
        let m=self.0;
        let determinant=m[0]*m[5]-m[1]*m[4];
        // Resolved quaternion rotations carry a few float rounding bits in
        // their otherwise unchanged Z scale. Require exact centered-Z
        // structure, with a machine-precision bound on that scale residue.
        // Do not snap the matrix or relax any XY geometry/pixel comparison.
        let centered_z=(m[10]-1.0).abs()<=4.0*f32::EPSILON && m[14]==-0.5*m[10];
        if m.iter().any(|v| !v.is_finite() || v.abs()>16.0)
            || [2,3,6,7,8,9,11].into_iter().any(|i| m[i]!=0.0)
            || !centered_z || m[15]!=1.0
            || !determinant.is_finite() || determinant<=0.000001 {
            return Err(GalError::invalid_argument("unsupported flat item model transform"));
        }
        Ok(())
    }

    pub fn lower(self, geometry: GuiItemRasterGeometry) -> GalResult<GuiItemRasterGeometry> {
        self.validate()?;
        geometry.validate()?;
        if self==Self::default() { return Ok(geometry); }
        let m=self.0;
        let mut corners=geometry.corners;
        for point in corners.chunks_exact_mut(2) {
            let x=point[0]/16.0;
            let y=1.0-point[1]/16.0;
            point[0]=(m[0]*x+m[4]*y+m[12]+0.5)*16.0;
            point[1]=(0.5-(m[1]*x+m[5]*y+m[13]))*16.0;
        }
        let transformed=GuiItemRasterGeometry {corners};
        transformed.validate()?;
        Ok(transformed)
    }
}

pub(crate) fn item_uv_identity(uv: [f32;4]) -> GalResult<[u32;4]> {
    if uv.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        || uv[0] >= uv[2] || uv[1] >= uv[3] {
        return Err(GalError::invalid_argument("invalid item-local UV rectangle"));
    }
    Ok(uv.map(|v| if v == 0.0 {0} else {v.to_bits()}))
}

/// Authored item-local geometry, independent of the final screen transform.
/// The fourth affine corner is implied. This bounded slice is contained in
/// the canonical 16-unit item cell; other shapes remain unadmitted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuiItemRasterGeometry {
    pub corners: [f32; 6],
}

impl Default for GuiItemRasterGeometry {
    fn default() -> Self { Self { corners: [0.0,0.0,16.0,0.0,0.0,16.0] } }
}

impl GuiItemRasterGeometry {
    pub fn validate(self) -> GalResult<()> {
        let [x0,y0,x1,y1,x3,y3] = self.corners;
        let fourth = [x1+x3-x0,y1+y3-y0];
        let area = (x1-x0)*(y3-y0)-(y1-y0)*(x3-x0);
        if self.corners.into_iter().chain(fourth).any(|v| !v.is_finite() || !(0.0..=16.0).contains(&v))
            || !area.is_finite() || area.abs() <= 0.000001 {
            return Err(GalError::invalid_argument("invalid bounded item-local raster geometry"));
        }
        Ok(())
    }

    pub fn identity(self) -> GalResult<[u32;6]> {
        self.validate()?;
        Ok(self.corners.map(|v| if v == 0.0 { 0 } else { v.to_bits() }))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GuiItemRasterRows { TopDown, BottomUp }

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct GuiItemRasterIdentity {
    pub asset_id: u64,
    pub atlas_generation: u64,
    pub texture_id: u32,
    pub region: [u32; 4],
    pub color_argb: u32,
    pub lighting_rgb: [u32; 3],
    pub cutout: bool,
    pub geometry: [u32; 6],
    pub uv: [u32; 4],
}

/// Semantic slot history survives texture/GUI asset reloads. A replacement
/// model incarnation gets a new slot; GUI-scale changes and atlas exhaustion
/// rebuild the layout. No obsolete GPU image or Java model object is retained.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct GuiItemRasterSlots {
    scale: u32,
    layout: Option<GuiItemRasterLayout>,
    entries: BTreeMap<Vec<GuiItemRasterIdentity>, u32>,
}

impl GuiItemRasterSlots {
    pub fn prepare(&mut self, scale: u32, identities: &[GuiItemRasterIdentity], max_side: u32)
        -> GalResult<Vec<GuiItemRasterPlacement>> {
        self.prepare_groups(scale, &identities.iter().map(|id| vec![*id]).collect::<Vec<_>>(), max_side)
    }

    /// Layer order and every layer's resource incarnation are part of item
    /// identity. Never assign independently composited slots to child layers.
    pub fn prepare_groups(&mut self, scale: u32, identities: &[Vec<GuiItemRasterIdentity>], max_side: u32)
        -> GalResult<Vec<GuiItemRasterPlacement>> {
        if identities.is_empty() || identities.len() > MAX_ITEMS as usize
            || identities.iter().any(|group| group.is_empty() || group.len() > MAX_ITEM_LAYERS
                || group.iter().any(|id| id.asset_id == 0 || id.atlas_generation == 0 || id.texture_id == 0
                    || id.region[2] == 0 || id.region[3] == 0)) {
            return Err(GalError::invalid_argument("invalid bounded item raster identities"));
        }
        let unique: BTreeSet<_> = identities.iter().cloned().collect();
        let added = unique.iter().filter(|id| !self.entries.contains_key(*id)).count();
        let total = self.entries.len()+added;
        let rebuild = self.scale != scale || self.layout.map_or(true, |layout| {
            let columns = layout.side/layout.cell;
            total as u64 >= u64::from(columns)*u64::from(columns) || layout.side > max_side
        });
        let mut next = self.clone();
        if rebuild {
            next.layout = Some(GuiItemRasterLayout::new(scale,unique.len() as u32,max_side)?);
            next.entries.clear();
            next.scale = scale;
        } else if total > MAX_ITEMS as usize {
            return Err(GalError::invalid_argument("item raster identity residency bound exceeded"));
        }
        for id in identities {
            let index = next.entries.len() as u32;
            next.entries.entry(id.clone()).or_insert(index);
        }
        let layout = next.layout.as_mut().expect("validated raster layout");
        layout.items = next.entries.len() as u32;
        let placements = identities.iter().map(|id|
            layout.placement_with_rows(next.entries[id],GuiItemRasterRows::BottomUp))
            .collect::<GalResult<Vec<_>>>()?;
        *self = next;
        Ok(placements)
    }
}

/// One bounded Rust-owned intermediate. Its views never leave GAL, and it
/// does not acquire a surface or submit/present independently of the frame.
pub(crate) struct GuiItemRasterTarget {
    pub color: Handle,
    pub view: Handle,
    pub target: Handle,
    pub pass: Handle,
    pub extent: Extent3d,
}

impl GuiItemRasterTarget {
    pub fn create(gal: &mut VulkanicGal, extent: Extent3d) -> GalResult<Self> {
        if extent.width == 0 || extent.height == 0 || extent.depth != 1
            || extent.width > 4096 || extent.height > 4096 {
            return Err(GalError::invalid_argument("invalid bounded item raster target extent"));
        }
        let mut created = Vec::new();
        let result = (|| {
            let color = gal.create_texture(TextureDesc { label: "gui.item-raster.color".into(),
                dimension: TextureDimension::D2, format: TextureFormat::Rgba8Unorm, extent,
                mip_levels: 1, array_layers: 1,
                usages: vec![TextureUsage::ColorAttachment, TextureUsage::Sampled, TextureUsage::TransferSrc] })?;
            created.push(color);
            let view = gal.create_texture_view(TextureViewDesc { label: "gui.item-raster.view".into(),
                texture: color, format: TextureFormat::Rgba8Unorm, base_mip: 0, mip_count: 1,
                base_layer: 0, layer_count: 1 })?;
            created.push(view);
            let target = gal.create_render_target(RenderTargetDesc { label: "gui.item-raster.target".into(),
                color_views: vec![view], depth_stencil_view: None, extent })?;
            created.push(target);
            let pass = gal.create_render_pass(RenderPassDesc { label: "gui.item-raster.pass".into(),
                target, color_formats: vec![TextureFormat::Rgba8Unorm], depth_format: None })?;
            created.push(pass);
            Ok(Self {color,view,target,pass,extent})
        })();
        if result.is_err() {
            for handle in created.into_iter().rev() { let _ = gal.destroy(handle); }
        }
        result
    }

    /// Call only after dependent compositor bindings have been retired.
    /// GAL owns completion-aware destruction of the underlying objects.
    pub fn destroy(self, gal: &mut VulkanicGal) -> GalResult<()> {
        let mut error = None;
        for handle in [self.pass,self.target,self.view,self.color] {
            if let Err(cause) = gal.destroy(handle) { if error.is_none() {error = Some(cause);} }
        }
        match error {Some(error) => Err(error), None => Ok(())}
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GuiItemRasterLayout {
    side: u32,
    cell: u32,
    items: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GuiItemRasterPlacement {
    pub target_extent: [u32; 2],
    pub rect: [u32; 4],
    pub rows: GuiItemRasterRows,
}

impl GuiItemRasterLayout {
    /// Initial allocation policy of vanilla's item raster atlas, expressed
    /// in Rust. Cached identity lifetime/growth is deliberately not admitted
    /// by this stateless layout; callers must invalidate that cache explicitly.
    pub fn new(gui_scale: u32, items: u32, max_texture_side: u32) -> GalResult<Self> {
        if gui_scale == 0 || items == 0 || items > MAX_ITEMS || max_texture_side < 512 {
            return Err(GalError::invalid_argument("invalid GUI item raster layout"));
        }
        let cell = gui_scale.checked_mul(16)
            .ok_or_else(|| GalError::invalid_argument("GUI item raster cell overflow"))?;
        let padded = items + items / 2;
        let mut square_side = 1u32;
        while square_side * square_side < padded { square_side += 1; }
        let desired = square_side.checked_mul(cell).and_then(u32::checked_next_power_of_two)
            .ok_or_else(|| GalError::invalid_argument("GUI item raster extent overflow"))?;
        let side = desired.max(512).min(max_texture_side);
        let columns = side / cell;
        if columns == 0 || u64::from(columns) * u64::from(columns) < u64::from(items) {
            return Err(GalError::invalid_argument("GUI item raster target cannot fit the complete frame"));
        }
        Ok(Self { side, cell, items })
    }

    pub fn placement(self, index: u32) -> GalResult<GuiItemRasterPlacement> {
        self.placement_with_rows(index, GuiItemRasterRows::TopDown)
    }

    /// Item-local row orientation is part of the explicit raster transform,
    /// not a backend query. Vanilla item cells use BottomUp: nearest sampling
    /// at exact texel boundaries is sensitive to interpolation direction.
    pub fn placement_with_rows(self, index: u32, rows: GuiItemRasterRows) -> GalResult<GuiItemRasterPlacement> {
        if index >= self.items {
            return Err(GalError::invalid_argument("GUI item raster index out of bounds"));
        }
        let columns = self.side / self.cell;
        let y = (index / columns) * self.cell;
        let y = match rows { GuiItemRasterRows::TopDown => y,
            GuiItemRasterRows::BottomUp => self.side-y-self.cell };
        Ok(GuiItemRasterPlacement { target_extent: [self.side; 2], rows,
            rect: [(index % columns) * self.cell, y, self.cell, self.cell] })
    }
}

impl GuiItemRasterPlacement {
    pub(crate) fn validate(self) -> GalResult<()> {
        let [x, y, width, height] = self.rect;
        if width == 0 || height == 0 || self.target_extent.iter().any(|v| *v > i32::MAX as u32)
            || x.checked_add(width).map_or(true, |end| end > self.target_extent[0])
            || y.checked_add(height).map_or(true, |end| end > self.target_extent[1]) {
            return Err(GalError::invalid_argument("invalid GUI item raster placement"));
        }
        Ok(())
    }
    /// Lower item-local geometry, not final screen coordinates, into the
    /// explicitly allocated raster target. Screen clipping and composition
    /// remain a separate command; rasterization clips only to this item cell.
    pub fn lower_quad(self, local: &super::gui_frontend::GuiAffineQuadRequest)
        -> GalResult<super::gui_frontend::GuiAffineQuadRequest> {
        let [x, y, width, height] = self.rect;
        self.validate()?;
        let p0 = self.raster_point([local.x0, local.y0])?;
        let p1 = self.raster_point([local.x1, local.y1])?;
        let p3 = self.raster_point([local.x3, local.y3])?;
        let mut raster = local.clone();
        [raster.x0, raster.y0] = p0;
        [raster.x1, raster.y1] = p1;
        [raster.x3, raster.y3] = p3;
        [raster.gui_width, raster.gui_height] = self.target_extent;
        raster.projection_extent = self.target_extent.map(|v| v as f32);
        raster.clip_mode = 1;
        raster.clip_left = x as i32;
        raster.clip_top = y as i32;
        raster.clip_width = width as i32;
        raster.clip_height = height as i32;
        Ok(raster)
    }

    /// Transform a semantic local GUI point (16 logical units per standard
    /// item) into this offscreen pass. No rounding, clamping or texel bias.
    pub fn raster_point(self, point: [f32; 2]) -> GalResult<[f32; 2]> {
        if point.iter().any(|value| !value.is_finite()) {
            return Err(GalError::invalid_argument("non-finite GUI item raster point"));
        }
        self.validate()?;
        let y = match self.rows {
            GuiItemRasterRows::TopDown => self.rect[1] as f32 + point[1] * (self.rect[3] as f32 / 16.0),
            GuiItemRasterRows::BottomUp => (self.rect[1]+self.rect[3]) as f32 - point[1] * (self.rect[3] as f32 / 16.0),
        };
        let result = [self.rect[0] as f32 + point[0] * (self.rect[2] as f32 / 16.0), y];
        if result.iter().any(|value| !value.is_finite()) {
            return Err(GalError::invalid_argument("GUI item raster point overflow"));
        }
        Ok(result)
    }

    /// Backend-neutral sampled subresource coordinates for final composition.
    /// The inverse of the authored intermediate transform, on either backend.
    pub fn composite_uv(self) -> [f32; 4] {
        let low = self.rect[1] as f32 / self.target_extent[1] as f32;
        let high = (self.rect[1] + self.rect[3]) as f32 / self.target_extent[1] as f32;
        let [v0,v1] = match self.rows { GuiItemRasterRows::TopDown => [low,high],
            GuiItemRasterRows::BottomUp => [high,low] };
        [self.rect[0] as f32 / self.target_extent[0] as f32, v0,
         (self.rect[0] + self.rect[2]) as f32 / self.target_extent[0] as f32,
         v1]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_model_transforms_lower_only_in_rust_and_preserve_legacy_bits() {
        let full=GuiItemRasterGeometry::default();
        assert_eq!(GuiItemModelTransform::default().lower(full).unwrap().identity().unwrap(),full.identity().unwrap());
        let mut scaled=GuiItemModelTransform::default();
        scaled.0[0]=0.5; scaled.0[5]=0.5; scaled.0[12]=-0.25; scaled.0[13]=-0.25;
        assert_eq!(scaled.lower(full).unwrap().corners,[4.0,4.0,12.0,4.0,4.0,12.0]);
        scaled.0[12]+=0.125;
        assert_eq!(scaled.lower(full).unwrap().corners,[6.0,4.0,14.0,4.0,6.0,12.0]);
        let mut rotated=GuiItemModelTransform::default();
        rotated.0[0]=0.0; rotated.0[1]=1.0; rotated.0[4]=-1.0; rotated.0[5]=0.0; rotated.0[12]=0.5;
        assert_eq!(rotated.lower(full).unwrap().corners,[0.0,16.0,0.0,0.0,16.0,16.0]);
        for (index,value) in [(0,f32::NAN),(0,-1.0),(0,0.0),(2,0.1),(3,0.1),(8,0.1),(10,2.0),(14,0.0),(15,0.0)] {
            let mut invalid=GuiItemModelTransform::default(); invalid.0[index]=value;
            assert!(invalid.lower(full).is_err(),"unsupported transform entry{index}");
        }
        let mut outside=GuiItemModelTransform::default(); outside.0[12]=0.0;
        assert!(outside.lower(full).is_err(),"transformed geometry cannot silently clip outside the item cell");
    }

    #[test]
    fn game_authored_quaternion_display_transform_preserves_planar_coverage() {
        // Exact output of ItemTransform.apply for GUI Z90, XY scale0.5,
        // translation X2/16. Quaternion arithmetic does not produce Z=1
        // bit-exactly; keep the actual copied matrix, including that residue.
        let matrix=GuiItemModelTransform([0.0,0.49999997,0.0,0.0,
            -0.49999997,0.0,0.0,0.0, 0.0,0.0,0.99999994,0.0,
            0.375,-0.24999999,-0.49999997,1.0]);
        let lowered=matrix.lower(GuiItemRasterGeometry::default()).unwrap();
        for (actual,expected) in lowered.corners.into_iter().zip([6.0,12.0,6.0,4.0,14.0,12.0]) {
            assert!((actual-expected).abs()<=0.000002,"actual={actual}, expected={expected}");
        }
        let mut tilted=matrix; tilted.0[2]=f32::EPSILON;
        assert!(tilted.validate().is_err(),"actual plane tilt is not numerical Z-scale residue");
        let mut translated=matrix; translated.0[14]=0.0;
        assert!(translated.validate().is_err(),"Z translation still cannot enter this flat family");
        let mut rescaled=matrix; rescaled.0[10]=0.99; rescaled.0[14]=-0.495;
        assert!(rescaled.validate().is_err(),"authored Z scale is not quaternion roundoff");
    }

    fn identities(generation:u64,count:u64) -> Vec<GuiItemRasterIdentity> {
        (1..=count).map(|asset_id| GuiItemRasterIdentity {asset_id,atlas_generation:generation,cutout:false,
            texture_id:1,region:[0,0,32,32],color_argb:u32::MAX,
            lighting_rgb:[1.0_f32.to_bits();3],
            geometry:GuiItemRasterGeometry::default().identity().unwrap(),
            uv:item_uv_identity([0.0,0.0,1.0,1.0]).unwrap()}).collect()
    }

    #[test]
    fn grouped_slots_preserve_layer_order_and_all_resource_incarnations() {
        let pair = identities(1,2);
        let mut replaced = pair.clone();
        replaced[1].atlas_generation = 2;
        let mut relit = pair.clone();
        relit[1].lighting_rgb = [0.5_f32.to_bits();3];
        let groups = vec![pair.clone(),vec![pair[1],pair[0]],replaced,relit,pair];
        let mut slots = GuiItemRasterSlots::default();
        let placements = slots.prepare_groups(2,&groups,4096).unwrap();
        assert_eq!(placements[0],placements[4]);
        for i in 0..4 { for j in i+1..4 { assert_ne!(placements[i],placements[j]); } }
        assert_eq!(slots.prepare_groups(2,&groups,4096).unwrap(),placements);
        let before = slots.clone();
        for invalid in [vec![],vec![vec![]],vec![vec![groups[0][0];MAX_ITEM_LAYERS+1]]] {
            assert!(slots.prepare_groups(2,&invalid,4096).is_err());
            assert_eq!(slots,before,"invalid group must not mutate retained slots");
        }
    }

    #[test]
    fn authored_uv_subrectangles_are_validated_and_part_of_raster_identity() {
        let full = identities(1,1)[0];
        let mut partial = full;
        partial.uv = item_uv_identity([0.25,0.0,0.75,0.5]).unwrap();
        let mut slots = GuiItemRasterSlots::default();
        let placements = slots.prepare(2,&[full,partial,full],4096).unwrap();
        assert_ne!(placements[0],placements[1]);
        assert_eq!(placements[0],placements[2]);
        assert_eq!(full.uv,item_uv_identity([-0.0,0.0,1.0,1.0]).unwrap());
        for uv in [[f32::NAN,0.0,1.0,1.0], [0.0,0.0,f32::INFINITY,1.0],
            [-0.001,0.0,1.0,1.0], [0.0,0.0,1.001,1.0], [0.5,0.0,0.5,1.0],
            [0.0,0.5,1.0,0.25]] {
            assert!(item_uv_identity(uv).is_err());
        }
    }

    #[test]
    fn item_local_geometry_validates_all_four_corners_and_canonicalizes_zero() {
        let full = GuiItemRasterGeometry::default();
        assert!(full.validate().is_ok());
        assert!(GuiItemRasterGeometry { corners:[4.0,2.0,12.0,2.0,4.0,14.0] }.validate().is_ok());
        assert!(GuiItemRasterGeometry { corners:[16.0,0.0,0.0,0.0,16.0,16.0] }.validate().is_ok(),
            "UV reflection reverses parameterization, not the semantic face");
        for corners in [[0.0,0.0,16.0,4.0,4.0,16.0], [0.0,0.0,0.0,0.0,0.0,16.0],
            [-1.0,0.0,16.0,0.0,0.0,16.0],
            [f32::NAN,0.0,16.0,0.0,0.0,16.0], [0.0,0.0,f32::INFINITY,0.0,0.0,16.0]] {
            assert!(GuiItemRasterGeometry {corners}.validate().is_err());
        }
        let mut negative_zero = full;
        negative_zero.corners[0] = -0.0;
        assert_eq!(full.identity().unwrap(), negative_zero.identity().unwrap());
    }

    #[test]
    fn different_geometry_on_the_same_atlas_region_cannot_alias_a_raster_slot() {
        let mut slots = GuiItemRasterSlots::default();
        let full = identities(1,1)[0];
        let mut inset = full;
        inset.geometry = GuiItemRasterGeometry {corners:[4.0,2.0,12.0,2.0,4.0,14.0]}.identity().unwrap();
        let placements = slots.prepare(2,&[full,inset,full],4096).unwrap();
        assert_ne!(placements[0],placements[1]);
        assert_eq!(placements[0],placements[2]);
    }

    #[test]
    fn replacement_models_append_to_retained_slots_as_observed_after_real_reload() {
        let mut slots=GuiItemRasterSlots::default();
        let a=identities(1,9);
        let b=identities(2,9);
        assert_eq!(slots.prepare(3,&a,4096).unwrap()[0].rect,[0,464,48,48]);
        let after=slots.prepare(3,&b,4096).unwrap();
        assert_eq!(after[0].rect,[432,464,48,48],"r230 Frozen replacement feather remains in row zero, slot nine");
        assert_eq!(after[8].rect,[336,416,48,48],"r230 replacement apple wraps to row one, slot seventeen");
        assert_eq!(slots.entries.len(),18);
        assert_eq!(slots.prepare(3,&b,4096).unwrap(),after);
        assert_eq!(slots.entries.len(),18);
        let before=slots.clone();
        assert!(slots.prepare(0,&b,4096).is_err());
        assert_eq!(slots,before,"failed plans cannot mutate slot history");
        assert_eq!(slots.prepare(2,&b,4096).unwrap()[0].rect,[0,480,32,32]);
        assert_eq!(slots.entries.len(),9,"GUI-scale changes explicitly invalidate slot history");
    }

    #[test]
    fn slot_cache_rebuilds_at_capacity_and_deduplicates_current_semantic_models() {
        let mut slots=GuiItemRasterSlots::default();
        let first=identities(1,1);
        slots.prepare(3,&first,4096).unwrap();
        for generation in 2..100 {slots.prepare(3,&identities(generation,1),4096).unwrap();}
        assert_eq!(slots.entries.len(),99);
        let next=identities(100,1);
        let placements=slots.prepare(3,&[next[0],next[0]],4096).unwrap();
        assert_eq!(placements[0],placements[1]);
        assert_eq!(placements[0].rect,[0,464,48,48]);
        assert_eq!(slots.entries.len(),1,"vanilla rebuilds when the union reaches existing capacity");
    }

    #[test]
    fn slot_metadata_has_a_hard_bound_and_failed_growth_is_transactional() {
        let mut slots=GuiItemRasterSlots::default();
        slots.prepare(1,&identities(1,MAX_ITEMS as u64),4096).unwrap();
        let before=slots.clone();
        assert!(slots.prepare(1,&identities(2,1),4096).is_err());
        assert_eq!(slots,before);
    }

    #[test]
    fn observed_nine_item_scale_three_layout_has_separate_raster_coordinates() {
        let layout = GuiItemRasterLayout::new(3, 9, 16384).unwrap();
        for index in 0..9 {
            let slot = layout.placement(index).unwrap();
            assert_eq!(slot.target_extent, [512, 512]);
            assert_eq!(slot.rect, [48 * index, 0, 48, 48]);
            assert_eq!(slot.raster_point([0.0, 0.0]).unwrap(), [48.0 * index as f32, 0.0]);
            assert_eq!(slot.raster_point([16.0, 16.0]).unwrap(), [48.0 * (index + 1) as f32, 48.0]);
            assert_eq!(slot.composite_uv(), [48.0 * index as f32 / 512.0, 0.0,
                48.0 * (index + 1) as f32 / 512.0, 48.0 / 512.0]);
        }
        assert!(layout.placement(9).is_err());
    }

    #[test]
    fn observed_source_atlas_uvs_survive_local_transport_bit_exactly() {
        use crate::render::vulkanic::gui_atlas_reference::{AcceptedAtlasIncarnation, GuiAtlasReference};
        // Exact endpoint bits recorded independently by both callsites in r226.
        for (x, y, expected) in [
            (1536, 1904, [0x3ec00000, 0x3f6e0000, 0x3ec40000, 0x3f720000]),
            (1568, 1808, [0x3ec40000, 0x3f620000, 0x3ec80000, 0x3f660000]),
            (1536, 1776, [0x3ec00000, 0x3f5e0000, 0x3ec40000, 0x3f620000]),
        ] {
            let reference = GuiAtlasReference { asset_id: 1,
                atlas: AcceptedAtlasIncarnation { texture_id: 1, generation: 1, width: 4096, height: 2048 },
                x, y, width: 32, height: 32 };
            let low = reference.atlas_uv([0.0, 0.0]).unwrap();
            let high = reference.atlas_uv([1.0, 1.0]).unwrap();
            assert_eq!([low[0].to_bits(), low[1].to_bits(), high[0].to_bits(), high[1].to_bits()], expected);
        }
    }

    #[test]
    fn layout_wraps_and_rejects_overflow_without_dropping_items() {
        let layout = GuiItemRasterLayout::new(3, 11, 16384).unwrap();
        assert_eq!(layout.placement(10).unwrap().rect, [0, 48, 48, 48]);
        for (scale, count, limit) in [(0, 1, 512), (1, 0, 512), (1, 4097, 16384),
                                     (1, 1, 256), (u32::MAX, 1, 16384), (64, 2, 512)] {
            assert!(GuiItemRasterLayout::new(scale, count, limit).is_err());
        }
        let slot = layout.placement(0).unwrap();
        assert!(slot.raster_point([f32::NAN, 0.0]).is_err());
        assert!(slot.raster_point([f32::MAX, 0.0]).is_err());
        assert_eq!(slot.raster_point([-0.5, 16.5]).unwrap(), [-1.5, 49.5],
            "geometry is not silently clipped; the explicit raster pass owns its scissor");
    }
}
