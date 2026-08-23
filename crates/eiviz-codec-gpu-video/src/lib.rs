use eiviz_media::Capability;

/// Isolated gpu-video adapter. Default builds never import the crate so wgpu
/// 24 (GUI) and wgpu 29 (gpu-video 0.4) cannot collide.
pub fn probe() -> Capability {
    Capability {
        id: "gpu-video".into(),
        available: cfg!(feature = "gpu-video"),
        detail: if cfg!(feature = "gpu-video") {
            "gpu-video 0.4 feature compiled; Vulkan Video required at runtime".into()
        } else {
            "not compiled; software codec is the CI path (ADR-0002 / ADR-0009)".into()
        },
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn default_build_does_not_enable_gpu_video() {
        assert!(!super::probe().available);
    }
}
