//! Translucent geometry analysis and sorting.
//!
//! The analyzer records retained translucent quads in Java-compatible ABI
//! layouts, classifies simple static ordering cases, and falls back to topology
//! or BSP ordering when raw quad order is not semantically sufficient. The FFI
//! wrapper module owns raw pointer validation; this module owns the algorithms
//! and data structures needed by both native meshing and replay tests.

use std::slice;

use super::gfni_trigger::NativeGeometryPlanes;
use super::{index, meshing};

const OK: i32 = 0;
const SORT_FAILED: i32 = 1;
const ERR_NULL_POINTER: i32 = -1;
const ERR_INVALID_ARGUMENT: i32 = -2;
const ERR_CAPACITY: i32 = -3;

const FACING_COUNT: usize = 7;
const FACING_DIRECTIONS: usize = 6;
const FACING_POS_X: i32 = 0;
const FACING_POS_Y: i32 = 1;
const FACING_POS_Z: i32 = 2;
const FACING_NEG_X: i32 = 3;
const FACING_NEG_Y: i32 = 4;
const FACING_NEG_Z: i32 = 5;
const FACING_UNASSIGNED: i32 = 6;

const SORT_MODE_NONE: i32 = 0;
const SORT_MODE_STATIC: i32 = 1;

const SORT_TYPE_NONE: i32 = 2;
const SORT_TYPE_STATIC_NORMAL_RELATIVE: i32 = 3;
const SORT_TYPE_STATIC_TOPO: i32 = 4;
const SORT_TYPE_DYNAMIC: i32 = 5;

const VERTEX_EPSILON: f32 = 0.00001;
const QUANTIZE_EPSILON: f32 = 1.0 / 256.0;
const HALF_SPACE_EPSILON: f32 = 0.001;
const NORMAL_COMPONENT_RANGE: f32 = 127.0;
const NATIVE_QUAD_STRIDE: u64 = 152;
const BSP_NODE_REUSE_THRESHOLD: usize = 30;

const ALIGNED_NORMALS: [(i8, i8, i8); FACING_DIRECTIONS] = [
    (127, 0, 0),
    (0, 127, 0),
    (0, 0, 127),
    (-127, 0, 0),
    (0, -127, 0),
    (0, 0, -127),
];
const STATIC_TOPO_SORT_ATTEMPT_LIMITS: [i32; 6] = [-1, -1, 250, 100, 50, 30];
#[repr(C)]
#[derive(Clone, Copy)]
pub struct TranslucentQuadRecord {
    positions: [f32; 12],
    facing: i32,
    packed_normal: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TranslucentTopoQuadRecord {
    positions: [f32; 12],
    extents: [f32; 6],
    accurate_dot_product: f32,
    facing: i32,
    packed_normal: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct NativeFullQuadVertex {
    x: f32,
    y: f32,
    z: f32,
    color: i32,
    ao: f32,
    u: f32,
    v: f32,
    light: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct NativeFullQuadBuffer {
    vertices: [NativeFullQuadVertex; 4],
    block_emission: u8,
    render_type: u8,
    ignore_mid_block: u8,
    _padding: u8,
    block_id: i32,
    local_x: i32,
    local_y: i32,
    local_z: i32,
    material_bits: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NativeFullQuadState {
    positions: [f32; 12],
    extents: [f32; 6],
    center: [f32; 3],
    accurate_normal: [f32; 3],
    accurate_dot_product: f32,
    quantized_dot_product: f32,
    facing: i32,
    packed_normal: i32,
    same_vertex_map: i32,
    normal_is_very_accurate: i32,
    has_updated_vertices: i32,
    write_to_index: i32,
}

#[derive(Clone)]
struct QuadInfo {
    positions: [f32; 12],
    center: (f32, f32, f32),
    facing: i32,
    topo_facing: i32,
    packed_normal: i32,
    extents: [f32; 6],
    accurate_dot_product: f32,
    quantized_dot_product: f32,
}

#[derive(Clone)]
struct Analyzer {
    mesh_facing_counts: [i32; FACING_COUNT],
    sort_type: i32,
    quad_hash: i32,
    aligned_facing_bitmap: i32,
    is_double_unaligned: bool,
    static_keys: Vec<i32>,
}

struct NativeTranslucentSectionGeometry {
    quads: Vec<QuadInfo>,
    aligned_separator_distances: [Vec<f32>; FACING_DIRECTIONS],
}

enum NativeTranslucentSortDataKind {
    StaticIndexData(Vec<i32>),
    DynamicTopo(NativeTranslucentSectionGeometry),
}

struct NativeTranslucentSortData {
    quad_count: usize,
    kind: NativeTranslucentSortDataKind,
}

struct NativeTranslucentAnalyzer {
    records: Vec<TranslucentQuadRecord>,
}

struct NativeTopoQuadStore {
    quads: Vec<Option<QuadInfo>>,
}

#[derive(Clone)]
struct NativeFullTQuad {
    quad: NativeFullQuadBuffer,
    info: QuadInfo,
    same_vertex_map: i32,
    normal_is_very_accurate: bool,
    accurate_normal: [f32; 3],
    has_updated_vertices: bool,
    write_to_index: i32,
}

#[derive(Clone)]
enum BspRemapKind {
    None,
    FixedOffset(i32),
    IndexMap(Vec<i32>),
}

#[derive(Clone)]
struct BspRemap {
    index_count: usize,
    kind: BspRemapKind,
}

impl BspRemap {
    fn none() -> Self {
        Self {
            index_count: 0,
            kind: BspRemapKind::None,
        }
    }

    fn is_active(&self) -> bool {
        !matches!(self.kind, BspRemapKind::None)
    }
}

#[derive(Clone)]
enum BspNode {
    LeafSingle {
        quad: i32,
    },
    LeafDouble {
        quad_a: i32,
        quad_b: i32,
    },
    LeafMulti {
        quads: Vec<i32>,
    },
    FixedDouble {
        remap: BspRemap,
        first: i32,
        second: i32,
    },
    Binary {
        remap: BspRemap,
        normal: [f32; 3],
        distance: f32,
        inside: i32,
        outside: i32,
        on_plane: Vec<i32>,
    },
    MultiPartition {
        remap: BspRemap,
        normal: [f32; 3],
        plane_distances: Vec<f32>,
        partitions: Vec<i32>,
        on_plane_quads: Vec<Vec<i32>>,
    },
}

#[derive(Clone)]
struct NativeBspTree {
    nodes: Vec<BspNode>,
    root: i32,
    index_quad_count: usize,
}

struct NativeBspReusableRoot {
    tree: NativeBspTree,
    geometry_planes: NativeGeometryPlanes,
    reuse_data: BspNodeReuseData,
}

struct NativeBspBuildResult {
    geometry_planes: Option<Box<NativeGeometryPlanes>>,
    tree: Option<Box<NativeBspTree>>,
    owned_split_quads: Vec<Box<NativeFullTQuad>>,
}

struct BspBuildOutput {
    tree: NativeBspTree,
    reusable_root: Option<Box<NativeBspReusableRoot>>,
}

struct BspNodeReuseData {
    quad_extents: Vec<[f32; 6]>,
    indexes: Vec<i32>,
    index_count: usize,
    max_index: i32,
}

struct BspBuildPartition {
    distance: f32,
    before: Vec<i32>,
    on: Vec<i32>,
}

struct BspBuildIntervalPoint {
    distance_key: i32,
    distance: f32,
    quad_index: i32,
    point_type: i32,
}

struct BspFullQuadBuildOutput {
    tree: NativeBspTree,
    reusable_root: Option<Box<NativeBspReusableRoot>>,
    owned_split_quads: Vec<Box<NativeFullTQuad>>,
    updated_quad_handles: Vec<u64>,
    mesh_quad_count: usize,
    index_quad_count: usize,
}

struct BspFullQuadWorkspace<'a> {
    tree: NativeBspTree,
    geometry_planes: &'a mut NativeGeometryPlanes,
    quads: Vec<QuadInfo>,
    handles: Vec<u64>,
    owned_split_quads: Vec<Box<NativeFullTQuad>>,
    updated_quad_handles: Vec<u64>,
    mesh_quad_count: usize,
    index_quad_count: usize,
    max_quad_count: usize,
    quantize_trigger_normals: bool,
}

struct BspActiveRemap {
    node_index: usize,
    remaining: usize,
}

struct BspTraversalState {
    quad_indexes: Vec<i32>,
    active_remap: Option<BspActiveRemap>,
}

mod analyzer;
mod bsp;
mod ffi;
mod geometry;
pub(crate) mod semantic;
mod sort;
mod topology;

use analyzer::*;
use bsp::*;
use geometry::*;
use sort::*;
use topology::*;

pub use ffi::*;

#[cfg(test)]
mod tests;
