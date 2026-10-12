use std::fmt;

/// Transport-independent control failure. Adapters map this onto C ABI `i32`,
/// vMix HTTP status, and Protobuf `Status`. Raw ABI codes must not circulate
/// inside the control domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlError {
    InvalidArgument { message: String },
    NotFound { message: String },
    Ambiguous { message: String },
    Conflict { message: String },
    Unavailable { message: String },
    PermissionDenied { message: String },
    NotSupportedPlan { message: String },
    Io { message: String },
    Internal { message: String },
}

impl ControlError {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::InvalidArgument {
            message: message.into(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound {
            message: message.into(),
        }
    }

    pub fn ambiguous(message: impl Into<String>) -> Self {
        Self::Ambiguous {
            message: message.into(),
        }
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::Conflict {
            message: message.into(),
        }
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::Unavailable {
            message: message.into(),
        }
    }

    pub fn permission(message: impl Into<String>) -> Self {
        Self::PermissionDenied {
            message: message.into(),
        }
    }

    pub fn not_supported_plan(message: impl Into<String>) -> Self {
        Self::NotSupportedPlan {
            message: message.into(),
        }
    }

    pub fn io(message: impl Into<String>) -> Self {
        Self::Io {
            message: message.into(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Self::InvalidArgument { message }
            | Self::NotFound { message }
            | Self::Ambiguous { message }
            | Self::Conflict { message }
            | Self::Unavailable { message }
            | Self::PermissionDenied { message }
            | Self::NotSupportedPlan { message }
            | Self::Io { message }
            | Self::Internal { message } => message,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidArgument { .. } => "INVALID_ARGUMENT",
            Self::NotFound { .. } => "NOT_FOUND",
            Self::Ambiguous { .. } => "AMBIGUOUS",
            Self::Conflict { .. } => "CONFLICT",
            Self::Unavailable { .. } => "UNAVAILABLE",
            Self::PermissionDenied { .. } => "PERMISSION_DENIED",
            Self::NotSupportedPlan { .. } => "NOT_SUPPORTED_PLAN",
            Self::Io { .. } => "IO",
            Self::Internal { .. } => "INTERNAL",
        }
    }

    /// C ABI integer mapping. `7` is `ERR_NOT_SUPPORTED_PLAN`.
    pub fn to_abi(&self) -> i32 {
        match self {
            Self::Conflict { message } if message.contains("already created") => 1,
            Self::Unavailable { .. } => 2,
            Self::InvalidArgument { .. }
            | Self::NotFound { .. }
            | Self::Ambiguous { .. }
            | Self::Conflict { .. }
            | Self::PermissionDenied { .. } => 3,
            Self::Internal { .. } => 4,
            Self::Io { .. } => 5,
            Self::NotSupportedPlan { .. } => 7,
        }
    }

    pub fn from_abi(code: i32, message: impl Into<String>) -> Self {
        let message = message.into();
        match code {
            0 => Self::internal("unexpected ok mapped as error"),
            1 => Self::conflict(if message.is_empty() {
                "already created".into()
            } else {
                message
            }),
            2 => Self::unavailable(if message.is_empty() {
                "mixer not created".into()
            } else {
                message
            }),
            3 => Self::invalid(message),
            4 => Self::internal(if message.is_empty() {
                "device".into()
            } else {
                message
            }),
            5 => Self::io(message),
            7 => Self::not_supported_plan(if message.is_empty() {
                "not included in this plan".into()
            } else {
                message
            }),
            other => Self::internal(format!("abi {other}: {message}")),
        }
    }

    pub fn http_status(&self) -> u16 {
        match self {
            Self::InvalidArgument { .. } | Self::Ambiguous { .. } => 400,
            Self::PermissionDenied { .. } => 401,
            Self::NotSupportedPlan { .. } => 403,
            Self::NotFound { .. } => 404,
            Self::Conflict { .. } => 409,
            Self::Unavailable { .. } => 503,
            Self::Io { .. } | Self::Internal { .. } => 500,
        }
    }
}

impl fmt::Display for ControlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code(), self.message())
    }
}

impl std::error::Error for ControlError {}

pub type ControlResult<T> = Result<T, ControlError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abi_round_trip_keeps_historical_range() {
        assert_eq!(ControlError::invalid("x").to_abi(), 3);
        assert_eq!(ControlError::not_found("x").to_abi(), 3);
        assert_eq!(ControlError::ambiguous("x").to_abi(), 3);
        assert_eq!(ControlError::conflict("revision").to_abi(), 3);
        assert_eq!(ControlError::conflict("already created").to_abi(), 1);
        assert_eq!(ControlError::unavailable("x").to_abi(), 2);
        assert_eq!(ControlError::internal("x").to_abi(), 4);
        assert_eq!(ControlError::io("x").to_abi(), 5);
        assert_eq!(ControlError::not_supported_plan("x").to_abi(), 7);
        assert_eq!(
            ControlError::from_abi(7, "plan").code(),
            "NOT_SUPPORTED_PLAN"
        );
        assert_eq!(ControlError::not_supported_plan("x").http_status(), 403);
    }
}
