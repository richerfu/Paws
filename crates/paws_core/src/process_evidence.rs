//! Bounded process-local evidence. A sample never claims a termination cause.
use serde::{Deserialize, Serialize};
use std::{fs, io::Read, path::Path};

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProcessSample {
    captured_at_ms: u64,
    pid: u32,
    rss_kb: Option<u64>,
    peak_rss_kb: Option<u64>,
    virtual_kb: Option<u64>,
    threads: Option<u64>,
    fd_count: Option<usize>,
    user_cpu_ticks: Option<u64>,
    system_cpu_ticks: Option<u64>,
    process_start_ticks: Option<u64>,
    clock_ticks_per_second: Option<u64>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProcessEvidenceSummary {
    sample_count: usize,
    distinct_processes: usize,
    window_ms: u64,
    first_rss_kb: Option<u64>,
    last_rss_kb: Option<u64>,
    peak_rss_kb: Option<u64>,
    rss_delta_kb: Option<i64>,
    rss_slope_kb_per_hour: Option<i64>,
    first_fd_count: Option<usize>,
    last_fd_count: Option<usize>,
    max_fd_count: Option<usize>,
    max_threads: Option<u64>,
    assessment: &'static str,
}

pub(crate) fn summarize(samples: &[ProcessSample]) -> ProcessEvidenceSummary {
    let mut summary = ProcessEvidenceSummary {
        sample_count: samples.len(),
        distinct_processes: samples
            .iter()
            .map(|sample| (sample.pid, sample.process_start_ticks))
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        window_ms: samples
            .last()
            .and_then(|last| {
                samples
                    .first()
                    .map(|first| last.captured_at_ms.saturating_sub(first.captured_at_ms))
            })
            .unwrap_or_default(),
        first_rss_kb: samples.iter().find_map(|sample| sample.rss_kb),
        last_rss_kb: samples.iter().rev().find_map(|sample| sample.rss_kb),
        peak_rss_kb: samples.iter().filter_map(|sample| sample.peak_rss_kb).max(),
        first_fd_count: samples.iter().find_map(|sample| sample.fd_count),
        last_fd_count: samples.iter().rev().find_map(|sample| sample.fd_count),
        max_fd_count: samples.iter().filter_map(|sample| sample.fd_count).max(),
        max_threads: samples.iter().filter_map(|sample| sample.threads).max(),
        assessment: "insufficient-window",
        ..Default::default()
    };
    summary.rss_delta_kb = summary
        .first_rss_kb
        .zip(summary.last_rss_kb)
        .map(|(first, last)| last as i64 - first as i64);
    summary.rss_slope_kb_per_hour = rss_slope_kb_per_hour(samples);
    summary.assessment = if summary.distinct_processes > 1 {
        "process-restarted-window-not-comparable"
    } else if summary.sample_count < 10 || summary.window_ms < 5 * 60 * 1000 {
        "insufficient-window"
    } else if summary.rss_slope_kb_per_hour.unwrap_or_default() > 8 * 1024 {
        "sustained-growth-observed-run-longer-test"
    } else {
        "no-fast-growth-observed"
    };
    summary
}

fn rss_slope_kb_per_hour(samples: &[ProcessSample]) -> Option<i64> {
    let origin = samples.first()?.captured_at_ms;
    let values: Vec<(f64, f64)> = samples
        .iter()
        .filter_map(|sample| {
            sample.rss_kb.map(|rss| {
                (
                    sample.captured_at_ms.saturating_sub(origin) as f64 / 3_600_000.0,
                    rss as f64,
                )
            })
        })
        .collect();
    if values.len() < 2 {
        return None;
    }
    let count = values.len() as f64;
    let mean_x = values.iter().map(|(x, _)| x).sum::<f64>() / count;
    let mean_y = values.iter().map(|(_, y)| y).sum::<f64>() / count;
    let denominator = values
        .iter()
        .map(|(x, _)| (x - mean_x).powi(2))
        .sum::<f64>();
    if denominator <= f64::EPSILON {
        return None;
    }
    let slope = values
        .iter()
        .map(|(x, y)| (x - mean_x) * (y - mean_y))
        .sum::<f64>()
        / denominator;
    Some(slope.round() as i64)
}

pub(crate) fn read_bounded(path: &Path, cap: usize) -> Option<String> {
    let mut content = String::new();
    fs::File::open(path)
        .ok()?
        .take((cap + 1) as u64)
        .read_to_string(&mut content)
        .ok()?;
    (content.len() <= cap).then_some(content)
}

#[cfg(any(target_env = "ohos", test))]
fn parse_sample(status: &str, stat: &str) -> ProcessSample {
    let value = |key: &str| {
        status.lines().find_map(|line| {
            line.strip_prefix(key)?
                .split_whitespace()
                .next()?
                .parse::<u64>()
                .ok()
        })
    };
    // Linux/OhOS stat comm may contain spaces and parentheses. Numeric fields
    // follow its final ')'; field 3 is index 0 in this suffix.
    let fields: Vec<_> = stat
        .rsplit_once(')')
        .map(|(_, tail)| tail.split_whitespace().collect())
        .unwrap_or_default();
    let number = |index| fields.get(index).and_then(|v: &&str| v.parse::<u64>().ok());
    ProcessSample {
        rss_kb: value("VmRSS:"),
        peak_rss_kb: value("VmHWM:"),
        virtual_kb: value("VmSize:"),
        threads: value("Threads:"),
        user_cpu_ticks: number(11),
        system_cpu_ticks: number(12),
        process_start_ticks: number(19),
        ..Default::default()
    }
}

#[cfg(any(target_env = "ohos", test))]
fn append_sample(root: &Path, sample: ProcessSample) -> std::io::Result<()> {
    let path = root.join("vpn-process-evidence.json");
    let mut samples: Vec<ProcessSample> = read_bounded(&path, 65536)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    if samples.len() >= 120 {
        samples.drain(..samples.len() - 119);
    }
    samples.push(sample);
    let temporary = root.join("vpn-process-evidence.json.tmp");
    fs::write(&temporary, serde_json::to_vec(&samples)?)?;
    fs::rename(temporary, path)
}

pub(crate) fn record(root: &Path) {
    #[cfg(target_env = "ohos")]
    {
        use std::sync::atomic::{AtomicU64, Ordering};
        static LAST: AtomicU64 = AtomicU64::new(0);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let previous = LAST.load(Ordering::Relaxed);
        if now.saturating_sub(previous) < 30000
            || LAST
                .compare_exchange(previous, now, Ordering::Relaxed, Ordering::Relaxed)
                .is_err()
        {
            return;
        }
        let Some(status) = read_bounded(Path::new("/proc/self/status"), 16384) else {
            return;
        };
        let stat = read_bounded(Path::new("/proc/self/stat"), 4096).unwrap_or_default();
        let mut sample = parse_sample(&status, &stat);
        sample.captured_at_ms = now;
        sample.pid = std::process::id();
        sample.fd_count = fs::read_dir("/proc/self/fd")
            .ok()
            .map(|entries| entries.take(65536).filter_map(Result::ok).count());
        let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        sample.clock_ticks_per_second = (ticks > 0).then_some(ticks as u64);
        if append_sample(root, sample).is_err() {
            tracing::warn!("VPN process evidence write failed");
        }
    }
    #[cfg(not(target_env = "ohos"))]
    let _ = root;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proc_fields_handle_spaces_and_missing_memory() {
        let stat = format!(
            "123 (vpn worker (x)) S {}",
            (4..=24)
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join(" ")
        );
        let p = parse_sample("VmRSS:\t123 kB\nVmHWM: 456 kB\nThreads: 9\n", &stat);
        assert_eq!(p.rss_kb, Some(123));
        assert_eq!(p.peak_rss_kb, Some(456));
        assert_eq!(p.threads, Some(9));
        assert_eq!(p.virtual_kb, None);
        assert_eq!(p.user_cpu_ticks, Some(14));
        assert_eq!(p.system_cpu_ticks, Some(15));
        assert_eq!(p.process_start_ticks, Some(22));
        assert!(parse_sample("", "malformed").user_cpu_ticks.is_none());
    }
    #[test]
    fn disk_history_is_bounded_and_survives_reopen() {
        let root = std::env::temp_dir().join(format!("tinypaws-evidence-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        for n in 0..125 {
            append_sample(
                &root,
                ProcessSample {
                    captured_at_ms: n,
                    ..Default::default()
                },
            )
            .unwrap();
        }
        let path = root.join("vpn-process-evidence.json");
        let samples: Vec<ProcessSample> =
            serde_json::from_str(&read_bounded(&path, 65536).unwrap()).unwrap();
        assert_eq!(samples.len(), 120);
        assert_eq!(samples[0].captured_at_ms, 5);
        assert_eq!(samples[119].captured_at_ms, 124);
        assert!(read_bounded(&path, 32).is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn summary_quantifies_one_process_without_calling_short_windows_leaks() {
        let samples = (0..12)
            .map(|index| ProcessSample {
                captured_at_ms: index * 30_000,
                pid: 7,
                process_start_ticks: Some(9),
                rss_kb: Some(100_000 + index * 10),
                peak_rss_kb: Some(101_000),
                fd_count: Some(80 + index as usize % 3),
                threads: Some(36),
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let summary = summarize(&samples);
        assert_eq!(summary.sample_count, 12);
        assert_eq!(summary.distinct_processes, 1);
        assert_eq!(summary.rss_delta_kb, Some(110));
        assert_eq!(summary.rss_slope_kb_per_hour, Some(1200));
        assert_eq!(summary.assessment, "no-fast-growth-observed");
    }
}
