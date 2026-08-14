//! Display controller lifecycle and finalizer state.

use crate::{
    FINALIZER, WaylandSpecError,
    policy::{FilterInput, WaylandPolicy},
    principal::{PrincipalLease, PrincipalPool},
    process::{LaunchTicket, ProcessObservation},
    spec::WaylandSessionSpec,
};
use d2b_contracts::v3::{ResourceRef, ZoneId};
use std::collections::BTreeMap;

/// Closed display-session lifecycle phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Dependencies or workers are not Ready.
    Pending,
    /// Both display workers are Ready.
    Ready,
    /// The session is usable only with a dependency warning.
    Degraded,
    /// Bounded retries are exhausted or admission failed.
    Failed,
    /// Finalization is in progress.
    Terminating,
}

/// Session condition projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SessionCondition {
    /// The GPU cross-domain endpoint is available.
    GpuEndpointAvailable,
    /// The user portal can issue a compositor grant.
    UserPortalReady,
    /// The compiled policy is current.
    PolicyApplied,
    /// The explicit cross-domain opt-in is present.
    CrossDomainTrusted,
    /// The host proxy is Ready.
    ProxyReady,
    /// The guest frontend is Ready.
    GuestFrontendReady,
    /// The finalizer is blocked by ambiguous process state.
    FinalizerAmbiguous,
    /// The GPU lacks the optional virgl video capability.
    VirglVideoUnsupported,
    /// All pre-provisioned dynamic principals are occupied.
    NoPrincipalAvailable,
}

/// Dependency observations supplied by Core.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DependencyState {
    /// GPU cross-domain endpoint status.
    pub gpu_ready: bool,
    /// User portal status.
    pub portal_ready: bool,
    /// Optional clipboard bridge status.
    pub clipboard_ready: bool,
    /// GPU virgl video capability status.
    pub virgl_video_supported: bool,
    /// Same-Zone identity observed for Core policy resolution.
    pub zone: Option<ZoneId>,
}

impl DependencyState {
    /// Construct all required dependencies as Ready.
    pub const fn ready() -> Self {
        Self {
            gpu_ready: true,
            portal_ready: true,
            clipboard_ready: true,
            virgl_video_supported: true,
            zone: None,
        }
    }
}

/// Bounded status written to the owning resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaylandSessionStatus {
    /// Current lifecycle phase.
    pub phase: Phase,
    /// Closed conditions currently true.
    pub conditions: Vec<SessionCondition>,
    /// Compiled policy digest.
    pub policy_digest: String,
    /// Authenticated Core policy generation.
    pub policy_generation: u64,
    /// Opaque principal account name, when allocated.
    pub principal: Option<String>,
    /// Fixed finalizer identifier.
    pub finalizer: &'static str,
}

/// Result of one reconcile pass.
#[derive(Debug, PartialEq, Eq)]
pub struct ReconcileResult {
    /// Projected status.
    pub status: WaylandSessionStatus,
    /// LaunchTicket when the workers need to be started.
    pub launch_ticket: Option<LaunchTicket>,
}

/// Finalization observations supplied by the Process controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinalizationInput {
    /// Whether graceful stop was requested.
    pub stop_requested: bool,
    /// Whether the proxy reached a verified terminal phase.
    pub proxy_terminal: bool,
    /// Whether Process deletion was confirmed.
    pub proxy_deleted: bool,
    /// Whether runtime Volume deletion was confirmed.
    pub volume_deleted: bool,
    /// Whether Core observed the dynamic principal release.
    pub principal_released: bool,
    /// Whether the compositor portal revoke completed.
    pub portal_revoked: bool,
    /// Whether the bounded grace period expired.
    pub grace_expired: bool,
}

/// Finalizer action decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinalizationDecision {
    /// Whether to issue a graceful Process stop.
    pub stop_proxy: bool,
    /// Whether the runtime Volume may now be deleted.
    pub delete_runtime_volume: bool,
    /// Whether the finalizer may be removed.
    pub remove_finalizer: bool,
    /// Projected lifecycle phase.
    pub phase: Phase,
    /// Whether the finalizer must retain ownership due to ambiguity.
    pub ambiguous: bool,
}

/// A Core-resolved WaylandPolicy snapshot.
///
/// Policy filters and the generation are carried together so reconciliation
/// cannot silently compile a default policy after the referenced resource
/// changes.  Construction is private to the Core adapter; callers receive
/// this value only after authenticated resource resolution.
#[derive(Clone, PartialEq, Eq)]
pub struct WaylandPolicySnapshot {
    policy_ref: ResourceRef,
    zone: ZoneId,
    generation: u64,
    defaults: FilterInput,
    zone_policy: FilterInput,
}

impl WaylandPolicySnapshot {
    pub(crate) fn from_core(
        policy_ref: ResourceRef,
        zone: ZoneId,
        generation: u64,
        defaults: FilterInput,
        zone_policy: FilterInput,
    ) -> Self {
        Self {
            policy_ref,
            zone,
            generation,
            defaults,
            zone_policy,
        }
    }

    fn compatibility(spec: &WaylandSessionSpec) -> Self {
        Self {
            policy_ref: spec.policy_ref().clone(),
            zone: ZoneId::parse("local").expect("compiled compatibility Zone"),
            generation: 0,
            defaults: FilterInput::default(),
            zone_policy: FilterInput::default(),
        }
    }

    /// Borrow the referenced policy resource.
    pub const fn policy_ref(&self) -> &ResourceRef {
        &self.policy_ref
    }

    /// Borrow the authenticated policy Zone.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Return the monotonic policy generation.
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    fn compile(
        &self,
        spec: &WaylandSessionSpec,
    ) -> Result<crate::policy::CompiledWaylandPolicy, WaylandSpecError> {
        if self.policy_ref != *spec.policy_ref() {
            return Err(WaylandSpecError::InvalidReference);
        }
        WaylandPolicy::compile(&self.defaults, &self.zone_policy, spec.filter()).map_err(
            |error| match error {
                crate::policy::PolicyCompileError::UnknownInterface(_) => {
                    WaylandSpecError::UnknownInterface
                }
                crate::policy::PolicyCompileError::BoundsExceeded => {
                    WaylandSpecError::InvalidReference
                }
            },
        )
    }
}

impl core::fmt::Debug for WaylandPolicySnapshot {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("WaylandPolicySnapshot(REDACTED)")
    }
}

/// Provider-issued principal release receipt.
pub struct PrincipalReleaseReceipt {
    session_key: String,
}

impl core::fmt::Debug for PrincipalReleaseReceipt {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("PrincipalReleaseReceipt(REDACTED)")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PolicyBinding {
    digest: String,
    generation: u64,
}

/// Zone-local display controller.
pub struct DisplayController {
    principal_pool: PrincipalPool,
    principals: BTreeMap<String, PrincipalLease>,
    active_policies: BTreeMap<String, PolicyBinding>,
}

impl DisplayController {
    /// Construct a controller with a bounded dynamic principal pool.
    pub fn new(pool_size: usize) -> Self {
        Self {
            principal_pool: PrincipalPool::new(std::iter::empty::<String>(), pool_size)
                .expect("display principal pool size is validated by the signed descriptor"),
            principals: BTreeMap::new(),
            active_policies: BTreeMap::new(),
        }
    }

    /// Reconcile one session from authenticated dependency and worker state.
    pub fn reconcile(
        &mut self,
        spec: &WaylandSessionSpec,
        dependencies: DependencyState,
        observation: ProcessObservation,
    ) -> Result<ReconcileResult, WaylandSpecError> {
        let policy = WaylandPolicySnapshot::compatibility(spec);
        self.reconcile_with_policy(spec, dependencies, observation, None, &policy)
    }

    /// Reconcile one session with grants issued by Core/Supervisor.
    pub fn reconcile_with_grants(
        &mut self,
        spec: &WaylandSessionSpec,
        dependencies: DependencyState,
        observation: ProcessObservation,
        grants: Option<crate::process::LaunchGrants>,
    ) -> Result<ReconcileResult, WaylandSpecError> {
        let policy = WaylandPolicySnapshot::compatibility(spec);
        self.reconcile_with_policy(spec, dependencies, observation, grants, &policy)
    }

    /// Reconcile using the authenticated Core-resolved WaylandPolicy.
    pub fn reconcile_with_policy(
        &mut self,
        spec: &WaylandSessionSpec,
        dependencies: DependencyState,
        observation: ProcessObservation,
        grants: Option<crate::process::LaunchGrants>,
        policy: &WaylandPolicySnapshot,
    ) -> Result<ReconcileResult, WaylandSpecError> {
        if !spec.cross_domain_trusted() {
            return Err(WaylandSpecError::CrossDomainUntrusted);
        }
        let compiled = policy.compile(spec)?;
        if let Some(zone) = &dependencies.zone
            && zone != policy.zone()
        {
            return Err(WaylandSpecError::InvalidReference);
        }
        let session_key = session_key(spec);
        let policy_binding = PolicyBinding {
            digest: compiled.digest().to_owned(),
            generation: policy.generation(),
        };
        if let Some(active) = self.active_policies.get(&session_key)
            && policy_binding.generation < active.generation
        {
            return Err(WaylandSpecError::InvalidReference);
        }
        let policy_changed = self
            .active_policies
            .get(&session_key)
            .is_some_and(|active| active != &policy_binding);
        if spec.virgl_video() && !dependencies.virgl_video_supported {
            return Ok(ReconcileResult {
                status: self.status(
                    Phase::Degraded,
                    compiled.digest().to_owned(),
                    policy.generation(),
                    String::new(),
                    vec![SessionCondition::VirglVideoUnsupported],
                ),
                launch_ticket: None,
            });
        }
        let conditions = [
            (
                dependencies.gpu_ready,
                SessionCondition::GpuEndpointAvailable,
            ),
            (dependencies.portal_ready, SessionCondition::UserPortalReady),
            (
                spec.cross_domain_trusted(),
                SessionCondition::CrossDomainTrusted,
            ),
            (observation.proxy_ready, SessionCondition::ProxyReady),
            (
                observation.frontend_ready,
                SessionCondition::GuestFrontendReady,
            ),
        ];
        if observation.proxy_failure_count >= 5 || observation.frontend_failure_count >= 5 {
            self.release_principal_if_owned(&session_key)?;
            self.active_policies.remove(&session_key);
            return Ok(ReconcileResult {
                status: self.status(
                    Phase::Failed,
                    compiled.digest().to_owned(),
                    policy.generation(),
                    String::new(),
                    conditions
                        .iter()
                        .filter_map(|(present, condition)| present.then_some(*condition))
                        .collect::<Vec<_>>(),
                ),
                launch_ticket: None,
            });
        }
        if !dependencies.gpu_ready || !dependencies.portal_ready {
            return Ok(ReconcileResult {
                status: self.status(
                    Phase::Pending,
                    compiled.digest().to_owned(),
                    policy.generation(),
                    String::new(),
                    conditions
                        .iter()
                        .filter_map(|(present, condition)| present.then_some(*condition))
                        .collect::<Vec<_>>(),
                ),
                launch_ticket: None,
            });
        }
        let needs_worker_launch =
            policy_changed || !observation.proxy_ready || !observation.frontend_ready;
        if needs_worker_launch && grants.is_none() {
            return Ok(ReconcileResult {
                status: self.status(
                    Phase::Pending,
                    compiled.digest().to_owned(),
                    policy.generation(),
                    String::new(),
                    conditions
                        .iter()
                        .filter_map(|(present, condition)| present.then_some(*condition))
                        .collect::<Vec<_>>(),
                ),
                launch_ticket: None,
            });
        }
        let principal = if let Some(lease) = self.principals.get(&session_key) {
            lease.principal().to_owned()
        } else {
            let lease = match self.principal_pool.acquire_dynamic() {
                Ok(lease) => lease,
                Err(crate::principal::PrincipalPoolError::NoPrincipalAvailable) => {
                    return Ok(ReconcileResult {
                        status: self.status(
                            Phase::Failed,
                            compiled.digest().to_owned(),
                            policy.generation(),
                            String::new(),
                            vec![SessionCondition::NoPrincipalAvailable],
                        ),
                        launch_ticket: None,
                    });
                }
                Err(_) => return Err(WaylandSpecError::InvalidReference),
            };
            let principal = lease.principal().to_owned();
            self.principals.insert(session_key.clone(), lease);
            principal
        };
        let launch_ticket = if needs_worker_launch {
            let grants = grants.expect("launch grants checked before principal allocation");
            let (compositor, gpu) = grants.into_parts();
            Some(
                LaunchTicket::new_with_generation(
                    compositor,
                    gpu,
                    compiled.digest().to_owned(),
                    policy.generation(),
                    spec.identity().label().to_owned(),
                )
                .expect("controller-generated launch ticket uses validated fields"),
            )
        } else {
            None
        };
        if !needs_worker_launch || launch_ticket.is_some() {
            self.active_policies
                .insert(session_key.clone(), policy_binding);
        }
        let phase = if observation.proxy_ready && observation.frontend_ready {
            Phase::Ready
        } else {
            Phase::Pending
        };
        Ok(ReconcileResult {
            status: self.status(
                phase,
                compiled.digest().to_owned(),
                policy.generation(),
                principal,
                conditions
                    .iter()
                    .filter_map(|(present, condition)| present.then_some(*condition))
                    .chain((!needs_worker_launch).then_some(SessionCondition::PolicyApplied))
                    .collect(),
            ),
            launch_ticket,
        })
    }

    /// Decide the safe finalizer action for one session.
    pub const fn finalize(input: FinalizationInput) -> FinalizationDecision {
        if input.grace_expired && !(input.proxy_terminal && input.proxy_deleted) {
            return FinalizationDecision {
                stop_proxy: false,
                delete_runtime_volume: false,
                remove_finalizer: false,
                phase: Phase::Degraded,
                ambiguous: true,
            };
        }
        if !input.stop_requested {
            return FinalizationDecision {
                stop_proxy: true,
                delete_runtime_volume: false,
                remove_finalizer: false,
                phase: Phase::Terminating,
                ambiguous: false,
            };
        }
        if !input.proxy_terminal || !input.proxy_deleted {
            return FinalizationDecision {
                stop_proxy: false,
                delete_runtime_volume: false,
                remove_finalizer: false,
                phase: Phase::Terminating,
                ambiguous: false,
            };
        }
        if !input.volume_deleted {
            return FinalizationDecision {
                stop_proxy: false,
                delete_runtime_volume: true,
                remove_finalizer: false,
                phase: Phase::Terminating,
                ambiguous: false,
            };
        }
        if !input.principal_released || !input.portal_revoked {
            return FinalizationDecision {
                stop_proxy: false,
                delete_runtime_volume: false,
                remove_finalizer: false,
                phase: Phase::Terminating,
                ambiguous: true,
            };
        }
        FinalizationDecision {
            stop_proxy: false,
            delete_runtime_volume: false,
            remove_finalizer: true,
            phase: Phase::Terminating,
            ambiguous: false,
        }
    }

    /// Return the fixed finalizer name.
    pub const fn finalizer() -> &'static str {
        FINALIZER
    }

    /// Display never declares a Provider-owned state Volume.
    pub const fn provider_state_set_empty() -> bool {
        true
    }

    /// Release a session's dynamic principal after verified Process cleanup.
    pub fn release_session_principal(
        &mut self,
        receipt: PrincipalReleaseReceipt,
    ) -> Result<(), crate::principal::PrincipalPoolError> {
        let Some(lease) = self.principals.get(&receipt.session_key) else {
            return Err(crate::principal::PrincipalPoolError::UnknownLease);
        };
        if !self.principal_pool.owns(lease) {
            return Err(crate::principal::PrincipalPoolError::UnknownLease);
        }
        let lease = self
            .principals
            .remove(&receipt.session_key)
            .ok_or(crate::principal::PrincipalPoolError::UnknownLease)?;
        self.active_policies.remove(&receipt.session_key);
        self.principal_pool.release(lease)
    }

    pub(crate) fn principal_release_receipt(
        &mut self,
        session_key: &str,
    ) -> Result<PrincipalReleaseReceipt, crate::principal::PrincipalPoolError> {
        if !self.principals.contains_key(session_key) {
            return Err(crate::principal::PrincipalPoolError::UnknownLease);
        }
        Ok(PrincipalReleaseReceipt {
            session_key: session_key.to_owned(),
        })
    }

    fn release_principal_if_owned(&mut self, session_key: &str) -> Result<(), WaylandSpecError> {
        let Some(lease) = self.principals.remove(session_key) else {
            return Ok(());
        };
        self.principal_pool
            .release(lease)
            .map_err(|_| WaylandSpecError::InvalidReference)?;
        Ok(())
    }

    fn status(
        &self,
        phase: Phase,
        policy_digest: String,
        policy_generation: u64,
        principal: String,
        conditions: Vec<SessionCondition>,
    ) -> WaylandSessionStatus {
        WaylandSessionStatus {
            phase,
            conditions,
            policy_digest,
            policy_generation,
            principal: (!principal.is_empty()).then_some(principal),
            finalizer: FINALIZER,
        }
    }
}

impl core::fmt::Debug for DisplayController {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("DisplayController")
            .field("principal_count", &self.principals.len())
            .field("available_principals", &self.principal_pool.available())
            .finish()
    }
}

fn session_key(spec: &WaylandSessionSpec) -> String {
    format!(
        "{}|{}|{}",
        spec.guest_ref().to_canonical_string(),
        spec.host_ref().to_canonical_string(),
        spec.user_ref().to_canonical_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        policy::FilterInput,
        process::{AttachmentGrantHandle, LaunchGrants, ProcessObservation},
        spec::DisplayIdentity,
    };

    fn session_spec() -> WaylandSessionSpec {
        WaylandSessionSpec::new(
            ResourceRef::parse("Guest/demo").unwrap(),
            ResourceRef::parse("Host/demo").unwrap(),
            ResourceRef::parse("User/alice").unwrap(),
            ResourceRef::parse("display-wayland.d2bus.org.WaylandPolicy/demo").unwrap(),
            DisplayIdentity::new("demo", "#112233", "#223344", "#334455").unwrap(),
            true,
        )
        .unwrap()
    }

    #[test]
    fn core_policy_snapshot_and_principal_receipt_are_consumed_by_controller() {
        let spec = session_spec();
        let policy = WaylandPolicySnapshot::from_core(
            spec.policy_ref().clone(),
            ZoneId::parse("local").unwrap(),
            7,
            FilterInput::default(),
            FilterInput::default(),
        );
        assert_eq!(policy.generation(), 7);

        let mut controller = DisplayController::new(1);
        let result = controller
            .reconcile_with_policy(
                &spec,
                DependencyState::ready(),
                ProcessObservation::ready(),
                None,
                &policy,
            )
            .unwrap();
        assert_eq!(result.status.phase, Phase::Ready);

        let receipt = controller
            .principal_release_receipt("Guest/demo|Host/demo|User/alice")
            .unwrap();
        controller.release_session_principal(receipt).unwrap();
    }

    #[test]
    fn policy_generation_change_requires_a_new_supervisor_launch() {
        let spec = session_spec();
        let first_policy = WaylandPolicySnapshot::from_core(
            spec.policy_ref().clone(),
            ZoneId::parse("local").unwrap(),
            7,
            FilterInput::default(),
            FilterInput::default(),
        );
        let second_policy = WaylandPolicySnapshot::from_core(
            spec.policy_ref().clone(),
            ZoneId::parse("local").unwrap(),
            8,
            FilterInput::default(),
            FilterInput::default(),
        );
        let mut controller = DisplayController::new(1);
        controller
            .reconcile_with_policy(
                &spec,
                DependencyState::ready(),
                ProcessObservation::ready(),
                None,
                &first_policy,
            )
            .unwrap();

        let pending = controller
            .reconcile_with_policy(
                &spec,
                DependencyState::ready(),
                ProcessObservation::ready(),
                None,
                &second_policy,
            )
            .unwrap();
        assert_eq!(pending.status.phase, Phase::Pending);
        assert!(!pending
            .status
            .conditions
            .contains(&SessionCondition::PolicyApplied));

        let launched = controller
            .reconcile_with_policy(
                &spec,
                DependencyState::ready(),
                ProcessObservation::ready(),
                Some(LaunchGrants::from_supervisor(
                    AttachmentGrantHandle::from_supervisor([9; 32]),
                    AttachmentGrantHandle::from_supervisor([10; 32]),
                )),
                &second_policy,
            )
            .unwrap();
        assert!(launched.launch_ticket.is_some());
        assert_eq!(launched.status.phase, Phase::Pending);

        let ready = controller
            .reconcile_with_policy(
                &spec,
                DependencyState::ready(),
                ProcessObservation::ready(),
                None,
                &second_policy,
            )
            .unwrap();
        assert_eq!(ready.status.phase, Phase::Ready);
        assert!(ready
            .status
            .conditions
            .contains(&SessionCondition::PolicyApplied));
    }
}
