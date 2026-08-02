//! ShellSession lifecycle and terminal attachment commands.

use clap::{Args, Subcommand};
use serde_json::json;

use crate::{
    CliFailure,
    context::{OutputMode, RequestDeadline, ZoneContext, parse_resource_ref},
};

#[derive(Debug, Args, Clone)]
pub(crate) struct ShellArgs {
    #[command(subcommand)]
    pub(crate) command: ShellCommand,
}

#[derive(Debug, Subcommand, Clone)]
pub(crate) enum ShellCommand {
    Open(ShellOpenArgs),
    Attach(ShellAttachArgs),
    List(ShellListArgs),
    Detach(ShellRefArgs),
    Kill(ShellRefArgs),
    Status(ShellStatusArgs),
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ShellOpenArgs {
    pub(crate) execution_ref: String,
    #[arg(long)]
    pub(crate) name: Option<String>,
    #[arg(long)]
    pub(crate) force: bool,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ShellAttachArgs {
    pub(crate) resource_ref: String,
    #[arg(long)]
    pub(crate) force: bool,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ShellListArgs {
    pub(crate) execution_ref: Option<String>,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ShellRefArgs {
    pub(crate) resource_ref: String,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ShellStatusArgs {
    pub(crate) resource_ref: String,
    #[arg(long)]
    pub(crate) watch: bool,
}

pub(crate) fn run(
    context: &ZoneContext,
    args: &ShellArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    match &args.command {
        ShellCommand::Open(args) => open(context, args, mode, deadline),
        ShellCommand::Attach(args) => attach(context, args, mode, deadline),
        ShellCommand::List(args) => list(context, args, mode, deadline),
        ShellCommand::Detach(args) => detach_or_kill(context, "Detach", args, mode, deadline),
        ShellCommand::Kill(args) => detach_or_kill(context, "Kill", args, mode, deadline),
        ShellCommand::Status(args) => status(context, args, mode, deadline),
    }
}

fn open(
    context: &ZoneContext,
    args: &ShellOpenArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let execution_ref = parse_resource_ref(&args.execution_ref, None)?;
    if !matches!(execution_ref.resource_type().as_str(), "Host" | "Guest") {
        return Err(context.failure(
            "ref-invalid",
            "shell open requires a Host or Guest executionRef",
            mode,
            2,
        ));
    }
    if !mode.is_json() && !crate::stdout_is_tty() {
        return Err(context.failure(
            "shell-transport-error",
            "shell open requires a terminal unless --json is used",
            mode,
            2,
        ));
    }
    warn_unsafe_local(&execution_ref, mode);
    let value = context.invoke(
        "Create",
        json!({
            "resourceType": "ShellSession",
            "executionRef": execution_ref.to_canonical_string(),
            "name": args.name,
            "force": args.force,
            "attach": !mode.is_json(),
            "detachOnHangup": true,
        }),
        deadline,
        mode,
    )?;
    context.emit(&with_unsafe_posture(value, &execution_ref), mode)?;
    Ok(0)
}

fn attach(
    context: &ZoneContext,
    args: &ShellAttachArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    if mode.is_json() || !crate::stdout_is_tty() {
        return Err(context.failure(
            "shell-transport-error",
            "shell attach requires a terminal and does not emit JSON",
            mode,
            2,
        ));
    }
    let resource_ref = validate_session_ref(context, &args.resource_ref, mode)?;
    let value = context.invoke(
        "OpenTerminal",
        json!({
            "kind": "shell",
            "resourceRef": resource_ref.to_canonical_string(),
            "force": args.force,
            "detachOnHangup": true,
        }),
        deadline,
        mode,
    )?;
    context.emit(&value, mode)?;
    Ok(0)
}

fn list(
    context: &ZoneContext,
    args: &ShellListArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let execution_ref = args
        .execution_ref
        .as_deref()
        .map(|value| parse_resource_ref(value, None))
        .transpose()?;
    if let Some(reference) = &execution_ref
        && !matches!(reference.resource_type().as_str(), "Host" | "Guest")
    {
        return Err(context.failure(
            "ref-invalid",
            "shell list executionRef must name a Host or Guest",
            mode,
            2,
        ));
    }
    let value = context.invoke(
        "List",
        json!({
            "resourceType": "ShellSession",
            "executionRef": execution_ref.map(|reference| reference.to_canonical_string()),
        }),
        deadline,
        mode,
    )?;
    context.emit(&value, mode)?;
    Ok(0)
}

fn detach_or_kill(
    context: &ZoneContext,
    method: &str,
    args: &ShellRefArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let resource_ref = validate_session_ref(context, &args.resource_ref, mode)?;
    let value = context.invoke(
        method,
        json!({ "resourceRef": resource_ref.to_canonical_string() }),
        deadline,
        mode,
    )?;
    context.emit(&value, mode)?;
    Ok(0)
}

fn status(
    context: &ZoneContext,
    args: &ShellStatusArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let resource_ref = validate_session_ref(context, &args.resource_ref, mode)?;
    let value = context.invoke(
        "Status",
        json!({
            "resourceRef": resource_ref.to_canonical_string(),
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

fn validate_session_ref(
    context: &ZoneContext,
    value: &str,
    mode: OutputMode,
) -> Result<d2b_contracts::v3::ResourceRef, CliFailure> {
    let resource_ref = parse_resource_ref(value, None)?;
    if resource_ref.resource_type().as_str() != "ShellSession"
        && !resource_ref
            .resource_type()
            .as_str()
            .contains("ShellSession")
    {
        return Err(context.failure(
            "ref-invalid",
            "shell command requires a ShellSession ResourceRef",
            mode,
            2,
        ));
    }
    Ok(resource_ref)
}

fn warn_unsafe_local(resource_ref: &d2b_contracts::v3::ResourceRef, mode: OutputMode) {
    if resource_ref.resource_type().as_str() == "Host" && !mode.is_json() {
        crate::print_stderr(
            "warning: no isolation boundary - this process runs as your host user\n",
        );
    }
}

fn with_unsafe_posture(
    mut value: serde_json::Value,
    resource_ref: &d2b_contracts::v3::ResourceRef,
) -> serde_json::Value {
    if resource_ref.resource_type().as_str() == "Host"
        && let serde_json::Value::Object(object) = &mut value
    {
        object.insert(
            "isolationPosture".to_owned(),
            serde_json::Value::String("none".to_owned()),
        );
    }
    value
}
