//! Authenticated vsock transport service lifecycle.

use crate::{
    ReadySession,
    bridge::{
        BridgeControl, BridgeExit, BridgeStats, NamedStreamError, NamedStreamId, NamedStreamPort,
        TransportHandle, run_bridge,
    },
    effect_port::{OpaqueBindingId, OpaqueEndpointId, TransportRole, VsockEffectPort},
    errors::ServiceError,
    framing::VsockTransportDescriptor,
    limits::{CLOSE_GRACE_MS, MAX_ACTIVE_TRANSPORTS, MAX_OPEN_DEADLINE_MS, MIN_OPEN_DEADLINE_MS},
};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    sync::{Mutex, OwnedSemaphorePermit, Semaphore},
    time::timeout,
};

const PROVIDER_REF: &str = "Provider/transport-vsock";

/// Request to open one ZoneLink byte transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenTransportRequest {
    /// Core-issued endpoint resolution identity.
    pub endpoint_id: OpaqueEndpointId,
    /// Core-issued binding identity.
    pub binding_id: OpaqueBindingId,
    /// Initiator or responder role.
    pub role: TransportRole,
    /// Bounded connect/accept deadline.
    pub deadline_ms: u32,
}

impl OpenTransportRequest {
    /// Construct an open request.
    pub const fn new(
        endpoint_id: OpaqueEndpointId,
        binding_id: OpaqueBindingId,
        role: TransportRole,
        deadline_ms: u32,
    ) -> Self {
        Self {
            endpoint_id,
            binding_id,
            role,
            deadline_ms,
        }
    }

    /// Parse a wire-shaped request at the service boundary.
    pub fn from_raw(
        endpoint_id: impl Into<String>,
        binding_id: impl Into<String>,
        role: TransportRole,
        deadline_ms: u32,
    ) -> Result<Self, ServiceError> {
        let endpoint_id =
            OpaqueEndpointId::parse(endpoint_id).map_err(|_| ServiceError::InvalidEndpointId)?;
        let binding_id =
            OpaqueBindingId::parse(binding_id).map_err(|_| ServiceError::InvalidBindingId)?;
        Ok(Self::new(endpoint_id, binding_id, role, deadline_ms))
    }

    fn validate(&self) -> Result<(), ServiceError> {
        if !(MIN_OPEN_DEADLINE_MS..=MAX_OPEN_DEADLINE_MS).contains(&self.deadline_ms) {
            return Err(ServiceError::InvalidDeadline);
        }
        Ok(())
    }
}

/// Result of opening one transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenTransportResponse {
    /// Opaque handle used by CloseTransport and ObserveTransport.
    pub transport_handle: TransportHandle,
    /// ComponentSession named stream carrying the bridge bytes.
    pub stream_id: NamedStreamId,
    /// Native-vsock transport descriptor.
    pub descriptor: VsockTransportDescriptor,
}

/// Request to close one transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloseTransportRequest {
    /// Handle returned by OpenTransport.
    pub transport_handle: TransportHandle,
}

/// Request to observe one transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObserveTransportRequest {
    /// Handle returned by OpenTransport.
    pub transport_handle: TransportHandle,
    /// Include bounded byte counters in the response.
    pub include_bytes: bool,
}

/// Provider lifecycle phase for one transport handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportPhase {
    /// The effect and named stream were acquired.
    Acquired,
    /// The owner requested closure.
    Closing,
    /// The bridge and both endpoints are closed.
    Released,
    /// Closure could not be confirmed within the bound.
    Degraded,
}

/// Bounded observation returned by ObserveTransport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransportObservation {
    /// Current lifecycle phase.
    pub phase: TransportPhase,
    /// Fixed provider descriptor.
    pub descriptor: VsockTransportDescriptor,
    /// Bytes received from the vsock side when requested.
    pub bytes_rx: Option<u64>,
    /// Bytes sent to the vsock side when requested.
    pub bytes_tx: Option<u64>,
    /// Last bridge termination reason.
    pub last_exit: Option<BridgeExit>,
}

/// Provider service readiness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServicePhase {
    /// The service has no active transport.
    Ready,
    /// At least one transport is active.
    Serving,
    /// One transport failed bounded closure.
    Degraded,
}

/// The single transport-vsock service component for one Zone.
pub struct VsockTransportService<P, N>
where
    P: VsockEffectPort,
    N: NamedStreamPort,
{
    effect: Arc<P>,
    streams: Arc<N>,
    active: Arc<Mutex<HashMap<TransportHandle, TransportEntry>>>,
    completed: Arc<Mutex<HashMap<TransportHandle, TransportObservation>>>,
    slots: Arc<Semaphore>,
    next_handle: AtomicU64,
}

struct TransportEntry {
    control: BridgeControl,
    phase: Arc<Mutex<TransportPhase>>,
    exit: Arc<Mutex<Option<BridgeExit>>>,
    stats: Arc<BridgeStats>,
    _permit: OwnedSemaphorePermit,
}

impl<P, N> VsockTransportService<P, N>
where
    P: VsockEffectPort,
    N: NamedStreamPort,
{
    /// Construct a service over the child-core effect and stream ports.
    pub fn new(effect: P, streams: N) -> Self {
        Self {
            effect: Arc::new(effect),
            streams: Arc::new(streams),
            active: Arc::new(Mutex::new(HashMap::new())),
            completed: Arc::new(Mutex::new(HashMap::new())),
            slots: Arc::new(Semaphore::new(MAX_ACTIVE_TRANSPORTS)),
            next_handle: AtomicU64::new(1),
        }
    }

    /// Return the stable Provider reference.
    pub const fn provider_ref(&self) -> &'static str {
        PROVIDER_REF
    }

    /// Return the current service phase.
    pub async fn phase(&self) -> ServicePhase {
        let active = self.active.lock().await;
        if active.values().any(futures_phase_is_degraded) {
            ServicePhase::Degraded
        } else if active.is_empty() {
            ServicePhase::Ready
        } else {
            ServicePhase::Serving
        }
    }

    /// Open one authenticated transport and its named stream bridge.
    pub async fn open_transport(
        &self,
        session: &ReadySession,
        request: OpenTransportRequest,
    ) -> Result<OpenTransportResponse, ServiceError> {
        if session.state() != crate::SessionState::Ready {
            return Err(ServiceError::SessionNotReady);
        }
        request.validate()?;
        let permit = Arc::clone(&self.slots)
            .try_acquire_owned()
            .map_err(|_| ServiceError::ProviderOverloaded)?;
        let deadline = Instant::now() + Duration::from_millis(u64::from(request.deadline_ms));
        let effect_stream = self
            .effect
            .open(
                &request.endpoint_id,
                &request.binding_id,
                request.role,
                deadline,
            )
            .await
            .map_err(ServiceError::Effect)?;
        let (stream_id, named_stream) = match self.streams.open_named_stream().await {
            Ok(value) => value,
            Err(_) => {
                let _ = self.effect.close(effect_stream).await;
                return Err(ServiceError::StreamUnavailable);
            }
        };
        let handle = TransportHandle::from_core(self.next_handle.fetch_add(1, Ordering::Relaxed));
        let (control, stop) = BridgeControl::new();
        let completion = control.completion();
        let phase = Arc::new(Mutex::new(TransportPhase::Acquired));
        let exit = Arc::new(Mutex::new(None));
        let stats = Arc::new(BridgeStats::default());
        let task_effect = Arc::clone(&self.effect);
        let task_streams = Arc::clone(&self.streams);
        let task_phase = Arc::clone(&phase);
        let task_exit = Arc::clone(&exit);
        let task_stats = Arc::clone(&stats);
        tokio::spawn(async move {
            let (effect_stream, _named_stream, reason) =
                run_bridge(effect_stream, named_stream, stop, task_stats).await;
            let effect_result = task_effect.close(effect_stream).await;
            let stream_result = task_streams.close_named_stream(stream_id).await;
            *task_exit.lock().await = Some(reason);
            *task_phase.lock().await = if effect_result.is_ok() && stream_result.is_ok() {
                TransportPhase::Released
            } else {
                TransportPhase::Degraded
            };
            completion.notify_waiters();
        });
        self.active.lock().await.insert(
            handle,
            TransportEntry {
                control,
                phase,
                exit,
                stats,
                _permit: permit,
            },
        );
        Ok(OpenTransportResponse {
            transport_handle: handle,
            stream_id,
            descriptor: VsockTransportDescriptor::default(),
        })
    }

    /// Close one transport. The bridge closes before the effect is released.
    pub async fn close_transport(
        &self,
        request: CloseTransportRequest,
    ) -> Result<(), ServiceError> {
        let entry = {
            let active = self.active.lock().await;
            active
                .get(&request.transport_handle)
                .map(|entry| (entry.control.completion(), Arc::clone(&entry.phase)))
        };
        if self
            .completed
            .lock()
            .await
            .contains_key(&request.transport_handle)
        {
            return Ok(());
        }
        let Some((completion, phase)) = entry else {
            return Err(ServiceError::UnknownTransportHandle);
        };
        if *phase.lock().await == TransportPhase::Released {
            if let Some(entry) = self.active.lock().await.remove(&request.transport_handle) {
                self.completed.lock().await.insert(
                    request.transport_handle,
                    TransportObservation {
                        phase: TransportPhase::Released,
                        descriptor: VsockTransportDescriptor::default(),
                        bytes_rx: Some(entry.stats.bytes_from_vsock()),
                        bytes_tx: Some(entry.stats.bytes_to_vsock()),
                        last_exit: *entry.exit.lock().await,
                    },
                );
            }
            return Ok(());
        }
        if let Some(entry) = self.active.lock().await.get(&request.transport_handle) {
            entry.control.stop();
            *entry.phase.lock().await = TransportPhase::Closing;
        }
        if timeout(Duration::from_millis(CLOSE_GRACE_MS), completion.notified())
            .await
            .is_err()
        {
            if let Some(entry) = self.active.lock().await.get(&request.transport_handle) {
                *entry.phase.lock().await = TransportPhase::Degraded;
            }
            return Err(ServiceError::CloseUnconfirmed);
        }
        if let Some(entry) = self.active.lock().await.remove(&request.transport_handle) {
            let observation = TransportObservation {
                phase: TransportPhase::Released,
                descriptor: VsockTransportDescriptor::default(),
                bytes_rx: Some(entry.stats.bytes_from_vsock()),
                bytes_tx: Some(entry.stats.bytes_to_vsock()),
                last_exit: *entry.exit.lock().await,
            };
            let mut completed = self.completed.lock().await;
            if completed.len() >= MAX_ACTIVE_TRANSPORTS
                && let Some(oldest) = completed.keys().next().copied()
            {
                completed.remove(&oldest);
            }
            completed.insert(request.transport_handle, observation);
        }
        Ok(())
    }

    /// Observe one transport without exposing identity, path, CID, or port.
    pub async fn observe_transport(
        &self,
        request: ObserveTransportRequest,
    ) -> Result<TransportObservation, ServiceError> {
        let active = self.active.lock().await;
        let Some(entry) = active.get(&request.transport_handle) else {
            return self
                .completed
                .lock()
                .await
                .get(&request.transport_handle)
                .copied()
                .ok_or(ServiceError::UnknownTransportHandle);
        };
        let phase = *entry.phase.lock().await;
        let stats = if request.include_bytes {
            Some((entry.stats.bytes_from_vsock(), entry.stats.bytes_to_vsock()))
        } else {
            None
        };
        Ok(TransportObservation {
            phase,
            descriptor: VsockTransportDescriptor::default(),
            bytes_rx: stats.map(|(rx, _)| rx),
            bytes_tx: stats.map(|(_, tx)| tx),
            last_exit: *entry.exit.lock().await,
        })
    }

    /// Finalize all handles owned by this service.
    pub async fn finalize(&self) -> Result<(), ServiceError> {
        let handles = self.active.lock().await.keys().copied().collect::<Vec<_>>();
        for handle in handles {
            self.close_transport(CloseTransportRequest {
                transport_handle: handle,
            })
            .await?;
        }
        Ok(())
    }
}

fn futures_phase_is_degraded(entry: &TransportEntry) -> bool {
    entry
        .phase
        .try_lock()
        .is_ok_and(|phase| *phase == TransportPhase::Degraded)
}

impl From<NamedStreamError> for ServiceError {
    fn from(_: NamedStreamError) -> Self {
        ServiceError::StreamUnavailable
    }
}
