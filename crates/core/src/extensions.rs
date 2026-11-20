//! Versioned, explicitly invoked extension RPC and typed plugin configuration.
//! Trusted Python packages are not a sandbox; no host token or arbitrary host command is passed.
use crate::{auth::Auth,error::{ApiError,ApiResult},state::AppState};
use axum::{extract::{Path,State},Json};
use serde_json::{json,Value};
const LIMIT:usize=64*1024;
fn invalid()->ApiError{ApiError::bad("插件扩展合同、配置或数据不符合声明")}
fn name(s:&str)->bool{!s.is_empty()&&s.len()<=64&&s.bytes().all(|c|c.is_ascii_alphanumeric()||b"_-".contains(&c))}
/// Deliberately small closed schema dialect; unsupported keywords fail installation.
fn schema(s:&Value,depth:usize)->bool{
 if depth>8{return false;}let Some(o)=s.as_object()else{return false;};
 if o.keys().any(|key|!["type","properties","required","items","enum","minimum","maximum","maxLength","maxItems","additionalProperties","description","secret"].contains(&key.as_str())){return false;}
 if s.get("secret").is_some_and(|v|!v.is_boolean())||s.get("description").is_some_and(|v|!v.is_string()){return false;}
 if s.get("enum").is_some_and(|v|v.as_array().is_none_or(|a|a.is_empty()||a.len()>64||a.iter().any(|v|!v.is_boolean()&&!v.is_number()&&!v.is_string()))){return false;}
 if ["minimum","maximum"].iter().any(|k|s.get(k).is_some_and(|v|v.as_f64().is_none_or(|n|!n.is_finite()))){return false;}
 if let (Some(a),Some(b))=(s["minimum"].as_f64(),s["maximum"].as_f64()){if a>b{return false;}}
 match s["type"].as_str(){
  Some("object")=>s["additionalProperties"]==false&&s["properties"].as_object().is_some_and(|p|p.len()<=32&&p.iter().all(|(k,v)|name(k)&&schema(v,depth+1))&&s.get("required").is_none_or(|r|r.as_array().is_some_and(|a|a.len()<=p.len()&&a.iter().all(|k|k.as_str().is_some_and(|k|p.contains_key(k)))))),
  Some("array")=>s["maxItems"].as_u64().is_some_and(|n|n<=1000)&&schema(&s["items"],depth+1),
  Some("string")=>s["maxLength"].as_u64().is_some_and(|n|n<=8192),
  Some("integer"|"number"|"boolean")=>true,_=>false,
 }
}
fn conforms(v:&Value,s:&Value,depth:usize)->bool{
 if depth>8{return false;}if let Some(values)=s["enum"].as_array(){if !values.contains(v){return false;}}
 match s["type"].as_str(){
  Some("object")=>v.as_object().is_some_and(|o|o.len()<=32&&o.iter().all(|(k,v)|s["properties"].get(k).is_some_and(|p|conforms(v,p,depth+1)))&&s["required"].as_array().is_none_or(|keys|keys.iter().all(|k|k.as_str().is_some_and(|k|o.contains_key(k))))),
  Some("array")=>v.as_array().is_some_and(|a|a.len()<=s["maxItems"].as_u64().unwrap_or(0) as usize&&a.iter().all(|v|conforms(v,&s["items"],depth+1))),
  Some("string")=>v.as_str().is_some_and(|v|v.len()<=s["maxLength"].as_u64().unwrap_or(0) as usize),
  Some("boolean")=>v.is_boolean(),Some("integer"|"number")=>v.as_f64().is_some_and(|n|n.is_finite()&&(s["type"]!="integer"||v.is_i64()||v.is_u64())&&s["minimum"].as_f64().is_none_or(|min|n>=min)&&s["maximum"].as_f64().is_none_or(|max|n<=max)),_=>false,
 }
}
pub(crate) fn validate_manifest(m:&Value)->ApiResult<()> {
 if m.get("kind").is_some_and(|v|v!="source"&&v!="extension"){return Err(invalid());}
 if let Some(config)=m.get("configuration_v1"){if config["type"]!="object"||!schema(config,0){return Err(invalid());}}
 if m["kind"]!="extension" {if m.get("extension_api").is_some()||m.get("extensions").is_some(){return Err(invalid());}return Ok(());}
 if m["extension_api"]!="1" {return Err(invalid());}let Some(methods)=m["extensions"].as_array().filter(|a|!a.is_empty()&&a.len()<=32)else{return Err(invalid());};let mut seen=std::collections::HashSet::new();
 for method in methods{
  let Some(id)=method["method"].as_str().filter(|s|name(s))else{return Err(invalid());};
  if ["title","description"].iter().any(|key|method.get(key).is_some_and(|v|v.as_str().is_none_or(|s|s.is_empty()||s.len()>500)))||!seen.insert(id)||!["metadata","statistics","automation"].contains(&method["kind"].as_str().unwrap_or(""))||!["read","library","control","admin"].contains(&method["scope"].as_str().unwrap_or(""))||!["none","proposal"].contains(&method["effect"].as_str().unwrap_or(""))||method["effect"]=="proposal"&&method["scope"]=="read"||!schema(&method["input"],0)||!schema(&method["output"],0)||method.get("context").is_some_and(|v|v!="library_summary_v1"){return Err(invalid());}
 }Ok(())
}
pub async fn list(State(s):State<AppState>,a:Auth)->ApiResult<Json<Value>>{
 a.require("read")?;let mut result=vec![];let mut dirs=tokio::fs::read_dir(&s.config.plugin_dir).await.map_err(anyhow::Error::from)?;
 while let Ok(Some(dir))=dirs.next_entry().await {let id=dir.file_name().to_string_lossy().to_string();if let Ok(m)=crate::connectors::manifest(&s,&id).await{if m["kind"]=="extension"&&validate_manifest(&m).is_ok(){result.push(json!({"id":id,"name":m["name"],"version":m["version"],"extension_api":"1","extensions":m["extensions"],"enabled":crate::plugin::check_enabled(&s,&id).await.is_ok()}));}}}
 result.sort_by_key(|v|v["id"].as_str().unwrap_or("").to_owned());Ok(Json(json!({"items":result})))
}
pub async fn invoke(State(s):State<AppState>,a:Auth,Path((id,method)):Path<(String,String)>,Json(input):Json<Value>)->ApiResult<Json<Value>>{
 if serde_json::to_vec(&input).map_err(anyhow::Error::from)?.len()>LIMIT{return Err(invalid());}
 let m=crate::connectors::manifest(&s,&id).await?;validate_manifest(&m)?;crate::plugin::check_enabled(&s,&id).await?;
 let descriptor=m["extensions"].as_array().and_then(|methods|methods.iter().find(|v|v["method"]==method)).ok_or_else(invalid)?;
 a.require(descriptor["scope"].as_str().ok_or_else(invalid)?)?;if !conforms(&input,&descriptor["input"],0){return Err(invalid());}
 let context=if descriptor["context"]=="library_summary_v1"{s.db.read(|c|Ok(json!({"version":1,"track_count":c.query_row("SELECT COUNT(*) FROM tracks WHERE available=1",[],|r|r.get::<_,i64>(0))?,"source_counts":c.prepare("SELECT source,COUNT(*) FROM tracks WHERE available=1 GROUP BY source ORDER BY source LIMIT 64")?.query_map([],|r|Ok(json!({"source":r.get::<_,String>(0)?,"count":r.get::<_,i64>(1)?})))?.collect::<rusqlite::Result<Vec<_>>>()?}))).await?}else{Value::Null};
 let result=tokio::time::timeout(std::time::Duration::from_secs(30),s.plugin.call_declared(&id,json!({"version":1,"method":method,"input":input,"context":context}),m.clone())).await.map_err(|_|ApiError(axum::http::StatusCode::GATEWAY_TIMEOUT,"extension_timeout".into(),"扩展执行超时".into()))??;
 if result["version"]!=1||serde_json::to_vec(&result).map_err(anyhow::Error::from)?.len()>LIMIT||!conforms(&result["output"],&descriptor["output"],0){return Err(ApiError(axum::http::StatusCode::BAD_GATEWAY,"invalid_extension_response".into(),"扩展返回与声明不一致".into()));}
 Ok(Json(json!({"plugin_id":id,"method":method,"version":1,"effect":descriptor["effect"],"host_actions_applied":false,"output":result["output"]})))
}
fn config_path(s:&AppState,id:&str)->std::path::PathBuf{s.config.data.join("providers").join(id).join("configuration.json")}
fn merge(target:&mut Value,patch:&Value){if let (Some(to),Some(from))=(target.as_object_mut(),patch.as_object()){for(k,v)in from{if v.is_object()&&to.get(k).is_some_and(Value::is_object){merge(to.get_mut(k).unwrap(),v);}else{to.insert(k.clone(),v.clone());}}}}
fn redact(value:&Value,schema:&Value)->Value{
 if schema["secret"]==true{return json!({"configured":!value.is_null()});}
 match value {Value::Object(o)=>Value::Object(o.iter().filter_map(|(k,v)|schema["properties"].get(k).map(|s|(k.clone(),redact(v,s)))).collect()),Value::Array(a)=>Value::Array(a.iter().map(|v|redact(v,&schema["items"])).collect()),_=>value.clone()}
}
pub async fn configuration(State(s):State<AppState>,a:Auth,Path(id):Path<String>)->ApiResult<Json<Value>>{
 a.require("admin")?;let m=crate::connectors::manifest(&s,&id).await?;validate_manifest(&m)?;let schema=m.get("configuration_v1").ok_or_else(invalid)?;
 let bytes=tokio::fs::read(config_path(&s,&id)).await.ok().filter(|b|b.len()<=LIMIT);
 let revision=bytes.as_ref().map(|b|crate::auth::hash(&data_encoding::BASE64.encode(b))).unwrap_or("0".into());
 let value=bytes.and_then(|b|serde_json::from_slice::<Value>(&b).ok()).unwrap_or(json!({}));
 Ok(Json(json!({"schema":schema,"values":redact(&value,schema),"revision":revision})))
}
#[derive(serde::Deserialize)]#[serde(deny_unknown_fields)]pub struct Configure {expected_revision:String,values:Value}
pub async fn configure(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Json(input):Json<Configure>)->ApiResult<Json<Value>>{
 a.require("admin")?;
 if serde_json::to_vec(&input.values).map_err(anyhow::Error::from)?.len()>LIMIT||!input.values.is_object(){return Err(invalid());}
 static LOCK:tokio::sync::Mutex<()>=tokio::sync::Mutex::const_new(());let _lock=LOCK.lock().await;
 let runtime=crate::plugin_lifecycle::RUNTIME.get_or_init(||tokio::sync::RwLock::new(()));let _runtime=runtime.write().await;
 let m=crate::connectors::manifest(&s,&id).await?;validate_manifest(&m)?;let schema=m.get("configuration_v1").ok_or_else(invalid)?;
 // Revision belongs to the same atomic file as its contents, including after an interrupted save.
 let path=config_path(&s,&id);let previous=tokio::fs::read(&path).await.ok().filter(|b|b.len()<=LIMIT);
 let revision=previous.as_ref().map(|b|crate::auth::hash(&data_encoding::BASE64.encode(b))).unwrap_or("0".into());
 if input.expected_revision!=revision{return Err(ApiError(axum::http::StatusCode::CONFLICT,"plugin_configuration_conflict".into(),"插件配置已改变，请重新读取".into()));}
 let mut value=previous.and_then(|b|serde_json::from_slice::<Value>(&b).ok()).filter(Value::is_object).unwrap_or(json!({}));
 merge(&mut value,&input.values);
 if !conforms(&value,schema,0){return Err(invalid());}
 let bytes=serde_json::to_vec(&value).map_err(anyhow::Error::from)?;if bytes.len()>LIMIT{return Err(invalid());}
 let next=crate::auth::hash(&data_encoding::BASE64.encode(&bytes));let parent=path.parent().unwrap();tokio::fs::create_dir_all(parent).await.map_err(anyhow::Error::from)?;
 let tmp=parent.join(format!(".configuration-{}",uuid::Uuid::new_v4()));let mut options=tokio::fs::OpenOptions::new();options.create_new(true).write(true);#[cfg(unix)]options.mode(0o600);
 use tokio::io::AsyncWriteExt;let mut file=options.open(&tmp).await.map_err(anyhow::Error::from)?;file.write_all(&bytes).await.map_err(anyhow::Error::from)?;file.sync_all().await.map_err(anyhow::Error::from)?;drop(file);
 s.plugin.stop(&id).await;tokio::fs::rename(tmp,path).await.map_err(anyhow::Error::from)?;
 Ok(Json(json!({"saved":true,"revision":next,"restart":"next_request"})))
}
#[cfg(test)]mod tests{
 use super::*;
 #[test]fn schema_is_closed_bounded_and_rejects_unknown_dialect(){let s=json!({"type":"object","additionalProperties":false,"properties":{"secret":{"type":"string","maxLength":20,"secret":true}},"required":["secret"]});assert!(schema(&s,0));assert!(conforms(&json!({"secret":"abc"}),&s,0));assert!(!conforms(&json!({"secret":"abc","extra":1}),&s,0));assert_eq!(redact(&json!({"secret":"abc"}),&s),json!({"secret":{"configured":true}}));let mut bad=s;bad["$ref"]=json!("http://remote");assert!(!schema(&bad,0));}
}
