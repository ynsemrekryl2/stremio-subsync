//! Translation settings the dashboard changes (SS-048): the model and its
//! reasoning effort. A saved choice is kept in a small JSON file so restarts
//! and deploys keep it, and it wins over `SUBSYNC_TRANSLATE_MODEL` and
//! `SUBSYNC_TRANSLATE_REASONING`. The file holds no key: the OpenRouter key
//! and the target language stay in the environment.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::openrouter::{self, DEFAULT_MODEL, DEFAULT_REASONING, REASONING_EFFORTS};

/// What `reasoning` holds for "send no effort, the model's own default".
pub const MODEL_DEFAULT: &str = "default";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranslateSettings {
    /// An OpenRouter model id, `vendor/name`.
    pub model: String,
    /// One of [`REASONING_EFFORTS`], or [`MODEL_DEFAULT`].
    pub reasoning: String,
}

/// Why a choice was refused. Safe to log and to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Invalid {
    Model,
    Reasoning,
}

impl TranslateSettings {
    pub fn defaults() -> Self {
        Self {
            model: DEFAULT_MODEL.to_owned(),
            reasoning: DEFAULT_REASONING.to_owned(),
        }
    }

    /// From the environment variables' values; an unset one is the default.
    pub fn from_env(model: Option<String>, reasoning: Option<String>) -> Self {
        let defaults = Self::defaults();
        Self {
            model: model.unwrap_or(defaults.model),
            reasoning: reasoning.unwrap_or(defaults.reasoning),
        }
    }

    pub fn check(&self) -> Result<(), Invalid> {
        if !openrouter::valid_model(&self.model) {
            return Err(Invalid::Model);
        }
        if self.reasoning != MODEL_DEFAULT && !REASONING_EFFORTS.contains(&self.reasoning.as_str())
        {
            return Err(Invalid::Reasoning);
        }
        Ok(())
    }

    /// The effort to send, `None` for the model's own default.
    pub fn effort(&self) -> Option<&str> {
        openrouter::reasoning_setting(Some(&self.reasoning))
    }

    /// Part of the translation key: another effort is another translation;
    /// `default` keeps the key translations had before SS-046.
    pub fn key_label(&self) -> String {
        match self.effort() {
            Some(effort) => format!("{} reasoning={effort}", self.model),
            None => self.model.clone(),
        }
    }
}

/// The saved choice, if there is a valid one at `path`.
pub fn load(path: &Path) -> Result<Option<TranslateSettings>, LoadError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(LoadError::Read(err.kind())),
    };
    let settings: TranslateSettings =
        serde_json::from_slice(&bytes).map_err(|_| LoadError::Malformed)?;
    settings.check().map_err(LoadError::Invalid)?;
    Ok(Some(settings))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadError {
    Read(std::io::ErrorKind),
    Malformed,
    Invalid(Invalid),
}

/// Writes `settings` to `path` whole: a crash leaves the old file or the new
/// one, never half of it.
pub fn save(path: &Path, settings: &TranslateSettings) -> std::io::Result<()> {
    if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_vec_pretty(settings).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(model: &str, reasoning: &str) -> TranslateSettings {
        TranslateSettings {
            model: model.to_owned(),
            reasoning: reasoning.to_owned(),
        }
    }

    fn temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir()
            .join(format!(
                "subsync-settings-{name}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ))
            .join("settings.json")
    }

    #[test]
    fn unset_environment_is_the_default() {
        assert_eq!(
            TranslateSettings::from_env(None, None),
            TranslateSettings::defaults()
        );
        assert_eq!(
            TranslateSettings::from_env(Some("a/b".to_owned()), None),
            settings("a/b", DEFAULT_REASONING)
        );
    }

    #[test]
    fn check_takes_model_ids_and_known_efforts_only() {
        assert_eq!(settings("openai/gpt-6-luna", "low").check(), Ok(()));
        assert_eq!(settings("vendor/name:free", "default").check(), Ok(()));
        for model in ["", "no-vendor", "a/b/c", "a/../b", "a/b c", "a/b\n"] {
            assert_eq!(
                settings(model, "low").check(),
                Err(Invalid::Model),
                "{model:?}"
            );
        }
        for effort in ["", "Low", "huge", "low "] {
            assert_eq!(
                settings("a/b", effort).check(),
                Err(Invalid::Reasoning),
                "{effort:?}"
            );
        }
    }

    #[test]
    fn key_label_matches_the_one_before_the_dashboard() {
        assert_eq!(settings("a/b", "low").key_label(), "a/b reasoning=low");
        assert_eq!(settings("a/b", "default").key_label(), "a/b");
        assert_eq!(settings("a/b", "default").effort(), None);
    }

    #[test]
    fn saved_settings_load_back_and_bad_files_are_refused() {
        let path = temp_file("roundtrip");
        assert_eq!(load(&path), Ok(None), "no file yet");
        let chosen = settings("vendor/model", "high");
        save(&path, &chosen).unwrap();
        assert_eq!(load(&path), Ok(Some(chosen)));
        assert!(
            !path.with_extension("json.tmp").exists(),
            "no temp file left"
        );

        std::fs::write(&path, "{not json").unwrap();
        assert_eq!(load(&path), Err(LoadError::Malformed));
        std::fs::write(&path, r#"{"model":"bad","reasoning":"low"}"#).unwrap();
        assert_eq!(load(&path), Err(LoadError::Invalid(Invalid::Model)));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
