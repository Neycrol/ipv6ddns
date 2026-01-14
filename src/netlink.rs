//! IPv6 address monitoring using netlink socket or polling fallback
//!
//! Primary method: NETLINK_ROUTE to receive RTM_NEWADDR/RTM_DELADDR events
//! Fallback: Periodic polling with configurable interval
//! Event-driven design means zero CPU usage when no network changes occur.

use std::io::ErrorKind;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use tokio::io::unix::AsyncFd;

const NETLINK_ROUTE: i32 = libc::AF_NETLINK as i32;
const SOCK_RAW: i32 = libc::SOCK_RAW;
const SOCK_CLOEXEC: i32 = libc::SOCK_CLOEXEC;
const NETLINK_ROUTE_PROTOCOL: i32 = libc::NETLINK_ROUTE as i32;
const RTMGRP_IPV6_ADDR: u32 = 1 << 1;
const NLM_F_REQUEST: u16 = 0x0001;
const NLM_F_DUMP: u16 = 0x0300;

const RTM_NEWADDR_VAL: u16 = libc::RTM_NEWADDR as u16;
const RTM_DELADDR_VAL: u16 = libc::RTM_DELADDR as u16;
const RTM_GETADDR_VAL: u16 = libc::RTM_GETADDR as u16;
const IFA_ADDRESS_VAL: u16 = libc::IFA_ADDRESS as u16;
const IFA_LOCAL_VAL: u16 = libc::IFA_LOCAL as u16;
const NLMSG_HDRLEN: usize = 16;
const IFADDRMSG_LEN: usize = 8;
const ALIGN_TO: usize = 4;

const POLL_INTERVAL_DEFAULT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetlinkEvent {
    Ipv6Added(String),
    Ipv6Removed,
    Unknown,
}

#[async_trait]
pub trait Ipv6Monitor: Send + Sync {
    async fn next_event(&mut self) -> NetlinkEvent;
    #[allow(dead_code)]
    fn is_event_driven(&self) -> bool;
}

struct NetlinkImpl {
    fd: AsyncFd<OwnedFd>,
}

impl NetlinkImpl {
    fn new() -> Result<Self> {
        let fd = unsafe {
            libc::socket(
                NETLINK_ROUTE,
                SOCK_RAW | SOCK_CLOEXEC,
                NETLINK_ROUTE_PROTOCOL,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error())
                .context("create netlink socket")
                .map_err(Into::into);
        }

        let mut addr: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
        addr.nl_family = NETLINK_ROUTE as libc::sa_family_t;
        addr.nl_groups = RTMGRP_IPV6_ADDR;
        addr.nl_pid = 0;

        let res = unsafe {
            libc::bind(
                fd,
                &addr as *const _ as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t,
            )
        };
        if res < 0 {
            let err = std::io::Error::last_os_error();
            unsafe { libc::close(fd) };
            return Err(err)
                .context("netlink bind")
                .map_err(Into::into);
        }

        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 {
            unsafe { libc::close(fd) };
            return Err(std::io::Error::last_os_error())
                .context("fcntl F_GETFL")
                .map_err(Into::into);
        }
        if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            unsafe { libc::close(fd) };
            return Err(std::io::Error::last_os_error())
                .context("fcntl F_SETFL")
                .map_err(Into::into);
        }

        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        let fd = AsyncFd::new(fd).context("AsyncFd")?;
        Ok(Self { fd })
    }

    fn recv_raw_io(&self) -> std::io::Result<Option<Vec<u8>>> {
        let mut buf = vec![0u8; 8192];
        let n = unsafe {
            libc::recv(
                self.fd.as_raw_fd(),
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len(),
                0,
            )
        };
        if n < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == ErrorKind::WouldBlock {
                return Ok(None);
            }
            return Err(err);
        }
        if n == 0 {
            return Ok(None);
        }
        buf.truncate(n as usize);
        Ok(Some(buf))
    }


    fn parse_message(&self, data: &[u8]) -> Option<NetlinkEvent> {
        let mut msg_offset = 0usize;

        while msg_offset + NLMSG_HDRLEN <= data.len() {
            let nlmsg_len =
                u32::from_ne_bytes(data[msg_offset..msg_offset + 4].try_into().unwrap()) as usize;
            if nlmsg_len < NLMSG_HDRLEN {
                break;
            }
            if nlmsg_len == 0 {
                break;
            }

            let nlmsg_type =
                u16::from_ne_bytes(data[msg_offset + 4..msg_offset + 6].try_into().unwrap());

            if nlmsg_type == libc::NLMSG_DONE as u16 || nlmsg_type == libc::NLMSG_ERROR as u16 {
                msg_offset += nlmsg_align(nlmsg_len);
                continue;
            }

            if nlmsg_type != RTM_NEWADDR_VAL && nlmsg_type != RTM_DELADDR_VAL {
                msg_offset += nlmsg_align(nlmsg_len);
                continue;
            }

            let msg_end = (msg_offset + nlmsg_len).min(data.len());
            if msg_end < msg_offset + NLMSG_HDRLEN + IFADDRMSG_LEN {
                msg_offset += nlmsg_align(nlmsg_len);
                continue;
            }

            let ifa_offset = msg_offset + NLMSG_HDRLEN;
            let ifa_family = data[ifa_offset];
            let ifa_flags = data[ifa_offset + 2];
            let ifa_scope = data[ifa_offset + 3];

            if ifa_family != libc::AF_INET6 as u8 {
                msg_offset += nlmsg_align(nlmsg_len);
                continue;
            }
            if ifa_scope != libc::RT_SCOPE_UNIVERSE as u8 {
                msg_offset += nlmsg_align(nlmsg_len);
                continue;
            }
            if (ifa_flags as u32) & (libc::IFA_F_TEMPORARY as u32) != 0 {
                msg_offset += nlmsg_align(nlmsg_len);
                continue;
            }
            if (ifa_flags as u32) & (libc::IFA_F_TENTATIVE as u32) != 0 {
                msg_offset += nlmsg_align(nlmsg_len);
                continue;
            }
            if (ifa_flags as u32) & (libc::IFA_F_DADFAILED as u32) != 0 {
                msg_offset += nlmsg_align(nlmsg_len);
                continue;
            }
            if (ifa_flags as u32) & (libc::IFA_F_DEPRECATED as u32) != 0 {
                msg_offset += nlmsg_align(nlmsg_len);
                continue;
            }

            let mut rta_offset = msg_offset + NLMSG_HDRLEN + IFADDRMSG_LEN;
            while rta_offset + 4 <= msg_end {
                let rta_len = u16::from_ne_bytes([
                    data[rta_offset],
                    data[rta_offset + 1],
                ]) as usize;
                if rta_len < 4 {
                    break;
                }
                let rta_type = u16::from_ne_bytes([
                    data[rta_offset + 2],
                    data[rta_offset + 3],
                ]);

                let payload_len = rta_len - 4;
                let payload_offset = rta_offset + 4;
                if payload_offset + payload_len > msg_end {
                    break;
                }

                if (rta_type == IFA_ADDRESS_VAL || rta_type == IFA_LOCAL_VAL) && payload_len == 16 {
                    let addr: [u8; 16] = match data[payload_offset..payload_offset + 16].try_into() {
                        Ok(a) => a,
                        Err(_) => return None,
                    };
                    let ip = std::net::Ipv6Addr::from(addr);
                    let event = match nlmsg_type {
                        RTM_NEWADDR_VAL => NetlinkEvent::Ipv6Added(ip.to_string()),
                        RTM_DELADDR_VAL => NetlinkEvent::Ipv6Removed,
                        _ => NetlinkEvent::Unknown,
                    };
                    return Some(event);
                }

                rta_offset += rta_align(rta_len);
            }

            msg_offset += nlmsg_align(nlmsg_len);
        }

        None
    }
}

#[async_trait]
impl Ipv6Monitor for NetlinkImpl {
    async fn next_event(&mut self) -> NetlinkEvent {
        loop {
            let mut guard = match self.fd.readable().await {
                Ok(g) => g,
                Err(_) => return NetlinkEvent::Unknown,
            };

            let data = match guard.try_io(|_| self.recv_raw_io()) {
                Ok(Ok(Some(d))) => d,
                Ok(Ok(None)) => continue,
                Ok(Err(_)) => return NetlinkEvent::Unknown,
                Err(_would_block) => continue,
            };

            if let Some(event) = self.parse_message(&data) {
                return event;
            }
        }
    }

    fn is_event_driven(&self) -> bool {
        true
    }
}

struct PollingImpl {
    interval: Duration,
    running: Arc<AtomicBool>,
    last_ip: Option<String>,
}

impl PollingImpl {
    fn new(interval: Duration, running: Arc<AtomicBool>) -> Self {
        Self {
            interval,
            running,
            last_ip: None,
        }
    }
}

#[async_trait]
impl Ipv6Monitor for PollingImpl {
    #[allow(unused)]
    async fn next_event(&mut self) -> NetlinkEvent {
        loop {
            if !self.running.load(Ordering::Relaxed) {
                return NetlinkEvent::Unknown;
            }

            tokio::time::sleep(self.interval).await;

            let current_ip = detect_global_ipv6();

            match (&self.last_ip, &current_ip) {
                (None, Some(ip)) => {
                    self.last_ip = Some(ip.clone());
                    return NetlinkEvent::Ipv6Added(ip.clone());
                }
                (Some(_), None) => {
                    self.last_ip = None;
                    return NetlinkEvent::Ipv6Removed;
                }
                (Some(old), Some(new)) if old != new => {
                    self.last_ip = Some(new.clone());
                    return NetlinkEvent::Ipv6Added(new.clone());
                }
                (Some(old), Some(ip)) if ip == old => {
                    self.last_ip = Some(ip.clone());
                }
                _ => {}
            }
        }
    }

    fn is_event_driven(&self) -> bool {
        false
    }
}

pub struct NetlinkSocket {
    monitor: Box<dyn Ipv6Monitor>,
    is_event_driven: bool,
}

impl NetlinkSocket {
    pub fn new(poll_interval: Option<Duration>) -> Result<Self> {
        let interval = poll_interval.unwrap_or(POLL_INTERVAL_DEFAULT);

        match NetlinkImpl::new() {
            Ok(netlink) => {
                tracing::info!("Using event-driven netlink socket");
                Ok(Self {
                    monitor: Box::new(netlink),
                    is_event_driven: true,
                })
            }
            Err(e) => {
                tracing::warn!("Netlink socket failed ({:#}), falling back to polling", e);
                tracing::info!("Polling interval: {} seconds", interval.as_secs());
                Ok(Self {
                    monitor: Box::new(PollingImpl::new(
                        interval,
                        Arc::new(AtomicBool::new(true)),
                    )),
                    is_event_driven: false,
                })
            }
        }
    }

    pub async fn recv(&mut self) -> Result<NetlinkEvent> {
        Ok(self.monitor.next_event().await)
    }

    pub fn is_event_driven(&self) -> bool {
        self.is_event_driven
    }
}

pub fn detect_global_ipv6() -> Option<String> {
    match netlink_dump_ipv6() {
        Ok((stable, temporary)) => stable.or(temporary),
        Err(_) => None,
    }
}

fn nlmsg_align(len: usize) -> usize {
    (len + ALIGN_TO - 1) & !(ALIGN_TO - 1)
}

fn rta_align(len: usize) -> usize {
    (len + ALIGN_TO - 1) & !(ALIGN_TO - 1)
}

fn netlink_dump_ipv6() -> Result<(Option<String>, Option<String>)> {
    let fd = unsafe { libc::socket(NETLINK_ROUTE, SOCK_RAW | SOCK_CLOEXEC, NETLINK_ROUTE_PROTOCOL) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error())
            .context("create netlink socket")
            .map_err(Into::into);
    }

    let mut addr: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
    addr.nl_family = NETLINK_ROUTE as libc::sa_family_t;
    addr.nl_groups = 0;
    addr.nl_pid = 0;

    let res = unsafe {
        libc::bind(
            fd,
            &addr as *const _ as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t,
        )
    };
    if res < 0 {
        let err = std::io::Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(err)
            .context("netlink bind")
            .map_err(Into::into);
    }

    let seq = 1u32;
    let mut buf = vec![0u8; NLMSG_HDRLEN + IFADDRMSG_LEN];
    let nlmsg_len = (NLMSG_HDRLEN + IFADDRMSG_LEN) as u32;
    buf[0..4].copy_from_slice(&nlmsg_len.to_ne_bytes());
    buf[4..6].copy_from_slice(&RTM_GETADDR_VAL.to_ne_bytes());
    buf[6..8].copy_from_slice(&(NLM_F_REQUEST | NLM_F_DUMP).to_ne_bytes());
    buf[8..12].copy_from_slice(&seq.to_ne_bytes());
    buf[12..16].copy_from_slice(&0u32.to_ne_bytes());
    buf[16] = libc::AF_INET6 as u8;

    let send_res = unsafe {
        libc::send(
            fd,
            buf.as_ptr() as *const libc::c_void,
            buf.len(),
            0,
        )
    };
    if send_res < 0 {
        let err = std::io::Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(err).context("netlink send").map_err(Into::into);
    }

    let mut stable: Option<String> = None;
    let mut temporary: Option<String> = None;
    let mut recv_buf = vec![0u8; 16384];

    loop {
        let n = unsafe {
            libc::recv(
                fd,
                recv_buf.as_mut_ptr() as *mut libc::c_void,
                recv_buf.len(),
                0,
            )
        };
        if n < 0 {
            let err = std::io::Error::last_os_error();
            unsafe { libc::close(fd) };
            return Err(err).context("netlink recv").map_err(Into::into);
        }
        if n == 0 {
            break;
        }

        let data = &recv_buf[..n as usize];
        let mut msg_offset = 0usize;
        while msg_offset + NLMSG_HDRLEN <= data.len() {
            let nlmsg_len =
                u32::from_ne_bytes(data[msg_offset..msg_offset + 4].try_into().unwrap()) as usize;
            if nlmsg_len < NLMSG_HDRLEN || nlmsg_len == 0 {
                break;
            }

            let nlmsg_type =
                u16::from_ne_bytes(data[msg_offset + 4..msg_offset + 6].try_into().unwrap());
            if nlmsg_type == libc::NLMSG_DONE as u16 {
                unsafe { libc::close(fd) };
                return Ok((stable, temporary));
            }
            if nlmsg_type == libc::NLMSG_ERROR as u16 {
                unsafe { libc::close(fd) };
                return Err(anyhow::anyhow!("netlink error response"));
            }

            if nlmsg_type == RTM_NEWADDR_VAL {
                let msg_end = (msg_offset + nlmsg_len).min(data.len());
                if msg_end >= msg_offset + NLMSG_HDRLEN + IFADDRMSG_LEN {
                    let ifa_offset = msg_offset + NLMSG_HDRLEN;
                    let ifa_family = data[ifa_offset];
                    let ifa_flags = data[ifa_offset + 2];
                    let ifa_scope = data[ifa_offset + 3];

                    if ifa_family == libc::AF_INET6 as u8
                        && ifa_scope == libc::RT_SCOPE_UNIVERSE as u8
                        && (ifa_flags as u32 & libc::IFA_F_TENTATIVE as u32) == 0
                        && (ifa_flags as u32 & libc::IFA_F_DADFAILED as u32) == 0
                        && (ifa_flags as u32 & libc::IFA_F_DEPRECATED as u32) == 0
                    {
                        let is_temp = (ifa_flags as u32 & libc::IFA_F_TEMPORARY as u32) != 0;

                        let mut rta_offset = msg_offset + NLMSG_HDRLEN + IFADDRMSG_LEN;
                        while rta_offset + 4 <= msg_end {
                            let rta_len = u16::from_ne_bytes([
                                data[rta_offset],
                                data[rta_offset + 1],
                            ]) as usize;
                            if rta_len < 4 {
                                break;
                            }
                            let rta_type = u16::from_ne_bytes([
                                data[rta_offset + 2],
                                data[rta_offset + 3],
                            ]);
                            let payload_len = rta_len - 4;
                            let payload_offset = rta_offset + 4;
                            if payload_offset + payload_len > msg_end {
                                break;
                            }

                            if (rta_type == IFA_ADDRESS_VAL || rta_type == IFA_LOCAL_VAL)
                                && payload_len == 16
                            {
                                let addr: [u8; 16] =
                                    match data[payload_offset..payload_offset + 16].try_into() {
                                        Ok(a) => a,
                                        Err(_) => break,
                                    };
                                let ip = std::net::Ipv6Addr::from(addr).to_string();
                                if is_temp {
                                    if temporary.is_none() {
                                        temporary = Some(ip);
                                    }
                                } else if stable.is_none() {
                                    stable = Some(ip);
                                }
                                break;
                            }

                            rta_offset += rta_align(rta_len);
                        }
                    }
                }
            }

            msg_offset += nlmsg_align(nlmsg_len);
        }
    }

    unsafe { libc::close(fd) };
    Ok((stable, temporary))
}
