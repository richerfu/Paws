use super::*;

pub(super) fn parse_udp_packet(ip_data: &[u8]) -> Option<(u32, u16, u32, u16, &[u8])> {
    if ip_data.len() < 28 || (ip_data[0] >> 4) != 4 || ip_data[9] != 17 {
        return None;
    }
    let ihl = usize::from(ip_data[0] & 0x0f) * 4;
    if ihl < 20 || ip_data.len() < ihl + 8 {
        return None;
    }
    let total_len = u16::from_be_bytes([ip_data[2], ip_data[3]]) as usize;
    if total_len < ihl + 8 || total_len > ip_data.len() {
        return None;
    }
    let fragment_field = u16::from_be_bytes([ip_data[6], ip_data[7]]);
    if fragment_field & 0x3fff != 0 {
        return None;
    }
    let src_ip = u32::from_ne_bytes([ip_data[12], ip_data[13], ip_data[14], ip_data[15]]);
    let dst_ip = u32::from_ne_bytes([ip_data[16], ip_data[17], ip_data[18], ip_data[19]]);
    let src_port = u16::from_be_bytes([ip_data[ihl], ip_data[ihl + 1]]);
    let dst_port = u16::from_be_bytes([ip_data[ihl + 2], ip_data[ihl + 3]]);
    let udp_len = u16::from_be_bytes([ip_data[ihl + 4], ip_data[ihl + 5]]) as usize;
    if udp_len < 8 || ihl + udp_len > total_len {
        return None;
    }
    let start = ihl + 8;
    let end = ihl + udp_len;
    (start <= end).then_some((src_ip, src_port, dst_ip, dst_port, &ip_data[start..end]))
}

pub(super) fn tun_dns_query_from_packet(
    dns_hijacking: bool,
    ip_data: &[u8],
) -> Option<(u32, u16, u32, u16, &[u8])> {
    if !dns_hijacking {
        return None;
    }
    let packet = parse_udp_packet(ip_data)?;
    (packet.3 == 53).then_some(packet)
}

pub(super) fn build_udp_packet(
    src_ip: u32,
    src_port: u16,
    dst_ip: u32,
    dst_port: u16,
    payload: &[u8],
) -> Vec<u8> {
    let udp_len = 8 + payload.len();
    let total_len = 20 + udp_len;
    let mut packet = vec![0_u8; total_len];
    packet[0] = 0x45;
    packet[2..4].copy_from_slice(&(total_len as u16).to_be_bytes());
    packet[6] = 0x40;
    packet[8] = 64;
    packet[9] = 17;
    packet[12..16].copy_from_slice(&src_ip.to_ne_bytes());
    packet[16..20].copy_from_slice(&dst_ip.to_ne_bytes());
    let checksum = ip_checksum(&packet[..20]);
    packet[10..12].copy_from_slice(&checksum.to_be_bytes());
    packet[20..22].copy_from_slice(&src_port.to_be_bytes());
    packet[22..24].copy_from_slice(&dst_port.to_be_bytes());
    packet[24..26].copy_from_slice(&(udp_len as u16).to_be_bytes());
    packet[28..].copy_from_slice(payload);
    packet
}

pub(super) fn ip_checksum(header: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    for i in (0..header.len()).step_by(2) {
        sum += if i + 1 < header.len() {
            (u32::from(header[i]) << 8) | u32::from(header[i + 1])
        } else {
            u32::from(header[i]) << 8
        };
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !sum as u16
}

pub(super) async fn write_tun_packet(fd: RawFd, pkt: &[u8]) -> bool {
    let mut retries = 0_u8;
    loop {
        let written = unsafe { libc::write(fd, pkt.as_ptr() as *const c_void, pkt.len()) };
        if written >= 0 {
            return true;
        }
        let errno = io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if errno == libc::EAGAIN && retries < 3 {
            retries += 1;
            tokio::task::yield_now().await;
            continue;
        }
        return false;
    }
}

pub(super) fn duplicate_fd(fd: i32) -> Result<i32, HMetaError> {
    let duplicated = unsafe { libc::dup(fd) };
    if duplicated < 0 {
        Err(HMetaError::Io(std::io::Error::last_os_error().to_string()))
    } else {
        Ok(duplicated)
    }
}

pub(super) fn set_nonblocking(fd: RawFd) -> Result<(), HMetaError> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(HMetaError::Io(io::Error::last_os_error().to_string()));
    }
    let rc = unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
    if rc < 0 {
        Err(HMetaError::Io(io::Error::last_os_error().to_string()))
    } else {
        Ok(())
    }
}
