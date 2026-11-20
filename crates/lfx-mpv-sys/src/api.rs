//! Dynamisches Laden von libmpv und Funktionszeiger-Tabelle.

use std::ffi::{OsStr, c_char, c_int, c_void};
use std::fmt;
use std::path::{Path, PathBuf};

use crate::ffi::*;

/// Fehler beim Laden der Bibliothek.
#[derive(Debug)]
pub enum LoadError {
    /// Keine der Kandidaten-Dateien ließ sich laden (Pfade und Fehlertexte).
    NotFound(Vec<(PathBuf, String)>),
    /// Ein erwartetes Symbol fehlt.
    MissingSymbol(&'static str, String),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(list) => {
                write!(f, "libmpv nicht gefunden")?;
                for (p, e) in list {
                    write!(f, "\n  {}: {e}", p.display())?;
                }
                Ok(())
            }
            Self::MissingSymbol(s, e) => write!(f, "Symbol {s} fehlt: {e}"),
        }
    }
}

impl std::error::Error for LoadError {}

/// Dateinamen der Bibliothek je Plattform.
pub fn library_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["libmpv-2.dll", "mpv-2.dll", "libmpv.dll"]
    } else if cfg!(target_os = "macos") {
        &["libmpv.2.dylib", "libmpv.dylib"]
    } else {
        &["libmpv.so.2", "libmpv.so"]
    }
}

/// Verzeichnisname unter `vendor/mpv/` für die aktuelle Plattform.
pub fn target_dir_name() -> &'static str {
    if cfg!(all(windows, target_arch = "aarch64")) {
        "aarch64-pc-windows-msvc"
    } else if cfg!(all(windows, target_arch = "x86_64")) {
        "x86_64-pc-windows-msvc"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else if cfg!(target_os = "macos") {
        "x86_64-apple-darwin"
    } else {
        "unknown"
    }
}

/// Kandidatenpfade: erst neben der exe, dann `vendor/mpv/<target>/` (Dev) aufwärts
/// ab exe-Verzeichnis und Arbeitsverzeichnis, zuletzt der reine Name (System-Suche).
pub fn candidate_paths() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut starts: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(d) = exe.parent()
    {
        dirs.push(d.to_path_buf());
        starts.push(d.to_path_buf());
    }
    if let Ok(cwd) = std::env::current_dir() {
        starts.push(cwd);
    }
    for start in starts {
        for anc in start.ancestors() {
            let v = anc.join("vendor").join("mpv").join(target_dir_name());
            if v.is_dir() && !dirs.contains(&v) {
                dirs.push(v);
            }
        }
    }
    let mut out = Vec::new();
    for d in &dirs {
        for n in library_names() {
            let p = d.join(n);
            if p.is_file() {
                out.push(p);
            }
        }
    }
    for n in library_names() {
        out.push(PathBuf::from(n));
    }
    out
}

macro_rules! api_table {
    ($( $field:ident : fn( $($arg:ty),* ) $(-> $ret:ty)? ),* $(,)?) => {
        /// Geladene libmpv mit allen benötigten Funktionen.
        pub struct Api {
            $( pub $field: unsafe extern "C" fn($($arg),*) $(-> $ret)?, )*
            path: PathBuf,
            _lib: libloading::Library,
        }

        impl Api {
            fn from_library(lib: libloading::Library, path: PathBuf) -> Result<Self, LoadError> {
                $(
                    let $field = {
                        // SAFETY: Signatur nach libmpv-Header; Zeiger lebt, solange `_lib` lebt.
                        let sym: libloading::Symbol<'_, unsafe extern "C" fn($($arg),*) $(-> $ret)?> =
                            unsafe { lib.get(concat!(stringify!($field), "\0").as_bytes()) }
                                .map_err(|e| LoadError::MissingSymbol(stringify!($field), e.to_string()))?;
                        *sym
                    };
                )*
                Ok(Self { $( $field, )* path, _lib: lib })
            }
        }
    };
}

api_table! {
    mpv_client_api_version: fn() -> std::ffi::c_ulong,
    mpv_error_string: fn(c_int) -> *const c_char,
    mpv_free: fn(*mut c_void),
    mpv_create: fn() -> *mut mpv_handle,
    mpv_initialize: fn(*mut mpv_handle) -> c_int,
    mpv_destroy: fn(*mut mpv_handle),
    mpv_terminate_destroy: fn(*mut mpv_handle),
    mpv_free_node_contents: fn(*mut mpv_node),
    mpv_set_option: fn(*mut mpv_handle, *const c_char, c_int, *mut c_void) -> c_int,
    mpv_set_option_string: fn(*mut mpv_handle, *const c_char, *const c_char) -> c_int,
    mpv_command: fn(*mut mpv_handle, *const *const c_char) -> c_int,
    mpv_command_async: fn(*mut mpv_handle, u64, *const *const c_char) -> c_int,
    mpv_command_node: fn(*mut mpv_handle, *mut mpv_node, *mut mpv_node) -> c_int,
    mpv_set_property: fn(*mut mpv_handle, *const c_char, c_int, *mut c_void) -> c_int,
    mpv_set_property_string: fn(*mut mpv_handle, *const c_char, *const c_char) -> c_int,
    mpv_get_property: fn(*mut mpv_handle, *const c_char, c_int, *mut c_void) -> c_int,
    mpv_get_property_string: fn(*mut mpv_handle, *const c_char) -> *mut c_char,
    mpv_observe_property: fn(*mut mpv_handle, u64, *const c_char, c_int) -> c_int,
    mpv_unobserve_property: fn(*mut mpv_handle, u64) -> c_int,
    mpv_request_log_messages: fn(*mut mpv_handle, *const c_char) -> c_int,
    mpv_wait_event: fn(*mut mpv_handle, f64) -> *mut mpv_event,
    mpv_wakeup: fn(*mut mpv_handle),
    mpv_set_wakeup_callback: fn(*mut mpv_handle, Option<mpv_wakeup_fn>, *mut c_void),
    mpv_render_context_create: fn(*mut *mut mpv_render_context, *mut mpv_handle, *mut mpv_render_param) -> c_int,
    mpv_render_context_set_parameter: fn(*mut mpv_render_context, mpv_render_param) -> c_int,
    mpv_render_context_get_info: fn(*mut mpv_render_context, mpv_render_param) -> c_int,
    mpv_render_context_set_update_callback: fn(*mut mpv_render_context, Option<mpv_render_update_fn>, *mut c_void),
    mpv_render_context_update: fn(*mut mpv_render_context) -> u64,
    mpv_render_context_render: fn(*mut mpv_render_context, *mut mpv_render_param) -> c_int,
    mpv_render_context_report_swap: fn(*mut mpv_render_context),
    mpv_render_context_free: fn(*mut mpv_render_context),
}

impl Api {
    /// Sucht und lädt libmpv über [`candidate_paths`].
    pub fn load() -> Result<Self, LoadError> {
        let mut errs = Vec::new();
        for p in candidate_paths() {
            match Self::load_from(&p) {
                Ok(a) => return Ok(a),
                Err(LoadError::NotFound(mut e)) => errs.append(&mut e),
                Err(e) => errs.push((p, e.to_string())),
            }
        }
        Err(LoadError::NotFound(errs))
    }

    /// Lädt libmpv aus einem bestimmten Pfad.
    pub fn load_from(path: &Path) -> Result<Self, LoadError> {
        let lib = open_library(path.as_os_str())
            .map_err(|e| LoadError::NotFound(vec![(path.to_path_buf(), e.to_string())]))?;
        Self::from_library(lib, path.to_path_buf())
    }

    /// Pfad der geladenen Bibliothek.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Client-API-Version (`major << 16 | minor`).
    #[allow(clippy::unnecessary_cast)] // c_ulong ist unter Windows u32, sonst u64
    pub fn client_api_version(&self) -> u32 {
        // SAFETY: parameterlose Funktion ohne Seiteneffekte.
        unsafe { (self.mpv_client_api_version)() as u32 }
    }

    /// Fehlertext zu einem mpv-Fehlercode.
    pub fn error_string(&self, code: i32) -> String {
        // SAFETY: liefert einen statischen, nullterminierten String.
        unsafe {
            let p = (self.mpv_error_string)(code);
            if p.is_null() {
                format!("mpv-Fehler {code}")
            } else {
                std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
            }
        }
    }
}

#[cfg(windows)]
fn open_library(path: &OsStr) -> Result<libloading::Library, libloading::Error> {
    use libloading::os::windows::{LOAD_WITH_ALTERED_SEARCH_PATH, Library};
    // Absolute Pfade: Abhängigkeiten auch im DLL-Verzeichnis suchen.
    let flags = if Path::new(path).is_absolute() {
        LOAD_WITH_ALTERED_SEARCH_PATH
    } else {
        0
    };
    // SAFETY: Laden einer vertrauten Bibliothek; Initialisierungscode von libmpv ist unkritisch.
    unsafe { Library::load_with_flags(path, flags) }.map(Into::into)
}

#[cfg(not(windows))]
fn open_library(path: &OsStr) -> Result<libloading::Library, libloading::Error> {
    // SAFETY: wie oben.
    unsafe { libloading::Library::new(path) }
}
