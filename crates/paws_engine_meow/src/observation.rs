//! Session-owned metadata revisions. No roster is cloned on mutation paths.
use meow_common::{adapter::ProxyHealthObserver, Proxy};
use paws_engine::{EngineError, EngineRuntimeRevisions};
use paws_model::VpnEngine;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, RwLock,
};

pub(crate) struct ObservationClock {
    active: RwLock<bool>,
    groups: AtomicU64,
    providers: AtomicU64,
    rules: AtomicU64,
    exhausted: AtomicBool,
}
impl ObservationClock {
    pub fn new() -> Self {
        Self {
            active: RwLock::new(false),
            groups: AtomicU64::new(1),
            providers: AtomicU64::new(1),
            rules: AtomicU64::new(1),
            exhausted: AtomicBool::new(false),
        }
    }
    pub fn activate(&self) {
        *self
            .active
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
    }
    pub fn revoke(&self) {
        *self
            .active
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = false;
    }
    #[allow(
        deprecated,
        reason = "fetch_update supports the workspace Rust 1.90 MSRV for checked atomic increments"
    )]
    fn advance(&self, revision: &AtomicU64) {
        if revision
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |old| {
                old.checked_add(1)
            })
            .is_err()
        {
            self.exhausted.store(true, Ordering::Release);
        }
    }
    pub fn bind(self: &Arc<Self>, proxy: &Arc<dyn Proxy>) {
        let observer: Arc<dyn ProxyHealthObserver> = self.clone();
        proxy.health().observe(Arc::downgrade(&observer));
    }
    pub fn snapshot(&self) -> Result<EngineRuntimeRevisions, EngineError> {
        if self.exhausted.load(Ordering::Acquire) {
            return Err(EngineError::Backend {
                engine: VpnEngine::Meow,
                operation: "runtime revisions",
                message: "runtime revision exhausted".into(),
            });
        }
        Ok(EngineRuntimeRevisions {
            groups: self.groups.load(Ordering::Acquire),
            providers: self.providers.load(Ordering::Acquire),
            rules: self.rules.load(Ordering::Acquire),
            ..Default::default()
        })
    }
}
impl ProxyHealthObserver for ObservationClock {
    fn update(&self, mutation: &mut dyn FnMut()) -> bool {
        let active = self
            .active
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !*active {
            return false;
        }
        mutation();
        self.advance(&self.groups);
        self.advance(&self.providers);
        true
    }
}
impl meow_config::observation::ProviderObserver for ObservationClock {
    fn updated(&self, proxies: Option<&[Arc<dyn Proxy>]>) {
        let active = self
            .active
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !*active {
            return;
        }
        // Proxy attachment is performed by ProviderClock, which retains the
        // Arc needed to create weak health bindings without a back-reference.
        self.advance(&self.providers);
        if proxies.is_some() {
            self.advance(&self.groups);
        } else {
            self.advance(&self.rules);
        }
    }
}
pub(crate) struct ProviderClock(pub Arc<ObservationClock>);
impl meow_config::observation::ProviderObserver for ProviderClock {
    fn updated(&self, proxies: Option<&[Arc<dyn Proxy>]>) {
        if let Some(proxies) = proxies {
            for proxy in proxies {
                self.0.bind(proxy);
            }
        }
        meow_config::observation::ProviderObserver::updated(self.0.as_ref(), proxies);
    }
}
