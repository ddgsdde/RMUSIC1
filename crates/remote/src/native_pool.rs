//! Reuse idle Core TLS connections without queueing controls behind response bodies.
use axum::body::Body;
use hyper::{
    body::{Body as _, Frame, Incoming},
    client::conn::http1::SendRequest,
};
use std::{
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::{Duration, Instant},
};

pub type Pool = Arc<Mutex<Vec<Connection>>>;
pub type Activity = Arc<Mutex<Option<Instant>>>;
pub struct Connection {
    pub sender: SendRequest<Body>,
    pub activity: Activity,
}
pub fn take(pool: &Pool) -> Option<Connection> {
    let mut connections = pool.lock().unwrap();
    connections.retain(|c| !c.sender.is_closed());
    let at = connections.iter().position(|c| c.sender.is_ready())?;
    let connection = connections.swap_remove(at);
    *connection.activity.lock().unwrap() = None;
    Some(connection)
}
pub async fn idle(activity: Activity) {
    loop {
        tokio::time::sleep(Duration::from_secs(20)).await;
        if activity
            .lock()
            .unwrap()
            .is_some_and(|at| at.elapsed() >= Duration::from_secs(20))
        {
            return;
        }
    }
}
pub fn response(
    pool: Pool,
    connection: Connection,
    response: hyper::Response<Incoming>,
) -> axum::response::Response {
    let (parts, inner) = response.into_parts();
    axum::response::Response::from_parts(
        parts,
        Body::new(ReusableBody {
            inner,
            connection: Some(connection),
            pool,
        }),
    )
}
struct ReusableBody {
    inner: Incoming,
    connection: Option<Connection>,
    pool: Pool,
}
impl hyper::body::Body for ReusableBody {
    type Data = hyper::body::Bytes;
    type Error = hyper::Error;
    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let polled = Pin::new(&mut self.inner).poll_frame(cx);
        if matches!(polled, Poll::Ready(None))
            || matches!(polled, Poll::Ready(Some(Ok(_)))) && self.inner.is_end_stream()
        {
            self.release();
        }
        polled
    }
    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }
    fn size_hint(&self) -> hyper::body::SizeHint {
        self.inner.size_hint()
    }
}
impl ReusableBody {
    fn release(&mut self) {
        if let Some(connection) = self.connection.take() {
            if connection.sender.is_closed() {
                return;
            }
            *connection.activity.lock().unwrap() = Some(Instant::now());
            let mut pool = self.pool.lock().unwrap();
            pool.retain(|c| !c.sender.is_closed());
            if pool.len() < 4 {
                pool.push(connection);
            }
        }
    }
}
impl Drop for ReusableBody {
    fn drop(&mut self) {
        if self.inner.is_end_stream() {
            self.release();
        }
    }
}
