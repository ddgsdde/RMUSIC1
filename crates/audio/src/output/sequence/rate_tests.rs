use super::*;
use crate::adaptive::Converter;
use std::io::Write;

fn f64_source(rate:u32,values:&[f64])->tempfile::NamedTempFile {
    let mut file=tempfile::Builder::new().suffix(".wav").tempfile().unwrap();let size=values.len() as u32*8;
    for bytes in [b"RIFF".as_slice(),&(36+size).to_le_bytes(),b"WAVEfmt ",&16u32.to_le_bytes(),&3u16.to_le_bytes(),&2u16.to_le_bytes(),&rate.to_le_bytes(),&(rate*16).to_le_bytes(),&16u16.to_le_bytes(),&64u16.to_le_bytes(),b"data",&size.to_le_bytes()] {file.write_all(bytes).unwrap();}
    for value in values {file.write_all(&value.to_le_bytes()).unwrap();}file.flush().unwrap();file
}
fn reference(rate:u32,input:&[f64],ppm:f64)->(Vec<u8>,Converter) {
    let mut converter=Converter::new(rate,2).unwrap();converter.set_ppm(ppm).unwrap();let mut output=vec![0.;converter.output_frames_max()*2];let mut consumed=0;let mut bytes=Vec::new();
    loop {
        let needed=converter.input_frames_next()*2;let remaining=input.len()-consumed;
        let block=if remaining>=needed {converter.process(&input[consumed..consumed+needed],&mut output).unwrap()}else{converter.finish(&input[consumed..],&mut output).unwrap()};
        consumed+=block.input_frames*2;bytes.extend(output[..block.output_frames*2].iter().flat_map(|value|value.clamp(-1.,1.).to_le_bytes()));
        if remaining<needed {let mut last=block;while !last.ended {last=converter.finish(&[],&mut output).unwrap();bytes.extend(output[..last.output_frames*2].iter().flat_map(|value|value.clamp(-1.,1.).to_le_bytes()));}break;}
    }(bytes,converter)
}

#[test]
fn adaptive_stream_ffmpeg_f64_768k_queue_matches_kernel_and_counts_actual_heard_output() {
    let rate=768000;let sizes=[4097usize,22003,1];let ids=["a","b","c"];
    let values=sizes.iter().enumerate().map(|(track,&frames)|(0..frames).flat_map(|n|{let value=if track==0 {0.25}else if track==1 {-0.125}else{0.5};[value+(n%7) as f64/1024.,value+2f64.powi(-31)+(n%7) as f64/1024.]}).collect::<Vec<_>>()).collect::<Vec<_>>();
    let files=values.iter().map(|values|f64_source(rate,values)).collect::<Vec<_>>();let input=values.concat();
    for ppm in [-500.,0.,500.] {
        let requests=files.iter().zip(sizes).map(|(file,frames)|PlaybackRequest{sample_rate:rate,source_integer_lossless:false,duration:Some(frames as f64/rate as f64),..request(file,64,frames)}).collect::<Vec<_>>();
        let format=SequenceFormat{sample_rate:rate,float_output:true,bit_depth:64,..format()};let sequence=PlaybackSequence::new("a".into(),requests[0].clone(),format).unwrap();sequence.enable_rate_adjustment(ppm).unwrap();
        for n in 1..3 {sequence.reserve().unwrap().prepare(ids[n].into(),requests[n].clone()).unwrap();}
        let control=Arc::new(PlaybackControl::default());control.output_sample_rate.store(rate,Ordering::Relaxed);let mut stream=PcmStream::open(requests[0].clone(),sequence.clone(),control.clone(),format).unwrap();
        let (expected,reference)=reference(rate,&input,ppm);let source_boundaries=[0,sizes[0] as u64,(sizes[0]+sizes[1]) as u64,sizes.iter().sum::<usize>() as u64];
        let boundaries=source_boundaries.map(|source|reference.output_at_source(source).unwrap().unwrap());let mut actual=Vec::new();let mut ended=Vec::new();
        loop {match stream.read(137).unwrap() {
            Read::Data(chunk)=>{
                assert!(chunk.frames<=137);actual.extend(chunk.bytes);let written=control.frames_written.fetch_add(chunk.frames as u64,Ordering::Release)+chunk.frames as u64;control.frames_played.store(written,Ordering::Release);
                let ((id,position,_,done),frames)=sequence.heard_with_frames(&control,false).unwrap();ended.extend(done);
                let track=(0..3).rev().find(|&n|boundaries[n]<=written&&boundaries[n]<written).unwrap();assert_eq!(id,ids[track]);assert_eq!(frames,written.min(boundaries[track+1])-boundaries[track]);
                let source=reference.position_at_output(written).unwrap();let source_position=(source.frame as f64+source.fraction).min(source_boundaries[track+1] as f64)-source_boundaries[track] as f64;
                let clock=sequence.source_clock_at_frame(&control,written).unwrap();assert_eq!(clock.position.playback_id,id);assert_eq!(clock.position.frames,frames);assert_eq!(clock.source_frame,source.frame);assert_eq!(clock.source_fraction,source.fraction);assert_eq!(clock.source_per_output,reference.source_per_output_at(written).unwrap());assert!(sequence.source_clock_at_frame(&control,written+1).is_err());
                assert!((position-source_position/rate as f64).abs()<1e-14,"ppm={ppm}, heard={written}, track={id}, position={position}, expected={}, source={source:?}, programme_end={}",source_position/rate as f64,control.programme_end.load(Ordering::Acquire));assert!(sequence.position_at_frame(&control,written+1).is_err());
                control.paused.store(true,Ordering::Relaxed);let stopped=sequence.position(&control).unwrap();assert_eq!(stopped.frames,frames);assert_eq!(stopped.position,position);control.paused.store(false,Ordering::Relaxed);
            },Read::Pending=>panic!("preloaded PCM must be ready"),Read::End=>break,
        }}
        ended.extend(sequence.heard(&control,true).unwrap().3);assert!(sequence.heard(&control,true).unwrap().3.is_empty());
        assert!(actual==expected,"PCM must match one continuous f64 conversion across all source boundaries, ppm={ppm}");
        assert_eq!(ended.len(),3);for n in 0..3 {assert_eq!(ended[n].playback_id,ids[n]);assert_eq!(ended[n].frames,boundaries[n+1]-boundaries[n]);assert!((ended[n].position-sizes[n] as f64/rate as f64).abs()<1e-14);}
        let total=actual.len() as u64/16;assert_eq!(ended.iter().map(|track|track.frames).sum::<u64>(),total);assert_eq!(control.programme_end.load(Ordering::Acquire),total);assert_eq!(control.seconds(rate),total as f64/rate as f64);
        if ppm!=0. {assert_ne!(total,source_boundaries[3]);}
        assert!(sequence.set_rate_ppm(0.).is_err());let mut path=SignalPath{output:"memory".into(),sample_rate:rate,channels:2,exclusive:true,bit_depth:64,sample_type:"float64".into(),mixer_sample_rate:None,bit_perfect_eligible:true,processing:Vec::new()};sequence.mark_rate_adjustment(&mut path);assert!(!path.bit_perfect_eligible);
        println!("ADAPTIVE_STREAM_QA={}",serde_json::json!({"rate":rate,"ppm":ppm,"source_frames":source_boundaries[3],"output_frames":total,"per_track_output":ended.iter().map(|track|track.frames).collect::<Vec<_>>(),"bytes_match":true,"physical_device":false}));
    }
}

#[test]
fn adaptive_stream_zero_heard_frames_keep_each_source_base_position() {
    let rate=96000;let a=f64_source(rate,&vec![0.;2000]);let b=f64_source(rate,&vec![0.;8192]);
    let req=PlaybackRequest{sample_rate:rate,source_integer_lossless:false,..request(&a,64,1000)};
    let format=SequenceFormat{sample_rate:rate,float_output:true,bit_depth:64,..format()};
    let sequence=PlaybackSequence::new("a".into(),req.clone(),format).unwrap();sequence.enable_rate_adjustment(0.).unwrap();
    sequence.reserve().unwrap().prepare("b".into(),PlaybackRequest{sample_rate:rate,source_integer_lossless:false,..request(&b,64,4096)}).unwrap();
    let control=Arc::new(PlaybackControl::default());let mut stream=PcmStream::open(req,sequence.clone(),control.clone(),format).unwrap();
    loop {match stream.read(137).unwrap() {Read::Data(chunk)=>{control.frames_written.fetch_add(chunk.frames as u64,Ordering::Release);},Read::End=>break,Read::Pending=>panic!("both sources are prepared")}}
    let boundary=sequence.state.lock().unwrap().history[1].output_start.unwrap();
    for played in [0,boundary] {
        let position=sequence.position_at_frame(&control,played).unwrap();
        assert_eq!(position.frames,0);assert_eq!(position.position,0.,"fractional resampler phase must not move a source before its first heard frame");
        assert_eq!(position.playback_id,if played==0 {"a"}else{"b"});
    }
}

#[test]
fn adaptive_stream_pending_is_not_eof_and_stop_does_not_submit_buffered_pcm() {
    let first=(0..100).flat_map(|_|[0.25,0.25+2f64.powi(-31)]).collect::<Vec<_>>();let next=vec![-0.125;4096];let a=f64_source(48000,&first);let b=f64_source(48000,&next);
    let req=PlaybackRequest{source_integer_lossless:false,..request(&a,64,100)};let format=SequenceFormat{float_output:true,bit_depth:64,..format()};
    let sequence=PlaybackSequence::new("a".into(),req.clone(),format).unwrap();sequence.enable_rate_adjustment(350.).unwrap();sequence.queue_complete(false).unwrap();
    let control=Arc::new(PlaybackControl::default());let mut stream=PcmStream::open(req,sequence.clone(),control.clone(),format).unwrap();
    assert!(matches!(stream.read(137).unwrap(),Read::Pending));assert_eq!(control.frames_written.load(Ordering::Relaxed),0);assert_eq!(sequence.position(&control).unwrap().position,0.);
    sequence.reserve().unwrap().prepare("b".into(),PlaybackRequest{source_integer_lossless:false,..request(&b,64,2048)}).unwrap();sequence.queue_complete(true).unwrap();
    let (expected,_)=reference(48000,&[first,next].concat(),350.);let actual=collect(&mut stream);assert!(actual==expected,"partial source input must survive a pending next-source window without zero padding");
    assert_eq!(control.frames_played.load(Ordering::Relaxed),0);assert_eq!(sequence.position(&control).unwrap().frames,0);
    control.stop();assert!(matches!(stream.read(137).unwrap(),Read::End));assert_eq!(control.frames_written.load(Ordering::Relaxed),0);
}

#[test]
fn adaptive_stream_opt_in_rejects_dsd_late_preparation_and_nonfinite_rates() {
    let source=f64_source(48000,&[0.25;4096]);let req=PlaybackRequest{source_integer_lossless:false,..request(&source,64,2048)};let format=SequenceFormat{float_output:true,bit_depth:64,..format()};
    let sequence=PlaybackSequence::new("a".into(),req.clone(),format).unwrap();for ppm in [f64::NAN,f64::INFINITY,500.001] {assert!(sequence.enable_rate_adjustment(ppm).is_err());assert!(!sequence.rate_adjustment_active());}
    sequence.enable_rate_adjustment(0.).unwrap();let mut stream=PcmStream::open(req.clone(),sequence.clone(),Arc::default(),format).unwrap();assert!(sequence.enable_rate_adjustment(0.).is_err());sequence.set_rate_ppm(500.).unwrap();assert!(matches!(stream.read(137).unwrap(),Read::Data(_)));
    let raw=PlaybackRequest{source_bit_depth:1,..req.clone()};let dsd=PlaybackSequence::new("raw".into(),raw,format).unwrap();assert!(dsd.enable_rate_adjustment(0.).is_err());
    let prepared=PlaybackSequence::new("a".into(),req.clone(),format).unwrap();let reservation=prepared.reserve().unwrap();assert!(prepared.enable_rate_adjustment(0.).is_err());drop(reservation);
}

#[test]
fn adaptive_stream_integer_output_quantizes_once_and_retires_heard_history() {
    let rate=48000;let length=rate as usize*3;let values=(0..length).flat_map(|n|{let value=0.25+(n%13) as f64*2f64.powi(-33);[value,-value]}).collect::<Vec<_>>();let source=f64_source(rate,&values);
    let request=PlaybackRequest{source_integer_lossless:false,..request(&source,64,length)};let sequence=PlaybackSequence::new("a".into(),request.clone(),format()).unwrap();sequence.enable_rate_adjustment(400.).unwrap();
    let control=Arc::new(PlaybackControl::default());let mut stream=PcmStream::open(request,sequence.clone(),control.clone(),format()).unwrap();
    let (reference,_)=reference(rate,&values,400.);let expected=reference.chunks_exact(8).flat_map(|bytes|((f64::from_le_bytes(bytes.try_into().unwrap())*2147483648.).round().clamp(i32::MIN as f64,i32::MAX as f64) as i32).to_le_bytes()).collect::<Vec<_>>();let mut actual=Vec::new();
    loop {match stream.read(997).unwrap() {
        Read::Data(chunk)=>{actual.extend(chunk.bytes);let written=control.frames_written.fetch_add(chunk.frames as u64,Ordering::Release)+chunk.frames as u64;control.frames_played.store(written,Ordering::Release);sequence.heard(&control,false).unwrap();},
        Read::Pending=>panic!("single source cannot await a reservation"),Read::End=>break,
    }}
    assert!(actual==expected,"integer conversion must follow the f64 resampler without intermediate quantization");assert!(sequence.position_at_frame(&control,0).is_err(),"history older than two heard seconds must retire");
    let ended=sequence.heard(&control,true).unwrap().3;assert_eq!(ended.len(),1);assert_eq!(ended[0].frames,actual.len() as u64/8);assert_eq!(ended[0].position,3.);
}

#[test]
fn adaptive_stream_rate_update_during_decoder_fill_keeps_all_real_pcm() {
    let first=vec![0.25;200];let next=vec![-0.125;4096];let a=f64_source(48000,&first);let b=f64_source(48000,&next);let format=SequenceFormat{float_output:true,bit_depth:64,..format()};
    let req=PlaybackRequest{source_integer_lossless:false,..request(&a,64,100)};let sequence=PlaybackSequence::new("a".into(),req.clone(),format).unwrap();sequence.enable_rate_adjustment(500.).unwrap();
    let update=sequence.clone();sequence.state.lock().unwrap().initial_check=Some(Arc::new(move |_|update.set_rate_ppm(-500.)));
    sequence.reserve().unwrap().prepare("b".into(),PlaybackRequest{source_integer_lossless:false,..request(&b,64,2048)}).unwrap();
    let mut stream=PcmStream::open(req,sequence,Arc::default(),format).unwrap();let actual=collect(&mut stream);let (expected,_)=reference(48000,&[first,next].concat(),-500.);
    assert!(actual==expected,"a rate update while the decoder fills a block must not discard real input or pad a short read");
}
