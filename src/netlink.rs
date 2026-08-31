//! IPv6 address monitoring using netlink socket or polling fallback
//!
//! This module provides two methods for monitoring IPv6 address changes:
//!
//! 1. **Event-driven (netlink)**: Uses NETLINK_ROUTE to receive RTM_NEWADDR/RTM_DELADDR
//!    events. This is the preferred method as it has zero CPU usage when no network
//!    changes occur.
//!
//! 2. **Polling fallback**: Periodic polling with configurable interval. This is used
//!    when netlink is not available (e.g., in some containerized environments).
//!
//! # Features
//!
//! - Automatic filtering of temporary, tentative, deprecated, and DAD-failed addresses
//! - Automatic fallback to polling if netlink is unavailable
//! - Zero CPU usage when idle (event-driven mode)
//! - Configurable polling interval
//! - Support for loopback addresses (optional)
//!
//! # Usage
//!
//! ```text
//! use ipv6ddns::netlink::NetlinkSocket;
//! use std::time::Duration;
//!
//! let socket = NetlinkSocket::new(Some(Duration::from_secs(60)), false)?;
//! loop {
//!     match socket.recv().await? {
//!         NetlinkEvent::Ipv6Added(ip) => println!("IPv6 added: {}", ip),
//!         NetlinkEvent::Ipv6Removed => println!("IPv6 removed"),
//!         NetlinkEvent::Unknown => {},
//!     }
//! }
//! ```
//!
//! # Address Filtering
//!
//! The module automatically filters out:
//! - Temporary addresses (privacy extensions)
//! - Tentative addresses (still undergoing DAD)
//! - Deprecated addresses
//! - DAD-failed addresses
//! - Non-global scope addresses (unless loopback is allowed)
//!
//! # Netlink Protocol
//!
//! The module uses the NETLINK_ROUTE protocol to subscribe to RTMGRP_IPV6_IFADDR
//! multicast group, which receives notifications for IPv6 address changes.

use std::collections::VecDeque;
use std::net::Ipv6Addr;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::time::Duration;

use anyhow::{Context as _, Result};
use tokio::io::unix::AsyncFd;

use crate::validation::is_valid_ipv6;

//==============================================================================
// RAII Socket Wrapper
//==============================================================================

/// RAII wrapper for netlink socket file descriptors
///
/// This wrapper ensures that the file descriptor is properly closed when
/// the wrapper is dropped, even if a panic occurs or there's an early return.
struct NetlinkFd(i32);

impl NetlinkFd {
    /// Creates a new netlink socket with proper error handling
    ///
    /// # Returns
    ///
    /// Returns `Ok(NetlinkFd)` containing the wrapped fd or an error if socket creation fails
    fn new() -> Result<Self> {
        let fd = unsafe {
            libc::socket(
                NETLINK_ROUTE,
                SOCK_RAW | SOCK_CLOEXEC,
                NETLINK_ROUTE_PROTOCOL,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error()).context("create netlink socket");
        }
        Ok(Self(fd))
    }

    /// Returns the raw file descriptor
    fn as_raw_fd(&self) -> i32 {
        self.0
    }
}

impl AsRawFd for NetlinkFd {
    fn as_raw_fd(&self) -> i32 {
        self.0
    }
}

impl Drop for NetlinkFd {
    fn drop(&mut self) {
        if self.0 >= 0 {
            unsafe {
                libc::close(self.0);
            }
        }
    }
}

//==============================================================================
// Netlink Constants
//==============================================================================

// Netlink constants
const NETLINK_ROUTE: i32 = libc::AF_NETLINK;
const SOCK_RAW: i32 = libc::SOCK_RAW;
const SOCK_CLOEXEC: i32 = libc::SOCK_CLOEXEC;
const NETLINK_ROUTE_PROTOCOL: i32 = libc::NETLINK_ROUTE;
const RTMGRP_IPV6_ADDR: u32 = libc::RTMGRP_IPV6_IFADDR as u32;
const NLM_F_REQUEST: u16 = 0x0001;
const NLM_F_DUMP: u16 = 0x0300;

// Netlink message types
const RTM_NEWADDR_VAL: u16 = libc::RTM_NEWADDR;
const RTM_DELADDR_VAL: u16 = libc::RTM_DELADDR;
const RTM_GETADDR_VAL: u16 = libc::RTM_GETADDR;

// Interface address attribute types
const IFA_ADDRESS_VAL: u16 = libc::IFA_ADDRESS;
const IFA_LOCAL_VAL: u16 = libc::IFA_LOCAL;

// Netlink message structure constants
const NLMSG_HDRLEN: usize = 16;
const IFADDRMSG_LEN: usize = 8;
const ALIGN_TO: usize = 4;

// Buffer sizes for netlink operations
const NETLINK_RECV_BUFFER_SIZE: usize = 8192;

/// Requested `SO_RCVBUF` for the netlink socket (1 MiB).
///
/// The kernel clamps this to `net.core.rmem_max` and only commits memory as
/// messages are actually queued, so asking for headroom costs nothing when
/// the queue is empty. Overflow itself is also handled via `ENOBUFS`.
const NETLINK_RCVBUF_REQUEST: libc::c_int = 1 << 20;
const NETLINK_DUMP_BUFFER_SIZE: usize = 16384;
const IPV6_ADDR_BYTES: usize = 16;

// Address family constants
const AF_INET6: u8 = libc::AF_INET6 as u8;
const RT_SCOPE_UNIVERSE: u8 = libc::RT_SCOPE_UNIVERSE;

// Address flag constants
const IFA_F_TEMPORARY: u32 = libc::IFA_F_TEMPORARY;
const IFA_F_TENTATIVE: u32 = libc::IFA_F_TENTATIVE;
const IFA_F_DADFAILED: u32 = libc::IFA_F_DADFAILED;
const IFA_F_DEPRECATED: u32 = libc::IFA_F_DEPRECATED;

// Netlink message type constants
const NLMSG_DONE: u16 = libc::NLMSG_DONE as u16;
const NLMSG_ERROR: u16 = libc::NLMSG_ERROR as u16;

// Attribute header size
const RTA_HEADER_SIZE: usize = 4;

// Default polling interval
const POLL_INTERVAL_DEFAULT: Duration = Duration::from_secs(60);

/// Represents a netlink event related to IPv6 address changes
///
/// This enum describes different types of events that can occur on
/// the network interface, such as IPv6 addresses being added or removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetlinkEvent {
    /// An IPv6 address was added or changed
    ///
    /// Carries the address as a native [`Ipv6Addr`] (16 bytes, `Copy`) so the
    /// hot event path performs no heap allocation; callers format to `String`
    /// only when they actually need text (logging or API payloads).
    Ipv6Added(Ipv6Addr),
    /// An IPv6 address was removed
    ///
    /// This event does not contain the specific address that was removed
    Ipv6Removed,
    /// An unknown or unhandled netlink event
    ///
    /// This is used for events that don't match the above categories
    Unknown,
    /// The kernel-side socket queue overflowed and messages were dropped
    ///
    /// Per netlink(7), netlink is unreliable: when the receive buffer fills,
    /// the kernel discards messages and reports `ENOBUFS`. User-space and
    /// kernel state have diverged, so consumers must re-detect the live
    /// state instead of relying on further events.
    Overflow,
}

struct NetlinkImpl {
    fd: AsyncFd<OwnedFd>,
    pending_events: VecDeque<NetlinkEvent>,
    /// Preallocated receive buffer, reused across reads so the event-driven
    /// hot path performs no heap allocation (and no 8 KiB zeroing) per event.
    recv_buf: Box<[u8; NETLINK_RECV_BUFFER_SIZE]>,
}

impl NetlinkImpl {
    fn new() -> Result<Self> {
        let socket = NetlinkFd::new()?;

        let mut addr: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
        addr.nl_family = NETLINK_ROUTE as libc::sa_family_t;
        addr.nl_groups = RTMGRP_IPV6_ADDR;
        addr.nl_pid = 0;

        let res = unsafe {
            libc::bind(
                socket.as_raw_fd(),
                &addr as *const _ as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t,
            )
        };
        if res < 0 {
            return Err(std::io::Error::last_os_error()).context("netlink bind");
        }

        // Best-effort receive-buffer headroom so short bursts of address
        // changes (e.g. an interface flap cycling through several addresses)
        // are less likely to overflow the kernel queue. Failure is harmless:
        // overflow is still detected and handled via `ENOBUFS`.
        let rcvbuf = NETLINK_RCVBUF_REQUEST;
        unsafe {
            libc::setsockopt(
                socket.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_RCVBUF,
                &rcvbuf as *const libc::c_int as *const libc::c_void,
                std::mem::size_of_val(&rcvbuf) as libc::socklen_t,
            )
        };

        let flags = unsafe { libc::fcntl(socket.as_raw_fd(), libc::F_GETFL) };
        if flags < 0 {
            return Err(std::io::Error::last_os_error()).context("fcntl F_GETFL");
        }
        if unsafe { libc::fcntl(socket.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(std::io::Error::last_os_error()).context("fcntl F_SETFL");
        }

        // Convert to OwnedFd and then to AsyncFd
        //
        // SAFETY: This is a safe ownership transfer from NetlinkFd to OwnedFd.
        // - The `socket` variable owns a valid, open file descriptor (validated above)
        // - We transfer ownership to `OwnedFd` using `from_raw_fd`
        // - We then call `std::mem::forget(socket)` to prevent NetlinkFd's Drop
        //   implementation from closing the fd (which would cause a double-close)
        // - The OwnedFd now has exclusive ownership and will close the fd when dropped
        // - This is a common pattern in Rust when converting RAII wrappers
        let owned_fd = unsafe { OwnedFd::from_raw_fd(socket.as_raw_fd()) };
        std::mem::forget(socket); // Prevent double-close
        let fd = AsyncFd::new(owned_fd).context("AsyncFd")?;
        Ok(Self {
            fd,
            pending_events: VecDeque::new(),
            recv_buf: Box::new([0u8; NETLINK_RECV_BUFFER_SIZE]),
        })
    }

    /// Reads pending bytes from the netlink socket into `buf`.
    ///
    /// Returns the number of bytes written into `buf`.
    ///
    /// # Critical contract
    ///
    /// `WouldBlock` **must propagate out of this function verbatim**. This
    /// function is called inside [`tokio::io::AsyncFd`]'s `try_io`, which
    /// intercepts `WouldBlock` to clear the socket's readiness. If we swallowed
    /// it here (e.g. by mapping it to a sentinel), readiness would never be
    /// cleared and `readable().await` would resolve instantly forever — a
    /// 100% CPU busy loop on the single-threaded runtime (observed in real
    /// testing, not just theory).
    fn recv_raw_fd(fd: i32, buf: &mut [u8]) -> std::io::Result<usize> {
        // SAFETY: `fd` is a valid open netlink socket and `buf` is writable for
        // `buf.len()` bytes.
        let n = unsafe { libc::recv(fd, buf.as_mut_ptr().cast::<libc::c_void>(), buf.len(), 0) };
        if n < 0 {
            // Note: ENOBUFS (kernel dropped queued messages, netlink(7)) has a
            // different error kind than WouldBlock, so callers can distinguish
            // "retry later" from "state diverged, re-detect".
            return Err(std::io::Error::last_os_error());
        }
        Ok(n as usize)
    }

    /// Parses a received buffer and appends events to `out`.
    ///
    /// Takes the destination queue as a parameter so callers can reuse their
    /// existing queue instead of allocating a fresh one per batch.
    fn parse_messages_into(data: &[u8], out: &mut VecDeque<NetlinkEvent>) {
        let mut msg_offset = 0usize;
        let events = out;

        while msg_offset + NLMSG_HDRLEN <= data.len() {
            // Safely extract nlmsg_len with bounds checking
            let Some(nlmsg_len_bytes) = data.get(msg_offset..msg_offset + 4) else {
                break;
            };
            let Some(nlmsg_len_arr) = <[u8; 4]>::try_from(nlmsg_len_bytes).ok() else {
                break;
            };
            let nlmsg_len = u32::from_ne_bytes(nlmsg_len_arr) as usize;
            if nlmsg_len < NLMSG_HDRLEN {
                break;
            }
            if nlmsg_len == 0 {
                break;
            }

            // Safely extract nlmsg_type with bounds checking
            let Some(nlmsg_type_bytes) = data.get(msg_offset + 4..msg_offset + 6) else {
                break;
            };
            let Some(nlmsg_type_arr) = <[u8; 2]>::try_from(nlmsg_type_bytes).ok() else {
                break;
            };
            let nlmsg_type = u16::from_ne_bytes(nlmsg_type_arr);

            if nlmsg_type == NLMSG_DONE || nlmsg_type == NLMSG_ERROR {
                msg_offset += nlmsg_align(nlmsg_len);
                continue;
            }

            if nlmsg_type != RTM_NEWADDR_VAL && nlmsg_type != RTM_DELADDR_VAL {
                msg_offset += nlmsg_align(nlmsg_len);
                continue;
            }

            // Use the helper function to extract IPv6 address
            if let Some(event) =
                extract_ipv6_from_ifaddrmsg(data, msg_offset, nlmsg_len, nlmsg_type)
            {
                events.push_back(event);
            }

            msg_offset += nlmsg_align(nlmsg_len);
        }
    }

    #[cfg(test)]
    fn parse_message(data: &[u8]) -> Option<NetlinkEvent> {
        let mut events = VecDeque::new();
        Self::parse_messages_into(data, &mut events);
        events.pop_front()
    }
}

impl NetlinkImpl {
    /// Waits for the next IPv6 address change event
    async fn next_event(&mut self) -> NetlinkEvent {
        loop {
            if let Some(event) = self.pending_events.pop_front() {
                return event;
            }

            let mut guard = match self.fd.readable().await {
                Ok(g) => g,
                Err(_) => return NetlinkEvent::Unknown,
            };

            let received = guard.try_io(|inner| {
                // Reuses the preallocated buffer: no allocation on the hot
                // path. Only `recv_buf` is captured, keeping the borrow
                // disjoint from the guard's borrow of `fd`.
                Self::recv_raw_fd(inner.get_ref().as_raw_fd(), self.recv_buf.as_mut())
            });
            match received {
                // tokio intercepted WouldBlock: readiness has been cleared, so
                // re-awaiting `readable()` below will properly sleep.
                Err(_would_block) => continue,
                Ok(Ok(n)) if n > 0 => {
                    drop(guard);
                    // Parse straight into the reused queue: no fresh
                    // VecDeque allocation per received batch.
                    Self::parse_messages_into(&self.recv_buf[..n], &mut self.pending_events);
                    if let Some(event) = self.pending_events.pop_front() {
                        return event;
                    }
                }
                Ok(Ok(_)) => {
                    // Zero-length datagram: nothing to parse, keep waiting.
                    drop(guard);
                }
                Ok(Err(e)) => {
                    return if e.raw_os_error() == Some(libc::ENOBUFS) {
                        NetlinkEvent::Overflow
                    } else {
                        NetlinkEvent::Unknown
                    };
                }
            }
        }
    }
}

struct PollingImpl {
    interval: Duration,
    allow_loopback: bool,
    last_ip: Option<Ipv6Addr>,
}

impl PollingImpl {
    fn new(interval: Duration, allow_loopback: bool) -> Self {
        Self {
            interval,
            allow_loopback,
            last_ip: None,
        }
    }
}

impl PollingImpl {
    /// Waits for the next IPv6 address change event
    async fn next_event(&mut self) -> NetlinkEvent {
        loop {
            tokio::time::sleep(self.interval).await;

            let current_ip = detect_global_ipv6(self.allow_loopback);
            if self.last_ip.as_ref() == current_ip.as_ref() {
                continue;
            }

            match current_ip {
                Some(ip) => {
                    // Ipv6Addr is Copy — no clone needed.
                    self.last_ip = Some(ip);
                    return NetlinkEvent::Ipv6Added(ip);
                }
                None => {
                    self.last_ip = None;
                    return NetlinkEvent::Ipv6Removed;
                }
            }
        }
    }
}

/// Socket for monitoring IPv6 address changes via netlink or polling
///
/// This struct provides a unified interface for IPv6 address monitoring,
/// automatically falling back to polling if netlink is not available.
pub struct NetlinkSocket {
    monitor: Monitor,
}

/// Internal dispatcher for the two monitoring strategies.
///
/// An enum instead of a trait object keeps dispatch static and makes the
/// (closed) set of strategies explicit.
enum Monitor {
    EventDriven(NetlinkImpl),
    Polling(PollingImpl),
}

impl Monitor {
    /// Waits for the next IPv6 address change event
    async fn next_event(&mut self) -> NetlinkEvent {
        match self {
            Self::EventDriven(monitor) => monitor.next_event().await,
            Self::Polling(monitor) => monitor.next_event().await,
        }
    }
}

impl NetlinkSocket {
    /// Creates a new netlink socket with optional polling fallback
    ///
    /// This method attempts to create an event-driven netlink socket for
    /// real-time IPv6 address change detection. If netlink is not available,
    /// it falls back to polling with the specified interval.
    ///
    /// # Arguments
    ///
    /// * `poll_interval` - Optional polling interval. Defaults to 60 seconds if None.
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing the `NetlinkSocket` or an error if initialization fails
    ///
    /// # Behavior
    ///
    /// - If netlink is available: Uses event-driven monitoring (zero CPU when idle)
    /// - If netlink is unavailable: Falls back to polling with the specified interval
    pub fn new(poll_interval: Option<Duration>, allow_loopback: bool) -> Result<Self> {
        let interval = poll_interval.unwrap_or(POLL_INTERVAL_DEFAULT);

        match NetlinkImpl::new() {
            Ok(netlink) => {
                tracing::info!("Using event-driven netlink socket");
                Ok(Self {
                    monitor: Monitor::EventDriven(netlink),
                })
            }
            Err(e) => {
                tracing::warn!("Netlink socket failed ({:#}), falling back to polling", e);
                tracing::info!("Polling interval: {} seconds", interval.as_secs());
                Ok(Self {
                    monitor: Monitor::Polling(PollingImpl::new(interval, allow_loopback)),
                })
            }
        }
    }

    /// Receives the next IPv6 address change event
    ///
    /// This method is async and will block until a new event is detected.
    /// It delegates to the underlying monitor implementation.
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing a `NetlinkEvent` or an error
    pub async fn recv(&mut self) -> Result<NetlinkEvent> {
        Ok(self.monitor.next_event().await)
    }

    /// Returns whether this socket is using event-driven monitoring
    ///
    /// # Returns
    ///
    /// `true` if using netlink (event-driven), `false` if using polling
    pub fn is_event_driven(&self) -> bool {
        matches!(self.monitor, Monitor::EventDriven(_))
    }
}

/// Detects the current global IPv6 address on the system
///
/// This function queries the system for global IPv6 addresses, preferring
/// stable addresses over temporary ones.
///
/// # Returns
///
/// /// Returns `Some(String)` containing the IPv6 address if found, `None` otherwise
///
/// # Behavior
///
/// - Returns stable IPv6 addresses if available
/// - Falls back to temporary addresses if no stable address exists
/// - Returns `None` if no global IPv6 address is found or an error occurs
#[must_use]
pub fn detect_global_ipv6(allow_loopback: bool) -> Option<Ipv6Addr> {
    match netlink_dump_ipv6() {
        Ok((stable, temporary)) => {
            // Validate routability before handing the address to callers
            stable
                .filter(|ip| is_valid_ipv6(*ip, allow_loopback))
                .or_else(|| temporary.filter(|ip| is_valid_ipv6(*ip, allow_loopback)))
        }
        Err(_) => None,
    }
}

fn nlmsg_align(len: usize) -> usize {
    (len + ALIGN_TO - 1) & !(ALIGN_TO - 1)
}

fn rta_align(len: usize) -> usize {
    (len + ALIGN_TO - 1) & !(ALIGN_TO - 1)
}

/// Parses RTA attributes to extract an IPv6 address
///
/// This helper function iterates through the RTA attributes in a netlink message
/// and returns the first valid IPv6 address found.
///
/// # Arguments
///
/// * `data` - The raw netlink message data
/// * `msg_offset` - Offset to the start of the netlink message
/// * `msg_end` - End offset of the netlink message
///
/// # Returns
///
/// Returns `Some(Ipv6Addr)` if found, `None` otherwise
fn parse_rta_ipv6_address(data: &[u8], msg_offset: usize, msg_end: usize) -> Option<Ipv6Addr> {
    let mut rta_offset = msg_offset + NLMSG_HDRLEN + IFADDRMSG_LEN;
    while rta_offset + RTA_HEADER_SIZE <= msg_end {
        let rta_len = u16::from_ne_bytes([data[rta_offset], data[rta_offset + 1]]) as usize;
        if rta_len < RTA_HEADER_SIZE {
            break;
        }
        let rta_type = u16::from_ne_bytes([data[rta_offset + 2], data[rta_offset + 3]]);

        let payload_len = rta_len - RTA_HEADER_SIZE;
        let payload_offset = rta_offset + RTA_HEADER_SIZE;
        if payload_offset + payload_len > msg_end {
            break;
        }

        // Check for IFA_ADDRESS or IFA_LOCAL attribute with correct payload size
        if (rta_type == IFA_ADDRESS_VAL || rta_type == IFA_LOCAL_VAL)
            && payload_len == IPV6_ADDR_BYTES
        {
            let addr: [u8; IPV6_ADDR_BYTES] =
                match data[payload_offset..payload_offset + IPV6_ADDR_BYTES].try_into() {
                    Ok(a) => a,
                    Err(_) => return None,
                };
            // Keep the address in its native 16-byte form; converting to text
            // is deferred until something actually needs a string.
            return Some(Ipv6Addr::from(addr));
        }

        rta_offset += rta_align(rta_len);
    }

    None
}

/// Extracts an IPv6 address from a netlink interface address message
///
/// This helper function parses the netlink message to extract IPv6 addresses,
/// filtering out temporary, tentative, deprecated, and DAD-failed addresses.
///
/// # Arguments
///
/// * `data` - The raw netlink message data
/// * `msg_offset` - Offset to the start of the netlink message
/// * `nlmsg_len` - Length of the netlink message
/// * `nlmsg_type` - Type of the netlink message (RTM_NEWADDR or RTM_DELADDR)
///
/// # Returns
///
/// Returns `Some(NetlinkEvent)` if a valid IPv6 address is found, `None` otherwise
fn extract_ipv6_from_ifaddrmsg(
    data: &[u8],
    msg_offset: usize,
    nlmsg_len: usize,
    nlmsg_type: u16,
) -> Option<NetlinkEvent> {
    let msg_end = (msg_offset + nlmsg_len).min(data.len());
    if msg_end < msg_offset + NLMSG_HDRLEN + IFADDRMSG_LEN {
        return None;
    }

    let ifa_offset = msg_offset + NLMSG_HDRLEN;
    let ifa_family = data[ifa_offset];
    let ifa_flags = data[ifa_offset + 2];
    let ifa_scope = data[ifa_offset + 3];

    // Filter: must be IPv6, global scope, and not tentative/deprecated/DAD-failed
    if ifa_family != AF_INET6 {
        return None;
    }
    if ifa_scope != RT_SCOPE_UNIVERSE {
        return None;
    }
    if (ifa_flags as u32) & IFA_F_TEMPORARY != 0 {
        return None;
    }
    if (ifa_flags as u32) & IFA_F_TENTATIVE != 0 {
        return None;
    }
    if (ifa_flags as u32) & IFA_F_DADFAILED != 0 {
        return None;
    }
    if (ifa_flags as u32) & IFA_F_DEPRECATED != 0 {
        return None;
    }

    // Parse RTA attributes to find the IPv6 address
    if let Some(ip) = parse_rta_ipv6_address(data, msg_offset, msg_end) {
        let event = match nlmsg_type {
            RTM_NEWADDR_VAL => NetlinkEvent::Ipv6Added(ip),
            RTM_DELADDR_VAL => NetlinkEvent::Ipv6Removed,
            _ => NetlinkEvent::Unknown,
        };
        return Some(event);
    }

    None
}

/// Extracts IPv6 addresses from a netlink interface address message for dump operations
///
/// This helper function is similar to `extract_ipv6_from_ifaddrmsg` but returns both
/// stable and temporary addresses separately, which is needed for dump operations.
///
/// # Arguments
///
/// * `data` - The raw netlink message data
/// * `msg_offset` - Offset to the start of the netlink message
/// * `nlmsg_len` - Length of the netlink message
///
/// # Returns
///
/// Returns `Some((stable, temporary))` where each is an `Option<Ipv6Addr>`,
/// or `None` if no valid address is found
fn extract_ipv6_addresses_for_dump(
    data: &[u8],
    msg_offset: usize,
    nlmsg_len: usize,
) -> Option<(Option<Ipv6Addr>, Option<Ipv6Addr>)> {
    let msg_end = (msg_offset + nlmsg_len).min(data.len());
    if msg_end < msg_offset + NLMSG_HDRLEN + IFADDRMSG_LEN {
        return None;
    }

    let ifa_offset = msg_offset + NLMSG_HDRLEN;
    let ifa_family = data[ifa_offset];
    let ifa_flags = data[ifa_offset + 2];
    let ifa_scope = data[ifa_offset + 3];

    // Filter: must be IPv6, global scope, and not tentative/deprecated/DAD-failed
    // Note: Temporary addresses are NOT filtered out here (unlike in extract_ipv6_from_ifaddrmsg)
    if ifa_family != AF_INET6 {
        return None;
    }
    if ifa_scope != RT_SCOPE_UNIVERSE {
        return None;
    }
    if (ifa_flags as u32) & IFA_F_TENTATIVE != 0 {
        return None;
    }
    if (ifa_flags as u32) & IFA_F_DADFAILED != 0 {
        return None;
    }
    if (ifa_flags as u32) & IFA_F_DEPRECATED != 0 {
        return None;
    }

    let is_temp = (ifa_flags as u32 & IFA_F_TEMPORARY) != 0;

    // Parse RTA attributes to find the IPv6 address
    if let Some(ip) = parse_rta_ipv6_address(data, msg_offset, msg_end) {
        if is_temp {
            return Some((None, Some(ip)));
        } else {
            return Some((Some(ip), None));
        }
    }

    None
}

fn netlink_dump_ipv6() -> Result<(Option<Ipv6Addr>, Option<Ipv6Addr>)> {
    let socket = NetlinkFd::new()?;

    let mut addr: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
    addr.nl_family = NETLINK_ROUTE as libc::sa_family_t;
    addr.nl_groups = 0;
    addr.nl_pid = 0;

    let res = unsafe {
        libc::bind(
            socket.as_raw_fd(),
            &addr as *const _ as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t,
        )
    };
    if res < 0 {
        return Err(std::io::Error::last_os_error()).context("netlink bind");
    }

    let seq = 1u32;
    let mut buf = [0u8; NLMSG_HDRLEN + IFADDRMSG_LEN];
    let nlmsg_len = (NLMSG_HDRLEN + IFADDRMSG_LEN) as u32;
    buf[0..4].copy_from_slice(&nlmsg_len.to_ne_bytes());
    buf[4..6].copy_from_slice(&RTM_GETADDR_VAL.to_ne_bytes());
    buf[6..8].copy_from_slice(&(NLM_F_REQUEST | NLM_F_DUMP).to_ne_bytes());
    buf[8..12].copy_from_slice(&seq.to_ne_bytes());
    buf[12..16].copy_from_slice(&0u32.to_ne_bytes());
    buf[16] = AF_INET6;

    let send_res = unsafe {
        libc::send(
            socket.as_raw_fd(),
            buf.as_ptr() as *const libc::c_void,
            buf.len(),
            0,
        )
    };
    if send_res < 0 {
        return Err(std::io::Error::last_os_error()).context("netlink send");
    }

    let mut stable: Option<Ipv6Addr> = None;
    let mut temporary: Option<Ipv6Addr> = None;
    let mut recv_buf = vec![0u8; NETLINK_DUMP_BUFFER_SIZE];

    loop {
        let n = unsafe {
            libc::recv(
                socket.as_raw_fd(),
                recv_buf.as_mut_ptr() as *mut libc::c_void,
                recv_buf.len(),
                0,
            )
        };
        if n < 0 {
            return Err(std::io::Error::last_os_error()).context("netlink recv");
        }
        if n == 0 {
            break;
        }

        let data = &recv_buf[..n as usize];
        let mut msg_offset = 0usize;
        while msg_offset + NLMSG_HDRLEN <= data.len() {
            // Safely extract nlmsg_len with bounds checking
            let nlmsg_len_bytes = data.get(msg_offset..msg_offset + 4);
            let nlmsg_len = match nlmsg_len_bytes {
                Some(bytes) => {
                    u32::from_ne_bytes(bytes.try_into().expect("slice is exactly 4 bytes")) as usize
                }
                None => break,
            };
            if nlmsg_len < NLMSG_HDRLEN || nlmsg_len == 0 {
                break;
            }

            // Safely extract nlmsg_type with bounds checking
            let nlmsg_type_bytes = data.get(msg_offset + 4..msg_offset + 6);
            let nlmsg_type = match nlmsg_type_bytes {
                Some(bytes) => {
                    u16::from_ne_bytes(bytes.try_into().expect("slice is exactly 2 bytes"))
                }
                None => break,
            };
            if nlmsg_type == NLMSG_DONE {
                return Ok((stable, temporary));
            }
            if nlmsg_type == NLMSG_ERROR {
                return Err(anyhow::anyhow!("netlink error response"));
            }

            if nlmsg_type == RTM_NEWADDR_VAL {
                // Use the helper function to extract IPv6 addresses
                if let Some((addr_stable, addr_temp)) =
                    extract_ipv6_addresses_for_dump(data, msg_offset, nlmsg_len)
                {
                    if let Some(ip) = addr_stable {
                        if stable.is_none() {
                            stable = Some(ip);
                        }
                    }
                    if let Some(ip) = addr_temp {
                        if temporary.is_none() {
                            temporary = Some(ip);
                        }
                    }
                }
            }

            msg_offset += nlmsg_align(nlmsg_len);
        }
    }

    Ok((stable, temporary))
}

//==============================================================================
// Tests
//==============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds an `Ipv6Added` event from a textual address (test helper).
    fn ipv6_added(s: &str) -> NetlinkEvent {
        NetlinkEvent::Ipv6Added(s.parse().unwrap())
    }

    #[test]
    fn test_nlmsg_align() {
        assert_eq!(nlmsg_align(0), 0);
        assert_eq!(nlmsg_align(1), 4);
        assert_eq!(nlmsg_align(4), 4);
        assert_eq!(nlmsg_align(5), 8);
        assert_eq!(nlmsg_align(16), 16);
        assert_eq!(nlmsg_align(17), 20);
        assert_eq!(nlmsg_align(19), 20);
    }

    #[test]
    fn test_rta_align() {
        assert_eq!(rta_align(0), 0);
        assert_eq!(rta_align(1), 4);
        assert_eq!(rta_align(4), 4);
        assert_eq!(rta_align(5), 8);
        assert_eq!(nlmsg_align(16), 16);
    }

    #[test]
    fn test_parse_message_valid_rtm_newaddr() {
        let mut buf = vec![0u8; 64];

        // Netlink header
        let nlmsg_len = 44u32;
        buf[0..4].copy_from_slice(&nlmsg_len.to_ne_bytes());
        buf[4..6].copy_from_slice(&RTM_NEWADDR_VAL.to_ne_bytes());
        buf[6..8].copy_from_slice(&0u16.to_ne_bytes()); // flags
        buf[8..12].copy_from_slice(&1u32.to_ne_bytes()); // seq
        buf[12..16].copy_from_slice(&0u32.to_ne_bytes()); // pid

        // Ifaddrmsg
        let ifa_offset = 16;
        buf[ifa_offset] = AF_INET6; // family
        buf[ifa_offset + 1] = 64; // prefixlen
        buf[ifa_offset + 2] = 0; // flags (1 byte)
        buf[ifa_offset + 3] = RT_SCOPE_UNIVERSE; // scope
        buf[ifa_offset + 4..ifa_offset + 8].copy_from_slice(&0u32.to_ne_bytes()); // ifa_index

        // RTA header for IFA_ADDRESS
        let rta_offset = ifa_offset + 8;
        let rta_len = 20u16;
        buf[rta_offset..rta_offset + 2].copy_from_slice(&rta_len.to_ne_bytes());
        buf[rta_offset + 2..rta_offset + 4].copy_from_slice(&IFA_ADDRESS_VAL.to_ne_bytes());

        // IPv6 address
        let ip_bytes = [0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
        buf[rta_offset + 4..rta_offset + 20].copy_from_slice(&ip_bytes);
        let event = NetlinkImpl::parse_message(&buf);

        assert_eq!(event, Some(ipv6_added("2001:db8::1")));
    }

    #[test]
    fn test_parse_message_rtm_deladdr() {
        let mut buf = vec![0u8; 64];

        // Netlink header
        let nlmsg_len = 44u32;
        buf[0..4].copy_from_slice(&nlmsg_len.to_ne_bytes());
        buf[4..6].copy_from_slice(&RTM_DELADDR_VAL.to_ne_bytes());
        buf[6..8].copy_from_slice(&0u16.to_ne_bytes());
        buf[8..12].copy_from_slice(&1u32.to_ne_bytes());
        buf[12..16].copy_from_slice(&0u32.to_ne_bytes());

        // Ifaddrmsg
        let ifa_offset = 16;
        buf[ifa_offset] = AF_INET6;
        buf[ifa_offset + 1] = 64;
        buf[ifa_offset + 2] = 0;
        buf[ifa_offset + 3] = RT_SCOPE_UNIVERSE;
        buf[ifa_offset + 4..ifa_offset + 8].copy_from_slice(&0u32.to_ne_bytes()); // ifa_index

        // RTA header for IFA_ADDRESS
        let rta_offset = ifa_offset + 8;
        let rta_len = 20u16;
        buf[rta_offset..rta_offset + 2].copy_from_slice(&rta_len.to_ne_bytes());
        buf[rta_offset + 2..rta_offset + 4].copy_from_slice(&IFA_ADDRESS_VAL.to_ne_bytes());

        // IPv6 address
        let ip_bytes = [0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
        buf[rta_offset + 4..rta_offset + 20].copy_from_slice(&ip_bytes);
        let event = NetlinkImpl::parse_message(&buf);

        assert_eq!(event, Some(NetlinkEvent::Ipv6Removed));
    }

    #[test]
    fn test_parse_message_nlmsg_done() {
        let mut buf = vec![0u8; 16];

        let nlmsg_len = 16u32;
        buf[0..4].copy_from_slice(&nlmsg_len.to_ne_bytes());
        buf[4..6].copy_from_slice(&NLMSG_DONE.to_ne_bytes());
        buf[6..8].copy_from_slice(&0u16.to_ne_bytes());
        buf[8..12].copy_from_slice(&1u32.to_ne_bytes());
        buf[12..16].copy_from_slice(&0u32.to_ne_bytes());
        let event = NetlinkImpl::parse_message(&buf);

        assert_eq!(event, None);
    }

    #[test]
    fn test_parse_message_nlmsg_error() {
        let mut buf = vec![0u8; 20];

        let nlmsg_len = 20u32;
        buf[0..4].copy_from_slice(&nlmsg_len.to_ne_bytes());
        buf[4..6].copy_from_slice(&NLMSG_ERROR.to_ne_bytes());
        buf[6..8].copy_from_slice(&0u16.to_ne_bytes());
        buf[8..12].copy_from_slice(&1u32.to_ne_bytes());
        buf[12..16].copy_from_slice(&0u32.to_ne_bytes());
        buf[16..20].copy_from_slice(&0xFFFFFFFFu32.to_ne_bytes()); // error code
        let event = NetlinkImpl::parse_message(&buf);

        assert_eq!(event, None);
    }

    #[test]
    fn test_parse_message_truncated_header() {
        let buf = vec![0u8; 10]; // Less than NLMSG_HDRLEN
        let event = NetlinkImpl::parse_message(&buf);

        assert_eq!(event, None);
    }

    #[test]
    fn test_parse_message_invalid_nlmsg_len() {
        let mut buf = vec![0u8; 16];

        // Invalid nlmsg_len (less than header)
        buf[0..4].copy_from_slice(&8u32.to_ne_bytes());
        let event = NetlinkImpl::parse_message(&buf);

        assert_eq!(event, None);
    }

    #[test]
    fn test_parse_message_zero_nlmsg_len() {
        let mut buf = vec![0u8; 16];

        buf[0..4].copy_from_slice(&0u32.to_ne_bytes());
        let event = NetlinkImpl::parse_message(&buf);

        assert_eq!(event, None);
    }

    #[test]
    fn test_parse_message_non_ipv6_family() {
        let mut buf = vec![0u8; 64];

        let nlmsg_len = 40u32;
        buf[0..4].copy_from_slice(&nlmsg_len.to_ne_bytes());
        buf[4..6].copy_from_slice(&RTM_NEWADDR_VAL.to_ne_bytes());
        buf[6..8].copy_from_slice(&0u16.to_ne_bytes());
        buf[8..12].copy_from_slice(&1u32.to_ne_bytes());
        buf[12..16].copy_from_slice(&0u32.to_ne_bytes());

        let ifa_offset = 16;
        buf[ifa_offset] = libc::AF_INET as u8; // IPv4, not IPv6
        buf[ifa_offset + 1] = 32;
        buf[ifa_offset + 2] = 0;
        buf[ifa_offset + 3] = RT_SCOPE_UNIVERSE;
        let event = NetlinkImpl::parse_message(&buf);

        assert_eq!(event, None);
    }

    #[test]
    fn test_parse_message_non_universe_scope() {
        let mut buf = vec![0u8; 64];

        let nlmsg_len = 40u32;
        buf[0..4].copy_from_slice(&nlmsg_len.to_ne_bytes());
        buf[4..6].copy_from_slice(&RTM_NEWADDR_VAL.to_ne_bytes());
        buf[6..8].copy_from_slice(&0u16.to_ne_bytes());
        buf[8..12].copy_from_slice(&1u32.to_ne_bytes());
        buf[12..16].copy_from_slice(&0u32.to_ne_bytes());

        let ifa_offset = 16;
        buf[ifa_offset] = AF_INET6;
        buf[ifa_offset + 1] = 64;
        buf[ifa_offset + 2] = 0;
        buf[ifa_offset + 3] = libc::RT_SCOPE_LINK; // Link scope, not universe
        let event = NetlinkImpl::parse_message(&buf);

        assert_eq!(event, None);
    }

    #[test]
    fn test_parse_message_temporary_address() {
        let mut buf = vec![0u8; 64];

        let nlmsg_len = 44u32;
        buf[0..4].copy_from_slice(&nlmsg_len.to_ne_bytes());
        buf[4..6].copy_from_slice(&RTM_NEWADDR_VAL.to_ne_bytes());
        buf[6..8].copy_from_slice(&0u16.to_ne_bytes());
        buf[8..12].copy_from_slice(&1u32.to_ne_bytes());
        buf[12..16].copy_from_slice(&0u32.to_ne_bytes());

        let ifa_offset = 16;
        buf[ifa_offset] = AF_INET6;
        buf[ifa_offset + 1] = 64;
        buf[ifa_offset + 2] = IFA_F_TEMPORARY as u8;
        buf[ifa_offset + 3] = RT_SCOPE_UNIVERSE;
        buf[ifa_offset + 4..ifa_offset + 8].copy_from_slice(&0u32.to_ne_bytes()); // ifa_index

        // RTA header for IFA_ADDRESS
        let rta_offset = ifa_offset + 8;
        let rta_len = 20u16;
        buf[rta_offset..rta_offset + 2].copy_from_slice(&rta_len.to_ne_bytes());
        buf[rta_offset + 2..rta_offset + 4].copy_from_slice(&IFA_ADDRESS_VAL.to_ne_bytes());

        let ip_bytes = [0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
        buf[rta_offset + 4..rta_offset + 20].copy_from_slice(&ip_bytes);
        let event = NetlinkImpl::parse_message(&buf);

        assert_eq!(event, None);
    }

    #[test]
    fn test_parse_message_tentative_address() {
        let mut buf = vec![0u8; 64];

        let nlmsg_len = 44u32;
        buf[0..4].copy_from_slice(&nlmsg_len.to_ne_bytes());
        buf[4..6].copy_from_slice(&RTM_NEWADDR_VAL.to_ne_bytes());
        buf[6..8].copy_from_slice(&0u16.to_ne_bytes());
        buf[8..12].copy_from_slice(&1u32.to_ne_bytes());
        buf[12..16].copy_from_slice(&0u32.to_ne_bytes());

        let ifa_offset = 16;
        buf[ifa_offset] = AF_INET6;
        buf[ifa_offset + 1] = 64;
        buf[ifa_offset + 2] = IFA_F_TENTATIVE as u8;
        buf[ifa_offset + 3] = RT_SCOPE_UNIVERSE;
        buf[ifa_offset + 4..ifa_offset + 8].copy_from_slice(&0u32.to_ne_bytes()); // ifa_index

        // RTA header for IFA_ADDRESS
        let rta_offset = ifa_offset + 8;
        let rta_len = 20u16;
        buf[rta_offset..rta_offset + 2].copy_from_slice(&rta_len.to_ne_bytes());
        buf[rta_offset + 2..rta_offset + 4].copy_from_slice(&IFA_ADDRESS_VAL.to_ne_bytes());

        let ip_bytes = [0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
        buf[rta_offset + 4..rta_offset + 20].copy_from_slice(&ip_bytes);
        let event = NetlinkImpl::parse_message(&buf);

        assert_eq!(event, None);
    }

    #[test]
    fn test_parse_message_deprecated_address() {
        let mut buf = vec![0u8; 64];

        let nlmsg_len = 44u32;
        buf[0..4].copy_from_slice(&nlmsg_len.to_ne_bytes());
        buf[4..6].copy_from_slice(&RTM_NEWADDR_VAL.to_ne_bytes());
        buf[6..8].copy_from_slice(&0u16.to_ne_bytes());
        buf[8..12].copy_from_slice(&1u32.to_ne_bytes());
        buf[12..16].copy_from_slice(&0u32.to_ne_bytes());

        let ifa_offset = 16;
        buf[ifa_offset] = AF_INET6;
        buf[ifa_offset + 1] = 64;
        buf[ifa_offset + 2] = IFA_F_DEPRECATED as u8;
        buf[ifa_offset + 3] = RT_SCOPE_UNIVERSE;
        buf[ifa_offset + 4..ifa_offset + 8].copy_from_slice(&0u32.to_ne_bytes()); // ifa_index

        // RTA header for IFA_ADDRESS
        let rta_offset = ifa_offset + 8;
        let rta_len = 20u16;
        buf[rta_offset..rta_offset + 2].copy_from_slice(&rta_len.to_ne_bytes());
        buf[rta_offset + 2..rta_offset + 4].copy_from_slice(&IFA_ADDRESS_VAL.to_ne_bytes());

        let ip_bytes = [0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
        buf[rta_offset + 4..rta_offset + 20].copy_from_slice(&ip_bytes);
        let event = NetlinkImpl::parse_message(&buf);

        assert_eq!(event, None);
    }

    #[test]
    fn test_parse_message_dadfailed_address() {
        let mut buf = vec![0u8; 64];

        let nlmsg_len = 44u32;
        buf[0..4].copy_from_slice(&nlmsg_len.to_ne_bytes());
        buf[4..6].copy_from_slice(&RTM_NEWADDR_VAL.to_ne_bytes());
        buf[6..8].copy_from_slice(&0u16.to_ne_bytes());
        buf[8..12].copy_from_slice(&1u32.to_ne_bytes());
        buf[12..16].copy_from_slice(&0u32.to_ne_bytes());

        let ifa_offset = 16;
        buf[ifa_offset] = AF_INET6;
        buf[ifa_offset + 1] = 64;
        buf[ifa_offset + 2] = IFA_F_DADFAILED as u8;
        buf[ifa_offset + 3] = RT_SCOPE_UNIVERSE;
        buf[ifa_offset + 4..ifa_offset + 8].copy_from_slice(&0u32.to_ne_bytes()); // ifa_index

        // RTA header for IFA_ADDRESS
        let rta_offset = ifa_offset + 8;
        let rta_len = 20u16;
        buf[rta_offset..rta_offset + 2].copy_from_slice(&rta_len.to_ne_bytes());
        buf[rta_offset + 2..rta_offset + 4].copy_from_slice(&IFA_ADDRESS_VAL.to_ne_bytes());

        let ip_bytes = [0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
        buf[rta_offset + 4..rta_offset + 20].copy_from_slice(&ip_bytes);
        let event = NetlinkImpl::parse_message(&buf);

        assert_eq!(event, None);
    }

    #[test]
    fn test_parse_message_multiple_messages() {
        let mut buf = vec![0u8; 128];

        // First message: RTM_NEWADDR
        let offset1 = 0;
        let nlmsg_len1 = 44u32;
        buf[offset1..offset1 + 4].copy_from_slice(&nlmsg_len1.to_ne_bytes());
        buf[offset1 + 4..offset1 + 6].copy_from_slice(&RTM_NEWADDR_VAL.to_ne_bytes());
        buf[offset1 + 6..offset1 + 8].copy_from_slice(&0u16.to_ne_bytes());
        buf[offset1 + 8..offset1 + 12].copy_from_slice(&1u32.to_ne_bytes());
        buf[offset1 + 12..offset1 + 16].copy_from_slice(&0u32.to_ne_bytes());

        let ifa_offset1 = offset1 + 16;
        buf[ifa_offset1] = AF_INET6;
        buf[ifa_offset1 + 1] = 64;
        buf[ifa_offset1 + 2] = 0;
        buf[ifa_offset1 + 3] = RT_SCOPE_UNIVERSE;
        buf[ifa_offset1 + 4..ifa_offset1 + 8].copy_from_slice(&0u32.to_ne_bytes()); // ifa_index

        let rta_offset1 = ifa_offset1 + 8;
        let rta_len1 = 20u16;
        buf[rta_offset1..rta_offset1 + 2].copy_from_slice(&rta_len1.to_ne_bytes());
        buf[rta_offset1 + 2..rta_offset1 + 4].copy_from_slice(&IFA_ADDRESS_VAL.to_ne_bytes());
        let ip_bytes1 = [0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
        buf[rta_offset1 + 4..rta_offset1 + 20].copy_from_slice(&ip_bytes1);

        // Second message: RTM_NEWADDR (different IP)
        let offset2 = 44;
        let nlmsg_len2 = 44u32;
        buf[offset2..offset2 + 4].copy_from_slice(&nlmsg_len2.to_ne_bytes());
        buf[offset2 + 4..offset2 + 6].copy_from_slice(&RTM_NEWADDR_VAL.to_ne_bytes());
        buf[offset2 + 6..offset2 + 8].copy_from_slice(&0u16.to_ne_bytes());
        buf[offset2 + 8..offset2 + 12].copy_from_slice(&2u32.to_ne_bytes());
        buf[offset2 + 12..offset2 + 16].copy_from_slice(&0u32.to_ne_bytes());

        let ifa_offset2 = offset2 + 16;
        buf[ifa_offset2] = AF_INET6;
        buf[ifa_offset2 + 1] = 64;
        buf[ifa_offset2 + 2] = 0;
        buf[ifa_offset2 + 3] = RT_SCOPE_UNIVERSE;
        buf[ifa_offset2 + 4..ifa_offset2 + 8].copy_from_slice(&0u32.to_ne_bytes()); // ifa_index

        let rta_offset2 = ifa_offset2 + 8;
        let rta_len2 = 20u16;
        buf[rta_offset2..rta_offset2 + 2].copy_from_slice(&rta_len2.to_ne_bytes());
        buf[rta_offset2 + 2..rta_offset2 + 4].copy_from_slice(&IFA_ADDRESS_VAL.to_ne_bytes());
        let ip_bytes2 = [0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2];
        buf[rta_offset2 + 4..rta_offset2 + 20].copy_from_slice(&ip_bytes2);
        let event = NetlinkImpl::parse_message(&buf);

        // Should return the first valid event
        assert_eq!(event, Some(ipv6_added("2001:db8::1")));
    }

    #[test]
    fn test_parse_messages_multiple_messages_keeps_order() {
        let mut buf = vec![0u8; 128];

        // First message: RTM_NEWADDR
        let offset1 = 0;
        let nlmsg_len1 = 44u32;
        buf[offset1..offset1 + 4].copy_from_slice(&nlmsg_len1.to_ne_bytes());
        buf[offset1 + 4..offset1 + 6].copy_from_slice(&RTM_NEWADDR_VAL.to_ne_bytes());
        buf[offset1 + 6..offset1 + 8].copy_from_slice(&0u16.to_ne_bytes());
        buf[offset1 + 8..offset1 + 12].copy_from_slice(&1u32.to_ne_bytes());
        buf[offset1 + 12..offset1 + 16].copy_from_slice(&0u32.to_ne_bytes());

        let ifa_offset1 = offset1 + 16;
        buf[ifa_offset1] = AF_INET6;
        buf[ifa_offset1 + 1] = 64;
        buf[ifa_offset1 + 2] = 0;
        buf[ifa_offset1 + 3] = RT_SCOPE_UNIVERSE;
        buf[ifa_offset1 + 4..ifa_offset1 + 8].copy_from_slice(&0u32.to_ne_bytes());

        let rta_offset1 = ifa_offset1 + 8;
        let rta_len1 = 20u16;
        buf[rta_offset1..rta_offset1 + 2].copy_from_slice(&rta_len1.to_ne_bytes());
        buf[rta_offset1 + 2..rta_offset1 + 4].copy_from_slice(&IFA_ADDRESS_VAL.to_ne_bytes());
        let ip_bytes1 = [0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
        buf[rta_offset1 + 4..rta_offset1 + 20].copy_from_slice(&ip_bytes1);

        // Second message: RTM_NEWADDR (different IP)
        let offset2 = 44;
        let nlmsg_len2 = 44u32;
        buf[offset2..offset2 + 4].copy_from_slice(&nlmsg_len2.to_ne_bytes());
        buf[offset2 + 4..offset2 + 6].copy_from_slice(&RTM_NEWADDR_VAL.to_ne_bytes());
        buf[offset2 + 6..offset2 + 8].copy_from_slice(&0u16.to_ne_bytes());
        buf[offset2 + 8..offset2 + 12].copy_from_slice(&2u32.to_ne_bytes());
        buf[offset2 + 12..offset2 + 16].copy_from_slice(&0u32.to_ne_bytes());

        let ifa_offset2 = offset2 + 16;
        buf[ifa_offset2] = AF_INET6;
        buf[ifa_offset2 + 1] = 64;
        buf[ifa_offset2 + 2] = 0;
        buf[ifa_offset2 + 3] = RT_SCOPE_UNIVERSE;
        buf[ifa_offset2 + 4..ifa_offset2 + 8].copy_from_slice(&0u32.to_ne_bytes());

        let rta_offset2 = ifa_offset2 + 8;
        let rta_len2 = 20u16;
        buf[rta_offset2..rta_offset2 + 2].copy_from_slice(&rta_len2.to_ne_bytes());
        buf[rta_offset2 + 2..rta_offset2 + 4].copy_from_slice(&IFA_ADDRESS_VAL.to_ne_bytes());
        let ip_bytes2 = [0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2];
        buf[rta_offset2 + 4..rta_offset2 + 20].copy_from_slice(&ip_bytes2);

        let mut events = VecDeque::new();
        NetlinkImpl::parse_messages_into(&buf, &mut events);
        let ordered: Vec<NetlinkEvent> = events.into_iter().collect();

        assert_eq!(
            ordered,
            vec![ipv6_added("2001:db8::1"), ipv6_added("2001:db8::2"),]
        );
    }

    #[test]
    fn test_parse_message_malformed_rta() {
        let mut buf = vec![0u8; 64];

        let nlmsg_len = 40u32;
        buf[0..4].copy_from_slice(&nlmsg_len.to_ne_bytes());
        buf[4..6].copy_from_slice(&RTM_NEWADDR_VAL.to_ne_bytes());
        buf[6..8].copy_from_slice(&0u16.to_ne_bytes());
        buf[8..12].copy_from_slice(&1u32.to_ne_bytes());
        buf[12..16].copy_from_slice(&0u32.to_ne_bytes());

        let ifa_offset = 16;
        buf[ifa_offset] = AF_INET6;
        buf[ifa_offset + 1] = 64;
        buf[ifa_offset + 2] = 0;
        buf[ifa_offset + 3] = RT_SCOPE_UNIVERSE;

        let rta_offset = ifa_offset + 8;
        // Invalid RTA length (less than header)
        buf[rta_offset..rta_offset + 2].copy_from_slice(&2u16.to_ne_bytes());
        let event = NetlinkImpl::parse_message(&buf);

        assert_eq!(event, None);
    }

    #[test]
    fn test_parse_message_wrong_payload_length() {
        let mut buf = vec![0u8; 64];

        let nlmsg_len = 40u32;
        buf[0..4].copy_from_slice(&nlmsg_len.to_ne_bytes());
        buf[4..6].copy_from_slice(&RTM_NEWADDR_VAL.to_ne_bytes());
        buf[6..8].copy_from_slice(&0u16.to_ne_bytes());
        buf[8..12].copy_from_slice(&1u32.to_ne_bytes());
        buf[12..16].copy_from_slice(&0u32.to_ne_bytes());

        let ifa_offset = 16;
        buf[ifa_offset] = AF_INET6;
        buf[ifa_offset + 1] = 64;
        buf[ifa_offset + 2] = 0;
        buf[ifa_offset + 3] = RT_SCOPE_UNIVERSE;

        let rta_offset = ifa_offset + 8;
        let rta_len = 8u16; // Wrong payload length (not 16 bytes for IPv6)
        buf[rta_offset..rta_offset + 2].copy_from_slice(&rta_len.to_ne_bytes());
        buf[rta_offset + 2..rta_offset + 4].copy_from_slice(&IFA_ADDRESS_VAL.to_ne_bytes());
        let event = NetlinkImpl::parse_message(&buf);

        assert_eq!(event, None);
    }

    #[test]
    fn test_parse_message_uses_ifa_local() {
        let mut buf = vec![0u8; 64];

        let nlmsg_len = 44u32;
        buf[0..4].copy_from_slice(&nlmsg_len.to_ne_bytes());
        buf[4..6].copy_from_slice(&RTM_NEWADDR_VAL.to_ne_bytes());
        buf[6..8].copy_from_slice(&0u16.to_ne_bytes());
        buf[8..12].copy_from_slice(&1u32.to_ne_bytes());
        buf[12..16].copy_from_slice(&0u32.to_ne_bytes());

        let ifa_offset = 16;
        buf[ifa_offset] = AF_INET6;
        buf[ifa_offset + 1] = 64;
        buf[ifa_offset + 2] = 0;
        buf[ifa_offset + 3] = RT_SCOPE_UNIVERSE;
        buf[ifa_offset + 4..ifa_offset + 8].copy_from_slice(&0u32.to_ne_bytes()); // ifa_index

        let rta_offset = ifa_offset + 8;
        let rta_len = 20u16;
        buf[rta_offset..rta_offset + 2].copy_from_slice(&rta_len.to_ne_bytes());
        // Use IFA_LOCAL instead of IFA_ADDRESS
        buf[rta_offset + 2..rta_offset + 4].copy_from_slice(&IFA_LOCAL_VAL.to_ne_bytes());
        let ip_bytes = [0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
        buf[rta_offset + 4..rta_offset + 20].copy_from_slice(&ip_bytes);
        let event = NetlinkImpl::parse_message(&buf);

        assert_eq!(event, Some(ipv6_added("2001:db8::1")));
    }

    /// Manual CPU benchmarks for the per-event hot path.
    ///
    /// Run with:
    /// `cargo test --release --ignored bench_ -- --nocapture`
    ///
    /// These exist to guard the project's core principle that CPU
    /// performance must never regress for the sake of other goals
    /// (e.g. binary size): any codegen-level change must be measured here.
    mod cpu_bench {
        use super::*;
        use std::hint::black_box;
        use std::time::Instant;

        fn synth_batch(n_msgs: usize) -> Vec<u8> {
            let mut buf = Vec::with_capacity(64 * n_msgs);
            for i in 0..n_msgs {
                buf.extend_from_slice(&44u32.to_ne_bytes()); // nlmsg_len
                buf.extend_from_slice(&RTM_NEWADDR_VAL.to_ne_bytes());
                buf.extend_from_slice(&0u16.to_ne_bytes()); // flags
                buf.extend_from_slice(&i.to_ne_bytes()); // seq
                buf.extend_from_slice(&0u32.to_ne_bytes()); // pid
                buf.push(AF_INET6);
                buf.push(64); // prefixlen
                buf.push(0); // flags
                buf.push(RT_SCOPE_UNIVERSE);
                buf.extend_from_slice(&0u32.to_ne_bytes()); // ifa_index
                buf.extend_from_slice(&20u16.to_ne_bytes()); // rta_len
                buf.extend_from_slice(&IFA_ADDRESS_VAL.to_ne_bytes());
                buf.extend_from_slice(&[
                    0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
                ]);
            }
            buf
        }

        #[test]
        #[ignore = "manual benchmark"]
        fn bench_parse_messages_into() {
            // Rotate through several distinct buffers so LLVM cannot prove the
            // input invariant and hoist the whole parse out of the loop.
            let bufs: Vec<Vec<u8>> = (0..8)
                .map(|i| synth_batch(black_box(4 + (i % 3))))
                .collect();
            let n_bufs = black_box(bufs.len() as u32);
            const BATCHES_PER_PASS: u32 = 50_000;
            const PASSES: u32 = 7;
            let mut mins = Vec::new();
            let mut events: VecDeque<NetlinkEvent> = VecDeque::with_capacity(6);
            let mut idx: u32 = 0;
            // Warmup
            for _ in 0..BATCHES_PER_PASS {
                events.clear();
                idx = idx.wrapping_add(1) % black_box(n_bufs);
                NetlinkImpl::parse_messages_into(black_box(&bufs[idx as usize]), &mut events);
                black_box(&events);
            }
            for _pass in 0..PASSES {
                let start = Instant::now();
                for _ in 0..BATCHES_PER_PASS {
                    events.clear();
                    idx = idx.wrapping_add(1) % black_box(n_bufs);
                    NetlinkImpl::parse_messages_into(black_box(&bufs[idx as usize]), &mut events);
                    black_box(&events);
                }
                mins.push(start.elapsed() / BATCHES_PER_PASS);
            }
            mins.sort();
            println!(
                "bench_parse_messages_into: min {:?}/batch median {:?}  [{} passes x {}]",
                mins[0],
                mins[mins.len() / 2],
                PASSES,
                BATCHES_PER_PASS
            );
        }
    }
}
