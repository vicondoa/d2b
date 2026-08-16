//! Bounded, closed-label shell-terminal observations.

use std::collections::VecDeque;

use crate::ShellTerminalError;

/// Closed execution-location label set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionKind {
    /// A Host user-domain pool.
    Host,
    /// A Guest user-domain pool.
    Guest,
}

/// Closed diagnostic result set with no caller-controlled text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticKind {
    /// An attach was denied by policy, capacity, or stale generation.
    AttachDenied,
    /// A supervisor was missing or could not be verified.
    SupervisorLost,
}

/// A bounded accumulator that stores only closed diagnostic kinds.
pub struct DiagnosticAccumulator {
    capacity: usize,
    entries: VecDeque<DiagnosticKind>,
}

impl DiagnosticAccumulator {
    /// Construct a bounded diagnostic accumulator.
    pub fn new(capacity: usize, byte_budget: usize) -> Result<Self, ShellTerminalError> {
        if capacity == 0 || byte_budget < capacity {
            return Err(ShellTerminalError::CapacityOutOfRange);
        }
        Ok(Self {
            capacity,
            entries: VecDeque::with_capacity(capacity),
        })
    }

    /// Record one closed diagnostic kind, evicting the oldest entry if needed.
    pub fn record(&mut self, kind: DiagnosticKind) {
        if self.entries.len() == self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(kind);
    }

    /// Return the number of retained diagnostic kinds.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Return whether no diagnostic kinds are retained.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl std::fmt::Debug for DiagnosticAccumulator {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DiagnosticAccumulator")
            .field("len", &self.entries.len())
            .finish()
    }
}

/// Aggregate metrics with only closed execution labels.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ShellMetrics {
    host_attach_denied: u64,
    guest_attach_denied: u64,
}

impl ShellMetrics {
    /// Record an attach result without retaining any session or terminal data.
    pub fn record_attach(&mut self, execution: ExecutionKind, success: bool) {
        if success {
            return;
        }
        match execution {
            ExecutionKind::Host => {
                self.host_attach_denied = self.host_attach_denied.saturating_add(1)
            }
            ExecutionKind::Guest => {
                self.guest_attach_denied = self.guest_attach_denied.saturating_add(1)
            }
        }
    }

    /// Return denied attach count for one closed execution label.
    pub const fn attach_denied(&self, execution: ExecutionKind) -> u64 {
        match execution {
            ExecutionKind::Host => self.host_attach_denied,
            ExecutionKind::Guest => self.guest_attach_denied,
        }
    }
}
