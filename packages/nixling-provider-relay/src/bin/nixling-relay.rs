//! `nixling-relay`: the productionized relay endpoint binary (ADR 0032, P0).
//!
//! Runs either side of the Azure Relay hybrid connection that carries a
//! constellation display/byte stream:
//!   nixling-relay sender   --target unix-listen:/run/nixling/wp.sock
//!   nixling-relay listener --target unix:/run/user/1000/wpc.sock
//!
//! Auth (per the three-plane model): a pre-minted short-lived Send bearer from
//! NIXLING_RELAY_SAS_TOKEN, a Microsoft Entra bearer token from
//! NIXLING_RELAY_ENTRA_TOKEN, or finally a SAS rule via NIXLING_RELAY_KEY_NAME
//! + NIXLING_RELAY_KEY for tools/tests.
//!
//! Inside an ACA sandbox, NIXLING_RELAY_CA_FILE points at the egress-proxy CA
//! (/etc/ssl/certs/adc-egress-proxy-ca.crt).
use nixling_provider_relay::{
    LocalTarget, RelayCredential, RelayEndpoint, run_listener, run_sender,
};

fn env(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("missing required env {name}"))
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mode = std::env::args().nth(1).unwrap_or_default();
    // --target may be an arg (--target X) or NIXLING_RELAY_TARGET.
    let args: Vec<String> = std::env::args().collect();
    let target_spec = args
        .windows(2)
        .find(|w| w[0] == "--target")
        .map(|w| w[1].clone())
        .or_else(|| std::env::var("NIXLING_RELAY_TARGET").ok())
        .ok_or("missing --target or NIXLING_RELAY_TARGET")?;

    let endpoint = RelayEndpoint {
        namespace: env("NIXLING_RELAY_NAMESPACE")?,
        entity: env("NIXLING_RELAY_ENTITY")?,
    };
    let target = LocalTarget::parse(&target_spec);
    let credential = if let Ok(token) = std::env::var("NIXLING_RELAY_SAS_TOKEN")
        && !token.trim().is_empty()
    {
        RelayCredential::SasToken(token)
    } else if let Ok(token) = std::env::var("NIXLING_RELAY_ENTRA_TOKEN")
        && !token.trim().is_empty()
    {
        RelayCredential::EntraBearer(token)
    } else {
        RelayCredential::Sas {
            key_name: env("NIXLING_RELAY_KEY_NAME")?,
            key: env("NIXLING_RELAY_KEY")?,
        }
    };
    let ca_pem = match std::env::var("NIXLING_RELAY_CA_FILE") {
        Ok(path) => Some(std::fs::read(path)?),
        Err(_) => None,
    };
    let ca = ca_pem.as_deref();

    match mode.as_str() {
        "sender" => {
            eprintln!("[nixling-relay] sender -> {target:?}");
            run_sender(&endpoint, &credential, &target, 600, ca).await?;
        }
        "listener" => {
            eprintln!("[nixling-relay] listener -> {target:?}");
            loop {
                if let Err(e) = run_listener(&endpoint, &credential, &target, 600, ca).await {
                    eprintln!("[nixling-relay] control channel ended: {e}; reconnecting");
                }
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        }
        other => {
            return Err(format!("usage: nixling-relay <sender|listener>; got {other:?}").into());
        }
    }
    Ok(())
}
