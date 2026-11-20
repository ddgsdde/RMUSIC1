use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FadeConfig {
    pub fade_in_ms: u32,
    pub fade_out_ms: u32,
}
impl FadeConfig {
    pub fn validate(&self) -> Result<()> {
        if self.fade_in_ms > 10000 || self.fade_out_ms > 10000 { bail!("fade duration must be 0..10000 ms") }
        Ok(())
    }
    pub fn enabled(&self) -> bool { self.fade_in_ms > 0 || self.fade_out_ms > 0 }
    /// Equal-power envelopes at song boundaries. Seeking does not restart the intro.
    pub fn gain(&self, position: f64, duration: Option<f64>) -> f64 {
        let ramp = |seconds: f64, milliseconds: u32| {
            if milliseconds == 0 {1.} else {
                ((seconds * 1000. / milliseconds as f64).clamp(0., 1.) * std::f64::consts::FRAC_PI_2).sin()
            }
        };
        ramp(position, self.fade_in_ms) * duration.filter(|d| d.is_finite() && *d > 0.)
            .map(|d| ramp(d - position, self.fade_out_ms)).unwrap_or(1.)
    }
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn boundaries_midpoint_and_short_tracks() {
        let fade=FadeConfig{fade_in_ms:2000,fade_out_ms:2000};
        assert_eq!(fade.gain(0.,Some(10.)),0.);
        assert!((fade.gain(1.,Some(10.))-0.5_f64.sqrt()).abs()<1e-12);
        assert_eq!(fade.gain(5.,Some(10.)),1.);
        assert_eq!(fade.gain(10.,Some(10.)),0.);
        assert!((fade.gain(0.5,Some(1.))-0.14644660940672624).abs()<1e-12);
        assert_eq!(FadeConfig::default().gain(0.,Some(10.)),1.);
        assert!(FadeConfig{fade_in_ms:10001,fade_out_ms:0}.validate().is_err());
    }
}
