//! Native driver/clock QA. Accepts only a completely silent WAV programme.
use anyhow::{bail,ensure,Result};
use rmusic_audio::{impulse::{ImpulseResponse,ImpulseBank},output::{self,OutputConfig,PlaybackControl,PlaybackRequest,PlaybackSequence,SequenceFormat},DspConfig};
use std::{sync::{Arc,atomic::Ordering},time::{Duration,Instant}};

fn main()->Result<()> {
    let args=std::env::args().skip(1).collect::<Vec<_>>();
    ensure!((3..=4).contains(&args.len()),"usage: native_fir_output_qa silent.wav impulse.wav single|sequence|stop|pause [device]");
    let mode=args[2].as_str();ensure!(["single","sequence","stop","pause"].contains(&mode),"invalid mode");
    let source=ImpulseResponse::from_wav(&std::fs::read(&args[0])?)?;
    ensure!(source.kernels().iter().flatten().all(|v|*v==0.),"native QA requires a completely silent programme");
    let response=Arc::new(ImpulseResponse::from_wav(&std::fs::read(&args[1])?)?);
    let rate=source.sample_rate();let channels=source.channels() as u16;
    let programme=source.frames() as u64*if mode=="sequence"{2}else{1};let expected=programme+response.frames() as u64-1;
    let request=PlaybackRequest{flac_samples:None,dsd_pcm:None,input:args[0].clone(),position:0.,sample_rate:rate,channels,source_integer_lossless:true,source_bit_depth:32,dsp:DspConfig{enabled:true,..Default::default()},impulse:Some(Arc::new(ImpulseBank::new(vec![response])?)),measured_lufs:None,volume:1.,duration:Some(source.frames() as f64/rate as f64),transitions:Default::default()};
    let config=OutputConfig{device_id:args.get(3).cloned(),..Default::default()};let control=Arc::new(PlaybackControl::default());
    let pause=if mode=="pause" {
        control.paused.store(true,Ordering::Relaxed);let observer=control.clone();
        Some(std::thread::spawn(move||->Result<()>{
            let start=Instant::now();while observer.output_sample_rate.load(Ordering::Relaxed)==0 {if start.elapsed()>Duration::from_secs(5){bail!("output did not configure");}std::thread::sleep(Duration::from_millis(10));}
            std::thread::sleep(Duration::from_millis(200));let held=observer.frames_written.load(Ordering::Relaxed)==0;
            observer.set_volume(0.25)?;observer.muted.store(true,Ordering::Relaxed);observer.paused.store(false,Ordering::Relaxed);ensure!(held,"initial pause wrote programme frames");Ok(())
        }))
    }else{None};
    let(mut path,mut peak,mut updates,mut stopped_at)=(None,0_f64,0_u64,None);let mut ended=Vec::new();let start=Instant::now();
    let mut observe=|signal:output::SignalPath,meter:rmusic_audio::Meter| {
        path=Some(signal);peak=peak.max(meter.peak);updates+=1;
        if mode=="stop"&&stopped_at.is_none(){stopped_at=Some(control.frames_written.load(Ordering::Relaxed));control.stop();}
    };
    let result=if mode=="sequence" {
        let sequence=PlaybackSequence::new("a".into(),request.clone(),SequenceFormat{sample_rate:rate,channels,channel_mask:0,exclusive:false,float_output:true,bit_depth:32})?;
        sequence.reserve()?.prepare("b".into(),request.clone())?;
        output::play_sequence(request,config,control.clone(),sequence,|update|{ended.extend(update.ended);observe(update.signal_path,update.meter);})
    }else{output::play(request,config,control.clone(),observe)};
    if let Some(worker)=pause{worker.join().map_err(|_|anyhow::anyhow!("pause observer panicked"))??;}result?;
    let written=control.frames_written.load(Ordering::Relaxed);let played=control.frames_played.load(Ordering::Acquire);let seconds=control.seconds(rate);
    ensure!(peak==0.,"silent programme produced a nonzero DSP peak");ensure!(played<=written,"device clock exceeded submitted frames");
    if mode=="stop"{ensure!(Some(written)==stopped_at&&written<expected,"stop submitted additional output");ensure!(seconds<=programme as f64/rate as f64,"stop overcredited programme");}
    else{ensure!(written==expected&&played==expected,"native output lost programme or tail frames");ensure!(control.programme_end.load(Ordering::Acquire)==programme&&seconds==programme as f64/rate as f64,"tail advanced programme clock");}
    if mode=="sequence"{ensure!(ended.len()==2&&ended[0].playback_id=="a"&&ended[1].playback_id=="b"&&ended.iter().all(|v|v.frames==source.frames() as u64),"per-track credit mismatch");}
    let path=path.ok_or_else(||anyhow::anyhow!("no signal path report"))?;
    ensure!(!path.bit_perfect_eligible&&path.processing.iter().any(|v|v.contains("FIR")&&v.contains("original gain")),"FIR signal path missing");
    println!("{}",serde_json::json!({"mode":mode,"programme_frames":programme,"physical_expected":expected,"frames_written":written,"frames_played":played,"programme_seconds":seconds,"clock_ns":control.clock_ns.load(Ordering::Relaxed),"peak":peak,"updates":updates,"ended":ended,"pause_verified":mode=="pause","volume":control.volume_setting(),"muted":control.muted.load(Ordering::Relaxed),"elapsed_seconds":start.elapsed().as_secs_f64(),"signal_path":path}));
    Ok(())
}
