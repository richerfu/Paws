use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const HANDOFF_MAGIC: u32 = 0x484F_5646;
const HANDOFF_HEADER_SIZE: usize = 12;
const HANDOFF_MAX_PAYLOAD: usize = 1024 * 1024;
const HANDOFF_POLL: Duration = Duration::from_secs(1);
const HANDOFF_MAX_WAIT: Duration = Duration::from_secs(600);
const HANDOFF_SOCKET_PREFIX: &str = "paws-vpn-handoff-";

static HANDOFF_LISTENER: OnceLock<Mutex<Option<OwnedFd>>> = OnceLock::new();

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HandoffPayload {
    pub(crate) attempt_id: String,
    pub(crate) options_json: String,
}

pub(crate) struct ReceivedHandoff {
    pub(crate) payload: HandoffPayload,
    pub(crate) ashmem: OwnedFd,
    pub(crate) notification: OwnedFd,
}

pub(crate) fn prepare(attempt_id: &str) -> io::Result<String> {
    if attempt_id.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "VPN handoff attempt id is empty",
        ));
    }
    let token = random_token()?;
    let address = HandoffAddress::new(&token)?;
    let listener = create_socket()?;
    // SAFETY: listener is live and address contains an initialized sockaddr_un.
    if unsafe { libc::bind(listener.as_raw_fd(), address.pointer(), address.length) } < 0
        // SAFETY: the descriptor remains live after a successful bind.
        || unsafe { libc::listen(listener.as_raw_fd(), 1) } < 0
    {
        return Err(io::Error::last_os_error());
    }
    set_close_on_exec(listener.as_raw_fd())?;
    set_nonblocking(listener.as_raw_fd())?;
    let mut slot = listener_slot()
        .lock()
        .map_err(|_| io::Error::other("VPN handoff listener lock is poisoned"))?;
    *slot = Some(listener);
    Ok(token)
}

pub(crate) fn send<F>(
    ashmem_fd: RawFd,
    notification_fd: RawFd,
    attempt_id: String,
    options_json: String,
    keep_waiting: F,
) -> io::Result<()>
where
    F: Fn() -> io::Result<()>,
{
    if ashmem_fd < 0 || notification_fd < 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "VPN handoff descriptors are invalid",
        ));
    }
    let listener = listener_slot()
        .lock()
        .map_err(|_| io::Error::other("VPN handoff listener lock is poisoned"))?
        .take()
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotConnected, "VPN handoff is not prepared")
        })?;
    wait_readable(listener.as_raw_fd(), keep_waiting)?;
    let connection = loop {
        // SAFETY: listener is live and no peer address is requested.
        let fd = unsafe {
            libc::accept(
                listener.as_raw_fd(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if fd >= 0 {
            // SAFETY: accept returned a fresh descriptor owned by this value.
            break unsafe { OwnedFd::from_raw_fd(fd) };
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    };
    set_close_on_exec(connection.as_raw_fd())?;
    let payload = serde_json::to_vec(&HandoffPayload {
        attempt_id,
        options_json,
    })
    .map_err(io::Error::other)?;
    if payload.len() > HANDOFF_MAX_PAYLOAD {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "VPN handoff payload is too large",
        ));
    }
    send_frame(
        connection.as_raw_fd(),
        &[ashmem_fd, notification_fd],
        &payload,
    )
}

pub(crate) fn receive(token: &str) -> io::Result<ReceivedHandoff> {
    let address = HandoffAddress::new(token)?;
    let connection = create_socket()?;
    set_close_on_exec(connection.as_raw_fd())?;
    // SAFETY: connection is live and address contains an initialized sockaddr_un.
    if unsafe { libc::connect(connection.as_raw_fd(), address.pointer(), address.length) } < 0 {
        return Err(io::Error::last_os_error());
    }
    let (payload, mut descriptors) = receive_frame(connection.as_raw_fd())?;
    if descriptors.len() != 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "VPN handoff carried {} descriptors instead of 2",
                descriptors.len()
            ),
        ));
    }
    let notification = descriptors.pop().expect("validated descriptor count");
    let ashmem = descriptors.pop().expect("validated descriptor count");
    let payload = serde_json::from_slice(&payload).map_err(io::Error::other)?;
    Ok(ReceivedHandoff {
        payload,
        ashmem,
        notification,
    })
}

fn listener_slot() -> &'static Mutex<Option<OwnedFd>> {
    HANDOFF_LISTENER.get_or_init(|| Mutex::new(None))
}

fn random_token() -> io::Result<String> {
    let mut bytes = [0_u8; 16];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    let mut token = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut token, "{byte:02x}").expect("writing to String cannot fail");
    }
    Ok(token)
}

struct HandoffAddress {
    raw: libc::sockaddr_un,
    length: libc::socklen_t,
}

impl HandoffAddress {
    fn new(token: &str) -> io::Result<Self> {
        if token.len() != 32 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "VPN handoff token is invalid",
            ));
        }
        // SAFETY: a zeroed sockaddr_un is a valid base before setting fields.
        let mut raw: libc::sockaddr_un = unsafe { std::mem::zeroed() };
        raw.sun_family = libc::AF_UNIX as libc::sa_family_t;
        let name = format!("\0{HANDOFF_SOCKET_PREFIX}{token}");
        if name.len() > raw.sun_path.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "VPN handoff socket name is too long",
            ));
        }
        for (target, byte) in raw.sun_path.iter_mut().zip(name.bytes()) {
            *target = byte as libc::c_char;
        }
        Ok(Self {
            raw,
            length: (std::mem::offset_of!(libc::sockaddr_un, sun_path) + name.len())
                as libc::socklen_t,
        })
    }

    fn pointer(&self) -> *const libc::sockaddr {
        (&self.raw as *const libc::sockaddr_un).cast()
    }
}

fn create_socket() -> io::Result<OwnedFd> {
    // SAFETY: socket has no borrowed pointers and returns a fresh descriptor.
    let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a successful socket result transfers ownership to this value.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn set_close_on_exec(fd: RawFd) -> io::Result<()> {
    // SAFETY: fd is a live descriptor owned by the caller.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0
        // SAFETY: the same live descriptor receives its previous flags plus CLOEXEC.
        || unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn set_nonblocking(fd: RawFd) -> io::Result<()> {
    // SAFETY: fd is a live descriptor owned by the caller.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0
        // SAFETY: the same live descriptor receives its previous flags plus NONBLOCK.
        || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn wait_readable<F>(fd: RawFd, keep_waiting: F) -> io::Result<()>
where
    F: Fn() -> io::Result<()>,
{
    let deadline = Instant::now() + HANDOFF_MAX_WAIT;
    let mut descriptor = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    loop {
        keep_waiting()?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "VPN Extension did not connect to the handoff socket",
            ));
        }
        let timeout = remaining.min(HANDOFF_POLL);
        // SAFETY: descriptor points to one initialized pollfd value.
        let result = unsafe {
            libc::poll(
                &mut descriptor,
                1,
                timeout.as_millis().min(i32::MAX as u128) as i32,
            )
        };
        if result > 0 && descriptor.revents & libc::POLLIN != 0 {
            return Ok(());
        }
        if result == 0 {
            continue;
        }
        if result < 0 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
            continue;
        }
        if result > 0 {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "VPN handoff socket closed before Extension connected",
            ));
        }
        return Err(io::Error::last_os_error());
    }
}

fn send_frame(fd: RawFd, descriptors: &[RawFd; 2], payload: &[u8]) -> io::Result<()> {
    let mut frame = Vec::with_capacity(HANDOFF_HEADER_SIZE + payload.len());
    frame.extend_from_slice(&HANDOFF_MAGIC.to_le_bytes());
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(&(descriptors.len() as u32).to_le_bytes());
    frame.extend_from_slice(payload);

    let mut io_vector = libc::iovec {
        iov_base: frame.as_ptr().cast_mut().cast(),
        iov_len: frame.len(),
    };
    // SAFETY: CMSG_SPACE computes the storage required for two RawFd values.
    let control_len = unsafe {
        libc::CMSG_SPACE((descriptors.len() * std::mem::size_of::<RawFd>()) as libc::c_uint)
            as usize
    };
    let mut control = vec![0_u8; control_len];
    // SAFETY: zero is a valid base for msghdr before its pointers are populated.
    let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
    message.msg_iov = &mut io_vector;
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen = control.len() as _;
    // SAFETY: message owns a control buffer sized with CMSG_SPACE.
    let header = unsafe { libc::CMSG_FIRSTHDR(&message) };
    if header.is_null() {
        return Err(io::Error::other(
            "failed to build VPN handoff control message",
        ));
    }
    // SAFETY: header points into the writable control buffer above.
    unsafe {
        (*header).cmsg_level = libc::SOL_SOCKET;
        (*header).cmsg_type = libc::SCM_RIGHTS;
        (*header).cmsg_len =
            libc::CMSG_LEN((descriptors.len() * std::mem::size_of::<RawFd>()) as libc::c_uint) as _;
        std::ptr::copy_nonoverlapping(
            descriptors.as_ptr(),
            libc::CMSG_DATA(header).cast::<RawFd>(),
            descriptors.len(),
        );
    }

    let sent = loop {
        // SAFETY: all message pointers remain valid during sendmsg.
        let sent = unsafe { libc::sendmsg(fd, &message, send_flags()) };
        if sent >= 0 {
            break sent as usize;
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    };
    if sent == 0 {
        return Err(io::Error::new(
            io::ErrorKind::WriteZero,
            "VPN handoff socket wrote no data",
        ));
    }
    send_remaining(fd, &frame[sent..])
}

fn send_remaining(fd: RawFd, mut bytes: &[u8]) -> io::Result<()> {
    while !bytes.is_empty() {
        // SAFETY: fd is live and bytes remains valid for the call.
        let sent = unsafe { libc::send(fd, bytes.as_ptr().cast(), bytes.len(), send_flags()) };
        if sent > 0 {
            bytes = &bytes[sent as usize..];
            continue;
        }
        if sent == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "VPN handoff socket wrote no data",
            ));
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
    Ok(())
}

fn receive_frame(fd: RawFd) -> io::Result<(Vec<u8>, Vec<OwnedFd>)> {
    let mut frame = vec![0_u8; HANDOFF_HEADER_SIZE + HANDOFF_MAX_PAYLOAD];
    let mut io_vector = libc::iovec {
        iov_base: frame.as_mut_ptr().cast(),
        iov_len: frame.len(),
    };
    // Leave room for more descriptors than expected so extras can be closed safely.
    // SAFETY: CMSG_SPACE computes the required ancillary-data storage.
    let control_len =
        unsafe { libc::CMSG_SPACE((8 * std::mem::size_of::<RawFd>()) as libc::c_uint) as usize };
    let mut control = vec![0_u8; control_len];
    // SAFETY: zero is a valid base for msghdr before pointers are populated.
    let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
    message.msg_iov = &mut io_vector;
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen = control.len() as _;

    let mut received = loop {
        // SAFETY: all message pointers remain valid during recvmsg.
        let count = unsafe { libc::recvmsg(fd, &mut message, 0) };
        if count > 0 {
            break count as usize;
        }
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "VPN handoff peer closed before sending data",
            ));
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    };
    let descriptors = receive_descriptors(&message)?;
    while received < HANDOFF_HEADER_SIZE {
        received += receive_more(fd, &mut frame[received..HANDOFF_HEADER_SIZE])?;
    }
    let magic = u32::from_le_bytes(frame[0..4].try_into().expect("fixed handoff header"));
    let payload_len =
        u32::from_le_bytes(frame[4..8].try_into().expect("fixed handoff header")) as usize;
    let descriptor_count =
        u32::from_le_bytes(frame[8..12].try_into().expect("fixed handoff header")) as usize;
    if magic != HANDOFF_MAGIC || payload_len > HANDOFF_MAX_PAYLOAD {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "VPN handoff frame header is invalid",
        ));
    }
    if descriptor_count != descriptors.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "VPN handoff descriptor count does not match the frame",
        ));
    }
    let total = HANDOFF_HEADER_SIZE + payload_len;
    while received < total {
        received += receive_more(fd, &mut frame[received..total])?;
    }
    frame.drain(..HANDOFF_HEADER_SIZE);
    frame.truncate(payload_len);
    Ok((frame, descriptors))
}

fn receive_more(fd: RawFd, bytes: &mut [u8]) -> io::Result<usize> {
    loop {
        // SAFETY: fd is live and bytes is writable for the supplied length.
        let count = unsafe { libc::recv(fd, bytes.as_mut_ptr().cast(), bytes.len(), 0) };
        if count > 0 {
            return Ok(count as usize);
        }
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "VPN handoff frame was truncated",
            ));
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

fn receive_descriptors(message: &libc::msghdr) -> io::Result<Vec<OwnedFd>> {
    let mut descriptors = Vec::new();
    // SAFETY: message owns the control buffer for the duration of this function.
    let mut header = unsafe { libc::CMSG_FIRSTHDR(message) };
    while !header.is_null() {
        // SAFETY: header is returned by the CMSG traversal helpers.
        unsafe {
            if (*header).cmsg_level == libc::SOL_SOCKET && (*header).cmsg_type == libc::SCM_RIGHTS {
                let base = libc::CMSG_LEN(0) as usize;
                let length = (*header).cmsg_len as usize;
                if length < base {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "VPN handoff control message is malformed",
                    ));
                }
                let count = (length - base) / std::mem::size_of::<RawFd>();
                let data = libc::CMSG_DATA(header).cast::<RawFd>();
                for index in 0..count {
                    let fd = *data.add(index);
                    if fd >= 0 {
                        // SAFETY: SCM_RIGHTS created a fresh descriptor for this process.
                        let owned = OwnedFd::from_raw_fd(fd);
                        set_close_on_exec(owned.as_raw_fd())?;
                        descriptors.push(owned);
                    }
                }
            }
            header = libc::CMSG_NXTHDR(message, header);
        }
    }
    Ok(descriptors)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
const fn send_flags() -> libc::c_int {
    libc::MSG_NOSIGNAL
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
const fn send_flags() -> libc::c_int {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_tokens_that_cannot_name_a_one_time_socket() {
        assert!(HandoffAddress::new("").is_err());
        assert!(HandoffAddress::new("not-a-token").is_err());
        assert!(HandoffAddress::new("0123456789abcdef0123456789abcdef").is_ok());
    }
}
