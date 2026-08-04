//! The v3 command registry and command-to-surface dispatch.

use std::{ffi::OsString, path::PathBuf};

use crate::context::{OutputMode, ZoneContext, output_mode};
use crate::{
    CliFailure, activation, complete, endpoint, exec, guest, host, provider, resource, share,
    shell, zone,
};
use clap::{Args, Parser, Subcommand};

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

/// The clean-break CLI parser. Legacy parser definitions remain in `lib.rs`
/// only for their existing pure unit fixtures; this parser is the sole runtime
/// entry point.
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

#[derive(Debug, Subcommand, Clone)]
pub(crate) enum GenericAuthCommand {
    Status,
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
    _context: &ZoneContext,
    args: &GenericAuditArgs,
    mode: OutputMode,
    _deadline: crate::context::RequestDeadline,
) -> Result<i32, CliFailure> {
    let legacy = crate::LegacyContext::from_env()?;
    crate::cmd_audit(
        &legacy,
        &crate::AuditArgs {
            strict: args.strict,
            json: mode.is_json(),
            human: !mode.is_json(),
        },
        &[],
    )
}

fn auth(
    context: &ZoneContext,
    args: &GenericAuthArgs,
    mode: OutputMode,
) -> Result<i32, CliFailure> {
    match &args.command {
        GenericAuthCommand::Status => {
            let legacy = crate::LegacyContext::from_env()?;
            crate::cmd_auth_status(
                &legacy,
                &crate::AuthStatusArgs {
                    json: mode.is_json(),
                    human: !mode.is_json(),
                    test_uid: args.test_uid,
                },
            )
            .map_err(|error| {
                if error.message.starts_with("zone-unavailable") {
                    context.failure("zone-unavailable", "Zone runtime is unavailable", mode, 1)
                } else {
                    error
                }
            })
        }
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
                | host::HostCommand::Doctor(_),
        })
    ) || matches!(&cli.command, ModernCommand::Auth(_));
    let user_domain = raw_args
        .windows(2)
        .any(|window| window[0] == "--domain" && window[1] == "user")
        || raw_args
            .iter()
            .any(|arg| arg.to_string_lossy() == "--domain=user");
    let context = if local_host_command {
        ZoneContext::local_only()
    } else {
        match ZoneContext::discover_for_domain(cli.zone.as_deref(), user_domain) {
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
            let zone = if d2b_contracts::v3::ZoneId::parse(requested_zone.clone()).is_ok() {
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
}
