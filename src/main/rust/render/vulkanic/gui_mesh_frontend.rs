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
uniform sampler2D Sampler0;
layout(std140) uniform GuiMeshFrame { vec4 raster_extent; vec4 light0; vec4 light1; };
in vec2 v_uv;
in vec4 v_color;
in vec3 v_normal;
out vec4 out_color;
void main() {
    vec4 color = texture(Sampler0, v_uv) * v_color;
    if (color.a <= raster_extent.z) discard;
    if (raster_extent.w > 0.5) {
        vec2 light = max(vec2(0.0), vec2(dot(light0.xyz, normalize(v_normal)), dot(light1.xyz, normalize(v_normal))));
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
        vec2 light = max(vec2(0.0), vec2(dot(light0.xyz, normalize(v_normal)), dot(light1.xyz, normalize(v_normal))));
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
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum GuiMeshLightingMode {
    Flat,
    Block,
    /// Ordinary inventory models, with normals in the Y-down GUI space.
    /// Distinct from upright picture-in-picture model lighting.
    InventoryBlock,
}

/// One copied model vertex. `normal_packed` is a Java-resolved normal in the
/// item-lighting space, packed with the stable vanilla signed-i8 encoding.
/// Rust owns decoding and drawing, but must not apply a second model transform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuiMeshVertex {
    pub position: [f32; 3],
    pub atlas_uv: [f32; 2],
    pub local_uv: [f32; 2],
    pub color_argb: u32,
    pub normal_packed: u32,
}

/// A material-homogeneous indexed mesh for one GUI item layer. `asset_id`
/// refers to a Rust-owned raw image asset, never a Minecraft atlas object.
#[derive(Clone, Debug, PartialEq)]
pub struct GuiMeshBatchRequest {
    /// Zero for explicit meshes; positive for a native 16-unit flat item cell.
    pub item_raster_scale: u32,
    /// Resolved solely from the Rust-owned frame lightmap, never transported.
    pub item_lighting: Option<GuiFlatItemLighting>,
    /// When present, vertices contain original atlas UVs and Rust prepares
    /// standard item foil. Absent for ordinary or explicitly prepared meshes.
    pub item_foil: Option<GuiItemFoil>,
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
    /// Rust-owned offscreen raster extent, including vanilla's guard pixels.
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
            data: frame_uniform_bytes(draw.projection_extent, draw.alpha_cutoff, draw.lighting_mode),
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
            data: frame_uniform_bytes(draw.render_extent.map(|axis| axis as f32), draw.alpha_cutoff, draw.lighting_mode),
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

/// Flat semantic meshes carry original model-space normals. Resolve the
/// inverse transpose in Rust, including the native GUI Y reflection.
/// Explicit/PIP meshes already carry resolved normals and do not use this.
fn transform_flat_item_normal(matrix: [f32; 16], normal: [f32; 3]) -> GalResult<[f32; 3]> {
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
        self.append_composite_uniforms(source.color, TextureUsageState::ColorAttachment,
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

fn frame_uniform_bytes(
    extent: [f32; 2],
    alpha_cutoff: f32,
    lighting_mode: GuiMeshLightingMode,
) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(GUI_MESH_FRAME_UNIFORM_BYTES);
    let enabled = matches!(lighting_mode, GuiMeshLightingMode::Block | GuiMeshLightingMode::InventoryBlock) as u8 as f32;
    for value in [extent[0] as f32, extent[1] as f32, alpha_cutoff, enabled] {
        bytes.extend_from_slice(&value.to_le_bytes());
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

    pub(crate) fn mark_initialized(&mut self, target: Handle) {
        for cached in self.targets.values_mut() {
            if cached.target == target {
                cached.initialized = true;
                break;
            }
        }
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
fn resolved_item_raster(batch: &GuiMeshBatchRequest) -> GalResult<([u32; 2], [f32; 16])> {
    if batch.item_raster_scale == 0 {
        if batch.item_lighting.is_some() && !batch.requires_item_lightmap() {
            return Err(GalError::invalid_argument("explicit mesh cannot carry flat item lighting"));
        }
        return Ok((batch.render_extent, batch.model_transform));
    }
    if batch.render_extent != [0, 0] || batch.guard_pixels != 0
        || batch.lighting_mode != GuiMeshLightingMode::Flat
        || batch.material_mode == GuiMeshMaterialMode::Panorama
        || (batch.material_mode == GuiMeshMaterialMode::Glint && batch.item_foil.is_none()) {
        return Err(GalError::invalid_argument("flat item mesh has conflicting raster/material semantics"));
    }
    let side = batch.item_raster_scale.checked_mul(16)
        .filter(|side| *side <= GUI_MESH_MAX_OFFSCREEN_AXIS)
        .ok_or_else(|| GalError::unsupported_feature("flat item raster scale exceeds bounded extent"))?;
    super::gui_item_raster::GuiItemModelTransform(batch.model_transform).validate()?;
    let mut matrix = batch.model_transform;
    let k = side as f32;
    for column in 0..4 {
        let index = column * 4;
        matrix[index] = k * batch.model_transform[index] + k * 0.5 * batch.model_transform[index + 3];
        matrix[index + 1] = -k * batch.model_transform[index + 1] + k * 0.5 * batch.model_transform[index + 3];
        matrix[index + 2] = k * batch.model_transform[index + 2];
    }
    Ok(([side, side], matrix))
}

pub fn validate_batch(batch: &GuiMeshBatchRequest) -> GalResult<()> {
    let render_extent = resolved_item_raster(batch)?.0;
    if let Some(foil) = batch.item_foil {
        foil.validate()?;
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
    if batch.guard_pixels.saturating_mul(2) >= render_extent[0]
        || batch.guard_pixels.saturating_mul(2) >= render_extent[1]
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
    let (render_extent, model_transform) = resolved_item_raster(batch)?;
    let vertices = batch
        .vertices
        .iter()
        .map(|vertex| {
            let position = transform_point(model_transform, vertex.position)?;
            let mut normal = normalize_semantic_normal(unpack_normal_i8(vertex.normal_packed))?;
            if batch.item_raster_scale != 0 {
                normal = transform_flat_item_normal(model_transform, normal)?;
                if position[0] < 0.0 || position[1] < 0.0
                    || position[0] > render_extent[0] as f32 || position[1] > render_extent[1] as f32 {
                    return Err(GalError::unsupported_feature("flat item mesh exceeds admitted cell geometry"));
                }
            }
            let mut color=argb_to_rgba(vertex.color_argb);
            let local_uv = if let Some(foil) = batch.item_foil {
                color = foil.color()?;
                foil.texture_uv(vertex.atlas_uv)?
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
    if batch.item_raster_scale != 0 {
        let first = vertices[0].normal;
        if vertices.iter().any(|vertex| vertex.normal.iter().zip(first)
            .any(|(actual, expected)| (actual - expected).abs() > 0.000001)) {
            return Err(GalError::unsupported_feature("flat item face has inconsistent copied normals"));
        }
    }
    let front_face = transformed_front_face(model_transform, &vertices, &batch.indices)?;
    Ok(GuiMeshPreparedDraw {
        stratum: batch.stratum,
        layer_index: batch.layer_index,
        sequence: batch.sequence,
        asset_id: batch.asset_id,
        material_mode: batch.material_mode,
        front_face,
        lighting_mode: batch.lighting_mode,
        alpha_cutoff: batch.alpha_cutoff,
        gui_pose: batch.gui_pose,
        bounds: batch.bounds,
        gui_extent: batch.gui_extent,
        projection_extent: batch.projection_extent,
        render_extent,
        guard_pixels: batch.guard_pixels,
        clip_mode: batch.clip_mode,
        clip_left: batch.clip_left,
        clip_top: batch.clip_top,
        clip_width: batch.clip_width,
        clip_height: batch.clip_height,
        vertices,
        indices: batch.indices.clone(),
    })
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
            item_raster_scale: 0,
            item_lighting: None,
            item_foil: None,
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
                    normal_packed: 0x007f_0000,
                },
                GuiMeshVertex {
                    position: [1.0, 0.0, 0.0],
                    atlas_uv: [1.0, 0.0],
                    local_uv: [1.0, 0.0],
                    color_argb: 0xffff_ffff,
                    normal_packed: 0x007f_0000,
                },
                GuiMeshVertex {
                    position: [0.0, 1.0, 0.0],
                    atlas_uv: [0.0, 1.0],
                    local_uv: [0.0, 1.0],
                    color_argb: 0xffff_ffff,
                    normal_packed: 0x007f_0000,
                },
            ],
            indices: vec![0, 1, 2],
        }
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
    fn inventory_block_lights_match_frozen_opengl_top_face_space() {
        let uniforms = |mode| frame_uniform_bytes([34.0, 34.0], 0.1, mode)
            .chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect::<Vec<_>>();
        let inventory = uniforms(GuiMeshLightingMode::InventoryBlock);
        let upright = uniforms(GuiMeshLightingMode::Block);
        assert_eq!(inventory[3], 1.0);
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
                    width: 34,
                    height: 34,
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
            crate::render::vulkanic::resources::FrontFace::CounterClockwise,
        )
        .expect("create owned mesh pass resources");
        let draw = prepare_draws(&[batch()])
            .expect("prepare test mesh")
            .pop()
            .unwrap();
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
        gal.submit(SubmissionBatch {
            label: "gui-mesh-test-submit".to_string(),
            command_lists: vec![CommandList::from(CommandListDesc {
                label: "gui-mesh-test-commands".to_string(),
                operations,
            })],
        })
        .expect("GAL validates one complete owned GUI mesh draw");
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
        foil.item_foil = Some(GuiItemFoil { clock_millis: 0, speed: 0.0, strength: 0.5 });
        let draws = prepare_draws(&[base, foil]).unwrap();
        for (base, foil) in draws[0].vertices.iter().zip(&draws[1].vertices) {
            assert_eq!(base.position, foil.position);
            assert_eq!(foil.color, [0.5, 0.5, 0.5, 1.0]);
        }
        assert_eq!(gui_mesh_raster_state(GuiMeshMaterialMode::Glint).2, Some(CompareOp::Equal));
    }

    #[test]
    fn flat_mesh_rejects_conflicting_layout_unbounded_scale_and_unadmitted_geometry() {
        let mut source = flat_mesh();
        source.resolve_item_lighting(Some(flat_lightmap())).unwrap();
        let mut invalid = source.clone(); invalid.render_extent = [32, 32];
        assert!(prepare_draws(&[invalid]).is_err());
        let mut invalid = source.clone(); invalid.guard_pixels = 1;
        assert!(prepare_draws(&[invalid]).is_err());
        let mut invalid = source.clone(); invalid.item_raster_scale = u32::MAX;
        assert!(prepare_draws(&[invalid]).is_err());
        let mut invalid = source.clone(); invalid.model_transform[12] += 0.25;
        assert!(prepare_draws(&[invalid]).is_err());
        let mut invalid = source.clone(); invalid.vertices[0].normal_packed = 0x00007f00;
        assert!(prepare_draws(&[invalid]).is_err());
        let mut invalid = source; invalid.lighting_mode = GuiMeshLightingMode::Block;
        assert!(prepare_draws(&[invalid]).is_err());
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
        assert_eq!(transform_flat_item_normal(matrix, [1.0,0.0,0.0]).unwrap(), [0.0,-1.0,0.0]);
        assert_eq!(transform_flat_item_normal(matrix, [0.0,1.0,0.0]).unwrap(), [-1.0,0.0,0.0]);
        assert_eq!(transform_flat_item_normal(matrix, [0.0,0.0,-1.0]).unwrap(), [0.0,0.0,-1.0]);
        let actual = transform_flat_item_normal(matrix, [1.0,1.0,0.0]).unwrap();
        let expected = normalize([-0.25,-0.5,0.0]).unwrap();
        for axis in 0..3 { assert!((actual[axis]-expected[axis]).abs()<0.000001); }
        let mut singular = matrix; singular[10] = 0.0;
        assert!(transform_flat_item_normal(singular, [0.0,0.0,1.0]).is_err());
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
        }
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
    fn semantic_item_foil_animation_and_strength_invalidate_prepared_geometry() {
        let mut request = batch();
        request.material_mode = GuiMeshMaterialMode::Glint;
        request.lighting_mode = GuiMeshLightingMode::Flat;
        request.alpha_cutoff = 0.1;
        request.item_foil = Some(GuiItemFoil { clock_millis: 0, speed: 0.5, strength: 0.5 });
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
        request.item_foil = Some(GuiItemFoil { clock_millis: 12_345, speed: 0.5, strength: 0.5 });
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
                normal_packed: 0x007f_0000,
            },
            GuiMeshVertex {
                position: [0.0, 1.0, 0.0],
                atlas_uv: [0.0, 1.0],
                local_uv: [0.0, 1.0],
                color_argb: 0xffff_ffff,
                normal_packed: 0x007f_0000,
            },
            GuiMeshVertex {
                position: [1.0, 0.0, 0.0],
                atlas_uv: [1.0, 0.0],
                local_uv: [1.0, 0.0],
                color_argb: 0xffff_ffff,
                normal_packed: 0x007f_0000,
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
