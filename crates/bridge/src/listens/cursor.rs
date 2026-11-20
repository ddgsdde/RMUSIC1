//! Ordered source checkpoints share the listening writer; stale workers cannot reactivate a cursor.
use super::{Owner, Row, Store};
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Boundary {
    pub playback_id: String,
    pub track_id: String,
    pub position: f64,
    pub frames: u64,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Checkpoint {
    #[serde(default="legacy_timing",skip_serializing_if="is_legacy_timing")]
    pub measured_version:u32,
    pub owner: Owner,
    pub endpoint: String,
    pub zone: String,
    pub run_id: String,
    pub version: u64,
    pub plan_id: String,
    pub sample_rate: u32,
    pub heard: Vec<Boundary>,
    pub terminal: bool,
    pub request: Option<Value>,
}
fn legacy_timing()->u32{1}
fn is_legacy_timing(version:&u32)->bool{*version==1}
const TABLES:[&str;2]=["playback_cursors","output_playback_cursors"];
fn table(version:u32)->Result<&'static str>{match version {1=>Ok(TABLES[0]),2=>Ok(TABLES[1]),_=>bail!("unsupported recovery timing version")}}
pub(super) fn create(db: &Connection) -> Result<()> {
    for name in TABLES {db.execute_batch(&format!("CREATE TABLE IF NOT EXISTS {name}(core_id TEXT NOT NULL,user_id TEXT NOT NULL,endpoint TEXT NOT NULL,zone TEXT NOT NULL,run_id TEXT NOT NULL,version INTEGER NOT NULL,data TEXT NOT NULL,retired INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(core_id,user_id,endpoint,zone));"))?;}
    Ok(())
}
pub(super) enum Write {
    Begin(Checkpoint),
    Sample(Checkpoint, Vec<Row>),
    Retire(Handle),
    Close(Owner, String),
}
impl Write {
    pub(super) fn apply(self, db: &mut Connection) -> Result<()> {
        match self {
            Self::Begin(row) => {
                let tx = db.transaction()?;
                let name=table(row.measured_version)?;
                for existing in TABLES {tx.execute(&format!("UPDATE {existing} SET retired=1 WHERE core_id=?1 AND user_id=?2 AND endpoint=?3"),params![row.owner.core_id,row.owner.user_id,row.endpoint])?;}
                tx.execute(&format!("INSERT INTO {name}(core_id,user_id,endpoint,zone,run_id,version,data,retired) VALUES(?1,?2,?3,?4,?5,?6,?7,0) ON CONFLICT(core_id,user_id,endpoint,zone) DO UPDATE SET run_id=excluded.run_id,version=excluded.version,data=excluded.data,retired=0"),params![row.owner.core_id,row.owner.user_id,row.endpoint,row.zone,row.run_id,i64::try_from(row.version)?,serde_json::to_string(&row)?])?;
                tx.commit()?;
            }
            Self::Sample(row, listens) => {
                let name=table(row.measured_version)?;
                let tx = db.transaction()?;
                for listen in listens {
                    if listen.owner != row.owner {
                        bail!("游标与听音身份不符");
                    }
                    super::persist(&tx, &listen)?;
                }
                tx.execute(&format!("UPDATE {name} SET version=?6,data=?7 WHERE core_id=?1 AND user_id=?2 AND endpoint=?3 AND zone=?4 AND run_id=?5 AND retired=0 AND version<?6 AND json_extract(data,'$.request') IS NULL"),params![row.owner.core_id,row.owner.user_id,row.endpoint,row.zone,row.run_id,i64::try_from(row.version)?,serde_json::to_string(&row)?])?;
                tx.commit()?;
            }
            Self::Retire(handle) => {
                db.execute(&format!("UPDATE {} SET retired=1 WHERE core_id=?1 AND user_id=?2 AND endpoint=?3 AND zone=?4 AND run_id=?5",table(handle.measured_version)?),params![handle.owner.core_id,handle.owner.user_id,handle.endpoint,handle.zone,handle.run_id])?;
            }
            Self::Close(owner, endpoint) => {
                for name in TABLES {db.execute(&format!("UPDATE {name} SET retired=1 WHERE core_id=?1 AND user_id=?2 AND endpoint=?3"),params![owner.core_id,owner.user_id,endpoint])?;}
            }
        }
        Ok(())
    }
}
#[derive(Clone)]
pub struct Handle {
    measured_version:u32,
    store: Arc<Store>,
    owner: Owner,
    endpoint: String,
    zone: String,
    run_id: String,
}
impl Handle {
    pub fn retire(&self) -> Result<()> {
        self.store
            .sender
            .send(super::Write::Cursor(Write::Retire(self.clone())))
            .context("游标写入线程已退出")
    }
    pub fn ready(&self) -> Result<bool> {
        self.store.drain()?;
        self.is_current()
    }
    pub fn is_current(&self) -> Result<bool> {
        let db = self
            .store
            .db
            .lock()
            .map_err(|_| anyhow::anyhow!("游标数据库锁不可用"))?;
        let name=table(self.measured_version)?;let other=table(if self.measured_version==1{2}else{1})?;
        Ok(db.query_row(&format!("SELECT EXISTS(SELECT 1 FROM {name} WHERE core_id=?1 AND user_id=?2 AND endpoint=?3 AND zone=?4 AND run_id=?5 AND retired=0) AND NOT EXISTS(SELECT 1 FROM {other} WHERE core_id=?1 AND user_id=?2 AND endpoint=?3 AND retired=0)"),params![self.owner.core_id,self.owner.user_id,self.endpoint,self.zone,self.run_id],|r|r.get(0))?)
    }
}
pub fn close(store: &Store, owner: &Owner, endpoint: &str) -> Result<()> {
    store
        .sender
        .send(super::Write::Cursor(Write::Close(
            owner.clone(),
            endpoint.into(),
        )))
        .context("游标写入线程已退出")
}
pub fn rejected_evidence(store: &Store, checkpoint: &Checkpoint) -> Result<bool> {
    let db = store
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("游标数据库锁不可用"))?;
    let table=match checkpoint.measured_version {1=>"measured_listens",2=>"output_listens",_=>bail!("unsupported recovery timing version")};
    let mut statement=db.prepare(&format!("SELECT payload FROM {table} WHERE core_id=?1 AND user_id=?2 AND error IS NOT NULL"))?;
    let rows = statement
        .query_map(
            params![checkpoint.owner.core_id, checkpoint.owner.user_id],
            |r| r.get::<_, String>(0),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for data in rows {
        let row: Row = serde_json::from_str(&data)?;
        if row.snapshot.measured.is_some_and(|m| {
            checkpoint
                .heard
                .iter()
                .any(|source| source.playback_id == m.playback_id)
        }) {
            return Ok(true);
        }
    }
    Ok(false)
}
/// Call after output teardown and writer drain. Freeze the recovery body before any HTTP request.
pub fn freeze(
    store: Arc<Store>,
    owner: &Owner,
    endpoint: &str,
) -> Result<Option<(Checkpoint, Handle)>> {
    let mut db = store
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("游标数据库锁不可用"))?;
    let tx = db.transaction()?;
    let mut saved=None;
    for (index,name) in TABLES.iter().enumerate() {
        let data:Option<String>=tx.query_row(&format!("SELECT data FROM {name} WHERE core_id=?1 AND user_id=?2 AND endpoint=?3 AND retired=0 ORDER BY rowid DESC LIMIT 1"),params![owner.core_id,owner.user_id,endpoint],|r|r.get(0)).optional()?;
        if let Some(data)=data {if saved.is_some(){bail!("multiple recovery timing versions are active; manual playback required");}saved=Some((index as u32+1,data));}
    }
    let Some((version,data))=saved else {return Ok(None)};
    let mut row: Checkpoint = serde_json::from_str(&data)?;
    if row.owner != *owner || row.endpoint != endpoint {
        bail!("播放游标身份不符");
    }
    if row.measured_version!=version {bail!("recovery cursor timing table mismatch");}
    if row.request.is_none() {
        row.request = Some(
            json!({"endpoint_id":row.endpoint,"request_id":uuid::Uuid::new_v4().to_string(),
            "plan_id":row.plan_id,"sample_rate":row.sample_rate,"heard":row.heard,"terminal":row.terminal}),
        );
        if row.measured_version==2 {row.request.as_mut().unwrap()["measured_version"]=json!(2);}
        tx.execute(&format!("UPDATE {} SET data=?6 WHERE core_id=?1 AND user_id=?2 AND endpoint=?3 AND zone=?4 AND run_id=?5 AND retired=0",table(row.measured_version)?),params![owner.core_id,owner.user_id,endpoint,row.zone,row.run_id,serde_json::to_string(&row)?])?;
    }
    let handle = Handle {
        measured_version:row.measured_version,
        store: store.clone(),
        owner: owner.clone(),
        endpoint: row.endpoint.clone(),
        zone: row.zone.clone(),
        run_id: row.run_id.clone(),
    };
    tx.commit()?;
    Ok(Some((row, handle)))
}
pub struct Recorder {
    pub handle: Handle,
    row: Checkpoint,
    last: Instant,
}
impl Recorder {
    pub fn begin(
        store: Arc<Store>,
        owner: Owner,
        endpoint: String,
        zone: String,
        plan_id: String,
        sample_rate: u32,
        current: Boundary,
    ) -> Result<Self> {
        Self::begin_timed(store,owner,endpoint,zone,plan_id,sample_rate,current,1)
    }
    pub fn begin_timed(store:Arc<Store>,owner:Owner,endpoint:String,zone:String,plan_id:String,sample_rate:u32,current:Boundary,measured_version:u32)->Result<Self> {
        if ![1,2].contains(&measured_version){bail!("unsupported recovery timing version");}
        let row = Checkpoint {
            measured_version,
            owner: owner.clone(),
            endpoint: endpoint.clone(),
            zone: zone.clone(),
            run_id: uuid::Uuid::new_v4().to_string(),
            version: 1,
            plan_id,
            sample_rate,
            heard: vec![current],
            terminal: false,
            request: None,
        };
        store
            .sender
            .send(super::Write::Cursor(Write::Begin(row.clone())))?;
        let handle = Handle {
            measured_version,
            store,
            owner,
            endpoint,
            zone,
            run_id: row.run_id.clone(),
        };
        Ok(Self {
            handle,
            row,
            last: Instant::now(),
        })
    }
    fn boundary(&mut self, value: Boundary) -> Result<bool> {
        if !value.position.is_finite() || value.position < 0. {
            bail!("游标位置无效");
        }
        if let Some(last) = self
            .row
            .heard
            .last_mut()
            .filter(|last| last.playback_id == value.playback_id)
        {
            if last.track_id != value.track_id
                || last.frames > value.frames
                || last.position > value.position
            {
                bail!("游标回退或实例冲突");
            }
            *last = value;
            Ok(false)
        } else {
            if self
                .row
                .heard
                .iter()
                .any(|old| old.playback_id == value.playback_id)
            {
                bail!("游标边界顺序无效");
            }
            self.row.heard.push(value);
            if self.row.heard.len() > 9 {
                self.row.heard.remove(0);
            }
            Ok(true)
        }
    }
    pub fn sample(
        &mut self,
        ended: Vec<Boundary>,
        current: Boundary,
        rate: u32,
        terminal: bool,
        force: bool,
    ) -> Result<()> {
        self.sample_with(ended, current, rate, terminal, force, || Ok(Vec::new()))
    }
    pub fn sample_with(
        &mut self,
        ended: Vec<Boundary>,
        current: Boundary,
        rate: u32,
        terminal: bool,
        force: bool,
        before_save: impl FnOnce() -> Result<Vec<Row>>,
    ) -> Result<()> {
        if self.row.measured_version==2&&self.row.sample_rate!=rate {bail!("recovery output sample rate changed");}
        if !(8000..=rmusic_audio::MAX_CLOCK_RATE).contains(&rate) {
            bail!("游标设备采样率无效");
        }
        let mut changed = false;
        for value in ended {
            changed |= self.boundary(value)?;
        }
        changed |= self.boundary(current)?;
        self.row.sample_rate = rate;
        self.row.terminal |= terminal;
        if force || changed || terminal || self.last.elapsed() >= Duration::from_secs(1) {
            let listens = before_save()?;
            self.row.version += 1;
            self.handle
                .store
                .sender
                .send(super::Write::Cursor(Write::Sample(
                    self.row.clone(),
                    listens,
                )))?;
            self.last = Instant::now();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dual_recovery_v2_legacy_readers_cannot_see_output_cursors_or_choose_ambiguous_runs() {
        let dir=tempfile::tempdir().unwrap();let store=Store::open(dir.path()).unwrap();let run=Recorder::begin_timed(store.clone(),owner("a"),"endpoint".into(),"zone".into(),"plan".into(),48000,boundary("a",0.),2).unwrap();assert!(run.handle.ready().unwrap());
        let db=store.db.lock().unwrap();assert_eq!(db.query_row("SELECT COUNT(*) FROM playback_cursors WHERE retired=0",[],|r|r.get::<_,i64>(0)).unwrap(),0);drop(db);
        // An old binary can later write its own table without knowing that output cursors exist.
        let mut old=run.row.clone();old.measured_version=1;old.run_id=uuid::Uuid::new_v4().to_string();store.db.lock().unwrap().execute("INSERT INTO playback_cursors VALUES(?1,?2,?3,?4,?5,?6,?7,0)",params![old.owner.core_id,old.owner.user_id,old.endpoint,old.zone,old.run_id,i64::try_from(old.version).unwrap(),serde_json::to_string(&old).unwrap()]).unwrap();
        assert!(!run.handle.is_current().unwrap());assert!(freeze(store.clone(),&owner("a"),"endpoint").is_err());close(&store,&owner("a"),"endpoint").unwrap();store.drain().unwrap();assert!(freeze(store,&owner("a"),"endpoint").unwrap().is_none());
    }
    #[test]
    fn dual_recovery_v2_cursor_output_rows_are_atomic_and_freeze_exact_version_and_frames() {
        let dir=tempfile::tempdir().unwrap();let store=Store::open(dir.path()).unwrap();
        let mut run=Recorder::begin_timed(store.clone(),owner("a"),"endpoint".into(),"zone".into(),"plan".into(),768000,Boundary{playback_id:"a".into(),track_id:"track-a".into(),position:20.,frames:0},2).unwrap();assert!(run.handle.ready().unwrap());
        let mut collector=super::super::Collector::new(store.clone(),owner("a"));collector.sample_output("a",20.,20.050025,38400,768000,||Ok(Some(super::super::Source{listen_session_id:uuid::Uuid::new_v4().to_string(),track_id:"track-a".into(),title:"QA".into(),artist:"QA".into(),duration:30.}))).unwrap();collector.sample_output("a",20.,20.10005,76800,768000,||panic!("source already captured")).unwrap();store.drain().unwrap();
        store.db.lock().unwrap().execute_batch("CREATE TRIGGER qa_output_cursor_fail BEFORE UPDATE OF data ON output_playback_cursors BEGIN SELECT RAISE(FAIL,'output cursor rollback'); END;").unwrap();
        let current=Boundary{playback_id:"a".into(),track_id:"track-a".into(),position:20.10005,frames:76800};run.sample_with(vec![],current.clone(),768000,false,true,||Ok(collector.checkpoint_rows("a"))).unwrap();store.drain().unwrap();assert!(store.take_error().unwrap().contains("output cursor rollback"));assert_eq!(store.pending(&owner("a")).unwrap()[0].snapshot.position,20.050025);
        {let db=store.db.lock().unwrap();assert_eq!(db.query_row("SELECT json_extract(data,'$.heard[0].frames') FROM output_playback_cursors",[],|r|r.get::<_,i64>(0)).unwrap(),0);db.execute_batch("DROP TRIGGER qa_output_cursor_fail").unwrap();}
        run.sample_with(vec![],current.clone(),768000,false,true,||Ok(collector.checkpoint_rows("a"))).unwrap();store.drain().unwrap();let rows=store.pending(&owner("a")).unwrap();assert_eq!(rows.len(),1);assert_eq!(rows[0].snapshot.measured.as_ref().unwrap().version,2);assert_eq!(rows[0].snapshot.measured.as_ref().unwrap().segments[0].output.as_ref().unwrap().end,76800);
        let(row,_)=freeze(store.clone(),&owner("a"),"endpoint").unwrap().unwrap();assert_eq!(row.measured_version,2);assert_eq!(row.request.as_ref().unwrap()["measured_version"],2);assert_eq!(row.heard[0],current);let body=row.request.clone();drop(run);drop(collector);drop(store);let store=Store::open(dir.path()).unwrap();assert_eq!(freeze(store,&owner("a"),"endpoint").unwrap().unwrap().0.request,body);
    }
    #[test]
    fn dual_recovery_v2_rejected_output_evidence_blocks_restore_and_rate_changes_are_rejected() {
        let dir=tempfile::tempdir().unwrap();let store=Store::open(dir.path()).unwrap();let mut run=Recorder::begin_timed(store.clone(),owner("a"),"endpoint".into(),"zone".into(),"plan".into(),48000,boundary("a",0.),2).unwrap();assert!(run.handle.ready().unwrap());
        let mut collector=super::super::Collector::new(store.clone(),owner("a"));collector.sample_output("a",0.,0.10005,4800,48000,||Ok(Some(super::super::Source{listen_session_id:uuid::Uuid::new_v4().to_string(),track_id:"track-a".into(),title:"QA".into(),artist:"QA".into(),duration:30.}))).unwrap();let current=Boundary{position:0.10005,frames:4800,..boundary("a",0.)};
        assert!(run.sample(vec![],current.clone(),768000,false,true).is_err());run.sample_with(vec![],current,48000,false,true,||Ok(collector.checkpoint_rows("a"))).unwrap();store.drain().unwrap();store.db.lock().unwrap().execute("UPDATE output_listens SET error='server rejected immutable claim'",[]).unwrap();
        let(row,_)=freeze(store.clone(),&owner("a"),"endpoint").unwrap().unwrap();assert!(rejected_evidence(&store,&row).unwrap());let mut legacy=row;legacy.measured_version=1;assert!(!rejected_evidence(&store,&legacy).unwrap());
    }
    fn owner(user: &str) -> Owner {
        Owner {
            core_id: "core".into(),
            user_id: user.into(),
        }
    }
    fn boundary(id: &str, position: f64) -> Boundary {
        Boundary {
            playback_id: id.into(),
            track_id: format!("track-{id}"),
            position,
            frames: (position * 48000.) as u64,
        }
    }
    fn recorder(store: Arc<Store>, user: &str) -> Recorder {
        Recorder::begin(
            store,
            owner(user),
            "endpoint".into(),
            "zone".into(),
            "plan".into(),
            48000,
            boundary("a", 0.),
        )
        .unwrap()
    }
    #[test]
    fn restart_freezes_the_same_bounded_request_and_account_isolation_keeps_old_records() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let mut recorder = recorder(store.clone(), "a");
        assert!(recorder.handle.ready().unwrap());
        for i in 0..12 {
            let id = format!("source-{i}");
            recorder
                .sample(vec![], boundary(&id, 1.), 48000, false, true)
                .unwrap();
        }
        store.drain().unwrap();
        drop(recorder);
        drop(store);
        let store = Store::open(dir.path()).unwrap();
        assert!(freeze(store.clone(), &owner("b"), "endpoint")
            .unwrap()
            .is_none());
        let first = freeze(store.clone(), &owner("a"), "endpoint")
            .unwrap()
            .unwrap()
            .0;
        assert_eq!(first.heard.len(), 9);
        assert_eq!(first.heard[0].playback_id, "source-3");
        let again = freeze(store, &owner("a"), "endpoint").unwrap().unwrap().0;
        assert_eq!(first.request, again.request);
    }
    #[test]
    fn stop_and_new_run_reject_late_samples_and_old_retirement() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let mut old = recorder(store.clone(), "a");
        old.handle.retire().unwrap();
        old.sample(vec![], boundary("a", 1.), 48000, false, true)
            .unwrap();
        store.drain().unwrap();
        assert!(freeze(store.clone(), &owner("a"), "endpoint")
            .unwrap()
            .is_none());
        let mut new = recorder(store.clone(), "a");
        old.handle.retire().unwrap();
        old.sample(vec![], boundary("a", 2.), 48000, false, true)
            .unwrap();
        new.sample(vec![], boundary("a", 0.5), 48000, false, true)
            .unwrap();
        store.drain().unwrap();
        let (row, _) = freeze(store.clone(), &owner("a"), "endpoint")
            .unwrap()
            .unwrap();
        assert_eq!(row.run_id, new.row.run_id);
        assert_eq!(row.heard[0].position, 0.5);
        new.sample(vec![], boundary("a", 1.), 48000, false, true)
            .unwrap();
        store.drain().unwrap();
        assert_eq!(
            freeze(store, &owner("a"), "endpoint")
                .unwrap()
                .unwrap()
                .0
                .request,
            row.request
        );
    }
    #[test]
    fn terminal_eof_is_monotone_and_normal_close_cannot_auto_resume() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let mut run = recorder(store.clone(), "a");
        run.sample(
            vec![boundary("a", 2.)],
            boundary("b", 1.),
            48000,
            false,
            true,
        )
        .unwrap();
        run.sample(vec![], boundary("b", 3.), 48000, true, true)
            .unwrap();
        store.drain().unwrap();
        let row = freeze(store.clone(), &owner("a"), "endpoint")
            .unwrap()
            .unwrap()
            .0;
        assert!(row.terminal);
        assert_eq!(row.heard.len(), 2);
        close(&store, &owner("a"), "endpoint").unwrap();
        store.drain().unwrap();
        assert!(freeze(store, &owner("a"), "endpoint").unwrap().is_none());
    }
    #[test]
    fn cursor_and_its_actual_listening_snapshot_roll_back_together() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let mut run = recorder(store.clone(), "a");
        let mut collector = super::super::Collector::new(store.clone(), owner("a"));
        collector
            .sample("a", 0.125, 6000, 48000, || {
                Ok(Some(super::super::Source {
                    listen_session_id: uuid::Uuid::new_v4().to_string(),
                    track_id: "track-a".into(),
                    title: "QA".into(),
                    artist: "QA".into(),
                    duration: 2.,
                }))
            })
            .unwrap();
        collector
            .sample("a", 0.5, 24000, 48000, || {
                panic!("source metadata already captured")
            })
            .unwrap();
        store.drain().unwrap();
        store.db.lock().unwrap().execute_batch("CREATE TRIGGER qa_cursor_fail BEFORE UPDATE OF data ON playback_cursors BEGIN SELECT RAISE(FAIL,'cursor rollback QA'); END;").unwrap();
        run.sample_with(vec![], boundary("a", 0.5), 48000, false, true, || {
            Ok(collector.checkpoint_rows("a"))
        })
        .unwrap();
        store.drain().unwrap();
        assert!(store.take_error().unwrap().contains("cursor rollback QA"));
        assert_eq!(
            store.pending(&owner("a")).unwrap()[0].snapshot.position,
            0.125
        );
        {
            let db = store.db.lock().unwrap();
            assert_eq!(
                db.query_row(
                    "SELECT json_extract(data,'$.heard[0].position') FROM playback_cursors",
                    [],
                    |r| r.get::<_, f64>(0)
                )
                .unwrap(),
                0.
            );
            db.execute_batch("DROP TRIGGER qa_cursor_fail").unwrap();
        }
        collector
            .sample("a", 0.75, 36000, 48000, || {
                panic!("source metadata already captured")
            })
            .unwrap();
        run.sample_with(vec![], boundary("a", 0.75), 48000, false, true, || {
            Ok(collector.checkpoint_rows("a"))
        })
        .unwrap();
        store.drain().unwrap();
        assert_eq!(
            store.pending(&owner("a")).unwrap()[0].snapshot.position,
            0.75
        );
        assert_eq!(
            freeze(store, &owner("a"), "endpoint")
                .unwrap()
                .unwrap()
                .0
                .heard[0]
                .position,
            0.75
        );
    }
}
