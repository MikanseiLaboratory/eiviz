use crate::error::{ControlError, ControlResult};
use crate::session::{Document, parse};

pub const CURRENT_VERSION: i32 = 2;

/// Explicit session version migration. Unknown newer versions are rejected
/// instead of being silently rewritten to version 2.
pub fn migrate_bytes(bytes: &[u8]) -> ControlResult<Document> {
    let text =
        std::str::from_utf8(bytes).map_err(|error| ControlError::invalid(error.to_string()))?;
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|error| ControlError::invalid(error.to_string()))?;
    let version = value
        .get("version")
        .and_then(|item| item.as_i64())
        .unwrap_or(CURRENT_VERSION as i64);
    if version > CURRENT_VERSION as i64 {
        return Err(ControlError::invalid(format!(
            "unsupported session version {version}"
        )));
    }
    if version < 0 {
        return Err(ControlError::invalid(format!(
            "unsupported session version {version}"
        )));
    }
    parse(bytes).map_err(ControlError::invalid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_version_is_rejected() {
        let err = migrate_bytes(br#"{"version": 99}"#).unwrap_err();
        assert!(err.message().contains("unsupported session version"));
    }
}
