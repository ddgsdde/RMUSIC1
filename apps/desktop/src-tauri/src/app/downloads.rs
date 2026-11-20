//! App-owned original downloads. Journal contains identity and progress, never sessions or URLs.
use super::*;
use std::{collections::HashSet,path::{Path,PathBuf},sync::atomic::AtomicBool};
#[derive(Clone,Serialize,Deserialize)]
struct Task {id:String,profile_id:String,owner:ListenOwner,track:NativeTrack,status:String,cache_id:Option<String>,received:u64,total:u64,error:Option<String>}
pub(super) struct Control {path:PathBuf,tasks:Mutex<Vec<Task>>,active:Mutex<HashMap<String,Arc<AtomicBool>>>,authorized:Mutex<HashSet<String>>,slots:Arc<tokio::sync::Semaphore>,wake:tokio::sync::Notify}
impl Control {
 pub fn open(root:&Path)->Result<Self>{
  let path=root.join("downloads.json");let mut tasks:Vec<Task>=if path.exists(){serde_json::from_slice(&std::fs::read(&path)?)?}else{vec![]};
  for task in &mut tasks {if matches!(task.status.as_str(),"queued"|"downloading"|"verifying"){task.status="interrupted".into();}}
  Ok(Self{path,tasks:Mutex::new(tasks),active:Default::default(),authorized:Default::default(),slots:Arc::new(tokio::sync::Semaphore::new(2)),wake:Default::default()})
 }
 fn save(&self,tasks:&[Task])->CommandResult<()> {use std::io::Write;let path=self.path.with_extension("next");let mut file=std::fs::File::create(&path).map_err(err)?;file.write_all(&serde_json::to_vec(tasks).map_err(err)?).map_err(err)?;file.sync_all().map_err(err)?;std::fs::rename(path,&self.path).map_err(err)}
 fn update(&self,id:&str,f:impl FnOnce(&mut Task))->CommandResult<()> {let mut tasks=self.tasks.lock().map_err(err)?;if let Some(task)=tasks.iter_mut().find(|t|t.id==id){f(task);}self.save(&tasks)}
 pub fn bind_legacy_cache(&self,cache:&mut CacheStore)->Result<()>{let mut owners=vec![];for task in self.tasks.lock().map_err(|_|anyhow::anyhow!("下载记录无法读取"))?.iter(){if let Some(id)=task.cache_id.as_ref(){if cache.manifest.entries.get(id).is_some_and(|entry|entry.profile_id==task.profile_id&&entry.track_id==task.track.id){owners.push((id.clone(),task.owner.clone()));}}}cache.bind_owners(&owners)}
 pub fn authorize(&self,profile:&str){if let Ok(mut profiles)=self.authorized.lock(){profiles.insert(profile.into());}self.wake.notify_one();}
 pub fn forget(&self,profile:&str){if let Ok(mut profiles)=self.authorized.lock(){profiles.remove(profile);}if let (Ok(tasks),Ok(active))=(self.tasks.lock(),self.active.lock()){for task in tasks.iter().filter(|t|t.profile_id==profile){if let Some(cancel)=active.get(&task.id){cancel.store(true,Ordering::Release);}}}}
 fn notification(&self)->CommandResult<()> {
  let tasks=self.tasks.lock().map_err(err)?;let active=self.active.lock().map_err(err)?;
  let (mut received,mut total)=(0u64,0u64);for task in tasks.iter().filter(|task|active.contains_key(&task.id)){received=received.saturating_add(task.received);total=total.saturating_add(task.total);}
  let count=active.len();drop(active);drop(tasks);
  if android::download_notice(json!({"count":count,"received":received,"total":total})).map_err(err)?["allowed"]==false{return Err("系统下载后台额度已用尽，任务保留，请重新打开后继续".into());}Ok(())
 }
 fn allowed(&self,state:&DesktopState,task:&Task)->bool {self.authorized.lock().is_ok_and(|profiles|profiles.contains(&task.profile_id))&&state.listens.owner(&task.profile_id).ok().flatten().as_ref()==Some(&task.owner)}
}
pub(super) fn enqueue(app:tauri::AppHandle,state:&DesktopState,profile:String,track:NativeTrack)->CommandResult<Value>{
 let owner=state.listens.owner(&profile).map_err(err)?.ok_or("请登录后下载")?;
 let id=entry_id(&profile,&owner.user_id,&track.id);let manager=&state.downloads;
 {let mut tasks=manager.tasks.lock().map_err(err)?;
  if !tasks.iter().any(|task|task.id==id){if tasks.len()>=1000{return Err("下载任务已达1000项，请先处理现有任务".into());}tasks.push(Task{id:id.clone(),profile_id:profile,owner,track,status:"queued".into(),cache_id:None,received:0,total:0,error:None});manager.save(&tasks)?;}}
 launch(app,id.clone())?;Ok(json!({"id":id,"status":"queued"}))
}
fn launch(app:tauri::AppHandle,id:String)->CommandResult<()> {
 let state=app.state::<DesktopState>();let manager=&state.downloads;
 let task=manager.tasks.lock().map_err(err)?.iter().find(|task|task.id==id).cloned().ok_or("下载任务不存在")?;
 if !manager.allowed(&state,&task){return Err("请连接原Core并以原账号登录后继续下载".into());}
 let mut active=manager.active.lock().map_err(err)?;if active.contains_key(&id){return Ok(());}
 let cancel=Arc::new(AtomicBool::new(false));active.insert(id.clone(),cancel.clone());drop(active);
 manager.update(&id,|task|{task.status="queued".into();task.error=None;})?;
 if let Err(error)=manager.notification(){manager.active.lock().map_err(err)?.remove(&id);manager.update(&id,|task|{task.status="failed".into();task.error=Some(error.clone());})?;return Err(error);}
 let semaphore=manager.slots.clone();drop(state);
 tauri::async_runtime::spawn(async move {
  let result=async {let _slot=semaphore.acquire_owned().await.map_err(err)?;let state=app.state::<DesktopState>();transfer(&state,&id,&cancel).await}.await;
  let state=app.state::<DesktopState>();let _=state.downloads.update(&id,|task|{
    if cancel.load(Ordering::Acquire){if task.status!="paused" {task.status="interrupted".into();}}
    else {match result {Ok(())=>{task.status="complete".into();task.received=task.total;task.error=None;},Err(error)=>{task.status="failed".into();task.error=Some(error);}}}
  });if let Ok(mut active)=state.downloads.active.lock(){active.remove(&id);};let _=state.downloads.notification();
 });Ok(())
}
async fn transfer(state:&DesktopState,id:&str,cancel:&AtomicBool)->CommandResult<()> {
 let manager=&state.downloads;let task=manager.tasks.lock().map_err(err)?.iter().find(|task|task.id==id).cloned().ok_or("下载任务不存在")?;
 if cancel.load(Ordering::Acquire)||!manager.allowed(state,&task){return Err("下载已暂停".into());}
 let profile=connected_profile(state,&task.profile_id).await?;let session=token(state,&task.profile_id)?;
 let url=media_ticket(&profile,&session,&task.track.id).await.map_err(err)?;let client=profile.http().map_err(err)?;
 let head=client.head(&url).send().await.map_err(err)?.error_for_status().map_err(err)?;
 let total=head.headers().get(reqwest::header::CONTENT_LENGTH).and_then(|v|v.to_str().ok()).and_then(|v|v.parse::<u64>().ok()).filter(|n|*n>0).ok_or("音源未提供完整长度，不能验证下载")?;
 let mut track=task.track.clone();response_metadata(&mut track,head.headers());
 let strong=crate::download_contract::strong_etag(track.source_etag.as_deref());
 // Without a stable strong entity every attempt gets a new cache identity, never joins old bytes.
 if !strong {track.source_etag=Some(format!("unverified-{}",uuid::Uuid::new_v4()));}
 let entry=owned_entry(&task.profile_id,&track,total,true,Some(&task.owner));let cache_id=entry.id.clone();
 let tree=super::storage::target().map_err(err)?;
 let offset={let mut cache=state.cache.lock().map_err(err)?;cache.ensure_download(entry,tree.as_deref()).map_err(err)?;cache.mark_manual(&cache_id).map_err(err)?;let entry=&cache.manifest.entries[&cache_id];if entry.complete{total}else if strong{entry.ranges.first().filter(|r|r.start==0).map_or(0,|r|r.end.min(total))}else{0}};
 manager.update(id,|task|{task.cache_id=Some(cache_id.clone());task.received=offset;task.total=total;task.status="downloading".into();})?;
 let _lease=CacheLease::acquire(state.cache.clone(),cache_id.clone()).map_err(err)?;
 if offset<total {
  let mut request=client.get(&url).header(reqwest::header::RANGE,format!("bytes={offset}-"));
  if strong {request=request.header(reqwest::header::IF_RANGE,track.source_etag.as_ref().unwrap());}
  let response=request.send().await.map_err(err)?.error_for_status().map_err(err)?;
  crate::download_contract::validate_response(response.status().as_u16(),
   response.headers().get(reqwest::header::CONTENT_RANGE).and_then(|v|v.to_str().ok()),
   response.headers().get(reqwest::header::ETAG).and_then(|v|v.to_str().ok()),
   if strong{track.source_etag.as_deref()}else{None},response.content_length(),offset,total).map_err(str::to_owned)?;
  let mut stream=response.bytes_stream();let mut received=offset;let mut last=std::time::Instant::now();
  loop {
   if cancel.load(Ordering::Acquire)||!manager.allowed(state,&task){return Err("下载已暂停".into());}
   let next=tokio::select!{result=stream.next()=>result,_=tokio::time::sleep(std::time::Duration::from_millis(200))=>{continue;}};
   let Some(bytes)=next else{break};let bytes=bytes.map_err(err)?;
   if received+bytes.len() as u64>total{return Err("音源超出完整长度".into());}
   let cache=state.cache.clone();let current=cache_id.clone();let start=received;let count=bytes.len() as u64;
   tokio::task::spawn_blocking(move||cache.lock().map_err(err)?.write_range(&current,start,&bytes).map_err(err)).await.map_err(err)??;received+=count;
   if last.elapsed()>=std::time::Duration::from_millis(500){manager.update(id,|task|task.received=received)?;manager.notification()?;last=std::time::Instant::now();}
  }
  if received!=total{return Err("下载中断，已确认片段保留，可继续".into());}
 }
 if cancel.load(Ordering::Acquire){return Err("下载已暂停".into());}
 manager.update(id,|task|{task.status="verifying".into();task.received=total;})?;
 let cache=state.cache.clone();tokio::task::spawn_blocking(move||{
  cache.lock().map_err(err)?.finish_download(&cache_id).map_err(err)?;
  upgrade::verify(&cache,&cache_id).map_err(err)
 }).await.map_err(err)??;Ok(())
}
#[tauri::command]
pub(super) fn native_download_tasks(address:String,state:State<'_,DesktopState>)->CommandResult<Value>{
 let selected=state.profiles.lock().map_err(err)?.list().into_iter().find(|p|p.address.trim_end_matches('/')==address.trim_end_matches('/')).map(|p|p.id);
 let tasks=state.downloads.tasks.lock().map_err(err)?;let items:Vec<_>=tasks.iter().filter(|task|selected.as_ref()==Some(&task.profile_id)&&state.downloads.allowed(&state,task)).cloned().collect();Ok(json!({"items":items}))
}
#[tauri::command]
pub(super) fn native_download_action(id:String,action:String,app:tauri::AppHandle,state:State<'_,DesktopState>)->CommandResult<()> {
 let task=state.downloads.tasks.lock().map_err(err)?.iter().find(|task|task.id==id).cloned().ok_or("下载任务不存在")?;
 if !state.downloads.allowed(&state,&task){return Err("请以原账号登录后操作下载".into());}
 match action.as_str(){"resume"=>launch(app,id),"pause"=>{state.downloads.update(&id,|task|task.status="paused".into())?;if let Some(cancel)=state.downloads.active.lock().map_err(err)?.get(&id){cancel.store(true,Ordering::Release);}Ok(())},_=>Err("无效下载操作".into())}
}
pub(super) fn schedule(app:tauri::AppHandle){tauri::async_runtime::spawn(async move {loop{app.state::<DesktopState>().downloads.wake.notified().await;let state=app.state::<DesktopState>();let pending=state.downloads.tasks.lock().map(|tasks|tasks.iter().filter(|task|task.status=="interrupted"&&state.downloads.allowed(&state,task)).map(|task|task.id.clone()).collect::<Vec<_>>()).unwrap_or_default();drop(state);for id in pending{let _=launch(app.clone(),id);}}});}
