//! Rohe C-Typen und Konstanten nach den libmpv-Headern (eigene Formulierung).

#![allow(non_camel_case_types, non_snake_case, missing_docs)]

use std::ffi::{c_char, c_double, c_int, c_void};

#[repr(C)]
pub struct mpv_handle {
    _private: [u8; 0],
}

#[repr(C)]
pub struct mpv_render_context {
    _private: [u8; 0],
}

// --- Fehlercodes (mpv_error) ---
pub const MPV_ERROR_SUCCESS: c_int = 0;
pub const MPV_ERROR_EVENT_QUEUE_FULL: c_int = -1;
pub const MPV_ERROR_NOMEM: c_int = -2;
pub const MPV_ERROR_UNINITIALIZED: c_int = -3;
pub const MPV_ERROR_INVALID_PARAMETER: c_int = -4;
pub const MPV_ERROR_OPTION_NOT_FOUND: c_int = -5;
pub const MPV_ERROR_PROPERTY_UNAVAILABLE: c_int = -10;
pub const MPV_ERROR_UNSUPPORTED: c_int = -18;
pub const MPV_ERROR_NOT_IMPLEMENTED: c_int = -19;
pub const MPV_ERROR_GENERIC: c_int = -20;

// --- mpv_format ---
pub const MPV_FORMAT_NONE: c_int = 0;
pub const MPV_FORMAT_STRING: c_int = 1;
pub const MPV_FORMAT_OSD_STRING: c_int = 2;
pub const MPV_FORMAT_FLAG: c_int = 3;
pub const MPV_FORMAT_INT64: c_int = 4;
pub const MPV_FORMAT_DOUBLE: c_int = 5;
pub const MPV_FORMAT_NODE: c_int = 6;
pub const MPV_FORMAT_NODE_ARRAY: c_int = 7;
pub const MPV_FORMAT_NODE_MAP: c_int = 8;
pub const MPV_FORMAT_BYTE_ARRAY: c_int = 9;

// --- mpv_event_id ---
pub const MPV_EVENT_NONE: c_int = 0;
pub const MPV_EVENT_SHUTDOWN: c_int = 1;
pub const MPV_EVENT_LOG_MESSAGE: c_int = 2;
pub const MPV_EVENT_GET_PROPERTY_REPLY: c_int = 3;
pub const MPV_EVENT_SET_PROPERTY_REPLY: c_int = 4;
pub const MPV_EVENT_COMMAND_REPLY: c_int = 5;
pub const MPV_EVENT_START_FILE: c_int = 6;
pub const MPV_EVENT_END_FILE: c_int = 7;
pub const MPV_EVENT_FILE_LOADED: c_int = 8;
pub const MPV_EVENT_IDLE: c_int = 11;
pub const MPV_EVENT_TICK: c_int = 14;
pub const MPV_EVENT_CLIENT_MESSAGE: c_int = 16;
pub const MPV_EVENT_VIDEO_RECONFIG: c_int = 17;
pub const MPV_EVENT_AUDIO_RECONFIG: c_int = 18;
pub const MPV_EVENT_SEEK: c_int = 20;
pub const MPV_EVENT_PLAYBACK_RESTART: c_int = 21;
pub const MPV_EVENT_PROPERTY_CHANGE: c_int = 22;
pub const MPV_EVENT_QUEUE_OVERFLOW: c_int = 24;
pub const MPV_EVENT_HOOK: c_int = 25;

// --- mpv_end_file_reason ---
pub const MPV_END_FILE_REASON_EOF: c_int = 0;
pub const MPV_END_FILE_REASON_STOP: c_int = 2;
pub const MPV_END_FILE_REASON_QUIT: c_int = 3;
pub const MPV_END_FILE_REASON_ERROR: c_int = 4;
pub const MPV_END_FILE_REASON_REDIRECT: c_int = 5;

#[repr(C)]
#[derive(Clone, Copy)]
pub union mpv_node_union {
    pub string: *mut c_char,
    pub flag: c_int,
    pub int64: i64,
    pub double_: c_double,
    pub list: *mut mpv_node_list,
    pub ba: *mut mpv_byte_array,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct mpv_node {
    pub u: mpv_node_union,
    pub format: c_int,
}

#[repr(C)]
pub struct mpv_node_list {
    pub num: c_int,
    pub values: *mut mpv_node,
    pub keys: *mut *mut c_char,
}

#[repr(C)]
pub struct mpv_byte_array {
    pub data: *mut c_void,
    pub size: usize,
}

#[repr(C)]
pub struct mpv_event {
    pub event_id: c_int,
    pub error: c_int,
    pub reply_userdata: u64,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct mpv_event_property {
    pub name: *const c_char,
    pub format: c_int,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct mpv_event_log_message {
    pub prefix: *const c_char,
    pub level: *const c_char,
    pub text: *const c_char,
    pub log_level: c_int,
}

#[repr(C)]
pub struct mpv_event_start_file {
    pub playlist_entry_id: i64,
}

#[repr(C)]
pub struct mpv_event_end_file {
    pub reason: c_int,
    pub error: c_int,
    pub playlist_entry_id: i64,
    pub playlist_insert_id: i64,
    pub playlist_insert_num_entries: c_int,
}

#[repr(C)]
pub struct mpv_event_client_message {
    pub num_args: c_int,
    pub args: *const *const c_char,
}

// --- render.h ---
pub const MPV_RENDER_PARAM_INVALID: c_int = 0;
pub const MPV_RENDER_PARAM_API_TYPE: c_int = 1;
pub const MPV_RENDER_PARAM_OPENGL_INIT_PARAMS: c_int = 2;
pub const MPV_RENDER_PARAM_OPENGL_FBO: c_int = 3;
pub const MPV_RENDER_PARAM_FLIP_Y: c_int = 4;
pub const MPV_RENDER_PARAM_DEPTH: c_int = 5;
pub const MPV_RENDER_PARAM_ICC_PROFILE: c_int = 6;
pub const MPV_RENDER_PARAM_AMBIENT_LIGHT: c_int = 7;
pub const MPV_RENDER_PARAM_ADVANCED_CONTROL: c_int = 10;
pub const MPV_RENDER_PARAM_NEXT_FRAME_INFO: c_int = 11;
pub const MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME: c_int = 12;
pub const MPV_RENDER_PARAM_SKIP_RENDERING: c_int = 13;
pub const MPV_RENDER_PARAM_SW_SIZE: c_int = 17;
pub const MPV_RENDER_PARAM_SW_FORMAT: c_int = 18;
pub const MPV_RENDER_PARAM_SW_STRIDE: c_int = 19;
pub const MPV_RENDER_PARAM_SW_POINTER: c_int = 20;
/// Seit API 2.6.
pub const MPV_RENDER_PARAM_D3D11_INIT_PARAMS: c_int = 21;
/// Seit API 2.6.
pub const MPV_RENDER_PARAM_D3D11_FBO: c_int = 22;

pub const MPV_RENDER_API_TYPE_D3D11: &[u8] = b"d3d11\0";
pub const MPV_RENDER_API_TYPE_OPENGL: &[u8] = b"opengl\0";
pub const MPV_RENDER_API_TYPE_SW: &[u8] = b"sw\0";

pub const MPV_RENDER_FRAME_INFO_PRESENT: u64 = 1 << 0;
pub const MPV_RENDER_FRAME_INFO_REDRAW: u64 = 1 << 1;
pub const MPV_RENDER_FRAME_INFO_REPEAT: u64 = 1 << 2;
pub const MPV_RENDER_FRAME_INFO_BLOCK_VSYNC: u64 = 1 << 3;

/// Rückgabebit von `mpv_render_context_update`.
pub const MPV_RENDER_UPDATE_FRAME: u64 = 1 << 0;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct mpv_render_param {
    pub type_: c_int,
    pub data: *mut c_void,
}

impl mpv_render_param {
    /// Abschluss-Eintrag der Parameterliste.
    pub const END: Self = Self {
        type_: MPV_RENDER_PARAM_INVALID,
        data: std::ptr::null_mut(),
    };
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct mpv_render_frame_info {
    pub flags: u64,
    pub target_time: i64,
}

pub type mpv_render_update_fn = unsafe extern "C" fn(cb_ctx: *mut c_void);
pub type mpv_wakeup_fn = unsafe extern "C" fn(d: *mut c_void);

// --- render_gl.h ---
#[repr(C)]
pub struct mpv_opengl_init_params {
    pub get_proc_address:
        Option<unsafe extern "C" fn(ctx: *mut c_void, name: *const c_char) -> *mut c_void>,
    pub get_proc_address_ctx: *mut c_void,
}

#[repr(C)]
pub struct mpv_opengl_fbo {
    pub fbo: c_int,
    pub w: c_int,
    pub h: c_int,
    pub internal_format: c_int,
}

// --- render_d3d11.h (API 2.6) ---
#[repr(C)]
pub struct mpv_d3d11_init_params {
    /// `ID3D11Device*`; libmpv hält eine eigene Referenz.
    pub device: *mut c_void,
}

#[repr(C)]
pub struct mpv_d3d11_fbo {
    /// `ID3D11Texture2D*` (Render-Target, DEFAULT, kein MSAA/Array/Mip, kein sRGB-Format).
    pub tex: *mut c_void,
}

/// `MPV_MAKE_VERSION(major, minor)`.
pub const fn make_version(major: u32, minor: u32) -> u32 {
    (major << 16) | minor
}
