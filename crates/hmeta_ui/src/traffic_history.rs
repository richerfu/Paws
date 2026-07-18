#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TrafficHistorySummary {
    pub(crate) samples: usize,
    pub(crate) peak_download: u64,
    pub(crate) peak_upload: u64,
    pub(crate) latest_download: u64,
    pub(crate) latest_upload: u64,
}

pub(crate) fn summarize_traffic_history(points: &[(u64, u64)]) -> Option<TrafficHistorySummary> {
    let latest = points.last()?;
    let peak_download = points
        .iter()
        .map(|(download, _)| *download)
        .max()
        .unwrap_or(0);
    let peak_upload = points.iter().map(|(_, upload)| *upload).max().unwrap_or(0);

    Some(TrafficHistorySummary {
        samples: points.len(),
        peak_download,
        peak_upload,
        latest_download: latest.0,
        latest_upload: latest.1,
    })
}
