use super::*;
use rmusic_bridge::listens::cursor::{Boundary,Recorder};

fn source(id:&str,position:f64,frames:u64,entries:&Entries)->Result<Boundary> {
    Ok(serde_json::from_value(boundary(id,position,frames,entries)?)?)
}
pub(super) fn update(recorder:&mut Recorder,update:&PlaybackUpdate,rate:u32,entries:&Entries,before_save:impl FnOnce()->Result<Vec<rmusic_bridge::listens::Row>>)->Result<()> {
    let ended=update.ended.iter().map(|item|source(&item.playback_id,item.position,item.frames,entries)).collect::<Result<Vec<_>>>()?;
    let terminal=update.ended.iter().any(|item|item.playback_id==update.playback_id);
    recorder.sample_with(ended,source(&update.playback_id,update.position,update.output_frames,entries)?,rate,terminal,!update.ended.is_empty(),before_save)
}
pub(super) fn finish(recorder:&mut Recorder,position:&SequencePosition,rate:u32,entries:&Entries,terminal:bool,rows:Vec<rmusic_bridge::listens::Row>)->Result<()> {
    recorder.sample_with(vec![],source(&position.playback_id,position.position,position.frames,entries)?,rate,terminal,true,||Ok(rows))
}
