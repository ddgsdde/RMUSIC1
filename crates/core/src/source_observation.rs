//! Sanitized source attributes, separate from platform hints and the live output path.
use crate::models::Track;
use rusqlite::params;
use serde_json::{json, Value};

pub fn evidence(value: &Value, basis: &str) -> Option<Value> {
    if value["verified"] != true { return None }
    let format = value["format"].as_str()?.to_ascii_lowercase();
    if !["flac","alac","wav","aiff","aif","ogg","opus","mp3","aac","m4a","dsf","dff"].contains(&format.as_str()) { return None }
    let mut result = json!({"format":format,"verified":true,"basis":basis});
    for (key, maximum) in [("sample_rate",49_152_000),("bit_depth",64),("channels",64),("bitrate",100_000_000)] {
        if let Some(n) = value[key].as_u64().filter(|n| *n>0 && *n<=maximum) { result[key]=json!(n); }
    }
    if let Some(quality) = value["quality"].as_str().filter(|v|v.len()<=64) { result["quality"]=json!(quality); }
    Some(result)
}

pub fn store(c: &rusqlite::Connection, id: &str, source: &str, value: &Value) -> anyhow::Result<()> {
    c.execute("UPDATE tracks SET provider_data=json_set(provider_data,'$.source_audio',json(?1)) WHERE id=?2 AND source=?3 AND COALESCE(json_extract(provider_data,'$.source_audio'),'')<>?1", params![value.to_string(),id,source])?;
    Ok(())
}

pub fn file(c: &rusqlite::Connection, t: &Track) -> anyhow::Result<()> {
    if t.source!="local" || t.sample_rate==0 { return Ok(()) }
    let (Some(source),Some(id))=(t.provider_data["cached_from"]["source"].as_str(),t.provider_data["cached_from"]["track_id"].as_str()) else {return Ok(())};
    let value=json!({"format":t.format,"sample_rate":t.sample_rate,"bit_depth":t.bit_depth,"channels":t.channels,"bitrate":t.bitrate,"verified":true,"quality":t.provider_data["cache_quality"]["quality"]});
    if let Some(value)=evidence(&value,"cached_file_record") {store(c,id,source,&value)?;}
    Ok(())
}

pub fn migrate(c: &rusqlite::Connection) -> anyhow::Result<()> {
    if c.query_row("SELECT EXISTS(SELECT 1 FROM settings WHERE key='source_audio_v1')",[],|r|r.get::<_,bool>(0))? {return Ok(())}
    let tx=c.unchecked_transaction()?;
    let mut after=0i64;
    loop {
        let rows=tx.prepare("SELECT rowid,t.* FROM tracks t WHERE rowid>?1 AND source='local' AND sample_rate>0 AND json_type(provider_data,'$.cached_from.track_id')='text' ORDER BY rowid LIMIT 128")?.query_map([after],|r|Ok((r.get::<_,i64>(0)?,Track::from_row(r)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        if rows.is_empty(){break;}
        for (row,t) in rows {file(&tx,&t)?;after=row;}
    }
    tx.execute("INSERT INTO settings(key,value)VALUES('source_audio_v1','true')",[])?;
    tx.commit()?;
    Ok(())
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn source_evidence_excludes_secrets_and_never_guesses_unknown_pcm_parameters() {
        assert!(evidence(&json!({"format":"flac","sample_rate":96000,"verified":false}),"test").is_none());
        let value=evidence(&json!({"format":"ogg","verified":true,"url":"https://secret","headers":{"Authorization":"secret"},"bit_depth":0}),"source_header").unwrap();
        assert_eq!(value,json!({"format":"ogg","verified":true,"basis":"source_header"}));
        let value=evidence(&json!({"format":"flac","sample_rate":44100,"bit_depth":16,"verified":true}),"source_header").unwrap();
        assert_eq!(value["sample_rate"],44100);assert_eq!(value["bit_depth"],16);
    }
}
