//! Bounded rebuffering policy. Pausing the HAL preserves queued programme frames.
use std::time::{Duration, Instant};

pub(super) struct Buffering {
    rate: u32,
    since: Option<Instant>,
}
impl Buffering {
    pub fn new(rate: u32, _milliseconds: u32) -> Self {
        Self {
            rate,
            since: None,
        }
    }
    pub fn active(&self) -> bool {
        self.since.is_some()
    }
    pub fn wait(&mut self, queued: u64, capacity: u32, quantum: u32, now: Instant) -> bool {
        let low = self.low(capacity, quantum);
        if self.active() || capacity == 0 || queued > low {
            return false;
        }
        self.since = Some(now);
        true
    }
    fn low(&self, capacity: u32, quantum: u32) -> u64 {
        u64::from(
            (self.rate / 40)
                .max(quantum.saturating_mul(2))
                .min(capacity / 2)
                .max(1),
        )
    }
    /// The allocated driver ring also includes its minimum callback headroom. Prime it
    /// completely; capping at 250 ms silently defeats larger user buffers and large slices.
    pub fn primed(queued: u64, capacity: u32, eof: bool) -> bool {
        queued > 0 && (eof || capacity > 0 && queued >= u64::from(capacity))
    }
    pub fn resume(&mut self, queued: u64, capacity: u32, _quantum: u32, eof: bool) -> bool {
        if self.active() && (eof || Self::primed(queued, capacity, false)) {
            self.since = None;
            return true;
        }
        false
    }
    pub fn timed_out(&mut self, paused: bool, now: Instant) -> bool {
        if paused && self.active() {
            self.since = Some(now);
        }
        self.since
            .is_some_and(|since| now.duration_since(since) >= Duration::from_secs(30))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rebuffering_preserves_hysteresis_and_respects_small_device_capacity() {
        let now = Instant::now();
        let mut gate = Buffering::new(48000, 100);
        assert!(!gate.wait(3000, 4800, 512, now));
        assert!(gate.wait(1024, 4800, 512, now));
        assert!(!gate.resume(2000, 4800, 512, false));
        assert!(gate.resume(4800, 4800, 512, false));
        let mut tiny = Buffering::new(768000, 20);
        assert!(tiny.wait(100, 512, 256, now));
        assert!(tiny.resume(512, 512, 256, false));
        assert!(tiny.wait(0, 512, 256, now));
        assert!(tiny.resume(0, 512, 256, true));
    }
    #[test]
    fn configured_large_buffer_is_fully_primed_before_resuming() {
        let mut gate = Buffering::new(48000, 1000);
        assert!(gate.wait(100, 48000, 512, Instant::now()));
        assert!(!gate.resume(12000, 48000, 512, false), "a one-second buffer must not resume at the old 250 ms cap");
        assert!(gate.resume(48000, 48000, 512, false));
        assert!(!Buffering::primed(4800, 8192, false), "HAL minimum ring exceeds the requested 100 ms");
        assert!(Buffering::primed(8192, 8192, false));
        assert!(Buffering::primed(3, 8192, true));
        assert!(!Buffering::primed(0, 8192, true));
    }
    #[test]
    fn user_pause_does_not_expire_rebuffering_but_source_wait_is_bounded() {
        let now = Instant::now();
        let mut gate = Buffering::new(48000, 100);
        gate.wait(0, 4800, 512, now);
        assert!(!gate.timed_out(true, now + Duration::from_secs(45)));
        assert!(!gate.timed_out(false, now + Duration::from_secs(60)));
        assert!(gate.timed_out(false, now + Duration::from_secs(76)));
    }
}
