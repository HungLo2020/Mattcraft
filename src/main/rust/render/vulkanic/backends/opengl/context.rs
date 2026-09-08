use std::ffi::{c_char, c_int, c_ulong, c_void, CString};
use std::num::NonZeroU32;
use std::ptr;
use std::rc::Rc;
use std::thread::ThreadId;

use glow::HasContext;
use libloading::Library;

use super::trace;
use crate::render::vulkanic::error::{GalError, GalResult};

const EGL_FALSE: c_int = 0;
const EGL_NONE: c_int = 0x3038;
const EGL_NO_CONTEXT: EglContextHandle = ptr::null_mut();
const EGL_NO_DISPLAY: EglDisplay = ptr::null_mut();
const EGL_NO_SURFACE: EglSurface = ptr::null_mut();
const EGL_WIDTH: c_int = 0x3057;
const EGL_HEIGHT: c_int = 0x3056;
const EGL_SURFACE_TYPE: c_int = 0x3033;
const EGL_PBUFFER_BIT: c_int = 0x0001;
const EGL_RED_SIZE: c_int = 0x3024;
const EGL_GREEN_SIZE: c_int = 0x3023;
const EGL_BLUE_SIZE: c_int = 0x3022;
const EGL_ALPHA_SIZE: c_int = 0x3021;
const EGL_DEPTH_SIZE: c_int = 0x3025;
const EGL_RENDERABLE_TYPE: c_int = 0x3040;
const EGL_OPENGL_BIT: c_int = 0x0008;
const EGL_OPENGL_API: c_int = 0x30A2;
const EGL_EXTENSIONS: c_int = 0x3055;
const EGL_PLATFORM_SURFACELESS_MESA: c_int = 0x31DD;
const EGL_CONTEXT_MAJOR_VERSION: c_int = 0x3098;
const EGL_CONTEXT_MINOR_VERSION: c_int = 0x30FB;
const EGL_CONTEXT_OPENGL_PROFILE_MASK: c_int = 0x30FD;
const EGL_CONTEXT_OPENGL_CORE_PROFILE_BIT: c_int = 0x0001;

type EglDisplay = *mut c_void;
type EglConfig = *mut c_void;
type EglSurface = *mut c_void;
type EglContextHandle = *mut c_void;
type EglBoolean = c_int;
type EglInt = c_int;

type EglGetDisplay = unsafe extern "C" fn(*mut c_void) -> EglDisplay;
type EglInitialize = unsafe extern "C" fn(EglDisplay, *mut EglInt, *mut EglInt) -> EglBoolean;
type EglChooseConfig = unsafe extern "C" fn(
    EglDisplay,
    *const EglInt,
    *mut EglConfig,
    EglInt,
    *mut EglInt,
) -> EglBoolean;
type EglBindApi = unsafe extern "C" fn(EglInt) -> EglBoolean;
type EglCreatePbufferSurface =
    unsafe extern "C" fn(EglDisplay, EglConfig, *const EglInt) -> EglSurface;
type EglCreateContext = unsafe extern "C" fn(
    EglDisplay,
    EglConfig,
    EglContextHandle,
    *const EglInt,
) -> EglContextHandle;
type EglMakeCurrent =
    unsafe extern "C" fn(EglDisplay, EglSurface, EglSurface, EglContextHandle) -> EglBoolean;
type EglDestroySurface = unsafe extern "C" fn(EglDisplay, EglSurface) -> EglBoolean;
type EglDestroyContext = unsafe extern "C" fn(EglDisplay, EglContextHandle) -> EglBoolean;
type EglTerminate = unsafe extern "C" fn(EglDisplay) -> EglBoolean;
type EglGetProcAddress = unsafe extern "C" fn(*const c_char) -> *const c_void;
type EglGetError = unsafe extern "C" fn() -> EglInt;
type EglQueryString = unsafe extern "C" fn(EglDisplay, EglInt) -> *const c_char;
type EglGetPlatformDisplayExt =
    unsafe extern "C" fn(EglInt, *mut c_void, *const EglInt) -> EglDisplay;

const GLX_RGBA_BIT: c_int = 0x0000_0001;
const GLX_PBUFFER_BIT: c_int = 0x0000_0004;
const GLX_DOUBLEBUFFER: c_int = 5;
const GLX_RED_SIZE: c_int = 8;
const GLX_GREEN_SIZE: c_int = 9;
const GLX_BLUE_SIZE: c_int = 10;
const GLX_ALPHA_SIZE: c_int = 11;
const GLX_DEPTH_SIZE: c_int = 12;
const GLX_DRAWABLE_TYPE: c_int = 0x8010;
const GLX_RENDER_TYPE: c_int = 0x8011;
const GLX_X_RENDERABLE: c_int = 0x8012;
const GLX_RGBA_TYPE: c_int = 0x8014;
const GLX_PBUFFER_HEIGHT: c_int = 0x8040;
const GLX_PBUFFER_WIDTH: c_int = 0x8041;

type XDisplay = *mut c_void;
type GlxFbConfig = *mut c_void;
type GlxContextHandle = *mut c_void;
type GlxDrawable = c_ulong;
type GlxPbuffer = c_ulong;

type XOpenDisplay = unsafe extern "C" fn(*const c_char) -> XDisplay;
type XDefaultScreen = unsafe extern "C" fn(XDisplay) -> c_int;
type XCloseDisplay = unsafe extern "C" fn(XDisplay) -> c_int;
type XFree = unsafe extern "C" fn(*mut c_void) -> c_int;
type GlxChooseFbConfig =
    unsafe extern "C" fn(XDisplay, c_int, *const c_int, *mut c_int) -> *mut GlxFbConfig;
type GlxCreateNewContext =
    unsafe extern "C" fn(XDisplay, GlxFbConfig, c_int, GlxContextHandle, c_int) -> GlxContextHandle;
type GlxCreatePbuffer = unsafe extern "C" fn(XDisplay, GlxFbConfig, *const c_int) -> GlxPbuffer;
type GlxMakeContextCurrent =
    unsafe extern "C" fn(XDisplay, GlxDrawable, GlxDrawable, GlxContextHandle) -> c_int;
type GlxDestroyPbuffer = unsafe extern "C" fn(XDisplay, GlxPbuffer);
type GlxDestroyContext = unsafe extern "C" fn(XDisplay, GlxContextHandle);
type GlxGetProcAddress = unsafe extern "C" fn(*const u8) -> *const c_void;

struct EglFns {
    _library: Library,
    get_display: EglGetDisplay,
    initialize: EglInitialize,
    choose_config: EglChooseConfig,
    bind_api: EglBindApi,
    create_pbuffer_surface: EglCreatePbufferSurface,
    create_context: EglCreateContext,
    make_current: EglMakeCurrent,
    destroy_surface: EglDestroySurface,
    destroy_context: EglDestroyContext,
    terminate: EglTerminate,
    get_proc_address: EglGetProcAddress,
    get_error: EglGetError,
    query_string: EglQueryString,
    get_platform_display_ext: Option<EglGetPlatformDisplayExt>,
}

pub(super) struct OpenGlContext {
    native: NativeOpenGlContext,
    gl: Rc<glow::Context>,
    _gl_library: Option<Library>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::render::vulkanic::backends) struct ExistingOpenGlContextDesc {
    pub(super) label: String,
    pub(super) stable_window_id: u64,
    pub(super) render_thread: ThreadId,
}

enum NativeOpenGlContext {
    Egl {
        egl: EglFns,
        display: EglDisplay,
        surface: EglSurface,
        context: EglContextHandle,
    },
    Glx(GlxContextState),
    Existing {
        _stable_window_id: u64,
        render_thread: ThreadId,
    },
}

impl OpenGlContext {
    pub(super) fn new(label: &str) -> GalResult<Self> {
        let gl_library = unsafe { Library::new("libGL.so.1") }.ok();
        let native = create_native_context()?;
        let gl = unsafe {
            glow::Context::from_loader_function(|name| {
                let cname = CString::new(name).expect("GL symbol names do not contain NUL");
                let ptr = native.get_proc_address(cname.as_ptr());
                if !ptr.is_null() {
                    return ptr.cast();
                }
                gl_library
                    .as_ref()
                    .and_then(|library| {
                        library.get::<*const c_void>(cname.as_bytes_with_nul()).ok()
                    })
                    .map(|symbol| *symbol)
                    .unwrap_or(ptr::null())
                    .cast()
            })
        };
        let gl = Rc::new(gl);
        unsafe {
            if gl.supports_debug() {
                gl.debug_message_insert(
                    glow::DEBUG_SOURCE_APPLICATION,
                    glow::DEBUG_TYPE_MARKER,
                    1,
                    glow::DEBUG_SEVERITY_NOTIFICATION,
                    &format!("MattMC OpenGL VulkanicGAL context: {label}"),
                );
            }
        }

        Ok(Self {
            native,
            gl,
            _gl_library: gl_library,
        })
    }

    /// Native texture limits remain a backend detail. The GAL consumes the
    /// resulting conservative semantic limits through `BackendCapabilities`.
    pub(super) fn texture_extent_limits(&self) -> (u32, u32) {
        unsafe {
            let max_2d = self.gl.get_parameter_i32(glow::MAX_TEXTURE_SIZE).max(1) as u32;
            let max_3d = self.gl.get_parameter_i32(glow::MAX_3D_TEXTURE_SIZE).max(0) as u32;
            (max_2d, max_3d)
        }
    }

    pub(super) fn from_existing_context(desc: ExistingOpenGlContextDesc) -> GalResult<Self> {
        if std::thread::current().id() != desc.render_thread {
            return Err(GalError::backend(
                "existing OpenGL context must be registered on the render thread",
            ));
        }
        let gl_library = unsafe { Library::new("libGL.so.1") }
            .map_err(|error| GalError::backend(format!("failed to load libGL.so.1: {error}")))?;
        let get_proc_address = unsafe {
            gl_library
                .get::<GlxGetProcAddress>(b"glXGetProcAddress\0")
                .ok()
                .map(|symbol| *symbol)
        };
        let gl = unsafe {
            glow::Context::from_loader_function(|name| {
                let cname = CString::new(name).expect("GL symbol names do not contain NUL");
                if let Some(get_proc_address) = get_proc_address {
                    let ptr = get_proc_address(cname.as_ptr().cast::<u8>());
                    if !ptr.is_null() {
                        return ptr.cast();
                    }
                }
                gl_library
                    .get::<*const c_void>(cname.as_bytes_with_nul())
                    .ok()
                    .map(|symbol| *symbol)
                    .unwrap_or(ptr::null())
                    .cast()
            })
        };
        let gl = Rc::new(gl);
        unsafe {
            if gl.supports_debug() {
                gl.debug_message_insert(
                    glow::DEBUG_SOURCE_APPLICATION,
                    glow::DEBUG_TYPE_MARKER,
                    2,
                    glow::DEBUG_SEVERITY_NOTIFICATION,
                    &format!(
                        "MattMC OpenGL VulkanicGAL borrowed context: {} window={}",
                        desc.label, desc.stable_window_id
                    ),
                );
            }
        }
        Ok(Self {
            native: NativeOpenGlContext::Existing {
                _stable_window_id: desc.stable_window_id,
                render_thread: desc.render_thread,
            },
            gl,
            _gl_library: Some(gl_library),
        })
    }

    pub(super) fn gl(&self) -> &Rc<glow::Context> {
        &self.gl
    }

    pub(super) fn borrowed_state_guard(&self) -> Option<BorrowedOpenGlStateGuard> {
        self.borrowed_state_guard_with_images(false)
    }

    pub(super) fn borrowed_state_guard_with_images(
        &self,
        capture_images: bool,
    ) -> Option<BorrowedOpenGlStateGuard> {
        self.native
            .is_existing()
            .then(|| BorrowedOpenGlStateGuard::capture(self.gl.clone(), capture_images))
    }

    pub(super) fn supports_storage_textures(&self) -> bool {
        supports_storage_textures(&self.gl)
    }

    pub(super) fn supports_compute_shaders(&self) -> bool {
        supports_compute_shaders(&self.gl)
    }

    pub(super) fn make_current(&self) -> GalResult<()> {
        self.native.make_current()
    }

    pub(super) fn current_draw_framebuffer(&self) -> Option<glow::Framebuffer> {
        unsafe { framebuffer_name(self.gl.get_parameter_i32(glow::DRAW_FRAMEBUFFER_BINDING)) }
    }
}

impl Drop for OpenGlContext {
    fn drop(&mut self) {
        self.native.destroy();
    }
}

impl NativeOpenGlContext {
    fn make_current(&self) -> GalResult<()> {
        match self {
            NativeOpenGlContext::Egl {
                egl,
                display,
                surface,
                context,
            } => {
                if unsafe { (egl.make_current)(*display, *surface, *surface, *context) }
                    == EGL_FALSE
                {
                    return Err(GalError::backend(egl_error(egl, "EGL make-current failed")));
                }
                Ok(())
            }
            NativeOpenGlContext::Glx(state) => state.make_current(),
            NativeOpenGlContext::Existing { render_thread, .. } => {
                if std::thread::current().id() != *render_thread {
                    return Err(GalError::backend(
                        "borrowed OpenGL context used from a non-render thread",
                    ));
                }
                Ok(())
            }
        }
    }

    fn is_existing(&self) -> bool {
        matches!(self, NativeOpenGlContext::Existing { .. })
    }

    fn get_proc_address(&self, name: *const c_char) -> *const c_void {
        match self {
            NativeOpenGlContext::Egl { egl, .. } => unsafe { (egl.get_proc_address)(name) },
            NativeOpenGlContext::Glx(state) => unsafe {
                (state.fns.get_proc_address)(name.cast::<u8>())
            },
            NativeOpenGlContext::Existing { .. } => ptr::null(),
        }
    }

    fn destroy(&mut self) {
        match self {
            NativeOpenGlContext::Egl {
                egl,
                display,
                surface,
                context,
            } => unsafe {
                (egl.make_current)(*display, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
                (egl.destroy_context)(*display, *context);
                if !surface.is_null() {
                    (egl.destroy_surface)(*display, *surface);
                }
                (egl.terminate)(*display);
            },
            NativeOpenGlContext::Glx(state) => state.destroy(),
            NativeOpenGlContext::Existing { .. } => {}
        }
    }
}

pub(super) struct BorrowedOpenGlStateGuard {
    gl: Rc<glow::Context>,
    program: Option<glow::NativeProgram>,
    vertex_array: Option<glow::NativeVertexArray>,
    array_buffer: Option<glow::NativeBuffer>,
    element_array_buffer: Option<glow::NativeBuffer>,
    copy_read_buffer: Option<glow::NativeBuffer>,
    copy_write_buffer: Option<glow::NativeBuffer>,
    pixel_unpack_buffer: Option<glow::NativeBuffer>,
    pixel_pack_buffer: Option<glow::NativeBuffer>,
    uniform_buffer: Option<glow::NativeBuffer>,
    storage_buffer: Option<glow::NativeBuffer>,
    indexed_uniform_buffers: Vec<IndexedBufferBinding>,
    indexed_storage_buffers: Vec<IndexedBufferBinding>,
    draw_framebuffer: Option<glow::NativeFramebuffer>,
    read_framebuffer: Option<glow::NativeFramebuffer>,
    active_texture: i32,
    texture_units: Vec<TextureUnitState>,
    image_units: Vec<ImageUnitState>,
    viewport: [i32; 4],
    scissor_box: [i32; 4],
    scissor_enabled: bool,
    cull_enabled: bool,
    cull_face: i32,
    blend_enabled: bool,
    blend_src_rgb: i32,
    blend_dst_rgb: i32,
    blend_src_alpha: i32,
    blend_dst_alpha: i32,
    blend_equation_rgb: i32,
    blend_equation_alpha: i32,
    blend_color: [f32; 4],
    color_writemask: [bool; 4],
    front_face: i32,
    stencil_enabled: bool,
    stencil_front: StencilFaceState,
    stencil_back: StencilFaceState,
    unpack_alignment: i32,
    unpack_row_length: i32,
    unpack_skip_rows: i32,
    unpack_skip_pixels: i32,
    unpack_image_height: i32,
    pack_alignment: i32,
    pack_row_length: i32,
    pack_skip_rows: i32,
    pack_skip_pixels: i32,
    pack_image_height: i32,
    depth_enabled: bool,
    depth_func: i32,
    depth_writemask: bool,
    depth_range: [f32; 2],
    line_width: f32,
    polygon_mode: [i32; 2],
    polygon_offset_fill_enabled: bool,
    polygon_offset_factor: f32,
    polygon_offset_units: f32,
    primitive_restart_enabled: bool,
    rasterizer_discard_enabled: bool,
    dither_enabled: bool,
    multisample_enabled: bool,
}

struct TextureUnitState {
    unit: u32,
    texture_2d: Option<glow::NativeTexture>,
    texture_3d: Option<glow::NativeTexture>,
    sampler: Option<glow::NativeSampler>,
}

struct ImageUnitState {
    unit: u32,
    texture: Option<glow::NativeTexture>,
    level: i32,
    layered: bool,
    layer: i32,
    access: u32,
    format: u32,
}

struct IndexedBufferBinding {
    index: u32,
    buffer: Option<glow::NativeBuffer>,
    start: i64,
    size: i64,
}

struct StencilFaceState {
    func: i32,
    reference: i32,
    value_mask: i32,
    write_mask: i32,
    fail: i32,
    pass_depth_fail: i32,
    pass_depth_pass: i32,
}

impl BorrowedOpenGlStateGuard {
    fn capture(gl: Rc<glow::Context>, capture_images: bool) -> Self {
        let _zone = trace::Zone::new("opengl.borrowed-state.capture");
        unsafe {
            let active_texture = gl.get_parameter_i32(glow::ACTIVE_TEXTURE);
            let mut texture_units = Vec::new();
            let texture_unit_count = gl
                .get_parameter_i32(glow::MAX_COMBINED_TEXTURE_IMAGE_UNITS)
                .max(1);
            for unit in 0..u32::try_from(texture_unit_count).unwrap_or(1) {
                gl.active_texture(glow::TEXTURE0 + unit);
                texture_units.push(TextureUnitState {
                    unit,
                    texture_2d: texture_name(gl.get_parameter_i32(glow::TEXTURE_BINDING_2D)),
                    texture_3d: texture_name(gl.get_parameter_i32(glow::TEXTURE_BINDING_3D)),
                    sampler: sampler_name(gl.get_parameter_i32(glow::SAMPLER_BINDING)),
                });
            }
            gl.active_texture(active_texture as u32);

            let image_units = if capture_images && supports_storage_textures(&gl) {
                let image_unit_count = gl.get_parameter_i32(glow::MAX_IMAGE_UNITS).max(0);
                (0..u32::try_from(image_unit_count).unwrap_or(0))
                    .map(|unit| ImageUnitState {
                        unit,
                        texture: texture_name(
                            gl.get_parameter_indexed_i32(glow::IMAGE_BINDING_NAME, unit),
                        ),
                        level: gl.get_parameter_indexed_i32(glow::IMAGE_BINDING_LEVEL, unit),
                        layered: gl.get_parameter_indexed_i32(glow::IMAGE_BINDING_LAYERED, unit)
                            != 0,
                        layer: gl.get_parameter_indexed_i32(glow::IMAGE_BINDING_LAYER, unit),
                        access: gl.get_parameter_indexed_i32(glow::IMAGE_BINDING_ACCESS, unit)
                            as u32,
                        format: gl.get_parameter_indexed_i32(glow::IMAGE_BINDING_FORMAT, unit)
                            as u32,
                    })
                    .collect()
            } else {
                Vec::new()
            };

            Self {
                program: program_name(gl.get_parameter_i32(glow::CURRENT_PROGRAM)),
                vertex_array: vertex_array_name(gl.get_parameter_i32(glow::VERTEX_ARRAY_BINDING)),
                array_buffer: buffer_name(gl.get_parameter_i32(glow::ARRAY_BUFFER_BINDING)),
                element_array_buffer: buffer_name(
                    gl.get_parameter_i32(glow::ELEMENT_ARRAY_BUFFER_BINDING),
                ),
                copy_read_buffer: buffer_name(gl.get_parameter_i32(glow::COPY_READ_BUFFER_BINDING)),
                copy_write_buffer: buffer_name(
                    gl.get_parameter_i32(glow::COPY_WRITE_BUFFER_BINDING),
                ),
                pixel_unpack_buffer: buffer_name(
                    gl.get_parameter_i32(glow::PIXEL_UNPACK_BUFFER_BINDING),
                ),
                pixel_pack_buffer: buffer_name(
                    gl.get_parameter_i32(glow::PIXEL_PACK_BUFFER_BINDING),
                ),
                uniform_buffer: buffer_name(gl.get_parameter_i32(glow::UNIFORM_BUFFER_BINDING)),
                storage_buffer: buffer_name(
                    gl.get_parameter_i32(glow::SHADER_STORAGE_BUFFER_BINDING),
                ),
                indexed_uniform_buffers: indexed_buffer_bindings(
                    &gl,
                    glow::MAX_UNIFORM_BUFFER_BINDINGS,
                    glow::UNIFORM_BUFFER_BINDING,
                    glow::UNIFORM_BUFFER_START,
                    glow::UNIFORM_BUFFER_SIZE,
                ),
                indexed_storage_buffers: indexed_buffer_bindings(
                    &gl,
                    glow::MAX_SHADER_STORAGE_BUFFER_BINDINGS,
                    glow::SHADER_STORAGE_BUFFER_BINDING,
                    glow::SHADER_STORAGE_BUFFER_START,
                    glow::SHADER_STORAGE_BUFFER_SIZE,
                ),
                draw_framebuffer: framebuffer_name(
                    gl.get_parameter_i32(glow::DRAW_FRAMEBUFFER_BINDING),
                ),
                read_framebuffer: framebuffer_name(
                    gl.get_parameter_i32(glow::READ_FRAMEBUFFER_BINDING),
                ),
                active_texture,
                texture_units,
                image_units,
                viewport: parameter_i32x4(&gl, glow::VIEWPORT),
                scissor_box: parameter_i32x4(&gl, glow::SCISSOR_BOX),
                scissor_enabled: gl.is_enabled(glow::SCISSOR_TEST),
                cull_enabled: gl.is_enabled(glow::CULL_FACE),
                cull_face: gl.get_parameter_i32(glow::CULL_FACE_MODE),
                blend_enabled: gl.is_enabled(glow::BLEND),
                blend_src_rgb: gl.get_parameter_i32(glow::BLEND_SRC_RGB),
                blend_dst_rgb: gl.get_parameter_i32(glow::BLEND_DST_RGB),
                blend_src_alpha: gl.get_parameter_i32(glow::BLEND_SRC_ALPHA),
                blend_dst_alpha: gl.get_parameter_i32(glow::BLEND_DST_ALPHA),
                blend_equation_rgb: gl.get_parameter_i32(glow::BLEND_EQUATION_RGB),
                blend_equation_alpha: gl.get_parameter_i32(glow::BLEND_EQUATION_ALPHA),
                blend_color: parameter_f32x4(&gl, glow::BLEND_COLOR),
                color_writemask: parameter_boolx4(&gl, glow::COLOR_WRITEMASK),
                front_face: gl.get_parameter_i32(glow::FRONT_FACE),
                stencil_enabled: gl.is_enabled(glow::STENCIL_TEST),
                stencil_front: StencilFaceState {
                    func: gl.get_parameter_i32(glow::STENCIL_FUNC),
                    reference: gl.get_parameter_i32(glow::STENCIL_REF),
                    value_mask: gl.get_parameter_i32(glow::STENCIL_VALUE_MASK),
                    write_mask: gl.get_parameter_i32(glow::STENCIL_WRITEMASK),
                    fail: gl.get_parameter_i32(glow::STENCIL_FAIL),
                    pass_depth_fail: gl.get_parameter_i32(glow::STENCIL_PASS_DEPTH_FAIL),
                    pass_depth_pass: gl.get_parameter_i32(glow::STENCIL_PASS_DEPTH_PASS),
                },
                stencil_back: StencilFaceState {
                    func: gl.get_parameter_i32(glow::STENCIL_BACK_FUNC),
                    reference: gl.get_parameter_i32(glow::STENCIL_BACK_REF),
                    value_mask: gl.get_parameter_i32(glow::STENCIL_BACK_VALUE_MASK),
                    write_mask: gl.get_parameter_i32(glow::STENCIL_BACK_WRITEMASK),
                    fail: gl.get_parameter_i32(glow::STENCIL_BACK_FAIL),
                    pass_depth_fail: gl.get_parameter_i32(glow::STENCIL_BACK_PASS_DEPTH_FAIL),
                    pass_depth_pass: gl.get_parameter_i32(glow::STENCIL_BACK_PASS_DEPTH_PASS),
                },
                unpack_alignment: gl.get_parameter_i32(glow::UNPACK_ALIGNMENT),
                unpack_row_length: gl.get_parameter_i32(glow::UNPACK_ROW_LENGTH),
                unpack_skip_rows: gl.get_parameter_i32(glow::UNPACK_SKIP_ROWS),
                unpack_skip_pixels: gl.get_parameter_i32(glow::UNPACK_SKIP_PIXELS),
                unpack_image_height: gl.get_parameter_i32(glow::UNPACK_IMAGE_HEIGHT),
                pack_alignment: gl.get_parameter_i32(glow::PACK_ALIGNMENT),
                pack_row_length: gl.get_parameter_i32(glow::PACK_ROW_LENGTH),
                pack_skip_rows: gl.get_parameter_i32(glow::PACK_SKIP_ROWS),
                pack_skip_pixels: gl.get_parameter_i32(glow::PACK_SKIP_PIXELS),
                pack_image_height: gl.get_parameter_i32(glow::PACK_IMAGE_HEIGHT),
                depth_enabled: gl.is_enabled(glow::DEPTH_TEST),
                depth_func: gl.get_parameter_i32(glow::DEPTH_FUNC),
                depth_writemask: gl.get_parameter_i32(glow::DEPTH_WRITEMASK) != 0,
                depth_range: parameter_f32x2(&gl, glow::DEPTH_RANGE),
                line_width: gl.get_parameter_f32(glow::LINE_WIDTH),
                polygon_mode: polygon_mode_state(&gl),
                polygon_offset_fill_enabled: gl.is_enabled(glow::POLYGON_OFFSET_FILL),
                polygon_offset_factor: gl.get_parameter_f32(glow::POLYGON_OFFSET_FACTOR),
                polygon_offset_units: gl.get_parameter_f32(glow::POLYGON_OFFSET_UNITS),
                primitive_restart_enabled: gl.is_enabled(glow::PRIMITIVE_RESTART),
                rasterizer_discard_enabled: gl.is_enabled(glow::RASTERIZER_DISCARD),
                dither_enabled: gl.is_enabled(glow::DITHER),
                multisample_enabled: gl.is_enabled(glow::MULTISAMPLE),
                gl,
            }
        }
    }
}

impl Drop for BorrowedOpenGlStateGuard {
    fn drop(&mut self) {
        let _zone = trace::Zone::new("opengl.borrowed-state.restore");
        unsafe {
            self.gl.use_program(self.program);
            self.gl.bind_vertex_array(self.vertex_array);
            self.gl.bind_buffer(glow::ARRAY_BUFFER, self.array_buffer);
            if self.vertex_array.is_some() {
                self.gl
                    .bind_buffer(glow::ELEMENT_ARRAY_BUFFER, self.element_array_buffer);
            }
            self.gl
                .bind_buffer(glow::COPY_READ_BUFFER, self.copy_read_buffer);
            self.gl
                .bind_buffer(glow::COPY_WRITE_BUFFER, self.copy_write_buffer);
            self.gl
                .bind_buffer(glow::PIXEL_UNPACK_BUFFER, self.pixel_unpack_buffer);
            self.gl
                .bind_buffer(glow::PIXEL_PACK_BUFFER, self.pixel_pack_buffer);
            restore_indexed_buffer_bindings(
                &self.gl,
                glow::UNIFORM_BUFFER,
                &self.indexed_uniform_buffers,
            );
            restore_indexed_buffer_bindings(
                &self.gl,
                glow::SHADER_STORAGE_BUFFER,
                &self.indexed_storage_buffers,
            );
            self.gl
                .bind_buffer(glow::UNIFORM_BUFFER, self.uniform_buffer);
            self.gl
                .bind_buffer(glow::SHADER_STORAGE_BUFFER, self.storage_buffer);
            self.gl
                .bind_framebuffer(glow::DRAW_FRAMEBUFFER, self.draw_framebuffer);
            self.gl
                .bind_framebuffer(glow::READ_FRAMEBUFFER, self.read_framebuffer);
            for state in &self.texture_units {
                self.gl.active_texture(glow::TEXTURE0 + state.unit);
                self.gl.bind_texture(glow::TEXTURE_2D, state.texture_2d);
                self.gl.bind_texture(glow::TEXTURE_3D, state.texture_3d);
                self.gl.bind_sampler(state.unit, state.sampler);
            }
            for state in &self.image_units {
                self.gl.bind_image_texture(
                    state.unit,
                    state.texture,
                    state.level,
                    state.layered,
                    state.layer,
                    state.access,
                    state.format,
                );
            }
            self.gl.active_texture(self.active_texture as u32);
            self.gl.viewport(
                self.viewport[0],
                self.viewport[1],
                self.viewport[2],
                self.viewport[3],
            );
            self.gl.scissor(
                self.scissor_box[0],
                self.scissor_box[1],
                self.scissor_box[2],
                self.scissor_box[3],
            );
            set_enabled(&self.gl, glow::SCISSOR_TEST, self.scissor_enabled);
            set_enabled(&self.gl, glow::CULL_FACE, self.cull_enabled);
            self.gl.cull_face(self.cull_face as u32);
            set_enabled(&self.gl, glow::BLEND, self.blend_enabled);
            self.gl.blend_func_separate(
                self.blend_src_rgb as u32,
                self.blend_dst_rgb as u32,
                self.blend_src_alpha as u32,
                self.blend_dst_alpha as u32,
            );
            self.gl.blend_equation_separate(
                self.blend_equation_rgb as u32,
                self.blend_equation_alpha as u32,
            );
            self.gl.blend_color(
                self.blend_color[0],
                self.blend_color[1],
                self.blend_color[2],
                self.blend_color[3],
            );
            self.gl.color_mask(
                self.color_writemask[0],
                self.color_writemask[1],
                self.color_writemask[2],
                self.color_writemask[3],
            );
            self.gl.front_face(self.front_face as u32);
            set_enabled(&self.gl, glow::STENCIL_TEST, self.stencil_enabled);
            restore_stencil_face(&self.gl, glow::FRONT, &self.stencil_front);
            restore_stencil_face(&self.gl, glow::BACK, &self.stencil_back);
            self.gl
                .pixel_store_i32(glow::UNPACK_ALIGNMENT, self.unpack_alignment);
            self.gl
                .pixel_store_i32(glow::UNPACK_ROW_LENGTH, self.unpack_row_length);
            self.gl
                .pixel_store_i32(glow::UNPACK_SKIP_ROWS, self.unpack_skip_rows);
            self.gl
                .pixel_store_i32(glow::UNPACK_SKIP_PIXELS, self.unpack_skip_pixels);
            self.gl
                .pixel_store_i32(glow::UNPACK_IMAGE_HEIGHT, self.unpack_image_height);
            self.gl
                .pixel_store_i32(glow::PACK_ALIGNMENT, self.pack_alignment);
            self.gl
                .pixel_store_i32(glow::PACK_ROW_LENGTH, self.pack_row_length);
            self.gl
                .pixel_store_i32(glow::PACK_SKIP_ROWS, self.pack_skip_rows);
            self.gl
                .pixel_store_i32(glow::PACK_SKIP_PIXELS, self.pack_skip_pixels);
            self.gl
                .pixel_store_i32(glow::PACK_IMAGE_HEIGHT, self.pack_image_height);
            set_enabled(&self.gl, glow::DEPTH_TEST, self.depth_enabled);
            self.gl.depth_func(self.depth_func as u32);
            self.gl.depth_mask(self.depth_writemask);
            self.gl
                .depth_range_f32(self.depth_range[0], self.depth_range[1]);
            self.gl.line_width(self.line_width);
            restore_polygon_mode(&self.gl, self.polygon_mode);
            set_enabled(
                &self.gl,
                glow::POLYGON_OFFSET_FILL,
                self.polygon_offset_fill_enabled,
            );
            self.gl
                .polygon_offset(self.polygon_offset_factor, self.polygon_offset_units);
            set_enabled(
                &self.gl,
                glow::PRIMITIVE_RESTART,
                self.primitive_restart_enabled,
            );
            set_enabled(
                &self.gl,
                glow::RASTERIZER_DISCARD,
                self.rasterizer_discard_enabled,
            );
            set_enabled(&self.gl, glow::DITHER, self.dither_enabled);
            set_enabled(&self.gl, glow::MULTISAMPLE, self.multisample_enabled);
        }
    }
}

fn supports_storage_textures(gl: &glow::Context) -> bool {
    let version = gl.version();
    (!version.is_embedded && (version.major > 4 || (version.major == 4 && version.minor >= 2)))
        || (version.is_embedded
            && (version.major > 3 || (version.major == 3 && version.minor >= 1)))
        || gl
            .supported_extensions()
            .contains("GL_ARB_shader_image_load_store")
}

fn supports_compute_shaders(gl: &glow::Context) -> bool {
    let version = gl.version();
    (!version.is_embedded && (version.major > 4 || (version.major == 4 && version.minor >= 3)))
        || (version.is_embedded
            && (version.major > 3 || (version.major == 3 && version.minor >= 1)))
        || gl.supported_extensions().contains("GL_ARB_compute_shader")
}

fn parameter_i32x2(gl: &glow::Context, parameter: u32) -> [i32; 2] {
    let mut values = [0; 2];
    unsafe {
        gl.get_parameter_i32_slice(parameter, &mut values);
    }
    values
}

fn parameter_i32x4(gl: &glow::Context, parameter: u32) -> [i32; 4] {
    let mut values = [0; 4];
    unsafe {
        gl.get_parameter_i32_slice(parameter, &mut values);
    }
    values
}

fn polygon_mode_state(gl: &glow::Context) -> [i32; 2] {
    let mut mode = parameter_i32x2(gl, glow::POLYGON_MODE);
    if mode[1] == 0 {
        mode[1] = mode[0];
    }
    mode
}

fn parameter_f32x2(gl: &glow::Context, parameter: u32) -> [f32; 2] {
    let mut values = [0.0; 2];
    unsafe {
        gl.get_parameter_f32_slice(parameter, &mut values);
    }
    values
}

fn parameter_f32x4(gl: &glow::Context, parameter: u32) -> [f32; 4] {
    let mut values = [0.0; 4];
    unsafe {
        gl.get_parameter_f32_slice(parameter, &mut values);
    }
    values
}

fn parameter_boolx4(gl: &glow::Context, parameter: u32) -> [bool; 4] {
    let values = parameter_i32x4(gl, parameter);
    [
        values[0] != 0,
        values[1] != 0,
        values[2] != 0,
        values[3] != 0,
    ]
}

fn restore_stencil_face(gl: &glow::Context, face: u32, state: &StencilFaceState) {
    unsafe {
        gl.stencil_func_separate(
            face,
            state.func as u32,
            state.reference,
            state.value_mask as u32,
        );
        gl.stencil_mask_separate(face, state.write_mask as u32);
        gl.stencil_op_separate(
            face,
            state.fail as u32,
            state.pass_depth_fail as u32,
            state.pass_depth_pass as u32,
        );
    }
}

fn restore_polygon_mode(gl: &glow::Context, mode: [i32; 2]) {
    unsafe {
        if mode[0] == mode[1] {
            gl.polygon_mode(glow::FRONT_AND_BACK, mode[0] as u32);
        } else {
            gl.polygon_mode(glow::FRONT, mode[0] as u32);
            gl.polygon_mode(glow::BACK, mode[1] as u32);
        }
    }
}

fn indexed_buffer_bindings(
    gl: &glow::Context,
    max_parameter: u32,
    binding_parameter: u32,
    start_parameter: u32,
    size_parameter: u32,
) -> Vec<IndexedBufferBinding> {
    let count = unsafe { gl.get_parameter_i32(max_parameter) }.clamp(0, 256);
    (0..u32::try_from(count).unwrap_or_default())
        .map(|index| IndexedBufferBinding {
            index,
            buffer: buffer_name(unsafe { gl.get_parameter_indexed_i32(binding_parameter, index) }),
            start: unsafe { gl.get_parameter_indexed_i64(start_parameter, index) },
            size: unsafe { gl.get_parameter_indexed_i64(size_parameter, index) },
        })
        .collect()
}

fn restore_indexed_buffer_bindings(
    gl: &glow::Context,
    target: u32,
    bindings: &[IndexedBufferBinding],
) {
    unsafe {
        for binding in bindings {
            if let (Some(buffer), Ok(start), Ok(size)) = (
                binding.buffer,
                i32::try_from(binding.start),
                i32::try_from(binding.size),
            ) {
                if binding.size > 0 {
                    gl.bind_buffer_range(target, binding.index, Some(buffer), start, size);
                    continue;
                }
            }
            gl.bind_buffer_base(target, binding.index, binding.buffer);
        }
    }
}

fn set_enabled(gl: &glow::Context, flag: u32, enabled: bool) {
    unsafe {
        if enabled {
            gl.enable(flag);
        } else {
            gl.disable(flag);
        }
    }
}

fn non_zero_name(value: i32) -> Option<NonZeroU32> {
    u32::try_from(value).ok().and_then(NonZeroU32::new)
}

fn program_name(value: i32) -> Option<glow::NativeProgram> {
    non_zero_name(value).map(glow::NativeProgram)
}

fn buffer_name(value: i32) -> Option<glow::NativeBuffer> {
    non_zero_name(value).map(glow::NativeBuffer)
}

fn vertex_array_name(value: i32) -> Option<glow::NativeVertexArray> {
    non_zero_name(value).map(glow::NativeVertexArray)
}

fn texture_name(value: i32) -> Option<glow::NativeTexture> {
    non_zero_name(value).map(glow::NativeTexture)
}

fn sampler_name(value: i32) -> Option<glow::NativeSampler> {
    non_zero_name(value).map(glow::NativeSampler)
}

fn framebuffer_name(value: i32) -> Option<glow::NativeFramebuffer> {
    non_zero_name(value).map(glow::NativeFramebuffer)
}

fn create_native_context() -> GalResult<NativeOpenGlContext> {
    let mut errors = Vec::new();
    match load_egl().and_then(|egl| {
        let (display, surface, context) = create_isolated_context(&egl)?;
        Ok(NativeOpenGlContext::Egl {
            egl,
            display,
            surface,
            context,
        })
    }) {
        Ok(context) => return Ok(context),
        Err(error) => errors.push(format!("EGL: {error}")),
    }
    match GlxContextState::new() {
        Ok(context) => Ok(NativeOpenGlContext::Glx(context)),
        Err(error) => {
            errors.push(format!("GLX: {error}"));
            Err(GalError::backend(format!(
                "failed to create isolated Rust OpenGL context: {}",
                errors.join("; ")
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glow::HasContext;

    #[test]
    fn borrowed_state_guard_restores_indexed_buffer_bindings() {
        let _graphics_lock = crate::render::vulkanic::backends::graphics_backend_lock()
            .lock()
            .expect("OpenGL test lock");
        let context = match OpenGlContext::new("borrowed-state-indexed-bindings-test") {
            Ok(context) => context,
            Err(error) => {
                assert!(
                    error.to_string().contains("OpenGL")
                        || error.to_string().contains("EGL")
                        || error.to_string().contains("GLX"),
                    "unexpected OpenGL bootstrap failure: {error}"
                );
                return;
            }
        };
        context
            .make_current()
            .expect("test OpenGL context should become current");
        let gl = context.gl().clone();
        unsafe {
            let max_ubo = gl.get_parameter_i32(glow::MAX_UNIFORM_BUFFER_BINDINGS);
            let max_ssbo = gl.get_parameter_i32(glow::MAX_SHADER_STORAGE_BUFFER_BINDINGS);
            if max_ubo <= 0 || max_ssbo <= 0 {
                return;
            }
            let original_ubo = gl.create_buffer().expect("create original UBO");
            let replacement_ubo = gl.create_buffer().expect("create replacement UBO");
            let original_ssbo = gl.create_buffer().expect("create original SSBO");
            let replacement_ssbo = gl.create_buffer().expect("create replacement SSBO");
            for (target, buffer) in [
                (glow::UNIFORM_BUFFER, original_ubo),
                (glow::UNIFORM_BUFFER, replacement_ubo),
                (glow::SHADER_STORAGE_BUFFER, original_ssbo),
                (glow::SHADER_STORAGE_BUFFER, replacement_ssbo),
            ] {
                gl.bind_buffer(target, Some(buffer));
                gl.buffer_data_size(target, 64, glow::DYNAMIC_DRAW);
            }
            gl.bind_buffer_range(glow::UNIFORM_BUFFER, 0, Some(original_ubo), 16, 32);
            gl.bind_buffer_range(glow::SHADER_STORAGE_BUFFER, 0, Some(original_ssbo), 8, 40);
            let original_generic_ubo = gl.get_parameter_i32(glow::UNIFORM_BUFFER_BINDING);
            let original_generic_ssbo = gl.get_parameter_i32(glow::SHADER_STORAGE_BUFFER_BINDING);
            let original_ubo_binding =
                gl.get_parameter_indexed_i32(glow::UNIFORM_BUFFER_BINDING, 0);
            let original_ubo_start = gl.get_parameter_indexed_i64(glow::UNIFORM_BUFFER_START, 0);
            let original_ubo_size = gl.get_parameter_indexed_i64(glow::UNIFORM_BUFFER_SIZE, 0);
            let original_ssbo_binding =
                gl.get_parameter_indexed_i32(glow::SHADER_STORAGE_BUFFER_BINDING, 0);
            let original_ssbo_start =
                gl.get_parameter_indexed_i64(glow::SHADER_STORAGE_BUFFER_START, 0);
            let original_ssbo_size =
                gl.get_parameter_indexed_i64(glow::SHADER_STORAGE_BUFFER_SIZE, 0);
            {
                let _guard = BorrowedOpenGlStateGuard::capture(gl.clone(), false);
                gl.bind_buffer_range(glow::UNIFORM_BUFFER, 0, Some(replacement_ubo), 0, 64);
                gl.bind_buffer_range(
                    glow::SHADER_STORAGE_BUFFER,
                    0,
                    Some(replacement_ssbo),
                    0,
                    64,
                );
            }
            assert_eq!(
                original_generic_ubo,
                gl.get_parameter_i32(glow::UNIFORM_BUFFER_BINDING),
                "borrowed OpenGL guard must restore generic UBO binding after indexed ranges"
            );
            assert_eq!(
                original_generic_ssbo,
                gl.get_parameter_i32(glow::SHADER_STORAGE_BUFFER_BINDING),
                "borrowed OpenGL guard must restore generic SSBO binding after indexed ranges"
            );
            assert_eq!(
                original_ubo_binding,
                gl.get_parameter_indexed_i32(glow::UNIFORM_BUFFER_BINDING, 0),
                "borrowed OpenGL guard must restore indexed UBO binding"
            );
            assert_eq!(
                original_ubo_start,
                gl.get_parameter_indexed_i64(glow::UNIFORM_BUFFER_START, 0),
                "borrowed OpenGL guard must restore indexed UBO range offset"
            );
            assert_eq!(
                original_ubo_size,
                gl.get_parameter_indexed_i64(glow::UNIFORM_BUFFER_SIZE, 0),
                "borrowed OpenGL guard must restore indexed UBO range size"
            );
            assert_eq!(
                original_ssbo_binding,
                gl.get_parameter_indexed_i32(glow::SHADER_STORAGE_BUFFER_BINDING, 0),
                "borrowed OpenGL guard must restore indexed SSBO binding"
            );
            assert_eq!(
                original_ssbo_start,
                gl.get_parameter_indexed_i64(glow::SHADER_STORAGE_BUFFER_START, 0),
                "borrowed OpenGL guard must restore indexed SSBO range offset"
            );
            assert_eq!(
                original_ssbo_size,
                gl.get_parameter_indexed_i64(glow::SHADER_STORAGE_BUFFER_SIZE, 0),
                "borrowed OpenGL guard must restore indexed SSBO range size"
            );
            gl.bind_buffer_base(glow::UNIFORM_BUFFER, 0, None);
            gl.bind_buffer_base(glow::SHADER_STORAGE_BUFFER, 0, None);
            gl.delete_buffer(original_ubo);
            gl.delete_buffer(replacement_ubo);
            gl.delete_buffer(original_ssbo);
            gl.delete_buffer(replacement_ssbo);
        }
    }

    #[test]
    fn borrowed_state_guard_restores_d3_texture_and_image_bindings() {
        let _graphics_lock = crate::render::vulkanic::backends::graphics_backend_lock()
            .lock()
            .expect("OpenGL test lock");
        let context = match OpenGlContext::new("borrowed-state-d3-image-test") {
            Ok(context) => context,
            Err(error) => {
                assert!(
                    error.to_string().contains("OpenGL")
                        || error.to_string().contains("EGL")
                        || error.to_string().contains("GLX"),
                    "unexpected OpenGL bootstrap failure: {error}"
                );
                return;
            }
        };
        context
            .make_current()
            .expect("test OpenGL context should become current");
        if !context.supports_storage_textures() {
            return;
        }
        let gl = context.gl().clone();
        unsafe {
            let original = gl.create_texture().expect("create original D3 texture");
            let replacement = gl.create_texture().expect("create replacement D3 texture");
            for texture in [original, replacement] {
                gl.bind_texture(glow::TEXTURE_3D, Some(texture));
                gl.tex_image_3d(
                    glow::TEXTURE_3D,
                    0,
                    glow::R8UI as i32,
                    2,
                    2,
                    2,
                    0,
                    glow::RED_INTEGER,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(None),
                );
            }
            gl.active_texture(glow::TEXTURE0 + 2);
            gl.bind_texture(glow::TEXTURE_3D, Some(original));
            gl.bind_image_texture(0, Some(original), 0, true, 0, glow::READ_ONLY, glow::R8UI);
            let original_image = gl.get_parameter_indexed_i32(glow::IMAGE_BINDING_NAME, 0);
            {
                let _guard = BorrowedOpenGlStateGuard::capture(gl.clone(), true);
                gl.active_texture(glow::TEXTURE0 + 2);
                gl.bind_texture(glow::TEXTURE_3D, Some(replacement));
                gl.bind_image_texture(
                    0,
                    Some(replacement),
                    0,
                    true,
                    0,
                    glow::WRITE_ONLY,
                    glow::R8UI,
                );
            }
            gl.active_texture(glow::TEXTURE0 + 2);
            assert_eq!(
                original.0.get() as i32,
                gl.get_parameter_i32(glow::TEXTURE_BINDING_3D),
                "borrowed OpenGL guard must restore D3 texture unit bindings"
            );
            assert_eq!(
                original_image,
                gl.get_parameter_indexed_i32(glow::IMAGE_BINDING_NAME, 0),
                "borrowed OpenGL guard must restore image-unit bindings"
            );
            gl.bind_image_texture(0, None, 0, false, 0, glow::READ_ONLY, glow::R8UI);
            gl.bind_texture(glow::TEXTURE_3D, None);
            gl.delete_texture(original);
            gl.delete_texture(replacement);
        }
    }

    #[test]
    fn borrowed_state_guard_restores_outline_sensitive_pipeline_state() {
        let _graphics_lock = crate::render::vulkanic::backends::graphics_backend_lock()
            .lock()
            .expect("OpenGL test lock");
        let context = match OpenGlContext::new("borrowed-state-outline-pipeline-test") {
            Ok(context) => context,
            Err(error) => {
                assert!(
                    error.to_string().contains("OpenGL")
                        || error.to_string().contains("EGL")
                        || error.to_string().contains("GLX"),
                    "unexpected OpenGL bootstrap failure: {error}"
                );
                return;
            }
        };
        context
            .make_current()
            .expect("test OpenGL context should become current");
        let gl = context.gl().clone();
        unsafe {
            drain_gl_errors(&gl);

            let original_vao = gl.create_vertex_array().expect("create original VAO");
            let replacement_vao = gl.create_vertex_array().expect("create replacement VAO");
            let original_indices = gl.create_buffer().expect("create original index buffer");
            let replacement_indices = gl.create_buffer().expect("create replacement index buffer");
            gl.bind_vertex_array(Some(original_vao));
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(original_indices));
            gl.buffer_data_u8_slice(glow::ELEMENT_ARRAY_BUFFER, &[0, 0, 0, 0], glow::STATIC_DRAW);
            gl.bind_vertex_array(Some(replacement_vao));
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(replacement_indices));
            gl.buffer_data_u8_slice(glow::ELEMENT_ARRAY_BUFFER, &[0, 0, 0, 0], glow::STATIC_DRAW);
            gl.bind_vertex_array(Some(original_vao));
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(original_indices));
            gl.line_width(1.0);
            gl.blend_color(0.125, 0.25, 0.375, 0.5);
            gl.depth_range_f32(0.125, 0.875);
            gl.polygon_mode(glow::FRONT_AND_BACK, glow::LINE);
            gl.enable(glow::POLYGON_OFFSET_FILL);
            gl.polygon_offset(1.25, 2.5);
            gl.enable(glow::PRIMITIVE_RESTART);
            gl.disable(glow::RASTERIZER_DISCARD);
            gl.enable(glow::DITHER);
            gl.enable(glow::MULTISAMPLE);
            gl.stencil_func_separate(glow::FRONT, glow::ALWAYS, 3, 0x7f);
            gl.stencil_mask_separate(glow::FRONT, 0x3f);
            gl.stencil_op_separate(glow::FRONT, glow::KEEP, glow::REPLACE, glow::INCR_WRAP);
            gl.stencil_func_separate(glow::BACK, glow::NOTEQUAL, 5, 0xef);
            gl.stencil_mask_separate(glow::BACK, 0xcf);
            gl.stencil_op_separate(glow::BACK, glow::ZERO, glow::INVERT, glow::DECR_WRAP);

            {
                let _guard = BorrowedOpenGlStateGuard::capture(gl.clone(), false);
                gl.bind_vertex_array(Some(replacement_vao));
                gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(replacement_indices));
                gl.line_width(1.0);
                gl.blend_color(0.625, 0.5, 0.375, 0.25);
                gl.depth_range_f32(0.0, 1.0);
                gl.polygon_mode(glow::FRONT_AND_BACK, glow::FILL);
                gl.disable(glow::POLYGON_OFFSET_FILL);
                gl.polygon_offset(0.0, 0.0);
                gl.disable(glow::PRIMITIVE_RESTART);
                gl.enable(glow::RASTERIZER_DISCARD);
                gl.disable(glow::DITHER);
                gl.disable(glow::MULTISAMPLE);
                gl.stencil_func_separate(glow::FRONT, glow::NEVER, 0, 0);
                gl.stencil_mask_separate(glow::FRONT, 0);
                gl.stencil_op_separate(glow::FRONT, glow::ZERO, glow::ZERO, glow::ZERO);
                gl.stencil_func_separate(glow::BACK, glow::ALWAYS, 0, 0);
                gl.stencil_mask_separate(glow::BACK, 0);
                gl.stencil_op_separate(glow::BACK, glow::KEEP, glow::KEEP, glow::KEEP);
            }

            assert_eq!(
                Some(original_vao),
                vertex_array_name(gl.get_parameter_i32(glow::VERTEX_ARRAY_BINDING)),
                "borrowed OpenGL guard must restore Java VAO before later outline draws"
            );
            assert_eq!(
                Some(original_indices),
                buffer_name(gl.get_parameter_i32(glow::ELEMENT_ARRAY_BUFFER_BINDING)),
                "borrowed OpenGL guard must restore VAO-scoped index buffer"
            );
            // GL_BLEND_COLOR writes four floats; a scalar query corrupts the stack.
            let blend_color = parameter_f32x4(&gl, glow::BLEND_COLOR);
            assert_f32_eq(0.125, blend_color[0]);
            assert_f32_eq(0.25, blend_color[1]);
            assert_f32_eq(0.375, blend_color[2]);
            assert_f32_eq(0.5, blend_color[3]);
            let depth_range = parameter_f32x2(&gl, glow::DEPTH_RANGE);
            assert_f32_eq(0.125, depth_range[0]);
            assert_f32_eq(0.875, depth_range[1]);
            assert_eq!(
                [glow::LINE as i32, glow::LINE as i32],
                polygon_mode_state(&gl),
                "borrowed OpenGL guard must restore polygon mode"
            );
            assert!(gl.is_enabled(glow::POLYGON_OFFSET_FILL));
            assert_f32_eq(1.25, gl.get_parameter_f32(glow::POLYGON_OFFSET_FACTOR));
            assert_f32_eq(2.5, gl.get_parameter_f32(glow::POLYGON_OFFSET_UNITS));
            assert!(gl.is_enabled(glow::PRIMITIVE_RESTART));
            assert!(!gl.is_enabled(glow::RASTERIZER_DISCARD));
            assert!(gl.is_enabled(glow::DITHER));
            assert!(gl.is_enabled(glow::MULTISAMPLE));
            assert_eq!(
                glow::ALWAYS as i32,
                gl.get_parameter_i32(glow::STENCIL_FUNC)
            );
            assert_eq!(3, gl.get_parameter_i32(glow::STENCIL_REF));
            assert_eq!(0x7f, gl.get_parameter_i32(glow::STENCIL_VALUE_MASK));
            assert_eq!(0x3f, gl.get_parameter_i32(glow::STENCIL_WRITEMASK));
            assert_eq!(glow::KEEP as i32, gl.get_parameter_i32(glow::STENCIL_FAIL));
            assert_eq!(
                glow::REPLACE as i32,
                gl.get_parameter_i32(glow::STENCIL_PASS_DEPTH_FAIL)
            );
            assert_eq!(
                glow::INCR_WRAP as i32,
                gl.get_parameter_i32(glow::STENCIL_PASS_DEPTH_PASS)
            );
            assert_eq!(
                glow::NOTEQUAL as i32,
                gl.get_parameter_i32(glow::STENCIL_BACK_FUNC)
            );
            assert_eq!(5, gl.get_parameter_i32(glow::STENCIL_BACK_REF));
            assert_eq!(0xef, gl.get_parameter_i32(glow::STENCIL_BACK_VALUE_MASK));
            assert_eq!(0xcf, gl.get_parameter_i32(glow::STENCIL_BACK_WRITEMASK));
            assert_eq!(
                glow::ZERO as i32,
                gl.get_parameter_i32(glow::STENCIL_BACK_FAIL)
            );
            assert_eq!(
                glow::INVERT as i32,
                gl.get_parameter_i32(glow::STENCIL_BACK_PASS_DEPTH_FAIL)
            );
            assert_eq!(
                glow::DECR_WRAP as i32,
                gl.get_parameter_i32(glow::STENCIL_BACK_PASS_DEPTH_PASS)
            );
            assert_eq!(
                glow::NO_ERROR,
                gl.get_error(),
                "borrowed OpenGL state restore produced GL errors"
            );

            gl.disable(glow::PRIMITIVE_RESTART);
            gl.disable(glow::POLYGON_OFFSET_FILL);
            gl.polygon_mode(glow::FRONT_AND_BACK, glow::FILL);
            gl.bind_vertex_array(None);
            gl.delete_buffer(original_indices);
            gl.delete_buffer(replacement_indices);
            gl.delete_vertex_array(original_vao);
            gl.delete_vertex_array(replacement_vao);
        }
    }

    unsafe fn drain_gl_errors(gl: &glow::Context) {
        while gl.get_error() != glow::NO_ERROR {}
    }

    fn assert_f32_eq(expected: f32, actual: f32) {
        assert!(
            (expected - actual).abs() <= 0.0001,
            "expected {expected}, got {actual}"
        );
    }
}

fn create_isolated_context(egl: &EglFns) -> GalResult<(EglDisplay, EglSurface, EglContextHandle)> {
    let mut errors = Vec::new();
    if client_supports(egl, "EGL_MESA_platform_surfaceless") {
        match create_context_attempt(egl, true) {
            Ok(objects) => return Ok(objects),
            Err(error) => errors.push(format!("surfaceless EGL: {error}")),
        }
    } else {
        errors.push("surfaceless EGL: EGL_MESA_platform_surfaceless unavailable".to_string());
    }
    match create_context_attempt(egl, false) {
        Ok(objects) => Ok(objects),
        Err(error) => {
            errors.push(format!("pbuffer EGL: {error}"));
            Err(GalError::backend(errors.join("; ")))
        }
    }
}

fn create_context_attempt(
    egl: &EglFns,
    surfaceless: bool,
) -> GalResult<(EglDisplay, EglSurface, EglContextHandle)> {
    let display = choose_display(egl, surfaceless)?;
    if display.is_null() {
        return Err(GalError::backend("EGL returned no display"));
    }
    let mut major = 0;
    let mut minor = 0;
    if unsafe { (egl.initialize)(display, &mut major, &mut minor) } == EGL_FALSE {
        return Err(GalError::backend(egl_error(
            egl,
            "EGL initialization failed",
        )));
    }
    if unsafe { (egl.bind_api)(EGL_OPENGL_API) } == EGL_FALSE {
        unsafe { (egl.terminate)(display) };
        return Err(GalError::backend(egl_error(
            egl,
            "EGL OpenGL API binding failed",
        )));
    }
    if surfaceless && !display_supports(egl, display, "EGL_KHR_surfaceless_context") {
        unsafe { (egl.terminate)(display) };
        return Err(GalError::backend(
            "EGL_KHR_surfaceless_context unavailable on display",
        ));
    }

    let config = choose_config(egl, display, surfaceless)?;
    let surface = if surfaceless {
        EGL_NO_SURFACE
    } else {
        let surface_attribs = [EGL_WIDTH, 16, EGL_HEIGHT, 16, EGL_NONE];
        let surface =
            unsafe { (egl.create_pbuffer_surface)(display, config, surface_attribs.as_ptr()) };
        if surface.is_null() {
            unsafe { (egl.terminate)(display) };
            return Err(GalError::backend(egl_error(
                egl,
                "EGL pbuffer surface creation failed",
            )));
        }
        surface
    };

    let context = create_context(egl, display, config).map_err(|error| {
        if !surface.is_null() {
            unsafe { (egl.destroy_surface)(display, surface) };
        }
        unsafe { (egl.terminate)(display) };
        error
    })?;
    if unsafe { (egl.make_current)(display, surface, surface, context) } == EGL_FALSE {
        let error = egl_error(egl, "EGL make-current failed");
        unsafe {
            (egl.destroy_context)(display, context);
            if !surface.is_null() {
                (egl.destroy_surface)(display, surface);
            }
            (egl.terminate)(display);
        }
        return Err(GalError::backend(error));
    }
    Ok((display, surface, context))
}

fn choose_display(egl: &EglFns, surfaceless: bool) -> GalResult<EglDisplay> {
    if surfaceless {
        let Some(get_platform_display) = egl.get_platform_display_ext else {
            return Err(GalError::backend(
                "eglGetPlatformDisplayEXT unavailable for surfaceless context",
            ));
        };
        return Ok(unsafe {
            get_platform_display(EGL_PLATFORM_SURFACELESS_MESA, ptr::null_mut(), ptr::null())
        });
    }
    Ok(unsafe { (egl.get_display)(ptr::null_mut()) })
}

fn choose_config(egl: &EglFns, display: EglDisplay, surfaceless: bool) -> GalResult<EglConfig> {
    let pbuffer_config = [
        EGL_SURFACE_TYPE,
        EGL_PBUFFER_BIT,
        EGL_RENDERABLE_TYPE,
        EGL_OPENGL_BIT,
        EGL_RED_SIZE,
        8,
        EGL_GREEN_SIZE,
        8,
        EGL_BLUE_SIZE,
        8,
        EGL_ALPHA_SIZE,
        8,
        EGL_DEPTH_SIZE,
        24,
        EGL_NONE,
    ];
    let surfaceless_config = [
        EGL_RENDERABLE_TYPE,
        EGL_OPENGL_BIT,
        EGL_RED_SIZE,
        8,
        EGL_GREEN_SIZE,
        8,
        EGL_BLUE_SIZE,
        8,
        EGL_ALPHA_SIZE,
        8,
        EGL_DEPTH_SIZE,
        24,
        EGL_NONE,
    ];
    let attribs = if surfaceless {
        &surfaceless_config[..]
    } else {
        &pbuffer_config[..]
    };
    let mut config = ptr::null_mut();
    let mut config_count = 0;
    if unsafe { (egl.choose_config)(display, attribs.as_ptr(), &mut config, 1, &mut config_count) }
        == EGL_FALSE
        || config.is_null()
        || config_count == 0
    {
        unsafe { (egl.terminate)(display) };
        return Err(GalError::backend(egl_error(
            egl,
            "EGL could not choose an isolated OpenGL config",
        )));
    }
    Ok(config)
}

fn create_context(
    egl: &EglFns,
    display: EglDisplay,
    config: EglConfig,
) -> GalResult<EglContextHandle> {
    let context_attribs = [
        EGL_CONTEXT_MAJOR_VERSION,
        3,
        EGL_CONTEXT_MINOR_VERSION,
        3,
        EGL_CONTEXT_OPENGL_PROFILE_MASK,
        EGL_CONTEXT_OPENGL_CORE_PROFILE_BIT,
        EGL_NONE,
    ];
    let mut context =
        unsafe { (egl.create_context)(display, config, EGL_NO_CONTEXT, context_attribs.as_ptr()) };
    if context.is_null() {
        let fallback = [
            EGL_CONTEXT_MAJOR_VERSION,
            3,
            EGL_CONTEXT_MINOR_VERSION,
            3,
            EGL_NONE,
        ];
        context =
            unsafe { (egl.create_context)(display, config, EGL_NO_CONTEXT, fallback.as_ptr()) };
    }
    if context.is_null() {
        return Err(GalError::backend(egl_error(
            egl,
            "EGL OpenGL context creation failed",
        )));
    }
    Ok(context)
}

fn client_supports(egl: &EglFns, extension: &str) -> bool {
    query_extensions(egl, EGL_NO_DISPLAY).contains(extension)
}

fn display_supports(egl: &EglFns, display: EglDisplay, extension: &str) -> bool {
    query_extensions(egl, display).contains(extension)
}

fn query_extensions(egl: &EglFns, display: EglDisplay) -> String {
    let ptr = unsafe { (egl.query_string)(display, EGL_EXTENSIONS) };
    if ptr.is_null() {
        String::new()
    } else {
        unsafe { std::ffi::CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned()
    }
}

fn egl_error(egl: &EglFns, message: &str) -> String {
    let code = unsafe { (egl.get_error)() };
    format!("{message}; eglGetError=0x{code:04x}")
}

struct GlxFns {
    _gl_library: Library,
    _x11_library: Library,
    open_display: XOpenDisplay,
    default_screen: XDefaultScreen,
    close_display: XCloseDisplay,
    x_free: XFree,
    choose_fb_config: GlxChooseFbConfig,
    create_new_context: GlxCreateNewContext,
    create_pbuffer: GlxCreatePbuffer,
    make_context_current: GlxMakeContextCurrent,
    destroy_pbuffer: GlxDestroyPbuffer,
    destroy_context: GlxDestroyContext,
    get_proc_address: GlxGetProcAddress,
}

struct GlxContextState {
    fns: GlxFns,
    display: XDisplay,
    pbuffer: GlxPbuffer,
    context: GlxContextHandle,
}

impl GlxContextState {
    fn new() -> GalResult<Self> {
        let fns = load_glx()?;
        let display = unsafe { (fns.open_display)(ptr::null()) };
        if display.is_null() {
            return Err(GalError::backend(
                "XOpenDisplay returned null for isolated GLX context",
            ));
        }
        let screen = unsafe { (fns.default_screen)(display) };
        let attribs = [
            GLX_X_RENDERABLE,
            1,
            GLX_DRAWABLE_TYPE,
            GLX_PBUFFER_BIT,
            GLX_RENDER_TYPE,
            GLX_RGBA_BIT,
            GLX_RED_SIZE,
            8,
            GLX_GREEN_SIZE,
            8,
            GLX_BLUE_SIZE,
            8,
            GLX_ALPHA_SIZE,
            8,
            GLX_DEPTH_SIZE,
            24,
            GLX_DOUBLEBUFFER,
            0,
            0,
        ];
        let mut config_count = 0;
        let configs =
            unsafe { (fns.choose_fb_config)(display, screen, attribs.as_ptr(), &mut config_count) };
        if configs.is_null() || config_count <= 0 {
            unsafe { (fns.close_display)(display) };
            return Err(GalError::backend(
                "glXChooseFBConfig returned no pbuffer-capable framebuffer config",
            ));
        }
        let config = unsafe { *configs };
        unsafe { (fns.x_free)(configs.cast()) };

        let context =
            unsafe { (fns.create_new_context)(display, config, GLX_RGBA_TYPE, ptr::null_mut(), 1) };
        if context.is_null() {
            unsafe { (fns.close_display)(display) };
            return Err(GalError::backend(
                "glXCreateNewContext failed for isolated Rust OpenGL context",
            ));
        }
        let pbuffer_attribs = [GLX_PBUFFER_WIDTH, 16, GLX_PBUFFER_HEIGHT, 16, 0];
        let pbuffer = unsafe { (fns.create_pbuffer)(display, config, pbuffer_attribs.as_ptr()) };
        if pbuffer == 0 {
            unsafe {
                (fns.destroy_context)(display, context);
                (fns.close_display)(display);
            }
            return Err(GalError::backend(
                "glXCreatePbuffer failed for isolated Rust OpenGL context",
            ));
        }
        let state = Self {
            fns,
            display,
            pbuffer,
            context,
        };
        state.make_current()?;
        Ok(state)
    }

    fn make_current(&self) -> GalResult<()> {
        let ok = unsafe {
            (self.fns.make_context_current)(self.display, self.pbuffer, self.pbuffer, self.context)
        };
        if ok == 0 {
            return Err(GalError::backend(
                "glXMakeContextCurrent failed for isolated Rust OpenGL context",
            ));
        }
        Ok(())
    }

    fn destroy(&mut self) {
        unsafe {
            (self.fns.make_context_current)(self.display, 0, 0, ptr::null_mut());
            if self.pbuffer != 0 {
                (self.fns.destroy_pbuffer)(self.display, self.pbuffer);
            }
            if !self.context.is_null() {
                (self.fns.destroy_context)(self.display, self.context);
            }
            if !self.display.is_null() {
                (self.fns.close_display)(self.display);
            }
        }
    }
}

fn load_glx() -> GalResult<GlxFns> {
    let gl_library = unsafe { Library::new("libGL.so.1") }
        .map_err(|error| GalError::backend(format!("failed to load libGL.so.1: {error}")))?;
    let x11_library = unsafe { Library::new("libX11.so.6") }
        .map_err(|error| GalError::backend(format!("failed to load libX11.so.6: {error}")))?;
    unsafe {
        macro_rules! gl_sym {
            ($name:literal, $ty:ty) => {
                *gl_library
                    .get::<$ty>(concat!($name, "\0").as_bytes())
                    .map_err(|error| {
                        GalError::backend(format!("failed to load GLX symbol {}: {error}", $name))
                    })?
            };
        }
        macro_rules! x_sym {
            ($name:literal, $ty:ty) => {
                *x11_library
                    .get::<$ty>(concat!($name, "\0").as_bytes())
                    .map_err(|error| {
                        GalError::backend(format!("failed to load X11 symbol {}: {error}", $name))
                    })?
            };
        }
        Ok(GlxFns {
            open_display: x_sym!("XOpenDisplay", XOpenDisplay),
            default_screen: x_sym!("XDefaultScreen", XDefaultScreen),
            close_display: x_sym!("XCloseDisplay", XCloseDisplay),
            x_free: x_sym!("XFree", XFree),
            choose_fb_config: gl_sym!("glXChooseFBConfig", GlxChooseFbConfig),
            create_new_context: gl_sym!("glXCreateNewContext", GlxCreateNewContext),
            create_pbuffer: gl_sym!("glXCreatePbuffer", GlxCreatePbuffer),
            make_context_current: gl_sym!("glXMakeContextCurrent", GlxMakeContextCurrent),
            destroy_pbuffer: gl_sym!("glXDestroyPbuffer", GlxDestroyPbuffer),
            destroy_context: gl_sym!("glXDestroyContext", GlxDestroyContext),
            get_proc_address: gl_sym!("glXGetProcAddressARB", GlxGetProcAddress),
            _gl_library: gl_library,
            _x11_library: x11_library,
        })
    }
}

fn load_egl() -> GalResult<EglFns> {
    let library = unsafe { Library::new("libEGL.so.1") }
        .map_err(|error| GalError::backend(format!("failed to load libEGL.so.1: {error}")))?;
    unsafe {
        macro_rules! sym {
            ($name:literal, $ty:ty) => {
                *library
                    .get::<$ty>(concat!($name, "\0").as_bytes())
                    .map_err(|error| {
                        GalError::backend(format!("failed to load EGL symbol {}: {error}", $name))
                    })?
            };
        }
        let get_proc_address = sym!("eglGetProcAddress", EglGetProcAddress);
        let get_platform_display_ext = library
            .get::<EglGetPlatformDisplayExt>(b"eglGetPlatformDisplayEXT\0")
            .ok()
            .map(|symbol| *symbol)
            .or_else(|| {
                let name = CString::new("eglGetPlatformDisplayEXT").ok()?;
                let ptr = get_proc_address(name.as_ptr());
                if ptr.is_null() {
                    None
                } else {
                    Some(std::mem::transmute::<*const c_void, EglGetPlatformDisplayExt>(ptr))
                }
            });
        Ok(EglFns {
            get_display: sym!("eglGetDisplay", EglGetDisplay),
            initialize: sym!("eglInitialize", EglInitialize),
            choose_config: sym!("eglChooseConfig", EglChooseConfig),
            bind_api: sym!("eglBindAPI", EglBindApi),
            create_pbuffer_surface: sym!("eglCreatePbufferSurface", EglCreatePbufferSurface),
            create_context: sym!("eglCreateContext", EglCreateContext),
            make_current: sym!("eglMakeCurrent", EglMakeCurrent),
            destroy_surface: sym!("eglDestroySurface", EglDestroySurface),
            destroy_context: sym!("eglDestroyContext", EglDestroyContext),
            terminate: sym!("eglTerminate", EglTerminate),
            get_proc_address,
            get_error: sym!("eglGetError", EglGetError),
            query_string: sym!("eglQueryString", EglQueryString),
            get_platform_display_ext,
            _library: library,
        })
    }
}
