#![cfg(all(target_os="linux",feature="alsa-output"))]
use rmusic_audio::{output::{self,OutputConfig,PlaybackControl,PlaybackRequest},DspConfig};
use std::{path::Path,sync::{Arc,Mutex,OnceLock,atomic::Ordering},time::Duration};
static LOCK:Mutex<()>=Mutex::new(());
static ROOT:OnceLock<tempfile::TempDir>=OnceLock::new();
fn fixture()->&'static Path {
    ROOT.get_or_init(||{
        let root=tempfile::tempdir().unwrap();
        let config=root.path().join("alsa.conf");
        std::fs::write(&config,format!("pcm.null {{ type null }}\npcm.capture {{ type file slave.pcm null file \"{}\" format raw hint {{ show on description \"R-Music QA capture\" }} }}\n",root.path().join("captured.pcm").display())).unwrap();
        std::env::set_var("ALSA_CONFIG_PATH",config);root
    }).path()
}
fn wav(root:&Path,bits:u16,rate:u32,channels:u16,frames:usize,pattern:&[i32])->(PlaybackRequest,Vec<u8>){
    let mut payload=Vec::new();let mut expected=Vec::new();
    for index in 0..frames*channels as usize {
        let sample=pattern[index%pattern.len()];payload.extend_from_slice(&sample.to_le_bytes()[..bits as usize/8]);expected.extend_from_slice(&(sample<<(32-bits)).to_le_bytes());
    }
    let mut data=Vec::new();data.extend_from_slice(b"RIFF");data.extend_from_slice(&(36+payload.len() as u32).to_le_bytes());data.extend_from_slice(b"WAVEfmt ");data.extend_from_slice(&16u32.to_le_bytes());data.extend_from_slice(&1u16.to_le_bytes());data.extend_from_slice(&channels.to_le_bytes());data.extend_from_slice(&rate.to_le_bytes());data.extend_from_slice(&(rate*channels as u32*bits as u32/8).to_le_bytes());data.extend_from_slice(&(channels*bits/8).to_le_bytes());data.extend_from_slice(&bits.to_le_bytes());data.extend_from_slice(b"data");data.extend_from_slice(&(payload.len() as u32).to_le_bytes());data.extend_from_slice(&payload);
    let path=root.join("source.wav");std::fs::write(&path,data).unwrap();
    (PlaybackRequest{flac_samples: None, dsd_pcm: None, input:path.to_string_lossy().into(),position:0.,sample_rate:rate,channels,source_integer_lossless:true,source_bit_depth:bits,dsp:DspConfig::default(),impulse:None,measured_lufs:None,volume:1.,duration:Some(frames as f64/rate as f64),transitions:Default::default()},expected)
}
fn config()->OutputConfig {OutputConfig{device_id:Some("capture".into()),exclusive:false,buffer_ms:100,..Default::default()}}
fn captured(root:&Path)->Vec<u8>{std::fs::read(root.join("captured.pcm")).unwrap()}
fn impulse(kernel:&[f64])->Arc<rmusic_audio::impulse::ImpulseBank>{
    impulse_at(kernel,48000)
}
fn impulse_at(kernel:&[f64],rate:u32)->Arc<rmusic_audio::impulse::ImpulseBank>{
    let mut bytes=b"RIFF".to_vec();bytes.extend((36+kernel.len() as u32*8).to_le_bytes());bytes.extend(b"WAVEfmt ");bytes.extend(16_u32.to_le_bytes());bytes.extend(3_u16.to_le_bytes());bytes.extend(1_u16.to_le_bytes());bytes.extend(rate.to_le_bytes());bytes.extend((rate*8).to_le_bytes());bytes.extend(8_u16.to_le_bytes());bytes.extend(64_u16.to_le_bytes());bytes.extend(b"data");bytes.extend((kernel.len() as u32*8).to_le_bytes());for tap in kernel{bytes.extend(tap.to_le_bytes());}
    Arc::new(rmusic_audio::impulse::ImpulseBank::new(vec![Arc::new(rmusic_audio::impulse::ImpulseResponse::from_wav(&bytes).unwrap())]).unwrap())
}

#[test]
fn real_alsa_selects_the_matching_rate_from_the_explicit_response_bank(){
    let _lock=LOCK.lock().unwrap();let root=fixture();let(mut req,source)=wav(root,32,48000,2,113,&[1<<27,-(1<<26)]);req.dsp.enabled=true;
    let a=impulse_at(&[2.],48000).response(48000).unwrap().clone();let b=impulse_at(&[-3.],96000).response(96000).unwrap().clone();req.impulse=Some(Arc::new(rmusic_audio::impulse::ImpulseBank::new(vec![b,a]).unwrap()));
    output::play(req,config(),Arc::default(),|path,_|{assert_eq!(path.sample_rate,48000);assert!(path.processing.iter().any(|stage|stage.contains("FIR")&&stage.contains("48000 Hz")));}).unwrap();
    let expected=source.chunks_exact(4).flat_map(|value|(i32::from_le_bytes(value.try_into().unwrap())*2).to_le_bytes()).collect::<Vec<_>>();assert_eq!(captured(root),expected);
}
#[test]
fn real_alsa_single_fir_keeps_faded_full_tail_and_excludes_tail_from_source_clock(){
    let _lock=LOCK.lock().unwrap();let root=fixture();let(mut req,original)=wav(root,32,48000,2,65,&[1<<28,-(1<<27)]);
    req.dsp.enabled=true;req.dsp.preamp_db=6.020599913279624;req.transitions.fade_out_ms=1;let mut kernel=vec![0.;65];kernel[0]=1.;kernel[64]=0.5;req.impulse=Some(impulse(&kernel));
    let mut expected=vec![0.;129*2];let samples=original.chunks_exact(4).map(|b|i32::from_le_bytes(b.try_into().unwrap())).collect::<Vec<_>>();
    for frame in 0..65{for channel in 0..2{let value=samples[frame*2+channel] as f64/2147483648.*2.*req.transitions.gain(frame as f64/48000.,req.duration);expected[frame*2+channel]+=value;expected[(frame+64)*2+channel]+=value*0.5;}}
    let expected=expected.iter().flat_map(|v|((v*2147483648.).round() as i32).to_le_bytes()).collect::<Vec<_>>();let control=Arc::new(PlaybackControl::default());let mut path=None;
    output::play(req,config(),control.clone(),|signal,_|path=Some(signal)).unwrap();assert_eq!(captured(root),expected);assert_eq!(control.frames_written.load(Ordering::Relaxed),129);assert_eq!(control.frames_played.load(Ordering::Acquire),129);assert_eq!(control.seconds(48000),65./48000.);
    let path=path.unwrap();assert!(!path.bit_perfect_eligible);assert!(path.processing.iter().any(|v|v.contains("FIR")&&v.contains("original gain")));
}
#[test]
fn real_alsa_continuous_fir_preserves_raw_gain_across_sources_and_credits_only_programme_frames(){
    let _lock=LOCK.lock().unwrap();let root=fixture();let(mut first,_)=wav(root,32,48000,2,1,&[1<<28,-(1<<27)]);
    let path=root.join("first-fir.wav");std::fs::copy(&first.input,&path).unwrap();first.input=path.to_string_lossy().into();
    let(mut second,_)=wav(root,32,48000,2,2,&[0,0]);first.dsp.enabled=true;second.dsp=first.dsp.clone();let mut kernel=vec![0.;65];kernel[0]=2.;kernel[64]=-3.;first.impulse=Some(impulse(&kernel));second.impulse=first.impulse.clone();
    let queue=sequence(first.clone());queue.reserve().unwrap().prepare("b".into(),second).unwrap();let control=Arc::new(PlaybackControl::default());let mut ended=Vec::new();
    output::play_sequence(first,config(),control.clone(),queue,|update|ended.extend(update.ended)).unwrap();
    let mut expected=vec![0_i32;67*2];expected[..2].copy_from_slice(&[1<<29,-(1<<28)]);expected[64*2..65*2].copy_from_slice(&[-(3<<28),3<<27]);
    assert_eq!(captured(root),expected.iter().flat_map(|v|v.to_le_bytes()).collect::<Vec<_>>());assert_eq!(control.seconds(48000),3./48000.);
    assert_eq!(ended.iter().map(|v|(v.playback_id.as_str(),v.frames)).collect::<Vec<_>>(),vec![("a",1),("b",2)]);
}
#[test]
fn real_alsa_fir_explicit_stop_before_first_packet_is_zero_output(){
    let _lock=LOCK.lock().unwrap();let root=fixture();let(mut req,_)=wav(root,32,48000,2,200,&[1<<28,-(1<<27)]);req.dsp.enabled=true;req.impulse=Some(impulse(&[2.,-3.]));let control=Arc::new(PlaybackControl::default());
    let before=std::fs::read(root.join("captured.pcm")).ok();
    control.stop();output::play(req,config(),control.clone(),|_,_|panic!("stopped playback must not publish output")).unwrap();assert_eq!(std::fs::read(root.join("captured.pcm")).ok(),before,"pre-stopped playback must not open or truncate an output");assert_eq!(control.frames_written.load(Ordering::Relaxed),0);assert_eq!(control.seconds(48000),0.);
}

#[test]
fn real_alsa_file_output_preserves_16_24_and_32_bit_pcm(){
    let _lock=LOCK.lock().unwrap();let root=fixture();let devices=output::devices().unwrap();assert!(devices.iter().any(|device|device.id=="capture"));
    for (bits,rate,channels,pattern) in [(16,44100,1,vec![0,1,-1,32767,-32768,12013,-9999]),(24,96000,2,vec![0,1,-1,8388607,-8388608,1234567,-7654321]),(32,192000,2,vec![0,1,-1,i32::MAX,i32::MIN,123456789,-987654321])]{
        let (request,expected)=wav(root,bits,rate,channels,4096,&pattern);let control=Arc::new(PlaybackControl::default());let mut previous=0;let mut reports=0;
        output::play(request,config(),control.clone(),|path,_|{
            assert_eq!(path.sample_rate,rate);assert_eq!(path.bit_depth,32);assert!(!path.bit_perfect_eligible);assert!(!path.exclusive);
            assert!(path.processing.iter().any(|stage|stage.contains("downstream format unverified")));
            let played=control.frames_played.load(Ordering::Relaxed);assert!(played>=previous);assert!(played<=control.frames_written.load(Ordering::Relaxed));previous=played;reports+=1;
        }).unwrap();
        assert_eq!(captured(root),expected,"PCM mismatch: {bits} bit / {rate} Hz");assert!(reports>0);assert_eq!(control.frames_written.load(Ordering::Relaxed),4096);assert_eq!(control.frames_played.load(Ordering::Relaxed),4096);assert!(control.clock_ns.load(Ordering::Relaxed)>0);
    }
}
#[test]
fn real_alsa_output_applies_gain_mute_dsp_and_fade(){
    let _lock=LOCK.lock().unwrap();let root=fixture();
    let (mut request,expected)=wav(root,16,48000,2,9600,&[10000]);request.volume=0.25;
    output::play(request,config(),Arc::new(PlaybackControl::default()),|path,_|assert!(path.processing.iter().any(|stage|stage=="Software volume"))).unwrap();
    assert_eq!(captured(root),expected.chunks_exact(4).flat_map(|bytes|((i32::from_le_bytes(bytes.try_into().unwrap()) as f64*0.25).round() as i32).to_le_bytes()).collect::<Vec<_>>());
    let (request,_)=wav(root,16,48000,2,9600,&[10000]);let control=Arc::new(PlaybackControl::default());control.muted.store(true,Ordering::Relaxed);
    output::play(request,config(),control,|_,_|{}).unwrap();assert!(captured(root).iter().all(|byte|*byte==0));
    let (mut request,_)=wav(root,16,48000,2,9600,&[10000]);request.dsp.enabled=true;request.dsp.preamp_db=-6.020599913279624;
    output::play(request,config(),Arc::new(PlaybackControl::default()),|path,_|assert!(path.processing.iter().any(|stage|stage=="DSP f64 → integer PCM"))).unwrap();
    assert!(captured(root).chunks_exact(4).all(|bytes|(i32::from_le_bytes(bytes.try_into().unwrap()) as i64-327680000).abs()<=1));
    let (mut request,_)=wav(root,16,48000,2,9600,&[10000]);request.transitions.fade_in_ms=50;request.transitions.fade_out_ms=50;
    output::play(request,config(),Arc::new(PlaybackControl::default()),|path,_|assert!(path.processing.iter().any(|stage|stage=="Fade envelope"))).unwrap();
    let samples:Vec<i32>=captured(root).chunks_exact(4).map(|bytes|i32::from_le_bytes(bytes.try_into().unwrap())).collect();assert_eq!(samples[0],0);assert_eq!(samples[9600],655360000);assert!(samples.last().unwrap().abs()<1_000_000);
}
#[test]
fn initial_and_midstream_pause_do_not_skip_pcm_frames(){
    let _lock=LOCK.lock().unwrap();let root=fixture();let (request,expected)=wav(root,24,96000,2,192000,&[1234567,-7654321,0,1]);
    let control=Arc::new(PlaybackControl::default());control.paused.store(true,Ordering::Relaxed);let worker_control=control.clone();
    let (ready,waiting)=std::sync::mpsc::channel();let (done,finished)=std::sync::mpsc::channel();
    let worker=std::thread::spawn(move||{let mut initial=false;let mut paused=false;let observer=worker_control.clone();let result=output::play(request,config(),worker_control,move|_,_|{
        let written=observer.frames_written.load(Ordering::Relaxed);if !initial {ready.send(written).unwrap();initial=true;}
        if written>0&&!paused {observer.paused.store(true,Ordering::Relaxed);ready.send(written).unwrap();paused=true;}
    });done.send(result).unwrap();});
    assert_eq!(waiting.recv_timeout(Duration::from_secs(5)).unwrap(),0);std::thread::sleep(Duration::from_millis(80));assert_eq!(control.frames_written.load(Ordering::Relaxed),0);control.paused.store(false,Ordering::Relaxed);
    let paused_at=waiting.recv_timeout(Duration::from_secs(5)).unwrap();std::thread::sleep(Duration::from_millis(100));assert_eq!(control.frames_written.load(Ordering::Relaxed),paused_at);control.paused.store(false,Ordering::Relaxed);
    let result=finished.recv_timeout(Duration::from_secs(5));if result.is_err(){control.stop();}worker.join().unwrap();result.unwrap().unwrap();assert_eq!(captured(root),expected);
}
#[test]
fn stop_while_paused_exits_and_keeps_only_the_written_prefix(){
    let _lock=LOCK.lock().unwrap();let root=fixture();let (request,expected)=wav(root,24,96000,2,192000,&[1234567,-7654321]);
    let control=Arc::new(PlaybackControl::default());let worker_control=control.clone();let observer=control.clone();
    let (ready,waiting)=std::sync::mpsc::channel();let (done,finished)=std::sync::mpsc::channel();
    let worker=std::thread::spawn(move||{let mut paused=false;let result=output::play(request,config(),worker_control,move|_,_|{if !paused&&observer.frames_written.load(Ordering::Relaxed)>0{observer.paused.store(true,Ordering::Relaxed);ready.send(()).unwrap();paused=true;}});done.send(result).unwrap();});
    waiting.recv_timeout(Duration::from_secs(5)).unwrap();control.stop();finished.recv_timeout(Duration::from_secs(1)).unwrap().unwrap();worker.join().unwrap();
    let bytes=captured(root);assert!(!bytes.is_empty()&&bytes.len()<expected.len());assert_eq!(bytes,expected[..bytes.len()]);
}
#[test]
fn unsupported_output_is_rejected_before_altering_the_capture(){
    let _lock=LOCK.lock().unwrap();let root=fixture();let (request,_)=wav(root,24,96000,2,4096,&[1,-1]);let before=std::fs::read(root.join("captured.pcm")).ok();
    let mut exclusive=config();exclusive.exclusive=true;assert!(output::play(request.clone(),exclusive,Arc::new(PlaybackControl::default()),|_,_|{}).unwrap_err().to_string().contains("hw:"));
    let mut invalid=config();invalid.buffer_ms=0;assert!(output::play(request,invalid,Arc::new(PlaybackControl::default()),|_,_|{}).is_err());assert_eq!(std::fs::read(root.join("captured.pcm")).ok(),before);
}
fn track(root:&Path,name:&str,bits:u16,rate:u32,channels:u16,frames:usize,pattern:&[i32])->(PlaybackRequest,Vec<u8>){
    let directory=root.join(name);std::fs::create_dir_all(&directory).unwrap();wav(&directory,bits,rate,channels,frames,pattern)
}
fn sequence(request:PlaybackRequest)->output::PlaybackSequence {
    output::PlaybackSequence::new("a".into(),request.clone(),output::SequenceFormat{sample_rate:request.sample_rate,channels:request.channels,channel_mask:0,exclusive:false,float_output:false,bit_depth:32}).unwrap()
}
fn wait_start_ready(control:&PlaybackControl) {
    let until=std::time::Instant::now()+Duration::from_secs(4);
    while control.start.snapshot().unwrap().phase!=output::start::Phase::Ready {
        assert!(std::time::Instant::now()<until,"output never became ready");std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn scheduled_start_alsa_prefills_without_consumption_then_starts_pcm768k_and_preserves_tracks(){
    let _lock=LOCK.lock().unwrap();let root=fixture();
    let(a,mut expected)=track(root,"scheduled-a",24,768000,2,113,&[8388607,-8388608,1,-1]);
    let(b,next)=track(root,"scheduled-b",24,768000,2,997,&[7654321,-7654321]);expected.extend(next);
    let queue=sequence(a.clone());queue.reserve().unwrap().prepare("b".into(),b).unwrap();
    let control=Arc::new(PlaybackControl::default());control.start.arm(output::clock::host_ns().unwrap()+5_000_000_000).unwrap();let observer=control.clone();
    let worker=std::thread::spawn(move||{let mut ended=Vec::new();let result=output::play_sequence(a,config(),observer,queue,|event|ended.extend(event.ended));(result,ended)});
    wait_start_ready(&control);assert_eq!(control.frames_played.load(Ordering::Acquire),0);assert!(control.frames_written.load(Ordering::Relaxed)>0);assert!(!control.device_clock.sample().unwrap().running);
    let deadline=output::clock::host_ns().unwrap()+100_000_000;control.start.commit(deadline).unwrap();std::thread::sleep(Duration::from_millis(10));assert_eq!(control.frames_played.load(Ordering::Acquire),0);
    let(result,ended)=worker.join().unwrap();result.unwrap();let start=control.start.snapshot().unwrap();
    assert_eq!(start.phase,output::start::Phase::Released);assert!((deadline..=deadline+20_000_000).contains(&start.released_host_ns));assert_eq!(captured(root),expected);assert_eq!(control.frames_played.load(Ordering::Acquire),1110);
    assert_eq!(ended.iter().map(|e|(e.playback_id.as_str(),e.frames)).collect::<Vec<_>>(),[("a",113),("b",997)]);println!("SCHEDULED_START_QA={}",serde_json::to_string(&start).unwrap());
}
#[test]
fn scheduled_start_alsa_cancels_a_prefilled_device_without_consuming_programme(){
    let _lock=LOCK.lock().unwrap();let root=fixture();for pause in [false,true] {let(a,_)=track(root,"scheduled-cancel",16,48000,2,257,&[32767,-32768]);let queue=sequence(a.clone());
    let control=Arc::new(PlaybackControl::default());control.start.arm(output::clock::host_ns().unwrap()+5_000_000_000).unwrap();let observer=control.clone();
    let worker=std::thread::spawn(move||output::play_sequence(a,config(),observer,queue,|event|assert_eq!(event.position,0.)));
    wait_start_ready(&control);if pause{control.paused.store(true,Ordering::Release);}else{control.stop();}worker.join().unwrap().unwrap();assert_eq!(control.start.snapshot().unwrap().phase,output::start::Phase::Cancelled);assert_eq!(control.frames_played.load(Ordering::Acquire),0);assert_eq!(control.seconds(48000),0.);
    assert!(control.stopped.load(Ordering::Acquire));}
}
#[test]
fn real_alsa_continuous_output_keeps_every_sample_across_multiple_subperiod_tracks(){
    let _lock=LOCK.lock().unwrap();let root=fixture();
    let (a,mut expected)=track(root,"gapless-a",16,48000,2,113,&[32767,-32768,1,-1]);
    let (b,bytes_b)=track(root,"gapless-b",24,48000,2,997,&[8388607,-8388608,1234567,-7654321]);
    let (c,bytes_c)=track(root,"gapless-c",32,48000,2,333,&[i32::MIN,i32::MAX,1,-1]);expected.extend(bytes_b);expected.extend(bytes_c);
    let queue=sequence(a.clone());queue.reserve().unwrap().prepare("b".into(),b).unwrap();queue.reserve().unwrap().prepare("c".into(),c).unwrap();
    let control=Arc::new(PlaybackControl::default());let mut ended=Vec::new();let mut reports=Vec::new();
    let mut clock_seen=false;
    output::play_sequence(a,config(),control.clone(),queue,|event|{if let Some(sample)=control.device_clock.sample(){assert_eq!(sample.source,output::clock::Source::AlsaFrames);assert_eq!(sample.timestamp_domain,output::clock::Domain::Monotonic);assert_eq!(sample.nominal_frequency_hz,48000);assert!(sample.host_read_after_ns>=sample.host_read_before_ns);clock_seen=true;}assert!(!event.continuity_lost);ended.extend(event.ended);reports.push((event.playback_id,event.position));}).unwrap();
    assert!(clock_seen);assert!(control.device_clock.sample().is_none());assert_eq!(captured(root),expected);assert_eq!(control.frames_written.load(Ordering::Relaxed),1443);assert_eq!(control.frames_played.load(Ordering::Relaxed),1443);
    assert_eq!(ended.iter().map(|event|(event.playback_id.as_str(),event.frames)).collect::<Vec<_>>(),[("a",113),("b",997),("c",333)]);assert_eq!(reports.last().unwrap(),&("c".into(),333./48000.));
}
#[test]
fn real_alsa_continuous_shared_route_converts_the_next_source_to_the_open_device_format(){
    use rmusic_audio::decoder::{FfmpegDecoder,PcmEncoding};
    let _lock=LOCK.lock().unwrap();let root=fixture();
    let(a,mut expected)=track(root,"converted-a",16,48000,2,1301,&[12000,-12000]);let(b,_)=track(root,"converted-b",24,44100,1,441,&[1234567,-7654321]);
    let mut reference=FfmpegDecoder::open(&b.input,0.,48000,2,PcmEncoding::Signed32).unwrap();let mut buffer=vec![0;8192];let mut next_frames=0;loop{let frames=reference.read_frames(&mut buffer,8).unwrap();if frames==0{break;}expected.extend_from_slice(&buffer[..frames*8]);next_frames+=frames;}
    let queue=sequence(a.clone());queue.reserve().unwrap().prepare("b".into(),b).unwrap();let mut converted=false;
    output::play_sequence(a,config(),Arc::default(),queue,|event|{if event.playback_id=="b"{assert_eq!((event.signal_path.sample_rate,event.signal_path.channels),(48000,2));assert!(event.signal_path.processing.iter().any(|stage|stage.contains("44100 → 48000")));assert!(event.signal_path.processing.iter().any(|stage|stage.contains("1 → 2")));if let Some(end)=event.ended.iter().find(|end|end.playback_id=="b"){assert_eq!(end.frames,next_frames as u64);}converted=true;}}).unwrap();
    assert!(converted);assert_eq!(captured(root),expected);
}
#[test]
fn real_alsa_continuous_pause_resume_keeps_staged_boundary_samples(){
    let _lock=LOCK.lock().unwrap();let root=fixture();
    let(a,mut expected)=track(root,"pause-a",16,48000,2,113,&[12345,-5432]);let(b,bytes)=track(root,"pause-b",24,48000,2,7200,&[1234567,-7654321]);expected.extend(bytes);
    let queue=sequence(a.clone());queue.reserve().unwrap().prepare("b".into(),b).unwrap();let control=Arc::new(PlaybackControl::default());let worker_control=control.clone();let (ready,waiting)=std::sync::mpsc::channel();let(done,finished)=std::sync::mpsc::channel();
    let worker=std::thread::spawn(move||{let observer=worker_control.clone();let mut paused=false;let result=output::play_sequence(a,config(),worker_control,queue,|_|{if !paused&&observer.frames_written.load(Ordering::Relaxed)>0 {paused=true;observer.paused.store(true,Ordering::Relaxed);ready.send(observer.frames_written.load(Ordering::Relaxed)).unwrap();}});done.send(result).unwrap();});
    let written=waiting.recv_timeout(Duration::from_secs(5)).unwrap();std::thread::sleep(Duration::from_millis(80));assert_eq!(control.frames_written.load(Ordering::Relaxed),written);control.paused.store(false,Ordering::Relaxed);let result=finished.recv_timeout(Duration::from_secs(5));if result.is_err(){control.stop();}worker.join().unwrap();result.unwrap().unwrap();assert_eq!(captured(root),expected);
}
#[test]
fn real_alsa_stop_cancels_a_stalled_preload_without_a_false_track_end(){
    let _lock=LOCK.lock().unwrap();let root=fixture();let(a,expected)=track(root,"stop-a",16,48000,2,113,&[12345,-5432]);let queue=sequence(a.clone());
    let server=std::net::TcpListener::bind("127.0.0.1:0").unwrap();let address=server.local_addr().unwrap();let(accepted,connected)=std::sync::mpsc::channel();let(release,waiting)=std::sync::mpsc::channel();
    let server_thread=std::thread::spawn(move||{let(socket,_)=server.accept().unwrap();accepted.send(()).unwrap();let _=waiting.recv_timeout(Duration::from_secs(5));drop(socket);});
    let mut blocked=a.clone();blocked.input=format!("http://{address}/stalled.wav");let preload=queue.reserve().unwrap();let prep=std::thread::spawn(move||preload.prepare("b".into(),blocked));connected.recv_timeout(Duration::from_secs(5)).unwrap();
    let control=Arc::new(PlaybackControl::default());let observer=control.clone();let(done,finished)=std::sync::mpsc::channel();let worker=std::thread::spawn(move||{let result=output::play_sequence(a,config(),control,queue,|event|{assert!(event.ended.is_empty());observer.stop();});done.send(result).unwrap();});
    let result=finished.recv_timeout(Duration::from_secs(1));let _=release.send(());worker.join().unwrap();server_thread.join().unwrap();result.unwrap().unwrap();assert!(prep.join().unwrap().is_err());assert_eq!(captured(root),expected);
}
#[test]
fn failed_continuous_device_setup_cancels_preload_and_preserves_the_previous_capture(){
    let _lock=LOCK.lock().unwrap();let root=fixture();let(a,_)=track(root,"failed-setup-a",16,48000,2,113,&[1,-1]);let queue=sequence(a.clone());let before=std::fs::read(root.join("captured.pcm")).ok();
    let server=std::net::TcpListener::bind("127.0.0.1:0").unwrap();let address=server.local_addr().unwrap();let(accepted,connected)=std::sync::mpsc::channel();let(release,waiting)=std::sync::mpsc::channel();
    let server_thread=std::thread::spawn(move||{let(socket,_)=server.accept().unwrap();accepted.send(()).unwrap();let _=waiting.recv_timeout(Duration::from_secs(5));drop(socket);});
    let mut blocked=a.clone();blocked.input=format!("http://{address}/stalled.wav");let preload=queue.reserve().unwrap();let(done,finished)=std::sync::mpsc::channel();let prep=std::thread::spawn(move||{done.send(preload.prepare("b".into(),blocked).err().unwrap().is::<output::PreloadCancelled>()).unwrap();});connected.recv_timeout(Duration::from_secs(5)).unwrap();
    let mut invalid=config();invalid.buffer_ms=0;assert!(output::play_sequence(a,invalid,Arc::default(),queue.clone(),|_|{}).is_err());assert!(queue.reserve().is_err());
    let result=finished.recv_timeout(Duration::from_secs(1));let _=release.send(());prep.join().unwrap();server_thread.join().unwrap();assert!(result.unwrap());assert_eq!(std::fs::read(root.join("captured.pcm")).ok(),before);
}
