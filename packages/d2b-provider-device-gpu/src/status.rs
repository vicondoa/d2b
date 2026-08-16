//! Bounded, redacted Device status projection.

use core::fmt;

/// Public GPU status phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuStatusPhase {
    /// The Device is waiting for a first observation.
    Pending,
    /// The Device and all requested workers are healthy.
    Ready,
    /// The Device remains usable but an observation or worker is impaired.
    Degraded,
    /// The current generation cannot complete.
    Failed,
    /// The controller cannot prove current state.
    Unknown,
    /// Deletion is draining owned workers.
    Finalizing,
    /// The Device was finalized.
    Finalized,
    /// Restart identity was ambiguous and is quarantined.
    Quarantined,
}

/// GPU-specific status condition kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuConditionType {
    /// Physical DRM presence.
    DevicePresent,
    /// Host-global claim state.
    DeviceClaimed,
    /// GPU worker readiness.
    GpuWorkerReady,
    /// Video worker readiness.
    VideoWorkerReady,
    /// Restart or inventory identity is ambiguous.
    IdentityTrusted,
}

/// Closed condition states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuConditionState {
    /// The condition is proven true.
    True,
    /// The condition is proven false.
    False,
    /// The controller has not proved either state.
    Unknown,
}

/// One bounded status condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuCondition {
    /// Condition kind.
    pub kind: GpuConditionType,
    /// Condition state.
    pub state: GpuConditionState,
}

/// Redacted Device status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuStatus {
    phase: GpuStatusPhase,
    video_enabled: bool,
    present: Option<bool>,
    render_node_available: Option<bool>,
    gpu_worker_ready: GpuConditionState,
    video_worker_ready: GpuConditionState,
    conditions: Vec<GpuCondition>,
    diagnostic: Option<String>,
}

impl GpuStatus {
    /// Construct the initial pending status.
    pub fn new(video_sidecar: bool) -> Self {
        let video_state = if video_sidecar {
            GpuConditionState::Unknown
        } else {
            GpuConditionState::False
        };
        Self {
            phase: GpuStatusPhase::Pending,
            video_enabled: video_sidecar,
            present: None,
            render_node_available: None,
            gpu_worker_ready: GpuConditionState::Unknown,
            video_worker_ready: video_state,
            conditions: Vec::new(),
            diagnostic: None,
        }
    }

    /// Return the current phase.
    pub const fn phase(&self) -> GpuStatusPhase {
        self.phase
    }

    /// Return the physical presence observation.
    pub const fn present(&self) -> Option<bool> {
        self.present
    }

    /// Return the render-node observation.
    pub const fn render_node_available(&self) -> Option<bool> {
        self.render_node_available
    }

    /// Return the GPU worker condition.
    pub const fn gpu_worker_ready(&self) -> GpuConditionState {
        self.gpu_worker_ready
    }

    /// Return the video worker condition.
    pub const fn video_worker_ready(&self) -> GpuConditionState {
        self.video_worker_ready
    }

    /// Borrow bounded conditions.
    pub fn conditions(&self) -> &[GpuCondition] {
        &self.conditions
    }

    /// Borrow the bounded diagnostic, if present.
    pub fn diagnostic(&self) -> Option<&str> {
        self.diagnostic.as_deref()
    }

    /// Apply a physical probe observation.
    pub fn observe_device(&mut self, present: bool, render_node_available: bool) {
        self.present = Some(present);
        self.render_node_available = Some(render_node_available);
        self.set_condition(
            GpuConditionType::DevicePresent,
            if present {
                GpuConditionState::True
            } else {
                GpuConditionState::False
            },
        );
        if !present {
            self.phase = GpuStatusPhase::Degraded;
        }
    }

    /// Set the Host-global claim condition.
    pub fn set_claimed(&mut self, claimed: bool) {
        self.set_condition(
            GpuConditionType::DeviceClaimed,
            if claimed {
                GpuConditionState::True
            } else {
                GpuConditionState::False
            },
        );
    }

    /// Set GPU worker readiness.
    pub fn set_gpu_worker(&mut self, state: GpuConditionState) {
        self.gpu_worker_ready = state;
        self.set_condition(GpuConditionType::GpuWorkerReady, state);
        self.refresh_phase();
    }

    /// Set video worker readiness.
    pub fn set_video_worker(&mut self, state: GpuConditionState) {
        self.video_worker_ready = state;
        self.set_condition(GpuConditionType::VideoWorkerReady, state);
        self.refresh_phase();
    }

    /// Mark restart identity as quarantined.
    pub fn quarantine(&mut self) {
        self.phase = GpuStatusPhase::Quarantined;
        self.set_condition(GpuConditionType::IdentityTrusted, GpuConditionState::False);
    }

    /// Set a bounded, path-free diagnostic.
    pub fn set_diagnostic(&mut self, value: impl Into<String>) -> Result<(), GpuStatusError> {
        let value = value.into();
        if value.len() > 4 * 1024
            || value.contains('\0')
            || value.contains("/dev/")
            || value.contains("/run/")
            || value.contains("socket")
        {
            return Err(GpuStatusError::DiagnosticRejected);
        }
        self.diagnostic = Some(value);
        Ok(())
    }

    fn set_condition(&mut self, kind: GpuConditionType, state: GpuConditionState) {
        if let Some(condition) = self
            .conditions
            .iter_mut()
            .find(|condition| condition.kind == kind)
        {
            condition.state = state;
        } else {
            self.conditions.push(GpuCondition { kind, state });
        }
    }

    fn refresh_phase(&mut self) {
        if self.present == Some(false) {
            self.phase = GpuStatusPhase::Degraded;
        } else if self.gpu_worker_ready == GpuConditionState::True
            && (!self.video_enabled || self.video_worker_ready == GpuConditionState::True)
        {
            self.phase = GpuStatusPhase::Ready;
        } else if self.gpu_worker_ready == GpuConditionState::False
            || self.video_worker_ready == GpuConditionState::False
        {
            self.phase = GpuStatusPhase::Degraded;
        } else {
            self.phase = GpuStatusPhase::Pending;
        }
    }
}

/// Status projection failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuStatusError {
    /// Diagnostic text was not bounded or contained a forbidden host detail.
    DiagnosticRejected,
}

impl fmt::Display for GpuStatusError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("gpu-status-diagnostic-rejected")
    }
}

impl std::error::Error for GpuStatusError {}
