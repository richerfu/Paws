use super::{PlatformVpnControl, PlatformVpnState, PlatformVpnTelemetry};
use ohos_ashmem_binding::Ashmem;
use serde::{Deserialize, Serialize};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

const REGION_SIZE: usize = 8 * 1024 * 1024;
const REGION_HEADER_SIZE: usize = 4096;
const UI_LANE_SIZE: usize = 1024 * 1024;
const FRAME_HEADER_SIZE: usize = 32;
const FRAME_MAGIC: u32 = 0x5041_5753;
const REGION_MAGIC: &[u8; 8] = b"PAWSIPC\0";
const PROTOCOL_VERSION: u32 = 1;
const SLOT_COUNT: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlatformRole {
    Ui,
    Vpn,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct PlatformEnvelope {
    pub(crate) state: Option<PlatformVpnState>,
    pub(crate) control: Option<PlatformVpnControl>,
    pub(crate) telemetry: Option<PlatformVpnTelemetry>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum PlatformIpcError {
    #[error("platform shared memory lock is poisoned")]
    LockPoisoned,
    #[error("invalid platform shared memory header")]
    InvalidHeader,
    #[error("platform shared memory frame is too large: {actual} bytes, maximum {maximum}")]
    FrameTooLarge { actual: usize, maximum: usize },
    #[error("serialize platform shared memory frame failed: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("platform shared memory operation failed: {0}")]
    Memory(String),
    #[error("platform change notification failed: {0}")]
    Notification(String),
}

type Result<T> = std::result::Result<T, PlatformIpcError>;

pub(crate) struct PlatformSharedMemoryFds {
    pub(crate) ashmem: RawFd,
    pub(crate) notification: RawFd,
}

pub(crate) struct PlatformIpc {
    memory: Mutex<Ashmem>,
    role: PlatformRole,
    published: Mutex<PlatformEnvelope>,
    next_generation: AtomicU64,
    notification: SocketNotification,
}

struct SocketNotification {
    local: OwnedFd,
    transfer: Option<OwnedFd>,
    subscription: Mutex<()>,
}

impl PlatformIpc {
    pub(crate) fn create_ui() -> Result<(Arc<Self>, PlatformSharedMemoryFds)> {
        let mut ashmem = Ashmem::create("paws-platform-session", REGION_SIZE)
            .map_err(|error| PlatformIpcError::Memory(error.to_string()))?;
        ashmem
            .map_read_write()
            .map_err(|error| PlatformIpcError::Memory(error.to_string()))?;
        let (local, transfer) = create_notification_pair()?;
        let fds = PlatformSharedMemoryFds {
            ashmem: ashmem.as_raw_fd(),
            notification: transfer.as_raw_fd(),
        };
        let ipc = Arc::new(Self {
            memory: Mutex::new(ashmem),
            role: PlatformRole::Ui,
            published: Mutex::new(PlatformEnvelope::default()),
            next_generation: AtomicU64::new(1),
            notification: SocketNotification {
                local,
                transfer: Some(transfer),
                subscription: Mutex::new(()),
            },
        });
        ipc.initialize_region()?;
        Ok((ipc, fds))
    }

    pub(crate) fn attach_vpn_raw(ashmem_fd: RawFd, notification_fd: RawFd) -> Result<Arc<Self>> {
        if ashmem_fd < 0 || notification_fd < 0 {
            return Err(PlatformIpcError::InvalidHeader);
        }
        // The descriptors originate from ArkTS Want parameters and remain
        // owned by that runtime. Keep independent duplicates so a repeated
        // onRequest can safely rebind the same session without double-closing
        // the descriptors managed by ArkTS.
        let ashmem_fd = duplicate_fd(ashmem_fd)?;
        let notification_fd = duplicate_fd(notification_fd)?;
        let mut ashmem = Ashmem::from_owned_fd(ashmem_fd)
            .map_err(|error| PlatformIpcError::Memory(error.to_string()))?;
        if ashmem.size() != REGION_SIZE {
            return Err(PlatformIpcError::InvalidHeader);
        }
        ashmem
            .map_read_write()
            .map_err(|error| PlatformIpcError::Memory(error.to_string()))?;
        configure_nonblocking(notification_fd.as_raw_fd())?;
        let ipc = Arc::new(Self {
            memory: Mutex::new(ashmem),
            role: PlatformRole::Vpn,
            published: Mutex::new(PlatformEnvelope::default()),
            next_generation: AtomicU64::new(1),
            notification: SocketNotification {
                local: notification_fd,
                transfer: None,
                subscription: Mutex::new(()),
            },
        });
        ipc.validate_region()?;
        ipc.seed_next_generation()?;
        Ok(ipc)
    }

    pub(crate) fn ui_fds(&self) -> Result<PlatformSharedMemoryFds> {
        if self.role != PlatformRole::Ui {
            return Err(PlatformIpcError::InvalidHeader);
        }
        let ashmem = self
            .memory
            .lock()
            .map_err(|_| PlatformIpcError::LockPoisoned)?
            .as_raw_fd();
        let notification = self
            .notification
            .transfer
            .as_ref()
            .ok_or(PlatformIpcError::InvalidHeader)?
            .as_raw_fd();
        Ok(PlatformSharedMemoryFds {
            ashmem,
            notification,
        })
    }

    pub(crate) fn publish_state(&self, state: PlatformVpnState) -> Result<()> {
        let envelope = {
            let mut published = self
                .published
                .lock()
                .map_err(|_| PlatformIpcError::LockPoisoned)?;
            published.state = Some(state);
            published.clone()
        };
        self.publish(&envelope)
    }

    pub(crate) fn publish_control(&self, control: PlatformVpnControl) -> Result<()> {
        let envelope = {
            let mut published = self
                .published
                .lock()
                .map_err(|_| PlatformIpcError::LockPoisoned)?;
            published.control = Some(control);
            published.clone()
        };
        self.publish(&envelope)
    }

    pub(crate) fn publish_telemetry(&self, telemetry: PlatformVpnTelemetry) -> Result<()> {
        let envelope = {
            let mut published = self
                .published
                .lock()
                .map_err(|_| PlatformIpcError::LockPoisoned)?;
            published.telemetry = Some(telemetry);
            published.clone()
        };
        self.publish(&envelope)
    }

    pub(crate) fn read_remote(&self) -> Result<Option<PlatformEnvelope>> {
        let remote_role = match self.role {
            PlatformRole::Ui => PlatformRole::Vpn,
            PlatformRole::Vpn => PlatformRole::Ui,
        };
        self.read_lane(remote_role)
    }

    /// Wait for the peer process to publish a new frame.
    ///
    /// The UI process gives this blocking subscription to one dedicated
    /// event pump. All in-process consumers subscribe to a Rust watch channel
    /// instead of racing to drain this socket.
    pub(crate) fn wait_for_change_event(&self) -> Result<()> {
        while !self.notification.wait(None)? {}
        Ok(())
    }

    /// Block until the peer publishes a new frame or the wait is cancelled.
    ///
    /// The wait parks on the session notification socket together with a
    /// process-local cancellation socket, so no polling is involved. Returns
    /// `Ok(false)` when cancelled; the caller decides whether to keep waiting
    /// on the current (possibly replaced) session.
    pub(crate) fn wait_for_change_event_cancellable(&self) -> Result<bool> {
        self.notification.wait_event_cancellable()
    }

    pub(crate) fn is_ui(&self) -> bool {
        self.role == PlatformRole::Ui
    }

    fn publish(&self, envelope: &PlatformEnvelope) -> Result<()> {
        let content = serde_json::to_vec(envelope)?;
        let generation = self.next_generation.fetch_add(1, Ordering::Relaxed);
        self.write_frame(self.role, generation, &content)?;
        self.notification.notify()
    }

    fn initialize_region(&self) -> Result<()> {
        let mut header = [0_u8; 32];
        header[..REGION_MAGIC.len()].copy_from_slice(REGION_MAGIC);
        header[8..12].copy_from_slice(&PROTOCOL_VERSION.to_le_bytes());
        header[12..16].copy_from_slice(&(REGION_SIZE as u32).to_le_bytes());
        header[16..24].copy_from_slice(&session_id().to_le_bytes());
        let header_checksum = checksum(&header[..24]);
        header[24..28].copy_from_slice(&header_checksum.to_le_bytes());
        self.write_memory(0, &header)
    }

    fn validate_region(&self) -> Result<()> {
        let header = self.read_memory(0, 32)?;
        if header[..REGION_MAGIC.len()] != REGION_MAGIC[..]
            || read_u32(&header[8..12]) != PROTOCOL_VERSION
            || read_u32(&header[12..16]) as usize != REGION_SIZE
            || read_u32(&header[24..28]) != checksum(&header[..24])
        {
            return Err(PlatformIpcError::InvalidHeader);
        }
        Ok(())
    }

    fn seed_next_generation(&self) -> Result<()> {
        let latest = self.latest_generation(self.role)?.unwrap_or(0);
        self.next_generation
            .store(latest.saturating_add(1), Ordering::Relaxed);
        Ok(())
    }

    fn write_frame(&self, role: PlatformRole, generation: u64, content: &[u8]) -> Result<()> {
        let (lane_offset, slot_size) = lane_layout(role);
        let maximum = slot_size - FRAME_HEADER_SIZE;
        if content.len() > maximum {
            return Err(PlatformIpcError::FrameTooLarge {
                actual: content.len(),
                maximum,
            });
        }
        let slot_offset = lane_offset + generation as usize % SLOT_COUNT * slot_size;
        self.write_memory(slot_offset + FRAME_HEADER_SIZE, content)?;

        let mut header = [0_u8; FRAME_HEADER_SIZE];
        header[0..4].copy_from_slice(&FRAME_MAGIC.to_le_bytes());
        header[4..8].copy_from_slice(&PROTOCOL_VERSION.to_le_bytes());
        header[8..16].copy_from_slice(&generation.to_le_bytes());
        header[16..20].copy_from_slice(&(content.len() as u32).to_le_bytes());
        header[20..24].copy_from_slice(&checksum(content).to_le_bytes());
        let header_checksum = checksum(&header[..24]);
        header[24..28].copy_from_slice(&header_checksum.to_le_bytes());
        self.write_memory(slot_offset, &header)
    }

    fn read_lane(&self, role: PlatformRole) -> Result<Option<PlatformEnvelope>> {
        let (lane_offset, slot_size) = lane_layout(role);
        let mut latest: Option<(u64, Vec<u8>)> = None;
        for slot_index in 0..SLOT_COUNT {
            let slot_offset = lane_offset + slot_index * slot_size;
            let Some((generation, content)) = self.read_frame(slot_offset, slot_size)? else {
                continue;
            };
            if latest
                .as_ref()
                .is_none_or(|(current, _)| generation > *current)
            {
                latest = Some((generation, content));
            }
        }
        latest
            .map(|(_, content)| serde_json::from_slice(&content).map_err(PlatformIpcError::from))
            .transpose()
    }

    fn latest_generation(&self, role: PlatformRole) -> Result<Option<u64>> {
        let (lane_offset, slot_size) = lane_layout(role);
        let mut latest = None;
        for slot_index in 0..SLOT_COUNT {
            let slot_offset = lane_offset + slot_index * slot_size;
            let header = self.read_memory(slot_offset, FRAME_HEADER_SIZE)?;
            if let Some(frame) = FrameHeader::parse(&header, slot_size) {
                latest =
                    Some(latest.map_or(frame.generation, |value: u64| value.max(frame.generation)));
            }
        }
        Ok(latest)
    }

    fn read_frame(&self, slot_offset: usize, slot_size: usize) -> Result<Option<(u64, Vec<u8>)>> {
        for _ in 0..3 {
            let first = self.read_memory(slot_offset, FRAME_HEADER_SIZE)?;
            let Some(header) = FrameHeader::parse(&first, slot_size) else {
                // The writer commits a frame by replacing its header after the
                // payload. A concurrent reader can briefly observe a torn
                // header, so retry before treating this slot as empty.
                continue;
            };
            let content =
                self.read_memory(slot_offset + FRAME_HEADER_SIZE, header.content_length)?;
            let second = self.read_memory(slot_offset, FRAME_HEADER_SIZE)?;
            if first == second && checksum(&content) == header.content_checksum {
                return Ok(Some((header.generation, content)));
            }
        }
        Ok(None)
    }

    fn read_memory(&self, offset: usize, length: usize) -> Result<Vec<u8>> {
        self.memory
            .lock()
            .map_err(|_| PlatformIpcError::LockPoisoned)?
            .read(offset, length)
            .map_err(|error| PlatformIpcError::Memory(error.to_string()))
    }

    fn write_memory(&self, offset: usize, data: &[u8]) -> Result<()> {
        self.memory
            .lock()
            .map_err(|_| PlatformIpcError::LockPoisoned)?
            .write(offset, data)
            .map_err(|error| PlatformIpcError::Memory(error.to_string()))
    }
}

impl SocketNotification {
    fn notify(&self) -> Result<()> {
        let value = [1_u8];
        loop {
            let written = unsafe {
                libc::send(
                    self.local.as_raw_fd(),
                    value.as_ptr().cast(),
                    value.len(),
                    libc::MSG_NOSIGNAL,
                )
            };
            if written == value.len() as isize {
                return Ok(());
            }
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            if error.kind() == io::ErrorKind::WouldBlock {
                return Ok(());
            }
            return Err(PlatformIpcError::Notification(error.to_string()));
        }
    }

    fn wait(&self, timeout: Option<Duration>) -> Result<bool> {
        let _subscription = self
            .subscription
            .lock()
            .map_err(|_| PlatformIpcError::LockPoisoned)?;
        let timeout_ms = timeout
            .map(|timeout| timeout.as_millis().min(i32::MAX as u128) as i32)
            .unwrap_or(-1);
        let fd = self.local.as_raw_fd();
        let mut descriptor = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        loop {
            let ready = unsafe { libc::poll(&mut descriptor, 1, timeout_ms) };
            if ready == 0 {
                return Ok(false);
            }
            if ready < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(PlatformIpcError::Notification(error.to_string()));
            }
            return drain_notifications(fd);
        }
    }

    /// Block on the session notification socket until a frame arrives or the
    /// process-local cancellation socket is signalled. No timeout, no polling.
    fn wait_event_cancellable(&self) -> Result<bool> {
        let _subscription = self
            .subscription
            .lock()
            .map_err(|_| PlatformIpcError::LockPoisoned)?;
        let session_fd = self.local.as_raw_fd();
        let cancel_fd = cancellation_fd();
        // Drop stale cancellation bytes so a previous stop cannot make the
        // next subscription iteration return immediately in a busy loop.
        drain_cancel_fd(cancel_fd);
        let mut descriptors = [
            libc::pollfd {
                fd: session_fd,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: cancel_fd,
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        loop {
            let ready = unsafe { libc::poll(descriptors.as_mut_ptr(), 2, -1) };
            if ready < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(PlatformIpcError::Notification(error.to_string()));
            }
            if descriptors[1].revents != 0 {
                drain_cancel_fd(cancel_fd);
                return Ok(false);
            }
            if descriptors[0].revents != 0 {
                let changed = drain_notifications(session_fd)?;
                if changed {
                    return Ok(true);
                }
                // Spurious wakeup on the session socket: park again.
            }
        }
    }
}

/// Process-local cancellation socketpair for waking blocked event waits.
struct CancelPair {
    read: OwnedFd,
    write: OwnedFd,
}

fn cancel_pair() -> &'static CancelPair {
    static PAIR: OnceLock<CancelPair> = OnceLock::new();
    PAIR.get_or_init(|| {
        let (read, write) = create_notification_pair().expect("create cancellation socketpair");
        CancelPair { read, write }
    })
}

fn cancellation_fd() -> RawFd {
    cancel_pair().read.as_raw_fd()
}

/// Wake every in-process waiter parked in
/// [`PlatformIpc::wait_for_change_event_cancellable`].
///
/// The wakeup rides a process-local socketpair that is independent from the
/// session notification socket, so cancelling never pollutes or shuts down the
/// cross-process notification fd that later Want rebinds rely on.
pub(crate) fn cancel_event_waits() {
    let write = cancel_pair().write.as_raw_fd();
    let value = [1_u8];
    let written = unsafe {
        libc::send(
            write,
            value.as_ptr().cast(),
            value.len(),
            libc::MSG_NOSIGNAL,
        )
    };
    let _ = written; // Best effort: a pending cancel byte already covers the wakeup.
}

fn drain_cancel_fd(fd: RawFd) {
    let mut buffer = [0_u8; 64];
    loop {
        let read = unsafe { libc::recv(fd, buffer.as_mut_ptr().cast(), buffer.len(), 0) };
        if read > 0 {
            continue;
        }
        if read == 0 {
            return;
        }
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::Interrupted {
            continue;
        }
        return;
    }
}

struct FrameHeader {
    generation: u64,
    content_length: usize,
    content_checksum: u32,
}

impl FrameHeader {
    fn parse(bytes: &[u8], slot_size: usize) -> Option<Self> {
        if bytes.len() != FRAME_HEADER_SIZE
            || read_u32(&bytes[0..4]) != FRAME_MAGIC
            || read_u32(&bytes[4..8]) != PROTOCOL_VERSION
            || read_u32(&bytes[24..28]) != checksum(&bytes[..24])
        {
            return None;
        }
        let content_length = read_u32(&bytes[16..20]) as usize;
        if content_length > slot_size - FRAME_HEADER_SIZE {
            return None;
        }
        Some(Self {
            generation: read_u64(&bytes[8..16]),
            content_length,
            content_checksum: read_u32(&bytes[20..24]),
        })
    }
}

const fn lane_layout(role: PlatformRole) -> (usize, usize) {
    match role {
        PlatformRole::Ui => (REGION_HEADER_SIZE, UI_LANE_SIZE / SLOT_COUNT),
        PlatformRole::Vpn => {
            let offset = REGION_HEADER_SIZE + UI_LANE_SIZE;
            (offset, (REGION_SIZE - offset) / SLOT_COUNT)
        }
    }
}

fn create_notification_pair() -> Result<(OwnedFd, OwnedFd)> {
    let mut fds = [-1; 2];
    let result = unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_DGRAM, 0, fds.as_mut_ptr()) };
    if result < 0 {
        return Err(PlatformIpcError::Notification(
            io::Error::last_os_error().to_string(),
        ));
    }
    let first = unsafe { OwnedFd::from_raw_fd(fds[0]) };
    let second = unsafe { OwnedFd::from_raw_fd(fds[1]) };
    configure_nonblocking(first.as_raw_fd())?;
    configure_nonblocking(second.as_raw_fd())?;
    Ok((first, second))
}

fn configure_nonblocking(fd: RawFd) -> Result<()> {
    let status = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if status < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, status | libc::O_NONBLOCK) } < 0 {
        return Err(PlatformIpcError::Notification(
            io::Error::last_os_error().to_string(),
        ));
    }
    let descriptor = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if descriptor < 0
        || unsafe { libc::fcntl(fd, libc::F_SETFD, descriptor | libc::FD_CLOEXEC) } < 0
    {
        return Err(PlatformIpcError::Notification(
            io::Error::last_os_error().to_string(),
        ));
    }
    Ok(())
}

fn duplicate_fd(fd: RawFd) -> Result<OwnedFd> {
    let duplicate = unsafe { libc::dup(fd) };
    if duplicate < 0 {
        return Err(PlatformIpcError::Notification(
            io::Error::last_os_error().to_string(),
        ));
    }
    Ok(unsafe { OwnedFd::from_raw_fd(duplicate) })
}

fn drain_notifications(fd: RawFd) -> Result<bool> {
    let mut changed = false;
    let mut buffer = [0_u8; 64];
    loop {
        let read = unsafe { libc::recv(fd, buffer.as_mut_ptr().cast(), buffer.len(), 0) };
        if read > 0 {
            changed = true;
            continue;
        }
        if read == 0 {
            return Ok(changed);
        }
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::Interrupted {
            continue;
        }
        if error.kind() == io::ErrorKind::WouldBlock {
            return Ok(changed);
        }
        return Err(PlatformIpcError::Notification(error.to_string()));
    }
}

fn checksum(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c_9dc5, |hash, byte| {
        hash.wrapping_mul(0x0100_0193) ^ u32::from(*byte)
    })
}

fn read_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes.try_into().expect("validated u32 slice"))
}

fn read_u64(bytes: &[u8]) -> u64 {
    u64::from_le_bytes(bytes.try_into().expect("validated u64 slice"))
}

fn session_id() -> u64 {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or_default();
    timestamp ^ u64::from(std::process::id()).rotate_left(32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_header_rejects_corruption() {
        let mut header = [0_u8; FRAME_HEADER_SIZE];
        header[0..4].copy_from_slice(&FRAME_MAGIC.to_le_bytes());
        header[4..8].copy_from_slice(&PROTOCOL_VERSION.to_le_bytes());
        header[8..16].copy_from_slice(&7_u64.to_le_bytes());
        header[16..20].copy_from_slice(&12_u32.to_le_bytes());
        header[20..24].copy_from_slice(&34_u32.to_le_bytes());
        let header_checksum = checksum(&header[..24]);
        header[24..28].copy_from_slice(&header_checksum.to_le_bytes());
        assert_eq!(
            FrameHeader::parse(&header, 1024).map(|frame| frame.generation),
            Some(7)
        );
        header[8] ^= 1;
        assert!(FrameHeader::parse(&header, 1024).is_none());
    }

    #[test]
    fn blocking_event_subscription_wakes_for_peer_publication() {
        let (local, peer) = create_notification_pair().unwrap();
        let subscription = Arc::new(SocketNotification {
            local,
            transfer: None,
            subscription: Mutex::new(()),
        });
        let notifier = SocketNotification {
            local: peer,
            transfer: None,
            subscription: Mutex::new(()),
        };
        let waiter = {
            let subscription = Arc::clone(&subscription);
            std::thread::spawn(move || subscription.wait(None))
        };

        notifier.notify().unwrap();

        assert!(waiter.join().unwrap().unwrap());
    }
}
