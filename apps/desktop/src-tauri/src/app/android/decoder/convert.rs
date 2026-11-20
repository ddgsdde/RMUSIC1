use super::*;
use rmusic_audio::resample::PcmConverter;
pub(super) struct Float {pub inner:Box<dyn PcmDecoder>,pub channels:usize}
impl PcmDecoder for Float {
    fn read_frames(&mut self,output:&mut[u8],frame_bytes:usize)->Result<usize> {
        if frame_bytes!=self.channels*8||output.len()%frame_bytes!=0{bail!("invalid Android f64 decoder block");}
        let mut integers=vec![0;output.len()/2];let count=self.inner.read_frames(&mut integers,self.channels*4)?;
        for (source,target) in integers[..count*self.channels*4].chunks_exact(4).zip(output.chunks_exact_mut(8)) {
            target.copy_from_slice(&(i32::from_le_bytes(source.try_into().unwrap()) as f64/2147483648.).to_le_bytes());
        }Ok(count)
    }
    fn abort_handle(&self)->DecoderAbort{self.inner.abort_handle()}
    fn processing(&self)->Vec<String>{self.inner.processing()}
}
pub(super) struct Converted {inner:Box<dyn PcmDecoder>,converter:PcmConverter,source_channels:usize,source_rate:u32,target_rate:u32,target_channels:usize,pending:Vec<u8>,cursor:usize,eof:bool}
impl Converted {
    pub(super) fn new(inner:Box<dyn PcmDecoder>,request:&PlaybackRequest,format:SequenceFormat)->Result<Self>{
        Ok(Self{inner,converter:PcmConverter::new(request.sample_rate,format.sample_rate,request.channels,format.channels)?,source_channels:request.channels as usize,source_rate:request.sample_rate,target_rate:format.sample_rate,target_channels:format.channels as usize,pending:vec![],cursor:0,eof:false})
    }
}
impl PcmDecoder for Converted {
    fn read_frames(&mut self,output:&mut[u8],frame_bytes:usize)->Result<usize>{
        if frame_bytes!=self.target_channels*4||output.len()%frame_bytes!=0{bail!("invalid converted PCM block");}
        while self.cursor==self.pending.len(){
            if self.converter.ended(){return Ok(0);}
            let needed=self.converter.needed_frames();let source_frame=self.source_channels*4;
            let mut input=vec![0;needed*source_frame];let mut frames=0;
            while !self.eof&&frames<needed {let count=self.inner.read_frames(&mut input[frames*source_frame..],source_frame)?;if count==0{self.eof=true;break;}frames+=count;}
            if let Some(mask)=self.inner.channel_mask(){self.converter.channel_layout(mask)?;}
            input.truncate(frames*source_frame);self.pending=self.converter.convert(&input,self.eof)?;self.cursor=0;
        }
        let count=output.len().min(self.pending.len()-self.cursor);output[..count].copy_from_slice(&self.pending[self.cursor..self.cursor+count]);self.cursor+=count;Ok(count/frame_bytes)
    }
    fn abort_handle(&self)->DecoderAbort{self.inner.abort_handle()}
    fn processing(&self)->Vec<String>{let mut values=self.inner.processing();if self.source_rate!=self.target_rate{values.push(format!("原生 f64 sinc 重采样 {} → {} Hz",self.source_rate,self.target_rate));}if self.source_channels!=self.target_channels{values.push(format!("声道混音 {} → {}（防削波留量）",self.source_channels,self.target_channels));}values}
}
