//! Provider-agent and retained gateway compatibility composition.

use std::sync::Arc;

pub mod aca_workload;
pub mod audit_jsonl;
pub mod credential;
pub mod display_listener;
pub mod production;
pub mod provider_agent;
pub mod waypipe_display;
pub use aca_workload::{
    AcaGatewayWorkload, AgentBinaries, RelayCoords, build_agent_command, build_cleanup_command,
    default_entra_token_snippet, relay_sas_token_snippet,
};
pub use audit_jsonl::{DEFAULT_GATEWAY_AUDIT_RETENTION_DAYS, JsonlGatewayAudit};
pub use credential::{
    CredentialEnvelopeMeta, CredentialError, CredentialFilePolicy, GATEWAY_CREDENTIAL_MODE,
    GATEWAY_CREDENTIAL_SCHEMA_VERSION, GATEWAY_SEAL_KEY_LEN, GATEWAY_SEAL_KEY_MODE,
    GatewayCredential, GatewayCredentialMaterial, MintedRelaySendToken, SealingKey,
};
pub use display_listener::RelayDisplayListener;
pub use production::production_deps_with_audit;
pub use production::{SystemClock, UrandomIds, production_deps, system_now_fn, system_now_unix};
pub use waypipe_display::{
    WaypipeCompression, WaypipeDisplayProvider, WaypipeRunnerConfig, WaypipeSystemdService,
    gated_relay_sender_argv, guest_waypipe_server_argv, guest_waypipe_service,
    host_waypipe_client_argv, host_waypipe_service,
};

/// Compatibility clock accepted by the retired relay display constructor.
pub type NowFn = Arc<dyn Fn() -> u64 + Send + Sync>;
