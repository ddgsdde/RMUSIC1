use super::*;
use rmusic_bridge::listens::{Collector,Source as ListenSource};

pub(super) struct Sampler {
    collector:Collector,version:u32,
}
impl Sampler {
    pub fn new(store:Arc<ListenStore>,owner:ListenOwner,version:u32)->Self {
        Self {collector:Collector::new(store,owner),version}
    }
    fn sample(&mut self,id:&str,position:f64,frames:u64,rate:u32,entries:&Entries)->Result<()> {
        let source=||{
            let entries=entries.lock().map_err(|_|anyhow::anyhow!("source lock poisoned"))?;
            let Some(entry)=entries.get(id) else{return Ok(None)};
            let (Some(group),Some(source))=(&entry.listen_session_id,&entry.source) else{return Ok(None)};
            Ok(Some(ListenSource {listen_session_id:group.clone(),track_id:source.track.id.clone(),title:source.track.title.clone().unwrap_or_else(||source.track.id.clone()),artist:source.track.artist.clone().unwrap_or_default(),duration:source.metadata.duration.unwrap_or(0.)}))
        };
        if self.version==2 {
            let start=entries.lock().map_err(|_|anyhow::anyhow!("source lock poisoned"))?.get(id).and_then(|entry|entry.request.as_ref()).map(|request|request.position);
            if let Some(start)=start{self.collector.sample_output(id,start,position,frames,rate,source)}else{Ok(())}
        }else{self.collector.sample(id,position,frames,rate,source)}
    }
    pub fn update(&mut self,update:&PlaybackUpdate,rate:u32,entries:&Entries)->Result<()> {
        for ended in &update.ended {
            self.sample(&ended.playback_id,ended.position,ended.frames,rate,entries)?;
            self.collector.finish(&ended.playback_id,"ended")?;
        }
        self.sample(&update.playback_id,update.position,update.output_frames,rate,entries)
    }
    pub fn stop(&mut self,position:Option<&SequencePosition>,rate:u32,entries:&Entries,reason:&str)->Result<()> {
        if let Some(position)=position {self.sample(&position.playback_id,position.position,position.frames,rate,entries)?;}
        self.collector.finish_all(reason)
    }
    pub fn checkpoint_rows(&mut self,id:&str)->Vec<rmusic_bridge::listens::Row>{self.collector.checkpoint_rows(id)}
}
