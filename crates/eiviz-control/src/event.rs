use crate::command::Command;
use crate::error::ControlError;
use crate::live::{LiveState, ResourceStatus};
use crate::session::Document;

#[derive(Debug, Clone, PartialEq)]
pub struct EnvelopeMeta {
    pub sequence: u64,
    pub session_revision: u64,
    pub unix_ms: u64,
    pub request_id: String,
}

#[derive(Debug, Clone)]
pub enum Event {
    Ready {
        meta: EnvelopeMeta,
    },
    SessionChanged {
        meta: EnvelopeMeta,
        document: Box<Document>,
    },
    LiveChanged {
        meta: EnvelopeMeta,
        live: LiveState,
    },
    Resource {
        meta: EnvelopeMeta,
        status: ResourceStatus,
    },
    CommandApplied {
        meta: EnvelopeMeta,
        command: String,
    },
    TransitionStarted {
        meta: EnvelopeMeta,
        unit_id: u64,
    },
    TransitionCompleted {
        meta: EnvelopeMeta,
        unit_id: u64,
    },
    Lag {
        meta: EnvelopeMeta,
        missed: u64,
    },
    Failed {
        meta: EnvelopeMeta,
        error: ControlError,
    },
    Discovered {
        meta: EnvelopeMeta,
        kind: String,
        payload: String,
    },
    Shutdown {
        meta: EnvelopeMeta,
    },
}

impl Event {
    pub fn meta(&self) -> &EnvelopeMeta {
        match self {
            Self::Ready { meta }
            | Self::SessionChanged { meta, .. }
            | Self::LiveChanged { meta, .. }
            | Self::Resource { meta, .. }
            | Self::CommandApplied { meta, .. }
            | Self::TransitionStarted { meta, .. }
            | Self::TransitionCompleted { meta, .. }
            | Self::Lag { meta, .. }
            | Self::Failed { meta, .. }
            | Self::Discovered { meta, .. }
            | Self::Shutdown { meta } => meta,
        }
    }

    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Ready { .. } => "Ready",
            Self::SessionChanged { .. } => "SessionChanged",
            Self::LiveChanged { .. } => "LiveChanged",
            Self::Resource { .. } => "Resource",
            Self::CommandApplied { .. } => "CommandApplied",
            Self::TransitionStarted { .. } => "TransitionStarted",
            Self::TransitionCompleted { .. } => "TransitionCompleted",
            Self::Lag { .. } => "Lag",
            Self::Failed { .. } => "Failed",
            Self::Discovered { .. } => "Discovered",
            Self::Shutdown { .. } => "Shutdown",
        }
    }
}

pub fn command_name(command: &Command) -> &'static str {
    match command {
        Command::Preview { .. } => "Preview",
        Command::Cut { .. } => "Cut",
        Command::Auto { .. } => "Auto",
        Command::SetMix { .. } => "SetMix",
        Command::OverlayAuto { .. } => "OverlayAuto",
        Command::VideoPlay { .. } => "VideoPlay",
        Command::VideoLoop { .. } => "VideoLoop",
        Command::VideoSeek { .. } => "VideoSeek",
        Command::AudioSetInput { .. } => "AudioSetInput",
        Command::AudioSetBus { .. } => "AudioSetBus",
        Command::ReplaceSession { .. } => "ReplaceSession",
        Command::MutateSession { .. } => "MutateSession",
        Command::Snapshot { .. } => "Snapshot",
        Command::Discover { .. } => "Discover",
        Command::Shutdown => "Shutdown",
    }
}
