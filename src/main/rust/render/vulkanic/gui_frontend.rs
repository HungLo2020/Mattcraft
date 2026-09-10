use std::collections::{BTreeMap, BTreeSet};
use std::io::BufReader;

use super::commands::{
    AttachmentLoadOp, AttachmentStoreOp, CommandList, CommandOp, PassAttachment, ResourceBarrier,
    SubmissionBatch, TextureImageCopyRegion, TextureOrigin3d, TextureUsageState,
};
use super::error::{GalError, GalResult, StatusCode};
use super::gal::VulkanicGal;
use super::gui_atlas_reference::{AcceptedAtlasIncarnation, GuiAtlasReference, GuiAtlasReferences};
use super::gui_mesh_frontend::{
    geometry_fingerprint as gui_mesh_geometry_fingerprint, prepare_draws as prepare_gui_mesh_draws,
    GuiMeshBatchRequest, GuiMeshCompositeResources, GuiMeshLightingMode, GuiMeshMaterialMode,
    GuiMeshOffscreenTargetCache, GuiMeshPassResources, GuiMeshPreparedDraw, GuiMeshSharedProgram,
    GuiMeshStreamRange, GUI_MESH_COMPOSITE_UNIFORM_STRIDE,
};
use super::handles::Handle;
use super::resources::{
    AccessFlags, BackendApi, BlendMode, BufferDesc, BufferUsage, ColorFormat,
    CombinedTextureSamplerDesc, CompareOp, Extent3d, GraphicsPipelineDesc, MemoryDomain,
    PipelineLayoutDesc, PipelineStageFlags, PrimitiveTopology, QueueClass, RenderPassDesc,
    RenderTargetDesc, ResourceBinding, ResourceBindingDesc, ResourceBindingKind,
    ResourceLayoutDesc, ResourceSetDesc, SamplerAddressMode, SamplerDesc, SamplerFilter,
    ShaderCodeFormat, ShaderModuleDesc, ShaderStage, TextureDesc, TextureDimension, TextureFormat,
    TextureSubresourceRange, TextureUsage, TextureViewDesc,
};
use super::shader_pack::vanilla_post_effect_executor::VanillaPostEffectExternalTargetBindings;
use super::sync::SubmissionId;
#[cfg(test)]
#[path = "gui_item_raster_gpu_tests.rs"]
mod gui_item_raster_gpu_tests;
use super::{BufferImageCopyRegion, CommandListDesc, CullMode};

/// Maximum number of simultaneously staged dynamic GUI images. This mirrors
/// the Java semantic asset collector and keeps the Rust-owned image map
/// bounded independently of the generic FFI batch limit.
pub(crate) const GUI_MAX_RAW_IMAGES: usize = 4_096;
/// Matches Java's copied GUI-image admission bound before any Rust-owned
/// pixel buffer is retained.
pub(crate) const GUI_MAX_RAW_IMAGE_PIXELS: usize = 16 * 1024 * 1024;
/// Aggregate raw-image bytes retained by one generation. This bounds the
/// replacement map independently of the per-image pixel limit.
pub(crate) const GUI_MAX_RAW_IMAGE_BYTES_TOTAL: usize = 256 * 1024 * 1024;
/// Maximum semantic GUI viewport axis accepted by the Rust frontend.
pub(crate) const GUI_MAX_VIEWPORT_AXIS: i32 = super::SEMANTIC_MAX_VIEWPORT_AXIS;

pub const GUI_MAX_PACKED_SPRITES: usize = 256;
/// Hard cap for one semantic GUI submission. Java's coordinator enforces the
/// same ceiling, but the Rust frontend must reject direct FFI callers too.
pub(crate) const GUI_MAX_MESH_BATCHES: usize = 1_024;
pub(crate) const GUI_POST_EFFECT_INVERT_ID: u32 = 92;
pub(crate) const GUI_POST_EFFECT_CREEPER_ID: u32 = 93;
pub(crate) const GUI_POST_EFFECT_SPIDER_ID: u32 = 94;
const GUI_OPAQUE_BLIT_STRATUM: u32 = 760;
const GUI_VIGNETTE_BLIT_STRATUM: u32 = 770;
const GUI_INVERT_RECTANGLE_STRATUM: u32 = 780;
// Matches GuiRenderStratum.GUI_CROSSHAIR. The affine request keeps its
// scheduler order separately, while this semantic stratum selects the exact
// invert blend used by vanilla's CROSSHAIR pipeline.
const GUI_CROSSHAIR_INVERT_STRATUM: u32 = 200;
const GUI_PREMULTIPLIED_BLIT_STRATUM: u32 = 790;
const GUI_ADDITIVE_BLIT_STRATUM: u32 = 795;
const GUI_LEQUAL_DEPTH_BLIT_STRATUM: u32 = 805;
const GUI_UNIFORM_BYTES: usize = 96;
const GUI_PACKED_UNIFORM_BYTES: u64 = (GUI_MAX_PACKED_SPRITES * GUI_UNIFORM_BYTES) as u64;
pub(super) const MAX_CUSTOM_POST_EFFECT_PASSES: usize = 10;
const MAX_CUSTOM_POST_EFFECT_INTERMEDIATES: usize = 4;
const MAX_CUSTOM_POST_EFFECT_UNIFORM_BYTES: usize = 1024 * 1024;
const MAX_CUSTOM_POST_EFFECT_UNIFORM_GRAPH_BYTES: usize = 2 * 1024 * 1024;
/// A mesh raster owns a complete pipeline/resource-set family. Bound the
/// cross-product of image, material, lighting, and extent variants before it
/// can turn a dynamic GUI stream into unbounded GAL residency.
const GUI_MAX_MESH_RASTER_RESOURCES: usize = 4_096;
/// Immutable mesh programs are keyed only by their explicit raster contract;
/// asset-local buffers and resource sets are deliberately excluded.
const GUI_MAX_MESH_SHARED_PROGRAMS: usize = 16;
/// Composite resources are keyed by target extent/format and likewise retain
/// explicit pipelines and uniform storage until the GUI generation changes.
const GUI_MAX_MESH_COMPOSITE_RESOURCES: usize = 256;

const VERTEX_SHADER_OPENGL: &[u8] = br#"#version 330 core
struct PackedGuiQuad {
    vec4 origin_axis_u;
    vec4 axis_v_mode;
    vec4 viewport;
    vec4 clip;
    vec4 uv_region;
    vec4 color;
};
layout(std140) uniform GuiSpriteBatch {
    PackedGuiQuad sprites[256];
};
out vec2 v_uv;
out vec2 v_sprite_corner;
out vec2 v_pixel;
out vec4 v_color;
flat out vec4 v_uv_region;
flat out vec4 v_clip;
flat out float v_texture_mode;
flat out float v_clip_enabled;
const vec2 corner[6] = vec2[6](
    vec2(0.0, 0.0),
    vec2(0.0, 1.0),
    vec2(1.0, 1.0),
    vec2(1.0, 1.0),
    vec2(1.0, 0.0),
    vec2(0.0, 0.0)
);
void main() {
    int vertex = gl_VertexID;
    PackedGuiQuad sprite = sprites[gl_InstanceID];
    vec2 pixel = sprite.origin_axis_u.xy + corner[vertex].x * sprite.origin_axis_u.zw + corner[vertex].y * sprite.axis_v_mode.xy;
    // Match the explicit orthographic matrix coefficients. Dividing each
    // vertex first changes rounding at minified texture sample boundaries.
    float top_left_y = pixel.y * (-2.0 / sprite.viewport.y) + 1.0;
    float ndc_y = mix(top_left_y, -top_left_y, sprite.viewport.w);
    vec2 ndc = vec2(pixel.x * (2.0 / sprite.viewport.x) - 1.0, ndc_y);
    gl_Position = vec4(ndc, sprite.axis_v_mode.w, 1.0);
    v_uv_region = sprite.uv_region;
    v_clip = sprite.clip;
    v_clip_enabled = sprite.viewport.z;
    v_texture_mode = sprite.axis_v_mode.z;
    v_sprite_corner = corner[vertex];
    v_pixel = pixel;
    v_uv = vec2(
        sprite.uv_region.x + corner[vertex].x * sprite.uv_region.z,
        sprite.uv_region.y + corner[vertex].y * sprite.uv_region.w
    );
    v_color = sprite.color;
}
"#;

const VERTEX_SHADER_VULKAN: &[u8] = br#"#version 450
struct PackedGuiQuad {
    vec4 origin_axis_u;
    vec4 axis_v_mode;
    vec4 viewport;
    vec4 clip;
    vec4 uv_region;
    vec4 color;
};
layout(set = 0, binding = 0, std140) uniform GuiSpriteBatch {
    PackedGuiQuad sprites[256];
};
layout(location = 0) out vec2 v_uv;
layout(location = 1) out vec2 v_sprite_corner;
layout(location = 2) out vec2 v_pixel;
layout(location = 3) out vec4 v_color;
layout(location = 4) flat out vec4 v_uv_region;
layout(location = 5) flat out vec4 v_clip;
layout(location = 6) flat out float v_texture_mode;
layout(location = 7) flat out float v_clip_enabled;
const vec2 corner[6] = vec2[6](
    vec2(0.0, 0.0),
    vec2(0.0, 1.0),
    vec2(1.0, 1.0),
    vec2(1.0, 1.0),
    vec2(1.0, 0.0),
    vec2(0.0, 0.0)
);
void main() {
    int vertex = gl_VertexIndex;
    PackedGuiQuad sprite = sprites[gl_InstanceIndex];
    vec2 pixel = sprite.origin_axis_u.xy + corner[vertex].x * sprite.origin_axis_u.zw + corner[vertex].y * sprite.axis_v_mode.xy;
    // Match the explicit orthographic matrix coefficients. Dividing each
    // vertex first changes rounding at minified texture sample boundaries.
    float top_left_y = pixel.y * (-2.0 / sprite.viewport.y) + 1.0;
    float ndc_y = mix(top_left_y, -top_left_y, sprite.viewport.w);
    vec2 ndc = vec2(pixel.x * (2.0 / sprite.viewport.x) - 1.0, ndc_y);
    gl_Position = vec4(ndc, sprite.axis_v_mode.w, 1.0);
    v_uv_region = sprite.uv_region;
    v_clip = sprite.clip;
    v_clip_enabled = sprite.viewport.z;
    v_texture_mode = sprite.axis_v_mode.z;
    v_sprite_corner = corner[vertex];
    v_pixel = pixel;
    v_uv = vec2(
        sprite.uv_region.x + corner[vertex].x * sprite.uv_region.z,
        sprite.uv_region.y + corner[vertex].y * sprite.uv_region.w
    );
    v_color = sprite.color;
}
"#;

const FRAGMENT_SHADER_OPENGL: &[u8] = br#"#version 330 core
uniform sampler2D Sampler0;
in vec2 v_uv;
in vec2 v_sprite_corner;
in vec2 v_pixel;
in vec4 v_color;
flat in vec4 v_uv_region;
flat in vec4 v_clip;
flat in float v_texture_mode;
flat in float v_clip_enabled;
out vec4 out_color;
void main() {
    if (v_clip_enabled > 0.5 && (v_pixel.x < v_clip.x || v_pixel.y < v_clip.y || v_pixel.x >= v_clip.z || v_pixel.y >= v_clip.w)) {
        discard;
    }
    // The explicit resource set owns filtering/addressing. Reconstructing a
    // texel with floor() bypasses that contract and changes minification ties.
    vec4 sampled = texture(Sampler0, v_uv);
#ifdef GUI_ITEM_CUTOUT
    if (sampled.a < 0.1) discard;
#endif
    vec4 color = (v_texture_mode < 0.5 ? vec4(1.0, 1.0, 1.0, sampled.r) : sampled) * v_color;
    if (color.a <= 0.0) {
        discard;
    }
#ifdef GUI_ITEM_RASTER
    if (color.a < 0.1) discard;
#endif
    out_color = color;
}
"#;

const FRAGMENT_SHADER_VULKAN: &[u8] = br#"#version 450
layout(set = 0, binding = 1) uniform texture2D Tex0;
layout(set = 0, binding = 2) uniform sampler Samp0;
layout(location = 0) in vec2 v_uv;
layout(location = 1) in vec2 v_sprite_corner;
layout(location = 2) in vec2 v_pixel;
layout(location = 3) in vec4 v_color;
layout(location = 4) flat in vec4 v_uv_region;
layout(location = 5) flat in vec4 v_clip;
layout(location = 6) flat in float v_texture_mode;
layout(location = 7) flat in float v_clip_enabled;
layout(location = 0) out vec4 out_color;
void main() {
    if (v_clip_enabled > 0.5 && (v_pixel.x < v_clip.x || v_pixel.y < v_clip.y || v_pixel.x >= v_clip.z || v_pixel.y >= v_clip.w)) {
        discard;
    }
    // The explicit resource set owns filtering/addressing. Reconstructing a
    // texel with floor() bypasses that contract and changes minification ties.
    vec4 sampled = texture(sampler2D(Tex0, Samp0), v_uv);
#ifdef GUI_ITEM_CUTOUT
    if (sampled.a < 0.1) discard;
#endif
    vec4 color = (v_texture_mode < 0.5 ? vec4(1.0, 1.0, 1.0, sampled.r) : sampled) * v_color;
    if (color.a <= 0.0) {
        discard;
    }
#ifdef GUI_ITEM_RASTER
    if (color.a < 0.1) discard;
#endif
    out_color = color;
}
"#;

const BLUR_VERTEX_SHADER_VULKAN: &[u8] = br#"#version 450
const vec2 positions[3] = vec2[3](vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
layout(location = 0) out vec2 v_uv;
void main() {
    vec2 position = positions[gl_VertexIndex];
    gl_Position = vec4(position, 0.0, 1.0);
    v_uv = position * 0.5 + 0.5;
}
"#;

const BLUR_FRAGMENT_SHADER_VULKAN: &[u8] = br#"#version 450
layout(set = 0, binding = 0) uniform texture2D Source;
layout(set = 0, binding = 1) uniform sampler SourceSampler;
layout(set = 0, binding = 2, std140) uniform BlurConfig {
    vec2 BlurDir;
    float Radius;
};
layout(location = 0) in vec2 v_uv;
layout(location = 0) out vec4 out_color;
void main() {
    vec2 texel = 1.0 / vec2(textureSize(sampler2D(Source, SourceSampler), 0));
    // Source and destination have the same explicit framebuffer extent.
    // Fragment coordinates preserve copied-image orientation under the GAL's
    // negative-height Vulkan viewport; interpolated clip-space UVs do not.
    vec2 source_uv = gl_FragCoord.xy * texel;
    vec2 sample_step = texel * BlurDir;
    float actual_radius = max(round(Radius), 0.0);
    vec4 blurred = vec4(0.0);
    for (float a = -actual_radius + 0.5; a <= actual_radius; a += 2.0) {
        blurred += texture(sampler2D(Source, SourceSampler), source_uv + sample_step * a);
    }
    blurred += texture(sampler2D(Source, SourceSampler), source_uv + sample_step * actual_radius) * 0.5;
    out_color = blurred / (actual_radius + 0.5);
}
"#;

const INVERT_FRAGMENT_SHADER_VULKAN: &[u8] = br#"#version 450
layout(set = 0, binding = 0) uniform texture2D Source;
layout(set = 0, binding = 1) uniform sampler SourceSampler;
layout(set = 0, binding = 2, std140) uniform InvertConfig {
    float InverseAmount;
};
layout(location = 0) in vec2 v_uv;
layout(location = 0) out vec4 out_color;
void main() {
    // The copied image and destination share a framebuffer extent. Address
    // physical rows directly; clip-space UVs flip under a negative viewport.
    vec2 source_uv = gl_FragCoord.xy / vec2(textureSize(sampler2D(Source, SourceSampler), 0));
    vec4 source = texture(sampler2D(Source, SourceSampler), source_uv);
    vec4 inverted = vec4(1.0) - source;
    vec4 result = mix(source, inverted, clamp(InverseAmount, 0.0, 1.0));
    out_color = vec4(result.rgb, 1.0);
}
"#;

const CREEPER_COLOR_FRAGMENT_SHADER_VULKAN: &[u8] = br#"#version 450
layout(set = 0, binding = 0) uniform texture2D Source;
layout(set = 0, binding = 1) uniform sampler SourceSampler;
layout(set = 0, binding = 2, std140) uniform ColorConfig { vec3 RedMatrix; vec3 GreenMatrix; vec3 BlueMatrix; };
layout(location = 0) in vec2 v_uv;
layout(location = 0) out vec4 out_color;
void main() {
    vec3 input_color = texture(sampler2D(Source, SourceSampler), v_uv).rgb;
    vec3 color = vec3(dot(input_color, RedMatrix), dot(input_color, GreenMatrix), dot(input_color, BlueMatrix));
    float luma = dot(color, vec3(0.3, 0.59, 0.11));
    color = (color - luma) * 1.8 + luma;
    out_color = vec4(color, 1.0);
}
"#;

const CREEPER_BITS_FRAGMENT_SHADER_VULKAN: &[u8] = br#"#version 450
layout(set = 0, binding = 0) uniform texture2D Source;
layout(set = 0, binding = 1) uniform sampler SourceSampler;
layout(set = 0, binding = 2, std140) uniform BitsConfig { float Resolution; float MosaicSize; };
layout(location = 0) in vec2 v_uv;
layout(location = 0) out vec4 out_color;
void main() {
    vec2 size = vec2(textureSize(sampler2D(Source, SourceSampler), 0));
    vec2 mosaic = size / max(MosaicSize, 1.0);
    vec2 fract_pixel = fract(v_uv * mosaic) / mosaic;
    vec4 base = texture(sampler2D(Source, SourceSampler), v_uv - fract_pixel);
    vec3 quantized = base.rgb - fract(base.rgb * Resolution) / Resolution;
    float luma = dot(quantized, vec3(0.3, 0.59, 0.11));
    base.rgb = luma + (quantized - luma) * 1.5;
    out_color = vec4(base.rgb, 1.0);
}
"#;

const SPIDER_BOX_BLUR_FRAGMENT_SHADER_VULKAN: &[u8] = br#"#version 450
layout(set = 0, binding = 0) uniform texture2D Source;
layout(set = 0, binding = 1) uniform sampler SourceSampler;
layout(set = 0, binding = 2, std140) uniform BlurConfig { vec2 BlurDir; float Radius; };
layout(location = 0) in vec2 v_uv;
layout(location = 0) out vec4 out_color;
void main() {
    vec2 texel = 1.0 / vec2(textureSize(sampler2D(Source, SourceSampler), 0));
    vec2 sample_step = texel * BlurDir;
    float actual_radius = max(round(Radius), 0.0);
    vec4 blurred = vec4(0.0);
    for (float a = -actual_radius + 0.5; a <= actual_radius; a += 2.0) {
        blurred += texture(sampler2D(Source, SourceSampler), v_uv + sample_step * a);
    }
    blurred += texture(sampler2D(Source, SourceSampler), v_uv + sample_step * actual_radius) * 0.5;
    out_color = blurred / (actual_radius + 0.5);
}
"#;

const SPIDER_ROT_SCALE_VERTEX_SHADER_VULKAN: &[u8] = br#"#version 450
const vec2 positions[3] = vec2[3](vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
layout(set = 0, binding = 3, std140) uniform SpiderConfig {
    vec2 InScale;
    vec2 InOffset;
    float InRotation;
    vec4 Scissor;
    vec4 Vignette;
};
layout(location = 0) out vec2 v_uv;
layout(location = 1) out vec2 v_scaled_uv;
void main() {
    vec2 position = positions[gl_VertexIndex];
    gl_Position = vec4(position, 0.0, 1.0);
    v_uv = position * 0.5 + 0.5;
    float radians_value = InRotation * 0.0174532925;
    float c = cos(radians_value);
    float s = sin(radians_value);
    vec2 rotated = vec2(v_uv.x * c - v_uv.y * s, v_uv.y * c + v_uv.x * s);
    v_scaled_uv = rotated * InScale + InOffset;
}
"#;

const SPIDER_CLIP_FRAGMENT_SHADER_VULKAN: &[u8] = br#"#version 450
layout(set = 0, binding = 0) uniform texture2D InTexture;
layout(set = 0, binding = 1) uniform texture2D BlurTexture;
layout(set = 0, binding = 2) uniform sampler SourceSampler;
layout(set = 0, binding = 3, std140) uniform SpiderConfig {
    vec2 InScale;
    vec2 InOffset;
    float InRotation;
    vec4 Scissor;
    vec4 Vignette;
};
layout(location = 0) in vec2 v_uv;
layout(location = 1) in vec2 v_scaled_uv;
layout(location = 0) out vec4 out_color;
void main() {
    vec4 scaled = texture(sampler2D(InTexture, SourceSampler), clamp(v_scaled_uv, 0.0, 1.0));
    vec4 blurred = texture(sampler2D(BlurTexture, SourceSampler), v_uv);
    vec4 result = scaled;
    if (v_scaled_uv.x < Scissor.x || v_scaled_uv.y < Scissor.y || v_scaled_uv.x > Scissor.z || v_scaled_uv.y > Scissor.w) result = blurred;
    if (v_scaled_uv.x < Vignette.x) result = mix(blurred, result, clamp((Scissor.x - v_scaled_uv.x) / (Scissor.x - Vignette.x), 0.0, 1.0));
    if (v_scaled_uv.y < Vignette.y) result = mix(blurred, result, clamp((Scissor.y - v_scaled_uv.y) / (Scissor.y - Vignette.y), 0.0, 1.0));
    if (v_scaled_uv.x > Vignette.z) result = mix(blurred, result, clamp((Scissor.z - v_scaled_uv.x) / (Scissor.z - Vignette.z), 0.0, 1.0));
    if (v_scaled_uv.y > Vignette.w) result = mix(blurred, result, clamp((Scissor.w - v_scaled_uv.y) / (Scissor.w - Vignette.w), 0.0, 1.0));
    out_color = vec4(result.rgb, 1.0);
}
"#;

const SPIDER_BLIT_FRAGMENT_SHADER_VULKAN: &[u8] = br#"#version 450
layout(set = 0, binding = 0) uniform texture2D Source;
layout(set = 0, binding = 1) uniform sampler SourceSampler;
layout(set = 0, binding = 2, std140) uniform BlitConfig { vec4 ColorModulate; };
layout(location = 0) in vec2 v_uv;
layout(location = 0) out vec4 out_color;
void main() { out_color = texture(sampler2D(Source, SourceSampler), v_uv) * ColorModulate; }
"#;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum TextureGroup {
    Alpha,
    Invert,
    Dynamic(u64),
    /// A copied continuous image whose semantic producer requires filtering.
    /// The panorama shares its Rust-owned image with other dynamic GUI users;
    /// only this explicit sampler policy differs.
    DynamicLinear(u64),
    /// Standard/decal glint image: explicit linear repeating sampling.
    DynamicGlint(u64),
    DynamicOpaque(u64),
    DynamicVignette(u64),
    DynamicInvert(u64),
    DynamicPremultiplied(u64),
    DynamicAdditive(u64),
    DynamicLequalDepth(u64),
    /// Item-local translucent material; cutoff is shader state, not a GUI stratum.
    DynamicItemRaster(u64),
    DynamicItemCutout(u64),
}

impl TextureGroup {
    fn label(self) -> String {
        match self {
            Self::Alpha => "gui-alpha".to_string(),
            Self::Invert => "gui-invert".to_string(),
            Self::Dynamic(asset_id) => format!("gui-image-{asset_id}"),
            Self::DynamicLinear(asset_id) => format!("gui-image-linear-{asset_id}"),
            Self::DynamicGlint(asset_id) => format!("gui-image-glint-{asset_id}"),
            Self::DynamicOpaque(asset_id) => format!("gui-image-opaque-{asset_id}"),
            Self::DynamicVignette(asset_id) => format!("gui-image-vignette-{asset_id}"),
            Self::DynamicInvert(asset_id) => format!("gui-image-invert-{asset_id}"),
            Self::DynamicPremultiplied(asset_id) => format!("gui-image-premultiplied-{asset_id}"),
            Self::DynamicAdditive(asset_id) => format!("gui-image-additive-{asset_id}"),
            Self::DynamicLequalDepth(asset_id) => format!("gui-image-lequal-depth-{asset_id}"),
            Self::DynamicItemRaster(asset_id) => format!("gui-item-raster-{asset_id}"),
            Self::DynamicItemCutout(asset_id) => format!("gui-item-cutout-{asset_id}"),
        }
    }

    fn blend(self) -> BlendMode {
        match self {
            Self::Alpha => BlendMode::Alpha,
            Self::Invert => BlendMode::Invert,
            Self::Dynamic(_) => BlendMode::Alpha,
            Self::DynamicLinear(_) => BlendMode::Alpha,
            Self::DynamicGlint(_) => BlendMode::Glint,
            Self::DynamicOpaque(_) => BlendMode::Disabled,
            Self::DynamicVignette(_) => BlendMode::Vignette,
            Self::DynamicInvert(_) => BlendMode::Invert,
            Self::DynamicPremultiplied(_) => BlendMode::Premultiplied,
            Self::DynamicAdditive(_) => BlendMode::Additive,
            Self::DynamicLequalDepth(_) => BlendMode::Alpha,
            Self::DynamicItemRaster(_) => BlendMode::Alpha,
            Self::DynamicItemCutout(_) => BlendMode::Disabled,
        }
    }

    fn sampling(self) -> SamplerFilter {
        match self {
            Self::DynamicLinear(_) | Self::DynamicGlint(_) => SamplerFilter::Linear,
            _ => SamplerFilter::Nearest,
        }
    }
}

fn dynamic_texture_group(stratum: u32, asset_id: u64) -> TextureGroup {
    if stratum == GUI_OPAQUE_BLIT_STRATUM {
        TextureGroup::DynamicOpaque(asset_id)
    } else if stratum == GUI_VIGNETTE_BLIT_STRATUM {
        TextureGroup::DynamicVignette(asset_id)
    } else if stratum == GUI_INVERT_RECTANGLE_STRATUM {
        TextureGroup::DynamicInvert(asset_id)
    } else if stratum == GUI_CROSSHAIR_INVERT_STRATUM {
        TextureGroup::DynamicInvert(asset_id)
    } else if stratum == GUI_PREMULTIPLIED_BLIT_STRATUM {
        TextureGroup::DynamicPremultiplied(asset_id)
    } else if stratum == GUI_ADDITIVE_BLIT_STRATUM {
        TextureGroup::DynamicAdditive(asset_id)
    } else if stratum == GUI_LEQUAL_DEPTH_BLIT_STRATUM {
        TextureGroup::DynamicLequalDepth(asset_id)
    } else {
        TextureGroup::Dynamic(asset_id)
    }
}

fn dynamic_mesh_texture_group(draw: &GuiMeshPreparedDraw) -> TextureGroup {
    if draw.material_mode == GuiMeshMaterialMode::Panorama {
        TextureGroup::DynamicLinear(draw.asset_id)
    } else if draw.material_mode == GuiMeshMaterialMode::Glint {
        TextureGroup::DynamicGlint(draw.asset_id)
    } else {
        dynamic_texture_group(draw.stratum, draw.asset_id)
    }
}

#[derive(Clone, Copy)]
struct SpriteDef {
    id: u32,
    stratum: u32,
    name: &'static str,
    path: &'static str,
    width: u32,
    height: u32,
    group: TextureGroup,
}

#[derive(Clone, Copy)]
struct AtlasRegion {
    x: u32,
    y: u32,
}

#[derive(Clone)]
struct TextureAtlas {
    width: u32,
    height: u32,
    bytes: Vec<u8>,
    regions: BTreeMap<u32, AtlasRegion>,
}

#[derive(Clone, Copy)]
enum GuiImageOwnership {
    Owned { upload_buffer: Handle },
    SharedRaw { key: (u64, GuiRawImageFormat) },
    /// The GUI atlas cache owns the view; the world owner owns the image.
    /// This binding owns only its sampler and draw resources.
    AtlasView,
}

struct GuiResources {
    /// An optional sampler owned by this binding, distinct from the image's
    /// shared clamp samplers. Allocated only for repeating glint bindings.
    private_sampler: Option<Handle>,
    index_buffer: Handle,
    uniform_buffer: Handle,
    texture: Handle,
    sampler: Handle,
    texture_view: Handle,
    resource_set: Handle,
    pipeline_layout: Handle,
    pipeline: Handle,
    image_ownership: GuiImageOwnership,
}

impl GuiResources {
    /// The program objects are borrowed from `GuiFrontend::shared_pipelines`.
    /// A texture resource owns only its explicit texture/buffer/set state.
    fn handles_in_destroy_order(&self) -> Vec<Handle> {
        let mut handles = vec![self.resource_set, self.uniform_buffer, self.index_buffer];
        handles.extend(self.private_sampler);
        match self.image_ownership {
            GuiImageOwnership::Owned { upload_buffer } => handles.extend([
                self.texture_view,
                self.sampler,
                self.texture,
                upload_buffer,
            ]),
            GuiImageOwnership::SharedRaw { .. } => {},
            GuiImageOwnership::AtlasView => handles.push(self.sampler),
        }
        handles
    }
}

#[derive(Clone, Copy)]
struct SharedDynamicGuiTexture {
    upload_buffer: Handle,
    texture: Handle,
    nearest_sampler: Handle,
    linear_sampler: Handle,
    texture_view: Handle,
}

impl SharedDynamicGuiTexture {
    fn sampler(self, sampling: SamplerFilter) -> Handle {
        match sampling {
            SamplerFilter::Linear => self.linear_sampler,
            SamplerFilter::Nearest => self.nearest_sampler,
        }
    }
}

/// Immutable GUI program state shared by all texture resources with the same
/// explicit framebuffer and raster contract.  Texture resources still own
/// their descriptor sets because each set contains a distinct texture and
/// uniform buffer; only the identical shader/layout/pipeline objects are
/// reused.  This prevents an atlas or raw image count from multiplying native
/// pipeline compiler residency.
#[derive(Clone, Copy)]
struct GuiSharedPipeline {
    vertex_shader: Handle,
    fragment_shader: Handle,
    resource_layout: Handle,
    pipeline_layout: Handle,
    pipeline: Handle,
}

impl GuiSharedPipeline {
    fn handles_in_destroy_order(self) -> [Handle; 5] {
        [
            self.pipeline,
            self.pipeline_layout,
            self.resource_layout,
            self.fragment_shader,
            self.vertex_shader,
        ]
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct GuiSharedPipelineKey {
    color_format: ColorFormat,
    depth_format: Option<TextureFormat>,
    blend: u32,
    depth_compare: u32,
    item_raster: bool,
    item_cutout: bool,
}

impl GuiSharedPipelineKey {
    fn new(
        group: TextureGroup,
        color_format: ColorFormat,
        depth_format: Option<TextureFormat>,
    ) -> Self {
        Self {
            color_format,
            depth_format,
            blend: group.blend() as u32,
            item_raster: matches!(group, TextureGroup::DynamicItemRaster(_)),
            item_cutout: matches!(group, TextureGroup::DynamicItemCutout(_)),
            depth_compare: if matches!(group, TextureGroup::DynamicLequalDepth(_)) {
                CompareOp::LessOrEqual as u32
            } else {
                0
            },
        }
    }
}

const GUI_MAX_SHARED_PIPELINES: usize = 16;

fn gui_fragment_material_code(source: &[u8], item_raster: bool, item_cutout: bool) -> Vec<u8> {
    if !item_raster && !item_cutout { return source.to_vec(); }
    // GLSL requires #version first. This specialization belongs to the explicit
    // item pipeline and cannot affect normal GUI images sharing its atlas.
    let version_end = source.iter().position(|byte| *byte == b'\n').unwrap() + 1;
    let mut code = source[..version_end].to_vec();
    code.extend_from_slice(if item_cutout { b"#define GUI_ITEM_CUTOUT\n" } else { b"#define GUI_ITEM_RASTER\n" });
    code.extend_from_slice(&source[version_end..]);
    code
}

#[derive(Clone, Copy)]
struct CachedPass {
    frame_target: Handle,
    pass: Handle,
    depth_format: Option<TextureFormat>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct GuiMeshRasterKey {
    asset_id: u64,
    material_mode: GuiMeshMaterialMode,
    front_face: super::resources::FrontFace,
    /// The raster pass owns one frame uniform buffer. Its placement and
    /// lighting inputs are therefore part of the cache identity: reusing the
    /// same texture pipeline for a different PIP target must not overwrite an
    /// earlier mesh draw's extent before the command list executes.
    render_extent: [u32; 2],
    alpha_cutoff_bits: u32,
    lighting_mode: GuiMeshLightingMode,
    /// Panorama is rasterized directly into the acquired frame target. Its
    /// pipeline must therefore be distinct from an otherwise-identical PIP
    /// raster whose attachment format is the private RGBA8 target.
    direct_target_format: Option<TextureFormat>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct GuiMeshSharedProgramKey {
    color_format: TextureFormat,
    depth_format: Option<TextureFormat>,
    material_mode: GuiMeshMaterialMode,
    front_face: super::resources::FrontFace,
}

fn gui_mesh_raster_key(draw: &GuiMeshPreparedDraw) -> GuiMeshRasterKey {
    GuiMeshRasterKey {
        asset_id: draw.asset_id,
        material_mode: draw.material_mode,
        front_face: draw.front_face,
        render_extent: draw.render_extent,
        alpha_cutoff_bits: draw.alpha_cutoff.to_bits(),
        lighting_mode: draw.lighting_mode,
        direct_target_format: None,
    }
}

fn direct_gui_mesh_raster_key(
    draw: &GuiMeshPreparedDraw,
    target_format: TextureFormat,
) -> GuiMeshRasterKey {
    let mut key = gui_mesh_raster_key(draw);
    key.direct_target_format = Some(target_format);
    key
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct GuiMeshCompositeKey {
    item_identity: u64,
    width: u32,
    height: u32,
    color_format: ColorFormat,
    depth_format: Option<TextureFormat>,
}

/// One private GUI-mesh stream allocation. It remains unavailable until the
/// submission that references it has completed, then can be reused by later
/// semantic mesh work without overwriting in-flight GPU reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct GuiMeshGeometryResidency {
    stream: GuiMeshStreamRange,
    vertex_bytes: u64,
    index_bytes: u64,
    last_submission: SubmissionId,
}

#[derive(Default)]
pub struct GuiFrontend {
    generation: u64,
    asset_generation: u64,
    raw_image_generation: u64,
    asset_overrides: BTreeMap<u32, Vec<u8>>,
    raw_images: BTreeMap<u64, RawGuiImage>,
    atlas_references: GuiAtlasReferences,
    atlas_views: BTreeMap<u64, (AcceptedAtlasIncarnation, Handle)>,
    atlases: BTreeMap<TextureGroupKey, TextureAtlas>,
    resources: BTreeMap<ResourceKey, GuiResources>,
    dynamic_textures: BTreeMap<(u64, GuiRawImageFormat), SharedDynamicGuiTexture>,
    shared_pipelines: BTreeMap<GuiSharedPipelineKey, GuiSharedPipeline>,
    cached_pass: Option<CachedPass>,
    mesh_targets: GuiMeshOffscreenTargetCache,
    mesh_rasters: BTreeMap<GuiMeshRasterKey, GuiMeshPassResources>,
    mesh_shared_programs: BTreeMap<GuiMeshSharedProgramKey, GuiMeshSharedProgram>,
    mesh_composites: BTreeMap<GuiMeshCompositeKey, GuiMeshCompositeResources>,
    item_rasters: BTreeMap<GuiMeshCompositeKey, GuiItemRasterResources>,
    item_raster_slots: super::gui_item_raster::GuiItemRasterSlots,
    // A prepared upload is reusable only within its command transaction.
    // Older allocations remain reserved until completion, but preparation
    // alone is never evidence that their bytes reached the GPU.
    mesh_geometry_transaction: u64,
    mesh_geometry_cache: BTreeMap<(GuiMeshRasterKey, u64, u64), GuiMeshGeometryResidency>,
    mesh_geometry_free_ranges: BTreeMap<GuiMeshRasterKey, Vec<GuiMeshGeometryResidency>>,
    mesh_composite_uniform_cursor: u64,
    blur_resources: Option<GuiBlurResources>,
    custom_post_effect_resources: Vec<CustomPostEffectResources>,
    custom_post_effect_intermediates: BTreeMap<String, CustomPostEffectIntermediate>,
    /// Cached color-only alias for a depth-bearing render target. It aliases
    /// the original color view but deliberately has no depth attachment, so a
    /// custom pass can sample the original depth image without an attachment
    /// layout conflict. The alias is Rust-owned and never exposed as a native
    /// or Java handle.
    custom_post_effect_depth_target: Option<CustomPostEffectDepthTarget>,
    blur_snapshot_initialized: bool,
    custom_post_effect_snapshot_initialized: Vec<Vec<bool>>,
    custom_post_effect_image_initialized: Vec<Vec<bool>>,
    creeper_intermediate_initialized: bool,
    spider_initialized: [bool; 4],
}

struct GuiItemRasterResources {
    target: super::gui_item_raster::GuiItemRasterTarget,
    composite: GuiMeshCompositeResources,
    usage: TextureUsageState,
}

/// Rust-owned resources for the bounded main-target custom post-effect
/// contract. Each pass owns an independent snapshot and pipeline; this keeps
/// sequential main-target chains explicit without allocating arbitrary graph
/// intermediates or borrowing Java PostChain/Iris handles.
struct CustomPostEffectResources {
    frame_copy_scratch: Option<Handle>,
    identity: String,
    width: u32,
    height: u32,
    color_format: ColorFormat,
    snapshots: Vec<Handle>,
    snapshot_views: Vec<Handle>,
    target_samplers: Vec<Handle>,
    image_textures: Vec<Option<Handle>>,
    image_views: Vec<Option<Handle>>,
    image_samplers: Vec<Option<Handle>>,
    image_upload_buffers: Vec<Option<Handle>>,
    combined_samplers: Vec<Handle>,
    uniform_buffers: Vec<Handle>,
    vertex_shader: Handle,
    fragment_shader: Handle,
    resource_layout: Handle,
    resource_set: Handle,
    pipeline_layout: Handle,
    pipeline: Handle,
}

/// Rust-owned topology used when an acquired frame target is the source of a
/// depth-reading effect. The swapchain image stays opaque; the effect renders
/// into these owned attachments and is copied back through the explicit GAL
/// command at the end of the chain.
struct CustomPostEffectDepthTarget {
    source: Handle,
    target: Handle,
    color_texture: Option<Handle>,
    color_view: Handle,
    owns_color_view: bool,
    depth_texture: Option<Handle>,
    depth_view: Handle,
    owns_depth_view: bool,
}

impl CustomPostEffectDepthTarget {
    fn handles_in_destroy_order(self) -> Vec<Handle> {
        let mut handles = vec![self.target];
        if self.owns_color_view {
            handles.push(self.color_view);
        }
        if let Some(texture) = self.color_texture {
            handles.push(texture);
        }
        if self.owns_depth_view {
            handles.push(self.depth_view);
        }
        if let Some(texture) = self.depth_texture {
            handles.push(texture);
        }
        handles
    }
}

/// One bounded Rust-owned intermediate target for a custom post-effect graph.
/// The graph admission layer owns each declared target independently; the
/// bounded scheduler still rejects feedback and cycles before allocation.
struct CustomPostEffectIntermediate {
    identity: String,
    width: u32,
    height: u32,
    color_format: ColorFormat,
    texture: Handle,
    view: Handle,
    target: Handle,
    pass: Handle,
    usage: TextureUsageState,
}

impl CustomPostEffectIntermediate {
    fn handles_in_destroy_order(&self) -> [Handle; 4] {
        [self.pass, self.target, self.view, self.texture]
    }
}

impl CustomPostEffectResources {
    fn handles_in_destroy_order(&self) -> Vec<Handle> {
        // Destroy dependents before the resources they reference. In
        // particular, combined samplers retain both the sampler and the
        // snapshot view, while the resource set retains the combined samplers
        // and uniform buffers. The explicit GAL permits deferred retirement,
        // but preserving this order keeps destruction valid for immediate
        // validation and for backends with stricter lifetime checks.
        let mut handles = vec![
            self.pipeline,
            self.pipeline_layout,
            self.resource_set,
            self.resource_layout,
            self.fragment_shader,
            self.vertex_shader,
        ];
        handles.extend(self.combined_samplers.iter().copied());
        handles.extend(self.image_upload_buffers.iter().flatten().copied());
        handles.extend(self.image_samplers.iter().flatten().copied());
        handles.extend(self.image_views.iter().flatten().copied());
        handles.extend(self.image_textures.iter().flatten().copied());
        handles.extend(self.uniform_buffers.iter().copied());
        handles.extend(self.target_samplers.iter().copied());
        handles.extend(self.snapshot_views.iter().copied());
        handles.extend(self.snapshots.iter().copied());
        handles.extend(self.frame_copy_scratch);
        handles
    }
}

/// One copied custom post-effect pass. Shader code and static uniform bytes
/// originate in the Rust-owned resource snapshot; no Java uniform object or
/// backend handle crosses this boundary.
pub(crate) struct CustomPostEffectSource {
    /// Framebuffer-derived input row transformation; uploaded images retain
    /// their declared pixel order independently of attachment coordinates.
    pub(crate) input_row_order: super::commands::TextureRowOrder,
    pub(crate) input_bilinear: Vec<bool>,
    pub(crate) sampler_info_uniform: Option<usize>,
    pub(crate) vertex_shader: Vec<u8>,
    pub(crate) fragment_shader: Vec<u8>,
    pub(crate) input_count: usize,
    pub(crate) input_targets: Vec<String>,
    pub(crate) input_images: Vec<Option<CustomPostEffectImage>>,
    pub(crate) input_use_depth: Vec<bool>,
    pub(crate) output_target: String,
    pub(crate) uniform_blocks: Vec<Vec<u8>>,
}

/// Copied semantic pixels for one resource-pack texture input. This type owns
/// no GAL object; upload and sampler lowering happen only in the Rust GUI
/// frontend after the graph has passed admission.
#[derive(Clone)]
pub(crate) struct CustomPostEffectImage {
    pub(crate) path: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) pixels_rgba8: Vec<u8>,
    pub(crate) bilinear: bool,
}

struct GuiBlurResources {
    width: u32,
    height: u32,
    color_format: ColorFormat,
    texture: Handle,
    creeper_texture: Handle,
    creeper_view: Handle,
    creeper_target: Handle,
    creeper_pass: Handle,
    uniform_buffer: Handle,
    texture_view: Handle,
    sampler: Handle,
    vertex_shader: Handle,
    fragment_shader: Handle,
    invert_fragment_shader: Handle,
    creeper_color_shader: Handle,
    creeper_bits_shader: Handle,
    resource_layout: Handle,
    resource_set: Handle,
    creeper_resource_set: Handle,
    pipeline_layout: Handle,
    pipeline: Handle,
    invert_pipeline: Handle,
    creeper_color_pipeline: Handle,
    creeper_bits_pipeline: Handle,
    spider_textures: [Handle; 4],
    spider_views: [Handle; 4],
    spider_targets: [Handle; 4],
    spider_passes: [Handle; 4],
    spider_single_sets: [Handle; 4],
    spider_dual_sets: [Handle; 3],
    spider_dual_layout: Handle,
    spider_dual_pipeline_layout: Handle,
    spider_box_shader: Handle,
    spider_rot_shader: Handle,
    spider_clip_shader: Handle,
    spider_blit_shader: Handle,
    spider_box_pipeline: Handle,
    spider_clip_pipeline: Handle,
    spider_blit_pipeline: Handle,
}

impl GuiBlurResources {
    fn handles_in_destroy_order(&self) -> Vec<Handle> {
        let mut handles = vec![
            self.spider_blit_pipeline,
            self.spider_clip_pipeline,
            self.spider_box_pipeline,
            self.spider_dual_pipeline_layout,
            self.spider_dual_sets[0],
            self.spider_dual_sets[1],
            self.spider_dual_sets[2],
            self.spider_single_sets[0],
            self.spider_single_sets[1],
            self.spider_single_sets[2],
            self.spider_single_sets[3],
            self.spider_dual_layout,
            self.creeper_pass,
            self.creeper_target,
            self.creeper_bits_pipeline,
            self.creeper_color_pipeline,
            self.pipeline,
            self.invert_pipeline,
            self.pipeline_layout,
            self.creeper_resource_set,
            self.resource_set,
            self.resource_layout,
            self.texture_view,
            self.fragment_shader,
            self.invert_fragment_shader,
            self.creeper_bits_shader,
            self.creeper_color_shader,
            self.vertex_shader,
            self.sampler,
            self.uniform_buffer,
            self.texture,
            self.creeper_view,
            self.creeper_texture,
        ];
        handles.extend(self.spider_passes);
        handles.extend(self.spider_targets);
        handles.extend(self.spider_views);
        handles.extend(self.spider_textures);
        handles.extend([
            self.spider_rot_shader,
            self.spider_clip_shader,
            self.spider_blit_shader,
            self.spider_box_shader,
        ]);
        handles
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum TextureGroupKey {
    Alpha,
    Invert,
    Dynamic(u64),
    DynamicLinear(u64),
    DynamicGlint(u64),
    DynamicOpaque(u64),
    DynamicVignette(u64),
    DynamicInvert(u64),
    DynamicPremultiplied(u64),
    DynamicAdditive(u64),
    DynamicLequalDepth(u64),
    DynamicItemRaster(u64),
    DynamicItemCutout(u64),
}

impl TextureGroupKey {
    fn dynamic_asset_id(self) -> Option<u64> {
        match self {
            Self::Dynamic(asset_id)
            | Self::DynamicLinear(asset_id)
            | Self::DynamicGlint(asset_id)
            | Self::DynamicOpaque(asset_id)
            | Self::DynamicVignette(asset_id)
            | Self::DynamicInvert(asset_id)
            | Self::DynamicPremultiplied(asset_id)
            | Self::DynamicAdditive(asset_id)
            | Self::DynamicLequalDepth(asset_id)
            | Self::DynamicItemRaster(asset_id)
            | Self::DynamicItemCutout(asset_id) => Some(asset_id),
            Self::Alpha | Self::Invert => None,
        }
    }

    fn is_dynamic(self) -> bool {
        matches!(
            self,
            Self::Dynamic(_)
                | Self::DynamicLinear(_)
                | Self::DynamicGlint(_)
                | Self::DynamicOpaque(_)
                | Self::DynamicVignette(_)
                | Self::DynamicInvert(_)
                | Self::DynamicPremultiplied(_)
                | Self::DynamicAdditive(_)
                | Self::DynamicLequalDepth(_)
                | Self::DynamicItemRaster(_)
                | Self::DynamicItemCutout(_)
        )
    }
}

impl From<TextureGroup> for TextureGroupKey {
    fn from(value: TextureGroup) -> Self {
        match value {
            TextureGroup::Alpha => Self::Alpha,
            TextureGroup::Invert => Self::Invert,
            TextureGroup::Dynamic(asset_id) => Self::Dynamic(asset_id),
            TextureGroup::DynamicLinear(asset_id) => Self::DynamicLinear(asset_id),
            TextureGroup::DynamicGlint(asset_id) => Self::DynamicGlint(asset_id),
            TextureGroup::DynamicOpaque(asset_id) => Self::DynamicOpaque(asset_id),
            TextureGroup::DynamicVignette(asset_id) => Self::DynamicVignette(asset_id),
            TextureGroup::DynamicInvert(asset_id) => Self::DynamicInvert(asset_id),
            TextureGroup::DynamicPremultiplied(asset_id) => Self::DynamicPremultiplied(asset_id),
            TextureGroup::DynamicAdditive(asset_id) => Self::DynamicAdditive(asset_id),
            TextureGroup::DynamicLequalDepth(asset_id) => Self::DynamicLequalDepth(asset_id),
            TextureGroup::DynamicItemRaster(asset_id) => Self::DynamicItemRaster(asset_id),
            TextureGroup::DynamicItemCutout(asset_id) => Self::DynamicItemCutout(asset_id),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ResourceKey {
    group: TextureGroupKey,
    color_format: ColorFormat,
    depth_format: Option<TextureFormat>,
}

impl ResourceKey {
    fn new(
        group: TextureGroup,
        color_format: ColorFormat,
        depth_format: Option<TextureFormat>,
    ) -> Self {
        Self {
            group: TextureGroupKey::from(group),
            color_format,
            depth_format,
        }
    }
}

#[derive(Clone, Debug)]
pub struct GuiSpriteRequest {
    pub stratum: u32,
    pub sprite_id: u32,
    pub selected_slot: i32,
    pub progress_fraction: f32,
    pub fill_direction: u32,
    pub color_argb: u32,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub gui_width: u32,
    pub gui_height: u32,
    /// Exact semantic projection, distinct from rounded layout/clip bounds.
    pub projection_extent: [f32; 2],
    pub sequence: u64,
}

#[derive(Clone, Debug, Default)]
pub struct GuiSubmitStats {
    pub submission_id: u64,
    pub sprite_count: u64,
    pub affine_quad_count: u64,
    /// Coarse standard-3D GUI items that completed owned raster/composite
    /// command construction in this submission.
    pub mesh_item_count: u64,
    /// Semantic mesh layers consumed by the owned GUI-mesh raster path.
    pub mesh_batch_count: u64,
    /// Raster plus compose draws emitted for the mesh items.
    pub mesh_draw_count: u64,
    pub sprite_batch_count: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub resource_creates: u64,
    pub command_lists: u64,
    pub command_ops: u64,
    /// Private Rust-owned offscreen raster targets used by standard-3D GUI
    /// items before they are composited into the requested GUI target. This
    /// never contains an acquired frame target or a Java-owned surface.
    pub(crate) owned_intermediate_targets: Vec<Handle>,
    /// Frame-local pass objects used only by a bounded diagnostic replay.
    /// They are not cached because their target retires with that capture.
    pub(crate) transient_diagnostic_passes: Vec<Handle>,
}

/// Semantic ordering plan for the screen-background blur boundary. Requests
/// from a render-state node use `node * 3 + phase` as their order; fixed
/// compatibility strata use larger orders and remain after a dynamic node
/// boundary. This plan contains counts only—no Java renderer or backend
/// resource can cross it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GuiBlurBoundaryPlan {
    pub boundary_stratum: u32,
    pub before_count: usize,
    pub after_count: usize,
}

impl GuiBlurBoundaryPlan {
    fn phase_threshold(self) -> GalResult<u32> {
        self.boundary_stratum.checked_mul(3).ok_or_else(|| {
            GalError::invalid_argument("GUI blur boundary stratum overflows phase order")
        })
    }
}

pub(crate) fn plan_gui_blur_boundary(
    boundary_stratum: i32,
    request_strata: impl IntoIterator<Item = u32>,
) -> GalResult<GuiBlurBoundaryPlan> {
    let boundary_stratum = u32::try_from(boundary_stratum).map_err(|_| {
        GalError::invalid_argument("GUI blur boundary must be non-negative when planned")
    })?;
    let threshold = boundary_stratum.checked_mul(3).ok_or_else(|| {
        GalError::invalid_argument("GUI blur boundary stratum overflows phase order")
    })?;
    let mut before_count = 0usize;
    let mut after_count = 0usize;
    for stratum in request_strata {
        if stratum < threshold {
            before_count = before_count.saturating_add(1);
        } else {
            after_count = after_count.saturating_add(1);
        }
    }
    Ok(GuiBlurBoundaryPlan {
        boundary_stratum,
        before_count,
        after_count,
    })
}

#[derive(Clone, Debug)]
pub struct GuiAssetPayload {
    pub sprite_id: u32,
    pub png_bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum GuiRawImageFormat {
    Alpha8,
    Rgba8,
}

impl GuiRawImageFormat {
    fn texture_format(self) -> TextureFormat {
        match self {
            Self::Alpha8 => TextureFormat::R8Unorm,
            Self::Rgba8 => TextureFormat::Rgba8Unorm,
        }
    }

    fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Alpha8 => 1,
            Self::Rgba8 => 4,
        }
    }

    fn shader_mode(self) -> f32 {
        match self {
            Self::Alpha8 => 0.0,
            Self::Rgba8 => 1.0,
        }
    }
}

/// CPU-owned image data. The FFI boundary will carry this as one bounded asset
/// update; it deliberately contains no atlas, renderer, or backend objects.
#[derive(Clone, Debug)]
pub struct GuiRawImageAssetPayload {
    pub asset_id: u64,
    pub format: GuiRawImageFormat,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    pub sampling: Option<(SamplerFilter, SamplerAddressMode)>,
}

/// One immutable item-local layer: no screen placement or scheduler identity.
#[derive(Clone, Debug)]
pub struct GuiItemRasterLayer {
    pub asset_id: u64,
    pub color_argb: u32,
    pub material: super::gui_item_material::GuiAffineMaterial,
    pub geometry: super::gui_item_raster::GuiItemRasterGeometry,
    pub uv: [f32;4],
    pub model_transform: super::gui_item_raster::GuiItemModelTransform,
}

#[derive(Clone, Debug)]
pub(crate) struct GuiItemRasterGroup {
    pub presentation: GuiAffineQuadRequest,
    pub layers: Vec<GuiItemRasterLayer>,
}

impl GuiItemRasterGroup {
    fn single(request: &GuiAffineQuadRequest) -> Self {
        if !request.item_raster_layers.is_empty() {
            return Self {presentation:request.clone(),layers:request.item_raster_layers.clone()};
        }
        Self { presentation: request.clone(), layers: vec![GuiItemRasterLayer {
            asset_id:request.asset_id,color_argb:request.color_argb,material:request.material,
            geometry:request.item_raster_geometry,uv:[request.u0,request.v0,request.u1,request.v1],
            model_transform: Default::default(),
        }] }
    }
}

/// An affine textured GUI primitive. Four explicit corners from Minecraft font
/// layout reduce to an origin plus two axes without losing italic shear.
#[derive(Clone, Debug)]
pub struct GuiAffineQuadRequest {
    /// Standard 16x16 item-cell raster scale; zero is an ordinary screen quad.
    pub item_raster_scale: u32,
    pub item_raster_layers: Vec<GuiItemRasterLayer>,
    pub item_raster_geometry: super::gui_item_raster::GuiItemRasterGeometry,
    pub material: super::gui_item_material::GuiAffineMaterial,
    pub stratum: u32,
    pub asset_id: u64,
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub x3: f32,
    pub y3: f32,
    pub z: f32,
    pub u0: f32,
    pub v0: f32,
    pub u1: f32,
    pub v1: f32,
    pub color_argb: u32,
    pub gui_width: u32,
    pub gui_height: u32,
    pub projection_extent: [f32; 2],
    pub sequence: u64,
    pub clip_mode: u32,
    pub clip_left: i32,
    pub clip_top: i32,
    pub clip_width: i32,
    pub clip_height: i32,
}

/// Typed immutable tiled command. Children retain their parent's scheduler
/// identity; only Rust lowers the repetition into explicit draw instances.
/// Private until Java/FFI transport and whole-frame admission are connected.
#[derive(Clone, Debug)]
pub(crate) struct GuiTiledQuadRequest {
    pub geometry: super::gui_tiling::GuiTileGeometry,
    pub stratum: u32,
    pub asset_id: u64,
    pub z: f32,
    pub color_argb: u32,
    pub gui_extent: [u32; 2],
    pub projection_extent: [f32; 2],
    pub sequence: u64,
    pub clip: Option<[i32; 4]>,
}

/// Includes ordinary affine requests and all expanded tiled children.
pub(crate) const GUI_MAX_EXPANDED_AFFINE_QUADS: usize = 65_536;

impl GuiTiledQuadRequest {
    pub(crate) fn validate(&self) -> GalResult<()> {
        validate_gui_projection(self.gui_extent, self.projection_extent)?;
        if self.asset_id == 0 || self.sequence == 0 || self.stratum == 0 || !self.z.is_finite()
            || self.gui_extent.iter().any(|v| *v == 0 || *v > GUI_MAX_VIEWPORT_AXIS as u32) {
            return Err(GalError::invalid_argument("invalid semantic tiled GUI identity or extent"));
        }
        if let Some([left, top, width, height]) = self.clip {
            if left < 0 || top < 0 || width < 0 || height < 0
                || i64::from(left) + i64::from(width) > i64::from(self.gui_extent[0])
                || i64::from(top) + i64::from(height) > i64::from(self.gui_extent[1]) {
                return Err(GalError::invalid_argument("tiled GUI clip must be frame-local"));
            }
        }
        Ok(())
    }
}

pub(crate) fn preflight_tiled_affine_count(
    requests: &[GuiTiledQuadRequest], ordinary_affine_count: usize,
) -> GalResult<usize> {
    let mut count = ordinary_affine_count;
    if count > GUI_MAX_EXPANDED_AFFINE_QUADS {
        return Err(GalError::invalid_argument("GUI frame affine expansion exceeds bounded limit"));
    }
    for request in requests {
        request.validate()?;
        count = count.checked_add(super::gui_tiling::tile_segment_count(request.geometry)?)
            .filter(|n| *n <= GUI_MAX_EXPANDED_AFFINE_QUADS)
            .ok_or_else(|| GalError::invalid_argument("GUI frame affine expansion exceeds bounded limit"))?;
    }
    Ok(count)
}

fn lower_tiled_request(request: GuiTiledQuadRequest) -> GalResult<Vec<GuiAffineQuadRequest>> {
    request.validate()?;
    let [clip_left, clip_top, clip_width, clip_height] = request.clip.unwrap_or([0; 4]);
    super::gui_tiling::lower_tiles(request.geometry)?.into_iter().map(|quad| {
        let child = GuiAffineQuadRequest {
            item_raster_layers: vec![],
            item_raster_scale: 0,
            item_raster_geometry: Default::default(),
            material: super::gui_item_material::GuiAffineMaterial::Unlit,
            stratum: request.stratum, asset_id: request.asset_id,
            x0: quad.origin[0], y0: quad.origin[1],
            x1: quad.origin[0] + quad.axis_u[0], y1: quad.origin[1] + quad.axis_u[1],
            x3: quad.origin[0] + quad.axis_v[0], y3: quad.origin[1] + quad.axis_v[1],
            z: request.z, u0: quad.uv[0], v0: quad.uv[1], u1: quad.uv[2], v1: quad.uv[3],
            color_argb: request.color_argb,
            gui_width: request.gui_extent[0], gui_height: request.gui_extent[1],
            projection_extent: request.projection_extent, sequence: request.sequence,
            clip_mode: u32::from(request.clip.is_some()), clip_left, clip_top, clip_width, clip_height,
        };
        validate_affine_quad(&child)?;
        Ok(child)
    }).collect()
}

pub(crate) fn validate_gui_frame_sequences(
    sprites: &[GuiSpriteRequest], affine: &[GuiAffineQuadRequest],
    meshes: &[GuiMeshBatchRequest], tiles: &[GuiTiledQuadRequest],
) -> GalResult<()> {
    let mesh_sequences = meshes.iter().map(|request| request.sequence)
        .collect::<std::collections::BTreeSet<_>>();
    let mut seen = std::collections::BTreeSet::new();
    for sequence in sprites.iter().map(|request| request.sequence)
        .chain(affine.iter().map(|request| request.sequence))
        .chain(tiles.iter().map(|request| request.sequence)).chain(mesh_sequences) {
        if sequence == 0 {
            return Err(GalError::invalid_argument("GUI requests require non-zero scheduler sequences"));
        }
        if !seen.insert(sequence) {
            return Err(GalError::invalid_argument("GUI request scheduler sequences must be unique within one frame"));
        }
    }
    Ok(())
}

#[derive(Debug)]
enum GuiFrameRequest {
    Sprite(GuiSpriteRequest),
    Affine(GuiAffineQuadRequest),
    /// Consecutive affine quads sharing one semantic texture group.  Their
    /// instance order remains the scheduler order, but one explicit draw can
    /// carry all of them.
    AffineBatch(Vec<GuiAffineQuadRequest>),
    Mesh(GuiMeshItem),
}

#[derive(Debug)]
struct GuiMeshItem {
    stratum: u32,
    sequence: u64,
    layers: Vec<GuiMeshBatchRequest>,
}

impl GuiFrameRequest {
    fn stratum(&self) -> u32 {
        match self {
            Self::Sprite(request) => request.stratum,
            Self::Affine(request) => request.stratum,
            Self::AffineBatch(requests) => requests[0].stratum,
            Self::Mesh(request) => request.stratum,
        }
    }

    fn sequence(&self) -> u64 {
        match self {
            Self::Sprite(request) => request.sequence,
            Self::Affine(request) => request.sequence,
            Self::AffineBatch(requests) => requests[0].sequence,
            Self::Mesh(request) => request.sequence,
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
struct RawGuiImage {
    format: GuiRawImageFormat,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    sampling: Option<(SamplerFilter, SamplerAddressMode)>,
}

#[derive(Clone)]
struct GuiTextureSource {
    width: u32,
    height: u32,
    format: GuiRawImageFormat,
    bytes: Vec<u8>,
}

#[derive(Clone, Copy)]
struct PackedGuiQuad {
    origin: [f32; 2],
    axis_u: [f32; 2],
    axis_v: [f32; 2],
    viewport: [f32; 2],
    clip: [f32; 4],
    clip_enabled: bool,
    /// The selected-source overlay receives one later final copy. Preserve
    /// Java's top-left GUI semantics by precompensating that copy in the
    /// Rust-owned GUI stream. Texture coordinates stay semantic: the later
    /// image copy supplies the corresponding texel-row inversion.
    pre_present_y_flip: bool,
    uv: [f32; 4],
    color: [f32; 4],
    texture_mode: f32,
    z: f32,
}

struct GuiBatch {
    stratum: u32,
    group: TextureGroup,
    quads: Vec<PackedGuiQuad>,
}

impl GuiFrontend {
    pub fn reset(&mut self, gal: &mut VulkanicGal) -> GalResult<()> {
        self.destroy_render_resources(gal);
        self.item_raster_slots = Default::default();
        self.retire_atlas_views_for_assets(gal, &self.atlas_views.keys().copied().collect())?;
        self.asset_overrides.clear();
        self.raw_images.clear();
        self.atlas_references.clear();
        self.asset_generation = 0;
        self.raw_image_generation = 0;
        self.generation = 0;
        Ok(())
    }

    fn reclaim_completed_mesh_geometry(&mut self, completed: SubmissionId) {
        let released: Vec<_> = self
            .mesh_geometry_cache
            .iter()
            .filter_map(|(key, residency)| {
                (residency.last_submission <= completed).then_some((*key, *residency))
            })
            .collect();
        for (key, residency) in released {
            self.mesh_geometry_cache.remove(&key);
            self.mesh_geometry_free_ranges
                .entry(key.0)
                .or_default()
                .push(residency);
        }
        for ranges in self.mesh_geometry_free_ranges.values_mut() {
            ranges.sort_by_key(|range| (range.stream.vertex_offset, range.stream.index_offset));
            let mut merged: Vec<GuiMeshGeometryResidency> = Vec::with_capacity(ranges.len());
            for range in ranges.drain(..) {
                if let Some(previous) = merged.last_mut() {
                    let vertex_end = previous.stream.vertex_offset + previous.vertex_bytes;
                    let index_end = previous.stream.index_offset + previous.index_bytes;
                    if vertex_end == range.stream.vertex_offset
                        && index_end == range.stream.index_offset
                    {
                        previous.vertex_bytes += range.vertex_bytes;
                        previous.index_bytes += range.index_bytes;
                        continue;
                    }
                }
                merged.push(range);
            }
            *ranges = merged;
        }
    }

    fn allocate_mesh_geometry(
        &mut self,
        gal: &mut VulkanicGal,
        raster_key: GuiMeshRasterKey,
        vertex_bytes: u64,
        index_bytes: u64,
        pending_submission: SubmissionId,
    ) -> GalResult<GuiMeshGeometryResidency> {
        self.mesh_geometry_free_ranges
            .entry(raster_key)
            .or_insert_with(|| {
                vec![GuiMeshGeometryResidency {
                    stream: GuiMeshStreamRange::default(),
                    vertex_bytes: super::gui_mesh_frontend::GUI_MESH_MAX_VERTEX_BYTES,
                    index_bytes: super::gui_mesh_frontend::GUI_MESH_MAX_INDEX_BYTES,
                    last_submission: SubmissionId(0),
                }]
            });
        let index = self.mesh_geometry_free_ranges[&raster_key]
            .iter()
            .position(|range| {
                range.vertex_bytes >= vertex_bytes && range.index_bytes >= index_bytes
            });
        let range = if let Some(index) = index {
            self.mesh_geometry_free_ranges
                .get_mut(&raster_key)
                .expect("range entry was initialized")
                .remove(index)
        } else {
            let oldest = self
                .mesh_geometry_cache
                .iter()
                .filter(|((key, _, _), _)| *key == raster_key)
                .map(|(_, residency)| residency.last_submission)
                .min()
                .ok_or_else(|| {
                    GalError::ffi(
                        StatusCode::InvalidArgument,
                        "GUI mesh geometry request exceeds the fixed stream capacity",
                    )
                })?;
            if oldest >= pending_submission {
                return Err(GalError::ffi(
                    StatusCode::InvalidArgument,
                    "GUI mesh frame requires more than the fixed stream capacity",
                ));
            }
            // This is bounded, explicit backpressure: wait for exactly the
            // oldest range that can make space, never overwrite an in-flight
            // stream and never grow the private buffers beyond their cap.
            gal.retire_through(oldest)?;
            self.reclaim_completed_mesh_geometry(oldest);
            return self.allocate_mesh_geometry(
                gal,
                raster_key,
                vertex_bytes,
                index_bytes,
                pending_submission,
            );
        };
        let allocation = GuiMeshGeometryResidency {
            stream: range.stream,
            vertex_bytes,
            index_bytes,
            last_submission: pending_submission,
        };
        let remaining_vertex_bytes = range.vertex_bytes - vertex_bytes;
        let remaining_index_bytes = range.index_bytes - index_bytes;
        if remaining_vertex_bytes != 0 || remaining_index_bytes != 0 {
            self.mesh_geometry_free_ranges
                .get_mut(&raster_key)
                .expect("range entry remains initialized")
                .push(GuiMeshGeometryResidency {
                    stream: GuiMeshStreamRange {
                        vertex_offset: allocation.stream.vertex_offset + vertex_bytes,
                        index_offset: allocation.stream.index_offset + index_bytes,
                    },
                    vertex_bytes: remaining_vertex_bytes,
                    index_bytes: remaining_index_bytes,
                    last_submission: SubmissionId(0),
                });
        }
        Ok(allocation)
    }

    fn destroy_render_resources(&mut self, gal: &mut VulkanicGal) {
        for (_, resources) in std::mem::take(&mut self.item_rasters) {
            resources.composite.destroy(gal);
            let _ = resources.target.destroy(gal);
        }
        if let Some(pass) = self.cached_pass.take() {
            let _ = gal.destroy(pass.pass);
        }
        let resources = std::mem::take(&mut self.resources);
        for resource in resources.values() {
            for handle in resource.handles_in_destroy_order() {
                let _ = gal.destroy(handle);
            }
        }
        for pipeline in std::mem::take(&mut self.shared_pipelines).into_values() {
            for handle in pipeline.handles_in_destroy_order() {
                let _ = gal.destroy(handle);
            }
        }
        let mesh_rasters = std::mem::take(&mut self.mesh_rasters);
        for resources in mesh_rasters.into_values() {
            resources.destroy_asset_resources(gal);
        }
        // Mesh raster resource sets also reference these images/samplers.
        // Release every consuming set before its owned image: otherwise GAL
        // correctly rejects destruction and taking the map would orphan it.
        for texture in std::mem::take(&mut self.dynamic_textures).into_values() {
            for handle in [
                texture.texture_view,
                texture.linear_sampler,
                texture.nearest_sampler,
                texture.texture,
                texture.upload_buffer,
            ] {
                let _ = gal.destroy(handle);
            }
        }
        for program in std::mem::take(&mut self.mesh_shared_programs).into_values() {
            program.destroy(gal);
        }
        self.mesh_geometry_cache.clear();
        self.mesh_geometry_free_ranges.clear();
        let mesh_composites = std::mem::take(&mut self.mesh_composites);
        for resources in mesh_composites.into_values() {
            resources.destroy(gal);
        }
        self.discard_prepared_post_effects(gal);
        self.mesh_targets.clear(gal);
        self.atlases.clear();
    }

    /// A cancelled transaction cannot confirm the uploads/layouts recorded
    /// while building post-effect commands. Retire their private resources
    /// through GAL so the next frame starts from new, explicitly undefined
    /// images rather than guessing which part of a failed frame executed.
    pub(crate) fn discard_prepared_post_effects(&mut self, gal: &mut VulkanicGal) {
        self.clear_frame_pass(gal);
        if let Some(resources) = self.blur_resources.take() {
            for handle in resources.handles_in_destroy_order() {
                let _ = gal.destroy(handle);
            }
        }
        for resources in std::mem::take(&mut self.custom_post_effect_resources) {
            for handle in resources.handles_in_destroy_order() {
                let _ = gal.destroy(handle);
            }
        }
        for intermediate in std::mem::take(&mut self.custom_post_effect_intermediates).into_values()
        {
            for handle in intermediate.handles_in_destroy_order() {
                let _ = gal.destroy(handle);
            }
        }
        if let Some(resources) = self.custom_post_effect_depth_target.take() {
            for handle in resources.handles_in_destroy_order() {
                let _ = gal.destroy(handle);
            }
        }
        self.blur_snapshot_initialized = false;
        self.custom_post_effect_snapshot_initialized.clear();
        self.custom_post_effect_image_initialized.clear();
        self.creeper_intermediate_initialized = false;
        self.spider_initialized = [false; 4];
    }

    pub fn apply_asset_update(
        &mut self,
        gal: &mut VulkanicGal,
        generation: u64,
        payloads: Vec<GuiAssetPayload>,
    ) -> GalResult<()> {
        if generation == 0 {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "GUI asset generation must be non-zero",
            ));
        }
        if generation <= self.asset_generation {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                format!(
                    "stale GUI asset generation {generation}; current generation is {}",
                    self.asset_generation
                ),
            ));
        }
        let mut overrides = BTreeMap::new();
        for payload in payloads {
            let def = sprite_def(payload.sprite_id)?;
            if payload.png_bytes.is_empty() {
                continue;
            }
            if overrides.contains_key(&payload.sprite_id) {
                return Err(GalError::ffi(
                    StatusCode::InvalidArgument,
                    format!(
                        "duplicate GUI asset payload for sprite id {}",
                        payload.sprite_id
                    ),
                ));
            }
            let _ = decode_sprite_bytes(def, &payload.png_bytes)?;
            overrides.insert(payload.sprite_id, payload.png_bytes);
        }
        for def in SPRITES {
            // Rust-owned procedural sprites have no vanilla pack payload. They
            // are synthesized by `load_sprite` and must not participate in
            // bundled-asset validation or override generation checks.
            if def.id == GUI_POST_EFFECT_INVERT_ID
                || def.id == GUI_POST_EFFECT_CREEPER_ID
                || def.id == GUI_POST_EFFECT_SPIDER_ID
            {
                continue;
            }
            let bytes = overrides
                .get(&def.id)
                .map(Vec::as_slice)
                .or_else(|| bundled_sprite_bytes(def.path))
                .ok_or_else(|| {
                    GalError::backend(format!("missing bundled GUI sprite '{}'", def.path))
                })?;
            let _ = decode_sprite_bytes(def, bytes)?;
        }
        self.asset_generation = generation;
        self.asset_overrides = overrides;
        self.destroy_render_resources(gal);
        Ok(())
    }

    /// Replaces the complete dynamic GUI image generation atomically. The caller
    /// retains no ownership after this returns; malformed images leave the last
    /// valid generation intact.
    pub fn apply_raw_image_update(
        &mut self,
        gal: &mut VulkanicGal,
        generation: u64,
        payloads: Vec<GuiRawImageAssetPayload>,
    ) -> GalResult<()> {
        if generation == 0 || generation <= self.raw_image_generation {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                format!(
                    "stale raw GUI image generation {generation}; current generation is {}",
                    self.raw_image_generation
                ),
            ));
        }
        if payloads.len() > GUI_MAX_RAW_IMAGES {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                format!(
                    "raw GUI image payload count {} exceeds bounded limit {GUI_MAX_RAW_IMAGES}",
                    payloads.len()
                ),
            ));
        }
        let mut images = BTreeMap::new();
        let mut total_bytes = 0usize;
        for payload in payloads {
            if payload.asset_id == 0
                || payload.width == 0
                || payload.height == 0
                || payload.width > 8192
                || payload.height > 8192
            {
                return Err(GalError::ffi(
                    StatusCode::InvalidArgument,
                    format!(
                        "invalid raw GUI image {} dimensions {}x{}",
                        payload.asset_id, payload.width, payload.height
                    ),
                ));
            }
            let pixel_count = (payload.width as usize)
                .checked_mul(payload.height as usize)
                .ok_or_else(|| {
                    GalError::ffi(
                        StatusCode::InvalidArgument,
                        "raw GUI image pixel count overflows",
                    )
                })?;
            if pixel_count > GUI_MAX_RAW_IMAGE_PIXELS {
                return Err(GalError::ffi(
                    StatusCode::InvalidArgument,
                    format!(
                        "raw GUI image {} has {pixel_count} pixels; maximum is {GUI_MAX_RAW_IMAGE_PIXELS}",
                        payload.asset_id
                    ),
                ));
            }
            let expected = (payload.width as usize)
                .checked_mul(payload.height as usize)
                .and_then(|pixels| pixels.checked_mul(payload.format.bytes_per_pixel()))
                .ok_or_else(|| {
                    GalError::ffi(StatusCode::InvalidArgument, "raw GUI image size overflows")
                })?;
            if payload.pixels.len() != expected {
                return Err(GalError::ffi(
                    StatusCode::InvalidArgument,
                    format!(
                        "raw GUI image {} has {} bytes; expected {expected}",
                        payload.asset_id,
                        payload.pixels.len()
                    ),
                ));
            }
            total_bytes = total_bytes.checked_add(expected).ok_or_else(|| {
                GalError::ffi(
                    StatusCode::InvalidArgument,
                    "raw GUI image aggregate byte count overflows",
                )
            })?;
            if total_bytes > GUI_MAX_RAW_IMAGE_BYTES_TOTAL {
                return Err(GalError::ffi(
                    StatusCode::InvalidArgument,
                    format!(
                        "raw GUI image aggregate bytes {total_bytes} exceed bounded limit {GUI_MAX_RAW_IMAGE_BYTES_TOTAL}"
                    ),
                ));
            }
            if images
                .insert(
                    payload.asset_id,
                    RawGuiImage {
                        sampling: payload.sampling,
                        format: payload.format,
                        width: payload.width,
                        height: payload.height,
                        pixels: payload.pixels,
                    },
                )
                .is_some()
            {
                return Err(GalError::ffi(
                    StatusCode::InvalidArgument,
                    "duplicate raw GUI image asset id",
                ));
            }
        }
        if images.keys().any(|id| self.atlas_references.contains(*id)) {
            return Err(GalError::ffi(StatusCode::InvalidArgument, "raw GUI image collides with explicit atlas reference"));
        }
        let changed_assets = self.changed_raw_image_assets(&images);
        self.destroy_dynamic_resources_for_assets(gal, &changed_assets);
        self.raw_images = images;
        self.raw_image_generation = generation;
        Ok(())
    }

    /// Private declaration prerequisite. No FFI or renderer capability admits
    /// sampling these references until native view/lifetime lowering is complete.
    #[cfg(test)]
    pub(crate) fn stage_atlas_references(&mut self, revision: u64, references: &[GuiAtlasReference],
        accepted: impl FnMut(u32) -> Option<AcceptedAtlasIncarnation>) -> GalResult<()> {
        if !self.atlas_views.is_empty() {
            return Err(GalError::invalid_argument("resident atlas views require owner-coordinated staging"));
        }
        self.atlas_references.replace(revision, references, &self.raw_images.keys().copied().collect(), accepted)
    }

    /// Context-owned staging path: declarations cannot certify their own acceptance.
    pub(crate) fn stage_owned_atlas_references(&mut self, gal: &mut VulkanicGal,
        world: &super::world_primitive_frontend::WorldPrimitiveFrontend,
        revision: u64, references: &[GuiAtlasReference]) -> GalResult<()> {
        let mut next = self.atlas_references.clone();
        next.replace(revision, references, &self.raw_images.keys().copied().collect(),
            |id| world.accepted_gui_atlas_incarnation(id))?;
        let changed = self.atlas_views.iter().filter_map(|(asset, (incarnation, _))| {
            let retained = next.resolve(*asset, |id| world.accepted_gui_atlas_incarnation(id))
                .is_ok_and(|reference| reference.atlas == *incarnation);
            (!retained).then_some(*asset)
        }).collect();
        self.retire_atlas_views_for_assets(gal, &changed)?;
        self.atlas_references = next;
        Ok(())
    }

    /// Bounded, private preparation only. Native draw binding remains unadmitted.
    pub(crate) fn prepare_owned_atlas_view(&mut self, gal: &mut VulkanicGal,
        world: &mut super::world_primitive_frontend::WorldPrimitiveFrontend, asset: u64) -> GalResult<Handle> {
        let reference = self.atlas_references.resolve(asset, |id| world.accepted_gui_atlas_incarnation(id))?;
        if let Some((incarnation, view)) = self.atlas_views.get(&asset) {
            if *incarnation != reference.atlas {
                return Err(GalError::ffi(StatusCode::StaleHandle, "GUI atlas view was not invalidated before replacement"));
            }
            return Ok(*view);
        }
        if self.atlas_views.len() >= super::gui_atlas_reference::MAX_GUI_ATLAS_REFERENCES {
            return Err(GalError::invalid_argument("GUI atlas view residency bound exceeded"));
        }
        let view = world.create_gui_atlas_view(gal, reference)?;
        self.atlas_views.insert(asset, (reference.atlas, view));
        Ok(view)
    }

    /// Called before the same context replaces owner textures. Keep declarations
    /// so failed publication can retry; consumption still checks exact incarnation.
    pub(crate) fn invalidate_atlas_texture_views(&mut self, gal: &mut VulkanicGal,
        textures: impl IntoIterator<Item = u32>) -> GalResult<()> {
        if self.atlas_views.is_empty() { return Ok(()); }
        let textures: BTreeSet<_> = textures.into_iter().collect();
        let assets = self.atlas_views.iter().filter_map(|(asset, (incarnation, _))|
            textures.contains(&incarnation.texture_id).then_some(*asset)).collect();
        self.retire_atlas_views_for_assets(gal, &assets)
    }

    /// Private native binding preparation. No Java/FFI capability admits this
    /// path yet. The owner's explicit upload prepares the image; this binding
    /// allocates no image, copied pixels, or texture-upload buffer.
    pub(crate) fn prepare_owned_atlas_binding(&mut self, gal: &mut VulkanicGal,
        world: &mut super::world_primitive_frontend::WorldPrimitiveFrontend,
        asset: u64, sampling: SamplerFilter, color: ColorFormat,
        depth: Option<TextureFormat>) -> GalResult<()> {
        let group = match sampling {
            SamplerFilter::Nearest => TextureGroup::Dynamic(asset),
            SamplerFilter::Linear => TextureGroup::DynamicLinear(asset),
        };
        self.prepare_owned_atlas_binding_group(gal, world, group, color, depth)
    }

    fn prepare_owned_atlas_binding_group(&mut self, gal: &mut VulkanicGal,
        world: &mut super::world_primitive_frontend::WorldPrimitiveFrontend,
        group: TextureGroup, color: ColorFormat, depth: Option<TextureFormat>) -> GalResult<()> {
        let asset = TextureGroupKey::from(group).dynamic_asset_id()
            .ok_or_else(|| GalError::invalid_argument("GUI atlas binding needs a semantic image identity"))?;
        let sampling = group.sampling();
        let view = self.prepare_owned_atlas_view(gal, world, asset)?;
        let key = ResourceKey::new(group, color, depth);
        if let Some(binding) = self.resources.get(&key) {
            if !matches!(binding.image_ownership, GuiImageOwnership::AtlasView)
                || binding.texture_view != view {
                return Err(GalError::invalid_argument("GUI atlas binding ownership mismatch"));
            }
            return Ok(());
        }
        let texture = gal.texture_view_info(view)?.texture;
        let pipeline = self.ensure_shared_pipeline(gal, group, color, depth)?;
        let mut created = Vec::new();
        let result = (|| -> GalResult<GuiResources> {
            let index_buffer = gal.create_buffer(BufferDesc {
                label: format!("gui-atlas-{asset}.index"), size: index_bytes().len() as u64,
                memory: MemoryDomain::Upload,
                usages: vec![BufferUsage::Index, BufferUsage::TransferDst, BufferUsage::HostWrite],
            })?;
            created.push(index_buffer);
            let uniform_buffer = gal.create_buffer(BufferDesc {
                label: format!("gui-atlas-{asset}.uniform"), size: GUI_PACKED_UNIFORM_BYTES,
                memory: MemoryDomain::Upload,
                usages: vec![BufferUsage::Uniform, BufferUsage::TransferDst, BufferUsage::HostWrite],
            })?;
            created.push(uniform_buffer);
            let sampler = gal.create_sampler(SamplerDesc {
                label: format!("gui-atlas-{asset}.sampler"), min_filter: sampling,
                mag_filter: sampling, mip_filter: SamplerFilter::Nearest,
                address_u: SamplerAddressMode::ClampToEdge,
                address_v: SamplerAddressMode::ClampToEdge,
                address_w: SamplerAddressMode::ClampToEdge, comparison: None,
            })?;
            created.push(sampler);
            let resource_set = gal.create_resource_set(ResourceSetDesc {
                label: format!("gui-atlas-{asset}.set"), layout: pipeline.resource_layout,
                bindings: [(uniform_buffer, ResourceBindingKind::UniformBuffer),
                    (view, ResourceBindingKind::SampledTexture), (sampler, ResourceBindingKind::Sampler)]
                    .into_iter().enumerate().map(|(binding, (resource, kind))| ResourceBinding {
                        binding: binding as u32, array_index: 0, resource, kind,
                        access: AccessFlags::READ, dynamic_offsets: Vec::new(), buffer_range: None,
                    }).collect(),
            })?;
            created.push(resource_set);
            gal.submit(SubmissionBatch {
                label: format!("gui-atlas-{asset}.index-upload"),
                command_lists: vec![CommandList::from(CommandListDesc {
                    label: format!("gui-atlas-{asset}.index-upload.commands"),
                    operations: gui_index_upload_ops(index_buffer),
                })],
            })?;
            Ok(GuiResources { private_sampler: None, index_buffer, uniform_buffer, texture, sampler,
                texture_view: view, resource_set, pipeline_layout: pipeline.pipeline_layout,
                pipeline: pipeline.pipeline, image_ownership: GuiImageOwnership::AtlasView })
        })();
        match result {
            Ok(binding) => { self.resources.insert(key, binding); Ok(()) },
            Err(error) => {
                for handle in created.into_iter().rev() { let _ = gal.destroy(handle); }
                Err(error)
            },
        }
    }

    /// Private semantic atlas command consumer. It appends to the caller's GAL
    /// pass; it owns no target or presenter. Public mixed GUI admission remains
    /// disabled until its scheduler and FFI supply these typed references.
    pub(crate) fn append_owned_atlas_quads(&mut self, gal: &mut VulkanicGal,
        world: &mut super::world_primitive_frontend::WorldPrimitiveFrontend,
        frame_pass: Handle, target: Handle, color_view: Handle, depth_view: Option<Handle>,
        color: ColorFormat, depth: Option<TextureFormat>, pre_present_y_flip: bool,
        requests: &[GuiAffineQuadRequest]) -> GalResult<Vec<CommandOp>> {
        if requests.len() > GUI_MAX_EXPANDED_AFFINE_QUADS {
            return Err(GalError::invalid_argument("GUI atlas quad count exceeds frame bound"));
        }
        validate_gui_frame_sequences(&[], requests, &[], &[])?;
        let mut ordered = requests.to_vec();
        ordered.sort_by_key(|request| (request.stratum, request.sequence));
        self.append_scheduled_owned_atlas_quads(gal, world, frame_pass, target, color_view,
            depth_view, color, depth, pre_present_y_flip, &ordered, false)
    }

    /// Private item-local raster stage. The target and source atlas are both
    /// Rust GAL resources. No presentation or CPU pixel round-trip occurs.
    /// The caller owns target initialization, transitions and composition.
    pub(crate) fn append_owned_item_raster_quads(&mut self, gal: &mut VulkanicGal,
        world: &mut super::world_primitive_frontend::WorldPrimitiveFrontend,
        frame_pass: Handle, target: Handle, color_view: Handle,
        requests: &[(super::gui_item_raster::GuiItemRasterPlacement, GuiAffineQuadRequest)])
        -> GalResult<Vec<CommandOp>> {
        if requests.len() > GUI_MAX_RAW_IMAGES {
            return Err(GalError::invalid_argument("GUI item raster count exceeds bounded limit"));
        }
        let extent = gal.pass_target_extent(target)?;
        let quads = requests.iter().map(|(placement, local)| {
            if placement.target_extent != [extent.width, extent.height] {
                return Err(GalError::invalid_argument("GUI item raster target extent mismatch"));
            }
            placement.lower_quad(local)
        }).collect::<GalResult<Vec<_>>>()?;
        validate_gui_frame_sequences(&[], &quads, &[], &[])?;
        self.append_scheduled_owned_atlas_quads(gal, world, frame_pass, target, color_view,
            None, ColorFormat::Rgba8Unorm, None, false, &quads, true)
    }

    /// Only the parent-validated scheduler (or the validating wrapper above)
    /// calls this encoder. Expanded tile children legitimately share a parent
    /// sequence; they must not be reclassified as duplicate parent commands.
    fn append_scheduled_owned_atlas_quads(&mut self, gal: &mut VulkanicGal,
        world: &mut super::world_primitive_frontend::WorldPrimitiveFrontend,
        frame_pass: Handle, target: Handle, color_view: Handle, depth_view: Option<Handle>,
        color: ColorFormat, depth: Option<TextureFormat>, pre_present_y_flip: bool,
        requests: &[GuiAffineQuadRequest], item_local_oriented_uv: bool) -> GalResult<Vec<CommandOp>> {
        if requests.len() > GUI_MAX_EXPANDED_AFFINE_QUADS {
            return Err(GalError::invalid_argument("GUI atlas quad count exceeds frame bound"));
        }
        if requests.is_empty() { return Ok(Vec::new()); }
        world.require_gui_atlas_upload_boundary()?;
        let mut batches = Vec::new();
        // Validate the complete semantic input before allocating GPU bindings.
        for request in requests {
            if item_local_oriented_uv {
                // Ordered item-local quad vertices may run either direction
                // through the same bounded UV interval. Validate its bounds
                // without replacing the authored order used for rasterization.
                let mut bounds = request.clone();
                if ![bounds.u0,bounds.u1,bounds.v0,bounds.v1].iter().all(|v| v.is_finite()) {
                    return Err(GalError::invalid_argument("non-finite item raster UV"));
                }
                if bounds.u0 > bounds.u1 { std::mem::swap(&mut bounds.u0, &mut bounds.u1); }
                if bounds.v0 > bounds.v1 { std::mem::swap(&mut bounds.v0, &mut bounds.v1); }
                validate_affine_quad(&bounds)?;
            } else {
                validate_affine_quad(request)?;
            }
            let reference = self.atlas_references.resolve(request.asset_id,
                |id| world.accepted_gui_atlas_incarnation(id))?;
            let low = reference.atlas_uv([request.u0, request.v0])?;
            let high = reference.atlas_uv([request.u1, request.v1])?;
            let uv = [low[0], low[1], high[0] - low[0], high[1] - low[1]];
            if uv.iter().any(|value| !value.is_finite()) {
                return Err(GalError::invalid_argument("GUI atlas UV interval overflows"));
            }
            append_gui_quad(&mut batches, request.stratum,
                if item_local_oriented_uv && request.material.is_cutout() { TextureGroup::DynamicItemCutout(request.asset_id) }
                else if item_local_oriented_uv { TextureGroup::DynamicItemRaster(request.asset_id) }
                else { dynamic_texture_group(request.stratum, request.asset_id) }, PackedGuiQuad {
                    origin: [request.x0, request.y0],
                    axis_u: [request.x1 - request.x0, request.y1 - request.y0],
                    axis_v: [request.x3 - request.x0, request.y3 - request.y0],
                    viewport: request.projection_extent,
                    clip: [request.clip_left as f32, request.clip_top as f32,
                        (request.clip_left + request.clip_width) as f32,
                        (request.clip_top + request.clip_height) as f32],
                    clip_enabled: request.clip_mode == 1, pre_present_y_flip,
                    uv,
                    color: request.material.color(argb_to_rgba(request.color_argb))?,
                    texture_mode: GuiRawImageFormat::Rgba8.shader_mode(), z: request.z,
                });
        }
        for batch in &batches {
            self.prepare_owned_atlas_binding_group(gal, world, batch.group, color, depth)?;
        }
        let mut ops = Vec::new();
        append_gui_batches_ops(self, frame_pass, target, color_view, depth_view,
            color, depth, &batches, &mut ops)?;
        Ok(ops)
    }

    fn retire_atlas_views_for_assets(&mut self, gal: &mut VulkanicGal, assets: &BTreeSet<u64>) -> GalResult<()> {
        // Descriptor sets must release their view dependencies before the views.
        self.destroy_dynamic_resources_for_assets(gal, assets);
        for asset in assets {
            if let Some((_, view)) = self.atlas_views.get(asset) {
                gal.destroy(*view)?;
                // A failed destroy retains its ownership record for retry.
                self.atlas_views.remove(asset);
            }
        }
        Ok(())
    }

    fn changed_raw_image_assets(&self, next: &BTreeMap<u64, RawGuiImage>) -> BTreeSet<u64> {
        self.raw_images
            .keys()
            .chain(next.keys())
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter(|asset_id| self.raw_images.get(asset_id) != next.get(asset_id))
            .collect()
    }

    fn destroy_dynamic_resources_for_assets(
        &mut self,
        gal: &mut VulkanicGal,
        asset_ids: &BTreeSet<u64>,
    ) {
        let keys: Vec<_> = self
            .resources
            .keys()
            .copied()
            .filter(|key| {
                key.group
                    .dynamic_asset_id()
                    .is_some_and(|asset_id| asset_ids.contains(&asset_id))
            })
            .collect();
        let mut texture_keys = BTreeSet::new();
        for key in keys {
            if let Some(resource) = self.resources.remove(&key) {
                if let GuiImageOwnership::SharedRaw { key } = resource.image_ownership {
                    texture_keys.insert(key);
                }
                for handle in resource.handles_in_destroy_order() {
                    let _ = gal.destroy(handle);
                }
            }
        }
        // Partial image replacement has the same dependency order as full
        // teardown: mesh descriptor sets must release their sampled views
        // and samplers before the shared texture ownership records are removed.
        let mesh_rasters = std::mem::take(&mut self.mesh_rasters);
        for (key, resources) in mesh_rasters {
            if asset_ids.contains(&key.asset_id) {
                resources.destroy_asset_resources(gal);
            } else {
                self.mesh_rasters.insert(key, resources);
            }
        }
        for texture_key in texture_keys {
            if let Some(texture) = self.dynamic_textures.remove(&texture_key) {
                for handle in [
                    texture.texture_view,
                    texture.linear_sampler,
                    texture.nearest_sampler,
                    texture.texture,
                    texture.upload_buffer,
                ] {
                    let _ = gal.destroy(handle);
                }
            }
        }
        self.mesh_geometry_cache
            .retain(|(key, _, _), _| !asset_ids.contains(&key.asset_id));
        self.mesh_geometry_free_ranges
            .retain(|key, _| !asset_ids.contains(&key.asset_id));
    }

    fn ensure_mesh_shared_program(
        &mut self,
        gal: &mut VulkanicGal,
        color_format: TextureFormat,
        depth_format: Option<TextureFormat>,
        material_mode: GuiMeshMaterialMode,
        front_face: super::resources::FrontFace,
    ) -> GalResult<GuiMeshSharedProgram> {
        let key = GuiMeshSharedProgramKey {
            color_format,
            depth_format,
            material_mode,
            front_face,
        };
        if let Some(program) = self.mesh_shared_programs.get(&key) {
            return Ok(*program);
        }
        if self.mesh_shared_programs.len() >= GUI_MAX_MESH_SHARED_PROGRAMS {
            return Err(GalError::unsupported_feature(format!(
                "GUI mesh shared-program cache exceeds bounded limit {}",
                GUI_MAX_MESH_SHARED_PROGRAMS
            )));
        }
        let program = GuiMeshSharedProgram::create(
            gal,
            &format!("minecraft.gui.mesh.program.{material_mode:?}.{front_face:?}"),
            color_format,
            depth_format,
            material_mode,
            front_face,
        )?;
        self.mesh_shared_programs.insert(key, program);
        Ok(program)
    }

    pub fn clear_frame_pass(&mut self, gal: &mut VulkanicGal) {
        if let Some(pass) = self.cached_pass.take() {
            let _ = gal.destroy(pass.pass);
        }
    }

    pub fn clear_frame_passes_for_targets(&mut self, gal: &mut VulkanicGal, targets: &[Handle]) {
        let Some(pass) = self.cached_pass else {
            return;
        };
        if targets.contains(&pass.frame_target) {
            self.cached_pass = None;
            let _ = gal.destroy(pass.pass);
        }
    }

    pub fn submit_frame(
        &mut self,
        gal: &mut VulkanicGal,
        generation: u64,
        frame_target: Handle,
        requests: Vec<GuiSpriteRequest>,
    ) -> GalResult<GuiSubmitStats> {
        self.submit_frame_with_affine_quads(gal, generation, frame_target, requests, Vec::new())
    }

    pub fn submit_frame_with_affine_quads(
        &mut self,
        gal: &mut VulkanicGal,
        generation: u64,
        frame_target: Handle,
        requests: Vec<GuiSpriteRequest>,
        affine_quads: Vec<GuiAffineQuadRequest>,
    ) -> GalResult<GuiSubmitStats> {
        self.submit_frame_with_affine_quads_and_mesh_batches(
            gal,
            generation,
            frame_target,
            requests,
            affine_quads,
            Vec::new(),
        )
    }

    pub fn submit_frame_with_affine_quads_and_mesh_batches(
        &mut self,
        gal: &mut VulkanicGal,
        generation: u64,
        frame_target: Handle,
        requests: Vec<GuiSpriteRequest>,
        affine_quads: Vec<GuiAffineQuadRequest>,
        mesh_batches: Vec<GuiMeshBatchRequest>,
    ) -> GalResult<GuiSubmitStats> {
        self.submit_frame_with_tiled_quads(gal, generation, frame_target,
            requests, affine_quads, mesh_batches, Vec::new())
    }

    pub(crate) fn submit_frame_with_tiled_quads(
        &mut self, gal: &mut VulkanicGal, generation: u64, frame_target: Handle,
        requests: Vec<GuiSpriteRequest>, affine_quads: Vec<GuiAffineQuadRequest>,
        mesh_batches: Vec<GuiMeshBatchRequest>, tiled_quads: Vec<GuiTiledQuadRequest>,
    ) -> GalResult<GuiSubmitStats> {
        self.submit_frame_with_owned_atlases(gal, None, generation, frame_target,
            requests, affine_quads, mesh_batches, tiled_quads)
    }

    pub(crate) fn submit_frame_with_owned_atlases(
        &mut self, gal: &mut VulkanicGal,
        world: Option<&mut super::world_primitive_frontend::WorldPrimitiveFrontend>,
        generation: u64, frame_target: Handle,
        requests: Vec<GuiSpriteRequest>, affine_quads: Vec<GuiAffineQuadRequest>,
        mesh_batches: Vec<GuiMeshBatchRequest>, tiled_quads: Vec<GuiTiledQuadRequest>,
    ) -> GalResult<GuiSubmitStats> {
        validate_gui_frame_sequences(&requests, &affine_quads, &mesh_batches, &tiled_quads)?;
        let (ops, mut stats) = self.append_frame_ops_with_owned_atlases_to_target(
            gal,
            world,
            generation,
            frame_target,
            frame_target,
            None,
            None,
            None,
            false,
            requests,
            affine_quads,
            mesh_batches,
            tiled_quads,
        )?;
        let token = gal.submit(SubmissionBatch {
            label: "minecraft.gui.frame".to_string(),
            command_lists: vec![CommandList::from(CommandListDesc {
                label: "minecraft.gui.frame.commands".to_string(),
                operations: ops,
            })],
        })?;
        stats.submission_id = token.submission.0;
        Ok(stats)
    }

    pub fn append_frame_ops(
        &mut self,
        gal: &mut VulkanicGal,
        generation: u64,
        frame_target: Handle,
        requests: Vec<GuiSpriteRequest>,
    ) -> GalResult<(Vec<CommandOp>, GuiSubmitStats)> {
        self.append_frame_ops_with_affine_quads(gal, generation, frame_target, requests, Vec::new())
    }

    pub fn append_frame_ops_with_affine_quads(
        &mut self,
        gal: &mut VulkanicGal,
        generation: u64,
        frame_target: Handle,
        requests: Vec<GuiSpriteRequest>,
        affine_quads: Vec<GuiAffineQuadRequest>,
    ) -> GalResult<(Vec<CommandOp>, GuiSubmitStats)> {
        self.append_frame_ops_with_affine_quads_to_target(
            gal,
            generation,
            frame_target,
            frame_target,
            None,
            None,
            None,
            false,
            requests,
            affine_quads,
        )
    }

    /// Records GUI work against an explicit GAL render target and its color
    /// attachment. The ordinary whole-frame route uses the acquired target for
    /// both values; source-owned shader frames provide their overlay target
    /// and view so GUI joins the final Rust composition before presentation.
    pub fn append_frame_ops_with_affine_quads_to_target(
        &mut self,
        gal: &mut VulkanicGal,
        generation: u64,
        render_target: Handle,
        color_attachment: Handle,
        render_pass: Option<Handle>,
        depth_attachment: Option<Handle>,
        depth_format: Option<TextureFormat>,
        pre_present_y_flip: bool,
        requests: Vec<GuiSpriteRequest>,
        affine_quads: Vec<GuiAffineQuadRequest>,
    ) -> GalResult<(Vec<CommandOp>, GuiSubmitStats)> {
        let needs_depth = affine_quads
            .iter()
            .any(|request| request.stratum == GUI_LEQUAL_DEPTH_BLIT_STRATUM);
        let infer_depth = needs_depth && depth_attachment.is_none();
        let (depth_attachment, depth_format) = if needs_depth && depth_attachment.is_none() {
            let Some((_, view)) = gal.pass_target_depth_attachment(render_target)? else {
                return Err(GalError::backend(
                    "depth-tested GUI semantic blit requires the Rust-owned frame depth attachment",
                ));
            };
            (Some(view), Some(TextureFormat::Depth32Float))
        } else {
            (depth_attachment, depth_format)
        };
        let render_pass = if infer_depth { None } else { render_pass };
        if generation != self.generation {
            self.destroy_render_resources(gal);
            self.generation = generation;
        }
        if depth_attachment.is_some() != depth_format.is_some() {
            return Err(GalError::command(
                StatusCode::InvalidArgument,
                "GUI depth attachment and depth format must be supplied together",
            ));
        }
        if render_target.kind() == Some(super::handles::HandleKind::FrameTarget) {
            if let Some(depth_attachment) = depth_attachment {
                let (_, owned_view) = gal.frame_target_owned_depth_attachment(render_target)?;
                if depth_attachment != owned_view {
                    return Err(GalError::command(
                        StatusCode::InvalidArgument,
                        "frame-target GUI depth attachment must be the Rust-owned frame depth view",
                    ));
                }
            }
        }
        let color_format = gal.pass_target_color_format(render_target)?;
        let frame_pass = match render_pass {
            Some(pass) => pass,
            None => self.frame_pass(gal, render_target, depth_format)?,
        };
        let mut batches: Vec<GuiBatch> = Vec::new();
        let mut stats = GuiSubmitStats {
            sprite_count: requests.len() as u64,
            affine_quad_count: affine_quads.len() as u64,
            ..GuiSubmitStats::default()
        };
        let ordered = order_gui_requests(requests, affine_quads)?;
        for request in ordered {
            match request {
                GuiFrameRequest::Sprite(request) => {
                    let def = sprite_def(request.sprite_id)?;
                    if request.stratum != def.stratum {
                        return Err(GalError::ffi(
                            StatusCode::InvalidArgument,
                            format!(
                                "GUI sprite '{}' requested stratum {} but registry stratum is {}",
                                def.name, request.stratum, def.stratum
                            ),
                        ));
                    }
                    validate_request(&request, def)?;
                    let group = def.group;
                    self.ensure_resources(gal, group, color_format, depth_format, &mut stats)?;
                    let quad = self.pack_sprite(&request, def, pre_present_y_flip)?;
                    append_gui_quad(&mut batches, request.stratum, group, quad);
                }
                GuiFrameRequest::AffineBatch(requests) => {
                    for request in requests {
                        let image_format = self
                            .raw_images
                            .get(&request.asset_id)
                            .ok_or_else(|| {
                                GalError::ffi(
                                    StatusCode::InvalidArgument,
                                    format!("unknown raw GUI image asset {}", request.asset_id),
                                )
                            })?
                            .format;
                        validate_affine_quad(&request)?;
                        let group = dynamic_texture_group(request.stratum, request.asset_id);
                        self.ensure_resources(gal, group, color_format, depth_format, &mut stats)?;
                        let quad = PackedGuiQuad {
                            origin: [request.x0, request.y0],
                            axis_u: [request.x1 - request.x0, request.y1 - request.y0],
                            axis_v: [request.x3 - request.x0, request.y3 - request.y0],
                            viewport: request.projection_extent,
                            clip: [
                                request.clip_left as f32,
                                request.clip_top as f32,
                                (request.clip_left + request.clip_width) as f32,
                                (request.clip_top + request.clip_height) as f32,
                            ],
                            clip_enabled: request.clip_mode == 1,
                            pre_present_y_flip,
                            uv: [
                                request.u0,
                                request.v0,
                                request.u1 - request.u0,
                                request.v1 - request.v0,
                            ],
                            color: request.material.color(argb_to_rgba(request.color_argb))?,
                            texture_mode: image_format.shader_mode(),
                            z: request.z,
                        };
                        append_gui_quad(&mut batches, request.stratum, group, quad);
                    }
                }
                GuiFrameRequest::Affine(_) => unreachable!("ordered affine requests are coalesced"),
                GuiFrameRequest::Mesh(_) => {
                    return Err(GalError::backend(
                        "mesh GUI request reached the sprite/affine-only frame builder",
                    ));
                }
            }
        }
        stats.sprite_batch_count = batches.len() as u64;
        let mut ops = Vec::new();
        let whole_frame_vulkan = gal.capabilities().api == BackendApi::Vulkan;
        if whole_frame_vulkan {
            ops.push(CommandOp::BeginPass {
                pass: frame_pass,
                target: render_target,
                colors: vec![loaded_frame_color_attachment(color_attachment)],
                depth_stencil: depth_attachment.map(loaded_frame_depth_attachment),
            });
            ops.push(CommandOp::EndPass);
        } else if batches.is_empty() {
            ops.push(CommandOp::BeginPass {
                pass: frame_pass,
                target: render_target,
                colors: vec![loaded_frame_color_attachment(color_attachment)],
                depth_stencil: depth_attachment.map(loaded_frame_depth_attachment),
            });
            ops.push(CommandOp::EndPass);
        }
        for batch in &batches {
            let resources = self
                .resources
                .get(&ResourceKey::new(batch.group, color_format, depth_format))
                .ok_or_else(|| GalError::backend("GUI resources vanished before submit"))?;
            let uniforms = packed_uniform_bytes(batch)?;
            ops.push(CommandOp::Barrier(buffer_barrier(
                resources.uniform_buffer,
                TextureUsageState::ShaderRead,
                TextureUsageState::TransferDst,
            )));
            ops.push(CommandOp::HostWriteBuffer {
                buffer: resources.uniform_buffer,
                offset: 0,
                data: uniforms,
            });
            ops.push(CommandOp::Barrier(buffer_barrier(
                resources.uniform_buffer,
                TextureUsageState::TransferDst,
                TextureUsageState::ShaderRead,
            )));
            ops.push(CommandOp::BeginPass {
                pass: frame_pass,
                target: render_target,
                colors: vec![loaded_frame_color_attachment(color_attachment)],
                depth_stencil: depth_attachment.map(loaded_frame_depth_attachment),
            });
            ops.push(CommandOp::BindGraphicsPipeline(resources.pipeline));
            ops.push(CommandOp::BindResourceSet {
                pipeline_layout: resources.pipeline_layout,
                set_index: 0,
                set: resources.resource_set,
                dynamic_offsets: Vec::new(),
            });
            ops.push(CommandOp::SetIndexBuffer {
                buffer: resources.index_buffer,
                offset: 0,
                index_type: super::resources::IndexType::U32,
            });
            ops.push(CommandOp::DrawIndexed {
                indices: 6,
                instances: batch.quads.len() as u32,
            });
            ops.push(CommandOp::EndPass);
        }
        stats.command_lists = 1;
        stats.command_ops = ops.len() as u64;
        Ok((ops, stats))
    }

    /// Ordered variant for the private standard-3D item family. Mesh items
    /// split ordinary GUI batches at their scheduler sequence so a 3D PIP
    /// composite can never jump ahead of text or sprites in the same stratum.
    pub(crate) fn append_frame_ops_with_blur_boundary(
        &mut self,
        gal: &mut VulkanicGal,
        generation: u64,
        render_target: Handle,
        color_attachment: Handle,
        requests: Vec<GuiSpriteRequest>,
        affine_quads: Vec<GuiAffineQuadRequest>,
        mesh_batches: Vec<GuiMeshBatchRequest>,
        boundary_stratum: i32,
        blur_radius: i32,
        pre_present_y_flip: bool,
    ) -> GalResult<(Vec<CommandOp>, GuiSubmitStats)> {
        self.append_frame_ops_with_tiled_blur_boundary(gal, generation, render_target,
            color_attachment, requests, affine_quads, mesh_batches, Vec::new(),
            boundary_stratum, blur_radius, pre_present_y_flip)
    }

    pub(crate) fn append_frame_ops_with_tiled_blur_boundary(
        &mut self, gal: &mut VulkanicGal, generation: u64, render_target: Handle,
        color_attachment: Handle, requests: Vec<GuiSpriteRequest>,
        affine_quads: Vec<GuiAffineQuadRequest>, mesh_batches: Vec<GuiMeshBatchRequest>,
        tiled_quads: Vec<GuiTiledQuadRequest>, boundary_stratum: i32, blur_radius: i32,
        pre_present_y_flip: bool,
    ) -> GalResult<(Vec<CommandOp>, GuiSubmitStats)> {
        self.append_frame_ops_with_owned_atlases_and_blur_boundary(gal, None, generation,
            render_target, color_attachment, requests, affine_quads, mesh_batches,
            tiled_quads, boundary_stratum, blur_radius, pre_present_y_flip)
    }

    pub(crate) fn append_frame_ops_with_owned_atlases_and_blur_boundary(
        &mut self, gal: &mut VulkanicGal,
        mut world: Option<&mut super::world_primitive_frontend::WorldPrimitiveFrontend>,
        generation: u64, render_target: Handle, color_attachment: Handle,
        requests: Vec<GuiSpriteRequest>, affine_quads: Vec<GuiAffineQuadRequest>,
        mesh_batches: Vec<GuiMeshBatchRequest>, tiled_quads: Vec<GuiTiledQuadRequest>,
        boundary_stratum: i32, blur_radius: i32, pre_present_y_flip: bool,
    ) -> GalResult<(Vec<CommandOp>, GuiSubmitStats)> {
        // Validate the whole frame before partitioning or creating resources;
        // splitting at blur must not bypass aggregate bounds or hide collisions.
        preflight_tiled_affine_count(&tiled_quads, affine_quads.len())?;
        validate_gui_frame_sequences(&requests, &affine_quads, &mesh_batches, &tiled_quads)?;
        self.preflight_owned_atlas_commands(world.as_deref(), &affine_quads, &tiled_quads)?;
        self.preflight_mesh_atlas_commands(world.as_deref(), &mesh_batches)?;
        let boundary_plan = plan_gui_blur_boundary(boundary_stratum, std::iter::empty())?;
        let threshold = boundary_plan.phase_threshold().map_err(|_| {
            GalError::invalid_argument("GUI blur boundary is outside the bounded semantic range")
        })?;
        if blur_radius < -1 || blur_radius > 64 {
            return Err(GalError::invalid_argument(
                "GUI blur radius must be -1 or within the bounded range 0..=64",
            ));
        }
        let blur_radius = blur_radius.max(0);
        let mut before_requests = Vec::new();
        let mut after_requests = Vec::new();
        for request in requests {
            if request.stratum < threshold {
                before_requests.push(request);
            } else {
                after_requests.push(request);
            }
        }
        let mut before_affine = Vec::new();
        let mut after_affine = Vec::new();
        for request in affine_quads {
            if request.stratum < threshold {
                before_affine.push(request);
            } else {
                after_affine.push(request);
            }
        }
        let mut before_mesh = Vec::new();
        let mut after_mesh = Vec::new();
        for request in mesh_batches {
            if request.stratum < threshold {
                before_mesh.push(request);
            } else {
                after_mesh.push(request);
            }
        }
        let (before_tiled, after_tiled) = tiled_quads.into_iter()
            .partition(|request| request.stratum < threshold);
        let (mut ops, mut stats) = self
            .append_frame_ops_with_owned_atlases_to_target(
                gal,
                world.as_deref_mut(),
                generation,
                render_target,
                color_attachment,
                None,
                None,
                None,
                pre_present_y_flip,
                before_requests,
                before_affine,
                before_mesh,
                before_tiled,
            )?;
        let extent = gal.pass_target_extent(render_target)?;
        let color_format = gal.pass_target_color_format(render_target)?;
        let resources =
            self.ensure_blur_resources(gal, extent.width, extent.height, color_format)?;
        let snapshot = resources.texture;
        let pipeline = resources.pipeline;
        let pipeline_layout = resources.pipeline_layout;
        let resource_set = resources.resource_set;
        let uniform_buffer = resources.uniform_buffer;
        // Reuse the owned fullscreen scratch pool. These images are private
        // GAL resources, not Java targets or borrowed shader-pack state.
        let scratch_textures = resources.spider_textures;
        let scratch_views = resources.spider_views;
        let scratch_targets = resources.spider_targets;
        let scratch_passes = resources.spider_passes;
        let scratch_sets = resources.spider_single_sets;
        let pass = self.frame_pass(gal, render_target, None)?;
        let snapshot_before = if self.blur_snapshot_initialized {
            TextureUsageState::ShaderRead
        } else {
            TextureUsageState::Undefined
        };
        ops.push(CommandOp::Barrier(ResourceBarrier {
            resource: snapshot,
            subresources: None,
            before: snapshot_before,
            after: TextureUsageState::TransferDst,
            src_queue: QueueClass::Graphics,
            dst_queue: QueueClass::Transfer,
        }));
        if render_target.kind() == Some(super::handles::HandleKind::FrameTarget) {
            ops.push(CommandOp::CopyFrameTargetToTexture {
                src: render_target,
                dst: snapshot,
                extent,
            });
        } else {
            let source_texture = gal.pass_target_color_texture(render_target)?;
            ops.extend([
                CommandOp::Barrier(ResourceBarrier {
                    resource: source_texture,
                    subresources: None,
                    before: TextureUsageState::ColorAttachment,
                    after: TextureUsageState::TransferSrc,
                    src_queue: QueueClass::Graphics,
                    dst_queue: QueueClass::Transfer,
                }),
                CommandOp::CopyTexture(TextureImageCopyRegion {
                    row_order: crate::render::vulkanic::commands::TextureRowOrder::Preserve,
                    src_texture: source_texture,
                    src_mip: 0,
                    src_layer: 0,
                    src_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
                    dst_texture: snapshot,
                    dst_mip: 0,
                    dst_layer: 0,
                    dst_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
                    extent,
                }),
                CommandOp::Barrier(ResourceBarrier {
                    resource: source_texture,
                    subresources: None,
                    before: TextureUsageState::TransferSrc,
                    after: TextureUsageState::ColorAttachment,
                    src_queue: QueueClass::Transfer,
                    dst_queue: QueueClass::Graphics,
                }),
            ]);
        }
        ops.push(CommandOp::Barrier(ResourceBarrier {
                resource: snapshot,
                subresources: None,
                before: TextureUsageState::TransferDst,
                after: TextureUsageState::ShaderRead,
                src_queue: QueueClass::Transfer,
                dst_queue: QueueClass::Graphics,
            }));
        // Frozen minecraft:blur is three horizontal/vertical box-blur pairs.
        // Ping-pong owned scratch images, writing the final pass to the frame.
        let mut initialized = self.spider_initialized;
        for index in 0..6 {
            let final_pass = index == 5;
            let output = if index % 2 == 0 { 2 } else { 3 };
            let source_set = if index == 0 { resource_set }
                else if index % 2 == 1 { scratch_sets[1] }
                else { scratch_sets[3] };
            let mut bytes = vec![0u8; 64];
            let direction: [f32; 2] = if index % 2 == 0 { [1.0, 0.0] } else { [0.0, 1.0] };
            bytes[..4].copy_from_slice(&direction[0].to_le_bytes());
            bytes[4..8].copy_from_slice(&direction[1].to_le_bytes());
            bytes[8..12].copy_from_slice(&(blur_radius as f32).to_le_bytes());
            if index > 0 || self.blur_snapshot_initialized {
                ops.push(CommandOp::Barrier(buffer_barrier(
                    uniform_buffer,
                    TextureUsageState::ShaderRead,
                    TextureUsageState::TransferDst,
                )));
            }
            push_uniform_write(&mut ops, uniform_buffer, bytes);
            if !final_pass {
                ops.push(CommandOp::Barrier(ResourceBarrier {
                    resource: scratch_textures[output], subresources: None,
                    before: if initialized[output] { TextureUsageState::ShaderRead } else { TextureUsageState::Undefined },
                    after: TextureUsageState::ColorAttachment,
                    src_queue: QueueClass::Graphics, dst_queue: QueueClass::Graphics,
                }));
            }
            ops.extend([
                CommandOp::BeginPass {
                    pass: if final_pass { pass } else { scratch_passes[output] },
                    target: if final_pass { render_target } else { scratch_targets[output] },
                    colors: vec![if final_pass { loaded_frame_color_attachment(color_attachment) }
                        else { PassAttachment { view: scratch_views[output], load_op: AttachmentLoadOp::DontCare,
                            store_op: AttachmentStoreOp::Store, clear_color: None } }],
                    depth_stencil: None,
                },
                CommandOp::BindGraphicsPipeline(pipeline),
                CommandOp::BindResourceSet { pipeline_layout, set_index: 0, set: source_set, dynamic_offsets: Vec::new() },
                CommandOp::Draw { vertices: 3, instances: 1 },
                CommandOp::EndPass,
            ]);
            if !final_pass {
                ops.push(CommandOp::Barrier(ResourceBarrier {
                    resource: scratch_textures[output], subresources: None,
                    before: TextureUsageState::ColorAttachment, after: TextureUsageState::ShaderRead,
                    src_queue: QueueClass::Graphics, dst_queue: QueueClass::Graphics,
                }));
                initialized[output] = true;
            }
        }
        self.spider_initialized = initialized;
        let (after_ops, after_stats) = self
            .append_frame_ops_with_owned_atlases_to_target(
                gal,
                world,
                generation,
                render_target,
                color_attachment,
                None,
                None,
                None,
                false,
                after_requests,
                after_affine,
                after_mesh,
                after_tiled,
            )?;
        ops.extend(after_ops);
        stats.sprite_count = stats.sprite_count.saturating_add(after_stats.sprite_count);
        stats.affine_quad_count = stats
            .affine_quad_count
            .saturating_add(after_stats.affine_quad_count);
        stats.mesh_item_count = stats
            .mesh_item_count
            .saturating_add(after_stats.mesh_item_count);
        stats.mesh_batch_count = stats
            .mesh_batch_count
            .saturating_add(after_stats.mesh_batch_count);
        stats.mesh_draw_count = stats
            .mesh_draw_count
            .saturating_add(after_stats.mesh_draw_count);
        stats.sprite_batch_count = stats
            .sprite_batch_count
            .saturating_add(after_stats.sprite_batch_count);
        stats.cache_hits = stats.cache_hits.saturating_add(after_stats.cache_hits);
        stats.cache_misses = stats.cache_misses.saturating_add(after_stats.cache_misses);
        stats.resource_creates = stats
            .resource_creates
            .saturating_add(after_stats.resource_creates);
        stats.command_lists = 1;
        stats.command_ops = ops.len() as u64;
        stats
            .owned_intermediate_targets
            .extend(after_stats.owned_intermediate_targets);
        stats
            .transient_diagnostic_passes
            .extend(after_stats.transient_diagnostic_passes);
        self.blur_snapshot_initialized = true;
        Ok((ops, stats))
    }

    /// Lowers the vanilla invert post effect against the acquired Rust target.
    /// The source is copied into the persistent Rust-owned snapshot, then a
    /// fullscreen Rust pipeline writes the inverted result back before GUI
    /// composition. No Java post-chain or GUI fallback is consulted.
    pub(crate) fn append_invert_post_effect(
        &mut self,
        gal: &mut VulkanicGal,
        render_target: Handle,
        color_attachment: Handle,
    ) -> GalResult<Vec<CommandOp>> {
        let extent = gal.pass_target_extent(render_target)?;
        let color_format = gal.pass_target_color_format(render_target)?;
        let resources =
            self.ensure_blur_resources(gal, extent.width, extent.height, color_format)?;
        let snapshot = resources.texture;
        let uniform_buffer = resources.uniform_buffer;
        let pipeline = resources.invert_pipeline;
        let pipeline_layout = resources.pipeline_layout;
        let resource_set = resources.resource_set;
        let pass = self.frame_pass(gal, render_target, None)?;
        let snapshot_before = if self.blur_snapshot_initialized {
            TextureUsageState::ShaderRead
        } else {
            TextureUsageState::Undefined
        };
        let mut ops = vec![
            CommandOp::HostWriteBuffer {
                buffer: uniform_buffer,
                offset: 0,
                data: {
                    let mut bytes = vec![0u8; 16];
                    bytes[..4].copy_from_slice(&0.8f32.to_le_bytes());
                    bytes
                },
            },
            CommandOp::Barrier(buffer_barrier(
                uniform_buffer,
                TextureUsageState::TransferDst,
                TextureUsageState::ShaderRead,
            )),
            CommandOp::Barrier(ResourceBarrier {
                resource: snapshot,
                subresources: None,
                before: snapshot_before,
                after: TextureUsageState::TransferDst,
                src_queue: QueueClass::Graphics,
                dst_queue: QueueClass::Transfer,
            }),
        ];
        if render_target.kind() == Some(super::handles::HandleKind::FrameTarget) {
            ops.push(CommandOp::CopyFrameTargetToTexture {
                src: render_target,
                dst: snapshot,
                extent,
            });
        } else {
            let source_texture = gal.pass_target_color_texture(render_target)?;
            ops.extend([
                CommandOp::Barrier(ResourceBarrier {
                    resource: source_texture,
                    subresources: None,
                    before: TextureUsageState::ColorAttachment,
                    after: TextureUsageState::TransferSrc,
                    src_queue: QueueClass::Graphics,
                    dst_queue: QueueClass::Transfer,
                }),
                CommandOp::CopyTexture(TextureImageCopyRegion {
                    row_order: crate::render::vulkanic::commands::TextureRowOrder::Preserve,
                    src_texture: source_texture,
                    src_mip: 0,
                    src_layer: 0,
                    src_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
                    dst_texture: snapshot,
                    dst_mip: 0,
                    dst_layer: 0,
                    dst_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
                    extent,
                }),
                CommandOp::Barrier(ResourceBarrier {
                    resource: source_texture,
                    subresources: None,
                    before: TextureUsageState::TransferSrc,
                    after: TextureUsageState::ColorAttachment,
                    src_queue: QueueClass::Transfer,
                    dst_queue: QueueClass::Graphics,
                }),
            ]);
        }
        ops.extend([
            CommandOp::Barrier(ResourceBarrier {
                resource: snapshot,
                subresources: None,
                before: TextureUsageState::TransferDst,
                after: TextureUsageState::ShaderRead,
                src_queue: QueueClass::Transfer,
                dst_queue: QueueClass::Graphics,
            }),
            CommandOp::BeginPass {
                pass,
                target: render_target,
                colors: vec![loaded_frame_color_attachment(color_attachment)],
                depth_stencil: None,
            },
            CommandOp::BindGraphicsPipeline(pipeline),
            CommandOp::BindResourceSet {
                pipeline_layout,
                set_index: 0,
                set: resource_set,
                dynamic_offsets: Vec::new(),
            },
            CommandOp::Draw {
                vertices: 3,
                instances: 1,
            },
            CommandOp::EndPass,
        ]);
        self.blur_snapshot_initialized = true;
        Ok(ops)
    }

    fn ensure_custom_post_effect_resources(
        &mut self,
        gal: &mut VulkanicGal,
        identity: &str,
        pass_index: usize,
        width: u32,
        height: u32,
        color_format: ColorFormat,
        vertex_source: &[u8],
        fragment_source: &[u8],
        input_count: usize,
        input_row_order: super::commands::TextureRowOrder,
        needs_frame_scratch: bool,
        input_bilinear: &[bool],
        input_targets: &[String],
        input_images: &[Option<CustomPostEffectImage>],
        input_depth_views: &[Option<Handle>],
        output_target: &str,
        uniform_blocks: &[Vec<u8>],
    ) -> GalResult<&CustomPostEffectResources> {
        if gal.capabilities().api != BackendApi::Vulkan {
            return Err(GalError::unsupported_feature(
                "Rust custom post effects require the Vulkan backend",
            ));
        }
        if pass_index >= MAX_CUSTOM_POST_EFFECT_PASSES {
            return Err(GalError::unsupported_feature(format!(
                "custom post-effect pass index exceeds bounded limit {MAX_CUSTOM_POST_EFFECT_PASSES}"
            )));
        }
        if input_depth_views.len() != input_count || input_bilinear.len() != input_count {
            return Err(GalError::invalid_argument(
                "custom post-effect depth-view/filter count does not match semantic input count",
            ));
        }
        let mut source_fingerprint = 0xcbf29ce484222325u64;
        source_fingerprint ^= input_row_order as u64 | ((needs_frame_scratch as u64) << 8);
        for byte in vertex_source.iter().chain(fragment_source.iter()) {
            source_fingerprint ^= *byte as u64;
            source_fingerprint = source_fingerprint.wrapping_mul(0x100000001b3);
        }
        for block in uniform_blocks {
            // Values are rewritten explicitly on each submission. Only the
            // allocation/layout belongs in pipeline-resource cache identity;
            // frame time must not rebuild the graph every frame.
            for byte in block.len().to_le_bytes() {
                source_fingerprint ^= byte as u64;
                source_fingerprint = source_fingerprint.wrapping_mul(0x100000001b3);
            }
        }
        source_fingerprint ^= input_count as u64;
        source_fingerprint = source_fingerprint.wrapping_mul(0x100000001b3);
        for bilinear in input_bilinear {
            source_fingerprint ^= *bilinear as u64;
            source_fingerprint = source_fingerprint.wrapping_mul(0x100000001b3);
        }
        for target in input_targets
            .iter()
            .chain(std::iter::once(&output_target.to_owned()))
        {
            for byte in target.as_bytes() {
                source_fingerprint ^= *byte as u64;
                source_fingerprint = source_fingerprint.wrapping_mul(0x100000001b3);
            }
        }
        for image in input_images.iter().flatten() {
            for byte in image
                .path
                .as_bytes()
                .iter()
                .chain(image.pixels_rgba8.iter())
            {
                source_fingerprint ^= *byte as u64;
                source_fingerprint = source_fingerprint.wrapping_mul(0x100000001b3);
            }
            source_fingerprint ^= image.bilinear as u64;
            source_fingerprint = source_fingerprint.wrapping_mul(0x100000001b3);
        }
        for view in input_depth_views.iter().flatten() {
            source_fingerprint ^= view.raw();
            source_fingerprint = source_fingerprint.wrapping_mul(0x100000001b3);
        }
        let cache_identity = format!("{identity}#{source_fingerprint:016x}");
        if self
            .custom_post_effect_resources
            .get(pass_index)
            .is_some_and(|resources| {
                resources.identity == cache_identity
                    && resources.width == width
                    && resources.height == height
                    && resources.color_format == color_format
            })
        {
            return Ok(&self.custom_post_effect_resources[pass_index]);
        }
        if pass_index == 0 {
            for previous in std::mem::take(&mut self.custom_post_effect_resources) {
                for handle in previous.handles_in_destroy_order() {
                    let _ = gal.destroy(handle);
                }
            }
            self.custom_post_effect_snapshot_initialized.clear();
            self.custom_post_effect_image_initialized.clear();
        } else if pass_index < self.custom_post_effect_resources.len() {
            // A changed pass invalidates every later pass: their descriptor
            // bindings and intermediate-target assumptions were compiled
            // against the previous graph suffix. Retire the whole suffix so
            // vector indices remain aligned with the current semantic chain.
            let stale_resources = self
                .custom_post_effect_resources
                .drain(pass_index..)
                .collect::<Vec<_>>();
            for previous in stale_resources {
                for handle in previous.handles_in_destroy_order() {
                    let _ = gal.destroy(handle);
                }
            }
            self.custom_post_effect_snapshot_initialized
                .truncate(pass_index);
            self.custom_post_effect_image_initialized
                .truncate(pass_index);
        }
        let label = format!("minecraft.post-effect.{identity}.pass-{pass_index}.{width}x{height}");
        let mut created = Vec::new();
        let result = (|| -> GalResult<CustomPostEffectResources> {
            if input_count == 0 || input_count > 4 {
                return Err(GalError::unsupported_feature(
                    "custom post-effect pass input count must be in the bounded range 1..=4",
                ));
            }
            if input_images.len() != input_count {
                return Err(GalError::invalid_argument(
                    "custom post-effect image-input count does not match semantic input count",
                ));
            }
            let mut snapshots = Vec::with_capacity(input_count);
            let mut snapshot_views = Vec::with_capacity(input_count);
            let frame_copy_scratch = if needs_frame_scratch {
                let texture = gal.create_texture(TextureDesc {
                    label: format!("{label}.frame-copy-scratch"),
                    dimension: TextureDimension::D2, format: color_format,
                    extent: Extent3d { width, height, depth: 1 }, mip_levels: 1, array_layers: 1,
                    usages: vec![TextureUsage::TransferSrc, TextureUsage::TransferDst],
                })?;
                created.push(texture);
                Some(texture)
            } else { None };
            for input_index in 0..input_count {
                let snapshot_format = if input_row_order == super::commands::TextureRowOrder::Reverse {
                    input_depth_views[input_index].map(|view| gal.texture_view_info(view).map(|info| info.format))
                        .transpose()?.unwrap_or(color_format)
                } else { color_format };
                let snapshot = gal.create_texture(TextureDesc {
                    label: format!("{label}.snapshot-{input_index}"),
                    dimension: TextureDimension::D2,
                    format: snapshot_format,
                    extent: Extent3d {
                        width,
                        height,
                        depth: 1,
                    },
                    mip_levels: 1,
                    array_layers: 1,
                    usages: vec![TextureUsage::Sampled, TextureUsage::TransferDst],
                })?;
                created.push(snapshot);
                let snapshot_view = gal.create_texture_view(TextureViewDesc {
                    label: format!("{label}.snapshot-view-{input_index}"),
                    texture: snapshot,
                    format: snapshot_format,
                    base_mip: 0,
                    mip_count: 1,
                    base_layer: 0,
                    layer_count: 1,
                })?;
                created.push(snapshot_view);
                snapshots.push(snapshot);
                snapshot_views.push(snapshot_view);
            }
            let mut image_textures = vec![None; input_count];
            let mut image_views = vec![None; input_count];
            let mut image_samplers = vec![None; input_count];
            let mut image_upload_buffers = vec![None; input_count];
            for (input_index, image) in input_images.iter().enumerate() {
                let Some(image) = image else { continue };
                let expected_bytes = (image.width as usize)
                    .checked_mul(image.height as usize)
                    .and_then(|pixels| pixels.checked_mul(4))
                    .ok_or_else(|| {
                        GalError::invalid_argument("custom post-effect texture dimensions overflow")
                    })?;
                if image.pixels_rgba8.len() != expected_bytes {
                    return Err(GalError::invalid_argument(format!(
                        "custom post-effect texture '{}' has {} bytes, expected {}",
                        image.path,
                        image.pixels_rgba8.len(),
                        expected_bytes
                    )));
                }
                let upload = gal.create_buffer(BufferDesc {
                    label: format!("{label}.image-{input_index}.upload"),
                    size: expected_bytes as u64,
                    memory: MemoryDomain::Upload,
                    usages: vec![
                        BufferUsage::TransferSrc,
                        BufferUsage::TransferDst,
                        BufferUsage::HostWrite,
                    ],
                })?;
                created.push(upload);
                let texture = gal.create_texture(TextureDesc {
                    label: format!("{label}.image-{input_index}.texture"),
                    dimension: TextureDimension::D2,
                    format: TextureFormat::Rgba8Unorm,
                    extent: Extent3d {
                        width: image.width,
                        height: image.height,
                        depth: 1,
                    },
                    mip_levels: 1,
                    array_layers: 1,
                    usages: vec![TextureUsage::Sampled, TextureUsage::TransferDst],
                })?;
                created.push(texture);
                let view = gal.create_texture_view(TextureViewDesc {
                    label: format!("{label}.image-{input_index}.view"),
                    texture,
                    format: TextureFormat::Rgba8Unorm,
                    base_mip: 0,
                    mip_count: 1,
                    base_layer: 0,
                    layer_count: 1,
                })?;
                created.push(view);
                let image_sampler = gal.create_sampler(SamplerDesc {
                    label: format!("{label}.image-{input_index}.sampler"),
                    min_filter: if image.bilinear {
                        SamplerFilter::Linear
                    } else {
                        SamplerFilter::Nearest
                    },
                    mag_filter: if image.bilinear {
                        SamplerFilter::Linear
                    } else {
                        SamplerFilter::Nearest
                    },
                    mip_filter: SamplerFilter::Nearest,
                    address_u: SamplerAddressMode::ClampToEdge,
                    address_v: SamplerAddressMode::ClampToEdge,
                    address_w: SamplerAddressMode::ClampToEdge,
                    comparison: None,
                })?;
                created.push(image_sampler);
                image_upload_buffers[input_index] = Some(upload);
                image_textures[input_index] = Some(texture);
                image_views[input_index] = Some(view);
                image_samplers[input_index] = Some(image_sampler);
            }
            let mut combined_samplers = Vec::with_capacity(input_count);
            let mut target_samplers = Vec::new();
            for input_index in 0..input_count {
                let sampler = if let Some(sampler) = image_samplers[input_index] {
                    sampler
                } else {
                    let filter = if input_bilinear[input_index] { SamplerFilter::Linear } else { SamplerFilter::Nearest };
                    let sampler = gal.create_sampler(SamplerDesc {
                        label: format!("{label}.target-{input_index}.sampler"),
                        min_filter: filter, mag_filter: filter, mip_filter: SamplerFilter::Nearest,
                        address_u: SamplerAddressMode::ClampToEdge,
                        address_v: SamplerAddressMode::ClampToEdge,
                        address_w: SamplerAddressMode::ClampToEdge,
                        comparison: None,
                    })?;
                    created.push(sampler);
                    target_samplers.push(sampler);
                    sampler
                };
                let (texture_view, input_sampler) = match (
                    input_depth_views[input_index],
                    image_views[input_index],
                    image_samplers[input_index],
                ) {
                    (Some(depth_view), _, _) => (if input_row_order == super::commands::TextureRowOrder::Reverse {
                        snapshot_views[input_index]
                    } else { depth_view }, sampler),
                    (None, Some(view), Some(input_sampler)) => (view, input_sampler),
                    (None, _, _) => (snapshot_views[input_index], sampler),
                };
                let combined_sampler =
                    gal.create_combined_texture_sampler(CombinedTextureSamplerDesc {
                        label: format!("{label}.combined-sampler-{input_index}"),
                        texture_view,
                        sampler: input_sampler,
                    })?;
                created.push(combined_sampler);
                combined_samplers.push(combined_sampler);
            }
            if uniform_blocks.len() > 4 {
                return Err(GalError::unsupported_feature(
                    "custom post-effect pass supports at most four static uniform blocks",
                ));
            }
            let mut uniform_buffers = Vec::with_capacity(uniform_blocks.len());
            for (block_index, bytes) in uniform_blocks.iter().enumerate() {
                let uniform_buffer = gal.create_buffer(BufferDesc {
                    label: format!("{label}.uniform-{block_index}"),
                    size: bytes.len().max(16) as u64,
                    memory: MemoryDomain::Upload,
                    usages: vec![
                        BufferUsage::Uniform,
                        BufferUsage::TransferDst,
                        BufferUsage::HostWrite,
                    ],
                })?;
                created.push(uniform_buffer);
                uniform_buffers.push(uniform_buffer);
            }
            let vertex_shader = gal.create_shader_module(ShaderModuleDesc {
                label: format!("{label}.vertex"),
                stage: ShaderStage::Vertex,
                code_format: ShaderCodeFormat::Glsl,
                code: vertex_source.to_vec(),
                entry_point: "main".to_string(),
            })?;
            created.push(vertex_shader);
            let fragment_shader = gal.create_shader_module(ShaderModuleDesc {
                label: format!("{label}.fragment"),
                stage: ShaderStage::Fragment,
                code_format: ShaderCodeFormat::Glsl,
                code: fragment_source.to_vec(),
                entry_point: "main".to_string(),
            })?;
            created.push(fragment_shader);
            let resource_layout = gal.create_resource_layout(ResourceLayoutDesc {
                label: format!("{label}.resource-layout"),
                bindings: {
                    let mut bindings = (0..input_count)
                        .map(|input_index| ResourceBindingDesc {
                            binding: input_index as u32,
                            kind: ResourceBindingKind::CombinedTextureSampler,
                            stages: PipelineStageFlags::DRAW,
                            array_count: 1,
                            optional: false,
                            dynamic_offset_count: 0,
                        })
                        .collect::<Vec<_>>();
                    bindings.extend((0..uniform_blocks.len()).map(|block_index| {
                        ResourceBindingDesc {
                            binding: input_count as u32 + block_index as u32,
                            kind: ResourceBindingKind::UniformBuffer,
                            stages: PipelineStageFlags::DRAW,
                            array_count: 1,
                            optional: false,
                            dynamic_offset_count: 0,
                        }
                    }));
                    bindings
                },
            })?;
            created.push(resource_layout);
            let resource_set = gal.create_resource_set(ResourceSetDesc {
                label: format!("{label}.resource-set"),
                layout: resource_layout,
                bindings: {
                    let mut bindings = combined_samplers
                        .iter()
                        .enumerate()
                        .map(|(input_index, combined_sampler)| ResourceBinding {
                            binding: input_index as u32,
                            array_index: 0,
                            resource: *combined_sampler,
                            kind: ResourceBindingKind::CombinedTextureSampler,
                            access: AccessFlags::READ,
                            dynamic_offsets: Vec::new(),
                            buffer_range: None,
                        })
                        .collect::<Vec<_>>();
                    bindings.extend(
                            uniform_buffers
                                .iter()
                                .enumerate()
                                .map(|(block_index, buffer)| ResourceBinding {
                                    binding: input_count as u32 + block_index as u32,
                                    array_index: 0,
                                    resource: *buffer,
                                    kind: ResourceBindingKind::UniformBuffer,
                                    access: AccessFlags::READ,
                                    dynamic_offsets: Vec::new(),
                                    buffer_range: Some(
                                        uniform_blocks[block_index].len().max(16) as u64
                                    ),
                                }),
                        );
                    bindings
                },
            })?;
            created.push(resource_set);
            let pipeline_layout = gal.create_pipeline_layout(PipelineLayoutDesc {
                label: format!("{label}.pipeline-layout"),
                resource_layouts: vec![resource_layout],
            })?;
            created.push(pipeline_layout);
            let pipeline = gal.create_graphics_pipeline(GraphicsPipelineDesc {
                label: format!("{label}.pipeline"),
                layout: pipeline_layout,
                vertex_shader,
                fragment_shader,
                topology: PrimitiveTopology::Triangles,
                cull_mode: CullMode::None,
                front_face: super::resources::FrontFace::CounterClockwise,
                provoking_vertex: crate::render::vulkanic::resources::ProvokingVertex::Last,
                raster_y_direction: crate::render::vulkanic::resources::RasterYDirection::Up,
                blend: BlendMode::Disabled,
                depth_compare: None,
                depth_write: false,
                depth_bias: None,
                color_formats: vec![color_format],
                depth_format: None,
                stencil: None,
            })?;
            created.push(pipeline);
            Ok(CustomPostEffectResources {
                frame_copy_scratch,
                identity: cache_identity,
                width,
                height,
                color_format,
                snapshots,
                snapshot_views,
                target_samplers,
                image_textures,
                image_views,
                image_samplers,
                image_upload_buffers,
                combined_samplers,
                uniform_buffers,
                vertex_shader,
                fragment_shader,
                resource_layout,
                resource_set,
                pipeline_layout,
                pipeline,
            })
        })();
        if result.is_err() {
            for handle in created.into_iter().rev() {
                let _ = gal.destroy(handle);
            }
        }
        self.custom_post_effect_resources.push(result?);
        self.custom_post_effect_snapshot_initialized
            .push(vec![false; input_count]);
        self.custom_post_effect_image_initialized
            .push(vec![false; input_count]);
        Ok(self.custom_post_effect_resources.last().unwrap())
    }

    fn ensure_custom_post_effect_intermediate(
        &mut self,
        gal: &mut VulkanicGal,
        identity: &str,
        target_name: &str,
        width: u32,
        height: u32,
        color_format: ColorFormat,
    ) -> GalResult<Option<(Handle, Handle, Handle, Handle)>> {
        if !self
            .custom_post_effect_intermediates
            .contains_key(target_name)
            && self.custom_post_effect_intermediates.len() >= MAX_CUSTOM_POST_EFFECT_INTERMEDIATES
        {
            return Err(GalError::unsupported_feature(format!(
                "custom post-effect intermediate cache exceeds bounded limit {MAX_CUSTOM_POST_EFFECT_INTERMEDIATES}"
            )));
        }
        let cache_identity = format!("{identity}#{target_name}");
        if let Some(existing) = self.custom_post_effect_intermediates.get(target_name) {
            if existing.identity == cache_identity
                && existing.width == width
                && existing.height == height
                && existing.color_format == color_format
            {
                return Ok(Some((
                    existing.texture,
                    existing.view,
                    existing.target,
                    existing.pass,
                )));
            }
        }
        if let Some(previous) = self.custom_post_effect_intermediates.remove(target_name) {
            for handle in previous.handles_in_destroy_order() {
                let _ = gal.destroy(handle);
            }
        }
        let label =
            format!("minecraft.post-effect.{identity}.intermediate.{target_name}.{width}x{height}");
        let mut created = Vec::new();
        let result = (|| -> GalResult<CustomPostEffectIntermediate> {
            let texture = gal.create_texture(TextureDesc {
                label: format!("{label}.texture"),
                dimension: TextureDimension::D2,
                format: color_format,
                extent: Extent3d {
                    width,
                    height,
                    depth: 1,
                },
                mip_levels: 1,
                array_layers: 1,
                usages: vec![
                    TextureUsage::Sampled,
                    TextureUsage::ColorAttachment,
                    TextureUsage::TransferSrc,
                ],
            })?;
            created.push(texture);
            let view = gal.create_texture_view(TextureViewDesc {
                label: format!("{label}.view"),
                texture,
                format: color_format,
                base_mip: 0,
                mip_count: 1,
                base_layer: 0,
                layer_count: 1,
            })?;
            created.push(view);
            let target = gal.create_render_target(RenderTargetDesc {
                label: format!("{label}.target"),
                color_views: vec![view],
                depth_stencil_view: None,
                extent: Extent3d {
                    width,
                    height,
                    depth: 1,
                },
            })?;
            created.push(target);
            let pass = gal.create_render_pass(RenderPassDesc {
                label: format!("{label}.pass"),
                target,
                color_formats: vec![color_format],
                depth_format: None,
            })?;
            created.push(pass);
            Ok(CustomPostEffectIntermediate {
                identity: cache_identity,
                width,
                height,
                color_format,
                texture,
                view,
                target,
                pass,
                usage: TextureUsageState::Undefined,
            })
        })();
        if result.is_err() {
            for handle in created.into_iter().rev() {
                let _ = gal.destroy(handle);
            }
        }
        let intermediate = result?;
        let handles = (
            intermediate.texture,
            intermediate.view,
            intermediate.target,
            intermediate.pass,
        );
        self.custom_post_effect_intermediates
            .insert(target_name.to_owned(), intermediate);
        Ok(Some(handles))
    }

    /// Executes a bounded resource-pack post-effect chain. Main-target reads
    /// are snapshotted; each declared private target is a Rust-owned
    /// generation-bound intermediate. Each pass may carry bounded static
    /// uniform blocks; dynamic uniform writers and feedback graphs remain
    /// unavailable until their explicit contracts exist.
    pub(crate) fn append_custom_post_effect(
        &mut self,
        gal: &mut VulkanicGal,
        render_target: Handle,
        color_attachment: Handle,
        identity: &str,
        shader_sources: &[CustomPostEffectSource],
    ) -> GalResult<Vec<CommandOp>> {
        self.append_custom_post_effect_with_external_targets(
            gal,
            render_target,
            color_attachment,
            identity,
            shader_sources,
            None,
        )
    }

    /// Executes a custom graph with a validated Rust-owned external target
    /// inventory. The legacy wrapper above intentionally supplies no
    /// inventory, keeping external roles unavailable until the frame
    /// coordinator has installed the complete attachment set.
    pub(crate) fn append_custom_post_effect_with_external_targets(
        &mut self,
        gal: &mut VulkanicGal,
        render_target: Handle,
        color_attachment: Handle,
        identity: &str,
        shader_sources: &[CustomPostEffectSource],
        external_targets: Option<&VanillaPostEffectExternalTargetBindings>,
    ) -> GalResult<Vec<CommandOp>> {
        if shader_sources.is_empty() || shader_sources.len() > MAX_CUSTOM_POST_EFFECT_PASSES {
            return Err(GalError::unsupported_feature(
                format!(
                    "custom post-effect pass count must be in the bounded range 1..={MAX_CUSTOM_POST_EFFECT_PASSES}"
                ),
            ));
        }
        let graph_uniform_bytes = shader_sources
            .iter()
            .flat_map(|source| source.uniform_blocks.iter())
            .try_fold(0usize, |total, block| total.checked_add(block.len()))
            .ok_or_else(|| {
                GalError::unsupported_feature(
                    "custom post-effect graph uniform byte count overflowed the bounded contract",
                )
            })?;
        if graph_uniform_bytes > MAX_CUSTOM_POST_EFFECT_UNIFORM_GRAPH_BYTES {
            return Err(GalError::unsupported_feature(format!(
                "custom post-effect graph uniform bytes exceed bounded limit {MAX_CUSTOM_POST_EFFECT_UNIFORM_GRAPH_BYTES}"
            )));
        }
        let extent = gal.pass_target_extent(render_target)?;
        let color_format = gal.pass_target_color_format(render_target)?;
        let mut intermediate_names = BTreeMap::<String, ()>::new();
        for source in shader_sources {
            if source.input_bilinear.len() != source.input_count {
                return Err(GalError::invalid_argument("custom post-effect filter inventory does not match input count"));
            }
            if source.sampler_info_uniform.is_some_and(|index| index >= source.uniform_blocks.len()) {
                return Err(GalError::invalid_argument("post-effect SamplerInfo binding is outside its uniform inventory"));
            }
            let uniform_bytes = source
                .uniform_blocks
                .iter()
                .try_fold(0usize, |total, block| total.checked_add(block.len()))
                .ok_or_else(|| {
                    GalError::unsupported_feature(
                        "custom post-effect uniform byte count overflowed the bounded contract",
                    )
                })?;
            if uniform_bytes > MAX_CUSTOM_POST_EFFECT_UNIFORM_BYTES {
                return Err(GalError::unsupported_feature(format!(
                    "custom post-effect uniform bytes exceed bounded limit {MAX_CUSTOM_POST_EFFECT_UNIFORM_BYTES}"
                )));
            }
            if source.input_images.len() != source.input_count {
                return Err(GalError::unsupported_feature(
                    "custom post-effect image-input inventory does not match its semantic input count",
                ));
            }
            if source.input_use_depth.len() != source.input_count {
                return Err(GalError::unsupported_feature(
                    "custom post-effect depth-input inventory does not match its semantic input count",
                ));
            }
            for target in source
                .input_targets
                .iter()
                .chain(std::iter::once(&source.output_target))
            {
                if !target.is_empty()
                    && target != "minecraft:main"
                    && !external_targets.is_some_and(|targets| targets.get(target).is_some())
                {
                    intermediate_names.insert(target.clone(), ());
                }
            }
        }
        let mut produced_targets = BTreeSet::new();
        for source in shader_sources {
            let invalid_shape = source.input_targets.len() != source.input_count
                || source.input_count == 0
                || source.input_count > 4
                || source.input_targets.iter().enumerate().any(|(input_index, target)| {
                    source.input_images.get(input_index).is_none_or(Option::is_none)
                        && target != "minecraft:main"
                        && !external_targets.is_some_and(|targets| targets.get(target).is_some())
                        && !produced_targets.contains(target)
                })
                // Each pass reads the last completed version of its inputs.
                // Reusing a private output in a later pass is legal; sampling
                // that same private output during its own write is not.
                || (source.output_target != "minecraft:main"
                    && !external_targets.is_some_and(|targets| targets.get(&source.output_target).is_some())
                    && (source.input_targets.iter().enumerate().any(|(input_index, target)| {
                            source.input_images.get(input_index).is_none_or(Option::is_none)
                                && target == &source.output_target
                        })
                        || (!intermediate_names.contains_key(&source.output_target)
                            && !external_targets.is_some_and(|targets| targets.get(&source.output_target).is_some()))
                        ));
            if invalid_shape {
                return Err(GalError::unsupported_feature(
                    "custom post-effect graph has an invalid bounded target contract",
                ));
            }
            for (index, target) in source.input_targets.iter().enumerate() {
                if source.input_use_depth[index] && (source.input_images[index].is_some()
                    || (target != "minecraft:main" && external_targets
                        .and_then(|targets| targets.get(target))
                        .and_then(|binding| binding.depth_attachment).is_none()))
                {
                    return Err(GalError::unsupported_feature(
                        "custom post-effect named depth input has no matching owned depth attachment",
                    ));
                }
            }
            if source.output_target != "minecraft:main" {
                produced_targets.insert(source.output_target.clone());
            }
        }
        if intermediate_names.len() > MAX_CUSTOM_POST_EFFECT_INTERMEDIATES {
            return Err(GalError::unsupported_feature(format!(
                "custom post-effect intermediate target count exceeds bounded limit {MAX_CUSTOM_POST_EFFECT_INTERMEDIATES}"
            )));
        }
        let depth_requested = shader_sources
            .iter()
            .any(|source| source.input_use_depth.iter().any(|uses_depth| *uses_depth));
        let (execution_target, depth_view, depth_texture) = if depth_requested {
            let Some((depth_texture, depth_view)) =
                gal.pass_target_depth_attachment(render_target)?
            else {
                return Err(GalError::unsupported_feature(
                    "custom post-effect depth input requires a Rust-owned render-target depth attachment",
                ));
            };
            let color_view = gal.pass_target_color_attachment(render_target)?;
            let execution_target = if self
                .custom_post_effect_depth_target
                .as_ref()
                .is_some_and(|cached| cached.source == render_target)
            {
                self.custom_post_effect_depth_target
                    .as_ref()
                    .unwrap()
                    .target
            } else {
                if let Some(previous) = self.custom_post_effect_depth_target.take() {
                    for handle in previous.handles_in_destroy_order() {
                        let _ = gal.destroy(handle);
                    }
                }
                let target = gal.create_render_target(RenderTargetDesc {
                    label: format!("minecraft.post-effect.{identity}.color-only-target"),
                    color_views: vec![color_view],
                    depth_stencil_view: None,
                    extent,
                })?;
                self.custom_post_effect_depth_target = Some(CustomPostEffectDepthTarget {
                    source: render_target,
                    target,
                    color_texture: None,
                    color_view,
                    owns_color_view: false,
                    depth_texture: None,
                    depth_view,
                    owns_depth_view: false,
                });
                target
            };
            (execution_target, Some(depth_view), Some(depth_texture))
        } else {
            (render_target, None, None)
        };
        let stale_intermediate_names = self
            .custom_post_effect_intermediates
            .keys()
            .filter(|name| !intermediate_names.contains_key(*name))
            .cloned()
            .collect::<Vec<_>>();
        for target_name in stale_intermediate_names {
            if let Some(previous) = self.custom_post_effect_intermediates.remove(&target_name) {
                for handle in previous.handles_in_destroy_order() {
                    let _ = gal.destroy(handle);
                }
            }
        }
        let mut intermediates = BTreeMap::new();
        for target_name in intermediate_names.keys() {
            let handles = self
                .ensure_custom_post_effect_intermediate(
                    gal,
                    identity,
                    target_name,
                    extent.width,
                    extent.height,
                    color_format,
                )?
                .expect("named private post-effect target must be allocated");
            intermediates.insert(target_name.clone(), handles);
        }
        let mut ops = Vec::new();
        for (pass_index, source) in shader_sources.iter().enumerate() {
            let input_depth_views = (0..source.input_count)
                .map(|input_index| {
                    if !source.input_use_depth[input_index] {
                        return None;
                    }
                    if source.input_targets[input_index] == "minecraft:main" {
                        depth_view
                    } else {
                        external_targets
                            .and_then(|targets| targets.get(&source.input_targets[input_index]))
                            .and_then(|binding| binding.depth_attachment)
                    }
                })
                .collect::<Vec<_>>();
            let (
                snapshots,
                frame_copy_scratch,
                image_textures,
                image_upload_buffers,
                pipeline,
                pipeline_layout,
                resource_set,
            ) = {
                let resources = self.ensure_custom_post_effect_resources(
                    gal,
                    identity,
                    pass_index,
                    extent.width,
                    extent.height,
                    color_format,
                    &source.vertex_shader,
                    &source.fragment_shader,
                    source.input_count,
                    source.input_row_order,
                    source.input_row_order == super::commands::TextureRowOrder::Reverse
                        && render_target.kind() == Some(super::handles::HandleKind::FrameTarget)
                        && source.input_targets.iter().enumerate().any(|(i, name)|
                            name == "minecraft:main" && !source.input_use_depth[i]
                                && source.input_images[i].is_none()),
                    &source.input_bilinear,
                    &source.input_targets,
                    &source.input_images,
                    &input_depth_views,
                    &source.output_target,
                    &source.uniform_blocks,
                )?;
                (
                    resources.snapshots.clone(),
                    resources.frame_copy_scratch,
                    resources.image_textures.clone(),
                    resources.image_upload_buffers.clone(),
                    resources.pipeline,
                    resources.pipeline_layout,
                    resources.resource_set,
                )
            };
            let uniform_buffers = self.custom_post_effect_resources[pass_index]
                .uniform_buffers
                .clone();
            let snapshot_initialized =
                self.custom_post_effect_snapshot_initialized[pass_index].clone();
            let image_initialized = self.custom_post_effect_image_initialized[pass_index].clone();
            let (pass_target, pass_color_attachment, pass_handle) =
                if source.output_target == "minecraft:main" {
                    (
                        execution_target,
                        color_attachment,
                        self.frame_pass(gal, execution_target, None)?,
                    )
                } else if let Some(binding) =
                    external_targets.and_then(|targets| targets.get(&source.output_target))
                {
                    (
                        binding.render_target,
                        binding.color_attachment,
                        binding.render_pass,
                    )
                } else {
                    let (_, view, target, pass) = intermediates
                        .get(&source.output_target)
                        .copied()
                        .ok_or_else(|| {
                            GalError::unsupported_feature(
                                "custom post-effect intermediate target was not allocated",
                            )
                        })?;
                    (target, view, pass)
                };
            let mut source_states = BTreeMap::<String, TextureUsageState>::new();
            let reverse_rows = source.input_row_order == super::commands::TextureRowOrder::Reverse;
            let mut scratch_initialized = snapshot_initialized.iter().any(|value| *value);
            for (input_index, source_target_name) in source.input_targets.iter().enumerate() {
                if source
                    .input_images
                    .get(input_index)
                    .is_some_and(Option::is_some)
                    || (source.input_use_depth[input_index] && !reverse_rows)
                {
                    continue;
                }
                let snapshot = snapshots.get(input_index).copied().ok_or_else(|| {
                    GalError::unsupported_feature(
                        "custom post-effect input snapshot was not allocated",
                    )
                })?;
                ops.push(CommandOp::Barrier(ResourceBarrier {
                    resource: snapshot,
                    subresources: None,
                    before: if snapshot_initialized
                        .get(input_index)
                        .copied()
                        .unwrap_or(false)
                    {
                        TextureUsageState::ShaderRead
                    } else {
                        TextureUsageState::Undefined
                    },
                    after: TextureUsageState::TransferDst,
                    src_queue: QueueClass::Graphics,
                    dst_queue: QueueClass::Transfer,
                }));
                if source.input_use_depth[input_index] {
                    let info = gal.texture_view_info(input_depth_views[input_index].ok_or_else(||
                        GalError::unsupported_feature("row-converted depth input has no owned view"))?)?;
                    if info.range.base_mip != 0 || info.range.mip_count != 1
                        || info.range.base_layer != 0 || info.range.layer_count != 1
                        || info.extent != extent || !info.usages.contains(&TextureUsage::TransferSrc)
                    {
                        return Err(GalError::unsupported_feature(
                            "row-converted depth input requires a full-size transferable single-subresource attachment"));
                    }
                    let restore = external_targets.and_then(|targets| targets.get(source_target_name))
                        .and_then(|binding| binding.depth_usage)
                        .unwrap_or(TextureUsageState::DepthStencilAttachment);
                    ops.extend([
                        CommandOp::Barrier(texture_barrier(info.texture, restore, TextureUsageState::TransferSrc)),
                        CommandOp::CopyTexture(TextureImageCopyRegion {
                            row_order: source.input_row_order,
                            src_texture: info.texture, src_mip: 0, src_layer: 0,
                            src_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
                            dst_texture: snapshot, dst_mip: 0, dst_layer: 0,
                            dst_origin: TextureOrigin3d { x: 0, y: 0, z: 0 }, extent,
                        }),
                        CommandOp::Barrier(texture_barrier(info.texture, TextureUsageState::TransferSrc, restore)),
                    ]);
                } else if source_target_name == "minecraft:main"
                    && render_target.kind() == Some(super::handles::HandleKind::FrameTarget)
                {
                    let copy_destination = frame_copy_scratch.unwrap_or(snapshot);
                    if let Some(scratch) = frame_copy_scratch {
                        ops.push(CommandOp::Barrier(texture_barrier(scratch,
                            if scratch_initialized { TextureUsageState::TransferSrc } else { TextureUsageState::Undefined },
                            TextureUsageState::TransferDst)));
                    }
                    ops.push(CommandOp::CopyFrameTargetToTexture {
                        src: render_target,
                        dst: copy_destination,
                        extent,
                    });
                    if let Some(scratch) = frame_copy_scratch {
                        ops.extend([
                            CommandOp::Barrier(texture_barrier(scratch, TextureUsageState::TransferDst, TextureUsageState::TransferSrc)),
                            CommandOp::CopyTexture(TextureImageCopyRegion {
                                row_order: source.input_row_order,
                                src_texture: scratch, src_mip: 0, src_layer: 0,
                                src_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
                                dst_texture: snapshot, dst_mip: 0, dst_layer: 0,
                                dst_origin: TextureOrigin3d { x: 0, y: 0, z: 0 }, extent,
                            }),
                        ]);
                        scratch_initialized = true;
                    }
                } else {
                    let (source_texture, restore_state) = if source_target_name == "minecraft:main"
                    {
                        (
                            gal.pass_target_color_texture(render_target)?,
                            TextureUsageState::ColorAttachment,
                        )
                    } else if let Some(binding) =
                        external_targets.and_then(|targets| targets.get(source_target_name))
                    {
                        (
                            gal.pass_target_color_texture(binding.render_target)?,
                            binding.color_usage,
                        )
                    } else {
                        let (source_texture, _, _, _) = intermediates
                            .get(source_target_name)
                            .copied()
                            .ok_or_else(|| {
                                GalError::unsupported_feature(
                                    "custom post-effect intermediate input was not allocated",
                                )
                            })?;
                        (source_texture, TextureUsageState::ShaderRead)
                    };
                    let before_state = source_states
                        .insert(source_target_name.clone(), TextureUsageState::TransferSrc)
                        .unwrap_or_else(|| {
                            external_targets
                                .and_then(|targets| targets.get(source_target_name))
                                .map(|binding| binding.color_usage)
                            .unwrap_or_else(|| self.custom_post_effect_intermediates
                                .get(source_target_name).map(|target| target.usage)
                                .unwrap_or(TextureUsageState::ColorAttachment))
                        });
                    ops.extend([
                        CommandOp::Barrier(ResourceBarrier {
                            resource: source_texture,
                            subresources: None,
                            before: before_state,
                            after: TextureUsageState::TransferSrc,
                            src_queue: QueueClass::Graphics,
                            dst_queue: QueueClass::Transfer,
                        }),
                        CommandOp::CopyTexture(TextureImageCopyRegion {
                            row_order: source.input_row_order,
                            src_texture: source_texture,
                            src_mip: 0,
                            src_layer: 0,
                            src_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
                            dst_texture: snapshot,
                            dst_mip: 0,
                            dst_layer: 0,
                            dst_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
                            extent,
                        }),
                        CommandOp::Barrier(ResourceBarrier {
                            resource: source_texture,
                            subresources: None,
                            before: TextureUsageState::TransferSrc,
                            after: restore_state,
                            src_queue: QueueClass::Transfer,
                            dst_queue: QueueClass::Graphics,
                        }),
                    ]);
                    source_states.insert(source_target_name.clone(), restore_state);
                    if let Some(target) = self.custom_post_effect_intermediates.get_mut(source_target_name) {
                        target.usage = restore_state;
                    }
                }
            }
            for (input_index, image) in source.input_images.iter().enumerate() {
                let Some(image) = image else { continue };
                if image_initialized.get(input_index).copied().unwrap_or(false) {
                    continue;
                }
                let upload = image_upload_buffers
                    .get(input_index)
                    .and_then(|buffer| *buffer)
                    .ok_or_else(|| {
                        GalError::unsupported_feature(
                            "custom post-effect texture upload buffer was not allocated",
                        )
                    })?;
                let texture = image_textures
                    .get(input_index)
                    .and_then(|texture| *texture)
                    .ok_or_else(|| {
                        GalError::unsupported_feature(
                            "custom post-effect texture resource was not allocated",
                        )
                    })?;
                ops.extend([
                    CommandOp::HostWriteBuffer {
                        buffer: upload,
                        offset: 0,
                        data: image.pixels_rgba8.clone(),
                    },
                    CommandOp::Barrier(buffer_barrier(
                        upload,
                        TextureUsageState::TransferDst,
                        TextureUsageState::TransferSrc,
                    )),
                    CommandOp::Barrier(texture_barrier(
                        texture,
                        TextureUsageState::Undefined,
                        TextureUsageState::TransferDst,
                    )),
                    CommandOp::CopyBufferToTexture(BufferImageCopyRegion {
                        buffer: upload,
                        buffer_offset: 0,
                        bytes_per_row: image.width * 4,
                        rows_per_image: image.height,
                        texture,
                        texture_mip: 0,
                        texture_layer: 0,
                        texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
                        extent: Extent3d {
                            width: image.width,
                            height: image.height,
                            depth: 1,
                        },
                    }),
                    CommandOp::Barrier(texture_barrier(
                        texture,
                        TextureUsageState::TransferDst,
                        TextureUsageState::ShaderRead,
                    )),
                ]);
                self.custom_post_effect_image_initialized[pass_index][input_index] = true;
            }
            for (index, (buffer, bytes)) in uniform_buffers.iter().zip(source.uniform_blocks.iter()).enumerate() {
                push_uniform_write(&mut ops, *buffer,
                    if source.sampler_info_uniform == Some(index) {
                        custom_sampler_info_bytes(extent, &source.input_images)
                    } else if bytes.is_empty() {
                        vec![0; 16]
                    } else {
                        bytes.clone()
                    },
                );
            }
            for (input_index, snapshot) in snapshots.iter().enumerate() {
                if source
                    .input_images
                    .get(input_index)
                    .is_some_and(Option::is_some)
                    || (source.input_use_depth[input_index] && !reverse_rows)
                {
                    continue;
                }
                ops.push(CommandOp::Barrier(ResourceBarrier {
                    resource: *snapshot,
                    subresources: None,
                    before: TextureUsageState::TransferDst,
                    after: TextureUsageState::ShaderRead,
                    src_queue: QueueClass::Transfer,
                    dst_queue: QueueClass::Graphics,
                }));
            }
            if let Some(depth_texture) = depth_texture
                .filter(|_| !reverse_rows && source.input_use_depth.iter().any(|uses_depth| *uses_depth))
            {
                ops.push(CommandOp::Barrier(ResourceBarrier {
                    resource: depth_texture,
                    subresources: None,
                    before: TextureUsageState::DepthStencilAttachment,
                    after: TextureUsageState::ShaderRead,
                    src_queue: QueueClass::Graphics,
                    dst_queue: QueueClass::Graphics,
                }));
            }
            for (input_index, uses_depth) in source.input_use_depth.iter().copied().enumerate() {
                if !uses_depth || reverse_rows {
                    continue;
                }
                let target_name = &source.input_targets[input_index];
                let Some(binding) = external_targets.and_then(|targets| targets.get(target_name))
                else {
                    continue;
                };
                let (external_depth_texture, _) = gal
                    .pass_target_depth_attachment(binding.render_target)?
                    .ok_or_else(|| GalError::unsupported_feature(
                        "custom post-effect external depth input has no Rust-owned depth attachment",
                    ))?;
                ops.push(CommandOp::Barrier(ResourceBarrier {
                    resource: external_depth_texture,
                    subresources: None,
                    before: binding
                        .depth_usage
                        .unwrap_or(TextureUsageState::DepthStencilAttachment),
                    after: TextureUsageState::ShaderRead,
                    src_queue: QueueClass::Graphics,
                    dst_queue: QueueClass::Graphics,
                }));
            }
            if let Some(binding) =
                external_targets.and_then(|targets| targets.get(&source.output_target))
            {
                if binding.color_usage != TextureUsageState::ColorAttachment {
                    let output_texture = gal.pass_target_color_texture(binding.render_target)?;
                    ops.push(CommandOp::Barrier(ResourceBarrier {
                        resource: output_texture,
                        subresources: None,
                        before: binding.color_usage,
                        after: TextureUsageState::ColorAttachment,
                        src_queue: QueueClass::Graphics,
                        dst_queue: QueueClass::Graphics,
                    }));
                }
            }
            if let Some(target) = self.custom_post_effect_intermediates.get_mut(&source.output_target) {
                ops.push(CommandOp::Barrier(texture_barrier(target.texture, target.usage,
                    TextureUsageState::ColorAttachment)));
                target.usage = TextureUsageState::ColorAttachment;
            }
            ops.extend([
                CommandOp::BeginPass {
                    pass: pass_handle,
                    target: pass_target,
                    colors: vec![loaded_frame_color_attachment(pass_color_attachment)],
                    depth_stencil: None,
                },
                CommandOp::BindGraphicsPipeline(pipeline),
                CommandOp::BindResourceSet {
                    pipeline_layout,
                    set_index: 0,
                    set: resource_set,
                    dynamic_offsets: Vec::new(),
                },
                CommandOp::Draw {
                    vertices: 3,
                    instances: 1,
                },
                CommandOp::EndPass,
            ]);
            for (input_index, uses_depth) in source.input_use_depth.iter().copied().enumerate() {
                if !uses_depth || reverse_rows {
                    continue;
                }
                let target_name = &source.input_targets[input_index];
                let Some(binding) = external_targets.and_then(|targets| targets.get(target_name))
                else {
                    continue;
                };
                let (external_depth_texture, _) = gal
                    .pass_target_depth_attachment(binding.render_target)?
                    .ok_or_else(|| GalError::unsupported_feature(
                        "custom post-effect external depth input has no Rust-owned depth attachment",
                    ))?;
                let restore = binding
                    .depth_usage
                    .unwrap_or(TextureUsageState::DepthStencilAttachment);
                if restore != TextureUsageState::ShaderRead {
                    ops.push(CommandOp::Barrier(ResourceBarrier {
                        resource: external_depth_texture,
                        subresources: None,
                        before: TextureUsageState::ShaderRead,
                        after: restore,
                        src_queue: QueueClass::Graphics,
                        dst_queue: QueueClass::Graphics,
                    }));
                }
            }
            if let Some(depth_texture) = depth_texture
                .filter(|_| !reverse_rows && source.input_use_depth.iter().any(|uses_depth| *uses_depth))
            {
                ops.push(CommandOp::Barrier(ResourceBarrier {
                    resource: depth_texture,
                    subresources: None,
                    before: TextureUsageState::ShaderRead,
                    after: TextureUsageState::DepthStencilAttachment,
                    src_queue: QueueClass::Graphics,
                    dst_queue: QueueClass::Graphics,
                }));
            }
            self.custom_post_effect_snapshot_initialized[pass_index].fill(true);
        }
        Ok(ops)
    }

    /// Lowers the vanilla creeper vision graph: color-convolve into a private
    /// intermediate target, then apply the bounded mosaic/bits pass back to
    /// the acquired target. The effect is intentionally explicit and uses no
    /// Java post-chain state.
    pub(crate) fn append_creeper_post_effect(
        &mut self,
        gal: &mut VulkanicGal,
        render_target: Handle,
        color_attachment: Handle,
    ) -> GalResult<Vec<CommandOp>> {
        let extent = gal.pass_target_extent(render_target)?;
        let color_format = gal.pass_target_color_format(render_target)?;
        let resources =
            self.ensure_blur_resources(gal, extent.width, extent.height, color_format)?;
        let snapshot = resources.texture;
        let intermediate = resources.creeper_texture;
        let intermediate_view = resources.creeper_view;
        let intermediate_target = resources.creeper_target;
        let intermediate_pass = resources.creeper_pass;
        let uniform_buffer = resources.uniform_buffer;
        let source_set = resources.resource_set;
        let intermediate_set = resources.creeper_resource_set;
        let layout = resources.pipeline_layout;
        let color_pipeline = resources.creeper_color_pipeline;
        let bits_pipeline = resources.creeper_bits_pipeline;
        let final_pass = self.frame_pass(gal, render_target, None)?;
        let snapshot_before = if self.blur_snapshot_initialized {
            TextureUsageState::ShaderRead
        } else {
            TextureUsageState::Undefined
        };
        let mut ops = vec![
            CommandOp::HostWriteBuffer {
                buffer: uniform_buffer,
                offset: 0,
                data: {
                    let mut bytes = vec![0u8; 64];
                    for (index, value) in [
                        0.0f32, 0.0, 0.0, 0.0, 0.3, 0.59, 0.11, 0.0, 0.0, 0.0, 0.0, 0.0,
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
                    }
                    bytes
                },
            },
            CommandOp::Barrier(buffer_barrier(
                uniform_buffer,
                TextureUsageState::TransferDst,
                TextureUsageState::ShaderRead,
            )),
            CommandOp::Barrier(ResourceBarrier {
                resource: snapshot,
                subresources: None,
                before: snapshot_before,
                after: TextureUsageState::TransferDst,
                src_queue: QueueClass::Graphics,
                dst_queue: QueueClass::Transfer,
            }),
        ];
        if render_target.kind() == Some(super::handles::HandleKind::FrameTarget) {
            ops.push(CommandOp::CopyFrameTargetToTexture {
                src: render_target,
                dst: snapshot,
                extent,
            });
        } else {
            let source_texture = gal.pass_target_color_texture(render_target)?;
            ops.extend([
                CommandOp::Barrier(ResourceBarrier {
                    resource: source_texture,
                    subresources: None,
                    before: TextureUsageState::ColorAttachment,
                    after: TextureUsageState::TransferSrc,
                    src_queue: QueueClass::Graphics,
                    dst_queue: QueueClass::Transfer,
                }),
                CommandOp::CopyTexture(TextureImageCopyRegion {
                    row_order: crate::render::vulkanic::commands::TextureRowOrder::Preserve,
                    src_texture: source_texture,
                    src_mip: 0,
                    src_layer: 0,
                    src_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
                    dst_texture: snapshot,
                    dst_mip: 0,
                    dst_layer: 0,
                    dst_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
                    extent,
                }),
                CommandOp::Barrier(ResourceBarrier {
                    resource: source_texture,
                    subresources: None,
                    before: TextureUsageState::TransferSrc,
                    after: TextureUsageState::ColorAttachment,
                    src_queue: QueueClass::Transfer,
                    dst_queue: QueueClass::Graphics,
                }),
            ]);
        }
        ops.extend([
            CommandOp::Barrier(ResourceBarrier {
                resource: snapshot,
                subresources: None,
                before: TextureUsageState::TransferDst,
                after: TextureUsageState::ShaderRead,
                src_queue: QueueClass::Transfer,
                dst_queue: QueueClass::Graphics,
            }),
            CommandOp::Barrier(ResourceBarrier {
                resource: intermediate,
                subresources: None,
                before: if self.creeper_intermediate_initialized {
                    TextureUsageState::ShaderRead
                } else {
                    TextureUsageState::Undefined
                },
                after: TextureUsageState::ColorAttachment,
                src_queue: QueueClass::Graphics,
                dst_queue: QueueClass::Graphics,
            }),
            CommandOp::BeginPass {
                pass: intermediate_pass,
                target: intermediate_target,
                colors: vec![PassAttachment {
                    view: intermediate_view,
                    load_op: AttachmentLoadOp::DontCare,
                    store_op: AttachmentStoreOp::Store,
                    clear_color: None,
                }],
                depth_stencil: None,
            },
            CommandOp::BindGraphicsPipeline(color_pipeline),
            CommandOp::BindResourceSet {
                pipeline_layout: layout,
                set_index: 0,
                set: source_set,
                dynamic_offsets: Vec::new(),
            },
            CommandOp::Draw {
                vertices: 3,
                instances: 1,
            },
            CommandOp::EndPass,
            CommandOp::Barrier(ResourceBarrier {
                resource: intermediate,
                subresources: None,
                before: TextureUsageState::ColorAttachment,
                after: TextureUsageState::ShaderRead,
                src_queue: QueueClass::Graphics,
                dst_queue: QueueClass::Graphics,
            }),
            CommandOp::HostWriteBuffer {
                buffer: uniform_buffer,
                offset: 0,
                data: {
                    let mut bytes = vec![0u8; 64];
                    bytes[..4].copy_from_slice(&16.0f32.to_le_bytes());
                    bytes[4..8].copy_from_slice(&4.0f32.to_le_bytes());
                    bytes
                },
            },
            CommandOp::Barrier(buffer_barrier(
                uniform_buffer,
                TextureUsageState::TransferDst,
                TextureUsageState::ShaderRead,
            )),
            CommandOp::BeginPass {
                pass: final_pass,
                target: render_target,
                colors: vec![loaded_frame_color_attachment(color_attachment)],
                depth_stencil: None,
            },
            CommandOp::BindGraphicsPipeline(bits_pipeline),
            CommandOp::BindResourceSet {
                pipeline_layout: layout,
                set_index: 0,
                set: intermediate_set,
                dynamic_offsets: Vec::new(),
            },
            CommandOp::Draw {
                vertices: 3,
                instances: 1,
            },
            CommandOp::EndPass,
        ]);
        self.blur_snapshot_initialized = true;
        self.creeper_intermediate_initialized = true;
        Ok(ops)
    }

    pub(crate) fn append_spider_post_effect(
        &mut self,
        gal: &mut VulkanicGal,
        render_target: Handle,
        color_attachment: Handle,
    ) -> GalResult<Vec<CommandOp>> {
        let extent = gal.pass_target_extent(render_target)?;
        let color_format = gal.pass_target_color_format(render_target)?;
        let (
            snapshot,
            uniform,
            single_layout,
            dual_layout,
            box_pipeline,
            clip_pipeline,
            blit_pipeline,
            single_sets,
            dual_sets,
            targets,
            views,
            passes,
            spider_textures,
        ) = {
            let resources =
                self.ensure_blur_resources(gal, extent.width, extent.height, color_format)?;
            (
                resources.texture,
                resources.uniform_buffer,
                resources.pipeline_layout,
                resources.spider_dual_pipeline_layout,
                resources.spider_box_pipeline,
                resources.spider_clip_pipeline,
                resources.spider_blit_pipeline,
                resources.spider_single_sets,
                resources.spider_dual_sets,
                resources.spider_targets,
                resources.spider_views,
                resources.spider_passes,
                resources.spider_textures,
            )
        };
        let final_pass = self.frame_pass(gal, render_target, None)?;
        let snapshot_before = if self.blur_snapshot_initialized {
            TextureUsageState::ShaderRead
        } else {
            TextureUsageState::Undefined
        };
        let mut spider_initialized = self.spider_initialized;
        let mut ops = vec![CommandOp::Barrier(ResourceBarrier {
            resource: snapshot,
            subresources: None,
            before: snapshot_before,
            after: TextureUsageState::TransferDst,
            src_queue: QueueClass::Graphics,
            dst_queue: QueueClass::Transfer,
        })];
        if render_target.kind() == Some(super::handles::HandleKind::FrameTarget) {
            ops.push(CommandOp::CopyFrameTargetToTexture {
                src: render_target,
                dst: snapshot,
                extent,
            });
        } else {
            let source_texture = gal.pass_target_color_texture(render_target)?;
            ops.extend([
                CommandOp::Barrier(ResourceBarrier {
                    resource: source_texture,
                    subresources: None,
                    before: TextureUsageState::ColorAttachment,
                    after: TextureUsageState::TransferSrc,
                    src_queue: QueueClass::Graphics,
                    dst_queue: QueueClass::Transfer,
                }),
                CommandOp::CopyTexture(TextureImageCopyRegion {
                    row_order: crate::render::vulkanic::commands::TextureRowOrder::Preserve,
                    src_texture: source_texture,
                    src_mip: 0,
                    src_layer: 0,
                    src_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
                    dst_texture: snapshot,
                    dst_mip: 0,
                    dst_layer: 0,
                    dst_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
                    extent,
                }),
                CommandOp::Barrier(ResourceBarrier {
                    resource: source_texture,
                    subresources: None,
                    before: TextureUsageState::TransferSrc,
                    after: TextureUsageState::ColorAttachment,
                    src_queue: QueueClass::Transfer,
                    dst_queue: QueueClass::Graphics,
                }),
            ]);
        }
        ops.push(CommandOp::Barrier(ResourceBarrier {
            resource: snapshot,
            subresources: None,
            before: TextureUsageState::TransferDst,
            after: TextureUsageState::ShaderRead,
            src_queue: QueueClass::Transfer,
            dst_queue: QueueClass::Graphics,
        }));
        let mut append_single = |source_set: Handle,
                                 output: usize,
                                 pass: Handle,
                                 target: Handle,
                                 view: Handle,
                                 radius: f32,
                                 dir: [f32; 2]| {
            let mut bytes = vec![0u8; 64];
            bytes[..4].copy_from_slice(&dir[0].to_le_bytes());
            bytes[4..8].copy_from_slice(&dir[1].to_le_bytes());
            bytes[8..12].copy_from_slice(&radius.to_le_bytes());
            push_uniform_write(&mut ops, uniform, bytes);
            let before = if spider_initialized[output] {
                TextureUsageState::ShaderRead
            } else {
                TextureUsageState::Undefined
            };
            ops.push(CommandOp::Barrier(ResourceBarrier {
                resource: spider_textures[output],
                subresources: None,
                before,
                after: TextureUsageState::ColorAttachment,
                src_queue: QueueClass::Graphics,
                dst_queue: QueueClass::Graphics,
            }));
            ops.extend([
                CommandOp::BeginPass {
                    pass,
                    target,
                    colors: vec![PassAttachment {
                        view,
                        load_op: AttachmentLoadOp::DontCare,
                        store_op: AttachmentStoreOp::Store,
                        clear_color: None,
                    }],
                    depth_stencil: None,
                },
                CommandOp::BindGraphicsPipeline(box_pipeline),
                CommandOp::BindResourceSet {
                    pipeline_layout: single_layout,
                    set_index: 0,
                    set: source_set,
                    dynamic_offsets: Vec::new(),
                },
                CommandOp::Draw {
                    vertices: 3,
                    instances: 1,
                },
                CommandOp::EndPass,
                CommandOp::Barrier(ResourceBarrier {
                    resource: spider_textures[output],
                    subresources: None,
                    before: TextureUsageState::ColorAttachment,
                    after: TextureUsageState::ShaderRead,
                    src_queue: QueueClass::Graphics,
                    dst_queue: QueueClass::Graphics,
                }),
            ]);
            spider_initialized[output] = true;
        };
        append_single(
            single_sets[0],
            2,
            passes[2],
            targets[2],
            views[2],
            15.0,
            [1.0, 0.0],
        );
        append_single(
            single_sets[1],
            0,
            passes[0],
            targets[0],
            views[0],
            15.0,
            [0.0, 1.0],
        );
        append_single(
            single_sets[0],
            2,
            passes[2],
            targets[2],
            views[2],
            7.0,
            [1.0, 0.0],
        );
        append_single(
            single_sets[1],
            1,
            passes[1],
            targets[1],
            views[1],
            7.0,
            [0.0, 1.0],
        );
        drop(append_single);
        let mut append_clip = |set: Handle,
                               output: usize,
                               pass: Handle,
                               target: Handle,
                               view: Handle,
                               scale: [f32; 2],
                               offset: [f32; 2],
                               rotation: f32,
                               scissor: [f32; 4],
                               vignette: [f32; 4]| {
            let mut bytes = vec![0u8; 64];
            for (index, value) in [
                scale[0],
                scale[1],
                offset[0],
                offset[1],
                rotation,
                0.0,
                0.0,
                0.0,
                scissor[0],
                scissor[1],
                scissor[2],
                scissor[3],
                vignette[0],
                vignette[1],
                vignette[2],
                vignette[3],
            ]
            .into_iter()
            .enumerate()
            {
                bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
            }
            push_uniform_write(&mut ops, uniform, bytes);
            let before = if spider_initialized[output] {
                TextureUsageState::ShaderRead
            } else {
                TextureUsageState::Undefined
            };
            ops.push(CommandOp::Barrier(ResourceBarrier {
                resource: spider_textures[output],
                subresources: None,
                before,
                after: TextureUsageState::ColorAttachment,
                src_queue: QueueClass::Graphics,
                dst_queue: QueueClass::Graphics,
            }));
            ops.extend([
                CommandOp::BeginPass {
                    pass,
                    target,
                    colors: vec![PassAttachment {
                        view,
                        load_op: AttachmentLoadOp::DontCare,
                        store_op: AttachmentStoreOp::Store,
                        clear_color: None,
                    }],
                    depth_stencil: None,
                },
                CommandOp::BindGraphicsPipeline(clip_pipeline),
                CommandOp::BindResourceSet {
                    pipeline_layout: dual_layout,
                    set_index: 0,
                    set,
                    dynamic_offsets: Vec::new(),
                },
                CommandOp::Draw {
                    vertices: 3,
                    instances: 1,
                },
                CommandOp::EndPass,
                CommandOp::Barrier(ResourceBarrier {
                    resource: spider_textures[output],
                    subresources: None,
                    before: TextureUsageState::ColorAttachment,
                    after: TextureUsageState::ShaderRead,
                    src_queue: QueueClass::Graphics,
                    dst_queue: QueueClass::Graphics,
                }),
            ]);
            spider_initialized[output] = true;
        };
        let full = [0.0, 0.0, 1.0, 1.0];
        let vignette = [0.1, 0.1, 0.9, 0.9];
        append_clip(
            dual_sets[0],
            2,
            passes[2],
            targets[2],
            views[2],
            [1.25, 2.0],
            [-0.125, -0.1],
            0.0,
            full,
            vignette,
        );
        append_clip(
            dual_sets[1],
            3,
            passes[3],
            targets[3],
            views[3],
            [2.35, 4.2],
            [-1.1, -1.5],
            -45.0,
            [0.21, 0.0, 0.79, 1.0],
            [0.31, 0.1, 0.69, 0.9],
        );
        append_clip(
            dual_sets[2],
            2,
            passes[2],
            targets[2],
            views[2],
            [2.35, 2.35],
            [-0.385, -1.29],
            0.0,
            full,
            vignette,
        );
        let mut blit_bytes = vec![0u8; 64];
        blit_bytes[..4].copy_from_slice(&1.0f32.to_le_bytes());
        blit_bytes[4..8].copy_from_slice(&1.0f32.to_le_bytes());
        blit_bytes[8..12].copy_from_slice(&1.0f32.to_le_bytes());
        blit_bytes[12..16].copy_from_slice(&1.0f32.to_le_bytes());
        push_uniform_write(&mut ops, uniform, blit_bytes);
        ops.extend([
            CommandOp::BeginPass {
                pass: final_pass,
                target: render_target,
                colors: vec![loaded_frame_color_attachment(color_attachment)],
                depth_stencil: None,
            },
            CommandOp::BindGraphicsPipeline(blit_pipeline),
            CommandOp::BindResourceSet {
                pipeline_layout: single_layout,
                set_index: 0,
                set: single_sets[1],
                dynamic_offsets: Vec::new(),
            },
            CommandOp::Draw {
                vertices: 3,
                instances: 1,
            },
            CommandOp::EndPass,
        ]);
        self.blur_snapshot_initialized = true;
        self.spider_initialized = spider_initialized;
        Ok(ops)
    }

    pub(crate) fn append_frame_ops_with_affine_quads_and_mesh_batches_to_target(
        &mut self,
        gal: &mut VulkanicGal,
        generation: u64,
        render_target: Handle,
        color_attachment: Handle,
        render_pass: Option<Handle>,
        depth_attachment: Option<Handle>,
        depth_format: Option<TextureFormat>,
        pre_present_y_flip: bool,
        requests: Vec<GuiSpriteRequest>,
        affine_quads: Vec<GuiAffineQuadRequest>,
        mesh_batches: Vec<GuiMeshBatchRequest>,
    ) -> GalResult<(Vec<CommandOp>, GuiSubmitStats)> {
        self.append_frame_ops_with_tiled_quads_to_target(gal, generation, render_target,
            color_attachment, render_pass, depth_attachment, depth_format, pre_present_y_flip,
            requests, affine_quads, mesh_batches, Vec::new())
    }

    pub(crate) fn append_frame_ops_with_tiled_quads_to_target(
        &mut self, gal: &mut VulkanicGal, generation: u64, render_target: Handle,
        color_attachment: Handle, render_pass: Option<Handle>, depth_attachment: Option<Handle>,
        depth_format: Option<TextureFormat>, pre_present_y_flip: bool,
        requests: Vec<GuiSpriteRequest>, affine_quads: Vec<GuiAffineQuadRequest>,
        mesh_batches: Vec<GuiMeshBatchRequest>, tiled_quads: Vec<GuiTiledQuadRequest>,
    ) -> GalResult<(Vec<CommandOp>, GuiSubmitStats)> {
        self.append_frame_ops_with_owned_atlases_to_target(gal, None, generation, render_target,
            color_attachment, render_pass, depth_attachment, depth_format, pre_present_y_flip,
            requests, affine_quads, mesh_batches, tiled_quads)
    }

    /// Prepare all full-item rasters before GUI ordering consumes their image
    /// cells. Source identity is immutable semantic data, not a Java cache key.
    fn prepare_full_item_rasters(&mut self, gal: &mut VulkanicGal,
        world: &mut super::world_primitive_frontend::WorldPrimitiveFrontend,
        ordered: &[GuiFrameRequest], color: ColorFormat, depth: Option<TextureFormat>,
        stats: &mut GuiSubmitStats, ops: &mut Vec<CommandOp>)
        -> GalResult<BTreeMap<u64, (GuiMeshCompositeKey, super::gui_item_raster::GuiItemRasterPlacement, u64)>> {
        let mut items: Vec<_> = ordered.iter().filter_map(|entry| match entry {
            GuiFrameRequest::AffineBatch(batch) => Some(batch.iter()), _ => None,
        }).flatten().filter(|request| request.item_raster_scale != 0).collect();
        items.sort_by_key(|request| request.sequence);
        self.prepare_item_raster_groups(gal, world,
            &items.into_iter().map(GuiItemRasterGroup::single).collect::<Vec<_>>(), color, depth, stats, ops)
    }

    /// One ordered layer group becomes one offscreen item and one presentation.
    /// This private native entry point is shared by single-quad lowering and
    /// grouped-raster tests; multi-layer Java admission remains disabled.
    fn prepare_item_raster_groups(&mut self, gal: &mut VulkanicGal,
        world: &mut super::world_primitive_frontend::WorldPrimitiveFrontend,
        items: &[GuiItemRasterGroup], color: ColorFormat, depth: Option<TextureFormat>,
        stats: &mut GuiSubmitStats, ops: &mut Vec<CommandOp>)
        -> GalResult<BTreeMap<u64, (GuiMeshCompositeKey, super::gui_item_raster::GuiItemRasterPlacement, u64)>> {
        use super::gui_item_raster::{GuiItemRasterIdentity, GuiItemRasterTarget, MAX_ITEM_LAYERS};
        if items.is_empty() { return Ok(BTreeMap::new()); }
        if items.len() > GUI_MAX_MESH_BATCHES || items.iter().any(|item|
            item.layers.is_empty() || item.layers.len() > MAX_ITEM_LAYERS)
            || items.iter().map(|item| item.layers.len()).sum::<usize>() > GUI_MAX_RAW_IMAGES {
            return Err(GalError::invalid_argument("full-item raster frame exceeds bounded composite stream"));
        }
        let scale = items[0].presentation.item_raster_scale;
        let mut sequences = BTreeSet::new();
        let mut identities = BTreeMap::new();
        let mut unique_identities = Vec::new();
        let mut unique = Vec::new();
        let mut indices = Vec::new();
        for item in items {
            let request = &item.presentation;
            validate_affine_quad(request)?;
            if scale == 0 || request.item_raster_scale != scale
                || request.sequence == 0 || !sequences.insert(request.sequence) {
                return Err(GalError::invalid_argument("item rasters require one explicit frame GUI scale"));
            }
            let mut prepared = item.clone();
            let mut identity = Vec::with_capacity(prepared.layers.len());
            for layer in &mut prepared.layers {
                layer.geometry = layer.model_transform.lower(layer.geometry)?;
                layer.model_transform = Default::default();
                layer.material.color(argb_to_rgba(layer.color_argb))?;
                let lighting = match layer.material {
                    super::gui_item_material::GuiAffineMaterial::FlatItem(value)
                    | super::gui_item_material::GuiAffineMaterial::FlatItemCutout(value) => value,
                    _ => return Err(GalError::invalid_argument("item layer requires explicit item material")),
                };
                if lighting.rgb.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
                    return Err(GalError::invalid_argument("invalid item layer lighting"));
                }
                let reference = self.atlas_references.resolve(layer.asset_id,
                    |id| world.accepted_gui_atlas_incarnation(id))?;
                identity.push(GuiItemRasterIdentity {asset_id:layer.asset_id,color_argb:layer.color_argb,
                    lighting_rgb:lighting.rgb.map(|v| if v == 0.0 {0} else {v.to_bits()}),
                    cutout:layer.material.is_cutout(),
                    atlas_generation:reference.atlas.generation,texture_id:reference.atlas.texture_id,
                    geometry:layer.geometry.identity()?,
                    uv:super::gui_item_raster::item_uv_identity(layer.uv)?,
                    region:[reference.x,reference.y,reference.width,reference.height]});
            }
            let index = *identities.entry(identity.clone()).or_insert_with(|| {
                unique_identities.push(identity);
                unique.push(prepared); (unique.len()-1) as u32
            });
            indices.push(index);
        }
        let mut next_slots = self.item_raster_slots.clone();
        let placements = next_slots.prepare_groups(scale,&unique_identities,4096)?;
        let [width,height] = placements[0].target_extent;
        let key = GuiMeshCompositeKey {item_identity:0,width,height,color_format:color,depth_format:depth};
        if !self.item_rasters.contains_key(&key) {
            let pixels: u64 = self.item_rasters.keys().map(|key| u64::from(key.width)*u64::from(key.height)).sum();
            if self.item_rasters.len() >= 4 || pixels + u64::from(width)*u64::from(height) > 32*1024*1024 {
                return Err(GalError::invalid_argument("item raster residency budget exhausted"));
            }
            let target = GuiItemRasterTarget::create(gal, Extent3d {width,height,depth:1})?;
            let composite = match GuiMeshCompositeResources::create(gal, "gui.item-raster.composite", color, depth, target.view) {
                Ok(composite) => composite,
                Err(error) => { let _ = target.destroy(gal); return Err(error); }
            };
            self.item_rasters.insert(key, GuiItemRasterResources {target,composite,usage:TextureUsageState::Undefined});
        }
        let resource = self.item_rasters.get(&key).expect("created item raster");
        let (pass,target,view,image) = (resource.target.pass,resource.target.target,resource.target.view,resource.target.color);
        let before = resource.usage;
        ops.push(CommandOp::Barrier(texture_barrier(image,before,TextureUsageState::ColorAttachment)));
        ops.push(CommandOp::BeginPass {pass,target,
            colors:vec![PassAttachment {view,load_op:AttachmentLoadOp::Clear,store_op:AttachmentStoreOp::Store,
                clear_color:Some(super::commands::ClearColor {r:0.0,g:0.0,b:0.0,a:0.0})}],depth_stencil:None});
        ops.push(CommandOp::EndPass);
        let mut raster_requests = Vec::new();
        for (index, item) in unique.into_iter().enumerate() {
          for layer in item.layers {
            let mut source = item.presentation.clone();
            source.item_raster_scale = 0;
            source.item_raster_layers.clear();
            source.sequence = raster_requests.len() as u64 + 1;
            source.asset_id = layer.asset_id;
            source.color_argb = layer.color_argb;
            source.material = layer.material;
            source.item_raster_geometry = layer.geometry;
            [source.u0,source.v0,source.u1,source.v1] = layer.uv;
            [source.x0,source.y0,source.x1,source.y1,source.x3,source.y3] = layer.geometry.corners;
            raster_requests.push((placements[index],source));
          }
        }
        ops.extend(self.append_owned_item_raster_quads(gal,world,pass,target,view,&raster_requests)?);
        self.item_rasters.get_mut(&key).unwrap().usage = TextureUsageState::ColorAttachment;
        self.item_raster_slots = next_slots;
        stats.owned_intermediate_targets.push(target);
        items.iter().zip(indices).enumerate().map(|(offset,(item,index))| {
            Ok((item.presentation.sequence,(key,placements[index as usize],
                offset as u64*GUI_MESH_COMPOSITE_UNIFORM_STRIDE)))
        }).collect()
    }

    fn preflight_owned_atlas_commands(
        &self, world: Option<&super::world_primitive_frontend::WorldPrimitiveFrontend>,
        affine_quads: &[GuiAffineQuadRequest], tiled_quads: &[GuiTiledQuadRequest],
    ) -> GalResult<bool> {
        if affine_quads.iter().any(|quad| quad.item_raster_scale != 0 &&
            (!self.atlas_references.contains(quad.asset_id) || quad.item_raster_layers.iter().any(|layer|
                !self.atlas_references.contains(layer.asset_id)))) {
            return Err(GalError::invalid_argument("item raster requires an explicit owner-backed atlas source"));
        }
        let has_atlases = affine_quads.iter().any(|quad| self.atlas_references.contains(quad.asset_id))
            || tiled_quads.iter().any(|quad| self.atlas_references.contains(quad.asset_id));
        if has_atlases {
            let owner = world.ok_or_else(|| GalError::unsupported_feature(
                "GUI atlas commands require explicit world owner access"))?;
            owner.require_gui_atlas_upload_boundary()?;
            for asset in affine_quads.iter().map(|quad| quad.asset_id)
                .chain(affine_quads.iter().flat_map(|quad|quad.item_raster_layers.iter().map(|layer|layer.asset_id)))
                .chain(tiled_quads.iter().map(|quad| quad.asset_id)) {
                if self.atlas_references.contains(asset) {
                    self.atlas_references.resolve(asset, |id| owner.accepted_gui_atlas_incarnation(id))?;
                }
            }
        }
        Ok(has_atlases)
    }

    fn preflight_mesh_atlas_commands(&self,
        world: Option<&super::world_primitive_frontend::WorldPrimitiveFrontend>,
        batches: &[GuiMeshBatchRequest]) -> GalResult<()> {
        for batch in batches {
            let atlas = self.atlas_references.contains(batch.asset_id);
            if batch.item_raster_scale != 0 && !atlas && !self.raw_images.contains_key(&batch.asset_id) {
                return Err(GalError::invalid_argument("native item mesh requires an explicitly owned atlas or image resource"));
            }
            if !atlas { continue; }
            if !mesh_atlas_contract_supported(batch) {
                return Err(GalError::unsupported_feature("GUI mesh atlas sampling requires an explicit inventory base or front-model overlay layer"));
            }
            let owner = world.ok_or_else(|| GalError::invalid_argument("GUI mesh atlas sampling requires its explicit Rust image owner"))?;
            owner.require_gui_atlas_upload_boundary()?;
            let reference = self.atlas_references.resolve(batch.asset_id,
                |id| owner.accepted_gui_atlas_incarnation(id))?;
            for vertex in &batch.vertices { reference.atlas_uv(vertex.local_uv)?; }
        }
        Ok(())
    }

    /// Same mixed scheduler and target, with explicit access to the image owner.
    pub(crate) fn append_frame_ops_with_owned_atlases_to_target(
        &mut self, gal: &mut VulkanicGal,
        mut world: Option<&mut super::world_primitive_frontend::WorldPrimitiveFrontend>,
        generation: u64, render_target: Handle, color_attachment: Handle,
        render_pass: Option<Handle>, depth_attachment: Option<Handle>,
        depth_format: Option<TextureFormat>, pre_present_y_flip: bool,
        requests: Vec<GuiSpriteRequest>, affine_quads: Vec<GuiAffineQuadRequest>,
        mesh_batches: Vec<GuiMeshBatchRequest>, tiled_quads: Vec<GuiTiledQuadRequest>,
    ) -> GalResult<(Vec<CommandOp>, GuiSubmitStats)> {
        preflight_tiled_affine_count(&tiled_quads, affine_quads.len())?;
        let has_atlases = self.preflight_owned_atlas_commands(world.as_deref(), &affine_quads, &tiled_quads)?;
        self.preflight_mesh_atlas_commands(world.as_deref(), &mesh_batches)?;
        let needs_depth = affine_quads
            .iter()
            .any(|request| request.stratum == GUI_LEQUAL_DEPTH_BLIT_STRATUM)
            || tiled_quads.iter().any(|request| request.stratum == GUI_LEQUAL_DEPTH_BLIT_STRATUM);
        let infer_depth = needs_depth && depth_attachment.is_none();
        let (depth_attachment, depth_format) = if needs_depth && depth_attachment.is_none() {
            let Some((_, view)) = gal.pass_target_depth_attachment(render_target)? else {
                return Err(GalError::backend(
                    "depth-tested GUI semantic blit requires the Rust-owned frame depth attachment",
                ));
            };
            (Some(view), Some(TextureFormat::Depth32Float))
        } else {
            (depth_attachment, depth_format)
        };
        let render_pass = if infer_depth { None } else { render_pass };
        if mesh_batches.is_empty() && tiled_quads.is_empty() && !has_atlases {
            return self.append_frame_ops_with_affine_quads_to_target(
                gal,
                generation,
                render_target,
                color_attachment,
                render_pass,
                depth_attachment,
                depth_format,
                pre_present_y_flip,
                requests,
                affine_quads,
            );
        }
        let ordered = order_gui_requests_with_tiles(requests, affine_quads, mesh_batches, tiled_quads)?;
        self.mesh_geometry_transaction = self.mesh_geometry_transaction.checked_add(1)
            .ok_or_else(|| GalError::invalid_argument("GUI mesh transaction identity exhausted"))?;
        if generation != self.generation {
            self.destroy_render_resources(gal);
            self.generation = generation;
        }
        // The backend reads mesh streams asynchronously. Reclaim only ranges
        // whose submission has completed, so animated semantic meshes cannot
        // exhaust permanent residency or overwrite in-flight vertices.
        self.reclaim_completed_mesh_geometry(gal.poll_completed());
        let pending_submission = gal.next_submission_id();
        let color_format = gal.pass_target_color_format(render_target)?;
        let frame_pass = match render_pass {
            Some(pass) => pass,
            None => self.frame_pass(gal, render_target, depth_format)?,
        };
        self.mesh_composite_uniform_cursor = 0;
        let mut stats = GuiSubmitStats::default();
        let mut ops = Vec::new();
        if gal.capabilities().api == BackendApi::Vulkan {
            ops.push(CommandOp::BeginPass {
                pass: frame_pass,
                target: render_target,
                colors: vec![loaded_frame_color_attachment(color_attachment)],
                depth_stencil: depth_attachment.map(loaded_frame_depth_attachment),
            });
            ops.push(CommandOp::EndPass);
        }
        let item_raster_placements = if ordered.iter().any(|entry| matches!(entry,
            GuiFrameRequest::AffineBatch(batch) if batch.iter().any(|request| request.item_raster_scale != 0))) {
            let creates = gal.metrics().resource_creates;
            let owner = world.as_deref_mut().ok_or_else(|| GalError::invalid_argument("item raster owner missing"))?;
            let placements = self.prepare_full_item_rasters(gal,owner,&ordered,color_format,depth_format,&mut stats,&mut ops)?;
            stats.resource_creates += gal.metrics().resource_creates-creates;
            placements
        } else {BTreeMap::new()};
        let mut pending_gui_batches = Vec::new();
        for request in ordered {
            match request {
                GuiFrameRequest::Sprite(request) => {
                    let def = sprite_def(request.sprite_id)?;
                    if request.stratum != def.stratum {
                        return Err(GalError::ffi(
                            StatusCode::InvalidArgument,
                            format!(
                                "GUI sprite '{}' requested stratum {} but registry stratum is {}",
                                def.name, request.stratum, def.stratum
                            ),
                        ));
                    }
                    validate_request(&request, def)?;
                    self.ensure_resources(gal, def.group, color_format, depth_format, &mut stats)?;
                    append_gui_quad(
                        &mut pending_gui_batches,
                        request.stratum,
                        def.group,
                        self.pack_sprite(&request, def, pre_present_y_flip)?,
                    );
                    stats.sprite_count = stats.sprite_count.saturating_add(1);
                }
                GuiFrameRequest::AffineBatch(requests) => {
                    let first = requests.first().ok_or_else(|| {
                        GalError::ffi(StatusCode::InvalidArgument, "empty GUI affine batch")
                    })?;
                    if first.item_raster_scale != 0 {
                        append_gui_batches_ops(self,frame_pass,render_target,color_attachment,
                            depth_attachment,color_format,depth_format,&pending_gui_batches,&mut ops)?;
                        stats.sprite_batch_count += pending_gui_batches.len() as u64;
                        pending_gui_batches.clear();
                        for request in &requests {
                            let (key,placement,offset) = item_raster_placements.get(&request.sequence)
                                .ok_or_else(|| GalError::invalid_argument("item raster command not prepared"))?;
                            let resource = self.item_rasters.get_mut(key).expect("prepared raster resource");
                            resource.composite.append_item_raster_composite(resource.target.color,resource.usage,
                                *placement,frame_pass,render_target,color_attachment,depth_attachment,request,
                                pre_present_y_flip,*offset,&mut ops)?;
                            resource.usage = TextureUsageState::ShaderRead;
                        }
                        stats.affine_quad_count += requests.len() as u64;
                        stats.sprite_batch_count += requests.len() as u64;
                        continue;
                    }
                    if self.atlas_references.contains(first.asset_id) {
                        append_gui_batches_ops(self, frame_pass, render_target, color_attachment,
                            depth_attachment, color_format, depth_format, &pending_gui_batches, &mut ops)?;
                        stats.sprite_batch_count += pending_gui_batches.len() as u64;
                        pending_gui_batches.clear();
                        let owner = world.as_deref_mut().ok_or_else(|| GalError::unsupported_feature(
                            "GUI atlas commands require explicit world owner access"))?;
                        let cached = self.resources.contains_key(&ResourceKey::new(
                            dynamic_texture_group(first.stratum, first.asset_id), color_format, depth_format));
                        let creates_before = gal.metrics().resource_creates;
                        let atlas_ops = self.append_scheduled_owned_atlas_quads(gal, owner, frame_pass,
                            render_target, color_attachment, depth_attachment, color_format,
                            depth_format, pre_present_y_flip, &requests, false)?;
                        stats.resource_creates += gal.metrics().resource_creates - creates_before;
                        if cached { stats.cache_hits += 1; } else { stats.cache_misses += 1; }
                        stats.affine_quad_count += requests.len() as u64;
                        stats.sprite_batch_count += atlas_ops.iter().filter(|op|
                            matches!(op, CommandOp::DrawIndexed { .. })).count() as u64;
                        ops.extend(atlas_ops);
                        continue;
                    }
                    let image_format = self
                        .raw_images
                        .get(&first.asset_id)
                        .ok_or_else(|| {
                            GalError::ffi(
                                StatusCode::InvalidArgument,
                                format!("unknown raw GUI image asset {}", first.asset_id),
                            )
                        })?
                        .format;
                    let group = dynamic_texture_group(first.stratum, first.asset_id);
                    self.ensure_resources(gal, group, color_format, depth_format, &mut stats)?;
                    for request in &requests {
                        if request.stratum != first.stratum || request.asset_id != first.asset_id {
                            return Err(GalError::ffi(
                                StatusCode::InvalidArgument,
                                "GUI affine batch changed semantic texture group",
                            ));
                        }
                        validate_affine_quad(request)?;
                        append_gui_quad(
                            &mut pending_gui_batches,
                            request.stratum,
                            group,
                            PackedGuiQuad {
                                origin: [request.x0, request.y0],
                                axis_u: [request.x1 - request.x0, request.y1 - request.y0],
                                axis_v: [request.x3 - request.x0, request.y3 - request.y0],
                                viewport: request.projection_extent,
                                clip: [
                                    request.clip_left as f32,
                                    request.clip_top as f32,
                                    (request.clip_left + request.clip_width) as f32,
                                    (request.clip_top + request.clip_height) as f32,
                                ],
                                clip_enabled: request.clip_mode == 1,
                                pre_present_y_flip,
                                uv: [
                                    request.u0,
                                    request.v0,
                                    request.u1 - request.u0,
                                    request.v1 - request.v0,
                                ],
                                color: request.material.color(argb_to_rgba(request.color_argb))?,
                                texture_mode: image_format.shader_mode(),
                                z: request.z,
                            },
                        );
                    }
                    stats.affine_quad_count = stats
                        .affine_quad_count
                        .saturating_add(requests.len() as u64);
                }
                GuiFrameRequest::Affine(_) => unreachable!("ordered affine requests are coalesced"),
                GuiFrameRequest::Mesh(item) => {
                    if !pending_gui_batches.is_empty() {
                        stats.sprite_batch_count = stats
                            .sprite_batch_count
                            .saturating_add(pending_gui_batches.len() as u64);
                        append_gui_batches_ops(
                            self,
                            frame_pass,
                            render_target,
                            color_attachment,
                            depth_attachment,
                            color_format,
                            depth_format,
                            &pending_gui_batches,
                            &mut ops,
                        )?;
                        pending_gui_batches.clear();
                    }
                    let mesh_ops = self.append_mesh_items_to_target(
                        gal,
                        world.as_deref_mut(),
                        generation,
                        pending_submission,
                        render_target,
                        color_attachment,
                        Some(frame_pass),
                        depth_attachment,
                        depth_format,
                        item.layers,
                        &mut stats,
                    )?;
                    ops.extend(mesh_ops);
                }
            }
        }
        if !pending_gui_batches.is_empty() {
            stats.sprite_batch_count = stats
                .sprite_batch_count
                .saturating_add(pending_gui_batches.len() as u64);
            append_gui_batches_ops(
                self,
                frame_pass,
                render_target,
                color_attachment,
                depth_attachment,
                color_format,
                depth_format,
                &pending_gui_batches,
                &mut ops,
            )?;
        }
        stats.command_lists = 1;
        stats.command_ops = ops.len() as u64;
        Self::require_gui_draw_receipt(&stats, &ops)?;
        Ok((ops, stats))
    }

    /// A non-empty semantic GUI submission must lower to at least one explicit
    /// Rust draw. Copies, barriers, and target setup alone cannot satisfy the GUI
    /// contract or justify publishing the frame.
    fn require_gui_draw_receipt(stats: &GuiSubmitStats, operations: &[CommandOp]) -> GalResult<()> {
        let semantic_items = stats
            .sprite_count
            .saturating_add(stats.affine_quad_count)
            .saturating_add(stats.mesh_batch_count);
        if semantic_items == 0 {
            return Ok(());
        }
        let draws = operations
            .iter()
            .filter(|operation| {
                matches!(
                    operation,
                    CommandOp::Draw { .. } | CommandOp::DrawIndexed { .. }
                )
            })
            .count();
        if draws == 0 {
            return Err(GalError::backend(format!(
                "GUI source writer recorded {semantic_items} semantic items but no draw operations"
            )));
        }
        Ok(())
    }

    /// Appends one complete Rust-owned standard-3D GUI-item family. The caller
    /// supplies copied semantic mesh batches only; source images are resolved
    /// from the existing Rust GUI asset generation, each item is rasterized
    /// into an owned PIP target, and its result is composed through the normal
    /// GUI target. This remains private until the frame ABI and route select it.
    pub(crate) fn append_mesh_items_to_target(
        &mut self,
        gal: &mut VulkanicGal,
        mut world: Option<&mut super::world_primitive_frontend::WorldPrimitiveFrontend>,
        generation: u64,
        pending_submission: SubmissionId,
        render_target: Handle,
        color_attachment: Handle,
        render_pass: Option<Handle>,
        depth_attachment: Option<Handle>,
        depth_format: Option<TextureFormat>,
        mesh_batches: Vec<GuiMeshBatchRequest>,
        stats: &mut GuiSubmitStats,
    ) -> GalResult<Vec<CommandOp>> {
        self.preflight_mesh_atlas_commands(world.as_deref(), &mesh_batches)?;
        if generation != self.generation {
            self.destroy_render_resources(gal);
            self.generation = generation;
        }
        let mut prepared = prepare_gui_mesh_draws(&mesh_batches)?;
        // Resolve exact owner incarnations before allocating any mesh resources.
        // The geometry keeps original sprite-local UVs until this native boundary.
        for (batch, draw) in mesh_batches.iter().zip(&mut prepared) {
            if !self.atlas_references.contains(draw.asset_id) { continue; }
            if !mesh_atlas_contract_supported(batch) {
                return Err(GalError::unsupported_feature("GUI mesh atlas sampling requires an explicit inventory base or front-model overlay layer"));
            }
            let owner = world.as_deref().ok_or_else(|| GalError::invalid_argument(
                "GUI mesh atlas sampling requires its explicit Rust image owner"))?;
            let reference = self.atlas_references.resolve(draw.asset_id,
                |id| owner.accepted_gui_atlas_incarnation(id))?;
            for vertex in &mut draw.vertices {
                vertex.local_uv = reference.atlas_uv(vertex.local_uv)?;
            }
        }
        prepared.sort_by_key(|draw| (draw.stratum, draw.sequence, draw.layer_index));
        if prepared.is_empty() {
            return Ok(Vec::new());
        }
        let frame_pass = match render_pass {
            Some(pass) => pass,
            None => self.frame_pass(gal, render_target, depth_format)?,
        };
        let color_format = gal.pass_target_color_format(render_target)?;
        let mut operations = Vec::new();
        let mut cursor = 0;
        while cursor < prepared.len() {
            let first = &prepared[cursor];
            let group_key = (first.stratum, first.sequence);
            let group_end = prepared[cursor..]
                .iter()
                .position(|draw| (draw.stratum, draw.sequence) != group_key)
                .map(|offset| cursor + offset)
                .unwrap_or(prepared.len());
            let item_layers = &prepared[cursor..group_end];
            validate_mesh_item_layers(item_layers)?;
            stats.mesh_item_count = stats.mesh_item_count.saturating_add(1);
            // Frozen submits the title panorama straight to its main frame
            // target. Keep that native-resolution semantic pass distinct from
            // standard-3D item PIP work, whose private target is part of its
            // actual rendering contract.
            if first.material_mode == GuiMeshMaterialMode::Panorama {
                if item_layers.len() != 1 {
                    return Err(GalError::ffi(
                        StatusCode::InvalidArgument,
                        "a semantic panorama must contain exactly one fullscreen layer",
                    ));
                }
                let texture_group = dynamic_mesh_texture_group(first);
                self.ensure_resources(gal, texture_group, color_format, depth_format, stats)?;
                let (texture_view, sampler) = {
                    let raw_resources = self
                        .resources
                        .get(&ResourceKey::new(texture_group, color_format, depth_format))
                        .ok_or_else(|| GalError::backend("GUI panorama image resources vanished before raster"))?;
                    (raw_resources.texture_view, raw_resources.sampler)
                };
                let raster_key = direct_gui_mesh_raster_key(first, color_format);
                if !self.mesh_rasters.contains_key(&raster_key) {
                    if self.mesh_rasters.len() >= GUI_MAX_MESH_RASTER_RESOURCES {
                        return Err(GalError::unsupported_feature(format!(
                            "GUI mesh raster resource cache exceeds bounded limit {}",
                            GUI_MAX_MESH_RASTER_RESOURCES
                        )));
                    }
                    let shared_program = self.ensure_mesh_shared_program(
                        gal,
                        color_format,
                        depth_format,
                        first.material_mode,
                        first.front_face,
                    )?;
                    let raster = GuiMeshPassResources::create_with_shared_program(
                        gal,
                        &format!("minecraft.gui.panorama.asset{}.gen{}", first.asset_id, generation),
                        texture_view,
                        sampler,
                        shared_program,
                    )?;
                    self.mesh_rasters.insert(raster_key, raster);
                    stats.resource_creates = stats.resource_creates.saturating_add(1);
                }
                let geometry_key = (raster_key, gui_mesh_geometry_fingerprint(first), self.mesh_geometry_transaction);
                let vertex_bytes = (first.vertices.len() * super::gui_mesh_frontend::GUI_MESH_GPU_VERTEX_BYTES) as u64;
                let index_bytes = (first.indices.len() * std::mem::size_of::<u32>()) as u64;
                let (stream, reused) = if let Some(residency) = self.mesh_geometry_cache.get_mut(&geometry_key) {
                    residency.last_submission = pending_submission;
                    (residency.stream, true)
                } else {
                    let residency = self.allocate_mesh_geometry(gal, raster_key, vertex_bytes, index_bytes, pending_submission)?;
                    let stream = residency.stream;
                    self.mesh_geometry_cache.insert(geometry_key, residency);
                    (stream, false)
                };
                self.mesh_rasters
                    .get(&raster_key)
                    .ok_or_else(|| GalError::backend("GUI panorama raster resources vanished before draw"))?
                    .append_direct_frame_draw(
                        frame_pass,
                        render_target,
                        color_attachment,
                        depth_attachment,
                        first,
                        stream,
                        !reused,
                        &mut operations,
                    )?;
                stats.mesh_batch_count = stats.mesh_batch_count.saturating_add(1);
                stats.mesh_draw_count = stats.mesh_draw_count.saturating_add(1);
                cursor = group_end;
                continue;
            }
            let item_identity = first.item_cache.map_or(0, |cache| cache.identity);
            let mut target = self.mesh_targets.stage_item(
                gal,
                generation,
                Extent3d {
                    width: first.render_extent[0],
                    height: first.render_extent[1],
                    depth: 1,
                },
                item_identity,
            )?;
            let accepted_raster = gal.render_pass_last_submission(target.pass)?;
            // Preparation may be discarded. Persisted layout state is proven
            // by this pass's accepted submission, or by an earlier item in
            // the same command transaction, never by a previous preparation.
            target.initialized = accepted_raster.is_some()
                || stats.owned_intermediate_targets.contains(&target.target);
            let reuse_pixels = first.item_cache.is_some_and(|cache| !cache.animated)
                && accepted_raster.is_some();
            if !stats.owned_intermediate_targets.contains(&target.target) {
                stats.owned_intermediate_targets.push(target.target);
            }
            for (execution_index, layer_index) in mesh_item_layer_execution_order(item_layers).into_iter().enumerate().filter(|_| !reuse_pixels) {
                let draw = &item_layers[layer_index];
                let texture_group = dynamic_mesh_texture_group(draw);
                if self.atlas_references.contains(draw.asset_id) {
                    self.prepare_owned_atlas_binding_group(gal, world.as_deref_mut().ok_or_else(||
                        GalError::invalid_argument("GUI mesh atlas owner unavailable"))?,
                        texture_group, color_format, depth_format)?;
                }
                self.ensure_resources(
                    gal,
                    texture_group,
                    color_format,
                    depth_format,
                    stats,
                )?;
                let (texture_view, sampler) = {
                    let raw_resources = self
                        .resources
                        .get(&ResourceKey::new(
                            texture_group,
                            color_format,
                            depth_format,
                        ))
                        .ok_or_else(|| {
                            GalError::backend("GUI mesh image resources vanished before raster")
                        })?;
                    (raw_resources.texture_view, raw_resources.sampler)
                };
                let raster_key = gui_mesh_raster_key(draw);
                if !self.mesh_rasters.contains_key(&raster_key) {
                    if self.mesh_rasters.len() >= GUI_MAX_MESH_RASTER_RESOURCES {
                        return Err(GalError::unsupported_feature(format!(
                            "GUI mesh raster resource cache exceeds bounded limit {}",
                            GUI_MAX_MESH_RASTER_RESOURCES
                        )));
                    }
                    let shared_program = self.ensure_mesh_shared_program(
                        gal,
                        TextureFormat::Rgba8Unorm,
                        Some(TextureFormat::Depth32Float),
                        draw.material_mode,
                        draw.front_face,
                    )?;
                    let raster = GuiMeshPassResources::create_with_shared_program(
                        gal,
                        &format!(
                            "minecraft.gui.mesh.asset{}.gen{}",
                            draw.asset_id, generation
                        ),
                        texture_view,
                        sampler,
                        shared_program,
                    )?;
                    self.mesh_rasters.insert(raster_key, raster);
                    stats.resource_creates = stats.resource_creates.saturating_add(1);
                }
                let geometry_key = (raster_key, gui_mesh_geometry_fingerprint(draw), self.mesh_geometry_transaction);
                let vertex_bytes = (draw.vertices.len()
                    * super::gui_mesh_frontend::GUI_MESH_GPU_VERTEX_BYTES)
                    as u64;
                let index_bytes = (draw.indices.len() * std::mem::size_of::<u32>()) as u64;
                let (stream, reused) =
                    if let Some(residency) = self.mesh_geometry_cache.get_mut(&geometry_key) {
                        residency.last_submission = pending_submission;
                        (residency.stream, true)
                    } else {
                        let residency = self.allocate_mesh_geometry(
                            gal,
                            raster_key,
                            vertex_bytes,
                            index_bytes,
                            pending_submission,
                        )?;
                        let stream = residency.stream;
                        self.mesh_geometry_cache.insert(geometry_key, residency);
                        (stream, false)
                    };
                let raster = self.mesh_rasters.get(&raster_key).ok_or_else(|| {
                    GalError::backend("GUI mesh raster resources vanished before draw")
                })?;
                if reused {
                    raster.append_draw_reusing_geometry(
                        target,
                        draw,
                        stream,
                        execution_index == 0,
                        &mut operations,
                    )?;
                } else {
                    raster.append_draw(
                        target,
                        draw,
                        stream,
                        execution_index == 0,
                        &mut operations,
                    )?;
                }
                target.initialized = true;
                stats.mesh_batch_count = stats.mesh_batch_count.saturating_add(1);
                stats.mesh_draw_count = stats.mesh_draw_count.saturating_add(1);
            }
            let composite_key = GuiMeshCompositeKey {
                item_identity,
                width: target.extent.width,
                height: target.extent.height,
                color_format,
                depth_format,
            };
            if !self.mesh_composites.contains_key(&composite_key) {
                if self.mesh_composites.len() >= GUI_MAX_MESH_COMPOSITE_RESOURCES {
                    return Err(GalError::unsupported_feature(format!(
                        "GUI mesh composite resource cache exceeds bounded limit {}",
                        GUI_MAX_MESH_COMPOSITE_RESOURCES
                    )));
                }
                let composite = GuiMeshCompositeResources::create(
                    gal,
                    &format!(
                        "minecraft.gui.mesh.composite.gen{}.{}x{}",
                        generation, target.extent.width, target.extent.height
                    ),
                    color_format,
                    depth_format,
                    target.color_view,
                )?;
                self.mesh_composites.insert(composite_key, composite);
                stats.resource_creates = stats.resource_creates.saturating_add(1);
            }
            let composite = self
                .mesh_composites
                .get(&composite_key)
                .ok_or_else(|| GalError::backend("GUI mesh compositor vanished before draw"))?;
            composite.append_composite(
                target,
                if reuse_pixels { TextureUsageState::ShaderRead } else { TextureUsageState::ColorAttachment },
                frame_pass,
                render_target,
                color_attachment,
                depth_attachment,
                first,
                self.mesh_composite_uniform_cursor,
                &mut operations,
            )?;
            self.mesh_composite_uniform_cursor = self
                .mesh_composite_uniform_cursor
                .checked_add(GUI_MESH_COMPOSITE_UNIFORM_STRIDE)
                .ok_or_else(|| {
                    GalError::ffi(
                        StatusCode::InvalidArgument,
                        "GUI mesh composite uniform stream cursor overflows",
                    )
                })?;
            stats.mesh_draw_count = stats.mesh_draw_count.saturating_add(1);
            cursor = group_end;
        }
        stats.command_ops = stats.command_ops.saturating_add(operations.len() as u64);
        Ok(operations)
    }

    /// Replays GUI semantics into a frame-local diagnostic target without
    /// placing the target in the normal frame-pass cache. The caller retires
    /// the returned pass after the capture submission completes.
    pub(crate) fn append_frame_ops_to_transient_diagnostic_target(
        &mut self,
        gal: &mut VulkanicGal,
        generation: u64,
        render_target: Handle,
        color_attachment: Handle,
        requests: Vec<GuiSpriteRequest>,
        affine_quads: Vec<GuiAffineQuadRequest>,
        mesh_batches: Vec<GuiMeshBatchRequest>,
    ) -> GalResult<(Vec<CommandOp>, GuiSubmitStats)> {
        self.append_frame_ops_to_transient_tiled_diagnostic_target(gal, generation, render_target,
            color_attachment, requests, affine_quads, mesh_batches, Vec::new())
    }

    pub(crate) fn append_frame_ops_to_transient_tiled_diagnostic_target(
        &mut self, gal: &mut VulkanicGal, generation: u64, render_target: Handle,
        color_attachment: Handle, requests: Vec<GuiSpriteRequest>,
        affine_quads: Vec<GuiAffineQuadRequest>, mesh_batches: Vec<GuiMeshBatchRequest>,
        tiled_quads: Vec<GuiTiledQuadRequest>,
    ) -> GalResult<(Vec<CommandOp>, GuiSubmitStats)> {
        preflight_tiled_affine_count(&tiled_quads, affine_quads.len())?;
        validate_gui_frame_sequences(&requests, &affine_quads, &mesh_batches, &tiled_quads)?;
        let pass = gal.create_render_pass(RenderPassDesc {
            label: "minecraft.gui.diagnostic-frame.pass".to_string(),
            target: render_target,
            color_formats: vec![gal.pass_target_color_format(render_target)?],
            depth_format: None,
        })?;
        match self.append_frame_ops_with_tiled_quads_to_target(
            gal,
            generation,
            render_target,
            color_attachment,
            Some(pass),
            None,
            None,
            false,
            requests,
            affine_quads,
            mesh_batches,
            tiled_quads,
        ) {
            Ok((ops, mut stats)) => {
                stats.transient_diagnostic_passes.push(pass);
                Ok((ops, stats))
            }
            Err(error) => {
                let _ = gal.destroy(pass);
                Err(error)
            }
        }
    }

    fn frame_pass(
        &mut self,
        gal: &mut VulkanicGal,
        frame_target: Handle,
        depth_format: Option<TextureFormat>,
    ) -> GalResult<Handle> {
        if let Some(cached) = self.cached_pass {
            if cached.frame_target == frame_target && cached.depth_format == depth_format {
                return Ok(cached.pass);
            }
            gal.destroy(cached.pass)?;
            self.cached_pass = None;
        }
        let pass = gal.create_render_pass(RenderPassDesc {
            label: "minecraft.gui.frame.pass".to_string(),
            target: frame_target,
            color_formats: vec![gal.pass_target_color_format(frame_target)?],
            depth_format,
        })?;
        self.cached_pass = Some(CachedPass {
            frame_target,
            pass,
            depth_format,
        });
        Ok(pass)
    }

    fn ensure_shared_pipeline(
        &mut self,
        gal: &mut VulkanicGal,
        group: TextureGroup,
        color_format: ColorFormat,
        depth_format: Option<TextureFormat>,
    ) -> GalResult<GuiSharedPipeline> {
        let key = GuiSharedPipelineKey::new(group, color_format, depth_format);
        if let Some(pipeline) = self.shared_pipelines.get(&key) {
            return Ok(*pipeline);
        }
        if self.shared_pipelines.len() >= GUI_MAX_SHARED_PIPELINES {
            return Err(GalError::unsupported_feature(format!(
                "GUI shared pipeline cache exceeds bounded limit {GUI_MAX_SHARED_PIPELINES}"
            )));
        }
        let label = format!(
            "minecraft.gui.shared-pipeline.format{}-depth{}-blend{}-compare{}-item{}-cutout{}",
            color_format as u32,
            depth_format.map(|format| format as u32).unwrap_or(0),
            key.blend,
            key.depth_compare,
            key.item_raster,
            key.item_cutout,
        );
        let (vertex_code, fragment_code) = if gal.capabilities().api == BackendApi::Vulkan {
            (VERTEX_SHADER_VULKAN, FRAGMENT_SHADER_VULKAN)
        } else {
            (VERTEX_SHADER_OPENGL, FRAGMENT_SHADER_OPENGL)
        };
        let fragment_code = gui_fragment_material_code(fragment_code, key.item_raster, key.item_cutout);
        let mut created = Vec::new();
        let result = (|| -> GalResult<GuiSharedPipeline> {
            let vertex_shader = gal.create_shader_module(ShaderModuleDesc {
                label: format!("{label}.vertex"),
                stage: ShaderStage::Vertex,
                code_format: ShaderCodeFormat::Glsl,
                code: vertex_code.to_vec(),
                entry_point: "main".to_string(),
            })?;
            created.push(vertex_shader);
            let fragment_shader = gal.create_shader_module(ShaderModuleDesc {
                label: format!("{label}.fragment"),
                stage: ShaderStage::Fragment,
                code_format: ShaderCodeFormat::Glsl,
                code: fragment_code.to_vec(),
                entry_point: "main".to_string(),
            })?;
            created.push(fragment_shader);
            let resource_layout = gal.create_resource_layout(ResourceLayoutDesc {
                label: format!("{label}.resource-layout"),
                bindings: vec![
                    ResourceBindingDesc {
                        binding: 0,
                        kind: ResourceBindingKind::UniformBuffer,
                        stages: PipelineStageFlags::DRAW,
                        array_count: 1,
                        optional: false,
                        dynamic_offset_count: 0,
                    },
                    ResourceBindingDesc {
                        binding: 1,
                        kind: ResourceBindingKind::SampledTexture,
                        stages: PipelineStageFlags::DRAW,
                        array_count: 1,
                        optional: false,
                        dynamic_offset_count: 0,
                    },
                    ResourceBindingDesc {
                        binding: 2,
                        kind: ResourceBindingKind::Sampler,
                        stages: PipelineStageFlags::DRAW,
                        array_count: 1,
                        optional: false,
                        dynamic_offset_count: 0,
                    },
                ],
            })?;
            created.push(resource_layout);
            let pipeline_layout = gal.create_pipeline_layout(PipelineLayoutDesc {
                label: format!("{label}.pipeline-layout"),
                resource_layouts: vec![resource_layout],
            })?;
            created.push(pipeline_layout);
            let pipeline = gal.create_graphics_pipeline(GraphicsPipelineDesc {
                label: format!("{label}.pipeline"),
                layout: pipeline_layout,
                vertex_shader,
                fragment_shader,
                topology: PrimitiveTopology::Triangles,
                cull_mode: CullMode::None,
                front_face: crate::render::vulkanic::resources::FrontFace::CounterClockwise,
                provoking_vertex: crate::render::vulkanic::resources::ProvokingVertex::Last,
                raster_y_direction: crate::render::vulkanic::resources::RasterYDirection::Up,
                blend: group.blend(),
                depth_compare: if key.depth_compare == 0 {
                    None
                } else {
                    Some(CompareOp::LessOrEqual)
                },
                depth_write: false,
                depth_bias: None,
                color_formats: vec![color_format],
                depth_format,
                stencil: None,
            })?;
            created.push(pipeline);
            Ok(GuiSharedPipeline {
                vertex_shader,
                fragment_shader,
                resource_layout,
                pipeline_layout,
                pipeline,
            })
        })();
        match result {
            Ok(pipeline) => {
                self.shared_pipelines.insert(key, pipeline);
                Ok(pipeline)
            }
            Err(error) => {
                for handle in created.into_iter().rev() {
                    let _ = gal.destroy(handle);
                }
                Err(error)
            }
        }
    }

    fn create_resources(
        &mut self,
        gal: &mut VulkanicGal,
        group: TextureGroup,
        shared_pipeline: GuiSharedPipeline,
    ) -> GalResult<GuiResources> {
        let label = format!("gui-textured-{}-gen{}", group.label(), self.generation);
        let source = self.texture_source(group)?;
        let dynamic_texture_key = TextureGroupKey::from(group)
            .dynamic_asset_id()
            .map(|asset_id| (asset_id, source.format));
        let mut created = Vec::new();
        let mut newly_shared_texture = None;
        let result = (|| -> GalResult<GuiResources> {
            let shared_texture = dynamic_texture_key.and_then(|key| self.dynamic_textures.get(&key).copied());
            let upload_buffer = if let Some(shared) = shared_texture {
                shared.upload_buffer
            } else {
                let upload = gal.create_buffer(BufferDesc {
                label: format!("{label}.texture-upload"),
                size: source.bytes.len() as u64,
                memory: MemoryDomain::Upload,
                usages: vec![
                    BufferUsage::TransferSrc,
                    BufferUsage::TransferDst,
                    BufferUsage::HostWrite,
                ],
                })?;
                created.push(upload);
                upload
            };
            let index_buffer = gal.create_buffer(BufferDesc {
                label: format!("{label}.index"),
                size: index_bytes().len() as u64,
                memory: MemoryDomain::Upload,
                usages: vec![
                    BufferUsage::Index,
                    BufferUsage::TransferDst,
                    BufferUsage::HostWrite,
                ],
            })?;
            created.push(index_buffer);
            let uniform_buffer = gal.create_buffer(BufferDesc {
                label: format!("{label}.uniform"),
                size: GUI_PACKED_UNIFORM_BYTES,
                memory: MemoryDomain::Upload,
                usages: vec![
                    BufferUsage::Uniform,
                    BufferUsage::TransferDst,
                    BufferUsage::HostWrite,
                ],
            })?;
            created.push(uniform_buffer);
            let (upload_buffer, texture, sampler, texture_view, reused_shared_texture) =
                if let Some(key) = dynamic_texture_key {
                    if let Some(shared) = self.dynamic_textures.get(&key).copied() {
                        // Shared dynamic textures are immutable for one raw-image
                        // generation. Reuse them directly so a second blend
                        // stratum does not transiently allocate duplicate native
                        // image objects before immediately destroying them.
                        (
                            shared.upload_buffer,
                            shared.texture,
                            shared.sampler(group.sampling()),
                            shared.texture_view,
                            true,
                        )
                    } else {
                        let texture = gal.create_texture(TextureDesc {
                            label: format!("{label}.texture"),
                            dimension: TextureDimension::D2,
                            format: source.format.texture_format(),
                            extent: Extent3d {
                                width: source.width,
                                height: source.height,
                                depth: 1,
                            },
                            mip_levels: 1,
                            array_layers: 1,
                            usages: vec![TextureUsage::Sampled, TextureUsage::TransferDst],
                        })?;
                        created.push(texture);
                        let nearest_sampler = gal.create_sampler(SamplerDesc {
                            label: format!("{label}.sampler.nearest"),
                            min_filter: SamplerFilter::Nearest,
                            mag_filter: SamplerFilter::Nearest,
                            mip_filter: SamplerFilter::Nearest,
                            address_u: SamplerAddressMode::ClampToEdge,
                            address_v: SamplerAddressMode::ClampToEdge,
                            address_w: SamplerAddressMode::ClampToEdge,
                            comparison: None,
                        })?;
                        created.push(nearest_sampler);
                        let linear_sampler = gal.create_sampler(SamplerDesc {
                            label: format!("{label}.sampler.linear"),
                            min_filter: SamplerFilter::Linear,
                            mag_filter: SamplerFilter::Linear,
                            mip_filter: SamplerFilter::Nearest,
                            address_u: SamplerAddressMode::ClampToEdge,
                            address_v: SamplerAddressMode::ClampToEdge,
                            address_w: SamplerAddressMode::ClampToEdge,
                            comparison: None,
                        })?;
                        created.push(linear_sampler);
                        let texture_view = gal.create_texture_view(TextureViewDesc {
                            label: format!("{label}.texture-view"),
                            texture,
                            format: source.format.texture_format(),
                            base_mip: 0,
                            mip_count: 1,
                            base_layer: 0,
                            layer_count: 1,
                        })?;
                        created.push(texture_view);
                        newly_shared_texture = Some((
                            key,
                            SharedDynamicGuiTexture {
                                upload_buffer,
                                texture,
                                nearest_sampler,
                                linear_sampler,
                                texture_view,
                            },
                        ));
                        (
                            upload_buffer,
                            texture,
                            match group.sampling() {
                                SamplerFilter::Linear => linear_sampler,
                                SamplerFilter::Nearest => nearest_sampler,
                            },
                            texture_view,
                            false,
                        )
                    }
                } else {
                    let texture = gal.create_texture(TextureDesc {
                        label: format!("{label}.texture"),
                        dimension: TextureDimension::D2,
                        format: source.format.texture_format(),
                        extent: Extent3d {
                            width: source.width,
                            height: source.height,
                            depth: 1,
                        },
                        mip_levels: 1,
                        array_layers: 1,
                        usages: vec![TextureUsage::Sampled, TextureUsage::TransferDst],
                    })?;
                    created.push(texture);
                    let sampler = gal.create_sampler(SamplerDesc {
                        label: format!("{label}.sampler"),
                        min_filter: SamplerFilter::Nearest,
                        mag_filter: SamplerFilter::Nearest,
                        mip_filter: SamplerFilter::Nearest,
                        address_u: SamplerAddressMode::ClampToEdge,
                        address_v: SamplerAddressMode::ClampToEdge,
                        address_w: SamplerAddressMode::ClampToEdge,
                        comparison: None,
                    })?;
                    created.push(sampler);
                    let texture_view = gal.create_texture_view(TextureViewDesc {
                        label: format!("{label}.texture-view"),
                        texture,
                        format: source.format.texture_format(),
                        base_mip: 0,
                        mip_count: 1,
                        base_layer: 0,
                        layer_count: 1,
                    })?;
                    created.push(texture_view);
                    (upload_buffer, texture, sampler, texture_view, false)
                };
            let private_sampler = if matches!(group, TextureGroup::DynamicGlint(_)) {
                let TextureGroup::DynamicGlint(asset_id) = group else { unreachable!() };
                let (filter, address) = self.raw_images.get(&asset_id)
                    .and_then(|image| image.sampling)
                    .unwrap_or((SamplerFilter::Linear, SamplerAddressMode::Repeat));
                let sampler = gal.create_sampler(SamplerDesc {
                    label: format!("{label}.sampler.resource-glint"),
                    min_filter: filter,
                    mag_filter: filter,
                    mip_filter: SamplerFilter::Nearest,
                    address_u: address,
                    address_v: address,
                    address_w: address,
                    comparison: None,
                })?;
                created.push(sampler);
                Some(sampler)
            } else { None };
            let sampler = private_sampler.unwrap_or(sampler);
            let resource_set = gal.create_resource_set(ResourceSetDesc {
                label: format!("{label}.resource-set"),
                layout: shared_pipeline.resource_layout,
                bindings: vec![
                    ResourceBinding {
                        binding: 0,
                        array_index: 0,
                        resource: uniform_buffer,
                        kind: ResourceBindingKind::UniformBuffer,
                        access: AccessFlags::READ,
                        dynamic_offsets: Vec::new(),
                        buffer_range: None,
                    },
                    ResourceBinding {
                        binding: 1,
                        array_index: 0,
                        resource: texture_view,
                        kind: ResourceBindingKind::SampledTexture,
                        access: AccessFlags::READ,
                        dynamic_offsets: Vec::new(),
                        buffer_range: None,
                    },
                    ResourceBinding {
                        binding: 2,
                        array_index: 0,
                        resource: sampler,
                        kind: ResourceBindingKind::Sampler,
                        access: AccessFlags::READ,
                        dynamic_offsets: Vec::new(),
                        buffer_range: None,
                    },
                ],
            })?;
            created.push(resource_set);
            let resources = GuiResources {
                private_sampler,
                index_buffer,
                uniform_buffer,
                texture,
                sampler,
                texture_view,
                resource_set,
                pipeline_layout: shared_pipeline.pipeline_layout,
                pipeline: shared_pipeline.pipeline,
                image_ownership: match dynamic_texture_key {
                    Some(key) => GuiImageOwnership::SharedRaw { key },
                    None => GuiImageOwnership::Owned { upload_buffer },
                },
            };
            self.upload_resources(gal, &source, group, &resources,
                (!reused_shared_texture).then_some(upload_buffer))?;
            if let Some((key, shared)) = newly_shared_texture.take() {
                self.dynamic_textures.insert(key, shared);
            }
            Ok(resources)
        })();
        if result.is_err() {
            for handle in created.into_iter().rev() {
                let _ = gal.destroy(handle);
            }
        }
        result
    }

    fn ensure_blur_resources(
        &mut self,
        gal: &mut VulkanicGal,
        width: u32,
        height: u32,
        color_format: ColorFormat,
    ) -> GalResult<&GuiBlurResources> {
        if gal.capabilities().api != BackendApi::Vulkan {
            return Err(GalError::unsupported_feature(
                "Rust GUI blur requires the Vulkan backend",
            ));
        }
        if self.blur_resources.as_ref().is_some_and(|resources| {
            resources.width == width
                && resources.height == height
                && resources.color_format == color_format
        }) {
            return Ok(self.blur_resources.as_ref().unwrap());
        }
        if let Some(previous) = self.blur_resources.take() {
            for handle in previous.handles_in_destroy_order() {
                let _ = gal.destroy(handle);
            }
            self.blur_snapshot_initialized = false;
            self.creeper_intermediate_initialized = false;
            self.spider_initialized = [false; 4];
        }
        let label = format!("minecraft.gui.blur.{}x{}", width, height);
        let mut created = Vec::new();
        let result = (|| -> GalResult<GuiBlurResources> {
            let texture = gal.create_texture(TextureDesc {
                label: format!("{label}.snapshot"),
                dimension: TextureDimension::D2,
                format: color_format,
                extent: Extent3d {
                    width,
                    height,
                    depth: 1,
                },
                mip_levels: 1,
                array_layers: 1,
                usages: vec![TextureUsage::Sampled, TextureUsage::TransferDst],
            })?;
            created.push(texture);
            let texture_view = gal.create_texture_view(TextureViewDesc {
                label: format!("{label}.snapshot-view"),
                texture,
                format: color_format,
                base_mip: 0,
                mip_count: 1,
                base_layer: 0,
                layer_count: 1,
            })?;
            created.push(texture_view);
            let creeper_texture = gal.create_texture(TextureDesc {
                label: format!("{label}.creeper-intermediate"),
                dimension: TextureDimension::D2,
                format: color_format,
                extent: Extent3d {
                    width,
                    height,
                    depth: 1,
                },
                mip_levels: 1,
                array_layers: 1,
                usages: vec![TextureUsage::Sampled, TextureUsage::ColorAttachment],
            })?;
            created.push(creeper_texture);
            let creeper_view = gal.create_texture_view(TextureViewDesc {
                label: format!("{label}.creeper-intermediate-view"),
                texture: creeper_texture,
                format: color_format,
                base_mip: 0,
                mip_count: 1,
                base_layer: 0,
                layer_count: 1,
            })?;
            created.push(creeper_view);
            let creeper_target = gal.create_render_target(RenderTargetDesc {
                label: format!("{label}.creeper-intermediate-target"),
                color_views: vec![creeper_view],
                depth_stencil_view: None,
                extent: Extent3d {
                    width,
                    height,
                    depth: 1,
                },
            })?;
            created.push(creeper_target);
            let creeper_pass = gal.create_render_pass(RenderPassDesc {
                label: format!("{label}.creeper-intermediate-pass"),
                target: creeper_target,
                color_formats: vec![color_format],
                depth_format: None,
            })?;
            created.push(creeper_pass);
            let mut spider_textures = Vec::with_capacity(4);
            let mut spider_views = Vec::with_capacity(4);
            let mut spider_targets = Vec::with_capacity(4);
            let mut spider_passes = Vec::with_capacity(4);
            for name in ["large-blur", "small-blur", "temp", "swap"] {
                let texture = gal.create_texture(TextureDesc {
                    label: format!("{label}.spider-{name}"),
                    dimension: TextureDimension::D2,
                    format: color_format,
                    extent: Extent3d {
                        width,
                        height,
                        depth: 1,
                    },
                    mip_levels: 1,
                    array_layers: 1,
                    usages: vec![TextureUsage::Sampled, TextureUsage::ColorAttachment],
                })?;
                created.push(texture);
                let view = gal.create_texture_view(TextureViewDesc {
                    label: format!("{label}.spider-{name}-view"),
                    texture,
                    format: color_format,
                    base_mip: 0,
                    mip_count: 1,
                    base_layer: 0,
                    layer_count: 1,
                })?;
                created.push(view);
                let target = gal.create_render_target(RenderTargetDesc {
                    label: format!("{label}.spider-{name}-target"),
                    color_views: vec![view],
                    depth_stencil_view: None,
                    extent: Extent3d {
                        width,
                        height,
                        depth: 1,
                    },
                })?;
                created.push(target);
                let pass = gal.create_render_pass(RenderPassDesc {
                    label: format!("{label}.spider-{name}-pass"),
                    target,
                    color_formats: vec![color_format],
                    depth_format: None,
                })?;
                created.push(pass);
                spider_textures.push(texture);
                spider_views.push(view);
                spider_targets.push(target);
                spider_passes.push(pass);
            }
            let uniform_buffer = gal.create_buffer(BufferDesc {
                label: format!("{label}.uniform"),
                size: 64,
                memory: MemoryDomain::Upload,
                usages: vec![
                    BufferUsage::Uniform,
                    BufferUsage::TransferDst,
                    BufferUsage::HostWrite,
                ],
            })?;
            created.push(uniform_buffer);
            let sampler = gal.create_sampler(SamplerDesc {
                label: format!("{label}.sampler"),
                min_filter: SamplerFilter::Linear,
                mag_filter: SamplerFilter::Linear,
                mip_filter: SamplerFilter::Nearest,
                address_u: SamplerAddressMode::ClampToEdge,
                address_v: SamplerAddressMode::ClampToEdge,
                address_w: SamplerAddressMode::ClampToEdge,
                comparison: None,
            })?;
            created.push(sampler);
            let vertex_shader = gal.create_shader_module(ShaderModuleDesc {
                label: format!("{label}.vertex"),
                stage: ShaderStage::Vertex,
                code_format: ShaderCodeFormat::Glsl,
                code: BLUR_VERTEX_SHADER_VULKAN.to_vec(),
                entry_point: "main".to_string(),
            })?;
            created.push(vertex_shader);
            let fragment_shader = gal.create_shader_module(ShaderModuleDesc {
                label: format!("{label}.fragment"),
                stage: ShaderStage::Fragment,
                code_format: ShaderCodeFormat::Glsl,
                code: BLUR_FRAGMENT_SHADER_VULKAN.to_vec(),
                entry_point: "main".to_string(),
            })?;
            created.push(fragment_shader);
            let invert_fragment_shader = gal.create_shader_module(ShaderModuleDesc {
                label: format!("{label}.invert-fragment"),
                stage: ShaderStage::Fragment,
                code_format: ShaderCodeFormat::Glsl,
                code: INVERT_FRAGMENT_SHADER_VULKAN.to_vec(),
                entry_point: "main".to_string(),
            })?;
            created.push(invert_fragment_shader);
            let creeper_color_shader = gal.create_shader_module(ShaderModuleDesc {
                label: format!("{label}.creeper-color-fragment"),
                stage: ShaderStage::Fragment,
                code_format: ShaderCodeFormat::Glsl,
                code: CREEPER_COLOR_FRAGMENT_SHADER_VULKAN.to_vec(),
                entry_point: "main".to_string(),
            })?;
            created.push(creeper_color_shader);
            let creeper_bits_shader = gal.create_shader_module(ShaderModuleDesc {
                label: format!("{label}.creeper-bits-fragment"),
                stage: ShaderStage::Fragment,
                code_format: ShaderCodeFormat::Glsl,
                code: CREEPER_BITS_FRAGMENT_SHADER_VULKAN.to_vec(),
                entry_point: "main".to_string(),
            })?;
            created.push(creeper_bits_shader);
            let spider_box_shader = gal.create_shader_module(ShaderModuleDesc {
                label: format!("{label}.spider-box-blur"),
                stage: ShaderStage::Fragment,
                code_format: ShaderCodeFormat::Glsl,
                code: SPIDER_BOX_BLUR_FRAGMENT_SHADER_VULKAN.to_vec(),
                entry_point: "main".to_string(),
            })?;
            created.push(spider_box_shader);
            let spider_rot_shader = gal.create_shader_module(ShaderModuleDesc {
                label: format!("{label}.spider-rot-scale"),
                stage: ShaderStage::Vertex,
                code_format: ShaderCodeFormat::Glsl,
                code: SPIDER_ROT_SCALE_VERTEX_SHADER_VULKAN.to_vec(),
                entry_point: "main".to_string(),
            })?;
            created.push(spider_rot_shader);
            let spider_clip_shader = gal.create_shader_module(ShaderModuleDesc {
                label: format!("{label}.spider-clip"),
                stage: ShaderStage::Fragment,
                code_format: ShaderCodeFormat::Glsl,
                code: SPIDER_CLIP_FRAGMENT_SHADER_VULKAN.to_vec(),
                entry_point: "main".to_string(),
            })?;
            created.push(spider_clip_shader);
            let spider_blit_shader = gal.create_shader_module(ShaderModuleDesc {
                label: format!("{label}.spider-blit"),
                stage: ShaderStage::Fragment,
                code_format: ShaderCodeFormat::Glsl,
                code: SPIDER_BLIT_FRAGMENT_SHADER_VULKAN.to_vec(),
                entry_point: "main".to_string(),
            })?;
            created.push(spider_blit_shader);
            let resource_layout = gal.create_resource_layout(ResourceLayoutDesc {
                label: format!("{label}.resource-layout"),
                bindings: vec![
                    ResourceBindingDesc {
                        binding: 0,
                        kind: ResourceBindingKind::SampledTexture,
                        stages: PipelineStageFlags::DRAW,
                        array_count: 1,
                        optional: false,
                        dynamic_offset_count: 0,
                    },
                    ResourceBindingDesc {
                        binding: 1,
                        kind: ResourceBindingKind::Sampler,
                        stages: PipelineStageFlags::DRAW,
                        array_count: 1,
                        optional: false,
                        dynamic_offset_count: 0,
                    },
                    ResourceBindingDesc {
                        binding: 2,
                        kind: ResourceBindingKind::UniformBuffer,
                        stages: PipelineStageFlags::DRAW,
                        array_count: 1,
                        optional: false,
                        dynamic_offset_count: 0,
                    },
                ],
            })?;
            created.push(resource_layout);
            let resource_set = gal.create_resource_set(ResourceSetDesc {
                label: format!("{label}.resource-set"),
                layout: resource_layout,
                bindings: vec![
                    ResourceBinding {
                        binding: 0,
                        array_index: 0,
                        resource: texture_view,
                        kind: ResourceBindingKind::SampledTexture,
                        access: AccessFlags::READ,
                        dynamic_offsets: Vec::new(),
                        buffer_range: None,
                    },
                    ResourceBinding {
                        binding: 1,
                        array_index: 0,
                        resource: sampler,
                        kind: ResourceBindingKind::Sampler,
                        access: AccessFlags::READ,
                        dynamic_offsets: Vec::new(),
                        buffer_range: None,
                    },
                    ResourceBinding {
                        binding: 2,
                        array_index: 0,
                        resource: uniform_buffer,
                        kind: ResourceBindingKind::UniformBuffer,
                        access: AccessFlags::READ,
                        dynamic_offsets: Vec::new(),
                        buffer_range: Some(64),
                    },
                ],
            })?;
            created.push(resource_set);
            let pipeline_layout = gal.create_pipeline_layout(PipelineLayoutDesc {
                label: format!("{label}.pipeline-layout"),
                resource_layouts: vec![resource_layout],
            })?;
            created.push(pipeline_layout);
            let pipeline = gal.create_graphics_pipeline(GraphicsPipelineDesc {
                label: format!("{label}.pipeline"),
                layout: pipeline_layout,
                vertex_shader,
                fragment_shader,
                topology: PrimitiveTopology::Triangles,
                cull_mode: CullMode::None,
                front_face: crate::render::vulkanic::resources::FrontFace::CounterClockwise,
                provoking_vertex: crate::render::vulkanic::resources::ProvokingVertex::Last,
                raster_y_direction: crate::render::vulkanic::resources::RasterYDirection::Up,
                blend: BlendMode::Disabled,
                depth_compare: None,
                depth_write: false,
                depth_bias: None,
                color_formats: vec![color_format],
                depth_format: None,
                stencil: None,
            })?;
            created.push(pipeline);
            let creeper_resource_set = gal.create_resource_set(ResourceSetDesc {
                label: format!("{label}.creeper-resource-set"),
                layout: resource_layout,
                bindings: vec![
                    ResourceBinding {
                        binding: 0,
                        array_index: 0,
                        resource: creeper_view,
                        kind: ResourceBindingKind::SampledTexture,
                        access: AccessFlags::READ,
                        dynamic_offsets: Vec::new(),
                        buffer_range: None,
                    },
                    ResourceBinding {
                        binding: 1,
                        array_index: 0,
                        resource: sampler,
                        kind: ResourceBindingKind::Sampler,
                        access: AccessFlags::READ,
                        dynamic_offsets: Vec::new(),
                        buffer_range: None,
                    },
                    ResourceBinding {
                        binding: 2,
                        array_index: 0,
                        resource: uniform_buffer,
                        kind: ResourceBindingKind::UniformBuffer,
                        access: AccessFlags::READ,
                        dynamic_offsets: Vec::new(),
                        buffer_range: Some(64),
                    },
                ],
            })?;
            created.push(creeper_resource_set);
            let single_views = [
                texture_view,
                spider_views[2],
                spider_views[1],
                spider_views[3],
            ];
            let mut spider_single_sets = Vec::with_capacity(4);
            for (index, view) in single_views.into_iter().enumerate() {
                let set = gal.create_resource_set(ResourceSetDesc {
                    label: format!("{label}.spider-single-set-{index}"),
                    layout: resource_layout,
                    bindings: vec![
                        ResourceBinding {
                            binding: 0,
                            array_index: 0,
                            resource: view,
                            kind: ResourceBindingKind::SampledTexture,
                            access: AccessFlags::READ,
                            dynamic_offsets: Vec::new(),
                            buffer_range: None,
                        },
                        ResourceBinding {
                            binding: 1,
                            array_index: 0,
                            resource: sampler,
                            kind: ResourceBindingKind::Sampler,
                            access: AccessFlags::READ,
                            dynamic_offsets: Vec::new(),
                            buffer_range: None,
                        },
                        ResourceBinding {
                            binding: 2,
                            array_index: 0,
                            resource: uniform_buffer,
                            kind: ResourceBindingKind::UniformBuffer,
                            access: AccessFlags::READ,
                            dynamic_offsets: Vec::new(),
                            buffer_range: Some(64),
                        },
                    ],
                })?;
                created.push(set);
                spider_single_sets.push(set);
            }
            let spider_dual_layout = gal.create_resource_layout(ResourceLayoutDesc {
                label: format!("{label}.spider-dual-layout"),
                bindings: vec![
                    ResourceBindingDesc {
                        binding: 0,
                        kind: ResourceBindingKind::SampledTexture,
                        stages: PipelineStageFlags::DRAW,
                        array_count: 1,
                        optional: false,
                        dynamic_offset_count: 0,
                    },
                    ResourceBindingDesc {
                        binding: 1,
                        kind: ResourceBindingKind::SampledTexture,
                        stages: PipelineStageFlags::DRAW,
                        array_count: 1,
                        optional: false,
                        dynamic_offset_count: 0,
                    },
                    ResourceBindingDesc {
                        binding: 2,
                        kind: ResourceBindingKind::Sampler,
                        stages: PipelineStageFlags::DRAW,
                        array_count: 1,
                        optional: false,
                        dynamic_offset_count: 0,
                    },
                    ResourceBindingDesc {
                        binding: 3,
                        kind: ResourceBindingKind::UniformBuffer,
                        stages: PipelineStageFlags::DRAW,
                        array_count: 1,
                        optional: false,
                        dynamic_offset_count: 0,
                    },
                ],
            })?;
            created.push(spider_dual_layout);
            let dual_inputs = [
                (texture_view, spider_views[0]),
                (spider_views[1], spider_views[2]),
                (spider_views[1], spider_views[3]),
            ];
            let mut spider_dual_sets = Vec::with_capacity(3);
            for (index, (input, blur)) in dual_inputs.into_iter().enumerate() {
                let set = gal.create_resource_set(ResourceSetDesc {
                    label: format!("{label}.spider-dual-set-{index}"),
                    layout: spider_dual_layout,
                    bindings: vec![
                        ResourceBinding {
                            binding: 0,
                            array_index: 0,
                            resource: input,
                            kind: ResourceBindingKind::SampledTexture,
                            access: AccessFlags::READ,
                            dynamic_offsets: Vec::new(),
                            buffer_range: None,
                        },
                        ResourceBinding {
                            binding: 1,
                            array_index: 0,
                            resource: blur,
                            kind: ResourceBindingKind::SampledTexture,
                            access: AccessFlags::READ,
                            dynamic_offsets: Vec::new(),
                            buffer_range: None,
                        },
                        ResourceBinding {
                            binding: 2,
                            array_index: 0,
                            resource: sampler,
                            kind: ResourceBindingKind::Sampler,
                            access: AccessFlags::READ,
                            dynamic_offsets: Vec::new(),
                            buffer_range: None,
                        },
                        ResourceBinding {
                            binding: 3,
                            array_index: 0,
                            resource: uniform_buffer,
                            kind: ResourceBindingKind::UniformBuffer,
                            access: AccessFlags::READ,
                            dynamic_offsets: Vec::new(),
                            buffer_range: Some(64),
                        },
                    ],
                })?;
                created.push(set);
                spider_dual_sets.push(set);
            }
            let invert_pipeline = gal.create_graphics_pipeline(GraphicsPipelineDesc {
                label: format!("{label}.invert-pipeline"),
                layout: pipeline_layout,
                vertex_shader,
                fragment_shader: invert_fragment_shader,
                topology: PrimitiveTopology::Triangles,
                cull_mode: CullMode::None,
                front_face: crate::render::vulkanic::resources::FrontFace::CounterClockwise,
                provoking_vertex: crate::render::vulkanic::resources::ProvokingVertex::Last,
                raster_y_direction: crate::render::vulkanic::resources::RasterYDirection::Up,
                blend: BlendMode::Disabled,
                depth_compare: None,
                depth_write: false,
                depth_bias: None,
                color_formats: vec![color_format],
                depth_format: None,
                stencil: None,
            })?;
            created.push(invert_pipeline);
            let creeper_color_pipeline = gal.create_graphics_pipeline(GraphicsPipelineDesc {
                label: format!("{label}.creeper-color-pipeline"),
                layout: pipeline_layout,
                vertex_shader,
                fragment_shader: creeper_color_shader,
                topology: PrimitiveTopology::Triangles,
                cull_mode: CullMode::None,
                front_face: crate::render::vulkanic::resources::FrontFace::CounterClockwise,
                provoking_vertex: crate::render::vulkanic::resources::ProvokingVertex::Last,
                raster_y_direction: crate::render::vulkanic::resources::RasterYDirection::Up,
                blend: BlendMode::Disabled,
                depth_compare: None,
                depth_write: false,
                depth_bias: None,
                color_formats: vec![color_format],
                depth_format: None,
                stencil: None,
            })?;
            created.push(creeper_color_pipeline);
            let creeper_bits_pipeline = gal.create_graphics_pipeline(GraphicsPipelineDesc {
                label: format!("{label}.creeper-bits-pipeline"),
                layout: pipeline_layout,
                vertex_shader,
                fragment_shader: creeper_bits_shader,
                topology: PrimitiveTopology::Triangles,
                cull_mode: CullMode::None,
                front_face: crate::render::vulkanic::resources::FrontFace::CounterClockwise,
                provoking_vertex: crate::render::vulkanic::resources::ProvokingVertex::Last,
                raster_y_direction: crate::render::vulkanic::resources::RasterYDirection::Up,
                blend: BlendMode::Disabled,
                depth_compare: None,
                depth_write: false,
                depth_bias: None,
                color_formats: vec![color_format],
                depth_format: None,
                stencil: None,
            })?;
            created.push(creeper_bits_pipeline);
            let spider_dual_pipeline_layout = gal.create_pipeline_layout(PipelineLayoutDesc {
                label: format!("{label}.spider-dual-pipeline-layout"),
                resource_layouts: vec![spider_dual_layout],
            })?;
            created.push(spider_dual_pipeline_layout);
            let spider_box_pipeline = gal.create_graphics_pipeline(GraphicsPipelineDesc {
                label: format!("{label}.spider-box-pipeline"),
                layout: pipeline_layout,
                vertex_shader,
                fragment_shader: spider_box_shader,
                topology: PrimitiveTopology::Triangles,
                cull_mode: CullMode::None,
                front_face: crate::render::vulkanic::resources::FrontFace::CounterClockwise,
                provoking_vertex: crate::render::vulkanic::resources::ProvokingVertex::Last,
                raster_y_direction: crate::render::vulkanic::resources::RasterYDirection::Up,
                blend: BlendMode::Disabled,
                depth_compare: None,
                depth_write: false,
                depth_bias: None,
                color_formats: vec![color_format],
                depth_format: None,
                stencil: None,
            })?;
            created.push(spider_box_pipeline);
            let spider_clip_pipeline = gal.create_graphics_pipeline(GraphicsPipelineDesc {
                label: format!("{label}.spider-clip-pipeline"),
                layout: spider_dual_pipeline_layout,
                vertex_shader: spider_rot_shader,
                fragment_shader: spider_clip_shader,
                topology: PrimitiveTopology::Triangles,
                cull_mode: CullMode::None,
                front_face: crate::render::vulkanic::resources::FrontFace::CounterClockwise,
                provoking_vertex: crate::render::vulkanic::resources::ProvokingVertex::Last,
                raster_y_direction: crate::render::vulkanic::resources::RasterYDirection::Up,
                blend: BlendMode::Disabled,
                depth_compare: None,
                depth_write: false,
                depth_bias: None,
                color_formats: vec![color_format],
                depth_format: None,
                stencil: None,
            })?;
            created.push(spider_clip_pipeline);
            let spider_blit_pipeline = gal.create_graphics_pipeline(GraphicsPipelineDesc {
                label: format!("{label}.spider-blit-pipeline"),
                layout: pipeline_layout,
                vertex_shader,
                fragment_shader: spider_blit_shader,
                topology: PrimitiveTopology::Triangles,
                cull_mode: CullMode::None,
                front_face: crate::render::vulkanic::resources::FrontFace::CounterClockwise,
                provoking_vertex: crate::render::vulkanic::resources::ProvokingVertex::Last,
                raster_y_direction: crate::render::vulkanic::resources::RasterYDirection::Up,
                blend: BlendMode::Disabled,
                depth_compare: None,
                depth_write: false,
                depth_bias: None,
                color_formats: vec![color_format],
                depth_format: None,
                stencil: None,
            })?;
            created.push(spider_blit_pipeline);
            Ok(GuiBlurResources {
                width,
                height,
                color_format,
                texture,
                creeper_texture,
                creeper_view,
                creeper_target,
                creeper_pass,
                uniform_buffer,
                texture_view,
                sampler,
                vertex_shader,
                fragment_shader,
                invert_fragment_shader,
                creeper_color_shader,
                creeper_bits_shader,
                resource_layout,
                resource_set,
                creeper_resource_set,
                pipeline_layout,
                pipeline,
                invert_pipeline,
                creeper_color_pipeline,
                creeper_bits_pipeline,
                spider_textures: spider_textures.try_into().expect("four spider textures"),
                spider_views: spider_views.try_into().expect("four spider views"),
                spider_targets: spider_targets.try_into().expect("four spider targets"),
                spider_passes: spider_passes.try_into().expect("four spider passes"),
                spider_single_sets: spider_single_sets.try_into().expect("four spider sets"),
                spider_dual_sets: spider_dual_sets.try_into().expect("three spider dual sets"),
                spider_dual_layout,
                spider_dual_pipeline_layout,
                spider_box_shader,
                spider_rot_shader,
                spider_clip_shader,
                spider_blit_shader,
                spider_box_pipeline,
                spider_clip_pipeline,
                spider_blit_pipeline,
            })
        })();
        match result {
            Ok(resources) => {
                self.blur_resources = Some(resources);
                Ok(self.blur_resources.as_ref().unwrap())
            }
            Err(error) => {
                for handle in created.into_iter().rev() {
                    let _ = gal.destroy(handle);
                }
                Err(error)
            }
        }
    }

    fn upload_resources(
        &mut self,
        gal: &mut VulkanicGal,
        source: &GuiTextureSource,
        group: TextureGroup,
        resources: &GuiResources,
        image_upload: Option<Handle>,
    ) -> GalResult<()> {
        // Each binding owns a new index buffer even if its image is shared.
        // Initialize it independently; an existing image stays ShaderRead.
        let mut operations = gui_index_upload_ops(resources.index_buffer);
        if let Some(upload_buffer) = image_upload {
            operations.extend([
                CommandOp::HostWriteBuffer {
                    buffer: upload_buffer, offset: 0, data: source.bytes.clone(),
                },
                CommandOp::Barrier(buffer_barrier(upload_buffer,
                    TextureUsageState::TransferDst, TextureUsageState::TransferSrc)),
                CommandOp::Barrier(texture_barrier(resources.texture,
                    TextureUsageState::Undefined, TextureUsageState::TransferDst)),
                CommandOp::CopyBufferToTexture(BufferImageCopyRegion {
                    buffer: upload_buffer, buffer_offset: 0,
                    bytes_per_row: source.width * source.format.bytes_per_pixel() as u32,
                    rows_per_image: source.height, texture: resources.texture,
                    texture_mip: 0, texture_layer: 0,
                    texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
                    extent: Extent3d { width: source.width, height: source.height, depth: 1 },
                }),
                CommandOp::Barrier(texture_barrier(resources.texture,
                    TextureUsageState::TransferDst, TextureUsageState::ShaderRead)),
            ]);
        }
        gal.submit(SubmissionBatch {
            label: format!("gui-textured-{}.upload", group.label()),
            command_lists: vec![CommandList::from(CommandListDesc {
                label: format!("gui-textured-{}.upload.commands", group.label()),
                operations,
            })],
        })?;
        Ok(())
    }

    fn atlas_for(&mut self, group: TextureGroup) -> GalResult<&TextureAtlas> {
        if matches!(
            group,
            TextureGroup::Dynamic(_)
                | TextureGroup::DynamicLinear(_)
                | TextureGroup::DynamicGlint(_)
                | TextureGroup::DynamicOpaque(_)
                | TextureGroup::DynamicVignette(_)
                | TextureGroup::DynamicInvert(_)
                | TextureGroup::DynamicPremultiplied(_)
                | TextureGroup::DynamicAdditive(_)
        ) {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "raw GUI images do not belong to the static sprite atlas",
            ));
        }
        let key = TextureGroupKey::from(group);
        if !self.atlases.contains_key(&key) {
            let atlas = build_atlas(group, &self.asset_overrides)?;
            self.atlases.insert(key, atlas);
        }
        Ok(self.atlases.get(&key).expect("atlas was just inserted"))
    }

    fn texture_source(&mut self, group: TextureGroup) -> GalResult<GuiTextureSource> {
        match group {
            TextureGroup::Alpha | TextureGroup::Invert => {
                let atlas = self.atlas_for(group)?.clone();
                Ok(GuiTextureSource {
                    width: atlas.width,
                    height: atlas.height,
                    format: GuiRawImageFormat::Rgba8,
                    bytes: atlas.bytes,
                })
            }
            TextureGroup::Dynamic(asset_id)
            | TextureGroup::DynamicLinear(asset_id)
            | TextureGroup::DynamicGlint(asset_id)
            | TextureGroup::DynamicOpaque(asset_id)
            | TextureGroup::DynamicVignette(asset_id)
            | TextureGroup::DynamicInvert(asset_id)
            | TextureGroup::DynamicPremultiplied(asset_id)
            | TextureGroup::DynamicAdditive(asset_id)
            | TextureGroup::DynamicLequalDepth(asset_id)
            | TextureGroup::DynamicItemRaster(asset_id)
            | TextureGroup::DynamicItemCutout(asset_id) => {
                if self.atlas_references.contains(asset_id) {
                    return Err(GalError::ffi(StatusCode::UnsupportedFeature,
                        "explicit GUI atlas sampling is not yet admitted; copied-image fallback is forbidden"));
                }
                let image = self.raw_images.get(&asset_id).ok_or_else(|| {
                    GalError::ffi(
                        StatusCode::InvalidArgument,
                        format!("unknown raw GUI image asset {asset_id}"),
                    )
                })?;
                Ok(GuiTextureSource {
                    width: image.width,
                    height: image.height,
                    format: image.format,
                    bytes: image.pixels.clone(),
                })
            }
        }
    }

    fn ensure_resources(
        &mut self,
        gal: &mut VulkanicGal,
        group: TextureGroup,
        color_format: ColorFormat,
        depth_format: Option<TextureFormat>,
        stats: &mut GuiSubmitStats,
    ) -> GalResult<()> {
        let key = ResourceKey::new(group, color_format, depth_format);
        if self.resources.contains_key(&key) {
            stats.cache_hits += 1;
            return Ok(());
        }
        let shared_pipeline =
            self.ensure_shared_pipeline(gal, group, color_format, depth_format)?;
        let resources = self.create_resources(gal, group, shared_pipeline)?;
        self.resources.insert(key, resources);
        stats.cache_misses += 1;
        stats.resource_creates += 7;
        Ok(())
    }

    fn pack_sprite(
        &mut self,
        request: &GuiSpriteRequest,
        def: &SpriteDef,
        pre_present_y_flip: bool,
    ) -> GalResult<PackedGuiQuad> {
        let atlas = self.atlas_for(def.group)?;
        let region = atlas.regions.get(&def.id).ok_or_else(|| {
            GalError::backend(format!("sprite '{}' missing from atlas", def.name))
        })?;
        let source_width = request.width.min(def.width);
        let source_height = request.height.min(def.height);
        Ok(PackedGuiQuad {
            origin: [request.x as f32, request.y as f32],
            axis_u: [request.width as f32, 0.0],
            axis_v: [0.0, request.height as f32],
            viewport: request.projection_extent,
            clip: [0.0; 4],
            clip_enabled: false,
            pre_present_y_flip,
            // GUI atlases and raw GUI images use the same semantic convention:
            // byte row zero and V=0 are the top edge. Backends upload the bytes
            // unchanged, and the shader samples the explicit UV/sampler so no
            // API-specific texture-origin conversion leaks into this frontend.
            uv: [
                region.x as f32 / atlas.width as f32,
                region.y as f32 / atlas.height as f32,
                source_width as f32 / atlas.width as f32,
                source_height as f32 / atlas.height as f32,
            ],
            color: argb_to_rgba(request.color_argb),
            texture_mode: GuiRawImageFormat::Rgba8.shader_mode(),
            z: 0.0,
        })
    }
}

fn gui_index_upload_ops(index_buffer: Handle) -> Vec<CommandOp> {
    vec![
        CommandOp::HostWriteBuffer { buffer: index_buffer, offset: 0, data: index_bytes() },
        CommandOp::Barrier(buffer_barrier(index_buffer,
            TextureUsageState::TransferDst, TextureUsageState::IndexRead)),
    ]
}

fn append_gui_quad(
    batches: &mut Vec<GuiBatch>,
    stratum: u32,
    group: TextureGroup,
    quad: PackedGuiQuad,
) {
    if let Some(batch) = batches.last_mut().filter(|batch| {
        batch.stratum == stratum
            && batch.group == group
            && batch.quads.len() < GUI_MAX_PACKED_SPRITES
    }) {
        batch.quads.push(quad);
    } else {
        batches.push(GuiBatch {
            stratum,
            group,
            quads: vec![quad],
        });
    }
}

fn append_gui_batches_ops(
    frontend: &GuiFrontend,
    frame_pass: Handle,
    render_target: Handle,
    color_attachment: Handle,
    depth_attachment: Option<Handle>,
    color_format: ColorFormat,
    depth_format: Option<TextureFormat>,
    batches: &[GuiBatch],
    ops: &mut Vec<CommandOp>,
) -> GalResult<()> {
    for batch in batches {
        let resources = frontend
            .resources
            .get(&ResourceKey::new(batch.group, color_format, depth_format))
            .ok_or_else(|| GalError::backend("GUI resources vanished before ordered submit"))?;
        let uniforms = packed_uniform_bytes(batch)?;
        ops.push(CommandOp::Barrier(buffer_barrier(
            resources.uniform_buffer,
            TextureUsageState::ShaderRead,
            TextureUsageState::TransferDst,
        )));
        ops.push(CommandOp::HostWriteBuffer {
            buffer: resources.uniform_buffer,
            offset: 0,
            data: uniforms,
        });
        ops.push(CommandOp::Barrier(buffer_barrier(
            resources.uniform_buffer,
            TextureUsageState::TransferDst,
            TextureUsageState::ShaderRead,
        )));
        ops.push(CommandOp::BeginPass {
            pass: frame_pass,
            target: render_target,
            colors: vec![loaded_frame_color_attachment(color_attachment)],
            depth_stencil: depth_attachment.map(loaded_frame_depth_attachment),
        });
        ops.push(CommandOp::BindGraphicsPipeline(resources.pipeline));
        ops.push(CommandOp::BindResourceSet {
            pipeline_layout: resources.pipeline_layout,
            set_index: 0,
            set: resources.resource_set,
            dynamic_offsets: Vec::new(),
        });
        ops.push(CommandOp::SetIndexBuffer {
            buffer: resources.index_buffer,
            offset: 0,
            index_type: super::resources::IndexType::U32,
        });
        ops.push(CommandOp::DrawIndexed {
            indices: 6,
            instances: batch.quads.len() as u32,
        });
        ops.push(CommandOp::EndPass);
    }
    Ok(())
}

fn mesh_atlas_contract_supported(batch: &GuiMeshBatchRequest) -> bool {
    if batch.material_mode == GuiMeshMaterialMode::ModelOverlay {
        return batch.item_raster_scale != 0
            && batch.lighting_mode == GuiMeshLightingMode::FrontModel
            && batch.alpha_cutoff == 0.0 && batch.item_foil.is_none();
    }
    (batch.item_raster_scale != 0 || batch.lighting_mode == GuiMeshLightingMode::InventoryBlock)
        && matches!(batch.material_mode,
            GuiMeshMaterialMode::Opaque | GuiMeshMaterialMode::Cutout | GuiMeshMaterialMode::Translucent)
}

/// Semantic model overlays finish before the item's depth-equal foil pass.
/// Frozen's fixed entity-glint buffer is flushed after shield pattern buffers;
/// express that dependency here without asking Java to sort GPU passes.
/// Other item families keep their existing authored order.
fn mesh_item_layer_execution_order(layers: &[GuiMeshPreparedDraw]) -> Vec<usize> {
    let mut order: Vec<_> = (0..layers.len()).collect();
    if layers.iter().any(|layer| layer.material_mode == GuiMeshMaterialMode::ModelOverlay) {
        order.sort_by_key(|index| (layers[*index].material_mode == GuiMeshMaterialMode::Glint, *index));
    }
    order
}

fn validate_mesh_item_layers(layers: &[GuiMeshPreparedDraw]) -> GalResult<()> {
    let first = layers.first().ok_or_else(|| {
        GalError::ffi(
            StatusCode::InvalidArgument,
            "GUI mesh item must contain one or more layers",
        )
    })?;
    for (expected_layer, layer) in layers.iter().enumerate() {
        if layer.layer_index != expected_layer as u32
            || layer.item_cache != first.item_cache
            || layer.bounds != first.bounds
            || layer.gui_pose != first.gui_pose
            || layer.gui_extent != first.gui_extent
            || layer.render_extent != first.render_extent
            || layer.guard_pixels != first.guard_pixels
        {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "GUI mesh item layers must share target and composition semantics",
            ));
        }
    }
    Ok(())
}

fn order_gui_requests(
    sprites: Vec<GuiSpriteRequest>,
    affine_quads: Vec<GuiAffineQuadRequest>,
) -> GalResult<Vec<GuiFrameRequest>> {
    order_gui_requests_with_mesh(sprites, affine_quads, Vec::new())
}

fn order_gui_requests_with_mesh(
    sprites: Vec<GuiSpriteRequest>,
    affine_quads: Vec<GuiAffineQuadRequest>,
    mesh_batches: Vec<GuiMeshBatchRequest>,
) -> GalResult<Vec<GuiFrameRequest>> {
    order_gui_requests_with_tiles(sprites, affine_quads, mesh_batches, Vec::new())
}

fn order_gui_requests_with_tiles(
    sprites: Vec<GuiSpriteRequest>, affine_quads: Vec<GuiAffineQuadRequest>,
    mesh_batches: Vec<GuiMeshBatchRequest>, tiled_quads: Vec<GuiTiledQuadRequest>,
) -> GalResult<Vec<GuiFrameRequest>> {
    preflight_tiled_affine_count(&tiled_quads, affine_quads.len())?;
    if mesh_batches.len() > GUI_MAX_MESH_BATCHES {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!(
                "GUI mesh batch count {} exceeds bounded limit {GUI_MAX_MESH_BATCHES}",
                mesh_batches.len()
            ),
        ));
    }
    let mesh_items = group_gui_mesh_items(mesh_batches)?;
    let tiled_items = tiled_quads.into_iter().map(|request| {
        lower_tiled_request(request).map(GuiFrameRequest::AffineBatch)
    }).collect::<GalResult<Vec<_>>>()?;
    let mut ordered = sprites
        .into_iter()
        .map(GuiFrameRequest::Sprite)
        .chain(affine_quads.into_iter().map(GuiFrameRequest::Affine))
        .chain(mesh_items.into_iter().map(GuiFrameRequest::Mesh))
        .chain(tiled_items)
        .collect::<Vec<_>>();
    ordered.sort_by_key(|request| (request.stratum(), request.sequence()));
    validate_ordered_gui_requests(&ordered)?;
    Ok(coalesce_ordered_affine_requests(ordered))
}

fn coalesce_ordered_affine_requests(ordered: Vec<GuiFrameRequest>) -> Vec<GuiFrameRequest> {
    let mut result = Vec::with_capacity(ordered.len());
    for request in ordered {
        match request {
            GuiFrameRequest::Affine(request) => {
                let can_append = result.last().is_some_and(|previous| {
                    if let GuiFrameRequest::AffineBatch(batch) = previous {
                        batch.last().is_some_and(|last| {
                            last.stratum == request.stratum && last.asset_id == request.asset_id
                                && last.item_raster_scale == request.item_raster_scale
                        })
                    } else {
                        false
                    }
                });
                if can_append {
                    if let Some(GuiFrameRequest::AffineBatch(batch)) = result.last_mut() {
                        batch.push(request);
                    }
                } else {
                    result.push(GuiFrameRequest::AffineBatch(vec![request]));
                }
            }
            other => result.push(other),
        }
    }
    result
}

fn group_gui_mesh_items(mut batches: Vec<GuiMeshBatchRequest>) -> GalResult<Vec<GuiMeshItem>> {
    super::gui_mesh_frontend::validate_batches(&batches)?;
    batches.sort_by_key(|batch| (batch.stratum, batch.sequence, batch.layer_index));
    let mut items = Vec::new();
    let mut cursor = 0;
    while cursor < batches.len() {
        let first = &batches[cursor];
        let key = (first.stratum, first.sequence);
        let end = batches[cursor..]
            .iter()
            .position(|batch| (batch.stratum, batch.sequence) != key)
            .map(|offset| cursor + offset)
            .unwrap_or(batches.len());
        items.push(GuiMeshItem {
            stratum: key.0,
            sequence: key.1,
            layers: batches[cursor..end].to_vec(),
        });
        cursor = end;
    }
    Ok(items)
}

fn validate_ordered_gui_requests(requests: &[GuiFrameRequest]) -> GalResult<()> {
    let mut previous = None;
    for request in requests {
        if request.sequence() == 0 {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "GUI request sequence must be non-zero",
            ));
        }
        let key = (request.stratum(), request.sequence());
        if previous.is_some_and(|previous| previous >= key) {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "GUI requests must have unique scheduler sequences in stratum order",
            ));
        }
        previous = Some(key);
    }
    Ok(())
}

fn packed_uniform_bytes(batch: &GuiBatch) -> GalResult<Vec<u8>> {
    if batch.quads.is_empty() || batch.quads.len() > GUI_MAX_PACKED_SPRITES {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!(
                "packed GUI quad count must be in 1..{}: {}",
                GUI_MAX_PACKED_SPRITES,
                batch.quads.len()
            ),
        ));
    }
    let mut out = Vec::with_capacity(batch.quads.len() * GUI_UNIFORM_BYTES);
    for quad in &batch.quads {
        for value in quad.origin.into_iter().chain(quad.axis_u) {
            push_f32(&mut out, value);
        }
        for value in quad.axis_v.into_iter().chain([quad.texture_mode, quad.z]) {
            push_f32(&mut out, value);
        }
        for value in quad.viewport.into_iter().chain([
            if quad.clip_enabled { 1.0 } else { 0.0 },
            if quad.pre_present_y_flip { 1.0 } else { 0.0 },
        ]) {
            push_f32(&mut out, value);
        }
        for value in quad.clip {
            push_f32(&mut out, value);
        }
        for value in quad.uv {
            push_f32(&mut out, value);
        }
        for value in quad.color {
            push_f32(&mut out, value);
        }
    }
    Ok(out)
}

fn argb_to_rgba(color_argb: u32) -> [f32; 4] {
    [
        ((color_argb >> 16) & 0xff) as f32 / 255.0,
        ((color_argb >> 8) & 0xff) as f32 / 255.0,
        (color_argb & 0xff) as f32 / 255.0,
        ((color_argb >> 24) & 0xff) as f32 / 255.0,
    ]
}

pub(crate) fn validate_gui_projection(layout: [u32; 2], projection: [f32; 2]) -> GalResult<()> {
    if projection.iter().zip(layout).any(|(&value, bound)|
        !value.is_finite() || value <= 0.0 || value.ceil() != bound as f32) {
        return Err(GalError::ffi(StatusCode::InvalidArgument,
            "GUI projection must be finite, positive, and ceil to the explicit layout extent"));
    }
    Ok(())
}

pub(crate) fn validate_affine_quad(request: &GuiAffineQuadRequest) -> GalResult<()> {
    if request.item_raster_layers.len() > super::gui_item_raster::MAX_ITEM_LAYERS
        || (!request.item_raster_layers.is_empty() && request.item_raster_scale == 0) {
        return Err(GalError::invalid_argument("invalid bounded item layer request"));
    }
    if request.item_raster_scale != 0 {
        request.item_raster_geometry.validate()?;
        super::gui_item_raster::item_uv_identity([request.u0,request.v0,request.u1,request.v1])?;
    }
    if request.item_raster_scale > 256 || (request.item_raster_scale != 0
        && (matches!(request.material, super::gui_item_material::GuiAffineMaterial::Unlit)
            || request.z != 0.0)) {
        return Err(GalError::invalid_argument("invalid full-item raster semantics"));
    }
    validate_gui_projection([request.gui_width, request.gui_height], request.projection_extent)?;
    const GUI_UV_OVERLAP_LIMIT: f32 = 1.0 / 16.0;
    if request.asset_id == 0 || request.gui_width == 0 || request.gui_height == 0 {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "GUI affine quad requires a non-zero asset and viewport",
        ));
    }
    let values = [
        request.x0, request.y0, request.x1, request.y1, request.x3, request.y3, request.z,
        request.u0, request.v0, request.u1, request.v1,
    ];
    if values.into_iter().any(|value| !value.is_finite()) {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "GUI affine quad coordinates must be finite",
        ));
    }
    if request.u0 < -GUI_UV_OVERLAP_LIMIT
        || request.v0 < -GUI_UV_OVERLAP_LIMIT
        || request.u1 > 1.0 + GUI_UV_OVERLAP_LIMIT
        || request.v1 > 1.0 + GUI_UV_OVERLAP_LIMIT
        || request.u1 < request.u0
        || request.v1 < request.v0
    {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "GUI affine quad UV range must stay inside the semantic image",
        ));
    }
    match request.clip_mode {
        0 if request.clip_left == 0
            && request.clip_top == 0
            && request.clip_width == 0
            && request.clip_height == 0 => {}
        1 if request.clip_left >= 0
            && request.clip_top >= 0
            && request.clip_width >= 0
            && request.clip_height >= 0
            && request.clip_left.saturating_add(request.clip_width) <= request.gui_width as i32
            && request.clip_top.saturating_add(request.clip_height)
                <= request.gui_height as i32 => {}
        _ => {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "GUI affine quad clip must be disabled or a bounded frame-local rectangle",
            ));
        }
    }
    Ok(())
}

fn push_f32(out: &mut Vec<u8>, value: f32) {
    out.extend_from_slice(&value.to_ne_bytes());
}

fn buffer_barrier(
    resource: Handle,
    before: TextureUsageState,
    after: TextureUsageState,
) -> ResourceBarrier {
    ResourceBarrier {
        resource,
        subresources: None,
        before,
        after,
        src_queue: QueueClass::Graphics,
        dst_queue: QueueClass::Graphics,
    }
}

fn custom_sampler_info_bytes(extent: Extent3d, images: &[Option<CustomPostEffectImage>]) -> Vec<u8> {
    let mut sizes = vec![extent.width as f32, extent.height as f32];
    for image in images {
        sizes.extend(image.as_ref().map(|image| [image.width as f32, image.height as f32])
            .unwrap_or([extent.width as f32, extent.height as f32]));
    }
    let mut data = sizes.into_iter().flat_map(f32::to_le_bytes).collect::<Vec<_>>();
    data.resize((data.len() + 15) / 16 * 16, 0);
    data
}

fn push_uniform_write(operations: &mut Vec<CommandOp>, buffer: Handle, data: Vec<u8>) {
    operations.push(CommandOp::HostWriteBuffer {
        buffer,
        offset: 0,
        data,
    });
    operations.push(CommandOp::Barrier(buffer_barrier(
        buffer,
        TextureUsageState::TransferDst,
        TextureUsageState::ShaderRead,
    )));
}

fn texture_barrier(
    resource: Handle,
    before: TextureUsageState,
    after: TextureUsageState,
) -> ResourceBarrier {
    ResourceBarrier {
        resource,
        subresources: Some(TextureSubresourceRange {
            base_mip: 0,
            mip_count: 1,
            base_layer: 0,
            layer_count: 1,
        }),
        before,
        after,
        src_queue: QueueClass::Graphics,
        dst_queue: QueueClass::Graphics,
    }
}

fn validate_request(request: &GuiSpriteRequest, def: &SpriteDef) -> GalResult<()> {
    validate_gui_projection([request.gui_width, request.gui_height], request.projection_extent)?;
    if !request.progress_fraction.is_finite() {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "GUI sprite progress fraction must be finite",
        ));
    }
    if request.width == 0
        || request.height == 0
        || request.gui_width == 0
        || request.gui_height == 0
    {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "GUI sprite dimensions and viewport must be non-zero",
        ));
    }
    if def.id != GUI_POST_EFFECT_INVERT_ID
        && (request.width > def.width || request.height > def.height)
    {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!(
                "GUI sprite '{}' requested dimensions {}x{} exceed semantic sprite {}x{}",
                def.name, request.width, request.height, def.width, def.height
            ),
        ));
    }
    if request.fill_direction > 3 {
        return Err(GalError::ffi(
            StatusCode::UnknownEnum,
            format!("unknown GUI fill direction {}", request.fill_direction),
        ));
    }
    Ok(())
}

fn build_atlas(
    group: TextureGroup,
    asset_overrides: &BTreeMap<u32, Vec<u8>>,
) -> GalResult<TextureAtlas> {
    let sprites: Vec<&SpriteDef> = SPRITES
        .iter()
        .filter(|sprite| sprite.group == group)
        .collect();
    let mut width = 1u32;
    let mut height = 0u32;
    let mut row_width = 0u32;
    let mut row_height = 0u32;
    for sprite in &sprites {
        if row_width > 0 && row_width + sprite.width > 4096 {
            width = width.max(row_width);
            height += row_height;
            row_width = 0;
            row_height = 0;
        }
        row_width += sprite.width;
        row_height = row_height.max(sprite.height);
    }
    width = width.max(row_width);
    height += row_height;
    let mut bytes = vec![0u8; (width * height * 4) as usize];
    let mut regions = BTreeMap::new();
    let mut x_offset = 0u32;
    let mut y_offset = 0u32;
    row_height = 0;
    for sprite in sprites {
        if x_offset > 0 && x_offset + sprite.width > 4096 {
            x_offset = 0;
            y_offset += row_height;
            row_height = 0;
        }
        let sprite_bytes = load_sprite(sprite, asset_overrides)?;
        for y in 0..sprite.height {
            let src = (y * sprite.width * 4) as usize;
            let dst = ((y_offset + y) * width * 4 + x_offset * 4) as usize;
            bytes[dst..dst + (sprite.width * 4) as usize]
                .copy_from_slice(&sprite_bytes[src..src + (sprite.width * 4) as usize]);
        }
        regions.insert(
            sprite.id,
            AtlasRegion {
                x: x_offset,
                y: y_offset,
            },
        );
        x_offset += sprite.width;
        row_height = row_height.max(sprite.height);
    }
    Ok(TextureAtlas {
        width,
        height,
        bytes,
        regions,
    })
}

fn load_sprite(sprite: &SpriteDef, asset_overrides: &BTreeMap<u32, Vec<u8>>) -> GalResult<Vec<u8>> {
    if sprite.id == GUI_POST_EFFECT_INVERT_ID
        || sprite.id == GUI_POST_EFFECT_CREEPER_ID
        || sprite.id == GUI_POST_EFFECT_SPIDER_ID
    {
        // Vanilla's bundled invert post chain uses InverseAmount = 0.8.
        // BlendMode::Invert computes `src * (1-dst) + dst * (1-src)`, so a
        // Rust-owned 0.8 source reproduces that semantic mix without sampling
        // the destination through a backend-specific attachment.
        return Ok(vec![204u8, 204u8, 204u8, 255u8]);
    }
    let bytes = asset_overrides
        .get(&sprite.id)
        .map(Vec::as_slice)
        .or_else(|| bundled_sprite_bytes(sprite.path))
        .ok_or_else(|| {
            GalError::backend(format!("missing bundled GUI sprite '{}'", sprite.path))
        })?;
    decode_sprite_bytes(sprite, bytes)
}

fn decode_sprite_bytes(sprite: &SpriteDef, bytes: &[u8]) -> GalResult<Vec<u8>> {
    let mut decoder = png::Decoder::new(BufReader::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|error| {
        GalError::backend(format!(
            "failed to decode GUI sprite '{}': {error}",
            sprite.path
        ))
    })?;
    let header = reader.info();
    let header_pixels = (header.width as u64)
        .checked_mul(header.height as u64)
        .ok_or_else(|| {
            GalError::backend(format!("GUI sprite '{}' dimensions overflow", sprite.name))
        })?;
    if header.width != sprite.width || header.height != sprite.height {
        return Err(GalError::backend(format!(
            "unexpected GUI sprite dimensions for '{}': {}x{}, expected {}x{}",
            sprite.name, header.width, header.height, sprite.width, sprite.height
        )));
    }
    if header_pixels == 0 || header_pixels > GUI_MAX_RAW_IMAGE_PIXELS as u64 {
        return Err(GalError::backend(format!(
            "GUI sprite '{}' decoded pixel count {header_pixels} exceeds {GUI_MAX_RAW_IMAGE_PIXELS}",
            sprite.name
        )));
    }
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).map_err(|error| {
        GalError::backend(format!(
            "failed to read GUI sprite '{}': {error}",
            sprite.path
        ))
    })?;
    let data = &buf[..info.buffer_size()];
    match info.color_type {
        png::ColorType::Rgba => Ok(data.to_vec()),
        png::ColorType::Rgb => {
            let mut rgba = Vec::with_capacity((info.width * info.height * 4) as usize);
            for pixel in data.chunks_exact(3) {
                rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
            }
            Ok(rgba)
        }
        png::ColorType::GrayscaleAlpha => {
            let mut rgba = Vec::with_capacity((info.width * info.height * 4) as usize);
            for pixel in data.chunks_exact(2) {
                rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]);
            }
            Ok(rgba)
        }
        png::ColorType::Grayscale => {
            let mut rgba = Vec::with_capacity((info.width * info.height * 4) as usize);
            for value in data {
                rgba.extend_from_slice(&[*value, *value, *value, 255]);
            }
            Ok(rgba)
        }
        png::ColorType::Indexed => Err(GalError::backend(format!(
            "indexed GUI sprite '{}' was not expanded by the PNG decoder",
            sprite.name
        ))),
    }
}

fn bundled_sprite_bytes(path: &str) -> Option<&'static [u8]> {
    match path {
        "/assets/minecraft/textures/gui/sprites/boss_bar/blue_background.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/boss_bar/blue_background.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/boss_bar/blue_progress.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/boss_bar/blue_progress.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/boss_bar/green_background.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/boss_bar/green_background.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/boss_bar/green_progress.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/boss_bar/green_progress.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/boss_bar/notched_10_background.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/boss_bar/notched_10_background.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/boss_bar/notched_10_progress.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/boss_bar/notched_10_progress.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/boss_bar/notched_12_background.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/boss_bar/notched_12_background.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/boss_bar/notched_12_progress.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/boss_bar/notched_12_progress.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/boss_bar/notched_20_background.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/boss_bar/notched_20_background.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/boss_bar/notched_20_progress.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/boss_bar/notched_20_progress.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/boss_bar/notched_6_background.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/boss_bar/notched_6_background.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/boss_bar/notched_6_progress.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/boss_bar/notched_6_progress.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/boss_bar/pink_background.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/boss_bar/pink_background.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/boss_bar/pink_progress.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/boss_bar/pink_progress.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/boss_bar/purple_background.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/boss_bar/purple_background.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/boss_bar/purple_progress.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/boss_bar/purple_progress.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/boss_bar/red_background.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/boss_bar/red_background.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/boss_bar/red_progress.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/boss_bar/red_progress.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/boss_bar/white_background.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/boss_bar/white_background.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/boss_bar/white_progress.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/boss_bar/white_progress.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/boss_bar/yellow_background.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/boss_bar/yellow_background.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/boss_bar/yellow_progress.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/boss_bar/yellow_progress.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/armor_empty.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/armor_empty.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/armor_full.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/armor_full.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/armor_half.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/armor_half.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/air.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/air.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/air_bursting.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/air_bursting.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/air_empty.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/air_empty.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/crosshair.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/crosshair.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/crosshair_attack_indicator_background.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/crosshair_attack_indicator_background.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/crosshair_attack_indicator_full.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/crosshair_attack_indicator_full.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/crosshair_attack_indicator_progress.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/crosshair_attack_indicator_progress.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/experience_bar_background.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/experience_bar_background.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/experience_bar_progress.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/experience_bar_progress.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/absorbing_full.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/absorbing_full.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/absorbing_full_blinking.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/absorbing_full_blinking.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/absorbing_half.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/absorbing_half.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/absorbing_half_blinking.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/absorbing_half_blinking.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/absorbing_hardcore_full.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/absorbing_hardcore_full.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/absorbing_hardcore_full_blinking.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/absorbing_hardcore_full_blinking.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/absorbing_hardcore_half.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/absorbing_hardcore_half.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/absorbing_hardcore_half_blinking.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/absorbing_hardcore_half_blinking.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/container.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/container.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/container_blinking.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/container_blinking.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/container_hardcore.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/container_hardcore.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/container_hardcore_blinking.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/container_hardcore_blinking.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/frozen_full.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/frozen_full.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/frozen_full_blinking.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/frozen_full_blinking.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/frozen_half.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/frozen_half.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/frozen_half_blinking.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/frozen_half_blinking.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/frozen_hardcore_full.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/frozen_hardcore_full.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/frozen_hardcore_full_blinking.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/frozen_hardcore_full_blinking.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/frozen_hardcore_half.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/frozen_hardcore_half.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/frozen_hardcore_half_blinking.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/frozen_hardcore_half_blinking.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/full.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/full.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/full_blinking.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/full_blinking.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/half.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/half.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/half_blinking.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/half_blinking.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/hardcore_full.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/hardcore_full.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/hardcore_full_blinking.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/hardcore_full_blinking.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/hardcore_half.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/hardcore_half.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/hardcore_half_blinking.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/hardcore_half_blinking.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/poisoned_full.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/poisoned_full.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/poisoned_full_blinking.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/poisoned_full_blinking.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/poisoned_half.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/poisoned_half.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/poisoned_half_blinking.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/poisoned_half_blinking.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/poisoned_hardcore_full.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/poisoned_hardcore_full.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/poisoned_hardcore_full_blinking.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/poisoned_hardcore_full_blinking.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/poisoned_hardcore_half.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/poisoned_hardcore_half.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/poisoned_hardcore_half_blinking.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/poisoned_hardcore_half_blinking.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/withered_full.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/withered_full.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/withered_full_blinking.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/withered_full_blinking.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/withered_half.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/withered_half.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/withered_half_blinking.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/withered_half_blinking.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/withered_hardcore_full.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/withered_hardcore_full.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/withered_hardcore_full_blinking.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/withered_hardcore_full_blinking.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/withered_hardcore_half.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/withered_hardcore_half.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/withered_hardcore_half_blinking.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/withered_hardcore_half_blinking.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/vehicle_container.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/vehicle_container.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/vehicle_full.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/vehicle_full.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/heart/vehicle_half.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/heart/vehicle_half.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/hotbar.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/hotbar.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/hotbar_attack_indicator_background.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/hotbar_attack_indicator_background.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/hotbar_attack_indicator_progress.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/hotbar_attack_indicator_progress.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/hotbar_selection.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/hotbar_selection.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/food_empty.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/food_empty.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/food_half.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/food_half.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/food_full.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/food_full.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/food_empty_hunger.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/food_empty_hunger.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/food_half_hunger.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/food_half_hunger.png").as_slice()),
        "/assets/minecraft/textures/gui/sprites/hud/food_full_hunger.png" => Some(include_bytes!("../../../resources/assets/minecraft/textures/gui/sprites/hud/food_full_hunger.png").as_slice()),
        _ => None,
    }
}

fn index_bytes() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(24);
    for value in [0u32, 1, 2, 3, 4, 5] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn sprite_def(sprite_id: u32) -> GalResult<&'static SpriteDef> {
    SPRITES
        .iter()
        .find(|sprite| sprite.id == sprite_id)
        .ok_or_else(|| {
            GalError::ffi(
                StatusCode::UnknownEnum,
                format!("unknown GUI sprite id {sprite_id}"),
            )
        })
}

const fn group(invert: bool) -> TextureGroup {
    if invert {
        TextureGroup::Invert
    } else {
        TextureGroup::Alpha
    }
}

const SPRITES: &[SpriteDef] = &[
    SpriteDef {
        id: 1,
        stratum: 200,
        name: "crosshair",
        path: "/assets/minecraft/textures/gui/sprites/hud/crosshair.png",
        width: 15,
        height: 15,
        group: group(true),
    },
    SpriteDef {
        id: 2,
        stratum: 300,
        name: "hotbar-base",
        path: "/assets/minecraft/textures/gui/sprites/hud/hotbar.png",
        width: 182,
        height: 22,
        group: group(false),
    },
    SpriteDef {
        id: 3,
        stratum: 310,
        name: "hotbar-selection",
        path: "/assets/minecraft/textures/gui/sprites/hud/hotbar_selection.png",
        width: 24,
        height: 23,
        group: group(false),
    },
    SpriteDef {
        id: 4,
        stratum: 350,
        name: "armor-empty",
        path: "/assets/minecraft/textures/gui/sprites/hud/armor_empty.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 5,
        stratum: 350,
        name: "armor-half",
        path: "/assets/minecraft/textures/gui/sprites/hud/armor_half.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 6,
        stratum: 350,
        name: "armor-full",
        path: "/assets/minecraft/textures/gui/sprites/hud/armor_full.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 7,
        stratum: 360,
        name: "player-heart-container",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/container.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 8,
        stratum: 360,
        name: "player-heart-container",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/container_blinking.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 9,
        stratum: 360,
        name: "player-heart-container",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/container_hardcore.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 10,
        stratum: 360,
        name: "player-heart-container",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/container_hardcore_blinking.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 11,
        stratum: 360,
        name: "player-heart-normal",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/full.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 12,
        stratum: 360,
        name: "player-heart-normal",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/full_blinking.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 13,
        stratum: 360,
        name: "player-heart-normal",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/half.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 14,
        stratum: 360,
        name: "player-heart-normal",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/half_blinking.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 15,
        stratum: 360,
        name: "player-heart-normal",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/hardcore_full.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 16,
        stratum: 360,
        name: "player-heart-normal",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/hardcore_full_blinking.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 17,
        stratum: 360,
        name: "player-heart-normal",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/hardcore_half.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 18,
        stratum: 360,
        name: "player-heart-normal",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/hardcore_half_blinking.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 19,
        stratum: 360,
        name: "player-heart-poisoned",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/poisoned_full.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 20,
        stratum: 360,
        name: "player-heart-poisoned",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/poisoned_full_blinking.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 21,
        stratum: 360,
        name: "player-heart-poisoned",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/poisoned_half.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 22,
        stratum: 360,
        name: "player-heart-poisoned",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/poisoned_half_blinking.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 23,
        stratum: 360,
        name: "player-heart-poisoned",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/poisoned_hardcore_full.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 24,
        stratum: 360,
        name: "player-heart-poisoned",
        path:
            "/assets/minecraft/textures/gui/sprites/hud/heart/poisoned_hardcore_full_blinking.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 25,
        stratum: 360,
        name: "player-heart-poisoned",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/poisoned_hardcore_half.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 26,
        stratum: 360,
        name: "player-heart-poisoned",
        path:
            "/assets/minecraft/textures/gui/sprites/hud/heart/poisoned_hardcore_half_blinking.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 27,
        stratum: 360,
        name: "player-heart-withered",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/withered_full.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 28,
        stratum: 360,
        name: "player-heart-withered",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/withered_full_blinking.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 29,
        stratum: 360,
        name: "player-heart-withered",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/withered_half.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 30,
        stratum: 360,
        name: "player-heart-withered",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/withered_half_blinking.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 31,
        stratum: 360,
        name: "player-heart-withered",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/withered_hardcore_full.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 32,
        stratum: 360,
        name: "player-heart-withered",
        path:
            "/assets/minecraft/textures/gui/sprites/hud/heart/withered_hardcore_full_blinking.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 33,
        stratum: 360,
        name: "player-heart-withered",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/withered_hardcore_half.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 34,
        stratum: 360,
        name: "player-heart-withered",
        path:
            "/assets/minecraft/textures/gui/sprites/hud/heart/withered_hardcore_half_blinking.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 35,
        stratum: 360,
        name: "player-heart-frozen",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/frozen_full.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 36,
        stratum: 360,
        name: "player-heart-frozen",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/frozen_full_blinking.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 37,
        stratum: 360,
        name: "player-heart-frozen",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/frozen_half.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 38,
        stratum: 360,
        name: "player-heart-frozen",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/frozen_half_blinking.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 39,
        stratum: 360,
        name: "player-heart-frozen",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/frozen_hardcore_full.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 40,
        stratum: 360,
        name: "player-heart-frozen",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/frozen_hardcore_full_blinking.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 41,
        stratum: 360,
        name: "player-heart-frozen",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/frozen_hardcore_half.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 42,
        stratum: 360,
        name: "player-heart-frozen",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/frozen_hardcore_half_blinking.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 43,
        stratum: 360,
        name: "absorption-heart",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/absorbing_full.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 44,
        stratum: 360,
        name: "absorption-heart",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/absorbing_full_blinking.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 45,
        stratum: 360,
        name: "absorption-heart",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/absorbing_half.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 46,
        stratum: 360,
        name: "absorption-heart",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/absorbing_half_blinking.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 47,
        stratum: 360,
        name: "absorption-heart",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/absorbing_hardcore_full.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 48,
        stratum: 360,
        name: "absorption-heart",
        path:
            "/assets/minecraft/textures/gui/sprites/hud/heart/absorbing_hardcore_full_blinking.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 49,
        stratum: 360,
        name: "absorption-heart",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/absorbing_hardcore_half.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 50,
        stratum: 360,
        name: "absorption-heart",
        path:
            "/assets/minecraft/textures/gui/sprites/hud/heart/absorbing_hardcore_half_blinking.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 51,
        stratum: 400,
        name: "experience-background",
        path: "/assets/minecraft/textures/gui/sprites/hud/experience_bar_background.png",
        width: 182,
        height: 5,
        group: group(false),
    },
    SpriteDef {
        id: 52,
        stratum: 410,
        name: "experience-progress",
        path: "/assets/minecraft/textures/gui/sprites/hud/experience_bar_progress.png",
        width: 182,
        height: 5,
        group: group(false),
    },
    SpriteDef {
        id: 53,
        stratum: 510,
        name: "attack-crosshair-full",
        path: "/assets/minecraft/textures/gui/sprites/hud/crosshair_attack_indicator_full.png",
        width: 16,
        height: 16,
        group: group(false),
    },
    SpriteDef {
        id: 54,
        stratum: 500,
        name: "attack-crosshair-background",
        path:
            "/assets/minecraft/textures/gui/sprites/hud/crosshair_attack_indicator_background.png",
        width: 16,
        height: 4,
        group: group(false),
    },
    SpriteDef {
        id: 55,
        stratum: 510,
        name: "attack-crosshair-progress",
        path: "/assets/minecraft/textures/gui/sprites/hud/crosshair_attack_indicator_progress.png",
        width: 16,
        height: 4,
        group: group(false),
    },
    SpriteDef {
        id: 56,
        stratum: 520,
        name: "attack-hotbar-background",
        path: "/assets/minecraft/textures/gui/sprites/hud/hotbar_attack_indicator_background.png",
        width: 18,
        height: 18,
        group: group(false),
    },
    SpriteDef {
        id: 57,
        stratum: 530,
        name: "attack-hotbar-progress",
        path: "/assets/minecraft/textures/gui/sprites/hud/hotbar_attack_indicator_progress.png",
        width: 18,
        height: 18,
        group: group(false),
    },
    SpriteDef {
        id: 58,
        stratum: 600,
        name: "boss-bar-background",
        path: "/assets/minecraft/textures/gui/sprites/boss_bar/pink_background.png",
        width: 182,
        height: 5,
        group: group(false),
    },
    SpriteDef {
        id: 59,
        stratum: 600,
        name: "boss-bar-background",
        path: "/assets/minecraft/textures/gui/sprites/boss_bar/blue_background.png",
        width: 182,
        height: 5,
        group: group(false),
    },
    SpriteDef {
        id: 60,
        stratum: 600,
        name: "boss-bar-background",
        path: "/assets/minecraft/textures/gui/sprites/boss_bar/red_background.png",
        width: 182,
        height: 5,
        group: group(false),
    },
    SpriteDef {
        id: 61,
        stratum: 600,
        name: "boss-bar-background",
        path: "/assets/minecraft/textures/gui/sprites/boss_bar/green_background.png",
        width: 182,
        height: 5,
        group: group(false),
    },
    SpriteDef {
        id: 62,
        stratum: 600,
        name: "boss-bar-background",
        path: "/assets/minecraft/textures/gui/sprites/boss_bar/yellow_background.png",
        width: 182,
        height: 5,
        group: group(false),
    },
    SpriteDef {
        id: 63,
        stratum: 600,
        name: "boss-bar-background",
        path: "/assets/minecraft/textures/gui/sprites/boss_bar/purple_background.png",
        width: 182,
        height: 5,
        group: group(false),
    },
    SpriteDef {
        id: 64,
        stratum: 600,
        name: "boss-bar-background",
        path: "/assets/minecraft/textures/gui/sprites/boss_bar/white_background.png",
        width: 182,
        height: 5,
        group: group(false),
    },
    SpriteDef {
        id: 65,
        stratum: 610,
        name: "boss-bar-progress",
        path: "/assets/minecraft/textures/gui/sprites/boss_bar/pink_progress.png",
        width: 182,
        height: 5,
        group: group(false),
    },
    SpriteDef {
        id: 66,
        stratum: 610,
        name: "boss-bar-progress",
        path: "/assets/minecraft/textures/gui/sprites/boss_bar/blue_progress.png",
        width: 182,
        height: 5,
        group: group(false),
    },
    SpriteDef {
        id: 67,
        stratum: 610,
        name: "boss-bar-progress",
        path: "/assets/minecraft/textures/gui/sprites/boss_bar/red_progress.png",
        width: 182,
        height: 5,
        group: group(false),
    },
    SpriteDef {
        id: 68,
        stratum: 610,
        name: "boss-bar-progress",
        path: "/assets/minecraft/textures/gui/sprites/boss_bar/green_progress.png",
        width: 182,
        height: 5,
        group: group(false),
    },
    SpriteDef {
        id: 69,
        stratum: 610,
        name: "boss-bar-progress",
        path: "/assets/minecraft/textures/gui/sprites/boss_bar/yellow_progress.png",
        width: 182,
        height: 5,
        group: group(false),
    },
    SpriteDef {
        id: 70,
        stratum: 610,
        name: "boss-bar-progress",
        path: "/assets/minecraft/textures/gui/sprites/boss_bar/purple_progress.png",
        width: 182,
        height: 5,
        group: group(false),
    },
    SpriteDef {
        id: 71,
        stratum: 610,
        name: "boss-bar-progress",
        path: "/assets/minecraft/textures/gui/sprites/boss_bar/white_progress.png",
        width: 182,
        height: 5,
        group: group(false),
    },
    SpriteDef {
        id: 72,
        stratum: 600,
        name: "boss-bar-overlay-background",
        path: "/assets/minecraft/textures/gui/sprites/boss_bar/notched_6_background.png",
        width: 182,
        height: 5,
        group: group(false),
    },
    SpriteDef {
        id: 73,
        stratum: 600,
        name: "boss-bar-overlay-background",
        path: "/assets/minecraft/textures/gui/sprites/boss_bar/notched_10_background.png",
        width: 182,
        height: 5,
        group: group(false),
    },
    SpriteDef {
        id: 74,
        stratum: 600,
        name: "boss-bar-overlay-background",
        path: "/assets/minecraft/textures/gui/sprites/boss_bar/notched_12_background.png",
        width: 182,
        height: 5,
        group: group(false),
    },
    SpriteDef {
        id: 75,
        stratum: 600,
        name: "boss-bar-overlay-background",
        path: "/assets/minecraft/textures/gui/sprites/boss_bar/notched_20_background.png",
        width: 182,
        height: 5,
        group: group(false),
    },
    SpriteDef {
        id: 76,
        stratum: 610,
        name: "boss-bar-overlay-progress",
        path: "/assets/minecraft/textures/gui/sprites/boss_bar/notched_6_progress.png",
        width: 182,
        height: 5,
        group: group(false),
    },
    SpriteDef {
        id: 77,
        stratum: 610,
        name: "boss-bar-overlay-progress",
        path: "/assets/minecraft/textures/gui/sprites/boss_bar/notched_10_progress.png",
        width: 182,
        height: 5,
        group: group(false),
    },
    SpriteDef {
        id: 78,
        stratum: 610,
        name: "boss-bar-overlay-progress",
        path: "/assets/minecraft/textures/gui/sprites/boss_bar/notched_12_progress.png",
        width: 182,
        height: 5,
        group: group(false),
    },
    SpriteDef {
        id: 79,
        stratum: 610,
        name: "boss-bar-overlay-progress",
        path: "/assets/minecraft/textures/gui/sprites/boss_bar/notched_20_progress.png",
        width: 182,
        height: 5,
        group: group(false),
    },
    SpriteDef {
        id: 80,
        stratum: 370,
        name: "hunger-empty",
        path: "/assets/minecraft/textures/gui/sprites/hud/food_empty.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 81,
        stratum: 370,
        name: "hunger-half",
        path: "/assets/minecraft/textures/gui/sprites/hud/food_half.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 82,
        stratum: 370,
        name: "hunger-full",
        path: "/assets/minecraft/textures/gui/sprites/hud/food_full.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 83,
        stratum: 370,
        name: "hunger-effect-empty",
        path: "/assets/minecraft/textures/gui/sprites/hud/food_empty_hunger.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 84,
        stratum: 370,
        name: "hunger-effect-half",
        path: "/assets/minecraft/textures/gui/sprites/hud/food_half_hunger.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 85,
        stratum: 370,
        name: "hunger-effect-full",
        path: "/assets/minecraft/textures/gui/sprites/hud/food_full_hunger.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 86,
        stratum: 380,
        name: "air-full",
        path: "/assets/minecraft/textures/gui/sprites/hud/air.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 87,
        stratum: 380,
        name: "air-popping",
        path: "/assets/minecraft/textures/gui/sprites/hud/air_bursting.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 88,
        stratum: 380,
        name: "air-empty",
        path: "/assets/minecraft/textures/gui/sprites/hud/air_empty.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 89,
        stratum: 390,
        name: "mount-heart-container",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/vehicle_container.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 90,
        stratum: 390,
        name: "mount-heart-full",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/vehicle_full.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: 91,
        stratum: 390,
        name: "mount-heart-half",
        path: "/assets/minecraft/textures/gui/sprites/hud/heart/vehicle_half.png",
        width: 9,
        height: 9,
        group: group(false),
    },
    SpriteDef {
        id: GUI_POST_EFFECT_INVERT_ID,
        stratum: 80,
        name: "post-effect-invert",
        path: "/assets/minecraft/textures/misc/rust_generated_white.png",
        width: 1,
        height: 1,
        group: group(true),
    },
    SpriteDef {
        id: GUI_POST_EFFECT_CREEPER_ID,
        stratum: 80,
        name: "post-effect-creeper",
        path: "/assets/minecraft/textures/misc/rust_generated_white.png",
        width: 1,
        height: 1,
        group: group(false),
    },
    SpriteDef {
        id: GUI_POST_EFFECT_SPIDER_ID,
        stratum: 80,
        name: "post-effect-spider",
        path: "/assets/minecraft/textures/misc/rust_generated_white.png",
        width: 1,
        height: 1,
        group: group(false),
    },
];

fn loaded_frame_color_attachment(frame_target: Handle) -> PassAttachment {
    PassAttachment {
        view: frame_target,
        load_op: AttachmentLoadOp::Load,
        store_op: AttachmentStoreOp::Store,
        clear_color: None,
    }
}

fn loaded_frame_depth_attachment(depth_attachment: Handle) -> PassAttachment {
    PassAttachment {
        view: depth_attachment,
        load_op: AttachmentLoadOp::Load,
        store_op: AttachmentStoreOp::Store,
        clear_color: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::vulkanic::backends::{
        mock::MockBackend, vulkan::VulkanBackend, vulkan_capabilities,
    };
    use crate::render::vulkanic::commands::ClearColor;
    use crate::render::vulkanic::frame::{FrameSurfaceDesc, PresentMode};
    use crate::render::vulkanic::gui_mesh_frontend::{
        GuiMeshLightingMode, GuiMeshMaterialMode, GuiMeshVertex,
    };
    use crate::render::vulkanic::handles::HandleKind;
    use crate::render::vulkanic::resources::{
        Extent3d, FrameTargetDesc, RenderTargetDesc, TextureFormat,
    };

    #[test]
    fn blur_boundary_plan_partitions_node_phase_orders_without_backend_state() {
        let plan = plan_gui_blur_boundary(2, [0, 1, 2, 5, 6, 100]).unwrap();
        assert_eq!(2, plan.boundary_stratum);
        assert_eq!(4, plan.before_count);
        assert_eq!(2, plan.after_count);
    }

    #[test]
    fn blur_boundary_plan_rejects_negative_and_overflowing_boundaries() {
        assert!(plan_gui_blur_boundary(-1, []).is_err());
        assert!(plan_gui_blur_boundary(i32::MAX, []).is_err());
    }

    #[test]
    fn gui_draw_receipt_rejects_semantics_without_rust_draw() {
        let missing = GuiFrontend::require_gui_draw_receipt(
            &GuiSubmitStats {
                sprite_count: 1,
                ..GuiSubmitStats::default()
            },
            &[],
        )
        .unwrap_err()
        .to_string();
        assert!(
            missing.contains("GUI source writer recorded 1 semantic items but no draw operations")
        );

        GuiFrontend::require_gui_draw_receipt(
            &GuiSubmitStats {
                affine_quad_count: 1,
                ..GuiSubmitStats::default()
            },
            &[CommandOp::Draw {
                vertices: 3,
                instances: 1,
            }],
        )
        .expect("a Rust GUI draw should satisfy the semantic receipt");
    }

    /// Exercises the same dynamic raw-image, uniform, indexed-draw, and
    /// `BlendMode::Vignette` path used by the whole-frame HUD.  A pipeline
    /// descriptor assertion cannot catch a later frontend regression that
    /// loses the loaded target or samples the raw mask as black.
    #[test]
    fn vulkan_dynamic_vignette_affine_quad_darkens_the_loaded_target() {
        let Some(bytes) = vulkan_gui_sample(
            GUI_VIGNETTE_BLIT_STRATUM, vec![191, 191, 191, 255], 0.0, 1.0, None,
        ) else { return };
        // 0.8 * (1 - 191 / 255) is approximately 51/255.
        for channel in &bytes[..3] {
            assert!((*channel as i16 - 51).abs() <= 1, "unexpected GUI vignette: {bytes:?}");
        }
    }

    #[test]
    fn vulkan_gui_sampler_preserves_fractional_uv_intervals() {
        // At the destination pixel center, U=.275 selects texel 1 in a
        // four-texel nearest-filtered image. Rounding the region origin and
        // extent before interpolating incorrectly selects texel 0.
        let Some(bytes) = vulkan_gui_sample(
            GUI_OPAQUE_BLIT_STRATUM,
            vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255],
            0.1, 0.45, None,
        ) else { return };
        assert_eq!(&[0, 255, 0, 255], bytes.as_slice());
    }

    #[test]
    fn vulkan_gui_tiles_sample_repeated_atlas_region_and_partial_last_tile() {
        use crate::render::vulkanic::gui_tiling::GuiTileGeometry;
        // Five logical units contain two full 2-unit tiles and one half tile.
        // First sample the second full tile, then translate to sample the last
        // partial tile. Both centers are .25 of a full tile into the same atlas
        // interval [.1,.9], so U=.3 selects green, not the unstaged neighbor.
        for translation in [0.0, -0.4] {
            let Some(bytes) = vulkan_gui_sample(
                GUI_OPAQUE_BLIT_STRATUM,
                vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255],
                0.1, 0.9,
                Some(GuiTileGeometry { bounds: [0, 0, 5, 1], tile_extent: [2, 1],
                    uv: [0.1, 0.0, 0.9, 1.0], pose: [0.2, 0.0, 0.0, 1.0, translation, 0.0] }),
            ) else { return };
            assert_eq!(&[0, 255, 0, 255], bytes.as_slice(), "translation={translation}");
        }
    }

    fn gui_tiled_request() -> GuiTiledQuadRequest {
        GuiTiledQuadRequest {
            geometry: super::super::gui_tiling::GuiTileGeometry {
                bounds: [7, 11, 71, 43], tile_extent: [32, 32],
                uv: [0.25, 0.5, 0.75, 1.0], pose: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            },
            stratum: GUI_OPAQUE_BLIT_STRATUM, asset_id: 41, z: 0.25, color_argb: 0xffaabbcc,
            gui_extent: [320, 180], projection_extent: [319.75, 179.5], sequence: 11,
            clip: Some([1, 2, 100, 50]),
        }
    }

    #[test]
    fn gui_tiled_command_preserves_parent_identity_material_clip_and_projection() {
        let request = gui_tiled_request();
        let quads = lower_tiled_request(request.clone()).unwrap();
        assert_eq!(2, quads.len());
        for quad in quads {
            assert_eq!(request.sequence, quad.sequence);
            assert_eq!(request.stratum, quad.stratum);
            assert_eq!(request.asset_id, quad.asset_id);
            assert_eq!(request.z, quad.z);
            assert_eq!(request.color_argb, quad.color_argb);
            assert_eq!(request.projection_extent, quad.projection_extent);
            assert_eq!(1, quad.clip_mode);
            assert_eq!([1, 2, 100, 50], [quad.clip_left, quad.clip_top, quad.clip_width, quad.clip_height]);
        }
        let mut invalid = request;
        invalid.clip = Some([319, 0, 2, 1]);
        assert!(lower_tiled_request(invalid).is_err());
    }

    #[test]
    fn gui_tiled_children_do_not_consume_or_collide_with_other_parent_sequences() {
        let request = gui_tiled_request();
        let mut ordinary = lower_tiled_request(request.clone()).unwrap().remove(0);
        ordinary.sequence = 12;
        let ordered = order_gui_requests_with_tiles(Vec::new(), vec![ordinary.clone()], Vec::new(), vec![request.clone()]).unwrap();
        let GuiFrameRequest::AffineBatch(batch) = &ordered[0] else { panic!("expected compatible batch") };
        assert_eq!(vec![11, 11, 12], batch.iter().map(|q| q.sequence).collect::<Vec<_>>());
        ordinary.sequence = 11;
        assert!(order_gui_requests_with_tiles(Vec::new(), vec![ordinary], Vec::new(), vec![request.clone()]).is_err());
        let mut maximum = request;
        maximum.sequence = u64::MAX;
        assert!(order_gui_requests_with_tiles(Vec::new(), Vec::new(), Vec::new(), vec![maximum]).is_ok());
    }

    #[test]
    fn gui_tiled_frame_preflight_bounds_the_sum_before_expansion() {
        let mut request = gui_tiled_request();
        request.geometry.bounds = [0, 0, 4096, 4096];
        request.geometry.uv = [0.0, 0.0, 1.0, 1.0];
        let four = vec![request.clone(); 4];
        assert_eq!(GUI_MAX_EXPANDED_AFFINE_QUADS, preflight_tiled_affine_count(&four, 0).unwrap());
        assert!(preflight_tiled_affine_count(&four, 1).is_err());
        assert!(preflight_tiled_affine_count(&vec![request; 5], 0).is_err());
        assert!(preflight_tiled_affine_count(&[], usize::MAX).is_err());
    }

    fn vulkan_gui_sample(stratum: u32, pixels: Vec<u8>, u0: f32, u1: f32,
        tiles: Option<crate::render::vulkanic::gui_tiling::GuiTileGeometry>) -> Option<Vec<u8>> {
        vulkan_gui_sample_with_effect(stratum, pixels, u0, u1, tiles, 1, false)
    }

    #[test]
    fn vulkan_invert_post_effect_preserves_asymmetric_framebuffer_rows() {
        let Some(bytes) = vulkan_gui_sample_with_effect(GUI_OPAQUE_BLIT_STRATUM,
            vec![20, 60, 100, 255, 180, 220, 240, 255], 0.0, 1.0, None, 2, true)
            else { return };
        let expected = [192u8, 168, 144, 255, 96, 72, 60, 255];
        assert_eq!(8, bytes.len());
        for (actual, expected) in bytes.iter().zip(expected) {
            assert!((*actual as i16 - expected as i16).abs() <= 1,
                "inversion must preserve source row order and vanilla 0.8 mix: {bytes:?}");
        }
    }

    fn vulkan_gui_sample_with_effect(stratum: u32, pixels: Vec<u8>, u0: f32, u1: f32,
        tiles: Option<crate::render::vulkanic::gui_tiling::GuiTileGeometry>, height: u32,
        invert: bool) -> Option<Vec<u8>> {
        vulkan_gui_sample_with_reused_image(stratum, pixels, u0, u1, tiles, height, invert, false, None, false)
    }

    #[test]
    fn vulkan_gui_reused_image_preserves_opaque_pixels_after_alpha_preparation() {
        let bytes = vulkan_gui_sample_with_reused_image(GUI_OPAQUE_BLIT_STRATUM,
            vec![17, 83, 211, 255], 0.0, 1.0, None, 1, false, true, None, false)
            .expect("Vulkan is required for reused GUI image regression");
        assert_eq!(bytes, vec![17, 83, 211, 255]);
    }

    #[test]
    fn vulkan_gui_private_atlas_binding_samples_the_declared_region_without_copied_pixels() {
        let bytes = vulkan_gui_sample_with_reused_image(1,
            vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255],
            0.0, 1.0, None, 2, false, false, Some([1, 0, 1, 2]), false)
            .expect("Vulkan required for owner-backed atlas sampling regression");
        assert_eq!(bytes, vec![0, 255, 0, 255, 255, 255, 0, 255],
            "sample only the right atlas column and preserve semantic row orientation");
    }

    #[test]
    fn vulkan_mixed_gui_scheduler_preserves_raw_atlas_raw_blend_order() {
        let bytes = vulkan_gui_sample_with_reused_image(1,
            vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255],
            0.0, 1.0, None, 2, false, false, Some([1, 0, 1, 2]), true)
            .expect("Vulkan required for mixed GUI ownership regression");
        assert_eq!(bytes, vec![0, 127, 128, 255, 127, 127, 128, 255]);
    }

    #[test]
    fn vulkan_mixed_gui_scheduler_tiles_local_coordinates_inside_the_atlas_region() {
        use super::super::gui_tiling::GuiTileGeometry;
        let bytes = vulkan_gui_sample_with_reused_image(1,
            vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255],
            0.0, 1.0, Some(GuiTileGeometry { bounds: [0, 0, 5, 1], tile_extent: [2, 1],
                uv: [0.0, 0.0, 1.0, 1.0], pose: [0.2, 0.0, 0.0, 1.0, 0.0, 0.0] }),
            1, false, false, Some([1, 0, 2, 1]), false)
            .expect("Vulkan required for tiled GUI atlas regression");
        assert_eq!(bytes, vec![0, 255, 0, 255]);
    }

    fn vulkan_gui_sample_with_reused_image(stratum: u32, pixels: Vec<u8>, u0: f32, u1: f32,
        tiles: Option<crate::render::vulkanic::gui_tiling::GuiTileGeometry>, height: u32,
        invert: bool, prewarm_alpha: bool, atlas_region: Option<[u32; 4]>, mixed: bool) -> Option<Vec<u8>> {
        vulkan_gui_sample_with_blur(stratum, pixels, u0, u1, tiles, height, invert,
            prewarm_alpha, atlas_region, mixed, None)
    }

    #[test]
    fn vulkan_owned_atlas_draws_on_both_sides_of_blur_boundary() {
        for stratum in [0, 100] {
            let pixels = vec![255, 0, 0, 255, 0, 255, 0, 255];
            let actual = vulkan_gui_sample_with_blur(stratum, pixels, 0.0, 1.0,
                None, 1, false, false, Some([1, 0, 1, 1]), false, Some((2, 2)))
                .expect("Vulkan required for atlas blur boundary regression");
            assert_eq!(actual, vec![0, 255, 0, 255], "stratum {stratum}");
        }
    }

    fn vulkan_gui_sample_with_blur(stratum: u32, pixels: Vec<u8>, u0: f32, u1: f32,
        tiles: Option<crate::render::vulkanic::gui_tiling::GuiTileGeometry>, height: u32,
        invert: bool, prewarm_alpha: bool, atlas_region: Option<[u32; 4]>, mixed: bool,
        blur: Option<(i32, i32)>) -> Option<Vec<u8>> {
        vulkan_gui_sample_with_material(stratum, pixels, u0, u1, tiles, height, invert,
            prewarm_alpha, atlas_region, mixed, blur,
            crate::render::vulkanic::gui_item_material::GuiAffineMaterial::Unlit)
    }

    #[test]
    fn vulkan_gui_flat_item_material_modulates_atlas_without_lighting_unlit_controls() {
        use crate::render::vulkanic::gui_item_material::GuiAffineMaterial;
        use crate::render::vulkanic::shader_pack::lightmap::{VanillaLightmapFrame, VanillaLightmapInputs};
        let mut material = GuiAffineMaterial::FlatItemPending;
        material.resolve(Some(VanillaLightmapFrame { generation: 1, inputs: VanillaLightmapInputs {
            ambient_light_factor: 0.0, sky_factor: 1.0, block_factor: 1.5,
            night_vision_factor: 0.0, darkness_scale: 0.0, darken_world_factor: 0.0,
            brightness_factor: 0.0, sky_light_color: [1.0; 3], ambient_color: [1.0; 3],
        }})).unwrap();
        for blur in [None, Some((2, 2))] {
            for stratum in [0, 100] {
                let pixels = vec![255, 0, 0, 255, 255, 255, 255, 255];
                let actual = vulkan_gui_sample_with_material(stratum, pixels.clone(), 0.0, 1.0,
                    None, 1, false, false, Some([1, 0, 1, 1]), false, blur, material)
                    .expect("Vulkan required for explicit GUI item material regression");
                assert_eq!(actual, vec![252, 252, 252, 255]);
                let mixed = vulkan_gui_sample_with_material(stratum, pixels, 0.0, 1.0,
                    None, 1, false, false, Some([1, 0, 1, 1]), true, blur, material)
                    .expect("Vulkan required for unlit material isolation regression");
                assert_eq!(mixed, vec![126, 126, 254, 255]);
            }
        }
    }

    fn vulkan_gui_sample_with_material(stratum: u32, pixels: Vec<u8>, u0: f32, u1: f32,
        tiles: Option<crate::render::vulkanic::gui_tiling::GuiTileGeometry>, height: u32,
        invert: bool, prewarm_alpha: bool, atlas_region: Option<[u32; 4]>, mixed: bool,
        blur: Option<(i32, i32)>, material: crate::render::vulkanic::gui_item_material::GuiAffineMaterial) -> Option<Vec<u8>> {
        let backend = match VulkanBackend::new("MattMC GUI vignette frontend conformance") {
            Ok(backend) => backend,
            Err(error) => {
                eprintln!("skipping Vulkan GUI vignette frontend conformance: {error}");
                return None;
            }
        };
        let mut gal = VulkanicGal::new_with_backend(Box::new(backend), false);
        let extent = Extent3d {
            width: 1,
            height,
            depth: 1,
        };
        let color = gal
            .create_texture(TextureDesc {
                label: "gui-vignette-frontend.color".to_owned(),
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba8Unorm,
                extent,
                mip_levels: 1,
                array_layers: 1,
                usages: vec![TextureUsage::ColorAttachment, TextureUsage::TransferSrc],
            })
            .unwrap();
        let color_view = gal
            .create_texture_view(TextureViewDesc {
                label: "gui-vignette-frontend.color-view".to_owned(),
                texture: color,
                format: TextureFormat::Rgba8Unorm,
                base_mip: 0,
                mip_count: 1,
                base_layer: 0,
                layer_count: 1,
            })
            .unwrap();
        let target = gal
            .create_render_target(RenderTargetDesc {
                label: "gui-vignette-frontend.target".to_owned(),
                color_views: vec![color_view],
                depth_stencil_view: None,
                extent,
            })
            .unwrap();
        let clear_pass = gal
            .create_render_pass(RenderPassDesc {
                label: "gui-vignette-frontend.clear-pass".to_owned(),
                target,
                color_formats: vec![TextureFormat::Rgba8Unorm],
                depth_format: None,
            })
            .unwrap();
        let readback = gal
            .create_buffer(BufferDesc {
                label: "gui-vignette-frontend.readback".to_owned(),
                size: 4 * u64::from(height),
                memory: MemoryDomain::Readback,
                usages: vec![BufferUsage::TransferDst, BufferUsage::HostRead],
            })
            .unwrap();
        let mut frontend = GuiFrontend::default();
        let mut world = super::super::world_primitive_frontend::WorldPrimitiveFrontend::default();
        let atlas_reference = if let Some([x, y, width, region_height]) = atlas_region {
            use super::super::world_primitive_frontend::{WorldMeshTextureAssetPayload, WORLD_MATERIAL_TEXTURE_STONE};
            let image_width = (pixels.len() / 4) as u32 / height;
            let mut encoded = Vec::new();
            {
                let mut encoder = png::Encoder::new(&mut encoded, image_width, height);
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                encoder.write_header().unwrap().write_image_data(&pixels).unwrap();
            }
            world.apply_world_mesh_asset_update(&mut gal, 1, Vec::new(), vec![WorldMeshTextureAssetPayload {
                texture_id: WORLD_MATERIAL_TEXTURE_STONE, png_bytes: encoded, mip_png_bytes: Vec::new(),
                frame_width: 0, frame_height: 0, frame_count: 1, frame_ticks: 1,
                animation_flags: 0, frame_row_size: 0, interpolation_policy: 0,
                animation_frames: Vec::new(), coordinate_origin: 0, sampling: None, requested_mip_levels: 1,
            }]).unwrap();
            let reference = GuiAtlasReference { asset_id: 41,
                atlas: world.accepted_gui_atlas_incarnation(WORLD_MATERIAL_TEXTURE_STONE).unwrap(),
                x, y, width, height: region_height };
            frontend.stage_owned_atlas_references(&mut gal, &world, 1, &[reference]).unwrap();
            assert!(frontend.raw_images.is_empty(), "native atlas sampling must not stage copied GUI pixels");
            Some(reference)
        } else { None };
        if atlas_reference.is_none() {
        frontend
            .apply_raw_image_update(
                &mut gal,
                1,
                vec![GuiRawImageAssetPayload { sampling: None,
                    asset_id: 41,
                    format: GuiRawImageFormat::Rgba8,
                    width: (pixels.len() / 4) as u32 / height,
                    height,
                    pixels,
                }],
            )
            .unwrap();
        }
        if prewarm_alpha {
            frontend.ensure_resources(&mut gal, TextureGroup::Dynamic(41), ColorFormat::Rgba8Unorm,
                None, &mut GuiSubmitStats::default()).unwrap();
        }
        let template = GuiAffineQuadRequest {
                    item_raster_layers: vec![],
                    item_raster_scale: 0,
                    item_raster_geometry: Default::default(),
                    material,
                    stratum,
                    asset_id: 41,
                    x0: 0.0,
                    y0: 0.0,
                    x1: 1.0,
                    y1: 0.0,
                    x3: 0.0,
                    y3: height as f32,
                    z: 0.0,
                    u0,
                    v0: 0.0,
                    u1,
                    v1: 1.0,
                    color_argb: 0xffff_ffff,
                    gui_width: 1,
                    gui_height: height,
                    projection_extent: [1.0, height as f32],
                    sequence: 1,
                    clip_mode: 0,
                    clip_left: 0,
                    clip_top: 0,
                    clip_width: 0,
                    clip_height: 0,
                };
        let (quads, tiled) = if let Some(geometry) = tiles {
            (Vec::new(), vec![GuiTiledQuadRequest {
                geometry, stratum, asset_id: 41, z: 0.0, color_argb: 0xffff_ffff,
                gui_extent: [1, 1], projection_extent: [1.0, 1.0], sequence: 1, clip: None,
            }])
        } else { (vec![template], Vec::new()) };
        let mut quads = quads;
        if mixed {
            frontend.apply_raw_image_update(&mut gal, 1, vec![GuiRawImageAssetPayload { sampling: None,
                asset_id: 42, format: GuiRawImageFormat::Rgba8, width: 1, height: 1,
                pixels: vec![0, 0, 255, 128],
            }]).unwrap();
            let mut before = quads[0].clone();
            before.material = crate::render::vulkanic::gui_item_material::GuiAffineMaterial::Unlit;
            before.asset_id = 42; before.sequence = 1;
            let mut after = before.clone(); after.sequence = 3;
            quads[0].sequence = 2;
            // Deliberately scrambled input: semantic scheduling must put the
            // blue control before and after the opaque atlas sample.
            quads = vec![after, quads.remove(0), before];
        }
        let quad_count = preflight_tiled_affine_count(&tiled, quads.len()).unwrap();
        let (mut ops, stats) = if atlas_reference.is_some() {
            let before = gal.metrics().resource_creates;
            if let Some(valid) = quads.iter().find(|quad| quad.asset_id == 41) {
            let mut invalid = valid.clone();
            invalid.asset_id = 999;
            invalid.sequence = 99;
            assert!(frontend.append_owned_atlas_quads(&mut gal, &mut world,
                clear_pass, target, color_view, None, ColorFormat::Rgba8Unorm, None,
                false, &[valid.clone(), invalid]).is_err());
            assert_eq!(before, gal.metrics().resource_creates, "invalid semantic batch must reject before GPU allocation");
            }
            assert!(frontend.append_frame_ops_with_tiled_quads_to_target(&mut gal,
                1, target, color_view, Some(clear_pass), None, None, false,
                Vec::new(), quads.clone(), Vec::new(), tiled.clone()).is_err(),
                "mixed atlas submission must not bypass explicit owner access");
            assert_eq!(before, gal.metrics().resource_creates);
            let duplicate_affine = quads.iter().find(|quad| quad.asset_id == 41)
                .map(|quad| vec![quad.clone(), quad.clone()]).unwrap_or_default();
            let duplicate_tiles = tiled.first().map(|tile| vec![tile.clone(), tile.clone()])
                .unwrap_or_default();
            assert!(frontend.append_frame_ops_with_owned_atlases_to_target(&mut gal, Some(&mut world),
                1, target, color_view, Some(clear_pass), None, None, false,
                Vec::new(), duplicate_affine, Vec::new(), duplicate_tiles).is_err(),
                "duplicate parent commands must still reject even though tile children share an ID");
            assert_eq!(before, gal.metrics().resource_creates);
            if let Some((boundary, radius)) = blur {
                frontend.append_frame_ops_with_owned_atlases_and_blur_boundary(
                    &mut gal, Some(&mut world), 1, target, color_view,
                    Vec::new(), quads, Vec::new(), tiled, boundary, radius, false).unwrap()
            } else { frontend.append_frame_ops_with_owned_atlases_to_target(&mut gal, Some(&mut world),
                1, target, color_view, Some(clear_pass), None, None, false,
                Vec::new(), quads, Vec::new(), tiled).unwrap() }
        } else { frontend
            .append_frame_ops_with_tiled_quads_to_target(
                &mut gal, 1, target, color_view, None, None, None, false, Vec::new(), quads,
                Vec::new(), tiled,
            )
            .unwrap() };
        assert_eq!(quad_count as u64, stats.affine_quad_count);
        let mut commands = vec![
            CommandOp::Barrier(texture_barrier(
                color,
                TextureUsageState::Undefined,
                TextureUsageState::ColorAttachment,
            )),
            CommandOp::BeginPass {
                pass: clear_pass,
                target,
                colors: vec![PassAttachment {
                    view: color_view,
                    load_op: AttachmentLoadOp::Clear,
                    store_op: AttachmentStoreOp::Store,
                    clear_color: Some(ClearColor {
                        r: 0.8,
                        g: 0.8,
                        b: 0.8,
                        a: 1.0,
                    }),
                }],
                depth_stencil: None,
            },
            CommandOp::EndPass,
        ];
        commands.append(&mut ops);
        if invert {
            commands.extend(frontend.append_invert_post_effect(&mut gal, target, color_view).unwrap());
        }
        commands.extend([
            CommandOp::Barrier(texture_barrier(
                color,
                TextureUsageState::ColorAttachment,
                TextureUsageState::TransferSrc,
            )),
            CommandOp::CopyTextureToBuffer(BufferImageCopyRegion {
                buffer: readback,
                buffer_offset: 0,
                bytes_per_row: 4,
                rows_per_image: height,
                texture: color,
                texture_mip: 0,
                texture_layer: 0,
                texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
                extent,
            }),
            CommandOp::Barrier(buffer_barrier(
                readback,
                TextureUsageState::TransferDst,
                TextureUsageState::ShaderRead,
            )),
            CommandOp::HostReadBuffer {
                buffer: readback,
                offset: 0,
                size: 4 * u64::from(height),
            },
        ]);
        let list = gal
            .create_command_list(CommandListDesc {
                label: "gui-vignette-frontend.commands".to_owned(),
                operations: commands,
            })
            .unwrap();
        let token = gal
            .submit(SubmissionBatch {
                label: "gui-vignette-frontend.submit".to_owned(),
                command_lists: vec![list],
            })
            .unwrap();
        gal.retire_through_for_test(token.submission).unwrap();
        let bytes = gal
            .completed_host_reads()
            .iter()
            .rev()
            .find(|read| read.buffer == readback)
            .expect("GUI vignette frontend must produce a readback")
            .bytes
            .clone();
        frontend.reset(&mut gal).unwrap();
        world.reset(&mut gal);
        for handle in [clear_pass, target, color_view, color, readback] {
            gal.destroy(handle).unwrap();
        }
        gal.retire_through(gal.latest_submission_id()).unwrap();
        assert_eq!(gal.metrics().resource_creates, gal.metrics().resource_destroys,
            "GUI readback fixture must retire draw bindings, source owner and target resources");
        Some(bytes)
    }

    #[test]
    fn blur_boundary_replay_builds_owned_snapshot_copy_and_fullscreen_draw() {
        let mut gal = mock_gal();
        let target = frame_target(&mut gal);
        let mut frontend = GuiFrontend::default();
        let (ops, _stats) = frontend
            .append_frame_ops_with_blur_boundary(
                &mut gal,
                1,
                target,
                target,
                Vec::new(),
                Vec::new(),
                Vec::new(),
                1,
                0,
                false,
            )
            .unwrap();
        assert!(ops
            .iter()
            .any(|op| matches!(op, CommandOp::CopyFrameTargetToTexture { .. })));
        assert_eq!(6, ops.iter().filter(|op| matches!(
            op,
            CommandOp::Draw {
                vertices: 3,
                instances: 1
            }
        )).count(), "Frozen's menu blur requires all three horizontal/vertical pairs");
        let configs = ops.iter().filter_map(|op| match op {
            CommandOp::HostWriteBuffer { data, .. } if data.len() == 64 => Some(data),
            _ => None,
        }).collect::<Vec<_>>();
        assert_eq!(6, configs.len());
        for (index, bytes) in configs.into_iter().enumerate() {
            let x = f32::from_le_bytes(bytes[..4].try_into().unwrap());
            let y = f32::from_le_bytes(bytes[4..8].try_into().unwrap());
            assert_eq!([x, y], if index % 2 == 0 { [1.0, 0.0] } else { [0.0, 1.0] });
        }
        assert!(std::str::from_utf8(BLUR_FRAGMENT_SHADER_VULKAN).unwrap()
            .contains("gl_FragCoord.xy * texel"));
        gal.submit(SubmissionBatch {
            label: "gui-blur-replay".to_owned(),
            command_lists: vec![CommandList::from(CommandListDesc {
                label: "gui-blur-replay-commands".to_owned(),
                operations: ops,
            })],
        })
        .unwrap();
        let (next_ops, _) = frontend
            .append_frame_ops_with_blur_boundary(
                &mut gal,
                1,
                target,
                target,
                Vec::new(),
                Vec::new(),
                Vec::new(),
                1,
                0,
                false,
            )
            .unwrap();
        assert!(matches!(
            next_ops.iter().find(|op| matches!(
                op,
                CommandOp::Barrier(ResourceBarrier {
                    before: TextureUsageState::ShaderRead,
                    after: TextureUsageState::TransferDst,
                    ..
                })
            )),
            Some(CommandOp::Barrier(ResourceBarrier {
                before: TextureUsageState::ShaderRead,
                after: TextureUsageState::TransferDst,
                ..
            }))
        ));
        gal.submit(SubmissionBatch {
            label: "gui-blur-replay-next-frame".to_owned(),
            command_lists: vec![CommandList::from(CommandListDesc {
                label: "gui-blur-replay-next-frame-commands".to_owned(),
                operations: next_ops,
            })],
        })
        .unwrap();
    }

    #[test]
    fn invert_post_effect_replay_is_rust_owned_and_precedes_gui_composition() {
        let mut gal = mock_gal();
        let target = frame_target(&mut gal);
        let mut frontend = GuiFrontend::default();
        let ops = frontend
            .append_invert_post_effect(&mut gal, target, target)
            .unwrap();
        assert!(ops
            .iter()
            .any(|op| matches!(op, CommandOp::CopyFrameTargetToTexture { .. })));
        let draw_index = ops
            .iter()
            .position(|op| {
                matches!(
                    op,
                    CommandOp::Draw {
                        vertices: 3,
                        instances: 1
                    }
                )
            })
            .expect("invert effect must emit a fullscreen draw");
        assert!(ops[..draw_index].iter().any(|op| matches!(
            op,
            CommandOp::Barrier(ResourceBarrier {
                before: TextureUsageState::TransferDst,
                after: TextureUsageState::ShaderRead,
                ..
            })
        )));
        gal.create_command_list(CommandListDesc {
            label: "gui-invert-replay".to_owned(),
            operations: ops,
        })
        .unwrap();
    }

    #[test]
    fn bounded_custom_post_effect_replay_uses_owned_snapshot_and_explicit_bindings() {
        let mut gal = mock_gal();
        let target = frame_target(&mut gal);
        let mut frontend = GuiFrontend::default();
        let vertex = br#"#version 330
out vec2 texCoord;
void main() { texCoord = vec2(0.0); }
"#;
        let fragment = br#"#version 330
in vec2 texCoord;
uniform sampler2D InSampler;
out vec4 fragColor;
void main() { fragColor = texture(InSampler, texCoord); }
"#;
        let ops = frontend
            .append_custom_post_effect(
                &mut gal,
                target,
                target,
                "minecraft:test_custom",
                &[
                    CustomPostEffectSource {
                        input_row_order: crate::render::vulkanic::commands::TextureRowOrder::Preserve,
                        sampler_info_uniform: None,
                        vertex_shader: vertex.to_vec(),
                        fragment_shader: fragment.to_vec(),
                        input_count: 1,
                        input_bilinear: vec![false; 1],
                        input_targets: vec!["minecraft:main".to_owned()],
                        input_images: vec![None],
                        input_use_depth: vec![false],
                        output_target: "minecraft:main".to_owned(),
                        uniform_blocks: vec![vec![7; 16]],
                    },
                    CustomPostEffectSource {
                        input_row_order: crate::render::vulkanic::commands::TextureRowOrder::Preserve,
                        sampler_info_uniform: None,
                        vertex_shader: vertex.to_vec(),
                        fragment_shader: fragment.to_vec(),
                        input_count: 1,
                        input_bilinear: vec![false; 1],
                        input_targets: vec!["minecraft:main".to_owned()],
                        input_images: vec![None],
                        input_use_depth: vec![false],
                        output_target: "minecraft:main".to_owned(),
                        uniform_blocks: vec![vec![7; 16]],
                    },
                ],
            )
            .unwrap();
        assert!(ops
            .iter()
            .any(|op| matches!(op, CommandOp::CopyFrameTargetToTexture { .. })));
        assert!(ops
            .iter()
            .any(|op| matches!(op, CommandOp::BindResourceSet { .. })));
        assert!(ops.iter().any(|op| matches!(
            op,
            CommandOp::HostWriteBuffer { data, .. } if data == &vec![7; 16]
        )));
        for (index, op) in ops.iter().enumerate() {
            if let CommandOp::HostWriteBuffer { buffer, data, .. } = op {
                if data == &vec![7; 16] {
                    assert!(matches!(ops.get(index + 1), Some(CommandOp::Barrier(barrier))
                        if barrier.resource == *buffer
                            && barrier.before == TextureUsageState::TransferDst
                            && barrier.after == TextureUsageState::ShaderRead),
                        "every post-effect uniform upload needs an explicit read dependency");
                }
            }
        }
        assert_eq!(
            2,
            ops.iter()
                .filter(|op| matches!(
                    op,
                    CommandOp::Draw {
                        vertices: 3,
                        instances: 1
                    }
                ))
                .count()
        );
        gal.create_command_list(CommandListDesc {
            label: "gui-custom-post-effect-replay".to_owned(),
            operations: ops,
        })
        .unwrap();
    }

    #[test]
    fn custom_post_effect_texture_input_uploads_and_binds_rust_owned_pixels() {
        let mut gal = mock_gal();
        let target = frame_target(&mut gal);
        let mut frontend = GuiFrontend::default();
        let vertex = br#"#version 330
out vec2 texCoord;
void main() { texCoord = vec2(0.0); }
"#;
        let fragment = br#"#version 330
in vec2 texCoord;
uniform sampler2D MaskSampler;
out vec4 fragColor;
void main() { fragColor = texture(MaskSampler, texCoord); }
"#;
        let pixels: Vec<u8> = (1..=24).collect();
        let ops = frontend
            .append_custom_post_effect(
                &mut gal,
                target,
                target,
                "minecraft:test_texture_input",
                &[CustomPostEffectSource {
                    input_row_order: crate::render::vulkanic::commands::TextureRowOrder::Reverse,
                    sampler_info_uniform: None,
                    vertex_shader: vertex.to_vec(),
                    fragment_shader: fragment.to_vec(),
                    input_count: 1,
                        input_bilinear: vec![false; 1],
                    input_targets: vec![String::new()],
                    input_images: vec![Some(CustomPostEffectImage {
                        path: "textures/effect/mask.png".to_owned(),
                        width: 2,
                        height: 3,
                        pixels_rgba8: pixels.clone(),
                        bilinear: false,
                    })],
                    input_use_depth: vec![false],
                    output_target: "minecraft:main".to_owned(),
                    uniform_blocks: Vec::new(),
                }],
            )
            .unwrap();
        assert!(ops.iter().any(|op| matches!(
            op,
            CommandOp::HostWriteBuffer { data, .. } if data == &pixels
        )));
        assert!(ops
            .iter()
            .any(|op| matches!(op, CommandOp::CopyBufferToTexture(_))));
        assert!(ops
            .iter()
            .any(|op| matches!(op, CommandOp::BindResourceSet { .. })));
        assert!(!ops
            .iter()
            .any(|op| matches!(op, CommandOp::CopyFrameTargetToTexture { .. })));
        assert!(!ops.iter().any(|op| matches!(op, CommandOp::CopyTexture(_))),
            "uploaded image rows must not inherit framebuffer row conversion");
        let second_ops = frontend
            .append_custom_post_effect(
                &mut gal,
                target,
                target,
                "minecraft:test_texture_input",
                &[CustomPostEffectSource {
                    input_row_order: crate::render::vulkanic::commands::TextureRowOrder::Reverse,
                    sampler_info_uniform: None,
                    vertex_shader: vertex.to_vec(),
                    fragment_shader: fragment.to_vec(),
                    input_count: 1,
                        input_bilinear: vec![false; 1],
                    input_targets: vec![String::new()],
                    input_images: vec![Some(CustomPostEffectImage {
                        path: "textures/effect/mask.png".to_owned(),
                        width: 2,
                        height: 3,
                        pixels_rgba8: pixels,
                        bilinear: false,
                    })],
                    input_use_depth: vec![false],
                    output_target: "minecraft:main".to_owned(),
                    uniform_blocks: Vec::new(),
                }],
            )
            .unwrap();
        assert!(!second_ops
            .iter()
            .any(|op| matches!(op, CommandOp::CopyBufferToTexture(_))));
        let depth_error = frontend
            .append_custom_post_effect(
                &mut gal,
                target,
                target,
                "minecraft:test_depth_input",
                &[CustomPostEffectSource {
                    input_row_order: crate::render::vulkanic::commands::TextureRowOrder::Preserve,
                    sampler_info_uniform: None,
                    vertex_shader: vertex.to_vec(),
                    fragment_shader: fragment.to_vec(),
                    input_count: 1,
                        input_bilinear: vec![false; 1],
                    input_targets: vec!["minecraft:main".to_owned()],
                    input_images: vec![None],
                    input_use_depth: vec![true],
                    output_target: "minecraft:main".to_owned(),
                    uniform_blocks: Vec::new(),
                }],
            )
            .unwrap_err();
        assert!(depth_error
            .to_string()
            .contains("requires a Rust-owned render-target depth attachment"));
    }

    #[test]
    fn custom_post_effect_depth_input_uses_color_only_alias_and_explicit_barriers() {
        let mut gal = mock_gal();
        let (target, color_view, depth_texture) = depth_render_target(&mut gal);
        let mut frontend = GuiFrontend::default();
        let source = CustomPostEffectSource {
            input_row_order: crate::render::vulkanic::commands::TextureRowOrder::Preserve,
            sampler_info_uniform: None,
            vertex_shader: br#"#version 330
out vec2 texCoord; void main() { texCoord = vec2(0.0); }"#
                .to_vec(),
            fragment_shader: br#"#version 330
in vec2 texCoord; uniform sampler2D MainDepthSampler; out vec4 fragColor;
void main() { fragColor = texture(MainDepthSampler, texCoord); }"#
                .to_vec(),
            input_count: 1,
                        input_bilinear: vec![false; 1],
            input_targets: vec!["minecraft:main".to_owned()],
            input_images: vec![None],
            input_use_depth: vec![true],
            output_target: "minecraft:main".to_owned(),
            uniform_blocks: Vec::new(),
        };
        let ops = frontend
            .append_custom_post_effect(
                &mut gal,
                target,
                color_view,
                "minecraft:test-depth-alias",
                &[source],
            )
            .unwrap();
        assert!(ops.iter().any(|op| matches!(
            op,
            CommandOp::Barrier(ResourceBarrier { resource, before: TextureUsageState::DepthStencilAttachment, after: TextureUsageState::ShaderRead, .. }) if *resource == depth_texture
        )));
        assert!(ops.iter().any(|op| matches!(
            op,
            CommandOp::Barrier(ResourceBarrier { resource, before: TextureUsageState::ShaderRead, after: TextureUsageState::DepthStencilAttachment, .. }) if *resource == depth_texture
        )));
        assert!(ops.iter().any(|op| matches!(
            op,
            CommandOp::BeginPass { target: pass_target, depth_stencil: None, .. } if *pass_target != target
        )));
    }

    #[test]
    fn frame_gui_depth_declaration_requires_a_complete_pair() {
        let mut gal = mock_gal();
        let target = frame_target(&mut gal);
        let mut frontend = GuiFrontend::default();
        let error = frontend
            .append_frame_ops_with_affine_quads_to_target(
                &mut gal,
                1,
                target,
                target,
                None,
                None,
                Some(TextureFormat::Depth32Float),
                false,
                Vec::new(),
                Vec::new(),
            )
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("depth attachment and depth format"));
        let (_, depth_view) = gal.frame_target_owned_depth_attachment(target).unwrap();
        let (ops, _) = frontend
            .append_frame_ops_with_affine_quads_to_target(
                &mut gal,
                1,
                target,
                target,
                None,
                Some(depth_view),
                Some(TextureFormat::Depth32Float),
                false,
                Vec::new(),
                Vec::new(),
            )
            .unwrap();
        gal.create_command_list(CommandListDesc {
            label: "gui-frame-owned-depth-pass".to_owned(),
            operations: ops,
        })
        .unwrap();
    }

    #[test]
    fn creeper_post_effect_replay_owns_intermediate_target_and_two_fullscreen_passes() {
        let mut gal = mock_gal();
        let target = frame_target(&mut gal);
        let mut frontend = GuiFrontend::default();
        let ops = frontend
            .append_creeper_post_effect(&mut gal, target, target)
            .unwrap();
        assert!(ops
            .iter()
            .any(|op| matches!(op, CommandOp::CopyFrameTargetToTexture { .. })));
        assert_eq!(
            2,
            ops.iter()
                .filter(|op| matches!(
                    op,
                    CommandOp::Draw {
                        vertices: 3,
                        instances: 1
                    }
                ))
                .count()
        );
        assert!(ops.iter().any(|op| matches!(
            op,
            CommandOp::BeginPass { target: pass_target, .. } if *pass_target != target
        )));
        gal.create_command_list(CommandListDesc {
            label: "gui-creeper-replay".to_owned(),
            operations: ops,
        })
        .unwrap();
        let next_ops = frontend
            .append_creeper_post_effect(&mut gal, target, target)
            .unwrap();
        assert!(next_ops.iter().any(|op| matches!(
            op,
            CommandOp::Barrier(ResourceBarrier {
                before: TextureUsageState::ShaderRead,
                after: TextureUsageState::ColorAttachment,
                ..
            })
        )));
    }

    #[test]
    fn bounded_custom_post_effect_replay_owns_one_intermediate_target() {
        let mut gal = mock_gal();
        let target = frame_target(&mut gal);
        let mut frontend = GuiFrontend::default();
        let vertex = br#"#version 330
out vec2 texCoord;
void main() { texCoord = vec2(0.0); }
"#;
        let fragment = br#"#version 330
in vec2 texCoord;
uniform sampler2D InSampler;
out vec4 fragColor;
void main() { fragColor = texture(InSampler, texCoord); }
"#;
        let ops = frontend
            .append_custom_post_effect(
                &mut gal,
                target,
                target,
                "minecraft:test_intermediate",
                &[
                    CustomPostEffectSource {
                        input_row_order: crate::render::vulkanic::commands::TextureRowOrder::Preserve,
                        sampler_info_uniform: None,
                        vertex_shader: vertex.to_vec(),
                        fragment_shader: fragment.to_vec(),
                        input_count: 1,
                        input_bilinear: vec![false; 1],
                        input_targets: vec!["minecraft:main".to_owned()],
                        input_images: vec![None],
                        input_use_depth: vec![false],
                        output_target: "intermediate".to_owned(),
                        uniform_blocks: Vec::new(),
                    },
                    CustomPostEffectSource {
                        input_row_order: crate::render::vulkanic::commands::TextureRowOrder::Preserve,
                        sampler_info_uniform: None,
                        vertex_shader: vertex.to_vec(),
                        fragment_shader: fragment.to_vec(),
                        input_count: 1,
                        input_bilinear: vec![false; 1],
                        input_targets: vec!["intermediate".to_owned()],
                        input_images: vec![None],
                        input_use_depth: vec![false],
                        output_target: "minecraft:main".to_owned(),
                        uniform_blocks: Vec::new(),
                    },
                ],
            )
            .unwrap();
        assert!(ops.iter().any(|op| matches!(
            op,
            CommandOp::BeginPass { target: pass_target, .. } if *pass_target != target
        )));
        assert!(ops
            .iter()
            .any(|op| matches!(op, CommandOp::CopyFrameTargetToTexture { .. })));
        assert_eq!(
            2,
            ops.iter()
                .filter(|op| matches!(
                    op,
                    CommandOp::Draw {
                        vertices: 3,
                        instances: 1
                    }
                ))
                .count()
        );
        gal.create_command_list(CommandListDesc {
            label: "gui-custom-intermediate-replay".to_owned(),
            operations: ops.clone(),
        })
        .unwrap();
        let intermediate = &frontend.custom_post_effect_intermediates["intermediate"];
        let begin = ops.iter().position(|op| matches!(op, CommandOp::BeginPass { target, .. }
            if *target == intermediate.target)).unwrap();
        assert!(ops[..begin].iter().any(|op| matches!(op, CommandOp::Barrier(barrier)
            if barrier.resource == intermediate.texture && barrier.before == TextureUsageState::Undefined
                && barrier.after == TextureUsageState::ColorAttachment)));
        assert_eq!(TextureUsageState::ShaderRead, intermediate.usage);
    }

    #[test]
    fn custom_post_effect_sampler_info_packs_explicit_output_and_input_extents() {
        let image = CustomPostEffectImage { path: "test".into(), width: 8, height: 4,
            pixels_rgba8: vec![0; 8 * 4 * 4], bilinear: false };
        let bytes = custom_sampler_info_bytes(Extent3d { width: 320, height: 180, depth: 1 },
            &[None, Some(image)]);
        assert_eq!(32, bytes.len());
        let values = bytes.chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect::<Vec<_>>();
        assert_eq!(vec![320.0, 180.0, 320.0, 180.0, 8.0, 4.0, 0.0, 0.0], values);
    }

    #[test]
    fn custom_post_effect_external_target_lowering_uses_owned_attachment_pass() {
        let mut gal = mock_gal();
        let extent = Extent3d {
            width: 320,
            height: 180,
            depth: 1,
        };
        let mut make_target = |label: &str| {
            let texture = gal
                .create_texture(TextureDesc {
                    label: format!("{label}.texture"),
                    dimension: TextureDimension::D2,
                    format: TextureFormat::Rgba8Unorm,
                    extent,
                    mip_levels: 1,
                    array_layers: 1,
                    usages: vec![
                        TextureUsage::Sampled,
                        TextureUsage::ColorAttachment,
                        TextureUsage::TransferSrc,
                        TextureUsage::TransferDst,
                    ],
                })
                .unwrap();
            let view = gal
                .create_texture_view(TextureViewDesc {
                    label: format!("{label}.view"),
                    texture,
                    format: TextureFormat::Rgba8Unorm,
                    base_mip: 0,
                    mip_count: 1,
                    base_layer: 0,
                    layer_count: 1,
                })
                .unwrap();
            let target = gal
                .create_render_target(RenderTargetDesc {
                    label: format!("{label}.target"),
                    color_views: vec![view],
                    depth_stencil_view: None,
                    extent,
                })
                .unwrap();
            let pass = gal
                .create_render_pass(RenderPassDesc {
                    label: format!("{label}.pass"),
                    target,
                    color_formats: vec![TextureFormat::Rgba8Unorm],
                    depth_format: None,
                })
                .unwrap();
            let sampler = gal
                .create_sampler(SamplerDesc {
                    label: format!("{label}.sampler"),
                    min_filter: SamplerFilter::Linear,
                    mag_filter: SamplerFilter::Linear,
                    mip_filter: SamplerFilter::Nearest,
                    address_u: SamplerAddressMode::ClampToEdge,
                    address_v: SamplerAddressMode::ClampToEdge,
                    address_w: SamplerAddressMode::ClampToEdge,
                    comparison: None,
                })
                .unwrap();
            (target, view, pass, sampler)
        };
        let (main_target, main_view, main_pass, main_sampler) = make_target("external-main");
        let (external_target, external_view, external_pass, external_sampler) =
            make_target("external-role");
        let plan = super::super::shader_pack::vanilla_post_effect_contract::VanillaPostEffectExecutionPlan {
            effect_name: "minecraft:external-test".to_owned(),
            intermediate_targets: Vec::new(),
            ordered_passes: vec![super::super::shader_pack::vanilla_post_effect_contract::VanillaPostEffectPass {
                vertex_shader: "vertex".to_owned(),
                fragment_shader: "fragment".to_owned(),
                inputs: vec![super::super::shader_pack::vanilla_post_effect_contract::VanillaPostEffectInput {
                    sampler_name: "InSampler".to_owned(),
                    target: "minecraft:main".to_owned(),
                    texture_path: None,
                    texture_width: None,
                    texture_height: None,
                    bilinear: true,
                    use_depth_buffer: false,
                }],
                output: "minecraft:translucent".to_owned(),
                uniform_blocks: BTreeSet::new(),
                uniform_values: BTreeMap::new(),
            }],
        };
        let external = super::super::shader_pack::vanilla_post_effect_executor::VanillaPostEffectExternalTargetBindings::new(
            &plan,
            BTreeMap::from([
                (
                    "minecraft:main".to_owned(),
                    super::super::shader_pack::vanilla_post_effect_executor::VanillaPostEffectExternalTargetBinding {
                        render_pass: main_pass,
                        render_target: main_target,
                        color_attachment: main_view,
                        depth_attachment: None,
                        sampler: main_sampler,
                        color_usage: TextureUsageState::ColorAttachment,
                        depth_usage: None,
                    },
                ),
                (
                    "minecraft:translucent".to_owned(),
                    super::super::shader_pack::vanilla_post_effect_executor::VanillaPostEffectExternalTargetBinding {
                        render_pass: external_pass,
                        render_target: external_target,
                        color_attachment: external_view,
                        depth_attachment: None,
                        sampler: external_sampler,
                        color_usage: TextureUsageState::ColorAttachment,
                        depth_usage: None,
                    },
                ),
            ]),
        ).unwrap();
        let vertex = br#"#version 330
out vec2 texCoord;
void main() { texCoord = vec2(0.0); }
"#;
        let fragment = br#"#version 330
in vec2 texCoord;
uniform sampler2D InSampler;
out vec4 fragColor;
void main() { fragColor = texture(InSampler, texCoord); }
"#;
        let mut frontend = GuiFrontend::default();
        let ops = frontend
            .append_custom_post_effect_with_external_targets(
                &mut gal,
                main_target,
                main_view,
                "minecraft:external-test",
                &[CustomPostEffectSource {
                    input_row_order: crate::render::vulkanic::commands::TextureRowOrder::Preserve,
                    sampler_info_uniform: None,
                    vertex_shader: vertex.to_vec(),
                    fragment_shader: fragment.to_vec(),
                    input_count: 1,
                        input_bilinear: vec![false; 1],
                    input_targets: vec!["minecraft:main".to_owned()],
                    input_images: vec![None],
                    input_use_depth: vec![false],
                    output_target: "minecraft:translucent".to_owned(),
                    uniform_blocks: Vec::new(),
                }],
                Some(&external),
            )
            .unwrap();
        assert!(ops.iter().any(|op| matches!(
            op,
            CommandOp::CopyTexture(TextureImageCopyRegion { src_texture, .. })
                if *src_texture == gal.pass_target_color_texture(main_target).unwrap()
        )));
        assert!(ops.iter().any(|op| matches!(
            op,
            CommandOp::BeginPass { pass, target, .. }
                if *pass == external_pass && *target == external_target
        )));
        gal.create_command_list(CommandListDesc {
            label: "gui-custom-external-target".to_owned(),
            operations: ops,
        })
        .unwrap();
    }

    #[test]
    fn bounded_custom_post_effect_replay_owns_multiple_private_targets() {
        let mut gal = mock_gal();
        let target = frame_target(&mut gal);
        let mut frontend = GuiFrontend::default();
        let vertex = br#"#version 330
out vec2 texCoord;
void main() { texCoord = vec2(0.0); }
"#;
        let fragment = br#"#version 330
in vec2 texCoord;
uniform sampler2D InSampler;
out vec4 fragColor;
void main() { fragColor = texture(InSampler, texCoord); }
"#;
        let ops = frontend
            .append_custom_post_effect(
                &mut gal,
                target,
                target,
                "minecraft:test_multi_intermediate",
                &[
                    CustomPostEffectSource {
                        input_row_order: crate::render::vulkanic::commands::TextureRowOrder::Preserve,
                        sampler_info_uniform: None,
                        vertex_shader: vertex.to_vec(),
                        fragment_shader: fragment.to_vec(),
                        input_count: 1,
                        input_bilinear: vec![false; 1],
                        input_targets: vec!["minecraft:main".into()],
                        input_images: vec![None],
                        input_use_depth: vec![false],
                        output_target: "first".into(),
                        uniform_blocks: Vec::new(),
                    },
                    CustomPostEffectSource {
                        input_row_order: crate::render::vulkanic::commands::TextureRowOrder::Preserve,
                        sampler_info_uniform: None,
                        vertex_shader: vertex.to_vec(),
                        fragment_shader: fragment.to_vec(),
                        input_count: 1,
                        input_bilinear: vec![false; 1],
                        input_targets: vec!["minecraft:main".into()],
                        input_images: vec![None],
                        input_use_depth: vec![false],
                        output_target: "second".into(),
                        uniform_blocks: Vec::new(),
                    },
                    CustomPostEffectSource {
                        input_row_order: crate::render::vulkanic::commands::TextureRowOrder::Preserve,
                        sampler_info_uniform: None,
                        vertex_shader: vertex.to_vec(),
                        fragment_shader: fragment.to_vec(),
                        input_count: 1,
                        input_bilinear: vec![false; 1],
                        input_targets: vec!["first".into()],
                        input_images: vec![None],
                        input_use_depth: vec![false],
                        output_target: "minecraft:main".into(),
                        uniform_blocks: Vec::new(),
                    },
                ],
            )
            .unwrap();
        let private_passes = ops
            .iter()
            .filter_map(|op| match op {
                CommandOp::BeginPass {
                    target: pass_target,
                    ..
                } if *pass_target != target => Some(*pass_target),
                _ => None,
            })
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(2, private_passes.len());
        assert_eq!(
            3,
            ops.iter()
                .filter(|op| matches!(
                    op,
                    CommandOp::Draw {
                        vertices: 3,
                        instances: 1
                    }
                ))
                .count()
        );
        gal.create_command_list(CommandListDesc {
            label: "gui-custom-multi-intermediate-replay".into(),
            operations: ops,
        })
        .unwrap();
    }

    #[test]
    fn bounded_custom_post_effect_binds_distinct_input_snapshots() {
        let mut gal = mock_gal();
        let target = frame_target(&mut gal);
        let mut frontend = GuiFrontend::default();
        let vertex = br#"#version 330
out vec2 texCoord;
void main() { texCoord = vec2(0.0); }
"#;
        let fragment = br#"#version 330
in vec2 texCoord;
uniform sampler2D MainSampler;
uniform sampler2D PrivateSampler;
out vec4 fragColor;
void main() { fragColor = texture(MainSampler, texCoord) + texture(PrivateSampler, texCoord) * 0.0; }
"#;
        let ops = frontend
            .append_custom_post_effect(
                &mut gal,
                target,
                target,
                "minecraft:test-distinct-inputs",
                &[
                    CustomPostEffectSource {
                        input_row_order: crate::render::vulkanic::commands::TextureRowOrder::Preserve,
                        sampler_info_uniform: None,
                        vertex_shader: vertex.to_vec(),
                        fragment_shader: fragment.to_vec(),
                        input_count: 1,
                        input_bilinear: vec![false; 1],
                        input_targets: vec!["minecraft:main".to_owned()],
                        input_images: vec![None],
                        input_use_depth: vec![false],
                        output_target: "private".to_owned(),
                        uniform_blocks: Vec::new(),
                    },
                    CustomPostEffectSource {
                        input_row_order: crate::render::vulkanic::commands::TextureRowOrder::Preserve,
                        sampler_info_uniform: None,
                        vertex_shader: vertex.to_vec(),
                        fragment_shader: fragment.to_vec(),
                        input_count: 2,
                        input_bilinear: vec![false; 2],
                        input_targets: vec!["minecraft:main".to_owned(), "private".to_owned()],
                        input_images: vec![None, None],
                        input_use_depth: vec![false, false],
                        output_target: "minecraft:main".to_owned(),
                        uniform_blocks: Vec::new(),
                    },
                ],
            )
            .unwrap();
        assert_eq!(
            2,
            ops.iter()
                .filter(|op| matches!(op, CommandOp::CopyFrameTargetToTexture { .. }))
                .count()
        );
        assert_eq!(
            2,
            ops.iter()
                .filter(|op| matches!(op, CommandOp::BindResourceSet { .. }))
                .count()
        );
        gal.create_command_list(CommandListDesc {
            label: "gui-custom-distinct-input-replay".to_owned(),
            operations: ops,
        })
        .unwrap();
    }

    #[test]
    fn custom_post_effect_reversed_frame_snapshots_reuse_scratch_and_retire_all_resources() {
        let mut gal = mock_gal();
        let target = frame_target(&mut gal);
        let mut frontend = GuiFrontend::default();
        let mut live = None;
        for iteration in 0..8 {
            let source = row_conversion_fixture(false, 2);
            let ops = frontend.append_custom_post_effect(&mut gal, target, target,
                "row-conversion-fixture", &[source]).unwrap();
            let scratch = frontend.custom_post_effect_resources[0].frame_copy_scratch.unwrap();
            assert_eq!(2, ops.iter().filter(|op| matches!(op,
                CommandOp::CopyTexture(region) if region.row_order == super::super::commands::TextureRowOrder::Reverse
                    && region.src_texture == scratch)).count());
            let starts_undefined = ops.iter().any(|op| matches!(op,
                CommandOp::Barrier(ResourceBarrier { resource, before: TextureUsageState::Undefined, .. })
                    if *resource == scratch));
            assert_eq!(iteration == 0, starts_undefined);
            let list = gal.create_command_list(CommandListDesc { label: "row-fixture".into(), operations: ops }).unwrap();
            let token = gal.submit(SubmissionBatch { label: "row-fixture".into(), command_lists: vec![list] }).unwrap();
            gal.retire_through_for_test(token.submission).unwrap();
            let count = gal.metrics().resource_creates - gal.metrics().resource_destroys;
            assert_eq!(*live.get_or_insert(count), count);
        }
        frontend.reset(&mut gal).unwrap();
        gal.destroy(target).unwrap();
        assert_eq!(gal.metrics().resource_creates, gal.metrics().resource_destroys);
    }

    fn row_conversion_fixture(depth: bool, count: usize) -> CustomPostEffectSource {
        CustomPostEffectSource {
            input_row_order: super::super::commands::TextureRowOrder::Reverse,
            input_bilinear: vec![false; count], sampler_info_uniform: None,
            vertex_shader: b"#version 450\nvoid main() { gl_Position=vec4(0.0); }".to_vec(),
            fragment_shader: b"#version 450\nlayout(location=0) out vec4 color; void main() { color=vec4(1.0); }".to_vec(),
            input_count: count, input_targets: vec!["minecraft:main".into(); count],
            input_images: vec![None; count], input_use_depth: vec![depth; count],
            output_target: "minecraft:main".into(), uniform_blocks: vec![],
        }
    }

    #[test]
    fn custom_post_effect_depth_does_not_substitute_main_for_a_color_only_private_target() {
        let mut gal = mock_gal();
        let (target, color, _) = depth_render_target(&mut gal);
        let mut frontend = GuiFrontend::default();
        let mut produce = row_conversion_fixture(false, 1);
        produce.output_target = "private-color".into();
        let mut consume = row_conversion_fixture(true, 1);
        consume.input_targets[0] = "private-color".into();
        let creates = gal.metrics().resource_creates;
        let result = frontend.append_custom_post_effect(&mut gal, target, color,
            "missing-named-depth", &[produce, consume]);
        assert!(result.is_err(), "a named depth input must not sample main depth instead");
        assert_eq!(creates, gal.metrics().resource_creates, "reject missing depth before graph allocation");
    }

    #[test]
    fn custom_post_effect_reversed_depth_uses_matching_owned_snapshot() {
        let mut gal = mock_gal();
        let (target, color_view, depth) = depth_render_target(&mut gal);
        let mut frontend = GuiFrontend::default();
        let ops = frontend.append_custom_post_effect(&mut gal, target, color_view,
            "depth-row-conversion", &[row_conversion_fixture(true, 1)]).unwrap();
        let resources = &frontend.custom_post_effect_resources[0];
        assert!(resources.frame_copy_scratch.is_none());
        let snapshot = resources.snapshots[0];
        let info = gal.texture_view_info(resources.snapshot_views[0]).unwrap();
        assert_eq!(TextureFormat::Depth32Float, info.format);
        assert_ne!(depth, info.texture);
        assert!(ops.iter().any(|op| matches!(op, CommandOp::CopyTexture(region)
            if region.src_texture == depth && region.dst_texture == snapshot
                && region.row_order == super::super::commands::TextureRowOrder::Reverse)));
        assert!(!ops.iter().any(|op| matches!(op, CommandOp::Barrier(ResourceBarrier {
            resource, after: TextureUsageState::ShaderRead, .. }) if *resource == depth)));
        gal.create_command_list(CommandListDesc { label: "depth-row-fixture".into(), operations: ops }).unwrap();
    }

    #[test]
    fn custom_post_effect_target_filters_are_explicit_and_invalidate_cached_resources() {
        let mut gal = mock_gal();
        let target = frame_target(&mut gal);
        let mut frontend = GuiFrontend::default();
        let mut previous = Vec::new();
        for flags in [vec![false, true], vec![true, false]] {
            let source = CustomPostEffectSource {
                input_row_order: crate::render::vulkanic::commands::TextureRowOrder::Preserve,
                input_bilinear: flags.clone(), sampler_info_uniform: None,
                vertex_shader: b"#version 450\nvoid main() { gl_Position=vec4(0.0); }".to_vec(),
                fragment_shader: b"#version 450\nlayout(location=0) out vec4 color; void main() { color=vec4(1.0); }".to_vec(),
                input_count: 2, input_targets: vec!["minecraft:main".into(); 2],
                input_images: vec![None; 2], input_use_depth: vec![false; 2],
                output_target: "minecraft:main".into(), uniform_blocks: Vec::new(),
            };
            frontend.append_custom_post_effect(&mut gal, target, target,
                "minecraft:sampler-fixture", &[source]).unwrap();
            for handle in &previous {
                assert!(gal.sampler_descriptor_for_test(*handle).is_err(), "changed filters must retire old bindings");
            }
            let samplers = &frontend.custom_post_effect_resources[0].target_samplers;
            assert_eq!(2, samplers.len());
            for (handle, bilinear) in samplers.iter().zip(flags) {
                let desc = gal.sampler_descriptor_for_test(*handle).unwrap();
                let expected = if bilinear { SamplerFilter::Linear } else { SamplerFilter::Nearest };
                assert_eq!(expected, desc.min_filter);
                assert_eq!(expected, desc.mag_filter);
            }
            previous = samplers.clone();
        }
        frontend.reset(&mut gal).unwrap();
        gal.destroy(target).unwrap();
        assert_eq!(gal.metrics().resource_creates, gal.metrics().resource_destroys);
    }

    #[test]
    fn cancelled_post_effect_preparation_restarts_with_fresh_image_states() {
        let mut gal = mock_gal();
        let target = frame_target(&mut gal);
        let mut frontend = GuiFrontend::default();
        let source = |input: &str, output: &str| CustomPostEffectSource {
            input_row_order: crate::render::vulkanic::commands::TextureRowOrder::Preserve,
            sampler_info_uniform: None,
            vertex_shader: b"#version 450\nvoid main() { gl_Position = vec4(0.0); }".to_vec(),
            fragment_shader: b"#version 450\nlayout(location=0) out vec4 color; void main() { color=vec4(1.0); }".to_vec(),
            input_count: 1,
                        input_bilinear: vec![false; 1], input_targets: vec![input.into()], input_images: vec![None],
            input_use_depth: vec![false], output_target: output.into(), uniform_blocks: Vec::new(),
        };
        let sources = [source("minecraft:main", "swap"), source("swap", "minecraft:main")];
        let unsubmitted = frontend.append_custom_post_effect(&mut gal, target, target,
            "minecraft:cancel-fixture", &sources).unwrap();
        let old_texture = frontend.custom_post_effect_intermediates["swap"].texture;
        assert!(frontend.custom_post_effect_snapshot_initialized.iter().flatten().all(|v| *v));
        drop(unsubmitted);
        frontend.discard_prepared_post_effects(&mut gal);
        assert!(frontend.custom_post_effect_resources.is_empty());
        assert!(frontend.custom_post_effect_snapshot_initialized.is_empty());
        let retry = frontend.append_custom_post_effect(&mut gal, target, target,
            "minecraft:cancel-fixture", &sources).unwrap();
        let new_texture = frontend.custom_post_effect_intermediates["swap"].texture;
        assert_ne!(old_texture, new_texture);
        assert!(retry.iter().any(|op| matches!(op, CommandOp::Barrier(barrier)
            if barrier.resource == new_texture && barrier.before == TextureUsageState::Undefined
                && barrier.after == TextureUsageState::ColorAttachment)));
        let list = gal.create_command_list(CommandListDesc {
            label: "post-effect-cancel-retry".into(), operations: retry,
        }).unwrap();
        let token = gal.submit(SubmissionBatch {
            label: "post-effect-cancel-retry".into(), command_lists: vec![list],
        }).unwrap();
        gal.retire_through(token.submission).unwrap();
        frontend.reset(&mut gal).unwrap();
        gal.destroy(target).unwrap();
        assert_eq!(gal.metrics().resource_creates, gal.metrics().resource_destroys);
    }

    #[test]
    fn custom_post_effect_resize_and_uniform_changes_keep_resources_bounded() {
        let mut gal = mock_gal();
        let initial = frame_target(&mut gal);
        gal.destroy(initial).unwrap();
        let mut frontend = GuiFrontend::default();
        let mut expected_live = None;
        for iteration in 0..32u64 {
            let width = if iteration % 2 == 0 { 320 } else { 640 };
            let height = if iteration % 2 == 0 { 180 } else { 360 };
            let target = gal.create_frame_target(FrameTargetDesc {
                label: "post-effect-resize-test".into(), frame_id: iteration + 2,
                render_target: crate::render::vulkanic::frame::FrameRenderTargetId(iteration + 2),
                extent: Extent3d { width, height, depth: 1 },
                color_format: TextureFormat::Rgba8Unorm,
            }).unwrap();
            let source = |input: &str, output: &str| CustomPostEffectSource {
                input_row_order: crate::render::vulkanic::commands::TextureRowOrder::Preserve,
                sampler_info_uniform: Some(1),
                vertex_shader: b"#version 450\nvoid main() { gl_Position = vec4(0.0); }".to_vec(),
                fragment_shader: b"#version 450\nlayout(location=0) out vec4 color; void main() { color=vec4(1.0); }".to_vec(),
                input_count: 1,
                        input_bilinear: vec![false; 1], input_targets: vec![input.into()], input_images: vec![None],
                input_use_depth: vec![false], output_target: output.into(),
                uniform_blocks: vec![vec![iteration as u8; 16], vec![0; 16]],
            };
            let ops = frontend.append_custom_post_effect(&mut gal, target, target,
                "minecraft:resize-fixture", &[source("minecraft:main", "swap"),
                    source("swap", "minecraft:main")]).unwrap();
            let expected_extents = [width as f32, height as f32, width as f32, height as f32]
                .into_iter().flat_map(f32::to_le_bytes).collect::<Vec<_>>();
            assert_eq!(2, ops.iter().filter(|op| matches!(op,
                CommandOp::HostWriteBuffer { data, .. } if data == &expected_extents)).count());
            let list = gal.create_command_list(CommandListDesc {
                label: "post-effect-resize-test".into(), operations: ops,
            }).unwrap();
            let token = gal.submit(SubmissionBatch {
                label: "post-effect-resize-test".into(), command_lists: vec![list],
            }).unwrap();
            gal.retire_through(token.submission).unwrap();
            frontend.clear_frame_passes_for_targets(&mut gal, &[target]);
            gal.destroy(target).unwrap();
            assert_eq!(2, frontend.custom_post_effect_resources.len());
            assert_eq!(1, frontend.custom_post_effect_intermediates.len());
            let live = gal.metrics().resource_creates - gal.metrics().resource_destroys;
            assert_eq!(*expected_live.get_or_insert(live), live,
                "resizing and changing uniform bytes must replace, not accumulate, resources");
        }
        frontend.reset(&mut gal).unwrap();
        gal.retire_completed().unwrap();
        assert_eq!(gal.metrics().resource_creates, gal.metrics().resource_destroys);
    }

    #[test]
    fn custom_post_effect_cache_invalidates_changed_pass_suffix() {
        let mut gal = mock_gal();
        let target = frame_target(&mut gal);
        let mut frontend = GuiFrontend::default();
        let vertex = br#"#version 330
out vec2 texCoord;
void main() { texCoord = vec2(0.0); }
"#;
        let fragment = br#"#version 330
in vec2 texCoord;
uniform sampler2D InSampler;
out vec4 fragColor;
void main() { fragColor = texture(InSampler, texCoord); }
"#;
        let source = |marker: &[u8], output: &str| CustomPostEffectSource {
            input_row_order: crate::render::vulkanic::commands::TextureRowOrder::Preserve,
            sampler_info_uniform: None,
            vertex_shader: vertex.to_vec(),
            fragment_shader: [fragment, marker].concat(),
            input_count: 1,
                        input_bilinear: vec![false; 1],
            input_targets: vec!["minecraft:main".to_owned()],
            input_images: vec![None],
            input_use_depth: vec![false],
            output_target: output.to_owned(),
            uniform_blocks: Vec::new(),
        };
        frontend
            .append_custom_post_effect(
                &mut gal,
                target,
                target,
                "minecraft:test-cache-suffix",
                &[
                    source(b"//a", "first"),
                    source(b"//b", "second"),
                    source(b"//c", "minecraft:main"),
                ],
            )
            .unwrap();
        assert_eq!(3, frontend.custom_post_effect_resources.len());
        frontend
            .append_custom_post_effect(
                &mut gal,
                target,
                target,
                "minecraft:test-cache-suffix",
                &[
                    source(b"//a", "first"),
                    source(b"//changed", "minecraft:main"),
                ],
            )
            .unwrap();
        assert_eq!(2, frontend.custom_post_effect_resources.len());
        assert_eq!(1, frontend.custom_post_effect_intermediates.len());
    }

    #[test]
    fn custom_post_effect_rejects_forward_and_feedback_target_edges() {
        let mut gal = mock_gal();
        let target = frame_target(&mut gal);
        let mut frontend = GuiFrontend::default();
        let vertex = br#"#version 330
out vec2 texCoord;
void main() { texCoord = vec2(0.0); }
"#;
        let fragment = br#"#version 330
in vec2 texCoord;
uniform sampler2D InSampler;
out vec4 fragColor;
void main() { fragColor = texture(InSampler, texCoord); }
"#;
        let forward_reference = frontend.append_custom_post_effect(
            &mut gal,
            target,
            target,
            "minecraft:test-forward-reference",
            &[
                CustomPostEffectSource {
                    input_row_order: crate::render::vulkanic::commands::TextureRowOrder::Preserve,
                    sampler_info_uniform: None,
                    vertex_shader: vertex.to_vec(),
                    fragment_shader: fragment.to_vec(),
                    input_count: 1,
                        input_bilinear: vec![false; 1],
                    input_targets: vec!["later".to_owned()],
                    input_images: vec![None],
                    input_use_depth: vec![false],
                    output_target: "first".to_owned(),
                    uniform_blocks: Vec::new(),
                },
                CustomPostEffectSource {
                    input_row_order: crate::render::vulkanic::commands::TextureRowOrder::Preserve,
                    sampler_info_uniform: None,
                    vertex_shader: vertex.to_vec(),
                    fragment_shader: fragment.to_vec(),
                    input_count: 1,
                        input_bilinear: vec![false; 1],
                    input_targets: vec!["minecraft:main".to_owned()],
                    input_images: vec![None],
                    input_use_depth: vec![false],
                    output_target: "later".to_owned(),
                    uniform_blocks: Vec::new(),
                },
            ],
        );
        assert!(forward_reference.is_err());

        let feedback = frontend.append_custom_post_effect(
            &mut gal,
            target,
            target,
            "minecraft:test-feedback-reference",
            &[
                CustomPostEffectSource {
                    input_row_order: crate::render::vulkanic::commands::TextureRowOrder::Preserve,
                    sampler_info_uniform: None,
                    vertex_shader: vertex.to_vec(),
                    fragment_shader: fragment.to_vec(),
                    input_count: 1,
                        input_bilinear: vec![false; 1],
                    input_targets: vec!["minecraft:main".to_owned()],
                    input_images: vec![None],
                    input_use_depth: vec![false],
                    output_target: "loop".to_owned(),
                    uniform_blocks: Vec::new(),
                },
                CustomPostEffectSource {
                    input_row_order: crate::render::vulkanic::commands::TextureRowOrder::Preserve,
                    sampler_info_uniform: None,
                    vertex_shader: vertex.to_vec(),
                    fragment_shader: fragment.to_vec(),
                    input_count: 1,
                        input_bilinear: vec![false; 1],
                    input_targets: vec!["loop".to_owned()],
                    input_images: vec![None],
                    input_use_depth: vec![false],
                    output_target: "loop".to_owned(),
                    uniform_blocks: Vec::new(),
                },
            ],
        );
        assert!(feedback.is_err());
    }

    #[test]
    fn custom_post_effect_rejects_oversized_uniform_payload_before_allocation() {
        let mut gal = mock_gal();
        let target = frame_target(&mut gal);
        let mut frontend = GuiFrontend::default();
        let source = CustomPostEffectSource {
            input_row_order: crate::render::vulkanic::commands::TextureRowOrder::Preserve,
            sampler_info_uniform: None,
            vertex_shader: b"#version 330\nvoid main() {}".to_vec(),
            fragment_shader: b"#version 330\nvoid main() {}".to_vec(),
            input_count: 1,
                        input_bilinear: vec![false; 1],
            input_targets: vec!["minecraft:main".to_owned()],
            input_images: vec![None],
            input_use_depth: vec![false],
            output_target: "minecraft:main".to_owned(),
            uniform_blocks: vec![vec![0; MAX_CUSTOM_POST_EFFECT_UNIFORM_BYTES + 1]],
        };
        let result = frontend.append_custom_post_effect(
            &mut gal,
            target,
            target,
            "minecraft:test-uniform-bound",
            &[source],
        );
        assert!(result.is_err());
        assert!(frontend.custom_post_effect_resources.is_empty());

        let bounded_source = || CustomPostEffectSource {
            input_row_order: crate::render::vulkanic::commands::TextureRowOrder::Preserve,
            sampler_info_uniform: None,
            vertex_shader: b"#version 330\nvoid main() {}".to_vec(),
            fragment_shader: b"#version 330\nvoid main() {}".to_vec(),
            input_count: 1,
                        input_bilinear: vec![false; 1],
            input_targets: vec!["minecraft:main".to_owned()],
            input_images: vec![None],
            input_use_depth: vec![false],
            output_target: "minecraft:main".to_owned(),
            uniform_blocks: vec![vec![0; MAX_CUSTOM_POST_EFFECT_UNIFORM_BYTES]],
        };
        let graph_result = frontend.append_custom_post_effect(
            &mut gal,
            target,
            target,
            "minecraft:test-uniform-graph-bound",
            &[bounded_source(), bounded_source(), bounded_source()],
        );
        assert!(graph_result.is_err());
        assert!(frontend.custom_post_effect_resources.is_empty());
    }

    #[test]
    fn spider_post_effect_replay_emits_blur_clip_and_blit_passes() {
        let mut gal = mock_gal();
        let target = frame_target(&mut gal);
        let mut frontend = GuiFrontend::default();
        let ops = frontend
            .append_spider_post_effect(&mut gal, target, target)
            .unwrap();
        assert!(ops
            .iter()
            .any(|op| matches!(op, CommandOp::CopyFrameTargetToTexture { .. })));
        assert_eq!(
            8,
            ops.iter()
                .filter(|op| matches!(
                    op,
                    CommandOp::Draw {
                        vertices: 3,
                        instances: 1
                    }
                ))
                .count()
        );
        gal.create_command_list(CommandListDesc {
            label: "gui-spider-replay".to_owned(),
            operations: ops,
        })
        .unwrap();
        frontend.reset(&mut gal).unwrap();
    }

    #[test]
    fn blur_boundary_replay_copies_owned_render_targets_with_explicit_attachment_barriers() {
        let mut gal = mock_gal();
        let source_texture = gal
            .create_texture(TextureDesc {
                label: "test-gui-source-texture".to_owned(),
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba8Unorm,
                extent: Extent3d {
                    width: 320,
                    height: 180,
                    depth: 1,
                },
                mip_levels: 1,
                array_layers: 1,
                usages: vec![TextureUsage::ColorAttachment, TextureUsage::TransferSrc],
            })
            .unwrap();
        let source_view = gal
            .create_texture_view(TextureViewDesc {
                label: "test-gui-source-view".to_owned(),
                texture: source_texture,
                format: TextureFormat::Rgba8Unorm,
                base_mip: 0,
                mip_count: 1,
                base_layer: 0,
                layer_count: 1,
            })
            .unwrap();
        let source_target = gal
            .create_render_target(RenderTargetDesc {
                label: "test-gui-source-target".to_owned(),
                color_views: vec![source_view],
                depth_stencil_view: None,
                extent: Extent3d {
                    width: 320,
                    height: 180,
                    depth: 1,
                },
            })
            .unwrap();
        let mut frontend = GuiFrontend::default();
        let (ops, _) = frontend
            .append_frame_ops_with_blur_boundary(
                &mut gal,
                1,
                source_target,
                source_view,
                Vec::new(),
                Vec::new(),
                Vec::new(),
                1,
                0,
                false,
            )
            .unwrap();
        assert!(ops.iter().any(|op| matches!(op, CommandOp::CopyTexture(_))));
        assert!(ops.iter().any(|op| matches!(
            op,
            CommandOp::Barrier(ResourceBarrier {
                before: TextureUsageState::ColorAttachment,
                after: TextureUsageState::TransferSrc,
                ..
            })
        )));
        gal.create_command_list(CommandListDesc {
            label: "gui-blur-owned-target".to_owned(),
            operations: ops,
        })
        .unwrap();
    }

    fn mock_gal() -> VulkanicGal {
        let mut capabilities = vulkan_capabilities();
        capabilities.features.presentation = true;
        VulkanicGal::new_with_backend(
            Box::new(MockBackend::with_capabilities(capabilities)),
            false,
        )
    }

    fn frame_target(gal: &mut VulkanicGal) -> Handle {
        gal.configure_frame_surface(FrameSurfaceDesc {
            label: "test-gui-frame-surface".to_owned(),
            extent: Extent3d {
                width: 320,
                height: 180,
                depth: 1,
            },
            color_format: TextureFormat::Rgba8Unorm,
            present_mode: PresentMode::Fifo,
            max_frames_in_flight: 2,
        })
        .unwrap();
        gal.create_frame_target(FrameTargetDesc {
            label: "test-gui-frame-target".to_owned(),
            frame_id: 1,
            render_target: crate::render::vulkanic::frame::FrameRenderTargetId(1),
            extent: Extent3d {
                width: 320,
                height: 180,
                depth: 1,
            },
            color_format: TextureFormat::Rgba8Unorm,
        })
        .unwrap()
    }

    fn depth_render_target(gal: &mut VulkanicGal) -> (Handle, Handle, Handle) {
        let extent = Extent3d {
            width: 320,
            height: 180,
            depth: 1,
        };
        let color = gal
            .create_texture(TextureDesc {
                label: "test-custom-depth-color".to_owned(),
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba8Unorm,
                extent,
                mip_levels: 1,
                array_layers: 1,
                usages: vec![
                    TextureUsage::Sampled,
                    TextureUsage::ColorAttachment,
                    TextureUsage::TransferSrc,
                ],
            })
            .unwrap();
        let color_view = gal
            .create_texture_view(TextureViewDesc {
                label: "test-custom-depth-color-view".to_owned(),
                texture: color,
                format: TextureFormat::Rgba8Unorm,
                base_mip: 0,
                mip_count: 1,
                base_layer: 0,
                layer_count: 1,
            })
            .unwrap();
        let depth = gal
            .create_texture(TextureDesc {
                label: "test-custom-depth".to_owned(),
                dimension: TextureDimension::D2,
                format: TextureFormat::Depth32Float,
                extent,
                mip_levels: 1,
                array_layers: 1,
                usages: vec![TextureUsage::Sampled, TextureUsage::DepthStencilAttachment, TextureUsage::TransferSrc],
            })
            .unwrap();
        let depth_view = gal
            .create_texture_view(TextureViewDesc {
                label: "test-custom-depth-view".to_owned(),
                texture: depth,
                format: TextureFormat::Depth32Float,
                base_mip: 0,
                mip_count: 1,
                base_layer: 0,
                layer_count: 1,
            })
            .unwrap();
        let target = gal
            .create_render_target(RenderTargetDesc {
                label: "test-custom-depth-target".to_owned(),
                color_views: vec![color_view],
                depth_stencil_view: Some(depth_view),
                extent,
            })
            .unwrap();
        (target, color_view, depth)
    }

    fn request(sprite_id: u32) -> GuiSpriteRequest {
        let def = sprite_def(sprite_id).unwrap();
        GuiSpriteRequest {
            stratum: def.stratum,
            sprite_id,
            selected_slot: -1,
            progress_fraction: 1.0,
            fill_direction: 0,
            color_argb: 0xffffffff,
            x: 10,
            y: 10,
            width: def.width.min(16),
            height: def.height.min(16),
            gui_width: 320,
            gui_height: 180,
            projection_extent: [320.0, 180.0],
            sequence: 1,
        }
    }

    #[test]
    fn native_model_item_uses_registered_owned_image_without_borrowing_an_atlas() {
        let mut gal = mock_gal();
        let target = frame_target(&mut gal);
        let mut frontend = GuiFrontend::default();
        let mut request = mesh_batch(0);
        request.item_raster_scale = 2;
        request.render_extent = [0,0];
        request.guard_pixels = 0;
        request.model_transform = [0.8,0.,-0.6,0., 0.,1.,0.,0., 0.6,0.,0.8,0., 0.,0.,0.,1.];
        request.lighting_mode = GuiMeshLightingMode::Flat;
        request.item_lighting = Some(super::super::gui_mesh_frontend::GuiFlatItemLighting {
            lightmap_generation: 1, rgb: [1.;3],
        });
        let before = gal.metrics().resource_creates;
        assert!(frontend.append_mesh_items_to_target(&mut gal, None, 1, SubmissionId(1),
            target, target, None, None, None, vec![request.clone()], &mut GuiSubmitStats::default()).is_err());
        assert_eq!(before, gal.metrics().resource_creates);
        frontend.apply_raw_image_update(&mut gal, 1, vec![GuiRawImageAssetPayload { sampling: None,
            asset_id: 7, format: GuiRawImageFormat::Rgba8, width: 2, height: 2, pixels: vec![255;16],
        }]).unwrap();
        let mut stats = GuiSubmitStats::default();
        let ops = frontend.append_mesh_items_to_target(&mut gal, None, 1, SubmissionId(1),
            target, target, None, None, None, vec![request.clone()], &mut stats).unwrap();
        assert!(!ops.is_empty());
        assert_eq!(stats.mesh_item_count, 1);
        assert!(frontend.resources.values().all(|binding| !matches!(binding.image_ownership, GuiImageOwnership::AtlasView)));
        request.asset_id = 8;
        assert!(frontend.preflight_mesh_atlas_commands(None, &[request]).is_err());
        frontend.reset(&mut gal).unwrap();
        gal.destroy(target).unwrap();
        // Creating the owned image submitted an upload. Destruction must
        // remain deferred until that actual submission has completed.
        let completed = gal.latest_submission_id();
        gal.mock_backend_mut().unwrap().complete_through(completed);
        gal.retire_through(completed).unwrap();
        assert_eq!(gal.metrics().resource_creates, gal.metrics().resource_destroys);
    }

    #[test]
    fn model_overlay_passes_finish_before_foil_without_changing_authored_layers() {
        let requests: Vec<_> = (0..6).map(|i| {
            let mut request=mesh_batch(i);
            request.item_raster_scale=3; request.render_extent=[0,0];request.guard_pixels=0;
            request.model_transform=[1.,0.,0.,0.,0.,1.,0.,0.,0.,0.,1.,0.,0.,0.,0.,1.];
            request.material_mode=if i==2 {GuiMeshMaterialMode::Glint}
                else if i>=3 {GuiMeshMaterialMode::ModelOverlay} else {GuiMeshMaterialMode::Opaque};
            request.lighting_mode=if i==2 {GuiMeshLightingMode::Flat} else {GuiMeshLightingMode::FrontModel};
            request.alpha_cutoff=if i==2 {0.1} else {0.0};
            if i==2 {
                request.item_foil=Some(super::super::item_foil::StandardItemFoil {
                    kind:super::super::item_foil::StandardFoilKind::Entity,clock_millis:0,speed:0.,strength:0.5 });
            } else {
                request.item_lighting=Some(super::super::gui_mesh_frontend::GuiFlatItemLighting {
                    lightmap_generation:1,rgb:[1.;3] });
            }
            request
        }).collect();
        let mut draws=prepare_gui_mesh_draws(&requests).unwrap();
        validate_mesh_item_layers(&draws).unwrap();
        assert_eq!(mesh_item_layer_execution_order(&draws),vec![0,1,3,4,5,2]);
        assert_eq!(draws.iter().map(|draw|draw.layer_index).collect::<Vec<_>>(),vec![0,1,2,3,4,5]);
        // This dependency must not reorder unrelated authored item layers.
        for draw in &mut draws { if draw.material_mode==GuiMeshMaterialMode::ModelOverlay {
            draw.material_mode=GuiMeshMaterialMode::Translucent;
        }}
        assert_eq!(mesh_item_layer_execution_order(&draws),vec![0,1,2,3,4,5]);
    }

    fn mesh_batch(layer_index: u32) -> GuiMeshBatchRequest {
        GuiMeshBatchRequest {
            item_cache: None,
            block_item_raster: None,
            item_raster_scale: 0,
            item_lighting: None,
            item_foil: None,
            decal_foil: None,
            stratum: 420,
            layer_index,
            sequence: 9,
            asset_id: 7,
            material_mode: GuiMeshMaterialMode::Cutout,
            lighting_mode: GuiMeshLightingMode::Block,
            alpha_cutoff: 0.5,
            model_transform: [
                16.0, 0.0, 0.0, 0.0, 0.0, -16.0, 0.0, 0.0, 0.0, 0.0, 16.0, 0.0, 17.0, 17.0, 0.0,
                1.0,
            ],
            gui_pose: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            bounds: [12, 34, 28, 50],
            gui_extent: [320, 180],
            projection_extent: [320.0, 180.0],
            render_extent: [34, 34],
            guard_pixels: 1,
            clip_mode: 0,
            clip_left: 0,
            clip_top: 0,
            clip_width: 0,
            clip_height: 0,
            vertices: vec![
                GuiMeshVertex {
                    position: [0.0, 0.0, 0.0],
                    atlas_uv: [0.0, 0.0],
                    local_uv: [0.0, 0.0],
                    color_argb: 0xffff_ffff,
                    normal_packed: 0x007f_0000, source_face: 0, source_foil_type: 0,
                },
                GuiMeshVertex {
                    position: [1.0, 0.0, 0.0],
                    atlas_uv: [1.0, 0.0],
                    local_uv: [1.0, 0.0],
                    color_argb: 0xffff_ffff,
                    normal_packed: 0x007f_0000, source_face: 0, source_foil_type: 0,
                },
                GuiMeshVertex {
                    position: [0.0, 1.0, 0.0],
                    atlas_uv: [0.0, 1.0],
                    local_uv: [0.0, 1.0],
                    color_argb: 0xffff_ffff,
                    normal_packed: 0x007f_0000, source_face: 0, source_foil_type: 0,
                },
            ],
            indices: vec![0, 1, 2],
        }
    }

    #[test]
    fn mesh_geometry_reuses_only_ranges_released_by_completed_submission() {
        let batches = vec![mesh_batch(0)];
        let prepared = prepare_gui_mesh_draws(&batches)
            .expect("prepare mesh fixture")
            .pop()
            .expect("one prepared mesh draw");
        let key = gui_mesh_raster_key(&prepared);
        let mut frontend = GuiFrontend::default();
        let mut gal = mock_gal();
        let first = frontend
            .allocate_mesh_geometry(&mut gal, key, 1_024, 256, SubmissionId(3))
            .expect("first private stream range");
        frontend.mesh_geometry_cache.insert((key, 1, 0), first);
        let second = frontend
            .allocate_mesh_geometry(&mut gal, key, 1_024, 256, SubmissionId(4))
            .expect("second private stream range");
        frontend.mesh_geometry_cache.insert((key, 2, 0), second);

        frontend.reclaim_completed_mesh_geometry(SubmissionId(2));
        let while_in_flight = frontend
            .allocate_mesh_geometry(&mut gal, key, 1_024, 256, SubmissionId(5))
            .expect("a distinct range while earlier work is in flight");
        assert_eq!(2_048, while_in_flight.stream.vertex_offset);
        assert_eq!(512, while_in_flight.stream.index_offset);

        frontend.reclaim_completed_mesh_geometry(SubmissionId(3));
        let reused_after_completion = frontend
            .allocate_mesh_geometry(&mut gal, key, 1_024, 256, SubmissionId(6))
            .expect("a completed range is reusable");
        assert_eq!(0, reused_after_completion.stream.vertex_offset);
        assert_eq!(0, reused_after_completion.stream.index_offset);
    }

    #[test]
    fn mesh_geometry_applies_bounded_backpressure_before_stream_exhaustion() {
        let batches = vec![mesh_batch(0)];
        let prepared = prepare_gui_mesh_draws(&batches)
            .expect("prepare mesh fixture")
            .pop()
            .expect("one prepared mesh draw");
        let key = gui_mesh_raster_key(&prepared);
        let mut frontend = GuiFrontend::default();
        let mut gal = mock_gal();
        let reserved = gal.next_submission_id();
        let full = frontend
            .allocate_mesh_geometry(
                &mut gal,
                key,
                crate::render::vulkanic::gui_mesh_frontend::GUI_MESH_MAX_VERTEX_BYTES,
                crate::render::vulkanic::gui_mesh_frontend::GUI_MESH_MAX_INDEX_BYTES,
                reserved,
            )
            .expect("the fixed stream admits one full allocation");
        frontend.mesh_geometry_cache.insert((key, 1, 0), full);
        let accepted = gal.submit(SubmissionBatch {
            label: "accepted-stream-reservation".into(),
            command_lists: vec![CommandList::from(CommandListDesc {
                label: "allocator-completion-fixture".into(), operations: vec![],
            })],
        }).unwrap();
        assert_eq!(reserved, accepted.submission);
        let next = gal.next_submission_id();

        let after_wait = frontend
            .allocate_mesh_geometry(&mut gal, key, 48, 4, next)
            .expect("the allocator retires the oldest range instead of growing or failing");
        assert_eq!(0, after_wait.stream.vertex_offset);
        assert_eq!(0, after_wait.stream.index_offset);
    }

    #[test]
    fn mesh_geometry_never_retires_an_unsubmitted_frame_reservation() {
        let batches = vec![mesh_batch(0)];
        let prepared = prepare_gui_mesh_draws(&batches)
            .expect("prepare mesh fixture")
            .pop()
            .expect("one prepared mesh draw");
        let key = gui_mesh_raster_key(&prepared);
        let mut frontend = GuiFrontend::default();
        let mut gal = mock_gal();
        let reservation = frontend
            .allocate_mesh_geometry(
                &mut gal,
                key,
                crate::render::vulkanic::gui_mesh_frontend::GUI_MESH_MAX_VERTEX_BYTES,
                crate::render::vulkanic::gui_mesh_frontend::GUI_MESH_MAX_INDEX_BYTES,
                SubmissionId(3),
            )
            .expect("the current frame can reserve the stream");
        frontend.mesh_geometry_cache.insert((key, 1, 0), reservation);

        let error = frontend
            .allocate_mesh_geometry(&mut gal, key, 48, 4, SubmissionId(3))
            .expect_err("a second current-frame allocation cannot retire future work");
        assert_eq!(StatusCode::InvalidArgument, error.code);
        assert_eq!(
            SubmissionId(0),
            gal.poll_completed(),
            "a failed current-frame reservation must not advance completion"
        );
    }

    #[test]
    fn rotating_panorama_meshes_stay_within_the_bounded_rust_stream() {
        let mut gal = mock_gal();
        let target = frame_target(&mut gal);
        let mut frontend = GuiFrontend::default();
        frontend
            .apply_raw_image_update(
                &mut gal,
                1,
                vec![GuiRawImageAssetPayload { sampling: None,
                    asset_id: 7,
                    format: GuiRawImageFormat::Rgba8,
                    width: 1,
                    height: 6,
                    pixels: vec![255; 24],
                }],
            )
            .expect("stage the Rust-owned stacked cube-map image");

        // The semantic panorama is Frozen's oversized fullscreen triangle. Its
        // three camera rays
        // change with the camera, while Rust resolves the cube face per pixel.
        for frame in 1..=10u64 {
            let mut panorama = mesh_batch(0);
            panorama.sequence = frame;
            panorama.material_mode = GuiMeshMaterialMode::Panorama;
            panorama.lighting_mode = GuiMeshLightingMode::Flat;
            panorama.alpha_cutoff = 0.0;
            panorama.model_transform = [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ];
            panorama.vertices = vec![
                GuiMeshVertex {
                    position: [0.0, 1.0, frame as f32 / 10.0],
                    atlas_uv: [0.0, 0.0],
                    local_uv: [-1.0, -1.0],
                    color_argb: 0xffff_ffff,
                    normal_packed: 0x007f_0000, source_face: 0, source_foil_type: 0,
                },
                GuiMeshVertex {
                    position: [2.0, 1.0, frame as f32 / 10.0],
                    atlas_uv: [0.0, 0.0],
                    local_uv: [3.0, -1.0],
                    color_argb: 0xffff_ffff,
                    normal_packed: 0x007f_0000, source_face: 0, source_foil_type: 0,
                },
                GuiMeshVertex {
                    position: [0.0, -1.0, frame as f32 / 10.0],
                    atlas_uv: [0.0, 0.0],
                    local_uv: [-1.0, 3.0],
                    color_argb: 0xffff_ffff,
                    normal_packed: 0x007f_0000, source_face: 0, source_foil_type: 0,
                },
            ];
            panorama.indices = vec![0, 1, 2];
            let stats = frontend
                .submit_frame_with_affine_quads_and_mesh_batches(
                    &mut gal,
                    1,
                    target,
                    Vec::new(),
                    Vec::new(),
                    vec![panorama],
                )
                .expect("animated panorama frame must not exhaust persistent geometry");
            assert_eq!(1, stats.mesh_batch_count);
            assert_eq!(
                1,
                stats.mesh_draw_count,
                "Frozen's semantic panorama is a direct native-resolution frame draw, not a PIP raster plus composite"
            );
            assert!(
                stats.owned_intermediate_targets.is_empty(),
                "a panorama must not allocate a logical-GUI-sized intermediate before frame presentation"
            );
        }
        assert!(
            gal.metrics().submissions >= 10,
            "every rotating semantic panorama frame must be submitted by Rust"
        );
    }

    fn mesh_item_batches(sequence: u64) -> Vec<GuiMeshBatchRequest> {
        let mut layers = vec![mesh_batch(0), mesh_batch(1), mesh_batch(2)];
        for layer in &mut layers {
            layer.sequence = sequence;
        }
        layers
    }

    #[test]
    fn mesh_raster_cache_never_reuses_one_uniform_buffer_for_different_pip_inputs() {
        let first = prepare_gui_mesh_draws(&[mesh_batch(0)])
            .expect("prepare first GUI mesh draw")
            .pop()
            .unwrap();
        let mut changed_extent_batch = mesh_batch(0);
        changed_extent_batch.render_extent = [66, 34];
        let changed_extent = prepare_gui_mesh_draws(&[changed_extent_batch])
            .expect("prepare changed-extent GUI mesh draw")
            .pop()
            .unwrap();
        let mut changed_lighting_batch = mesh_batch(0);
        changed_lighting_batch.lighting_mode = GuiMeshLightingMode::Flat;
        let changed_lighting = prepare_gui_mesh_draws(&[changed_lighting_batch])
            .expect("prepare changed-lighting GUI mesh draw")
            .pop()
            .unwrap();

        assert_ne!(
            gui_mesh_raster_key(&first),
            gui_mesh_raster_key(&changed_extent)
        );
        assert_ne!(
            gui_mesh_raster_key(&first),
            gui_mesh_raster_key(&changed_lighting)
        );
        assert_eq!(gui_mesh_raster_key(&first), gui_mesh_raster_key(&first));
    }

    #[test]
    fn inventory_mesh_atlas_uses_explicit_owner_and_native_sprite_uv_mapping() {
        for (block_lit, overlay) in [(false, false), (true, false), (false, true)] {
        use super::super::world_primitive_frontend::{WorldPrimitiveFrontend, WorldMeshTextureAssetPayload, WORLD_MATERIAL_TEXTURE_STONE};
        let mut gal = mock_gal();
        let target = frame_target(&mut gal);
        let mut world = WorldPrimitiveFrontend::default();
        let mut png_bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png_bytes, 2, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.write_header().unwrap().write_image_data(&[255,0,0,255, 0,255,0,255]).unwrap();
        }
        world.apply_world_mesh_asset_update(&mut gal, 1, Vec::new(), vec![WorldMeshTextureAssetPayload {
            texture_id: WORLD_MATERIAL_TEXTURE_STONE, png_bytes, mip_png_bytes: Vec::new(),
            frame_width: 0, frame_height: 0, frame_count: 1, frame_ticks: 1, animation_flags: 0,
            frame_row_size: 0, interpolation_policy: 0, animation_frames: Vec::new(),
            coordinate_origin: 0, sampling: None, requested_mip_levels: 1,
        }]).unwrap();
        let mut frontend = GuiFrontend::default();
        let reference = GuiAtlasReference { asset_id: 7,
            atlas: world.accepted_gui_atlas_incarnation(WORLD_MATERIAL_TEXTURE_STONE).unwrap(),
            x: 1, y: 0, width: 1, height: 1 };
        frontend.stage_owned_atlas_references(&mut gal, &world, 1, &[reference]).unwrap();
        let mut request = mesh_batch(0);
        request.item_raster_scale = 2;
        request.render_extent = [0, 0]; request.guard_pixels = 0;
        request.model_transform = super::super::gui_item_raster::GuiItemModelTransform::default().0;
        request.lighting_mode = GuiMeshLightingMode::Flat;
        request.item_lighting = Some(super::super::gui_mesh_frontend::GuiFlatItemLighting {
            lightmap_generation: 1, rgb: [1.0; 3],
        });
        if block_lit {
            request.item_raster_scale = 0;
            request.render_extent = [34,34];
            request.guard_pixels = 1;
            request.lighting_mode = GuiMeshLightingMode::InventoryBlock;
        }
        if overlay {
            request.material_mode = GuiMeshMaterialMode::ModelOverlay;
            request.lighting_mode = GuiMeshLightingMode::FrontModel;
            request.alpha_cutoff = 0.0;
            let mut wrong_lighting = request.clone();
            wrong_lighting.lighting_mode = GuiMeshLightingMode::Flat;
            assert!(frontend.preflight_mesh_atlas_commands(Some(&world), &[wrong_lighting]).is_err());
            let mut wrong_cutout = request.clone();
            wrong_cutout.alpha_cutoff = 0.1;
            assert!(frontend.preflight_mesh_atlas_commands(Some(&world), &[wrong_cutout]).is_err());
        }
        let before = gal.metrics().resource_creates;
        let mut invalid = request.clone(); invalid.asset_id = 999;
        if !block_lit { assert!(frontend.preflight_mesh_atlas_commands(Some(&world), &[invalid]).is_err()); }
        let mut unsupported = request.clone();
        unsupported.item_raster_scale = 0;
        unsupported.lighting_mode = GuiMeshLightingMode::Block;
        assert!(frontend.preflight_mesh_atlas_commands(Some(&world), &[unsupported]).is_err());
        let mut foil = request.clone(); foil.material_mode = GuiMeshMaterialMode::Glint;
        assert!(frontend.preflight_mesh_atlas_commands(Some(&world), &[foil]).is_err());
        assert!(frontend.append_frame_ops_with_owned_atlases_and_blur_boundary(&mut gal, None,
            1, target, target, Vec::new(), Vec::new(), vec![request.clone()], Vec::new(),
            400, 2, false).is_err());
        assert_eq!(before, gal.metrics().resource_creates);
        assert!(frontend.append_mesh_items_to_target(&mut gal, None, 1, SubmissionId(2),
            target, target, None, None, None, vec![request.clone()], &mut GuiSubmitStats::default()).is_err());
        assert_eq!(before, gal.metrics().resource_creates);
        let mut stats = GuiSubmitStats::default();
        let ops = frontend.append_mesh_items_to_target(&mut gal, Some(&mut world), 1, SubmissionId(2),
            target, target, None, None, None, vec![request], &mut stats).unwrap();
        assert_eq!(stats.mesh_item_count, 1);
        assert!(frontend.raw_images.is_empty());
        assert!(frontend.dynamic_textures.is_empty());
        assert!(frontend.resources.values().all(|binding| matches!(binding.image_ownership, GuiImageOwnership::AtlasView)));
        let writes: Vec<_> = ops.iter().filter_map(|op| match op {
            CommandOp::HostWriteBuffer { data, .. } if data.len() == 3 * 48 => Some(data),
            _ => None,
        }).collect();
        assert_eq!(writes.len(), 1);
        let values: Vec<_> = writes[0].chunks_exact(4).map(|v| f32::from_le_bytes(v.try_into().unwrap())).collect();
        assert_eq!(values[3], 0.5, "local U=0 resolves to the right sprite's start");
        assert_eq!(values[15], 1.0, "local U=1 resolves to the right sprite's end");
        frontend.invalidate_atlas_texture_views(&mut gal, [WORLD_MATERIAL_TEXTURE_STONE]).unwrap();
        assert!(frontend.atlas_views.is_empty());
        assert!(frontend.mesh_rasters.is_empty());
        frontend.reset(&mut gal).unwrap();
        }
    }

    #[test]
    fn vulkan_item_cache_retains_pixels_moves_composition_and_invalidates_identity_and_reload() {
        use crate::render::vulkanic::gui_mesh_frontend::GuiItemCache;
        use crate::render::vulkanic::gui_item_raster::GuiItemRasterTarget;
        for animated in [false, true] {
            let backend = VulkanBackend::new("GUI item cache pixel lifetime").unwrap();
            let mut gal = VulkanicGal::new_with_backend(Box::new(backend), false);
            let mut frontend = GuiFrontend::default();
            let extent = Extent3d { width: 32, height: 16, depth: 1 };
            let target = GuiItemRasterTarget::create(&mut gal, extent).unwrap();
            let readback = gal.create_buffer(BufferDesc {
                label: "item-cache.readback".into(), size: 32 * 16 * 4,
                memory: MemoryDomain::Readback,
                usages: vec![BufferUsage::TransferDst, BufferUsage::HostRead],
            }).unwrap();
            let mut first_pixels = Vec::new();
            let mut second_pixels = Vec::new();
            for step in 0..=4 {
                let generation = if step == 4 { 2 } else { 1 };
                if step == 0 || step == 4 {
                    frontend.apply_raw_image_update(&mut gal, generation, vec![GuiRawImageAssetPayload {
                        sampling: None, asset_id: 7, format: GuiRawImageFormat::Rgba8,
                        width: 1, height: 1,
                        pixels: if step == 0 { vec![255,0,0,255] } else { vec![0,0,255,255] },
                    }]).unwrap();
                }
                if step == 1 {
                    // Simulate an in-place owned-atlas animation upload. This
                    // must not invalidate a static item's already rasterized pixels.
                    let source = frontend.dynamic_textures[&(7, GuiRawImageFormat::Rgba8)];
                    gal.submit(SubmissionBatch { label: "source mutation".into(),
                        command_lists: vec![CommandList::from(CommandListDesc {
                            label: "owned texture mutation".into(), operations: vec![
                                CommandOp::Barrier(buffer_barrier(source.upload_buffer,
                                    TextureUsageState::TransferSrc, TextureUsageState::TransferDst)),
                                CommandOp::HostWriteBuffer { buffer: source.upload_buffer, offset: 0,
                                    data: vec![0,255,0,255] },
                                CommandOp::Barrier(buffer_barrier(source.upload_buffer,
                                    TextureUsageState::TransferDst, TextureUsageState::TransferSrc)),
                                CommandOp::Barrier(texture_barrier(source.texture,
                                    TextureUsageState::ShaderRead, TextureUsageState::TransferDst)),
                                CommandOp::CopyBufferToTexture(BufferImageCopyRegion {
                                    buffer: source.upload_buffer, buffer_offset: 0, bytes_per_row: 4,
                                    rows_per_image: 1, texture: source.texture, texture_mip: 0,
                                    texture_layer: 0, texture_origin: TextureOrigin3d {x:0,y:0,z:0},
                                    extent: Extent3d {width:1,height:1,depth:1},
                                }),
                                CommandOp::Barrier(texture_barrier(source.texture,
                                    TextureUsageState::TransferDst, TextureUsageState::ShaderRead)),
                            ],
                        })],
                    }).unwrap();
                }
                let mut request = mesh_batch(0);
                request.item_cache = Some(GuiItemCache { identity: if step == 3 { 2 } else { 1 }, animated });
                request.item_raster_scale = 1;
                request.render_extent = [0,0];
                request.guard_pixels = 0;
                request.lighting_mode = GuiMeshLightingMode::FrontModel;
                request.item_lighting = Some(crate::render::vulkanic::gui_item_material::GuiFlatItemLighting {
                    lightmap_generation: 1, rgb: [1.0;3],
                });
                request.model_transform = [1.,0.,0.,0., 0.,1.,0.,0., 0.,0.,1.,0., 0.,0.,0.,1.];
                for (vertex, position) in request.vertices.iter_mut().zip([
                    [-0.5,-0.5,0.], [0.5,-0.5,0.], [-0.5,0.5,0.],
                ]) { vertex.position = position; }
                request.bounds = if step == 2 { [16,0,32,16] } else { [0,0,16,16] };
                request.gui_extent = [32,16];
                request.projection_extent = [32.,16.];
                let (draws, stats) = frontend.append_frame_ops_with_affine_quads_and_mesh_batches_to_target(
                    &mut gal, generation, target.target, target.view, Some(target.pass),
                    None, None, false, vec![], vec![], vec![request]).unwrap();
                let mut ops = vec![
                    CommandOp::Barrier(texture_barrier(target.color,
                        if step == 0 { TextureUsageState::Undefined } else { TextureUsageState::TransferSrc },
                        TextureUsageState::ColorAttachment)),
                    CommandOp::BeginPass {pass:target.pass,target:target.target,
                        colors:vec![PassAttachment {view:target.view,load_op:AttachmentLoadOp::Clear,
                            store_op:AttachmentStoreOp::Store,
                            clear_color:Some(ClearColor {r:0.,g:0.,b:0.,a:1.})}],depth_stencil:None},
                    CommandOp::EndPass,
                ];
                ops.extend(draws);
                ops.extend([
                    CommandOp::Barrier(texture_barrier(target.color,
                        TextureUsageState::ColorAttachment, TextureUsageState::TransferSrc)),
                    CommandOp::CopyTextureToBuffer(BufferImageCopyRegion {
                        buffer:readback,buffer_offset:0,bytes_per_row:32*4,rows_per_image:16,
                        texture:target.color,texture_mip:0,texture_layer:0,
                        texture_origin:TextureOrigin3d{x:0,y:0,z:0},extent,
                    }),
                    CommandOp::Barrier(buffer_barrier(readback,
                        TextureUsageState::TransferDst,TextureUsageState::ShaderRead)),
                    CommandOp::HostReadBuffer{buffer:readback,offset:0,size:32*16*4},
                ]);
                let token = gal.submit(SubmissionBatch {label:"cached item frame".into(),
                    command_lists:vec![CommandList::from(CommandListDesc {label:"cached item commands".into(),operations:ops})],
                }).unwrap();
                gal.retire_through_for_test(token.submission).unwrap();
                let pixels = gal.completed_host_reads().iter().rev()
                    .find(|read|read.buffer==readback).unwrap().bytes.clone();
                let channel = if step == 4 { 2 } else if step == 3 || animated && step > 0 { 1 } else { 0 };
                assert!(pixels.chunks_exact(4).filter(|p|p[channel]>32).count()>40,
                    "step {step} animated={animated}: actual raster must contain the expected colored geometry");
                assert!(pixels.chunks_exact(4).all(|p|(0..3).all(|c|c==channel || p[c]==0)));
                assert_eq!(stats.mesh_batch_count, u64::from(animated || step==0 || step>=3),
                    "static pixels must be reused; animated/identity/reload paths must redraw");
                if step == 0 { first_pixels = pixels.clone(); }
                if step == 1 {
                    if !animated { assert_eq!(pixels,first_pixels); }
                    second_pixels = pixels.clone();
                }
                if step == 2 {
                    for y in 0..16 { for x in 0..16 {
                        assert_eq!(&pixels[(y*32+x+16)*4..][..4],&second_pixels[(y*32+x)*4..][..4]);
                        assert_eq!(&pixels[(y*32+x)*4..][..3], &[0,0,0]);
                    }}
                }
            }
            frontend.reset(&mut gal).unwrap();
            target.destroy(&mut gal).unwrap();
            gal.destroy(readback).unwrap();
            gal.retire_through(gal.latest_submission_id()).unwrap();
            assert_eq!(gal.metrics().resource_creates,gal.metrics().resource_destroys);
        }
    }

    #[test]
    fn discarded_mesh_preparation_does_not_publish_attachment_layout() {
        let mut gal = mock_gal();
        let target = frame_target(&mut gal);
        let mut frontend = GuiFrontend::default();
        frontend.apply_raw_image_update(&mut gal, 1, vec![GuiRawImageAssetPayload {
            sampling: None, asset_id: 7, format: GuiRawImageFormat::Rgba8,
            width: 1, height: 1, pixels: vec![255; 4],
        }]).unwrap();
        let prepare = |frontend: &mut GuiFrontend, gal: &mut VulkanicGal| {
            frontend.append_frame_ops_with_affine_quads_and_mesh_batches_to_target(
                gal, 1, target, target, None, None, None, false,
                Vec::new(), Vec::new(), vec![mesh_batch(0)]).unwrap().0
        };
        let discarded = prepare(&mut frontend, &mut gal);
        let depth = discarded.iter().find_map(|op| match op {
            CommandOp::Barrier(barrier) if barrier.after == TextureUsageState::DepthStencilAttachment
                => Some(barrier.resource),
            _ => None,
        }).expect("private raster depth attachment");
        drop(discarded);
        let unsubmitted_retry = prepare(&mut frontend, &mut gal);
        assert!(unsubmitted_retry.iter().any(|op| matches!(op,
            CommandOp::HostWriteBuffer { data, .. } if data.len() == 3 * 48)),
            "a discarded preparation cannot satisfy the retry's vertex upload");
        assert!(unsubmitted_retry.iter().any(|op| matches!(op,
            CommandOp::HostWriteBuffer { data, .. } if data.len() == 3 * 4)),
            "a discarded preparation cannot satisfy the retry's index upload");
        drop(unsubmitted_retry);
        // Even consuming its predicted ID elsewhere proves nothing about
        // this target's texture layouts.
        gal.submit(SubmissionBatch {
            label: "unrelated accepted submission".into(),
            command_lists: vec![CommandList::from(CommandListDesc {
                label: "empty unrelated list".into(), operations: vec![],
            })],
        }).unwrap();
        let retry = prepare(&mut frontend, &mut gal);
        assert!(retry.iter().any(|op| matches!(op,
            CommandOp::Barrier(barrier) if barrier.resource == depth
                && barrier.before == TextureUsageState::Undefined
                && barrier.after == TextureUsageState::DepthStencilAttachment)));
        frontend.reset(&mut gal).unwrap();
    }

    #[test]
    fn owned_mesh_items_rasterize_layers_then_compose_once_into_the_gui_target() {
        let mut gal = mock_gal();
        let target = frame_target(&mut gal);
        let mut frontend = GuiFrontend::default();
        frontend
            .apply_raw_image_update(
                &mut gal,
                1,
                vec![GuiRawImageAssetPayload { sampling: None,
                    asset_id: 7,
                    format: GuiRawImageFormat::Rgba8,
                    width: 1,
                    height: 1,
                    pixels: vec![255, 255, 255, 255],
                }],
            )
            .expect("stage owned mesh image");
        let affine = |sequence, x| GuiAffineQuadRequest {
            item_raster_layers: vec![],
            item_raster_scale: 0,
            item_raster_geometry: Default::default(),
            material: crate::render::vulkanic::gui_item_material::GuiAffineMaterial::Unlit,
            stratum: 420,
            asset_id: 7,
            x0: x,
            y0: 4.0,
            x1: x + 8.0,
            y1: 4.0,
            x3: x,
            y3: 12.0,
            z: 0.0,
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
            color_argb: 0xffff_ffff,
            gui_width: 320,
            gui_height: 180,
            projection_extent: [320.0, 180.0],
            sequence,
            clip_mode: 0,
            clip_left: 0,
            clip_top: 0,
            clip_width: 0,
            clip_height: 0,
        };
        let (ops, stats) = frontend
            .append_frame_ops_with_affine_quads_and_mesh_batches_to_target(
                &mut gal,
                1,
                target,
                target,
                None,
                None,
                None,
                false,
                Vec::new(),
                vec![affine(8, 2.0), affine(10, 20.0)],
                vec![mesh_batch(0), mesh_batch(1), mesh_batch(2)],
            )
            .expect("append ordered GUI item mesh layers");
        assert_eq!(1, stats.mesh_item_count);
        assert_eq!(3, stats.mesh_batch_count);
        assert_eq!(4, stats.mesh_draw_count);
        assert_eq!(
            1,
            stats.owned_intermediate_targets.len(),
            "the GUI frontend must explicitly report its one Rust-owned raster target"
        );
        assert_ne!(target, stats.owned_intermediate_targets[0]);
        assert_eq!(
            3,
            ops.iter()
                .filter(|op| matches!(
                    op,
                    CommandOp::DrawIndexed {
                        indices: 3,
                        instances: 1
                    }
                ))
                .count()
        );
        let vertex_writes = ops
            .iter()
            .filter_map(|op| match op {
                CommandOp::HostWriteBuffer { offset, data, .. } if data.len() == 3 * 48 => {
                    Some((*offset, data.clone()))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            vec![0],
            vertex_writes
                .iter()
                .map(|(offset, _)| *offset)
                .collect::<Vec<_>>(),
            "identical same-asset geometry should be uploaded once and reused"
        );
        let index_writes = ops
            .iter()
            .filter_map(|op| match op {
                CommandOp::HostWriteBuffer { offset, data, .. } if data.len() == 3 * 4 => {
                    Some((*offset, data.clone()))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            vec![0],
            index_writes
                .iter()
                .map(|(offset, _)| *offset)
                .collect::<Vec<_>>()
        );
        let mesh_draw = ops
            .iter()
            .position(|op| {
                matches!(
                    op,
                    CommandOp::DrawIndexed {
                        indices: 3,
                        instances: 1
                    }
                )
            })
            .expect("mesh raster draw");
        let composite_draw = ops
            .iter()
            .position(|op| {
                matches!(
                    op,
                    CommandOp::Draw {
                        vertices: 6,
                        instances: 1
                    }
                )
            })
            .expect("mesh composite draw");
        let affine_draws = ops
            .iter()
            .enumerate()
            .filter_map(|(index, op)| {
                matches!(
                    op,
                    CommandOp::DrawIndexed {
                        indices: 6,
                        instances: 1
                    }
                )
                .then_some(index)
            })
            .collect::<Vec<_>>();
        assert_eq!(2, affine_draws.len());
        assert!(
            affine_draws[0] < mesh_draw
                && mesh_draw < composite_draw
                && composite_draw < affine_draws[1]
        );
        assert_eq!(1, frontend.mesh_targets.len());
        assert_eq!(1, frontend.mesh_composites.len());
        assert_eq!(1, frontend.mesh_rasters.len());
        assert_eq!(
            1,
            frontend.mesh_shared_programs.len(),
            "the asset-local mesh raster must borrow one Rust-owned immutable program"
        );
        gal.submit(SubmissionBatch {
            label: "gui-mesh-frontend-test".to_string(),
            command_lists: vec![CommandList::from(CommandListDesc {
                label: "gui-mesh-frontend-test-commands".to_string(),
                operations: ops,
            })],
        })
        .expect("GAL validates ordered mesh raster and composite passes");
        frontend.reset(&mut gal).unwrap();
    }

    #[test]
    fn mesh_items_reusing_an_extent_transition_the_raster_target_back_to_attachment_usage() {
        let mut gal = mock_gal();
        let mut frontend = GuiFrontend::default();
        let target = frame_target(&mut gal);
        frontend
            .apply_raw_image_update(
                &mut gal,
                1,
                vec![
                    GuiRawImageAssetPayload { sampling: None,
                        asset_id: 7,
                        format: GuiRawImageFormat::Rgba8,
                        width: 1,
                        height: 1,
                        pixels: vec![255, 255, 255, 255],
                    },
                    GuiRawImageAssetPayload { sampling: None,
                        asset_id: 8,
                        format: GuiRawImageFormat::Rgba8,
                        width: 1,
                        height: 1,
                        pixels: vec![255, 255, 255, 255],
                    },
                ],
            )
            .expect("raw images");
        let mut requests = mesh_item_batches(9);
        let mut next_item = mesh_item_batches(10);
        for layer in &mut next_item {
            layer.asset_id = 8;
        }
        requests.append(&mut next_item);
        let (ops, stats) = frontend
            .append_frame_ops_with_affine_quads_and_mesh_batches_to_target(
                &mut gal,
                1,
                target,
                target,
                None,
                None,
                None,
                false,
                Vec::new(),
                Vec::new(),
                requests,
            )
            .expect("two mesh items sharing an owned raster target");
        assert_eq!(stats.mesh_item_count, 2);
        assert_eq!(stats.mesh_batch_count, 6);
        assert_eq!(stats.mesh_draw_count, 8);
        let composite_uniform_buffers = frontend
            .mesh_composites
            .values()
            .map(|resources| resources.uniform_buffer)
            .collect::<Vec<_>>();
        let composite_resource_sets = frontend
            .mesh_composites
            .values()
            .map(|resources| resources.resource_set)
            .collect::<Vec<_>>();
        let composite_bind_indices = ops
            .iter()
            .enumerate()
            .filter_map(|operation| match operation {
                (
                    index,
                    CommandOp::BindResourceSet {
                        set,
                        dynamic_offsets,
                        ..
                    },
                ) if composite_resource_sets.contains(set) => {
                    Some((index, dynamic_offsets.clone()))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            vec![vec![0], vec![GUI_MESH_COMPOSITE_UNIFORM_STRIDE]],
            composite_bind_indices
                .iter()
                .map(|(_, offsets)| offsets.clone())
                .collect::<Vec<_>>(),
            "each GUI item composite must bind its own uniform range"
        );
        let composite_uniform_offsets = composite_bind_indices
            .iter()
            .map(|(bind_index, _)| {
                ops[..*bind_index]
                    .iter()
                    .rev()
                    .find_map(|operation| match operation {
                        CommandOp::HostWriteBuffer { buffer, offset, .. }
                            if composite_uniform_buffers.contains(buffer) =>
                        {
                            Some(*offset)
                        }
                        _ => None,
                    })
                    .expect("a GUI mesh composite must write its bound UBO range")
            })
            .collect::<Vec<_>>();
        assert_eq!(
            vec![0, GUI_MESH_COMPOSITE_UNIFORM_STRIDE],
            composite_uniform_offsets,
            "same-sized GUI items must not overwrite one another's composite pose before execution"
        );
        assert_eq!(
            ops.iter()
                .filter(|operation| matches!(
                    operation,
                    CommandOp::Barrier(ResourceBarrier {
                        after: TextureUsageState::ColorAttachment,
                        ..
                    })
                ))
                .count(),
            2,
            "each GUI item establishes attachment-write usage once before its raster layers"
        );
        assert_eq!(
            2,
            frontend.mesh_rasters.len(),
            "each asset retains independent mutable bindings"
        );
        assert_eq!(
            1,
            frontend.mesh_shared_programs.len(),
            "matching mesh raster contracts share exactly one immutable program"
        );
        let rasters = frontend.mesh_rasters.values().collect::<Vec<_>>();
        assert_eq!(rasters[0].pipeline, rasters[1].pipeline);
        assert_eq!(rasters[0].pipeline_layout, rasters[1].pipeline_layout);
        assert_ne!(rasters[0].resource_set, rasters[1].resource_set);
        assert_ne!(rasters[0].uniform_buffer, rasters[1].uniform_buffer);
        gal.submit(SubmissionBatch {
            label: "gui-mesh-reuse-target-test".to_string(),
            command_lists: vec![CommandList::from(CommandListDesc {
                label: "gui".to_string(),
                operations: ops,
            })],
        })
        .expect("GAL validates sampled-to-attachment target reuse");
        frontend.reset(&mut gal).unwrap();
    }

    #[test]
    fn sprite_registry_ids_are_stable() {
        assert_eq!(94, SPRITES.len());
        assert_eq!("crosshair", sprite_def(1).unwrap().name);
        assert_eq!("boss-bar-overlay-progress", sprite_def(79).unwrap().name);
        assert_eq!("hunger-effect-full", sprite_def(85).unwrap().name);
        assert_eq!("air-full", sprite_def(86).unwrap().name);
        assert_eq!("air-popping", sprite_def(87).unwrap().name);
        assert_eq!("air-empty", sprite_def(88).unwrap().name);
        assert_eq!("mount-heart-container", sprite_def(89).unwrap().name);
        assert_eq!("mount-heart-full", sprite_def(90).unwrap().name);
        assert_eq!("mount-heart-half", sprite_def(91).unwrap().name);
        assert_eq!("post-effect-invert", sprite_def(92).unwrap().name);
        assert_eq!("post-effect-creeper", sprite_def(93).unwrap().name);
        assert_eq!("post-effect-spider", sprite_def(94).unwrap().name);
    }

    #[test]
    fn invert_post_effect_accepts_viewport_request_and_generates_vanilla_amount_source() {
        let definition = sprite_def(GUI_POST_EFFECT_INVERT_ID).unwrap();
        let request = GuiSpriteRequest {
            stratum: 80,
            sprite_id: GUI_POST_EFFECT_INVERT_ID,
            selected_slot: -1,
            progress_fraction: -1.0,
            fill_direction: 0,
            color_argb: 0xffff_ffff,
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            gui_width: 1920,
            gui_height: 1080,
            projection_extent: [1920.0, 1080.0],
            sequence: 1,
        };
        validate_request(&request, definition).unwrap();
        let bytes = load_sprite(definition, &std::collections::BTreeMap::new()).unwrap();
        assert_eq!(vec![204, 204, 204, 255], bytes);
    }

    #[test]
    fn compatible_batches_split_on_group_stratum_and_size() {
        let requests = vec![
            GuiSpriteRequest {
                stratum: 300,
                sprite_id: 2,
                selected_slot: -1,
                progress_fraction: 1.0,
                fill_direction: 0,
                color_argb: 0xffffffff,
                x: 0,
                y: 0,
                width: 1,
                height: 1,
                gui_width: 320,
                gui_height: 180,
                projection_extent: [320.0, 180.0],
                sequence: 1,
            },
            GuiSpriteRequest {
                stratum: 310,
                sprite_id: 3,
                selected_slot: -1,
                progress_fraction: 1.0,
                fill_direction: 0,
                color_argb: 0xffffffff,
                x: 0,
                y: 0,
                width: 1,
                height: 1,
                gui_width: 320,
                gui_height: 180,
                projection_extent: [320.0, 180.0],
                sequence: 2,
            },
        ];
        let mut batches = Vec::<GuiBatch>::new();
        for request in requests {
            let group = sprite_def(request.sprite_id).unwrap().group;
            append_gui_quad(
                &mut batches,
                request.stratum,
                group,
                PackedGuiQuad {
                    origin: [0.0, 0.0],
                    axis_u: [1.0, 0.0],
                    axis_v: [0.0, 1.0],
                    viewport: [320.0, 180.0],
                    clip: [0.0; 4],
                    clip_enabled: false,
                    pre_present_y_flip: false,
                    uv: [0.0, 0.0, 1.0, 1.0],
                    color: [1.0; 4],
                    texture_mode: 1.0,
                    z: 0.0,
                },
            );
        }
        assert_eq!(2, batches.len());
    }

    #[test]
    fn mixed_gui_record_families_preserve_scheduler_order_and_reject_duplicates() {
        let mut sprite = request(1);
        sprite.stratum = 700;
        sprite.sequence = 12;
        let affine = GuiAffineQuadRequest {
            item_raster_layers: vec![],
            item_raster_scale: 0,
            item_raster_geometry: Default::default(),
            material: crate::render::vulkanic::gui_item_material::GuiAffineMaterial::Unlit,
            stratum: 700,
            asset_id: 41,
            x0: 0.0,
            y0: 0.0,
            x1: 8.0,
            y1: 0.0,
            x3: 0.0,
            y3: 8.0,
            z: 0.0,
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
            color_argb: 0xffff_ffff,
            gui_width: 320,
            gui_height: 180,
            projection_extent: [320.0, 180.0],
            sequence: 11,
            clip_mode: 0,
            clip_left: 0,
            clip_top: 0,
            clip_width: 0,
            clip_height: 0,
        };
        let affine_second = GuiAffineQuadRequest {
            item_raster_layers: vec![],
            item_raster_scale: 0,
            item_raster_geometry: Default::default(),
            sequence: 10,
            x0: 12.0,
            x1: 20.0,
            x3: 12.0,
            ..affine.clone()
        };
        let ordered =
            order_gui_requests(vec![sprite.clone()], vec![affine.clone(), affine_second]).unwrap();
        assert!(matches!(&ordered[0], GuiFrameRequest::AffineBatch(batch) if batch.len() == 2));
        assert!(matches!(ordered[1], GuiFrameRequest::Sprite(_)));

        let error = order_gui_requests(
            vec![sprite],
            vec![GuiAffineQuadRequest {
                item_raster_layers: vec![],
                item_raster_scale: 0,
                item_raster_geometry: Default::default(),
                sequence: 12,
                ..affine
            }],
        )
        .unwrap_err();
        assert_eq!(StatusCode::InvalidArgument, error.code);
    }

    #[test]
    fn gui_mesh_batch_ordering_rejects_unbounded_submission_before_grouping() {
        let batches = (0..=GUI_MAX_MESH_BATCHES as u32)
            .map(mesh_batch)
            .collect::<Vec<_>>();
        let error = order_gui_requests_with_mesh(Vec::new(), Vec::new(), batches).unwrap_err();
        assert_eq!(StatusCode::InvalidArgument, error.code);
        assert!(error.to_string().contains("bounded limit"));
    }

    #[test]
    fn affine_gui_clip_is_frame_local_and_rejects_out_of_bounds_rectangles() {
        let mut request = GuiAffineQuadRequest {
            item_raster_layers: vec![],
            item_raster_scale: 0,
            item_raster_geometry: Default::default(),
            material: crate::render::vulkanic::gui_item_material::GuiAffineMaterial::Unlit,
            stratum: 700,
            asset_id: 41,
            x0: 0.0,
            y0: 0.0,
            x1: 8.0,
            y1: 0.0,
            x3: 0.0,
            y3: 8.0,
            z: 0.0,
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
            color_argb: 0xffff_ffff,
            gui_width: 320,
            gui_height: 180,
            projection_extent: [320.0, 180.0],
            sequence: 1,
            clip_mode: 1,
            clip_left: 12,
            clip_top: 18,
            clip_width: 64,
            clip_height: 20,
        };
        validate_affine_quad(&request).unwrap();
        request.clip_width = 309;
        let error = validate_affine_quad(&request).unwrap_err();
        assert_eq!(StatusCode::InvalidArgument, error.code);
    }

    #[test]
    fn asset_updates_are_generation_ordered_and_copied() {
        let mut gal = mock_gal();
        let mut frontend = GuiFrontend::default();
        let sprite = sprite_def(1).unwrap();
        let mut bytes = bundled_sprite_bytes(sprite.path).unwrap().to_vec();
        frontend
            .apply_asset_update(
                &mut gal,
                2,
                vec![GuiAssetPayload {
                    sprite_id: sprite.id,
                    png_bytes: bytes.clone(),
                }],
            )
            .unwrap();
        bytes.clear();
        assert_eq!(2, frontend.asset_generation);
        assert!(!frontend.asset_overrides.get(&sprite.id).unwrap().is_empty());
        let stale = frontend
            .apply_asset_update(&mut gal, 2, Vec::new())
            .unwrap_err();
        assert_eq!(StatusCode::InvalidArgument, stale.code);
        assert_eq!(2, frontend.asset_generation);
    }

    #[test]
    fn malformed_asset_update_rolls_back_to_last_valid_assets() {
        let mut gal = mock_gal();
        let mut frontend = GuiFrontend::default();
        let sprite = sprite_def(1).unwrap();
        let valid = bundled_sprite_bytes(sprite.path).unwrap().to_vec();
        frontend
            .apply_asset_update(
                &mut gal,
                2,
                vec![GuiAssetPayload {
                    sprite_id: sprite.id,
                    png_bytes: valid.clone(),
                }],
            )
            .unwrap();
        let before = frontend.asset_overrides.get(&sprite.id).unwrap().clone();
        let error = frontend
            .apply_asset_update(
                &mut gal,
                3,
                vec![GuiAssetPayload {
                    sprite_id: sprite.id,
                    png_bytes: vec![1, 2, 3, 4],
                }],
            )
            .unwrap_err();
        assert!(error.message.contains("failed to decode GUI sprite"));
        assert_eq!(2, frontend.asset_generation);
        assert_eq!(before, *frontend.asset_overrides.get(&sprite.id).unwrap());
    }

    #[test]
    fn missing_asset_payloads_fall_back_to_vanilla() {
        let mut gal = mock_gal();
        let mut frontend = GuiFrontend::default();
        frontend
            .apply_asset_update(&mut gal, 2, Vec::new())
            .unwrap();
        assert!(frontend.asset_overrides.is_empty());
        let atlas = frontend.atlas_for(TextureGroup::Alpha).unwrap();
        assert!(atlas.regions.contains_key(&2));
    }

    #[test]
    fn static_and_raw_gui_images_share_top_origin_uv_coordinates() {
        let mut gal = mock_gal();
        let mut frontend = GuiFrontend::default();
        frontend
            .apply_asset_update(&mut gal, 2, Vec::new())
            .unwrap();

        let mut sprite_request = request(58);
        let sprite = sprite_def(sprite_request.sprite_id).unwrap();
        sprite_request.width = sprite.width;
        sprite_request.height = sprite.height;
        sprite_request.projection_extent = [319.75, 179.5];
        let packed = frontend
            .pack_sprite(&sprite_request, sprite, false)
            .unwrap();
        let atlas = frontend.atlas_for(sprite.group).unwrap();
        let region = atlas.regions.get(&sprite.id).unwrap();
        assert_eq!(
            [
                region.x as f32 / atlas.width as f32,
                region.y as f32 / atlas.height as f32,
            ],
            packed.uv[..2]
        );

        let direct_bytes = packed_uniform_bytes(&GuiBatch {
            stratum: sprite_request.stratum,
            group: sprite.group,
            quads: vec![packed],
        })
        .unwrap();
        let direct_flip = f32::from_le_bytes(direct_bytes[44..48].try_into().unwrap());
        assert_eq!(319.75, f32::from_le_bytes(direct_bytes[32..36].try_into().unwrap()));
        assert_eq!(179.5, f32::from_le_bytes(direct_bytes[36..40].try_into().unwrap()));
        assert_eq!(
            0.0, direct_flip,
            "ordinary GUI targets must not pre-flip their stream"
        );

        let source_packed = frontend.pack_sprite(&sprite_request, sprite, true).unwrap();
        let source_bytes = packed_uniform_bytes(&GuiBatch {
            stratum: sprite_request.stratum,
            group: sprite.group,
            quads: vec![source_packed],
        })
        .unwrap();
        let source_flip = f32::from_le_bytes(source_bytes[44..48].try_into().unwrap());
        assert_eq!(
            1.0, source_flip,
            "the source overlay must precompensate its final present copy"
        );

        // Raw font images arrive in the same row-zero-is-top convention, so
        // their top-left semantic UV must address the first uploaded texel.
        let raw = GuiAffineQuadRequest {
            item_raster_layers: vec![],
            item_raster_scale: 0,
            item_raster_geometry: Default::default(),
            asset_id: 7,
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
            ..GuiAffineQuadRequest {
                item_raster_layers: vec![],
                item_raster_scale: 0,
                item_raster_geometry: Default::default(),
                material: crate::render::vulkanic::gui_item_material::GuiAffineMaterial::Unlit,
                stratum: 700,
                asset_id: 7,
                x0: 0.0,
                y0: 0.0,
                x1: 1.0,
                y1: 0.0,
                x3: 0.0,
                y3: 1.0,
                z: 0.0,
                u0: 0.0,
                v0: 0.0,
                u1: 1.0,
                v1: 1.0,
                color_argb: 0xffff_ffff,
                gui_width: 1,
                gui_height: 1,
                projection_extent: [1.0, 1.0],
                sequence: 1,
                clip_mode: 0,
                clip_left: 0,
                clip_top: 0,
                clip_width: 0,
                clip_height: 0,
            }
        };
        validate_affine_quad(&raw).unwrap();
    }

    #[test]
    fn asset_reload_rebuilds_once_then_reuses_cached_resources() {
        let mut gal = mock_gal();
        let mut frontend = GuiFrontend::default();
        let target = frame_target(&mut gal);
        frontend
            .apply_asset_update(&mut gal, 2, Vec::new())
            .unwrap();

        let first = frontend
            .submit_frame(&mut gal, 10, target, vec![request(2), GuiSpriteRequest { sequence: 2, ..request(3) }])
            .unwrap();
        assert!(first.resource_creates > 0);
        assert!(first.cache_misses > 0);

        let second = frontend
            .submit_frame(&mut gal, 10, target, vec![request(2), GuiSpriteRequest { sequence: 2, ..request(3) }])
            .unwrap();
        assert_eq!(0, second.resource_creates);
        assert!(second.cache_hits > 0);
        assert_eq!(0, second.cache_misses);
    }

    #[test]
    fn vulkan_whole_frame_gui_submission_clears_once_before_batches() {
        let mut gal = mock_gal();
        let mut frontend = GuiFrontend::default();
        let target = frame_target(&mut gal);

        let stats = frontend
            .submit_frame(&mut gal, 10, target, vec![request(2), GuiSpriteRequest { sequence: 2, ..request(3) }])
            .unwrap();

        assert_eq!(2, stats.sprite_batch_count);
        assert_eq!(1, stats.command_lists);
        assert_eq!(20, stats.command_ops);
    }

    #[test]
    fn raw_image_assets_are_copied_validated_and_rendered_as_affine_quads() {
        let mut gal = mock_gal();
        let mut frontend = GuiFrontend::default();
        let target = frame_target(&mut gal);
        let mut pixels = vec![0, 64, 128, 255];
        frontend
            .apply_raw_image_update(
                &mut gal,
                3,
                vec![GuiRawImageAssetPayload { sampling: None,
                    asset_id: 41,
                    format: GuiRawImageFormat::Alpha8,
                    width: 2,
                    height: 2,
                    pixels: pixels.clone(),
                }],
            )
            .unwrap();
        pixels.fill(7);
        assert_eq!(vec![0, 64, 128, 255], frontend.raw_images[&41].pixels);

        let (_, stats) = frontend
            .append_frame_ops_with_affine_quads(
                &mut gal,
                9,
                target,
                Vec::new(),
                vec![GuiAffineQuadRequest {
                    item_raster_layers: vec![],
                    item_raster_scale: 0,
                    item_raster_geometry: Default::default(),
                    material: crate::render::vulkanic::gui_item_material::GuiAffineMaterial::Unlit,
                    stratum: 420,
                    asset_id: 41,
                    x0: 10.0,
                    y0: 20.0,
                    x1: 18.0,
                    y1: 21.0,
                    x3: 9.0,
                    y3: 29.0,
                    z: 0.03,
                    u0: 0.0,
                    v0: 0.0,
                    u1: 1.0,
                    v1: 1.0,
                    color_argb: 0xff336699,
                    gui_width: 320,
                    gui_height: 180,
                    projection_extent: [320.0, 180.0],
                    sequence: 1,
                    clip_mode: 0,
                    clip_left: 0,
                    clip_top: 0,
                    clip_width: 0,
                    clip_height: 0,
                }],
            )
            .unwrap();
        assert_eq!(1, stats.affine_quad_count);
        assert_eq!(1, stats.sprite_batch_count);
        assert!(frontend.resources.contains_key(&ResourceKey::new(
            TextureGroup::Dynamic(41),
            ColorFormat::Rgba8Unorm,
            None,
        )));
    }

    #[test]
    fn unchanged_raw_image_generation_retains_dynamic_gpu_resources() {
        let mut gal = mock_gal();
        let mut frontend = GuiFrontend::default();
        let payload = GuiRawImageAssetPayload { sampling: None,
            asset_id: 41,
            format: GuiRawImageFormat::Rgba8,
            width: 1,
            height: 1,
            pixels: vec![1, 2, 3, 4],
        };
        frontend
            .apply_raw_image_update(&mut gal, 1, vec![payload.clone()])
            .unwrap();
        let mut stats = GuiSubmitStats::default();
        frontend
            .ensure_resources(
                &mut gal,
                TextureGroup::Dynamic(41),
                ColorFormat::Rgba8Unorm,
                None,
                &mut stats,
            )
            .unwrap();
        let key = ResourceKey::new(TextureGroup::Dynamic(41), ColorFormat::Rgba8Unorm, None);
        let first = frontend.resources[&key].texture;
        frontend
            .apply_raw_image_update(&mut gal, 2, vec![payload])
            .unwrap();
        assert_eq!(first, frontend.resources[&key].texture);
    }

    #[test]
    fn gui_textures_share_one_immutable_pipeline_per_explicit_raster_contract() {
        let mut gal = mock_gal();
        let mut frontend = GuiFrontend::default();
        frontend
            .apply_raw_image_update(
                &mut gal,
                3,
                vec![
                    GuiRawImageAssetPayload { sampling: None,
                        asset_id: 41,
                        format: GuiRawImageFormat::Alpha8,
                        width: 2,
                        height: 2,
                        pixels: vec![255; 4],
                    },
                    GuiRawImageAssetPayload { sampling: None,
                        asset_id: 42,
                        format: GuiRawImageFormat::Alpha8,
                        width: 2,
                        height: 2,
                        pixels: vec![127; 4],
                    },
                ],
            )
            .unwrap();
        let mut stats = GuiSubmitStats::default();
        frontend
            .ensure_resources(
                &mut gal,
                TextureGroup::Dynamic(41),
                ColorFormat::Rgba8Unorm,
                None,
                &mut stats,
            )
            .unwrap();
        frontend
            .ensure_resources(
                &mut gal,
                TextureGroup::Dynamic(42),
                ColorFormat::Rgba8Unorm,
                None,
                &mut stats,
            )
            .unwrap();

        assert_eq!(2, frontend.resources.len());
        assert_eq!(1, frontend.shared_pipelines.len());
        let first = &frontend.resources
            [&ResourceKey::new(TextureGroup::Dynamic(41), ColorFormat::Rgba8Unorm, None)];
        let second = &frontend.resources
            [&ResourceKey::new(TextureGroup::Dynamic(42), ColorFormat::Rgba8Unorm, None)];
        assert_eq!(first.pipeline, second.pipeline);
        assert_eq!(first.pipeline_layout, second.pipeline_layout);
        assert_ne!(first.resource_set, second.resource_set);
    }

    #[test]
    fn dynamic_gui_blend_strata_share_one_explicit_texture_resource() {
        let mut gal = mock_gal();
        let mut frontend = GuiFrontend::default();
        frontend
            .apply_raw_image_update(
                &mut gal,
                1,
                vec![GuiRawImageAssetPayload { sampling: None,
                    asset_id: 41,
                    format: GuiRawImageFormat::Rgba8,
                    width: 2,
                    height: 2,
                    pixels: vec![1; 16],
                }],
            )
            .unwrap();
        let mut stats = GuiSubmitStats::default();
        frontend
            .ensure_resources(
                &mut gal,
                TextureGroup::Dynamic(41),
                ColorFormat::Rgba8Unorm,
                None,
                &mut stats,
            )
            .unwrap();
        let texture_creates_after_first = gal
            .mock_backend()
            .unwrap()
            .creates
            .iter()
            .filter(|(_, kind)| *kind == HandleKind::Texture)
            .count();
        frontend
            .ensure_resources(
                &mut gal,
                TextureGroup::DynamicOpaque(41),
                ColorFormat::Rgba8Unorm,
                None,
                &mut stats,
            )
            .unwrap();
        frontend
            .ensure_resources(
                &mut gal,
                TextureGroup::DynamicLinear(41),
                ColorFormat::Rgba8Unorm,
                None,
                &mut stats,
            )
            .unwrap();
        let texture_creates_after_all_groups = gal
            .mock_backend()
            .unwrap()
            .creates
            .iter()
            .filter(|(_, kind)| *kind == HandleKind::Texture)
            .count();
        let alpha = &frontend.resources
            [&ResourceKey::new(TextureGroup::Dynamic(41), ColorFormat::Rgba8Unorm, None)];
        let opaque = &frontend.resources[&ResourceKey::new(
            TextureGroup::DynamicOpaque(41),
            ColorFormat::Rgba8Unorm,
            None,
        )];
        let linear = &frontend.resources[&ResourceKey::new(
            TextureGroup::DynamicLinear(41),
            ColorFormat::Rgba8Unorm,
            None,
        )];
        assert_eq!(alpha.texture, opaque.texture);
        assert_eq!(alpha.texture_view, opaque.texture_view);
        assert_eq!(alpha.sampler, opaque.sampler);
        assert_eq!(alpha.texture, linear.texture);
        assert_eq!(alpha.texture_view, linear.texture_view);
        assert_ne!(alpha.sampler, linear.sampler,
            "continuous semantic images require a distinct linear sampler without copying their Rust-owned texture");
        assert_eq!(1, frontend.dynamic_textures.len());
        assert_eq!(1, texture_creates_after_first);
        assert_eq!(texture_creates_after_first, texture_creates_after_all_groups);
        gal.mock_backend_mut().unwrap().fail_next_submit();
        assert!(frontend.ensure_resources(&mut gal, TextureGroup::DynamicPremultiplied(41),
            ColorFormat::Rgba8Unorm, None, &mut stats).is_err());
        frontend.ensure_resources(&mut gal, TextureGroup::DynamicPremultiplied(41),
            ColorFormat::Rgba8Unorm, None, &mut stats).unwrap();
        frontend.destroy_render_resources(&mut gal);
        gal.retire_through(gal.latest_submission_id()).unwrap();
        assert_eq!(gal.metrics().resource_creates, gal.metrics().resource_destroys,
            "reused GUI image bindings must not orphan a newly allocated upload buffer");
    }

    #[test]
    fn gui_glint_sampling_is_selected_by_material_not_scheduler_stratum() {
        let mut batch = mesh_batch(0);
        for stratum in [1, GUI_OPAQUE_BLIT_STRATUM, GUI_VIGNETTE_BLIT_STRATUM] {
            batch.stratum = stratum;
            batch.material_mode = GuiMeshMaterialMode::Glint;
            batch.alpha_cutoff = 0.1;
            let glint = prepare_gui_mesh_draws(&[batch.clone()]).unwrap().remove(0);
            assert_eq!(dynamic_mesh_texture_group(&glint), TextureGroup::DynamicGlint(batch.asset_id));
            batch.material_mode = GuiMeshMaterialMode::Opaque;
            batch.alpha_cutoff = 0.0;
            let ordinary = prepare_gui_mesh_draws(&[batch.clone()]).unwrap().remove(0);
            assert_ne!(dynamic_mesh_texture_group(&ordinary), TextureGroup::DynamicGlint(batch.asset_id));
        }
    }

    #[test]
    fn gui_glint_sampler_is_explicit_shared_image_safe_and_failure_atomic() {
        for glint_first in [false, true] {
            let mut gal = mock_gal();
            let mut frontend = GuiFrontend::default();
            frontend.apply_raw_image_update(&mut gal, 1, vec![GuiRawImageAssetPayload { sampling: None,
                asset_id: 41, format: GuiRawImageFormat::Rgba8, width: 2, height: 2,
                pixels: vec![127; 16],
            }]).unwrap();
            let glint = TextureGroup::DynamicGlint(41);
            let ordinary = TextureGroup::Dynamic(41);
            let groups = if glint_first { [glint, ordinary] } else { [ordinary, glint] };
            for group in groups {
                if group == glint {
                    gal.mock_backend_mut().unwrap().fail_next_submit();
                    assert!(frontend.ensure_resources(&mut gal, group, ColorFormat::Rgba8Unorm,
                        None, &mut GuiSubmitStats::default()).is_err());
                }
                frontend.ensure_resources(&mut gal, group, ColorFormat::Rgba8Unorm,
                    None, &mut GuiSubmitStats::default()).unwrap();
            }
            let repeat = &frontend.resources[&ResourceKey::new(glint, ColorFormat::Rgba8Unorm, None)];
            let clamp = &frontend.resources[&ResourceKey::new(ordinary, ColorFormat::Rgba8Unorm, None)];
            assert_eq!(repeat.texture, clamp.texture);
            assert_eq!(repeat.texture_view, clamp.texture_view);
            assert_eq!(repeat.private_sampler, Some(repeat.sampler));
            assert_eq!(clamp.private_sampler, None);
            let desc = gal.sampler_descriptor_for_test(repeat.sampler).unwrap();
            assert_eq!(desc.min_filter, SamplerFilter::Linear);
            assert_eq!(desc.mag_filter, SamplerFilter::Linear);
            assert_eq!(desc.address_u, SamplerAddressMode::Repeat);
            assert_eq!(desc.address_v, SamplerAddressMode::Repeat);
            assert_eq!(gal.sampler_descriptor_for_test(clamp.sampler).unwrap().address_u,
                SamplerAddressMode::ClampToEdge);
            let old_sampler = repeat.sampler;
            frontend.apply_raw_image_update(&mut gal, 2, vec![GuiRawImageAssetPayload { sampling: None,
                asset_id: 41, format: GuiRawImageFormat::Rgba8, width: 2, height: 2,
                pixels: vec![255; 16],
            }]).unwrap();
            gal.retire_through(gal.latest_submission_id()).unwrap();
            assert!(gal.sampler_descriptor_for_test(old_sampler).is_err());
            frontend.reset(&mut gal).unwrap();
            gal.retire_through(gal.latest_submission_id()).unwrap();
            assert_eq!(gal.metrics().resource_creates, gal.metrics().resource_destroys);
        }
    }

    #[test]
    fn gui_glint_resource_sampling_metadata_only_reload_rebinds_without_changing_shared_sampler() {
        let mut gal = mock_gal();
        let mut frontend = GuiFrontend::default();
        let mut generation = 0;
        let mut previous_sampler = None;
        for filter in [SamplerFilter::Nearest, SamplerFilter::Linear] {
            for address in [SamplerAddressMode::Repeat, SamplerAddressMode::ClampToEdge] {
                generation += 1;
                frontend.apply_raw_image_update(&mut gal, generation, vec![GuiRawImageAssetPayload {
                    asset_id: 41, format: GuiRawImageFormat::Rgba8, width: 2, height: 2,
                    pixels: vec![127; 16], sampling: Some((filter, address)),
                }]).unwrap();
                for group in [TextureGroup::Dynamic(41), TextureGroup::DynamicGlint(41)] {
                    frontend.ensure_resources(&mut gal, group, ColorFormat::Rgba8Unorm,
                        None, &mut GuiSubmitStats::default()).unwrap();
                }
                let glint = &frontend.resources[&ResourceKey::new(TextureGroup::DynamicGlint(41), ColorFormat::Rgba8Unorm, None)];
                let ordinary = &frontend.resources[&ResourceKey::new(TextureGroup::Dynamic(41), ColorFormat::Rgba8Unorm, None)];
                let desc = gal.sampler_descriptor_for_test(glint.sampler).unwrap();
                assert_eq!(filter, desc.min_filter);
                assert_eq!(filter, desc.mag_filter);
                assert_eq!(address, desc.address_u);
                assert_eq!(address, desc.address_v);
                assert_eq!(glint.texture, ordinary.texture);
                assert_eq!(SamplerAddressMode::ClampToEdge, gal.sampler_descriptor_for_test(ordinary.sampler).unwrap().address_u);
                if let Some(old) = previous_sampler {
                    assert_ne!(old, glint.sampler);
                    gal.retire_through(gal.latest_submission_id()).unwrap();
                    assert!(gal.sampler_descriptor_for_test(old).is_err());
                }
                previous_sampler = Some(glint.sampler);
            }
        }
        frontend.reset(&mut gal).unwrap();
        gal.retire_through(gal.latest_submission_id()).unwrap();
        assert_eq!(gal.metrics().resource_creates, gal.metrics().resource_destroys);
    }

    #[test]
    fn vulkan_gui_shared_image_reload_retires_every_binding_and_upload_buffer() {
        let backend = VulkanBackend::new("GUI shared image resource bound regression").unwrap();
        let mut gal = VulkanicGal::new_with_backend(Box::new(backend), false);
        let mut frontend = GuiFrontend::default();
        let mut stable_live_count = None;
        for generation in 1..=16 {
            frontend.apply_raw_image_update(&mut gal, generation, vec![GuiRawImageAssetPayload { sampling: None,
                asset_id: 41, format: GuiRawImageFormat::Rgba8, width: 2, height: 2,
                pixels: vec![generation as u8; 16],
            }]).unwrap();
            for group in [TextureGroup::Dynamic(41), TextureGroup::DynamicOpaque(41),
                TextureGroup::DynamicLinear(41), TextureGroup::DynamicPremultiplied(41),
                TextureGroup::DynamicGlint(41)] {
                frontend.ensure_resources(&mut gal, group, ColorFormat::Rgba8Unorm, None,
                    &mut GuiSubmitStats::default()).unwrap();
            }
            gal.retire_through(gal.latest_submission_id()).unwrap();
            let live_count = gal.metrics().resource_creates - gal.metrics().resource_destroys;
            assert_eq!(*stable_live_count.get_or_insert(live_count), live_count,
                "changing image pixels cannot accumulate discarded upload buffers");
        }
        frontend.reset(&mut gal).unwrap();
        gal.retire_through(gal.latest_submission_id()).unwrap();
        assert_eq!(gal.metrics().resource_creates, gal.metrics().resource_destroys);
    }

    #[test]
    fn explicit_atlas_declarations_cannot_alias_copied_images_or_admit_fallback_sampling() {
        let mut gal = mock_gal();
        let mut frontend = GuiFrontend::default();
        let reference = GuiAtlasReference {
            asset_id: 101,
            atlas: AcceptedAtlasIncarnation { texture_id: 202, generation: 3, width: 32, height: 16 },
            x: 16, y: 0, width: 16, height: 16,
        };
        frontend.stage_atlas_references(1, &[reference], |_| Some(reference.atlas)).unwrap();
        assert_eq!(frontend.texture_source(TextureGroup::Dynamic(101)).err().unwrap().code,
            StatusCode::UnsupportedFeature);
        let payload = || GuiRawImageAssetPayload { sampling: None,
            asset_id: 101, format: GuiRawImageFormat::Rgba8, width: 1, height: 1, pixels: vec![255; 4],
        };
        assert!(frontend.apply_raw_image_update(&mut gal, 1, vec![payload()]).is_err());
        assert!(frontend.raw_images.is_empty());
        assert_eq!(frontend.raw_image_generation, 0);
        assert!(frontend.resources.is_empty());
        assert!(frontend.dynamic_textures.is_empty());
        frontend.atlas_references.clear();
        frontend.apply_raw_image_update(&mut gal, 1, vec![payload()]).unwrap();
        assert!(frontend.stage_atlas_references(1, &[reference], |_| Some(reference.atlas)).is_err());
        assert!(!frontend.atlas_references.contains(101));
        assert_eq!(frontend.texture_source(TextureGroup::Dynamic(101)).unwrap().bytes, vec![255; 4]);
        frontend.reset(&mut gal).unwrap();
    }

    #[test]
    fn changed_dynamic_gui_image_retires_shared_texture_before_recreation() {
        let mut gal = mock_gal();
        let mut frontend = GuiFrontend::default();
        frontend
            .apply_raw_image_update(
                &mut gal,
                1,
                vec![GuiRawImageAssetPayload { sampling: None,
                    asset_id: 41,
                    format: GuiRawImageFormat::Rgba8,
                    width: 2,
                    height: 2,
                    pixels: vec![1; 16],
                }],
            )
            .unwrap();
        let mut stats = GuiSubmitStats::default();
        frontend
            .ensure_resources(
                &mut gal,
                TextureGroup::Dynamic(41),
                ColorFormat::Rgba8Unorm,
                None,
                &mut stats,
            )
            .unwrap();
        let first_texture = frontend
            .resources
            .get(&ResourceKey::new(
                TextureGroup::Dynamic(41),
                ColorFormat::Rgba8Unorm,
                None,
            ))
            .unwrap()
            .texture;

        frontend
            .apply_raw_image_update(
                &mut gal,
                2,
                vec![GuiRawImageAssetPayload { sampling: None,
                    asset_id: 41,
                    format: GuiRawImageFormat::Rgba8,
                    width: 2,
                    height: 2,
                    pixels: vec![2; 16],
                }],
            )
            .unwrap();
        assert!(frontend.resources.is_empty());
        assert!(frontend.dynamic_textures.is_empty());
        gal.mock_backend_mut()
            .unwrap()
            .complete_through(SubmissionId(1));
        gal.retire_completed().unwrap();

        frontend
            .ensure_resources(
                &mut gal,
                TextureGroup::Dynamic(41),
                ColorFormat::Rgba8Unorm,
                None,
                &mut stats,
            )
            .unwrap();
        let second_texture = frontend
            .resources
            .get(&ResourceKey::new(
                TextureGroup::Dynamic(41),
                ColorFormat::Rgba8Unorm,
                None,
            ))
            .unwrap()
            .texture;
        assert_ne!(first_texture, second_texture);
        assert_eq!(
            2,
            gal.mock_backend()
                .unwrap()
                .creates
                .iter()
                .filter(|(_, kind)| *kind == HandleKind::Texture)
                .count()
        );
        assert_eq!(
            1,
            gal.mock_backend()
                .unwrap()
                .destroys
                .iter()
                .filter(|(_, kind)| *kind == HandleKind::Texture)
                .count()
        );
        frontend.destroy_render_resources(&mut gal);
    }

    #[test]
    fn affine_pipeline_strata_select_exact_blend_groups() {
        assert_eq!(BlendMode::Disabled, dynamic_texture_group(760, 1).blend());
        assert_eq!(BlendMode::Vignette, dynamic_texture_group(770, 1).blend());
        assert_eq!(BlendMode::Invert, dynamic_texture_group(780, 1).blend());
        assert_eq!(BlendMode::Invert, dynamic_texture_group(200, 1).blend());
        assert_eq!(
            BlendMode::Premultiplied,
            dynamic_texture_group(790, 1).blend()
        );
        assert_eq!(BlendMode::Additive, dynamic_texture_group(795, 1).blend());
        assert_eq!(BlendMode::Alpha, dynamic_texture_group(805, 1).blend());
        assert_eq!(BlendMode::Alpha, dynamic_texture_group(750, 1).blend());
    }

    #[test]
    fn copied_png_gui_assets_preserve_frozen_rgba8_channels_for_vignette_blending() {
        assert_eq!(
            TextureFormat::Rgba8Unorm,
            GuiRawImageFormat::Rgba8.texture_format()
        );
        assert_eq!(4, GuiRawImageFormat::Rgba8.bytes_per_pixel());
        assert_eq!(1.0, GuiRawImageFormat::Rgba8.shader_mode());
        let shader = std::str::from_utf8(FRAGMENT_SHADER_VULKAN).unwrap();
        assert!(shader.contains("vec4 sampled = texture(sampler2D(Tex0, Samp0), v_uv)"));
        assert!(!shader.contains("texelFetch") && !shader.contains("floor(v_sprite_corner"));
        assert!(!shader.contains("v_texture_mode > 1.5"));
    }

    #[test]
    fn every_dynamic_gui_blend_group_is_retirable() {
        assert!(TextureGroupKey::from(TextureGroup::DynamicItemRaster(1)).is_dynamic());
        assert!(TextureGroupKey::from(TextureGroup::DynamicItemCutout(1)).is_dynamic());
        assert!(TextureGroupKey::from(TextureGroup::Dynamic(1)).is_dynamic());
        assert!(TextureGroupKey::from(TextureGroup::DynamicOpaque(1)).is_dynamic());
        assert!(TextureGroupKey::from(TextureGroup::DynamicVignette(1)).is_dynamic());
        assert!(TextureGroupKey::from(TextureGroup::DynamicInvert(1)).is_dynamic());
        assert!(TextureGroupKey::from(TextureGroup::DynamicPremultiplied(1)).is_dynamic());
        assert!(TextureGroupKey::from(TextureGroup::DynamicAdditive(1)).is_dynamic());
        assert!(TextureGroupKey::from(TextureGroup::DynamicLequalDepth(1)).is_dynamic());
        assert!(!TextureGroupKey::Alpha.is_dynamic());
    }

    #[test]
    fn malformed_raw_image_update_rolls_back_without_destroying_valid_generation() {
        let mut gal = mock_gal();
        let mut frontend = GuiFrontend::default();
        frontend
            .apply_raw_image_update(
                &mut gal,
                4,
                vec![GuiRawImageAssetPayload { sampling: None,
                    asset_id: 7,
                    format: GuiRawImageFormat::Rgba8,
                    width: 1,
                    height: 1,
                    pixels: vec![1, 2, 3, 4],
                }],
            )
            .unwrap();
        let error = frontend
            .apply_raw_image_update(
                &mut gal,
                5,
                vec![GuiRawImageAssetPayload { sampling: None,
                    asset_id: 8,
                    format: GuiRawImageFormat::Alpha8,
                    width: 2,
                    height: 2,
                    pixels: vec![1, 2, 3],
                }],
            )
            .unwrap_err();
        assert_eq!(StatusCode::InvalidArgument, error.code);
        assert_eq!(4, frontend.raw_image_generation);
        assert!(frontend.raw_images.contains_key(&7));
        assert!(!frontend.raw_images.contains_key(&8));
    }

    #[test]
    fn oversized_raw_image_pixel_count_is_rejected_before_payload_validation() {
        let mut gal = mock_gal();
        let mut frontend = GuiFrontend::default();
        let error = frontend
            .apply_raw_image_update(
                &mut gal,
                1,
                vec![GuiRawImageAssetPayload { sampling: None,
                    asset_id: 99,
                    format: GuiRawImageFormat::Alpha8,
                    width: 8192,
                    height: 8192,
                    pixels: Vec::new(),
                }],
            )
            .unwrap_err();
        assert_eq!(StatusCode::InvalidArgument, error.code);
        assert!(error.message.contains("pixels"));
        assert_eq!(0, frontend.raw_image_generation);
        assert!(frontend.raw_images.is_empty());
    }

    #[test]
    fn failed_asset_generation_preserves_last_valid_atlas_bytes() {
        let mut gal = mock_gal();
        let mut frontend = GuiFrontend::default();
        let sprite = sprite_def(2).unwrap();
        frontend
            .apply_asset_update(
                &mut gal,
                2,
                vec![GuiAssetPayload {
                    sprite_id: sprite.id,
                    png_bytes: bundled_sprite_bytes(sprite.path).unwrap().to_vec(),
                }],
            )
            .unwrap();
        let atlas_before = frontend
            .atlas_for(TextureGroup::Alpha)
            .unwrap()
            .bytes
            .clone();

        let error = frontend
            .apply_asset_update(
                &mut gal,
                3,
                vec![GuiAssetPayload {
                    sprite_id: sprite.id,
                    png_bytes: vec![0, 1, 2, 3],
                }],
            )
            .unwrap_err();

        assert!(error.message.contains("failed to decode GUI sprite"));
        assert_eq!(2, frontend.asset_generation);
        assert_eq!(
            atlas_before,
            frontend.atlas_for(TextureGroup::Alpha).unwrap().bytes
        );
    }

    #[test]
    fn custom_post_effect_cleanup_orders_dependents_before_sampler_inputs() {
        let handle = |kind| Handle::new(kind, 1, 1).unwrap();
        let resources = CustomPostEffectResources {
            frame_copy_scratch: None,
            identity: "test".to_owned(),
            width: 1,
            height: 1,
            color_format: ColorFormat::Rgba8Unorm,
            snapshots: vec![handle(HandleKind::Texture)],
            snapshot_views: vec![handle(HandleKind::TextureView)],
            target_samplers: vec![handle(HandleKind::Sampler)],
            image_textures: vec![None],
            image_views: vec![None],
            image_samplers: vec![None],
            image_upload_buffers: vec![None],
            combined_samplers: vec![handle(HandleKind::CombinedTextureSampler)],
            uniform_buffers: vec![handle(HandleKind::Buffer)],
            vertex_shader: handle(HandleKind::ShaderModule),
            fragment_shader: handle(HandleKind::ShaderModule),
            resource_layout: handle(HandleKind::ResourceLayout),
            resource_set: handle(HandleKind::ResourceSet),
            pipeline_layout: handle(HandleKind::PipelineLayout),
            pipeline: handle(HandleKind::GraphicsPipeline),
        };
        let kinds = resources
            .handles_in_destroy_order()
            .into_iter()
            .map(|handle| handle.kind().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec![
                HandleKind::GraphicsPipeline,
                HandleKind::PipelineLayout,
                HandleKind::ResourceSet,
                HandleKind::ResourceLayout,
                HandleKind::ShaderModule,
                HandleKind::ShaderModule,
                HandleKind::CombinedTextureSampler,
                HandleKind::Buffer,
                HandleKind::Sampler,
                HandleKind::TextureView,
                HandleKind::Texture,
            ]
        );
    }
}
