//! Process placement controller for desktop notification components.

use d2b_contracts::v3::{EvidenceClass, Locality, ResourceRef, ZoneId};
use d2b_provider_display_wayland::SERVICE_PACKAGE as DISPLAY_SERVICE_PACKAGE;
use d2b_session::AuthenticatedSessionRouteBinding;
use std::collections::BTreeSet;

use crate::Category;

const DISPLAY_PROVIDER_REF: &str = d2b_provider_display_wayland::PROVIDER_REF;
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
    user_ref: ResourceRef,
    generation: u64,
    state: DisplayDependencyState,
}

impl DisplayDependencyEvidence {
    /// Resolve one display dependency from an authenticated display route.
    pub fn from_route(
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
            || route
                .provider_generation()
                .is_none_or(|observed| observed.get() != generation)
        {
            return Err("display-dependency-unauthenticated");
        }
        Ok(Self {
            provider_ref: provider.clone(),
            zone: route.zone().clone(),
            user_ref: route.subject_ref().clone(),
            generation,
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

    /// Borrow the authenticated display user.
    pub const fn user_ref(&self) -> &ResourceRef {
        &self.user_ref
    }

    /// Return the Core-observed display readiness generation.
    pub const fn generation(&self) -> u64 {
        self.generation
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
        Ok(Self { guest_sources })
    }

    /// Borrow configured Guest sources.
    pub fn guest_sources(&self) -> &[GuestSourceConfig] {
        &self.guest_sources
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
}

/// Notification placement controller.
pub struct NotificationController {
    provider_ref: ResourceRef,
    active_sources: BTreeSet<ResourceRef>,
    active_display_generation: Option<u64>,
    host_sink_active: bool,
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
            active_sources: BTreeSet::new(),
            active_display_generation: None,
            host_sink_active: false,
        })
    }

    /// Plan component processes from typed display evidence and configuration.
    pub fn plan(
        &self,
        display: &DisplayDependencyEvidence,
        config: &NotificationProviderConfig,
    ) -> Result<Vec<ProcessPlan>, &'static str> {
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
        }];
        if display.is_ready() {
            plans.push(ProcessPlan {
                template: "notification-desktop-host-sink",
                domain: "user",
                mounts_state_volume: false,
                source_ref: None,
            });
        }
        plans.extend(config.guest_sources().iter().map(|source| ProcessPlan {
            template: "notification-desktop-guest-source",
            domain: "guest",
            mounts_state_volume: false,
            source_ref: Some(source.source_ref().clone()),
        }));
        Ok(plans)
    }

    /// Reconcile configured Guest source endpoints and the host sink.
    pub fn reconcile_sources(
        &mut self,
        display: &DisplayDependencyEvidence,
        config: &NotificationProviderConfig,
    ) -> Result<SourceReconcileResult, &'static str> {
        self.plan(display, config)?;
        let configured = config
            .guest_sources()
            .iter()
            .map(|source| source.source_ref().clone())
            .collect::<BTreeSet<_>>();
        let start = configured
            .difference(&self.active_sources)
            .cloned()
            .collect::<Vec<_>>();
        let stop = self
            .active_sources
            .difference(&configured)
            .cloned()
            .collect::<Vec<_>>();
        let generation_changed = self
            .active_display_generation
            .is_some_and(|generation| generation != display.generation());
        let stop_host_sink = self.host_sink_active
            && (!display.is_ready() || generation_changed);
        let start_host_sink = display.is_ready() && (!self.host_sink_active || generation_changed);
        self.active_sources = configured;
        self.host_sink_active = display.is_ready();
        self.active_display_generation = display.is_ready().then_some(display.generation());
        Ok(SourceReconcileResult {
            start,
            stop,
            start_host_sink,
            stop_host_sink,
        })
    }

    /// Drain and forget all source endpoints during shutdown or finalization.
    pub fn drain_sources(&mut self) -> Vec<ResourceRef> {
        let drained = self.active_sources.iter().cloned().collect();
        self.active_sources.clear();
        self.active_display_generation = None;
        self.host_sink_active = false;
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

    fn display(state: DisplayDependencyState) -> DisplayDependencyEvidence {
        DisplayDependencyEvidence {
            provider_ref: ResourceRef::parse(DISPLAY_PROVIDER_REF).unwrap(),
            zone: ZoneId::parse("work").unwrap(),
            user_ref: ResourceRef::parse("User/alice").unwrap(),
            generation: 4,
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

    #[test]
    fn planning_requires_ready_same_zone_display_evidence() {
        let controller = NotificationController::new(crate::PROVIDER_REF).unwrap();
        let config = NotificationProviderConfig::new(vec![source("one")]).unwrap();
        let pending_plans = controller
            .plan(&display(DisplayDependencyState::Pending), &config)
            .unwrap();
        assert_eq!(pending_plans.len(), 2);
        assert!(pending_plans
            .iter()
            .all(|plan| plan.template != "notification-desktop-host-sink"));
        let wrong_zone = GuestSourceConfig::new(
            ResourceRef::parse("Guest/two").unwrap(),
            ZoneId::parse("personal").unwrap(),
            [Category::SystemInfo],
        )
        .unwrap();
        let wrong_zone_config = NotificationProviderConfig::new(vec![wrong_zone]).unwrap();
        assert_eq!(
            controller.plan(&display(DisplayDependencyState::Ready), &wrong_zone_config),
            Err("notification-source-zone-mismatch")
        );
    }

    #[test]
    fn source_reconciliation_starts_stops_and_drains_exact_endpoints() {
        let mut controller = NotificationController::new(crate::PROVIDER_REF).unwrap();
        let first = NotificationProviderConfig::new(vec![source("one")]).unwrap();
        let second = NotificationProviderConfig::new(vec![source("two")]).unwrap();
        let dependency = display(DisplayDependencyState::Ready);
        let first_result = controller.reconcile_sources(&dependency, &first).unwrap();
        assert_eq!(first_result.start, vec![ResourceRef::parse("Guest/one").unwrap()]);
        assert!(first_result.start_host_sink);
        assert!(!first_result.stop_host_sink);
        assert_eq!(
            controller.reconcile_sources(&dependency, &second).unwrap().stop,
            vec![ResourceRef::parse("Guest/one").unwrap()]
        );
        let restarted = controller
            .reconcile_sources(
                &display(DisplayDependencyState::Ready),
                &second,
            )
            .unwrap();
        assert!(!restarted.start_host_sink);
        assert!(!restarted.stop_host_sink);
        let unavailable = controller
            .reconcile_sources(&display(DisplayDependencyState::Pending), &second)
            .unwrap();
        assert!(unavailable.stop_host_sink);
        let recovered = controller
            .reconcile_sources(&display(DisplayDependencyState::Ready), &second)
            .unwrap();
        assert!(recovered.start_host_sink);
        assert_eq!(
            controller.drain_sources(),
            vec![ResourceRef::parse("Guest/two").unwrap()]
        );
        assert!(controller.drain_sources().is_empty());
    }
}
