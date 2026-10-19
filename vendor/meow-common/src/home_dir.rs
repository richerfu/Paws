//! Process-wide meow home directory.
//!
//! `set_home_dir` must be called once at startup (from `-d` CLI flag) before
//! any of the path helpers in `meow-config` or `meow-rules` are invoked.
//! After that, all resource-path helpers (`default_geoip_path`,
//! `default_asn_path`, `default_geosite_candidates`, …) resolve under this
//! directory instead of the XDG / `$HOME/.config` fallback.

use std::path::PathBuf;

static MEOW_HOME_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Set the process-wide meow home directory from the `-d` CLI flag.
///
/// First write wins; subsequent calls are silently ignored (idempotent).
/// Must be called before any path helper that delegates to `meow_home_dir`.
pub fn set_home_dir(dir: PathBuf) {
    let _ = MEOW_HOME_DIR.set(dir);
}

/// Return the stored home directory, or `None` if `set_home_dir` was never
/// called.  Callers should fall back to the XDG / `$HOME/.config` chain when
/// this returns `None`.
pub fn meow_home_dir() -> Option<PathBuf> {
    MEOW_HOME_DIR.get().cloned()
}

#[cfg(test)]
mod tests {
    // OnceLock is process-global, so we cannot meaningfully test set/get in
    // isolation across parallel test threads.  The compile-time check below is
    // enough to confirm the API surface is correct.
    use super::*;
    use std::path::PathBuf;

    #[allow(dead_code, reason = "compile-time shape check, never executed")]
    fn _shape_check() {
        set_home_dir(PathBuf::from("/tmp/meow"));
        let _: Option<PathBuf> = meow_home_dir();
    }
}
