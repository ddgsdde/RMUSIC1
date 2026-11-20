//! Opt-in real Core transport probe. Credentials arrive on stdin; audio stays in memory.
use super::*;
use serde_json::{json,Value};
use futures_util::StreamExt;
use sha2::{Digest,Sha256};

#[tokio::test]
#[ignore = "requires explicitly supplied NAS credentials on stdin; no output device is opened"]
async fn nas_lan_and_https_preserve_the_same_original_entity()->Result<()> {
 let input:Value=serde_json::from_reader(std::io::stdin())?;
 let mut lan:ConnectionProfile=serde_json::from_value(json!({"id":"live-transport","name":"NAS transport probe","address":input["lan"],"core_id":input["core_id"]}))?;
 lan=lan.resolve().await?;
 anyhow::ensure!(!crate::network_route::remote(&lan.address).await,"NAS address must use an on-link physical route for this LAN test");
 let mut https=lan.clone();https.address=input["https"].as_str().context("missing HTTPS address")?.into();
 let https=https.resolve().await?;
 anyhow::ensure!(https.core_id==lan.core_id,"Core identity mismatch");
 let auth:Value=lan.http()?.post(lan.endpoint("api/v1/auth/login")?).json(&json!({"username":input["username"],"password":input["password"]})).send().await?.error_for_status()?.json().await?;
 let token=auth["token"].as_str().context("missing session")?;
 let result=async {
  let list:Value=lan.http()?.get(lan.endpoint("api/v1/library/tracks?storage=cache&limit=20")?).bearer_auth(token).send().await?.error_for_status()?.json().await?;
  let track=list["items"].as_array().context("missing tracks")?.iter().find(|t|t["format"]=="flac").context("NAS needs an existing cached FLAC; this probe does not fetch another platform track")?;
  let id=track["id"].as_str().context("missing track ID")?;
  let info:Value=lan.http()?.get(lan.endpoint(&format!("api/v1/library/tracks/{id}/audio-info"))?).bearer_auth(token).send().await?.error_for_status()?.json().await?;
  anyhow::ensure!(info["verified"]==true&&info["format"]=="flac","source must be verified original FLAC");
  let ticket:Value=lan.http()?.get(lan.endpoint(&format!("api/v1/library/tracks/{id}/ticket"))?).bearer_auth(token).send().await?.error_for_status()?.json().await?;
  let path=ticket["api_path"].as_str().context("missing media path")?;
  let mut evidence=Vec::new();let mut expected=None;
  for (profile,buffered) in [(&lan,false),(&https,true)] {
   let source=profile.media_url(path)?.to_string();
   let head=profile.http()?.head(&source).send().await?.error_for_status()?;
   let tag=head.headers().get("etag").and_then(|v|v.to_str().ok()).filter(|s|s.starts_with('"')&&s.ends_with('"')).context("strong ETag required")?.to_owned();
   let total=head.headers().get("content-length").and_then(|h|h.to_str().ok()).and_then(|h|h.parse::<u64>().ok()).context("entity length required")?;let count=total.min(256*1024);anyhow::ensure!(count>65536,"fixture too short");
   let mut proxy=MediaProxy::start_entity(profile,&source,None,Some(tag)).await?;
   if buffered {proxy=proxy.with_network_buffer(rmusic_bridge::media_proxy::NetworkBuffer::for_audio(&info,30,32*1024*1024));}
   let started=std::time::Instant::now();
   let response=reqwest::Client::new().get(&proxy.url).header("Range",format!("bytes=0-{}",count-1)).header("Accept-Encoding","identity").send().await?.error_for_status()?;
   anyhow::ensure!(response.status()==206&&response.content_length()==Some(count),"range changed");
   let mut stream=response.bytes_stream();let mut digest=Sha256::new();let mut bytes=0;
   while let Some(chunk)=stream.next().await {let chunk=chunk?;bytes+=chunk.len() as u64;anyhow::ensure!(bytes<=count,"unbounded response");digest.update(chunk);}
   anyhow::ensure!(bytes==count,"truncated prefix");let hash=format!("{:x}",digest.finalize());
   if let Some(expected)=expected.as_ref(){anyhow::ensure!(expected==&hash,"HTTPS changed original bytes");}else{expected=Some(hash.clone());}
   evidence.push(json!({"transport":if buffered{"https_remote_buffer"}else{"lan_direct"},"bytes":bytes,"elapsed_ms":started.elapsed().as_millis(),"sha256":hash,"range_validated":true}));
  }
  println!("NAS_TRANSPORT_EVIDENCE {}",json!({"sample_rate":info["sample_rate"],"bit_depth":info["bit_depth"],"original_flac":true,"physical_output_opened":false,"checks":evidence}));Ok::<_,anyhow::Error>(())
 }.await;
 let _=lan.http()?.post(lan.endpoint("api/v1/auth/logout")?).bearer_auth(token).send().await;
 result
}
