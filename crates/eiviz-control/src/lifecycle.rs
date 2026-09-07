#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifecycle {
    Starting,
    Ready,
    Failed,
    Stopping,
    Stopped,
}

impl Lifecycle {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Ready => "ready",
            Self::Failed => "failed",
            Self::Stopping => "stopping",
            Self::Stopped => "stopped",
        }
    }

    pub fn is_ready(self) -> bool {
        self == Self::Ready
    }
}

impl Default for Lifecycle {
    fn default() -> Self {
        Self::Stopped
    }
}
