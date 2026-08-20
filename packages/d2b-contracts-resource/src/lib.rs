#![doc = "Canonical standard resource contracts for d2b."]

pub use d2b_contracts::{error, ids, types};

pub mod generated;
pub mod resource_proto {
    pub use crate::generated::d2b_resource_v3::*;
}
pub mod v3;
