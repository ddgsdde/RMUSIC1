//! Publish an original file without overwriting user files. Never shell-interpolate a filename.
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
pub fn publish(
    source: &Path,
    directory: &Path,
    name: &str,
    size: u64,
    digest: &str,
    cancel: &AtomicBool,
) -> Result<PathBuf> {
    let target = directory.join(name);
    anyhow::ensure!(directory.is_dir(), "下载目录不可用");
    if target.exists() {
        anyhow::ensure!(
            std::fs::metadata(&target)?.is_file()
                && std::fs::metadata(&target)?.len() == size
                && checksum(&target, cancel)? == digest,
            "同名文件已存在，未覆盖。请选择其他目录"
        );
        return Ok(target);
    }
    anyhow::ensure!(
        std::fs::metadata(source)?.len() == size && checksum(source, cancel)? == digest,
        "原文件校验失败，请重新下载"
    );
    anyhow::ensure!(!cancel.load(Ordering::Acquire), "下载已暂停");
    // APFS copy-on-write saves space without sharing mutable contents with the cache.
    if clone_exclusive(source, &target).is_ok() {
        return Ok(target);
    }
    anyhow::ensure!(!target.exists(), "目标文件已存在，未覆盖");
    copy_verified(source, directory, &target, size, digest, cancel)
}
fn copy_verified(
    source: &Path,
    directory: &Path,
    target: &Path,
    size: u64,
    digest: &str,
    cancel: &AtomicBool,
) -> Result<PathBuf> {
    let temporary = directory.join(format!(".rmusic-{}.part", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut input = std::fs::File::open(source)?;
        let mut output = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 131072];
        let mut copied = 0u64;
        loop {
            anyhow::ensure!(!cancel.load(Ordering::Acquire), "下载已暂停，可继续");
            let n = input.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            output.write_all(&buffer[..n])?;
            hash.update(&buffer[..n]);
            copied += n as u64;
        }
        anyhow::ensure!(
            copied == size && format!("{:x}", hash.finalize()) == digest,
            "输出文件校验失败，未发布"
        );
        output.sync_all()?;
        // create-new publication even when a file appears during the copy.
        publish_exclusive(&temporary, &target)
            .context("无法发布文件；未覆盖已有文件，请检查目标文件系统")?;
        Ok(target.to_path_buf())
    })();
    // Only the uniquely named temporary file created by this operation is removed.
    let _ = std::fs::remove_file(temporary);
    result
}
pub(crate) fn publish_exclusive(source: &Path, target: &Path) -> Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    unsafe extern "C" {
        fn renamex_np(
            from: *const std::ffi::c_char,
            to: *const std::ffi::c_char,
            flags: u32,
        ) -> i32;
    }
    let source = CString::new(source.as_os_str().as_bytes())?;
    let target = CString::new(target.as_os_str().as_bytes())?;
    // SDK sys/stdio.h: RENAME_EXCL = 0x4. This also works when hard links are unavailable.
    if unsafe { renamex_np(source.as_ptr(), target.as_ptr(), 4) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
fn clone_exclusive(source: &Path, target: &Path) -> Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    unsafe extern "C" {
        fn clonefile(from: *const std::ffi::c_char, to: *const std::ffi::c_char, flags: u32)
            -> i32;
    }
    let source = CString::new(source.as_os_str().as_bytes())?;
    let target = CString::new(target.as_os_str().as_bytes())?;
    // SDK sys/clonefile.h: CLONE_NOFOLLOW. Existing destinations are rejected.
    if unsafe { clonefile(source.as_ptr(), target.as_ptr(), 1) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
fn checksum(path: &Path, cancel: &AtomicBool) -> Result<String> {
    let mut input = std::fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 131072];
    loop {
        anyhow::ensure!(!cancel.load(Ordering::Acquire), "下载已暂停");
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
pub fn filename(artist: &str, title: &str, id: &str, format: &str) -> String {
    let raw = if artist.is_empty() {
        title.to_owned()
    } else {
        format!("{artist} - {title}")
    };
    let mut base = String::new();
    for c in raw.chars() {
        let c = if c.is_control() || "/\\:".contains(c) {
            '_'
        } else {
            c
        };
        if base.len() + c.len_utf8() > 160 {
            break;
        }
        base.push(c);
    }
    let base = base.trim_matches(|c: char| c == '.' || c.is_whitespace());
    let base = if base.is_empty() { "Music" } else { base };
    format!("{base} [{}].{format}", &id[..id.len().min(12)])
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_export_is_exact_idempotent_and_never_overwrites() {
        let root =
            std::env::temp_dir().join(format!("rmusic-export-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("source");
        std::fs::write(&source, b"original-audio-bytes").unwrap();
        let stop = AtomicBool::new(false);
        let digest = checksum(&source, &stop).unwrap();
        let target = publish(&source, &root, "song.flac", 20, &digest, &stop).unwrap();
        let copied = copy_verified(
            &source,
            &root,
            &root.join("copied.flac"),
            20,
            &digest,
            &stop,
        )
        .unwrap();
        assert_eq!(std::fs::read(copied).unwrap(), b"original-audio-bytes");
        assert_eq!(std::fs::read(&target).unwrap(), b"original-audio-bytes");
        assert_eq!(
            publish(&source, &root, "song.flac", 20, &digest, &stop).unwrap(),
            target
        );
        std::fs::write(&target, b"edited-export").unwrap();
        assert_eq!(std::fs::read(&source).unwrap(), b"original-audio-bytes");
        std::fs::write(root.join("other.flac"), b"user-file").unwrap();
        assert!(publish(&source, &root, "other.flac", 20, &digest, &stop).is_err());
        assert_eq!(
            std::fs::read(root.join("other.flac")).unwrap(),
            b"user-file"
        );
        assert!(publish_exclusive(&source, &root.join("other.flac")).is_err());
        assert_eq!(
            std::fs::read(root.join("other.flac")).unwrap(),
            b"user-file"
        );
        assert!(!filename("../Artist", "Name/xx", "0123456789abcdef", "flac").contains('/'));
    }
}
