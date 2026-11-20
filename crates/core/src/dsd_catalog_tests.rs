use super::*;

async fn scan_catalog(f:&Fixture) {
    let(status,job)=f.call("POST","/api/v1/library/scan",Some(json!({}))).await;assert_eq!(status,StatusCode::OK);
    let done=tokio::time::timeout(std::time::Duration::from_secs(5),async{loop {
        let(_,jobs)=f.call("GET","/api/v1/jobs",None).await;
        if let Some(current)=jobs["items"].as_array().unwrap().iter().find(|j|j["id"]==job["id"]) {if matches!(current["status"].as_str(),Some("completed"|"failed"|"cancelled")){break current.clone();}}
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }}).await.unwrap();assert_eq!(done["status"],"completed");assert!(done["error"].is_null(),"{done}");
}

#[tokio::test]
async fn dsd_scan_repairs_unchanged_unknown_specs_without_replacing_identity_or_history() {
    let f=Fixture::new().await;f.call("PATCH","/api/v1/settings",Some(json!({"metadata_enabled":false}))).await;
    let music=f.s.config.data.join("DSD");std::fs::create_dir(&music).unwrap();
    let mut bytes=b"DSD ".to_vec();bytes.extend(28_u64.to_le_bytes());bytes.extend(8284_u64.to_le_bytes());bytes.extend(0_u64.to_le_bytes());bytes.extend(b"fmt ");bytes.extend(52_u64.to_le_bytes());
    for value in [1_u32,0,2,2,11289600,1]{bytes.extend(value.to_le_bytes());}
    bytes.extend(32767_u64.to_le_bytes());bytes.extend(4096_u32.to_le_bytes());bytes.extend(0_u32.to_le_bytes());bytes.extend(b"data");bytes.extend(8204_u64.to_le_bytes());bytes.extend([0x96;8192]);
    std::fs::write(music.join("DSD256.dsf"),bytes).unwrap();
    assert_eq!(f.call("POST","/api/v1/library/roots",Some(json!({"path":music}))).await.0,StatusCode::OK);scan_catalog(&f).await;
    let(_,tracks)=f.call("GET","/api/v1/library/tracks",None).await;assert_eq!(tracks["total"],1);
    let first=tracks["items"][0].clone();assert_eq!(first["sample_rate"],11289600);assert_eq!(first["bit_depth"],1);assert_eq!(first["channels"],2);assert_eq!(first["duration"].as_f64().unwrap(),32767./11289600.);
    let id=first["id"].as_str().unwrap().to_owned();
    assert_eq!(f.call("POST","/api/v1/playlists",Some(json!({"name":"DSD identity","track_ids":[id]}))).await.0,StatusCode::OK);
    let saved=id.clone();f.s.db.run(move|c|{
        c.execute("UPDATE tracks SET sample_rate=0,bit_depth=0,channels=0,duration=0 WHERE id=?1",[&saved])?;
        c.execute("INSERT INTO track_stats(track_id,play_seconds) VALUES(?1,42)",[&saved])?;Ok(())
    }).await.unwrap();
    scan_catalog(&f).await;
    let(_,tracks)=f.call("GET","/api/v1/library/tracks",None).await;assert_eq!(tracks["total"],1);assert_eq!(tracks["items"][0],first);
    // A previous parser may already know the bitclock but have never read embedded tags.
    let saved=id.clone();f.s.db.run(move|c|{c.execute("UPDATE tracks SET provider_data='{}',title='old parser title' WHERE id=?1",[saved])?;Ok(())}).await.unwrap();
    scan_catalog(&f).await;
    assert_eq!(f.call("GET","/api/v1/library/tracks",None).await.1["items"][0],first);
    let(_,playlists)=f.call("GET","/api/v1/playlists",None).await;assert_eq!(playlists["items"][0]["track_ids"][0],id);
    let seconds:f64=f.s.db.run(move|c|Ok(c.query_row("SELECT play_seconds FROM track_stats WHERE track_id=?1",[id],|r|r.get(0))?)).await.unwrap();assert_eq!(seconds,42.);
}


#[tokio::test]
async fn dff_diin_scan_repairs_unchanged_names_without_replacing_identity_or_history() {
    let f=Fixture::new().await;f.call("PATCH","/api/v1/settings",Some(json!({"metadata_enabled":false}))).await;
    let music=f.s.config.data.join("DIIN");std::fs::create_dir(&music).unwrap();
    fn chunk(kind:&[u8;4],data:&[u8])->Vec<u8>{let mut bytes=kind.to_vec();bytes.extend((data.len() as u64).to_be_bytes());bytes.extend(data);if data.len()%2!=0{bytes.push(0);}bytes}
    let mut prop=b"SND ".to_vec();prop.extend(chunk(b"FS  ",&5644800_u32.to_be_bytes()));prop.extend(chunk(b"CHNL",b"\0\x02SLFTSRGT"));prop.extend(chunk(b"CMPR",b"DSD \x03raw"));
    let mut metadata=Vec::new();for (kind,name) in [(b"DITI",b"Project title".as_slice()),(b"DIAR",b"DIIN performer")] {let mut payload=(name.len() as u32).to_be_bytes().to_vec();payload.extend(name);metadata.extend(chunk(kind,&payload));}
    let mut form=b"DSD ".to_vec();form.extend(chunk(b"PROP",&prop));form.extend(chunk(b"DSD ",&[0x69;8192]));form.extend(chunk(b"DIIN",&metadata));
    let mut bytes=b"FRM8".to_vec();bytes.extend((form.len() as u64).to_be_bytes());bytes.extend(form);std::fs::write(music.join("old-name.dff"),&bytes).unwrap();
    assert_eq!(f.call("POST","/api/v1/library/roots",Some(json!({"path":music}))).await.0,StatusCode::OK);scan_catalog(&f).await;
    let first=f.call("GET","/api/v1/library/tracks",None).await.1["items"][0].clone();let id=first["id"].as_str().unwrap().to_owned();
    assert_eq!(first["title"],"Project title");assert_eq!(first["artist"],"DIIN performer");assert_eq!(first["provider_data"]["dsd_diin_v1"],true);
    assert_eq!(f.call("POST","/api/v1/playlists",Some(json!({"name":"DIIN identity","track_ids":[id]}))).await.0,StatusCode::OK);
    let saved=id.clone();f.s.db.run(move|c|{
        c.execute("UPDATE tracks SET title='old parser',artist='',provider_data=json_remove(provider_data,'$.dsd_diin_v1') WHERE id=?1",[&saved])?;
        c.execute("INSERT INTO track_stats(track_id,play_seconds) VALUES(?1,42)",[saved])?;Ok(())
    }).await.unwrap();
    scan_catalog(&f).await;assert_eq!(f.call("GET","/api/v1/library/tracks",None).await.1["items"][0],first);
    scan_catalog(&f).await;assert_eq!(f.call("GET","/api/v1/library/tracks",None).await.1["items"][0],first);
    assert_eq!(f.call("GET","/api/v1/playlists",None).await.1["items"][0]["track_ids"][0],id);
    let seconds:f64=f.s.db.run(move|c|Ok(c.query_row("SELECT play_seconds FROM track_stats WHERE track_id=?1",[id],|r|r.get(0))?)).await.unwrap();assert_eq!(seconds,42.);
    assert_eq!(std::fs::read(music.join("old-name.dff")).unwrap(),bytes);
}
