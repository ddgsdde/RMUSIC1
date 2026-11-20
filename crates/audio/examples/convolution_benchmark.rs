//! Software-only packet timing and full-tail measurement, not hardware certification.
use rmusic_audio::convolution::FirConvolver;
use std::time::Instant;

fn trial(rate:usize,taps:usize,channels:usize,kernel_channels:usize)->anyhow::Result<()> {
    let kernels:Vec<Vec<f64>>=(0..kernel_channels).map(|channel|(0..taps).map(|i|((i*(channel+3)%101) as f64-50.)*0.00001).collect()).collect();
    let kernel_sums=kernels.iter().map(|k|k.iter().sum::<f64>()).collect::<Vec<_>>();let mut input_sums=vec![0.;channels];
    let setup=Instant::now();let mut convolver=FirConvolver::new(kernels,channels)?;let setup_ms=setup.elapsed().as_secs_f64()*1000.;
    let frames=rate*12;let packet=rate/100;let mut data=vec![0.;packet*channels];let mut worst=0_f64;let mut convolution_ms=0.;let mut checksum=0.;let started=Instant::now();
    let mut timings=Vec::with_capacity((frames+taps-1).div_ceil(packet));
    for offset in (0..frames).step_by(packet) {
        for (i,value) in data.iter_mut().enumerate(){*value=((offset*channels+i)%97) as f64/9700.;input_sums[i%channels]+=*value;}
        let before=Instant::now();convolver.process(&mut data)?;let elapsed=before.elapsed().as_secs_f64()*1000.;worst=worst.max(elapsed);convolution_ms+=elapsed;timings.push(elapsed);
        checksum+=data.iter().sum::<f64>();
    }
    let mut tail=0;
    while convolver.tail_remaining()>0 {
        let before=Instant::now();let samples=convolver.drain(packet);let elapsed=before.elapsed().as_secs_f64()*1000.;worst=worst.max(elapsed);convolution_ms+=elapsed;timings.push(elapsed);
        tail+=samples.len()/channels;checksum+=samples.iter().sum::<f64>();
    }
    assert_eq!(tail,taps-1);
    let expected_checksum=input_sums.iter().enumerate().map(|(channel,input)|input*kernel_sums[if kernel_channels==1{0}else{channel}]).sum::<f64>();let checksum_error=(checksum-expected_checksum).abs();
    assert!(checksum_error<expected_checksum.abs()*1e-9+1e-9);
    timings.sort_by(f64::total_cmp);let p99=timings[(timings.len()*99).div_ceil(100)-1];
    println!("{}",serde_json::json!({"sample_rate":rate,"channels":channels,"kernel_channels":kernel_channels,"taps_per_channel":taps,"programme_frames":frames,"tail_frames":tail,"setup_ms":setup_ms,"processing_ms":started.elapsed().as_secs_f64()*1000.,"convolution_ms":convolution_ms,"worst_packet_ms":worst,"packet_p99_ms":p99,"packet_mean_ms":convolution_ms/timings.len() as f64,"packet_count":timings.len(),"over_budget_packets":timings.iter().filter(|value|**value>10.).count(),"packet_audio_ms":10,"packet_budget_exceeded":worst>10.,"checksum":checksum,"checksum_error":checksum_error}));
    Ok(())
}
fn main()->anyhow::Result<()> {
    for (rate,taps,channels,kernels) in [(48000,16384,2,2),(48000,65536,2,2),(192000,65536,2,2),(192000,131072,2,2),(192000,262144,2,1),(192000,262144,4,1),(192000,131072,8,1),(192000,65536,16,1),(192000,32768,32,1),(192000,64,2,1),(192000,64,32,1)] {trial(rate,taps,channels,kernels)?;}
    Ok(())
}
