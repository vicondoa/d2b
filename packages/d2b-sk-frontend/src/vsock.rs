//! AF_VSOCK connection management with exponential backoff.
//!
//! The guest sk-frontend must tolerate the host broker being unavailable at
//! startup or after a d2bd restart. This module wraps `tokio-vsock`'s
//! `VsockStream::connect` with a bounded exponential-backoff retry loop so
//! the frontend recovers automatically without spinning.

use std::time::Duration;

use tokio_vsock::{VsockAddr, VsockStream};

/// AF_VSOCK CID for the hypervisor host (VMADDR_CID_HOST = 2).
pub const VSOCK_HOST_CID: u32 = 2;

/// Default VSOCK port for the d2b security-key CTAPHID relay.
pub const SK_VSOCK_PORT: u32 = 14320;

/// Parameters controlling the backoff.
#[derive(Debug, Clone, Copy)]
pub struct BackoffParams {
    /// Initial reconnect wait before the first retry.
    pub initial: Duration,
    /// Maximum wait between reconnect attempts.
    pub max: Duration,
    /// Multiply the current wait by this factor on each failure (2.0 = double).
    pub factor: f64,
}

impl Default for BackoffParams {
    fn default() -> Self {
        Self {
            initial: Duration::from_secs(1),
            max: Duration::from_secs(60),
            factor: 2.0,
        }
    }
}

/// Attempt to connect to the host broker over AF_VSOCK with exponential
/// backoff.
///
/// Sleeps `initial` before the first attempt (giving d2bd a moment to create
/// the socket endpoint). Returns only when a connection is established.
pub async fn connect_with_backoff(port: u32, params: BackoffParams, vm_id: &str) -> VsockStream {
    let mut wait = params.initial;
    loop {
        tokio::time::sleep(wait).await;
        eprintln!("[d2b-sk-frontend/{vm_id}] connecting to vsock:{VSOCK_HOST_CID}:{port}");
        match VsockStream::connect(VsockAddr::new(VSOCK_HOST_CID, port)).await {
            Ok(stream) => {
                eprintln!("[d2b-sk-frontend/{vm_id}] vsock connected");
                return stream;
            }
            Err(e) => {
                eprintln!(
                    "[d2b-sk-frontend/{vm_id}] vsock connect error: {e}; retry in {}s",
                    wait.as_secs()
                );
                let next_wait_secs =
                    (wait.as_secs_f64() * params.factor).min(params.max.as_secs_f64());
                wait = Duration::from_secs_f64(next_wait_secs);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_params_default_sensible() {
        let p = BackoffParams::default();
        assert!(p.initial >= Duration::from_millis(100));
        assert!(p.max >= p.initial);
        assert!(p.factor >= 1.0);
    }

    #[test]
    fn backoff_clamped_to_max() {
        let p = BackoffParams {
            initial: Duration::from_secs(1),
            max: Duration::from_secs(10),
            factor: 4.0,
        };
        let mut wait = p.initial;
        for _ in 0..10 {
            let next = (wait.as_secs_f64() * p.factor).min(p.max.as_secs_f64());
            wait = Duration::from_secs_f64(next);
        }
        assert!(wait <= p.max + Duration::from_millis(1));
    }
}
