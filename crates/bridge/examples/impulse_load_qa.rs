//! Read-only loader QA against a disposable Core; does not open an audio output.
use anyhow::{Context,Result};
use rmusic_bridge::{connection::ConnectionProfile,impulses};
use serde::Deserialize;
use serde_json::json;
use std::path::PathBuf;
#[derive(Deserialize)]
struct Input {profile:ConnectionProfile,token_file:PathBuf,response_ids:Vec<String>,probe_rates:Vec<u32>,zone_id:Option<String>}
#[tokio::main]
async fn main()->Result<()> {
    let path=std::env::args().nth(1).context("usage: impulse_load_qa manifest.json")?;
    let input:Input=serde_json::from_slice(&std::fs::read(path)?)?;let token=std::fs::read_to_string(input.token_file)?;
    let profile=input.profile.resolve().await?;let bank=if let Some(zone)=&input.zone_id {impulses::load_selected(&profile,token.trim(),zone,&input.response_ids).await?}else{impulses::load(&profile,token.trim(),&input.response_ids).await?};
    let probes=input.probe_rates.into_iter().map(|rate|match bank.response(rate){
        Ok(response)=>{let values=&response.kernels()[0];json!({"rate":rate,"frames":response.frames(),"channels":response.channels(),"channel_mask":response.channel_mask(),"first":values[0],"first_bits":format!("{:016x}",values[0].to_bits()),"last_bits":format!("{:016x}",values.last().unwrap().to_bits())})},
        Err(error)=>json!({"rate":rate,"error":error.to_string()}),
    }).collect::<Vec<_>>();
    println!("{}",json!({"core_id":profile.core_id,"probes":probes}));Ok(())
}
