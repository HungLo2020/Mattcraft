use super::*;

#[no_mangle]
pub unsafe extern "C" fn mattmc_vulkanic_gal_frame_configure(
    context_id: u64,
    request: *const FfiFrameSurfaceConfigRequest,
    status_out: *mut FfiStatusResult,
) -> i32 {
    with_registry_mut(|registry| {
        let Some(context) = registry.contexts.get_mut(&context_id) else {
            let error = GalError::ffi(
                StatusCode::StaleHandle,
                format!("unknown context id {context_id}"),
            );
            write_status_out(status_out, status_result_from_error(&error));
            return error.code as i32;
        };
        context.ffi_calls += 1;
        context.ffi_input_bytes = context
            .ffi_input_bytes
            .saturating_add(size_of::<FfiFrameSurfaceConfigRequest>() as u64);
        context.ffi_output_bytes = context
            .ffi_output_bytes
            .saturating_add(size_of::<FfiStatusResult>() as u64);
        let result = (|| -> GalResult<()> {
            let request = read_struct(request, "frame configure request")?;
            validate_header::<FfiFrameSurfaceConfigRequest>(request.header)?;
            let desc = FrameSurfaceDesc {
                label: read_label(request.label, "frame surface label")?,
                extent: request.extent.into(),
                color_format: texture_format(request.color_format)?,
                present_mode: present_mode(request.present_mode)?,
                max_frames_in_flight: request.max_frames_in_flight,
            };
            context.gui_frontend.clear_frame_pass(&mut context.gal);
            context
                .world_primitive_frontend
                .clear_frame_pass(&mut context.gal);
            destroy_all_frame_targets(context)?;
            context.gal.configure_frame_surface(desc)
        })();
        match result {
            Ok(()) => {
                write_status_out(status_out, status_ok(context));
                StatusCode::Ok as i32
            }
            Err(error) => {
                set_last_error(context, &error);
                write_status_out(status_out, status_error(Some(context), &error));
                error.code as i32
            }
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn mattmc_vulkanic_gal_frame_acquire(
    context_id: u64,
    request: *const FfiFrameAcquireRequest,
    out: *mut FfiFrameAcquireResult,
) -> i32 {
    with_registry_mut(|registry| {
        let Some(context) = registry.contexts.get_mut(&context_id) else {
            return StatusCode::StaleHandle as i32;
        };
        context.ffi_calls += 1;
        context.ffi_input_bytes = context
            .ffi_input_bytes
            .saturating_add(size_of::<FfiFrameAcquireRequest>() as u64);
        context.ffi_output_bytes = context
            .ffi_output_bytes
            .saturating_add(size_of::<FfiFrameAcquireResult>() as u64);
        let result = (|| -> GalResult<FfiFrameAcquireResult> {
            let request = read_struct(request, "frame acquire request")?;
            validate_header::<FfiFrameAcquireRequest>(request.header)?;
            let acquired = context.gal.acquire_frame(FrameAcquireDesc {
                correlation_id: FrameCorrelationId(request.correlation_id),
                expected_extent: request.expected_extent.into(),
            })?;
            let frame_target = if matches!(
                acquired.status,
                FrameAcquireStatus::Minimized | FrameAcquireStatus::Resized
            ) {
                Handle::NULL
            } else if let Some(cached) = context.frame_targets.get(&acquired.render_target) {
                cached.handle
            } else {
                let handle = context.gal.create_frame_target(FrameTargetDesc {
                    label: format!("ffi.frame-target.{}", acquired.frame.0),
                    frame_id: acquired.frame.0,
                    render_target: acquired.render_target,
                    extent: acquired.extent,
                    color_format: acquired.color_format,
                })?;
                context
                    .frame_targets
                    .insert(acquired.render_target, CachedFrameTarget { handle });
                handle
            };
            Ok(FfiFrameAcquireResult {
                status: StatusCode::Ok as i32,
                error_domain: 0,
                frame_id: acquired.frame.0,
                correlation_id: acquired.correlation_id.0,
                acquire_status: acquire_status_raw(acquired.status),
                frame_target: FfiHandle::from(frame_target),
                frame_target_identity: acquired.render_target.0,
                extent: acquired.extent.into(),
                color_format: acquired.color_format as u32,
                metrics: context_metrics(context),
                ..FfiFrameAcquireResult::default()
            })
        })();
        match result {
            Ok(value) => {
                let _ = write_out(out, value, "frame acquire result");
                StatusCode::Ok as i32
            }
            Err(error) => {
                set_last_error(context, &error);
                let _ = write_out(
                    out,
                    FfiFrameAcquireResult {
                        status: error.code as i32,
                        error_domain: error.domain as u32,
                        metrics: context_metrics(context),
                        ..FfiFrameAcquireResult::default()
                    },
                    "frame acquire result",
                );
                error.code as i32
            }
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn mattmc_vulkanic_gal_frame_resize(
    context_id: u64,
    request: *const FfiFrameResizeRequest,
    out: *mut FfiFrameResizeResult,
) -> i32 {
    with_registry_mut(|registry| {
        let Some(context) = registry.contexts.get_mut(&context_id) else {
            return StatusCode::StaleHandle as i32;
        };
        context.ffi_calls += 1;
        context.ffi_input_bytes = context
            .ffi_input_bytes
            .saturating_add(size_of::<FfiFrameResizeRequest>() as u64);
        context.ffi_output_bytes = context
            .ffi_output_bytes
            .saturating_add(size_of::<FfiFrameResizeResult>() as u64);
        let result = (|| -> GalResult<FfiFrameResizeResult> {
            let request = read_struct(request, "frame resize request")?;
            validate_header::<FfiFrameResizeRequest>(request.header)?;
            context.gui_frontend.clear_frame_pass(&mut context.gal);
            context
                .world_primitive_frontend
                .clear_frame_pass(&mut context.gal);
            destroy_all_frame_targets(context)?;
            let resized = context.gal.resize_frame_surface(FrameResizeDesc {
                correlation_id: FrameCorrelationId(request.correlation_id),
                extent: request.extent.into(),
            })?;
            Ok(FfiFrameResizeResult {
                status: StatusCode::Ok as i32,
                resize_status: acquire_status_raw(resized.status),
                extent: resized.extent.into(),
                ..FfiFrameResizeResult::default()
            })
        })();
        match result {
            Ok(value) => {
                let _ = write_out(out, value, "frame resize result");
                StatusCode::Ok as i32
            }
            Err(error) => {
                set_last_error(context, &error);
                let _ = write_out(
                    out,
                    FfiFrameResizeResult {
                        status: error.code as i32,
                        error_domain: error.domain as u32,
                        ..FfiFrameResizeResult::default()
                    },
                    "frame resize result",
                );
                error.code as i32
            }
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn mattmc_vulkanic_gal_frame_present(
    context_id: u64,
    request: *const FfiFramePresentRequest,
    out: *mut FfiFramePresentResult,
) -> i32 {
    with_registry_mut(|registry| {
        let Some(context) = registry.contexts.get_mut(&context_id) else {
            return StatusCode::StaleHandle as i32;
        };
        context.ffi_calls += 1;
        context.ffi_input_bytes = context
            .ffi_input_bytes
            .saturating_add(size_of::<FfiFramePresentRequest>() as u64);
        context.ffi_output_bytes = context
            .ffi_output_bytes
            .saturating_add(size_of::<FfiFramePresentResult>() as u64);
        let result = (|| -> GalResult<FfiFramePresentResult> {
            let request = read_struct(request, "frame present request")?;
            validate_header::<FfiFramePresentRequest>(request.header)?;
            let presented = context.gal.present_frame(PresentFrameDesc {
                frame: VulkanicFrameId(request.frame_id),
                correlation_id: FrameCorrelationId(request.correlation_id),
                wait_for: SubmissionId(request.wait_submission_id),
            })?;
            // Presentation has synchronized the submitted frame.  Retire the complete
            // Rust-owned submission prefix at this ownership boundary as well: resource
            // uploads and the whole-frame submission can be newer than the frame token,
            // and polling alone is allowed to lag behind the explicit timeline wait on
            // drivers.  Using the latest Rust submission keeps command buffers,
            // descriptor-backed resources, and deferred GAL destroys bounded without
            // introducing a second presenter or borrowing Java/Iris state.
            let latest_submission = context.gal.latest_submission_id();
            context.gal.retire_through(latest_submission)?;
            if std::env::var_os("MATTMC_TRACE_SUBMISSIONS").is_some() {
                println!(
                    "vulkan.submission.present-retire frame={} waited={} retired_through={}",
                    presented.frame.0, request.wait_submission_id, latest_submission.0,
                );
            }
            Ok(FfiFramePresentResult {
                status: StatusCode::Ok as i32,
                frame_id: presented.frame.0,
                correlation_id: presented.correlation_id.0,
                present_status: present_status_raw(presented.status),
                completed_submission_id: presented.completed_submission.0,
                frame_target_identity: presented.render_target.0,
                ..FfiFramePresentResult::default()
            })
        })();
        match result {
            Ok(value) => {
                let _ = write_out(out, value, "frame present result");
                StatusCode::Ok as i32
            }
            Err(error) => {
                set_last_error(context, &error);
                let _ = write_out(
                    out,
                    FfiFramePresentResult {
                        status: error.code as i32,
                        error_domain: error.domain as u32,
                        ..FfiFramePresentResult::default()
                    },
                    "frame present result",
                );
                error.code as i32
            }
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn mattmc_vulkanic_gal_frame_cancel(
    context_id: u64,
    request: *const FfiFrameCancelRequest,
    status_out: *mut FfiStatusResult,
) -> i32 {
    with_registry_mut(|registry| {
        let Some(context) = registry.contexts.get_mut(&context_id) else {
            let error = GalError::ffi(
                StatusCode::StaleHandle,
                format!("unknown context id {context_id}"),
            );
            write_status_out(status_out, status_result_from_error(&error));
            return error.code as i32;
        };
        context.ffi_calls += 1;
        context.ffi_input_bytes = context
            .ffi_input_bytes
            .saturating_add(size_of::<FfiFrameCancelRequest>() as u64);
        context.ffi_output_bytes = context
            .ffi_output_bytes
            .saturating_add(size_of::<FfiStatusResult>() as u64);
        let result = (|| -> GalResult<()> {
            let request = read_struct(request, "frame cancel request")?;
            validate_header::<FfiFrameCancelRequest>(request.header)?;
            context
                .gal
                .cancel_frame(crate::render::vulkanic::frame::FrameId(request.frame_id))?;
            context.gui_frontend.discard_prepared_post_effects(&mut context.gal);
            // The swapchain recreation waits for device quiescence, so all
            // cached frame-target wrappers are now safe to retire as well.
            destroy_all_frame_targets(context)
        })();
        match result {
            Ok(()) => {
                write_status_out(status_out, status_ok(context));
                StatusCode::Ok as i32
            }
            Err(error) => {
                set_last_error(context, &error);
                write_status_out(status_out, status_error(Some(context), &error));
                error.code as i32
            }
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn mattmc_vulkanic_gal_frame_shutdown(
    context_id: u64,
    status_out: *mut FfiStatusResult,
) -> i32 {
    with_registry_mut(|registry| {
        let Some(context) = registry.contexts.get_mut(&context_id) else {
            let error = GalError::ffi(
                StatusCode::StaleHandle,
                format!("unknown context id {context_id}"),
            );
            write_status_out(status_out, status_result_from_error(&error));
            return error.code as i32;
        };
        context.ffi_calls += 1;
        context.ffi_output_bytes = context
            .ffi_output_bytes
            .saturating_add(size_of::<FfiStatusResult>() as u64);
        if let Err(error) = context.gui_frontend.reset(&mut context.gal) {
            set_last_error(context, &error);
            write_status_out(status_out, status_error(Some(context), &error));
            return error.code as i32;
        }
        context.world_primitive_frontend.reset(&mut context.gal);
        if let Err(error) = destroy_all_frame_targets(context) {
            set_last_error(context, &error);
            write_status_out(status_out, status_error(Some(context), &error));
            return error.code as i32;
        }
        match context.gal.shutdown_frame_surface() {
            Ok(()) => {
                write_status_out(status_out, status_ok(context));
                StatusCode::Ok as i32
            }
            Err(error) => {
                set_last_error(context, &error);
                write_status_out(status_out, status_error(Some(context), &error));
                error.code as i32
            }
        }
    })
}
