//! Free-plan stand-in for the private Pro module.
//!
//! Official release builds replace `pro/` with a checkout of
//! `MikanseiLaboratory/eiviz-pro`. This crate keeps the same package name and
//! `module()` entry so the mixer does not change.

use eiviz_pro_api::{DeckLinkBackend, Entitlements, ProModule, StreamBackend};

struct Stub;

impl ProModule for Stub {
    fn module_name(&self) -> &'static str {
        "stub"
    }

    fn entitlements(&self) -> Entitlements {
        Entitlements::free()
    }

    fn decklink(&self) -> Option<&dyn DeckLinkBackend> {
        None
    }

    fn streaming(&self) -> Option<&dyn StreamBackend> {
        None
    }
}

pub fn module() -> &'static dyn ProModule {
    static MODULE: std::sync::OnceLock<Stub> = std::sync::OnceLock::new();
    MODULE.get_or_init(Stub::new)
}

impl Stub {
    fn new() -> Self {
        Self
    }
}
