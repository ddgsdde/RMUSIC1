//! CPU-only benchmark: no device, decoder, Core, or user audio is opened.
use rmusic_audio::adaptive::Converter;
use std::time::Instant;
fn main()->anyhow::Result<()> {
    let initialization=Instant::now();let mut converter=Converter::new(768000,2)?;let initialization=initialization.elapsed().as_secs_f64();
    let input=(0..converter.input_frames_max()*2).map(|n|0.125+(n%16) as f64/1024.).collect::<Vec<_>>();let mut output=vec![0.;converter.output_frames_max()*2];
    let start=Instant::now();let mut frames=0u64;let mut blocks=0usize;
    while frames<768000 {
        converter.set_ppm([0.,450.,-450.][blocks/50%3])?;let needed=converter.input_frames_next()*2;
        let block=converter.process(&input[..needed],&mut output)?;frames+=block.output_frames as u64;blocks+=1;
    }
    let elapsed=start.elapsed().as_secs_f64();let seconds=frames as f64/768000.;
    println!("{}",serde_json::json!({"sample_rate":768000,"channels":2,"output_frames":frames,"audio_seconds":seconds,"processing_seconds":elapsed,"realtime_multiple":seconds/elapsed,"initialization_seconds":initialization,"debug_assertions":cfg!(debug_assertions),"os":std::env::consts::OS,"arch":std::env::consts::ARCH,"physical_device":false}));Ok(())
}
