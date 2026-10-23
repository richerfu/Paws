//! Session-owned, bounded event journals. Reading a page never consumes events.
use crate::{EngineError, EngineLogPage, EngineLogRecord};
use paws_model::{LogEntry, VpnEngine};
use std::{collections::VecDeque, sync::Mutex};

struct LogState {
    records: VecDeque<EngineLogRecord>,
    bytes: usize,
    sequence: u64,
}

pub struct EngineLogBuffer {
    engine: VpnEngine,
    max_records: usize,
    max_bytes: usize,
    state: Mutex<LogState>,
}
impl EngineLogBuffer {
    pub fn new(engine: VpnEngine, max_records: usize, max_bytes: usize) -> Self {
        Self {
            engine,
            max_records,
            max_bytes,
            state: Mutex::new(LogState {
                records: VecDeque::new(),
                bytes: 0,
                sequence: 0,
            }),
        }
    }
    fn error(&self, message: &'static str) -> EngineError {
        EngineError::Backend {
            engine: self.engine,
            operation: "runtime log journal",
            message: message.into(),
        }
    }
    pub fn record(&self, entry: LogEntry) -> Result<u64, EngineError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| self.error("journal lock poisoned"))?;
        let sequence = state
            .sequence
            .checked_add(1)
            .ok_or_else(|| self.error("journal sequence exhausted"))?;
        state.sequence = sequence;
        let record = EngineLogRecord {
            sequence,
            level: entry.level,
            message: entry.message,
            timestamp: entry.timestamp,
        };
        let bytes = record_bytes(&record);
        if self.max_records == 0 || bytes > self.max_bytes {
            // A gap is observable through dropped_before. Never truncate a
            // message and present it as the original engine log record.
            state.records.clear();
            state.bytes = 0;
            return Ok(sequence);
        }
        while state.records.len() >= self.max_records || state.bytes > self.max_bytes - bytes {
            if let Some(removed) = state.records.pop_front() {
                state.bytes -= record_bytes(&removed);
            } else {
                break;
            }
        }
        state.bytes += bytes;
        state.records.push_back(record);
        Ok(sequence)
    }
    pub fn revision(&self) -> Result<u64, EngineError> {
        self.state
            .lock()
            .map(|state| state.sequence)
            .map_err(|_| self.error("journal lock poisoned"))
    }
    pub fn page(&self, after: u64, limit: u16) -> Result<EngineLogPage, EngineError> {
        if !(1..=512).contains(&limit) {
            return Err(self.error("page limit must be between 1 and 512"));
        }
        let state = self
            .state
            .lock()
            .map_err(|_| self.error("journal lock poisoned"))?;
        if after > state.sequence {
            return Err(self.error("cursor belongs to a later journal or another session"));
        }
        let first = state
            .records
            .front()
            .map_or(state.sequence.saturating_add(1), |entry| entry.sequence);
        let entries: Vec<_> = state
            .records
            .iter()
            .filter(|entry| entry.sequence > after)
            .take(usize::from(limit))
            .cloned()
            .collect();
        let next_cursor = entries
            .last()
            .map_or(state.sequence, |entry| entry.sequence);
        Ok(EngineLogPage {
            entries,
            next_cursor,
            high_watermark: state.sequence,
            dropped_before: (after.saturating_add(1) < first).then_some(first),
        })
    }
    pub fn trim(&self) -> Result<(), EngineError> {
        self.state
            .lock()
            .map_err(|_| self.error("journal lock poisoned"))?
            .records
            .shrink_to_fit();
        Ok(())
    }
}
fn record_bytes(record: &EngineLogRecord) -> usize {
    std::mem::size_of::<EngineLogRecord>()
        + record.level.len()
        + record.message.len()
        + record.timestamp.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(message: &str) -> LogEntry {
        LogEntry {
            level: "info".into(),
            message: message.into(),
            timestamp: "1".into(),
        }
    }
    #[test]
    fn pages_are_incremental_and_report_count_and_byte_eviction() {
        let journal = EngineLogBuffer::new(VpnEngine::Meow, 2, 400);
        journal.record(entry("one")).unwrap();
        journal.record(entry("two")).unwrap();
        let page = journal.page(0, 1).unwrap();
        assert_eq!(page.next_cursor, 1);
        assert_eq!(page.entries[0].message, "one");
        journal.record(entry("three")).unwrap();
        let page = journal.page(0, 10).unwrap();
        assert_eq!(page.dropped_before, Some(2));
        assert_eq!(page.entries.len(), 2);
        assert!(journal
            .page(page.next_cursor, 10)
            .unwrap()
            .entries
            .is_empty());
        journal.record(entry(&"x".repeat(500))).unwrap();
        let page = journal.page(3, 10).unwrap();
        assert!(page.entries.is_empty());
        assert_eq!(page.next_cursor, 4);
        assert_eq!(page.dropped_before, Some(5));
    }
    #[test]
    fn sessions_and_invalid_cursors_do_not_silently_replay() {
        let first = EngineLogBuffer::new(VpnEngine::Meow, 2, 400);
        first.record(entry("old")).unwrap();
        let fresh = EngineLogBuffer::new(VpnEngine::Meow, 2, 400);
        assert!(fresh.page(1, 1).is_err());
        assert!(fresh.page(0, 0).is_err());
        assert_eq!(fresh.page(0, 1).unwrap().next_cursor, 0);
    }
}
