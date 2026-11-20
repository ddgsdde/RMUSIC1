//! A native encrypted Core connection for isolated HTTP acceptance tools.
use anyhow::{Context, Result};
use rmusic_bridge::connection::ConnectionProfile;
use serde_json::json;
use tokio::io::AsyncReadExt;

#[tokio::main(worker_threads = 2)]
async fn main() -> Result<()> {
    let address=std::env::args().nth(1).context("需要 Core 地址")?;
    let profile:ConnectionProfile=serde_json::from_value(json!({"id":"image-qa","name":"Image QA","address":address}))?;
    let selected=profile.resolve().await?;
    selected.require_encrypted_credentials()?;
    println!("{}",json!({"address":selected.address}));
    let mut input=Vec::new();tokio::io::stdin().read_to_end(&mut input).await?;
    Ok(())
}
