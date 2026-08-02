//! Zone-wide EmergencyPolicy admission and fail-closed action planning.
//!
//! The policy contract describes the wire fields.  This module owns the
//! exactly-one Zone scope authority, the union of enabled policy contributions,
//! and the admission gate that runs before a new resource operation.

use std::collections::BTreeMap;

use d2b_contracts::v3::{
    EmergencyPolicySpec, EmergencyPolicyStatusResource, EmergencyScope, ResourceUid, Timestamp,
    effective_scope,
};

use super::{
    AuthorityError, AuthorityLease, AuthorityOwnerProof, AuthorityRequest, HostGlobalAuthorityIndex,
};

/// Closed actions emitted by an effective emergency scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EmergencyAction {
    /// Reject new resource admissions.
    StopNewAdmissions,
    /// Gracefully disconnect active ZoneLinks.
    DisconnectZoneLinks,
    /// Stop non-bootstrap Provider component Processes without deleting rows.
    StopProviderProcesses,
    /// Drain in-flight operations to the effective deadline.
    DrainOngoingOperations,
    /// Keep incident-held state and evidence intact.
    PreserveIncidentHeldState,
}

/// Closed emergency-policy refusal reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmergencyPolicyError {
    /// The scope authority or generic authority request failed.
    Authority(AuthorityError),
    /// The policy UID is already installed in this scope.
    DuplicatePolicy,
    /// The policy UID is not installed.
    UnknownPolicy,
    /// The effective scope rejects a new admission.
    AdmissionDenied,
    /// The scope is draining and cannot accept a new operation.
    DrainPending,
    /// A delete drain has not been explicitly confirmed complete.
    DrainIncomplete,
}

impl EmergencyPolicyError {
    /// Return the stable identity-free failure code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Authority(error) => error.code(),
            Self::DuplicatePolicy => "emergency-policy-duplicate",
            Self::UnknownPolicy => "emergency-policy-unknown",
            Self::AdmissionDenied => "emergency-admission-denied",
            Self::DrainPending => "emergency-drain-pending",
            Self::DrainIncomplete => "emergency-drain-incomplete",
        }
    }
}

impl core::fmt::Display for EmergencyPolicyError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for EmergencyPolicyError {}

impl From<AuthorityError> for EmergencyPolicyError {
    fn from(value: AuthorityError) -> Self {
        Self::Authority(value)
    }
}

/// Core-owned EmergencyPolicy authority for one Zone.
pub struct EmergencyPolicyAuthority {
    zone_uid: ResourceUid,
    lease: AuthorityLease,
    policies: BTreeMap<ResourceUid, EmergencyPolicySpec>,
    active: bool,
    deletion_requested: Option<ResourceUid>,
    drain_pending: bool,
    activated_at: Option<Timestamp>,
    deactivated_at: Option<Timestamp>,
    drain_completed_at: Option<Timestamp>,
}

impl core::fmt::Debug for EmergencyPolicyAuthority {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("EmergencyPolicyAuthority")
            .field("has_zone", &true)
            .field("policy_count", &self.policies.len())
            .field("active", &self.active)
            .field("drain_pending", &self.drain_pending)
            .finish()
    }
}

impl EmergencyPolicyAuthority {
    /// Admit the exactly-one EmergencyPolicy scope before applying effects.
    pub fn admit(
        index: &mut HostGlobalAuthorityIndex,
        zone_uid: ResourceUid,
        policy_uid: ResourceUid,
        policy: EmergencyPolicySpec,
        owner_proof: AuthorityOwnerProof,
    ) -> Result<Self, EmergencyPolicyError> {
        let lease = index.admit_authority(AuthorityRequest::emergency_policy(
            zone_uid.clone(),
            owner_proof,
        )?)?;
        let active = policy.enabled();
        let mut policies = BTreeMap::new();
        policies.insert(policy_uid, policy);
        Ok(Self {
            zone_uid,
            lease,
            policies,
            active,
            deletion_requested: None,
            drain_pending: false,
            activated_at: None,
            deactivated_at: None,
            drain_completed_at: None,
        })
    }

    /// Borrow the Zone identity used by the trusted controller.
    pub const fn zone_uid(&self) -> &ResourceUid {
        &self.zone_uid
    }

    /// Install a second policy contribution under this already-admitted
    /// handler owner.  It does not create a second scope authority.
    pub fn install_policy(
        &mut self,
        policy_uid: ResourceUid,
        policy: EmergencyPolicySpec,
    ) -> Result<(), EmergencyPolicyError> {
        if self.drain_pending {
            return Err(EmergencyPolicyError::DrainPending);
        }
        if self.policies.contains_key(&policy_uid) {
            return Err(EmergencyPolicyError::DuplicatePolicy);
        }
        self.policies.insert(policy_uid, policy);
        self.refresh_active(None);
        Ok(())
    }

    /// Replace an installed policy spec without changing its scope authority.
    pub fn update_policy(
        &mut self,
        policy_uid: &ResourceUid,
        policy: EmergencyPolicySpec,
    ) -> Result<(), EmergencyPolicyError> {
        if self.drain_pending {
            return Err(EmergencyPolicyError::DrainPending);
        }
        if !self.policies.contains_key(policy_uid) {
            return Err(EmergencyPolicyError::UnknownPolicy);
        }
        self.policies.insert(policy_uid.clone(), policy);
        self.refresh_active(None);
        Ok(())
    }

    /// Toggle one policy and record a bounded status transition time.
    pub fn set_enabled(
        &mut self,
        policy_uid: &ResourceUid,
        enabled: bool,
        now: Timestamp,
    ) -> Result<(), EmergencyPolicyError> {
        if self.drain_pending {
            return Err(EmergencyPolicyError::DrainPending);
        }
        let current = self
            .policies
            .get(policy_uid)
            .ok_or(EmergencyPolicyError::UnknownPolicy)?;
        if current.enabled() == enabled {
            return Ok(());
        }
        let replacement = EmergencyPolicySpec::new(
            enabled,
            current.scope(),
            current.drain_deadline_seconds(),
            current.reason(),
        )
        .map_err(|_| EmergencyPolicyError::UnknownPolicy)?;
        self.policies.insert(policy_uid.clone(), replacement);
        let was_active = self.active;
        self.refresh_active(Some(now.clone()));
        if !was_active && self.active {
            self.activated_at = Some(now);
            self.drain_completed_at = None;
        } else if was_active && !self.active {
            self.deactivated_at = Some(now);
        }
        Ok(())
    }

    /// Compute the union and tightest deadline of enabled policies.
    pub fn effective_scope(&self) -> Option<(EmergencyScope, u32)> {
        effective_scope(self.policies.values())
    }

    /// Return the closed action plan for the current effective scope.
    pub fn actions(&self) -> Vec<EmergencyAction> {
        let Some((scope, _)) = self.effective_scope() else {
            return Vec::new();
        };
        let mut actions = Vec::with_capacity(5);
        if scope.stop_new_admissions() {
            actions.push(EmergencyAction::StopNewAdmissions);
        }
        if scope.disconnect_zone_links() {
            actions.push(EmergencyAction::DisconnectZoneLinks);
        }
        if scope.stop_provider_processes() {
            actions.push(EmergencyAction::StopProviderProcesses);
        }
        if scope.drain_ongoing_operations() {
            actions.push(EmergencyAction::DrainOngoingOperations);
        }
        actions.push(EmergencyAction::PreserveIncidentHeldState);
        actions
    }

    /// Reject a new resource admission when the effective gate is enabled.
    pub fn check_admission(&self) -> Result<(), EmergencyPolicyError> {
        if self
            .effective_scope()
            .is_some_and(|(scope, _)| scope.stop_new_admissions())
        {
            Err(EmergencyPolicyError::AdmissionDenied)
        } else if self.drain_pending {
            Err(EmergencyPolicyError::DrainPending)
        } else {
            Ok(())
        }
    }

    /// Return the minimum drain deadline, if any policy is enabled.
    pub fn drain_deadline_seconds(&self) -> Option<u32> {
        self.effective_scope().map(|(_, deadline)| deadline)
    }

    /// Whether any enabled policy currently has effects applied.
    pub const fn active(&self) -> bool {
        self.active
    }

    /// Request deletion of one policy and begin its drain if it is active.
    pub fn request_delete(
        &mut self,
        policy_uid: &ResourceUid,
        now: Timestamp,
    ) -> Result<(), EmergencyPolicyError> {
        if !self.policies.contains_key(policy_uid) {
            return Err(EmergencyPolicyError::UnknownPolicy);
        }
        let was_active = self.active;
        if self.policies[policy_uid].enabled() {
            let current = self.policies.get(policy_uid).expect("checked above");
            let replacement = EmergencyPolicySpec::new(
                false,
                current.scope(),
                current.drain_deadline_seconds(),
                current.reason(),
            )
            .map_err(|_| EmergencyPolicyError::UnknownPolicy)?;
            self.policies.insert(policy_uid.clone(), replacement);
        }
        self.deletion_requested = Some(policy_uid.clone());
        self.refresh_active(Some(now.clone()));
        self.drain_pending = was_active;
        if was_active && !self.active {
            self.deactivated_at = Some(now);
        }
        Ok(())
    }

    /// Whether the emergency drain finalizer must remain installed.
    pub const fn drain_pending(&self) -> bool {
        self.drain_pending
    }

    /// Complete a requested policy deletion after the drain boundary.
    pub fn complete_delete(
        &mut self,
        index: &mut HostGlobalAuthorityIndex,
    ) -> Result<(), EmergencyPolicyError> {
        if self.drain_pending {
            return Err(EmergencyPolicyError::DrainIncomplete);
        }
        let policy_uid = self
            .deletion_requested
            .take()
            .ok_or(EmergencyPolicyError::UnknownPolicy)?;
        self.policies.remove(&policy_uid);
        if self.policies.is_empty() {
            index.release_authority(&self.lease)?;
        }
        Ok(())
    }

    /// Confirm the ongoing drain and make final deletion eligible.
    pub fn confirm_drain(&mut self, now: Timestamp) -> Result<(), EmergencyPolicyError> {
        if !self.drain_pending {
            return Err(EmergencyPolicyError::DrainIncomplete);
        }
        self.drain_pending = false;
        self.drain_completed_at = Some(now);
        Ok(())
    }

    /// Return the status projection for one installed policy.
    pub fn status(
        &self,
        policy_uid: &ResourceUid,
    ) -> Result<EmergencyPolicyStatusResource, EmergencyPolicyError> {
        let policy = self
            .policies
            .get(policy_uid)
            .ok_or(EmergencyPolicyError::UnknownPolicy)?;
        Ok(EmergencyPolicyStatusResource::new(
            policy.enabled(),
            self.activated_at.clone(),
            self.deactivated_at.clone(),
            self.drain_completed_at.clone(),
        ))
    }

    fn refresh_active(&mut self, _now: Option<Timestamp>) {
        self.active = self.policies.values().any(EmergencyPolicySpec::enabled);
    }
}

/// Short name used by the fixed core handler catalog.
pub type EmergencyPolicyHandler = EmergencyPolicyAuthority;

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::v3::{EmergencyScope, ResourceGeneration};

    fn uid(value: &str) -> ResourceUid {
        ResourceUid::parse(value).unwrap()
    }

    fn owner(value: &str) -> AuthorityOwnerProof {
        AuthorityOwnerProof::new(uid(value), ResourceGeneration::new(1).unwrap())
    }

    fn policy(
        enabled: bool,
        scope: EmergencyScope,
        deadline: u32,
        reason: &str,
    ) -> EmergencyPolicySpec {
        EmergencyPolicySpec::new(enabled, scope, deadline, reason).unwrap()
    }

    #[test]
    fn only_one_emergency_scope_authority_is_admitted_per_zone() {
        let mut index = HostGlobalAuthorityIndex::default();
        EmergencyPolicyAuthority::admit(
            &mut index,
            uid("123e4567-e89b-42d3-a456-426614174000"),
            uid("223e4567-e89b-42d3-a456-426614174001"),
            policy(false, EmergencyScope::default(), 30, ""),
            owner("323e4567-e89b-42d3-a456-426614174002"),
        )
        .unwrap();
        let error = EmergencyPolicyAuthority::admit(
            &mut index,
            uid("123e4567-e89b-42d3-a456-426614174000"),
            uid("423e4567-e89b-42d3-a456-426614174003"),
            policy(false, EmergencyScope::default(), 30, ""),
            owner("523e4567-e89b-42d3-a456-426614174004"),
        )
        .unwrap_err();
        assert_eq!(error.code(), "duplicateConflict");
    }

    #[test]
    fn enabled_policy_union_keeps_other_contributions_on_partial_deactivation() {
        let mut index = HostGlobalAuthorityIndex::default();
        let mut authority = EmergencyPolicyAuthority::admit(
            &mut index,
            uid("623e4567-e89b-42d3-a456-426614174005"),
            uid("723e4567-e89b-42d3-a456-426614174006"),
            policy(
                true,
                EmergencyScope::new(true, false, false, true),
                30,
                "operator reason",
            ),
            owner("823e4567-e89b-42d3-a456-426614174007"),
        )
        .unwrap();
        let second_uid = uid("923e4567-e89b-42d3-a456-426614174008");
        authority
            .install_policy(
                second_uid.clone(),
                policy(true, EmergencyScope::new(false, true, true, false), 5, ""),
            )
            .unwrap();
        let (scope, deadline) = authority.effective_scope().unwrap();
        assert!(scope.stop_new_admissions());
        assert!(scope.disconnect_zone_links());
        assert!(scope.stop_provider_processes());
        assert!(scope.drain_ongoing_operations());
        assert_eq!(deadline, 5);
        assert_eq!(
            authority.check_admission(),
            Err(EmergencyPolicyError::AdmissionDenied)
        );
        assert!(
            authority
                .actions()
                .contains(&EmergencyAction::StopProviderProcesses)
        );

        authority
            .set_enabled(
                &uid("723e4567-e89b-42d3-a456-426614174006"),
                false,
                Timestamp::parse("2026-08-02T09:51:14.140Z").unwrap(),
            )
            .unwrap();
        let (scope, deadline) = authority.effective_scope().unwrap();
        assert!(!scope.stop_new_admissions());
        assert!(scope.disconnect_zone_links());
        assert!(scope.stop_provider_processes());
        assert_eq!(deadline, 5);
        assert_eq!(authority.check_admission(), Ok(()));
        assert!(authority.status(&second_uid).unwrap().active());
    }

    #[test]
    fn active_delete_retains_finalizer_until_drain_confirmation() {
        let mut index = HostGlobalAuthorityIndex::default();
        let policy_uid = uid("a23e4567-e89b-42d3-a456-426614174009");
        let mut authority = EmergencyPolicyAuthority::admit(
            &mut index,
            uid("b23e4567-e89b-42d3-a456-426614174010"),
            policy_uid.clone(),
            policy(true, EmergencyScope::new(false, false, true, true), 10, ""),
            owner("c23e4567-e89b-42d3-a456-426614174011"),
        )
        .unwrap();
        authority
            .request_delete(
                &policy_uid,
                Timestamp::parse("2026-08-02T09:51:14.140Z").unwrap(),
            )
            .unwrap();
        assert!(authority.drain_pending());
        assert_eq!(
            authority.complete_delete(&mut index),
            Err(EmergencyPolicyError::DrainIncomplete)
        );
        authority
            .confirm_drain(Timestamp::parse("2026-08-02T09:51:15.140Z").unwrap())
            .unwrap();
        authority.complete_delete(&mut index).unwrap();
        // The finalizer release makes the original Zone scope available again.
        EmergencyPolicyAuthority::admit(
            &mut index,
            uid("b23e4567-e89b-42d3-a456-426614174012"),
            uid("d23e4567-e89b-42d3-a456-426614174013"),
            policy(false, EmergencyScope::default(), 30, ""),
            owner("e23e4567-e89b-42d3-a456-426614174014"),
        )
        .unwrap();
    }

    #[test]
    fn reason_is_not_present_in_status_or_debug() {
        let mut index = HostGlobalAuthorityIndex::default();
        let authority = EmergencyPolicyAuthority::admit(
            &mut index,
            uid("c33e4567-e89b-42d3-a456-426614174018"),
            uid("d33e4567-e89b-42d3-a456-426614174019"),
            policy(
                true,
                EmergencyScope::new(true, false, false, false),
                30,
                "private reason canary",
            ),
            owner("e33e4567-e89b-42d3-a456-426614174020"),
        )
        .unwrap();
        let status = format!(
            "{:?}",
            authority
                .status(&uid("d33e4567-e89b-42d3-a456-426614174019"))
                .unwrap()
        );
        assert!(!status.contains("private reason canary"));
        assert!(!format!("{authority:?}").contains("private reason canary"));
    }
}
