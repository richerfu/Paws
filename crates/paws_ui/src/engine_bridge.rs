//! Platform-only bindings. Engine selection and native execution belong to Core.
use napi_derive_ohos::napi;
use napi_ohos::bindgen_prelude::{FnArgs, Function};
use napi_ohos::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode};
use napi_ohos::{Result, Status};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

type UidCallback = ThreadsafeFunction<
    FnArgs<(i32, String, String)>,
    i32,
    FnArgs<(i32, String, String)>,
    Status,
    false,
    true,
    32,
>;
type PackageCallback = ThreadsafeFunction<i32, String, i32, Status, false, true, 32>;

struct PlatformProcessResolver {
    uid: UidCallback,
    package: PackageCallback,
    active: AtomicBool,
    owner_thread: std::thread::ThreadId,
    token: u32,
}
impl PlatformProcessResolver {
    fn can_dispatch(&self) -> bool {
        self.active.load(Ordering::Acquire) && std::thread::current().id() != self.owner_thread
    }
}
impl paws_core::ProcessResolver for PlatformProcessResolver {
    fn resolve_uid(&self, protocol: i32, source: &str, target: &str) -> Option<i32> {
        if !self.can_dispatch() {
            return None;
        }
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let status = self.uid.call_with_return_value(
            (protocol, source.into(), target.into()).into(),
            ThreadsafeFunctionCallMode::NonBlocking,
            move |result, _| {
                let _ = sender.send(result.ok());
                Ok(())
            },
        );
        if status != Status::Ok {
            return None;
        }
        receiver
            .recv_timeout(Duration::from_millis(150))
            .ok()
            .flatten()
            .filter(|uid| *uid >= 0 && self.active.load(Ordering::Acquire))
    }
    fn resolve_package(&self, uid: i32) -> Option<String> {
        if !self.can_dispatch() {
            return None;
        }
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let status = self.package.call_with_return_value(
            uid,
            ThreadsafeFunctionCallMode::NonBlocking,
            move |result, _| {
                let _ = sender.send(result.ok());
                Ok(())
            },
        );
        if status != Status::Ok {
            return None;
        }
        receiver
            .recv_timeout(Duration::from_millis(150))
            .ok()
            .flatten()
            .filter(|name| !name.is_empty() && self.active.load(Ordering::Acquire))
    }
}
static RESOLVER_TOKEN: AtomicU32 = AtomicU32::new(0);
static RESOLVER: OnceLock<Mutex<Option<Arc<PlatformProcessResolver>>>> = OnceLock::new();

#[napi]
pub fn register_engine_process_resolver(
    uid: Function<'_, FnArgs<(i32, String, String)>, i32>,
    package: Function<'_, i32, String>,
) -> Result<u32> {
    let resolver = Arc::new(PlatformProcessResolver {
        uid: uid
            .build_threadsafe_function()
            .weak::<true>()
            .max_queue_size::<32>()
            .build()?,
        package: package
            .build_threadsafe_function()
            .weak::<true>()
            .max_queue_size::<32>()
            .build()?,
        active: AtomicBool::new(true),
        owner_thread: std::thread::current().id(),
        token: RESOLVER_TOKEN
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1),
    });
    let mut current = RESOLVER.get_or_init(Default::default).lock().map_err(|_| {
        napi_ohos::Error::new(Status::GenericFailure, "process resolver lock poisoned")
    })?;
    if let Some(previous) = current.replace(resolver.clone()) {
        previous.active.store(false, Ordering::Release);
    }
    let token = resolver.token;
    paws_core::register_engine_process_resolver(Some(resolver));
    Ok(token)
}
#[napi]
pub fn clear_engine_process_resolver(token: u32) {
    if let Ok(mut current) = RESOLVER.get_or_init(Default::default).lock() {
        if current
            .as_ref()
            .is_none_or(|resolver| resolver.token != token)
        {
            return;
        }
        if let Some(previous) = current.take() {
            previous.active.store(false, Ordering::Release);
        }
        paws_core::register_engine_process_resolver(None);
    }
}
#[napi]
pub fn engine_tun_mtu(options_json: String) -> Result<u32> {
    paws_core::shared_core()
        .engine_tun_mtu(&options_json)
        .map(u32::from)
        .map_err(super::to_napi_error)
}
#[napi]
pub async fn prepare_engine(options_json: String, attempt_id: String) -> Result<String> {
    paws_core::shared_core()
        .prepare_engine(&options_json, &attempt_id)
        .await
        .map_err(super::to_napi_error)
}
#[napi]
pub async fn start_engine(
    fd: i32,
    underlying_net_id: i32,
    options_json: String,
    attempt_id: String,
    plan_json: String,
) -> Result<()> {
    paws_core::shared_core()
        .start_engine(
            fd,
            underlying_net_id,
            &options_json,
            &attempt_id,
            &plan_json,
        )
        .await
        .map_err(super::to_napi_error)
}
#[napi]
pub async fn stop_engine(attempt_id: String) -> Result<()> {
    paws_core::shared_core()
        .stop_engine(&attempt_id)
        .await
        .map_err(super::to_napi_error)
}
#[napi]
pub async fn reload_engine(
    options_json: String,
    attempt_id: String,
    plan_json: String,
    underlying_net_id: i32,
) -> Result<()> {
    paws_core::shared_core()
        .reload_engine(&options_json, &attempt_id, &plan_json, underlying_net_id)
        .await
        .map_err(super::to_napi_error)
}
#[napi]
pub async fn poll_engine_traffic() -> Result<()> {
    paws_core::shared_core()
        .poll_engine_traffic()
        .await
        .map_err(super::to_napi_error)
}
#[napi]
pub async fn poll_engine_observations() -> Result<()> {
    paws_core::shared_core()
        .poll_engine_observations()
        .await
        .map_err(super::to_napi_error)
}
#[napi]
pub fn trim_engine_memory() -> Result<bool> {
    let core = paws_core::shared_core();
    // Absence of a trim capability is reported to the host as `false`.
    // Backend failures remain errors and never count as a completed trim.
    if !core
        .engine_can_trim_memory()
        .map_err(super::to_napi_error)?
    {
        return Ok(false);
    }
    core.trim_engine_memory()
        .map(|_| true)
        .map_err(super::to_napi_error)
}
