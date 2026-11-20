//! Hit-Test-Logik der rahmenlosen Titelleiste (plattformneutral, rein rechnerisch).
//!
//! Die UI meldet Rechtecke in logischen Pixeln ([`HitRegions`]). Unter Windows beantwortet die Shell
//! damit `WM_NCHITTEST`: Rand-Resize, Maximieren-Knopf (löst das Snap-Layouts-Flyout aus) und
//! Ziehfläche (`HTCAPTION`, damit Aero-Snap und Doppelklick-Maximieren vom System kommen).

/// Rechteck in logischen Pixeln (Ursprung oben links im Fenster-Client).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    /// Enthält das Rechteck den Punkt (linke/obere Kante inklusive, rechte/untere exklusive)?
    pub fn contains(&self, px: f32, py: f32) -> bool {
        self.w > 0.0
            && self.h > 0.0
            && px >= self.x
            && px < self.x + self.w
            && py >= self.y
            && py < self.y + self.h
    }
}

/// Von der UI gemeldete Flächen (logische Pixel).
#[derive(Clone, Debug, PartialEq)]
pub struct HitRegions {
    /// Ziehflächen (Titelleiste); liefern `HTCAPTION`.
    pub caption: Vec<Rect>,
    /// Bedienelemente innerhalb der Ziehfläche (Minimieren, Schließen, Suchfeld, ...): bleiben
    /// Client-Fläche, damit Slint die Klicks bekommt.
    pub interactive: Vec<Rect>,
    /// Maximieren-Knopf: liefert `HTMAXBUTTON` (Snap-Layouts-Flyout).
    pub maximize_button: Option<Rect>,
    /// Breite des Resize-Rands in logischen Pixeln links, rechts und unten (0 = kein Resize).
    pub resize_border: f32,
    /// Höhe des oberen Resize-Rands (klein halten, damit Knöpfe am oberen Rand nutzbar bleiben).
    pub resize_border_top: f32,
    /// Kantenlänge der Eckgriffe.
    pub corner: f32,
}

impl Default for HitRegions {
    fn default() -> Self {
        Self {
            caption: Vec::new(),
            interactive: Vec::new(),
            maximize_button: None,
            resize_border: 6.0,
            resize_border_top: 4.0,
            corner: 12.0,
        }
    }
}

/// Ergebnis des Hit-Tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitResult {
    Client,
    Caption,
    MaximizeButton,
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl HitResult {
    /// Win32-Wert (`HTCLIENT`, `HTCAPTION`, `HTMAXBUTTON`, `HTLEFT`, ...).
    pub const fn to_win32(self) -> isize {
        match self {
            HitResult::Client => 1,
            HitResult::Caption => 2,
            HitResult::MaximizeButton => 9,
            HitResult::Left => 10,
            HitResult::Right => 11,
            HitResult::Top => 12,
            HitResult::TopLeft => 13,
            HitResult::TopRight => 14,
            HitResult::Bottom => 15,
            HitResult::BottomLeft => 16,
            HitResult::BottomRight => 17,
        }
    }
}

/// Prüft einen Punkt (logische Pixel) gegen die gemeldeten Flächen.
///
/// Reihenfolge: Resize-Ränder (nicht bei maximiertem Fenster), Maximieren-Knopf, Bedienelemente,
/// Ziehfläche, sonst Client. `size` ist die Client-Größe in logischen Pixeln.
pub fn hit_test(
    regions: &HitRegions,
    x: f32,
    y: f32,
    size: (f32, f32),
    maximized: bool,
) -> HitResult {
    let (w, h) = size;
    if !maximized && regions.resize_border > 0.0 && x >= 0.0 && y >= 0.0 && x < w && y < h {
        let b = regions.resize_border;
        let bt = regions.resize_border_top.min(b);
        let c = regions.corner.max(b);
        let left = x < b;
        let right = x >= w - b;
        let top = y < bt;
        let bottom = y >= h - b;
        // Eckgriffe: ein Rand plus die Nähe zur Ecke entlang der anderen Achse.
        let near_left = x < c;
        let near_right = x >= w - c;
        let near_top = y < c.max(bt);
        let near_bottom = y >= h - c;
        if (top && near_left) || (left && near_top) {
            return HitResult::TopLeft;
        }
        if (top && near_right) || (right && near_top) {
            return HitResult::TopRight;
        }
        if (bottom && near_left) || (left && near_bottom) {
            return HitResult::BottomLeft;
        }
        if (bottom && near_right) || (right && near_bottom) {
            return HitResult::BottomRight;
        }
        if top {
            return HitResult::Top;
        }
        if bottom {
            return HitResult::Bottom;
        }
        if left {
            return HitResult::Left;
        }
        if right {
            return HitResult::Right;
        }
    }
    if let Some(r) = regions.maximize_button
        && r.contains(x, y)
    {
        return HitResult::MaximizeButton;
    }
    if regions.interactive.iter().any(|r| r.contains(x, y)) {
        return HitResult::Client;
    }
    if regions.caption.iter().any(|r| r.contains(x, y)) {
        return HitResult::Caption;
    }
    HitResult::Client
}

#[cfg(test)]
mod tests {
    use super::*;

    fn regions() -> HitRegions {
        HitRegions {
            caption: vec![Rect::new(0.0, 0.0, 1000.0, 40.0)],
            interactive: vec![
                Rect::new(900.0, 0.0, 50.0, 40.0),
                Rect::new(950.0, 0.0, 50.0, 40.0),
            ],
            maximize_button: Some(Rect::new(850.0, 0.0, 50.0, 40.0)),
            ..HitRegions::default()
        }
    }

    const SIZE: (f32, f32) = (1000.0, 700.0);

    #[test]
    fn caption_and_client() {
        let r = regions();
        assert_eq!(hit_test(&r, 400.0, 20.0, SIZE, false), HitResult::Caption);
        assert_eq!(hit_test(&r, 400.0, 300.0, SIZE, false), HitResult::Client);
    }

    #[test]
    fn maximize_button_wins_over_caption() {
        let r = regions();
        assert_eq!(
            hit_test(&r, 860.0, 20.0, SIZE, false),
            HitResult::MaximizeButton
        );
        // Rand des Knopfes: rechte Kante exklusiv.
        assert_eq!(
            hit_test(&r, 899.9, 20.0, SIZE, false),
            HitResult::MaximizeButton
        );
        assert_eq!(hit_test(&r, 849.9, 20.0, SIZE, false), HitResult::Caption);
    }

    #[test]
    fn interactive_controls_stay_client() {
        let r = regions();
        assert_eq!(hit_test(&r, 925.0, 20.0, SIZE, false), HitResult::Client);
        assert_eq!(hit_test(&r, 975.0, 20.0, SIZE, false), HitResult::Client);
    }

    #[test]
    fn resize_edges() {
        let r = regions();
        assert_eq!(hit_test(&r, 2.0, 300.0, SIZE, false), HitResult::Left);
        assert_eq!(hit_test(&r, 997.0, 300.0, SIZE, false), HitResult::Right);
        assert_eq!(hit_test(&r, 400.0, 697.0, SIZE, false), HitResult::Bottom);
        assert_eq!(hit_test(&r, 400.0, 1.0, SIZE, false), HitResult::Top);
        // Unterhalb der oberen Randhöhe (4 px) gilt wieder die Ziehfläche.
        assert_eq!(hit_test(&r, 400.0, 5.0, SIZE, false), HitResult::Caption);
    }

    #[test]
    fn resize_corners() {
        let r = regions();
        assert_eq!(hit_test(&r, 1.0, 1.0, SIZE, false), HitResult::TopLeft);
        assert_eq!(hit_test(&r, 3.0, 10.0, SIZE, false), HitResult::TopLeft);
        assert_eq!(hit_test(&r, 998.0, 2.0, SIZE, false), HitResult::TopRight);
        assert_eq!(hit_test(&r, 2.0, 698.0, SIZE, false), HitResult::BottomLeft);
        assert_eq!(
            hit_test(&r, 997.0, 690.0, SIZE, false),
            HitResult::BottomRight
        );
        assert_eq!(hit_test(&r, 8.0, 698.0, SIZE, false), HitResult::BottomLeft);
    }

    #[test]
    fn maximized_has_no_resize_border() {
        let r = regions();
        assert_eq!(hit_test(&r, 2.0, 300.0, SIZE, true), HitResult::Client);
        assert_eq!(hit_test(&r, 1.0, 1.0, SIZE, true), HitResult::Caption);
    }

    #[test]
    fn zero_border_disables_resize() {
        let r = HitRegions {
            resize_border: 0.0,
            ..regions()
        };
        assert_eq!(hit_test(&r, 1.0, 300.0, SIZE, false), HitResult::Client);
    }

    #[test]
    fn outside_is_client() {
        let r = regions();
        assert_eq!(hit_test(&r, -5.0, 20.0, SIZE, false), HitResult::Client);
        assert_eq!(hit_test(&r, 1200.0, 20.0, SIZE, false), HitResult::Client);
    }

    #[test]
    fn win32_codes() {
        assert_eq!(HitResult::Client.to_win32(), 1);
        assert_eq!(HitResult::Caption.to_win32(), 2);
        assert_eq!(HitResult::MaximizeButton.to_win32(), 9);
        assert_eq!(HitResult::Left.to_win32(), 10);
        assert_eq!(HitResult::Right.to_win32(), 11);
        assert_eq!(HitResult::Top.to_win32(), 12);
        assert_eq!(HitResult::TopLeft.to_win32(), 13);
        assert_eq!(HitResult::TopRight.to_win32(), 14);
        assert_eq!(HitResult::Bottom.to_win32(), 15);
        assert_eq!(HitResult::BottomLeft.to_win32(), 16);
        assert_eq!(HitResult::BottomRight.to_win32(), 17);
    }

    #[test]
    fn empty_rect_never_contains() {
        assert!(!Rect::new(0.0, 0.0, 0.0, 10.0).contains(0.0, 1.0));
    }
}
