//! Debug-only real AudioTrack sequence fixture, with bounded completion records.
use super::*;
use jni::{JNIEnv,objects::{JClass,JString},sys::{jstring,jlong}};
struct Worker {control:Arc<PlaybackControl>,thread:Option<std::thread::JoinHandle<Result<()>>>,report:Arc<Mutex<Value>>,stall:Arc<std::sync::atomic::AtomicU64>}
static WORKER:Mutex<Option<Worker>>=Mutex::new(None);
#[no_mangle]
pub extern "system" fn Java_com_rmusic_android_development_QueueDiagnostics_start(mut env:JNIEnv<'_>,_:JClass<'_>,first:JString<'_>,second:JString<'_>) {
    let result=(||->Result<()> {
        let first:String=env.get_string(&first)?.into();
        let options=if first.starts_with('{'){serde_json::from_str::<Value>(&first)?}else{json!({"input":first,"output":{}})};
        let first=options["input"].as_str().context("missing configured fixture input")?.to_owned();let config:OutputConfig=serde_json::from_value(options["output"].clone())?;
        let second:String=env.get_string(&second)?.into();
        let mut saved=WORKER.lock().unwrap();if saved.as_ref().is_some_and(|worker|worker.thread.is_some()){bail!("queue fixture already running");}
        let request=|input,bits|PlaybackRequest{flac_samples: None, dsd_pcm: None, input,position:0.,sample_rate:96000,channels:2,source_bit_depth:bits,source_integer_lossless:true,
            dsp:Default::default(),impulse:None,measured_lufs:None,volume:1.,duration:Some(2.),transitions:Default::default()};
        let first=request(first,16);let next=request(second,32);
        let format=output::SequenceFormat{sample_rate:96000,channels:2,channel_mask:0,exclusive:false,float_output:false,bit_depth:32};
        let sequence=output::PlaybackSequence::new("first".into(),first.clone(),format)?;
        #[cfg(feature="adaptive-sync")] if let Some(ppm)=options["rate_ppm"].as_f64(){sequence.enable_rate_adjustment(ppm)?;}
        sequence.on_configured(move|sequence|{sequence.reserve()?.prepare("second".into(),next)?;sequence.queue_complete(true)})?;
        let control=Arc::new(PlaybackControl::default());let output=control.clone();
        if options["scheduled"]==true{control.start.arm(output::clock::host_ns().context("missing Android monotonic clock")?+8_000_000_000)?;}
        let report=Arc::new(Mutex::new(json!({"ended":{},"saw_second":false})));let target=report.clone();
        let stall=Arc::new(std::sync::atomic::AtomicU64::new(0));let delay=stall.clone();
        let thread=std::thread::spawn(move||output::play_sequence(first,config,output,sequence,move|update| {
            let millis=if update.playback_id=="first"&&update.position>=0.1 {delay.swap(0,Ordering::AcqRel)}else{0};
            if millis>0 {
                if let Ok(mut report)=target.lock(){report["injected_producer_stall_ms"]=json!(millis);}
                std::thread::sleep(Duration::from_millis(millis));
            }
            if let Ok(mut report)=target.lock() {
                if update.playback_id=="second" {report["saw_second"]=json!(true);}
                for completed in update.ended {report["ended"][&completed.playback_id]=json!(completed.frames);}
                if update.continuity_lost&&report.get("first_underrun_observation").is_none() {
                    report["first_underrun_observation"]=json!({"playback_id":update.playback_id,"position":update.position,"output_frames":update.output_frames});
                }
                report["continuity_lost"]=json!(update.continuity_lost);report["signal_path"]=json!(update.signal_path);report["position"]=json!(update.position);
            }
        }));
        *saved=Some(Worker{control,thread:Some(thread),report,stall});Ok(())
    })();if let Err(error)=result{let _=env.throw_new("java/lang/IllegalStateException",error.to_string());}
}
#[no_mangle]
pub extern "system" fn Java_com_rmusic_android_development_QueueDiagnostics_stall(mut env:JNIEnv<'_>,_:JClass<'_>,milliseconds:jlong) {
    let result=(||->Result<()> {
        if !(1..=2000).contains(&milliseconds){bail!("invalid bounded producer stall");}
        WORKER.lock().unwrap().as_ref().context("queue fixture absent")?.stall.store(milliseconds as u64,Ordering::Release);Ok(())
    })();if let Err(error)=result{let _=env.throw_new("java/lang/IllegalStateException",error.to_string());}
}
fn snapshot(worker:&Worker)->Value {let mut report=worker.report.lock().unwrap().clone();report["played"]=json!(worker.control.frames_played.load(Ordering::Acquire));report["written"]=json!(worker.control.frames_written.load(Ordering::Acquire));report["finished"]=json!(worker.thread.as_ref().is_none_or(|thread|thread.is_finished()));report["gate"]=json!(worker.control.start.snapshot());report["device_clock"]=json!(worker.control.device_clock.sample());report}
#[no_mangle]
pub extern "system" fn Java_com_rmusic_android_development_QueueDiagnostics_commit(mut env:JNIEnv<'_>,_:JClass<'_>,delay:jlong) {
    let result=(||->Result<()> {if !(10..=1000).contains(&delay){bail!("invalid scheduled fixture delay");}let saved=WORKER.lock().unwrap();let worker=saved.as_ref().context("queue fixture absent")?;
        worker.control.start.commit(output::clock::host_ns().context("missing Android clock")?+delay as u64*1_000_000)
    })();if let Err(error)=result{let _=env.throw_new("java/lang/IllegalStateException",error.to_string());}
}
#[no_mangle]
pub extern "system" fn Java_com_rmusic_android_development_QueueDiagnostics_conflict(mut env:JNIEnv<'_>,_:JClass<'_>)->jstring {
    let result=(||->Result<_> {
        let request=PlaybackRequest{flac_samples: None, dsd_pcm: None, input:"not-opened".into(),position:0.,sample_rate:96000,channels:2,source_bit_depth:32,source_integer_lossless:true,dsp:Default::default(),impulse:None,measured_lufs:None,volume:1.,duration:Some(1.),transitions:Default::default()};
        let control=PlaybackControl::default();let result=output::android::backend()?.pcm_output(&request,&OutputConfig::default(),&control);
        let message=match result{Ok(mut sink)=>{let _=sink.close();"unexpected second physical output".into()},Err(error)=>error.to_string()};
        Ok(env.new_string(message)?.into_raw())
    })();match result{Ok(reply)=>reply,Err(error)=>{let _=env.throw_new("java/lang/IllegalStateException",error.to_string());std::ptr::null_mut()}}
}
#[no_mangle]
pub extern "system" fn Java_com_rmusic_android_development_QueueDiagnostics_state(mut env:JNIEnv<'_>,_:JClass<'_>,paused:jni::sys::jboolean)->jstring {
    let result=(||->Result<_> {let saved=WORKER.lock().unwrap();let worker=saved.as_ref().context("queue fixture absent")?;worker.control.set_paused(paused!=0);Ok(env.new_string(snapshot(worker).to_string())?.into_raw())})();
    match result{Ok(reply)=>reply,Err(error)=>{let _=env.throw_new("java/lang/IllegalStateException",error.to_string());std::ptr::null_mut()}}
}
#[no_mangle]
pub extern "system" fn Java_com_rmusic_android_development_QueueDiagnostics_stop(mut env:JNIEnv<'_>,_:JClass<'_>)->jstring {
    let result=(||->Result<_> {let mut saved=WORKER.lock().unwrap();let worker=saved.as_mut().context("queue fixture absent")?;
        if let Some(thread)=worker.thread.take(){worker.control.stop();thread.join().map_err(|_|anyhow::anyhow!("queue fixture panicked"))??;}
        Ok(env.new_string(snapshot(worker).to_string())?.into_raw())})();
    match result{Ok(reply)=>reply,Err(error)=>{let _=env.throw_new("java/lang/IllegalStateException",error.to_string());std::ptr::null_mut()}}
}
