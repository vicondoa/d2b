//! Productionized host-side relay listener: bridges the Azure Relay hybrid
//! connection to a local target (the host waypipe-client socket), using
//! nixling-provider-relay's run_listener. Reconnects the control channel.
//!
//! Env: NIXLING_RELAY_NAMESPACE, NIXLING_RELAY_ENTITY, NIXLING_RELAY_TARGET
//! (e.g. unix:/run/user/1000/wpc.sock), and the Listen credential via either
//! NIXLING_RELAY_ENTRA_TOKEN or NIXLING_RELAY_KEY_NAME + NIXLING_RELAY_KEY.
use nixling_provider_relay::{run_listener, LocalTarget, RelayCredential, RelayEndpoint};

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = RelayEndpoint {
        namespace: std::env::var("NIXLING_RELAY_NAMESPACE")?,
        entity: std::env::var("NIXLING_RELAY_ENTITY")?,
    };
    let target = LocalTarget::parse(&std::env::var("NIXLING_RELAY_TARGET")?);
    let credential = if let Ok(token) = std::env::var("NIXLING_RELAY_ENTRA_TOKEN") {
        RelayCredential::EntraBearer(token)
    } else {
        RelayCredential::Sas {
            key_name: std::env::var("NIXLING_RELAY_KEY_NAME")?,
            key: std::env::var("NIXLING_RELAY_KEY")?,
        }
    };
    eprintln!("[host-listener] ready; bridging relay -> {:?}", target);
    loop {
        if let Err(e) = run_listener(&endpoint, &credential, &target, 600, None).await {
            eprintln!("[host-listener] control channel ended: {e}; reconnecting in 1s");
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}
