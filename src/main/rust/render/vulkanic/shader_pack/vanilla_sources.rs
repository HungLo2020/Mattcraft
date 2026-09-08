//! Explicit resource-namespace identity for copied vanilla shader snapshots.
use super::source::ShaderPackSource;
use crate::render::vulkanic::error::{GalError, GalResult};

pub(super) const FORMAT_PATH: &str = "mattmc/vanilla-shader-snapshot.properties";
pub(super) const FORMAT_V1: &str = "namespace-qualified=1\n";

pub(super) fn qualified(source: &ShaderPackSource) -> GalResult<bool> {
    match source.get(FORMAT_PATH) {
        None => Ok(false),
        Some(FORMAT_V1) => Ok(true),
        Some(_) => Err(GalError::unsupported_feature("unknown copied vanilla shader snapshot format")),
    }
}

pub(super) fn key(namespace: &str, path: &str) -> GalResult<String> {
    if namespace.is_empty() || matches!(namespace, "." | "..")
        || !namespace.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-".contains(&c))
    {
        return Err(GalError::invalid_argument("invalid copied shader namespace"));
    }
    Ok(format!("assets/{namespace}/shaders/{path}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{source::ShaderSourceFile, vanilla_imports, vanilla_post_effect_contract::VanillaPostEffectContract};

    #[test]
    fn qualified_imports_keep_same_named_resources_distinct_and_quoted_root_semantics() {
        let source = ShaderPackSource::new("qualified", 1, vec![
            ShaderSourceFile::new(FORMAT_PATH, FORMAT_V1),
            ShaderSourceFile::new("assets/minecraft/shaders/include/shared.glsl", "float minecraftValue;\n"),
            ShaderSourceFile::new("assets/example/shaders/include/shared.glsl", "#moj_import <minecraft:shared.glsl>\n#moj_import \"local.glsl\"\nfloat exampleValue;\n"),
            ShaderSourceFile::new("assets/minecraft/shaders/post/local.glsl", "float originalRoot;\n"),
            ShaderSourceFile::new("assets/example/shaders/include/local.glsl", "float incorrectRoot;\n"),
        ]).unwrap();
        let expanded = vanilla_imports::expand(&source, "minecraft", "post/test.fsh",
            "#version 330\n#moj_import <minecraft:shared.glsl>\n#moj_import <example:shared.glsl>\n").unwrap();
        assert_eq!(1, expanded.matches("float minecraftValue;").count());
        assert_eq!(1, expanded.matches("float exampleValue;").count());
        assert!(expanded.contains("float originalRoot;"));
        assert!(!expanded.contains("incorrectRoot"));
    }

    #[test]
    fn qualified_stages_cross_namespaces_without_flat_or_bundled_substitution() {
        let contract = VanillaPostEffectContract::parse("minecraft:fixture",
            br#"{"targets":{},"passes":[{"vertex_shader":"minecraft:core/screenquad","fragment_shader":"example:post/probe","inputs":[{"sampler_name":"In","target":"minecraft:main"}],"output":"minecraft:main"}]}"#).unwrap();
        let files = vec![
            ShaderSourceFile::new(FORMAT_PATH, FORMAT_V1),
            ShaderSourceFile::new("assets/minecraft/shaders/core/screenquad.vsh", "#version 330\nvoid main(){}\n"),
            ShaderSourceFile::new("assets/example/shaders/post/probe.fsh", "#version 330\n#moj_import <example:value.glsl>\n#moj_import \"local.glsl\"\n"),
            ShaderSourceFile::new("assets/example/shaders/post/local.glsl", "float localValue;\n"),
            ShaderSourceFile::new("assets/example/shaders/include/value.glsl", "float selectedValue;\n"),
            ShaderSourceFile::new("post/probe.fsh", "float wrongFlatSource;\n"),
            ShaderSourceFile::new("program/post/probe.fsh", "float wrongLegacyProgramDirectory;\n"),
        ];
        let source = ShaderPackSource::new("qualified", 1, files.clone()).unwrap();
        let stages = contract.expanded_shader_sources_from_source(&source).unwrap();
        let fragment = std::str::from_utf8(&stages[0].fragment_shader).unwrap();
        assert!(fragment.contains("selectedValue"));
        assert!(fragment.contains("localValue"));
        assert!(!fragment.contains("wrongFlatSource"));
        for missing in ["assets/example/shaders/post/probe.fsh", "assets/minecraft/shaders/core/screenquad.vsh",
                        "assets/example/shaders/include/value.glsl"] {
            let source = ShaderPackSource::new("missing", 1, files.iter().filter(|file| file.path != missing).cloned().collect()).unwrap();
            assert!(contract.expanded_shader_sources_from_source(&source).is_err(), "{missing}");
        }
        let source = ShaderPackSource::new("unknown", 1, vec![ShaderSourceFile::new(FORMAT_PATH, "namespace-qualified=2\n")]).unwrap();
        assert!(qualified(&source).is_err());
    }
}
