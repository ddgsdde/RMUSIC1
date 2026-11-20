//! The portable renderer remains usable while Core and the WebView are unavailable.
use super::*;
use futures_util::FutureExt;
use rmusic_bridge::client_queue::{Action, Queue};
use tokio::sync::{mpsc, oneshot};
use std::time::{Duration, Instant};
#[cfg(target_os="android")] mod remote_media;

fn retain_loudness(track:&mut Value,saved:&Value) {
    if track["id"]!=saved["id"] || saved["integrated_lufs"].is_null() {return;}
    track["integrated_lufs"]=saved["integrated_lufs"].clone();
    if !saved["true_peak_dbfs"].is_null() {track["true_peak_dbfs"]=saved["true_peak_dbfs"].clone();}
}

enum Message {
    Register {profile:String,endpoint:String,name:String,output:NativeOutput,reply:oneshot::Sender<CommandResult<Value>>},
    Request {profile:String,method:String,path:String,body:Value,reply:oneshot::Sender<CommandResult<Value>>},
    Report(Arc<PlaybackControl>,Value),
    Prepared {generation:u64,result:CommandResult<Value>},
    Metadata {generation:u64,track:Value},
    Recommendations {instance:String,request:u64,tracks:Vec<Value>},
    Prefetched {instance:String,next:usize,source:CommandResult<Arc<sequence::Source>>,session:String},
    Remote {profile:String,zones:Value,endpoints:Value,devices:Value},
    RemoteEvent {profile:String,event:Value},
    Loudness {profile:String,data:Value},
    System(String,Option<f64>),
    #[cfg(target_os="android")] SelectMedia { profile:String, zone:Option<String>, legacy_groups:bool },
    #[cfg(target_os="android")] MediaAction { zone:String, media:String, action:String },
    #[cfg(target_os="android")] MediaZone { profile:String, zone:Value },
    Forget(String),
    Wake { network_changed: bool },
}
#[derive(Default)]
pub(super) struct Control {
    sender:Mutex<Option<mpsc::UnboundedSender<Message>>>,active:Mutex<Option<(String,String)>>,
    listen:Mutex<Option<(ListenOwner,String,Arc<Mutex<Recorder>>)>>,
}
impl Control {
    pub fn listen_for(&self,store:Arc<ListenStore>,owner:ListenOwner,instance:String,track:&NativeTrack,duration:f64)->CommandResult<Arc<Mutex<Recorder>>> {
        let mut current=self.listen.lock().map_err(err)?;
        if let Some((saved,id,recorder))=&*current {if *saved==owner&&*id==instance{return Ok(recorder.clone());}}
        if let Some((_,_,old))=current.take(){old.lock().map_err(err)?.finish("replaced").map_err(err)?;}
        let recorder=Arc::new(Mutex::new(Recorder::new(store,owner.clone(),track.id.clone(),track.title.clone().unwrap_or_default(),track.artist.clone().unwrap_or_default(),duration)));
        *current=Some((owner,instance,recorder.clone()));Ok(recorder)
    }
    pub fn finish_listen(&self,reason:&str) {
        if let Some((_,_,recorder))=self.listen.lock().unwrap().take(){let _=recorder.lock().unwrap().finish(reason);}
    }
    fn send(&self,message:Message) {if let Some(sender)=self.sender.lock().unwrap().as_ref(){let _=sender.send(message);}}
    pub fn report(&self,control:Arc<PlaybackControl>,value:Value){self.send(Message::Report(control,value));}
    pub fn system(&self,action:String,position:Option<f64>){self.send(Message::System(action,position));}
    #[cfg(target_os="android")]
    pub(super) fn select_media(&self,profile:String,zone:Option<String>,legacy_groups:bool) {
        self.send(Message::SelectMedia { profile,zone,legacy_groups });
    }
    #[cfg(target_os="android")]
    pub(super) fn media_action(&self,zone:String,media:String,action:String) {
        self.send(Message::MediaAction { zone,media,action });
    }
    pub fn wake(&self, network_changed: bool){self.send(Message::Wake { network_changed });}
    pub fn forget(&self,profile:&str){if self.owns(profile){self.finish_listen("stopped");}self.send(Message::Forget(profile.into()));}
    pub fn owns(&self,profile:&str)->bool {self.active.lock().unwrap().as_ref().is_some_and(|(id,_)|id==profile)}
    pub fn owned(&self)->bool {self.active.lock().unwrap().is_some()}
    pub async fn register(&self,profile:String,endpoint:String,name:String,output:NativeOutput)->CommandResult<Value> {
        let (reply,result)=oneshot::channel();self.send(Message::Register {profile,endpoint,name,output,reply});result.await.map_err(err)?
    }
    pub async fn request(&self,profile:String,method:String,path:String,body:Value)->CommandResult<Value>{
        let (reply,result)=oneshot::channel();self.send(Message::Request{profile,method,path,body,reply});result.await.map_err(err)?
    }
}
struct Session {
    profile:String,owner:ListenOwner,queue:Queue,endpoint:Value,output:NativeOutput,
    remote_zones:Vec<Value>,remote_endpoints:Vec<Value>,remote_devices:Vec<Value>,import_legacy:bool,last_remote:Instant,remote_pending:bool,
    core_reachable: Option<bool>,
    remote_watch:Option<rmusic_bridge::connection::ConnectionEvents>,
    loudness_refresh:Option<tokio::task::JoinHandle<()>>,
    #[cfg(target_os="android")] remote_media:remote_media::RemoteMedia,
    portable_publish:Option<tokio::task::JoinHandle<Option<u64>>>,last_portable:Instant,portable_queue:Option<u64>,
    prefetch:Option<tokio::task::JoinHandle<()>>,next:Option<usize>,recommendations:Option<String>,
    recommendation_task:Option<tokio::task::JoinHandle<()>>,radio_request:u64,radio_retry:Option<(Instant,u64)>,waiting_end:bool,recommended_tracks:Vec<Value>,
    generation:u64,preparing:Option<tokio::task::JoinHandle<()>>,retry:u8,retry_at:Option<Instant>,
    changed:bool,queue_dirty:bool,last_save:Instant,
}
impl Drop for Session {
    fn drop(&mut self) {
        for task in [&mut self.preparing,&mut self.prefetch,&mut self.recommendation_task,&mut self.loudness_refresh] {
            if let Some(task)=task.take(){task.abort();}
        }
        if let Some(task)=self.portable_publish.take(){task.abort();}
    }
}
impl Session {
    fn device(&self)->Value {
        json!({"id":self.endpoint["id"],"name":self.endpoint["name"],"platform":"android","online":true,"renameable":false,
            "outputs":[{"id":self.endpoint["id"],"name":"系统音频输出","kind":"android_audiotrack","online":true,"default":true,"configurable":false,
                "config":self.output,"endpoint":self.endpoint}]})
    }
    fn portable_id(&self)->String {
        format!("portable-{}",self.endpoint["id"].as_str().unwrap_or_default())
    }
    fn publish_portable(&mut self,app:&tauri::AppHandle,urgent:bool) {
        if self.portable_publish.as_ref().is_some_and(|task|!task.is_finished()){return;}
        if let Some(task)=self.portable_publish.take() {
            self.portable_queue=task.now_or_never().and_then(Result::ok).flatten();
        }
        if !urgent&&self.last_portable.elapsed()<Duration::from_secs(if self.queue.zone["status"]=="playing"{3}else{25}){return;}
        let state=app.state::<DesktopState>();
        if token(&state,&self.profile).is_err(){return;}
        self.last_portable=Instant::now();
        let app=app.clone();let profile=self.profile.clone();let id=self.portable_id();
        let revision=self.queue.revision();
        let full=urgent||self.portable_queue!=Some(revision);
        let zone=if full{self.queue.zone.clone()}else{self.queue.state()};
        let body=json!({"platform":"android","zone":zone,"stamp":portable_now()});
        self.portable_publish=Some(tokio::spawn(async move{
            let state=app.state::<DesktopState>();
            tokio::time::timeout(Duration::from_secs(5),core_api_request(&state,profile,"PUT".into(),format!("api/v1/portable/{id}/state"),Some(body),None))
                .await.is_ok_and(|result|result.is_ok()).then_some(revision)
        }));
    }
    async fn portable_command(&mut self,app:&tauri::AppHandle,data:&Value){
        let id=self.portable_id();
        let Some(request)=data["request_id"].as_str()else{return;};
        if data["id"]!=id||data["expires_at"].as_i64().unwrap_or(0)<portable_now(){return;}
        let method=data["method"].as_str().unwrap_or("");
        let action=data["action"].as_str().unwrap_or("");
        let result=self.action(app,method,action,data["body"].clone()).await;
        let body=match result{Ok(_)=>json!({"request_id":request,"stamp":portable_now(),"zone":self.queue.zone}),
            Err(error)=>json!({"request_id":request,"stamp":portable_now(),"error":error.to_string()})};
        let app=app.clone();let profile=self.profile.clone();
        tokio::spawn(async move{
            let state=app.state::<DesktopState>();
            let _=tokio::time::timeout(Duration::from_secs(5),core_api_request(&state,profile,"POST".into(),format!("api/v1/portable/{id}/ack"),Some(body),None)).await;
        });
    }
    fn portable_loudness(&mut self,app:&tauri::AppHandle,data:&Value) {
        let (Some(track),Some(lufs))=(data["track_id"].as_str(),data["integrated_lufs"].as_f64()) else{return;};
        let peak=data["true_peak_dbfs"].as_f64();
        let update=|value:&mut Value|{
            value["integrated_lufs"]=json!(lufs);
            value["true_peak_dbfs"]=json!(peak);
        };
        if let Some(queue)=self.queue.zone["queue"].as_array_mut(){
            for row in queue.iter_mut().filter(|row|row["id"]==track) {update(row);self.queue_dirty=true;}
        }
        let current=&mut self.queue.zone["track"];
        if current["id"]==track {
            update(current);
            self.apply_current_loudness(&app.state::<DesktopState>());
        }
        if self.queue_dirty { self.changed=true; self.publish_value(app,&self.queue.zone); }
    }
    fn apply_current_loudness(&self,state:&DesktopState) {
        let zone=&self.queue.zone;
        let (Some(id),Some(lufs))=(zone["playback_id"].as_str(),zone["track"]["integrated_lufs"].as_f64()) else{return;};
        let controls=state.playback_controls.lock().unwrap();
        if controls.playback_id==id && controls.profile_id.as_ref()==Some(&self.profile) {
            let peak=zone["track"]["true_peak_dbfs"].as_f64();
            let _=controls.control.set_loudness("single",lufs,peak);
        }
    }
    fn refresh_loudness(&mut self,app:&tauri::AppHandle) {
        let zone=&self.queue.zone;
        if zone["status"]!="playing"||zone["dsp"]["enabled"]!=true||zone["dsp"]["normalize"]!=true
            ||self.loudness_refresh.as_ref().is_some_and(|task|!task.is_finished()) {return;}
        let next=self.queue.next(true).and_then(|index|self.queue.tracks().get(index));
        let tracks=std::iter::once(&zone["track"]).chain(next)
            .filter(|track|track["integrated_lufs"].is_null())
            .filter_map(|track|track["id"].as_str().map(str::to_owned)).collect::<Vec<_>>();
        if tracks.is_empty(){return;}
        let profile=self.profile.clone();let app=app.clone();
        self.loudness_refresh=Some(tokio::spawn(async move{
            let state=app.state::<DesktopState>();
            for track in tracks {
                let path=format!("api/v1/library/tracks/{}",urlencoding(&track));
                if let Ok(Ok(value))=tokio::time::timeout(Duration::from_secs(3),core_api_request(&state,profile.clone(),"GET".into(),path,None,None)).await {
                    if value["integrated_lufs"].is_number(){
                        state.travel.send(Message::Loudness{profile:profile.clone(),data:json!({"track_id":track,
                            "integrated_lufs":value["integrated_lufs"],"true_peak_dbfs":value["provider_data"]["analysis"]["loudness"]["true_peak_dbfs"]})});
                    }
                }
            }
        }));
    }
    fn remote_zones(&mut self,rows:&[Value]) {
        if self.import_legacy {
            if let Some(old)=rows.iter().find(|z|z["endpoint_id"]==self.endpoint["id"]&&!z["queue"].as_array().is_none_or(Vec::is_empty)) {
                let id=self.queue.zone["id"].clone();self.queue.zone=old.clone();self.queue.zone["id"]=id;self.queue.restart();self.changed=true;self.queue_dirty=true;
            }
            self.import_legacy=false;
        }
        let portable=self.portable_id();
        self.remote_zones=rows.iter().filter(|z|z["endpoint_id"]!=self.endpoint["id"]&&z["endpoint_id"]!=portable).cloned().collect();
    }
    fn watch_remote(&mut self,app:&tauri::AppHandle) {
        #[cfg(target_os="android")] if !android::foreground()&&self.queue.zone["status"]=="stopped"&&!self.remote_media.selected(){self.remote_watch=None;return;}
        if self.remote_watch.is_some(){return;}
        let state=app.state::<DesktopState>();
        let (Ok(saved),Ok(token))=(super::profile(&state,&self.profile),token(&state,&self.profile))else{return};
        let app=app.clone();let profile=self.profile.clone();
        self.remote_watch=Some(saved.follow_events(token,Arc::new(move|event|app.state::<DesktopState>().travel.send(Message::RemoteEvent{profile:profile.clone(),event}))));
    }
    #[cfg(target_os="android")]
    fn publish_remote_media(&mut self,app:&tauri::AppHandle) {
        self.remote_media.publish(app,&self.profile,&self.remote_zones,&self.remote_endpoints);
    }
    fn persist(&mut self,state:&DesktopState)->CommandResult<()> {
        let key=format!("client-queue:{}:{}:{}",self.owner.core_id,self.owner.user_id,self.endpoint["id"].as_str().unwrap());
        self.queue.save(&state.listens,&key,self.queue_dirty).map_err(err)?;
        self.changed=false;self.queue_dirty=false;self.last_save=Instant::now();Ok(())
    }
    fn publish(&self,app:&tauri::AppHandle){
        #[cfg(target_os="android")] if !self.remote_media.selected(){android::update_artwork(app,&self.profile,&self.queue.zone["track"]);}
        self.publish_value(app,&self.queue.zone);
    }
    fn publish_value(&self,app:&tauri::AppHandle,value:&Value){
        #[cfg(target_os="android")] if !android::foreground(){return;}
        let _=app.emit("native-client-zone",value);
    }
    fn stop_preparation(&mut self,state:&DesktopState){
        self.generation=self.generation.wrapping_add(1);self.retry_at=None;
        if let Some(task)=self.preparing.take(){task.abort();}
        state.playback_generation.fetch_add(1,Ordering::AcqRel);
    }
    fn start(&mut self,app:&tauri::AppHandle) {
        let state=app.state::<DesktopState>();self.stop_preparation(&state);
        self.queue.zone["buffering"]=json!(true);
        self.queue.zone.as_object_mut().unwrap().remove("error");
        let generation=self.generation;let zone=self.queue.state();
        let profile=self.profile.clone();let output=self.output.clone();let handle=app.clone();
        self.preparing=Some(tokio::spawn(async move {
            let state=handle.state::<DesktopState>();
            let result=async {
                let id=zone["track"]["id"].as_str().ok_or("缺少歌曲")?;
                let owner=s_owner(&state,&profile)?;
                let cached=state.cache.lock().map_err(err)?.manifest.entries.values().find(|e|e.owner.as_ref()==Some(&owner)&&e.track_id==id&&e.complete).cloned();
                let mut track=if let Some(cached)=cached {let mut t=zone["track"].clone();t["title"]=json!(cached.title);t["artist"]=json!(cached.artist);t["recording_id"]=json!(cached.recording_id);t["bit_depth"]=json!(cached.bit_depth);t["sample_rate"]=json!(cached.sample_rate);t["channels"]=json!(cached.channels);t["format"]=json!(cached.format);t}else{match core_api_request(&state,profile.clone(),"GET".into(),format!("api/v1/library/tracks/{}",urlencoding(id)),None,None).await {
                    Ok(value)=>value,Err(error) if !error.starts_with("HTTP 401")=>zone["track"].clone(),Err(e)=>return Err(e)
                }};
                if track["true_peak_dbfs"].is_null() {
                    track["true_peak_dbfs"]=track["provider_data"]["analysis"]["loudness"]["true_peak_dbfs"].clone();
                }
                retain_loudness(&mut track,&zone["track"]);
                state.travel.send(Message::Metadata{generation,track:track.clone()});
                let track:NativeTrack=serde_json::from_value(track).map_err(err)?;
                native_play_inner(zone["playback_id"].as_str().map(String::from),None,profile,track,
                    zone["position"].as_f64(),Some(serde_json::from_value(zone["dsp"].clone()).map_err(err)?),Some(output),
                    zone["volume"].as_f64(),zone["muted"].as_bool(),Some(zone["status"]!="playing"),
                    Some(serde_json::from_value(zone["transitions"].clone()).map_err(err)?),handle.clone(),&state,None,None).await
            }.await;
            state.travel.send(Message::Prepared{generation,result});
        }));
        self.publish(app);
    }
    async fn action(&mut self,app:&tauri::AppHandle,method:&str,action:&str,body:Value)->CommandResult<Value>{
        let state=app.state::<DesktopState>();
        if s_owner(&state,&self.profile)?!=self.owner{return Err("本机账号已切换".into());}
        let old_length=self.queue.tracks().len();
        let mut added_radio=false;
        if action=="ended"&&self.queue.next(true).is_none()&&!self.recommended_tracks.is_empty() {
            let trim=old_length.saturating_sub(200).min(self.queue.index());
            if trim>0 {
                self.queue.zone["queue_index"]=json!(self.queue.index()-trim);
                self.queue.zone["queue"].as_array_mut().unwrap().drain(..trim);
            }
            self.queue.zone["queue"].as_array_mut().unwrap().append(&mut self.recommended_tracks);
            added_radio=true;
        }
        let was_waiting=self.waiting_end;
        let mut effect=if action=="ended"&&self.next.is_some_and(|n|n<self.queue.tracks().len()){
            self.queue.select(self.next.take().unwrap()).map_err(err)?;Action::Start
        }else{self.queue.command(method,action,&body).map_err(err)?};
        self.queue_dirty|=action.starts_with("queue")||body.get("shuffle").is_some()||old_length!=self.queue.tracks().len()||added_radio;
        if action!="progress"&&(action!=""||body.get("repeat").is_some()||body.get("shuffle").is_some()||body.get("autoplay").is_some()) {
            if let Some(task)=self.prefetch.take(){task.abort();}
            if let Some(task)=self.recommendation_task.take(){task.abort();}
            self.radio_request+=1;self.radio_retry=None;self.waiting_end=false;
            self.recommendations=None;self.recommended_tracks.clear();
            if action!="ended"{self.next=None;}
        }
        if was_waiting && effect==Action::None {
            if (action.starts_with("queue")||action.is_empty()&&body.get("repeat").is_some()) && self.queue.next(true).is_some() {
                effect=self.queue.command("POST","ended",&Value::Null).map_err(err)?;
            }else if action.is_empty()&&body["autoplay"]["enabled"]==false{
                effect=self.queue.command("POST","ended",&Value::Null).map_err(err)?;
            }else if action.starts_with("queue") && self.queue.zone["status"]=="playing" && self.queue.zone["autoplay"]["enabled"]==true {
                self.waiting_end=true;self.prepare_next(app,false);
            }
        }
        match effect {
            Action::Start=>{self.retry=0;self.start(app);}
            Action::Pause=>{state.pause(true)?;if self.preparing.is_some(){self.stop_preparation(&state);}}
            Action::Resume=>{
                if state.player.lock().unwrap().as_ref().is_some_and(|p|!p.retired&&p.playback_id==self.queue.zone["playback_id"]&&p.thread.as_ref().is_some_and(|t|!t.is_finished())){state.pause(false)?;}
                else{self.start(app);}
            }
            Action::Seek(_)=>{let paused=self.queue.zone["status"]=="paused";self.start(app);if paused{state.pause(true)?;}}
            Action::Stop=>{
                self.stop_preparation(&state);
                state.client_prepared.lock().map_err(err)?.take();
                let player=state.player.lock().map_err(err)?.take();
                if let Some(player)=player{player.stop(Some("stopped")).await;}
                state.travel.finish_listen("stopped");
            }
            Action::Volume=>{state.volume(self.queue.zone["volume"].as_f64(),self.queue.zone["muted"].as_bool())?;}
            Action::None=>{},
        }
        if matches!(effect,Action::Pause|Action::Stop){self.queue.zone["buffering"]=json!(false);}
        let snapshot=if self.queue_dirty{self.queue.zone.clone()}else{self.queue.state()};
        self.changed=true;self.persist(&state)?;self.publish_value(app,&snapshot);self.publish_portable(app,true);Ok(snapshot)
    }
    fn prepare_next(&mut self,app:&tauri::AppHandle,allow_media:bool){
        let instance=self.queue.zone["playback_id"].as_str().unwrap_or("").to_owned();
        if let Some(next)=self.queue.next(true).filter(|_|allow_media&&self.prefetch.is_none()) {
            self.next=Some(next);let track=self.queue.tracks()[next].clone();let profile=self.profile.clone();let output=self.output.clone();
            let app=app.clone();let instance=instance.clone();
            self.prefetch=Some(tokio::spawn(async move{
                let state=app.state::<DesktopState>();let session=token(&state,&profile).unwrap_or_default();
                let result=async{
                    let owner=state.listens.owner(&profile).map_err(err)?;
                    let cached=state.cache.lock().map_err(err)?.manifest.entries.values().any(|e|e.profile_id==profile&&e.owner==owner&&e.complete&&(1..=8).contains(&e.channels)&&Some(e.track_id.as_str())==track["id"].as_str());
                    let selected=if cached{super::profile(&state,&profile)?}else{connected_profile(&state,&profile).await?};
                    let track=if cached{track}else{core_api_request(&state,profile.clone(),"GET".into(),format!("api/v1/library/tracks/{}",urlencoding(track["id"].as_str().unwrap_or(""))),None,None).await.unwrap_or(track)};
                    let source=prepare_source(app.clone(),state.cache.clone(),selected,session.clone(),profile.clone(),state.listens.owner(&profile).map_err(err)?,
                        serde_json::from_value(track).map_err(err)?,&output.clone().into(),true,true).await?;
                    source.proxy.prefetch_start().await.map_err(err)?;Ok(Arc::new(source))
                }.await;
                state.travel.send(Message::Prefetched{instance,next,source:result,session});
            }));
        }
        if self.queue.zone["repeat"]=="off" && self.queue.tracks().len().saturating_sub(self.queue.index()+1)<=3
            && self.queue.zone["autoplay"]["enabled"]==true && self.recommendations.as_deref()!=Some(&instance){
            self.recommendations=Some(instance.clone());let profile=self.profile.clone();let owner=self.owner.clone();let app=app.clone();
            let mut recent=self.queue.tracks().iter().rev().take(200).cloned().collect::<Vec<_>>();
            let ids=recent.iter().filter_map(|track|track["id"].as_str()).collect::<Vec<_>>();
            let body=json!({"seed_track_id":self.queue.tracks().last().and_then(|track|track["id"].as_str()),"context":{"tail":ids.iter().take(12).collect::<Vec<_>>(),"recent":ids}});
            let playing=self.queue.zone["track"]["id"].clone();
            self.radio_request+=1;
            let request=self.radio_request;
            self.recommendation_task=Some(tokio::spawn(async move{
                let state=app.state::<DesktopState>();
                let response=tokio::time::timeout(Duration::from_secs(4),core_api_request(&state,profile.clone(),"POST".into(),"api/v1/recommendations/radio".into(),Some(body),None)).await;
                let mut tracks=response.ok().and_then(Result::ok).and_then(|v|v["items"].as_array().cloned()).unwrap_or_default();
                if tracks.is_empty() {
                    if let Ok(cache)=state.cache.lock(){
                        let cached=cache.manifest.entries.values().filter(|entry|entry.profile_id==profile&&entry.owner.as_ref()==Some(&owner)&&entry.complete&&(1..=8).contains(&entry.channels)).map(|entry|entry.track_id.as_str()).collect::<std::collections::HashSet<_>>();
                        let mut picked=std::collections::HashSet::new();
                        recent.reverse();
                        tracks.extend(recent.iter().filter(|track|track["id"]!=playing&&track["id"].as_str().is_some_and(|id|cached.contains(id)&&picked.insert(id))).take(5).cloned());
                        if tracks.is_empty(){tracks.extend(recent.iter().filter(|track|track["id"]==playing&&track["id"].as_str().is_some_and(|id|cached.contains(id))).take(1).cloned());}
                    }
                }
                state.travel.send(Message::Recommendations{instance,request,tracks});
            }));
        }
    }
    fn remote(&mut self,app:&tauri::AppHandle,force:bool){
        if self.remote_pending||!force&&self.last_remote.elapsed()<Duration::from_secs(300){return;}
        self.remote_pending=true;self.last_remote=Instant::now();let app=app.clone();let profile=self.profile.clone();
        tokio::spawn(async move{
            let state=app.state::<DesktopState>();
            let zones=core_api_request(&state,profile.clone(),"GET".into(),"api/v1/zones".into(),None,None);
            let endpoints=core_api_request(&state,profile.clone(),"GET".into(),"api/v1/endpoints".into(),None,None);
            let devices=core_api_request(&state,profile.clone(),"GET".into(),"api/v1/devices".into(),None,None);
            let (zones,endpoints,devices)=tokio::join!(zones,endpoints,devices);
            state.travel.send(Message::Remote {profile,zones:zones.unwrap_or(Value::Null),endpoints:endpoints.unwrap_or(Value::Null),devices:devices.unwrap_or(Value::Null)});
        });
    }
}
fn portable_now()->i64 {std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0,|v|v.as_millis() as i64)}
fn s_owner(state:&DesktopState,profile:&str)->CommandResult<ListenOwner>{state.listens.owner(profile).map_err(err)?.ok_or_else(||"本机账号已切换".into())}
fn urlencoding(id:&str)->String{id.bytes().map(|b|if b.is_ascii_alphanumeric(){(b as char).to_string()}else{format!("%{b:02X}")}).collect()}
pub(super) fn install(app:tauri::AppHandle){
    let (sender,mut messages)=mpsc::unbounded_channel();
    *app.state::<DesktopState>().travel.sender.lock().unwrap()=Some(sender);
    tauri::async_runtime::spawn(async move{
        let mut session:Option<Session>=None;
        let mut tick=tokio::time::interval(Duration::from_secs(1));tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop{
            let message=tokio::select!{m=messages.recv()=>{let Some(m)=m else{break};Some(m)},_=tick.tick(),if session.is_some()=>None};
            let state=app.state::<DesktopState>();
            match message {
                Some(Message::Register{profile,endpoint,name,output,reply})=>{
                    let result=(||->CommandResult<Value>{
                        let owner=state.listens.owner(&profile).map_err(err)?.ok_or("请先登录")?;
                        if let Some(s)=session.as_mut().filter(|s|s.profile==profile&&s.owner==owner&&s.endpoint["id"]==endpoint){
                            s.output=output.clone();s.endpoint["output_config"]=json!(output);return Ok(s.endpoint.clone());
                        }
                        if let Some(s)=&mut session{s.stop_preparation(&state);s.persist(&state)?;if let Some(player)=state.player.lock().unwrap().as_ref(){player.control.stop();}}
                        let key=format!("client-queue:{}:{}:{endpoint}",owner.core_id,owner.user_id);
                        let saved=Queue::load(&state.listens,&key).map_err(err)?;
                        let import_legacy=saved.is_none();let mut queue=saved.unwrap_or_else(||Queue::new(&endpoint,&name));
                        queue.restart();
                        let endpoint=json!({"id":endpoint,"name":name,"kind":"browser","online":true,"output_config":output,
                            "capabilities":{"native_client_v1":true,"platform":"android","playback_instance":true}});
                        session=Some(Session {profile:profile.clone(),owner,queue,endpoint:endpoint.clone(),output,remote_zones:vec![],remote_endpoints:vec![],remote_devices:vec![],import_legacy,
                            last_remote:Instant::now()-Duration::from_secs(600),remote_pending:false,remote_watch:None,loudness_refresh:None,core_reachable:None,
                            #[cfg(target_os="android")] remote_media:Default::default(),
                            portable_publish:None,last_portable:Instant::now()-Duration::from_secs(30),portable_queue:None,prefetch:None,next:None,recommendations:None,recommendation_task:None,radio_request:0,radio_retry:None,waiting_end:false,recommended_tracks:vec![],generation:0,preparing:None,retry:0,retry_at:None,changed:false,queue_dirty:import_legacy,last_save:Instant::now()});
                        *state.travel.active.lock().unwrap()=Some((profile,endpoint["id"].as_str().unwrap().into()));
                        if let Some(s)=&mut session{s.watch_remote(&app);s.remote(&app,true);s.publish(&app);s.publish_portable(&app,true);}
                        Ok(endpoint)
                    })();let _=reply.send(result);
                }
                Some(Message::Request{profile,method,path,body,reply})=>{
                    let result=if let Some(s)=session.as_mut().filter(|s|s.profile==profile) {
                        let stem=format!("api/v1/zones/{}",s.queue.zone["id"].as_str().unwrap());
                        if path=="api/v1/zones" {s.remote(&app,false);let mut rows=s.remote_zones.clone();rows.push(s.queue.zone.clone());Ok(json!({"items":rows,"core_reachable":s.core_reachable}))}
                        else if path=="api/v1/endpoints" {s.remote(&app,false);let mut rows=s.remote_endpoints.clone();rows.push(s.endpoint.clone());Ok(json!({"items":rows}))}
                        else if path=="api/v1/devices" {s.remote(&app,false);let mut rows=s.remote_devices.clone();rows.push(s.device());Ok(json!({"items":rows}))}
                        else if path==stem&&method=="GET"{Ok(s.queue.zone.clone())}
                        else if path==format!("{stem}/state")&&method=="GET"{Ok(s.queue.state())}
                        else if path==format!("{stem}/radio") {
                            if method=="POST"&&s.recommended_tracks.first().is_some_and(|track|track["id"]==body["track_id"]){s.recommended_tracks.remove(0);}
                            Ok(json!({"status":if !s.recommended_tracks.is_empty(){"ready"}else if s.waiting_end{"waiting"}else if s.recommendation_task.is_some(){"preparing"}else{"idle"},"items":s.recommended_tracks.iter().take(3).collect::<Vec<_>>()}))
                        }
                        else if path==stem||path.starts_with(&(stem.clone()+"/")){s.action(&app,&method,path.strip_prefix(&stem).unwrap().trim_start_matches('/'),body).await}
                        else {Err("未知的本机播放路径".into())}
                    }else{Err("本机账号已切换".into())};let _=reply.send(result);
                }
                Some(Message::Recommendations{instance,request,tracks})=>if let Some(s)=session.as_mut().filter(|s|s.queue.zone["playback_id"]==instance&&s.recommendations.as_deref()==Some(&instance)&&s.radio_request==request){
                    s.recommended_tracks=tracks;
                    s.recommendation_task=None;
                    if s.waiting_end && s.recommended_tracks.is_empty(){
                        let delay=s.radio_retry.map_or(5,|(_,delay)|(delay*2).min(30));
                        s.radio_retry=Some((Instant::now()+Duration::from_secs(delay),delay));
                    } else if s.waiting_end {
                        let _=s.action(&app,"POST","ended",Value::Null).await;
                    }
                    let _=app.emit("native-core-event",json!({"type":"autoplay"}));
                }
                Some(Message::Prefetched{instance,next,source,session:token})=>if let Some(s)=session.as_mut().filter(|s|s.queue.zone["playback_id"]==instance&&s.next==Some(next)){
                    if let Ok(source)=source {
                        let output:OutputConfig=s.output.clone().into();
                        let i=next;
                        if let Ok(mut metadata)=serde_json::to_value(&source.track){
                            retain_loudness(&mut metadata,&s.queue.zone["queue"][i]);
                            if let Value::Object(metadata)=metadata{for(k,v)in metadata{if !v.is_null()&&s.queue.zone["queue"][i][k.as_str()]!=v{s.queue.zone["queue"][i][k]=v;s.queue_dirty=true;s.changed=true;}}}
                        }
                        *state.client_prepared.lock().unwrap()=Some((s.profile.clone(),token,output,source));
                    }
                }
                Some(Message::Metadata{generation,mut track})=>if let Some(s)=session.as_mut().filter(|s|s.generation==generation){
                    let i=s.queue.index();if track["id"]==s.queue.zone["track"]["id"]{retain_loudness(&mut track,&s.queue.zone["track"]);s.queue_dirty|=s.queue.zone["queue"][i]!=track;s.queue.zone["queue"][i]=track.clone();s.queue.zone["track"]=track;s.changed=true;s.publish(&app);}
                }
                Some(Message::Prepared{generation,result})=>if let Some(s)=session.as_mut().filter(|s|s.generation==generation){
                    s.preparing=None;
                    match result {
                        Ok(value)=>if let Some(fields)=value["metadata"].as_object(){
                            let index=s.queue.index();for(k,v)in fields{if !v.is_null(){s.queue_dirty|=s.queue.zone["queue"][index][k]!=*v;s.queue.zone["track"][k]=v.clone();s.queue.zone["queue"][index][k]=v.clone();}}
                            s.apply_current_loudness(&state);
                            s.changed=true;s.publish(&app);
                        },
                        Err(error)=>{s.queue.zone["error"]=json!(error);if !error.starts_with("HTTP 401")&&!error.contains("离线目录")&&s.retry<4 {s.retry+=1;s.retry_at=Some(Instant::now()+Duration::from_secs(1<<s.retry));}else{s.queue.zone["status"]=json!("paused");s.queue.zone["buffering"]=json!(false);let _=app.emit("native-cache-error",&error);}s.publish(&app);}
                    }
                }
                Some(Message::Report(control,report))=>if let Some(s)=session.as_mut().filter(|s|s.preparing.is_none()&&s.queue.zone["playback_id"]==report["playback_id"]&&Arc::ptr_eq(&control,&state.playback_controls.lock().unwrap().control)){
                    if let Some(position)=report["position"].as_f64(){s.queue.zone["position"]=json!(position);s.changed=true;}
                    #[cfg(target_os="android")] if android::foreground(){let _=app.emit("native-client-state",&report);}
                    s.publish_portable(&app,false);
                    if s.changed&&s.last_save.elapsed()>=Duration::from_secs(15){let _=s.persist(&state);}
                    if report["status"]=="playing" {
                        if s.queue.zone["buffering"]==true {s.queue.zone["buffering"]=json!(false);s.publish(&app);s.publish_portable(&app,true);}
                        let remaining=s.queue.zone["track"]["duration"].as_f64().unwrap_or(0.)-s.queue.zone["position"].as_f64().unwrap_or(0.);
                        if report["paused"]!=true&&remaining>0.&&remaining<20.
                            && (report["filling_current"]!=true||s.queue.tracks().len().saturating_sub(s.queue.index()+1)<=3) {
                            s.prepare_next(&app,report["filling_current"]!=true);
                        }
                    }
                    if report["status"]=="ended" {
                        if s.queue.next(true).is_none()&&s.queue.zone["autoplay"]["enabled"]==true&&s.recommended_tracks.is_empty() {
                            s.waiting_end=true;s.queue.zone["buffering"]=json!(true);
                            if s.recommendation_task.is_none(){s.recommendations=None;}
                            s.prepare_next(&app,false);s.publish(&app);s.publish_portable(&app,true);
                        }else{let _=s.action(&app,"POST","ended",Value::Null).await;}
                    }
                    else if report["status"]=="error" {
                        if report["source_failed"]==true&&s.retry<4 {s.retry+=1;s.queue.zone["buffering"]=json!(true);s.retry_at=Some(Instant::now()+Duration::from_secs(1<<s.retry));}
                        else{s.queue.zone["status"]=json!("paused");s.queue.zone["buffering"]=json!(false);s.queue.zone["error"]=report["error"].clone();let _=app.emit("native-cache-error",report["error"].as_str().unwrap_or("本机播放已暂停，请检查音源或输出"));}
                        s.publish(&app);
                    }
                }
                Some(Message::System(action,position))=>if let Some(s)=&mut session{let action=if action=="resume"{"play"}else{action.as_str()};let _=s.action(&app,"POST",action,json!({"position":position})).await;}
                #[cfg(target_os="android")]
                Some(Message::SelectMedia{profile,zone,legacy_groups})=>if let Some(s)=session.as_mut().filter(|s|s.profile==profile){
                    s.remote_media.select(zone,legacy_groups);
                    s.publish_remote_media(&app);
                    if !s.remote_media.selected(){s.publish(&app);}
                }
                #[cfg(target_os="android")]
                Some(Message::MediaAction{zone,media,action})=>if let Some(s)=&mut session {
                    s.remote_media.command(&app,&s.profile,zone,media,action);
                }
                #[cfg(target_os="android")]
                Some(Message::MediaZone{profile,zone})=>if let Some(s)=session.as_mut().filter(|s|s.profile==profile) {
                    if let Some(current)=s.remote_zones.iter_mut().find(|z|z["id"]==zone["id"]){
                        if zone["updated_at"].as_i64()>=current["updated_at"].as_i64() {*current=zone;}
                    }
                    s.publish_remote_media(&app);
                    let _=app.emit("native-client-remote",Value::Null);
                }
                Some(Message::Remote{profile,zones,endpoints,devices})=>if let Some(s)=session.as_mut().filter(|s|s.profile==profile){
                    s.remote_pending=false;if let Some(items)=zones["items"].as_array(){
                        s.core_reachable=Some(true);
                        s.remote_zones(items);
                        #[cfg(target_os="android")] s.remote_media.connected(true);
                    }
                    let portable=s.portable_id();
                    if let Some(items)=endpoints["items"].as_array(){s.remote_endpoints=items.iter().filter(|e|e["id"]!=s.endpoint["id"]&&e["id"]!=portable).cloned().collect();}
                    if let Some(items)=devices["items"].as_array(){s.remote_devices=items.iter().filter(|d|d["id"]!=s.endpoint["id"]&&d["id"]!=portable&&!d["outputs"].as_array().is_some_and(|outputs|outputs.iter().any(|o|o["id"]==s.endpoint["id"]))).cloned().collect();}
                    #[cfg(target_os="android")] s.publish_remote_media(&app);
                    let _=app.emit("native-client-remote",Value::Null);
                }
                Some(Message::Loudness{profile,data})=>if let Some(s)=session.as_mut().filter(|s|s.profile==profile){
                    s.portable_loudness(&app,&data);
                }
                Some(Message::RemoteEvent{profile,event})=>if let Some(s)=session.as_mut().filter(|s|s.profile==profile){
                    match event["type"].as_str().unwrap_or("") {
                        "portable_command"=>s.portable_command(&app,&event["data"]).await,
                        "portable_loudness"=>s.portable_loudness(&app,&event["data"]),
                        "connection_ready"=>{
                            s.core_reachable=Some(true);
                            s.refresh_loudness(&app);
                            #[cfg(target_os="android")] if s.remote_media.selected(){s.remote(&app,true);}
                            s.publish_portable(&app,true);let _=app.emit("native-core-event",&event);
                        },
                        "connection_interrupted"=>{
                            s.core_reachable=Some(false);
                            #[cfg(target_os="android")] {s.remote_media.connected(false);s.publish_remote_media(&app);}
                            let _=app.emit("native-core-event",&event);
                        },
                        "zones"=>if let Some(rows)=event["data"]["items"].as_array(){
                            s.core_reachable=Some(true);
                            s.remote_zones(rows);
                            #[cfg(target_os="android")] {s.remote_media.connected(true);s.publish_remote_media(&app);}
                            let _=app.emit("native-client-remote",Value::Null);
                        },
                        "zone_progress"=>{
                            if let Some(z)=s.remote_zones.iter_mut().find(|z|z["id"]==event["data"]["id"]){
                                if event["data"].get("playback_id").is_none()||z["playback_id"]==event["data"]["playback_id"] {
                                    if let Some(fields)=event["data"].as_object(){for(k,v)in fields{z[k]=v.clone();}}
                                    let _=app.emit("native-core-event",&event);
                                }
                            }
                            #[cfg(target_os="android")] s.publish_remote_media(&app);
                        },
                        "endpoints"|"devices"|"device_groups"|"resync"|"zones_changed"=>{
                            s.remote(&app,true);
                            let _=app.emit("native-core-event",&event);
                        },
                        "endpoint_signal"|"device_volume"=>{
                            if let Some(endpoint)=s.remote_endpoints.iter_mut().find(|e|e["id"]==event["data"]["endpoint_id"]){
                                if event["type"]=="endpoint_signal"{endpoint["playback"]=event["data"]["playback"].clone();}else{endpoint["device_volume"]=event["data"]["state"].clone();}
                            }
                            #[cfg(target_os="android")] s.publish_remote_media(&app);
                            let _=app.emit("native-core-event",&event);
                        },
                        "library"=>{s.refresh_loudness(&app);let _=app.emit("native-core-event",&event);},
                        "playlists"|"stats"|"autoplay"=>{let _=app.emit("native-core-event",&event);},
                        _=>{}
                    }
                }
                Some(Message::Forget(profile))=>if session.as_ref().is_some_and(|s|s.profile==profile){
                    if let Some(mut s)=session.take(){s.stop_preparation(&state);let _=s.persist(&state);}
                    let player=state.player.lock().unwrap().take();if let Some(player)=player{player.stop(Some("stopped")).await;}
                    *state.client_prepared.lock().unwrap()=None;
                    *state.travel.active.lock().unwrap()=None;
                }
                Some(Message::Wake{network_changed})=>if let Some(s)=&mut session{
                    if network_changed {s.remote_watch=None;}
                    s.watch_remote(&app);
                    #[cfg(target_os="android")] if !android::foreground(){continue;}
                    s.publish(&app);s.remote(&app,true);if s.retry_at.is_some(){s.retry_at=Some(Instant::now());}
                }
                None=>if let Some(s)=&mut session{
                    s.publish_portable(&app,false);
                    if s.changed&&s.last_save.elapsed()>=Duration::from_secs(15){let _=s.persist(&state);}
                    if s.retry_at.is_some_and(|at|Instant::now()>=at){s.start(&app);}
                    if s.waiting_end&&s.recommendation_task.is_none()&&s.radio_retry.is_some_and(|(at,_)|Instant::now()>=at){
                        s.recommendations=None;s.prepare_next(&app,false);
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portable_retries_keep_one_record_when_preparation_fails_between_output_workers() {
        let root=tempfile::tempdir().unwrap();let store=ListenStore::open(root.path()).unwrap();let control=Control::default();
        let owner=ListenOwner{core_id:"core".into(),user_id:"user".into()};
        let track:NativeTrack=serde_json::from_value(json!({"id":"song","title":"曲目","artist":"作者","recording_id":"","format":"flac","sample_rate":48000,"channels":2})).unwrap();
        let mut queue=Queue::new("qa","手机");queue.command("POST","play",&json!({"track_id":"not-yet-loaded"})).unwrap();
        assert!(serde_json::from_value::<NativeTrack>(queue.zone["track"].clone()).is_ok());
        let first=control.listen_for(store.clone(),owner.clone(),"one".into(),&track,120.).unwrap();
        first.lock().unwrap().sample(8.,true,false).unwrap();first.lock().unwrap().checkpoint().unwrap();
        let retry=control.listen_for(store.clone(),owner.clone(),"one".into(),&track,120.).unwrap();assert!(Arc::ptr_eq(&first,&retry));
        control.listen_for(store.clone(),owner.clone(),"two".into(),&track,120.).unwrap();control.finish_listen("stopped");store.drain().unwrap();
        let rows=store.pending(&owner).unwrap();assert_eq!(rows.len(),2);assert!(rows.iter().any(|r|r.snapshot.reason=="replaced"));
    }
}
