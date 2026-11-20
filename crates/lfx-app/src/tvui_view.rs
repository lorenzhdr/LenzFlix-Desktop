//! W2-tvui: Umwandlung der Anzeige-Modelle aus `lfx-tv` in die Slint-Strukturen der TV-Modus-Seite.
//! Rein und ohne Oberflächenzugriff, damit die Texte und Maße testbar sind.

use lfx_tv::channel_list::{Badge, ChannelRow};
use lfx_tv::epg::{self, CellSize, EpgCell, EpgGrid, EpgRow};
use lfx_tv::logos::{channel_color, channel_ink, short_name};
use lfx_tv::model::{Description, Kind, channel_name};
use lfx_tv_schedule::Channel;
use lfx_ui::{TvCellData, TvChanRow, TvEpgRowData, TvMark};
use slint::{Color, ModelRc, SharedString, VecModel};

/// Pixel je Minute im Programm-Raster (Desktop wie im Web).
pub const PPM: f64 = epg::DEFAULT_PPM;

/// `#rrggbb` als Farbe (bei Fehler Grau).
pub fn hex_color(s: &str) -> Color {
    let h = s.trim_start_matches('#');
    if h.len() == 6
        && let Ok(v) = u32::from_str_radix(h, 16)
    {
        return Color::from_rgb_u8((v >> 16) as u8, (v >> 8) as u8, v as u8);
    }
    Color::from_rgb_u8(0x80, 0x80, 0x80)
}

/// Kurzname, Farbe und Schriftfarbe eines Senders.
pub fn badge_parts(c: &Channel) -> (Color, Color, String) {
    (
        hex_color(channel_color(c.no)),
        hex_color(channel_ink(c.no)),
        short_name(c),
    )
}

/// Zeile der Senderliste.
pub fn chan_row(r: &ChannelRow) -> TvChanRow {
    let Badge {
        color,
        ink,
        short,
        name,
    } = &r.badge;
    TvChanRow {
        no: r.no as i32,
        color: hex_color(color),
        ink: hex_color(ink),
        short: short.as_str().into(),
        name: name.as_str().into(),
        title: r.title.as_str().into(),
        sub: r.sub.as_str().into(),
        next: r.next.clone().unwrap_or_default().into(),
        progress: r.progress as f32,
        dim: r.dim,
    }
}

fn cell_kind(k: Kind) -> i32 {
    match k {
        Kind::Program => 0,
        Kind::Fill | Kind::Filler => 1,
        Kind::Closedown => 2,
    }
}

/// Zelle des Rasters (x/w in Pixeln).
pub fn cell(c: &EpgCell) -> TvCellData {
    let size = match epg::classify(c, PPM) {
        CellSize::Normal => 0,
        CellSize::Short => 1,
        CellSize::Tiny => 2,
    };
    TvCellData {
        x: (c.start_min * PPM) as f32,
        w: (c.width_min * PPM) as f32,
        title: c.title.as_str().into(),
        sub: c.sub.as_str().into(),
        time: c.time.as_str().into(),
        kind: cell_kind(c.kind),
        current: c.current,
        past: c.past,
        progress: c.progress_pct.map_or(0.0, |p| (p / 100.0) as f32),
        size,
    }
}

/// Zeile des Rasters.
pub fn epg_row(r: &EpgRow, current_channel: i64) -> TvEpgRowData {
    let (color, ink, short) = badge_parts(&r.channel);
    TvEpgRowData {
        no: r.channel.no as i32,
        color,
        ink,
        short: short.as_str().into(),
        current: r.channel.no == current_channel,
        cells: ModelRc::new(VecModel::from(r.cells.iter().map(cell).collect::<Vec<_>>())),
    }
}

/// Alle Zeilen des Rasters als Modell.
pub fn epg_rows(g: &EpgGrid, current_channel: i64) -> ModelRc<TvEpgRowData> {
    ModelRc::new(VecModel::from(
        g.rows()
            .iter()
            .map(|r| epg_row(r, current_channel))
            .collect::<Vec<_>>(),
    ))
}

/// Zeitskala als Modell.
pub fn scale(g: &EpgGrid) -> ModelRc<TvMark> {
    ModelRc::new(VecModel::from(
        g.scale()
            .iter()
            .map(|m| TvMark {
                x: (m.offset_min * PPM) as f32,
                label: m.label.as_str().into(),
                half: m.half,
            })
            .collect::<Vec<_>>(),
    ))
}

/// Liste von Texten als Modell.
pub fn strings(v: &[String]) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(
        v.iter().map(|s| s.as_str().into()).collect::<Vec<_>>(),
    ))
}

/// Zeile „Danach: …“ unter der Sendung; leer ohne Folgesendung.
pub fn next_line(title: &str, at: &str) -> String {
    if title.is_empty() {
        String::new()
    } else {
        format!("Danach: {title} · {at}")
    }
}

/// Sendername für Beschriftungen (`Sender 7`, wenn der Planer keinen kennt).
pub fn channel_label(c: &Channel) -> String {
    let n = channel_name(c);
    if n.is_empty() {
        format!("Sender {}", c.no)
    } else {
        n
    }
}

/// Meta-Zeile der Beschreibung (`1999 · 95 Min. · Komödie`).
pub fn meta_line(d: &Description) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(y) = d.year {
        parts.push(y.to_string());
    }
    if let Some(m) = d.run_min {
        parts.push(format!("{m} Min."));
    }
    if !d.genres.is_empty() {
        parts.push(d.genres.join(", "));
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn farben() {
        assert_eq!(hex_color("#ff8000"), Color::from_rgb_u8(255, 128, 0));
        assert_eq!(hex_color("xyz"), Color::from_rgb_u8(128, 128, 128));
    }

    #[test]
    fn danach_zeile() {
        assert_eq!(next_line("", "20:15"), "");
        assert_eq!(next_line("Film", "20:15"), "Danach: Film · 20:15");
    }
}
