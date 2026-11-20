//! Exact cache lineage for projections; never merges editions by title.
use rusqlite::{Connection, OptionalExtension};
pub const ORIGINAL:&str="CASE WHEN t.source='local' AND t.recording_id<>'' AND EXISTS(SELECT 1 FROM tracks origin WHERE origin.id=json_extract(t.provider_data,'$.cached_from.track_id') AND origin.source=json_extract(t.provider_data,'$.cached_from.source') AND origin.recording_id=t.recording_id) THEN json_extract(t.provider_data,'$.cached_from.track_id') ELSE t.id END";
pub fn canonical(c: &Connection, id: &str) -> anyhow::Result<String> {
    Ok(c.query_row(
        &format!("SELECT {ORIGINAL} FROM tracks t WHERE t.id=?1"),
        [id],
        |r| r.get(0),
    )
    .optional()?
    .unwrap_or_else(|| id.into()))
}
pub fn representative(c: &Connection, id: &str) -> anyhow::Result<String> {
    let id = canonical(c, id)?;
    Ok(c.query_row("SELECT l.id FROM tracks origin JOIN tracks l ON l.recording_id=origin.recording_id AND l.source='local' AND l.available=1 AND json_extract(l.provider_data,'$.cached_from.track_id')=origin.id AND json_extract(l.provider_data,'$.cached_from.source')=origin.source WHERE origin.id=?1 AND origin.recording_id<>'' ORDER BY (lower(l.format) IN ('flac','wav','aiff','alac','dsf','dff')) DESC,l.bit_depth DESC,l.sample_rate DESC,l.id LIMIT 1",[&id],|r|r.get(0)).optional()?.unwrap_or(id))
}
pub fn unique(c: &Connection, ids: Vec<String>, preferred: bool) -> anyhow::Result<Vec<String>> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for id in ids {
        let key = canonical(c, &id)?;
        if seen.insert(key.clone()) {
            out.push(if preferred {
                representative(c, &key)?
            } else {
                key
            });
        }
    }
    Ok(out)
}
pub fn favorites(c: &Connection, item: &mut serde_json::Value) -> anyhow::Result<()> {
    let ids = serde_json::from_value(item["track_ids"].clone())?;
    let mut ids = unique(c, ids, false)?;
    ids.reverse();
    let mut aliases = ids.clone();
    for id in &ids {
        let mut query=c.prepare("SELECT l.id FROM tracks origin JOIN tracks l ON l.recording_id=origin.recording_id AND l.source='local' AND json_extract(l.provider_data,'$.cached_from.track_id')=origin.id AND json_extract(l.provider_data,'$.cached_from.source')=origin.source WHERE origin.id=?1 AND origin.recording_id<>''")?;
        aliases.extend(
            query
                .query_map([id], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?,
        );
    }
    item["track_ids"] = serde_json::json!(ids);
    item["alias_track_ids"] = serde_json::json!(aliases);
    Ok(())
}
