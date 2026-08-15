use super::*;

pub(super) fn monotonic_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

#[derive(Debug, Clone, Default)]
pub(super) struct DnsTable {
    pub(super) records: Arc<Mutex<HashMap<IpAddr, Vec<DnsTableRecord>>>>,
}

#[derive(Debug, Clone)]
pub(super) struct DnsTableRecord {
    pub(super) host: String,
    pub(super) expires_at_ms: u64,
}

impl DnsTable {
    pub(super) fn clear(&self) {
        if let Ok(mut records) = self.records.lock() {
            records.clear();
        }
    }

    pub(super) fn insert(&self, ip: IpAddr, host: String, ttl: u32) {
        let ttl_ms = u64::from(ttl.clamp(1, 3600)) * 1000;
        if let Ok(mut records) = self.records.lock() {
            prune_expired_dns_table_records(&mut records);
            if !records.contains_key(&ip) && records.len() >= DNS_TABLE_MAX_RECORDS {
                evict_earliest_dns_table_record(&mut records);
            }
            let expires_at_ms = monotonic_ms().saturating_add(ttl_ms);
            let candidates = records.entry(ip).or_default();
            if let Some(existing) = candidates
                .iter_mut()
                .find(|record| record.host.eq_ignore_ascii_case(&host))
            {
                existing.host = host;
                existing.expires_at_ms = expires_at_ms;
                return;
            }
            if candidates.len() >= DNS_TABLE_MAX_HOSTS_PER_IP {
                if let Some(index) = candidates
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, record)| record.expires_at_ms)
                    .map(|(index, _)| index)
                {
                    candidates.remove(index);
                }
            }
            candidates.push(DnsTableRecord {
                host,
                expires_at_ms,
            });
        }
    }

    pub(super) fn lookup(&self, ip: IpAddr) -> Option<String> {
        let mut records = self.records.lock().ok()?;
        prune_expired_dns_table_records(&mut records);
        let candidates = records.get(&ip)?;
        (candidates.len() == 1).then(|| candidates[0].host.clone())
    }

    pub(super) fn has_candidates(&self, ip: IpAddr) -> bool {
        let Ok(mut records) = self.records.lock() else {
            return false;
        };
        prune_expired_dns_table_records(&mut records);
        records.get(&ip).is_some_and(|records| !records.is_empty())
    }

    #[cfg(test)]
    pub(super) fn lookup_candidates(&self, ip: IpAddr) -> Vec<String> {
        let Ok(mut records) = self.records.lock() else {
            return Vec::new();
        };
        prune_expired_dns_table_records(&mut records);
        records
            .get(&ip)
            .map(|records| records.iter().map(|record| record.host.clone()).collect())
            .unwrap_or_default()
    }
}

pub(super) fn prune_expired_dns_table_records(records: &mut HashMap<IpAddr, Vec<DnsTableRecord>>) {
    let now = monotonic_ms();
    records.retain(|_, candidates| {
        candidates.retain(|record| record.expires_at_ms > now);
        !candidates.is_empty()
    });
}

pub(super) fn evict_earliest_dns_table_record(records: &mut HashMap<IpAddr, Vec<DnsTableRecord>>) {
    if let Some(ip) = records
        .iter()
        .min_by_key(|(_, candidates)| {
            candidates
                .iter()
                .map(|record| record.expires_at_ms)
                .min()
                .unwrap_or(u64::MAX)
        })
        .map(|(ip, _)| *ip)
    {
        records.remove(&ip);
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct DnsResponseCache {
    pub(super) records: Arc<Mutex<HashMap<Vec<u8>, DnsResponseCacheRecord>>>,
}

#[derive(Debug, Clone)]
pub(super) struct DnsResponseCacheRecord {
    pub(super) response: Vec<u8>,
    pub(super) expires_at_ms: u64,
}

impl DnsResponseCache {
    pub(super) fn clear(&self) {
        if let Ok(mut records) = self.records.lock() {
            records.clear();
        }
    }

    pub(super) fn lookup(&self, query: &[u8]) -> Option<Vec<u8>> {
        let key = dns_cache_key(query)?;
        let mut records = self.records.lock().ok()?;
        let now = monotonic_ms();
        let record = records.get(&key)?;
        if record.expires_at_ms <= now {
            records.remove(&key);
            return None;
        }
        let mut response = record.response.clone();
        if response.len() >= 2 && query.len() >= 2 {
            response[0..2].copy_from_slice(&query[0..2]);
        }
        rewrite_dns_response_question(&mut response, query);
        let remaining_ttl = record.expires_at_ms.saturating_sub(now).saturating_add(999) / 1000;
        rewrite_dns_response_ttls(
            &mut response,
            remaining_ttl.clamp(1, u64::from(u32::MAX)) as u32,
        );
        Some(response)
    }

    pub(super) fn insert(&self, query: &[u8], response: &[u8], records: &[(IpAddr, String, u32)]) {
        if records.is_empty() {
            return;
        }
        let Some(key) = dns_cache_key(query) else {
            return;
        };
        let ttl = records
            .iter()
            .map(|(_, _, ttl)| *ttl)
            .min()
            .unwrap_or(0)
            .clamp(1, DNS_CACHE_MAX_TTL);
        if let Ok(mut cache_records) = self.records.lock() {
            prune_expired_dns_cache_records(&mut cache_records);
            if !cache_records.contains_key(&key) && cache_records.len() >= DNS_CACHE_MAX_RECORDS {
                evict_earliest_dns_cache_record(&mut cache_records);
            }
            cache_records.insert(
                key,
                DnsResponseCacheRecord {
                    response: response.to_vec(),
                    expires_at_ms: monotonic_ms().saturating_add(u64::from(ttl) * 1000),
                },
            );
        }
    }
}

pub(super) fn prune_expired_dns_cache_records(
    records: &mut HashMap<Vec<u8>, DnsResponseCacheRecord>,
) {
    let now = monotonic_ms();
    records.retain(|_, record| record.expires_at_ms > now);
}

pub(super) fn evict_earliest_dns_cache_record(
    records: &mut HashMap<Vec<u8>, DnsResponseCacheRecord>,
) {
    if let Some(key) = records
        .iter()
        .min_by_key(|(_, record)| record.expires_at_ms)
        .map(|(key, _)| key.clone())
    {
        records.remove(&key);
    }
}

pub(super) async fn handle_dns_query(
    tunnel: Tunnel,
    dns_table: DnsTable,
    dns_cache: DnsResponseCache,
    stats: Arc<SharedStats>,
    src_ip: u32,
    src_port: u16,
    dst_ip: u32,
    dst_port: u16,
    query: Vec<u8>,
    reply_tx: mpsc::UnboundedSender<Vec<u8>>,
) {
    if let Some(response) = dns_cache.lookup(&query) {
        stats.dns_cache_hits.fetch_add(1, Ordering::Relaxed);
        for (ip, host, ttl) in parse_dns_response_records(&response) {
            dns_table.insert(ip, host, ttl);
        }
        let _ = reply_tx.send(build_udp_packet(
            dst_ip, dst_port, src_ip, src_port, &response,
        ));
        return;
    }
    if dns_cache_key(&query).is_some() {
        stats.dns_cache_misses.fetch_add(1, Ordering::Relaxed);
    }

    let response = match meow_dns::DnsServer::handle_query(&query, tunnel.resolver()).await {
        Ok(response) => response,
        Err(_) => build_dns_error_response(&query, DnsResponseCode::ServFail),
    };
    let response_records = parse_dns_response_records(&response);
    dns_cache.insert(&query, &response, &response_records);
    for (ip, host, ttl) in response_records {
        dns_table.insert(ip, host, ttl);
    }
    let _ = reply_tx.send(build_udp_packet(
        dst_ip, dst_port, src_ip, src_port, &response,
    ));
}

pub(super) fn dns_cache_key(query: &[u8]) -> Option<Vec<u8>> {
    let question = parse_dns_query(query)?;
    let mut key = Vec::with_capacity(question.name.len() + 2);
    key.extend_from_slice(question.name.to_ascii_lowercase().as_bytes());
    key.push(0);
    key.push(match question.kind {
        DnsRecordKind::A => 1,
        DnsRecordKind::Aaaa => 28,
    });
    Some(key)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DnsQuery {
    pub(super) name: String,
    pub(super) question_end: usize,
    pub(super) kind: DnsRecordKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DnsRecordKind {
    A,
    Aaaa,
}

impl DnsRecordKind {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::Aaaa => "AAAA",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DnsResponseCode {
    ServFail = 2,
}

pub(super) fn parse_dns_query(query: &[u8]) -> Option<DnsQuery> {
    if query.len() < 12 {
        return None;
    }
    let qdcount = u16::from_be_bytes([query[4], query[5]]);
    if qdcount == 0 {
        return None;
    }
    let mut offset = 12;
    let mut labels = Vec::new();
    loop {
        let len = *query.get(offset)? as usize;
        offset += 1;
        if len == 0 {
            break;
        }
        if len & 0xc0 != 0 || offset + len > query.len() {
            return None;
        }
        labels.push(std::str::from_utf8(&query[offset..offset + len]).ok()?);
        offset += len;
    }
    if offset + 4 > query.len() {
        return None;
    }
    let qtype = u16::from_be_bytes([query[offset], query[offset + 1]]);
    let qclass = u16::from_be_bytes([query[offset + 2], query[offset + 3]]);
    if qclass != 1 {
        return None;
    }
    let kind = match qtype {
        1 => DnsRecordKind::A,
        28 => DnsRecordKind::Aaaa,
        _ => return None,
    };
    Some(DnsQuery {
        name: labels.join("."),
        question_end: offset + 4,
        kind,
    })
}

pub(super) fn parse_dns_response_records(response: &[u8]) -> Vec<(IpAddr, String, u32)> {
    if response.len() < 12 {
        return Vec::new();
    }
    let qdcount = u16::from_be_bytes([response[4], response[5]]) as usize;
    let ancount = u16::from_be_bytes([response[6], response[7]]) as usize;
    let mut offset = 12;
    let mut question_name = String::new();
    for question_index in 0..qdcount {
        let Some((name, next_offset)) = read_dns_name(response, offset) else {
            return Vec::new();
        };
        if question_index == 0 {
            question_name = name;
        }
        if next_offset + 4 > response.len() {
            return Vec::new();
        }
        offset = next_offset + 4;
    }

    let mut answers = Vec::new();
    for _ in 0..ancount {
        let Some((name, next_offset)) = read_dns_name(response, offset) else {
            break;
        };
        if next_offset + 10 > response.len() {
            break;
        }
        let record_type = u16::from_be_bytes([response[next_offset], response[next_offset + 1]]);
        let record_class =
            u16::from_be_bytes([response[next_offset + 2], response[next_offset + 3]]);
        let ttl = u32::from_be_bytes([
            response[next_offset + 4],
            response[next_offset + 5],
            response[next_offset + 6],
            response[next_offset + 7],
        ]);
        let rdlen =
            u16::from_be_bytes([response[next_offset + 8], response[next_offset + 9]]) as usize;
        let rdata_offset = next_offset + 10;
        let next_record = rdata_offset + rdlen;
        if next_record > response.len() {
            break;
        }
        let host = if name.is_empty() {
            question_name.clone()
        } else {
            name
        };
        if record_class == 1 && record_type == 1 && rdlen == 4 {
            answers.push(DnsAnswer::Address {
                host,
                ip: IpAddr::V4(Ipv4Addr::new(
                    response[rdata_offset],
                    response[rdata_offset + 1],
                    response[rdata_offset + 2],
                    response[rdata_offset + 3],
                )),
                ttl,
            });
        } else if record_class == 1 && record_type == 28 && rdlen == 16 {
            let mut octets = [0_u8; 16];
            octets.copy_from_slice(&response[rdata_offset..rdata_offset + 16]);
            answers.push(DnsAnswer::Address {
                host,
                ip: IpAddr::V6(octets.into()),
                ttl,
            });
        } else if record_class == 1 && record_type == 5 {
            if let Some((target, _)) =
                read_dns_name(response, rdata_offset).filter(|(target, _)| !target.is_empty())
            {
                answers.push(DnsAnswer::Cname { host, target });
            }
        }
        offset = next_record;
    }
    dns_records_from_answers(question_name, answers)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DnsAnswer {
    Address { host: String, ip: IpAddr, ttl: u32 },
    Cname { host: String, target: String },
}

pub(super) fn dns_records_from_answers(
    question_name: String,
    answers: Vec<DnsAnswer>,
) -> Vec<(IpAddr, String, u32)> {
    let mut cname_sources = HashMap::new();
    for answer in &answers {
        let DnsAnswer::Cname { host, target } = answer else {
            continue;
        };
        if host.is_empty() || target.is_empty() {
            continue;
        }
        cname_sources.insert(target.to_ascii_lowercase(), host.clone());
    }

    answers
        .into_iter()
        .filter_map(|answer| match answer {
            DnsAnswer::Address { host, ip, ttl } => Some((
                ip,
                dns_response_host_for_address(&host, &question_name, &cname_sources),
                ttl,
            )),
            DnsAnswer::Cname { .. } => None,
        })
        .collect()
}

pub(super) fn dns_response_host_for_address(
    host: &str,
    question_name: &str,
    cname_sources: &HashMap<String, String>,
) -> String {
    let mut current = if host.is_empty() {
        question_name.to_owned()
    } else {
        host.to_owned()
    };
    for _ in 0..8 {
        let Some(source) = cname_sources.get(&current.to_ascii_lowercase()) else {
            break;
        };
        current = source.clone();
    }
    if current.is_empty() {
        question_name.to_owned()
    } else {
        current
    }
}

pub(super) fn rewrite_dns_response_question(response: &mut [u8], query: &[u8]) {
    let Some(query_question_end) = parse_dns_question_end(query) else {
        return;
    };
    let Some(response_question_end) = parse_dns_question_end(response) else {
        return;
    };
    if query_question_end != response_question_end || query_question_end > response.len() {
        return;
    }
    response[12..query_question_end].copy_from_slice(&query[12..query_question_end]);
}

pub(super) fn rewrite_dns_response_ttls(response: &mut [u8], ttl: u32) {
    if response.len() < 12 {
        return;
    }
    let qdcount = u16::from_be_bytes([response[4], response[5]]) as usize;
    let ancount = u16::from_be_bytes([response[6], response[7]]) as usize;
    let mut offset = 12;
    for _ in 0..qdcount {
        let Some((_, next_offset)) = read_dns_name(response, offset) else {
            return;
        };
        if next_offset + 4 > response.len() {
            return;
        }
        offset = next_offset + 4;
    }

    for _ in 0..ancount {
        let Some((_, next_offset)) = read_dns_name(response, offset) else {
            return;
        };
        if next_offset + 10 > response.len() {
            return;
        }
        response[next_offset + 4..next_offset + 8].copy_from_slice(&ttl.to_be_bytes());
        let rdlen =
            u16::from_be_bytes([response[next_offset + 8], response[next_offset + 9]]) as usize;
        let next_record = next_offset + 10 + rdlen;
        if next_record > response.len() {
            return;
        }
        offset = next_record;
    }
}

pub(super) fn read_dns_name(packet: &[u8], offset: usize) -> Option<(String, usize)> {
    let mut labels = Vec::new();
    let mut pos = offset;
    let mut next_offset = offset;
    let mut jumped = false;
    let mut jumps = 0_u8;
    loop {
        let len = *packet.get(pos)?;
        if len & 0xc0 == 0xc0 {
            let second = *packet.get(pos + 1)?;
            let ptr = (usize::from(len & 0x3f) << 8) | usize::from(second);
            if ptr >= packet.len() {
                return None;
            }
            if !jumped {
                next_offset = pos + 2;
            }
            pos = ptr;
            jumped = true;
            jumps = jumps.saturating_add(1);
            if jumps > 8 {
                return None;
            }
            continue;
        }
        if len & 0xc0 != 0 {
            return None;
        }
        pos += 1;
        if len == 0 {
            if !jumped {
                next_offset = pos;
            }
            break;
        }
        let len = usize::from(len);
        if pos + len > packet.len() {
            return None;
        }
        labels.push(
            std::str::from_utf8(&packet[pos..pos + len])
                .ok()?
                .to_owned(),
        );
        pos += len;
    }
    Some((labels.join("."), next_offset))
}

pub(super) fn build_dns_error_response(query: &[u8], code: DnsResponseCode) -> Vec<u8> {
    if query.len() < 12 {
        return Vec::new();
    }
    let question_end = parse_dns_question_end(query).unwrap_or(query.len().min(12));
    let mut response = Vec::with_capacity(question_end);
    response.extend_from_slice(&query[..question_end]);
    response[2] = 0x81;
    response[3] = 0x80 | code as u8;
    response[6..8].copy_from_slice(&0_u16.to_be_bytes());
    response[8..10].copy_from_slice(&0_u16.to_be_bytes());
    response[10..12].copy_from_slice(&0_u16.to_be_bytes());
    response
}

pub(super) fn build_dns_servfail_udp_packet(
    src_ip: u32,
    src_port: u16,
    dst_ip: u32,
    dst_port: u16,
    query: &[u8],
) -> Option<Vec<u8>> {
    let response = build_dns_error_response(query, DnsResponseCode::ServFail);
    if response.is_empty() {
        None
    } else {
        Some(build_udp_packet(
            dst_ip, dst_port, src_ip, src_port, &response,
        ))
    }
}

pub(super) fn parse_dns_question_end(query: &[u8]) -> Option<usize> {
    let qdcount = u16::from_be_bytes([query[4], query[5]]);
    if qdcount == 0 {
        return Some(12);
    }
    let (_, offset) = read_dns_name(query, 12)?;
    (offset + 4 <= query.len()).then_some(offset + 4)
}

#[cfg(test)]
pub(super) fn build_dns_response(query: &[u8], request: &DnsQuery, ip: IpAddr) -> Vec<u8> {
    let mut response = Vec::with_capacity(query.len() + 32);
    response.extend_from_slice(&query[..request.question_end]);
    response[2] = 0x81;
    response[3] = 0x80;
    response[6..8].copy_from_slice(&1_u16.to_be_bytes());
    response[8..10].copy_from_slice(&0_u16.to_be_bytes());
    response[10..12].copy_from_slice(&0_u16.to_be_bytes());
    response.extend_from_slice(&[0xc0, 0x0c]);
    match ip {
        IpAddr::V4(ip) => {
            response.extend_from_slice(&1_u16.to_be_bytes());
            response.extend_from_slice(&1_u16.to_be_bytes());
            response.extend_from_slice(&60_u32.to_be_bytes());
            response.extend_from_slice(&4_u16.to_be_bytes());
            response.extend_from_slice(&ip.octets());
        }
        IpAddr::V6(ip) => {
            response.extend_from_slice(&28_u16.to_be_bytes());
            response.extend_from_slice(&1_u16.to_be_bytes());
            response.extend_from_slice(&60_u32.to_be_bytes());
            response.extend_from_slice(&16_u16.to_be_bytes());
            response.extend_from_slice(&ip.octets());
        }
    }
    response
}
