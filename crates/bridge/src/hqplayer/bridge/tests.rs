use super::*;
use crate::hqplayer::{command,document};
use std::sync::Mutex;
use tokio::{io::{AsyncReadExt,AsyncWriteExt},net::TcpListener};

fn profile()->ConnectionProfile {ConnectionProfile{core_public_key:None,id:"qa".into(),name:"QA".into(),address:"http://127.0.0.1:7800".into(),skip_certificate_verification: false, certificate_sha256: None,alternatives:vec![],core_id:None}}
fn play()->Value {json!({"type":"play","zone_id":"zone","track":{"id":"song","title":"歌曲 & live","artist":"歌手","album":"专辑","duration":12},"playback_id":"core-1","url":"http://127.0.0.1:7800/api/v1/media/song?ticket=test","volume":1.,"position":0.,"paused":false,"muted":false})}
fn range()->VolumeRange {VolumeRange{min:-60.,max:0.,enabled:true}}

#[test]
fn commands_preserve_identity_controls_and_reject_before_mutating(){
    let mut desired=Desired::default();desired.command("play",&play(),&profile()).unwrap();assert_eq!(desired.generation,1);
    desired.command("pause",&json!({}),&profile()).unwrap();desired.command("volume",&json!({"volume":0.2,"muted":true}),&profile()).unwrap();
    desired.command("seek",&json!({"position":6.,"playback_id":"core-2"}),&profile()).unwrap();assert_eq!(desired.generation,2);assert!(desired.paused&&desired.muted);assert_eq!(desired.volume,0.2);assert_eq!(desired.track.as_ref().unwrap().core_id,"core-2");
    let before=format!("{desired:?}");
    for (kind,value) in [("volume",json!({"volume":2.})),("seek",json!({"position":-1.})),("play",{let mut value=play();value["url"]=json!("http://other-core/api/v1/media/song");value}),("play",{let mut value=play();value["dsp"]=json!({"enabled":true});value}),("play",{let mut value=play();value["transitions"]=json!({"fade_in_ms":250});value})]{
        assert!(desired.command(kind,&value,&profile()).is_err());assert_eq!(format!("{desired:?}"),before);
    }
    let mut resumed=play();resumed["playback_id"]=json!("core-2");resumed["volume"]=json!(0.2);resumed["muted"]=json!(true);
    desired.command("play",&resumed,&profile()).unwrap();assert_eq!(desired.generation,2);assert_eq!(desired.track.as_ref().unwrap().position,6.);assert!(!desired.paused);
    desired.command("stop",&json!({}),&profile()).unwrap();assert_eq!(desired.generation,3);assert!(desired.track.is_none());
}

#[test]
fn volume_range_is_finite_and_cannot_amplify_above_zero_db(){
    let response=document(b"<VolumeRange min='-60' max='12' enabled='1'/>").unwrap().unwrap().0;
    let range=VolumeRange::parse(response).unwrap();assert_eq!(range.db(1.),0.);assert!((range.db(0.5)+6.020599913).abs()<1e-6);assert_eq!(range.db(0.),-60.);assert_eq!(range.db(1e-10),-60.);
    assert!(VolumeRange::parse(document(b"<VolumeRange min='1' max='12' enabled='1'/>").unwrap().unwrap().0).is_err());
}

#[derive(Default)]
struct Engine {requests:Vec<String>,uri:String,state:u8,volume:f64,sec:u32,external:bool,delay_add:bool,add_seen:bool,closed_delayed:bool}
struct Server {target:Target,engine:Arc<Mutex<Engine>>,task:tokio::task::JoinHandle<()>}
impl Drop for Server{fn drop(&mut self){self.task.abort();}}
impl Server {
    async fn start()->Self {
        let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();let target=Target::new("127.0.0.1",listener.local_addr().unwrap().port()).unwrap();let engine=Arc::new(Mutex::new(Engine::default()));let shared=engine.clone();
        let task=tokio::spawn(async move{loop{let(mut socket,_)=listener.accept().await.unwrap();let shared=shared.clone();tokio::spawn(async move{
            let mut pending=vec![];
            loop {
                let request=loop {
                    if let Some((request,used))=document(&pending).unwrap(){pending.drain(..used);break request;}
                    let mut block=[0;4096];let size=match socket.read(&mut block).await{Ok(size)=>size,Err(_)=>return};if size==0{return;}pending.extend_from_slice(&block[..size]);
                };
                let name=request.command.clone();let(mut attrs,mut metadata,delay)={
                    let mut state=shared.lock().unwrap();state.requests.push(name.clone());let mut attrs=vec![("result".to_owned(),"OK".to_owned())];let mut metadata=vec![];
                    match name.as_str(){
                        "Stop"=>{state.state=0;state.sec=0;},
                        "PlaylistAdd"=>{state.uri=request.attributes["uri"].clone();state.add_seen=true;},
                        "Volume"=>{state.volume=request.attributes["value"].parse().unwrap();},
                        "VolumeMute"=>{state.volume=-144.;},
                        "State"=>{attrs.push(("volume".into(),state.volume.to_string()));},
                        "Play"=>state.state=2,
                        "Pause"=>state.state=1,
                        "Seek"=>state.sec=request.attributes["position"].parse().unwrap(),
                        "Status"=>{
                            attrs.extend([("state".into(),state.state.to_string()),("min".into(),(state.sec/60).to_string()),("sec".into(),(state.sec%60).to_string()),("volume".into(),state.volume.to_string()),("active_mode".into(),"PCM".into()),("active_rate".into(),"192000".into()),("active_bits".into(),"24".into()),("active_channels".into(),"2".into()),("active_filter".into(),"poly-sinc".into())]);
                            metadata.push(("uri".to_owned(),if state.external{"http://another-player/track".into()}else{state.uri.clone()}));
                        },_=>{},
                    }
                    (attrs,metadata,name=="PlaylistAdd"&&state.delay_add)
                };
                if delay {let mut byte=[0];if socket.read(&mut byte).await.unwrap_or(0)==0{shared.lock().unwrap().closed_delayed=true;}return;}
                let attrs=attrs.drain(..).collect::<Vec<_>>();let metadata=metadata.drain(..).collect::<Vec<_>>();
                let bytes=command(&name,&attrs.iter().map(|(a,b)|(a.as_str(),b.as_str())).collect::<Vec<_>>(),&metadata.iter().map(|(a,b)|(a.as_str(),b.as_str())).collect::<Vec<_>>()).unwrap();
                if socket.write_all(&bytes).await.is_err(){return;}
            }
        });}});
        Self{target,engine,task}
    }
    async fn wait(&self,check:impl Fn(&Engine)->bool){tokio::time::timeout(Duration::from_secs(2),async{loop{if check(&self.engine.lock().unwrap()){break;}tokio::time::sleep(Duration::from_millis(5)).await;}}).await.unwrap();}
}

#[tokio::test]
async fn real_worker_retains_preloaded_controls_seek_and_reports_only_its_media(){
    let server=Server::start().await;let client=Client::connect(&server.target).await.unwrap();let(sender,receiver)=watch::channel(Desired::default());let(reports,mut events)=mpsc::channel(64);let owned=Arc::new(AtomicBool::new(false));
    let task=tokio::spawn(worker(server.target.clone(),profile(),receiver,reports,owned.clone(),client,range()));
    tokio::time::sleep(Duration::from_millis(50)).await;assert!(server.engine.lock().unwrap().requests.is_empty(),"starting the adapter must not stop another HQPlayer session");
    let mut first=play();first["paused"]=json!(true);let mut desired=Desired::default();desired.command("play",&first,&profile()).unwrap();sender.send_replace(desired.clone());
    server.wait(|state|state.add_seen&&state.requests.iter().any(|name|name=="State")).await;assert!(!server.engine.lock().unwrap().requests.iter().any(|name|name=="Play"));
    desired.command("volume",&json!({"volume":0.25,"muted":true}),&profile()).unwrap();desired.command("resume",&json!({}),&profile()).unwrap();sender.send_replace(desired.clone());
    server.wait(|state|state.state==2&&state.volume==-144.).await;
    let event=tokio::time::timeout(Duration::from_secs(2),events.recv()).await.unwrap().unwrap();let WorkerEvent::Report(generation,report)=event else{panic!("unexpected failure")};
    assert_eq!(generation,1);assert_eq!(report["core_playback_id"],"core-1");assert_eq!(report["signal_path"]["sample_rate"],192000);assert_eq!(report["signal_path"]["bit_depth"],24);assert_eq!(report["signal_path"]["bit_perfect_eligible"],false);
    desired.command("pause",&json!({}),&profile()).unwrap();sender.send_replace(desired.clone());server.wait(|state|state.state==1).await;
    desired.command("seek",&json!({"position":6.,"playback_id":"core-2"}),&profile()).unwrap();sender.send_replace(desired.clone());server.wait(|state|state.requests.iter().filter(|name|name.as_str()=="PlaylistAdd").count()==2).await;
    assert_eq!(server.engine.lock().unwrap().state,0);desired.command("volume",&json!({"volume":0.5,"muted":false}),&profile()).unwrap();desired.command("resume",&json!({}),&profile()).unwrap();sender.send_replace(desired.clone());
    server.wait(|state|state.state==2&&state.sec==6&&(state.volume+6.020599913).abs()<1e-6).await;
    let order=server.engine.lock().unwrap().requests.clone();let seek=order.iter().position(|name|name=="Seek").unwrap();assert_eq!(order[seek+1],"Play");
    desired.command("stop",&json!({}),&profile()).unwrap();sender.send_replace(desired);server.wait(|state|state.state==0).await;assert!(!owned.load(Ordering::Acquire));
    drop(sender);tokio::time::timeout(Duration::from_secs(2),task).await.unwrap().unwrap().unwrap();
}

#[tokio::test]
async fn stop_cancels_a_real_delayed_tcp_transaction_and_clears_old_playback(){
    let server=Server::start().await;server.engine.lock().unwrap().delay_add=true;
    let client=Client::connect(&server.target).await.unwrap();let(sender,receiver)=watch::channel(Desired::default());let(reports,_events)=mpsc::channel(64);let owned=Arc::new(AtomicBool::new(false));
    let task=tokio::spawn(worker(server.target.clone(),profile(),receiver,reports,owned.clone(),client,range()));
    let mut desired=Desired::default();desired.command("play",&play(),&profile()).unwrap();sender.send_replace(desired.clone());server.wait(|state|state.add_seen).await;
    desired.command("stop",&json!({}),&profile()).unwrap();sender.send_replace(desired);server.wait(|state|state.closed_delayed&&state.requests.iter().filter(|name|name.as_str()=="Stop").count()>=2).await;
    assert!(!server.engine.lock().unwrap().requests.iter().any(|name|name=="Play"));assert!(!owned.load(Ordering::Acquire));
    drop(sender);tokio::time::timeout(Duration::from_secs(2),task).await.unwrap().unwrap().unwrap();
}

#[tokio::test]
async fn another_controller_source_is_not_reported_or_stopped_as_ours(){
    let server=Server::start().await;server.engine.lock().unwrap().external=true;
    let client=Client::connect(&server.target).await.unwrap();let(sender,receiver)=watch::channel(Desired::default());let(reports,mut events)=mpsc::channel(64);let owned=Arc::new(AtomicBool::new(false));
    let task=tokio::spawn(worker(server.target.clone(),profile(),receiver,reports,owned.clone(),client,range()));
    let mut desired=Desired::default();desired.command("play",&play(),&profile()).unwrap();sender.send_replace(desired);
    let error=tokio::time::timeout(Duration::from_secs(2),task).await.unwrap().unwrap().unwrap_err();assert!(error.to_string().contains("another controller"));
    assert!(!owned.load(Ordering::Acquire));assert!(events.recv().await.is_none());assert_eq!(server.engine.lock().unwrap().requests.iter().filter(|name|name.as_str()=="Stop").count(),1);
}

#[tokio::test]
async fn stop_is_not_blocked_by_a_full_playback_report_channel(){
    let server=Server::start().await;let client=Client::connect(&server.target).await.unwrap();let(sender,receiver)=watch::channel(Desired::default());let(reports,_events)=mpsc::channel(1);let owned=Arc::new(AtomicBool::new(false));
    let task=tokio::spawn(worker(server.target.clone(),profile(),receiver,reports,owned.clone(),client,range()));
    let mut desired=Desired::default();desired.command("play",&play(),&profile()).unwrap();sender.send_replace(desired.clone());
    server.wait(|state|state.requests.iter().filter(|name|name.as_str()=="Status").count()>=2).await;
    desired.command("stop",&json!({}),&profile()).unwrap();sender.send_replace(desired);
    server.wait(|state|state.requests.iter().filter(|name|name.as_str()=="Stop").count()>=2).await;
    drop(sender);tokio::time::timeout(Duration::from_secs(2),task).await.unwrap().unwrap().unwrap();assert!(!owned.load(Ordering::Acquire));
}
