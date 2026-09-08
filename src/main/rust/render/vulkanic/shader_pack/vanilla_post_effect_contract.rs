//! Backend-neutral contracts for vanilla fullscreen post-effect graphs.
//!
//! This layer only owns copied JSON semantics. It deliberately does not create
//! GAL resources or admit a rendering route until a Rust fullscreen executor
//! supplies the required shader and target implementations.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::source::ShaderPackSource;
use crate::render::vulkanic::error::{GalError, GalResult};

const MAX_TARGETS: usize = 64;
const MAX_PASSES: usize = 64;
const MAX_INPUTS_PER_PASS: usize = 32;
const MAX_UNIFORM_BLOCKS_PER_PASS: usize = 16;
const MAX_UNIFORMS_PER_BLOCK: usize = 64;
const MAX_UNIFORM_NAME_LENGTH: usize = 128;
const MAX_TEXTURE_INPUT_DIMENSION: u32 = 16_384;
const MAIN_TARGET: &str = "minecraft:main";

/// Targets supplied by the Rust frame coordinator rather than allocated by the
/// post-effect graph itself. They remain named semantic attachments; no Java
/// post-chain handle crosses this contract.
const EXTERNAL_TARGETS: &[&str] = &[
    "minecraft:main",
    "minecraft:translucent",
    "minecraft:item_entity",
    "minecraft:particles",
    "minecraft:clouds",
    "minecraft:weather",
    "minecraft:entity_outline",
];

fn is_external_target(target: &str) -> bool {
    EXTERNAL_TARGETS.contains(&target)
}

#[derive(Clone, Debug, PartialEq)]
pub struct VanillaPostEffectPass {
    pub vertex_shader: String,
    pub fragment_shader: String,
    pub inputs: Vec<VanillaPostEffectInput>,
    pub output: String,
    pub uniform_blocks: BTreeSet<String>,
    pub uniform_values: BTreeMap<String, Vec<VanillaPostEffectUniform>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VanillaPostEffectUniform {
    pub name: String,
    pub value_type: String,
    pub values: Vec<f32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VanillaPostEffectInput {
    pub sampler_name: String,
    /// A target input is represented by `target`; a resource-pack texture
    /// input is represented by `texture_path` and is deliberately kept
    /// separate so it can never be mistaken for an external attachment.
    pub target: String,
    pub texture_path: Option<String>,
    pub texture_width: Option<u32>,
    pub texture_height: Option<u32>,
    pub bilinear: bool,
    pub use_depth_buffer: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VanillaPostEffectContract {
    pub effect_name: String,
    pub targets: BTreeSet<String>,
    pub passes: Vec<VanillaPostEffectPass>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VanillaPostEffectExecutionPlan {
    pub effect_name: String,
    pub intermediate_targets: Vec<String>,
    pub ordered_passes: Vec<VanillaPostEffectPass>,
}

impl VanillaPostEffectExecutionPlan {
    /// External attachment roles which the frame coordinator must bind before
    /// admitting this graph. Intermediate targets remain executor-private;
    /// this inventory prevents a syntactically valid graph from silently
    /// omitting a required layer such as particles, clouds, or weather.
    pub fn required_external_targets(&self) -> BTreeSet<String> {
        let mut required = BTreeSet::new();
        for pass in &self.ordered_passes {
            if is_external_target(&pass.output) {
                required.insert(pass.output.clone());
            }
            for input in &pass.inputs {
                if is_external_target(&input.target) {
                    required.insert(input.target.clone());
                }
            }
        }
        required
    }

    /// Validates the coordinator's copied attachment-role inventory before any
    /// GAL resource binding or command lowering occurs.  Missing roles are an
    /// explicit admission failure; the caller must not replace them with a
    /// Java post-chain target or a borrowed backend view.
    pub fn validate_external_targets(&self, provided: &BTreeSet<String>) -> GalResult<()> {
        let required = self.required_external_targets();
        let missing = required.difference(provided).cloned().collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(GalError::unsupported_feature(format!(
                "vanilla post effect {} is unavailable until Rust owns external targets: {}",
                self.effect_name,
                missing.join(", ")
            )));
        }
        let extra = provided.difference(&required).cloned().collect::<Vec<_>>();
        if !extra.is_empty() {
            return Err(GalError::invalid_argument(format!(
                "vanilla post effect {} received undeclared external targets: {}",
                self.effect_name,
                extra.join(", ")
            )));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VanillaPostEffectShaderSource {
    pub vertex_shader: Vec<u8>,
    pub fragment_shader: Vec<u8>,
}

impl VanillaPostEffectContract {
    pub fn parse(effect_name: impl Into<String>, bytes: &[u8]) -> GalResult<Self> {
        let effect_name = effect_name.into();
        let root: Value = serde_json::from_slice(bytes).map_err(|error| {
            GalError::invalid_argument(format!(
                "vanilla post effect {effect_name} is malformed JSON: {error}"
            ))
        })?;
        let object = root.as_object().ok_or_else(|| {
            GalError::invalid_argument(format!(
                "vanilla post effect {effect_name} root must be an object"
            ))
        })?;

        let mut targets = BTreeSet::new();
        if let Some(value) = object.get("targets") {
            let target_object = value.as_object().ok_or_else(|| {
                GalError::invalid_argument(format!(
                    "vanilla post effect {effect_name} targets must be an object"
                ))
            })?;
            if target_object.len() > MAX_TARGETS {
                return Err(GalError::invalid_argument(format!(
                    "vanilla post effect {effect_name} exceeds target budget {MAX_TARGETS}"
                )));
            }
            targets.extend(target_object.keys().cloned());
        }

        let pass_values = object
            .get("passes")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                GalError::invalid_argument(format!(
                    "vanilla post effect {effect_name} has no pass array"
                ))
            })?;
        if pass_values.is_empty() || pass_values.len() > MAX_PASSES {
            return Err(GalError::invalid_argument(format!(
                "vanilla post effect {effect_name} pass count must be 1..={MAX_PASSES}"
            )));
        }

        let mut passes = Vec::with_capacity(pass_values.len());
        let mut produced_targets = BTreeSet::from([MAIN_TARGET.to_string()]);
        for (index, value) in pass_values.iter().enumerate() {
            let pass = value.as_object().ok_or_else(|| {
                GalError::invalid_argument(format!(
                    "vanilla post effect {effect_name} pass {index} must be an object"
                ))
            })?;
            let shader = |key: &str| -> GalResult<String> {
                pass.get(key)
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .ok_or_else(|| {
                        GalError::invalid_argument(format!(
                            "vanilla post effect {effect_name} pass {index} lacks string {key}"
                        ))
                    })
            };
            let output = pass
                .get("output")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| {
                    GalError::invalid_argument(format!(
                        "vanilla post effect {effect_name} pass {index} lacks output"
                    ))
                })?;
            if !is_external_target(&output) && !targets.contains(&output) {
                return Err(GalError::invalid_argument(format!(
                    "vanilla post effect {effect_name} pass {index} writes undeclared target {output}"
                )));
            }
            let mut inputs = Vec::new();
            if let Some(input_values) = pass.get("inputs").and_then(Value::as_array) {
                if input_values.len() > MAX_INPUTS_PER_PASS {
                    return Err(GalError::invalid_argument(format!(
                        "vanilla post effect {effect_name} pass {index} exceeds input budget {MAX_INPUTS_PER_PASS}"
                    )));
                }
                for input in input_values {
                    let input = input.as_object().ok_or_else(|| {
                        GalError::invalid_argument(format!(
                            "vanilla post effect {effect_name} pass {index} input must be an object"
                        ))
                    })?;
                    let sampler_name = input.get("sampler_name").and_then(Value::as_str).ok_or_else(|| {
                        GalError::invalid_argument(format!(
                            "vanilla post effect {effect_name} pass {index} input lacks sampler_name"
                        ))
                    })?;
                    if sampler_name.is_empty() || sampler_name.len() > 128 {
                        return Err(GalError::invalid_argument(format!(
                            "vanilla post effect {effect_name} pass {index} input sampler_name has invalid length"
                        )));
                    }
                    let (target, texture_path, texture_width, texture_height) = if let Some(
                        target,
                    ) =
                        input.get("target").and_then(Value::as_str)
                    {
                        if !is_external_target(target) && !targets.contains(target) {
                            return Err(GalError::invalid_argument(format!(
                                    "vanilla post effect {effect_name} pass {index} reads undeclared target {target}"
                                )));
                        }
                        if !is_external_target(target) && !produced_targets.contains(target) {
                            return Err(GalError::invalid_argument(format!(
                                    "vanilla post effect {effect_name} pass {index} reads target {target} before it is produced"
                                )));
                        }
                        (target.to_owned(), None, None, None)
                    } else {
                        let location = input.get("location").and_then(Value::as_str).ok_or_else(|| {
                                GalError::invalid_argument(format!(
                                    "vanilla post effect {effect_name} pass {index} input lacks target or location"
                                ))
                            })?;
                        if location.is_empty() || location.len() > 512 || location.contains('\0') {
                            return Err(GalError::invalid_argument(format!(
                                    "vanilla post effect {effect_name} pass {index} texture location is invalid"
                                )));
                        }
                        let width = input.get("width").and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok()).ok_or_else(|| {
                                GalError::invalid_argument(format!(
                                    "vanilla post effect {effect_name} pass {index} texture width is invalid"
                                ))
                            })?;
                        let height = input.get("height").and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok()).ok_or_else(|| {
                                GalError::invalid_argument(format!(
                                    "vanilla post effect {effect_name} pass {index} texture height is invalid"
                                ))
                            })?;
                        if width == 0
                            || height == 0
                            || width > MAX_TEXTURE_INPUT_DIMENSION
                            || height > MAX_TEXTURE_INPUT_DIMENSION
                        {
                            return Err(GalError::invalid_argument(format!(
                                    "vanilla post effect {effect_name} pass {index} texture dimensions exceed {MAX_TEXTURE_INPUT_DIMENSION}"
                                )));
                        }
                        (
                            String::new(),
                            Some(location.to_owned()),
                            Some(width),
                            Some(height),
                        )
                    };
                    let bilinear = input
                        .get("bilinear")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    let use_depth_buffer = input
                        .get("use_depth_buffer")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    inputs.push(VanillaPostEffectInput {
                        sampler_name: sampler_name.to_owned(),
                        target,
                        texture_path,
                        texture_width,
                        texture_height,
                        bilinear,
                        use_depth_buffer,
                    });
                }
            }
            let mut uniform_blocks = BTreeSet::new();
            let mut uniform_values = BTreeMap::new();
            if let Some(uniforms) = pass.get("uniforms").and_then(Value::as_object) {
                if uniforms.len() > MAX_UNIFORM_BLOCKS_PER_PASS {
                    return Err(GalError::invalid_argument(format!(
                        "vanilla post effect {effect_name} pass {index} exceeds uniform block budget {MAX_UNIFORM_BLOCKS_PER_PASS}"
                    )));
                }
                uniform_blocks.extend(uniforms.keys().cloned());
                for (block_name, entries) in uniforms {
                    let entries = entries.as_array().ok_or_else(|| {
                        GalError::invalid_argument(format!(
                            "vanilla post effect {effect_name} pass {index} uniform block {block_name} must be an array"
                        ))
                    })?;
                    if entries.len() > MAX_UNIFORMS_PER_BLOCK {
                        return Err(GalError::invalid_argument(format!(
                            "vanilla post effect {effect_name} pass {index} uniform block {block_name} exceeds entry budget {MAX_UNIFORMS_PER_BLOCK}"
                        )));
                    }
                    let mut parsed = Vec::with_capacity(entries.len());
                    let mut seen_uniform_names = BTreeSet::new();
                    for entry in entries {
                        let entry = entry.as_object().ok_or_else(|| {
                            GalError::invalid_argument(format!(
                                "vanilla post effect {effect_name} pass {index} uniform block {block_name} entry must be an object"
                            ))
                        })?;
                        let name = entry.get("name").and_then(Value::as_str).ok_or_else(|| {
                            GalError::invalid_argument(format!(
                                "vanilla post effect {effect_name} pass {index} uniform block {block_name} entry lacks name"
                            ))
                        })?;
                        let value_type = entry.get("type").and_then(Value::as_str).ok_or_else(|| {
                            GalError::invalid_argument(format!(
                                "vanilla post effect {effect_name} pass {index} uniform {name} lacks type"
                            ))
                        })?;
                        if name.is_empty() || name.len() > MAX_UNIFORM_NAME_LENGTH {
                            return Err(GalError::invalid_argument(format!(
                                "vanilla post effect {effect_name} pass {index} uniform name has invalid length"
                            )));
                        }
                        if !seen_uniform_names.insert(name.to_owned()) {
                            return Err(GalError::invalid_argument(format!(
                                "vanilla post effect {effect_name} pass {index} uniform block {block_name} repeats uniform {name}"
                            )));
                        }
                        let expected_len = match value_type {
                            "float" | "int" => 1,
                            "vec2" | "ivec2" => 2,
                            "vec3" | "ivec3" => 3,
                            "vec4" | "ivec4" => 4,
                            "matrix4x4" => 16,
                            _ => {
                                return Err(GalError::unsupported_feature(format!(
                                    "vanilla post effect {effect_name} uses unsupported uniform type {value_type}"
                                )))
                            }
                        };
                        let values = entry.get("value").map_or_else(
                            || Ok(Vec::new()),
                            |value| {
                                if expected_len == 1 {
                                    value.as_f64().map(|value| vec![value as f32]).ok_or_else(|| {
                                        GalError::invalid_argument(format!(
                                            "vanilla post effect {effect_name} uniform {name} must contain a finite float"
                                        ))
                                    })
                                } else {
                                    value
                                        .as_array()
                                        .ok_or_else(|| GalError::invalid_argument(format!(
                                            "vanilla post effect {effect_name} uniform {name} must contain {expected_len} numeric values"
                                        )))?
                                        .iter()
                                        .map(|value| value.as_f64().map(|value| value as f32).ok_or_else(|| GalError::invalid_argument(format!(
                                            "vanilla post effect {effect_name} uniform {name} contains a non-numeric value"
                                        ))))
                                        .collect()
                                }
                            },
                        )?;
                        if values.len() != expected_len
                            || values.iter().any(|value| !value.is_finite())
                        {
                            return Err(GalError::invalid_argument(format!(
                                "vanilla post effect {effect_name} uniform {name} has the wrong or non-finite value count"
                            )));
                        }
                        if value_type == "int" || matches!(value_type, "ivec2" | "ivec3" | "ivec4")
                        {
                            if values.iter().any(|value| {
                                value.fract() != 0.0
                                    || *value < i32::MIN as f32
                                    || *value > i32::MAX as f32
                            }) {
                                return Err(GalError::invalid_argument(format!(
                                    "vanilla post effect {effect_name} integer uniform {name} must contain finite 32-bit integral values"
                                )));
                            }
                        }
                        parsed.push(VanillaPostEffectUniform {
                            name: name.to_owned(),
                            value_type: value_type.to_owned(),
                            values,
                        });
                    }
                    uniform_values.insert(block_name.clone(), parsed);
                }
            }
            for required in
                required_uniform_blocks(&shader("vertex_shader")?, &shader("fragment_shader")?)
            {
                if !uniform_blocks.contains(required) {
                    return Err(GalError::invalid_argument(format!(
                        "vanilla post effect {effect_name} pass {index} lacks required uniform block {required}"
                    )));
                }
            }
            let writes_intermediate = output != MAIN_TARGET;
            if writes_intermediate {
                produced_targets.insert(output.clone());
            }
            passes.push(VanillaPostEffectPass {
                vertex_shader: shader("vertex_shader")?,
                fragment_shader: shader("fragment_shader")?,
                inputs,
                output,
                uniform_blocks,
                uniform_values,
            });
        }
        if passes
            .last()
            .is_none_or(|pass| !is_external_target(&pass.output))
        {
            return Err(GalError::invalid_argument(format!(
                "vanilla post effect {effect_name} must finish by writing a Rust-owned external target"
            )));
        }
        Ok(Self {
            effect_name,
            targets,
            passes,
        })
    }

    pub fn target_names(&self) -> BTreeMap<String, usize> {
        self.targets
            .iter()
            .enumerate()
            .map(|(index, name)| (name.clone(), index))
            .collect()
    }

    /// Produces the immutable pass plan consumed by a future Rust fullscreen
    /// executor. No GAL handles or Java post-chain state cross this boundary.
    pub fn execution_plan(&self) -> VanillaPostEffectExecutionPlan {
        VanillaPostEffectExecutionPlan {
            effect_name: self.effect_name.clone(),
            intermediate_targets: self.targets.iter().cloned().collect(),
            ordered_passes: self.passes.clone(),
        }
    }

    /// Resolves only bundled vanilla shader identities to copied source bytes.
    /// A missing identity is an explicit admission failure, never a Java or
    /// backend lookup fallback.
    pub fn shader_sources(&self) -> GalResult<Vec<VanillaPostEffectShaderSource>> {
        self.passes
            .iter()
            .map(|pass| {
                let vertex_shader =
                    bundled_shader_source(&pass.vertex_shader).ok_or_else(|| {
                        GalError::unsupported_feature(format!(
                            "vanilla post effect {} lacks owned vertex shader {}",
                            self.effect_name, pass.vertex_shader
                        ))
                    })?;
                let fragment_shader =
                    bundled_shader_source(&pass.fragment_shader).ok_or_else(|| {
                        GalError::unsupported_feature(format!(
                            "vanilla post effect {} lacks owned fragment shader {}",
                            self.effect_name, pass.fragment_shader
                        ))
                    })?;
                Ok(VanillaPostEffectShaderSource {
                    vertex_shader: vertex_shader.to_vec(),
                    fragment_shader: fragment_shader.to_vec(),
                })
            })
            .collect()
    }

    /// Resolves post-effect shader identities against a copied Rust-owned
    /// shader-pack source snapshot, falling back to bundled vanilla sources
    /// only when that snapshot does not provide the stage. This keeps custom
    /// resource-pack/post-effect code on the semantic source boundary: no
    /// Java ResourceProvider, Iris program, or backend handle is consulted.
    pub fn shader_sources_from_source(
        &self,
        source: &ShaderPackSource,
    ) -> GalResult<Vec<VanillaPostEffectShaderSource>> {
        self.passes
            .iter()
            .map(|pass| {
                let vertex_shader = resolve_shader_source(source, &pass.vertex_shader, "vsh")?;
                let fragment_shader = resolve_shader_source(source, &pass.fragment_shader, "fsh")?;
                Ok(VanillaPostEffectShaderSource {
                    vertex_shader,
                    fragment_shader,
                })
            })
            .collect()
    }

    /// Resolve imports for the copied programmable graph. Bundled identity
    /// comparisons retain their raw-source contract; compilation uses this
    /// stage-local expansion over the same immutable resource generation.
    pub fn expanded_shader_sources_from_source(
        &self,
        source: &ShaderPackSource,
    ) -> GalResult<Vec<VanillaPostEffectShaderSource>> {
        self.passes.iter().map(|pass| {
            let expand = |identity: &str, extension: &str| -> GalResult<Vec<u8>> {
                let bytes = resolve_shader_source(source, identity, extension)?;
                let contents = std::str::from_utf8(&bytes)
                    .map_err(|_| GalError::invalid_argument("post-effect source is not UTF-8"))?;
                let (namespace, path) = identity.split_once(':').unwrap_or(("minecraft", identity));
                let snapshot_namespace = self.effect_name.split_once(':')
                    .map_or("minecraft", |(namespace, _)| namespace);
                if !super::vanilla_sources::qualified(source)? && namespace != snapshot_namespace {
                    return Err(GalError::unsupported_feature(
                        "cross-namespace post-effect stage requires a namespace-qualified source snapshot"));
                }
                let direct = format!("{path}.{extension}");
                let program = format!("program/{path}.{extension}");
                let resolved = if !super::vanilla_sources::qualified(source)?
                    && source.get(&direct).is_none() && source.get(&program).is_some() {
                    &program
                } else { &direct };
                super::vanilla_imports::expand(source, namespace, resolved, contents)
                    .map(String::into_bytes)
            };
            Ok(VanillaPostEffectShaderSource {
                vertex_shader: expand(&pass.vertex_shader, "vsh")?,
                fragment_shader: expand(&pass.fragment_shader, "fsh")?,
            })
        }).collect()
    }
}

fn resolve_shader_source(
    source: &ShaderPackSource,
    identity: &str,
    extension: &str,
) -> GalResult<Vec<u8>> {
    if super::vanilla_sources::qualified(source)? {
        let (namespace, path) = identity.split_once(':').unwrap_or(("minecraft", identity));
        let key = super::vanilla_sources::key(namespace, &format!("{path}.{extension}"))?;
        return source.get(&key).map(|contents| contents.as_bytes().to_vec())
            .ok_or_else(|| GalError::unsupported_feature(format!(
                "copied vanilla shader '{identity}.{extension}' is missing from its qualified source snapshot")));
    }
    let path = identity.split_once(':').map_or(identity, |(_, path)| path);
    let candidates = [
        format!("{path}.{extension}"),
        format!("program/{path}.{extension}"),
    ];
    for candidate in candidates {
        if let Some(contents) = source.get(&candidate) {
            return Ok(contents.as_bytes().to_vec());
        }
    }
    bundled_shader_source(identity)
        .map(|contents| contents.to_vec())
        .ok_or_else(|| {
            GalError::unsupported_feature(format!(
                "vanilla post effect {} lacks Rust-owned {extension} shader source {identity}",
                source.name()
            ))
        })
}

fn bundled_shader_source(identity: &str) -> Option<&'static [u8]> {
    Some(match identity {
        "minecraft:core/screenquad" => {
            include_bytes!("../../../../resources/assets/minecraft/shaders/core/screenquad.vsh")
        }
        "minecraft:post/rotscale" => {
            include_bytes!("../../../../resources/assets/minecraft/shaders/post/rotscale.vsh")
        }
        "minecraft:post/invert" => {
            include_bytes!("../../../../resources/assets/minecraft/shaders/post/invert.fsh")
        }
        "minecraft:post/box_blur" => {
            include_bytes!("../../../../resources/assets/minecraft/shaders/post/box_blur.fsh")
        }
        "minecraft:post/entity_sobel" => {
            include_bytes!("../../../../resources/assets/minecraft/shaders/post/entity_sobel.fsh")
        }
        "minecraft:post/entity_outline_box_blur" => include_bytes!(
            "../../../../resources/assets/minecraft/shaders/post/entity_outline_box_blur.fsh"
        ),
        "minecraft:post/spiderclip" => {
            include_bytes!("../../../../resources/assets/minecraft/shaders/post/spiderclip.fsh")
        }
        "minecraft:post/color_convolve" => {
            include_bytes!("../../../../resources/assets/minecraft/shaders/post/color_convolve.fsh")
        }
        "minecraft:post/bits" => {
            include_bytes!("../../../../resources/assets/minecraft/shaders/post/bits.fsh")
        }
        "minecraft:post/blit" => {
            include_bytes!("../../../../resources/assets/minecraft/shaders/post/blit.fsh")
        }
        "minecraft:post/transparency" => {
            include_bytes!("../../../../resources/assets/minecraft/shaders/post/transparency.fsh")
        }
        _ => return None,
    })
}

fn required_uniform_blocks(vertex_shader: &str, fragment_shader: &str) -> Vec<&'static str> {
    let mut required = Vec::new();
    match vertex_shader {
        "minecraft:post/rotscale" => required.push("RotScaleConfig"),
        _ => {}
    }
    match fragment_shader {
        "minecraft:post/invert" => required.push("InvertConfig"),
        "minecraft:post/box_blur" => required.push("BlurConfig"),
        "minecraft:post/entity_outline_box_blur" => required.push("BlurConfig"),
        "minecraft:post/spiderclip" => required.push("SpiderConfig"),
        "minecraft:post/color_convolve" => required.push("ColorConfig"),
        "minecraft:post/bits" => required.push("BitsConfig"),
        "minecraft:post/blit" => required.push("BlitConfig"),
        _ => {}
    }
    required
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::VanillaPostEffectContract;

    #[test]
    fn bundled_effects_are_explicit_graphs() {
        let invert = VanillaPostEffectContract::parse(
            "invert",
            include_bytes!("../../../../resources/assets/minecraft/post_effect/invert.json"),
        )
        .unwrap();
        assert_eq!(2, invert.passes.len());
        assert_eq!("minecraft:post/invert", invert.passes[0].fragment_shader);
        assert_eq!("minecraft:main", invert.passes[1].output);

        let spider = VanillaPostEffectContract::parse(
            "spider",
            include_bytes!("../../../../resources/assets/minecraft/post_effect/spider.json"),
        )
        .unwrap();
        assert!(spider.passes.len() > 4);
        assert!(spider.targets.contains("large_blur"));
        assert!(spider
            .passes
            .iter()
            .any(|pass| pass.fragment_shader == "minecraft:post/spiderclip"));
        let plan = spider.execution_plan();
        assert_eq!(spider.passes, plan.ordered_passes);
        assert!(plan
            .intermediate_targets
            .contains(&"large_blur".to_string()));
        let sources = spider.shader_sources().unwrap();
        assert_eq!(spider.passes.len(), sources.len());
        assert!(sources
            .iter()
            .all(|source| !source.vertex_shader.is_empty() && !source.fragment_shader.is_empty()));

        let entity_outline = VanillaPostEffectContract::parse(
            "entity_outline",
            include_bytes!(
                "../../../../resources/assets/minecraft/post_effect/entity_outline.json"
            ),
        )
        .unwrap();
        assert_eq!(
            "minecraft:entity_outline",
            entity_outline.passes.last().unwrap().output
        );
        assert_eq!(
            entity_outline.passes.len(),
            entity_outline.shader_sources().unwrap().len()
        );

        let transparency = VanillaPostEffectContract::parse(
            "transparency",
            include_bytes!("../../../../resources/assets/minecraft/post_effect/transparency.json"),
        )
        .unwrap();
        assert_eq!(
            transparency.passes.len(),
            transparency.shader_sources().unwrap().len()
        );
        assert_eq!(
            transparency.execution_plan().required_external_targets(),
            BTreeSet::from([
                "minecraft:clouds".to_owned(),
                "minecraft:item_entity".to_owned(),
                "minecraft:main".to_owned(),
                "minecraft:particles".to_owned(),
                "minecraft:translucent".to_owned(),
                "minecraft:weather".to_owned(),
            ])
        );
        let mut incomplete = BTreeSet::from([
            "minecraft:main".to_owned(),
            "minecraft:translucent".to_owned(),
        ]);
        let error = transparency
            .execution_plan()
            .validate_external_targets(&incomplete)
            .unwrap_err();
        assert!(error.to_string().contains("minecraft:clouds"));
        incomplete.extend([
            "minecraft:clouds".to_owned(),
            "minecraft:item_entity".to_owned(),
            "minecraft:particles".to_owned(),
            "minecraft:weather".to_owned(),
        ]);
        transparency
            .execution_plan()
            .validate_external_targets(&incomplete)
            .unwrap();
        incomplete.insert("minecraft:undeclared".to_owned());
        let extra_error = transparency
            .execution_plan()
            .validate_external_targets(&incomplete)
            .unwrap_err();
        assert!(extra_error.to_string().contains("minecraft:undeclared"));

        for (name, bytes) in [
            (
                "blur",
                include_bytes!("../../../../resources/assets/minecraft/post_effect/blur.json")
                    .as_slice(),
            ),
            (
                "creeper",
                include_bytes!("../../../../resources/assets/minecraft/post_effect/creeper.json")
                    .as_slice(),
            ),
        ] {
            let effect = VanillaPostEffectContract::parse(name, bytes).unwrap();
            assert_eq!(effect.passes.len(), effect.shader_sources().unwrap().len());
        }
    }

    #[test]
    fn undeclared_target_is_rejected_before_resource_creation() {
        let json = br#"{
            "targets": {},
            "passes": [{
                "vertex_shader": "minecraft:core/screenquad",
                "fragment_shader": "minecraft:post/blit",
                "inputs": [{"sampler_name": "In", "target": "missing"}],
                "output": "minecraft:main"
            }]
        }"#;
        let error = VanillaPostEffectContract::parse("bad", json).unwrap_err();
        assert!(error.to_string().contains("undeclared target missing"));
    }

    #[test]
    fn forward_intermediate_reads_are_rejected() {
        let json = br#"{
            "targets": {"later": {}},
            "passes": [
                {"vertex_shader":"v", "fragment_shader":"f", "inputs":[{"sampler_name":"In", "target":"later"}], "output":"minecraft:main"}
            ]
        }"#;
        let error = VanillaPostEffectContract::parse("forward", json).unwrap_err();
        assert!(error.to_string().contains("before it is produced"));
    }

    #[test]
    fn required_shader_uniform_abi_is_rejected_when_missing() {
        let json = br#"{
            "targets": {},
            "passes": [{
                "vertex_shader":"minecraft:core/screenquad",
                "fragment_shader":"minecraft:post/invert",
                "inputs":[{"sampler_name":"In", "target":"minecraft:main"}],
                "output":"minecraft:main",
                "uniforms": {}
            }]
        }"#;
        let error = VanillaPostEffectContract::parse("missing-uniform", json).unwrap_err();
        assert!(error.to_string().contains("InvertConfig"));
    }

    #[test]
    fn duplicate_uniform_names_are_rejected_before_execution() {
        let json = br#"{
            "targets": {},
            "passes": [{
                "vertex_shader":"minecraft:core/screenquad",
                "fragment_shader":"minecraft:post/invert",
                "inputs":[{"sampler_name":"In", "target":"minecraft:main"}],
                "output":"minecraft:main",
                "uniforms":{"InvertConfig":[
                    {"name":"InverseAmount","type":"float","value":0.5},
                    {"name":"InverseAmount","type":"float","value":0.8}
                ]}
            }]
        }"#;
        let error = VanillaPostEffectContract::parse("duplicate", json).unwrap_err();
        assert!(error.to_string().contains("repeats uniform InverseAmount"));
    }

    #[test]
    fn integer_uniform_vectors_are_copied_with_bounded_integral_values() {
        let json = br#"{
            "targets": {},
            "passes": [{
                "vertex_shader":"v",
                "fragment_shader":"f",
                "inputs":[{"sampler_name":"In", "target":"minecraft:main"}],
                "output":"minecraft:main",
                "uniforms":{"Config":[
                    {"name":"Mode","type":"int","value":3},
                    {"name":"Mask","type":"ivec3","value":[1,2,4]}
                ]}
            }]
        }"#;
        let contract = VanillaPostEffectContract::parse("integer-uniforms", json).unwrap();
        let values = &contract.passes[0].uniform_values["Config"];
        assert_eq!(values[0].value_type, "int");
        assert_eq!(values[1].values, vec![1.0, 2.0, 4.0]);
    }

    #[test]
    fn fractional_integer_uniforms_are_rejected() {
        let json = br#"{
            "targets": {},
            "passes": [{
                "vertex_shader":"v", "fragment_shader":"f",
                "inputs":[{"sampler_name":"In", "target":"minecraft:main"}],
                "output":"minecraft:main",
                "uniforms":{"Config":[{"name":"Mode","type":"int","value":1.5}]}
            }]
        }"#;
        let error = VanillaPostEffectContract::parse("fractional-integer", json).unwrap_err();
        assert!(error.to_string().contains("32-bit integral values"));
    }

    #[test]
    fn matrix4x4_uniforms_require_sixteen_finite_values() {
        let json = br#"{
            "targets": {},
            "passes": [{
                "vertex_shader":"v", "fragment_shader":"f",
                "inputs":[{"sampler_name":"In", "target":"minecraft:main"}],
                "output":"minecraft:main",
                "uniforms":{"Transform":[{"name":"Model","type":"matrix4x4","value":[1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1]}]}
            }]
        }"#;
        let contract = VanillaPostEffectContract::parse("matrix-uniform", json).unwrap();
        let uniform = &contract.passes[0].uniform_values["Transform"][0];
        assert_eq!(uniform.value_type, "matrix4x4");
        assert_eq!(uniform.values.len(), 16);
    }

    #[test]
    fn input_sampling_semantics_are_retained_for_multi_target_effects() {
        let transparency = VanillaPostEffectContract::parse(
            "transparency",
            include_bytes!("../../../../resources/assets/minecraft/post_effect/transparency.json"),
        )
        .unwrap();
        let depth_input = transparency.passes[0]
            .inputs
            .iter()
            .find(|input| input.sampler_name == "MainDepth")
            .unwrap();
        assert_eq!("minecraft:main", depth_input.target);
        assert!(depth_input.use_depth_buffer);
        assert!(!depth_input.bilinear);

        let blur = VanillaPostEffectContract::parse(
            "blur",
            include_bytes!("../../../../resources/assets/minecraft/post_effect/blur.json"),
        )
        .unwrap();
        let blur_config = blur.passes[0].uniform_values.get("BlurConfig").unwrap();
        assert_eq!(blur_config[0].name, "BlurDir");
        assert_eq!(blur_config[0].values, vec![1.0, 0.0]);
        assert_eq!(blur_config[1].value_type, "float");
        assert_eq!(blur_config[1].values, vec![0.0]);
    }

    #[test]
    fn texture_inputs_are_copied_as_explicit_unadmitted_assets() {
        let json = br#"{
            "targets": {},
            "passes": [{
                "vertex_shader":"v",
                "fragment_shader":"f",
                "inputs":[{"sampler_name":"Mask","location":"minecraft:textures/effect/mask.png","width":64,"height":32,"bilinear":true}],
                "output":"minecraft:main"
            }]
        }"#;
        let contract = VanillaPostEffectContract::parse("texture-input", json).unwrap();
        let input = &contract.passes[0].inputs[0];
        assert!(input.target.is_empty());
        assert_eq!(
            input.texture_path.as_deref(),
            Some("minecraft:textures/effect/mask.png")
        );
        assert_eq!(input.texture_width, Some(64));
        assert_eq!(input.texture_height, Some(32));
        assert_eq!(
            contract.execution_plan().required_external_targets(),
            BTreeSet::from(["minecraft:main".to_owned()])
        );
    }

    #[test]
    fn copied_shader_pack_sources_override_bundled_post_effect_stages() {
        let contract = VanillaPostEffectContract::parse(
            "custom_effect",
            br#"{
                "targets": {},
                "passes": [{
                    "vertex_shader": "minecraft:post/custom",
                    "fragment_shader": "minecraft:post/custom",
                    "output": "minecraft:main"
                }]
            }"#,
        )
        .unwrap();
        let source = super::super::source::ShaderPackSource::new(
            "custom-pack",
            1,
            vec![
                super::super::source::ShaderSourceFile::new(
                    "post/custom.vsh",
                    "#version 450\nvoid main(){}",
                ),
                super::super::source::ShaderSourceFile::new(
                    "post/custom.fsh",
                    "#version 450\nvoid main(){}",
                ),
            ],
        )
        .unwrap();
        let stages = contract.shader_sources_from_source(&source).unwrap();
        assert_eq!(
            b"#version 450\nvoid main(){}",
            stages[0].vertex_shader.as_slice()
        );
        assert_eq!(
            b"#version 450\nvoid main(){}",
            stages[0].fragment_shader.as_slice()
        );
    }
}
