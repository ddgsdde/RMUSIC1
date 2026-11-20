//! Optional f64 PCM rate conversion. Not wired into players, drivers or Core controls yet.
//! Positive ppm consumes programme faster at the same nominal device rate. DSD is not PCM input.
use anyhow::{bail,Result};
use audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Adjustable,Async,FixedAsync,Indexing,Resampler,SincInterpolationParameters,SincInterpolationType,WindowFunction};
mod timeline;

pub const BLOCK_FRAMES:usize=1024;
pub const MAX_PPM:f64=500.;
const SINC_LEN:usize=512;
const OVERSAMPLING:usize=128;

/// Integer origin stays separate from the fraction, including beyond the f64 exact-integer range.
#[derive(Clone,Copy,Debug,PartialEq)]
pub struct Position {pub frame:u64,pub fraction:f64}
#[derive(Clone,Copy,Debug)]
pub struct Span {
    origin:u64,index:f64,step:f64,delta:f64,skip:usize,limit:Option<u64>,
    pub frames:usize,
}
impl Span {
    fn local(&self,frame:usize)->f64 {
        let k=(self.skip+frame+1) as f64;
        // Pinned Rubato 5.0.1 updates inverse ratio before each sinc interpolation step.
        // Its phase-0 sinc centre is L/2 - 1 + 1/oversampling in the input window.
        self.index+k*self.step+0.5*k*(k+1.)*self.delta+SINC_LEN as f64/2.-1.+1./OVERSAMPLING as f64
    }
    /// Position of an output sample, or the next-sample cursor when frame == frames.
    pub fn position(&self,frame:usize)->Option<Position> {
        if frame>self.frames{return None;}
        self.at(self.local(frame))
    }
    // A heard cursor follows the last sample's rate; it cannot depend on a future block's ramp.
    fn cursor(&self,frame:usize)->Option<Position> {
        if frame==0{return self.position(0);}
        if frame>self.frames{return None;}
        self.at(self.local(frame-1)+self.step+(self.skip+frame) as f64*self.delta)
    }
    fn source_per_output_at(&self,frame:usize)->Option<f64> {
        if frame>self.frames{return None;}
        Some(self.step+(self.skip+frame.max(1)) as f64*self.delta)
    }
    fn at(&self,local:f64)->Option<Position> {
        let floor=local.floor();let whole=self.origin as i128+floor as i128;
        if whole<0{return None;}
        if let Some(limit)=self.limit {if whole>=limit as i128{return Some(Position{frame:limit,fraction:0.});}}
        Some(Position{frame:whole as u64,fraction:local-floor})
    }
    fn before_start(&self,frame:usize)->bool {self.origin as i128+(self.local(frame).floor() as i128)<0}
    fn past_end(&self,frame:usize,end:u64)->bool {self.origin as i128+self.local(frame).floor() as i128>=end as i128}
}
#[derive(Clone,Copy,Debug)]
pub struct Block {pub input_frames:usize,pub output_frames:usize,pub span:Span,pub ended:bool}

pub struct Converter {
    resampler:Async<f64>,channels:usize,rate:u32,origin:u64,index:f64,real_input:u64,
    target_ratio:f64,base_ratio:f64,end:Option<u64>,ended:bool,
    timeline:timeline::Timeline,
}
impl Converter {
    pub fn new(sample_rate:u32,channels:usize)->Result<Self> {Self::new_rates(sample_rate,sample_rate,channels)}
    pub fn new_rates(sample_rate:u32,output_rate:u32,channels:usize)->Result<Self> {
        if !(8000..=768000).contains(&sample_rate)||!(1..=32).contains(&channels){bail!("adaptive PCM format is outside supported limits");}
        if !(8000..=768000).contains(&output_rate){bail!("PCM output rate is outside supported limits");}
        let ratio=output_rate as f64/sample_rate as f64;
        let parameters=SincInterpolationParameters{sinc_len:SINC_LEN,f_cutoff:None,oversampling_factor:OVERSAMPLING,interpolation:SincInterpolationType::Cubic,window:WindowFunction::BlackmanHarris2};
        let resampler=Async::new_sinc(ratio,1.001,&parameters,BLOCK_FRAMES,channels,FixedAsync::Output)?;
        Ok(Self{resampler,channels,rate:sample_rate,origin:0,index:-(SINC_LEN as f64-1.),real_input:0,target_ratio:ratio,base_ratio:ratio,end:None,ended:false,timeline:timeline::Timeline::new()})
    }
    pub fn sample_rate(&self)->u32{self.rate}
    pub fn channels(&self)->usize{self.channels}
    pub fn input_frames_next(&self)->usize{self.resampler.input_frames_next()}
    pub fn input_frames_max(&self)->usize{self.resampler.input_frames_max()}
    pub fn output_frames_max(&self)->usize{self.resampler.output_frames_max()}
    pub fn ppm(&self)->f64{(self.base_ratio/self.target_ratio-1.)*1e6}
    /// Converted output only, excluding startup samples and padded EOF drain.
    pub fn output_frames(&self)->u64{self.timeline.written()}
    /// Resolve a captured output cursor without rereading a newer device clock.
    /// Zero means no samples heard; EOF resolves to the exact real source end.
    pub fn position_at_output(&self,frame:u64)->Result<Position>{self.timeline.position(frame)}
    /// Instantaneous slope at this retained cursor, independent of later conversion ramps.
    pub fn source_per_output_at(&self,frame:u64)->Result<f64>{self.timeline.source_per_output_at(frame)}
    /// First output cursor at or beyond an integer source boundary; None if not reached.
    pub fn output_at_source(&self,frame:u64)->Result<Option<u64>>{self.timeline.output_at_source(frame)}
    /// Control-thread acknowledgement only, after old clock samples are no longer needed.
    /// Retired queries fail rather than being mapped using newer PCM.
    pub fn retire_before(&mut self,frame:u64)->Result<()>{self.timeline.retire_before(frame)}
    /// Queue one smooth block transition. Invalid updates leave the converter unchanged.
    pub fn set_ppm(&mut self,ppm:f64)->Result<()> {
        if self.end.is_some()||self.ended{bail!("adaptive converter is finishing");}
        if !ppm.is_finite()||ppm.abs()>MAX_PPM{bail!("adaptive compensation must be finite and within +/-500 ppm");}
        let ratio=self.base_ratio/(1.+ppm/1e6);self.resampler.set_resample_ratio(ratio,true)?;self.target_ratio=ratio;Ok(())
    }
    /// A short read is not EOF. Supply exactly the requested complete PCM block.
    pub fn process(&mut self,input:&[f64],output:&mut[f64])->Result<Block> {
        if self.end.is_some()||self.ended{bail!("adaptive converter has reached EOF");}
        if input.len()!=self.input_frames_next()*self.channels{bail!("adaptive input block is incomplete");}
        self.run(input,output,None)
    }
    /// Only after verified source EOF. Call again with empty input until ended is true.
    /// Startup and zero-padded drain samples outside the real source are never returned.
    pub fn finish(&mut self,input:&[f64],output:&mut[f64])->Result<Block> {
        if input.len()%self.channels!=0||input.len()/self.channels>self.input_frames_next(){bail!("adaptive final input is invalid");}
        if self.end.is_some()&&!input.is_empty(){bail!("new PCM cannot follow adaptive EOF");}
        let end=self.end.unwrap_or(self.real_input+input.len() as u64/self.channels as u64);
        if self.ended{return Ok(Block{input_frames:0,output_frames:0,span:Span{origin:self.origin,index:self.index,step:1./self.target_ratio,delta:0.,skip:0,limit:Some(end),frames:0},ended:true});}
        self.run(input,output,Some(end))
    }
    fn run(&mut self,input:&[f64],output:&mut[f64],end:Option<u64>)->Result<Block> {
        if output.len()<self.output_frames_max()*self.channels||input.iter().any(|value|!value.is_finite()){bail!("adaptive buffer is too small or contains nonfinite PCM");}
        self.timeline.reserve_block()?;
        let available=input.len()/self.channels;let needed=self.input_frames_next();let produced=self.resampler.output_frames_next();
        let step=1./self.resampler.resample_ratio();let delta=(1./self.target_ratio-step)/produced as f64;
        let mut span=Span{origin:self.origin,index:self.index,step,delta,skip:0,limit:end,frames:produced};
        let capacity=output.len()/self.channels;
        let input=InterleavedSlice::new(input,self.channels,available)?;let mut destination=InterleavedSlice::new_mut(output,self.channels,capacity)?;
        let indexing=Indexing{partial_len:end.map(|_|available),..Indexing::new()};
        let(used,written)=self.resampler.process_into_buffer(&input,&mut destination,Some(&indexing))?;
        debug_assert_eq!((used,written),(needed,produced));
        // Mirror only the small per-block index recurrence, never accumulate a huge f64 source cursor.
        let mut index=self.index;let mut increment=step;for _ in 0..written {increment+=delta;index+=increment;}
        self.index=index-used as f64;self.origin+=used as u64;self.real_input+=available as u64;self.end=end;
        let mut first=0;while first<written&&span.before_start(first){first+=1;}
        let mut last=first;while last<written&&end.is_none_or(|end|!span.past_end(last,end)){last+=1;}
        let count=last-first;if first>0&&count>0{output.copy_within(first*self.channels..last*self.channels,0);}
        self.ended=end.is_some()&&(last<written||span.past_end(written,end.unwrap()));
        span.skip=first;span.frames=count;
        self.timeline.record(span,self.ended);
        Ok(Block{input_frames:available,output_frames:count,span,ended:self.ended})
    }
}

#[cfg(test)]
mod tests;
