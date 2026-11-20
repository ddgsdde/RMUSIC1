//! Kernel notifications only wake reconciliation. The actual addresses and PEM are always re-read.
use std::{path::Path, time::Duration};

#[cfg(target_os="linux")]
mod linux {
    use std::{collections::HashSet, ffi::CString, io, os::{fd::{AsRawFd,FromRawFd,OwnedFd},unix::ffi::OsStrExt}, path::Path};
    use tokio::io::unix::AsyncFd;

    pub(super) struct Events {
        fd: AsyncFd<OwnedFd>,
        // Directory descriptor + basename. An empty set denotes a netlink socket.
        files: HashSet<(i32, Vec<u8>)>,
    }
    fn owned(raw:i32)->io::Result<OwnedFd>{
        if raw<0 { return Err(io::Error::last_os_error()); }
        Ok(unsafe { OwnedFd::from_raw_fd(raw) })
    }
    impl Events {
        pub(super) fn addresses()->io::Result<Self>{
            let fd=owned(unsafe{libc::socket(libc::AF_NETLINK,libc::SOCK_RAW|libc::SOCK_NONBLOCK|libc::SOCK_CLOEXEC,libc::NETLINK_ROUTE)})?;
            let mut addr:libc::sockaddr_nl=unsafe{std::mem::zeroed()};
            addr.nl_family=libc::AF_NETLINK as _;
            addr.nl_groups=(libc::RTMGRP_IPV6_IFADDR|libc::RTMGRP_IPV6_ROUTE|libc::RTMGRP_LINK) as _;
            if unsafe{libc::bind(fd.as_raw_fd(),(&addr as *const libc::sockaddr_nl).cast(),std::mem::size_of_val(&addr) as _)}<0 {return Err(io::Error::last_os_error());}
            Ok(Self{fd:AsyncFd::new(fd)?,files:Default::default()})
        }
        pub(super) fn files(paths:&[&Path])->io::Result<Self>{
            let fd=owned(unsafe{libc::inotify_init1(libc::IN_NONBLOCK|libc::IN_CLOEXEC)})?;
            let mut files=HashSet::new();
            for path in paths {
                // Support in-place writes, atomically replaced files, and certificate-manager symlinks.
                for p in std::iter::once(path.to_path_buf()).chain(path.canonicalize().ok()) {
                    let (Some(parent),Some(name))=(p.parent(),p.file_name()) else {continue};
                    let dir=CString::new(parent.as_os_str().as_bytes())?;
                    let mask=libc::IN_CLOSE_WRITE|libc::IN_MOVED_TO|libc::IN_MOVED_FROM|libc::IN_CREATE|libc::IN_DELETE|libc::IN_MOVE_SELF|libc::IN_DELETE_SELF;
                    let wd=unsafe{libc::inotify_add_watch(fd.as_raw_fd(),dir.as_ptr(),mask)};
                    if wd>=0 { files.insert((wd,name.as_bytes().to_vec())); }
                }
            }
            if files.is_empty(){return Err(io::Error::new(io::ErrorKind::NotFound,"certificate directory unavailable"));}
            Ok(Self{fd:AsyncFd::new(fd)?,files})
        }
        pub(super) async fn changed(&self)->io::Result<()> {
            let mut bytes=[0u8;8192];
            loop {
                let mut ready=self.fd.readable().await?;
                let read=ready.try_io(|fd| {
                    let n=unsafe{libc::read(fd.get_ref().as_raw_fd(),bytes.as_mut_ptr().cast(),bytes.len())};
                    if n<0 {Err(io::Error::last_os_error())} else {Ok(n as usize)}
                });
                let Ok(size)=read else {continue};let size=size?;
                if size==0 {return Err(io::ErrorKind::UnexpectedEof.into());}
                if self.files.is_empty(){return Ok(());}
                let mut data=&bytes[..size];
                while data.len()>=std::mem::size_of::<libc::inotify_event>() {
                    let event=unsafe{data.as_ptr().cast::<libc::inotify_event>().read_unaligned()};
                    let head=std::mem::size_of_val(&event);let end=head+event.len as usize;
                    if end>data.len(){return Ok(());}
                    let name=data[head..end].split(|v|*v==0).next().unwrap_or_default();
                    if name.is_empty() || self.files.contains(&(event.wd,name.to_vec())) {return Ok(());}
                    data=&data[end..];
                }
            }
        }
    }
}

pub(super) struct Changes {
    #[cfg(target_os="linux")]
    events: Option<linux::Events>,
}
impl Changes {
    pub(super) fn addresses()->Self { Self {
        #[cfg(target_os="linux")] events:linux::Events::addresses().ok()
    }}
    pub(super) fn files(_paths:&[&Path])->Self { Self {
        #[cfg(target_os="linux")] events:linux::Events::files(_paths).ok()
    }}
    pub(super) fn fallback_interval(&self)->Duration {
        #[cfg(target_os="linux")]
        if self.events.is_some(){return Duration::from_secs(300);}
        Duration::from_secs(15)
    }
    pub(super) async fn wait(&self, fallback:Duration){
        #[cfg(target_os="linux")]
        if let Some(events)=&self.events {
            match tokio::time::timeout(fallback,events.changed()).await {
                Ok(Ok(()))=>{tokio::time::sleep(Duration::from_millis(250)).await;return},
                Err(_)=>return,
                Ok(Err(_))=>{}
            }
        }
        tokio::time::sleep(fallback).await;
    }
}

#[cfg(all(test,target_os="linux"))]
mod tests {
    #[tokio::test]
    async fn certificate_atomic_replace_wakes_without_waiting_for_fallback(){
        let root=tempfile::tempdir().unwrap();let cert=root.path().join("cert.pem");
        std::fs::write(&cert,b"old").unwrap();
        let events=super::Changes::files(&[&cert]);
        let replacement=root.path().join("next");std::fs::write(&replacement,b"new").unwrap();std::fs::rename(replacement,&cert).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2),events.wait(std::time::Duration::from_secs(300))).await.unwrap();
    }
}
