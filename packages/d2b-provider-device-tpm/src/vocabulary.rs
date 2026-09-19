//! The TPM family's declared vocabulary.
//!
//! Every fact here is owned by this crate: the host device-matrix class the
//! TPM role claims, the inventory `busClass` the Device spec uses to select a
//! physical TPM, the row-name prefixes the family's declared worker rows are
//! authored under, the per-VM state-directory name and owner suffix, the
//! trusted storage-row prefix, the legacy migration intent prefix, and the
//! host device-node facts (major/minor pair and owner account) the privileged
//! open validates. Shared consumers read these constants instead of
//! restating the spellings.

/// The host device-matrix class the TPM role claims.
pub const TPM_DEVICE_CLASS: &str = "tpm";

/// The inventory `busClass` a physical TPM Device spec selects.
pub const TPM_BUS_CLASS: &str = "tpm";

/// The prefix of the declared long-lived swtpm Process row
/// (`Process/swtpm-<device>`).
pub const TPM_PROCESS_ROW_PREFIX: &str = "Process/swtpm-";

/// The prefix of the declared one-shot flush EphemeralProcess row
/// (`EphemeralProcess/swtpm-flush-<device>`).
pub const TPM_FLUSH_ROW_PREFIX: &str = "EphemeralProcess/swtpm-flush-";

/// The prefix of the declared TPM Endpoint row (`Endpoint/tpm-<device>`).
pub const TPM_ENDPOINT_ROW_PREFIX: &str = "Endpoint/tpm-";

/// The suffix of the controller-owned state Volume name
/// (`device-<32hex>-tpm-state`).
pub const TPM_STATE_VOLUME_NAME_SUFFIX: &str = "-tpm-state";

/// The per-VM state directory name under `<stateDir>/vms/<vm>`.
pub const TPM_STATE_DIR_NAME: &str = "swtpm";

/// The principal-account suffix the TPM worker principals carry
/// (`d2b-<zone>-<device>-swtpm` and its `-flush` sibling).
pub const TPM_STATE_OWNER_SUFFIX: &str = "-swtpm";

/// The trusted storage-row prefix the zone-native TPM state root is
/// resolved under (`path:swtpm-state:<guest>`).
pub const TPM_STATE_STORAGE_ROW_PREFIX: &str = "path:swtpm-state:";

/// The legacy swtpm migration intent prefix (`legacy-swtpm:vm:<vm>`).
pub const TPM_LEGACY_MIGRATION_INTENT_PREFIX: &str = "legacy-swtpm:vm:";

/// The TPM device-node major number the privileged open validates.
pub const TPM_DEVICE_MAJOR: u64 = 10;

/// The TPM device-node minor number the privileged open validates.
pub const TPM_DEVICE_MINOR: u64 = 224;

/// The owner account the TPM device node is validated against.
pub const TPM_DEVICE_OWNER: &str = "tss";