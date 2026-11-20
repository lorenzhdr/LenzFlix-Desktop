//! Rechtecke und die Gewichtung der räumlichen Navigation.

use crate::action::NavAction;

#[derive(Clone, Copy, Debug, PartialEq, Default)]
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
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn center(&self) -> (f32, f32) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl Direction {
    pub fn from_action(a: NavAction) -> Option<Self> {
        match a {
            NavAction::Up => Some(Self::Up),
            NavAction::Down => Some(Self::Down),
            NavAction::Left => Some(Self::Left),
            NavAction::Right => Some(Self::Right),
            _ => None,
        }
    }
    pub fn action(self) -> NavAction {
        match self {
            Self::Up => NavAction::Up,
            Self::Down => NavAction::Down,
            Self::Left => NavAction::Left,
            Self::Right => NavAction::Right,
        }
    }
}

/// Gewichtung: Abstand entlang der Bewegungsachse zählt voll, der Querabstand halb.
const CROSS_WEIGHT: f32 = 0.5;
/// Kleiner Anteil des Mittelpunktsabstands (entscheidet bei gleichem Spalt).
const CENTER_WEIGHT: f32 = 0.1;
/// Kandidaten ohne Überlappung auf der Querachse verlieren gegen jeden überlappenden.
const UNALIGNED_PENALTY: f32 = 100_000.0;

/// Rechteck in ein Bezugssystem, in dem "vorwärts" positiv ist:
/// (Längsbeginn, Längsende, Querbeginn, Querende).
fn project(r: &Rect, dir: Direction) -> (f32, f32, f32, f32) {
    match dir {
        Direction::Down => (r.y, r.bottom(), r.x, r.right()),
        Direction::Up => (-r.bottom(), -r.y, r.x, r.right()),
        Direction::Right => (r.x, r.right(), r.y, r.bottom()),
        Direction::Left => (-r.right(), -r.x, r.y, r.bottom()),
    }
}

/// Güte (kleiner = besser) von `to` als Nachbar von `from` in Richtung `dir`;
/// `None`, wenn `to` nicht in dieser Richtung liegt.
pub fn direction_score(from: &Rect, to: &Rect, dir: Direction) -> Option<f32> {
    let (fa0, fa1, fc0, fc1) = project(from, dir);
    let (ta0, ta1, tc0, tc1) = project(to, dir);
    let f_mid = (fa0 + fa1) / 2.0;
    let t_mid = (ta0 + ta1) / 2.0;
    if t_mid <= f_mid || ta1 <= fa1 {
        return None;
    }
    let gap = (ta0 - fa1).max(0.0);
    let cross_gap = (tc0 - fc1).max(fc0 - tc1).max(0.0);
    let cross_center = (((tc0 + tc1) - (fc0 + fc1)) / 2.0).abs();
    let mut s = gap
        + CENTER_WEIGHT * (t_mid - f_mid)
        + CROSS_WEIGHT * cross_gap
        + CENTER_WEIGHT * cross_center;
    if cross_gap > 0.0 {
        s += UNALIGNED_PENALTY;
    }
    Some(s)
}

/// Güte beim Betreten einer Zone: Querabweichung der Mittelpunkte zählt stark,
/// der Längsabstand nur schwach (alle Items einer Zeile liegen gleich weit).
pub fn entry_score(from: &Rect, to: &Rect, dir: Direction) -> f32 {
    let (fx, fy) = from.center();
    let (tx, ty) = to.center();
    let (along, cross) = match dir {
        Direction::Up | Direction::Down => ((ty - fy).abs(), (tx - fx).abs()),
        Direction::Left | Direction::Right => ((tx - fx).abs(), (ty - fy).abs()),
    };
    cross + 0.25 * along
}
