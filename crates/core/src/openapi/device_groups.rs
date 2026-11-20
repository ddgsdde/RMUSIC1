use super::*;
pub(super) fn schemas(s: &mut Value) {
    s["Status"]["properties"]["capabilities"]["properties"]["device_groups_v1"] = boolean();
    s["DeviceGroup"] = object(
        json!({"id":string(),"name":string(),"zone_id":string(),"endpoint_id":string(),"zone_ids":array(string()),"source_zone_id":string(),"revision":unsigned(),"phase":string(),"job_id":nullable(string()),"error":nullable(string())}),
        &[
            "id",
            "name",
            "zone_id",
            "endpoint_id",
            "zone_ids",
            "source_zone_id",
            "phase",
        ],
    );
    s["DeviceGroupCreate"] = json!({"type":"object","additionalProperties":false,"properties":{"request_id":{"type":"string","format":"uuid"},"name":{"type":"string","minLength":1,"maxLength":80},"zone_ids":{"type":"array","minItems":2,"maxItems":4,"uniqueItems":true,"items":{"type":"string"}},"source_zone_id":string()},"required":["request_id","name","zone_ids","source_zone_id"]});
    s["DeviceGroupEdit"] = json!({"type":"object","additionalProperties":false,"properties":{"name":{"type":"string","minLength":1,"maxLength":80},"zone_ids":{"type":"array","minItems":2,"maxItems":4,"uniqueItems":true,"items":string()},"expected_revision":unsigned()},"required":["name","zone_ids","expected_revision"]});
    s["DeviceGroupPage"] = object(json!({"items":array(reference("DeviceGroup"))}), &["items"]);
}
pub(super) fn paths(paths: &mut Value) {
    operation(
        paths,
        "/device-groups",
        "get",
        "read",
        None,
        "DeviceGroupPage",
    );
    operation(
        paths,
        "/device-groups",
        "post",
        "control",
        Some(("DeviceGroupCreate", true)),
        "DeviceGroup",
    );
    operation(
        paths,
        "/device-groups/{id}",
        "delete",
        "control",
        None,
        "AnyObject",
    );
    operation(paths,"/device-groups/{id}","put","control",Some(("DeviceGroupEdit",true)),"DeviceGroup");
    paths["/api/v1/device-groups/{id}"]["put"]["description"]=json!("调整名称和成员，保留组合身份、当前歌曲及队列。移出成员恢复原队列；新增成员保存自己的当前队列。正在播放时先收尾旧输出，再准备新成员。相同内容重试幂等，修订冲突返回 409。");
    paths["/api/v1/device-groups"]["post"]["responses"]["201"] = json!({"description":"成员队列已持久保存，创建独立的组合播放设备。","content":{"application/json":{"schema":reference("DeviceGroup")}}});
    paths["/api/v1/device-groups/{id}"]["delete"]["responses"]["204"] = json!({"description":"停止组合并恢复成员原队列、顺序、位置和设置；原播放保持暂停。重复请求幂等。"});
    for path in ["/api/v1/device-groups", "/api/v1/device-groups/{id}"] {
        for method in ["get", "post", "delete"] {
            if paths[path][method].is_object() {
                paths[path][method]["description"]=json!("仅家庭账号使用。成员必须是具备完整同步能力的 Mac／Windows 原生输出或 Linux 网桥，网页和安卓只遥控。组合以虚拟 zone/endpoint 出现在普通设备列表，通过标准 zone 播放接口控制。成员加入期间不可单独改队列或播放。" );
            }
        }
    }
}
