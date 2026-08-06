//! Opaque state-directory and tamper-marker contracts.

use core::fmt;

/// A Core-derived state-directory identity.
#[derive(Clone, PartialEq, Eq)]
pub struct StateDirectoryToken([u8; 32]);

impl StateDirectoryToken {
    /// Construct a token at the Core effect-adapter boundary.
    pub const fn from_core(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Borrow the token for equality checks at the effect boundary.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for StateDirectoryToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StateDirectoryToken(<redacted>)")
    }
}

/// A Core-derived identity-bound tamper marker.
#[derive(Clone, PartialEq, Eq)]
pub struct TamperMarkerToken([u8; 32]);

impl TamperMarkerToken {
    /// Construct a token at the Core effect-adapter boundary.
    pub const fn from_core(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Borrow the token for equality checks at the effect boundary.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for TamperMarkerToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TamperMarkerToken(<redacted>)")
    }
}

/// An opaque owner identity for the swtpm state principal.
#[derive(Clone, PartialEq, Eq)]
pub struct StateOwnerToken([u8; 16]);

impl StateOwnerToken {
    /// Construct a token at the Core effect-adapter boundary.
    pub const fn from_core(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }
}

impl fmt::Debug for StateOwnerToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StateOwnerToken(<redacted>)")
    }
}

/// The only state-directory intent a TPM Provider may submit.
#[derive(Clone, PartialEq, Eq)]
pub struct StateDirIntent {
    directory: StateDirectoryToken,
    marker: TamperMarkerToken,
    owner: StateOwnerToken,
}

impl StateDirIntent {
    /// Construct an opaque state-directory hardening request.
    pub const fn new(
        directory: StateDirectoryToken,
        marker: TamperMarkerToken,
        owner: StateOwnerToken,
    ) -> Self {
        Self {
            directory,
            marker,
            owner,
        }
    }

    /// Borrow the state-directory identity.
    pub const fn directory(&self) -> &StateDirectoryToken {
        &self.directory
    }

    /// Borrow the identity-bound marker token.
    pub const fn marker(&self) -> &TamperMarkerToken {
        &self.marker
    }

    /// Borrow the expected state owner token.
    pub const fn owner(&self) -> &StateOwnerToken {
        &self.owner
    }

    /// Validate one Core observation before any runner launch.
    pub fn validate(
        &self,
        observation: &TpmStateObservation,
    ) -> Result<TpmStatePreparation, TpmStateValidationError> {
        match observation.kind {
            TpmStateObservationKind::Fresh => Ok(TpmStatePreparation::Fresh),
            TpmStateObservationKind::ExistingWithMarker => Ok(TpmStatePreparation::Existing),
            TpmStateObservationKind::MissingMarker => {
                Err(TpmStateValidationError::PreviouslyProvisionedStateMissing)
            }
            TpmStateObservationKind::IdentityMismatch => {
                Err(TpmStateValidationError::StateIdentityMismatch)
            }
            TpmStateObservationKind::WrongOwnerOrType => {
                Err(TpmStateValidationError::WrongOwnerOrType)
            }
        }
    }
}

impl fmt::Debug for StateDirIntent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StateDirIntent(<redacted>)")
    }
}

/// A closed observation returned by Core's state-directory hardening effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TpmStateObservationKind {
    /// The directory and marker were created together.
    Fresh,
    /// A previously provisioned directory and marker match.
    ExistingWithMarker,
    /// A marker proving prior provisioning is absent.
    MissingMarker,
    /// The marker no longer identifies the directory.
    IdentityMismatch,
    /// The directory is not an exact owner/type/mode match.
    WrongOwnerOrType,
}

/// State-directory observation with no path or filesystem metadata.
#[derive(Clone, PartialEq, Eq)]
pub struct TpmStateObservation {
    kind: TpmStateObservationKind,
}

impl TpmStateObservation {
    /// Construct an observation supplied by the trusted Core adapter.
    pub const fn from_core(kind: TpmStateObservationKind) -> Self {
        Self { kind }
    }

    /// Return the closed observation class.
    pub const fn kind(&self) -> TpmStateObservationKind {
        self.kind
    }
}

impl fmt::Debug for TpmStateObservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TpmStateObservation")
            .field("kind", &self.kind)
            .finish()
    }
}

/// Result of validating state before the flush step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TpmStatePreparation {
    /// State was newly provisioned.
    Fresh,
    /// Existing state and marker were preserved.
    Existing,
}

/// Fail-closed state-directory validation errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TpmStateValidationError {
    /// The marker proving prior provisioning is absent.
    PreviouslyProvisionedStateMissing,
    /// The marker and state identity differ.
    StateIdentityMismatch,
    /// Owner, type, or mode does not match the trusted descriptor.
    WrongOwnerOrType,
}

impl TpmStateValidationError {
    /// Return the stable error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::PreviouslyProvisionedStateMissing => "previously-provisioned-swtpm-state-missing",
            Self::StateIdentityMismatch => "device-state-integrity-failure",
            Self::WrongOwnerOrType => "device-state-owner-mismatch",
        }
    }
}

impl fmt::Display for TpmStateValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for TpmStateValidationError {}
