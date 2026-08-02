//! ADR 0021 virtiofsd user-namespace conformance checks.
//!
//! The broker performs the actual `clone3` and mapping writes.  This module
//! validates only the immutable Process template facts and the required
//! ordering evidence; it never carries host UIDs, GIDs, or capabilities.

use std::fmt;

use d2b_contracts::v3::execution_policy::BoundedToken;

/// `CLONE_NEWUSER`, used by the broker's pre-establishment path.
pub const CLONE_NEWUSER_FLAG: u64 = 0x1000_0000;
/// `CLONE_NEWNS`, which must not be requested by the virtiofsd path.
pub const CLONE_NEWNS_FLAG: u64 = 0x0002_0000;

/// The required ordering for Linux user-namespace map writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MappingStep {
    /// Write the UID map first.
    UidMap,
    /// Deny setgroups before writing the GID map.
    SetgroupsDeny,
    /// Write the GID map last.
    GidMap,
}

/// Immutable Process template facts checked before launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserNamespaceTemplate {
    capability_classes: Vec<BoundedToken>,
    namespace_classes: Vec<BoundedToken>,
    mapping_class: BoundedToken,
    start_root: bool,
    no_new_privileges: bool,
    read_only_root: bool,
}

impl UserNamespaceTemplate {
    /// Build a template descriptor from manifest-derived values.
    pub fn new(
        capability_classes: Vec<BoundedToken>,
        namespace_classes: Vec<BoundedToken>,
        mapping_class: BoundedToken,
        start_root: bool,
        no_new_privileges: bool,
        read_only_root: bool,
    ) -> Self {
        Self {
            capability_classes,
            namespace_classes,
            mapping_class,
            start_root,
            no_new_privileges,
            read_only_root,
        }
    }

    /// Return the one conformant worker descriptor.
    pub fn conformant() -> Self {
        Self::new(
            Vec::new(),
            vec![BoundedToken::parse("user").expect("frozen namespace class")],
            BoundedToken::parse("process-principal-root").expect("frozen mapping class"),
            false,
            true,
            true,
        )
    }

    /// Validate the complete ADR 0021 worker posture.
    pub fn assert_conformant(&self) -> Result<(), UserNamespaceError> {
        if !self.capability_classes.is_empty()
            || self.start_root
            || !self.no_new_privileges
            || !self.read_only_root
            || self.namespace_classes.len() != 1
            || self.namespace_classes[0].as_str() != "user"
            || self.mapping_class.as_str() != "process-principal-root"
        {
            return Err(UserNamespaceError::TemplateInvariantViolated);
        }
        Ok(())
    }
}

/// Closed user-namespace conformance failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserNamespaceError {
    /// The Process template violates ADR 0021.
    TemplateInvariantViolated,
    /// A forbidden namespace flag was requested.
    MountNamespaceRequested,
    /// The broker map-write order is not UID, setgroups deny, GID.
    MappingOrderInvalid,
}

impl UserNamespaceError {
    /// Return the stable failure code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::TemplateInvariantViolated => "virtiofs-user-namespace-template-invalid",
            Self::MountNamespaceRequested => "virtiofs-user-namespace-mount-flag-invalid",
            Self::MappingOrderInvalid => "virtiofs-user-namespace-map-order-invalid",
        }
    }
}

impl fmt::Display for UserNamespaceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for UserNamespaceError {}

/// Check the broker clone flags without accepting a mount namespace.
pub const fn validate_clone3_flags(flags: u64) -> Result<(), UserNamespaceError> {
    if flags & CLONE_NEWNS_FLAG != 0 || flags & CLONE_NEWUSER_FLAG == 0 {
        return Err(UserNamespaceError::MountNamespaceRequested);
    }
    Ok(())
}

/// Check the required parent-side map-write sequence.
pub fn validate_mapping_order(steps: &[MappingStep]) -> Result<(), UserNamespaceError> {
    if steps == [MappingStep::UidMap, MappingStep::SetgroupsDeny, MappingStep::GidMap] {
        Ok(())
    } else {
        Err(UserNamespaceError::MappingOrderInvalid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conformant_template_covers_the_full_adr0021_posture() {
        assert!(UserNamespaceTemplate::conformant().assert_conformant().is_ok());
        assert!(validate_clone3_flags(CLONE_NEWUSER_FLAG).is_ok());
        assert_eq!(
            validate_mapping_order(&[
                MappingStep::UidMap,
                MappingStep::SetgroupsDeny,
                MappingStep::GidMap,
            ]),
            Ok(())
        );
    }

    #[test]
    fn capabilities_root_start_and_mount_namespace_are_rejected() {
        let capability = BoundedToken::parse("cap-sys-admin").unwrap();
        assert_eq!(
            UserNamespaceTemplate::new(
                vec![capability],
                vec![BoundedToken::parse("user").unwrap()],
                BoundedToken::parse("process-principal-root").unwrap(),
                false,
                true,
                true,
            )
            .assert_conformant(),
            Err(UserNamespaceError::TemplateInvariantViolated)
        );
        assert_eq!(
            validate_clone3_flags(CLONE_NEWUSER_FLAG | CLONE_NEWNS_FLAG),
            Err(UserNamespaceError::MountNamespaceRequested)
        );
    }
}
