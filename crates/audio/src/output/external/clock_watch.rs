use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct ClockWatch {
    stale_since: Option<Instant>,
}

impl ClockWatch {
    pub fn stalled(&mut self, now: Instant, clock_age: Option<Duration>) -> bool {
        if clock_age.is_some_and(|age| age <= Duration::from_millis(500)) {
            self.stale_since = None;
            return false;
        }
        // A delayed output worker can read the previous callback just before HAL resumes.
        // Require a full observation window without a fresh callback before teardown.
        let since = self.stale_since.get_or_insert(now);
        now.saturating_duration_since(*since) >= Duration::from_secs(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delayed_worker_can_observe_the_next_callback_before_stopping_output() {
        let now = Instant::now();
        let mut watch = ClockWatch::default();
        assert!(!watch.stalled(now, Some(Duration::from_secs(1))));
        assert!(!watch.stalled(now + Duration::from_millis(10), Some(Duration::from_millis(2))));
        assert!(!watch.stalled(now + Duration::from_secs(3), Some(Duration::from_secs(1))));
    }

    #[test]
    fn absent_callbacks_have_a_bounded_deadline_and_a_fresh_callback_resets_it() {
        let now = Instant::now();
        let mut watch = ClockWatch::default();
        assert!(!watch.stalled(now, None));
        assert!(!watch.stalled(now + Duration::from_millis(999), None));
        assert!(watch.stalled(now + Duration::from_secs(1), None));
        assert!(!watch.stalled(now + Duration::from_secs(2), Some(Duration::ZERO)));
        assert!(!watch.stalled(now + Duration::from_secs(3), None));
    }
}
