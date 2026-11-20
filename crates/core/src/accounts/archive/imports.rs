//! Staged, bounded, resumable NDJSON import. No account is inferred from a name.
use super::*;
use std::sync::{Arc,Mutex,OnceLock,Weak};
fn lock(s:&AppState,id:&str)->Arc<tokio::sync::Mutex<()>>{
 static LOCKS:OnceLock<Mutex<std::collections::HashMap<String,Weak<tokio::sync::Mutex<()>>>>>=OnceLock::new();
 let mut locks=LOCKS.get_or_init(Default::default).lock().unwrap();locks.retain(|_,v|v.strong_count()>0);
 let key=format!("{}:{id}",s.core_id);if let Some(lock)=locks.get(&key).and_then(Weak::upgrade){return lock;}
 let lock=Arc::new(tokio::sync::Mutex::new(()));locks.insert(key,Arc::downgrade(&lock));lock
}
pub async fn list(State(s):State<AppState>,a:Auth)->ApiResult<Json<Value>>{
 a.require("admin")?;require_enabled(&s).await?;if !a.interactive{return Err(ApiError::forbidden());}
 let value=s.db.read(move|c|{let items=c.prepare("SELECT id,state,manifest,bytes,records,applied,target_user,error FROM account_import_jobs WHERE admin_user=?1 ORDER BY created_at DESC,id DESC LIMIT 50")?.query_map([a.id],|r|Ok(json!({"id":r.get::<_,String>(0)?,"state":r.get::<_,String>(1)?,"manifest":r.get::<_,Option<String>>(2)?.and_then(|s|serde_json::from_str::<Value>(&s).ok()),"bytes":r.get::<_,i64>(3)?,"records":r.get::<_,i64>(4)?,"applied":r.get::<_,i64>(5)?,"target_user":r.get::<_,Option<String>>(6)?,"error":r.get::<_,Option<String>>(7)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;Ok(json!({"items":items}))}).await?;Ok(Json(value))
}
pub async fn create(State(s):State<AppState>,a:Auth,Json(v):Json<Credentials>)->ApiResult<Json<Value>>{
 admin::gate(&s,&a).await?;let guard=a.clone();
 let id=s.db.run(move|c|{let tx=c.transaction()?;admin::verify(&tx,&guard,&v.credentials)?;
  let count:i64=tx.query_row("SELECT COUNT(*) FROM account_import_jobs WHERE state NOT IN('completed','invalid')",[],|r|r.get(0))?;if count>=2{return Err(invalid("已有两个未完成的账号导入，请先完成或移除"));}
  let id=uuid::Uuid::new_v4().to_string();tx.execute("INSERT INTO account_import_jobs(id,admin_user,session_hash,state,created_at) VALUES(?1,?2,?3,'created',?4)",params![id,guard.id,guard.session_hash,chrono::Utc::now().timestamp_millis()])?;tx.commit()?;Ok(id)}).await?;
 crate::auth::account::security_success(&s,&a).await;Ok(Json(json!({"id":id,"state":"created","max_bytes":MAX_BYTES})))
}
async fn owned(s:&AppState,a:&Auth,id:&str)->ApiResult<()> {
 a.require("admin")?;require_enabled(s).await?;if !a.interactive{return Err(ApiError::forbidden());}let uid=a.id.clone();let id=id.to_owned();
 if !s.db.read(move|c|Ok(c.query_row("SELECT EXISTS(SELECT 1 FROM account_import_jobs WHERE id=?1 AND admin_user=?2)",params![id,uid],|r|r.get::<_,bool>(0))?)).await?{return Err(ApiError::missing());}Ok(())
}
pub async fn authorize_upload(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Json(v):Json<Credentials>)->ApiResult<Json<Value>>{
 owned(&s,&a,&id).await?;admin::gate(&s,&a).await?;let _lock=lock(&s,&id).try_lock_owned().map_err(|_|ApiError::bad("档案正在上传或导入"))?;let guard=a.clone();let key=id.clone();
 s.db.run(move|c|{let tx=c.transaction()?;admin::verify(&tx,&guard,&v.credentials)?;
  let(state,target):(String,Option<String>)=tx.query_row("SELECT state,target_user FROM account_import_jobs WHERE id=?1 AND admin_user=?2",params![key,guard.id],|r|Ok((r.get(0)?,r.get(1)?)))?;
  if target.is_some()||!matches!(state.as_str(),"created"|"invalid"|"uploading"){return Err(invalid("此任务已完成校验，请继续导入；不能替换已经应用的档案"));}
  tx.execute("UPDATE account_import_jobs SET session_hash=?2,state='created',error=NULL WHERE id=?1",params![key,guard.session_hash])?;tx.commit()?;Ok(())}).await?;
 crate::auth::account::security_success(&s,&a).await;get(State(s),a,Path(id)).await
}
pub async fn upload(State(s):State<AppState>,a:Auth,Path(id):Path<String>,headers:HeaderMap,body:Body)->ApiResult<Json<Value>>{
 owned(&s,&a,&id).await?;let _lock=lock(&s,&id).try_lock_owned().map_err(|_|ApiError::bad("此导入任务正在处理"))?;
 if headers.get("content-length").and_then(|v|v.to_str().ok()).and_then(|v|v.parse::<usize>().ok()).is_some_and(|n|n>MAX_BYTES){return Err(ApiError::bad("账号档案不能超过 2 GiB"));}
 let uid=a.id.clone();let session=a.session_hash.clone();let key=id.clone();
 s.db.run(move|c|{let tx=c.transaction()?;let(state,owner):(String,String)=tx.query_row("SELECT state,session_hash FROM account_import_jobs WHERE id=?1 AND admin_user=?2",params![key,uid],|r|Ok((r.get(0)?,r.get(1)?)))?;
  if owner!=session||!matches!(state.as_str(),"created"|"invalid"|"uploading"){return Err(invalid("请在创建任务的会话中重新选择档案"));}
  tx.execute("DELETE FROM account_import_rows WHERE job_id=?1",[&key])?;tx.execute("UPDATE account_import_jobs SET state='uploading',manifest=NULL,bytes=0,records=0,applied=0,error=NULL WHERE id=?1",[key])?;tx.commit()?;Ok(())}).await?;
 let result=read_upload(&s,&id,&a.session_hash,body).await;
 if result.is_err(){let key=id.clone();s.db.run(move|c|{c.execute("UPDATE account_import_jobs SET state='invalid',error='档案校验失败，请重新上传' WHERE id=?1",[key])?;Ok(())}).await?;}
 result?;get(State(s),a,Path(id)).await
}
async fn read_upload(s:&AppState,id:&str,session:&str,body:Body)->ApiResult<()> {
 let mut stream=body.into_data_stream();let mut line=Vec::new();let mut bytes=0usize;let mut sequence=0usize;let mut manifest=false;let mut preferences=false;let mut batch=Vec::new();let mut batch_bytes=0;
 while let Some(chunk)=stream.next().await {
  let chunk=chunk.map_err(|_|ApiError::bad("账号档案上传中断"))?;bytes=bytes.checked_add(chunk.len()).filter(|n|*n<=MAX_BYTES).ok_or_else(||ApiError::bad("账号档案不能超过 2 GiB"))?;
  for part in chunk.split_inclusive(|b|*b==b'\n'){
   if line.len()+part.len()>MAX_LINE{return Err(ApiError::bad("单条账号记录不能超过 4 MiB"));}line.extend_from_slice(part);
   if part.last()!=Some(&b'\n'){continue;}
   let record=parse_line(&line)?;line.clear();
   if let Some(record)=record{admit_record(s,id,record,&mut manifest,&mut preferences,&mut sequence,&mut batch,&mut batch_bytes).await?;}
  }
  if batch_bytes>=MAX_LINE||batch.len()>=BATCH{flush(s,id,&mut batch).await?;batch_bytes=0;if !s.session_active(session.to_owned()).await{return Err(ApiError::auth());}}
 }
 if !line.is_empty(){if let Some(record)=parse_line(&line)?{admit_record(s,id,record,&mut manifest,&mut preferences,&mut sequence,&mut batch,&mut batch_bytes).await?;}}
 if !manifest{return Err(ApiError::bad("账号档案缺少版本与来源头"));}flush(s,id,&mut batch).await?;
 let key=id.to_owned();s.db.run(move|c|{c.execute("UPDATE account_import_jobs SET state='uploaded',bytes=?2,records=?3 WHERE id=?1",params![key,bytes as i64,sequence as i64])?;Ok(())}).await?;Ok(())
}
fn parse_line(line:&[u8])->ApiResult<Option<Record>> {
 if line.iter().all(|b|b.is_ascii_whitespace()){return Ok(None);}let record:Record=serde_json::from_slice(line).map_err(|_|ApiError::bad("账号档案记录格式无效，未导入账号凭据或未知字段"))?;
 if !record.valid(){return Err(ApiError::bad("账号档案记录超出允许范围"));}Ok(Some(record))
}
#[allow(clippy::too_many_arguments)]
async fn admit_record(s:&AppState,id:&str,mut record:Record,manifest:&mut bool,preferences:&mut bool,sequence:&mut usize,batch:&mut Vec<(usize,Record)>,batch_bytes:&mut usize)->ApiResult<()> {
 if !*manifest {
  if !matches!(&record,Record::Manifest{..}){return Err(ApiError::bad("档案第一条必须是来源头"));}let key=id.to_owned();let value=serde_json::to_string(&record).map_err(anyhow::Error::from)?;
  s.db.run(move|c|{c.execute("UPDATE account_import_jobs SET manifest=?2 WHERE id=?1",params![key,value])?;Ok(())}).await?;*manifest=true;return Ok(());
 }
 if matches!(&record,Record::Manifest{..}){return Err(ApiError::bad("档案中出现重复来源头"));}
 if let Record::Preferences{data}=&mut record{if *preferences{return Err(ApiError::bad("档案中出现重复偏好设置"));}*preferences=true;*data=records::safe_preferences(std::mem::take(data));}
 *sequence+=1;if *sequence>2_000_000{return Err(ApiError::bad("单个档案最多两百万条记录"));}
 let size=serde_json::to_vec(&record).map_err(anyhow::Error::from)?.len();
 if *batch_bytes+size>MAX_LINE||batch.len()>=BATCH{flush(s,id,batch).await?;*batch_bytes=0;}
 *batch_bytes+=size;batch.push((*sequence,record));Ok(())
}
async fn flush(s:&AppState,id:&str,batch:&mut Vec<(usize,Record)>)->ApiResult<()> {
 if batch.is_empty(){return Ok(());}let rows=std::mem::take(batch);let key=id.to_owned();
 s.db.run(move|c|{let tx=c.transaction()?;for(seq,record)in rows{tx.execute("INSERT INTO account_import_rows(job_id,sequence,kind,data) VALUES(?1,?2,?3,?4)",params![key,seq as i64,record.kind(),serde_json::to_string(&record)?])?;}tx.commit()?;Ok(())}).await?;Ok(())
}
pub async fn get(State(s):State<AppState>,a:Auth,Path(id):Path<String>)->ApiResult<Json<Value>>{
 owned(&s,&a,&id).await?;
 let value=s.db.read(move|c|{let mut value=c.query_row("SELECT id,state,manifest,bytes,records,applied,target_user,new_user,error FROM account_import_jobs WHERE id=?1",[&id],|r|{let manifest=r.get::<_,Option<String>>(2)?.and_then(|s|serde_json::from_str::<Value>(&s).ok());Ok(json!({"id":r.get::<_,String>(0)?,"state":r.get::<_,String>(1)?,"manifest":manifest,"bytes":r.get::<_,i64>(3)?,"records":r.get::<_,i64>(4)?,"applied":r.get::<_,i64>(5)?,"target_user":r.get::<_,Option<String>>(6)?,"new_user":r.get::<_,bool>(7)?,"error":r.get::<_,Option<String>>(8)?}))})?;
  let counts=c.prepare("SELECT kind,COUNT(*) FROM account_import_rows WHERE job_id=?1 GROUP BY kind")?.query_map([&id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?)))?.collect::<rusqlite::Result<std::collections::HashMap<_,_>>>()?;value["counts"]=json!(counts);
  let core=value["manifest"]["origin_core"].as_str().unwrap_or("");let user=value["manifest"]["origin_user"].as_str().unwrap_or("");
  let candidates=c.prepare("SELECT u.id,u.username FROM users u WHERE u.role='external' AND ((u.id=?2 AND ?1=(SELECT json_extract(value,'$') FROM settings WHERE key='core_id')) OR EXISTS(SELECT 1 FROM account_origins o WHERE o.user_id=u.id AND o.origin_core=?1 AND o.origin_user=?2)) ORDER BY u.id LIMIT 100")?.query_map(params![core,user],|r|Ok(json!({"id":r.get::<_,String>(0)?,"username":r.get::<_,String>(1)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;value["merge_candidates"]=json!(candidates);Ok(value)}).await?;Ok(Json(value))
}
#[derive(Deserialize)]#[serde(deny_unknown_fields)]pub struct Apply{
 #[serde(flatten)]credentials:Reauthenticate,mode:String,target_user:Option<String>,username:Option<String>,password:Option<String>,#[serde(default)]policy:Policy,
}
pub async fn apply(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Json(v):Json<Apply>)->ApiResult<Json<Value>>{
 owned(&s,&a,&id).await?;admin::gate(&s,&a).await?;let lease=lock(&s,&id).try_lock_owned().map_err(|_|ApiError::bad("此导入任务正在处理"))?;
 if !matches!(v.mode.as_str(),"new"|"merge"){return Err(ApiError::bad("请选择合并或按新用户导入"));}v.policy.validate()?;
 let password=if v.mode=="new"{let name=v.username.as_deref().unwrap_or("");let password=v.password.as_deref().unwrap_or("");crate::auth::validate_credentials(name,password)?;Some(crate::auth::password_hash(password)?)}else{None};
 let auth=a.clone();let key=id.clone();let target=s.db.run(move|c|{let tx=c.transaction()?;admin::verify(&tx,&auth,&v.credentials)?;
  let(state,target):(String,Option<String>)=tx.query_row("SELECT state,target_user FROM account_import_jobs WHERE id=?1 AND admin_user=?2",params![key,auth.id],|r|Ok((r.get(0)?,r.get(1)?)))?;
  if state=="completed"{return target.ok_or_else(||invalid("导入目标已被移除"));}
  if !matches!(state.as_str(),"uploaded"|"interrupted"){return Err(invalid("请先完成档案上传与预览"));}
  let target=if let Some(target)=target{if public_user(&tx,&target)?.is_none(){return Err(invalid("导入目标已被移除"));}target}else if v.mode=="merge"{
   let target=v.target_user.ok_or_else(||invalid("请选择要合并的外部账号"))?;if public_user(&tx,&target)?.is_none(){return Err(invalid("只能合并到现有外部账号"));}target
  }else{
   let name=v.username.unwrap().trim().to_owned();if tx.query_row("SELECT EXISTS(SELECT 1 FROM users WHERE username=?1)",[&name],|r|r.get::<_,bool>(0))?{return Err(invalid("此名称已存在，请明确选择合并或输入新账号名称"));}
   let target=uuid::Uuid::new_v4().to_string();tx.execute("INSERT INTO users(id,username,password,role) VALUES(?1,?2,?3,'external')",params![target,name,password.unwrap()])?;
   tx.execute("INSERT INTO external_accounts(user_id,policy,enabled,created_at) VALUES(?1,?2,0,?3)",params![target,serde_json::to_string(&v.policy)?,chrono::Utc::now().timestamp_millis()])?;target
  };
  tx.execute("UPDATE account_import_jobs SET state='applying',target_user=?2,new_user=CASE WHEN target_user IS NULL THEN ?3 ELSE new_user END,error=NULL WHERE id=?1",params![key,target,v.mode=="new"])?;tx.commit()?;Ok(target)}).await?;
 crate::auth::account::security_success(&s,&a).await;
 let response_auth=a.clone();let worker=s.clone();let job=id.clone();tokio::spawn(async move{let _lease=lease;let result=run(&worker,&job,&a,&target).await;if result.is_err(){let id=job.clone();let _=worker.db.run(move|c|{c.execute("UPDATE account_import_jobs SET state='interrupted',error='导入中断，可重新验证管理员后继续' WHERE id=?1 AND state='applying'",[id])?;Ok(())}).await;}});
 get(State(s),response_auth,Path(id)).await
}
async fn run(s:&AppState,job:&str,a:&Auth,user:&str)->ApiResult<()> {
 loop {
  if !s.session_active(a.session_hash.clone()).await{return Err(ApiError::auth());}
  let key=job.to_owned();let uid=user.to_owned();let core=s.core_id.to_string();
  let done=s.db.run(move|c|{let tx=c.transaction()?;let(applied,manifest,new_user):(i64,String,bool)=tx.query_row("SELECT applied,manifest,new_user FROM account_import_jobs WHERE id=?1 AND target_user=?2",params![key,uid],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
   let manifest:Record=serde_json::from_str(&manifest)?;let Record::Manifest{origin_core,origin_user,..}=manifest else{return Err(invalid("来源头无效"))};
   let rows=tx.prepare("SELECT sequence,data FROM account_import_rows WHERE job_id=?1 AND sequence>?2 ORDER BY sequence LIMIT ?3")?.query_map(params![key,applied,BATCH as i64],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
   if rows.is_empty(){
    tx.execute("INSERT OR IGNORE INTO account_origins(user_id,origin_core,origin_user) VALUES(?1,?2,?3)",params![uid,origin_core,origin_user])?;
    if new_user{tx.execute("UPDATE external_accounts SET enabled=1 WHERE user_id=?1",[&uid])?;}tx.execute("UPDATE account_import_jobs SET state='completed',error=NULL WHERE id=?1",[key])?;tx.commit()?;return Ok(true);
   }
   let mut last=applied;for(seq,raw)in rows{let record:Record=serde_json::from_str(&raw)?;super::records::apply_record(&tx,&uid,&core,&origin_core,&origin_user,record)?;last=seq;}
   tx.execute("UPDATE account_import_jobs SET applied=?2 WHERE id=?1",params![key,last])?;tx.commit()?;Ok(false)}).await?;
  if done{break;}tokio::task::yield_now().await;
 }Ok(())
}
pub async fn delete(State(s):State<AppState>,a:Auth,Path(id):Path<String>)->ApiResult<Json<Value>>{
 owned(&s,&a,&id).await?;let _lock=lock(&s,&id).try_lock_owned().map_err(|_|ApiError::bad("正在导入，暂不能移除任务"))?;
 s.db.run(move|c|{let row:(String,Option<String>)=c.query_row("SELECT state,target_user FROM account_import_jobs WHERE id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?)))?;
  if row.0=="applying"||row.0=="interrupted"&&row.1.is_some(){return Err(invalid("请继续未完成的导入，或先删除其导入目标账号"));}c.execute("DELETE FROM account_import_jobs WHERE id=?1",[id])?;Ok(())}).await?;Ok(Json(json!({"removed":true})))
}
