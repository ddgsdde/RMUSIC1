//! Local facade for isolated Core acceptance scripts; stdin EOF closes the fixture.
use anyhow::{Context, Result};
use tokio::io::AsyncReadExt;

#[tokio::main]
async fn main() -> Result<()> {
    let base = std::env::args().nth(1).context("expected Core address")?;
    let status: serde_json::Value = reqwest::Client::new()
        .get(format!("{}/api/v1/status", base.trim_end_matches('/')))
        .send().await?.error_for_status()?.json().await?;
    let gateway = rmusic_remote::native::open_core(
        &base,
        status["core_id"].as_str().context("missing Core identity")?.into(),
        status["core_public_key"].as_str().context("missing Core key")?.into(),
        None, false,
    ).await?;
    println!("{}", serde_json::json!({"address": gateway.address}));
    let mut input = Vec::new();
    tokio::io::stdin().read_to_end(&mut input).await?;
    Ok(())
}
