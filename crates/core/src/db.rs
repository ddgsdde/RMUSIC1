use anyhow::Result;
use rusqlite::Connection;
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

#[derive(Clone)]
pub struct Db(pub Arc<Mutex<Connection>>, Arc<Runtime>);
struct Runtime {
    readers: Vec<Arc<Mutex<Connection>>>,
    interrupts: Vec<Arc<rusqlite::InterruptHandle>>,
    closed: std::sync::atomic::AtomicBool,
    writer_slot: Arc<tokio::sync::Semaphore>,
    reader_slots: Arc<tokio::sync::Semaphore>,
    available: Mutex<Vec<usize>>,
    queued: std::sync::atomic::AtomicUsize,
    completed: std::sync::atomic::AtomicU64,
    wait_us: std::sync::atomic::AtomicU64,
    execution_us: std::sync::atomic::AtomicU64,
    max_wait_us: std::sync::atomic::AtomicU64,
}
#[derive(Debug)]
pub struct Busy;
impl std::fmt::Display for Busy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("数据库繁忙，请稍后重试")
    }
}
impl std::error::Error for Busy {}
struct Cancellation(Arc<CancelState>);
struct CancelState {
    handle: Arc<rusqlite::InterruptHandle>,
    active: Mutex<bool>,
    cancelled: std::sync::atomic::AtomicBool,
}
impl Drop for Cancellation {
    fn drop(&mut self) {
        use std::sync::atomic::Ordering::SeqCst;
        self.0.cancelled.store(true, SeqCst);
        let active=self.0.active.lock().unwrap();
        if *active {
            self.0.handle.interrupt();
        }
    }
}
struct ReadSlot {
    runtime: Arc<Runtime>,
    index: Option<usize>,
}
impl Drop for ReadSlot {
    fn drop(&mut self) {
        if let Some(index) = self.index {
            self.runtime.available.lock().unwrap().push(index);
        }
    }
}
struct Queued(Arc<Runtime>);
impl Drop for Queued {
    fn drop(&mut self) {
        self.0
            .queued
            .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(include_str!("schema.sql"))?;
        let job_columns = conn
            .prepare("PRAGMA table_info(jobs)")?
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (name, definition) in [
            ("payload", "TEXT NOT NULL DEFAULT '{}'"),
            ("priority", "INTEGER NOT NULL DEFAULT 0"),
            ("phase", "TEXT NOT NULL DEFAULT ''"),
            ("updated_at", "INTEGER NOT NULL DEFAULT 0"),
            ("next_try", "INTEGER NOT NULL DEFAULT 0"),
        ] {
            if !job_columns.iter().any(|column| column == name) {
                conn.execute(
                    &format!("ALTER TABLE jobs ADD COLUMN {name} {definition}"),
                    [],
                )?;
            }
        }
        let has_allocated = conn
            .prepare("PRAGMA table_info(cache_entries)")?
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .iter()
            .any(|name| name == "allocated_size");
        if !has_allocated {
            conn.execute(
                "ALTER TABLE cache_entries ADD COLUMN allocated_size INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
            let entries = conn
                .prepare("SELECT id,path,size FROM cache_entries")?
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for (id, path, size) in entries {
                let allocated = std::fs::metadata(path)
                    .map(|m| m.len() as i64)
                    .unwrap_or(size);
                conn.execute(
                    "UPDATE cache_entries SET allocated_size=?2 WHERE id=?1",
                    rusqlite::params![id, allocated.max(size)],
                )?;
            }
        }
        let download_columns = conn
            .prepare("PRAGMA table_info(downloads)")?
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if !download_columns
            .iter()
            .any(|column| column == "source_etag")
        {
            conn.execute("ALTER TABLE downloads ADD COLUMN source_etag TEXT", [])?;
        }
        let cache_columns = conn
            .prepare("PRAGMA table_info(cache_entries)")?
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (name, definition) in [
            ("single_file", "INTEGER NOT NULL DEFAULT 0"),
            ("content_etag", "TEXT"),
        ] {
            if !cache_columns.iter().any(|column| column == name) {
                conn.execute(
                    &format!("ALTER TABLE cache_entries ADD COLUMN {name} {definition}"),
                    [],
                )?;
            }
        }
        let export_columns = conn
            .prepare("PRAGMA table_info(cache_exports)")?
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if !export_columns
            .iter()
            .any(|column| column == "finalize_single")
        {
            conn.execute(
                "ALTER TABLE cache_exports ADD COLUMN finalize_single INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        let device_columns = conn
            .prepare("PRAGMA table_info(devices)")?
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (name, definition) in [
            ("removed", "INTEGER NOT NULL DEFAULT 0"),
            ("removed_zones", "TEXT NOT NULL DEFAULT '[]'"),
        ] {
            if !device_columns.iter().any(|column| column == name) {
                conn.execute(
                    &format!("ALTER TABLE devices ADD COLUMN {name} {definition}"),
                    [],
                )?;
            }
        }
        let playlist_columns = conn
            .prepare("PRAGMA table_info(playlists)")?
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if !playlist_columns.iter().any(|name| name == "smart_query") {
            conn.execute("ALTER TABLE playlists ADD COLUMN smart_query TEXT", [])?;
        }
        let claim_columns = conn
            .prepare("PRAGMA table_info(listen_claims)")?
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if !claim_columns.iter().any(|name| name == "timing_version") {
            conn.execute(
                "ALTER TABLE listen_claims ADD COLUMN timing_version INTEGER NOT NULL DEFAULT 1",
                [],
            )?;
        }
        // One-time compact metadata snapshot for already aggregated history; no audio scan.
        let migrated: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM settings WHERE key='listening_metadata_v1')",
            [],
            |r| r.get(0),
        )?;
        if !migrated {
            conn.execute("INSERT OR IGNORE INTO listening_metadata(track_id,album,album_artist,source) SELECT t.id,t.album,CASE WHEN trim(t.album_artist)='' THEN t.artist ELSE t.album_artist END,COALESCE(json_extract(t.provider_data,'$.cached_from.source'),t.source) FROM (SELECT DISTINCT track_id FROM daily_stats) d JOIN tracks t ON t.id=d.track_id",[])?;
            conn.execute(
                "INSERT OR IGNORE INTO settings(key,value) VALUES('listening_metadata_v1','true')",
                [],
            )?;
        }
        crate::artist_credits::migrate(&conn)?;
        crate::media_assets::migrate(&conn)?;
        crate::media_assets::repair_metadata_covers(&conn)?;
        conn.execute_batch("CREATE INDEX IF NOT EXISTS tracks_artwork_release ON tracks(COALESCE(json_extract(provider_data,'$.cached_from.source'),source),COALESCE(json_extract(provider_data,'$.release_id'),json_extract(provider_data,'$.album_mid')));")?;
        crate::album_artwork::migrate(&conn)?;
        crate::cached_artwork::migrate(&conn)?;
        crate::source_observation::migrate(&conn)?;
        let mut interrupts = vec![Arc::new(conn.get_interrupt_handle())];
        let mut readers = Vec::new();
        for _ in 0..2 {
            let reader = Connection::open_with_flags(
                path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                    | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )?;
            reader.busy_timeout(std::time::Duration::from_secs(5))?;
            reader.execute_batch("PRAGMA foreign_keys=ON; PRAGMA cache_size=-4096;")?;
            interrupts.push(Arc::new(reader.get_interrupt_handle()));
            readers.push(Arc::new(Mutex::new(reader)));
        }
        Ok(Self(
            Arc::new(Mutex::new(conn)),
            Arc::new(Runtime {
                readers,
                interrupts,
                closed: false.into(),
                writer_slot: Arc::new(tokio::sync::Semaphore::new(1)),
                reader_slots: Arc::new(tokio::sync::Semaphore::new(2)),
                available: Mutex::new(vec![0, 1]),
                queued: 0.into(),
                completed: 0.into(),
                wait_us: 0.into(),
                execution_us: 0.into(),
                max_wait_us: 0.into(),
            }),
        ))
    }
    #[track_caller]
    pub fn run<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    ) -> impl std::future::Future<Output=Result<T>> + '_ {
        self.execute(false, std::panic::Location::caller(), f)
    }
    /// Read-only WAL snapshots: aggregate and paginated reads never monopolize the writer.
    #[track_caller]
    pub fn read<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    ) -> impl std::future::Future<Output=Result<T>> + '_ {
        self.execute(true, std::panic::Location::caller(), f)
    }
    async fn execute<T: Send + 'static>(
        &self,
        read: bool,
        caller: &'static std::panic::Location<'static>,
        f: impl FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        use std::sync::atomic::Ordering::Relaxed;
        let runtime = self.1.clone();
        let account_scope=crate::accounts::scope::current();
        if self.is_closed() {
            anyhow::bail!("核心正在停止");
        }
        let before = runtime.queued.fetch_add(1, Relaxed);
        let queued = Queued(runtime.clone());
        if before >= 256 {
            return Err(Busy.into());
        }
        let started = std::time::Instant::now();
        let gate = if read {
            runtime.reader_slots.clone()
        } else {
            runtime.writer_slot.clone()
        };
        let permit = tokio::time::timeout(std::time::Duration::from_secs(5), gate.acquire_owned())
            .await
            .map_err(|_| Busy)??;
        if self.is_closed() {
            anyhow::bail!("核心正在停止");
        }
        let index = if read {
            Some(
                runtime
                    .available
                    .lock()
                    .unwrap()
                    .pop()
                    .expect("reader semaphore and pool agree"),
            )
        } else {
            None
        };
        let conn = index
            .map(|i| runtime.readers[i].clone())
            .unwrap_or_else(|| self.0.clone());
        let cancel_state = index.map(|i| {
            Arc::new(CancelState {
                handle: runtime.interrupts[i + 1].clone(),
                active: Mutex::new(false),
                cancelled: false.into(),
            })
        });
        let cancellation = cancel_state.clone().map(Cancellation);
        let slot = ReadSlot {
            runtime: runtime.clone(),
            index,
        };
        let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel::<()>();
        let result = tokio::task::spawn_blocking(move || {
            let _account_scope=crate::accounts::scope::enter_blocking(account_scope);
            let _permit = permit;
            let _queued = queued;
            let _slot = slot;
            if matches!(
                cancel_rx.try_recv(),
                Err(tokio::sync::oneshot::error::TryRecvError::Closed)
            ) {
                anyhow::bail!("database request cancelled");
            }
            let mut connection = conn
                .lock()
                .map_err(|_| anyhow::anyhow!("database lock poisoned"))?;
            let wait = started.elapsed().as_micros().min(u64::MAX as u128) as u64;
            runtime.wait_us.fetch_add(wait, Relaxed);
            runtime.max_wait_us.fetch_max(wait, Relaxed);
            let execution = std::time::Instant::now();
            let result = if read {
                connection.execute_batch("BEGIN DEFERRED")?;
                if let Some(state) = &cancel_state {
                    *state.active.lock().unwrap()=true;
                }
                let value = if cancel_state
                    .as_ref()
                    .is_some_and(|s| s.cancelled.load(std::sync::atomic::Ordering::SeqCst))
                {
                    Err(anyhow::anyhow!("database read cancelled"))
                } else {
                    f(&mut connection)
                };
                if let Some(state) = &cancel_state {
                    // Finish every in-flight interrupt before rolling the snapshot back.
                    *state.active.lock().unwrap()=false;
                }
                let end = connection.execute_batch("ROLLBACK");
                match (value, end) {
                    (Ok(value), Ok(())) => Ok(value),
                    (Err(error), _) => Err(error),
                    (_, Err(error)) => Err(error.into()),
                }
            } else {
                f(&mut connection)
            };
            runtime.execution_us.fetch_add(
                execution.elapsed().as_micros().min(u64::MAX as u128) as u64,
                Relaxed,
            );
            runtime.completed.fetch_add(1, Relaxed);
            if execution.elapsed().as_millis()>250 || wait>1_000_000 {
                tracing::warn!(file=caller.file(),line=caller.line(),read,wait_ms=wait/1000,execution_ms=execution.elapsed().as_millis() as u64,"slow database operation");
            }
            result
        })
        .await?;
        drop(cancel_tx);
        drop(cancellation);
        result
    }
    pub fn is_closed(&self) -> bool {
        self.1.closed.load(std::sync::atomic::Ordering::Acquire)
    }
    /// Stop admission and interrupt running SQLite statements before Tokio joins blocking workers.
    pub fn shutdown(&self) {
        self.1
            .closed
            .store(true, std::sync::atomic::Ordering::Release);
        for handle in &self.1.interrupts {
            handle.interrupt();
        }
    }
    pub fn metrics(&self) -> serde_json::Value {
        use std::sync::atomic::Ordering::Relaxed;
        let r = &self.1;
        serde_json::json!({"queued_or_running":r.queued.load(Relaxed),"completed":r.completed.load(Relaxed),"wait_us":r.wait_us.load(Relaxed),"max_wait_us":r.max_wait_us.load(Relaxed),"execution_us":r.execution_us.load(Relaxed),"read_connections":r.readers.len(),"queue_limit":256})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn cancelled_and_shutdown_reads_interrupt_sql_and_release_all_slots() {
        let temp = tempfile::tempdir().unwrap();
        let db = Db::open(&temp.path().join("db")).unwrap();
        for shutdown in [false, true] {
            let clone = db.clone();
            let (start, ready) = tokio::sync::oneshot::channel();
            let task = tokio::spawn(async move {
                clone.read(move|c|{let _=start.send(());c.query_row("WITH RECURSIVE n(v) AS(SELECT 0 UNION ALL SELECT v+1 FROM n WHERE v<10000000000) SELECT SUM(v) FROM n",[],|r|Ok(r.get::<_,i64>(0)?)).map_err(Into::into)}).await
            });
            ready.await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            if shutdown {
                db.shutdown();
                assert!(task.await.unwrap().is_err());
            } else {
                task.abort();
                let _ = task.await;
            }
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                while db.metrics()["queued_or_running"] != 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
            if !shutdown {
                assert_eq!(
                    db.read(|c| Ok(c.query_row("SELECT 1", [], |r| r.get::<_, i64>(0))?))
                        .await
                        .unwrap(),
                    1
                );
            }
        }
        assert!(db.run(|_| Ok(())).await.is_err());
    }
    #[tokio::test]
    async fn wal_readers_keep_snapshot_while_writer_commits_and_cannot_mutate() {
        let temp = tempfile::tempdir().unwrap();
        let db = Db::open(&temp.path().join("db")).unwrap();
        db.run(|c| {
            c.execute("INSERT INTO settings VALUES('test','1')", [])?;
            Ok(())
        })
        .await
        .unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let reader = db.clone();
        let task = tokio::spawn(async move {
            reader
                .read(move |c| {
                    let first: String =
                        c.query_row("SELECT value FROM settings WHERE key='test'", [], |r| {
                            r.get(0)
                        })?;
                    tx.send(()).unwrap();
                    std::thread::sleep(std::time::Duration::from_millis(200));
                    let second: String =
                        c.query_row("SELECT value FROM settings WHERE key='test'", [], |r| {
                            r.get(0)
                        })?;
                    assert_eq!(first, second);
                    Ok(())
                })
                .await
        });
        tokio::task::spawn_blocking(move || rx.recv().unwrap())
            .await
            .unwrap();
        tokio::time::timeout(
            std::time::Duration::from_millis(150),
            db.run(|c| {
                c.execute("UPDATE settings SET value='2' WHERE key='test'", [])?;
                Ok(())
            }),
        )
        .await
        .unwrap()
        .unwrap();
        task.await.unwrap().unwrap();
        assert!(db
            .read(|c| {
                c.execute("UPDATE settings SET value='3'", [])?;
                Ok(())
            })
            .await
            .is_err());
        assert_eq!(
            db.read(|c| Ok(c.query_row(
                "SELECT value FROM settings WHERE key='test'",
                [],
                |r| r.get::<_, String>(0)
            )?))
            .await
            .unwrap(),
            "2"
        );
        assert_eq!(db.metrics()["queued_or_running"], 0);
    }
    #[tokio::test]
    async fn cancelled_queued_work_does_not_execute_or_hold_budget() {
        let temp = tempfile::tempdir().unwrap();
        let db = Db::open(&temp.path().join("db")).unwrap();
        let hold = db.1.writer_slot.clone().acquire_owned().await.unwrap();
        let clone = db.clone();
        let task = tokio::spawn(async move {
            clone
                .run(|c| {
                    c.execute("INSERT INTO settings VALUES('cancelled','true')", [])?;
                    Ok(())
                })
                .await
        });
        tokio::task::yield_now().await;
        task.abort();
        let _ = task.await;
        drop(hold);
        assert_eq!(db.metrics()["queued_or_running"], 0);
        assert!(!db
            .read(|c| Ok(c.query_row(
                "SELECT EXISTS(SELECT 1 FROM settings WHERE key='cancelled')",
                [],
                |r| r.get::<_, bool>(0)
            )?))
            .await
            .unwrap());
    }
    #[tokio::test]
    async fn disk_full_rolls_back_atomically_and_recovers_after_capacity_returns() {
        let temp = tempfile::tempdir().unwrap();
        let db = Db::open(&temp.path().join("db")).unwrap();
        let result = db
            .run(|c| {
                c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA max_page_count=1;")?;
                let tx = c.transaction()?;
                tx.execute(
                    "INSERT INTO settings VALUES('uncommitted',zeroblob(1048576))",
                    [],
                )?;
                tx.commit()?;
                Ok(())
            })
            .await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .downcast_ref::<rusqlite::Error>()
            .is_some());
        db.run(|c| {
            c.execute_batch("PRAGMA max_page_count=2147483646")?;
            c.execute("INSERT INTO settings VALUES('recovered','yes')", [])?;
            Ok(())
        })
        .await
        .unwrap();
        assert!(!db
            .read(|c| Ok(c.query_row(
                "SELECT EXISTS(SELECT 1 FROM settings WHERE key='uncommitted')",
                [],
                |r| r.get::<_, bool>(0)
            )?))
            .await
            .unwrap());
    }
}
