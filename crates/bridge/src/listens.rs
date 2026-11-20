//! Shared native client/bridge listening journal; SQLite writes run on a separate worker.
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant},
};
#[cfg(test)]
#[path = "listens/tests.rs"]
mod tests;
mod measured;
mod output;
mod collector;
pub mod sync;
pub mod cursor;
pub mod group;
pub use collector::{Collector,Source};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OutputRange {pub start:u64,pub end:u64,pub sample_rate:u32}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Segment {
    pub start:f64,pub end:f64,pub last_at:i64,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub output:Option<OutputRange>,
}
fn legacy_timing()->u32{1}
fn is_legacy_timing(version:&u32)->bool{*version==1}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Measured {
    pub playback_id:String,pub listen_session_id:String,pub segments:Vec<Segment>,
    #[serde(default="legacy_timing",skip_serializing_if="is_legacy_timing")]
    pub version:u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Owner {
    pub core_id: String,
    pub user_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Day {
    pub seconds: f64,
    pub last_at: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub id: String,
    pub version: u64,
    pub track_id: String,
    pub title: String,
    pub artist: String,
    pub duration: f64,
    pub started_at: i64,
    pub updated_at: i64,
    pub position: f64,
    pub days: BTreeMap<String, Day>,
    pub reason: String,
    #[serde(default, skip_serializing_if="Option::is_none")]
    pub measured: Option<Measured>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Row {
    pub owner: Owner,
    pub snapshot: Snapshot,
}
#[derive(Deserialize)]
pub struct Ack {
    pub id: String,
    pub version: u64,
}
#[derive(Deserialize)]
pub struct Rejected {
    pub id: String,
    pub version: u64,
    pub code: String,
}
#[derive(Deserialize)]
pub struct SyncResult {
    pub accepted: Vec<Ack>,
    pub rejected: Vec<Rejected>,
}
enum Write {
    Snapshot(Row),
    Cursor(cursor::Write),
    Barrier(mpsc::Sender<()>),
}
pub struct Store {
    db: Arc<Mutex<Connection>>,
    sender: mpsc::Sender<Write>,
    error: Arc<Mutex<Option<String>>>,
    pub client_id: String,
    pub changes: tokio::sync::watch::Sender<u64>,
}
fn persist(c: &Connection, row: &Row) -> Result<()> {
    // Older clients must never deserialize measured rows as legacy additive history.
    let table=match row.snapshot.measured.as_ref().map(|m|m.version){None=>"listens",Some(1)=>"measured_listens",Some(2)=>"output_listens",_=>anyhow::bail!("unsupported listening timing version")};
    c.execute(&format!("INSERT INTO {table}(id,core_id,user_id,version,payload) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET version=excluded.version,payload=excluded.payload WHERE excluded.version>{table}.version"),params![row.snapshot.id,row.owner.core_id,row.owner.user_id,i64::try_from(row.snapshot.version)?,serde_json::to_string(row)?])?;
    Ok(())
}
impl Store {
    pub fn system_suspend_state(&self)->Result<(u64,bool)> {
        #[derive(Deserialize)] struct Saved {generation:u64,held:bool}
        let db=self.db.lock().map_err(|_|anyhow::anyhow!("播放日志锁不可用"))?;
        let value=db.query_row("SELECT value FROM settings WHERE key='system_suspend'",[],|r|r.get::<_,String>(0)).optional()?;
        let Some(value)=value else{return Ok((0,false))};
        let value:Saved=serde_json::from_str(&value)?;
        if value.held&&value.generation==0 {anyhow::bail!("系统挂起记录无效");}
        Ok((value.generation,value.held))
    }
    pub fn save_system_suspend(&self,generation:u64,held:bool)->Result<()> {
        let value=serde_json::json!({"generation":generation,"held":held}).to_string();
        self.db.lock().map_err(|_|anyhow::anyhow!("播放日志锁不可用"))?.execute("INSERT INTO settings(key,value) VALUES('system_suspend',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[value])?;Ok(())
    }
    pub fn open(root: &Path) -> Result<Arc<Self>> {
        std::fs::create_dir_all(root)?;
        let db = Connection::open(root.join("offline-stats.db"))?;
        db.busy_timeout(Duration::from_secs(5))?;
        db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS owners(profile_id TEXT PRIMARY KEY,core_id TEXT NOT NULL,user_id TEXT NOT NULL); CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY,value TEXT NOT NULL); CREATE TABLE IF NOT EXISTS listens(id TEXT PRIMARY KEY,core_id TEXT NOT NULL,user_id TEXT NOT NULL,version INTEGER NOT NULL,payload TEXT NOT NULL,ack INTEGER NOT NULL DEFAULT 0,error TEXT); CREATE INDEX IF NOT EXISTS pending_owner ON listens(core_id,user_id,ack,version);")?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS measured_listens(id TEXT PRIMARY KEY,core_id TEXT NOT NULL,user_id TEXT NOT NULL,version INTEGER NOT NULL,payload TEXT NOT NULL,ack INTEGER NOT NULL DEFAULT 0,error TEXT); CREATE INDEX IF NOT EXISTS measured_pending_owner ON measured_listens(core_id,user_id,ack,version);")?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS output_listens(id TEXT PRIMARY KEY,core_id TEXT NOT NULL,user_id TEXT NOT NULL,version INTEGER NOT NULL,payload TEXT NOT NULL,ack INTEGER NOT NULL DEFAULT 0,error TEXT); CREATE INDEX IF NOT EXISTS output_pending_owner ON output_listens(core_id,user_id,ack,version);")?;
        db.execute("UPDATE listens SET payload=json_set(payload,'$.snapshot.reason','stop'),error=NULL WHERE version>ack AND (error IS NULL OR error='invalid_snapshot') AND json_extract(payload,'$.snapshot.reason')='stopped'",[])?;
        cursor::create(&db)?;
        group::create(&db)?;
        db.execute(
            "INSERT OR IGNORE INTO settings(key,value) VALUES('client_id',?1)",
            [uuid::Uuid::new_v4().to_string()],
        )?;
        let client_id = db.query_row(
            "SELECT value FROM settings WHERE key='client_id'",
            [],
            |r| r.get(0),
        )?;
        let db = Arc::new(Mutex::new(db));
        let error = Arc::new(Mutex::new(None));
        let (sender, receiver) = mpsc::channel();
        let writer_db = db.clone();
        let writer_error = error.clone();
        std::thread::Builder::new()
            .name("offline-stats".into())
            .spawn(move || {
                while let Ok(write) = receiver.recv() {
                    match write {
                        Write::Cursor(write)=>{
                            let result=writer_db.lock().map_err(|_|anyhow::anyhow!("游标数据库锁不可用")).and_then(|mut db|write.apply(&mut db));
                            if let Err(err)=result {if let Ok(mut error)=writer_error.lock(){*error=Some(format!("播放游标保存失败：{err}"));}}
                        }
                        Write::Snapshot(row) => {
                            let result = writer_db
                                .lock()
                                .map_err(|_| anyhow::anyhow!("统计数据库锁不可用"))
                                .and_then(|db| persist(&db, &row));
                            if let Err(err) = result {
                                if let Ok(mut error) = writer_error.lock() {
                                    *error = Some(format!("离线统计保存失败：{err}"));
                                }
                            }
                        }
                        Write::Barrier(done) => {
                            let _ = done.send(());
                        }
                    }
                }
            })?;
        Ok(Arc::new(Self {
            db,
            sender,
            error,
            client_id,
            changes:tokio::sync::watch::channel(0).0,
        }))
    }
    pub fn setting(&self,key:&str,value:Option<&str>)->Result<Option<String>> {
        let db=self.db.lock().map_err(|_|anyhow::anyhow!("播放日志锁不可用"))?;
        if let Some(value)=value {db.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,value])?;}
        Ok(db.query_row("SELECT value FROM settings WHERE key=?1",[key],|r|r.get(0)).optional()?)
    }
    pub fn owner(&self, profile: &str) -> Result<Option<Owner>> {
        let db = self
            .db
            .lock()
            .map_err(|_| anyhow::anyhow!("统计数据库锁不可用"))?;
        Ok(db
            .query_row(
                "SELECT core_id,user_id FROM owners WHERE profile_id=?1",
                [profile],
                |r| {
                    Ok(Owner {
                        core_id: r.get(0)?,
                        user_id: r.get(1)?,
                    })
                },
            )
            .optional()?)
    }
    pub fn remember(&self, profile: &str, owner: &Owner) -> Result<()> {
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction()?;
        tx.execute(
            "UPDATE owners SET user_id=?2 WHERE core_id=?1",
            params![owner.core_id, owner.user_id],
        )?;
        tx.execute("INSERT INTO owners(profile_id,core_id,user_id) VALUES(?1,?2,?3) ON CONFLICT(profile_id) DO UPDATE SET core_id=excluded.core_id,user_id=excluded.user_id",params![profile,owner.core_id,owner.user_id])?;
        tx.commit()?;
        Ok(())
    }
    pub fn forget(&self, profile: &str) -> Result<()> {
        self.db.lock().unwrap().execute(
            "DELETE FROM owners WHERE core_id=(SELECT core_id FROM owners WHERE profile_id=?1)",
            [profile],
        )?;
        Ok(())
    }
    pub fn save(&self, row: Row) -> Result<()> {
        self.sender
            .send(Write::Snapshot(row))
            .context("离线统计写入线程已退出")?;
        self.changes.send_modify(|v|*v=v.wrapping_add(1));
        Ok(())
    }
    pub fn drain(&self) -> Result<()> {
        let (sender, receiver) = mpsc::channel();
        self.sender.send(Write::Barrier(sender))?;
        receiver.recv_timeout(Duration::from_secs(10))?;
        Ok(())
    }
    pub fn take_error(&self) -> Option<String> {
        self.error.lock().ok()?.take()
    }
    pub fn pending(&self, owner: &Owner) -> Result<Vec<Row>> {
        self.pending_timing(owner,2)
    }
    pub fn pending_supported(&self, owner: &Owner, measured: bool) -> Result<Vec<Row>> {
        self.pending_timing(owner,u32::from(measured))
    }
    pub fn pending_timing(&self,owner:&Owner,version:u32)->Result<Vec<Row>> {
        let db = self.db.lock().unwrap();
        let sql="SELECT payload FROM (SELECT payload,rowid AS ordinal FROM listens WHERE core_id=?1 AND user_id=?2 AND version>ack AND error IS NULL UNION ALL SELECT payload,rowid AS ordinal FROM measured_listens WHERE core_id=?1 AND user_id=?2 AND version>ack AND error IS NULL AND ?3>=1 UNION ALL SELECT payload,rowid AS ordinal FROM output_listens WHERE core_id=?1 AND user_id=?2 AND version>ack AND error IS NULL AND ?3>=2) ORDER BY ordinal LIMIT 32";
        let mut stmt=db.prepare(sql)?;
        let rows = stmt
            .query_map(params![owner.core_id,owner.user_id,version], |r| {
                r.get::<_, String>(0)
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|payload| Ok(serde_json::from_str(&payload)?))
            .collect()
    }
    pub fn acknowledge(&self, owner: &Owner, result: &SyncResult) -> Result<()> {
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction()?;
        for table in ["listens","measured_listens","output_listens"] {
        for ack in &result.accepted {
            tx.execute(
                &format!("UPDATE {table} SET ack=MAX(ack,?4) WHERE id=?1 AND core_id=?2 AND user_id=?3 AND ?4<=version"),
                params![
                    ack.id,
                    owner.core_id,
                    owner.user_id,
                    i64::try_from(ack.version)?
                ],
            )?;
        }
        for ack in &result.rejected {
            tx.execute(&format!("UPDATE {table} SET error=?5 WHERE id=?1 AND core_id=?2 AND user_id=?3 AND version=?4"),params![ack.id,owner.core_id,owner.user_id,i64::try_from(ack.version)?,ack.code])?;
        }
        }
        tx.commit()?;
        Ok(())
    }
}
pub struct Recorder {
    pub row: Row,
    store: Arc<Store>,
    baseline: Option<(i64, f64, f64, bool)>,
    device_baseline: Option<(u64, f64, f64)>,
    clock: Instant,
    saved: f64,
}
impl Recorder {
    pub fn new(
        store: Arc<Store>,
        owner: Owner,
        track_id: String,
        title: String,
        artist: String,
        duration: f64,
    ) -> Self {
        let now = chrono::Utc::now().timestamp_millis();
        Self {
            row: Row {
                owner,
                snapshot: Snapshot {
                    id: uuid::Uuid::new_v4().to_string(),
                    version: 0,
                    track_id,
                    title,
                    artist,
                    duration,
                    started_at: now,
                    updated_at: now,
                    position: 0.,
                    days: BTreeMap::new(),
                    reason: "active".into(),
                    measured: None,
                },
            },
            store,
            baseline: None,
            device_baseline: None,
            clock: Instant::now(),
            saved: -5.,
        }
    }
    pub fn sample(&mut self, position: f64, playing: bool, reset: bool) -> Result<()> {
        self.sample_at(
            position,
            playing,
            reset,
            chrono::Utc::now().timestamp_millis(),
            self.clock.elapsed().as_secs_f64(),
        )
    }
    fn sample_at(
        &mut self,
        position: f64,
        playing: bool,
        reset: bool,
        at: i64,
        mono: f64,
    ) -> Result<()> {
        let s = &mut self.row.snapshot;
        if s.measured.as_ref().is_some_and(|m|m.version==2){return Ok(());} // v2 requires a paired output counter.
        if s.reason != "active" || !position.is_finite() || position < 0. {
            return Ok(());
        }
        if let Some((prior_at, prior_mono, prior_position, prior_playing)) = self.baseline {
            let elapsed = mono - prior_mono;
            let delta = position - prior_position;
            if !reset
                && prior_playing
                && elapsed > 0.
                && elapsed <= 30.
                && delta > 0.
                && delta <= elapsed + 1.
                && at > prior_at
                && ((at - prior_at) as f64 / 1000. - elapsed).abs() < 2.
            {
                let mut start = prior_at;
                while start < at {
                    let date = chrono::DateTime::from_timestamp_millis(start)
                        .context("无效统计时间")?
                        .date_naive();
                    let boundary = date
                        .succ_opt()
                        .context("统计日期超出范围")?
                        .and_hms_opt(0, 0, 0)
                        .unwrap()
                        .and_utc()
                        .timestamp_millis();
                    let stop = at.min(boundary);
                    let seconds = delta * (stop - start) as f64 / (at - prior_at) as f64;
                    let d = s.days.entry(date.to_string()).or_insert(Day {
                        seconds: 0.,
                        last_at: start,
                    });
                    d.seconds += seconds;
                    d.last_at = if stop == boundary { stop - 1 } else { stop };
                    start = stop;
                }
            }
        }
        s.position = position;
        s.updated_at = s.updated_at.max(at);
        self.baseline = Some((at, mono, position, playing));
        if mono - self.saved >= 5. {
            self.flush(mono)?;
        }
        Ok(())
    }
    fn flush(&mut self, mono: f64) -> Result<()> {
        self.row.snapshot.version += 1;
        self.store.save(self.row.clone())?;
        self.saved = mono;
        Ok(())
    }
    pub fn checkpoint(&mut self) -> Result<()> {
        if self.row.snapshot.reason == "active" {
            self.flush(self.clock.elapsed().as_secs_f64())?;
        }
        Ok(())
    }
    pub fn finish(&mut self, reason: &str) -> Result<()> {
        if self.row.snapshot.reason == "active" {
            self.row.snapshot.reason = if reason=="stopped"{"stop"}else{reason}.into();
            self.flush(self.clock.elapsed().as_secs_f64())?;
        }
        Ok(())
    }
}
