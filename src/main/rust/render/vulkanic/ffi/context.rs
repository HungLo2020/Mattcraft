use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Maximum number of simultaneously live FFI contexts in one thread-local
/// bridge registry. Contexts own the GAL, frontend caches, and backend state;
/// admission is fail-closed until an existing context is explicitly destroyed.
pub(crate) const MAX_BRIDGE_CONTEXTS: usize = 16;
static LIVE_BRIDGE_CONTEXTS: AtomicUsize = AtomicUsize::new(0);
static WINDOWED_PRESENTER_ACTIVE: AtomicBool = AtomicBool::new(false);

pub(crate) struct BridgeContext {
    pub(crate) gal: VulkanicGal,
    pub(crate) windowed_presenter: bool,
    pub(crate) gui_frontend: GuiFrontend,
    pub(crate) world_primitive_frontend: WorldPrimitiveFrontend,
    pub(crate) ffi_calls: u64,
    pub(crate) ffi_input_bytes: u64,
    pub(crate) ffi_output_bytes: u64,
    pub(crate) last_error: String,
    pub(crate) frame_targets: BTreeMap<FrameRenderTargetId, CachedFrameTarget>,
    pub(crate) stale_frame_targets: Vec<Handle>,
}

#[derive(Clone, Copy)]
pub(crate) struct CachedFrameTarget {
    pub(crate) handle: Handle,
}

pub(crate) fn destroy_stale_frame_targets(context: &mut BridgeContext) -> GalResult<()> {
    if !context.stale_frame_targets.is_empty() {
        context
            .gui_frontend
            .clear_frame_passes_for_targets(&mut context.gal, &context.stale_frame_targets);
        context
            .world_primitive_frontend
            .clear_frame_passes_for_targets(&mut context.gal, &context.stale_frame_targets);
    }
    for handle in std::mem::take(&mut context.stale_frame_targets) {
        context.gal.destroy(handle)?;
    }
    Ok(())
}

pub(crate) fn destroy_all_frame_targets(context: &mut BridgeContext) -> GalResult<()> {
    let active_targets = context
        .frame_targets
        .values()
        .map(|cached| cached.handle)
        .collect::<Vec<_>>();
    if !active_targets.is_empty() {
        context
            .gui_frontend
            .clear_frame_passes_for_targets(&mut context.gal, &active_targets);
        context
            .world_primitive_frontend
            .clear_frame_passes_for_targets(&mut context.gal, &active_targets);
    } else {
        context.gui_frontend.clear_frame_pass(&mut context.gal);
        context
            .world_primitive_frontend
            .clear_frame_pass(&mut context.gal);
    }
    destroy_stale_frame_targets(context)?;
    for (_identity, cached) in std::mem::take(&mut context.frame_targets) {
        context.gal.destroy(cached.handle)?;
    }
    Ok(())
}

#[derive(Default)]
pub(crate) struct BridgeRegistry {
    pub(crate) next_context_id: u64,
    pub(crate) contexts: BTreeMap<u64, BridgeContext>,
    pub(crate) last_error: String,
    pub(crate) windowed_presenter_active: bool,
}

impl Drop for BridgeRegistry {
    fn drop(&mut self) {
        // The registry is thread-local, so a thread can terminate with live
        // contexts if its owner did not reach the explicit destroy seam. Do
        // not strand the process-wide admission counters in that case.
        let live = self.contexts.len();
        let presenter_active = self.windowed_presenter_active;
        // Drop the owning GALs before releasing their admission slots. This
        // keeps the process-wide bound meaningful during teardown: a new
        // context cannot be admitted while the old native resources are still
        // being destroyed as thread-local fields unwind.
        self.contexts.clear();
        if live != 0 {
            let result =
                LIVE_BRIDGE_CONTEXTS.fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                    count.checked_sub(live)
                });
            debug_assert!(
                result.is_ok(),
                "Rust VulkanicGAL context slots underflow during registry drop"
            );
        }
        if presenter_active {
            WINDOWED_PRESENTER_ACTIVE.store(false, Ordering::Release);
        }
    }
}

thread_local! {
    static BRIDGE_REGISTRY: RefCell<BridgeRegistry> = RefCell::new(BridgeRegistry {
        next_context_id: 1,
        contexts: BTreeMap::new(),
        last_error: String::new(),
        windowed_presenter_active: false,
    });
}

pub(crate) fn with_registry_mut<T>(f: impl FnOnce(&mut BridgeRegistry) -> T) -> T {
    BRIDGE_REGISTRY.with(|registry| f(&mut registry.borrow_mut()))
}

pub(crate) fn with_registry<T>(f: impl FnOnce(&BridgeRegistry) -> T) -> T {
    BRIDGE_REGISTRY.with(|registry| f(&registry.borrow()))
}

fn reserve_windowed_presenter(registry: &mut BridgeRegistry) -> GalResult<()> {
    if registry.windowed_presenter_active
        || WINDOWED_PRESENTER_ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
    {
        return Err(GalError::ffi(
            StatusCode::InvalidArgument,
            "a Rust Vulkan windowed presenter is already active",
        ));
    }
    registry.windowed_presenter_active = true;
    Ok(())
}

fn release_windowed_presenter(registry: &mut BridgeRegistry) {
    registry.windowed_presenter_active = false;
    WINDOWED_PRESENTER_ACTIVE.store(false, Ordering::Release);
}

fn reserve_context_slot() -> GalResult<()> {
    LIVE_BRIDGE_CONTEXTS
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |live| {
            (live < MAX_BRIDGE_CONTEXTS).then_some(live + 1)
        })
        .map(|_| ())
        .map_err(|_| {
            GalError::ffi(
                StatusCode::InvalidArgument,
                format!("live Rust VulkanicGAL context limit exceeded ({MAX_BRIDGE_CONTEXTS})"),
            )
        })
}

fn release_context_slot() {
    let result = LIVE_BRIDGE_CONTEXTS.fetch_update(Ordering::AcqRel, Ordering::Acquire, |live| {
        live.checked_sub(1)
    });
    debug_assert!(result.is_ok(), "Rust VulkanicGAL context slot underflow");
}

fn ensure_context_capacity() -> GalResult<()> {
    with_registry(|registry| {
        if registry.contexts.len() >= MAX_BRIDGE_CONTEXTS {
            Err(GalError::ffi(
                StatusCode::InvalidArgument,
                format!("live Rust VulkanicGAL context limit exceeded ({MAX_BRIDGE_CONTEXTS})"),
            ))
        } else {
            Ok(())
        }
    })
}

fn backend_kind(raw: u32) -> GalResult<BackendKind> {
    match raw {
        1 => Ok(BackendKind::Vulkan),
        2 => Ok(BackendKind::OpenGl),
        _ => Err(GalError::ffi(
            StatusCode::UnknownEnum,
            format!("unknown Rust VulkanicGAL backend kind {raw}"),
        )),
    }
}
#[no_mangle]
pub unsafe extern "C" fn mattmc_vulkanic_gal_context_create(
    request: *const FfiContextCreateRequest,
    out: *mut FfiContextResult,
) -> i32 {
    let result = (|| -> GalResult<FfiContextResult> {
        let request = read_struct(request, "context create request")?;
        validate_header::<FfiContextCreateRequest>(request.header)?;
        ensure_context_capacity()?;
        let kind = backend_kind(request.backend_kind)?;
        let label = read_label(request.label, "context label")?;
        let backend = create_backend(kind, &label)?;
        let gal = VulkanicGal::new_with_backend(
            backend,
            bool_flag(request.tracy_enabled, "tracy enabled")?,
        );
        let capabilities = gal.capabilities();
        let context_id = with_registry_mut(|registry| -> GalResult<u64> {
            if registry.contexts.len() >= MAX_BRIDGE_CONTEXTS {
                return Err(GalError::ffi(
                    StatusCode::InvalidArgument,
                    format!("live Rust VulkanicGAL context limit exceeded ({MAX_BRIDGE_CONTEXTS})"),
                ));
            }
            let context_id = registry.next_context_id;
            registry.next_context_id =
                registry.next_context_id.checked_add(1).ok_or_else(|| {
                    GalError::ffi(
                        StatusCode::GenerationExhausted,
                        "context id space exhausted",
                    )
                })?;
            reserve_context_slot()?;
            registry.contexts.insert(
                context_id,
                BridgeContext {
                    gal,
                    windowed_presenter: false,
                    gui_frontend: GuiFrontend::default(),
                    world_primitive_frontend: WorldPrimitiveFrontend::default(),
                    ffi_calls: 1,
                    ffi_input_bytes: size_of::<FfiContextCreateRequest>() as u64,
                    ffi_output_bytes: size_of::<FfiContextResult>() as u64,
                    last_error: String::new(),
                    frame_targets: BTreeMap::new(),
                    stale_frame_targets: Vec::new(),
                },
            );
            Ok(context_id)
        })?;
        Ok(FfiContextResult {
            context_id,
            supported_feature_bits: capability_feature_bits(capabilities),
            limits: capabilities.limits.into(),
            metrics: FfiMetricsSnapshot {
                ffi_calls: 1,
                ffi_input_bytes: size_of::<FfiContextCreateRequest>() as u64,
                ffi_output_bytes: size_of::<FfiContextResult>() as u64,
                ..FfiMetricsSnapshot::default()
            },
            ..FfiContextResult::default()
        })
    })();
    match result {
        Ok(value) => {
            write_context_out(out, value);
            StatusCode::Ok as i32
        }
        Err(error) => {
            with_registry_mut(|registry| {
                registry.last_error = error.to_string();
            });
            write_context_out(
                out,
                FfiContextResult {
                    status: error.code as i32,
                    error_domain: error.domain as u32,
                    ..FfiContextResult::default()
                },
            );
            error.code as i32
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn mattmc_vulkanic_gal_context_create_borrowed_opengl(
    request: *const FfiBorrowedOpenGlContextCreateRequest,
    out: *mut FfiContextResult,
) -> i32 {
    let result = (|| -> GalResult<FfiContextResult> {
        let request = read_struct(request, "borrowed OpenGL context create request")?;
        validate_header::<FfiBorrowedOpenGlContextCreateRequest>(request.header)?;
        ensure_context_capacity()?;
        if request.stable_window_id == 0 {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "borrowed OpenGL context requires a stable non-zero window id",
            ));
        }
        let label = read_label(request.label, "borrowed OpenGL context label")?;
        let backend = create_borrowed_opengl_backend(&label, request.stable_window_id)?;
        let gal = VulkanicGal::new_with_backend(
            backend,
            bool_flag(request.tracy_enabled, "tracy enabled")?,
        );
        let capabilities = gal.capabilities();
        let context_id = with_registry_mut(|registry| -> GalResult<u64> {
            if registry.contexts.len() >= MAX_BRIDGE_CONTEXTS {
                return Err(GalError::ffi(
                    StatusCode::InvalidArgument,
                    format!("live Rust VulkanicGAL context limit exceeded ({MAX_BRIDGE_CONTEXTS})"),
                ));
            }
            let context_id = registry.next_context_id;
            registry.next_context_id =
                registry.next_context_id.checked_add(1).ok_or_else(|| {
                    GalError::ffi(
                        StatusCode::GenerationExhausted,
                        "context id space exhausted",
                    )
                })?;
            reserve_context_slot()?;
            registry.contexts.insert(
                context_id,
                BridgeContext {
                    gal,
                    windowed_presenter: false,
                    gui_frontend: GuiFrontend::default(),
                    world_primitive_frontend: WorldPrimitiveFrontend::default(),
                    ffi_calls: 1,
                    ffi_input_bytes: size_of::<FfiBorrowedOpenGlContextCreateRequest>() as u64,
                    ffi_output_bytes: size_of::<FfiContextResult>() as u64,
                    last_error: String::new(),
                    frame_targets: BTreeMap::new(),
                    stale_frame_targets: Vec::new(),
                },
            );
            Ok(context_id)
        })?;
        Ok(FfiContextResult {
            context_id,
            supported_feature_bits: capability_feature_bits(capabilities),
            limits: capabilities.limits.into(),
            metrics: FfiMetricsSnapshot {
                ffi_calls: 1,
                ffi_input_bytes: size_of::<FfiBorrowedOpenGlContextCreateRequest>() as u64,
                ffi_output_bytes: size_of::<FfiContextResult>() as u64,
                ..FfiMetricsSnapshot::default()
            },
            ..FfiContextResult::default()
        })
    })();
    match result {
        Ok(value) => {
            write_context_out(out, value);
            StatusCode::Ok as i32
        }
        Err(error) => {
            with_registry_mut(|registry| {
                registry.last_error = error.to_string();
            });
            write_context_out(
                out,
                FfiContextResult {
                    status: error.code as i32,
                    error_domain: error.domain as u32,
                    ..FfiContextResult::default()
                },
            );
            error.code as i32
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn mattmc_vulkanic_gal_context_create_windowed_vulkan(
    request: *const FfiWindowedVulkanContextCreateRequest,
    out: *mut FfiContextResult,
) -> i32 {
    let result = (|| -> GalResult<FfiContextResult> {
        let request = read_struct(request, "windowed Vulkan context create request")?;
        validate_header::<FfiWindowedVulkanContextCreateRequest>(request.header)?;
        ensure_context_capacity()?;
        if request.stable_window_id == 0 {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "windowed Vulkan context requires a stable non-zero window id",
            ));
        }
        if request.native_display == 0 || request.native_window == 0 {
            return Err(GalError::ffi(
                StatusCode::InvalidArgument,
                "windowed Vulkan context requires non-zero native display and window handles",
            ));
        }
        let label = read_label(request.label, "windowed Vulkan context label")?;
        let surface_label = read_label(request.surface_label, "windowed Vulkan surface label")?;
        let surface_desc = FrameSurfaceDesc {
            label: surface_label,
            extent: request.extent.into(),
            color_format: texture_format(request.color_format)?,
            present_mode: present_mode(request.present_mode)?,
            max_frames_in_flight: request.max_frames_in_flight,
        };
        let backend = create_native_windowed_vulkan_backend(
            &label,
            request.platform,
            request.stable_window_id,
            request.native_display,
            request.native_window,
            surface_desc,
        )?;
        let gal = VulkanicGal::new_with_backend(
            backend,
            bool_flag(request.tracy_enabled, "tracy enabled")?,
        );
        let capabilities = gal.capabilities();
        let context_id = with_registry_mut(|registry| -> GalResult<u64> {
            if registry.contexts.len() >= MAX_BRIDGE_CONTEXTS {
                return Err(GalError::ffi(
                    StatusCode::InvalidArgument,
                    format!("live Rust VulkanicGAL context limit exceeded ({MAX_BRIDGE_CONTEXTS})"),
                ));
            }
            let context_id = registry.next_context_id;
            registry.next_context_id =
                registry.next_context_id.checked_add(1).ok_or_else(|| {
                    GalError::ffi(
                        StatusCode::GenerationExhausted,
                        "context id space exhausted",
                    )
                })?;
            reserve_context_slot()?;
            if let Err(error) = reserve_windowed_presenter(registry) {
                release_context_slot();
                return Err(error);
            }
            registry.contexts.insert(
                context_id,
                BridgeContext {
                    gal,
                    windowed_presenter: true,
                    gui_frontend: GuiFrontend::default(),
                    world_primitive_frontend: WorldPrimitiveFrontend::default(),
                    ffi_calls: 1,
                    ffi_input_bytes: size_of::<FfiWindowedVulkanContextCreateRequest>() as u64,
                    ffi_output_bytes: size_of::<FfiContextResult>() as u64,
                    last_error: String::new(),
                    frame_targets: BTreeMap::new(),
                    stale_frame_targets: Vec::new(),
                },
            );
            Ok(context_id)
        })?;
        Ok(FfiContextResult {
            context_id,
            supported_feature_bits: capability_feature_bits(capabilities),
            limits: capabilities.limits.into(),
            metrics: FfiMetricsSnapshot {
                ffi_calls: 1,
                ffi_input_bytes: size_of::<FfiWindowedVulkanContextCreateRequest>() as u64,
                ffi_output_bytes: size_of::<FfiContextResult>() as u64,
                ..FfiMetricsSnapshot::default()
            },
            ..FfiContextResult::default()
        })
    })();
    match result {
        Ok(value) => {
            write_context_out(out, value);
            StatusCode::Ok as i32
        }
        Err(error) => {
            with_registry_mut(|registry| {
                registry.last_error = error.to_string();
            });
            write_context_out(
                out,
                FfiContextResult {
                    status: error.code as i32,
                    error_domain: error.domain as u32,
                    ..FfiContextResult::default()
                },
            );
            error.code as i32
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn mattmc_vulkanic_gal_context_destroy(
    context_id: u64,
    out: *mut FfiStatusResult,
) -> i32 {
    with_registry_mut(|registry| {
        let Some(context) = registry.contexts.get_mut(&context_id) else {
            let error = GalError::ffi(
                StatusCode::StaleHandle,
                format!("unknown context id {context_id}"),
            );
            write_status_out(out, status_result_from_error(&error));
            return error.code as i32;
        };
        if let Err(error) = context.gui_frontend.reset(&mut context.gal) {
            // Keep ownership and presenter reservation intact on dependency
            // failure, so the caller can release the dependent and retry.
            set_last_error(context, &error);
            write_status_out(out, status_error(Some(context), &error));
            return error.code as i32;
        }
        let mut context = registry.contexts.remove(&context_id).expect("context was checked above");
        if context.windowed_presenter {
            release_windowed_presenter(registry);
        }
        release_context_slot();
        context.world_primitive_frontend.reset(&mut context.gal);
        let mut status = status_ok(&context);
        status.metrics.ffi_calls = status.metrics.ffi_calls.saturating_add(1);
        status.metrics.ffi_output_bytes = status
            .metrics
            .ffi_output_bytes
            .saturating_add(size_of::<FfiStatusResult>() as u64);
        write_status_out(out, status);
        StatusCode::Ok as i32
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windowed_presenter_reservation_is_exclusive_and_releasable() {
        let mut registry = BridgeRegistry::default();
        assert!(reserve_windowed_presenter(&mut registry).is_ok());
        let second =
            reserve_windowed_presenter(&mut registry).expect_err("second presenter admitted");
        assert_eq!(second.code, StatusCode::InvalidArgument);
        release_windowed_presenter(&mut registry);
        assert!(reserve_windowed_presenter(&mut registry).is_ok());
    }

    #[test]
    fn registry_drop_releases_presenter_admission() {
        {
            let mut registry = BridgeRegistry::default();
            reserve_windowed_presenter(&mut registry).expect("first presenter reservation");
        }
        let mut replacement = BridgeRegistry::default();
        assert!(reserve_windowed_presenter(&mut replacement).is_ok());
        release_windowed_presenter(&mut replacement);
    }
}

#[no_mangle]
pub unsafe extern "C" fn mattmc_vulkanic_gal_capabilities(
    context_id: u64,
    request: *const FfiCapabilityQueryRequest,
    out: *mut FfiCapabilityResult,
) -> i32 {
    with_registry_mut(|registry| {
        let Some(context) = registry.contexts.get_mut(&context_id) else {
            return StatusCode::StaleHandle as i32;
        };
        context.ffi_calls += 1;
        context.ffi_input_bytes = context
            .ffi_input_bytes
            .saturating_add(size_of::<FfiCapabilityQueryRequest>() as u64);
        context.ffi_output_bytes = context
            .ffi_output_bytes
            .saturating_add(size_of::<FfiCapabilityResult>() as u64);
        match answer_capability_query(request, context.gal.capabilities()) {
            Ok(mut result) => {
                result.status = StatusCode::Ok as i32;
                let _ = write_out(out, result, "capability result");
                StatusCode::Ok as i32
            }
            Err(error) => {
                set_last_error(context, &error);
                let _ = write_out(
                    out,
                    FfiCapabilityResult {
                        status: error.code as i32,
                        error_domain: error.domain as u32,
                        supported_feature_bits: capability_feature_bits(context.gal.capabilities()),
                        limits: context.gal.capabilities().limits.into(),
                        ..FfiCapabilityResult {
                            header: FfiHeader {
                                version: FFI_ABI_VERSION,
                                byte_size: size_of::<FfiCapabilityResult>() as u32,
                            },
                            status: error.code as i32,
                            error_domain: error.domain as u32,
                            supported_feature_bits: 0,
                            negotiated_feature_bits: 0,
                            limits: FfiBackendLimits::default(),
                            initial_presentation_supported: 0,
                        }
                    },
                    "capability result",
                );
                error.code as i32
            }
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn mattmc_vulkanic_gal_last_error(
    context_id: u64,
    out_bytes: *mut u8,
    out_capacity: u64,
) -> u64 {
    with_registry(|registry| {
        let Some(context) = registry.contexts.get(&context_id) else {
            let bytes = registry.last_error.as_bytes();
            let copy_len = bytes.len().min(usize::try_from(out_capacity).unwrap_or(0));
            if copy_len > 0 && !out_bytes.is_null() {
                ptr::copy_nonoverlapping(bytes.as_ptr(), out_bytes, copy_len);
            }
            return bytes.len() as u64;
        };
        let bytes = context.last_error.as_bytes();
        let copy_len = bytes.len().min(usize::try_from(out_capacity).unwrap_or(0));
        if copy_len > 0 && !out_bytes.is_null() {
            ptr::copy_nonoverlapping(bytes.as_ptr(), out_bytes, copy_len);
        }
        bytes.len() as u64
    })
}
