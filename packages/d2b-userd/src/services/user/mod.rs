//! User-owned Secret Service, credential lease, and scoped TPM2 export flows.

mod export;
mod identity;
mod secret_service;
mod service;

pub use export::{
    ExportCommitError, ExportCommitPort, ExportHandle, ExportInspection, ExportRequest,
    ExportState, InMemoryExportCommitPort, ScopedExportManager, SealedExport,
    SystemdCredsTpm2Sealer, Tpm2SealContext, Tpm2SealError, Tpm2Sealer,
};
pub use identity::{
    AuthenticatedUser, ClosedOutcome, NoopSecretMetrics, OwnerBinding, SecretMetricEvent,
    SecretMetricSink, SecretOperation, UserSecretError,
};
#[cfg(feature = "secret-service")]
pub use secret_service::Oo7SecretStore;
pub use secret_service::{
    EntropySource, OsEntropy, OwnedSecretMetadata, OwnedSecretSelector, SecretMaterial,
    SecretStore, SecretStoreError, SystemUserdClock, UserdClock, UserdSecretServicePort,
};
pub use service::{AdmittedUserRequest, UserSecretService};

pub const SERVICE_PACKAGE: &str = "d2b.user.v2";
pub const ENDPOINT_PURPOSE: &str = "user-agent";
pub const ENDPOINT_ROLE: &str = "user-agent";
