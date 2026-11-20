use super::*;
pub use crate::dsp::PCM_RATES;
#[derive(Debug,Clone,PartialEq,Serialize,Deserialize)]
#[serde(default)]
pub struct OutputLimits {
    pub max_sample_rate:Option<u32>,pub max_bit_depth:Option<u16>,pub max_dsd_rate:Option<u32>,
    pub channels:Option<u16>,pub resync_delay_ms:u32,pub volume_control:VolumeControl,pub volume_max:f64,pub conversion_location:ConversionLocation,
}
#[derive(Debug,Clone,Copy,Default,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="snake_case")]
pub enum VolumeControl {#[default] Software,Fixed,Device}
#[derive(Debug,Clone,Copy,Default,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="snake_case")]
pub enum ConversionLocation {#[default] Device,Core}
impl Default for OutputLimits {fn default()->Self{Self{max_sample_rate:None,max_bit_depth:None,max_dsd_rate:None,channels:None,resync_delay_ms:0,volume_control:VolumeControl::Software,volume_max:1.,conversion_location:ConversionLocation::Device}}}
impl OutputLimits {
    pub fn validate(&self)->Result<()> {
        if self.max_sample_rate.is_some_and(|v|!PCM_RATES.contains(&v))||self.max_bit_depth.is_some_and(|v|![16,24,32].contains(&v))||self.max_dsd_rate.is_some_and(|v|![2822400,3072000,5644800,6144000,11289600,12288000,22579200,24576000].contains(&v))||self.channels.is_some_and(|v|![1,2].contains(&v))||self.resync_delay_ms>5000||!self.volume_max.is_finite()||!(0. ..=1.).contains(&self.volume_max){bail!("输出上限、声道、延迟或音量限制无效");}
        if self.volume_control!=VolumeControl::Software&&self.volume_max!=1.{bail!("固定音量模式的音量上限须为 100%");}Ok(())
    }
    pub fn active(&self)->bool {self!=&Self::default()}
    pub fn rates(&self,source:u32)->Vec<u32> {
        let ceiling=self.max_sample_rate.unwrap_or(768000).min(source);
        if source<=ceiling{return vec![source];}
        let mut rates:Vec<_>=PCM_RATES.iter().copied().filter(|r|*r<=ceiling).rev().collect();
        let family=if source%44100==0{44100}else{48000};rates.sort_by_key(|r|(r%family!=0,std::cmp::Reverse(*r)));rates
    }
    pub fn rate(&self,source:u32)->Result<u32>{self.rates(source).first().copied().ok_or_else(||anyhow::anyhow!("没有符合所设上限的采样率"))}
    pub fn bits(&self,source:u16)->u16 {self.max_bit_depth.unwrap_or(if source==0{32}else{source.max(16).min(32)})}
    pub fn volume(&self,value:f64)->f64 {if self.volume_control!=VolumeControl::Software{1.}else{value.min(self.volume_max)}}
    pub fn signal_path(&self,request:&PlaybackRequest,path:&mut SignalPath){
        if request.is_dsd(){return;}
        if let Some(bits)=self.max_bit_depth{if path.bit_depth>bits{path.bit_depth=bits;path.processing.push(format!("有效位深限制 {} bit",bits));}if request.source_bit_depth>bits{path.bit_perfect_eligible=false;}}
    }
    pub fn check_dsd(&self,request:&PlaybackRequest,mode:DsdOutput)->Result<()> {
        if request.is_dsd(){let rate=request.sample_rate.saturating_mul(if mode==DsdOutput::Native{8}else{16});
            if self.max_dsd_rate.is_some_and(|max|rate>max){bail!("DSD {} Hz 超过此输出的 DSD 上限；原样模式不会静默转换",rate);}
            if self.channels.is_some_and(|c|c!=request.channels)||self.max_bit_depth.is_some_and(|b|b<24&&mode==DsdOutput::Dop){bail!("DSD 原样输出不支持所选声道／位深转换");}}
        Ok(())
    }
}
impl OutputConfig {
    pub fn desired_format(&self,request:&PlaybackRequest)->Result<SequenceFormat>{
        self.limits.validate()?;self.limits.check_dsd(request,self.dsd_output)?;
        if request.is_dsd(){return Ok(SequenceFormat{sample_rate:request.sample_rate,channels:request.channels,channel_mask:0,exclusive:self.exclusive,float_output:false,bit_depth:if self.dsd_output==DsdOutput::Native{1}else{32}});}
        let rate=self.limits.rate(request.sample_rate)?;let channels=self.limits.channels.unwrap_or(if cfg!(target_os="android"){request.channels.min(2)}else{request.channels});
        if self.android_mode==Some(AndroidMode::UsbBitperfect)&&(rate!=request.sample_rate||channels!=request.channels||self.limits.max_bit_depth.is_some_and(|b|b<request.source_bit_depth)){bail!("USB 原样模式不允许采样率、位深或声道转换；请调整上限或明确选择系统输出");}
        Ok(SequenceFormat{sample_rate:rate,channels,channel_mask:0,exclusive:self.exclusive,float_output:request.dsp.enabled&&self.limits.max_bit_depth.is_none(),bit_depth:self.limits.max_bit_depth.unwrap_or(32)})
    }
    pub(super) fn prepare(&self,request:&PlaybackRequest,control:&PlaybackControl,sequence:&PlaybackSequence)->Result<()> {
        self.limits.validate()?;self.limits.check_dsd(request,self.dsd_output)?;self.desired_format(request)?;
        control.configure_limits(&self.limits)?;sequence.output_limits(self.limits.clone())?;Ok(())
    }
    pub(super) fn resync(&self,control:&PlaybackControl)->Result<()> {
        let end=std::time::Instant::now()+std::time::Duration::from_millis(self.limits.resync_delay_ms as u64);
        while std::time::Instant::now()<end{if control.stopped.load(Ordering::Acquire){bail!("输出等待已取消");}std::thread::sleep(std::time::Duration::from_millis(5));}Ok(())
    }
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn closure_limits_keep_rate_families_and_reject_raw_conversion(){
        let request=PlaybackRequest{flac_samples: None, dsd_pcm: None, input:"never-opened".into(),position:0.,sample_rate:768000,channels:2,source_integer_lossless:true,source_bit_depth:32,dsp:Default::default(),impulse:None,measured_lufs:None,volume:1.,duration:Some(1.),transitions:Default::default()};
        let mut config=OutputConfig::default();config.limits.max_sample_rate=Some(192000);config.limits.max_bit_depth=Some(24);
        assert_eq!(config.desired_format(&request).unwrap().sample_rate,192000);
        assert_eq!(config.desired_format(&PlaybackRequest{sample_rate:44100,..request.clone()}).unwrap().sample_rate,44100);
        assert_eq!(config.limits.rate(705600).unwrap(),176400);
        config.android_mode=Some(AndroidMode::UsbBitperfect);assert!(config.desired_format(&request).unwrap_err().to_string().contains("原样"));
        config.limits.max_dsd_rate=Some(2822400);assert!(config.limits.check_dsd(&PlaybackRequest{source_bit_depth:1,source_integer_lossless:false,sample_rate:352800,..request.clone()},DsdOutput::Dop).is_err());
        config.limits.volume_max=0.6;let control=PlaybackControl::default();control.configure_limits(&config.limits).unwrap();control.set_volume(0.9).unwrap();assert_eq!(control.volume_setting(),0.6);
        config.limits.volume_control=VolumeControl::Fixed;assert!(config.limits.validate().is_err());config.limits.volume_max=1.;control.configure_limits(&config.limits).unwrap();control.set_volume(0.1).unwrap();assert_eq!(control.volume_setting(),1.);
    }
}
