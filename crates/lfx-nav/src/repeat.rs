//! Tastenwiederholung mit Beschleunigung und Langdruck, zeitgesteuert von außen (ms).

#[derive(Clone, Copy, Debug)]
pub struct RepeatConfig {
    /// Verzögerung bis zur ersten Wiederholung.
    pub delay_ms: u64,
    pub initial_interval_ms: u64,
    pub min_interval_ms: u64,
    /// Faktor je Wiederholung (< 1 = schneller).
    pub decay: f32,
    /// Nach so vielen Wiederholungen verdoppelt sich die Schrittweite.
    pub accel_every: u32,
    pub max_step: u32,
    pub long_press_ms: u64,
}

impl Default for RepeatConfig {
    fn default() -> Self {
        Self {
            delay_ms: 400,
            initial_interval_ms: 150,
            min_interval_ms: 45,
            decay: 0.85,
            accel_every: 5,
            max_step: 8,
            long_press_ms: 700,
        }
    }
}

/// Eine Wiederholung: laufende Nummer (ab 1) und Schrittweite (1, 2, 4, ... für Spulen/Blättern).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RepeatTick {
    pub count: u32,
    pub step: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepeatEvent {
    Repeat(RepeatTick),
    /// Einmalig, wenn die Taste lange genug gehalten wird.
    LongPress,
}

#[derive(Debug)]
struct Held {
    id: u32,
    pressed_at: u64,
    next_at: u64,
    count: u32,
    long_fired: bool,
}

#[derive(Debug)]
pub struct KeyRepeater {
    cfg: RepeatConfig,
    held: Option<Held>,
}

impl KeyRepeater {
    pub fn new(cfg: RepeatConfig) -> Self {
        Self { cfg, held: None }
    }

    fn step_for(&self, count: u32) -> u32 {
        let shift = (count / self.cfg.accel_every.max(1)).min(16);
        (1u32 << shift).min(self.cfg.max_step.max(1))
    }

    fn interval(&self, count: u32) -> u64 {
        let v = self.cfg.initial_interval_ms as f32
            * self.cfg.decay.powi(count.saturating_sub(1) as i32);
        (v.round() as u64).max(self.cfg.min_interval_ms)
    }

    /// Taste gedrückt. `true` bei neuem Druck, `false` bei bereits gehaltener Taste (OS-Repeat).
    /// Eine andere Taste löst die bisherige ab.
    pub fn press(&mut self, id: u32, now_ms: u64) -> bool {
        if self.held.as_ref().is_some_and(|h| h.id == id) {
            return false;
        }
        self.held = Some(Held {
            id,
            pressed_at: now_ms,
            next_at: now_ms + self.cfg.delay_ms,
            count: 0,
            long_fired: false,
        });
        true
    }

    /// Taste losgelassen; `true`, wenn es die gehaltene war.
    pub fn release(&mut self, id: u32) -> bool {
        if self.held.as_ref().is_some_and(|h| h.id == id) {
            self.held = None;
            true
        } else {
            false
        }
    }

    pub fn held_id(&self) -> Option<u32> {
        self.held.as_ref().map(|h| h.id)
    }

    pub fn clear(&mut self) {
        self.held = None;
    }

    /// Nächstes fälliges Ereignis bis `now_ms` (so lange aufrufen, bis `None` kommt).
    pub fn poll(&mut self, now_ms: u64) -> Option<RepeatEvent> {
        let long_ms = self.cfg.long_press_ms;
        let h = self.held.as_ref()?;
        let (next_at, count) = (h.next_at, h.count);
        let long_at = (!h.long_fired).then_some(h.pressed_at + long_ms);
        let long_due = long_at.filter(|l| *l <= now_ms);
        // Langdruck vor einer Wiederholung, wenn er früher fällig wurde.
        if let Some(l) = long_due
            && (next_at > now_ms || l < next_at)
        {
            self.held.as_mut()?.long_fired = true;
            return Some(RepeatEvent::LongPress);
        }
        if next_at <= now_ms {
            let count = count + 1;
            let interval = self.interval(count);
            let step = self.step_for(count);
            let h = self.held.as_mut()?;
            h.count = count;
            h.next_at = next_at + interval;
            return Some(RepeatEvent::Repeat(RepeatTick { count, step }));
        }
        None
    }

    /// Für Systeme mit eigener Tastenwiederholung: zählt eine OS-Wiederholung der gehaltenen Taste
    /// und liefert die Beschleunigung.
    pub fn note_os_repeat(&mut self, id: u32) -> Option<RepeatTick> {
        let count = {
            let h = self.held.as_mut().filter(|h| h.id == id)?;
            h.count += 1;
            h.count
        };
        Some(RepeatTick {
            count,
            step: self.step_for(count),
        })
    }
}
