#![cfg(all(target_os="linux",feature="alsa-output"))]
#[path="support/logind.rs"] mod logind;
use rmusic_audio::{output::{self,power,PlaybackControl,PlaybackRequest,OutputConfig},DspConfig};
use std::{io::Read,sync::{Arc,Mutex,atomic::{AtomicBool,Ordering},mpsc},time::Duration};

#[test]
fn logind_authenticates_signals_and_releases_delay_after_final_checkpoint() {
    let bus=logind::Bus::new();let manager=logind::Manager::new(&bus);std::env::set_var("DBUS_SYSTEM_BUS_ADDRESS",&bus.address);
    let root=tempfile::tempdir().unwrap();let captured=root.path().join("captured.pcm");
    let config=root.path().join("alsa.conf");std::fs::write(&config,format!("pcm.null {{ type null }}\npcm.capture {{ type file slave.pcm null file \"{}\" format raw }}\n",captured.display())).unwrap();std::env::set_var("ALSA_CONFIG_PATH",config);
    let frames=8192usize;let payload:Vec<u8>=(0..frames*2).flat_map(|i|((i as i32)*321-1000).to_le_bytes()).collect();
    let mut wav=Vec::new();wav.extend(b"RIFF");wav.extend((36+payload.len() as u32).to_le_bytes());wav.extend(b"WAVEfmt ");wav.extend(16u32.to_le_bytes());wav.extend(1u16.to_le_bytes());wav.extend(2u16.to_le_bytes());wav.extend(768000u32.to_le_bytes());wav.extend((768000u32*8).to_le_bytes());wav.extend(8u16.to_le_bytes());wav.extend(32u16.to_le_bytes());wav.extend(b"data");wav.extend((payload.len() as u32).to_le_bytes());wav.extend(&payload);let source=root.path().join("qa.wav");std::fs::write(&source,wav).unwrap();
    let request=PlaybackRequest {flac_samples: None, dsd_pcm: None, input:source.to_string_lossy().into(),position:0.,sample_rate:768000,channels:2,source_integer_lossless:true,source_bit_depth:32,dsp:DspConfig::default(),impulse:None,measured_lufs:None,volume:1.,duration:Some(frames as f64/768000.),transitions:Default::default()};
    let saved=Arc::new(Mutex::new(Vec::new()));let history=saved.clone();let final_checkpoint=Arc::new(AtomicBool::new(false));let final_saved=final_checkpoint.clone();
    power::configure("e70465ba-68d8-444d-b8ed-7bfb90243872".into(),0,false,move|generation,held|{history.lock().unwrap().push((generation,held,final_saved.load(Ordering::Acquire)));Ok(())}).unwrap();assert!(power::supported());
    let mut inhibitor=manager.lock();inhibitor.set_nonblocking(true).unwrap();
    let control=Arc::new(PlaybackControl::default());let worker=control.clone();let checkpoint=power::checkpoint(&control);let input=request.clone();let done=final_checkpoint.clone();
    let (playing,heard)=mpsc::channel();let (release,wait)=mpsc::channel();
    let thread=std::thread::spawn(move||{let _checkpoint=checkpoint;let mut announced=false;let result=output::play(input,OutputConfig {device_id:Some("capture".into()),..Default::default()},worker.clone(),|_,_|{if !announced&&worker.frames_played.load(Ordering::Acquire)>0{announced=true;playing.send(()).unwrap();wait.recv_timeout(Duration::from_secs(4)).unwrap();}});done.store(true,Ordering::Release);result});
    if let Err(error)=heard.recv_timeout(Duration::from_secs(3)){control.stop();let _=release.send(());panic!("output did not reach a heard callback: {error}; result: {:?}",thread.join().unwrap());}
    bus.forge(true);std::thread::sleep(Duration::from_millis(150));assert!(!control.system_interrupted());
    manager.sleep(true);logind::wait(||control.system_interrupted());assert!(power::held_stamp().unwrap().is_some());
    bus.forge(false);std::thread::sleep(Duration::from_millis(50));assert!(power::acknowledge(&power::held_stamp().unwrap().unwrap()).is_err());
    assert!(inhibitor.read(&mut [0;1]).is_err(),"检查点仍占用时不得释放延时锁");release.send(()).unwrap();assert!(output::is_device_unavailable(&thread.join().unwrap().unwrap_err()));
    logind::wait(||matches!(inhibitor.read(&mut [0;1]),Ok(0)));assert_eq!(saved.lock().unwrap().last().copied(),Some((1,true,true)));
    manager.sleep(false);let _next_lock=manager.lock();logind::wait(power::supported);let stamp=power::held_stamp().unwrap().unwrap();assert!(power::held_stamp().unwrap().is_some());power::acknowledge(&stamp).unwrap();
    output::play(request,OutputConfig {device_id:Some("capture".into()),..Default::default()},Arc::new(PlaybackControl::default()),|_,_|{}).unwrap();assert_eq!(std::fs::read(captured).unwrap(),payload);
    let pending=Arc::new(PlaybackControl::default());power::watch(&pending).unwrap();drop(manager);logind::wait(||pending.system_interrupted());assert!(!power::supported());
    let replacement=logind::Manager::new(&bus);let _replacement_lock=replacement.lock();logind::wait(power::supported);assert!(power::held_stamp().unwrap().is_some(),"服务重连不能自动放行输出");
}
