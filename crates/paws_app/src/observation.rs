use crate::AppError;
use paws_core::{RuntimeDomain, RuntimeDomainKey, RuntimeRevisions};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Mutex,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Subscription {
    generation: u64,
    domains: Vec<RuntimeDomainKey>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn observation_is_metadata_only_coalesces_latest_versions_and_stop_barrier_is_small() {
        let initial = RuntimeRevisions {
            revision: 1,
            config_revision: 1,
            status_revision: 1,
            resource_revision: 1,
            ..Default::default()
        };
        let (tx, rx) = tokio::sync::watch::channel(initial);
        let observer = StateObservation::new(rx);
        let subscription = serde_json::from_value(json!({"generation":1,"domains":[{"domain":"rules"},{"domain":"proxies","groupKey":"A"}]})).unwrap();
        let init = serde_json::to_value(observer.subscribe(subscription).unwrap()).unwrap();
        assert_eq!(init["changes"].as_array().unwrap().len(), 4);
        tx.send_replace(RuntimeRevisions {
            revision: 9,
            telemetry_revision: 8,
            ..initial
        });
        let tick = serde_json::to_value(observer.observe(false).unwrap()).unwrap();
        assert!(
            tick["change"]["changes"].as_array().unwrap().is_empty(),
            "telemetry cannot dirty rules/proxies"
        );
        tx.send_replace(RuntimeRevisions {
            revision: 12,
            config_revision: 2,
            telemetry_revision: 8,
            ..initial
        });
        let changed = serde_json::to_value(observer.observe(false).unwrap()).unwrap();
        assert!(changed["change"]["changes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v["revision"] == 12));
        let stop = serde_json::to_value(observer.observe(true).unwrap()).unwrap();
        assert_eq!(stop["barrier"]["required"].as_array().unwrap().len(), 2);
        assert!(stop["barrier"]["required"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| ["config", "status"].contains(&v["domain"].as_str().unwrap())));
        assert!(serde_json::to_string(&changed).unwrap().len() < 2048);
        observer.close();
        assert!(observer.observe(false).is_err());
    }

    #[test]
    fn query_leases_reject_duplicates_and_old_cleanup_cannot_release_new_generation() {
        let (_, rx) = tokio::sync::watch::channel(RuntimeRevisions {
            revision: 1,
            ..Default::default()
        });
        let observer = StateObservation::new(rx);
        let subscription = |generation| {
            serde_json::from_value(json!({"generation":generation,"domains":[{"domain":"rules"}]}))
                .unwrap()
        };
        observer.subscribe(subscription(1)).unwrap();
        let key = RuntimeDomainKey {
            domain: RuntimeDomain::Rules,
            group_key: None,
        };
        let old = observer.begin_query(1, key.clone()).unwrap();
        assert!(matches!(
            observer.begin_query(1, key.clone()),
            Err(AppError::ObservationQueryBusy)
        ));
        observer.subscribe(subscription(2)).unwrap();
        assert!(observer.validate(1, &key).is_err());
        let new = observer.begin_query(2, key.clone()).unwrap();
        drop(old);
        assert!(matches!(
            observer.begin_query(2, key.clone()),
            Err(AppError::ObservationQueryBusy)
        ));
        drop(new);
        assert!(observer.begin_query(2, key.clone()).is_ok());
        observer.close();
        assert!(matches!(
            observer.begin_query(2, key),
            Err(AppError::ObservationClosed)
        ));
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Query {
    pub generation: u64,
    pub domain: RuntimeDomain,
    pub group_key: Option<String>,
}

impl Query {
    pub fn key(&self) -> RuntimeDomainKey {
        RuntimeDomainKey {
            domain: self.domain,
            group_key: self.group_key.clone(),
        }
    }
}

#[derive(Clone, Serialize)]
pub(crate) struct DomainRevision {
    #[serde(flatten)]
    key: RuntimeDomainKey,
    revision: u64,
}

#[derive(Serialize)]
pub(crate) struct Changed {
    generation: u64,
    revision: u64,
    changes: Vec<DomainRevision>,
}

#[derive(Serialize)]
pub(crate) struct Barrier {
    generation: u64,
    required: Vec<DomainRevision>,
}

#[derive(Serialize)]
pub(crate) struct Observation {
    change: Changed,
    barrier: Barrier,
}

struct State {
    generation: u64,
    closed: bool,
    last: RuntimeRevisions,
    domains: BTreeMap<RuntimeDomainKey, u64>,
    in_flight: BTreeSet<RuntimeDomainKey>,
}

/// Owns a UI-independent interest set and only revision metadata. No domain
/// data is cached or implicitly queried by this subscription.
pub(crate) struct StateObservation {
    state: Mutex<State>,
    revisions: tokio::sync::watch::Receiver<RuntimeRevisions>,
}

impl StateObservation {
    pub fn new(revisions: tokio::sync::watch::Receiver<RuntimeRevisions>) -> Self {
        let last = *revisions.borrow();
        Self {
            state: Mutex::new(State {
                generation: 0,
                closed: false,
                last,
                domains: BTreeMap::new(),
                in_flight: BTreeSet::new(),
            }),
            revisions,
        }
    }

    pub fn close(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.closed = true;
            state.domains.clear();
            state.in_flight.clear();
        }
    }

    pub fn subscribe(&self, request: Subscription) -> Result<Changed, AppError> {
        if request.generation == 0
            || request.generation > 9_007_199_254_740_991
            || request.domains.len() > 64
        {
            return Err(AppError::InvalidRequest(
                "invalid observation generation or domain count".into(),
            ));
        }
        let mut domains = BTreeMap::new();
        for key in request.domains {
            key.validate()?;
            domains.insert(key, 0);
        }
        for domain in [RuntimeDomain::Config, RuntimeDomain::Status] {
            domains.insert(
                RuntimeDomainKey {
                    domain,
                    group_key: None,
                },
                0,
            );
        }
        let mut state = self.state.lock().map_err(|_| AppError::Poisoned)?;
        if state.closed {
            return Err(AppError::ObservationClosed);
        }
        if request.generation < state.generation
            || request.generation == state.generation && !state.domains.keys().eq(domains.keys())
        {
            return Err(AppError::ObservationSuperseded);
        }
        let current = *self.revisions.borrow();
        if request.generation > state.generation {
            state.generation = request.generation;
            state.domains = domains;
            state.in_flight.clear();
        }
        state.last = current;
        for version in state.domains.values_mut() {
            *version = current.revision;
        }
        Ok(Changed {
            generation: state.generation,
            revision: current.revision,
            changes: Self::versions(&state, false),
        })
    }

    pub fn validate(&self, generation: u64, key: &RuntimeDomainKey) -> Result<(), AppError> {
        let state = self.state.lock().map_err(|_| AppError::Poisoned)?;
        if state.closed {
            return Err(AppError::ObservationClosed);
        }
        if state.generation != generation || !state.domains.contains_key(key) {
            return Err(AppError::ObservationSuperseded);
        }
        Ok(())
    }

    pub fn begin_query(
        &self,
        generation: u64,
        key: RuntimeDomainKey,
    ) -> Result<QueryLease<'_>, AppError> {
        let mut state = self.state.lock().map_err(|_| AppError::Poisoned)?;
        if state.closed {
            return Err(AppError::ObservationClosed);
        }
        if state.generation != generation || !state.domains.contains_key(&key) {
            return Err(AppError::ObservationSuperseded);
        }
        if !state.in_flight.insert(key.clone()) {
            return Err(AppError::ObservationQueryBusy);
        }
        Ok(QueryLease {
            owner: self,
            generation,
            key,
        })
    }

    pub fn observe(&self, critical_only: bool) -> Result<Observation, AppError> {
        let mut state = self.state.lock().map_err(|_| AppError::Poisoned)?;
        if state.closed {
            return Err(AppError::ObservationClosed);
        }
        let current = *self.revisions.borrow();
        let previous = state.last;
        let mut changes = Vec::new();
        for (key, version) in &mut state.domains {
            if key.domain.changed(previous, current) {
                *version = current.revision;
                changes.push(DomainRevision {
                    key: key.clone(),
                    revision: *version,
                });
            }
        }
        state.last = current;
        Ok(Observation {
            change: Changed {
                generation: state.generation,
                revision: current.revision,
                changes,
            },
            barrier: Barrier {
                generation: state.generation,
                required: Self::versions(&state, critical_only),
            },
        })
    }

    fn versions(state: &State, critical_only: bool) -> Vec<DomainRevision> {
        state
            .domains
            .iter()
            .filter(|(key, _)| !critical_only || key.domain.is_critical())
            .map(|(key, revision)| DomainRevision {
                key: key.clone(),
                revision: *revision,
            })
            .collect()
    }
}

pub(crate) struct QueryLease<'a> {
    owner: &'a StateObservation,
    generation: u64,
    key: RuntimeDomainKey,
}

impl Drop for QueryLease<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.owner.state.lock() {
            if state.generation == self.generation {
                state.in_flight.remove(&self.key);
            }
        }
    }
}
