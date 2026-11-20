use super::*;
use rmusic_bridge::listens::{Collector,Source};

pub(super) struct Sampler(Collector,u32);
impl Sampler {
    pub fn new(store:Arc<ListenStore>,owner:ListenOwner,version:u32)->Self {Self(Collector::new(store,owner),version)}
    fn sample(&mut self,id:&str,position:f64,frames:u64,rate:u32,tracks:&Tracks)->Result<()> {
        let source=||{
            let tracks=tracks.lock().map_err(|_|anyhow::anyhow!("sequence tracks lock poisoned"))?;
            let Some(entry)=tracks.get(id) else{return Ok(None)};
            let (Some(group),Some(request))=(&entry.listen_session_id,&entry.request) else{return Ok(None)};
            Ok(Some(Source {listen_session_id:group.clone(),track_id:entry.track.clone(),title:entry.track.clone(),artist:String::new(),duration:request.duration.unwrap_or(0.)}))
        };
        if self.1==2 {
            let start=tracks.lock().map_err(|_|anyhow::anyhow!("sequence tracks lock poisoned"))?.get(id).and_then(|entry|entry.request.as_ref()).map(|request|request.position);
            if let Some(start)=start{self.0.sample_output(id,start,position,frames,rate,source)}else{Ok(())}
        }else{self.0.sample(id,position,frames,rate,source)}
    }
    pub fn update(&mut self,update:&PlaybackUpdate,rate:u32,tracks:&Tracks)->Result<()> {
        for ended in &update.ended {
            self.sample(&ended.playback_id,ended.position,ended.frames,rate,tracks)?;
            self.0.finish(&ended.playback_id,"ended")?;
        }
        let start=tracks.lock().map_err(|_|anyhow::anyhow!("sequence tracks lock poisoned"))?.get(&update.playback_id).and_then(|entry|entry.request.as_ref()).map(|r|r.position);
        if start.is_some() {self.sample(&update.playback_id,update.position,update.output_frames,rate,tracks)?;}
        Ok(())
    }
    pub fn stop(&mut self,position:Option<&SequencePosition>,rate:u32,tracks:&Tracks,reason:&str)->Result<()> {
        if let Some(position)=position {self.sample(&position.playback_id,position.position,position.frames,rate,tracks)?;}
        self.0.finish_all(reason)
    }
    pub fn checkpoint_rows(&mut self,id:&str)->Vec<rmusic_bridge::listens::Row>{self.0.checkpoint_rows(id)}
}


#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_output_v2_sampler_keeps_seek_baseline_output_frames_and_terminal_dedup() {
        let dir=tempfile::tempdir().unwrap();let store=ListenStore::open(dir.path()).unwrap();let owner=ListenOwner{core_id:"qa-core".into(),user_id:"qa-owner".into()};let mut sampler=Sampler::new(store.clone(),owner.clone(),2);
        let request=PlaybackRequest{flac_samples: None, dsd_pcm: None, input:"unused".into(),position:10.,sample_rate:768000,channels:2,source_integer_lossless:true,source_bit_depth:24,dsp:DspConfig::default(),impulse:None,measured_lufs:None,volume:1.,duration:Some(120.),transitions:FadeConfig::default()};
        let tracks=Arc::new(Mutex::new(HashMap::from([("a".into(),Entry{track:"track".into(),request:Some(request),audio:None,listen_session_id:Some("listen".into())})])));
        let mut update=PlaybackUpdate{playback_id:"a".into(),position:10.250125,output_frames:192000,ended:vec![],signal_path:output::SignalPath{output:"memory-qa".into(),sample_rate:768000,channels:2,exclusive:true,bit_depth:24,sample_type:"integer".into(),mixer_sample_rate:None,bit_perfect_eligible:false,processing:vec![]},meter:Default::default(),continuity_lost:false};
        sampler.update(&update,768000,&tracks).unwrap();
        update.playback_id="unheard".into();update.position=0.;update.output_frames=0;sampler.update(&update,768000,&tracks).unwrap();
        let end=SequencePosition{playback_id:"a".into(),position:10.50025,frames:384000};sampler.stop(Some(&end),768000,&tracks,"ended").unwrap();sampler.stop(Some(&end),768000,&tracks,"ended").unwrap();store.drain().unwrap();
        let rows=store.pending(&owner).unwrap();assert_eq!(rows.len(),1);let snapshot=&rows[0].snapshot;assert_eq!(snapshot.days.values().map(|day|day.seconds).sum::<f64>(),0.5);assert_eq!(snapshot.position,10.50025);assert_eq!(snapshot.reason,"ended");let measured=snapshot.measured.as_ref().unwrap();assert_eq!(measured.version,2);assert_eq!(measured.segments[0].start,10.);assert_eq!(measured.segments.last().unwrap().output.as_ref().unwrap().end,384000);
    }
}
