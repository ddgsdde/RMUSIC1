//! One HQPlayer instance is one output endpoint. HQPlayer alone owns its DAC/NAA.
use super::{Client,Response,Target,TIMEOUT};
use crate::{connection::ConnectionProfile,media_proxy::MediaProxy};
use anyhow::{bail,Context,Result};
use futures_util::{SinkExt,StreamExt};
use serde_json::{json,Value};
use std::{sync::{Arc,atomic::{AtomicBool,Ordering}},time::Duration};
use tokio::sync::{mpsc,watch};
use tokio_tungstenite::{connect_async_tls_with_config,Connector,tungstenite::{client::IntoClientRequest,Message}};

#[derive(Clone,Debug)]
struct Track {
    zone:String,id:String,core_id:Value,source:String,position:f64,duration:f64,title:String,artist:String,album:String,
}
#[derive(Clone,Debug)]
struct Desired {generation:u64,track:Option<Track>,paused:bool,volume:f64,muted:bool}
impl Default for Desired {fn default()->Self{Self{generation:0,track:None,paused:false,volume:1.,muted:false}}}
fn position(value:f64)->Result<f64>{if !value.is_finite()||value<0.||value>i32::MAX as f64 {bail!("HQPlayer position must be finite, nonnegative and within integer seconds");}Ok(value)}
fn volume(value:f64)->Result<f64>{if !value.is_finite()||!(0. ..=1.).contains(&value){bail!("volume must be between zero and one");}Ok(value)}
fn processing_supported(command:&Value)->Result<()> {
    if command["dsp"]["enabled"]==true{bail!("HQPlayer manages DSP; disable R-Music DSP for this output");}
    if ["fade_in_ms","fade_out_ms"].iter().any(|key|command["transitions"][key].as_u64().unwrap_or(0)>0){bail!("R-Music fades are not applied by HQPlayer; disable them for this output");}Ok(())
}
impl Desired {
    fn command(&mut self,kind:&str,value:&Value,profile:&ConnectionProfile)->Result<bool>{
        match kind {
            "play"=>{
                processing_supported(value)?;
                let source=value["url"].as_str().context("missing HQPlayer media URL")?;let source=profile.media_url(source)?.to_string();
                let gain=volume(value["volume"].as_f64().unwrap_or(1.))?;let start=position(value["position"].as_f64().unwrap_or(0.))?;
                let zone=value["zone_id"].as_str().filter(|id|!id.is_empty()).context("missing zone identity")?;
                let id=value["track"]["id"].as_str().filter(|id|!id.is_empty()).context("missing track identity")?;
                let resume=self.track.as_ref().is_some_and(|track|track.zone==zone&&track.id==id&&!track.core_id.is_null()&&track.core_id==value["playback_id"]);
                if !resume{self.generation+=1;self.track=Some(Track{zone:zone.into(),id:id.into(),core_id:value["playback_id"].clone(),source:source.into(),position:start,
                    duration:value["track"]["duration"].as_f64().filter(|v|v.is_finite()&&*v>=0.).unwrap_or(0.),title:value["track"]["title"].as_str().unwrap_or("").into(),artist:value["track"]["artist"].as_str().unwrap_or("").into(),album:value["track"]["album"].as_str().unwrap_or("").into()});}
                self.volume=gain;self.muted=value["muted"].as_bool().unwrap_or(false);self.paused=value["paused"].as_bool().unwrap_or(false);
            },
            "seek"=>{let start=position(value["position"].as_f64().context("seek requires position")?)?;if let Some(track)=&mut self.track{track.position=start;track.core_id=value["playback_id"].clone();self.generation+=1;}},
            "pause"=>self.paused=true,
            "resume"=>self.paused=false,
            "volume"=>{let gain=volume(value["volume"].as_f64().unwrap_or(self.volume))?;self.volume=gain;self.muted=value["muted"].as_bool().unwrap_or(self.muted);},
            "stop"=>{self.generation+=1;self.track=None;},
            "dsp"=>processing_supported(value)?,
            "registered"|"hello"|"ping"|"clock"=>return Ok(false),
            _=>bail!("unsupported HQPlayer command: {kind}"),
        }Ok(true)
    }
}

#[derive(Clone,Copy)]
struct VolumeRange {min:f64,max:f64,enabled:bool}
impl VolumeRange {
    fn parse(value:Response)->Result<Self>{let min=value.number("min")?;let max=value.number("max")?.min(0.);if min>=max{bail!("invalid HQPlayer volume range");}Ok(Self{min,max,enabled:value.attributes.get("enabled").is_some_and(|v|v=="1")})}
    fn db(&self,value:f64)->f64 {if value==0.{self.min}else{(20.*value.log10()).clamp(self.min,self.max)}}
}
#[derive(Debug)]
struct Superseded;
impl std::fmt::Display for Superseded {fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{f.write_str("HQPlayer command superseded")}}
impl std::error::Error for Superseded {}
async fn cancelled(mut desired:watch::Receiver<Desired>,generation:u64){loop{if desired.borrow().generation!=generation{return;}if desired.changed().await.is_err(){return;}}}
async fn rpc(client:&mut Client,desired:&watch::Receiver<Desired>,generation:u64,name:&str,attrs:&[(&str,&str)],metadata:&[(&str,&str)])->Result<Response>{
    tokio::select!{biased;_ = cancelled(desired.clone(),generation)=>Err(Superseded.into()),result=client.request(name,attrs,metadata)=>result}
}
async fn connect(target:&Target)->Result<(Client,VolumeRange,Response)>{
    let mut client=Client::connect(target).await?;let info=client.request("GetInfo",&[],&[]).await?;let range=VolumeRange::parse(client.request("VolumeRange",&[],&[]).await?)?;Ok((client,range,info))
}
pub async fn inspect(target:&Target)->Result<Value>{
    let(mut client,range,info)=connect(target).await?;let status=client.request("Status",&[("subscribe","0")],&[]).await?;
    Ok(json!({"server":info.attributes,"volume_range":{"min_db":range.min,"max_db":range.max,"enabled":range.enabled},"status":safe_status(&status)}))
}
fn safe_status(response:&Response)->Value {
    let names=["state","min","sec","volume","clips","active_mode","active_rate","active_bits","active_channels","active_filter","active_shaper","correction","output_delay"];
    json!(response.attributes.iter().filter(|(key,_)|names.contains(&key.as_str())).collect::<std::collections::BTreeMap<_,_>>())
}
fn signal_path(response:&Response)->Value {
    let mut stages=vec!["HQPlayer manages decoding, DSP and DAC/NAA output".to_owned()];
    for (key,label) in [("active_mode","模式"),("active_filter","滤波器"),("active_shaper","调制器"),("volume","音量 dB")]{if let Some(value)=response.attributes.get(key){stages.push(format!("HQPlayer {label}: {value}"));}}
    let mut result=json!({"output":"HQPlayer → DAC / NAA","exclusive":false,"processing":stages,"bit_perfect_eligible":false});
    for (attribute,field) in [("active_rate","sample_rate"),("active_bits","bit_depth"),("active_channels","channels")]{if let Some(value)=response.attributes.get(attribute).and_then(|v|v.parse::<u32>().ok()).filter(|v|*v>0){result[field]=json!(value);}}
    result["sample_type"]=json!(if response.attributes.get("active_mode").is_some_and(|mode|mode.eq_ignore_ascii_case("sdm")||mode.eq_ignore_ascii_case("dsd")){"sdm"}else{"integer"});result
}
struct Active {
    generation:u64,track:Track,uri:String,_proxy:Option<MediaProxy>,running:bool,paused:bool,volume:f64,muted:bool,
    observed:bool,position:f64,started:tokio::time::Instant,
}
impl Active {
    fn report(&self,status:&str)->Value {json!({"type":"state","zone_id":self.track.zone,"track_id":self.track.id,"core_playback_id":self.track.core_id,"status":status,"position":self.position})}
}
enum WorkerEvent {Report(u64,Value),Failed(String)}

async fn gain(client:&mut Client,desired:&watch::Receiver<Desired>,generation:u64,range:VolumeRange,active:&mut Active,setting:f64,muted:bool)->Result<()> {
    let silence=muted||setting==0.;
    if !range.enabled {if setting!=1.||silence{bail!("HQPlayer volume control is disabled");}active.volume=setting;active.muted=false;return Ok(());}
    if active.muted&&silence{active.volume=setting;return Ok(());}
    let db=range.db(setting).to_string();
    rpc(client,desired,generation,"Volume",&[("value",&db)],&[]).await?;
    let observed=rpc(client,desired,generation,"State",&[],&[]).await?.number("volume")?;
    if (observed-range.db(setting)).abs()>0.2{bail!("HQPlayer did not confirm the requested volume");}
    if silence{
        rpc(client,desired,generation,"VolumeMute",&[],&[]).await?;
        let after=rpc(client,desired,generation,"State",&[],&[]).await?.number("volume")?;
        if after>range.min||after>=observed{bail!("HQPlayer did not confirm mute");}
    }
    active.volume=setting;active.muted=silence;Ok(())
}
async fn worker(target:Target,profile:ConnectionProfile,mut desired:watch::Receiver<Desired>,reports:mpsc::Sender<WorkerEvent>,owned:Arc<AtomicBool>,initial:Client,range:VolumeRange)->Result<()> {
    let mut client=Some(initial);let mut active:Option<Active>=None;let mut applied=None;let mut interval=tokio::time::interval(Duration::from_millis(250));interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        let current=desired.borrow_and_update().clone();let generation=current.generation;
        let result:Result<()> = async {
            if client.is_none(){
                let connection=tokio::select!{biased;_=cancelled(desired.clone(),generation)=>return Err(Superseded.into()),result=Client::connect(&target)=>result?};client=Some(connection);
                if owned.load(Ordering::Acquire){rpc(client.as_mut().unwrap(),&desired,generation,"Stop",&[],&[]).await?;owned.store(false,Ordering::Release);}
            }
            let client=client.as_mut().unwrap();
            if applied!=Some(generation){
                active=None;
                if let Some(track)=current.track.clone(){
                    owned.store(true,Ordering::Release);rpc(client,&desired,generation,"Stop",&[],&[]).await?;
                    let proxy=if target.local(){Some(MediaProxy::start(&profile,&track.source,None).await?)}else{None};
                    let uri=proxy.as_ref().map(|proxy|proxy.url.clone()).unwrap_or_else(||track.source.clone());
                    rpc(client,&desired,generation,"PlaylistAdd",&[("uri",&uri),("queued","0"),("clear","1"),("start","0"),("freewheel","0")],&[("song",&track.title),("artist",&track.artist),("album",&track.album)]).await?;
                    active=Some(Active{generation,position:track.position,track,uri,_proxy:proxy,running:false,paused:true,volume:-1.,muted:false,observed:false,started:tokio::time::Instant::now()});
                }else if owned.load(Ordering::Acquire){rpc(client,&desired,generation,"Stop",&[],&[]).await?;owned.store(false,Ordering::Release);}
                applied=Some(generation);
            }
            let mut finished=false;
            if let Some(active)=active.as_mut(){
                let latest=desired.borrow().clone();if latest.generation!=generation{return Err(Superseded.into());}
                if active.volume!=latest.volume||active.muted!=(latest.muted||latest.volume==0.){gain(client,&desired,generation,range,active,latest.volume,latest.muted).await?;}
                if latest.paused {
                    if active.running&&!active.paused{rpc(client,&desired,generation,"Pause",&[],&[]).await?;active.paused=true;}
                    if !active.running{return Ok(());}
                }else if !active.running||active.paused{
                    if !active.running&&active.track.position>0.{let start=active.track.position.floor().to_string();rpc(client,&desired,generation,"Seek",&[("position",&start)],&[]).await?;}
                    // A newer pause/volume received while preparing is applied before Play.
                    let latest=desired.borrow().clone();if latest.paused||active.volume!=latest.volume||active.muted!=(latest.muted||latest.volume==0.){return Ok(());}
                    rpc(client,&desired,generation,"Play",&[("last","0")],&[]).await?;active.running=true;active.paused=false;active.started=tokio::time::Instant::now();
                }
                let status=rpc(client,&desired,generation,"Status",&[("subscribe","0")],&[]).await?;
                let observed_uri=status.metadata.get("uri");
                if observed_uri.is_some_and(|uri|!uri.is_empty()&&uri!=&active.uri){owned.store(false,Ordering::Release);bail!("HQPlayer source changed in another controller");}
                if observed_uri!=Some(&active.uri){if active.started.elapsed()>Duration::from_secs(10){bail!("HQPlayer has not confirmed the selected media");}return Ok(());}
                let state=status.number("state")? as i32;let min=status.number("min")?;let sec=status.number("sec")?;
                if min<0.||!(0. ..60.).contains(&sec){bail!("invalid HQPlayer playback position");}active.position=min*60.+sec;
                if state==0&&!active.observed{if active.started.elapsed()<Duration::from_secs(10){return Ok(());}bail!("HQPlayer did not start the selected media");}
                active.observed|=state==2;
                let state=match state{0=>if active.observed&&active.track.duration>0.&&active.position>=active.track.duration-1.&&!active.paused{"ended"}else{"stopped"},1=>"paused",2=>"playing",3=>return Ok(()),_=>bail!("unknown HQPlayer transport state")};
                let mut report=active.report(state);report["signal_path"]=signal_path(&status);report["hqplayer_status"]=safe_status(&status);
                tokio::select!{biased;_=cancelled(desired.clone(),generation)=>return Err(Superseded.into()),result=reports.send(WorkerEvent::Report(active.generation,report))=>{result.context("HQPlayer report channel closed")?;}}
                if state=="ended"||state=="stopped"{owned.store(false,Ordering::Release);finished=true;}
            }
            if finished{active=None;}Ok(())
        }.await;
        if let Err(error)=result{
            active=None;
            if error.is::<Superseded>(){client=None;if desired.has_changed().is_err(){break;}continue;}
            if owned.load(Ordering::Acquire){if let Some(client)=client.as_mut(){if client.request("Stop",&[],&[]).await.is_ok(){owned.store(false,Ordering::Release);}}}
            return Err(error);
        }
        tokio::select!{result=desired.changed()=>if result.is_err(){break;},_=interval.tick()=>{}}
    }
    if owned.load(Ordering::Acquire){let mut client=match client{Some(client)=>client,None=>Client::connect(&target).await?};client.request("Stop",&[],&[]).await?;owned.store(false,Ordering::Release);}Ok(())
}

pub async fn run(profile:&ConnectionProfile,token:&str,endpoint_id:&str,name:&str,target:&Target)->Result<()> {
    let mut profile=profile.clone();let owned=Arc::new(AtomicBool::new(false));let mut delay=1;
    loop {
        let result=async {
            let(selected_client,range,_)=connect(target).await?;
            let mut selected_client=selected_client;
            if owned.load(Ordering::Acquire){selected_client.request("Stop",&[],&[]).await?;owned.store(false,Ordering::Release);}
            let selected=profile.resolve_lan_only().await?;if profile.core_id.is_none(){profile.core_id=selected.core_id.clone();}
            let _announcement=crate::lan::Announcement::start(endpoint_id,name,selected.core_id.as_deref()).await.ok();
            session(&selected,token,endpoint_id,name,target,owned.clone(),selected_client,range).await
        }.await;
        match result {Ok(())=>return Ok(()),Err(error)=>{eprintln!("HQPlayer bridge disconnected: {error:#}; retry in {delay}s");tokio::select!{_=tokio::time::sleep(Duration::from_secs(delay))=>{},_=tokio::signal::ctrl_c()=>return Ok(())};delay=(delay*2).min(30);}}
    }
}
async fn session(profile:&ConnectionProfile,token:&str,endpoint_id:&str,name:&str,target:&Target,owned:Arc<AtomicBool>,client:Client,range:VolumeRange)->Result<()> {
    let mut url=profile.endpoint("api/v1/bridge/events")?;url.set_scheme(if url.scheme()=="https"{"wss"}else{"ws"}).map_err(|_|anyhow::anyhow!("invalid bridge URL"))?;
    let mut request=url.as_str().into_client_request()?;request.headers_mut().insert("Authorization",format!("Bearer {token}").parse()?);
    let(mut socket,_)=connect_async_tls_with_config(request,None,false,Some(Connector::Rustls(profile.tls_config()?))).await?;
    socket.send(Message::Text(json!({"type":"register","protocol_version":1,"endpoint":{"id":endpoint_id,"name":name,"kind":"hqplayer","online":true,"verified":false,
        "capabilities":{"http_pull":true,"playback_instance":true,"media_url_api_relative":true,"dsp":false,"transitions":false,"strict_sync":false,"gapless":false,"volume":range.enabled,"seek_resolution_seconds":1,"hqplayer_control":"xml-tcp","media_path":if target.local(){"verified_loopback_proxy"}else{"direct_core_url"}}}}).to_string().into())).await?;
    eprintln!("HQPlayer bridge connected: {name} ({endpoint_id})");
    let(sender,receiver)=watch::channel(Desired::default());let(reports,mut events)=mpsc::channel(64);let worker_reports=reports.clone();
    let target=target.clone();let selected=profile.clone();let worker_owned=owned.clone();
    let mut task=tokio::spawn(async move{if let Err(error)=worker(target,selected,receiver,worker_reports.clone(),worker_owned,client,range).await{let _=worker_reports.send(WorkerEvent::Failed(error.to_string())).await;}});
    let result=async {loop {tokio::select!{
        _=tokio::signal::ctrl_c()=>return Ok(()),
        event=events.recv()=>match event.context("HQPlayer worker exited")?{
            WorkerEvent::Report(generation,mut value)=>if sender.borrow().generation==generation{value["endpoint_id"]=json!(endpoint_id);socket.send(Message::Text(value.to_string().into())).await?;},
            WorkerEvent::Failed(error)=>{
                let current=sender.borrow().clone();if let Some(track)=current.track{socket.send(Message::Text(json!({"type":"error","endpoint_id":endpoint_id,"zone_id":track.zone,"track_id":track.id,"core_playback_id":track.core_id,"error":error}).to_string().into())).await?;}
                bail!("{error}");
            },
        },
        message=socket.next()=>{
            let text=match message.context("core closed HQPlayer bridge")??{Message::Text(text)=>text,Message::Ping(data)=>{socket.send(Message::Pong(data)).await?;continue;},Message::Close(_)=>bail!("core closed HQPlayer bridge"),_=>continue};
            let value:Value=serde_json::from_str(&text)?;let command=value.get("command").unwrap_or(&value);let kind=command["type"].as_str().unwrap_or("");
            let mut desired=sender.borrow().clone();match desired.command(kind,command,profile){
                Ok(true)=>{sender.send_replace(desired);},Ok(false)=>{},Err(error)=>{
                    if kind=="play"{desired.generation+=1;desired.track=None;sender.send_replace(desired);}
                    socket.send(Message::Text(json!({"type":"error","endpoint_id":endpoint_id,"zone_id":command["zone_id"],"track_id":command["track"]["id"],"core_playback_id":command["playback_id"],"error":error.to_string()}).to_string().into())).await?;
                }
            }
        },
    }}}.await;
    drop(sender);drop(reports);
    if tokio::time::timeout(TIMEOUT*3,&mut task).await.is_err(){task.abort();let _=task.await;}
    result
}

#[cfg(test)]
mod tests;
