//! Ticket verification surface. The private Pro crate implements this.
//! The Community stand-in returns `None` from [`crate::ProModule::license`].

use crate::error::ProResult;
use crate::plan::Plan;

/// Why a stored ticket is or is not in force.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LicenseCondition {
    Unregistered,
    Valid,
    Expired,
    FingerprintMismatch,
    BadSignature,
}

impl LicenseCondition {
    /// C ABI sentinel. `0` unregistered, then valid, expired, fingerprint, signature.
    pub fn abi(self) -> u32 {
        match self {
            Self::Unregistered => 0,
            Self::Valid => 1,
            Self::Expired => 2,
            Self::FingerprintMismatch => 3,
            Self::BadSignature => 4,
        }
    }

    pub fn from_abi(value: u32) -> Self {
        match value {
            1 => Self::Valid,
            2 => Self::Expired,
            3 => Self::FingerprintMismatch,
            4 => Self::BadSignature,
            _ => Self::Unregistered,
        }
    }
}

/// Result of reading the stored ticket.
/// `plan` is the plan named by a parsed ticket, or [`Plan::Community`] when nothing is stored.
/// Only [`LicenseCondition::Valid`] changes what the mixer is allowed to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LicenseStatus {
    pub condition: LicenseCondition,
    pub plan: Plan,
    pub expires_at: i64,
    pub ticket_id: String,
}

impl LicenseStatus {
    pub fn unregistered() -> Self {
        Self {
            condition: LicenseCondition::Unregistered,
            plan: Plan::Community,
            expires_at: 0,
            ticket_id: String::new(),
        }
    }
}

/// Installed by the host through the mixer. The mixer does not parse tickets.
pub trait LicenseBackend: Send + Sync {
    fn install(&self, ticket: &str) -> ProResult<LicenseStatus>;
    fn status(&self) -> LicenseStatus;
    fn clear(&self) -> ProResult<()>;
    /// JSON array of salted fingerprint hashes for the machine running this process.
    fn machine_fingerprint(&self) -> ProResult<String>;
}
