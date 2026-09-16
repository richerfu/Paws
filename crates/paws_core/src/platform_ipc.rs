use super::{PlatformVpnControl, PlatformVpnState, PlatformVpnTelemetry};
use ohos_ashmem_binding::Ashmem;
use serde::{Deserialize, Serialize};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const REGION_SIZE: usize = 8 * 1024 * 1024;
const REGION_HEADER_SIZE: usize = 4096;
const UI_LANE_SIZE: usize = 1024 * 1024;
const FRAME_HEADER_SIZE: usize = 32;
const FRAME_MAGIC: u32 = 0x5041_5753;
const REGION_MAGIC: &[u8; 8] = b"PAWSIPC\0";
const PROTOCOL_VERSION: u32 = 2;
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
    listener: Option<OwnedFd>,
    address: NotificationAddress,
    connection: Mutex<Option<Arc<OwnedFd>>>,
    subscription: Mutex<()>,
    cancel_read: OwnedFd,
    cancel_write: OwnedFd,
    cancel_pending: AtomicBool,
}

impl PlatformIpc {
    pub(crate) fn create_ui() -> Result<(Arc<Self>, PlatformSharedMemoryFds)> {
        let mut ashmem = Ashmem::create("paws-platform-session", REGION_SIZE)
            .map_err(|error| PlatformIpcError::Memory(error.to_string()))?;
        ashmem
            .map_read_write()
            .map_err(|error| PlatformIpcError::Memory(error.to_string()))?;
        let notification = SocketNotification::listen(session_id())?;
        let fds = PlatformSharedMemoryFds {
            ashmem: ashmem.as_raw_fd(),
            notification: notification.listener_fd()?,
        };
        let ipc = Arc::new(Self {
            memory: Mutex::new(ashmem),
            role: PlatformRole::Ui,
            published: Mutex::new(PlatformEnvelope::default()),
            next_generation: AtomicU64::new(1),
            notification,
        });
        ipc.initialize_region()?;
        Ok((ipc, fds))
    }

    pub(crate) fn attach_vpn_raw(ashmem_fd: RawFd, notification_fd: RawFd) -> Result<Arc<Self>> {
        if ashmem_fd < 0 || notification_fd < 0 {
            return Err(PlatformIpcError::InvalidHeader);
        }
        // ArkTS owns the Want descriptors. Duplicate ashmem for this binding.
        // The notification descriptor now identifies the UI listener; the VPN
        // must create its own endpoint, using the session address in ashmem,
        // rather than write a transferred UI-domain socket.
        let ashmem_fd = duplicate_fd(ashmem_fd)?;
        let mut ashmem = Ashmem::from_owned_fd(ashmem_fd)
            .map_err(|error| PlatformIpcError::Memory(error.to_string()))?;
        if ashmem.size() != REGION_SIZE {
            return Err(PlatformIpcError::InvalidHeader);
        }
        ashmem
            .map_read_write()
            .map_err(|error| PlatformIpcError::Memory(error.to_string()))?;
        let mut ipc = Self {
            memory: Mutex::new(ashmem),
            role: PlatformRole::Vpn,
            published: Mutex::new(PlatformEnvelope::default()),
            next_generation: AtomicU64::new(1),
            notification: SocketNotification::connect_lazily(0)?,
        };
        ipc.validate_region()?;
        let header = ipc.read_memory(0, 32)?;
        ipc.notification.address = NotificationAddress::new(read_u64(&header[16..24]));
        ipc.seed_next_generation()?;
        Ok(Arc::new(ipc))
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
        let notification = self.notification.listener_fd()?;
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

    pub(crate) fn cancel_event_waits(&self) {
        self.notification.cancel_waits();
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
        header[16..24].copy_from_slice(&self.notification.address.session.to_le_bytes());
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

struct NotificationAddress {
    session: u64,
    raw: libc::sockaddr_un,
    length: libc::socklen_t,
}

impl NotificationAddress {
    fn new(session: u64) -> Self {
        let mut raw: libc::sockaddr_un = unsafe { std::mem::zeroed() };
        raw.sun_family = libc::AF_UNIX as libc::sa_family_t;
        #[cfg(any(target_os = "linux", target_env = "ohos"))]
        let name = format!("\0paws-platform-{session:016x}");
        #[cfg(not(any(target_os = "linux", target_env = "ohos")))]
        let name = format!("/tmp/paws-platform-{session:016x}.sock");
        for (target, byte) in raw.sun_path.iter_mut().zip(name.bytes()) {
            *target = byte as libc::c_char;
        }
        let length = std::mem::offset_of!(libc::sockaddr_un, sun_path) + name.len();
        #[cfg(not(any(target_os = "linux", target_env = "ohos")))]
        let length = length + 1;
        #[cfg(target_os = "macos")]
        {
            raw.sun_len = length as u8;
        }
        Self {
            session,
            raw,
            length: length as libc::socklen_t,
        }
    }

    fn pointer(&self) -> *const libc::sockaddr {
        (&self.raw as *const libc::sockaddr_un).cast()
    }
}

impl SocketNotification {
    fn listen(session: u64) -> Result<Self> {
        let address = NotificationAddress::new(session);
        let listener = Self::create_socket()?;
        if unsafe { libc::bind(listener.as_raw_fd(), address.pointer(), address.length) } < 0
            || unsafe { libc::listen(listener.as_raw_fd(), 16) } < 0
        {
            return Err(notification_error());
        }
        configure_nonblocking(listener.as_raw_fd())?;
        Self::new(Some(listener), address, None)
    }

    fn connect_lazily(session: u64) -> Result<Self> {
        // Want validation only maps ashmem. Connect when this binding actually
        // publishes or subscribes, so a duplicate Want cannot replace a live peer.
        Self::new(None, NotificationAddress::new(session), None)
    }

    fn new(
        listener: Option<OwnedFd>,
        address: NotificationAddress,
        connection: Option<OwnedFd>,
    ) -> Result<Self> {
        let (cancel_read, cancel_write) = create_notification_pair()?;
        Ok(Self {
            listener,
            address,
            connection: Mutex::new(connection.map(Arc::new)),
            subscription: Mutex::new(()),
            cancel_read,
            cancel_write,
            cancel_pending: AtomicBool::new(false),
        })
    }

    fn listener_fd(&self) -> Result<RawFd> {
        self.listener
            .as_ref()
            .map(AsRawFd::as_raw_fd)
            .ok_or(PlatformIpcError::InvalidHeader)
    }

    fn create_socket() -> Result<OwnedFd> {
        let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
        if fd < 0 {
            return Err(notification_error());
        }
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }

    fn connection(&self) -> Result<Option<Arc<OwnedFd>>> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| PlatformIpcError::LockPoisoned)?;
        if let Some(listener) = &self.listener {
            loop {
                let fd = unsafe {
                    libc::accept(
                        listener.as_raw_fd(),
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                    )
                };
                if fd >= 0 {
                    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
                    configure_nonblocking(fd.as_raw_fd())?;
                    *connection = Some(Arc::new(fd));
                    // A publisher may accept while the waiter is parked on
                    // only the listener. Wake it to rebuild its poll set.
                    self.wake_local_waiter();
                    continue;
                }
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                if error.kind() == io::ErrorKind::WouldBlock {
                    break;
                }
                return Err(PlatformIpcError::Notification(error.to_string()));
            }
        } else if connection.is_none() {
            // Each process creates its own endpoint in its own SELinux domain.
            // Passing a UI-created socketpair endpoint through Want grants FD
            // use, but does not grant vpn_isolate_hap write access to that socket.
            let fd = Self::create_socket()?;
            if unsafe { libc::connect(fd.as_raw_fd(), self.address.pointer(), self.address.length) }
                < 0
            {
                return Err(notification_error());
            }
            configure_nonblocking(fd.as_raw_fd())?;
            *connection = Some(Arc::new(fd));
        }
        Ok(connection.clone())
    }

    fn notify(&self) -> Result<()> {
        let Some(connection) = self.connection()? else {
            // The UI can publish before the Extension attaches. Its latest
            // frame is already in ashmem and is read when the peer binds.
            return Ok(());
        };
        let value = [1_u8];
        loop {
            let written = unsafe {
                libc::send(
                    connection.as_raw_fd(),
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
            if self.listener.is_some()
                && matches!(error.raw_os_error(), Some(libc::EPIPE | libc::ECONNRESET))
            {
                self.clear_connection(&connection)?;
                return Ok(());
            }
            return Err(PlatformIpcError::Notification(error.to_string()));
        }
    }

    fn clear_connection(&self, expected: &Arc<OwnedFd>) -> Result<()> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| PlatformIpcError::LockPoisoned)?;
        if connection
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, expected))
        {
            *connection = None;
        }
        Ok(())
    }

    fn wait(&self, timeout: Option<Duration>) -> Result<bool> {
        self.wait_internal(timeout, false)
    }

    fn wait_event_cancellable(&self) -> Result<bool> {
        self.wait_internal(None, true)
    }

    fn wait_internal(&self, timeout: Option<Duration>, cancellable: bool) -> Result<bool> {
        let _subscription = self
            .subscription
            .lock()
            .map_err(|_| PlatformIpcError::LockPoisoned)?;
        let cancel_fd = self.cancel_read.as_raw_fd();
        if cancellable && self.cancel_pending.swap(false, Ordering::AcqRel) {
            drain_cancel_fd(cancel_fd);
            return Ok(false);
        }
        let timeout_ms = timeout
            .map(|timeout| timeout.as_millis().min(i32::MAX as u128) as i32)
            .unwrap_or(-1);
        loop {
            // Hold an Arc while polling: a concurrent publication may accept a
            // replacement session, but cannot close or reuse this descriptor.
            let connection = self.connection()?;
            let mut descriptors = [
                libc::pollfd {
                    fd: connection.as_ref().map_or(-1, |fd| fd.as_raw_fd()),
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: self.listener.as_ref().map_or(-1, AsRawFd::as_raw_fd),
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: cancel_fd,
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            let ready = unsafe { libc::poll(descriptors.as_mut_ptr(), 3, timeout_ms) };
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
            if descriptors[2].revents != 0 {
                drain_cancel_fd(cancel_fd);
                if cancellable && self.cancel_pending.swap(false, Ordering::AcqRel) {
                    return Ok(false);
                }
                continue;
            }
            if descriptors[1].revents != 0 {
                // Attachment itself wakes consumers to read the latest ashmem
                // lane, including a terminal delivery acknowledgement.
                self.connection()?;
                return Ok(true);
            }
            if descriptors[0].revents != 0 {
                if let Some(connection) = connection {
                    let (changed, closed) = drain_notifications(connection.as_raw_fd())?;
                    if closed {
                        self.clear_connection(&connection)?;
                        if self.listener.is_none() {
                            return Err(PlatformIpcError::Notification(
                                "platform notification peer closed".to_owned(),
                            ));
                        }
                        // A dead Extension cannot publish. Wake once, then wait
                        // on the listener for a future owner instead of spinning.
                        return Ok(true);
                    }
                    if changed {
                        return Ok(true);
                    }
                }
            }
        }
    }

    fn cancel_waits(&self) {
        if self.cancel_pending.swap(true, Ordering::AcqRel) {
            return;
        }
        self.wake_local_waiter();
    }

    fn wake_local_waiter(&self) {
        let value = [1_u8];
        unsafe {
            libc::send(
                self.cancel_write.as_raw_fd(),
                value.as_ptr().cast(),
                value.len(),
                libc::MSG_NOSIGNAL,
            );
        }
    }
}

impl Drop for SocketNotification {
    fn drop(&mut self) {
        #[cfg(not(any(target_os = "linux", target_env = "ohos")))]
        if self.listener.is_some() {
            unsafe {
                libc::unlink(self.address.raw.sun_path.as_ptr());
            }
        }
    }
}

fn notification_error() -> PlatformIpcError {
    PlatformIpcError::Notification(io::Error::last_os_error().to_string())
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
    let result = unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM, 0, fds.as_mut_ptr()) };
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

fn drain_notifications(fd: RawFd) -> Result<(bool, bool)> {
    let mut changed = false;
    let mut buffer = [0_u8; 64];
    loop {
        let read = unsafe { libc::recv(fd, buffer.as_mut_ptr().cast(), buffer.len(), 0) };
        if read > 0 {
            changed = true;
            continue;
        }
        if read == 0 {
            return Ok((changed, true));
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ECONNRESET) {
            return Ok((changed, true));
        }
        if error.kind() == io::ErrorKind::Interrupted {
            continue;
        }
        if error.kind() == io::ErrorKind::WouldBlock {
            return Ok((changed, false));
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
        let subscription = Arc::new(
            SocketNotification::new(None, NotificationAddress::new(0), Some(local)).unwrap(),
        );
        let notifier =
            SocketNotification::new(None, NotificationAddress::new(0), Some(peer)).unwrap();
        let waiter = {
            let subscription = Arc::clone(&subscription);
            std::thread::spawn(move || subscription.wait(None))
        };

        notifier.notify().unwrap();

        assert!(waiter.join().unwrap().unwrap());
    }

    #[test]
    fn stream_notifications_coalesce_and_remain_bidirectional() {
        let (local, peer) = create_notification_pair().unwrap();
        let first =
            SocketNotification::new(None, NotificationAddress::new(0), Some(local)).unwrap();
        let second =
            SocketNotification::new(None, NotificationAddress::new(0), Some(peer)).unwrap();

        for _ in 0..100 {
            first.notify().unwrap();
        }
        assert!(second.wait(Some(Duration::ZERO)).unwrap());
        assert!(!second.wait(Some(Duration::ZERO)).unwrap());
        second.notify().unwrap();
        assert!(first.wait(Some(Duration::ZERO)).unwrap());
    }

    #[test]
    fn stream_peer_close_terminates_the_wait() {
        let (local, peer) = create_notification_pair().unwrap();
        let subscription =
            SocketNotification::new(None, NotificationAddress::new(0), Some(local)).unwrap();
        drop(peer);

        assert!(subscription.wait_event_cancellable().is_err());
        assert!(subscription.notify().is_err());
    }

    #[test]
    fn listener_accepts_independently_created_endpoint_and_rebinds() {
        let session = session_id();
        let ui = SocketNotification::listen(session).unwrap();
        ui.notify().unwrap(); // Publishing before attachment must succeed.
        assert!(!ui.wait(Some(Duration::ZERO)).unwrap());
        let vpn = SocketNotification::connect_lazily(session).unwrap();
        vpn.notify().unwrap();
        assert!(ui.wait(Some(Duration::ZERO)).unwrap());
        ui.notify().unwrap();
        assert!(vpn.wait(Some(Duration::ZERO)).unwrap());

        // A read-only Want probe must not connect or displace the live owner.
        let probe = SocketNotification::connect_lazily(session).unwrap();
        assert!(probe.connection.lock().unwrap().is_none());
        drop(probe);
        ui.notify().unwrap();
        assert!(vpn.wait(Some(Duration::ZERO)).unwrap());

        // Closing with an unread UI wake byte may reset the stream. It must
        // still retire the peer and leave the listener ready for a new owner.
        ui.notify().unwrap();
        drop(vpn);
        assert!(ui.wait(Some(Duration::ZERO)).unwrap());
        assert!(!ui.wait(Some(Duration::ZERO)).unwrap());
        let rebound = SocketNotification::connect_lazily(session).unwrap();
        rebound.notify().unwrap();
        assert!(ui.wait(Some(Duration::ZERO)).unwrap());
        ui.notify().unwrap();
        assert!(rebound.wait(Some(Duration::ZERO)).unwrap());
    }

    #[test]
    fn accepting_on_a_publisher_wakes_a_parked_listener_subscription() {
        let session = session_id();
        let ui = Arc::new(SocketNotification::listen(session).unwrap());
        let waiter = {
            let ui = Arc::clone(&ui);
            std::thread::spawn(move || ui.wait_event_cancellable())
        };
        let vpn = SocketNotification::connect_lazily(session).unwrap();
        vpn.connection().unwrap();
        // Force acceptance on the publishing thread. The waiter may already
        // be polling the listener without a connection descriptor.
        ui.notify().unwrap();
        vpn.notify().unwrap();
        assert!(waiter.join().unwrap().unwrap());
    }

    #[test]
    fn cancellable_wait_keeps_a_pre_registration_wakeup() {
        let (local, _peer) = create_notification_pair().unwrap();
        let subscription =
            SocketNotification::new(None, NotificationAddress::new(0), Some(local)).unwrap();

        subscription.cancel_waits();

        assert!(!subscription.wait_event_cancellable().unwrap());
    }

    #[test]
    fn cancellation_is_scoped_to_one_ipc_binding() {
        let (first_local, _first_peer) = create_notification_pair().unwrap();
        let first =
            SocketNotification::new(None, NotificationAddress::new(0), Some(first_local)).unwrap();
        let (second_local, second_peer) = create_notification_pair().unwrap();
        let second =
            SocketNotification::new(None, NotificationAddress::new(0), Some(second_local)).unwrap();
        let notifier =
            SocketNotification::new(None, NotificationAddress::new(0), Some(second_peer)).unwrap();

        first.cancel_waits();
        notifier.notify().unwrap();

        assert!(!first.wait_event_cancellable().unwrap());
        assert!(second.wait_event_cancellable().unwrap());
    }
}
