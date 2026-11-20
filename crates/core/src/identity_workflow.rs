//! Dry-run identity changes and audited restore; audio and playlist IDs stay immutable.
use crate::{
    auth::Auth,
    error::{ApiError, ApiResult},
    music_identity::{Assertion, Edit},
    state::AppState,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use rusqlite::{params, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
fn conflict() -> ApiError {
    ApiError(
        StatusCode::CONFLICT,
        "identity_revision_conflict".into(),
        "身份已变化，请重新载入预览".into(),
    )
}
pub fn resolution(graph: &Value) -> Value {
    let mut result = json!({});
    for kind in ["work", "recording", "release"] {
        let mut rows = graph["entities"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|v| v["kind"] == kind)
            .collect::<Vec<_>>();
        let priority = |v: &&Value| match v["basis"].as_str() {
            Some("manual") => 0,
            Some("file_tags") => 1,
            _ => 2,
        };
        rows.sort_by_key(priority);
        let best = rows.first().map(priority);
        let rows = rows
            .into_iter()
            .filter(|v| Some(priority(v)) == best)
            .collect::<Vec<_>>();
        let ids = rows
            .iter()
            .filter_map(|v| v["id"].as_str())
            .collect::<std::collections::HashSet<_>>();
        result[kind] = json!({"resolved":ids.len()==1,"conflict":ids.len()>1,"selected":if ids.len()==1{rows.first().map(|v|(*v).clone())}else{None},"candidates":rows});
    }
    result
}
pub async fn preview(
    State(s): State<AppState>,
    a: Auth,
    Path(id): Path<String>,
    Json(edit): Json<Edit>,
) -> ApiResult<Json<Value>> {
    a.require("library")?;
    edit.validate()?;
    let value=s.db.run(move|c|{let tx=c.transaction()?;if !tx.query_row("SELECT EXISTS(SELECT 1 FROM tracks WHERE id=?1)",[&id],|r|r.get::<_,bool>(0))?{return Ok(Err(ApiError::missing()));}
  let before=crate::music_identity::graph(&tx,&id)?;if before["revision"].as_u64()!=Some(edit.expected_revision){return Ok(Err(conflict()));}
  tx.execute("DELETE FROM music_assertions WHERE track_id=?1 AND basis='manual'",[&id])?;for assertion in &edit.assertions{crate::music_identity::insert(&tx,&id,assertion,"manual")?;}
  let after=crate::music_identity::graph(&tx,&id)?;let resolved=resolution(&after);
  let recording=crate::music_versions::anchor(&tx,&id)?;
  let affected=if let Some(key)=recording {tx.query_row("SELECT COUNT(DISTINCT track_id) FROM music_assertions WHERE entity_id=?1 AND role='recording'",[key],|r|r.get::<_,i64>(0))?}else{1};
  tx.rollback()?;Ok(Ok(json!({"before":before,"after":after,"resolution":resolved,"affected_candidates":affected,"playback_ids_unchanged":true,"preview_only":true})))
 }).await??;
    Ok(Json(value))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Restore {
    expected_revision: u64,
    revision: u64,
}
pub async fn restore(
    State(s): State<AppState>,
    a: Auth,
    Path(id): Path<String>,
    Json(q): Json<Restore>,
) -> ApiResult<Json<Value>> {
    a.require("library")?;
    if q.revision >= i64::MAX as u64 || q.expected_revision >= i64::MAX as u64 {
        return Err(ApiError::bad("身份修订无效"));
    }
    let key = id.clone();
    let revision = q.revision as i64;
    let snapshot =
        s.db.read(move |c| {
            Ok(c.query_row(
                "SELECT after_data FROM music_identity_audit WHERE track_id=?1 AND revision=?2",
                params![key, revision],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
        })
        .await?
        .ok_or_else(ApiError::missing)?;
    let snapshot: Value = serde_json::from_str(&snapshot).map_err(anyhow::Error::from)?;
    let assertions=snapshot["entities"].as_array().into_iter().flatten().filter(|v|v["basis"]=="manual").map(|v|serde_json::from_value::<Assertion>(json!({"kind":v["kind"],"namespace":v["namespace"],"external_id":v["external_id"],"title":v["title"],"role":v["role"],"position":v["position"]}))).collect::<Result<Vec<_>,_>>().map_err(anyhow::Error::from)?;
    crate::music_identity::put(
        State(s),
        a,
        Path(id),
        Json(Edit {
            expected_revision: q.expected_revision,
            assertions,
        }),
    )
    .await
}
pub fn quality(t: &crate::models::Track) -> Value {
    let hint = t
        .provider_data
        .get("quality_hint")
        .cloned()
        .unwrap_or(Value::Null);
    let format = (!t.format.is_empty() && !matches!(t.format.as_str(), "unknown" | "stream"))
        .then_some(t.format.clone());
    let measured = t.source == "local" && t.sample_rate > 0;
    json!({"storage":if t.source!="local"{"platform"}else if t.provider_data["cached_from"].is_object(){"core_cache"}else{"local_file"},"source":t.provider_data["cached_from"]["source"].as_str().unwrap_or(&t.source),"source_track_id":t.provider_data["cached_from"]["track_id"].as_str().unwrap_or(&t.id),"provider_declared":hint,"file_record":{"format":format,"sample_rate":if t.sample_rate>0{Some(t.sample_rate)}else{None},"bit_depth":if t.bit_depth>0{Some(t.bit_depth)}else{None},"channels":if t.channels>0{Some(t.channels)}else{None},"duration":t.duration,"verified":measured},"output_requires_live_report":true,"labels_are_not_quality_certification":true})
}
