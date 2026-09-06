//! vMix HTTP compatibility lives in `eiviz_mixer` so the GPU `cdylib` does not
//! take a Tokio/Protobuf dependency. Functions dispatch through `ControlService`.

pub use eiviz_control::command::{Command, Incoming};
