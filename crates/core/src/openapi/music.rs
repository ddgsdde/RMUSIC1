//! Contracts for music workflows, separated from the transport catalogue.
use super::*;

pub(super) fn schemas(s: &mut Value) {
    let properties = &mut s["SmartPlaylistQuery"]["properties"];
    for field in ["storage", "format"] {
        properties[field] = json!({"type":"string","maxLength":500});
    }
    for field in [
        "min_plays",
        "max_plays",
        "min_seconds",
        "max_seconds",
        "min_sample_rate",
        "min_bit_depth",
        "not_played_days",
    ] {
        properties[field] = unsigned();
    }
    properties["sort"] = json!({"type":"string","enum":["title","recent","least_played","most_played","recent_added"]});
    properties["max_results"] = json!({"type":"integer","minimum":1,"maximum":10000});
    for field in ["favorites_only", "exclude_saved"] {
        properties[field] = boolean();
    }
    s["SmartPlaylistQuery"]["description"]=json!("参数化动态筛选。聆听统计为共享Core数据；收藏／已保存条件仅当前用户。最多10000首，max_results为显式选取上限。旧版四个筛选字段保持兼容。");
    s["SmartRulePreview"] = object(
        json!({"rule":reference("SmartPlaylistQuery"),"offset":unsigned()}),
        &["rule"],
    );
    s["VersionPreference"] = object(
        json!({"expected_revision":unsigned(),"track_id":nullable(string())}),
        &["expected_revision", "track_id"],
    );
    s["MusicEditionEdit"] = object(
        json!({"expected_revision":unsigned(),"kind":{"type":"string","enum":["unspecified","original","live","remaster","remix","cover","instrumental","radio_edit","other"]},"label":{"type":"string","maxLength":200},"notes":{"type":"string","maxLength":2000}}),
        &["expected_revision", "kind", "label", "notes"],
    );
    s["LlmConfigurationEdit"] = object(
        json!({"expected_revision":string(),"enabled":boolean(),"base_url":{"type":"string","maxLength":2048},"model":{"type":"string","maxLength":200},"key":{"type":"string","maxLength":8192,"writeOnly":true,"description":"省略保留，空字符串清除；更换地址时省略则不沿用旧密钥"}}),
        &["expected_revision", "enabled", "base_url", "model"],
    );
    s["LlmModelRequest"] = object(
        json!({"base_url":string(),"key":{"type":"string","writeOnly":true}}),
        &[],
    );
    s["LlmRankRequest"] = object(
        json!({"track_ids":{"type":"array","minItems":1,"maxItems":50,"items":{"type":"string","maxLength":256}},"preference":{"type":"string","maxLength":1000}}),
        &["track_ids"],
    );
    for field in [
        "music_versions_v1",
        "smart_playlist_rules_v2",
        "llm_recommendations_v1",
        "plugin_developer_v1",
    ] {
        s["Status"]["properties"]["capabilities"]["properties"][field] = boolean();
    }
}
pub(super) fn paths(paths: &mut Value) {
    for (path, method, scope, body) in [
        ("/library/tracks/{id}/versions", "get", "read", None),
        (
            "/library/tracks/{id}/versions",
            "put",
            "control",
            Some("VersionPreference"),
        ),
        (
            "/library/tracks/{id}/edition",
            "put",
            "library",
            Some("MusicEditionEdit"),
        ),
        (
            "/smart-playlists/preview",
            "post",
            "read",
            Some("SmartRulePreview"),
        ),
        ("/llm/settings", "get", "admin", None),
        (
            "/llm/settings",
            "put",
            "admin",
            Some("LlmConfigurationEdit"),
        ),
        ("/llm/models", "post", "admin", Some("LlmModelRequest")),
        ("/llm/recommend", "post", "read", Some("LlmRankRequest")),
        ("/plugins/development", "get", "admin", None),
        ("/plugins/development/template", "get", "admin", None),
        ("/plugins/{id}/contract-check", "post", "admin", None),
    ] {
        operation(
            paths,
            path,
            method,
            scope,
            body.map(|v| (v, true)),
            "AnyObject",
        );
    }
    for path in [
        "/library/tracks/{id}/versions",
        "/library/tracks/{id}/edition",
        "/llm/settings",
    ] {
        paths[format!("/api/v1{path}")]["put"]["responses"]["409"] =
            error_response("修订冲突，重新读取后再确认");
    }
    paths["/api/v1/library/tracks/{id}/versions"]["get"]["parameters"]
        .as_array_mut()
        .unwrap()
        .push(parameter("offset", "query", false, unsigned()));
    let template = &mut paths["/api/v1/plugins/development/template"]["get"];
    template["parameters"] = json!([
        parameter(
            "kind",
            "query",
            true,
            json!({"type":"string","enum":["extension","source"]})
        ),
        parameter("id", "query", true, json!({"type":"string","maxLength":64}))
    ]);
    template["responses"]["200"] = json!({"description":"开发模板ZIP，包含SDK、manifest与入口文件","content":{"application/zip":{"schema":{"type":"string","format":"binary"}}}});
    for path in ["/llm/models", "/llm/recommend"] {
        let op = &mut paths[format!("/api/v1{path}")]["post"];
        op["responses"]["429"] = error_response("请求并发已满或服务限流");
        op["responses"]["502"] = error_response("服务不可用、响应无效或过大；不回显上游正文");
    }
    paths["/api/v1/llm/recommend"]["post"]["responses"]["409"] = error_response("Core尚未启用LLM");
}
