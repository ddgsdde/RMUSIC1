//! Drain ready PCM into available driver credit before yielding the producer.
//! One-packet pacing caps throughput at packet duration / wake interval and
//! starves HAL even when the bounded read-ahead is full.
use super::*;

pub(super) struct Feed {
    pending: Vec<u8>,
    following: Option<Read>,
    cursor: usize,
    ended: bool,
    meter: Meter,
    frame_bytes: usize,
    packet_frames: usize,
}
impl Feed {
    pub fn new(format: SequenceFormat) -> Self {
        Self {
            pending: Vec::new(), following: None, cursor: 0, ended: false,
            meter: Meter::default(),
            frame_bytes: usize::from(format.channels) * if format.float_output && format.bit_depth == 64 { 8 } else { 4 },
            packet_frames: (format.sample_rate as usize / 100).clamp(80, 7680),
        }
    }
    pub fn ended(&self) -> bool { self.ended }
    pub fn submitted_all(&self) -> bool { self.ended && self.cursor == self.pending.len() }
    pub fn pending_bytes(&self) -> usize { self.pending.len().saturating_sub(self.cursor) }
    pub fn meter(&self) -> Meter { self.meter.clone() }
    /// Returns true only when source data is pending. Driver room is a bounded
    /// credit snapshot; callbacks may free more room while we fill that credit.
    pub fn pump(&mut self, driver: &mut dyn Driver, stream: &mut ReadAhead,
        sequence: &PlaybackSequence, control: &PlaybackControl, mut room: u64) -> Result<bool>
    {
        while room > 0 && !control.stopped.load(Ordering::Acquire) && !control.paused.load(Ordering::Acquire) {
            if self.cursor == self.pending.len() {
                if self.ended { break; }
                match self.following.take().map_or_else(|| stream.read(self.packet_frames), Ok)? {
                    Read::Data(chunk) => { self.pending = chunk.bytes; self.cursor = 0; self.meter = chunk.meter; }
                    Read::End => { self.ended = true; driver.finish_input(); break; }
                    Read::Pending => return Ok(Self::starved(stream, sequence)),
                }
            }
            // Preserve packet look-ahead: publish EOF atomically with the final
            // submitted frames, including a partial final HAL packet.
            if self.following.is_none() && !self.ended {
                match stream.read(self.packet_frames)? {
                    Read::End => self.ended = true,
                    Read::Data(chunk) => self.following = Some(Read::Data(chunk)),
                    Read::Pending => return Ok(Self::starved(stream, sequence)),
                }
            }
            let count = self.pending_bytes().min(room as usize * self.frame_bytes);
            let end = self.cursor + count;
            let frames = if self.ended && end == self.pending.len() {
                driver.write_final(&self.pending[self.cursor..end])?
            } else { driver.write(&self.pending[self.cursor..end])? };
            if frames > count / self.frame_bytes { bail!("invalid native driver write count"); }
            if frames == 0 { break; }
            self.cursor += frames * self.frame_bytes;
            room -= frames as u64;
            control.frames_written.fetch_add(frames as u64, Ordering::Release);
        }
        Ok(false)
    }
    fn starved(stream: &ReadAhead, sequence: &PlaybackSequence) -> bool {
        if stream.waiting_for_next() { sequence.finish_window_if_empty(); }
        true
    }
}
