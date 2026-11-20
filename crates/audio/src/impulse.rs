//! Bounded, uncompressed RIFF/WAVE impulse import. Sample order and gain are retained.
use anyhow::{bail,Result};
use crate::convolution::{validate_coefficients,MAX_COEFFICIENTS};
use std::{collections::{BTreeMap,BTreeSet},sync::Arc};

pub const MAX_WAV_BYTES:usize=4*1024*1024;
pub const MAX_BANK_RESPONSES:usize=16;
pub const MAX_BANK_COEFFICIENTS:usize=1024*1024;
pub const MAX_BANK_BYTES:usize=32*1024*1024;
#[derive(Clone,Debug,PartialEq,serde::Serialize,serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConvolutionConfig {pub response_ids:Vec<String>}
impl ConvolutionConfig {
    pub fn validate(&self)->Result<()> {
        if self.response_ids.is_empty()||self.response_ids.len()>MAX_BANK_RESPONSES {bail!("响应组须包含 1 到 {MAX_BANK_RESPONSES} 个文件");}
        let mut unique=BTreeSet::new();
        for id in &self.response_ids {if id.len()!=64||!id.bytes().all(|byte|byte.is_ascii_digit()||(b'a'..=b'f').contains(&byte))||!unique.insert(id){bail!("响应文件 ID 无效或重复");}}
        Ok(())
    }
}
/// One explicitly selected response per rate; no nearest-rate fallback or resampling.
#[derive(Clone,Debug,PartialEq)]
pub struct ImpulseBank(BTreeMap<u32,Arc<ImpulseResponse>>);
impl ImpulseBank {
    pub fn new(responses:Vec<Arc<ImpulseResponse>>)->Result<Self> {
        if responses.is_empty()||responses.len()>MAX_BANK_RESPONSES {bail!("响应组须包含 1 到 {MAX_BANK_RESPONSES} 个采样率响应");}
        let(mut rates,mut coefficients)=(BTreeMap::new(),0usize);
        for response in responses {
            coefficients+=response.frames()*response.channels();
            if coefficients>MAX_BANK_COEFFICIENTS {bail!("响应组系数超过 {MAX_BANK_COEFFICIENTS} 个的加载预算");}
            if rates.insert(response.sample_rate(),response).is_some(){bail!("响应组每个采样率只能选择一个文件");}
        }
        Ok(Self(rates))
    }
    pub fn response(&self,rate:u32)->Result<&Arc<ImpulseResponse>> {
        self.0.get(&rate).ok_or_else(||anyhow::anyhow!("缺少 {rate} Hz 的卷积响应，请导入并选择对应采样率文件"))
    }
    pub fn validate_output(&self,rate:u32,channels:u16,mask:Option<u32>)->Result<&Arc<ImpulseResponse>> {
        let response=self.response(rate)?;response.validate_output(rate,channels,mask)?;Ok(response)
    }
}
#[derive(Clone,Debug,PartialEq)]
pub struct ImpulseResponse {
    sample_rate:u32,
    channel_mask:u32,
    kernels:Vec<Vec<f64>>,
}
impl ImpulseResponse {
    pub fn sample_rate(&self)->u32{self.sample_rate}
    pub fn channel_mask(&self)->u32{self.channel_mask}
    pub fn channels(&self)->usize{self.kernels.len()}
    pub fn frames(&self)->usize{self.kernels[0].len()}
    pub fn kernels(&self)->&[Vec<f64>]{&self.kernels}
    pub fn validate_output(&self,rate:u32,channels:u16,mask:Option<u32>)->Result<()> {
        if self.sample_rate!=rate {bail!("FIR impulse response must match the processing sample rate");}
        if channels==0||channels>32||!(self.channels()==1||self.channels()==channels as usize) {bail!("FIR response channels do not match the output");}
        if self.frames()*channels as usize>crate::convolution::MAX_EXPANDED_COEFFICIENTS {bail!("FIR exceeds the output channel processing budget");}
        if self.channels()>1&&self.channel_mask!=0&&mask.is_some_and(|mask|self.channel_mask!=mask) {bail!("FIR response speaker layout does not match the known output layout");}
        Ok(())
    }
    pub fn from_wav(bytes:&[u8])->Result<Self> {
        if bytes.len()<12||bytes.len()>MAX_WAV_BYTES||&bytes[..4]!=b"RIFF"||&bytes[8..12]!=b"WAVE"
            ||(u32le(&bytes[4..8]) as usize).checked_add(8)!=Some(bytes.len()) {bail!("invalid or oversized RIFF/WAVE impulse");}
        let(mut format,mut data)=(None,None);let mut offset=12;
        while offset<bytes.len() {
            if bytes.len()-offset<8 {bail!("truncated WAVE chunk");}
            let size=u32le(&bytes[offset+4..offset+8]) as usize;
            let end=offset.checked_add(8).and_then(|start|start.checked_add(size)).filter(|end|*end<=bytes.len()).ok_or_else(||anyhow::anyhow!("invalid WAVE chunk size"))?;
            let chunk=&bytes[offset+8..end];
            match &bytes[offset..offset+4] {
                b"fmt "=>{if format.replace(chunk).is_some(){bail!("duplicate WAVE format");}},
                b"data"=>{if data.replace(chunk).is_some(){bail!("multiple WAVE data chunks are unsupported");}},
                _=>{},
            }
            offset=end.checked_add(size%2).filter(|end|*end<=bytes.len()).ok_or_else(||anyhow::anyhow!("missing WAVE chunk padding"))?;
        }
        let format=format.filter(|v|v.len()>=16).ok_or_else(||anyhow::anyhow!("missing WAVE format"))?;
        let data=data.ok_or_else(||anyhow::anyhow!("missing WAVE samples"))?;
        let mut tag=u16le(&format[..2]);let channels=u16le(&format[2..4]) as usize;let sample_rate=u32le(&format[4..8]);
        let align=u16le(&format[12..14]) as usize;let bits=u16le(&format[14..16]);let(mut valid_bits,mut channel_mask)=(bits,0);
        if tag==0xfffe {
            if format.len()<40||u16le(&format[16..18])<22||18+u16le(&format[16..18]) as usize>format.len()
                ||format[28..40]!=[0,0,16,0,128,0,0,170,0,56,155,113] {bail!("invalid extensible WAVE format");}
            let subformat=u32le(&format[24..28]);if subformat!=1&&subformat!=3 {bail!("unsupported WAVE subformat");}
            tag=subformat as u16;valid_bits=u16le(&format[18..20]);channel_mask=u32le(&format[20..24]);
        }
        let supported=(tag==1&&[8,16,24,32].contains(&bits))||(tag==3&&[32,64].contains(&bits));
        if !supported||channels==0||channels>32||sample_rate<8000||valid_bits==0||valid_bits>bits||(tag==3&&valid_bits!=bits)
            ||(channel_mask!=0&&channel_mask.count_ones() as usize!=channels) {bail!("unsupported WAVE sample format or channel layout");}
        let width=bits as usize/8;
        if align!=channels*width||(align as u32).checked_mul(sample_rate)!=Some(u32le(&format[8..12]))
            ||data.is_empty()||data.len()%align!=0||data.len()/width>MAX_COEFFICIENTS {bail!("invalid WAVE frame layout or FIR coefficient budget");}
        let frames=data.len()/align;let mut kernels=vec![Vec::with_capacity(frames);channels];
        let scale=(1u64<<(bits-1)) as f64;
        for (index,sample) in data.chunks_exact(width).enumerate() {
            let value=if tag==3 {
                if bits==32 {f32::from_le_bytes(sample.try_into().unwrap()) as f64}else{f64::from_le_bytes(sample.try_into().unwrap())}
            }else{
                let mut raw=[0;4];raw[..width].copy_from_slice(sample);let raw=u32::from_le_bytes(raw);let unused=bits-valid_bits;
                if unused>0&&raw&((1u32<<unused)-1)!=0 {bail!("nonzero unused PCM bits");}
                if bits==8 {(raw as f64-128.)/128.}else{(((raw as i32)<<(32-bits))>>(32-bits)) as f64/scale}
            };
            kernels[index%channels].push(value);
        }
        validate_coefficients(&kernels)?;Ok(Self{sample_rate,channel_mask,kernels})
    }
}
fn u16le(bytes:&[u8])->u16{u16::from_le_bytes(bytes.try_into().unwrap())}
fn u32le(bytes:&[u8])->u32{u32::from_le_bytes(bytes.try_into().unwrap())}

#[cfg(test)]
mod tests {
    use super::*;
    fn wav(tag:u16,bits:u16,channels:u16,data:&[u8],extensible:bool)->Vec<u8>{
        let align=channels*bits/8;let mut format=Vec::new();
        format.extend((if extensible{0xfffe_u16}else{tag}).to_le_bytes());format.extend(channels.to_le_bytes());format.extend(48000_u32.to_le_bytes());format.extend((48000_u32*align as u32).to_le_bytes());format.extend(align.to_le_bytes());format.extend(bits.to_le_bytes());
        if extensible {format.extend(22_u16.to_le_bytes());format.extend(bits.to_le_bytes());format.extend(0_u32.to_le_bytes());format.extend((tag as u32).to_le_bytes());format.extend([0,0,16,0,128,0,0,170,0,56,155,113]);}
        let mut bytes=b"RIFF\0\0\0\0WAVEfmt ".to_vec();bytes.extend((format.len() as u32).to_le_bytes());bytes.extend(format);bytes.extend(b"data");bytes.extend((data.len() as u32).to_le_bytes());bytes.extend(data);if data.len()%2!=0 {bytes.push(0);}
        let size=bytes.len() as u32-8;bytes[4..8].copy_from_slice(&size.to_le_bytes());bytes
    }
    #[test]
    fn pcm_widths_keep_exact_gain_and_channel_order(){
        for (bits,data,positive) in [(8,vec![0,255],127./128.),(16,vec![0,128,255,127],32767./32768.),(24,vec![0,0,128,255,255,127],8388607./8388608.),(32,vec![0,0,0,128,255,255,255,127],2147483647./2147483648.)] {
            let impulse=ImpulseResponse::from_wav(&wav(1,bits,2,&data,false)).unwrap();assert_eq!(impulse.sample_rate(),48000);assert_eq!(impulse.kernels(),&[vec![-1.],vec![positive]]);
        }
    }
    #[test]
    fn float_responses_keep_original_gain_and_f64_bit_patterns(){
        let values=[2.,-3.,-0.,f64::from_bits(1)];let data=values.iter().flat_map(|v|v.to_le_bytes()).collect::<Vec<_>>();
        let impulse=ImpulseResponse::from_wav(&wav(3,64,1,&data,true)).unwrap();assert_eq!(impulse.kernels()[0].iter().map(|v|v.to_bits()).collect::<Vec<_>>(),values.iter().map(|v|v.to_bits()).collect::<Vec<_>>());
        let data=[2_f32,-3.].iter().flat_map(|v|v.to_le_bytes()).collect::<Vec<_>>();assert_eq!(ImpulseResponse::from_wav(&wav(3,32,1,&data,false)).unwrap().kernels()[0],vec![2.,-3.]);
    }
    #[test]
    fn extensible_valid_bits_layout_and_odd_unknown_chunks_are_preserved(){
        let mut bytes=wav(1,32,2,&[0,0,0,128,0,255,255,127],true);bytes[38..40].copy_from_slice(&24_u16.to_le_bytes());bytes[40..44].copy_from_slice(&3_u32.to_le_bytes());
        bytes.splice(12..12,b"JUNK\x01\0\0\0x\0".iter().copied());let size=bytes.len() as u32-8;bytes[4..8].copy_from_slice(&size.to_le_bytes());
        let impulse=ImpulseResponse::from_wav(&bytes).unwrap();assert_eq!(impulse.channel_mask(),3);assert_eq!(impulse.kernels(),&[vec![-1.],vec![8388607./8388608.]]);
    }
    #[test]
    fn configured_selection_requires_unique_bounded_content_ids(){
        for response_ids in [vec![],vec!["0".repeat(64);17],vec!["0".repeat(64);2],vec!["A".repeat(64)],vec!["../file".into()]]{assert!(ConvolutionConfig{response_ids}.validate().is_err());}
        assert!(ConvolutionConfig{response_ids:vec!["0".repeat(64),"abcdef0123456789".repeat(4)]}.validate().is_ok());
    }
    #[test]
    fn selected_bank_requires_exact_rates_and_preserves_each_responses_gain(){
        let response=|rate:u32,values:&[f64]|{
            let data=values.iter().flat_map(|value|value.to_le_bytes()).collect::<Vec<_>>();let mut bytes=wav(3,64,1,&data,false);bytes[24..28].copy_from_slice(&rate.to_le_bytes());bytes[28..32].copy_from_slice(&(rate*8).to_le_bytes());Arc::new(ImpulseResponse::from_wav(&bytes).unwrap())
        };
        let a=response(48000,&[2.,-3.,-0.]);let b=response(96000,&[-5.,7.]);let bank=ImpulseBank::new(vec![b.clone(),a.clone()]).unwrap();
        assert!(Arc::ptr_eq(bank.response(48000).unwrap(),&a));assert!(Arc::ptr_eq(bank.response(96000).unwrap(),&b));assert_eq!(bank.response(48000).unwrap().kernels()[0][2].to_bits(),(-0_f64).to_bits());
        assert!(bank.response(44100).unwrap_err().to_string().contains("44100 Hz"));assert!(bank.response(192000).is_err());assert!(ImpulseBank::new(vec![a.clone(),a]).unwrap_err().to_string().contains("每个采样率"));
    }
    #[test]
    fn selected_bank_has_bounded_count_and_total_loaded_coefficients(){
        assert!(ImpulseBank::new(vec![]).is_err());let tiny=Arc::new(ImpulseResponse::from_wav(&wav(3,64,1,&1_f64.to_le_bytes(),false)).unwrap());
        assert!(ImpulseBank::new(vec![tiny;MAX_BANK_RESPONSES+1]).unwrap_err().to_string().contains("16"));
        let mut responses=Vec::new();
        for rate in [44100_u32,48000,88200,96000,192000]{let mut bytes=wav(3,64,1,&vec![0;MAX_COEFFICIENTS*8],false);bytes[24..28].copy_from_slice(&rate.to_le_bytes());bytes[28..32].copy_from_slice(&(rate*8).to_le_bytes());responses.push(Arc::new(ImpulseResponse::from_wav(&bytes).unwrap()));}
        assert!(ImpulseBank::new(responses[..4].to_vec()).is_ok());assert!(ImpulseBank::new(responses).unwrap_err().to_string().contains("加载预算"));
    }
    #[test]
    fn malformed_headers_chunks_and_nonfinite_or_unbounded_samples_are_rejected(){
        let valid=wav(3,64,1,&1_f64.to_le_bytes(),false);
        for end in 0..valid.len(){assert!(ImpulseResponse::from_wav(&valid[..end]).is_err());}
        for value in [f64::NAN,f64::INFINITY,f64::MAX] {assert!(ImpulseResponse::from_wav(&wav(3,64,1,&value.to_le_bytes(),false)).is_err());}
        for index in [4,16,20,24,28,32,34,40] {let mut bad=valid.clone();bad[index]^=0x7f;assert!(ImpulseResponse::from_wav(&bad).is_err(),"index {index}");}
        let mut bad=valid.clone();bad.extend_from_slice(&valid[12..]);let size=bad.len() as u32-8;bad[4..8].copy_from_slice(&size.to_le_bytes());assert!(ImpulseResponse::from_wav(&bad).is_err());
        assert!(ImpulseResponse::from_wav(&wav(1,16,1,&vec![0;2*(MAX_COEFFICIENTS+1)],false)).is_err());
        assert!(ImpulseResponse::from_wav(&vec![0;MAX_WAV_BYTES+1]).is_err());
    }
    #[test]
    fn empty_compressed_partial_frames_and_invalid_extensible_bits_are_rejected(){
        for bytes in [wav(1,16,1,&[],false),wav(6,16,1,&[0,0],false),wav(1,16,2,&[0,0],false),wav(1,32,1,&[1,0,0,0],true)] {
            let mut bytes=bytes;if u16le(&bytes[20..22])==0xfffe {bytes[38..40].copy_from_slice(&24_u16.to_le_bytes());}
            assert!(ImpulseResponse::from_wav(&bytes).is_err());
        }
        let mut bytes=wav(3,32,2,&[0;8],true);bytes[40..44].copy_from_slice(&1_u32.to_le_bytes());assert!(ImpulseResponse::from_wav(&bytes).is_err());
    }
}
