//! Private semantic model for Rust-owned 3D GUI meshes.
//!
//! This module deliberately stops before GAL allocation and native lowering.
//! It gives the GUI frontend one validated, backend-neutral representation for
//! copied vanilla item meshes. Java/PIP renderer objects and backend state are
//! not part of this boundary.

use ch::Hasher;
use core::hash as ch;
use std::collections::{BTreeMap, BTreeSet};

use super::commands::{
    AttachmentLoadOp, AttachmentStoreOp, ClearColor, CommandOp, PassAttachment, ResourceBarrier,
    TextureUsageState,
};
use super::error::{GalError, GalResult, StatusCode};
use super::gal::VulkanicGal;
pub use super::item_foil::StandardItemFoil as GuiItemFoil;
pub use super::gui_item_material::GuiFlatItemLighting;
use super::gui_frontend::GUI_MAX_VIEWPORT_AXIS;
use super::handles::Handle;
use super::resources::{
    AccessFlags, BackendApi, BlendMode, BufferDesc, BufferUsage, ColorFormat, CompareOp, Extent3d,
    GraphicsPipelineDesc, IndexType, MemoryDomain, PipelineLayoutDesc, PipelineStageFlags,
    PrimitiveTopology, QueueClass, RenderPassDesc, RenderTargetDesc, ResourceBinding,
    ResourceBindingDesc, ResourceBindingKind, ResourceLayoutDesc, ResourceSetDesc,
    ShaderCodeFormat, ShaderModuleDesc, ShaderStage, TextureDesc, TextureDimension, TextureFormat,
    TextureUsage, TextureViewDesc,
};
use super::CullMode;

pub const GUI_MESH_MAX_BATCHES: usize = 1_024;
pub const GUI_MESH_MAX_VERTICES: usize = 65_536;
pub const GUI_MESH_MAX_INDICES: usize = 196_608;
pub const GUI_MESH_GPU_VERTEX_BYTES: usize = 3 * 4 * 4;
/// Aggregate copied GUI geometry admitted for one semantic frame. This keeps
/// nested mesh slices from multiplying the per-batch limits into an
/// unbounded allocation before command generation.
pub const GUI_MESH_MAX_FRAME_PAYLOAD_BYTES: u64 = 128 * 1024 * 1024;
/// Maximum dimension of a Rust-owned GUI item offscreen raster.
pub const GUI_MESH_MAX_OFFSCREEN_AXIS: u32 = 4096;
const GUI_MESH_FRAME_UNIFORM_BYTES: usize = 48;
const GUI_MESH_COMPOSITE_UNIFORM_BYTES: usize = 80;
/// Conservative dynamic-UBO alignment valid for both backend lowerings.
pub const GUI_MESH_COMPOSITE_UNIFORM_STRIDE: u64 = 256;
const GUI_MESH_MAX_COMPOSITE_UNIFORM_BYTES: u64 =
    GUI_MESH_MAX_BATCHES as u64 * GUI_MESH_COMPOSITE_UNIFORM_STRIDE;
pub(crate) const GUI_MESH_MAX_VERTEX_BYTES: u64 =
    (GUI_MESH_MAX_VERTICES * GUI_MESH_GPU_VERTEX_BYTES) as u64;
pub(crate) const GUI_MESH_MAX_INDEX_BYTES: u64 =
    (GUI_MESH_MAX_INDICES * std::mem::size_of::<u32>()) as u64;

const GUI_MESH_VERTEX_SHADER_OPENGL: &[u8] = br#"#version 430 core
layout(std430) readonly buffer GuiMeshVertices { vec4 vertex_words[]; };
layout(std140) uniform GuiMeshFrame { vec4 raster_extent; vec4 light0; vec4 light1; };
out vec2 v_uv;
out vec4 v_color;
out vec3 v_normal;
void main() {
    int base = gl_VertexID * 3;
    vec4 position_u = vertex_words[base];
    vec4 uv_color_rg = vertex_words[base + 1];
    vec4 color_ba_normal = vertex_words[base + 2];
    float top_left_y = 1.0 - (position_u.y / raster_extent.y) * 2.0;
    // Standard3dItemRenderer's owned orthographic projection is
    // setOrtho(0, width, height, 0, -1000, 1000, false): model-space Z is
    // negated into OpenGL clip depth. Keep that convention in the OpenGL
    // lowering so the nearest item face wins its private depth test.
    gl_Position = vec4((position_u.x / raster_extent.x) * 2.0 - 1.0, top_left_y, -position_u.z / 1000.0, 1.0);
    v_uv = vec2(position_u.w, uv_color_rg.x);
    v_color = vec4(uv_color_rg.y, uv_color_rg.z, uv_color_rg.w, color_ba_normal.x);
    v_normal = color_ba_normal.yzw;
}
"#;

const GUI_MESH_VERTEX_SHADER_VULKAN: &[u8] = br#"#version 450
layout(set = 0, binding = 0, std430) readonly buffer GuiMeshVertices { vec4 vertex_words[]; };
layout(set = 0, binding = 1, std140) uniform GuiMeshFrame { vec4 raster_extent; vec4 light0; vec4 light1; };
layout(location = 0) out vec2 v_uv;
layout(location = 1) out vec4 v_color;
layout(location = 2) out vec3 v_normal;
void main() {
    int base = gl_VertexIndex * 3;
    vec4 position_u = vertex_words[base];
    vec4 uv_color_rg = vertex_words[base + 1];
    vec4 color_ba_normal = vertex_words[base + 2];
    float top_left_y = 1.0 - (position_u.y / raster_extent.y) * 2.0;
    // The copied vanilla PIP projection above is authored in OpenGL's
    // [-1, 1] clip-depth convention. Vulkan consumes [0, 1], so convert it
    // after preserving the same negative model-space Z scale.
    float vanilla_pip_clip_depth = -position_u.z / 1000.0;
    gl_Position = vec4((position_u.x / raster_extent.x) * 2.0 - 1.0, top_left_y, vanilla_pip_clip_depth * 0.5 + 0.5, 1.0);
    v_uv = vec2(position_u.w, uv_color_rg.x);
    v_color = vec4(uv_color_rg.y, uv_color_rg.z, uv_color_rg.w, color_ba_normal.x);
    v_normal = color_ba_normal.yzw;
}
"#;

const GUI_MESH_FRAGMENT_SHADER_OPENGL: &[u8] = br#"#version 430 core
layout(binding = 2) uniform sampler2D Sampler0;
layout(std140) uniform GuiMeshFrame { vec4 raster_extent; vec4 light0; vec4 light1; };
in vec2 v_uv;
in vec4 v_color;
in vec3 v_normal;
out vec4 out_color;
void main() {
    vec4 color = texture(Sampler0, v_uv) * v_color;
    if (color.a <= raster_extent.z) discard;
    if (raster_extent.w > 0.5) {
        vec3 normal = raster_extent.w > 1.5 ? v_normal : normalize(v_normal);
        if (raster_extent.w > 2.5 && !gl_FrontFacing) normal = -normal;
        vec2 light = max(vec2(0.0), vec2(dot(light0.xyz, normal), dot(light1.xyz, normal)));
        color.rgb *= min(1.0, (light.x + light.y) * 0.6 + 0.4);
    }
    out_color = color;
}
"#;

const GUI_MESH_FRAGMENT_SHADER_VULKAN: &[u8] = br#"#version 450
layout(set = 0, binding = 2) uniform texture2D GuiMeshTexture;
layout(set = 0, binding = 3) uniform sampler GuiMeshSampler;
layout(set = 0, binding = 1, std140) uniform GuiMeshFrame { vec4 raster_extent; vec4 light0; vec4 light1; };
layout(location = 0) in vec2 v_uv;
layout(location = 1) in vec4 v_color;
layout(location = 2) in vec3 v_normal;
layout(location = 0) out vec4 out_color;
void main() {
    vec4 color = texture(sampler2D(GuiMeshTexture, GuiMeshSampler), v_uv) * v_color;
    if (color.a <= raster_extent.z) discard;
    if (raster_extent.w > 0.5) {
        vec3 normal = raster_extent.w > 1.5 ? v_normal : normalize(v_normal);
        if (raster_extent.w > 2.5 && !gl_FrontFacing) normal = -normal;
        vec2 light = max(vec2(0.0), vec2(dot(light0.xyz, normal), dot(light1.xyz, normal)));
        color.rgb *= min(1.0, (light.x + light.y) * 0.6 + 0.4);
    }
    out_color = color;
}
"#;

// The title panorama is semantic background imagery, not a copied 3D item.
// It deliberately shares the bounded mesh stream and Rust-owned image cache
// with GUI meshes, but must not inherit the item alpha-cutoff or directional
// lighting policy.  In particular, a cube-face edge with a transparent texel
// must not punch a hole into the title background.
const GUI_PANORAMA_VERTEX_SHADER_OPENGL: &[u8] = br#"#version 430 core
layout(std430) readonly buffer GuiMeshVertices { vec4 vertex_words[]; };
layout(std140) uniform GuiMeshFrame { vec4 raster_extent; vec4 light0; vec4 light1; };
out vec3 v_ray;
void main() {
    int base = gl_VertexID * 3;
    vec4 position_u = vertex_words[base];
    vec4 uv_color_rg = vertex_words[base + 1];
    float top_left_y = 1.0 - (position_u.y / raster_extent.y) * 2.0;
    gl_Position = vec4((position_u.x / raster_extent.x) * 2.0 - 1.0, top_left_y, 0.0, 1.0);
    v_ray = vec3(position_u.z, position_u.w, uv_color_rg.x);
}
"#;

const GUI_PANORAMA_VERTEX_SHADER_VULKAN: &[u8] = br#"#version 450
layout(set = 0, binding = 0, std430) readonly buffer GuiMeshVertices { vec4 vertex_words[]; };
layout(set = 0, binding = 1, std140) uniform GuiMeshFrame { vec4 raster_extent; vec4 light0; vec4 light1; };
layout(location = 0) out vec3 v_ray;
void main() {
    int base = gl_VertexIndex * 3;
    vec4 position_u = vertex_words[base];
    vec4 uv_color_rg = vertex_words[base + 1];
    float top_left_y = 1.0 - (position_u.y / raster_extent.y) * 2.0;
    gl_Position = vec4((position_u.x / raster_extent.x) * 2.0 - 1.0, top_left_y, 0.5, 1.0);
    v_ray = vec3(position_u.z, position_u.w, uv_color_rg.x);
}
"#;

const GUI_PANORAMA_FRAGMENT_SHADER_OPENGL: &[u8] = br#"#version 430 core
uniform sampler2D Sampler0;
in vec3 v_ray;
out vec4 out_color;
void main() {
    vec3 ray = normalize(v_ray);
    vec3 axis = abs(ray);
    float face;
    float u;
    float v;
    if (axis.x >= axis.y && axis.x >= axis.z) {
        float scale = 0.5 / axis.x;
        face = ray.x > 0.0 ? 0.0 : 1.0;
        u = (ray.x > 0.0 ? -ray.z : ray.z) * scale + 0.5;
        v = -ray.y * scale + 0.5;
    } else if (axis.y >= axis.z) {
        float scale = 0.5 / axis.y;
        face = ray.y > 0.0 ? 2.0 : 3.0;
        u = ray.x * scale + 0.5;
        v = (ray.y > 0.0 ? ray.z : -ray.z) * scale + 0.5;
    } else {
        float scale = 0.5 / axis.z;
        face = ray.z > 0.0 ? 4.0 : 5.0;
        u = (ray.z > 0.0 ? ray.x : -ray.x) * scale + 0.5;
        v = -ray.y * scale + 0.5;
    }
    // Match Frozen's panorama fragment shader exactly: its continuous sampler
    // is allowed to filter at the stacked-face edge.  Insetting this lookup by
    // half a texel changes the title image at every cube-face boundary.
    vec2 atlas_uv = vec2(clamp(u, 0.0, 1.0), (face + clamp(v, 0.0, 1.0)) / 6.0);
    out_color = texture(Sampler0, atlas_uv);
}
"#;

const GUI_PANORAMA_FRAGMENT_SHADER_VULKAN: &[u8] = br#"#version 450
layout(set = 0, binding = 2) uniform texture2D GuiMeshTexture;
layout(set = 0, binding = 3) uniform sampler GuiMeshSampler;
layout(location = 0) in vec3 v_ray;
layout(location = 0) out vec4 out_color;
void main() {
    vec3 ray = normalize(v_ray);
    vec3 axis = abs(ray);
    float face;
    float u;
    float v;
    if (axis.x >= axis.y && axis.x >= axis.z) {
        float scale = 0.5 / axis.x;
        face = ray.x > 0.0 ? 0.0 : 1.0;
        u = (ray.x > 0.0 ? -ray.z : ray.z) * scale + 0.5;
        v = -ray.y * scale + 0.5;
    } else if (axis.y >= axis.z) {
        float scale = 0.5 / axis.y;
        face = ray.y > 0.0 ? 2.0 : 3.0;
        u = ray.x * scale + 0.5;
        v = (ray.y > 0.0 ? ray.z : -ray.z) * scale + 0.5;
    } else {
        float scale = 0.5 / axis.z;
        face = ray.z > 0.0 ? 4.0 : 5.0;
        u = (ray.z > 0.0 ? ray.x : -ray.x) * scale + 0.5;
        v = -ray.y * scale + 0.5;
    }
    // Match Frozen's panorama fragment shader exactly: its continuous sampler
    // is allowed to filter at the stacked-face edge.  Insetting this lookup by
    // half a texel changes the title image at every cube-face boundary.
    vec2 atlas_uv = vec2(clamp(u, 0.0, 1.0), (face + clamp(v, 0.0, 1.0)) / 6.0);
    out_color = texture(sampler2D(GuiMeshTexture, GuiMeshSampler), atlas_uv);
}
"#;

const GUI_MESH_COMPOSITE_VERTEX_SHADER_OPENGL: &[u8] = br#"#version 430 core
layout(std140) uniform GuiMeshComposite {
    vec4 pose_linear;
    vec4 pose_translation_viewport;
    vec4 bounds;
    vec4 uv_region;
    vec4 clip_rect;
};
out vec2 v_uv;
out vec2 v_pixel;
const vec2 corner[6] = vec2[6](
    vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0),
    vec2(1.0, 1.0), vec2(0.0, 1.0), vec2(0.0, 0.0)
);
void main() {
    vec2 local = mix(bounds.xy, bounds.zw, corner[gl_VertexID]);
    vec2 pixel = vec2(
        pose_linear.x * local.x + pose_linear.z * local.y + pose_translation_viewport.x,
        pose_linear.y * local.x + pose_linear.w * local.y + pose_translation_viewport.y
    );
    gl_Position = vec4(
        (pixel.x / pose_translation_viewport.z) * 2.0 - 1.0,
        1.0 - (pixel.y / pose_translation_viewport.w) * 2.0,
        0.0,
        1.0
    );
    v_pixel = pixel;
    // Standard3dItemRenderer blits its private PIP target with V increasing
    // from the top edge. This owned target already has that orientation after
    // the raster pass, so applying the generic PIP V inversion here turns the
    // completed item model upside down.
    v_uv = uv_region.xy + corner[gl_VertexID] * uv_region.zw;
}
"#;

const GUI_MESH_COMPOSITE_VERTEX_SHADER_VULKAN: &[u8] = br#"#version 450
layout(set = 0, binding = 0, std140) uniform GuiMeshComposite {
    vec4 pose_linear;
    vec4 pose_translation_viewport;
    vec4 bounds;
    vec4 uv_region;
    vec4 clip_rect;
};
layout(location = 0) out vec2 v_uv;
layout(location = 1) out vec2 v_pixel;
const vec2 corner[6] = vec2[6](
    vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0),
    vec2(1.0, 1.0), vec2(0.0, 1.0), vec2(0.0, 0.0)
);
void main() {
    vec2 local = mix(bounds.xy, bounds.zw, corner[gl_VertexIndex]);
    vec2 pixel = vec2(
        pose_linear.x * local.x + pose_linear.z * local.y + pose_translation_viewport.x,
        pose_linear.y * local.x + pose_linear.w * local.y + pose_translation_viewport.y
    );
    gl_Position = vec4(
        (pixel.x / pose_translation_viewport.z) * 2.0 - 1.0,
        1.0 - (pixel.y / pose_translation_viewport.w) * 2.0,
        0.0,
        1.0
    );
    v_pixel = pixel;
    // Keep the semantic Standard3dItemRenderer PIP orientation identical on
    // Vulkan and OpenGL. Backend coordinate conversion ends at rasterization;
    // the sampled owned image is not a generic GUI blit source.
    v_uv = uv_region.xy + corner[gl_VertexIndex] * uv_region.zw;
}
"#;

const GUI_MESH_COMPOSITE_FRAGMENT_SHADER_OPENGL: &[u8] = br#"#version 430 core
uniform sampler2D Sampler0;
layout(std140) uniform GuiMeshComposite { vec4 pose_linear; vec4 pose_translation_viewport; vec4 bounds; vec4 uv_region; vec4 clip_rect; };
in vec2 v_uv;
in vec2 v_pixel;
out vec4 out_color;
void main() {
    if (v_pixel.x < clip_rect.x || v_pixel.y < clip_rect.y || v_pixel.x >= clip_rect.z || v_pixel.y >= clip_rect.w) discard;
    vec4 color = texture(Sampler0, v_uv);
    if (color.a <= 0.0) discard;
    out_color = color;
}
"#;

const GUI_MESH_COMPOSITE_FRAGMENT_SHADER_VULKAN: &[u8] = br#"#version 450
layout(set = 0, binding = 1) uniform texture2D GuiMeshColor;
layout(set = 0, binding = 2) uniform sampler GuiMeshColorSampler;
layout(set = 0, binding = 0, std140) uniform GuiMeshComposite { vec4 pose_linear; vec4 pose_translation_viewport; vec4 bounds; vec4 uv_region; vec4 clip_rect; };
layout(location = 0) in vec2 v_uv;
layout(location = 1) in vec2 v_pixel;
layout(location = 0) out vec4 out_color;
void main() {
    if (v_pixel.x < clip_rect.x || v_pixel.y < clip_rect.y || v_pixel.x >= clip_rect.z || v_pixel.y >= clip_rect.w) discard;
    vec4 color = texture(sampler2D(GuiMeshColor, GuiMeshColorSampler), v_uv);
    if (color.a <= 0.0) discard;
    out_color = color;
}
"#;

#[cfg(test)]
pub(crate) fn vulkan_shader_sources_for_backend_test() -> (&'static str, &'static str) {
    vulkan_shader_sources_for_material(GuiMeshMaterialMode::Opaque)
}

#[cfg(test)]
fn vulkan_shader_sources_for_material(
    material_mode: GuiMeshMaterialMode,
) -> (&'static str, &'static str) {
    let (vertex, fragment) = gui_mesh_shader_sources(BackendApi::Vulkan, material_mode);
    (
        std::str::from_utf8(vertex).expect("GUI mesh Vulkan vertex source is UTF-8"),
        std::str::from_utf8(fragment).expect("GUI mesh Vulkan fragment source is UTF-8"),
    )
}

#[cfg(test)]
pub(crate) fn vulkan_panorama_shader_sources_for_backend_test() -> (&'static str, &'static str) {
    vulkan_shader_sources_for_material(GuiMeshMaterialMode::Panorama)
}

#[cfg(test)]
pub(crate) fn opengl_panorama_shader_sources_for_backend_test() -> (&'static str, &'static str) {
    let (vertex, fragment) =
        gui_mesh_shader_sources(BackendApi::OpenGl, GuiMeshMaterialMode::Panorama);
    (
        std::str::from_utf8(vertex).expect("GUI panorama OpenGL vertex source is UTF-8"),
        std::str::from_utf8(fragment).expect("GUI panorama OpenGL fragment source is UTF-8"),
    )
}

fn gui_mesh_shader_sources(
    api: BackendApi,
    material_mode: GuiMeshMaterialMode,
) -> (&'static [u8], &'static [u8]) {
    let fragment_is_panorama = material_mode == GuiMeshMaterialMode::Panorama;
    match api {
        BackendApi::OpenGl => (
            if fragment_is_panorama {
                GUI_PANORAMA_VERTEX_SHADER_OPENGL
            } else {
                GUI_MESH_VERTEX_SHADER_OPENGL
            },
            if fragment_is_panorama {
                GUI_PANORAMA_FRAGMENT_SHADER_OPENGL
            } else {
                GUI_MESH_FRAGMENT_SHADER_OPENGL
            },
        ),
        BackendApi::Vulkan | BackendApi::Mock => (
            if fragment_is_panorama {
                GUI_PANORAMA_VERTEX_SHADER_VULKAN
            } else {
                GUI_MESH_VERTEX_SHADER_VULKAN
            },
            if fragment_is_panorama {
                GUI_PANORAMA_FRAGMENT_SHADER_VULKAN
            } else {
                GUI_MESH_FRAGMENT_SHADER_VULKAN
            },
        ),
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum GuiMeshMaterialMode {
    Opaque,
    Cutout,
    Translucent,
    Glint,
    /// Fullscreen panorama image sampling. This is deliberately separate
    /// from opaque item geometry: vanilla's panorama pipeline has no culling
    /// or depth attachment interaction.
    Panorama,
    /// Vanilla entity_no_outline layers: two-sided alpha, no depth writes,
    /// and front/back directional lighting. Not ordinary translucent items.
    ModelOverlay,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum GuiMeshLightingMode {
    Flat,
    Block,
    /// Ordinary inventory models, with normals in the Y-down GUI space.
    /// Distinct from upright picture-in-picture model lighting.
    InventoryBlock,
    /// Vanilla gui_light=front models: ITEMS_FLAT directional lights, not unlit.
    FrontModel,
}

/// One copied model vertex with the stable vanilla signed-i8 normal encoding.
/// Native item layouts carry original model-space normals for Rust to transform;
/// explicit legacy mesh requests already carry item-lighting-space normals and
/// must not receive a second normal transform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuiMeshVertex {
    pub position: [f32; 3],
    pub atlas_uv: [f32; 2],
    pub local_uv: [f32; 2],
    pub color_argb: u32,
    pub normal_packed: u32,
    /// Original baked face: 0 absent, 1..6 down/up/north/south/west/east.
    pub source_face: u32,
    /// Original layer foil type: 0 none, 1 standard. Not a lighting policy.
    pub source_foil_type: u32,
}

/// Immutable semantic poses for sheeted GUI foil. These are the unscaled model
/// and normal poses used to emit vertices, never inverse/decal texture matrices.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuiDecalFoilProjection {
    pub native_item_layout: bool,
    pub model_pose: [f32; 16],
    pub normal_pose: [f32; 9],
}

impl GuiDecalFoilProjection {
    pub(crate) fn decode(mode: u32, model_pose: [f32;16], normal_pose: [f32;9]) -> GalResult<Option<Self>> {
        match mode {
            0 if model_pose.iter().chain(normal_pose.iter()).all(|v| v.to_bits() == 0) => Ok(None),
            1 => {
                let value = Self { native_item_layout: false, model_pose, normal_pose };
                value.prepare(None)?;
                Ok(Some(value))
            }
            2 if model_pose.iter().chain(normal_pose.iter()).all(|v| v.to_bits() == 0) =>
                Ok(Some(Self { native_item_layout: true, model_pose, normal_pose })),
            _ => Err(GalError::invalid_argument("invalid or noncanonical GUI decal foil projection")),
        }
    }
    fn prepare(self, native_model: Option<[f32;16]>) -> GalResult<super::special_item_foil::SpecialFoilProjection> {
        if self.native_item_layout {
            if self.model_pose.iter().chain(self.normal_pose.iter()).any(|v| v.to_bits()!=0) {
                return Err(GalError::invalid_argument("native item decal layout cannot carry caller raster poses"));
            }
            return super::special_item_foil::SpecialFoilProjection::from_native_gui_model(
                native_model.ok_or_else(|| GalError::invalid_argument("native decal layout requires native item raster semantics"))?);
        }
        if native_model.is_some() {
            return Err(GalError::invalid_argument("emitted-space decal poses cannot be used as model-space item poses"));
        }
        super::special_item_foil::SpecialFoilProjection::new(self.model_pose, self.normal_pose,
            super::special_item_foil::FoilDisplayContext::Gui)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GuiItemCache {
    pub identity: u64,
    pub animated: bool,
}

impl GuiItemCache {
    pub fn decode(identity: u64, mode: u32) -> GalResult<Option<Self>> {
        match (identity, mode) {
            (0, 0) => Ok(None),
            (identity, 1 | 2) if identity != 0 => Ok(Some(Self { identity, animated: mode == 2 })),
            _ => Err(GalError::invalid_argument("invalid semantic GUI item cache identity or mode")),
        }
    }
}

/// A material-homogeneous indexed mesh for one GUI item layer. `asset_id`
/// refers to a Rust-owned raw image asset, never a Minecraft atlas object.
#[derive(Clone, Debug, PartialEq)]
pub struct GuiMeshBatchRequest {
    pub item_cache: Option<GuiItemCache>,
    /// Private native contract: copied model-space bounds, original normals,
    /// and GUI scale. No caller-selected offscreen extent/guard/raster matrix.
    pub block_item_raster: Option<GuiBlockItemRaster>,
    /// Zero for explicit meshes; positive for a native 16-unit flat item cell.
    pub item_raster_scale: u32,
    /// Resolved solely from the Rust-owned frame lightmap, never transported.
    pub item_lighting: Option<GuiFlatItemLighting>,
    /// When present, vertices contain original atlas UVs and Rust prepares
    /// standard item foil. Absent for ordinary or explicitly prepared meshes.
    pub item_foil: Option<GuiItemFoil>,
    /// Optional sheeted coordinate generation before native glint animation.
    /// Requires item foil; flat GUI callsites request the native-owned item layout.
    pub decal_foil: Option<GuiDecalFoilProjection>,
    pub stratum: u32,
    /// Ordering within one item PIP raster. Every layer of a GUI item shares
    /// its scheduler sequence and composes only after the final layer.
    pub layer_index: u32,
    pub sequence: u64,
    pub asset_id: u64,
    pub material_mode: GuiMeshMaterialMode,
    pub lighting_mode: GuiMeshLightingMode,
    pub alpha_cutoff: f32,
    /// Vanilla-resolved item-layer transform copied before FFI.
    pub model_transform: [f32; 16],
    /// GUI affine pose expressed as m00, m01, m10, m11, m20, m21.
    pub gui_pose: [f32; 6],
    /// Logical GUI placement bounds: left, top, right, bottom.
    pub bounds: [i32; 4],
    pub gui_extent: [u32; 2],
    pub projection_extent: [f32; 2],
    /// Rust-owned offscreen raster extent, including native guard padding.
    /// This is deliberately distinct from the final GUI viewport.
    pub render_extent: [u32; 2],
    /// Copied PIP guard band. Composition excludes it from the visible GUI
    /// rectangle while rasterization retains it for filtered edge safety.
    pub guard_pixels: u32,
    pub clip_mode: u32,
    pub clip_left: i32,
    pub clip_top: i32,
    pub clip_width: i32,
    pub clip_height: i32,
    pub vertices: Vec<GuiMeshVertex>,
    pub indices: Vec<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuiBlockItemRaster {
    pub model_min: [f64; 3],
    pub model_max: [f64; 3],
    pub gui_scale: u32,
    pub oversized_gui: bool,
}

impl GuiBlockItemRaster {
    pub(crate) fn decode(gui_scale: u32, bounds: [f64;6], mode: u32) -> GalResult<Option<Self>> {
        if gui_scale == 0 {
            if mode != 0 || bounds.iter().any(|v| v.to_bits() != 0) {
                return Err(GalError::invalid_argument("absent block layout requires canonical zero bounds"));
            }
            return Ok(None);
        }
        let model_min=[bounds[0],bounds[1],bounds[2]];
        let model_max=[bounds[3],bounds[4],bounds[5]];
        if !matches!(mode,1|2) { return Err(GalError::invalid_argument("invalid block item layout mode")); }
        super::gui_item_layout::GuiItemRasterLayout::block_bounds(model_min,model_max,gui_scale)?;
        Ok(Some(Self {model_min,model_max,gui_scale,oversized_gui:mode==2}))
    }

    fn oversized_layout(self, bounds: [i32;4]) -> GalResult<Option<(super::gui_item_layout::GuiItemRasterLayout,[i32;4])>> {
        if !self.oversized_gui { return Ok(None); }
        if bounds[2] as i64-bounds[0] as i64 != 16 || bounds[3] as i64-bounds[1] as i64 != 16 {
            return Err(GalError::invalid_argument("native oversized item requires original logical item box"));
        }
        super::gui_item_layout::GuiItemRasterLayout::oversized_gui(
            self.model_min,self.model_max,self.gui_scale,[bounds[0],bounds[1]])
    }
}

/// Backend-neutral, owned vertex data produced after the Java-owned source
/// snapshot is validated. The future GUI mesh pass may choose its private
/// buffer layout, but it must consume these copied semantics rather than a
/// Java model or PIP object.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuiMeshPreparedVertex {
    pub position: [f32; 3],
    pub local_uv: [f32; 2],
    pub color: [f32; 4],
    pub normal: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct GuiMeshPreparedDraw {
    pub item_cache: Option<GuiItemCache>,
    pub stratum: u32,
    pub layer_index: u32,
    pub sequence: u64,
    pub asset_id: u64,
    pub material_mode: GuiMeshMaterialMode,
    /// The copied item transform can contain a reflection (vanilla's GUI PIP
    /// pose does). Keep the resulting winding explicit so back-face culling
    /// remains correct instead of exposing the model interior.
    pub front_face: super::resources::FrontFace,
    pub lighting_mode: GuiMeshLightingMode,
    pub alpha_cutoff: f32,
    pub gui_pose: [f32; 6],
    pub bounds: [i32; 4],
    pub gui_extent: [u32; 2],
    pub projection_extent: [f32; 2],
    pub render_extent: [u32; 2],
    pub guard_pixels: u32,
    pub clip_mode: u32,
    pub clip_left: i32,
    pub clip_top: i32,
    pub clip_width: i32,
    pub clip_height: i32,
    pub vertices: Vec<GuiMeshPreparedVertex>,
    pub indices: Vec<u32>,
}

/// Stable process-local identity for copied GUI geometry. Transform, clip,
/// and layer fields are intentionally excluded: those remain per-draw
/// uniforms, while this key permits immutable vertex/index residency across
/// frames without retaining Java objects or native pointers.
pub fn geometry_fingerprint(draw: &GuiMeshPreparedDraw) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    hasher.write_u64(draw.vertices.len() as u64);
    for vertex in &draw.vertices {
        for value in vertex.position {
            hasher.write_u32(value.to_bits());
        }
        for value in vertex.local_uv {
            hasher.write_u32(value.to_bits());
        }
        for value in vertex.color {
            hasher.write_u32(value.to_bits());
        }
        for value in vertex.normal {
            hasher.write_u32(value.to_bits());
        }
    }
    hasher.write_u64(draw.indices.len() as u64);
    for index in &draw.indices {
        hasher.write_u32(*index);
    }
    hasher.finish()
}

/// One non-overlapping range in a persistent GUI-mesh stream. A command list
/// may rasterize several quads that use the same texture; every draw must
/// retain its own bytes until GPU execution instead of overwriting offset zero
/// before the submission reaches the backend.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GuiMeshStreamRange {
    pub vertex_offset: u64,
    pub index_offset: u64,
}

/// Immutable GUI-mesh GPU program for one explicit raster contract. Texture
/// bindings and stream buffers remain per asset, so sharing this object never
/// aliases mutable draw state between semantic callsites.
#[derive(Clone, Copy, Debug)]
pub struct GuiMeshSharedProgram {
    pub vertex_shader: Handle,
    pub fragment_shader: Handle,
    pub resource_layout: Handle,
    pub pipeline_layout: Handle,
    pub pipeline: Handle,
}

impl GuiMeshSharedProgram {
    pub fn create(
        gal: &mut VulkanicGal,
        label: &str,
        color_format: ColorFormat,
        depth_format: Option<TextureFormat>,
        material_mode: GuiMeshMaterialMode,
        front_face: super::resources::FrontFace,
    ) -> GalResult<Self> {
        let mut created = Vec::new();
        let result = (|| -> GalResult<Self> {
            let (vertex_code, fragment_code) =
                gui_mesh_shader_sources(gal.capabilities().api, material_mode);
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
                label: format!("{label}.layout"),
                bindings: vec![
                    resource_binding_desc(0, ResourceBindingKind::StorageBuffer),
                    resource_binding_desc(1, ResourceBindingKind::UniformBuffer),
                    resource_binding_desc(2, ResourceBindingKind::SampledTexture),
                    resource_binding_desc(3, ResourceBindingKind::Sampler),
                ],
            })?;
            created.push(resource_layout);
            let pipeline_layout = gal.create_pipeline_layout(PipelineLayoutDesc {
                label: format!("{label}.pipeline-layout"),
                resource_layouts: vec![resource_layout],
            })?;
            created.push(pipeline_layout);
            let (cull_mode, blend, depth_compare, depth_write) =
                gui_mesh_raster_state(material_mode);
            let pipeline = gal.create_graphics_pipeline(GraphicsPipelineDesc {
                label: format!("{label}.pipeline"),
                layout: pipeline_layout,
                vertex_shader,
                fragment_shader,
                topology: PrimitiveTopology::Triangles,
                cull_mode,
                front_face,
                provoking_vertex: crate::render::vulkanic::resources::ProvokingVertex::Last,
                raster_y_direction: crate::render::vulkanic::resources::RasterYDirection::Up,
                blend,
                depth_compare,
                depth_write,
                depth_bias: None,
                color_formats: vec![color_format],
                depth_format,
                stencil: None,
            })?;
            created.push(pipeline);
            Ok(Self {
                vertex_shader,
                fragment_shader,
                resource_layout,
                pipeline_layout,
                pipeline,
            })
        })();
        if result.is_err() {
            for handle in created.into_iter().rev() {
                let _ = gal.destroy(handle);
            }
        }
        result
    }

    pub fn destroy(self, gal: &mut VulkanicGal) {
        for handle in [
            self.pipeline,
            self.pipeline_layout,
            self.resource_layout,
            self.fragment_shader,
            self.vertex_shader,
        ] {
            let _ = gal.destroy(handle);
        }
    }
}

/// Private Rust-owned pass objects for one GUI mesh asset. The texture view
/// and sampler come from the Rust GUI asset cache; Java never observes these
/// handles and no backend resource crosses FFI.
#[derive(Clone, Copy, Debug)]
pub struct GuiMeshPassResources {
    pub vertex_buffer: Handle,
    pub index_buffer: Handle,
    pub uniform_buffer: Handle,
    pub vertex_shader: Handle,
    pub fragment_shader: Handle,
    pub resource_layout: Handle,
    pub resource_set: Handle,
    pub pipeline_layout: Handle,
    pub pipeline: Handle,
    owned_program: Option<GuiMeshSharedProgram>,
}

impl GuiMeshPassResources {
    pub fn create(
        gal: &mut VulkanicGal,
        label: &str,
        color_format: ColorFormat,
        texture_view: Handle,
        sampler: Handle,
        material_mode: GuiMeshMaterialMode,
        front_face: super::resources::FrontFace,
    ) -> GalResult<Self> {
        let program =
            GuiMeshSharedProgram::create(
                gal,
                label,
                color_format,
                Some(TextureFormat::Depth32Float),
                material_mode,
                front_face,
            )?;
        match Self::create_with_shared_program(gal, label, texture_view, sampler, program) {
            Ok(mut resources) => {
                resources.owned_program = Some(program);
                Ok(resources)
            }
            Err(error) => {
                program.destroy(gal);
                Err(error)
            }
        }
    }

    /// Creates mutable asset resources which borrow an immutable program owned
    /// by the caller. The caller must destroy assets before that program.
    pub fn create_with_shared_program(
        gal: &mut VulkanicGal,
        label: &str,
        texture_view: Handle,
        sampler: Handle,
        program: GuiMeshSharedProgram,
    ) -> GalResult<Self> {
        let mut created = Vec::new();
        let result = (|| -> GalResult<Self> {
            let vertex_buffer = gal.create_buffer(BufferDesc {
                label: format!("{label}.vertices"),
                size: GUI_MESH_MAX_VERTEX_BYTES,
                memory: MemoryDomain::Upload,
                usages: vec![
                    BufferUsage::Storage,
                    BufferUsage::TransferDst,
                    BufferUsage::HostWrite,
                ],
            })?;
            created.push(vertex_buffer);
            let index_buffer = gal.create_buffer(BufferDesc {
                label: format!("{label}.indices"),
                size: GUI_MESH_MAX_INDEX_BYTES,
                memory: MemoryDomain::Upload,
                usages: vec![
                    BufferUsage::Index,
                    BufferUsage::TransferDst,
                    BufferUsage::HostWrite,
                ],
            })?;
            created.push(index_buffer);
            let uniform_buffer = gal.create_buffer(BufferDesc {
                label: format!("{label}.frame"),
                size: GUI_MESH_FRAME_UNIFORM_BYTES as u64,
                memory: MemoryDomain::Upload,
                usages: vec![
                    BufferUsage::Uniform,
                    BufferUsage::TransferDst,
                    BufferUsage::HostWrite,
                ],
            })?;
            created.push(uniform_buffer);
            let resource_set = gal.create_resource_set(ResourceSetDesc {
                label: format!("{label}.set"),
                layout: program.resource_layout,
                bindings: vec![
                    read_binding(0, vertex_buffer, ResourceBindingKind::StorageBuffer),
                    read_binding(1, uniform_buffer, ResourceBindingKind::UniformBuffer),
                    read_binding(2, texture_view, ResourceBindingKind::SampledTexture),
                    read_binding(3, sampler, ResourceBindingKind::Sampler),
                ],
            })?;
            created.push(resource_set);
            Ok(Self {
                vertex_buffer,
                index_buffer,
                uniform_buffer,
                vertex_shader: program.vertex_shader,
                fragment_shader: program.fragment_shader,
                resource_layout: program.resource_layout,
                resource_set,
                pipeline_layout: program.pipeline_layout,
                pipeline: program.pipeline,
                owned_program: None,
            })
        })();
        if result.is_err() {
            for handle in created.into_iter().rev() {
                let _ = gal.destroy(handle);
            }
        }
        result
    }

    pub fn append_draw(
        &self,
        target: GuiMeshOffscreenTarget,
        draw: &GuiMeshPreparedDraw,
        stream: GuiMeshStreamRange,
        clear: bool,
        operations: &mut Vec<CommandOp>,
    ) -> GalResult<()> {
        self.append_draw_internal(target, draw, stream, clear, true, operations)
    }

    /// Appends a draw that reuses an already uploaded immutable geometry range.
    /// Uniforms and raster/composite state remain explicit per draw; only the
    /// redundant vertex/index host writes are omitted.
    pub fn append_draw_reusing_geometry(
        &self,
        target: GuiMeshOffscreenTarget,
        draw: &GuiMeshPreparedDraw,
        stream: GuiMeshStreamRange,
        clear: bool,
        operations: &mut Vec<CommandOp>,
    ) -> GalResult<()> {
        self.append_draw_internal(target, draw, stream, clear, false, operations)
    }

    /// Rasterizes a semantic panorama directly into the acquired Rust frame
    /// target. Unlike item PIP meshes, Frozen's panorama is a native-resolution
    /// full-frame pass and has no intermediate texture to magnify afterwards.
    pub fn append_direct_frame_draw(
        &self,
        pass: Handle,
        target: Handle,
        color_attachment: Handle,
        depth_attachment: Option<Handle>,
        draw: &GuiMeshPreparedDraw,
        stream: GuiMeshStreamRange,
        write_geometry: bool,
        operations: &mut Vec<CommandOp>,
    ) -> GalResult<()> {
        if draw.material_mode != GuiMeshMaterialMode::Panorama {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "only the semantic panorama may bypass the private GUI mesh target",
            ));
        }
        if stream.vertex_offset % GUI_MESH_GPU_VERTEX_BYTES as u64 != 0
            || stream.index_offset % std::mem::size_of::<u32>() as u64 != 0
        {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "GUI mesh stream ranges must align to their vertex and index elements",
            ));
        }
        let vertex_bytes = packed_vertices(draw);
        let vertex_base = u32::try_from(stream.vertex_offset / GUI_MESH_GPU_VERTEX_BYTES as u64)
            .map_err(|_| GalError::ffi(StatusCode::InvalidArgument, "GUI mesh vertex stream offset exceeds u32 indices"))?;
        let index_bytes = packed_indices_with_base(&draw.indices, vertex_base)?;
        let vertex_end = stream.vertex_offset.checked_add(vertex_bytes.len() as u64)
            .ok_or_else(|| GalError::ffi(StatusCode::InvalidArgument, "GUI mesh vertex stream range overflows"))?;
        let index_end = stream.index_offset.checked_add(index_bytes.len() as u64)
            .ok_or_else(|| GalError::ffi(StatusCode::InvalidArgument, "GUI mesh index stream range overflows"))?;
        if vertex_end > GUI_MESH_MAX_VERTEX_BYTES || index_end > GUI_MESH_MAX_INDEX_BYTES {
            return Err(GalError::ffi(StatusCode::InvalidArgument, "GUI mesh draws exceed their persistent stream capacity"));
        }
        let mut buffers = vec![(self.uniform_buffer, TextureUsageState::ShaderRead)];
        if write_geometry {
            buffers.insert(0, (self.vertex_buffer, TextureUsageState::ShaderRead));
            buffers.insert(1, (self.index_buffer, TextureUsageState::IndexRead));
        }
        for (buffer, before) in buffers {
            operations.push(CommandOp::Barrier(buffer_barrier(buffer, before, TextureUsageState::TransferDst)));
        }
        if write_geometry {
            operations.push(CommandOp::HostWriteBuffer { buffer: self.vertex_buffer, offset: stream.vertex_offset, data: vertex_bytes });
            operations.push(CommandOp::HostWriteBuffer { buffer: self.index_buffer, offset: stream.index_offset, data: index_bytes });
        }
        // Panorama vertices are supplied in logical GUI coordinates, whereas
        // the target is the physical acquired frame. Clip-space conversion
        // intentionally follows the semantic coordinate domain.
        operations.push(CommandOp::HostWriteBuffer {
            buffer: self.uniform_buffer,
            offset: 0,
            data: draw_frame_uniform_bytes(draw, draw.projection_extent),
        });
        operations.push(CommandOp::Barrier(buffer_barrier(self.vertex_buffer, TextureUsageState::TransferDst, TextureUsageState::ShaderRead)));
        operations.push(CommandOp::Barrier(buffer_barrier(self.index_buffer, TextureUsageState::TransferDst, TextureUsageState::IndexRead)));
        operations.push(CommandOp::Barrier(buffer_barrier(self.uniform_buffer, TextureUsageState::TransferDst, TextureUsageState::ShaderRead)));
        operations.push(CommandOp::BeginPass {
            pass,
            target,
            colors: vec![PassAttachment { view: color_attachment, load_op: AttachmentLoadOp::Load, store_op: AttachmentStoreOp::Store, clear_color: None }],
            depth_stencil: depth_attachment.map(|view| PassAttachment { view, load_op: AttachmentLoadOp::Load, store_op: AttachmentStoreOp::Store, clear_color: None }),
        });
        operations.push(CommandOp::BindGraphicsPipeline(self.pipeline));
        operations.push(CommandOp::BindResourceSet { pipeline_layout: self.pipeline_layout, set_index: 0, set: self.resource_set, dynamic_offsets: Vec::new() });
        operations.push(CommandOp::SetIndexBuffer { buffer: self.index_buffer, offset: stream.index_offset, index_type: IndexType::U32 });
        operations.push(CommandOp::DrawIndexed { indices: draw.indices.len() as u32, instances: 1 });
        operations.push(CommandOp::EndPass);
        Ok(())
    }

    fn append_draw_internal(
        &self,
        target: GuiMeshOffscreenTarget,
        draw: &GuiMeshPreparedDraw,
        stream: GuiMeshStreamRange,
        clear: bool,
        write_geometry: bool,
        operations: &mut Vec<CommandOp>,
    ) -> GalResult<()> {
        if draw.render_extent != [target.extent.width, target.extent.height] {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "GUI mesh draw offscreen extent does not match its Rust-owned target",
            ));
        }
        // The raster target is sampled by the preceding item's composite pass.
        // Only the first layer transitions it back to attachment-write ownership;
        // consecutive layers remain in COLOR_ATTACHMENT_OPTIMAL until the single
        // composite pass below. A newly staged target is still UNDEFINED, while a
        // cached target was left in SHADER_READ_ONLY by that prior composite.
        if clear {
            operations.push(CommandOp::Barrier(ResourceBarrier {
                resource: target.color,
                subresources: None,
                before: if target.initialized {
                    TextureUsageState::ShaderRead
                } else {
                    TextureUsageState::Undefined
                },
                after: TextureUsageState::ColorAttachment,
                src_queue: QueueClass::Graphics,
                dst_queue: QueueClass::Graphics,
            }));
            // The depth attachment is a separate explicit resource. Clearing
            // it in BeginPass does not transition an UNDEFINED Vulkan image.
            operations.push(CommandOp::Barrier(ResourceBarrier {
                resource: target.depth,
                subresources: None,
                before: if target.initialized {
                    TextureUsageState::DepthStencilAttachment
                } else {
                    TextureUsageState::Undefined
                },
                after: TextureUsageState::DepthStencilAttachment,
                src_queue: QueueClass::Graphics,
                dst_queue: QueueClass::Graphics,
            }));
        }
        if stream.vertex_offset % GUI_MESH_GPU_VERTEX_BYTES as u64 != 0
            || stream.index_offset % std::mem::size_of::<u32>() as u64 != 0
        {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "GUI mesh stream ranges must align to their vertex and index elements",
            ));
        }
        let vertex_bytes = packed_vertices(draw);
        let vertex_base = u32::try_from(stream.vertex_offset / GUI_MESH_GPU_VERTEX_BYTES as u64)
            .map_err(|_| {
                GalError::ffi(
                    StatusCode::InvalidArgument,
                    "GUI mesh vertex stream offset exceeds u32 indices",
                )
            })?;
        let index_bytes = packed_indices_with_base(&draw.indices, vertex_base)?;
        let vertex_end = stream
            .vertex_offset
            .checked_add(vertex_bytes.len() as u64)
            .ok_or_else(|| {
                GalError::ffi(
                    StatusCode::InvalidArgument,
                    "GUI mesh vertex stream range overflows",
                )
            })?;
        let index_end = stream
            .index_offset
            .checked_add(index_bytes.len() as u64)
            .ok_or_else(|| {
                GalError::ffi(
                    StatusCode::InvalidArgument,
                    "GUI mesh index stream range overflows",
                )
            })?;
        if vertex_end > GUI_MESH_MAX_VERTEX_BYTES || index_end > GUI_MESH_MAX_INDEX_BYTES {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "GUI mesh draws exceed their persistent stream capacity",
            ));
        }
        let mut buffers = vec![(self.uniform_buffer, TextureUsageState::ShaderRead)];
        if write_geometry {
            buffers.insert(0, (self.vertex_buffer, TextureUsageState::ShaderRead));
            // Persistent GUI index storage was last consumed by the index
            // input stage, not by a shader. Preserve that explicit state so
            // the upload barrier also synchronizes the prior indexed draw.
            buffers.insert(1, (self.index_buffer, TextureUsageState::IndexRead));
        }
        for (buffer, before) in buffers {
            operations.push(CommandOp::Barrier(buffer_barrier(
                buffer,
                before,
                TextureUsageState::TransferDst,
            )));
        }
        if write_geometry {
            operations.push(CommandOp::HostWriteBuffer {
                buffer: self.vertex_buffer,
                offset: stream.vertex_offset,
                data: vertex_bytes,
            });
            operations.push(CommandOp::HostWriteBuffer {
                buffer: self.index_buffer,
                offset: stream.index_offset,
                data: index_bytes,
            });
        }
        operations.push(CommandOp::HostWriteBuffer {
            buffer: self.uniform_buffer,
            offset: 0,
            data: draw_frame_uniform_bytes(draw, draw.render_extent.map(|axis| axis as f32)),
        });
        operations.push(CommandOp::Barrier(buffer_barrier(
            self.vertex_buffer,
            TextureUsageState::TransferDst,
            TextureUsageState::ShaderRead,
        )));
        operations.push(CommandOp::Barrier(buffer_barrier(
            self.index_buffer,
            TextureUsageState::TransferDst,
            TextureUsageState::IndexRead,
        )));
        operations.push(CommandOp::Barrier(buffer_barrier(
            self.uniform_buffer,
            TextureUsageState::TransferDst,
            TextureUsageState::ShaderRead,
        )));
        operations.push(CommandOp::BeginPass {
            pass: target.pass,
            target: target.target,
            colors: vec![PassAttachment {
                view: target.color_view,
                load_op: if clear {
                    AttachmentLoadOp::Clear
                } else {
                    AttachmentLoadOp::Load
                },
                store_op: AttachmentStoreOp::Store,
                clear_color: clear.then_some(ClearColor {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.0,
                }),
            }],
            depth_stencil: Some(PassAttachment {
                view: target.depth_view,
                load_op: if clear {
                    AttachmentLoadOp::Clear
                } else {
                    AttachmentLoadOp::Load
                },
                store_op: AttachmentStoreOp::Store,
                clear_color: None,
            }),
        });
        operations.push(CommandOp::BindGraphicsPipeline(self.pipeline));
        operations.push(CommandOp::BindResourceSet {
            pipeline_layout: self.pipeline_layout,
            set_index: 0,
            set: self.resource_set,
            dynamic_offsets: Vec::new(),
        });
        operations.push(CommandOp::SetIndexBuffer {
            buffer: self.index_buffer,
            offset: stream.index_offset,
            index_type: IndexType::U32,
        });
        operations.push(CommandOp::DrawIndexed {
            indices: draw.indices.len() as u32,
            instances: 1,
        });
        operations.push(CommandOp::EndPass);
        Ok(())
    }

    pub fn destroy_asset_resources(self, gal: &mut VulkanicGal) {
        for handle in [
            self.resource_set,
            self.uniform_buffer,
            self.index_buffer,
            self.vertex_buffer,
        ] {
            let _ = gal.destroy(handle);
        }
    }

    pub fn destroy(self, gal: &mut VulkanicGal) {
        self.destroy_asset_resources(gal);
        if let Some(program) = self.owned_program {
            program.destroy(gal);
        }
    }
}

/// The standard 3D item PIP route accepts vanilla's SOLID, CUTOUT, and
/// TRANSLUCENT item layers. Their alpha threshold is explicit per draw, while their
/// fixed-function policy is shared: depth-tested, back-face-culled draws.
/// Translucent layers additionally use the explicit GAL alpha blend equation;
/// depth writes remain enabled to match vanilla's item-entity translucent
/// render type ordering inside the private PIP target.
/// Keeping this policy here prevents a GUI texture-group blend policy from
/// silently changing copied item-model geometry.
fn gui_mesh_raster_state(
    material_mode: GuiMeshMaterialMode,
) -> (CullMode, BlendMode, Option<CompareOp>, bool) {
    match material_mode {
        GuiMeshMaterialMode::ModelOverlay => (CullMode::None, BlendMode::Alpha, Some(CompareOp::LessOrEqual), false),
        GuiMeshMaterialMode::Panorama => (CullMode::None, BlendMode::Disabled, None, false),
        GuiMeshMaterialMode::Opaque | GuiMeshMaterialMode::Cutout => (
            CullMode::Back,
            BlendMode::Disabled,
            Some(CompareOp::LessOrEqual),
            true,
        ),
        GuiMeshMaterialMode::Translucent => (
            CullMode::Back,
            BlendMode::Alpha,
            Some(CompareOp::LessOrEqual),
            true,
        ),
        GuiMeshMaterialMode::Glint => (
            CullMode::None,
            BlendMode::Glint,
            Some(CompareOp::Equal),
            false,
        ),
    }
}

fn transformed_front_face(
    matrix: [f32; 16],
    vertices: &[GuiMeshPreparedVertex],
    indices: &[u32],
) -> GalResult<super::resources::FrontFace> {
    let determinant = model_transform_determinant(matrix)?;
    // The mesh vertex stage maps GUI pixels to top-left-origin clip space,
    // which contributes one final Y reflection. This must be included with
    // the copied model basis when deciding the front face. Vanilla's
    // standard PIP pose itself is reflected, so its complete transform
    // remains counter-clockwise rather than being culled as an interior.
    let mut front_face = if determinant.is_sign_negative() {
        super::resources::FrontFace::CounterClockwise
    } else {
        super::resources::FrontFace::Clockwise
    };

    // A copied GUI item batch is one baked quad. Its source winding is a
    // per-face semantic, so the item transform alone cannot choose culling
    // correctly for every model face. Reconcile the first copied triangle
    // against its transformed vertex normal before choosing raster state.
    let [first, second, third] = first_triangle_indices(indices, vertices.len())?;
    let first = vertices[first];
    let second = vertices[second];
    let third = vertices[third];
    let geometric_normal = cross(
        subtract(second.position, first.position),
        subtract(third.position, first.position),
    );
    let average_normal = normalize(add(add(first.normal, second.normal), third.normal))?;
    let alignment = dot(geometric_normal, average_normal);
    if !alignment.is_finite() || alignment.abs() <= f32::EPSILON {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "GUI mesh triangle winding cannot be reconciled with its copied normal",
        ));
    }
    // Under a reflected transform the geometric cross product changes handedness
    // while the inverse-transposed normal intentionally does not. A negative
    // alignment is therefore expected precisely when the model determinant is
    // negative. Only the opposite relation denotes an independently reversed
    // source quad.
    if alignment.is_sign_negative() != determinant.is_sign_negative() {
        front_face = flip_front_face(front_face);
    }
    Ok(front_face)
}

fn model_transform_determinant(matrix: [f32; 16]) -> GalResult<f32> {
    let a = matrix[0];
    let b = matrix[4];
    let c = matrix[8];
    let d = matrix[1];
    let e = matrix[5];
    let f = matrix[9];
    let g = matrix[2];
    let h = matrix[6];
    let i = matrix[10];
    let determinant = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "GUI mesh model transform has no usable winding determinant",
        ));
    }
    Ok(determinant)
}

/// Native item meshes carry original model-space normals. Resolve the
/// inverse transpose in Rust, including the native GUI Y reflection.
/// Explicit/PIP meshes already carry resolved normals and do not use this.
fn transform_model_item_normal(matrix: [f32; 16], normal: [f32; 3]) -> GalResult<[f32; 3]> {
    let determinant = model_transform_determinant(matrix)?;
    let a = [matrix[0], matrix[1], matrix[2]];
    let b = [matrix[4], matrix[5], matrix[6]];
    let c = [matrix[8], matrix[9], matrix[10]];
    let cofactors = [cross(b, c), cross(c, a), cross(a, b)];
    normalize(std::array::from_fn(|axis|
        (cofactors[0][axis] * normal[0] + cofactors[1][axis] * normal[1]
            + cofactors[2][axis] * normal[2]) / determinant))
}

fn first_triangle_indices(indices: &[u32], vertex_count: usize) -> GalResult<[usize; 3]> {
    let [first, second, third, ..] = indices else {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "GUI mesh requires a first triangle to establish front-face orientation",
        ));
    };
    let indices = [*first as usize, *second as usize, *third as usize];
    if indices.iter().any(|index| *index >= vertex_count) {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "GUI mesh front-face triangle references a missing vertex",
        ));
    }
    Ok(indices)
}

fn flip_front_face(front_face: super::resources::FrontFace) -> super::resources::FrontFace {
    match front_face {
        super::resources::FrontFace::Clockwise => super::resources::FrontFace::CounterClockwise,
        super::resources::FrontFace::CounterClockwise => super::resources::FrontFace::Clockwise,
    }
}

fn subtract(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

fn add(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] + right[0], left[1] + right[1], left[2] + right[2]]
}

fn cross(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

fn dot(left: [f32; 3], right: [f32; 3]) -> f32 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn normalize(vector: [f32; 3]) -> GalResult<[f32; 3]> {
    let length_squared = dot(vector, vector);
    if !length_squared.is_finite() || length_squared <= f32::EPSILON {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "GUI mesh normal must have a finite non-zero length",
        ));
    }
    let inverse_length = length_squared.sqrt().recip();
    Ok([
        vector[0] * inverse_length,
        vector[1] * inverse_length,
        vector[2] * inverse_length,
    ])
}

/// Private Rust-owned resources that composite one PIP raster result into the
/// ordered GUI target. This is intentionally a normal textured GUI pass: the
/// Java PIP target, program, and blit state are neither observed nor reused.
#[derive(Clone, Copy, Debug)]
pub struct GuiMeshCompositeResources {
    pub uniform_buffer: Handle,
    pub sampler: Handle,
    pub vertex_shader: Handle,
    pub fragment_shader: Handle,
    pub resource_layout: Handle,
    pub resource_set: Handle,
    pub pipeline_layout: Handle,
    pub pipeline: Handle,
}

impl GuiMeshCompositeResources {
    pub fn create(
        gal: &mut VulkanicGal,
        label: &str,
        color_format: ColorFormat,
        depth_format: Option<TextureFormat>,
        source_color_view: Handle,
    ) -> GalResult<Self> {
        let mut created = Vec::new();
        let result = (|| -> GalResult<Self> {
            let uniform_buffer = gal.create_buffer(BufferDesc {
                label: format!("{label}.uniform"),
                size: GUI_MESH_MAX_COMPOSITE_UNIFORM_BYTES,
                memory: MemoryDomain::Upload,
                usages: vec![
                    BufferUsage::Uniform,
                    BufferUsage::TransferDst,
                    BufferUsage::HostWrite,
                ],
            })?;
            created.push(uniform_buffer);
            let sampler = gal.create_sampler(super::resources::SamplerDesc {
                label: format!("{label}.sampler"),
                min_filter: super::resources::SamplerFilter::Nearest,
                mag_filter: super::resources::SamplerFilter::Nearest,
                mip_filter: super::resources::SamplerFilter::Nearest,
                address_u: super::resources::SamplerAddressMode::ClampToEdge,
                address_v: super::resources::SamplerAddressMode::ClampToEdge,
                address_w: super::resources::SamplerAddressMode::ClampToEdge,
                comparison: None,
            })?;
            created.push(sampler);
            let (vertex_code, fragment_code) = match gal.capabilities().api {
                BackendApi::OpenGl => (
                    GUI_MESH_COMPOSITE_VERTEX_SHADER_OPENGL,
                    GUI_MESH_COMPOSITE_FRAGMENT_SHADER_OPENGL,
                ),
                BackendApi::Vulkan | BackendApi::Mock => (
                    GUI_MESH_COMPOSITE_VERTEX_SHADER_VULKAN,
                    GUI_MESH_COMPOSITE_FRAGMENT_SHADER_VULKAN,
                ),
            };
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
                label: format!("{label}.layout"),
                bindings: vec![
                    dynamic_resource_binding_desc(0, ResourceBindingKind::UniformBuffer),
                    resource_binding_desc(1, ResourceBindingKind::SampledTexture),
                    resource_binding_desc(2, ResourceBindingKind::Sampler),
                ],
            })?;
            created.push(resource_layout);
            let resource_set = gal.create_resource_set(ResourceSetDesc {
                label: format!("{label}.set"),
                layout: resource_layout,
                bindings: vec![
                    dynamic_read_binding(
                        0,
                        uniform_buffer,
                        ResourceBindingKind::UniformBuffer,
                        GUI_MESH_COMPOSITE_UNIFORM_BYTES as u64,
                    ),
                    read_binding(1, source_color_view, ResourceBindingKind::SampledTexture),
                    read_binding(2, sampler, ResourceBindingKind::Sampler),
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
                front_face: super::resources::FrontFace::CounterClockwise,
                provoking_vertex: crate::render::vulkanic::resources::ProvokingVertex::Last,
                raster_y_direction: crate::render::vulkanic::resources::RasterYDirection::Up,
                blend: BlendMode::Alpha,
                depth_compare: None,
                depth_write: false,
                depth_bias: None,
                color_formats: vec![color_format],
                depth_format,
                stencil: None,
            })?;
            created.push(pipeline);
            Ok(Self {
                uniform_buffer,
                sampler,
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
        result
    }

    pub fn append_composite(
        &self,
        source: GuiMeshOffscreenTarget,
        source_usage: TextureUsageState,
        destination_pass: Handle,
        destination_target: Handle,
        destination_color_view: Handle,
        destination_depth_view: Option<Handle>,
        draw: &GuiMeshPreparedDraw,
        uniform_offset: u64,
        operations: &mut Vec<CommandOp>,
    ) -> GalResult<()> {
        if draw.render_extent != [source.extent.width, source.extent.height] {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "GUI mesh composite source extent does not match its prepared draw",
            ));
        }
        self.append_composite_uniforms(source.color, source_usage,
            destination_pass, destination_target, destination_color_view, destination_depth_view,
            composite_uniform_bytes(draw), uniform_offset, operations)
    }

    /// Compose a Rust-owned item raster cell. Screen-space geometry remains
    /// distinct from item-local raster geometry, and the caller supplies the
    /// source's explicit usage instead of inferring an implicit framebuffer.
    pub(crate) fn append_item_raster_composite(
        &self, source_color: Handle, source_usage: TextureUsageState,
        placement: super::gui_item_raster::GuiItemRasterPlacement,
        destination_pass: Handle, destination_target: Handle,
        destination_color_view: Handle, destination_depth_view: Option<Handle>,
        screen: &super::gui_frontend::GuiAffineQuadRequest,
        pre_present_y_flip: bool, uniform_offset: u64, operations: &mut Vec<CommandOp>,
    ) -> GalResult<()> {
        placement.validate()?;
        let [u0,v0,u1,v1] = placement.composite_uv();
        let mut clip = if screen.clip_mode == 1 {
            [screen.clip_left as f32, screen.clip_top as f32,
             (screen.clip_left as f32 + screen.clip_width as f32),
             (screen.clip_top as f32 + screen.clip_height as f32)]
        } else { [0.0,0.0,screen.projection_extent[0],screen.projection_extent[1]] };
        let (origin_y,axis_u_y,axis_v_y) = if pre_present_y_flip {
            clip = [clip[0],screen.projection_extent[1]-clip[3],clip[2],screen.projection_extent[1]-clip[1]];
            (screen.projection_extent[1]-screen.y0,screen.y0-screen.y1,screen.y0-screen.y3)
        } else {(screen.y0,screen.y1-screen.y0,screen.y3-screen.y0)};
        let values = [screen.x1-screen.x0, axis_u_y,
            screen.x3-screen.x0, axis_v_y,
            screen.x0,origin_y,screen.projection_extent[0],screen.projection_extent[1],
            0.0,0.0,1.0,1.0,u0,v0,u1-u0,v1-v0,clip[0],clip[1],clip[2],clip[3]];
        if values.iter().any(|v| !v.is_finite()) || screen.projection_extent.iter().any(|v| *v <= 0.0)
            || screen.clip_mode > 1 || screen.z != 0.0 {
            return Err(GalError::invalid_argument("invalid or unsupported item raster composition"));
        }
        self.append_composite_uniforms(source_color, source_usage, destination_pass,
            destination_target, destination_color_view, destination_depth_view,
            values.into_iter().flat_map(f32::to_le_bytes).collect(), uniform_offset, operations)
    }

    fn append_composite_uniforms(&self, source_color: Handle, source_usage: TextureUsageState,
        destination_pass: Handle, destination_target: Handle, destination_color_view: Handle,
        destination_depth_view: Option<Handle>, uniforms: Vec<u8>, uniform_offset: u64,
        operations: &mut Vec<CommandOp>) -> GalResult<()> {
        if uniform_offset % GUI_MESH_COMPOSITE_UNIFORM_STRIDE != 0
            || uniform_offset
                .checked_add(GUI_MESH_COMPOSITE_UNIFORM_BYTES as u64)
                .map_or(true, |end| end > GUI_MESH_MAX_COMPOSITE_UNIFORM_BYTES)
        {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "GUI mesh composite uniform stream range is invalid",
            ));
        }
        operations.push(CommandOp::Barrier(buffer_barrier(
            self.uniform_buffer,
            TextureUsageState::ShaderRead,
            TextureUsageState::TransferDst,
        )));
        operations.push(CommandOp::HostWriteBuffer {
            buffer: self.uniform_buffer,
            offset: uniform_offset,
            data: uniforms,
        });
        operations.push(CommandOp::Barrier(buffer_barrier(
            self.uniform_buffer,
            TextureUsageState::TransferDst,
            TextureUsageState::ShaderRead,
        )));
        if source_usage != TextureUsageState::ShaderRead {
            operations.push(CommandOp::Barrier(ResourceBarrier {
                resource: source_color,
                subresources: None,
                before: source_usage,
                after: TextureUsageState::ShaderRead,
                src_queue: QueueClass::Graphics,
                dst_queue: QueueClass::Graphics,
            }));
        }
        operations.push(CommandOp::BeginPass {
            pass: destination_pass,
            target: destination_target,
            colors: vec![PassAttachment {
                view: destination_color_view,
                load_op: AttachmentLoadOp::Load,
                store_op: AttachmentStoreOp::Store,
                clear_color: None,
            }],
            depth_stencil: destination_depth_view.map(|view| PassAttachment {
                view,
                load_op: AttachmentLoadOp::Load,
                store_op: AttachmentStoreOp::Store,
                clear_color: None,
            }),
        });
        operations.push(CommandOp::BindGraphicsPipeline(self.pipeline));
        operations.push(CommandOp::BindResourceSet {
            pipeline_layout: self.pipeline_layout,
            set_index: 0,
            set: self.resource_set,
            dynamic_offsets: vec![uniform_offset],
        });
        operations.push(CommandOp::Draw {
            vertices: 6,
            instances: 1,
        });
        operations.push(CommandOp::EndPass);
        Ok(())
    }

    pub fn destroy(self, gal: &mut VulkanicGal) {
        for handle in [
            self.pipeline,
            self.pipeline_layout,
            self.resource_set,
            self.resource_layout,
            self.fragment_shader,
            self.vertex_shader,
            self.sampler,
            self.uniform_buffer,
        ] {
            let _ = gal.destroy(handle);
        }
    }
}

fn resource_binding_desc(binding: u32, kind: ResourceBindingKind) -> ResourceBindingDesc {
    ResourceBindingDesc {
        binding,
        kind,
        stages: PipelineStageFlags::DRAW,
        array_count: 1,
        optional: false,
        dynamic_offset_count: 0,
    }
}

fn dynamic_resource_binding_desc(binding: u32, kind: ResourceBindingKind) -> ResourceBindingDesc {
    ResourceBindingDesc {
        dynamic_offset_count: 1,
        ..resource_binding_desc(binding, kind)
    }
}

fn read_binding(binding: u32, resource: Handle, kind: ResourceBindingKind) -> ResourceBinding {
    ResourceBinding {
        binding,
        array_index: 0,
        resource,
        kind,
        access: AccessFlags::READ,
        dynamic_offsets: Vec::new(),
        buffer_range: None,
    }
}

fn dynamic_read_binding(
    binding: u32,
    resource: Handle,
    kind: ResourceBindingKind,
    buffer_range: u64,
) -> ResourceBinding {
    ResourceBinding {
        dynamic_offsets: vec![0],
        buffer_range: Some(buffer_range),
        ..read_binding(binding, resource, kind)
    }
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

fn packed_indices_with_base(indices: &[u32], vertex_base: u32) -> GalResult<Vec<u8>> {
    let mut bytes = Vec::with_capacity(indices.len() * std::mem::size_of::<u32>());
    for index in indices {
        let adjusted = index.checked_add(vertex_base).ok_or_else(|| {
            GalError::ffi(
                StatusCode::InvalidArgument,
                "GUI mesh index stream base overflows u32",
            )
        })?;
        bytes.extend_from_slice(&adjusted.to_le_bytes());
    }
    Ok(bytes)
}

fn draw_frame_uniform_bytes(draw: &GuiMeshPreparedDraw, extent: [f32; 2]) -> Vec<u8> {
    // Opaque means no fragment discard, not an alpha threshold of zero.
    // A zero-alpha opaque model surface still replaces color and writes depth;
    // discarding it exposes geometry behind it (e.g. the shield handle).
    // This is private shader lowering of the explicit material, not new Java
    // policy or an exposed implicit alpha-test state. Other materials retain
    // their declared cutoff unchanged.
    let cutoff = if draw.material_mode == GuiMeshMaterialMode::Opaque {
        -1.0
    } else {
        draw.alpha_cutoff
    };
    let mut bytes = frame_uniform_bytes(extent, cutoff, draw.lighting_mode);
    if draw.material_mode == GuiMeshMaterialMode::ModelOverlay {
        bytes[12..16].copy_from_slice(&3.0_f32.to_le_bytes());
    }
    bytes
}

fn frame_uniform_bytes(
    extent: [f32; 2],
    alpha_cutoff: f32,
    lighting_mode: GuiMeshLightingMode,
) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(GUI_MESH_FRAME_UNIFORM_BYTES);
    // Inventory lighting consumes the signed-normalized packed vertex normal
    // directly, as Frozen entity.vsh/light.glsl do. Renormalizing after i8
    // quantization changes the light intensity. Preserve the existing separate
    // upright mesh convention rather than silently changing unrelated routes.
    let lighting_policy = match lighting_mode {
        GuiMeshLightingMode::InventoryBlock | GuiMeshLightingMode::FrontModel => 2.0,
        GuiMeshLightingMode::Block => 1.0,
        GuiMeshLightingMode::Flat => 0.0,
    };
    for value in [extent[0] as f32, extent[1] as f32, alpha_cutoff, lighting_policy] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    if lighting_mode == GuiMeshLightingMode::FrontModel {
        // Frozen Lighting.ITEMS_FLAT: rotationY(-pi/8).rotateX(3pi/4).
        // Normals already include the item atlas's Y reflection; the light
        // directions themselves do not. No borrowed Lighting UBO/state.
        let (sx, cx) = (std::f32::consts::PI * 0.75).sin_cos();
        let (sy, cy) = (-std::f32::consts::PI / 8.0).sin_cos();
        for [x, y, z] in [[0.2_f32, 1.0, -0.7], [-0.2, 1.0, 0.7]] {
            let length = (x*x + y*y + z*z).sqrt();
            let ry = cx*y - sx*z;
            let rz = sx*y + cx*z;
            for value in [(cy*x + sy*rz)/length, ry/length, (-sy*x + cy*rz)/length, 0.0] {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        return bytes;
    }
    // Frozen OpenGL GuiRenderer.renderItemToAtlas uses ITEMS_3D with
    // scale(k, -k, k) normals. ITEMS_3D_UPRIGHT belongs to the separate
    // PIP convention, not ordinary inventory icons. Keep the light-space
    // selection explicit; never infer it from a backend or borrowed UBO.
    let light_y_sign = if lighting_mode == GuiMeshLightingMode::InventoryBlock { -1.0 } else { 1.0 };
    for value in [
        -0.933_439_2_f32,
        0.262_694_72 * light_y_sign,
        -0.244_300_16,
        0.0,
        -0.103_571_37,
        0.976_606_8 * light_y_sign,
        0.188_446_42,
        0.0,
    ] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn composite_uniform_bytes(draw: &GuiMeshPreparedDraw) -> Vec<u8> {
    let [m00, m01, m10, m11, m20, m21] = draw.gui_pose;
    let [left, top, right, bottom] = draw.bounds;
    let [width, height] = draw.render_extent;
    let guard = draw.guard_pixels as f32;
    let width = width as f32;
    let height = height as f32;
    let mut bytes = Vec::with_capacity(GUI_MESH_COMPOSITE_UNIFORM_BYTES);
    for value in [
        m00,
        m01,
        m10,
        m11,
        m20,
        m21,
        draw.projection_extent[0],
        draw.projection_extent[1],
        left as f32,
        top as f32,
        right as f32,
        bottom as f32,
        guard / width,
        guard / height,
        (width - guard * 2.0) / width,
        (height - guard * 2.0) / height,
        if draw.clip_mode == 1 {
            draw.clip_left as f32
        } else {
            0.0
        },
        if draw.clip_mode == 1 {
            draw.clip_top as f32
        } else {
            0.0
        },
        if draw.clip_mode == 1 {
            (draw.clip_left + draw.clip_width) as f32
        } else {
            draw.projection_extent[0]
        },
        if draw.clip_mode == 1 {
            (draw.clip_top + draw.clip_height) as f32
        } else {
            draw.projection_extent[1]
        },
    ] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct OffscreenTargetKey {
    generation: u64,
    item_identity: u64,
    width: u32,
    height: u32,
}

/// Rust-owned color/depth target for GUI mesh rasterization. This is separate
/// from the final frame target so GUI-item depth cannot interact with terrain
/// depth; later GUI composition consumes only `color_view`.
#[derive(Clone, Copy, Debug)]
pub struct GuiMeshOffscreenTarget {
    pub color: Handle,
    pub color_view: Handle,
    pub depth: Handle,
    pub depth_view: Handle,
    pub target: Handle,
    pub pass: Handle,
    pub extent: Extent3d,
    pub(crate) initialized: bool,
}

#[derive(Default)]
pub struct GuiMeshOffscreenTargetCache {
    targets: BTreeMap<OffscreenTargetKey, GuiMeshOffscreenTarget>,
}

const GUI_MESH_MAX_OFFSCREEN_TARGETS_PER_GENERATION: usize = 64;
impl GuiMeshOffscreenTargetCache {
    pub(crate) fn len(&self) -> usize {
        self.targets.len()
    }

    /// Returns an owned target for this GUI resource generation and logical
    /// raster extent. A new generation atomically retires old target objects
    /// before staging its replacements; no Java target or view is involved.
    pub fn stage(
        &mut self,
        gal: &mut VulkanicGal,
        generation: u64,
        extent: Extent3d,
    ) -> GalResult<GuiMeshOffscreenTarget> {
        self.stage_item(gal, generation, extent, 0)
    }

    pub(crate) fn stage_item(&mut self, gal: &mut VulkanicGal, generation: u64,
        extent: Extent3d, item_identity: u64) -> GalResult<GuiMeshOffscreenTarget> {
        if generation == 0 || extent.width == 0 || extent.height == 0 || extent.depth != 1 {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "GUI mesh offscreen target requires a non-zero generation and D2 extent",
            ));
        }
        if extent.width > GUI_MESH_MAX_OFFSCREEN_AXIS || extent.height > GUI_MESH_MAX_OFFSCREEN_AXIS
        {
            return Err(GalError::unsupported_feature(format!(
                "GUI mesh offscreen extent {}x{} exceeds bounded axis {}",
                extent.width, extent.height, GUI_MESH_MAX_OFFSCREEN_AXIS
            )));
        }
        let key = OffscreenTargetKey {
            generation,
            item_identity,
            width: extent.width,
            height: extent.height,
        };
        if let Some(target) = self.targets.get(&key).copied() {
            return Ok(target);
        }
        self.destroy_other_generations(gal, generation);
        if self.targets.len() >= GUI_MESH_MAX_OFFSCREEN_TARGETS_PER_GENERATION {
            return Err(GalError::unsupported_feature(format!(
                "GUI mesh offscreen target cache exceeds bounded limit {GUI_MESH_MAX_OFFSCREEN_TARGETS_PER_GENERATION}"
            )));
        }
        let label = format!(
            "minecraft.gui.mesh.gen{generation}.{}x{}",
            extent.width, extent.height
        );
        let mut created = Vec::new();
        let result = (|| -> GalResult<GuiMeshOffscreenTarget> {
            let color = gal.create_texture(TextureDesc {
                label: format!("{label}.color"),
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba8Unorm,
                extent,
                mip_levels: 1,
                array_layers: 1,
                usages: vec![
                    TextureUsage::ColorAttachment,
                    TextureUsage::Sampled,
                    TextureUsage::TransferSrc,
                ],
            })?;
            created.push(color);
            let color_view = gal.create_texture_view(TextureViewDesc {
                label: format!("{label}.color-view"),
                texture: color,
                format: TextureFormat::Rgba8Unorm,
                base_mip: 0,
                mip_count: 1,
                base_layer: 0,
                layer_count: 1,
            })?;
            created.push(color_view);
            let depth = gal.create_texture(TextureDesc {
                label: format!("{label}.depth"),
                dimension: TextureDimension::D2,
                format: TextureFormat::Depth32Float,
                extent,
                mip_levels: 1,
                array_layers: 1,
                usages: vec![TextureUsage::DepthStencilAttachment],
            })?;
            created.push(depth);
            let depth_view = gal.create_texture_view(TextureViewDesc {
                label: format!("{label}.depth-view"),
                texture: depth,
                format: TextureFormat::Depth32Float,
                base_mip: 0,
                mip_count: 1,
                base_layer: 0,
                layer_count: 1,
            })?;
            created.push(depth_view);
            let target = gal.create_render_target(RenderTargetDesc {
                label: format!("{label}.target"),
                color_views: vec![color_view],
                depth_stencil_view: Some(depth_view),
                extent,
            })?;
            created.push(target);
            let pass = gal.create_render_pass(RenderPassDesc {
                label: format!("{label}.pass"),
                target,
                color_formats: vec![TextureFormat::Rgba8Unorm],
                depth_format: Some(TextureFormat::Depth32Float),
            })?;
            created.push(pass);
            Ok(GuiMeshOffscreenTarget {
                color,
                color_view,
                depth,
                depth_view,
                target,
                pass,
                extent,
                initialized: false,
            })
        })();
        match result {
            Ok(target) => {
                self.targets.insert(key, target);
                Ok(target)
            }
            Err(error) => {
                for handle in created.into_iter().rev() {
                    let _ = gal.destroy(handle);
                }
                Err(error)
            }
        }
    }

    pub fn clear(&mut self, gal: &mut VulkanicGal) {
        let targets = std::mem::take(&mut self.targets);
        for (_, target) in targets {
            destroy_target(gal, target);
        }
    }

    fn destroy_other_generations(&mut self, gal: &mut VulkanicGal, generation: u64) {
        let stale = self
            .targets
            .keys()
            .copied()
            .filter(|key| key.generation != generation)
            .collect::<Vec<_>>();
        for key in stale {
            if let Some(target) = self.targets.remove(&key) {
                destroy_target(gal, target);
            }
        }
    }
}

fn destroy_target(gal: &mut VulkanicGal, target: GuiMeshOffscreenTarget) {
    for handle in [
        target.pass,
        target.target,
        target.depth_view,
        target.depth,
        target.color_view,
        target.color,
    ] {
        let _ = gal.destroy(handle);
    }
}

/// Diagnostic counts from decoded semantic meshes; no rendering state mutation.
pub(crate) fn flat_item_mesh_decode_counts(batches: &[GuiMeshBatchRequest]) -> (usize, usize, usize, usize) {
    let mut groups = BTreeSet::new();
    let mut transforms = BTreeSet::new();
    let mut layers = 0;
    let mut nonidentity = 0;
    // Even an untransformed item includes the authored model-centering
    // translation; compare with that convention, not a raw identity matrix.
    let identity = super::gui_item_raster::GuiItemModelTransform::default().0;
    for batch in batches.iter().filter(|batch| batch.item_raster_scale != 0) {
        groups.insert((batch.stratum, batch.sequence));
        layers += 1;
        nonidentity += usize::from(batch.model_transform != identity);
        transforms.insert(batch.model_transform.map(|v| if v == 0.0 { 0 } else { v.to_bits() }));
    }
    (groups.len(), layers, nonidentity, transforms.len())
}

pub fn validate_batches(batches: &[GuiMeshBatchRequest]) -> GalResult<()> {
    if batches.len() > GUI_MESH_MAX_BATCHES {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!(
                "GUI mesh batch count {} exceeds maximum {}",
                batches.len(),
                GUI_MESH_MAX_BATCHES
            ),
        ));
    }
    let mut layer_groups = BTreeMap::<(u32, u64), BTreeSet<u32>>::new();
    for batch in batches {
        validate_batch(batch)?;
        if batch.sequence == 0 {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "GUI mesh batches require non-zero item frame sequences",
            ));
        }
        if !layer_groups
            .entry((batch.stratum, batch.sequence))
            .or_default()
            .insert(batch.layer_index)
        {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "GUI mesh item layers require unique layer indices",
            ));
        }
    }
    for layers in layer_groups.values() {
        if layers
            .iter()
            .copied()
            .enumerate()
            .any(|(expected, actual)| actual != expected as u32)
        {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "GUI mesh item layers must be contiguous from zero",
            ));
        }
    }
    Ok(())
}

impl GuiMeshBatchRequest {
    fn requires_item_lightmap(&self) -> bool {
        (self.item_raster_scale != 0 || self.lighting_mode == GuiMeshLightingMode::InventoryBlock)
            && self.material_mode != GuiMeshMaterialMode::Glint
    }

    pub fn resolve_item_lighting(&mut self, frame: Option<super::shader_pack::lightmap::VanillaLightmapFrame>) -> GalResult<()> {
        if self.requires_item_lightmap() {
            self.item_lighting = Some(GuiFlatItemLighting::prepare(frame.ok_or_else(||
                GalError::invalid_argument("GUI item mesh requires explicit frame lightmap inputs"))?)?);
        }
        Ok(())
    }
}

/// Frozen's flat atlas cell uses translate(k/2,k/2,0), scale(k,-k,k).
/// Resolve that layout here, without a caller-provided PIP target or guard band.
fn resolved_item_raster(batch: &GuiMeshBatchRequest) -> GalResult<([u32; 2], [f32; 16], u32)> {
    if let Some(block) = batch.block_item_raster {
        if block.model_min.iter().chain(block.model_max.iter()).any(|v| !v.is_finite())
            || (0..3).any(|axis| block.model_min[axis] > block.model_max[axis]) {
            return Err(GalError::invalid_argument("invalid semantic item bounds"));
        }
        if batch.item_raster_scale != 0 || batch.render_extent != [0,0] || batch.guard_pixels != 0
            || batch.decal_foil.is_some()
            || !matches!((batch.material_mode, batch.lighting_mode),
                (GuiMeshMaterialMode::Opaque | GuiMeshMaterialMode::Cutout | GuiMeshMaterialMode::Translucent,
                 GuiMeshLightingMode::InventoryBlock)
                | (GuiMeshMaterialMode::Glint, GuiMeshLightingMode::Flat))
            || (batch.material_mode == GuiMeshMaterialMode::Glint && batch.item_foil.is_none()) {
            return Err(GalError::invalid_argument("conflicting native block item layout semantics"));
        }
        let layout = match block.oversized_layout(batch.bounds)? {
            Some((layout,_)) => layout,
            None => super::gui_item_layout::GuiItemRasterLayout::inventory_block(block.gui_scale)?,
        };
        return Ok((layout.extent, layout.compose(batch.model_transform)?, layout.guard_pixels));
    }
    if batch.item_raster_scale == 0 {
        if batch.item_lighting.is_some() && !batch.requires_item_lightmap() {
            return Err(GalError::invalid_argument("explicit mesh cannot carry flat item lighting"));
        }
        return Ok((batch.render_extent, batch.model_transform, batch.guard_pixels));
    }
    if batch.render_extent != [0, 0] || batch.guard_pixels != 0
        || !matches!(batch.lighting_mode, GuiMeshLightingMode::Flat | GuiMeshLightingMode::FrontModel)
        || batch.material_mode == GuiMeshMaterialMode::Panorama
        || (batch.material_mode == GuiMeshMaterialMode::Glint && batch.item_foil.is_none()) {
        return Err(GalError::invalid_argument("flat item mesh has conflicting raster/material semantics"));
    }
    let layout = super::gui_item_layout::GuiItemRasterLayout::flat(batch.item_raster_scale)?;
    // Indexed front-lit items include genuine 3D models (e.g. shields), not
    // just generated 2D item layers. Preserve their explicit affine transform;
    // the separate quad/sprite frontend retains its 2D transform contract.
    if batch.model_transform.iter().any(|v| !v.is_finite() || v.abs() > 16.0)
        || [3,7,11].iter().any(|i| batch.model_transform[*i] != 0.0)
        || batch.model_transform[15] != 1.0 {
        return Err(GalError::invalid_argument("native item mesh requires a bounded affine model transform"));
    }
    model_transform_determinant(batch.model_transform)?;
    Ok((layout.extent, layout.compose(batch.model_transform)?, layout.guard_pixels))
}

pub fn validate_batch(batch: &GuiMeshBatchRequest) -> GalResult<()> {
    if let Some(cache) = batch.item_cache {
        if cache.identity == 0 || batch.item_raster_scale == 0
            || (!cache.animated && batch.item_foil.is_some()) {
            return Err(GalError::invalid_argument("GUI item caching requires a native item raster and coherent animation semantics"));
        }
    }
    if batch.material_mode == GuiMeshMaterialMode::ModelOverlay
        && (batch.item_raster_scale == 0 || batch.lighting_mode != GuiMeshLightingMode::FrontModel
            || batch.alpha_cutoff != 0.0 || batch.item_foil.is_some()) {
        return Err(GalError::invalid_argument("model overlay requires native front-lit base geometry without cutout or foil"));
    }
    if batch.lighting_mode == GuiMeshLightingMode::FrontModel
        && (batch.item_raster_scale == 0 || batch.material_mode == GuiMeshMaterialMode::Glint
            || batch.decal_foil.is_some() || batch.block_item_raster.is_some()) {
        return Err(GalError::invalid_argument("front model lighting requires a native base item mesh"));
    }
    let (render_extent, model_transform, guard_pixels) = resolved_item_raster(batch)?;
    if let Some(decal) = batch.decal_foil {
        if batch.item_foil.is_none() {
            return Err(GalError::invalid_argument("decal foil requires explicit item foil timing and strength"));
        }
        decal.prepare((batch.item_raster_scale != 0).then_some(model_transform))?;
    }
    if let Some(foil) = batch.item_foil {
        foil.validate()?;
        if foil.kind == super::item_foil::StandardFoilKind::Entity
            && (batch.item_raster_scale == 0 || batch.block_item_raster.is_some()
                || batch.decal_foil.is_some() || batch.lighting_mode != GuiMeshLightingMode::Flat) {
            return Err(GalError::invalid_argument("entity foil requires native front-lit model item layout"));
        }
        if batch.material_mode != GuiMeshMaterialMode::Glint {
            return Err(GalError::invalid_argument("item foil requires the glint material"));
        }
    }
    if batch.stratum == 0 {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "GUI mesh batch requires a non-zero GUI stratum",
        ));
    }
    if batch.asset_id == 0 {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "GUI mesh batch requires a non-zero semantic image asset id",
        ));
    }
    if batch.gui_extent[0] == 0 || batch.gui_extent[1] == 0 {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "GUI mesh batch requires a positive GUI extent",
        ));
    }
    if batch.gui_extent[0] > GUI_MAX_VIEWPORT_AXIS as u32
        || batch.gui_extent[1] > GUI_MAX_VIEWPORT_AXIS as u32
    {
        return Err(GalError::unsupported_feature(format!(
            "GUI mesh logical extent {}x{} exceeds bounded axis {}",
            batch.gui_extent[0], batch.gui_extent[1], GUI_MAX_VIEWPORT_AXIS
        )));
    }
    if render_extent[0] == 0 || render_extent[1] == 0 {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "GUI mesh batch requires a positive offscreen raster extent",
        ));
    }
    super::gui_frontend::validate_gui_projection(batch.gui_extent, batch.projection_extent)?;
    if render_extent[0] > GUI_MESH_MAX_OFFSCREEN_AXIS
        || render_extent[1] > GUI_MESH_MAX_OFFSCREEN_AXIS
    {
        return Err(GalError::unsupported_feature(format!(
            "GUI mesh offscreen extent {}x{} exceeds bounded axis {}",
            batch.render_extent[0], batch.render_extent[1], GUI_MESH_MAX_OFFSCREEN_AXIS
        )));
    }
    if batch.bounds[0] >= batch.bounds[2] || batch.bounds[1] >= batch.bounds[3] {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "GUI mesh batch requires ordered non-empty logical bounds",
        ));
    }
    if !batch.alpha_cutoff.is_finite() || !(0.0..=1.0).contains(&batch.alpha_cutoff) {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "GUI mesh alpha cutoff must be finite and within [0, 1]",
        ));
    }
    if batch.material_mode == GuiMeshMaterialMode::Opaque && batch.alpha_cutoff != 0.0 {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "opaque GUI mesh batches must use a zero alpha cutoff",
        ));
    }
    if batch.vertices.len() < 3 || batch.vertices.len() > GUI_MESH_MAX_VERTICES {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!(
                "GUI mesh vertex count {} is outside 3..={}",
                batch.vertices.len(),
                GUI_MESH_MAX_VERTICES
            ),
        ));
    }
    if batch.indices.len() < 3
        || batch.indices.len() > GUI_MESH_MAX_INDICES
        || batch.indices.len() % 3 != 0
    {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            format!(
                "GUI mesh index count {} must be a bounded triangle list",
                batch.indices.len()
            ),
        ));
    }
    if !batch.model_transform.iter().all(|value| value.is_finite())
        || !batch.gui_pose.iter().all(|value| value.is_finite())
    {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "GUI mesh transforms must be finite",
        ));
    }
    if guard_pixels.saturating_mul(2) >= render_extent[0]
        || guard_pixels.saturating_mul(2) >= render_extent[1]
    {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "GUI mesh guard band must leave a non-empty offscreen raster area",
        ));
    }
    match batch.clip_mode {
        0 if batch.clip_left == 0
            && batch.clip_top == 0
            && batch.clip_width == 0
            && batch.clip_height == 0 => {}
        1 if batch.clip_left >= 0
            && batch.clip_top >= 0
            && batch.clip_width >= 0
            && batch.clip_height >= 0
            && batch.clip_left <= batch.gui_extent[0] as i32
            && batch.clip_top <= batch.gui_extent[1] as i32
            && batch.clip_width <= batch.gui_extent[0] as i32 - batch.clip_left
            && batch.clip_height <= batch.gui_extent[1] as i32 - batch.clip_top => {}
        _ => {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "GUI mesh clip must be disabled or a bounded frame-local rectangle",
            ))
        }
    }
    for vertex in &batch.vertices {
        if vertex.source_face > 6 || vertex.source_foil_type > 1
            || (vertex.source_foil_type != 0 && vertex.source_face == 0)
            || ((vertex.source_face != 0 || vertex.source_foil_type != 0) && batch.block_item_raster.is_none()) {
            return Err(GalError::invalid_argument("GUI baked face/foil requires valid native block semantics"));
        }
        if !vertex.position.iter().all(|value| value.is_finite())
            || !vertex.atlas_uv.iter().all(|value| value.is_finite())
            || !vertex.local_uv.iter().all(|value| value.is_finite())
        {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "GUI mesh vertices must contain finite positions and atlas UVs",
            ));
        }
    }
    if batch
        .indices
        .iter()
        .any(|index| *index as usize >= batch.vertices.len())
    {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "GUI mesh indices must reference a copied vertex in their batch",
        ));
    }
    Ok(())
}

/// Consumes the caller-independent request family into a compact render-plan
/// family. Transforming at this boundary means later GUI mesh resource and
/// command construction only sees Rust-owned data. Standard item foil consumes
/// original `atlas_uv`; other meshes use the supplied local texture UVs.
pub fn prepare_draws(batches: &[GuiMeshBatchRequest]) -> GalResult<Vec<GuiMeshPreparedDraw>> {
    validate_batches(batches)?;
    batches.iter().map(prepare_draw).collect()
}

fn prepare_draw(batch: &GuiMeshBatchRequest) -> GalResult<GuiMeshPreparedDraw> {
    let (render_extent, model_transform, guard_pixels) = resolved_item_raster(batch)?;
    let decal_projection = batch.decal_foil.map(|decal|
        decal.prepare((batch.item_raster_scale != 0).then_some(model_transform))).transpose()?;
    let vertices = batch
        .vertices
        .iter()
        .map(|vertex| {
            let position = transform_point(model_transform, vertex.position)?;
            let packed_normal = unpack_normal_i8(vertex.normal_packed);
            let mut normal = normalize_semantic_normal(packed_normal)?;
            if vertex.source_foil_type == 1 {
                // Frozen's enchanted GUI multi-consumer emits the baked face
                // direction, unlike the unenchanted packed-vertex encoder.
                // Select and transform it here, never in the semantic caller.
                normal = match vertex.source_face {
                    1 => [0.0,-1.0,0.0], 2 => [0.0,1.0,0.0],
                    3 => [0.0,0.0,-1.0], 4 => [0.0,0.0,1.0],
                    5 => [-1.0,0.0,0.0], 6 => [1.0,0.0,0.0],
                    _ => return Err(GalError::invalid_argument("missing enchanted baked face")),
                };
            }
            if batch.lighting_mode == GuiMeshLightingMode::InventoryBlock && batch.block_item_raster.is_none() {
                normal = packed_normal;
            }
            if batch.item_raster_scale != 0 {
                normal = transform_model_item_normal(model_transform, normal)?;
                if batch.lighting_mode == GuiMeshLightingMode::FrontModel {
                    normal = normal.map(|v| ((v.clamp(-1.0,1.0)*127.0) as i8) as f32 / 127.0);
                }
                // Frozen clips model geometry to the item atlas cell. Keep
                // original vertices and let the explicit offscreen viewport
                // clip them; never fit or clamp the model to the image.
            }
            if batch.block_item_raster.is_some() {
                // Out-of-cell vertices are valid for ordinary item models.
                // The explicit raster viewport clips them like Frozen's atlas.
                normal = transform_model_item_normal(model_transform, normal)?;
                // Frozen's item encoder normalizes, then packs to signed i8;
                // its shader consumes that quantized direction. Keep that
                // quantization in Rust rather than asking Java to rasterize normals.
                normal = normal.map(|v|
                    ((v.clamp(-1.0,1.0)*127.0) as i8) as f32 / 127.0);
            }
            let mut color=argb_to_rgba(vertex.color_argb);
            let local_uv = if let Some(foil) = batch.item_foil {
                color = foil.color()?;
                let source_uv = if let Some(decal) = &decal_projection {
                    decal.texture_uv(position, normal)?
                } else { vertex.atlas_uv };
                foil.texture_uv(source_uv)?
            } else {
                vertex.local_uv
            };
            if batch.item_foil.is_none() && batch.material_mode==GuiMeshMaterialMode::Glint {
                // The copied item glint color's alpha encodes strength, not
                // coverage. Frozen glint.fsh applies GlintAlpha to RGB after
                // the texture alpha test, preserving the destination alpha.
                let strength=color[3];
                color=[color[0]*strength,color[1]*strength,color[2]*strength,1.0];
            }
            if batch.requires_item_lightmap() {
                color = batch.item_lighting.ok_or_else(|| GalError::invalid_argument(
                    "GUI item mesh requires resolved frame lightmap semantics"))?.modulate(color)?;
            }
            Ok(GuiMeshPreparedVertex {
                position,
                local_uv,
                color,
                normal,
            })
        })
        .collect::<GalResult<Vec<_>>>()?;
    let (indices, front_face) = if batch.item_raster_scale != 0 || batch.block_item_raster.is_some() {
        reconcile_model_mesh_topology(model_transform, &vertices, &batch.indices)?
    } else {
        (batch.indices.clone(), transformed_front_face(model_transform, &vertices, &batch.indices)?)
    };
    trace_prepared_item_vertices(batch, &vertices, model_transform, render_extent);
    Ok(GuiMeshPreparedDraw {
        item_cache: batch.item_cache,
        stratum: batch.stratum,
        layer_index: batch.layer_index,
        sequence: batch.sequence,
        asset_id: batch.asset_id,
        material_mode: batch.material_mode,
        front_face,
        lighting_mode: batch.lighting_mode,
        alpha_cutoff: batch.alpha_cutoff,
        gui_pose: batch.gui_pose,
        bounds: match batch.block_item_raster.map(|block|block.oversized_layout(batch.bounds)).transpose()?.flatten() {
            Some((_,bounds)) => bounds,
            None => batch.bounds,
        },
        gui_extent: batch.gui_extent,
        projection_extent: batch.projection_extent,
        render_extent,
        guard_pixels,
        clip_mode: batch.clip_mode,
        clip_left: batch.clip_left,
        clip_top: batch.clip_top,
        clip_width: batch.clip_width,
        clip_height: batch.clip_height,
        vertices,
        indices,
    })
}

/// Optional bounded CPU observations only; no rendering inputs or GPU handles.
fn trace_prepared_item_vertices(batch: &GuiMeshBatchRequest, vertices: &[GuiMeshPreparedVertex],
    matrix: [f32;16], extent: [u32;2]) {
    use std::sync::{OnceLock, atomic::{AtomicUsize,Ordering}};
    static SELECTOR: OnceLock<Option<[i32;2]>> = OnceLock::new();
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    let selected=SELECTOR.get_or_init(|| std::env::var("MATTMC_GUI_MESH_VERTEX_TRACE")
        .ok().and_then(|text| parse_vertex_trace_selector(&text)));
    let Some(selected)=selected else { return; };
    if batch.bounds[..2] != selected[..] || COUNT.fetch_add(1,Ordering::Relaxed)>=32 { return; }
    let copied:Vec<_>=batch.vertices.iter().zip(vertices).map(|(source,prepared)|
        [source.position[0],source.position[1],source.position[2],
         prepared.position[0],prepared.position[1],prepared.position[2],
         source.atlas_uv[0],source.atlas_uv[1]]).collect();
    eprintln!("[gui.mesh.vertex-trace] {{\"layer\":{},\"material\":\"{:?}\",\"bounds\":{:?},\"extent\":{:?},\"matrix\":{:?},\"vertices\":{:?}}}",
        batch.layer_index,batch.material_mode,batch.bounds,extent,matrix,copied);
}

fn parse_vertex_trace_selector(text: &str) -> Option<[i32;2]> {
    let (x,y)=text.split_once(',')?;
    Some([x.parse().ok()?,y.parse().ok()?])
}

/// A resource layer can contain multiple faces with different normals. Keep
/// per-triangle normal validation and resolve winding here, not in Java.
/// Reversing a triangle whose copied normal specifies the opposite front
/// preserves its intended culling without splitting one semantic mesh per face.
fn reconcile_model_mesh_topology(
    matrix: [f32; 16], vertices: &[GuiMeshPreparedVertex], indices: &[u32],
) -> GalResult<(Vec<u32>, super::resources::FrontFace)> {
    let front = transformed_front_face(matrix, vertices, indices)?;
    let mut resolved = indices.to_vec();
    for triangle in resolved.chunks_exact_mut(3) {
        let first = vertices[triangle[0] as usize].normal;
        for index in triangle.iter().copied() {
            if vertices[index as usize].normal.iter().zip(first)
                .any(|(actual, expected)| (actual-expected).abs()>0.000001) {
                return Err(GalError::unsupported_feature("flat item face has inconsistent copied normals"));
            }
        }
        if transformed_front_face(matrix, vertices, triangle)? != front {
            triangle.swap(1,2);
        }
    }
    Ok((resolved,front))
}

/// Packs exactly three vec4 values per vertex: position/local-U, local-V and
/// RGBA, then normal. This is a private streaming layout, deliberately not
/// part of the FFI ABI or a Java renderer contract.
pub fn packed_vertices(draw: &GuiMeshPreparedDraw) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(draw.vertices.len() * GUI_MESH_GPU_VERTEX_BYTES);
    for vertex in &draw.vertices {
        push_f32(&mut bytes, vertex.position[0]);
        push_f32(&mut bytes, vertex.position[1]);
        push_f32(&mut bytes, vertex.position[2]);
        push_f32(&mut bytes, vertex.local_uv[0]);
        push_f32(&mut bytes, vertex.local_uv[1]);
        push_f32(&mut bytes, vertex.color[0]);
        push_f32(&mut bytes, vertex.color[1]);
        push_f32(&mut bytes, vertex.color[2]);
        push_f32(&mut bytes, vertex.color[3]);
        push_f32(&mut bytes, vertex.normal[0]);
        push_f32(&mut bytes, vertex.normal[1]);
        push_f32(&mut bytes, vertex.normal[2]);
    }
    bytes
}

fn transform_point(matrix: [f32; 16], position: [f32; 3]) -> GalResult<[f32; 3]> {
    let result = [
        matrix[0] * position[0] + matrix[4] * position[1] + matrix[8] * position[2] + matrix[12],
        matrix[1] * position[0] + matrix[5] * position[1] + matrix[9] * position[2] + matrix[13],
        matrix[2] * position[0] + matrix[6] * position[1] + matrix[10] * position[2] + matrix[14],
    ];
    if result.iter().all(|value| value.is_finite()) {
        Ok(result)
    } else {
        Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "GUI mesh model transform produced a non-finite position",
        ))
    }
}

fn normalize_semantic_normal(normal: [f32; 3]) -> GalResult<[f32; 3]> {
    let length_squared = normal.iter().map(|value| value * value).sum::<f32>();
    if !length_squared.is_finite() || length_squared <= f32::EPSILON {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "GUI mesh contains a degenerate item-lighting-space normal",
        ));
    }
    let inverse_length = length_squared.sqrt().recip();
    Ok([
        normal[0] * inverse_length,
        normal[1] * inverse_length,
        normal[2] * inverse_length,
    ])
}

fn unpack_normal_i8(packed: u32) -> [f32; 3] {
    let component = |shift| {
        let value = ((packed >> shift) & 0xffu32) as u8 as i8;
        (value as f32 / 127.0).clamp(-1.0, 1.0)
    };
    [component(0), component(8), component(16)]
}

fn argb_to_rgba(color: u32) -> [f32; 4] {
    [
        ((color >> 16) & 0xff) as f32 / 255.0,
        ((color >> 8) & 0xff) as f32 / 255.0,
        (color & 0xff) as f32 / 255.0,
        ((color >> 24) & 0xff) as f32 / 255.0,
    ]
}

fn push_f32(bytes: &mut Vec<u8>, value: f32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::vulkanic::backends::{mock::MockBackend, vulkan_capabilities};
    use crate::render::vulkanic::commands::{CommandList, CommandListDesc, SubmissionBatch};

    fn batch() -> GuiMeshBatchRequest {
        GuiMeshBatchRequest {
            item_cache: None,
            block_item_raster: None,
            item_raster_scale: 0,
            item_lighting: None,
            item_foil: None,
            decal_foil: None,
            stratum: 420,
            layer_index: 0,
            sequence: 1,
            asset_id: 7,
            material_mode: GuiMeshMaterialMode::Cutout,
            lighting_mode: GuiMeshLightingMode::Block,
            alpha_cutoff: 0.5,
            model_transform: [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
            gui_pose: [1.0, 0.0, 0.0, 1.0, 12.0, 34.0],
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
    fn enchanted_native_block_uses_semantic_face_normal_not_baked_vertex_normal() {
        let mut native=batch();
        native.block_item_raster=Some(GuiBlockItemRaster {
            model_min:[-0.5;3],model_max:[0.5;3],gui_scale:2,oversized_gui:false });
        native.render_extent=[0,0];native.guard_pixels=0;
        native.lighting_mode=GuiMeshLightingMode::InventoryBlock;
        native.resolve_item_lighting(Some(flat_lightmap())).unwrap();
        for vertex in &mut native.vertices {
            vertex.normal_packed=0x005a_005a;
            vertex.source_face=4; // Original SOUTH baked direction.
        }
        let plain=prepare_draws(&[native.clone()]).unwrap();
        assert!(plain[0].vertices[0].normal[0]>0.7 && plain[0].vertices[0].normal[2]>0.7);
        for vertex in &mut native.vertices {vertex.source_foil_type=1;}
        let enchanted=prepare_draws(&[native.clone()]).unwrap();
        assert!(enchanted[0].vertices.iter().all(|v|v.normal==[0.,0.,1.]));
        assert_eq!(plain[0].vertices[0].position,enchanted[0].vertices[0].position);
        assert_eq!(plain[0].vertices[0].color,enchanted[0].vertices[0].color);
        for face in [0,7] {
            let mut invalid=native.clone();invalid.vertices[0].source_face=face;
            assert!(prepare_draws(&[invalid]).is_err());
        }
        let mut special=native.clone();special.vertices[0].source_foil_type=2;
        assert!(prepare_draws(&[special]).is_err());
        native.block_item_raster=None;native.render_extent=[34,34];native.guard_pixels=1;
        assert!(prepare_draws(&[native]).is_err(),"face metadata is original model space, not a legacy resolved normal");
    }

    #[test]
    fn native_block_layout_resolves_raster_normals_and_lightmap_without_caller_raster_state() {
        let mut native = batch();
        native.block_item_raster = Some(GuiBlockItemRaster {
            model_min: [-0.5;3], model_max: [0.5;3], gui_scale: 2, oversized_gui:false });
        native.render_extent=[0,0]; native.guard_pixels=0;
        native.lighting_mode=GuiMeshLightingMode::InventoryBlock;
        native.model_transform[12]=-0.5;native.model_transform[13]=-0.5;
        assert!(native.resolve_item_lighting(None).is_err());
        native.resolve_item_lighting(Some(flat_lightmap())).unwrap();
        let draw=prepare_draw(&native).unwrap();
        assert_eq!(draw.render_extent,[34,34]); assert_eq!(draw.guard_pixels,1);
        assert_eq!(draw.vertices[0].position,[1.,33.,0.]);
        assert_eq!(draw.vertices[1].position,[33.,33.,0.]);
        assert_eq!(draw.vertices[2].position,[1.,1.,0.]);
        assert_eq!(draw.vertices[0].normal,[0.,0.,1.]);
        // The legacy explicit request supplies the same resolved raster pose;
        // native layout must not change the resulting geometry or resource extent.
        let mut explicit=native.clone();explicit.block_item_raster=None;
        explicit.render_extent=draw.render_extent;explicit.guard_pixels=1;
        explicit.model_transform=resolved_item_raster(&native).unwrap().1;
        let old=prepare_draw(&explicit).unwrap();
        assert_eq!(draw.vertices,old.vertices);
        assert_eq!(draw.render_extent,old.render_extent);
        // Mutating semantic layout inputs changes the actual prepared target.
        native.block_item_raster.as_mut().unwrap().gui_scale=3;
        assert_eq!(prepare_draw(&native).unwrap().render_extent,[50,50]);
        // A genuine 3D X rotation: Frozen's signed-normalized packed item normal
        // is (0,63,109) after the GUI reflection, not the original +Z direction.
        let (s,c)=30.0_f32.to_radians().sin_cos();
        native.model_transform=[1.,0.,0.,0.,0.,c,s,0.,0.,-s,c,0.,-0.5,-0.5*c,-0.5*s,1.];
        let rotated=prepare_draw(&native).unwrap();
        let expected=[0.,63./127.,109./127.];
        for axis in 0..3 { assert!((rotated.vertices[0].normal[axis]-expected[axis]).abs()<1e-6); }
    }

    #[test]
    fn native_block_layout_rejects_mixed_spaces_and_missing_foil_but_preserves_clipping() {
        let mut native=batch();
        native.block_item_raster=Some(GuiBlockItemRaster {
            model_min:[-0.5;3],model_max:[0.5;3],gui_scale:2, oversized_gui:false });
        native.render_extent=[0,0];native.guard_pixels=0;
        native.lighting_mode=GuiMeshLightingMode::InventoryBlock;
        native.model_transform[12]=-0.5;native.model_transform[13]=-0.5;
        native.resolve_item_lighting(Some(flat_lightmap())).unwrap();
        validate_batch(&native).unwrap();
        let mut bad=native.clone();bad.block_item_raster.as_mut().unwrap().model_min[0]=f64::NAN;
        assert!(validate_batch(&bad).is_err());
        let mut bad=native.clone();bad.block_item_raster.as_mut().unwrap().model_min[0]=2.;
        assert!(validate_batch(&bad).is_err());
        let mut bad=native.clone();bad.guard_pixels=1;assert!(validate_batch(&bad).is_err());
        let mut bad=native.clone();bad.render_extent=[34,34];assert!(validate_batch(&bad).is_err());
        let mut bad=native.clone();bad.item_raster_scale=2;assert!(validate_batch(&bad).is_err());
        let mut bad=native.clone();bad.lighting_mode=GuiMeshLightingMode::Block;assert!(validate_batch(&bad).is_err());
        let mut bad=native.clone();bad.material_mode=GuiMeshMaterialMode::Glint;bad.lighting_mode=GuiMeshLightingMode::Flat;
        assert!(validate_batch(&bad).is_err());
        let mut extended=native.clone();extended.vertices[0].position[0]=10.;
        extended.block_item_raster.as_mut().unwrap().model_max=[10.;3];
        let draw=prepare_draw(&extended).unwrap();
        assert_eq!(draw.render_extent,[34,34]);
        assert_eq!(draw.vertices[0].position[0],321.);
        // Neither large bounds nor off-center bounds may shrink/recenter an
        // ordinary GUI model. Finite vertices remain available for GPU clipping.
        extended.block_item_raster.as_mut().unwrap().model_min=[2.;3];
        assert_eq!(prepare_draw(&extended).unwrap().vertices,draw.vertices);
        let mut bad=native;bad.model_transform[0]=0.;assert!(prepare_draw(&bad).is_err());
    }

    #[test]
    fn mesh_batches_validate_one_coarse_item_layer() {
        validate_batches(&[batch()]).expect("bounded copied GUI mesh is valid");
    }

    #[test]
    fn mesh_batches_reject_invalid_geometry_and_sequences() {
        let mut out_of_range = batch();
        out_of_range.indices[2] = 9;
        assert!(validate_batch(&out_of_range).is_err());

        let mut invalid_cutoff = batch();
        invalid_cutoff.alpha_cutoff = f32::NAN;
        assert!(validate_batch(&invalid_cutoff).is_err());

        let mut guard_consumes_target = batch();
        guard_consumes_target.guard_pixels = 17;
        assert!(validate_batch(&guard_consumes_target).is_err());

        let mut oversized_target = batch();
        oversized_target.render_extent = [GUI_MESH_MAX_OFFSCREEN_AXIS + 1, 32];
        assert!(validate_batch(&oversized_target).is_err());

        let mut oversized_gui = batch();
        oversized_gui.gui_extent = [GUI_MAX_VIEWPORT_AXIS as u32 + 1, 32];
        assert!(validate_batch(&oversized_gui).is_err());

        let mut second_layer = batch();
        second_layer.layer_index = 1;
        validate_batches(&[batch(), second_layer])
            .expect("contiguous item layers share one sequence");

        let duplicate_layer = batch();
        assert!(validate_batches(&[batch(), duplicate_layer]).is_err());

        let mut flat = batch();
        flat.lighting_mode = GuiMeshLightingMode::Flat;
        validate_batch(&flat).expect("flat item lighting remains an explicit mesh semantic");
    }

    #[test]
    fn prepared_mesh_vertices_are_owned_transformed_and_normally_packed() {
        let mut source = batch();
        source.model_transform[0] = 2.0;
        source.model_transform[12] = 4.0;
        let prepared = prepare_draws(&[source]).expect("prepare copied mesh");
        let draw = &prepared[0];
        assert_eq!([4.0, 0.0, 0.0], draw.vertices[0].position);
        assert_eq!([0.0, 0.0, 1.0], draw.vertices[0].normal);
        assert_eq!([1.0, 1.0, 1.0, 1.0], draw.vertices[0].color);
        assert_eq!(GUI_MESH_GPU_VERTEX_BYTES * 3, packed_vertices(draw).len());

        let mut degenerate = batch();
        degenerate.model_transform[0] = 0.0;
        assert!(prepare_draws(&[degenerate]).is_err());
    }

    #[test]
    fn packed_rgba_lanes_match_both_backend_shader_decoders() {
        let mut source = batch();
        source.vertices[0].color_argb = 0x8040_80c0;
        let prepared = prepare_draws(&[source]).expect("prepare copied mesh");
        let packed = packed_vertices(&prepared[0]);
        let words = packed
            .chunks_exact(std::mem::size_of::<f32>())
            .map(|word| f32::from_le_bytes(word.try_into().unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(
            words[5..9],
            [64.0 / 255.0, 128.0 / 255.0, 192.0 / 255.0, 128.0 / 255.0]
        );
        let decode = "vec4(uv_color_rg.y, uv_color_rg.z, uv_color_rg.w, color_ba_normal.x)";
        assert!(std::str::from_utf8(GUI_MESH_VERTEX_SHADER_OPENGL)
            .unwrap()
            .contains(decode));
        assert!(std::str::from_utf8(GUI_MESH_VERTEX_SHADER_VULKAN)
            .unwrap()
            .contains(decode));
    }

    #[test]
    fn mesh_frame_uniform_carries_the_semantic_cutout_threshold() {
        let bytes = frame_uniform_bytes([34.0, 18.0], 0.5, GuiMeshLightingMode::Block);
        let values = bytes
            .chunks_exact(std::mem::size_of::<f32>())
            .map(|word| f32::from_le_bytes(word.try_into().unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(&values[..4], &[34.0, 18.0, 0.5, 1.0]);
        assert_eq!(values.len(), 12);
        assert_eq!(&values[4..7], &[-0.933_439_2, 0.262_694_72, -0.244_300_16]);
    }

    #[test]
    fn opaque_mesh_uniform_disables_discard_without_changing_cutout_contract() {
        let mut request = batch();
        request.material_mode = GuiMeshMaterialMode::Opaque;
        request.alpha_cutoff = 0.0;
        let draw = prepare_draw(&request).unwrap();
        let bytes = draw_frame_uniform_bytes(&draw, [34.0, 18.0]);
        assert_eq!(f32::from_le_bytes(bytes[8..12].try_into().unwrap()), -1.0);
        request.material_mode = GuiMeshMaterialMode::Cutout;
        request.alpha_cutoff = 0.1;
        let draw = prepare_draw(&request).unwrap();
        let bytes = draw_frame_uniform_bytes(&draw, [34.0, 18.0]);
        assert_eq!(f32::from_le_bytes(bytes[8..12].try_into().unwrap()), 0.1);
    }

    #[test]
    fn inventory_block_lights_match_frozen_opengl_top_face_space() {
        let uniforms = |mode| frame_uniform_bytes([34.0, 34.0], 0.1, mode)
            .chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect::<Vec<_>>();
        let inventory = uniforms(GuiMeshLightingMode::InventoryBlock);
        let upright = uniforms(GuiMeshLightingMode::Block);
        assert_eq!(inventory[3], 2.0);
        assert_eq!(uniforms(GuiMeshLightingMode::Flat)[3], 0.0);
        for i in [4, 6, 8, 10] { assert_eq!(inventory[i], upright[i]); }
        for i in [5, 9] { assert_eq!(inventory[i], -upright[i]); }
        // Independently evaluated with Frozen's JOML matrix sequence and
        // block/block.json GUI rotation [30,225,0], scale .625. The ordinary
        // OpenGL atlas pose scale(k,-k,k) makes the top normal point down in
        // GUI coordinates. Do not compare against Frozen's Vulkan PIP path.
        let shade = |u: &[f32], n: [f32; 3]| {
            let a = dot([u[4],u[5],u[6]], n).max(0.0);
            let b = dot([u[8],u[9],u[10]], n).max(0.0);
            ((a+b)*0.6+0.4).min(1.0)
        };
        let top = [0.0, -0.8660254, 0.5];
        assert!((shade(&inventory, top)-1.0).abs() < 1e-6);
        assert!((shade(&upright, top)-0.4).abs() < 1e-6);
        let side = [0.70710677,-0.35355338,-0.6123724];
        assert!((shade(&inventory, side)-0.4939883).abs() < 1e-6);
        assert!(shade(&inventory, top) > shade(&inventory, side));
    }

    #[test]
    fn mesh_composite_preserves_fractional_projection_and_integer_layout() {
        let mut request = batch();
        request.projection_extent = [319.75, 179.5];
        let draw = prepare_draw(&request).unwrap();
        assert_eq!([320, 180], draw.gui_extent);
        let bytes = composite_uniform_bytes(&draw);
        assert_eq!(319.75, f32::from_le_bytes(bytes[24..28].try_into().unwrap()));
        assert_eq!(179.5, f32::from_le_bytes(bytes[28..32].try_into().unwrap()));
    }

    #[test]
    fn offscreen_targets_are_rust_owned_generation_and_extent_resources() {
        let gal_capabilities = vulkan_capabilities();
        let mut gal = VulkanicGal::new_with_backend(
            Box::new(MockBackend::with_capabilities(gal_capabilities)),
            false,
        );
        let mut cache = GuiMeshOffscreenTargetCache::default();
        let extent = Extent3d {
            width: 16,
            height: 16,
            depth: 1,
        };
        let first = cache.stage(&mut gal, 1, extent).expect("stage target");
        let reused = cache.stage(&mut gal, 1, extent).expect("reuse target");
        assert_eq!(first.target, reused.target);
        assert_eq!(first.color_view, reused.color_view);

        let replacement = cache
            .stage(
                &mut gal,
                2,
                Extent3d {
                    width: 32,
                    height: 16,
                    depth: 1,
                },
            )
            .expect("replace target generation");
        assert_ne!(first.target, replacement.target);
        cache.clear(&mut gal);
    }

    #[test]
    fn item_cache_names_isolate_pixels_and_unsubmitted_rasters_are_not_accepted() {
        let mut gal = VulkanicGal::new_with_backend(
            Box::new(MockBackend::with_capabilities(vulkan_capabilities())), false);
        let mut cache = GuiMeshOffscreenTargetCache::default();
        let extent = Extent3d { width: 16, height: 16, depth: 1 };
        let first = cache.stage_item(&mut gal, 1, extent, 1).unwrap();
        let other = cache.stage_item(&mut gal, 1, extent, 2).unwrap();
        assert_ne!(first.color_view, other.color_view);
        assert_eq!(first.target, cache.stage_item(&mut gal, 1, extent, 1).unwrap().target);
        let predicted = gal.next_submission_id();
        // Prepared raster commands were discarded. An unrelated accepted
        // submission consumes their predicted ID, but proves no raster write.
        gal.submit(SubmissionBatch {
            label: "unrelated".into(),
            command_lists: vec![CommandList::from(CommandListDesc {
                label: "no raster".into(), operations: vec![],
            })],
        }).unwrap();
        assert_eq!(gal.latest_submission_id(), predicted);
        assert_eq!(gal.render_pass_last_submission(first.pass).unwrap(), None);
        let replacement = cache.stage_item(&mut gal, 2, extent, 1).unwrap();
        assert_ne!(first.color_view, replacement.color_view);
        assert_eq!(gal.render_pass_last_submission(replacement.pass).unwrap(), None);
        cache.clear(&mut gal);
    }

    #[test]
    fn item_cache_wire_policy_requires_a_named_native_raster() {
        assert_eq!(GuiItemCache::decode(0, 0).unwrap(), None);
        assert_eq!(GuiItemCache::decode(9, 1).unwrap(), Some(GuiItemCache { identity: 9, animated: false }));
        assert_eq!(GuiItemCache::decode(9, 2).unwrap(), Some(GuiItemCache { identity: 9, animated: true }));
        for (identity, mode) in [(0, 1), (0, 2), (1, 0), (1, 3)] {
            assert!(GuiItemCache::decode(identity, mode).is_err());
        }
        let mut request = batch();
        request.item_cache = GuiItemCache::decode(9, 1).unwrap();
        assert!(prepare_draws(&[request]).is_err(), "unnamed raster scale must not admit a pixel cache");
    }

    #[test]
    fn vertex_trace_selector_is_an_exact_logical_item_origin() {
        assert_eq!(parse_vertex_trace_selector("292,341"),Some([292,341]));
        for invalid in ["", "292", "292,341,0", "nan,0", "2147483648,0"] {
            assert_eq!(parse_vertex_trace_selector(invalid),None);
        }
    }

    #[test]
    fn offscreen_target_cache_rejects_unbounded_extent_variants() {
        let mut gal = VulkanicGal::new_with_backend(
            Box::new(MockBackend::with_capabilities(vulkan_capabilities())),
            false,
        );
        let mut cache = GuiMeshOffscreenTargetCache::default();
        for width in 1..=GUI_MESH_MAX_OFFSCREEN_TARGETS_PER_GENERATION as u32 {
            cache
                .stage(
                    &mut gal,
                    7,
                    Extent3d {
                        width,
                        height: 1,
                        depth: 1,
                    },
                )
                .expect("bounded GUI mesh target variant");
        }
        let rejected = cache.stage(
            &mut gal,
            7,
            Extent3d {
                width: GUI_MESH_MAX_OFFSCREEN_TARGETS_PER_GENERATION as u32 + 1,
                height: 1,
                depth: 1,
            },
        );
        assert!(rejected.is_err());
        assert_eq!(GUI_MESH_MAX_OFFSCREEN_TARGETS_PER_GENERATION, cache.len());
        cache.clear(&mut gal);
    }

    #[test]
    fn owned_mesh_pass_writes_only_to_its_matching_offscreen_target() {
        exercise_owned_mesh_pass(batch());
    }

    /// Differential diagnostic only, not a substitute for Frozen game parity.
    /// Positions are the actual r395 prepared west/east bush-face observations.
    #[test]
    fn captured_leaf_coplanar_foil_gpu_diagnostic() {
        for opengl in [false,true] {
            for offset in [0.0,2.0] {
                let backend:Box<dyn super::super::backends::Backend>=if opengl {
                    Box::new(super::super::backends::opengl::OpenGlBackend::new("leaf depth diagnostic").unwrap())
                } else {
                    Box::new(super::super::backends::vulkan::VulkanBackend::new("leaf depth diagnostic").unwrap())
                };
                let mut gal=VulkanicGal::new_with_backend(backend,false);
                let pixels=captured_leaf_depth_pixels(&mut gal,offset,34,[0.0,0.0],TextureFormat::Depth32Float,false,false,false);
                let mut histogram=BTreeMap::new();
                for pixel in pixels.chunks_exact(4) { *histogram.entry(pixel[0]).or_insert(0usize)+=1; }
                eprintln!("leaf-depth-diagnostic opengl={opengl} foil_z_offset={offset} red_histogram={histogram:?}");
                assert!(pixels.chunks_exact(4).any(|p|p[1]>=63),"base plane must actually render");
                if offset==0.0 {
                    assert!(pixels.chunks_exact(4).any(|p|p[0]>=63),"equal-depth foil must actually render");
                } else {
                    assert!(pixels.chunks_exact(4).all(|p|p[0]==0),"displaced foil must fail exact depth equality");
                }
            }
        }
    }

    /// Varies allocation/projection and placement independently, using the
    /// observed atlas translation, not a fitted alignment. Diagnostic only:
    /// native OpenGL is not the Frozen correctness baseline.
    #[test]
    fn captured_leaf_raster_placement_gpu_diagnostic() {
        for opengl in [false,true] {
          for depth_format in [TextureFormat::Depth32Float,TextureFormat::Depth24Stencil8] {
           for reference_positions in [false,true] {
            for (size,origin) in [(34,[0.0,0.0]),(64,[0.0,0.0]),
                                  (512,[0.0,0.0]),(512,[63.0,-1.0])] {
                let backend:Box<dyn super::super::backends::Backend>=if opengl {
                    Box::new(super::super::backends::opengl::OpenGlBackend::new("leaf placement diagnostic").unwrap())
                } else {
                    Box::new(super::super::backends::vulkan::VulkanBackend::new("leaf placement diagnostic").unwrap())
                };
                let mut gal=VulkanicGal::new_with_backend(backend,false);
                let pixels=captured_leaf_depth_pixels(&mut gal,0.0,size,origin,depth_format,reference_positions,false,false);
                let mut histogram=BTreeMap::new();
                let left=(origin[0]+1.0) as usize;
                let top=(origin[1]+1.0) as usize;
                let mut base_pixels=0;
                for y in top..top+32 { for x in left..left+32 {
                    let pixel=&pixels[(y*size as usize+x)*4..][..4];
                    *histogram.entry(pixel[0]).or_insert(0usize)+=1;
                    base_pixels+=usize::from(pixel[1]>=63);
                }}
                assert_eq!(base_pixels,400,"placement and stencil clear must preserve the entire captured plane footprint");
                assert!(histogram.keys().any(|red|*red>=63),"placement must preserve visible foil");
                eprintln!("leaf-placement-diagnostic opengl={opengl} depth={depth_format:?} reference_positions={reference_positions} size={size} origin={origin:?} base_pixels={base_pixels} red_histogram={histogram:?}");
            }
           }
          }
        }
    }

    #[test]
    fn captured_leaf_full_cutout_gpu_diagnostic() {
        for opengl in [false,true] {
          for depth in [TextureFormat::Depth32Float,TextureFormat::Depth24Stencil8] {
            for (size,origin) in [(34,[0.0,0.0]),(512,[63.0,-1.0])] {
                let backend:Box<dyn super::super::backends::Backend>=if opengl {
                    Box::new(super::super::backends::opengl::OpenGlBackend::new("full leaf diagnostic").unwrap())
                } else {
                    Box::new(super::super::backends::vulkan::VulkanBackend::new("full leaf diagnostic").unwrap())
                };
                let mut gal=VulkanicGal::new_with_backend(backend,false);
                let pixels=captured_leaf_depth_pixels(&mut gal,0.0,size,origin,depth,false,true,false);
                let mut histogram=BTreeMap::new();
                let left=(origin[0]+1.0) as usize;let top=(origin[1]+1.0) as usize;
                for y in top..top+32 { for x in left..left+32 {
                    let pixel=&pixels[(y*size as usize+x)*4..][..4];
                    *histogram.entry(pixel[0]).or_insert(0usize)+=1;
                }}
                assert!(histogram.contains_key(&0),"actual cutout must leave uncovered pixels");
                assert!(histogram.keys().any(|red|*red>=63),"full leaf model must receive visible foil");
                eprintln!("leaf-full-diagnostic opengl={opengl} depth={depth:?} size={size} origin={origin:?} red_histogram={histogram:?}");
            }
          }
        }
    }

    #[test]
    fn captured_leaf_pattern_gpu_diagnostic() {
        let observed:serde_json::Value=serde_json::from_str(include_str!("fixtures/gui_leaf_r389_effect.json")).unwrap();
        let references:[Vec<i32>;2]=["frozen_red","current_red"].map(|key|{
            let rows:Vec<Vec<i32>>=serde_json::from_value(observed[key].clone()).unwrap();
            assert_eq!(rows.len(),32);assert!(rows.iter().all(|row|row.len()==32));
            rows.into_iter().flatten().collect()
        });
        for opengl in [false,true] {
          for depth in [TextureFormat::Depth32Float,TextureFormat::Depth24Stencil8] {
            // The early trace used64,0; the actual r389/r394 frame receipts
            // locate the leaf at352,0. Compare both explicitly.
            for (size,origin) in [(34,[0.0,0.0]),(512,[63.0,-1.0]),(512,[351.0,-1.0])] {
                let backend:Box<dyn super::super::backends::Backend>=if opengl {
                    Box::new(super::super::backends::opengl::OpenGlBackend::new("pattern leaf diagnostic").unwrap())
                } else {
                    Box::new(super::super::backends::vulkan::VulkanBackend::new("pattern leaf diagnostic").unwrap())
                };
                let mut gal=VulkanicGal::new_with_backend(backend,false);
                let pixels=captured_leaf_depth_pixels(&mut gal,0.0,size,origin,depth,false,true,true);
                let left=(origin[0]+1.0) as usize;let top=(origin[1]+1.0) as usize;
                let actual:Vec<i32>=(top..top+32).flat_map(|y|(left..left+32).map(move|x|(y*size as usize+x)*4))
                    .map(|index|i32::from(pixels[index])).collect();
                assert!(actual.iter().any(|v|*v>8));
                let errors=references.each_ref().map(|reference|{
                    let delta:Vec<i32>=actual.iter().zip(reference).map(|(a,b)|(a-b).abs()).collect();
                    (delta.iter().sum::<i32>() as f64/1024.0,*delta.iter().max().unwrap(),delta.iter().filter(|v|**v>8).count())
                });
                eprintln!("leaf-pattern-diagnostic opengl={opengl} depth={depth:?} size={size} origin={origin:?} errors_frozen_current={errors:?}");
            }
          }
        }
    }

    fn captured_leaf_depth_pixels(gal:&mut VulkanicGal,foil_offset:f32,size:u32,origin:[f32;2],depth_format:TextureFormat,reference_positions:bool,full_model:bool,patterned:bool)->Vec<u8> {
        captured_leaf_depth_pixels_at_clock(gal,foil_offset,size,origin,depth_format,reference_positions,full_model,patterned,
            GuiItemFoil{kind:super::super::item_foil::StandardFoilKind::Item,clock_millis:0,speed:0.0,strength:0.5},None,None)
    }

    #[test]
    fn captured_leaf_moving_timing_gpu_diagnostic() {
        let observed:serde_json::Value=serde_json::from_str(include_str!("fixtures/gui_leaf_r394_effect.json")).unwrap();
        let references:[Vec<[i32;3]>;3]=["frozen","current","base_delta"].map(|key|{
            let rows:Vec<Vec<[i32;3]>>=serde_json::from_value(observed[key].clone()).unwrap();
            assert_eq!(rows.len(),32);assert!(rows.iter().all(|row|row.len()==32));
            rows.into_iter().flatten().collect()
        });
        let frozen_ticks=observed["frozen_scaled_ticks"].as_u64().unwrap();
        assert_eq!(frozen_ticks%4,0);
        let clocks=[frozen_ticks/4,observed["current_clock_millis"].as_u64().unwrap()];
        for opengl in [false,true] {
          for depth in [TextureFormat::Depth32Float,TextureFormat::Depth24Stencil8] {
           for clock_millis in clocks {
                let backend:Box<dyn super::super::backends::Backend>=if opengl {
                    Box::new(super::super::backends::opengl::OpenGlBackend::new("moving leaf diagnostic").unwrap())
                } else {
                    Box::new(super::super::backends::vulkan::VulkanBackend::new("moving leaf diagnostic").unwrap())
                };
                let mut gal=VulkanicGal::new_with_backend(backend,false);
                let pixels=captured_leaf_depth_pixels_at_clock(&mut gal,0.0,34,[0.0,0.0],depth,false,true,true,
                    GuiItemFoil{kind:super::super::item_foil::StandardFoilKind::Item,clock_millis,speed:0.5,strength:0.5},None,None);
                let actual:Vec<[i32;3]>=(1..33).flat_map(|y|(1..33).map(move|x|(y*34+x)*4)).map(|index|{
                    let p=&pixels[index..][..4];
                    [i32::from(p[0]),i32::from(p[1])-if p[3]>0{64}else{0},i32::from(p[2])]
                }).collect();
                assert!(actual.iter().any(|p|p[0]>8));
                let errors:[([f64;3],i32);2]=[0,1].map(|reference|{
                    let mut sums=[0;3];let mut max=0;
                    for (index,pixel) in actual.iter().enumerate(){for c in 0..3 {
                        let delta=(pixel[c]-references[reference][index][c]).abs();
                        sums[c]+=delta;max=max.max(delta);
                    }}
                    (sums.map(|v|v as f64/1024.0),max)
                });
                let mut whole=[0;3];
                for (index,pixel) in actual.iter().enumerate(){for c in 0..3 {
                    whole[c]+=(pixel[c]-references[0][index][c]+references[2][index][c]).abs();
                }}
                eprintln!("leaf-moving-diagnostic opengl={opengl} depth={depth:?} clock_millis={clock_millis} effect_errors_frozen_current={errors:?} whole_mean_channels={:?}",whole.map(|v|v as f64/1024.0));
           }
          }
        }
    }

    #[test]
    fn inventory_lighting_gpu_preserves_frozen_packed_normal_magnitude() {
        // Frozen BufferBuilder truncates normalized components to signed i8;
        // entity.vsh/light.glsl dot that decoded value without renormalizing.
        let normal=[0.0,-63.0/127.0,109.0/127.0];
        let mut request=batch();
        request.lighting_mode=GuiMeshLightingMode::InventoryBlock;
        request.resolve_item_lighting(Some(flat_lightmap())).unwrap();
        for vertex in &mut request.vertices {vertex.normal_packed=0x006d_c100;}
        assert!(prepare_draw(&request).unwrap().vertices.iter().all(|vertex|vertex.normal==normal));
        for opengl in [false,true] {
            let backend:Box<dyn super::super::backends::Backend>=if opengl {
                Box::new(super::super::backends::opengl::OpenGlBackend::new("packed normal lighting").unwrap())
            }else{
                Box::new(super::super::backends::vulkan::VulkanBackend::new("packed normal lighting").unwrap())
            };
            let mut gal=VulkanicGal::new_with_backend(backend,false);
            let pixels=captured_leaf_depth_pixels_at_clock(&mut gal,0.0,34,[0.0,0.0],TextureFormat::Depth32Float,false,false,false,
                GuiItemFoil{kind:super::super::item_foil::StandardFoilKind::Item,clock_millis:0,speed:0.0,strength:0.0},Some(normal),None);
            let visible:Vec<_>=pixels.chunks_exact(4).filter(|p|p[3]!=0).collect();
            assert!(!visible.is_empty());
            assert!(visible.iter().all(|p|p[..3]==[201,201,201]),"raw packed normal must produce Frozen lighting201, not renormalized202");
        }
    }

    #[test]
    fn captured_leaf_face_contributions_gpu_diagnostic() {
        let observed:serde_json::Value=serde_json::from_str(include_str!("fixtures/gui_leaf_r394_effect.json")).unwrap();
        let rows:Vec<Vec<[i32;3]>>=serde_json::from_value(observed["frozen"].clone()).unwrap();
        let frozen:Vec<_>=rows.into_iter().flatten().collect();
        assert_eq!(frozen.len(),1024);
        let foil=GuiItemFoil{kind:super::super::item_foil::StandardFoilKind::Item,
            clock_millis:observed["frozen_scaled_ticks"].as_u64().unwrap()/4,speed:0.5,strength:0.5};
        let mut gal=VulkanicGal::new_with_backend(Box::new(
            super::super::backends::vulkan::VulkanBackend::new("leaf face contributions").unwrap()),false);
        let effect=|pixels:Vec<u8>|->Vec<[i32;3]>{
            (1..33).flat_map(|y|(1..33).map(move|x|(y*34+x)*4)).map(|index|{
                let p=&pixels[index..][..4];
                [i32::from(p[0]),i32::from(p[1])-if p[3]>0{64}else{0},i32::from(p[2])]
            }).collect()
        };
        let full=effect(captured_leaf_depth_pixels_at_clock(&mut gal,0.0,34,[0.0,0.0],
            TextureFormat::Depth32Float,false,true,true,foil,None,None));
        let mut equal=Vec::new();let mut unoccluded=Vec::new();
        for always in [false,true] {for face in 0..10 {
            let pixels=effect(captured_leaf_depth_pixels_at_clock(&mut gal,0.0,34,[0.0,0.0],
                TextureFormat::Depth32Float,false,true,true,foil,None,Some((face,always))));
            if always{unoccluded.push(pixels);}else{equal.push(pixels);}
        }}
        let mut unexplained=0;let mut differing_masks=0;let mut max_error=0;
        let mut transitions=BTreeMap::new();
        for pixel in 0..1024 {
            let sum:[i32;3]=std::array::from_fn(|c|equal.iter().map(|face|face[pixel][c]).sum());
            assert!((0..3).all(|c|(sum[c]-full[pixel][c]).abs()<=1),"per-face decomposition must reconstruct the actual full draw");
            let equal_mask=equal.iter().enumerate().fold(0u16,|mask,(face,values)|
                mask|if values[pixel].iter().any(|v|*v>1){1<<face}else{0});
            // Try zero, one or two visible source faces. The full diagnostic
            // observes at most two contributions; this is classification, NOT
            // a replacement depth rule or a visual acceptance gate.
            let mut best=(i32::MAX,i32::MAX,0u16);
            for a in 0..=10 {for b in a..=10 {
                if a==b && a<10 {continue;}
                let candidate:[i32;3]=std::array::from_fn(|c|
                    if a<10{unoccluded[a][pixel][c]}else{0} + if b<10{unoccluded[b][pixel][c]}else{0});
                let delta:[i32;3]=std::array::from_fn(|c|(candidate[c]-frozen[pixel][c]).abs());
                let mask=if a<10{1<<a}else{0}|if b<10{1<<b}else{0};
                let score=(*delta.iter().max().unwrap(),delta.iter().sum(),mask);
                if score<best{best=score;}
            }}
            max_error=max_error.max(best.0);
            if best.0>1 {unexplained+=1;}else if best.2!=equal_mask {
                differing_masks+=1;
                *transitions.entry((equal_mask,best.2)).or_insert(0usize)+=1;
            }
        }
        eprintln!("leaf-face-diagnostic unexplained_pixels={unexplained} max_best_rgb_error={max_error} differing_masks={differing_masks} transitions={transitions:?}");
    }

    fn captured_leaf_depth_pixels_at_clock(gal:&mut VulkanicGal,foil_offset:f32,size:u32,origin:[f32;2],depth_format:TextureFormat,reference_positions:bool,full_model:bool,patterned:bool,foil_animation:GuiItemFoil,lighting_probe:Option<[f32;3]>,face_probe:Option<(usize,bool)>)->Vec<u8> {
        use super::super::commands::{BufferImageCopyRegion,TextureOrigin3d};
        use super::super::resources::{SamplerDesc,SamplerFilter,SamplerAddressMode};
        let extent=Extent3d{width:size,height:size,depth:1};
        let byte_count=u64::from(size)*u64::from(size)*4;
        let texture=gal.create_texture(TextureDesc{label:"leaf-diagnostic.white".into(),dimension:TextureDimension::D2,
            format:TextureFormat::Rgba8Unorm,extent:Extent3d{width:1,height:1,depth:1},mip_levels:1,array_layers:1,
            usages:vec![TextureUsage::Sampled,TextureUsage::TransferDst]}).unwrap();
        let view=gal.create_texture_view(TextureViewDesc{label:"leaf-diagnostic.view".into(),texture,
            format:TextureFormat::Rgba8Unorm,base_mip:0,mip_count:1,base_layer:0,layer_count:1}).unwrap();
        let sampler=gal.create_sampler(SamplerDesc{label:"leaf-diagnostic.sampler".into(),min_filter:SamplerFilter::Nearest,
            mag_filter:SamplerFilter::Nearest,mip_filter:SamplerFilter::Nearest,address_u:SamplerAddressMode::Repeat,
            address_v:SamplerAddressMode::Repeat,address_w:SamplerAddressMode::Repeat,comparison:None}).unwrap();
        let upload=gal.create_buffer(BufferDesc{label:"leaf-diagnostic.upload".into(),size:4,memory:MemoryDomain::Upload,
            usages:vec![BufferUsage::HostWrite,BufferUsage::TransferSrc]}).unwrap();
        let readback=gal.create_buffer(BufferDesc{label:"leaf-diagnostic.readback".into(),size:byte_count,memory:MemoryDomain::Readback,
            usages:vec![BufferUsage::TransferDst,BufferUsage::HostRead]}).unwrap();
        let barrier=|resource,before,after|CommandOp::Barrier(ResourceBarrier{resource,subresources:None,before,after,
            src_queue:QueueClass::Graphics,dst_queue:QueueClass::Graphics});
        let mut operations=vec![CommandOp::HostWriteBuffer{buffer:upload,offset:0,data:vec![255;4]},
            barrier(upload,TextureUsageState::TransferDst,TextureUsageState::TransferSrc),
            barrier(texture,TextureUsageState::Undefined,TextureUsageState::TransferDst),
            CommandOp::CopyBufferToTexture(BufferImageCopyRegion{buffer:upload,buffer_offset:0,bytes_per_row:4,rows_per_image:1,
                texture,texture_mip:0,texture_layer:0,texture_origin:TextureOrigin3d{x:0,y:0,z:0},extent:Extent3d{width:1,height:1,depth:1}}),
            barrier(texture,TextureUsageState::TransferDst,TextureUsageState::ShaderRead)];
        let mut alpha_resources=Vec::new();
        let glint=if patterned {
            let bytes=(0..16).flat_map(|y|(0..16).flat_map(move|x|
                [64+(13*x+7*y)%128,48+(5*x+17*y)%144,80+(11*x+3*y)%112,255].map(|v|v as u8))).collect();
            let view=leaf_diagnostic_rgba_texture(gal,bytes,16,&mut operations,&mut alpha_resources);
            let sampler=gal.create_sampler(SamplerDesc{label:"leaf-diagnostic.linear-foil".into(),
                min_filter:SamplerFilter::Linear,mag_filter:SamplerFilter::Linear,mip_filter:SamplerFilter::Nearest,
                address_u:SamplerAddressMode::Repeat,address_v:SamplerAddressMode::Repeat,address_w:SamplerAddressMode::Repeat,
                comparison:None}).unwrap();
            alpha_resources.push(sampler);
            Some((view,sampler))
        }else{None};
        let alpha_views=if full_model {
            Some([
                leaf_diagnostic_alpha_texture(gal,include_bytes!("../../../resources/assets/minecraft/textures/block/oak_leaves_bushy.png"),32,&mut operations,&mut alpha_resources),
                leaf_diagnostic_alpha_texture(gal,include_bytes!("../../../resources/assets/minecraft/textures/block/oak_leaves.png"),16,&mut operations,&mut alpha_resources),
            ])
        }else{None};
        let mut targets=GuiMeshOffscreenTargetCache::default();
        let mut target=targets.stage(gal,1,extent).unwrap();
        let mut diagnostic_depth_resources=Vec::new();
        if depth_format!=TextureFormat::Depth32Float {
            // Test-only format isolation. Keep the production cache policy intact.
            let depth=gal.create_texture(TextureDesc{label:"leaf-diagnostic.depth".into(),
                dimension:TextureDimension::D2,format:depth_format,extent,mip_levels:1,array_layers:1,
                usages:vec![TextureUsage::DepthStencilAttachment]}).unwrap();
            let depth_view=gal.create_texture_view(TextureViewDesc{label:"leaf-diagnostic.depth-view".into(),
                texture:depth,format:depth_format,base_mip:0,mip_count:1,base_layer:0,layer_count:1}).unwrap();
            let render_target=gal.create_render_target(RenderTargetDesc{label:"leaf-diagnostic.target".into(),
                color_views:vec![target.color_view],depth_stencil_view:Some(depth_view),extent}).unwrap();
            let pass=gal.create_render_pass(RenderPassDesc{label:"leaf-diagnostic.pass".into(),target:render_target,
                color_formats:vec![TextureFormat::Rgba8Unorm],depth_format:Some(depth_format)}).unwrap();
            target=GuiMeshOffscreenTarget{depth,depth_view,target:render_target,pass,..target};
            diagnostic_depth_resources.extend([pass,render_target,depth_view,depth]);
        }
        let positions=[[24.91278,11.176079,24.912676],[24.91278,41.48697,7.412678],
            [9.6054535,23.00938,-24.59145],[9.6054535,-7.3015137,-7.0914507]];
        // Encoded float32 observations from Frozen r395, before atlas sampling.
        let observed=[[87.91278,10.176079,24.912676],[87.91278,40.48697,7.4126773],
            [72.60545,22.00938,-24.59145],[72.60545,-8.301512,-7.091451]];
        let mut resources=Vec::new();
        let mut requests=Vec::new();
        let fixture:serde_json::Value=serde_json::from_str(include_str!("fixtures/gui_leaf_r395.json")).unwrap();
        let quads:Vec<[[f32;5];4]>=serde_json::from_value(fixture["vertices"].clone()).unwrap();
        assert_eq!(quads.len(),10);
        let base_count=if full_model{10}else{2};
        for layer in 0..base_count*2 {
            let foil=layer>=base_count;let reverse=layer%2==1;
            let quad_index=(layer%base_count) as usize;
            let mut request=batch();request.layer_index=layer;
            request.render_extent=[size,size];
            request.lighting_mode=GuiMeshLightingMode::Flat;
            request.material_mode=if foil {GuiMeshMaterialMode::Glint} else {GuiMeshMaterialMode::Cutout};
            request.alpha_cutoff=0.1;
            if foil {request.item_foil=Some(foil_animation);}
            request.vertices=(0..4).map(|i| {
                let index=if reverse {3-i}else{i};
                let mut position=if reference_positions {observed[index]}else{positions[index]};
                if full_model {position=quads[quad_index][i][..3].try_into().unwrap();}
                position[0]+=origin[0]-if reference_positions{63.0}else{0.0};
                position[1]+=origin[1]+if reference_positions{1.0}else{0.0};
                if foil {position[2]+=foil_offset;}
                let atlas_uv=if full_model{[quads[quad_index][i][3],quads[quad_index][i][4]]}else{[0.0;2]};
                // Atlas regions were observed in Frozen/current r395. Keep
                // original UVs for foil; local UVs select only source alpha.
                let (u,v,w,h)=if quad_index<4{(0.26953125,0.921875,0.0078125,0.015625)}
                    else{(0.42578125,0.5546875,0.00390625,0.0078125)};
                let local_uv=if full_model{[(atlas_uv[0]-u)/w,(atlas_uv[1]-v)/h]}else{[0.0;2]};
                GuiMeshVertex{position,atlas_uv,local_uv,color_argb:0xff004000,
                    normal_packed:if reverse{0x00810000}else{0x007f0000},source_face:0,source_foil_type:0}
            }).collect();
            request.indices=vec![0,1,2,2,3,0];
            requests.push(request);
        }
        for (layer,mut draw) in prepare_draws(&requests).unwrap().into_iter().enumerate() {
            if layer>=base_count as usize && face_probe.is_some_and(|(face,_)|layer-base_count as usize!=face) {continue;}
            if draw.material_mode!=GuiMeshMaterialMode::Glint {
                if let Some(normal)=lighting_probe {
                    draw.lighting_mode=GuiMeshLightingMode::InventoryBlock;
                    for vertex in &mut draw.vertices {vertex.normal=normal;vertex.color=[1.0;4];}
                }
            }
            // Original baked winding under the captured reflected GUI pose.
            if full_model {draw.front_face=super::super::resources::FrontFace::CounterClockwise;}
            let view=if let Some(views)=alpha_views {
                if layer<4{views[0]}else if layer<10{views[1]}else{view}
            }else{view};
            let (view,sampler)=if draw.material_mode==GuiMeshMaterialMode::Glint {
                glint.unwrap_or((view,sampler))
            }else{(view,sampler)};
            let packed_depth=depth_format==TextureFormat::Depth24Stencil8;
            if packed_depth && layer==0 {
                // Write a nonzero stencil first. The following clear must erase
                // it before an Equal(0) draw; undefined initial zeros cannot pass
                // this regression by accident.
                for (index,face) in [
                    super::super::resources::StencilFaceState::replace(7,0xff,0xff),
                    // Leave writes disabled to exercise clear's private GL
                    // mask restoration, not just its clear value.
                    super::super::resources::StencilFaceState::keep(CompareOp::Equal,7,0xff),
                ].into_iter().enumerate() {
                    let program=leaf_diagnostic_program(gal,&draw,depth_format,Some(face),None);
                    let mut prefill=GuiMeshPassResources::create_with_shared_program(gal,"leaf-diagnostic.prefill",
                        view,sampler,program).unwrap();
                    prefill.owned_program=Some(program);
                    prefill.append_draw(target,&draw,GuiMeshStreamRange::default(),index==0,&mut operations).unwrap();
                    resources.push(prefill);
                }
                operations.push(barrier(target.color,TextureUsageState::ColorAttachment,TextureUsageState::ShaderRead));
                target.initialized=true;
            }
            let stencil=packed_depth.then_some(super::super::resources::StencilFaceState::keep(CompareOp::Equal,0,0xff));
            let depth_override=(draw.material_mode==GuiMeshMaterialMode::Glint && face_probe.is_some_and(|(_,always)|always))
                .then_some(CompareOp::Always);
            let program=leaf_diagnostic_program(gal,&draw,depth_format,stencil,depth_override);
            let mut resource=GuiMeshPassResources::create_with_shared_program(gal,"leaf-diagnostic.draw",
                view,sampler,program).unwrap();
            resource.owned_program=Some(program);
            resource.append_draw(target,&draw,GuiMeshStreamRange::default(),layer==0,&mut operations).unwrap();
            resources.push(resource);
        }
        operations.extend([barrier(target.color,TextureUsageState::ColorAttachment,TextureUsageState::TransferSrc),
            CommandOp::CopyTextureToBuffer(BufferImageCopyRegion{buffer:readback,buffer_offset:0,bytes_per_row:size*4,rows_per_image:size,
                texture:target.color,texture_mip:0,texture_layer:0,texture_origin:TextureOrigin3d{x:0,y:0,z:0},extent}),
            barrier(readback,TextureUsageState::TransferDst,TextureUsageState::TransferSrc),
            CommandOp::HostReadBuffer{buffer:readback,offset:0,size:byte_count}]);
        let result=gal.submit(SubmissionBatch{label:"leaf-diagnostic.submit".into(),
            command_lists:vec![CommandList::from(CommandListDesc{label:"leaf-diagnostic.commands".into(),operations})]}).unwrap();
        for resource in resources {resource.destroy(gal);}
        for handle in diagnostic_depth_resources {gal.destroy(handle).unwrap();}
        targets.clear(gal);
        for handle in alpha_resources.into_iter().rev() {gal.destroy(handle).unwrap();}
        for handle in [sampler,view,texture,upload,readback] {gal.destroy(handle).unwrap();}
        gal.retire_through_for_test(result.submission).unwrap();
        gal.completed_host_reads().into_iter().find(|read|read.buffer==readback).unwrap().bytes
    }

    fn leaf_diagnostic_alpha_texture(gal:&mut VulkanicGal,png_bytes:&[u8],size:u32,
        operations:&mut Vec<CommandOp>,owned:&mut Vec<Handle>)->Handle {
        let mut decoder=png::Decoder::new(png_bytes);
        decoder.set_transformations(png::Transformations::EXPAND|png::Transformations::STRIP_16);
        let mut reader=decoder.read_info().unwrap();
        let mut bytes=vec![0;reader.output_buffer_size()];
        let info=reader.next_frame(&mut bytes).unwrap();
        assert_eq!((info.width,info.height,info.color_type),(size,size,png::ColorType::Rgba));
        bytes.truncate(info.buffer_size());
        assert!(bytes.chunks_exact(4).any(|p|p[3]==0));
        assert!(bytes.chunks_exact(4).any(|p|p[3]==255));
        for pixel in bytes.chunks_exact_mut(4){pixel[..3].fill(255);}
        leaf_diagnostic_rgba_texture(gal,bytes,size,operations,owned)
    }

    fn leaf_diagnostic_rgba_texture(gal:&mut VulkanicGal,bytes:Vec<u8>,size:u32,
        operations:&mut Vec<CommandOp>,owned:&mut Vec<Handle>)->Handle {
        use super::super::commands::{BufferImageCopyRegion,TextureOrigin3d};
        let extent=Extent3d{width:size,height:size,depth:1};
        let texture=gal.create_texture(TextureDesc{label:"leaf-diagnostic.alpha".into(),dimension:TextureDimension::D2,
            format:TextureFormat::Rgba8Unorm,extent,mip_levels:1,array_layers:1,
            usages:vec![TextureUsage::Sampled,TextureUsage::TransferDst]}).unwrap();
        let view=gal.create_texture_view(TextureViewDesc{label:"leaf-diagnostic.alpha-view".into(),texture,
            format:TextureFormat::Rgba8Unorm,base_mip:0,mip_count:1,base_layer:0,layer_count:1}).unwrap();
        let upload=gal.create_buffer(BufferDesc{label:"leaf-diagnostic.alpha-upload".into(),size:bytes.len() as u64,
            memory:MemoryDomain::Upload,usages:vec![BufferUsage::HostWrite,BufferUsage::TransferSrc]}).unwrap();
        owned.extend([upload,texture,view]);
        let barrier=|resource,before,after|CommandOp::Barrier(ResourceBarrier{resource,subresources:None,before,after,
            src_queue:QueueClass::Graphics,dst_queue:QueueClass::Graphics});
        operations.extend([CommandOp::HostWriteBuffer{buffer:upload,offset:0,data:bytes},
            barrier(upload,TextureUsageState::TransferDst,TextureUsageState::TransferSrc),
            barrier(texture,TextureUsageState::Undefined,TextureUsageState::TransferDst),
            CommandOp::CopyBufferToTexture(BufferImageCopyRegion{buffer:upload,buffer_offset:0,bytes_per_row:size*4,
                rows_per_image:size,texture,texture_mip:0,texture_layer:0,texture_origin:TextureOrigin3d{x:0,y:0,z:0},extent}),
            barrier(texture,TextureUsageState::TransferDst,TextureUsageState::ShaderRead)]);
        view
    }

    fn leaf_diagnostic_program(gal:&mut VulkanicGal,draw:&GuiMeshPreparedDraw,
        depth_format:TextureFormat,stencil:Option<super::super::resources::StencilFaceState>,depth_override:Option<CompareOp>)->GuiMeshSharedProgram {
        let mut program=GuiMeshSharedProgram::create(gal,"leaf-diagnostic.program",TextureFormat::Rgba8Unorm,
            Some(depth_format),draw.material_mode,draw.front_face).unwrap();
        if stencil.is_some() || depth_override.is_some() {
            let (cull_mode,blend,depth_compare,depth_write)=gui_mesh_raster_state(draw.material_mode);
            let pipeline=gal.create_graphics_pipeline(GraphicsPipelineDesc{
                label:"leaf-diagnostic.stencil-pipeline".into(),layout:program.pipeline_layout,
                vertex_shader:program.vertex_shader,fragment_shader:program.fragment_shader,
                topology:PrimitiveTopology::Triangles,cull_mode,front_face:draw.front_face,
                provoking_vertex: crate::render::vulkanic::resources::ProvokingVertex::Last,
                raster_y_direction: crate::render::vulkanic::resources::RasterYDirection::Up,
                blend,depth_compare:depth_override.or(depth_compare),depth_write,depth_bias:None,color_formats:vec![TextureFormat::Rgba8Unorm],
                depth_format:Some(depth_format),stencil:stencil.map(|face|super::super::resources::StencilState{front:face,back:face}),
            }).unwrap();
            gal.destroy(program.pipeline).unwrap();
            program.pipeline=pipeline;
        }
        program
    }

    #[test]
    fn native_block_layout_executes_through_explicit_owned_pass_and_composite() {
        let mut request=batch();
        request.block_item_raster=Some(GuiBlockItemRaster {
            model_min:[-0.5;3],model_max:[0.5;3],gui_scale:2,oversized_gui:false });
        request.render_extent=[0,0];request.guard_pixels=0;
        request.lighting_mode=GuiMeshLightingMode::InventoryBlock;
        request.model_transform[12]=-0.5;request.model_transform[13]=-0.5;
        request.resolve_item_lighting(Some(flat_lightmap())).unwrap();
        exercise_owned_mesh_pass(request);
    }

    #[test]
    fn explicit_oversized_item_derives_composite_bounds_and_owned_target_from_semantics() {
        let mut request=batch();
        request.block_item_raster=Some(GuiBlockItemRaster {
            model_min:[-0.8,-0.4,-0.5],model_max:[1.1,0.8,0.5],gui_scale:2,oversized_gui:true });
        request.bounds=[10,20,26,36];
        request.render_extent=[0,0];request.guard_pixels=0;
        request.lighting_mode=GuiMeshLightingMode::InventoryBlock;
        request.resolve_item_lighting(Some(flat_lightmap())).unwrap();
        let draw=prepare_draw(&request).unwrap();
        assert_eq!(draw.bounds,[5,16,36,36]);
        assert_eq!(draw.render_extent,[64,42]);
        assert_eq!(draw.vertices[0].position,[27.,6.,0.]);
        let mut bad=request.clone();bad.bounds[2]=30;
        assert!(prepare_draw(&bad).is_err());
        exercise_owned_mesh_pass(request);
    }

    fn exercise_owned_mesh_pass(request: GuiMeshBatchRequest) {
        let draw=prepare_draws(&[request]).unwrap().pop().unwrap();
        let mut gal = VulkanicGal::new_with_backend(
            Box::new(MockBackend::with_capabilities(vulkan_capabilities())),
            false,
        );
        let texture = gal
            .create_texture(TextureDesc {
                label: "gui-mesh-test-image".to_string(),
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba8Unorm,
                extent: Extent3d {
                    width: 1,
                    height: 1,
                    depth: 1,
                },
                mip_levels: 1,
                array_layers: 1,
                usages: vec![TextureUsage::Sampled],
            })
            .expect("create Rust-owned test image");
        let view = gal
            .create_texture_view(TextureViewDesc {
                label: "gui-mesh-test-image-view".to_string(),
                texture,
                format: TextureFormat::Rgba8Unorm,
                base_mip: 0,
                mip_count: 1,
                base_layer: 0,
                layer_count: 1,
            })
            .expect("create test image view");
        let sampler = gal
            .create_sampler(super::super::resources::SamplerDesc {
                label: "gui-mesh-test-sampler".to_string(),
                min_filter: super::super::resources::SamplerFilter::Nearest,
                mag_filter: super::super::resources::SamplerFilter::Nearest,
                mip_filter: super::super::resources::SamplerFilter::Nearest,
                address_u: super::super::resources::SamplerAddressMode::ClampToEdge,
                address_v: super::super::resources::SamplerAddressMode::ClampToEdge,
                address_w: super::super::resources::SamplerAddressMode::ClampToEdge,
                comparison: None,
            })
            .expect("create test sampler");
        let mut targets = GuiMeshOffscreenTargetCache::default();
        let target = targets
            .stage(
                &mut gal,
                1,
                Extent3d {
                    width: draw.render_extent[0],
                    height: draw.render_extent[1],
                    depth: 1,
                },
            )
            .expect("create mesh target");
        let destination = targets
            .stage(
                &mut gal,
                1,
                Extent3d {
                    width: 320,
                    height: 180,
                    depth: 1,
                },
            )
            .expect("create final GUI target");
        let destination_pass = gal
            .create_render_pass(RenderPassDesc {
                label: "gui-mesh-test-final-pass".to_string(),
                target: destination.target,
                color_formats: vec![TextureFormat::Rgba8Unorm],
                depth_format: Some(TextureFormat::Depth32Float),
            })
            .expect("create final GUI pass");
        let resources = GuiMeshPassResources::create(
            &mut gal,
            "gui-mesh-test",
            TextureFormat::Rgba8Unorm,
            view,
            sampler,
            GuiMeshMaterialMode::Cutout,
            draw.front_face,
        )
        .expect("create owned mesh pass resources");
        let composite = GuiMeshCompositeResources::create(
            &mut gal,
            "gui-mesh-test-composite",
            TextureFormat::Rgba8Unorm,
            Some(TextureFormat::Depth32Float),
            target.color_view,
        )
        .expect("create owned GUI mesh compositor");
        let mut operations = Vec::new();
        resources
            .append_draw(
                target,
                &draw,
                GuiMeshStreamRange::default(),
                true,
                &mut operations,
            )
            .expect("append one owned mesh draw");
        assert!(operations.iter().any(|operation| matches!(operation,
            CommandOp::Barrier(barrier) if barrier.resource == target.depth
                && barrier.before == TextureUsageState::Undefined
                && barrier.after == TextureUsageState::DepthStencilAttachment)));
        let mut reused = target;
        reused.initialized = true;
        let mut reused_ops = Vec::new();
        resources.append_draw(reused, &draw, GuiMeshStreamRange::default(), true, &mut reused_ops).unwrap();
        assert!(reused_ops.iter().any(|operation| matches!(operation,
            CommandOp::Barrier(barrier) if barrier.resource == target.depth
                && barrier.before == TextureUsageState::DepthStencilAttachment
                && barrier.after == TextureUsageState::DepthStencilAttachment)));
        composite
            .append_composite(
                target,
                TextureUsageState::ColorAttachment,
                destination_pass,
                destination.target,
                destination.color_view,
                Some(destination.depth_view),
                &draw,
                0,
                &mut operations,
            )
            .expect("append one owned GUI mesh composite");
        assert!(operations.iter().any(|operation| matches!(
            operation,
            CommandOp::BeginPass { target: actual, .. } if *actual == target.target
        )));
        assert!(operations.iter().any(|operation| matches!(
            operation,
            CommandOp::DrawIndexed {
                indices: 3,
                instances: 1
            }
        )));
        assert!(operations.iter().any(|operation| matches!(
            operation,
            CommandOp::BeginPass { target: actual, .. } if *actual == destination.target
        )));
        assert!(operations.iter().any(|operation| matches!(
            operation,
            CommandOp::Draw {
                vertices: 6,
                instances: 1
            }
        )));
        let accepted_raster_submission = gal.next_submission_id();
        assert_eq!(gal.render_pass_last_submission(target.pass).unwrap(), None);
        gal.submit(SubmissionBatch {
            label: "gui-mesh-test-submit".to_string(),
            command_lists: vec![CommandList::from(CommandListDesc {
                label: "gui-mesh-test-commands".to_string(),
                operations,
            })],
        })
        .expect("GAL validates one complete owned GUI mesh draw");
        assert_eq!(gal.render_pass_last_submission(target.pass).unwrap(), Some(accepted_raster_submission));
        composite.destroy(&mut gal);
        resources.destroy(&mut gal);
        let _ = gal.destroy(destination_pass);
        targets.clear(&mut gal);
        let _ = gal.destroy(sampler);
        let _ = gal.destroy(view);
        let _ = gal.destroy(texture);
    }

    #[test]
    fn vulkan_gui_mesh_fragment_samples_the_owned_image() {
        let source = std::str::from_utf8(GUI_MESH_FRAGMENT_SHADER_VULKAN)
            .expect("GUI mesh Vulkan fragment source is UTF-8");
        assert!(
            source.contains("texture(sampler2D(GuiMeshTexture, GuiMeshSampler), v_uv) * v_color"),
            "the Vulkan GUI mesh fragment must sample the Rust-owned image rather than a diagnostic constant"
        );
        assert!(!source.contains("vec4(1.0, 0.0, 1.0, 1.0)"));
    }

    #[test]
    fn panorama_uses_an_unlit_rust_owned_material_program() {
        for api in [BackendApi::OpenGl, BackendApi::Vulkan] {
            let (_, panorama) = gui_mesh_shader_sources(api, GuiMeshMaterialMode::Panorama);
            let source = std::str::from_utf8(panorama).expect("panorama shader source is UTF-8");
            assert!(
                source.contains("texture("),
                "the semantic panorama must sample its Rust-owned copied cube-face image"
            );
            assert!(
                !source.contains("discard") && !source.contains("normalize(v_normal)"),
                "the panorama must not inherit 3D item cutout or directional-lighting behavior"
            );

            let (_, item) = gui_mesh_shader_sources(api, GuiMeshMaterialMode::Opaque);
            assert_ne!(
                panorama, item,
                "Panorama must select a distinct material program rather than the generic item shader"
            );
        }
    }

    #[test]
    fn flat_mesh_native_raster_matches_frozen_cell_pose_and_fullbright_lightmap() {
        let mut request = flat_mesh();
        assert!(prepare_draws(&[request.clone()]).is_err());
        assert!(request.resolve_item_lighting(None).is_err());
        request.resolve_item_lighting(Some(flat_lightmap())).unwrap();
        for scale in [1, 2, 3, 4] {
            request.item_raster_scale = scale;
            let draw = prepare_draws(&[request.clone()]).unwrap().remove(0);
            let side = (16 * scale) as f32;
            assert_eq!(draw.render_extent, [16 * scale; 2]);
            assert_eq!(draw.guard_pixels, 0);
            assert_eq!(draw.vertices[0].position, [0.0, side, -side * 0.5]);
            assert_eq!(draw.vertices[1].position, [side, side, -side * 0.5]);
            for vertex in draw.vertices {
                assert_eq!(vertex.color, [252.0 / 255.0, 252.0 / 255.0, 252.0 / 255.0, 1.0]);
            }
        }
    }

    #[test]
    fn decoded_flat_item_counts_use_scheduler_groups_and_actual_transforms() {
        let mut first = flat_mesh();
        first.sequence = 1;
        let mut second = first.clone();
        second.layer_index = 1;
        second.model_transform[12] = 2.0;
        let mut third = second.clone();
        third.sequence = 2;
        third.layer_index = 0;
        let mut pip = first.clone();
        pip.item_raster_scale = 0;
        assert_eq!(flat_item_mesh_decode_counts(&[first.clone(),second.clone(),third,pip]), (2,3,2,2));
        assert_eq!(flat_item_mesh_decode_counts(&[first,second]), (1,2,1,2));
        assert_eq!(flat_item_mesh_decode_counts(&[]), (0,0,0,0));
    }

    #[test]
    fn inventory_block_mesh_requires_and_refreshes_explicit_lightmap() {
        let mut request = batch();
        request.lighting_mode = GuiMeshLightingMode::InventoryBlock;
        assert!(prepare_draws(&[request.clone()]).is_err());
        assert!(request.resolve_item_lighting(None).is_err());
        request.vertices[0].color_argb = 0x8040ff20;
        let mut frame = flat_lightmap();
        request.resolve_item_lighting(Some(frame)).unwrap();
        let original = prepare_draws(&[request.clone()]).unwrap().remove(0);
        assert_eq!(original.vertices[0].color,
            [(64.0/255.0)*(252.0/255.0),252.0/255.0,(32.0/255.0)*(252.0/255.0),128.0/255.0]);
        frame.generation += 1;
        frame.inputs.darkness_scale = 4.0;
        request.resolve_item_lighting(Some(frame)).unwrap();
        let changed = prepare_draws(&[request]).unwrap().remove(0);
        assert_ne!(geometry_fingerprint(&original), geometry_fingerprint(&changed));
        assert_eq!(original.vertices[0].normal, changed.vertices[0].normal);
    }

    #[test]
    fn flat_mesh_frame_lighting_refreshes_cached_geometry_and_preserves_tint_alpha() {
        let mut request = flat_mesh();
        request.vertices[0].color_argb = 0x8040ff20;
        let mut frame = flat_lightmap();
        request.resolve_item_lighting(Some(frame)).unwrap();
        let original = prepare_draws(&[request.clone()]).unwrap().remove(0);
        assert_eq!(original.vertices[0].color[3], 128.0 / 255.0);
        assert_eq!(original.vertices[0].color[0], (64.0 / 255.0) * (252.0 / 255.0));
        frame.generation += 1;
        frame.inputs.darkness_scale = 4.0;
        request.resolve_item_lighting(Some(frame)).unwrap();
        let changed = prepare_draws(&[request.clone()]).unwrap().remove(0);
        assert_ne!(geometry_fingerprint(&original), geometry_fingerprint(&changed));
        assert_eq!(original.vertices[0].color[3], changed.vertices[0].color[3]);
        assert_eq!(request.item_lighting.unwrap().lightmap_generation, frame.generation);
    }

    #[test]
    fn flat_mesh_native_foil_uses_the_same_depth_without_lightmap_modulation() {
        let mut base = flat_mesh();
        base.resolve_item_lighting(Some(flat_lightmap())).unwrap();
        let mut foil = base.clone();
        foil.layer_index = 1;
        foil.material_mode = GuiMeshMaterialMode::Glint;
        foil.alpha_cutoff = 0.1;
        foil.item_lighting = None;
        foil.item_foil = Some(GuiItemFoil { kind: super::super::item_foil::StandardFoilKind::Item, clock_millis: 0, speed: 0.0, strength: 0.5 });
        let draws = prepare_draws(&[base, foil]).unwrap();
        for (base, foil) in draws[0].vertices.iter().zip(&draws[1].vertices) {
            assert_eq!(base.position, foil.position);
            assert_eq!(foil.color, [0.5, 0.5, 0.5, 1.0]);
        }
        assert_eq!(gui_mesh_raster_state(GuiMeshMaterialMode::Glint).2, Some(CompareOp::Equal));
    }

    #[test]
    fn native_model_entity_foil_keeps_depth_and_uses_original_atlas_coordinates() {
        let mut base = flat_mesh();
        base.resolve_item_lighting(Some(flat_lightmap())).unwrap();
        let mut foil = base.clone();
        foil.layer_index = 1;
        foil.material_mode = GuiMeshMaterialMode::Glint;
        foil.item_lighting = None;
        let semantics = GuiItemFoil { kind: super::super::item_foil::StandardFoilKind::Entity,
            clock_millis: 12345, speed: 0.5, strength: 0.375 };
        foil.item_foil = Some(semantics);
        let draws = prepare_draws(&[base, foil.clone()]).unwrap();
        for ((base, output), input) in draws[0].vertices.iter().zip(&draws[1].vertices).zip(&foil.vertices) {
            assert_eq!(base.position, output.position);
            assert_eq!(output.color, [0.375, 0.375, 0.375, 1.0]);
            assert_eq!(output.local_uv, semantics.texture_uv(input.atlas_uv).unwrap());
        }
        let mut legacy = foil.clone();
        legacy.item_raster_scale = 0;
        legacy.render_extent = [48,48];
        assert!(validate_batch(&legacy).is_err());
        foil.decal_foil = GuiDecalFoilProjection::decode(2,[0.;16],[0.;9]).unwrap();
        assert!(validate_batch(&foil).is_err(), "entity foil is not projected item foil");
    }

    #[test]
    fn native_item_mesh_rejects_conflicting_layout_unbounded_scale_and_invalid_geometry() {
        let mut source = flat_mesh();
        source.resolve_item_lighting(Some(flat_lightmap())).unwrap();
        let mut invalid = source.clone(); invalid.render_extent = [32, 32];
        assert!(prepare_draws(&[invalid]).is_err());
        let mut invalid = source.clone(); invalid.guard_pixels = 1;
        assert!(prepare_draws(&[invalid]).is_err());
        let mut invalid = source.clone(); invalid.item_raster_scale = u32::MAX;
        assert!(prepare_draws(&[invalid]).is_err());
        for matrix in [
            { let mut m=source.model_transform; m[3]=0.25; m },
            { let mut m=source.model_transform; m[0]=0.; m },
            { let mut m=source.model_transform; m[12]=17.; m },
        ] {
            let mut invalid = source.clone(); invalid.model_transform = matrix;
            assert!(prepare_draws(&[invalid]).is_err());
        }
        let mut invalid = source.clone(); invalid.vertices[0].normal_packed = 0x00007f00;
        assert!(prepare_draws(&[invalid]).is_err());
        let mut invalid = source; invalid.lighting_mode = GuiMeshLightingMode::Block;
        assert!(prepare_draws(&[invalid]).is_err());
    }

    #[test]
    fn native_front_lit_model_keeps_three_dimensional_pose_and_out_of_cell_vertices() {
        let mut request = flat_mesh();
        request.resolve_item_lighting(Some(flat_lightmap())).unwrap();
        // A real Y-axis rotation, retaining authored geometry across the cell
        // boundary. CPU lowering must not flatten Z, fit or clamp this model.
        request.model_transform = [0.8,0.,-0.6,0., 0.,1.,0.,0., 0.6,0.,0.8,0., 0.25,-0.5,0.,1.];
        let draw = prepare_draws(&[request.clone()]).unwrap().remove(0);
        assert_eq!(draw.render_extent, [32,32]);
        assert!(draw.vertices.iter().any(|v|v.position[0] > 32.));
        assert_ne!(draw.vertices[0].position[2], draw.vertices[1].position[2]);
        for (source, output) in request.vertices.iter().zip(&draw.vertices) {
            let transformed = transform_point(request.model_transform, source.position).unwrap();
            assert_eq!(output.position, [16.+32.*transformed[0],16.-32.*transformed[1],32.*transformed[2]]);
        }
        assert_eq!(draw.indices.len(), request.indices.len());
        assert_eq!(draw.vertices.len(), request.vertices.len());
    }

    fn flat_mesh() -> GuiMeshBatchRequest {
        let mut request = batch();
        request.item_raster_scale = 2;
        request.render_extent = [0, 0];
        request.guard_pixels = 0;
        request.lighting_mode = GuiMeshLightingMode::Flat;
        request.model_transform = super::super::gui_item_raster::GuiItemModelTransform::default().0;
        request
    }

    #[test]
    fn flat_item_normals_use_inverse_transpose_not_position_or_direction_transform() {
        let matrix = [
            0.0, -2.0, 0.0, 0.0,
            -4.0, 0.0, 0.0, 0.0,
            0.0, 0.0, 8.0, 0.0,
            16.0, 16.0, 0.0, 1.0,
        ];
        assert_eq!(transform_model_item_normal(matrix, [1.0,0.0,0.0]).unwrap(), [0.0,-1.0,0.0]);
        assert_eq!(transform_model_item_normal(matrix, [0.0,1.0,0.0]).unwrap(), [-1.0,0.0,0.0]);
        assert_eq!(transform_model_item_normal(matrix, [0.0,0.0,-1.0]).unwrap(), [0.0,0.0,-1.0]);
        let actual = transform_model_item_normal(matrix, [1.0,1.0,0.0]).unwrap();
        let expected = normalize([-0.25,-0.5,0.0]).unwrap();
        for axis in 0..3 { assert!((actual[axis]-expected[axis]).abs()<0.000001); }
        let mut singular = matrix; singular[10] = 0.0;
        assert!(transform_model_item_normal(singular, [0.0,0.0,1.0]).is_err());
    }

    #[test]
    fn flat_mesh_accepts_copied_back_and_edge_faces_without_discarding_geometry() {
        let faces = [
            ([[0.0,1.0,0.45],[1.0,1.0,0.45],[1.0,0.0,0.45],[0.0,0.0,0.45]], 0x00810000),
            ([[1.0,1.0,0.55],[1.0,0.0,0.55],[1.0,0.0,0.45],[1.0,1.0,0.45]], 0x0000007f),
            ([[0.0,1.0,0.45],[0.0,0.0,0.45],[0.0,0.0,0.55],[0.0,1.0,0.55]], 0x00000081),
            ([[0.0,1.0,0.45],[0.0,1.0,0.55],[1.0,1.0,0.55],[1.0,1.0,0.45]], 0x00007f00),
            ([[0.0,0.0,0.55],[0.0,0.0,0.45],[1.0,0.0,0.45],[1.0,0.0,0.55]], 0x00008100),
        ];
        let mut combined = flat_mesh();
        combined.resolve_item_lighting(Some(flat_lightmap())).unwrap();
        combined.vertices.clear();
        combined.indices.clear();
        for (positions, normal_packed) in faces {
            let mut request = flat_mesh();
            request.resolve_item_lighting(Some(flat_lightmap())).unwrap();
            let template = request.vertices[0];
            request.vertices = positions.into_iter().map(|position| GuiMeshVertex {
                position, normal_packed, ..template
            }).collect();
            request.indices = vec![0,1,2,2,3,0];
            let output = prepare_draws(&[request.clone()]).unwrap();
            assert_eq!(output.len(), 1);
            assert_eq!(output[0].vertices.len(), 4);
            assert_eq!(output[0].indices, request.indices);
            assert_eq!(output[0].front_face, super::super::resources::FrontFace::CounterClockwise);
            let base = combined.vertices.len() as u32;
            combined.vertices.extend(request.vertices);
            combined.indices.extend([base,base+1,base+2,base+2,base+3,base]);
        }
        let output = prepare_draws(&[combined.clone()]).unwrap();
        assert_eq!(output.len(),1);
        assert_eq!(output[0].vertices.len(),20);
        assert_eq!(output[0].indices,combined.indices);
        // Independent reversed source winding is reconciled in Rust, while
        // the copied resource/vertex order remains unchanged.
        combined.indices.swap(7,8);
        let reversed = prepare_draws(&[combined.clone()]).unwrap();
        assert_eq!(reversed[0].indices,output[0].indices);
        assert_eq!(reversed[0].vertices.len(),20);
        // Different faces may differ; different normals within one face may not.
        combined.vertices[1].normal_packed = 0x0000007f;
        assert!(prepare_draws(&[combined]).is_err());
    }

    fn flat_lightmap() -> super::super::shader_pack::lightmap::VanillaLightmapFrame {
        use super::super::shader_pack::lightmap::{VanillaLightmapFrame, VanillaLightmapInputs};
        VanillaLightmapFrame { generation: 7, inputs: VanillaLightmapInputs {
            ambient_light_factor: 0.0, sky_factor: 1.0, block_factor: 1.5,
            night_vision_factor: 0.0, darkness_scale: 0.0, darken_world_factor: 0.0,
            brightness_factor: 0.0, sky_light_color: [1.0; 3], ambient_color: [1.0; 3],
        } }
    }

    #[test]
    fn native_flat_decal_projection_is_scale_and_layout_independent() {
        let foil = GuiItemFoil { kind: super::super::item_foil::StandardFoilKind::Item,
            clock_millis: 12345, speed: 0.5, strength: 0.375 };
        for scale in [1,2,3,4] {
            for rotated in [false,true] {
                let mut request = flat_mesh();
                request.item_raster_scale = scale;
                request.material_mode = GuiMeshMaterialMode::Glint;
                request.item_foil = Some(foil);
                request.decal_foil = GuiDecalFoilProjection::decode(2,[0.;16],[0.;9]).unwrap();
                if rotated {
                    request.model_transform = [0.,1.,0.,0.,-1.,0.,0.,0.,0.,0.,1.,0.,0.5,-0.5,-0.5,1.];
                }
                let draw=prepare_draws(&[request.clone()]).unwrap().remove(0);
                for (source,output) in request.vertices.iter().zip(&draw.vertices) {
                    let expected=foil.texture_uv([source.position[0]/64.,-source.position[1]/64.]).unwrap();
                    for axis in 0..2 { assert!((output.local_uv[axis]-expected[axis]).abs()<1e-6); }
                }
                request.decal_foil = GuiDecalFoilProjection::decode(1,
                    batch().model_transform,[1.,0.,0.,0.,1.,0.,0.,0.,1.]).unwrap();
                assert!(prepare_draws(&[request]).is_err(),"caller raster poses must not masquerade as native item layout");
            }
        }
        let mut explicit=batch();
        explicit.material_mode=GuiMeshMaterialMode::Glint;
        explicit.item_foil=Some(foil);
        explicit.decal_foil=GuiDecalFoilProjection::decode(2,[0.;16],[0.;9]).unwrap();
        assert!(prepare_draws(&[explicit]).is_err());
        let mut invalid=[0.;16];invalid[0]=1.;
        assert!(GuiDecalFoilProjection::decode(2,invalid,[0.;9]).is_err());
    }

    #[test]
    fn decal_foil_lowering_uses_semantic_poses_not_caller_uvs_or_colors() {
        let mut request = batch();
        request.material_mode = GuiMeshMaterialMode::Glint;
        request.lighting_mode = GuiMeshLightingMode::Flat;
        let foil = GuiItemFoil { kind: super::super::item_foil::StandardFoilKind::Item,
            clock_millis: 12_345, speed: 0.5, strength: 0.37 };
        request.item_foil = Some(foil);
        request.decal_foil = Some(GuiDecalFoilProjection {
            native_item_layout: false,
            model_pose: request.model_transform,
            normal_pose: [1.,0.,0.,0.,1.,0.,0.,0.,1.],
        });
        request.vertices[0].position = [2.,3.,5.];
        request.vertices[0].normal_packed = 0x0000_7f00;
        request.vertices[0].atlas_uv = [97.,89.];
        request.vertices[0].local_uv = [-53.,-71.];
        request.vertices[0].color_argb = 0;
        let draw = prepare_draws(&[request.clone()]).unwrap().remove(0);
        assert_eq!(draw.vertices[0].position, [2.,3.,5.]);
        assert_eq!(draw.vertices[0].local_uv, foil.texture_uv([4./128.,10./128.]).unwrap());
        assert_eq!(draw.vertices[0].color, foil.color().unwrap());
        request.vertices[0].atlas_uv = [-9.,11.];
        request.vertices[0].local_uv = [0.25,0.75];
        request.vertices[0].color_argb = 0xffff_ffff;
        let changed_irrelevant = prepare_draws(&[request.clone()]).unwrap().remove(0);
        assert_eq!(geometry_fingerprint(&draw), geometry_fingerprint(&changed_irrelevant));
        request.decal_foil.as_mut().unwrap().normal_pose[4] = -1.;
        let changed_normal_pose = prepare_draws(&[request.clone()]).unwrap().remove(0);
        assert_ne!(geometry_fingerprint(&draw), geometry_fingerprint(&changed_normal_pose));
        request.decal_foil.as_mut().unwrap().model_pose[12] = 1.;
        let changed_model_pose = prepare_draws(&[request.clone()]).unwrap().remove(0);
        assert_ne!(geometry_fingerprint(&changed_normal_pose), geometry_fingerprint(&changed_model_pose));
        request.item_foil.as_mut().unwrap().clock_millis += 1000;
        let changed_clock = prepare_draws(&[request.clone()]).unwrap().remove(0);
        assert_ne!(geometry_fingerprint(&changed_model_pose), geometry_fingerprint(&changed_clock));
        request.item_foil.as_mut().unwrap().strength = 0.75;
        let changed_strength = prepare_draws(&[request]).unwrap().remove(0);
        assert_ne!(geometry_fingerprint(&changed_clock), geometry_fingerprint(&changed_strength));
    }

    #[test]
    fn decal_foil_rejects_missing_material_semantics_and_invalid_poses() {
        let mut request = batch();
        request.decal_foil = Some(GuiDecalFoilProjection {
            native_item_layout: false,
            model_pose: request.model_transform,
            normal_pose: [1.,0.,0.,0.,1.,0.,0.,0.,1.],
        });
        assert!(prepare_draws(&[request.clone()]).is_err());
        request.item_foil = Some(GuiItemFoil { kind: super::super::item_foil::StandardFoilKind::Item,
            clock_millis: 0, speed: 0., strength: 0.5 });
        assert!(prepare_draws(&[request.clone()]).is_err());
        request.material_mode = GuiMeshMaterialMode::Glint;
        request.lighting_mode = GuiMeshLightingMode::Flat;
        assert!(prepare_draws(&[request.clone()]).is_ok());
        request.decal_foil.as_mut().unwrap().normal_pose = [0.;9];
        assert!(prepare_draws(&[request.clone()]).is_err());
        request.decal_foil.as_mut().unwrap().normal_pose = [1.,0.,0.,0.,1.,0.,0.,0.,1.];
        request.decal_foil.as_mut().unwrap().model_pose[3] = 1.;
        assert!(prepare_draws(&[request]).is_err());
    }

    #[test]
    fn semantic_item_foil_animation_and_strength_invalidate_prepared_geometry() {
        let mut request = batch();
        request.material_mode = GuiMeshMaterialMode::Glint;
        request.lighting_mode = GuiMeshLightingMode::Flat;
        request.alpha_cutoff = 0.1;
        request.item_foil = Some(GuiItemFoil { kind: super::super::item_foil::StandardFoilKind::Item, clock_millis: 0, speed: 0.5, strength: 0.5 });
        let original = prepare_draws(&[request.clone()]).unwrap().remove(0);
        request.item_foil.as_mut().unwrap().clock_millis = 12_345;
        let animated = prepare_draws(&[request.clone()]).unwrap().remove(0);
        assert_ne!(geometry_fingerprint(&original), geometry_fingerprint(&animated));
        request.item_foil.as_mut().unwrap().strength = 0.25;
        let dimmed = prepare_draws(&[request.clone()]).unwrap().remove(0);
        assert_ne!(geometry_fingerprint(&animated), geometry_fingerprint(&dimmed));
        assert_eq!(original.vertices[0].position, dimmed.vertices[0].position);
        request.item_foil.as_mut().unwrap().speed = 0.0;
        let stopped = prepare_draws(&[request.clone()]).unwrap().remove(0);
        request.item_foil.as_mut().unwrap().clock_millis = 555_555;
        let later = prepare_draws(&[request]).unwrap().remove(0);
        assert_eq!(geometry_fingerprint(&stopped), geometry_fingerprint(&later));
    }

    #[test]
    fn semantic_item_foil_prepares_original_uvs_and_unquantized_strength() {
        let mut request = batch();
        request.material_mode = GuiMeshMaterialMode::Glint;
        request.lighting_mode = GuiMeshLightingMode::Flat;
        request.alpha_cutoff = 0.1;
        request.item_foil = Some(GuiItemFoil { kind: super::super::item_foil::StandardFoilKind::Item, clock_millis: 12_345, speed: 0.5, strength: 0.5 });
        for vertex in &mut request.vertices {
            vertex.atlas_uv = [0.25, 0.75];
            // These must not drive the semantic foil's UVs or strength.
            vertex.local_uv = [0.9, 0.1];
            vertex.color_argb = 0x11223344;
        }
        let draw = prepare_draws(&[request.clone()]).unwrap().remove(0);
        for vertex in draw.vertices {
            assert!((vertex.local_uv[0] - 0.478817225).abs() <= 0.000002);
            assert!((vertex.local_uv[1] - 6.902142525).abs() <= 0.000002);
            assert_eq!(vertex.color, [0.5, 0.5, 0.5, 1.0]);
        }
        request.material_mode = GuiMeshMaterialMode::Opaque;
        assert!(prepare_draws(&[request]).is_err());
    }

    #[test]
    fn front_model_lighting_preserves_flat_light_basis_and_quantized_normals() {
        let bytes = frame_uniform_bytes([48.,48.],0.,GuiMeshLightingMode::FrontModel);
        let values: Vec<f32> = bytes.chunks_exact(4).map(|v| f32::from_le_bytes(v.try_into().unwrap())).collect();
        assert_eq!(values[3],2.0); // consume packed normals without renormalization
        // Independent Rodrigues rotation: Rx maps (y,z) to
        // (-(y+z)/sqrt(2), (y-z)/sqrt(2)); Ry follows.
        let c = (std::f64::consts::PI/8.).cos();
        let s = (std::f64::consts::PI/8.).sin();
        for (i,(x,z)) in [(0.2_f64,-0.7_f64),(-0.2,0.7)].into_iter().enumerate() {
            let y1=-(1.+z)/2_f64.sqrt(); let z1=(1.-z)/2_f64.sqrt();
            let expected=[(c*x-s*z1)/1.53_f64.sqrt(),y1/1.53_f64.sqrt(),(s*x+c*z1)/1.53_f64.sqrt()];
            for axis in 0..3 { assert!((values[4+i*4+axis] as f64-expected[axis]).abs()<0.000001); }
        }
        let mut request=batch();
        request.lighting_mode=GuiMeshLightingMode::FrontModel;
        request.item_raster_scale=3; request.render_extent=[0,0]; request.guard_pixels=0;
        request.resolve_item_lighting(Some(flat_lightmap())).unwrap();
        let draw=prepare_draws(&[request.clone()]).unwrap().remove(0);
        for vertex in draw.vertices {
            for n in vertex.normal { assert!((n*127.-(n*127.).round()).abs()<0.00001); }
        }
        request.item_raster_scale=0; request.render_extent=[48,48];
        assert!(validate_batch(&request).is_err());
        request.item_raster_scale=3; request.render_extent=[0,0];
        request.material_mode=GuiMeshMaterialMode::Glint;
        assert!(validate_batch(&request).is_err());
    }

    #[test]
    fn model_overlay_is_two_sided_lit_alpha_without_depth_writes_or_cutout() {
        assert_eq!(gui_mesh_raster_state(GuiMeshMaterialMode::ModelOverlay),
            (CullMode::None,BlendMode::Alpha,Some(CompareOp::LessOrEqual),false));
        let mut request=batch();
        request.material_mode=GuiMeshMaterialMode::ModelOverlay;
        request.lighting_mode=GuiMeshLightingMode::FrontModel;
        request.alpha_cutoff=0.; request.item_raster_scale=3;
        request.render_extent=[0,0];request.guard_pixels=0;
        request.resolve_item_lighting(Some(flat_lightmap())).unwrap();
        let draw=prepare_draw(&request).unwrap();
        let bytes=draw_frame_uniform_bytes(&draw,[48.,48.]);
        assert_eq!(f32::from_le_bytes(bytes[12..16].try_into().unwrap()),3.);
        for api in [BackendApi::OpenGl,BackendApi::Vulkan] {
            let (_,fragment)=gui_mesh_shader_sources(api,GuiMeshMaterialMode::ModelOverlay);
            assert!(std::str::from_utf8(fragment).unwrap().contains("!gl_FrontFacing) normal = -normal"));
        }
        request.alpha_cutoff=0.1;assert!(validate_batch(&request).is_err());
        request.alpha_cutoff=0.;request.lighting_mode=GuiMeshLightingMode::Flat;
        assert!(validate_batch(&request).is_err());
        request.lighting_mode=GuiMeshLightingMode::FrontModel;request.item_raster_scale=0;
        assert!(validate_batch(&request).is_err());
    }

    #[test]
    fn item_glint_strength_modulates_rgb_without_changing_texture_alpha() {
        let mut request=batch();
        request.material_mode=GuiMeshMaterialMode::Glint;
        request.lighting_mode=GuiMeshLightingMode::Flat;
        request.alpha_cutoff=0.1;
        for strength in [0u32,128,255] {
            for vertex in &mut request.vertices {vertex.color_argb=(strength<<24)|0x00ffffff;}
            let draw=prepare_draws(&[request.clone()]).unwrap().remove(0);
            for vertex in draw.vertices {
                assert_eq!([strength as f32/255.0;3],vertex.color[..3]);
                assert_eq!(1.0,vertex.color[3],"GlintAlpha affects RGB, not alpha-test coverage");
            }
        }
    }

    #[test]
    fn standard_3d_item_raster_policy_matches_vanilla_material_modes() {
        for material_mode in [GuiMeshMaterialMode::Opaque, GuiMeshMaterialMode::Cutout] {
            assert_eq!(
                gui_mesh_raster_state(material_mode),
                (
                    CullMode::Back,
                    BlendMode::Disabled,
                    Some(CompareOp::LessOrEqual),
                    true
                ),
            );
        }
        assert_eq!(
            gui_mesh_raster_state(GuiMeshMaterialMode::Translucent),
            (
                CullMode::Back,
                BlendMode::Alpha,
                Some(CompareOp::LessOrEqual),
                true
            ),
        );
        assert_eq!(
            gui_mesh_raster_state(GuiMeshMaterialMode::Glint),
            (
                CullMode::None,
                BlendMode::Glint,
                Some(CompareOp::Equal),
                false
            ),
        );
        assert_eq!(
            gui_mesh_raster_state(GuiMeshMaterialMode::Panorama),
            (CullMode::None, BlendMode::Disabled, None, false),
            "Frozen's panorama pipeline explicitly disables culling and depth testing",
        );
    }

    #[test]
    fn standard_3d_item_composite_preserves_the_pip_target_v_orientation() {
        for source in [
            std::str::from_utf8(GUI_MESH_COMPOSITE_VERTEX_SHADER_OPENGL)
                .expect("OpenGL composite source is UTF-8"),
            std::str::from_utf8(GUI_MESH_COMPOSITE_VERTEX_SHADER_VULKAN)
                .expect("Vulkan composite source is UTF-8"),
        ] {
            assert!(source.contains("v_uv = uv_region.xy + corner["));
            assert!(source.contains("] * uv_region.zw;"));
            assert!(
                !source.contains("1.0 - corner["),
                "standard-3D item composition must not apply the generic PIP V inversion"
            );
        }
    }

    #[test]
    fn mesh_composite_uniform_carries_bounded_clip_rectangle() {
        let mut clipped = batch();
        clipped.clip_mode = 1;
        clipped.clip_left = 4;
        clipped.clip_top = 6;
        clipped.clip_width = 20;
        clipped.clip_height = 24;
        let draw = prepare_draw(&clipped).expect("bounded GUI mesh clip prepares");
        let bytes = composite_uniform_bytes(&draw);
        assert_eq!(GUI_MESH_COMPOSITE_UNIFORM_BYTES, bytes.len());
        let tail = &bytes[64..80];
        let values = (0..4)
            .map(|index| f32::from_le_bytes(tail[index * 4..index * 4 + 4].try_into().unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(vec![4.0, 6.0, 24.0, 30.0], values);
    }

    #[test]
    fn native_oversized_pose_and_clip_only_affect_final_composition() {
        let mut request=batch();
        request.block_item_raster=Some(GuiBlockItemRaster {
            model_min:[-0.8,-0.4,-0.5],model_max:[1.1,0.8,0.5],gui_scale:2,oversized_gui:true });
        request.render_extent=[0,0];request.guard_pixels=0;
        request.lighting_mode=GuiMeshLightingMode::InventoryBlock;
        request.resolve_item_lighting(Some(flat_lightmap())).unwrap();
        let original=prepare_draw(&request).unwrap();
        request.gui_pose=[0.,1.25,-0.75,0.,32.,17.];
        request.clip_mode=1;request.clip_left=12;request.clip_top=18;request.clip_width=14;request.clip_height=17;
        let posed=prepare_draw(&request).unwrap();
        assert_eq!(original.vertices,posed.vertices);
        assert_eq!(original.render_extent,posed.render_extent);
        assert_eq!(original.bounds,posed.bounds);
        let bytes=composite_uniform_bytes(&posed);
        let floats:Vec<_>=bytes.chunks_exact(4).map(|b|f32::from_le_bytes(b.try_into().unwrap())).collect();
        assert_eq!(&floats[..6],&request.gui_pose);
        assert_eq!(&floats[16..20],&[12.,18.,26.,35.]);
        let mut invalid=request.clone();invalid.clip_left=-1;assert!(prepare_draws(&[invalid]).is_err());
        exercise_owned_mesh_pass(request);
    }

    #[test]
    fn standard_3d_item_raster_preserves_vanilla_pip_depth_order_on_each_backend() {
        let opengl = std::str::from_utf8(GUI_MESH_VERTEX_SHADER_OPENGL)
            .expect("OpenGL mesh source is UTF-8");
        assert!(opengl.contains("-position_u.z / 1000.0"));
        assert!(
            !opengl.contains(", top_left_y, position_u.z / 1000.0, 1.0);"),
            "OpenGL PIP depth must retain vanilla's negative orthographic Z scale"
        );

        let vulkan = std::str::from_utf8(GUI_MESH_VERTEX_SHADER_VULKAN)
            .expect("Vulkan mesh source is UTF-8");
        assert!(vulkan.contains("float vanilla_pip_clip_depth = -position_u.z / 1000.0;"));
        assert!(vulkan.contains("vanilla_pip_clip_depth * 0.5 + 0.5"));
    }

    #[test]
    fn reflected_gui_item_pose_accounts_for_the_clip_space_y_reflection() {
        let mut reflected = batch();
        reflected.model_transform[5] = -1.0;
        let draw = prepare_draws(&[reflected])
            .expect("reflected vanilla GUI item transform remains valid")
            .pop()
            .unwrap();
        assert_eq!(
            draw.front_face,
            super::super::resources::FrontFace::CounterClockwise
        );
        assert_eq!(gui_mesh_raster_state(draw.material_mode).0, CullMode::Back);
    }

    #[test]
    fn reflected_gui_item_pose_keeps_posestack_lighting_normals() {
        let mut reflected = batch();
        reflected.model_transform[5] = -1.0;

        let draw = prepare_draws(&[reflected])
            .expect("reflected vanilla GUI item transform remains valid")
            .pop()
            .unwrap();

        assert_eq!(
            draw.front_face,
            super::super::resources::FrontFace::CounterClockwise,
            "culling remains derived before the lighting orientation correction"
        );
        assert_eq!(
            [0.0, 0.0, 1.0],
            draw.vertices[0].normal,
            "the GUI clip-space reflection changes raster winding only; vanilla lighting uses the PoseStack normal matrix"
        );
    }

    #[test]
    fn semantic_item_normals_are_not_transformed_twice() {
        // Java has already applied PoseStack's item normal matrix before the
        // record crosses FFI. The position transform remains necessary for
        // raster placement, but it must not reorient this semantic normal.
        let matrix = [
            1.0, 0.0, 0.0, 0.0, // column 0
            0.0, 0.0, 1.0, 0.0, // column 1
            0.0, -1.0, 0.0, 0.0, // column 2
            0.0, 0.0, 0.0, 1.0, // translation
        ];
        let mut request = batch();
        for vertex in &mut request.vertices {
            // The Java PoseStack normal matrix has already rotated +Z to -Y.
            vertex.normal_packed = 0x0000_8100;
        }
        request.model_transform = matrix;
        assert_eq!(
            [0.0, -1.0, 0.0],
            normalize_semantic_normal([0.0, -1.0, 0.0]).expect("semantic normal")
        );
        let draw = prepare_draws(&[request]).expect("prepare semantic normal");
        assert_eq!([0.0, -1.0, 0.0], draw[0].vertices[0].normal);
    }

    #[test]
    fn non_reflected_gui_item_pose_reverses_the_complete_clip_space_front_face() {
        let draw = prepare_draws(&[batch()])
            .expect("non-reflected GUI item transform remains valid")
            .pop()
            .unwrap();
        assert_eq!(
            draw.front_face,
            super::super::resources::FrontFace::Clockwise
        );
    }

    #[test]
    fn source_quad_winding_selects_the_per_face_front_face_without_disabling_culling() {
        let mut reverse_wound = batch();
        reverse_wound.vertices = vec![
            GuiMeshVertex {
                position: [0.0, 0.0, 0.0],
                atlas_uv: [0.0, 0.0],
                local_uv: [0.0, 0.0],
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
            GuiMeshVertex {
                position: [1.0, 0.0, 0.0],
                atlas_uv: [1.0, 0.0],
                local_uv: [1.0, 0.0],
                color_argb: 0xffff_ffff,
                normal_packed: 0x007f_0000, source_face: 0, source_foil_type: 0,
            },
        ];
        let draw = prepare_draws(&[reverse_wound])
            .expect("opposite baked winding remains an explicit culled draw")
            .pop()
            .unwrap();
        assert_eq!(
            draw.front_face,
            super::super::resources::FrontFace::CounterClockwise
        );
        assert_eq!(gui_mesh_raster_state(draw.material_mode).0, CullMode::Back);
    }
}
