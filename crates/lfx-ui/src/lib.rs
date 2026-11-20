//! Slint-Oberfläche von LENZFLIX Desktop (Theme-Global, Komponenten-Baukasten, Galerie).
//!
//! `slint::include_modules!()` erzeugt die Typen aus `ui/lib.slint` (`Theme`, `Design`, `LfxCard`-Daten wie
//! [`CardData`], das Fenster [`Gallery`] usw.). [`demo`] liefert erfundene Platzhalter-Daten für Galerie und
//! Snapshot-Tests – nie echte Titel oder Cover.

slint::include_modules!();

pub mod demo {
    //! Synthetische Demo-Daten (erfundene Titel, erzeugte Verlaufs-Cover).

    use super::{AvatarData, CardData, Gallery};
    use slint::{Color, Image, Rgba8Pixel, SharedPixelBuffer, SharedString};

    /// Erzeugt ein deterministisches Platzhalter-Cover (Verlauf plus Streifen) der Größe `w`×`h`.
    #[must_use]
    pub fn placeholder_cover(w: u32, h: u32, seed: u32) -> Image {
        let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(w, h);
        let hue = (seed.wrapping_mul(67) % 360) as f32;
        let (r1, g1, b1) = hsv(hue, 0.55, 0.75);
        let (r2, g2, b2) = hsv((hue + 50.0) % 360.0, 0.7, 0.22);
        let stripes = 3 + seed % 4;
        let pixels = buf.make_mut_slice();
        for y in 0..h {
            for x in 0..w {
                let t = (y as f32 / h as f32) * 0.8 + (x as f32 / w as f32) * 0.2;
                let mut c = [lerp(r1, r2, t), lerp(g1, g2, t), lerp(b1, b2, t)];
                // diagonale Streifen
                if ((x + y) * stripes / w.max(1)).is_multiple_of(2) {
                    for v in &mut c {
                        *v = (*v * 0.88).round();
                    }
                }
                pixels[(y * w + x) as usize] = Rgba8Pixel {
                    r: c[0] as u8,
                    g: c[1] as u8,
                    b: c[2] as u8,
                    a: 255,
                };
            }
        }
        Image::from_rgba8(buf)
    }

    fn lerp(a: f32, b: f32, t: f32) -> f32 {
        a + (b - a) * t
    }

    fn hsv(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
        let c = v * s;
        let hp = h / 60.0;
        let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
        let (r, g, b) = match hp as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = v - c;
        ((r + m) * 255.0, (g + m) * 255.0, (b + m) * 255.0)
    }

    const TITLES: [&str; 12] = [
        "Nebelhafen",
        "Die letzte Station",
        "Kupfermond",
        "Sternenstaub-Express",
        "Alte Bibliothek",
        "Zwölf Uhr nachts",
        "Salzwind",
        "Glasgarten",
        "Stille Kanäle",
        "Fuchsbau",
        "Hinter dem Deich",
        "Papierlaternen",
    ];

    /// Zwölf erfundene Karten (Index 0..5 mit Badge, Fortschritt, Trend zur Schau).
    #[must_use]
    pub fn cards() -> Vec<CardData> {
        TITLES
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let i = i as u32;
                CardData {
                    title: SharedString::from(*t),
                    subtitle: SharedString::from("2026"),
                    image: placeholder_cover(480, 270, i + 1),
                    has_image: true,
                    progress: if i % 3 == 1 {
                        0.35 + 0.1 * (i % 4) as f32
                    } else {
                        0.0
                    },
                    badge: if i.is_multiple_of(4) {
                        "Neu".into()
                    } else {
                        SharedString::new()
                    },
                    trend_rank: if i % 5 == 2 { (i / 5 + 1) as i32 } else { 0 },
                }
            })
            .collect()
    }

    /// Vier erfundene Profile.
    #[must_use]
    pub fn avatars() -> Vec<AvatarData> {
        [
            ("Mara", "M", 0x3a6ea5u32),
            ("Jonas", "J", 0x9b3d3d),
            ("Kiki", "K", 0x4b8f5a),
            ("Gast", "G", 0x7a5ea8),
        ]
        .iter()
        .map(|(n, i, c)| AvatarData {
            name: (*n).into(),
            initial: (*i).into(),
            color: Color::from_rgb_u8((c >> 16) as u8, (c >> 8) as u8, *c as u8),
            image: Image::default(),
            has_image: false,
        })
        .collect()
    }

    /// Füllt die Galerie mit Demo-Daten.
    pub fn fill_gallery(g: &Gallery) {
        g.set_cards(slint::ModelRc::new(slint::VecModel::from(cards())));
        g.set_avatars(slint::ModelRc::new(slint::VecModel::from(avatars())));
    }
}

/// Verzeichnis der Slint-Quellen (`ui/`). Andere Crates nehmen es als Include-Pfad, um die Komponenten
/// (`components/index.slint`) in eigenen Seiten zu importieren.
pub const UI_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/ui");
