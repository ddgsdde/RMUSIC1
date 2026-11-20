use super::*;
pub(super) fn schemas(s:&mut Value){
 s["ExternalSourcePolicy"]=json!({"type":"object","additionalProperties":false,"properties":{"providers":{"type":"array","maxItems":32,"uniqueItems":true,"items":{"type":"string","pattern":"^[A-Za-z0-9_-]{1,80}$"}},"local_cache":{"type":"boolean","default":false},"local_library":{"type":"boolean","default":false},"bandwidth_bytes_per_second":{"type":"integer","minimum":0,"maximum":1073741824,"default":0,"description":"账号所有设备的媒体响应合计；0 不限速，按实际传输字节计费"}}});
 s["ExternalAccount"]=object(json!({"id":string(),"username":string(),"role":{"const":"external"},"totp_enabled":boolean(),"enabled":boolean(),"created_at":unsigned(),"policy":reference("ExternalSourcePolicy")}),&["id","username","role","totp_enabled","enabled","created_at","policy"]);
 s["ExternalAccountPage"]=object(json!({"items":array(reference("ExternalAccount")),"next_cursor":nullable(string())}),&["items","next_cursor"]);
 for (name,extra,required) in [
  ("ExternalAccountCreate",json!({"username":string(),"password":{"type":"string","minLength":10,"maxLength":1024},"policy":reference("ExternalSourcePolicy")}),vec!["username","password","current_password"]),
  ("ExternalAccountUpdate",json!({"username":string(),"new_password":{"type":"string","minLength":10,"maxLength":1024},"policy":reference("ExternalSourcePolicy"),"enabled":boolean(),"reset_totp":boolean()}),vec!["current_password"]),
  ("ExternalAccountDelete",json!({"confirmation":string()}),vec!["current_password","confirmation"]),
  ("AccountImportApply",json!({"mode":{"enum":["merge","new"]},"target_user":string(),"username":string(),"password":{"type":"string","minLength":10,"maxLength":1024},"policy":reference("ExternalSourcePolicy")}),vec!["current_password","mode"]),
 ]{let mut properties=extra.as_object().cloned().unwrap_or_default();properties.insert("current_password".into(),string());properties.insert("totp".into(),string());s[name]=object(Value::Object(properties),&required);s[name]["additionalProperties"]=json!(false);}
 s["AccountImportJob"]=object(json!({"id":string(),"state":{"enum":["created","uploading","uploaded","applying","interrupted","completed","invalid"]},"manifest":nullable(reference("AnyObject")),"bytes":unsigned(),"records":unsigned(),"applied":unsigned(),"target_user":nullable(string()),"error":nullable(string()),"merge_candidates":array(reference("AnyObject"))}),&["id","state"]);
}
pub(super) fn paths(paths:&mut Value){
 for(path,method,request,result)in[
  ("/auth/external-accounts","get",None,"ExternalAccountPage"),
  ("/auth/external-accounts","post",Some(("ExternalAccountCreate",true)),"ExternalAccount"),
  ("/auth/external-accounts/{id}","patch",Some(("ExternalAccountUpdate",true)),"ExternalAccount"),
  ("/auth/external-accounts/{id}","delete",Some(("ExternalAccountDelete",true)),"AnyObject"),
  ("/auth/external-accounts/{id}/export-ticket","post",Some(("ReauthenticateRequest",true)),"AnyObject"),
  ("/auth/external-accounts/{id}/export","get",None,"AnyObject"),
  ("/auth/account-imports","get",None,"AnyObject"),
  ("/auth/account-imports","post",Some(("ReauthenticateRequest",true)),"AccountImportJob"),
  ("/auth/account-imports/{id}","get",None,"AccountImportJob"),
  ("/auth/account-imports/{id}","delete",None,"AnyObject"),
  ("/auth/account-imports/{id}/file","put",None,"AccountImportJob"),
  ("/auth/account-imports/{id}/authorize-upload","post",Some(("ReauthenticateRequest",true)),"AccountImportJob"),
  ("/auth/account-imports/{id}/apply","post",Some(("AccountImportApply",true)),"AccountImportJob"),
 ]{operation(paths,path,method,"admin",request,result);}
 let export=&mut paths["/api/v1/auth/external-accounts/{id}/export"]["get"];
 export["security"]=json!([{"ticketAuth":[]}]);export["responses"]["200"]=json!({"description":"流式账号档案；不含认证凭据。来源身份与统计修订保证重复合并幂等。","content":{"application/x-ndjson":{"schema":{"type":"string","format":"binary"}}}});
 let upload=&mut paths["/api/v1/auth/account-imports/{id}/file"]["put"];
 upload["requestBody"]=json!({"required":true,"content":{"application/x-ndjson":{"schema":{"type":"string","format":"binary"}}},"description":"最多 2 GiB；每行最多 4 MiB；最多两百万条记录；每批 128 条。先预览，再显式选择合并或新建。"});
}
