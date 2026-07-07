//! The per-session display credential and its handshake (ADR 0032 P0, design
//! §2). Azure Relay Send/Listen and the sandbox managed identity are
//! **reachability-only**; display bytes are admitted only by a gateway-minted
//! one-shot HMAC binding verified *before any byte is bridged to Waypipe*.
//!
//! Flow: the gateway mints a 32-byte secret `S` and a [`SessionBinding`]
//! (realm, gateway generation, session id, operation id, principal, workload,
//! expiry). `S` + the binding are delivered to the sandbox agent over the
//! MI-authenticated ACA control plane (never over Relay, never logged). The
//! agent sends a [`Handshake`] (binding + `HMAC-SHA256(S, canonical(binding))`)
//! as the first bytes on the relay display stream; the gateway
//! [`verify`](SessionBinding::verify)s it constant-time with generation,
//! expiry, one-shot anti-replay, and field-equality checks.

use d2b_realm_core::{OperationId, PrincipalId, RealmPath, WorkloadId};
use hmac::{Mac, SimpleHmac};
use sha2::Sha256;

type HmacSha256 = SimpleHmac<Sha256>;

/// The minted MAC length (HMAC-SHA256).
pub const MAC_LEN: usize = 32;
/// The minted secret length.
pub const SECRET_LEN: usize = 32;

/// An opaque, **non-secret** display-session id (safe to log/audit).
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct DisplaySessionId(String);

impl DisplaySessionId {
    /// Wrap a non-secret id token.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
    /// Borrow the id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Display for DisplaySessionId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The gateway-minted one-shot session secret `S`. Secret bearer: its `Debug`
/// is redacted and it is never serialized into any log/audit/metric/Relay
/// surface. It travels to the agent only over the MI-authenticated ACA
/// control plane.
#[derive(Clone, PartialEq, Eq)]
pub struct SessionSecret([u8; SECRET_LEN]);

impl SessionSecret {
    /// Wrap raw secret bytes (e.g. from an injected CSPRNG `IdSource`).
    pub fn from_bytes(bytes: [u8; SECRET_LEN]) -> Self {
        Self(bytes)
    }
    /// Borrow the raw bytes (for delivery over the ACA control plane only).
    pub fn expose(&self) -> &[u8; SECRET_LEN] {
        &self.0
    }
}

impl core::fmt::Debug for SessionSecret {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SessionSecret(<redacted>)")
    }
}

/// The fields the session MAC binds. Carrying it in the handshake lets the
/// gateway re-derive the MAC and enforce field equality against the
/// authorizing `GatewayDisplayOpen`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionBinding {
    /// The realm of the authorizing operation (most-specific-first form).
    pub realm: String,
    /// The gateway generation (bumped on every gateway-daemon restart).
    pub generation: u64,
    /// The opaque session id.
    pub session_id: String,
    /// The reopen epoch: 0 for the first credential, incremented on every
    /// Degraded->reopen re-mint. The one-shot anti-replay guard is keyed by
    /// `(session_id, epoch)`, so a reopen gets a fresh single-use credential
    /// while a true replay of the same epoch is still rejected.
    pub epoch: u64,
    /// The authorizing operation id.
    pub operation_id: String,
    /// The authorizing caller principal.
    pub principal: String,
    /// The workload presenting the UI.
    pub workload: String,
    /// Unix-seconds expiry; the binding is invalid at or after this.
    pub not_after: u64,
}

impl SessionBinding {
    /// Build a binding from typed ids.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        realm: &RealmPath,
        generation: u64,
        session_id: &DisplaySessionId,
        epoch: u64,
        operation_id: &OperationId,
        principal: &PrincipalId,
        workload: &WorkloadId,
        not_after: u64,
    ) -> Self {
        Self {
            realm: realm.target_form(),
            generation,
            session_id: session_id.as_str().to_owned(),
            epoch,
            operation_id: operation_id.as_str().to_owned(),
            principal: principal.as_str().to_owned(),
            workload: workload.as_str().to_owned(),
            not_after,
        }
    }

    /// The one-shot anti-replay key: a credential is single-use per
    /// `(session_id, epoch)`.
    fn replay_key(&self) -> String {
        format!("{}:{}", self.session_id, self.epoch)
    }

    /// The canonical, unambiguous byte encoding the MAC covers. Each field is
    /// length-prefixed (u32-be) so no field boundary can be shifted into
    /// another (e.g. a realm ending in a digit cannot masquerade as part of
    /// the generation).
    fn canonical(&self) -> Vec<u8> {
        fn put(buf: &mut Vec<u8>, field: &[u8]) {
            buf.extend_from_slice(&(field.len() as u32).to_be_bytes());
            buf.extend_from_slice(field);
        }
        let mut buf = Vec::new();
        put(&mut buf, b"d2b-display-session-v1");
        put(&mut buf, self.realm.as_bytes());
        put(&mut buf, &self.generation.to_be_bytes());
        put(&mut buf, self.session_id.as_bytes());
        put(&mut buf, &self.epoch.to_be_bytes());
        put(&mut buf, self.operation_id.as_bytes());
        put(&mut buf, self.principal.as_bytes());
        put(&mut buf, self.workload.as_bytes());
        put(&mut buf, &self.not_after.to_be_bytes());
        buf
    }

    /// Compute the binding MAC under `secret`.
    pub fn mac(&self, secret: &SessionSecret) -> [u8; MAC_LEN] {
        let mut mac =
            HmacSha256::new_from_slice(secret.expose()).expect("HMAC accepts a 32-byte key");
        mac.update(&self.canonical());
        let out = mac.finalize().into_bytes();
        let mut arr = [0u8; MAC_LEN];
        arr.copy_from_slice(&out);
        arr
    }

    /// Verify a received [`Handshake`] against `secret`, the gateway's
    /// `current_generation`, the authorizing-open `expected` binding, the
    /// current time `now`, and a one-shot `replay`-guard. Fail-closed: any
    /// mismatch returns a typed [`HandshakeError`] and **no** byte is admitted.
    pub fn verify(
        handshake: &Handshake,
        secret: &SessionSecret,
        expected: &SessionBinding,
        current_generation: u64,
        now: u64,
        replay: &mut dyn ReplayGuard,
    ) -> Result<(), HandshakeError> {
        let b = &handshake.binding;
        // 1. Constant-time MAC verification (recompute under the secret).
        let mut mac =
            HmacSha256::new_from_slice(secret.expose()).map_err(|_| HandshakeError::BadMac)?;
        mac.update(&b.canonical());
        mac.verify_slice(&handshake.mac)
            .map_err(|_| HandshakeError::BadMac)?;
        // 2. Generation: reject senders minted before the current gateway life.
        if b.generation != current_generation {
            return Err(HandshakeError::StaleGeneration);
        }
        // 3. Expiry.
        if now >= b.not_after {
            return Err(HandshakeError::Expired);
        }
        // 4. Field equality against the authorizing operation.
        if b.realm != expected.realm
            || b.session_id != expected.session_id
            || b.epoch != expected.epoch
            || b.operation_id != expected.operation_id
            || b.principal != expected.principal
            || b.workload != expected.workload
            || b.not_after != expected.not_after
            || b.generation != expected.generation
        {
            return Err(HandshakeError::BindingMismatch);
        }
        // 5. One-shot anti-replay (a credential admits exactly one stream per
        // (session_id, epoch); a Degraded reopen re-mints with epoch+1). Only
        // reached after every other check passes, so a rejected attempt never
        // consumes the one-shot.
        if !replay.claim(&b.replay_key()) {
            return Err(HandshakeError::Replay);
        }
        Ok(())
    }
}

/// Verify a raw received handshake **frame** (the first bytes on the display
/// channel) before any further byte is bridged. The transport adapter (the
/// relay listener, or a test duplex) reads exactly one length-delimited frame
/// off the channel and hands the JSON bytes here; this deserializes and runs
/// the full [`SessionBinding::verify`] check. Fail-closed: a malformed frame
/// is rejected just like a bad MAC, and the one-shot replay guard is only
/// consumed on a fully-valid handshake.
pub fn verify_handshake_frame(
    frame: &[u8],
    secret: &SessionSecret,
    expected: &SessionBinding,
    current_generation: u64,
    now: u64,
    replay: &mut dyn ReplayGuard,
) -> Result<(), HandshakeError> {
    let hs: Handshake = serde_json::from_slice(frame).map_err(|_| HandshakeError::Malformed)?;
    SessionBinding::verify(&hs, secret, expected, current_generation, now, replay)
}

/// Encode a handshake as a length-delimited frame: a 4-byte big-endian length
/// prefix followed by the JSON body. This is the exact wire the agent writes
/// as the first bytes on the display channel and [`verify_handshake_frame`]
/// reads (after the adapter strips the length prefix).
pub fn encode_handshake_frame(hs: &Handshake) -> Vec<u8> {
    let body = serde_json::to_vec(hs).expect("Handshake serializes");
    let mut out = Vec::with_capacity(4 + body.len());
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    out
}

/// The maximum handshake frame body the listener will read before rejecting
/// (a bound so a hostile sender cannot force unbounded buffering).
pub const MAX_HANDSHAKE_FRAME: usize = 8 * 1024;
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Handshake {
    /// The binding the agent claims.
    pub binding: SessionBinding,
    /// `HMAC-SHA256(S, canonical(binding))`.
    #[serde(with = "mac_bytes")]
    pub mac: [u8; MAC_LEN],
}

impl Handshake {
    /// Build a handshake by MACing `binding` under `secret` (agent side).
    pub fn sign(binding: SessionBinding, secret: &SessionSecret) -> Self {
        let mac = binding.mac(secret);
        Self { binding, mac }
    }
}

/// One-shot anti-replay guard: `claim` returns true the first time a
/// credential key (`session_id:epoch`) is seen and false on every subsequent
/// call.
pub trait ReplayGuard {
    /// Claim `key`; true iff this is its first claim.
    fn claim(&mut self, key: &str) -> bool;
}

/// An in-memory [`ReplayGuard`] backed by a set of claimed credential keys.
#[derive(Debug, Default)]
pub struct SetReplayGuard {
    seen: std::collections::HashSet<String>,
}

impl ReplayGuard for SetReplayGuard {
    fn claim(&mut self, key: &str) -> bool {
        self.seen.insert(key.to_owned())
    }
}

/// Why a handshake was rejected. Every variant is fail-closed; none carries
/// secret material.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakeError {
    /// The MAC did not verify under the session secret.
    BadMac,
    /// The binding's generation is not the gateway's current generation (a
    /// pre-restart survivor).
    StaleGeneration,
    /// The binding has expired (`now >= not_after`).
    Expired,
    /// A binding field differed from the authorizing operation.
    BindingMismatch,
    /// The session id was already claimed (replay).
    Replay,
    /// The handshake frame was not a valid encoded handshake.
    Malformed,
}

impl core::fmt::Display for HandshakeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = match self {
            HandshakeError::BadMac => "session MAC did not verify",
            HandshakeError::StaleGeneration => "session generation is stale",
            HandshakeError::Expired => "session credential expired",
            HandshakeError::BindingMismatch => "session binding mismatch",
            HandshakeError::Replay => "session credential already used",
            HandshakeError::Malformed => "session handshake frame is malformed",
        };
        f.write_str(s)
    }
}

impl std::error::Error for HandshakeError {}

/// Serde for the fixed-size MAC as a byte array.
mod mac_bytes {
    use super::MAC_LEN;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(mac: &[u8; MAC_LEN], s: S) -> Result<S::Ok, S::Error> {
        mac.as_slice().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; MAC_LEN], D::Error> {
        let v = Vec::<u8>::deserialize(d)?;
        let arr: [u8; MAC_LEN] = v
            .try_into()
            .map_err(|_| serde::de::Error::custom("mac must be 32 bytes"))?;
        Ok(arr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> (
        RealmPath,
        DisplaySessionId,
        OperationId,
        PrincipalId,
        WorkloadId,
    ) {
        (
            RealmPath::new(vec![d2b_realm_core::RealmId::parse("work").unwrap()]).unwrap(),
            DisplaySessionId::new("sess-1"),
            OperationId::parse("op-1").unwrap(),
            PrincipalId::parse("alice").unwrap(),
            WorkloadId::parse("demo").unwrap(),
        )
    }

    fn binding(generation: u64, not_after: u64) -> (SessionBinding, SessionSecret) {
        let (realm, sid, op, pr, wl) = ids();
        let secret = SessionSecret::from_bytes([7u8; SECRET_LEN]);
        let b = SessionBinding::new(&realm, generation, &sid, 0, &op, &pr, &wl, not_after);
        (b, secret)
    }

    #[test]
    fn good_handshake_verifies() {
        let (b, secret) = binding(5, 1000);
        let hs = Handshake::sign(b.clone(), &secret);
        let mut replay = SetReplayGuard::default();
        assert_eq!(
            SessionBinding::verify(&hs, &secret, &b, 5, 900, &mut replay),
            Ok(())
        );
    }

    #[test]
    fn wrong_secret_is_bad_mac() {
        let (b, secret) = binding(5, 1000);
        let hs = Handshake::sign(b.clone(), &secret);
        let other = SessionSecret::from_bytes([9u8; SECRET_LEN]);
        let mut replay = SetReplayGuard::default();
        assert_eq!(
            SessionBinding::verify(&hs, &other, &b, 5, 900, &mut replay),
            Err(HandshakeError::BadMac)
        );
    }

    #[test]
    fn tampered_binding_is_bad_mac() {
        let (b, secret) = binding(5, 1000);
        let mut hs = Handshake::sign(b.clone(), &secret);
        hs.binding.workload = "evil".into(); // MAC no longer matches
        let mut replay = SetReplayGuard::default();
        assert_eq!(
            SessionBinding::verify(&hs, &secret, &b, 5, 900, &mut replay),
            Err(HandshakeError::BadMac)
        );
    }

    #[test]
    fn stale_generation_rejected() {
        let (b, secret) = binding(4, 1000);
        let hs = Handshake::sign(b.clone(), &secret);
        let mut replay = SetReplayGuard::default();
        // Gateway restarted: current generation is now 5; the survivor minted
        // at 4 must be rejected even though its MAC is valid.
        assert_eq!(
            SessionBinding::verify(&hs, &secret, &b, 5, 900, &mut replay),
            Err(HandshakeError::StaleGeneration)
        );
    }

    #[test]
    fn expired_rejected() {
        let (b, secret) = binding(5, 1000);
        let hs = Handshake::sign(b.clone(), &secret);
        let mut replay = SetReplayGuard::default();
        assert_eq!(
            SessionBinding::verify(&hs, &secret, &b, 5, 1000, &mut replay),
            Err(HandshakeError::Expired)
        );
    }

    #[test]
    fn binding_mismatch_rejected() {
        // A valid MAC over a binding whose principal differs from the
        // authorizing operation: caught by field equality, not the MAC.
        let (realm, sid, op, _pr, wl) = ids();
        let secret = SessionSecret::from_bytes([7u8; SECRET_LEN]);
        let authorized = SessionBinding::new(
            &realm,
            5,
            &sid,
            0,
            &op,
            &PrincipalId::parse("alice").unwrap(),
            &wl,
            1000,
        );
        let attacker = SessionBinding::new(
            &realm,
            5,
            &sid,
            0,
            &op,
            &PrincipalId::parse("mallory").unwrap(),
            &wl,
            1000,
        );
        let hs = Handshake::sign(attacker, &secret);
        let mut replay = SetReplayGuard::default();
        assert_eq!(
            SessionBinding::verify(&hs, &secret, &authorized, 5, 900, &mut replay),
            Err(HandshakeError::BindingMismatch)
        );
    }

    #[test]
    fn every_field_mismatch_is_rejected() {
        // Table-driven: a valid MAC over a binding that differs from the
        // authorizing binding in exactly one field must be rejected, so
        // removing any single equality check fails a test.
        let secret = SessionSecret::from_bytes([7u8; SECRET_LEN]);
        let realm = RealmPath::new(vec![d2b_realm_core::RealmId::parse("work").unwrap()]).unwrap();
        let realm2 = RealmPath::new(vec![d2b_realm_core::RealmId::parse("ops").unwrap()]).unwrap();
        let base = SessionBinding::new(
            &realm,
            5,
            &DisplaySessionId::new("s1"),
            0,
            &OperationId::parse("op-1").unwrap(),
            &PrincipalId::parse("alice").unwrap(),
            &WorkloadId::parse("demo").unwrap(),
            1000,
        );
        let variants = [
            SessionBinding {
                realm: realm2.target_form(),
                ..base.clone()
            },
            SessionBinding {
                generation: 6,
                ..base.clone()
            },
            SessionBinding {
                session_id: "s2".into(),
                ..base.clone()
            },
            SessionBinding {
                epoch: 1,
                ..base.clone()
            },
            SessionBinding {
                operation_id: "op-2".into(),
                ..base.clone()
            },
            SessionBinding {
                principal: "bob".into(),
                ..base.clone()
            },
            SessionBinding {
                workload: "other".into(),
                ..base.clone()
            },
            SessionBinding {
                not_after: 1001,
                ..base.clone()
            },
        ];
        for (i, v) in variants.into_iter().enumerate() {
            // Sign the *variant* (valid MAC) but verify against `base`.
            let hs = Handshake::sign(v, &secret);
            let mut replay = SetReplayGuard::default();
            let r = SessionBinding::verify(&hs, &secret, &base, 5, 900, &mut replay);
            // generation/expiry have dedicated kinds; all others are mismatch.
            assert!(
                matches!(
                    r,
                    Err(HandshakeError::BindingMismatch)
                        | Err(HandshakeError::StaleGeneration)
                        | Err(HandshakeError::Expired)
                ),
                "variant {i} should be rejected, got {r:?}"
            );
            assert!(r.is_err(), "variant {i} must not verify");
        }
    }

    #[test]
    fn rejected_attempt_does_not_consume_the_one_shot() {
        // A bad-MAC attempt for a credential key must NOT consume the replay
        // guard; the legitimate sender can then still use it exactly once.
        let (b, secret) = binding(5, 1000);
        let bad = Handshake::sign(b.clone(), &SessionSecret::from_bytes([0u8; SECRET_LEN]));
        let good = Handshake::sign(b.clone(), &secret);
        let mut replay = SetReplayGuard::default();
        assert_eq!(
            SessionBinding::verify(&bad, &secret, &b, 5, 900, &mut replay),
            Err(HandshakeError::BadMac)
        );
        // The good handshake (same session_id:epoch) still succeeds.
        assert_eq!(
            SessionBinding::verify(&good, &secret, &b, 5, 900, &mut replay),
            Ok(())
        );
    }

    #[test]
    fn reopen_with_next_epoch_is_a_fresh_one_shot() {
        // epoch 0 consumed; a Degraded->reopen re-mints epoch 1, which the
        // replay guard treats as a distinct one-shot credential.
        let (realm, sid, op, pr, wl) = ids();
        let secret = SessionSecret::from_bytes([7u8; SECRET_LEN]);
        let e0 = SessionBinding::new(&realm, 5, &sid, 0, &op, &pr, &wl, 1000);
        let e1 = SessionBinding::new(&realm, 5, &sid, 1, &op, &pr, &wl, 1000);
        let mut replay = SetReplayGuard::default();
        assert_eq!(
            SessionBinding::verify(
                &Handshake::sign(e0.clone(), &secret),
                &secret,
                &e0,
                5,
                900,
                &mut replay
            ),
            Ok(())
        );
        // epoch 0 replay rejected, epoch 1 accepted.
        assert_eq!(
            SessionBinding::verify(
                &Handshake::sign(e0.clone(), &secret),
                &secret,
                &e0,
                5,
                900,
                &mut replay
            ),
            Err(HandshakeError::Replay)
        );
        assert_eq!(
            SessionBinding::verify(
                &Handshake::sign(e1.clone(), &secret),
                &secret,
                &e1,
                5,
                900,
                &mut replay
            ),
            Ok(())
        );
    }

    #[test]
    fn replay_rejected_on_second_use() {
        let (b, secret) = binding(5, 1000);
        let hs = Handshake::sign(b.clone(), &secret);
        let mut replay = SetReplayGuard::default();
        assert_eq!(
            SessionBinding::verify(&hs, &secret, &b, 5, 900, &mut replay),
            Ok(())
        );
        // Same one-shot session id, replayed: rejected.
        assert_eq!(
            SessionBinding::verify(&hs, &secret, &b, 5, 900, &mut replay),
            Err(HandshakeError::Replay)
        );
    }

    #[test]
    fn handshake_round_trips_through_json() {
        let (b, secret) = binding(5, 1000);
        let hs = Handshake::sign(b, &secret);
        let json = serde_json::to_string(&hs).unwrap();
        let back: Handshake = serde_json::from_str(&json).unwrap();
        assert_eq!(hs, back);
    }

    #[test]
    fn secret_debug_is_redacted() {
        let s = SessionSecret::from_bytes([1u8; SECRET_LEN]);
        assert_eq!(format!("{s:?}"), "SessionSecret(<redacted>)");
    }

    #[test]
    fn frame_round_trip_verifies() {
        let (b, secret) = binding(5, 1000);
        let hs = Handshake::sign(b.clone(), &secret);
        let frame = encode_handshake_frame(&hs);
        // Strip the 4-byte length prefix the adapter would consume.
        let body = &frame[4..];
        assert_eq!(
            u32::from_be_bytes(frame[..4].try_into().unwrap()) as usize,
            body.len()
        );
        let mut replay = SetReplayGuard::default();
        assert_eq!(
            verify_handshake_frame(body, &secret, &b, 5, 900, &mut replay),
            Ok(())
        );
    }

    #[test]
    fn malformed_frame_rejected() {
        let (b, secret) = binding(5, 1000);
        let mut replay = SetReplayGuard::default();
        assert_eq!(
            verify_handshake_frame(b"not json", &secret, &b, 5, 900, &mut replay),
            Err(HandshakeError::Malformed)
        );
    }

    #[test]
    fn frame_with_bad_mac_rejected_without_consuming_replay() {
        let (b, secret) = binding(5, 1000);
        let other = SessionSecret::from_bytes([9u8; SECRET_LEN]);
        let frame = encode_handshake_frame(&Handshake::sign(b.clone(), &secret));
        let body = frame[4..].to_vec();
        let mut replay = SetReplayGuard::default();
        // Wrong secret -> BadMac, replay not consumed.
        assert_eq!(
            verify_handshake_frame(&body, &other, &b, 5, 900, &mut replay),
            Err(HandshakeError::BadMac)
        );
        // Correct secret still works (one-shot intact).
        assert_eq!(
            verify_handshake_frame(&body, &secret, &b, 5, 900, &mut replay),
            Ok(())
        );
    }
}
