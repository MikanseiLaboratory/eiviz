use std::fs;
use std::path::{Path, PathBuf};

use eiviz_api::Role;
use eiviz_control::session::Renderer;
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub renderer: Option<String>,
}

impl HeadlessPrefs {
    pub fn path() -> PathBuf {
        config_dir().join("headless-prefs.json")
    }

    pub fn load() -> Result<Self, String> {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(path)
            .map_err(|error| format!("read {}: {error}", path.display()))?;
        let prefs: Self = serde_json::from_str(&text)
            .map_err(|error| format!("parse {}: {error}", path.display()))?;
        prefs.validate()?;
        Ok(prefs)
    }

    pub fn save(&self) -> Result<PathBuf, String> {
        let path = Self::path();
        self.save_to(&path)?;
        Ok(path)
    }

    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        self.validate()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|error| error.to_string())?;
        fs::write(path, text).map_err(|error| error.to_string())
    }

    pub fn validate(&self) -> Result<(), String> {
        if let Some(role) = self.max_role.as_deref() {
            Role::try_from_name(role)?;
        }
        if let Some(renderer) = self.renderer.as_deref() {
            parse_renderer(renderer)?;
        }
        Ok(())
    }

    pub fn renderer(&self) -> Result<Option<Renderer>, String> {
        self.renderer.as_deref().map(parse_renderer).transpose()
    }

    pub fn get(&self, key: &str) -> Result<Option<String>, String> {
        match normalize_key(key).as_str() {
            "bind" => Ok(self.bind.clone()),
            "token" => Ok(self.token.clone()),
            "mediadirectory" => Ok(self.media_directory.clone()),
            "maxrole" => Ok(self.max_role.clone()),
            "renderer" => Ok(self.renderer.clone()),
            _ => Err(unknown_key(key)),
        }
    }

    pub fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        let value = value.trim();
        match normalize_key(key).as_str() {
            "bind" => self.bind = empty_to_none(value),
            "token" => self.token = empty_to_none(value),
            "mediadirectory" => self.media_directory = empty_to_none(value),
            "maxrole" => {
                if !value.is_empty() {
                    Role::try_from_name(value)?;
                }
                self.max_role = empty_to_none(value);
            }
            "renderer" => {
                if !value.is_empty() {
                    parse_renderer(value)?;
                }
                self.renderer = empty_to_none(value);
            }
            _ => return Err(unknown_key(key)),
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
        let renderer = self.renderer.as_deref().unwrap_or("");
        format!(
            "bind={bind}\ntoken={token}\nmediaDirectory={media}\nmaxRole={role}\nrenderer={renderer}"
        )
    }
}

pub fn parse_renderer(raw: &str) -> Result<Renderer, String> {
    Renderer::parse_name(raw)
        .ok_or_else(|| format!("unknown renderer '{raw}' (auto, dx12, vulkan, metal)"))
}

pub fn require_os_renderer(renderer: Renderer) -> Result<Renderer, String> {
    if let Some(message) = renderer.unsupported_os_message() {
        return Err(message);
    }
    Ok(renderer)
}

pub fn resolve_renderer(cli: Option<&str>, prefs: &HeadlessPrefs) -> Result<Renderer, String> {
    let renderer = if let Some(raw) = cli.filter(|value| !value.trim().is_empty()) {
        parse_renderer(raw)?
    } else {
        prefs.renderer()?.unwrap_or(Renderer::Auto)
    };
    require_os_renderer(renderer)
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

fn unknown_key(key: &str) -> String {
    format!("unknown prefs key '{key}' (bind, token, mediaDirectory, maxRole, renderer)")
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
        prefs.set("renderer", "vulkan").unwrap();
        assert_eq!(
            prefs.get("bind").unwrap().as_deref(),
            Some("127.0.0.1:9400")
        );
        assert_eq!(
            prefs.get("mediaDirectory").unwrap().as_deref(),
            Some("/tmp/eiviz-media")
        );
        assert_eq!(prefs.get("renderer").unwrap().as_deref(), Some("vulkan"));
        let json = serde_json::to_string(&prefs).unwrap();
        let again: HeadlessPrefs = serde_json::from_str(&json).unwrap();
        assert_eq!(prefs, again);
    }

    #[test]
    fn unknown_renderer_and_role_are_rejected() {
        let mut prefs = HeadlessPrefs::default();
        assert!(prefs.set("renderer", "opengl").is_err());
        assert!(prefs.set("maxRole", "root").is_err());
        let broken = HeadlessPrefs {
            renderer: Some("opengl".into()),
            ..HeadlessPrefs::default()
        };
        assert!(broken.validate().is_err());
    }

    #[test]
    fn load_missing_file_is_default() {
        let path =
            std::env::temp_dir().join(format!("eiviz-missing-prefs-{}.json", std::process::id()));
        let _ = fs::remove_file(&path);
        assert_eq!(
            HeadlessPrefs::load_from(&path).unwrap(),
            HeadlessPrefs::default()
        );
    }

    #[test]
    fn load_broken_json_is_error() {
        let path =
            std::env::temp_dir().join(format!("eiviz-broken-prefs-{}.json", std::process::id()));
        fs::write(&path, "{not json").unwrap();
        assert!(
            HeadlessPrefs::load_from(&path)
                .unwrap_err()
                .contains("parse")
        );
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn resolve_renderer_prefers_cli() {
        let mut prefs = HeadlessPrefs::default();
        prefs.set("renderer", "auto").unwrap();
        #[cfg(windows)]
        {
            assert_eq!(
                resolve_renderer(Some("dx12"), &prefs).unwrap(),
                Renderer::Dx12
            );
            assert!(resolve_renderer(Some("metal"), &prefs).is_err());
        }
        #[cfg(target_os = "linux")]
        {
            assert_eq!(
                resolve_renderer(Some("vulkan"), &prefs).unwrap(),
                Renderer::Vulkan
            );
            assert!(resolve_renderer(Some("dx12"), &prefs).is_err());
        }
        #[cfg(target_os = "macos")]
        {
            assert_eq!(
                resolve_renderer(Some("metal"), &prefs).unwrap(),
                Renderer::Metal
            );
            assert!(resolve_renderer(Some("vulkan"), &prefs).is_err());
        }
    }

    #[test]
    fn token_display_is_masked() {
        let mut prefs = HeadlessPrefs::default();
        prefs.set("token", "secret").unwrap();
        assert!(prefs.display().contains("token=(set)"));
        assert!(!prefs.display().contains("secret"));
    }
}
