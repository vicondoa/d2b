//! Display controller lifecycle and finalizer state.

use crate::{
    FINALIZER, WaylandSpecError,
    policy::{FilterInput, WaylandPolicy},
    principal::{PrincipalLease, PrincipalPool},
    process::{AttachmentGrantHandle, LaunchTicket, ProcessObservation},
    spec::WaylandSessionSpec,
};
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DependencyState {
    /// GPU cross-domain endpoint status.
    pub gpu_ready: bool,
    /// User portal status.
    pub portal_ready: bool,
    /// Optional clipboard bridge status.
    pub clipboard_ready: bool,
    /// GPU virgl video capability status.
    pub virgl_video_supported: bool,
}

impl DependencyState {
    /// Construct all required dependencies as Ready.
    pub const fn ready() -> Self {
        Self {
            gpu_ready: true,
            portal_ready: true,
            clipboard_ready: true,
            virgl_video_supported: true,
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
    /// Opaque principal account name, when allocated.
    pub principal: Option<String>,
    /// Fixed finalizer identifier.
    pub finalizer: &'static str,
}

/// Result of one reconcile pass.
#[derive(Debug, Clone, PartialEq, Eq)]
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

/// Zone-local display controller.
pub struct DisplayController {
    principal_pool: PrincipalPool,
    principals: BTreeMap<String, PrincipalLease>,
}

impl DisplayController {
    /// Construct a controller with a bounded dynamic principal pool.
    pub fn new(pool_size: usize) -> Self {
        Self {
            principal_pool: PrincipalPool::new(std::iter::empty::<String>(), pool_size)
                .expect("display principal pool size is validated by the signed descriptor"),
            principals: BTreeMap::new(),
        }
    }

    /// Reconcile one session from authenticated dependency and worker state.
    pub fn reconcile(
        &mut self,
        spec: &WaylandSessionSpec,
        dependencies: DependencyState,
        observation: ProcessObservation,
    ) -> Result<ReconcileResult, WaylandSpecError> {
        if !spec.cross_domain_trusted() {
            return Err(WaylandSpecError::CrossDomainUntrusted);
        }
        let compiled = WaylandPolicy::compile(
            &FilterInput::default(),
            &FilterInput::default(),
            spec.filter(),
        )
        .map_err(|error| match error {
            crate::policy::PolicyCompileError::UnknownInterface(_) => {
                WaylandSpecError::UnknownInterface
            }
            crate::policy::PolicyCompileError::BoundsExceeded => WaylandSpecError::InvalidReference,
        })?;
        if spec.virgl_video() && !dependencies.virgl_video_supported {
            return Ok(ReconcileResult {
                status: self.status(
                    Phase::Degraded,
                    compiled.digest().to_owned(),
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
        if observation.proxy_failure_count >= 5 {
            return Ok(ReconcileResult {
                status: self.status(
                    Phase::Failed,
                    compiled.digest().to_owned(),
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
                    String::new(),
                    conditions
                        .iter()
                        .filter_map(|(present, condition)| present.then_some(*condition))
                        .collect::<Vec<_>>(),
                ),
                launch_ticket: None,
            });
        }
        let session_key = spec.identity().label().to_owned();
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
        let launch_ticket = (!observation.proxy_ready).then(|| {
            LaunchTicket::new(
                AttachmentGrantHandle::from_core([1; 32]),
                AttachmentGrantHandle::from_core([2; 32]),
                compiled.digest().to_owned(),
                spec.identity().label().to_owned(),
            )
            .expect("controller-generated launch ticket uses validated fields")
        });
        let phase = if observation.proxy_ready && observation.frontend_ready {
            Phase::Ready
        } else {
            Phase::Pending
        };
        Ok(ReconcileResult {
            status: self.status(
                phase,
                compiled.digest().to_owned(),
                principal,
                conditions
                    .iter()
                    .filter_map(|(present, condition)| present.then_some(*condition))
                    .chain([SessionCondition::PolicyApplied])
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
        FinalizationDecision {
            stop_proxy: false,
            delete_runtime_volume: false,
            remove_finalizer: true,
            phase: Phase::Ready,
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
        session_key: &str,
    ) -> Result<(), crate::principal::PrincipalPoolError> {
        let lease = self
            .principals
            .remove(session_key)
            .ok_or(crate::principal::PrincipalPoolError::UnknownLease)?;
        self.principal_pool.release(lease)
    }

    fn status(
        &self,
        phase: Phase,
        policy_digest: String,
        principal: String,
        conditions: Vec<SessionCondition>,
    ) -> WaylandSessionStatus {
        WaylandSessionStatus {
            phase,
            conditions,
            policy_digest,
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
