use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HeadlessPrefs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_directory: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_role: Option<String>,
}

impl HeadlessPrefs {
    pub fn path() -> PathBuf {
        config_dir().join("headless-prefs.json")
    }

    pub fn load() -> Self {
        Self::load_from(&Self::path()).unwrap_or_default()
    }

    pub fn load_from(path: &Path) -> Option<Self> {
        let text = fs::read_to_string(path).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save(&self) -> Result<PathBuf, String> {
        let path = Self::path();
        self.save_to(&path)?;
        Ok(path)
    }

    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|error| error.to_string())?;
        fs::write(path, text).map_err(|error| error.to_string())
    }

    pub fn get(&self, key: &str) -> Result<Option<String>, String> {
        match normalize_key(key).as_str() {
            "bind" => Ok(self.bind.clone()),
            "token" => Ok(self.token.clone()),
            "mediadirectory" => Ok(self.media_directory.clone()),
            "maxrole" => Ok(self.max_role.clone()),
            _ => Err(format!(
                "unknown prefs key '{key}' (bind, token, mediaDirectory, maxRole)"
            )),
        }
    }

    pub fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        let value = value.trim();
        match normalize_key(key).as_str() {
            "bind" => self.bind = empty_to_none(value),
            "token" => self.token = empty_to_none(value),
            "mediadirectory" => self.media_directory = empty_to_none(value),
            "maxrole" => self.max_role = empty_to_none(value),
            _ => {
                return Err(format!(
                    "unknown prefs key '{key}' (bind, token, mediaDirectory, maxRole)"
                ));
            }
        }
        Ok(())
    }

    pub fn display(&self) -> String {
        let bind = self.bind.as_deref().unwrap_or("");
        let token = if self.token.as_deref().unwrap_or("").is_empty() {
            ""
        } else {
            "(set)"
        };
        let media = self.media_directory.as_deref().unwrap_or("");
        let role = self.max_role.as_deref().unwrap_or("");
        format!("bind={bind}\ntoken={token}\nmediaDirectory={media}\nmaxRole={role}")
    }
}

fn empty_to_none(value: &str) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn normalize_key(key: &str) -> String {
    key.trim().replace(['-', '_'], "").to_ascii_lowercase()
}

fn config_dir() -> PathBuf {
    if let Ok(root) = std::env::var("LOCALAPPDATA")
        && !root.is_empty()
    {
        return PathBuf::from(root).join("eiviz");
    }
    if let Ok(root) = std::env::var("XDG_CONFIG_HOME")
        && !root.is_empty()
    {
        return PathBuf::from(root).join("eiviz");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".config").join("eiviz");
    }
    PathBuf::from("eiviz")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_roundtrip_keys() {
        let mut prefs = HeadlessPrefs::default();
        prefs.set("bind", "127.0.0.1:9400").unwrap();
        prefs.set("media-directory", "/tmp/eiviz-media").unwrap();
        assert_eq!(
            prefs.get("bind").unwrap().as_deref(),
            Some("127.0.0.1:9400")
        );
        assert_eq!(
            prefs.get("mediaDirectory").unwrap().as_deref(),
            Some("/tmp/eiviz-media")
        );
        let json = serde_json::to_string(&prefs).unwrap();
        let again: HeadlessPrefs = serde_json::from_str(&json).unwrap();
        assert_eq!(prefs, again);
    }
}
