use paws_model::PawsError;
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const PLATFORM_VPN_OWNER_JOURNAL_VERSION: u32 = 1;
const MAX_PLATFORM_VPN_OWNER_JOURNAL_BYTES: u64 = 16 * 1024;
const PLATFORM_VPN_OWNER_LEASE_VERSION: u32 = 1;
const MAX_PLATFORM_VPN_OWNER_LEASE_BYTES: u64 = 16 * 1024;
static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TestFailure {
    Read,
    WriteBeforeRename,
    Delete,
}

#[cfg(test)]
static TEST_FAILURES: std::sync::Mutex<Vec<(PathBuf, TestFailure)>> =
    std::sync::Mutex::new(Vec::new());

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProcessIdentity {
    pub(crate) boot_id: String,
    pub(crate) pid: u32,
    pub(crate) start_time: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PlatformVpnOwnerPhase {
    Pending,
    Attached,
    /// An exact Stop intent fenced Pending before HarmonyOS Ability stop.
    /// Keep the tombstone until that stop is confirmed so late Wants cannot
    /// attach and pre-stop journal absence is never treated as cleanup proof.
    Stopping,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PlatformVpnOwnerJournal {
    pub(crate) attempt_id: String,
    pub(crate) issuer: ProcessIdentity,
    pub(crate) extension: Option<ProcessIdentity>,
    pub(crate) phase: PlatformVpnOwnerPhase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum JournalRead {
    Missing,
    Present(PlatformVpnOwnerJournal),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PlatformVpnOwnerLeaseRole {
    Issuer,
    Extension,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PlatformVpnOwnerLeaseRecord {
    pub(crate) attempt_id: String,
    pub(crate) identity: ProcessIdentity,
    pub(crate) role: PlatformVpnOwnerLeaseRole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlatformVpnOwnerLeaseObservation {
    HeldExact,
    HeldOther,
    Released,
}

/// An exclusive lease on one fixed inode. Dropping it releases ownership; the
/// inode itself is deliberately never renamed or removed by this module.
#[derive(Debug)]
pub(crate) struct PlatformVpnOwnerLease {
    file: File,
}

impl Drop for PlatformVpnOwnerLease {
    fn drop(&mut self) {
        // Explicit unlock prevents a forked child from extending ownership
        // during the short interval before its O_CLOEXEC descriptors close.
        unsafe {
            libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredPlatformVpnOwnerJournal {
    version: u32,
    attempt_id: String,
    issuer: ProcessIdentity,
    extension: Option<ProcessIdentity>,
    phase: PlatformVpnOwnerPhase,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredPlatformVpnOwnerLeaseRecord {
    version: u32,
    attempt_id: String,
    identity: ProcessIdentity,
    role: PlatformVpnOwnerLeaseRole,
}

impl From<PlatformVpnOwnerJournal> for StoredPlatformVpnOwnerJournal {
    fn from(record: PlatformVpnOwnerJournal) -> Self {
        Self {
            version: PLATFORM_VPN_OWNER_JOURNAL_VERSION,
            attempt_id: record.attempt_id,
            issuer: record.issuer,
            extension: record.extension,
            phase: record.phase,
        }
    }
}

impl TryFrom<StoredPlatformVpnOwnerJournal> for PlatformVpnOwnerJournal {
    type Error = PawsError;

    fn try_from(stored: StoredPlatformVpnOwnerJournal) -> Result<Self, Self::Error> {
        if stored.version != PLATFORM_VPN_OWNER_JOURNAL_VERSION {
            return Err(journal_error(format!(
                "unsupported version {}",
                stored.version
            )));
        }
        let record = Self {
            attempt_id: stored.attempt_id,
            issuer: stored.issuer,
            extension: stored.extension,
            phase: stored.phase,
        };
        validate_record(&record)?;
        Ok(record)
    }
}

impl From<PlatformVpnOwnerLeaseRecord> for StoredPlatformVpnOwnerLeaseRecord {
    fn from(record: PlatformVpnOwnerLeaseRecord) -> Self {
        Self {
            version: PLATFORM_VPN_OWNER_LEASE_VERSION,
            attempt_id: record.attempt_id,
            identity: record.identity,
            role: record.role,
        }
    }
}

impl TryFrom<StoredPlatformVpnOwnerLeaseRecord> for PlatformVpnOwnerLeaseRecord {
    type Error = PawsError;

    fn try_from(stored: StoredPlatformVpnOwnerLeaseRecord) -> Result<Self, Self::Error> {
        if stored.version != PLATFORM_VPN_OWNER_LEASE_VERSION {
            return Err(lease_error(format!(
                "unsupported version {}",
                stored.version
            )));
        }
        let record = Self {
            attempt_id: stored.attempt_id,
            identity: stored.identity,
            role: stored.role,
        };
        validate_lease_record(&record)?;
        Ok(record)
    }
}

struct JournalLock {
    _file: File,
}

impl Drop for JournalLock {
    fn drop(&mut self) {
        // A multi-threaded process can fork while this descriptor is open.
        // Until exec closes inherited descriptors, merely closing our copy
        // can leave the shared open-file description (and its flock) alive in
        // the child. Explicitly unlock before close so an unrelated spawn
        // cannot extend this critical section.
        unsafe {
            libc::flock(self._file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

pub(crate) fn acquire_owner_lease_exact(
    path: &Path,
    record: PlatformVpnOwnerLeaseRecord,
) -> Result<PlatformVpnOwnerLease, PawsError> {
    validate_lease_record(&record)?;
    let file = open_owner_lease(path, true)?;
    if !try_lock_owner_lease(&file, path)? {
        return Err(lease_error(format!(
            "busy: lease '{}' is held by another owner",
            path.display()
        )));
    }
    let mut lease = PlatformVpnOwnerLease { file };
    write_owner_lease_record(&mut lease.file, path, record)?;
    Ok(lease)
}

pub(crate) fn observe_owner_lease_exact(
    path: &Path,
    expected: &PlatformVpnOwnerLeaseRecord,
) -> Result<PlatformVpnOwnerLeaseObservation, PawsError> {
    validate_lease_record(expected)?;
    let mut file = open_owner_lease(path, false)?;
    if try_lock_owner_lease(&file, path)? {
        drop(PlatformVpnOwnerLease { file });
        return Ok(PlatformVpnOwnerLeaseObservation::Released);
    }

    // A busy lock proves only that this fixed inode has a holder. Read the
    // strict record from the same descriptor: stale or partially rewritten
    // content is HeldOther, never proof about the expected owner.
    let observed = read_owner_lease_record(&mut file, path)?;
    Ok(if observed.as_ref() == Some(expected) {
        PlatformVpnOwnerLeaseObservation::HeldExact
    } else {
        PlatformVpnOwnerLeaseObservation::HeldOther
    })
}

/// Hold an already released lease without changing its record. The observer
/// must verify the owner journal while holding this guard; a replacement
/// Extension cannot acquire ownership until the observation is committed.
pub(crate) fn lock_released_owner_lease(
    path: &Path,
) -> Result<Option<PlatformVpnOwnerLease>, PawsError> {
    let file = open_owner_lease(path, false)?;
    if try_lock_owner_lease(&file, path)? {
        Ok(Some(PlatformVpnOwnerLease { file }))
    } else {
        Ok(None)
    }
}

pub(crate) fn read(path: &Path) -> Result<JournalRead, PawsError> {
    let _lock = lock_journal(path)?;
    read_unlocked(path)
}

pub(crate) fn create_pending_exact(
    path: &Path,
    record: PlatformVpnOwnerJournal,
) -> Result<(), PawsError> {
    validate_record(&record)?;
    if record.phase != PlatformVpnOwnerPhase::Pending || record.extension.is_some() {
        return Err(journal_error(
            "a new owner must be Pending without an Extension identity",
        ));
    }

    let _lock = lock_journal(path)?;
    match read_unlocked(path)? {
        JournalRead::Missing => write_unlocked(path, record),
        JournalRead::Present(current) if current == record => Ok(()),
        JournalRead::Present(current) => Err(journal_error(format!(
            "create conflict: existing attempt '{}' is not the requested attempt '{}'",
            current.attempt_id, record.attempt_id
        ))),
    }
}

pub(crate) fn upgrade_attached_exact(
    path: &Path,
    expected_attempt: &str,
    expected_issuer: ProcessIdentity,
    extension: ProcessIdentity,
) -> Result<(), PawsError> {
    validate_attempt_id(expected_attempt)?;
    validate_identity("expected issuer", &expected_issuer)?;
    validate_identity("Extension", &extension)?;

    let _lock = lock_journal(path)?;
    let JournalRead::Present(mut current) = read_unlocked(path)? else {
        return Err(journal_error(format!(
            "cannot attach missing attempt '{expected_attempt}'"
        )));
    };
    if current.attempt_id != expected_attempt || current.issuer != expected_issuer {
        return Err(journal_error(format!(
            "attach conflict: attempt '{expected_attempt}' or its issuer is no longer current"
        )));
    }
    match (current.phase, current.extension.as_ref()) {
        (PlatformVpnOwnerPhase::Pending, None) => {
            current.phase = PlatformVpnOwnerPhase::Attached;
            current.extension = Some(extension);
            write_unlocked(path, current)
        }
        (PlatformVpnOwnerPhase::Attached, Some(current_extension))
            if current_extension == &extension =>
        {
            Ok(())
        }
        _ => Err(journal_error(format!(
            "attach conflict: attempt '{expected_attempt}' has a different phase or Extension owner"
        ))),
    }
}

/// Persistently fence an exact Pending attempt before asking HarmonyOS to
/// stop the Extension Ability. `false` means attachment or replacement won
/// the race and the caller must re-read the journal; it is never cleanup
/// confirmation.
pub(crate) fn fence_pending_stop_exact(
    path: &Path,
    expected_attempt: &str,
    expected_issuer: ProcessIdentity,
) -> Result<bool, PawsError> {
    validate_attempt_id(expected_attempt)?;
    validate_identity("expected issuer", &expected_issuer)?;

    let _lock = lock_journal(path)?;
    let JournalRead::Present(mut current) = read_unlocked(path)? else {
        return Ok(false);
    };
    if current.attempt_id != expected_attempt || current.issuer != expected_issuer {
        return Ok(false);
    }
    match (current.phase, current.extension.as_ref()) {
        (PlatformVpnOwnerPhase::Pending, None) => {
            current.phase = PlatformVpnOwnerPhase::Stopping;
            write_unlocked(path, current)?;
            Ok(true)
        }
        (PlatformVpnOwnerPhase::Stopping, None) => Ok(true),
        (PlatformVpnOwnerPhase::Attached, Some(_)) => Ok(false),
        _ => Err(journal_error(format!(
            "stop fence conflict: attempt '{expected_attempt}' has an invalid phase or Extension owner"
        ))),
    }
}

/// Replace the process identity of an already attached Extension without
/// changing the attempt or its issuer. The caller must prove that
/// `expected_extension` released its exact ownership lease before invoking
/// this compare-and-swap.
pub(crate) fn rebind_attached_exact(
    path: &Path,
    expected_attempt: &str,
    expected_issuer: ProcessIdentity,
    expected_extension: ProcessIdentity,
    replacement_extension: ProcessIdentity,
) -> Result<(), PawsError> {
    validate_attempt_id(expected_attempt)?;
    validate_identity("expected issuer", &expected_issuer)?;
    validate_identity("expected Extension", &expected_extension)?;
    validate_identity("replacement Extension", &replacement_extension)?;

    let _lock = lock_journal(path)?;
    let JournalRead::Present(mut current) = read_unlocked(path)? else {
        return Err(journal_error(format!(
            "cannot rebind missing attempt '{expected_attempt}'"
        )));
    };
    if current.attempt_id != expected_attempt
        || current.issuer != expected_issuer
        || current.phase != PlatformVpnOwnerPhase::Attached
    {
        return Err(journal_error(format!(
            "rebind conflict: attempt '{expected_attempt}' or its issuer is no longer current"
        )));
    }
    match current.extension.as_ref() {
        Some(extension) if extension == &expected_extension => {
            if extension == &replacement_extension {
                return Ok(());
            }
            current.extension = Some(replacement_extension);
            write_unlocked(path, current)
        }
        // A retry of the same completed CAS is idempotent even though the
        // journal no longer contains its former expected identity.
        Some(extension) if extension == &replacement_extension => Ok(()),
        _ => Err(journal_error(format!(
            "rebind conflict: attempt '{expected_attempt}' has a different Extension owner"
        ))),
    }
}

pub(crate) fn delete_exact(
    path: &Path,
    expected_attempt: &str,
    expected_extension: Option<ProcessIdentity>,
) -> Result<bool, PawsError> {
    validate_attempt_id(expected_attempt)?;
    if let Some(extension) = expected_extension.as_ref() {
        validate_identity("expected Extension", extension)?;
    }

    let _lock = lock_journal(path)?;
    let JournalRead::Present(current) = read_unlocked(path)? else {
        return Ok(false);
    };
    // None is an exact expectation, not a wildcard. This makes an unattached
    // cleanup incapable of deleting a record which an Extension has adopted.
    if current.attempt_id != expected_attempt || current.extension != expected_extension {
        return Ok(false);
    }
    #[cfg(test)]
    fail_test_operation(path, TestFailure::Delete)?;
    fs::remove_file(path)
        .map_err(|error| io_error("remove platform VPN owner journal", path, error))?;
    sync_parent(path)?;
    Ok(true)
}

/// Delete only an exact still-Pending record. Unlike `delete_exact(...,
/// None)`, this cannot remove a durable Stopping tombstone and is therefore
/// safe for a pre-OS-stop dispatch failure.
pub(crate) fn delete_pending_exact(path: &Path, expected_attempt: &str) -> Result<bool, PawsError> {
    validate_attempt_id(expected_attempt)?;

    let _lock = lock_journal(path)?;
    let JournalRead::Present(current) = read_unlocked(path)? else {
        return Ok(false);
    };
    if current.attempt_id != expected_attempt
        || current.phase != PlatformVpnOwnerPhase::Pending
        || current.extension.is_some()
    {
        return Ok(false);
    }
    #[cfg(test)]
    fail_test_operation(path, TestFailure::Delete)?;
    fs::remove_file(path)
        .map_err(|error| io_error("remove pending platform VPN owner journal", path, error))?;
    sync_parent(path)?;
    Ok(true)
}

fn validate_lease_record(record: &PlatformVpnOwnerLeaseRecord) -> Result<(), PawsError> {
    if record.attempt_id.trim().is_empty() {
        return Err(lease_error("attempt id is empty"));
    }
    if record.identity.boot_id.trim().is_empty()
        || record.identity.pid == 0
        || record.identity.start_time == 0
    {
        return Err(lease_error(
            "holder identity must contain a boot id, non-zero PID, and start time",
        ));
    }
    Ok(())
}

fn open_owner_lease(path: &Path, create: bool) -> Result<File, PawsError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| lease_error(format!("path '{}' has no parent directory", path.display())))?;
    if create {
        fs::create_dir_all(parent).map_err(|error| {
            io_error("create platform VPN owner lease directory", parent, error)
        })?;
    }

    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.is_file() => {
            return Err(lease_error(format!(
                "path '{}' is not a regular file",
                path.display()
            )))
        }
        Ok(_) => {}
        Err(error) if create && error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(io_error("inspect platform VPN owner lease", path, error)),
    }

    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create(create)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
    let file = options
        .open(path)
        .map_err(|error| io_error("open platform VPN owner lease", path, error))?;
    let metadata = file
        .metadata()
        .map_err(|error| io_error("inspect opened platform VPN owner lease", path, error))?;
    if !metadata.is_file() {
        return Err(lease_error(format!(
            "opened path '{}' is not a regular file",
            path.display()
        )));
    }
    Ok(file)
}

fn try_lock_owner_lease(file: &File, path: &Path) -> Result<bool, PawsError> {
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::WouldBlock {
        return Ok(false);
    }
    Err(io_error("lock platform VPN owner lease", path, error))
}

fn write_owner_lease_record(
    file: &mut File,
    path: &Path,
    record: PlatformVpnOwnerLeaseRecord,
) -> Result<(), PawsError> {
    let bytes = serde_json::to_vec(&StoredPlatformVpnOwnerLeaseRecord::from(record))
        .map_err(|error| lease_error(format!("serialize file '{}': {error}", path.display())))?;
    if bytes.len() as u64 > MAX_PLATFORM_VPN_OWNER_LEASE_BYTES {
        return Err(lease_error(format!(
            "serialized file '{}' is too large ({} bytes)",
            path.display(),
            bytes.len()
        )));
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|error| io_error("seek platform VPN owner lease", path, error))?;
    file.set_len(0)
        .map_err(|error| io_error("truncate platform VPN owner lease", path, error))?;
    file.write_all(&bytes)
        .map_err(|error| io_error("write platform VPN owner lease", path, error))?;
    file.sync_all()
        .map_err(|error| io_error("sync platform VPN owner lease", path, error))?;
    sync_parent(path)
}

fn read_owner_lease_record(
    file: &mut File,
    path: &Path,
) -> Result<Option<PlatformVpnOwnerLeaseRecord>, PawsError> {
    file.seek(SeekFrom::Start(0))
        .map_err(|error| io_error("seek platform VPN owner lease", path, error))?;
    let mut bytes = Vec::new();
    (&mut *file)
        .take(MAX_PLATFORM_VPN_OWNER_LEASE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| io_error("read platform VPN owner lease", path, error))?;
    if bytes.len() as u64 > MAX_PLATFORM_VPN_OWNER_LEASE_BYTES {
        return Ok(None);
    }
    let Ok(stored) = serde_json::from_slice::<StoredPlatformVpnOwnerLeaseRecord>(&bytes) else {
        return Ok(None);
    };
    Ok(PlatformVpnOwnerLeaseRecord::try_from(stored).ok())
}

fn validate_record(record: &PlatformVpnOwnerJournal) -> Result<(), PawsError> {
    validate_attempt_id(&record.attempt_id)?;
    validate_identity("issuer", &record.issuer)?;
    if let Some(extension) = record.extension.as_ref() {
        validate_identity("Extension", extension)?;
    }
    match (record.phase, record.extension.as_ref()) {
        (PlatformVpnOwnerPhase::Pending, None)
        | (PlatformVpnOwnerPhase::Attached, Some(_))
        | (PlatformVpnOwnerPhase::Stopping, None) => Ok(()),
        (PlatformVpnOwnerPhase::Pending, Some(_)) => Err(journal_error(
            "Pending owner unexpectedly contains an Extension identity",
        )),
        (PlatformVpnOwnerPhase::Attached, None) => Err(journal_error(
            "Attached owner is missing its Extension identity",
        )),
        (PlatformVpnOwnerPhase::Stopping, Some(_)) => Err(journal_error(
            "Stopping owner unexpectedly contains an Extension identity",
        )),
    }
}

fn validate_attempt_id(attempt_id: &str) -> Result<(), PawsError> {
    if attempt_id.trim().is_empty() {
        return Err(journal_error("attempt id is empty"));
    }
    Ok(())
}

fn validate_identity(label: &str, identity: &ProcessIdentity) -> Result<(), PawsError> {
    if identity.boot_id.trim().is_empty() || identity.pid == 0 || identity.start_time == 0 {
        return Err(journal_error(format!(
            "{label} process identity must contain a boot id, non-zero PID, and start time"
        )));
    }
    Ok(())
}

fn lock_journal(path: &Path) -> Result<JournalLock, PawsError> {
    let parent = journal_parent(path)?;
    fs::create_dir_all(parent)
        .map_err(|error| io_error("create platform VPN journal directory", parent, error))?;
    let lock_path = lock_path(path)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_CLOEXEC)
        .open(&lock_path)
        .map_err(|error| io_error("open platform VPN owner journal lock", &lock_path, error))?;
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result != 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::WouldBlock {
            return Err(journal_error(format!(
                "busy: lock '{}' is held by another process",
                lock_path.display()
            )));
        }
        return Err(io_error(
            "lock platform VPN owner journal",
            &lock_path,
            error,
        ));
    }
    Ok(JournalLock { _file: file })
}

fn read_unlocked(path: &Path) -> Result<JournalRead, PawsError> {
    #[cfg(test)]
    fail_test_operation(path, TestFailure::Read)?;
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(JournalRead::Missing)
        }
        Err(error) => return Err(io_error("inspect platform VPN owner journal", path, error)),
    };
    if !metadata.is_file() {
        return Err(journal_error(format!(
            "path '{}' is not a regular file",
            path.display()
        )));
    }
    if metadata.len() > MAX_PLATFORM_VPN_OWNER_JOURNAL_BYTES {
        return Err(journal_error(format!(
            "file '{}' is too large ({} bytes)",
            path.display(),
            metadata.len()
        )));
    }
    let bytes =
        fs::read(path).map_err(|error| io_error("read platform VPN owner journal", path, error))?;
    let stored: StoredPlatformVpnOwnerJournal = serde_json::from_slice(&bytes)
        .map_err(|error| journal_error(format!("malformed file '{}': {error}", path.display())))?;
    Ok(JournalRead::Present(stored.try_into()?))
}

fn write_unlocked(path: &Path, record: PlatformVpnOwnerJournal) -> Result<(), PawsError> {
    validate_record(&record)?;
    let bytes = serde_json::to_vec(&StoredPlatformVpnOwnerJournal::from(record))
        .map_err(|error| journal_error(format!("serialize file '{}': {error}", path.display())))?;
    let temp_path = unique_temp_path(path)?;
    let result = (|| -> Result<(), PawsError> {
        let mut temp = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp_path)
            .map_err(|error| {
                io_error(
                    "create temporary platform VPN owner journal",
                    &temp_path,
                    error,
                )
            })?;
        temp.write_all(&bytes).map_err(|error| {
            io_error(
                "write temporary platform VPN owner journal",
                &temp_path,
                error,
            )
        })?;
        temp.sync_all().map_err(|error| {
            io_error(
                "sync temporary platform VPN owner journal",
                &temp_path,
                error,
            )
        })?;
        #[cfg(test)]
        fail_test_operation(path, TestFailure::WriteBeforeRename)?;
        fs::rename(&temp_path, path)
            .map_err(|error| io_error("replace platform VPN owner journal", path, error))?;
        sync_parent(path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

fn sync_parent(path: &Path) -> Result<(), PawsError> {
    let parent = journal_parent(path)?;
    let directory = File::open(parent)
        .map_err(|error| io_error("open platform VPN journal directory", parent, error))?;
    directory
        .sync_all()
        .map_err(|error| io_error("sync platform VPN journal directory", parent, error))
}

fn journal_parent(path: &Path) -> Result<&Path, PawsError> {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| journal_error(format!("path '{}' has no parent directory", path.display())))
}

fn lock_path(path: &Path) -> Result<PathBuf, PawsError> {
    let file_name = path
        .file_name()
        .ok_or_else(|| journal_error(format!("path '{}' has no file name", path.display())))?;
    let mut lock_name = OsString::from(file_name);
    lock_name.push(".lock");
    Ok(path.with_file_name(lock_name))
}

fn unique_temp_path(path: &Path) -> Result<PathBuf, PawsError> {
    let file_name = path
        .file_name()
        .ok_or_else(|| journal_error(format!("path '{}' has no file name", path.display())))?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let mut temp_name = OsString::from(".");
    temp_name.push(file_name);
    temp_name.push(format!(
        ".tmp-{}-{timestamp}-{sequence}",
        std::process::id()
    ));
    Ok(path.with_file_name(temp_name))
}

fn journal_error(message: impl Into<String>) -> PawsError {
    PawsError::Core(format!(
        "invalid platform VPN owner journal: {}",
        message.into()
    ))
}

fn lease_error(message: impl Into<String>) -> PawsError {
    PawsError::Core(format!(
        "invalid platform VPN owner lease: {}",
        message.into()
    ))
}

fn io_error(context: &str, path: &Path, error: std::io::Error) -> PawsError {
    PawsError::Io(format!("{context} '{}': {error}", path.display()))
}

#[cfg(test)]
fn fail_test_operation(path: &Path, failure: TestFailure) -> Result<(), PawsError> {
    let mut failures = TEST_FAILURES
        .lock()
        .map_err(|_| journal_error("test failure-injection lock is poisoned"))?;
    let Some(index) = failures.iter().position(|(candidate, candidate_failure)| {
        candidate == path && *candidate_failure == failure
    }) else {
        return Ok(());
    };
    failures.swap_remove(index);
    Err(io_error(
        "injected platform VPN owner journal failure",
        path,
        std::io::Error::other(format!("{failure:?}")),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;
    use std::process::{Command, Stdio};
    use std::thread;
    use std::time::Duration;

    const LOCK_CHILD_PATH_ENV: &str = "PAWS_PLATFORM_OWNER_LOCK_CHILD_PATH";
    const LOCK_CHILD_MARKER_ENV: &str = "PAWS_PLATFORM_OWNER_LOCK_CHILD_MARKER";
    const LOCK_INHERITANCE_MARKER_ENV: &str = "PAWS_PLATFORM_OWNER_LOCK_INHERITANCE_MARKER";
    const LEASE_CHILD_PATH_ENV: &str = "PAWS_PLATFORM_OWNER_LEASE_CHILD_PATH";
    const LEASE_CHILD_MARKER_ENV: &str = "PAWS_PLATFORM_OWNER_LEASE_CHILD_MARKER";

    fn test_path(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir()
            .join(format!(
                "paws-platform-owner-{label}-{}-{nonce}",
                std::process::id()
            ))
            .join("runtime/platform-vpn-owner.json")
    }

    fn identity(pid: u32, start_time: u64) -> ProcessIdentity {
        ProcessIdentity {
            boot_id: "boot-a".to_owned(),
            pid,
            start_time,
        }
    }

    fn identity_on_boot(boot_id: &str, pid: u32, start_time: u64) -> ProcessIdentity {
        ProcessIdentity {
            boot_id: boot_id.to_owned(),
            pid,
            start_time,
        }
    }

    fn pending(attempt_id: &str, issuer: ProcessIdentity) -> PlatformVpnOwnerJournal {
        PlatformVpnOwnerJournal {
            attempt_id: attempt_id.to_owned(),
            issuer,
            extension: None,
            phase: PlatformVpnOwnerPhase::Pending,
        }
    }

    fn lease_record(
        attempt_id: &str,
        identity: ProcessIdentity,
        role: PlatformVpnOwnerLeaseRole,
    ) -> PlatformVpnOwnerLeaseRecord {
        PlatformVpnOwnerLeaseRecord {
            attempt_id: attempt_id.to_owned(),
            identity,
            role,
        }
    }

    fn cleanup(path: &Path) {
        if let Some(root) = path.parent().and_then(Path::parent) {
            let _ = fs::remove_dir_all(root);
        }
    }

    fn arm_failure(path: &Path, failure: TestFailure) {
        TEST_FAILURES
            .lock()
            .unwrap()
            .push((path.to_path_buf(), failure));
    }

    #[test]
    fn owner_lease_is_strict_about_missing_io_and_symlink_paths() {
        let path = test_path("lease-paths").with_file_name("platform-vpn-issuer.lease");
        let expected = lease_record(
            "attempt-lease",
            identity(510, 5_010),
            PlatformVpnOwnerLeaseRole::Issuer,
        );

        assert!(matches!(
            observe_owner_lease_exact(&path, &expected),
            Err(PawsError::Io(_))
        ));

        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let missing_target = path.with_file_name("missing-lease-target");
        std::os::unix::fs::symlink(&missing_target, &path).unwrap();
        let error = observe_owner_lease_exact(&path, &expected)
            .unwrap_err()
            .to_string();
        assert!(error.contains("not a regular file"), "{error}");
        assert!(acquire_owner_lease_exact(&path, expected.clone()).is_err());

        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(observe_owner_lease_exact(&path, &expected).is_err());
        fs::remove_dir(&path).unwrap();

        let mut malformed_holder = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&path)
            .unwrap();
        malformed_holder.write_all(b"{malformed").unwrap();
        malformed_holder.sync_all().unwrap();
        assert_eq!(
            unsafe { libc::flock(malformed_holder.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0
        );
        assert_eq!(
            observe_owner_lease_exact(&path, &expected).unwrap(),
            PlatformVpnOwnerLeaseObservation::HeldOther
        );
        assert_eq!(
            unsafe { libc::flock(malformed_holder.as_raw_fd(), libc::LOCK_UN) },
            0
        );
        drop(malformed_holder);
        fs::remove_file(&path).unwrap();

        let blocking_parent = path.with_file_name("not-a-directory");
        fs::write(&blocking_parent, b"file").unwrap();
        let child = blocking_parent.join("lease");
        assert!(matches!(
            observe_owner_lease_exact(&child, &expected),
            Err(PawsError::Io(_))
        ));
        cleanup(&child);
    }

    #[test]
    fn owner_lease_observes_exact_cross_process_holder_then_release() {
        let expected = lease_record(
            "attempt-lease",
            identity(511, 5_011),
            PlatformVpnOwnerLeaseRole::Issuer,
        );
        if let Some(child_path) = std::env::var_os(LEASE_CHILD_PATH_ENV) {
            let lease = acquire_owner_lease_exact(&PathBuf::from(child_path), expected).unwrap();
            let marker = std::env::var_os(LEASE_CHILD_MARKER_ENV).unwrap();
            fs::write(marker, b"lease-held").unwrap();
            thread::sleep(Duration::from_millis(500));
            drop(lease);
            return;
        }

        let path = test_path("lease-process").with_file_name("platform-vpn-issuer.lease");
        let marker = path.with_file_name("lease-child-held");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("platform_owner::tests::owner_lease_observes_exact_cross_process_holder_then_release")
            .arg("--test-threads=1")
            .env(LEASE_CHILD_PATH_ENV, &path)
            .env(LEASE_CHILD_MARKER_ENV, &marker)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        for _ in 0..200 {
            if marker.exists() {
                break;
            }
            assert!(
                child.try_wait().unwrap().is_none(),
                "lease child exited before acquiring its lock"
            );
            thread::sleep(Duration::from_millis(10));
        }
        assert!(marker.exists(), "lease child did not acquire its lock");

        let inode = fs::symlink_metadata(&path).unwrap().ino();
        let busy = acquire_owner_lease_exact(&path, expected.clone())
            .unwrap_err()
            .to_string();
        assert!(busy.contains("busy"), "{busy}");
        assert_eq!(
            observe_owner_lease_exact(&path, &expected).unwrap(),
            PlatformVpnOwnerLeaseObservation::HeldExact
        );
        let different = lease_record(
            "attempt-other",
            identity(512, 5_012),
            PlatformVpnOwnerLeaseRole::Extension,
        );
        assert_eq!(
            observe_owner_lease_exact(&path, &different).unwrap(),
            PlatformVpnOwnerLeaseObservation::HeldOther
        );
        assert!(lock_released_owner_lease(&path).unwrap().is_none());

        assert!(child.wait().unwrap().success());
        assert_eq!(
            observe_owner_lease_exact(&path, &expected).unwrap(),
            PlatformVpnOwnerLeaseObservation::Released
        );
        let released_guard = lock_released_owner_lease(&path).unwrap().unwrap();
        assert!(acquire_owner_lease_exact(&path, different.clone()).is_err());
        drop(released_guard);
        let replacement = acquire_owner_lease_exact(&path, different.clone()).unwrap();
        assert_eq!(fs::symlink_metadata(&path).unwrap().ino(), inode);
        assert_eq!(
            observe_owner_lease_exact(&path, &different).unwrap(),
            PlatformVpnOwnerLeaseObservation::HeldExact
        );
        assert_eq!(
            observe_owner_lease_exact(&path, &expected).unwrap(),
            PlatformVpnOwnerLeaseObservation::HeldOther
        );
        drop(replacement);
        assert_eq!(fs::symlink_metadata(&path).unwrap().ino(), inode);
        cleanup(&path);
    }

    #[test]
    fn missing_is_distinct_from_malformed() {
        let path = test_path("strict-read");
        assert_eq!(read(&path).unwrap(), JournalRead::Missing);

        fs::write(&path, br#"{"version":1,"attemptId":7}"#).unwrap();
        let error = read(&path).unwrap_err().to_string();
        assert!(error.contains("malformed file"), "{error}");

        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        let error = read(&path).unwrap_err().to_string();
        assert!(error.contains("not a regular file"), "{error}");

        fs::remove_dir(&path).unwrap();
        let missing_target = path.with_file_name("missing-owner-target.json");
        std::os::unix::fs::symlink(&missing_target, &path).unwrap();
        let error = read(&path).unwrap_err().to_string();
        assert!(error.contains("not a regular file"), "{error}");
        cleanup(&path);
    }

    #[test]
    fn lock_contention_is_explicit_across_processes() {
        if let Some(marker) = std::env::var_os(LOCK_INHERITANCE_MARKER_ENV) {
            fs::write(marker, b"child-running").unwrap();
            thread::sleep(Duration::from_millis(500));
            return;
        }
        if let Some(child_path) = std::env::var_os(LOCK_CHILD_PATH_ENV) {
            let child_path = PathBuf::from(child_path);
            let error = read(&child_path).unwrap_err().to_string();
            assert!(error.contains("busy"), "{error}");
            let marker = std::env::var_os(LOCK_CHILD_MARKER_ENV).unwrap();
            fs::write(marker, b"observed-busy").unwrap();
            return;
        }

        // Exercise the fork/exec inheritance window independently of direct
        // contention. After the parent's explicit unlock, a still-running
        // child must not retain the parent's lock through an inherited file
        // description.
        let inherited_path = test_path("lock-inheritance");
        let inherited_marker = inherited_path.with_file_name("child-running");
        let inherited_lock = lock_journal(&inherited_path).unwrap();
        let mut inherited_child = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("platform_owner::tests::lock_contention_is_explicit_across_processes")
            .arg("--test-threads=1")
            .env(LOCK_INHERITANCE_MARKER_ENV, &inherited_marker)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        for _ in 0..200 {
            if inherited_marker.exists() {
                break;
            }
            assert!(
                inherited_child.try_wait().unwrap().is_none(),
                "inheritance child exited before reaching its hold point"
            );
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            inherited_marker.exists(),
            "inheritance child did not reach its hold point"
        );
        drop(inherited_lock);
        assert_eq!(read(&inherited_path).unwrap(), JournalRead::Missing);
        assert!(inherited_child.wait().unwrap().success());
        cleanup(&inherited_path);

        let path = test_path("lock-contention");
        let marker = path.with_file_name("child-observed-busy");
        let lock = lock_journal(&path).unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("platform_owner::tests::lock_contention_is_explicit_across_processes")
            .arg("--test-threads=1")
            .env(LOCK_CHILD_PATH_ENV, &path)
            .env(LOCK_CHILD_MARKER_ENV, &marker)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut status = None;
        for _ in 0..200 {
            if let Some(observed) = child.try_wait().unwrap() {
                status = Some(observed);
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let status = match status {
            Some(status) => status,
            None => {
                child.kill().unwrap();
                let _ = child.wait();
                panic!("contending journal process did not return promptly");
            }
        };
        assert!(status.success(), "contending journal process failed");
        assert_eq!(fs::read(&marker).unwrap(), b"observed-busy");
        drop(lock);
        cleanup(&path);
    }

    #[test]
    fn pending_create_is_exact_and_idempotent() {
        let path = test_path("create");
        let issuer = identity(101, 1_001);
        let first = pending("attempt-a", issuer.clone());
        create_pending_exact(&path, first.clone()).unwrap();
        create_pending_exact(&path, first.clone()).unwrap();
        assert_eq!(read(&path).unwrap(), JournalRead::Present(first));

        let error = create_pending_exact(&path, pending("attempt-b", issuer))
            .unwrap_err()
            .to_string();
        assert!(error.contains("create conflict"), "{error}");
        assert!(matches!(
            read(&path).unwrap(),
            JournalRead::Present(record) if record.attempt_id == "attempt-a"
        ));
        cleanup(&path);
    }

    #[test]
    fn attach_requires_the_exact_attempt_and_issuer() {
        let path = test_path("attach");
        let issuer = identity(102, 1_002);
        let extension = identity(202, 2_002);
        create_pending_exact(&path, pending("attempt-a", issuer.clone())).unwrap();

        assert!(
            upgrade_attached_exact(&path, "attempt-b", issuer.clone(), extension.clone()).is_err()
        );
        assert!(upgrade_attached_exact(
            &path,
            "attempt-a",
            identity(103, 1_003),
            extension.clone()
        )
        .is_err());
        assert!(upgrade_attached_exact(
            &path,
            "attempt-a",
            identity_on_boot("boot-b", issuer.pid, issuer.start_time),
            extension.clone()
        )
        .is_err());
        upgrade_attached_exact(&path, "attempt-a", issuer.clone(), extension.clone()).unwrap();
        upgrade_attached_exact(&path, "attempt-a", issuer.clone(), extension.clone()).unwrap();
        assert!(upgrade_attached_exact(&path, "attempt-a", issuer, identity(203, 2_003)).is_err());

        assert!(matches!(
            read(&path).unwrap(),
            JournalRead::Present(PlatformVpnOwnerJournal {
                phase: PlatformVpnOwnerPhase::Attached,
                extension: Some(owner),
                ..
            }) if owner == extension
        ));
        cleanup(&path);
    }

    #[test]
    fn pending_stop_fence_is_exact_durable_and_not_a_dispatch_failure_target() {
        let path = test_path("stop-fence");
        let issuer = identity(109, 1_009);
        let pending_record = pending("attempt-a", issuer.clone());
        create_pending_exact(&path, pending_record.clone()).unwrap();

        assert!(!fence_pending_stop_exact(&path, "attempt-a", identity(110, 1_010),).unwrap());
        assert_eq!(
            read(&path).unwrap(),
            JournalRead::Present(pending_record.clone())
        );

        arm_failure(&path, TestFailure::WriteBeforeRename);
        assert!(fence_pending_stop_exact(&path, "attempt-a", issuer.clone()).is_err());
        assert_eq!(read(&path).unwrap(), JournalRead::Present(pending_record));

        assert!(fence_pending_stop_exact(&path, "attempt-a", issuer.clone()).unwrap());
        assert!(fence_pending_stop_exact(&path, "attempt-a", issuer.clone()).unwrap());
        assert!(matches!(
            read(&path).unwrap(),
            JournalRead::Present(PlatformVpnOwnerJournal {
                phase: PlatformVpnOwnerPhase::Stopping,
                extension: None,
                ..
            })
        ));
        assert!(
            upgrade_attached_exact(&path, "attempt-a", issuer.clone(), identity(209, 2_009),)
                .is_err()
        );
        assert!(!delete_pending_exact(&path, "attempt-a").unwrap());
        assert!(delete_exact(&path, "attempt-a", None).unwrap());

        create_pending_exact(&path, pending("attempt-b", issuer.clone())).unwrap();
        let extension = identity(210, 2_010);
        upgrade_attached_exact(&path, "attempt-b", issuer.clone(), extension.clone()).unwrap();
        assert!(!fence_pending_stop_exact(&path, "attempt-b", issuer).unwrap());
        assert!(matches!(
            read(&path).unwrap(),
            JournalRead::Present(PlatformVpnOwnerJournal {
                phase: PlatformVpnOwnerPhase::Attached,
                extension: Some(owner),
                ..
            }) if owner == extension
        ));
        cleanup(&path);
    }

    #[test]
    fn delete_none_is_exact_and_old_cleanup_cannot_delete_a_new_owner() {
        let path = test_path("delete");
        let old_issuer = identity(104, 1_004);
        let old_extension = identity(204, 2_004);
        create_pending_exact(&path, pending("attempt-old", old_issuer.clone())).unwrap();
        upgrade_attached_exact(&path, "attempt-old", old_issuer, old_extension.clone()).unwrap();

        assert!(!delete_exact(&path, "attempt-old", None).unwrap());
        assert!(delete_exact(&path, "attempt-old", Some(old_extension.clone())).unwrap());
        assert!(!delete_exact(&path, "attempt-old", Some(old_extension.clone())).unwrap());

        let new_record = pending("attempt-new", identity(105, 1_005));
        create_pending_exact(&path, new_record.clone()).unwrap();
        assert!(!delete_exact(&path, "attempt-old", Some(old_extension)).unwrap());
        assert_eq!(read(&path).unwrap(), JournalRead::Present(new_record));
        cleanup(&path);
    }

    #[test]
    fn rebind_is_an_exact_cas_and_old_extension_cannot_delete_replacement() {
        let path = test_path("rebind");
        let issuer = identity(107, 1_007);
        let old_extension = identity(207, 2_007);
        let replacement = identity(307, 3_007);
        create_pending_exact(&path, pending("attempt-a", issuer.clone())).unwrap();
        upgrade_attached_exact(&path, "attempt-a", issuer.clone(), old_extension.clone()).unwrap();

        rebind_attached_exact(
            &path,
            "attempt-a",
            issuer.clone(),
            old_extension.clone(),
            replacement.clone(),
        )
        .unwrap();
        rebind_attached_exact(
            &path,
            "attempt-a",
            issuer.clone(),
            old_extension.clone(),
            replacement.clone(),
        )
        .unwrap();

        assert!(!delete_exact(&path, "attempt-a", Some(old_extension.clone())).unwrap());
        let old_extension_other_boot =
            identity_on_boot("boot-b", old_extension.pid, old_extension.start_time);
        assert!(!delete_exact(&path, "attempt-a", Some(old_extension_other_boot)).unwrap());
        let competitor = identity(407, 4_007);
        assert!(
            rebind_attached_exact(&path, "attempt-a", issuer, old_extension, competitor,).is_err()
        );
        assert!(matches!(
            read(&path).unwrap(),
            JournalRead::Present(PlatformVpnOwnerJournal {
                extension: Some(owner),
                ..
            }) if owner == replacement
        ));
        assert!(delete_exact(&path, "attempt-a", Some(replacement)).unwrap());
        cleanup(&path);
    }

    #[test]
    fn invalid_records_never_reach_storage() {
        let path = test_path("invalid");
        let error = create_pending_exact(&path, pending("", identity(106, 1_006)))
            .unwrap_err()
            .to_string();
        assert!(error.contains("attempt id is empty"), "{error}");
        let error = create_pending_exact(&path, pending("attempt", identity(0, 1_006)))
            .unwrap_err()
            .to_string();
        assert!(error.contains("non-zero PID"), "{error}");
        let error =
            create_pending_exact(&path, pending("attempt", identity_on_boot(" ", 106, 1_006)))
                .unwrap_err()
                .to_string();
        assert!(error.contains("boot id"), "{error}");
        assert_eq!(read(&path).unwrap(), JournalRead::Missing);
        cleanup(&path);
    }

    #[test]
    fn io_failures_are_explicit_and_preserve_the_previous_owner() {
        let path = test_path("io-failures");
        let issuer = identity(108, 1_008);
        let extension = identity(208, 2_008);
        let pending_record = pending("attempt-a", issuer.clone());

        arm_failure(&path, TestFailure::WriteBeforeRename);
        let error = create_pending_exact(&path, pending_record.clone())
            .unwrap_err()
            .to_string();
        assert!(error.contains("injected"), "{error}");
        assert_eq!(read(&path).unwrap(), JournalRead::Missing);

        create_pending_exact(&path, pending_record.clone()).unwrap();
        arm_failure(&path, TestFailure::Read);
        let error = read(&path).unwrap_err().to_string();
        assert!(error.contains("injected"), "{error}");
        assert_eq!(
            read(&path).unwrap(),
            JournalRead::Present(pending_record.clone())
        );

        arm_failure(&path, TestFailure::WriteBeforeRename);
        let error = upgrade_attached_exact(&path, "attempt-a", issuer.clone(), extension.clone())
            .unwrap_err()
            .to_string();
        assert!(error.contains("injected"), "{error}");
        assert_eq!(read(&path).unwrap(), JournalRead::Present(pending_record));

        upgrade_attached_exact(&path, "attempt-a", issuer, extension.clone()).unwrap();
        let attached_record = match read(&path).unwrap() {
            JournalRead::Present(record) => record,
            JournalRead::Missing => panic!("attached record disappeared"),
        };
        arm_failure(&path, TestFailure::Delete);
        let error = delete_exact(&path, "attempt-a", Some(extension))
            .unwrap_err()
            .to_string();
        assert!(error.contains("injected"), "{error}");
        assert_eq!(read(&path).unwrap(), JournalRead::Present(attached_record));
        cleanup(&path);
    }
}
