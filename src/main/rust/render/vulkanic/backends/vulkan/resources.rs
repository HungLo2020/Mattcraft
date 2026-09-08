use std::collections::{BTreeMap, HashMap};
use std::ffi::{CStr, CString};
use std::io::Cursor;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use ash::vk;

use super::device::VulkanContext;
use super::shaderc_spirv_compiler::compile_glsl_for_backend;
use super::trace;
use crate::render::vulkanic::backends::{BackendCreateDesc, BackendToken};
use crate::render::vulkanic::error::{GalError, GalResult};
use crate::render::vulkanic::handles::{Handle, HandleKind};
use crate::render::vulkanic::resources::*;

/// A modest page amortizes the native allocation granularity many Vulkan
/// drivers apply to small buffers.  This is deliberately backend-private: GAL
/// still owns individually addressable buffers, lifetimes, and barriers.
// The NVIDIA Vulkan driver used by the supported Linux route maps each host
// allocation at 64 MiB granularity.  Keep uploads in pages of that size so the
// many small, concurrently-live source buffers share one native mapping
// instead of each 8 MiB page becoming a separate driver-sized allocation.
// This is backend-private suballocation; GAL buffers remain distinct resources
// with their existing explicit lifetime and synchronization.
const HOST_VISIBLE_BUFFER_PAGE_SIZE: u64 = 64 * 1024 * 1024;
const DEVICE_LOCAL_BUFFER_PAGE_SIZE: u64 = 64 * 1024 * 1024;
/// Images are individually addressable GAL resources, but allocating one
/// `VkDeviceMemory` object per image causes the NVIDIA driver to reserve a
/// large native mapping for each small render target, atlas, and GUI image.
/// Keep the backend-private allocation granularity aligned with device-local
/// buffer pages while preserving each image's explicit GAL lifetime.
const DEVICE_LOCAL_TEXTURE_PAGE_SIZE: u64 = 64 * 1024 * 1024;
/// A mesh section's descriptor set is small, but many sections become visible
/// during a normal terrain warm-up.  Backends commonly reserve sizeable native
/// chunks per descriptor pool, so a pool per set makes otherwise bounded GAL
/// resource-set residency consume unbounded-looking host memory.
const DESCRIPTOR_POOL_PAGE_SET_COUNT: u32 = 128;
/// Keep one completely free page for each native memory type.  NVIDIA keeps a
/// large process mapping after `vkFreeMemory`; immediately freeing every
/// short-lived upload page therefore turns a sequence of small staging writes
/// into many driver-sized mappings.  This is deliberately a strict bound: it
/// is a backend-private reuse cache, not an extension of GAL resource
/// lifetime, and teardown still frees every page.
const EMPTY_MEMORY_PAGES_RETAINED_PER_TYPE: usize = 1;
const MAX_COMPILED_SHADER_CACHE_ENTRIES: usize = 512;
/// The cache stores only weak references: it never extends a GAL pipeline's
/// lifetime.  Its fixed key bound prevents one-off shader-pack variants from
/// accumulating Rust-side lookup metadata while still letting concurrently
/// live, semantically identical GAL pipelines share one native pipeline.
const MAX_NATIVE_GRAPHICS_PIPELINE_CACHE_KEYS: usize = 512;
/// Native Vulkan drivers commonly keep substantial freed allocations in the
/// process heap after a streaming-world route changes ABI.  Trim only after a
/// bounded group of actual object retirements and only when glibc reports a
/// meaningful reusable arena; this is a backend-private residency policy, not
/// a GAL lifetime or synchronization decision.
const NATIVE_ALLOCATOR_TRIM_RETIRE_INTERVAL: u32 = 32;
const NATIVE_ALLOCATOR_TRIM_MIN_FREE_BYTES: usize = 128 * 1024 * 1024;
static GLIBC_ALLOCATION_TRACE_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static NATIVE_GRAPHICS_PIPELINE_CACHE_TRACE_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static VULKAN_RESIDENCY_TRACE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Captures allocator residency only in an explicitly requested diagnostic
/// run. The Vulkan driver shares this process allocator, so this separates
/// ordinary Rust/GAL ownership from driver-side anonymous mappings without
/// changing any resource lifetime or allocation policy.
fn trace_glibc_allocator_checkpoint(resource_label: &str) {
    if std::env::var_os("MATTMC_TRACE_GLIBC_ALLOCATOR").is_none() {
        return;
    }
    let sequence = GLIBC_ALLOCATION_TRACE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    if std::env::var_os("MATTMC_TRACE_GLIBC_ALLOCATOR_VERBOSE").is_none() && sequence % 64 != 0 {
        return;
    }
    #[cfg(target_os = "linux")]
    {
        let info = unsafe { libc::mallinfo2() };
        eprintln!(
            "vulkan.glibc-allocator sequence={} resource={} arena_bytes={} allocated_bytes={} free_bytes={} mmap_blocks={} mmap_bytes={}",
            sequence,
            resource_label,
            info.arena,
            info.uordblks,
            info.fordblks,
            info.hblks,
            info.hblkhd,
        );
    }
}

fn trace_native_graphics_pipeline_cache(hit: bool, resource_label: &str, key_count: usize) {
    if std::env::var_os("MATTMC_TRACE_VK_NATIVE_PIPELINE_CACHE").is_none() {
        return;
    }
    let sequence = NATIVE_GRAPHICS_PIPELINE_CACHE_TRACE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    eprintln!(
        "vulkan.graphics-pipeline.cache sequence={} result={} keys={} resource={}",
        sequence,
        if hit { "hit" } else { "miss" },
        key_count,
        resource_label,
    );
}

fn should_trim_native_allocator(retired_since_trim: u32, free_bytes: usize) -> bool {
    retired_since_trim >= NATIVE_ALLOCATOR_TRIM_RETIRE_INTERVAL
        && free_bytes >= NATIVE_ALLOCATOR_TRIM_MIN_FREE_BYTES
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ShaderCompileKey {
    stage: ShaderStage,
    entry_point: String,
    code: Vec<u8>,
}

/// Immutable Vulkan pipeline state, deliberately excluding the diagnostic
/// label.  Handles include their generation, so a recycled GAL slot can never
/// alias a pipeline whose shader or layout dependency has changed.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct GraphicsPipelineCacheKey {
    layout: Handle,
    vertex_shader: Handle,
    fragment_shader: Handle,
    topology: u32,
    cull_mode: u32,
    front_face: u32,
    blend: u32,
    depth_compare: Option<u32>,
    depth_write: bool,
    depth_bias: Option<DepthBiasCacheKey>,
    color_formats: Vec<TextureFormat>,
    depth_format: Option<TextureFormat>,
    stencil: Option<StencilStateCacheKey>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct DepthBiasCacheKey {
    constant_factor: u32,
    slope_factor: u32,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct StencilFaceStateCacheKey {
    compare: u32,
    reference: u32,
    read_mask: u32,
    write_mask: u32,
    fail_op: u32,
    depth_fail_op: u32,
    pass_op: u32,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct StencilStateCacheKey {
    front: StencilFaceStateCacheKey,
    back: StencilFaceStateCacheKey,
}

impl GraphicsPipelineCacheKey {
    fn from_desc(desc: &GraphicsPipelineDesc) -> Self {
        let stencil_face = |face: StencilFaceState| StencilFaceStateCacheKey {
            compare: face.compare as u32,
            reference: face.reference,
            read_mask: face.read_mask,
            write_mask: face.write_mask,
            fail_op: face.fail_op as u32,
            depth_fail_op: face.depth_fail_op as u32,
            pass_op: face.pass_op as u32,
        };
        Self {
            layout: desc.layout,
            vertex_shader: desc.vertex_shader,
            fragment_shader: desc.fragment_shader,
            topology: desc.topology as u32,
            cull_mode: desc.cull_mode as u32,
            front_face: desc.front_face as u32,
            blend: desc.blend as u32,
            depth_compare: desc.depth_compare.map(|compare| compare as u32),
            depth_write: desc.depth_write,
            depth_bias: desc.depth_bias.map(|bias| DepthBiasCacheKey {
                constant_factor: bias.constant_factor.to_bits(),
                slope_factor: bias.slope_factor.to_bits(),
            }),
            color_formats: desc.color_formats.clone(),
            depth_format: desc.depth_format,
            stencil: desc.stencil.map(|stencil| StencilStateCacheKey {
                front: stencil_face(stencil.front),
                back: stencil_face(stencil.back),
            }),
        }
    }
}

#[derive(Clone, Copy)]
struct DeviceMemoryAllocation {
    block_id: u64,
    memory: vk::DeviceMemory,
    offset: u64,
    size: u64,
}

struct DeviceMemoryBlock {
    id: u64,
    memory: vk::DeviceMemory,
    memory_type_index: u32,
    size: u64,
    free_ranges: Vec<BufferMemoryRange>,
}

#[derive(Clone, Copy)]
struct BufferMemoryRange {
    offset: u64,
    size: u64,
}

impl DeviceMemoryBlock {
    fn allocate(&mut self, size: u64, alignment: u64) -> Option<u64> {
        for index in 0..self.free_ranges.len() {
            let range = self.free_ranges[index];
            let offset = align_up(range.offset, alignment)?;
            let end = offset.checked_add(size)?;
            let range_end = range.offset.checked_add(range.size)?;
            if end > range_end {
                continue;
            }
            self.free_ranges.remove(index);
            if offset > range.offset {
                self.free_ranges.insert(
                    index,
                    BufferMemoryRange {
                        offset: range.offset,
                        size: offset - range.offset,
                    },
                );
            }
            if end < range_end {
                let tail_index = if offset > range.offset {
                    index + 1
                } else {
                    index
                };
                self.free_ranges.insert(
                    tail_index,
                    BufferMemoryRange {
                        offset: end,
                        size: range_end - end,
                    },
                );
            }
            return Some(offset);
        }
        None
    }

    fn release(&mut self, allocation: DeviceMemoryAllocation) {
        debug_assert_eq!(self.id, allocation.block_id);
        self.free_ranges.push(BufferMemoryRange {
            offset: allocation.offset,
            size: allocation.size,
        });
        self.free_ranges.sort_by_key(|range| range.offset);
        let mut merged: Vec<BufferMemoryRange> = Vec::with_capacity(self.free_ranges.len());
        for range in self.free_ranges.drain(..) {
            if let Some(previous) = merged.last_mut() {
                let previous_end = previous
                    .offset
                    .checked_add(previous.size)
                    .expect("buffer allocator range overflow");
                if previous_end == range.offset {
                    previous.size = previous
                        .size
                        .checked_add(range.size)
                        .expect("buffer allocator range overflow");
                    continue;
                }
                debug_assert!(
                    previous_end < range.offset,
                    "overlapping buffer allocations"
                );
            }
            merged.push(range);
        }
        self.free_ranges = merged;
    }

    fn is_empty(&self) -> bool {
        matches!(self.free_ranges.as_slice(), [range] if range.offset == 0 && range.size == self.size)
    }
}

/// Reclaimable native memory for distinct Vulkan buffer objects.  Vulkan
/// command offsets remain relative to their buffer, so suballocation does not
/// leak an implicit global buffer into the explicit GAL API.
struct DeviceMemoryAllocator {
    context: Arc<VulkanContext>,
    blocks: Vec<DeviceMemoryBlock>,
    next_block_id: u64,
}

/// Backend-private page allocator for explicit GAL resource sets.  The GAL
/// API still creates and destroys independently addressable resource sets;
/// only Vulkan's pool bookkeeping is shared.  Pages are keyed by their exact
/// per-set descriptor signature, which keeps capacity accounting explicit and
/// makes an empty page immediately reclaimable.
struct DescriptorPoolAllocator {
    context: Arc<VulkanContext>,
    blocks: Vec<DescriptorPoolBlock>,
    next_block_id: u64,
}

struct DescriptorPoolBlock {
    id: u64,
    pool: vk::DescriptorPool,
    signature: Vec<(vk::DescriptorType, u32)>,
    allocated_sets: u32,
}

impl DescriptorPoolAllocator {
    fn new(context: Arc<VulkanContext>) -> Self {
        Self {
            context,
            blocks: Vec::new(),
            next_block_id: 1,
        }
    }

    fn allocate(
        &mut self,
        signature: Vec<(vk::DescriptorType, u32)>,
        layout: vk::DescriptorSetLayout,
        label: &str,
    ) -> GalResult<(u64, vk::DescriptorPool, vk::DescriptorSet)> {
        let mut selected = self.blocks.iter().position(|block| {
            block.signature == signature && block.allocated_sets < DESCRIPTOR_POOL_PAGE_SET_COUNT
        });
        if selected.is_none() {
            let pool_sizes = signature
                .iter()
                .map(|(ty, count)| vk::DescriptorPoolSize {
                    ty: *ty,
                    descriptor_count: count.saturating_mul(DESCRIPTOR_POOL_PAGE_SET_COUNT),
                })
                .collect::<Vec<_>>();
            let pool_info = vk::DescriptorPoolCreateInfo::default()
                .flags(vk::DescriptorPoolCreateFlags::FREE_DESCRIPTOR_SET)
                .max_sets(DESCRIPTOR_POOL_PAGE_SET_COUNT)
                .pool_sizes(&pool_sizes);
            let pool = unsafe { self.context.device.create_descriptor_pool(&pool_info, None) }
                .map_err(|error| {
                    GalError::backend(format!(
                        "failed to create descriptor-pool page '{label}': {error:?}"
                    ))
                })?;
            self.context.set_object_name(pool, label);
            let id = self.next_block_id;
            self.next_block_id = self.next_block_id.checked_add(1).ok_or_else(|| {
                GalError::backend("Vulkan descriptor-pool block identifier overflow")
            })?;
            self.blocks.push(DescriptorPoolBlock {
                id,
                pool,
                signature,
                allocated_sets: 0,
            });
            selected = Some(self.blocks.len() - 1);
        }
        let block = &mut self.blocks[selected.expect("descriptor pool page selected")];
        let layouts = [layout];
        let allocate_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(block.pool)
            .set_layouts(&layouts);
        let set = unsafe { self.context.device.allocate_descriptor_sets(&allocate_info) }
            .map_err(|error| {
                GalError::backend(format!(
                    "failed to allocate descriptor set '{label}': {error:?}"
                ))
            })?
            .remove(0);
        block.allocated_sets = block.allocated_sets.checked_add(1).ok_or_else(|| {
            GalError::backend("Vulkan descriptor-pool allocation counter overflow")
        })?;
        Ok((block.id, block.pool, set))
    }

    fn release(&mut self, block_id: u64, pool: vk::DescriptorPool, set: vk::DescriptorSet) {
        let Some(index) = self.blocks.iter().position(|block| block.id == block_id) else {
            debug_assert!(false, "released unknown Vulkan descriptor-pool allocation");
            return;
        };
        let block = &mut self.blocks[index];
        debug_assert_eq!(block.pool, pool);
        unsafe {
            let _ = self.context.device.free_descriptor_sets(pool, &[set]);
        }
        block.allocated_sets = block.allocated_sets.saturating_sub(1);
        if block.allocated_sets == 0 {
            let block = self.blocks.remove(index);
            unsafe {
                self.context
                    .device
                    .destroy_descriptor_pool(block.pool, None)
            };
        }
    }

    fn destroy_all(&mut self) {
        for block in self.blocks.drain(..) {
            unsafe {
                self.context
                    .device
                    .destroy_descriptor_pool(block.pool, None)
            };
        }
    }
}

impl DeviceMemoryAllocator {
    fn new(context: Arc<VulkanContext>) -> Self {
        Self {
            context,
            blocks: Vec::new(),
            next_block_id: 1,
        }
    }

    fn allocate(
        &mut self,
        requirements: vk::MemoryRequirements,
        properties: vk::MemoryPropertyFlags,
        page_size: u64,
    ) -> GalResult<DeviceMemoryAllocation> {
        let memory_type_index = self
            .context
            .find_memory_type(requirements.memory_type_bits, properties)?;
        for block in &mut self.blocks {
            if block.memory_type_index != memory_type_index {
                continue;
            }
            if let Some(offset) = block.allocate(requirements.size, requirements.alignment) {
                return Ok(DeviceMemoryAllocation {
                    block_id: block.id,
                    memory: block.memory,
                    offset,
                    size: requirements.size,
                });
            }
        }

        let allocation_size = align_up(page_size.max(requirements.size), requirements.alignment)
            .ok_or_else(|| GalError::backend("Vulkan buffer allocation size overflow"))?;
        let allocate_info = vk::MemoryAllocateInfo::default()
            .allocation_size(allocation_size)
            .memory_type_index(memory_type_index);
        let memory = unsafe { self.context.device.allocate_memory(&allocate_info, None) }.map_err(
            |error| {
                GalError::backend(format!(
                    "failed to allocate Vulkan buffer memory page: {error:?}"
                ))
            },
        )?;
        let mut block = DeviceMemoryBlock {
            id: self.next_block_id,
            memory,
            memory_type_index,
            size: allocation_size,
            free_ranges: vec![BufferMemoryRange {
                offset: 0,
                size: allocation_size,
            }],
        };
        self.next_block_id = self.next_block_id.checked_add(1).ok_or_else(|| {
            GalError::backend("Vulkan buffer memory allocator block identifier overflow")
        })?;
        let offset = block
            .allocate(requirements.size, requirements.alignment)
            .expect("fresh Vulkan buffer memory page must fit requested allocation");
        let allocation = DeviceMemoryAllocation {
            block_id: block.id,
            memory,
            offset,
            size: requirements.size,
        };
        if std::env::var_os("MATTMC_TRACE_VULKAN_BUFFER_PAGES").is_some() {
            eprintln!(
                "vulkan.buffer-page.create id={} memory_type_index={} memory_property_bits={} host_visible={} page_bytes={} request_bytes={} alignment={}",
                block.id,
                memory_type_index,
                properties.as_raw(),
                properties.contains(vk::MemoryPropertyFlags::HOST_VISIBLE),
                allocation_size,
                requirements.size,
                requirements.alignment,
            );
        }
        self.blocks.push(block);
        Ok(allocation)
    }

    fn release(&mut self, allocation: DeviceMemoryAllocation) {
        let Some(index) = self
            .blocks
            .iter()
            .position(|block| block.id == allocation.block_id)
        else {
            debug_assert!(false, "released unknown Vulkan buffer memory allocation");
            return;
        };
        let (is_empty, memory_type_index) = {
            let block = &mut self.blocks[index];
            debug_assert_eq!(block.memory, allocation.memory);
            block.release(allocation);
            (block.is_empty(), block.memory_type_index)
        };
        let retained_empty_pages = self
            .blocks
            .iter()
            .enumerate()
            .filter(|(candidate_index, candidate)| {
                *candidate_index != index
                    && candidate.memory_type_index == memory_type_index
                    && candidate.is_empty()
            })
            .count();
        if is_empty && retained_empty_pages >= EMPTY_MEMORY_PAGES_RETAINED_PER_TYPE {
            let block = self.blocks.remove(index);
            unsafe { self.context.device.free_memory(block.memory, None) };
        }
    }

    /// Bytes reserved from Vulkan for this allocator's backend-private pages.
    /// This is deliberately separate from logical GAL resource counts: page
    /// retention and suballocation are observable memory policy, not hidden
    /// resource ownership.
    fn reserved_bytes(&self) -> u64 {
        self.blocks.iter().map(|block| block.size).sum()
    }

    /// Bytes currently occupied by live suballocations, excluding free ranges
    /// retained in otherwise reusable pages.
    fn allocated_bytes(&self) -> u64 {
        self.blocks
            .iter()
            .map(|block| {
                let free_bytes = block
                    .free_ranges
                    .iter()
                    .map(|range| range.size)
                    .sum::<u64>();
                block.size.saturating_sub(free_bytes)
            })
            .sum()
    }
}

impl Drop for DeviceMemoryAllocator {
    fn drop(&mut self) {
        // Logical GAL resources release suballocations through `release`, but
        // intentionally retained pages can still contain free ranges when the
        // backend shuts down. Free every remaining page while the Vulkan
        // device is still alive; otherwise validation reports leaked
        // VkDeviceMemory at vkDestroyDevice.
        for block in self.blocks.drain(..) {
            unsafe { self.context.device.free_memory(block.memory, None) };
        }
    }
}

fn align_up(value: u64, alignment: u64) -> Option<u64> {
    let alignment = alignment.max(1);
    let remainder = value % alignment;
    value.checked_add((alignment - remainder) % alignment)
}

pub(super) struct VulkanObjects {
    context: Arc<VulkanContext>,
    /// Vulkan buffers remain distinct explicit GAL resources.  Their native
    /// memory, however, is suballocated here so a terrain section cannot turn
    /// into a driver-sized `VkDeviceMemory` allocation of its own.
    buffer_memory: DeviceMemoryAllocator,
    /// Texture images use the same explicit-memory suballocation discipline
    /// as buffers, but never share pages with buffers: Vulkan image and buffer
    /// requirements are independently typed and may select different memory
    /// types or alignments.
    texture_memory: DeviceMemoryAllocator,
    descriptor_pools: DescriptorPoolAllocator,
    objects: BTreeMap<Handle, VulkanObject>,
    next_token: u64,
    /// Backend-private compiler output cache.  GAL shader handles remain
    /// distinct resources; only the expensive GLSL-to-SPIR-V translation is
    /// shared, and the bounded cache owns no Vulkan handles.
    compiled_shader_cache: HashMap<ShaderCompileKey, Vec<u8>>,
    /// Weak backend-private reuse of native immutable pipelines.  A logical
    /// GAL graphics-pipeline handle remains independently created, bound, and
    /// completion-retired; duplicate native work is the only thing shared.
    graphics_pipeline_cache: HashMap<GraphicsPipelineCacheKey, Weak<NativeGraphicsPipeline>>,
    native_objects_retired_since_allocator_trim: u32,
}

impl VulkanObjects {
    pub(super) fn new(context: Arc<VulkanContext>) -> Self {
        Self {
            buffer_memory: DeviceMemoryAllocator::new(context.clone()),
            texture_memory: DeviceMemoryAllocator::new(context.clone()),
            descriptor_pools: DescriptorPoolAllocator::new(context.clone()),
            context,
            objects: BTreeMap::new(),
            next_token: 1,
            compiled_shader_cache: HashMap::new(),
            graphics_pipeline_cache: HashMap::new(),
            native_objects_retired_since_allocator_trim: 0,
        }
    }

    pub(super) fn create(
        &mut self,
        handle: Handle,
        desc: BackendCreateDesc<'_>,
    ) -> GalResult<BackendToken> {
        let _zone = trace::Zone::new("vulkan.resources.create-native");
        let token = BackendToken(self.next_token);
        self.next_token += 1;
        let object = match desc {
            BackendCreateDesc::Buffer(desc) => {
                VulkanObject::Buffer(self.create_buffer(handle, desc, token)?)
            }
            BackendCreateDesc::Texture(desc) => {
                VulkanObject::Texture(self.create_texture(handle, desc, token)?)
            }
            BackendCreateDesc::TextureView(desc) => {
                VulkanObject::TextureView(self.create_texture_view(handle, desc, token)?)
            }
            BackendCreateDesc::Sampler(desc) => {
                VulkanObject::Sampler(self.create_sampler(handle, desc, token)?)
            }
            BackendCreateDesc::CombinedTextureSampler(desc) => {
                VulkanObject::CombinedTextureSampler(CombinedTextureSamplerObject {
                    token,
                    texture_view: desc.texture_view,
                    sampler: desc.sampler,
                })
            }
            BackendCreateDesc::ShaderModule(desc) => {
                VulkanObject::ShaderModule(self.create_shader_module(handle, desc, token)?)
            }
            BackendCreateDesc::ResourceLayout(desc) => {
                VulkanObject::ResourceLayout(self.create_resource_layout(handle, desc, token)?)
            }
            BackendCreateDesc::ResourceSet(desc) => {
                VulkanObject::ResourceSet(self.create_resource_set(handle, desc, token)?)
            }
            BackendCreateDesc::PipelineLayout(desc) => {
                VulkanObject::PipelineLayout(self.create_pipeline_layout(handle, desc, token)?)
            }
            BackendCreateDesc::GraphicsPipeline(desc) => {
                VulkanObject::GraphicsPipeline(self.create_graphics_pipeline(handle, desc, token)?)
            }
            BackendCreateDesc::ComputePipeline(desc) => {
                VulkanObject::ComputePipeline(self.create_compute_pipeline(handle, desc, token)?)
            }
            BackendCreateDesc::RenderTarget(desc) => {
                self.validate_render_target_desc(desc)?;
                VulkanObject::RenderTarget(RenderTargetObject {
                    token,
                    color_views: desc.color_views.clone(),
                    depth_stencil_view: desc.depth_stencil_view,
                    extent: desc.extent,
                })
            }
            BackendCreateDesc::FrameTarget(desc) => VulkanObject::FrameTarget(FrameTargetObject {
                token,
                frame_id: desc.frame_id,
                render_target: desc.render_target,
                extent: desc.extent,
                color_format: desc.color_format,
                image_index: u32::MAX,
                image: vk::Image::null(),
                image_view: vk::ImageView::null(),
                image_layout: vk::ImageLayout::UNDEFINED,
            }),
            BackendCreateDesc::RenderPass(desc) => {
                self.validate_render_pass_desc(desc)?;
                VulkanObject::RenderPass(RenderPassObject {
                    token,
                    label: desc.label.clone(),
                    target: desc.target,
                    color_formats: desc.color_formats.clone(),
                    depth_format: desc.depth_format,
                })
            }
        };
        self.objects.insert(handle, object);
        Ok(token)
    }

    pub(super) fn create_frame_target_from_swapchain(
        &mut self,
        handle: Handle,
        desc: &FrameTargetDesc,
        make_object: impl FnOnce(BackendToken) -> GalResult<FrameTargetObject>,
    ) -> GalResult<BackendToken> {
        let _zone = trace::Zone::new("vulkan.resources.create-frame-target");
        let token = BackendToken(self.next_token);
        self.next_token += 1;
        let object = make_object(token)?;
        if object.render_target != desc.render_target
            || object.extent != desc.extent
            || object.color_format != desc.color_format
        {
            return Err(GalError::backend(
                "swapchain frame target metadata does not match GAL frame target",
            ));
        }
        self.objects
            .insert(handle, VulkanObject::FrameTarget(object));
        Ok(token)
    }

    pub(super) fn refresh_frame_target_from_swapchain(
        &mut self,
        handle: Handle,
        make_object: impl FnOnce(
            BackendToken,
            crate::render::vulkanic::frame::FrameRenderTargetId,
            Extent3d,
            TextureFormat,
        ) -> GalResult<FrameTargetObject>,
    ) -> GalResult<()> {
        let Some(VulkanObject::FrameTarget(existing)) = self.objects.get(&handle) else {
            return Err(GalError::backend(
                "Vulkan frame target refresh for unknown frame target handle",
            ));
        };
        let token = existing.token;
        let render_target = existing.render_target;
        let extent = existing.extent;
        let color_format = existing.color_format;
        let object = make_object(token, render_target, extent, color_format)?;
        if object.token != token
            || object.render_target != render_target
            || object.extent != extent
            || object.color_format != color_format
        {
            return Err(GalError::backend(
                "refreshed swapchain frame target metadata does not match GAL frame target",
            ));
        }
        self.objects
            .insert(handle, VulkanObject::FrameTarget(object));
        Ok(())
    }

    pub(super) fn destroy(
        &mut self,
        handle: Handle,
        kind: HandleKind,
        token: BackendToken,
    ) -> GalResult<()> {
        let Some(object) = self.objects.remove(&handle) else {
            return Err(GalError::backend("Vulkan destroy for unknown handle"));
        };
        if object.kind() != kind || object.token() != token {
            self.objects.insert(handle, object);
            return Err(GalError::backend("Vulkan destroy kind or token mismatch"));
        }
        self.destroy_object(object);
        self.trim_native_allocator_after_retirement();
        Ok(())
    }

    /// On Linux this returns free glibc heap pages after an explicit Vulkan
    /// resource retirement. The call is deliberately unavailable to GAL and
    /// other backends: object lifetime remains controlled solely by the GAL
    /// completion-aware destroy path above.
    fn trim_native_allocator_after_retirement(&mut self) {
        self.native_objects_retired_since_allocator_trim = self
            .native_objects_retired_since_allocator_trim
            .saturating_add(1);
        #[cfg(target_os = "linux")]
        {
            let info = unsafe { libc::mallinfo2() };
            if should_trim_native_allocator(
                self.native_objects_retired_since_allocator_trim,
                info.fordblks,
            ) {
                let released = unsafe { libc::malloc_trim(0) } != 0;
                if std::env::var_os("MATTMC_TRACE_GLIBC_ALLOCATOR").is_some() {
                    eprintln!(
                        "vulkan.glibc-allocator-trim retired={} free_bytes={} released={}",
                        self.native_objects_retired_since_allocator_trim, info.fordblks, released,
                    );
                }
                self.native_objects_retired_since_allocator_trim = 0;
            }
        }
    }

    pub(super) fn buffer(&self, handle: Handle) -> GalResult<&BufferObject> {
        match self.objects.get(&handle) {
            Some(VulkanObject::Buffer(object)) => Ok(object),
            _ => Err(GalError::backend(format!(
                "expected Vulkan buffer for handle 0x{:016x}",
                handle.raw()
            ))),
        }
    }

    pub(super) fn texture(&self, handle: Handle) -> GalResult<&TextureObject> {
        match self.objects.get(&handle) {
            Some(VulkanObject::Texture(object)) => Ok(object),
            _ => Err(GalError::backend(format!(
                "expected Vulkan texture for handle 0x{:016x}",
                handle.raw()
            ))),
        }
    }

    pub(super) fn texture_view(&self, handle: Handle) -> GalResult<&TextureViewObject> {
        match self.objects.get(&handle) {
            Some(VulkanObject::TextureView(object)) => Ok(object),
            _ => Err(GalError::backend(format!(
                "expected Vulkan texture view for handle 0x{:016x}",
                handle.raw()
            ))),
        }
    }

    pub(super) fn resource_set(&self, handle: Handle) -> GalResult<&ResourceSetObject> {
        match self.objects.get(&handle) {
            Some(VulkanObject::ResourceSet(object)) => Ok(object),
            _ => Err(GalError::backend(format!(
                "expected Vulkan resource set for handle 0x{:016x}",
                handle.raw()
            ))),
        }
    }

    pub(super) fn pipeline_layout(&self, handle: Handle) -> GalResult<&PipelineLayoutObject> {
        match self.objects.get(&handle) {
            Some(VulkanObject::PipelineLayout(object)) => Ok(object),
            _ => Err(GalError::backend(format!(
                "expected Vulkan pipeline layout for handle 0x{:016x}",
                handle.raw()
            ))),
        }
    }

    pub(super) fn graphics_pipeline(&self, handle: Handle) -> GalResult<&GraphicsPipelineObject> {
        match self.objects.get(&handle) {
            Some(VulkanObject::GraphicsPipeline(object)) => Ok(object),
            _ => Err(GalError::backend(format!(
                "expected Vulkan graphics pipeline for handle 0x{:016x}",
                handle.raw()
            ))),
        }
    }

    pub(super) fn compute_pipeline(&self, handle: Handle) -> GalResult<&ComputePipelineObject> {
        match self.objects.get(&handle) {
            Some(VulkanObject::ComputePipeline(object)) => Ok(object),
            _ => Err(GalError::backend(format!(
                "expected Vulkan compute pipeline for handle 0x{:016x}",
                handle.raw()
            ))),
        }
    }

    pub(super) fn render_target(&self, handle: Handle) -> GalResult<&RenderTargetObject> {
        match self.objects.get(&handle) {
            Some(VulkanObject::RenderTarget(object)) => Ok(object),
            _ => Err(GalError::backend(format!(
                "expected Vulkan render target for handle 0x{:016x}",
                handle.raw()
            ))),
        }
    }

    pub(super) fn frame_target(&self, handle: Handle) -> GalResult<&FrameTargetObject> {
        match self.objects.get(&handle) {
            Some(VulkanObject::FrameTarget(object)) => Ok(object),
            _ => Err(GalError::backend(format!(
                "expected Vulkan frame target for handle 0x{:016x}",
                handle.raw()
            ))),
        }
    }

    pub(super) fn render_pass(&self, handle: Handle) -> GalResult<&RenderPassObject> {
        match self.objects.get(&handle) {
            Some(VulkanObject::RenderPass(object)) => Ok(object),
            _ => Err(GalError::backend(format!(
                "expected Vulkan render pass for handle 0x{:016x}",
                handle.raw()
            ))),
        }
    }

    pub(super) fn destroy_all(&mut self) {
        let objects = std::mem::take(&mut self.objects);
        for (_, object) in objects.into_iter().rev() {
            self.destroy_object(object);
        }
        self.graphics_pipeline_cache.clear();
        // Normally every resource-set destruction releases its page.  Keep a
        // final backend teardown sweep for partial native initialization paths
        // where no logical resource object was installed.
        self.descriptor_pools.destroy_all();
    }

    /// Diagnostic-only accounting for backend-private Vulkan memory pages.
    /// Logical GAL lifetimes remain authoritative; this reports both live
    /// suballocations and retained page capacity without changing policy.
    fn trace_memory_residency(&self, label: &str) {
        if std::env::var_os("MATTMC_TRACE_VULKAN_RESIDENCY").is_none() {
            return;
        }
        // Resource retirement is frequent during ordinary GUI and terrain
        // streaming. Sample it so this opt-in diagnostic cannot itself become
        // an unbounded log producer; creates remain exact and every 1,024th
        // retirement still reports allocator high-water behavior.
        if label == "destroy"
            && VULKAN_RESIDENCY_TRACE_SEQUENCE.fetch_add(1, Ordering::Relaxed) % 1024 != 0
        {
            return;
        }
        eprintln!(
            "vulkan.residency label={} buffer_reserved_bytes={} buffer_allocated_bytes={} buffer_pages={} texture_reserved_bytes={} texture_allocated_bytes={} texture_pages={} logical_objects={}",
            label,
            self.buffer_memory.reserved_bytes(),
            self.buffer_memory.allocated_bytes(),
            self.buffer_memory.blocks.len(),
            self.texture_memory.reserved_bytes(),
            self.texture_memory.allocated_bytes(),
            self.texture_memory.blocks.len(),
            self.objects.len(),
        );
    }

    fn create_buffer(
        &mut self,
        handle: Handle,
        desc: &BufferDesc,
        token: BackendToken,
    ) -> GalResult<BufferObject> {
        let usage = buffer_usage_flags(&desc.usages);
        let create_info = vk::BufferCreateInfo::default()
            .size(desc.size)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let buffer =
            unsafe { self.context.device.create_buffer(&create_info, None) }.map_err(|error| {
                GalError::backend(format!(
                    "failed to create buffer '{}': {error:?}",
                    desc.label
                ))
            })?;
        let requirements = unsafe { self.context.device.get_buffer_memory_requirements(buffer) };
        if std::env::var_os("MATTMC_TRACE_VULKAN_BUFFER_OBJECTS").is_some() {
            eprintln!(
                "vulkan.buffer.create label={} requested_bytes={} requirement_bytes={} alignment={} memory={:?}",
                desc.label, desc.size, requirements.size, requirements.alignment, desc.memory,
            );
        }
        let allocation = match self.buffer_memory.allocate(
            requirements,
            memory_flags(desc.memory),
            if matches!(desc.memory, MemoryDomain::Upload | MemoryDomain::Readback) {
                HOST_VISIBLE_BUFFER_PAGE_SIZE
            } else {
                DEVICE_LOCAL_BUFFER_PAGE_SIZE
            },
        ) {
            Ok(allocation) => allocation,
            Err(error) => {
                unsafe { self.context.device.destroy_buffer(buffer, None) };
                return Err(error);
            }
        };
        if let Err(error) = unsafe {
            self.context
                .device
                .bind_buffer_memory(buffer, allocation.memory, allocation.offset)
        } {
            unsafe {
                self.context.device.destroy_buffer(buffer, None);
            }
            self.buffer_memory.release(allocation);
            return Err(GalError::backend(format!(
                "failed to bind buffer memory '{}': {error:?}",
                desc.label
            )));
        }
        self.context
            .set_object_name(buffer, &debug_name("buffer", handle, &desc.label));
        trace_glibc_allocator_checkpoint(&desc.label);
        self.trace_memory_residency(&format!("buffer.create.{}", desc.label));
        Ok(BufferObject {
            token,
            buffer,
            memory: allocation.memory,
            memory_offset: allocation.offset,
            allocation,
            size: desc.size,
            memory_domain: desc.memory,
        })
    }

    fn create_texture(
        &mut self,
        handle: Handle,
        desc: &TextureDesc,
        token: BackendToken,
    ) -> GalResult<TextureObject> {
        if !matches!(desc.dimension, TextureDimension::D2 | TextureDimension::D3) {
            return Err(GalError::backend(
                "Vulkan backend currently supports D2 and D3 textures in the isolated path",
            ));
        }
        let format = texture_format(desc.format);
        let usage = texture_usage_flags(&desc.usages);
        let image_type = match desc.dimension {
            TextureDimension::D2 => vk::ImageType::TYPE_2D,
            TextureDimension::D3 => vk::ImageType::TYPE_3D,
            _ => unreachable!("GAL validated supported texture dimension"),
        };
        let properties = unsafe {
            self.context
                .instance
                .get_physical_device_image_format_properties(
                    self.context.physical_device,
                    format,
                    image_type,
                    vk::ImageTiling::OPTIMAL,
                    usage,
                    vk::ImageCreateFlags::empty(),
                )
        }
        .map_err(|error| {
            if error == vk::Result::ERROR_FORMAT_NOT_SUPPORTED {
                GalError::unsupported_feature(format!(
                    "Vulkan device does not support {:?} format {:?} with requested usages {:?}",
                    desc.dimension, desc.format, desc.usages
                ))
            } else {
                GalError::backend(format!(
                    "failed to query Vulkan image format support for '{}': {error:?}",
                    desc.label
                ))
            }
        })?;
        if desc.dimension == TextureDimension::D3 {
            validate_d3_image_format_properties(desc, properties)?;
        }
        let create_info = vk::ImageCreateInfo::default()
            .image_type(image_type)
            .format(format)
            .extent(vk::Extent3D {
                width: desc.extent.width,
                height: desc.extent.height,
                depth: desc.extent.depth,
            })
            .mip_levels(desc.mip_levels)
            .array_layers(desc.array_layers)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        let image =
            unsafe { self.context.device.create_image(&create_info, None) }.map_err(|error| {
                GalError::backend(format!(
                    "failed to create image '{}': {error:?}",
                    desc.label
                ))
            })?;
        let requirements = unsafe { self.context.device.get_image_memory_requirements(image) };
        if std::env::var_os("MATTMC_TRACE_VULKAN_IMAGE_ALLOCATIONS").is_some() {
            eprintln!(
                "vulkan.image.allocate label={} dimension={:?} format={:?} extent={}x{}x{} mip_levels={} array_layers={} bytes={} alignment={}",
                desc.label,
                desc.dimension,
                desc.format,
                desc.extent.width,
                desc.extent.height,
                desc.extent.depth,
                desc.mip_levels,
                desc.array_layers,
                requirements.size,
                requirements.alignment,
            );
        }
        let allocation = match self.texture_memory.allocate(
            requirements,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
            DEVICE_LOCAL_TEXTURE_PAGE_SIZE,
        ) {
            Ok(allocation) => allocation,
            Err(error) => {
                unsafe { self.context.device.destroy_image(image, None) };
                return Err(error);
            }
        };
        if let Err(error) = unsafe {
            self.context
                .device
                .bind_image_memory(image, allocation.memory, allocation.offset)
        } {
            unsafe {
                self.context.device.destroy_image(image, None);
            }
            self.texture_memory.release(allocation);
            return Err(GalError::backend(format!(
                "failed to bind image memory '{}': {error:?}",
                desc.label
            )));
        }
        self.context
            .set_object_name(image, &debug_name("texture", handle, &desc.label));
        trace_glibc_allocator_checkpoint(&desc.label);
        self.trace_memory_residency(&format!("texture.create.{}", desc.label));
        Ok(TextureObject {
            token,
            label: desc.label.clone(),
            image,
            memory: allocation.memory,
            allocation,
            format,
            copy_bytes_per_texel: desc.format.copy_bytes_per_texel().unwrap_or(0),
            extent: desc.extent,
            dimension: desc.dimension,
            mip_levels: desc.mip_levels,
            array_layers: desc.array_layers,
            aspect: aspect_for_format(desc.format),
        })
    }

    fn create_texture_view(
        &self,
        handle: Handle,
        desc: &TextureViewDesc,
        token: BackendToken,
    ) -> GalResult<TextureViewObject> {
        let texture = self.texture(desc.texture)?;
        let create_info = vk::ImageViewCreateInfo::default()
            .image(texture.image)
            .view_type(image_view_type(texture.dimension, texture.array_layers))
            .format(texture_format(desc.format))
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: texture.aspect,
                base_mip_level: desc.base_mip,
                level_count: desc.mip_count,
                base_array_layer: desc.base_layer,
                layer_count: desc.layer_count,
            });
        let view = unsafe { self.context.device.create_image_view(&create_info, None) }.map_err(
            |error| {
                GalError::backend(format!(
                    "failed to create image view '{}': {error:?}",
                    desc.label
                ))
            },
        )?;
        self.context
            .set_object_name(view, &debug_name("texture-view", handle, &desc.label));
        Ok(TextureViewObject {
            token,
            label: desc.label.clone(),
            view,
            texture: desc.texture,
            format: texture_format(desc.format),
            aspect: texture.aspect,
            base_mip: desc.base_mip,
            mip_levels: desc.mip_count,
            base_layer: desc.base_layer,
            array_layers: desc.layer_count,
        })
    }

    fn validate_render_target_desc(&self, desc: &RenderTargetDesc) -> GalResult<()> {
        for (index, view_handle) in desc.color_views.iter().enumerate() {
            let view = self.texture_view(*view_handle).map_err(|_| {
                GalError::backend(format!(
                    "render target '{}' color attachment {} is not a texture view",
                    desc.label, index
                ))
            })?;
            let texture = self.texture(view.texture)?;
            if texture.extent != desc.extent {
                return Err(GalError::invalid_argument(format!(
                    "render target '{}' color attachment {} extent {:?} does not match target {:?}",
                    desc.label, index, texture.extent, desc.extent
                )));
            }
            if !view.aspect.contains(vk::ImageAspectFlags::COLOR) {
                return Err(GalError::invalid_argument(format!(
                    "render target '{}' color attachment {} is not a color view",
                    desc.label, index
                )));
            }
        }
        if let Some(depth_handle) = desc.depth_stencil_view {
            let view = self.texture_view(depth_handle).map_err(|_| {
                GalError::backend(format!(
                    "render target '{}' depth attachment is not a texture view",
                    desc.label
                ))
            })?;
            let texture = self.texture(view.texture)?;
            if texture.extent != desc.extent {
                return Err(GalError::invalid_argument(format!(
                    "render target '{}' depth attachment extent {:?} does not match target {:?}",
                    desc.label, texture.extent, desc.extent
                )));
            }
            if !view.aspect.contains(vk::ImageAspectFlags::DEPTH) {
                return Err(GalError::invalid_argument(format!(
                    "render target '{}' depth attachment is not a depth view",
                    desc.label
                )));
            }
        }
        Ok(())
    }

    fn validate_render_pass_desc(&self, desc: &RenderPassDesc) -> GalResult<()> {
        let Some(VulkanObject::RenderTarget(target)) = self.objects.get(&desc.target) else {
            // Swapchain-backed frame targets have a single dynamic color view;
            // their compatibility is checked when the frame target is refreshed.
            return Ok(());
        };
        if target.color_views.len() != desc.color_formats.len() {
            return Err(GalError::invalid_argument(format!(
                "render pass '{}' declares {} color formats for {} target views",
                desc.label,
                desc.color_formats.len(),
                target.color_views.len()
            )));
        }
        if target.depth_stencil_view.is_none() && desc.depth_format.is_some() {
            return Err(GalError::invalid_argument(format!(
                "render pass '{}' depth attachment presence does not match target",
                desc.label
            )));
        }
        Ok(())
    }

    fn create_sampler(
        &self,
        handle: Handle,
        desc: &SamplerDesc,
        token: BackendToken,
    ) -> GalResult<SamplerObject> {
        let create_info = vk::SamplerCreateInfo::default()
            .mag_filter(filter(desc.mag_filter))
            .min_filter(filter(desc.min_filter))
            .mipmap_mode(mipmap_filter(desc.mip_filter))
            .address_mode_u(address_mode(desc.address_u))
            .address_mode_v(address_mode(desc.address_v))
            .address_mode_w(address_mode(desc.address_w))
            .compare_enable(desc.comparison.is_some())
            .compare_op(
                desc.comparison
                    .map(compare_op)
                    .unwrap_or(vk::CompareOp::ALWAYS),
            )
            .max_lod(vk::LOD_CLAMP_NONE);
        let sampler =
            unsafe { self.context.device.create_sampler(&create_info, None) }.map_err(|error| {
                GalError::backend(format!(
                    "failed to create sampler '{}': {error:?}",
                    desc.label
                ))
            })?;
        self.context
            .set_object_name(sampler, &debug_name("sampler", handle, &desc.label));
        Ok(SamplerObject { token, sampler })
    }

    fn create_shader_module(
        &mut self,
        handle: Handle,
        desc: &ShaderModuleDesc,
        token: BackendToken,
    ) -> GalResult<ShaderModuleObject> {
        trace_glibc_allocator_checkpoint(&format!("shader.begin.{}", desc.label));
        let code = if desc.code_format == ShaderCodeFormat::Glsl {
            let source = std::str::from_utf8(&desc.code).map_err(|error| {
                GalError::backend(format!(
                    "GLSL shader '{}' is not UTF-8: {error}",
                    desc.label
                ))
            })?;
            let cache_key = ShaderCompileKey {
                stage: desc.stage,
                entry_point: desc.entry_point.clone(),
                code: desc.code.clone(),
            };
            if let Some(cached) = self.compiled_shader_cache.get(&cache_key) {
                if std::env::var_os("MATTMC_TRACE_VK_SHADER_COMPILE").is_some() {
                    println!("vulkan.shader.compile.cache_hit label={}", desc.label);
                }
                cached.clone()
            } else {
                if std::env::var_os("MATTMC_TRACE_VK_SHADER_COMPILE").is_some() {
                    println!(
                        "vulkan.shader.compile.begin label={} stage={:?} bytes={}",
                        desc.label,
                        desc.stage,
                        source.len()
                    );
                }
                let compiled = compile_glsl_for_backend(
                    shaderc_kind(desc.stage)?,
                    source,
                    &desc.label,
                    &desc.entry_point,
                )
                .map_err(|error| {
                    GalError::backend(format!(
                        "failed to compile GLSL shader '{}' for Vulkan backend: {error}",
                        desc.label
                    ))
                })?;
                if self.compiled_shader_cache.len() >= MAX_COMPILED_SHADER_CACHE_ENTRIES {
                    self.compiled_shader_cache.clear();
                }
                self.compiled_shader_cache
                    .insert(cache_key, compiled.clone());
                compiled
            }
        } else {
            desc.code.clone()
        };
        if desc.code_format == ShaderCodeFormat::Glsl
            && std::env::var_os("MATTMC_TRACE_VK_SHADER_COMPILE").is_some()
        {
            println!("vulkan.shader.compile.end label={}", desc.label);
        }
        if desc.code_format != ShaderCodeFormat::Spirv && desc.code_format != ShaderCodeFormat::Glsl
            || code.len() % 4 != 0
        {
            return Err(GalError::backend(
                "Vulkan backend requires 4-byte aligned SPIR-V shader code",
            ));
        }
        let words = ash::util::read_spv(&mut Cursor::new(&code)).map_err(|error| {
            GalError::backend(format!("failed to read SPIR-V '{}': {error}", desc.label))
        })?;
        let create_info = vk::ShaderModuleCreateInfo::default().code(&words);
        let module = unsafe { self.context.device.create_shader_module(&create_info, None) }
            .map_err(|error| {
                GalError::backend(format!(
                    "failed to create shader module '{}': {error:?}",
                    desc.label
                ))
            })?;
        self.context
            .set_object_name(module, &debug_name("shader", handle, &desc.label));
        trace_glibc_allocator_checkpoint(&format!("shader.end.{}", desc.label));
        let entry_point = CString::new(desc.entry_point.clone())
            .map_err(|_| GalError::backend("shader entry point contains NUL"))?;
        Ok(ShaderModuleObject {
            token,
            module,
            stage: shader_stage(desc.stage),
            entry_point,
        })
    }

    fn create_resource_layout(
        &self,
        handle: Handle,
        desc: &ResourceLayoutDesc,
        token: BackendToken,
    ) -> GalResult<ResourceLayoutObject> {
        let bindings = desc
            .bindings
            .iter()
            .map(|binding| {
                vk::DescriptorSetLayoutBinding::default()
                    .binding(binding.binding)
                    .descriptor_type(descriptor_type_for_binding(
                        binding.kind,
                        binding.dynamic_offset_count,
                    ))
                    .descriptor_count(binding.array_count)
                    .stage_flags(shader_stage_flags(binding.stages))
            })
            .collect::<Vec<_>>();
        let create_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
        let layout = unsafe {
            self.context
                .device
                .create_descriptor_set_layout(&create_info, None)
        }
        .map_err(|error| {
            GalError::backend(format!(
                "failed to create descriptor set layout '{}': {error:?}",
                desc.label
            ))
        })?;
        self.context
            .set_object_name(layout, &debug_name("resource-layout", handle, &desc.label));
        Ok(ResourceLayoutObject {
            token,
            layout,
            bindings: desc.bindings.clone(),
        })
    }

    fn create_resource_set(
        &mut self,
        handle: Handle,
        desc: &ResourceSetDesc,
        token: BackendToken,
    ) -> GalResult<ResourceSetObject> {
        let layout_object = match self.objects.get(&desc.layout) {
            Some(VulkanObject::ResourceLayout(layout)) => layout,
            _ => return Err(GalError::backend("resource set references missing layout")),
        };
        let mut sizes: BTreeMap<vk::DescriptorType, u32> = BTreeMap::new();
        for binding in &layout_object.bindings {
            *sizes
                .entry(descriptor_type_for_binding(
                    binding.kind,
                    binding.dynamic_offset_count,
                ))
                .or_default() += binding.array_count;
        }
        let signature = sizes.into_iter().collect::<Vec<_>>();

        enum WritePlan {
            Buffer {
                binding: u32,
                array_index: u32,
                ty: vk::DescriptorType,
                info_index: usize,
            },
            Image {
                binding: u32,
                array_index: u32,
                ty: vk::DescriptorType,
                info_index: usize,
            },
        }
        let mut buffer_infos = Vec::new();
        let mut image_infos = Vec::new();
        let mut plans = Vec::new();
        for binding in &desc.bindings {
            match binding.kind {
                ResourceBindingKind::UniformBuffer | ResourceBindingKind::StorageBuffer => {
                    let buffer = self.buffer(binding.resource)?;
                    let info_index = buffer_infos.len();
                    let range = if let Some(range) = binding.buffer_range {
                        range
                    } else if binding.dynamic_offsets.is_empty() {
                        buffer.size
                    } else {
                        let max_default_offset =
                            binding.dynamic_offsets.iter().copied().max().unwrap_or(0);
                        buffer.size.saturating_sub(max_default_offset)
                    };
                    buffer_infos.push(vk::DescriptorBufferInfo {
                        buffer: buffer.buffer,
                        offset: 0,
                        range,
                    });
                    plans.push(WritePlan::Buffer {
                        binding: binding.binding,
                        array_index: binding.array_index,
                        ty: descriptor_type_for_resource_binding(binding),
                        info_index,
                    });
                }
                ResourceBindingKind::SampledTexture | ResourceBindingKind::StorageTexture => {
                    let view = self.texture_view(binding.resource)?;
                    let info_index = image_infos.len();
                    // A combined sampler may refer to a depth-only view (the
                    // source `depthtex*` contract) or a D24S8 view.  Vulkan
                    // requires the image layout to match the selected
                    // aspect; advertising a color SHADER_READ_ONLY layout
                    // for D32 caused depth samples to remain effectively
                    // far-depth/undefined in shader-pack fullscreen passes.
                    let sampled_layout = sampled_image_layout_for_aspect(view.aspect);
                    image_infos.push(vk::DescriptorImageInfo {
                        sampler: vk::Sampler::null(),
                        image_view: view.view,
                        image_layout: if binding.kind == ResourceBindingKind::StorageTexture {
                            vk::ImageLayout::GENERAL
                        } else {
                            sampled_layout
                        },
                    });
                    plans.push(WritePlan::Image {
                        binding: binding.binding,
                        array_index: binding.array_index,
                        ty: descriptor_type_for_resource_binding(binding),
                        info_index,
                    });
                }
                ResourceBindingKind::Sampler => {
                    let sampler = match self.objects.get(&binding.resource) {
                        Some(VulkanObject::Sampler(sampler)) => sampler.sampler,
                        _ => {
                            return Err(GalError::backend(
                                "sampler binding references missing sampler",
                            ));
                        }
                    };
                    let info_index = image_infos.len();
                    image_infos.push(vk::DescriptorImageInfo {
                        sampler,
                        image_view: vk::ImageView::null(),
                        image_layout: vk::ImageLayout::UNDEFINED,
                    });
                    plans.push(WritePlan::Image {
                        binding: binding.binding,
                        array_index: binding.array_index,
                        ty: vk::DescriptorType::SAMPLER,
                        info_index,
                    });
                }
                ResourceBindingKind::CombinedTextureSampler => {
                    let combined = match self.objects.get(&binding.resource) {
                        Some(VulkanObject::CombinedTextureSampler(combined)) => combined,
                        _ => {
                            return Err(GalError::backend(
                                "combined texture-sampler binding references missing pair",
                            ));
                        }
                    };
                    let view = self.texture_view(combined.texture_view)?;
                    let sampler = match self.objects.get(&combined.sampler) {
                        Some(VulkanObject::Sampler(sampler)) => sampler.sampler,
                        _ => {
                            return Err(GalError::backend(
                                "combined texture-sampler binding references missing sampler",
                            ));
                        }
                    };
                    let info_index = image_infos.len();
                    let sampled_layout = sampled_image_layout_for_aspect(view.aspect);
                    if std::env::var_os("MATTMC_TRACE_DESCRIPTOR_REALIZATION").is_some() {
                        eprintln!(
                            "vulkan.descriptor.combined resource={:?} texture_view={:?} texture={:?} label={} image_view={:?} sampler={:?} layout={}",
                            binding.resource,
                            combined.texture_view,
                            view.texture,
                            view.label,
                            view.view,
                            sampler,
                            sampled_layout.as_raw(),
                        );
                    }
                    image_infos.push(vk::DescriptorImageInfo {
                        sampler,
                        image_view: view.view,
                        image_layout: sampled_layout,
                    });
                    plans.push(WritePlan::Image {
                        binding: binding.binding,
                        array_index: binding.array_index,
                        ty: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
                        info_index,
                    });
                }
            }
        }
        let dynamic_offsets = desc
            .bindings
            .iter()
            .flat_map(|binding| binding.dynamic_offsets.iter().copied())
            .map(|offset| {
                u32::try_from(offset)
                    .map_err(|_| GalError::backend("dynamic descriptor offset exceeds u32"))
            })
            .collect::<GalResult<Vec<_>>>()?;
        // Resource lookup and dynamic-offset validation can fail above.
        // Allocate only after that work so an invalid GAL request cannot
        // strand a native descriptor set/page allocation.
        let (pool_block_id, pool, set) = self.descriptor_pools.allocate(
            signature,
            layout_object.layout,
            &debug_name("resource-set-pool-page", handle, &desc.label),
        )?;
        let writes = plans
            .iter()
            .map(|plan| match *plan {
                WritePlan::Buffer {
                    binding,
                    array_index,
                    ty,
                    info_index,
                } => vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(binding)
                    .dst_array_element(array_index)
                    .descriptor_type(ty)
                    .buffer_info(&buffer_infos[info_index..info_index + 1]),
                WritePlan::Image {
                    binding,
                    array_index,
                    ty,
                    info_index,
                } => vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(binding)
                    .dst_array_element(array_index)
                    .descriptor_type(ty)
                    .image_info(&image_infos[info_index..info_index + 1]),
            })
            .collect::<Vec<_>>();
        unsafe { self.context.device.update_descriptor_sets(&writes, &[]) };
        Ok(ResourceSetObject {
            token,
            pool_block_id,
            pool,
            set,
            dynamic_offsets,
        })
    }

    fn create_pipeline_layout(
        &self,
        handle: Handle,
        desc: &PipelineLayoutDesc,
        token: BackendToken,
    ) -> GalResult<PipelineLayoutObject> {
        let set_layouts = desc
            .resource_layouts
            .iter()
            .map(|handle| match self.objects.get(handle) {
                Some(VulkanObject::ResourceLayout(layout)) => Ok(layout.layout),
                _ => Err(GalError::backend(
                    "pipeline layout references missing resource layout",
                )),
            })
            .collect::<GalResult<Vec<_>>>()?;
        let create_info = vk::PipelineLayoutCreateInfo::default().set_layouts(&set_layouts);
        let layout = unsafe {
            self.context
                .device
                .create_pipeline_layout(&create_info, None)
        }
        .map_err(|error| {
            GalError::backend(format!(
                "failed to create pipeline layout '{}': {error:?}",
                desc.label
            ))
        })?;
        self.context
            .set_object_name(layout, &debug_name("pipeline-layout", handle, &desc.label));
        Ok(PipelineLayoutObject { token, layout })
    }

    fn create_graphics_pipeline(
        &mut self,
        handle: Handle,
        desc: &GraphicsPipelineDesc,
        token: BackendToken,
    ) -> GalResult<GraphicsPipelineObject> {
        let cache_key = GraphicsPipelineCacheKey::from_desc(desc);
        if let Some(pipeline) = self
            .graphics_pipeline_cache
            .get(&cache_key)
            .and_then(Weak::upgrade)
        {
            trace_native_graphics_pipeline_cache(
                true,
                &desc.label,
                self.graphics_pipeline_cache.len(),
            );
            return Ok(GraphicsPipelineObject {
                token,
                label: desc.label.clone(),
                pipeline,
                layout: desc.layout,
            });
        }
        trace_glibc_allocator_checkpoint(&format!("graphics-pipeline.begin.{}", desc.label));
        let vertex = self.shader(desc.vertex_shader)?;
        let fragment = self.shader(desc.fragment_shader)?;
        let layout = self.pipeline_layout(desc.layout)?.layout;
        let stages = [shader_stage_create(vertex), shader_stage_create(fragment)];
        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default();
        let input_assembly =
            vk::PipelineInputAssemblyStateCreateInfo::default().topology(topology(desc.topology));
        let viewport_state = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);
        let depth_bias = desc.depth_bias;
        let rasterization = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .line_width(1.0)
            .cull_mode(cull_mode(desc.cull_mode))
            .front_face(front_face(desc.front_face))
            .depth_bias_enable(depth_bias.is_some())
            .depth_bias_constant_factor(depth_bias.map_or(0.0, |bias| bias.constant_factor))
            .depth_bias_slope_factor(depth_bias.map_or(0.0, |bias| bias.slope_factor));
        let multisample = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        let color_blend_attachments = desc
            .color_formats
            .iter()
            .enumerate()
            .map(|(index, _)| color_blend_attachment(desc.blend, index))
            .collect::<Vec<_>>();
        if desc.blend == BlendMode::TerrainTranslucent
            && desc.color_formats.len() > 1
            && !self.context.independent_blend
        {
            return Err(GalError::unsupported_feature(
                "terrain-translucent MRT requires Vulkan independentBlend",
            ));
        }
        let color_blend =
            vk::PipelineColorBlendStateCreateInfo::default().attachments(&color_blend_attachments);
        let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let dynamic_state =
            vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);
        let depth_state = depth_stencil_state(desc);
        let color_formats = desc
            .color_formats
            .iter()
            .copied()
            .map(texture_format)
            .collect::<Vec<_>>();
        let mut rendering = vk::PipelineRenderingCreateInfo::default()
            .color_attachment_formats(&color_formats)
            .depth_attachment_format(
                desc.depth_format
                    .map(texture_format)
                    .unwrap_or(vk::Format::UNDEFINED),
            );
        let mut create_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .vertex_input_state(&vertex_input)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&rasterization)
            .multisample_state(&multisample)
            .color_blend_state(&color_blend)
            .dynamic_state(&dynamic_state)
            .layout(layout)
            .push_next(&mut rendering);
        if let Some(depth_state) = depth_state.as_ref() {
            create_info = create_info.depth_stencil_state(depth_state);
        }
        let pipeline = unsafe {
            self.context.device.create_graphics_pipelines(
                vk::PipelineCache::null(),
                &[create_info],
                None,
            )
        }
        .map_err(|(_, error)| {
            GalError::backend(format!(
                "failed to create graphics pipeline '{}': {error:?}",
                desc.label
            ))
        })?
        .remove(0);
        self.context.set_object_name(
            pipeline,
            &debug_name("graphics-pipeline", handle, &desc.label),
        );
        trace_glibc_allocator_checkpoint(&format!("graphics-pipeline.end.{}", desc.label));
        self.retain_live_graphics_pipeline_cache_keys();
        let pipeline = Arc::new(NativeGraphicsPipeline {
            context: self.context.clone(),
            pipeline,
        });
        self.graphics_pipeline_cache
            .insert(cache_key, Arc::downgrade(&pipeline));
        trace_native_graphics_pipeline_cache(
            false,
            &desc.label,
            self.graphics_pipeline_cache.len(),
        );
        Ok(GraphicsPipelineObject {
            token,
            label: desc.label.clone(),
            pipeline,
            layout: desc.layout,
        })
    }

    fn retain_live_graphics_pipeline_cache_keys(&mut self) {
        if self.graphics_pipeline_cache.len() < MAX_NATIVE_GRAPHICS_PIPELINE_CACHE_KEYS {
            return;
        }
        self.graphics_pipeline_cache
            .retain(|_, pipeline| pipeline.strong_count() != 0);
        // A cache key is purely an optimization.  Once the bound is still
        // reached by live logical resources, dropping keys cannot affect
        // ownership or rendering correctness; it merely forgoes a future
        // reuse opportunity until one of those resources retires.
        if self.graphics_pipeline_cache.len() >= MAX_NATIVE_GRAPHICS_PIPELINE_CACHE_KEYS {
            self.graphics_pipeline_cache.clear();
        }
    }

    fn create_compute_pipeline(
        &self,
        handle: Handle,
        desc: &ComputePipelineDesc,
        token: BackendToken,
    ) -> GalResult<ComputePipelineObject> {
        trace_glibc_allocator_checkpoint(&format!("compute-pipeline.begin.{}", desc.label));
        let shader = self.shader(desc.shader)?;
        let layout = self.pipeline_layout(desc.layout)?.layout;
        let stage = shader_stage_create(shader);
        let create_info = vk::ComputePipelineCreateInfo::default()
            .stage(stage)
            .layout(layout);
        let pipeline = unsafe {
            self.context.device.create_compute_pipelines(
                vk::PipelineCache::null(),
                &[create_info],
                None,
            )
        }
        .map_err(|(_, error)| {
            GalError::backend(format!(
                "failed to create compute pipeline '{}': {error:?}",
                desc.label
            ))
        })?
        .remove(0);
        self.context.set_object_name(
            pipeline,
            &debug_name("compute-pipeline", handle, &desc.label),
        );
        trace_glibc_allocator_checkpoint(&format!("compute-pipeline.end.{}", desc.label));
        Ok(ComputePipelineObject {
            token,
            pipeline,
            layout: desc.layout,
        })
    }

    fn shader(&self, handle: Handle) -> GalResult<&ShaderModuleObject> {
        match self.objects.get(&handle) {
            Some(VulkanObject::ShaderModule(shader)) => Ok(shader),
            _ => Err(GalError::backend(
                "pipeline references missing shader module",
            )),
        }
    }

    fn destroy_object(&mut self, object: VulkanObject) {
        let _zone = trace::Zone::new("vulkan.resources.destroy-native");
        unsafe {
            match object {
                VulkanObject::Buffer(object) => {
                    self.context.device.destroy_buffer(object.buffer, None);
                    self.buffer_memory.release(object.allocation);
                }
                VulkanObject::Texture(object) => {
                    self.context.device.destroy_image(object.image, None);
                    self.texture_memory.release(object.allocation);
                }
                VulkanObject::TextureView(object) => {
                    self.context.device.destroy_image_view(object.view, None);
                }
                VulkanObject::Sampler(object) => {
                    self.context.device.destroy_sampler(object.sampler, None);
                }
                // Vulkan combines the two native objects in descriptor writes;
                // the logical GAL pair does not own another Vulkan object.
                VulkanObject::CombinedTextureSampler(_) => {}
                VulkanObject::ShaderModule(object) => {
                    self.context
                        .device
                        .destroy_shader_module(object.module, None);
                }
                VulkanObject::ResourceLayout(object) => {
                    self.context
                        .device
                        .destroy_descriptor_set_layout(object.layout, None);
                }
                VulkanObject::ResourceSet(object) => {
                    self.descriptor_pools
                        .release(object.pool_block_id, object.pool, object.set);
                }
                VulkanObject::PipelineLayout(object) => {
                    self.context
                        .device
                        .destroy_pipeline_layout(object.layout, None);
                }
                VulkanObject::GraphicsPipeline(_) => {}
                VulkanObject::ComputePipeline(object) => {
                    self.context.device.destroy_pipeline(object.pipeline, None);
                }
                VulkanObject::RenderTarget(_)
                | VulkanObject::FrameTarget(_)
                | VulkanObject::RenderPass(_) => {}
            }
        }
        self.trace_memory_residency("destroy");
    }
}

fn validate_d3_image_format_properties(
    desc: &TextureDesc,
    properties: vk::ImageFormatProperties,
) -> GalResult<()> {
    let max_extent = properties.max_extent;
    if desc.extent.width > max_extent.width
        || desc.extent.height > max_extent.height
        || desc.extent.depth > max_extent.depth
        || desc.mip_levels > properties.max_mip_levels
        || desc.array_layers > properties.max_array_layers
        || !properties
            .sample_counts
            .contains(vk::SampleCountFlags::TYPE_1)
    {
        return Err(GalError::unsupported_feature(format!(
            "Vulkan D3 image '{}' exceeds device format limits for {:?}",
            desc.label, desc.format
        )));
    }
    Ok(())
}

impl Drop for VulkanObjects {
    fn drop(&mut self) {
        self.destroy_all();
    }
}

pub(super) enum VulkanObject {
    Buffer(BufferObject),
    Texture(TextureObject),
    TextureView(TextureViewObject),
    Sampler(SamplerObject),
    CombinedTextureSampler(CombinedTextureSamplerObject),
    ShaderModule(ShaderModuleObject),
    ResourceLayout(ResourceLayoutObject),
    ResourceSet(ResourceSetObject),
    PipelineLayout(PipelineLayoutObject),
    GraphicsPipeline(GraphicsPipelineObject),
    ComputePipeline(ComputePipelineObject),
    RenderTarget(RenderTargetObject),
    FrameTarget(FrameTargetObject),
    RenderPass(RenderPassObject),
}

impl VulkanObject {
    fn token(&self) -> BackendToken {
        match self {
            Self::Buffer(object) => object.token,
            Self::Texture(object) => object.token,
            Self::TextureView(object) => object.token,
            Self::Sampler(object) => object.token,
            Self::CombinedTextureSampler(object) => object.token,
            Self::ShaderModule(object) => object.token,
            Self::ResourceLayout(object) => object.token,
            Self::ResourceSet(object) => object.token,
            Self::PipelineLayout(object) => object.token,
            Self::GraphicsPipeline(object) => object.token,
            Self::ComputePipeline(object) => object.token,
            Self::RenderTarget(object) => object.token,
            Self::FrameTarget(object) => object.token,
            Self::RenderPass(object) => object.token,
        }
    }

    fn kind(&self) -> HandleKind {
        match self {
            Self::Buffer(_) => HandleKind::Buffer,
            Self::Texture(_) => HandleKind::Texture,
            Self::TextureView(_) => HandleKind::TextureView,
            Self::Sampler(_) => HandleKind::Sampler,
            Self::CombinedTextureSampler(_) => HandleKind::CombinedTextureSampler,
            Self::ShaderModule(_) => HandleKind::ShaderModule,
            Self::ResourceLayout(_) => HandleKind::ResourceLayout,
            Self::ResourceSet(_) => HandleKind::ResourceSet,
            Self::PipelineLayout(_) => HandleKind::PipelineLayout,
            Self::GraphicsPipeline(_) => HandleKind::GraphicsPipeline,
            Self::ComputePipeline(_) => HandleKind::ComputePipeline,
            Self::RenderTarget(_) => HandleKind::RenderTarget,
            Self::FrameTarget(_) => HandleKind::FrameTarget,
            Self::RenderPass(_) => HandleKind::RenderPass,
        }
    }
}

#[allow(dead_code)]
pub(super) struct BufferObject {
    pub(super) token: BackendToken,
    pub(super) buffer: vk::Buffer,
    pub(super) memory: vk::DeviceMemory,
    /// The physical byte offset where this logical GAL buffer is bound.
    /// Commands use `buffer` offsets and therefore remain in logical space;
    /// only host mapping needs to add this offset.
    pub(super) memory_offset: u64,
    allocation: DeviceMemoryAllocation,
    pub(super) size: u64,
    pub(super) memory_domain: MemoryDomain,
}

#[allow(dead_code)]
pub(super) struct TextureObject {
    pub(super) token: BackendToken,
    pub(super) label: String,
    pub(super) image: vk::Image,
    pub(super) memory: vk::DeviceMemory,
    allocation: DeviceMemoryAllocation,
    pub(super) format: vk::Format,
    pub(super) copy_bytes_per_texel: u32,
    pub(super) extent: Extent3d,
    pub(super) dimension: TextureDimension,
    pub(super) mip_levels: u32,
    pub(super) array_layers: u32,
    pub(super) aspect: vk::ImageAspectFlags,
}

#[allow(dead_code)]
pub(super) struct TextureViewObject {
    pub(super) token: BackendToken,
    pub(super) label: String,
    pub(super) view: vk::ImageView,
    pub(super) texture: Handle,
    pub(super) format: vk::Format,
    pub(super) aspect: vk::ImageAspectFlags,
    pub(super) base_mip: u32,
    pub(super) mip_levels: u32,
    pub(super) base_layer: u32,
    pub(super) array_layers: u32,
}

pub(super) struct SamplerObject {
    pub(super) token: BackendToken,
    pub(super) sampler: vk::Sampler,
}

pub(super) struct CombinedTextureSamplerObject {
    pub(super) token: BackendToken,
    pub(super) texture_view: Handle,
    pub(super) sampler: Handle,
}

pub(super) struct ShaderModuleObject {
    pub(super) token: BackendToken,
    pub(super) module: vk::ShaderModule,
    pub(super) stage: vk::ShaderStageFlags,
    pub(super) entry_point: CString,
}

pub(super) struct ResourceLayoutObject {
    pub(super) token: BackendToken,
    pub(super) layout: vk::DescriptorSetLayout,
    pub(super) bindings: Vec<ResourceBindingDesc>,
}

pub(super) struct ResourceSetObject {
    pub(super) token: BackendToken,
    pool_block_id: u64,
    pub(super) pool: vk::DescriptorPool,
    pub(super) set: vk::DescriptorSet,
    pub(super) dynamic_offsets: Vec<u32>,
}

pub(super) struct PipelineLayoutObject {
    pub(super) token: BackendToken,
    pub(super) layout: vk::PipelineLayout,
}

/// Shared only by equal immutable graphics-pipeline descriptions.  Destruction
/// is reference-counted by the logical GAL pipeline objects, so the cache's
/// weak entry cannot keep a native pipeline alive after explicit retirement.
pub(super) struct NativeGraphicsPipeline {
    context: Arc<VulkanContext>,
    pub(super) pipeline: vk::Pipeline,
}

impl Drop for NativeGraphicsPipeline {
    fn drop(&mut self) {
        unsafe { self.context.device.destroy_pipeline(self.pipeline, None) };
    }
}

pub(super) struct GraphicsPipelineObject {
    pub(super) token: BackendToken,
    pub(super) label: String,
    pub(super) pipeline: Arc<NativeGraphicsPipeline>,
    pub(super) layout: Handle,
}

pub(super) struct ComputePipelineObject {
    pub(super) token: BackendToken,
    pub(super) pipeline: vk::Pipeline,
    pub(super) layout: Handle,
}

#[allow(dead_code)]
pub(super) struct RenderTargetObject {
    pub(super) token: BackendToken,
    pub(super) color_views: Vec<Handle>,
    pub(super) depth_stencil_view: Option<Handle>,
    pub(super) extent: Extent3d,
}

#[allow(dead_code)]
pub(super) struct FrameTargetObject {
    pub(super) token: BackendToken,
    pub(super) frame_id: u64,
    pub(super) render_target: crate::render::vulkanic::frame::FrameRenderTargetId,
    pub(super) extent: Extent3d,
    pub(super) color_format: TextureFormat,
    pub(super) image_index: u32,
    pub(super) image: vk::Image,
    pub(super) image_view: vk::ImageView,
    pub(super) image_layout: vk::ImageLayout,
}

#[allow(dead_code)]
pub(super) struct RenderPassObject {
    pub(super) token: BackendToken,
    pub(super) label: String,
    pub(super) target: Handle,
    pub(super) color_formats: Vec<ColorFormat>,
    pub(super) depth_format: Option<TextureFormat>,
}

pub(super) fn texture_format(format: TextureFormat) -> vk::Format {
    match format {
        TextureFormat::Rgba8Unorm => vk::Format::R8G8B8A8_UNORM,
        TextureFormat::Bgra8Unorm => vk::Format::B8G8R8A8_UNORM,
        TextureFormat::Rgba16Float => vk::Format::R16G16B16A16_SFLOAT,
        TextureFormat::Depth24Stencil8 => vk::Format::D24_UNORM_S8_UINT,
        TextureFormat::Depth32Float => vk::Format::D32_SFLOAT,
        TextureFormat::R8Uint => vk::Format::R8_UINT,
        TextureFormat::R11fG11fB10f => vk::Format::B10G11R11_UFLOAT_PACK32,
        TextureFormat::R32Float => vk::Format::R32_SFLOAT,
        TextureFormat::Rgb16Float => vk::Format::R16G16B16_SFLOAT,
        TextureFormat::R8Unorm => vk::Format::R8_UNORM,
        TextureFormat::Rgba8Snorm => vk::Format::R8G8B8A8_SNORM,
    }
}

pub(super) fn aspect_for_format(format: TextureFormat) -> vk::ImageAspectFlags {
    match format {
        TextureFormat::Depth24Stencil8 => {
            vk::ImageAspectFlags::DEPTH | vk::ImageAspectFlags::STENCIL
        }
        TextureFormat::Depth32Float => vk::ImageAspectFlags::DEPTH,
        _ => vk::ImageAspectFlags::COLOR,
    }
}

pub(super) fn texture_usage_flags(usages: &[TextureUsage]) -> vk::ImageUsageFlags {
    let mut flags = vk::ImageUsageFlags::empty();
    for usage in usages {
        flags |= match usage {
            TextureUsage::Sampled => vk::ImageUsageFlags::SAMPLED,
            TextureUsage::Storage => vk::ImageUsageFlags::STORAGE,
            TextureUsage::ColorAttachment => vk::ImageUsageFlags::COLOR_ATTACHMENT,
            TextureUsage::DepthStencilAttachment => vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT,
            TextureUsage::TransferSrc | TextureUsage::HostRead => vk::ImageUsageFlags::TRANSFER_SRC,
            TextureUsage::TransferDst | TextureUsage::HostWrite => {
                vk::ImageUsageFlags::TRANSFER_DST
            }
            TextureUsage::Present => vk::ImageUsageFlags::COLOR_ATTACHMENT,
        };
    }
    flags
}

pub(super) fn buffer_usage_flags(usages: &[BufferUsage]) -> vk::BufferUsageFlags {
    let mut flags = vk::BufferUsageFlags::empty();
    for usage in usages {
        flags |= match usage {
            BufferUsage::Vertex => vk::BufferUsageFlags::VERTEX_BUFFER,
            BufferUsage::Index => vk::BufferUsageFlags::INDEX_BUFFER,
            BufferUsage::Uniform => vk::BufferUsageFlags::UNIFORM_BUFFER,
            BufferUsage::Storage => vk::BufferUsageFlags::STORAGE_BUFFER,
            BufferUsage::TransferSrc | BufferUsage::HostRead => vk::BufferUsageFlags::TRANSFER_SRC,
            BufferUsage::TransferDst | BufferUsage::HostWrite => vk::BufferUsageFlags::TRANSFER_DST,
            BufferUsage::Indirect => vk::BufferUsageFlags::INDIRECT_BUFFER,
        };
    }
    flags
}

pub(super) fn memory_flags(domain: MemoryDomain) -> vk::MemoryPropertyFlags {
    match domain {
        MemoryDomain::DeviceLocal => vk::MemoryPropertyFlags::DEVICE_LOCAL,
        MemoryDomain::Upload | MemoryDomain::Readback => {
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT
        }
    }
}

pub(super) fn filter(filter: SamplerFilter) -> vk::Filter {
    match filter {
        SamplerFilter::Nearest => vk::Filter::NEAREST,
        SamplerFilter::Linear => vk::Filter::LINEAR,
    }
}

pub(super) fn mipmap_filter(filter: SamplerFilter) -> vk::SamplerMipmapMode {
    match filter {
        SamplerFilter::Nearest => vk::SamplerMipmapMode::NEAREST,
        SamplerFilter::Linear => vk::SamplerMipmapMode::LINEAR,
    }
}

pub(super) fn address_mode(mode: SamplerAddressMode) -> vk::SamplerAddressMode {
    match mode {
        SamplerAddressMode::ClampToEdge => vk::SamplerAddressMode::CLAMP_TO_EDGE,
        SamplerAddressMode::Repeat => vk::SamplerAddressMode::REPEAT,
        SamplerAddressMode::MirroredRepeat => vk::SamplerAddressMode::MIRRORED_REPEAT,
    }
}

pub(super) fn shader_stage(stage: ShaderStage) -> vk::ShaderStageFlags {
    match stage {
        ShaderStage::Vertex => vk::ShaderStageFlags::VERTEX,
        ShaderStage::Fragment => vk::ShaderStageFlags::FRAGMENT,
        ShaderStage::Compute => vk::ShaderStageFlags::COMPUTE,
        ShaderStage::Geometry => vk::ShaderStageFlags::GEOMETRY,
        ShaderStage::TessControl => vk::ShaderStageFlags::TESSELLATION_CONTROL,
        ShaderStage::TessEvaluation => vk::ShaderStageFlags::TESSELLATION_EVALUATION,
    }
}

fn shaderc_kind(stage: ShaderStage) -> GalResult<shaderc::ShaderKind> {
    match stage {
        ShaderStage::Vertex => Ok(shaderc::ShaderKind::Vertex),
        ShaderStage::Fragment => Ok(shaderc::ShaderKind::Fragment),
        ShaderStage::Compute => Ok(shaderc::ShaderKind::Compute),
        ShaderStage::Geometry => Ok(shaderc::ShaderKind::Geometry),
        ShaderStage::TessControl => Ok(shaderc::ShaderKind::TessControl),
        ShaderStage::TessEvaluation => Ok(shaderc::ShaderKind::TessEvaluation),
    }
}

pub(super) fn shader_stage_flags(stages: PipelineStageFlags) -> vk::ShaderStageFlags {
    let mut flags = vk::ShaderStageFlags::empty();
    if stages.0 & PipelineStageFlags::DRAW.0 != 0 {
        flags |= vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT;
    }
    if stages.0 & PipelineStageFlags::COMPUTE.0 != 0 {
        flags |= vk::ShaderStageFlags::COMPUTE;
    }
    flags
}

pub(super) fn descriptor_type(kind: ResourceBindingKind) -> vk::DescriptorType {
    match kind {
        ResourceBindingKind::UniformBuffer => vk::DescriptorType::UNIFORM_BUFFER,
        ResourceBindingKind::StorageBuffer => vk::DescriptorType::STORAGE_BUFFER,
        ResourceBindingKind::SampledTexture => vk::DescriptorType::SAMPLED_IMAGE,
        ResourceBindingKind::StorageTexture => vk::DescriptorType::STORAGE_IMAGE,
        ResourceBindingKind::Sampler => vk::DescriptorType::SAMPLER,
        ResourceBindingKind::CombinedTextureSampler => vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
    }
}

fn descriptor_type_for_binding(
    kind: ResourceBindingKind,
    dynamic_offset_count: u32,
) -> vk::DescriptorType {
    if dynamic_offset_count == 0 {
        return descriptor_type(kind);
    }
    match kind {
        ResourceBindingKind::UniformBuffer => vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC,
        ResourceBindingKind::StorageBuffer => vk::DescriptorType::STORAGE_BUFFER_DYNAMIC,
        _ => descriptor_type(kind),
    }
}

fn sampled_image_layout_for_aspect(aspect: vk::ImageAspectFlags) -> vk::ImageLayout {
    if aspect.contains(vk::ImageAspectFlags::STENCIL) {
        vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL
    } else if aspect.contains(vk::ImageAspectFlags::DEPTH) {
        vk::ImageLayout::DEPTH_READ_ONLY_OPTIMAL
    } else {
        vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL
    }
}

#[cfg(test)]
mod sampled_image_layout_tests {
    use super::*;

    #[test]
    fn depth_only_views_use_depth_read_only_layout_for_all_sampled_bindings() {
        assert_eq!(
            sampled_image_layout_for_aspect(vk::ImageAspectFlags::DEPTH).as_raw(),
            vk::ImageLayout::DEPTH_READ_ONLY_OPTIMAL.as_raw()
        );
    }

    #[test]
    fn depth_stencil_views_use_combined_read_only_layout() {
        assert_eq!(
            sampled_image_layout_for_aspect(
                vk::ImageAspectFlags::DEPTH | vk::ImageAspectFlags::STENCIL
            )
            .as_raw(),
            vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL.as_raw()
        );
    }

    #[test]
    fn color_views_use_shader_read_only_layout() {
        assert_eq!(
            sampled_image_layout_for_aspect(vk::ImageAspectFlags::COLOR).as_raw(),
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL.as_raw()
        );
    }
}

fn descriptor_type_for_resource_binding(binding: &ResourceBinding) -> vk::DescriptorType {
    descriptor_type_for_binding(binding.kind, binding.dynamic_offsets.len() as u32)
}

pub(super) fn shader_stage_create(
    shader: &ShaderModuleObject,
) -> vk::PipelineShaderStageCreateInfo<'_> {
    let name: &CStr = shader.entry_point.as_c_str();
    vk::PipelineShaderStageCreateInfo::default()
        .stage(shader.stage)
        .module(shader.module)
        .name(name)
}

pub(super) fn topology(topology: PrimitiveTopology) -> vk::PrimitiveTopology {
    match topology {
        PrimitiveTopology::Points => vk::PrimitiveTopology::POINT_LIST,
        PrimitiveTopology::Lines => vk::PrimitiveTopology::LINE_LIST,
        PrimitiveTopology::Triangles => vk::PrimitiveTopology::TRIANGLE_LIST,
        PrimitiveTopology::TriangleFan => vk::PrimitiveTopology::TRIANGLE_FAN,
    }
}

pub(super) fn cull_mode(mode: CullMode) -> vk::CullModeFlags {
    match mode {
        CullMode::None => vk::CullModeFlags::NONE,
        CullMode::Front => vk::CullModeFlags::FRONT,
        CullMode::Back => vk::CullModeFlags::BACK,
    }
}

pub(super) fn front_face(face: crate::render::vulkanic::resources::FrontFace) -> vk::FrontFace {
    match face {
        crate::render::vulkanic::resources::FrontFace::CounterClockwise => {
            vk::FrontFace::COUNTER_CLOCKWISE
        }
        crate::render::vulkanic::resources::FrontFace::Clockwise => vk::FrontFace::CLOCKWISE,
    }
}

pub(super) fn compare_op(compare: CompareOp) -> vk::CompareOp {
    match compare {
        CompareOp::Always => vk::CompareOp::ALWAYS,
        CompareOp::Less => vk::CompareOp::LESS,
        CompareOp::LessOrEqual => vk::CompareOp::LESS_OR_EQUAL,
        CompareOp::Equal => vk::CompareOp::EQUAL,
        CompareOp::Greater => vk::CompareOp::GREATER,
    }
}

fn stencil_op(operation: StencilOp) -> vk::StencilOp {
    match operation {
        StencilOp::Keep => vk::StencilOp::KEEP,
        StencilOp::Replace => vk::StencilOp::REPLACE,
    }
}

/// Keeps Vulkan lowering aligned with the explicit GAL contract: absence of a
/// compare operation means depth testing is disabled. A depth attachment may
/// still be present for another pass or for compatible render-target reuse.
fn depth_stencil_state(
    desc: &GraphicsPipelineDesc,
) -> Option<vk::PipelineDepthStencilStateCreateInfo<'static>> {
    if desc.depth_format.is_none() && desc.stencil.is_none() {
        return None;
    }
    Some({
        let depth_test_enabled = desc.depth_compare.is_some();
        let mut state = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(depth_test_enabled)
            .depth_write_enable(depth_test_enabled && desc.depth_write)
            .depth_compare_op(
                desc.depth_compare
                    .map(compare_op)
                    .unwrap_or(vk::CompareOp::ALWAYS),
            );
        if let Some(stencil) = desc.stencil {
            state = state
                .stencil_test_enable(true)
                .front(vk::StencilOpState {
                    fail_op: stencil_op(stencil.front.fail_op),
                    pass_op: stencil_op(stencil.front.pass_op),
                    depth_fail_op: stencil_op(stencil.front.depth_fail_op),
                    compare_op: compare_op(stencil.front.compare),
                    compare_mask: stencil.front.read_mask,
                    write_mask: stencil.front.write_mask,
                    reference: stencil.front.reference,
                })
                .back(vk::StencilOpState {
                    fail_op: stencil_op(stencil.back.fail_op),
                    pass_op: stencil_op(stencil.back.pass_op),
                    depth_fail_op: stencil_op(stencil.back.depth_fail_op),
                    compare_op: compare_op(stencil.back.compare),
                    compare_mask: stencil.back.read_mask,
                    write_mask: stencil.back.write_mask,
                    reference: stencil.back.reference,
                });
        }
        state
    })
}

pub(super) fn color_blend_attachment(
    blend: BlendMode,
    attachment_index: usize,
) -> vk::PipelineColorBlendAttachmentState {
    if blend == BlendMode::TerrainTranslucent && attachment_index != 0 {
        return color_blend_attachment(BlendMode::Disabled, attachment_index);
    }
    if blend == BlendMode::TerrainTranslucent {
        return color_blend_attachment(BlendMode::Alpha, attachment_index);
    }
    match blend {
        BlendMode::Disabled => vk::PipelineColorBlendAttachmentState::default()
            .color_write_mask(vk::ColorComponentFlags::RGBA),
        BlendMode::Alpha => vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
            .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .alpha_blend_op(vk::BlendOp::ADD)
            .color_write_mask(vk::ColorComponentFlags::RGBA),
        BlendMode::Premultiplied => vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::ONE)
            .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .alpha_blend_op(vk::BlendOp::ADD)
            .color_write_mask(vk::ColorComponentFlags::RGBA),
        BlendMode::Additive => vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::ONE)
            .dst_color_blend_factor(vk::BlendFactor::ONE)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ONE)
            .alpha_blend_op(vk::BlendOp::ADD)
            .color_write_mask(vk::ColorComponentFlags::RGBA),
        BlendMode::Invert => vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::ONE_MINUS_DST_COLOR)
            .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_COLOR)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ZERO)
            .alpha_blend_op(vk::BlendOp::ADD)
            .color_write_mask(vk::ColorComponentFlags::RGBA),
        BlendMode::Multiply => vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::DST_COLOR)
            .dst_color_blend_factor(vk::BlendFactor::ZERO)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ZERO)
            .alpha_blend_op(vk::BlendOp::ADD)
            .color_write_mask(vk::ColorComponentFlags::RGBA),
        BlendMode::Overlay => vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
            .dst_color_blend_factor(vk::BlendFactor::ONE)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ZERO)
            .alpha_blend_op(vk::BlendOp::ADD)
            .color_write_mask(vk::ColorComponentFlags::RGBA),
        BlendMode::Glint => vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::SRC_COLOR)
            .dst_color_blend_factor(vk::BlendFactor::ONE)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ZERO)
            .dst_alpha_blend_factor(vk::BlendFactor::ONE)
            .alpha_blend_op(vk::BlendOp::ADD)
            .color_write_mask(vk::ColorComponentFlags::RGBA),
        BlendMode::Vignette => vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::ZERO)
            .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_COLOR)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ZERO)
            .alpha_blend_op(vk::BlendOp::ADD)
            .color_write_mask(vk::ColorComponentFlags::RGBA),
        BlendMode::TerrainTranslucent => unreachable!(),
    }
}

fn debug_name(kind: &str, handle: Handle, label: &str) -> String {
    format!("gal.{kind}.0x{:016x}.{label}", handle.raw())
}

/// Select the Vulkan view shape from the explicit GAL texture description.
/// Layered two-dimensional images must use an array view so every layer is
/// addressable by atlas and shader-pack consumers.
fn image_view_type(dimension: TextureDimension, array_layers: u32) -> vk::ImageViewType {
    match dimension {
        TextureDimension::D2 if array_layers > 1 => vk::ImageViewType::TYPE_2D_ARRAY,
        TextureDimension::D2 => vk::ImageViewType::TYPE_2D,
        TextureDimension::D3 => vk::ImageViewType::TYPE_3D,
        _ => unreachable!("GAL validated supported texture dimension"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triangle_fan_topology_lowers_to_native_vulkan_fan_assembly() {
        assert!(topology(PrimitiveTopology::TriangleFan) == vk::PrimitiveTopology::TRIANGLE_FAN);
    }

    #[test]
    fn device_memory_block_reuses_released_ranges_without_growing_residency() {
        let mut block = DeviceMemoryBlock {
            id: 7,
            memory: vk::DeviceMemory::null(),
            memory_type_index: 0,
            size: 4096,
            free_ranges: vec![BufferMemoryRange {
                offset: 0,
                size: 4096,
            }],
        };
        let first = block.allocate(512, 256).expect("first allocation fits");
        let second = block.allocate(768, 256).expect("second allocation fits");
        assert_eq!(0, first);
        assert_eq!(512, second);
        block.release(DeviceMemoryAllocation {
            block_id: 7,
            memory: vk::DeviceMemory::null(),
            offset: first,
            size: 512,
        });
        let reused = block
            .allocate(256, 256)
            .expect("released range is reusable");
        assert_eq!(0, reused);
        assert!(!block.is_empty());
    }

    #[test]
    fn device_memory_block_reports_empty_only_after_every_suballocation_retires() {
        let mut block = DeviceMemoryBlock {
            id: 9,
            memory: vk::DeviceMemory::null(),
            memory_type_index: 0,
            size: 1024,
            free_ranges: vec![BufferMemoryRange {
                offset: 0,
                size: 1024,
            }],
        };
        let allocation = block.allocate(1024, 1).expect("whole page fits");
        assert!(!block.is_empty());
        block.release(DeviceMemoryAllocation {
            block_id: 9,
            memory: vk::DeviceMemory::null(),
            offset: allocation,
            size: 1024,
        });
        assert!(block.is_empty());
    }

    #[test]
    fn native_allocator_trim_requires_both_retirement_batch_and_large_free_arena() {
        assert!(!should_trim_native_allocator(
            NATIVE_ALLOCATOR_TRIM_RETIRE_INTERVAL - 1,
            NATIVE_ALLOCATOR_TRIM_MIN_FREE_BYTES,
        ));
        assert!(!should_trim_native_allocator(
            NATIVE_ALLOCATOR_TRIM_RETIRE_INTERVAL,
            NATIVE_ALLOCATOR_TRIM_MIN_FREE_BYTES - 1,
        ));
        assert!(should_trim_native_allocator(
            NATIVE_ALLOCATOR_TRIM_RETIRE_INTERVAL,
            NATIVE_ALLOCATOR_TRIM_MIN_FREE_BYTES,
        ));
    }

    #[test]
    fn shader_compile_cache_key_preserves_exact_stage_entry_point_and_source() {
        let vertex = ShaderCompileKey {
            stage: ShaderStage::Vertex,
            entry_point: "main".to_owned(),
            code: b"void main(){}".to_vec(),
        };
        let fragment = ShaderCompileKey {
            stage: ShaderStage::Fragment,
            entry_point: "main".to_owned(),
            code: b"void main(){}".to_vec(),
        };
        let changed_source = ShaderCompileKey {
            stage: ShaderStage::Vertex,
            entry_point: "main".to_owned(),
            code: b"void main(){gl_Position=vec4(0.0);}".to_vec(),
        };
        let mut cache = HashMap::new();
        cache.insert(vertex.clone(), vec![1]);
        cache.insert(fragment.clone(), vec![2]);
        cache.insert(changed_source.clone(), vec![3]);
        assert_eq!(cache.len(), 3);
        assert_eq!(cache.get(&vertex), Some(&vec![1]));
        assert_eq!(cache.get(&fragment), Some(&vec![2]));
        assert_eq!(cache.get(&changed_source), Some(&vec![3]));
    }

    #[test]
    fn device_memory_block_reuses_aligned_ranges_after_destroy() {
        let memory = vk::DeviceMemory::null();
        let mut block = DeviceMemoryBlock {
            id: 7,
            memory,
            memory_type_index: 0,
            size: 256,
            free_ranges: vec![BufferMemoryRange {
                offset: 0,
                size: 256,
            }],
        };
        let first_offset = block.allocate(32, 64).expect("first allocation");
        let second_offset = block.allocate(32, 64).expect("second allocation");
        assert_eq!(0, first_offset);
        assert_eq!(64, second_offset);

        block.release(DeviceMemoryAllocation {
            block_id: 7,
            memory,
            offset: first_offset,
            size: 32,
        });
        block.release(DeviceMemoryAllocation {
            block_id: 7,
            memory,
            offset: second_offset,
            size: 32,
        });
        assert!(block.is_empty());
        assert_eq!(0, block.allocate(128, 128).expect("reused allocation"));
    }

    #[test]
    fn buffer_memory_alignment_rejects_overflow() {
        assert_eq!(Some(192), align_up(129, 64));
        assert_eq!(None, align_up(u64::MAX, 2));
    }

    fn d3_texture_desc() -> TextureDesc {
        TextureDesc {
            label: "d3-format-properties".to_owned(),
            dimension: TextureDimension::D3,
            format: TextureFormat::R8Uint,
            extent: Extent3d {
                width: 8,
                height: 4,
                depth: 2,
            },
            mip_levels: 2,
            array_layers: 1,
            usages: vec![TextureUsage::Sampled, TextureUsage::Storage],
        }
    }

    fn d3_format_properties() -> vk::ImageFormatProperties {
        vk::ImageFormatProperties {
            max_extent: vk::Extent3D {
                width: 16,
                height: 16,
                depth: 16,
            },
            max_mip_levels: 4,
            max_array_layers: 1,
            sample_counts: vk::SampleCountFlags::TYPE_1,
            max_resource_size: u64::MAX,
        }
    }

    #[test]
    fn d3_format_properties_reject_unsupported_extent_mips_and_samples() {
        let properties = d3_format_properties();
        validate_d3_image_format_properties(&d3_texture_desc(), properties)
            .expect("bounded D3 image should fit device format properties");

        let mut oversized = d3_texture_desc();
        oversized.extent.depth = 17;
        assert!(validate_d3_image_format_properties(&oversized, properties).is_err());

        let mut too_many_mips = d3_texture_desc();
        too_many_mips.mip_levels = 5;
        assert!(validate_d3_image_format_properties(&too_many_mips, properties).is_err());

        let mut no_single_sample = properties;
        no_single_sample.sample_counts = vk::SampleCountFlags::TYPE_2;
        assert!(validate_d3_image_format_properties(&d3_texture_desc(), no_single_sample).is_err());
    }

    #[test]
    fn shader_pack_color_formats_map_to_exact_vulkan_formats() {
        assert!(vk::Format::B10G11R11_UFLOAT_PACK32 == texture_format(TextureFormat::R11fG11fB10f));
        assert!(vk::Format::R32_SFLOAT == texture_format(TextureFormat::R32Float));
        assert!(vk::Format::R16G16B16_SFLOAT == texture_format(TextureFormat::Rgb16Float));
        assert!(vk::Format::R8_UNORM == texture_format(TextureFormat::R8Unorm));
        assert!(vk::Format::R8G8B8A8_SNORM == texture_format(TextureFormat::Rgba8Snorm));
    }

    #[test]
    fn layered_2d_textures_use_array_views() {
        assert!(vk::ImageViewType::TYPE_2D == image_view_type(TextureDimension::D2, 1));
        assert!(vk::ImageViewType::TYPE_2D_ARRAY == image_view_type(TextureDimension::D2, 2));
        assert!(vk::ImageViewType::TYPE_3D == image_view_type(TextureDimension::D3, 1));
    }

    #[test]
    fn overlay_blend_lowers_to_source_alpha_additive_equation() {
        let attachment = color_blend_attachment(BlendMode::Overlay, 0);
        assert_eq!(vk::TRUE, attachment.blend_enable);
        assert!(attachment.src_color_blend_factor == vk::BlendFactor::SRC_ALPHA);
        assert!(attachment.dst_color_blend_factor == vk::BlendFactor::ONE);
        assert!(attachment.color_blend_op == vk::BlendOp::ADD);
        assert!(attachment.src_alpha_blend_factor == vk::BlendFactor::ONE);
        assert!(attachment.dst_alpha_blend_factor == vk::BlendFactor::ZERO);
        assert!(attachment.alpha_blend_op == vk::BlendOp::ADD);
        assert!(attachment.color_write_mask == vk::ColorComponentFlags::RGBA);
    }

    #[test]
    fn multiply_blend_lowers_to_single_source_times_destination() {
        let attachment = color_blend_attachment(BlendMode::Multiply, 0);
        assert_eq!(vk::TRUE, attachment.blend_enable);
        assert!(attachment.src_color_blend_factor == vk::BlendFactor::DST_COLOR);
        assert!(attachment.dst_color_blend_factor == vk::BlendFactor::ZERO);
        assert!(attachment.color_blend_op == vk::BlendOp::ADD);
        assert!(attachment.src_alpha_blend_factor == vk::BlendFactor::ONE);
        assert!(attachment.dst_alpha_blend_factor == vk::BlendFactor::ZERO);
        assert!(attachment.alpha_blend_op == vk::BlendOp::ADD);
        assert!(attachment.color_write_mask == vk::ColorComponentFlags::RGBA);
    }

    #[test]
    fn premultiplied_blend_lowers_to_one_times_destination_alpha() {
        let attachment = color_blend_attachment(BlendMode::Premultiplied, 0);
        assert_eq!(vk::TRUE, attachment.blend_enable);
        assert!(attachment.src_color_blend_factor == vk::BlendFactor::ONE);
        assert!(attachment.dst_color_blend_factor == vk::BlendFactor::ONE_MINUS_SRC_ALPHA);
        assert!(attachment.color_blend_op == vk::BlendOp::ADD);
        assert!(attachment.src_alpha_blend_factor == vk::BlendFactor::ONE);
        assert!(attachment.dst_alpha_blend_factor == vk::BlendFactor::ONE_MINUS_SRC_ALPHA);
        assert!(attachment.alpha_blend_op == vk::BlendOp::ADD);
        assert!(attachment.color_write_mask == vk::ColorComponentFlags::RGBA);
    }

    #[test]
    fn terrain_translucent_blends_only_primary_mrt_attachment() {
        let primary = color_blend_attachment(BlendMode::TerrainTranslucent, 0);
        let auxiliary = color_blend_attachment(BlendMode::TerrainTranslucent, 1);
        assert_eq!(vk::TRUE, primary.blend_enable);
        assert!(vk::BlendFactor::SRC_ALPHA == primary.src_color_blend_factor);
        assert!(vk::FALSE == auxiliary.blend_enable);
        assert!(vk::ColorComponentFlags::RGBA == auxiliary.color_write_mask);
    }

    #[test]
    fn explicit_front_face_lowers_without_defaulting_to_counter_clockwise() {
        use crate::render::vulkanic::resources::FrontFace;

        assert!(front_face(FrontFace::CounterClockwise) == vk::FrontFace::COUNTER_CLOCKWISE);
        assert!(front_face(FrontFace::Clockwise) == vk::FrontFace::CLOCKWISE);
    }

    #[test]
    fn depth_compare_none_disables_depth_testing_even_with_a_depth_attachment() {
        let desc = GraphicsPipelineDesc {
            label: "depth-disabled".to_owned(),
            layout: Handle::from_raw(1),
            vertex_shader: Handle::from_raw(2),
            fragment_shader: Handle::from_raw(3),
            topology: PrimitiveTopology::Triangles,
            cull_mode: CullMode::Back,
            front_face: crate::render::vulkanic::resources::FrontFace::CounterClockwise,
            blend: BlendMode::Disabled,
            depth_compare: None,
            depth_write: true,
            depth_bias: None,
            color_formats: vec![TextureFormat::Rgba8Unorm],
            depth_format: Some(TextureFormat::Depth32Float),
            stencil: None,
        };
        let state = depth_stencil_state(&desc).expect("depth attachment has Vulkan state");
        assert_eq!(vk::FALSE, state.depth_test_enable);
        assert_eq!(vk::FALSE, state.depth_write_enable);
        assert!(state.depth_compare_op == vk::CompareOp::ALWAYS);
    }

    #[test]
    fn explicit_depth_compare_preserves_testing_and_write_policy() {
        let desc = GraphicsPipelineDesc {
            label: "depth-enabled".to_owned(),
            layout: Handle::from_raw(1),
            vertex_shader: Handle::from_raw(2),
            fragment_shader: Handle::from_raw(3),
            topology: PrimitiveTopology::Triangles,
            cull_mode: CullMode::Back,
            front_face: crate::render::vulkanic::resources::FrontFace::CounterClockwise,
            blend: BlendMode::Disabled,
            depth_compare: Some(CompareOp::LessOrEqual),
            depth_write: true,
            depth_bias: None,
            color_formats: vec![TextureFormat::Rgba8Unorm],
            depth_format: Some(TextureFormat::Depth32Float),
            stencil: None,
        };
        let state = depth_stencil_state(&desc).expect("depth attachment has Vulkan state");
        assert_eq!(vk::TRUE, state.depth_test_enable);
        assert_eq!(vk::TRUE, state.depth_write_enable);
        assert!(state.depth_compare_op == vk::CompareOp::LESS_OR_EQUAL);
    }

    #[test]
    fn explicit_stencil_mask_lowers_front_and_back_replace_state() {
        let desc = GraphicsPipelineDesc {
            label: "stencil-optical-mask".to_owned(),
            layout: Handle::from_raw(1),
            vertex_shader: Handle::from_raw(2),
            fragment_shader: Handle::from_raw(3),
            topology: PrimitiveTopology::Triangles,
            cull_mode: CullMode::None,
            front_face: crate::render::vulkanic::resources::FrontFace::CounterClockwise,
            blend: BlendMode::Disabled,
            depth_compare: Some(CompareOp::LessOrEqual),
            depth_write: false,
            depth_bias: None,
            color_formats: vec![TextureFormat::Rgba8Unorm],
            depth_format: Some(TextureFormat::Depth24Stencil8),
            stencil: Some(StencilState {
                front: StencilFaceState::replace(7, 0xff, 0xff),
                back: StencilFaceState::keep(CompareOp::Equal, 7, 0xff),
            }),
        };
        let state = depth_stencil_state(&desc).expect("stencil attachment has Vulkan state");
        assert_eq!(vk::TRUE, state.stencil_test_enable);
        assert!(state.front.compare_op == vk::CompareOp::ALWAYS);
        assert!(state.front.pass_op == vk::StencilOp::REPLACE);
        assert_eq!(7, state.front.reference);
        assert!(state.back.compare_op == vk::CompareOp::EQUAL);
        assert!(state.back.pass_op == vk::StencilOp::KEEP);
    }

    #[test]
    fn native_graphics_pipeline_cache_key_ignores_diagnostic_label() {
        let mut first = GraphicsPipelineDesc {
            label: "first logical label".to_owned(),
            layout: Handle::from_raw(1),
            vertex_shader: Handle::from_raw(2),
            fragment_shader: Handle::from_raw(3),
            topology: PrimitiveTopology::Triangles,
            cull_mode: CullMode::Back,
            front_face: crate::render::vulkanic::resources::FrontFace::CounterClockwise,
            blend: BlendMode::Alpha,
            depth_compare: Some(CompareOp::LessOrEqual),
            depth_write: true,
            depth_bias: Some(DepthBias::new(1.0, 2.0)),
            color_formats: vec![TextureFormat::Rgba8Unorm],
            depth_format: Some(TextureFormat::Depth32Float),
            stencil: Some(StencilState {
                front: StencilFaceState::replace(7, 0xff, 0xff),
                back: StencilFaceState::keep(CompareOp::Equal, 7, 0xff),
            }),
        };
        let first_key = GraphicsPipelineCacheKey::from_desc(&first);
        first.label = "same immutable state, another label".to_owned();
        assert_eq!(first_key, GraphicsPipelineCacheKey::from_desc(&first));
    }

    #[test]
    fn native_graphics_pipeline_cache_key_distinguishes_immutable_state() {
        let first = GraphicsPipelineDesc {
            label: "same label".to_owned(),
            layout: Handle::from_raw(1),
            vertex_shader: Handle::from_raw(2),
            fragment_shader: Handle::from_raw(3),
            topology: PrimitiveTopology::Triangles,
            cull_mode: CullMode::Back,
            front_face: crate::render::vulkanic::resources::FrontFace::CounterClockwise,
            blend: BlendMode::Disabled,
            depth_compare: Some(CompareOp::LessOrEqual),
            depth_write: true,
            depth_bias: None,
            color_formats: vec![TextureFormat::Rgba8Unorm],
            depth_format: Some(TextureFormat::Depth32Float),
            stencil: None,
        };
        let mut second = first.clone();
        second.depth_write = false;
        assert_ne!(
            GraphicsPipelineCacheKey::from_desc(&first),
            GraphicsPipelineCacheKey::from_desc(&second)
        );
        second = first.clone();
        second.fragment_shader = Handle::from_raw(4);
        assert_ne!(
            GraphicsPipelineCacheKey::from_desc(&first),
            GraphicsPipelineCacheKey::from_desc(&second)
        );
        second = first.clone();
        second.color_formats = vec![TextureFormat::Rgba16Float];
        assert_ne!(
            GraphicsPipelineCacheKey::from_desc(&first),
            GraphicsPipelineCacheKey::from_desc(&second)
        );
    }
}
