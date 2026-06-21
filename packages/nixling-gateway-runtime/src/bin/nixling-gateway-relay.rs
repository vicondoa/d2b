//! `nixling-gateway-relay` — the handshake-gated relay endpoint (ADR 0032).
//!
//! Unlike the plain `nixling-relay` byte tunnel, this endpoint carries the
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
//!   NL_SESSION_SECRET_B64   base64 of the 32-byte session secret S
//!   NL_SESSION_REALM        realm target form (e.g. `work`)
//!   NL_SESSION_GENERATION   gateway generation (u64)
//!   NL_SESSION_ID           opaque session id
//!   NL_SESSION_EPOCH        reopen epoch (u64, default 0)
//!   NL_SESSION_OP           authorizing operation id
//!   NL_SESSION_PRINCIPAL    authorizing principal
//!   NL_SESSION_WORKLOAD     workload id
//!   NL_SESSION_NOT_AFTER    unix-seconds expiry (u64)
//!
//!   NIXLING_RELAY_NAMESPACE / NIXLING_RELAY_ENTITY / NIXLING_RELAY_TARGET
//!   NIXLING_RELAY_SAS_TOKEN    short-lived Send bearer
//!   NIXLING_RELAY_ENTRA_TOKEN  Entra bearer, or key envs for tools/tests
//!   NIXLING_RELAY_KEY_NAME / NIXLING_RELAY_KEY
//!   NIXLING_RELAY_CA_FILE      (optional, sandbox egress-proxy CA)

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use nixling_gateway::{SECRET_LEN, SessionBinding, SessionSecret};
use nixling_gateway_runtime::{agent_prologue, make_prologue_verifier};
use nixling_provider_relay::{LocalTarget, RelayCredential, RelayEndpoint};

type Err = Box<dyn std::error::Error>;

fn env(k: &str) -> Result<String, Err> {
    std::env::var(k).map_err(|_| format!("missing env {k}").into())
}

fn binding_and_secret() -> Result<(SessionBinding, SessionSecret), Err> {
    let raw = base64::engine::general_purpose::STANDARD
        .decode(env("NL_SESSION_SECRET_B64")?.trim())
        .map_err(|_| "NL_SESSION_SECRET_B64 is not valid base64")?;
    let bytes: [u8; SECRET_LEN] = raw
        .as_slice()
        .try_into()
        .map_err(|_| "session secret must be 32 bytes")?;
    let binding = SessionBinding {
        realm: env("NL_SESSION_REALM")?,
        generation: env("NL_SESSION_GENERATION")?.parse()?,
        session_id: env("NL_SESSION_ID")?,
        epoch: std::env::var("NL_SESSION_EPOCH")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
        operation_id: env("NL_SESSION_OP")?,
        principal: env("NL_SESSION_PRINCIPAL")?,
        workload: env("NL_SESSION_WORKLOAD")?,
        not_after: env("NL_SESSION_NOT_AFTER")?.parse()?,
    };
    Ok((binding, SessionSecret::from_bytes(bytes)))
}

fn endpoint() -> Result<RelayEndpoint, Err> {
    Ok(RelayEndpoint {
        namespace: env("NIXLING_RELAY_NAMESPACE")?,
        entity: env("NIXLING_RELAY_ENTITY")?,
    })
}

fn credential() -> Result<RelayCredential, Err> {
    if let Ok(token) = std::env::var("NIXLING_RELAY_SAS_TOKEN")
        && !token.trim().is_empty()
    {
        Ok(RelayCredential::SasToken(token))
    } else if let Ok(token) = std::env::var("NIXLING_RELAY_ENTRA_TOKEN")
        && !token.trim().is_empty()
    {
        Ok(RelayCredential::EntraBearer(token))
    } else {
        Ok(RelayCredential::Sas {
            key_name: env("NIXLING_RELAY_KEY_NAME")?,
            key: env("NIXLING_RELAY_KEY")?,
        })
    }
}

fn ca() -> Option<Vec<u8>> {
    std::env::var("NIXLING_RELAY_CA_FILE")
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
    let target = LocalTarget::parse(&env("NIXLING_RELAY_TARGET")?);
    let endpoint = endpoint()?;
    let credential = credential()?;
    let ca = ca();
    let (binding, secret) = binding_and_secret()?;

    match mode.as_str() {
        "sender" => {
            eprintln!("[nixling-gateway-relay] sender (handshake prologue) -> {target:?}");
            let prologue = agent_prologue(&secret, binding);
            nixling_provider_relay::run_sender_with_prologue(
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
            eprintln!("[nixling-gateway-relay] listener (verifying handshake) -> {target:?}");
            let generation = binding.generation;
            let verify = make_prologue_verifier(secret, binding, generation, Arc::new(now_unix));
            loop {
                eprintln!("[nixling-gateway-relay] arming verified listener...");
                if let Err(e) = nixling_provider_relay::run_listener_verified(
                    &endpoint,
                    &credential,
                    &target,
                    600,
                    ca.as_deref(),
                    verify.clone(),
                )
                .await
                {
                    eprintln!("[nixling-gateway-relay] listener ended: {e}; reconnecting in 1s");
                }
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        }
        other => {
            return Err(
                format!("usage: nixling-gateway-relay <sender|listener>; got {other:?}").into(),
            );
        }
    }
    Ok(())
}
