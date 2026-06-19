//! Live relay connect probe. Verifies the relay accepts the credential by
//! completing (or rejecting) the WebSocket handshake.
//!
//! Env: NIXLING_RELAY_NAMESPACE, NIXLING_RELAY_ENTITY, role via NIXLING_RELAY_ROLE
//! (listen|connect), and EITHER NIXLING_RELAY_ENTRA_TOKEN OR
//! NIXLING_RELAY_KEY_NAME + NIXLING_RELAY_KEY.
use nixling_provider_relay::{RelayCredential, RelayEndpoint, RelayRole, connect};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = RelayEndpoint {
        namespace: std::env::var("NIXLING_RELAY_NAMESPACE")?,
        entity: std::env::var("NIXLING_RELAY_ENTITY")?,
    };
    let role = match std::env::var("NIXLING_RELAY_ROLE").as_deref() {
        Ok("listen") => RelayRole::Listener,
        _ => RelayRole::Sender,
    };
    let credential = if let Ok(token) = std::env::var("NIXLING_RELAY_ENTRA_TOKEN") {
        RelayCredential::EntraBearer(token)
    } else {
        RelayCredential::Sas {
            key_name: std::env::var("NIXLING_RELAY_KEY_NAME")?,
            key: std::env::var("NIXLING_RELAY_KEY")?,
        }
    };
    eprintln!("[probe] connecting role={:?} cred={:?}", role, credential);
    match connect(&endpoint, role, &credential, 600).await {
        Ok(_ws) => {
            println!("[probe] HANDSHAKE OK — relay accepted the credential");
            Ok(())
        }
        Err(e) => {
            println!("[probe] HANDSHAKE FAILED — {e}");
            std::process::exit(1);
        }
    }
}
