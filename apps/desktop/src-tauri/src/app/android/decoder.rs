//! Native queue decoding, independent of Android's precision-limiting extractors.
use super::*;
use std::{io::{self, Read, Seek, SeekFrom}, sync::atomic::{AtomicBool, AtomicUsize}};
use rmusic_audio::{decoder::DecoderAbort, output::{SequenceFormat, android::PcmDecoder}};
use symphonia::core::{audio::SampleBuffer, codecs::{CodecParameters, DecoderOptions, CODEC_TYPE_FLAC, CODEC_TYPE_ALAC, CODEC_TYPE_MP3, CODEC_TYPE_VORBIS, CODEC_TYPE_AAC, CODEC_TYPE_OPUS},
    errors::Error as DecodeError, formats::{FormatOptions, FormatReader}, io::{MediaSource, MediaSourceStream},
    meta::MetadataOptions, probe::Hint};

static ACTIVE: AtomicUsize = AtomicUsize::new(0);
mod http;
mod platform;
mod convert;
mod vorbis;
#[cfg(debug_assertions)] pub(super) mod audit;
#[cfg(debug_assertions)] pub(super) fn active_count()->usize {ACTIVE.load(Ordering::Acquire)}
#[cfg(debug_assertions)] pub(super) fn active_platform_sources()->usize {platform::active()}
struct Cancel {stopped:AtomicBool, task:Mutex<Option<tauri::async_runtime::JoinHandle<()>>>}
impl Cancel {
    fn stop(&self) {self.stopped.store(true,Ordering::Release);if let Ok(task)=self.task.lock(){if let Some(task)=task.as_ref(){task.abort();}}}
}
enum Source {File(std::fs::File), Http(http::Http)}
struct Input {source:Source,cancel:Arc<Cancel>}
impl Input {
    fn open(input:&str,cancel:Arc<Cancel>)->Result<Self> {
        let source=if input.starts_with("http:")||input.starts_with("https:") {
            let url=reqwest::Url::parse(input)?;
            if url.scheme()!="http"||!matches!(url.host_str(),Some("127.0.0.1"|"[::1]"|"::1")) {bail!("Android 无损预载仅接受本机媒体代理");}
            Source::Http(http::Http::new(url,cancel.clone())?)
        }else {Source::File(rmusic_audio::file::open(input)?)};
        Ok(Self{source,cancel})
    }
    fn prepare(&mut self)->Result<()> {if let Source::Http(source)=&mut self.source{source.prepare()?;}Ok(())}
}
impl Read for Input {
    fn read(&mut self,output:&mut[u8])->io::Result<usize> {
        if self.cancel.stopped.load(Ordering::Acquire){return Err(io::Error::other("decoder cancelled"));}
        match &mut self.source {
            Source::File(file)=>file.read(output),
            Source::Http(source)=>source.read(output),
        }
    }
}
impl Seek for Input {
    fn seek(&mut self,from:SeekFrom)->io::Result<u64> {match &mut self.source {Source::File(file)=>file.seek(from),Source::Http(source)=>source.seek(from)}}
}
impl MediaSource for Input {
    fn is_seekable(&self)->bool {match &self.source{Source::File(_)=>true,Source::Http(source)=>source.seekable()}}
    fn byte_len(&self)->Option<u64> {match &self.source {Source::File(file)=>file.metadata().ok().map(|m|m.len()),Source::Http(source)=>source.length()}}
}
impl Drop for Input {fn drop(&mut self){self.cancel.stop();}}
fn demux(mut input:Input)->Result<Box<dyn FormatReader>> {
    input.prepare()?;
    Ok(symphonia::default::get_probe().format(&Hint::new(),MediaSourceStream::new(Box::new(input),Default::default()),
        &FormatOptions{enable_gapless:true,..Default::default()},&MetadataOptions::default())?.format)
}
fn integer(params:&CodecParameters)->bool {
    use symphonia::core::codecs::{CODEC_TYPE_PCM_U8,CODEC_TYPE_PCM_S16LE,CODEC_TYPE_PCM_S24LE,CODEC_TYPE_PCM_S32LE};
    matches!(params.codec,CODEC_TYPE_FLAC|CODEC_TYPE_ALAC|CODEC_TYPE_PCM_U8|CODEC_TYPE_PCM_S16LE|CODEC_TYPE_PCM_S24LE|CODEC_TYPE_PCM_S32LE)
}
fn cancellation()->Arc<Cancel> {Arc::new(Cancel{stopped:AtomicBool::new(false),task:Mutex::new(None)})}
fn codec(params:&CodecParameters)->Result<(CodecParameters,Box<dyn symphonia::core::codecs::Decoder>)> {
    let decoder=symphonia::default::get_codecs().make(params,&DecoderOptions{verify:true})?;
    let mut params=params.clone();
    if params.codec==CODEC_TYPE_ALAC {
        // The MP4 reader leaves these unset; the ALAC decoder validates its 24/48-byte cookie.
        let spec=*decoder.last_decoded().spec();
        params.sample_rate=Some(spec.rate);params.channels=Some(spec.channels);
        params.bits_per_sample=Some(params.extra_data.as_ref().context("Missing ALAC configuration")?[5] as u32);
    }
    Ok((params,decoder))
}
pub(super) fn probe_native(path:&Path)->Result<Option<AudioMetadata>> {
    let input=Input::open(path.to_str().context("Invalid audio path")?,cancellation())?;
    let format=match demux(input) {Ok(format)=>format,Err(error)=> {
        if matches!(error.downcast_ref::<DecodeError>(),Some(DecodeError::Unsupported(_))){return Ok(None);}return Err(error);
    }};
    let params=&format.default_track().context("Missing audio track")?.codec_params;
    if matches!(params.codec,CODEC_TYPE_AAC|CODEC_TYPE_OPUS){return Ok(None);}
    if symphonia::default::get_codecs().get_codec(params.codec).is_none(){return Ok(None);}
    let (params,_)=codec(params)?;
    let rate=params.sample_rate.context("Missing sample rate")?;
    let channels=params.channels.context("Missing channel layout")?.count() as u16;
    let bits=if integer(&params){params.bits_per_sample.unwrap_or(0) as u16}else{0};
    let codec=match params.codec {CODEC_TYPE_FLAC=>"flac",CODEC_TYPE_ALAC=>"alac",CODEC_TYPE_MP3=>"mp3",CODEC_TYPE_VORBIS=>"ogg",_=>"wav"};
    let duration=params.n_frames.map(|frames|params.time_base.map(|base|{let t=base.calc_time(frames);t.seconds as f64+t.frac}).unwrap_or(frames as f64/rate as f64));
    Ok(Some(AudioMetadata {flac_samples:None,duration,format:codec.into(),
        sample_rate:rate,channels,bit_depth:bits,bitrate:0,quality_rank:rmusic_audio::probe::quality_rank(codec,rate,bits,0),
        source_integer_lossless:integer(&params)&&bits>0}))
}
struct Stream {format:Box<dyn FormatReader>,codec:Box<dyn symphonia::core::codecs::Decoder>,track:u32,label:String,bits:u32,processing:Vec<String>}
struct Decoder {layout:Option<u32>,input:Option<Input>,stream:Option<Stream>,platform:Option<platform::Decoder>,vorbis:Option<vorbis::Decoder>,cancel:Arc<Cancel>,rate:u32,channels:u16,bits:u16,lossless:bool,skip:u64,duration:Option<f64>,pending:Vec<u8>,cursor:usize}
pub(super) fn open(request:&PlaybackRequest,format:SequenceFormat)->Result<Box<dyn PcmDecoder>> {
    let inner=if request.sample_rate!=format.sample_rate||request.channels!=format.channels{
        if format.exclusive{bail!("USB 原样输出不允许采样率或声道转换");}
        let source=SequenceFormat{sample_rate:request.sample_rate,channels:request.channels,channel_mask:0,exclusive:false,float_output:false,bit_depth:32};
        let inner=raw_open(request,source)?;Box::new(convert::Converted::new(inner,request,format)?) as Box<dyn PcmDecoder>
    }else{raw_open(request,format)?};
    if format.float_output&&format.bit_depth==64{Ok(Box::new(convert::Float{inner,channels:format.channels as usize}))}else{Ok(inner)}
}
fn raw_open(request:&PlaybackRequest,format:SequenceFormat)->Result<Box<dyn PcmDecoder>> {
    let cancel=cancellation();let input=Input::open(&request.input,cancel.clone())?;
    ACTIVE.fetch_add(1,Ordering::AcqRel);
    Ok(Box::new(Decoder {layout:None,input:Some(input),stream:None,platform:None,vorbis:None,cancel,rate:format.sample_rate,channels:format.channels,bits:request.source_bit_depth,lossless:request.source_integer_lossless,
        skip:(request.position*format.sample_rate as f64).round() as u64,duration:request.duration,pending:Vec::new(),cursor:0}))
}
impl Decoder {
    fn initialize(&mut self)->Result<()> {
        let mut input=self.input.take().context("decoder source unavailable")?;input.prepare()?;
        let packet_opus=!input.is_seekable();
        let format=demux(input)?;
        let track=format.default_track().context("Missing audio track")?;
        self.layout=track.codec_params.channels.map(|channels|channels.bits());
        if matches!(track.codec_params.codec,CODEC_TYPE_AAC|CODEC_TYPE_OPUS) {
            if self.lossless {bail!("AAC／Opus不能标记为整数无损源");}
            self.platform=Some(if track.codec_params.codec==CODEC_TYPE_OPUS&&packet_opus {
                platform::Decoder::open_opus(format,self.cancel.clone(),self.rate,self.channels,self.skip,self.duration)?
            }else{platform::Decoder::open(Box::new(format.into_inner()),self.cancel.clone(),self.rate,self.channels,self.skip,self.duration)?});return Ok(());
        }
        let (params,codec)=codec(&track.codec_params)?;
        self.layout=params.channels.map(|channels|channels.bits());
        if params.sample_rate!=Some(self.rate)||params.channels.map(|c|c.count())!=Some(self.channels as usize) {bail!("源格式变化；不自动重采样");}
        if self.lossless&&(!integer(&params)||params.bits_per_sample.unwrap_or(0)<self.bits as u32) {bail!("无损预载无法保留源整数精度");}
        if params.codec==CODEC_TYPE_VORBIS {
            let id=track.id;self.vorbis=Some(vorbis::Decoder::open(format.into_inner(),codec,id,self.rate,self.channels)?);return Ok(());
        }
        let label=match params.codec{CODEC_TYPE_FLAC=>"FLAC",CODEC_TYPE_ALAC=>"ALAC",CODEC_TYPE_MP3=>"MP3",_=>"WAV"}.into();
        let bits=params.bits_per_sample.unwrap_or(0);let track=track.id;self.stream=Some(Stream{format,codec,track,label,bits,processing:Vec::new()});Ok(())
    }
}
impl PcmDecoder for Decoder {
    fn read_frames(&mut self,output:&mut[u8],frame_bytes:usize)->Result<usize> {
        if frame_bytes!=self.channels as usize*4||output.is_empty()||output.len()%frame_bytes!=0 {bail!("invalid Android signed32 frame buffer");}
        if self.stream.is_none()&&self.platform.is_none()&&self.vorbis.is_none(){self.initialize()?;}
        if let Some(decoder)=&mut self.platform{return decoder.read(output,frame_bytes);}
        while self.cursor==self.pending.len() {
            if self.cancel.stopped.load(Ordering::Acquire){bail!("decoder cancelled");}
            if let Some(decoder)=&mut self.vorbis {
                let Some(bytes)=decoder.read_block()? else{return Ok(0);};
                let frames=(bytes.len()/frame_bytes) as u64;let skip=self.skip.min(frames);self.skip-=skip;
                if skip==frames{continue;}
                self.pending=bytes;self.cursor=skip as usize*frame_bytes;break;
            }
            let stream=self.stream.as_mut().unwrap();
            let packet=match stream.format.next_packet() {
                Ok(packet)=>packet,
                Err(DecodeError::IoError(error)) if error.kind()==io::ErrorKind::UnexpectedEof=>return Ok(0),
                Err(error)=>return Err(error.into()),
            };
            if packet.track_id()!=stream.track {continue;}
            let decoded=stream.codec.decode(&packet)?;
            if decoded.spec().rate!=self.rate||decoded.spec().channels.count()!=self.channels as usize {bail!("源格式变化；不自动重采样");}
            if decoded.capacity()>65536 {bail!("decoded block exceeds bounded preload budget");}
            if stream.processing.is_empty() {
                use symphonia::core::audio::AudioBufferRef;
                let precision=match &decoded{AudioBufferRef::F32(_)=>"float32".into(),AudioBufferRef::F64(_)=>"float64".into(),_=>format!("整数 {} bit",stream.bits)};
                stream.processing=vec![format!("原生 {} 解码：{} → PCM32 容器",stream.label,precision)];
            }
            let frames=decoded.frames() as u64;let skip=self.skip.min(frames);self.skip-=skip;
            if skip==frames {continue;}
            let mut samples=SampleBuffer::<i32>::new(decoded.capacity() as u64,*decoded.spec());samples.copy_interleaved_ref(decoded);
            self.pending.clear();self.cursor=0;
            for sample in &samples.samples()[skip as usize*self.channels as usize..] {self.pending.extend_from_slice(&sample.to_le_bytes());}
        }
        let count=output.len().min(self.pending.len()-self.cursor);
        output[..count].copy_from_slice(&self.pending[self.cursor..self.cursor+count]);self.cursor+=count;Ok(count/frame_bytes)
    }
    fn channel_mask(&self)->Option<u32>{self.layout}
    fn abort_handle(&self)->DecoderAbort {let cancel=self.cancel.clone();DecoderAbort::native(move||cancel.stop())}
    fn processing(&self)->Vec<String> {
        if let Some(decoder)=&self.platform{return decoder.processing();}
        if self.vorbis.is_some(){return vec!["原生 Vorbis 解码：float32 → PCM32 容器".into()];}
        self.stream.as_ref().map(|stream|stream.processing.clone()).unwrap_or_default()
    }
}
impl Drop for Decoder {fn drop(&mut self){self.cancel.stop();ACTIVE.fetch_sub(1,Ordering::AcqRel);}}
