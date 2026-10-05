//! Canonical manifest signed by the Pro release and checked by the mixer.
//!
//! The signed bytes are UTF-8 JSON with these keys in order:
//! `abiHash`, `abiMajor`, `moduleVersion`, `sha256`, `target`.
//! `sha256` is the lowercase hex digest of the module file.

/// Fields covered by the detached module signature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleManifest {
    pub abi_hash: u64,
    pub abi_major: u32,
    pub module_version: String,
    pub sha256_hex: String,
    pub target: String,
}

impl ModuleManifest {
    pub fn canonical(&self) -> Result<String, String> {
        validate_token(&self.module_version, "module version")?;
        validate_hex(&self.sha256_hex)?;
        validate_token(&self.target, "target")?;
        Ok(format!(
            "{{\"abiHash\":{},\"abiMajor\":{},\"moduleVersion\":\"{}\",\"sha256\":\"{}\",\"target\":\"{}\"}}",
            self.abi_hash, self.abi_major, self.module_version, self.sha256_hex, self.target
        ))
    }

    pub fn parse(canonical: &str) -> Result<Self, String> {
        let value: serde_json::Value =
            serde_json::from_str(canonical).map_err(|error| error.to_string())?;
        let object = value.as_object().ok_or("manifest is not an object")?;
        let keys: Vec<_> = object.keys().cloned().collect();
        let expected = ["abiHash", "abiMajor", "moduleVersion", "sha256", "target"];
        if keys != expected {
            return Err("manifest keys are out of order or unknown".into());
        }
        let manifest = Self {
            abi_hash: object["abiHash"].as_u64().ok_or("abiHash")?,
            abi_major: object["abiMajor"].as_u64().ok_or("abiMajor")? as u32,
            module_version: object["moduleVersion"]
                .as_str()
                .ok_or("moduleVersion")?
                .to_string(),
            sha256_hex: object["sha256"].as_str().ok_or("sha256")?.to_string(),
            target: object["target"].as_str().ok_or("target")?.to_string(),
        };
        if manifest.canonical()? != canonical {
            return Err("manifest is not canonical".into());
        }
        Ok(manifest)
    }
}

fn validate_token(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(format!("{label} contains unsupported characters"));
    }
    Ok(())
}

fn validate_hex(value: &str) -> Result<(), String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("sha256 must be 64 hex characters".into());
    }
    if value.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return Err("sha256 must be lowercase".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ModuleManifest {
        ModuleManifest {
            abi_hash: 99,
            abi_major: 1,
            module_version: "0.3.0".into(),
            sha256_hex: "ab".repeat(32),
            target: "x86_64-windows".into(),
        }
    }

    #[test]
    fn canonical_round_trip() {
        let text = sample().canonical().unwrap();
        assert!(text.starts_with("{\"abiHash\":99,"));
        assert_eq!(ModuleManifest::parse(&text).unwrap(), sample());
    }

    #[test]
    fn rejects_reordered_keys() {
        let text = sample().canonical().unwrap().replace("abiHash", "sha256");
        assert!(ModuleManifest::parse(&text).is_err());
    }
}
