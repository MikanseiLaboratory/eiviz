use eiviz_media::Capability;

/// Isolated gpu-video adapter. Default builds never import the crate so wgpu
/// 24 (GUI) and wgpu 29 (gpu-video 0.4) cannot collide.
pub fn probe() -> Capability {
    Capability {
        id: "gpu-video".into(),
        available: false,
        detail: if cfg!(feature = "gpu-video") {
            "gpu-video feature selected, but the adapter is not implemented; refusing to report availability".into()
        } else {
            "not compiled; software codec is the CI path (ADR-0002 / ADR-0009)".into()
        },
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn unimplemented_adapter_never_reports_available() {
        assert!(!super::probe().available);
    }
}
