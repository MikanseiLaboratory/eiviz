pub mod eiviz {
    pub mod control {
        pub mod v1 {
            include!(concat!(env!("OUT_DIR"), "/eiviz.control.v1.rs"));
        }
    }
}

pub use eiviz::control::v1::*;
