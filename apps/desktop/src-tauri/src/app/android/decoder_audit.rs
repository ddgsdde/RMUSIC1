//! Debug APK instrumentation entry points; excluded from release builds and frontend commands.
use super::*;
use jni::{JNIEnv,objects::{JClass,JString},sys::{jboolean,jbyteArray,jlong}};
#[derive(serde::Deserialize)]
struct Input {input:String,sample_rate:u32,channels:u16,bits:u16,#[serde(default="lossless_default")] lossless:bool,#[serde(default)] position:f64,#[serde(default)] dsp:rmusic_audio::DspConfig,#[serde(default)] measured_lufs:Option<f64>,#[serde(default)] usb_bitperfect:bool,#[serde(default)] duration:Option<f64>,#[serde(default)] transitions:rmusic_audio::fade::FadeConfig}
fn lossless_default()->bool {true}
struct DecoderOnly;
impl output::android::Backend for DecoderOnly {
    fn devices(&self)->Result<Vec<DeviceInfo>> {bail!("decoder instrumentation has no output")}
    fn probe(&self,_:&Path)->Result<AudioMetadata> {bail!("decoder instrumentation has no probe")}
    fn play(&self,_:PlaybackRequest,_:OutputConfig,_:Arc<PlaybackControl>,_:&mut dyn FnMut(SignalPath,Meter))->Result<()> {bail!("decoder instrumentation has no output")}
    fn decoder(&self,request:&PlaybackRequest,format:output::SequenceFormat)->Result<Box<dyn output::android::PcmDecoder>> {super::decoder::open(request,format)}
}
#[no_mangle]
pub extern "system" fn Java_com_rmusic_android_development_DecoderDiagnostics_install(mut env:JNIEnv<'_>,_:JClass<'_>) {
    if !output::native_available(){if let Err(error)=output::android::install(Arc::new(DecoderOnly)){let _=env.throw_new("java/lang/IllegalStateException",error.to_string());}}
}
fn request(input:Input)->PlaybackRequest {
    PlaybackRequest{flac_samples: None, dsd_pcm: None, input:input.input,position:input.position,sample_rate:input.sample_rate,channels:input.channels,
        source_bit_depth:input.bits,source_integer_lossless:input.lossless,dsp:input.dsp,impulse:None,measured_lufs:input.measured_lufs,
        volume:1.,duration:input.duration,transitions:input.transitions}
}
#[no_mangle]
pub extern "system" fn Java_com_rmusic_android_development_DecoderDiagnostics_ready(_:JNIEnv<'_>,_:JClass<'_>)->jboolean {output::native_available() as jboolean}
#[no_mangle]
pub extern "system" fn Java_com_rmusic_android_development_DecoderDiagnostics_decodeFloat(mut env:JNIEnv<'_>,_:JClass<'_>,path:JString<'_>)->jbyteArray {
    let result=(||->Result<_> {
        let input:String=env.get_string(&path)?.into();let rate=768000;
        let request=PlaybackRequest{flac_samples: None, dsd_pcm: None, input,position:7./rate as f64,sample_rate:rate,channels:2,source_integer_lossless:true,source_bit_depth:32,dsp:Default::default(),impulse:None,measured_lufs:None,volume:1.,duration:None,transitions:Default::default()};
        let format=output::SequenceFormat{sample_rate:rate,channels:2,channel_mask:0,exclusive:false,float_output:true,bit_depth:64};
        let mut decoder=super::decoder::open(&request,format)?;let mut bytes=Vec::new();let mut block=vec![0;512*16];
        loop {let count=decoder.read_frames(&mut block,16)?;if count==0{break;}if bytes.len()+count*16>1024*1024{bail!("float audit budget exceeded");}bytes.extend_from_slice(&block[..count*16]);}
        Ok(env.byte_array_from_slice(&bytes)?.into_raw())
    })();match result{Ok(bytes)=>bytes,Err(error)=>{let _=env.throw_new("java/lang/IllegalStateException",error.to_string());std::ptr::null_mut()}}
}
#[no_mangle]
pub extern "system" fn Java_com_rmusic_android_development_DecoderDiagnostics_httpSeek(mut env:JNIEnv<'_>,_:JClass<'_>,input:JString<'_>)->jbyteArray {
    let result=(||->Result<_>{let input:String=env.get_string(&input)?.into();Ok(env.byte_array_from_slice(&super::decoder::audit::seek(&input)?)?.into_raw())})();
    match result {Ok(bytes)=>bytes,Err(error)=>{let _=env.throw_new("java/lang/IllegalStateException",error.to_string());std::ptr::null_mut()}}
}
#[no_mangle]
pub extern "system" fn Java_com_rmusic_android_development_DecoderDiagnostics_cancelHttpSeek(mut env:JNIEnv<'_>,_:JClass<'_>,body:jboolean)->jlong {
    match super::decoder::audit::cancel_seek(body!=0){Ok(elapsed)=>elapsed as jlong,Err(error)=>{let _=env.throw_new("java/lang/IllegalStateException",error.to_string());-1}}
}
#[no_mangle]
pub extern "system" fn Java_com_rmusic_android_development_DecoderDiagnostics_activeDecoderCount(_:JNIEnv<'_>,_:JClass<'_>)->jni::sys::jint {super::decoder::active_count() as i32}
#[no_mangle]
pub extern "system" fn Java_com_rmusic_android_development_DecoderDiagnostics_activePlatformSources(_:JNIEnv<'_>,_:JClass<'_>)->jni::sys::jint {super::decoder::active_platform_sources() as i32}
#[no_mangle]
pub extern "system" fn Java_com_rmusic_android_development_DecoderDiagnostics_cancelPlatform(mut env:JNIEnv<'_>,_:JClass<'_>,path:JString<'_>,body:jboolean)->jlong {
    let result=(||->Result<_>{let path:String=env.get_string(&path)?.into();super::decoder::audit::cancel_platform(&path,body!=0)})();
    match result {Ok(elapsed)=>elapsed as jlong,Err(error)=>{let _=env.throw_new("java/lang/IllegalStateException",error.to_string());-1}}
}
#[no_mangle]
pub extern "system" fn Java_com_rmusic_android_development_DecoderDiagnostics_probe(mut env:JNIEnv<'_>,_:JClass<'_>,input:JString<'_>)->jni::sys::jstring {
    let result=(||->Result<_> {
        let input:String=env.get_string(&input)?.into();
        let metadata=super::decoder::probe_native(Path::new(&input))?.context("unsupported native probe")?;
        Ok(env.new_string(serde_json::to_string(&metadata)?)?.into_raw())
    })();
    match result {Ok(text)=>text,Err(error)=>{let _=env.throw_new("java/lang/IllegalStateException",error.to_string());std::ptr::null_mut()}}
}
#[no_mangle]
pub extern "system" fn Java_com_rmusic_android_development_DecoderDiagnostics_decode(mut env:JNIEnv<'_>,_:JClass<'_>,inputs:JString<'_>)->jbyteArray {
    let result=(||->Result<_> {
        let text:String=env.get_string(&inputs)?.into();let inputs:Vec<Input>=serde_json::from_str(&text)?;
        if inputs.is_empty()||inputs.len()>4 {bail!("audit requires 1..4 sources");}
        let exclusive=inputs[0].usb_bitperfect;
        let bytes=output::android::audit_queue(inputs.into_iter().map(request).collect(),Arc::new(PlaybackControl::default()),exclusive)?;
        Ok(env.byte_array_from_slice(&bytes)?.into_raw())
    })();
    match result {Ok(bytes)=>bytes,Err(error)=>{let _=env.throw_new("java/lang/IllegalStateException",error.to_string());std::ptr::null_mut()}}
}
#[no_mangle]
pub extern "system" fn Java_com_rmusic_android_development_DecoderDiagnostics_cancelSlow(mut env:JNIEnv<'_>,_:JClass<'_>)->jlong {
    let result=(||->Result<u128> {
        let server=std::net::TcpListener::bind("127.0.0.1:0")?;let address=server.local_addr()?;
        let (accepted,connection)=std::sync::mpsc::channel();let(release,wait)=std::sync::mpsc::channel();
        server.set_nonblocking(true)?;
        let server_thread=std::thread::spawn(move|| {
            let deadline=std::time::Instant::now()+Duration::from_secs(6);
            while std::time::Instant::now()<deadline {
                if let Ok((socket,_))=server.accept(){let _=accepted.send(());let _=wait.recv_timeout(Duration::from_secs(6));drop(socket);return;}
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        let control=Arc::new(PlaybackControl::default());let worker=control.clone();
        let task=std::thread::spawn(move||output::android::audit_queue(vec![request(Input {input:format!("http://{address}/blocked.wav"),sample_rate:96000,channels:2,bits:16,lossless:true,position:0.,dsp:Default::default(),measured_lufs:None,usb_bitperfect:false,duration:None,transitions:Default::default()})],worker,false));
        let connected=connection.recv_timeout(Duration::from_secs(5));
        if connected.is_ok(){std::thread::sleep(Duration::from_millis(100));}
        let started=std::time::Instant::now();control.stop();let stopped=task.join();let elapsed=started.elapsed().as_millis();
        let _=release.send(());let _=server_thread.join();connected.context("decoder did not open its source")?;
        let stopped=stopped.map_err(|_|anyhow::anyhow!("decoder worker panicked"))?;
        if stopped.is_ok_and(|bytes|!bytes.is_empty()) {bail!("cancelled source produced programme PCM");}
        Ok(elapsed)
    })();
    match result {Ok(elapsed)=>elapsed as jlong,Err(error)=>{let _=env.throw_new("java/lang/IllegalStateException",error.to_string());-1}}
}
