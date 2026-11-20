//! Version 2 retains programme positions and exact cumulative heard PCM frame intervals separately.
use super::*;
#[cfg(test)]
#[path="output_tests.rs"]
mod tests;
impl Recorder {
    pub fn with_measured_output(mut self,playback:String,session:String,clock:Instant,source_start:f64)->Result<Self> {
        if !source_start.is_finite()||source_start<0. {anyhow::bail!("invalid measured source baseline");}
        self.row.snapshot.measured=Some(Measured{playback_id:playback,listen_session_id:session,segments:Vec::new(),version:2});
        self.row.snapshot.position=source_start;self.clock=clock;self.device_baseline=Some((0,source_start,0.));Ok(self)
    }
    pub(super) fn sample_output_at(&mut self,position:f64,frames:u64,rate:u32,at:i64,mono:f64)->Result<()> {
        let s=&mut self.row.snapshot;
        if s.reason!="active"||!position.is_finite()||position<0.||!mono.is_finite()||!(8000..=768000).contains(&rate){return Ok(());}
        let Some((prior_frames,prior_position,prior_mono))=self.device_baseline else{return Ok(())};
        if frames<prior_frames||position<prior_position||mono<prior_mono{return Ok(());}
        if s.measured.as_ref().and_then(|m|m.segments.last()).and_then(|segment|segment.output.as_ref()).is_some_and(|output|output.sample_rate!=rate){return Ok(());}
        let count=frames-prior_frames;let seconds=count as f64/rate as f64;
        if count==0&&position!=prior_position{return Ok(());}
        if seconds>mono-prior_mono+1.||((position-prior_position)-seconds).abs()>seconds*0.0005+2./rate as f64{return Ok(());}
        if count>0 {
            let start=at-(seconds*1000.).round() as i64;
            if s.measured.as_ref().unwrap().segments.is_empty(){s.started_at=start;s.updated_at=start;}
            let mut cursor=start;let mut output_start=prior_frames;let mut source_start=prior_position;
            loop {
                let date=chrono::DateTime::from_timestamp_millis(cursor).context("invalid measured output date")?.date_naive();
                let midnight=date.succ_opt().context("measured output date overflow")?.and_hms_opt(0,0,0).unwrap().and_utc().timestamp_millis();
                let stop=at.min(midnight);let last_at=if stop==midnight {stop-1}else{stop};
                let output_end=if stop==at {frames}else{prior_frames+(count as u128*(stop-start) as u128/(at-start) as u128) as u64};
                let source_end=if stop==at {position}else{prior_position+(position-prior_position)*(output_end-prior_frames) as f64/count as f64};
                if output_end>output_start {
                    let measured=s.measured.as_mut().unwrap();
                    let prior=measured.segments.last_mut().filter(|segment| (segment.end-source_start).abs()<1e-6&&segment.output.as_ref().is_some_and(|output|output.end==output_start&&output.sample_rate==rate)&&chrono::DateTime::from_timestamp_millis(segment.last_at).is_some_and(|time|time.date_naive()==date));
                    if let Some(prior)=prior {prior.end=source_end;prior.last_at=prior.last_at.max(last_at);prior.output.as_mut().unwrap().end=output_end;}
                    else {measured.segments.push(Segment{start:source_start,end:source_end,last_at,output:Some(OutputRange{start:output_start,end:output_end,sample_rate:rate})});}
                    let day=s.days.entry(date.to_string()).or_insert(Day{seconds:0.,last_at});day.seconds+=(output_end-output_start) as f64/rate as f64;day.last_at=day.last_at.max(last_at);
                }
                if stop==at {break;}
                output_start=output_end;source_start=source_end;cursor=stop;
            }
        }
        s.position=position;s.updated_at=s.updated_at.max(at);self.device_baseline=Some((frames,position,mono));
        if count>0&&mono-self.saved>=5. {self.flush(mono)?;}
        Ok(())
    }
}
