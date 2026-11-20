use super::*;
use std::{alloc::{GlobalAlloc,Layout,System},cell::Cell};

struct CountAlloc;
thread_local! {static TRACK:Cell<bool>=const{Cell::new(false)};static ALLOCS:Cell<usize>=const{Cell::new(0)};}
fn allocated(){TRACK.with(|track|if track.get(){ALLOCS.with(|count|count.set(count.get()+1));});}
unsafe impl GlobalAlloc for CountAlloc {
    unsafe fn alloc(&self,layout:Layout)->*mut u8{allocated();System.alloc(layout)}
    unsafe fn alloc_zeroed(&self,layout:Layout)->*mut u8{allocated();System.alloc_zeroed(layout)}
    unsafe fn realloc(&self,p:*mut u8,layout:Layout,size:usize)->*mut u8{allocated();System.realloc(p,layout,size)}
    unsafe fn dealloc(&self,p:*mut u8,layout:Layout){System.dealloc(p,layout)}
}
#[global_allocator]
static ALLOCATOR:CountAlloc=CountAlloc;

fn run(rate:u32,channels:usize,input:&[f64],mut ppm:impl FnMut(usize)->f64)->(Vec<f64>,Vec<Position>) {
    let mut converter=Converter::new(rate,channels).unwrap();let mut output=vec![0.;converter.output_frames_max()*channels];let mut samples=Vec::new();let mut positions=Vec::new();let mut consumed=0;let mut block=0;
    loop {
        let remaining=input.len()/channels-consumed;
        converter.set_ppm(ppm(block)).unwrap();let needed=converter.input_frames_next();
        let result=if remaining>=needed{converter.process(&input[consumed*channels..(consumed+needed)*channels],&mut output).unwrap()}else{converter.finish(&input[consumed*channels..],&mut output).unwrap()};
        consumed+=result.input_frames;samples.extend_from_slice(&output[..result.output_frames*channels]);for frame in 0..result.output_frames{positions.push(result.span.position(frame).unwrap());}block+=1;
        if remaining<needed {
            let mut last=result;
            while !last.ended {last=converter.finish(&[],&mut output).unwrap();samples.extend_from_slice(&output[..last.output_frames*channels]);for frame in 0..last.output_frames{positions.push(last.span.position(frame).unwrap());}assert!(block<10000);block+=1;}
            assert_eq!(last.span.position(last.output_frames).unwrap(),Position{frame:input.len() as u64/channels as u64,fraction:0.});assert_eq!(converter.finish(&[],&mut output).unwrap().output_frames,0);break;
        }
    }(samples,positions)
}

#[test]
fn adaptive_bounds_invalid_inputs_and_eof_leave_no_false_progress() {
    for (rate,channels) in [(7999,2),(768001,2),(48000,0),(48000,33)]{assert!(Converter::new(rate,channels).is_err());}
    let mut converter=Converter::new(768000,2).unwrap();converter.set_ppm(123.).unwrap();let before=converter.input_frames_next();
    for ppm in [f64::NAN,f64::INFINITY,f64::NEG_INFINITY,500.001,-500.001] {assert!(converter.set_ppm(ppm).is_err());assert!((converter.ppm()-123.).abs()<1e-6);assert_eq!(converter.input_frames_next(),before);}
    let mut output=vec![0.;converter.output_frames_max()*2];let mut input=vec![0.25;converter.input_frames_next()*2];
    assert!(converter.process(&input[..input.len()-1],&mut output).is_err());assert!(converter.process(&input,&mut output[..1]).is_err());input[0]=f64::NAN;assert!(converter.process(&input,&mut output).is_err());assert_eq!(converter.origin,0);assert_eq!(converter.real_input,0);input[0]=0.25;
    converter.process(&input,&mut output).unwrap();let mut last=converter.finish(&[],&mut output).unwrap();while !last.ended{last=converter.finish(&[],&mut output).unwrap();}
    assert!(converter.set_ppm(0.).is_err());assert!(converter.process(&input,&mut output).is_err());assert!(converter.finish(&[0.;2],&mut output).is_err());
}

#[test]
fn adaptive_ramps_count_real_output_only_and_preserve_stream_boundaries() {
    for rate in [48000,768000] {
        for length in [0,1,17,520,1024,1025,32768] {
            let mut input=vec![0.;length*2];if length>0{input[0]=1.;}
            let(samples,positions)=run(rate,2,&input,|_|0.);assert_eq!(samples.len(),input.len());assert!(positions.windows(2).all(|p|p[1].frame>=p[0].frame));assert!(positions.iter().all(|p|p.frame<length as u64));
            if length>0 {assert!(samples[0]>0.9,"first source impulse must not be dropped: {rate}, {length}, {}",samples[0]);}
        }
        let input=vec![0.25;32768*2];let(fast,_)=run(rate,2,&input,|_|500.);let(slow,_)=run(rate,2,&input,|_|-500.);
        assert!((fast.len() as f64/2.-32768./1.0005).abs()<2.);assert!((slow.len() as f64/2.-32768./0.9995).abs()<2.);assert!(fast.len()<input.len()&&slow.len()>input.len());
        let(_,positions)=run(rate,2,&input,|n|[0.,450.,-450.,500.,-500.][n%5]);assert!(positions.windows(2).all(|p|p[1].frame>p[0].frame||p[1].frame==p[0].frame&&p[1].fraction>p[0].fraction));
        let crossing=positions.iter().filter(|p|p.frame<257).count();assert!(crossing>0&&crossing<positions.len());assert!(positions[crossing-1].frame<257&&positions[crossing].frame>=257);
    }
}

#[test]
fn adaptive_sinc_f64_preserves_low_bits_and_audio_passband_against_analytic_tones() {
    let low=2f64.powi(-31);let input=(0..8192).flat_map(|_|[0.5,0.5+low]).collect::<Vec<_>>();let(samples,positions)=run(768000,2,&input,|n|if n%2==0{450.}else{-450.});
    let mut difference=0.;let mut count=0;
    for (sample,position) in samples.chunks_exact(2).zip(positions){if (1024..7168).contains(&position.frame){difference+=sample[1]-sample[0];count+=1;}}
    assert!((difference/count as f64-low).abs()<1e-13,"sub-f32 DC difference was lost");
    for (rate,hz) in [(44100,20000.),(48000,20000.),(768000,20000.),(768000,350000.)] {
        let length=24576;let frequency=hz/rate as f64;let input=(0..length).flat_map(|n|{let phase=std::f64::consts::TAU*frequency*n as f64;[0.25*phase.sin(),0.125*phase.cos()]}).collect::<Vec<_>>();
        let(samples,positions)=run(rate,2,&input,|n|if n%3==0{450.}else if n%3==1{-450.}else{0.});let(mut error,mut energy,mut count)=(0.,0.,0);
        for (sample,position) in samples.chunks_exact(2).zip(positions){if !(1024..(length-1024) as u64).contains(&position.frame){continue;}let phase=std::f64::consts::TAU*frequency*(position.frame as f64+position.fraction);for (actual,reference) in sample.iter().zip([0.25*phase.sin(),0.125*phase.cos()]) {error+=(actual-reference).powi(2);energy+=reference.powi(2);}count+=1;}
        let snr=10.*(energy/error).log10();println!("ADAPTIVE_QUALITY_QA={}",serde_json::json!({"rate":rate,"tone_hz":hz,"snr_db":snr,"frames":count,"sinc_len":SINC_LEN,"oversampling":OVERSAMPLING}));assert!(snr>145.,"{rate} Hz / {hz} Hz: {snr} dB");
    }
}

#[test]
fn adaptive_processing_ratio_updates_and_drain_allocate_nothing() {
    let mut converter=Converter::new(768000,2).unwrap();let input=vec![0.1;converter.input_frames_max()*2];let mut output=vec![0.;converter.output_frames_max()*2];
    ALLOCS.with(|count|count.set(0));TRACK.with(|track|track.set(true));
    for n in 0..20 {converter.set_ppm([0.,450.,-450.][n%3]).unwrap();let count=converter.input_frames_next()*2;converter.process(&input[..count],&mut output).unwrap();}
    let mut last=converter.finish(&[],&mut output).unwrap();while !last.ended{last=converter.finish(&[],&mut output).unwrap();}
    TRACK.with(|track|track.set(false));assert_eq!(ALLOCS.with(Cell::get),0);
}

#[test]
fn adaptive_positions_keep_integer_origins_above_f64_precision() {
    let origin=1u64<<60;let span=Span{origin,index:-255.7578125,step:1.,delta:0.,skip:0,limit:None,frames:8};
    assert_eq!(span.position(0).unwrap(),Position{frame:origin,fraction:0.25});assert_eq!(span.position(1).unwrap(),Position{frame:origin+1,fraction:0.25});assert!(span.position(9).is_none());
    let clipped=Span{limit:Some(origin+3),..span};assert_eq!(clipped.position(8).unwrap(),Position{frame:origin+3,fraction:0.});
}
