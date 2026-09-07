//! Transport-independent control domain for eiviz.
//!
//! This crate owns canonical session DTOs, Command/Query/Event types, the
//! session store, and `MixerPort`. GPU, OS, and network transports stay out.

#![allow(clippy::collapsible_if)]
#![allow(clippy::collapsible_match)]
#![allow(clippy::derivable_impls)]

pub mod command;
pub mod error;
pub mod event;
pub mod event_hub;
pub mod geometry;
pub mod ids;
pub mod lifecycle;
pub mod live;
pub mod port;
pub mod query;
pub mod service;
pub mod session;
pub mod video_trigger;

pub use command::{Command, Incoming, SessionMutation};
pub use error::{ControlError, ControlResult};
pub use event::Event;
pub use event_hub::EventHub;
pub use ids::{Resolver, ResourceKind, ResourceRef};
pub use lifecycle::Lifecycle;
pub use live::{LiveState, ResourceStatus, UnitLiveState};
pub use port::{MixerPort, NullMixer};
pub use query::{Capabilities, Query, Snapshot};
pub use service::{CommandOutcome, ControlFacade, ControlService, RequestKey};
pub use session::{Document, parse};
