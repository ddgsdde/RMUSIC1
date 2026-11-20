//! Shared connector pacing and song transfer admission. Waiting work owns no network request.
use serde::{Deserialize,Serialize};
use std::{collections::{HashMap,HashSet},sync::{Arc,Mutex,RwLock},time::{Duration,Instant}};
use tokio::sync::Notify;

tokio::task_local! { pub static BACKGROUND: bool; }
pub fn background()->bool {BACKGROUND.try_with(|value|*value).unwrap_or(false)}

#[derive(Clone,Debug,Deserialize,Serialize)]
#[serde(default,deny_unknown_fields)]
pub struct Policy {
    pub search_ms:u64,pub resolve_ms:u64,pub lyrics_ms:u64,pub playlists_ms:u64,
    pub recommendations_ms:u64,pub metadata_ms:u64,pub comments_ms:u64,pub cooldown_seconds:u64,
}
impl Default for Policy {
    fn default()->Self {Self{search_ms:350,resolve_ms:150,lyrics_ms:350,playlists_ms:600,recommendations_ms:1000,metadata_ms:1100,comments_ms:1000,cooldown_seconds:60}}
}
impl Policy {
    pub fn valid(&self)->bool {[self.search_ms,self.resolve_ms,self.lyrics_ms,self.playlists_ms,self.recommendations_ms,self.metadata_ms,self.comments_ms].iter().all(|n|(50..=60000).contains(n))&&(1..=3600).contains(&self.cooldown_seconds)}
    fn interval(&self,category:&str)->Duration {Duration::from_millis(match category {"search"=>self.search_ms,"resolve"|"qualities"=>self.resolve_ms,"lyrics"=>self.lyrics_ms,"recommendations"=>self.recommendations_ms,"metadata"=>self.metadata_ms,"comments"=>self.comments_ms,_=>self.playlists_ms})}
}
fn category(method:&str)->&str {match method {"related_catalog"|"artist_albums"|"album_catalog"|"extension_invoke"=>"metadata","qualities"=>"resolve","playlist_tracks"|"playlist_snapshot"|"playlist_apply"|"sync"=>"playlists",other=>other}}
fn limited(method:&str)->bool {matches!(method,"extension_invoke"|"related_catalog"|"artist_albums"|"album_catalog"|"search"|"resolve"|"qualities"|"lyrics"|"playlists"|"playlist_tracks"|"playlist_snapshot"|"playlist_apply"|"sync"|"recommendations"|"metadata"|"comments")}
#[derive(Clone)]
struct Waiter {id:u64,provider:String,key:String,category:String,priority:u8}
#[derive(Default)]
struct State {
    serial:u64,queue:Vec<Waiter>,active:HashSet<String>,next:HashMap<(String,String),Instant>,cooldown:HashMap<String,Instant>,
    downloads:HashMap<String,(usize,usize)>,download_queue:Vec<(u64,String,bool)>,
}
pub struct Traffic {pub policy:RwLock<Policy>,pub song_limit:std::sync::atomic::AtomicUsize,state:Mutex<State>,changed:Notify}
impl Default for Traffic {fn default()->Self {Self{policy:RwLock::new(Policy::default()),song_limit:std::sync::atomic::AtomicUsize::new(2),state:Mutex::new(State::default()),changed:Notify::new()}}}
struct Waiting {traffic:Arc<Traffic>,id:u64}
impl Drop for Waiting {fn drop(&mut self){let mut state=self.traffic.state.lock().unwrap();state.queue.retain(|w|w.id!=self.id);state.download_queue.retain(|w|w.0!=self.id);drop(state);self.traffic.changed.notify_waiters();}}
pub struct Request {traffic:Arc<Traffic>,provider:String,key:String,category:String,paced:bool}
impl Drop for Request {fn drop(&mut self){let mut state=self.traffic.state.lock().unwrap();state.active.remove(&self.key);if self.paced {let delay=self.traffic.policy.read().unwrap().interval(&self.category);state.next.insert((self.provider.clone(),self.category.clone()),Instant::now()+delay);}drop(state);self.traffic.changed.notify_waiters();}}
pub struct Song {traffic:Arc<Traffic>,key:String,background:bool}
impl Drop for Song {fn drop(&mut self){let mut state=self.traffic.state.lock().unwrap();if let Some(count)=state.downloads.get_mut(&self.key){if self.background{count.1-=1;}else{count.0-=1;}if count.0+count.1==0{state.downloads.remove(&self.key);}}drop(state);self.traffic.changed.notify_waiters();}}
impl Traffic {
    pub async fn request(self:&Arc<Self>,provider:&str,key:&str,method:&str,background:bool)->Request {
        let waiter={let mut state=self.state.lock().unwrap();state.serial+=1;let waiter=Waiter{id:state.serial,provider:provider.into(),key:key.into(),category:category(method).into(),priority:if background{2}else if matches!(method,"resolve"|"qualities"){0}else{1}};state.queue.push(waiter.clone());waiter};
        let _waiting=Waiting{traffic:self.clone(),id:waiter.id};
        loop {
            let changed=self.changed.notified();tokio::pin!(changed);changed.as_mut().enable();
            {
                let mut state=self.state.lock().unwrap();let now=Instant::now();
                let next=state.queue.iter().filter(|w|!state.active.contains(&w.key)&&(!limited(&w.category)||state.cooldown.get(&w.provider).is_none_or(|at|*at<=now))&&state.next.get(&(w.provider.clone(),w.category.clone())).is_none_or(|at|*at<=now)).min_by_key(|w|(w.priority,w.id)).map(|w|w.id);
                if next==Some(waiter.id){state.queue.retain(|w|w.id!=waiter.id);state.active.insert(waiter.key.clone());return Request{traffic:self.clone(),provider:waiter.provider,key:waiter.key,category:waiter.category,paced:limited(method)};}
            }
            tokio::select!{_=changed.as_mut()=>{},_=tokio::time::sleep(Duration::from_millis(50))=>{}}
        }
    }
    pub fn cooldown(&self,provider:&str){let seconds=self.policy.read().unwrap().cooldown_seconds;self.state.lock().unwrap().cooldown.insert(provider.into(),Instant::now()+Duration::from_secs(seconds));self.changed.notify_waiters();}
    pub fn cooling_down(&self)->bool {self.state.lock().unwrap().cooldown.values().any(|at|*at>Instant::now())}
    pub fn playback_waiting(&self)->bool {self.state.lock().unwrap().download_queue.iter().any(|w|!w.2)}
    pub async fn song(self:&Arc<Self>,key:&str,background:bool)->Song {
        let id={let mut state=self.state.lock().unwrap();state.serial+=1;let id=state.serial;state.download_queue.push((id,key.into(),background));id};
        let _waiting=Waiting{traffic:self.clone(),id};self.changed.notify_waiters();
        loop {
            let changed=self.changed.notified();tokio::pin!(changed);changed.as_mut().enable();
            {
                let mut state=self.state.lock().unwrap();let limit=self.song_limit.load(std::sync::atomic::Ordering::Acquire);
                let candidate=state.download_queue.iter().filter(|w|!w.2||!state.download_queue.iter().any(|other|!other.2)).min_by_key(|w|(w.2,w.0)).map(|w|w.0);
                if state.downloads.contains_key(key)||candidate==Some(id)&&state.downloads.len()<limit {
                    state.download_queue.retain(|w|w.0!=id);let count=state.downloads.entry(key.into()).or_default();if background{count.1+=1;}else{count.0+=1;}
                    return Song{traffic:self.clone(),key:key.into(),background};
                }
            }
            tokio::select!{_=changed.as_mut()=>{},_=tokio::time::sleep(Duration::from_millis(50))=>{}}
        }
    }
}

#[cfg(test)] mod tests {
    use super::*;
    #[tokio::test] async fn playback_overtakes_background_and_cancelled_waiter_is_removed(){
        let traffic=Arc::new(Traffic::default());traffic.song_limit.store(1,std::sync::atomic::Ordering::Release);
        let first=traffic.song("first",true).await;
        let t=traffic.clone();let background=tokio::spawn(async move{t.song("queued",true).await});tokio::task::yield_now().await;
        let t=traffic.clone();let foreground=tokio::spawn(async move{t.song("playing",false).await});tokio::task::yield_now().await;
        assert!(traffic.playback_waiting());drop(first);let playing=tokio::time::timeout(Duration::from_secs(1),foreground).await.unwrap().unwrap();assert!(!background.is_finished());
        let same=traffic.song("playing",false).await;drop(playing);assert!(!background.is_finished());drop(same);
        let queued=tokio::time::timeout(Duration::from_secs(1),background).await.unwrap().unwrap();let t=traffic.clone();let cancelled=tokio::spawn(async move{t.song("cancelled",false).await});tokio::task::yield_now().await;cancelled.abort();let _=cancelled.await;assert!(!traffic.playback_waiting());drop(queued);
    }
    #[tokio::test] async fn request_spacing_and_playback_priority_share_process_admission(){
        let traffic=Arc::new(Traffic::default());traffic.policy.write().unwrap().search_ms=80;
        let first=traffic.request("qq","qq","search",false).await;
        let t=traffic.clone();let bg=tokio::spawn(async move{t.request("qq","qq","playlists",true).await});tokio::task::yield_now().await;
        let t=traffic.clone();let fg=tokio::spawn(async move{t.request("qq","qq","resolve",false).await});tokio::task::yield_now().await;
        let at=Instant::now();drop(first);let fg=tokio::time::timeout(Duration::from_secs(1),fg).await.unwrap().unwrap();assert!(!bg.is_finished());drop(fg);drop(bg.await.unwrap());
        drop(traffic.request("qq","qq","search",false).await);assert!(at.elapsed()>=Duration::from_millis(80));
    }
}
