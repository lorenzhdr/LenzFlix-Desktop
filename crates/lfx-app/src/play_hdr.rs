//! HDR im Player: aus den Videoparametern (mpv `video-params`) und den Eigenschaften der Anzeige wird das
//! Format der Video-Ebene gewählt (SDR `BGRA8`, HDR10/HLG `RGB10A2`+PQ oder scRGB `RGBA16F`).
//!
//! **Nicht am Bildschirm geprüft:** Der Sichttest braucht HDR an der Anzeige und erfolgt erst mit dem Nutzer.
//! Ohne HDR-Anzeige ergibt der Plan immer SDR (mpv bildet HDR-Inhalte dann auf SDR ab).

use lfx_player::VideoParams;
use lfx_shell::{ContentKind, HdrMetadata, OutputHdrInfo, VideoFormat, VideoLayer, plan_format};

/// Ergebnis der Formatwahl.
#[derive(Debug, Clone, PartialEq)]
pub struct HdrPlan {
    pub format: VideoFormat,
    /// Metadaten für `SetHDRMetaData` (nur im PQ-Format).
    pub metadata: Option<HdrMetadata>,
    /// mpv-Ziel (`target-trc`, `target-prim`).
    pub target: (&'static str, &'static str),
    /// Text für das Info-Menü.
    pub status: String,
}

/// Wählt Format und Ziel. `prefer_scrgb` stellt das Modell B/C (scRGB) statt PQ-Durchreichen ein.
pub fn plan(video: &VideoParams, output: &OutputHdrInfo, prefer_scrgb: bool) -> HdrPlan {
    let content_hdr = video.is_hdr();
    let content = if content_hdr {
        ContentKind::Hdr10
    } else {
        ContentKind::Sdr
    };
    let format = plan_format(content, output, prefer_scrgb);
    let metadata = (format == VideoFormat::Rgb10A2Pq).then(|| {
        let mut md = HdrMetadata::bt2020_1000();
        if let Some(c) = video.max_cll.filter(|c| *c > 0.0) {
            md.max_cll = c as f32;
        }
        if let Some(c) = video.max_fall.filter(|c| *c > 0.0) {
            md.max_fall = c as f32;
        }
        md
    });
    let status = match (content_hdr, output.hdr_enabled, format) {
        (false, _, _) => "Aus (SDR-Inhalt)".to_owned(),
        (true, false, _) => "Inhalt ist HDR, Anzeige nicht: Umwandlung auf SDR".to_owned(),
        (true, true, VideoFormat::Rgba16FScRgb) => "An (scRGB)".to_owned(),
        (true, true, _) => "An (HDR10, PQ)".to_owned(),
    };
    HdrPlan {
        format,
        metadata,
        target: format.mpv_target(),
        status,
    }
}

/// Wendet den Plan auf die Video-Ebene und mpv an.
pub fn apply(plan: &HdrPlan, layer: &VideoLayer, mpv: &lfx_player::Player) {
    layer.set_format(plan.format);
    if let Some(md) = plan.metadata {
        layer.set_hdr_metadata(md);
    }
    let m = mpv.mpv();
    // SDR: mpv-Vorgabe beibehalten (`auto`), nur HDR-Ziele ausdrücklich setzen.
    let (trc, prim) = if plan.format.is_hdr() {
        plan.target
    } else {
        ("auto", "auto")
    };
    let _ = m.set_property_string("target-trc", trc);
    let _ = m.set_property_string("target-prim", prim);
    // Ohne Spitzenwert-Berechnung hält die HDR-Ausgabe 60 fps (Messung G3).
    let _ = m.set_property_string("hdr-compute-peak", "no");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hdr_video() -> VideoParams {
        VideoParams {
            width: 3840,
            height: 2160,
            gamma: Some("pq".into()),
            max_cll: Some(1200.0),
            max_fall: Some(300.0),
            ..VideoParams::default()
        }
    }

    fn display(hdr: bool) -> OutputHdrInfo {
        OutputHdrInfo {
            hdr_enabled: hdr,
            ..OutputHdrInfo::default()
        }
    }

    #[test]
    fn sdr_display_stays_sdr_even_for_hdr_content() {
        let p = plan(&hdr_video(), &display(false), false);
        assert_eq!(p.format, VideoFormat::Bgra8Srgb);
        assert!(p.metadata.is_none());
        assert!(p.status.contains("SDR"));
    }

    #[test]
    fn hdr_display_with_hdr_content_uses_pq_and_metadata() {
        let p = plan(&hdr_video(), &display(true), false);
        assert_eq!(p.format, VideoFormat::Rgb10A2Pq);
        assert_eq!(p.target, ("pq", "bt.2020"));
        let md = p.metadata.unwrap();
        assert_eq!(md.max_cll, 1200.0);
        assert_eq!(md.max_fall, 300.0);
        assert_eq!(
            plan(&hdr_video(), &display(true), true).format,
            VideoFormat::Rgba16FScRgb
        );
    }

    #[test]
    fn sdr_content_is_srgb_everywhere() {
        let p = plan(&VideoParams::default(), &display(true), false);
        assert_eq!(p.format, VideoFormat::Bgra8Srgb);
        assert_eq!(p.status, "Aus (SDR-Inhalt)");
    }
}
