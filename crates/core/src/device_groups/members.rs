use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edit {name:String,zone_ids:Vec<String>,expected_revision:u64}
pub async fn edit(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Json(mut input):Json<Edit>)->ApiResult<Json<Value>>{
    family(&a,"control")?;
    tokio::spawn(async move{
        input.name=input.name.trim().into();input.zone_ids.sort();
        if input.name.is_empty()||input.name.chars().count()>80||input.name.chars().any(char::is_control)||!(2..=4).contains(&input.zone_ids.len())||input.zone_ids.windows(2).any(|w|w[0]==w[1]){
            return Err(ApiError::bad("请输入名称并选择二至四个不同输出"));
        }
        let _operation=s.device_group_lock.lock().await;
        let mut g=load(&s,&id).await?.filter(|g|!g.deleted).ok_or_else(ApiError::missing)?;
        if g.name==input.name&&g.zone_ids==input.zone_ids{return Ok(Json(public(&g)));}
        if g.revision!=input.expected_revision{return Err(ApiError(StatusCode::CONFLICT,"group_conflict".into(),"组合已改变，请刷新后重试。".into()));}
        let _zone=s.zone_lock.lock().await;
        let mut incoming=Vec::new();let mut endpoints=HashSet::new();
        for id in &input.zone_ids{
            member_guard(&s,id,Some(&g.id)).await?;
            crate::accounts::scope::check(&s,&a,"zone",id).await?;
            let z=s.zones.read().await.get(id).cloned().ok_or_else(ApiError::missing)?;
            crate::accounts::scope::check(&s,&a,"endpoint",&z.endpoint_id).await?;
            let e=s.endpoints.read().await.get(&z.endpoint_id).cloned().unwrap_or(Value::Null);
            if !eligible(&e)||e["online"]!=true||!endpoints.insert(z.endpoint_id.clone())||!s.bridges.read().await.contains_key(&z.endpoint_id){
                return Err(ApiError::bad("请选择在线并支持同步的不同原生输出"));
            }
            if !g.zone_ids.contains(id){
                if crate::sync_start::active_zone(&s,id).await{return Err(ApiError::bad("新成员仍在其他同步任务中"));}
                incoming.push(z);
            }
        }
        let current=s.zones.read().await.get(&g.zone_id).cloned().ok_or_else(ApiError::missing)?;
        let playing=current.status=="playing";
        drop(_zone);
        transport::stop(&s,&a,&mut g).await?;
        let _zone=s.zone_lock.lock().await;
        // A normal zone can change while the old group drains. Preserve its latest
        // queue when taking ownership, rather than restoring the earlier preview later.
        for z in &mut incoming {
            member_guard(&s,&z.id,Some(&g.id)).await?;
            *z=s.zones.read().await.get(&z.id).cloned().ok_or_else(ApiError::missing)?;
            let endpoint=s.endpoints.read().await.get(&z.endpoint_id).cloned().unwrap_or(Value::Null);
            if !eligible(&endpoint)||endpoint["online"]!=true{return Err(ApiError::bad("新成员已离线，组合保持暂停，请重新选择"));}
        }
        let mut logical=s.zones.read().await.get(&g.zone_id).cloned().ok_or_else(ApiError::missing)?;
        logical.name=input.name.clone();logical.status=if logical.track.is_some(){"paused"}else{"stopped"}.into();logical.playback_id=None;
        let removed:Vec<String>=g.zone_ids.iter().filter(|id|!input.zone_ids.contains(id)).cloned().collect();
        g.name=input.name;g.zone_ids=input.zone_ids;g.revision+=1;g.controller=a.session_hash.clone();
        if !g.zone_ids.contains(&g.source_zone_id){g.source_zone_id=g.zone_ids[0].clone();}
        g.phase="editing".into();g.error=None;
        let saved=g.clone();let next=logical.clone();let added=incoming.clone();
        let restored=s.db.run(move|c|{
            let tx=c.transaction()?;let mut restored=Vec::new();
            for id in removed{
                let raw:String=tx.query_row("SELECT snapshot FROM device_group_members WHERE group_id=?1 AND zone_id=?2",params![saved.id,id],|r|r.get(0))?;
                let mut z:Zone=serde_json::from_str(&raw)?;
                if z.status=="playing"{z.status="paused".into();}z.playback_id=None;z.updated_at=chrono::Utc::now().timestamp_millis();
                let previous:String=tx.query_row("SELECT data FROM zones WHERE id=?1",[&z.id],|r|r.get(0))?;
                AppState::revise_queue(&mut z,Some(&serde_json::from_str::<Zone>(&previous)?));
                tx.execute("UPDATE zones SET data=?2 WHERE id=?1",params![z.id,serde_json::to_string(&z)?])?;
                tx.execute("DELETE FROM device_group_members WHERE group_id=?1 AND zone_id=?2",params![saved.id,id])?;restored.push(z);
            }
            for z in added{tx.execute("INSERT INTO device_group_members(zone_id,endpoint_id,group_id,snapshot) VALUES(?1,?2,?3,?4)",params![z.id,z.endpoint_id,saved.id,serde_json::to_string(&z)?])?;}
            tx.execute("UPDATE device_groups SET job_id=NULL,data=?2 WHERE id=?1",params![saved.id,serde_json::to_string(&saved)?])?;
            tx.execute("UPDATE zones SET data=?2 WHERE id=?1",params![next.id,serde_json::to_string(&next)?])?;
            tx.commit()?;Ok(restored)
        }).await?;
        {let mut zones=s.zones.write().await;for z in restored{zones.insert(z.id.clone(),z);}zones.insert(logical.id.clone(),logical.clone());}
        drop(_zone);
        for z in &incoming{
            if let Err(error)=transport::stop_member(&s,&a,&z.id).await{
                g.phase="ready".into();g.error=Some(error.2.clone());save(&s,&g).await?;publish_endpoint(&s,&g).await?;
                return Ok(Json(public(&g)));
            }
        }
        g.phase="ready".into();save(&s,&g).await?;publish_endpoint(&s,&g).await?;
        if playing&&logical.track.is_some(){
            logical.status="playing".into();s.save_zone(logical.clone()).await?;
            if let Err(error)=transport::start(&s,&a,&mut g,&logical).await{
                g.error=Some(error.2);logical.status="paused".into();s.save_zone(logical).await?;save(&s,&g).await?;
            }
        }
        s.emit("device_groups",json!({}));s.emit("zones_changed",json!({}));
        Ok(Json(public(&g)))
    }).await.map_err(anyhow::Error::from)?
}
