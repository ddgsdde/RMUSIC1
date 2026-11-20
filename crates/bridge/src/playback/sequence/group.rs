//! Keep only unacknowledged programme boundaries; final capture runs after output join.
use super::*;
use serde::Serialize;

#[derive(Clone,Serialize)]
struct Source {playback_id:String,track_id:String,position:f64,frames:u64,ended:bool}
#[derive(Default)]
pub(super) struct Ledger {sources:VecDeque<Source>,acknowledged:u64,error:Option<String>}
pub(super) type Shared=Arc<Mutex<Ledger>>;
impl Ledger {
    fn record(&mut self,position:&SequencePosition,tracks:&Tracks,ended:bool)->Result<()> {
        let tracks=tracks.lock().map_err(|_|anyhow::anyhow!("group tracks lock poisoned"))?;
        let track=tracks.get(&position.playback_id).context("group source identity unavailable")?;
        if let Some(previous)=self.sources.iter_mut().find(|source|source.playback_id==position.playback_id){
            if previous.track_id!=track.track||position.frames<previous.frames{anyhow::bail!("group source frames moved backwards");}
            previous.position=position.position;previous.frames=position.frames;previous.ended|=ended;
        }else{
            if self.sources.len()>=9{anyhow::bail!("group final ledger exceeded its reservation window");}
            self.sources.push_back(Source{playback_id:position.playback_id.clone(),track_id:track.track.clone(),position:position.position,frames:position.frames,ended});
        }Ok(())
    }
    fn capture(&mut self,position:&SequencePosition,ended:&[rmusic_audio::output::CompletedTrack],tracks:&Tracks)->Result<()> {
        for source in ended{self.record(&SequencePosition{playback_id:source.playback_id.clone(),position:source.position,frames:source.frames},tracks,true)?;}
        self.record(position,tracks,false)
    }
    pub(super) fn update(&mut self,update:&PlaybackUpdate,tracks:&Tracks,_rate:u32)->Result<()> {
        let position=SequencePosition{playback_id:update.playback_id.clone(),position:update.position,frames:update.output_frames};
        if let Err(error)=self.capture(&position,&update.ended,tracks){self.error=Some(error.to_string());return Err(error);}Ok(())
    }
    pub(super) fn acknowledge(&mut self,ended:&[rmusic_audio::output::CompletedTrack])->Result<()> {
        for source in ended{
            if let Some(index)=self.sources.iter().position(|row|row.playback_id==source.playback_id){
                let row=&self.sources[index];if !row.ended||row.frames!=source.frames{anyhow::bail!("group acknowledged boundary differs from actual output");}
                self.acknowledged+=self.sources.remove(index).unwrap().frames;
            }
        }Ok(())
    }
    pub(super) fn finish(&mut self,sequence:&PlaybackSequence,control:&PlaybackControl,tracks:&Tracks)->Result<Value> {
        if let Some(error)=&self.error{anyhow::bail!("group frame capture failed: {error}");}
        let (current,ended)=sequence.checkpoint(control)?;self.capture(&current,&ended,tracks)?;
        Ok(json!({"sequence_final":self.sources,"programme_frames_played":self.acknowledged+self.sources.iter().map(|source|source.frames).sum::<u64>()}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn adaptive_stream_group_ledger_keeps_output_count_separate_from_song_position() {
        let tracks=Arc::new(Mutex::new(HashMap::from([("a".into(),Entry{track:"track".into(),request:None,audio:None,listen_session_id:None})])));
        let update=PlaybackUpdate{playback_id:"a".into(),position:20.2,output_frames:76800,ended:Vec::new(),signal_path:output::SignalPath{output:"memory".into(),sample_rate:768000,channels:2,exclusive:true,bit_depth:64,sample_type:"float64".into(),mixer_sample_rate:None,bit_perfect_eligible:false,processing:Vec::new()},meter:rmusic_audio::Meter::default(),continuity_lost:false};
        let mut ledger=Ledger::default();ledger.update(&update,&tracks,768000).unwrap();assert_eq!(ledger.sources[0].frames,76800);assert_eq!(ledger.sources[0].position,20.2);
        ledger.update(&update,&tracks,768000).unwrap();assert_eq!(ledger.sources.len(),1);assert_eq!(ledger.sources[0].frames,76800);
    }
    #[test]
    fn group_ledger_prunes_only_acknowledged_boundaries_and_keeps_total_and_final_source() {
        let tracks=Arc::new(Mutex::new(HashMap::from(["a","b","c"].map(|id|(id.into(),Entry{track:"duplicate-track".into(),request:None,audio:None,listen_session_id:None})))));
        let ended=vec![("a",101),("b",100)].into_iter().map(|(id,frames)|output::CompletedTrack{playback_id:id.into(),position:frames as f64/48000.,frames}).collect::<Vec<_>>();
        let current=SequencePosition{playback_id:"c".into(),position:37./48000.,frames:37};let mut ledger=Ledger::default();
        ledger.capture(&current,&ended,&tracks).unwrap();assert_eq!(ledger.sources.len(),3);
        let invalid=output::CompletedTrack{frames:99,..ended[0].clone()};assert!(ledger.acknowledge(&[invalid]).is_err());assert_eq!(ledger.acknowledged,0);
        ledger.acknowledge(&ended).unwrap();ledger.acknowledge(&ended).unwrap();assert_eq!(ledger.acknowledged,201);assert_eq!(ledger.sources.len(),1);
        {let mut tracks=tracks.lock().unwrap();tracks.remove("a");tracks.remove("b");}
        let later=SequencePosition{frames:50,position:50./48000.,..current};ledger.capture(&later,&[],&tracks).unwrap();
        assert_eq!(ledger.acknowledged+ledger.sources.iter().map(|row|row.frames).sum::<u64>(),251);assert_eq!(ledger.sources[0].playback_id,"c");
    }
}
