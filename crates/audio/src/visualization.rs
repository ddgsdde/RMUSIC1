//! Read-only visual features. Runs on the decoder worker, never in the hardware callback.
//! The output bytes, format and clock are not touched. A short renewable lease bounds all work.
use realfft::{RealFftPlanner, RealToComplex};
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, sync::{Arc, Mutex, OnceLock, atomic::{AtomicU64, Ordering}}, time::Instant};
fn now() -> u64 { static START: OnceLock<Instant> = OnceLock::new(); START.get_or_init(Instant::now).elapsed().as_millis() as u64 }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Frame {
    pub playback_id: String,
    pub position: f64,
    /// Eight logarithmic bands (20 Hz–12 kHz), quantized to 0..255. Analysis only.
    pub bands: [u8; 8],
    pub energy: u8,
    pub peak: u8,
}
#[derive(Default)]
pub struct Tap { until: AtomicU64, frames: Mutex<VecDeque<Frame>> }
impl Tap {
    pub fn renew(&self, milliseconds: u64) {
        if milliseconds==0 {self.until.store(0,Ordering::Relaxed);}
        else {self.until.fetch_max(now().saturating_add(milliseconds.min(5000)),Ordering::Relaxed);}
    }
    pub fn active(&self) -> bool { now() < self.until.load(Ordering::Relaxed) }
    pub fn snapshot(&self, id: &str, position: f64) -> Vec<Frame> {
        self.window(id,position,0.03)
    }
    /// Local presentation may schedule already decoded features against the actual output cursor.
    pub fn preview(&self,id:&str,position:f64)->Vec<Frame>{self.window(id,position,0.25)}
    fn window(&self,id:&str,position:f64,ahead:f64)->Vec<Frame>{
        if !self.active() { return Vec::new(); }
        self.frames.try_lock().map(|frames|frames.iter().filter(|f| f.playback_id == id && f.position <= position + ahead && f.position >= position - 0.3).take(24).cloned().collect()).unwrap_or_default()
    }
    fn publish(&self, frame: Frame) { if let Ok(mut frames) = self.frames.try_lock() { if frames.len() == 80 { frames.pop_front(); } frames.push_back(frame); } }
}
pub(crate) struct Analyzer {
    fft: Arc<dyn RealToComplex<f64>>,
    input: Vec<f64>, output: Vec<realfft::num_complex::Complex<f64>>, scratch: Vec<realfft::num_complex::Complex<f64>>,
    samples: Vec<f64>, sum: f64, count: u32, cursor: f64, id: String,
}
impl Default for Analyzer {
    fn default() -> Self {
        let fft = RealFftPlanner::<f64>::new().plan_fft_forward(1024);
        Self { input: fft.make_input_vec(), output: fft.make_output_vec(), scratch: fft.make_scratch_vec(), fft, samples: Vec::with_capacity(1024), sum: 0., count: 0, cursor: 0., id: String::new() }
    }
}
impl Analyzer {
    pub fn feed(&mut self, tap: &Tap, id: &str, position: f64, rate: u32, channels: usize, values: &[f64]) {
        if !tap.active() { self.samples.clear(); self.sum=0.;self.count=0;self.cursor=0.;self.id.clear();return; }
        if self.id != id { self.id=id.into();self.samples.clear();self.sum=0.;self.count=0;self.cursor=0.; }
        if channels==0 || rate==0 { return; }
        // Box-integrated analysis resampling, capped at 24 kHz. No change to playback PCM.
        let analysis_rate=24000_u32.min(rate); let step=rate as f64/analysis_rate as f64;
        for (frame, input) in values.chunks_exact(channels).enumerate() {
            self.sum += if channels>1 { (input[0]+input[1])*0.5 } else { input[0] };self.count+=1;self.cursor+=1.;
            if self.cursor < step { continue; } self.cursor-=step;
            self.samples.push(self.sum/self.count as f64);self.sum=0.;self.count=0;
            if self.samples.len()!=1024 {continue;}
            let energy=(self.samples.iter().map(|v|v*v).sum::<f64>()/1024.).sqrt();
            let peak=self.samples.iter().map(|v|v.abs()).fold(0.,f64::max);
            for (i,(target,sample)) in self.input.iter_mut().zip(&self.samples).enumerate() { *target=sample*(0.5-0.5*(2.*std::f64::consts::PI*i as f64/1023.).cos()); }
            if self.fft.process_with_scratch(&mut self.input,&mut self.output,&mut self.scratch).is_ok() {
                let edges=[20.,80.,200.,500.,1500.,3000.,6000.,9000.,12001.];let mut bands=[0;8];
                for b in 0..8 { let start=((edges[b]/analysis_rate as f64*1024.).ceil() as usize).max(1);let end=((edges[b+1]/analysis_rate as f64*1024.).ceil() as usize).min(513);
                    let magnitude=self.output.get(start..end).unwrap_or(&[]).iter().map(|v|v.norm()*4./1024.).fold(0.,f64::max);
                    bands[b]=quantize(magnitude);
                }
                tap.publish(Frame { playback_id:id.into(), position:position+frame as f64/rate as f64, bands, energy:quantize(energy), peak:quantize(peak) });
            }
            self.samples.clear();
        }
    }
}
fn quantize(value:f64)->u8 { if !value.is_finite() || value<=0. {0} else { (((20.*value.log10()+72.)/72.).clamp(0.,1.)*255.).round() as u8 } }
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn lease_gate_and_actual_tone_bands() {
        let tap=Tap::default();let mut analyzer=Analyzer::default();
        let samples=(0..4800).map(|i|(i as f64*2.*std::f64::consts::PI*1000./48000.).sin()*0.5).collect::<Vec<_>>();let original=samples.clone();
        analyzer.feed(&tap,"one",0.,48000,1,&samples);assert!(tap.frames.lock().unwrap().is_empty());
        tap.renew(3000);analyzer.feed(&tap,"one",0.,48000,1,&samples);
        let frames=tap.snapshot("one",0.1);assert!(!frames.is_empty());let peak=frames.last().unwrap().bands.iter().enumerate().max_by_key(|(_,v)|*v).unwrap().0;assert_eq!(peak,3);assert_eq!(samples,original);
        assert!(tap.snapshot("other",0.1).is_empty());assert!(tap.snapshot("one",10.).is_empty());tap.renew(0);assert!(tap.snapshot("one",0.1).is_empty());
    }
    #[test] fn bounded_and_new_track_discards_partial_window() { let tap=Tap::default();tap.renew(3000);let mut analyzer=Analyzer::default();for _ in 0..100 {analyzer.feed(&tap,"a",0.,24000,1,&vec![0.;1024]);}assert_eq!(tap.frames.lock().unwrap().len(),80);analyzer.feed(&tap,"b",0.,24000,1,&vec![0.;1024]);assert_eq!(tap.snapshot("b",0.1).len(),1); }
    #[test] fn local_viewer_cannot_shorten_remote_lease_and_preview_keeps_output_timestamps(){
        let tap=Tap::default();tap.renew(3000);let until=tap.until.load(Ordering::Relaxed);tap.renew(600);assert_eq!(tap.until.load(Ordering::Relaxed),until);
        let mut analyzer=Analyzer::default();analyzer.feed(&tap,"p",1.,24000,1,&vec![0.;4096]);
        assert!(tap.snapshot("p",1.).is_empty());let frames=tap.preview("p",1.);assert!(!frames.is_empty());assert!(frames.iter().all(|f|f.position>1.&&f.position<1.25));assert!(frames.len()<=24);
    }
}
