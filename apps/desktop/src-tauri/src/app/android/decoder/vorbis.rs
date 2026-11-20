//! Granules bound fully lapped PCM, including a first audio page that is also EOS.
use super::*;
use symphonia::core::{formats::Packet,io::SeekBuffered};
pub(super) struct Decoder {
    packets:ogg::PacketReader<MediaSourceStream>,codec:Box<dyn symphonia::core::codecs::Decoder>,
    serial:u32,rate:u32,channels:u16,audio_packets:u64,frames:u64,origin:u64,ended:bool,
}
impl Decoder {
    pub(super) fn open(mut input:MediaSourceStream,codec:Box<dyn symphonia::core::codecs::Decoder>,serial:u32,rate:u32,channels:u16)->Result<Self> {
        if input.seek_buffered(0)!=0 {input.seek(SeekFrom::Start(0))?;}
        Ok(Self{packets:ogg::PacketReader::new(input),codec,serial,rate,channels,audio_packets:0,frames:0,origin:0,ended:false})
    }
    pub(super) fn read_block(&mut self)->Result<Option<Vec<u8>>> {
        loop {
            let packet=match self.packets.read_packet()? {Some(packet)=>packet,None if self.ended=>return Ok(None),None=>bail!("Vorbis 流未完整结束，缺少EOS")};
            if packet.stream_serial()!=self.serial {
                if self.ended&&packet.first_in_stream()&&packet.data.starts_with(b"\x01vorbis") {bail!("链式Vorbis需重新协商，尚不支持当前连续源");}
                continue;
            }
            if self.ended {bail!("Vorbis EOS之后仍有同流数据");}
            if packet.data.first().is_some_and(|byte|byte&1!=0) {
                if self.audio_packets!=0||!packet.data.get(1..7).is_some_and(|tag|tag==b"vorbis") {bail!("无效Vorbis头部顺序");}
                continue;
            }
            if packet.data.is_empty()||packet.data.len()>4*1024*1024 {bail!("Vorbis包超过有界预算或为空");}
            self.audio_packets+=1;
            let coded=Packet::new_from_slice(self.serial,self.frames,0,&packet.data);
            let decoded=self.codec.decode(&coded)?;
            if decoded.spec().rate!=self.rate||decoded.spec().channels.count()!=self.channels as usize {bail!("源格式变化；不自动重采样");}
            if decoded.capacity()>65536 {bail!("decoded block exceeds bounded preload budget");}
            let count=decoded.frames() as u64;let mut start=0;
            if packet.last_in_page()&&packet.absgp_page()==u64::MAX {bail!("Vorbis页缺少有效granule位置");}
            // A nonzero start offset must be exposed by the end of the second audio packet.
            if self.audio_packets==2&&packet.last_in_page()&&!packet.last_in_stream() {
                let granule=packet.absgp_page();
                if granule<count {start=count-granule;}else{self.origin=granule-count;}
            }
            let end=if packet.last_in_stream() {
                let total=packet.absgp_page().checked_sub(self.origin).context("Vorbis结束位置早于源起点")?;
                let remaining=total.checked_sub(self.frames).context("Vorbis结束位置倒退")?;
                if remaining>count {bail!("Vorbis结束位置超过实际解码帧");}
                self.ended=true;remaining
            }else{count};
            if start>end {bail!("Vorbis裁剪区间无效");}
            self.frames+=end-start;
            if start==end {continue;}
            let mut samples=SampleBuffer::<i32>::new(decoded.capacity() as u64,*decoded.spec());samples.copy_interleaved_ref(decoded);
            let mut bytes=Vec::with_capacity((end-start) as usize*self.channels as usize*4);
            for sample in &samples.samples()[start as usize*self.channels as usize..end as usize*self.channels as usize] {bytes.extend_from_slice(&sample.to_le_bytes());}
            return Ok(Some(bytes));
        }
    }
}
