//! ShellSession lifecycle and terminal attachment commands.

use clap::{Args, Subcommand};
use serde_json::json;

use crate::{
    CliFailure,
    context::{OutputMode, RequestDeadline, ZoneContext, parse_resource_ref},
};

const SHELL_SESSION_TYPE: &str = "shell-terminal.d2bus.org.ShellSession";

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
    if let Some(name) = args.name.as_deref()
        && !valid_session_name(name)
    {
        return Err(context.failure(
            "ref-invalid",
            "shell session name is outside its bounds",
            mode,
            2,
        ));
    }
    warn_unsafe_local(&execution_ref, mode);
    let name = args.name.as_deref().unwrap_or("primary");
    let session_ref =
        d2b_contracts::v3::ResourceRef::parse(&format!("{SHELL_SESSION_TYPE}/{name}"))
            .map_err(|_| context.failure("ref-invalid", "invalid shell session name", mode, 2))?;
    if mode.is_json() {
        let value = context.invoke(
            "Create",
            json!({
                "resourceType": SHELL_SESSION_TYPE,
                "resourceRef": session_ref.to_canonical_string(),
                "executionRef": execution_ref.to_canonical_string(),
                "force": args.force,
                "attach": false,
                "initialSize": {
                    "rows": 24,
                    "cols": 80,
                },
            }),
            deadline,
            mode,
        )?;
        context.emit(&with_unsafe_posture(value, &execution_ref), mode)?;
        return Ok(0);
    }
    context.attach_shell(session_ref, Some(execution_ref), args.force, true, deadline)?;
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
    context.attach_shell(resource_ref, None, args.force, false, deadline)?;
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
            "resourceType": SHELL_SESSION_TYPE,
            "executionRef": execution_ref.map(|reference| reference.to_canonical_string()),
        }),
        deadline,
        mode,
    )?;
    context.emit(&value, mode)?;
    Ok(0)
}

fn valid_session_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 63
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
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
    if args.watch && !mode.is_json() {
        return Err(context.failure(
            "ref-invalid",
            "shell status --watch requires --json",
            mode,
            2,
        ));
    }
    if args.watch {
        let started = std::time::Instant::now();
        let mut previous = None;
        loop {
            let Some(call_deadline) = deadline.remaining(started.elapsed()) else {
                return Ok(0);
            };
            let value = context.invoke(
                "Status",
                json!({
                    "resourceRef": resource_ref.to_canonical_string(),
                    "watch": false,
                }),
                call_deadline,
                mode,
            )?;
            if previous.as_ref() != Some(&value) {
                context.emit_stream(&value, mode)?;
            }
            if terminal_shell_status(&value) {
                return Ok(0);
            }
            previous = Some(value);
            let Some(remaining) = deadline.remaining(started.elapsed()) else {
                return Ok(0);
            };
            std::thread::sleep(
                remaining
                    .duration()
                    .min(std::time::Duration::from_millis(250)),
            );
        }
    }
    let value = context.invoke(
        "Status",
        json!({
            "resourceRef": resource_ref.to_canonical_string(),
            "watch": false,
        }),
        deadline,
        mode,
    )?;
    context.emit(&value, mode)?;
    Ok(0)
}

fn terminal_shell_status(value: &serde_json::Value) -> bool {
    matches!(
        value.get("state").and_then(serde_json::Value::as_str),
        Some("killed" | "pool-unavailable" | "feature-disabled" | "output-gap")
    )
}

fn validate_session_ref(
    context: &ZoneContext,
    value: &str,
    mode: OutputMode,
) -> Result<d2b_contracts::v3::ResourceRef, CliFailure> {
    let canonical = value
        .strip_prefix("ShellSession/")
        .map(|name| format!("{SHELL_SESSION_TYPE}/{name}"))
        .unwrap_or_else(|| value.to_owned());
    let resource_ref = parse_resource_ref(&canonical, None)?;
    if resource_ref.resource_type().as_str() != SHELL_SESSION_TYPE {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{SessionClient, TransportError};
    use std::sync::{Arc, Mutex};

    struct RecordingClient {
        requests: Mutex<Vec<Vec<u8>>>,
    }

    impl SessionClient for RecordingClient {
        fn invoke(
            &self,
            request: &[u8],
            _deadline: RequestDeadline,
        ) -> Result<Vec<u8>, TransportError> {
            self.requests.lock().unwrap().push(request.to_vec());
            Ok(
                br#"{"attached":false,"resourceRef":"shell-terminal.d2bus.org.ShellSession/primary","status":{"name":"primary","state":"detached","attached":false}}"#
                    .to_vec(),
            )
        }
    }

    #[test]
    fn json_open_creates_without_opening_a_terminal_stream() {
        let client = Arc::new(RecordingClient {
            requests: Mutex::new(Vec::new()),
        });
        let context =
            ZoneContext::with_client("dev", "/run/d2b/zones/dev/public.sock", client.clone())
                .unwrap();
        assert_eq!(
            open(
                &context,
                &ShellOpenArgs {
                    execution_ref: "Host/tools".to_owned(),
                    name: Some("primary".to_owned()),
                    force: false,
                },
                OutputMode::Json,
                ZoneContext::deadline(None).unwrap(),
            )
            .unwrap(),
            0
        );
        let requests = client.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        let request: serde_json::Value = serde_json::from_slice(&requests[0]).unwrap();
        assert_eq!(request["method"], "Create");
        assert_eq!(request["attach"], false);
        assert_eq!(request["executionRef"], "Host/tools");
        assert_eq!(
            request["resourceRef"],
            "shell-terminal.d2bus.org.ShellSession/primary"
        );
    }

    #[test]
    fn watch_stops_only_for_terminal_shell_states() {
        assert!(!terminal_shell_status(&json!({"state": "attached"})));
        assert!(!terminal_shell_status(&json!({"state": "detached"})));
        for state in [
            "killed",
            "pool-unavailable",
            "feature-disabled",
            "output-gap",
        ] {
            assert!(terminal_shell_status(&json!({"state": state})));
        }
    }
}
