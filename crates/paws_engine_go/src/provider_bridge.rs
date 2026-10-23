//! Process-lifetime pure Rust conversion callback used by the Go provider
//! transaction. All buffers are length-delimited and allocated/freed by libc.
use crate::config::{
    geo::{self, GeoKind},
    provider::{self, ProviderFormat, ProviderKind, RuleBehavior},
};
use std::ffi::c_void;

pub(crate) type CompileProvider =
    unsafe extern "C" fn(u32, u32, u32, *const u8, usize, *mut usize) -> *mut c_void;

/// The Go bridge validates the input budget too; this second check owns the
/// unsafe Rust slice boundary. A null result is allocation/ABI failure only.
pub(crate) unsafe extern "C" fn compile_provider(
    kind: u32,
    behavior: u32,
    format: u32,
    data: *const u8,
    length: usize,
    result_length: *mut usize,
) -> *mut c_void {
    if result_length.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        *result_length = 0;
    }
    let result = std::panic::catch_unwind(|| {
        let conversion = (|| {
            let limit = if matches!(kind, 3..=5) {
                geo::MAX_INPUT_BYTES
            } else {
                provider::MAX_INPUT_BYTES
            };
            if length > limit || (length > 0 && data.is_null()) {
                return Err("provider input buffer is invalid".to_owned());
            }
            let bytes = if length == 0 {
                &[]
            } else {
                unsafe { std::slice::from_raw_parts(data, length) }
            };
            if kind == 6 {
                if behavior != 0 || format != 1 {
                    return Err("invalid provider declaration compiler mode".into());
                }
                let root: serde_yaml::Value = serde_yaml::from_slice(bytes)
                    .map_err(|_| "invalid provider declarations".to_owned())?;
                crate::config::validate_sing_box_provider_definitions(
                    root.as_mapping()
                        .ok_or_else(|| "provider declarations must be a map".to_owned())?,
                )
                .map_err(|error| error.to_string())?;
                return Ok(serde_json::json!({"rules":[]}));
            }
            if matches!(kind, 3..=5) {
                if behavior != 0 || format != 3 {
                    return Err("invalid geodata compiler mode".into());
                }
                let kind = match kind {
                    3 => GeoKind::Site,
                    4 => GeoKind::Country,
                    _ => GeoKind::Asn,
                };
                return geo::compile(kind, bytes).map_err(|error| error.to_string());
            }
            let kind = match kind {
                1 => ProviderKind::Proxy,
                2 => ProviderKind::Rule,
                _ => return Err("unknown provider kind".into()),
            };
            let behavior = match behavior {
                0 | 1 => RuleBehavior::Domain,
                2 => RuleBehavior::IpCidr,
                3 => RuleBehavior::Classical,
                _ => return Err("unknown provider behavior".into()),
            };
            let format = match format {
                1 => ProviderFormat::Yaml,
                2 => ProviderFormat::Text,
                3 => ProviderFormat::Mrs,
                _ => return Err("unknown provider format".into()),
            };
            provider::compile(kind, behavior, format, bytes).map_err(|error| error.to_string())
        })();
        match conversion {
            Ok(value) => serde_json::to_vec(&serde_json::json!({"ok":true,"data":value})),
            Err(error) => serde_json::to_vec(&serde_json::json!({"ok":false,"error":error})),
        }
    });
    let Ok(Ok(bytes)) = result else {
        return std::ptr::null_mut();
    };
    let bytes = if bytes.len() > provider::MAX_OUTPUT_BYTES {
        br#"{"ok":false,"error":"compiler output exceeds byte budget"}"#.to_vec()
    } else {
        bytes
    };
    let allocated = unsafe { libc::malloc(bytes.len()) };
    if allocated.is_null() {
        return allocated;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), allocated.cast::<u8>(), bytes.len());
        *result_length = bytes.len();
    }
    allocated
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_callback_is_length_delimited_and_buffers_use_the_libc_owner() {
        let bytes = b"proxies: [{name: direct-test, type: direct}]";
        let mut length = 0;
        let callback: CompileProvider = compile_provider;
        let pointer = unsafe { callback(1, 0, 1, bytes.as_ptr(), bytes.len(), &mut length) };
        assert!(!pointer.is_null());
        let value: serde_json::Value = serde_json::from_slice(unsafe {
            std::slice::from_raw_parts(pointer.cast::<u8>(), length)
        })
        .unwrap();
        unsafe { libc::free(pointer) };
        assert_eq!(value["ok"], true);
        assert_eq!(value["data"]["outbounds"][0]["tag"], "direct-test");
        let pointer = unsafe { callback(2, 1, 1, std::ptr::null(), 1, &mut length) };
        let value: serde_json::Value = serde_json::from_slice(unsafe {
            std::slice::from_raw_parts(pointer.cast::<u8>(), length)
        })
        .unwrap();
        unsafe { libc::free(pointer) };
        assert_eq!(value["ok"], false);
    }
}
