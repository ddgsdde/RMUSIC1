use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

pub const PCM_RATES: &[u32] = &[
    8000, 11025, 16000, 22050, 24000, 32000, 44100, 48000,
    88200, 96000, 176400, 192000, 352800, 384000, 705600, 768000,
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DspConfig {
    pub enabled: bool,
    pub preamp_db: f64,
    pub balance: f64,
    pub normalize: bool,
    pub target_lufs: f64,
    pub bands: Vec<EqBand>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub extra_eq: Vec<EqStage>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub eq_bypass: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stereo: Option<StereoConfig>,
    pub preset_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sample_rate: Option<u32>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub convolution:Option<crate::impulse::ConvolutionConfig>,
}
impl Default for DspConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            preamp_db: 0.,
            balance: 0.,
            normalize: false,
            target_lufs: -18.,
            bands: vec![],
            extra_eq: vec![],
            eq_bypass: false,
            stereo: None,
            preset_name: None,
            sample_rate: None,
            convolution:None,
        }
    }
}
impl DspConfig {
    pub fn eq_stages(&self) -> impl Iterator<Item = (bool, &[EqBand])> {
        std::iter::once((self.eq_bypass, self.bands.as_slice()))
            .chain(self.extra_eq.iter().map(|stage| (stage.bypass, stage.bands.as_slice())))
    }
    pub fn valid_eq_size(&self) -> bool {
        self.extra_eq.len() <= 3 && self.eq_stages().all(|(_, bands)| bands.len() <= 32)
            && self.eq_stages().map(|(_, bands)| bands.len()).sum::<usize>() <= 64
    }

    pub fn loudness_only(self) -> Self {
        Self {
            enabled: self.normalize,
            normalize: self.normalize,
            target_lufs: self.target_lufs,
            ..Self::default()
        }
    }

    /// Represent the requested LUFS gain with at most the measured true-peak headroom.
    pub fn peak_limited_measurement(&self, measured: Option<f64>, true_peak: Option<f64>) -> Option<f64> {
        measured.filter(|value| value.is_finite()).map(|value| {
            true_peak.filter(|peak| peak.is_finite()).map_or(value, |peak| value.max(self.target_lufs + peak))
        })
    }
}
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EqStage {
    pub bypass: bool,
    pub bands: Vec<EqBand>,
}
#[derive(Debug, Default, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EqFilter {
    #[default]
    Peak,
    LowShelf,
    HighShelf,
    LowPass,
    HighPass,
    BandPass,
    Notch,
    AllPass,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EqBand {
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "is_peak")]
    pub kind: EqFilter,
    pub frequency: f64,
    pub gain_db: f64,
    pub q: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slope_db_oct: Option<u8>,
}
impl EqBand {
    pub fn valid_slope(&self) -> bool {
        self.slope_db_oct.is_none_or(|slope| {
            matches!(self.kind, EqFilter::LowPass | EqFilter::HighPass)
                && matches!(slope, 12 | 24 | 36 | 48)
        })
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StereoConfig {
    pub crossfeed: f64,
    pub crossover_hz: f64,
    pub mono: bool,
    pub invert_left: bool,
    pub invert_right: bool,
}
impl Default for StereoConfig {
    fn default() -> Self {
        Self { crossfeed: 0., crossover_hz: 700., mono: false, invert_left: false, invert_right: false }
    }
}
impl StereoConfig {
    pub fn valid(&self) -> bool {
        (0. ..=0.5).contains(&self.crossfeed) && (200. ..=2000.).contains(&self.crossover_hz)
    }
}
fn is_peak(kind: &EqFilter) -> bool { *kind == EqFilter::Peak }
fn yes() -> bool {
    true
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct Meter {
    pub peak: f64,
    pub clipped_samples: u64,
    pub normalization_applied: bool,
}

#[derive(Clone)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    state: Vec<(f64, f64)>,
}
impl Biquad {
    fn new(band: &EqBand, sample_rate: u32, channels: usize) -> Self {
        let a = 10f64.powf(band.gain_db / 40.);
        let omega = 2. * PI * band.frequency / sample_rate as f64;
        let cos = omega.cos();
        let alpha = omega.sin() / (2. * band.q);
        let shelf = 2. * a.sqrt() * alpha;
        let sum = a + 1.;
        let diff = a - 1.;
        let [b0, b1, b2, a0, a1, a2] = match band.kind {
            EqFilter::Peak => [1. + alpha * a, -2. * cos, 1. - alpha * a, 1. + alpha / a, -2. * cos, 1. - alpha / a],
            EqFilter::LowShelf => [
                a * (sum - diff * cos + shelf), 2. * a * (diff - sum * cos), a * (sum - diff * cos - shelf),
                sum + diff * cos + shelf, -2. * (diff + sum * cos), sum + diff * cos - shelf,
            ],
            EqFilter::HighShelf => [
                a * (sum + diff * cos + shelf), -2. * a * (diff + sum * cos), a * (sum + diff * cos - shelf),
                sum - diff * cos + shelf, 2. * (diff - sum * cos), sum - diff * cos - shelf,
            ],
            EqFilter::LowPass => [(1. - cos) / 2., 1. - cos, (1. - cos) / 2., 1. + alpha, -2. * cos, 1. - alpha],
            EqFilter::HighPass => [(1. + cos) / 2., -(1. + cos), (1. + cos) / 2., 1. + alpha, -2. * cos, 1. - alpha],
            EqFilter::BandPass => [alpha, 0., -alpha, 1. + alpha, -2. * cos, 1. - alpha],
            EqFilter::Notch => [1., -2. * cos, 1., 1. + alpha, -2. * cos, 1. - alpha],
            EqFilter::AllPass => [1. - alpha, -2. * cos, 1. + alpha, 1. + alpha, -2. * cos, 1. - alpha],
        };
        Self {
            b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0,
            state: vec![(0., 0.); channels],
        }
    }
    fn process(&mut self, sample: f64, channel: usize) -> f64 {
        let (z1, z2) = self.state[channel];
        let value = self.b0 * sample + z1;
        self.state[channel] = (
            self.b1 * sample - self.a1 * value + z2,
            self.b2 * sample - self.a2 * value,
        );
        value
    }
}

pub struct DspProcessor {
    config: DspConfig,
    channels: usize,
    gain: f64,
    gain_target:f64,
    gain_remaining:u64,
    filters: Vec<Biquad>,
    normalization_applied: bool,
    sample_rate:u32,
    convolution:Option<crate::convolution::FirConvolver>,
    stereo_low: [f64; 2],
    stereo_alpha: f64,
}
impl DspProcessor {
    /// Attach already rate-matched coefficients. Loading/negotiation belongs to the caller.
    pub fn attach_convolution(&mut self,rate:u32,kernels:Vec<Vec<f64>>)->Result<()> {
        if rate!=self.sample_rate {bail!("FIR impulse response must match the processing sample rate");}
        self.convolution=Some(crate::convolution::FirConvolver::new(kernels,self.channels)?);Ok(())
    }
    pub fn convolution_tail_frames(&self)->usize {
        if self.config.enabled{self.convolution.as_ref().map(|filter|filter.tail_remaining()).unwrap_or(0)}else{0}
    }
    /// Filter tail only: do not reapply programme gain/EQ or credit these as source frames.
    pub fn drain_convolution(&mut self,max_frames:usize)->(Vec<f64>,Meter) {
        let samples=if self.config.enabled {self.convolution.as_mut().map(|filter|filter.drain(max_frames)).unwrap_or_default()}else{vec![]};
        let mut meter=Meter{normalization_applied:self.config.enabled&&self.normalization_applied,..Meter::default()};
        for value in &samples {meter.peak=meter.peak.max(value.abs());meter.clipped_samples+=u64::from(value.abs()>1.);}
        (samples,meter)
    }
    /// A new track can change normalization without clearing continuous filter history.
    pub fn update_loudness(&mut self, measured_lufs: Option<f64>) {
        let measured=measured_lufs.filter(|value|value.is_finite()).filter(|_|self.config.normalize);
        let gain=self.config.preamp_db+measured.map(|value|self.config.target_lufs-value).unwrap_or(0.);
        self.gain=10f64.powf(gain/20.);self.normalization_applied=measured.is_some();
        self.gain_target=self.gain;self.gain_remaining=0;
    }
    pub fn prime_loudness(&mut self, measured: f64, true_peak: Option<f64>) {
        self.update_loudness(self.config.peak_limited_measurement(Some(measured), true_peak));
    }
    /// Apply a newly measured current track over 500 ms without resetting filters or time.
    pub fn smooth_loudness(&mut self,measured:f64,true_peak:Option<f64>){
        if !self.config.enabled||!self.config.normalize||!measured.is_finite(){return;}
        let mut gain=self.config.target_lufs-measured;
        if let Some(peak)=true_peak.filter(|v|v.is_finite()){gain=gain.min(-peak);}
        self.gain_target=10f64.powf((self.config.preamp_db+gain)/20.);
        self.gain_remaining=(self.sample_rate as u64/2).max(1);self.normalization_applied=true;
    }
    /// Normalization requires a measured integrated LUFS value. No guessed gain is applied.
    pub fn new(
        config: DspConfig,
        rate: u32,
        channels: usize,
        measured_lufs: Option<f64>,
    ) -> Result<Self> {
        if rate < 8000 || channels == 0 || channels > 32 {
            bail!("invalid audio format")
        }
        if let Some(selection)=&config.convolution {selection.validate()?;}
        if !config.preamp_db.is_finite()
            || !(-60. ..=24.).contains(&config.preamp_db)
            || !config.balance.is_finite()
            || !(-1. ..=1.).contains(&config.balance)
            || !config.target_lufs.is_finite()
            || !(-40. ..=-5.).contains(&config.target_lufs)
            || !config.valid_eq_size()
            || config.sample_rate.is_some_and(|rate| !PCM_RATES.contains(&rate))
            || config.stereo.as_ref().is_some_and(|stereo| !stereo.valid())
        {
            bail!("invalid DSP configuration")
        }
        for band in config.eq_stages().flat_map(|(_, bands)| bands) {
            if !band.frequency.is_finite()
                || !(10. ..=192000.).contains(&band.frequency)
                || !band.gain_db.is_finite()
                || !(-30. ..=30.).contains(&band.gain_db)
                || !band.q.is_finite()
                || !(0.1..=30.).contains(&band.q)
                || !band.valid_slope()
            {
                bail!("invalid EQ band")
            }
        }
        let normalization = measured_lufs
            .filter(|x| x.is_finite())
            .filter(|_| config.normalize);
        let gain_db =
            config.preamp_db + normalization.map(|x| config.target_lufs - x).unwrap_or(0.);
        let filters = config
            .eq_stages()
            .filter(|(bypass, _)| !bypass)
            .flat_map(|(_, bands)| bands)
            .filter(|b| b.enabled && b.frequency < rate as f64 / 2.)
            .flat_map(|b| {
                let sections = b.slope_db_oct.unwrap_or(12) / 12;
                (0..sections).map(move |index| {
                    let mut section = b.clone();
                    if sections > 1 {
                        // Butterworth pole Qs, scaled by the user's resonance setting.
                        let angle = PI * f64::from(2 * index + 1) / f64::from(4 * sections);
                        section.q *= std::f64::consts::FRAC_1_SQRT_2 / angle.cos();
                    }
                    Biquad::new(&section, rate, channels)
                })
            })
            .collect();
        let stereo_alpha = config.stereo.as_ref().map_or(0., |s|
            1. - (-2. * PI * s.crossover_hz / rate as f64).exp());
        Ok(Self {
            config,
            channels,
            gain: 10f64.powf(gain_db / 20.),
            gain_target:10f64.powf(gain_db / 20.),
            gain_remaining:0,
            filters,
            normalization_applied: normalization.is_some(),
            sample_rate:rate,
            convolution:None,
            stereo_low: [0.; 2],
            stereo_alpha,
        })
    }
    /// Disabled DSP leaves every sample bit unchanged, including floating point signed zero.
    /// Clipping is measured before the output stage; this processor does not hide it with a limiter.
    pub fn process(&mut self, samples: &mut [f64]) -> Result<Meter> {
        self.process_enveloped(samples,|_|1.)
    }
    pub(crate) fn has_convolution(&self)->bool{self.config.enabled&&self.convolution.is_some()}
    pub fn process_enveloped(&mut self, samples: &mut [f64],envelope:impl Fn(usize)->f64) -> Result<Meter> {
        if self.config.enabled&&self.config.convolution.is_some()&&self.convolution.is_none(){bail!("卷积响应尚未加载，已拒绝处理");}
        if samples.len() % self.channels != 0 {
            bail!("incomplete interleaved frame")
        }
        let mut meter = Meter {
            normalization_applied: self.config.enabled && self.normalization_applied,
            ..Meter::default()
        };
        let mut frame_gain=1.;
        if let Some(stereo) = self.config.stereo.as_ref().filter(|_| self.config.enabled && self.channels == 2) {
            for frame in samples.chunks_exact_mut(2) {
                if stereo.crossfeed > 0. {
                    self.stereo_low[0] += self.stereo_alpha * (frame[0] - self.stereo_low[0]);
                    self.stereo_low[1] += self.stereo_alpha * (frame[1] - self.stereo_low[1]);
                    let exchange = stereo.crossfeed * (self.stereo_low[1] - self.stereo_low[0]);
                    frame[0] += exchange;
                    frame[1] -= exchange;
                }
                if stereo.mono { frame.fill(0.5 * (frame[0] + frame[1])); }
                if stereo.invert_left { frame[0] = -frame[0]; }
                if stereo.invert_right { frame[1] = -frame[1]; }
            }
        }
        for (i, sample) in samples.iter_mut().enumerate() {
            if self.config.enabled {
                let channel = i % self.channels;
                if channel==0&&self.gain_remaining>0{self.gain+=(self.gain_target-self.gain)/self.gain_remaining as f64;self.gain_remaining-=1;}
                let mut value = *sample * self.gain;
                if self.channels == 2 {
                    value *= match channel {
                        0 if self.config.balance > 0. => 1. - self.config.balance,
                        1 if self.config.balance < 0. => 1. + self.config.balance,
                        _ => 1.,
                    };
                }
                for filter in &mut self.filters {
                    value = filter.process(value, channel);
                }
                if channel==0 {frame_gain=envelope(i/self.channels);}
                *sample = value*frame_gain;
            }
            meter.peak = meter.peak.max(sample.abs());
            if sample.abs() > 1. {
                meter.clipped_samples += 1;
            }
        }
        if self.config.enabled {
            if let Some(filter)=&mut self.convolution {
                filter.process(samples)?;meter.peak=0.;meter.clipped_samples=0;
                for value in samples {meter.peak=meter.peak.max(value.abs());meter.clipped_samples+=u64::from(value.abs()>1.);}
            }
        }
        Ok(meter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn live_loudness_ramps_in_half_second_and_keeps_stereo_frames(){
        let mut dsp=DspProcessor::new(DspConfig{enabled:true,normalize:true,target_lufs:-18.,..Default::default()},48000,2,None).unwrap();
        dsp.smooth_loudness(-24.,Some(-10.));let mut samples=vec![0.1;48000];dsp.process(&mut samples).unwrap();
        assert!((samples[0]-0.1).abs()<0.00001);assert_eq!(samples[0],samples[1]);assert!((samples[47998]-0.1*10f64.powf(6./20.)).abs()<1e-12);
        let mut next=[0.1,0.1];dsp.process(&mut next).unwrap();assert_eq!(next[0],samples[47998]);
    }
    #[test]
    fn configured_fir_rejects_unloaded_processing_before_mutating_samples(){
        let mut config=DspConfig{enabled:true,convolution:Some(crate::impulse::ConvolutionConfig{response_ids:vec!["0".repeat(64)]}),..Default::default()};
        let mut dsp=DspProcessor::new(config.clone(),48000,2,None).unwrap();let mut values=[0.25,-0.];let bits=values.map(f64::to_bits);
        assert!(dsp.process(&mut values).unwrap_err().to_string().contains("尚未加载"));assert_eq!(values.map(f64::to_bits),bits);
        dsp.attach_convolution(48000,vec![vec![2.]]).unwrap();dsp.process(&mut values).unwrap();assert_eq!(values[0],0.5);
        config.enabled=false;let mut bypass=DspProcessor::new(config,48000,2,None).unwrap();let mut values=[0.25,-0.];bypass.process(&mut values).unwrap();assert_eq!(values.map(f64::to_bits),bits);
    }
    #[test]
    fn optional_fir_reference_keeps_legacy_dsp_json_and_rejects_unknown_processing_policies(){
        let config:DspConfig=serde_json::from_value(serde_json::json!({"enabled":true,"preamp_db":-3})).unwrap();assert!(config.convolution.is_none());assert!(serde_json::to_value(config).unwrap().get("convolution").is_none());
        let value=serde_json::json!({"enabled":true,"convolution":{"response_ids":["0".repeat(64)]}});let config:DspConfig=serde_json::from_value(value.clone()).unwrap();assert_eq!(serde_json::to_value(config).unwrap()["convolution"],value["convolution"]);
        let mut bad=value;bad["convolution"]["normalize"]=serde_json::json!(true);assert!(serde_json::from_value::<DspConfig>(bad).is_err());
    }
    #[test]
    fn convolution_tail_uses_original_programme_gain_and_output_meter(){
        let config=DspConfig{enabled:true,preamp_db:6.020599913279624,..DspConfig::default()};
        let mut dsp=DspProcessor::new(config,48000,2,None).unwrap();let mut kernel=vec![0.;65];kernel[0]=0.25;kernel[64]=0.5;
        dsp.attach_convolution(48000,vec![kernel]).unwrap();let mut programme=[1.,-1.];let meter=dsp.process(&mut programme).unwrap();
        assert!((programme[0]-0.5).abs()<1e-12);assert!((meter.peak-0.5).abs()<1e-12);assert_eq!(meter.clipped_samples,0);
        assert_eq!(dsp.convolution_tail_frames(),64);let(tail,meter)=dsp.drain_convolution(100);
        assert_eq!(tail.len(),128);assert!((tail[126]-1.).abs()<1e-12);assert!((tail[127]+1.).abs()<1e-12);assert!((meter.peak-1.).abs()<1e-12);
        assert_eq!(dsp.convolution_tail_frames(),0);assert!(dsp.drain_convolution(100).0.is_empty());
        let mut dsp=DspProcessor::new(DspConfig{enabled:true,..DspConfig::default()},48000,2,None).unwrap();dsp.attach_convolution(48000,vec![vec![2.]]).unwrap();
        let mut samples=[0.75,-0.75];let meter=dsp.process(&mut samples).unwrap();assert_eq!(samples,[1.5,-1.5]);assert_eq!(meter.clipped_samples,2);
    }
    #[test]
    fn disabled_convolution_preserves_bits_and_rate_mismatch_does_not_apply(){
        let mut dsp=DspProcessor::new(DspConfig::default(),48000,2,None).unwrap();
        assert!(dsp.attach_convolution(44100,vec![vec![2.]]).is_err());dsp.attach_convolution(48000,vec![vec![2.,0.5]]).unwrap();
        let mut samples=[-0.,0.,f64::MIN_POSITIVE,0.123456789012345];let bits=samples.map(f64::to_bits);
        dsp.process(&mut samples).unwrap();assert_eq!(samples.map(f64::to_bits),bits);assert_eq!(dsp.convolution_tail_frames(),0);assert!(dsp.drain_convolution(100).0.is_empty());
    }
    #[test]
    fn loudness_update_does_not_reset_or_rescale_prior_convolution_history(){
        let config=DspConfig{enabled:true,normalize:true,..DspConfig::default()};let mut dsp=DspProcessor::new(config,48000,1,None).unwrap();
        let mut kernel=vec![0.;65];kernel[0]=1.;kernel[64]=0.5;dsp.attach_convolution(48000,vec![kernel]).unwrap();
        let mut first=[1.];dsp.process(&mut first).unwrap();assert_eq!(first,[1.]);dsp.update_loudness(Some(-12.));
        let mut next=[0.;64];dsp.process(&mut next).unwrap();assert!((next[63]-0.5).abs()<1e-12);
        let mut actual=[1.];dsp.process(&mut actual).unwrap();assert!((actual[0]-10_f64.powf(-6./20.)).abs()<1e-12);
    }
    #[test]
    fn disabled_is_bit_exact() {
        let mut samples = vec![-0., 0., -1., 1., f64::MIN_POSITIVE, 0.123456789012345];
        let bits: Vec<_> = samples.iter().map(|s: &f64| s.to_bits()).collect();
        let mut dsp = DspProcessor::new(DspConfig::default(), 48000, 2, None).unwrap();
        dsp.process(&mut samples).unwrap();
        assert_eq!(
            bits,
            samples.iter().map(|s| s.to_bits()).collect::<Vec<_>>()
        );
    }
    #[test]
    fn track_loudness_changes_preserve_equalizer_history() {
        let mut config=DspConfig::default();config.enabled=true;config.normalize=true;config.bands=vec![EqBand {enabled:true,kind:EqFilter::Peak,frequency:1000.,gain_db:6.,q:1., slope_db_oct: None }];
        let mut changed=DspProcessor::new(config.clone(),48000,1,None).unwrap();let mut reference=DspProcessor::new(config,48000,1,None).unwrap();
        changed.process(&mut [0.5]).unwrap();reference.process(&mut [0.5]).unwrap();changed.update_loudness(Some(-12.));
        let mut tail=[0.;32];let mut expected=[0.;32];changed.process(&mut tail).unwrap();reference.process(&mut expected).unwrap();assert_eq!(tail,expected);assert!(tail.iter().any(|sample|sample.abs()>0.001));
    }
    #[test]
    fn gain_balance_and_clip_meter() {
        let config = DspConfig {
            enabled: true,
            preamp_db: 6.020599913279624,
            balance: 0.5,
            ..DspConfig::default()
        };
        let mut dsp = DspProcessor::new(config, 48000, 2, None).unwrap();
        let mut samples = [0.75, 0.75, -0.75, -0.75];
        let meter = dsp.process(&mut samples).unwrap();
        assert!((samples[0] - 0.75).abs() < 1e-12);
        assert!((samples[1] - 1.5).abs() < 1e-12);
        assert_eq!(meter.clipped_samples, 2);
    }
    #[test]
    fn normalization_needs_measurement() {
        let config = DspConfig {
            enabled: true,
            normalize: true,
            ..DspConfig::default()
        };
        let mut missing = DspProcessor::new(config.clone(), 48000, 1, None).unwrap();
        let mut sample = [0.5];
        assert!(!missing.process(&mut sample).unwrap().normalization_applied);
        assert_eq!(sample, [0.5]);
        let mut measured = DspProcessor::new(config, 48000, 1, Some(-24.)).unwrap();
        assert!(measured.process(&mut sample).unwrap().normalization_applied);
        assert!((sample[0] - 0.9976311574844398).abs() < 1e-12);
    }

    #[test]
    fn portable_loudness_respects_true_peak_and_keeps_other_effects_out() {
        let config = DspConfig {
            enabled: true, normalize: true, target_lufs: -14.,
            preamp_db: 6., sample_rate: Some(96000),
            ..Default::default()
        }.loudness_only();
        assert_eq!(config.preamp_db, 0.);
        assert_eq!(config.sample_rate, None);
        let measured = config.peak_limited_measurement(Some(-24.), Some(-2.));
        let mut processor = DspProcessor::new(config, 48000, 1, measured).unwrap();
        let mut frame = [0.25];
        assert!(processor.process(&mut frame).unwrap().normalization_applied);
        assert!((frame[0] - 0.25 * 10f64.powf(2. / 20.)).abs() < 1e-12);
    }
    #[test]
    fn peq_matches_requested_center_gain_and_preserves_chunk_state() {
        let config = DspConfig {
            enabled: true,
            bands: vec![EqBand {
                kind: EqFilter::Peak,
                enabled: true,
                frequency: 1000.,
                gain_db: 6.,
                q: 2.,
                slope_db_oct: None,
            }],
            ..DspConfig::default()
        };
        let input: Vec<f64> = (0..48000)
            .map(|n| 0.1 * (2. * PI * 1000. * n as f64 / 48000.).sin())
            .collect();
        let mut whole = input.clone();
        DspProcessor::new(config.clone(), 48000, 1, None)
            .unwrap()
            .process(&mut whole)
            .unwrap();
        let mut chunked = input.clone();
        let mut processor = DspProcessor::new(config, 48000, 1, None).unwrap();
        for chunk in chunked.chunks_mut(127) {
            processor.process(chunk).unwrap();
        }
        assert_eq!(whole, chunked);
        let rms =
            |data: &[f64]| (data.iter().map(|x| x * x).sum::<f64>() / data.len() as f64).sqrt();
        let gain_db = 20. * (rms(&whole[24000..]) / rms(&input[24000..])).log10();
        assert!((gain_db - 6.).abs() < 0.01);
    }
    #[test]
    fn shelves_and_pass_filters_have_the_expected_dc_response() {
        for (kind, expected) in [
            (EqFilter::LowShelf, 10_f64.powf(-6. / 20.)),
            (EqFilter::HighShelf, 1.),
            (EqFilter::LowPass, 1.),
            (EqFilter::HighPass, 0.),
            (EqFilter::BandPass, 0.),
            (EqFilter::Notch, 1.),
            (EqFilter::AllPass, 1.),
        ] {
            let band = EqBand { enabled: true, kind, frequency: 1000., gain_db: -6., q: 0.707, slope_db_oct: None };
            let mut filter = Biquad::new(&band, 48000, 1);
            let response = (0..4096).map(|_| filter.process(0.25, 0)).last().unwrap();
            assert!((response - 0.25 * expected).abs() < 1e-10, "{kind:?}: {response}");
        }
    }

    #[test]
    fn eq_skips_nyquist_bands_and_rejects_nonfinite_values() {
        let mut config = DspConfig {
            enabled: true,
            bands: vec![EqBand {
                kind: EqFilter::Peak,
                enabled: true,
                frequency: 24000.,
                gain_db: 2.,
                q: 1.,
                slope_db_oct: None,
            }],
            ..DspConfig::default()
        };
        let mut samples = [0.1, -0.1];
        DspProcessor::new(config.clone(), 48000, 2, None).unwrap().process(&mut samples).unwrap();
        assert_eq!(samples, [0.1, -0.1]);
        config.bands[0].frequency = f64::NAN;
        assert!(DspProcessor::new(config, 48000, 2, None).is_err());
    }

    #[test]
    fn high_pass_slope_and_eq_bypass_change_the_rendered_signal() {
        let samples: Vec<_> = (0..48_000).map(|i| (2. * PI * 100. * i as f64 / 48_000.).sin()).collect();
        let power = |slope, bypass| {
            let band = EqBand { enabled:true, kind:EqFilter::HighPass, frequency:1000., q:std::f64::consts::FRAC_1_SQRT_2, gain_db:0., slope_db_oct:Some(slope) };
            let mut renderer = DspProcessor::new(DspConfig { enabled:true, eq_bypass:bypass, bands:vec![band], ..Default::default() }, 48_000, 1, None).unwrap();
            let mut output = samples.clone();
            renderer.process(&mut output).unwrap();
            if bypass { assert_eq!(output, samples); }
            output[24_000..].iter().map(|value| value * value).sum::<f64>()
        };
        assert!(power(24, false) < power(12, false) / 1000.);
        assert!(power(48, false) < power(24, false) / 1_000_000.);
        power(48, true);
    }
    #[test]
    fn equalizer_modules_cascade_and_bypass_independently() {
        let band = EqBand { enabled:true, kind:EqFilter::Peak, frequency:1000., q:1., gain_db:6., slope_db_oct:None };
        let input: Vec<_> = (0..4800).map(|i| 0.1 * (2. * PI * 1000. * i as f64 / 48000.).sin()).collect();
        let gain = |bypass| {
            let config = DspConfig { enabled:true, bands:vec![band.clone()], extra_eq:vec![EqStage { bypass, bands:vec![band.clone()] }], ..Default::default() };
            let mut processor = DspProcessor::new(config,48000,1,None).unwrap();
            let mut output = input.clone();
            processor.process(&mut output).unwrap();
            output[2400..].iter().map(|v| v.abs()).fold(0.,f64::max) / 0.1
        };
        assert!((20. * gain(false).log10() - 12.).abs() < 1e-8);
        assert!((20. * gain(true).log10() - 6.).abs() < 1e-8);
    }

    #[test]
    fn stereo_crossfeed_mono_and_polarity_are_applied_before_output() {
        let stereo = StereoConfig { crossfeed:0.25, ..Default::default() };
        let mut signal = [1., 0.].repeat(1000);
        let mut renderer = DspProcessor::new(DspConfig { enabled:true, stereo:Some(stereo.clone()), ..Default::default() }, 48_000, 2, None).unwrap();
        renderer.process(&mut signal).unwrap();
        assert!((signal[1998] - 0.75).abs() < 1e-9);
        assert!((signal[1999] - 0.25).abs() < 1e-9);
        let stereo = StereoConfig { mono:true, invert_right:true, ..stereo };
        let mut renderer = DspProcessor::new(DspConfig { enabled:true, stereo:Some(stereo), ..Default::default() }, 48_000, 2, None).unwrap();
        renderer.process(&mut [1., 0.]).unwrap();
        let mut frame = [0.6, 0.8];
        renderer.process(&mut frame).unwrap();
        assert!((frame[0] - 0.7).abs() < 1e-9);
        assert!((frame[1] + 0.7).abs() < 1e-9);
    }
}
