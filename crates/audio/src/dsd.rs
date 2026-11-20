//! Raw DSF/DSDIFF transport and DoP 1.1 packing. No PCM conversion or device output.
use crate::decoder::{DecodeFailure,DecoderAbort,FfmpegBytes};
use anyhow::{bail,Context,Result};
use std::io::Read;

const HEADER_LIMIT:u64=16*1024*1024;
const DSF_BLOCK:u64=4096;
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
enum Storage {Dsf{lsb_first:bool},Dff}
#[derive(Debug,Clone,PartialEq,Eq)]
pub struct DsdLayout {
    pub sample_rate:u32,
    pub channels:u16,
    /// Known channel order in WAVE speaker-mask order; zero means unrecognized/ambiguous.
    pub channel_mask:u32,
    /// Number of single-bit samples per channel, excluding file-block/DoP padding.
    pub samples:u64,
    pub data_offset:u64,
    pub data_end:u64,
    pub file_end:u64,
    storage:Storage,
}
impl DsdLayout {
    pub fn duration(&self)->f64 {self.samples as f64/self.sample_rate as f64}
    pub fn dop_rate(&self)->Result<u32> {
        let rate=self.sample_rate/16;
        if self.sample_rate%16!=0||!(8000..=768000).contains(&rate){bail!("DSD rate exceeds the 768 kHz DoP carrier limit");}
        Ok(rate)
    }
    pub fn frame_at(&self,seconds:f64)->Result<u64> {
        if !seconds.is_finite()||seconds<0. {bail!("invalid DSD seek position");}
        if seconds>=self.duration(){return Ok(self.samples.div_ceil(16));}
        // Seek to the nearest 16-bit boundary; round-tripping seconds must not lose a frame.
        Ok(((seconds*(self.sample_rate/16) as f64).round() as u64).min(self.samples.div_ceil(16)))
    }
    /// Restart the byte transport here; DSF seeks to a complete channel-block group.
    pub fn seek_offset(&self,frame:u64)->Result<u64> {
        if frame>self.samples.div_ceil(16){bail!("DSD seek exceeds the source");}
        let byte=frame.checked_mul(2).context("DSD seek overflow")?;
        self.byte_offset(byte)
    }
    fn byte_offset(&self,byte:u64)->Result<u64> {
        let relative=match self.storage {Storage::Dsf{..}=>byte/DSF_BLOCK*DSF_BLOCK*self.channels as u64,Storage::Dff=>byte*self.channels as u64};
        self.data_offset.checked_add(relative).context("DSD seek overflow")
    }
    pub fn read(reader:&mut impl Read)->Result<Self> {
        let mut header=Header{reader,offset:0};
        let kind=header.bytes::<4>()?;
        let layout=match &kind {
            b"DSD "=>dsf(&mut header)?,
            b"FRM8"=>dff(&mut header)?,
            _=>bail!("expected a DSF or DSDIFF source"),
        };
        if ![2822400,3072000,5644800,6144000,11289600,12288000,22579200,24576000].contains(&layout.sample_rate)
            ||layout.samples==0||!(1..=6).contains(&layout.channels) {bail!("unsupported DSD rate, channel count or empty source");}
        Ok(layout)
    }
}
struct Header<'a,R>{reader:&'a mut R,offset:u64}
impl<R:Read> Header<'_,R> {
    fn bytes<const N:usize>(&mut self)->Result<[u8;N]> {
        self.offset=self.offset.checked_add(N as u64).context("DSD header overflow")?;
        if self.offset>HEADER_LIMIT {bail!("DSD header exceeds 16 MiB");}
        let mut value=[0;N];read_exact(self.reader,&mut value)?;Ok(value)
    }
    fn skip(&mut self,bytes:u64)->Result<()> {
        if self.offset.checked_add(bytes).is_none_or(|end|end>HEADER_LIMIT){bail!("DSD header exceeds 16 MiB");}
        let mut remaining=bytes;let mut buffer=[0;4096];
        while remaining>0 {let n=remaining.min(buffer.len() as u64) as usize;read_exact(self.reader,&mut buffer[..n])?;remaining-=n as u64;}
        self.offset+=bytes;Ok(())
    }
    fn le32(&mut self)->Result<u32>{Ok(u32::from_le_bytes(self.bytes()?))}
    fn le64(&mut self)->Result<u64>{Ok(u64::from_le_bytes(self.bytes()?))}
    fn be32(&mut self)->Result<u32>{Ok(u32::from_be_bytes(self.bytes()?))}
    fn be64(&mut self)->Result<u64>{Ok(u64::from_be_bytes(self.bytes()?))}
}
fn read_exact(reader:&mut impl Read,bytes:&mut[u8])->Result<()> {
    reader.read_exact(bytes).map_err(|error|DecodeFailure(format!("truncated or unavailable DSD data: {error}")).into())
}
fn dsf<R:Read>(header:&mut Header<'_,R>)->Result<DsdLayout> {
    if header.le64()?!=28 {bail!("invalid DSF DSD chunk");}
    let file_size=header.le64()?;let metadata=header.le64()?;
    if header.bytes::<4>()?!=*b"fmt "||header.le64()?!=52||header.le32()?!=1||header.le32()?!=0 {bail!("unsupported DSF format chunk");}
    let channel_type=header.le32()?;let channels=header.le32()?;
    let expected=[0,1,2,3,4,4,5,6].get(channel_type as usize).copied().unwrap_or(0);
    if channels!=expected||expected==0 {bail!("invalid DSF channel layout");}
    let sample_rate=header.le32()?;let bits=header.le32()?;
    if ![1,8].contains(&bits){bail!("unsupported DSF bit order");}
    let samples=header.le64()?;
    if header.le32()?!=4096||header.le32()?!=0||header.bytes::<4>()?!=*b"data" {bail!("invalid DSF block or data chunk");}
    let size=header.le64()?.checked_sub(12).context("invalid DSF data size")?;
    let audio_bytes=samples.checked_add(7).context("DSF sample count overflow")?/8;
    let padded=audio_bytes.div_ceil(DSF_BLOCK).checked_mul(DSF_BLOCK*channels as u64).context("DSF data size overflow")?;
    let end=header.offset.checked_add(size).context("DSF data size overflow")?;
    if size!=padded||end>file_size||(metadata!=0&&(metadata<end||metadata>=file_size)){bail!("DSF data size or metadata offset disagrees with its sample count");}
    Ok(DsdLayout{sample_rate,channels:channels as u16,channel_mask:[0,4,3,7,0x33,0xf,0x37,0x3f][channel_type as usize],samples,data_offset:header.offset,data_end:end,file_end:file_size,storage:Storage::Dsf{lsb_first:bits==1}})
}
fn dff<R:Read>(header:&mut Header<'_,R>)->Result<DsdLayout> {
    let end=header.be64()?.checked_add(12).context("DSDIFF size overflow")?;
    if header.bytes::<4>()?!=*b"DSD "{bail!("unsupported DSDIFF form");}
    let (mut rate,mut channels,mut raw)=(None,None,false);
    let mut channel_mask=0;
    for _ in 0..256 {
        if header.offset.checked_add(12).is_none_or(|next|next>end){bail!("DSDIFF has no audio data");}
        let kind=header.bytes::<4>()?;let size=header.be64()?;
        let chunk_end=header.offset.checked_add(size).context("DSDIFF chunk size overflow")?;
        if chunk_end>end {bail!("DSDIFF chunk exceeds its form");}
        match &kind {
            b"PROP"=>{
                if size<4||header.bytes::<4>()?!=*b"SND "{bail!("invalid DSDIFF sound properties");}
                for _ in 0..256 {
                    if header.offset==chunk_end {break;}
                    if header.offset.checked_add(12).is_none_or(|next|next>chunk_end){bail!("truncated DSDIFF property");}
                    let property=header.bytes::<4>()?;let length=header.be64()?;
                    let property_end=header.offset.checked_add(length).context("DSDIFF property overflow")?;
                    if property_end>chunk_end {bail!("DSDIFF property exceeds its chunk");}
                    match &property {
                        b"FS  "=>{if length!=4||rate.is_some(){bail!("invalid DSDIFF sample rate property");}rate=Some(header.be32()?);},
                        b"CHNL"=>{
                            if length<2||channels.is_some(){bail!("invalid DSDIFF channel property");}
                            let count=u16::from_be_bytes(header.bytes()?);
                            if !(1..=6).contains(&count)||length!=2+count as u64*4 {bail!("invalid DSDIFF channel count");}
                            // Preserve the declared channel order; device/channel-map admission is separate.
                            let mut labels=Vec::with_capacity(count as usize);
                            for _ in 0..count{labels.push(header.bytes::<4>()?);}
                            channel_mask=if labels.as_slice()==[*b"SLFT",*b"SRGT"]{3}
                                else if labels.as_slice()==[*b"MLFT",*b"MRGT",*b"C   ",*b"LS  ",*b"RS  "]{0x607}
                                else if labels.as_slice()==[*b"MLFT",*b"MRGT",*b"C   ",*b"LFE ",*b"LS  ",*b"RS  "]{0x60f}else{0};
                            channels=Some(count);
                        },
                        b"CMPR"=>{
                            if length<5||header.bytes::<4>()?!=*b"DSD "{bail!("compressed DST is not raw DSD");}
                            raw=true;header.skip(length-4)?;
                        },
                        _=>header.skip(length)?,
                    }
                    header.skip(length%2)?;
                }
                if header.offset!=chunk_end {bail!("too many DSDIFF properties");}
            },
            b"DSD "=>{
                let channels=channels.context("missing DSDIFF channels")?;
                if !raw||size%channels as u64!=0 {bail!("invalid raw DSDIFF data");}
                let samples=(size/channels as u64).checked_mul(8).context("DSDIFF sample count overflow")?;
                let data_end=chunk_end.checked_add(size%2).filter(|padded|*padded<=end).context("DSDIFF data padding exceeds its form")?;
                return Ok(DsdLayout{sample_rate:rate.context("missing DSDIFF rate")?,channels,channel_mask,samples,data_offset:header.offset,data_end,file_end:end,storage:Storage::Dff});
            },
            b"DST "=>bail!("compressed DST is not supported"),
            _=>header.skip(size)?,
        }
        header.skip(size%2)?;
    }
    bail!("too many DSDIFF chunks")
}

#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub enum DopContainer {Packed24,LeftAligned32}
impl DopContainer {pub fn bytes(self)->usize {if self==Self::Packed24{3}else{4}}}
/// One encoder per physical output session, including silence and track changes.
pub struct DopEncoder {channels:usize,frames:u64}
impl DopEncoder {
    pub fn new(channels:u16)->Result<Self> {
        if !(1..=6).contains(&channels){bail!("invalid DoP channel count");}
        Ok(Self{channels:channels as usize,frames:0})
    }
    /// Input: two MSB-first DSD bytes per channel, in byte-interleaved time order.
    pub fn pack(&mut self,input:&[u8],output:&mut[u8],container:DopContainer,muted:bool)->Result<usize> {
        let source_frame=self.channels*2;let target_frame=self.channels*container.bytes();
        if input.len()%source_frame!=0||output.len()<input.len()/source_frame*target_frame {bail!("invalid DoP buffer");}
        for (input,output) in input.chunks_exact(source_frame).zip(output.chunks_exact_mut(target_frame)) {
            self.frame(input,output,container,muted);
        }
        Ok(input.len()/source_frame)
    }
    pub fn silence(&mut self,output:&mut[u8],container:DopContainer)->Result<()> {
        let frame=self.channels*container.bytes();
        if output.len()%frame!=0{bail!("invalid DoP silence buffer");}
        for output in output.chunks_exact_mut(frame){self.frame(&[0x69;12][..self.channels*2],output,container,true);}
        Ok(())
    }
    /// Stamp the physical packet after staging/preload; tail and mute remain valid DSD silence.
    pub fn stamp(&mut self,output:&mut[u8],container:DopContainer,programme_frames:usize,muted:bool)->Result<()> {
        let width=self.channels*container.bytes();
        if output.len()%width!=0||programme_frames>output.len()/width {bail!("invalid DoP device packet");}
        for (frame,output) in output.chunks_exact_mut(width).enumerate() {
            let marker=self.marker();
            for sample in output.chunks_exact_mut(container.bytes()) {
                let offset=if container==DopContainer::LeftAligned32{sample[0]=0;1}else{0};
                if muted||frame>=programme_frames{sample[offset]=0x69;sample[offset+1]=0x69;}
                sample[offset+2]=marker;
            }
        }Ok(())
    }
    pub fn marker(&mut self)->u8 {let marker=if self.frames%2==0{0x05}else{0xfa};self.frames=self.frames.wrapping_add(1);marker}
    fn frame(&mut self,input:&[u8],output:&mut[u8],container:DopContainer,muted:bool) {
        let marker=self.marker();
        for (channel,sample) in output.chunks_exact_mut(container.bytes()).enumerate() {
            let older=if muted{0x69}else{input[channel]};
            let newer=if muted{0x69}else{input[channel+self.channels]};
            let offset=if container==DopContainer::LeftAligned32{sample[0]=0;1}else{0};
            sample[offset..].copy_from_slice(&[newer,older,marker]);
        }
    }
}
#[derive(Debug)]
pub struct DsdReader<R:Read> {reader:R,layout:DsdLayout,frame:u64,frame_bits:u64,block:Vec<u8>,loaded:bool}
impl DsdReader<FfmpegBytes> {pub fn abort_handle(&self)->DecoderAbort {self.reader.abort_handle()}}
impl<R:Read> DsdReader<R> {
    pub fn new(mut reader:R)->Result<Self> {let layout=DsdLayout::read(&mut reader)?;Self::at(reader,layout,0)}
    /// The reader must already be positioned at layout.seek_offset(frame).
    pub fn at(reader:R,layout:DsdLayout,frame:u64)->Result<Self> {
        layout.seek_offset(frame)?;
        Self::at_bits(reader,layout,frame,16)
    }
    fn at_bits(reader:R,layout:DsdLayout,frame:u64,frame_bits:u64)->Result<Self> {
        let block=if matches!(layout.storage,Storage::Dsf{..}){vec![0;DSF_BLOCK as usize*layout.channels as usize]}else{Vec::new()};
        Ok(Self{reader,layout,frame,frame_bits,block,loaded:false})
    }
    pub fn layout(&self)->&DsdLayout {&self.layout}
    pub fn position(&self)->f64 {(self.frame.saturating_mul(self.frame_bits).min(self.layout.samples)) as f64/self.layout.sample_rate as f64}
    fn read_frame(&mut self,pair:&mut[u8])->Result<bool> {
        if self.frame>=self.layout.samples.div_ceil(self.frame_bits){return Ok(false);}
        let channels=self.layout.channels as usize;let byte=self.frame*(self.frame_bits/8);
        let valid=(self.layout.samples-self.frame*self.frame_bits).min(self.frame_bits) as usize;
        match self.layout.storage {
            Storage::Dsf{lsb_first}=>{
                let offset=(byte%DSF_BLOCK) as usize;
                if !self.loaded||offset==0 {read_exact(&mut self.reader,&mut self.block)?;self.loaded=true;}
                for channel in 0..channels {for index in 0..self.frame_bits as usize/8 {
                    let raw=self.block[channel*DSF_BLOCK as usize+offset+index];
                    pair[index*channels+channel]=if lsb_first{raw.reverse_bits()}else{raw};
                }}
            },
            Storage::Dff=>read_exact(&mut self.reader,&mut pair[..channels*valid.div_ceil(8)])?,
        }
        for index in 0..self.frame_bits as usize/8 {
            let bits=valid.saturating_sub(index*8).min(8);let mask=if bits==0{0}else{0xff_u8<<(8-bits)};
            for channel in 0..channels {let value=&mut pair[index*channels+channel];*value=(*value&mask)|(0x69&!mask);}
        }
        self.frame+=1;Ok(true)
    }
    /// Interleaved raw DSD bytes, chronological MSB first; one frame carries eight bits per channel.
    pub fn read_native(&mut self,output:&mut[u8])->Result<usize> {
        let channels=self.layout.channels as usize;
        if self.frame_bits!=8||output.len()%channels!=0 {bail!("invalid native DSD buffer");}
        let mut frames=0;
        for frame in output.chunks_exact_mut(channels) {frame.fill(0x69);if !self.read_frame(frame)?{break;}frames+=1;}
        Ok(frames)
    }
    pub fn read_dop(&mut self,encoder:&mut DopEncoder,output:&mut[u8],container:DopContainer,muted:bool)->Result<usize> {
        let channels=self.layout.channels as usize;let width=channels*container.bytes();
        if self.frame_bits!=16||encoder.channels!=channels||output.len()%width!=0 {bail!("DSD/DoP channel or buffer mismatch");}
        let mut frames=0;
        for output in output.chunks_exact_mut(width) {
            let mut pair=[0x69;12];
            if !self.read_frame(&mut pair)?{break;}
            encoder.frame(&pair[..channels*2],output,container,muted);frames+=1;
        }
        Ok(frames)
    }
}
/// Register each transport before reading its header so stop/seek can abort blocked I/O.
pub fn open(input:&str,position:f64,mut register:impl FnMut(DecoderAbort)->Result<()>)->Result<DsdReader<FfmpegBytes>> {
    open_bits(input,position,16,&mut register)
}
pub fn open_native(input:&str,position:f64,mut register:impl FnMut(DecoderAbort)->Result<()>)->Result<DsdReader<FfmpegBytes>> {
    open_bits(input,position,8,&mut register)
}
fn open_bits(input:&str,position:f64,frame_bits:u64,mut register:impl FnMut(DecoderAbort)->Result<()>)->Result<DsdReader<FfmpegBytes>> {
    if !position.is_finite()||position<0. {bail!("invalid DSD seek position");}
    let mut source=FfmpegBytes::open(input,0)?;register(source.abort_handle())?;
    let layout=DsdLayout::read(&mut source)?;
    let frame=if frame_bits==16{layout.frame_at(position)?}else if position>=layout.duration(){layout.samples.div_ceil(8)}else{(position*(layout.sample_rate/8) as f64).round() as u64};
    if frame>0 {
        let offset=layout.byte_offset(frame*(frame_bits/8))?;
        drop(source);
        source=FfmpegBytes::open(input,offset)?;register(source.abort_handle())?;
    }
    DsdReader::at_bits(source,layout,frame,frame_bits)
}

#[cfg(test)]
pub(crate) mod tests;
