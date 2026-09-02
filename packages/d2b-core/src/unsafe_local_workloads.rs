pub use d2b_contracts::unsafe_local_workloads::*;
pub use d2b_contracts_resource::v3::ZoneResourceIdentity;

/// Zone-neutral identity used by unsafe-local launcher and shell consumers.
///
/// The resource identity carries the Zone UID, resource UID, desired
/// generation, and committed Zone revision, so equal resource names in
/// different Zones or generations cannot share runtime state.
pub type UnsafeLocalWorkloadIdentity = ZoneResourceIdentity;
