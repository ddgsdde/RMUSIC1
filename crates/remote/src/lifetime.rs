//! Hyper hands upgraded streams to another task before its connection future
//! completes. The byte tunnel must outlive that future until the IO is dropped.
use std::{
    pin::Pin,
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
pub struct Tracked<T> {
    inner: T,
    closed: Option<tokio::sync::oneshot::Sender<()>>,
}
pub fn track<T>(inner: T) -> (Tracked<T>, tokio::sync::oneshot::Receiver<()>) {
    let (tx, rx) = tokio::sync::oneshot::channel();
    (
        Tracked {
            inner,
            closed: Some(tx),
        },
        rx,
    )
}
impl<T> Drop for Tracked<T> {
    fn drop(&mut self) {
        if let Some(tx) = self.closed.take() {
            let _ = tx.send(());
        }
    }
}
impl<T: AsyncRead + Unpin> AsyncRead for Tracked<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_read(cx, buf)
    }
}
impl<T: AsyncWrite + Unpin> AsyncWrite for Tracked<T> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, buf)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}
