use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

use crate::AppError;

#[derive(Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    #[default]
    System,
    ZhCn,
    En,
}

#[derive(Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct StoredPreferences {
    advanced: bool,
    language: Language,
    theme: Theme,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreferencesPatch {
    advanced: Option<bool>,
    language: Option<Language>,
    theme: Option<Theme>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreferencesSnapshot {
    pub advanced: bool,
    pub language: Language,
    pub theme: Theme,
    pub system_locale: String,
    pub system_color_mode: i32,
}

struct SystemPreferences {
    locale: String,
    color_mode: i32,
}

static SYSTEM: LazyLock<Mutex<SystemPreferences>> = LazyLock::new(|| {
    Mutex::new(SystemPreferences {
        locale: "zh-CN".into(),
        color_mode: 1,
    })
});
static STORAGE_LOCK: Mutex<()> = Mutex::new(());

/// Owns persisted presentation preferences independently of any UI framework.
pub struct PreferencesStore {
    path: PathBuf,
}

impl PreferencesStore {
    pub fn configured() -> Result<Self, AppError> {
        let root = std::env::var_os("PAWS_HOME").ok_or(AppError::InvalidRequest(
            "application home is not configured".into(),
        ))?;
        Ok(Self {
            path: PathBuf::from(root).join("ui-preferences.json"),
        })
    }

    pub fn set_system_locale(locale: String) -> Result<bool, AppError> {
        if locale.trim().is_empty() {
            return Err(AppError::InvalidRequest("empty system locale".into()));
        }
        let mut system = SYSTEM.lock().map_err(|_| AppError::Poisoned)?;
        let changed = system.locale != locale;
        system.locale = locale;
        Ok(changed)
    }

    pub fn set_system_color_mode(mode: i32) -> Result<bool, AppError> {
        if !matches!(mode, -1..=1) {
            return Err(AppError::InvalidRequest("invalid color mode".into()));
        }
        let mut system = SYSTEM.lock().map_err(|_| AppError::Poisoned)?;
        let changed = system.color_mode != mode;
        system.color_mode = mode;
        Ok(changed)
    }

    fn read(&self) -> Result<StoredPreferences, AppError> {
        match std::fs::read(&self.path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(StoredPreferences::default()),
            Err(e) => Err(e.into()),
        }
    }

    fn snapshot_of(value: StoredPreferences) -> Result<PreferencesSnapshot, AppError> {
        let system = SYSTEM.lock().map_err(|_| AppError::Poisoned)?;
        Ok(PreferencesSnapshot {
            advanced: value.advanced,
            language: value.language,
            theme: value.theme,
            system_locale: system.locale.clone(),
            system_color_mode: system.color_mode,
        })
    }

    pub fn snapshot(&self) -> Result<PreferencesSnapshot, AppError> {
        let _guard = STORAGE_LOCK.lock().map_err(|_| AppError::Poisoned)?;
        Self::snapshot_of(self.read()?)
    }

    pub fn update(&self, patch: PreferencesPatch) -> Result<PreferencesSnapshot, AppError> {
        let _guard = STORAGE_LOCK.lock().map_err(|_| AppError::Poisoned)?;
        let mut value = self.read()?;
        if let Some(v) = patch.advanced {
            value.advanced = v;
        }
        if let Some(v) = patch.language {
            value.language = v;
        }
        if let Some(v) = patch.theme {
            value.theme = v;
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temporary = self.path.with_extension("json.tmp");
        std::fs::write(&temporary, serde_json::to_vec_pretty(&value)?)?;
        std::fs::rename(&temporary, &self.path)?;
        Self::snapshot_of(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_updates_preserve_other_preferences_and_corrupt_files_are_not_overwritten() {
        let root = std::env::temp_dir().join(format!(
            "paws-app-preferences-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = PreferencesStore {
            path: root.join("ui-preferences.json"),
        };
        store
            .update(serde_json::from_str(r#"{"language":"en","advanced":true}"#).unwrap())
            .unwrap();
        store
            .update(serde_json::from_str(r#"{"theme":"dark"}"#).unwrap())
            .unwrap();
        let snapshot = serde_json::to_value(store.snapshot().unwrap()).unwrap();
        assert_eq!(snapshot["language"], "en");
        assert_eq!(snapshot["theme"], "dark");
        assert_eq!(snapshot["advanced"], true);
        std::fs::write(&store.path, b"invalid preferences").unwrap();
        assert!(store.update(PreferencesPatch::default()).is_err());
        assert_eq!(std::fs::read(&store.path).unwrap(), b"invalid preferences");
        std::fs::remove_dir_all(root).unwrap();
    }
}
