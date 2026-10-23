//! Benchmark for `IpCidrRuleSet` lookups (issue #172 / audit H2).
//!
//! Rule-providers with `behavior: ipcidr` commonly carry thousands of CIDRs
//! (country/ASN lists). This bench builds a 10k-entry synthetic set and times
//! hit and miss lookups two ways:
//!
//! 1. **`ipcidr_ruleset/*`** — the real `IpCidrRuleSet` (split `IpRange`
//!    Patricia tries, O(prefix-depth) lookup).
//! 2. **`linear_scan_baseline/*`** — the pre-#172 representation
//!    (`Vec<IpNet>` + `iter().any()`), kept here so the before/after delta
//!    stays recorded per ADR-0006 without digging through git history.
//!
//! Build cost is benched separately (`build_10k`) since it runs only at
//! config (re)load.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use ipnet::IpNet;
use meow_common::{Metadata, RuleMatchHelper};
use meow_rules::parser::ParserContext;
use meow_rules::rule_set::{build_rule_set, RuleSetBehavior};
use std::net::{IpAddr, Ipv4Addr};

/// 10k /24 networks spread over 10.0.0.0/8 — disjoint, realistic prefix mix.
fn synthetic_entries() -> Vec<String> {
    let mut out = Vec::with_capacity(10_000);
    for i in 0..10_000u32 {
        let b = (i >> 8) & 0xff;
        let c = i & 0xff;
        out.push(format!("10.{b}.{c}.0/24"));
    }
    out
}

fn meta_for(ip: IpAddr) -> Metadata {
    Metadata {
        dst_ip: Some(ip),
        ..Default::default()
    }
}

fn bench_ipcidr(c: &mut Criterion) {
    let entries = synthetic_entries();
    let ctx = ParserContext::default();
    let helper = RuleMatchHelper;

    let set = build_rule_set(RuleSetBehavior::IpCidr, &entries, &ctx);
    // Worst-case-ish hit: an entry late in insertion order.
    let hit = meta_for(IpAddr::V4(Ipv4Addr::new(10, 38, 200, 7)));
    let miss = meta_for(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9)));

    let mut g = c.benchmark_group("ipcidr_ruleset");
    g.bench_function("hit_10k", |b| {
        b.iter(|| black_box(set.matches(black_box(&hit), &helper)));
    });
    g.bench_function("miss_10k", |b| {
        b.iter(|| black_box(set.matches(black_box(&miss), &helper)));
    });
    g.bench_function("build_10k", |b| {
        b.iter(|| black_box(build_rule_set(RuleSetBehavior::IpCidr, &entries, &ctx)));
    });
    g.finish();

    // Pre-#172 baseline: Vec<IpNet> linear scan.
    let cidrs: Vec<IpNet> = entries.iter().map(|e| e.parse().unwrap()).collect();
    let hit_ip = IpAddr::V4(Ipv4Addr::new(10, 38, 200, 7));
    let miss_ip = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9));

    let mut g = c.benchmark_group("linear_scan_baseline");
    g.bench_function("hit_10k", |b| {
        b.iter(|| black_box(cidrs.iter().any(|net| net.contains(black_box(&hit_ip)))));
    });
    g.bench_function("miss_10k", |b| {
        b.iter(|| black_box(cidrs.iter().any(|net| net.contains(black_box(&miss_ip)))));
    });
    g.finish();
}

criterion_group!(benches, bench_ipcidr);
criterion_main!(benches);
