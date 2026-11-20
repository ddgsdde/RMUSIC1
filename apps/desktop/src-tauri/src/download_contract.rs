//! Only an exact, strongly identified original entity may extend persisted bytes.
pub fn strong_etag(tag:Option<&str>)->bool {tag.is_some_and(|value|value.len()>=2&&value.starts_with('"')&&value.ends_with('"')&&!value[1..value.len()-1].contains(['"','\r','\n']))}
pub fn validate_response(status:u16,range:Option<&str>,etag:Option<&str>,expected_etag:Option<&str>,length:Option<u64>,offset:u64,total:u64)->Result<(),&'static str>{
 if total==0||offset>=total{return Err("无效的下载范围");}
 if let Some(expected)=expected_etag {if !strong_etag(Some(expected))||etag!=Some(expected){return Err("音源实体已变化，请重试下载；旧片段保留");}}
 else if offset!=0{return Err("缺少强实体校验，不能续传旧片段");}
 let exact=format!("bytes {offset}-{}/{total}",total-1);
 if !(status==206&&range==Some(exact.as_str())||offset==0&&status==200){return Err("音源未确认下载范围，拒绝拼接");}
 if length!=Some(total-offset){return Err("音源长度已变化，拒绝拼接");}Ok(())
}
#[cfg(test)]mod tests{
 use super::*;
 #[test]fn only_strong_entities_resume(){assert!(strong_etag(Some("\"v1\"")));for tag in [None,Some("W/\"v1\""),Some("v1"),Some("\"bad\"tag\""),Some("\"line\nbreak\"")]{assert!(!strong_etag(tag));}}
 #[test]fn exact_resume_accepts_only_matching_entity_and_range(){assert!(validate_response(206,Some("bytes 100-999/1000"),Some("\"a\""),Some("\"a\""),Some(900),100,1000).is_ok());for (status,range,etag,length) in [(200,None,Some("\"a\""),Some(1000)),(206,Some("bytes 0-999/1000"),Some("\"a\""),Some(900)),(206,Some("bytes 100-999/1000"),Some("\"b\""),Some(900)),(206,Some("bytes 100-999/1000"),Some("\"a\""),Some(899))]{assert!(validate_response(status,range,etag,Some("\"a\""),length,100,1000).is_err());}}
 #[test]fn unversioned_response_can_only_start_from_zero(){assert!(validate_response(200,None,None,None,Some(1000),0,1000).is_ok());assert!(validate_response(206,Some("bytes 100-999/1000"),None,None,Some(900),100,1000).is_err());}
}
