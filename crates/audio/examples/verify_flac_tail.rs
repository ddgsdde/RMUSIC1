//! Read-only streaming check. JSON arrives on stdin so media tickets stay out of argv/logs.
//! Never opens an audio output or writes the source to disk.
use anyhow::{Context,Result};
use rmusic_audio::{decoder::{FfmpegDecoder,PcmEncoding},probe};
use serde_json::{json,Value};
use std::io::Read;
fn main()->Result<()> {
    let mut text=String::new();std::io::stdin().take(16384).read_to_string(&mut text)?;
    let input:Value=serde_json::from_str(&text)?;
    let path=input["input"].as_str().context("missing input")?;
    let metadata=probe::probe(std::path::Path::new(path))?;
    let samples=metadata.flac_samples.context("missing exact STREAMINFO sample count")?;
    let position=input["position"].as_f64().unwrap_or(0.);
    let rate=input["rate"].as_u64().map(|r|u32::try_from(r)).transpose()?.unwrap_or(metadata.sample_rate);
    let mut decoder=FfmpegDecoder::open_flac(path,position,rate,metadata.channels,PcmEncoding::Signed32,Some((samples,metadata.sample_rate)))?;
    let width=metadata.channels as usize*4;let mut buffer=vec![0;width*4096];let mut frames=0u64;
    loop {let n=decoder.read_frames(&mut buffer,width)?;if n==0{break;}frames+=n as u64;}
    let expected=(samples as u128*rate as u128).div_ceil(metadata.sample_rate as u128).saturating_sub((position*rate as f64).round() as u128);
    anyhow::ensure!(frames as u128==expected,"FLAC programme frame count differs");
    println!("{}",json!({"source_rate":metadata.sample_rate,"source_samples":samples,"output_rate":rate,"position":position,"decoded_frames":frames,"expected_frames":expected,"physical_output_opened":false}));
    Ok(())
}
