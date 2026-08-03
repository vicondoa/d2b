//! Core-owned lifecycle for one ResourceImport projection Service.
//!
//! Admission produces the only identity this controller may use.  The
//! controller then plans an effect-free, child-safe lifecycle around that
//! identity: a projection is created only after a bound lease exists, route
//! loss revokes it before the lease is released, and import deletion drains
//! authored Bindings before any projection cleanup.

use std::collections::BTreeSet;

use d2b_contracts::v3::{
    FinalizerId, RESOURCE_IMPORT_DRAIN_FINALIZER, ResourceImportConditionType, ResourceRef,
    ResourceTypeName, SchemaFingerprint, SemanticProjectionProtocolVersion,
};

use crate::export_import::{AdmittedImport, ProjectionServiceIdentity};

/// The lifecycle phase observed for a projection Service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProjectionPhase {
    /// No projection row exists.
    Absent,
    /// The projection row exists but its Provider has not reported readiness.
    Pending,
    /// The projection is available to same-Zone Bindings.
    Ready,
    /// The remote route or lease was revoked.
    Revoked,
    /// Import deletion is waiting for all authored Bindings to drain.
    Draining,
    /// Cleanup has completed and the projection row is gone.
    Deleted,
}

impl ProjectionPhase {
    /// Return the stable status spelling.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Absent => "absent",
            Self::Pending => "pending",
            Self::Ready => "ready",
            Self::Revoked => "revoked",
            Self::Draining => "draining",
            Self::Deleted => "deleted",
        }
    }

    /// Return whether a projection row is expected to exist.
    pub const fn is_present(self) -> bool {
        matches!(
            self,
            Self::Pending | Self::Ready | Self::Revoked | Self::Draining
        )
    }
}

/// Whether the remote export route is usable for this reconcile pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProjectionRouteState {
    /// The ZoneLink route and remote advertisement are reachable.
    Reachable,
    /// The route is gone and the import must be revoked.
    Lost,
    /// The remote export explicitly revoked the lease.
    Revoked,
}

impl ProjectionRouteState {
    const fn is_reachable(self) -> bool {
        matches!(self, Self::Reachable)
    }
}

/// State of the remote lease as observed by Core.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProjectionLeaseState {
    /// No lease has been admitted for this import.
    Unbound,
    /// A lease is active and may ground one projection.
    Bound,
    /// The remote lease is revoked but its local record still needs release.
    Revoked,
    /// The lease release has been durably acknowledged.
    Released,
}

/// Closed failures from projection construction and reconciliation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionLifecycleError {
    /// The owner row was not a ResourceImport.
    InvalidImportOwner,
    /// The admitted identity did not describe a same-qualified Service.
    InvalidProjectionIdentity,
    /// More than one projection row was observed for one import.
    MultipleProjections,
    /// The observed projection was not owned by this ResourceImport.
    OwnerRefMismatch,
    /// The observed projection used a different projection ResourceRef.
    ProjectionRefMismatch,
    /// The observed projection used a different qualified Service type.
    ServiceTypeMismatch,
    /// A binding reference was repeated in one observation.
    DuplicateBindingReference,
    /// A projection row was observed after its terminal deletion phase.
    InvalidDeletedProjection,
    /// A deletion race attempted to recreate a draining projection.
    DrainInProgress,
    /// The fixed Core finalizer could not be parsed.
    FinalizerInvalid,
}

impl ProjectionLifecycleError {
    /// Return the stable, identity-free diagnostic label.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidImportOwner => "projection-import-owner-invalid",
            Self::InvalidProjectionIdentity => "projection-identity-invalid",
            Self::MultipleProjections => "projection-multiple-rows",
            Self::OwnerRefMismatch => "projection-owner-ref-mismatch",
            Self::ProjectionRefMismatch => "projection-ref-mismatch",
            Self::ServiceTypeMismatch => "projection-service-type-mismatch",
            Self::DuplicateBindingReference => "projection-binding-reference-duplicate",
            Self::InvalidDeletedProjection => "projection-deleted-row-invalid",
            Self::DrainInProgress => "projection-drain-in-progress",
            Self::FinalizerInvalid => "projection-finalizer-invalid",
        }
    }
}

impl core::fmt::Display for ProjectionLifecycleError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for ProjectionLifecycleError {}

/// The immutable Service row Core derives from one admitted import.
///
/// This type has no constructor accepting an arbitrary owner or authority
/// mode.  Its owner and all semantic metadata come from
/// [`AdmittedImport::projection_identity`].
#[derive(Clone, PartialEq, Eq)]
pub struct ProjectionService {
    identity: ProjectionServiceIdentity,
}

impl ProjectionService {
    /// Construct the one projection Service for an admitted import.
    pub fn from_admitted_import(
        admitted: &AdmittedImport,
        import_ref: &ResourceRef,
    ) -> Result<Self, ProjectionLifecycleError> {
        let identity = admitted
            .projection_identity(import_ref)
            .map_err(|_| ProjectionLifecycleError::InvalidImportOwner)?;
        validate_identity(&identity)?;
        Ok(Self { identity })
    }

    /// Alias emphasizing that the import identity is the source of truth.
    pub fn new(
        admitted: &AdmittedImport,
        import_ref: &ResourceRef,
    ) -> Result<Self, ProjectionLifecycleError> {
        Self::from_admitted_import(admitted, import_ref)
    }

    /// Borrow the complete admitted identity.
    pub const fn identity(&self) -> &ProjectionServiceIdentity {
        &self.identity
    }

    /// Borrow the same-qualified semantic Service type.
    pub const fn service_type(&self) -> &ResourceTypeName {
        self.identity.service_type()
    }

    /// Borrow the local projection Service reference.
    pub const fn projection_ref(&self) -> &ResourceRef {
        self.identity.projection_ref()
    }

    /// Borrow the immutable ResourceImport owner reference.
    pub const fn owner_ref(&self) -> &ResourceRef {
        self.identity.owner_ref()
    }

    /// Borrow the projection schema fingerprint.
    pub const fn projection_schema_fingerprint(&self) -> &SchemaFingerprint {
        self.identity.projection_schema_fingerprint()
    }

    /// Borrow the semantic factory fingerprint.
    pub const fn factory_fingerprint(&self) -> &SchemaFingerprint {
        self.identity.factory_fingerprint()
    }

    /// Borrow the semantic projection protocol version.
    pub const fn projection_protocol_version(&self) -> &SemanticProjectionProtocolVersion {
        self.identity.projection_protocol_version()
    }

    /// Return whether this row is import-owned.
    pub fn is_import_owned(&self) -> bool {
        self.owner_ref().resource_type().as_str() == "ResourceImport"
    }

    /// Import-owned projections cannot become export authorities.
    pub const fn can_be_export_target(&self) -> bool {
        false
    }

    /// Import-owned projections cannot become backing authorities.
    pub const fn can_be_backing_reference(&self) -> bool {
        false
    }

    /// Positive control for callers that classify authority-bearing rows.
    pub const fn is_export_authority(&self) -> bool {
        false
    }

    /// Positive control for callers that classify backing-bearing rows.
    pub const fn is_backing_authority(&self) -> bool {
        false
    }
}

impl core::fmt::Debug for ProjectionService {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ProjectionService")
            .field("identity", &self.identity)
            .finish()
    }
}

/// An observed projection row supplied by the resource-store relist.
///
/// Unlike [`ProjectionService`], this type accepts arbitrary observed
/// metadata so Core can reject a tampered or foreign owner instead of
/// repairing it into an authority.
#[derive(Clone, PartialEq, Eq)]
pub struct ProjectionServiceObservation {
    projection_ref: ResourceRef,
    service_type: ResourceTypeName,
    owner_ref: Option<ResourceRef>,
    projection_schema_fingerprint: SchemaFingerprint,
    factory_fingerprint: SchemaFingerprint,
    projection_protocol_version: SemanticProjectionProtocolVersion,
    phase: ProjectionPhase,
}

impl ProjectionServiceObservation {
    /// Construct one complete observed projection row.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        projection_ref: ResourceRef,
        service_type: ResourceTypeName,
        owner_ref: Option<ResourceRef>,
        projection_schema_fingerprint: SchemaFingerprint,
        factory_fingerprint: SchemaFingerprint,
        projection_protocol_version: SemanticProjectionProtocolVersion,
        phase: ProjectionPhase,
    ) -> Self {
        Self {
            projection_ref,
            service_type,
            owner_ref,
            projection_schema_fingerprint,
            factory_fingerprint,
            projection_protocol_version,
            phase,
        }
    }

    /// Build an observed row from a Core-derived projection.
    pub fn from_service(service: &ProjectionService, phase: ProjectionPhase) -> Self {
        Self {
            projection_ref: service.projection_ref().clone(),
            service_type: service.service_type().clone(),
            owner_ref: Some(service.owner_ref().clone()),
            projection_schema_fingerprint: service.projection_schema_fingerprint().clone(),
            factory_fingerprint: service.factory_fingerprint().clone(),
            projection_protocol_version: service.projection_protocol_version().clone(),
            phase,
        }
    }

    /// Borrow the observed projection reference.
    pub const fn projection_ref(&self) -> &ResourceRef {
        &self.projection_ref
    }

    /// Borrow the observed Service type.
    pub const fn service_type(&self) -> &ResourceTypeName {
        &self.service_type
    }

    /// Borrow the observed owner reference.
    pub const fn owner_ref(&self) -> Option<&ResourceRef> {
        self.owner_ref.as_ref()
    }

    /// Borrow the observed projection schema fingerprint.
    pub const fn projection_schema_fingerprint(&self) -> &SchemaFingerprint {
        &self.projection_schema_fingerprint
    }

    /// Borrow the observed factory fingerprint.
    pub const fn factory_fingerprint(&self) -> &SchemaFingerprint {
        &self.factory_fingerprint
    }

    /// Borrow the observed protocol version.
    pub const fn projection_protocol_version(&self) -> &SemanticProjectionProtocolVersion {
        &self.projection_protocol_version
    }

    /// Return the observed lifecycle phase.
    pub const fn phase(&self) -> ProjectionPhase {
        self.phase
    }

    fn has_same_resource_identity(&self, desired: &ProjectionService) -> bool {
        self.projection_ref == *desired.projection_ref()
            && self.service_type == *desired.service_type()
    }

    fn has_same_metadata(&self, desired: &ProjectionService) -> bool {
        self.owner_ref.as_ref() == Some(desired.owner_ref())
            && self.projection_schema_fingerprint == *desired.projection_schema_fingerprint()
            && self.factory_fingerprint == *desired.factory_fingerprint()
            && self.projection_protocol_version == *desired.projection_protocol_version()
    }
}

impl core::fmt::Debug for ProjectionServiceObservation {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ProjectionServiceObservation")
            .field("projection_ref", &self.projection_ref)
            .field("service_type", &self.service_type)
            .field("has_owner_ref", &self.owner_ref.is_some())
            .field("phase", &self.phase)
            .finish_non_exhaustive()
    }
}

/// Complete store observation consumed by the projection planner.
#[derive(Clone, PartialEq, Eq)]
pub struct ProjectionObservation {
    projections: Vec<ProjectionServiceObservation>,
    route: ProjectionRouteState,
    lease: ProjectionLeaseState,
    binding_references: BTreeSet<ResourceRef>,
    provider_owned_children: u32,
    deletion_requested: bool,
}

impl ProjectionObservation {
    /// Construct a complete observation.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        projections: impl IntoIterator<Item = ProjectionServiceObservation>,
        route: ProjectionRouteState,
        lease: ProjectionLeaseState,
        binding_references: impl IntoIterator<Item = ResourceRef>,
        provider_owned_children: u32,
        deletion_requested: bool,
    ) -> Result<Self, ProjectionLifecycleError> {
        let binding_references = binding_references.into_iter().collect::<Vec<_>>();
        let unique_references = binding_references.iter().cloned().collect::<BTreeSet<_>>();
        if unique_references.len() != binding_references.len() {
            return Err(ProjectionLifecycleError::DuplicateBindingReference);
        }
        Ok(Self {
            projections: projections.into_iter().collect(),
            route,
            lease,
            binding_references: unique_references,
            provider_owned_children,
            deletion_requested,
        })
    }

    /// Construct the normal missing-projection observation.
    pub fn missing() -> Self {
        Self::new(
            [],
            ProjectionRouteState::Reachable,
            ProjectionLeaseState::Bound,
            [],
            0,
            false,
        )
        .expect("the missing projection observation is valid")
    }

    /// Construct a one-row observation for a Core-derived projection.
    pub fn present(service: &ProjectionService, phase: ProjectionPhase) -> Self {
        Self::new(
            [ProjectionServiceObservation::from_service(service, phase)],
            ProjectionRouteState::Reachable,
            ProjectionLeaseState::Bound,
            [],
            0,
            false,
        )
        .expect("the one-row projection observation is valid")
    }

    /// Construct an import-deletion observation with authored Binding refs.
    pub fn deleting(
        service: Option<&ProjectionService>,
        phase: ProjectionPhase,
        lease: ProjectionLeaseState,
        binding_references: impl IntoIterator<Item = ResourceRef>,
        provider_owned_children: u32,
    ) -> Result<Self, ProjectionLifecycleError> {
        Self::new(
            service
                .map(|service| [ProjectionServiceObservation::from_service(service, phase)])
                .into_iter()
                .flatten(),
            ProjectionRouteState::Reachable,
            lease,
            binding_references,
            provider_owned_children,
            true,
        )
    }

    /// Borrow all rows found for the import's projection key.
    pub fn projections(&self) -> &[ProjectionServiceObservation] {
        &self.projections
    }

    /// Return the route state.
    pub const fn route(&self) -> ProjectionRouteState {
        self.route
    }

    /// Return the lease state.
    pub const fn lease(&self) -> ProjectionLeaseState {
        self.lease
    }

    /// Borrow the sorted, unique authored Binding references.
    pub fn binding_references(&self) -> impl Iterator<Item = &ResourceRef> {
        self.binding_references.iter()
    }

    /// Return the number of authored Bindings still referencing the projection.
    pub fn binding_reference_count(&self) -> usize {
        self.binding_references.len()
    }

    /// Return the number of remaining Provider-owned children.
    pub const fn provider_owned_children(&self) -> u32 {
        self.provider_owned_children
    }

    /// Return whether ResourceImport deletion was requested.
    pub const fn deletion_requested(&self) -> bool {
        self.deletion_requested
    }
}

impl core::fmt::Debug for ProjectionObservation {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ProjectionObservation")
            .field("projection_count", &self.projections.len())
            .field("route", &self.route)
            .field("lease", &self.lease)
            .field("binding_reference_count", &self.binding_references.len())
            .field("provider_owned_children", &self.provider_owned_children)
            .field("deletion_requested", &self.deletion_requested)
            .finish()
    }
}

/// One durable mutation or wait emitted by the projection controller.
#[derive(Clone, PartialEq, Eq)]
pub enum ProjectionAction {
    /// Create the one projection Service with Core-derived metadata.
    CreateService { service: ProjectionService },
    /// Update only the projection Service's admitted identity metadata.
    UpdateService { service: ProjectionService },
    /// Mark the projection available again after a validated reconnect.
    MarkReady { projection_ref: ResourceRef },
    /// Mark the projection revoked before releasing its remote lease.
    MarkRevoked { projection_ref: ResourceRef },
    /// Mark the projection draining before any teardown effect.
    MarkDraining { projection_ref: ResourceRef },
    /// Revoke the remote lease after the local projection is marked revoked.
    RevokeLease,
    /// Release the remote lease during import finalization.
    ReleaseLease,
    /// Delete remaining Provider-owned projection children.
    DeleteProviderChildren { count: u32 },
    /// Delete the projection Service after lease and Binding cleanup.
    DeleteService { projection_ref: ResourceRef },
    /// Clear the ResourceImport finalizer after every cleanup stage completed.
    ClearImportFinalizer,
    /// Wait for the remote lease before creating a local projection.
    WaitForLease,
    /// Keep the import finalizer while authored Bindings still reference the projection.
    WaitForBindings { count: usize },
}

impl ProjectionAction {
    /// Return whether this action mutates an import-owned projection row.
    pub const fn mutates_projection(&self) -> bool {
        matches!(
            self,
            Self::CreateService { .. }
                | Self::UpdateService { .. }
                | Self::MarkReady { .. }
                | Self::MarkRevoked { .. }
                | Self::MarkDraining { .. }
                | Self::DeleteService { .. }
        )
    }

    /// Return whether this action can delete the projection Service.
    pub const fn deletes_projection(&self) -> bool {
        matches!(self, Self::DeleteService { .. })
    }

    /// Core never creates or deletes operator-authored Binding rows.
    pub const fn mutates_binding(&self) -> bool {
        false
    }

    /// Core never grants export or backing authority to a projection.
    pub const fn grants_authority(&self) -> bool {
        false
    }
}

impl core::fmt::Debug for ProjectionAction {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::CreateService { service } => formatter
                .debug_struct("CreateService")
                .field("service", service)
                .finish(),
            Self::UpdateService { service } => formatter
                .debug_struct("UpdateService")
                .field("service", service)
                .finish(),
            Self::MarkReady { projection_ref } => formatter
                .debug_struct("MarkReady")
                .field("projection_ref", projection_ref)
                .finish(),
            Self::MarkRevoked { projection_ref } => formatter
                .debug_struct("MarkRevoked")
                .field("projection_ref", projection_ref)
                .finish(),
            Self::MarkDraining { projection_ref } => formatter
                .debug_struct("MarkDraining")
                .field("projection_ref", projection_ref)
                .finish(),
            Self::DeleteService { projection_ref } => formatter
                .debug_struct("DeleteService")
                .field("projection_ref", projection_ref)
                .finish(),
            Self::DeleteProviderChildren { count } => formatter
                .debug_struct("DeleteProviderChildren")
                .field("count", count)
                .finish(),
            Self::WaitForBindings { count } => formatter
                .debug_struct("WaitForBindings")
                .field("count", count)
                .finish(),
            Self::RevokeLease => formatter.write_str("RevokeLease"),
            Self::ReleaseLease => formatter.write_str("ReleaseLease"),
            Self::ClearImportFinalizer => formatter.write_str("ClearImportFinalizer"),
            Self::WaitForLease => formatter.write_str("WaitForLease"),
        }
    }
}

/// Effect-free projection lifecycle plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionPlan {
    phase: ProjectionPhase,
    condition: Option<ResourceImportConditionType>,
    actions: Vec<ProjectionAction>,
}

impl ProjectionPlan {
    /// Return the phase Core expects after this plan is durably applied.
    pub const fn phase(&self) -> ProjectionPhase {
        self.phase
    }

    /// Return the visible ResourceImport condition, when one is pending.
    pub const fn condition(&self) -> Option<ResourceImportConditionType> {
        self.condition
    }

    /// Borrow mutations and wait decisions in deterministic order.
    pub fn actions(&self) -> &[ProjectionAction] {
        &self.actions
    }

    /// Return whether no further Core-owned action is needed for this observation.
    pub fn is_converged(&self) -> bool {
        self.actions.is_empty()
    }

    /// Return whether cleanup is blocked by authored Binding references.
    pub fn is_waiting_for_bindings(&self) -> bool {
        self.condition == Some(ResourceImportConditionType::BindingReferencesRemain)
    }

    /// Return whether the plan contains the projection deletion mutation.
    pub fn deletes_projection(&self) -> bool {
        self.actions
            .iter()
            .any(ProjectionAction::deletes_projection)
    }
}

/// Core planner for one admitted ResourceImport.
#[derive(Clone)]
pub struct ProjectionController {
    identity: ProjectionServiceIdentity,
    finalizer: FinalizerId,
}

impl ProjectionController {
    /// Bind the controller to an admitted import and its committed owner row.
    pub fn new(
        admitted: &AdmittedImport,
        import_ref: &ResourceRef,
    ) -> Result<Self, ProjectionLifecycleError> {
        let service = ProjectionService::from_admitted_import(admitted, import_ref)?;
        let finalizer = FinalizerId::parse(RESOURCE_IMPORT_DRAIN_FINALIZER)
            .map_err(|_| ProjectionLifecycleError::FinalizerInvalid)?;
        Ok(Self {
            identity: service.identity().clone(),
            finalizer,
        })
    }

    /// Alias for callers naming the controller after its import owner.
    pub fn from_admitted_import(
        admitted: &AdmittedImport,
        import_ref: &ResourceRef,
    ) -> Result<Self, ProjectionLifecycleError> {
        Self::new(admitted, import_ref)
    }

    /// Borrow the bound admitted identity.
    pub const fn identity(&self) -> &ProjectionServiceIdentity {
        &self.identity
    }

    /// Borrow the committed ResourceImport owner reference.
    pub const fn import_ref(&self) -> &ResourceRef {
        self.identity.owner_ref()
    }

    /// Borrow the Core finalizer that remains until projection cleanup.
    pub const fn finalizer(&self) -> &FinalizerId {
        &self.finalizer
    }

    /// Build the desired projection Service. Every call is deterministic.
    pub fn desired_service(&self) -> ProjectionService {
        ProjectionService {
            identity: self.identity.clone(),
        }
    }

    /// Reconcile one complete store observation without releasing effects.
    pub fn reconcile(
        &self,
        observation: &ProjectionObservation,
    ) -> Result<ProjectionPlan, ProjectionLifecycleError> {
        let existing = match observation.projections.as_slice() {
            [] => None,
            [projection] => Some(projection),
            _ => return Err(ProjectionLifecycleError::MultipleProjections),
        };
        let desired = self.desired_service();
        if let Some(projection) = existing {
            self.validate_existing(projection, &desired)?;
        }

        if observation.deletion_requested {
            return self.plan_deletion(observation, existing);
        }

        if !observation.route.is_reachable() {
            return Ok(self.plan_revocation(observation, existing));
        }

        if existing.is_some_and(|projection| projection.phase() == ProjectionPhase::Draining) {
            return Err(ProjectionLifecycleError::DrainInProgress);
        }

        if observation.lease != ProjectionLeaseState::Bound {
            return Ok(ProjectionPlan {
                phase: existing.map_or(ProjectionPhase::Absent, |projection| projection.phase()),
                condition: None,
                actions: vec![ProjectionAction::WaitForLease],
            });
        }

        let Some(existing) = existing else {
            return Ok(ProjectionPlan {
                phase: ProjectionPhase::Pending,
                condition: None,
                actions: vec![ProjectionAction::CreateService { service: desired }],
            });
        };

        if !existing.has_same_metadata(&desired) {
            return Ok(ProjectionPlan {
                phase: ProjectionPhase::Pending,
                condition: None,
                actions: vec![ProjectionAction::UpdateService { service: desired }],
            });
        }

        if existing.phase() == ProjectionPhase::Revoked {
            return Ok(ProjectionPlan {
                phase: ProjectionPhase::Pending,
                condition: None,
                actions: vec![ProjectionAction::MarkReady {
                    projection_ref: desired.projection_ref().clone(),
                }],
            });
        }

        Ok(ProjectionPlan {
            phase: existing.phase(),
            condition: None,
            actions: Vec::new(),
        })
    }

    fn validate_existing(
        &self,
        existing: &ProjectionServiceObservation,
        desired: &ProjectionService,
    ) -> Result<(), ProjectionLifecycleError> {
        if existing.phase() == ProjectionPhase::Deleted {
            return Err(ProjectionLifecycleError::InvalidDeletedProjection);
        }
        if existing.owner_ref() != Some(desired.owner_ref()) {
            return Err(ProjectionLifecycleError::OwnerRefMismatch);
        }
        if existing.projection_ref() != desired.projection_ref() {
            return Err(ProjectionLifecycleError::ProjectionRefMismatch);
        }
        if existing.service_type() != desired.service_type() {
            return Err(ProjectionLifecycleError::ServiceTypeMismatch);
        }
        if !existing.has_same_resource_identity(desired) {
            return Err(ProjectionLifecycleError::InvalidProjectionIdentity);
        }
        Ok(())
    }

    fn plan_revocation(
        &self,
        observation: &ProjectionObservation,
        existing: Option<&ProjectionServiceObservation>,
    ) -> ProjectionPlan {
        if existing.is_some_and(|projection| projection.phase() == ProjectionPhase::Draining) {
            return ProjectionPlan {
                phase: ProjectionPhase::Draining,
                condition: None,
                actions: Vec::new(),
            };
        }
        if let Some(projection) = existing
            && projection.phase() != ProjectionPhase::Revoked
        {
            return ProjectionPlan {
                phase: ProjectionPhase::Revoked,
                condition: Some(ResourceImportConditionType::Degraded),
                actions: vec![ProjectionAction::MarkRevoked {
                    projection_ref: projection.projection_ref().clone(),
                }],
            };
        }
        let actions = match observation.lease {
            ProjectionLeaseState::Bound => vec![ProjectionAction::RevokeLease],
            ProjectionLeaseState::Revoked => vec![ProjectionAction::ReleaseLease],
            ProjectionLeaseState::Unbound | ProjectionLeaseState::Released => Vec::new(),
        };
        ProjectionPlan {
            phase: ProjectionPhase::Revoked,
            condition: Some(ResourceImportConditionType::Degraded),
            actions,
        }
    }

    fn plan_deletion(
        &self,
        observation: &ProjectionObservation,
        existing: Option<&ProjectionServiceObservation>,
    ) -> Result<ProjectionPlan, ProjectionLifecycleError> {
        if observation.binding_reference_count() > 0 {
            let condition = Some(ResourceImportConditionType::BindingReferencesRemain);
            if let Some(projection) = existing
                && projection.phase() != ProjectionPhase::Draining
            {
                return Ok(ProjectionPlan {
                    phase: ProjectionPhase::Draining,
                    condition,
                    actions: vec![ProjectionAction::MarkDraining {
                        projection_ref: projection.projection_ref().clone(),
                    }],
                });
            }
            return Ok(ProjectionPlan {
                phase: ProjectionPhase::Draining,
                condition,
                actions: vec![ProjectionAction::WaitForBindings {
                    count: observation.binding_reference_count(),
                }],
            });
        }

        if let Some(projection) = existing
            && projection.phase() != ProjectionPhase::Draining
        {
            return Ok(ProjectionPlan {
                phase: ProjectionPhase::Draining,
                condition: None,
                actions: vec![ProjectionAction::MarkDraining {
                    projection_ref: projection.projection_ref().clone(),
                }],
            });
        }

        match observation.lease {
            ProjectionLeaseState::Bound | ProjectionLeaseState::Revoked => {
                return Ok(ProjectionPlan {
                    phase: ProjectionPhase::Draining,
                    condition: None,
                    actions: vec![ProjectionAction::ReleaseLease],
                });
            }
            ProjectionLeaseState::Unbound | ProjectionLeaseState::Released => {}
        }

        if observation.provider_owned_children > 0 {
            return Ok(ProjectionPlan {
                phase: ProjectionPhase::Draining,
                condition: None,
                actions: vec![ProjectionAction::DeleteProviderChildren {
                    count: observation.provider_owned_children,
                }],
            });
        }

        if let Some(projection) = existing {
            return Ok(ProjectionPlan {
                phase: ProjectionPhase::Draining,
                condition: None,
                actions: vec![ProjectionAction::DeleteService {
                    projection_ref: projection.projection_ref().clone(),
                }],
            });
        }

        Ok(ProjectionPlan {
            phase: ProjectionPhase::Deleted,
            condition: None,
            actions: vec![ProjectionAction::ClearImportFinalizer],
        })
    }
}

fn validate_identity(identity: &ProjectionServiceIdentity) -> Result<(), ProjectionLifecycleError> {
    if identity.owner_ref().resource_type().as_str() != "ResourceImport" {
        return Err(ProjectionLifecycleError::InvalidImportOwner);
    }
    if identity.projection_ref().resource_type() != identity.service_type() {
        return Err(ProjectionLifecycleError::InvalidProjectionIdentity);
    }
    Ok(())
}

impl core::fmt::Debug for ProjectionController {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ProjectionController")
            .field("identity", &self.identity)
            .field("has_finalizer", &true)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use d2b_contracts::v3::{
        BindingTargetType, ConsumerZonePolicy, ExportArbitration, Exportability,
        ResourceExportSpec, ResourceImportSpec, ResourceName, ResourceTypeName, SchemaFingerprint,
        SemanticProjectionProtocolVersion, execution_policy::BoundedToken,
    };

    use super::*;
    use crate::export_import::admit_import;

    fn fingerprint(digit: char) -> SchemaFingerprint {
        SchemaFingerprint::parse(format!("sha256:{}", digit.to_string().repeat(64))).unwrap()
    }

    fn factory() -> d2b_contracts::v3::ProjectionFactory {
        d2b_contracts::v3::ProjectionFactory::new(
            ResourceTypeName::parse("security-key.d2bus.org.SecurityKeyService").unwrap(),
            ResourceTypeName::parse("security-key.d2bus.org.SecurityKeyBinding").unwrap(),
            [],
            [BindingTargetType::Guest],
            fingerprint('a'),
            fingerprint('b'),
            Exportability::ExplicitExport,
        )
        .unwrap()
    }

    fn admitted_import() -> (AdmittedImport, ResourceRef) {
        let factory = factory();
        let export = ResourceExportSpec::minimal(
            ResourceRef::parse("security-key.d2bus.org.SecurityKeyService/owner").unwrap(),
            factory.service_type().clone(),
            factory.projection_schema_fingerprint().clone(),
            factory.factory_fingerprint().clone(),
            vec![BoundedToken::parse("use").unwrap()],
            ExportArbitration::Exclusive,
            ConsumerZonePolicy::new(
                vec![d2b_contracts::v3::ZoneId::parse("child").unwrap()],
                vec![BoundedToken::parse("use").unwrap()],
            )
            .unwrap(),
        )
        .unwrap();
        let import = ResourceImportSpec::minimal(
            ResourceRef::parse("ZoneLink/parent").unwrap(),
            "parent/owner",
            factory.service_type().clone(),
            factory.projection_schema_fingerprint().clone(),
            factory.factory_fingerprint().clone(),
            ResourceName::parse("projection").unwrap(),
            vec![BoundedToken::parse("use").unwrap()],
        )
        .unwrap();
        let import_ref = ResourceRef::parse("ResourceImport/share").unwrap();
        (
            admit_import(&import, &export, &factory, &factory).unwrap(),
            import_ref,
        )
    }

    fn controller() -> ProjectionController {
        let (admitted, import_ref) = admitted_import();
        ProjectionController::new(&admitted, &import_ref).unwrap()
    }

    fn binding_ref(name: &str) -> ResourceRef {
        ResourceRef::parse(&format!("security-key.d2bus.org.SecurityKeyBinding/{name}")).unwrap()
    }

    fn action_is<F>(plan: &ProjectionPlan, predicate: F) -> bool
    where
        F: Fn(&ProjectionAction) -> bool,
    {
        plan.actions().iter().any(predicate)
    }

    #[test]
    fn create_uses_one_same_qualified_service_and_import_owner() {
        let controller = controller();
        let desired = controller.desired_service();
        assert_eq!(
            desired.projection_ref().resource_type(),
            desired.service_type()
        );
        assert_eq!(
            desired.owner_ref().to_canonical_string(),
            "ResourceImport/share"
        );
        assert!(desired.is_import_owned());
        assert!(!desired.can_be_export_target());
        assert!(!desired.can_be_backing_reference());
        assert!(!desired.is_export_authority());
        assert!(!desired.is_backing_authority());

        let plan = controller
            .reconcile(&ProjectionObservation::missing())
            .unwrap();
        assert_eq!(plan.actions().len(), 1);
        match &plan.actions()[0] {
            ProjectionAction::CreateService { service } => {
                assert_eq!(service.owner_ref(), desired.owner_ref());
                assert_eq!(
                    service.projection_schema_fingerprint(),
                    desired.projection_schema_fingerprint()
                );
                assert_eq!(service.factory_fingerprint(), desired.factory_fingerprint());
                assert_eq!(
                    service.projection_protocol_version(),
                    desired.projection_protocol_version()
                );
            }
            other => panic!("unexpected projection action: {other:?}"),
        }
    }

    #[test]
    fn replay_and_restart_converge_without_a_second_projection() {
        let controller = controller();
        let desired = controller.desired_service();
        let create = controller
            .reconcile(&ProjectionObservation::missing())
            .unwrap();
        let created = match &create.actions()[0] {
            ProjectionAction::CreateService { service } => service,
            other => panic!("unexpected projection action: {other:?}"),
        };
        assert_eq!(created, &desired);

        let persisted = ProjectionObservation::present(created, ProjectionPhase::Pending);
        assert!(controller.reconcile(&persisted).unwrap().is_converged());
        let restarted = ProjectionController::new(
            &admitted_import().0,
            &ResourceRef::parse("ResourceImport/share").unwrap(),
        )
        .unwrap();
        assert!(restarted.reconcile(&persisted).unwrap().is_converged());
    }

    #[test]
    fn changed_admitted_fingerprints_produce_a_deterministic_update() {
        let controller = controller();
        let desired = controller.desired_service();
        let altered = ProjectionServiceObservation::new(
            desired.projection_ref().clone(),
            desired.service_type().clone(),
            Some(desired.owner_ref().clone()),
            fingerprint('c'),
            desired.factory_fingerprint().clone(),
            desired.projection_protocol_version().clone(),
            ProjectionPhase::Ready,
        );
        let observation = ProjectionObservation::new(
            [altered],
            ProjectionRouteState::Reachable,
            ProjectionLeaseState::Bound,
            [],
            0,
            false,
        )
        .unwrap();
        let plan = controller.reconcile(&observation).unwrap();
        assert_eq!(plan.actions().len(), 1);
        match &plan.actions()[0] {
            ProjectionAction::UpdateService { service } => {
                assert_eq!(
                    service.projection_schema_fingerprint(),
                    desired.projection_schema_fingerprint()
                );
                assert_eq!(service.owner_ref(), desired.owner_ref());
            }
            other => panic!("unexpected projection action: {other:?}"),
        }
    }

    #[test]
    fn revocation_marks_before_releasing_the_lease() {
        let controller = controller();
        let desired = controller.desired_service();
        let observation = ProjectionObservation::new(
            [ProjectionServiceObservation::from_service(
                &desired,
                ProjectionPhase::Ready,
            )],
            ProjectionRouteState::Revoked,
            ProjectionLeaseState::Bound,
            [],
            0,
            false,
        )
        .unwrap();
        let mark = controller.reconcile(&observation).unwrap();
        assert_eq!(
            mark.actions(),
            &[ProjectionAction::MarkRevoked {
                projection_ref: desired.projection_ref().clone()
            }]
        );
        assert_eq!(
            mark.condition(),
            Some(ResourceImportConditionType::Degraded)
        );

        let revoked = ProjectionObservation::new(
            [ProjectionServiceObservation::from_service(
                &desired,
                ProjectionPhase::Revoked,
            )],
            ProjectionRouteState::Revoked,
            ProjectionLeaseState::Bound,
            [],
            0,
            false,
        )
        .unwrap();
        assert_eq!(
            controller.reconcile(&revoked).unwrap().actions(),
            &[ProjectionAction::RevokeLease]
        );
    }

    #[test]
    fn deletion_marks_draining_and_waits_without_touching_bindings() {
        let controller = controller();
        let desired = controller.desired_service();
        let observation = ProjectionObservation::deleting(
            Some(&desired),
            ProjectionPhase::Ready,
            ProjectionLeaseState::Bound,
            [binding_ref("mic")],
            1,
        )
        .unwrap();
        let mark = controller.reconcile(&observation).unwrap();
        assert_eq!(
            mark.actions(),
            &[ProjectionAction::MarkDraining {
                projection_ref: desired.projection_ref().clone()
            }]
        );
        assert!(mark.is_waiting_for_bindings());
        assert!(!mark.deletes_projection());
        assert!(!mark.actions().iter().any(ProjectionAction::mutates_binding));
        assert!(
            !mark
                .actions()
                .iter()
                .any(ProjectionAction::grants_authority)
        );

        let waiting = ProjectionObservation::deleting(
            Some(&desired),
            ProjectionPhase::Draining,
            ProjectionLeaseState::Bound,
            [binding_ref("mic")],
            1,
        )
        .unwrap();
        let wait = controller.reconcile(&waiting).unwrap();
        assert_eq!(
            wait.actions(),
            &[ProjectionAction::WaitForBindings { count: 1 }]
        );
        assert!(wait.is_waiting_for_bindings());
        assert!(!wait.deletes_projection());
    }

    #[test]
    fn deletion_orders_lease_children_projection_then_finalizer() {
        let controller = controller();
        let desired = controller.desired_service();
        let drain = ProjectionObservation::deleting(
            Some(&desired),
            ProjectionPhase::Draining,
            ProjectionLeaseState::Bound,
            [],
            2,
        )
        .unwrap();
        assert_eq!(
            controller.reconcile(&drain).unwrap().actions(),
            &[ProjectionAction::ReleaseLease]
        );

        let children = ProjectionObservation::deleting(
            Some(&desired),
            ProjectionPhase::Draining,
            ProjectionLeaseState::Released,
            [],
            2,
        )
        .unwrap();
        assert_eq!(
            controller.reconcile(&children).unwrap().actions(),
            &[ProjectionAction::DeleteProviderChildren { count: 2 }]
        );

        let projection = ProjectionObservation::deleting(
            Some(&desired),
            ProjectionPhase::Draining,
            ProjectionLeaseState::Released,
            [],
            0,
        )
        .unwrap();
        assert_eq!(
            controller.reconcile(&projection).unwrap().actions(),
            &[ProjectionAction::DeleteService {
                projection_ref: desired.projection_ref().clone()
            }]
        );

        let finalizer = ProjectionObservation::deleting(
            None,
            ProjectionPhase::Deleted,
            ProjectionLeaseState::Released,
            [],
            0,
        )
        .unwrap();
        assert_eq!(
            controller.reconcile(&finalizer).unwrap().actions(),
            &[ProjectionAction::ClearImportFinalizer]
        );
    }

    #[test]
    fn foreign_owner_and_duplicate_rows_fail_closed() {
        let controller = controller();
        let desired = controller.desired_service();
        let foreign = ProjectionServiceObservation::new(
            desired.projection_ref().clone(),
            desired.service_type().clone(),
            Some(ResourceRef::parse("ResourceImport/other").unwrap()),
            desired.projection_schema_fingerprint().clone(),
            desired.factory_fingerprint().clone(),
            desired.projection_protocol_version().clone(),
            ProjectionPhase::Ready,
        );
        let foreign_observation = ProjectionObservation::new(
            [foreign],
            ProjectionRouteState::Reachable,
            ProjectionLeaseState::Bound,
            [],
            0,
            false,
        )
        .unwrap();
        assert_eq!(
            controller.reconcile(&foreign_observation),
            Err(ProjectionLifecycleError::OwnerRefMismatch)
        );

        let row = ProjectionServiceObservation::from_service(&desired, ProjectionPhase::Ready);
        let duplicate_observation = ProjectionObservation::new(
            [row.clone(), row],
            ProjectionRouteState::Reachable,
            ProjectionLeaseState::Bound,
            [],
            0,
            false,
        )
        .unwrap();
        assert_eq!(
            controller.reconcile(&duplicate_observation),
            Err(ProjectionLifecycleError::MultipleProjections)
        );
    }

    #[test]
    fn finalizer_is_the_fixed_import_drain_finalizer() {
        let controller = controller();
        assert_eq!(
            controller.finalizer().as_str(),
            RESOURCE_IMPORT_DRAIN_FINALIZER
        );
        assert!(!format!("{controller:?}").contains("ResourceImport/share"));
        assert_eq!(
            SemanticProjectionProtocolVersion::parse("1.0")
                .unwrap()
                .as_str(),
            "1.0"
        );
    }

    #[test]
    fn duplicate_binding_observations_are_rejected() {
        let binding = binding_ref("mic");
        assert_eq!(
            ProjectionObservation::new(
                [],
                ProjectionRouteState::Reachable,
                ProjectionLeaseState::Bound,
                [binding.clone(), binding],
                0,
                true,
            ),
            Err(ProjectionLifecycleError::DuplicateBindingReference)
        );
    }

    #[test]
    fn action_helpers_are_positive_controls() {
        let controller = controller();
        let plan = controller
            .reconcile(&ProjectionObservation::missing())
            .unwrap();
        assert!(action_is(&plan, ProjectionAction::mutates_projection));
        assert!(!action_is(&plan, ProjectionAction::mutates_binding));
        assert!(!action_is(&plan, ProjectionAction::grants_authority));
    }
}
