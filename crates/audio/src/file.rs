use std::{fs::File, io, path::Path};

/// Open a local source, including a held Android document-provider descriptor.
pub fn open(path: impl AsRef<Path>) -> io::Result<File> {
    let path = path.as_ref();
    #[cfg(target_os = "android")]
    if let Some(fd) = path.to_str().and_then(|p| p.strip_prefix("/proc/self/fd/")) {
        use std::{io::{Seek, SeekFrom}, os::fd::BorrowedFd};
        let fd: i32 = fd.parse().map_err(|_| io::Error::other("invalid document descriptor"))?;
        if fd < 0 { return Err(io::Error::other("invalid document descriptor")); }
        // The cache lease owns the original descriptor throughout probing and playback.
        let mut file = File::from(unsafe { BorrowedFd::borrow_raw(fd) }.try_clone_to_owned()?);
        file.seek(SeekFrom::Start(0))?;
        return Ok(file);
    }
    File::open(path)
}
