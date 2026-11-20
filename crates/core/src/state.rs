use crate::{db::Db, models::Zone};
use serde_json::{json, Value};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use tokio::sync::{broadcast, mpsc, Mutex, RwLock};

#[derive(Clone)]
pub struct Config {
    pub data: PathBuf,
    pub web: PathBuf,
    pub base_path: String,
    pub public_url: Option<String>,
    pub plugin_dir: PathBuf,
    pub python: String,
}
#[derive(Clone)]
pub struct Ticket {
    pub scope: String,
    pub expires: i64,
    pub owner: Option<String>,
    pub account: Option<String>,
    pub(crate) recipe: Option<crate::render_media::Recipe>,
}
#[derive(Clone)]
pub struct AppState {
    pub remote:Arc<rmusic_remote::runtime::Runtime>,
    pub(crate) visualizations: Arc<Mutex<crate::visualization::Registry>>,
    pub llm: Arc<crate::llm::Runtime>,
    pub cache_enabled: Arc<std::sync::atomic::AtomicBool>,
    pub core_id: Arc<String>,
    pub queue_epoch: Arc<String>,
    pub discovery: Arc<std::sync::Mutex<Option<Arc<rmusic_discovery::Discovery>>>>,
    pub db: Db,
    pub config: Config,
    pub bootstrap: Arc<String>,
    pub events: broadcast::Sender<Value>,
    pub(crate) loudness: Arc<crate::loudness::Runtime>,
    pub zones: Arc<RwLock<HashMap<String, Zone>>>,
    pub endpoints: Arc<RwLock<HashMap<String, Value>>>,
    pub bridges: Arc<RwLock<HashMap<String, mpsc::Sender<Value>>>>,
    pub(crate) clock_filters: Arc<RwLock<HashMap<String,(mpsc::Sender<Value>,crate::clock::Filter)>>>,
    pub bridge_owners: Arc<RwLock<HashMap<String, String>>>,
    pub browser_owners: Arc<RwLock<HashMap<String, String>>>,
    pub sequences: Arc<Mutex<HashMap<String, crate::sequence::Plan>>>,
    pub sync_starts: Arc<Mutex<crate::sync_start::Registry>>,
    pub tickets: Arc<Mutex<HashMap<String, Ticket>>>,
    pub plugin: crate::plugin::PluginManager,
    pub scanner: Arc<Mutex<()>>,
    pub zone_lock: Arc<Mutex<()>>,
    pub portable: Arc<crate::portable::Registry>,
    pub device_group_lock: Arc<Mutex<()>>,
    pub impulse_lock: Arc<Mutex<()>>,
    pub autoplay_batches: Arc<Mutex<HashMap<String,crate::autoplay::Batch>>>,
    pub(crate) daily_recommendation_locks: Arc<Mutex<HashMap<String,Arc<Mutex<()>>>>>,
    pub(crate) daily_locks: Arc<crate::radio::personal::DailyLocks>,
    pub(crate) radio_cloud: Arc<crate::radio::CloudCache>,
    pub(crate) discovery_snapshots: Arc<Mutex<HashMap<crate::radio::DiscoveryKey,crate::radio::DiscoveryEntry>>>,
    pub autoplay_slots: Arc<tokio::sync::Semaphore>,
    pub cache_lock: Arc<Mutex<()>>,
    pub resources: Arc<crate::resources::Resources>,
    pub traffic: Arc<crate::traffic::Traffic>,
    pub(crate) cache_buffers: Arc<Mutex<HashMap<String,crate::media::WriteBuffer>>>,
    pub(crate) playback_cache: Arc<crate::media::PreviewCache>,
    pub cache_active: Arc<Mutex<HashMap<String, usize>>>,
    pub cache_directory: Arc<RwLock<PathBuf>>,
    pub login_attempts: Arc<Mutex<HashMap<String, (u32, i64)>>>,
    pub http: reqwest::Client,
    pub resolved_sources: Arc<Mutex<HashMap<String,crate::catalog::ResolvedSource>>>,
    pub source_resolutions: Arc<Mutex<HashMap<String,std::sync::Weak<Mutex<()>>>>>,
    pub foreground_requests: Arc<std::sync::atomic::AtomicUsize>,
    pub foreground_last_activity: Arc<std::sync::atomic::AtomicI64>,
}
impl AppState {
    /// Re-evaluate accepted samples at the time of use; a published best sample may have expired.
    pub(crate) async fn host_clock_estimate(&self,id:&str,owner:&mpsc::Sender<Value>,now:u64)->Value {
        self.clock_filters.read().await.get(id).filter(|(tx,_)|tx.same_channel(owner)).map(|(_,filter)|filter.estimate(now)).unwrap_or(Value::Null)
    }
    pub async fn new(db: Db, config: Config, bootstrap: String) -> anyhow::Result<Self> {
        let core_id=db.run(|c|{
            c.execute("INSERT OR IGNORE INTO settings(key,value) VALUES('core_id',?1)",[serde_json::to_string(&uuid::Uuid::new_v4().to_string())?])?;
            let value:String=c.query_row("SELECT value FROM settings WHERE key='core_id'",[],|r|r.get(0))?;
            Ok(serde_json::from_str::<String>(&value)?)
        }).await?;
        let zones=db.run(|c| {
            crate::device_groups::schema(c)?;
            crate::sync_start::restart(c)?;
            c.execute("UPDATE jobs SET status='interrupted' WHERE status='running'",[])?;
            c.execute("UPDATE jobs SET status='cancelled',phase='旧试听补齐任务已取消',updated_at=unixepoch()
                WHERE kind='cache_fill' AND status IN ('pending','interrupted','paused','waiting') AND json_extract(payload,'$.origin')='playback'",[])?;
            c.execute("UPDATE downloads SET status='interrupted',error='核心重启，重新下载可恢复' WHERE status='downloading'",[])?;
            let mut s=c.prepare("SELECT data FROM zones")?;
            let data=s.query_map([],|r|r.get::<_,String>(0))?.collect::<Result<Vec<_>,_>>()?;
            drop(s);let tx=c.transaction()?;let mut zones=HashMap::new();
            for old in data.into_iter().filter_map(|v|serde_json::from_str::<Zone>(&v).ok()) {
                let z=crate::sequence::recovery::restart(&tx,old)?;zones.insert(z.id.clone(),z);
            }
            tx.commit()?;Ok(zones)
        }).await?;
        let cache_directory=db.run(|c|{use rusqlite::OptionalExtension;Ok(c.query_row("SELECT value FROM settings WHERE key='cache_directory'",[],|r|r.get::<_,String>(0)).optional()?.and_then(|s|serde_json::from_str::<String>(&s).ok()))}).await?;
        let cache_directory=cache_directory.or_else(||std::env::var("RMUSIC_CACHE_DIR").ok()).map(PathBuf::from).unwrap_or_else(||config.data.join("cache"));
        tokio::fs::create_dir_all(&cache_directory).await?;let cache_directory=tokio::fs::canonicalize(cache_directory).await?;
        let (events, _) = broadcast::channel(256);
        let mut endpoints = HashMap::new();
        endpoints.insert("browser-local".into(),json!({"id":"browser-local","name":"当前客户端","kind":"browser","online":true,"verified":true,"capabilities":{"range":true,"dsp":"browser","strict_sync":false}}));
        let foreground_requests=Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let foreground_last_activity=Arc::new(std::sync::atomic::AtomicI64::new(0));
        let traffic=Arc::new(crate::traffic::Traffic::default());
        let (policy,songs)=db.run(|c|{use rusqlite::OptionalExtension;let read=|key:&str|c.query_row("SELECT value FROM settings WHERE key=?1",[key],|r|r.get::<_,String>(0)).optional();Ok((read("provider_request_limits")?,read("song_fetch_concurrency")?))}).await?;
        if let Some(policy)=policy.and_then(|v|serde_json::from_str::<crate::traffic::Policy>(&v).ok()).filter(|v|v.valid()){*traffic.policy.write().unwrap()=policy;}
        if let Some(songs)=songs.and_then(|v|serde_json::from_str::<usize>(&v).ok()).filter(|n|(1..=16).contains(n)){traffic.song_limit.store(songs,std::sync::atomic::Ordering::Release);}
        // An explicit process override keeps isolated development Cores from writing
        // audio to a space-constrained Mac. Normal deployments still default on.
        let cache_enabled=!matches!(std::env::var("RMUSIC_CACHE_ENABLED").ok().as_deref(),Some("0"|"false"));
        db.run(move|c|{c.execute("INSERT INTO settings(key,value)VALUES('cache_enabled',?1) ON CONFLICT(key)DO UPDATE SET value=excluded.value",[cache_enabled.to_string()])?;Ok(())}).await?;
        let s = Self {
            remote:Arc::new(rmusic_remote::runtime::Runtime::open(&config.data.join("remote-access"))?),
            visualizations: Default::default(),
            llm: Default::default(),
            cache_enabled: Arc::new(std::sync::atomic::AtomicBool::new(cache_enabled)),
            core_id: Arc::new(core_id),
            queue_epoch: Arc::new(uuid::Uuid::new_v4().to_string()),
            discovery: Default::default(),
            db,
            plugin: crate::plugin::PluginManager::new(&config,foreground_requests.clone(),foreground_last_activity.clone(),traffic.clone()),
            config,
            bootstrap: Arc::new(bootstrap),
            events,
            loudness: Default::default(),
            zones: Arc::new(RwLock::new(zones)),
            endpoints: Arc::new(RwLock::new(endpoints)),
            bridges: Default::default(),
            clock_filters: Default::default(),
            bridge_owners: Default::default(),
            browser_owners: Default::default(),
            sequences: Default::default(),
            sync_starts: Default::default(),
            tickets: Default::default(),
            scanner: Default::default(),
            zone_lock: Default::default(),
            portable: Default::default(),
            device_group_lock: Default::default(),
            impulse_lock: Default::default(),
            autoplay_batches: Default::default(),
            daily_recommendation_locks: Default::default(),
            daily_locks: Default::default(),
            radio_cloud: Default::default(),
            discovery_snapshots: Default::default(),
            autoplay_slots: Arc::new(tokio::sync::Semaphore::new(4)),
            cache_lock: Default::default(),
            resources: Default::default(),
            traffic,
            cache_buffers: Default::default(),
            playback_cache: Default::default(),
            cache_active: Default::default(),
            cache_directory: Arc::new(RwLock::new(cache_directory)),
            login_attempts: Default::default(),
            resolved_sources: Default::default(),
            source_resolutions: Default::default(),
            foreground_requests,
            foreground_last_activity,
            http: reqwest::Client::builder()
                .pool_idle_timeout(std::time::Duration::from_secs(30))
                .pool_max_idle_per_host(4)
                .connect_timeout(std::time::Duration::from_secs(10))
                .read_timeout(std::time::Duration::from_secs(30))
                .build()?,
        };
        let removed_devices=s.db.run(|c|Ok(c.query_row("SELECT EXISTS(SELECT 1 FROM devices WHERE removed=1)",[],|r|r.get::<_,bool>(0))?)).await?;
        if s.zones.read().await.is_empty()&&!removed_devices {
            s.save_zone(Zone::new(
                "zone-local".into(),
                "本机".into(),
                "browser-local".into(),
            ))
            .await?;
        }
        crate::device_groups::restore(&s).await?;
        s.refresh_queue_metadata().await?;
        Ok(s)
    }
    pub async fn cache_dir(&self)->PathBuf{self.cache_directory.read().await.clone()}
    pub async fn portable_dir(&self)->PathBuf{self.setting("portable_directory",json!(self.cache_dir().await.join("portable"))).await.as_str().map(PathBuf::from).unwrap_or_else(||self.config.data.join("music-cache"))}
    pub async fn save_zone(&self, zone: Zone) -> anyhow::Result<()> {
        self.save_zone_action(zone,None).await
    }
    pub(crate) async fn revise_queues(&self, zones: &mut [Zone]) {
        let current = self.zones.read().await;
        for zone in zones {
            Self::revise_queue(zone, current.get(&zone.id));
        }
    }
    pub(crate) fn revise_queue(zone: &mut Zone, old: Option<&Zone>) {
        if let Some(old)=old {
            let changed=old.queue.len()!=zone.queue.len() || old.queue.iter().zip(&zone.queue).any(|(a,b)|a.id!=b.id);
            zone.queue_revision=zone.queue_revision.max(old.queue_revision.saturating_add(u64::from(changed)));
        } else if !zone.queue.is_empty() { zone.queue_revision=zone.queue_revision.max(1); }
    }

    pub async fn save_zone_action(&self, mut zone: Zone, action:Option<&str>) -> anyhow::Result<()> {
        self.revise_queues(std::slice::from_mut(&mut zone)).await;
        let id = zone.id.clone();
        let value = serde_json::to_string(&zone)?;
        let refreshing=crate::sequence::refresh_queue(self,&zone,action).await;
        let saved_refresh=refreshing.clone();
        let saved=zone.clone();let action=action.map(str::to_owned);
        let revoked=self.db.run(move |c|{let tx=c.transaction()?;
            tx.execute("INSERT INTO zones(id,data) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET data=excluded.data",[id,value])?;
            let revoked=if let Some(plan)=saved_refresh {crate::sequence::recovery::advance_plan(&tx,&saved,&plan)?;false}
                else {crate::sequence::recovery::reconcile(&tx,&saved,action.as_deref())?};tx.commit()?;Ok(revoked)}).await?;
        if let Some(plan)=refreshing {crate::sequence::publish_queue_refresh(self,&zone,plan).await;}
        else {
            if revoked {crate::sequence::invalidate_zone(self,&zone.id,Some(&zone)).await;}
            crate::sequence::invalidate_if_changed(self, &zone).await;
        }
        let zone_id=zone.id.clone();let queue_revision=zone.queue_revision;
        self.zones.write().await.insert(zone.id.clone(), zone);
        self.emit("zones_changed", json!({"id":zone_id,"queue_revision":queue_revision}));
        self.loudness.wake.notify_one();
        Ok(())
    }
    pub async fn publish_progress(&self,zone:Zone){
        if let Err(e)=crate::stats::record_progress(self,&zone).await{tracing::warn!(error=%e,"playback statistics update failed");}
        self.progress_event(zone).await;
    }
    pub async fn publish_device_progress(&self,zone:Zone,frames:u64,rate:u32)->bool{
        match crate::stats::record_device_progress(self,&zone,frames,rate).await {
            Ok(true)=>{self.progress_event(zone).await;true},
            Ok(false)=>false,Err(error)=>{tracing::warn!(%error,"measured playback statistics update failed");false}
        }
    }
    pub async fn publish_output_progress(&self,zone:Zone,frames:u64,rate:u32)->anyhow::Result<bool> {
        if !crate::stats::output::progress(self,&zone,frames,rate,false).await?{return Ok(false);}
        self.progress_event(zone).await;Ok(true)
    }
    async fn progress_event(&self,zone:Zone){
        let event=json!({"id":zone.id,"playback_id":zone.playback_id,"position":zone.position,"status":zone.status,"updated_at":zone.updated_at});
        self.zones.write().await.insert(zone.id.clone(),zone);self.emit("zone_progress",event);
    }
    pub async fn publish_control_progress(&self,zone:Zone)->anyhow::Result<()> {
        crate::stats::control_status(self,&zone).await?;self.progress_event(zone).await;Ok(())
    }
    pub async fn refresh_queue_metadata(&self)->anyhow::Result<()> {
        let _lock=self.zone_lock.lock().await;
        let zones:Vec<_>=self.zones.read().await.values().cloned().collect();
        for mut zone in zones {
            let old=serde_json::to_value(&zone.queue)?;let queue=zone.queue.clone();
            zone.queue=self.db.run(move |c|{use rusqlite::OptionalExtension;let mut statement=c.prepare("SELECT * FROM tracks WHERE id=?1")?;queue.into_iter().map(|track|Ok(statement.query_row([&track.id],crate::models::Track::from_row).optional()?.unwrap_or(track))).collect::<anyhow::Result<Vec<_>>>()}).await?;
            if serde_json::to_value(&zone.queue)?!=old {zone.queue_revision=zone.queue_revision.saturating_add(1);zone.track=zone.queue.get(zone.queue_index).cloned();self.save_zone(zone).await?;}
        }Ok(())
    }
    pub async fn zone_list(&self) -> Value {
        let mut v: Vec<_> = self.zones.read().await.values().cloned().collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        json!({"items":v})
    }
    pub fn emit(&self, kind: &str, data: Value) {
        let _ = self.events.send(json!({"type":kind,"data":data}));
    }
    pub async fn ticket(&self, scope: String, seconds: i64) -> String {
        self.ticket_for(scope,seconds,None).await
    }
    pub async fn ticket_for(&self, scope: String, seconds: i64, owner:Option<String>) -> String {
        self.ticket_with_recipe(scope, seconds, owner, None).await
    }
    pub(crate) async fn ticket_with_recipe(&self, scope: String, seconds: i64, owner: Option<String>, recipe: Option<crate::render_media::Recipe>) -> String {
        let t = crate::auth::random_token();
        let account=if let Some(owner)=owner.as_ref(){crate::auth::authenticate_hash(self,owner.clone()).await.ok().filter(|a|a.role=="external").map(|a|a.id)}else{None};
        let mut map = self.tickets.lock().await;
        let now = chrono::Utc::now().timestamp();
        map.retain(|_, v| v.expires > now);
        if let Some(account)=account.as_ref(){if map.values().filter(|v|v.account.as_ref()==Some(account)).count()>=512{if let Some(key)=map.iter().filter(|(_,v)|v.account.as_ref()==Some(account)).min_by_key(|(_,v)|v.expires).map(|(k,_)|k.clone()){map.remove(&key);}}}
        if map.len() >= 10000 {
            let victim=map.iter().filter(|(_,v)|v.account.is_some()).min_by_key(|(_,v)|v.expires).map(|(k,_)|k.clone());
            if let Some(key)=victim{map.remove(&key);}else if account.is_some(){return t;}else if let Some(key)=map.iter().min_by_key(|(_,v)|v.expires).map(|(k,_)|k.clone()){map.remove(&key);}
        }
        map.insert(
            crate::auth::hash(&t),
            Ticket {
                scope,
                expires: now + seconds,
                owner,
                account,
                recipe,
            },
        );
        t
    }
    pub async fn check_ticket(&self, t: &str, scope: &str) -> bool {
        let ticket=self.tickets.lock().await.get(&crate::auth::hash(t)).cloned();
        match ticket {Some(v) if v.scope==scope&&v.expires>chrono::Utc::now().timestamp()=>match v.owner{Some(owner)=>self.session_active(owner).await,None=>true},_=>false}
    }
    pub async fn session_active(&self,hash:String)->bool {
        crate::auth::authenticate_hash(self,hash).await.is_ok()
    }
    pub async fn setting(&self, key: &str, default: Value) -> Value {
        let k = key.to_string();
        self.db
            .read(move |c| {
                use rusqlite::OptionalExtension;
                Ok(
                    c.query_row("SELECT value FROM settings WHERE key=?1", [k], |r| {
                        r.get::<_, String>(0)
                    })
                    .optional()?
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or(default),
                )
            })
            .await
            .unwrap_or(Value::Null)
    }
}
