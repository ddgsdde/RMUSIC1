//! Bounded source recovery. Never splice bytes without a strong, unchanged entity validator.
use crate::{catalog, error::{ApiError, ApiResult}, models::Track, state::AppState};
use axum::{body::Bytes, http::{header, StatusCode}};
use futures_util::{stream::BoxStream, StreamExt};
use serde_json::Value;

pub fn strong_etag(response: &reqwest::Response) -> Option<String> {
    response.headers().get(header::ETAG)?.to_str().ok()
        .filter(|v|v.len()>=2&&v.starts_with('"')&&v.ends_with('"')).map(String::from)
}
pub fn content_range(value: &str) -> Option<(u64,u64,u64)> {
    let (bounds,total)=value.strip_prefix("bytes ")?.split_once('/')?;
    let (start,end)=bounds.split_once('-')?;
    let (start,end,total)=(start.parse().ok()?,end.parse().ok()?,total.parse().ok()?);
    (start<=end&&end<total).then_some((start,end,total))
}
fn expired(status: StatusCode) -> bool {
    matches!(status,StatusCode::UNAUTHORIZED|StatusCode::FORBIDDEN|StatusCode::NOT_FOUND|StatusCode::GONE)
}
async fn request(s:&AppState,resolved:&Value,range:Option<&str>,validator:Option<&str>)->ApiResult<reqwest::Response> {
    let url=resolved["url"].as_str().ok_or_else(||ApiError::bad("音源未返回可播放地址"))?;
    let parsed=url::Url::parse(url).map_err(|_|ApiError::bad("音源地址无效"))?;
    if !["http","https"].contains(&parsed.scheme()){return Err(ApiError::bad("音源协议不支持"));}
    let mut req=s.http.get(parsed);
    if let Some(headers)=resolved["headers"].as_object(){for(k,v)in headers {if let Some(v)=v.as_str(){req=req.header(k,v);}}}
    if let Some(range)=range {req=req.header(header::RANGE,range);}
    if let Some(validator)=validator {req=req.header(header::IF_RANGE,validator);}
    req.send().await.map_err(|_|ApiError(StatusCode::BAD_GATEWAY,"upstream_failed".into(),"音源连接失败，请稍后重试".into()))
}
async fn request_with_refresh(s:&AppState,t:&Track,quality:&str,resolved:&mut Value,range:Option<&str>,validator:Option<&str>)->ApiResult<reqwest::Response> {
    let response=request(s,resolved,range,validator).await?;
    if !expired(response.status()){return Ok(response);}
    drop(response);
    *resolved=catalog::refresh_quality(s,t,quality,resolved).await?;
    request(s,resolved,range,validator).await
}
pub async fn open(s:&AppState,t:&Track,quality:&str,range:Option<&str>)->ApiResult<(Value,reqwest::Response)> {
    let mut resolved=catalog::resolve_quality(s,t,quality).await?;
    let response=request_with_refresh(s,t,quality,&mut resolved,range,None).await?;
    Ok((resolved,response))
}
struct SourceStream {
    s: AppState,
    track: Track,
    quality: String,
    resolved: Value,
    input: BoxStream<'static,Result<Bytes,reqwest::Error>>,
    next: u64,
    end: u64,
    total: u64,
    validator: Option<String>,
    retries: u8,
    finished: bool,
}
impl SourceStream {
    async fn next_chunk(&mut self)->Option<Result<Bytes,std::io::Error>> {
        if self.finished||self.next>self.end {return None;}
        loop {
            match self.input.next().await {
                Some(Ok(bytes)) if bytes.is_empty()=>continue,
                Some(Ok(bytes)) if bytes.len() as u64<=self.end-self.next+1=>{
                    self.next+=bytes.len() as u64;return Some(Ok(bytes));
                }
                Some(Ok(_))=>return self.fail("音源返回了超出请求范围的数据"),
                Some(Err(_))|None=>{}
            }
            let Some(validator)=self.validator.clone() else{return self.fail("音源中断，缺少可靠的文件校验信息，无法续传");};
            let format=self.resolved["format"].clone();
            let quality=self.resolved["quality"].clone();
            let cache_allowed=self.resolved["cache_allowed"].as_bool()==Some(true);
            loop {
                if self.retries>=2 {return self.fail("音源中断，续传重试次数已用尽");}
                self.retries+=1;
                let range=format!("bytes={}-{}",self.next,self.end);
                let response=match request_with_refresh(&self.s,&self.track,&self.quality,&mut self.resolved,Some(&range),Some(&validator)).await {
                    Ok(response)=>response,
                    Err(error) if error.1=="upstream_failed"=>continue,
                    Err(_)=>return self.fail("音源地址无法恢复，请稍后重试"),
                };
                let bounds=response.headers().get(header::CONTENT_RANGE).and_then(|v|v.to_str().ok()).and_then(content_range);
                if response.status()!=StatusCode::PARTIAL_CONTENT||strong_etag(&response).as_deref()!=Some(&validator)
                    ||bounds!=Some((self.next,self.end,self.total))
                    ||self.resolved["format"]!=format||self.resolved["quality"]!=quality
                    ||(cache_allowed&&self.resolved["cache_allowed"].as_bool()!=Some(true))
                    ||response.content_length().is_some_and(|n|n!=self.end-self.next+1) {
                    return self.fail("音源文件或续传范围发生变化，已停止拼接");
                }
                self.input=response.bytes_stream().boxed();break;
            }
        }
    }
    fn fail(&mut self,message:&str)->Option<Result<Bytes,std::io::Error>> {
        self.finished=true;Some(Err(std::io::Error::other(message)))
    }
}
#[allow(clippy::too_many_arguments)]
pub fn stream(s:AppState,track:Track,quality:String,resolved:Value,response:reqwest::Response,start:u64,end:u64,total:u64)->BoxStream<'static,Result<Bytes,std::io::Error>> {
    let validator=strong_etag(&response);
    let state=SourceStream{s,track,quality,resolved,input:response.bytes_stream().boxed(),next:start,end,total,validator,retries:0,finished:false};
    futures_util::stream::unfold(state,|mut state|async move {let chunk=state.next_chunk().await?;Some((chunk,state))}).boxed()
}
