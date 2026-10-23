//! Actual UDP + DnsServer regression tests for the terminal policy boundary.
use super::*;
use crate::server::DnsServer;
use hickory_proto::op::{MessageType, OpCode, Query, ResponseCode};
use hickory_proto::rr::rdata::{A, AAAA, TXT};
use hickory_proto::rr::{RData, Record};
use hickory_proto::serialize::binary::BinEncodable;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::task::JoinHandle;

#[derive(Clone, Copy)]
enum Outcome {
    Positive(u8),
    NoData,
    NxDomain,
    Drop,
}

struct Upstream {
    address: SocketAddr,
    requests: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}

impl Upstream {
    async fn new(outcome: Outcome) -> Self {
        let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let address = socket.local_addr().unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&requests);
        let task = tokio::spawn(async move {
            let mut buffer = [0_u8; 4096];
            loop {
                let (length, peer) = socket.recv_from(&mut buffer).await.unwrap();
                let request = Message::from_vec(&buffer[..length]).unwrap();
                observed.fetch_add(1, Ordering::SeqCst);
                if matches!(outcome, Outcome::Drop) {
                    continue;
                }
                let question = request.queries[0].clone();
                let mut response =
                    Message::new(request.metadata.id, MessageType::Response, OpCode::Query);
                response.add_query(question.clone());
                match outcome {
                    Outcome::Positive(tail) => {
                        let data = match question.query_type {
                            RecordType::A => RData::A(A(Ipv4Addr::new(203, 0, 113, tail))),
                            RecordType::AAAA => RData::AAAA(AAAA(
                                format!("2001:db8::{tail}").parse::<Ipv6Addr>().unwrap(),
                            )),
                            RecordType::TXT => {
                                RData::TXT(TXT::new(vec![format!("upstream-{tail}")]))
                            }
                            _ => unreachable!(),
                        };
                        response.add_answer(Record::from_rdata(question.name, 2, data));
                    }
                    Outcome::NxDomain => response.metadata.response_code = ResponseCode::NXDomain,
                    Outcome::NoData => {}
                    Outcome::Drop => unreachable!(),
                }
                socket
                    .send_to(&response.to_bytes().unwrap(), peer)
                    .await
                    .unwrap();
            }
        });
        Self {
            address,
            requests,
            task,
        }
    }

    fn client(&self) -> Arc<DnsClient> {
        Arc::new(DnsClient::udp(self.address).with_timeout(Duration::from_millis(80)))
    }

    fn count(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

impl Drop for Upstream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn resolver(
    policy: Option<&Upstream>,
    main: &Upstream,
    fallback: &Upstream,
    domain_gate: bool,
) -> Resolver {
    let mut resolver = Resolver::new(
        vec![],
        vec![],
        DnsMode::Normal,
        DomainTrie::new(),
        false,
        true,
    );
    resolver.main = vec![main.client()];
    resolver.fallback = Some(vec![fallback.client()]);
    resolver.policy = policy.map(|upstream| {
        let mut policy = NameserverPolicy::new();
        policy.insert_wildcard(
            "+.policy.qa.invalid",
            PolicyEntry {
                nameservers: vec![upstream.client()],
            },
        );
        policy
    });
    let mut domain = DomainTrie::new();
    if domain_gate {
        domain.insert("+.policy.qa.invalid", ());
    }
    resolver.fallback_filter = Some(FallbackFilter {
        geoip_enabled: false,
        geoip_code: "CN".to_owned(),
        ipcidr: vec![
            "203.0.113.0/24".parse().unwrap(),
            "2001:db8::/32".parse().unwrap(),
        ],
        domain,
        geoip_reader: None,
    });
    resolver
}

async fn wire(resolver: &Resolver, host: &str, record_type: RecordType) -> Message {
    let mut query = Message::new(0x3210, MessageType::Query, OpCode::Query);
    query.metadata.recursion_desired = true;
    query.add_query(Query::query(host.parse().unwrap(), record_type));
    let query_bytes = query.to_bytes().unwrap();
    // DNS names become FQDNs on the wire. Compare to the actual sent
    // question, including name, type and class, rather than its pre-wire form.
    let wire_query = Message::from_vec(&query_bytes).unwrap();
    let reply = tokio::time::timeout(
        Duration::from_secs(2),
        DnsServer::handle_query(&query_bytes, resolver),
    )
    .await
    .unwrap()
    .unwrap();
    let reply = Message::from_vec(&reply).unwrap();
    assert_eq!(reply.metadata.id, 0x3210);
    assert_eq!(reply.queries, wire_query.queries);
    reply
}

#[tokio::test]
async fn policy_positive_precedes_global_domain_and_ip_filters_for_all_wire_types() {
    for domain_gate in [false, true] {
        let policy = Upstream::new(Outcome::Positive(11)).await;
        let main = Upstream::new(Outcome::Positive(12)).await;
        let fallback = Upstream::new(Outcome::NxDomain).await;
        let resolver = resolver(Some(&policy), &main, &fallback, domain_gate);
        for record_type in [RecordType::A, RecordType::AAAA, RecordType::TXT] {
            let response = wire(&resolver, "probe.policy.qa.invalid", record_type).await;
            assert_eq!(response.metadata.response_code, ResponseCode::NoError);
            assert_eq!(response.answers.len(), 1);
            match &response.answers[0].data {
                RData::A(value) => assert_eq!(value.0.to_string(), "203.0.113.11"),
                RData::AAAA(value) => assert_eq!(value.0.to_string(), "2001:db8::11"),
                RData::TXT(value) => assert_eq!(value.txt_data[0].as_ref(), b"upstream-11"),
                _ => panic!("unexpected upstream answer"),
            }
        }
        assert_eq!(policy.count(), 3);
        assert_eq!(main.count(), 0);
        assert_eq!(fallback.count(), 0);
    }
}

#[tokio::test]
async fn policy_negative_and_nodata_are_terminal_without_global_queries() {
    for outcome in [Outcome::NxDomain, Outcome::NoData] {
        let policy = Upstream::new(outcome).await;
        let main = Upstream::new(Outcome::Positive(12)).await;
        let fallback = Upstream::new(Outcome::Positive(13)).await;
        let resolver = resolver(Some(&policy), &main, &fallback, false);
        for record_type in [RecordType::A, RecordType::AAAA, RecordType::TXT] {
            let host = format!("negative-{}.policy.qa.invalid", u16::from(record_type));
            let response = wire(&resolver, &host, record_type).await;
            assert_eq!(
                response.metadata.response_code,
                if matches!(outcome, Outcome::NxDomain) {
                    ResponseCode::NXDomain
                } else {
                    ResponseCode::NoError
                }
            );
            assert!(response.answers.is_empty());
        }
        assert_eq!(policy.count(), 3);
        assert_eq!(main.count(), 0);
        assert_eq!(fallback.count(), 0);
    }
}

#[tokio::test]
async fn policy_transport_failure_is_terminal_without_global_queries() {
    let policy = Upstream::new(Outcome::Drop).await;
    let main = Upstream::new(Outcome::Positive(12)).await;
    let fallback = Upstream::new(Outcome::Positive(13)).await;
    let resolver = resolver(Some(&policy), &main, &fallback, false);
    for record_type in [RecordType::A, RecordType::AAAA, RecordType::TXT] {
        let host = format!("timeout-{}.policy.qa.invalid", u16::from(record_type));
        let response = wire(&resolver, &host, record_type).await;
        assert_eq!(response.metadata.response_code, ResponseCode::ServFail);
        assert!(response.answers.is_empty());
    }
    assert_eq!(policy.count(), 3);
    assert_eq!(main.count(), 0);
    assert_eq!(fallback.count(), 0);
}

#[tokio::test]
async fn unmatched_domains_retain_the_global_domain_and_ip_fallback_pipeline() {
    let main = Upstream::new(Outcome::Positive(12)).await;
    let fallback = Upstream::new(Outcome::Positive(13)).await;
    let resolver = resolver(None, &main, &fallback, true);
    for record_type in [RecordType::A, RecordType::AAAA, RecordType::TXT] {
        let response = wire(&resolver, "domain-gated.policy.qa.invalid", record_type).await;
        assert_eq!(response.metadata.response_code, ResponseCode::NoError);
        assert_eq!(response.answers.len(), 1);
    }
    assert_eq!(main.count(), 0);
    assert_eq!(fallback.count(), 3);

    let resolver = self::resolver(None, &main, &fallback, false);
    for record_type in [RecordType::A, RecordType::AAAA] {
        let response = wire(&resolver, "ip-gated.policy.qa.invalid", record_type).await;
        assert_eq!(response.metadata.response_code, ResponseCode::NoError);
        let value = &response.answers[0].data;
        assert!(
            matches!(value,RData::A(value) if value.0.to_string()=="203.0.113.13")
                || matches!(value,RData::AAAA(value) if value.0.to_string()=="2001:db8::13")
        );
    }
    assert_eq!(main.count(), 2);
    assert_eq!(fallback.count(), 5);
}

#[tokio::test]
async fn unmatched_main_transport_failure_still_uses_fallback() {
    let main = Upstream::new(Outcome::Drop).await;
    let fallback = Upstream::new(Outcome::Positive(13)).await;
    let resolver = resolver(None, &main, &fallback, false);
    for record_type in [RecordType::A, RecordType::AAAA, RecordType::TXT] {
        let host = format!("fallback-{}.policy.qa.invalid", u16::from(record_type));
        let response = wire(&resolver, &host, record_type).await;
        assert_eq!(response.metadata.response_code, ResponseCode::NoError);
        assert_eq!(response.answers.len(), 1);
    }
    assert_eq!(main.count(), 3);
    assert_eq!(fallback.count(), 3);
}
