//! Progress is periodic; a buffering transition is user-visible immediately.
use super::PlaybackUpdate;
use std::time::{Duration, Instant};

pub(super) struct ReportGate {
    last: Instant,
    state: Option<(bool, bool)>,
}
impl ReportGate {
    pub fn new() -> Self { Self { last: Instant::now(), state: None } }
    pub fn due(&mut self, update: &PlaybackUpdate, now: Instant) -> bool {
        let state = (update.signal_path.processing.iter().any(|p| p.starts_with("Playback buffering:")), update.continuity_lost);
        if self.state != Some(state) || !update.ended.is_empty() || now.duration_since(self.last) >= Duration::from_secs(1) {
            self.state = Some(state);
            self.last = now;
            true
        } else { false }
    }
}
#[cfg(test)] mod tests {
    use super::*;
    #[test]
    fn buffering_and_recovery_bypass_position_throttle() {
        let mut event = PlaybackUpdate { playback_id: "p".into(), position: 1., output_frames: 48000,
            ended: vec![], signal_path: rmusic_audio::output::SignalPath { output: "fixture".into(), sample_rate: 48000,
            channels: 2, exclusive: false, bit_depth: 24, sample_type: "integer".into(), mixer_sample_rate: None,
            bit_perfect_eligible: false, processing: vec![] }, meter: Default::default(), continuity_lost: false };
        let now = Instant::now(); let mut gate = ReportGate::new();
        assert!(gate.due(&event, now));
        assert!(!gate.due(&event, now + Duration::from_millis(20)));
        event.signal_path.processing.push("Playback buffering: source data pending".into());
        assert!(gate.due(&event, now + Duration::from_millis(40)));
        assert!(!gate.due(&event, now + Duration::from_millis(60)));
        event.signal_path.processing.clear();
        assert!(gate.due(&event, now + Duration::from_millis(80)));
        event.continuity_lost = true;
        assert!(gate.due(&event, now + Duration::from_millis(100)));
        assert!(!gate.due(&event, now + Duration::from_millis(120)));
        assert!(gate.due(&event, now + Duration::from_millis(1100)));
    }
}
