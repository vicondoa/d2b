//! `nixling-gateway-runtime` — the composition root that ties the
//! `nixling-gateway` per-session handshake to the `nixling-provider-relay`
//! prologue gate (ADR 0032, P0). It provides the **matched pair** that makes
//! the session credential gate the display byte stream over Azure Relay:
//!
//! - [`agent_prologue`] — built by the in-sandbox agent from the secret + the
//!   binding it received over the MI-authenticated ACA control plane; written
//!   as the relay sender's prologue (the first bytes on the display channel).
//! - [`make_prologue_verifier`] — the gateway-side
//!   [`PrologueVerifier`](nixling_provider_relay::PrologueVerifier) the relay
//!   listener runs **before bridging any byte**; it deserializes + verifies the
//!   handshake (MAC, generation, expiry, field-equality, one-shot anti-replay).
//!
//! Because the relay treats the prologue as opaque bytes + a closure, this is
//! the only place the relay and the gateway credential meet — keeping the
//! transport crate free of any gateway dependency.

use std::sync::{Arc, Mutex};

pub mod aca_workload;
pub mod audit_jsonl;
pub mod credential;
pub mod display_listener;
pub mod production;
pub use aca_workload::{
    AcaGatewayWorkload, AgentBinaries, RelayCoords, build_agent_command, build_cleanup_command,
    default_entra_token_snippet,
};
pub use audit_jsonl::{DEFAULT_GATEWAY_AUDIT_RETENTION_DAYS, JsonlGatewayAudit};
pub use credential::{
    CredentialError, CredentialFilePolicy, GATEWAY_CREDENTIAL_MODE, GatewayCredential,
    MintedRelaySendToken,
};
pub use display_listener::{RelayDisplayListener, notifying_verifier};
pub use production::production_deps_with_audit;
pub use production::{SystemClock, UrandomIds, production_deps, system_now_fn, system_now_unix};

use nixling_gateway::{
    Handshake, SessionBinding, SessionSecret, SetReplayGuard, encode_handshake_frame,
    verify_handshake_frame,
};
use nixling_provider_relay::PrologueVerifier;

/// Build the length-delimited handshake prologue the in-sandbox agent writes
/// as the first bytes on the relay display channel. The agent holds `secret`
/// (delivered over the MI-authenticated ACA control plane) and the `binding`
/// the gateway authorized; this MACs the binding and frames it.
pub fn agent_prologue(secret: &SessionSecret, binding: SessionBinding) -> Vec<u8> {
    let hs = Handshake::sign(binding, secret);
    encode_handshake_frame(&hs)
}

/// A clock the verifier consults at handshake-arrival time (so the expiry check
/// uses when the handshake actually arrived, not when the listener was armed).
pub type NowFn = Arc<dyn Fn() -> u64 + Send + Sync>;

/// Build the gateway-side prologue verifier for one display session. The
/// returned closure, when handed each accepted rendezvous's first frame by
/// [`nixling_provider_relay::run_listener_verified`], admits the connection
/// **only** if the frame is a handshake that verifies against `secret`, the
/// authorizing `expected` binding, the current `generation`, the current time,
/// and the one-shot anti-replay guard. Any failure returns `false` and the
/// relay forwards zero bytes.
pub fn make_prologue_verifier(
    secret: SessionSecret,
    expected: SessionBinding,
    generation: u64,
    now: NowFn,
) -> PrologueVerifier {
    // One-shot replay state, shared across the (possibly retried) rendezvous
    // attempts for this session. `Fn` + interior mutability because the relay
    // verifier is `Fn(&[u8]) -> bool`.
    let replay = Arc::new(Mutex::new(SetReplayGuard::default()));
    Arc::new(move |frame: &[u8]| {
        let mut guard = replay.lock().expect("prologue replay guard mutex poisoned");
        verify_handshake_frame(frame, &secret, &expected, generation, now(), &mut *guard).is_ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nixling_constellation_core::{OperationId, PrincipalId, RealmId, RealmPath, WorkloadId};
    use nixling_gateway::{DisplaySessionId, SECRET_LEN};

    fn binding(generation: u64, not_after: u64) -> (SessionBinding, SessionSecret) {
        let realm = RealmPath::new(vec![RealmId::parse("work").unwrap()]).unwrap();
        let b = SessionBinding::new(
            &realm,
            generation,
            &DisplaySessionId::new("s1"),
            0,
            &OperationId::parse("op-1").unwrap(),
            &PrincipalId::parse("alice").unwrap(),
            &WorkloadId::parse("demo").unwrap(),
            not_after,
        );
        (b, SessionSecret::from_bytes([7u8; SECRET_LEN]))
    }

    fn now_fn(t: u64) -> NowFn {
        Arc::new(move || t)
    }

    #[test]
    fn agent_prologue_is_accepted_by_matching_verifier() {
        let (b, secret) = binding(5, 1000);
        // Agent side: build the prologue from S + binding.
        let frame = agent_prologue(&secret, b.clone());
        // Gateway side: the verifier admits exactly that frame (strip the
        // 4-byte length prefix, as run_listener_verified hands the body).
        let verify = make_prologue_verifier(secret, b, 5, now_fn(900));
        let body = &frame[4..];
        assert!(verify(body));
    }

    #[test]
    fn verifier_rejects_a_tampered_frame() {
        let (b, secret) = binding(5, 1000);
        let mut frame = agent_prologue(&secret, b.clone());
        // Flip a byte in the body.
        let last = frame.len() - 1;
        frame[last] ^= 0xff;
        let verify = make_prologue_verifier(secret, b, 5, now_fn(900));
        assert!(!verify(&frame[4..]));
    }

    #[test]
    fn verifier_rejects_wrong_generation() {
        let (b, secret) = binding(4, 1000);
        let frame = agent_prologue(&secret, b.clone());
        // Gateway restarted -> generation 5; the gen-4 survivor is rejected.
        let verify = make_prologue_verifier(secret, b, 5, now_fn(900));
        assert!(!verify(&frame[4..]));
    }

    #[test]
    fn verifier_rejects_expired() {
        let (b, secret) = binding(5, 1000);
        let frame = agent_prologue(&secret, b.clone());
        let verify = make_prologue_verifier(secret, b, 5, now_fn(1000)); // now == not_after
        assert!(!verify(&frame[4..]));
    }

    #[test]
    fn verifier_is_one_shot() {
        let (b, secret) = binding(5, 1000);
        let frame = agent_prologue(&secret, b.clone());
        let verify = make_prologue_verifier(secret, b, 5, now_fn(900));
        assert!(verify(&frame[4..])); // first use admitted
        assert!(!verify(&frame[4..])); // replay rejected
    }

    #[test]
    fn verifier_rejects_malformed_frame() {
        let (b, secret) = binding(5, 1000);
        let verify = make_prologue_verifier(secret, b, 5, now_fn(900));
        assert!(!verify(b"not a handshake"));
    }
}
