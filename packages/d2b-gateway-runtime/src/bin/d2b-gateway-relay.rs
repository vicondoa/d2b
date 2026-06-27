//! `d2b-gateway-relay` — the handshake-gated relay endpoint (ADR 0032).
//!
//! Unlike the plain `d2b-relay` byte tunnel, this endpoint carries the
//! per-session credential as the relay prologue:
//!
//! - `sender` (in the sandbox): builds the handshake from the session secret +
//!   binding it received over the MI-authenticated ACA control plane and writes
//!   it as the relay prologue, then bridges the local Waypipe socket.
//! - `listener` (on the gateway): verifies that prologue before bridging any
//!   byte to the operator Waypipe socket — so display bytes are admitted only
//!   by the verified session credential.
//!
//! Session parameters arrive as env so the gateway can hand them to the agent
//! over ACA exec without ever putting the secret on a command line.
//!
//!   D2B_SESSION_SECRET_B64   base64 of the 32-byte session secret S
//!   D2B_SESSION_REALM        realm target form (e.g. `work`)
//!   D2B_SESSION_GENERATION   gateway generation (u64)
//!   D2B_SESSION_ID           opaque session id
//!   D2B_SESSION_EPOCH        reopen epoch (u64, default 0)
//!   D2B_SESSION_OP           authorizing operation id
//!   D2B_SESSION_PRINCIPAL    authorizing principal
//!   D2B_SESSION_WORKLOAD     workload id
//!   D2B_SESSION_NOT_AFTER    unix-seconds expiry (u64)
//!
//!   D2B_RELAY_NAMESPACE / D2B_RELAY_ENTITY / D2B_RELAY_TARGET
//!   D2B_RELAY_SAS_TOKEN    short-lived Send bearer
//!   D2B_RELAY_ENTRA_TOKEN  Entra bearer, or key envs for tools/tests
//!   D2B_RELAY_KEY_NAME / D2B_RELAY_KEY
//!   D2B_RELAY_CA_FILE      (optional, sandbox egress-proxy CA)

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use d2b_gateway::{SECRET_LEN, SessionBinding, SessionSecret};
use d2b_gateway_runtime::{agent_prologue, make_prologue_verifier};
use d2b_provider_relay::{LocalTarget, RelayCredential, RelayEndpoint};

type Err = Box<dyn std::error::Error>;

fn env(k: &str) -> Result<String, Err> {
    std::env::var(k).map_err(|_| format!("missing env {k}").into())
}

fn binding_and_secret() -> Result<(SessionBinding, SessionSecret), Err> {
    let raw = base64::engine::general_purpose::STANDARD
        .decode(env("D2B_SESSION_SECRET_B64")?.trim())
        .map_err(|_| "D2B_SESSION_SECRET_B64 is not valid base64")?;
    let bytes: [u8; SECRET_LEN] = raw
        .as_slice()
        .try_into()
        .map_err(|_| "session secret must be 32 bytes")?;
    let binding = SessionBinding {
        realm: env("D2B_SESSION_REALM")?,
        generation: env("D2B_SESSION_GENERATION")?.parse()?,
        session_id: env("D2B_SESSION_ID")?,
        epoch: std::env::var("D2B_SESSION_EPOCH")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
        operation_id: env("D2B_SESSION_OP")?,
        principal: env("D2B_SESSION_PRINCIPAL")?,
        workload: env("D2B_SESSION_WORKLOAD")?,
        not_after: env("D2B_SESSION_NOT_AFTER")?.parse()?,
    };
    Ok((binding, SessionSecret::from_bytes(bytes)))
}

fn endpoint() -> Result<RelayEndpoint, Err> {
    Ok(RelayEndpoint {
        namespace: env("D2B_RELAY_NAMESPACE")?,
        entity: env("D2B_RELAY_ENTITY")?,
    })
}

fn credential() -> Result<RelayCredential, Err> {
    if let Ok(token) = std::env::var("D2B_RELAY_SAS_TOKEN")
        && !token.trim().is_empty()
    {
        Ok(RelayCredential::SasToken(token))
    } else if let Ok(token) = std::env::var("D2B_RELAY_ENTRA_TOKEN")
        && !token.trim().is_empty()
    {
        Ok(RelayCredential::EntraBearer(token))
    } else {
        Ok(RelayCredential::Sas {
            key_name: env("D2B_RELAY_KEY_NAME")?,
            key: env("D2B_RELAY_KEY")?,
        })
    }
}

fn ca() -> Option<Vec<u8>> {
    std::env::var("D2B_RELAY_CA_FILE")
        .ok()
        .and_then(|p| std::fs::read(p).ok())
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Err> {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let target = LocalTarget::parse(&env("D2B_RELAY_TARGET")?);
    let endpoint = endpoint()?;
    let credential = credential()?;
    let ca = ca();
    let (binding, secret) = binding_and_secret()?;

    match mode.as_str() {
        "sender" => {
            eprintln!("[d2b-gateway-relay] sender (handshake prologue) -> {target:?}");
            let prologue = agent_prologue(&secret, binding);
            d2b_provider_relay::run_sender_with_prologue(
                &endpoint,
                &credential,
                &target,
                600,
                ca.as_deref(),
                &prologue,
            )
            .await?;
        }
        "listener" => {
            eprintln!("[d2b-gateway-relay] listener (verifying handshake) -> {target:?}");
            let generation = binding.generation;
            let verify = make_prologue_verifier(secret, binding, generation, Arc::new(now_unix));
            loop {
                eprintln!("[d2b-gateway-relay] arming verified listener...");
                if let Err(e) = d2b_provider_relay::run_listener_verified(
                    &endpoint,
                    &credential,
                    &target,
                    600,
                    ca.as_deref(),
                    verify.clone(),
                )
                .await
                {
                    eprintln!("[d2b-gateway-relay] listener ended: {e}; reconnecting in 1s");
                }
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        }
        other => {
            return Err(
                format!("usage: d2b-gateway-relay <sender|listener>; got {other:?}").into(),
            );
        }
    }
    Ok(())
}
