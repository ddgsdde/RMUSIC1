//! Bounded, explicit PCM format conversion for native decoders without FFmpeg.
use crate::adaptive::Converter;
mod downmix;
use anyhow::{bail,Result};
pub struct PcmConverter {
    matrix:downmix::Matrix,converter:Option<Converter>,input_channels:usize,output_channels:usize,input_rate:u32,output_rate:u32,
    input_frames:u64,output_frames:u64,limit:Option<u64>,ended:bool,
}
impl PcmConverter {
    pub fn new(input_rate:u32,output_rate:u32,input_channels:u16,output_channels:u16)->Result<Self>{
        if !(1..=8).contains(&input_channels)||!(1..=2).contains(&output_channels){bail!("原生格式转换支持一至八声道输入及单／双声道输出");}
        if output_rate>input_rate{bail!("默认格式限制不允许升频");}
        let converter=if input_rate!=output_rate{Some(Converter::new_rates(input_rate,output_rate,output_channels as usize)?)}else{None};
        Ok(Self{matrix:downmix::Matrix::new(input_channels)?,converter,input_channels:input_channels as usize,output_channels:output_channels as usize,input_rate,output_rate,input_frames:0,output_frames:0,limit:None,ended:false})
    }
    pub fn channel_layout(&mut self,mask:u32)->Result<()> {self.matrix=downmix::Matrix::layout(self.input_channels as u16,mask)?;Ok(())}
    pub fn needed_frames(&self)->usize{self.converter.as_ref().map_or(4096,Converter::input_frames_next)}
    pub fn ended(&self)->bool{self.ended}
    pub fn convert(&mut self,bytes:&[u8],eof:bool)->Result<Vec<u8>> {
        if self.ended{return Ok(vec![]);}
        let frame_bytes=self.input_channels*4;
        if bytes.len()%frame_bytes!=0||bytes.len()/frame_bytes>self.needed_frames(){bail!("格式转换输入块不完整或超过内存窗口");}
        self.input_frames+=(bytes.len()/frame_bytes) as u64;
        if eof{self.limit=Some(((self.input_frames as u128*self.output_rate as u128+self.input_rate as u128/2)/self.input_rate as u128) as u64);}
        let mut input=Vec::with_capacity(bytes.len()/frame_bytes*self.output_channels);
        for frame in bytes.chunks_exact(frame_bytes){
            let [left,right]=self.matrix.mix(frame);
            if self.output_channels==1{input.push((left+right)/2.);}else{input.extend([left,right]);}
        }
        let values=if let Some(converter)=&mut self.converter{
            let mut output=vec![0.;converter.output_frames_max()*self.output_channels];
            let block=if self.limit.is_some(){converter.finish(&input,&mut output)?}else{converter.process(&input,&mut output)?};
            self.ended=block.ended;output.truncate(block.output_frames*self.output_channels);output
        }else{self.ended=eof;input};
        let frames=(values.len()/self.output_channels) as u64;
        let frames=self.limit.map_or(frames,|limit|frames.min(limit.saturating_sub(self.output_frames)));
        self.output_frames+=frames;
        let mut output=Vec::with_capacity(frames as usize*self.output_channels*4);
        for value in values.into_iter().take(frames as usize*self.output_channels){output.extend_from_slice(&(value.mul_add(2147483648.,0.).round().clamp(i32::MIN as f64,i32::MAX as f64) as i32).to_le_bytes());}
        Ok(output)
    }
}

#[cfg(test)]mod tests{
    use super::*;
    #[test]fn configured_limits_native_resampling_keeps_duration_and_rejects_implicit_upsampling(){
        let mut converter=PcmConverter::new(96000,48000,2,2).unwrap();let input=(0..9600).flat_map(|n|{let v=(0.1*(std::f64::consts::TAU*1000.*n as f64/96000.).sin()*2147483648.) as i32;[v.to_le_bytes(),v.to_le_bytes()].concat()}).collect::<Vec<_>>();
        let mut offset=0;let mut output=vec![];
        while !converter.ended(){let count=(converter.needed_frames()*8).min(input.len()-offset);let eof=offset+count==input.len();output.extend(converter.convert(&input[offset..offset+count],eof).unwrap());offset+=count;}
        assert_eq!(offset,input.len());assert_eq!(output.len()/8,4800);
        let peak=output.chunks_exact(4).map(|b|i32::from_le_bytes(b.try_into().unwrap()).unsigned_abs()).max().unwrap();assert!((200_000_000..230_000_000).contains(&peak));
        assert!(PcmConverter::new(44100,48000,2,2).is_err());
        let mut mono=PcmConverter::new(48000,48000,2,1).unwrap();assert_eq!(mono.convert(&[100_i32.to_le_bytes(),300_i32.to_le_bytes()].concat(),true).unwrap(),200_i32.to_le_bytes());
    }
}
