use crate::locale::UiLocale;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

const PREFERENCES_FILE: &str = "ui-preferences.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LanguagePreference {
    #[default]
    System,
    ZhCn,
    En,
}

impl LanguagePreference {
    pub(crate) fn resolve(self, system_language: &str) -> UiLocale {
        match self {
            Self::System => UiLocale::from_language_tag(system_language),
            Self::ZhCn => UiLocale::ZhCn,
            Self::En => UiLocale::En,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemePreference {
    pub(crate) fn resolve_dark(self, system_color_mode: i32) -> bool {
        match self {
            Self::System => system_color_mode == 0,
            Self::Light => false,
            Self::Dark => true,
        }
    }

    /// HarmonyOS `ConfigurationConstant.ColorMode` value.
    pub(crate) const fn platform_color_mode(self) -> i32 {
        match self {
            Self::System => -1,
            Self::Dark => 0,
            Self::Light => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct UiPreferences {
    pub(crate) language: LanguagePreference,
    pub(crate) theme: ThemePreference,
}

impl UiPreferences {
    pub(crate) fn load() -> Self {
        preferences_path()
            .and_then(|path| Self::load_from(&path).ok())
            .unwrap_or_default()
    }

    pub(crate) fn save(&self) -> Result<(), String> {
        let path = preferences_path()
            .ok_or_else(|| "PAWS_HOME is not configured for UI preferences".to_owned())?;
        self.save_to(&path)
    }

    pub(crate) fn load_from(path: &Path) -> Result<Self, String> {
        let content = fs::read_to_string(path)
            .map_err(|err| format!("read UI preferences {} failed: {err}", path.display()))?;
        serde_json::from_str(&content)
            .map_err(|err| format!("parse UI preferences {} failed: {err}", path.display()))
    }

    pub(crate) fn save_to(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                format!(
                    "create UI preferences directory {} failed: {err}",
                    parent.display()
                )
            })?;
        }
        let content = serde_json::to_vec_pretty(self)
            .map_err(|err| format!("serialize UI preferences failed: {err}"))?;
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, content).map_err(|err| {
            format!(
                "write UI preferences temporary file {} failed: {err}",
                temporary.display()
            )
        })?;
        fs::rename(&temporary, path).map_err(|err| {
            let _ = fs::remove_file(&temporary);
            format!("replace UI preferences {} failed: {err}", path.display())
        })
    }
}

fn preferences_path() -> Option<PathBuf> {
    std::env::var_os("PAWS_HOME")
        .map(PathBuf::from)
        .map(|home| home.join(PREFERENCES_FILE))
}
