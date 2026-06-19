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
//!   3. fetches the sandbox Managed-Identity token for `relay.azure.net`
//!      (no SAS handed to the container) into `NIXLING_RELAY_ENTRA_TOKEN`;
//!   4. starts a persistent `waypipe server` exposing `WAYLAND_DISPLAY` and the
//!      gated `nixling-gateway-relay sender` (which writes the handshake
//!      prologue), then launches the requested app against that display.
//!
//! The command-shaping is a pure function ([`build_agent_command`]) so it is
//! unit-tested without any Azure round-trip; the I/O is a single delegated
//! `exec_shell` call.

use async_trait::async_trait;
use base64::Engine;
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

/// The shell expression that must set `NIXLING_RELAY_ENTRA_TOKEN` to a bearer
/// token for `https://relay.azure.net` (so the container authenticates to the
/// relay with its Managed Identity, never a handed SAS key). The default uses
/// the ACA-injected `IDENTITY_ENDPOINT`/`IDENTITY_HEADER` MSI endpoint.
pub fn default_entra_token_snippet() -> String {
    // The ACA minimal image lacks grep/sed; `nixling-gateway-relay` reads the
    // token from env, so we extract it with a here-doc-free, tool-light
    // parse: curl the MSI endpoint and let the relay binary's own `--msi`
    // helper would normally parse it, but to keep the agent self-contained we
    // rely on the image's baked `nl-msi-token` helper (provisioned in
    // image.nix) which prints the bare access_token for a resource.
    "NIXLING_RELAY_ENTRA_TOKEN=\"$(nl-msi-token https://relay.azure.net)\"".to_owned()
}

/// The production [`GatewayWorkload`]: drives the in-sandbox agent over ACA
/// `executeShellCommand`.
pub struct AcaGatewayWorkload {
    provider: AcaWorkloadProvider,
    sandbox_id: String,
    relay: RelayCoords,
    bins: AgentBinaries,
    entra_token_snippet: String,
}

impl AcaGatewayWorkload {
    /// Build the adapter for one sandbox.
    pub fn new(
        provider: AcaWorkloadProvider,
        sandbox_id: impl Into<String>,
        relay: RelayCoords,
    ) -> Self {
        Self {
            provider,
            sandbox_id: sandbox_id.into(),
            relay,
            bins: AgentBinaries::default(),
            entra_token_snippet: default_entra_token_snippet(),
        }
    }

    /// Override the in-image binary names / compression / display.
    pub fn with_binaries(mut self, bins: AgentBinaries) -> Self {
        self.bins = bins;
        self
    }

    /// Override the MI-token-fetch shell snippet (e.g. for a SAS test rig).
    pub fn with_entra_token_snippet(mut self, snippet: impl Into<String>) -> Self {
        self.entra_token_snippet = snippet.into();
        self
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
    entra_token_snippet: &str,
) -> String {
    let b: &SessionBinding = &req.binding;
    let secret_b64 = base64::engine::general_purpose::STANDARD.encode(req.secret.expose());

    let mut s = String::new();
    s.push_str("set -e\n");
    s.push_str("W=$(mktemp -d /tmp/nl-disp.XXXXXX)\n");
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
    // Container -> Azure auth (Managed Identity, no SAS).
    s.push_str(entra_token_snippet);
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
        "NIXLING_RELAY_NAMESPACE",
        "NIXLING_RELAY_ENTITY",
        "NIXLING_RELAY_TARGET",
    ];
    if let Some(ca) = &relay.ca_file {
        s.push_str(&format!("NIXLING_RELAY_CA_FILE={}\n", sh_quote(ca)));
        relay_exports.push("NIXLING_RELAY_CA_FILE");
    }
    // Persistent waypipe server (multiplexes N clients on one display).
    s.push_str(&format!(
        "( {wp} --no-gpu -c {comp} -s \"$W/wp.sock\" --display {disp} server -- sleep infinity >\"$W/wp.log\" 2>&1 & echo $! >> \"$W/pids\" )\n",
        wp = sh_quote(&bins.waypipe),
        comp = bins.compression,
        disp = bins.display,
    ));
    // Gated relay sender: writes the handshake prologue, then bridges wp.sock.
    s.push_str(&format!(
        "( export {relay_exports}; {relay_bin} sender >\"$W/relay.log\" 2>&1 & echo $! >> \"$W/pids\" )\n",
        relay_exports = relay_exports.join(" "),
        relay_bin = sh_quote(&bins.gateway_relay),
    ));
    // Wait (bounded) for the display socket, then launch the app against it.
    s.push_str(&format!(
        "for _ in $(seq 1 50); do [ -S \"$W/{disp}\" ] && break; sleep 0.1; done\n",
        disp = bins.display,
    ));
    let app_argv: String = req
        .app
        .argv()
        .iter()
        .map(|a| sh_quote(a))
        .collect::<Vec<_>>()
        .join(" ");
    s.push_str(&format!(
        "( WAYLAND_DISPLAY={disp} {argv} >\"$W/app.log\" 2>&1 & echo $! >> \"$W/pids\" )\n",
        disp = bins.display,
        argv = app_argv,
    ));
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

#[async_trait]
impl GatewayWorkload for AcaGatewayWorkload {
    async fn spawn_agent(&self, req: &AgentSpawnRequest) -> Result<AgentHandle, GatewayError> {
        let cmd = build_agent_command(req, &self.relay, &self.bins, &self.entra_token_snippet);
        let result = self
            .provider
            .exec_shell(&self.sandbox_id, &cmd)
            .await
            .map_err(|_| GatewayError::ProviderAllocationFailed)?;
        let workdir =
            parse_workdir(&result.stdout).ok_or(GatewayError::ProviderAllocationFailed)?;
        Ok(AgentHandle(workdir))
    }

    async fn cleanup(&self, handle: &AgentHandle) -> Result<(), GatewayError> {
        let cmd = build_cleanup_command(&handle.0);
        // Best-effort: a cleanup failure must not wedge teardown.
        let _ = self.provider.exec_shell(&self.sandbox_id, &cmd).await;
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
        // The gated sender + persistent waypipe server + the app are launched.
        assert!(cmd.contains("export NL_SESSION_SECRET_B64"));
        assert!(cmd.contains("'nixling-gateway-relay' sender"));
        assert!(cmd.contains("--display wayland-nl server -- sleep infinity"));
        assert!(cmd.contains("WAYLAND_DISPLAY=wayland-nl 'foot'"));
        // Relay coords.
        assert!(cmd.contains("NIXLING_RELAY_ENTITY='hc-nixling-display'"));
        assert!(cmd.contains("NIXLING_RELAY_TARGET=\"unix-listen:$W/wp.sock\""));
        // Workdir handle is the last line.
        assert!(cmd.trim_end().ends_with("echo \"NL_AGENT_WORKDIR=$W\""));
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
            .find(|l| l.contains("WAYLAND_DISPLAY=wayland-nl 'foot'"))
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
}
