//! Security-key relay and Guest frontend Process declarations.

use core::fmt;
use d2b_contracts::v3::{ResourceRef, ResourceUid};

/// Security-key Provider Process role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityKeyProcessRole {
    /// Unprivileged Host relay receiving the hidraw LaunchTicket.
    HostRelay,
    /// Guest-side UHID frontend.
    GuestFrontend,
}

/// Guest frontend Process declaration.
#[derive(Clone, PartialEq, Eq)]
pub struct FrontendProcessDeclaration {
    name: String,
    execution_ref: ResourceRef,
    role: SecurityKeyProcessRole,
    domain: &'static str,
}

impl FrontendProcessDeclaration {
    /// Construct the Guest frontend declaration for one Device and Guest.
    pub fn new(
        device_uid: &ResourceUid,
        execution_ref: ResourceRef,
    ) -> Result<Self, ProcessDeclarationError> {
        if execution_ref.resource_type().as_str() != "Guest" {
            return Err(ProcessDeclarationError::WrongExecutionRef);
        }
        Ok(Self {
            name: security_key_process_name(device_uid, SecurityKeyProcessRole::GuestFrontend)?,
            execution_ref,
            role: SecurityKeyProcessRole::GuestFrontend,
            domain: "user",
        })
    }

    /// Borrow the deterministic Process resource name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Borrow the Guest execution reference.
    pub const fn execution_ref(&self) -> &ResourceRef {
        &self.execution_ref
    }

    /// Return the fixed Process role.
    pub const fn role(&self) -> SecurityKeyProcessRole {
        self.role
    }

    /// Return the fixed Guest user domain.
    pub const fn domain(&self) -> &'static str {
        self.domain
    }
}

impl fmt::Debug for FrontendProcessDeclaration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FrontendProcessDeclaration")
            .field("name", &self.name)
            .field("execution_ref", &self.execution_ref)
            .field("role", &self.role)
            .field("domain", &self.domain)
            .finish()
    }
}

/// Process declaration failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessDeclarationError {
    /// The execution context was not a Guest.
    WrongExecutionRef,
    /// The UID did not have the canonical UUID shape.
    InvalidUid,
}

impl fmt::Display for ProcessDeclarationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::WrongExecutionRef => "security-key-frontend-requires-guest",
            Self::InvalidUid => "security-key-process-uid-invalid",
        })
    }
}

impl std::error::Error for ProcessDeclarationError {}

/// Derive the required `device-<uid-short>-sk-*` Process resource name.
pub fn security_key_process_name(
    device_uid: &ResourceUid,
    role: SecurityKeyProcessRole,
) -> Result<String, ProcessDeclarationError> {
    let short = device_uid
        .as_str()
        .bytes()
        .filter(|byte| *byte != b'-')
        .take(12)
        .collect::<Vec<_>>();
    if short.len() != 12 {
        return Err(ProcessDeclarationError::InvalidUid);
    }
    let component = match role {
        SecurityKeyProcessRole::HostRelay => "sk-relay",
        SecurityKeyProcessRole::GuestFrontend => "sk-frontend",
    };
    let short = String::from_utf8(short).map_err(|_| ProcessDeclarationError::InvalidUid)?;
    Ok(format!("device-{short}-{component}"))
}
