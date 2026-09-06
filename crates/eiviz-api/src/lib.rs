//! Native control API: Protobuf contract, authentication, and transports.

#![allow(clippy::collapsible_if)]
#![allow(clippy::result_large_err)]

pub mod auth;
pub mod client;
pub mod codec;
pub mod media;
pub mod proto;
pub mod server;
pub mod vmix;

pub use auth::{AuthConfig, Role};
pub use client::{ControlClient, ControlSession};
pub use media::{FileMediaStorage, MediaStorage, MediaStorageConfig};
pub use server::{ServerBind, ServerConfig, listen, spawn};
