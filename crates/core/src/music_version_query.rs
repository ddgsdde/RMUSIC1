//! SQL-side identity resolution and pagination. No candidate-size cap or per-row anchor scan.
use crate::{models::Track, music_versions::Page};
use rusqlite::{named_params, Connection};
use serde_json::{json, Value};

/// Only unambiguous assertions at the strongest evidence level form a recording group.
pub fn resolved_for_key(kind: &str, key: &str) -> String {
    resolved_filtered(
        kind,
        &format!("AND a.track_id IN(SELECT track_id FROM music_assertions WHERE entity_id={key})"),
    )
}
fn resolved_filtered(kind: &str, filter: &str) -> String {
    let source = if kind == "recording" {
        "AND (a.basis!='source_identity' OR e.external_id GLOB t.source||':*' OR (t.source='local' AND json_extract(t.provider_data,'$.cached_from.track_id')=e.external_id))"
    } else {
        "AND a.basis IN ('manual','file_tags')"
    };
    format!("WITH evidence AS (SELECT a.track_id,a.entity_id,CASE a.basis WHEN 'manual' THEN 0 WHEN 'file_tags' THEN 1 ELSE 2 END priority FROM music_assertions a JOIN music_entities e ON e.id=a.entity_id JOIN tracks t ON t.id=a.track_id WHERE e.kind='{kind}' AND a.role='{kind}' AND a.basis IN ('manual','file_tags','source_identity') {source} {filter}), best AS (SELECT track_id,MIN(priority) priority FROM evidence GROUP BY track_id), resolved AS (SELECT e.track_id,MIN(e.entity_id) entity_id FROM evidence e JOIN best b ON b.track_id=e.track_id AND b.priority=e.priority GROUP BY e.track_id HAVING COUNT(DISTINCT e.entity_id)=1)")
}
pub fn page_scoped(c:&Connection,id:&str,key:Option<&str>,p:&Page,other:bool,permission:&str)->anyhow::Result<(Vec<Track>,i64)>{
    let mut cte = resolved_for_key("recording", ":key");
    let membership = if other {
        let work = resolved_filtered("work", "AND a.track_id IN(SELECT b.track_id FROM music_assertions seed JOIN music_entities e ON e.id=seed.entity_id JOIN music_assertions b ON b.entity_id=seed.entity_id WHERE seed.track_id=:id AND e.kind='work' AND seed.role='work')");
        cte.push_str(&format!(
            ", works AS (SELECT * FROM ({work} SELECT * FROM resolved))"
        ));
        "t.id!=:id AND t.id IN(SELECT w.track_id FROM works w JOIN works seed ON seed.entity_id=w.entity_id WHERE seed.track_id=:id) AND (:key IS NULL OR NOT EXISTS(SELECT 1 FROM resolved r WHERE r.track_id=t.id AND r.entity_id=:key))"
    } else {
        "t.id IN(SELECT :id UNION SELECT track_id FROM resolved WHERE entity_id=:key)"
    };
    let filter="(:source='' OR t.source=:source) AND (:available IS NULL OR t.available=:available) AND (:q='' OR instr(lower(t.title||' '||t.artist||' '||t.album),lower(:q))>0)";
    let query = format!("{cte} SELECT {{fields}} FROM tracks t WHERE {membership} AND {filter} AND {permission}");
    let count = c.query_row(
        &query.replace("{fields}", "COUNT(*)"),
        named_params! {":id":id,":key":key,":source":p.source,":available":p.available,":q":p.q},
        |r| r.get::<_, i64>(0),
    )?;
    let order = match p.sort.as_str() {
        "source" => "t.source,t.title COLLATE NOCASE,t.id",
        "added" => "t.created_at DESC,t.id",
        _ => "t.title COLLATE NOCASE,t.artist COLLATE NOCASE,t.id",
    };
    let offset = if other { p.other_offset } else { p.offset };
    let mut stmt = c.prepare(&format!(
        "{} ORDER BY {order} LIMIT :limit OFFSET :offset",
        query.replace("{fields}", "t.*")
    ))?;
    let items=stmt.query_map(named_params!{":id":id,":key":key,":source":p.source,":available":p.available,":q":p.q,":limit":p.limit,":offset":offset},Track::from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok((items, count))
}
pub fn navigation(offset: u32, limit: u32, total: i64) -> Value {
    json!({"offset":offset,"limit":limit,"total":total,"next_offset":if i64::from(offset)+i64::from(limit)<total{Some(offset+limit)}else{None}})
}
