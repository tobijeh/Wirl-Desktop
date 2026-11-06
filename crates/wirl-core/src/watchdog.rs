//! Resets internal state if anything is stuck for too long.

pub const STUCK_AFTER_MS: u64 = 30_000;

/// Tracks when the app last left the idle state. The app ticks it every
/// second; recording is excluded because a long dictation is legitimate.
#[derive(Debug, Default)]
pub struct Watchdog {
    busy_since: Option<u64>,
    label: String,
}

impl Watchdog {
    pub fn busy(&mut self, now_ms: u64, label: &str) {
        if self.busy_since.is_none() || self.label != label {
            self.busy_since = Some(now_ms);
            self.label = label.to_string();
        }
    }
    pub fn idle(&mut self) {
        self.busy_since = None;
        self.label.clear();
    }
    /// Returns the stuck stage label if busy for over 30 s.
    pub fn check(&self, now_ms: u64) -> Option<&str> {
        match self.busy_since {
            Some(t) if now_ms.saturating_sub(t) > STUCK_AFTER_MS => Some(&self.label),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trips_after_30s() {
        let mut w = Watchdog::default();
        assert!(w.check(100_000).is_none());
        w.busy(0, "processing");
        assert!(w.check(29_000).is_none());
        w.busy(10_000, "processing"); // same stage doesn't restart the clock
        assert_eq!(w.check(31_000), Some("processing"));
        w.idle();
        assert!(w.check(100_000).is_none());
    }
}
