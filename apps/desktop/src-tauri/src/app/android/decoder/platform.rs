//! Platform codecs consume the same validated source; callback errors cannot become EOF.
use super::*;
use jni::{JNIEnv,objects::{JByteArray,JClass,JString},sys::{jboolean,jint,jlong}};
use std::{collections::HashMap,sync::OnceLock};
struct Position {input:Box<dyn MediaSource>,offset:Option<u64>}
enum PlatformInput {Bytes(Position),Packets{format:Box<dyn FormatReader>,track:u32}}
struct Entry {source:Mutex<PlatformInput>,cancel:Arc<Cancel>,error:Mutex<Option<String>>}
static SOURCES:OnceLock<Mutex<HashMap<String,Arc<Entry>>>>=OnceLock::new();
fn sources()->&'static Mutex<HashMap<String,Arc<Entry>>> {SOURCES.get_or_init(Default::default)}
fn source(id:&str)->Result<Arc<Entry>> {sources().lock().map_err(|_|anyhow::anyhow!("decoder source registry unavailable"))?.get(id).cloned().context("decoder source closed")}
#[cfg(debug_assertions)] pub(super) fn active()->usize {sources().lock().map(|sources|sources.len()).unwrap_or(usize::MAX)}
pub(super) struct Decoder {id:String,entry:Arc<Entry>,processing:Vec<String>}
impl Decoder {
    pub(super) fn open(input:Box<dyn MediaSource>,cancel:Arc<Cancel>,rate:u32,channels:u16,skip:u64,duration:Option<f64>)->Result<Self> {
        let id=uuid::Uuid::new_v4().to_string();let entry=Arc::new(Entry{source:Mutex::new(PlatformInput::Bytes(Position{input,offset:None})),cancel,error:Mutex::new(None)});
        sources().lock().map_err(|_|anyhow::anyhow!("decoder source registry unavailable"))?.insert(id.clone(),entry.clone());
        let decoder=Self{id,entry,processing:Vec::new()};
        let result=super::super::jni_controls::decoder("open",json!({"id":decoder.id,"rate":rate,"channels":channels,"skip":skip,"duration":duration}),&mut[]);
        decoder.check()?;result?;Ok(decoder)
    }
    pub(super) fn open_opus(format:Box<dyn FormatReader>,cancel:Arc<Cancel>,rate:u32,channels:u16,skip:u64,duration:Option<f64>)->Result<Self> {
        let track=format.default_track().context("Missing Opus track")?;
        let header=track.codec_params.extra_data.as_ref().context("Missing Opus header")?.to_vec();
        if rate!=48000||header.len()<19||header.len()>255||!header.starts_with(b"OpusHead") {bail!("Invalid streaming Opus configuration");}
        let track=track.id;let id=uuid::Uuid::new_v4().to_string();
        let entry=Arc::new(Entry{source:Mutex::new(PlatformInput::Packets{format,track}),cancel,error:Mutex::new(None)});
        sources().lock().map_err(|_|anyhow::anyhow!("decoder source registry unavailable"))?.insert(id.clone(),entry.clone());
        let decoder=Self{id,entry,processing:Vec::new()};
        let result=super::super::jni_controls::decoder("open",json!({"id":decoder.id,"rate":rate,"channels":channels,"skip":skip,"duration":duration,"opus_header":header}),&mut[]);
        decoder.check()?;result?;Ok(decoder)
    }
    fn check(&self)->Result<()> {
        if let Some(error)=self.entry.error.lock().map_err(|_|anyhow::anyhow!("decoder source error lock unavailable"))?.as_ref(){bail!("媒体读取失败：{error}");}Ok(())
    }
    pub(super) fn read(&mut self,output:&mut[u8],frame_bytes:usize)->Result<usize> {
        let result=super::super::jni_controls::decoder("read",json!({"id":self.id}),output);self.check()?;
        let reply=result?;
        if let Some(processing)=reply["processing"].as_str(){self.processing=vec![processing.to_owned()];}
        let bytes=reply["bytes"].as_u64().context("Missing decoded byte count")? as usize;
        if bytes%frame_bytes!=0 {bail!("Android decoder returned a partial PCM frame");}Ok(bytes/frame_bytes)
    }
    pub(super) fn processing(&self)->Vec<String> {self.processing.clone()}
}
impl Drop for Decoder {fn drop(&mut self){let _=super::super::jni_controls::decoder("close",json!({"id":self.id}),&mut[]);if let Ok(mut sources)=sources().lock(){sources.remove(&self.id);}}}
#[no_mangle]
pub extern "system" fn Java_com_rmusic_android_development_NativeDecoder_cancelled(mut env:JNIEnv<'_>,_:JClass<'_>,id:JString<'_>)->jboolean {
    let id=env.get_string(&id).map(|id|String::from(id));id.ok().and_then(|id|source(&id).ok()).is_none_or(|entry|entry.cancel.stopped.load(Ordering::Acquire)) as jboolean
}
#[no_mangle]
pub extern "system" fn Java_com_rmusic_android_development_NativeDecoder_size(mut env:JNIEnv<'_>,_:JClass<'_>,id:JString<'_>)->jlong {
    let result=(||->Result<i64>{let id:String=env.get_string(&id)?.into();let entry=source(&id)?;let input=entry.source.lock().map_err(|_|anyhow::anyhow!("decoder source lock unavailable"))?;Ok(match &*input {PlatformInput::Bytes(input)=>input.input.byte_len().and_then(|n|n.try_into().ok()).unwrap_or(-1),PlatformInput::Packets{..}=>-1})})();
    match result {Ok(size)=>size,Err(error)=>{let _=env.throw_new("java/io/IOException",error.to_string());-1}}
}
#[no_mangle]
pub extern "system" fn Java_com_rmusic_android_development_NativeDecoder_readAt(mut env:JNIEnv<'_>,_:JClass<'_>,id:JString<'_>,position:jlong,output:JByteArray<'_>,offset:jint,size:jint)->jint {
    let result=(||->Result<i32>{
        let id:String=env.get_string(&id)?.into();let entry=source(&id)?;
        let read=(||->Result<_>{
            let length=env.get_array_length(&output)?;
            if position<0||offset<0||size<0||offset as i64+size as i64>length as i64 {bail!("invalid decoder source read");}
            if entry.cancel.stopped.load(Ordering::Acquire){bail!("decoder cancelled");}
            if size==0{return Ok(0);}
            let mut input=entry.source.lock().map_err(|_|anyhow::anyhow!("decoder source lock unavailable"))?;
            let PlatformInput::Bytes(input)=&mut *input else {bail!("Packet source does not expose byte seeking");};
            if input.input.byte_len().is_some_and(|length|position as u64>=length){return Ok(-1);}
            // MediaSourceStream's stream_position() invalidates its read-ahead cache.
            if input.offset!=Some(position as u64) {input.input.seek(SeekFrom::Start(position as u64))?;input.offset=Some(position as u64);}
            let mut bytes=vec![0u8;(size as usize).min(64*1024)];let count=input.input.read(&mut bytes)?;input.offset=Some(position as u64+count as u64);
            if count==0{return Ok(-1);}
            let bytes:Vec<i8>=bytes[..count].iter().map(|n|*n as i8).collect();env.set_byte_array_region(&output,offset,&bytes)?;Ok(count as i32)
        })();
        if let Err(error)=&read {if let Ok(mut saved)=entry.error.lock(){*saved=Some(error.to_string());}}
        read
    })();
    match result {Ok(count)=>count,Err(error)=>{let _=env.throw_new("java/io/IOException",error.to_string());-1}}
}

// Feed the existing Ogg packet reader directly into MediaCodec. MediaExtractor can scan and
// rewind a live Ogg response beyond any bounded byte replay window.
#[no_mangle]
pub extern "system" fn Java_com_rmusic_android_development_NativeDecoder_nextPacket(mut env:JNIEnv<'_>,_:JClass<'_>,id:JString<'_>,output:JByteArray<'_>)->jni::sys::jstring {
    let result=(||->Result<_>{
        let id:String=env.get_string(&id)?.into();let entry=source(&id)?;
        let result=(||->Result<_>{
            if entry.cancel.stopped.load(Ordering::Acquire){bail!("decoder cancelled");}
            let mut source=entry.source.lock().map_err(|_|anyhow::anyhow!("decoder source lock unavailable"))?;
            let PlatformInput::Packets{format,track}=&mut *source else {bail!("Not an Opus packet source");};
            loop {
                let packet=match format.next_packet(){Ok(packet)=>packet,Err(DecodeError::IoError(error)) if error.kind()==io::ErrorKind::UnexpectedEof=>return Ok(json!({"bytes":-1})),Err(error)=>return Err(error.into())};
                if packet.track_id()!=*track {continue;}
                if packet.data.len()>env.get_array_length(&output)? as usize {bail!("Opus packet exceeds codec input budget");}
                let data:Vec<i8>=packet.data.iter().map(|byte|*byte as i8).collect();env.set_byte_array_region(&output,0,&data)?;
                // Ogg gapless timestamps already exclude the pre-skip applied by MediaCodec.
                let pts=packet.ts.checked_mul(1_000_000).context("Opus timestamp overflow")?/48000;
                return Ok(json!({"bytes":data.len(),"pts":pts}));
            }
        })();
        if let Err(error)=&result {if let Ok(mut saved)=entry.error.lock(){*saved=Some(error.to_string());}}
        Ok(env.new_string(serde_json::to_string(&result?)?)?.into_raw())
    })();
    match result {Ok(value)=>value,Err(error)=>{let _=env.throw_new("java/io/IOException",error.to_string());std::ptr::null_mut()}}
}
