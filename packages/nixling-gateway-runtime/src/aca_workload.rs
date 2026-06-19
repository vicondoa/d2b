//! `AcaGatewayWorkload` — the production [`GatewayWorkload`] adapter that drives
//! the in-sandbox display agent through the Azure Container Apps data plane
//! (ADR 0032, P0).
//!
//! The orchestrator hands this adapter an [`AgentSpawnRequest`] carrying the
//! session binding + the one-shot secret `S`; the adapter shapes a single
//! `executeShellCommand` body (delivered over the MI-authenticated, TLS ACA
//! control plane — never the relay, never a persistent log) that:
//!
//!   1. receives `S` in the exec body, writes it to a tmpfile, reads it into
//!      `NL_SESSION_SECRET_B64`, and shreds the file (so `S` is never a
//!      long-lived process argv);
//!   2. exports the binding fields as `NL_SESSION_*`;
//!   3. installs relay sender auth: either a gateway-minted short-lived Send
//!      bearer in `NIXLING_RELAY_SAS_TOKEN` (P0's live path) or an ACA
//!      Managed-Identity token in `NIXLING_RELAY_ENTRA_TOKEN`;
//!   4. starts the gated `nixling-gateway-relay sender` (which writes the
//!      handshake prologue), then launches the requested app as the child of
//!      `waypipe server`.
//!
//! The command-shaping is a pure function ([`build_agent_command`]) so it is
//! unit-tested without any Azure round-trip; the I/O is a single delegated
//! `exec_shell` call.

use async_trait::async_trait;
use base64::Engine;
use nixling_constellation_core::WorkloadId;
use nixling_gateway::{
    AgentHandle, AgentSpawnRequest, GatewayError, GatewayWorkload, SessionBinding,
};
use nixling_provider_aca::AcaWorkloadProvider;

/// Azure Relay coordinates the in-sandbox sender dials out to.
#[derive(Debug, Clone)]
pub struct RelayCoords {
    /// Namespace FQDN, e.g. `relns-xxxx.servicebus.windows.net`.
    pub namespace: String,
    /// Hybrid-connection (entity) name, e.g. `hc-nixling-display`.
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
    /// The gated relay sender (this crate's `nixling-gateway-relay`).
    pub gateway_relay: String,
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
            gateway_relay: "nixling-gateway-relay".to_owned(),
            waypipe: "waypipe".to_owned(),
            compression: "zstd".to_owned(),
            display: "wayland-nl".to_owned(),
        }
    }
}

/// The shell expression that sets `NIXLING_RELAY_ENTRA_TOKEN` to a bearer token
/// for `https://relay.azure.net`. The default uses the ACA-injected
/// `IDENTITY_ENDPOINT`/`IDENTITY_HEADER` MSI endpoint; production P0 can
/// override this with [`relay_sas_token_snippet`] to avoid the observed ACA
/// Entra Relay substream close while still never handing the SAS rule key to
/// the container.
pub fn default_entra_token_snippet() -> String {
    // The ACA minimal image lacks grep/sed; `nixling-gateway-relay` reads the
    // token from env, so we extract it with a here-doc-free, tool-light
    // parse: curl the MSI endpoint and let the relay binary's own `--msi`
    // helper would normally parse it, but to keep the agent self-contained we
    // rely on the image's baked `nl-msi-token` helper (provisioned in
    // image.nix) which prints the bare access_token for a resource.
    "NIXLING_RELAY_ENTRA_TOKEN=\"$(nl-msi-token https://relay.azure.net/ \"${NIXLING_MI_CLIENT_ID:-}\")\"".to_owned()
}

/// Build a relay-auth snippet from a gateway-minted short-lived SAS bearer.
/// This passes only the bearer token to the sandbox, never the authorization
/// rule key; the per-session handshake still gates display-byte admission.
pub fn relay_sas_token_snippet(token: &str) -> String {
    format!("NIXLING_RELAY_SAS_TOKEN={}", sh_quote(token))
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

/// Build the `executeShellCommand` body for an [`AgentSpawnRequest`]. Pure: the
/// session secret is base64-encoded into the (TLS, MI-authed) exec body and
/// shredded from the in-sandbox filesystem after it is read into env. The
/// returned script prints `NL_AGENT_WORKDIR=<dir>` as its last line so the
/// caller can derive a durable cleanup handle.
pub fn build_agent_command(
    req: &AgentSpawnRequest,
    relay: &RelayCoords,
    bins: &AgentBinaries,
    relay_auth_snippet: &str,
) -> String {
    let b: &SessionBinding = &req.binding;
    let secret_b64 = base64::engine::general_purpose::STANDARD.encode(req.secret.expose());

    let mut s = String::new();
    s.push_str("set -e\n");
    s.push_str("W=$(mktemp -d /tmp/nl-disp.XXXXXX)\n");
    s.push_str("echo \"NL_AGENT_WORKDIR=$W\"\n");
    s.push_str("export XDG_RUNTIME_DIR=\"$W\"\n");
    // Secret: arrives in the exec body, written to a file, read into env, then
    // shredded so it is never a long-lived process argv.
    s.push_str(&format!(
        "printf '%s' {} > \"$W/s\"\n",
        sh_quote(&secret_b64)
    ));
    s.push_str("NL_SESSION_SECRET_B64=\"$(cat \"$W/s\")\"; rm -f \"$W/s\"\n");
    // Binding fields (non-secret identifiers).
    for (k, v) in [
        ("NL_SESSION_REALM", b.realm.as_str()),
        ("NL_SESSION_GENERATION", &b.generation.to_string()),
        ("NL_SESSION_ID", b.session_id.as_str()),
        ("NL_SESSION_EPOCH", &b.epoch.to_string()),
        ("NL_SESSION_OP", b.operation_id.as_str()),
        ("NL_SESSION_PRINCIPAL", b.principal.as_str()),
        ("NL_SESSION_WORKLOAD", b.workload.as_str()),
        ("NL_SESSION_NOT_AFTER", &b.not_after.to_string()),
    ] {
        s.push_str(&format!("{k}={}\n", sh_quote(v)));
    }
    // Relay sender auth. The default uses container MI; production P0 can
    // override this with a gateway-minted short-lived Send bearer.
    if let Some(client_id) = relay
        .managed_identity_client_id
        .as_ref()
        .filter(|client_id| !client_id.trim().is_empty())
    {
        s.push_str(&format!("NIXLING_MI_CLIENT_ID={}\n", sh_quote(client_id)));
    }
    s.push_str(relay_auth_snippet);
    s.push('\n');
    // Relay coordinates for the gated sender.
    s.push_str(&format!(
        "NIXLING_RELAY_NAMESPACE={}\n",
        sh_quote(&relay.namespace)
    ));
    s.push_str(&format!(
        "NIXLING_RELAY_ENTITY={}\n",
        sh_quote(&relay.entity)
    ));
    s.push_str("NIXLING_RELAY_TARGET=\"unix-listen:$W/wp.sock\"\n");
    let mut relay_exports = vec![
        "NL_SESSION_SECRET_B64",
        "NL_SESSION_REALM",
        "NL_SESSION_GENERATION",
        "NL_SESSION_ID",
        "NL_SESSION_EPOCH",
        "NL_SESSION_OP",
        "NL_SESSION_PRINCIPAL",
        "NL_SESSION_WORKLOAD",
        "NL_SESSION_NOT_AFTER",
        "NIXLING_RELAY_ENTRA_TOKEN",
        "NIXLING_RELAY_SAS_TOKEN",
        "NIXLING_RELAY_NAMESPACE",
        "NIXLING_RELAY_ENTITY",
        "NIXLING_RELAY_TARGET",
    ];
    if let Some(ca) = &relay.ca_file {
        s.push_str(&format!("NIXLING_RELAY_CA_FILE={}\n", sh_quote(ca)));
        relay_exports.push("NIXLING_RELAY_CA_FILE");
    }
    // Gated relay sender: binds wp.sock, writes the handshake prologue to the
    // relay, then bridges the Waypipe server connection.
    s.push_str(&format!(
        "( export {relay_exports}; nohup {relay_bin} sender >\"$W/relay.log\" 2>&1 < /dev/null & echo $! >> \"$W/pids\" )\n",
        relay_exports = relay_exports.join(" "),
        relay_bin = sh_quote(&bins.gateway_relay),
    ));
    s.push_str("for _ in $(seq 1 300); do [ -S \"$W/wp.sock\" ] && break; sleep 0.1; done\n");
    s.push_str(
        "[ -S \"$W/wp.sock\" ] || { echo \"relay sender did not become ready\"; exit 1; }\n",
    );
    let app_argv: String = req
        .app
        .argv()
        .iter()
        .map(|a| sh_quote(a))
        .collect::<Vec<_>>()
        .join(" ");
    // Launch the app as Waypipe's child. This is the proven ACA path: Waypipe
    // owns the compositor socket lifecycle for the app instead of racing a
    // separately-started client against a persistent server.
    s.push_str(&format!(
        "( nohup {wp} --no-gpu -c {comp} -s \"$W/wp.sock\" server -- {argv} >\"$W/app.log\" 2>&1 < /dev/null & echo $! >> \"$W/pids\" )\n",
        wp = sh_quote(&bins.waypipe),
        comp = bins.compression,
        argv = app_argv,
    ));
    s.push_str("sleep 3\n");
    s.push_str("echo \"NL_AGENT_WORKDIR=$W\"\n");
    s
}

/// Build the idempotent cleanup command for a spawned agent: kill the recorded
/// pids and remove the workdir.
pub fn build_cleanup_command(workdir: &str) -> String {
    format!(
        "W={w}\n\
[ -f \"$W/pids\" ] && while read p; do kill \"$p\" 2>/dev/null || true; done < \"$W/pids\"\n\
rm -rf \"$W\"\n\
echo NL_AGENT_CLEANED\n",
        w = sh_quote(workdir),
    )
}

/// Extract the durable workdir handle from the agent script's stdout.
fn parse_workdir(stdout: &str) -> Option<String> {
    stdout.lines().rev().find_map(|l| {
        l.trim()
            .strip_prefix("NL_AGENT_WORKDIR=")
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
    use nixling_constellation_core::{OperationId, PrincipalId, RealmId, RealmPath, WorkloadId};
    use nixling_gateway::SECRET_LEN;
    use nixling_gateway::{AppCommand, DisplaySessionContext, DisplaySessionId, SessionSecret};

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
            entity: "hc-nixling-display".into(),
            ca_file: Some("/etc/ssl/certs/adc-egress-proxy-ca.crt".into()),
            managed_identity_client_id: None,
        }
    }

    #[test]
    fn command_delivers_binding_and_launches_the_gated_stack() {
        let cmd = build_agent_command(
            &req(vec!["foot"]),
            &coords(),
            &AgentBinaries::default(),
            "NIXLING_RELAY_ENTRA_TOKEN=tok",
        );
        // Binding fields are shell variables; only the relay subshell exports
        // them, so the app does not inherit control-plane metadata/secrets.
        assert!(cmd.contains("NL_SESSION_REALM='work'"));
        assert!(cmd.contains("NL_SESSION_GENERATION='7'"));
        assert!(cmd.contains("NL_SESSION_ID='sess-42'"));
        assert!(cmd.contains("NL_SESSION_OP='op-9'"));
        assert!(cmd.contains("NL_SESSION_WORKLOAD='demo'"));
        assert!(cmd.contains("NL_SESSION_NOT_AFTER='1000000'"));
        // The gated sender + waypipe-owned app launch are emitted.
        assert!(cmd.contains("export NL_SESSION_SECRET_B64"));
        assert!(cmd.contains("nohup 'nixling-gateway-relay' sender"));
        assert!(
            cmd.contains("nohup 'waypipe' --no-gpu -c zstd -s \"$W/wp.sock\" server -- 'foot'")
        );
        // Relay coords.
        assert!(cmd.contains("NIXLING_RELAY_ENTITY='hc-nixling-display'"));
        assert!(cmd.contains("NIXLING_RELAY_TARGET=\"unix-listen:$W/wp.sock\""));
        // Workdir handle is the last line.
        assert!(cmd.trim_end().ends_with("echo \"NL_AGENT_WORKDIR=$W\""));
    }

    #[test]
    fn command_can_pass_managed_identity_client_id_to_token_helper() {
        let mut relay = coords();
        relay.managed_identity_client_id = Some("b3ad7d90-e6d5-4d12-84e9-c9ef77b80b02".to_owned());
        let cmd = build_agent_command(
            &req(vec!["foot"]),
            &relay,
            &AgentBinaries::default(),
            &default_entra_token_snippet(),
        );
        assert!(cmd.contains("NIXLING_MI_CLIENT_ID='b3ad7d90-e6d5-4d12-84e9-c9ef77b80b02'"));
        assert!(cmd.contains(
            "NIXLING_RELAY_ENTRA_TOKEN=\"$(nl-msi-token https://relay.azure.net/ \"${NIXLING_MI_CLIENT_ID:-}\")\""
        ));
        let app_line = cmd
            .lines()
            .find(|l| l.contains("server -- 'foot'"))
            .unwrap();
        assert!(!app_line.contains("NIXLING_MI_CLIENT_ID"));
    }

    #[test]
    fn command_accepts_gateway_minted_sas_token_without_key_material() {
        let cmd = build_agent_command(
            &req(vec!["foot"]),
            &coords(),
            &AgentBinaries::default(),
            &relay_sas_token_snippet("SharedAccessSignature sr=x&sig=y"),
        );
        assert!(cmd.contains("NIXLING_RELAY_SAS_TOKEN='SharedAccessSignature sr=x&sig=y'"));
        assert!(cmd.contains("export NL_SESSION_SECRET_B64"));
        assert!(cmd.contains("NIXLING_RELAY_SAS_TOKEN"));
        assert!(!cmd.contains("NIXLING_RELAY_KEY="));
        assert!(!cmd.contains("NIXLING_RELAY_KEY_NAME="));
    }

    #[test]
    fn secret_is_filed_and_shredded_never_a_bare_export() {
        let cmd = build_agent_command(&req(vec!["foot"]), &coords(), &AgentBinaries::default(), "");
        // The raw base64 secret is delivered via a file write, then shredded.
        let b64 = base64::engine::general_purpose::STANDARD.encode([0xABu8; SECRET_LEN]);
        assert!(cmd.contains(&format!("printf '%s' '{b64}' > \"$W/s\"")));
        assert!(cmd.contains("rm -f \"$W/s\""));
        // It is never placed directly on an `export NL_SESSION_SECRET_B64=<b64>`.
        assert!(!cmd.contains(&format!("export NL_SESSION_SECRET_B64={b64}")));
        assert!(!cmd.contains(&format!("NL_SESSION_SECRET_B64='{b64}'")));
    }

    #[test]
    fn app_launch_does_not_inherit_relay_or_session_secrets() {
        let cmd = build_agent_command(
            &req(vec!["foot"]),
            &coords(),
            &AgentBinaries::default(),
            "NIXLING_RELAY_ENTRA_TOKEN=tok",
        );
        let app_line = cmd
            .lines()
            .find(|l| l.contains("server -- 'foot'"))
            .unwrap();
        assert!(!app_line.contains("NL_SESSION_SECRET_B64"));
        assert!(!app_line.contains("NIXLING_RELAY_ENTRA_TOKEN"));
        assert!(!app_line.contains("NIXLING_RELAY_KEY"));
    }

    #[test]
    fn app_argv_is_shell_quoted_against_injection() {
        let cmd = build_agent_command(
            &req(vec!["foot", "--title=a'b; rm -rf /"]),
            &coords(),
            &AgentBinaries::default(),
            "",
        );
        // The malicious arg is fully single-quoted; the `;` cannot start a new
        // command.
        assert!(cmd.contains("'--title=a'\\''b; rm -rf /'"));
        assert!(!cmd.contains("a'b; rm -rf /'\n"));
    }

    #[test]
    fn cleanup_kills_pids_and_removes_workdir() {
        let c = build_cleanup_command("/tmp/nl-disp.AbC123");
        assert!(c.contains("W='/tmp/nl-disp.AbC123'"));
        assert!(c.contains("kill \"$p\""));
        assert!(c.contains("rm -rf \"$W\""));
    }

    #[test]
    fn parse_workdir_takes_the_last_marker() {
        let out =
            "noise\nNL_AGENT_WORKDIR=/tmp/nl-disp.X\nmore noise\nNL_AGENT_WORKDIR=/tmp/nl-disp.Y\n";
        assert_eq!(parse_workdir(out).as_deref(), Some("/tmp/nl-disp.Y"));
        assert_eq!(parse_workdir("nothing here"), None);
    }

    #[test]
    fn agent_handle_carries_sandbox_id_and_workdir() {
        let handle =
            encode_agent_handle("7d9de4d2-f953-4a4c-84ae-e90bf208f9cf", "/tmp/nl-disp.abc");
        assert_eq!(
            decode_agent_handle(&handle),
            Some((
                "7d9de4d2-f953-4a4c-84ae-e90bf208f9cf".to_owned(),
                "/tmp/nl-disp.abc".to_owned()
            ))
        );
    }

    #[tokio::test]
    async fn invalid_agent_handle_fails_cleanup_closed() {
        use azure_core::credentials::{AccessToken, TokenRequestOptions};
        use nixling_provider_aca::{AcaConfig, AcaWorkloadProvider, HttpResponse, HttpTransport};
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
                _method: nixling_provider_aca::HttpMethod,
                _url: &str,
                _bearer: &str,
                _body: Option<String>,
            ) -> nixling_constellation_provider::error::ProviderResult<HttpResponse> {
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
            },
            nixling_constellation_core::NodeId::parse("gateway").unwrap(),
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
