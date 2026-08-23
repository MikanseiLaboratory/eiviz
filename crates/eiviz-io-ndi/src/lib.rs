use eiviz_media::Capability;

/// NDI is feature/SDK gated. Default builds report Unavailable and stay running.
pub fn probe() -> Capability {
    Capability {
        id: "ndi".into(),
        available: cfg!(feature = "ndi-sdk"),
        detail: if cfg!(feature = "ndi-sdk") {
            "grafton-ndi adapter compiled".into()
        } else {
            "NDI SDK not linked; enable feature ndi-sdk on a licensed build".into()
        },
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn default_is_unavailable() {
        let c = super::probe();
        assert!(!c.available);
        assert!(c.detail.contains("NDI"));
    }
}
