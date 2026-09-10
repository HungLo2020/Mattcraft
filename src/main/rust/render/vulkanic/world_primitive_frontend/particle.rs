//! Ordinary particle semantics lowered without Java geometry or GPU state.
//! Camera-relative orientation, size and UV bounds arrive as immutable game data.
use super::*;

#[derive(Clone, Copy, Debug)]
pub(crate) enum ParticleSurface {
    OrdinaryOpaque, OrdinaryTranslucent, TerrainOpaque, TerrainCutout, TerrainTranslucent,
}

impl ParticleSurface {
    pub(crate) fn from_wire(value: u32) -> GalResult<Self> {
        match value {
            0 => Ok(Self::OrdinaryOpaque), 1 => Ok(Self::OrdinaryTranslucent),
            2 => Ok(Self::TerrainOpaque), 3 => Ok(Self::TerrainCutout),
            4 => Ok(Self::TerrainTranslucent),
            _ => Err(GalError::invalid_argument("unknown particle surface")),
        }
    }
    pub(crate) fn translucent(self) -> bool {
        matches!(self, Self::OrdinaryTranslucent | Self::TerrainTranslucent)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ParticleQuad {
    pub center: [f32; 3],
    /// Gameplay orientation, in x/y/z/w order; not necessarily unit length.
    pub rotation: [f32; 4],
    /// Signed lifetime-interpolated size. Zero is a valid degenerate quad.
    pub size: f32,
    /// Original sprite bounds: u0/u1/v0/v1, including reversed U.
    pub uv_bounds: [f32; 4],
    pub color_argb: u32,
    pub packed_light: u32,
    /// Semantic identity of an independently published Rust-owned texture.
    pub texture_id: u32,
    pub translucent: bool,
}

/// A fragment of a block sprite, with game-provided tint and sampled lighting.
/// Separate from ordinary particles: these UVs address the owned block atlas,
/// and alpha-tested block surfaces are not alpha-blended particles.
/// Private until the typed transport and paired terrain captures are wired.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TerrainParticleQuad {
    pub quad: ParticleQuad,
    pub alpha_tested: bool,
}

impl TerrainParticleQuad {
    pub(crate) fn lower(self, viewport: [u32; 2]) -> GalResult<WorldMaterialQuadRequest> {
        if self.quad.translucent && self.alpha_tested {
            return Err(GalError::invalid_argument("terrain particle cannot be both cutout and translucent"));
        }
        let mut request = self.quad.lower(viewport)?;
        request.source_uv_space = WORLD_MATERIAL_SOURCE_UV_MINECRAFT_BLOCK_ATLAS;
        if self.alpha_tested {
            request.material_id = WORLD_MATERIAL_ID_CUTOUT_TEXTURED;
            request.material_mode = WORLD_MATERIAL_MODE_CUTOUT;
        }
        Ok(request)
    }
}

impl ParticleQuad {
    pub(crate) fn lower_surface(self, surface: ParticleSurface, viewport: [u32; 2])
        -> GalResult<WorldMaterialQuadRequest> {
        if self.translucent != surface.translucent() {
            return Err(GalError::invalid_argument("particle surface disagrees with translucency"));
        }
        match surface {
            ParticleSurface::TerrainOpaque | ParticleSurface::TerrainCutout | ParticleSurface::TerrainTranslucent =>
                TerrainParticleQuad { quad:self,
                    alpha_tested:matches!(surface,ParticleSurface::TerrainCutout) }.lower(viewport),
            _ => self.lower(viewport),
        }
    }
    pub(crate) fn lower(self, viewport: [u32; 2]) -> GalResult<WorldMaterialQuadRequest> {
        if self.texture_id == 0 || viewport.iter().any(|&v|
            v == 0 || v > super::super::SEMANTIC_MAX_VIEWPORT_AXIS as u32) {
            return Err(GalError::invalid_argument("particle requires a texture identity and bounded viewport"));
        }
        if self.center.iter().chain(self.rotation.iter()).chain(self.uv_bounds.iter())
            .chain(std::iter::once(&self.size)).any(|v| !v.is_finite()) {
            return Err(GalError::invalid_argument("particle semantics must be finite"));
        }
        let [u0,u1,v0,v1] = self.uv_bounds;
        for span in [(u1-u0).abs(), (v1-v0).abs()] {
            if !span.is_finite() || span == 0.0 || span > 4096.0 {
                return Err(GalError::invalid_argument("particle sprite span is invalid"));
            }
        }
        let [x,y,z,w] = self.rotation;
        let (xx,yy,zz,ww) = (x*x,y*y,z*z,w*w);
        let norm = xx+yy+zz+ww;
        if !norm.is_finite() || norm <= 1.0e-8 {
            return Err(GalError::invalid_argument("particle rotation must be nonzero and bounded"));
        }
        let k = 1.0/norm;
        let (xy,xz,yz,xw,zw,yw) = (x*y,x*z,y*z,x*w,z*w,y*w);
        // Frozen QuadParticleRenderState.renderVertex rotates the unit corner,
        // then multiplies by signed size, then adds camera-relative position.
        // Use the general quaternion transform, not a unit-quaternion shortcut.
        let columns = [
            [(xx-yy-zz+ww)*k, 2.0*(xy+zw)*k, 2.0*(xz-yw)*k],
            [2.0*(xy-zw)*k, (yy-xx-zz+ww)*k, 2.0*(yz+xw)*k],
        ];
        let vertices = [[1.0,-1.0],[1.0,1.0],[-1.0,1.0],[-1.0,-1.0]]
            .map(|[a,b]| std::array::from_fn(|i|
                (columns[0][i]*a + columns[1][i]*b)*self.size + self.center[i]));
        if vertices.iter().flatten().any(|v: &f32| !v.is_finite()) {
            return Err(GalError::invalid_argument("particle geometry overflow"));
        }
        Ok(WorldMaterialQuadRequest {
            stratum: WORLD_STRATUM_OPAQUE_TEXTURED_GEOMETRY,
            material_id: if self.translucent { WORLD_MATERIAL_ID_TRANSLUCENT_TEXTURED }
                else { WORLD_MATERIAL_ID_OPAQUE_TEXTURED },
            texture_id: self.texture_id,
            material_mode: if self.translucent { WORLD_MATERIAL_MODE_TRANSLUCENT }
                else { WORLD_MATERIAL_MODE_OPAQUE },
            // Ordinary Frozen particle pipelines retain depth writes and back culling.
            depth_policy: WORLD_DEPTH_POLICY_TEST_WRITE,
            cull_policy: WORLD_CULL_BACK,
            topology: WORLD_TOPOLOGY_TRIANGLES,
            winding: WORLD_WINDING_CCW,
            color_argb: self.color_argb,
            vertices,
            uvs: [[u1,v1],[u1,v0],[u0,v0],[u0,v1]],
            viewport_width: viewport[0], viewport_height: viewport[1],
            source_program: WORLD_MATERIAL_SOURCE_PARTICLES,
            source_uv_space: WORLD_MATERIAL_SOURCE_UV_LOCAL_TEXTURE,
            source_color_argb: self.color_argb,
            packed_light: self.packed_light,
            vertex_color_argb: [self.color_argb;4],
            vertex_packed_light: [self.packed_light;4],
            block_entity_id: -1,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn particle() -> ParticleQuad {
        ParticleQuad { center: [1.25,-0.7,2.5], rotation: [0.2,-0.3,0.4,0.5],
            size: 0.3, uv_bounds: [0.8,0.2,0.1,0.9], color_argb: 0x7fabcdef,
            packed_light: 0x00f00080, texture_id: 0x50415254, translucent: true }
    }
    #[test]
    fn frozen_joml_1_10_5_general_rotation_reference() {
        // Captured with the dependency used by Frozen, evaluating its exact
        // renderVertex expression (rotate, mul, add), not this Rust algorithm.
        let reference = [[1.5611111,-0.6222222,2.7777777],
            [0.98333335,-0.46666664,2.7333333],
            [0.9388889,-0.7777778,2.2222223],
            [1.5166667,-0.93333334,2.2666667]];
        let quad = particle().lower([1280,720]).unwrap();
        for (actual,expected) in quad.vertices.iter().flatten().zip(reference.iter().flatten()) {
            assert!((actual-expected).abs() <= 0.00000024, "{actual} != {expected}");
        }
        assert_eq!(quad.uvs, [[0.2,0.9],[0.2,0.1],[0.8,0.1],[0.8,0.9]]);
        assert_eq!(quad.depth_policy, WORLD_DEPTH_POLICY_TEST_WRITE);
        assert_eq!(quad.cull_policy, WORLD_CULL_BACK);
        assert_eq!(quad.source_program, WORLD_MATERIAL_SOURCE_PARTICLES);
        assert_eq!(quad.vertex_color_argb, [0x7fabcdef;4]);
        assert_eq!(quad.vertex_packed_light, [0x00f00080;4]);
    }
    #[test]
    fn signed_size_zero_size_and_opaque_policy() {
        let mut p = particle();
        p.center = [0.0;3]; p.rotation = [0.0,0.0,0.0,1.0]; p.size = -2.0;
        p.translucent = false;
        let q = p.lower([1280,720]).unwrap();
        assert_eq!(q.vertices, [[-2.0,2.0,0.0],[-2.0,-2.0,0.0],[2.0,-2.0,0.0],[2.0,2.0,0.0]]);
        assert_eq!(q.material_mode, WORLD_MATERIAL_MODE_OPAQUE);
        p.size = 0.0;
        assert_eq!(p.lower([1280,720]).unwrap().vertices, [[0.0;3];4]);
    }
    #[test]
    fn invalid_semantics_fail_before_material_admission() {
        let mut invalid = Vec::new();
        let mut p = particle(); p.rotation = [0.0;4]; invalid.push(p);
        let mut p = particle(); p.rotation[0] = f32::MAX; invalid.push(p);
        let mut p = particle(); p.center[0] = f32::NAN; invalid.push(p);
        let mut p = particle(); p.size = f32::INFINITY; invalid.push(p);
        let mut p = particle(); p.uv_bounds[0] = p.uv_bounds[1]; invalid.push(p);
        let mut p = particle(); p.uv_bounds[0] = 5000.0; invalid.push(p);
        let mut p = particle(); p.texture_id = 0; invalid.push(p);
        for p in invalid { assert!(p.lower([1280,720]).is_err()); }
        for viewport in [[0,720],[1280,0],[16385,720]] {
            assert!(particle().lower(viewport).is_err());
        }
    }

    #[test]
    fn terrain_fragment_keeps_block_atlas_uv_tint_light_and_surface_class() {
        let mut p = particle();
        p.translucent = false;
        // TerrainParticle.getU0/getU1 reverses U for its quarter-sprite fragment.
        p.uv_bounds = [0.5, 0.25, 0.25, 0.5];
        let ordinary = p.lower([1280,720]).unwrap();
        for alpha_tested in [false,true] {
            let q = TerrainParticleQuad { quad:p, alpha_tested }.lower([1280,720]).unwrap();
            assert_eq!(q.vertices, ordinary.vertices);
            assert_eq!(q.uvs, [[0.25,0.5],[0.25,0.25],[0.5,0.25],[0.5,0.5]]);
            assert_eq!(q.source_uv_space, WORLD_MATERIAL_SOURCE_UV_MINECRAFT_BLOCK_ATLAS);
            assert_eq!(q.source_program, WORLD_MATERIAL_SOURCE_PARTICLES);
            assert_eq!(q.color_argb, p.color_argb);
            assert_eq!(q.vertex_color_argb, [p.color_argb;4]);
            assert_eq!(q.vertex_packed_light, [p.packed_light;4]);
            assert_eq!(q.depth_policy, WORLD_DEPTH_POLICY_TEST_WRITE);
            assert_eq!(q.cull_policy, WORLD_CULL_BACK);
            assert_eq!(q.material_mode, if alpha_tested { WORLD_MATERIAL_MODE_CUTOUT }
                else { WORLD_MATERIAL_MODE_OPAQUE });
            assert_eq!(q.material_id, if alpha_tested { WORLD_MATERIAL_ID_CUTOUT_TEXTURED }
                else { WORLD_MATERIAL_ID_OPAQUE_TEXTURED });
        }
    }

    #[test]
    fn translucent_terrain_keeps_blending_block_uvs_tint_light_and_depth_write() {
        let mut p = particle();
        p.uv_bounds = [0.5,0.25,0.25,0.5];
        p.color_argb = 0x80604020;
        let q = p.lower_surface(ParticleSurface::TerrainTranslucent,[1280,720]).unwrap();
        assert_eq!(q.material_mode,WORLD_MATERIAL_MODE_TRANSLUCENT);
        assert_eq!(q.material_id,WORLD_MATERIAL_ID_TRANSLUCENT_TEXTURED);
        assert_eq!(q.source_uv_space,WORLD_MATERIAL_SOURCE_UV_MINECRAFT_BLOCK_ATLAS);
        assert_eq!(q.source_program,WORLD_MATERIAL_SOURCE_PARTICLES);
        assert_eq!(q.vertex_color_argb,[0x80604020;4]);
        assert_eq!(q.vertex_packed_light,[p.packed_light;4]);
        assert_eq!(q.depth_policy,WORLD_DEPTH_POLICY_TEST_WRITE);
        assert_eq!(q.cull_policy,WORLD_CULL_BACK);
        assert_eq!(q.uvs,[[0.25,0.5],[0.25,0.25],[0.5,0.25],[0.5,0.5]]);
        assert!(p.lower_surface(ParticleSurface::TerrainOpaque,[1280,720]).is_err());
        p.translucent=false;
        assert!(p.lower_surface(ParticleSurface::TerrainTranslucent,[1280,720]).is_err());
    }

    #[test]
    fn terrain_fragment_rejects_incompatible_or_malformed_semantics() {
        assert!(TerrainParticleQuad { quad:particle(), alpha_tested:true }.lower([1280,720]).is_err());
        let mut p = particle(); p.translucent = false; p.rotation = [0.0;4];
        assert!(TerrainParticleQuad { quad:p, alpha_tested:false }.lower([1280,720]).is_err());
    }
}
