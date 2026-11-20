//! Core-bound original-file downloads. One queue worker; cancellation joins file I/O.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::{
    sync::{watch, Notify},
    task::JoinHandle,
};

#[derive(Clone, Serialize, Deserialize)]
pub struct Record {
    pub id: String,
    pub track_id: String,
    pub directory: PathBuf,
    pub status: String,
    pub path: Option<PathBuf>,
    pub error: Option<String>,
}
struct State {
    records: Vec<Record>,
    active: Option<(String, Arc<AtomicBool>)>,
}
pub struct FileExports {
    state: Arc<Mutex<State>>,
    notify: Arc<Notify>,
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
    index: PathBuf,
}
impl FileExports {
    pub fn new(
        downloads: Arc<crate::downloads::Downloads>,
        root: &Path,
        scope: &str,
    ) -> Result<Self> {
        let index = root.join("exports").join(format!("{scope}.json"));
        std::fs::create_dir_all(index.parent().unwrap())?;
        let mut records: Vec<Record> = if index.exists() {
            anyhow::ensure!(
                std::fs::metadata(&index)?.len() <= 4 * 1024 * 1024,
                "下载记录过大"
            );
            serde_json::from_slice(&std::fs::read(&index)?)?
        } else {
            vec![]
        };
        anyhow::ensure!(
            records.len() <= 2000 && records.iter().all(|r| r.directory.is_absolute()),
            "下载记录无效"
        );
        for record in &mut records {
            if ["queued", "downloading", "exporting"].contains(&record.status.as_str()) {
                record.status = "paused".into();
            }
        }
        let state = Arc::new(Mutex::new(State {
            records,
            active: None,
        }));
        let notify = Arc::new(Notify::new());
        let (stop, mut stopped) = watch::channel(false);
        let work = state.clone();
        let wake = notify.clone();
        let file = index.clone();
        let task = tokio::spawn(async move {
            loop {
                if *stopped.borrow() {
                    break;
                }
                let record = {
                    let mut state = work.lock().unwrap();
                    let next = state
                        .records
                        .iter_mut()
                        .find(|r| r.status == "queued")
                        .map(|r| {
                            r.status = "downloading".into();
                            r.error = None;
                            r.clone()
                        });
                    if let Some(record) = &next {
                        state.active = Some((record.id.clone(), Arc::new(AtomicBool::new(false))));
                    }
                    next
                };
                let Some(record) = record else {
                    tokio::select! {_=stopped.changed()=>break,_=wake.notified()=>{}}
                    continue;
                };
                let cancel = work.lock().unwrap().active.as_ref().unwrap().1.clone();
                let result = async {
                    let started = downloads.start(record.track_id.clone())?["started"]
                        .as_bool()
                        .unwrap_or(false);
                    loop {
                        if cancel.load(Ordering::Acquire) || *stopped.borrow() {
                            if started {
                                downloads.cancel(&record.track_id).await?;
                            }
                            anyhow::bail!("下载已暂停，可继续");
                        }
                        let progress = downloads.job_state(&record.track_id);
                        match progress["state"].as_str() {
                            Some("complete") => break,
                            Some("paused") => anyhow::bail!(
                                "{}",
                                progress["error"].as_str().unwrap_or("下载已暂停")
                            ),
                            _ => {}
                        }
                        tokio::select! {
                            _ = stopped.changed() => { cancel.store(true, Ordering::Release); },
                            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
                        }
                    }
                    if let Some(item) = work
                        .lock()
                        .unwrap()
                        .records
                        .iter_mut()
                        .find(|r| r.id == record.id)
                    {
                        item.status = "exporting".into();
                    }
                    let owner = downloads.clone();
                    let track = record.track_id.clone();
                    let directory = record.directory.clone();
                    let cancel = cancel.clone();
                    // Never abort a blocking copy: cancellation is checked while copying, then joined.
                    tokio::task::spawn_blocking(move || {
                        owner.export_original(&track, &directory, &cancel)
                    })
                    .await?
                }
                .await;
                {
                    let mut state = work.lock().unwrap();
                    state.active = None;
                    if let Some(item) = state.records.iter_mut().find(|r| r.id == record.id) {
                        match result {
                            Ok(path) => {
                                item.path = Some(path);
                                item.status = "complete".into();
                                item.error = None;
                            }
                            Err(error) => {
                                item.status = "paused".into();
                                item.error = Some(error.to_string());
                            }
                        }
                    }
                    if let Err(error) = persist(&file, &state.records) {
                        eprintln!("Mac download record persistence failed: {error}");
                    }
                }
            }
            let mut state = work.lock().unwrap();
            for r in &mut state.records {
                if r.status == "queued" {
                    r.status = "paused".into();
                }
            }
            let _ = persist(&file, &state.records);
        });
        Ok(Self {
            state,
            notify,
            stop,
            task,
            index,
        })
    }
    pub fn snapshot(&self, offset: usize) -> Value {
        let state = self.state.lock().unwrap();
        let active = state
            .records
            .iter()
            .filter(|r| ["queued", "downloading", "exporting"].contains(&r.status.as_str()))
            .count();
        json!({"items":state.records.iter().rev().skip(offset).take(50).collect::<Vec<_>>(),"total":state.records.len(),"active":active,"offset":offset,"limit":50})
    }
    pub fn enqueue(&self, body: Value) -> Result<Value> {
        let ids = body["track_ids"].as_array().context("请选择 Core 歌曲")?;
        anyhow::ensure!(!ids.is_empty() && ids.len() <= 1000, "每批须为 1–1000 首");
        let directory = PathBuf::from(body["directory"].as_str().context("请选择下载目录")?);
        anyhow::ensure!(
            directory.is_absolute() && directory.is_dir(),
            "下载目录无效或已断开"
        );
        let directory = directory.canonicalize()?;
        let ids = ids
            .iter()
            .map(|id| {
                let id = id.as_str().context("歌曲身份无效")?;
                anyhow::ensure!(
                    !id.is_empty()
                        && id.len() <= 256
                        && id
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b)),
                    "歌曲身份无效"
                );
                Ok(id.to_owned())
            })
            .collect::<Result<std::collections::BTreeSet<_>>>()?;
        let mut state = self.state.lock().unwrap();
        let mut next = state.records.clone();
        let mut count = 0;
        for id in ids {
            if next.iter().any(|r| {
                r.track_id == id
                    && r.directory == directory
                    && ["queued", "downloading", "exporting"].contains(&r.status.as_str())
            }) {
                continue;
            }
            next.push(Record {
                id: uuid::Uuid::new_v4().to_string(),
                track_id: id,
                directory: directory.clone(),
                status: "queued".into(),
                path: None,
                error: None,
            });
            count += 1;
        }
        // Retain a bounded history, preserving every active/paused request.
        while next.len() > 2000 {
            let at = next
                .iter()
                .position(|r| r.status == "complete")
                .context("下载队列已满，请先完成或暂停当前批次")?;
            next.remove(at);
        }
        persist(&self.index, &next)?;
        state.records = next;
        drop(state);
        self.notify.notify_one();
        Ok(json!({"queued":count}))
    }
    pub fn change(&self, body: Value) -> Result<Value> {
        let id = body["id"].as_str().context("缺少下载任务")?;
        let resume = body["resume"].as_bool().unwrap_or(false);
        let mut state = self.state.lock().unwrap();
        anyhow::ensure!(
            !resume || state.active.as_ref().is_none_or(|(active, _)| active != id),
            "正在暂停，请稍后继续"
        );
        let mut next = state.records.clone();
        let record = next
            .iter_mut()
            .find(|r| r.id == id)
            .context("下载任务不存在")?;
        if resume {
            anyhow::ensure!(record.status == "paused", "此任务不能重复启动");
            if let Some(directory) = body["directory"].as_str() {
                let directory = PathBuf::from(directory);
                anyhow::ensure!(
                    directory.is_absolute() && directory.is_dir(),
                    "下载目录无效或已断开"
                );
                record.directory = directory.canonicalize()?;
            }
            record.status = "queued".into();
            record.error = None;
        } else {
            anyhow::ensure!(record.status != "complete", "已完成文件不会被删除");
            record.status = "paused".into();
        }
        persist(&self.index, &next)?;
        state.records = next;
        if !resume {
            if let Some((active, cancel)) = &state.active {
                if active == id {
                    cancel.store(true, Ordering::Release);
                }
            }
        }
        drop(state);
        self.notify.notify_one();
        Ok(Value::Null)
    }
    pub async fn close(self) {
        self.stop.send_replace(true);
        if let Some((_, cancel)) = &self.state.lock().unwrap().active {
            cancel.store(true, Ordering::Release);
        }
        let _ = self.task.await;
    }
}
fn persist(path: &Path, records: &[Record]) -> Result<()> {
    use std::io::Write;
    let bytes = serde_json::to_vec(records)?;
    anyhow::ensure!(bytes.len() <= 4 * 1024 * 1024, "下载记录过大");
    let temp = path.with_extension("pending");
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temp)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(&bytes)?;
    file.sync_all()?;
    std::fs::rename(temp, path)?;
    Ok(())
}
