//! A portable renderer owns its queue independently of Core's home outputs.
use anyhow::{bail, ensure, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Serialize, Deserialize)]
pub struct Queue {
    pub zone: Value,
    #[serde(default)]
    revision:u64,
}
#[derive(Debug, PartialEq)]
pub enum Action { None, Start, Pause, Resume, Seek(f64), Stop, Volume }
impl Queue {
    fn placeholder(id:&Value)->Value {json!({"id":id,"recording_id":"","title":"正在加载歌曲","artist":"","album":"","duration":0.,"format":"unknown","sample_rate":0,"bit_depth":0,"channels":0})}
    pub fn new(endpoint: &str, name: &str) -> Self {
        Self {revision:0,zone:json!({"id":format!("client-zone-{endpoint}"),"endpoint_id":endpoint,"name":name,
            "queue":[],"queue_index":0,"track":null,"playback_id":null,"status":"stopped","position":0.,
            "volume":1.,"muted":false,"repeat":"off","shuffle":false,
            "autoplay":{"enabled":true,"source":"local","fallback_local":true},
            "transitions":rmusic_audio::fade::FadeConfig::default(),"dsp":rmusic_audio::DspConfig::default()})}
    }
    pub fn load(store:&crate::listens::Store,key:&str)->Result<Option<Self>> {
        let Some(mut queue)=store.setting(key,None)?.and_then(|s|serde_json::from_str::<Self>(&s).ok()) else{return Ok(None);};
        if let Some(state)=store.setting(&format!("{key}:state"),None)?.and_then(|s|serde_json::from_str::<Value>(&s).ok()) {
            if state["revision"].as_u64()==Some(queue.revision){if let Some(fields)=state["zone"].as_object(){for(k,v)in fields{if k!="queue"{queue.zone[k]=v.clone();}}}}
        }
        Ok(Some(queue))
    }
    pub fn save(&mut self,store:&crate::listens::Store,key:&str,queue_changed:bool)->Result<()> {
        if queue_changed {
            self.revision+=1;
            store.setting(key,Some(&serde_json::to_string(self)?))?;
        }
        store.setting(&format!("{key}:state"),Some(&json!({"revision":self.revision,"zone":self.state()}).to_string()))?;
        Ok(())
    }
    pub fn revision(&self)->u64 {self.revision}
    pub fn state(&self)->Value {
        Value::Object(self.zone.as_object().unwrap().iter().filter(|(key,_)|*key!="queue").map(|(key,value)|(key.clone(),value.clone())).collect())
    }
    pub fn restart(&mut self) {
        fn complete_track(track:&mut Value) {
            if let Some(fields)=track.as_object_mut() {
                let depth=fields.entry("bit_depth").or_insert(Value::Null);
                if depth.is_null(){*depth=json!(0);}
            }
        }
        complete_track(&mut self.zone["track"]);
        if let Some(tracks)=self.zone["queue"].as_array_mut(){tracks.iter_mut().for_each(complete_track);}
        self.zone["status"]=json!(if self.zone["track"].is_null() {"stopped"}else{"paused"});
        self.zone["playback_id"]=Value::Null;
        self.zone["buffering"]=json!(false);
    }
    pub fn tracks(&self)->&Vec<Value> {self.zone["queue"].as_array().unwrap()}
    pub fn index(&self)->usize {self.zone["queue_index"].as_u64().unwrap_or(0) as usize}
    pub fn next(&self, ended:bool)->Option<usize> {
        if self.tracks().is_empty(){return None;}
        if ended && self.zone["repeat"]=="one" {return Some(self.index());}
        if self.index()+1<self.tracks().len(){Some(self.index()+1)}
        else if self.zone["repeat"]=="all"{Some(0)}else{None}
    }
    fn shuffle_tail(&mut self) {
        let index=self.index();
        let rows=self.zone["queue"].as_array_mut().unwrap();
        if let Some(tail)=rows.get_mut(index.saturating_add(1)..) {
            for i in (1..tail.len()).rev() {
                let other=(uuid::Uuid::new_v4().as_u128()%(i+1) as u128) as usize;
                tail.swap(i,other);
            }
        }
    }
    pub fn select(&mut self,index:usize)->Result<()> {
        ensure!(index<self.tracks().len(),"队列位置已经改变");
        self.zone["queue_index"]=json!(index);
        self.zone["track"]=self.tracks()[index].clone();
        self.zone["position"]=json!(0.);
        self.zone["playback_id"]=json!(uuid::Uuid::new_v4().to_string());
        self.zone["status"]=json!("playing");
        Ok(())
    }
    pub fn command(&mut self,method:&str,action:&str,body:&Value)->Result<Action> {
        let bulk=action.starts_with("queue")||action=="play"&&body["track_id"].is_string()
            ||action.is_empty()&&body.as_object().is_some_and(|fields|fields.len()>1);
        if !bulk{return self.apply(method,action,body);}
        let mut next = self.clone();
        let result = next.apply(method, action, body)?;
        ensure!(next.tracks().len() <= 10000, "队列上限为一万首");
        *self = next;
        Ok(result)
    }
    fn apply(&mut self,method:&str,action:&str,body:&Value)->Result<Action> {
        let result=match action {
            "queue"=>{
                let ids=body["track_ids"].as_array().ok_or_else(||anyhow::anyhow!("缺少歌曲列表"))?;
                ensure!(ids.len()<=10000&&ids.iter().all(|v|v.as_str().is_some_and(|s|!s.is_empty())),"歌曲列表无效");
                fn indexed(tracks:&[Value])->std::collections::HashMap<&str,&Value>{tracks.iter().filter_map(|track|track["id"].as_str().map(|id|(id,track))).collect()}
                let existing=indexed(self.tracks());
                let supplied=indexed(body["tracks"].as_array().map(Vec::as_slice).unwrap_or(&[]));
                let rows:Vec<Value>=ids.iter().map(|id|existing.get(id.as_str().unwrap()).or_else(||supplied.get(id.as_str().unwrap()))
                    .map(|track|(*track).clone()).unwrap_or_else(||Self::placeholder(id))).collect();
                if method=="POST" || body["append"]==true || body["play_next"]==true {
                    let at=if body["play_next"]==true&& !self.zone["track"].is_null(){(self.index()+1).min(self.tracks().len())}else{body["insertion_index"].as_u64().map(|i|i as usize).unwrap_or(self.tracks().len()).min(self.tracks().len())};
                    if at<=self.index()&&!self.tracks().is_empty(){self.zone["queue_index"]=json!(self.index()+rows.len());}
                    self.zone["queue"].as_array_mut().unwrap().splice(at..at,rows);
                    Action::None
                } else {
                    let current=self.zone["track"]["id"].clone();
                    let status=self.zone["status"].clone();
                    self.zone["queue"]=json!(rows);
                    if let Some(index)=body["queue_index"].as_u64(){
                        self.select(index as usize)?;self.zone["status"]=status;
                        if self.zone["status"]=="stopped"{self.zone["playback_id"]=Value::Null;Action::Stop}else{Action::Start}
                    }else if let Some(index)=self.tracks().iter().position(|t|t["id"]==current){
                        self.zone["queue_index"]=json!(index);Action::None
                    }else{
                        self.zone["queue_index"]=json!(0);self.zone["track"]=Value::Null;
                        self.zone["position"]=json!(0.);self.zone["status"]=json!("stopped");self.zone["playback_id"]=Value::Null;Action::Stop
                    }
                }
            }
            "queue/reorder"=>{
                let expected=body["expected_queue"].as_array().ok_or_else(||anyhow::anyhow!("队列已经改变"))?;
                ensure!(self.tracks().iter().map(|t|&t["id"]).eq(expected.iter()),"队列已经改变，请重试");
                let order:Vec<usize>=body["order"].as_array().ok_or_else(||anyhow::anyhow!("缺少顺序"))?.iter().map(|n|n.as_u64().map(|n|n as usize).ok_or_else(||anyhow::anyhow!("顺序无效"))).collect::<Result<_>>()?;
                let mut sorted=order.clone();sorted.sort_unstable();ensure!(sorted==(0..self.tracks().len()).collect::<Vec<_>>(),"顺序无效");
                let current=self.index();let rows=order.iter().map(|i|self.tracks()[*i].clone()).collect::<Vec<_>>();
                self.zone["queue_index"]=json!(order.iter().position(|i|*i==current).unwrap_or(0));self.zone["queue"]=json!(rows);Action::None
            }
            "play"=>{
                if let Some(id)=body["track_id"].as_str(){
                    ensure!(!id.is_empty(),"缺少歌曲 ID");
                    let index=if let Some(index)=self.tracks().iter().position(|t|t["id"]==id){index}else{
                        self.zone["queue"].as_array_mut().unwrap().push(Self::placeholder(&json!(id)));self.tracks().len()-1
                    };
                    self.select(index)?;Action::Start
                }else if body["queue_index"].is_null()&&self.zone["status"]=="paused"&&!self.zone["playback_id"].is_null(){self.zone["status"]=json!("playing");Action::Resume}
                else {self.select(body["queue_index"].as_u64().map(|i|i as usize).unwrap_or(self.index()))?;Action::Start}
            }
            "next"|"ended"=>{
                if let Some(next)=self.next(action=="ended"){self.select(next)?;Action::Start}
                else{self.zone["status"]=json!("stopped");self.zone["playback_id"]=Value::Null;Action::Stop}
            }
            "previous"=>{self.select(self.index().saturating_sub(1))?;Action::Start}
            "pause"=>{self.zone["status"]=json!("paused");Action::Pause}
            "stop"=>{self.zone["status"]=json!("stopped");self.zone["playback_id"]=Value::Null;self.zone["position"]=json!(0.);Action::Stop}
            "seek"=>{
                let pos=body["position"].as_f64().filter(|n|n.is_finite()&&*n>=0.).ok_or_else(||anyhow::anyhow!("播放位置无效"))?;
                ensure!(self.zone["track"]["duration"].as_f64().unwrap_or(0.)<=0.||pos<=self.zone["track"]["duration"].as_f64().unwrap(),"播放位置超出歌曲时长");
                self.zone["position"]=json!(pos);Action::Seek(pos)
            }
            "" if method=="PATCH"=>{
                ensure!(body.is_object(),"设置无效");
                for (k,v) in body.as_object().unwrap() {
                    match k.as_str(){
                        "volume"=>ensure!(v.as_f64().is_some_and(|n|(0. ..=1.).contains(&n)),"音量无效"),
                        "muted"|"shuffle"=>ensure!(v.is_boolean(),"设置无效"),
                        "repeat"=>ensure!(matches!(v.as_str(),Some("off"|"one"|"all")),"循环设置无效"),
                        "autoplay"=>ensure!(v["enabled"].is_boolean()&&v["source"].as_str().is_some_and(|id|!id.is_empty()&&id.len()<=80&&id.bytes().all(|b|b.is_ascii_alphanumeric()||b==b'-'||b==b'_')),"自动续播设置无效"),
                        "transitions"=>{let _:rmusic_audio::fade::FadeConfig=serde_json::from_value(v.clone())?;}
                        _=>bail!("此设置属于家庭音区"),
                    }self.zone[k]=v.clone();
                }Action::Volume
            }
            "dsp" if method=="PUT"=>{let dsp:rmusic_audio::DspConfig=serde_json::from_value(body.clone())?;ensure!(dsp.convolution.is_none(),"本机外出播放暂不加载卷积");self.zone["dsp"]=body.clone();if self.zone["track"].is_null()||self.zone["status"]=="stopped"{Action::None}else{Action::Seek(self.zone["position"].as_f64().unwrap_or(0.))}}
            _=>bail!("未知的本机播放操作"),
        };
        self.zone["updated_at"]=json!(chrono::Utc::now().timestamp_millis());
        if action.is_empty()&&body["shuffle"]==true { self.shuffle_tail(); }
        Ok(result)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn selecting_a_song_and_inserting_before_it_preserves_the_current_instance() {
        let mut q=Queue::new("phone","手机");
        q.command("PUT","queue",&json!({"track_ids":["a","b","a"]})).unwrap();
        assert_eq!(q.command("POST","play",&json!({"track_id":"b"})).unwrap(),Action::Start);
        assert_eq!(q.index(),1);
        let instance=q.zone["playback_id"].clone();
        q.command("POST","queue",&json!({"track_ids":["c"],"insertion_index":0})).unwrap();
        assert_eq!(q.index(),2);assert_eq!(q.zone["playback_id"],instance);assert_eq!(q.zone["track"]["id"],"b");
        assert_eq!(q.command("PUT","queue",&json!({"track_ids":["a","b","c"]})).unwrap(),Action::None);
        assert_eq!(q.index(),1);assert_eq!(q.zone["playback_id"],instance);
        q.command("POST","play",&json!({"track_id":"new"})).unwrap();
        assert_eq!(q.zone["track"]["id"],"new");
        let saved=q.zone.clone();assert!(q.command("POST","play",&json!({"queue_index":500})).is_err());assert_eq!(q.zone,saved);
    }
    #[test] fn queue_controls_survive_core_loss_and_preserve_duplicates() {
        let mut q=Queue::new("phone","手机");q.command("PUT","queue",&json!({"track_ids":["a","b","a"]})).unwrap();
        q.command("POST","play",&Value::Null).unwrap();
        assert_eq!(q.command("POST","pause",&Value::Null).unwrap(),Action::Pause);
        assert_eq!(q.command("POST","play",&Value::Null).unwrap(),Action::Resume);
        q.command("POST","queue/reorder",&json!({"expected_queue":["a","b","a"],"order":[2,0,1]})).unwrap();assert_eq!(q.index(),1);
        let mut restored:Queue=serde_json::from_str(&serde_json::to_string(&q).unwrap()).unwrap();restored.restart();
        assert_eq!(restored.zone["status"],"paused");assert_eq!(restored.zone["track"]["id"],"a");
        assert_eq!(restored.command("POST","seek",&json!({"position":12.5})).unwrap(),Action::Seek(12.5));
        assert!(restored.command("POST","queue/reorder",&json!({"expected_queue":["a","a","b"],"order":[0,0,2]})).is_err());
    }
    #[test] fn position_and_volume_checkpoints_do_not_rewrite_a_large_queue() {
        let root=tempfile::tempdir().unwrap();let store=crate::listens::Store::open(root.path()).unwrap();
        let mut queue=Queue::new("phone","手机");
        queue.command("PUT","queue",&json!({"track_ids":(0..10000).map(|i|format!("track-{i}")).collect::<Vec<_>>()})).unwrap();
        queue.select(5000).unwrap();queue.save(&store,"large",true).unwrap();
        let original=store.setting("large",None).unwrap().unwrap();
        queue.zone["position"]=json!(42.25);queue.command("PATCH","",&json!({"volume":0.4})).unwrap();queue.save(&store,"large",false).unwrap();
        assert_eq!(store.setting("large",None).unwrap().unwrap(),original);
        let restored=Queue::load(&store,"large").unwrap().unwrap();assert_eq!(restored.index(),5000);assert_eq!(restored.zone["position"],42.25);assert_eq!(restored.zone["volume"],0.4);
        assert!(serde_json::to_vec(&queue.state()).unwrap().len()<4096);
        let old=store.setting("large:state",None).unwrap().unwrap();
        queue.zone["position"]=json!(3.);queue.save(&store,"large",true).unwrap();store.setting("large:state",Some(&old)).unwrap();
        assert_eq!(Queue::load(&store,"large").unwrap().unwrap().zone["position"],3.);
    }
}
