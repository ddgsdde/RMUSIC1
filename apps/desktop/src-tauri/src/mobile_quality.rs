//! Android network profiles. A setting change applies only when preparing another source.
use anyhow::{Result,ensure};
use rmusic_audio::{output::OutputLimits,probe::AudioMetadata};
use serde_json::Value;
pub fn remote_network(address:&str,settings:&Value)->bool {
    use rmusic_bridge::network::{AddressKind,address_kind,on_link};
    if rmusic_bridge::connection::remote_gateway(address){return true;}
    let address=rmusic_bridge::connection::logical_address(address);
    let kind=address_kind(&address);
    if kind==AddressKind::Loopback{return false;}
    if settings["vpn"]==true||settings["network"]!="wifi"||kind==AddressKind::Overlay{return true;}
    let host=reqwest::Url::parse(&address).ok().and_then(|url|url.host_str().and_then(|v|v.trim_matches(['[',']']).parse::<std::net::IpAddr>().ok()));
    let subnets=settings["local_subnets"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect::<Vec<_>>();
    !host.is_some_and(|ip|on_link(ip,&subnets))
}
pub fn conversion(info:&Value,settings:&Value,limits:&OutputLimits)->Result<Option<(String,AudioMetadata)>> {
    ensure!(info["verified"]==true,"需要 Core 确认音源格式后再应用网络音质");
    let choice=settings[if settings["network"]=="wifi"{"wifi"}else{"mobile"}].as_str().unwrap_or("original");
    ensure!(["original","lossless","hires","opus96","opus160","opus320"].contains(&choice),"音质选择无效");
    let source=info["format"].as_str().unwrap_or("");let dsd=matches!(source,"dsf"|"dff");
    let rate=info["sample_rate"].as_u64().unwrap_or(0).min(u32::MAX as u64) as u32;
    let bits=info["bit_depth"].as_u64().unwrap_or(0).min(32) as u16;
    let channels=info["channels"].as_u64().unwrap_or(0).min(32) as u16;
    ensure!(rate>0&&channels>0,"音源格式尚未确认");
    let remote=settings["remote_lossless"]==true;
    let bitrate=if remote {0} else {choice.strip_prefix("opus").and_then(|s|s.parse::<u32>().ok()).unwrap_or(0)};
    let (max_rate,max_bits)=match choice {"lossless"=>(48000,16),"hires"=>(192000,24),_=>(768000,32)};
    let ceilings=OutputLimits{max_sample_rate:Some(limits.max_sample_rate.unwrap_or(768000).min(max_rate).min(settings["max_sample_rate"].as_u64().unwrap_or(768000) as u32)),..Default::default()};
    let rate=ceilings.rate(if dsd{rate/16}else{rate})?;
    let bit_limit=limits.max_bit_depth.unwrap_or(32).min(max_bits).min(settings["max_bit_depth"].as_u64().unwrap_or(32) as u16);
    let target_bits=if dsd||bits==0{bit_limit}else{bits.max(16).min(bit_limit)};
    let target_bits=if target_bits>24{32}else if target_bits>16{24}else{16};
    let target_channels=channels.min(limits.channels.unwrap_or(2)).min(2);
    let compress_pcm=remote&&!matches!(source,"flac"|"alac");
    if bitrate==0&&!dsd&&!compress_pcm&&rate as u64==info["sample_rate"].as_u64().unwrap_or(0)&&bits<=target_bits&&channels==target_channels&&!["ape","wv","unknown", ""].contains(&source){return Ok(None);}
    ensure!(bitrate==0||limits.max_sample_rate.unwrap_or(768000)>=48000,"Opus 48 kHz 解码时钟超过输出上限，请选择无损档位");
    let query=if bitrate>0 {format!("conversion=mobile_opus&rate=48000&bits=0&channels={target_channels}&bitrate={bitrate}")}
        else {format!("conversion=mobile_flac&rate={rate}&bits={target_bits}&channels={target_channels}")};
    Ok(Some((query,AudioMetadata{flac_samples:None,duration:info["duration"].as_f64().filter(|v|v.is_finite()&&*v>0.),format:if bitrate>0{"opus"}else{"flac"}.into(),sample_rate:if bitrate>0{48000}else{rate},channels:target_channels,bit_depth:if bitrate>0{0}else{target_bits},bitrate:bitrate*1000,quality_rank:0,source_integer_lossless:bitrate==0&&(matches!(source,"flac"|"alac")||matches!(source,"wav"|"aiff")&&(1..=24).contains(&bits))})))
}
#[cfg(test)] mod tests {
 use super::*;use serde_json::json;
 #[test]fn vpn_cellular_and_routed_private_addresses_use_remote_prefetch(){
 let s=json!({"network":"wifi","local_subnets":["192.168.50.2/24"]});
 assert!(!remote_network("http://192.168.50.10",&s));assert!(remote_network("http://10.22.0.20",&s));
 assert!(remote_network("http://100.64.1.2",&s));assert!(remote_network("http://[fd7a:115c:a1e0::1]",&s));
 let mut vpn=s;vpn["vpn"]=json!(true);assert!(remote_network("http://192.168.50.10",&vpn));
 }
 #[test]fn remote_integer_pcm_uses_lossless_flac_without_changing_specification(){
 let i=json!({"verified":true,"format":"wav","sample_rate":44100,"bit_depth":16,"channels":2});
 let s=json!({"network":"wifi","wifi":"original","remote_lossless":true});let(q,m)=conversion(&i,&s,&Default::default()).unwrap().unwrap();assert!(q.contains("mobile_flac"));assert_eq!((m.sample_rate,m.bit_depth),(44100,16));assert!(m.source_integer_lossless);
 }
 #[test] fn legacy_opus_setting_keeps_remote_transport_lossless_at_device_ceiling(){
 let info=json!({"verified":true,"format":"wav","sample_rate":768000,"bit_depth":32,"channels":2});
 let settings=json!({"network":"mobile","mobile":"opus160","remote_lossless":true});
 let (query,metadata)=conversion(&info,&settings,&Default::default()).unwrap().unwrap();
 assert!(query.contains("conversion=mobile_flac"));
 assert_eq!((metadata.sample_rate,metadata.bit_depth),(768000,32));
 let limits=OutputLimits {max_sample_rate:Some(192000),max_bit_depth:Some(24),..Default::default()};
 let (_,metadata)=conversion(&info,&settings,&limits).unwrap().unwrap();
 assert_eq!((metadata.sample_rate,metadata.bit_depth),(192000,24));
 }
 #[test] fn wifi_and_mobile_profiles_are_independent_and_lossy_is_explicit(){
 let info=json!({"verified":true,"format":"flac","sample_rate":96000,"bit_depth":24,"channels":2,"duration":6.});
 let settings=json!({"network":"wifi","wifi":"original","mobile":"opus160"});
 assert!(conversion(&info,&settings,&Default::default()).unwrap().is_none());
 let mut mobile=settings;mobile["network"]=json!("mobile");let (q,m)=conversion(&info,&mobile,&Default::default()).unwrap().unwrap();assert!(q.contains("bitrate=160"));assert_eq!(m.bit_depth,0);assert!(!m.source_integer_lossless);
 }
 #[test] fn limits_and_dsd_apply_without_upsampling_or_raw_dsd_transport(){
 let info=json!({"verified":true,"format":"dsf","sample_rate":2822400,"bit_depth":1,"channels":2,"duration":6.});
 let settings=json!({"network":"wifi","wifi":"lossless","mobile":"original"});let (q,m)=conversion(&info,&settings,&Default::default()).unwrap().unwrap();assert!(q.contains("mobile_flac"));assert_eq!(m.sample_rate,44100);assert_eq!(m.bit_depth,16);assert!(!m.source_integer_lossless);
 let mut unknown=info;unknown["verified"]=json!(false);assert!(conversion(&unknown,&settings,&Default::default()).is_err());
 }
}
