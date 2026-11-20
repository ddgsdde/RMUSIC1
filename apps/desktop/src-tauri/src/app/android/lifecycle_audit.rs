//! Debug instrumentation of the production single-track backend without Core or user data.
use super::*;
use jni::{JNIEnv,objects::{JClass,JString},sys::jlong};
type Worker=(Arc<PlaybackControl>,std::thread::JoinHandle<Result<()>>);
static PLAYBACK:Mutex<Option<Worker>>=Mutex::new(None);
#[no_mangle]
pub extern "system" fn Java_com_rmusic_android_development_LifecycleDiagnostics_start(mut env:JNIEnv<'_>,_:JClass<'_>,input:JString<'_>) {
    let result=(||->Result<()> {
        let input:String=env.get_string(&input)?.into();let mut saved=PLAYBACK.lock().map_err(|_|anyhow::anyhow!("audit lock unavailable"))?;
        if saved.is_some(){bail!("audit output already active");}
        let control=Arc::new(PlaybackControl::default());let worker=control.clone();
        let request=PlaybackRequest{flac_samples: None, dsd_pcm: None, input,position:0.,sample_rate:96000,channels:2,source_bit_depth:16,source_integer_lossless:true,
            dsp:Default::default(),impulse:None,measured_lufs:None,volume:1.,duration:Some(30.),transitions:Default::default()};
        *saved=Some((control,std::thread::spawn(move||output::play(request,Default::default(),worker,|_,_|{}))));Ok(())
    })();
    if let Err(error)=result {let _=env.throw_new("java/lang/IllegalStateException",error.to_string());}
}
#[no_mangle]
pub extern "system" fn Java_com_rmusic_android_development_LifecycleDiagnostics_frames(mut env:JNIEnv<'_>,_:JClass<'_>)->jlong {
    let result=(||->Result<_> {
        let saved=PLAYBACK.lock().map_err(|_|anyhow::anyhow!("audit lock unavailable"))?;
        let (control,task)=saved.as_ref().context("audit output unavailable")?;
        if task.is_finished(){bail!("audit output exited before stop");}
        Ok(control.frames_played.load(Ordering::Acquire) as jlong)
    })();
    match result {Ok(frames)=>frames,Err(error)=>{let _=env.throw_new("java/lang/IllegalStateException",error.to_string());-1}}
}
#[no_mangle]
pub extern "system" fn Java_com_rmusic_android_development_LifecycleDiagnostics_stop(mut env:JNIEnv<'_>,_:JClass<'_>) {
    let result=(||->Result<()> {
        let saved=PLAYBACK.lock().map_err(|_|anyhow::anyhow!("audit lock unavailable"))?.take();
        if let Some((control,task))=saved {control.stop();task.join().map_err(|_|anyhow::anyhow!("audit worker panicked"))??;}Ok(())
    })();
    if let Err(error)=result {let _=env.throw_new("java/lang/IllegalStateException",error.to_string());}
}
