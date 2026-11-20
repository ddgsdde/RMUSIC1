//! Persistent document destinations and foreground relocation of completed offline downloads.
use super::*;
use crate::cache::DocumentStorage;
use std::{path::{Path,PathBuf},io::{Read,Write}};
#[derive(Default)]
pub(super) struct Documents;
fn call(command:&str,args:Value)->Result<Value>{
    let value=android::storage_call(command,args)?;
    if let Some(error)=value["error"].as_str(){anyhow::bail!("{error}");}Ok(value)
}
impl DocumentStorage for Documents {
    fn create(&self,tree:&str,name:&str)->Result<String>{Ok(call("storage_create",json!({"tree":tree,"name":name}))?["uri"].as_str().context("存储目录未返回文件")?.into())}
    fn open(&self,uri:&str,write:bool)->Result<std::fs::File>{
        use std::os::fd::FromRawFd;
        let fd:i32=call("storage_open",json!({"uri":uri,"write":write}))?["fd"].as_i64().filter(|fd|*fd>=0).context("离线文件没有可用描述符")?.try_into()?;
        Ok(unsafe{std::fs::File::from_raw_fd(fd)})
    }
    fn remove(&self,uri:&str)->Result<()>{call("storage_remove",json!({"uri":uri}))?;Ok(())}
}
#[derive(Serialize,Deserialize)]
struct Pending{id:String,uri:Option<String>,path:Option<PathBuf>,source:CacheEntry}
pub(super) struct Control {pending:PathBuf,wake:Arc<tokio::sync::Notify>,progress:Mutex<Value>}
impl Control {
    pub fn open(root:&Path,wake:Arc<tokio::sync::Notify>)->Self{Self{pending:root.join("offline-storage-move.json"),wake,progress:Mutex::new(json!({"busy":false}))}}
    pub fn wake(&self){self.wake.notify_one();}
    fn progress(&self,app:&tauri::AppHandle,value:Value){*self.progress.lock().unwrap()=value.clone();if android::foreground(){let _=app.emit("native-storage-state",value);}}
    fn journal(&self,pending:&Pending)->Result<()> {
        let next=self.pending.with_extension("next");let mut file=std::fs::File::create(&next)?;file.write_all(&serde_json::to_vec(pending)?)?;file.sync_all()?;std::fs::rename(next,&self.pending)?;Ok(())
    }
    fn recover(&self,state:&DesktopState)->Result<()> {
        if !self.pending.exists(){return Ok(());}
        let pending:Pending=serde_json::from_slice(&std::fs::read(&self.pending)?)?;
        let committed=state.cache.lock().unwrap().manifest.entries.get(&pending.id).is_some_and(|e|e.document_uri==pending.uri&&e.complete);
        if committed{state.cache.lock().unwrap().discard_copy(&pending.source)?;}
        else{if let Some(uri)=pending.uri{Documents.remove(&uri)?;}else if let Some(path)=pending.path{if path.exists(){std::fs::remove_file(path)?;}}}
        std::fs::remove_file(&self.pending)?;Ok(())
    }
}
pub(super) fn target()->Result<Option<String>> {
    let value=call("storage_status",json!({}))?;
    if value["available"]!=true{anyhow::bail!("{}",value["error"].as_str().unwrap_or("离线目录不可访问，请重新选择"));}
    Ok(value["tree"].as_str().map(String::from))
}
fn candidates(state:&DesktopState,tree:&Option<String>)->Vec<CacheEntry>{
    let owners:HashMap<_,_>=state.profiles.lock().unwrap().list().into_iter().filter_map(|profile|state.listens.owner(&profile.id).ok().flatten().map(|owner|(profile.id,owner))).collect();
    state.cache.lock().unwrap().manifest.entries.values().filter(|e|e.manual_download&&e.complete&&e.document_tree!=*tree&&e.owner.as_ref().is_some_and(|owner|owners.get(&e.profile_id)==Some(owner))).cloned().collect()
}
fn move_one(app:&tauri::AppHandle,entry:CacheEntry,tree:Option<String>,moved:u64)->Result<bool>{
    let state=app.state::<DesktopState>();
    let (source,_lease,path)={
        let mut cache=state.cache.lock().unwrap();
        if cache.active_count(&entry.id)!=0{return Ok(false);}
        let source=cache.complete_file(&entry.id)?;let path=cache.private_file(&entry);
        cache.protect(&entry.id);drop(cache);
        (source,StorageLease{cache:state.cache.clone(),id:entry.id.clone()},path)
    };
    let uri=tree.as_ref().map(|tree|Documents.create(tree,&format!("{} [{}].{}",entry.title.as_deref().unwrap_or(&entry.track_id).chars().take(100).collect::<String>(),&entry.id[..entry.id.len().min(12)],entry.format))).transpose()?;
    let pending=Pending{id:entry.id.clone(),source:entry.clone(),uri:uri.clone(),path:if uri.is_none(){Some(path.with_extension("moving"))}else{None}};
    state.storage.journal(&pending)?;
    let result=(||->Result<bool>{
        let mut output=if let Some(uri)=&uri{Documents.open(uri,true)?}else{std::fs::OpenOptions::new().write(true).create_new(true).open(pending.path.as_ref().unwrap())?};
        let mut input=rmusic_audio::file::open(&source)?;let mut bytes=vec![0u8;256*1024];let mut received=0;let mut last=std::time::Instant::now();
        loop{if !android::foreground(){return Ok(false);}let count=input.read(&mut bytes)?;if count==0{break;}output.write_all(&bytes[..count])?;received+=count as u64;
            if last.elapsed()>=std::time::Duration::from_millis(500){state.storage.progress(app,json!({"busy":true,"file":entry.title,"received":received,"total":entry.total_bytes,"moved":moved}));last=std::time::Instant::now();}}
        anyhow::ensure!(received==entry.total_bytes,"文件复制未完成，原离线文件保留");output.sync_all()?;drop(output);
        if target()?!=tree||!cache_owned(&state,&entry){return Ok(false);}
        if uri.is_none(){std::fs::rename(pending.path.as_ref().unwrap(),&path)?;}
        if !state.cache.lock().unwrap().relocate(&entry.id,entry.document_uri.as_deref(),uri.clone(),tree)?{if uri.is_none(){std::fs::remove_file(&path)?;}return Ok(false);}
        state.cache.lock().unwrap().discard_copy(&entry)?;
        Ok(true)
    })();
    if !matches!(result,Ok(true)){
        if let Some(uri)=&uri{let _=Documents.remove(uri);}else if let Some(path)=&pending.path{let _=std::fs::remove_file(path);}
    }
    if result.is_ok(){std::fs::remove_file(&state.storage.pending)?;}result
}
struct StorageLease {cache:Arc<Mutex<CacheStore>>,id:String}
impl Drop for StorageLease {fn drop(&mut self){self.cache.lock().unwrap().release(&self.id);}}
#[tauri::command]
pub(super) async fn native_storage_status(app:tauri::AppHandle)->CommandResult<Value>{
    tauri::async_runtime::spawn_blocking(move||{let state=app.state::<DesktopState>();let mut value=android::storage_call("storage_status",json!({})).map_err(err)?;let tree=value["tree"].as_str().map(String::from);let pending=candidates(&state,&tree).len();
        value["pending"]=json!(pending);value["progress"]=state.storage.progress.lock().unwrap().clone();Ok(value)}).await.map_err(err)?
}
#[tauri::command]
pub(super) async fn native_storage_choose(app:tauri::AppHandle)->CommandResult<Value>{
    let selected=android::choose_storage(&app).await.map_err(err)?;
    if selected["cancelled"]!=true{app.state::<DesktopState>().storage.wake();}Ok(selected)
}
#[tauri::command]
pub(super) async fn native_storage_action(action:String,app:tauri::AppHandle)->CommandResult<()> {
    match action.as_str(){"reset"=>{call("storage_reset",json!({})).map_err(err)?;},"retry"=>{},_=>return Err("未知存储操作".into())};
    app.state::<DesktopState>().storage.wake();Ok(())
}
#[cfg(debug_assertions)]
#[tauri::command]
pub(super) fn native_storage_audit(action:String)->CommandResult<Value>{
    if action!="revoke"{return Err("未知存储验证操作".into());}call("storage_revoke",json!({})).map_err(err)
}
pub(super) fn schedule(app:tauri::AppHandle){
    tauri::async_runtime::spawn(async move{
        let wake=app.state::<DesktopState>().storage.wake.clone();wake.notify_one();
        loop {wake.notified().await;
            if !android::foreground(){continue;}
            let app=app.clone();let task=app.clone();
            let result=tauri::async_runtime::spawn_blocking(move||->Result<()> {
                let state=task.state::<DesktopState>();
                let result=(||->Result<()> {state.storage.recover(&state)?;let mut moved=0;
                    loop {if !android::foreground(){break;}let tree=target()?;let rows=candidates(&state,&tree);let next=rows.iter().find(|e|state.cache.lock().unwrap().active_count(&e.id)==0).cloned();
                        let Some(entry)=next else{break};
                        state.storage.progress(&task,json!({"busy":true,"file":entry.title,"received":0,"total":entry.total_bytes,"moved":moved}));
                        if move_one(&task,entry,tree,moved)?{moved+=1;}else{break;}}
                    state.storage.progress(&task,json!({"busy":false,"moved":moved}));Ok(())})();
                result
            }).await;
            if let Err(error)=result.map_err(err).and_then(|r|r.map_err(err)){app.state::<DesktopState>().storage.progress(&app,json!({"busy":false,"error":error}));}
        }
    });
}
