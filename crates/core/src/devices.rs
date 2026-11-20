use crate::{auth::Auth,error::{ApiError,ApiResult},state::AppState};
use axum::{extract::{Path,State},Json};
use rusqlite::{params,OptionalExtension};
use serde::Deserialize;
use serde_json::{json,Value};

#[derive(Deserialize)]
pub struct Registration {id:String,name:String,platform:String,outputs:Vec<Value>}
fn valid_id(id:&str)->bool {!id.trim().is_empty()&&id.len()<=200&&!id.chars().any(char::is_control)}
pub(crate) async fn is_removed(s:&AppState,id:&str)->ApiResult<bool> {
    let id=id.to_owned();Ok(s.db.run(move|c|Ok(c.query_row("SELECT EXISTS(SELECT 1 FROM devices d WHERE d.removed=1 AND (d.id=?1 OR EXISTS(SELECT 1 FROM json_each(d.outputs) WHERE json_extract(value,'$.id')=?1)))",[id],|r|r.get::<_,bool>(0))?)).await?)
}
pub async fn remember_client(s:&AppState,endpoint:&mut Value,platform:&str)->ApiResult<()> {
    let id=endpoint["id"].as_str().unwrap_or("").to_owned();let name=endpoint["name"].as_str().unwrap_or("客户端").to_owned();let platform=platform.to_owned();
    let android=platform=="android"&&endpoint["output_config"].is_object();let initial=endpoint["output_config"].clone();
    let outputs=json!([{"id":id,"name":"系统音频输出","kind":if android{"android"}else{"browser"},"configurable":android,"default":true,"native_device_id":initial["device_id"],"initial_config":initial}]);
    let saved=s.db.run(move|c|{let known=c.query_row("SELECT EXISTS(SELECT 1 FROM devices,json_each(devices.outputs) WHERE json_extract(json_each.value,'$.id')=?1)",[&id],|r|r.get::<_,bool>(0))?;if !known{c.execute("INSERT OR IGNORE INTO devices(id,name,platform,outputs) VALUES(?1,?2,?3,?4)",params![id,name,platform,outputs.to_string()])?;}else if android{c.execute("UPDATE devices SET name=?2,platform=?3,outputs=?4 WHERE id=?1",params![id,name,platform,outputs.to_string()])?;}
        if android{c.execute("INSERT OR IGNORE INTO output_settings(id,config) VALUES(?1,?2)",params![id,defaults(&outputs[0]).to_string()])?;return Ok(c.query_row("SELECT config FROM output_settings WHERE id=?1",[id],|r|r.get::<_,String>(0)).optional()?);}Ok(None)}).await?;
    if let Some(saved)=saved{endpoint["output_config"]=serde_json::from_str(&saved).map_err(anyhow::Error::from)?;}Ok(())
}
fn defaults(output:&Value)->Value {let mut value=output["initial_config"].clone();if !value.is_object(){value=json!({"device_id":output["native_device_id"],"exclusive":output["kind"]=="asio","buffer_ms":100,"dsd_output":"disabled","android_mode":null});}value["enabled"]=json!(output["default"]==true);value}
pub async fn register(State(s):State<AppState>,a:Auth,Json(v):Json<Registration>)->ApiResult<Json<Value>> {
    a.require("control")?;
    if !valid_id(&v.id)||v.name.trim().is_empty()||v.name.len()>100||v.platform.len()>40||v.outputs.is_empty()||v.outputs.len()>64{return Err(ApiError::bad("设备声明无效"));}
    let mut ids=std::collections::HashSet::new();
    for output in &v.outputs {
        let id=output["id"].as_str().unwrap_or("");
        if !valid_id(id)||!ids.insert(id)||output["name"].as_str().is_none_or(|name|name.is_empty()||name.len()>300){return Err(ApiError::bad("输出声明无效"));}
    }
    crate::accounts::scope::claim(&s,&a,"device",&v.id).await?;
    for output in &v.outputs{crate::accounts::scope::claim(&s,&a,"endpoint",output["id"].as_str().unwrap()).await?;}
    let outputs=v.outputs.clone();let id=v.id.clone();
    let accepted=s.db.run(move|c|{let tx=c.transaction()?;
        if tx.query_row("SELECT removed FROM devices WHERE id=?1",[&v.id],|r|r.get::<_,bool>(0)).optional()?.unwrap_or(false){return Ok(false);}
        tx.execute("INSERT INTO devices(id,name,platform,outputs) VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET name=excluded.name,platform=excluded.platform,outputs=excluded.outputs",params![v.id,v.name,v.platform,serde_json::to_string(&v.outputs)?])?;
        for output in &v.outputs {tx.execute("INSERT OR IGNORE INTO output_settings(id,config) VALUES(?1,?2)",params![output["id"].as_str().unwrap(),defaults(output).to_string()])?;}
        tx.commit()?;Ok(true)
    }).await?;
    if !accepted{return Ok(Json(json!({"id":id,"removed":true,"outputs":[]})));}
    let result=device(&s,&id,outputs).await?;
    // Announce inventory changes separately; periodic registration does not invalidate music lists.
    s.emit("devices",json!({"id":id}));Ok(Json(result))
}
async fn device(s:&AppState,id:&str,mut outputs:Vec<Value>)->ApiResult<Value> {
    let key=id.to_owned();
    let (name,platform,settings)=s.db.run(move|c|{
        let (name,platform)=c.query_row("SELECT COALESCE(custom_name,name),platform FROM devices WHERE id=?1",[key],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?;
        let mut q=c.prepare("SELECT id,config FROM output_settings")?;
        let settings=q.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<std::collections::HashMap<_,_>>>()?;
        Ok((name,platform,settings))
    }).await?;
    let aliases=s.db.run(|c|Ok(c.prepare("SELECT key,value FROM settings WHERE key LIKE 'output_alias:%'")?.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<std::collections::HashMap<_,_>>>()?)).await?;
    let endpoints=s.endpoints.read().await;
    let zones=s.zones.read().await;
    for output in &mut outputs {let id=output["id"].as_str().unwrap_or("").to_owned();
        if let Some(alias)=aliases.get(&format!("output_alias:{id}")).and_then(|s|serde_json::from_str::<String>(s).ok()){output["name"]=json!(alias);}
        output["config"]=if output["configurable"]==true {settings.get(&id).and_then(|v|serde_json::from_str(v).ok()).unwrap_or_else(||defaults(output))}else{json!({"enabled":true})};
        output["online"]=json!(endpoints.get(&id).is_some_and(|e|e["online"]==true));
        output["device_volume"]=endpoints.get(&id).and_then(|e|e.get("device_volume")).cloned().unwrap_or_else(||output["device_volume"].clone());
        output["endpoint"]=endpoints.get(&id).cloned().unwrap_or(Value::Null);
        output["in_use"]=json!(zones.values().any(|z|z.endpoint_id==id&&z.status!="stopped"&&z.track.is_some()));
    }
    Ok(json!({"id":id,"name":name,"platform":platform,"renameable":true,"outputs":outputs,"online":outputs.iter().any(|o|o["online"]==true)}))
}
pub async fn list(State(s):State<AppState>,a:Auth)->ApiResult<Json<Value>> {
    a.require("read")?;
    let owners=crate::accounts::scope::owners(&s,"device").await?;let realm=crate::accounts::scope::realm(&a);
    let inventory=s.db.run(|c|{let mut q=c.prepare("SELECT id,outputs,removed FROM devices ORDER BY COALESCE(custom_name,name),id")?;let rows=q.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,bool>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;Ok(rows)}).await?;
    let mut items=Vec::new();let mut removed=Vec::new();for(id,outputs,deleted)in inventory{if owners.get(&id).map(String::as_str).unwrap_or("family")!=realm{continue;}let value=device(&s,&id,serde_json::from_str(&outputs).unwrap_or_default()).await?;if deleted{removed.push(value);}else{items.push(value);}}
    let known:std::collections::HashSet<String>=items.iter().chain(&removed).flat_map(|d|d["outputs"].as_array().into_iter().flatten().filter_map(|o|o["id"].as_str().map(String::from))).collect();
    let endpoint_owners=crate::accounts::scope::owners(&s,"endpoint").await?;
    for endpoint in s.endpoints.read().await.values().filter(|e|e["id"]!="browser-local"&&!known.contains(e["id"].as_str().unwrap_or(""))) {
        if endpoint_owners.get(endpoint["id"].as_str().unwrap_or("")).map(String::as_str).unwrap_or("family")!=realm{continue;}
        items.push(json!({"id":endpoint["id"],"name":endpoint["name"],"platform":endpoint["capabilities"]["platform"],"online":endpoint["online"],"outputs":[{"id":endpoint["id"],"name":if endpoint["kind"]=="browser" {json!("系统音频输出")}else{endpoint["name"].clone()},"kind":endpoint["kind"],"online":endpoint["online"],"endpoint":endpoint,"configurable":false}]}));
    }
    for portable in crate::portable::devices(&s,&a).await {
        let mut matched = false;
        for host in &mut items {
            if let Some(output) = host["outputs"].as_array_mut().and_then(|rows| rows.iter_mut().find(|o|o["id"] == portable["id"])) {
                output["online"] = json!(true);
                output["endpoint"] = portable["outputs"][0]["endpoint"].clone();
                host["online"] = json!(true);
                matched = true;
                break;
            }
        }
        if !matched { items.push(portable); }
    }
    Ok(Json(json!({"items":items,"removed":removed})))
}
pub async fn remove(State(s):State<AppState>,a:Auth,Path(id):Path<String>)->ApiResult<Json<Value>> {
    a.require("control")?;if !valid_id(&id){return Err(ApiError::bad("设备标识无效"));}
    let _lock=s.zone_lock.lock().await;
    if is_removed(&s,&id).await?{return Ok(Json(json!({"removed":true})));}
    let key=id.clone();let stored=s.db.run(move|c|Ok(c.query_row("SELECT outputs FROM devices WHERE id=?1",[key],|r|r.get::<_,String>(0)).optional()?)).await?;
    let (outputs,name)=if let Some(outputs)=stored {(serde_json::from_str::<Vec<Value>>(&outputs).map_err(anyhow::Error::from)?,id.clone())}else{
        let endpoint=s.endpoints.read().await.get(&id).cloned();let zone=s.zones.read().await.values().find(|z|z.endpoint_id==id).cloned();
        if endpoint.is_none()&&zone.is_none(){return Err(ApiError::missing());}
        let name=endpoint.as_ref().and_then(|e|e["name"].as_str()).map(String::from).or_else(||zone.map(|z|z.name)).unwrap_or_else(||id.clone());
        (vec![json!({"id":id,"name":"系统音频输出","kind":"","configurable":false})],name)
    };
    let output_ids:std::collections::HashSet<_>=outputs.iter().filter_map(|o|o["id"].as_str().map(String::from)).collect();
    for output in &output_ids {crate::device_groups::endpoint_guard(&s,output).await?;}
    let zones:Vec<_>=s.zones.read().await.values().filter(|z|output_ids.contains(&z.endpoint_id)).cloned().collect();
    let mut disconnects=Vec::new();
    for output in &output_ids {if let Some(tx)=s.bridges.read().await.get(output).cloned(){disconnects.push((output.clone(),tx.try_reserve_owned().map_err(|_|ApiError::bad("设备正在处理其他控制，请稍后重试删除"))?));}}
    for zone in &zones {crate::sync_start::cancel_zone(&s,&zone.id).await?;crate::zones::dispatch(&s,zone,"stop").await?;crate::autoplay::cancel(&s,&zone.id).await;crate::stats::finish(&s,zone,"device_removed").await?;}
    let archived=serde_json::to_string(&zones).map_err(anyhow::Error::from)?;let key=id.clone();let rows=zones.clone();
    s.db.run(move|c|{let tx=c.transaction()?;
        tx.execute("INSERT OR IGNORE INTO devices(id,name,platform,outputs) VALUES(?1,?2,'',?3)",params![key,name,serde_json::to_string(&outputs)?])?;
        tx.execute("UPDATE devices SET removed=1,removed_zones=?2 WHERE id=?1",params![key,archived])?;
        for zone in rows {tx.execute("DELETE FROM zones WHERE id=?1",[zone.id])?;}
        tx.commit()?;Ok(())
    }).await?;
    for zone in zones {s.zones.write().await.remove(&zone.id);crate::sequence::invalidate(&s,&zone.id).await?;}
    for (output,permit) in disconnects {permit.send(json!({"type":"remove_endpoint","endpoint_id":output}));}
    for output in output_ids {s.bridges.write().await.remove(&output);s.clock_filters.write().await.remove(&output);s.bridge_owners.write().await.remove(&output);s.browser_owners.write().await.remove(&output);s.endpoints.write().await.remove(&output);}
    s.emit("devices",json!({}));s.emit("endpoints",json!({}));s.emit("zones_changed",json!({}));Ok(Json(json!({"removed":true})))
}
pub async fn restore(State(s):State<AppState>,a:Auth,Path(id):Path<String>)->ApiResult<Json<Value>> {
    a.require("control")?;let _lock=s.zone_lock.lock().await;
    let zones=s.db.run(move|c|{let tx=c.transaction()?;
        let Some(archive)=tx.query_row("SELECT removed_zones FROM devices WHERE id=?1 AND removed=1",[&id],|r|r.get::<_,String>(0)).optional()? else{return Ok(None);};
        let archived:Vec<crate::models::Zone>=serde_json::from_str(&archive)?;let mut zones=Vec::new();
        for mut zone in archived {if tx.query_row("SELECT EXISTS(SELECT 1 FROM zones WHERE id=?1)",[&zone.id],|r|r.get::<_,bool>(0))?{continue;}zone.status="stopped".into();zone.playback_id=None;zone.updated_at=chrono::Utc::now().timestamp_millis();tx.execute("INSERT INTO zones(id,data) VALUES(?1,?2)",params![zone.id,serde_json::to_string(&zone)?])?;zones.push(zone);}
        tx.execute("UPDATE devices SET removed=0,removed_zones='[]' WHERE id=?1",[id])?;tx.commit()?;Ok(Some(zones))
    }).await?.ok_or_else(ApiError::missing)?;
    for zone in zones {s.zones.write().await.insert(zone.id.clone(),zone);}
    s.emit("devices",json!({}));s.emit("zones_changed",json!({}));Ok(Json(json!({"restored":true})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Purge {confirmation:String}
pub async fn purge(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Json(v):Json<Purge>)->ApiResult<Json<Value>> {
    a.require("admin")?;
    if !valid_id(&id)||v.confirmation!=id{return Err(ApiError::bad("请确认要彻底删除的设备"));}
    let _lock=s.zone_lock.lock().await;
    let result=s.db.run(move|c|{let tx=c.transaction()?;
        let row=tx.query_row("SELECT removed,outputs,removed_zones FROM devices WHERE id=?1",[&id],|r|Ok((r.get::<_,bool>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional()?;
        let Some((removed,outputs,archive))=row else{return Ok(0);};if !removed{return Ok(1);}
        let outputs:Vec<Value>=serde_json::from_str(&outputs)?;let archive:Vec<crate::models::Zone>=serde_json::from_str(&archive)?;
        let zones:std::collections::HashSet<_>=archive.into_iter().map(|z|z.id).collect();
        for output in outputs {if let Some(key)=output["id"].as_str(){
            // A shared output ID may still belong to another registered device.
            let shared=tx.query_row("SELECT EXISTS(SELECT 1 FROM devices d,json_each(d.outputs) o WHERE d.id!=?1 AND json_extract(o.value,'$.id')=?2)",params![id,key],|r|r.get::<_,bool>(0))?;
            if !shared{tx.execute("DELETE FROM output_settings WHERE id=?1",[key])?;}
        }}
        let groups=tx.prepare("SELECT id,data FROM output_groups WHERE deleted=0")?.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        for (key,data) in groups {let mut data:Value=serde_json::from_str(&data)?;if let Some(members)=data["zone_ids"].as_array_mut(){let old=members.len();members.retain(|v|!v.as_str().is_some_and(|id|zones.contains(id)));if members.len()!=old{let deleted=members.len()<2;data["revision"]=json!(data["revision"].as_u64().unwrap_or(0)+1);data["updated_at"]=json!(chrono::Utc::now().timestamp_millis());tx.execute("UPDATE output_groups SET data=?2,deleted=?3 WHERE id=?1",params![key,data.to_string(),deleted])?;}}}
        tx.execute("DELETE FROM devices WHERE id=?1",[id])?;tx.commit()?;Ok(2)
    }).await?;
    match result {0=>return Err(ApiError::missing()),1=>return Err(ApiError::bad("请先将设备移除，再彻底删除")),_=>{}}
    s.emit("devices",json!({}));s.emit("output_groups",json!({}));Ok(Json(json!({"purged":true})))
}
pub async fn rename(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Json(v):Json<Value>)->ApiResult<Json<Value>> {
    a.require("control")?;let name=v["name"].as_str().unwrap_or("").trim();if name.is_empty()||name.len()>100{return Err(ApiError::bad("设备名称应为 1–100 字符"));}
    let name=name.to_owned();let changed=s.db.run(move|c|Ok(c.execute("UPDATE devices SET custom_name=?2 WHERE id=?1",params![id,name])?)).await?;
    if changed==0{return Err(ApiError::missing());}s.emit("devices",json!({}));Ok(Json(json!({"saved":true})))
}
async fn hardware_request(s:&AppState,id:&str,value:Option<f64>)->ApiResult<Value>{
    let tx=s.bridges.read().await.get(id).cloned().ok_or_else(||ApiError::bad("设备离线，无法读写音量"))?;
    let request=uuid::Uuid::new_v4().to_string();let mut receiver=s.events.subscribe();
    tx.try_send(json!({"type":"device_volume","request_id":request,"value":value})).map_err(|_|ApiError::bad("设备控制队列已满"))?;
    tokio::time::timeout(std::time::Duration::from_secs(5),async{loop{match receiver.recv().await{
        Ok(event) if event["type"]=="device_volume"&&event["data"]["endpoint_id"]==id&&event["data"]["request_id"]==request=>{
            let report=&event["data"];if let Some(error)=report["error"].as_str(){return Err(ApiError::bad(error));}return Ok(report["state"].clone());
        },Ok(_)|Err(tokio::sync::broadcast::error::RecvError::Lagged(_))=>{},Err(_)=>return Err(ApiError::bad("设备控制连接中断")),
    }}}).await.map_err(|_|ApiError::bad("设备音量未在时限内确认，请刷新设备状态"))?
}
pub async fn volume(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Json(v):Json<Value>)->ApiResult<Json<Value>>{
    a.require("control")?;let value=v["value"].as_f64().filter(|v|v.is_finite()&&(0. ..=1.).contains(v)).ok_or_else(||ApiError::bad("设备音量无效"))?;
    let key=id.clone();let mode=s.db.run(move|c|Ok(c.query_row("SELECT config FROM output_settings WHERE id=?1",[key],|r|r.get::<_,String>(0)).optional()?)).await?.and_then(|v|serde_json::from_str::<Value>(&v).ok());
    if mode.as_ref().is_none_or(|c|c["volume_control"]!="device"){return Err(ApiError::bad("请先启用设备硬件音量控制"));}
    Ok(Json(hardware_request(&s,&id,Some(value)).await?))
}
pub async fn rename_output(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Json(v):Json<Value>)->ApiResult<Json<Value>>{
    a.require("control")?;let name=v["name"].as_str().unwrap_or("").trim();if name.is_empty()||name.len()>100||name.chars().any(char::is_control){return Err(ApiError::bad("输出名称应为 1–100 字符"));}
    let name=name.to_owned();let key=id.clone();let alias=name.clone();
    let changed=s.db.run(move|c|{if !c.query_row("SELECT EXISTS(SELECT 1 FROM output_settings WHERE id=?1)",[&key],|r|r.get::<_,bool>(0))?{return Ok(false);}c.execute("INSERT INTO settings(key,value)VALUES(?1,?2)ON CONFLICT(key)DO UPDATE SET value=excluded.value",params![format!("output_alias:{key}"),serde_json::to_string(&alias)?])?;Ok(true)}).await?;
    if !changed{return Err(ApiError::missing());}if let Some(endpoint)=s.endpoints.write().await.get_mut(&id){endpoint["name"]=json!(name);}
    s.emit("devices",json!({"id":id}));Ok(Json(json!({"saved":true})))
}
pub async fn configure(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Json(v):Json<Value>)->ApiResult<Json<Value>> {
    a.require("control")?;
    let key=id.clone();let descriptor=s.db.run(move|c|{let mut q=c.prepare("SELECT outputs FROM devices")?;let rows=q.query_map([],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows.into_iter().filter_map(|r|serde_json::from_str::<Vec<Value>>(&r).ok()).flatten().find(|o|o["id"]==key))}).await?.ok_or_else(ApiError::missing)?;
    if descriptor["configurable"]!=true{return Err(ApiError::bad("此输出未声明远程配置能力"));}
    if v.as_object().is_none_or(|v|v.keys().any(|key|!["enabled","device_id","exclusive","buffer_ms","dsd_output","android_mode","max_sample_rate","max_bit_depth","max_dsd_rate","channels","resync_delay_ms","volume_control","volume_max","conversion_location"].contains(&key.as_str()))) {return Err(ApiError::bad("输出配置包含不支持的选项"));}
    let config:rmusic_audio::output::OutputConfig=serde_json::from_value(v.clone()).map_err(|_|ApiError::bad("输出配置格式无效"))?;
    config.limits.validate().map_err(|e|ApiError::bad(e.to_string()))?;
    if v["enabled"].as_bool().is_none()||!(20..=2000).contains(&config.buffer_ms)||(descriptor["kind"]!="android"&&v["device_id"]!=descriptor["native_device_id"])||config.device_id.as_ref().is_some_and(|id|id.len()>500) {return Err(ApiError::bad("输出设备或缓冲参数无效"));}
    if config.dsd_output!=rmusic_audio::output::DsdOutput::Disabled&&descriptor["kind"]!="asio"&&descriptor["kind"]!="wasapi"&&descriptor["kind"]!="alsa" {return Err(ApiError::bad("此输出不支持 DSD"));}
    if config.dsd_output==rmusic_audio::output::DsdOutput::Native&&descriptor["kind"]=="wasapi" {return Err(ApiError::bad("WASAPI 不支持原生 DSD"));}
    if config.dsd_output!=rmusic_audio::output::DsdOutput::Disabled&&!config.exclusive {return Err(ApiError::bad("DSD 需要独占输出"));}
    if config.limits.volume_control==rmusic_audio::output::VolumeControl::Device {
        if descriptor["device_volume"]["supported"]!=true{return Err(ApiError::bad("设备没有报告可写的硬件音量"));}
    }
    if config.limits.volume_control==rmusic_audio::output::VolumeControl::Fixed&&descriptor["device_volume"]["supported"]==true {
        let key=id.clone();let previous=s.db.run(move|c|Ok(c.query_row("SELECT config FROM output_settings WHERE id=?1",[key],|r|r.get::<_,String>(0))?)).await?;
        if serde_json::from_str::<Value>(&previous).ok().is_some_and(|old|old["volume_control"]!="fixed") {
            let actual=hardware_request(&s,&id,Some(1.)).await?;if actual["value"].as_f64().is_none_or(|v|v<0.999){return Err(ApiError::bad("驱动未确认固定最大音量"));}
        }
    }
    if config.limits.conversion_location==rmusic_audio::output::ConversionLocation::Core && !descriptor["capabilities"]["conversion_locations"].as_array().is_some_and(|v|v.iter().any(|v|v=="core")){return Err(ApiError::bad("此输出未声明 Core 转换通道"));}
    if descriptor["kind"]=="coreaudio" {
        if config.exclusive&&descriptor["capabilities"]["exclusive"]!=true{return Err(ApiError::bad("设备未报告独占支持"));}
        // Limits are ceilings, not requested device tuples: do not require the ceiling itself to be a driver rate.
    }
    let output_id=id.clone();
    let mut v=v;v["conversion_location"]=serde_json::to_value(config.limits.conversion_location).unwrap();
    let saved=v.clone();s.db.run(move|c|{let exists=c.query_row("SELECT 1 FROM output_settings WHERE id=?1",[&id],|_|Ok(())).optional()?;if exists.is_none(){anyhow::bail!("output missing");}c.execute("UPDATE output_settings SET config=?2 WHERE id=?1",params![id,saved.to_string()])?;Ok(())}).await?;
    if let Some(tx)=s.bridges.read().await.get(&output_id).cloned(){let _=tx.try_send(json!({"type":"output_config","config":v}));}
    if let Some(endpoint)=s.endpoints.write().await.get_mut(&output_id){endpoint["output_config"]=v.clone();}
    s.emit("devices",json!({}));s.emit("endpoints",json!({}));Ok(Json(json!({"config":v,"apply":"next_playback"})))
}
