//! Domain model. No GPU, GUI, or vendor SDK types.

mod audio;
mod graph;
mod ids;
mod input;
mod mixing;
mod output;
mod project;
mod scene;
mod transform;

pub use audio::*;
pub use graph::MixingGraph;
pub use ids::*;
pub use input::*;
pub use mixing::*;
pub use output::*;
pub use project::*;
pub use scene::*;
pub use transform::*;

use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DomainError {
    #[error("{0}")]
    Message(String),
    #[error("unknown id: {0}")]
    UnknownId(String),
    #[error("duplicate id: {0}")]
    DuplicateId(String),
    #[error("mixing graph contains a cycle")]
    Cycle,
    #[error("capacity exceeded: {0}")]
    Capacity(String),
    #[error("invalid reference: {0}")]
    InvalidRef(String),
}

impl DomainError {
    pub fn msg(msg: impl Into<String>) -> Self {
        Self::Message(msg.into())
    }
}

pub type Result<T> = std::result::Result<T, DomainError>;
