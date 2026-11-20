//! App-owned transport sockets follow the physical service instead of another VPN's TUN.
use core_foundation::{
    base::{CFType, TCFType},
    dictionary::CFDictionary,
    propertylist::CFPropertyList,
    string::CFString,
};
use std::{
    ffi::CString,
    io,
    net::SocketAddr,
    os::fd::{AsRawFd, RawFd},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
use system_configuration::dynamic_store::SCDynamicStoreBuilder;

struct Interfaces {
    at: Instant,
    v4: Option<String>,
    v6: Option<String>,
}
fn primary(family: i32) -> Option<String> {
    static CACHE: OnceLock<Mutex<Interfaces>> = OnceLock::new();
    let mut cached = CACHE
        .get_or_init(|| {
            Mutex::new(Interfaces {
                at: Instant::now() - Duration::from_secs(2),
                v4: None,
                v6: None,
            })
        })
        .lock()
        .ok()?;
    if cached.at.elapsed() >= Duration::from_secs(1) {
        let store = SCDynamicStoreBuilder::new("R-Music physical transport").build()?;
        let lookup = |key| -> Option<String> {
            let dictionary = store
                .get(key)
                .and_then(CFPropertyList::downcast_into::<CFDictionary>)?;
            let name = CFString::from_static_string("PrimaryInterface");
            let value = dictionary.find(name.as_CFTypeRef())?;
            let value = unsafe { CFType::wrap_under_get_rule(*value) }
                .downcast::<CFString>()?
                .to_string();
            (value.starts_with("en") || value.starts_with("bridge")).then_some(value)
        };
        cached.v4 = lookup("State:/Network/Global/IPv4");
        cached.v6 = lookup("State:/Network/Global/IPv6");
        cached.at = Instant::now();
    }
    if family == libc::AF_INET6 {
        cached.v6.clone().or_else(|| cached.v4.clone())
    } else {
        cached.v4.clone()
    }
}
pub fn physical_interface(family: i32) -> io::Result<String> {
    primary(family)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NetworkUnreachable, "没有可用的物理网络出口"))
}
pub fn protect_socket(fd: RawFd) -> io::Result<()> {
    let mut address: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    let mut length = std::mem::size_of_val(&address) as libc::socklen_t;
    if unsafe {
        libc::getsockname(
            fd,
            (&mut address as *mut libc::sockaddr_storage).cast(),
            &mut length,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    let family = i32::from(address.ss_family);
    let (level, option) = match family {
        libc::AF_INET => (libc::IPPROTO_IP, libc::IP_BOUND_IF),
        libc::AF_INET6 => (libc::IPPROTO_IPV6, libc::IPV6_BOUND_IF),
        _ => return Ok(()),
    };
    let name = CString::new(physical_interface(family)?).map_err(io::Error::other)?;
    let index = unsafe { libc::if_nametoindex(name.as_ptr()) };
    if index == 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe {
        libc::setsockopt(
            fd,
            level,
            option,
            (&index as *const u32).cast(),
            std::mem::size_of_val(&index) as libc::socklen_t,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
pub async fn connect(address: SocketAddr) -> io::Result<tokio::net::TcpStream> {
    // Local API facades and explicit VPN addresses retain their own routes.
    if !crate::config::public_ip(address.ip()) {
        return tokio::net::TcpStream::connect(address).await;
    }
    let physical = async {
        let socket = if address.is_ipv4() {
            tokio::net::TcpSocket::new_v4()?
        } else {
            tokio::net::TcpSocket::new_v6()?
        };
        protect_socket(socket.as_raw_fd())?;
        socket.connect(address).await
    };
    let physical = tokio::time::timeout(Duration::from_secs(3), physical);
    tokio::pin!(physical);
    tokio::select! {
        result = &mut physical => match result {
            Ok(Ok(stream)) => Ok(stream),
            _ => tokio::net::TcpStream::connect(address).await,
        },
        _ = tokio::time::sleep(Duration::from_millis(250)) => {
            // HTTPS can require a proxy on some networks. Race the ordinary
            // route after a short head start; NAT probes stay physical-only.
            let ordinary = tokio::net::TcpStream::connect(address);
            tokio::pin!(ordinary);
            tokio::select! {
                result = &mut physical => match result {
                    Ok(Ok(stream)) => Ok(stream),
                    _ => ordinary.await,
                },
                result = &mut ordinary => match result {
                    Ok(stream) => Ok(stream),
                    Err(error) => match physical.await {
                        Ok(Ok(stream)) => Ok(stream),
                        _ => Err(error),
                    },
                },
            }
        },
    }
}
