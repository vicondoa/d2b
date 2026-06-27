//! Production wiring for the gateway composition root (ADR 0032, P0): a
//! [`SystemClock`], a `/dev/urandom`-backed [`UrandomIds`] secret/id source, and
//! [`production_deps`] which assembles a [`GatewayDeps`] from the real
//! [`AcaGatewayWorkload`] + [`RelayDisplayListener`] adapters. This is what the
//! gateway-mode daemon calls to obtain a fully-wired [`GatewayOrchestrator`].

use std::io::Read;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use d2b_gateway::{
    Clock, DisplayListener, DisplaySessionId, GatewayAudit, GatewayDeps, GatewayWorkload, IdSource,
    NoopGatewayAudit, SECRET_LEN, SessionSecret,
};

use crate::NowFn;

/// A system-time [`Clock`] (Unix seconds).
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_unix(&self) -> u64 {
        system_now_unix()
    }
}

/// Current Unix seconds (saturating at 0 before the epoch, which never happens
/// on a sane host clock).
pub fn system_now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A [`NowFn`] backed by the system clock, for the relay verifier/listener.
pub fn system_now_fn() -> NowFn {
    Arc::new(system_now_unix)
}

/// Fill `buf` with cryptographically-strong bytes from `/dev/urandom`. Panics
/// (fail-closed) if the kernel CSPRNG cannot be read in full — the gateway must
/// never mint a session secret from weak/partial entropy.
fn fill_random(buf: &mut [u8]) {
    let mut f = std::fs::File::open("/dev/urandom")
        .expect("gateway: cannot open /dev/urandom for session entropy");
    f.read_exact(buf)
        .expect("gateway: short read from /dev/urandom for session entropy");
}

/// Lowercase-hex encode (no external dep).
fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        s.push(char::from_digit((b & 0x0f) as u32, 16).unwrap());
    }
    s
}

/// A `/dev/urandom`-backed [`IdSource`]: 32-byte session secrets and 128-bit
/// hex session ids.
#[derive(Debug, Default, Clone, Copy)]
pub struct UrandomIds;

impl IdSource for UrandomIds {
    fn new_session_id(&self) -> DisplaySessionId {
        let mut raw = [0u8; 16];
        fill_random(&mut raw);
        DisplaySessionId::new(format!("disp-{}", hex(&raw)))
    }

    fn new_secret(&self) -> SessionSecret {
        let mut raw = [0u8; SECRET_LEN];
        fill_random(&mut raw);
        SessionSecret::from_bytes(raw)
    }
}

/// Assemble production [`GatewayDeps`] from the real workload + display
/// adapters, the system clock, and the kernel CSPRNG id source.
pub fn production_deps(
    workload: Box<dyn GatewayWorkload>,
    listener: Box<dyn DisplayListener>,
) -> GatewayDeps {
    production_deps_with_audit(workload, listener, Box::new(NoopGatewayAudit))
}

/// Assemble production [`GatewayDeps`] with an explicit durable audit sink.
pub fn production_deps_with_audit(
    workload: Box<dyn GatewayWorkload>,
    listener: Box<dyn DisplayListener>,
    audit: Box<dyn GatewayAudit>,
) -> GatewayDeps {
    GatewayDeps {
        workload,
        listener,
        clock: Box::new(SystemClock),
        ids: Box::new(UrandomIds),
        audit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_clock_is_monotone_nonzero() {
        let c = SystemClock;
        assert!(c.now_unix() > 1_700_000_000);
        assert!(system_now_fn()() > 1_700_000_000);
    }

    #[test]
    fn hex_round_trips_known_vectors() {
        assert_eq!(hex(&[0x00, 0x0f, 0xa0, 0xff]), "000fa0ff");
        assert_eq!(hex(&[]), "");
    }

    #[test]
    fn urandom_ids_are_unique_and_secrets_are_full_length() {
        let ids = UrandomIds;
        let a = ids.new_session_id();
        let b = ids.new_session_id();
        assert_ne!(a.as_str(), b.as_str());
        assert!(a.as_str().starts_with("disp-"));
        // 32-byte secret, and two draws differ with overwhelming probability.
        let s1 = ids.new_secret();
        let s2 = ids.new_secret();
        assert_eq!(s1.expose().len(), SECRET_LEN);
        assert_ne!(s1.expose(), s2.expose());
    }
}
