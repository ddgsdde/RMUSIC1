//! Continue the existing HTTP response into a temporary file once playback is established.
use super::{cancelled, Bytes};
use futures_util::{stream, StreamExt};
use std::{
    io,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{mpsc, oneshot, watch},
};

type Item = Result<Bytes, io::Error>;
pub(super) const MAX_BYTES: u64 = 512 * 1024 * 1024;
const BLOCK_BYTES: usize = 64 * 1024;

struct DiskReader {
    file: File,
    tail: watch::Receiver<(u64, bool)>,
    offset: u64,
}
impl DiskReader {
    async fn next(&mut self) -> io::Result<Option<Bytes>> {
        loop {
            let (end, finished) = *self.tail.borrow_and_update();
            if self.offset < end {
                let size = (end - self.offset).min(BLOCK_BYTES as u64) as usize;
                let mut data = vec![0; size];
                self.file.read_exact(&mut data).await?;
                self.offset += size as u64;
                return Ok(Some(Bytes::from(data)));
            }
            if finished {
                return Ok(None);
            }
            self.tail
                .changed()
                .await
                .map_err(|_| io::Error::other("audio spool closed"))?;
        }
    }
}

fn receive(rx: mpsc::Receiver<Item>) -> impl futures_util::Stream<Item = Item> {
    stream::unfold(rx, |mut rx| async {
        rx.recv().await.map(|item| (item, rx))
    })
}

const INITIAL_AHEAD: f64 = 5.;
const WEAK_AHEAD: f64 = 10.;
const FILL_TO_END: f64 = 15.;
const LOW_AHEAD: f64 = 3.5;
const LOW_FOR: Duration = Duration::from_millis(650);

pub(super) struct Window {
    clock: watch::Receiver<f64>,
    bytes_per_second: u64,
    start: f64,
    granule_ms: Option<Arc<AtomicU64>>,
    received: u64,
    last_chunk: u64,
    low_since: Option<(f64,tokio::time::Instant)>,
    weak: bool,
}
impl Window {
    pub(super) fn new(
        clock: watch::Receiver<f64>,
        bytes_per_second: u64,
        start: f64,
        granule_ms: Option<Arc<AtomicU64>>,
    ) -> Self {
        Self {
            clock,
            bytes_per_second,
            start,
            granule_ms,
            received: 0,
            last_chunk: 0,
            low_since: None,
            weak: false,
        }
    }

    fn buffered_end(&self) -> f64 {
        let granule = self
            .granule_ms
            .as_ref()
            .map(|value| value.load(Ordering::Acquire))
            .unwrap_or(0);
        if granule > 0 {
            granule as f64 / 1000.
        } else {
            self.start + self.received as f64 / self.bytes_per_second as f64
        }
    }

    fn observe(&mut self) {
        let position = *self.clock.borrow_and_update();
        if self.weak || !(1. ..FILL_TO_END).contains(&position) {
            self.low_since=None;
            return;
        }
        if self.buffered_end() - position >= LOW_AHEAD {
            self.low_since = None;
        } else {
            let (started_at,since) = *self.low_since.get_or_insert_with(||(position,tokio::time::Instant::now()));
            self.weak = position-started_at>=LOW_FOR.as_secs_f64() || since.elapsed()>=LOW_FOR;
        }
    }

    async fn wait(&mut self) -> bool {
        loop {
            let position = *self.clock.borrow_and_update();
            if position >= FILL_TO_END {
                return true;
            }
            let ahead = if self.weak { WEAK_AHEAD } else { INITIAL_AHEAD };
            // Allow for one batched HTTP read while keeping at least three seconds ahead.
            let batch = self.last_chunk.saturating_sub(BLOCK_BYTES as u64) as f64
                / self.bytes_per_second as f64;
            let target =
                position.max(self.start) + ahead - if self.weak { 0. } else { batch.min(2.) };
            if self.received < BLOCK_BYTES as u64 || self.buffered_end() < target {
                return true;
            }
            tokio::select! {
                changed = self.clock.changed() => if changed.is_err() { return false; },
                // A large embedded tag can need more bytes before the first frame.
                _ = tokio::time::sleep(Duration::from_secs(4)), if position == 0. => return true,
            }
        }
    }

    async fn read(&mut self, body: &mut stream::BoxStream<'static, Item>) -> Option<Item> {
        self.observe();
        let reading = next(body);
        tokio::pin!(reading);
        let item = loop {
            tokio::select! {
                biased;
                changed = self.clock.changed(), if !self.weak && *self.clock.borrow() < FILL_TO_END => {
                    if changed.is_err() { return None; }
                    self.observe();
                }
                _ = tokio::time::sleep_until(self.low_since.map(|(_,since)|since).unwrap_or_else(tokio::time::Instant::now) + LOW_FOR),
                    if !self.weak && self.low_since.is_some() => { self.observe(); }
                item = &mut reading => break item,
            }
        };
        if let Some(Ok(bytes)) = &item {
            self.last_chunk = bytes.len() as u64;
            self.received += self.last_chunk;
            if self.buffered_end() - *self.clock.borrow() >= LOW_AHEAD {
                self.low_since = None;
            }
        }
        item
    }
}

pub(super) fn fill(
    mut body: stream::BoxStream<'static, Item>,
    mut window: Window,
    cancellation: watch::Receiver<bool>,
    directory: Option<std::path::PathBuf>,
    max_bytes: u64,
) -> stream::BoxStream<'static, Item> {
    let (before, prefix) = mpsc::channel(1);
    let (disk, middle) = oneshot::channel();
    let (after, suffix) = mpsc::channel(1);
    tokio::spawn(async move {
        while !window.weak && *window.clock.borrow() < FILL_TO_END {
            let permit = tokio::select! { biased;
                _ = cancelled(cancellation.clone()) => return,
                changed = window.clock.changed() => { if changed.is_err() { return; } continue; }
                permit = before.reserve() => match permit { Ok(permit) => permit, Err(_) => return },
            };
            let item = tokio::select! { biased;
                _ = cancelled(cancellation.clone()) => return,
                _ = before.closed() => return,
                item = async {
                    if !window.wait().await { return None; }
                    window.read(&mut body).await
                } => match item { Some(item) => item, None => return },
            };
            let failed = item.is_err();
            permit.send(item);
            if failed {
                return;
            }
        }
        let files = tokio::task::spawn_blocking(move || -> io::Result<_> {
            let writer = match directory {
                Some(path) => tempfile::NamedTempFile::new_in(path)?,
                None => tempfile::NamedTempFile::new()?,
            };
            let reader = writer.reopen()?;
            Ok((File::from_std(writer.into_file()), File::from_std(reader)))
        })
        .await;
        let Ok(Ok((mut writer, reader))) = files else {
            // With no disk space, keep the same bounded response and carry on playing.
            forward(body, None, before, cancellation).await;
            return;
        };
        let (written, tail) = watch::channel((0, false));
        if disk
            .send(DiskReader {
                file: reader,
                tail,
                offset: 0,
            })
            .is_err()
        {
            return;
        }
        drop(before);

        let mut end = 0;
        let remainder = loop {
            let item = tokio::select! { biased;
                _ = cancelled(cancellation.clone()) => return,
                _ = written.closed() => return,
                item = async {
                    if !window.wait().await { return None; }
                    window.read(&mut body).await
                } => match item { Some(item) => item, None => break None },
            };
            let Ok(bytes) = item else {
                break Some(item);
            };
            if end + bytes.len() as u64 > max_bytes
                || writer.write_all(&bytes).await.is_err()
                || writer.flush().await.is_err()
            {
                break Some(Ok(bytes));
            }
            end += bytes.len() as u64;
            written.send_replace((end, false));
        };
        written.send_replace((end, true));
        drop(writer);
        if remainder.is_some() {
            forward(body, remainder, after, cancellation).await;
        }
    });
    let on_disk = stream::once(middle)
        .filter_map(|reader| std::future::ready(reader.ok()))
        .flat_map(|reader| {
            stream::try_unfold(reader, |mut reader| async move {
                Ok(reader.next().await?.map(|bytes| (bytes, reader)))
            })
        });
    receive(prefix)
        .chain(on_disk)
        .chain(receive(suffix))
        .boxed()
}

async fn next(body: &mut stream::BoxStream<'static, Item>) -> Option<Item> {
    match tokio::time::timeout(Duration::from_secs(30), body.next()).await {
        Ok(item) => item,
        Err(_) => Some(Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "Core audio stream stalled",
        ))),
    }
}

async fn forward(
    mut body: stream::BoxStream<'static, Item>,
    mut pending: Option<Item>,
    tx: mpsc::Sender<Item>,
    cancellation: watch::Receiver<bool>,
) {
    loop {
        let item = match pending.take() {
            Some(item) => item,
            None => tokio::select! { biased;
                _ = cancelled(cancellation.clone()) => return,
                _ = tx.closed() => return,
                item = next(&mut body) => match item { Some(item) => item, None => return },
            },
        };
        let failed = item.is_err();
        tokio::select! { biased;
            _ = cancelled(cancellation.clone()) => return,
            result = tx.send(item) => if result.is_err() || failed { return; },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::TryStreamExt;

    #[tokio::test]
    async fn disk_limit_falls_back_to_the_same_response_without_losing_bytes() {
        let (_cancel, cancellation) = watch::channel(false);
        let (_gate, gate) = watch::channel(15.);
        let source =
            stream::iter([b"abc", b"def", b"ghi"].map(|chunk| Ok(Bytes::from_static(chunk))));
        let result: Vec<Bytes> = fill(
            source.boxed(),
            Window::new(gate, 20_000, 0., None),
            cancellation,
            None,
            4,
        )
        .try_collect()
        .await
        .unwrap();
        assert_eq!(result.concat(), b"abcdefghi");
    }

    #[tokio::test]
    async fn startup_is_limited_even_when_the_local_consumer_drains_immediately() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };

        let (_cancel, cancellation) = watch::channel(false);
        let (clock, gate) = watch::channel(0.);
        let received = Arc::new(AtomicUsize::new(0));
        let count = received.clone();
        let source = stream::iter((0..80).map(move |_| {
            count.fetch_add(10_000, Ordering::Relaxed);
            Ok(Bytes::from(vec![7; 10_000]))
        }));
        let body = fill(
            source.boxed(),
            Window::new(gate, 20_000, 0., None),
            cancellation,
            None,
            MAX_BYTES,
        );
        let consuming = tokio::spawn(async move { body.try_collect::<Vec<_>>().await.unwrap() });
        let wait_for = |bytes| {
            let received = received.clone();
            async move {
                tokio::time::timeout(Duration::from_secs(1), async {
                    while received.load(Ordering::Relaxed) < bytes {
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .unwrap();
                tokio::time::sleep(Duration::from_millis(10)).await;
                assert_eq!(received.load(Ordering::Relaxed), bytes);
            }
        };
        wait_for(100_000).await;
        clock.send_replace(10.);
        wait_for(300_000).await;
        clock.send_replace(15.);
        let chunks = tokio::time::timeout(Duration::from_secs(3), consuming)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(chunks.iter().map(Bytes::len).sum::<usize>(), 800_000);
    }

    #[tokio::test]
    async fn low_reserve_during_a_stalled_read_expands_the_window_before_fifteen_seconds() {
        let (_cancel, cancellation) = watch::channel(false);
        let (clock, gate) = watch::channel(0.);
        let (request, requested) = oneshot::channel();
        let (resume, resumed) = oneshot::channel();
        let read = Arc::new(AtomicU64::new(0));
        let count = read.clone();
        let chunk = || Ok(Bytes::from_static(&[9; 10_000]));
        let source = stream::iter((0..10).map(move |_| chunk()))
            .chain(stream::once(async move {
                request.send(()).unwrap();
                resumed.await.unwrap();
                chunk()
            }))
            .chain(stream::iter((0..59).map(move |_| chunk())))
            .inspect(move |_| {
                count.fetch_add(10_000, Ordering::Relaxed);
            });
        let mut body = fill(
            source.boxed(),
            Window::new(gate, 20_000, 0., None),
            cancellation,
            None,
            MAX_BYTES,
        );
        for _ in 0..10 {
            assert_eq!(body.next().await.unwrap().unwrap().len(), 10_000);
        }
        assert_eq!(read.load(Ordering::Relaxed), 100_000);

        clock.send_replace(2.);
        tokio::time::timeout(Duration::from_secs(1), requested)
            .await
            .unwrap()
            .unwrap();
        clock.send_replace(3.);
        resume.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while read.load(Ordering::Relaxed) < 260_000 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the weak connection must fill a ten-second lead while output is backpressured");
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(read.load(Ordering::Relaxed), 260_000);

        clock.send_replace(15.);
        let rest = tokio::time::timeout(Duration::from_secs(3), body.try_collect::<Vec<_>>())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(rest.iter().map(Bytes::len).sum::<usize>(), 600_000);
        assert!(rest
            .iter()
            .all(|bytes| bytes.iter().all(|value| *value == 9)));
    }
}
