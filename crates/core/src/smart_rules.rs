//! Parameterized, bounded rules shared by previews, saved lists and exports.
use crate::{
    auth::Auth,
    error::{ApiError, ApiResult},
    library::{self, Page},
    models::Track,
    state::AppState,
};
use axum::{extract::State, Json};
use rusqlite::{params, params_from_iter, types::Value as Sql, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Rule {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub q: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artist_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_plays: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_plays: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_seconds: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_seconds: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_sample_rate: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_bit_depth: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_played_days: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub favorites_only: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude_saved: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<u32>,
}
impl Rule {
    pub fn validate(&self) -> ApiResult<()> {
        if [
            &self.q,
            &self.source,
            &self.album_id,
            &self.artist_id,
            &self.format,
        ]
        .iter()
        .any(|v| v.as_ref().is_some_and(|v| v.len() > 500))
            || self
                .storage
                .as_deref()
                .is_some_and(|v| !["", "local", "cache"].contains(&v))
            || self.sort.as_deref().is_some_and(|v| {
                ![
                    "title",
                    "recent",
                    "least_played",
                    "most_played",
                    "recent_added",
                ]
                .contains(&v)
            })
            || self.max_results.is_some_and(|v| v == 0 || v > 10000)
            || self.min_sample_rate.is_some_and(|v| v > 768000)
            || self.min_bit_depth.is_some_and(|v| v > 64)
            || self.not_played_days.is_some_and(|v| v > 36500)
            || self.max_seconds.is_some_and(|v| v > 86400)
            || self.min_seconds.is_some_and(|v| v > 86400)
            || self
                .min_plays
                .zip(self.max_plays)
                .is_some_and(|(a, b)| a > b)
            || self
                .min_seconds
                .zip(self.max_seconds)
                .is_some_and(|(a, b)| a > b)
        {
            return Err(ApiError::bad("智能规则的范围、排序或结果上限无效"));
        }
        Ok(())
    }
    pub fn ids(&self, c: &Connection, user: &str) -> anyhow::Result<(Vec<String>, bool)> {
        self.validate().map_err(|e| anyhow::anyhow!(e.2))?;
        use rusqlite::OptionalExtension;
        let policy=c.query_row("SELECT e.policy FROM external_accounts e JOIN users u ON u.id=e.user_id WHERE e.user_id=?1 AND e.enabled=1 AND u.role='external'",[user],|r|r.get::<_,String>(0)).optional()?.map(|v|serde_json::from_str::<crate::accounts::Policy>(&v)).transpose()?;
        let stats=if policy.is_some(){format!("(SELECT * FROM account_track_stats WHERE user_id='{}')",user.replace('\'',"''"))}else{"track_stats".into()};
        let page = Page {
            q: self.q.clone(),
            source: self.source.clone(),
            album_id: self.album_id.clone(),
            artist_id: self.artist_id.clone(),
            storage: self.storage.clone(),
            ..Default::default()
        };
        let (mut condition, mut args, searching) =
            library::track_filter(&page).map_err(|e| anyhow::anyhow!(e.2))?;
        if let Some(policy)=policy{policy.validate().map_err(|e|anyhow::anyhow!(e.2))?;condition=condition.replace(library::PREFERRED_COPY,"1");condition.push_str(&format!(" AND {}",policy.sql("t")));}
        for (field, op, value) in [
            (crate::library_query::PLAYS, ">=", self.min_plays),
            (crate::library_query::PLAYS, "<=", self.max_plays),
            ("t.duration", ">=", self.min_seconds),
            ("t.duration", "<=", self.max_seconds),
            ("t.sample_rate", ">=", self.min_sample_rate),
            ("t.bit_depth", ">=", self.min_bit_depth),
        ] {
            if let Some(value) = value {
                condition.push_str(&format!(" AND {field}{op}?"));
                args.push(Sql::Integer(value.into()));
            }
        }
        if let Some(format) = self.format.as_ref().filter(|v| !v.is_empty()) {
            condition.push_str(" AND lower(t.format)=lower(?)");
            args.push(format.clone().into());
        }
        if let Some(days) = self.not_played_days {
            condition.push_str(" AND (p.last_played_at IS NULL OR p.last_played_at<?)");
            args.push((chrono::Utc::now().timestamp() - i64::from(days) * 86400).into());
        }
        if self.favorites_only == Some(true) {
            condition.push_str(&format!(" AND {} IN (SELECT COALESCE({},j.value) FROM playlists f,json_each(f.tracks) j LEFT JOIN tracks saved ON saved.id=j.value WHERE f.id=? AND f.user_id=? AND j.type='text')",crate::cache_identity::ORIGINAL,crate::cache_identity::ORIGINAL.replace("t.","saved.")));
            args.push(format!("favorites-{}", crate::auth::hash(user)).into());
            args.push(user.to_owned().into());
        }
        if self.exclude_saved == Some(true) {
            condition.push_str(&format!(" AND {} NOT IN (SELECT COALESCE({},j.value) FROM playlists f,json_each(f.tracks) j LEFT JOIN tracks saved ON saved.id=j.value WHERE f.user_id=? AND f.smart_query IS NULL AND j.type='text')",crate::cache_identity::ORIGINAL,crate::cache_identity::ORIGINAL.replace("t.","saved.")));
            args.push(user.to_owned().into());
        }
        let from = if searching {
            "track_search CROSS JOIN tracks t ON t.rowid=track_search.rowid"
        } else {
            "tracks t"
        };
        let order = match self.sort.as_deref().unwrap_or("title") {
            "recent" => "p.last_played_at DESC,t.id",
            "least_played" => "COALESCE(p.play_count,0),t.title,t.id",
            "most_played" => "COALESCE(p.play_count,0) DESC,t.title,t.id",
            "recent_added" => "t.created_at DESC,t.id",
            _ if searching => "rank,t.id",
            _ => "t.title,t.id",
        };
        let cap = self.max_results.unwrap_or(10000) as usize;
        args.push(Sql::Integer((cap + 1) as i64));
        let mut ids=c.prepare(&format!("SELECT t.id FROM {from} LEFT JOIN {stats} p ON p.track_id=t.id WHERE {condition} ORDER BY {order} LIMIT ?"))?.query_map(params_from_iter(args),|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let truncated = ids.len() > cap;
        ids.truncate(cap);
        Ok((ids, truncated))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preview {
    pub rule: Rule,
    #[serde(default)]
    pub offset: usize,
}
pub async fn preview(
    State(s): State<AppState>,
    a: Auth,
    Json(q): Json<Preview>,
) -> ApiResult<Json<Value>> {
    a.require("read")?;
    q.rule.validate()?;
    if q.offset > 10000 {
        return Err(ApiError::bad("预览分页无效"));
    }
    let value=s.db.read(move|c|{let(ids,truncated)=q.rule.ids(c,&a.id)?;let mut items=vec![];for id in ids.iter().skip(q.offset).take(30){items.push(c.query_row("SELECT * FROM tracks WHERE id=?1",params![id],Track::from_row)?);}Ok(json!({"items":items,"total":ids.len(),"truncated":truncated,"offset":q.offset,"limit":30,"statistics_scope":if a.role=="external"{"account"}else{"shared_core"}}))}).await?;
    Ok(Json(value))
}
