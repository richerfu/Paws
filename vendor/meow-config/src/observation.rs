//! Per-provider runtime authority. A refresh retains its captured owner across
//! acquisition and publication; retiring joins that work before the next VPN.
use meow_common::Proxy;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Weak,
};

pub trait ProviderObserver: Send + Sync {
    /// Called once after a real publication. Proxy members are borrowed only
    /// while attaching metadata observers; no roster is retained or serialized.
    fn updated(&self, proxies: Option<&[Arc<dyn Proxy>]>);
}
#[derive(Default)]
pub(crate) struct ProviderObservation {
    binding: Arc<tokio::sync::RwLock<Binding>>,
    cancelled: Arc<AtomicBool>,
    cancellation: Arc<tokio::sync::Notify>,
}
#[derive(Clone)]
pub(crate) struct ProviderLease {
    binding: Arc<tokio::sync::OwnedRwLockReadGuard<Binding>>,
    cancelled: Arc<AtomicBool>,
    cancellation: Arc<tokio::sync::Notify>,
}
#[derive(Default)]
pub(crate) struct Binding {
    observer: Option<Weak<dyn ProviderObserver>>,
    retired: bool,
}
impl ProviderObservation {
    pub async fn bind(&self, observer: Weak<dyn ProviderObserver>) -> Result<(), String> {
        let mut binding = self.binding.write().await;
        if binding.retired
            || binding
                .observer
                .as_ref()
                .is_some_and(|old| !Weak::ptr_eq(old, &observer))
        {
            return Err("provider belongs to another or retired runtime; reload its source".into());
        }
        binding.observer = Some(observer);
        Ok(())
    }
    pub async fn enter(&self) -> Result<ProviderLease, String> {
        let binding = self.binding.clone().read_owned().await;
        if self.cancelled.load(Ordering::Acquire)
            || binding.retired
            || binding
                .observer
                .as_ref()
                .is_some_and(|observer| observer.upgrade().is_none())
        {
            return Err("provider runtime is retired".into());
        }
        Ok(ProviderLease {
            binding: Arc::new(binding),
            cancelled: self.cancelled.clone(),
            cancellation: self.cancellation.clone(),
        })
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.cancellation.notify_waiters();
    }
    pub async fn retire(&self) {
        self.cancel();
        self.binding.write().await.retired = true;
    }
}
impl Binding {
    pub fn updated(&self, proxies: Option<&[Arc<dyn Proxy>]>) {
        if let Some(observer) = self.observer.as_ref().and_then(Weak::upgrade) {
            observer.updated(proxies);
        }
    }
}

impl ProviderLease {
    pub fn check(&self) -> Result<(), String> {
        if self.cancelled.load(Ordering::Acquire) {
            Err("provider runtime is retiring".into())
        } else {
            Ok(())
        }
    }
    pub async fn cancelled(&self) {
        let notified = self.cancellation.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if self.check().is_err() {
            return;
        }
        notified.await;
    }
    pub fn updated(&self, proxies: Option<&[Arc<dyn Proxy>]>) {
        self.binding.updated(proxies);
    }
}
