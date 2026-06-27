//! Productionized host-side relay listener: bridges the Azure Relay hybrid
//! connection to a local target (the host waypipe-client socket), using
//! d2b-provider-relay's run_listener. Reconnects the control channel.
//!
//! Env: D2B_RELAY_NAMESPACE, D2B_RELAY_ENTITY, D2B_RELAY_TARGET
//! (e.g. unix:/run/user/1000/wpc.sock), and the Listen credential via either
//! D2B_RELAY_ENTRA_TOKEN or D2B_RELAY_KEY_NAME + D2B_RELAY_KEY.
use d2b_provider_relay::{LocalTarget, RelayCredential, RelayEndpoint, run_listener};

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = RelayEndpoint {
        namespace: std::env::var("D2B_RELAY_NAMESPACE")?,
        entity: std::env::var("D2B_RELAY_ENTITY")?,
    };
    let target = LocalTarget::parse(&std::env::var("D2B_RELAY_TARGET")?);
    let credential = if let Ok(token) = std::env::var("D2B_RELAY_ENTRA_TOKEN") {
        RelayCredential::EntraBearer(token)
    } else {
        RelayCredential::Sas {
            key_name: std::env::var("D2B_RELAY_KEY_NAME")?,
            key: std::env::var("D2B_RELAY_KEY")?,
        }
    };
    eprintln!("[host-listener] starting; bridging relay -> {:?}", target);
    loop {
        eprintln!("[host-listener] opening listen control channel...");
        match run_listener(&endpoint, &credential, &target, 600, None).await {
            Ok(()) => eprintln!("[host-listener] control channel closed; reconnecting in 1s"),
            Err(e) => eprintln!("[host-listener] control channel error: {e}; reconnecting in 1s"),
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}
