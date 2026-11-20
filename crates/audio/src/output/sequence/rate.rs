//! Explicit PCM rate adjustment on the output producer thread. Never called by device callbacks.
use super::*;
use crate::adaptive::{Converter,MAX_PPM};

pub(super) struct Rate {ppm:f64,converter:Option<Converter>,retired:u64}
impl Rate {
    pub fn new(ppm:f64)->Result<Self> {
        if !ppm.is_finite()||ppm.abs()>MAX_PPM {bail!("PCM rate adjustment must be finite and within +/-500 ppm");}
        Ok(Self{ppm,converter:None,retired:0})
    }
    pub fn configure(&mut self,format:SequenceFormat)->Result<()> {
        if format.bit_depth==1 {bail!("raw DSD cannot use PCM rate adjustment");}
        let mut converter=Converter::new(format.sample_rate,format.channels as usize)?;converter.set_ppm(self.ppm)?;
        self.converter=Some(converter);Ok(())
    }
    pub fn set_ppm(&mut self,ppm:f64)->Result<()> {
        if let Some(converter)=&mut self.converter {converter.set_ppm(ppm)?;}else{Self::new(ppm)?;}
        self.ppm=ppm;Ok(())
    }
}
fn converter(state:&State)->Result<&Converter> {state.rate.as_ref().and_then(|rate|rate.converter.as_ref()).context("PCM rate adjustment is not configured")}
fn converter_mut(state:&mut State)->Result<&mut Converter> {state.rate.as_mut().and_then(|rate|rate.converter.as_mut()).context("PCM rate adjustment is not configured")}
pub(super) fn source_clock(state:&State,control:&PlaybackControl,played:u64)->Result<SourceClockPosition> {
    let converter=converter(state)?;let source=converter.position_at_output(played)?;
    Ok(SourceClockPosition{position:checkpoint(state,control,played,false)?.0,source_frame:source.frame,source_fraction:source.fraction,source_per_output:converter.source_per_output_at(played)?})
}

// Cache inverse boundaries before old mapping blocks can be retired. Future boundaries stay pending.
fn resolve(state:&mut State,control:&PlaybackControl)->Result<()> {
    let State{history,rate,programme_end,..}=state;
    let converter=rate.as_ref().and_then(|rate|rate.converter.as_ref()).context("PCM rate adjustment is not configured")?;
    for track in history {
        if track.output_start.is_none(){track.output_start=converter.output_at_source(track.start)?;}
        if track.output_end.is_none(){if let Some(end)=track.end {track.output_end=converter.output_at_source(end)?;}}
    }
    if control.programme_end.load(Ordering::Acquire)==u64::MAX {if let Some(end)=programme_end {if let Some(output)=converter.output_at_source(*end)?{control.programme_end.store(output,Ordering::Release);}}}
    Ok(())
}
pub(super) fn checkpoint(state:&State,control:&PlaybackControl,played:u64,include_ended:bool)->Result<(SequencePosition,Vec<CompletedTrack>)> {
    let written=control.frames_written.load(Ordering::Acquire);
    let played=played.min(written).min(control.programme_end.load(Ordering::Acquire));
    if state.rate.as_ref().is_some_and(|rate|rate.converter.is_none())&&written==0 {
        let first=&state.history[0];return Ok((SequencePosition{playback_id:first.id.clone(),position:first.request.position,frames:0},Vec::new()));
    }
    let source=if let Some(end)=state.programme_end.filter(|_|played==control.programme_end.load(Ordering::Acquire)) {crate::adaptive::Position{frame:end,fraction:0.}}else{converter(state)?.position_at_output(played)?};
    let current=state.history.iter().rev().find(|track|track.output_start.is_some_and(|start|start<=played&&(start<written||written==0))).unwrap_or(&state.history[0]);
    let start=current.output_start.context("current PCM source has no output boundary")?;
    let frames=played.min(current.output_end.unwrap_or(played)).saturating_sub(start);
    let limit=current.end.or(state.programme_end);
    let (whole,fraction)=if limit.is_some_and(|end|source.frame>=end){(limit.unwrap(),0.)}else{(source.frame,source.fraction)};
    // The inverse resampler boundary can land slightly past the source origin. Until
    // one output frame is heard, keep its authorized base position exactly unchanged.
    let position=if frames==0 {current.request.position}else{current.request.position+(whole.saturating_sub(current.start) as f64+fraction)/state.format.sample_rate as f64};
    let ended=if include_ended {state.history.iter().filter_map(|track|{
        let end=track.output_end?;let start=track.output_start?;let source_end=track.end?;
        if end>played||start>=written{return None;}
        Some(CompletedTrack{playback_id:track.id.clone(),position:track.request.position_after(source_end-track.start,state.format.sample_rate as f64),frames:end-start})
    }).collect()}else{Vec::new()};
    Ok((SequencePosition{playback_id:current.id.clone(),position,frames},ended))
}
pub(super) fn heard(state:&mut State,control:&PlaybackControl,final_end:bool)->Result<(Heard,u64)> {
    let written=control.frames_written.load(Ordering::Acquire);let played=control.frames_played.load(Ordering::Acquire).min(written);
    let (_,completed)=checkpoint(state,control,played,true)?;let mut ended=Vec::new();
    while state.history.len()>1&&state.history[1].output_start.is_some_and(|start|start<=played&&(start<written||final_end)) {
        let old=state.history.pop_front().unwrap();
        if let Some(row)=completed.iter().find(|row|row.playback_id==old.id){ended.push(row.clone());}
        else if old.output_start==old.output_end {ended.push(CompletedTrack{playback_id:old.id,position:old.request.position,frames:0});}
        else {bail!("completed PCM source has no heard output boundary");}
    }
    let (position,_)=checkpoint(state,control,played,false)?;let current=state.history.front_mut().context("PCM timeline is empty")?;
    if final_end&&played==written&&current.end.is_none() {
        current.end=Some(state.programme_end.unwrap_or(current.start+((position.position-current.request.position)*state.format.sample_rate as f64).round().max(0.) as u64));
        current.output_end=Some(played.min(control.programme_end.load(Ordering::Acquire)));
        ended.push(CompletedTrack{playback_id:current.id.clone(),position:position.position,frames:position.frames});
    }
    let request=current.request.clone();
    // Preserve two seconds for a delayed paired driver sample; callbacks never touch this ledger.
    let retire=played.saturating_sub(state.format.sample_rate as u64*2);
    let rate=state.rate.as_mut().unwrap();if retire>rate.retired {if let Some(converter)=&mut rate.converter {converter.retire_before(retire)?;rate.retired=retire;}}
    Ok(((position.playback_id,position.position,request,ended),position.frames))
}

pub(super) struct Buffer {
    input:Vec<f64>,output:Vec<f64>,bytes:Vec<u8>,cursor:usize,finishing:bool,ended:bool,meter:Meter,
}
impl Buffer {
    pub fn new(sequence:&PlaybackSequence)->Result<Self> {
        let state=sequence.state.lock().map_err(|_|anyhow::anyhow!("playback sequence lock poisoned"))?;let converter=converter(&state)?;
        Ok(Self{input:Vec::with_capacity(converter.input_frames_max()*converter.channels()),output:vec![0.;converter.output_frames_max()*converter.channels()],bytes:Vec::new(),cursor:0,finishing:false,ended:false,meter:Meter::default()})
    }
    pub fn read(&mut self,stream:&mut PcmStream,max_frames:usize)->Result<Read> {
        let channels=stream.format.channels as usize;let width=channels*if stream.format.float_output&&stream.format.bit_depth==64 {8}else{4};
        loop {
            if stream.control.stopped.load(Ordering::Acquire){return Ok(Read::End);}
            if self.cursor<self.bytes.len() {
                let frames=max_frames.min((self.bytes.len()-self.cursor)/width);let end=self.cursor+frames*width;
                let bytes=self.bytes[self.cursor..end].to_vec();self.cursor=end;
                return Ok(Read::Data(Chunk{bytes,frames,meter:self.meter.clone()}));
            }
            if self.ended {return Ok(Read::End);}
            let needed={let state=stream.sequence.state.lock().map_err(|_|anyhow::anyhow!("playback sequence lock poisoned"))?;converter(&state)?.input_frames_next()};
            while self.input.len()/channels<needed&&!self.finishing {
                match stream.read_raw(needed-self.input.len()/channels)? {
                    Read::Data(chunk)=>{self.input.extend(chunk.bytes.chunks_exact(8).map(|bytes|f64::from_le_bytes(bytes.try_into().unwrap())));self.meter=chunk.meter;}
                    Read::Pending=>return Ok(Read::Pending),
                    Read::End=>{if stream.control.stopped.load(Ordering::Acquire){return Ok(Read::End);}self.finishing=true;}
                }
            }
            let block={
                let mut state=stream.sequence.state.lock().map_err(|_|anyhow::anyhow!("playback sequence lock poisoned"))?;
                let converter=converter_mut(&mut state)?;
                // Control updates may change the required input while a decoder read is blocked.
                // Recheck under the same lock as processing, and keep any excess real PCM.
                let required=converter.input_frames_next()*channels;
                if !self.finishing&&self.input.len()<required {continue;}
                let block=if self.finishing&&self.input.len()<=required {converter.finish(&self.input,&mut self.output)?}else{converter.process(&self.input[..required],&mut self.output)?};
                resolve(&mut state,&stream.control)?;block
            };
            drop(self.input.drain(..block.input_frames*channels));self.ended=block.ended;self.cursor=0;
            let values=&self.output[..block.output_frames*channels];self.meter.peak=values.iter().map(|value|value.abs().min(1.)).fold(0.,f64::max);
            self.bytes=output_bytes(SequenceFormat{bit_depth:stream.format.bit_depth.min(stream.max_bits.unwrap_or(64)),..stream.format},values,|_|1.);
        }
    }
}
