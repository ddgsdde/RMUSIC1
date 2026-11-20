use std::{collections::HashMap,future::Future,sync::Mutex,time::{Duration,Instant}};
use tokio::sync::watch;

const CANCELLED:&str="请求已取消";
struct Entry {cancel:watch::Sender<bool>,running:bool,created:Instant}
#[derive(Default)]
pub struct Requests(Mutex<HashMap<String,Entry>>);
fn valid(id:&str)->Result<(),String>{
    if id.is_empty()||id.len()>128||!id.bytes().all(|b|b.is_ascii_alphanumeric()||b==b'-') {return Err("无效请求标识".into());}
    Ok(())
}
fn prune(entries:&mut HashMap<String,Entry>){entries.retain(|_,entry|entry.running||entry.created.elapsed()<Duration::from_secs(30));}
struct Guard<'a>{requests:&'a Requests,id:String}
impl Drop for Guard<'_>{fn drop(&mut self){if let Ok(mut entries)=self.requests.0.lock(){entries.remove(&self.id);}}}
impl Requests {
    pub fn cancel(&self,id:String)->Result<(),String>{
        valid(&id)?;
        let mut entries=self.0.lock().map_err(|_|"请求状态锁不可用")?;prune(&mut entries);
        if let Some(entry)=entries.get(&id){entry.cancel.send_replace(true);}
        else {
            if entries.len()>=256{return Err("取消队列已满".into());}
            let (cancel,_)=watch::channel(true);
            entries.insert(id,Entry{cancel,running:false,created:Instant::now()});
        }
        Ok(())
    }
    pub async fn run<T>(&self,id:Option<String>,operation:impl Future<Output=Result<T,String>>)->Result<T,String>{
        let Some(id)=id else{return operation.await;};valid(&id)?;
        let mut cancellation={
            let mut entries=self.0.lock().map_err(|_|"请求状态锁不可用")?;prune(&mut entries);
            if let Some(entry)=entries.get(&id){return Err(if *entry.cancel.borrow(){CANCELLED}else{"请求标识正在使用"}.into());}
            if entries.len()>=256||entries.values().filter(|entry|entry.running).count()>=128{return Err("并发请求过多".into());}
            let (cancel,receiver)=watch::channel(false);
            entries.insert(id.clone(),Entry{cancel,running:true,created:Instant::now()});receiver
        };
        let _guard=Guard{requests:self,id};
        tokio::select!{biased;
            _=cancellation.changed()=>Err(CANCELLED.into()),
            result=operation=>result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc,atomic::{AtomicBool,Ordering}};
    #[tokio::test]
    async fn cancellation_before_registration_never_polls_the_request(){
        let requests=Requests::default();requests.cancel("early".into()).unwrap();let polled=AtomicBool::new(false);
        assert_eq!(requests.run(Some("early".into()),async{polled.store(true,Ordering::Relaxed);Ok(())}).await.unwrap_err(),CANCELLED);
        assert!(!polled.load(Ordering::Relaxed));
    }
    #[tokio::test]
    async fn cancellation_is_scoped_and_cleans_up_running_requests(){
        let requests=Arc::new(Requests::default());let running=requests.clone();let started=Arc::new(tokio::sync::Notify::new());let ready=started.clone();
        let task=tokio::spawn(async move{running.run(Some("old".into()),async{ready.notify_one();std::future::pending::<Result<(),String>>().await}).await});
        started.notified().await;
        assert_eq!(requests.run(Some("old".into()),async{Ok(())}).await.unwrap_err(),"请求标识正在使用");
        requests.cancel("old".into()).unwrap();assert_eq!(task.await.unwrap().unwrap_err(),CANCELLED);
        assert_eq!(requests.run(Some("new".into()),async{Ok(42)}).await.unwrap(),42);assert!(requests.0.lock().unwrap().is_empty());
    }
    #[tokio::test]
    async fn task_drop_releases_registration_and_cancel_tombstones_are_bounded(){
        let requests=Arc::new(Requests::default());let running=requests.clone();let started=Arc::new(tokio::sync::Notify::new());let ready=started.clone();
        let task=tokio::spawn(async move{running.run(Some("aborted".into()),async{ready.notify_one();std::future::pending::<Result<(),String>>().await}).await});
        started.notified().await;task.abort();assert!(task.await.unwrap_err().is_cancelled());assert!(requests.0.lock().unwrap().is_empty());
        for index in 0..256{requests.cancel(format!("cancel-{index}")).unwrap();}
        assert!(requests.cancel("overflow".into()).is_err());
        for entry in requests.0.lock().unwrap().values_mut(){entry.created=Instant::now()-Duration::from_secs(31);}
        assert_eq!(requests.run(Some("after-expiry".into()),async{Ok(42)}).await.unwrap(),42);
        assert!(requests.run(Some("../invalid".into()),async{Ok(())}).await.is_err());
        assert_eq!(requests.run(None,async{Ok(7)}).await.unwrap(),7);
    }
}
