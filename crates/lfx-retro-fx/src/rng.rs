//! Kleiner deterministischer Zufallsgenerator (Mulberry32, wie im Web für Seeds üblich).

/// Mulberry32: 32-Bit-Zustand, gleiche Folge bei gleichem Seed auf allen Plattformen.
#[derive(Debug, Clone)]
pub struct Rng {
    state: u32,
}

impl Rng {
    pub fn new(seed: u32) -> Self {
        Self { state: seed }
    }

    pub fn next_u32(&mut self) -> u32 {
        self.state = self.state.wrapping_add(0x6D2B_79F5);
        let mut t = self.state;
        t = (t ^ (t >> 15)).wrapping_mul(t | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
        t ^ (t >> 14)
    }

    /// Zufallszahl in [0, 1).
    pub fn next_f64(&mut self) -> f64 {
        f64::from(self.next_u32()) / 4_294_967_296.0
    }

    /// Zufallszahl in [0, 1) als `f32`.
    pub fn next_f32(&mut self) -> f32 {
        self.next_f64() as f32
    }

    /// Zufallszahl in [min, max).
    pub fn range(&mut self, min: f64, max: f64) -> f64 {
        min + self.next_f64() * (max - min)
    }

    /// Ganzzahl in [0, n) (n = 0 liefert 0).
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_f64() * n as f64) as usize
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministisch_und_im_bereich() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..100 {
            let x = a.next_f64();
            assert_eq!(x, b.next_f64());
            assert!((0.0..1.0).contains(&x));
        }
        assert_ne!(Rng::new(1).next_u32(), Rng::new(2).next_u32());
    }

    #[test]
    fn range_und_below() {
        let mut r = Rng::new(7);
        for _ in 0..200 {
            let v = r.range(20.0, 40.0);
            assert!((20.0..40.0).contains(&v));
            assert!(r.below(5) < 5);
        }
        assert_eq!(r.below(0), 0);
    }
}
