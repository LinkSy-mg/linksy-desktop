//! Network changes, as the kernel tells them: when this computer's addresses change, the
//! node checks its links at once instead of waiting for them to time out.

use std::io;
use std::net::IpAddr;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::time::Duration;

use linksy_core::Node;
use tokio::io::Interest;
use tokio::io::unix::AsyncFd;
use tracing::{debug, warn};

/// Addresses come and go in bursts (joining Wi-Fi, DHCP, IPv6): what they settle on counts.
const SETTLE: Duration = Duration::from_secs(1);

pub async fn run(node: Node) {
    // SAFETY: an OwnedFd keeps its descriptor open, and the same one, until it's dropped
    let socket = match subscribe().and_then(|fd| unsafe { AsyncFd::register_with_interest(fd, Interest::READABLE) }.map_err(|e| e.into_parts().1)) {
        Ok(s) => s,
        Err(e) => return warn!("network changes won't be noticed: {e}"),
    };
    let mut known = addresses();
    loop {
        let Ok(mut ready) = socket.readable().await else { return };
        drain(&socket);
        ready.clear_ready();
        tokio::time::sleep(SETTLE).await;
        drain(&socket);
        // Lifetimes renewed or a router's announcement repeated: nothing changed
        let now = addresses();
        if now != known {
            debug!("network changed: {now:?}");
            known = now;
            node.network_changed();
        }
    }
}

fn addresses() -> Vec<IpAddr> {
    let mut all: Vec<IpAddr> = if_addrs::get_if_addrs().unwrap_or_default().iter().filter(|i| !i.is_loopback()).map(|i| i.ip()).collect();
    all.sort();
    all
}

/// A netlink socket that hears of addresses added or removed.
fn subscribe() -> io::Result<OwnedFd> {
    // SAFETY: a new socket, owned from here on
    let fd = unsafe { libc::socket(libc::AF_NETLINK, libc::SOCK_RAW | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC, libc::NETLINK_ROUTE) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: just opened, and nothing else holds it
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    // SAFETY: an all-zero sockaddr_nl is valid
    let mut addr: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
    addr.nl_family = libc::AF_NETLINK as libc::sa_family_t;
    addr.nl_groups = (libc::RTMGRP_IPV4_IFADDR | libc::RTMGRP_IPV6_IFADDR) as u32;
    // SAFETY: `addr` is a sockaddr_nl of the size given
    let bound = unsafe { libc::bind(fd.as_raw_fd(), (&raw const addr).cast(), std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t) };
    if bound < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(fd)
}

/// Read what's waiting: only that something came matters.
fn drain(socket: &AsyncFd<OwnedFd>) {
    let mut buf = [0u8; 4096];
    // SAFETY: reads into `buf`, at most its size
    while unsafe { libc::recv(socket.get_ref().as_raw_fd(), buf.as_mut_ptr().cast(), buf.len(), libc::MSG_DONTWAIT) } > 0 {}
}
