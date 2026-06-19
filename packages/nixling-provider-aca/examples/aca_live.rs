//! Live smoke: drive a real ACA sandbox exec through the Rust provider.
//! Reads ACA_SUBSCRIPTION/ACA_RESOURCE_GROUP/ACA_SANDBOX_GROUP/ACA_REGION and
//! ACA_SANDBOX_ID from the environment, authenticates via the operator's
//! Azure CLI session (plane 1), and runs a command in the sandbox.
use nixling_constellation_core::NodeId;
use nixling_provider_aca::{AcaConfig, AcaWorkloadProvider};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = AcaConfig {
        subscription: std::env::var("ACA_SUBSCRIPTION")?,
        resource_group: std::env::var("ACA_RESOURCE_GROUP")?,
        sandbox_group: std::env::var("ACA_SANDBOX_GROUP")?,
        region: std::env::var("ACA_REGION")?,
        endpoint: std::env::var("ACA_ENDPOINT").ok(),
    };
    let sandbox = std::env::var("ACA_SANDBOX_ID")?;
    let cmd = std::env::var("ACA_CMD")
        .unwrap_or_else(|_| "echo nixling-rust-provider-live; id -un; uname -sr".to_string());

    let provider = AcaWorkloadProvider::new(cfg, NodeId::parse("gateway").unwrap())?;
    println!(
        "[live] reachable: {}",
        provider.sandbox_reachable(&sandbox).await?
    );
    let r = provider.exec_shell(&sandbox, &cmd).await?;
    println!("[live] exit_code = {}", r.exit_code);
    println!("[live] execution_time_ms = {}", r.execution_time_ms);
    println!("[live] stdout:\n{}", r.stdout);
    if !r.stderr.is_empty() {
        println!("[live] stderr:\n{}", r.stderr);
    }
    std::process::exit(r.exit_code);
}
