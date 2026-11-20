use super::*;
use std::collections::{HashMap,VecDeque};

pub struct Source {
    pub listen_session_id:String,pub track_id:String,pub title:String,pub artist:String,pub duration:f64,
}
/// One output worker owns this collection. Only actual heard frames create a record.
pub struct Collector {
    store:Arc<Store>,owner:Owner,clock:Instant,active:HashMap<String,Recorder>,finished:VecDeque<String>,checkpoints:VecDeque<Row>,
}
impl Collector {
    pub fn new(store:Arc<Store>,owner:Owner)->Self {
        Self {store,owner,clock:Instant::now(),active:HashMap::new(),finished:VecDeque::new(),checkpoints:VecDeque::new()}
    }
    pub fn sample(&mut self,id:&str,position:f64,frames:u64,rate:u32,source:impl FnOnce()->Result<Option<Source>>)->Result<()> {
        self.sample_mode(id,position,frames,rate,None,source)
    }
    pub fn sample_output(&mut self,id:&str,source_start:f64,position:f64,frames:u64,rate:u32,source:impl FnOnce()->Result<Option<Source>>)->Result<()> {
        self.sample_mode(id,position,frames,rate,Some(source_start),source)
    }
    fn sample_mode(&mut self,id:&str,position:f64,frames:u64,rate:u32,start:Option<f64>,source:impl FnOnce()->Result<Option<Source>>)->Result<()> {
        if frames==0||self.finished.iter().any(|finished|finished==id){return Ok(());}
        if !self.active.contains_key(id) {
            let Some(source)=source()? else{return Ok(())};
            let recorder=Recorder::new(self.store.clone(),self.owner.clone(),source.track_id,source.title,source.artist,source.duration);
            let recorder=if let Some(start)=start {recorder.with_measured_output(id.into(),source.listen_session_id,self.clock,start)?}else{recorder.with_measured(id.into(),source.listen_session_id,self.clock)};
            self.active.insert(id.into(),recorder);
        }
        let recorder=self.active.get_mut(id).unwrap();
        if recorder.row.snapshot.measured.as_ref().is_some_and(|m|(m.version==2)!=start.is_some()){anyhow::bail!("listening timing version cannot change within one playback instance");}
        recorder.sample_device(position,frames,rate)
    }
    pub fn finish(&mut self,id:&str,reason:&str)->Result<()> {
        if let Some(mut recorder)=self.active.remove(id){recorder.finish(reason)?;self.checkpoints.push_back(recorder.row.clone());while self.checkpoints.len()>9{self.checkpoints.pop_front();}}
        if !self.finished.iter().any(|finished|finished==id){self.finished.push_back(id.into());while self.finished.len()>128{self.finished.pop_front();}}
        Ok(())
    }
    /// A durable playback cursor must never advance beyond its saved actual listening interval.
    pub fn checkpoint_rows(&mut self,id:&str)->Vec<Row> {
        let mut rows:Vec<_>=self.checkpoints.iter().cloned().collect();
        if let Some(recorder)=self.active.get_mut(id){recorder.row.snapshot.version+=1;recorder.saved=self.clock.elapsed().as_secs_f64();rows.push(recorder.row.clone());}
        rows
    }
    pub fn finish_all(&mut self,reason:&str)->Result<()> {
        for id in self.active.keys().cloned().collect::<Vec<_>>() {self.finish(&id,reason)?;}
        Ok(())
    }
}
