#![allow(dead_code)]

use std::{
    ffi::OsString,
    io::{self, IsTerminal as _, Write as _},
};

use clap::CommandFactory;
use d2b_core::{error::Error as CoreError, host_check};
use serde::Serialize;
use serde_json::Value;

mod activation;
mod complete;
mod context;
mod dispatch;
mod doctor;
mod endpoint;
mod exec;
mod exec_client;
mod guest;
mod host;
pub mod host_generation;
mod host_validate;
mod provider;
mod resource;
mod share;
mod shell;
mod terminal_client;
mod zone;
mod zone_audit;
mod zone_doctor;
mod zone_support_bundle;

pub(crate) const MAX_FRAME_BYTES: usize = d2b_contracts::MAX_FRAME_SIZE;

/// Exit code for api-ready timeout in strict mode.
pub const EXIT_API_TIMEOUT: i32 = 33;

#[derive(Debug)]
pub(crate) struct CliFailure {
    pub(crate) exit_code: i32,
    pub(crate) message: String,
    pub(crate) rendered_stderr: Option<String>,
    pub(crate) admission_recovery: bool,
}

impl CliFailure {
    pub(crate) fn new(exit_code: i32, message: impl Into<String>) -> Self {
        Self {
            exit_code,
            message: message.into(),
            rendered_stderr: None,
            admission_recovery: false,
        }
    }

    pub(crate) fn host_check_probe_error(error: host_check::ProbeError) -> Self {
        let operator_error = CoreError::internal_io(error.opaque_reason);
        Self {
            exit_code: 1,
            message: operator_error.message(),
            rendered_stderr: render_operator_error(&operator_error, Some("host check")),
            admission_recovery: false,
        }
    }
}

pub(crate) fn print_json<T>(value: &T) -> Result<(), CliFailure>
where
    T: Serialize,
{
    let mut data = serde_json::to_string_pretty(value)
        .map_err(|err| CliFailure::new(1, format!("failed to render JSON: {err}")))?;
    data.push('\n');
    print_stdout(&data);
    Ok(())
}

#[cfg(test)]
thread_local! {
    static TEST_STDOUT_CAPTURE: std::cell::RefCell<Option<Vec<u8>>> =
        const { std::cell::RefCell::new(None) };
    static TEST_STDERR_CAPTURE: std::cell::RefCell<Option<Vec<u8>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) static TEST_STDOUT_CAPTURE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
pub(crate) fn with_test_stdout_capture<T>(f: impl FnOnce() -> T) -> (T, Vec<u8>) {
    let _guard = TEST_STDOUT_CAPTURE_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    TEST_STDOUT_CAPTURE.with(|capture| {
        *capture.borrow_mut() = Some(Vec::new());
    });
    let result = f();
    let stdout = TEST_STDOUT_CAPTURE
        .with(|capture| capture.borrow_mut().take())
        .expect("stdout capture active");
    (result, stdout)
}

#[cfg(test)]
pub(crate) fn with_test_output_capture<T>(f: impl FnOnce() -> T) -> (T, Vec<u8>, Vec<u8>) {
    let _guard = TEST_STDOUT_CAPTURE_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    TEST_STDOUT_CAPTURE.with(|capture| {
        *capture.borrow_mut() = Some(Vec::new());
    });
    TEST_STDERR_CAPTURE.with(|capture| {
        *capture.borrow_mut() = Some(Vec::new());
    });
    let result = f();
    let stdout = TEST_STDOUT_CAPTURE
        .with(|capture| capture.borrow_mut().take())
        .expect("stdout capture active");
    let stderr = TEST_STDERR_CAPTURE
        .with(|capture| capture.borrow_mut().take())
        .expect("stderr capture active");
    (result, stdout, stderr)
}

pub(crate) fn print_stdout(text: &str) {
    let _ = write_stdout_bytes(text.as_bytes());
}

pub(crate) fn print_stderr(text: &str) {
    let _ = write_stderr_bytes(text.as_bytes());
}

pub(crate) fn write_stdout_bytes(bytes: &[u8]) -> io::Result<()> {
    #[cfg(test)]
    {
        let captured = TEST_STDOUT_CAPTURE.with(|capture| {
            if let Some(buffer) = capture.borrow_mut().as_mut() {
                buffer.extend_from_slice(bytes);
                true
            } else {
                false
            }
        });
        if captured {
            return Ok(());
        }
    }
    let mut stdout = io::stdout().lock();
    stdout.write_all(bytes)?;
    stdout.flush()
}

pub(crate) fn write_stderr_bytes(bytes: &[u8]) -> io::Result<()> {
    #[cfg(test)]
    {
        let captured = TEST_STDERR_CAPTURE.with(|capture| {
            if let Some(buffer) = capture.borrow_mut().as_mut() {
                buffer.extend_from_slice(bytes);
                true
            } else {
                false
            }
        });
        if captured {
            return Ok(());
        }
    }
    let mut stderr = io::stderr().lock();
    stderr.write_all(bytes)?;
    stderr.flush()
}

pub(crate) fn report_failure(err: CliFailure) -> i32 {
    let mut stderr = io::stderr().lock();
    if let Some(rendered_stderr) = err.rendered_stderr {
        let _ = stderr.write_all(rendered_stderr.as_bytes());
    } else {
        let _ = writeln!(stderr, "d2b: {}", err.message);
    }
    err.exit_code
}

pub(crate) fn render_operator_error(
    error: &CoreError,
    owning_command: Option<&str>,
) -> Option<String> {
    let mut value = serde_json::to_value(error).ok()?;
    if let Some(owning_command) = owning_command {
        value.as_object_mut()?.insert(
            "owningCommand".to_owned(),
            Value::String(owning_command.to_owned()),
        );
    }
    let mut rendered = serde_json::to_string_pretty(&value).ok()?;
    rendered.push('\n');
    Some(rendered)
}

pub(crate) fn stdout_is_tty() -> bool {
    io::stdout().is_terminal()
}

pub(crate) fn sha256_hex(data: &[u8]) -> String {
    use sha2::Digest as _;
    let digest: [u8; 32] = sha2::Sha256::digest(data).into();
    let mut hex = String::with_capacity("sha256:".len() + 64);
    hex.push_str("sha256:");
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

pub fn cli_command() -> clap::Command {
    let mut command = dispatch::ModernCli::command();
    command.set_bin_name("d2b");
    command
}

pub fn run<I>(args: I) -> i32
where
    I: IntoIterator<Item = OsString>,
{
    let raw_args: Vec<OsString> = args.into_iter().collect();
    if raw_args.is_empty() {
        return 1;
    }

    if raw_args.len() == 1 {
        print_stdout("d2b 0.0.0-bootstrap\n");
        print_stdout("Run `d2b --help` for the typed Zone command surface.\n");
        return 0;
    }

    dispatch::modern_run(raw_args)
}
