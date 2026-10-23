use paws_engine::{EngineError, EngineMemory, EngineMemoryScope};
use paws_model::VpnEngine;

pub(crate) struct ProcessMemory;
impl ProcessMemory {
    pub fn sample(&self) -> Result<EngineMemory, EngineError> {
        self.resident_bytes()
            .map(|in_use_bytes| EngineMemory {
                scope: EngineMemoryScope::ProcessResident,
                in_use_bytes,
                limit_bytes: None,
            })
            .map_err(|message| EngineError::Backend {
                engine: VpnEngine::Meow,
                operation: "process memory",
                message,
            })
    }
    #[cfg(target_os = "macos")]
    fn resident_bytes(&self) -> Result<u64, String> {
        let mut info = std::mem::MaybeUninit::<libc::mach_task_basic_info>::zeroed();
        let mut count = libc::MACH_TASK_BASIC_INFO_COUNT;
        #[allow(
            deprecated,
            reason = "libc exposes the stable Mach task port ABI; no separate platform dependency is needed"
        )]
        let task = unsafe { libc::mach_task_self_ };
        // task_info writes exactly the declared kernel ABI struct. resident_size
        // is current RSS; getrusage.ru_maxrss would instead be a high watermark.
        let status = unsafe {
            libc::task_info(
                task,
                libc::MACH_TASK_BASIC_INFO,
                info.as_mut_ptr().cast(),
                &mut count,
            )
        };
        if status != 0 {
            return Err(format!("task_info returned {status}"));
        }
        Ok(unsafe { info.assume_init() }.resident_size)
    }
    #[cfg(any(target_os = "linux", target_env = "ohos"))]
    fn resident_bytes(&self) -> Result<u64, String> {
        let status =
            std::fs::read_to_string("/proc/self/status").map_err(|error| error.to_string())?;
        parse_resident_bytes(&status).ok_or_else(|| "process status missing valid VmRSS".into())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_env = "ohos")))]
    fn resident_bytes(&self) -> Result<u64, String> {
        Err("current resident memory is unavailable on this platform".into())
    }
}
#[cfg(any(target_os = "linux", target_env = "ohos", test))]
fn parse_resident_bytes(status: &str) -> Option<u64> {
    let mut values = status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))?
        .split_whitespace();
    let kib = values.next()?.parse::<u64>().ok()?;
    (values.next()? == "kB")
        .then(|| kib.checked_mul(1024))
        .flatten()
}
#[cfg(test)]
mod tests {
    #[test]
    fn resident_is_not_peak_and_invalid_units_are_rejected() {
        assert_eq!(
            super::parse_resident_bytes("VmHWM: 999 kB\nVmRSS: 12 kB\n"),
            Some(12288)
        );
        assert_eq!(super::parse_resident_bytes("VmRSS: 12 bytes"), None);
        assert_eq!(super::parse_resident_bytes("VmHWM: 999 kB"), None);
    }
}
