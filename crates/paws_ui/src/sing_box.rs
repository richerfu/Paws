#[derive(Default)]
pub(crate) struct TrafficSample {
    pub upload_speed: u64,
    pub download_speed: u64,
    pub upload_total: u64,
    pub download_total: u64,
}

#[cfg(target_env = "ohos")]
mod platform {
    use super::TrafficSample;
    use std::ffi::{c_char, c_int, c_void, CString};
    use std::sync::OnceLock;

    type StartFn = unsafe extern "C" fn(c_int, c_int, *const c_char, *const c_char) -> c_int;
    type StopFn = unsafe extern "C" fn() -> c_int;
    type ErrorFn = unsafe extern "C" fn(*mut c_char, usize) -> usize;
    type VersionFn = unsafe extern "C" fn() -> *const c_char;
    type TrafficFn = unsafe extern "C" fn(*mut u64, *mut u64, *mut u64, *mut u64) -> c_int;

    struct Api {
        _handle: *mut c_void,
        start: StartFn,
        stop: StopFn,
        copy_error: ErrorFn,
        copy_log: ErrorFn,
        version: VersionFn,
        traffic: TrafficFn,
    }

    unsafe impl Send for Api {}
    unsafe impl Sync for Api {}

    static API: OnceLock<Result<Api, String>> = OnceLock::new();

    unsafe fn symbol<T: Copy>(handle: *mut c_void, name: &'static [u8]) -> Result<T, String> {
        let pointer = unsafe { libc::dlsym(handle, name.as_ptr().cast()) };
        if pointer.is_null() {
            return Err(format!(
                "missing sing-box bridge symbol {}",
                String::from_utf8_lossy(&name[..name.len().saturating_sub(1)])
            ));
        }
        Ok(unsafe { std::mem::transmute_copy::<*mut c_void, T>(&pointer) })
    }

    fn api() -> Result<&'static Api, String> {
        API.get_or_init(|| unsafe {
            let library = b"libsingbox.so\0";
            let handle = libc::dlopen(library.as_ptr().cast(), libc::RTLD_NOW | libc::RTLD_LOCAL);
            if handle.is_null() {
                return Err("cannot load libsingbox.so".to_owned());
            }
            Ok(Api {
                _handle: handle,
                start: symbol(handle, b"TinyPawsSingStart\0")?,
                stop: symbol(handle, b"TinyPawsSingStop\0")?,
                copy_error: symbol(handle, b"TinyPawsSingCopyLastError\0")?,
                copy_log: symbol(handle, b"TinyPawsSingCopyLog\0")?,
                version: symbol(handle, b"TinyPawsSingVersion\0")?,
                traffic: symbol(handle, b"TinyPawsSingTraffic\0")?,
            })
        })
        .as_ref()
        .map_err(Clone::clone)
    }

    fn last_error(api: &Api) -> String {
        let required = unsafe { (api.copy_error)(std::ptr::null_mut(), 0) };
        let mut buffer = vec![0u8; required.saturating_add(1).max(2)];
        unsafe {
            (api.copy_error)(buffer.as_mut_ptr().cast(), buffer.len());
        }
        let length = buffer
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(buffer.len());
        String::from_utf8_lossy(&buffer[..length]).into_owned()
    }

    pub fn start(fd: i32, net_id: i32, config_json: &str, work_dir: &str) -> Result<(), String> {
        let api = api()?;
        let config =
            CString::new(config_json).map_err(|_| "sing-box config contains NUL".to_owned())?;
        let work_dir =
            CString::new(work_dir).map_err(|_| "sing-box work path contains NUL".to_owned())?;
        let result = unsafe { (api.start)(fd, net_id, config.as_ptr(), work_dir.as_ptr()) };
        if result == 0 {
            Ok(())
        } else {
            Err(last_error(api))
        }
    }

    pub fn stop() -> Result<(), String> {
        let api = api()?;
        let result = unsafe { (api.stop)() };
        if result == 0 {
            Ok(())
        } else {
            Err(last_error(api))
        }
    }

    pub fn traffic() -> Result<TrafficSample, String> {
        let api = api()?;
        let mut sample = TrafficSample::default();
        let result = unsafe {
            (api.traffic)(
                &mut sample.upload_speed,
                &mut sample.download_speed,
                &mut sample.upload_total,
                &mut sample.download_total,
            )
        };
        if result != 0 {
            return Err("sing-box bridge could not read traffic counters".to_owned());
        }
        Ok(sample)
    }

    pub fn version() -> Result<String, String> {
        let api = api()?;
        let pointer = unsafe { (api.version)() };
        if pointer.is_null() {
            return Err("sing-box bridge returned an empty version".to_owned());
        }
        Ok(unsafe { std::ffi::CStr::from_ptr(pointer) }
            .to_string_lossy()
            .into_owned())
    }

    pub fn runtime_log() -> Result<String, String> {
        let api = api()?;
        let required = unsafe { (api.copy_log)(std::ptr::null_mut(), 0) };
        let mut buffer = vec![0u8; required.saturating_add(1).max(2)];
        unsafe {
            (api.copy_log)(buffer.as_mut_ptr().cast(), buffer.len());
        }
        let length = buffer
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(buffer.len());
        Ok(String::from_utf8_lossy(&buffer[..length]).into_owned())
    }
}

#[cfg(not(target_env = "ohos"))]
mod platform {
    use super::TrafficSample;

    pub fn start(
        _fd: i32,
        _net_id: i32,
        _config_json: &str,
        _work_dir: &str,
    ) -> Result<(), String> {
        Err("sing-box bridge is only available in the OpenHarmony build".to_owned())
    }
    pub fn stop() -> Result<(), String> {
        Err("sing-box bridge is only available in the OpenHarmony build".to_owned())
    }
    pub fn traffic() -> Result<TrafficSample, String> {
        Err("sing-box bridge is only available in the OpenHarmony build".to_owned())
    }
    pub fn version() -> Result<String, String> {
        Ok("sing-box bridge unavailable on host".to_owned())
    }
    pub fn runtime_log() -> Result<String, String> {
        Ok(String::new())
    }
}

pub fn start(fd: i32, net_id: i32, config_json: &str, work_dir: &str) -> Result<(), String> {
    platform::start(fd, net_id, config_json, work_dir)
}
pub fn stop() -> Result<(), String> {
    platform::stop()
}
pub fn version() -> Result<String, String> {
    platform::version()
}
pub fn runtime_log() -> Result<String, String> {
    platform::runtime_log()
}

pub(crate) fn traffic() -> Result<TrafficSample, String> {
    platform::traffic()
}
