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
}

#[derive(Debug, Args, Clone)]
pub(crate) struct GenericGetArgs {
    pub(crate) resource_ref: String,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct GenericListArgs {
    pub(crate) resource_type: String,
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
    #[arg(long = "spec-file", conflicts_with = "spec-stdin")]
    pub(crate) spec_file: Option<PathBuf>,
    #[arg(long = "spec-stdin", conflicts_with = "spec-file")]
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
    #[arg(long = "spec-file", conflicts_with = "spec-stdin")]
    pub(crate) spec_file: Option<PathBuf>,
    #[arg(long = "spec-stdin", conflicts_with = "spec-file")]
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
        ModernCommand::Complete(args) => complete::run(args),
        ModernCommand::Audit(args) => unsupported(context, "audit", args.strict, mode),
        ModernCommand::Op(args) => unsupported(context, "op", &args.command, mode),
        ModernCommand::Auth(args) => unsupported(context, "auth", &args.command, mode),
    }
}

fn unsupported<T: std::fmt::Debug>(
    context: &ZoneContext,
    command: &str,
    _details: T,
    mode: OutputMode,
) -> Result<i32, CliFailure> {
    Err(context.failure(
        "not-implemented",
        &format!("{command} is not available through this resource-plane build"),
        mode,
        78,
    ))
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
    let context = match ZoneContext::discover(cli.zone.as_deref()) {
        Ok(context) => context,
        Err(error) => return crate::report_failure(error),
    };
    match runtime_dispatch(&cli, &context) {
        Ok(code) => code,
        Err(error) => crate::report_failure(error),
    }
}
