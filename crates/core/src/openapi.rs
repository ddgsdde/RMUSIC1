//! OpenAPI 3.1 contract for implemented handlers.
mod music;
mod accounts;
mod device_groups;
use axum::Json;
use serde_json::{json, Value};

fn reference(name: &str) -> Value {
    json!({"$ref":format!("#/components/schemas/{name}")})
}
fn string() -> Value {
    json!({"type":"string"})
}
fn boolean() -> Value {
    json!({"type":"boolean"})
}
fn unsigned() -> Value {
    json!({"type":"integer","minimum":0,"format":"int64"})
}
fn number() -> Value {
    json!({"type":"number"})
}
fn nullable(schema: Value) -> Value {
    json!({"anyOf":[schema,{"type":"null"}]})
}
fn array(schema: Value) -> Value {
    json!({"type":"array","items":schema})
}
fn object(properties: Value, required: &[&str]) -> Value {
    let mut result = json!({"type":"object","properties":properties,"additionalProperties":true});
    if !required.is_empty() {
        result["required"] = json!(required);
    }
    result
}
fn items(name: &str, paged: bool) -> Value {
    let mut properties = json!({"items":array(reference(name))});
    if paged {
        properties["total"] = unsigned();
    }
    object(
        properties,
        if paged {
            &["items", "total"]
        } else {
            &["items"]
        },
    )
}
fn schemas() -> Value {
    let mut s = json!({});
    s["VisualizationRequest"]=object(json!({"after":number()}),&[]);
    s["VisualizationFrame"]=object(json!({"playback_id":string(),"position":number(),"bands":{"type":"array","minItems":8,"maxItems":8,"items":{"type":"integer","minimum":0,"maximum":255}},"energy":{"type":"integer","minimum":0,"maximum":255},"peak":{"type":"integer","minimum":0,"maximum":255}}),&["playback_id","position","bands","energy","peak"]);
    s["MusicPlaybackPolicy"]=object(json!({"preferred_track_id":nullable(string()),"apply_to_new_playback":boolean(),"automatic_fallback":boolean(),"same_release":boolean(),"lossless_only":boolean()}),&["preferred_track_id","apply_to_new_playback","automatic_fallback","same_release","lossless_only"]);
    s["MusicPlaybackPolicyEdit"]=object(json!({"expected_revision":unsigned(),"policy":reference("MusicPlaybackPolicy")}),&["expected_revision","policy"]);
    s["CatalogAliasEdit"]=object(json!({"other_id":string(),"linked":boolean(),"expected_revision":unsigned(),"preview_only":boolean()}),&["other_id","linked","expected_revision"]);
    s["VisualizationReport"]=object(json!({"version":unsigned(),"supported":boolean(),"playback_id":nullable(string()),"track_id":nullable(string()),"position":nullable(json!({"type":"number"})),"age_ms":nullable(json!({"type":"number"})),"frames":{"type":"array","maxItems":24,"items":reference("VisualizationFrame")}}),&["supported","frames"]);
    s["IdentityRestoreRequest"]=object(json!({"expected_revision":unsigned(),"revision":unsigned()}),&["expected_revision","revision"]);
    s["IdentityPreview"]=object(json!({"before":reference("AnyObject"),"after":reference("AnyObject"),"resolution":reference("AnyObject"),"affected_candidates":unsigned(),"preview_only":{"const":true},"playback_ids_unchanged":{"const":true}}),&["before","after","resolution","preview_only"]);
    s["ArtworkRefreshRequest"]=object(json!({"ids":{"type":"array","minItems":1,"maxItems":50,"items":{"type":"string","pattern":"^[a-fA-F0-9]{64}$"}}}),&["ids"]);
    s["ArtworkCacheStatus"]=object(json!({"items":array(reference("AnyObject")),"total":unsigned(),"limit":unsigned(),"offset":unsigned(),"used_bytes":unsigned(),"limit_bytes":unsigned(),"file_count":unsigned(),"protected_base_images":{"const":true},"derived_count":unsigned(),"metrics":reference("AnyObject"),"jobs":array(reference("AnyObject"))}),&["items","total","used_bytes","limit_bytes","metrics"]);
    s["ArtworkCacheStatus"]["description"]=json!("管理员图片维护。预算独立于音频；不自动删除永久基图、人工图或音乐。检查只标记且不回源；显式刷新才合并回源并遵守冷却。指标自本次启动累计，response_bytes为实际读取的图片正文，不含HTTP头部；upstream_bytes统计完整取得的上游正文。所有列表分页，不加载全库图片。");
    s["MusicIdentityEdit"]=object(json!({"expected_revision":unsigned(),"assertions":{"type":"array","maxItems":32,"items":object(json!({"kind":{"type":"string","enum":["work","recording","release","artist"]},"namespace":string(),"external_id":string(),"title":string(),"role":string(),"position":unsigned()}),&["kind","namespace","external_id","title","role"])}}),&["expected_revision","assertions"]);
    s["PluginConfigurationEdit"]=object(json!({"expected_revision":string(),"values":reference("AnyObject")}),&["expected_revision","values"]);
    s["AnyObject"]=object(json!({}),&[]);
    s["PlaylistImportPreview"]=json!({"type":"object","additionalProperties":false,"required":["format","content"],"properties":{"format":{"type":"string","enum":["json","m3u","m3u8","pls","csv"]},"content":{"type":"string","maxLength":2097152},"offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":100}}});
    s["PlaylistImportCommit"]=json!({"type":"object","additionalProperties":false,"required":["name","track_ids"],"properties":{"request_id":{"type":"string","format":"uuid","description":"同一账号相同请求与内容安全重试，不重复创建播放列表"},"name":{"type":"string","minLength":1,"maxLength":100},"track_ids":{"type":"array","minItems":1,"maxItems":10000,"items":{"type":"string","maxLength":512}}}});
    s["BackupPolicy"]=object(json!({"directory":string(),"enabled":boolean(),"interval_hours":{"type":"integer","minimum":1,"maximum":720},"keep":{"type":"integer","minimum":1,"maximum":90}}),&["directory","enabled","interval_hours","keep"]);
    s["NewDirectory"]=object(json!({"parent":string(),"name":string()}),&["parent","name"]);
    s["FileAction"]=object(json!({"operation":{"type":"string","enum":["check","recache","relocate","trash"]},"ids":{"type":"array","maxItems":100000,"items":string()},"filter":reference("AnyObject"),"exclude_ids":{"type":"array","maxItems":100000,"items":string()},"expected_count":unsigned(),"directory":string(),"cleanup":reference("AnyObject")}),&["operation"]);
    s["PurgeFile"]=object(json!({"confirmation":{"type":"string","const":"永久删除"}}),&["confirmation"]);

    s["ImpulseAsset"] = object(json!({"id":{"type":"string","pattern":"^[0-9a-f]{64}$"},"name":string(),"bytes":unsigned(),"sample_rate":unsigned(),"channels":unsigned(),"frames":unsigned(),"channel_mask":unsigned(),"created_at":unsigned()}),&["id","name","bytes","sample_rate","channels","frames","channel_mask","created_at"]);
    s["ImpulseAssets"] = object(json!({"items":array(reference("ImpulseAsset")),"limits":object(json!({"max_files":unsigned(),"max_storage_bytes":unsigned(),"max_upload_bytes":unsigned()}),&["max_files","max_storage_bytes","max_upload_bytes"])}),&["items","limits"]);
    s["ConvolutionConfig"]=object(json!({"response_ids":{"type":"array","minItems":1,"maxItems":16,"uniqueItems":true,"items":{"type":"string","pattern":"^[0-9a-f]{64}$"}}}),&["response_ids"]);
    s["ConvolutionConfig"]["additionalProperties"]=json!(false);
    s["ConvolutionConfig"]["description"]=json!("显式选中一组响应，每个采样率一条；最多 32 MiB 原文件和 1048576 系数。保留原始增益，实际处理率精确匹配，缺少时拒绝；仅声明 convolution_v1 的端点可启用。");
    s["ImpulseDownload"]=object(json!({"asset":reference("ImpulseAsset"),"api_path":{"type":"string","format":"uri-reference","description":"相对于所选核心／反代前缀的 API 路径；票据绑定资源、音区、当前响应组和签发会话"}}),&["asset","api_path"]);
    s["ImpulseManifest"]=object(json!({"items":{"type":"array","minItems":1,"maxItems":16,"items":reference("ImpulseDownload")},"expires_at":unsigned()}),&["items","expires_at"]);
    s["OpenApiDocument"] = object(
        json!({"openapi":{"type":"string","const":"3.1.0"},"info":{"type":"object","additionalProperties":true},"paths":{"type":"object","additionalProperties":true},"components":{"type":"object","additionalProperties":true}}),
        &["openapi", "info", "paths", "components"],
    );
    let username = json!({"type":"string","minLength":1,"maxLength":80,"description":"非空；最多 80 UTF-8 字节"});
    let password = json!({"type":"string","minLength":1,"maxLength":1024,"writeOnly":true,"format":"password","description":"创建账号须为 10..1024 UTF-8 字节"});
    let status = json!({"type":"string","enum":["playing","paused","stopped"]});
    let name = json!({"type":"string","minLength":1,"maxLength":100,"description":"最多 100 UTF-8 字节；名称不能全为空白"});
    let ids = json!({"type":"array","maxItems":10000,"items":{"type":"string"}});
    let volume = json!({"type":"number","minimum":0,"maximum":1});
    let seconds = json!({"type":"number","minimum":0,"description":"秒"});
    let timestamp = json!({"type":"integer","format":"int64","description":"Unix 时间戳，秒"});
    let milliseconds = json!({"type":"integer","format":"int64","description":"Unix 时间戳，毫秒"});
    s["Error"] = object(
        json!({"error":object(json!({"code":string(),"message":string()}),&["code","message"])}),
        &["error"],
    );
    s["Status"] = object(
        json!({"initialized":boolean(),"core_id":string(),"version":string(),"name":string(),"capabilities":object(json!({"measured_listens":boolean(),"measured_listens_v2":boolean(),"measured_listens_online_v2":boolean(),"measured_listens_socket_v2":boolean(),"measured_listens_recovery_v2":boolean(),"programme_clock_v2":boolean(),"group_output_v2":boolean(),"group_output_final_v2":boolean(),"group_manual_recovery_v2":boolean(),"group_volume_v1":boolean(),"continuous_recovery":boolean(),"recovery_controls":boolean(),"browser_pcm":boolean(),"browser_aac_pcm":boolean()}),&["measured_listens"])}),
        &["initialized", "core_id", "version", "name"],
    );
    s["SyncPolicy"]=object(json!({"max_known_error_us":{"type":"integer","minimum":100,"maximum":50000},"warmup_ms":{"type":"integer","minimum":1000,"maximum":10000}}),&["max_known_error_us","warmup_ms"]);
    s["Status"]["properties"]["capabilities"]["properties"]["convolution_config_v1"]=boolean();
    s["Status"]["properties"]["capabilities"]["properties"]["smart_playlists"]=boolean();
    s["Status"]["properties"]["capabilities"]["properties"]["output_groups_v1"]=boolean();
    s["Status"]["properties"]["capabilities"]["properties"]["group_queue_jump_v1"]=boolean();
    s["Status"]["properties"]["capabilities"]["properties"]["group_membership_v1"]=boolean();
    s["Status"]["properties"]["capabilities"]["properties"]["group_clock_diagnostics_v1"]=boolean();
    s["Status"]["properties"]["capabilities"]["properties"]["system_suspend_recovery"]=boolean();
    s["Status"]["properties"]["capabilities"]["properties"]["system_suspend_fresh_start"]=boolean();
    s["Status"]["properties"]["capabilities"]["properties"]["guarded_transport_v1"]=boolean();
    s["Status"]["properties"]["capabilities"]["properties"]["frame_clock_max_rate"]=json!({"type":"integer","minimum":8000,"maximum":rmusic_audio::MAX_CLOCK_RATE,"description":"实际节目帧计时上限；原生 DSD 一帧含每声道八比特。PCM 输出格式上限另行协商。"});
    s["User"] = object(
        json!({"id":string(),"username":string(),"role":{"type":"string","enum":["admin","member"]},"totp_enabled":boolean()}),
        &["id", "username", "role"],
    );
    s["DiscoveredService"]=object(json!({"id":string(),"name":string(),"kind":{"type":"string","enum":["core","bridge"]},"addresses":array(string()),"core_id":nullable(string())}),&["id","name","kind","addresses","core_id"]);
    s["Discovery"]=object(json!({"available":boolean(),"items":array(reference("DiscoveredService"))}),&["available","items"]);
    s["AuthResult"] = object(
        json!({"token":{"type":"string","description":"Bearer 会话；仅创建或登录时返回"},"user":reference("User")}),
        &["token", "user"],
    );
    s["SetupRequest"] = object(
        json!({"bootstrap_token":{"type":"string","writeOnly":true},"username":username,"password":password}),
        &["bootstrap_token", "username", "password"],
    );
    s["LoginClient"] = object(
        json!({"id":{"type":"string","pattern":"^[a-zA-Z0-9-]{1,100}$"},"name":{"type":"string","maxLength":100},"type":{"enum":["macos","android","windows","linux","browser"]}}),
        &["id", "name", "type"],
    );
    s["LoginRequest"] = object(
        json!({"username":username,"password":password,"totp":{"type":["string","null"],"writeOnly":true,"description":"启用 2FA 时提供六位 TOTP 或一次性恢复码"},"client":nullable(reference("LoginClient"))}),
        &["username", "password"],
    );
    s["NewUserRequest"] = object(
        json!({"username":username,"password":password}),
        &["username", "password"],
    );
    s["ReauthenticateRequest"] = object(json!({"current_password":{"type":"string","maxLength":1024,"writeOnly":true},"totp":{"type":"string","maxLength":128,"writeOnly":true,"description":"已启用 2FA 时，输入未使用的动态码或恢复码"}}), &["current_password"]);
    s["AccountUpdateRequest"] = s["ReauthenticateRequest"].clone();
    s["AccountUpdateRequest"]["properties"]["username"] = json!({"type":"string","minLength":1,"maxLength":80,"description":"仅管理员可修改自己的登录账号"});
    s["AccountUpdateRequest"]["properties"]["new_password"] = password.clone();
    s["AccountUpdated"] = object(json!({"user":reference("User"),"revoked_sessions":unsigned()}), &["user","revoked_sessions"]);
    s["TotpDisabled"] = object(json!({"totp_enabled":boolean(),"revoked_sessions":unsigned()}), &["totp_enabled","revoked_sessions"]);
    s["TotpStart"] = object(
        json!({"secret":string(),"uri":{"type":"string","format":"uri"}}),
        &["secret", "uri"],
    );
    s["TotpConfirmRequest"] = object(
        json!({"code":{"type":"string","pattern":"^[0-9]{6}$","writeOnly":true}}),
        &["code"],
    );
    s["RecoveryCodes"] = object(
        json!({"recovery_codes":array(string())}),
        &["recovery_codes"],
    );
    s["TokenRequest"] = object(
        json!({"name":name,"scopes":{"type":"array","minItems":1,"items":{"type":"string","enum":["read","control","library","admin"]}}}),
        &["name", "scopes"],
    );
    s["TokenCreated"] = object(json!({"id":string(),"token":string()}), &["id", "token"]);
    s["Token"] = object(
        json!({"id":string(),"name":string(),"scopes":array(string()),"created_at":timestamp,"expires_at":timestamp}),
        &["id", "name", "scopes", "created_at", "expires_at"],
    );
    s["Session"] = object(
        json!({"id":string(),"name":string(),"created_at":timestamp,"expires_at":timestamp,"current":boolean(),"device_name":nullable(string()),"device_type":nullable(string())}),
        &["id", "name", "created_at", "expires_at", "current"],
    );
    s["Track"] = object(
        json!({
            "id":string(),"title":string(),"artist":string(),"album":string(),"album_artist":string(),
            "duration":seconds,"track_number":unsigned(),"disc_number":unsigned(),"source":string(),"format":string(),
            "sample_rate":unsigned(),"bit_depth":unsigned(),"channels":unsigned(),"bitrate":unsigned(),
            "cover_url":nullable(string()),"recording_id":string(),"available":boolean(),"created_at":timestamp,
            "integrated_lufs":nullable(json!({"type":"number"})),"provider_data":{"description":"音源与人工来源元数据；可能为任意 JSON"}
        }),
        &[
            "id",
            "title",
            "artist",
            "album",
            "album_artist",
            "duration",
            "track_number",
            "disc_number",
            "source",
            "format",
            "sample_rate",
            "bit_depth",
            "channels",
            "bitrate",
            "cover_url",
            "recording_id",
            "available",
            "created_at",
        ],
    );
    s["Track"]["description"] =
        json!("技术参数 0 表示未知；recording_id 是录音边界，不表示已有完整作品关系图。");
    s["Album"] = object(
        json!({"id":string(),"title":string(),"artist":string(),"cover_url":nullable(string()),"track_count":unsigned()}),
        &["id", "title", "artist", "cover_url", "track_count"],
    );
    s["Artist"] = object(
        json!({"id":string(),"name":string(),"track_count":unsigned(),"image_url":nullable(string()),"description":nullable(string())}),
        &["id", "name", "track_count"],
    );
    s["Root"] = object(json!({"id":string(),"path":string()}), &["id", "path"]);
    s["RootRequest"] = object(
        json!({"path":{"type":"string","description":"核心可访问的目录；Docker 挂载目录使用 /music"}}),
        &["path"],
    );
    s["Job"] = object(
        json!({"id":string(),"kind":string(),"status":string(),"processed":unsigned(),"total":unsigned(),"error":nullable(string())}),
        &["id", "kind", "status", "processed", "total", "error"],
    );
    s["JobStarted"] = object(json!({"id":string(),"status":string()}), &["id", "status"]);
    s["JobActionRequest"] = object(
        json!({"status":{"type":"string","enum":["paused","running","cancelled"]}}),
        &["status"],
    );
    s["MediaTicket"] = object(
        json!({"url":{"type":"string","description":"含路径前缀的相对 URL，附限定曲目票据"},"api_path":{"type":"string","description":"从 api/v1/media/ 开始的相对路径，追加到当前连接地址及反代前缀"},"expires_at":timestamp}),
        &["url", "api_path", "expires_at"],
    );
    s["EventTicket"] = object(
        json!({"ticket":{"type":"string","description":"60 秒内建立事件 WebSocket，绑定创建会话"}}),
        &["ticket"],
    );
    s["LyricDocument"] = object(
        json!({"format":{"type":"string","enum":["lrc","plain","enhanced_lrc"]},"text":string(),"translation":nullable(string()),"offset_ms":number(),"source":string()}),
        &["format", "text", "offset_ms", "source"],
    );
    s["EqBand"] = object(
        json!({"enabled":{"type":"boolean","default":true},"kind":{"type":"string","enum":["peak","low_shelf","high_shelf","low_pass","high_pass","band_pass","notch","all_pass"],"default":"peak"},"frequency":{"type":"number","minimum":10,"maximum":192000},"gain_db":{"type":"number","minimum":-30,"maximum":30},"q":{"type":"number","minimum":0.1,"maximum":30}}),
        &["frequency", "gain_db", "q"],
    );
    s["EqualizerImport"] = object(json!({"text":{"type":"string","maxLength":65536}}), &["text"]);
    s["EqualizerPreset"] = object(json!({"preamp_db":number(),"bands":{"type":"array","maxItems":32,"items":reference("EqBand")}}), &["preamp_db","bands"]);
    s["DspConfig"] = object(
        json!({"enabled":{"type":"boolean","default":false},"preamp_db":{"type":"number","minimum":-60,"maximum":24,"default":0},"balance":{"type":"number","minimum":-1,"maximum":1,"default":0},"normalize":{"type":"boolean","default":false},"target_lufs":{"type":"number","minimum":-40,"maximum":-5,"default":-14},"bands":{"type":"array","maxItems":32,"items":reference("EqBand")},"preset_name":nullable(string())}),
        &[],
    );
    s["DspConfig"]["description"] = json!(
        "PUT 替换整个 DSP 对象，建议发送全部字段；归一化需测量值，AirPlay 当前不支持此 DSP。"
    );
    s["DspConfig"]["properties"]["convolution"]=nullable(reference("ConvolutionConfig"));
    s["EqBand"]["properties"]["slope_db_oct"]=json!({"type":"integer","enum":[12,24,36,48],"description":"限低通或高通。默认 12 dB/oct"});
    s["DspConfig"]["properties"]["eq_bypass"]=boolean();
    s["DspConfig"]["properties"]["modules"]=json!({"type":"array","uniqueItems":true,"items":{"enum":["rate","stereo","loudness","gain","eq","fades","convolution"]}});
    s["DspConfig"]["properties"]["extra_eq"]=json!({"type":"array","maxItems":3,"items":{"type":"object","properties":{"bypass":boolean(),"bands":{"type":"array","maxItems":32,"items":reference("EqBand")}}}});
    s["DspConfig"]["properties"]["stereo"]=nullable(object(json!({
      "crossfeed":{"type":"number","minimum":0,"maximum":0.5},
      "crossover_hz":{"type":"number","minimum":200,"maximum":2000},
      "mono":boolean(),"invert_left":boolean(),"invert_right":boolean()
    }), &[]));
    s["DspConfig"]["properties"]["sample_rate"]=nullable(json!({
        "type":"integer","enum":rmusic_audio::output::PCM_RATES,
        "description":"支持 Core DSP 的网桥音区可设置目标采样率。省略或 null 时跟随音源。设备格式上限优先。"
    }));
    s["Autoplay"] = object(
        json!({"enabled":{"type":"boolean","default":true},"source":{"type":"string","description":"local、mixed 或已启用连接器 ID","default":"mixed"},"fallback_local":{"type":"boolean","default":true}}),
        &["enabled", "source", "fallback_local"],
    );
    s["Autoplay"]["description"] = json!("PATCH Zone 时提供完整对象；新音区默认开启，已有显式配置保留。仅在 repeat=off 的队列末尾准备续播，云端失败时是否回退本地由 fallback_local 决定。");
    s["Zone"] = object(
        json!({"id":string(),"name":string(),"endpoint_id":string(),"status":status,"volume":volume,"muted":boolean(),"repeat":{"type":"string","enum":["off","one","all"]},"shuffle":boolean(),"position":seconds,"track":nullable(reference("Track")),"queue":array(reference("Track")),"queue_index":unsigned(),"updated_at":milliseconds,"dsp":reference("DspConfig"),"autoplay":reference("Autoplay")}),
        &[
            "id",
            "name",
            "endpoint_id",
            "status",
            "volume",
            "muted",
            "repeat",
            "shuffle",
            "position",
            "track",
            "queue",
            "queue_index",
            "updated_at",
            "dsp",
            "autoplay",
        ],
    );
    s["NewZoneRequest"] = object(
        json!({"name":name,"endpoint_id":nullable(string())}),
        &["name"],
    );
    s["ZonePatch"] = object(
        json!({"name":name,"endpoint_id":string(),"volume":volume,"muted":boolean(),"repeat":{"type":"string","enum":["off","one","all"]},"shuffle":boolean(),"autoplay":reference("Autoplay")}),
        &[],
    );
    s["Transitions"]=object(json!({"fade_in_ms":{"type":"integer","minimum":0,"maximum":10000,"default":0},"fade_out_ms":{"type":"integer","minimum":0,"maximum":10000,"default":0}}),&["fade_in_ms","fade_out_ms"]);
    s["Zone"]["properties"]["transitions"]=reference("Transitions");
    s["Zone"]["properties"]["queue_revision"]=unsigned();
    s["Zone"]["properties"]["queue_length"]=unsigned();
    s["ZoneQueuePage"]=object(json!({"id":string(),"revision":unsigned(),"epoch":string(),"offset":unsigned(),"total":unsigned(),"items":array(reference("Track"))}),&["id","revision","offset","total","items"]);
    s["Zone"]["properties"]["playback_id"]=nullable(string());
    s["Zone"]["required"].as_array_mut().unwrap().push(json!("playback_id"));
    s["ZonePatch"]["properties"]["transitions"]=reference("Transitions");
    s["QueuePutRequest"] = object(
        json!({"track_ids":ids,"queue_index":nullable(unsigned()),"shuffle":boolean()}),
        &["track_ids"],
    );
    s["QueuePutRequest"]["description"] = json!("替换队列；未指定 index 时保留仍在队列中的当前曲目和进度，移除当前曲目则停止。显式 queue_index 必须指向新队列。");
    s["QueuePostRequest"] = object(
        json!({"track_ids":ids,"insertion_index":nullable(unsigned()),"play_next":{"type":"boolean","default":false}}),
        &["track_ids"],
    );
    s["QueuePostRequest"]["description"] =
        json!("追加或在 insertion_index 插入；play_next 在锁内插到当前曲后且保留随机模式及播放进度，不可同时指定 index。显式插入不再次打乱。");
    s["QueueReorderRequest"]=object(json!({"expected_queue":ids,"expected_playback_id":nullable(string()),"order":{"type":"array","maxItems":10000,"items":unsigned()}}),&["expected_queue","expected_playback_id","order"]);
    s["QueueReorderRequest"]["description"]=json!("按原位置的完整排列调整队列；保留重复曲目的当前实例、位置、状态及随机模式。快照变化或正在整组同步时返回409，不撤销组。仅失效后续预载，不重新播放当前曲。");
    s["PlayRequest"] = object(
        json!({"track_id":nullable(string()),"queue_index":nullable(unsigned())}),
        &[],
    );
    s["ControlIdentity"]=object(json!({"endpoint_id":string(),"track_id":string(),"playback_id":string()}),&["endpoint_id","track_id","playback_id"]);
    s["SystemControlRequest"]=object(json!({"control":reference("ControlIdentity"),"position":seconds}),&[]);
    s["SystemControlRequest"]["description"]=json!("guarded_transport_v1：可选control须精确匹配当前端点、曲目和播放实例；在音区锁内先检查，冲突409且无控制或组撤销副作用。仅允许原曲播放／暂停／停止，不改变队列；position为端点已确认源位置。省略control沿用既有控制。");
    s["PlayRequest"]["properties"]["control"]=reference("ControlIdentity");
    s["PlayRequest"]["properties"]["position"]=seconds.clone();
    s["PlayRequest"]["properties"]["exact_version"]=json!({"type":"boolean","default":false,"description":"显式track_id点播时按指定版本播放，绕过长期播放首选及跨版本自动回退；暂停续播及原队列不换版。"});
    s["PlayRequest"]["description"]=json!("省略control沿用播放／选曲接口；携带control须协商guarded_transport_v1，只能恢复身份相同的当前曲目，不得附track_id或queue_index。可选position为端点确认位置，冲突409且不改变播放。");
    s["SeekRequest"] = object(json!({"position":seconds}), &["position"]);
    s["ProgressRequest"] = object(
        json!({"position":seconds,"status":status,"track_id":nullable(string()),"playback_id":nullable(string()),"recoverable":boolean(),"device_failed":boolean()}),
        &[],
    );
    s["ProgressRequest"]["description"] = json!("实际端点进度。声明 playback_instance 能力的端点必须附 track_id 和 Zone.playback_id；身份缺失／不匹配返回204且不改音区／统计。连续原生端点可配对提交 frames/sample_rate 和实际 position，凭证须属于端点注册会话；帧数为当前源在本次设备会话实际消费的累计节目帧，不含定位偏移／设备填充。核心暂停时接受显式 paused 或省略 status 的设备帧报告，省略 status 保留当前状态，迟到 playing 报告不恢复播放。连续端点仅提交 status、不提交 position 时不增加听音时长。旧端点可省略ID；显式旧ID始终拒绝。");
    s["EndedRequest"] = object(
        json!({"track_id":string(),"position":seconds,"playback_id":nullable(string())}),
        &["track_id"],
    );
    for name in ["ProgressRequest","EndedRequest"] {
        s[name]["properties"]["frames"]=unsigned();
        s[name]["properties"]["measured_version"]=json!({"type":"integer","enum":[1,2],"default":1});
        s[name]["properties"]["sample_rate"]=json!({"type":"integer","minimum":8000,"maximum":rmusic_audio::MAX_CLOCK_RATE});
        s[name]["dependentRequired"]=json!({"frames":["sample_rate","position"],"sample_rate":["frames","position"]});
    }
    s["EndedRequest"]["description"]=json!("仅用于真实自然EOF；声明 playback_instance 的端点须附 track_id 和 Zone.playback_id。身份不匹配／缺失或音区非playing时返回当前Zone且不变。连续原生端点可配对提交 frames/sample_rate 和实际 position，凭证须属于端点注册会话，最多二十四小时节目帧；统计按实际帧结束，不替换成曲库估计时长。先提交已听跨界，再结束最后实例；WebView 不应重复上报原生协调器已提交的EOF。旧端点仍可省略ID或实际position。");
    s["ZoneProgress"] = object(
        json!({"id":string(),"playback_id":nullable(string()),"position":seconds,"status":status,"updated_at":milliseconds}),
        &["id", "playback_id", "position", "status", "updated_at"],
    );
    s["PlaybackTransferRequest"] = object(json!({"target_zone_id":string(),"expected_playback_id":nullable(string())}), &["target_zone_id"]);
    s["PlaybackTransferResult"] = object(json!({"source":reference("Zone"),"target":reference("Zone")}), &["source","target"]);
    s["OutputReadyRequest"] = object(json!({"track_id":string(),"playback_id":string()}), &["track_id","playback_id"]);
    s["VisualizationPublishRequest"] = json!({"type":"object","description":"Current output session only; at most 24 frames, 8 byte-valued bands, actual playback identity and position. Accepted only under a remote viewer lease."});
    s["VisualizationLease"] = object(json!({"active":{"type":"boolean"}}), &["active"]);
    s["SequencePrepareRequest"] = object(json!({"endpoint_id":string(),"track_id":string(),"playback_id":string(),"plan_id":string(),"committed_playback_ids":{"type":"array","maxItems":32,"items":string()},"measured_version":{"type":"integer","enum":[1,2],"default":1}}), &["endpoint_id","track_id","playback_id"]);
    s["SequenceSlot"] = object(json!({"playback_id":string(),"listen_session_id":{"type":"string","format":"uuid"},"track":reference("Track"),"media_path":string(),"queue":{"type":"array","maxItems":10000,"items":reference("Track")},"queue_index":unsigned()}), &["playback_id","track","media_path"]);
    s["SequencePrepared"] = json!({"oneOf":[
        object(json!({"accepted":{"const":true},"measured_version":{"type":"integer","enum":[1,2]},"plan_id":string(),"listen_session_id":{"type":"string","format":"uuid"},"from_playback_id":string(),"items":{"type":"array","maxItems":8,"items":reference("SequenceSlot")},"has_more":boolean(),"refresh":boolean(),"continuous_recovery":boolean()}),&["accepted","plan_id","from_playback_id","items","has_more"]),
        object(json!({"accepted":{"const":false},"zone":reference("Zone")}),&["accepted","zone"])
    ]});
    s["SequenceBoundary"] = object(json!({"playback_id":string(),"track_id":string(),"position":seconds,"frames":unsigned()}), &["playback_id","track_id","position","frames"]);
    s["SequenceHandoffRequest"] = object(json!({
        "endpoint_id":string(),"request_id":{"type":"string","format":"uuid"},"plan_id":{"type":"string","format":"uuid"},"from_playback_id":{"type":"string","format":"uuid"},
        "sample_rate":{"type":"integer","minimum":8000,"maximum":rmusic_audio::MAX_CLOCK_RATE},
        "ended":{"type":"array","minItems":1,"maxItems":8,"items":reference("SequenceBoundary")},"current":reference("SequenceBoundary")
    }), &["endpoint_id","request_id","plan_id","from_playback_id","sample_rate","ended","current"]);
    s["SequenceHandoffRequest"]["description"] = json!("仅注册会话所属、声明 continuous_pcm 与 playback_instance 的端点可用。ended 按实际已听边界顺序，current 为下一预载实例；后续曲位置须等于 frames/sample_rate（容差一毫秒）。一次最多报告二十四小时的设备帧，总未记进度受核心计时约束。队列、统计、持久回执原子提交；相同请求重试 applied=false，ID 内容冲突409，旧计划 accepted=false。保留较新的暂停、音量和静音。");
    s["SequenceHandoffResult"] = object(json!({"accepted":boolean(),"applied":boolean(),"request_id":string(),"zone":reference("Zone")}), &["accepted","applied","zone"]);
    s["SequenceHandoffRequest"]["properties"]["measured_version"]=json!({"type":"integer","enum":[1,2],"default":1});
    s["SequenceHandoffRequest"]["description"]=json!("v1 保留源位置与帧时长相等的合同。v2 需核心 measured_listens_online_v2=true 且计划已显式准备为 measured_version=2：frames 为每曲实际输出累计帧，position 独立为源位置；固定 PCM 采样率、±500 ppm 及两采样边界误差。版本、实例、源顺序、计数／速率和总未报告时长全部校验后，队列、统计、游标与回执事务提交。相同请求重试 applied=false，改正文409；普通 v2 实例需显式 measured_listens_recovery_v2 能力才开放持久恢复；分组需 group_output_v2、同版计划及全组能力，按整组实际输出计一次。");
    s["SequenceRecoverRequest"] = object(json!({
        "endpoint_id":string(),"request_id":{"type":"string","format":"uuid"},"plan_id":{"type":"string","format":"uuid"},
        "sample_rate":{"type":"integer","minimum":8000,"maximum":rmusic_audio::MAX_CLOCK_RATE},
        "heard":{"type":"array","minItems":1,"maxItems":9,"items":reference("SequenceBoundary")},"terminal":boolean()
    }), &["endpoint_id","request_id","plan_id","sample_rate","heard","terminal"]);
    s["SystemSuspend"]=object(json!({"host_id":{"type":"string","format":"uuid"},"generation":{"type":"integer","minimum":1}}),&["host_id","generation"]);
    s["SystemSuspend"]["additionalProperties"]=json!(false);
    s["SequenceRecoverRequest"]["properties"]["measured_version"]=json!({"type":"integer","enum":[1,2],"default":1});
    s["SequenceRecoverRequest"]["properties"]["system_suspend"]=nullable(reference("SystemSuspend"));
    s["ProgressRequest"]["properties"]["system_suspend"]=nullable(reference("SystemSuspend"));
    s["SequenceRecoverRequest"]["description"]=json!("仅 status.capabilities.continuous_recovery=true 且端点在 prepare 前声明 continuous_recovery、continuous_pcm、playback_instance 时使用。重新注册后可由同一账号的新认证会话恢复；v2 还需 Core／端点 measured_listens_recovery_v2=true 且原计划绑定同一计时版本；恢复仍保持 v2，frames 是实际输出帧，position 独立为源位置并受 ±500ppm／两采样误差约束；分组不使用此单端恢复接口。heard 为有界、连续、实际听到的源检查点，最后一项以外均已结束，terminal 表示最后一项也自然结束。Core 校验持久计划、最新队列及控制意图，停止／定位／切歌／队列或处理变更撤销旧恢复。接受后绑定新的播放ID和plan_id，保留最新暂停／音量／静音，实际位置不回退已确认区间；已持久确认源EOF不重播。接口不补算听音，统计由持久设备日志独立去重补传。相同不可变请求重试幂等，内容冲突409；若已接受后又失联或Core重启，相同回执可重新绑定最新游标，仍不恢复被新控制撤销的计划。请求方收到响应后按新实例打开输出，不依赖旧 play 推送。");
    s["SequenceRecoverResult"] = object(json!({"accepted":boolean(),"applied":boolean(),"request_id":string(),"plan_id":nullable(string()),"finished":boolean(),"waiting_for_play":boolean(),"zone":reference("Zone")}), &["accepted","applied","zone"]);
    s["SequenceRecoverResult"]["properties"]["measured_version"]=json!({"type":"integer","enum":[1,2]});
    s["SequenceRecoverResult"]["properties"]["system_suspend_ack"]=nullable(reference("SystemSuspend"));
    s["SequenceRecoveryUnavailableRequest"]=object(json!({"endpoint_id":string()}),&["endpoint_id"]);
    s["SequenceRecoveryUnavailableRequest"]["properties"]["system_suspend"]=nullable(reference("SystemSuspend"));
    s["SequenceRecoveryUnavailableRequest"]["properties"]["playback_id"]=nullable(json!({"type":"string","format":"uuid"}));
    s["SequenceRecoveryUnavailableRequest"]["additionalProperties"]=json!(false);
    s["SequenceRecoveryUnavailableRequest"]["description"]=json!("仅已声明 recovery_controls 的当前注册端点，在确认没有本账号活跃本地检查点且没有存活输出时报告。省略 system_suspend 时沿用旧合同：只撤销同账号、同端点、仍被迫暂停且没有播放 ID 的持久计划，明确 play 后才从 Core 位置起播。携带 system_suspend 时须先协商 system_suspend_fresh_start：独立持久等待新的手动 play，不构造听音游标；确认绑定音区／端点／账号／挂起代数及新播放 ID，重启后的旧实例授权失效。可选 playback_id 防止迟到准入影响新实例。");
    s["SequenceRecoveryUnavailableResult"]=object(json!({"applied":boolean(),"zone":reference("Zone")}),&["applied","zone"]);
    s["SequenceRecoveryUnavailableResult"]["properties"]["accepted"]=boolean();
    s["SequenceRecoveryUnavailableResult"]["properties"]["waiting_for_play"]=boolean();
    s["SequenceRecoveryUnavailableResult"]["properties"]["system_suspend_ack"]=nullable(reference("SystemSuspend"));
    for name in ["SequencePrepareRequest","SequenceBoundary","SequenceHandoffRequest","SequenceRecoverRequest"] { s[name]["additionalProperties"] = json!(false); }
    s["HostClockCalibration"]=object(json!({"version":{"type":"integer","const":1},"core_clock_id":string(),"host_clock_id":string(),"host_minus_core_ns":{"type":"integer"},"network_rtt_ns":{"type":"integer","minimum":0},"network_uncertainty_ns":{"type":"integer","minimum":0},"observed_at_core_ns":{"type":"integer","minimum":0},"valid_until_core_ns":{"type":"integer","minimum":0},"sample_count":{"type":"integer","minimum":1,"maximum":8},"device_clock_mapped":{"type":"boolean","const":false}}),&["version","core_clock_id","host_clock_id","host_minus_core_ns","network_rtt_ns","network_uncertainty_ns","observed_at_core_ns","valid_until_core_ns","sample_count","device_clock_mapped"]);
    s["SyncStartRequest"]=object(json!({"request_id":{"type":"string","format":"uuid"},"zone_ids":{"type":"array","items":{"type":"string","minLength":1,"maxLength":200},"minItems":2,"maxItems":4,"uniqueItems":true},"track_id":string(),"position":{"type":"number","minimum":0,"default":0}}),&["request_id","zone_ids","track_id"]);
    s["SyncStartRequest"]["additionalProperties"]=json!(false);
    s["SyncStartRequest"]["properties"]["pcm_rate_adjustment"]=json!({"type":"boolean","default":false,"description":"实验 PCM 变速，默认关闭；须显式 measured_version=2，全部端点支持 pcm_rate_adjustment_v1 和 programme_clock_v2。每次重准备归零，不启用自动漂移控制。"});
    s["SyncStartRequest"]["properties"]["sync_policy"]=reference("SyncPolicy");
    s["SyncStartRequest"]["properties"]["automatic_rate_adjustment"]=json!({"type":"boolean","default":false,"description":"单独显式启用实验自动校正，须同时 PCM 变速、v2 及全部端点 group_rate_lease_v1。固定首个音区参考，±500 ppm、每秒20 ppm变化，使用有效观测速率和已知误差带，失效归零；原生租约最长2秒。物理误差未知，非严格同步。"});
    s["SyncStartRequest"]["properties"]["measured_version"]=json!({"type":"integer","enum":[1,2],"default":1,"description":"v2 需全组 group_output_v2 与 measured_listens_v2 能力；省略队列时转为一项有限 PCM 队列。源位置与实际输出时长分开，进度／交接／结束／最终回执均需相同版本；不启用变速或持久恢复。"});
    s["OutputGroup"]=object(json!({"id":{"type":"string","format":"uuid"},"name":{"type":"string","minLength":1,"maxLength":80},"zone_ids":{"type":"array","minItems":2,"maxItems":4,"uniqueItems":true,"items":string()},"revision":{"type":"integer","minimum":1},"updated_at":{"type":"integer"}}),&["id","name","zone_ids","revision","updated_at"]);
    s["OutputGroupSave"]=object(json!({"name":s["OutputGroup"]["properties"]["name"],"zone_ids":s["OutputGroup"]["properties"]["zone_ids"],"expected_revision":{"type":"integer","minimum":0}}),&["name","zone_ids","expected_revision"]);
    s["OutputGroupSave"]["additionalProperties"]=json!(false);
    s["OutputGroupList"]=object(json!({"items":{"type":"array","items":reference("OutputGroup")}}),&["items"]);
    s["SyncStartRequest"]["properties"]["queue"]=json!({"type":"array","maxItems":10000,"items":{"type":"string","minLength":1,"maxLength":200},"description":"有序 PCM 组队列，首项须为 track_id，允许重复曲目。所有端点须声明 scheduled_queue_v1；v1 省略或空队列维持单曲起播，v2 转为一项队列；有序队列在原始格式边界等待全员重新准备。"});
    s["SyncControlRequest"]=object(json!({"request_id":{"type":"string","format":"uuid"},"action":{"type":"string","enum":["pause","resume","seek","jump","members","resync"]},"position":{"type":"number","minimum":0,"maximum":86400},"queue_index":unsigned(),"from_queue_index":unsigned(),"zone_ids":{"type":"array","items":{"type":"string","minLength":1,"maxLength":200},"minItems":1,"maxItems":4,"uniqueItems":true},"from_zone_ids":{"type":"array","items":{"type":"string","minLength":1,"maxLength":200},"minItems":1,"maxItems":4,"uniqueItems":true}}),&["request_id","action"]);
    s["SyncControlRequest"]["additionalProperties"]=json!(false);
    s["SyncControlRequest"]["oneOf"]=json!([{"properties":{"action":{"enum":["pause","resume","resync"]}},"not":{"anyOf":[{"required":["position"]},{"required":["queue_index"]},{"required":["from_queue_index"]},{"required":["zone_ids"]},{"required":["from_zone_ids"]}]}},{"properties":{"action":{"const":"seek"}},"required":["position","queue_index"],"not":{"anyOf":[{"required":["from_queue_index"]},{"required":["zone_ids"]},{"required":["from_zone_ids"]}]}},{"properties":{"action":{"const":"jump"}},"required":["position","queue_index","from_queue_index"],"not":{"anyOf":[{"required":["zone_ids"]},{"required":["from_zone_ids"]}]}},{"properties":{"action":{"const":"members"}},"required":["zone_ids","from_zone_ids"],"not":{"anyOf":[{"required":["position"]},{"required":["queue_index"]},{"required":["from_queue_index"]}]}}]);
    s["GroupClockMember"]=object(json!({"zone_id":string(),"endpoint_id":string(),"available":boolean(),"reason":nullable(string()),"estimated_offset_ns":nullable(json!({"type":"integer"})),"relative_drift_ppm":nullable(json!({"type":"number"})),"projection":nullable(json!({"type":"string","enum":["observed","nominal"]})),"device_epoch":unsigned(),"valid_until_core_ns":unsigned(),"sample_age_ns":unsigned(),"driver_read_span_ns":unsigned(),"known_host_uncertainty_ns":unsigned()}),&["zone_id","endpoint_id","available","reason","estimated_offset_ns","relative_drift_ppm","projection"]);
    s["GroupClockMember"]["properties"]["programme_clock_version"]=json!({"type":"integer","enum":[1,2]});
    s["GroupClockMember"]["properties"]["source_per_output"]=json!({"type":"number","minimum":0.999499999999,"maximum":1.000500000001});
    s["GroupClockMember"]["properties"]["relative_programme_drift_ppm"]=nullable(json!({"type":"number"}));
    s["GroupClockMember"]["properties"]["processing_projection_uncertainty_ns"]=nullable(unsigned());
    s["GroupClockDiagnostics"]=object(json!({"version":{"type":"integer","const":1},"state":{"type":"string","enum":["inactive","single_output","available","partial","unavailable"]},"evaluated_at_core_ns":unsigned(),"valid_for_ms":{"type":"integer","minimum":0,"maximum":500},"reference_zone_id":nullable(string()),"members":array(reference("GroupClockMember")),"total_uncertainty_ns":{"type":"null"},"driver_accuracy_verified":{"type":"boolean","const":false},"strict_sync":{"type":"boolean","const":false},"correction":{"type":"string","const":"off"}}),&["version","state","evaluated_at_core_ns","valid_for_ms","reference_zone_id","members","total_uncertainty_ns","driver_accuracy_verified","strict_sync","correction"]);
    s["StartGate"]=object(json!({"phase":{"type":"string","enum":["immediate","armed","ready","committed","released","cancelled","expired"]},"expires_host_ns":unsigned(),"deadline_host_ns":unsigned(),"ready_host_ns":unsigned(),"released_host_ns":unsigned()}),&["phase","expires_host_ns","deadline_host_ns","ready_host_ns","released_host_ns"]);
    s["SyncStartMember"]=object(json!({"zone_id":string(),"endpoint_id":string(),"playback_id":{"type":"string","format":"uuid"},"phase":{"type":"string","enum":["preparing","ready","playing","pausing","paused","ended","cancelled"]},"deadline_host_ns":unsigned(),"sample_rate":unsigned(),"frames":unsigned(),"position":number(),"gate":nullable(reference("StartGate"))}),&["zone_id","endpoint_id","playback_id","phase","deadline_host_ns","sample_rate","frames","position","gate"]);
    s["GroupRateRequest"]=object(json!({"request_id":{"type":"string","format":"uuid"},"zone_id":{"type":"string","minLength":1,"maxLength":200},"playback_id":{"type":"string","format":"uuid","description":"members.playback_id 原起播纪元，不能使用后续曲目的 current_playback_id"},"ppm":{"type":"number","minimum":-500,"maximum":500}}),&["request_id","zone_id","playback_id","ppm"]);
    s["GroupRateRequest"]["additionalProperties"]=json!(false);
    s["SyncStartMember"]["properties"]["requested_rate_ppm"]=json!({"type":"number","minimum":-500,"maximum":500,"description":"仅实验组返回已接收目标，实际斜率须看 programme_clock v2；不表示硬件已应用。"});
    s["SyncStart"]=object(json!({"id":{"type":"string","format":"uuid"},"track_id":string(),"position":number(),"phase":{"type":"string","enum":["preparing","committed","playing","pausing","paused","seeking","ended","cancelled","failed","interrupted"]},"error":nullable(string()),"created_at_core_ns":unsigned(),"expires_at_core_ns":unsigned(),"deadline_core_ns":nullable(unsigned()),"may_have_started":boolean(),"statistics_scope":{"type":"string","const":"group"},"strict_sync":{"type":"boolean","const":false},"members":array(reference("SyncStartMember"))}),&["id","track_id","position","phase","error","created_at_core_ns","expires_at_core_ns","deadline_core_ns","may_have_started","statistics_scope","strict_sync","members"]);
    s["SyncStart"]["properties"]["measured_version"]=json!({"type":"integer","enum":[1,2],"default":1});
    s["SyncStart"]["properties"]["playhead"]=object(json!({"queue_index":unsigned(),"position":number()}),&["queue_index","position"]);
    s["SyncStart"]["properties"]["listened_seconds"]=number();
    s["SyncStart"]["properties"]["clock_diagnostics"]=reference("GroupClockDiagnostics");
    s["SyncStart"]["properties"]["transport"]=nullable(object(json!({"request_id":{"type":"string","format":"uuid"},"action":{"type":"string","enum":["pause","resume","seek","jump","members","resync"]},"revision":unsigned(),"state":{"type":"string","enum":["pending","applied","failed"]},"error":nullable(string()),"incomplete_final_reports":boolean(),"target_zone_ids":nullable(array(string()))}),&["request_id","action","revision","state","error","incomplete_final_reports"]));
    s["SyncStart"]["properties"]["settled"]=boolean();
    s["SyncStart"]["properties"]["title"]=nullable(string());
    s["SyncStartList"]=object(json!({"items":array(reference("SyncStart")),"total":unsigned()}),&["items","total"]);
    s["GroupFinalSource"]=object(json!({"playback_id":string(),"track_id":string(),"position":number(),"frames":unsigned(),"ended":boolean()}),&["playback_id","track_id","position","frames","ended"]);
    s["GroupFinalSource"]["additionalProperties"]=json!(false);
    s["GroupFinalReport"]=object(json!({"type":{"const":"start_state"},"version":{"const":1},"measured_version":{"const":2},"phase":{"enum":["cancelled","expired"]},"final":{"const":true},"start":{"type":"object"},"zone_id":string(),"core_playback_id":string(),"gate":{"type":"object"},"sample_rate":unsigned(),"programme_frames_played":unsigned(),"sequence_final":{"type":"array","maxItems":9,"items":reference("GroupFinalSource")}}),&["type","version","measured_version","phase","final","start","zone_id","core_playback_id","gate","sample_rate","programme_frames_played","sequence_final"]);
    s["GroupFinalReceipt"]=object(json!({"request_id":{"type":"string","format":"uuid"},"endpoint_id":string(),"captured_at":unsigned(),"captured_host_ns":unsigned(),"report":reference("GroupFinalReport")}),&["request_id","endpoint_id","captured_at","captured_host_ns","report"]);
    s["GroupFinalReceipt"]["additionalProperties"]=json!(false);
    s["GroupFinalReceipt"]["description"]=json!("输出退出后冻结的原生分组 v2 最终回执：captured_at 为 UTC 毫秒，captured_host_ns 为原宿主时钟纳秒。须原端点账号 control 权限、原授权令牌与当前／预载源；原纪元已退出在线输出及收尾窗口后接收。每纪元每端点仅一份不可变回执，UUID 和正文一起持久保存。纳秒／帧字段须保留精确整数，不经过 JavaScript Number 重建。不恢复播放；不修改后来源游标。旧授权缺少时钟预算时拒绝有声补报。");
    s["GroupFinalAck"]=object(json!({"applied":boolean()}),&["applied"]);
    s["GroupRecoveryState"]=object(json!({"available":boolean(),"reason":string(),"checkpoint_id":{"type":"string","pattern":"^[0-9a-f]{64}$"},"queue_index":unsigned(),"position":number()}),&["available"]);
    s["GroupRecoveryRequest"]=object(json!({"request_id":{"type":"string","format":"uuid"},"checkpoint_id":{"type":"string","pattern":"^[0-9a-f]{64}$"}}),&["request_id","checkpoint_id"]);
    s["GroupRecoveryRequest"]["additionalProperties"]=json!(false);
    s["GroupRecoveryRequest"]["description"]=json!("显式手动恢复已中断的 v2 整组；先 GET recovery 获取可用检查点，再冻结 UUID 和 checkpoint_id。只接受原听音账号 control 权限、全部旧最终帧已确认、原音区意图未被修改、原端点账号未变和当前全组能力／校准有效的情形。重新获取 Core／宿主时钟和输出身份，全部准备完成才提交起播；恢复不增加听音时长，续接当前曲原听音次数。旧单端恢复不适用。整组恢复不是自动播放。");
    s["SyncStartMember"]["properties"]["final_report_received"]=boolean();
    s["SyncStart"]["properties"]["rate_adjustment"]=object(json!({"enabled":{"type":"boolean","const":true},"mode":{"type":"string","const":"manual"},"automatic":{"type":"boolean","const":false},"last_request_id":nullable(json!({"type":"string","format":"uuid"})),"reset_on_rearm":{"type":"boolean","const":true}}),&["enabled","mode","automatic","last_request_id","reset_on_rearm"]);
    s["SyncStart"]["properties"]["rate_adjustment"]["properties"]["mode"]=json!({"type":"string","enum":["manual","experimental_auto"]});
    s["SyncStart"]["properties"]["rate_adjustment"]["properties"]["automatic"]=boolean();
    s["SyncStart"]["properties"]["rate_adjustment"]["properties"]["controller"]=object(json!({"state":{"type":"string","enum":["inactive","warming","tracking","held"]},"reason":nullable(string()),"lease_ms":{"type":"integer","const":1500},"max_ppm":{"type":"integer","const":500},"max_slew_ppm_per_second":{"type":"integer","const":20},"driver_accuracy_verified":{"type":"boolean","const":false}}),&["state","reason","lease_ms","max_ppm","max_slew_ppm_per_second","driver_accuracy_verified"]);
    s["GroupClockDiagnostics"]["properties"]["correction"]=json!({"type":"string","enum":["off","experimental"]});
    s["SyncStartMember"]["properties"]["queue_index"]=unsigned();
    s["SyncStartMember"]["properties"]["current_playback_id"]=nullable(string());
    s["SyncStart"]["properties"]["queue"]=nullable(object(json!({"length":unsigned(),"segment_start":unsigned()}),&["length","segment_start"]));
    s["Endpoint"] = object(
        json!({"id":string(),"name":string(),"kind":{"type":"string","examples":["browser","wasapi","airplay"]},"online":boolean(),"verified":boolean(),"capabilities":{"type":"object","properties":{"range":boolean(),"dsp":{"anyOf":[boolean(),string()]},"strict_sync":boolean(),"gapless":boolean(),"continuous_pcm":boolean(),"playback_instance":boolean()},"additionalProperties":true},"last_seen_ms":milliseconds,"clock":{"type":"object","additionalProperties":true}}),
        &["id", "name", "kind", "online", "verified", "capabilities"],
    );
    s["Endpoint"]["properties"]["clock_calibration"]=json!({"anyOf":[reference("HostClockCalibration"),{"type":"null"}]});
    s["Endpoint"]["properties"]["capabilities"]["properties"]["host_clock_v1"]=boolean();
    s["GroupPowerStamp"]=object(json!({"host_id":{"type":"string","format":"uuid"},"generation":unsigned(),"wake_generation":unsigned()}),&["host_id","generation","wake_generation"]);
    s["GroupPowerStamp"]["description"]=json!("当前认证时钟交换携带的电源代数。Core 仅在明确分组起播／继续／定位时捕获，自动换格式沿用旧授权；端点验证当前挂起与唤醒代数并持久确认，唤醒不自行起播。");
    s["Endpoint"]["properties"]["group_power"]=nullable(reference("GroupPowerStamp"));
    s["Endpoint"]["properties"]["group_power_clock_id"]=nullable(string());
    s["Endpoint"]["properties"]["capabilities"]["properties"]["group_power_v1"]=boolean();
    let source=json!({"type":"string","enum":["wasapi_ticks","alsa_frames","alsa_native_words","asio_frames","asio_dsd_bits","android_frames"]});
    let domain=json!({"type":"string","enum":["qpc","monotonic","driver_opaque"]});
    let ns=json!({"type":"integer","minimum":0});
    let frequency=json!({"type":"integer","minimum":1});
    s["DeviceClockSample"]=object(json!({"version":{"type":"integer","const":1},"epoch":frequency,"source":source,"timestamp_domain":domain,"counter":ns,"nominal_frequency_hz":frequency,"device_time_ns":ns,"host_read_before_ns":ns,"host_read_after_ns":ns,"running":boolean()}),&["version","epoch","source","timestamp_domain","counter","nominal_frequency_hz","device_time_ns","host_read_before_ns","host_read_after_ns","running"]);
    s["DeviceClockSample"]["properties"]["programme"]=object(json!({"frames":ns,"sample_rate":{"type":"integer","minimum":8000,"maximum":768000}}),&["frames","sample_rate"]);
    s["ProgrammeClockCalibration"]=object(json!({"version":{"type":"integer","const":1},"zone_id":string(),"core_playback_id":{"type":"string","format":"uuid"},"track_id":string(),"position":{"type":"number","minimum":0,"maximum":86400},"source_frames":ns,"output_frame":ns,"sample_rate":{"type":"integer","minimum":8000,"maximum":768000},"device_epoch":frequency,"device_counter":ns,"core_sample_ns":nullable(ns.clone()),"driver_accuracy_verified":{"type":"boolean","const":false}}),&["version","zone_id","core_playback_id","track_id","position","source_frames","output_frame","sample_rate","device_epoch","device_counter","core_sample_ns","driver_accuracy_verified"]);
    s["ProgrammeClockCalibrationV2"]=s["ProgrammeClockCalibration"].clone();
    s["ProgrammeClockCalibrationV2"]["properties"]["version"]=json!({"type":"integer","const":2});
    s["ProgrammeClockCalibrationV2"]["properties"].as_object_mut().unwrap().remove("source_frames");
    s["ProgrammeClockCalibrationV2"]["required"].as_array_mut().unwrap().retain(|v|v!="source_frames");
    for (name,schema) in [("source_frame",unsigned()),("source_fraction",json!({"type":"number","minimum":0,"exclusiveMaximum":1})),("source_per_output",json!({"type":"number","minimum":0.999499999999,"maximum":1.000500000001})),("track_output_frames",unsigned())] {
        s["ProgrammeClockCalibrationV2"]["properties"][name]=schema;s["ProgrammeClockCalibrationV2"]["required"].as_array_mut().unwrap().push(json!(name));
    }
    s["ProgrammeClockCalibrationV2"]["description"]=json!("Paired diagnostic v2: source_frame and source_fraction are independent programme coordinates since this output epoch; output_frame and track_output_frames remain actual output counts. source_per_output is the retained cursor slope, never a newer target. No hardware precision or strict-sync guarantee.");
    s["DeviceRateObservation"]=object(json!({"observed_frequency_hz":number(),"observed_drift_ppm":number(),"span_ns":ns,"sample_count":{"type":"integer","minimum":4,"maximum":32},"query_time_approximation":boolean(),"rate_sane":boolean()}),&["observed_frequency_hz","observed_drift_ppm","span_ns","sample_count","query_time_approximation","rate_sane"]);
    s["DeviceClockCalibration"]=object(json!({"version":{"type":"integer","const":1},"core_clock_id":string(),"host_clock_id":string(),"device_epoch":frequency,"source":source,"timestamp_domain":domain,"counter":ns,"nominal_frequency_hz":frequency,"core_sample_ns":nullable(ns.clone()),"core_read_before_ns":ns,"core_read_after_ns":ns,"host_network_uncertainty_ns":ns,"driver_read_span_ns":ns,"valid_until_core_ns":ns,"running":boolean(),"device_clock_mapped":boolean(),"driver_accuracy_verified":{"type":"boolean","const":false},"rate_observation":nullable(reference("DeviceRateObservation"))}),&["version","core_clock_id","host_clock_id","device_epoch","source","timestamp_domain","counter","nominal_frequency_hz","core_sample_ns","core_read_before_ns","core_read_after_ns","host_network_uncertainty_ns","driver_read_span_ns","valid_until_core_ns","running","device_clock_mapped","driver_accuracy_verified","rate_observation"]);
    s["DeviceClockCalibration"]["properties"]["programme_clock"]=nullable(json!({"oneOf":[reference("ProgrammeClockCalibration"),reference("ProgrammeClockCalibrationV2")]}));
    s["Endpoint"]["properties"]["device_clock_calibration"]=nullable(reference("DeviceClockCalibration"));
    s["Endpoint"]["properties"]["clock"]["properties"]["device_clock"]=nullable(reference("DeviceClockSample"));
    s["Endpoint"]["properties"]["capabilities"]["properties"]["device_clock_v1"]=boolean();
    s["Endpoint"]["properties"]["capabilities"]["properties"]["programme_clock_v2"]=boolean();
    s["Endpoint"]["properties"]["capabilities"]["properties"]["pcm_rate_adjustment_v1"]=boolean();
    s["Status"]["properties"]["capabilities"]["properties"]["group_rate_adjustment_v1"]=boolean();
    s["Status"]["properties"]["capabilities"]["properties"]["automatic_group_rate_v1"]=boolean();
    s["Endpoint"]["properties"]["capabilities"]["properties"]["group_rate_lease_v1"]=boolean();
    s["Endpoint"]["description"] =
        json!("能力以实际声明为准；HQPlayer 由独立网桥管理播放和报告输出，R-Music DSP 不重复处理。当前无严格同步或无缝播放保证。");
    s["Endpoint"]["properties"]["capabilities"]["properties"]["continuous_recovery"]=boolean();
    s["Endpoint"]["properties"]["capabilities"]["properties"]["recovery_controls"]=boolean();
    s["Endpoint"]["properties"]["capabilities"]["properties"]["convolution_v1"]=boolean();
    s["BrowserEndpointRequest"] = object(
        json!({"id":{"type":"string","maxLength":100,"pattern":"^browser-[A-Za-z0-9-]*$"},"name":name,"playback_instance":{"type":"boolean","default":false},"continuous_pcm":{"type":"boolean","default":false,"description":"仅在原生连续输出适配器就绪时声明；不等于 gapless 或 verified 验收通过"}}),
        &["id", "name"],
    );
    s["BrowserEndpointRequest"]["properties"]["registration_id"]=string();
    s["BrowserEndpointRequest"]["properties"]["visualization_v1"]=boolean();
    s["BrowserEndpointRequest"]["properties"]["continuous_recovery"]=json!({"type":"boolean","default":false,"description":"须同时声明 playback_instance 和 continuous_pcm；仅持久检查点及新实例恢复适配器实际就绪时启用，不代表硬件／长期验收"});
    for name in ["measured_listens_v2","measured_listens_recovery_v2"] {s["BrowserEndpointRequest"]["properties"][name]=json!({"type":"boolean","default":false});s["Endpoint"]["properties"]["capabilities"]["properties"][name]=boolean();}
    s["BrowserEndpointRequest"]["properties"]["recovery_controls"]=json!({"type":"boolean","default":false,"description":"须同时声明 continuous_recovery；恢复期间无指定曲目的 play 只更新最新意图，等待实际检查点。端点须实现 recover/unavailable，防止正常退出后没有活跃本地检查点时卡在等待恢复。旧适配器省略时保持原 play 行为。"});
    s["BrowserEndpointRequest"]["properties"]["convolution_v1"]=json!({"type":"boolean","default":false,"description":"须同时声明 playback_instance 和 continuous_pcm；仅在所选资源加载、按实际处理率匹配与 FIR 输出适配器接通后声明，旧端点默认不支持。"});
    s["CacheEntry"] = object(
        json!({"id":string(),"track_id":string(),"format":string(),"size":unsigned(),"complete":boolean(),"pinned":boolean(),"last_access":timestamp,"total_size":unsigned(),"title":nullable(string()),"artist":nullable(string()),"play_count":unsigned(),"exports":nullable(reference("CacheExport"))}),
        &[
            "id",
            "track_id",
            "format",
            "size",
            "complete",
            "pinned",
            "last_access",
            "total_size",
            "title",
            "artist",
            "play_count",
            "exports",
        ],
    );
    s["CacheExport"] = object(
        json!({"status":string(),"path":nullable(string()),"size":unsigned(),"error":nullable(string())}),
        &["status", "path", "size", "error"],
    );
    s["ExportQueued"] = object(
        json!({"status":{"type":"string","const":"pending"}}),
        &["status"],
    );
    s["CacheStatus"] = object(
        json!({"total":unsigned(),"limit_bytes":unsigned(),"used_bytes":unsigned(),"portable_bytes":unsigned(),"directory":string(),"eviction":{"type":"string","const":"least_played"},"entries":array(reference("CacheEntry"))}),
        &[
            "limit_bytes",
            "used_bytes",
            "portable_bytes",
            "directory",
            "eviction",
            "entries",
        ],
    );
    s["CacheLimit"] = object(
        json!({"limit_bytes":{"type":"integer","minimum":0,"maximum":1125899906842624_u64},"directory":{"type":"string","description":"核心容器或宿主的独立、可写、绝对路径；改变目录不自动迁移旧缓存文件"},"portable_directory":{"type":"string","description":"整理后的完整音乐输出目录，文件自动登记为受容量管理的本地曲库副本"}}),
        &[],
    );
    s["TrackStatistics"] = object(
        json!({"track_id":string(),"play_count":unsigned(),"skip_count":unsigned(),"listened_seconds":seconds,"last_played_at":nullable(timestamp.clone())}),
        &[
            "track_id",
            "play_count",
            "skip_count",
            "listened_seconds",
            "last_played_at",
        ],
    );
    s["RankedTrackStatistics"] = s["TrackStatistics"].clone();
    s["RankedTrackStatistics"]["properties"]["title"] = string();
    s["RankedTrackStatistics"]["properties"]["artist"] = string();
    s["RankedTrackStatistics"]["properties"]["cover_url"] = nullable(string());
    s["RankedTrackStatistics"]["required"]
        .as_array_mut()
        .unwrap()
        .extend([json!("title"), json!("artist"), json!("cover_url")]);
    s["StatisticsSummary"] = object(
        json!({"play_count":unsigned(),"skip_count":unsigned(),"listened_seconds":seconds,"unique_tracks":unsigned()}),
        &[
            "play_count",
            "skip_count",
            "listened_seconds",
            "unique_tracks",
        ],
    );
    s["DailyStatistics"] = object(
        json!({"day":{"type":"string","format":"date"},"play_count":unsigned(),"skip_count":unsigned(),"listened_seconds":seconds}),
        &["day", "play_count", "skip_count", "listened_seconds"],
    );
    s["ArtistStatistics"] = s["StatisticsSummary"].clone();
    s["ArtistStatistics"]["properties"]["artist"] = string();
    s["ArtistStatistics"]["required"]
        .as_array_mut()
        .unwrap()
        .push(json!("artist"));
    s["ListeningStatistics"] = object(
        json!({"period_days":{"type":"integer","minimum":0,"maximum":3650,"description":"0 表示历史所有"},"start_day":{"type":"string","format":"date"},"end_day":{"type":"string","format":"date"},"summary":reference("StatisticsSummary"),"days":array(reference("DailyStatistics")),"tracks":{"type":"array","maxItems":100,"items":reference("RankedTrackStatistics")},"artists":{"type":"array","maxItems":100,"items":reference("ArtistStatistics")}}),
        &[
            "period_days",
            "start_day",
            "end_day",
            "summary",
            "days",
            "tracks",
            "artists",
        ],
    );
    s["ListeningStatistics"]["properties"]["all_time"]=json!({"type":"boolean"});
    s["ListeningStatistics"]["properties"]["calendar_days"]=json!({"type":"integer","minimum":1});
    s["ListeningStatistics"]["properties"]["chart_bucket_days"]=json!({"type":"integer","minimum":1,"description":"图表每个点汇总的天数；图表最多366点，汇总与排行仍覆盖完整范围"});
    s["ListeningStatistics"]["description"]=json!("默认跟随 Core 的 statistics_timezone（初始 Asia/Shanghai），也可指定 IANA 时区；旧 UTC 日汇总保留原日期并通过 legacy_utc_seconds 标注。包含零播放日期；排名按真实听音秒数降序。每会话达到 min(30,max(5,曲长/2)) 秒后计一次；未知曲长需30秒。定位不累计听音，重启不累计停机时间。");
    for key in ["unique_albums","unique_artists","active_days","longest_streak"] {s["StatisticsSummary"]["properties"][key]=unsigned();}
    s["StatisticsSummary"]["properties"]["last_played_at"]=nullable(timestamp.clone());
    s["AlbumStatistics"]=object(json!({"album":string(),"artist":string(),"cover_url":nullable(string()),"play_count":unsigned(),"skip_count":unsigned(),"listened_seconds":number(),"unique_tracks":unsigned(),"last_played_at":nullable(timestamp.clone())}),&["album","artist","play_count","skip_count","listened_seconds","unique_tracks"]);
    s["ListeningStatistics"]["properties"]["timezone"]=string();s["ListeningStatistics"]["properties"]["legacy_utc_seconds"]=number();
    s["ListeningStatistics"]["properties"]["albums"]=json!({"type":"array","maxItems":100,"items":reference("AlbumStatistics")});
    s["ListeningStatistics"]["properties"]["sources"]=array(object(json!({"source":string(),"listened_seconds":number(),"play_count":unsigned()}),&["source","listened_seconds","play_count"]));
    s["ListeningStatistics"]["properties"]["ranking_limit"]=unsigned();
    s["ListeningHistoryItem"]=object(json!({"session_id":string(),"track_id":string(),"title":string(),"artist":string(),"album":string(),"album_artist":string(),"cover_url":nullable(string()),"source":string(),"started_at_ms":unsigned(),"finished_at_ms":nullable(unsigned()),"finish_reason":nullable(string()),"listened_seconds":number(),"counted":boolean(),"duration":number()}),&["session_id","track_id","title","artist","album","source","started_at_ms","listened_seconds","counted","duration"]);
    s["ListeningHistoryItem"]["description"]=json!("实际听音会话：时间戳为毫秒、时长为秒。保留中断与主动操作原因，不从播放到结尾推断整曲听完。旧记录缺失的专辑保持空，不猜测。统计是当前Core范围，不是个人偏好画像。");
    s["ListeningHistory"]=object(json!({"version":{"const":1},"items":{"type":"array","maxItems":100,"items":reference("ListeningHistoryItem")},"next_cursor":nullable(string()),"limit":unsigned(),"time_basis":{"const":"confirmed_playback"},"day_timezone":{"const":"UTC"},"scope":{"const":"core"}}),&["version","items","next_cursor","limit","time_basis","day_timezone","scope"]);
    s["OfflineListenDay"]=object(json!({"seconds":{"type":"number","exclusiveMinimum":0},"last_at":unsigned()}),&["seconds","last_at"]);
    s["OfflineListen"]=object(json!({"id":{"type":"string","format":"uuid"},"version":{"type":"integer","minimum":1},"track_id":string(),"title":string(),"artist":string(),"duration":{"type":"number","minimum":0,"maximum":604800},"started_at":unsigned(),"updated_at":unsigned(),"position":{"type":"number","minimum":0},"days":{"type":"object","maxProperties":64,"additionalProperties":reference("OfflineListenDay")},"reason":{"type":"string","enum":["active","ended","stop","replaced","replay","interrupted"]}}),&["id","version","track_id","title","artist","duration","started_at","updated_at","position","days","reason"]);
    s["OfflineListen"]["additionalProperties"]=json!(false);
    s["MeasuredOutputRange"]=object(json!({"start":unsigned(),"end":unsigned(),"sample_rate":{"type":"integer","minimum":8000,"maximum":768000}}),&["start","end","sample_rate"]);
    s["MeasuredListenSegment"]=object(json!({"start":{"type":"number","minimum":0},"end":{"type":"number","minimum":0},"last_at":unsigned(),"output":reference("MeasuredOutputRange")}),&["start","end","last_at"]);
    s["MeasuredListen"]=object(json!({"version":{"type":"integer","enum":[1,2],"default":1},"playback_id":{"type":"string","format":"uuid"},"listen_session_id":{"type":"string","format":"uuid"},"segments":{"type":"array","maxItems":64,"items":reference("MeasuredListenSegment")}}),&["playback_id","listen_session_id","segments"]);
    for name in ["MeasuredListenSegment","MeasuredListen","MeasuredOutputRange"] {s[name]["additionalProperties"]=json!(false);}
    s["OfflineListen"]["properties"]["measured"]=reference("MeasuredListen");
    s["OfflineStatsRequest"]=object(json!({"core_id":string(),"user_id":string(),"client_id":{"type":"string","format":"uuid"},"sessions":{"type":"array","minItems":1,"maxItems":32,"items":reference("OfflineListen")}}),&["core_id","user_id","client_id","sessions"]);
    s["OfflineStatsRequest"]["additionalProperties"]=json!(false);
    s["OfflineStatsRequest"]["description"]=json!("同一核心和已认证账号的累积实际听音快照；时间为 Unix 毫秒，days 使用 UTC 日期。按 user_id/client_id/id 与 version 原子去重，只增量累计秒数。同版本不同内容、已结束会话重开及累计倒退会隔离拒绝；较旧版本返回确认。元数据和起始时间不可改写，定位只更新位置基线。客户端保留记录直到对应版本获确认。不是声学验听或跨用户排行榜凭证。");
    s["MeasuredListen"]["description"]=json!("v1 仅 status.capabilities.measured_listens=true 时使用，省略 version 保持旧格式。v2 必须 measured_listens_v2=true、端点声明能力并在 prepare 显式选择 measured_version=2；该实例不能切换版本。v2 每段必须有 output 整数帧区间及固定 PCM 采样率，days 按实际帧时长计算；源位置独立保存，允许 ±500 ppm 与两个采样的边界误差。输出区间并集去重，旧源位置上报不为 v2 计时。v2 在线 HTTP 进度／交接／结束需 measured_listens_online_v2=true，WebSocket state 进度／结束另需 measured_listens_socket_v2=true，均显式传 measured_version=2；分组使用独立 group_output_v2 合同且不向端点发行个人听音 claim；普通 v2 持久恢复另需 measured_listens_recovery_v2 能力；整组恢复未开放，不得据此启用分组变速。playback_id/listen_session_id 须由当前认证账号在连续 prepare 中获得；片段为实际听到的源位置区间，递增不重叠，按 last_at 的 UTC 日期合计须与 days 一致。与在线设备上报按同一播放实例取区间并集，定位的新实例独立累计秒数但保留同一听音次数。历史补传不恢复音区。旧核心不得剥离此字段后按普通离线记录重传。");
    s["OfflineStatsAck"]=object(json!({"id":string(),"version":unsigned()}),&["id","version"]);
    s["OfflineStatsRejected"]=object(json!({"id":string(),"version":unsigned(),"code":{"type":"string","enum":["invalid_snapshot","conflicting_snapshot"]}}),&["id","version","code"]);
    s["OfflineStatsResult"]=object(json!({"accepted":array(reference("OfflineStatsAck")),"rejected":array(reference("OfflineStatsRejected")),"changed":boolean()}),&["accepted","rejected","changed"]);
    s["Download"] = object(
        json!({"id":string(),"track_id":string(),"status":{"type":"string","enum":["pending","downloading","complete","failed","interrupted"]},"size":unsigned(),"error":nullable(string()),"title":nullable(string()),"artist":nullable(string()),"format":nullable(string())}),
        &[
            "id", "track_id", "status", "size", "error", "title", "artist", "format",
        ],
    );
    s["Download"]["properties"]["source_audio"]=object(json!({"verified":boolean(),"format":string(),"sample_rate":unsigned(),"bit_depth":unsigned(),"channels":unsigned(),"bitrate":unsigned(),"duration":nullable(json!({"type":"number"}))}),&["verified","format","sample_rate","bit_depth","channels","bitrate"]);
    s["Download"]["properties"]["source_audio"]["description"]=json!("单项GET完成后对已批准下载文件的实际探测，不是播放源的更高权益声明；文件缺失或无效则失败。");
    s["DownloadRequest"] = object(json!({"track_id":string()}), &["track_id"]);
    s["DownloadStarted"] = object(
        json!({"id":string(),"track_id":string(),"status":string()}),
        &["id", "track_id", "status"],
    );
    s["ProviderManifest"] = object(
        json!({"id":string(),"name":string(),"version":string(),"api_version":{"type":"string","const":"1"},"core_api":string(),"runtime":string(),"entrypoint":string(),"license":string(),"upstream":string(),"capabilities":array(string()),"enabled":boolean(),"status":string(),"verified":boolean(),"error":nullable(string())}),
        &[
            "id",
            "name",
            "version",
            "api_version",
            "enabled",
            "status",
            "capabilities",
            "verified",
        ],
    );
    s["PluginToggleRequest"] = object(json!({"enabled":boolean()}), &["enabled"]);
    s["PluginToggleResult"] = object(
        json!({"id":string(),"enabled":boolean()}),
        &["id", "enabled"],
    );
    s["PluginInstalled"] = object(
        json!({"id":string(),"version":string(),"previous_version":nullable(string()),"status":{"type":"string","const":"installed"}}),
        &["id", "version", "previous_version", "status"],
    );
    s["ProviderHealth"] = object(
        json!({"status":string(),"authenticated":boolean(),"upstream_version":string(),"device_tested":boolean()}),
        &["status"],
    );
    s["ProviderLoginRequest"] = object(
        json!({"type":{"type":"string","enum":["qq","wx"],"default":"qq","description":"QQ 音乐扫码方式：qq 使用手机 QQ，wx 使用微信；省略请求体保持旧插件默认方式。"}}),
        &[],
    );
    s["ProviderLogin"] = object(
        json!({"session_id":string(),"login_type":{"type":"string","enum":["qq","wx","mobile"]},"status":{"type":"string","enum":["waiting","confirming","authenticated","expired","refused","error","cancelled"]},"qr_data_url":string(),"expires_at":timestamp,"error":nullable(string())}),
        &["session_id", "status"],
    );
    s["ProviderPlaylist"] = object(
        json!({"id":string(),"name":string(),"track_count":unsigned(),"cover_url":nullable(string()),"source":string(),"kind":{"type":"string","enum":["created","collected","liked"]}}),
        &["id", "name", "track_count", "cover_url", "source", "kind"],
    );
    s["ProviderPlaylist"]["description"] = json!("QQ 的 liked 是真实 get_fav_song（dirid=201）的入口；收藏外部歌单属于 collected，不冒充喜欢歌曲。");
    s["ProviderPlaylists"] = object(
        json!({"items":array(reference("ProviderPlaylist")),"next_page":nullable(unsigned())}),
        &["items", "next_page"],
    );
    s["ProviderPlaylistTracks"] = object(
        json!({"items":array(reference("Track")),"total":unsigned(),"playlist_id":string(),"next_page":nullable(unsigned())}),
        &["items", "total", "playlist_id", "next_page"],
    );
    s["PersonalPicks"] = object(
        json!({"date":nullable(json!({"type":"string","format":"date"})), "items":array(reference("Track")), "reasons":{"type":"object","additionalProperties":string()}}),
        &["date","items","reasons"],
    );
    s["SimilarTrackRequest"] = object(json!({"track_id":{"type":"string","minLength":1,"maxLength":256}}), &["track_id"]);
    s["RecommendationSelection"] = object(
        json!({"items":{"type":"array","maxItems":50,"items":reference("Track")},"source":{"type":"string","description":"实际来源：local、mixed 或连接器 ID"},"fallback":boolean(),"message":nullable(string()),"sections":{"type":"array","items":{"type":"object"}}}),
        &["items", "source", "fallback", "message", "sections"],
    );
    s["RecommendationSelection"]["description"] = json!("本地推荐使用真实曲库／播放统计；QQ使用真实猜你喜欢／雷达，公开上游不支持seed查询，种子只排除自身。授权或上游失败仅在 fallback_local=true 时回退本地，message说明原因。");
    s["ProviderSyncRequest"] = object(
        json!({"playlist_id":{"anyOf":[string(),unsigned()]},"collection":{"type":"string","enum":["favorites","all","created","collected"]},"page":{"type":"integer","minimum":1,"maximum":100000,"default":1}}),
        &[],
    );
    s["ProviderSync"] = object(
        json!({"items":array(reference("Track")),"playlists":array(reference("ProviderPlaylist")),"playlist_id":string(),"total":unsigned(),"next_page":nullable(unsigned())}),
        &["items"],
    );
    s["Settings"] = object(
        json!({"name":string(),"cache_limit_bytes":unsigned(),"metadata_enabled":boolean(),"low_performance":boolean(),"client_cache_default_bytes":unsigned(),"strict_sync_available":{"type":"boolean","const":false},"hqplayer_available":{"type":"boolean","description":"存在已注册且在线的 HQPlayer 网桥；不表示硬件已验证"}}),
        &[
            "name",
            "cache_limit_bytes",
            "metadata_enabled",
            "low_performance",
            "client_cache_default_bytes",
            "strict_sync_available",
            "hqplayer_available",
        ],
    );
    s["SettingsPatch"] = object(
        json!({"name":name,"cache_limit_bytes":{"type":"integer","minimum":0,"maximum":1125899906842624_u64},"metadata_enabled":boolean(),"low_performance":boolean()}),
        &[],
    );
    s["CacheRepairRequest"]=object(json!({"concurrency":{"type":"integer","minimum":1,"maximum":16}}),&["concurrency"]);
    s["CacheRepairStatus"]=object(json!({"enabled":boolean(),"job":{}}),&["enabled","job"]);
    s["Settings"]["properties"]["cache_enabled"]=json!({"type":"boolean","const":true});
    s["Settings"]["properties"]["cache_forced_off"]=boolean();
    s["SettingsPatch"]["properties"]["cache_enabled"]=json!({"type":"boolean","const":true});
    for schema in ["Settings","SettingsPatch"] {s[schema]["properties"]["mobile_opus_enabled"]=json!({"type":"boolean","default":false,"description":"允许客户端显式选择有损Opus，不改变原曲、下载权益或其他无损传输策略。"});}
    for schema in ["Settings","SettingsPatch"] {s[schema]["properties"]["statistics_timezone"]=json!({"type":"string","default":"Asia/Shanghai","maxLength":100,"description":"Core 全部客户端的统计归日和历史显示时区，使用有效 IANA 名称。"});}
    s["SettingsPatch"]["additionalProperties"] = json!(false);
    let stream_quality=json!({"type":"string","enum":["native","best","HIRES","MASTER","FLAC","MP3_320","MP3_128"],"default":"native","description":"QQ native 按 Hi-Res、FLAC、普通音质回退，排除增强版本；旧 best／MASTER 迁移为 native；HIRES 严格校验文件头，不可用时报错；其他连接器使用自身配置"});
    s["Settings"]["properties"]["stream_quality"]=stream_quality.clone();
    s["Settings"]["properties"]["metadata_provider"]=nullable(string());
    s["Settings"]["properties"]["metadata_provider_priority"]=json!({"type":"array","items":{"type":"string"},"maxItems":16,"uniqueItems":true});
    s["SettingsPatch"]["properties"]["metadata_provider_priority"]=json!({"type":"array","items":{"type":"string"},"maxItems":16,"uniqueItems":true});
    s["Settings"]["properties"]["metadata_providers"]=json!({"type":"array","items":{"type":"object"}});
    s["MediaProfile"]=object(json!({"profile":{"type":"object"},"manual":{"type":"object"},"revision":unsigned(),"updated_at":unsigned()}),&["profile","manual","revision"]);
    s["MediaProfileEdit"]=object(json!({"expected_revision":unsigned(),"fields":{"type":"object","additionalProperties":{"type":"string","maxLength":80000}},"unlock":{"type":"array","items":string()}}),&["expected_revision","fields"]);
    s["ArtistAlbums"]=object(json!({"artist_id":string(),"name":string(),"local":{"type":"object"},"platforms":array(json!({"type":"object"}))}),&["artist_id","name","local","platforms"]);
    s["CatalogDetail"]=json!({"type":"object","required":["id","kind","name","profile","local","platforms"],"properties":{"id":{"type":"string"},"kind":{"type":"string","enum":["artist","album"]},"name":{"type":"string"},"profile":{"type":"object"},"provenance":{"type":"object"},"local":{"$ref":"#/components/schemas/TrackPage"},"platforms":{"type":"array","items":{"type":"object"}},"anchor":{"$ref":"#/components/schemas/Track"}}});
    s["SettingsPatch"]["properties"]["stream_quality"]=stream_quality;
    let mut limits=json!({"type":"object","additionalProperties":false,"properties":{}});
    for (key,default) in [("search_ms",350),("resolve_ms",150),("lyrics_ms",350),("playlists_ms",600),("recommendations_ms",1000),("metadata_ms",1100),("comments_ms",1000)]{limits["properties"][key]=json!({"type":"integer","minimum":50,"maximum":60000,"default":default});}
    limits["properties"]["cooldown_seconds"]=json!({"type":"integer","minimum":1,"maximum":3600,"default":60});
    for schema in ["Settings","SettingsPatch"] {
        s[schema]["properties"]["artwork_limit_bytes"]=json!({"type":"integer","minimum":1,"maximum":1099511627776_u64,"default":536870912,"description":"独立永久图片预算；低于实际占用返回409，不自动删图"});
        s[schema]["properties"]["catalog_scrape_enabled"]=json!({"type":"boolean","default":false});
        s[schema]["properties"]["catalog_scrape_concurrency"]=json!({"type":"integer","minimum":1,"maximum":4,"default":1,"description":"后台资料刮削的共享并发，复用平台限流；不下载音频"});
        s[schema]["properties"]["provider_request_limits"]=limits.clone();
        s[schema]["properties"]["song_fetch_concurrency"]=json!({"type":"integer","minimum":1,"maximum":16,"default":2,"description":"同时拉取的歌曲数量，播放优先，同曲同质量共享名额"});
        s[schema]["properties"]["managed_memory_bytes"]=json!({"type":"integer","minimum":33554432,"maximum":137438953472_u64});
        for key in ["background_concurrency","analysis_threads"]{s[schema]["properties"][key]=json!({"type":"integer","minimum":1,"maximum":16});}
        s[schema]["properties"]["playback_cache_mode"]=json!({"type":"string","enum":["full","partial"],"default":"full"});
    }
    s["AudioInfo"]=object(json!({"track_id":string(),"source":string(),"format":nullable(string()),"sample_rate":nullable(unsigned()),"bit_depth":nullable(unsigned()),"channels":nullable(unsigned()),"bitrate":nullable(unsigned()),"verified":nullable(boolean()),"quality":nullable(string()),"quality_label":nullable(string()),"requested_quality":string()}),&["track_id","source","requested_quality"]);
    s["DevicePurge"]=object(json!({"confirmation":string()}),&["confirmation"]);
    s["TrackComments"]=object(json!({"supported":boolean(),"source":string(),"kind":{"type":"string","enum":["recommend","hot","new"]},"page":unsigned(),"page_size":unsigned(),"total":unsigned(),"has_more":boolean(),"next_cursor":string(),"fetched_at":unsigned(),"cached":boolean(),"stale":boolean(),"notice":string(),"items":array(object(json!({"id":string(),"author":string(),"text":string(),"published_at":unsigned(),"likes":unsigned(),"reply_count":unsigned(),"replies":array(object(json!({"id":string(),"author":string(),"text":string(),"published_at":unsigned(),"reply_to":string()}),&["id","author","text","published_at"]))}),&["id","author","text","published_at","likes","reply_count"]))}),&["supported","items"]);
    s["SearchResults"]=json!({"allOf":[reference("TrackPage"),{"type":"object","properties":{"artists":{"type":"array","items":{"type":"object"}},"albums":{"type":"array","items":{"type":"object"}},"catalog_scope":{"type":"string","enum":["matched_tracks"]},"notice":{"type":["string","null"]},"bounded":{"type":"boolean"}}}]});
    s["CatalogLinkRequest"]=object(json!({"resolve_credits":boolean(),"track_id":nullable(string()),"artist":string(),"album":string(),"album_artist":nullable(string()),"source":string(),"provider_data":{"type":"object"}}),&["artist","album"]);
    s["CatalogLinks"]=object(json!({"resolved":boolean(),"artists":array(object(json!({"id":string(),"name":string()}),&["id","name"])),"album":object(json!({"id":string(),"title":string()}),&["id","title"])}),&["artists","album"]);
    s["TrackBackground"]=object(json!({"matched":boolean(),"source":{"type":"string"},"mid":string(),"background":{"type":"object"},"artists":array(json!({"type":"object"})),"warnings":array(string()),"fetched_at":unsigned(),"url":string()}),&["matched","source"]);
    s["Saved"] = object(json!({"saved":{"type":"boolean","const":true}}), &["saved"]);
    let mut smart_field=string();smart_field["maxLength"]=json!(500);
    s["SmartPlaylistQuery"]=object(json!({"q":nullable(smart_field.clone()),"source":nullable(smart_field.clone()),"album_id":nullable(smart_field.clone()),"artist_id":nullable(smart_field)}),&[]);
    s["SmartPlaylistQuery"]["additionalProperties"]=json!(false);
    s["SmartPlaylistQuery"]["description"]=json!("沿用曲库搜索／音源／专辑／艺术家筛选，空对象匹配全部可用入库曲目；全部来源沿用同录音优先本地规则。字段上限为 UTF-8 500 字节。");
    s["Playlist"] = object(
        json!({"id":string(),"name":string(),"track_ids":ids,"alias_track_ids":array(string()),"smart_query":reference("SmartPlaylistQuery"),"truncated":boolean()}),
        &["id", "name", "track_ids"],
    );
    s["FavoriteChange"]=object(json!({"liked":boolean()}),&["liked"]);
    s["FavoritesBinding"]=object(json!({"provider":string(),"playlist_id":string(),"take_over":boolean()}),&["provider","playlist_id"]);
    s["FavoritesBinding"]["description"]=json!("将当前用户本地我喜欢绑定到一张支持 add/remove 的云端歌单。首次合并；已绑定其他副本返回409 binding_exists，显式 take_over 后保留原副本并转移。同步执行期间返回409 sync_busy。DELETE 仅解除绑定，保留两边内容。");
    s["Playlist"]["description"]=json!("列表中的智能歌单仅含规则，track_ids 为空；GET /playlists/{id} 按当前可用曲库解析最多 10000 首并报告 truncated。手动歌单保持保存的顺序与有意重复的位置。我喜欢的GET返回精确缓存来源去重后的track_ids，可选alias_track_ids包含源记录和已知缓存别名，供客户端标记收藏；旧客户端可忽略此新增字段。");
    s["PlaylistRequest"] = object(json!({"name":name,"track_ids":ids,"smart_query":reference("SmartPlaylistQuery")}), &["name"]);
    s["PlaylistRequest"]["oneOf"]=json!([{"required":["track_ids"],"not":{"required":["smart_query"]}},{"required":["smart_query"],"not":{"required":["track_ids"]}}]);
    s["PlaylistPatch"] = object(json!({"name":name,"track_ids":ids,"smart_query":reference("SmartPlaylistQuery")}), &[]);
    s["PlaylistPatch"]["properties"]["expected"] = object(json!({"name":string(),"track_ids":array(string())}), &["name","track_ids"]);
    s["PlaylistPatch"]["not"]=json!({"required":["track_ids","smart_query"]});
    s["PlaylistPatch"]["description"]=json!("名称可修改；手动歌单可改 track_ids，智能歌单可改 smart_query，不直接切换类型。操作仅限当前账号自己的歌单。可携带 expected 名称与完整曲序快照，在更新事务内比较；已变化返回409且不写入，防止覆盖其他控制端编辑。");
    s["IdResult"] = object(json!({"id":string()}), &["id"]);
    let color = json!({"type":"string","pattern":"^#[0-9a-fA-F]{6}$"});
    s["ThemeColors"] = object(
        json!({"bg":color,"surface":color,"raised":color,"text":color,"muted":color,"border":color,"accent":color,"accentText":color}),
        &[
            "bg",
            "surface",
            "raised",
            "text",
            "muted",
            "border",
            "accent",
            "accentText",
        ],
    );
    s["ThemeColors"]["additionalProperties"] = json!(false);
    s["ThemeManifest"] = object(
        json!({"schema_version":{"type":"integer","const":1},"id":{"type":"string","pattern":"^[-a-z0-9]{1,48}$"},"name":{"type":"string","maxLength":40},"light":reference("ThemeColors"),"dark":reference("ThemeColors")}),
        &["schema_version", "id", "name", "light", "dark"],
    );
    s["ThemeManifest"]["description"] =
        json!("已实现的颜色 JSON 主题格式；不含脚本、CSS 或静态资源主题包。");
    s["PlaybackModule"] = object(
        json!({"id":{"type":"string","enum":["cover","info","controls","lyrics","queue","zones","signal","meter","spectrum"]},"x":{"type":"number","minimum":0},"y":{"type":"number","minimum":0},"w":{"type":"number","minimum":10,"maximum":100},"h":{"type":"number","minimum":50,"maximum":2000},"visible":boolean()}),
        &["id", "x", "y", "w", "h", "visible"],
    );
    s["PlaybackLayout"] = object(
        json!({"version":{"type":"integer","const":1},"breakpoint":{"type":"string","enum":["desktop","tablet","phone"]},"modules":{"type":"array","maxItems":12,"items":reference("PlaybackModule")}}),
        &["version", "breakpoint", "modules"],
    );
    s["PlaybackLayout"]["description"] = json!("SDK 版本化布局交换模型；当前 WebUI preferences.layouts[breakpoint] 实际存 PlaybackModule[]，不要用交换对象替代该数组。");
    s["Preferences"] = object(
        json!({"theme":string(),"mode":{"type":"string","enum":["dark","light","system"]},"low_performance":boolean(),"custom_themes":array(reference("ThemeManifest")),"layouts":{"type":"object","properties":{"desktop":array(reference("PlaybackModule")),"tablet":array(reference("PlaybackModule")),"phone":array(reference("PlaybackModule"))},"additionalProperties":true},"lyric_font":number(),"lyric_offset":number(),"lyric_translation":boolean(),"client_cache_limit":unsigned()}),
        &[],
    );
    s["Preferences"]["description"] = json!("账号任意 JSON 对象，最多 1 MiB；PUT 替换全部。列出当前共享界面字段，主题／布局由客户端验证，核心保留扩展字段。");
    for (list, model, paged) in [
        ("UserList", "User", false),
        ("TokenList", "Token", false),
        ("SessionList", "Session", false),
        ("TrackPage", "Track", true),
        ("AlbumPage", "Album", true),
        ("ArtistPage", "Artist", true),
        ("RootList", "Root", false),
        ("JobList", "Job", true),
        ("ZoneList", "Zone", false),
        ("EndpointList", "Endpoint", false),
        ("DownloadList", "Download", false),
        ("PluginList", "ProviderManifest", false),
        ("PlaylistList", "Playlist", false),
    ] {
        s[list] = items(model, paged);
    }
    s["OutputRename"] = object(json!({"name":{"type":"string","minLength":1,"maxLength":100}}), &["name"]);
    s["DeviceVolumeWrite"] = object(json!({"value":{"type":"number","minimum":0,"maximum":1}}), &["value"]);
    s["ProviderSearch"] = items("Track", false);
    s["ProviderSearch"]["properties"]["total"] = unsigned();
    s["Event"] = object(
        json!({"type":{"type":"string","examples":["zones","zone_progress","library","jobs","plugins","cache","downloads","endpoints","stats","resync"]},"data":{}}),
        &["type", "data"],
    );
    for (draft, properties) in [
        ("Recording", json!({"id":string(),"title":string()})),
        (
            "SourceVariant",
            json!({"id":string(),"recording_id":string(),"source":string(),"format":string()}),
        ),
    ] {
        s[draft] = object(properties, &["id"]);
        s[draft]["x-rmusic-status"] = json!("draft");
        s[draft]["description"] =
            json!("规划草案；没有完整作品／录音／发行关系 API，也没有 REST 操作返回此模型。");
    }
    music::schemas(&mut s);
    accounts::schemas(&mut s);
    device_groups::schemas(&mut s);
    s
}
fn response(schema: &str) -> Value {
    json!({"description":"成功","content":{"application/json":{"schema":reference(schema)}}})
}
fn error_response(description: &str) -> Value {
    json!({"description":description,"content":{"application/json":{"schema":reference("Error")},"text/plain":{"schema":{"type":"string"}}}})
}
fn parameter(name: &str, location: &str, required: bool, schema: Value) -> Value {
    json!({"name":name,"in":location,"required":required,"schema":schema})
}
fn operation(
    paths: &mut Value,
    path: &str,
    method: &str,
    scope: &str,
    request: Option<(&str, bool)>,
    result: &str,
) {
    let full = format!("/api/v1{path}");
    let mut op = json!({"operationId":format!("{}_{}",method,path.trim_start_matches('/').replace('/',"_").replace(['{','}'],"")),"summary":format!("{} {path}",method.to_uppercase()),"tags":[path.split('/').nth(1).unwrap_or("core")],"x-rmusic-scope":scope,"responses":{
        "400":error_response("参数无效或提取失败"),"404":error_response("资源不存在"),"500":error_response("核心内部错误")
    }});
    if matches!(path,"/zones/{id}/play"|"/zones/{id}/pause"|"/zones/{id}/stop") {op["responses"]["409"]=error_response("带身份的系统控制与当前端点／曲目／播放实例不符");}
    if scope != "public" {
        op["security"] = json!([{"bearerAuth":[]}]);
        op["responses"]["401"] = error_response("凭证无效或过期");
        op["responses"]["403"] = error_response("权限不足");
    } else {
        op["security"] = json!([]);
    }
    if matches!(path,"/zones/{id}/handoff"|"/zones/{id}/recover") { op["responses"]["409"] = error_response("相同连续播放请求 ID 的内容冲突"); }
    if path=="/zones/{id}/recover" {op["responses"]["503"]=error_response("并发持久日志更新了源进度／EOF；原样重试恢复请求");}
    let mut params = Vec::new();
    if path=="/zones" && method=="get" {
        params.push(parameter("view","query",false,json!({"type":"string","enum":["state"],"description":"小型音区状态；队列按 queue_revision 从独立分页接口读取"})));
    }
    if path=="/zones/{id}/queue" && method=="get" {
        params.push(parameter("revision","query",false,unsigned()));
        params.push(parameter("epoch","query",false,string()));
        params.push(parameter("offset","query",false,unsigned()));
        params.push(parameter("limit","query",false,json!({"type":"integer","minimum":1,"maximum":500,"default":200})));
        op["responses"]["409"]=error_response("queue_revision_changed：翻页期间队列改变，重新获取状态后加载");
    }
    if path=="/cache"&&method=="get" {
        params.push(parameter("include_partial","query",false,boolean()));
        params.push(parameter("limit","query",false,json!({"type":"integer","minimum":1,"maximum":1000,"default":1000})));
        params.push(parameter("offset","query",false,json!({"type":"integer","minimum":0,"maximum":1000000,"default":0})));
    }
    if path=="/playlists/{id}"&&method=="put" {op["responses"]["409"]=error_response("playlist_edit_conflict：歌单与 expected 快照不同，保留用户编辑并读取最新内容");}
    for part in path.split('/').filter(|p| p.starts_with('{')) {
        params.push(parameter(
            part.trim_matches(['{', '}']),
            "path",
            true,
            string(),
        ));
    }
    if matches!(
        path,
        "/library/tracks" | "/library/albums" | "/library/artists"
    ) {
        params.push(parameter(
            "limit",
            "query",
            false,
            json!({"type":"integer","minimum":0,"default":50,"description":"核心夹到 1..200"}),
        ));
        params.push(parameter(
            "offset",
            "query",
            false,
            json!({"type":"integer","minimum":0,"default":0,"description":"核心夹到 10000000"}),
        ));
        for name in ["q", "source", "album_id", "artist_id"] {
            params.push(parameter(name, "query", false, string()));
        }
    }
    if matches!(path,"/providers/{id}/search"|"/search"|"/library/search") {
        params.push(parameter(
            "q",
            "query",
            true,
            json!({"type":"string","maxLength":500}),
        ));
    }
    if matches!(path, "/providers/{id}/playlists" | "/providers/{id}/playlists/{playlist_id}/tracks") {
        params.push(parameter("page", "query", false, json!({"type":"integer","minimum":1,"maximum":100000,"default":1})));
        if path == "/providers/{id}/playlists" {
            params.push(parameter("collection", "query", false, json!({"type":"string","enum":["all","created","collected"],"default":"all"})));
        }
        op["responses"]["502"] = error_response("音源未登录、上游不可用或返回的数据不属于该音源");
    }
    if matches!(path,"/library/tracks"|"/library/albums"|"/library/artists") {
        for key in ["sort","format","album_status","listening","artwork","playlist","released_after","released_before"] {params.push(parameter(key,"query",false,string()));}
        for key in ["min_rate","min_depth","min_seconds","max_seconds"] {params.push(parameter(key,"query",false,json!({"type":"integer","minimum":0})));}
    }
    if path=="/library/tracks/{id}/versions"{for name in ["offset","other_offset","limit"]{params.push(parameter(name,"query",false,unsigned()));}for name in ["q","source","sort"]{params.push(parameter(name,"query",false,string()));}params.push(parameter("available","query",false,boolean()));}
    if path=="/artwork-cache/events"{params.push(parameter("source","query",false,string()));}
    if path=="/library/catalog/{kind}/{id}/links"{params.push(parameter("q","query",false,string()));}
    if matches!(path,"/library/catalog/{kind}/{id}"|"/library/catalog/artist/{id}/albums"|"/library/tracks/{id}/related") {params.push(parameter("sort","query",false,string()));}
    if path=="/library/catalog/{kind}/{id}"{params.push(parameter("platform_sort","query",false,string()));params.push(parameter("client_metadata","query",false,boolean()));}
    if path=="/library/catalog/artist/{id}/albums"{params.push(parameter("local_sort","query",false,string()));}
    if path == "/library/catalog/{kind}/{id}" || path == "/library/catalog/artist/{id}/albums" {
        params.extend([parameter("page","query",false,json!({"type":"integer","minimum":1,"maximum":100000,"default":1})),parameter("local_page","query",false,json!({"type":"integer","minimum":1,"maximum":100000,"default":1})),parameter("provider","query",false,string()),parameter("remote","query",false,json!({"type":"boolean","default":true})),parameter("refresh","query",false,json!({"type":"boolean","default":false}))]);
    }
    if path == "/library/tracks/{id}/related" {
        params.extend([parameter("kind","query",false,json!({"type":"string","enum":["album","artist"],"default":"album"})),parameter("page","query",false,json!({"type":"integer","minimum":1,"maximum":100000,"default":1})),parameter("artist_mid","query",false,string())]);
        op["description"]=json!("按已确认歌曲的平台专辑／演唱者ID浏览，每页最多50首。无法确认版本时matched=false。不下载音频、不替换本地文件；仅登记返回页的云端元数据。现有本地副本须再次核对版本后才优先选择。");
    }
    if path == "/jobs" {
        params.extend([parameter("limit","query",false,json!({"type":"integer","minimum":1,"maximum":100,"default":10})),parameter("offset","query",false,unsigned()),parameter("status","query",false,json!({"type":"string","enum":["all","active","history"],"default":"all"})),parameter("id","query",false,string())]);
    }
    if matches!(path,"/native-pcm/{id}"|"/native-flac/{id}") {
        for (name,schema) in [("rate",json!({"type":"integer","minimum":8000,"maximum":768000})),("bits",json!({"type":"integer","enum":[16,24,32]})),("channels",json!({"type":"integer","minimum":1,"maximum":2}))] {params.push(parameter(name,"query",true,schema));}
        params.push(parameter("start","query",false,json!({"type":"number","minimum":0,"default":0})));
        op["responses"]["429"]=error_response("与网页 PCM 共用八路转换额度");
    }
    if matches!(path,"/native-pcm/{id}"|"/native-flac/{id}"|"/mobile-opus/{id}") {
        params.push(parameter("download_id","query",false,json!({"type":"string","description":"仅转换此已批准完成且track_id相同的下载文件，授权范围为download:{download_id}。缺失不回退到播放源。"})));
    }
    if path=="/mobile-opus/{id}" {
        params.extend([parameter("bitrate","query",false,json!({"type":"integer","minimum":48,"maximum":320,"default":160})),parameter("channels","query",false,json!({"type":"integer","minimum":1,"maximum":2,"default":2})),parameter("start","query",false,json!({"type":"number","minimum":0,"default":0})),parameter("accept_lossy","query",true,json!({"type":"boolean","const":true}))]);
        op["description"]=json!("可选移动端有损Opus省流；须Core管理员启用mobile_opus_enabled并逐请求accept_lossy=true。Ogg/Opus解码时钟48kHz，不提供虚构位深；标头X-RMusic-Mobile-Opus声明目标码率。八路共享额度、有界流、不落盘，取消即释放。");
        op["responses"]["403"]=error_response("Core未启用移动端Opus");
        op["responses"]["429"]=error_response("共享音频转换额度已满");
    }
    if path == "/settings" && method == "patch" {op["responses"]["409"]=error_response("cache_budget_below_usage：预算低于当前占用；先手动清理缓存，回收站也计入占用");}
    if path == "/recommendations" {
        params.extend([
            parameter("source", "query", false, json!({"type":"string","description":"local、mixed 或已启用连接器 ID","default":"mixed"})),
            parameter("seed_track_id", "query", false, json!({"type":"string","maxLength":256})),
            parameter("limit", "query", false, json!({"type":"integer","minimum":1,"maximum":50,"default":20})),
            parameter("fallback_local", "query", false, json!({"type":"boolean","default":true})),
            parameter("refresh", "query", false, json!({"type":"boolean","default":false,"description":"更新内容；QQ 同日复用每日快照"})),
        ]);
    }
    if path == "/stats/history" {params.push(parameter("limit","query",false,json!({"type":"integer","minimum":1,"maximum":100,"default":50})));params.push(parameter("cursor","query",false,json!({"type":"string","maxLength":210,"description":"上一页返回的next_cursor，稳定倒序分页"})));}
    if path == "/stats" {
        params.push(parameter(
            "period_days",
            "query",
            false,
            json!({"type":"integer","minimum":0,"maximum":3650,"default":30,"description":"0 表示历史所有；其他值为最近天数"}),
        ));
        params.push(parameter("timezone","query",false,json!({"type":"string","description":"省略时跟随 Core 的 statistics_timezone，默认 Asia/Shanghai；可显式指定 IANA 时区"})));
    }
    if matches!(
        result,
        "media" | "cover" | "aac" | "pcm" | "native_pcm" | "native_flac" | "mobile_opus" | "events" | "download" | "portable" | "impulse"
    ) {
        op["security"] = json!([{"bearerAuth":[]},{"ticketAuth":[]}]);
        params.push(parameter(
            "ticket",
            "query",
            false,
            json!({"type":"string","description":"限定资源票据；主会话只能用 Authorization 头"}),
        ));
    }
    if matches!(result, "media" | "download" | "portable") {
        for header in ["Range", "If-Range", "If-None-Match"] {
            if result == "portable" && header != "Range" || result == "download" && header == "If-None-Match" {
                continue;
            }
            params.push(parameter(header, "header", false, string()));
        }
    }
    if result == "portable" {
        params.push(parameter(
            "asset",
            "query",
            false,
            json!({"type":"string","enum":["audio","metadata","lyrics","cover"],"default":"audio"}),
        ));
        op["responses"]["409"] = error_response("便携副本尚未生成；后台空闲时处理");
    }
    if matches!(path,"/transcode/{id}"|"/pcm/{id}") {
        if path=="/pcm/{id}"{params.push(parameter("rate","query",true,json!({"type":"integer","minimum":8000,"maximum":192000,"description":"AudioContext 的实际输出采样率；响应为立体声交错 f32le，不含文件头"})));}
        else{params.push(parameter("bitrate","query",false,json!({"type":"integer","minimum":128,"maximum":256,"default":128,"description":"AAC-LC kbps"})));params.push(parameter("pcm_rate","query",false,json!({"type":"integer","minimum":8000,"maximum":192000,"description":"可选连续 AAC 解码输出时钟；分片 MP4 保留首尾裁切信息，与 PCM 共用八路额度"})));}
        params.push(parameter(
            "start",
            "query",
            false,
            json!({"type":"number","minimum":0,"default":0,"description":"起始秒数"}),
        ));
        op["responses"]["200"]["headers"]["X-RMusic-Start"]=json!({"description":"实际请求的起始秒数","schema":{"type":"number","minimum":0}});
        if path=="/transcode/{id}" {
            op["responses"]["200"]["headers"]["X-RMusic-Continuous"]=json!({"description":"仅 pcm_rate 模式返回 aac-mp4-v1; output_rate={pcm_rate}；客户端须按 MP4 edit 与包时长裁切编码延迟及尾填充","schema":{"type":"string"}});
        }
        op["responses"]["429"] = error_response(if path=="/pcm/{id}"{"最多同时八路 PCM／连续 AAC 解码，供四音区当前曲及预载使用"}else{"普通 AAC 最多四路；pcm_rate 模式与 PCM 共用八路额度"});
        op["responses"]["502"] = error_response("音源解码或转换失败");
        op["responses"]["503"] = error_response("FFmpeg 不可用");
    }
    if let Some((schema, required)) = request {
        op["requestBody"] = json!({"required":required,"content":{"application/json":{"schema":reference(schema)}}});
        op["responses"]["422"] = error_response("JSON 类型／必填项错误；Axum 可能返回 text/plain");
    }
    if path=="/library/catalog/{kind}/{id}/public-fetch" {
        if method=="get"{params.push(parameter("refresh","query",false,boolean()));}
        if method=="post"{op["requestBody"]=json!({"required":true,"content":{"application/json":{"schema":object(json!({"id":string(),"epoch":unsigned(),"revision":string(),"expires_at":unsigned(),"response":reference("AnyObject")}),&["id","epoch","revision","expires_at","response"])}}});}
        op["description"]=json!("按已登记的平台ID进行匿名资料请求／回填；Core永久缓存，人工字段优先；当前支持QQ，未声明public_metadata_v1则404。响应最多1 MiB，资料内容和插件修订变化时不覆盖。");
    }
    if path=="/artwork/{id}/public-fetch"&&method=="put" {
        params.push(parameter("x-rmusic-source-revision","header",true,string()));params.push(parameter("x-rmusic-cached-content","header",false,string()));
        op["requestBody"]=json!({"required":true,"content":{"application/octet-stream":{"schema":{"type":"string","format":"binary"}}},"description":"缺失的已登记QQ公开图片，最多20 MiB；已有有效Core文件时去重返回，来源修订变化时拒绝。"});
        op["responses"]["413"]=error_response("图片超过20 MiB");
    }
    if path == "/plugins/install" {
        params.push(parameter("install_dependencies","query",false,json!({"type":"boolean","default":false,"description":"明确允许 venv/PyPI 固定依赖下载，插件及依赖为可信可执行代码"})));
        op["requestBody"] = json!({"required":true,"content":{"application/zip":{"schema":{"type":"string","format":"binary"}}},"description":"ZIP 原始字节，最大 16 MiB；根目录 manifest.json + provider.py"});
        op["responses"]["413"] = error_response("插件包超过 16 MiB");
    }
    if path=="/dsp/impulses"&&method=="post" {
        params.push(parameter("name","query",false,json!({"type":"string","minLength":1,"maxLength":120,"description":"显示名称；不作为文件路径"})));
        op["requestBody"]=json!({"required":true,"content":{"audio/wav":{"schema":{"type":"string","format":"binary"}},"application/octet-stream":{"schema":{"type":"string","format":"binary"}}},"description":"最多 4 MiB 的原始 RIFF/WAVE 字节；PCM 8/16/24/32、浮点 32/64 位，含识别的 extensible 子格式；不归一化／重采样／截短。输入系数总计最多 262144。只导入响应，不启用播放器卷积。"});
        op["responses"]["413"]=error_response("超过 4 MiB；Axum 可能返回 text/plain");
        op["responses"]["409"]=error_response("响应库超过 128 个文件或 256 MiB；同字节导入幂等");
    }
    if path=="/dsp/impulses/{id}"&&method=="delete" {op["responses"]["409"]=error_response("响应仍由某音区配置引用，包括关闭的 DSP");}
    if result=="impulse" {
        params.push(parameter("zone_id","query",false,string()));params.push(parameter("selection","query",false,json!({"type":"string","pattern":"^[0-9a-f]{64}$"})));
        op["description"]=json!("read bearer 可直接读取；票据读取须同时提供 ticket、zone_id、selection。票据绑定当前响应组和签发会话，300 秒有效；更换响应组、删除音区或撤销会话后失效。");
    }
    if path=="/zones/{id}/dsp/impulses" {op["responses"]["409"]=error_response("请求快照与音区当前响应组不一致；重新读取设置，不得替换为其他滤波器");}
    if path == "/auth/login" {
        op["responses"]["401"] = error_response("账号、密码、TOTP 或恢复码无效");
        op["responses"]["429"] = error_response("登录限速，5 分钟后重试");
    }
    if path == "/setup" {
        op["responses"]["403"] = error_response("一次性初始化凭证无效");
    }
    if path=="/library/tracks/{id}/comments" {
        params.push(parameter("kind","query",false,json!({"type":"string","enum":["recommend","hot","new"],"default":"recommend"})));
        params.push(parameter("page","query",false,json!({"type":"integer","minimum":1,"maximum":1000,"default":1})));
        params.push(parameter("page_size","query",false,json!({"type":"integer","minimum":1,"maximum":50,"default":20})));
        params.push(parameter("cursor","query",false,json!({"type":"string","pattern":"^[0-9]*$","maxLength":128})));
    }
    if matches!(path,"/library/tracks/{id}/comments"|"/library/tracks/{id}/background"){params.push(parameter("refresh","query",false,json!({"type":"boolean","default":false,"description":"绕过内容缓存，仍遵守接口限流；失败保留已有内容"})));}
    if path=="/filesystem/directories"&&method=="get" {for (name,schema) in [("path",string()),("offset",unsigned())]{params.push(parameter(name,"query",false,schema));}}
    if path=="/library/files"||path=="/library/files/bin" {for name in ["limit","offset"]{params.push(parameter(name,"query",false,unsigned()));}if path=="/library/files"{params.push(parameter("sort","query",false,json!({"type":"string","enum":["title","play_count_asc","play_count_desc","size_desc"]})));params.push(parameter("max_play_count","query",false,unsigned()));params.push(parameter("unlisted","query",false,json!({"type":"boolean","description":"仅缓存；排除全部用户播放列表及收藏"})));for name in ["storage","availability","q","album_id","artist_id","folder"]{params.push(parameter(name,"query",false,string()));}}}
    match result {
        "playlist_file" => {params.push(parameter("format","query",false,json!({"type":"string","enum":["json","m3u8","pls","csv"],"default":"json"})));op["responses"]["200"]=json!({"description":"播放列表文件，仅曲目引用，无临时URL或凭据；智能歌单为当前结果快照","content":{"application/json":{"schema":{"type":"object"}},"text/plain":{"schema":{"type":"string"}}}});},
        "backup_archive" => {op["responses"]["200"]=json!({"description":"含私密登录状态的备份，仅管理员 Bearer 可导出，Cache-Control: no-store；无公开票据","content":{"application/zip":{"schema":{"type":"string","format":"binary"}}}});},
        "204" => op["responses"]["204"] = json!({"description":"成功，无响应体"}),
        "media" | "download" | "cover" | "artwork" | "aac" | "pcm" | "native_pcm" | "native_flac" | "mobile_opus" | "portable" | "impulse" => {
            binary_responses(&mut op, result, method)
        }
        "events" | "bridge" => {
            op["responses"]["101"] =
                json!({"description":"WebSocket 协议升级；消息格式见 docs/API.md"});
            op["x-rmusic-websocket"] = json!(true);
            if result == "events" {
                op["x-rmusic-event-schema"] = reference("Event");
            }
        }
        other => op["responses"]["200"] = response(other),
    }
    if path == "/library/tracks/{id}/analyze" {
        op["responses"]["501"] = error_response("首版仅支持本地文件响度分析");
    }
    if path=="/sync/starts"&&method=="get" {
        params.push(parameter("active","query",false,json!({"type":"boolean","default":false})));
        params.push(parameter("recoverable","query",false,json!({"type":"boolean","default":false,"description":"active=true 时同时列出原听音账号仍持有恢复意图的中断分组；最终帧可能尚未收齐"})));
        params.push(parameter("limit","query",false,json!({"type":"integer","minimum":1,"maximum":100,"default":50})));
        params.push(parameter("offset","query",false,json!({"type":"integer","minimum":0,"maximum":100000,"default":0})));
    }
    if !params.is_empty() {
        op["parameters"] = json!(params.clone());
    }
    if path=="/output-groups/{id}" {
        op["responses"]["409"]=error_response("组合版本已变化或已删除，请刷新选择");
        if method=="delete"{params.push(parameter("expected_revision","query",true,json!({"type":"integer","minimum":1})));}
        if method=="put"{op["responses"]["201"]=response("OutputGroup");}
        op["description"]=json!("当前 Core 共享的常用音区选择，不保存曲目或启动播放。创建使用新 UUID 和 expected_revision=0；编辑使用读取到的版本。相同内容的本次重试返回原版本；并发修改或删除返回 409。删除保留墓碑，迟到旧请求不能复活；保存与删除不改变活动播放。");
    }
    if path=="/sync/starts"&&method=="post"{op["responses"]["202"]=response("SyncStart");op["description"]=json!("二至四个已停止、独立原生端点的 PCM 准备／统一提交，可包含有序连续组队列。首次 202；同请求 UUID 和相同正文重放返回 200 原结果，Core 重启不自动恢复。保留原始格式，各端最终帧按整组曲序合并；不是严格设备同步。");}
    if path=="/sync/starts/{id}/control"{op["responses"]["202"]=response("SyncStart");op["description"]=json!("以新 UUID 暂停、继续或定位当前曲序；group_queue_jump_v1 另允许 jump，须给出 from_queue_index（当前已确认曲序）、queue_index（不同的目标曲序）和 position。旧曲序操作拒绝，等待所有旧输出最终帧后才换曲；暂停中换曲仍暂停，播放中继续。返回已听曲序开始新的听音播放，同曲 seek／暂停续播沿用原听音会话，跳过部分不计时。相同 UUID 和正文重试只返回状态。members 接受当前 from_zone_ids 与目标 zone_ids（一至四个独立输出），成员变化时拒绝旧请求；加入输出须空闲且支持当前格式／队列与有效时钟。等待旧组全部最终帧，事务更新成员后从确认位置共同重新缓冲；暂停组仍暂停，只留一个输出也保留组控制。改组不新计次，成员断线／普通控制会撤销待改组。六秒内缺最终确认则失败，不启动新输出；Core 重启不重放，真实设备与严格同步仍需另验。");}
    if path=="/sync/starts/{id}/final-receipts"{op["responses"]["409"]=error_response("原纪元仍在在线输出或收尾；或最终回执身份／正文冲突");op["description"]=json!("仅 group_output_final_v2 核心接收原生持久最终回执。按原纪元同曲端点最大输出时长的增量结算，重复 applied=false；授权、原始时钟预算、连续源顺序与位置／帧包络校验后，源事实、统计与确认在同一事务提交。不自动恢复整组。");}
    if path=="/sync/starts/{id}/rate"{op["responses"]["202"]=response("SyncStart");op["responses"]["409"]=error_response("组未显式开启实验变速、当前输出实例／连接／起播状态已变化、能力不足或请求 UUID 冲突");op["description"]=json!("实验手动 PCM 目标，仅 group_rate_adjustment_v1 核心；当前显式启用的 v2 组在 playing 且端点 Released 时接受 ±500 ppm。请求绑定原起播纪元；相同 UUID 和正文重试200只查询，不重新发送。首次202只代表目标已持久接收，实际换算看 programme_clock v2。重准备归零，重开不重放；此接口只用于手动组；自动组由独立实验控制器管理，严格同步仍关闭。");}
    if path=="/extensions/{id}/invoke/{method}"{op["description"]=json!("扩展RPC v1；需要清单中该方法声明的read/library/control/admin权限。输入和输出按受限闭合schema校验；最长30秒，64KiB；不向插件传Core会话凭证，不自动执行插件建议。");}
    if path=="/library/tracks/{id}/fingerprint"{op["description"]=json!("仅本地文件／已有完整缓存的Chromaprint证据；POST进入可取消任务，不下载音频、不远程上传。相似候选不自动合并，也不证明发行／母带一致。");}
    if path=="/sync/starts/{id}/recovery"{op["responses"]["409"]=error_response("检查点或播放意图已变化、旧回执不齐、输出账号／能力／校准／占用不满足");if method=="post"{op["responses"]["202"]=response("SyncStart");}op["description"]=json!("仅 group_manual_recovery_v2 核心。GET 查询不启动输出，available=false 给出原因；POST 为显式手动恢复，首次202，相同 UUID／检查点重试200查询当前组，改正文409。仅当前 v2 私有恢复记录，旧版本缺少检查点时要求重新起播。继续位置沿用组内最前已确认位置，已应用的暂停定位保留，未完成控制不重放；停止、音区修改或放弃后不可复活。");}
    paths[full][method] = op;
}
fn binary_responses(op: &mut Value, kind: &str, method: &str) {
    let formats = if matches!(kind,"cover"|"artwork") {
        vec!["image/png", "image/jpeg", "image/webp"]
    } else if kind == "aac" {
        vec!["audio/mp4"]
    } else if matches!(kind,"native_flac"|"mobile_opus") {
        vec!["audio/ogg"]
    } else if matches!(kind,"impulse"|"native_pcm") {
        vec!["audio/wav"]
    } else if kind == "pcm" {
        vec!["application/octet-stream"]
    } else {
        vec![
            "audio/flac",
            "audio/mpeg",
            "audio/mp4",
            "audio/ogg",
            "audio/wav",
            "application/octet-stream",
        ]
    };
    let mut content = json!({});
    for format in formats {
        content[format] = json!({"schema":{"type":"string","format":"binary"}});
    }
    if kind == "portable" {
        for mime in ["image/png", "image/jpeg", "image/webp", "text/plain"] {
            content[mime] = json!({"schema":{"type":"string","format":"binary"}});
        }
        content["application/json"] =
            json!({"schema":{"type":"object","additionalProperties":true}});
    }
    let mut success = json!({"description":"成功；HEAD 只返回头部","headers":{"Content-Type":{"schema":{"type":"string"}}}});
    if kind=="native_flac" {success["headers"]["X-RMusic-Native-FLAC"]=json!({"schema":{"type":"string"},"description":"FLAC 实际编码规格，转换数据不落盘；精度转换并非相对原音源无损"});}
    if kind=="mobile_opus" {success["headers"]["X-RMusic-Mobile-Opus"]=json!({"schema":{"type":"string"},"description":"v1;rate=48000;channels=...;target_bitrate=...;lossless=false；编码器时钟，不声明音源位深"});}
    if kind=="native_pcm" {success["headers"]["X-RMusic-Native-PCM"]=json!({"description":"已确认的 v1;rate=...;bits=...;channels=...，响应为整数 PCM WAV","schema":{"type":"string"}});}
    if matches!(kind,"cover"|"artwork"){
        for header in ["Content-Length","ETag","Cache-Control","Vary","X-RMusic-Artwork-Version"]{success["headers"][header]=json!({"schema":{"type":"string"}});}
        if !op["parameters"].is_array(){op["parameters"]=json!([]);}
        op["parameters"].as_array_mut().unwrap().push(parameter("If-None-Match","header",false,string()));
        op["responses"]["304"]=json!({"description":"当前可读图片版本未改变；无正文"});
        op["responses"]["307"]=json!({"description":"旧文件丢失且来源已换图，临时转向新内容版本；保留原票据授权范围"});
        op["description"]=json!("原图／已保存展示图按需读取，无控件专属尺寸。artwork为稳定来源入口（短缓存、ETag重验），covers为不可变文件版本（长缓存）。文件校验后才可304；缺失或损坏按已确认来源修复，不预加载曲库。Android等客户端可原样缓存小图并本地缩放。刷新仅对artwork提供refresh=true，限流仍生效。");
    }
    if kind=="impulse" {for header in ["Content-Length","ETag","Cache-Control"] {success["headers"][header]=json!({"schema":{"type":"string"}});}}
    if method != "head" {
        success["content"] = content;
    }
    if matches!(kind, "media" | "download" | "portable") {
        for header in ["Content-Length", "Accept-Ranges"] {
            success["headers"][header] = json!({"schema":{"type":"string"}});
        }
        if kind == "media" {
            success["headers"]["ETag"] = json!({"schema":{"type":"string"}});
        }
        if matches!(kind, "download" | "portable") {
            success["headers"]["Content-Disposition"] = json!({"schema":{"type":"string"}});
        }
        op["responses"]["200"] = success.clone();
        success["description"] = json!("单字节区间，206 Partial Content");
        success["headers"]["Content-Range"] =
            json!({"schema":{"type":"string"},"description":"bytes start-end/total"});
        op["responses"]["206"] = success;
        op["responses"]["416"] = json!({"description":"区间无效，无音频体","headers":{"Content-Range":{"schema":{"type":"string"},"description":"bytes */total"}}});
        if kind == "portable" {
            op["responses"]["416"] = error_response("无效的文件字节区间");
        }
        if kind == "media" {
            op["responses"]["304"] = json!({"description":"本地文件 ETag 未变化，无响应体"});
            op["responses"]["502"] = error_response("音源连接或解析失败");
        }
    } else {
        if matches!(kind,"aac"|"pcm") {
            for header in [if kind=="pcm"{"X-RMusic-PCM"}else{"X-RMusic-Transcoded"}, "X-RMusic-Source", "X-RMusic-Start"] {
                success["headers"][header] = json!({"schema":{"type":"string"}});
            }
        }
        op["responses"]["200"] = success;
    }
}
fn build_document() -> Value {
    let mut paths = json!({});
    for (path, method, scope, request, result) in [
        ("/status", "get", "public", None, "Status"),
        ("/dsp/equalizer/import", "post", "control", Some(("EqualizerImport",true)), "EqualizerPreset"),
        ("/dsp/impulses", "get", "read", None, "ImpulseAssets"),
        ("/dsp/impulses", "post", "control", None, "ImpulseAsset"),
        ("/dsp/impulses/{id}", "get", "read", None, "ImpulseAsset"),
        ("/dsp/impulses/{id}", "delete", "control", None, "204"),
        ("/dsp/impulses/{id}/file", "get", "read", None, "impulse"),
        ("/dsp/impulses/{id}/file", "head", "read", None, "impulse"),
        ("/zones/{id}/dsp/impulses","post","control",Some(("ConvolutionConfig",true)),"ImpulseManifest"),
        ("/discovery", "get", "read", None, "Discovery"),
        ("/openapi.json", "get", "public", None, "OpenApiDocument"),
        (
            "/setup",
            "post",
            "public",
            Some(("SetupRequest", true)),
            "AuthResult",
        ),
        (
            "/auth/login",
            "post",
            "public",
            Some(("LoginRequest", true)),
            "AuthResult",
        ),
        ("/auth/me", "get", "authenticated", None, "User"),
        ("/auth/logout", "post", "authenticated", None, "204"),
        ("/auth/account", "patch", "interactive", Some(("AccountUpdateRequest", true)), "AccountUpdated"),
        ("/auth/totp/disable", "post", "interactive", Some(("ReauthenticateRequest", true)), "TotpDisabled"),
        ("/auth/totp/start", "post", "interactive", Some(("ReauthenticateRequest", true)), "TotpStart"),
        (
            "/auth/totp/confirm",
            "post",
            "interactive",
            Some(("TotpConfirmRequest", true)),
            "RecoveryCodes",
        ),
        (
            "/auth/tokens",
            "get",
            "interactive-or-admin",
            None,
            "TokenList",
        ),
        (
            "/auth/tokens",
            "post",
            "admin",
            Some(("TokenRequest", true)),
            "TokenCreated",
        ),
        (
            "/auth/tokens/{id}",
            "delete",
            "interactive-or-admin",
            None,
            "204",
        ),
        ("/auth/users", "get", "admin", None, "UserList"),
        (
            "/auth/users",
            "post",
            "admin",
            Some(("NewUserRequest", true)),
            "User",
        ),
        (
            "/auth/sessions",
            "get",
            "interactive-or-admin",
            None,
            "SessionList",
        ),
        (
            "/auth/sessions/{id}",
            "delete",
            "interactive-or-admin",
            None,
            "204",
        ),
        ("/library/tracks", "get", "read", None, "TrackPage"),
        ("/library/tracks/{id}", "get", "read", None, "Track"),
        ("/search","get","read",None,"SearchResults"),
        ("/library/search","get","read",None,"SearchResults"),
        ("/library/tracks/{id}/audio-info","get","read",None,"AudioInfo"),
        ("/library/tracks/{id}/background","get","read",None,"TrackBackground"),
        ("/library/tracks/{id}/related","get","read",None,"AnyObject"),
        ("/library/tracks/{id}/identity","get","read",None,"AnyObject"),
        ("/library/tracks/{id}/identity","put","library",Some(("MusicIdentityEdit",true)),"AnyObject"),
        ("/artwork-cache/events","get","admin",None,"AnyObject"),
        ("/library/tracks/{id}/versions/policy","get","read",None,"AnyObject"),
        ("/library/tracks/{id}/versions/policy","put","control",Some(("MusicPlaybackPolicyEdit",true)),"AnyObject"),
        ("/library/catalog/{kind}/{id}/links","get","read",None,"AnyObject"),
        ("/library/catalog/{kind}/{id}/links","put","library",Some(("CatalogAliasEdit",true)),"AnyObject"),
        ("/artwork-cache","get","admin",None,"ArtworkCacheStatus"),
        ("/artwork-cache/check","post","admin",None,"AnyObject"),
        ("/artwork-cache/refresh","post","admin",Some(("ArtworkRefreshRequest",true)),"AnyObject"),
        ("/library/tracks/{id}/identity/preview","post","library",Some(("MusicIdentityEdit",true)),"IdentityPreview"),
        ("/library/tracks/{id}/identity/restore","post","library",Some(("IdentityRestoreRequest",true)),"AnyObject"),
        ("/library/tracks/{id}/identity/history","get","library",None,"AnyObject"),
        ("/library/tracks/{id}/fingerprint","get","read",None,"AnyObject"),
        ("/library/tracks/{id}/fingerprint","post","library",None,"AnyObject"),
        ("/music/entities/{id}","get","read",None,"AnyObject"),
        ("/music/index","post","library",None,"AnyObject"),
        ("/extensions","get","read",None,"AnyObject"),
        ("/extensions/{id}/invoke/{method}","post","read",Some(("AnyObject",true)),"AnyObject"),
        ("/plugins/{id}/configuration","get","admin",None,"AnyObject"),
        ("/plugins/{id}/configuration","put","admin",Some(("PluginConfigurationEdit",true)),"AnyObject"),
        ("/library/tracks/{id}/comments","get","read",None,"TrackComments"),
        ("/devices/{id}/purge","post","admin",Some(("DevicePurge",false)),"AnyObject"),
        (
            "/library/tracks/{id}/lyrics",
            "get",
            "read",
            None,
            "LyricDocument",
        ),
        (
            "/library/tracks/{id}/ticket",
            "get",
            "read",
            None,
            "MediaTicket",
        ),
        (
            "/library/tracks/{id}/analyze",
            "post",
            "library",
            None,
            "JobStarted",
        ),
        ("/library/albums", "get", "read", None, "AlbumPage"),
        ("/library/artists", "get", "read", None, "ArtistPage"),
        ("/library/catalog/{kind}/{id}", "get", "read", None, "CatalogDetail"),
        ("/library/links", "post", "read", Some(("CatalogLinkRequest",true)), "CatalogLinks"),
        ("/library/roots", "get", "library", None, "RootList"),
        (
            "/library/roots",
            "post",
            "library",
            Some(("RootRequest", true)),
            "Root",
        ),
        ("/library/roots/{id}", "delete", "library", None, "204"),
        ("/library/scan", "post", "library", None, "JobStarted"),
        ("/outputs/{id}","patch","control",Some(("OutputRename",true)),"Saved"),
        ("/outputs/{id}/volume","put","control",Some(("DeviceVolumeWrite",true)),"AnyObject"),
        ("/jobs", "get", "read", None, "JobList"),
        ("/library/cache-repair","get","read",None,"CacheRepairStatus"),
        ("/library/cache-repair","post","admin",Some(("CacheRepairRequest",true)),"JobStarted"),
        ("/library/cache-repair/{id}","patch","admin",Some(("CacheRepairRequest",true)),"Saved"),
        (
            "/jobs/{id}",
            "patch",
            "library",
            Some(("JobActionRequest", true)),
            "204",
        ),
        ("/media/{id}", "get", "read", None, "media"),
        ("/media/{id}", "head", "read", None, "media"),
        ("/artwork/{id}", "get", "read", None, "artwork"),
        ("/artwork/{id}/public-fetch","get","library",None,"AnyObject"),
        ("/artwork/{id}/public-fetch","put","library",None,"AnyObject"),
        ("/library/catalog/{kind}/{id}/public-fetch","get","library",None,"AnyObject"),
        ("/library/catalog/{kind}/{id}/public-fetch","post","library",Some(("AnyObject",true)),"AnyObject"),
        ("/library/catalog/{kind}/{id}/profile", "get", "read", None, "MediaProfile"),
        ("/library/catalog/artist/{id}/albums", "get", "read", None, "ArtistAlbums"),
        ("/library/catalog/{kind}/{id}/profile", "patch", "library", Some(("MediaProfileEdit",true)), "MediaProfile"),
        ("/covers/{id}", "get", "read", None, "cover"),
        ("/covers/{id}", "head", "read", None, "cover"),
        ("/transcode/{id}", "get", "read", None, "aac"),
        ("/pcm/{id}", "get", "read", None, "pcm"),
        ("/native-pcm/{id}", "get", "read", None, "native_pcm"),
        ("/native-flac/{id}", "get", "read", None, "native_flac"),
        ("/mobile-opus/{id}", "get", "read", None, "mobile_opus"),
        ("/sync/starts", "get", "read", None, "SyncStartList"),
        ("/output-groups", "get", "read", None, "OutputGroupList"),
        ("/output-groups/{id}", "put", "control", Some(("OutputGroupSave", true)), "OutputGroup"),
        ("/output-groups/{id}", "delete", "control", None, "204"),
        ("/sync/starts", "post", "control", Some(("SyncStartRequest", true)), "SyncStart"),
        ("/sync/starts/{id}", "get", "read", None, "SyncStart"),
        ("/sync/starts/{id}/cancel", "post", "control", None, "SyncStart"),
        ("/sync/starts/{id}/control", "post", "control", Some(("SyncControlRequest", true)), "SyncStart"),
        ("/sync/starts/{id}/rate", "post", "control", Some(("GroupRateRequest", true)), "SyncStart"),
        ("/sync/starts/{id}/final-receipts", "post", "control", Some(("GroupFinalReceipt", true)), "GroupFinalAck"),
        ("/sync/starts/{id}/recovery", "get", "control", None, "GroupRecoveryState"),
        ("/sync/starts/{id}/recovery", "post", "control", Some(("GroupRecoveryRequest", true)), "SyncStart"),
        ("/zones", "get", "read", None, "ZoneList"),
        ("/zones/{id}/queue", "get", "read", None, "ZoneQueuePage"),
        (
            "/zones",
            "post",
            "control",
            Some(("NewZoneRequest", true)),
            "Zone",
        ),
        ("/zones/{id}", "get", "read", None, "Zone"),
        (
            "/zones/{id}",
            "patch",
            "control",
            Some(("ZonePatch", true)),
            "Zone",
        ),
        ("/zones/{id}", "delete", "control", None, "204"),
        (
            "/zones/{id}/queue",
            "put",
            "control",
            Some(("QueuePutRequest", true)),
            "Zone",
        ),
        (
            "/zones/{id}/queue",
            "post",
            "control",
            Some(("QueuePostRequest", true)),
            "Zone",
        ),
        ("/zones/{id}/visualization", "post", "read", Some(("VisualizationRequest", true)), "VisualizationReport"),
        ("/zones/{id}/queue", "delete", "control", None, "Zone"),
        ("/zones/{id}/queue/reorder", "post", "control", Some(("QueueReorderRequest", true)), "Zone"),
        (
            "/zones/{id}/dsp",
            "put",
            "control",
            Some(("DspConfig", true)),
            "Zone",
        ),
        (
            "/zones/{id}/play",
            "post",
            "control",
            Some(("PlayRequest", false)),
            "Zone",
        ),
        ("/zones/{id}/pause", "post", "control", Some(("SystemControlRequest",false)), "Zone"),
        ("/zones/{id}/stop", "post", "control", Some(("SystemControlRequest",false)), "Zone"),
        ("/zones/{id}/transfer", "post", "control", Some(("PlaybackTransferRequest",true)), "PlaybackTransferResult"),
        ("/zones/{id}/ready", "post", "control", Some(("OutputReadyRequest",true)), "204"),
        ("/zones/{id}/visualization/report", "post", "control", Some(("VisualizationPublishRequest",true)), "204"),
        ("/zones/{id}/visualization/lease", "get", "control", None, "VisualizationLease"),
        ("/zones/{id}/prepare", "post", "control", Some(("SequencePrepareRequest",true)), "SequencePrepared"),
        ("/zones/{id}/handoff", "post", "control", Some(("SequenceHandoffRequest",true)), "SequenceHandoffResult"),
        ("/zones/{id}/recover", "post", "control", Some(("SequenceRecoverRequest",true)), "SequenceRecoverResult"),
        ("/zones/{id}/recover/unavailable", "post", "control", Some(("SequenceRecoveryUnavailableRequest",true)), "SequenceRecoveryUnavailableResult"),
        ("/zones/{id}/next", "post", "control", None, "Zone"),
        (
            "/zones/{id}/ended",
            "post",
            "control",
            Some(("EndedRequest", true)),
            "Zone",
        ),
        ("/zones/{id}/previous", "post", "control", None, "Zone"),
        (
            "/zones/{id}/seek",
            "post",
            "control",
            Some(("SeekRequest", true)),
            "Zone",
        ),
        (
            "/zones/{id}/progress",
            "post",
            "control",
            Some(("ProgressRequest", false)),
            "204",
        ),
        ("/endpoints", "get", "read", None, "EndpointList"),
        (
            "/endpoints/browser",
            "post",
            "control",
            Some(("BrowserEndpointRequest", true)),
            "Endpoint",
        ),
        ("/events/ticket", "get", "read", None, "EventTicket"),
        ("/endpoints/browser/{id}/disconnect", "post", "control", None, "204"),
        ("/events", "get", "read", None, "events"),
        ("/bridge/events", "get", "control", None, "bridge"),
        ("/cache", "get", "read", None, "CacheStatus"),
        (
            "/cache",
            "patch",
            "admin",
            Some(("CacheLimit", true)),
            "CacheStatus",
        ),
        ("/cache/{id}", "delete", "admin", None, "204"),
        ("/cache/{id}/export", "post", "read", None, "ExportQueued"),
        ("/cache/{id}/file", "get", "read", None, "portable"),
        ("/cache/{id}/file", "head", "read", None, "portable"),
        ("/downloads", "get", "read", None, "DownloadList"),
        (
            "/downloads",
            "post",
            "control",
            Some(("DownloadRequest", true)),
            "DownloadStarted",
        ),
        ("/downloads/{id}", "get", "read", None, "Download"),
        ("/downloads/{id}", "delete", "control", None, "204"),
        ("/downloads/{id}/file", "get", "read", None, "download"),
        ("/downloads/{id}/file", "head", "read", None, "download"),
        ("/plugins", "get", "read", None, "PluginList"),
        (
            "/plugins/{id}",
            "patch",
            "admin",
            Some(("PluginToggleRequest", true)),
            "PluginToggleResult",
        ),
        (
            "/plugins/{id}/health",
            "get",
            "admin",
            None,
            "ProviderHealth",
        ),
        ("/plugins/install", "post", "admin", None, "PluginInstalled"),
        (
            "/plugins/{id}/rollback",
            "post",
            "admin",
            None,
            "PluginInstalled",
        ),
        (
            "/providers/{id}/search",
            "get",
            "read",
            None,
            "ProviderSearch",
        ),
        ("/providers/{id}/playlists", "get", "read", None, "ProviderPlaylists"),
        ("/providers/{id}/playlists/{playlist_id}/tracks", "get", "read", None, "ProviderPlaylistTracks"),
        ("/recommendations", "get", "read", None, "RecommendationSelection"),
        ("/recommendations/daily", "get", "read", None, "PersonalPicks"),
        ("/recommendations/similar", "post", "read", Some(("SimilarTrackRequest",true)), "PersonalPicks"),
        ("/recommendations/discover", "get", "read", None, "RecommendationSelection"),
        ("/recommendations/radio", "post", "read", Some(("Object",true)), "Object"),
        ("/recommendations/settings", "get", "read", None, "Object"),
        ("/recommendations/settings", "put", "admin", Some(("Object",true)), "Object"),
        ("/recommendations/feedback/{id}", "put", "control", Some(("Object",true)), "Object"),
        ("/providers/{id}/recommendations", "get", "read", None, "Object"),
        ("/auth/sessions/current-device", "put", "read", Some(("Object",true)), "NoContent"),
        ("/portable/{id}/state", "put", "control", Some(("Object",true)), "NoContent"),
        ("/portable/{id}/state", "delete", "control", None, "NoContent"),
        ("/portable/{id}/ack", "post", "control", Some(("Object",true)), "NoContent"),
        (
            "/providers/{id}/login",
            "post",
            "admin",
            Some(("ProviderLoginRequest", false)),
            "ProviderLogin",
        ),
        (
            "/providers/{id}/login/{session}",
            "get",
            "admin",
            None,
            "ProviderLogin",
        ),
        (
            "/providers/{id}/sync",
            "post",
            "library",
            Some(("ProviderSyncRequest", true)),
            "ProviderSync",
        ),
        ("/settings", "get", "read", None, "Settings"),
        (
            "/settings",
            "patch",
            "admin",
            Some(("SettingsPatch", true)),
            "Saved",
        ),
        ("/preferences", "get", "authenticated", None, "Preferences"),
        (
            "/preferences",
            "put",
            "authenticated",
            Some(("Preferences", true)),
            "Preferences",
        ),
        ("/favorites", "get", "read", None, "Playlist"),
        ("/favorites/{id}", "put", "control", Some(("FavoriteChange",true)), "Playlist"),
        ("/favorites/binding", "put", "control", Some(("FavoritesBinding",true)), "IdResult"),
        ("/favorites/binding", "delete", "control", None, "204"),
        ("/filesystem/directories","get","admin",None,"AnyObject"),
        ("/filesystem/directories","post","admin",Some(("NewDirectory",true)),"AnyObject"),
        ("/maintenance/settings","get","admin",None,"BackupPolicy"),
        ("/maintenance/settings","put","admin",Some(("BackupPolicy",true)),"BackupPolicy"),
        ("/maintenance/health","get","admin",None,"AnyObject"),
        ("/maintenance/backups/{id}/verify","post","admin",None,"AnyObject"),
        ("/playlists/import/preview","post","control",Some(("PlaylistImportPreview",true)),"AnyObject"),
        ("/playlists/import","post","control",Some(("PlaylistImportCommit",true)),"AnyObject"),
        ("/playlists/{id}/export","get","read",None,"playlist_file"),
        ("/music/entities","get","library",None,"AnyObject"),
        ("/maintenance/backups","get","admin",None,"AnyObject"),
        ("/maintenance/backups","post","admin",None,"AnyObject"),
        ("/maintenance/backups/{id}/file","get","admin",None,"backup_archive"),
        ("/library/files","get","admin",None,"AnyObject"),
        ("/library/files/actions","post","admin",Some(("FileAction",true)),"AnyObject"),
        ("/library/files/bin","get","admin",None,"AnyObject"),
        ("/library/files/bin/{id}/restore","post","admin",None,"AnyObject"),
        ("/library/files/bin/{id}","delete","admin",Some(("PurgeFile",true)),"AnyObject"),
        ("/library/home", "get", "read", None, "AnyObject"),
        ("/library/folders", "get", "read", None, "AnyObject"),
        ("/library/tracks/{id}/cache", "get", "read", None, "AnyObject"),
        ("/library/tracks/{id}/cache", "post", "control", None, "AnyObject"),
        ("/playlists", "get", "read", None, "PlaylistList"),
        ("/playlists/{id}", "get", "read", None, "Playlist"),
        (
            "/playlists",
            "post",
            "control",
            Some(("PlaylistRequest", true)),
            "IdResult",
        ),
        (
            "/playlists/{id}",
            "put",
            "control",
            Some(("PlaylistPatch", true)),
            "Playlist",
        ),
        ("/playlists/{id}", "delete", "control", None, "204"),
        ("/stats", "get", "read", None, "ListeningStatistics"),
        ("/stats/history", "get", "read", None, "ListeningHistory"),
        ("/stats/offline", "post", "control", Some(("OfflineStatsRequest",true)), "OfflineStatsResult"),
        (
            "/library/tracks/{id}/stats",
            "get",
            "read",
            None,
            "TrackStatistics",
        ),
    ] {
        operation(&mut paths, path, method, scope, request, result);
    }
    music::paths(&mut paths);
    accounts::paths(&mut paths);
    device_groups::paths(&mut paths);
    json!({"openapi":"3.1.0","jsonSchemaDialect":"https://json-schema.org/draft/2020-12/schema","info":{"title":"R-Music Core","version":env!("CARGO_PKG_VERSION"),"description":"真实 v0.1 REST 合同；权限见 x-rmusic-scope，admin 包含其他权限，interactive 指账号密码会话。JSON 提取错误可能是 text/plain。未来关系模型明确为 draft。"},"servers":[{"url":"../..","description":"相对于 /api/v1/openapi.json 的核心根路径，保留反代前缀"}],"paths":paths,"components":{"securitySchemes":{"bearerAuth":{"type":"http","scheme":"bearer","description":"账号会话或限定权限 API Token"},"ticketAuth":{"type":"apiKey","in":"query","name":"ticket","description":"限定资源且绑定会话的票据；撤销会话后失效"}},"schemas":schemas()}})
}
pub async fn document() -> Json<Value> {
    Json(build_document())
}

#[cfg(test)]
mod tests {
    #[test]
    fn device_group_edit_requires_a_revision_and_two_to_four_unique_outputs() {
        let document=build_document();
        let edit=&document["components"]["schemas"]["DeviceGroupEdit"];
        assert!(edit["required"].as_array().unwrap().iter().any(|key|key=="expected_revision"));
        assert_eq!(edit["properties"]["zone_ids"]["minItems"],2);
        assert_eq!(edit["properties"]["zone_ids"]["maxItems"],4);
        assert_eq!(edit["properties"]["zone_ids"]["uniqueItems"],true);
        assert!(document["paths"]["/api/v1/device-groups/{id}"]["put"]["operationId"].is_string());
    }
    #[test]
    fn group_rate_contract_is_optional_manual_and_epoch_bound(){
        let doc=super::build_document();let schemas=&doc["components"]["schemas"];
        assert_eq!(schemas["SyncStartRequest"]["properties"]["pcm_rate_adjustment"]["default"],false);
        assert_eq!(schemas["GroupRateRequest"]["properties"]["ppm"]["maximum"],500);assert_eq!(schemas["GroupRateRequest"]["additionalProperties"],false);
        assert!(!schemas["SyncStart"]["required"].as_array().unwrap().iter().any(|v|v=="rate_adjustment"));
        assert_eq!(schemas["SyncStartRequest"]["properties"]["automatic_rate_adjustment"]["default"],false);assert_eq!(schemas["SyncStart"]["properties"]["rate_adjustment"]["properties"]["automatic"]["type"],"boolean");
        let op=&doc["paths"]["/api/v1/sync/starts/{id}/rate"]["post"];assert!(op["responses"]["202"].is_object());assert!(op["responses"]["409"].is_object());
    }
    use super::*;
    use std::collections::HashSet;
    #[test]
    fn source_clock_v2_contract_keeps_legacy_pair_and_processing_uncertainty_explicit(){
        let doc=build_document();let s=&doc["components"]["schemas"];
        assert_eq!(s["ProgrammeClockCalibration"]["properties"]["version"]["const"],1);assert_eq!(s["ProgrammeClockCalibrationV2"]["properties"]["version"]["const"],2);assert!(s["ProgrammeClockCalibrationV2"]["properties"]["source_frames"].is_null());
        for name in ["source_frame","source_fraction","source_per_output","track_output_frames"]{assert!(s["ProgrammeClockCalibrationV2"]["required"].as_array().unwrap().iter().any(|v|v==name));}
        assert!(s["DeviceClockCalibration"]["properties"]["programme_clock"].to_string().contains("ProgrammeClockCalibrationV2"));assert!(s["GroupClockMember"]["properties"]["processing_projection_uncertainty_ns"].to_string().contains("null"));assert_eq!(s["GroupClockDiagnostics"]["properties"]["strict_sync"]["const"],false);
    }
    #[test]
    fn convolution_contract_keeps_selection_tickets_and_endpoint_support_explicit(){
        let doc=build_document();let schemas=&doc["components"]["schemas"];let selection=&schemas["ConvolutionConfig"];
        assert_eq!(selection["additionalProperties"],false);assert_eq!(selection["properties"]["response_ids"]["maxItems"],16);assert_eq!(selection["properties"]["response_ids"]["uniqueItems"],true);assert!(schemas["DspConfig"]["properties"]["convolution"]["anyOf"].is_array());
        let manifest=&doc["paths"]["/api/v1/zones/{id}/dsp/impulses"]["post"];assert_eq!(manifest["x-rmusic-scope"],"control");assert!(manifest["responses"]["409"].is_object());
        let file=&doc["paths"]["/api/v1/dsp/impulses/{id}/file"]["get"];assert_eq!(file["security"].as_array().unwrap().len(),2);for name in ["ticket","zone_id","selection"]{assert!(file["parameters"].as_array().unwrap().iter().any(|parameter|parameter["name"]==name));}
        assert_eq!(schemas["BrowserEndpointRequest"]["properties"]["convolution_v1"]["default"],false);assert_eq!(schemas["Status"]["properties"]["capabilities"]["properties"]["convolution_config_v1"]["type"],"boolean");
    }
    fn check_references(value: &Value, root: &Value) {
        match value {
            Value::Object(map) => {
                if let Some(reference) = map.get("$ref").and_then(Value::as_str) {
                    assert!(
                        root.pointer(reference.trim_start_matches('#')).is_some(),
                        "missing {reference}"
                    );
                }
                for value in map.values() {
                    check_references(value, root);
                }
            }
            Value::Array(items) => {
                for value in items {
                    check_references(value, root);
                }
            }
            _ => {}
        }
    }
    #[test]
    fn all_references_and_operation_ids_are_valid() {
        let doc = build_document();
        check_references(&doc, &doc);
        let mut ids = HashSet::new();
        for (path, item) in doc["paths"].as_object().unwrap() {
            for op in item.as_object().unwrap().values() {
                assert!(
                    ids.insert(op["operationId"].as_str().unwrap()),
                    "duplicate operation id"
                );
                for part in path.split('/').filter(|p| p.starts_with('{')) {
                    let name = part.trim_matches(['{', '}']);
                    assert!(op["parameters"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|p| p["name"] == name && p["in"] == "path" && p["required"] == true));
                }
            }
        }
        if let Ok(path) = std::env::var("RMUSIC_OPENAPI_EXPORT") {
            std::fs::write(path, serde_json::to_vec_pretty(&doc).unwrap()).unwrap();
        }
    }
    #[test]
    fn status_codes_and_binary_bodies_match_handlers() {
        let doc = build_document();
        let paths = &doc["paths"];
        for path in [
            "/auth/logout",
            "/auth/tokens/{id}",
            "/auth/sessions/{id}",
            "/library/roots/{id}",
            "/zones/{id}",
            "/cache/{id}",
            "/downloads/{id}",
            "/playlists/{id}",
        ] {
            let method = if path == "/auth/logout" {
                "post"
            } else {
                "delete"
            };
            let responses = &paths[format!("/api/v1{path}")][method]["responses"];
            assert!(responses.get("204").is_some());
            assert!(responses.get("200").is_none());
            assert!(responses["204"].get("content").is_none());
        }
        assert!(paths["/api/v1/zones/{id}/progress"]["post"]["responses"]
            .get("204")
            .is_some());
        assert!(
            paths["/api/v1/zones/{id}/queue"]["delete"]["responses"]["200"]["content"].is_object()
        );
        for path in ["/api/v1/media/{id}", "/api/v1/downloads/{id}/file"] {
            for status in ["200", "206"] {
                assert!(paths[path]["get"]["responses"][status]["content"].is_object());
                assert!(paths[path]["head"]["responses"][status]
                    .get("content")
                    .is_none());
            }
            assert!(paths[path]["get"]["responses"].get("416").is_some());
        }
        assert!(
            paths["/api/v1/transcode/{id}"]["get"]["responses"]["200"]["content"]
                .get("audio/mp4")
                .is_some()
        );
        let flac=&paths["/api/v1/native-flac/{id}"]["get"];
        assert!(flac["responses"]["200"]["content"]["audio/ogg"].is_object());
        assert!(flac["responses"]["200"]["headers"]["X-RMusic-Native-FLAC"].is_object());
        let pcm=&paths["/api/v1/pcm/{id}"]["get"];
        assert!(pcm["responses"]["200"]["content"]["application/octet-stream"].is_object());
        assert!(pcm["responses"]["200"]["headers"]["X-RMusic-PCM"].is_object());
        assert!(pcm["parameters"].as_array().unwrap().iter().any(|p|p["name"]=="rate"&&p["required"]==true));
        assert!(pcm["responses"]["429"].is_object());assert!(pcm["responses"].get("206").is_none());
        assert!(
            paths["/api/v1/plugins/install"]["post"]["requestBody"]["content"]
                .get("application/zip")
                .is_some()
        );
        let impulses=&paths["/api/v1/dsp/impulses"];
        assert!(impulses["post"]["requestBody"]["content"]["audio/wav"].is_object());
        assert!(impulses["post"]["responses"]["413"].is_object());assert!(impulses["post"]["responses"]["409"].is_object());
        let file=&paths["/api/v1/dsp/impulses/{id}/file"];
        assert!(file["get"]["responses"]["200"]["content"]["audio/wav"].is_object());
        assert!(file["get"]["responses"].get("206").is_none());assert!(file["head"]["responses"]["200"].get("content").is_none());
    }
    #[test]
    fn serialized_zone_and_sdk_models_have_real_types() {
        let doc = build_document();
        let s = &doc["components"]["schemas"];
        let zone = serde_json::to_value(crate::models::Zone::new(
            "id".into(),
            "name".into(),
            "browser-local".into(),
        ))
        .unwrap();
        for key in zone.as_object().unwrap().keys() {
            assert!(
                s["Zone"]["properties"].get(key).is_some(),
                "missing zone property {key}"
            );
        }
        assert_eq!(s["Track"]["properties"]["created_at"]["type"], "integer");
        assert_eq!(s["Zone"]["properties"]["updated_at"]["type"], "integer");
        assert_eq!(
            s["ThemeManifest"]["properties"]["schema_version"]["type"],
            "integer"
        );
        assert_eq!(
            s["PlaybackLayout"]["properties"]["modules"]["type"],
            "array"
        );
        assert_eq!(s["Recording"]["x-rmusic-status"], "draft");
        assert_eq!(s["SourceVariant"]["x-rmusic-status"], "draft");
    }
    #[test]
    fn statistics_and_portable_cache_contracts_match_current_handlers() {
        let doc = build_document();
        let paths = &doc["paths"];
        let s = &doc["components"]["schemas"];
        assert_eq!(
            paths["/api/v1/stats"]["get"]["parameters"][0]["name"],
            "period_days"
        );
        assert_eq!(
            paths["/api/v1/library/tracks/{id}/stats"]["get"]["x-rmusic-scope"],
            "read"
        );
        assert_eq!(
            s["ListeningStatistics"]["properties"]["tracks"]["maxItems"],
            100
        );
        assert_eq!(
            s["TrackStatistics"]["properties"]["listened_seconds"]["type"],
            "number"
        );
        assert_eq!(
            s["CacheStatus"]["properties"]["eviction"]["const"],
            "least_played"
        );
        assert!(s["CacheLimit"].get("required").is_none());
        for key in ["200", "206", "409", "416"] {
            assert!(paths["/api/v1/cache/{id}/file"]["get"]["responses"]
                .get(key)
                .is_some());
        }
        assert!(paths["/api/v1/cache/{id}/file"]["head"]["responses"]["206"]
            .get("content")
            .is_none());
        assert_eq!(
            paths["/api/v1/cache"]["patch"]["responses"]["200"]["content"]["application/json"]
                ["schema"]["$ref"],
            "#/components/schemas/CacheStatus"
        );
    }
    #[test]
    fn provider_playlists_recommendations_and_autoplay_have_truthful_contracts() {
        let doc=build_document();
        let s=&doc["components"]["schemas"];
        for path in ["/api/v1/providers/{id}/playlists", "/api/v1/providers/{id}/playlists/{playlist_id}/tracks", "/api/v1/recommendations"] {
            assert_eq!(doc["paths"][path]["get"]["x-rmusic-scope"],"read");
            assert!(doc["paths"][path]["get"]["responses"]["200"].is_object());
        }
        assert_eq!(s["ProviderPlaylist"]["properties"]["kind"]["enum"], json!(["created","collected","liked"]));
        assert_eq!(s["RecommendationSelection"]["properties"]["items"]["maxItems"],50);
        assert_eq!(s["Autoplay"]["required"],json!(["enabled","source","fallback_local"]));
        assert_eq!(s["Autoplay"]["properties"]["enabled"]["default"],true);
        assert_eq!(s["ZonePatch"]["properties"]["autoplay"]["$ref"],"#/components/schemas/Autoplay");
        let parameters=doc["paths"]["/api/v1/recommendations"]["get"]["parameters"].as_array().unwrap();
        assert_eq!(parameters.iter().find(|p|p["name"]=="limit").unwrap()["schema"]["maximum"],50);
        assert_eq!(parameters.iter().find(|p|p["name"]=="fallback_local").unwrap()["schema"]["default"],true);
    }
    #[test]
    fn natural_eof_and_delayed_progress_have_explicit_track_contracts() {
        let doc = build_document();
        let schemas = &doc["components"]["schemas"];
        let ended = &doc["paths"]["/api/v1/zones/{id}/ended"]["post"];
        assert_eq!(ended["x-rmusic-scope"], "control");
        assert_eq!(ended["requestBody"]["required"], true);
        assert_eq!(
            ended["requestBody"]["content"]["application/json"]["schema"]["$ref"],
            "#/components/schemas/EndedRequest"
        );
        assert_eq!(schemas["EndedRequest"]["required"], json!(["track_id"]));
        assert_eq!(
            schemas["EndedRequest"]["properties"]["track_id"]["type"],
            "string"
        );
        assert!(schemas["ProgressRequest"]["properties"]
            .get("track_id")
            .is_some());
        assert!(schemas["ProgressRequest"].get("required").is_none());
        for name in ["ProgressRequest","EndedRequest"] {
            assert_eq!(schemas[name]["properties"]["frames"]["type"],"integer");
            assert_eq!(schemas[name]["properties"]["sample_rate"]["minimum"],8000);
            assert_eq!(schemas[name]["dependentRequired"]["frames"],json!(["sample_rate","position"]));
            assert_eq!(schemas[name]["dependentRequired"]["sample_rate"],json!(["frames","position"]));
        }
        for name in ["Zone","ZoneProgress","ProgressRequest","EndedRequest"] {assert!(schemas[name]["properties"].get("playback_id").is_some());}
        assert!(schemas["BrowserEndpointRequest"]["properties"].get("playback_instance").is_some());
        assert!(ended["responses"].get("200").is_some());
        assert!(ended["responses"].get("204").is_none());
        assert!(
            doc["paths"]["/api/v1/zones/{id}/progress"]["post"]["responses"]
                .get("204")
                .is_some()
        );
    }
    #[test]
    fn continuous_handoff_has_bounded_frames_required_owner_and_conflict_response() {
        let doc = build_document();
        let schemas = &doc["components"]["schemas"];
        for (path, request, result) in [("prepare","SequencePrepareRequest","SequencePrepared"),("handoff","SequenceHandoffRequest","SequenceHandoffResult"),("recover","SequenceRecoverRequest","SequenceRecoverResult"),("recover/unavailable","SequenceRecoveryUnavailableRequest","SequenceRecoveryUnavailableResult")] {
            let op = &doc["paths"][format!("/api/v1/zones/{{id}}/{path}")]["post"];
            assert_eq!(op["x-rmusic-scope"],"control");
            assert_eq!(op["requestBody"]["content"]["application/json"]["schema"]["$ref"],format!("#/components/schemas/{request}"));
            assert_eq!(op["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],format!("#/components/schemas/{result}"));
        }
        assert_eq!(schemas["SequenceHandoffRequest"]["properties"]["ended"]["maxItems"],8);
        assert_eq!(schemas["SequenceHandoffRequest"]["additionalProperties"],false);
        assert_eq!(schemas["SequenceBoundary"]["properties"]["frames"]["type"],"integer");
        assert!(doc["paths"]["/api/v1/zones/{id}/handoff"]["post"]["responses"]["409"].is_object());
        assert_eq!(schemas["BrowserEndpointRequest"]["properties"]["continuous_pcm"]["default"],false);
        assert_eq!(schemas["SequenceRecoverRequest"]["properties"]["heard"]["maxItems"],9);
        assert_eq!(schemas["SequenceRecoverRequest"]["additionalProperties"],false);
        assert!(doc["paths"]["/api/v1/zones/{id}/recover"]["post"]["responses"]["409"].is_object());
        assert!(doc["paths"]["/api/v1/zones/{id}/recover"]["post"]["responses"]["503"].is_object());
        assert_eq!(schemas["BrowserEndpointRequest"]["properties"]["continuous_recovery"]["default"],false);
        assert_eq!(schemas["BrowserEndpointRequest"]["properties"]["recovery_controls"]["default"],false);
    }
    #[test]
    fn provider_login_has_optional_qq_or_wechat_request_and_method_in_response() {
        let doc=build_document();let schemas=&doc["components"]["schemas"];
        let request=&doc["paths"]["/api/v1/providers/{id}/login"]["post"]["requestBody"];
        assert_eq!(request["required"],false);
        assert_eq!(request["content"]["application/json"]["schema"]["$ref"],"#/components/schemas/ProviderLoginRequest");
        assert_eq!(schemas["ProviderLoginRequest"]["properties"]["type"]["enum"],json!(["qq","wx"]));
        assert_eq!(schemas["ProviderLoginRequest"]["properties"]["type"]["default"],"qq");
        assert!(schemas["ProviderLogin"]["properties"]["login_type"].is_object());
        assert!(schemas["ProviderLogin"]["properties"]["status"]["enum"].as_array().unwrap().contains(&json!("cancelled")));
    }
}
