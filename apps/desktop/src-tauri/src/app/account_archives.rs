//! Bounded, session-bound archive upload. JS never materializes the entire file.
use super::*;
use std::sync::OnceLock;
use tokio::sync::{mpsc,oneshot};
struct Upload {profile:String,session:String,sender:Option<mpsc::Sender<Vec<u8>>>,result:Option<oneshot::Receiver<CommandResult<Value>>>,abort:tokio::task::AbortHandle,bytes:u64,total:u64}
fn uploads()->&'static Mutex<HashMap<String,Upload>>{static MAP:OnceLock<Mutex<HashMap<String,Upload>>>=OnceLock::new();MAP.get_or_init(Default::default)}
fn admitted(state:&DesktopState,u:&Upload)->CommandResult<()>{if token(state,&u.profile)?==u.session{Ok(())}else{Err("账号已变化，请重新选择档案".into())}}
pub(super) fn discard_profile(profile:&str)->CommandResult<()> {
 let mut map=uploads().lock().map_err(err)?;map.retain(|_,u|{if u.profile==profile{u.abort.abort();false}else{true}});Ok(())
}
#[tauri::command]
pub(super) async fn native_account_upload_begin(profile_id:String,job_id:String,total:u64,state:State<'_,DesktopState>)->CommandResult<String>{
 if uuid::Uuid::parse_str(&job_id).is_err()||total==0||total>2*1024*1024*1024{return Err("档案或导入任务无效，最大 2 GiB".into());}
 let profile=connected_profile(&state,&profile_id).await?;let session=token(&state,&profile_id)?;
 let(tx,rx)=mpsc::channel::<Vec<u8>>(2);let(done,result)=oneshot::channel();
 let session_for_request=session.clone();let http=profile.http().map_err(err)?;let url=profile.endpoint(&format!("api/v1/auth/account-imports/{job_id}/file")).map_err(err)?;
 let mut map=uploads().lock().map_err(err)?;if map.len()>=2{return Err("已有两个档案正在上传".into());}
 let task=tokio::spawn(async move{
  let stream=futures_util::stream::unfold(rx,|mut rx|async move{rx.recv().await.map(|bytes|(Ok::<_,std::io::Error>(bytes),rx))});
  let response=http.put(url).bearer_auth(session_for_request).header("Content-Type","application/x-ndjson").header("Content-Length",total).body(reqwest::Body::wrap_stream(stream)).send().await;
  let value=async{let response=response.map_err(err)?;let status=response.status();let value:Value=response.json().await.map_err(err)?;if !status.is_success(){return Err(value["error"]["message"].as_str().unwrap_or("档案上传失败").into());}Ok(value)}.await;
  let _=done.send(value);
 });
 let id=uuid::Uuid::new_v4().to_string();map.insert(id.clone(),Upload{profile:profile_id,session,sender:Some(tx),result:Some(result),abort:task.abort_handle(),bytes:0,total});Ok(id)
}
#[tauri::command]
pub(super) async fn native_account_upload_chunk(id:String,data:String,state:State<'_,DesktopState>)->CommandResult<()>{
 if data.len()>350_000{return Err("上传分片过大".into());}let bytes=base64::engine::general_purpose::STANDARD.decode(data).map_err(err)?;
 let sender={let mut map=uploads().lock().map_err(err)?;let u=map.get_mut(&id).ok_or("上传已结束")?;admitted(&state,u)?;u.bytes=u.bytes.checked_add(bytes.len()as u64).filter(|n|*n<=u.total).ok_or("档案长度不一致")?;u.sender.clone().ok_or("上传已结束")?};
 sender.send(bytes).await.map_err(|_|"上传连接已关闭".into())
}
#[tauri::command]
pub(super) async fn native_account_upload_finish(id:String,state:State<'_,DesktopState>)->CommandResult<Value>{
 let result={let mut map=uploads().lock().map_err(err)?;let u=map.get_mut(&id).ok_or("上传已结束")?;admitted(&state,u)?;if u.bytes!=u.total{u.abort.abort();return Err("档案上传未完成".into());}u.sender.take();u.result.take().ok_or("上传已在收尾")?};
 let value=result.await.map_err(err);uploads().lock().map_err(err)?.remove(&id);value?
}
#[tauri::command]
pub(super) fn native_account_upload_abort(id:String)->CommandResult<()>{if let Some(u)=uploads().lock().map_err(err)?.remove(&id){u.abort.abort();}Ok(())}
