//! EphemeralProcess execution commands.

use clap::{Args, Subcommand};
use serde_json::{Value, json};

use crate::{
    CliFailure,
    context::{OutputMode, RequestDeadline, ZoneContext, parse_resource_ref},
};

#[derive(Debug, Args, Clone)]
pub(crate) struct ExecArgs {
    #[command(subcommand)]
    pub(crate) command: ExecCommand,
}

#[derive(Debug, Subcommand, Clone)]
pub(crate) enum ExecCommand {
    Run(ExecRunArgs),
    Attach(ExecAttachArgs),
    Wait(ExecRefArgs),
    Status(ExecStatusArgs),
    List(ExecListArgs),
    Logs(ExecLogsArgs),
    Kill(ExecKillArgs),
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ExecRunArgs {
    pub(crate) execution_ref: String,
    #[arg(long)]
    pub(crate) name: Option<String>,
    #[arg(long)]
    pub(crate) domain: Option<String>,
    #[arg(long = "user")]
    pub(crate) user_ref: Option<String>,
    #[arg(long)]
    pub(crate) provider: Option<String>,
    #[arg(long = "env", value_name = "KEY=VALUE")]
    pub(crate) env: Vec<String>,
    #[arg(long)]
    pub(crate) cwd: Option<String>,
    #[arg(last = true, required = true)]
    pub(crate) command: Vec<String>,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ExecAttachArgs {
    pub(crate) resource_ref: String,
    #[arg(short = 'i', long)]
    pub(crate) interactive: bool,
    #[arg(short = 't', long)]
    pub(crate) tty: bool,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ExecRefArgs {
    pub(crate) resource_ref: String,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ExecStatusArgs {
    pub(crate) resource_ref: String,
    #[arg(long)]
    pub(crate) watch: bool,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ExecListArgs {
    pub(crate) execution_ref: Option<String>,
    #[arg(long)]
    pub(crate) phase: Option<String>,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ExecLogsArgs {
    pub(crate) resource_ref: String,
    #[arg(long = "stdout-offset")]
    pub(crate) stdout_offset: Option<u64>,
    #[arg(long = "stderr-offset")]
    pub(crate) stderr_offset: Option<u64>,
    #[arg(long = "max-len")]
    pub(crate) max_len: Option<u64>,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ExecKillArgs {
    pub(crate) resource_ref: String,
    #[arg(long, default_value = "term")]
    pub(crate) signal: String,
}

pub(crate) fn run(
    context: &ZoneContext,
    args: &ExecArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    match &args.command {
        ExecCommand::Run(args) => run_create(context, args, mode, deadline),
        ExecCommand::Attach(args) => attach(context, args, mode, deadline),
        ExecCommand::Wait(args) => wait(context, args, mode, deadline),
        ExecCommand::Status(args) => status(context, args, mode, deadline),
        ExecCommand::List(args) => list(context, args, mode, deadline),
        ExecCommand::Logs(args) => logs(context, args, mode, deadline),
        ExecCommand::Kill(args) => kill(context, args, mode, deadline),
    }
}

fn run_create(
    context: &ZoneContext,
    args: &ExecRunArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let execution_ref = parse_resource_ref(&args.execution_ref, None)?;
    if !matches!(execution_ref.resource_type().as_str(), "Host" | "Guest") {
        return Err(context.failure(
            "ref-invalid",
            "exec executionRef must name a Host or Guest",
            mode,
            2,
        ));
    }
    validate_env(&args.env)?;
    if let Some(domain) = args.domain.as_deref()
        && !matches!(domain, "system" | "user")
    {
        return Err(context.failure("ref-invalid", "exec domain must be system or user", mode, 2));
    }
    if let Some(user_ref) = args.user_ref.as_deref() {
        let user_ref = parse_resource_ref(user_ref, Some("User"))?;
        if user_ref.resource_type().as_str() != "User" {
            return Err(context.failure(
                "ref-invalid",
                "exec --user must name a User ResourceRef",
                mode,
                2,
            ));
        }
    }
    if let Some(provider_ref) = args.provider.as_deref() {
        let provider_ref = parse_resource_ref(provider_ref, Some("Provider"))?;
        if provider_ref.resource_type().as_str() != "Provider" {
            return Err(context.failure(
                "ref-invalid",
                "exec --provider must name a Provider ResourceRef",
                mode,
                2,
            ));
        }
    }
    if args
        .cwd
        .as_deref()
        .is_some_and(|cwd| cwd.is_empty() || cwd.len() > 4096 || cwd.chars().any(char::is_control))
    {
        return Err(context.failure(
            "ref-invalid",
            "exec working directory is outside its bounds",
            mode,
            2,
        ));
    }
    warn_unsafe_local(&execution_ref, mode);
    let value = context.invoke(
        "Create",
        json!({
            "resourceType": "EphemeralProcess",
            "executionRef": execution_ref.to_canonical_string(),
            "name": args.name,
            "domain": args.domain,
            "userRef": args.user_ref,
            "providerRef": args.provider,
            "env": args.env,
            "cwd": args.cwd,
            "command": args.command,
        }),
        deadline,
        mode,
    )?;
    context.emit(&with_unsafe_posture(value, &execution_ref), mode)?;
    Ok(0)
}

fn attach(
    context: &ZoneContext,
    args: &ExecAttachArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    if args.tty && mode.is_json() {
        return Err(context.failure("ref-invalid", "--tty is incompatible with --json", mode, 2));
    }
    let resource_ref = parse_resource_ref(&args.resource_ref, None)?;
    if resource_ref.resource_type().as_str() != "EphemeralProcess" {
        return Err(context.failure(
            "ref-invalid",
            "exec attach requires an EphemeralProcess ResourceRef",
            mode,
            2,
        ));
    }
    if args.tty && !crate::stdout_is_tty() {
        return Err(context.failure(
            "exec-transport-error",
            "exec attach --tty requires a terminal",
            mode,
            69,
        ));
    }
    let value = context.attach_process(resource_ref, args.interactive, args.tty, deadline, mode)?;
    context.emit(&value, mode)?;
    Ok(0)
}

fn wait(
    context: &ZoneContext,
    args: &ExecRefArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let resource_ref = validate_exec_ref(context, &args.resource_ref, mode)?;
    let value = context.invoke(
        "Wait",
        json!({ "resourceRef": resource_ref.to_canonical_string() }),
        deadline,
        mode,
    )?;
    let exit_code = value
        .get("guestExitCode")
        .or_else(|| value.get("exitCode"))
        .and_then(Value::as_i64)
        .filter(|code| (0..=255).contains(code))
        .map(|code| code as i32);
    context.emit(&value, mode)?;
    Ok(exit_code.unwrap_or(0))
}

fn status(
    context: &ZoneContext,
    args: &ExecStatusArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let resource_ref = validate_exec_ref(context, &args.resource_ref, mode)?;
    if args.watch && !mode.is_json() {
        return Err(context.failure(
            "ref-invalid",
            "exec status --watch output is JSON-lines only",
            mode,
            2,
        ));
    }
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

fn list(
    context: &ZoneContext,
    args: &ExecListArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let execution_ref = args
        .execution_ref
        .as_deref()
        .map(|value| parse_resource_ref(value, None))
        .transpose()?;
    if let Some(execution_ref) = &execution_ref
        && !matches!(execution_ref.resource_type().as_str(), "Host" | "Guest")
    {
        return Err(context.failure(
            "ref-invalid",
            "exec list executionRef must name a Host or Guest",
            mode,
            2,
        ));
    }
    let value = context.invoke(
        "List",
        json!({
            "resourceType": "EphemeralProcess",
            "executionRef": execution_ref.map(|value| value.to_canonical_string()),
            "phase": args.phase,
        }),
        deadline,
        mode,
    )?;
    context.emit(&value, mode)?;
    Ok(0)
}

fn logs(
    context: &ZoneContext,
    args: &ExecLogsArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let resource_ref = validate_exec_ref(context, &args.resource_ref, mode)?;
    let max_len = args.max_len.unwrap_or(64 * 1024);
    if max_len == 0 || max_len > 64 * 1024 {
        return Err(context.failure(
            "ref-invalid",
            "exec log max length must be between 1 and 65536",
            mode,
            2,
        ));
    }
    let value = context.invoke(
        "OpenRetainedLog",
        json!({
            "resourceRef": resource_ref.to_canonical_string(),
            "stdoutOffset": args.stdout_offset,
            "stderrOffset": args.stderr_offset,
            "maxLen": max_len,
        }),
        deadline,
        mode,
    )?;
    context.emit_stream(&value, mode)?;
    Ok(0)
}

fn kill(
    context: &ZoneContext,
    args: &ExecKillArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let resource_ref = validate_exec_ref(context, &args.resource_ref, mode)?;
    if !matches!(args.signal.as_str(), "term" | "kill" | "int" | "hup") {
        return Err(context.failure(
            "ref-invalid",
            "exec signal must be term, kill, int, or hup",
            mode,
            2,
        ));
    }
    let value = context.invoke(
        "Cancel",
        json!({
            "resourceRef": resource_ref.to_canonical_string(),
            "signal": args.signal,
        }),
        deadline,
        mode,
    )?;
    context.emit(&value, mode)?;
    Ok(0)
}

fn validate_exec_ref(
    context: &ZoneContext,
    value: &str,
    mode: OutputMode,
) -> Result<d2b_contracts::v3::ResourceRef, CliFailure> {
    let resource_ref = parse_resource_ref(value, None)?;
    if resource_ref.resource_type().as_str() != "EphemeralProcess" {
        return Err(context.failure(
            "ref-invalid",
            "exec command requires an EphemeralProcess ResourceRef",
            mode,
            2,
        ));
    }
    Ok(resource_ref)
}

fn validate_env(values: &[String]) -> Result<(), CliFailure> {
    for value in values {
        let Some((key, _)) = value.split_once('=') else {
            return Err(CliFailure::new(2, "exec environment must use KEY=VALUE"));
        };
        if key.is_empty()
            || key.len() > 64
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err(CliFailure::new(2, "exec environment key is invalid"));
        }
    }
    Ok(())
}

fn warn_unsafe_local(resource_ref: &d2b_contracts::v3::ResourceRef, mode: OutputMode) {
    if resource_ref.resource_type().as_str() == "Host" && !mode.is_json() {
        crate::print_stderr(
            "warning: no isolation boundary - this process runs as your host user\n",
        );
    }
}

fn with_unsafe_posture(mut value: Value, resource_ref: &d2b_contracts::v3::ResourceRef) -> Value {
    if resource_ref.resource_type().as_str() == "Host"
        && let Value::Object(object) = &mut value
    {
        object.insert(
            "isolationPosture".to_owned(),
            Value::String("none".to_owned()),
        );
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::OutputMode;

    #[test]
    fn attach_rejects_non_ephemeral_resources_with_the_existing_exit_code() {
        let context = ZoneContext::local_only();
        let args = ExecAttachArgs {
            resource_ref: "Process/not-attachable".to_owned(),
            interactive: false,
            tty: false,
        };
        let error = attach(
            &context,
            &args,
            OutputMode::Json,
            ZoneContext::deadline(Some("30s")).unwrap(),
        )
        .unwrap_err();
        assert_eq!(error.exit_code, 2);
        assert!(error.message.contains("EphemeralProcess"));
        assert!(!error.message.contains("not-attachable"));
    }

    #[test]
    fn json_tty_attach_is_refused_before_transport_access() {
        let context = ZoneContext::local_only();
        let args = ExecAttachArgs {
            resource_ref: "EphemeralProcess/command".to_owned(),
            interactive: true,
            tty: true,
        };
        let error = attach(
            &context,
            &args,
            OutputMode::Json,
            ZoneContext::deadline(Some("30s")).unwrap(),
        )
        .unwrap_err();
        assert_eq!(error.exit_code, 2);
        assert!(error.message.contains("--tty"));
    }
}
