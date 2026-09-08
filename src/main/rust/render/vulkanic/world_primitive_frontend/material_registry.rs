use super::*;

pub(crate) const WORLD_MATERIAL_REGISTRY_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MaterialSamplerPolicy {
    NearestClamp,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MaterialMipPolicy {
    SingleMip,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MaterialTintChannel {
    VertexColor,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SemanticMaterial {
    pub(crate) key: u32,
    pub(crate) resource_location: &'static str,
    pub(crate) mode: u32,
    pub(crate) cutout_threshold: f32,
    pub(crate) perspective_layer_scale: f32,
    pub(crate) sampler: MaterialSamplerPolicy,
    pub(crate) mip: MaterialMipPolicy,
    pub(crate) tint: MaterialTintChannel,
    pub(crate) emissive: bool,
    pub(crate) fullbright: bool,
    pub(crate) legacy_keys: &'static [u32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SemanticTexture {
    pub(crate) key: u32,
    pub(crate) resource_location: &'static str,
    pub(crate) default_png: &'static [u8],
    pub(crate) legacy_keys: &'static [u32],
}

const MATERIALS: &[SemanticMaterial] = &[
    SemanticMaterial {
        key: WORLD_MATERIAL_ID_TRANSLUCENT_CUTOUT_TEXTURED,
        resource_location: "minecraft:material/translucent_cutout_textured",
        mode: WORLD_MATERIAL_MODE_TRANSLUCENT_CUTOUT,
        cutout_threshold: 0.1,
        perspective_layer_scale: 1.0,
        sampler: MaterialSamplerPolicy::NearestClamp,
        mip: MaterialMipPolicy::SingleMip,
        tint: MaterialTintChannel::VertexColor,
        emissive: false,
        fullbright: false,
        legacy_keys: &[],
    },
    SemanticMaterial {
        key: WORLD_MATERIAL_ID_CELESTIAL,
        resource_location: "minecraft:material/celestial",
        mode: WORLD_MATERIAL_MODE_TRANSLUCENT,
        cutout_threshold: 0.0,
        perspective_layer_scale: 1.0,
        sampler: MaterialSamplerPolicy::NearestClamp,
        mip: MaterialMipPolicy::SingleMip,
        tint: MaterialTintChannel::VertexColor,
        emissive: true,
        fullbright: true,
        legacy_keys: &[],
    },
    SemanticMaterial {
        key: WORLD_MATERIAL_ID_SKY_STARS,
        resource_location: "minecraft:material/sky_stars",
        mode: WORLD_MATERIAL_MODE_TRANSLUCENT,
        cutout_threshold: 0.0,
        perspective_layer_scale: 1.0,
        sampler: MaterialSamplerPolicy::NearestClamp,
        mip: MaterialMipPolicy::SingleMip,
        tint: MaterialTintChannel::VertexColor,
        emissive: true,
        fullbright: true,
        legacy_keys: &[],
    },
    SemanticMaterial {
        key: WORLD_MATERIAL_ID_OPAQUE_TEXTURED,
        resource_location: "minecraft:material/opaque_textured",
        mode: WORLD_MATERIAL_MODE_OPAQUE,
        cutout_threshold: 0.0,
        sampler: MaterialSamplerPolicy::NearestClamp,
        mip: MaterialMipPolicy::SingleMip,
        tint: MaterialTintChannel::VertexColor,
        emissive: false,
        fullbright: false,
        legacy_keys: &[1],
        perspective_layer_scale: 1.0,
    },
    SemanticMaterial {
        key: WORLD_MATERIAL_ID_CUTOUT_TEXTURED,
        resource_location: "minecraft:material/cutout_textured",
        mode: WORLD_MATERIAL_MODE_CUTOUT,
        cutout_threshold: 0.5,
        sampler: MaterialSamplerPolicy::NearestClamp,
        mip: MaterialMipPolicy::SingleMip,
        tint: MaterialTintChannel::VertexColor,
        emissive: false,
        fullbright: false,
        legacy_keys: &[2],
        perspective_layer_scale: 1.0,
    },
    SemanticMaterial {
        key: WORLD_MATERIAL_ID_TRANSLUCENT_TEXTURED,
        resource_location: "minecraft:material/translucent_textured",
        mode: WORLD_MATERIAL_MODE_TRANSLUCENT,
        cutout_threshold: 0.0,
        sampler: MaterialSamplerPolicy::NearestClamp,
        mip: MaterialMipPolicy::SingleMip,
        tint: MaterialTintChannel::VertexColor,
        emissive: false,
        fullbright: false,
        legacy_keys: &[3],
        perspective_layer_scale: 1.0,
    },
    SemanticMaterial {
        key: WORLD_MATERIAL_ID_ENTITY_SHADOW,
        resource_location: "minecraft:material/entity_shadow",
        mode: WORLD_MATERIAL_MODE_TRANSLUCENT,
        cutout_threshold: 0.0,
        perspective_layer_scale: 1.0 - 1.0 / 4096.0,
        sampler: MaterialSamplerPolicy::NearestClamp,
        mip: MaterialMipPolicy::SingleMip,
        tint: MaterialTintChannel::VertexColor,
        emissive: false,
        fullbright: true,
        legacy_keys: &[],
    },
    SemanticMaterial {
        key: WORLD_MATERIAL_ID_GLINT_TEXTURED,
        perspective_layer_scale: 1.0,
        resource_location: "minecraft:material/glint_textured",
        mode: WORLD_MATERIAL_MODE_GLINT,
        cutout_threshold: 0.0,
        sampler: MaterialSamplerPolicy::NearestClamp,
        mip: MaterialMipPolicy::SingleMip,
        tint: MaterialTintChannel::VertexColor,
        emissive: true,
        fullbright: true,
        legacy_keys: &[],
    },
    SemanticMaterial {
        key: WORLD_MATERIAL_ID_WATER_TRANSLUCENT,
        perspective_layer_scale: 1.0,
        resource_location: "minecraft:material/water_translucent",
        mode: WORLD_MATERIAL_MODE_TRANSLUCENT,
        cutout_threshold: 0.0,
        sampler: MaterialSamplerPolicy::NearestClamp,
        mip: MaterialMipPolicy::SingleMip,
        tint: MaterialTintChannel::VertexColor,
        emissive: false,
        fullbright: false,
        legacy_keys: &[],
    },
    SemanticMaterial {
        key: WORLD_MATERIAL_ID_BLOCK_MARKER_CUTOUT,
        perspective_layer_scale: 1.0,
        resource_location: "minecraft:material/block_marker_cutout",
        mode: WORLD_MATERIAL_MODE_CUTOUT,
        cutout_threshold: 0.5,
        sampler: MaterialSamplerPolicy::NearestClamp,
        mip: MaterialMipPolicy::SingleMip,
        tint: MaterialTintChannel::VertexColor,
        emissive: false,
        fullbright: true,
        legacy_keys: &[100],
    },
];

/// Explicit material composition, independent of texture identity. Frozen's
/// star contract adds src.rgb * src.a and replaces alpha, unlike translucency.
pub(crate) fn blend_override(material_key: u32) -> Option<BlendMode> {
    matches!(material_key, WORLD_MATERIAL_ID_SKY_STARS | WORLD_MATERIAL_ID_CELESTIAL)
        .then_some(BlendMode::Overlay)
}

const TEXTURES: &[SemanticTexture] = &[
    SemanticTexture {
        key: WORLD_MATERIAL_TEXTURE_STONE,
        resource_location: "minecraft:textures/block/stone.png",
        default_png: include_bytes!(
            "../../../../resources/assets/minecraft/textures/block/stone.png"
        ),
        legacy_keys: &[1],
    },
    SemanticTexture {
        key: WORLD_MATERIAL_TEXTURE_DIRT,
        resource_location: "minecraft:textures/block/dirt.png",
        default_png: include_bytes!(
            "../../../../resources/assets/minecraft/textures/block/dirt.png"
        ),
        legacy_keys: &[2],
    },
    SemanticTexture {
        key: WORLD_MATERIAL_TEXTURE_OAK_LEAVES,
        resource_location: "minecraft:textures/block/oak_leaves.png",
        default_png: include_bytes!(
            "../../../../resources/assets/minecraft/textures/block/oak_leaves.png"
        ),
        legacy_keys: &[3],
    },
    SemanticTexture {
        key: WORLD_MATERIAL_TEXTURE_DEEPSLATE,
        resource_location: "minecraft:textures/block/deepslate.png",
        default_png: include_bytes!(
            "../../../../resources/assets/minecraft/textures/block/deepslate.png"
        ),
        legacy_keys: &[4],
    },
    SemanticTexture {
        key: WORLD_MATERIAL_TEXTURE_WHITE_WOOL,
        resource_location: "minecraft:textures/block/white_wool.png",
        default_png: include_bytes!(
            "../../../../resources/assets/minecraft/textures/block/white_wool.png"
        ),
        legacy_keys: &[5],
    },
    SemanticTexture {
        key: WORLD_MATERIAL_TEXTURE_BLOCK_MARKER_BARRIER,
        resource_location: "minecraft:textures/item/barrier.png",
        default_png: include_bytes!(
            "../../../../resources/assets/minecraft/textures/item/barrier.png"
        ),
        legacy_keys: &[100],
    },
    SemanticTexture {
        key: WORLD_MATERIAL_TEXTURE_BLOCK_MARKER_LIGHT_00,
        resource_location: "minecraft:textures/item/light_00.png",
        default_png: include_bytes!(
            "../../../../resources/assets/minecraft/textures/item/light_00.png"
        ),
        legacy_keys: &[200],
    },
    SemanticTexture {
        key: WORLD_MATERIAL_TEXTURE_BLOCK_MARKER_LIGHT_01,
        resource_location: "minecraft:textures/item/light_01.png",
        default_png: include_bytes!(
            "../../../../resources/assets/minecraft/textures/item/light_01.png"
        ),
        legacy_keys: &[201],
    },
    SemanticTexture {
        key: WORLD_MATERIAL_TEXTURE_BLOCK_MARKER_LIGHT_02,
        resource_location: "minecraft:textures/item/light_02.png",
        default_png: include_bytes!(
            "../../../../resources/assets/minecraft/textures/item/light_02.png"
        ),
        legacy_keys: &[202],
    },
    SemanticTexture {
        key: WORLD_MATERIAL_TEXTURE_BLOCK_MARKER_LIGHT_03,
        resource_location: "minecraft:textures/item/light_03.png",
        default_png: include_bytes!(
            "../../../../resources/assets/minecraft/textures/item/light_03.png"
        ),
        legacy_keys: &[203],
    },
    SemanticTexture {
        key: WORLD_MATERIAL_TEXTURE_BLOCK_MARKER_LIGHT_04,
        resource_location: "minecraft:textures/item/light_04.png",
        default_png: include_bytes!(
            "../../../../resources/assets/minecraft/textures/item/light_04.png"
        ),
        legacy_keys: &[204],
    },
    SemanticTexture {
        key: WORLD_MATERIAL_TEXTURE_BLOCK_MARKER_LIGHT_05,
        resource_location: "minecraft:textures/item/light_05.png",
        default_png: include_bytes!(
            "../../../../resources/assets/minecraft/textures/item/light_05.png"
        ),
        legacy_keys: &[205],
    },
    SemanticTexture {
        key: WORLD_MATERIAL_TEXTURE_BLOCK_MARKER_LIGHT_06,
        resource_location: "minecraft:textures/item/light_06.png",
        default_png: include_bytes!(
            "../../../../resources/assets/minecraft/textures/item/light_06.png"
        ),
        legacy_keys: &[206],
    },
    SemanticTexture {
        key: WORLD_MATERIAL_TEXTURE_BLOCK_MARKER_LIGHT_07,
        resource_location: "minecraft:textures/item/light_07.png",
        default_png: include_bytes!(
            "../../../../resources/assets/minecraft/textures/item/light_07.png"
        ),
        legacy_keys: &[207],
    },
    SemanticTexture {
        key: WORLD_MATERIAL_TEXTURE_BLOCK_MARKER_LIGHT_08,
        resource_location: "minecraft:textures/item/light_08.png",
        default_png: include_bytes!(
            "../../../../resources/assets/minecraft/textures/item/light_08.png"
        ),
        legacy_keys: &[208],
    },
    SemanticTexture {
        key: WORLD_MATERIAL_TEXTURE_BLOCK_MARKER_LIGHT_09,
        resource_location: "minecraft:textures/item/light_09.png",
        default_png: include_bytes!(
            "../../../../resources/assets/minecraft/textures/item/light_09.png"
        ),
        legacy_keys: &[209],
    },
    SemanticTexture {
        key: WORLD_MATERIAL_TEXTURE_BLOCK_MARKER_LIGHT_10,
        resource_location: "minecraft:textures/item/light_10.png",
        default_png: include_bytes!(
            "../../../../resources/assets/minecraft/textures/item/light_10.png"
        ),
        legacy_keys: &[210],
    },
    SemanticTexture {
        key: WORLD_MATERIAL_TEXTURE_BLOCK_MARKER_LIGHT_11,
        resource_location: "minecraft:textures/item/light_11.png",
        default_png: include_bytes!(
            "../../../../resources/assets/minecraft/textures/item/light_11.png"
        ),
        legacy_keys: &[211],
    },
    SemanticTexture {
        key: WORLD_MATERIAL_TEXTURE_BLOCK_MARKER_LIGHT_12,
        resource_location: "minecraft:textures/item/light_12.png",
        default_png: include_bytes!(
            "../../../../resources/assets/minecraft/textures/item/light_12.png"
        ),
        legacy_keys: &[212],
    },
    SemanticTexture {
        key: WORLD_MATERIAL_TEXTURE_BLOCK_MARKER_LIGHT_13,
        resource_location: "minecraft:textures/item/light_13.png",
        default_png: include_bytes!(
            "../../../../resources/assets/minecraft/textures/item/light_13.png"
        ),
        legacy_keys: &[213],
    },
    SemanticTexture {
        key: WORLD_MATERIAL_TEXTURE_BLOCK_MARKER_LIGHT_14,
        resource_location: "minecraft:textures/item/light_14.png",
        default_png: include_bytes!(
            "../../../../resources/assets/minecraft/textures/item/light_14.png"
        ),
        legacy_keys: &[214],
    },
    SemanticTexture {
        key: WORLD_MATERIAL_TEXTURE_BLOCK_MARKER_LIGHT_15,
        resource_location: "minecraft:textures/item/light_15.png",
        default_png: include_bytes!(
            "../../../../resources/assets/minecraft/textures/item/light_15.png"
        ),
        legacy_keys: &[215],
    },
    SemanticTexture {
        key: WORLD_MATERIAL_TEXTURE_WATER_STILL,
        resource_location: "minecraft:textures/block/water_still.png",
        default_png: include_bytes!(
            "../../../../resources/assets/minecraft/textures/block/water_still.png"
        ),
        legacy_keys: &[],
    },
    SemanticTexture {
        key: WORLD_MATERIAL_TEXTURE_WATER_FLOW,
        resource_location: "minecraft:textures/block/water_flow.png",
        default_png: include_bytes!(
            "../../../../resources/assets/minecraft/textures/block/water_flow.png"
        ),
        legacy_keys: &[],
    },
    SemanticTexture {
        key: WORLD_MATERIAL_TEXTURE_WATER_OVERLAY,
        resource_location: "minecraft:textures/block/water_overlay.png",
        default_png: include_bytes!(
            "../../../../resources/assets/minecraft/textures/block/water_overlay.png"
        ),
        legacy_keys: &[],
    },
    SemanticTexture {
        key: WORLD_MATERIAL_TEXTURE_WEATHER_RAIN,
        resource_location: "minecraft:textures/environment/rain.png",
        default_png: include_bytes!(
            "../../../../resources/assets/minecraft/textures/environment/rain.png"
        ),
        legacy_keys: &[],
    },
    SemanticTexture {
        key: WORLD_MATERIAL_TEXTURE_WEATHER_SNOW,
        resource_location: "minecraft:textures/environment/snow.png",
        default_png: include_bytes!(
            "../../../../resources/assets/minecraft/textures/environment/snow.png"
        ),
        legacy_keys: &[],
    },
    SemanticTexture {
        key: WORLD_MATERIAL_TEXTURE_SKY_SUN,
        resource_location: "minecraft:textures/environment/sun.png",
        default_png: include_bytes!(
            "../../../../resources/assets/minecraft/textures/environment/sun.png"
        ),
        legacy_keys: &[],
    },
    SemanticTexture {
        key: WORLD_MATERIAL_TEXTURE_SKY_MOON_PHASES,
        resource_location: "minecraft:textures/environment/moon_phases.png",
        default_png: include_bytes!(
            "../../../../resources/assets/minecraft/textures/environment/moon_phases.png"
        ),
        legacy_keys: &[],
    },
    SemanticTexture {
        key: WORLD_MATERIAL_TEXTURE_EXPERIENCE_ORB,
        resource_location: "minecraft:textures/entity/experience_orb.png",
        default_png: include_bytes!(
            "../../../../resources/assets/minecraft/textures/entity/experience_orb.png"
        ),
        legacy_keys: &[],
    },
    SemanticTexture {
        key: WORLD_MATERIAL_TEXTURE_BEACON_BEAM,
        resource_location: "minecraft:textures/entity/beacon_beam.png",
        default_png: include_bytes!(
            "../../../../resources/assets/minecraft/textures/entity/beacon_beam.png"
        ),
        legacy_keys: &[],
    },
    SemanticTexture {
        key: WORLD_MATERIAL_TEXTURE_CRYSTAL_BEAM,
        resource_location: "minecraft:textures/entity/end_crystal/end_crystal_beam.png",
        default_png: include_bytes!(
            "../../../../resources/assets/minecraft/textures/entity/end_crystal/end_crystal_beam.png"
        ),
        legacy_keys: &[],
    },
    SemanticTexture {
        key: WORLD_MATERIAL_TEXTURE_END_GATEWAY_BEAM,
        resource_location: "minecraft:textures/entity/end_gateway_beam.png",
        default_png: include_bytes!(
            "../../../../resources/assets/minecraft/textures/entity/end_gateway_beam.png"
        ),
        legacy_keys: &[],
    },
    SemanticTexture {
        key: WORLD_MATERIAL_TEXTURE_END_SKY,
        resource_location: "minecraft:textures/environment/end_sky.png",
        default_png: include_bytes!(
            "../../../../resources/assets/minecraft/textures/environment/end_sky.png"
        ),
        legacy_keys: &[],
    },
    SemanticTexture {
        key: WORLD_MATERIAL_TEXTURE_END_FLASH,
        resource_location: "minecraft:textures/environment/end_flash.png",
        default_png: include_bytes!(
            "../../../../resources/assets/minecraft/textures/environment/end_flash.png"
        ),
        legacy_keys: &[],
    },
    SemanticTexture {
        key: WORLD_MATERIAL_TEXTURE_END_PORTAL,
        resource_location: "minecraft:textures/entity/end_portal.png",
        default_png: include_bytes!(
            "../../../../resources/assets/minecraft/textures/entity/end_portal.png"
        ),
        legacy_keys: &[],
    },
    SemanticTexture {
        key: WORLD_MATERIAL_TEXTURE_ENTITY_SHADOW,
        resource_location: "minecraft:textures/misc/shadow.png",
        default_png: include_bytes!(
            "../../../../resources/assets/minecraft/textures/misc/shadow.png"
        ),
        legacy_keys: &[],
    },
    SemanticTexture {
        key: WORLD_MATERIAL_TEXTURE_GENERATED_WHITE,
        resource_location: "vulkanic:generated/white",
        default_png: include_bytes!(
            "../../../../resources/assets/minecraft/textures/item/white_dye.png"
        ),
        legacy_keys: &[],
    },
];

pub(crate) fn material(key: u32) -> Option<&'static SemanticMaterial> {
    let canonical = canonical_material_key(key)?;
    MATERIALS.iter().find(|entry| entry.key == canonical)
}

pub(crate) fn texture(key: u32) -> Option<&'static SemanticTexture> {
    let canonical = canonical_texture_key(key)?;
    TEXTURES.iter().find(|entry| entry.key == canonical)
}

pub(crate) fn canonical_material_key(key: u32) -> Option<u32> {
    MATERIALS
        .iter()
        .find(|entry| entry.key == key || entry.legacy_keys.contains(&key))
        .map(|entry| entry.key)
}

pub(crate) fn canonical_texture_key(key: u32) -> Option<u32> {
    TEXTURES
        .iter()
        .find(|entry| entry.key == key || entry.legacy_keys.contains(&key))
        .map(|entry| entry.key)
}

pub(crate) fn is_known_material_key(key: u32) -> bool {
    canonical_material_key(key).is_some()
}

pub(crate) fn material_matches_mode(material_key: u32, mode: u32) -> bool {
    material(material_key)
        .map(|entry| entry.mode == mode)
        .unwrap_or(false)
}

pub(crate) fn cutout_threshold(material_key: u32) -> f32 {
    material(material_key)
        .map(|entry| entry.cutout_threshold)
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vanilla_celestial_textures_are_owned_semantic_assets() {
        for key in [
            WORLD_MATERIAL_TEXTURE_SKY_SUN,
            WORLD_MATERIAL_TEXTURE_SKY_MOON_PHASES,
        ] {
            let texture = texture(key).expect("vanilla celestial texture must be registered");
            assert!(!texture.default_png.is_empty());
            assert_eq!(Some(key), canonical_texture_key(key));
        }
    }

    #[test]
    fn weather_textures_are_stable_rust_owned_semantic_assets() {
        let rain = texture(WORLD_MATERIAL_TEXTURE_WEATHER_RAIN).expect("rain texture");
        let snow = texture(WORLD_MATERIAL_TEXTURE_WEATHER_SNOW).expect("snow texture");

        assert_eq!(
            rain.resource_location,
            "minecraft:textures/environment/rain.png"
        );
        assert_eq!(
            snow.resource_location,
            "minecraft:textures/environment/snow.png"
        );
        assert_ne!(rain.key, snow.key);
        assert!(!rain.default_png.is_empty());
        assert!(!snow.default_png.is_empty());
        assert!(material_matches_mode(
            WORLD_MATERIAL_ID_TRANSLUCENT_TEXTURED,
            WORLD_MATERIAL_MODE_TRANSLUCENT
        ));
    }

    #[test]
    fn experience_orb_sheet_is_a_stable_translucent_semantic_asset() {
        let orb = texture(WORLD_MATERIAL_TEXTURE_EXPERIENCE_ORB).expect("experience-orb texture");

        assert_eq!(
            orb.resource_location,
            "minecraft:textures/entity/experience_orb.png"
        );
        assert!(!orb.default_png.is_empty());
        assert_ne!(orb.key, WORLD_MATERIAL_TEXTURE_GENERATED_WHITE);
        assert!(material_matches_mode(
            WORLD_MATERIAL_ID_TRANSLUCENT_TEXTURED,
            WORLD_MATERIAL_MODE_TRANSLUCENT
        ));
    }

    #[test]
    fn beacon_beam_sheet_is_a_stable_translucent_semantic_asset() {
        let beam = texture(WORLD_MATERIAL_TEXTURE_BEACON_BEAM).expect("beacon-beam texture");

        assert_eq!(
            beam.resource_location,
            "minecraft:textures/entity/beacon_beam.png"
        );
        assert!(!beam.default_png.is_empty());
        assert_ne!(beam.key, WORLD_MATERIAL_TEXTURE_EXPERIENCE_ORB);
        assert!(material_matches_mode(
            WORLD_MATERIAL_ID_TRANSLUCENT_TEXTURED,
            WORLD_MATERIAL_MODE_TRANSLUCENT
        ));
    }

    #[test]
    fn end_portal_layers_are_stable_rust_owned_semantic_assets() {
        let sky = texture(WORLD_MATERIAL_TEXTURE_END_SKY).expect("end-sky texture");
        let portal = texture(WORLD_MATERIAL_TEXTURE_END_PORTAL).expect("end-portal texture");
        assert_eq!(
            sky.resource_location,
            "minecraft:textures/environment/end_sky.png"
        );
        assert_eq!(
            portal.resource_location,
            "minecraft:textures/entity/end_portal.png"
        );
        assert_ne!(sky.key, portal.key);
        assert!(!sky.default_png.is_empty());
        assert!(!portal.default_png.is_empty());
        assert!(material_matches_mode(
            WORLD_MATERIAL_ID_TRANSLUCENT_TEXTURED,
            WORLD_MATERIAL_MODE_TRANSLUCENT
        ));
    }

    #[test]
    fn entity_shadow_texture_is_a_stable_translucent_semantic_asset() {
        let shadow = texture(WORLD_MATERIAL_TEXTURE_ENTITY_SHADOW).expect("entity-shadow texture");

        assert_eq!(
            shadow.resource_location,
            "minecraft:textures/misc/shadow.png"
        );
        assert!(!shadow.default_png.is_empty());
        assert_ne!(shadow.key, WORLD_MATERIAL_TEXTURE_BEACON_BEAM);
        assert!(material_matches_mode(
            WORLD_MATERIAL_ID_TRANSLUCENT_TEXTURED,
            WORLD_MATERIAL_MODE_TRANSLUCENT
        ));
    }
}
