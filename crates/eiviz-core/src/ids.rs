use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

macro_rules! typed_id {
    ($name:ident) => {
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            pub fn from_u128(value: u128) -> Self {
                Self(Uuid::from_u128(value))
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

typed_id!(ProjectId);
typed_id!(InputId);
typed_id!(SceneId);
typed_id!(SceneItemId);
typed_id!(MixingUnitId);
typed_id!(OverlayId);
typed_id!(OutputId);
typed_id!(MultiviewId);
typed_id!(AudioBusId);
typed_id!(AssetId);
typed_id!(DeviceBindingId);
typed_id!(CommandId);
typed_id!(ClientId);
