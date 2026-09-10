use super::*;

pub const FFI_ABI_V1_VERSION: u32 = 1;
pub const FFI_ABI_V2_VERSION: u32 = 2;
pub const FFI_ABI_V3_VERSION: u32 = 3;
pub const FFI_ABI_V4_VERSION: u32 = 4;
pub const FFI_ABI_V5_VERSION: u32 = 5;
pub const FFI_ABI_V6_VERSION: u32 = 6;
pub const FFI_ABI_V7_VERSION: u32 = 7;
pub const FFI_ABI_V8_VERSION: u32 = 8;
pub const FFI_ABI_V9_VERSION: u32 = 9;
pub const FFI_ABI_V10_VERSION: u32 = 10;
pub const FFI_ABI_V11_VERSION: u32 = 11;
pub const FFI_ABI_V12_VERSION: u32 = 12;
pub const FFI_ABI_V13_VERSION: u32 = 13;
pub const FFI_ABI_V14_VERSION: u32 = 14;
pub const FFI_ABI_V15_VERSION: u32 = 15;
pub const FFI_ABI_V16_VERSION: u32 = 16;
pub const FFI_ABI_V17_VERSION: u32 = 17;
pub const FFI_ABI_V18_VERSION: u32 = 18;
pub const FFI_ABI_V19_VERSION: u32 = 19;
pub const FFI_ABI_V20_VERSION: u32 = 20;
pub const FFI_ABI_V21_VERSION: u32 = 21;
/// v22 appends a dedicated first-person mesh stream to the whole-frame
/// request. It reuses the stable indexed-mesh record layout, but keeps the
/// hand depth/projection domain separate from ordinary world mesh instances.
pub const FFI_ABI_V22_VERSION: u32 = 22;
/// v23 appends the first-person model-view matrix. A hand source requires a
/// distinct transform domain in addition to its projection and cleared depth
/// domain; it must never inherit camera-space world matrices implicitly.
pub const FFI_ABI_V23_VERSION: u32 = 23;
/// v24 appends the semantic entity-outline color to mesh instances. Existing
/// offsets remain stable; zero means no outline request.
pub const FFI_ABI_V24_VERSION: u32 = 24;
/// v25 appends the bounded semantic post-effect identity to whole-frame
/// submission; Rust resolves its copied graph and shader stages.
pub const FFI_ABI_V25_VERSION: u32 = 25;
/// v26 appends explicit world-mesh retirement records to incremental asset
/// updates. Streaming producers must be able to release their Rust-owned
/// mesh resources without smuggling a renderer reset through an empty frame.
pub const FFI_ABI_V26_VERSION: u32 = 26;
/// v27 appends copied per-level atlas PNGs to mesh texture assets. This lets
/// Rust upload Frozen's sprite-isolated mip chain without reading a Java GPU
/// texture or generating whole-atlas mips that bleed across sprite borders.
pub const FFI_ABI_V27_VERSION: u32 = 27;
/// v28 separates fractional GUI projection from rounded layout bounds.
pub const FFI_ABI_V28_VERSION: u32 = 28;
/// v29 appends typed tiled GUI commands, keeping geometry expansion in Rust.
pub const FFI_ABI_V29_VERSION: u32 = 29;
/// v30 adds typed immutable atlas animation declarations (private staging only).
pub const FFI_ABI_V30_VERSION: u32 = 30;
/// v31 appends backend-neutral texture filter and address descriptors.
pub const FFI_ABI_V31_VERSION: u32 = 31;
/// v32 appends copied per-frame engine Globals inputs.
pub const FFI_ABI_V32_VERSION: u32 = 32;
/// v33 appends the explicit resource mip-level count to copied mesh textures.
pub const FFI_ABI_V33_VERSION: u32 = 33;
/// v34 appends an explicit affine GUI material designation.
pub const FFI_ABI_V34_VERSION: u32 = 34;
pub const FFI_ABI_V35_VERSION: u32 = 35;
pub const FFI_ABI_V36_VERSION: u32 = 36;
pub const FFI_ABI_V37_VERSION: u32 = 37;
pub const FFI_ABI_V38_VERSION: u32 = 38;
pub const FFI_ABI_V39_VERSION: u32 = 39;
/// v40 appends standard GUI item foil clock/speed/strength semantics.
pub const FFI_ABI_V40_VERSION: u32 = 40;
/// v41 carries semantic flat-item GUI scale instead of a caller raster extent.
pub const FFI_ABI_V41_VERSION: u32 = 41;
/// v42 appends standard world/held item foil semantics to mesh instances.
pub const FFI_ABI_V42_VERSION: u32 = 42;
/// v53 adds immutable orb appearance assets, lowered to geometry only in Rust.
/// v54 adds semantic orb placement to the ordered entity mesh stream.
pub const FFI_ABI_VERSION: u32 = 54;
pub const FFI_INITIAL_PRESENTATION_SUPPORTED: bool = false;
pub const FFI_ABI_NAME: &str = "MattMC VulkanicGAL Java-Rust batch ABI";
pub const FFI_MAX_LABEL_BYTES: usize = 1024;
pub const FFI_MAX_SHADER_BYTES: usize = 16 * 1024 * 1024;
pub const FFI_MAX_INLINE_BYTES: usize = 64 * 1024 * 1024;
/// Copied GUI images are bounded independently from encoded sprite assets.
/// A 16-million-pixel RGBA semantic image is the largest Java producer may
/// publish, so the ABI must admit its 64 MiB byte representation intact.
pub const FFI_MAX_GUI_ASSET_BYTES: usize = 64 * 1024 * 1024;
/// Hard ceiling for one copied whole-frame semantic request.  This bounds the
/// transient FFI decode arena before any producer-owned vectors are expanded.
pub const FFI_MAX_WHOLE_FRAME_INPUT_BYTES: u64 = 512 * 1024 * 1024;
pub const FFI_MAX_WORLD_BORDER_ASSET_BYTES: usize = 2 * 1024 * 1024;
pub const FFI_MAX_WORLD_CRACK_ASSET_BYTES: usize = 4 * 1024 * 1024;
pub const FFI_MAX_WORLD_MATERIAL_ASSET_BYTES: usize = 4 * 1024 * 1024;
pub const FFI_MAX_WORLD_MESH_TEXTURE_ASSET_BYTES: usize = 4 * 1024 * 1024;
pub const FFI_MAX_WORLD_MESH_INDEX_BYTES: usize = 1024 * 1024;
pub const FFI_MAX_SHADER_PACK_SOURCE_FILES: usize = 4096;
pub const FFI_MAX_SHADER_PACK_SOURCE_FILE_BYTES: usize = 4 * 1024 * 1024;
pub const FFI_MAX_SHADER_PACK_SOURCE_TOTAL_BYTES: usize = 64 * 1024 * 1024;
pub const FFI_MAX_SHADER_PACK_ASSET_FILES: usize = 4096;
pub const FFI_MAX_SHADER_PACK_ASSET_FILE_BYTES: usize = 32 * 1024 * 1024;
pub const FFI_MAX_SHADER_PACK_ASSET_TOTAL_BYTES: usize = 256 * 1024 * 1024;
pub const FFI_MAX_BATCH_ITEMS: usize = 65_536;

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FfiBackendKind {
    Vulkan = 1,
    OpenGl = 2,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiStructLayout {
    pub header: FfiHeader,
    pub struct_id: u32,
    pub byte_size: u32,
    pub alignment: u32,
    pub field_count: u32,
    pub field_offsets: [u32; 64],
}

impl Default for FfiStructLayout {
    fn default() -> Self {
        Self {
            header: FfiHeader::default(),
            struct_id: 0,
            byte_size: 0,
            alignment: 0,
            field_count: 0,
            field_offsets: [0; 64],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiContextCreateRequest {
    pub header: FfiHeader,
    pub backend_kind: u32,
    pub tracy_enabled: u32,
    pub label: FfiBytes,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiContextResult {
    pub header: FfiHeader,
    pub status: i32,
    pub error_domain: u32,
    pub context_id: u64,
    pub supported_feature_bits: u64,
    pub limits: FfiBackendLimits,
    pub metrics: FfiMetricsSnapshot,
}

impl Default for FfiContextResult {
    fn default() -> Self {
        Self {
            header: FfiHeader {
                version: FFI_ABI_VERSION,
                byte_size: size_of::<Self>() as u32,
            },
            status: StatusCode::Ok as i32,
            error_domain: 0,
            context_id: 0,
            supported_feature_bits: 0,
            limits: FfiBackendLimits::default(),
            metrics: FfiMetricsSnapshot::default(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiBorrowedOpenGlContextCreateRequest {
    pub header: FfiHeader,
    pub stable_window_id: u64,
    pub tracy_enabled: u32,
    pub reserved0: u32,
    pub label: FfiBytes,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiWindowedVulkanContextCreateRequest {
    pub header: FfiHeader,
    pub platform: u32,
    pub tracy_enabled: u32,
    pub stable_window_id: u64,
    pub native_display: u64,
    pub native_window: u64,
    pub label: FfiBytes,
    pub surface_label: FfiBytes,
    pub extent: FfiExtent3d,
    pub color_format: u32,
    pub present_mode: u32,
    pub max_frames_in_flight: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiFrameSurfaceConfigRequest {
    pub header: FfiHeader,
    pub label: FfiBytes,
    pub extent: FfiExtent3d,
    pub color_format: u32,
    pub present_mode: u32,
    pub max_frames_in_flight: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiFrameAcquireRequest {
    pub header: FfiHeader,
    pub correlation_id: u64,
    pub expected_extent: FfiExtent3d,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiFrameAcquireResult {
    pub header: FfiHeader,
    pub status: i32,
    pub error_domain: u32,
    pub frame_id: u64,
    pub correlation_id: u64,
    pub acquire_status: u32,
    pub frame_target: FfiHandle,
    pub frame_target_identity: u64,
    pub extent: FfiExtent3d,
    pub color_format: u32,
    pub metrics: FfiMetricsSnapshot,
}

impl Default for FfiFrameAcquireResult {
    fn default() -> Self {
        Self {
            header: FfiHeader {
                version: FFI_ABI_VERSION,
                byte_size: size_of::<Self>() as u32,
            },
            status: StatusCode::Ok as i32,
            error_domain: 0,
            frame_id: 0,
            correlation_id: 0,
            acquire_status: 0,
            frame_target: FfiHandle::default(),
            frame_target_identity: 0,
            extent: FfiExtent3d::default(),
            color_format: 0,
            metrics: FfiMetricsSnapshot::default(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiFrameResizeRequest {
    pub header: FfiHeader,
    pub correlation_id: u64,
    pub extent: FfiExtent3d,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiFrameResizeResult {
    pub header: FfiHeader,
    pub status: i32,
    pub error_domain: u32,
    pub resize_status: u32,
    pub extent: FfiExtent3d,
}

impl Default for FfiFrameResizeResult {
    fn default() -> Self {
        Self {
            header: FfiHeader {
                version: FFI_ABI_VERSION,
                byte_size: size_of::<Self>() as u32,
            },
            status: StatusCode::Ok as i32,
            error_domain: 0,
            resize_status: 0,
            extent: FfiExtent3d::default(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiFramePresentRequest {
    pub header: FfiHeader,
    pub frame_id: u64,
    pub correlation_id: u64,
    pub wait_submission_id: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiFramePresentResult {
    pub header: FfiHeader,
    pub status: i32,
    pub error_domain: u32,
    pub frame_id: u64,
    pub correlation_id: u64,
    pub present_status: u32,
    pub completed_submission_id: u64,
    pub frame_target_identity: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiFrameCancelRequest {
    pub header: FfiHeader,
    pub frame_id: u64,
    pub correlation_id: u64,
}

impl Default for FfiFramePresentResult {
    fn default() -> Self {
        Self {
            header: FfiHeader {
                version: FFI_ABI_VERSION,
                byte_size: size_of::<Self>() as u32,
            },
            status: StatusCode::Ok as i32,
            error_domain: 0,
            frame_id: 0,
            correlation_id: 0,
            present_status: 0,
            completed_submission_id: 0,
            frame_target_identity: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiReadbackRequest {
    pub header: FfiHeader,
    pub submission_id: u64,
    pub buffer: FfiHandle,
    pub offset: u64,
    pub size: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiReadbackResult {
    pub header: FfiHeader,
    pub status: i32,
    pub error_domain: u32,
    pub submission_id: u64,
    pub required_bytes: u64,
    pub written_bytes: u64,
    pub metrics: FfiMetricsSnapshot,
}

impl Default for FfiReadbackResult {
    fn default() -> Self {
        Self {
            header: FfiHeader {
                version: FFI_ABI_VERSION,
                byte_size: size_of::<Self>() as u32,
            },
            status: StatusCode::Ok as i32,
            error_domain: 0,
            submission_id: 0,
            required_bytes: 0,
            written_bytes: 0,
            metrics: FfiMetricsSnapshot::default(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FfiHeader {
    pub version: u32,
    pub byte_size: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FfiBytes {
    pub ptr: *const u8,
    pub len: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FfiSlice<T> {
    pub ptr: *const T,
    pub count: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FfiHandle {
    pub raw: u64,
}

impl From<Handle> for FfiHandle {
    fn from(handle: Handle) -> Self {
        Self { raw: handle.raw() }
    }
}

impl From<FfiHandle> for Handle {
    fn from(handle: FfiHandle) -> Self {
        Handle::from_raw(handle.raw)
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiResult {
    pub header: FfiHeader,
    pub status: i32,
    pub error_domain: u32,
    pub handle: FfiHandle,
    pub submission_id: u64,
    pub required_bytes: u64,
}

impl Default for FfiResult {
    fn default() -> Self {
        Self {
            header: FfiHeader {
                version: FFI_ABI_VERSION,
                byte_size: size_of::<Self>() as u32,
            },
            status: StatusCode::Ok as i32,
            error_domain: 0,
            handle: FfiHandle::default(),
            submission_id: 0,
            required_bytes: 0,
        }
    }
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FfiMemoryDomain {
    DeviceLocal = 1,
    Upload = 2,
    Readback = 3,
}

impl FfiMemoryDomain {
    pub fn validate(raw: u32) -> GalResult<Self> {
        match raw {
            1 => Ok(Self::DeviceLocal),
            2 => Ok(Self::Upload),
            3 => Ok(Self::Readback),
            _ => Err(GalError::ffi(
                StatusCode::UnknownEnum,
                format!("unknown memory domain {raw}"),
            )),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiBufferCreateRequest {
    pub header: FfiHeader,
    pub label: FfiBytes,
    pub size: u64,
    pub memory_domain: u32,
    pub usage_bits: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiResourceUse {
    pub resource: FfiHandle,
    pub stage_bits: u32,
    pub access_bits: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiCommandListRequest {
    pub header: FfiHeader,
    pub label: FfiBytes,
    pub encoded_ops: FfiBytes,
    pub resource_uses: FfiSlice<FfiResourceUse>,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiSubmissionRequest {
    pub header: FfiHeader,
    pub label: FfiBytes,
    pub command_lists: FfiSlice<FfiCommandListRequest>,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FfiGuiSpriteRequest {
    pub byte_size: u32,
    pub stratum: u32,
    pub sprite_id: u32,
    pub selected_slot: i32,
    pub progress_fraction: f32,
    pub fill_direction: u32,
    pub color_argb: u32,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub gui_width: i32,
    pub gui_height: i32,
    pub sequence: u64,
}

/// Generic GUI image quad. Java supplies affine screen-space geometry and
/// semantic image identity only; Rust owns its texture resource and batching.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FfiGuiAffineQuadRequest {
    pub byte_size: u32,
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
    pub gui_width: i32,
    pub gui_height: i32,
    pub sequence: u64,
    pub clip_mode: u32,
    pub clip_left: i32,
    pub clip_top: i32,
    pub clip_width: i32,
    pub clip_height: i32,
    pub material_mode: u32,
    pub item_raster_scale: u32,
    pub item_raster_corners: [f32; 6],
    pub item_raster_layers: FfiSlice<FfiGuiItemRasterLayer>,
}

/// ABI38: immutable item-local layers nested under one GUI presentation.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FfiGuiItemRasterLayer {
    pub byte_size: u32,
    pub material_mode: u32,
    pub asset_id: u64,
    pub color_argb: u32,
    pub corners: [f32;6],
    pub uv: [f32;4],
    pub model_transform: [f32;16],
}

/// ABI v29 typed tiled-GUI semantics. The producer remains diagnostic-only
/// until real paired captures establish admission; no native GPU handles cross.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FfiGuiTiledQuadRequest {
    pub byte_size: u32,
    pub stratum: u32,
    pub asset_id: u64,
    pub bounds: [i32; 4],
    pub tile_extent: [u32; 2],
    pub uv: [f32; 4],
    pub pose: [f32; 6],
    pub z: f32,
    pub color_argb: u32,
    pub sequence: u64,
    pub clip_mode: u32,
    pub clip: [i32; 4],
}

/// Fixed copied vertex for the private Rust-owned GUI mesh family. This is a
/// semantic vertex record: no Java renderer object, atlas object, or native
/// resource identity crosses this boundary.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FfiGuiMeshVertex {
    pub position: [f32; 3],
    pub atlas_uv: [f32; 2],
    pub local_uv: [f32; 2],
    pub color_argb: u32,
    pub normal_packed: u32,
    pub source_face: u32,
    pub source_foil_type: u32,
}

/// One coarse material-homogeneous GUI item mesh layer. Geometry payloads are
/// copied through nested bounded slices; the Rust GUI mesh frontend owns
/// offscreen targets, material resources, batching, and execution.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FfiGuiMeshBatchRequest {
    pub byte_size: u32,
    pub stratum: u32,
    pub layer_index: u32,
    pub material_mode: u32,
    pub lighting_mode: u32,
    pub asset_id: u64,
    pub sequence: u64,
    pub alpha_cutoff: f32,
    pub reserved0: u32,
    pub model_transform: [f32; 16],
    pub gui_pose: [f32; 6],
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub gui_width: i32,
    pub gui_height: i32,
    pub render_width: i32,
    pub render_height: i32,
    pub guard_pixels: u32,
    pub clip_mode: u32,
    pub clip_left: i32,
    pub clip_top: i32,
    pub clip_width: i32,
    pub clip_height: i32,
    pub vertices: FfiSlice<FfiGuiMeshVertex>,
    pub indices: FfiSlice<u32>,
    pub item_foil_mode: u32,
    pub item_foil_clock_millis: u64,
    pub item_foil_speed: f64,
    pub item_foil_strength: f32,
    pub item_raster_scale: u32,
    pub decal_foil_mode: u32,
    pub decal_model_pose: [f32; 16],
    pub decal_normal_pose: [f32; 9],
    pub block_item_scale: u32,
    pub block_model_bounds: [f64; 6],
    pub block_item_layout: u32,
    pub item_cache_identity: u64,
    pub item_cache_mode: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FfiGuiFrameSubmitRequest {
    pub header: FfiHeader,
    pub generation: u64,
    pub frame_id: u64,
    pub frame_target: FfiHandle,
    pub gui_width: i32,
    pub gui_height: i32,
    pub sprites: FfiSlice<FfiGuiSpriteRequest>,
    pub affine_quads: FfiSlice<FfiGuiAffineQuadRequest>,
    pub negotiated_feature_bits: u64,
    /// Appended semantic GUI item meshes. Each item may contain several
    /// ordered layers sharing one scheduler sequence.
    pub mesh_batches: FfiSlice<FfiGuiMeshBatchRequest>,
    pub gui_projection_width: f32,
    pub gui_projection_height: f32,
    pub tiled_quads: FfiSlice<FfiGuiTiledQuadRequest>,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiGuiFrameSubmitResult {
    pub header: FfiHeader,
    pub status: i32,
    pub error_domain: u32,
    pub submission_id: u64,
    pub sprite_count: u64,
    pub sprite_batch_count: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub resource_creates: u64,
    pub command_lists: u64,
    pub command_ops: u64,
    pub metrics: FfiMetricsSnapshot,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiGuiAssetPayload {
    pub byte_size: u32,
    pub sprite_id: u32,
    pub png_bytes: FfiBytes,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiGuiAssetUpdateRequest {
    pub header: FfiHeader,
    pub generation: u64,
    pub assets: FfiSlice<FfiGuiAssetPayload>,
    pub negotiated_feature_bits: u64,
}

/// Versioned raw GUI image transport. It is intentionally separate from the
/// legacy PNG sprite registry: font atlases are CPU-source pixel data, not
/// renderer textures or atlas handles.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiGuiRawImageAssetPayload {
    pub byte_size: u32,
    pub format: u32,
    pub asset_id: u64,
    pub width: i32,
    pub height: i32,
    pub pixels: FfiBytes,
    pub sampling_filter: u32,
    pub sampling_address: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiGuiRawImageUpdateRequest {
    pub header: FfiHeader,
    pub generation: u64,
    pub assets: FfiSlice<FfiGuiRawImageAssetPayload>,
    pub negotiated_feature_bits: u64,
}

/// Immutable semantic atlas identity and region, never pixels or GPU handles.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiGuiAtlasReference {
    pub byte_size: u32,
    pub texture_id: u32,
    pub asset_id: u64,
    pub atlas_generation: u64,
    pub atlas_width: u32,
    pub atlas_height: u32,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiGuiAtlasReferenceUpdate {
    pub header: FfiHeader,
    pub revision: u64,
    pub references: FfiSlice<FfiGuiAtlasReference>,
    pub negotiated_feature_bits: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiWorldBorderAssetUpdateRequest {
    pub header: FfiHeader,
    pub generation: u64,
    pub texture_id: u32,
    pub reserved0: u32,
    pub png_bytes: FfiBytes,
    pub negotiated_feature_bits: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiWorldCrackAssetPayload {
    pub byte_size: u32,
    pub stage: u32,
    pub png_bytes: FfiBytes,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiWorldCrackAssetUpdateRequest {
    pub header: FfiHeader,
    pub generation: u64,
    pub assets: FfiSlice<FfiWorldCrackAssetPayload>,
    pub negotiated_feature_bits: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiWorldMaterialAssetPayload {
    pub byte_size: u32,
    pub texture_id: u32,
    pub png_bytes: FfiBytes,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiWorldMaterialAssetUpdateRequest {
    pub header: FfiHeader,
    pub generation: u64,
    pub assets: FfiSlice<FfiWorldMaterialAssetPayload>,
    pub negotiated_feature_bits: u64,
}

/// One named, copied shader-pack source file. This is semantic pack input,
/// never a compiled shader, native resource, or backend object.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiShaderPackSourceFile {
    pub byte_size: u32,
    pub reserved0: u32,
    pub path_utf8: FfiBytes,
    pub contents_utf8: FfiBytes,
}

/// A complete source generation replaces the active owned source atomically.
/// It is deliberately separate from world mesh/material asset updates.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiShaderPackSourceUpdateRequest {
    pub header: FfiHeader,
    pub generation: u64,
    pub pack_name_utf8: FfiBytes,
    pub files: FfiSlice<FfiShaderPackSourceFile>,
}

/// One named, copied binary shader-pack asset. It carries pack-relative
/// semantic bytes only, never an atlas object, decoded texture, or backend
/// resource.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiShaderPackAssetFile {
    pub byte_size: u32,
    pub reserved0: u32,
    pub path_utf8: FfiBytes,
    pub contents: FfiBytes,
}

/// A complete binary asset generation paired with an active source
/// generation. Rust copies and validates it before any decode or backend
/// upload is considered.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiShaderPackAssetUpdateRequest {
    pub header: FfiHeader,
    pub generation: u64,
    pub pack_name_utf8: FfiBytes,
    pub files: FfiSlice<FfiShaderPackAssetFile>,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FfiWorldLineSegmentRequest {
    pub byte_size: u32,
    pub stratum: u32,
    pub style: u32,
    pub depth_policy: u32,
    pub color_argb: u32,
    pub line_width: f32,
    pub start_x: f32,
    pub start_y: f32,
    pub start_z: f32,
    pub end_x: f32,
    pub end_y: f32,
    pub end_z: f32,
    pub viewport_width: i32,
    pub viewport_height: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FfiWorldCrackQuadRequest {
    pub byte_size: u32,
    pub stratum: u32,
    pub stage: u32,
    pub depth_policy: u32,
    pub blend_policy: u32,
    pub cull_policy: u32,
    pub color_argb: u32,
    pub reserved0: u32,
    pub p0_x: f32,
    pub p0_y: f32,
    pub p0_z: f32,
    pub p1_x: f32,
    pub p1_y: f32,
    pub p1_z: f32,
    pub p2_x: f32,
    pub p2_y: f32,
    pub p2_z: f32,
    pub p3_x: f32,
    pub p3_y: f32,
    pub p3_z: f32,
    pub viewport_width: i32,
    pub viewport_height: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FfiWorldBorderQuadRequest {
    pub byte_size: u32,
    pub stratum: u32,
    pub texture_id: u32,
    pub depth_policy: u32,
    pub blend_policy: u32,
    pub cull_policy: u32,
    pub color_argb: u32,
    pub reserved0: u32,
    pub border_size: f32,
    pub distance_to_border: f32,
    pub scroll_u: f32,
    pub scroll_v: f32,
    pub uv_u: f32,
    pub uv_v: f32,
    pub uv_width: f32,
    pub uv_height: f32,
    pub p0_x: f32,
    pub p0_y: f32,
    pub p0_z: f32,
    pub p1_x: f32,
    pub p1_y: f32,
    pub p1_z: f32,
    pub p2_x: f32,
    pub p2_y: f32,
    pub p2_z: f32,
    pub p3_x: f32,
    pub p3_y: f32,
    pub p3_z: f32,
    pub viewport_width: i32,
    pub viewport_height: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FfiWorldMaterialQuadRequest {
    pub byte_size: u32,
    pub stratum: u32,
    pub material_id: u32,
    pub texture_id: u32,
    pub material_mode: u32,
    pub depth_policy: u32,
    pub cull_policy: u32,
    pub topology: u32,
    pub color_argb: u32,
    /// `0` preserves the historical CCW default. This direct-record field is
    /// part of the existing winding contract and must not be reused.
    pub winding: u32,
    pub p0_x: f32,
    pub p0_y: f32,
    pub p0_z: f32,
    pub p1_x: f32,
    pub p1_y: f32,
    pub p1_z: f32,
    pub p2_x: f32,
    pub p2_y: f32,
    pub p2_z: f32,
    pub p3_x: f32,
    pub p3_y: f32,
    pub p3_z: f32,
    pub uv0_u: f32,
    pub uv0_v: f32,
    pub uv1_u: f32,
    pub uv1_v: f32,
    pub uv2_u: f32,
    pub uv2_v: f32,
    pub uv3_u: f32,
    pub uv3_v: f32,
    pub viewport_width: i32,
    pub viewport_height: i32,
    /// Named source-pack material family. This is semantic program selection,
    /// never an Iris program or backend object.
    pub source_program: u32,
    /// Unlit producer color retained for a future source-derived material
    /// program. `color_argb` preserves the ordinary prelit material route.
    pub source_color_argb: u32,
    /// Vanilla packed block/sky light retained without baking it into source
    /// shader inputs.
    pub packed_light: u32,
    /// Semantic source UV coordinate space. It is ignored by the ordinary
    /// material route and consumed only by future source-derived staging.
    pub source_uv_space: u32,
    /// Per-vertex source modulation. Uniform quads repeat `source_color_argb`
    /// and `packed_light`; the legacy/full-record transport is reserved for
    /// the rare dynamic primitive that needs distinct endpoint values.
    pub vertex0_color_argb: u32,
    pub vertex1_color_argb: u32,
    pub vertex2_color_argb: u32,
    pub vertex3_color_argb: u32,
    pub vertex0_packed_light: u32,
    pub vertex1_packed_light: u32,
    pub vertex2_packed_light: u32,
    pub vertex3_packed_light: u32,
    pub block_entity_id: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiWorldMaterialTableRecord {
    pub byte_size: u32,
    pub stratum: u32,
    pub material_id: u32,
    pub texture_id: u32,
    pub material_mode: u32,
    pub depth_policy: u32,
    pub cull_policy: u32,
    pub topology: u32,
    pub winding: u32,
    pub source_program: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FfiWorldMaterialCompactQuadRequest {
    pub byte_size: u32,
    pub material_index: u32,
    pub color_argb: u32,
    /// Semantic source UV coordinate space, retained per quad because one
    /// material table entry can be used by more than one source family.
    pub source_uv_space: u32,
    pub p0_x: f32,
    pub p0_y: f32,
    pub p0_z: f32,
    pub p1_x: f32,
    pub p1_y: f32,
    pub p1_z: f32,
    pub p2_x: f32,
    pub p2_y: f32,
    pub p2_z: f32,
    pub p3_x: f32,
    pub p3_y: f32,
    pub p3_z: f32,
    pub uv0_u: f32,
    pub uv0_v: f32,
    pub uv1_u: f32,
    pub uv1_v: f32,
    pub uv2_u: f32,
    pub uv2_v: f32,
    pub uv3_u: f32,
    pub uv3_v: f32,
    pub source_color_argb: u32,
    pub packed_light: u32,
    pub block_entity_id: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FfiWorldMeshVertex {
    pub byte_size: u32,
    pub color_argb: u32,
    pub normal_packed: u32,
    pub light: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub u: f32,
    pub v: f32,
    pub atlas_u: f32,
    pub atlas_v: f32,
    pub shader_block_id: i32,
    pub shader_material_type: i32,
    /// Sodium's compact per-primitive material byte. Bit 0 selects mip
    /// sampling; bits 1..2 select the alpha-cutoff class.
    pub terrain_material_bits: u32,
    /// Copied signed-byte `at_midBlock` xyz plus the source emission byte.
    /// This is terrain-model semantics, not an Iris binding or GL state.
    pub mid_block_packed: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FfiWorldMeshSectionRecord {
    pub byte_size: u32,
    pub material_id: u32,
    pub texture_id: u32,
    pub material_mode: u32,
    pub cull_policy: u32,
    pub winding: u32,
    pub index_offset: u32,
    pub index_count: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FfiWorldMeshAssetRecord {
    pub byte_size: u32,
    pub vertex_layout_version: u32,
    pub index_type: u32,
    pub reserved0: u32,
    pub mesh_key: u64,
    pub mesh_generation: u64,
    pub vertices: FfiSlice<FfiWorldMeshVertex>,
    pub index_bytes: FfiBytes,
    pub sections: FfiSlice<FfiWorldMeshSectionRecord>,
    /// Canonical gameplay entity identity for entity-model assets. Empty means
    /// this immutable mesh is not an entity model. Rust resolves any selected
    /// source pack ID from `entity.properties`.
    pub entity_identity_utf8: FfiBytes,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiWorldMeshAnimationFrameRecord {
    pub byte_size: u32,
    pub frame_index: u32,
    pub duration_ticks: u32,
    pub reserved0: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiSpriteAnimationMip {
    pub byte_size: u32,
    pub width: u32,
    pub height: u32,
    pub reserved0: u32,
    pub rgba: FfiBytes,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiSpriteAnimationSource {
    pub byte_size: u32,
    pub sprite_id: u32,
    pub atlas_x: u32,
    pub atlas_y: u32,
    pub frame_width: u32,
    pub frame_height: u32,
    pub interpolate: u32,
    pub reserved0: u32,
    pub frames: FfiSlice<FfiWorldMeshAnimationFrameRecord>,
    pub mips: FfiSlice<FfiSpriteAnimationMip>,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiAtlasAnimationAssetUpdate {
    pub header: FfiHeader,
    pub texture_id: u32,
    pub reserved0: u32,
    pub generation: u64,
    pub initial_tick: u64,
    pub sprites: FfiSlice<FfiSpriteAnimationSource>,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiWorldMeshTextureAssetPayload {
    pub byte_size: u32,
    pub texture_id: u32,
    pub png_bytes: FfiBytes,
    pub frame_width: u32,
    pub frame_height: u32,
    pub frame_count: u32,
    pub frame_ticks: u32,
    pub animation_flags: u32,
    pub frame_row_size: u32,
    pub interpolation_policy: u32,
    pub reserved0: u32,
    pub animation_frames: FfiSlice<FfiWorldMeshAnimationFrameRecord>,
    pub mip_png_bytes: FfiSlice<FfiBytes>,
    /// Zero/zero retains the producer family's existing defaults. Otherwise
    /// filter is 1=nearest, 2=linear; address is 1=repeat, 2=clamp-to-edge.
    /// Partial or unknown descriptors are rejected before asset publication.
    pub sampling_filter: u32,
    pub sampling_address: u32,
    /// Zero retains the existing family contract; positive counts are exact.
    pub requested_mip_levels: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FfiWorldMeshSortedIndexRecord {
    pub byte_size: u32,
    pub index_type: u32,
    pub reserved0: u32,
    pub mesh_key: u64,
    pub mesh_generation: u64,
    pub index_generation: u64,
    pub index_bytes: FfiBytes,
}

/// Explicit retirement of one immutable world-mesh generation. This is a
/// backend-neutral resource-lifetime command; it is not a draw or a native
/// handle. Rust ignores a record whose generation no longer matches, so a
/// delayed eviction can never remove a newer replacement of the same key.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiWorldMeshAssetRetirementRecord {
    pub byte_size: u32,
    pub reserved0: u32,
    pub mesh_key: u64,
    pub mesh_generation: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FfiWorldMeshAssetUpdateRequest {
    pub header: FfiHeader,
    pub generation: u64,
    pub meshes: FfiSlice<FfiWorldMeshAssetRecord>,
    pub textures: FfiSlice<FfiWorldMeshTextureAssetPayload>,
    pub sorted_indices: FfiSlice<FfiWorldMeshSortedIndexRecord>,
    pub negotiated_feature_bits: u64,
    pub retirements: FfiSlice<FfiWorldMeshAssetRetirementRecord>,
    pub experience_orbs: FfiSlice<FfiWorldExperienceOrbAssetRecord>,
}

/// Immutable gameplay appearance and CPU resource identity, not GPU handles
/// or caller-selected vertices, UVs, materials, or raster state.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FfiWorldExperienceOrbAssetRecord {
    pub byte_size: u32,
    pub icon: u32,
    pub mesh_key: u64,
    pub mesh_generation: u64,
    pub red: u32,
    pub blue: u32,
    pub packed_light: u32,
    pub reserved0: u32,
}

/// Coarse, backend-neutral Distant Horizons LOD vertex semantics. This is a
/// copied CPU record, not a legacy DH/OpenGL vertex attribute declaration.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiWorldLodVertex {
    pub byte_size: u32,
    pub local_x: u16,
    pub local_y: u16,
    pub local_z: u16,
    pub packed_light_and_micro_offset: u16,
    pub color_rgba: u32,
    pub material_id: u32,
    pub normal_index: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiWorldLodSegmentRecord {
    pub byte_size: u32,
    pub layer: u32,
    pub vertices: FfiSlice<FfiWorldLodVertex>,
    /// Fixed 16-byte semantic DH vertex stream. Exactly one of this and
    /// `vertices` is populated; retaining the structured form keeps existing
    /// semantic producers ABI-compatible while the packed form avoids a
    /// Java-record/foreign-struct expansion for the real DH producer.
    pub packed_vertices: FfiSlice<u8>,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiWorldLodColumnAssetRecord {
    pub byte_size: u32,
    pub vertex_layout_version: u32,
    pub origin_x: i32,
    pub origin_y: i32,
    pub origin_z: i32,
    pub reserved0: u32,
    pub column_key: u64,
    pub column_generation: u64,
    pub segments: FfiSlice<FfiWorldLodSegmentRecord>,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiWorldLodColumnRetirementRecord {
    pub byte_size: u32,
    pub reserved0: u32,
    pub column_key: u64,
    pub column_generation: u64,
}

/// One stable, copied source identity for a Distant Horizons reduced quad.
/// The strings are semantic resource identities, never atlas objects or
/// backend handles. IDs in the segment records are one-based into the
/// enclosing column table; zero is unavailable and `u32::MAX` is mixed.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiWorldLodMaterialIdentityRecord {
    pub byte_size: u32,
    pub reserved0: u32,
    pub block_state_identity_utf8: FfiBytes,
    pub biome_identity_utf8: FfiBytes,
}

/// Per-emitted-quad semantic material references for one compact LOD segment.
/// This remains separate from DH's copied vertex stream so existing vertex
/// layout/version semantics stay intact.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiWorldLodSegmentMaterialProvenanceRecord {
    pub byte_size: u32,
    pub layer: u32,
    pub segment_index: u32,
    pub reserved0: u32,
    pub quad_material_ids: FfiSlice<u32>,
    pub quad_variant_states: FfiSlice<u8>,
    pub quad_variant_positions: FfiSlice<u64>,
}

/// One exact copied atlas region for one material identity and block face.
/// It is source data only: neither an atlas object nor a backend texture.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FfiWorldLodFaceMaterialRecord {
    pub byte_size: u32,
    pub material_id: u32,
    pub face: u32,
    /// Ordered co-planar face layer. Kept in the original reserved slot so
    /// the Panama layout remains exactly 88 bytes.
    pub face_layer: u32,
    pub atlas_identity_utf8: FfiBytes,
    pub sprite_identity_utf8: FfiBytes,
    pub u0: f32,
    pub v0: f32,
    pub u1: f32,
    pub v1: f32,
    /// Stable UV-corner permutation for the copied baked face. Tint semantics
    /// live in `face_layer` so this field stays an unambiguous permutation.
    pub uv_corner_order: u32,
    pub variant_position: u64,
}

/// Bounded semantic material sidecar paired with one immutable LOD column
/// asset generation. It is copied at the FFI boundary and cannot select a
/// textured route on its own.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FfiWorldLodColumnMaterialProvenanceRecord {
    pub byte_size: u32,
    pub reserved0: u32,
    pub column_key: u64,
    pub column_generation: u64,
    pub identities: FfiSlice<FfiWorldLodMaterialIdentityRecord>,
    pub segments: FfiSlice<FfiWorldLodSegmentMaterialProvenanceRecord>,
    pub face_materials: FfiSlice<FfiWorldLodFaceMaterialRecord>,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FfiWorldLodAssetUpdateRequest {
    pub header: FfiHeader,
    pub generation: u64,
    pub assets: FfiSlice<FfiWorldLodColumnAssetRecord>,
    pub retirements: FfiSlice<FfiWorldLodColumnRetirementRecord>,
    pub negotiated_feature_bits: u64,
    pub material_provenance: FfiSlice<FfiWorldLodColumnMaterialProvenanceRecord>,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiWorldLodColumnInstanceRecord {
    pub byte_size: u32,
    pub layer: u32,
    pub segment_index: u32,
    pub order: u32,
    pub column_key: u64,
    pub column_generation: u64,
}

/// Coarse resolved Distant Horizons render semantics for one combined frame.
/// This deliberately contains values consumed by the public DH terrain
/// program, never a program object, VBO/VAO identity, texture unit, or other
/// native renderer state.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FfiWorldLodRenderFrame {
    pub byte_size: u32,
    pub enabled: u32,
    pub flags: u32,
    pub world_y_offset: i32,
    pub combined_matrix: [f32; 16],
    pub model_view_matrix: [f32; 16],
    pub projection_matrix: [f32; 16],
    pub projection_inverse_matrix: [f32; 16],
    pub clip_distance: f32,
    pub micro_offset: f32,
    pub noise_intensity: f32,
    pub earth_radius: f32,
    pub noise_steps: u32,
    pub noise_dropoff: i32,
    pub reserved0: u32,
    pub camera_world_x: f32,
    pub camera_world_y: f32,
    pub camera_world_z: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FfiWorldMeshInstanceRecord {
    pub byte_size: u32,
    pub stratum: u32,
    pub mesh_section_index: u32,
    pub depth_policy: u32,
    pub cull_policy: u32,
    pub winding: u32,
    pub color_argb: u32,
    pub viewport_width: i32,
    pub viewport_height: i32,
    pub mesh_key: u64,
    pub mesh_generation: u64,
    /// Semantic entity/material identity for source-selected entity passes.
    /// Zero is the explicit generic default, not a backend handle or renderer
    /// object.
    pub entity_id: i32,
    /// Straight ARGB semantic entity-color override. A zero alpha value means
    /// no override, matching the source-program mix contract.
    pub entity_color_argb: u32,
    pub transform: [f32; 16],
    /// Straight ARGB semantic outline color consumed by the Rust-owned entity
    /// mask and post-effect chain; zero means no outline request.
    pub outline_color_argb: u32,
    /// Explicit semantic instance flags. Bit 0 requests outline-only drawing:
    /// the mesh feeds the Rust outline mask but is omitted from the regular
    /// material pass.
    pub flags: u32,
    /// Copied vanilla block-state registry identity for block-entity shader
    /// semantics. -1 is the explicit non-block-entity value; this is never an
    /// Iris map lookup or backend handle.
    pub block_entity_id: i32,
    /// 0: explicit matrix; 1: semantic section origin/full-precision camera.
    pub terrain_placement_mode: u32,
    pub terrain_origin: [i32; 3],
    pub terrain_camera: [f64; 3],
    pub item_foil_mode: u32,
    pub item_foil_clock_millis: u64,
    pub item_foil_speed: f64,
    pub item_foil_strength: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FfiWorldBackgroundRequest {
    pub byte_size: u32,
    pub enabled: u32,
    pub sky_type: u32,
    pub load_intent: u32,
    pub store_intent: u32,
    pub color_argb: u32,
    pub viewport_width: i32,
    pub viewport_height: i32,
    /// Appended vanilla sky semantics. These are copied gameplay/render-state
    /// values, not Java renderer resources, Iris state, or backend handles.
    pub sky_visible: u32,
    pub sky_sunrise_or_sunset: u32,
    pub sky_dark_disc: u32,
    pub sky_reserved0: u32,
    pub sky_sun_angle: f32,
    pub sky_time_of_day: f32,
    pub sky_rain_brightness: f32,
    pub sky_star_brightness: f32,
    pub sky_sunrise_and_sunset_color_argb: u32,
    pub sky_moon_phase: i32,
    pub sky_end_flash_intensity: f32,
    pub sky_end_flash_x_angle: f32,
    pub sky_end_flash_y_angle: f32,
    pub sky_color_argb: u32,
}

/// Coarse, backend-neutral world and camera semantics for a future
/// Rust-owned volume mapping. This deliberately carries no resource, shader,
/// renderer, or native backend object.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FfiWorldVoxelVolumeFrame {
    pub byte_size: u32,
    pub enabled: u32,
    pub reserved0: u32,
    pub reserved1: u32,
    pub world_generation: u64,
    pub resource_generation: u64,
    pub camera_x: f32,
    pub camera_y: f32,
    pub camera_z: f32,
    pub reserved2: u32,
}

/// Coarse, copied vanilla environment semantics for future Rust-owned
/// shader-pack execution. This deliberately excludes Iris state, shader
/// objects, GL/Vulkan objects, and pack-specific derived values.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FfiWorldShaderEnvironmentFrame {
    pub byte_size: u32,
    pub enabled: u32,
    pub frame_counter: i32,
    pub world_day: i32,
    pub world_generation: u64,
    pub world_time: u64,
    pub frame_time_seconds: f32,
    pub frame_time_counter: f32,
    pub time_of_day: f32,
    pub rain_strength: f32,
    pub thunder_strength: f32,
    pub sky_darken: f32,
    pub moon_phase: i32,
    /// Vanilla camera-submersion classification. This is semantic game
    /// state, never an Iris fog object: 0 none, 1 water, 2 lava, 3 powder
    /// snow.
    pub eye_submersion: i32,
    pub screen_brightness: f32,
    pub far_plane: f32,
    pub relative_eye_x: f32,
    pub relative_eye_y: f32,
    pub relative_eye_z: f32,
    pub sky_color_r: f32,
    pub sky_color_g: f32,
    pub sky_color_b: f32,
    pub darkness_light_factor: f32,
    pub night_vision: f32,
    pub fog_color_r: f32,
    pub fog_color_g: f32,
    pub fog_color_b: f32,
    /// Vanilla precipitation at the camera block: 0 none, 1 rain, 2 snow.
    /// Rust owns shader-pack-specific smoothing and use of this semantic.
    pub biome_precipitation: i32,
    /// Canonical vanilla or modded biome resource location at the camera.
    /// This remains gameplay data; Rust evaluates source-defined biome maps.
    pub biome_resource_location_utf8: FfiBytes,
    /// Canonical vanilla item-model identity in the player's main hand. An
    /// empty value represents no semantic held item; Rust owns `item.properties`
    /// resolution and never receives an Iris integer map.
    pub main_hand_item_model_resource_location_utf8: FfiBytes,
    /// Canonical vanilla item-model identity in the player's off hand. See
    /// `main_hand_item_model_resource_location_utf8` for ownership rules.
    pub off_hand_item_model_resource_location_utf8: FfiBytes,
    /// Raw vanilla block-light emission for the main-hand item. Rust applies
    /// pack-owned hand composition and never receives an Iris light provider.
    pub main_hand_item_light_emission: i32,
    /// Raw vanilla block-light emission for the off-hand item.
    pub off_hand_item_light_emission: i32,
    /// Exact scalar inputs for Rust's owned vanilla lightmap reconstruction.
    /// These are appended to preserve all prior Panama field offsets.
    pub lightmap_enabled: u32,
    pub lightmap_reserved: u32,
    pub lightmap_generation: u64,
    pub lightmap_ambient_light_factor: f32,
    pub lightmap_sky_factor: f32,
    pub lightmap_block_factor: f32,
    pub lightmap_night_vision_factor: f32,
    pub lightmap_darkness_scale: f32,
    pub lightmap_darken_world_factor: f32,
    pub lightmap_brightness_factor: f32,
    pub lightmap_sky_light_r: f32,
    pub lightmap_sky_light_g: f32,
    pub lightmap_sky_light_b: f32,
    pub lightmap_ambient_r: f32,
    pub lightmap_ambient_g: f32,
    pub lightmap_ambient_b: f32,
    /// Appended semantic shader-frame inputs. These preserve every prior
    /// Panama offset and contain only copied vanilla gameplay values.
    pub blindness: f32,
    pub darkness_factor: f32,
    pub eye_brightness_block: i32,
    pub eye_brightness_sky: i32,
    /// Copied vanilla/Sodium fog parameters. These preserve all earlier
    /// Panama offsets and are semantic inputs only: Rust derives any
    /// shader-pack compatibility representation from them.
    pub fog_parameter_color_r: f32,
    pub fog_parameter_color_g: f32,
    pub fog_parameter_color_b: f32,
    pub fog_parameter_color_a: f32,
    pub fog_environmental_start: f32,
    pub fog_environmental_end: f32,
    pub fog_render_distance_start: f32,
    pub fog_render_distance_end: f32,
    /// Appended copied DH configuration semantic. It intentionally carries no
    /// renderer instance, Java callback, or backend state.
    pub distant_horizons_render_distance: i32,
    /// Appended vanilla SKY pipeline fog range. This is immutable semantic
    /// game data, never a Java uniform buffer, GL state, or Iris object.
    pub fog_sky_end: f32,
    /// Appended vanilla CLOUDS pipeline fog range. It is distinct from the
    /// sky range in Frozen and remains copied gameplay data only.
    pub fog_clouds_end: f32,
}

/// Coarse inventory of Java feature families observed during the same real
/// whole-frame extraction. It is diagnostic route policy only: no Java
/// renderer object, render type, shader object, or backend state crosses FFI.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FfiWorldFeatureCoverage {
    pub byte_size: u32,
    pub model_submits: u32,
    pub model_part_submits: u32,
    pub block_model_submits: u32,
    pub ordinary_block_submits: u32,
    pub item_submits: u32,
    pub custom_geometry_submits: u32,
    pub shadow_submits: u32,
    pub flame_submits: u32,
    pub name_tag_submits: u32,
    pub text_submits: u32,
    pub hitbox_submits: u32,
    pub leash_submits: u32,
    pub particle_group_submits: u32,
}

/// Copied first-person frame semantics. This is deliberately an append-only
/// frame record: Java supplies neither an Iris hand pass nor any backend
/// object. Per-item transforms remain in ordinary semantic mesh records; the
/// hand pass owns its model-view, projection, and isolated depth domain
/// separately.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FfiWorldFirstPersonFrame {
    pub byte_size: u32,
    pub enabled: u32,
    pub clear_depth_before: u32,
    /// Number of leading entries in `world_first_person_mesh_instances` that
    /// belong to the main hand. Remaining entries belong to the off hand.
    /// This repurposes an ABI-reserved lane without changing the record size
    /// or any established Panama offset.
    pub main_hand_instance_count: u32,
    pub projection_matrix: [f32; 16],
    /// Appended in ABI v23 to preserve every pre-existing field offset.
    pub model_view_matrix: [f32; 16],
}

/// One copied, backend-neutral world-text glyph quad. The atlas asset is a
/// semantic Rust-owned resource identity; it is never a Java font texture or
/// a native backend handle.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FfiWorldTextQuadRequest {
    pub byte_size: u32,
    pub flags: u32,
    pub depth_policy: u32,
    pub packed_light: u32,
    pub color_argb: u32,
    pub reserved0: u32,
    pub asset_id: u64,
    pub atlas_generation: u64,
    pub atlas_revision: u64,
    pub distance_to_camera_sq: f64,
    pub model_view_matrix: [f32; 16],
    pub positions: [f32; 12],
    pub uvs: [f32; 8],
    /// Scoped semantic block-entity identity; `-1` means no block entity.
    pub block_entity_id: i32,
}

/// One copied source font-atlas image. This is an immutable semantic payload,
/// not a Java texture, atlas object, or backend resource.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FfiWorldTextImageAssetPayload {
    pub byte_size: u32,
    pub format: u32,
    pub width: u32,
    pub height: u32,
    pub asset_id: u64,
    pub atlas_generation: u64,
    pub atlas_revision: u64,
    pub pixels: FfiBytes,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FfiWorldTextImageUpdateRequest {
    pub header: FfiHeader,
    pub generation: u64,
    pub assets: FfiSlice<FfiWorldTextImageAssetPayload>,
    pub negotiated_feature_bits: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FfiWholeFrameSubmitRequest {
    pub header: FfiHeader,
    pub generation: u64,
    pub frame_id: u64,
    pub correlation_id: u64,
    pub frame_target: FfiHandle,
    pub gui_width: i32,
    pub gui_height: i32,
    pub viewport_width: i32,
    pub viewport_height: i32,
    pub view_matrix: [f32; 16],
    pub projection_matrix: [f32; 16],
    pub world_background: FfiWorldBackgroundRequest,
    pub world_segments: FfiSlice<FfiWorldLineSegmentRequest>,
    pub world_crack_quads: FfiSlice<FfiWorldCrackQuadRequest>,
    pub world_border_quads: FfiSlice<FfiWorldBorderQuadRequest>,
    pub world_material_quads: FfiSlice<FfiWorldMaterialQuadRequest>,
    pub world_material_table: FfiSlice<FfiWorldMaterialTableRecord>,
    pub world_material_compact_quads: FfiSlice<FfiWorldMaterialCompactQuadRequest>,
    pub world_mesh_instances: FfiSlice<FfiWorldMeshInstanceRecord>,
    pub gui_sprites: FfiSlice<FfiGuiSpriteRequest>,
    pub gui_affine_quads: FfiSlice<FfiGuiAffineQuadRequest>,
    pub negotiated_feature_bits: u64,
    pub voxel_volume_frame: FfiWorldVoxelVolumeFrame,
    pub shader_environment_frame: FfiWorldShaderEnvironmentFrame,
    pub world_lod_instances: FfiSlice<FfiWorldLodColumnInstanceRecord>,
    pub world_lod_render_frame: FfiWorldLodRenderFrame,
    pub world_feature_coverage: FfiWorldFeatureCoverage,
    /// Appended in ABI v20. The stream is semantic only and is consumed by
    /// the Rust-owned world-text pass; Java text draw state never crosses this
    /// boundary.
    pub world_text_quads: FfiSlice<FfiWorldTextQuadRequest>,
    /// Appended so legacy whole-frame field offsets remain stable. The mesh
    /// batch is consumed by the Rust-owned GUI mesh frontend.
    pub gui_mesh_batches: FfiSlice<FfiGuiMeshBatchRequest>,
    /// Appended in ABI v21. The enabled record selects the Rust-owned hand
    /// projection and fresh depth domain; disabled is the explicit semantic
    /// value for frames without first-person work.
    pub world_first_person_frame: FfiWorldFirstPersonFrame,
    /// Appended in ABI v22. These copied mesh references share the existing
    /// asset/cache contract, but are intentionally distinct from camera-space
    /// world instances. When the hand record is enabled they are consumed by
    /// the dedicated Rust first-person pass, never batched into the world.
    pub world_first_person_mesh_instances: FfiSlice<FfiWorldMeshInstanceRecord>,
    /// Appended in ABI v23. `-1` means no GUI blur boundary; a non-negative
    /// value identifies the source GUI stratum before which blur is requested.
    /// The request is consumed by the Rust-owned blur graph; selected source
    /// and normal-world composition both retain the same semantic boundary.
    pub gui_blur_before_stratum: i32,
    /// Appended in ABI v24. The vanilla menu-background blur radius in bounded
    /// semantic pixels; ignored when `gui_blur_before_stratum` is `-1`.
    pub gui_blur_radius: i32,
    /// Appended in ABI v25. Optional copied semantic post-effect identity;
    /// Rust resolves the definition and shader stages from its own matching
    /// shader-pack snapshots. Java post-chain objects and backend handles are
    /// never transported.
    pub post_effect_id: FfiBytes,
    pub gui_projection_width: f32,
    pub gui_projection_height: f32,
    pub gui_tiled_quads: FfiSlice<FfiGuiTiledQuadRequest>,
    /// ABI v32: immutable engine inputs, never the Java Globals GPU buffer.
    pub engine_globals_present: u32,
    pub engine_screen_width: u32,
    pub engine_screen_height: u32,
    pub engine_game_ticks: i64,
    pub engine_partial_tick: f32,
    pub engine_glint_alpha: f32,
    pub engine_menu_blur_radius: i32,
    /// ABI v50: ordinary particle semantics; no Java-expanded vertices.
    pub world_particle_quads: FfiSlice<FfiWorldParticleQuadRequest>,
    pub world_experience_orbs: FfiSlice<FfiWorldExperienceOrbInstanceRecord>,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FfiWorldExperienceOrbInstanceRecord {
    pub byte_size: u32,
    /// Insert before this ordinal in the non-orb mesh instance stream.
    pub mesh_index: u32,
    pub mesh_key: u64,
    pub mesh_generation: u64,
    pub entity_transform: [f32; 16],
    pub camera_orientation: [f32; 4],
    pub entity_id: i32,
    pub reserved0: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FfiWorldParticleQuadRequest {
    pub byte_size: u32,
    pub texture_id: u32,
    /// ABI 51: 0 ordinary opaque, 1 ordinary translucent, 2 terrain opaque,
    /// 3 terrain cutout. Backend policy is chosen in Rust, not transported.
    pub surface_kind: u32,
    /// Insert before this index in the decoded non-particle material stream.
    /// Nondecreasing values retain input order, including coincident particles.
    pub material_index: u32,
    pub center: [f32; 3],
    pub rotation: [f32; 4],
    pub size: f32,
    pub uv_bounds: [f32; 4],
    pub color_argb: u32,
    pub packed_light: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiWholeFrameSubmitResult {
    pub header: FfiHeader,
    pub status: i32,
    pub error_domain: u32,
    pub submission_id: u64,
    pub world_segment_count: u64,
    pub world_vertex_count: u64,
    pub world_batch_count: u64,
    pub world_draw_count: u64,
    pub world_crack_quad_count: u64,
    pub world_crack_batch_count: u64,
    pub world_crack_draw_count: u64,
    pub world_border_quad_count: u64,
    pub world_border_batch_count: u64,
    pub world_border_draw_count: u64,
    pub world_material_quad_count: u64,
    pub world_material_batch_count: u64,
    pub world_material_draw_count: u64,
    pub world_mesh_instance_count: u64,
    pub world_mesh_batch_count: u64,
    pub world_mesh_draw_count: u64,
    pub world_background_clear_count: u64,
    pub world_background_diagnostic_fallback_count: u64,
    pub world_background_sky_type: u64,
    pub world_background_color_argb: u64,
    pub depth_attachment_creates: u64,
    pub depth_attachment_reuses: u64,
    pub depth_attachment_retires: u64,
    pub outline_cache_hits: u64,
    pub outline_cache_misses: u64,
    pub crack_cache_hits: u64,
    pub crack_cache_misses: u64,
    pub border_cache_hits: u64,
    pub border_cache_misses: u64,
    pub material_cache_hits: u64,
    pub material_cache_misses: u64,
    pub mesh_cache_hits: u64,
    pub mesh_cache_misses: u64,
    pub sprite_count: u64,
    pub sprite_batch_count: u64,
    pub gui_mesh_item_count: u64,
    pub gui_mesh_batch_count: u64,
    pub gui_mesh_draw_count: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub resource_creates: u64,
    pub command_lists: u64,
    pub command_ops: u64,
    pub metrics: FfiMetricsSnapshot,
    pub profile: FfiWholeFrameProfileSnapshot,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FfiWholeFrameProfileSnapshot {
    pub ffi_decode_nanos: u64,
    pub gui_frontend_nanos: u64,
    pub world_frontend_total_nanos: u64,
    pub world_validate_frame_nanos: u64,
    pub world_batching_nanos: u64,
    pub world_resource_prepare_nanos: u64,
    pub world_prepare_target_query_nanos: u64,
    pub world_prepare_render_resources_nanos: u64,
    pub world_prepare_depth_attachment_nanos: u64,
    pub world_prepare_g_buffer_resources_nanos: u64,
    pub world_prepare_g_buffer_cache_check_nanos: u64,
    pub world_prepare_g_buffer_destroy_nanos: u64,
    pub world_prepare_g_buffer_plan_nanos: u64,
    pub world_prepare_g_buffer_create_nanos: u64,
    pub world_prepare_frame_pass_nanos: u64,
    pub world_mesh_section_expand_group_nanos: u64,
    pub shader_plan_lookup_nanos: u64,
    pub gal_command_generation_nanos: u64,
    pub gal_submit_total_nanos: u64,
    pub gal_validate_ops_nanos: u64,
    pub gal_validate_handles_nanos: u64,
    pub gal_hazard_analysis_nanos: u64,
    pub backend_encode_nanos: u64,
    pub backend_submit_nanos: u64,
    pub backend_retire_nanos: u64,
    pub vulkan_command_buffer_alloc_nanos: u64,
    pub vulkan_command_buffer_begin_nanos: u64,
    pub vulkan_command_recording_nanos: u64,
    pub vulkan_command_buffer_end_nanos: u64,
    pub vulkan_queue_submit_nanos: u64,
    pub vulkan_timeline_poll_nanos: u64,
    pub vulkan_timeline_wait_nanos: u64,
    pub vulkan_device_wait_idle_nanos: u64,
    pub vulkan_command_buffers_allocated: u64,
    pub vulkan_command_buffers_freed: u64,
    pub vulkan_wait_count: u64,
    pub vulkan_device_wait_idle_count: u64,
    pub resource_creates_delta: u64,
    pub resource_destroys_delta: u64,
    pub host_write_ops: u64,
    pub host_write_bytes: u64,
    pub barrier_ops: u64,
    pub pass_count: u64,
    pub draw_ops: u64,
    pub draw_indexed_ops: u64,
    pub pipeline_binds: u64,
    pub resource_set_binds: u64,
    pub gpu_timestamp_status: u64,
    pub gpu_shadow_depth_nanos: u64,
    pub gpu_terrain_opaque_nanos: u64,
    pub gpu_terrain_cutout_nanos: u64,
    pub gpu_deferred_lighting_nanos: u64,
    pub gpu_composite0_nanos: u64,
    pub gpu_composite1_nanos: u64,
    pub gpu_final_output_nanos: u64,
    pub gpu_frame_total_nanos: u64,
    pub g_buffer_persistent_cache_hits: u64,
    pub g_buffer_persistent_cache_misses: u64,
    pub g_buffer_final_binding_cache_hits: u64,
    pub g_buffer_final_binding_cache_misses: u64,
    pub g_buffer_attachment_creates: u64,
    pub g_buffer_pipeline_creates: u64,
    pub g_buffer_shader_module_creates: u64,
    pub g_buffer_descriptor_creates: u64,
    pub g_buffer_render_target_creates: u64,
    pub g_buffer_resources_retired: u64,
    pub world_prepare_g_buffer_persistent_key_nanos: u64,
    pub world_prepare_g_buffer_persistent_lookup_nanos: u64,
    pub world_prepare_g_buffer_final_key_nanos: u64,
    pub world_prepare_g_buffer_final_lookup_nanos: u64,
    pub world_prepare_g_buffer_final_create_nanos: u64,
    pub world_prepare_frame_target_attachment_query_nanos: u64,
    pub world_prepare_mesh_material_asset_nanos: u64,
    pub world_prepare_metrics_accounting_nanos: u64,
    pub g_buffer_final_pass_creates: u64,
    pub vulkan_acquire_nanos: u64,
    pub vulkan_present_nanos: u64,
    pub vulkan_present_wait_nanos: u64,
    pub vulkan_present_mode: u64,
    pub vulkan_requested_present_mode: u64,
    pub vulkan_supported_present_modes: u64,
    pub vulkan_present_mode_fallback_reason: u64,
    pub vulkan_acquired_image_index: u64,
    pub vulkan_swapchain_generation: u64,
    pub vulkan_swapchain_image_count: u64,
    pub vulkan_surface_min_image_count: u64,
    pub vulkan_surface_max_image_count: u64,
    pub vulkan_configured_frames_in_flight: u64,
    pub vulkan_images_in_flight: u64,
    pub vulkan_available_frame_slots: u64,
    pub gal_hazard_read_events: u64,
    pub gal_hazard_write_events: u64,
    pub gal_hazard_candidates_examined: u64,
    pub gal_hazard_conflicts: u64,
    pub gal_hazard_barriers_applied: u64,
    pub gal_hazard_active_read_entries: u64,
    pub gal_hazard_active_write_entries: u64,
    pub gal_command_ops_before_normalize: u64,
    pub gal_command_ops_after_normalize: u64,
    pub gal_redundant_pipeline_binds_removed: u64,
    pub gal_redundant_resource_set_binds_removed: u64,
    pub gal_redundant_vertex_buffer_binds_removed: u64,
    pub gal_redundant_index_buffer_binds_removed: u64,
    pub world_prepare_mesh_cache_scan_nanos: u64,
    pub world_prepare_material_resource_nanos: u64,
    pub world_prepare_mesh_stream_capacity_nanos: u64,
    pub world_prepare_mesh_stream_lookup_nanos: u64,
    pub world_prepare_mesh_stream_grow_nanos: u64,
    pub world_prepare_mesh_resource_nanos: u64,
    pub world_prepare_material_slot_check_nanos: u64,
    pub world_prepare_mesh_slot_check_nanos: u64,
    pub world_prepare_mesh_batch_count: u64,
    pub world_prepare_mesh_stream_required_bytes: u64,
    pub world_prepare_mesh_stream_capacity_bytes: u64,
    pub world_prepare_mesh_stream_grows: u64,
    pub world_mesh_stream_payload_pack_nanos: u64,
    pub world_mesh_draw_record_nanos: u64,
    pub world_mesh_stream_payload_bytes: u64,
    pub world_mesh_dynamic_offset_count: u64,
}

impl Default for FfiGuiFrameSubmitResult {
    fn default() -> Self {
        Self {
            header: FfiHeader {
                version: FFI_ABI_VERSION,
                byte_size: size_of::<Self>() as u32,
            },
            status: StatusCode::Ok as i32,
            error_domain: 0,
            submission_id: 0,
            sprite_count: 0,
            sprite_batch_count: 0,
            cache_hits: 0,
            cache_misses: 0,
            resource_creates: 0,
            command_lists: 0,
            command_ops: 0,
            metrics: FfiMetricsSnapshot::default(),
        }
    }
}

impl Default for FfiWholeFrameSubmitResult {
    fn default() -> Self {
        Self {
            header: FfiHeader {
                version: FFI_ABI_VERSION,
                byte_size: size_of::<Self>() as u32,
            },
            status: StatusCode::Ok as i32,
            error_domain: 0,
            submission_id: 0,
            world_segment_count: 0,
            world_vertex_count: 0,
            world_batch_count: 0,
            world_draw_count: 0,
            world_crack_quad_count: 0,
            world_crack_batch_count: 0,
            world_crack_draw_count: 0,
            world_border_quad_count: 0,
            world_border_batch_count: 0,
            world_border_draw_count: 0,
            world_material_quad_count: 0,
            world_material_batch_count: 0,
            world_material_draw_count: 0,
            world_mesh_instance_count: 0,
            world_mesh_batch_count: 0,
            world_mesh_draw_count: 0,
            world_background_clear_count: 0,
            world_background_diagnostic_fallback_count: 0,
            world_background_sky_type: 0,
            world_background_color_argb: 0,
            depth_attachment_creates: 0,
            depth_attachment_reuses: 0,
            depth_attachment_retires: 0,
            outline_cache_hits: 0,
            outline_cache_misses: 0,
            crack_cache_hits: 0,
            crack_cache_misses: 0,
            border_cache_hits: 0,
            border_cache_misses: 0,
            material_cache_hits: 0,
            material_cache_misses: 0,
            mesh_cache_hits: 0,
            mesh_cache_misses: 0,
            sprite_count: 0,
            sprite_batch_count: 0,
            gui_mesh_item_count: 0,
            gui_mesh_batch_count: 0,
            gui_mesh_draw_count: 0,
            cache_hits: 0,
            cache_misses: 0,
            resource_creates: 0,
            command_lists: 0,
            command_ops: 0,
            metrics: FfiMetricsSnapshot::default(),
            profile: FfiWholeFrameProfileSnapshot::default(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FfiRange {
    pub offset: u64,
    pub count: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FfiExtent3d {
    pub width: u32,
    pub height: u32,
    pub depth: u32,
}

impl From<FfiExtent3d> for Extent3d {
    fn from(extent: FfiExtent3d) -> Self {
        Self {
            width: extent.width,
            height: extent.height,
            depth: extent.depth,
        }
    }
}

impl From<Extent3d> for FfiExtent3d {
    fn from(extent: Extent3d) -> Self {
        Self {
            width: extent.width,
            height: extent.height,
            depth: extent.depth,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FfiTextureOrigin3d {
    pub x: u32,
    pub y: u32,
    pub z: u32,
}

impl From<FfiTextureOrigin3d> for TextureOrigin3d {
    fn from(origin: FfiTextureOrigin3d) -> Self {
        Self {
            x: origin.x,
            y: origin.y,
            z: origin.z,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FfiTextureSubresourceRange {
    pub base_mip: u32,
    pub mip_count: u32,
    pub base_layer: u32,
    pub layer_count: u32,
}

impl From<FfiTextureSubresourceRange> for TextureSubresourceRange {
    fn from(range: FfiTextureSubresourceRange) -> Self {
        Self {
            base_mip: range.base_mip,
            mip_count: range.mip_count,
            base_layer: range.base_layer,
            layer_count: range.layer_count,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FfiFeatureBits {
    pub bits: u64,
}

impl FfiFeatureBits {
    pub const GRAPHICS: u64 = 1 << 0;
    pub const COMPUTE: u64 = 1 << 1;
    pub const DESCRIPTOR_ARRAYS: u64 = 1 << 2;
    pub const OPTIONAL_BINDINGS: u64 = 1 << 3;
    pub const DYNAMIC_BUFFER_OFFSETS: u64 = 1 << 4;
    pub const UNIFORM_BUFFERS: u64 = 1 << 5;
    pub const STORAGE_BUFFERS: u64 = 1 << 6;
    pub const STORAGE_TEXTURES: u64 = 1 << 7;
    pub const INDIRECT_DRAW: u64 = 1 << 8;
    pub const INDIRECT_DISPATCH: u64 = 1 << 9;
    pub const MULTIPLE_COLOR_ATTACHMENTS: u64 = 1 << 10;
    pub const DEPTH_ONLY_PASS: u64 = 1 << 11;
    pub const BLENDED_PASS: u64 = 1 << 12;
    pub const TEXTURE_SUBRESOURCE_COPIES: u64 = 1 << 13;
    pub const TEXTURE_MIP_LEVELS: u64 = 1 << 14;
    pub const TEXTURE_ARRAY_LAYERS: u64 = 1 << 15;
    pub const HOST_BUFFER_ACCESS: u64 = 1 << 16;
    pub const PRESENTATION: u64 = 1 << 17;
    pub const RENDERDOC_CAPTURE: u64 = 1 << 18;
    pub const TRACY_ZONES: u64 = 1 << 19;
    pub const ALL_KNOWN: u64 = (1 << 20) - 1;
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FfiBackendLimits {
    pub max_buffer_size: u64,
    pub max_texture_extent_2d: u32,
    pub max_texture_mip_levels: u32,
    pub max_texture_array_layers: u32,
    pub max_resource_layout_bindings: u32,
    pub max_binding_array_count: u32,
    pub max_color_attachments: u32,
    pub max_dynamic_offsets_per_binding: u32,
    pub max_command_lists_per_submission: u32,
    pub max_commands_per_list: u32,
    pub max_draw_count: u32,
    pub max_dispatch_groups_per_axis: u32,
    pub max_label_bytes: u32,
    pub max_shader_bytes: u32,
    pub max_inline_bytes: u64,
    pub max_batch_items: u32,
}

impl From<BackendLimits> for FfiBackendLimits {
    fn from(limits: BackendLimits) -> Self {
        Self {
            max_buffer_size: limits.max_buffer_size,
            max_texture_extent_2d: limits.max_texture_extent_2d,
            max_texture_mip_levels: limits.max_texture_mip_levels,
            max_texture_array_layers: limits.max_texture_array_layers,
            max_resource_layout_bindings: limits.max_resource_layout_bindings,
            max_binding_array_count: limits.max_binding_array_count,
            max_color_attachments: limits.max_color_attachments,
            max_dynamic_offsets_per_binding: limits.max_dynamic_offsets_per_binding,
            max_command_lists_per_submission: limits.max_command_lists_per_submission,
            max_commands_per_list: limits.max_commands_per_list,
            max_draw_count: limits.max_draw_count,
            max_dispatch_groups_per_axis: limits.max_dispatch_groups_per_axis,
            max_label_bytes: FFI_MAX_LABEL_BYTES as u32,
            max_shader_bytes: FFI_MAX_SHADER_BYTES as u32,
            max_inline_bytes: FFI_MAX_INLINE_BYTES as u64,
            max_batch_items: FFI_MAX_BATCH_ITEMS as u32,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FfiCapabilityQueryRequest {
    pub header: FfiHeader,
    pub requested_feature_bits: u64,
    pub reserved0: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiCapabilityResult {
    pub header: FfiHeader,
    pub status: i32,
    pub error_domain: u32,
    pub supported_feature_bits: u64,
    pub negotiated_feature_bits: u64,
    pub limits: FfiBackendLimits,
    pub initial_presentation_supported: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FfiMetricsSnapshot {
    pub resource_creates: u64,
    pub resource_destroys: u64,
    pub submissions: u64,
    pub command_lists: u64,
    pub command_ops: u64,
    pub backend_submissions: u64,
    pub backend_waits: u64,
    pub retired_resources: u64,
    pub ffi_calls: u64,
    pub ffi_input_bytes: u64,
    pub ffi_output_bytes: u64,
}

impl From<&Metrics> for FfiMetricsSnapshot {
    fn from(metrics: &Metrics) -> Self {
        Self {
            resource_creates: metrics.resource_creates,
            resource_destroys: metrics.resource_destroys,
            submissions: metrics.submissions,
            command_lists: 0,
            command_ops: 0,
            backend_submissions: 0,
            backend_waits: 0,
            retired_resources: metrics.deferred_retires,
            ffi_calls: 0,
            ffi_input_bytes: 0,
            ffi_output_bytes: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiStatusResult {
    pub header: FfiHeader,
    pub status: i32,
    pub error_domain: u32,
    pub unsupported_feature: u32,
    pub primary_handle: FfiHandle,
    pub submission_id: u64,
    pub required_bytes: u64,
    pub metrics: FfiMetricsSnapshot,
}

impl Default for FfiStatusResult {
    fn default() -> Self {
        Self {
            header: FfiHeader {
                version: FFI_ABI_VERSION,
                byte_size: size_of::<Self>() as u32,
            },
            status: StatusCode::Ok as i32,
            error_domain: 0,
            unsupported_feature: 0,
            primary_handle: FfiHandle::default(),
            submission_id: 0,
            required_bytes: 0,
            metrics: FfiMetricsSnapshot::default(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FfiCreateResultEntry {
    pub request_id: u64,
    pub handle: FfiHandle,
    pub status: i32,
    pub error_domain: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiBufferDescAbi {
    pub byte_size: u32,
    pub request_id: u64,
    pub label: FfiBytes,
    pub size: u64,
    pub memory_domain: u32,
    pub usage_bits: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiTextureDescAbi {
    pub byte_size: u32,
    pub request_id: u64,
    pub label: FfiBytes,
    pub dimension: u32,
    pub format: u32,
    pub extent: FfiExtent3d,
    pub mip_levels: u32,
    pub array_layers: u32,
    pub usage_bits: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiTextureViewDescAbi {
    pub byte_size: u32,
    pub request_id: u64,
    pub label: FfiBytes,
    pub texture: FfiHandle,
    pub format: u32,
    pub base_mip: u32,
    pub mip_count: u32,
    pub base_layer: u32,
    pub layer_count: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiSamplerDescAbi {
    pub byte_size: u32,
    pub request_id: u64,
    pub label: FfiBytes,
    pub min_filter: u32,
    pub mag_filter: u32,
    pub mip_filter: u32,
    pub address_u: u32,
    pub address_v: u32,
    pub address_w: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiShaderModuleDescAbi {
    pub byte_size: u32,
    pub request_id: u64,
    pub label: FfiBytes,
    pub stage: u32,
    pub code_format: u32,
    pub code: FfiBytes,
    pub entry_point: FfiBytes,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiResourceBindingDescAbi {
    pub byte_size: u32,
    pub binding: u32,
    pub kind: u32,
    pub stage_bits: u32,
    pub array_count: u32,
    pub optional: u32,
    pub dynamic_offset_count: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiResourceLayoutDescAbi {
    pub byte_size: u32,
    pub request_id: u64,
    pub label: FfiBytes,
    pub bindings: FfiRange,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiResourceBindingAbi {
    pub byte_size: u32,
    pub binding: u32,
    pub array_index: u32,
    pub resource: FfiHandle,
    pub kind: u32,
    pub access_bits: u32,
    pub dynamic_offsets: FfiRange,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiResourceSetDescAbi {
    pub byte_size: u32,
    pub request_id: u64,
    pub label: FfiBytes,
    pub layout: FfiHandle,
    pub bindings: FfiRange,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiPipelineLayoutDescAbi {
    pub byte_size: u32,
    pub request_id: u64,
    pub label: FfiBytes,
    pub resource_layouts: FfiRange,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiGraphicsPipelineDescAbi {
    pub byte_size: u32,
    pub request_id: u64,
    pub label: FfiBytes,
    pub layout: FfiHandle,
    pub vertex_shader: FfiHandle,
    pub fragment_shader: FfiHandle,
    pub topology: u32,
    pub cull_mode: u32,
    pub blend: u32,
    pub depth_compare: u32,
    pub color_formats: FfiRange,
    pub depth_format: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiComputePipelineDescAbi {
    pub byte_size: u32,
    pub request_id: u64,
    pub label: FfiBytes,
    pub layout: FfiHandle,
    pub shader: FfiHandle,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiRenderTargetDescAbi {
    pub byte_size: u32,
    pub request_id: u64,
    pub label: FfiBytes,
    pub color_views: FfiRange,
    pub depth_stencil_view: FfiHandle,
    pub extent: FfiExtent3d,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiRenderPassDescAbi {
    pub byte_size: u32,
    pub request_id: u64,
    pub label: FfiBytes,
    pub target: FfiHandle,
    pub color_formats: FfiRange,
    pub depth_format: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiDestroyDescAbi {
    pub byte_size: u32,
    pub handle: FfiHandle,
    pub expected_kind: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiBufferUpdateAbi {
    pub byte_size: u32,
    pub buffer: FfiHandle,
    pub offset: u64,
    pub data: FfiBytes,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiTextureUpdateAbi {
    pub byte_size: u32,
    pub texture: FfiHandle,
    pub mip_level: u32,
    pub array_layer: u32,
    pub origin: FfiTextureOrigin3d,
    pub extent: FfiExtent3d,
    pub bytes_per_row: u32,
    pub rows_per_image: u32,
    pub data: FfiBytes,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiResourceBatch {
    pub header: FfiHeader,
    pub buffers: FfiSlice<FfiBufferDescAbi>,
    pub textures: FfiSlice<FfiTextureDescAbi>,
    pub texture_views: FfiSlice<FfiTextureViewDescAbi>,
    pub samplers: FfiSlice<FfiSamplerDescAbi>,
    pub shaders: FfiSlice<FfiShaderModuleDescAbi>,
    pub resource_layouts: FfiSlice<FfiResourceLayoutDescAbi>,
    pub resource_layout_bindings: FfiSlice<FfiResourceBindingDescAbi>,
    pub resource_sets: FfiSlice<FfiResourceSetDescAbi>,
    pub resource_set_bindings: FfiSlice<FfiResourceBindingAbi>,
    pub dynamic_offsets: FfiSlice<u64>,
    pub pipeline_layouts: FfiSlice<FfiPipelineLayoutDescAbi>,
    pub pipeline_layout_resource_layouts: FfiSlice<FfiHandle>,
    pub graphics_pipelines: FfiSlice<FfiGraphicsPipelineDescAbi>,
    pub compute_pipelines: FfiSlice<FfiComputePipelineDescAbi>,
    pub render_targets: FfiSlice<FfiRenderTargetDescAbi>,
    pub render_target_color_views: FfiSlice<FfiHandle>,
    pub render_passes: FfiSlice<FfiRenderPassDescAbi>,
    pub render_pass_color_formats: FfiSlice<u32>,
    pub buffer_updates: FfiSlice<FfiBufferUpdateAbi>,
    pub texture_updates: FfiSlice<FfiTextureUpdateAbi>,
    pub destroys: FfiSlice<FfiDestroyDescAbi>,
    pub negotiated_feature_bits: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FfiClearColor {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl From<FfiClearColor> for ClearColor {
    fn from(color: FfiClearColor) -> Self {
        Self {
            r: color.r,
            g: color.g,
            b: color.b,
            a: color.a,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FfiPassAttachmentAbi {
    pub byte_size: u32,
    pub view: FfiHandle,
    pub load_op: u32,
    pub store_op: u32,
    pub has_clear_color: u32,
    pub clear_color: FfiClearColor,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiBufferImageCopyAbi {
    pub byte_size: u32,
    pub buffer: FfiHandle,
    pub buffer_offset: u64,
    pub bytes_per_row: u32,
    pub rows_per_image: u32,
    pub texture: FfiHandle,
    pub texture_mip: u32,
    pub texture_layer: u32,
    pub texture_origin: FfiTextureOrigin3d,
    pub extent: FfiExtent3d,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiResourceBarrierAbi {
    pub byte_size: u32,
    pub resource: FfiHandle,
    pub has_subresources: u32,
    pub subresources: FfiTextureSubresourceRange,
    pub before: u32,
    pub after: u32,
    pub stage_bits: u32,
    pub access_bits: u32,
    pub src_queue: u32,
    pub dst_queue: u32,
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FfiCommandOpKind {
    BeginPass = 1,
    BindGraphicsPipeline = 2,
    BindComputePipeline = 3,
    BindResourceSet = 4,
    SetVertexBuffer = 5,
    SetIndexBuffer = 6,
    Draw = 7,
    DrawIndexed = 8,
    DrawIndirect = 9,
    Dispatch = 10,
    DispatchIndirect = 11,
    CopyBuffer = 12,
    CopyBufferToTexture = 13,
    CopyTextureToBuffer = 14,
    HostWriteBuffer = 15,
    HostReadBuffer = 16,
    Present = 17,
    Barrier = 18,
    EndPass = 19,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiCommandOpAbi {
    pub byte_size: u32,
    pub op_kind: u32,
    pub primary: FfiHandle,
    pub secondary: FfiHandle,
    pub tertiary: FfiHandle,
    pub set_index: u32,
    pub slot: u32,
    pub offset: u64,
    pub size: u64,
    pub count0: u32,
    pub count1: u32,
    pub count2: u32,
    pub colors: FfiRange,
    pub depth_stencil: FfiRange,
    pub copy_region: FfiRange,
    pub barrier: FfiRange,
    pub inline_bytes: FfiBytes,
    pub subresources: FfiTextureSubresourceRange,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiCommandListAbi {
    pub byte_size: u32,
    pub label: FfiBytes,
    pub operations: FfiRange,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FfiSubmissionBatchAbi {
    pub header: FfiHeader,
    pub label: FfiBytes,
    pub command_lists: FfiSlice<FfiCommandListAbi>,
    pub operations: FfiSlice<FfiCommandOpAbi>,
    pub pass_attachments: FfiSlice<FfiPassAttachmentAbi>,
    pub copy_regions: FfiSlice<FfiBufferImageCopyAbi>,
    pub barriers: FfiSlice<FfiResourceBarrierAbi>,
    pub negotiated_feature_bits: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiCompletionQueryRequest {
    pub header: FfiHeader,
    pub submission_id: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiCompletionResult {
    pub header: FfiHeader,
    pub status: i32,
    pub error_domain: u32,
    pub requested_submission_id: u64,
    pub completed_submission_id: u64,
    pub is_complete: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FfiRetirementBatch {
    pub header: FfiHeader,
    pub completed_submission_id: u64,
    pub handles: FfiSlice<FfiHandle>,
}
