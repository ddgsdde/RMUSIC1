use super::*;
#[test]
fn paused_output_wakes_on_controls_even_when_the_signal_precedes_parking() {
    let control=Arc::new(PlaybackControl::default());control.set_paused(true);
    let (ready,started)=std::sync::mpsc::channel();let (done,finished)=std::sync::mpsc::channel();
    let state=control.clone();let worker=std::thread::spawn(move||{
        state.register_output_thread();ready.send(()).unwrap();
        std::thread::park_timeout(std::time::Duration::from_secs(10));
        done.send(state.paused.load(Ordering::Acquire)).unwrap();
    });
    started.recv_timeout(std::time::Duration::from_secs(1)).unwrap();
    assert!(finished.recv_timeout(std::time::Duration::from_millis(30)).is_err());
    control.set_paused(false);
    assert!(!finished.recv_timeout(std::time::Duration::from_millis(500)).unwrap());worker.join().unwrap();
}
#[cfg(feature="adaptive-sync")]
#[path="sequence/rate_tests.rs"]
mod rate_tests;
#[test]
fn device_failure_classification_keeps_typed_causes_and_excludes_message_matches() {
    let error=anyhow::Error::new(DeviceUnavailable("device stopped".into())).context("output worker failed");
    assert!(is_device_unavailable(&error));
    assert!(!is_device_unavailable(&anyhow::anyhow!("decoder error: 0x88890004 DAC disconnected")));
    #[cfg(windows)] {
        wasapi::initialize_mta().ok().unwrap();
        let enumerator=wasapi::DeviceEnumerator::new().unwrap();
        let error=enumerator.get_device("{0.0.0.00000000}.{3895f8f9-5ea6-4e8e-9a14-3739456484bc}").err().unwrap();
        assert!(is_device_unavailable(&anyhow::Error::new(error).context("open selected output")));
        assert!(!is_device_unavailable(&wasapi::WasapiError::UnsupportedFormat.into()));
    }
    #[cfg(all(target_os="linux",feature="alsa-output"))] {
        assert!(is_device_unavailable(&anyhow::Error::new(::alsa::Error::new("snd_pcm_writei",19)).context("output worker")));
        for code in [11,16,22,32] {assert!(!is_device_unavailable(&::alsa::Error::new("snd_pcm_writei",code).into()));}
    }
}
#[test]
fn device_idle_frames_never_count_as_heard_audio_or_advance_a_newly_submitted_track() {
    let mut clock = DeviceClock::default();
    assert_eq!(clock.advance(0, 0, false), 0);
    clock.submitted(1000);
    assert_eq!(clock.advance(750, 1000, true), 750);
    assert_eq!(clock.advance(1500, 1000, true), 1000);
    assert!(clock.continuity_lost);
    assert_eq!(clock.advance(2500, 1000, true), 1000);
    clock.submitted(1000);
    assert_eq!(clock.advance(2500, 2000, true), 1000);
    assert_eq!(clock.advance(2750, 2000, true), 1250);
    assert_eq!(clock.advance(3500, 2000, true), 2000);
    // A final exclusive packet has device padding; only its real source samples count.
    clock.submitted(1000);
    assert_eq!(clock.advance(3600, 2113, false), 2100);
    assert_eq!(clock.advance(3613, 2113, false), 2113);
    assert_eq!(clock.advance(4500, 2113, false), 2113);
}
use std::{io::Write, sync::mpsc, time::Duration};
#[test]
fn dop_sequence_preserves_dsf_dff_payload_preload_clock_and_physical_packet_tail() {
    use crate::dsd::{tests::{samples,dsf,dff},DopEncoder,DopContainer};
    let first=samples(257*2);let second=samples(263*2);
    let mut files=Vec::new();
    for (extension,bytes) in [(".dsf",dsf(&first,257*16,true)),(".dff",dff(&second))] {
        let mut file=tempfile::Builder::new().suffix(extension).tempfile().unwrap();file.write_all(&bytes).unwrap();file.flush().unwrap();files.push(file);
    }
    let metadata=files.iter().map(|file|crate::probe::probe(file.path()).unwrap()).collect::<Vec<_>>();
    for info in &metadata {assert_eq!((info.sample_rate,info.output_rate(),info.bit_depth),(2822400,176400,1));assert!(info.is_dsd());assert!(!info.source_integer_lossless);}
    let initial=PlaybackRequest{sample_rate:176400,source_integer_lossless:false,duration:metadata[0].duration,..request(&files[0],1,257)};
    let next=PlaybackRequest{sample_rate:176400,source_integer_lossless:false,duration:metadata[1].duration,..request(&files[1],1,263)};
    let format=SequenceFormat{sample_rate:176400,exclusive:true,..format()};
    let sequence=PlaybackSequence::new("dsf".into(),initial.clone(),format).unwrap();
    sequence.reserve().unwrap().prepare("dff".into(),next).unwrap();
    let control=Arc::new(PlaybackControl::default());let mut stream=PcmStream::open(initial,sequence.clone(),control.clone(),format).unwrap();
    let mut output=Vec::new();let mut wire=DopEncoder::new(2).unwrap();
    loop {match stream.read(129).unwrap(){Read::Data(mut chunk)=>{wire.stamp(&mut chunk.bytes,DopContainer::LeftAligned32,chunk.frames,false).unwrap();output.extend(chunk.bytes);},Read::End=>break,Read::Pending=>panic!("preloaded DFF must be ready")}}
    assert_eq!(output.len(),520*8);
    let mut tail=[0;3*8];wire.stamp(&mut tail,DopContainer::LeftAligned32,0,false).unwrap();output.extend(tail);
    let check=|bytes:&[u8],width:usize,left_aligned:bool| {
        for (frame,packet) in bytes.chunks_exact(width*2).enumerate(){for channel in 0..2 {
            let sample=&packet[channel*width..][..width];let offset=usize::from(left_aligned);
            assert_eq!(sample[offset+2],if frame%2==0{5}else{250});
            let(older,newer)=if frame<257 {(first[channel][frame*2],first[channel][frame*2+1])}else if frame<520 {(second[channel][(frame-257)*2],second[channel][(frame-257)*2+1])}else{(0x69,0x69)};
            assert_eq!((sample[offset+1],sample[offset]),(older,newer));
        }}
    };
    check(&output,4,true);
    #[cfg(windows)] for (container,valid) in [(32,32),(32,24),(24,24)] {
        let device=wasapi::WaveFormat::new(container,valid,&wasapi::SampleType::Int,176400,2,None);
        let packed=super::wasapi_format::pack(&device,&output).unwrap();check(&packed,container as usize/8,container==32);
    }
    control.frames_written.store(520,Ordering::Relaxed);control.frames_played.store(520,Ordering::Relaxed);
    let(_,position,_,ended)=sequence.heard(&control,true).unwrap();
    assert_eq!(position,263./176400.);assert_eq!(ended.iter().map(|row|row.frames).collect::<Vec<_>>(),[257,263]);
    assert!((ended.iter().map(|row|row.position).sum::<f64>()-520./176400.).abs()<1e-12);
}
#[test]
fn dop_admission_rejects_processing_shared_float_mixed_and_unmapped_sources() {
    use crate::dsd::tests::{samples,dsf,dff};
    let data=samples(32);let mut file=tempfile::Builder::new().suffix(".dsf").tempfile().unwrap();file.write_all(&dsf(&data,256,true)).unwrap();file.flush().unwrap();
    let initial=PlaybackRequest{sample_rate:176400,source_integer_lossless:false,duration:Some(256./2822400.),..request(&file,1,16)};
    let config=OutputConfig{device_id:Some("selected-dac".into()),exclusive:true,dsd_output:DsdOutput::Dop,..Default::default()};
    initial.validate_dop(&config).unwrap();
    for invalid in [OutputConfig{dsd_output:DsdOutput::Disabled,..config.clone()},OutputConfig{exclusive:false,..config.clone()},OutputConfig{device_id:None,..config.clone()}]{assert!(initial.validate_dop(&invalid).is_err());}
    for invalid in [PlaybackRequest{volume:0.5,..initial.clone()},PlaybackRequest{dsp:DspConfig{enabled:true,..Default::default()},..initial.clone()},PlaybackRequest{transitions:crate::fade::FadeConfig{fade_in_ms:10,..Default::default()},..initial.clone()}]{assert!(invalid.validate_dop(&config).is_err());}
    let format=SequenceFormat{sample_rate:176400,exclusive:true,..format()};
    for invalid in [SequenceFormat{float_output:true,..format},SequenceFormat{bit_depth:16,..format},SequenceFormat{sample_rate:192000,..format},SequenceFormat{channel_mask:0x33,..format}] {assert!(SourceDecoder::open(&initial,invalid,&OutputLimits::default(),|_|Ok(())).is_err());}
    let sequence=PlaybackSequence::new("dsd".into(),initial.clone(),format).unwrap();
    let pcm=PlaybackRequest{source_bit_depth:24,source_integer_lossless:true,..initial.clone()};
    assert!(sequence.reserve().unwrap().prepare("pcm".into(),pcm.clone()).unwrap_err().is::<PreloadFormatChange>());
    let sequence=PlaybackSequence::new("pcm".into(),pcm,format).unwrap();
    assert!(sequence.reserve().unwrap().prepare("dsd".into(),initial.clone()).unwrap_err().is::<PreloadFormatChange>());
    let sequence=PlaybackSequence::new("dsd".into(),initial.clone(),format).unwrap();let control=Arc::new(PlaybackControl::default());
    let mut stream=PcmStream::open(initial.clone(),sequence,control.clone(),format).unwrap();control.set_volume(0.5).unwrap();assert!(stream.read(1).is_err());
    let mut unknown=dff(&data);let label=unknown.windows(4).position(|part|part==b"SLFT").unwrap();unknown[label..label+4].copy_from_slice(b"BAD!");
    let mut file=tempfile::Builder::new().suffix(".dff").tempfile().unwrap();file.write_all(&unknown).unwrap();file.flush().unwrap();
    assert!(SourceDecoder::open(&PlaybackRequest{input:file.path().to_str().unwrap().into(),..initial},format,&OutputLimits::default(),|_|Ok(())).is_err());
}
fn wav(samples: &[i32], bits: u16) -> tempfile::NamedTempFile {
    let mut file = tempfile::Builder::new().suffix(".wav").tempfile().unwrap();
    let width = bits / 8;
    let length = samples.len() as u32 * width as u32;
    for bytes in [
        b"RIFF".as_slice(),
        &(36 + length).to_le_bytes(),
        b"WAVEfmt ",
        &16u32.to_le_bytes(),
        &1u16.to_le_bytes(),
        &2u16.to_le_bytes(),
        &48000u32.to_le_bytes(),
        &(48000 * 2 * width as u32).to_le_bytes(),
        &(2 * width).to_le_bytes(),
        &bits.to_le_bytes(),
        b"data",
        &length.to_le_bytes(),
    ] {
        file.write_all(bytes).unwrap();
    }
    for sample in samples {
        file.write_all(&sample.to_le_bytes()[..width as usize])
            .unwrap();
    }
    file.flush().unwrap();
    file
}
fn request(file: &tempfile::NamedTempFile, bits: u16, frames: usize) -> PlaybackRequest {
    PlaybackRequest {flac_samples: None, dsd_pcm: None,
        input: file.path().to_string_lossy().into(),
        position: 0.,
        sample_rate: 48000,
        channels: 2,
        source_integer_lossless: true,
        source_bit_depth: bits,
        dsp: DspConfig::default(),
        impulse:None,
        measured_lufs: None,
        volume: 1.,
        duration: Some(frames as f64 / 48000.),
        transitions: Default::default(),
    }
}
fn format() -> SequenceFormat {
    SequenceFormat {
        sample_rate: 48000,
        channels: 2,
        channel_mask:0,
        exclusive: true,
        float_output: false,
        bit_depth: 32,
    }
}
fn sequence(request: PlaybackRequest) -> PlaybackSequence {
    PlaybackSequence::new("a".into(), request, format()).unwrap()
}
fn boundary(sequence:&PlaybackSequence,id:String,request:PlaybackRequest,start:u64) {
    let mut state=sequence.state.lock().unwrap();
    if let Some(last)=state.history.back_mut() {last.end=Some(start);}
    state.history.push_back(Boundary{id,request,start,end:None,
        #[cfg(target_os="android")] decoder_processing:Vec::new(),
        #[cfg(feature="adaptive-sync")] output_start:None,
        #[cfg(feature="adaptive-sync")] output_end:None,
    });
}
fn collect(stream: &mut PcmStream) -> Vec<u8> {
    let mut result = Vec::new();
    loop {
        match stream.read(137).unwrap() {
            Read::Data(chunk) => result.extend(chunk.bytes),
            Read::Pending => panic!("unexpected unprepared track"),
            Read::End => return result,
        }
    }
}
#[test]
fn balanced_fades_preserve_pcm_precision_channels_and_unity_middle_without_gain_boost() {
    for bits in [24,32] {
        let frames=48000;
        let samples=(0..frames).flat_map(|i|{
            let value=if bits==24{(i as i32*7919)%8_000_000-4_000_000}else{(i as i64*7919%4_000_000_000-2_000_000_000)as i32};[value,-value]
        }).collect::<Vec<_>>();
        let file=wav(&samples,bits);let mut req=request(&file,bits,frames);
        req.transitions=crate::fade::FadeConfig{fade_in_ms:80,fade_out_ms:120};
        for floating in [false,true] {
            let target=SequenceFormat{float_output:floating,bit_depth:if floating{64}else{32},..format()};
            let sequence=PlaybackSequence::new("fade-quality".into(),req.clone(),target).unwrap();
            let mut stream=PcmStream::open(req.clone(),sequence,Arc::default(),target).unwrap();let actual=collect(&mut stream);
            let width=if floating{8}else{4};assert_eq!(actual.len(),frames*2*width);
            for(i,bytes)in actual.chunks_exact(width).enumerate(){
                let source=if bits==24{samples[i]<<8}else{samples[i]};let gain=req.transitions.gain((i/2)as f64/48000.,req.duration);
                assert!((0. ..=1.).contains(&gain));
                if floating{assert_eq!(f64::from_le_bytes(bytes.try_into().unwrap()),source as f64/2147483648.*gain);}
                else{assert_eq!(i32::from_le_bytes(bytes.try_into().unwrap()),(source as f64*gain).round()as i32);}
                if (4800..42000).contains(&(i/2)){assert_eq!(gain,1.);}
            }
        }
    }
}
fn impulse(kernels:&[Vec<f64>],rate:u32,mask:u32)->Arc<crate::impulse::ImpulseBank> {
    let channels=kernels.len() as u16;let mut format=Vec::new();format.extend((if mask==0{3_u16}else{0xfffe_u16}).to_le_bytes());format.extend(channels.to_le_bytes());format.extend(rate.to_le_bytes());format.extend((rate*channels as u32*8).to_le_bytes());format.extend((channels*8).to_le_bytes());format.extend(64_u16.to_le_bytes());
    if mask!=0{format.extend(22_u16.to_le_bytes());format.extend(64_u16.to_le_bytes());format.extend(mask.to_le_bytes());format.extend(3_u32.to_le_bytes());format.extend([0,0,16,0,128,0,0,170,0,56,155,113]);}
    let mut bytes=b"RIFF\0\0\0\0WAVEfmt ".to_vec();bytes.extend((format.len() as u32).to_le_bytes());bytes.extend(format);bytes.extend(b"data");bytes.extend((kernels[0].len() as u32*channels as u32*8).to_le_bytes());
    for frame in 0..kernels[0].len(){for channel in kernels{bytes.extend(channel[frame].to_le_bytes());}}
    let size=bytes.len() as u32-8;bytes[4..8].copy_from_slice(&size.to_le_bytes());Arc::new(crate::impulse::ImpulseBank::new(vec![Arc::new(crate::impulse::ImpulseResponse::from_wav(&bytes).unwrap())]).unwrap())
}
#[test]
fn configured_fir_without_loaded_responses_is_rejected_before_opening_a_decoder(){
    let source=wav(&[0,0],32);let mut req=request(&source,32,1);req.input="qa-source-must-not-be-opened".into();req.dsp.enabled=true;req.dsp.convolution=Some(crate::impulse::ConvolutionConfig{response_ids:vec!["0".repeat(64)]});
    assert!(require_loaded_impulse(&req).unwrap_err().to_string().contains("尚未加载"));assert!(PcmStream::open(req.clone(),sequence(req),Arc::default(),format()).err().unwrap().to_string().contains("尚未加载"));
}
#[test]
fn selected_response_uses_the_negotiated_processing_rate_instead_of_the_source_rate(){
    use crate::decoder::{FfmpegDecoder,PcmEncoding};
    let source=wav(&(0..512).flat_map(|_|[1_i32<<26,-(1_i32<<25)]).collect::<Vec<_>>(),32);let mut req=request(&source,32,512);req.dsp.enabled=true;
    let a=impulse(&[vec![2.]],48000,0).response(48000).unwrap().clone();let b=impulse(&[vec![-3.]],96000,0).response(96000).unwrap().clone();req.impulse=Some(Arc::new(crate::impulse::ImpulseBank::new(vec![b,a]).unwrap()));
    for (rate,gain) in [(48000,2.),(96000,-3.)]{
        let mut actual=format();actual.sample_rate=rate;actual.exclusive=false;let mut reference=FfmpegDecoder::open(&req.input,0.,rate,2,PcmEncoding::Float64).unwrap();let mut input=vec![0;4096*16];let mut expected=Vec::new();
        loop{let frames=reference.read_frames(&mut input,16).unwrap();if frames==0{break;}for bytes in input[..frames*16].chunks_exact(8){let value=f64::from_le_bytes(bytes.try_into().unwrap())*gain;expected.extend_from_slice(&((value*2147483648.).round().clamp(i32::MIN as f64,i32::MAX as f64) as i32).to_le_bytes());}}
        assert!(!expected.is_empty());let mut stream=PcmStream::open(req.clone(),sequence(req.clone()),Arc::default(),actual).unwrap();assert_eq!(collect(&mut stream),expected);
    }
    let mut missing=format();missing.sample_rate=44100;assert!(PcmStream::open(req.clone(),sequence(req),Arc::default(),missing).err().unwrap().to_string().contains("44100 Hz"));
}
#[test]
fn response_selection_waits_for_actual_output_format_before_rejecting_preloads(){
    let source=wav(&vec![0_i32;1024],32);let mut req=request(&source,32,512);req.dsp.enabled=true;req.impulse=Some(impulse(&[vec![2.]],96000,0));
    let seq=sequence(req.clone());seq.reserve().unwrap().prepare("nominal".into(),req.clone()).unwrap();let mut actual=format();actual.sample_rate=96000;actual.exclusive=false;
    let mut stream=PcmStream::open(req.clone(),seq.clone(),Arc::default(),actual).unwrap();let prepared=seq.reserve().unwrap().prepare("actual".into(),req);
    #[cfg(target_os="macos")] assert!(prepared.unwrap_err().is::<PreloadFormatChange>(),"Mac reopens each source clock rather than pinning following tracks to 96 kHz");
    #[cfg(not(target_os="macos"))] prepared.unwrap();
    assert!(!collect(&mut stream).is_empty());
}
#[test]
fn loaded_response_applies_fade_before_convolution_and_caps_the_public_programme_clock(){
    let samples=(0..65).flat_map(|_|[1<<28,-(1<<27)]).collect::<Vec<_>>();let source=wav(&samples,32);let mut req=request(&source,32,65);req.dsp.enabled=true;req.dsp.preamp_db=6.020599913279624;req.transitions.fade_out_ms=1;
    let mut kernel=vec![0.;65];kernel[0]=1.;kernel[64]=0.5;req.impulse=Some(impulse(&[kernel.clone()],48000,0));
    let control=Arc::new(PlaybackControl::default());let seq=sequence(req.clone());let mut stream=PcmStream::open(req.clone(),seq,control.clone(),format()).unwrap();let actual=collect(&mut stream);
    let mut expected=vec![0.;129*2];
    for frame in 0..65 {for channel in 0..2 {let value=samples[frame*2+channel] as f64/2147483648.*2.*req.transitions.gain(frame as f64/48000.,req.duration);expected[frame*2+channel]+=value;expected[(frame+64)*2+channel]+=value*0.5;}}
    assert_eq!(actual,output_bytes(format(),&expected,|_|1.));
    control.frames_written.store(129,Ordering::Relaxed);control.frames_played.store(129,Ordering::Release);assert_eq!(control.seconds(48000),65./48000.);
}
#[test]
fn loaded_response_requires_actual_rate_mapping_layout_and_budget_but_disabled_dsp_bypasses_it(){
    let source=wav(&[1<<28,-(1<<27)],32);let mut req=request(&source,32,1);req.dsp.enabled=true;
    for response in [impulse(&[vec![1.]],44100,0),impulse(&[vec![1.],vec![1.],vec![1.]],48000,0)] {
        req.impulse=Some(response);assert!(PcmStream::open(req.clone(),sequence(req.clone()),Arc::default(),format()).is_err());
        let queue=sequence(req.clone());queue.configure(format()).unwrap();assert!(queue.reserve().unwrap().prepare("bad".into(),req.clone()).is_err());
    }
    req.impulse=Some(impulse(&[vec![1.],vec![1.]],48000,3));assert!(PcmStream::open(req.clone(),sequence(req.clone()),Arc::default(),format()).is_err());
    let mut known=format();known.channel_mask=3;assert!(PcmStream::open(req.clone(),sequence(req.clone()),Arc::default(),known).is_ok());
    req.impulse=Some(impulse(&[vec![1.;crate::convolution::MAX_COEFFICIENTS]],48000,0));assert!(req.impulse.as_ref().unwrap().validate_output(48000,32,Some(0)).unwrap_err().to_string().contains("budget"));let mut wide=format();wide.channels=32;assert!(PcmStream::open(req.clone(),sequence(req.clone()),Arc::default(),wide).is_err());
    req.dsp.enabled=false;req.impulse=Some(impulse(&[vec![2.]],44100,4));let mut bypass=PcmStream::open(req.clone(),sequence(req),Arc::default(),format()).unwrap();assert_eq!(collect(&mut bypass),[1_i32<<28,-(1_i32<<27)].iter().flat_map(|s|s.to_le_bytes()).collect::<Vec<_>>());
}
#[test]
fn loaded_response_changes_do_not_reuse_a_previous_filters_history(){
    let source=wav(&[1<<28,-(1<<27)],32);let mut first=request(&source,32,1);first.dsp.enabled=true;let mut kernel=vec![0.;65];kernel[0]=1.;kernel[64]=0.5;first.impulse=Some(impulse(&[kernel],48000,0));
    let mut second=first.clone();second.impulse=Some(impulse(&[vec![2.]],48000,0));let seq=sequence(first.clone());seq.reserve().unwrap().prepare("b".into(),second).unwrap();let mut stream=PcmStream::open(first,seq,Arc::default(),format()).unwrap();
    let actual=collect(&mut stream);let expected=[1_i32<<28,-(1_i32<<27),1_i32<<29,-(1_i32<<28)].iter().flat_map(|v|v.to_le_bytes()).collect::<Vec<_>>();assert_eq!(actual,expected);
}
#[test]
fn layout_negotiation_keeps_pcm_preloads_and_checks_deferred_response_layout_before_output(){
    let source=wav(&[1<<28,-(1<<27)],32);let first=request(&source,32,1);let mut second=first.clone();second.dsp.enabled=true;second.impulse=Some(impulse(&[vec![2.],vec![2.]],48000,3));
    for mask in [3,5] {
        let seq=sequence(first.clone());seq.reserve().unwrap().prepare("b".into(),second.clone()).unwrap();let mut actual=format();actual.channel_mask=mask;
        let mut stream=PcmStream::open(first.clone(),seq,Arc::default(),actual).unwrap();assert!(matches!(stream.read(1).unwrap(),Read::Data(_)));
        if mask==3 {assert_eq!(collect(&mut stream),[1_i32<<29,-(1_i32<<28)].iter().flat_map(|s|s.to_le_bytes()).collect::<Vec<_>>());}
        else {assert!(stream.read(1).err().unwrap().to_string().contains("layout"));}
    }
}
#[test]
fn convolution_crosses_real_source_boundaries_and_tail_frames_never_become_programme_frames(){
    let source=wav(&[1<<29,-(1<<28)],32);let quiet=wav(&[0,0,0,0],32);
    let mut first=request(&source,32,1);first.dsp.enabled=true;let mut second=request(&quiet,32,2);second.dsp=first.dsp.clone();
    let seq=sequence(first.clone());seq.reserve().unwrap().prepare("b".into(),second).unwrap();let control=Arc::new(PlaybackControl::default());
    let mut stream=PcmStream::open(first,seq.clone(),control.clone(),format()).unwrap();let mut kernel=vec![0.;65];kernel[0]=1.;kernel[64]=0.5;
    stream.dsp.attach_convolution(48000,vec![kernel]).unwrap();let mut output=Vec::new();let mut ended=Vec::new();
    loop {match stream.read(137).unwrap(){
        Read::Data(chunk)=>{output.extend(chunk.bytes);let written=control.frames_written.fetch_add(chunk.frames as u64,Ordering::Relaxed)+chunk.frames as u64;control.frames_played.store(written,Ordering::Relaxed);ended.extend(seq.heard(&control,false).unwrap().3);},
        Read::Pending=>panic!("prepared sources must be ready"),Read::End=>break,
    }}
    let samples=output.chunks_exact(4).map(|bytes|i32::from_le_bytes(bytes.try_into().unwrap())).collect::<Vec<_>>();
    assert_eq!(samples.len(),67*2);assert_eq!(&samples[..2],&[1<<29,-(1<<28)]);assert_eq!(&samples[64*2..65*2],&[1<<28,-(1<<27)]);
    assert!(samples[2..64*2].iter().all(|s|*s==0));assert!(samples[65*2..].iter().all(|s|*s==0));
    assert_eq!(control.frames_written.load(Ordering::Relaxed),67);assert_eq!(seq.position(&control).unwrap().frames,2);
    ended.extend(seq.heard(&control,true).unwrap().3);assert_eq!(ended.iter().map(|e|(&*e.playback_id,e.frames)).collect::<Vec<_>>(),vec![("a",1),("b",2)]);
    assert!(seq.heard(&control,true).unwrap().3.is_empty());
}
#[test]
fn convolution_tail_obeys_current_mute_volume_and_explicit_stop(){
    let source=wav(&[1<<29,-(1<<28)],32);let mut req=request(&source,32,1);req.dsp.enabled=true;
    let seq=sequence(req.clone());let control=Arc::new(PlaybackControl::default());let mut stream=PcmStream::open(req.clone(),seq,control.clone(),format()).unwrap();
    let mut kernel=vec![0.;65];kernel[0]=1.;kernel[64]=0.5;stream.dsp.attach_convolution(48000,vec![kernel.clone()]).unwrap();
    assert!(matches!(stream.read(1).unwrap(),Read::Data(_)));control.muted.store(true,Ordering::Relaxed);
    let Read::Data(muted)=stream.read(48).unwrap() else{panic!("missing tail");};assert!(muted.bytes.iter().all(|b|*b==0));
    control.muted.store(false,Ordering::Relaxed);control.set_volume(0.25).unwrap();
    let Read::Data(quiet)=stream.read(16).unwrap() else{panic!("missing tail");};
    let samples=quiet.bytes.chunks_exact(4).map(|b|i32::from_le_bytes(b.try_into().unwrap())).collect::<Vec<_>>();assert_eq!(&samples[30..],&[1<<26,-(1<<25)]);
    assert!(matches!(stream.read(1).unwrap(),Read::End));
    let seq=sequence(req.clone());let control=Arc::new(PlaybackControl::default());let mut stopped=PcmStream::open(req,seq,control.clone(),format()).unwrap();
    stopped.dsp.attach_convolution(48000,vec![kernel]).unwrap();assert!(matches!(stopped.read(1).unwrap(),Read::Data(_)));
    control.stopped.store(true,Ordering::Release);assert!(matches!(stopped.read(137).unwrap(),Read::End));
}
#[test]
fn ffmpeg_tracks_join_without_missing_duplicate_or_silent_frames_and_use_the_heard_clock() {
    let first = (0..226).map(|i| i * 101 - 16000).collect::<Vec<_>>();
    let second = (0..1994).map(|i| i * 511 - 500000).collect::<Vec<_>>();
    let third = vec![237; 666];
    let a = wav(&first, 16);
    let b = wav(&second, 24);
    let c = wav(&third, 16);
    let req = request(&a, 16, 113);
    let seq = sequence(req.clone());
    let preload_b = seq.reserve().unwrap();
    let preload_c = seq.reserve().unwrap();
    preload_c.prepare("c".into(), request(&c, 16, 333)).unwrap();
    let control = Arc::new(PlaybackControl::default());
    let mut stream = PcmStream::open(req, seq.clone(), control.clone(), format()).unwrap();
    let mut bytes = Vec::new();
    loop {
        match stream.read(137).unwrap() {
            Read::Data(chunk) => bytes.extend(chunk.bytes),
            Read::Pending => break,
            Read::End => panic!("pending b must not be bypassed by ready c"),
        }
    }
    preload_b.prepare("b".into(), request(&b, 24, 997)).unwrap();
    bytes.extend(collect(&mut stream));
    let expected = first
        .iter()
        .map(|n| n << 16)
        .chain(second.iter().map(|n| n << 8))
        .chain(third.iter().map(|n| n << 16))
        .flat_map(i32::to_le_bytes)
        .collect::<Vec<_>>();
    assert_eq!(bytes, expected);
    control.frames_written.store(1443, Ordering::Relaxed);
    control.frames_played.store(112, Ordering::Relaxed);
    let (id, _, _, ended) = seq.heard(&control, false).unwrap();
    assert_eq!(id, "a");
    assert!(ended.is_empty());
    control.frames_played.store(113, Ordering::Relaxed);
    assert_eq!(seq.position(&control).unwrap().playback_id, "b");
    assert_eq!(seq.position(&control).unwrap().frames, 0);
    let (id, pos, _, ended) = seq.heard(&control, false).unwrap();
    assert_eq!((id.as_str(), pos), ("b", 0.));
    assert_eq!(ended.len(), 1);
    assert_eq!(ended[0].frames, 113);
    assert!(seq.heard(&control, false).unwrap().3.is_empty());
    control.frames_played.store(1110, Ordering::Relaxed);
    let (id, _, _, ended) = seq.heard(&control, false).unwrap();
    assert_eq!(id, "c");
    assert_eq!(ended[0].frames, 997);
    control.frames_played.store(9999, Ordering::Relaxed);
    let (_, pos, _, ended) = seq.heard(&control, true).unwrap();
    assert_eq!(pos, 333. / 48000.);
    assert_eq!(ended[0].frames, 333);
    assert!(seq.heard(&control, true).unwrap().3.is_empty());
}

#[test]
fn native_768khz_continuous_pcm_keeps_integer_samples_and_source_clock() {
    use std::io::{Seek, SeekFrom};
    let rate = 768000_u32;
    let first = (0..514).map(|i| [i32::MIN, i32::MAX, 0, 1, -1, 123456789, -987654321][i % 7]).collect::<Vec<_>>();
    let second = (0..526).map(|i| [i32::MAX, i32::MIN, -1, 1, 0][i % 5]).collect::<Vec<_>>();
    let source = |samples: &[i32]| {
        let mut file = wav(samples, 32);
        file.as_file_mut().seek(SeekFrom::Start(24)).unwrap();
        file.write_all(&rate.to_le_bytes()).unwrap();
        file.write_all(&(rate * 8).to_le_bytes()).unwrap();
        file.flush().unwrap();
        file
    };
    let a = source(&first);
    let b = source(&second);
    let make_request = |file: &tempfile::NamedTempFile, frames| {
        let mut req = request(file, 32, frames);
        req.sample_rate = rate;
        req.duration = Some(frames as f64 / rate as f64);
        req
    };
    let req = make_request(&a, 257);
    let high_rate = SequenceFormat {sample_rate: rate, ..format()};
    let seq = PlaybackSequence::new("a".into(), req.clone(), high_rate).unwrap();
    seq.reserve().unwrap().prepare("b".into(), make_request(&b, 263)).unwrap();
    let control = Arc::new(PlaybackControl::default());
    let mut stream = PcmStream::open(req, seq.clone(), control.clone(), high_rate).unwrap();
    let expected = first.iter().chain(&second).flat_map(|sample| sample.to_le_bytes()).collect::<Vec<_>>();
    assert_eq!(collect(&mut stream), expected);
    assert_eq!(seq.format().unwrap().sample_rate, rate);
    control.frames_written.store(520, Ordering::Relaxed);
    control.frames_played.store(257, Ordering::Relaxed);
    let (id, position, _, ended) = seq.heard(&control, false).unwrap();
    assert_eq!((id.as_str(), position), ("b", 0.));
    assert_eq!(ended[0].frames, 257);
    control.frames_played.store(520, Ordering::Relaxed);
    let (_, position, _, ended) = seq.heard(&control, true).unwrap();
    assert_eq!(position, 263. / rate as f64);
    assert_eq!(ended[0].frames, 263);
}
#[test]
fn queue_limits_format_changes_and_newer_volume_are_preserved() {
    let file = wav(&[1000; 400], 16);
    let req = request(&file, 16, 200);
    let seq = sequence(req.clone());
    let mut holds = Vec::new();
    for _ in 0..8 {
        holds.push(seq.reserve().unwrap());
    }
    assert!(seq.reserve().is_err());
    drop(holds);
    assert!(seq.reserve().is_ok());
    let mut incompatible = req.clone();
    incompatible.sample_rate = 44100;
    assert!(seq
        .reserve()
        .unwrap()
        .prepare("wrong".into(), incompatible)
        .is_err());
    seq.cancel_next();
    let mut next = req.clone();
    next.volume = 0.9;
    seq.reserve().unwrap().prepare("b".into(), next).unwrap();
    let control = Arc::new(PlaybackControl::default());
    control.set_volume(0.25).unwrap();
    let mut stream = PcmStream::open(req, seq, control.clone(), format()).unwrap();
    let bytes = collect(&mut stream);
    assert_eq!(bytes.len(), 400 * 8);
    for sample in bytes.chunks_exact(4) {
        assert_eq!(
            i32::from_le_bytes(sample.try_into().unwrap()),
            1000 * 65536 / 4
        );
    }
    assert_eq!(control.volume_setting(), 0.25);
}

#[test]
fn native_format_transition_ends_only_the_heard_source_without_resampling_or_false_failure() {
    let samples = [0, 1, -1, 8388607, -8388608, 1234567];
    let file = wav(&samples, 24);
    let req = request(&file, 24, 3);
    let actual = SequenceFormat {bit_depth: 24, ..format()};
    for change in 0..3 {
        let seq = PlaybackSequence::new("a".into(), req.clone(), actual).unwrap();
        let mut next = req.clone();
        match change {0 => next.sample_rate = 768000, 1 => next.channels = 1, _ => next.source_bit_depth = 32}
        let error = seq.reserve().unwrap().prepare("b".into(), next).unwrap_err();
        assert!(error.is::<PreloadFormatChange>());
        let control = Arc::new(PlaybackControl::default());
        let mut stream = PcmStream::open(req.clone(), seq.clone(), control.clone(), actual).unwrap();
        let expected = samples.iter().flat_map(|sample| (sample << 8).to_le_bytes()).collect::<Vec<_>>();
        assert_eq!(collect(&mut stream), expected);
        control.frames_written.store(3, Ordering::Relaxed);
        control.frames_played.store(3, Ordering::Relaxed);
        let (id, position, _, ended) = seq.heard(&control, true).unwrap();
        assert_eq!((id.as_str(), position), ("a", 3. / 48000.));
        assert_eq!(ended.len(), 1);
        assert_eq!((ended[0].playback_id.as_str(), ended[0].frames), ("a", 3));
    }
    let mut corrupt = req.clone();
    corrupt.input = file.path().with_extension("missing.wav").to_string_lossy().into();
    let seq = PlaybackSequence::new("a".into(), req, actual).unwrap();
    assert!(!seq.reserve().unwrap().prepare("bad".into(), corrupt).unwrap_err().is::<PreloadFormatChange>());
}
#[test]
fn continuous_dsp_keeps_filter_state_across_track_boundaries() {
    let samples = (0..4096)
        .map(|i| ((i as f64 * 0.1).sin() * 10000.) as i32)
        .collect::<Vec<_>>();
    let a = wav(&samples[..1246], 16);
    let b = wav(&samples[1246..], 16);
    let all = wav(&samples, 16);
    let mut first = request(&a, 16, 623);
    first.dsp.enabled = true;
    first.dsp.bands = vec![crate::dsp::EqBand {
        kind: crate::dsp::EqFilter::Peak,
        enabled: true,
        frequency: 2000.,
        gain_db: 6.,
        q: 1.,
        slope_db_oct: None,
    }];
    let mut second = request(&b, 16, 1425);
    second.dsp = first.dsp.clone();
    let seq = sequence(first.clone());
    seq.reserve().unwrap().prepare("b".into(), second).unwrap();
    let mut split = PcmStream::open(first.clone(), seq, Arc::default(), format()).unwrap();
    let joined = collect(&mut split);
    first.input = all.path().to_string_lossy().into();
    first.duration = Some(2048. / 48000.);
    let mut single =
        PcmStream::open(first.clone(), sequence(first), Arc::default(), format()).unwrap();
    assert_eq!(joined, collect(&mut single));
}
#[test]
fn interrupted_http_seek_never_promotes_the_preloaded_song() {
    use std::{io::Read as _, net::TcpListener};
    let first = wav(&vec![1000; 48000 * 4 * 2],16);
    let next = wav(&vec![2000; 48000 * 2],16);
    let mut bytes = std::fs::read(first.path()).unwrap();
    let length = bytes.len(); bytes.truncate(44 + 48000 * 2 * 2 * 2);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = [0;4096]; socket.read(&mut request).unwrap();
        write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: audio/wav\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n").unwrap();
        socket.write_all(&bytes).unwrap();
    });
    let mut req = request(&first,16,48000*4);
    req.input = format!("http://{address}/source.wav"); req.position = 0.5;
    let seq = sequence(req.clone());
    seq.reserve().unwrap().prepare("next".into(),request(&next,16,48000)).unwrap();
    let control = Arc::new(PlaybackControl::default());
    let mut stream = PcmStream::open(req,seq.clone(),control.clone(),format()).unwrap();
    let error = loop {
        match stream.read(2048) {
            Ok(Read::Data(_)) => {},
            Ok(_) => panic!("an interrupted source must not finish normally"),
            Err(error) => break error,
        }
    };
    server.join().unwrap();
    assert!(error.is::<crate::decoder::DecodeFailure>());
    assert_eq!(seq.heard(&control,false).unwrap().0,"a");
}
#[test]
fn failed_source_validation_never_advances_to_the_prepared_track() {
    let file = wav(&[1000; 200], 16);
    let req = request(&file, 16, 100);
    let check: CompletionCheck =
        Arc::new(|_| Err(crate::decoder::DecodeFailure("truncated upstream".into()).into()));
    let seq = PlaybackSequence::checked("a".into(), req.clone(), format(), Some(check)).unwrap();
    seq.reserve()
        .unwrap()
        .prepare("b".into(), req.clone())
        .unwrap();
    let control = Arc::new(PlaybackControl::default());
    let mut stream = PcmStream::open(req, seq.clone(), control.clone(), format()).unwrap();
    assert!(matches!(stream.read(137).unwrap(), Read::Data(_)));
    assert!(stream
        .read(137)
        .err()
        .unwrap()
        .is::<crate::decoder::DecodeFailure>());
    assert_eq!(seq.heard(&control, false).unwrap().0, "a");
}
#[test]
fn queue_cancellation_terminates_a_real_blocked_preload_and_keeps_new_generation() {
    use std::net::TcpListener;
    let file = wav(&[1; 200], 16);
    let req = request(&file, 16, 100);
    let seq = sequence(req.clone());
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = server.local_addr().unwrap();
    let (accepted, connected) = mpsc::channel();
    let (release, waiting) = mpsc::channel();
    let server_thread = std::thread::spawn(move || {
        let (socket, _) = server.accept().unwrap();
        accepted.send(()).unwrap();
        let _ = waiting.recv_timeout(Duration::from_secs(5));
        drop(socket);
    });
    let mut blocked = req.clone();
    blocked.input = format!("http://{address}/stalled.wav");
    let preload = seq.reserve().unwrap();
    let (done, finished) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        done.send(
            preload
                .prepare("b".into(), blocked)
                .err()
                .unwrap()
                .is::<PreloadCancelled>(),
        )
        .unwrap();
    });
    connected.recv_timeout(Duration::from_secs(5)).unwrap();
    seq.cancel_next();
    seq.reserve()
        .unwrap()
        .prepare("c".into(), req.clone())
        .unwrap();
    let cancelled = finished.recv_timeout(Duration::from_secs(1));
    let _ = release.send(());
    worker.join().unwrap();
    server_thread.join().unwrap();
    assert!(cancelled.unwrap());
    let mut stream = PcmStream::open(req, seq, Arc::default(), format()).unwrap();
    assert_eq!(collect(&mut stream).len(), 200 * 8);
}
#[test]
fn negotiated_format_discards_incompatible_primers_and_sequence_is_single_use() {
    let file = wav(&[1; 2000], 16);
    let req = request(&file, 16, 1000);
    let mut shared = format();
    shared.exclusive = false;
    let seq = PlaybackSequence::new("a".into(), req.clone(), shared).unwrap();
    seq.reserve()
        .unwrap()
        .prepare("old-format".into(), req.clone())
        .unwrap();
    shared.sample_rate = 44100;
    let mut stream = PcmStream::open(req.clone(), seq.clone(), Arc::default(), shared).unwrap();
    assert_eq!(seq.format().unwrap().sample_rate, 44100);
    assert!(PcmStream::open(req, seq.clone(), Arc::default(), shared).is_err());
    let _ = collect(&mut stream);
    assert!(seq.reserve().is_err());
}
#[test]
fn failed_preload_ends_the_run_before_that_track_and_never_skips_to_later_ready_audio() {
    let file = wav(&[1; 200], 16);
    let req = request(&file, 16, 100);
    let seq = sequence(req.clone());
    let b = seq.reserve().unwrap();
    seq.reserve()
        .unwrap()
        .prepare("c".into(), req.clone())
        .unwrap();
    let mut missing = req.clone();
    missing.input = "missing-gapless-track.wav".into();
    assert!(b.prepare("b".into(), missing).is_err());
    let control = Arc::new(PlaybackControl::default());
    let mut stream = PcmStream::open(req, seq.clone(), control.clone(), format()).unwrap();
    assert_eq!(collect(&mut stream).len(), 100 * 8);
    control.frames_written.store(100, Ordering::Relaxed);
    control.frames_played.store(100, Ordering::Relaxed);
    let (id, _, _, ended) = seq.heard(&control, true).unwrap();
    assert_eq!(id, "a");
    assert_eq!(ended.len(), 1);
    assert_eq!(ended[0].playback_id, "a");
}
#[test]
fn rolling_queue_waits_for_more_sources_and_can_end_a_full_packet_window_without_deadlock() {
    let file = wav(&[1; 200], 16);
    let req = request(&file, 16, 100);
    let seq = sequence(req.clone());
    seq.queue_complete(false).unwrap();
    let mut stream = PcmStream::open(req.clone(), seq.clone(), Arc::default(), format()).unwrap();
    let first = match stream.read(200).unwrap() {
        Read::Data(chunk) => chunk.bytes,
        _ => panic!("missing first source"),
    };
    assert!(matches!(stream.read(200).unwrap(), Read::Pending));
    seq.reserve()
        .unwrap()
        .prepare("b".into(), req.clone())
        .unwrap();
    seq.queue_complete(true).unwrap();
    let second = collect(&mut stream);
    assert_eq!(first.len() + second.len(), 200 * 8);
    let seq = sequence(req.clone());
    seq.queue_complete(false).unwrap();
    let mut stream = PcmStream::open(req, seq.clone(), Arc::default(), format()).unwrap();
    assert!(seq.finish_window_if_empty());
    assert!(!seq.finish_window_if_empty());
    assert_eq!(collect(&mut stream).len(), 100 * 8);
}
#[test]
fn negotiated_startup_reserves_in_actual_format_and_external_probe_failure_is_a_barrier() {
    let file = wav(&[1; 2000], 16);
    let req = request(&file, 16, 1000);
    let mut desired = format();
    desired.exclusive = false;
    let seq = PlaybackSequence::new("a".into(), req.clone(), desired).unwrap();
    let next = req.clone();
    seq.on_configured(move |sequence| {
        assert_eq!(sequence.format()?.sample_rate, 44100);
        sequence.reserve()?.fail();
        let prepared=sequence.reserve()?.prepare("c".into(), next);
        #[cfg(target_os="macos")] assert!(prepared.unwrap_err().is::<PreloadFormatChange>());
        #[cfg(not(target_os="macos"))] prepared?;
        Ok(())
    })
    .unwrap();
    desired.sample_rate = 44100;
    let mut stream = PcmStream::open(req, seq.clone(), Arc::default(), desired).unwrap();
    // Exactly one source converted to 44.1 kHz. Ready C cannot bypass failed probe B.
    let actual = collect(&mut stream);
    assert_eq!(actual.len(), 919 * 8);
    assert!(seq.on_configured(|_| Ok(())).is_err());
}
#[test]
fn flac_and_tagged_mp3_keep_exact_program_frame_counts_at_the_sequence_boundary() {
    let frames = 12347;
    let samples = (0..frames * 2)
        .map(|i| ((i as f64 * 0.071).sin() * 12000.) as i32)
        .collect::<Vec<_>>();
    let source = wav(&samples, 16);
    let flac = tempfile::Builder::new().suffix(".flac").tempfile().unwrap();
    let mp3 = tempfile::Builder::new().suffix(".mp3").tempfile().unwrap();
    for (file, codec) in [(&flac, "flac"), (&mp3, "libmp3lame")] {
        let result = std::process::Command::new(crate::tool_binary("ffmpeg"))
            .args([
                "-v",
                "error",
                "-y",
                "-i",
                source.path().to_str().unwrap(),
                "-c:a",
                codec,
            ])
            .arg(file.path())
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let first = request(&flac, 16, frames);
    let mut second = request(&mp3, 16, frames);
    second.source_integer_lossless = false;
    let seq = sequence(first.clone());
    seq.reserve()
        .unwrap()
        .prepare("b".into(), second.clone())
        .unwrap();
    let mut expected = Vec::new();
    for req in [&first, &second] {
        let mut decoder =
            FfmpegDecoder::open(&req.input, 0., 48000, 2, PcmEncoding::Signed32).unwrap();
        let mut bytes = [0; 8192];
        let mut decoded = 0;
        loop {
            let n = decoder.read_frames(&mut bytes, 8).unwrap();
            if n == 0 {
                break;
            }
            decoded += n;
            expected.extend_from_slice(&bytes[..n * 8]);
        }
        assert_eq!(decoded, frames, "codec delay/padding was not trimmed");
    }
    let mut stream = PcmStream::open(first, seq, Arc::default(), format()).unwrap();
    let actual = collect(&mut stream);
    assert_eq!(actual, expected);
    assert_eq!(actual.len(), frames * 2 * 8);
}

#[test]
fn final_checkpoint_recovers_unreported_boundaries_without_consuming_events_or_packet_tail() {
    let file=tempfile::NamedTempFile::new().unwrap();
    let mut first=request(&file,32,1000);first.position=0.25;first.duration=None;
    let seq=PlaybackSequence::new("a".into(),first.clone(),format()).unwrap();
    let mut next=first.clone();next.position=0.;
    boundary(&seq,"b".into(),next.clone(),101);boundary(&seq,"c".into(),next,201);
    seq.state.lock().unwrap().programme_end=Some(251);
    let control=PlaybackControl::default();
    control.frames_written.store(280,Ordering::Release);control.frames_played.store(260,Ordering::Release);
    for _ in 0..2 {
        let(current,ended)=seq.checkpoint(&control).unwrap();assert_eq!(current.playback_id,"c");assert_eq!(current.frames,50);
        assert_eq!(ended.iter().map(|source|(source.playback_id.as_str(),source.frames)).collect::<Vec<_>>(),[("a",101),("b",100)]);
        assert_eq!(ended[0].position,0.25+101./48000.);
    }
    control.frames_written.store(0,Ordering::Release);control.frames_played.store(0,Ordering::Release);
    let(current,ended)=seq.checkpoint(&control).unwrap();assert_eq!(current.playback_id,"a");assert_eq!(current.frames,0);assert!(ended.is_empty());
    control.frames_written.store(280,Ordering::Release);control.frames_played.store(260,Ordering::Release);
    assert_eq!(seq.heard(&control,false).unwrap().3.len(),2);assert!(seq.checkpoint(&control).unwrap().1.is_empty());
}

#[test]
fn programme_clock_position_uses_the_paired_frame_and_rejects_retired_boundaries() {
    let file=wav(&[0;64],32);let mut first=request(&file,32,32);first.sample_rate=768000;first.position=0.25;first.duration=None;
    let mut high=format();high.sample_rate=768000;let sequence=PlaybackSequence::new("old".into(),first.clone(),high).unwrap();
    let mut next=first;next.position=1.25;boundary(&sequence,"new".into(),next,32);let control=PlaybackControl::default();control.frames_written.store(64,Ordering::Release);control.frames_played.store(60,Ordering::Release);
    let sample=sequence.position_at_frame(&control,40).unwrap();assert_eq!(sample.playback_id,"new");assert_eq!(sample.frames,8);assert_eq!(sample.position,1.25+8./768000.);assert_eq!(sequence.position(&control).unwrap().frames,28);
    assert_eq!(sequence.position_at_frame(&control,16).unwrap().playback_id,"old");assert!(sequence.position_at_frame(&control,65).is_err());
    sequence.heard(&control,false).unwrap();assert!(sequence.position_at_frame(&control,16).is_err());sequence.state.lock().unwrap().programme_end=Some(50);assert_eq!(sequence.position_at_frame(&control,64).unwrap().frames,18);
}

#[test]
fn dsd_pcm_decode_keeps_container_identity_pcm_counts_and_cancels_cleanly() {
    use crate::dsd::tests::{samples,dsf,dff};
    let raw=samples(35280); // 100 ms of synthetic non-silent DSD64; no device is opened.
    let mut decoded=Vec::new();
    for (extension,bytes) in [(".dsf",dsf(&raw,282240,true)),(".dff",dff(&raw))] {
        let mut file=tempfile::Builder::new().suffix(extension).tempfile().unwrap();file.write_all(&bytes).unwrap();file.flush().unwrap();
        let info=crate::probe::probe(file.path()).unwrap();
        assert_eq!(info.dsd_pcm_rate(),176400);
        let request=PlaybackRequest{dsd_pcm:Some(info.sample_rate),source_integer_lossless:false,source_bit_depth:32,sample_rate:info.dsd_pcm_rate(),duration:info.duration,..request(&file,32,17640)};
        assert!(!request.is_dsd(),"a decoded PCM stream is never a raw DSD carrier");
        let config=OutputConfig{limits:OutputLimits{max_sample_rate:Some(96000),volume_control:VolumeControl::Fixed,..Default::default()},..Default::default()};
        let format=config.desired_format(&request).unwrap();assert_eq!(format.sample_rate,88200);
        let sequence=PlaybackSequence::new("dsd-pcm".into(),request.clone(),format).unwrap();sequence.queue_complete(true).unwrap();
        let control=Arc::new(PlaybackControl::default());config.prepare(&request,&control,&sequence).unwrap();
        let mut stream=PcmStream::open(request.clone(),sequence,control.clone(),format).unwrap();let mut pcm=Vec::new();
        loop{match stream.read(137).unwrap(){Read::Data(chunk)=>pcm.extend(chunk.bytes),Read::End=>break,Read::Pending=>panic!("complete local source cannot need a queue window")}}
        assert_eq!(pcm.len(),8820*8,"DSD programme duration must not include DSF block padding");
        assert!(pcm.iter().any(|v|*v!=0));decoded.push(pcm);
        let wrong=PlaybackRequest{dsd_pcm:Some(5644800),..request.clone()};
        let sequence=PlaybackSequence::new("wrong".into(),wrong.clone(),format).unwrap();
        assert!(PcmStream::open(wrong,sequence,Arc::new(PlaybackControl::default()),format).is_err());
        control.stop();drop(stream);
    }
    // DSF planar/LSB and DFF interleaved/MSB contain exactly the same one-bit signal.
    assert_eq!(decoded[0],decoded[1]);
}

#[test]
fn queue_refresh_preserves_every_current_pcm_sample_and_replaces_unsubmitted_audio() {
    let a=wav(&[1000;400],24);let b=wav(&[2000;400],24);let c=wav(&[-3000;400],24);
    let request_a=request(&a,24,200);let seq=sequence(request_a.clone());
    seq.reserve().unwrap().prepare("b".into(),request(&b,24,200)).unwrap();
    let control=Arc::new(PlaybackControl::default());
    let mut stream=PcmStream::open(request_a,seq.clone(),control.clone(),format()).unwrap();
    let Read::Data(first)=stream.read(64).unwrap() else {panic!("current PCM missing")};
    let mut actual=first.bytes;
    assert_eq!(seq.refresh_queue().unwrap(),vec!["a"]);
    seq.reserve().unwrap().prepare("c".into(),request(&c,24,200)).unwrap();seq.queue_complete(true).unwrap();
    actual.extend(collect(&mut stream));
    let expected:Vec<u8>=[1000i32,-3000].into_iter().flat_map(|value|std::iter::repeat_n(value<<8,400).flat_map(i32::to_le_bytes)).collect();
    assert_eq!(actual,expected);assert!(!control.stopped.load(Ordering::Acquire));
}
#[test]
fn queue_refresh_preserves_an_admitted_boundary_without_replaying_or_truncating_it() {
    let a=wav(&[1000;400],24);let b=wav(&[2000;400],24);let c=wav(&[3000;400],24);let d=wav(&[-4000;400],24);
    let request_a=request(&a,24,200);let seq=sequence(request_a.clone());
    seq.reserve().unwrap().prepare("b".into(),request(&b,24,200)).unwrap();
    seq.reserve().unwrap().prepare("c".into(),request(&c,24,200)).unwrap();
    let control=Arc::new(PlaybackControl::default());
    let mut stream=PcmStream::open(request_a,seq.clone(),control.clone(),format()).unwrap();
    let mut actual=Vec::new();
    for size in [200,32] {let Read::Data(chunk)=stream.read(size).unwrap() else {panic!("admitted PCM missing")};actual.extend(chunk.bytes);}
    assert_eq!(seq.refresh_queue().unwrap(),vec!["a","b"]);
    seq.reserve().unwrap().prepare("d".into(),request(&d,24,200)).unwrap();seq.queue_complete(true).unwrap();
    actual.extend(collect(&mut stream));
    let expected:Vec<u8>=[1000i32,2000,-4000].into_iter().flat_map(|value|std::iter::repeat_n(value<<8,400).flat_map(i32::to_le_bytes)).collect();
    assert_eq!(actual,expected);assert!(!control.stopped.load(Ordering::Acquire));
}
