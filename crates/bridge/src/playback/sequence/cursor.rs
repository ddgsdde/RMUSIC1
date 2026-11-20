use super::*;
use rmusic_bridge::listens::cursor::{Boundary, Recorder};
fn boundary(id: &str, position: f64, frames: u64, tracks: &Tracks) -> Result<Boundary> {
    let tracks = tracks
        .lock()
        .map_err(|_| anyhow::anyhow!("sequence tracks lock poisoned"))?;
    let entry = tracks.get(id).context("missing cursor source")?;
    Ok(Boundary {
        playback_id: id.into(),
        track_id: entry.track.clone(),
        position,
        frames,
    })
}
pub(super) fn update(
    recorder: &mut Recorder,
    update: &PlaybackUpdate,
    rate: u32,
    tracks: &Tracks,
    before_save: impl FnOnce() -> Result<Vec<rmusic_bridge::listens::Row>>,
) -> Result<()> {
    let ended = update
        .ended
        .iter()
        .map(|r| boundary(&r.playback_id, r.position, r.frames, tracks))
        .collect::<Result<Vec<_>>>()?;
    let terminal = update
        .ended
        .iter()
        .any(|r| r.playback_id == update.playback_id);
    let start = tracks
        .lock()
        .map_err(|_| anyhow::anyhow!("sequence tracks lock poisoned"))?
        .get(&update.playback_id)
        .and_then(|e| e.request.as_ref())
        .map(|r| r.position)
        .context("missing cursor request")?;
    let current = boundary(
        &update.playback_id,
        update.position,
        ((update.position - start) * rate as f64).round().max(0.) as u64,
        tracks,
    )?;
    recorder.sample_with(
        ended,
        current,
        rate,
        terminal,
        !update.ended.is_empty(),
        before_save,
    )
}
pub(super) fn finish(
    recorder: &mut Recorder,
    position: &SequencePosition,
    rate: u32,
    tracks: &Tracks,
    terminal: bool,
    rows: Vec<rmusic_bridge::listens::Row>,
) -> Result<()> {
    recorder.sample_with(
        vec![],
        boundary(
            &position.playback_id,
            position.position,
            position.frames,
            tracks,
        )?,
        rate,
        terminal,
        true,
        || Ok(rows),
    )
}
