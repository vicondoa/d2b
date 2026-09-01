//! The v3 command registry and command-to-surface dispatch.

use std::{
    collections::BTreeSet,
    ffi::OsString,
    fmt::Write as _,
    io::{self, Write as _},
    path::{Path, PathBuf},
};

use crate::context::{
    CliContext, ErrorFrame, OutputMode, SeqpacketUnixSocket, ZoneContext,
    cli_failure_from_daemon_error, daemon_hello_frame, decode_daemon_frame, is_daemon_unreachable,
    output_mode, parse_hello_reply,
};
use crate::{
    CliFailure, activation, complete, endpoint, exec, guest, host, print_json, print_stdout,
    provider, resource, share, shell, zone,
};
use clap::{Args, Parser, Subcommand};
use d2b_contracts_broker::broker_wire::AuditExportCursor;
use d2b_contracts_control::{
    cli_output::{AuthDeniedSubcommandV2, AuthRoleV2, AuthSocketStatusV2, AuthStatusOutputV2},
    public_wire::{self, AuditFormat as IpcAuditFormat, AuditRequest as IpcAuditRequest},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The one built-in top-level command registry.
///
/// Provider projection binding and completion consume this list as well as
/// clap's parser, so collision handling never depends on a second list.
pub(crate) const BUILTIN_COMMANDS: &[&str] = &[
    "get",
    "list",
    "watch",
    "create",
    "update-spec",
    "delete",
    "status",
    "upgrade",
    "reconcile",
    "host",
    "guest",
    "process",
    "exec",
    "shell",
    "volume",
    "network",
    "device",
    "endpoint",
    "export",
    "import",
    "resource",
    "user",
    "credential",
    "provider",
    "zone",
    "quota",
    "emergency-policy",
    "activation",
    "audit",
    "op",
    "auth",
    "complete",
];

/// The clean-break CLI parser is the sole runtime entry point.
#[derive(Debug, Parser)]
#[command(
    name = "d2b",
    version,
    about = "Typed Zone resource client for d2b.",
    disable_help_subcommand = true
)]
pub(crate) struct ModernCli {
    /// Address a declared Zone. Without this flag the nearest local runtime is
    /// selected.
    #[arg(long, global = true, value_name = "ZONE")]
    pub(crate) zone: Option<String>,
    /// Emit the stable JSON envelope.
    #[arg(long, global = true, conflicts_with = "human")]
    pub(crate) json: bool,
    /// Force human-readable terminal output.
    #[arg(long, global = true, conflicts_with = "json")]
    pub(crate) human: bool,
    /// Bound all Zone requests and streams.
    #[arg(long, global = true, value_name = "DURATION")]
    pub(crate) deadline: Option<String>,
    /// Suppress the command default deadline.
    #[arg(long, global = true, conflicts_with = "deadline")]
    pub(crate) no_deadline: bool,
    #[command(subcommand)]
    pub(crate) command: ModernCommand,
}

#[derive(Debug, Subcommand)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum ModernCommand {
    Get(GenericGetArgs),
    List(GenericListArgs),
    Watch(GenericWatchArgs),
    Create(GenericCreateArgs),
    #[command(name = "update-spec")]
    UpdateSpec(GenericUpdateSpecArgs),
    Delete(GenericDeleteArgs),
    Status(GenericStatusArgs),
    Upgrade(GenericUpgradeArgs),
    Reconcile(GenericReconcileArgs),
    Host(host::HostArgs),
    Guest(guest::GuestArgs),
    Process(guest::ProcessArgs),
    Exec(exec::ExecArgs),
    Shell(shell::ShellArgs),
    Volume(resource::TypedResourceArgs),
    Network(resource::TypedResourceArgs),
    Device(resource::TypedResourceArgs),
    Endpoint(endpoint::EndpointArgs),
    Export(share::ExportArgs),
    Import(share::ImportArgs),
    Resource(resource::ResourceArgs),
    User(resource::TypedResourceArgs),
    Credential(resource::TypedResourceArgs),
    Provider(provider::ProviderArgs),
    Zone(zone::ZoneArgs),
    Quota(resource::TypedResourceArgs),
    #[command(name = "emergency-policy")]
    EmergencyPolicy(resource::TypedResourceArgs),
    Activation(activation::ActivationArgs),
    Audit(GenericAuditArgs),
    Op(GenericOpArgs),
    Auth(GenericAuthArgs),
    Complete(complete::CompleteArgs),
    Audio(ProjectionCommandArgs),
    Clipboard(ProjectionCommandArgs),
    Display(ProjectionCommandArgs),
}

#[derive(Debug, Args, Clone)]
pub(crate) struct GenericGetArgs {
    pub(crate) resource_ref: String,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct GenericListArgs {
    pub(crate) resource_type: String,
    #[arg(long = "execution-ref")]
    pub(crate) execution_ref: Option<String>,
    #[arg(long)]
    pub(crate) domain: Option<String>,
    #[arg(long)]
    pub(crate) phase: Option<String>,
    #[arg(long = "label-selector")]
    pub(crate) label_selector: Option<String>,
    #[arg(long)]
    pub(crate) updates: bool,
    #[arg(long = "page-token")]
    pub(crate) page_token: Option<String>,
    #[arg(long)]
    pub(crate) limit: Option<u32>,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct GenericWatchArgs {
    pub(crate) resource_type: String,
    #[arg(long = "since-revision")]
    pub(crate) since_revision: Option<String>,
    #[arg(long)]
    pub(crate) phase: Option<String>,
    #[arg(long = "label-selector")]
    pub(crate) label_selector: Option<String>,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct GenericCreateArgs {
    pub(crate) resource_type: String,
    #[arg(long = "spec-file", conflicts_with = "spec_stdin")]
    pub(crate) spec_file: Option<PathBuf>,
    #[arg(long = "spec-stdin", conflicts_with = "spec_file")]
    pub(crate) spec_stdin: bool,
    #[arg(long = "wait-for-reconcile")]
    pub(crate) wait_for_reconcile: bool,
    #[arg(long = "reconcile-deadline")]
    pub(crate) reconcile_deadline: Option<String>,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct GenericUpdateSpecArgs {
    pub(crate) resource_ref: String,
    #[arg(long)]
    pub(crate) revision: Option<String>,
    #[arg(long = "spec-file", conflicts_with = "spec_stdin")]
    pub(crate) spec_file: Option<PathBuf>,
    #[arg(long = "spec-stdin", conflicts_with = "spec_file")]
    pub(crate) spec_stdin: bool,
    #[arg(long = "wait-for-reconcile")]
    pub(crate) wait_for_reconcile: bool,
    #[arg(long = "reconcile-deadline")]
    pub(crate) reconcile_deadline: Option<String>,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct GenericDeleteArgs {
    pub(crate) resource_ref: String,
    #[arg(long)]
    pub(crate) revision: Option<String>,
    #[arg(long = "wait-for-reconcile")]
    pub(crate) wait_for_reconcile: bool,
    #[arg(long = "reconcile-deadline")]
    pub(crate) reconcile_deadline: Option<String>,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct GenericStatusArgs {
    pub(crate) resource_ref: String,
    #[arg(long)]
    pub(crate) watch: bool,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct GenericUpgradeArgs {
    pub(crate) resource_ref: String,
    #[arg(long)]
    pub(crate) recursive: bool,
    #[arg(long)]
    pub(crate) apply: bool,
    #[arg(long = "reconcile-deadline")]
    pub(crate) reconcile_deadline: Option<String>,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct GenericReconcileArgs {
    pub(crate) resource_ref: String,
    #[arg(long = "reconcile-deadline")]
    pub(crate) reconcile_deadline: Option<String>,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct GenericAuditArgs {
    #[arg(long)]
    pub(crate) strict: bool,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct GenericOpArgs {
    #[command(subcommand)]
    pub(crate) command: GenericOpCommand,
}

#[derive(Debug, Subcommand, Clone)]
pub(crate) enum GenericOpCommand {
    Inspect(GenericOpInspectArgs),
}

#[derive(Debug, Args, Clone)]
pub(crate) struct GenericOpInspectArgs {
    #[arg(long = "operation-id")]
    pub(crate) operation_id: Option<String>,
    #[arg(long)]
    pub(crate) trace_id: Option<String>,
    #[arg(long)]
    pub(crate) span_id: Option<String>,
    #[arg(long)]
    pub(crate) watch: bool,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct GenericAuthArgs {
    #[command(subcommand)]
    pub(crate) command: GenericAuthCommand,
    /// Test-only identity override retained as a hidden fixture seam.
    #[arg(long, hide = true)]
    pub(crate) test_uid: Option<u32>,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ProjectionCommandArgs {
    pub(crate) verb: String,
    #[arg(last = true, allow_hyphen_values = true)]
    pub(crate) args: Vec<String>,
}

/// Stable operator-facing error envelope shared by the CLI's local and
/// daemon-backed command surfaces.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct HostErrorEnvelope {
    pub(crate) kind: String,
    pub(crate) code: String,
    pub(crate) exit_code: i32,
    pub(crate) what_was_checked: String,
    pub(crate) observed_state: String,
    pub(crate) remediation: String,
    pub(crate) docs_anchor: String,
}

pub(crate) fn host_error_envelope(
    kind: &str,
    code: &str,
    exit_code: i32,
    what_was_checked: &str,
    observed_state: &str,
    remediation: &str,
    docs_anchor: &str,
) -> HostErrorEnvelope {
    HostErrorEnvelope {
        kind: kind.to_owned(),
        code: code.to_owned(),
        exit_code,
        what_was_checked: what_was_checked.to_owned(),
        observed_state: observed_state.to_owned(),
        remediation: remediation.to_owned(),
        docs_anchor: docs_anchor.to_owned(),
    }
}

pub(crate) fn emit_host_error(
    envelope: &HostErrorEnvelope,
    json_output: bool,
) -> Result<i32, CliFailure> {
    if json_output {
        let mut rendered = serde_json::to_string_pretty(envelope).map_err(|error| {
            CliFailure::new(
                1,
                format!("failed to serialize host error envelope: {error}"),
            )
        })?;
        rendered.push('\n');
        print_stdout(&rendered);
    } else {
        let _ = writeln!(
            io::stderr().lock(),
            "d2b: {} (code: {}, exit {})\n  what was checked : {}\n  observed         : {}\n  remediation      : {}\n  docs             : {}",
            envelope.kind,
            envelope.code,
            envelope.exit_code,
            envelope.what_was_checked,
            envelope.observed_state,
            envelope.remediation,
            envelope.docs_anchor,
        );
    }
    Ok(envelope.exit_code)
}

pub(crate) fn daemon_down_envelope(verb: &str) -> HostErrorEnvelope {
    host_error_envelope(
        &format!("d2b {verb} requires d2bd"),
        "daemon-down",
        1,
        "Daemon connectivity at /run/d2b/public.sock.",
        "d2bd is unreachable; the daemon is the only operator surface for mutating verbs.",
        "Start d2bd (systemctl start d2bd d2b-broker.socket) and re-run the same command. See docs/how-to/migrate-d2b-v1-0-to-v1-1.md#recovery-broker-bring-up-troubleshooting for the full bring-up checklist.",
        "docs/reference/error-codes.md#daemon-down",
    )
}

pub(crate) fn not_yet_implemented_envelope(verb: &str) -> HostErrorEnvelope {
    host_error_envelope(
        &format!("d2b {verb} has no daemon-native handler yet"),
        "not-yet-implemented",
        78,
        &format!("Native daemon dispatch for `d2b {verb}`"),
        "The daemon-native handler has not landed yet; the typed envelope contract is the only operator path until the native handler ships.",
        "Track the surface schedule in CHANGELOG.md \"Unreleased\"; the typed envelope is the only operator path until the native handler ships.",
        "docs/reference/error-codes.md#not-yet-implemented",
    )
}

pub(crate) fn missing_mutation_flag_envelope(verb: &str) -> HostErrorEnvelope {
    host_error_envelope(
        &format!("{verb} requires either --dry-run or --apply"),
        "--apply-or-dry-run-required",
        78,
        &format!("{verb} invocation flags."),
        "Neither --dry-run nor --apply was provided.",
        &format!("Re-run as `d2b {verb} --dry-run` to plan or `d2b {verb} --apply` to mutate."),
        "docs/reference/error-codes.md#--apply-or-dry-run-required",
    )
}

#[derive(Debug, Subcommand, Clone)]
pub(crate) enum GenericAuthCommand {
    Status,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AuditResponseFrame {
    #[serde(rename = "type")]
    _type_name: String,
    #[serde(flatten)]
    payload: public_wire::AuditResponse,
}

#[derive(Debug, Clone)]
pub(crate) enum AuditSocketOutcome {
    Unreachable,
    Lines(Vec<String>),
}

pub(crate) fn daemon_audit_frame(type_name: &str, json_mode: bool) -> Result<Vec<u8>, CliFailure> {
    daemon_audit_frame_with_cursor(type_name, json_mode, None)
}

fn daemon_audit_frame_with_cursor(
    type_name: &str,
    json_mode: bool,
    cursor: Option<AuditExportCursor>,
) -> Result<Vec<u8>, CliFailure> {
    let request = IpcAuditRequest {
        filter: None,
        format: if json_mode {
            IpcAuditFormat::Json
        } else {
            IpcAuditFormat::Human
        },
        since: None,
        cursor,
        limit: 1024,
    };
    crate::context::encode_type_tagged_message(type_name, &request, "audit request")
}

fn parse_audit_page(
    response: &[u8],
) -> Result<(Vec<String>, Option<AuditExportCursor>, bool), CliFailure> {
    let value = decode_daemon_frame(response, "audit reply")?;
    let Some(type_name) = value.get("type").and_then(Value::as_str) else {
        return Err(CliFailure::new(
            1,
            "daemon audit reply was missing a type discriminator",
        ));
    };
    match type_name {
        "auditResponse" => serde_json::from_value::<AuditResponseFrame>(value)
            .map(|frame| {
                let lines = frame
                    .payload
                    .entries
                    .into_iter()
                    .map(|entry| {
                        entry
                            .record
                            .map(|record| match record {
                                Value::String(line) => line,
                                record => record.to_string(),
                            })
                            .unwrap_or_else(|| {
                                serde_json::json!({
                                    "export_error": entry.error,
                                    "sequence": entry.sequence,
                                })
                                .to_string()
                            })
                    })
                    .collect();
                (lines, frame.payload.next_cursor, frame.payload.complete)
            })
            .map_err(|error| {
                CliFailure::new(1, format!("failed to decode auditResponse: {error}"))
            }),
        "error" => {
            let frame: ErrorFrame = serde_json::from_value(value).map_err(|error| {
                CliFailure::new(1, format!("failed to decode error reply: {error}"))
            })?;
            Err(cli_failure_from_daemon_error(frame.error))
        }
        other => Err(CliFailure::new(
            1,
            format!("unexpected audit reply type {other}"),
        )),
    }
}

pub(crate) fn parse_audit_reply(response: &[u8]) -> Result<Vec<String>, CliFailure> {
    parse_audit_page(response).map(|(lines, _, _)| lines)
}

pub(crate) fn render_daemon_audit_lines(
    lines: &[String],
    json_mode: bool,
) -> Result<(), CliFailure> {
    if json_mode {
        if let [line] = lines {
            let trimmed = line.trim_start();
            if trimmed.starts_with('{') || trimmed.starts_with('[') {
                if line.ends_with('\n') {
                    print_stdout(line);
                } else {
                    print_stdout(&(line.to_owned() + "\n"));
                }
                return Ok(());
            }
        }
        print_json(&json!({ "lines": lines }))?;
    } else if lines.is_empty() {
        print_stdout("");
    } else {
        print_stdout(&(lines.join("\n") + "\n"));
    }
    Ok(())
}

pub(crate) fn try_audit_via_socket(
    public_socket: &Path,
    json_mode: bool,
) -> Result<AuditSocketOutcome, CliFailure> {
    if !public_socket.exists() {
        return Ok(AuditSocketOutcome::Unreachable);
    }
    let mut socket = match SeqpacketUnixSocket::connect(public_socket) {
        Ok(socket) => socket,
        Err(error) if is_daemon_unreachable(&error) => {
            return Ok(AuditSocketOutcome::Unreachable);
        }
        Err(error) => {
            return Err(CliFailure::new(
                1,
                format!("failed to connect to {}: {error}", public_socket.display()),
            ));
        }
    };
    socket
        .send_frame(&daemon_hello_frame("hello")?)
        .map_err(|error| CliFailure::new(1, format!("failed to send hello frame: {error}")))?;
    let hello_response = socket
        .recv_frame()
        .map_err(|error| CliFailure::new(1, format!("failed to receive hello reply: {error}")))?;
    let _ = parse_hello_reply(&hello_response)?;

    let mut cursor = None;
    let mut lines = Vec::new();
    for _ in 0..1024 {
        let request = daemon_audit_frame_with_cursor("audit", json_mode, cursor.clone())?;
        socket.send_frame(&request).map_err(|error| {
            CliFailure::new(1, format!("failed to send audit request: {error}"))
        })?;
        let response = socket.recv_frame().map_err(|error| {
            CliFailure::new(1, format!("failed to receive audit reply: {error}"))
        })?;
        let (page, next_cursor, complete) = parse_audit_page(&response)?;
        lines.extend(page);
        if complete {
            return Ok(AuditSocketOutcome::Lines(lines));
        }
        cursor = next_cursor;
        if cursor.is_none() {
            return Err(CliFailure::new(
                1,
                "audit export pagination omitted continuation metadata",
            ));
        }
    }
    Err(CliFailure::new(
        1,
        "audit export exceeded the bounded pagination limit",
    ))
}

fn auth_status(
    context: &ZoneContext,
    test_uid: Option<u32>,
    mode: OutputMode,
) -> Result<i32, CliFailure> {
    let local = CliContext::from_env()?;
    let uid = test_uid.unwrap_or_else(|| nix::unistd::Uid::effective().as_raw());
    let admin_uids = parse_uid_env("D2B_TEST_ADMIN_UIDS");
    let launcher_uids = parse_uid_env("D2B_TEST_LAUNCHER_UIDS");
    let role = if admin_uids.contains(&uid) {
        AuthRoleV2::Admin
    } else if launcher_uids.contains(&uid) {
        AuthRoleV2::Launcher
    } else {
        AuthRoleV2::None
    };

    let public_probe = match local.auth_status_fixture.clone() {
        Some(fixture) => crate::context::SocketProbe {
            reachable: fixture.public_reachable.unwrap_or(false),
            version: fixture.public_version,
        },
        None => crate::context::probe_socket(context.public_socket_path()).unwrap_or(
            crate::context::SocketProbe {
                reachable: false,
                version: None,
            },
        ),
    };
    let broker_probe = match local.auth_status_fixture {
        Some(fixture) => crate::context::SocketProbe {
            reachable: fixture.broker_reachable.unwrap_or(false),
            version: fixture.broker_version,
        },
        None => crate::context::SocketProbe {
            reachable: false,
            version: None,
        },
    };

    let allowed = allowed_subcommands(role);
    let denied = all_known_subcommands()
        .into_iter()
        .filter(|command| !allowed.contains(command))
        .map(|name| AuthDeniedSubcommandV2 {
            reason: denied_reason(role, &name).to_owned(),
            name,
        })
        .collect::<Vec<_>>();
    let output = AuthStatusOutputV2 {
        role,
        effective_uid: uid,
        sockets: vec![
            AuthSocketStatusV2 {
                name: "public".to_owned(),
                path: context.public_socket_path().display().to_string(),
                reachable: public_probe.reachable,
                version: public_probe.version,
            },
            AuthSocketStatusV2 {
                name: "broker".to_owned(),
                path: local.broker_socket.display().to_string(),
                reachable: broker_probe.reachable,
                version: broker_probe.version,
            },
        ],
        allowed_subcommands: allowed.into_iter().collect(),
        denied_subcommands: denied,
    };
    if mode.is_json() {
        print_json(&output)?;
    } else {
        print_stdout(&render_auth_status_human(&output));
    }
    Ok(0)
}

pub(crate) fn effective_uid() -> u32 {
    nix::unistd::Uid::effective().as_raw()
}

pub(crate) fn parse_uid_env(name: &str) -> BTreeSet<u32> {
    std::env::var(name)
        .ok()
        .map(|value| {
            value
                .split(',')
                .filter_map(|part| part.trim().parse::<u32>().ok())
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn all_known_subcommands() -> Vec<String> {
    [
        "list",
        "status",
        "launch",
        "audit",
        "host check",
        "auth status",
        "op inspect",
        "realm list",
        "realm inspect",
        "realm enter",
        "realm run",
        "up",
        "down",
        "restart",
        "boot",
        "build",
        "switch",
        "test",
        "rollback",
        "generations",
        "gc",
        "usb",
        "console",
        "audio",
        "keys list",
        "rotate-known-host",
        "trust",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

pub(crate) fn allowed_subcommands(role: AuthRoleV2) -> BTreeSet<String> {
    match role {
        AuthRoleV2::Admin => all_known_subcommands().into_iter().collect(),
        AuthRoleV2::Launcher => all_known_subcommands()
            .into_iter()
            .filter(|command| command != "audit")
            .collect(),
        AuthRoleV2::None => [
            "list",
            "status",
            "host check",
            "auth status",
            "op inspect",
            "realm list",
            "realm inspect",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
    }
}

pub(crate) fn denied_reason(role: AuthRoleV2, command: &str) -> &'static str {
    match (role, command) {
        (AuthRoleV2::Admin, _) => "allowed",
        (_, "audit") => "audit requires admin role in `d2b.site.adminUsers`.",
        (AuthRoleV2::Launcher, _) => "allowed",
        (AuthRoleV2::None, _) => {
            "this subcommand requires launcher membership or daemon-admin privileges."
        }
    }
}

pub(crate) fn render_auth_status_human(output: &AuthStatusOutputV2) -> String {
    let mut text = String::new();
    let _ = writeln!(
        text,
        "role: {}",
        match output.role {
            AuthRoleV2::None => "none",
            AuthRoleV2::Launcher => "launcher",
            AuthRoleV2::Admin => "admin",
        }
    );
    let _ = writeln!(text, "effective uid: {}", output.effective_uid);
    text.push_str("sockets:\n");
    for socket in &output.sockets {
        let _ = writeln!(
            text,
            "- {}: {}{}",
            socket.name,
            if socket.reachable {
                "reachable"
            } else {
                "unreachable"
            },
            socket
                .version
                .as_ref()
                .map(|version| format!(" (version {version})"))
                .unwrap_or_default(),
        );
    }
    let _ = writeln!(
        text,
        "allowed subcommands: {}",
        output.allowed_subcommands.join(", ")
    );
    if !output.denied_subcommands.is_empty() {
        text.push_str("denied subcommands:\n");
        for denied in &output.denied_subcommands {
            let _ = writeln!(text, "- {}: {}", denied.name, denied.reason);
        }
    }
    text
}

pub(crate) fn runtime_dispatch(cli: &ModernCli, context: &ZoneContext) -> Result<i32, CliFailure> {
    let mode = output_mode(cli.json, cli.human)?;
    let deadline = if cli.no_deadline {
        ZoneContext::deadline(Some("900s"))?
    } else {
        ZoneContext::deadline(cli.deadline.as_deref())?
    };

    match &cli.command {
        ModernCommand::Get(args) => resource::get(context, args, mode, deadline),
        ModernCommand::List(args) => resource::list(context, args, mode, deadline),
        ModernCommand::Watch(args) => resource::watch(context, args, mode, deadline),
        ModernCommand::Create(args) => resource::create(context, args, mode, deadline),
        ModernCommand::UpdateSpec(args) => resource::update_spec(context, args, mode, deadline),
        ModernCommand::Delete(args) => resource::delete(context, args, mode, deadline),
        ModernCommand::Status(args) => resource::status(context, args, mode, deadline),
        ModernCommand::Upgrade(args) => resource::upgrade(context, args, mode, deadline),
        ModernCommand::Reconcile(args) => resource::reconcile(context, args, mode, deadline),
        ModernCommand::Host(args) => host::run(context, args, mode, deadline),
        ModernCommand::Guest(args) => guest::run_guest(context, args, mode, deadline),
        ModernCommand::Process(args) => guest::run_process(context, args, mode, deadline),
        ModernCommand::Exec(args) => exec::run(context, args, mode, deadline),
        ModernCommand::Shell(args) => shell::run(context, args, mode, deadline),
        ModernCommand::Volume(args) => resource::typed(context, "Volume", args, mode, deadline),
        ModernCommand::Network(args) => resource::typed(context, "Network", args, mode, deadline),
        ModernCommand::Device(args) => resource::typed(context, "Device", args, mode, deadline),
        ModernCommand::Endpoint(args) => endpoint::run(context, args, mode, deadline),
        ModernCommand::Export(args) => share::run_export(context, args, mode, deadline),
        ModernCommand::Import(args) => share::run_import(context, args, mode, deadline),
        ModernCommand::Resource(args) => resource::run_resource(context, args, mode, deadline),
        ModernCommand::User(args) => resource::typed(context, "User", args, mode, deadline),
        ModernCommand::Credential(args) => {
            resource::typed(context, "Credential", args, mode, deadline)
        }
        ModernCommand::Provider(args) => provider::run(context, args, mode, deadline),
        ModernCommand::Zone(args) => zone::run(context, args, mode, deadline),
        ModernCommand::Quota(args) => resource::typed(context, "Quota", args, mode, deadline),
        ModernCommand::EmergencyPolicy(args) => {
            resource::typed(context, "EmergencyPolicy", args, mode, deadline)
        }
        ModernCommand::Activation(args) => activation::run(context, args, mode, deadline),
        ModernCommand::Complete(args) => complete::run(args, Some(context), mode, deadline),
        ModernCommand::Audit(args) => audit(context, args, mode, deadline),
        ModernCommand::Op(args) => operation_inspect(context, args, mode, deadline),
        ModernCommand::Auth(args) => auth(context, args, mode),
        ModernCommand::Audio(args) => provider_projection(context, "audio", args, mode, deadline),
        ModernCommand::Clipboard(args) => {
            provider_projection(context, "clipboard", args, mode, deadline)
        }
        ModernCommand::Display(args) => {
            provider_projection(context, "display", args, mode, deadline)
        }
    }
}

fn audit(
    context: &ZoneContext,
    args: &GenericAuditArgs,
    mode: OutputMode,
    _deadline: crate::context::RequestDeadline,
) -> Result<i32, CliFailure> {
    if args.strict {
        return emit_host_error(
            &not_yet_implemented_envelope("audit --strict"),
            mode.is_json(),
        );
    }
    match try_audit_via_socket(context.public_socket_path(), mode.is_json())? {
        AuditSocketOutcome::Lines(lines) => {
            render_daemon_audit_lines(&lines, mode.is_json())?;
            Ok(0)
        }
        AuditSocketOutcome::Unreachable => {
            emit_host_error(&daemon_down_envelope("audit"), mode.is_json())
        }
    }
}

fn auth(
    context: &ZoneContext,
    args: &GenericAuthArgs,
    mode: OutputMode,
) -> Result<i32, CliFailure> {
    match &args.command {
        GenericAuthCommand::Status => auth_status(context, args.test_uid, mode),
    }
}

fn operation_inspect(
    context: &ZoneContext,
    args: &GenericOpArgs,
    mode: OutputMode,
    deadline: crate::context::RequestDeadline,
) -> Result<i32, CliFailure> {
    let GenericOpCommand::Inspect(args) = &args.command;
    for value in [&args.operation_id, &args.trace_id, &args.span_id]
        .into_iter()
        .flatten()
    {
        if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
            return Err(context.failure(
                "ref-invalid",
                "operation inspection identifiers are outside their bounds",
                mode,
                2,
            ));
        }
    }
    let value = context.invoke(
        "OperationInspect",
        serde_json::json!({
            "operationId": args.operation_id,
            "traceId": args.trace_id,
            "spanId": args.span_id,
            "watch": args.watch,
        }),
        deadline,
        mode,
    )?;
    if args.watch {
        context.emit_stream(&value, mode)?;
    } else {
        context.emit(&value, mode)?;
    }
    Ok(0)
}

fn provider_projection(
    context: &ZoneContext,
    top_level: &str,
    args: &ProjectionCommandArgs,
    mode: OutputMode,
    deadline: crate::context::RequestDeadline,
) -> Result<i32, CliFailure> {
    if crate::dispatch::BUILTIN_COMMANDS.contains(&top_level) {
        return Err(context.failure(
            "resource-schema-invalid",
            "Provider command collides with a built-in command",
            mode,
            1,
        ));
    }
    provider::validate_name(top_level, "Provider command name")
        .map_err(|message| context.failure("ref-invalid", &message, mode, 2))?;
    provider::validate_name(&args.verb, "Provider projection verb")
        .map_err(|message| context.failure("ref-invalid", &message, mode, 2))?;
    let value = context.invoke(
        "ProviderCommand",
        serde_json::json!({
            "topLevel": top_level,
            "verb": &args.verb,
            "args": &args.args,
        }),
        deadline,
        mode,
    )?;
    context.emit(&value, mode)?;
    Ok(0)
}

pub(crate) fn modern_run(raw_args: Vec<OsString>) -> i32 {
    let cli = match ModernCli::try_parse_from(raw_args.clone()) {
        Ok(cli) => cli,
        Err(error) => {
            let code = error.exit_code();
            let _ = error.print();
            return code;
        }
    };
    if let ModernCommand::Complete(args) = &cli.command {
        let mode = match output_mode(cli.json, cli.human) {
            Ok(mode) => mode,
            Err(error) => return crate::report_failure(error),
        };
        let deadline = match if cli.no_deadline {
            ZoneContext::deadline(Some("900s"))
        } else {
            ZoneContext::deadline(cli.deadline.as_deref())
        } {
            Ok(deadline) => deadline,
            Err(error) => return report_dispatch_failure(None, &cli, mode, error),
        };
        return match complete::run(args, None, mode, deadline) {
            Ok(code) => code,
            Err(error) => report_dispatch_failure(None, &cli, mode, error),
        };
    }
    let local_host_command = matches!(
        &cli.command,
        ModernCommand::Host(host::HostArgs {
            command: host::HostCommand::Check(_)
                | host::HostCommand::Install(_)
                | host::HostCommand::Reconcile(_)
                | host::HostCommand::Validate(_)
                | host::HostCommand::Doctor(_)
        })
    ) || matches!(&cli.command, ModernCommand::Auth(_));
    let context = if local_host_command {
        ZoneContext::local_only_with_explicit_zone(
            cli.zone.is_some() || std::env::var_os("D2B_ZONE").is_some(),
        )
    } else {
        match ZoneContext::discover(cli.zone.as_deref()) {
            Ok(context) => context,
            Err(error) => {
                let mode = output_mode(cli.json, cli.human).unwrap_or(OutputMode::Json);
                return report_dispatch_failure(None, &cli, mode, error);
            }
        }
    };
    match runtime_dispatch(&cli, &context) {
        Ok(code) => code,
        Err(error) => report_dispatch_failure(
            Some(&context),
            &cli,
            output_mode(cli.json, cli.human).unwrap_or(OutputMode::Json),
            error,
        ),
    }
}

fn report_dispatch_failure(
    context: Option<&ZoneContext>,
    cli: &ModernCli,
    mode: OutputMode,
    error: CliFailure,
) -> i32 {
    let exit_code = error.exit_code;
    if let Some(rendered) = error.rendered_stderr {
        crate::print_stdout(&rendered);
        return exit_code;
    }
    if mode.is_json() {
        let (prefix, suffix) = error
            .message
            .split_once(':')
            .unwrap_or(("", error.message.as_str()));
        let class = if !prefix.is_empty()
            && prefix
                .chars()
                .all(|character| character.is_ascii_lowercase() || character == '-')
        {
            prefix
        } else if exit_code == 2 {
            "ref-invalid"
        } else {
            "internal-error"
        };
        let detail = if suffix.trim().is_empty() {
            error.message.as_str()
        } else {
            suffix.trim()
        };
        let message = crate::context::bounded_message(detail);
        let failure = if let Some(context) = context {
            context.failure(class, &message, mode, exit_code)
        } else {
            let requested_zone = cli
                .zone
                .clone()
                .or_else(|| std::env::var("D2B_ZONE").ok())
                .unwrap_or_else(|| "local-root".to_owned());
            let zone = if d2b_contracts_resource::v3::ZoneId::parse(requested_zone.clone()).is_ok()
            {
                requested_zone
            } else {
                "local-root".to_owned()
            };
            let mut failure = CliFailure::new(exit_code, format!("{class}: {message}"));
            let mut rendered = serde_json::to_string(&serde_json::json!({
                "ok": false,
                "zoneRef": format!("Zone/{zone}"),
                "errorClass": class,
                "message": message,
                "schemaVersion": crate::context::JSON_SCHEMA_VERSION,
            }))
            .unwrap_or_else(|_| {
                "{\"ok\":false,\"errorClass\":\"internal-error\",\"schemaVersion\":1}".to_owned()
            });
            rendered.push('\n');
            failure.rendered_stderr = Some(rendered);
            failure
        };
        if let Some(rendered) = failure.rendered_stderr {
            crate::print_stdout(&rendered);
            return failure.exit_code;
        }
    }
    crate::report_failure(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn modern_parser_accepts_zone_resource_commands_and_global_flags() {
        let cli = ModernCli::try_parse_from([
            "d2b", "--zone", "dev", "guest", "start", "work", "--apply", "--json",
        ])
        .expect("modern guest command parses");
        assert_eq!(cli.zone.as_deref(), Some("dev"));
        assert!(cli.json);
        assert!(matches!(cli.command, ModernCommand::Guest(_)));
    }

    #[test]
    fn modern_list_requires_a_resource_type() {
        let cli = ModernCli::try_parse_from(["d2b", "list", "Zone", "--json"])
            .expect("typed resource list parses");
        assert!(cli.json);
        let ModernCommand::List(args) = cli.command else {
            panic!("expected typed list command");
        };
        assert_eq!(args.resource_type, "Zone");
        assert!(ModernCli::try_parse_from(["d2b", "list", "--json"]).is_err());
    }

    #[test]
    fn modern_parser_has_no_v2_alias_or_realm_dispatch() {
        assert!(ModernCli::try_parse_from(["d2b", "up", "work"]).is_err());
        assert!(ModernCli::try_parse_from(["d2b", "vm", "start", "work"]).is_err());
        assert!(ModernCli::try_parse_from(["d2b", "realm", "list"]).is_err());
        assert!(ModernCli::try_parse_from(["d2b", "unknown-provider-command"]).is_err());
    }

    #[test]
    fn modern_parser_covers_manifest_owned_command_surfaces() {
        for args in [
            &["d2b", "device", "usb", "probe"][..],
            &["d2b", "device", "security-key", "status"][..],
            &["d2b", "volume", "verify", "state"][..],
            &["d2b", "activation", "apply", "--dry-run"][..],
            &["d2b", "endpoint", "resolve", "ready"][..],
            &["d2b", "export", "list"][..],
            &["d2b", "import", "graph", "microphone"][..],
            &["d2b", "audio", "status"][..],
            &["d2b", "clipboard", "arm"][..],
            &["d2b", "display", "list"][..],
        ] {
            ModernCli::try_parse_from(args).unwrap_or_else(|error| {
                panic!("manifest command surface did not parse: {args:?}: {error}")
            });
        }
    }

    #[test]
    fn built_in_registry_is_unique_and_matches_expected_size() {
        let mut names = BUILTIN_COMMANDS.to_vec();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), BUILTIN_COMMANDS.len());
        assert_eq!(BUILTIN_COMMANDS.len(), 32);
        assert!(BUILTIN_COMMANDS.contains(&"endpoint"));
        assert!(BUILTIN_COMMANDS.contains(&"import"));
    }

    #[test]
    fn audit_request_frame_keeps_the_bounded_control_wire_shape() {
        let frame = daemon_audit_frame("audit", true).expect("encode audit request");
        let value: Value = serde_json::from_slice(&frame).expect("audit frame JSON");
        assert_eq!(value["type"], "audit");
        assert_eq!(value["format"], "json");
        assert_eq!(value["limit"], 1024);
        assert!(value.get("cursor").is_some());
        assert!(value["cursor"].is_null());
        assert!(value.get("zoneRef").is_none());
        assert!(value.get("realm").is_none());
    }
}
