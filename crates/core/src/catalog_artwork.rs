//! Refresh a known artist/release image using its exact provider anchor, never a name search.
use crate::{error::ApiResult, state::AppState};
use rusqlite::{params, OptionalExtension};
pub async fn refresh(s: &AppState, source: &str) -> ApiResult<Option<String>> {
    let key = source.to_owned();
    let owners=s.db.read(move|c|Ok(c.prepare("SELECT e.kind,e.entity_id FROM artwork_entities e WHERE e.source_id=?1 AND e.kind IN('artist','album') AND NOT EXISTS(SELECT 1 FROM catalog_profiles p WHERE p.kind=e.kind AND p.entity_id=e.entity_id AND json_type(p.manual,'$.image_url') IS NOT NULL) LIMIT 2")?.query_map([key],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?)).await?;
    if owners.len() != 1 {
        return Ok(None);
    };
    let (kind, id) = &owners[0];
    let (k, key) = (kind.clone(), id.clone());
    let seed=s.db.read(move|c|{
 let preferred=c.query_row("SELECT json_extract(automatic,'$._provenance.image_url') FROM catalog_profiles WHERE kind=?1 AND entity_id=?2",params![k,key],|r|r.get::<_,Option<String>>(0)).optional()?.flatten().unwrap_or_default();
 let sql=format!("SELECT t.* FROM tracks t WHERE t.source!='local' AND t.id IN ({}) ORDER BY (t.source=?2) DESC,t.id LIMIT 1",crate::catalog_detail::direct_ids(&k));Ok(c.query_row(&sql,params![key,preferred],crate::models::Track::from_row).optional()?)
 }).await?;
    let Some(seed) = seed else { return Ok(None) };
    let provider = seed.source.clone();
    let artist = if kind == "artist" {
        crate::catalog_detail::provider_artist(&seed, id, &seed.artist)
    } else {
        None
    };
    if kind == "artist" && artist.is_none() {
        return Ok(None);
    }
    let opts = crate::related_catalog::Options {
        kind: Some(kind.clone()),
        page: Some(1),
        artist_mid: artist,
        artist_name: None,
        refresh: true,
        skip_profile: false,
        sort: None,
    };
    let value = match tokio::time::timeout(
        std::time::Duration::from_secs(12),
        crate::related_catalog::browse_provider(s, seed, &provider, opts),
    )
    .await
    {
        Ok(Ok(v)) => v,
        _ => return Ok(None),
    };
    if value["matched"] != true || value["source"] != provider {
        return Ok(None);
    }
    let Some(url) = value["profile"]["image_url"]
        .as_str()
        .filter(|v| v.starts_with("https://"))
    else {
        return Ok(None);
    };
    let (source, kind, id, url) = (source.to_owned(), kind.clone(), id.clone(), url.to_owned());
    let updated = url.clone();
    let changed=s.db.run(move|c|Ok(c.execute("UPDATE artwork_sources SET url=?1 WHERE id=?2 AND EXISTS(SELECT 1 FROM artwork_entities e WHERE e.kind=?3 AND e.entity_id=?4 AND e.source_id=?2) AND NOT EXISTS(SELECT 1 FROM catalog_profiles p WHERE p.kind=?3 AND p.entity_id=?4 AND json_type(p.manual,'$.image_url') IS NOT NULL)",params![url,source,kind,id])?>0)).await?;
    Ok(changed.then_some(updated))
}
