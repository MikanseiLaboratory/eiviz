use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProErrorKind {
    /// The current plan does not include the feature or quota.
    NotSupportedPlan,
    /// The feature is entitled but its driver, SDK, or runtime library is missing.
    Unavailable,
    InvalidArgument,
    Device,
    Io,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProError {
    pub kind: ProErrorKind,
    pub message: String,
}

impl ProError {
    pub fn new(kind: ProErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn not_supported_plan(message: impl Into<String>) -> Self {
        Self::new(ProErrorKind::NotSupportedPlan, message)
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(ProErrorKind::Unavailable, message)
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ProErrorKind::InvalidArgument, message)
    }

    pub fn device(message: impl Into<String>) -> Self {
        Self::new(ProErrorKind::Device, message)
    }

    pub fn io(message: impl Into<String>) -> Self {
        Self::new(ProErrorKind::Io, message)
    }
}

impl fmt::Display for ProError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}

impl std::error::Error for ProError {}

pub type ProResult<T> = Result<T, ProError>;
