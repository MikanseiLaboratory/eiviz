//! Native control API: Protobuf contract, authentication, and transports.

#![allow(clippy::collapsible_if)]
#![allow(clippy::result_large_err)]

pub mod auth;
pub mod client;
pub mod codec;
pub mod proto;
pub mod server;
pub mod vmix;

pub use auth::{AuthConfig, Role};
pub use client::ControlClient;
pub use server::{ServerBind, ServerConfig, listen, spawn};
