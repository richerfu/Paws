//! Platform configuration events, independent of VPN telemetry and UI roots.
use std::sync::LazyLock;
use tokio::sync::watch;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SystemPreferences {
    pub locale: String,
    pub color_mode: i32,
}

static CURRENT: LazyLock<watch::Sender<SystemPreferences>> = LazyLock::new(|| {
    let initial = SystemPreferences {
        locale: std::env::var("PAWS_UI_LOCALE").unwrap_or_else(|_| "zh-CN".to_owned()),
        color_mode: std::env::var("PAWS_SYSTEM_COLOR_MODE")
            .ok()
            .and_then(|value| value.parse().ok())
            .filter(|value| matches!(value, -1..=1))
            .unwrap_or(1),
    };
    watch::channel(initial).0
});

pub(crate) fn current() -> SystemPreferences {
    CURRENT.borrow().clone()
}

pub(crate) fn subscribe() -> watch::Receiver<SystemPreferences> {
    CURRENT.subscribe()
}

pub(crate) fn set_locale(locale: String) {
    CURRENT.send_if_modified(|preferences| {
        if preferences.locale == locale {
            return false;
        }
        preferences.locale = locale;
        true
    });
}

pub(crate) fn set_color_mode(color_mode: i32) {
    CURRENT.send_if_modified(|preferences| {
        if preferences.color_mode == color_mode {
            return false;
        }
        preferences.color_mode = color_mode;
        true
    });
}
