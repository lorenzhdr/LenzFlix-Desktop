//! Bildzeit-Statistik für den Messmodus (plattformneutral).

/// Sammelt Bildabstände in Millisekunden und liefert Perzentile.
#[derive(Clone, Debug, Default)]
pub struct FrameTimes {
    intervals_ms: Vec<f32>,
    skip: usize,
}

/// Auswertung einer Messung.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameSummary {
    pub frames: usize,
    pub fps: f64,
    pub p50_ms: f32,
    pub p95_ms: f32,
    pub p99_ms: f32,
    pub max_ms: f32,
    pub over_20ms: usize,
}

impl FrameTimes {
    /// `skip`: so viele erste Bilder werden verworfen (Anlauf).
    pub fn new(skip: usize) -> Self {
        Self {
            intervals_ms: Vec::new(),
            skip,
        }
    }

    pub fn push(&mut self, interval_ms: f32) {
        if self.skip > 0 {
            self.skip -= 1;
        } else {
            self.intervals_ms.push(interval_ms);
        }
    }

    pub fn len(&self) -> usize {
        self.intervals_ms.len()
    }

    pub fn is_empty(&self) -> bool {
        self.intervals_ms.is_empty()
    }

    fn percentile(sorted: &[f32], p: f64) -> f32 {
        if sorted.is_empty() {
            return 0.0;
        }
        sorted[(((sorted.len() - 1) as f64) * p).round() as usize]
    }

    pub fn summary(&self) -> FrameSummary {
        let mut v = self.intervals_ms.clone();
        v.sort_by(f32::total_cmp);
        let total: f64 = v.iter().map(|x| *x as f64).sum();
        FrameSummary {
            frames: v.len(),
            fps: if total > 0.0 {
                v.len() as f64 * 1000.0 / total
            } else {
                0.0
            },
            p50_ms: Self::percentile(&v, 0.50),
            p95_ms: Self::percentile(&v, 0.95),
            p99_ms: Self::percentile(&v, 0.99),
            max_ms: v.last().copied().unwrap_or(0.0),
            over_20ms: v.iter().filter(|x| **x > 20.0).count(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_warmup_and_summarises() {
        let mut t = FrameTimes::new(2);
        t.push(100.0);
        t.push(100.0);
        for _ in 0..98 {
            t.push(10.0);
        }
        t.push(50.0);
        let s = t.summary();
        assert_eq!(s.frames, 99);
        assert_eq!(s.p50_ms, 10.0);
        assert_eq!(s.max_ms, 50.0);
        assert_eq!(s.over_20ms, 1);
        assert!(s.fps > 90.0 && s.fps < 100.0);
    }

    #[test]
    fn empty_is_zero() {
        let t = FrameTimes::new(0);
        assert!(t.is_empty());
        let s = t.summary();
        assert_eq!(s.frames, 0);
        assert_eq!(s.fps, 0.0);
    }
}
