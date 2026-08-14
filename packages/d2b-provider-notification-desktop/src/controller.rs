//! Process placement controller for desktop notification components.

use crate::SessionEvidence;
use d2b_contracts::v3::{EvidenceClass, Locality, ResourceRef, ZoneId};
use d2b_provider_toolkit::AuthenticatedSessionRouteBinding;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

use crate::Category;

const DISPLAY_PROVIDER_REF: &str = "Provider/display-wayland";
const DISPLAY_SERVICE_PACKAGE: &str = "d2b.display.v3";
const MAX_GUEST_SOURCES: usize = 16;

/// Readiness state reported by the authenticated display dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayDependencyState {
    /// The display endpoint accepted the current policy generation.
    Ready,
    /// The display endpoint is still starting.
    Pending,
    /// The display endpoint cannot serve this generation.
    Failed,
}

/// Same-Zone, route-authenticated display dependency evidence.
///
/// The constructor accepts only a route binding produced by the canonical
/// ComponentSession authority.  Provider configuration cannot manufacture a
/// ready dependency by passing a Provider reference or boolean.
#[derive(Clone, PartialEq, Eq)]
pub struct DisplayDependencyEvidence {
    provider_ref: ResourceRef,
    zone: ZoneId,
    host_execution_ref: ResourceRef,
    user_ref: ResourceRef,
    provider_generation: u64,
    reconnect_generation: u64,
    controller_generation: u64,
    state: DisplayDependencyState,
}

impl DisplayDependencyEvidence {
    /// Project authenticated Ready evidence from the display route.
    pub fn from_authenticated_route(
        route: AuthenticatedSessionRouteBinding,
    ) -> Result<Self, &'static str> {
        let provider_generation = route
            .provider_generation()
            .ok_or("display-dependency-unauthenticated")?
            .get();
        Self::from_route(route, DisplayDependencyState::Ready, provider_generation)
    }

    /// Resolve one display dependency from an authenticated display route.
    #[allow(dead_code)]
    pub(crate) fn from_route(
        route: AuthenticatedSessionRouteBinding,
        state: DisplayDependencyState,
        generation: u64,
    ) -> Result<Self, &'static str> {
        let Some(provider) = route.provider_ref() else {
            return Err("display-dependency-unauthenticated");
        };
        if route.service().as_str() != DISPLAY_SERVICE_PACKAGE
            || route.evidence_class() != EvidenceClass::UnixPeer
            || route.locality() != Locality::Local
            || provider.to_canonical_string() != DISPLAY_PROVIDER_REF
            || route.subject_ref().resource_type().as_str() != "User"
            || generation == 0
            || route.reconnect_generation().get() == 0
            || route
                .provider_generation()
                .is_none_or(|observed| observed.get() != generation)
        {
            return Err("display-dependency-unauthenticated");
        }
        let Some(host_execution_ref) = route.context().execution_ref() else {
            return Err("display-dependency-unauthenticated");
        };
        let Some(controller_generation) = route.controller_generation() else {
            return Err("display-dependency-unauthenticated");
        };
        if host_execution_ref.resource_type().as_str() != "Host" || controller_generation.get() == 0
        {
            return Err("display-dependency-unauthenticated");
        }
        Ok(Self {
            provider_ref: provider.clone(),
            zone: route.zone().clone(),
            host_execution_ref: host_execution_ref.clone(),
            user_ref: route.subject_ref().clone(),
            provider_generation: generation,
            reconnect_generation: route.reconnect_generation().get(),
            controller_generation: controller_generation.get(),
            state,
        })
    }

    /// Borrow the authenticated display Provider reference.
    pub const fn provider_ref(&self) -> &ResourceRef {
        &self.provider_ref
    }

    /// Borrow the authenticated dependency Zone.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the authenticated Host execution reference.
    pub const fn host_execution_ref(&self) -> &ResourceRef {
        &self.host_execution_ref
    }

    /// Borrow the authenticated display user.
    pub const fn user_ref(&self) -> &ResourceRef {
        &self.user_ref
    }

    /// Return the Core-observed display readiness generation.
    pub const fn generation(&self) -> u64 {
        self.provider_generation
    }

    /// Return the display reconnect generation.
    pub const fn reconnect_generation(&self) -> u64 {
        self.reconnect_generation
    }

    /// Return the Core controller generation.
    pub const fn controller_generation(&self) -> u64 {
        self.controller_generation
    }

    /// Return the dependency readiness state.
    pub const fn state(&self) -> DisplayDependencyState {
        self.state
    }

    /// Whether the dependency is ready for source admission.
    pub const fn is_ready(&self) -> bool {
        matches!(self.state, DisplayDependencyState::Ready)
    }
}

impl core::fmt::Debug for DisplayDependencyEvidence {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("DisplayDependencyEvidence(REDACTED)")
    }
}

/// One configured Guest notification source.
#[derive(Clone, PartialEq, Eq)]
pub struct GuestSourceConfig {
    source_ref: ResourceRef,
    zone: ZoneId,
    categories: BTreeSet<Category>,
}

impl GuestSourceConfig {
    /// Validate one Guest source configuration.
    pub fn new(
        source_ref: ResourceRef,
        zone: ZoneId,
        categories: impl IntoIterator<Item = Category>,
    ) -> Result<Self, &'static str> {
        if source_ref.resource_type().as_str() != "Guest" {
            return Err("notification-source-ref-invalid");
        }
        let categories = categories.into_iter().collect::<BTreeSet<_>>();
        if categories.is_empty() {
            return Err("notification-category-set-empty");
        }
        Ok(Self {
            source_ref,
            zone,
            categories,
        })
    }

    /// Borrow the configured Guest reference.
    pub const fn source_ref(&self) -> &ResourceRef {
        &self.source_ref
    }

    /// Borrow the configured source Zone.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the source category allowlist.
    pub fn categories(&self) -> &BTreeSet<Category> {
        &self.categories
    }
}

impl core::fmt::Debug for GuestSourceConfig {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("GuestSourceConfig")
            .field("source_ref", &"<redacted>")
            .field("zone", &"<redacted>")
            .field("category_count", &self.categories.len())
            .finish()
    }
}

/// Validated notification Provider configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationProviderConfig {
    guest_sources: Vec<GuestSourceConfig>,
    host_execution_ref: Option<ResourceRef>,
    host_user_ref: Option<ResourceRef>,
}

impl NotificationProviderConfig {
    /// Validate bounded, unique Guest source configuration.
    pub fn new(guest_sources: Vec<GuestSourceConfig>) -> Result<Self, &'static str> {
        if guest_sources.len() > MAX_GUEST_SOURCES {
            return Err("notification-source-capacity");
        }
        let mut seen = BTreeSet::new();
        for source in &guest_sources {
            if !seen.insert(source.source_ref.clone()) {
                return Err("notification-source-duplicate");
            }
        }
        Ok(Self {
            guest_sources,
            host_execution_ref: None,
            host_user_ref: None,
        })
    }

    /// Bind the configured processes to the Core-resolved Host and User.
    pub fn with_host_binding(
        mut self,
        host_execution_ref: ResourceRef,
        host_user_ref: ResourceRef,
    ) -> Result<Self, &'static str> {
        if host_execution_ref.resource_type().as_str() != "Host"
            || host_user_ref.resource_type().as_str() != "User"
        {
            return Err("notification-host-binding-invalid");
        }
        self.host_execution_ref = Some(host_execution_ref);
        self.host_user_ref = Some(host_user_ref);
        Ok(self)
    }

    /// Borrow configured Guest sources.
    pub fn guest_sources(&self) -> &[GuestSourceConfig] {
        &self.guest_sources
    }

    fn host_execution_ref(&self) -> Option<&ResourceRef> {
        self.host_execution_ref.as_ref()
    }

    fn host_user_ref(&self) -> Option<&ResourceRef> {
        self.host_user_ref.as_ref()
    }
}

/// Source process lifecycle change emitted by reconciliation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceReconcileResult {
    /// Guest sources whose process endpoint must be started.
    pub start: Vec<ResourceRef>,
    /// Guest sources whose endpoint must be drained and stopped.
    pub stop: Vec<ResourceRef>,
    /// Whether the host sink must be started in this pass.
    pub start_host_sink: bool,
    /// Whether the host sink must be drained and stopped in this pass.
    pub stop_host_sink: bool,
    /// Authenticated endpoints whose source process must be started.
    pub start_endpoints: Vec<SourceEndpoint>,
    /// Authenticated endpoints whose source process must be drained.
    pub stop_endpoints: Vec<SourceEndpoint>,
}

impl SourceReconcileResult {
    fn digest(&self) -> [u8; 32] {
        let mut digest = Sha256::new();
        for source in &self.start {
            digest.update(source.to_canonical_string().as_bytes());
            digest.update([0]);
        }
        digest.update([1]);
        for source in &self.stop {
            digest.update(source.to_canonical_string().as_bytes());
            digest.update([0]);
        }
        digest.update([self.start_host_sink as u8, self.stop_host_sink as u8]);
        for endpoint in &self.start_endpoints {
            digest.update(endpoint.endpoint_digest().as_bytes());
            digest.update([0]);
        }
        digest.update([2]);
        for endpoint in &self.stop_endpoints {
            digest.update(endpoint.endpoint_digest().as_bytes());
            digest.update([0]);
        }
        let bytes = digest.finalize();
        let mut result = [0; 32];
        result.copy_from_slice(&bytes);
        result
    }
}

/// Typed acknowledgement that a complete source effect plan was applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceProcessEffectReceipt {
    plan_digest: [u8; 32],
}

impl SourceProcessEffectReceipt {
    /// Mint a receipt at the trusted process-effect adapter boundary.
    pub fn complete(plan: &SourceReconcileResult) -> Self {
        Self {
            plan_digest: plan.digest(),
        }
    }

    fn matches(&self, plan: &SourceReconcileResult) -> bool {
        self.plan_digest == plan.digest()
    }
}

/// Process/effect boundary used to make reconciliation transactional.
///
/// The controller computes the complete stop/start set first.  Ownership is
/// committed only after this port confirms that every requested process
/// effect was accepted.
pub trait SourceProcessEffectPort {
    /// Apply one complete reconciliation plan.
    fn apply(
        &mut self,
        plan: &SourceReconcileResult,
    ) -> Result<SourceProcessEffectReceipt, &'static str>;
}

/// Authenticated Guest source endpoint evidence.
#[derive(Clone, PartialEq, Eq)]
pub struct SourceEndpoint {
    source_ref: ResourceRef,
    source_generation: u64,
    display_generation: u64,
    endpoint_digest: String,
}

impl SourceEndpoint {
    fn from_authenticated(
        source: &GuestSourceConfig,
        session: &SessionEvidence,
        display: &DisplayDependencyEvidence,
    ) -> Result<Self, &'static str> {
        session
            .admit_source()
            .map_err(|_| "notification-source-unauthenticated")?;
        if session.subject_ref() != source.source_ref() || session.zone() != source.zone() {
            return Err("notification-source-binding-mismatch");
        }
        let mut digest = Sha256::new();
        digest.update(source.source_ref().to_canonical_string().as_bytes());
        digest.update([0]);
        for category in source.categories() {
            digest.update(category.as_str().as_bytes());
            digest.update([0]);
        }
        digest.update(session.generation().to_be_bytes());
        digest.update([0]);
        digest.update(display.generation().to_be_bytes());
        digest.update([0]);
        digest.update(display.controller_generation().to_be_bytes());
        digest.update([0]);
        digest.update(display.reconnect_generation().to_be_bytes());
        digest.update([0]);
        digest.update(
            display
                .host_execution_ref()
                .to_canonical_string()
                .as_bytes(),
        );
        digest.update([0]);
        digest.update(display.user_ref().to_canonical_string().as_bytes());
        let endpoint_digest = format!(
            "sha256:{}",
            digest
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        Ok(Self {
            source_ref: source.source_ref().clone(),
            source_generation: session.generation(),
            display_generation: display.generation(),
            endpoint_digest,
        })
    }

    /// Borrow the exact configured Guest reference.
    pub const fn source_ref(&self) -> &ResourceRef {
        &self.source_ref
    }

    /// Return the authenticated Guest reconnect generation.
    pub const fn source_generation(&self) -> u64 {
        self.source_generation
    }

    /// Return the display generation this endpoint consumes.
    pub const fn display_generation(&self) -> u64 {
        self.display_generation
    }

    /// Borrow the opaque endpoint correlation.
    pub fn endpoint_digest(&self) -> &str {
        &self.endpoint_digest
    }
}

impl core::fmt::Debug for SourceEndpoint {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("SourceEndpoint(REDACTED)")
    }
}

/// A planned notification component process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessPlan {
    /// Stable process template.
    pub template: &'static str,
    /// Process execution domain.
    pub domain: &'static str,
    /// Whether a state Volume is mounted.
    pub mounts_state_volume: bool,
    /// Guest source reference for source processes, if this is a source plan.
    pub source_ref: Option<ResourceRef>,
    /// Authenticated execution reference for the process.
    pub execution_ref: ResourceRef,
    /// Authenticated User identity for user-domain processes.
    pub user_ref: Option<ResourceRef>,
}

/// Notification placement controller.
pub struct NotificationController {
    provider_ref: ResourceRef,
    active_sources: std::collections::BTreeMap<ResourceRef, SourceEndpoint>,
    active_display_generation: Option<u64>,
    host_sink_generation: Option<u64>,
}

impl NotificationController {
    /// Construct a controller for one exact Provider instance.
    pub fn new(provider_ref: impl AsRef<str>) -> Result<Self, &'static str> {
        let provider_ref = ResourceRef::parse(provider_ref.as_ref())
            .map_err(|_| "notification-provider-ref-invalid")?;
        if provider_ref.to_canonical_string() != crate::PROVIDER_REF {
            return Err("notification-provider-ref-invalid");
        }
        Ok(Self {
            provider_ref,
            active_sources: std::collections::BTreeMap::new(),
            active_display_generation: None,
            host_sink_generation: None,
        })
    }

    /// Plan component processes from typed display evidence and configuration.
    pub fn plan(
        &self,
        display: &DisplayDependencyEvidence,
        config: &NotificationProviderConfig,
    ) -> Result<Vec<ProcessPlan>, &'static str> {
        let host_execution_ref = config
            .host_execution_ref()
            .ok_or("notification-host-binding-missing")?;
        let host_user_ref = config
            .host_user_ref()
            .ok_or("notification-host-binding-missing")?;
        if display.host_execution_ref() != host_execution_ref || display.user_ref() != host_user_ref
        {
            return Err("notification-host-binding-mismatch");
        }
        if config
            .guest_sources()
            .iter()
            .any(|source| source.zone() != display.zone())
        {
            return Err("notification-source-zone-mismatch");
        }
        let mut plans = vec![ProcessPlan {
            template: "notification-desktop-controller",
            domain: "system",
            mounts_state_volume: false,
            source_ref: None,
            execution_ref: host_execution_ref.clone(),
            user_ref: None,
        }];
        if display.is_ready() {
            plans.push(ProcessPlan {
                template: "notification-desktop-host-sink",
                domain: "user",
                mounts_state_volume: false,
                source_ref: None,
                execution_ref: host_execution_ref.clone(),
                user_ref: Some(host_user_ref.clone()),
            });
        }
        if display.is_ready() {
            plans.extend(config.guest_sources().iter().map(|source| ProcessPlan {
                template: "notification-desktop-guest-source",
                domain: "guest",
                mounts_state_volume: false,
                source_ref: Some(source.source_ref().clone()),
                execution_ref: source.source_ref().clone(),
                user_ref: None,
            }));
        }
        Ok(plans)
    }

    /// Reconcile configured Guest source endpoints and the host sink.
    #[cfg(test)]
    pub fn reconcile_sources(
        &mut self,
        display: &DisplayDependencyEvidence,
        config: &NotificationProviderConfig,
        source_sessions: &[SessionEvidence],
    ) -> Result<SourceReconcileResult, &'static str> {
        let result = self.plan_reconciliation(display, config, source_sessions)?;
        self.commit_reconciliation(display, result.clone());
        Ok(result)
    }

    /// Reconcile Guest sources through an effect port, committing ownership
    /// only after process effects succeed.
    pub fn reconcile_sources_with_effects<E: SourceProcessEffectPort>(
        &mut self,
        display: &DisplayDependencyEvidence,
        config: &NotificationProviderConfig,
        source_sessions: &[SessionEvidence],
        effects: &mut E,
    ) -> Result<SourceReconcileResult, &'static str> {
        let result = self.plan_reconciliation(display, config, source_sessions)?;
        let receipt = effects.apply(&result)?;
        if !receipt.matches(&result) {
            return Err("notification-process-effect-proof-mismatch");
        }
        self.commit_reconciliation(display, result.clone());
        Ok(result)
    }

    fn plan_reconciliation(
        &self,
        display: &DisplayDependencyEvidence,
        config: &NotificationProviderConfig,
        source_sessions: &[SessionEvidence],
    ) -> Result<SourceReconcileResult, &'static str> {
        self.plan(display, config)?;
        let endpoints = if display.is_ready() {
            config
                .guest_sources()
                .iter()
                .map(|source| {
                    let mut matches = source_sessions
                        .iter()
                        .filter(|session| session.subject_ref() == source.source_ref());
                    let session = matches
                        .next()
                        .ok_or("notification-source-unauthenticated")?;
                    if matches.next().is_some() {
                        return Err("notification-source-ambiguous");
                    }
                    SourceEndpoint::from_authenticated(source, session, display)
                })
                .collect::<Result<Vec<_>, _>>()?
        } else {
            Vec::new()
        };
        let configured = endpoints
            .iter()
            .map(|endpoint| (endpoint.source_ref().clone(), endpoint.clone()))
            .collect::<std::collections::BTreeMap<_, _>>();
        let start = configured
            .iter()
            .filter(|(source, endpoint)| {
                self.active_sources
                    .get(*source)
                    .is_none_or(|active| active != *endpoint)
            })
            .map(|(source, _)| source.clone())
            .collect::<Vec<_>>();
        let stop = self
            .active_sources
            .iter()
            .filter(|(source, active)| {
                configured
                    .get(*source)
                    .is_none_or(|endpoint| endpoint != *active)
            })
            .map(|(source, _)| source.clone())
            .collect::<Vec<_>>();
        let start_endpoints = start
            .iter()
            .filter_map(|source| configured.get(source).cloned())
            .collect::<Vec<_>>();
        let stop_endpoints = stop
            .iter()
            .filter_map(|source| self.active_sources.get(source).cloned())
            .collect::<Vec<_>>();
        let generation_changed = self
            .active_display_generation
            .is_some_and(|generation| generation != display.generation());
        let stop_host_sink =
            self.host_sink_generation.is_some() && (!display.is_ready() || generation_changed);
        let start_host_sink =
            display.is_ready() && (self.host_sink_generation != Some(display.generation()));
        Ok(SourceReconcileResult {
            start,
            stop,
            start_host_sink,
            stop_host_sink,
            start_endpoints,
            stop_endpoints,
        })
    }

    fn commit_reconciliation(
        &mut self,
        display: &DisplayDependencyEvidence,
        result: SourceReconcileResult,
    ) {
        for source in result.stop {
            self.active_sources.remove(&source);
        }
        for endpoint in result.start_endpoints {
            self.active_sources
                .insert(endpoint.source_ref().clone(), endpoint);
        }
        self.active_display_generation = display.is_ready().then_some(display.generation());
        self.host_sink_generation = display.is_ready().then_some(display.generation());
    }

    /// Reconcile from a Core-authenticated display route.
    ///
    /// `None` is the fail-closed dependency state and drains every owned
    /// source/sink endpoint. A route is accepted only when the sealed
    /// ComponentSession authority has bound the display Provider, local Unix
    /// evidence, a User subject, and a non-zero Provider generation.
    #[cfg(test)]
    pub fn reconcile_authenticated_display(
        &mut self,
        display: Option<AuthenticatedSessionRouteBinding>,
        config: &NotificationProviderConfig,
        source_sessions: &[SessionEvidence],
    ) -> Result<SourceReconcileResult, &'static str> {
        let Some(proof) = display else {
            let stop_endpoints = self.active_sources.values().cloned().collect();
            let stop = self.active_sources.keys().cloned().collect();
            let result = SourceReconcileResult {
                start: Vec::new(),
                stop,
                start_host_sink: false,
                stop_host_sink: self.host_sink_generation.is_some(),
                start_endpoints: Vec::new(),
                stop_endpoints,
            };
            self.active_sources.clear();
            self.active_display_generation = None;
            self.host_sink_generation = None;
            return Ok(result);
        };
        let evidence = DisplayDependencyEvidence::from_authenticated_route(proof)?;
        self.reconcile_sources(&evidence, config, source_sessions)
    }

    /// Reconcile display and Guest-source ownership through the effect
    /// boundary, including fail-closed cleanup when the dependency vanishes.
    pub fn reconcile_authenticated_display_with_effects<E: SourceProcessEffectPort>(
        &mut self,
        display: Option<AuthenticatedSessionRouteBinding>,
        config: &NotificationProviderConfig,
        source_sessions: &[SessionEvidence],
        effects: &mut E,
    ) -> Result<SourceReconcileResult, &'static str> {
        let Some(proof) = display else {
            let stop_endpoints = self.active_sources.values().cloned().collect();
            let stop = self.active_sources.keys().cloned().collect();
            let result = SourceReconcileResult {
                start: Vec::new(),
                stop,
                start_host_sink: false,
                stop_host_sink: self.host_sink_generation.is_some(),
                start_endpoints: Vec::new(),
                stop_endpoints,
            };
            let receipt = effects.apply(&result)?;
            if !receipt.matches(&result) {
                return Err("notification-process-effect-proof-mismatch");
            }
            self.active_sources.clear();
            self.active_display_generation = None;
            self.host_sink_generation = None;
            return Ok(result);
        };
        let evidence = DisplayDependencyEvidence::from_authenticated_route(proof)?;
        self.reconcile_sources_with_effects(&evidence, config, source_sessions, effects)
    }

    /// Drain and forget all source endpoints during shutdown or finalization.
    #[cfg(test)]
    pub fn drain_sources(&mut self) -> Vec<ResourceRef> {
        let drained = self.active_sources.keys().cloned().collect();
        self.active_sources.clear();
        self.active_display_generation = None;
        self.host_sink_generation = None;
        drained
    }

    /// Notification state is transient and never has a Provider state Volume.
    pub const fn provider_state_set_empty(&self) -> bool {
        true
    }

    /// Borrow the exact Provider reference.
    pub const fn provider_ref(&self) -> &ResourceRef {
        &self.provider_ref
    }
}

impl core::fmt::Debug for NotificationController {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("NotificationController(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admission::{test_source, test_source_at};

    fn display(state: DisplayDependencyState) -> DisplayDependencyEvidence {
        DisplayDependencyEvidence {
            provider_ref: ResourceRef::parse(DISPLAY_PROVIDER_REF).unwrap(),
            zone: ZoneId::parse("work").unwrap(),
            host_execution_ref: ResourceRef::parse("Host/host-system").unwrap(),
            user_ref: ResourceRef::parse("User/alice").unwrap(),
            provider_generation: 4,
            reconnect_generation: 2,
            controller_generation: 3,
            state,
        }
    }

    fn source(name: &str) -> GuestSourceConfig {
        GuestSourceConfig::new(
            ResourceRef::parse(format!("Guest/{name}").as_str()).unwrap(),
            ZoneId::parse("work").unwrap(),
            [Category::SystemInfo],
        )
        .unwrap()
    }

    fn bound_config(sources: Vec<GuestSourceConfig>) -> NotificationProviderConfig {
        NotificationProviderConfig::new(sources)
            .unwrap()
            .with_host_binding(
                ResourceRef::parse("Host/host-system").unwrap(),
                ResourceRef::parse("User/alice").unwrap(),
            )
            .unwrap()
    }

    #[test]
    fn planning_requires_ready_same_zone_display_evidence() {
        let controller = NotificationController::new(crate::PROVIDER_REF).unwrap();
        let config = bound_config(vec![source("one")]);
        let pending_plans = controller
            .plan(&display(DisplayDependencyState::Pending), &config)
            .unwrap();
        assert_eq!(pending_plans.len(), 1);
        assert!(
            pending_plans
                .iter()
                .all(|plan| plan.template != "notification-desktop-host-sink")
        );
        let wrong_zone = GuestSourceConfig::new(
            ResourceRef::parse("Guest/two").unwrap(),
            ZoneId::parse("personal").unwrap(),
            [Category::SystemInfo],
        )
        .unwrap();
        let wrong_zone_config = bound_config(vec![wrong_zone]);
        assert_eq!(
            controller.plan(&display(DisplayDependencyState::Ready), &wrong_zone_config),
            Err("notification-source-zone-mismatch")
        );
    }

    #[test]
    fn source_reconciliation_starts_stops_and_drains_exact_endpoints() {
        let mut controller = NotificationController::new(crate::PROVIDER_REF).unwrap();
        let first = bound_config(vec![source("one")]);
        let second = bound_config(vec![source("two")]);
        let dependency = display(DisplayDependencyState::Ready);
        let first_result = controller
            .reconcile_sources(&dependency, &first, &[test_source("one")])
            .unwrap();
        assert_eq!(
            first_result.start,
            vec![ResourceRef::parse("Guest/one").unwrap()]
        );
        assert!(first_result.start_host_sink);
        assert!(!first_result.stop_host_sink);
        assert_eq!(
            controller
                .reconcile_sources(&dependency, &second, &[test_source("two")])
                .unwrap()
                .stop,
            vec![ResourceRef::parse("Guest/one").unwrap()]
        );
        let restarted = controller
            .reconcile_sources(
                &display(DisplayDependencyState::Ready),
                &second,
                &[test_source("two")],
            )
            .unwrap();
        assert!(!restarted.start_host_sink);
        assert!(!restarted.stop_host_sink);
        let unavailable = controller
            .reconcile_sources(&display(DisplayDependencyState::Pending), &second, &[])
            .unwrap();
        assert!(unavailable.stop_host_sink);
        let recovered = controller
            .reconcile_sources(
                &display(DisplayDependencyState::Ready),
                &second,
                &[test_source("two")],
            )
            .unwrap();
        assert!(recovered.start_host_sink);
        assert_eq!(
            controller.drain_sources(),
            vec![ResourceRef::parse("Guest/two").unwrap()]
        );
        assert!(controller.drain_sources().is_empty());
    }

    #[test]
    fn source_generation_change_drains_and_restarts_the_exact_endpoint() {
        let mut controller = NotificationController::new(crate::PROVIDER_REF).unwrap();
        let config = bound_config(vec![source("one")]);
        let dependency = display(DisplayDependencyState::Ready);
        controller
            .reconcile_sources(&dependency, &config, &[test_source("one")])
            .unwrap();
        let changed = controller
            .reconcile_sources(&dependency, &config, &[test_source_at("one", 2)])
            .unwrap();
        assert_eq!(
            changed.start,
            vec![ResourceRef::parse("Guest/one").unwrap()]
        );
        assert_eq!(changed.stop, vec![ResourceRef::parse("Guest/one").unwrap()]);
        assert_eq!(changed.start_endpoints[0].source_generation(), 2);
        assert_eq!(changed.stop_endpoints[0].source_generation(), 1);
    }

    #[test]
    fn duplicate_authenticated_source_sessions_are_rejected() {
        let mut controller = NotificationController::new(crate::PROVIDER_REF).unwrap();
        let config = bound_config(vec![source("one")]);
        let dependency = display(DisplayDependencyState::Ready);
        assert_eq!(
            controller.reconcile_sources(
                &dependency,
                &config,
                &[test_source_at("one", 1), test_source_at("one", 2)],
            ),
            Err("notification-source-ambiguous")
        );
    }

    struct FailingEffects;

    impl SourceProcessEffectPort for FailingEffects {
        fn apply(
            &mut self,
            _plan: &SourceReconcileResult,
        ) -> Result<SourceProcessEffectReceipt, &'static str> {
            Err("process-effect-failed")
        }
    }

    #[test]
    fn reconciliation_commits_source_ownership_only_after_effects_succeed() {
        let mut controller = NotificationController::new(crate::PROVIDER_REF).unwrap();
        let first = bound_config(vec![source("one")]);
        let second = bound_config(vec![source("two")]);
        let dependency = display(DisplayDependencyState::Ready);
        controller
            .reconcile_sources(&dependency, &first, &[test_source("one")])
            .unwrap();
        let mut effects = FailingEffects;
        assert_eq!(
            controller.reconcile_sources_with_effects(
                &dependency,
                &second,
                &[test_source("two")],
                &mut effects,
            ),
            Err("process-effect-failed")
        );
        let retry = controller
            .reconcile_sources(&dependency, &second, &[test_source("two")])
            .unwrap();
        assert_eq!(retry.stop, vec![ResourceRef::parse("Guest/one").unwrap()]);
        assert_eq!(retry.start, vec![ResourceRef::parse("Guest/two").unwrap()]);
    }
}
