use paws_model::{LogEntry, PawsError};
use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing_appender::rolling::{RollingFileAppender, Rotation};

pub(crate) const MAX_IN_MEMORY_LOGS: usize = 256;
const MAX_PENDING_RUNTIME_LOGS: usize = 4096;
const LOG_DIRECTORY: &str = "logs";
const RECORDING_MARKER: &str = ".recording";
const NATIVE_RECORDING_MARKER: &str = ".recording-native.json";
const RECORDING_CONTROL: &str = ".recording-control";
const RECORDING_LOCK: &str = ".recording.lock";
const NATIVE_TRANSACTION: &str = ".recording-native-transaction.json";
const MAX_NATIVE_PAGE_ENTRIES: usize = 4096;
const MAX_NATIVE_PAGE_BYTES: usize = 4 * 1024 * 1024;
const MAX_TRANSACTION_BYTES: u64 = 16 * 1024 * 1024;
const LOG_FILE_PREFIX: &str = "paws.";
const LEGACY_LOG_FILE_PREFIX: &str = "paws-";
const LOG_FILE_SUFFIX: &str = ".log";

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct NativeLogSource {
    pub attempt: String,
    pub epoch: String,
    pub config_revision: u64,
}

impl NativeLogSource {
    fn same_epoch(&self, other: &Self) -> bool {
        self.attempt == other.attempt && self.epoch == other.epoch
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct NativeLogBoundary {
    pub nonce: String,
    pub source: NativeLogSource,
    pub cursor: u64,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct RecordingSession {
    version: u8,
    started_at: String,
    nonce: String,
    native: Option<NativeLogBoundary>,
    #[serde(default)]
    native_started_at: Option<u128>,
}

impl RecordingSession {
    fn token(&self) -> String {
        format!("{}:{}", self.started_at, self.nonce)
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct NativeTransaction {
    boundary: NativeLogBoundary,
    next_cursor: u64,
    archive: String,
    offset: u64,
    suffix: String,
}

/// Owns the archive lock. No operation under this lock calls back into Core.
/// All processes use this stable inode, including ordinary runtime log writers.
struct RecordingStore {
    root: PathBuf,
    _lock: File,
}

impl RecordingStore {
    fn open(root: &Path) -> Result<Self, PawsError> {
        fs::create_dir_all(log_directory(root)).map_err(io_error)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(log_directory(root).join(RECORDING_LOCK))
            .map_err(io_error)?;
        lock.lock().map_err(io_error)?;
        let store = Self {
            root: root.to_path_buf(),
            _lock: lock,
        };
        store.recover_transaction()?;
        Ok(store)
    }

    fn read_optional(&self, name: &str) -> Result<Option<String>, PawsError> {
        match fs::read_to_string(log_directory(&self.root).join(name)) {
            Ok(value) => Ok(Some(value)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(io_error(error)),
        }
    }

    fn marker(&self) -> Result<Option<String>, PawsError> {
        self.read_optional(RECORDING_MARKER)?
            .map(|value| {
                let value = value.trim();
                if value.is_empty() {
                    Err(PawsError::Core(
                        "log recording marker is empty; refusing to treat its state as disabled"
                            .to_owned(),
                    ))
                } else {
                    Ok(value.to_owned())
                }
            })
            .transpose()
    }

    fn session(&self) -> Result<Option<RecordingSession>, PawsError> {
        let Some(marker) = self.marker()? else {
            return Ok(None);
        };
        let Some(value) = self.read_optional(NATIVE_RECORDING_MARKER)? else {
            return Ok(None);
        };
        let session: RecordingSession = serde_json::from_str(&value).map_err(metadata_error)?;
        if session.version != 1 || session.nonce.is_empty() {
            return Err(PawsError::Core(
                "invalid log recording session metadata".to_owned(),
            ));
        }
        // A legacy marker, or an interrupted enable before marker publication,
        // must never inherit the previous native source/cursor.
        Ok((session.started_at == marker).then_some(session))
    }

    fn active_session(&self) -> Result<Option<String>, PawsError> {
        match self.session()? {
            Some(session) => Ok(Some(session.token())),
            None => self.marker(),
        }
    }

    fn control_token(&self) -> Result<String, PawsError> {
        if let Some(value) = self.read_optional(RECORDING_CONTROL)? {
            let value = value.trim();
            if value.is_empty() {
                return Err(PawsError::Core(
                    "empty log recording control token".to_owned(),
                ));
            }
            return Ok(value.to_owned());
        }
        self.rotate_control()
    }

    fn rotate_control(&self) -> Result<String, PawsError> {
        let token = random_nonce()?;
        self.atomic_write(RECORDING_CONTROL, token.as_bytes())?;
        Ok(token)
    }

    fn atomic_write(&self, name: &str, bytes: &[u8]) -> Result<(), PawsError> {
        let directory = log_directory(&self.root);
        let temporary = directory.join(format!(
            "{name}.tmp-{}-{}",
            std::process::id(),
            random_nonce()?
        ));
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(io_error)?;
            file.write_all(bytes).map_err(io_error)?;
            file.sync_all().map_err(io_error)?;
            fs::rename(&temporary, directory.join(name)).map_err(io_error)?;
            self.sync_directory()
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    fn sync_directory(&self) -> Result<(), PawsError> {
        File::open(log_directory(&self.root))
            .map_err(io_error)?
            .sync_all()
            .map_err(io_error)
    }

    fn write_session(&self, session: &RecordingSession) -> Result<(), PawsError> {
        self.atomic_write(
            NATIVE_RECORDING_MARKER,
            &serde_json::to_vec(session).map_err(metadata_error)?,
        )
    }

    fn enable(
        &self,
        source: Option<(&NativeLogSource, u64)>,
    ) -> Result<RecordingSession, PawsError> {
        build_daily_appender(&self.root)?;
        let mut session = RecordingSession {
            version: 1,
            started_at: now_unix_nanos().to_string(),
            nonce: random_nonce()?,
            native: None,
            native_started_at: None,
        };
        if let Some((source, cursor)) = source {
            session.native = Some(NativeLogBoundary {
                nonce: session.token(),
                source: source.clone(),
                cursor,
            });
        }
        self.rotate_control()?;
        self.write_session(&session)?;
        self.atomic_write(RECORDING_MARKER, session.started_at.as_bytes())?;
        Ok(session)
    }

    fn disable(&self) -> Result<(), PawsError> {
        self.rotate_control()?;
        match fs::remove_file(recording_marker(&self.root)) {
            Ok(()) => self.sync_directory(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(io_error(error)),
        }
    }

    fn ensure_source(
        &self,
        source: &NativeLogSource,
        started_at: u128,
    ) -> Result<Option<NativeLogBoundary>, PawsError> {
        let Some(marker) = self.marker()? else {
            return Ok(None);
        };
        let mut session = self.session()?.unwrap_or(RecordingSession {
            version: 1,
            started_at: marker.clone(),
            nonce: random_nonce()?,
            native: None,
            native_started_at: None,
        });
        if let Some(boundary) = session
            .native
            .as_ref()
            .filter(|boundary| boundary.source.same_epoch(source))
        {
            return Ok(Some(boundary.clone()));
        }
        if session.native.is_some() && session.native_started_at.is_none() {
            // Explicit enable/legacy attachment cannot manufacture a native
            // start time. Replacement must use the caller's captured source CAS.
            return Ok(None);
        }
        if session
            .native_started_at
            .is_some_and(|current| started_at <= current)
        {
            return Ok(None);
        }
        let marker_start = marker
            .parse::<u128>()
            .map_err(|_| PawsError::Core("invalid log recording start timestamp".to_owned()))?;
        if marker_start > started_at {
            return Ok(None);
        }
        let boundary = NativeLogBoundary {
            nonce: session.token(),
            source: source.clone(),
            cursor: 0,
        };
        session.native = Some(boundary.clone());
        session.native_started_at = Some(started_at);
        self.write_session(&session)?;
        Ok(Some(boundary))
    }

    fn attach_source(
        &self,
        expected: &str,
        expected_source: Option<&NativeLogSource>,
        source: &NativeLogSource,
        cursor: u64,
        actual_source_started_at: Option<u128>,
    ) -> Result<Option<NativeLogBoundary>, PawsError> {
        if self.active_session()?.as_deref() != Some(expected) {
            return Ok(None);
        }
        let Some(marker) = self.marker()? else {
            return Ok(None);
        };
        let mut session = self.session()?.unwrap_or(RecordingSession {
            version: 1,
            started_at: marker,
            nonce: random_nonce()?,
            native: None,
            native_started_at: None,
        });
        let current_source = session.native.as_ref().map(|boundary| &boundary.source);
        let source_matches = match (expected_source, current_source) {
            (None, None) => true,
            (Some(expected), Some(current)) => expected.same_epoch(current),
            _ => false,
        };
        if !source_matches {
            return Ok(None);
        }
        if let Some(boundary) = session
            .native
            .as_ref()
            .filter(|boundary| boundary.source.same_epoch(source))
        {
            return Ok(Some(boundary.clone()));
        }
        if current_source.is_some() && actual_source_started_at.is_none() {
            // A UI enable/legacy attachment has no native publication clock
            // and cannot replace an existing native owner, even with its nonce.
            return Ok(None);
        }
        if actual_source_started_at
            .zip(session.native_started_at)
            .is_some_and(|(incoming, current)| incoming <= current)
        {
            return Ok(None);
        }
        let boundary = NativeLogBoundary {
            nonce: session.token(),
            source: source.clone(),
            cursor,
        };
        session.native = Some(boundary.clone());
        session.native_started_at = actual_source_started_at;
        self.write_session(&session)?;
        Ok(Some(boundary))
    }

    fn append_native(
        &self,
        expected: &NativeLogBoundary,
        entries: &[(u64, LogEntry)],
        next_cursor: u64,
        dropped_before: Option<u64>,
    ) -> Result<bool, PawsError> {
        let Some(mut session) = self.session()? else {
            return Ok(false);
        };
        let Some(current) = session.native.as_ref() else {
            return Ok(false);
        };
        if expected.nonce != session.token()
            || current.nonce != expected.nonce
            || current.source != expected.source
        {
            return Ok(false);
        }
        if next_cursor <= current.cursor {
            return Ok(true);
        }
        if entries.len() > MAX_NATIVE_PAGE_ENTRIES {
            return Err(PawsError::Core(
                "native log page exceeds entry limit".to_owned(),
            ));
        }
        let mut suffix = String::new();
        if let Some(first) =
            dropped_before.filter(|first| *first > current.cursor.saturating_add(1))
        {
            suffix.push_str(&encode_log_entry(&LogEntry {
                level: "warning".to_owned(),
                timestamp: now_unix_nanos().to_string(),
                message: format!(
                    "native log recording gap: sequences {}..{} unavailable",
                    current.cursor.saturating_add(1),
                    first.saturating_sub(1)
                ),
            }));
        }
        let mut previous = None;
        for (sequence, entry) in entries {
            if *sequence > next_cursor || previous.is_some_and(|value| *sequence <= value) {
                return Err(PawsError::Core(
                    "invalid native log page sequence".to_owned(),
                ));
            }
            previous = Some(*sequence);
            if *sequence > current.cursor {
                suffix.push_str(&encode_log_entry(entry));
                if suffix.len() > MAX_NATIVE_PAGE_BYTES {
                    return Err(PawsError::Core(
                        "native log page exceeds byte limit".to_owned(),
                    ));
                }
            }
        }
        let archive = current_archive_file_name();
        let path = log_directory(&self.root).join(&archive);
        let mut file = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(&path)
            .map_err(io_error)?;
        let offset = file.metadata().map_err(io_error)?.len();
        let transaction = NativeTransaction {
            boundary: current.clone(),
            next_cursor,
            archive,
            offset,
            suffix,
        };
        self.atomic_write(
            NATIVE_TRANSACTION,
            &serde_json::to_vec(&transaction).map_err(metadata_error)?,
        )?;
        file.write_all(transaction.suffix.as_bytes())
            .map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        session
            .native
            .as_mut()
            .expect("checked native boundary")
            .cursor = next_cursor;
        self.write_session(&session)?;
        fs::remove_file(log_directory(&self.root).join(NATIVE_TRANSACTION)).map_err(io_error)?;
        self.sync_directory()?;
        Ok(true)
    }

    fn recover_transaction(&self) -> Result<(), PawsError> {
        let path = log_directory(&self.root).join(NATIVE_TRANSACTION);
        let mut journal = match File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(io_error(error)),
        };
        if journal.metadata().map_err(io_error)?.len() > MAX_TRANSACTION_BYTES {
            return Err(PawsError::Core(
                "native log transaction exceeds recovery limit".to_owned(),
            ));
        }
        let mut data = Vec::new();
        journal.read_to_end(&mut data).map_err(io_error)?;
        let transaction: NativeTransaction =
            serde_json::from_slice(&data).map_err(metadata_error)?;
        if !is_log_file_name(&transaction.archive)
            || transaction.suffix.len() > MAX_NATIVE_PAGE_BYTES
        {
            return Err(PawsError::Core("invalid native log transaction".to_owned()));
        }
        let committed = self
            .session()?
            .and_then(|session| session.native)
            .is_some_and(|boundary| {
                boundary.nonce == transaction.boundary.nonce
                    && boundary.source == transaction.boundary.source
                    && boundary.cursor >= transaction.next_cursor
            });
        if !committed {
            let mut archive = OpenOptions::new()
                .read(true)
                .write(true)
                .open(log_directory(&self.root).join(&transaction.archive))
                .map_err(io_error)?;
            let length = archive.metadata().map_err(io_error)?.len();
            let tail_len = length
                .checked_sub(transaction.offset)
                .filter(|tail| *tail <= transaction.suffix.len() as u64)
                .ok_or_else(|| {
                    PawsError::Core(
                        "native log transaction archive changed during recovery".to_owned(),
                    )
                })?;
            let mut tail = vec![0; tail_len as usize];
            archive
                .seek(SeekFrom::Start(transaction.offset))
                .map_err(io_error)?;
            archive.read_exact(&mut tail).map_err(io_error)?;
            if !transaction.suffix.as_bytes().starts_with(&tail) {
                return Err(PawsError::Core(
                    "native log transaction suffix changed during recovery".to_owned(),
                ));
            }
            archive.set_len(transaction.offset).map_err(io_error)?;
            archive.sync_all().map_err(io_error)?;
        }
        fs::remove_file(path).map_err(io_error)?;
        self.sync_directory()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogArchiveSummary {
    pub file_name: String,
    pub date: String,
    pub bytes: u64,
    pub updated_at: Option<String>,
    pub active: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogRecordingStatus {
    pub enabled: bool,
    pub archives: Vec<LogArchiveSummary>,
    pub last_error: Option<String>,
}

pub(crate) struct RecordedLogBuffer {
    root: PathBuf,
    session_id: Option<String>,
    appender: Option<RollingFileAppender>,
    entries: Vec<LogEntry>,
    last_error: Option<String>,
    last_error_source: Option<LogRecordingErrorSource>,
    #[cfg(test)]
    fail_next_write: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LogRecordingErrorSource {
    Control,
    Session,
    Writer,
}

impl RecordedLogBuffer {
    pub(crate) fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            session_id: None,
            appender: None,
            entries: Vec::with_capacity(MAX_IN_MEMORY_LOGS),
            last_error: None,
            last_error_source: None,
            #[cfg(test)]
            fail_next_write: false,
        }
    }

    pub(crate) fn push(&mut self, entry: LogEntry) {
        self.sync_session();
        if self.session_id.is_none() {
            return;
        }
        if let Some(appender) = self.appender.as_mut() {
            let write_result = (|| {
                let store = RecordingStore::open(&self.root)?;
                if store.active_session()? != self.session_id {
                    return Ok(());
                }
                #[cfg(test)]
                if std::mem::take(&mut self.fail_next_write) {
                    return Err(PawsError::Core(
                        "injected recorded log write failure".to_owned(),
                    ));
                }
                write_log_entry(appender, &entry)
            })();
            self.record_writer_result(write_result);
        }
        if self.entries.len() >= MAX_IN_MEMORY_LOGS {
            self.entries.remove(0);
        }
        self.entries.push(entry);
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }

    /// Archive-only consumer: independent from the UI journal/cursor.
    pub(crate) fn append_native(
        &mut self,
        boundary: &NativeLogBoundary,
        entries: &[(u64, LogEntry)],
        next_cursor: u64,
        dropped_before: Option<u64>,
    ) -> Result<bool, PawsError> {
        RecordingStore::open(&self.root)?.append_native(
            boundary,
            entries,
            next_cursor,
            dropped_before,
        )
    }

    pub(crate) fn sync_session(&mut self) {
        let store = match RecordingStore::open(&self.root) {
            Ok(store) => store,
            Err(error) => {
                self.set_error(LogRecordingErrorSource::Session, error);
                return;
            }
        };
        let session_id = match store.active_session() {
            Ok(session_id) => session_id,
            Err(error) => {
                self.set_error(LogRecordingErrorSource::Session, error);
                return;
            }
        };
        self.clear_error_from(LogRecordingErrorSource::Session);
        if session_id != self.session_id {
            self.entries.clear();
            self.appender = None;
            self.session_id = session_id;
            self.clear_error_from(LogRecordingErrorSource::Writer);
        }
        if self.session_id.is_some() && self.appender.is_none() {
            match build_daily_appender(&self.root) {
                Ok(appender) => {
                    self.appender = Some(appender);
                    self.clear_error_from(LogRecordingErrorSource::Writer);
                }
                Err(error) => self.set_error(LogRecordingErrorSource::Writer, error),
            }
        }
    }

    pub(crate) fn enabled(&self) -> bool {
        self.session_id.is_some()
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub(crate) fn record_control_error(&mut self, error: &PawsError) {
        self.set_error(LogRecordingErrorSource::Control, error.to_string());
    }

    pub(crate) fn clear_control_error(&mut self) {
        self.clear_error_from(LogRecordingErrorSource::Control);
    }

    fn record_writer_result(&mut self, result: Result<(), PawsError>) {
        match result {
            Ok(()) => self.clear_error_from(LogRecordingErrorSource::Writer),
            Err(error) => self.set_error(LogRecordingErrorSource::Writer, error),
        }
    }

    fn set_error(&mut self, source: LogRecordingErrorSource, error: impl ToString) {
        self.last_error = Some(error.to_string());
        self.last_error_source = Some(source);
    }

    fn clear_error_from(&mut self, source: LogRecordingErrorSource) {
        if self.last_error_source == Some(source) {
            self.last_error = None;
            self.last_error_source = None;
        }
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
    last_error: Option<String>,
    #[cfg(test)]
    fail_next_write: bool,
}

impl Default for RuntimeLogBuffer {
    fn default() -> Self {
        Self {
            session_id: None,
            appender: None,
            entries: VecDeque::with_capacity(MAX_IN_MEMORY_LOGS),
            next_sequence: 1,
            persisted_sequence: 0,
            last_error: None,
            #[cfg(test)]
            fail_next_write: false,
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
        let store = match RecordingStore::open(root) {
            Ok(store) => store,
            Err(error) => {
                self.last_error = Some(error.to_string());
                return;
            }
        };
        let session_id = match store.active_session() {
            Ok(session_id) => session_id,
            Err(error) => {
                self.last_error = Some(error.to_string());
                return;
            }
        };
        if session_id != self.session_id {
            let started_at = session_id
                .as_deref()
                .and_then(|value| value.split(':').next()?.parse::<u128>().ok())
                .unwrap_or(u128::MAX);
            self.entries
                .retain(|captured| captured.captured_at >= started_at);
            self.persisted_sequence = 0;
            self.appender = None;
            self.session_id = session_id;
            self.last_error = None;
        }
        if self.session_id.is_none() {
            self.entries.clear();
            self.persisted_sequence = 0;
            self.appender = None;
            self.last_error = None;
            return;
        }

        if self.appender.is_none() {
            match build_daily_appender(root) {
                Ok(appender) => {
                    self.appender = Some(appender);
                    self.last_error = None;
                }
                Err(error) => self.last_error = Some(error.to_string()),
            }
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
            #[cfg(test)]
            let write_result = if std::mem::take(&mut self.fail_next_write) {
                Err(PawsError::Core(
                    "injected runtime log write failure".to_owned(),
                ))
            } else {
                write_log_entry(appender, &captured.entry)
            };
            #[cfg(not(test))]
            let write_result = write_log_entry(appender, &captured.entry);
            match write_result {
                Ok(()) => {
                    persisted_sequence = captured.sequence;
                    self.last_error = None;
                }
                Err(error) => {
                    self.last_error = Some(error.to_string());
                    break;
                }
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

    pub(crate) fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }
}

pub(crate) fn set_recording_enabled(root: &Path, enabled: bool) -> Result<(), PawsError> {
    let store = RecordingStore::open(root)?;
    if enabled {
        store.enable(None)?;
    } else {
        store.disable()?;
    }
    Ok(())
}

pub(crate) fn active_recording_session(root: &Path) -> Result<Option<String>, PawsError> {
    RecordingStore::open(root)?.active_session()
}

pub(crate) fn recording_control_token(root: &Path) -> Result<String, PawsError> {
    RecordingStore::open(root)?.control_token()
}

#[cfg(test)]
pub(crate) fn enable_native_recording(
    root: &Path,
    source: &NativeLogSource,
    high_watermark: u64,
) -> Result<NativeLogBoundary, PawsError> {
    RecordingStore::open(root)?
        .enable(Some((source, high_watermark)))?
        .native
        .ok_or_else(|| PawsError::Core("native recording boundary was not initialized".to_owned()))
}

pub(crate) fn enable_native_recording_if_current(
    root: &Path,
    expected_control_token: &str,
    source: &NativeLogSource,
    high_watermark: u64,
) -> Result<Option<NativeLogBoundary>, PawsError> {
    let store = RecordingStore::open(root)?;
    if store.control_token()? != expected_control_token {
        return Ok(None);
    }
    Ok(store.enable(Some((source, high_watermark)))?.native)
}

pub(crate) fn ensure_native_recording_source(
    root: &Path,
    source: &NativeLogSource,
    source_started_at_unix_nanos: u128,
) -> Result<Option<NativeLogBoundary>, PawsError> {
    RecordingStore::open(root)?.ensure_source(source, source_started_at_unix_nanos)
}

pub(crate) fn attach_native_recording_source(
    root: &Path,
    expected_nonce: &str,
    expected_source: Option<&NativeLogSource>,
    source: &NativeLogSource,
    high_watermark: u64,
    actual_source_started_at: Option<u128>,
) -> Result<Option<NativeLogBoundary>, PawsError> {
    RecordingStore::open(root)?.attach_source(
        expected_nonce,
        expected_source,
        source,
        high_watermark,
        actual_source_started_at,
    )
}

pub(crate) fn native_recording_boundary(
    root: &Path,
) -> Result<Option<NativeLogBoundary>, PawsError> {
    Ok(RecordingStore::open(root)?
        .session()?
        .and_then(|session| session.native))
}

pub(crate) fn recording_status(root: &Path) -> Result<LogRecordingStatus, PawsError> {
    let store = RecordingStore::open(root)?;
    let enabled = store.active_session()?.is_some();
    let active_file_name = enabled.then(current_archive_file_name);
    let mut archives = list_archives(root)?;
    for archive in &mut archives {
        archive.active = active_file_name.as_deref() == Some(archive.file_name.as_str());
    }
    Ok(LogRecordingStatus {
        enabled,
        archives,
        last_error: None,
    })
}

pub(crate) fn read_archive(root: &Path, file_name: &str) -> Result<String, PawsError> {
    if !is_log_file_name(file_name) {
        return Err(PawsError::Core("invalid log archive name".to_owned()));
    }
    let _store = RecordingStore::open(root)?;
    fs::read_to_string(log_directory(root).join(file_name)).map_err(io_error)
}

pub(crate) fn delete_archive(root: &Path, file_name: &str) -> Result<(), PawsError> {
    if !is_log_file_name(file_name) {
        return Err(PawsError::Core("invalid log archive name".to_owned()));
    }
    let store = RecordingStore::open(root)?;
    if store.active_session()?.is_some() && file_name == current_archive_file_name() {
        return Err(PawsError::Core(
            "stop log recording before deleting the active archive".to_owned(),
        ));
    }
    fs::remove_file(log_directory(root).join(file_name)).map_err(io_error)
}

fn list_archives(root: &Path) -> Result<Vec<LogArchiveSummary>, PawsError> {
    let directory = log_directory(root);
    if !directory.try_exists().map_err(io_error)? {
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
            active: false,
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

fn build_daily_appender(root: &Path) -> Result<RollingFileAppender, PawsError> {
    RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("paws")
        .filename_suffix("log")
        .build(log_directory(root))
        .map_err(|error| {
            PawsError::Core(format!("failed to initialize daily log appender: {error}"))
        })
}

fn write_log_entry(appender: &mut RollingFileAppender, entry: &LogEntry) -> Result<(), PawsError> {
    appender
        .write_all(encode_log_entry(entry).as_bytes())
        .map_err(io_error)
}

fn encode_log_entry(entry: &LogEntry) -> String {
    let level = entry.level.to_ascii_uppercase();
    let message = entry
        .message
        .replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace(['\r', '\n'], "\\n");
    format!("{}\t{level}\t{message}\n", entry.timestamp)
}

fn random_nonce() -> Result<String, PawsError> {
    let mut bytes = [0_u8; 16];
    getrandom::getrandom(&mut bytes)
        .map_err(|error| PawsError::Core(format!("log recording nonce unavailable: {error}")))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn metadata_error(error: serde_json::Error) -> PawsError {
    PawsError::Core(format!("invalid log recording metadata: {error}"))
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

fn current_archive_file_name() -> String {
    let date = time::OffsetDateTime::now_utc().date();
    format!(
        "paws.{:04}-{:02}-{:02}.log",
        date.year(),
        date.month() as u8,
        date.day()
    )
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

fn io_error(error: std::io::Error) -> PawsError {
    PawsError::Core(format!("log storage operation failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("paws-log-recording-{label}-{}", now_unix_nanos()))
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
        assert!(delete_archive(&root, "../paws-2026-01-01.log").is_err());
        assert!(delete_archive(&root, "other.log").is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn active_archive_requires_recording_to_stop_before_deletion() {
        let root = temp_root("delete-active");
        set_recording_enabled(&root, true).unwrap();
        let mut logs = RecordedLogBuffer::new(&root);
        logs.push(LogEntry {
            level: "info".to_owned(),
            message: "delete me".to_owned(),
            timestamp: "0".to_owned(),
        });

        let status = recording_status(&root).unwrap();
        assert_eq!(status.archives.len(), 1);
        assert!(status.archives[0].active);
        let file_name = status.archives[0].file_name.clone();
        assert!(delete_archive(&root, &file_name).is_err());

        let old_file_name = "paws.2000-01-01.log";
        fs::write(log_directory(&root).join(old_file_name), "old log").unwrap();
        delete_archive(&root, old_file_name).unwrap();
        assert!(!log_directory(&root).join(old_file_name).exists());

        set_recording_enabled(&root, false).unwrap();
        delete_archive(&root, &file_name).unwrap();
        assert!(recording_status(&root).unwrap().archives.is_empty());
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

    #[test]
    fn recorded_log_write_failure_is_visible_until_that_writer_recovers() {
        let root = temp_root("recorded-write-error");
        set_recording_enabled(&root, true).unwrap();
        let mut logs = RecordedLogBuffer::new(&root);
        logs.fail_next_write = true;
        logs.push(LogEntry {
            level: "error".to_owned(),
            message: "first write fails".to_owned(),
            timestamp: "0".to_owned(),
        });
        assert!(logs
            .last_error()
            .is_some_and(|error| error.contains("injected recorded log write failure")));

        logs.push(LogEntry {
            level: "info".to_owned(),
            message: "writer recovered".to_owned(),
            timestamp: "1".to_owned(),
        });
        assert_eq!(logs.last_error(), None);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn runtime_log_write_failure_is_visible_and_retried() {
        let root = temp_root("runtime-write-error");
        set_recording_enabled(&root, true).unwrap();
        let mut logs = RuntimeLogBuffer::default();
        logs.capture(LogEntry {
            level: "error".to_owned(),
            message: "first runtime write fails".to_owned(),
            timestamp: "0".to_owned(),
        });
        logs.fail_next_write = true;
        logs.sync(&root);
        assert!(logs
            .last_error()
            .is_some_and(|error| error.contains("injected runtime log write failure")));

        logs.sync(&root);
        assert_eq!(logs.last_error(), None);
        assert_eq!(logs.persisted_sequence, 1);
        let _ = fs::remove_dir_all(root);
    }

    fn source(epoch: &str) -> NativeLogSource {
        NativeLogSource {
            attempt: "attempt-a".to_owned(),
            epoch: epoch.to_owned(),
            config_revision: 7,
        }
    }

    fn entry(message: &str) -> LogEntry {
        LogEntry {
            level: "info".to_owned(),
            message: message.to_owned(),
            timestamp: "1".to_owned(),
        }
    }

    fn archive_text(root: &Path) -> String {
        recording_status(root)
            .unwrap()
            .archives
            .iter()
            .map(|archive| read_archive(root, &archive.file_name).unwrap())
            .collect()
    }

    #[test]
    fn native_pages_have_an_independent_persisted_cursor_and_preserve_ui_entries() {
        let root = temp_root("native-cursor");
        let boundary = enable_native_recording(&root, &source("epoch-a"), 10).unwrap();
        let mut first = RecordedLogBuffer::new(&root);
        first.push(entry("ui-entry"));
        assert!(first
            .append_native(
                &boundary,
                &[(10, entry("old")), (11, entry("native-11"))],
                11,
                None
            )
            .unwrap());
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].message, "ui-entry");
        let mut other = RecordedLogBuffer::new(&root);
        assert!(other
            .append_native(
                &boundary,
                &[(11, entry("duplicate")), (12, entry("native-12"))],
                12,
                None
            )
            .unwrap());
        assert!(first
            .append_native(&boundary, &[(11, entry("older-page"))], 11, None)
            .unwrap());
        assert_eq!(
            native_recording_boundary(&root).unwrap().unwrap().cursor,
            12
        );
        let text = archive_text(&root);
        assert_eq!(text.lines().count(), 3);
        assert!(
            !text.contains("duplicate")
                && !text.contains("older-page")
                && !text.contains("\told\n")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn native_archive_outcomes_never_replace_or_clear_another_error_owner() {
        let root = temp_root("archive-error-owner");
        let boundary = enable_native_recording(&root, &source("epoch-a"), 0).unwrap();
        let mut logs = RecordedLogBuffer::new(&root);
        logs.record_control_error(&PawsError::Core("control-failure".into()));
        let control = logs.last_error().unwrap().to_owned();
        assert!(logs
            .append_native(&boundary, &[(2, entry("bad-sequence"))], 1, None)
            .is_err());
        assert_eq!(logs.last_error(), Some(control.as_str()));
        assert!(logs
            .append_native(&boundary, &[(1, entry("native"))], 1, None)
            .unwrap());
        assert_eq!(logs.last_error(), Some(control.as_str()));
        logs.clear_control_error();
        logs.fail_next_write = true;
        logs.push(entry("normal-writer-failure"));
        let writer = logs.last_error().unwrap().to_owned();
        assert!(logs
            .append_native(&boundary, &[(2, entry("next-native"))], 2, None)
            .unwrap());
        assert_eq!(logs.last_error(), Some(writer.as_str()));
        logs.push(entry("normal-writer-recovered"));
        assert!(logs.last_error().is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn native_stop_reenable_and_epoch_rotation_reject_old_pages() {
        let root = temp_root("native-epoch");
        let first = enable_native_recording(&root, &source("epoch-a"), 0).unwrap();
        let mut logs = RecordedLogBuffer::new(&root);
        set_recording_enabled(&root, false).unwrap();
        assert!(!logs
            .append_native(&first, &[(1, entry("after-off"))], 1, None)
            .unwrap());
        let second = enable_native_recording(&root, &source("epoch-a"), 1).unwrap();
        assert_ne!(first.nonce, second.nonce);
        assert!(!logs
            .append_native(&first, &[(2, entry("old-recording"))], 2, None)
            .unwrap());
        let started_at = now_unix_nanos();
        assert!(
            ensure_native_recording_source(&root, &source("epoch-b"), started_at)
                .unwrap()
                .is_none()
        );
        let third = attach_native_recording_source(
            &root,
            &second.nonce,
            Some(&second.source),
            &source("epoch-b"),
            0,
            Some(started_at),
        )
        .unwrap()
        .unwrap();
        assert_eq!(third.cursor, 0);
        assert!(!logs
            .append_native(&second, &[(2, entry("old-endpoint"))], 2, None)
            .unwrap());
        assert!(logs
            .append_native(&third, &[(1, entry("current-endpoint"))], 1, None)
            .unwrap());
        assert_eq!(archive_text(&root).lines().count(), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn unchanged_epoch_preserves_cut_and_published_config_revision() {
        let root = temp_root("unchanged-epoch");
        let boundary = enable_native_recording(&root, &source("epoch-a"), 44).unwrap();
        let changed = NativeLogSource {
            config_revision: 99,
            ..source("epoch-a")
        };
        assert_eq!(
            ensure_native_recording_source(&root, &changed, 0).unwrap(),
            Some(boundary.clone())
        );
        assert_eq!(
            attach_native_recording_source(
                &root,
                &boundary.nonce,
                Some(&boundary.source),
                &changed,
                999,
                None
            )
            .unwrap(),
            Some(boundary)
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn late_recording_needs_real_watermark_and_attach_is_nonce_fenced() {
        let root = temp_root("late-marker");
        let native_started_at = now_unix_nanos();
        set_recording_enabled(&root, true).unwrap();
        assert!(
            ensure_native_recording_source(&root, &source("epoch-a"), native_started_at)
                .unwrap()
                .is_none()
        );
        let nonce = active_recording_session(&root).unwrap().unwrap();
        let boundary =
            attach_native_recording_source(&root, &nonce, None, &source("epoch-a"), 47, None)
                .unwrap()
                .unwrap();
        assert_eq!(boundary.cursor, 47);
        set_recording_enabled(&root, false).unwrap();
        set_recording_enabled(&root, true).unwrap();
        assert!(
            attach_native_recording_source(&root, &nonce, None, &source("epoch-a"), 48, None)
                .unwrap()
                .is_none()
        );
        assert!(native_recording_boundary(&root).unwrap().is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn attach_requires_exact_captured_source_and_never_treats_none_as_wildcard() {
        let root = temp_root("attach-source-cas");
        set_recording_enabled(&root, true).unwrap();
        let nonce = active_recording_session(&root).unwrap().unwrap();
        let first = ensure_native_recording_source(&root, &source("epoch-a"), now_unix_nanos())
            .unwrap()
            .unwrap();
        assert!(
            attach_native_recording_source(&root, &nonce, None, &source("epoch-a"), 100, None)
                .unwrap()
                .is_none()
        );
        assert!(attach_native_recording_source(
            &root,
            &nonce,
            Some(&first.source),
            &source("epoch-b"),
            100,
            None
        )
        .unwrap()
        .is_none());
        let metadata_only = NativeLogSource {
            config_revision: 99,
            ..first.source.clone()
        };
        assert_eq!(
            attach_native_recording_source(
                &root,
                &nonce,
                Some(&metadata_only),
                &metadata_only,
                100,
                None
            )
            .unwrap(),
            Some(first)
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn source_publication_rejects_older_start_and_preserves_the_new_cursor() {
        let root = temp_root("source-order");
        set_recording_enabled(&root, true).unwrap();
        let first_start = now_unix_nanos();
        let first = ensure_native_recording_source(&root, &source("epoch-a"), first_start)
            .unwrap()
            .unwrap();
        let second_start = first_start + 1;
        let second = ensure_native_recording_source(&root, &source("epoch-b"), second_start)
            .unwrap()
            .unwrap();
        let mut logs = RecordedLogBuffer::new(&root);
        assert!(logs
            .append_native(&second, &[(1, entry("current-source"))], 1, None)
            .unwrap());
        assert!(
            ensure_native_recording_source(&root, &first.source, first_start)
                .unwrap()
                .is_none()
        );
        assert!(attach_native_recording_source(
            &root,
            &first.nonce,
            Some(&second.source),
            &first.source,
            100,
            Some(first_start)
        )
        .unwrap()
        .is_none());
        let current = native_recording_boundary(&root).unwrap().unwrap();
        assert_eq!(current.source, second.source);
        assert_eq!(current.cursor, 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn stop_tombstone_blocks_an_enable_waiting_for_native_watermark() {
        let root = temp_root("enable-cas");
        let before = recording_control_token(&root).unwrap();
        set_recording_enabled(&root, false).unwrap();
        let stopped = recording_control_token(&root).unwrap();
        assert_ne!(before, stopped);
        assert!(
            enable_native_recording_if_current(&root, &before, &source("epoch-a"), 10)
                .unwrap()
                .is_none()
        );
        assert!(active_recording_session(&root).unwrap().is_none());
        let boundary = enable_native_recording_if_current(&root, &stopped, &source("epoch-a"), 10)
            .unwrap()
            .unwrap();
        assert_eq!(boundary.cursor, 10);
        assert!(
            enable_native_recording_if_current(&root, &stopped, &source("epoch-a"), 20)
                .unwrap()
                .is_none()
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn native_gaps_are_recorded_once_and_bad_pages_do_not_advance_cursor() {
        let root = temp_root("native-gap");
        let boundary = enable_native_recording(&root, &source("epoch-a"), 2).unwrap();
        let mut logs = RecordedLogBuffer::new(&root);
        assert!(logs
            .append_native(
                &boundary,
                &[(7, entry("seven")), (6, entry("six"))],
                7,
                Some(6)
            )
            .is_err());
        assert_eq!(native_recording_boundary(&root).unwrap().unwrap().cursor, 2);
        assert!(logs
            .append_native(
                &boundary,
                &[(6, entry("six")), (7, entry("seven"))],
                7,
                Some(6)
            )
            .unwrap());
        assert!(logs
            .append_native(
                &boundary,
                &[(6, entry("six")), (7, entry("seven"))],
                7,
                Some(6)
            )
            .unwrap());
        let text = archive_text(&root);
        assert_eq!(text.matches("native log recording gap").count(), 1);
        assert_eq!(text.lines().count(), 3);
        let _ = fs::remove_dir_all(root);
    }

    fn leave_transaction(
        root: &Path,
        boundary: &NativeLogBoundary,
        committed: bool,
        partial: bool,
    ) {
        let store = RecordingStore::open(root).unwrap();
        let path = log_directory(root).join(current_archive_file_name());
        let offset = fs::metadata(&path).unwrap().len();
        let suffix = encode_log_entry(&entry("transaction-entry"));
        let transaction = NativeTransaction {
            boundary: boundary.clone(),
            next_cursor: 1,
            archive: current_archive_file_name(),
            offset,
            suffix: suffix.clone(),
        };
        store
            .atomic_write(
                NATIVE_TRANSACTION,
                &serde_json::to_vec(&transaction).unwrap(),
            )
            .unwrap();
        let mut archive = OpenOptions::new().append(true).open(path).unwrap();
        archive
            .write_all(if partial {
                &suffix.as_bytes()[..5]
            } else {
                suffix.as_bytes()
            })
            .unwrap();
        archive.sync_all().unwrap();
        if committed {
            let mut session = store.session().unwrap().unwrap();
            session.native.as_mut().unwrap().cursor = 1;
            store.write_session(&session).unwrap();
        }
    }

    #[test]
    fn incomplete_native_transaction_is_rolled_back_before_another_writer_and_retry() {
        for partial in [false, true] {
            let root = temp_root("transaction-recovery");
            let boundary = enable_native_recording(&root, &source("epoch-a"), 0).unwrap();
            leave_transaction(&root, &boundary, false, partial);
            let mut logs = RecordedLogBuffer::new(&root);
            logs.push(entry("ordinary-after-crash"));
            assert!(logs
                .append_native(&boundary, &[(1, entry("transaction-entry"))], 1, None)
                .unwrap());
            let text = archive_text(&root);
            assert_eq!(text.lines().count(), 2);
            assert_eq!(text.matches("transaction-entry").count(), 1);
            assert_eq!(native_recording_boundary(&root).unwrap().unwrap().cursor, 1);
            let _ = fs::remove_dir_all(root);
        }
    }

    #[test]
    fn committed_transaction_recovery_does_not_erase_or_duplicate_archive_data() {
        let root = temp_root("committed-recovery");
        let boundary = enable_native_recording(&root, &source("epoch-a"), 0).unwrap();
        leave_transaction(&root, &boundary, true, false);
        let mut logs = RecordedLogBuffer::new(&root);
        assert!(logs
            .append_native(&boundary, &[(1, entry("duplicate"))], 1, None)
            .unwrap());
        assert_eq!(archive_text(&root).lines().count(), 1);
        assert!(!log_directory(&root).join(NATIVE_TRANSACTION).exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn runtime_capture_before_enable_is_excluded_but_post_enable_is_persisted() {
        let root = temp_root("runtime-boundary");
        let mut logs = RuntimeLogBuffer::default();
        logs.capture(entry("before-enable"));
        set_recording_enabled(&root, true).unwrap();
        logs.capture(entry("after-enable"));
        logs.sync(&root);
        let text = archive_text(&root);
        assert!(!text.contains("before-enable"));
        assert!(text.contains("after-enable"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn malformed_marker_and_native_metadata_fail_closed_and_recover_visibly() {
        let root = temp_root("corrupt-metadata");
        set_recording_enabled(&root, true).unwrap();
        let mut logs = RecordedLogBuffer::new(&root);
        logs.sync_session();
        let marker = fs::read(recording_marker(&root)).unwrap();
        fs::write(recording_marker(&root), "").unwrap();
        logs.sync_session();
        assert!(logs.last_error().unwrap().contains("marker is empty"));
        fs::write(recording_marker(&root), marker).unwrap();
        logs.sync_session();
        assert!(logs.last_error().is_none());
        fs::write(
            log_directory(&root).join(NATIVE_RECORDING_MARKER),
            "{broken",
        )
        .unwrap();
        assert!(native_recording_boundary(&root).is_err());
        logs.push(entry("not-written-with-bad-metadata"));
        assert!(logs.last_error().unwrap().contains("metadata"));
        // Explicit stop can remove a corrupt metadata session without accepting
        // its source or writing an archive entry.
        set_recording_enabled(&root, false).unwrap();
        set_recording_enabled(&root, true).unwrap();
        logs.sync_session();
        assert!(logs.last_error().is_none());
        assert!(!archive_text(&root).contains("not-written-with-bad-metadata"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn legacy_timestamp_marker_keeps_runtime_cut_and_can_attach_native_source() {
        let root = temp_root("legacy-marker");
        fs::create_dir_all(log_directory(&root)).unwrap();
        let marker = now_unix_nanos().to_string();
        fs::write(recording_marker(&root), &marker).unwrap();
        assert_eq!(
            active_recording_session(&root).unwrap(),
            Some(marker.clone())
        );
        let boundary =
            attach_native_recording_source(&root, &marker, None, &source("epoch-a"), 21, None)
                .unwrap()
                .unwrap();
        assert_eq!(boundary.cursor, 21);
        assert_eq!(
            active_recording_session(&root).unwrap(),
            Some(boundary.nonce)
        );
        let mut logs = RuntimeLogBuffer::default();
        logs.capture(entry("legacy-runtime"));
        logs.sync(&root);
        assert!(archive_text(&root).contains("legacy-runtime"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn recovery_refuses_to_erase_an_unexpected_archive_suffix() {
        let root = temp_root("recovery-suffix");
        let boundary = enable_native_recording(&root, &source("epoch-a"), 0).unwrap();
        leave_transaction(&root, &boundary, false, true);
        let archive = log_directory(&root).join(current_archive_file_name());
        OpenOptions::new()
            .append(true)
            .open(&archive)
            .unwrap()
            .write_all(b"unrelated")
            .unwrap();
        let before = fs::read(&archive).unwrap();
        assert!(RecordingStore::open(&root).is_err());
        assert_eq!(fs::read(&archive).unwrap(), before);
        assert!(log_directory(&root).join(NATIVE_TRANSACTION).exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn native_page_size_and_sequence_validation_precede_any_archive_mutation() {
        let root = temp_root("page-bounds");
        let boundary = enable_native_recording(&root, &source("epoch-a"), 0).unwrap();
        let mut logs = RecordedLogBuffer::new(&root);
        let oversized = LogEntry {
            message: "x".repeat(MAX_NATIVE_PAGE_BYTES + 1),
            ..entry("")
        };
        assert!(logs
            .append_native(&boundary, &[(1, oversized)], 1, None)
            .is_err());
        assert!(logs
            .append_native(&boundary, &[(2, entry("future"))], 1, None)
            .is_err());
        assert_eq!(native_recording_boundary(&root).unwrap().unwrap().cursor, 0);
        assert!(archive_text(&root).is_empty());
        assert!(logs
            .append_native(
                &boundary,
                &[(1, entry("line\nwith\ttabs\\and\rreturn"))],
                1,
                None
            )
            .unwrap());
        assert_eq!(archive_text(&root).lines().count(), 1);
        assert!(logs.last_error().is_none());
        let _ = fs::remove_dir_all(root);
    }

    fn await_file(path: &Path) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !path.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "child process synchronization timed out"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn worker(root: &Path, action: &str) -> std::process::Child {
        std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "log_recording::tests::cross_process_worker",
                "--nocapture",
            ])
            .env("PAWS_LOG_STORE_TEST_ROOT", root)
            .env("PAWS_LOG_STORE_TEST_ACTION", action)
            .spawn()
            .unwrap()
    }

    #[test]
    fn cross_process_worker() {
        let Some(root) = std::env::var_os("PAWS_LOG_STORE_TEST_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        let action = std::env::var("PAWS_LOG_STORE_TEST_ACTION").unwrap();
        let boundary: NativeLogBoundary =
            serde_json::from_slice(&fs::read(root.join("boundary.json")).unwrap()).unwrap();
        match action.as_str() {
            "duplicate-page" => {
                await_file(&root.join("start"));
                let entries: Vec<_> = (1..=20)
                    .map(|sequence| (sequence, entry(&format!("event-{sequence}"))))
                    .collect();
                assert!(RecordedLogBuffer::new(&root)
                    .append_native(&boundary, &entries, 20, None)
                    .unwrap());
            }
            "hold-append" => {
                let store = RecordingStore::open(&root).unwrap();
                fs::write(root.join("locked"), "").unwrap();
                await_file(&root.join("release"));
                assert!(store
                    .append_native(&boundary, &[(1, entry("before-stop"))], 1, None)
                    .unwrap());
            }
            "stop" => {
                fs::write(root.join("stop-started"), "").unwrap();
                set_recording_enabled(&root, false).unwrap();
                fs::write(root.join("stop-completed"), "").unwrap();
            }
            "late-enable" => {
                await_file(&root.join("start"));
                let token = fs::read_to_string(root.join("control-token")).unwrap();
                assert!(
                    enable_native_recording_if_current(&root, &token, &boundary.source, 100)
                        .unwrap()
                        .is_none()
                );
            }
            "old-source-attach" | "old-source-ensure" => {
                await_file(&root.join("start"));
                let started_at: u128 = fs::read_to_string(root.join("native-started-at"))
                    .unwrap()
                    .parse()
                    .unwrap();
                if action == "old-source-attach" {
                    assert!(attach_native_recording_source(
                        &root,
                        &boundary.nonce,
                        Some(&boundary.source),
                        &boundary.source,
                        999,
                        Some(started_at)
                    )
                    .unwrap()
                    .is_none());
                } else {
                    assert!(
                        ensure_native_recording_source(&root, &boundary.source, started_at)
                            .unwrap()
                            .is_none()
                    );
                }
            }
            _ => panic!("unexpected store test action"),
        }
    }

    #[test]
    fn independent_processes_append_each_native_sequence_exactly_once() {
        let root = temp_root("process-dedup");
        let boundary = enable_native_recording(&root, &source("epoch-a"), 0).unwrap();
        fs::write(
            root.join("boundary.json"),
            serde_json::to_vec(&boundary).unwrap(),
        )
        .unwrap();
        let mut first = worker(&root, "duplicate-page");
        let mut second = worker(&root, "duplicate-page");
        fs::write(root.join("start"), "").unwrap();
        assert!(first.wait().unwrap().success());
        assert!(second.wait().unwrap().success());
        assert_eq!(archive_text(&root).lines().count(), 20);
        assert_eq!(
            native_recording_boundary(&root).unwrap().unwrap().cursor,
            20
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn stop_waits_for_a_different_process_append_and_fences_late_writers() {
        let root = temp_root("process-stop");
        let boundary = enable_native_recording(&root, &source("epoch-a"), 0).unwrap();
        fs::write(
            root.join("boundary.json"),
            serde_json::to_vec(&boundary).unwrap(),
        )
        .unwrap();
        let mut append = worker(&root, "hold-append");
        await_file(&root.join("locked"));
        let mut stop = worker(&root, "stop");
        await_file(&root.join("stop-started"));
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(!root.join("stop-completed").exists());
        fs::write(root.join("release"), "").unwrap();
        assert!(append.wait().unwrap().success());
        assert!(stop.wait().unwrap().success());
        let before = archive_text(&root);
        assert!(before.contains("before-stop"));
        let mut late = RecordedLogBuffer::new(&root);
        assert!(!late
            .append_native(&boundary, &[(2, entry("late-native"))], 2, None)
            .unwrap());
        late.push(entry("late-ordinary"));
        let mut runtime = RuntimeLogBuffer::default();
        runtime.capture(entry("late-runtime"));
        runtime.sync(&root);
        assert_eq!(archive_text(&root), before);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn another_process_cannot_finish_a_pre_stop_enable_after_off_receipt() {
        let root = temp_root("process-enable");
        let boundary = enable_native_recording(&root, &source("epoch-a"), 0).unwrap();
        fs::write(
            root.join("boundary.json"),
            serde_json::to_vec(&boundary).unwrap(),
        )
        .unwrap();
        fs::write(
            root.join("control-token"),
            recording_control_token(&root).unwrap(),
        )
        .unwrap();
        let mut enable = worker(&root, "late-enable");
        set_recording_enabled(&root, false).unwrap();
        fs::write(root.join("start"), "").unwrap();
        assert!(enable.wait().unwrap().success());
        assert!(active_recording_session(&root).unwrap().is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn late_source_publication_and_attach_from_other_processes_cannot_roll_back_epoch() {
        let root = temp_root("process-source-order");
        set_recording_enabled(&root, true).unwrap();
        let started_at = now_unix_nanos();
        let first = ensure_native_recording_source(&root, &source("epoch-a"), started_at)
            .unwrap()
            .unwrap();
        fs::write(
            root.join("boundary.json"),
            serde_json::to_vec(&first).unwrap(),
        )
        .unwrap();
        fs::write(root.join("native-started-at"), started_at.to_string()).unwrap();
        let mut attach = worker(&root, "old-source-attach");
        let mut publication = worker(&root, "old-source-ensure");
        let second = ensure_native_recording_source(&root, &source("epoch-b"), started_at + 1)
            .unwrap()
            .unwrap();
        fs::write(root.join("start"), "").unwrap();
        assert!(attach.wait().unwrap().success());
        assert!(publication.wait().unwrap().success());
        assert_eq!(native_recording_boundary(&root).unwrap(), Some(second));
        let _ = fs::remove_dir_all(root);
    }
}
