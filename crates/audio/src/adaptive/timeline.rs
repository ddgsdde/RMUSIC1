//! Output/source cursor mapping on the producer/control thread, never the audio callback.
use super::{Position,Span};
use anyhow::{bail,Context,Result};
use std::collections::VecDeque;

// More than five seconds at 768 kHz / 1024 frames, above the current 2 s device-buffer limit.
// Stop accepting input when full; only an explicit consumer acknowledgement retires mappings.
const MAX_BLOCKS:usize=4096;
struct Entry {output:u64,span:Span}
pub(super) struct Timeline {
    entries:VecDeque<Entry>,written:u64,retained:u64,retained_position:Option<Position>,retained_slope:Option<f64>,retired_source:Option<u64>,end:Option<Position>,
}
impl Timeline {
    pub fn new()->Self {
        Self{entries:VecDeque::with_capacity(MAX_BLOCKS),written:0,retained:0,retained_position:None,retained_slope:None,retired_source:None,end:None}
    }
    pub fn written(&self)->u64{self.written}
    pub fn reserve_block(&self)->Result<()> {
        if self.entries.len()==self.entries.capacity(){bail!("adaptive timeline is full; acknowledge consumed output before reading more PCM");}
        Ok(())
    }
    pub fn record(&mut self,span:Span,ended:bool) {
        if span.frames>0 {
            debug_assert!(self.entries.len()<self.entries.capacity());
            self.entries.push_back(Entry{output:self.written,span});
            self.written+=span.frames as u64;
        }
        if ended {self.end=span.position(span.frames);}
    }
    pub fn position(&self,frame:u64)->Result<Position> {
        if frame<self.retained {bail!("adaptive output cursor has been retired");}
        if frame>self.written {bail!("adaptive output cursor exceeds converted output");}
        if frame==0 {return Ok(Position{frame:0,fraction:0.});}
        if frame==self.written {if let Some(end)=self.end{return Ok(end);}}
        if frame==self.retained {if let Some(position)=self.retained_position{return Ok(position);}}
        let entry=self.entry_at(frame)?;
        entry.span.cursor((frame-entry.output) as usize).context("adaptive output cursor lies outside its source mapping")
    }
    fn entry_at(&self,frame:u64)->Result<&Entry> {
        let mut lo=0;let mut hi=self.entries.len();
        while lo<hi {let mid=(lo+hi)/2;if self.entries[mid].output<frame {lo=mid+1;}else{hi=mid;}}
        lo.checked_sub(1).and_then(|index|self.entries.get(index)).context("adaptive output cursor has no retained source mapping")
    }
    pub fn source_per_output_at(&self,frame:u64)->Result<f64> {
        self.position(frame)?;
        if frame==self.retained {if let Some(slope)=self.retained_slope{return Ok(slope);}}
        let entry=if frame==0{self.entries.front().context("no converted source clock")?}else{self.entry_at(frame)?};
        entry.span.source_per_output_at((frame-entry.output) as usize).context("adaptive slope cursor lies outside its mapping")
    }
    pub fn output_at_source(&self,source:u64)->Result<Option<u64>> {
        // Compare integer parts only: fractions are nonnegative, so this also works above 2^53.
        let first=self.position(self.retained)?;
        if self.retired_source.map_or(source<first.frame,|last|source<=last) {bail!("adaptive source boundary has been retired");}
        if self.position(self.written)?.frame<source {return Ok(None);}
        let mut lo=self.retained;let mut hi=self.written;
        while lo<hi {let mid=lo+(hi-lo)/2;if self.position(mid)?.frame<source {lo=mid+1;}else{hi=mid;}}
        Ok(Some(lo))
    }
    pub fn retire_before(&mut self,frame:u64)->Result<()> {
        if frame<self.retained||frame>self.written {bail!("adaptive timeline acknowledgement is outside retained output");}
        if frame>self.retained {self.retired_source=Some(self.position(frame-1)?.frame);self.retained_position=Some(self.position(frame)?);self.retained_slope=Some(self.source_per_output_at(frame)?);}
        while self.entries.len()>1&&self.entries[1].output<=frame {self.entries.pop_front();}
        self.retained=frame;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptive::{Converter,SINC_LEN,OVERSAMPLING};
    #[test]
    fn adaptive_source_clock_keeps_large_origins_and_historical_slopes_after_future_ramps_and_retirement(){
        let base=1u64<<60;let mut timeline=Timeline::new();timeline.written=base;timeline.retained=base;
        let centre=SINC_LEN as f64/2.-1.+1./OVERSAMPLING as f64;
        timeline.record(Span{origin:base,index:-centre-1.,step:1.0004,delta:1e-8,skip:0,limit:None,frames:8},false);
        let cursor=base+8;let position=timeline.position(cursor).unwrap();assert_eq!(position.frame,base+8);assert!((position.fraction-0.00360044).abs()<1e-12);
        let slope=timeline.source_per_output_at(cursor).unwrap();assert!((slope-(1.0004+8e-8)).abs()<1e-15);
        timeline.record(Span{origin:base+8,index:-centre-1.,step:0.9995,delta:0.,skip:0,limit:None,frames:8},false);
        assert_eq!(timeline.position(cursor).unwrap(),position);assert_eq!(timeline.source_per_output_at(cursor).unwrap(),slope);
        timeline.retire_before(cursor).unwrap();assert_eq!(timeline.source_per_output_at(cursor).unwrap(),slope);assert_eq!(timeline.position(cursor).unwrap(),position);assert_eq!(timeline.source_per_output_at(cursor+1).unwrap(),0.9995);
        assert!(timeline.source_per_output_at(cursor-1).is_err());assert!(timeline.source_per_output_at(timeline.written()+1).is_err());
    }

    #[test]
    fn adaptive_timeline_separates_heard_output_from_source_and_splits_track_boundaries() {
        for ppm in [-500.,500.] {
            let source_frames=65536u64;let mut converter=Converter::new(768000,2).unwrap();converter.set_ppm(ppm).unwrap();
            let input=vec![0.25;converter.input_frames_max()*2];let mut output=vec![0.;converter.output_frames_max()*2];let mut consumed=0;
            loop {
                let needed=converter.input_frames_next() as u64;let remaining=source_frames-consumed;
                let block=if remaining>=needed {converter.process(&input[..needed as usize*2],&mut output).unwrap()}else{converter.finish(&input[..remaining as usize*2],&mut output).unwrap()};
                consumed+=block.input_frames as u64;
                if remaining<needed {let mut ended=block.ended;while !ended {ended=converter.finish(&[],&mut output).unwrap().ended;}break;}
            }
            let total=converter.output_frames();assert_ne!(total,source_frames);
            assert_eq!(converter.position_at_output(0).unwrap(),Position{frame:0,fraction:0.});
            assert_eq!(converter.position_at_output(total).unwrap(),Position{frame:source_frames,fraction:0.});
            let heard=total/2;let position=converter.position_at_output(heard).unwrap();
            assert!((position.frame as f64+position.fraction-heard as f64).abs()>10.);
            // These output counts, not the source-position delta, determine actual listen seconds.
            let boundaries=[0,10000,32001,source_frames].map(|source|{
                let cursor=converter.output_at_source(source).unwrap().unwrap();
                assert!(converter.position_at_output(cursor).unwrap().frame>=source);
                if cursor>0 {assert!(converter.position_at_output(cursor-1).unwrap().frame<source);}
                cursor
            });
            let per_track=boundaries.windows(2).map(|pair|pair[1]-pair[0]).collect::<Vec<_>>();
            assert_eq!(per_track.iter().sum::<u64>(),total);assert_ne!(per_track,[10000,22001,33535]);
            assert!(converter.position_at_output(total+1).is_err());assert_eq!(converter.output_at_source(source_frames+1).unwrap(),None);
            assert_eq!(converter.position_at_output(heard).unwrap(),position,"later queries cannot replace a captured cursor");
        }
    }

    #[test]
    fn adaptive_timeline_full_history_does_not_consume_input_and_retirement_is_explicit() {
        let mut converter=Converter::new(48000,1).unwrap();converter.timeline.entries=VecDeque::with_capacity(2);
        let input=vec![0.;converter.input_frames_max()];let mut output=vec![0.;converter.output_frames_max()];
        converter.process(&input[..converter.input_frames_next()],&mut output).unwrap();let first=converter.output_frames();
        converter.process(&input[..converter.input_frames_next()],&mut output).unwrap();
        let state=(converter.origin,converter.index,converter.real_input,converter.output_frames(),converter.input_frames_next());
        assert!(converter.process(&input[..converter.input_frames_next()],&mut output).is_err());
        assert!(converter.finish(&[],&mut output).is_err());assert!(converter.end.is_none());
        assert_eq!((converter.origin,converter.index,converter.real_input,converter.output_frames(),converter.input_frames_next()),state);
        let captured=converter.position_at_output(first).unwrap();converter.retire_before(first).unwrap();
        assert!(converter.position_at_output(first-1).is_err());assert_eq!(converter.position_at_output(first).unwrap(),captured);
        assert!(converter.output_at_source(0).is_err());
        assert!(converter.retire_before(first-1).is_err());assert!(converter.retire_before(converter.output_frames()+1).is_err());
        converter.process(&input[..converter.input_frames_next()],&mut output).unwrap();
        assert_eq!(converter.timeline.entries.len(),2);assert_eq!(converter.position_at_output(first).unwrap(),captured);
    }

    #[test]
    fn adaptive_timeline_preserves_large_integer_cursors_and_empty_verified_eof() {
        let base=1u64<<60;let mut timeline=Timeline::new();timeline.written=base;timeline.retained=base;
        // A retired block boundary retains its exact cursor even after its entry was removed.
        timeline.retained_position=Some(Position{frame:base+200,fraction:0.25});
        let centre=SINC_LEN as f64/2.-1.+1./OVERSAMPLING as f64;
        timeline.record(Span{origin:base+200,index:-centre-0.75,step:1.,delta:0.,skip:0,limit:None,frames:8},false);
        assert_eq!(timeline.position(base+3).unwrap(),Position{frame:base+203,fraction:0.25});
        assert_eq!(timeline.output_at_source(base+204).unwrap(),Some(base+4));
        assert!(timeline.output_at_source(base+199).is_err());timeline.retire_before(base+4).unwrap();
        assert!(timeline.position(base+3).is_err());assert_eq!(timeline.position(base+4).unwrap().frame,base+204);
        let mut converter=Converter::new(768000,2).unwrap();let mut output=vec![0.;converter.output_frames_max()*2];
        let mut ended=converter.finish(&[],&mut output).unwrap().ended;while !ended {ended=converter.finish(&[],&mut output).unwrap().ended;}
        assert_eq!(converter.output_frames(),0);assert_eq!(converter.position_at_output(0).unwrap(),Position{frame:0,fraction:0.});
        assert_eq!(converter.output_at_source(0).unwrap(),Some(0));assert_eq!(converter.output_at_source(1).unwrap(),None);
        converter.retire_before(0).unwrap();assert_eq!(converter.finish(&[],&mut output).unwrap().output_frames,0);
    }

    #[test]
    fn adaptive_timeline_retired_boundary_is_not_reassigned_to_a_later_output_frame() {
        let mut converter=Converter::new(768000,1).unwrap();converter.set_ppm(-500.).unwrap();
        let input=vec![0.;converter.input_frames_max()];let mut output=vec![0.;converter.output_frames_max()];
        for _ in 0..4 {converter.process(&input[..converter.input_frames_next()],&mut output).unwrap();}
        let repeated=(2..converter.output_frames()).find(|&n|converter.position_at_output(n-1).unwrap().frame==converter.position_at_output(n).unwrap().frame).unwrap();
        let source=converter.position_at_output(repeated).unwrap().frame;
        assert!(converter.output_at_source(source).unwrap().unwrap()<repeated);
        converter.retire_before(repeated).unwrap();
        assert!(converter.output_at_source(source).is_err(),"the actual first boundary is retired, not the later retained frame with the same source integer");
        let next=converter.output_at_source(source+1).unwrap().unwrap();assert!(next>repeated);
        let mut fast=Converter::new(768000,1).unwrap();fast.set_ppm(500.).unwrap();
        for _ in 0..4 {fast.process(&input[..fast.input_frames_next()],&mut output).unwrap();}
        let gap=(2..fast.output_frames()).find(|&n|fast.position_at_output(n).unwrap().frame>fast.position_at_output(n-1).unwrap().frame+1).unwrap();
        let crossed=fast.position_at_output(gap-1).unwrap().frame+1;
        assert_eq!(fast.output_at_source(crossed).unwrap(),Some(gap));fast.retire_before(gap).unwrap();
        assert_eq!(fast.output_at_source(crossed).unwrap(),Some(gap),"a boundary crossed by this retained sample is still valid");
    }

    #[test]
    fn adaptive_timeline_captured_block_end_does_not_change_after_a_future_ramp() {
        let mut converter=Converter::new(768000,1).unwrap();converter.set_ppm(-500.).unwrap();let input=vec![0.;converter.input_frames_max()];let mut output=vec![0.;converter.output_frames_max()];
        converter.process(&input[..converter.input_frames_next()],&mut output).unwrap();let end=converter.output_frames();let captured=converter.position_at_output(end).unwrap();
        converter.set_ppm(500.).unwrap();converter.process(&input[..converter.input_frames_next()],&mut output).unwrap();assert_eq!(converter.position_at_output(end).unwrap(),captured);
        converter.retire_before(end).unwrap();assert_eq!(converter.position_at_output(end).unwrap(),captured);
    }
}
