use crate::session::Document;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Query {
    GetSnapshot,
    GetCapabilities,
    GetRevision,
    GetLiveState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capabilities {
    pub protocol_version: String,
    pub mixer_version: String,
    pub platforms: Vec<String>,
    pub commands: Vec<String>,
    pub presentation_abi: bool,
    pub native_api: bool,
    pub vmix_http: bool,
}

impl Default for Capabilities {
    fn default() -> Self {
        Self {
            protocol_version: "eiviz.control.v1".into(),
            mixer_version: env!("CARGO_PKG_VERSION").into(),
            platforms: vec!["windows".into(), "macos".into(), "linux".into()],
            commands: vec![
                "Preview".into(),
                "Cut".into(),
                "Auto".into(),
                "SetMix".into(),
                "ReplaceSession".into(),
                "GetSnapshot".into(),
                "Subscribe".into(),
            ],
            presentation_abi: true,
            native_api: true,
            vmix_http: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub revision: u64,
    pub sequence: u64,
    pub document: Document,
    pub live: crate::live::LiveState,
    pub resources: Vec<crate::live::ResourceStatus>,
    pub capabilities: Capabilities,
    pub lifecycle: crate::lifecycle::Lifecycle,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_platforms_include_linux() {
        let caps = Capabilities::default();
        assert!(caps.platforms.iter().any(|p| p == "windows"));
        assert!(caps.platforms.iter().any(|p| p == "macos"));
        assert!(caps.platforms.iter().any(|p| p == "linux"));
    }
}
