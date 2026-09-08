//! Copied engine semantics for vanilla shader Globals. No renderer objects or
//! GPU handles enter this record; Rust packs and binds its own uniform data.

use super::vanilla_post_effect_contract::VanillaPostEffectUniform;
use crate::render::vulkanic::error::{GalError, GalResult};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct EngineGlobals {
    pub screen_width: u32,
    pub screen_height: u32,
    pub glint_alpha: f32,
    pub game_ticks: i64,
    pub partial_tick: f32,
    pub menu_blur_radius: i32,
}

impl EngineGlobals {
    pub(crate) fn uniforms(self) -> GalResult<Vec<VanillaPostEffectUniform>> {
        if self.screen_width == 0
            || self.screen_height == 0
            || !self.glint_alpha.is_finite()
            || !self.partial_tick.is_finite()
            || !(0..=64).contains(&self.menu_blur_radius)
        {
            return Err(GalError::invalid_argument(
                "invalid copied engine Globals values",
            ));
        }
        // Frozen GlobalSettingsUniform uses signed Java remainder, then a
        // float conversion and addition, not world/day time or a frame clock.
        let game_time = ((self.game_ticks % 24000) as f32 + self.partial_tick) / 24000.0;
        let uniform = |name: &str, value_type: &str, values: Vec<f32>| VanillaPostEffectUniform {
            name: name.into(),
            value_type: value_type.into(),
            values,
        };
        Ok(vec![
            uniform(
                "ScreenSize",
                "vec2",
                vec![self.screen_width as f32, self.screen_height as f32],
            ),
            uniform("GlintAlpha", "float", vec![self.glint_alpha]),
            uniform("GameTime", "float", vec![game_time]),
            uniform("MenuBlurRadius", "int", vec![self.menu_blur_radius as f32]),
        ])
    }
}

/// The engine supplies this exact semantic block, not arbitrary resource-pack
/// data with a coincidentally familiar name. Incompatible layouts fail before
/// pipeline/resource creation instead of binding bytes to the wrong fields.
pub(crate) fn uses_globals(source: &str) -> GalResult<bool> {
    let compact: String = source.chars().filter(|c| !c.is_whitespace()).collect();
    let marker = "uniformGlobals";
    let Some(start) = compact.find(marker) else {
        return Ok(false);
    };
    let block = &compact[start + marker.len()..];
    let expected = "{vec2ScreenSize;floatGlintAlpha;floatGameTime;intMenuBlurRadius;};";
    if !block.starts_with(expected) || block[expected.len()..].contains(marker) {
        return Err(GalError::unsupported_feature(
            "copied shader Globals layout differs from the engine semantic contract",
        ));
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::super::vanilla_post_effect_contract::VanillaPostEffectContract;
    use super::super::vanilla_post_effect_executor::pack_uniform_blocks;
    use super::*;

    #[test]
    fn copied_globals_pack_frozen_std140_offsets_and_signed_time() {
        let globals = EngineGlobals {
            screen_width: 854,
            screen_height: 480,
            glint_alpha: 0.75,
            game_ticks: 48001,
            partial_tick: 0.5,
            menu_blur_radius: 7,
        };
        let contract = VanillaPostEffectContract::parse("minecraft:fixture", br#"{"targets":{},"passes":[{"vertex_shader":"minecraft:core/screenquad","fragment_shader":"minecraft:post/globals_fixture","inputs":[{"sampler_name":"In","target":"minecraft:main"}],"output":"minecraft:main"}]}"#).unwrap();
        let mut pass = contract.execution_plan().ordered_passes.remove(0);
        pass.uniform_values
            .insert("Globals".into(), globals.uniforms().unwrap());
        let blocks = pack_uniform_blocks(&pass).unwrap();
        let bytes = &blocks["Globals"];
        assert_eq!(32, bytes.len());
        for (offset, expected) in [(0, 854.0f32), (4, 480.0), (8, 0.75), (12, 1.5 / 24000.0)] {
            assert_eq!(expected.to_le_bytes(), bytes[offset..offset + 4]);
        }
        assert_eq!(7i32.to_le_bytes(), bytes[16..20]);
        assert_eq!(&[0; 12], &bytes[20..]);
        let negative = EngineGlobals {
            game_ticks: -1,
            partial_tick: 0.0,
            ..globals
        };
        assert_eq!(-1.0 / 24000.0, negative.uniforms().unwrap()[2].values[0]);
        let rollover = EngineGlobals {
            game_ticks: 48000,
            partial_tick: 0.0,
            ..globals
        };
        assert_eq!(0.0, rollover.uniforms().unwrap()[2].values[0]);
        assert!(EngineGlobals {
            partial_tick: f32::NAN,
            ..globals
        }
        .uniforms()
        .is_err());
        assert!(EngineGlobals {
            glint_alpha: f32::INFINITY,
            ..globals
        }
        .uniforms()
        .is_err());
        assert!(EngineGlobals {
            screen_width: 0,
            ..globals
        }
        .uniforms()
        .is_err());
        assert!(EngineGlobals {
            menu_blur_radius: -1,
            ..globals
        }
        .uniforms()
        .is_err());
    }

    #[test]
    fn copied_globals_schema_is_checked_instead_of_assumed() {
        let source =
            include_str!("../../../../resources/assets/minecraft/shaders/include/globals.glsl");
        assert!(uses_globals(source).unwrap());
        assert!(!uses_globals("void main(){}").unwrap());
        assert!(
            uses_globals(&source.replace("int MenuBlurRadius", "float MenuBlurRadius")).is_err()
        );
        assert!(uses_globals(&source.replace("float GameTime;", "")).is_err());
        assert!(uses_globals(&format!("{source}\n{source}")).is_err());
    }

    #[test]
    fn copied_stage_cannot_alias_an_unrepresented_namespace() {
        use super::super::source::{ShaderPackSource, ShaderSourceFile};
        let source = ShaderPackSource::new("namespace-fixture", 1, vec![
            ShaderSourceFile::new("post/probe.vsh", "#version 330\nvoid main(){}\n"),
            ShaderSourceFile::new("post/probe.fsh", "#version 330\nvoid main(){}\n"),
        ]).unwrap();
        let contract = VanillaPostEffectContract::parse("minecraft:fixture",
            br#"{"targets":{},"passes":[{"vertex_shader":"other:post/probe","fragment_shader":"minecraft:post/probe","inputs":[{"sampler_name":"In","target":"minecraft:main"}],"output":"minecraft:main"}]}"#).unwrap();
        assert!(contract.expanded_shader_sources_from_source(&source).unwrap_err()
            .message.contains("namespace-qualified"));
    }
}
