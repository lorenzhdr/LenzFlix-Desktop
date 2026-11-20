//! Schlanke sichere Hülle um die libmpv-Client- und Render-API.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::fmt;
use std::ptr;
use std::sync::Arc;

use crate::api::Api;
use crate::ffi::*;

/// mpv-Fehlercode mit Text.
#[derive(Debug, Clone)]
pub struct MpvError {
    pub code: i32,
    pub text: String,
}

impl fmt::Display for MpvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "mpv-Fehler {}: {}", self.code, self.text)
    }
}

impl std::error::Error for MpvError {}

fn check(api: &Api, code: c_int) -> Result<c_int, MpvError> {
    if code < 0 {
        Err(MpvError {
            code,
            text: api.error_string(code),
        })
    } else {
        Ok(code)
    }
}

fn cstr(s: &str) -> Result<CString, MpvError> {
    CString::new(s).map_err(|_| MpvError {
        code: MPV_ERROR_INVALID_PARAMETER,
        text: "NUL im String".into(),
    })
}

/// Grund für das Ende einer Datei.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    Eof,
    Stop,
    Quit,
    Error(i32),
    Redirect,
    Other(i32),
}

/// Wert einer beobachteten Eigenschaft.
#[derive(Debug, Clone, PartialEq)]
pub enum PropValue {
    None,
    String(String),
    Flag(bool),
    Int(i64),
    Double(f64),
    /// Andere Formate (Node-Bäume) werden hier nicht ausgewertet.
    Unsupported(i32),
}

/// Ereignis aus `mpv_wait_event`.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Zeitüberschreitung, kein Ereignis.
    None,
    Shutdown,
    LogMessage {
        prefix: String,
        level: String,
        text: String,
    },
    StartFile,
    EndFile {
        reason: EndReason,
    },
    FileLoaded,
    Idle,
    Seek,
    PlaybackRestart,
    VideoReconfig,
    AudioReconfig,
    PropertyChange {
        userdata: u64,
        name: String,
        value: PropValue,
    },
    ClientMessage(Vec<String>),
    CommandReply {
        userdata: u64,
        error: i32,
    },
    QueueOverflow,
    Other(i32),
}

/// Ein mpv-Client-Handle. Wird beim Drop per `mpv_terminate_destroy` beendet.
///
/// Muss NACH allen davon erzeugten [`RenderContext`] fallen gelassen werden.
pub struct Mpv {
    api: Arc<Api>,
    ptr: *mut mpv_handle,
}

// SAFETY: Die mpv-Client-API ist laut Doku thread-sicher.
unsafe impl Send for Mpv {}
unsafe impl Sync for Mpv {}

impl Mpv {
    /// Erzeugt ein Handle (noch nicht initialisiert: erst Optionen setzen, dann `initialize`).
    pub fn new(api: Arc<Api>) -> Result<Self, MpvError> {
        // SAFETY: parameterlose Erzeugung.
        let ptr = unsafe { (api.mpv_create)() };
        if ptr.is_null() {
            return Err(MpvError {
                code: MPV_ERROR_NOMEM,
                text: "mpv_create lieferte NULL".into(),
            });
        }
        Ok(Self { api, ptr })
    }

    pub fn api(&self) -> &Arc<Api> {
        &self.api
    }

    /// Rohzeiger (für Render-Kontext und Fremdcode).
    pub fn as_ptr(&self) -> *mut mpv_handle {
        self.ptr
    }

    pub fn initialize(&self) -> Result<(), MpvError> {
        // SAFETY: gültiges Handle.
        check(&self.api, unsafe { (self.api.mpv_initialize)(self.ptr) }).map(|_| ())
    }

    pub fn set_option_string(&self, name: &str, value: &str) -> Result<(), MpvError> {
        let (n, v) = (cstr(name)?, cstr(value)?);
        // SAFETY: gültige C-Strings.
        let r = unsafe { (self.api.mpv_set_option_string)(self.ptr, n.as_ptr(), v.as_ptr()) };
        check(&self.api, r).map(|_| ())
    }

    pub fn set_property_string(&self, name: &str, value: &str) -> Result<(), MpvError> {
        let (n, v) = (cstr(name)?, cstr(value)?);
        // SAFETY: gültige C-Strings.
        let r = unsafe { (self.api.mpv_set_property_string)(self.ptr, n.as_ptr(), v.as_ptr()) };
        check(&self.api, r).map(|_| ())
    }

    pub fn set_property_i64(&self, name: &str, value: i64) -> Result<(), MpvError> {
        let mut v = value;
        self.set_raw(name, MPV_FORMAT_INT64, ptr::from_mut(&mut v).cast())
    }

    pub fn set_property_f64(&self, name: &str, value: f64) -> Result<(), MpvError> {
        let mut v = value;
        self.set_raw(name, MPV_FORMAT_DOUBLE, ptr::from_mut(&mut v).cast())
    }

    pub fn set_property_flag(&self, name: &str, value: bool) -> Result<(), MpvError> {
        let mut v: c_int = value.into();
        self.set_raw(name, MPV_FORMAT_FLAG, ptr::from_mut(&mut v).cast())
    }

    fn set_raw(&self, name: &str, fmt: c_int, data: *mut c_void) -> Result<(), MpvError> {
        let n = cstr(name)?;
        // SAFETY: `data` zeigt auf einen zum Format passenden Wert.
        let r = unsafe { (self.api.mpv_set_property)(self.ptr, n.as_ptr(), fmt, data) };
        check(&self.api, r).map(|_| ())
    }

    pub fn get_property_string(&self, name: &str) -> Result<String, MpvError> {
        let n = cstr(name)?;
        // SAFETY: Rückgabe ist mit mpv_free freizugeben.
        unsafe {
            let p = (self.api.mpv_get_property_string)(self.ptr, n.as_ptr());
            if p.is_null() {
                return Err(MpvError {
                    code: MPV_ERROR_PROPERTY_UNAVAILABLE,
                    text: format!("Eigenschaft {name} nicht lesbar"),
                });
            }
            let s = CStr::from_ptr(p).to_string_lossy().into_owned();
            (self.api.mpv_free)(p.cast());
            Ok(s)
        }
    }

    fn get_raw<T: Default>(&self, name: &str, fmt: c_int) -> Result<T, MpvError> {
        let n = cstr(name)?;
        let mut out = T::default();
        // SAFETY: `out` hat die Größe des angeforderten Formats.
        let r = unsafe {
            (self.api.mpv_get_property)(self.ptr, n.as_ptr(), fmt, ptr::from_mut(&mut out).cast())
        };
        check(&self.api, r)?;
        Ok(out)
    }

    pub fn get_property_i64(&self, name: &str) -> Result<i64, MpvError> {
        self.get_raw(name, MPV_FORMAT_INT64)
    }

    pub fn get_property_f64(&self, name: &str) -> Result<f64, MpvError> {
        self.get_raw(name, MPV_FORMAT_DOUBLE)
    }

    pub fn get_property_flag(&self, name: &str) -> Result<bool, MpvError> {
        self.get_raw::<c_int>(name, MPV_FORMAT_FLAG).map(|v| v != 0)
    }

    /// Befehl synchron ausführen, z. B. `["loadfile", "datei"]`.
    pub fn command(&self, args: &[&str]) -> Result<(), MpvError> {
        let owned = args
            .iter()
            .map(|a| cstr(a))
            .collect::<Result<Vec<_>, _>>()?;
        let mut ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
        ptrs.push(ptr::null());
        // SAFETY: nullterminiertes Array gültiger C-Strings.
        let r = unsafe { (self.api.mpv_command)(self.ptr, ptrs.as_ptr()) };
        check(&self.api, r).map(|_| ())
    }

    /// Befehl asynchron; Antwort als [`Event::CommandReply`] mit `userdata`.
    pub fn command_async(&self, userdata: u64, args: &[&str]) -> Result<(), MpvError> {
        let owned = args
            .iter()
            .map(|a| cstr(a))
            .collect::<Result<Vec<_>, _>>()?;
        let mut ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
        ptrs.push(ptr::null());
        // SAFETY: wie `command`; mpv kopiert die Argumente.
        let r = unsafe { (self.api.mpv_command_async)(self.ptr, userdata, ptrs.as_ptr()) };
        check(&self.api, r).map(|_| ())
    }

    pub fn observe_property(&self, userdata: u64, name: &str, format: i32) -> Result<(), MpvError> {
        let n = cstr(name)?;
        // SAFETY: gültiger C-String.
        let r = unsafe { (self.api.mpv_observe_property)(self.ptr, userdata, n.as_ptr(), format) };
        check(&self.api, r).map(|_| ())
    }

    pub fn unobserve_property(&self, userdata: u64) -> Result<(), MpvError> {
        // SAFETY: gültiges Handle.
        check(&self.api, unsafe {
            (self.api.mpv_unobserve_property)(self.ptr, userdata)
        })
        .map(|_| ())
    }

    pub fn request_log_messages(&self, min_level: &str) -> Result<(), MpvError> {
        let l = cstr(min_level)?;
        // SAFETY: gültiger C-String.
        check(&self.api, unsafe {
            (self.api.mpv_request_log_messages)(self.ptr, l.as_ptr())
        })
        .map(|_| ())
    }

    /// Weckt ein wartendes `wait_event` auf (thread-sicher).
    pub fn wakeup(&self) {
        // SAFETY: gültiges Handle.
        unsafe { (self.api.mpv_wakeup)(self.ptr) }
    }

    /// Wakeup-Callback setzen.
    ///
    /// # Safety
    /// `cb` wird aus beliebigen mpv-Threads aufgerufen und darf weder blockieren noch mpv
    /// aufrufen; `data` muss so lange gültig bleiben, bis der Callback ersetzt oder das
    /// Handle zerstört ist.
    pub unsafe fn set_wakeup_callback(&self, cb: Option<mpv_wakeup_fn>, data: *mut c_void) {
        // SAFETY: Vertrag siehe oben.
        unsafe { (self.api.mpv_set_wakeup_callback)(self.ptr, cb, data) }
    }

    /// Wartet auf das nächste Ereignis (`timeout` in Sekunden, 0 = nicht blockieren).
    pub fn wait_event(&self, timeout: f64) -> Event {
        // SAFETY: Das Ereignis gilt bis zum nächsten wait_event-Aufruf; alles wird kopiert.
        unsafe {
            let ev = (self.api.mpv_wait_event)(self.ptr, timeout);
            if ev.is_null() {
                return Event::None;
            }
            parse_event(&*ev)
        }
    }
}

impl Drop for Mpv {
    fn drop(&mut self) {
        // SAFETY: Handle wird genau einmal zerstört.
        unsafe { (self.api.mpv_terminate_destroy)(self.ptr) }
    }
}

unsafe fn lossy(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        // SAFETY: von mpv gelieferter nullterminierter String.
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }
}

unsafe fn parse_event(ev: &mpv_event) -> Event {
    // SAFETY: `ev.data` zeigt je nach Ereignis-ID auf die jeweilige Struktur.
    unsafe {
        match ev.event_id {
            MPV_EVENT_NONE => Event::None,
            MPV_EVENT_SHUTDOWN => Event::Shutdown,
            MPV_EVENT_LOG_MESSAGE => {
                let m = &*(ev.data as *const mpv_event_log_message);
                Event::LogMessage {
                    prefix: lossy(m.prefix),
                    level: lossy(m.level),
                    text: lossy(m.text),
                }
            }
            MPV_EVENT_START_FILE => Event::StartFile,
            MPV_EVENT_END_FILE => {
                let m = &*(ev.data as *const mpv_event_end_file);
                let reason = match m.reason {
                    MPV_END_FILE_REASON_EOF => EndReason::Eof,
                    MPV_END_FILE_REASON_STOP => EndReason::Stop,
                    MPV_END_FILE_REASON_QUIT => EndReason::Quit,
                    MPV_END_FILE_REASON_ERROR => EndReason::Error(m.error),
                    MPV_END_FILE_REASON_REDIRECT => EndReason::Redirect,
                    o => EndReason::Other(o),
                };
                Event::EndFile { reason }
            }
            MPV_EVENT_FILE_LOADED => Event::FileLoaded,
            MPV_EVENT_IDLE => Event::Idle,
            MPV_EVENT_SEEK => Event::Seek,
            MPV_EVENT_PLAYBACK_RESTART => Event::PlaybackRestart,
            MPV_EVENT_VIDEO_RECONFIG => Event::VideoReconfig,
            MPV_EVENT_AUDIO_RECONFIG => Event::AudioReconfig,
            MPV_EVENT_QUEUE_OVERFLOW => Event::QueueOverflow,
            MPV_EVENT_PROPERTY_CHANGE => {
                let p = &*(ev.data as *const mpv_event_property);
                let value = if p.data.is_null() {
                    PropValue::None
                } else {
                    match p.format {
                        MPV_FORMAT_STRING | MPV_FORMAT_OSD_STRING => {
                            PropValue::String(lossy(*(p.data as *const *const c_char)))
                        }
                        MPV_FORMAT_FLAG => PropValue::Flag(*(p.data as *const c_int) != 0),
                        MPV_FORMAT_INT64 => PropValue::Int(*(p.data as *const i64)),
                        MPV_FORMAT_DOUBLE => PropValue::Double(*(p.data as *const f64)),
                        MPV_FORMAT_NONE => PropValue::None,
                        f => PropValue::Unsupported(f),
                    }
                };
                Event::PropertyChange {
                    userdata: ev.reply_userdata,
                    name: lossy(p.name),
                    value,
                }
            }
            MPV_EVENT_CLIENT_MESSAGE => {
                let m = &*(ev.data as *const mpv_event_client_message);
                let args = (0..m.num_args.max(0) as usize)
                    .map(|i| lossy(*m.args.add(i)))
                    .collect();
                Event::ClientMessage(args)
            }
            MPV_EVENT_COMMAND_REPLY => Event::CommandReply {
                userdata: ev.reply_userdata,
                error: ev.error,
            },
            o => Event::Other(o),
        }
    }
}

/// Render-Kontext (OpenGL, D3D11 oder Software). Vor dem [`Mpv`] freigeben.
pub struct RenderContext {
    api: Arc<Api>,
    ptr: *mut mpv_render_context,
}

// SAFETY: Render-Aufrufe müssen vom Aufrufer serialisiert werden (ein Render-Thread);
// die Kontext-Zeiger selbst dürfen den Thread wechseln.
unsafe impl Send for RenderContext {}

impl RenderContext {
    /// Erzeugt einen Render-Kontext mit dem Typ-Namen (`"d3d11"`, `"opengl"`, `"sw"`).
    ///
    /// # Safety
    /// `extra` muss die zum Typ passenden Parameter (z. B. `D3D11_INIT_PARAMS`) mit gültigen
    /// Zeigern enthalten; sie müssen nur während des Aufrufs leben.
    pub unsafe fn create(
        mpv: &Mpv,
        api_type: &'static [u8],
        extra: &[mpv_render_param],
    ) -> Result<Self, MpvError> {
        debug_assert!(api_type.ends_with(&[0]));
        let mut params = vec![mpv_render_param {
            type_: MPV_RENDER_PARAM_API_TYPE,
            data: api_type.as_ptr() as *mut c_void,
        }];
        params.extend_from_slice(extra);
        params.push(mpv_render_param::END);
        let mut ctx: *mut mpv_render_context = ptr::null_mut();
        // SAFETY: Parameterliste ist durch INVALID abgeschlossen.
        let r =
            unsafe { (mpv.api.mpv_render_context_create)(&mut ctx, mpv.ptr, params.as_mut_ptr()) };
        check(&mpv.api, r)?;
        Ok(Self {
            api: mpv.api.clone(),
            ptr: ctx,
        })
    }

    /// D3D11-Kontext mit einem `ID3D11Device*`.
    ///
    /// # Safety
    /// `device` muss ein gültiger `ID3D11Device*` sein (Multithread-Schutz aktiv).
    pub unsafe fn create_d3d11(mpv: &Mpv, device: *mut c_void) -> Result<Self, MpvError> {
        let mut init = mpv_d3d11_init_params { device };
        let extra = [mpv_render_param {
            type_: MPV_RENDER_PARAM_D3D11_INIT_PARAMS,
            data: ptr::from_mut(&mut init).cast(),
        }];
        // SAFETY: Vertrag der Aufrufer-Funktion.
        unsafe { Self::create(mpv, MPV_RENDER_API_TYPE_D3D11, &extra) }
    }

    pub fn as_ptr(&self) -> *mut mpv_render_context {
        self.ptr
    }

    /// Update-Callback setzen.
    ///
    /// # Safety
    /// Der Callback läuft in mpv-Threads, darf weder blockieren noch mpv aufrufen; `data`
    /// muss bis zum Ersetzen oder Freigeben gültig bleiben.
    pub unsafe fn set_update_callback(&self, cb: Option<mpv_render_update_fn>, data: *mut c_void) {
        // SAFETY: Vertrag siehe oben.
        unsafe { (self.api.mpv_render_context_set_update_callback)(self.ptr, cb, data) }
    }

    /// Update-Flags (`MPV_RENDER_UPDATE_FRAME`).
    pub fn update(&self) -> u64 {
        // SAFETY: gültiger Kontext.
        unsafe { (self.api.mpv_render_context_update)(self.ptr) }
    }

    /// Rendert in das Ziel gemäß `params` (Liste wird hier abgeschlossen).
    ///
    /// # Safety
    /// Die Parameter (z. B. `D3D11_FBO`) müssen auf gültige Ziele zeigen.
    pub unsafe fn render(&self, params: &[mpv_render_param]) -> Result<(), MpvError> {
        let mut v = params.to_vec();
        v.push(mpv_render_param::END);
        // SAFETY: Liste ist abgeschlossen.
        let r = unsafe { (self.api.mpv_render_context_render)(self.ptr, v.as_mut_ptr()) };
        check(&self.api, r).map(|_| ())
    }

    pub fn report_swap(&self) {
        // SAFETY: gültiger Kontext.
        unsafe { (self.api.mpv_render_context_report_swap)(self.ptr) }
    }
}

impl Drop for RenderContext {
    fn drop(&mut self) {
        // SAFETY: wird genau einmal freigegeben.
        unsafe { (self.api.mpv_render_context_free)(self.ptr) }
    }
}
