//! One bounded image conversion at a time; never touches the audio processing path.
use crate::error::{ApiError, ApiResult};
use std::{process::Stdio, sync::LazyLock, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
    sync::Semaphore,
};
static CONVERT: LazyLock<Semaphore> = LazyLock::new(|| Semaphore::new(1));
pub const INPUT_LIMIT: usize = 20 * 1024 * 1024;
pub const DISPLAY_LIMIT: usize = 8 * 1024 * 1024;
fn dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.len() >= 24 {
        return Some((
            u32::from_be_bytes(bytes[16..20].try_into().ok()?),
            u32::from_be_bytes(bytes[20..24].try_into().ok()?),
        ));
    }
    if bytes.starts_with(b"\xff\xd8") {
        let mut offset = 2;
        while offset + 4 <= bytes.len() {
            if bytes[offset] != 0xff {
                return None;
            }
            let marker = bytes[offset + 1];
            offset += 2;
            if marker == 0xff {
                offset -= 1;
                continue;
            }
            if marker == 0xda || marker == 0xd9 {
                break;
            }
            let size = u16::from_be_bytes(bytes.get(offset..offset + 2)?.try_into().ok()?) as usize;
            if size < 2 || offset + size > bytes.len() {
                return None;
            }
            if matches!(marker,0xc0..=0xc3|0xc5..=0xc7|0xc9..=0xcb|0xcd..=0xcf) && size >= 7 {
                return Some((
                    u16::from_be_bytes(bytes[offset + 5..offset + 7].try_into().ok()?) as u32,
                    u16::from_be_bytes(bytes[offset + 3..offset + 5].try_into().ok()?) as u32,
                ));
            }
            offset += size;
        }
    }
    if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        match bytes.get(12..16)? {
            b"VP8X" if bytes.len() >= 30 => {
                let read = |start| {
                    u32::from_le_bytes([bytes[start], bytes[start + 1], bytes[start + 2], 0]) + 1
                };
                return Some((read(24), read(27)));
            }
            b"VP8L" if bytes.get(20) == Some(&0x2f) && bytes.len() >= 25 => {
                let bits = u32::from_le_bytes(bytes[21..25].try_into().ok()?);
                return Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1));
            }
            b"VP8 " if bytes.get(23..26) == Some(b"\x9d\x01\x2a") && bytes.len() >= 30 => {
                return Some((
                    (u16::from_le_bytes(bytes[26..28].try_into().ok()?) & 0x3fff) as u32,
                    (u16::from_le_bytes(bytes[28..30].try_into().ok()?) & 0x3fff) as u32,
                ));
            }
            _ => {}
        }
    }
    None
}
pub async fn prepare(bytes: Vec<u8>) -> ApiResult<Vec<u8>> {
    if bytes.len() > INPUT_LIMIT {
        return Err(ApiError::bad("图片超过20 MiB"));
    }
    let large = dimensions(&bytes).is_some_and(|(w, h)| w > 4096 || h > 4096);
    if bytes.len() <= DISPLAY_LIMIT && !large {
        return Ok(bytes);
    }
    let _slot = tokio::time::timeout(Duration::from_secs(4), CONVERT.acquire())
        .await
        .map_err(|_| {
            ApiError(
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "artwork_busy".into(),
                "大图处理繁忙，请稍后重试".into(),
            )
        })?
        .map_err(|_| ApiError::bad("图片处理服务停止"))?;
    let mut child =
        Command::new(std::env::var_os("RMUSIC_FFMPEG").unwrap_or_else(|| "ffmpeg".into()))
            .args([
                "-v",
                "error",
                "-nostdin",
                "-threads",
                "1",
                "-filter_threads",
                "1",
                "-max_alloc",
                "134217728",
                "-max_pixels",
                "64000000",
                "-f",
                "image2pipe",
                "-i",
                "pipe:0",
                "-vf",
                "scale=w='min(1280,iw)':h='min(1280,ih)':force_original_aspect_ratio=decrease",
                "-frames:v",
                "1",
                "-threads",
                "1",
                "-f",
                "image2pipe",
                "-c:v",
                "png",
                "pipe:1",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(anyhow::Error::from)?;
    let mut input = child.stdin.take().unwrap();
    let output = child.stdout.take().unwrap();
    let writer = tokio::spawn(async move {
        input.write_all(&bytes).await?;
        input.shutdown().await
    });
    let result = tokio::time::timeout(Duration::from_secs(8), async {
        let mut result = Vec::new();
        output
            .take(DISPLAY_LIMIT as u64 + 1)
            .read_to_end(&mut result)
            .await?;
        anyhow::ensure!(result.len() <= DISPLAY_LIMIT, "display image exceeds bound");
        let status = child.wait().await?;
        anyhow::ensure!(
            status.success() && result.starts_with(b"\x89PNG\r\n\x1a\n"),
            "image conversion failed"
        );
        Ok::<_, anyhow::Error>(result)
    })
    .await;
    writer.abort();
    let _ = writer.await;
    match result {
        Ok(Ok(value)) => Ok(value),
        _ => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            Err(ApiError::bad("图片尺寸超出处理能力或图片数据无效"))
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn large_artwork_is_converted_once_to_a_bounded_display_file() {
        let generated = std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:s=1600x1200",
                "-frames:v",
                "1",
                "-threads",
                "1",
                "-f",
                "image2pipe",
                "-c:v",
                "png",
                "pipe:1",
            ])
            .output()
            .unwrap();
        assert!(generated.status.success());
        let mut bytes = generated.stdout;
        bytes.resize(9 * 1024 * 1024, 0);
        let result = prepare(bytes).await.unwrap();
        let (w, h) = dimensions(&result).unwrap();
        assert_eq!((w, h), (1280, 960));
        assert!(result.len() < DISPLAY_LIMIT);
        assert!(prepare(vec![0; INPUT_LIMIT + 1]).await.is_err());
    }
}
