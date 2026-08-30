//! Pure Cloud Hypervisor Guest status projection.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Bounded public Guest lifecycle phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum GuestStatusPhase {
    /// Required dependencies or readiness evidence are incomplete.
    Pending,
    /// The current-generation Guest is fully ready.
    Ready,
    /// A previously usable Guest lost required health.
    Degraded,
    /// Deletion is in progress.
    Draining,
}

impl GuestStatusPhase {
    /// Return the stable lowercase phase name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Ready => "ready",
            Self::Degraded => "degraded",
            Self::Draining => "draining",
        }
    }
}

/// Generation observations that must describe one current Guest incarnation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GuestGenerationSet {
    /// Provider generation observed by the controller.
    pub provider: u64,
    /// Setup-descriptor generation observed by the controller.
    pub descriptor: u64,
    /// Guest-controller generation observed by the controller.
    pub controller: u64,
    /// Required-child generation observed by the controller.
    pub child: u64,
    /// Authenticated Guest-session generation observed by the controller.
    pub session: u64,
}

impl GuestGenerationSet {
    /// Construct observations for one exact, non-zero generation.
    pub const fn all(generation: u64) -> Self {
        Self {
            provider: generation,
            descriptor: generation,
            controller: generation,
            child: generation,
            session: generation,
        }
    }

    /// Return whether every required generation is present and exactly equal.
    pub const fn is_exact(self) -> bool {
        self.provider != 0
            && self.descriptor == self.provider
            && self.controller == self.provider
            && self.child == self.provider
            && self.session == self.provider
    }
}

/// Bounded observations consumed by the pure Guest status reducer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GuestStatusObservation {
    /// Current-generation observations for the Guest graph.
    pub generations: GuestGenerationSet,
    /// Whether all required external dependencies are ready.
    pub dependencies_ready: bool,
    /// Whether the VMM Process is ready.
    pub process_ready: bool,
    /// Whether all required Endpoints are ready.
    pub endpoint_ready: bool,
    /// Whether the authenticated Guest session is established.
    pub session_ready: bool,
    /// Whether descriptor-approved Guest-local seed Resources are ready.
    pub seed_ready: bool,
    /// Whether authenticated Guest-session health is good.
    pub session_healthy: bool,
    /// Whether required child Resource health is good.
    pub required_children_healthy: bool,
    /// Whether deletion has been requested.
    pub deletion_requested: bool,
    /// Whether an authenticated Guest session remains open.
    pub session_active: bool,
    /// Whether any owned descendant remains.
    pub descendants_present: bool,
    /// Whether the VMM Process is observed stopped or absent.
    pub process_stopped: bool,
}

impl GuestStatusObservation {
    /// Construct a fully ready running observation for one generation.
    pub const fn ready(generation: u64) -> Self {
        Self {
            generations: GuestGenerationSet::all(generation),
            dependencies_ready: true,
            process_ready: true,
            endpoint_ready: true,
            session_ready: true,
            seed_ready: true,
            session_healthy: true,
            required_children_healthy: true,
            deletion_requested: false,
            session_active: true,
            descendants_present: true,
            process_stopped: false,
        }
    }
}

/// Redacted public Guest runtime status.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuestRuntimeStatus {
    /// Bounded public lifecycle phase.
    pub phase: GuestStatusPhase,
    /// Whether the current-generation VMM Process is ready.
    pub runtime_ready: bool,
    /// Whether current-generation Endpoint, session, and seed evidence is ready.
    pub bootstrap_ready: bool,
    /// Number of active VMM processes observed.
    pub active_process_count: u16,
}

impl fmt::Debug for GuestRuntimeStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GuestRuntimeStatus")
            .field("phase", &self.phase)
            .field("runtime_ready", &self.runtime_ready)
            .field("bootstrap_ready", &self.bootstrap_ready)
            .field("active_process_count", &self.active_process_count)
            .finish()
    }
}

/// Reduce bounded observations into the public Guest status.
///
/// Deletion takes precedence. Otherwise, missing or stale current-generation
/// evidence is `Pending`; only a complete current-generation graph can become
/// `Ready`. Health loss is projected as `Degraded` after readiness is complete.
pub const fn reduce_status(observation: &GuestStatusObservation) -> GuestRuntimeStatus {
    let generations_current = observation.generations.is_exact();
    let runtime_ready = generations_current && observation.process_ready;
    let bootstrap_ready = generations_current
        && observation.endpoint_ready
        && observation.session_ready
        && observation.seed_ready;
    let ready = generations_current
        && observation.dependencies_ready
        && observation.process_ready
        && observation.endpoint_ready
        && observation.session_ready
        && observation.seed_ready;
    let phase = if observation.deletion_requested {
        GuestStatusPhase::Draining
    } else if !ready {
        GuestStatusPhase::Pending
    } else if !observation.session_healthy || !observation.required_children_healthy {
        GuestStatusPhase::Degraded
    } else {
        GuestStatusPhase::Ready
    };

    GuestRuntimeStatus {
        phase,
        runtime_ready,
        bootstrap_ready,
        active_process_count: if observation.process_ready && !observation.process_stopped {
            1
        } else {
            0
        },
    }
}

/// Return whether the Guest finalizer may be cleared.
///
/// This is eligibility only; it performs no finalizer or lifecycle mutation.
pub const fn finalization_eligible(observation: &GuestStatusObservation) -> bool {
    observation.deletion_requested
        && observation.generations.is_exact()
        && !observation.session_active
        && !observation.descendants_present
        && observation.process_stopped
}
