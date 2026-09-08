//! Material preparation for semantic flat GUI items, independent of backend state.
//!
//! Full-bright item vertices still sample the vanilla lightmap at (15, 15).
//! They do not imply white lighting. Keep this input distinct from texture
//! regions, packed tint colors, and GUI scheduling strata.

use super::error::{GalError, GalResult};
use super::shader_pack::lightmap::VanillaLightmapFrame;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GuiAffineMaterial {
    Unlit,
    FlatItemPending,
    FlatItem(GuiFlatItemLighting),
    FlatItemCutoutPending,
    FlatItemCutout(GuiFlatItemLighting),
}

impl GuiAffineMaterial {
    pub fn decode(mode: u32) -> GalResult<Self> {
        match mode {
            0 => Ok(Self::Unlit),
            1 => Ok(Self::FlatItemPending),
            2 => Ok(Self::FlatItemCutoutPending),
            _ => Err(GalError::invalid_argument("unknown GUI affine material")),
        }
    }

    pub fn resolve(&mut self, lightmap: Option<VanillaLightmapFrame>) -> GalResult<()> {
        if matches!(self, Self::FlatItemPending | Self::FlatItemCutoutPending) {
            let frame = lightmap.ok_or_else(|| GalError::invalid_argument(
                "flat GUI item requires explicit frame lightmap inputs"))?;
            let lighting = GuiFlatItemLighting::prepare(frame)?;
            *self = if matches!(self, Self::FlatItemCutoutPending) {
                Self::FlatItemCutout(lighting)
            } else { Self::FlatItem(lighting) };
        }
        Ok(())
    }

    pub fn color(self, color: [f32; 4]) -> GalResult<[f32; 4]> {
        match self {
            Self::Unlit => Ok(color),
            Self::FlatItemPending | Self::FlatItemCutoutPending => Err(GalError::invalid_argument("unresolved GUI item lighting")),
            Self::FlatItem(lighting) | Self::FlatItemCutout(lighting) => lighting.modulate(color),
        }
    }

    pub fn is_cutout(self) -> bool {
        matches!(self, Self::FlatItemCutoutPending | Self::FlatItemCutout(_))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuiFlatItemLighting {
    pub lightmap_generation: u64,
    pub rgb: [f32; 3],
}

impl GuiFlatItemLighting {
    pub fn prepare(frame: VanillaLightmapFrame) -> GalResult<Self> {
        frame.validate()?;
        let color = frame.inputs.texel(15, 15)?;
        // Match the explicit RGBA8 lightmap resource's quantization before
        // modulation; sampling its unquantized generation formula is different.
        let rgb = color.map(|value| (value.clamp(0.0, 1.0) * 255.0).round() / 255.0);
        Ok(Self { lightmap_generation: frame.generation, rgb })
    }

    pub fn modulate(self, color: [f32; 4]) -> GalResult<[f32; 4]> {
        if color.iter().any(|value| !value.is_finite() || !(0.0..=1.0).contains(value)) {
            return Err(GalError::invalid_argument("invalid flat GUI item material color"));
        }
        Ok([color[0] * self.rgb[0], color[1] * self.rgb[1], color[2] * self.rgb[2], color[3]])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::vulkanic::shader_pack::lightmap::VanillaLightmapInputs;

    fn frame() -> VanillaLightmapFrame {
        VanillaLightmapFrame { generation: 7, inputs: VanillaLightmapInputs {
            ambient_light_factor: 0.0, sky_factor: 1.0, block_factor: 1.5,
            night_vision_factor: 0.0, darkness_scale: 0.0, darken_world_factor: 0.0,
            brightness_factor: 0.0, sky_light_color: [1.0; 3], ambient_color: [1.0; 3],
        } }
    }

    #[test]
    fn cutout_material_preserves_tint_alpha_and_requires_explicit_lighting() {
        let mut material = GuiAffineMaterial::decode(2).unwrap();
        assert!(material.is_cutout());
        assert!(material.resolve(None).is_err());
        material.resolve(Some(frame())).unwrap();
        assert!(material.is_cutout());
        assert_eq!(material.color([1.0,1.0,1.0,0.05]).unwrap(), [252.0/255.0,252.0/255.0,252.0/255.0,0.05]);
    }

    #[test]
    fn affine_material_requires_explicit_lighting_only_for_items() {
        assert!(GuiAffineMaterial::decode(3).is_err());
        let mut unlit = GuiAffineMaterial::decode(0).unwrap();
        unlit.resolve(None).unwrap();
        assert_eq!(unlit.color([1.0; 4]).unwrap(), [1.0; 4]);
        let mut item = GuiAffineMaterial::decode(1).unwrap();
        assert!(item.color([1.0; 4]).is_err());
        assert!(item.resolve(None).is_err());
        assert_eq!(item, GuiAffineMaterial::FlatItemPending);
        item.resolve(Some(frame())).unwrap();
        assert_eq!(item.color([1.0; 4]).unwrap(), [252.0 / 255.0, 252.0 / 255.0, 252.0 / 255.0, 1.0]);
    }

    #[test]
    fn gui_fullbright_item_samples_quantized_lightmap_instead_of_white() {
        let frame = frame();
        let lighting = GuiFlatItemLighting::prepare(frame).unwrap();
        assert_eq!(lighting.lightmap_generation, 7);
        assert_eq!(lighting.rgb, [252.0 / 255.0; 3]);
        let pixels = frame.inputs.rgba8().unwrap();
        let offset = (15 * 16 + 15) * 4;
        assert_eq!(lighting.rgb, [pixels[offset] as f32 / 255.0,
            pixels[offset + 1] as f32 / 255.0, pixels[offset + 2] as f32 / 255.0]);
        assert_eq!(lighting.modulate([1.0, 0.5, 0.0, 0.25]).unwrap(),
            [252.0 / 255.0, 126.0 / 255.0, 0.0, 0.25]);
    }

    #[test]
    fn gui_item_lighting_tracks_semantic_environment_and_rejects_invalid_inputs() {
        let mut changed = frame();
        changed.generation = 8;
        changed.inputs.darkness_scale = 4.0;
        let lighting = GuiFlatItemLighting::prepare(changed).unwrap();
        assert_eq!(lighting.lightmap_generation, 8);
        assert_ne!(lighting.rgb, GuiFlatItemLighting::prepare(frame()).unwrap().rgb);
        assert!(lighting.modulate([f32::NAN, 1.0, 1.0, 1.0]).is_err());
        changed.generation = 0;
        assert!(GuiFlatItemLighting::prepare(changed).is_err());
        changed = frame();
        changed.inputs.sky_factor = f32::NAN;
        assert!(GuiFlatItemLighting::prepare(changed).is_err());
    }
}
