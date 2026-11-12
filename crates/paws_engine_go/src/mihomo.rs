use paws_engine::{EngineStart, EngineTraffic, ProcessResolver};
use std::sync::{Arc, OnceLock, RwLock};

static PROCESS_RESOLVER: OnceLock<RwLock<Option<Arc<dyn ProcessResolver>>>> = OnceLock::new();
pub(crate) fn register_process_resolver(resolver: Option<Arc<dyn ProcessResolver>>) {
    if let Ok(mut slot) = PROCESS_RESOLVER.get_or_init(Default::default).write() {
        *slot = resolver;
    }
}

#[cfg(target_env = "ohos")]
mod native {
    use super::*;
    use std::ffi::{c_char, c_int, c_void, CStr, CString};
    use std::sync::atomic::{AtomicI32, Ordering};
    use std::sync::Mutex;

    type Setup = unsafe extern "C" fn(*mut c_void, *mut c_char, *mut c_char);
    type StartTun =
        unsafe extern "C" fn(*mut c_void, c_int, *mut c_char, *mut c_char, *mut c_char) -> bool;
    type VoidCall = unsafe extern "C" fn();
    type Stop = unsafe extern "C" fn() -> *mut c_char;
    type CopyLog = unsafe extern "C" fn(*mut c_char, usize) -> usize;
    type Traffic = unsafe extern "C" fn(bool) -> *mut c_char;
    struct Api {
        _handle: *mut c_void,
        setup: Setup,
        start: StartTun,
        stop: Stop,
        traffic: Traffic,
        total: Traffic,
        trim: VoidCall,
        copy_log: CopyLog,
    }
    unsafe impl Send for Api {}
    unsafe impl Sync for Api {}
    static API: OnceLock<Result<Api, String>> = OnceLock::new();
    static PHYSICAL_NETWORK: OnceLock<Arc<AtomicI32>> = OnceLock::new();
    fn physical_network() -> Arc<AtomicI32> {
        PHYSICAL_NETWORK
            .get_or_init(|| Arc::new(AtomicI32::new(0)))
            .clone()
    }

    struct Callback {
        result: Mutex<Option<tokio::sync::oneshot::Sender<String>>>,
        resolver: Option<Arc<dyn ProcessResolver>>,
    }
    unsafe extern "C" fn release(pointer: *mut c_void) {
        if !pointer.is_null() {
            unsafe { drop(Box::from_raw(pointer.cast::<Callback>())) };
        }
    }
    unsafe extern "C" fn free_string(pointer: *mut c_char) {
        unsafe { libc::free(pointer.cast()) };
    }
    unsafe extern "C" fn result(pointer: *mut c_void, value: *const c_char) {
        if pointer.is_null() {
            return;
        }
        let callback = unsafe { &*pointer.cast::<Callback>() };
        let value = if value.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(value) }
                .to_string_lossy()
                .into_owned()
        };
        if let Ok(mut sender) = callback.result.lock() {
            if let Some(sender) = sender.take() {
                let _ = sender.send(value);
            }
        }
    }
    #[repr(C)]
    struct NetHandle {
        net_id: i32,
    }
    #[link(name = "net_connection")]
    unsafe extern "C" {
        fn OH_NetConn_BindSocket(fd: i32, handle: *mut NetHandle) -> i32;
    }
    unsafe extern "C" fn protect(_: *mut c_void, fd: c_int) -> c_int {
        let net_id = physical_network().load(Ordering::Acquire);
        if net_id <= 0 {
            return 0;
        }
        i32::from(unsafe { OH_NetConn_BindSocket(fd, &mut NetHandle { net_id }) } == 0)
    }
    unsafe extern "C" fn resolve_uid(
        pointer: *mut c_void,
        protocol: c_int,
        source: *const c_char,
        target: *const c_char,
    ) -> c_int {
        if pointer.is_null() || source.is_null() || target.is_null() {
            return -1;
        }
        let callback = unsafe { &*pointer.cast::<Callback>() };
        let Some(resolver) = &callback.resolver else {
            return -1;
        };
        let source = unsafe { CStr::from_ptr(source) }.to_string_lossy();
        let target = unsafe { CStr::from_ptr(target) }.to_string_lossy();
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            resolver.resolve_uid(protocol, &source, &target)
        }))
        .ok()
        .flatten()
        .unwrap_or(-1)
    }
    unsafe extern "C" fn resolve_package(pointer: *mut c_void, uid: c_int) -> *mut c_char {
        if pointer.is_null() {
            return std::ptr::null_mut();
        }
        let callback = unsafe { &*pointer.cast::<Callback>() };
        let value = callback.resolver.as_ref().and_then(|resolver| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                resolver.resolve_package(uid)
            }))
            .ok()
            .flatten()
        });
        let Some(value) = value.and_then(|value| CString::new(value).ok()) else {
            return std::ptr::null_mut();
        };
        unsafe { libc::strdup(value.as_ptr()) }
    }
    unsafe fn symbol<T: Copy>(handle: *mut c_void, name: &[u8]) -> Result<T, String> {
        let pointer = unsafe { libc::dlsym(handle, name.as_ptr().cast()) };
        if pointer.is_null() {
            return Err(format!(
                "missing mihomo bridge symbol {}",
                String::from_utf8_lossy(&name[..name.len() - 1])
            ));
        }
        Ok(unsafe { std::mem::transmute_copy(&pointer) })
    }
    fn api() -> Result<&'static Api, String> {
        API.get_or_init(|| unsafe {
            let handle = libc::dlopen(c"libclash.so".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
            if handle.is_null() {
                return Err("cannot load libclash.so".into());
            }
            let loaded = (|| {
                let api = Api {
                    _handle: handle,
                    setup: symbol(handle, b"quickSetup\0")?,
                    start: symbol(handle, b"startTUN\0")?,
                    stop: symbol(handle, b"TinyPawsMihomoStop\0")?,
                    traffic: symbol(handle, b"getTraffic\0")?,
                    total: symbol(handle, b"getTotalTraffic\0")?,
                    trim: symbol(handle, b"TinyPawsMihomoTrimMemory\0")?,
                    copy_log: symbol(handle, b"TinyPawsMihomoCopyLog\0")?,
                };
                // These are C bridge function-pointer variables, not functions.
                *symbol::<*mut Option<unsafe extern "C" fn(*mut c_void)>>(
                    handle,
                    b"release_object_func\0",
                )? = Some(release);
                *symbol::<*mut Option<unsafe extern "C" fn(*mut c_char)>>(
                    handle,
                    b"free_string_func\0",
                )? = Some(free_string);
                *symbol::<*mut Option<unsafe extern "C" fn(*mut c_void, *const c_char)>>(
                    handle,
                    b"result_func\0",
                )? = Some(result);
                *symbol::<*mut Option<unsafe extern "C" fn(*mut c_void, c_int) -> c_int>>(
                    handle,
                    b"protect_func\0",
                )? = Some(protect);
                *symbol::<
                    *mut Option<
                        unsafe extern "C" fn(
                            *mut c_void,
                            c_int,
                            *const c_char,
                            *const c_char,
                        ) -> c_int,
                    >,
                >(handle, b"resolve_uid_func\0")? = Some(resolve_uid);
                *symbol::<*mut Option<unsafe extern "C" fn(*mut c_void, c_int) -> *mut c_char>>(
                    handle,
                    b"resolve_package_func\0",
                )? = Some(resolve_package);
                Ok(api)
            })();
            if loaded.is_err() {
                libc::dlclose(handle);
            }
            loaded
        })
        .as_ref()
        .map_err(Clone::clone)
    }
    fn owned_string(value: &str) -> Result<*mut c_char, String> {
        let value = CString::new(value).map_err(|_| "mihomo argument contains NUL".to_owned())?;
        let owned = unsafe { libc::strdup(value.as_ptr()) };
        if owned.is_null() {
            Err("cannot allocate mihomo argument".into())
        } else {
            Ok(owned)
        }
    }
    async fn setup(init: &str, setup: &str) -> Result<(), String> {
        let api = api()?;
        let init = owned_string(init)?;
        let setup = match owned_string(setup) {
            Ok(value) => value,
            Err(error) => {
                unsafe { libc::free(init.cast()) };
                return Err(error);
            }
        };
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let callback = Box::into_raw(Box::new(Callback {
            result: Mutex::new(Some(sender)),
            resolver: None,
        }));
        // Go consumes both malloc strings and eventually releases the callback,
        // including after timeout/cancellation of this Rust future.
        unsafe { (api.setup)(callback.cast(), init, setup) };
        let error = receiver
            .await
            .map_err(|_| "mihomo setup callback closed".to_owned())?;
        if error.is_empty() {
            Ok(())
        } else {
            Err(error)
        }
    }
    pub async fn start(request: &EngineStart, init: &str, config: &str) -> Result<(), String> {
        physical_network().store(request.underlying_network, Ordering::Release);
        setup(init, config).await?;
        let options = &request.options;
        let stack = match options.stack.as_str() {
            "gvisor" => "gvisor",
            "system" => "system",
            _ => "mixed",
        };
        let addresses = if options.addresses.is_empty() {
            "172.19.0.1/30".into()
        } else {
            options.addresses.join(",")
        };
        let dns = if options.dns_hijacking {
            "0.0.0.0".into()
        } else if options.dns_addresses.is_empty() {
            "172.19.0.2".into()
        } else {
            options.dns_addresses.join(",")
        };
        let stack = owned_string(stack)?;
        let addresses = match owned_string(&addresses) {
            Ok(value) => value,
            Err(error) => {
                unsafe { libc::free(stack.cast()) };
                return Err(error);
            }
        };
        let dns = match owned_string(&dns) {
            Ok(value) => value,
            Err(error) => {
                unsafe {
                    libc::free(stack.cast());
                    libc::free(addresses.cast());
                };
                return Err(error);
            }
        };
        let resolver = PROCESS_RESOLVER
            .get()
            .and_then(|slot| slot.read().ok().and_then(|slot| slot.clone()));
        let callback = Box::into_raw(Box::new(Callback {
            result: Mutex::new(None),
            resolver,
        }));
        if unsafe { (api()?.start)(callback.cast(), request.tun_fd, stack, addresses, dns) } {
            Ok(())
        } else {
            Err("mihomo rejected the platform TUN descriptor".into())
        }
    }
    pub async fn reload(init: &str, config: &str, net_id: i32) -> Result<(), String> {
        physical_network().store(net_id, Ordering::Release);
        setup(init, config).await
    }
    pub fn stop() -> Result<(), String> {
        let error = unsafe { (api()?.stop)() };
        if error.is_null() {
            return Ok(());
        }
        let message = unsafe { CStr::from_ptr(error) }
            .to_string_lossy()
            .into_owned();
        unsafe { libc::free(error.cast()) };
        Err(message)
    }
    pub fn runtime_log() -> Result<String, String> {
        const MAX_LOG_BYTES: usize = 1024 * 1024;
        let api = api()?;
        let required = unsafe { (api.copy_log)(std::ptr::null_mut(), 0) };
        if required > MAX_LOG_BYTES {
            return Err("mihomo log journal exceeds its byte budget".into());
        }
        let mut bytes = vec![0_u8; required.saturating_add(1)];
        let actual = unsafe { (api.copy_log)(bytes.as_mut_ptr().cast(), bytes.len()) };
        if actual >= bytes.len() {
            return Err("mihomo log changed while copying; retry the bounded journal".into());
        }
        bytes.truncate(actual);
        String::from_utf8(bytes).map_err(|_| "mihomo returned an invalid UTF-8 log journal".into())
    }
    fn counters(call: Traffic) -> Result<serde_json::Value, String> {
        let value = unsafe { call(false) };
        if value.is_null() {
            return Err("mihomo returned no traffic sample".into());
        }
        let data = unsafe { CStr::from_ptr(value) }.to_bytes();
        let parsed = serde_json::from_slice(data)
            .map_err(|error| format!("invalid mihomo traffic: {error}"));
        unsafe { libc::free(value.cast()) };
        parsed
    }
    pub fn traffic() -> Result<EngineTraffic, String> {
        let api = api()?;
        let speed = counters(api.traffic)?;
        let total = counters(api.total)?;
        let counter = |value: &serde_json::Value, field: &str| {
            value
                .get(field)
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| format!("mihomo traffic missing {field}"))
        };
        Ok(EngineTraffic {
            upload_speed: counter(&speed, "up")?,
            download_speed: counter(&speed, "down")?,
            upload_total: counter(&total, "up")?,
            download_total: counter(&total, "down")?,
        })
    }
    pub fn version() -> Result<String, String> {
        api()?;
        Ok("mihomo 1.10.0 (pinned bridge)".into())
    }
    pub fn trim_memory() -> Result<(), String> {
        unsafe { (api()?.trim)() };
        Ok(())
    }
}
#[cfg(not(target_env = "ohos"))]
mod native {
    use super::*;
    pub async fn start(_: &EngineStart, _: &str, _: &str) -> Result<(), String> {
        Err("mihomo requires OpenHarmony".into())
    }
    pub async fn reload(_: &str, _: &str, _: i32) -> Result<(), String> {
        Err("mihomo requires OpenHarmony".into())
    }
    pub fn stop() -> Result<(), String> {
        Err("mihomo requires OpenHarmony".into())
    }
    pub fn traffic() -> Result<EngineTraffic, String> {
        Err("mihomo requires OpenHarmony".into())
    }
    pub fn version() -> Result<String, String> {
        Err("mihomo requires OpenHarmony".into())
    }
    pub fn trim_memory() -> Result<(), String> {
        Err("mihomo requires OpenHarmony".into())
    }
    pub fn runtime_log() -> Result<String, String> {
        Err("mihomo requires OpenHarmony".into())
    }
}
pub(crate) use native::{reload, runtime_log, start, stop, traffic, trim_memory, version};
