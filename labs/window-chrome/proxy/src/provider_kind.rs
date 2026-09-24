//! Which kind of workload runtime backs one proxy identity.
//!
//! Local copy of the provider-kind vocabulary: the proxy is a standalone
//! lab workspace and must not depend on the main repo's contract crates.

/// Security posture of the workload runtime the proxy serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkloadProviderKind {
    /// Locally supervised NixOS VM.
    LocalVm,
    /// Locally supervised external-media QEMU runtime.
    QemuMedia,
    /// Runtime owned by a provider adapter.
    ProviderManaged,
    /// Host-user runtime with no isolation boundary.
    UnsafeLocal,
}
