use super::*;
use std::{sync::OnceLock,time::Instant};

#[derive(Default)]
struct Current { key:Option<(String,String,String)>, task:Option<tauri::async_runtime::JoinHandle<()>>, retry:Option<Instant> }
static CURRENT:OnceLock<Mutex<Current>>=OnceLock::new();

/// System artwork follows the native queue, including automatic background transitions.
pub(in crate::app) fn update(app:&tauri::AppHandle,profile:&str,track:&Value){
 let (Some(id),Some(path))=(track["id"].as_str(),track["cover_url"].as_str())else{return;};
 let key=(profile.to_owned(),id.to_owned(),path.to_owned());
 let mut current=CURRENT.get_or_init(Default::default).lock().unwrap();
 if current.key.as_ref()==Some(&key)&&current.retry.is_none_or(|at|Instant::now()<at){return;}
 if let Some(task)=current.task.take(){task.abort();}
 current.key=Some(key.clone());current.retry=None;
 let app=app.clone();
 current.task=Some(tauri::async_runtime::spawn(async move{
  let result=super::super::core_cover(&app.state(), &key.0, &key.2).await;
  match result {
   Ok(image)=>{let _=tauri::async_runtime::spawn_blocking(move||jni_controls::call("artwork",json!({"track_id":key.1,"image":image}))).await;}
   Err(_)=>{let mut current=CURRENT.get().unwrap().lock().unwrap();if current.key.as_ref()==Some(&key){current.retry=Some(Instant::now()+Duration::from_secs(60));}}
  }
 }));
}
