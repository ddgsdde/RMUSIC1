//! Retain measured boundaries and immutable in-flight requests until the core acknowledges them.
use super::*;

#[derive(Clone,Serialize,Deserialize)]
pub(super) struct Checkpoint {
    pub playback_id:String,pub track_id:String,pub position:f64,pub frames:u64,
}
pub(super) struct Journal {
    measured_version:u32,confirmed:String,
    heard:VecDeque<Checkpoint>,
    pub(super) pending:Option<(&'static str,Value)>,
    pub(super) terminal:bool,
    invalidated:bool,
}
pub(super) type Shared=Arc<Mutex<Journal>>;
impl Journal {
    #[cfg(test)]
    pub fn new(first:Value)->Result<Shared> {Self::new_timed(first,1)}
    pub fn new_timed(first:Value,measured_version:u32)->Result<Shared> {
        if ![1,2].contains(&measured_version){anyhow::bail!("invalid listening timing version");}
        let first:Checkpoint=serde_json::from_value(first)?;
        Ok(Arc::new(Mutex::new(Self {measured_version,confirmed:first.playback_id.clone(),heard:VecDeque::from([first]),pending:None,terminal:false,invalidated:false})))
    }
    pub fn observe(&mut self,value:Value)->Result<()> {
        let next:Checkpoint=serde_json::from_value(value)?;
        if let Some(previous)=self.heard.iter_mut().find(|p|p.playback_id==next.playback_id) {
            if next.frames>=previous.frames&&next.position>=previous.position {*previous=next;}
        }else {
            // The device has one current source plus at most eight reserved future sources.
            if self.heard.len()>=9 {anyhow::bail!("unacknowledged playback window exceeded");}
            self.heard.push_back(next);
        }Ok(())
    }
    pub fn reports(&self)->Value {json!(self.heard)}
    pub fn begin(&mut self,action:&'static str,body:Value) {self.pending=Some((action,body));}
    pub fn acknowledge(&mut self,response:&Value)->Value {
        let Some((action,body))=self.pending.take() else{return Value::Null};
        let zone=if action=="handoff" {response["zone"].clone()}else{response.clone()};
        if action=="handoff"&&response["accepted"]==true&&zone["playback_id"]==body["current"]["playback_id"] {
            self.confirmed=body["current"]["playback_id"].as_str().unwrap().into();
            while self.heard.front().is_some_and(|p|p.playback_id!=self.confirmed) {self.heard.pop_front();}
        }else if action=="handoff"||zone["playback_id"]!=body["playback_id"]||zone["status"]=="stopped" {
            self.invalidated=true;
        }zone
    }
    fn next_handoff(&self,endpoint:&str,plan:&str,rate:u32)->Option<Value> {
        if self.invalidated||self.heard.len()<2||self.heard.front()?.playback_id!=self.confirmed {return None;}
        let ended:Vec<_>=self.heard.iter().take(self.heard.len()-1).cloned().collect();
        let mut body=json!({"endpoint_id":endpoint,"request_id":uuid::Uuid::new_v4().to_string(),"plan_id":plan,"from_playback_id":self.confirmed,
            "sample_rate":rate,"ended":ended,"current":self.heard.back()?});if self.measured_version==2{body["measured_version"]=json!(2);}Some(body)
    }
}
fn lock(journal:&Shared)->Result<std::sync::MutexGuard<'_,Journal>> {
    journal.lock().map_err(|_|anyhow::anyhow!("playback recovery lock poisoned"))
}
pub(in crate::app) struct Recovery {
    pub(super) journal:Shared,pub(super) sequence:PlaybackSequence,pub(super) control:Arc<PlaybackControl>,pub(super) entries:Entries,
    pub(super) context:Context,pub(super) plan_id:String,
    pub(super) gate:Arc<tokio::sync::Mutex<()>>,
}
impl Recovery {
    pub fn refresh(&self)->Result<()> {
        let position=self.sequence.position(&self.control)?;
        lock(&self.journal)?.observe(boundary(&position.playback_id,position.position,position.frames,&self.entries)?)
    }
    pub async fn reconcile(&self,core:&Config)->Result<Value> {
        let _gate=self.gate.lock().await;
        self.refresh()?;
        let rate=self.sequence.format()?.sample_rate;
        reconcile(&self.journal,core,&self.context.zone_id,&self.plan_id,rate).await
    }
}
async fn reconcile(journal:&Shared,core:&Config,zone_id:&str,plan:&str,rate:u32)->Result<Value> {
    let mut zone=core.zone(zone_id).await?;
    loop {
        let pending=lock(journal)?.pending.clone();
        if let Some((action,body))=pending {
            // A failure leaves exactly this body and request ID available for the next recovery.
            let response=core.request_json(zone_id,action,&body).await?;
            zone=lock(journal)?.acknowledge(&response);
        }
        let next=lock(journal)?.next_handoff(&core.endpoint,plan,rate);
        if let Some(body)=next {lock(journal)?.begin("handoff",body);continue;}
        break;
    }
    // Never undo a later pause, stop, seek or endpoint change while checkpointing heard audio.
    if zone["endpoint_id"]!=core.endpoint||!matches!(zone["status"].as_str(),Some("playing"|"paused")){return Ok(zone);}
    let checkpoint=lock(journal)?.heard.iter().find(|p|Some(p.playback_id.as_str())==zone["playback_id"].as_str()).cloned();
    let Some(checkpoint)=checkpoint else{return Ok(zone)};
    let mut body=json!(checkpoint);body["sample_rate"]=json!(rate);if lock(journal)?.measured_version==2{body["measured_version"]=json!(2);}
    core.post(zone_id,"progress",&body).await?.error_for_status()?;
    let terminal={let journal=lock(journal)?;journal.terminal&&!journal.invalidated&&journal.confirmed==checkpoint.playback_id};
    if terminal&&zone["status"]=="playing" {
        lock(journal)?.begin("ended",body.clone());
        let response=core.request_json(zone_id,"ended",&body).await?;
        return Ok(lock(journal)?.acknowledge(&response));
    }
    core.zone(zone_id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt,AsyncWriteExt};
    fn heard(id:&str,seconds:f64)->Value {json!({"playback_id":id,"track_id":format!("track-{id}"),"position":seconds,"frames":(seconds*48000.).round() as u64})}
    fn zone(id:&str,status:&str,position:f64)->Value {json!({"endpoint_id":"native","playback_id":id,"track":{"id":format!("track-{id}")},"status":status,"position":position})}
    fn initial()->(Shared,Value) {
        let journal=Journal::new(heard("a",0.)).unwrap();
        lock(&journal).unwrap().observe(heard("a",2.)).unwrap();lock(&journal).unwrap().observe(heard("b",0.125)).unwrap();
        let body=lock(&journal).unwrap().next_handoff("native","plan",48000).unwrap();
        lock(&journal).unwrap().begin("handoff",body.clone());(journal,body)
    }
    async fn server(replies:Vec<(&'static str,u16,Option<Value>)>)->(Config,tokio::task::JoinHandle<Vec<Value>>) {
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();
        let profile=ConnectionProfile{core_public_key:None,id:"qa".into(),name:"QA".into(),address:format!("http://{address}/external/music"),core_id:Some("qa-core".into()),skip_certificate_verification: false, certificate_sha256: None,alternatives:vec![]};
        let task=tokio::spawn(async move {
            let mut requests=Vec::new();
            for (action,status,response) in replies {
                let (mut socket,_)=tokio::time::timeout(Duration::from_secs(3),listener.accept()).await.unwrap().unwrap();
                let mut headers=Vec::new();while !headers.ends_with(b"\r\n\r\n"){headers.push(socket.read_u8().await.unwrap());}
                let headers=String::from_utf8(headers).unwrap();let path=if action=="zone" {"/external/music/api/v1/zones/z".into()}else{format!("/external/music/api/v1/zones/z/{action}")};
                assert!(headers.starts_with(&format!("{} {path} ",if action=="zone"{"GET"}else{"POST"})),"{headers}");assert!(headers.contains("Bearer qa-session"));
                let length=headers.lines().find_map(|line|line.to_ascii_lowercase().strip_prefix("content-length:").map(|v|v.trim().parse::<usize>().unwrap())).unwrap_or(0);
                let mut body=vec![0;length];socket.read_exact(&mut body).await.unwrap();
                requests.push(if body.is_empty(){Value::Null}else{serde_json::from_slice(&body).unwrap()});
                if let Some(response)=response {
                    let body=if status==204 {String::new()}else{response.to_string()};
                    socket.write_all(format!("HTTP/1.1 {status} QA\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
                }else{socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n{").await.unwrap();}
            }requests
        });
        (Config::new(profile,"qa-session".into(),"native".into()).unwrap(),task)
    }
    #[tokio::test]
    async fn lost_ack_retries_original_body_then_commits_later_heard_boundary_and_preserves_pause() {
        let (journal,original)=initial();lock(&journal).unwrap().observe(heard("b",3.)).unwrap();lock(&journal).unwrap().observe(heard("c",0.25)).unwrap();
        let (core,task)=server(vec![("zone",200,Some(zone("a","playing",1.8))),
            ("handoff",200,None),("handoff",200,None),("handoff",200,Some(json!({"accepted":true,"applied":false,"zone":zone("b","paused",0.125)}))),
            ("handoff",200,Some(json!({"accepted":true,"zone":zone("c","paused",0.25)}))),
            ("progress",204,Some(Value::Null)),("zone",200,Some(zone("c","paused",0.25)))]).await;
        let result=reconcile(&journal,&core,"z","plan",48000).await.unwrap();let requests=task.await.unwrap();
        for body in &requests[1..4]{assert_eq!(body,&original);}
        assert_eq!(requests[4]["from_playback_id"],"b");assert_eq!(requests[4]["ended"],json!([heard("b",3.)]));assert_eq!(requests[4]["current"],heard("c",0.25));
        assert_ne!(requests[4]["request_id"],original["request_id"]);assert!(requests[5].get("status").is_none());assert_eq!(requests[5]["frames"],12000);
        assert_eq!(result["status"],"paused");assert!(lock(&journal).unwrap().pending.is_none());assert_eq!(lock(&journal).unwrap().heard.len(),1);
    }
    #[tokio::test]
    async fn exhausted_recovery_keeps_identical_request_for_a_later_attempt() {
        let (journal,original)=initial();lock(&journal).unwrap().observe(heard("b",0.5)).unwrap();
        let (core,task)=server(vec![("zone",200,Some(zone("a","paused",1.8))),
            ("handoff",503,Some(Value::Null)),("handoff",503,Some(Value::Null)),("handoff",503,Some(Value::Null)),
            ("zone",200,Some(zone("a","paused",1.8))),("handoff",200,Some(json!({"accepted":true,"zone":zone("b","paused",0.125)}))),
            ("progress",204,Some(Value::Null)),("zone",200,Some(zone("b","paused",0.5)))]).await;
        assert!(reconcile(&journal,&core,"z","plan",48000).await.is_err());assert_eq!(lock(&journal).unwrap().pending.as_ref().unwrap().1,original);
        assert_eq!(reconcile(&journal,&core,"z","plan",48000).await.unwrap()["position"],0.5);
        let requests=task.await.unwrap();for i in [1,2,3,5]{assert_eq!(requests[i],original);}
        assert_eq!(requests[6]["position"],0.5);assert!(requests[6].get("status").is_none());
    }
    #[tokio::test]
    async fn historical_receipt_cannot_resurrect_stop_or_overwrite_new_seek() {
        for latest in [zone("a","stopped",0.),zone("new-seek","paused",17.)] {
            let (journal,_)=initial();let (core,task)=server(vec![("zone",200,Some(latest.clone())),("handoff",200,Some(json!({"accepted":true,"applied":false,"zone":latest})))]).await;
            let result=reconcile(&journal,&core,"z","plan",48000).await.unwrap();assert_eq!(result,latest);assert_eq!(task.await.unwrap().len(),2);
            assert!(lock(&journal).unwrap().pending.is_none());assert!(lock(&journal).unwrap().invalidated);
        }
    }
    #[tokio::test]
    async fn fully_heard_queue_recovers_final_eof_without_replaying_and_retries_the_same_end_identity() {
        let (journal,_)=initial();lock(&journal).unwrap().observe(heard("b",3.)).unwrap();lock(&journal).unwrap().terminal=true;
        let (core,task)=server(vec![("zone",200,Some(zone("a","playing",1.8))),("handoff",200,Some(json!({"accepted":true,"zone":zone("b","playing",0.125)}))),
            ("progress",204,Some(Value::Null)),("ended",200,None),("ended",200,Some(zone("next-queue","playing",0.)))]).await;
        let result=reconcile(&journal,&core,"z","plan",48000).await.unwrap();let requests=task.await.unwrap();
        assert_eq!(requests[2]["position"],3.);assert_eq!(requests[3],requests[4]);assert_eq!(requests[3]["playback_id"],"b");assert_eq!(result["playback_id"],"next-queue");
    }
    #[tokio::test]
    async fn native_output_v2_memory_reconciliation_preserves_version_frames_and_frozen_eof() {
        let journal=Journal::new_timed(heard("a",0.),2).unwrap();let mut first=heard("a",2.);first["position"]=json!(2.001);let mut current=heard("b",0.125);current["position"]=json!(0.1250625);
        lock(&journal).unwrap().observe(first.clone()).unwrap();lock(&journal).unwrap().observe(current.clone()).unwrap();lock(&journal).unwrap().terminal=true;
        let(core,task)=server(vec![("zone",200,Some(zone("a","playing",0.))),("handoff",200,Some(json!({"accepted":true,"zone":zone("b","playing",0.1250625)}))),("progress",204,Some(Value::Null)),("ended",200,None),("ended",200,Some(zone("next","playing",0.)))]).await;
        assert_eq!(reconcile(&journal,&core,"z","plan",48000).await.unwrap()["playback_id"],"next");let bodies=task.await.unwrap();assert_eq!(bodies.len(),5);for body in &bodies[1..]{assert_eq!(body["measured_version"],2);}assert_eq!(bodies[1]["ended"],json!([first]));assert_eq!(bodies[1]["current"],current);assert_eq!(bodies[2]["frames"],6000);assert_eq!(bodies[2]["position"],0.1250625);assert_eq!(bodies[3],bodies[4]);
    }

}
