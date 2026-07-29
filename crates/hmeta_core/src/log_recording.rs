use hmeta_model::{HMetaError, LogEntry};
use std::collections::VecDeque;
use std::fs;
use std::io::Write;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing_appender::rolling::{RollingFileAppender, Rotation};

pub(crate) const MAX_IN_MEMORY_LOGS: usize = 256;
const MAX_PENDING_RUNTIME_LOGS: usize = 4096;
const LOG_DIRECTORY: &str = "logs";
const RECORDING_MARKER: &str = ".recording";
const LOG_FILE_PREFIX: &str = "paws.";
const LEGACY_LOG_FILE_PREFIX: &str = "paws-";
const LOG_FILE_SUFFIX: &str = ".log";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogArchiveSummary {
    pub file_name: String,
    pub date: String,
    pub bytes: u64,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogRecordingStatus {
    pub enabled: bool,
    pub archives: Vec<LogArchiveSummary>,
}

pub(crate) struct RecordedLogBuffer {
    root: PathBuf,
    session_id: Option<String>,
    appender: Option<RollingFileAppender>,
    entries: Vec<LogEntry>,
}

impl RecordedLogBuffer {
    pub(crate) fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            session_id: None,
            appender: None,
            entries: Vec::with_capacity(MAX_IN_MEMORY_LOGS),
        }
    }

    pub(crate) fn push(&mut self, entry: LogEntry) {
        self.sync_session();
        if self.session_id.is_none() {
            return;
        }
        if let Some(appender) = self.appender.as_mut() {
            let _ = write_log_entry(appender, &entry);
        }
        if self.entries.len() >= MAX_IN_MEMORY_LOGS {
            self.entries.remove(0);
        }
        self.entries.push(entry);
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }

    pub(crate) fn sync_session(&mut self) {
        let session_id = active_session_id(&self.root);
        if session_id != self.session_id {
            self.entries.clear();
            self.appender = session_id
                .as_ref()
                .and_then(|_| build_daily_appender(&self.root).ok());
            self.session_id = session_id;
        }
    }

    pub(crate) fn enabled(&self) -> bool {
        self.session_id.is_some()
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }
}

impl Deref for RecordedLogBuffer {
    type Target = [LogEntry];

    fn deref(&self) -> &Self::Target {
        &self.entries
    }
}

struct CapturedRuntimeLog {
    sequence: u64,
    captured_at: u128,
    entry: LogEntry,
}

pub(crate) struct RuntimeLogBuffer {
    session_id: Option<String>,
    appender: Option<RollingFileAppender>,
    entries: VecDeque<CapturedRuntimeLog>,
    next_sequence: u64,
    persisted_sequence: u64,
}

impl Default for RuntimeLogBuffer {
    fn default() -> Self {
        Self {
            session_id: None,
            appender: None,
            entries: VecDeque::with_capacity(MAX_IN_MEMORY_LOGS),
            next_sequence: 1,
            persisted_sequence: 0,
        }
    }
}

impl RuntimeLogBuffer {
    pub(crate) fn capture(&mut self, entry: LogEntry) {
        if self.entries.len() >= MAX_PENDING_RUNTIME_LOGS {
            self.entries.pop_front();
        }
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.entries.push_back(CapturedRuntimeLog {
            sequence,
            captured_at: now_unix_nanos(),
            entry,
        });
    }

    pub(crate) fn sync(&mut self, root: &Path) {
        let session_id = active_session_id(root);
        if session_id != self.session_id {
            let started_at = session_id
                .as_deref()
                .and_then(|value| value.parse::<u128>().ok())
                .unwrap_or(u128::MAX);
            self.entries
                .retain(|captured| captured.captured_at >= started_at);
            self.persisted_sequence = 0;
            self.appender = session_id
                .as_ref()
                .and_then(|_| build_daily_appender(root).ok());
            self.session_id = session_id;
        }
        if self.session_id.is_none() {
            self.entries.clear();
            self.persisted_sequence = 0;
            self.appender = None;
            return;
        }

        let mut persisted_sequence = self.persisted_sequence;
        let Some(appender) = self.appender.as_mut() else {
            return;
        };
        for captured in self
            .entries
            .iter()
            .filter(|captured| captured.sequence > self.persisted_sequence)
        {
            if write_log_entry(appender, &captured.entry).is_ok() {
                persisted_sequence = captured.sequence;
            } else {
                break;
            }
        }
        self.persisted_sequence = persisted_sequence;
        while self.entries.len() > MAX_IN_MEMORY_LOGS
            && self
                .entries
                .front()
                .is_some_and(|captured| captured.sequence <= self.persisted_sequence)
        {
            self.entries.pop_front();
        }
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.persisted_sequence = 0;
    }

    pub(crate) fn entries(&self) -> impl Iterator<Item = &LogEntry> {
        self.entries.iter().map(|captured| &captured.entry)
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

pub(crate) fn reset_recording(root: &Path) -> Result<(), HMetaError> {
    let marker = recording_marker(root);
    if marker.exists() {
        fs::remove_file(marker).map_err(io_error)?;
    }
    Ok(())
}

pub(crate) fn set_recording_enabled(root: &Path, enabled: bool) -> Result<(), HMetaError> {
    let directory = log_directory(root);
    fs::create_dir_all(&directory).map_err(io_error)?;
    let marker = recording_marker(root);
    if enabled {
        build_daily_appender(root)?;
        let session_id = now_unix_nanos().to_string();
        let temporary_marker = directory.join(format!(
            "{RECORDING_MARKER}.tmp-{}-{}",
            std::process::id(),
            now_unix_nanos()
        ));
        fs::write(&temporary_marker, session_id).map_err(io_error)?;
        if let Err(error) = fs::rename(&temporary_marker, &marker) {
            let _ = fs::remove_file(temporary_marker);
            return Err(io_error(error));
        }
    } else if marker.exists() {
        fs::remove_file(marker).map_err(io_error)?;
    }
    Ok(())
}

pub(crate) fn recording_status(root: &Path) -> Result<LogRecordingStatus, HMetaError> {
    Ok(LogRecordingStatus {
        enabled: active_session_id(root).is_some(),
        archives: list_archives(root)?,
    })
}

pub(crate) fn read_archive(root: &Path, file_name: &str) -> Result<String, HMetaError> {
    if !is_log_file_name(file_name) {
        return Err(HMetaError::Core("invalid log archive name".to_owned()));
    }
    fs::read_to_string(log_directory(root).join(file_name)).map_err(io_error)
}

fn list_archives(root: &Path) -> Result<Vec<LogArchiveSummary>, HMetaError> {
    let directory = log_directory(root);
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let mut archives = Vec::new();
    for entry in fs::read_dir(directory).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        let file_name = entry.file_name().to_string_lossy().into_owned();
        if !is_log_file_name(&file_name) {
            continue;
        }
        let metadata = entry.metadata().map_err(io_error)?;
        if !metadata.is_file() {
            continue;
        }
        archives.push(LogArchiveSummary {
            date: log_file_date(&file_name).unwrap_or_default().to_owned(),
            file_name,
            bytes: metadata.len(),
            updated_at: metadata.modified().ok().and_then(system_time_secs),
        });
    }
    archives.sort_by(|left, right| {
        right
            .date
            .cmp(&left.date)
            .then_with(|| right.file_name.cmp(&left.file_name))
    });
    Ok(archives)
}

fn build_daily_appender(root: &Path) -> Result<RollingFileAppender, HMetaError> {
    RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("paws")
        .filename_suffix("log")
        .build(log_directory(root))
        .map_err(|error| {
            HMetaError::Core(format!("failed to initialize daily log appender: {error}"))
        })
}

fn write_log_entry(appender: &mut RollingFileAppender, entry: &LogEntry) -> Result<(), HMetaError> {
    let level = entry.level.to_ascii_uppercase();
    let message = entry
        .message
        .replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace(['\r', '\n'], "\\n");
    let line = format!("{}\t{level}\t{message}\n", entry.timestamp);
    appender.write_all(line.as_bytes()).map_err(io_error)
}

fn active_session_id(root: &Path) -> Option<String> {
    fs::read_to_string(recording_marker(root))
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn log_directory(root: &Path) -> PathBuf {
    root.join(LOG_DIRECTORY)
}

fn recording_marker(root: &Path) -> PathBuf {
    log_directory(root).join(RECORDING_MARKER)
}

fn is_log_file_name(file_name: &str) -> bool {
    let Some(date) = log_file_date(file_name) else {
        return false;
    };
    date.len() == 10
        && date.chars().enumerate().all(|(index, value)| match index {
            4 | 7 => value == '-',
            _ => value.is_ascii_digit(),
        })
}

fn log_file_date(file_name: &str) -> Option<&str> {
    [LOG_FILE_PREFIX, LEGACY_LOG_FILE_PREFIX]
        .into_iter()
        .find_map(|prefix| {
            file_name
                .strip_prefix(prefix)
                .and_then(|value| value.strip_suffix(LOG_FILE_SUFFIX))
        })
}

#[cfg(test)]
fn now_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn now_unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0)
}

fn system_time_secs(time: SystemTime) -> Option<String> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs().to_string())
}

fn io_error(error: std::io::Error) -> HMetaError {
    HMetaError::Core(format!("log storage operation failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("hmeta-log-recording-{label}-{}", now_unix_nanos()))
    }

    #[test]
    fn recording_is_disabled_until_explicitly_enabled() {
        let root = temp_root("disabled");
        let mut logs = RecordedLogBuffer::new(&root);
        logs.push(LogEntry {
            level: "info".to_owned(),
            message: "before enable".to_owned(),
            timestamp: "0".to_owned(),
        });
        assert!(logs.is_empty());
        assert!(!recording_status(&root).unwrap().enabled);
        assert!(recording_status(&root).unwrap().archives.is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn enabled_recording_uses_the_community_daily_appender() {
        let root = temp_root("daily");
        set_recording_enabled(&root, true).unwrap();
        let mut logs = RecordedLogBuffer::new(&root);
        logs.push(LogEntry {
            level: "info".to_owned(),
            message: "first event".to_owned(),
            timestamp: "0".to_owned(),
        });
        logs.push(LogEntry {
            level: "warning".to_owned(),
            message: "second event".to_owned(),
            timestamp: "1".to_owned(),
        });

        let status = recording_status(&root).unwrap();
        assert!(status.enabled);
        assert_eq!(status.archives.len(), 1);
        assert!(status.archives[0].file_name.starts_with(LOG_FILE_PREFIX));
        let content = read_archive(&root, &status.archives[0].file_name).unwrap();
        assert!(content.contains("first event"));
        assert!(content.contains("second event"));
        set_recording_enabled(&root, false).unwrap();
        logs.push(LogEntry {
            level: "error".to_owned(),
            message: "after disable".to_owned(),
            timestamp: "2".to_owned(),
        });
        let status = recording_status(&root).unwrap();
        assert!(!status.enabled);
        assert!(status.archives.iter().all(|archive| {
            !read_archive(&root, &archive.file_name)
                .unwrap()
                .contains("after disable")
        }));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn archive_names_cannot_escape_the_log_directory() {
        let root = temp_root("safe-name");
        assert!(read_archive(&root, "../paws-2026-01-01.log").is_err());
        assert!(read_archive(&root, "other.log").is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn runtime_file_persists_bursts_beyond_the_ui_history_limit() {
        let root = temp_root("runtime-burst");
        set_recording_enabled(&root, true).unwrap();
        let mut logs = RuntimeLogBuffer::default();
        for index in 0..300 {
            logs.capture(LogEntry {
                level: "debug".to_owned(),
                message: format!("runtime event {index}"),
                timestamp: now_unix_seconds().to_string(),
            });
        }
        logs.sync(&root);

        assert_eq!(logs.len(), MAX_IN_MEMORY_LOGS);
        let status = recording_status(&root).unwrap();
        assert_eq!(status.archives.len(), 1);
        let content = read_archive(&root, &status.archives[0].file_name).unwrap();
        assert_eq!(content.lines().count(), 300);
        let _ = fs::remove_dir_all(root);
    }
}
