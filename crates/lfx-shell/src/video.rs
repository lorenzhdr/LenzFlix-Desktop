//! Typen und Formatwahl des Video-Layers (plattformneutral).
//!
//! Der Video-Layer ist unter Windows eine D3D11-Composition-Swapchain unter der UI. Hier stehen
//! die Beschreibung der Formate und die reine Entscheidungslogik, damit sie ohne GPU testbar ist.

use crate::hit::Rect;

/// Ausgabeformat der Video-Swapchain samt Farbraum.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VideoFormat {
    /// `R16G16B16A16_FLOAT` mit scRGB (linear, BT.709-Primärfarben, 1,0 = 80 nit).
    Rgba16FScRgb,
    /// `R10G10B10A2_UNORM` mit PQ (ST 2084) und BT.2020.
    Rgb10A2Pq,
    /// `B8G8R8A8_UNORM` mit sRGB (G2.2, BT.709), der SDR-Normalfall.
    Bgra8Srgb,
}

impl VideoFormat {
    /// Numerischer `DXGI_FORMAT`.
    pub const fn dxgi_format(self) -> u32 {
        match self {
            VideoFormat::Rgba16FScRgb => 10,
            VideoFormat::Rgb10A2Pq => 24,
            VideoFormat::Bgra8Srgb => 87,
        }
    }

    /// Numerischer `DXGI_COLOR_SPACE_TYPE` (`SetColorSpace1`).
    pub const fn dxgi_color_space(self) -> i32 {
        match self {
            // RGB_FULL_G10_NONE_P709
            VideoFormat::Rgba16FScRgb => 1,
            // RGB_FULL_G2084_NONE_P2020
            VideoFormat::Rgb10A2Pq => 12,
            // RGB_FULL_G22_NONE_P709
            VideoFormat::Bgra8Srgb => 0,
        }
    }

    pub const fn is_hdr(self) -> bool {
        !matches!(self, VideoFormat::Bgra8Srgb)
    }

    pub const fn bytes_per_pixel(self) -> u32 {
        match self {
            VideoFormat::Rgba16FScRgb => 8,
            _ => 4,
        }
    }

    /// Passende mpv-Optionen (`target-trc`, `target-prim`) für das Ziel dieses Formats.
    pub const fn mpv_target(self) -> (&'static str, &'static str) {
        match self {
            VideoFormat::Rgba16FScRgb => ("linear", "bt.709"),
            VideoFormat::Rgb10A2Pq => ("pq", "bt.2020"),
            VideoFormat::Bgra8Srgb => ("srgb", "bt.709"),
        }
    }
}

/// HDR10-Metadaten für `SetHDRMetaData` (nur im PQ-Format sinnvoll).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HdrMetadata {
    /// Primärvalenzen und Weißpunkt als CIE-xy: Rot, Grün, Blau, Weiß.
    pub primaries: [(f32, f32); 4],
    pub max_mastering_nits: f32,
    pub min_mastering_nits: f32,
    pub max_cll: f32,
    pub max_fall: f32,
}

impl HdrMetadata {
    /// BT.2020-Primärvalenzen, D65, 1000 nit Spitze: üblicher Rückfall ohne Stream-Metadaten.
    pub const fn bt2020_1000() -> Self {
        Self {
            primaries: [
                (0.708, 0.292),
                (0.170, 0.797),
                (0.131, 0.046),
                (0.3127, 0.3290),
            ],
            max_mastering_nits: 1000.0,
            min_mastering_nits: 0.005,
            max_cll: 1000.0,
            max_fall: 400.0,
        }
    }

    /// Wert in 1/50000 für die DXGI-Struktur (auf `u16` begrenzt).
    pub fn chroma_units(v: f32) -> u16 {
        (v * 50000.0).round().clamp(0.0, 65535.0) as u16
    }
}

/// HDR-Eigenschaften der Anzeige, auf der das Fenster liegt.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OutputHdrInfo {
    /// Windows gibt HDR (PQ/BT.2020) für diese Ausgabe aus.
    pub hdr_enabled: bool,
    pub bits_per_color: u32,
    pub min_luminance_nits: f32,
    pub max_luminance_nits: f32,
    pub max_full_frame_nits: f32,
}

/// Inhaltsart für die Formatwahl.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentKind {
    Sdr,
    Hdr10,
}

/// Wählt das Swapchain-Format.
///
/// - Anzeige ohne HDR: immer `Bgra8Srgb` (mpv bildet HDR-Inhalte auf SDR ab).
/// - Anzeige mit HDR und HDR-Inhalt: `Rgb10A2Pq` (durchgereichtes PQ), oder `Rgba16FScRgb`, wenn
///   `prefer_scrgb` gesetzt ist.
/// - SDR-Inhalt auf HDR-Anzeige bleibt `Bgra8Srgb` (Windows hebt SDR selbst auf den Weißpegel).
pub fn plan_format(
    content: ContentKind,
    output: &OutputHdrInfo,
    prefer_scrgb: bool,
) -> VideoFormat {
    match (output.hdr_enabled, content) {
        (true, ContentKind::Hdr10) if prefer_scrgb => VideoFormat::Rgba16FScRgb,
        (true, ContentKind::Hdr10) => VideoFormat::Rgb10A2Pq,
        _ => VideoFormat::Bgra8Srgb,
    }
}

/// Zielfläche, die der Player pro Bild füllt.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VideoTarget {
    /// Roher `ID3D11Texture2D*` (geborgt, gültig bis zum nächsten `present()`).
    pub texture_raw: *mut std::ffi::c_void,
    pub width: u32,
    pub height: u32,
}

// SAFETY: der Zeiger ist ein reiner Wert; der Besitz bleibt beim Video-Layer, der Player darf ihn
// aus seinem eigenen Thread verwenden (das D3D11-Gerät ist Multithread-geschützt).
unsafe impl Send for VideoTarget {}

/// Rechteck des Video-Layers in logischen Pixeln; `None` heißt Vollbild.
pub type VideoRect = Option<Rect>;

#[cfg(test)]
mod tests {
    use super::*;

    fn out(hdr: bool) -> OutputHdrInfo {
        OutputHdrInfo {
            hdr_enabled: hdr,
            ..OutputHdrInfo::default()
        }
    }

    #[test]
    fn sdr_display_is_always_srgb() {
        assert_eq!(
            plan_format(ContentKind::Sdr, &out(false), false),
            VideoFormat::Bgra8Srgb
        );
        assert_eq!(
            plan_format(ContentKind::Hdr10, &out(false), false),
            VideoFormat::Bgra8Srgb
        );
        assert_eq!(
            plan_format(ContentKind::Hdr10, &out(false), true),
            VideoFormat::Bgra8Srgb
        );
    }

    #[test]
    fn hdr_display_with_hdr_content() {
        assert_eq!(
            plan_format(ContentKind::Hdr10, &out(true), false),
            VideoFormat::Rgb10A2Pq
        );
        assert_eq!(
            plan_format(ContentKind::Hdr10, &out(true), true),
            VideoFormat::Rgba16FScRgb
        );
    }

    #[test]
    fn sdr_content_on_hdr_display_stays_srgb() {
        assert_eq!(
            plan_format(ContentKind::Sdr, &out(true), false),
            VideoFormat::Bgra8Srgb
        );
        assert_eq!(
            plan_format(ContentKind::Sdr, &out(true), true),
            VideoFormat::Bgra8Srgb
        );
    }

    #[test]
    fn format_constants() {
        assert_eq!(VideoFormat::Rgba16FScRgb.dxgi_format(), 10);
        assert_eq!(VideoFormat::Rgb10A2Pq.dxgi_format(), 24);
        assert_eq!(VideoFormat::Bgra8Srgb.dxgi_format(), 87);
        assert_eq!(VideoFormat::Rgba16FScRgb.dxgi_color_space(), 1);
        assert_eq!(VideoFormat::Rgb10A2Pq.dxgi_color_space(), 12);
        assert_eq!(VideoFormat::Bgra8Srgb.dxgi_color_space(), 0);
        assert!(VideoFormat::Rgb10A2Pq.is_hdr());
        assert!(!VideoFormat::Bgra8Srgb.is_hdr());
        assert_eq!(VideoFormat::Rgba16FScRgb.bytes_per_pixel(), 8);
    }

    #[test]
    fn mpv_targets() {
        assert_eq!(VideoFormat::Rgb10A2Pq.mpv_target(), ("pq", "bt.2020"));
        assert_eq!(VideoFormat::Rgba16FScRgb.mpv_target(), ("linear", "bt.709"));
        assert_eq!(VideoFormat::Bgra8Srgb.mpv_target(), ("srgb", "bt.709"));
    }

    #[test]
    fn chroma_units_clamp() {
        assert_eq!(HdrMetadata::chroma_units(0.708), 35400);
        assert_eq!(HdrMetadata::chroma_units(2.0), 65535);
        assert_eq!(HdrMetadata::chroma_units(-1.0), 0);
    }
}
