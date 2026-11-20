use super::*;

impl Recorder {
    pub fn with_measured(mut self, playback_id: String, listen_session_id: String, clock: Instant) -> Self {
        self.row.snapshot.measured=Some(Measured {playback_id,listen_session_id,segments:Vec::new(),version:1});
        self.clock=clock;
        self
    }
    pub fn sample_device(&mut self, position: f64, frames: u64, rate: u32) -> Result<()> {
        self.sample_device_at(position,frames,rate,chrono::Utc::now().timestamp_millis(),self.clock.elapsed().as_secs_f64())
    }
    pub(super) fn sample_device_at(&mut self, position: f64, frames: u64, rate: u32, at: i64, mono: f64) -> Result<()> {
        if self.row.snapshot.measured.as_ref().is_some_and(|m|m.version==2){return self.sample_output_at(position,frames,rate,at,mono);}
        let s=&mut self.row.snapshot;
        if s.reason!="active" || s.measured.is_none() || !position.is_finite() || position<0. || !(8000..=rmusic_audio::MAX_CLOCK_RATE).contains(&rate) {return Ok(());}
        let (prior_frames,prior_position,prior_mono)=self.device_baseline.unwrap_or((0,position-frames as f64/rate as f64,0.));
        if frames<prior_frames || position+1e-6<prior_position || mono<prior_mono {return Ok(());}
        let delta=(frames-prior_frames) as f64/rate as f64;
        if prior_position < -1e-6 || ((position-prior_position)-delta).abs()>1e-5 || delta>mono-prior_mono+1. {return Ok(());}
        if delta>0. {
            let first=self.device_baseline.is_none();
            let start_at=at-(delta*1000.).round() as i64;
            if first {s.started_at=start_at;s.updated_at=start_at;}
            let mut cursor=start_at;
            let mut source=prior_position.max(0.);
            loop {
                let date=chrono::DateTime::from_timestamp_millis(cursor).context("无效统计时间")?.date_naive();
                let midnight=date.succ_opt().context("统计日期超出范围")?.and_hms_opt(0,0,0).unwrap().and_utc().timestamp_millis();
                let stop=at.min(midnight);
                let end=if stop==at {position}else{prior_position+delta*(stop-start_at) as f64/(at-start_at) as f64};
                let last_at=if stop==midnight {stop-1}else{stop};
                let measured=s.measured.as_mut().unwrap();
                let prior=measured.segments.last_mut().filter(|segment| {
                    (segment.end-source).abs()<1e-6 && chrono::DateTime::from_timestamp_millis(segment.last_at).is_some_and(|time|time.date_naive()==date)
                });
                if let Some(prior)=prior {prior.end=end;prior.last_at=prior.last_at.max(last_at);}
                else {measured.segments.push(Segment {start:source,end,last_at,output:None});}
                let day=s.days.entry(date.to_string()).or_insert(Day {seconds:0.,last_at});
                day.seconds+=end-source;day.last_at=day.last_at.max(last_at);
                if stop==at {break;}
                source=end;cursor=stop;
            }
        }
        s.position=position;s.updated_at=s.updated_at.max(at);
        self.device_baseline=Some((frames,position,mono));
        if delta>0. && mono-self.saved>=5. {self.flush(mono)?;}
        Ok(())
    }
}
