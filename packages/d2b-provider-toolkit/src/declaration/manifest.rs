//! Canonical Provider manifest emission and verification.
//!
//! Provider manifests are signed over their exact file bytes.  The
//! [`d2b_contracts_resource::v3::canonical_json_bytes`] implementation is therefore
//! used for both writing and checking rather than a general-purpose JSON
//! formatter.

use d2b_contracts_provider::v3::{
    ProviderContractError, ProviderDeclarationSpec, ProviderManifest,
};
use d2b_contracts_resource::v3::canonical_json_bytes;
use serde_json::Error as JsonError;

/// The bounded values used in a canonicality diagnostic.
///
/// A file on disk can be larger than the diagnostic representation.  The
/// conversion is saturating so an attacker-controlled file size cannot wrap
/// an offset or length in a user-facing error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanonicalMismatch {
    offset: u64,
    expected_len: u64,
    observed_len: u64,
}

impl CanonicalMismatch {
    /// Return the first byte offset at which the observed bytes differ.
    pub const fn offset(self) -> u64 {
        self.offset
    }

    /// Return the length of the canonical bytes, saturated to `u64`.
    pub const fn expected_len(self) -> u64 {
        self.expected_len
    }

    /// Return the length of the observed bytes, saturated to `u64`.
    pub const fn observed_len(self) -> u64 {
        self.observed_len
    }
}

/// A manifest could not be verified as its own canonical bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerificationError {
    /// The input was not a valid `ProviderManifest` JSON document.
    InvalidManifest,
    /// The input parsed, but its bytes differ from canonical emission.
    NotCanonical(CanonicalMismatch),
}

impl core::fmt::Display for VerificationError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidManifest => formatter.write_str("provider manifest is not valid JSON"),
            Self::NotCanonical(mismatch) => write!(
                formatter,
                "provider-manifest-not-canonical: offset={} expected-len={} observed-len={}",
                mismatch.offset(),
                mismatch.expected_len(),
                mismatch.observed_len()
            ),
        }
    }
}

impl std::error::Error for VerificationError {}

/// Emit a Provider manifest as exact `d2b-cjson/v1` bytes.
///
/// The returned buffer is the file payload: it has sorted object keys,
/// integer-only numbers, NFC-validated strings, no BOM, and no trailing
/// newline.  A `ProviderManifest` has already passed the contract
/// constructors, so an inability to serialize it indicates a programming
/// error rather than an authoring failure.
pub fn emit_canonical(manifest: &ProviderManifest) -> Vec<u8> {
    canonical_json_bytes(manifest).expect("ProviderManifest must be canonicalizable")
}

/// Emit a provider declaration's data projection as exact `d2b-cjson/v1`
/// bytes.
///
/// This is the bytes a generator consumes (KTD1, U4): the canonical manifest
/// inputs, the schema inputs, the Nix projection, and the static composition
/// source are all derived from it. The projection is deterministic - sorted
/// object keys, integer-only numbers, NFC-validated strings, no BOM, and no
/// trailing newline - so two runs over the same declaration produce the same
/// file, and an unchanged declaration produces an unchanged artifact.
///
/// The spec holds identities and semantics only. It carries no signing key,
/// no handler, no decoder or factory reference, and no runtime state, so the
/// projection cannot be a way to introduce a compiled privileged handler or
/// a private key into a generated file.
pub fn emit_declaration_canonical(spec: &ProviderDeclarationSpec) -> Vec<u8> {
    canonical_json_bytes(spec).expect("ProviderDeclarationSpec must be canonicalizable")
}

/// Validate the signed placement, target-artifact, and EffectPort contract
/// before a manifest enters a package or catalog.
pub fn validate_for_installation(manifest: &ProviderManifest) -> Result<(), ProviderContractError> {
    manifest.validate_installation_contract()
}

/// Verify that a serialized Provider manifest is already canonical.
///
/// The same typed parse and canonical re-emission used by the CLI are exposed
/// here so the compiler-facing and author-facing offset calculation cannot
/// drift.
pub fn verify_canonical(bytes: &[u8]) -> Result<(), VerificationError> {
    let manifest = serde_json::from_slice::<ProviderManifest>(bytes)
        .map_err(|_: JsonError| VerificationError::InvalidManifest)?;
    validate_for_installation(&manifest).map_err(|_| VerificationError::InvalidManifest)?;
    let expected = emit_canonical(&manifest);
    if expected == bytes {
        return Ok(());
    }
    Err(VerificationError::NotCanonical(first_mismatch(
        &expected, bytes,
    )))
}

fn first_mismatch(expected: &[u8], observed: &[u8]) -> CanonicalMismatch {
    let offset = expected
        .iter()
        .zip(observed)
        .position(|(expected, observed)| expected != observed)
        .unwrap_or_else(|| expected.len().min(observed.len()));
    CanonicalMismatch {
        offset: saturating_u64(offset),
        expected_len: saturating_u64(expected.len()),
        observed_len: saturating_u64(observed.len()),
    }
}

fn saturating_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}
