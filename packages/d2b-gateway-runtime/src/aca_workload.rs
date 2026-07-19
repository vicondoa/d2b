//! Fail-closed compatibility surface for the retired ACA gateway workload.
//!
//! The only command this adapter can shape is the reserved
//! `d2b-provider-agent` entrypoint. It does not place session, relay, credential,
//! or application data in the environment or argv.

use async_trait::async_trait;
use base64::Engine;
use d2b_gateway::{AgentHandle, AgentSpawnRequest, GatewayError, GatewayWorkload};
use d2b_provider_aca::AcaWorkloadProvider;
use d2b_realm_core::WorkloadId;

/// Azure Relay coordinates the in-sandbox sender dials out to.
#[derive(Debug, Clone)]
pub struct RelayCoords {
    /// Namespace FQDN, e.g. `relns-xxxx.servicebus.windows.net`.
    pub namespace: String,
    /// Hybrid-connection (entity) name, e.g. `hc-d2b-display`.
    pub entity: String,
    /// In-sandbox path to the egress-proxy CA the relay TLS is terminated by
    /// (ACA terminates egress TLS); `None` for direct egress.
    pub ca_file: Option<String>,
    /// Optional user-assigned managed identity client id for ACA's MSI
    /// endpoint. Some ACA sandboxes return 500 unless this is supplied.
    pub managed_identity_client_id: Option<String>,
}

/// In-image binary + tunable names for the agent command.
#[derive(Debug, Clone)]
pub struct AgentBinaries {
    /// The typed provider-agent process.
    pub provider_agent: String,
    /// The upstream `waypipe` binary.
    pub waypipe: String,
    /// Waypipe channel compression (`zstd`/`lz4`/`none`); must match the host.
    pub compression: String,
    /// The Wayland display name the persistent server exposes.
    pub display: String,
}

impl Default for AgentBinaries {
    fn default() -> Self {
        Self {
            provider_agent: "d2b-provider-agent".to_owned(),
            waypipe: "waypipe".to_owned(),
            compression: "zstd".to_owned(),
            display: "wayland-d2b".to_owned(),
        }
    }
}

/// Retained configuration helper for callers migrating from the old adapter.
/// The reserved provider-agent command does not consume this value.
pub fn default_entra_token_snippet() -> String {
    "D2B_RELAY_ENTRA_TOKEN=\"$(d2b-msi-token https://relay.azure.net/ \"${D2B_MI_CLIENT_ID:-}\")\""
        .to_owned()
}

/// Retained configuration helper. The reserved provider-agent command ignores
/// this value and never receives it through ambient process state.
pub fn relay_sas_token_snippet(token: &str) -> String {
    format!("D2B_RELAY_SAS_TOKEN={}", sh_quote(token))
}

/// The production [`GatewayWorkload`]: drives the in-sandbox agent over ACA
/// `executeShellCommand`.
pub struct AcaGatewayWorkload {
    provider: AcaWorkloadProvider,
    sandbox: AcaSandboxRef,
    relay: RelayCoords,
    bins: AgentBinaries,
    relay_auth_snippet: String,
}

#[derive(Debug, Clone)]
enum AcaSandboxRef {
    FixedId(String),
    WorkloadLabel,
}

impl AcaGatewayWorkload {
    /// Build the adapter for one already-known sandbox.
    pub fn new(
        provider: AcaWorkloadProvider,
        sandbox_id: impl Into<String>,
        relay: RelayCoords,
    ) -> Self {
        Self {
            provider,
            sandbox: AcaSandboxRef::FixedId(sandbox_id.into()),
            relay,
            bins: AgentBinaries::default(),
            relay_auth_snippet: default_entra_token_snippet(),
        }
    }

    /// Build the adapter that resolves/creates sandboxes by the workload alias
    /// carried in each authorized gateway request.
    pub fn for_workload_labels(provider: AcaWorkloadProvider, relay: RelayCoords) -> Self {
        Self {
            provider,
            sandbox: AcaSandboxRef::WorkloadLabel,
            relay,
            bins: AgentBinaries::default(),
            relay_auth_snippet: default_entra_token_snippet(),
        }
    }

    /// Override the in-image binary names / compression / display.
    pub fn with_binaries(mut self, bins: AgentBinaries) -> Self {
        self.bins = bins;
        self
    }

    /// Override the MI-token-fetch shell snippet (e.g. for a SAS test rig).
    pub fn with_entra_token_snippet(mut self, snippet: impl Into<String>) -> Self {
        self.relay_auth_snippet = snippet.into();
        self
    }

    /// Override the relay-auth shell snippet.
    pub fn with_relay_auth_snippet(mut self, snippet: impl Into<String>) -> Self {
        self.relay_auth_snippet = snippet.into();
        self
    }

    async fn resolve_sandbox_id(&self, req: &AgentSpawnRequest) -> Result<String, GatewayError> {
        match &self.sandbox {
            AcaSandboxRef::FixedId(id) => Ok(id.clone()),
            AcaSandboxRef::WorkloadLabel => {
                let workload = WorkloadId::parse(req.binding.workload.clone())
                    .map_err(|_| GatewayError::ProviderAllocationFailed)?;
                self.provider
                    .find_workload_sandbox(&workload)
                    .await
                    .map_err(|err| {
                        tracing::warn!(error = %err, "aca gateway workload sandbox lookup failed");
                        GatewayError::ProviderAllocationFailed
                    })?
                    .map(|sandbox| {
                        tracing::info!(
                            sandbox_id = %sandbox.id,
                            state = ?sandbox.state,
                            "aca gateway workload sandbox selected"
                        );
                        sandbox.id
                    })
                    .ok_or_else(|| {
                        tracing::warn!(
                            workload = %workload.as_str(),
                            "aca gateway workload sandbox not found"
                        );
                        GatewayError::ProviderAllocationFailed
                    })
            }
        }
    }
}

/// Single-quote a string for POSIX `sh`, escaping embedded single quotes so an
/// operator-supplied argv element can never break out of the quoting.
fn sh_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

/// Build the only permitted compatibility command. The request and legacy
/// relay parameters are deliberately ignored.
pub fn build_agent_command(
    _req: &AgentSpawnRequest,
    _relay: &RelayCoords,
    bins: &AgentBinaries,
    _relay_auth_snippet: &str,
) -> String {
    format!("set -e\nexec {}\n", sh_quote(&bins.provider_agent))
}

/// Build the idempotent cleanup command for a spawned agent: kill the recorded
/// pids and remove the workdir.
pub fn build_cleanup_command(workdir: &str) -> String {
    format!(
        "W={w}\n\
[ -f \"$W/pids\" ] && while read p; do kill \"$p\" 2>/dev/null || true; done < \"$W/pids\"\n\
rm -rf \"$W\"\n\
echo D2B_AGENT_CLEANED\n",
        w = sh_quote(workdir),
    )
}

/// Extract the durable workdir handle from the agent script's stdout.
fn parse_workdir(stdout: &str) -> Option<String> {
    stdout.lines().rev().find_map(|l| {
        l.trim()
            .strip_prefix("D2B_AGENT_WORKDIR=")
            .map(str::to_owned)
    })
}

fn encode_agent_handle(sandbox_id: &str, workdir: &str) -> AgentHandle {
    let workdir_b64 = base64::engine::general_purpose::STANDARD.encode(workdir.as_bytes());
    AgentHandle(format!("aca:{sandbox_id}:{workdir_b64}"))
}

fn decode_agent_handle(handle: &AgentHandle) -> Option<(String, String)> {
    let rest = handle.0.strip_prefix("aca:")?;
    let (sandbox_id, workdir_b64) = rest.split_once(':')?;
    let raw = base64::engine::general_purpose::STANDARD
        .decode(workdir_b64)
        .ok()?;
    let workdir = String::from_utf8(raw).ok()?;
    Some((sandbox_id.to_owned(), workdir))
}

#[async_trait]
impl GatewayWorkload for AcaGatewayWorkload {
    async fn spawn_agent(&self, req: &AgentSpawnRequest) -> Result<AgentHandle, GatewayError> {
        let sandbox_id = self.resolve_sandbox_id(req).await?;
        let cmd = build_agent_command(req, &self.relay, &self.bins, &self.relay_auth_snippet);
        let result = self
            .provider
            .exec_shell(&sandbox_id, &cmd)
            .await
            .map_err(|err| {
                tracing::warn!(error = %err, "aca gateway workload agent exec failed");
                GatewayError::ProviderAllocationFailed
            })?;
        tracing::info!(
            exit_code = result.exit_code,
            stdout_len = result.stdout.len(),
            stderr_len = result.stderr.len(),
            "aca gateway workload agent exec completed"
        );
        let workdir = parse_workdir(&result.stdout).ok_or_else(|| {
            tracing::warn!(
                exit_code = result.exit_code,
                stdout_len = result.stdout.len(),
                stderr_len = result.stderr.len(),
                "aca gateway workload agent did not report a workdir"
            );
            GatewayError::ProviderAllocationFailed
        })?;
        if result.exit_code != 0 {
            let _ = self
                .provider
                .exec_shell(&sandbox_id, &build_cleanup_command(&workdir))
                .await;
            tracing::warn!(
                exit_code = result.exit_code,
                "aca gateway workload agent setup exited nonzero"
            );
            return Err(GatewayError::ProviderAllocationFailed);
        }
        Ok(encode_agent_handle(&sandbox_id, &workdir))
    }

    async fn cleanup(&self, handle: &AgentHandle) -> Result<(), GatewayError> {
        let (sandbox_id, workdir) =
            decode_agent_handle(handle).ok_or(GatewayError::ProviderAllocationFailed)?;
        let cmd = build_cleanup_command(&workdir);
        // Best-effort: a cleanup failure must not wedge teardown.
        let _ = self.provider.exec_shell(&sandbox_id, &cmd).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_gateway::SECRET_LEN;
    use d2b_gateway::{
        AppCommand, DisplaySessionContext, DisplaySessionId, SessionBinding, SessionSecret,
    };
    use d2b_realm_core::{OperationId, PrincipalId, RealmId, RealmPath, WorkloadId};

    fn req(app: Vec<&str>) -> AgentSpawnRequest {
        let realm = RealmPath::new(vec![RealmId::parse("work").unwrap()]).unwrap();
        let binding = SessionBinding::new(
            &realm,
            7,
            &DisplaySessionId::new("sess-42"),
            0,
            &OperationId::parse("op-9").unwrap(),
            &PrincipalId::parse("alice").unwrap(),
            &WorkloadId::parse("demo").unwrap(),
            1_000_000,
        );
        AgentSpawnRequest {
            ctx: DisplaySessionContext {
                session_id: DisplaySessionId::new("sess-42"),
                operation_id: OperationId::parse("op-9").unwrap(),
                realm,
                generation: 7,
                peer_principal: PrincipalId::parse("alice").unwrap(),
            },
            binding,
            secret: SessionSecret::from_bytes([0xABu8; SECRET_LEN]),
            app: AppCommand::new(app.into_iter().map(String::from).collect()).unwrap(),
        }
    }

    fn coords() -> RelayCoords {
        RelayCoords {
            namespace: "relns-test.servicebus.windows.net".into(),
            entity: "hc-d2b-display".into(),
            ca_file: Some("/etc/ssl/certs/adc-egress-proxy-ca.crt".into()),
            managed_identity_client_id: None,
        }
    }

    #[test]
    fn command_uses_only_the_reserved_provider_agent() {
        let cmd = build_agent_command(
            &req(vec!["foot"]),
            &coords(),
            &AgentBinaries::default(),
            "D2B_RELAY_ENTRA_TOKEN=tok",
        );
        assert_eq!(cmd, "set -e\nexec 'd2b-provider-agent'\n");
        assert!(!cmd.contains("D2B_SESSION"));
        assert!(!cmd.contains("D2B_RELAY"));
        assert!(!cmd.contains("waypipe"));
        assert!(!cmd.contains("gateway-relay"));
    }

    #[test]
    fn cleanup_kills_pids_and_removes_workdir() {
        let c = build_cleanup_command("/tmp/d2b-disp.AbC123");
        assert!(c.contains("W='/tmp/d2b-disp.AbC123'"));
        assert!(c.contains("kill \"$p\""));
        assert!(c.contains("rm -rf \"$W\""));
    }

    #[test]
    fn parse_workdir_takes_the_last_marker() {
        let out = "noise\nD2B_AGENT_WORKDIR=/tmp/d2b-disp.X\nmore noise\nD2B_AGENT_WORKDIR=/tmp/d2b-disp.Y\n";
        assert_eq!(parse_workdir(out).as_deref(), Some("/tmp/d2b-disp.Y"));
        assert_eq!(parse_workdir("nothing here"), None);
    }

    #[test]
    fn agent_handle_carries_sandbox_id_and_workdir() {
        let handle =
            encode_agent_handle("7d9de4d2-f953-4a4c-84ae-e90bf208f9cf", "/tmp/d2b-disp.abc");
        assert_eq!(
            decode_agent_handle(&handle),
            Some((
                "7d9de4d2-f953-4a4c-84ae-e90bf208f9cf".to_owned(),
                "/tmp/d2b-disp.abc".to_owned()
            ))
        );
    }

    #[tokio::test]
    async fn invalid_agent_handle_fails_cleanup_closed() {
        use azure_core::credentials::{AccessToken, TokenRequestOptions};
        use d2b_provider_aca::{AcaConfig, AcaWorkloadProvider, HttpResponse, HttpTransport};
        use std::sync::Arc;
        use std::time::SystemTime;

        #[derive(Debug)]
        struct FakeCredential;
        #[async_trait]
        impl azure_core::credentials::TokenCredential for FakeCredential {
            async fn get_token(
                &self,
                _scopes: &[&str],
                _options: Option<TokenRequestOptions<'_>>,
            ) -> azure_core::Result<AccessToken> {
                let expires_on = (SystemTime::now() + std::time::Duration::from_secs(3600)).into();
                Ok(AccessToken::new("fake-token", expires_on))
            }
        }

        #[derive(Debug)]
        struct NeverHttp;
        #[async_trait]
        impl HttpTransport for NeverHttp {
            async fn request(
                &self,
                _method: d2b_provider_aca::HttpMethod,
                _url: &str,
                _bearer: &str,
                _body: Option<String>,
            ) -> d2b_realm_provider::error::ProviderResult<HttpResponse> {
                panic!("invalid handle cleanup must not call provider")
            }
        }

        let provider = AcaWorkloadProvider::with_parts(
            AcaConfig {
                subscription: "sub".into(),
                resource_group: "rg".into(),
                sandbox_group: "sg".into(),
                region: "centralus".into(),
                endpoint: None,
                managed_identity_client_id: None,
            },
            d2b_realm_core::NodeId::parse("gateway").unwrap(),
            Arc::new(FakeCredential),
            Arc::new(NeverHttp),
        );
        let workload = AcaGatewayWorkload::new(provider, "sandbox-1", coords());
        assert_eq!(
            workload
                .cleanup(&AgentHandle("not-an-aca-handle".to_owned()))
                .await
                .unwrap_err(),
            GatewayError::ProviderAllocationFailed
        );
    }
}
