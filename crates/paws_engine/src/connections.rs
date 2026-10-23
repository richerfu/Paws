use crate::{EngineConnectionEvent, EngineConnectionPage, EngineError};
use paws_model::{ConnectionSummary, VpnEngine};
use std::{collections::VecDeque, sync::Mutex};

struct State {
    entries: VecDeque<EngineConnectionEvent>,
    sequence: u64,
    bytes: usize,
    latest_rule_node: Option<String>,
}
pub struct EngineConnectionBuffer {
    engine: VpnEngine,
    max_entries: usize,
    max_bytes: usize,
    state: Mutex<State>,
}
impl EngineConnectionBuffer {
    pub fn new(engine: VpnEngine, max_entries: usize, max_bytes: usize) -> Self {
        Self {
            engine,
            max_entries,
            max_bytes,
            state: Mutex::new(State {
                entries: VecDeque::new(),
                sequence: 0,
                bytes: 0,
                latest_rule_node: None,
            }),
        }
    }
    fn error(&self, message: &'static str) -> EngineError {
        EngineError::Backend {
            engine: self.engine,
            operation: "connection journal",
            message: message.into(),
        }
    }
    pub fn record(&self, connection: ConnectionSummary, closed: bool) -> Result<(), EngineError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| self.error("journal lock poisoned"))?;
        let sequence = state
            .sequence
            .checked_add(1)
            .ok_or_else(|| self.error("journal sequence exhausted"))?;
        state.sequence = sequence;
        if !closed {
            state.latest_rule_node =
                (!connection.proxy.is_empty()).then(|| connection.proxy.clone());
        }
        let entry = EngineConnectionEvent {
            sequence,
            connection,
            closed,
        };
        let bytes = event_bytes(&entry);
        if self.max_entries == 0 || bytes > self.max_bytes {
            state.entries.clear();
            state.bytes = 0;
            return Ok(());
        }
        while state.entries.len() >= self.max_entries || state.bytes > self.max_bytes - bytes {
            if let Some(old) = state.entries.pop_front() {
                state.bytes -= event_bytes(&old);
            } else {
                break;
            }
        }
        state.bytes += bytes;
        state.entries.push_back(entry);
        Ok(())
    }
    pub fn page(&self, after: u64, limit: u16) -> Result<EngineConnectionPage, EngineError> {
        if !(1..=512).contains(&limit) {
            return Err(self.error("page limit must be 1..512"));
        }
        let state = self
            .state
            .lock()
            .map_err(|_| self.error("journal lock poisoned"))?;
        if after > state.sequence {
            return Err(self.error("cursor belongs to another session or later journal"));
        }
        let first = state
            .entries
            .front()
            .map_or(state.sequence.saturating_add(1), |entry| entry.sequence);
        let entries: Vec<_> = state
            .entries
            .iter()
            .filter(|entry| entry.sequence > after)
            .take(usize::from(limit))
            .cloned()
            .collect();
        let next_cursor = entries
            .last()
            .map_or(state.sequence, |entry| entry.sequence);
        Ok(EngineConnectionPage {
            entries,
            next_cursor,
            high_watermark: state.sequence,
            dropped_before: (after.saturating_add(1) < first).then_some(first),
        })
    }
    pub fn revision(&self) -> Result<u64, EngineError> {
        Ok(self
            .state
            .lock()
            .map_err(|_| self.error("journal lock poisoned"))?
            .sequence)
    }
    pub fn latest_rule_node(&self) -> Result<Option<String>, EngineError> {
        Ok(self
            .state
            .lock()
            .map_err(|_| self.error("journal lock poisoned"))?
            .latest_rule_node
            .clone())
    }
}
fn event_bytes(event: &EngineConnectionEvent) -> usize {
    let connection = &event.connection;
    std::mem::size_of::<EngineConnectionEvent>()
        + connection.domain_source.len()
        + connection.id.len()
        + connection.host.len()
        + connection.domain.len()
        + connection.destination_ip.len()
        + connection.network.len()
        + connection.rule.len()
        + connection.rule_payload.len()
        + connection.proxy.len()
        + connection.started_at.len()
        + connection
            .chains
            .iter()
            .map(|chain| std::mem::size_of::<String>() + chain.len())
            .sum::<usize>()
}
